#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)] // Test-only spawns outside the library may skip the spawn gate.
//! The operator path for canonical workers on a real project, through the
//! compiled CLI only: `profile verify-interaction --retain` produces the
//! launchable evidence (in a directory the agent trusts, with the owner's login
//! shared into the isolated home and the profile's model and effort pinned in
//! the agent's own configuration), then `launch PROJECT run` takes a planning
//! task from nothing to a reserved attempt, for a Codex and a Claude Code
//! profile, and is safe to repeat.
//!
//! Herdr is a Python stand-in that serves the native session API for real
//! (`server` binds a Unix socket); the agents are tiny binaries that refuse to
//! start unless their login is shared and their directory trusted, as the real
//! ones do. Tests marked "socket" bind Unix sockets and cannot run in a
//! sandbox that forbids it.
use herdr_farm::{domain::*, worker_supervision::{ProcessIncarnation, SupervisorIdentity}};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");

/// The stand-in Herdr: `--version`, probes and the API bridge as in the other
/// suites, plus `server`, which serves one native workspace backed by a real
/// process (stdin is a FIFO the gate reads its release token from).
const HERDR: &str = r#"#!/usr/bin/python3
import json,os,socket,subprocess,sys
args=sys.argv[1:]
if args==['--version']:print('herdr 0.9.1');sys.exit(0)
def serve(path):
 root=os.path.dirname(path);s={};next_tab=0;tabs=[{'tab_id':'owner-tab','label':'Owner'}]
 server=socket.socket(socket.AF_UNIX);server.bind(path);server.listen()
 while True:
  c,_=server.accept();f=c.makefile('rw');line=f.readline()
  if not line:c.close();continue
  r=json.loads(line);m=r['method'];p=r.get('params') or {}
  with open(os.path.join(root,'calls.jsonl'),'a') as log:log.write(json.dumps(r)+'\n')
  live='pid' in s
  kind=s.get('kind','claude')
  screen={'codex':'model: gpt-6.1-sol low   /model to change','claude':'Sonnet 5.5 · Claude Max'}[kind]
  pane={'pane_id':'w1:p1','workspace_id':'w1','tab_id':'w1:t1','terminal_id':'term1','cwd':s.get('cwd')}
  agent=dict(pane,agent=kind,interactive_ready=True,agent_status='working' if s.get('accepted') else 'idle',**({'name':s['name']} if 'name' in s else {}))
  res=None
  if m=='ping':res={'type':'pong','version':'0.9.1','capabilities':{'workspace_create_command':True}}
  elif m=='workspace.create_command' and not live:
   fifo=os.path.join(root,'input');os.mkfifo(fifo);fd=os.open(fifo,os.O_RDWR)
   child=subprocess.Popen(p['command'],cwd=p['cwd'],stdin=fd,stdout=subprocess.DEVNULL,stderr=open(os.path.join(root,'agent.log'),'w'),start_new_session=True,env={'PATH':'/usr/bin:/bin'})
   s.update(pid=child.pid,argv=p['command'],cwd=p['cwd'],label=p['label'],fifo=fifo,kind='codex' if any(a.endswith('/codex') for a in p['command']) else 'claude')
   res={'type':'workspace_created','workspace':{'workspace_id':'w1','pane_count':1},'root_pane':{'pane_id':'w1:p1'}}
  elif m=='workspace.list':res={'type':'workspace_list','workspaces':[{'workspace_id':'w1','label':s['label'],'pane_count':1,'tab_count':1}] if live else []}
  elif m=='tab.list':res={'tabs':tabs}
  elif m=='tab.create':
   next_tab+=1;tab='viewer-'+str(next_tab);tabs.append({'tab_id':tab,'label':p['label']})
   res={'root_pane':{'workspace_id':p['workspace_id'],'tab_id':tab,'pane_id':tab+':pane'}}
  elif m=='tab.rename':
   for t in tabs:
    if t['tab_id']==p['tab_id']:t['label']=p['label']
   res={'type':'ok'}
  elif m=='tab.close':tabs[:]=[t for t in tabs if t['tab_id']!=p['tab_id']];res={'type':'ok'}
  elif m in ['tab.focus','pane.run']:res={'type':'ok'}
  elif m=='pane.list':res={'panes':[pane] if live else []}
  elif m=='pane.get':res={'pane':dict(pane,agent=kind)}
  elif m=='pane.read':res={'type':'pane_read','text':screen}
  elif m=='pane.process_info':res={'process_info':{'pane_id':'w1:p1','foreground_processes':[{'pid':s['pid'],'argv':s['argv']}] if live else []}}
  elif m=='pane.send_input':
   fd=os.open(s['fifo'],os.O_WRONLY);os.write(fd,p['text'].encode());os.close(fd);s['released']=True;res={'type':'ok'}
  elif m=='agent.list':res={'type':'agent_list','agents':[agent] if live and s.get('released') else []}
  elif m=='agent.explain':res={'type':'agent_explain','explain':{'agent':kind,'state':'idle','manifest_source':'bundled','manifest_version':'2026.09.14.1',
   'matched_rule':{'id':'prompt','state':'idle'},'visible_idle':True,'visible_blocker':False,'visible_working':False,'screen_detection_skipped':False,
   'skip_state_update':False,'local_override_shadowing_remote':False,'fallback_reason':None,'warning':None}}
  elif m=='agent.prompt':s['accepted']=True;res={'type':'agent_prompted','agent':agent}
  elif m=='agent.rename':s['name']=agent['name']=p['name'];res={'type':'agent_info','agent':agent}
  if res is not None:f.write(json.dumps({'id':r['id'],'result':res})+'\n');f.flush()
  c.close()
if args==['server']:serve(os.environ['HERDR_SOCKET_PATH']);sys.exit(0)
probe={('pane','list'):'pane.list',('agent','list'):'agent.list'}.get(tuple(args))
if args[:2] in [['tab','list'],['tab','create'],['tab','close'],['tab','focus'],['tab','rename'],['pane','run']]:
 method='.'.join(args[:2]);params={}
 if method=='tab.list':params={'workspace_id':args[3]}
 elif method=='tab.create':params={'workspace_id':args[3],'cwd':args[5],'label':args[7],'focus':False}
 elif method=='tab.rename':params={'tab_id':args[2],'label':args[3]}
 elif method.startswith('tab.'):params={'tab_id':args[2]}
 else:params={'pane_id':args[2],'command':args[4:]}
 probe=None;bridge=json.dumps({'id':'viewer','method':method,'params':params}).encode()+b'\n'
else:
 assert probe or args==['remote-api-bridge']
 bridge=None
line=json.dumps({'id':'probe','method':probe}).encode()+b'\n' if probe else bridge or sys.stdin.buffer.readline()
c=socket.socket(socket.AF_UNIX);c.connect(os.environ['HERDR_SOCKET_PATH']);c.sendall(line)
reply=c.makefile('rb').readline()
if not reply:sys.exit(1)
sys.stdout.buffer.write(json.dumps({'result':json.loads(reply)['result']}).encode() if probe else reply)
"#;

/// A Herdr that serves no session: it answers its version and the empty
/// pane/agent probes, for runs against an operator-managed server socket.
const STATIC_HERDR: &str = r#"#!/bin/sh
[ "$1" = --version ] && echo 'herdr 0.9.1' && exit 0
[ "$1 $2" = 'pane list' ] && echo '{"result":{"panes":[]}}' && exit 0
[ "$1 $2" = 'agent list' ] && echo '{"result":{"type":"agent_list","agents":[]}}' && exit 0
exit 3
"#;

/// The stand-in agent's login check: a Codex agent must find the shared login
/// file (`login`) in its home; a Claude agent (`login` empty) must find the
/// setup token in its environment and no credentials file.
fn login_check(login: &str) -> String {
    if login.is_empty() {
        r#"if std::env::var("CLAUDE_CODE_OAUTH_TOKEN").as_deref()!=Ok("fixture-setup-token")||std::path::Path::new(&format!("{home}/.claude/.credentials.json")).exists(){eprintln!("not logged in");std::process::exit(7)}"#.to_owned()
    } else {
        format!(r#"if std::fs::read_to_string(format!("{{home}}/{login}")).unwrap_or_default().trim()!="shared-login"{{eprintln!("not logged in");std::process::exit(7)}}"#)
    }
}

/// A stand-in agent: answers `--version`; otherwise it must be logged in (see
/// `login_check`) and its working directory trusted in its own configuration.
fn agent_source(version: &str, login: &str, trust: &str) -> String {
    let check = login_check(login);
    format!(
        r#"fn main(){{
 if std::env::args().nth(1).as_deref()==Some("--version"){{println!({version:?});return}}
 let home=std::env::var("HOME").unwrap_or_default();
 {check}
 let cwd=std::env::current_dir().unwrap();
 if !std::fs::read_to_string(format!("{{home}}/{trust}")).unwrap_or_default().contains(cwd.to_str().unwrap()){{eprintln!("untrusted directory");std::process::exit(8)}}
 loop{{std::thread::park()}}
}}"#
    )
}

/// A stand-in agent that, like the real ones, writes into its execution home:
/// on `--version` (when it is given a HOME) and during the session. `version_extra`
/// is extra Rust run on `--version` with `home` bound, for hostile variants.
fn writing_agent_source(version: &str, login: &str, trust: &str, version_dir: &str, version_file: &str, session_file: &str, version_extra: &str) -> String {
    let check = login_check(login);
    format!(
        r#"fn main(){{
 let home=std::env::var("HOME").unwrap_or_default();
 if std::env::args().nth(1).as_deref()==Some("--version"){{
  if !home.is_empty(){{std::fs::create_dir_all(format!("{{home}}/{version_dir}")).unwrap();std::fs::write(format!("{{home}}/{version_dir}/{version_file}"),"probe").unwrap();}}
  {version_extra}
  println!({version:?});return
 }}
 {check}
 let cwd=std::env::current_dir().unwrap();
 if !std::fs::read_to_string(format!("{{home}}/{trust}")).unwrap_or_default().contains(cwd.to_str().unwrap()){{eprintln!("untrusted directory");std::process::exit(8)}}
 let session=format!("{{home}}/{session_file}");
 std::fs::create_dir_all(std::path::Path::new(&session).parent().unwrap()).unwrap();
 std::fs::write(session,"{{}}").unwrap();
 loop{{std::thread::park()}}
}}"#
    )
}

