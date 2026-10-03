#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)] // Isolated E2E CLI and local fixture processes.
use herdr_farm::{migration, runtime};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    process::{Command, Output},
};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");
// A deterministic fake Herdr and fake agent. The listener provides a genuine
// session incarnation; the fake CLI implements the public Herdr command/API
// boundary and persists the simulated agent's observable state between calls.
const HERDR: &str = r#"#!/usr/bin/python3
import fcntl,json,os,sys
root=os.environ['HOME'];lock=open(root+'/herdr-fixture.lock','w');fcntl.flock(lock,fcntl.LOCK_EX);path=root+'/herdr-state.json';a=sys.argv[1:]
s=json.load(open(path)) if os.path.exists(path) else {'creates':0,'starts':0,'prompts':[],'live':False,'agent':False,'accepted':False}
def pane():return {'pane_id':s.get('pane','w1:p1'),'workspace_id':'w1','tab_id':'w1:t1','terminal_id':'term'+str(s['creates']),'cwd':s.get('cwd','')}
def agent():return dict(pane(),agent='claude',name='hp-demo-coordinator',agent_status='working' if s['accepted'] else 'idle',interactive_ready=True)
if a==['--version']:print('herdr 0.9.1');sys.exit(0)
if a==['remote-api-bridge']:
 r=json.loads(sys.stdin.readline());m=r['method'];p=r.get('params') or {};res=None
 if m=='pane.list':res={'panes':[pane()] if s['live'] else []}
 elif m=='agent.list':res={'type':'agent_list','agents':[agent()] if s['live'] and s['agent'] else []}
 elif m=='agent.explain':res={'explain':{'agent':'claude','state':'working' if s['accepted'] else 'idle','manifest_source':'bundled','manifest_version':'fixture-1','matched_rule':{'id':'prompt','state':'idle'},'visible_idle':not s['accepted'],'visible_working':s['accepted'],'visible_blocker':False,'screen_detection_skipped':False,'skip_state_update':False,'local_override_shadowing_remote':False,'fallback_reason':None,'warning':None}}
 elif m=='agent.prompt':
  s['prompts'].append(p['text'])
  s['accepted']=len(s['prompts'])>s.get('swallow',0)
  res={'type':'agent_prompted','agent':agent()}
 else:res={'type':'ok'}
 reply={'id':r['id'],'result':res}
elif a[:2]==['workspace','create']:
 s['creates']+=1;s.update(live=True,agent=False,accepted=False,cwd=a[a.index('--cwd')+1],pane='w1:p'+str(s['creates']))
 reply={'result':{'type':'workspace_created','workspace':{'workspace_id':'w1'},'pane':pane(),'root_pane':pane(),'tab':{'tab_id':'w1:t1'}}}
elif a[:2]==['agent','start']:
 s['starts']+=1;s['agent']=True;s['args']=a;reply={'result':{'agent':agent()}}
elif a==['pane','list']:reply={'result':{'panes':[pane()] if s['live'] else []}}
elif a==['agent','list']:reply={'result':{'agents':[agent()] if s['live'] and s['agent'] else []}}
else:reply={'result':{'type':'ok'}}
json.dump(s,open(path,'w'))
if a==['remote-api-bridge'] and m=='agent.prompt' and os.path.exists(root+'/lose-prompt-reply'):
 os.unlink(root+'/lose-prompt-reply');sys.exit(1)
print(json.dumps(reply))
"#;
struct Lab {
    home: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    herdr: PathBuf,
}
impl Lab {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("root");
        let project = root.join("demo");
        let herdr = home.path().join("herdr");
        fs::write(&herdr, HERDR).unwrap();
        fs::set_permissions(&herdr, fs::Permissions::from_mode(0o700)).unwrap();
        let lab = Self {
            home,
            root,
            project,
            herdr,
        };
        lab.ok(&["new", "demo"]);
        lab.ok(&["pause", "demo"]);
        let plan = migration::inspect(&lab.project).unwrap();
        migration::apply(&lab.project, &plan, true).unwrap();
        lab
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .env_clear()
            .env("HOME", self.home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("HERDR_BIN_PATH", &self.herdr)
            .env("HERDR_SOCKET_PATH", self.home.path().join("s"))
            .env(
                "HERDR_FARM_TEST_TIME_SCALE",
                include_str!("support/time-scale.txt").trim(),
            )
            .args(["--root", self.root.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.cli(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    }
    fn state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.home.path().join("herdr-state.json")).unwrap())
            .unwrap()
    }
    fn stop(&self) {
        self.ok(&["ticker", "stop"]);
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        let _ = self.cli(&["ticker", "stop"]);
    }
}
#[test]
fn canonical_context_and_thread_refusals_use_public_commands() {
    let l = Lab::new();
    let head = runtime::snapshot(&l.project).unwrap().head.to_string();
    l.ok(&[
        "task",
        "demo",
        "add",
        "plan",
        "--title",
        "Plan migration",
        "--expected-head",
        &head,
    ]);
    let text = l.ok(&["context", "demo", "--peek"]);
    for expected in [
        "Runtime owner: SQLite",
        "Draft: Plan migration",
        "Attempts (running / awaiting review / verified / integrated)",
        "Recent results",
        "task demo add TASK --title TITLE --expected-head HEAD",
        "launch demo run --task TASK",
        "result demo verify SUBMISSION",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    for args in [
        vec!["thread", "list", "demo"],
        vec!["thread", "show", "demo", "t-0001"],
        vec![
            "thread",
            "start",
            "demo",
            "--title",
            "new",
            "--task-file",
            "/missing",
        ],
        vec![
            "thread",
            "prompt",
            "demo",
            "t-0001",
            "--text-file",
            "/missing",
        ],
    ] {
        let out = l.cli(&args);
        assert!(!out.status.success());
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains("thread commands are legacy-only"), "{error}");
        assert!(error.contains("launch demo run"));
    }
    assert_eq!(runtime::snapshot(&l.project).unwrap().tasks.len(), 1);
}
#[test]
fn socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane() {
    let l = Lab::new();
    let _listener = UnixListener::bind(l.home.path().join("s"))
        .expect("socket fixture: sandbox may deny Unix sockets");
    fs::write(l.home.path().join("herdr-state.json"),serde_json::to_vec(&json!({"creates":0,"starts":0,"prompts":[],"live":false,"agent":false,"accepted":false,"swallow":1})).unwrap()).unwrap();
    let config = l.home.path().join(".config/herdr-farm/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        format!(
            "[safety.{:?}]\ncoordinator_agent_args=['--fixture-safe']\ncoordinator_agent_args_kind='claude'\n",
            l.project.to_string_lossy()
        ),
    )
    .unwrap();
    l.ok(&["reconcile", "demo", "--record"]);
    let snapshot = runtime::snapshot(&l.project).unwrap();
    let control = snapshot.control.unwrap();
    l.ok(&[
        "runtime",
        "demo",
        "state",
        "active",
        "--expected-revision",
        &control.revision.to_string(),
        "--expected-head",
        &snapshot.head.to_string(),
    ]);
    l.ok(&["open", "demo"]);
    l.stop();
    let state = l.state();
    assert_eq!(state["creates"], 1);
    assert_eq!(state["starts"], 1);
    assert_eq!(state["prompts"].as_array().unwrap().len(), 2);
    assert!(
        state["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "--fixture-safe")
    );
    let snapshot = runtime::snapshot(&l.project).unwrap();
    assert!(
        snapshot
            .runtime_bindings
            .iter()
            .any(|b| b.id == "coordinator" && b.identity.pane_id == "w1:p1")
    );
    assert!(snapshot.ownership.iter().any(
        |o| o.binding == "coordinator" && o.agent.as_ref().is_some_and(|a| a.kind == "claude")
    ));
    assert!(
        snapshot
            .observations
            .iter()
            .any(|o| o.binding == "coordinator" && o.agent_present)
    );
    assert!(
        state["prompts"][0]
            .as_str()
            .unwrap()
            .contains("context demo")
    );
    l.ok(&["open", "demo"]);
    l.stop();
    assert_eq!(l.state()["starts"], 1);
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(), 2);
    let mut state = l.state();
    state["live"] = json!(false);
    state["agent"] = json!(false);
    fs::write(
        l.home.path().join("herdr-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    l.ok(&["open", "demo", "--reprime"]);
    l.stop();
    let state = l.state();
    assert_eq!(state["creates"], 2);
    assert_eq!(state["starts"], 2);
    assert!(
        runtime::snapshot(&l.project)
            .unwrap()
            .ownership
            .iter()
            .any(|o| o.binding == "coordinator" && o.binding_revision == 2)
    );
    // Lost acknowledgement after the fake agent accepted: recovery observes
    // the same execution and completes the pending receipt without re-sending.
    let mut state = l.state();
    state["accepted"] = json!(false);
    fs::write(
        l.home.path().join("herdr-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    fs::write(l.home.path().join("lose-prompt-reply"), "").unwrap();
    assert!(!l.cli(&["open", "demo", "--reprime"]).status.success());
    let sent = l.state()["prompts"].as_array().unwrap().len();
    l.ok(&["open", "demo"]);
    l.stop();
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(), sent);
}