struct Lab {
    _top: tempfile::TempDir,
    /// The private runtime directory every CLI run uses for its Herdr sockets,
    /// so a test sees (and never shares) the socket directories a run leaves.
    runtime: tempfile::TempDir,
    home: PathBuf,
    root: PathBuf,
    project: PathBuf,
    repo: PathBuf,
    key: PathBuf,
    extra_env: Vec<(String, PathBuf)>,
}

impl Drop for Lab {
    /// Safety net: a failed test must not leave a stand-in Herdr server behind.
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/pkill").args(["-KILL", "-f"]).arg(format!("{} server", self.home.join("bin/herdr").display())).status();
    }
}

impl Lab {
    /// A migrated, paused project `demo`, two worker profiles pinning model and
    /// effort (Codex `gpt-6.1-sol`/low, Claude `claude-sonnet-5-5`/low), a Git
    /// repository with an `integration` branch, and the owner's logins.
    fn new() -> Self { Self::with_herdr(HERDR) }
    /// As `new`, with `herdr` as the Herdr executable's source.
    fn with_herdr(herdr: &str) -> Self {
        // Not under /tmp: the worker sandbox keeps /tmp private, and the owner's
        // home (with its logins) is where the shared login files live.
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(base).unwrap();
        let top = tempfile::tempdir_in(base).unwrap();
        let home = top.path().canonicalize().unwrap().join("home");
        for dir in ["", "bin", "repo", "agent-home-codex", "agent-home-claude", ".codex", ".claude"] {
            fs::create_dir_all(home.join(dir)).unwrap();
        }
        // The Claude worker's login: a long-lived setup-token file, outside the
        // project and the agent directories (the owner's credentials file is
        // never shared with a worker).
        let token = home.join("claude-setup-token");
        fs::write(&token, "fixture-setup-token\n").unwrap();
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
        for login in [".codex/auth.json", ".claude/.credentials.json"] {
            fs::write(home.join(login), "shared-login").unwrap();
            fs::set_permissions(home.join(login), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let key = home.join(".config/herdr-farm/owner");
        fs::create_dir_all(key.parent().unwrap()).unwrap();
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).output().unwrap().status.success());
        let public = fs::read_to_string(key.with_extension("pub")).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let config = home.join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let budget = "max_wall_seconds=600\nunknown_usage='allow_with_warning'\n";
        fs::write(&config, format!(
            "[authority]\nversion=1\nrevision=1\napproval_public_key={public:?}\n\
[worker_isolation.login]\nclaude_token_file={token:?}\n\
[profiles.codex-sol]\nkind='codex'\npermission_policy='interactive'\nmodel='gpt-6.1-sol'\nreasoning_effort='low'\n[profiles.codex-sol.budget]\n{budget}\
[profiles.claude-sonnet]\nkind='claude'\npermission_policy='interactive'\nmodel='claude-sonnet-5-5'\nreasoning_effort='low'\n[profiles.claude-sonnet.budget]\n{budget}")).unwrap();
        let runtime = tempfile::Builder::new().prefix("hp").tempdir_in(std::env::temp_dir()).unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let lab = Lab { runtime, root: home.join("root"), project: home.join("root/demo"), repo: home.join("repo"), home, key, _top: top, extra_env: Vec::new() };
        for command in ["new", "pause"] {
            lab.ok(&[command, "demo"]);
        }
        herdr_farm::migration::apply(&lab.project, &herdr_farm::migration::inspect_with_config(&lab.project, &config).unwrap(), true).unwrap();
        fs::write(lab.home.join("bin/herdr"), herdr).unwrap();
        for (name, version, login, trust) in [("codex", "codex-cli 0.154.0", ".codex/auth.json", ".codex/config.toml"), ("claude", "2.1.0 (Claude Code)", "", ".claude.json")] {
            let source = lab.home.join(format!("bin/{name}.rs"));
            fs::write(&source, agent_source(version, login, trust)).unwrap();
            let built = Command::new("rustc").args(["--edition", "2021", "-o"]).arg(lab.home.join("bin").join(name)).arg(&source).output().unwrap();
            assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        }
        for name in ["herdr", "codex", "claude"] {
            fs::set_permissions(lab.home.join("bin").join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        lab.git(&["init", "-q", "-b", "master"]);
        for (key, value) in [("gc.auto", "0"), ("gc.autoDetach", "false"), ("maintenance.auto", "false")] {
            lab.git(&["config", "--local", key, value]);
        }
        lab.git(&["commit", "-q", "--allow-empty", "-m", "start"]);
        lab.git(&["branch", "integration"]);
        fs::write(lab.project.join("PROJECT.md"), format!("+++\n[[repos]]\npath={:?}\n+++\nShadow trial project. Follow the task.\n", lab.repo.to_str().unwrap())).unwrap();
        lab
    }
    /// Replace the stand-in agent `name` with one built from `source`.
    fn build_agent(&self, name: &str, source: &str) {
        let file = self.home.join(format!("bin/{name}.rs"));
        fs::write(&file, source).unwrap();
        let built = Command::new("rustc").args(["--edition", "2021", "-o"]).arg(self.home.join("bin").join(name)).arg(&file).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        fs::set_permissions(self.home.join("bin").join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn prepare(&self, profile: &str, kind: &str) -> Output {
        let (herdr, agent, home) = (self.home.join("bin/herdr"), self.home.join("bin").join(kind), self.home.join(format!("agent-home-{kind}")));
        self.cli(&["profile", "prepare", "demo", profile, "--herdr-executable", herdr.to_str().unwrap(),
            "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap()])
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("/usr/bin/git").env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("PATH", "/usr/bin:/bin").env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "fixture").env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "fixture").env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .current_dir(&self.repo).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }
    fn cli(&self, args: &[&str]) -> Output {
        let mut legacy_args = args.to_vec();
        if args.first() == Some(&"new") { legacy_args.insert(1, "--legacy"); }
        let args = legacy_args.as_slice();
        // Verification's disposable server socket lives under the temporary directory.
        Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", &self.home).env("HERDR_PROJECTS_OWNER_HOME", &self.home).env("PATH", "/usr/bin:/bin").env("HERDR_BIN_PATH", self.home.join("bin/herdr"))
            .env("TMPDIR", std::env::var_os("TMPDIR").unwrap_or("/tmp".into())).env("XDG_RUNTIME_DIR", self.runtime.path())
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_path())))
            .args(["--root", self.root.to_str().unwrap()]).args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.cli(args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }
    fn fail(&self, args: &[&str]) -> String {
        let out = self.cli(args);
        assert!(!out.status.success(), "{args:?} succeeded: {}", String::from_utf8_lossy(&out.stdout));
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
    /// `profile verify-interaction --retain` for `profile`, entirely through the CLI.
    fn verify(&self, profile: &str, kind: &str) -> Value {
        let (herdr, agent, home) = (self.home.join("bin/herdr"), self.home.join("bin").join(kind), self.home.join(format!("agent-home-{kind}")));
        self.ok(&["profile", "verify-interaction", "demo", profile, "--herdr-executable", herdr.to_str().unwrap(),
            "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap(), "--retain"])
    }
    /// Launchable evidence for `profile` as the existing suites plant it, for
    /// runs that must not need a live agent session (the evidence a real
    /// `verify-interaction --retain` produces is covered by the socket test).
    fn plant_launchable(&self, profile: &str, kind: &str, model: &str) {
        #[derive(serde::Serialize)] struct Pinned { model: String, reasoning_effort: String, model_on_screen: bool }
        #[derive(serde::Serialize)] struct Interaction { session: ResourceIdentity, terminal: &'static str, readiness_manifest: &'static str, prompt_digest: String, acknowledged_unix_ms: i64, pinned: Pinned }
        #[derive(serde::Serialize)] struct Evidence { version: u32, prepared_profile: VersionedReference, supervisor: SupervisorIdentity, native_kind: String, observed_unix_ms: i64, stopped_unix_ms: i64, interaction: Interaction }
        let (herdr, agent, home) = (self.home.join("bin/herdr"), self.home.join("bin").join(kind), self.home.join(format!("agent-home-{kind}")));
        let prepared = self.ok(&["profile", "prepare", "demo", profile, "--herdr-executable", herdr.to_str().unwrap(),
            "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap()]);
        let mut frozen: FrozenProfile = serde_json::from_value(prepared["profile"].clone()).unwrap();
        let evidence = Evidence { version: 2, prepared_profile: frozen.reference().unwrap(), native_kind: kind.into(), observed_unix_ms: 1000, stopped_unix_ms: 1001,
            supervisor: SupervisorIdentity { version: 1, boot_id: "00000000-0000-0000-0000-000000000001".into(), host_id: None, observer_namespace: (1, 2), worker_namespace: (1, 3),
                outer: ProcessIncarnation { pid: 20, device: 1, inode: 4 }, init: ProcessIncarnation { pid: 21, device: 1, inode: 5 } },
            interaction: Interaction { session: ResourceIdentity { device: 1, inode: 2, born_secs: 1, born_nanos: 0 }, terminal: "fixture-terminal", readiness_manifest: "fixture-manifest",
                prompt_digest: "a".repeat(64), acknowledged_unix_ms: 999, pinned: Pinned { model: model.into(), reasoning_effort: "low".into(), model_on_screen: true } } };
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&evidence).unwrap()));
        let supported = CapabilityEvidence::Supported { evidence: VersionedReference { id: format!("native-transport-{hash}"), revision: 1, digest: hash } };
        let c = &mut frozen.capabilities;
        (c.launch, c.stop, c.readiness_observation, c.prompt_submission) = (supported.clone(), supported.clone(), supported.clone(), supported);
        let reference = frozen.reference().unwrap();
        let store = self.project.join(".state/state.db").canonicalize().unwrap();
        let metadata = fs::metadata(&store).unwrap();
        let report = serde_json::json!({"preparation":{"profile":frozen,"reference":reference,"launchable":true,"protocol_capable":false,"certified":false},
            "evidence":evidence,"source_store":[store,metadata.dev(),metadata.ino()]}).to_string();
        rusqlite::Connection::open(&store).unwrap().execute("INSERT INTO native_profiles(profile_digest,report,report_digest,sequence) VALUES(?1,?2,?3,(SELECT max(sequence) FROM events))",
            rusqlite::params![reference.digest, report, format!("{:x}", Sha256::digest(report.as_bytes()))]).unwrap();
    }
    /// A control-socket inode for an operator-managed server (nothing listens on it).
    fn socket_inode_once(&self, name: &str) -> PathBuf {
        let path = self.home.join(name);
        if path.exists() {
            return path;
        }
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mknod(c.as_ptr(), libc::S_IFSOCK | 0o600, 0) }, 0);
        path
    }
    /// Follow the closing script of the attempt's retained brief as the worker
    /// would: edit the deliverable in a worktree of the repository and run the
    /// script. `herdr-farm` resolves to the product CLI with the spool
    /// variable removed, so outside any sandbox it records the submission
    /// directly (inside the sandbox the same command goes through the spool).
    fn follow_brief(&self, attempt: &str, output: &str, branch: &str) -> Output {
        let brief = self.ok(&["memory", "demo", "attempt-brief", "--attempt", attempt]);
        let text = brief["text"].as_str().unwrap();
        let script = text.split("```sh\n").nth(1).and_then(|rest| rest.split("```").next()).unwrap_or_else(|| panic!("the brief has no submission script:\n{text}"));
        let worktree = self.home.join(format!("worker-{branch}"));
        self.git(&["worktree", "add", "-q", "-b", branch, worktree.to_str().unwrap()]);
        fs::create_dir_all(worktree.join(output).parent().unwrap()).unwrap();
        fs::write(worktree.join(output), "The plan.\n").unwrap();
        let shim = self.home.join("shim");
        fs::create_dir_all(&shim).unwrap();
        fs::write(shim.join("herdr-farm"), format!("#!/bin/sh\nunset HERDR_FARM_SUBMISSION_SPOOL\nexec {BIN} \"$@\"\n")).unwrap();
        fs::set_permissions(shim.join("herdr-farm"), fs::Permissions::from_mode(0o700)).unwrap();
        Command::new("/bin/sh").arg("-c").arg(script).current_dir(&worktree).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim())
            .env("HOME", &self.home).env("HERDR_PROJECTS_OWNER_HOME", &self.home).env("PATH", format!("{}:/usr/bin:/bin", shim.display()))
            .env("TMPDIR", std::env::var_os("TMPDIR").unwrap_or("/tmp".into())).env("XDG_RUNTIME_DIR", self.runtime.path())
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("HERDR_FARM_SUBMISSION_SPOOL", self.project.join(".state/spool").join(attempt)).output().unwrap()
    }
    fn run_args<'a>(&'a self, task: &'a str, profile: &'a str, output: &'a str, prompt: &'a str) -> Vec<&'a str> {
        vec!["launch", "demo", "run", "--task", task, "--profile", profile, "--repository", self.repo.to_str().unwrap(), "--plan-output", output,
            "--prompt-file", prompt, "--integration-ref", "refs/heads/integration", "--sign-with", self.key.to_str().unwrap(),
            "--validity-seconds", "600", "--max-active-workers", "2"]
    }
}

/// Without retained launchable evidence the run stops at its first step and
/// leaves the project exactly as it was: no task, contract, server or binding.
#[test]
fn launch_run_refuses_loudly_at_the_first_failing_step_and_changes_nothing() {
    let lab = Lab::new();
    let before = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone.").unwrap();
    let error = lab.fail(&lab.run_args("plan-codex", "codex-sol", "docs/plan-codex.md", prompt.to_str().unwrap()));
    assert!(error.contains("verify-interaction"), "{error}");
    assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap(), before, "a refused run must not write");
    assert!(!lab.root.join(".herdr-run").exists(), "no Herdr server directory before the first step passes");
    // HERDR_BIN_PATH must name the verified Herdr by absolute path.
    let out = Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", &lab.home).env("HERDR_PROJECTS_OWNER_HOME", &lab.home).env("PATH", "/usr/bin:/bin").env("HERDR_BIN_PATH", "herdr")
        .args(["--root", lab.root.to_str().unwrap()]).args(lab.run_args("plan-codex", "codex-sol", "docs/plan-codex.md", prompt.to_str().unwrap())).output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("HERDR_BIN_PATH"), "{}", String::from_utf8_lossy(&out.stderr));
}

/// socket: verification trusts its own directory under the execution home,
/// pins the model and effort in the agent's configuration and checks them
/// after readiness, runs the agent as the owner's logged-in CLI, and retains
/// launchable evidence, with no SQL and no hand-edited trust.
#[test]
fn verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli() {
    let lab = Lab::new();
    for (profile, kind, model) in [("codex-sol", "codex", "gpt-6.1-sol"), ("claude-sonnet", "claude", "claude-sonnet-5-5")] {
        let report = lab.verify(profile, kind);
        assert_eq!(report["preparation"]["launchable"], true, "{kind}: {report}");
        assert_eq!(report["preparation"]["profile"]["kind"], kind);
        let pinned = &report["evidence"]["interaction"]["pinned"];
        assert_eq!((pinned["model"].as_str(), pinned["reasoning_effort"].as_str(), pinned["model_on_screen"].as_bool()), (Some(model), Some("low"), Some(true)), "{report}");
        let digest = report["preparation"]["reference"]["digest"].as_str().unwrap();
        let retained = lab.ok(&["profile", "retained", "demo", digest]);
        assert_eq!(retained["preparation"]["launchable"], true);
        // The agent's own configuration in the isolated home carries the pins and the trust.
        let home = lab.home.join(format!("agent-home-{kind}"));
        let work = home.join(".hp-verify-work");
        let config = fs::read_to_string(if kind == "codex" { home.join(".codex/config.toml") } else { home.join(".claude/settings.json") }).unwrap();
        assert!(config.contains(model) && config.contains("low"), "{config}");
        let trust = fs::read_to_string(if kind == "codex" { home.join(".codex/config.toml") } else { home.join(".claude.json") }).unwrap();
        assert!(trust.contains(work.to_str().unwrap()), "{trust}");
        // Codex's login stays the owner's single file: the home only has an empty
        // mount point. Claude's is the setup token in the agent's environment:
        // no credentials file and no copy of the token anywhere in the home.
        if kind == "codex" {
            assert_eq!(fs::read_to_string(home.join(".codex/auth.json")).unwrap(), "", "no copy of the login in the execution home");
        } else {
            assert!(!home.join(".claude/.credentials.json").exists(), "no Claude credentials file in the execution home");
            let leaked = Command::new("/usr/bin/grep").args(["-r", "-l", "fixture-setup-token"]).arg(&home).output().unwrap();
            assert!(leaked.stdout.is_empty(), "the token is in no file of the execution home: {}", String::from_utf8_lossy(&leaked.stdout));
        }
        let report_text = report.to_string();
        assert!(!report_text.contains("fixture-setup-token"), "the token never appears in a report");
    }
}

/// The one operator command takes a planning task from nothing to a reserved
/// attempt for each kind (an operator-managed server socket, launchable
/// evidence planted), signs through ssh-keygen with the owner's key, carries
/// the task text and deliverable into the retained brief, and a rerun skips every
/// finished step without reserving again.
#[test]
fn launch_run_reserves_a_planning_task_for_each_kind_and_reruns_safely() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    let config=lab.home.join(".config/herdr-farm/config.toml");
    let mut text=fs::read_to_string(&config).unwrap();
    text.push_str(&format!("\n[coordinator]\nsigning_key={:?}\n",lab.key));
    fs::write(&config,text).unwrap();
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone of the tactics game.").unwrap();
    let jobs = [("plan-codex", "codex-sol", "codex", "gpt-6.1-sol"), ("plan-claude", "claude-sonnet", "claude", "claude-sonnet-5-5")];
    let arguments = |lab: &Lab, (task, profile, ..): (&str, &str, &str, &str), prompt: &str| {
        let socket = lab.socket_inode_once(&format!("{task}.sock"));
        let mut args: Vec<String> = lab.run_args(task, profile, &format!("docs/{task}.md"), prompt).into_iter().map(str::to_owned).collect();
        args.extend(["--herdr-socket".into(), socket.display().to_string()]);
        let key=args.iter().position(|a|a=="--sign-with").unwrap();
        args.drain(key..key+2);
        args
    };
    // Prepare-only remains available for the first task; the second is added
    // directly while the first retains its reservation.
    for job in jobs {
        lab.plant_launchable(job.1, job.2, job.3);
        if job.0 != jobs[0].0 { continue; }
        let mut args = arguments(&lab, job, prompt.to_str().unwrap());
        args.push("--prepare-only".into());
        let report = lab.ok(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(report["attempt"].is_null(), "{report}");
    }
    let mut attempts = Vec::new();
    for job @ (task, _, kind, _) in jobs {
        let args = arguments(&lab, job, prompt.to_str().unwrap());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = format!("docs/{task}.md");
        let before = herdr_farm::runtime::snapshot(&lab.project).unwrap();
        let report = lab.ok(&args);
        let after = herdr_farm::runtime::snapshot(&lab.project).unwrap();
        assert_eq!(after.control, before.control, "adding a resource-free binding keeps the epoch and active control");
        let binding = after.runtime_bindings.iter().find(|b| b.task.as_ref().is_some_and(|id| id.as_str() == task)).unwrap();
        assert!(after.observations.iter().any(|o| o.binding == binding.id && o.binding_revision == binding.revision
            && o.task_revision == after.tasks.iter().find(|t| t.id.as_str() == task).map(|t| t.revision - 1)
            && o.pane == herdr_farm::reconcile::ResourceState::Unrecorded
            && o.worktree == herdr_farm::reconcile::ResourceState::Unrecorded && !o.agent_present));
        for previous in &before.attempts {
            assert_eq!(after.attempts.iter().find(|a| a.id == previous.id), Some(previous));
        }
        assert!(!after.events.iter().filter(|e| e.sequence > before.head).any(|e| e.kind == "project.reconciliation_invalidated"));
        assert_eq!(report["kind"], kind, "{report}");
        let attempt = report["attempt"].as_str().unwrap().to_owned();
        assert!(report["worktree"].as_str().unwrap().contains(".state/worktrees"), "{report}");
        let names: Vec<_> = report["steps"].as_array().unwrap().iter().map(|s| s["step"].as_str().unwrap()).collect();
        for step in ["profile_evidence", "project_control", "task", "contract", "queue", "scheduler_capacity", "integration_target", "herdr_server", "binding", "reconcile_and_activate", "knowledge_snapshot", "draft", "approval_import", "reserve"] {
            assert!(names.contains(&step), "{step} missing from {names:?}");
        }
        let state = herdr_farm::runtime::snapshot(&lab.project).unwrap();
        let record = state.tasks.iter().find(|t| t.id.as_str() == task).unwrap();
        assert_eq!(record.active_attempt.as_ref().map(|a| a.as_str()), Some(attempt.as_str()));
        // The retained brief carries the project instructions, the task text and the deliverable.
        let brief = lab.ok(&["memory", "demo", "attempt-brief", "--attempt", &attempt]).to_string();
        assert!(brief.contains("Shadow trial project") && brief.contains("Plan the next milestone") && brief.contains(&output), "{brief}");
        // The brief ends with the exact submission the worker must make; following
        // it records one submission bound to this attempt's contract.
        let brief_text = lab.ok(&["memory", "demo", "attempt-brief", "--attempt", &attempt])["text"].as_str().unwrap().to_owned();
        assert!(brief_text.contains("herdr-farm --root") && brief_text.contains("submission_id") && brief_text.contains("result demo submit"), "{brief_text}");
        let branch = format!("worker-{task}");
        let followed = lab.follow_brief(&attempt, &output, &branch);
        assert!(followed.status.success() && String::from_utf8_lossy(&followed.stdout).contains("submission_id"), "{}{}", String::from_utf8_lossy(&followed.stdout), String::from_utf8_lossy(&followed.stderr));
        let shown = lab.ok(&["result", "demo", "show"]);
        let mine: Vec<_> = shown.as_array().unwrap().iter().filter(|r| r["attempt_id"] == attempt.as_str()).collect();
        assert_eq!(mine.len(), 1, "{shown}");
        assert_eq!(mine[0]["task_id"], task);
        assert_eq!(mine[0]["artifact_manifest"][0]["path"], output.as_str(), "{shown}");
        assert_eq!(mine[0]["contract_revision"], 1);
        // Safe to repeat: finished steps are skipped and no second attempt appears.
        let again = lab.ok(&args);
        assert_eq!(again["attempt"], report["attempt"], "{again}");
        for step in again["steps"].as_array().unwrap() {
            if ["project_control", "task", "contract", "queue", "scheduler_capacity", "binding", "reserve"].contains(&step["step"].as_str().unwrap()) {
                assert_eq!(step["outcome"], "already_done", "{again}");
            }
        }
        assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap().attempts.len(), attempts.len() + 1);
        attempts.push(attempt);
    }
    assert_ne!(attempts[0], attempts[1]);
    let queue = lab.ok(&["scheduler", "demo", "inspect"]);
    assert_eq!(queue["policy"]["max_active_workers"], 2, "{queue}");
    // An explicit owner pause blocks another task even with workers retained.
    let before = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    lab.ok(&["runtime", "demo", "state", "paused", "--expected-head", &before.head.to_string(),
        "--expected-revision", &before.control.unwrap().revision.to_string()]);
    let args = lab.run_args("plan-third", "codex-sol", "docs/plan-third.md", prompt.to_str().unwrap());
    let error = lab.fail(&args);
    assert!(error.contains("explicitly paused"), "{error}");
    let after = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    assert_eq!(after.control.as_ref().unwrap().state, ProjectState::Paused);
    assert_eq!(after.runtime_bindings.len(), before.runtime_bindings.len());
}

/// A Claude profile whose pinned owner configuration names no setup-token file
/// is refused before any server starts or anything is
/// reserved, with a message that says what to configure, rather than launching
/// a worker that can only answer "login expired".
#[test]
fn a_claude_profile_without_a_setup_token_file_is_refused_by_verification_and_launch_run() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    let config = lab.home.join(".config/herdr-farm/config.toml");
    let token = lab.home.join("claude-setup-token");
    let with_token = fs::read_to_string(&config).unwrap();
    let without = with_token.replace(&format!("[worker_isolation.login]\nclaude_token_file={token:?}\n"), "");
    assert_ne!(with_token, without);
    fs::write(&config, &without).unwrap();
    lab.plant_launchable("claude-sonnet", "claude", "claude-sonnet-5-5");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone.").unwrap();
    let socket = lab.socket_inode_once("plan-claude.sock");
    let mut args: Vec<String> = lab.run_args("plan-claude", "claude-sonnet", "docs/plan-claude.md", prompt.to_str().unwrap()).into_iter().map(str::to_owned).collect();
    args.extend(["--herdr-socket".into(), socket.display().to_string()]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let before = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    let error = lab.fail(&args);
    assert!(error.contains("preflight") && error.contains("claude_token_file") && error.contains("claude setup-token"), "{error}");
    assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap(), before, "a refused run must not write");
    // Verification refuses the same profile before it starts any server.
    let (herdr, agent, home) = (lab.home.join("bin/herdr"), lab.home.join("bin/claude"), lab.home.join("agent-home-claude"));
    let verify = ["profile", "verify-interaction", "demo", "claude-sonnet", "--herdr-executable", herdr.to_str().unwrap(),
        "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap(), "--retain"];
    let error = lab.fail(&verify);
    assert!(error.contains("claude_token_file"), "{error}");
}

/// The owner edits the configuration after the project was made active: the next
/// `launch run` must re-acknowledge it (not report `project_control` as already
/// done and then fail at the contract with "owner configuration is not
/// acknowledged by project control").
#[test]
fn launch_run_reacknowledges_an_owner_configuration_edited_since_control_was_activated() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone of the tactics game.").unwrap();
    let prepare = |lab: &Lab, task: &str| {
        let socket = lab.socket_inode_once(&format!("{task}.sock"));
        let mut args: Vec<String> = lab.run_args(task, "codex-sol", &format!("docs/{task}.md"), prompt.to_str().unwrap()).into_iter().map(str::to_owned).collect();
        args.extend(["--herdr-socket".into(), socket.display().to_string(), "--prepare-only".into()]);
        lab.ok(&args.iter().map(String::as_str).collect::<Vec<_>>())
    };
    let step = |report: &Value, name: &str| report["steps"].as_array().unwrap().iter().find(|s| s["step"] == name).cloned().unwrap();
    let acknowledged = |lab: &Lab| herdr_farm::runtime::snapshot(&lab.project).unwrap().control.unwrap().config_digest;
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let first = prepare(&lab, "plan-one");
    assert_eq!(step(&first, "project_control")["outcome"], "done", "{first}");
    let before = acknowledged(&lab);
    assert!(before.is_some());

    // The owner edits the configuration; the profile is prepared against the new bytes.
    let config = lab.home.join(".config/herdr-farm/config.toml");
    let mut text = fs::read_to_string(&config).unwrap();
    text.push_str("\n# edited by the owner after the project was activated\n");
    fs::write(&config, text).unwrap();
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let second = prepare(&lab, "plan-two");
    let control = step(&second, "project_control");
    assert_eq!((control["outcome"].as_str(), control["detail"]["owner_configuration_reacknowledged"].as_bool()), (Some("done"), Some(true)), "{second}");
    assert_eq!(step(&second, "contract")["outcome"], "done", "{second}");
    let after = acknowledged(&lab);
    assert_ne!(after, before, "control acknowledges the edited configuration");
    assert_eq!(after, herdr_farm::migration::config_reference(&config).unwrap().digest);
    // Repeating changes nothing: the configuration is acknowledged now.
    let again = prepare(&lab, "plan-two");
    assert_eq!(step(&again, "project_control")["outcome"], "already_done", "{again}");
}

/// socket: explicit cancellation permits owner re-acknowledgment and a new attempt,
/// while an unobserved live worker still fences step 2.
#[test]
fn launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker() {
    struct FixtureTicker(std::process::Child);
    impl Drop for FixtureTicker {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    let lab = Lab::new();
    let owner = CoordinatorSession::start(&lab);
    // The canonical coordinator's public command surface: owner-configured
    // signing, explicit task creation, launch, then a real running attempt.
    let config=lab.home.join(".config/herdr-farm/config.toml");
    let mut text=fs::read_to_string(&config).unwrap();
    text.push_str(&format!("\n[coordinator]\nsigning_key={:?}\n",lab.key));
    fs::write(&config,text).unwrap();
    let head=herdr_farm::runtime::snapshot(&lab.project).unwrap().head.to_string();
    lab.ok(&["task","demo","add","plan-retry","--title","Coordinator plan","--expected-head",&head]);
    lab.verify("codex-sol", "codex");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone.").unwrap();
    let mut args = lab.run_args("plan-retry", "codex-sol", "docs/retry.md", prompt.to_str().unwrap());
    let key=args.iter().position(|a|*a=="--sign-with").unwrap();
    args.drain(key..key+2);
    let first = lab.ok(&args);
    let attempt = first["attempt"].as_str().unwrap();
    let mut ticker = FixtureTicker(Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", &lab.home)
        .env("HERDR_PROJECTS_OWNER_HOME", &lab.home).env("PATH", "/usr/bin:/bin")
        .env("HERDR_BIN_PATH", lab.home.join("bin/herdr"))
        .env("XDG_RUNTIME_DIR", lab.runtime.path())
        .args(["--root", lab.root.to_str().unwrap(), "ticker", "run"])
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
    let wait = |stage: &str, done: &dyn Fn() -> bool| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !done() {
            assert!(std::time::Instant::now() < until,
                "timed out waiting for {stage}\nattempt states: {:?}\nticker log:\n{}",
                herdr_farm::runtime::snapshot(&lab.project).map(|s| s.attempts),
                fs::read_to_string(lab.root.join(".ticker.log")).unwrap_or_default());
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    };
    wait("first attempt Running", &|| herdr_farm::runtime::snapshot(&lab.project).is_ok_and(|s|
        s.attempts.iter().any(|a| a.id.as_str() == attempt && a.state == AttemptState::Running)));
    let context=lab.cli(&["context","demo","--peek"]);
    assert!(context.status.success(),"{}",String::from_utf8_lossy(&context.stderr));
    let text=String::from_utf8(context.stdout).unwrap();
    assert!(text.contains(attempt) && text.contains("Running"),"{text}");
    // Stop only this fixture's ticker: the worker remains alive and its old
    // observation cannot acknowledge edited owner configuration.
    ticker.0.kill().unwrap();ticker.0.wait().unwrap();
    let config = lab.home.join(".config/herdr-farm/config.toml");
    let original = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("{original}\n# owner edit\n")).unwrap();
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let error = lab.fail(&args);
    assert!(error.contains("preflight") && error.contains("fresh resource identity evidence required"), "{error}");
    // Return to the launch's authorized config so the controller can stop it.
    fs::write(&config, &original).unwrap();
    let mut ticker = FixtureTicker(Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", &lab.home)
        .env("HERDR_PROJECTS_OWNER_HOME", &lab.home).env("PATH", "/usr/bin:/bin")
        .env("HERDR_BIN_PATH", lab.home.join("bin/herdr"))
        .env("XDG_RUNTIME_DIR", lab.runtime.path())
        .args(["--root", lab.root.to_str().unwrap(), "ticker", "run"])
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
    // The worker dies on its own, as the live trial's did (its budget ended it):
    // the attempt fails and the task is left blocked, not cancelled.
    let agent = fs::canonicalize(lab.home.join("bin/codex")).unwrap();
    let mut killed = 0;
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        if fs::read_link(entry.path().join("exe")).is_ok_and(|exe| exe == agent) {
            assert!(Command::new("/bin/kill").args(["-KILL", &pid.to_string()]).status().unwrap().success());
            killed += 1;
        }
    }
    assert_eq!(killed, 1, "exactly the fixture's worker agent is killed");
    wait("first attempt termination observed after the worker died", &|| herdr_farm::runtime::snapshot(&lab.project).is_ok_and(|s|
        s.attempts.iter().any(|a| a.id.as_str() == attempt && a.termination_observed)));
    wait("ticker closes the recorded viewer", &|| owner.calls().iter().any(|c|
        c["method"] == "tab.close" && c["params"]["tab_id"] == first["viewer"]["tab"]));
    assert!(owner.calls().iter().filter(|c| c["method"] == "tab.close")
        .all(|c| c["params"]["tab_id"] == first["viewer"]["tab"]));
    ticker.0.kill().unwrap();ticker.0.wait().unwrap();
    lab.ok(&["launch", "demo", "stop", "--task", "plan-retry"]);
    let old_worktree = PathBuf::from(first["worktree"].as_str().unwrap());
    assert!(old_worktree.is_dir());
    let ended = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    assert_eq!(ended.attempts.iter().find(|a| a.id.as_str() == attempt).unwrap().state, AttemptState::Failed);
    assert_eq!(ended.tasks.iter().find(|t| t.id.as_str() == "plan-retry").unwrap().state, TaskState::Blocked);
    fs::write(&config, format!("{original}\n# owner edit after termination\n")).unwrap();
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let second = lab.ok(&args);
    assert_ne!(second["attempt"], first["attempt"]);
    assert_ne!(second["worktree"], first["worktree"]);
    assert!(old_worktree.is_dir(), "old attempt worktree remains evidence");
    let state = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    assert_eq!(state.attempts.len(), 2);
    let binding = state.runtime_bindings.iter().find(|b| b.task.as_ref().is_some_and(|t| t.as_str() == "plan-retry")).unwrap();
    assert!(binding.identity.pane_id.is_empty() && binding.identity.worktree_path.is_empty(), "reservation uses the reset binding");
    assert!(state.attempts.iter().any(|a| a.id.as_str() == attempt && a.termination_observed));
    assert_eq!(state.tasks.iter().find(|t| t.id.as_str() == "plan-retry").unwrap().active_attempt.as_ref().unwrap().as_str(), second["attempt"].as_str().unwrap());
}

/// A control socket that could not be bound is refused before any server
/// starts, naming the path and its length, never as a vague "server exited".
#[test]
fn verify_interaction_refuses_a_too_long_socket_path_naming_path_and_length() {
    let mut lab = Lab::new();
    let long = lab.home.join("a-runtime-directory-name-that-is-deliberately-far-too-long-for-a-unix-socket-path-to-fit-in-sun-path");
    fs::create_dir(&long).unwrap();
    fs::set_permissions(&long, fs::Permissions::from_mode(0o700)).unwrap();
    lab.extra_env.push(("XDG_RUNTIME_DIR".into(), long.clone()));
    let (herdr, agent, home) = (lab.home.join("bin/herdr"), lab.home.join("bin/codex"), lab.home.join("agent-home-codex"));
    let error = lab.fail(&["profile", "verify-interaction", "demo", "codex-sol", "--herdr-executable", herdr.to_str().unwrap(),
        "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap(), "--retain"]);
    assert!(error.contains("Unix socket path") && error.contains(long.to_str().unwrap()) && error.contains("bytes; the limit is 107"), "{error}");
    assert!(fs::read_dir(&long).unwrap().next().is_none(), "no socket directory is left behind");
}

/// socket: the same operator sequence after real `verify-interaction`, with a
/// dedicated Herdr server started per task (the production shape).
#[test]
fn launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds() {
    let lab = Lab::new();
    lab.verify("codex-sol", "codex");
    lab.verify("claude-sonnet", "claude");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan the next milestone of the tactics game.").unwrap();
    let jobs = [("plan-codex", "codex-sol"), ("plan-claude", "claude-sonnet")];
    // Prepare both (each gets its own started Herdr server), then reserve both.
    for (task, profile) in jobs {
        let output = format!("docs/{task}.md");
        let mut args = lab.run_args(task, profile, &output, prompt.to_str().unwrap());
        args.push("--prepare-only");
        let report = lab.ok(&args);
        assert!(report["attempt"].is_null(), "{report}");
        assert_eq!(report["viewer"]["status"], "unavailable");
        assert!(Path::new(report["herdr_socket"].as_str().unwrap()).exists(), "{report}");
    }
    let mut attempts = Vec::new();
    for (task, profile) in jobs {
        let output = format!("docs/{task}.md");
        let out = lab.cli(&lab.run_args(task, profile, &output, prompt.to_str().unwrap()));
        let progress = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{progress}");
        assert!(progress.contains("worker wall budget 600 seconds"), "{progress}");
        let report: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(report["worker_wall_seconds"], 600, "{report}");
        let attempt = report["attempt"].as_str().unwrap().to_owned();
        assert!(report["worktree"].as_str().unwrap().contains(".state/worktrees"), "{report}");
        let again = lab.ok(&lab.run_args(task, profile, &output, prompt.to_str().unwrap()));
        assert_eq!(again["attempt"], report["attempt"], "{again}");
        attempts.push(attempt);
    }
    assert_ne!(attempts[0], attempts[1]);
    let state = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    assert_eq!(state.attempts.len(), 2);
    assert_eq!(state.control.unwrap().state, ProjectState::Active);

    // Each task's dedicated server is still running while its attempt holds the
    // worker, so `launch stop` refuses; with --force it stops the server and
    // removes its socket directory. Nothing is left running or on disk.
    let servers = || String::from_utf8(Command::new("/usr/bin/pgrep").args(["-f", &format!("{} server", lab.home.join("bin/herdr").display())]).output().unwrap().stdout).unwrap().lines().count();
    assert_eq!(servers(), 2, "one dedicated server per task");
    let refused = lab.fail(&["launch", "demo", "stop", "--task", "plan-codex"]);
    assert!(refused.contains("still holds its worker"), "{refused}");
    for (task, _) in jobs {
        let report = lab.ok(&["launch", "demo", "stop", "--task", task, "--force"]);
        assert_eq!((report["stopped"].as_bool(), report["socket_directory_removed"].as_bool()), (Some(true), Some(true)), "{report}");
        let again = lab.ok(&["launch", "demo", "stop", "--task", task, "--force"]);
        assert_eq!(again["stopped"], false, "stopping twice is harmless: {again}");
    }
    assert_eq!(servers(), 0, "no stand-in Herdr server is left running");
    let left: Vec<_> = fs::read_dir(lab.runtime.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(left.is_empty(), "no socket directory is left behind: {left:?}");
}

/// socket: agents that write into their execution home (`.codex/tmp/arg0` on
/// `--version`, a rollout during the session; `.claude.json`/`.claude/projects`)
/// must not fail verification; the retained evidence revalidates.
#[test]
fn verify_interaction_tolerates_agents_writing_into_their_execution_home() {
    let lab = Lab::new();
    lab.build_agent("codex", &writing_agent_source("codex-cli 0.159.2", ".codex/auth.json", ".codex/config.toml", ".codex/tmp", "arg0", ".codex/sessions/x.jsonl", ""));
    lab.build_agent("claude", &writing_agent_source("2.1.0 (Claude Code)", "", ".claude.json", ".claude/projects", "probe.jsonl", ".claude/projects/session.jsonl", ""));
    for (profile, kind) in [("codex-sol", "codex"), ("claude-sonnet", "claude")] {
        let report = lab.verify(profile, kind);
        assert_eq!(report["preparation"]["launchable"], true, "{kind}: {report}");
        let digest = report["preparation"]["reference"]["digest"].as_str().unwrap();
        // The home's contents changed (the agent wrote into it); revalidation still accepts it.
        let home = lab.home.join(format!("agent-home-{kind}"));
        assert!(if kind == "codex" { home.join(".codex/tmp/arg0").exists() } else { home.join(".claude/projects").exists() });
        let revalidated = lab.ok(&["profile", "revalidate", "demo", digest]);
        assert_eq!(revalidated["preparation"]["launchable"], true, "{kind}: {revalidated}");
    }
}

/// A home written to by the version probe is accepted by `profile prepare`.
#[test]
fn prepare_accepts_a_home_the_version_probe_wrote_into() {
    let lab = Lab::new();
    lab.build_agent("codex", &writing_agent_source("codex-cli 0.159.2", ".codex/auth.json", ".codex/config.toml", ".codex/tmp", "arg0", ".codex/sessions/x.jsonl", ""));
    let out = lab.prepare("codex-sol", "codex");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(lab.home.join("agent-home-codex/.codex/tmp/arg0").exists(), "the probe really wrote into the home");
}

/// A home swapped (renamed away and recreated at the same path) or made
/// group-writable while the agent is probed is still refused.
#[test]
fn prepare_refuses_a_swapped_or_repermissioned_execution_home() {
    let lab = Lab::new();
    let swap = r#"{ let h=std::path::Path::new(&home);let moved=format!("{home}.moved");std::fs::rename(h,&moved).unwrap();std::fs::create_dir(h).unwrap();std::fs::set_permissions(h,std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap(); }"#;
    lab.build_agent("codex", &writing_agent_source("codex-cli 0.159.2", ".codex/auth.json", ".codex/config.toml", ".codex/tmp", "arg0", ".codex/sessions/x.jsonl", swap));
    let out = lab.prepare("codex-sol", "codex");
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("execution home changed"), "{}", String::from_utf8_lossy(&out.stderr));
    fs::remove_dir_all(lab.home.join("agent-home-codex")).unwrap();
    fs::rename(lab.home.join("agent-home-codex.moved"), lab.home.join("agent-home-codex")).ok();
    fs::create_dir_all(lab.home.join("agent-home-codex")).unwrap();
    let chmod = r#"std::fs::set_permissions(&home,std::os::unix::fs::PermissionsExt::from_mode(0o770)).unwrap();"#;
    lab.build_agent("codex", &writing_agent_source("codex-cli 0.159.2", ".codex/auth.json", ".codex/config.toml", ".codex/tmp", "arg0", ".codex/sessions/x.jsonl", chmod));
    let out = lab.prepare("codex-sol", "codex");
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("execution home changed"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn code_launch_validates_scopes_and_outputs_before_writing() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    let base = ["launch", "demo", "run", "--task", "code", "--profile", "codex-sol", "--repository", lab.repo.to_str().unwrap()];
    let cases = [
        (vec!["--output", "src/lib.rs"], "--output requires --write"),
        (vec!["--write", "src/"], "name at least one file the result must contain"),
        (vec!["--write", "src/", "--output", "tests/test.rs"], "outside every --write path"),
        (vec!["--write", "src/*", "--output", "src/lib.rs"], "must not contain globs"),
        (vec!["--write", "../src/", "--output", "src/lib.rs"], "repo-relative"),
        (vec!["--write", "src/", "--output", "src/"], "exact file"),
        (vec!["--write", "src/", "--output", "src/1", "--output", "src/2", "--output", "src/3", "--output", "src/4", "--output", "src/5", "--output", "src/6", "--output", "src/7", "--output", "src/8", "--output", "src/9"], "at most 8 --output"),
    ];
    let before = herdr_farm::runtime::snapshot(&lab.project).unwrap();
    for (extra, message) in cases {
        let mut args = base.to_vec();
        args.extend(extra);
        let error = lab.fail(&args);
        assert!(error.contains(message), "{error}");
        assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap(), before);
    }
}

#[test]
fn code_launch_reserves_under_concurrent_writes_and_submits_all_scoped_changes() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let prompt = lab.home.join("code.txt");
    fs::write(&prompt, "Implement the code and its tests.").unwrap();
    let socket = lab.socket_inode_once("code.sock");
    let args = ["launch", "demo", "run", "--task", "code", "--profile", "codex-sol", "--repository", lab.repo.to_str().unwrap(),
        "--write", "./src//", "--write", "tests/", "--output", "src/lib.rs", "--prompt-file", prompt.to_str().unwrap(),
        "--sign-with", lab.key.to_str().unwrap(), "--herdr-socket", socket.to_str().unwrap()];
    let report = std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for n in 0..20 {
                if let Ok(snapshot) = herdr_farm::runtime::snapshot(&lab.project) {
                    let head = snapshot.head.to_string();
                    let _ = lab.cli(&["task", "demo", "add", &format!("other-{n}"), "--title", "Concurrent task", "--expected-head", &head]);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        let report = lab.ok(&args);
        writer.join().unwrap();
        report
    });
    assert!(herdr_farm::runtime::snapshot(&lab.project).unwrap().tasks.iter().any(|t| t.id.as_str().starts_with("other-")));
    let attempt = report["attempt"].as_str().unwrap();
    assert_eq!(report["write_paths"], serde_json::json!(["src/", "tests/"]));
    assert_eq!(report["outputs"], serde_json::json!(["src/lib.rs"]));
    let shown = lab.ok(&["task", "demo", "show", "code"]);
    let contract = &shown["contract"];
    assert_eq!(contract["scope"]["paths"], serde_json::json!([{"path":"src/","access":"write"},{"path":"tests/","access":"write"}]));
    assert_eq!(contract["outputs"], serde_json::json!([{"path":"src/lib.rs","kind":"git_file"}]));
    assert_eq!(contract["acceptance_policies"].as_array().unwrap().len(), 1);
    assert_eq!(contract["acceptance_policies"][0]["id"], "output-1");
    let brief = lab.ok(&["memory", "demo", "attempt-brief", "--attempt", attempt]);
    let script = brief["text"].as_str().unwrap().split("```sh\n").nth(1).unwrap().split("```").next().unwrap();
    let worktree = lab.home.join("code-worktree");
    lab.git(&["worktree", "add", "-q", "-b", "code-result", worktree.to_str().unwrap()]);
    for path in ["src/lib.rs", "tests/code.rs", "outside.txt"] {
        fs::create_dir_all(worktree.join(path).parent().unwrap()).unwrap();
        fs::write(worktree.join(path), "Content\n").unwrap();
    }
    let worker_git = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(&worktree).args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    };
    worker_git(&["add", "--", "outside.txt"]);
    worker_git(&["-c", "user.name=worker", "-c", "user.email=worker@invalid", "commit", "-q", "-m", "outside scope"]);
    let shim = lab.home.join("code-shim");
    fs::create_dir_all(&shim).unwrap();
    fs::write(shim.join("herdr-farm"), format!("#!/bin/sh\nunset HERDR_FARM_SUBMISSION_SPOOL\nexec {BIN} \"$@\"\n")).unwrap();
    fs::set_permissions(shim.join("herdr-farm"), fs::Permissions::from_mode(0o700)).unwrap();
    let submit = || Command::new("/bin/sh").args(["-c", script]).current_dir(&worktree).env_clear()
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .env("HOME", &lab.home).env("HERDR_PROJECTS_OWNER_HOME", &lab.home)
        .env("PATH", format!("{}:/usr/bin:/bin", shim.display())).env("XDG_RUNTIME_DIR", lab.runtime.path())
        .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("HERDR_FARM_SUBMISSION_SPOOL", lab.project.join(".state/spool").join(attempt)).output().unwrap();
    let refused = submit();
    assert!(!refused.status.success());
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(error.contains("outside.txt") && error.contains("Restore"), "{error}");
    fs::remove_file(worktree.join("outside.txt")).unwrap();
    worker_git(&["add", "-A", "--", "outside.txt"]);
    worker_git(&["-c", "user.name=worker", "-c", "user.email=worker@invalid", "commit", "-q", "-m", "restore outside scope"]);
    fs::remove_file(worktree.join("src/lib.rs")).unwrap();
    let missing = submit();
    assert!(!missing.status.success() && String::from_utf8_lossy(&missing.stderr).contains("declared output missing: src/lib.rs"));
    fs::write(worktree.join("src/lib.rs"), "Content\n").unwrap();
    let submitted = submit();
    assert!(submitted.status.success(), "{}", String::from_utf8_lossy(&submitted.stderr));
    assert!(String::from_utf8_lossy(&submitted.stdout).contains("submission_id"));
    let changed = Command::new("git").arg("-C").arg(&worktree).args(["diff", "--name-only", contract["base_oid"].as_str().unwrap(), "HEAD"]).output().unwrap();
    assert_eq!(String::from_utf8(changed.stdout).unwrap(), "src/lib.rs\ntests/code.rs\n");
}

#[test]
fn advanced_launch_fills_product_fields_and_refuses_identity_mismatches() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan.").unwrap();
    let socket = lab.socket_inode_once("advanced.sock");
    let mut planning = lab.run_args("template", "codex-sol", "docs/plan.md", prompt.to_str().unwrap());
    planning.extend(["--herdr-socket", socket.to_str().unwrap(), "--prepare-only"]);
    lab.ok(&planning);
    let mut contract = lab.ok(&["task", "demo", "show", "template"])["contract"].clone();
    for key in ["project_store", "expected_head", "contract_revision", "authority"] {
        contract.as_object_mut().unwrap().remove(key);
    }
    let file = lab.home.join("advanced.json");
    let args = ["launch", "demo", "run", "--task", "advanced", "--profile", "codex-sol", "--repository", lab.repo.to_str().unwrap(),
        "--contract-file", file.to_str().unwrap(), "--sign-with", lab.key.to_str().unwrap(), "--herdr-socket", socket.to_str().unwrap(), "--prepare-only"];
    fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let wrong = lab.fail(&args);
    assert!(wrong.contains("task_id must equal --task"), "{wrong}");
    contract["task_id"] = serde_json::json!("advanced");
    contract["profile_kind"] = serde_json::json!("claude");
    fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    assert!(lab.fail(&args).contains("profile_kind must equal"));
    contract["profile_kind"] = serde_json::json!("codex");
    fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    lab.ok(&args);
    let installed = lab.ok(&["task", "demo", "show", "advanced"])["contract"].clone();
    for (key, value) in contract.as_object().unwrap() {
        assert_eq!(&installed[key], value, "decision {key} changed");
    }
    assert_eq!(installed["contract_revision"], 1);
    assert!(installed["project_store"].is_string() && installed["authority"].is_object() && installed["expected_head"].is_number());
    contract["task_id"] = serde_json::json!("replacement");
    contract["project_store"] = serde_json::json!("/invalid/old-store");
    contract["expected_head"] = serde_json::json!(0);
    contract["contract_revision"] = serde_json::json!(99);
    contract["authority"] = serde_json::json!({"id":"stale","revision":99,"digest":"stale"});
    fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let mut replacement = args;
    replacement[4] = "replacement";
    lab.ok(&replacement);
    let replaced = lab.ok(&["task", "demo", "show", "replacement"])["contract"].clone();
    assert_eq!(replaced["contract_revision"], 1);
    assert_ne!(replaced["project_store"], contract["project_store"]);
    assert_ne!(replaced["expected_head"], contract["expected_head"]);
    assert_ne!(replaced["authority"], contract["authority"]);
}

struct CoordinatorSession {
    child: std::process::Child,
    socket: PathBuf,
    calls: PathBuf,
}

impl Drop for CoordinatorSession {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

impl CoordinatorSession {
    fn start(lab: &Lab) -> Self {
        let directory = lab.runtime.path().join("owner");
        fs::create_dir(&directory).unwrap();
        let socket = directory.join("s");
        // Fail directly with the sandbox's socket error, before spawning.
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        fs::remove_file(&socket).unwrap();
        let mut child = Command::new(lab.home.join("bin/herdr")).arg("server").env_clear()
            .env("HERDR_SOCKET_PATH", &socket)
            .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::os::unix::net::UnixStream::connect(&socket).is_err() {
            assert!(child.try_wait().unwrap().is_none(), "fixture session exited");
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let snapshot = herdr_farm::runtime::snapshot(&lab.project).unwrap();
        herdr_farm::runtime::create_binding(&lab.project, None, None, snapshot.head,
            &RuntimeRoute { socket: socket.display().to_string(), workspace_id: "owner-workspace".into(), ..Default::default() }).unwrap();
        Self { child, socket, calls: directory.join("calls.jsonl") }
    }

    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(&self.calls).unwrap_or_default().lines()
            .map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}

/// socket: viewer lifecycle through launch/view/stop and persisted server records.
#[test]
fn canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab() {
    let lab = Lab::new();
    let mut owner = CoordinatorSession::start(&lab);
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Plan.").unwrap();
    let mut args = lab.run_args("visible", "codex-sol", "docs/plan.md", prompt.to_str().unwrap());
    args.push("--prepare-only");
    let launched = lab.ok(&args);
    let viewer = &launched["viewer"];
    assert_eq!(viewer["socket"], owner.socket.display().to_string());
    assert_eq!(viewer["workspace"], "owner-workspace");
    assert_eq!(viewer["label"], "worker: visible");
    let config = PathBuf::from(viewer["config"].as_str().unwrap());
    assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o600);
    assert!(fs::read_to_string(&config).unwrap().contains("allow_nested = true"));
    let calls = owner.calls();
    assert_eq!(calls.iter().filter(|c| c["method"] == "tab.create").count(), 1);
    let command = calls.iter().find(|c| c["method"] == "pane.run").unwrap()["params"]["command"].to_string();
    assert!(command.contains(config.to_str().unwrap()));
    assert!(command.contains(launched["herdr_socket"].as_str().unwrap()));
    let focused = lab.ok(&["launch", "demo", "view", "--task", "visible"]);
    assert_eq!(focused["viewer"]["tab"], viewer["tab"]);
    assert_eq!(owner.calls().iter().filter(|c| c["method"] == "tab.create").count(), 1);
    assert!(owner.calls().iter().any(|c| c["method"] == "tab.focus" && c["params"]["tab_id"] == viewer["tab"]));
    // The owner closes the viewer; reopening must create a replacement.
    let output = Command::new(lab.home.join("bin/herdr")).args(["tab", "close", viewer["tab"].as_str().unwrap()])
        .env("HERDR_SOCKET_PATH", &owner.socket).output().unwrap();
    assert!(output.status.success());
    let reopened = lab.ok(&["launch", "demo", "view", "--task", "visible"]);
    assert!(reopened["viewer"]["tab"].is_string());
    assert_ne!(reopened["viewer"]["tab"], viewer["tab"]);
    lab.ok(&["launch", "demo", "stop", "--task", "visible"]);
    let closes: Vec<_> = owner.calls().into_iter().filter(|c| c["method"] == "tab.close").collect();
    assert_eq!(closes.len(), 2);
    assert!(closes.iter().all(|c| c["params"]["tab_id"] != "owner-tab"));
    assert_eq!(closes[1]["params"]["tab_id"], reopened["viewer"]["tab"]);
    lab.ok(&["launch", "demo", "stop", "--task", "visible"]);

    // Old records without a viewer can be reopened, then an owner-closed tab
    // is harmless during stop.
    lab.ok(&args);
    let path = lab.root.join(".herdr-run/demo-visible/herdr/server.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let old = record.as_object_mut().unwrap().remove("viewer").unwrap();
    let output = Command::new(lab.home.join("bin/herdr")).args(["tab", "close", old["tab"].as_str().unwrap()])
        .env("HERDR_SOCKET_PATH", &owner.socket).output().unwrap();
    assert!(output.status.success());
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let opened = lab.ok(&["launch", "demo", "view", "--task", "visible"]);
    let output = Command::new(lab.home.join("bin/herdr")).args(["tab", "close", opened["viewer"]["tab"].as_str().unwrap()])
        .env("HERDR_SOCKET_PATH", &owner.socket).output().unwrap();
    assert!(output.status.success());
    let before = owner.calls().iter().filter(|c| c["method"] == "tab.close").count();
    lab.ok(&["launch", "demo", "stop", "--task", "visible"]);
    assert_eq!(owner.calls().iter().filter(|c| c["method"] == "tab.close").count(), before);
    let again = lab.ok(&args);
    let output = Command::new(lab.home.join("bin/herdr"))
        .args(["tab", "rename", again["viewer"]["tab"].as_str().unwrap(), "Owner kept this"])
        .env("HERDR_SOCKET_PATH", &owner.socket).output().unwrap();
    assert!(output.status.success());
    lab.ok(&["launch", "demo", "stop", "--task", "visible"]);
    assert_eq!(owner.calls().iter().filter(|c| c["method"] == "tab.close").count(), before,
        "a tab whose label changed is never closed");
    owner.child.kill().unwrap();
    owner.child.wait().unwrap();
    let unavailable = lab.ok(&args);
    assert_eq!(unavailable["viewer"]["status"], "unavailable");
    assert!(unavailable["viewer"]["reason"].as_str().unwrap().contains("unreachable"));
    lab.ok(&["launch", "demo", "stop", "--task", "visible"]);
}

/// Discovery and policy refusals use the CLI and leave canonical state intact.
#[test]
fn automatic_signer_refusals_and_policy_limits_change_nothing() {
    for case in ["mismatch", "duplicate", "permissions", "repository", "cap", "passphrase"] {
        let lab = Lab::new();
        let config = lab.home.join(".config/herdr-farm/config.toml");
        if case == "cap" {
            let mut text=fs::read_to_string(&config).unwrap();
            text.push_str("\n[launch]\nmax_workers=1\n");
            fs::write(&config,text).unwrap();
        }
        if case == "passphrase" {
            fs::remove_file(&lab.key).unwrap();
            fs::remove_file(lab.key.with_extension("pub")).unwrap();
            assert!(Command::new("ssh-keygen").args(["-q","-t","ed25519","-N","secret","-f"]).arg(&lab.key).output().unwrap().status.success());
            let public=fs::read_to_string(lab.key.with_extension("pub")).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            let text=fs::read_to_string(&config).unwrap();
            let old=text.lines().find(|line|line.starts_with("approval_public_key=")).unwrap();
            fs::write(&config,text.replace(old,&format!("approval_public_key={public:?}"))).unwrap();
        }
        lab.plant_launchable("codex-sol","codex","gpt-6.1-sol");
        let socket=lab.socket_inode_once("automatic.socket");
        let args=["launch","demo","run","--task","auto","--profile","codex-sol","--repository",lab.repo.to_str().unwrap(),"--plan-output","docs/auto.md","--herdr-socket",socket.to_str().unwrap()];
        let expected=match case {
            "mismatch" => {fs::write(lab.key.with_extension("pub"),"ssh-ed25519 WRONG\n").unwrap(); "no signing key matching"},
            "duplicate" => {let second=lab.key.with_file_name("second");fs::copy(&lab.key,&second).unwrap();fs::copy(lab.key.with_extension("pub"),second.with_extension("pub")).unwrap(); "more than one signing key"},
            "permissions" => {fs::set_permissions(&lab.key,fs::Permissions::from_mode(0o640)).unwrap(); "no signing key matching"},
            "repository" => {fs::write(lab.project.join("PROJECT.md"),"+++\nrepos=[]\n+++\nTrial\n").unwrap(); "project repositories"},
            "cap" => {let report=lab.ok(&args); assert!(report["attempt"].is_string()); "owner cap"},
            "passphrase" => "the signing key needs a passphrase; load it into ssh-agent",
            _ => unreachable!(),
        };
        let before=herdr_farm::runtime::snapshot(&lab.project).unwrap();
        let started=std::time::Instant::now();
        let error=if case=="cap" {let mut second=args; second[4]="second";lab.fail(&second)} else {lab.fail(&args)};
        assert!(error.contains(expected),"{case}: {error}");
        if case=="passphrase" {assert!(started.elapsed()<std::time::Duration::from_secs(10),"passphrase probe must fail promptly");}
        assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap(),before,"{case}: refusal mutated state");
    }
}

#[test]
fn automatic_signer_reserves_and_defaults_to_owner_capacity() {
    let mut lab=Lab::new();
    lab.extra_env.push(("HERDR_BIN_PATH".into(), PathBuf::new()));
    lab.plant_launchable("codex-sol","codex","gpt-6.1-sol");
    let socket=lab.socket_inode_once("auto.socket");
    let args=["launch","demo","run","--task","auto","--profile","codex-sol","--repository",lab.repo.to_str().unwrap(),"--plan-output","docs/auto.md","--herdr-socket",socket.to_str().unwrap()];
    let report=lab.ok(&args);
    assert!(report["attempt"].is_string(),"{report}");
    assert_eq!(report["viewer"]["status"], "unavailable");
    assert_eq!(report["viewer"]["reason"], "workers run in the operator-supplied Herdr session");
    assert_eq!(herdr_farm::runtime::snapshot(&lab.project).unwrap().scheduler.unwrap().policy.max_active_workers,4);
    let again=lab.ok(&args);
    assert_eq!(again["attempt"],report["attempt"]);
    assert!(again["steps"].as_array().unwrap().iter().any(|s|s["step"]=="profile_evidence" && s["outcome"]=="already_done"),"{again}");
    let automation=herdr_farm::migration::open_active(&lab.project).unwrap().result_automation().unwrap();
    assert!(automation.verify);
    assert!(!automation.integrate);
}

/// socket: a configuration digest change is repaired without a separate command.
#[test]
fn stale_profile_evidence_is_refreshed_by_launch_run() {
    let lab=Lab::new();
    lab.verify("codex-sol","codex");
    let config=lab.home.join(".config/herdr-farm/config.toml");
    let mut text=fs::read_to_string(&config).unwrap();text.push_str("\n# owner comment\n");fs::write(config,text).unwrap();
    let socket=lab.socket_inode_once("refresh.socket");
    let report=lab.ok(&["launch","demo","run","--task","refresh","--profile","codex-sol","--repository",lab.repo.to_str().unwrap(),"--plan-output","docs/refresh.md","--herdr-socket",socket.to_str().unwrap()]);
    assert!(report["attempt"].is_string(),"{report}");
    assert!(report["steps"].as_array().unwrap().iter().any(|s|s["step"]=="profile_evidence" && s["outcome"]=="refreshed"),"{report}");
}

#[test]
fn contracted_launch_starts_at_an_unchecked_out_base_and_captures_only_worker_changes() {
    for mode in ["code", "planning", "advanced"] {
        let lab = Lab::with_herdr(r#"#!/usr/bin/python3
import json,sys
args=sys.argv[1:]
if args==['--version']:print('herdr 0.9.1')
elif args==['pane','list']:print('{"result":{"panes":[]}}')
elif args==['agent','list']:print('{"result":{"type":"agent_list","agents":[]}}')
elif args==['remote-api-bridge']:
 r=json.loads(sys.stdin.readline());assert r['method']=='ping'
 print(json.dumps({'id':r['id'],'result':{'type':'pong','version':'0.9.1','capabilities':{'workspace_create_command':True}}}))
else:sys.exit(3)
"#);
        lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
        lab.git(&["branch", "-M", "main"]);
        let owner = lab.git(&["rev-parse", "HEAD"]);
        lab.git(&["checkout", "-q", "-b", "contract-base"]);
        fs::write(lab.repo.join("base-only.txt"), "predecessor deliverable\n").unwrap();
        lab.git(&["add", "base-only.txt"]);
        lab.git(&["commit", "-q", "-m", "predecessor"]);
        let base = lab.git(&["rev-parse", "HEAD"]);
        lab.git(&["checkout", "-q", "main"]);
        let prompt = lab.home.join("prompt.txt");
        fs::write(&prompt, "Write the deliverable.").unwrap();
        let socket = lab.socket_inode_once("base.sock");
        let mut args = vec!["launch", "demo", "run", "--task", "base-worker", "--profile", "codex-sol",
            "--repository", lab.repo.to_str().unwrap(), "--base", "contract-base",
            "--prompt-file", prompt.to_str().unwrap(), "--sign-with", lab.key.to_str().unwrap(),
            "--herdr-socket", socket.to_str().unwrap()];
        let file = lab.home.join("advanced-base.json");
        if mode == "advanced" {
            let mut template = lab.run_args("template", "codex-sol", "docs/result.md", prompt.to_str().unwrap());
            template.extend(["--herdr-socket", socket.to_str().unwrap(), "--prepare-only"]);
            lab.ok(&template);
            let mut contract = lab.ok(&["task", "demo", "show", "template"])["contract"].clone();
            contract["task_id"] = serde_json::json!("base-worker");
            contract["base_oid"] = serde_json::json!(base);
            fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
            let index = args.iter().position(|a| *a == "--base").unwrap();
            args[index + 1] = "HEAD";
            args.extend(["--contract-file", file.to_str().unwrap()]);
        } else if mode == "planning" { args.extend(["--plan-output", "docs/result.md"]); }
        else { args.extend(["--write", "docs/result.md", "--output", "docs/result.md"]); }
        let report = lab.ok(&args);
        let attempt = report["attempt"].as_str().unwrap();
        let state = herdr_farm::runtime::snapshot(&lab.project).unwrap();
        let record = state.attempt_inputs.iter().find(|r| r.attempt.as_str() == attempt).unwrap();
        assert_eq!(record.inputs.repositories[0].commit, base);
        let receipts = herdr_farm::worktree_preparation::prepare(&lab.project, &record.operation, 1,
            std::time::Instant::now() + std::time::Duration::from_secs(45), Default::default()).unwrap();
        let worktree = PathBuf::from(&receipts[0].plan.path);
        let wt = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&worktree).args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        assert_eq!(wt(&["rev-parse", "HEAD"]), base);
        assert_eq!(fs::read_to_string(worktree.join("base-only.txt")).unwrap(), "predecessor deliverable\n");
        assert_eq!(lab.git(&["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(lab.git(&["rev-parse", "HEAD"]), owner);
        fs::create_dir_all(worktree.join("docs")).unwrap();
        fs::write(worktree.join("docs/result.md"), "Worker deliverable\n").unwrap();
        let captured = lab.ok(&["result", "demo", "capture", attempt]);
        assert_eq!(captured["base_oid"], base);
        assert_eq!(wt(&["diff", "--name-only", &base, "HEAD"]), "docs/result.md");
        lab.ok(&["result", "demo", "submit-captured", attempt]);
        assert_eq!(lab.git(&["rev-parse", "HEAD"]), owner);
        // Replacing the attempt branch with the owner's older history is refused,
        // without creating a capture commit or rewriting it back to the base.
        wt(&["reset", "--hard", &owner]);
        let error = lab.fail(&["result", "demo", "capture", attempt]);
        assert!(error.contains("history does not contain the contract base"), "{error}");
        assert_eq!(wt(&["rev-parse", "HEAD"]), owner);
    }
}

#[test]
fn an_unresolvable_advanced_contract_base_is_refused_before_reservation() {
    let lab = Lab::with_herdr(STATIC_HERDR);
    lab.plant_launchable("codex-sol", "codex", "gpt-6.1-sol");
    let prompt = lab.home.join("prompt.txt");
    fs::write(&prompt, "Write the deliverable.").unwrap();
    let socket = lab.socket_inode_once("missing-base.sock");
    let mut args = lab.run_args("template", "codex-sol", "docs/result.md", prompt.to_str().unwrap());
    args.extend(["--herdr-socket", socket.to_str().unwrap(), "--prepare-only"]);
    lab.ok(&args);
    let mut contract = lab.ok(&["task", "demo", "show", "template"])["contract"].clone();
    contract["task_id"] = serde_json::json!("missing-base");
    contract["base_oid"] = serde_json::json!("e".repeat(contract["base_oid"].as_str().unwrap().len()));
    let file = lab.home.join("missing-base.json");
    fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let error = lab.fail(&["launch", "demo", "run", "--task", "missing-base", "--profile", "codex-sol",
        "--repository", lab.repo.to_str().unwrap(), "--contract-file", file.to_str().unwrap(),
        "--prompt-file", prompt.to_str().unwrap(), "--sign-with", lab.key.to_str().unwrap(),
        "--herdr-socket", socket.to_str().unwrap()]);
    assert!(error.contains("base") || error.contains("commit"), "{error}");
    assert!(herdr_farm::runtime::snapshot(&lab.project).unwrap().attempts.is_empty());
}
