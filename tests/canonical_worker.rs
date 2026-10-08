#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)] // Test-only spawns outside the library may skip the spawn gate.
//! Canonical worker launch, brief and termination through the compiled CLI and
//! `ticker run`. A task is queued, drafted, owner-signed and reserved through
//! the CLI; the ticker then creates the worker on a Herdr stand-in server that
//! really runs the supervised command, briefs it and later stops it. The server
//! logs every request, so each test counts external effects from the outside.
use herdr_farm::{authority, domain::*, migration, operations::DeliveryState, runtime};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::{MetadataExt, PermissionsExt}, path::PathBuf, process::{Child, Command, Output, Stdio}, time::{Duration, Instant}};

mod inventory_history { include!("support/inventory_history.rs"); }
const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");

/// A Herdr server on `argv[1]` that runs `workspace.create_command` for real
/// and logs each request to `requests` beside the socket. `ping.json` there
/// replaces the ping reply; `lose-create` runs the command but drops the reply;
/// `drop-release` drops gate-release input unsent and unanswered; `vanish`
/// closes the worker's workspace, pane and agent without touching its process;
/// `swallow-prompts` is a count of prompts the agent ignores (it stays idle);
/// `reset-workspace` (consumed) forgets the ended worker so a lab can launch another.
const SERVER: &str = r#"
import json,os,sys,socket,subprocess
path=sys.argv[1];root=os.path.dirname(path);s={};gen=1
server=socket.socket(socket.AF_UNIX);server.bind(path);server.listen()
while True:
 c,_=server.accept();f=c.makefile('rw');r=json.loads(f.readline());m=r['method'];p=r.get('params') or {}
 if os.path.exists(os.path.join(root,'reset-workspace')):
  os.remove(os.path.join(root,'reset-workspace'));s.clear();gen+=1
  if os.path.exists(os.path.join(root,'input')):os.remove(os.path.join(root,'input'))
 live='pid' in s and not os.path.exists(os.path.join(root,'vanish'))
 with open(os.path.join(root,'requests'),'a') as log:log.write(json.dumps({'method':m,'params':p})+'\n')
 with open(os.path.join(root,'request-ids'),'a') as log:log.write(str(r['id'])+'\n')
 pane={'pane_id':f'w{gen}:p1','workspace_id':f'w{gen}','tab_id':f'w{gen}:t1','terminal_id':'term1','cwd':s.get('cwd')}
 if os.path.exists(os.path.join(root,'changed-terminal')):pane['terminal_id']='replacement-terminal'
 kind=open(os.path.join(root,'agent-kind')).read() if os.path.exists(os.path.join(root,'agent-kind')) else 'claude'
 status=open(os.path.join(root,'agent-status')).read() if os.path.exists(os.path.join(root,'agent-status')) else 'idle'
 if s.get('accepted') and not os.path.exists(os.path.join(root,'agent-status')):status='working'
 agent=dict(pane,agent=kind,interactive_ready=True,agent_status=status,**({'name':s['name']} if 'name' in s else {}))
 res=None
 if m=='ping':
  override=os.path.join(root,'ping.json')
  res=json.load(open(override)) if os.path.exists(override) else {'type':'pong','version':'0.9.1','capabilities':{'workspace_create_command':True}}
 elif m=='workspace.create_command' and 'pid' not in s:
  fifo=os.path.join(root,'input');os.mkfifo(fifo);fd=os.open(fifo,os.O_RDWR)
  child=subprocess.Popen(p['command'],cwd=p['cwd'],stdin=fd,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True,env={'PATH':'/usr/bin:/bin'})
  s.update(pid=child.pid,argv=p['command'],cwd=p['cwd'],label=p['label'],fifo=fifo)
  if not os.path.exists(os.path.join(root,'lose-create')):res={'type':'workspace_created','workspace':{'workspace_id':f'w{gen}'},'root_pane':{'pane_id':f'w{gen}:p1'}}
 elif m=='workspace.list':res={'type':'workspace_list','workspaces':[{'workspace_id':f'w{gen}','label':s['label'],'pane_count':1,'tab_count':1}] if live else []}
 elif m=='pane.list':res={'panes':[pane] if live else []}
 elif m=='pane.get':res={'pane':pane}
 elif m=='pane.report_metadata':res={'type':'ok'}
 elif m=='pane.process_info':res={'process_info':{'pane_id':f'w{gen}:p1','foreground_processes':[{'pid':s['pid'],'argv':s['argv']}] if live else []}}
 elif m=='pane.send_input' and os.path.exists(os.path.join(root,'drop-release')):pass
 elif m=='pane.send_input':
  fd=os.open(s['fifo'],os.O_WRONLY);os.write(fd,p['text'].encode());os.close(fd);s['released']=True;res={'type':'ok'}
 elif m=='agent.list':res={'type':'agent_list','agents':[agent] if live and s.get('released') else []}
 elif m=='agent.rename':s['name']=agent['name']=p['name'];res={'type':'agent_info','agent':agent}
 elif m=='agent.explain':res={'type':'agent_explain','explain':{'agent':kind,'state':'idle','manifest_source':open(os.path.join(root,'manifest-source')).read() if os.path.exists(os.path.join(root,'manifest-source')) else 'bundled','manifest_version':'2026.09.14.1',
  'matched_rule':{'id':'prompt','state':'idle'},'visible_idle':True,'visible_blocker':False,'visible_working':False,'screen_detection_skipped':False,
  'skip_state_update':False,'local_override_shadowing_remote':False,'fallback_reason':None,'warning':None}}
 elif m=='agent.prompt':
  # `swallow-prompts` holds how many prompts a startup banner eats before the agent takes one.
  s['prompts']=s.get('prompts',0)+1
  swallow=int(open(os.path.join(root,'swallow-prompts')).read()) if os.path.exists(os.path.join(root,'swallow-prompts')) else 0
  if s['prompts']>swallow:s['accepted']=True
  res={'type':'agent_prompted','agent':agent}
 if res is not None:f.write(json.dumps({'id':r['id'],'result':res})+'\n');f.flush()
 c.close()
"#;

struct Lab { home: tempfile::TempDir, project: PathBuf, key: PathBuf, repo: PathBuf, herdr: PathBuf, profile: VersionedReference, binding: String, server: Option<Child>,
    /// The `worker` profile's kind and its agent `bin/<kind>`.
    kind: &'static str }

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/pkill").args(["-KILL", "-f"]).arg(self.home.path().join("agent-home")).status();
        if let Some(server) = &mut self.server { let _ = server.kill(); let _ = server.wait(); }
    }
}

struct Ticker(Child);
impl Drop for Ticker { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

impl Lab {
    /// An active project with an owner key, a SHA-256 repository, the queued
    /// task `work` bound to the lab server, and a `worker` profile whose
    /// budget table is `budget`. The server runs only once `serve` is called.
    fn new(budget: &str) -> Self { Self::bound(budget, |repo| repo.to_owned()) }
    /// As `new`, with the binding's working directory `cwd(repository)`.
    fn bound(budget: &str, cwd: impl Fn(&std::path::Path) -> PathBuf) -> Self { Self::of_kind("claude", budget, cwd) }
    /// As `bound`, with a `worker` profile of `kind` (`claude` or `codex`).
    fn of_kind(kind: &'static str, budget: &str, cwd: impl Fn(&std::path::Path) -> PathBuf) -> Self {
        Self::of_kind_in(kind, budget, cwd, None)
    }
    fn of_kind_in(kind: &'static str, budget: &str, cwd: impl Fn(&std::path::Path) -> PathBuf, temp: Option<&str>) -> Self {
        let home = match temp {
            Some(path) => tempfile::Builder::new().prefix("hf-").tempdir_in(path).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let key = home.path().join("owner");
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).output().unwrap().status.success());
        let public = fs::read_to_string(key.with_extension("pub")).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let config = home.path().join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, format!("[authority]\nversion=1\nrevision=1\napproval_public_key={public:?}\n[profiles.worker]\nkind='{kind}'\npermission_policy='interactive'\n[profiles.worker.budget]\nmax_wall_seconds=600\n{budget}\n")).unwrap();
        for dir in ["repo", "bin", "agent-home", "lab"] { fs::create_dir(home.path().join(dir)).unwrap(); }
        let mut lab = Lab { project: home.path().join("root/demo"), key, repo: home.path().join("repo"), herdr: home.path().join("bin/herdr"),
            profile: VersionedReference { id: String::new(), revision: 1, digest: String::new() }, binding: String::new(), server: None, home, kind };
        fs::write(lab.path("lab/agent-kind"), kind).unwrap();
        lab.ok(&["new", "--legacy", "demo"]);
        lab.ok(&["pause", "demo"]);
        migration::apply(&lab.project, &migration::inspect_with_config(&lab.project, &config).unwrap(), true).unwrap();
        lab.git(&["init", "-q", "--object-format=sha256"]);
        lab.git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        lab.ok(&["task", "demo", "add", "work", "--title", "work", "--expected-head", &lab.head().to_string()]);
        let request = lab.path("queue.json");
        fs::write(&request, r#"{"priority":0,"dependencies":[]}"#).unwrap();
        lab.ok(&["task", "demo", "queue", "work", "--input-file", request.to_str().unwrap(), "--expected-revision", "1", "--expected-head", &lab.head().to_string()]);
        let policy = lab.state().scheduler.unwrap().policy.revision.to_string();
        lab.ok(&["scheduler", "demo", "policy", "--max-active-workers", "1", "--max-attempts-per-task", "3", "--expected-revision", &policy, "--expected-head", &lab.head().to_string()]);
        let route = RuntimeRoute { socket: lab.socket().display().to_string(), cwd: cwd(&lab.repo.canonicalize().unwrap()).display().to_string(), ..Default::default() };
        let id = TaskId::new("work").unwrap();
        let revision = runtime::snapshot(&lab.project).unwrap().tasks.into_iter().find(|t| t.id == id).unwrap().revision;
        let change = runtime::create_binding(&lab.project, Some(&id), Some(revision), lab.head(), &route).unwrap();
        lab.binding = change.binding.id;
        lab.resume();
        lab.write_binaries();
        lab
    }
    /// Record a fresh observation of every (resource-free) binding and set the project active.
    fn resume(&self) {
        let state = self.state();
        let config = self.path(".config/herdr-farm/config.toml");
        let observations = state.runtime_bindings.iter().map(|binding| herdr_farm::reconcile::RuntimeObservation { binding: binding.id.clone(), binding_revision: binding.revision,
            task_revision: binding.task.as_ref().map(|id| state.tasks.iter().find(|t| &t.id == id).unwrap().revision),
            observed_unix_ms: jiff::Timestamp::now().as_millisecond(), collector: "herdr-git-v2".into(),
            config_digest: migration::config_reference(&config).unwrap().digest.clone(), ..Default::default() }).collect::<Vec<_>>();
        migration::open_active(&self.project).unwrap().record_observations(state.head, &observations).unwrap();
        let control = self.state().control.unwrap().revision.to_string();
        self.ok(&["runtime", "demo", "state", "active", "--expected-revision", &control, "--expected-head", &self.head().to_string()]);
    }
    fn path(&self, name: &str) -> PathBuf { self.home.path().join(name) }
    fn socket(&self) -> PathBuf { self.path("lab/native.sock") }
    /// The lab agent executable, `bin/<kind>`.
    fn agent(&self) -> PathBuf { self.path(&format!("bin/{}", self.kind)) }
    /// What the lab agent prints for `--version`.
    fn version(&self) -> &'static str { if self.kind == "codex" { "codex-cli 0.154.0" } else { "2.1.0 (Claude Code)" } }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", self.home.path()).env("PATH", "/usr/bin:/bin")
            .args(["--root", self.path("root").to_str().unwrap()]).args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.cli(args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }
    /// `ok` against a running ticker. Its own writes can move the head, and its
    /// project turns can hold the lock, so the arguments are rebuilt from the
    /// current state and the command retried for a bounded time.
    fn ok_live(&self, args: &dyn Fn() -> Vec<String>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let args = args();
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            let out = self.cli(&args);
            if out.status.success() { return serde_json::from_slice(&out.stdout).unwrap_or(Value::Null); }
            assert!(Instant::now() < deadline, "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    /// A refused command writes nothing; returns its stderr.
    fn refused(&self, args: &[&str]) -> String {
        let before = self.state();
        let out = self.cli(args);
        assert!(!out.status.success(), "{args:?} accepted: {}", String::from_utf8_lossy(&out.stdout));
        assert_eq!(self.state(), before, "{args:?} was refused but wrote");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
    /// The store may be mid-publication under a running ticker; read again.
    fn state(&self) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop { match runtime::snapshot(&self.project) { Ok(s) => return s, Err(e) => { assert!(Instant::now() < deadline, "{e:#}"); std::thread::sleep(Duration::from_millis(20)); } } }
    }
    /// Since `before`, the ticker recorded runtime observations and nothing else.
    fn assert_only_observed(&self, before: &Snapshot) {
        let now = self.state();
        let written = now.events.iter().filter(|e| e.sequence > before.head && e.kind != "runtime.observed").map(|e| &e.kind).collect::<Vec<_>>();
        assert!(written.is_empty(), "{written:?}");
        assert_eq!((&now.tasks, &now.attempts, &now.operations, &now.deliveries, &now.approvals, &now.ownership, &now.control),
            (&before.tasks, &before.attempts, &before.operations, &before.deliveries, &before.approvals, &before.ownership, &before.control));
    }
    fn head(&self) -> u64 { self.state().head }
    fn events(&self, kind: &str) -> Vec<Event> { self.state().events.into_iter().filter(|e| e.kind == kind).collect() }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("/usr/bin/git").env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("PATH", "/usr/bin:/bin").env("HOME", self.home.path())
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "fixture").env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "fixture").env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .current_dir(&self.repo).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }
    /// A Herdr bridge to the lab server and a `claude` that stays up. The agent
    /// must be the exact executable it names, so build a tiny one.
    fn write_binaries(&self) {
        fs::write(&self.herdr, "#!/usr/bin/python3\nimport os,socket,sys,json\nif sys.argv[1:]==['--version']:print('herdr 0.9.1');sys.exit(0)\n\
probe={('pane','list'):'pane.list',('agent','list'):'agent.list'}.get(tuple(sys.argv[1:]))\nassert probe or sys.argv[1:]==['remote-api-bridge']\n\
line=json.dumps({'id':'probe','method':probe}).encode()+b'\\n' if probe else sys.stdin.buffer.readline()\n\
c=socket.socket(socket.AF_UNIX);c.connect(os.environ['HERDR_SOCKET_PATH']);c.sendall(line)\nreply=c.makefile('rb').readline()\nif not reply:sys.exit(1)\n\
sys.stdout.buffer.write(json.dumps({'result':json.loads(reply)['result']}).encode() if probe else reply)\n").unwrap();
        let (agent, source) = (self.agent(), self.path("bin/agent.rs"));
        fs::write(&source, format!("fn main(){{if std::env::args().nth(1).as_deref()==Some(\"--version\"){{println!({:?});return}}loop{{std::thread::park()}}}}", self.version())).unwrap();
        assert!(Command::new("rustc").args(["--edition", "2021", "-o"]).arg(&agent).arg(&source).status().unwrap().success());
        for path in [&self.herdr, &agent] { fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap(); }
    }
    /// Prepare `worker` over the lab binaries; only the native interaction
    /// evidence, which needs a real agent session, is planted.
    fn prepare_profile(&mut self) {
        use herdr_farm::worker_supervision::{ProcessIncarnation, SupervisorIdentity};
        let prepared = self.ok(&["profile", "prepare", "demo", "worker", "--herdr-executable", self.herdr.to_str().unwrap(),
            "--agent-executable", self.agent().to_str().unwrap(), "--execution-home", self.path("agent-home").to_str().unwrap()]);
        let mut profile: FrozenProfile = serde_json::from_value(prepared["profile"].clone()).unwrap();
        #[derive(serde::Serialize)] struct Interaction { session: ResourceIdentity, terminal: &'static str, readiness_manifest: &'static str, prompt_digest: String, acknowledged_unix_ms: i64 }
        #[derive(serde::Serialize)] struct Evidence { version: u32, prepared_profile: VersionedReference, supervisor: SupervisorIdentity, native_kind: String, observed_unix_ms: i64, stopped_unix_ms: i64, interaction: Interaction }
        let evidence = Evidence { version: 2, prepared_profile: profile.reference().unwrap(), native_kind: profile.kind.clone(), observed_unix_ms: 1000, stopped_unix_ms: 1001,
            supervisor: SupervisorIdentity { version: 1, boot_id: "00000000-0000-0000-0000-000000000001".into(), host_id: None, observer_namespace: (1, 2), worker_namespace: (1, 3),
                outer: ProcessIncarnation { pid: 20, device: 1, inode: 4 }, init: ProcessIncarnation { pid: 21, device: 1, inode: 5 } },
            interaction: Interaction { session: ResourceIdentity { device: 1, inode: 2, born_secs: 1, born_nanos: 0 }, terminal: "fixture-terminal", readiness_manifest: "fixture-manifest", prompt_digest: "a".repeat(64), acknowledged_unix_ms: 999 } };
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&evidence).unwrap()));
        let supported = CapabilityEvidence::Supported { evidence: VersionedReference { id: format!("native-transport-{hash}"), revision: 1, digest: hash } };
        let c = &mut profile.capabilities;
        (c.launch, c.stop, c.readiness_observation, c.prompt_submission) = (supported.clone(), supported.clone(), supported.clone(), supported);
        let reference = profile.reference().unwrap();
        let store = self.project.join(".state/state.db").canonicalize().unwrap();
        let metadata = fs::metadata(&store).unwrap();
        let report = json!({"preparation":{"profile":profile,"reference":reference,"launchable":true,"protocol_capable":false,"certified":false},
            "evidence":evidence,"source_store":[store,metadata.dev(),metadata.ino()]}).to_string();
        rusqlite::Connection::open(&store).unwrap().execute("INSERT INTO native_profiles(profile_digest,report,report_digest,sequence) VALUES(?1,?2,?3,(SELECT max(sequence) FROM events))",
            rusqlite::params![reference.digest, report, format!("{:x}", Sha256::digest(report.as_bytes()))]).unwrap();
        self.profile = reference;
    }
    /// Retain `instructions` as worker knowledge and write the launch selection.
    fn selection(&self, instructions: &str) -> PathBuf { self.selection_for("work", &self.binding, instructions) }
    /// As `selection`, for `task` on `binding`.
    fn selection_for(&self, task: &str, binding: &str, instructions: &str) -> PathBuf {
        fs::write(self.project.join("PROJECT.md"), instructions).unwrap();
        let scope = self.path("scope.json");
        fs::write(&scope, json!({"schema_version":1,"task_id":task,"profile":"worker","domains":[],"paths":[],"pinned_keys":[],"sensitivity":"default"}).to_string()).unwrap();
        let snapshot = self.ok(&["memory", "demo", "snapshot", "--task", task, "--profile", "worker", "--input-file", scope.to_str().unwrap(), "--worker"]);
        let selection = self.path("selection.json");
        fs::write(&selection, json!({"task":task,"binding":binding,"profile":self.profile,
            "knowledge":{"id":snapshot["id"],"revision":1,"digest":snapshot["manifest_hash"]},"repositories":[self.repo.canonicalize().unwrap()]}).to_string()).unwrap();
        selection
    }
    /// Draft, owner-sign, import and reserve a launch; returns (approval id, attempt).
    fn reserve(&mut self, instructions: &str) -> (String, AttemptId) {
        self.prepare_profile();
        let selection = self.selection(instructions);
        let drafted = self.ok(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &self.head().to_string()]);
        let document = self.path("approval.json");
        fs::write(&document, serde_json::to_vec_pretty(&drafted["approval"]).unwrap()).unwrap();
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&self.key).args(["-n", authority::SIGNATURE_NAMESPACE]).arg(&document).output().unwrap().status.success());
        let approval = self.ok(&["approval", "demo", "import", document.to_str().unwrap(), self.path("approval.json.sig").to_str().unwrap(), "--expected-head", &self.head().to_string()]);
        let reservation = self.ok(&["launch", "demo", "reserve", "--selection", selection.to_str().unwrap(), "--approval-digest", approval["digest"].as_str().unwrap(), "--expected-head", &self.head().to_string()]);
        let id = self.state().approvals.iter().find(|a| a.reference.digest == approval["digest"].as_str().unwrap()).unwrap().reference.id.clone();
        (id, AttemptId::new(reservation["record"]["attempt"].as_str().unwrap()).unwrap())
    }
    fn serve(&mut self) {
        self.server = Some(Command::new("/usr/bin/python3").args(["-c", SERVER]).arg(self.socket()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.socket().exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
    }
    /// Every request the server has received, as (method, params).
    fn requests(&self) -> Vec<(String, Value)> {
        fs::read_to_string(self.path("lab/requests")).unwrap_or_default().lines()
            .map(|line| { let v: Value = serde_json::from_str(line).unwrap(); (v["method"].as_str().unwrap().to_owned(), v["params"].clone()) }).collect()
    }
    fn count(&self, method: &str) -> usize { self.requests().iter().filter(|(m, _)| m == method).count() }
    fn attempt(&self, id: &AttemptId) -> Attempt { self.state().attempts.into_iter().find(|a| &a.id == id).unwrap() }
    fn spawn(&self) -> Ticker { self.spawn_attention_interval("300") }
    fn spawn_attention_interval(&self, interval: &str) -> Ticker { self.spawn_with_env(interval, &[]) }
    /// `spawn` with extra ticker environment (a product knob under test).
    fn spawn_with_env(&self, interval: &str, extra: &[(&str, &str)]) -> Ticker {
        let mut command = Command::new(BIN);
        command.env_clear();
        for (name, value) in extra { command.env(name, value); }
        Ticker(command.env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", self.home.path()).env("PATH", "/usr/bin:/bin").env("HERDR_BIN_PATH", &self.herdr).env("HERDR_FARM_TELEMETRY_COLLECT_SECS", interval)
            .args(["--root", self.path("root").to_str().unwrap(), "ticker", "run"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap())
    }
    fn wait(&self, ticker: &mut Ticker, seconds: u64, predicate: &dyn Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(seconds.max(15));
        // Predicates read `runtime::snapshot` (whole-store integrity check plus
        // a full read); at a fixed 20 ms they competed with the ticker being
        // waited on. Back off to 250 ms; the deadline is unchanged.
        let mut pause = Duration::from_millis(20);
        while !predicate() {
            assert!(ticker.0.try_wait().unwrap().is_none(), "ticker exited");
            assert!(Instant::now() < deadline, "{}", fs::read_to_string(self.path("root/.ticker.log")).unwrap_or_default());
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(250));
        }
    }
    /// `wait` that names the stage and reports the jobs, the attempt and the
    /// ticker log when it times out.
    fn wait_for(&self, ticker: &mut Ticker, stage: &str, attempt: &AttemptId, seconds: u64, predicate: &dyn Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(seconds.max(15));
        let mut pause = Duration::from_millis(20);
        while !predicate() {
            assert!(ticker.0.try_wait().unwrap().is_none(), "ticker exited while waiting for: {stage}");
            if Instant::now() >= deadline {
                let jobs = self.cli(&["result", "demo", "jobs"]);
                panic!("timed out after {seconds}s waiting for: {stage}\nattempt: {:?}\njobs: {}{}\nticker log:\n{}", self.attempt(attempt).state,
                    String::from_utf8_lossy(&jobs.stdout), String::from_utf8_lossy(&jobs.stderr), fs::read_to_string(self.path("root/.ticker.log")).unwrap_or_default());
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(250));
        }
    }
    fn stop(&self, mut ticker: Ticker) {
        let stop = self.path("root/.ticker.stop");
        fs::write(&stop, b"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while ticker.0.try_wait().unwrap().is_none() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
        fs::remove_file(&stop).unwrap();
    }
    /// Run a fresh ticker for `passes` completed passes (each pass republishes
    /// the executor metrics file), then stop it.
    fn run_passes(&self, passes: usize) {
        let metrics = self.path("root/.ticker-metrics.json");
        let inode = || fs::metadata(&metrics).map(|m| m.ino()).ok();
        let (mut last, mut seen) = (inode(), 0);
        let mut ticker = self.spawn();
        self.wait(&mut ticker, 60, &|| inode() != last);
        while seen < passes {
            self.wait(&mut ticker, 60, &|| inode() != last);
            (last, seen) = (inode(), seen + 1);
        }
        self.stop(ticker);
    }
    /// Run a fresh ticker until `done` holds, failing if `passes` passes
    /// complete first, then stop it. A pass publishes before the
    /// background work it admitted (an observation) finishes, and stopping
    /// the ticker cancels that work, so wait for its outcome, not a pass.
    fn run_until(&self, passes: usize, done: &dyn Fn() -> bool) {
        let metrics = self.path("root/.ticker-metrics.json");
        let inode = || fs::metadata(&metrics).map(|m| m.ino()).ok();
        let (mut last, mut seen) = (inode(), 0);
        // Accelerated passes are short, but native launch steps keep their
        // unscaled windows: the pass budget only expires after a real-time floor.
        let started = Instant::now();
        let mut ticker = self.spawn();
        while !done() {
            self.wait(&mut ticker, 60, &|| done() || inode() != last);
            if inode() != last && !done() {
                (last, seen) = (inode(), seen + 1);
                assert!(seen < passes || started.elapsed() < Duration::from_secs(90), "not done within {passes} passes: {}", fs::read_to_string(self.path("root/.ticker.log")).unwrap_or_default());
            }
        }
        self.stop(ticker);
    }
    /// Assert quiet behaviour across several completed controller passes.
    fn run_quiet(&self, passes: usize) { self.run_passes(passes); }
    fn run_for(&self, method: &str, passes: usize) {
        let seen = self.count(method);
        let mut ticker = self.spawn();
        self.wait(&mut ticker, 60, &|| self.count(method) >= seen + passes);
        self.stop(ticker);
    }
}

/// Replaces the unit tests `native_brief_submits_retained_bytes_once_and_commits_running_state`,
/// `native_resource_creation_records_gated_target_once_and_cancellation_retains_it`,
/// `controller_hints_rotate_brief_and_termination_without_granting_authority`,
/// `live_worker_without_cancellation_is_observed_without_being_stopped` and
/// `desired_stop_works_while_paused_and_revoked_and_retains_resources`.
#[test]
fn ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    fs::write(lab.repo.join(".gitignore"),".tools/\nignored-file\ntracked.txt\n").unwrap();
    fs::write(lab.repo.join("tracked.txt"),"tracked base").unwrap();
    lab.git(&["add","."]);lab.git(&["add","-f","tracked.txt"]);lab.git(&["commit","-q","-m","tracked work and ignore rules"]);
    let (approval, attempt) = lab.reserve("Retained instructions");
    // Instructions changed after approval are not what the worker is sent.
    fs::write(lab.project.join("PROJECT.md"), "Replacement must not be sent").unwrap();
    lab.serve();
    let briefed = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Confirmed)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &briefed);
    lab.stop(ticker);

    // One gated creation with the usage warning, one target in the created pane, one brief.
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    // The worker runs in its own namespace, killed with it, under the
    // profile's 600 s wall deadline, and ends by running the exact agent
    // executable.
    let argv: Vec<String> = serde_json::from_value(lab.requests().into_iter().find(|(m, _)| m == "workspace.create_command").unwrap().1["command"].clone()).unwrap();
    let agent = lab.path("bin/claude").display().to_string();
    assert!(argv.iter().any(|a| a == "--kill-child=KILL"), "{argv:?}");
    let wall = argv.iter().position(|a| a == "600s").unwrap_or_else(|| panic!("{argv:?}"));
    assert!(argv[..wall].ends_with(&["--".to_owned()]) && argv.last() == Some(&agent), "{argv:?}");
    // The native agent name is bounded and derived from the full attempt id.
    let names: Vec<String> = lab.requests().into_iter().filter(|(m, _)| m == "agent.rename").map(|(_, p)| p["name"].as_str().unwrap().to_owned()).collect();
    assert!(!names.is_empty() && names.iter().all(|n| n == &names[0]), "{names:?}");
    let name = &names[0];
    assert!(name.len() == 32 && name.as_bytes()[0].is_ascii_lowercase() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b)), "{name}");
    assert!(attempt.as_str().len() > 32 && !attempt.as_str().starts_with(name.as_str()), "{} {name}", attempt.as_str());
    let creation = &lab.events("runtime.launch_creation")[0];
    assert_eq!(creation.payload["usage_warning"], "provider_usage_unavailable");
    let target: LaunchTarget = serde_json::from_value(lab.events("runtime.launch_target")[0].payload.clone()).unwrap();
    assert_eq!((target.version, target.route.pane_id.as_str(), &target.attempt), (2, "w1:p1", &attempt));
    let text = lab.requests().into_iter().find(|(m, _)| m == "agent.prompt").unwrap().1["text"].as_str().unwrap().to_owned();
    assert!(text.contains("Retained instructions") && !text.contains("Replacement must not be sent"), "{text}");
    let state = lab.state();
    let brief = state.operations.iter().find(|o| o.kind == "runtime.worker_brief").unwrap();
    assert_eq!(brief.payload["prompt_digest"], format!("{:x}", Sha256::digest(text.as_bytes())));
    let running = lab.attempt(&attempt);
    assert_eq!((running.state, running.retains_capacity(), running.termination_observed), (AttemptState::Running, true, false));
    let started: LaunchStartedReceipt = serde_json::from_value(lab.events("runtime.launch_started")[0].payload.clone()).unwrap();
    let supervisor = started.supervisor.unwrap();

    // Later passes observe the live worker without stopping or briefing it again.
    let observed = lab.state();
    let listed = lab.count("agent.list");
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.count("agent.list") >= listed + 2);
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    assert_eq!(lab.attempt(&attempt), running);
    assert!(lab.events("runtime.worker_terminated").is_empty());
    assert_eq!((lab.state().tasks, lab.state().deliveries), (observed.tasks, observed.deliveries));
    assert!(!herdr_farm::worker_supervision::SupervisorObservation::recover_exited(&supervisor).unwrap(), "the worker is running");

    // The operator cancels the attempt, revokes its approval and pauses the
    // project while that same ticker keeps running. The ticker keeps its store
    // reads' connections across passes, and its next passes must still see
    // these writes.
    let worktree=lab.planned_worktree(&attempt);
    fs::write(worktree.join("tracked.txt"),b"tracked edits").unwrap();
    fs::write(worktree.join("partial.txt"),b"untracked work").unwrap();
    fs::write(worktree.join("ignored-file"),b"ignored bytes").unwrap();
    fs::create_dir(worktree.join(".tools")).unwrap();
    // Sparse file in this lab's TMPDIR: an excluded directory must never be
    // descended into, so its 218 MiB contents are not read by either scan.
    fs::File::create(worktree.join(".tools/toolchain")).unwrap().set_len(218*1024*1024).unwrap();
    let report = lab.project.join("REPORT.md");
    fs::write(&report, "retain this report").unwrap();
    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "operator stop"].map(String::from).to_vec());
    lab.ok_live(&|| ["approval", "demo", "revoke", &approval, "--expected-head", &lab.head().to_string(), "--reason", "stop execution"].map(String::from).to_vec());
    lab.ok_live(&|| { let s = lab.state(); ["runtime", "demo", "state", "paused", "--expected-revision", &s.control.unwrap().revision.to_string(), "--expected-head", &s.head.to_string()].map(String::from).to_vec() });
    let before = lab.state();
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let after = lab.state();
    assert!(herdr_farm::worker_supervision::SupervisorObservation::recover_exited(&supervisor).unwrap(), "the worker has exited");
    let stopped = lab.attempt(&attempt);
    assert_eq!((stopped.state, stopped.retains_capacity()), (AttemptState::Cancelled, false));
    assert_eq!((after.tasks[0].state, after.tasks[0].active_attempt.as_ref()), (TaskState::Cancelled, None));
    assert_eq!((&after.control, &after.ownership, &after.runtime_bindings), (&before.control, &before.ownership, &before.runtime_bindings));
    assert_eq!(fs::read_to_string(&report).unwrap(), "retain this report");
    assert_eq!(lab.events("runtime.worker_resources_retained").len(), 1);
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    let snapshots=herdr_farm::worktree_preservation::capture_stopped_files(&lab.project,&attempt,lab.head(),Instant::now()+Duration::from_secs(15),Default::default()).unwrap();
    let snapshot=&snapshots[0];
    for (path,bytes) in [("tracked.txt",b"tracked edits".as_slice()),("partial.txt",b"untracked work".as_slice())] {
        let entry=snapshot.manifest.entries.iter().find(|e|e.path==path).unwrap();
        assert_eq!(fs::read(snapshot.directory.join(&entry.sha256)).unwrap(),bytes);
    }
    assert!(snapshot.manifest.entries.iter().all(|e|!e.path.starts_with(".tools") && e.path!="ignored-file"));
    fs::File::create(worktree.join("oversized-work")).unwrap().set_len(51*1024*1024).unwrap();
    let error=herdr_farm::worktree_preservation::capture_stopped_files(&lab.project,&attempt,lab.head(),Instant::now()+Duration::from_secs(15),Default::default()).unwrap_err();
    assert!(error.to_string().contains("artifact source exceeds 50 MiB"),"{error:#}");
    assert_eq!(lab.attempt(&attempt),stopped);
    fs::remove_file(worktree.join("oversized-work")).unwrap();
    // A later pass has nothing left to stop.
    lab.run_for("agent.list", 1);
    lab.assert_only_observed(&after);
}


/// Replaces `lost_resource_creation_reply_retains_claim_and_never_creates_again`.
/// An acknowledged prompt is not an accepted one: a startup banner that eats the
/// typed text leaves the agent idle at an empty prompt. The brief is re-delivered
/// while the agent stays idle and confirmed only once it shows it took it.
#[test]
fn a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    fs::write(lab.path("lab/swallow-prompts"), "1").unwrap();
    lab.serve();
    let briefed = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Confirmed)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &briefed);
    lab.stop(ticker);
    assert_eq!(lab.count("agent.prompt"), 2, "the first prompt was swallowed, the second accepted");
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
}

/// A prompt the agent never takes is not confirmed: after the bounded
/// re-deliveries the operation is left ambiguous and the attempt is not running.
#[test]
fn a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    fs::write(lab.path("lab/swallow-prompts"), "99").unwrap();
    lab.serve();
    let ambiguous = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Ambiguous)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &ambiguous);
    lab.stop(ticker);
    assert_eq!(lab.count("agent.prompt"), 3, "bounded re-delivery");
    let state = lab.state();
    assert!(!state.deliveries.iter().any(|d| d.state == DeliveryState::Confirmed && state.operations.iter().any(|o| o.id == d.operation && o.kind == "runtime.worker_brief")), "never a false confirmation");
    assert_ne!(lab.attempt(&attempt).state, AttemptState::Running);
    // Nothing sends it a fourth time.
    lab.run_for("agent.list", 2);
    assert_eq!(lab.count("agent.prompt"), 3);
}

/// The permission mode is the profile's, set on every launch: a stale mode an
/// earlier run left in the execution home (Claude's `auto`) is replaced, other
/// settings stay.
#[test]
fn launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    lab.reserve("Retained instructions");
    let settings = lab.path("agent-home/.claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"permissions":{"defaultMode":"auto"},"env":{"DISABLE_AUTOUPDATER":"0"},"owner_extra":"kept"}"#).unwrap();
    lab.serve();
    let mut ticker = lab.spawn();
    // The home is prepared at the release gate, a later stage than workspace
    // creation: wait for the brief, which is delivered only after release.
    lab.wait(&mut ticker, 60, &|| lab.count("agent.prompt") >= 1);
    lab.stop(ticker);
    let written: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(written["permissions"]["defaultMode"], "acceptEdits", "{written}");
    assert_eq!(written["env"]["DISABLE_AUTOUPDATER"], "1", "{written}");
    assert_eq!(written["owner_extra"], "kept", "{written}");
}

/// `launch run` records the dedicated Herdr server it starts for a task; once the
/// task's worker has terminated the ticker stops that server and removes its
/// socket directory.
#[test]
fn ticker_stops_the_dedicated_herdr_server_of_a_finished_task() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let launched = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Confirmed)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &launched);
    // The server `launch run` would have started: a `server` process whose
    // environment names its socket, recorded beside the run's logs.
    let sockets = lab.path("run-sockets");
    fs::create_dir_all(sockets.join("rdedicated")).unwrap();
    let socket = sockets.join("rdedicated/s");
    fs::write(&socket, b"").unwrap();
    let mut server = Command::new("/usr/bin/python3").args(["-c", "import time; time.sleep(300)", "server"]).env("HERDR_SOCKET_PATH", &socket).spawn().unwrap();
    let run = lab.path("root/.herdr-run");
    let dir = run.join("demo-work/herdr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("server.json"), json!({"pid": server.id(), "socket": socket}).to_string()).unwrap();
    // While the attempt holds its worker nothing is stopped.
    lab.wait(&mut ticker, 60, &|| lab.count("agent.list") >= 3);
    assert!(server.try_wait().unwrap().is_none() && socket.exists());
    let server_record = fs::read(dir.join("server.json")).unwrap();
    fs::remove_file(dir.join("server.json")).unwrap();
    // The operator cancels the attempt; the ticker proves the worker's termination and then the server goes.
    let running = lab.attempt(&attempt);
    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "finished"].map(String::from).to_vec());
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    assert!(server.try_wait().unwrap().is_none());
    fs::write(dir.join("server.json"), server_record).unwrap();
    // A fresh ticker has no in-memory knowledge of this ended launch.
    let ticker = lab.spawn();
    let until = Instant::now() + Duration::from_secs(15);
    while server.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until, "the ticker never stopped the finished task's server: {}", fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(100));
    }
    lab.stop(ticker);
    assert!(!socket.exists() && !sockets.join("rdedicated").exists(), "its socket directory is removed");
    assert!(!run.join("demo-work/herdr/server.json").exists());
}

/// A worker that edits its deliverable and never submits. `herdr-farm result
/// DEMO submit-captured ATTEMPT` captures the worktree, builds the submission
/// from the capture (frozen contract revision and digest, base and candidate,
/// the declared output's blob) and records it; with verification and
/// integration automatic the ticker then verifies and integrates it, and
/// repeating the command replays.
const EDITING_AGENT: &str = r#"
use std::{fs, time::Duration};
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    fs::write("work.txt", "worker change\n").unwrap();
    loop { std::thread::sleep(Duration::from_secs(1)) }
}
"#;

#[test]
fn an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'\n");
    lab.write_agent(EDITING_AGENT, &[]);
    // The base holds a subtree the worker leaves untouched: the verifier needs
    // it too, since it rebuilds the base and the candidate from staged objects.
    fs::create_dir_all(lab.repo.join("docs/plan")).unwrap();
    fs::write(lab.repo.join("docs/plan/00-readme.md"), "the plan\n").unwrap();
    for index in 0..200 {
        let dir = lab.repo.join(format!("docs/nested/{}", index / 10));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("file-{index}.txt")), format!("base file {index}\n")).unwrap();
    }
    lab.git(&["add", "docs"]);
    lab.git(&["commit", "-q", "-m", "plan"]);
    let (contract, base) = lab.install_work_contract("verify_then_integrate");
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    let repository = lab.repo.canonicalize().unwrap().display().to_string();
    lab.git(&["branch", "integration"]);
    lab.ok(&["result", "demo", "configure-integration", "--repository", &repository, "--reference", "refs/heads/integration"]);
    lab.ok(&["result", "demo", "auto", "--verify", "on", "--integrate", "on", "--expected-head", &lab.head().to_string()]);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait_for(&mut ticker, "the worker to launch and edit work.txt", &attempt, 120, &|| fs::read_to_string(worktree.join("work.txt")).is_ok_and(|t| t == "worker change\n"));
    assert_eq!(lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec()), json!([]), "the worker never submitted");

    let report_dir=lab.project.join(".state/worker-output").join(attempt.as_str());
    fs::create_dir_all(&report_dir).unwrap();
    fs::write(report_dir.join("report.md"),"## Results\nEdited output\n\n## Remember\nThe verifier needs the unchanged base closure.\n").unwrap();
    let done = lab.ok_live(&|| ["result", "demo", "submit-captured", attempt.as_str()].map(String::from).to_vec());
    let candidate = done["capture"]["candidate_oid"].as_str().unwrap().to_owned();
    assert_eq!((done["capture"]["base_oid"].as_str(), done["contract_revision"].as_u64(), done["contract_digest"].as_str(), done["submission"]["replayed"].as_bool()),
        (Some(base.as_str()), Some(1), Some(contract.as_str()), Some(false)), "{done}");
    let blob = lab.git(&["rev-parse", &format!("{candidate}:work.txt")]);
    assert_eq!(done["artifacts"], json!([["work.txt", blob]]), "{done}");
    let shown = lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec());
    assert_eq!(shown.as_array().map(|a| (a.len(), a[0]["candidate_oid"].clone(), a[0]["attempt_id"].clone(), a[0]["artifact_manifest"].clone())),
        Some((1, json!(candidate), json!(attempt.as_str()), json!([{"path": "work.txt", "oid": blob}]))), "{shown}");
    // Repeating the command changes nothing.
    let again = lab.ok_live(&|| ["result", "demo", "submit-captured", attempt.as_str()].map(String::from).to_vec());
    assert_eq!((again["submission"]["replayed"].as_bool(), again["capture"]["captured"].as_bool(), again["submission"]["submission_id"].clone()), (Some(true), Some(false), done["submission"]["submission_id"].clone()), "{again}");
    assert_eq!(lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec()).as_array().unwrap().len(), 1);
    let candidates=lab.ok_live(&|| ["memory","demo","list"].map(String::from).to_vec());
    assert_eq!(candidates.as_array().unwrap().len(),1);
    assert_eq!(candidates[0]["remember"],"The verifier needs the unchanged base closure.");
    assert_eq!(candidates[0]["submission_id"],done["submission"]["submission_id"]);
    assert_eq!(lab.state().inbox.iter().filter(|i|i.content.kind=="memory.candidate_proposed").count(),1);
    // The submission enters automatic verification at once, while the worker is still running.
    let jobs = lab.ok_live(&|| ["result", "demo", "jobs"].map(String::from).to_vec());
    assert!(jobs.as_array().is_some_and(|jobs| jobs.iter().any(|job| job["kind"] == "verification.run" && job["submission_id"] == done["submission"]["submission_id"])), "no verification job after submit-captured: {jobs}\n{:?}", lab.attempt(&attempt).state);
    // Automatic verification and integration take it from here.
    lab.wait_for(&mut ticker, "the verified candidate to be integrated", &attempt, 120, &|| lab.git_ok(&["rev-parse", "--verify", "-q", "integration^2"]));
    lab.stop(ticker);
    assert_eq!(lab.git(&["rev-parse", "integration^2"]), candidate);
    assert_eq!(lab.git(&["cat-file", "-p", &format!("{candidate}:work.txt")]), "worker change");
}

// Like EDITING_AGENT, but commits and submits through the worker's public
// CLI/spool, then stays idle. An owner marker asks it to resubmit a new commit.
const SUBMITTING_EDITING_AGENT: &str = r#"
use std::{fs, path::Path, process::Command, time::Duration};
fn git(args: &[&str]) -> String {
    let out = Command::new("/usr/bin/git").args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    let spool = std::path::PathBuf::from(std::env::var("HERDR_FARM_SUBMISSION_SPOOL").unwrap());
    let attempt = spool.file_name().unwrap().to_str().unwrap();
    let output = std::path::PathBuf::from(std::env::var("HERDR_FARM_WORKER_OUTPUT").unwrap());
    assert_eq!(output, spool.parent().unwrap().parent().unwrap().join("worker-output").join(attempt));
    assert!(output.is_dir());
    for (key, fallback, names) in [
        ("user.name", format!("herdr-farm worker {attempt}"), ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"]),
        ("user.email", "worker@herdr-farm.invalid".into(), ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"]),
    ] {
        let configured = Command::new("/usr/bin/git").args(["config", "--get", key]).output().unwrap();
        let expected = if configured.status.success() { String::from_utf8(configured.stdout).unwrap().trim().to_owned() } else { fallback };
        for name in names { assert_eq!(std::env::var(name).unwrap(), expected); }
    }
    fs::write(output.join("report.md"), "Worker environment and Git commit verified\n").unwrap();
    for index in 0..2 {
        if index == 1 { while !Path::new("resubmit").exists() { std::thread::sleep(Duration::from_millis(50)); } }
        fs::write("work.txt", format!("worker change {index}\n")).unwrap();
        git(&["add", "work.txt"]); git(&["commit", "-qm", "worker change"]);
        let candidate = git(&["rev-parse", "HEAD"]);
        let blob = git(&["rev-parse", "HEAD:work.txt"]);
        let objects = git(&["rev-list", "--objects", "HEAD"]).lines().map(|line| {
            let oid = line.split_whitespace().next().unwrap();
            format!("{{\"oid\":\"{oid}\",\"relative_path\":\"{}/{}\"}}", &oid[..2], &oid[2..])
        }).collect::<Vec<_>>().join(",");
        let template = fs::read_to_string(TEMPLATE_PATH).unwrap();
        let document = template.replace("CANDIDATE", &candidate).replace("BLOB", &blob)
            .replace("\"OBJECTS\"", &format!("[{objects}]")).replace("KEY", &format!("editing-{index}"));
        fs::write("submission.json", document).unwrap();
        let mut submitted = false;
        for _ in 0..200 {
            let out = Command::new("herdr-farm").args(["--root", ROOT, "result", "demo", "submit", "--input-file", "submission.json"]).output().unwrap();
            if out.status.success() { submitted = true; break }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(submitted);
        fs::write(format!("submitted-{index}"), "ok").unwrap();
    }
    loop { std::thread::park() }
}
"#;

fn editing_submission_lab(route: &str, policy: &str, verify: bool, integrate: bool) -> (Lab, AttemptId, PathBuf) {
    editing_submission_lab_with(route, policy, verify, integrate, SUBMITTING_EDITING_AGENT)
}
/// As `editing_submission_lab`, with the worker built from `agent`.
fn editing_submission_lab_with(route: &str, policy: &str, verify: bool, integrate: bool, agent: &str) -> (Lab, AttemptId, PathBuf) {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'\n");
    if !integrate {
        lab.git(&["config", "user.name", "Repository worker"]);
        lab.git(&["config", "user.email", "repository@example.invalid"]);
    }
    let (contract, base) = lab.install_work_contract_policy(route, policy);
    // Reservation freezes the executable digest. Supply attempt-specific data
    // through a fixture file instead of rebuilding the worker afterward.
    let template_path = lab.path("submission-template.json");
    lab.write_agent(agent, &[("ROOT", lab.path("root").canonicalize().unwrap().display().to_string()), ("TEMPLATE_PATH", template_path.display().to_string())]);
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    let repository = lab.repo.canonicalize().unwrap().display().to_string();
    lab.git(&["branch", "integration"]);
    lab.ok(&["result", "demo", "configure-integration", "--repository", &repository, "--reference", "refs/heads/integration"]);
    lab.ok(&["result", "demo", "auto", "--verify", if verify {"on"} else {"off"}, "--integrate", if integrate {"on"} else {"off"}, "--expected-head", &lab.head().to_string()]);
    let template = json!({"idempotency_key":"KEY", "task_id":"work", "contract_revision":1, "contract_digest":contract,
        "attempt_id":attempt.as_str(), "repository":repository, "base_oid":base, "candidate_oid":"CANDIDATE", "object_format":"sha256",
        "artifact_manifest":[{"path":"work.txt", "oid":"BLOB"}], "claimed_checks":[], "objects":"OBJECTS"}).to_string();
    fs::write(template_path, template).unwrap();
    (lab, attempt, worktree)
}

#[test]
fn accepted_editing_worker_completes_automatically_after_integration() {
    let (mut lab, attempt, _) = editing_submission_lab("verify_then_integrate", WORK_POLICY, true, true);
    let mut waiter = Command::new(BIN).env_clear()
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .env("HOME", lab.home.path()).env("PATH", "/usr/bin:/bin")
        .args(["--root", lab.path("root").to_str().unwrap(), "inbox", "demo", "wait", "--timeout", "60"])
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    assert!(waiter.try_wait().unwrap().is_none());
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait_for(&mut ticker, "automatic completion and proven worker termination", &attempt, 120, &|| lab.attempt(&attempt).termination_observed);
    let ended = lab.attempt(&attempt);
    assert_eq!(ended.state, AttemptState::Completed);
    assert!(!ended.retains_capacity());
    assert_eq!(lab.state().tasks.iter().find(|t| t.id.as_str() == "work").unwrap().state, TaskState::Succeeded);
    assert!(lab.git_ok(&["rev-parse", "--verify", "integration^2"]));
    assert_eq!(lab.events("runtime.worker_terminated").len(), 1);
    assert_eq!(lab.events("attempt.completion_requested").len(), 1);
    let report = lab.ok_live(&|| ["telemetry", "demo", "attempts", "--json"].map(String::from).to_vec());
    let outcome = report["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == attempt.as_str()).unwrap();
    assert_eq!(outcome["terminal_state"], "completed", "{outcome}");
    assert_eq!(outcome["accepted"], true, "{outcome}");
    assert!(outcome["active_ms"].as_i64().is_some_and(|ms| ms < 600_000), "{outcome}");
    let replay = lab.ok_live(&|| ["task", "demo", "complete", "work", "--expected-revision", "0"].map(String::from).to_vec());
    assert_eq!(replay["replayed"], true);
    lab.stop(ticker);
    let output = waiter.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let wake: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = lab.ok(&["inbox", "list", "demo"]);
    let notices: Vec<_> = items.as_array().unwrap().iter().filter(|i| i["content"]["id"].as_str().unwrap().starts_with("worker-result-")).collect();
    for kind in ["result.submitted", "verification.accepted", "integration.succeeded"] {
        let item = notices.iter().find(|i| i["content"]["kind"] == kind).unwrap();
        let summary = item["content"]["summary"].as_str().unwrap();
        assert!(summary.contains("task work") && summary.contains(attempt.as_str()), "{summary}");
    }
    assert!(wake["items"].as_u64().unwrap() > 0, "{wake}");
    for id in wake["ids"].as_array().unwrap() {
        assert!(items.as_array().unwrap().iter().any(|i| i["content"]["id"] == *id && i["seen"] == false && i["done"] == false));
    }
    // The lab writes bare instructions into PROJECT.md; context needs the settings header.
    let instructions = fs::read_to_string(lab.project.join("PROJECT.md")).unwrap();
    fs::write(lab.project.join("PROJECT.md"), format!("+++\nname = \"demo\"\n+++\n{instructions}")).unwrap();
    let context = lab.cli(&["context", "demo"]);
    assert!(context.status.success(), "{}", String::from_utf8_lossy(&context.stderr));
    let text = String::from_utf8(context.stdout).unwrap();
    for item in &notices { assert!(text.contains(item["content"]["id"].as_str().unwrap())); }
    assert_eq!(lab.ok(&["inbox", "demo", "wait", "--timeout", "1"]), json!({"items":0,"timed_out":true}));
    lab.ok(&["inbox", "done", "demo", "--all"]);
    lab.run_quiet(5);
    let replayed = lab.ok(&["inbox", "list", "demo"]);
    assert_eq!(replayed.as_array().unwrap().iter().filter(|i| i["content"]["id"].as_str().unwrap().starts_with("worker-result-")).count(), notices.len());
    assert_eq!(lab.events("attempt.completion_requested").len(), 1);
    assert_eq!(lab.events("runtime.worker_terminated").len(), 1);
}

#[test]
fn accepted_verify_only_editing_worker_completes_without_integration_automation() {
    let (mut lab, attempt, _) = editing_submission_lab("verify_only", WORK_POLICY, true, false);
    lab.ok(&["telemetry", "demo", "collect"]);
    lab.serve();
    let mut ticker = lab.spawn_attention_interval("3600");
    lab.wait_for(&mut ticker, "verify-only automatic completion", &attempt, 120, &|| lab.attempt(&attempt).termination_observed);
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Completed);
    assert!(!lab.attempt(&attempt).retains_capacity());
    assert_eq!(lab.state().tasks.iter().find(|t| t.id.as_str() == "work").unwrap().state, TaskState::Succeeded);
    assert!(!lab.git_ok(&["rev-parse", "--verify", "-q", "integration^2"]));
    lab.stop(ticker);
    let db = rusqlite::Connection::open(lab.project.join(".state/telemetry.db")).unwrap();
    let samples: Vec<(i64, Option<String>, Option<String>)> = db.prepare("SELECT observed_unix_ms,state,gap FROM attention_samples WHERE attempt_id=?1 ORDER BY rowid").unwrap()
        .query_map([attempt.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
    assert!(!samples.is_empty(), "running observation: {samples:?}");
    assert!(samples.iter().all(|(_, state, gap)| state.is_some() && gap.is_none()), "{samples:?}");
    let canonical = rusqlite::Connection::open(lab.project.join(".state/state.db")).unwrap();
    let ended: i64 = canonical.query_row("SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=?1 AND state='completed'", [attempt.as_str()], |r| r.get(0)).unwrap();
    let interval: i64 = db.query_row("SELECT interval_ms FROM attention_samples WHERE attempt_id=?1 ORDER BY observed_unix_ms LIMIT 1", [attempt.as_str()], |r| r.get(0)).unwrap();
    assert!(ended - samples[0].0 < 2 * interval);
    let m31 = lab.ok(&["telemetry", "demo", "report", "--json"])["metrics"]["M31"].clone();
    assert_eq!(m31["coverage"], json!({"attempts": 1, "complete": 1, "not_observed": 0, "with_gaps": 0}));
    assert_eq!(m31["value"], "0/1");
}

#[test]
fn rejected_editing_worker_stays_running_and_can_resubmit() {
    let (mut lab, attempt, worktree) = editing_submission_lab("verify_only", r#"{"version":1,"checks":["/usr/bin/false"]}"#, true, false);
    herdr_farm::telemetry::sidecar::open(&lab.project, true).unwrap().unwrap();
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait_for(&mut ticker, "rejected verification", &attempt, 120, &|| {
        let report = herdr_farm::telemetry::outcome::attempts(&lab.project).unwrap();
        report["attempts"].as_array().unwrap().iter().any(|a| a["attempt_id"] == attempt.as_str() && a["verification"]["state"] == "rejected")
    });
    let items = lab.ok_live(&|| ["inbox", "list", "demo"].map(String::from).to_vec());
    let rejection = items.as_array().unwrap().iter().find(|i| i["content"]["kind"] == "verification.rejected").unwrap();
    let feedback = lab.ok_live(&|| ["feedback", "demo", "show"].map(String::from).to_vec());
    let reason = feedback.as_array().unwrap()[0]["reason"].as_str().unwrap();
    assert!(rejection["content"]["summary"].as_str().unwrap().contains(reason), "{rejection} / {feedback}");
    // The worker submits before its brief's acceptance window closes, so the
    // rejection can land while the attempt is still launching.
    lab.wait_for(&mut ticker, "the rejected attempt running", &attempt, 60, &|| lab.attempt(&attempt).state == AttemptState::Running);
    assert!(lab.attempt(&attempt).retains_capacity());
    assert!(lab.events("attempt.completion_requested").is_empty());
    fs::write(worktree.join("resubmit"), "go").unwrap();
    lab.wait_for(&mut ticker, "worker resubmission", &attempt, 120, &|| {
        lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec()).as_array().unwrap().len() == 2
    });
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
    assert!(lab.events("attempt.completion_requested").is_empty());
    lab.stop(ticker);
}

#[test]
fn editing_worker_requires_operator_completion_when_automation_is_off() {
    for (route, verify) in [("verify_only", false), ("verify_then_integrate", true)] {
        let (mut lab, attempt, _) = editing_submission_lab(route, WORK_POLICY, verify, false);
        lab.serve();
        let mut ticker = lab.spawn();
        lab.wait_for(&mut ticker, "worker submission", &attempt, 120, &|| {
            lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec()).as_array().unwrap().len() == 1
        });
        lab.wait_for(&mut ticker, "running worker", &attempt, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
        lab.stop(ticker);
        let shown = lab.ok(&["result", "demo", "show"]);
        let policy = lab.path("policy.json");
        fs::write(&policy, WORK_POLICY).unwrap();
        let verified = lab.ok(&["result", "demo", "verify", shown[0]["submission_id"].as_str().unwrap(), "--policy-id", "clean", "--policy-file", policy.to_str().unwrap(),
            "--idempotency-key", "manual-verify", "--work-dir", lab.path("manual-verify").to_str().unwrap()]);
        assert_eq!(verified["state"], "accepted");
        if route == "verify_then_integrate" {
            let repository = lab.repo.canonicalize().unwrap().display().to_string();
            let integrated = lab.ok(&["result", "demo", "integrate", verified["receipt"]["result_id"].as_str().unwrap(),
                "--repository", &repository, "--idempotency-key", "manual-integrate", "--work-dir", lab.path("manual-integrate").to_str().unwrap()]);
            assert_eq!(integrated["state"], "integrated");
        }
        lab.run_quiet(5);
        assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
        assert!(lab.attempt(&attempt).retains_capacity());
        assert!(lab.events("attempt.completion_requested").is_empty());
        let listed = lab.ok(&["task", "demo", "list"]);
        assert_eq!(listed["result_automation"]["verify"], verify);
        assert_eq!(listed["result_automation"]["integrate"], false);
        assert!(listed["completion_guidance"].as_str().unwrap().contains("task PROJECT complete"));
    }
}

#[test]
fn ticker_recovers_a_lost_creation_reply_without_creating_again() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    fs::write(lab.path("lab/lose-create"), "").unwrap();
    lab.serve();
    // The creation reply is lost. The claim is kept and the exact workspace is
    // recovered from observation; nothing is created twice, across a restart.
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.count("workspace.create_command") == 1);
    lab.stop(ticker);
    let launch = lab.state().deliveries.into_iter().find(|d| d.operation.as_str().starts_with("launch-")).unwrap();
    assert_eq!((launch.attempts, launch.state == DeliveryState::Pending), (1, false));
    assert!(lab.attempt(&attempt).retains_capacity());
    let briefed = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Confirmed)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &briefed);
    lab.stop(ticker);
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    assert!(lab.count("workspace.list") >= 1);
    assert_eq!((lab.events("runtime.launch_target").len(), lab.events("runtime.launch_started").len()), (1, 1));
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
}

/// Replaces `unsupported_connected_server_refuses_creation_before_approval_or_worktrees`
/// and, with the next test, the paused and cancelled cases of
/// `prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`.
#[test]
fn ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (approval, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let before = lab.state();
    for reply in [
        json!({"type":"pong","version":"0.9.1","capabilities":{"workspace_create_command":"true"}}),
        json!({"type":"pong","version":"0.9.2","capabilities":{"workspace_create_command":true}}),
        json!({"type":"unknown","version":"0.9.1","capabilities":{"workspace_create_command":true}}),
    ] {
        fs::write(lab.path("lab/ping.json"), reply.to_string()).unwrap();
        lab.run_for("ping", 2);
        assert!(lab.requests().iter().all(|(m, _)| m == "ping"), "{reply}: {:?}", lab.requests());
        lab.assert_only_observed(&before);
        assert!(!lab.project.join(".state/worktrees").exists());
    }
    assert!(lab.state().approvals.iter().any(|a| a.reference.id == approval && a.consumed.is_none()));
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Reserved);
    // A capable server, but the project is paused: passes run, nothing is dispatched.
    fs::remove_file(lab.path("lab/ping.json")).unwrap();
    let control = lab.state().control.unwrap().revision.to_string();
    lab.ok(&["runtime", "demo", "state", "paused", "--expected-revision", &control, "--expected-head", &lab.head().to_string()]);
    let (paused, asked) = (lab.state(), lab.requests().len());
    lab.run_passes(2);
    assert_eq!(lab.requests().len(), asked, "{:?}", &lab.requests()[asked..]);
    lab.assert_only_observed(&paused);
}

#[test]
fn ticker_does_not_dispatch_a_launch_cancelled_before_creation() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (approval, attempt) = lab.reserve("Retained instructions");
    let reserved = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &reserved.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "cancel selected launch"]);
    lab.serve();
    let cancelled = lab.state();
    lab.run_passes(2);
    assert!(lab.requests().is_empty(), "{:?}", lab.requests());
    lab.assert_only_observed(&cancelled);
    assert!(lab.state().approvals.iter().any(|a| a.reference.id == approval && a.consumed.is_none()));
    assert!(!lab.project.join(".state/worktrees").exists());
}

/// Replaces `resource_preparation_enforces_profile_budget_before_claim_or_external_effect`.
#[test]
fn profile_budgets_refuse_preparation_and_draft_before_any_approval() {
    // The retained knowledge including Remember intake and the worker command
    // card fits in 850 tokens (margin for longer root paths in the card);
    // the complete brief with worktree framing does not.
    let mut small = Lab::new("soft_input_tokens=850\nunknown_usage='allow_with_warning'");
    small.prepare_profile();
    let selection = small.selection("Retained instructions");
    let error = small.refused(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &small.head().to_string()]);
    assert!(error.contains("exceeding the captured budget"), "{error}");
    assert!(small.state().approvals.is_empty() && small.state().attempts.is_empty());

    let blocking = Lab::new("unknown_usage='block'");
    let error = blocking.refused(&["profile", "prepare", "demo", "worker", "--herdr-executable", blocking.herdr.to_str().unwrap(),
        "--agent-executable", blocking.path("bin/claude").to_str().unwrap(), "--execution-home", blocking.path("agent-home").to_str().unwrap()]);
    assert!(error.contains("verified usage telemetry is required"), "{error}");
}

/// Replaces `native_resource_creation_records_gated_target_once_and_cancellation_retains_it`.
#[test]
fn ticker_retires_a_cancelled_gated_worker_without_starting_it() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    fs::write(lab.path("lab/drop-release"), "").unwrap();
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| !lab.events("runtime.launch_target").is_empty() && lab.count("pane.send_input") >= 1);
    lab.stop(ticker);
    let target: LaunchTarget = serde_json::from_value(lab.events("runtime.launch_target")[0].payload.clone()).unwrap();
    let supervisor = target.supervisor.clone().unwrap();
    assert!(!herdr_farm::worker_supervision::SupervisorObservation::recover_exited(&supervisor).unwrap(), "the gated worker is waiting");
    let staged = lab.attempt(&attempt);
    assert_eq!((staged.state, staged.retains_capacity()), (AttemptState::Reserved, true));
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &staged.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "stop staged resource"]);
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let stopped = lab.attempt(&attempt);
    assert_eq!((stopped.state, stopped.retains_capacity()), (AttemptState::Cancelled, false));
    assert!(herdr_farm::worker_supervision::SupervisorObservation::recover_exited(&supervisor).unwrap(), "the gated worker has exited");
    assert!(lab.events("runtime.launch_started").is_empty() && lab.state().ownership.is_empty());
    assert_eq!(serde_json::from_value::<LaunchTarget>(lab.events("runtime.launch_target")[0].payload.clone()).unwrap(), target);
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 0));
}

impl Lab {
    /// The worktree `launch draft` planned for `attempt`, as `memory attempt-input` reports it.
    fn planned_worktree(&self, attempt: &AttemptId) -> PathBuf {
        PathBuf::from(self.ok(&["memory", "demo", "attempt-input", "--attempt", attempt.as_str()])["worktrees"][0]["path"].as_str().unwrap())
    }
    /// Ticker passes refuse to prepare the reserved launch with `reason`: no
    /// worktree, no Herdr request, the approval unused and the attempt reserved.
    fn assert_preparation_refused(&mut self, approval: &str, attempt: &AttemptId, reason: &str) {
        let worktree = self.planned_worktree(attempt);
        self.serve();
        let before = self.state();
        let log = || fs::read_to_string(self.path("root/.ticker.log")).unwrap_or_default();
        let mut ticker = self.spawn();
        self.wait(&mut ticker, 60, &|| log().matches(reason).count() >= 2);
        self.stop(ticker);
        assert_eq!(self.count("workspace.create_command"), 0, "{:?}", self.requests());
        assert!(!worktree.exists(), "{}", worktree.display());
        self.assert_only_observed(&before);
        assert!(self.state().approvals.iter().any(|a| a.reference.id == approval && a.consumed.is_none()));
        assert_eq!(self.attempt(attempt).state, AttemptState::Reserved);
    }
}

/// Replaces `draft_preflight_rejects_closed_capacity_without_writing_approval_or_task_state`.
#[test]
fn draft_is_refused_while_the_scheduler_admits_no_workers() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    lab.prepare_profile();
    let selection = lab.selection("Retained instructions");
    let policy = lab.state().scheduler.unwrap().policy.revision.to_string();
    lab.ok(&["scheduler", "demo", "policy", "--max-active-workers", "0", "--max-attempts-per-task", "3", "--expected-revision", &policy, "--expected-head", &lab.head().to_string()]);
    let error = lab.refused(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &lab.head().to_string()]);
    assert!(error.contains("project worker capacity is full"), "{error}");
    let state = lab.state();
    assert!(state.approvals.is_empty() && state.attempts.is_empty() && state.operations.is_empty());
    assert_eq!(state.tasks[0].active_attempt, None);
}

/// Replaces `worktree_route_maps_root_and_subdirectory_and_refuses_foreign_sources`.
#[test]
fn a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree() {
    let mut lab = Lab::bound("unknown_usage='allow_with_warning'", |repo| repo.join("subdir"));
    fs::create_dir(lab.repo.join("subdir")).unwrap();
    fs::write(lab.repo.join("subdir/file"), "tracked\n").unwrap();
    lab.git(&["add", "subdir/file"]);
    lab.git(&["commit", "-q", "-m", "subdir"]);
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.count("workspace.create_command") == 1);
    lab.stop(ticker);
    let creation = lab.requests().into_iter().find(|(m, _)| m == "workspace.create_command").unwrap().1;
    assert_eq!(creation["cwd"].as_str(), worktree.join("subdir").to_str(), "{creation}");
    assert_eq!(fs::read_to_string(worktree.join("subdir/file")).unwrap(), "tracked\n");

    // A binding outside every selected repository cannot even be drafted.
    let mut foreign = Lab::bound("unknown_usage='allow_with_warning'", |repo| repo.parent().unwrap().join("lab"));
    foreign.prepare_profile();
    let selection = foreign.selection("Retained instructions");
    let error = foreign.refused(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &foreign.head().to_string()]);
    assert!(error.contains("working directory is outside the approved repositories"), "{error}");
}

/// Replaces `source_only_working_directory_is_refused_before_approval_consumption`.
#[test]
fn an_untracked_working_directory_is_refused_before_the_approval_is_used() {
    let mut lab = Lab::bound("unknown_usage='allow_with_warning'", |repo| repo.join("untracked-dir"));
    fs::create_dir(lab.repo.join("untracked-dir")).unwrap();
    let (approval, attempt) = lab.reserve("Retained instructions");
    lab.assert_preparation_refused(&approval, &attempt, "working directory is absent from the approved repository tree");
}

/// Replaces `legacy_worktree_reference_blocks_new_creation_before_consuming_approval`.
#[test]
fn a_legacy_thread_holding_the_planned_worktree_blocks_its_creation() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (approval, attempt) = lab.reserve("Retained instructions");
    let neighbor = lab.path("root/legacy-neighbor");
    for dir in [".state", "threads"] { fs::create_dir_all(neighbor.join(dir)).unwrap(); }
    fs::write(neighbor.join("PROJECT.md"), "legacy fixture").unwrap();
    fs::write(neighbor.join("threads/t-0001.toml"), format!("id='t-0001'\nworktree_path={}\n", json!(lab.planned_worktree(&attempt)))).unwrap();
    lab.assert_preparation_refused(&approval, &attempt, "worktree path is referenced");
}

/// Replaces `worker_limits_and_literal_vector_are_explicit` with
/// `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`,
/// which checks the worker's argument vector.
///
/// A worker profile whose wall deadline is below one second or above seven
/// days cannot be prepared.
#[test]
fn a_worker_wall_deadline_outside_one_second_to_seven_days_is_refused() {
    let lab = Lab::new("unknown_usage='allow_with_warning'");
    let config = lab.path(".config/herdr-farm/config.toml");
    let original = fs::read_to_string(&config).unwrap();
    for (wall, reason) in [("0", "budget limits must be positive bounded integers"), ("604801", "worker wall deadline exceeds supported bounds")] {
        fs::write(&config, original.replace("max_wall_seconds=600", &format!("max_wall_seconds={wall}"))).unwrap();
        let error = lab.refused(&["profile", "prepare", "demo", "worker", "--herdr-executable", lab.herdr.to_str().unwrap(),
            "--agent-executable", lab.path("bin/claude").to_str().unwrap(), "--execution-home", lab.path("agent-home").to_str().unwrap()]);
        assert!(error.contains(reason), "{wall}: {error}");
    }
    fs::write(&config, original.replace("max_wall_seconds=600", "max_wall_seconds=604800")).unwrap();
    lab.ok(&["profile", "prepare", "demo", "worker", "--herdr-executable", lab.herdr.to_str().unwrap(),
        "--agent-executable", lab.path("bin/claude").to_str().unwrap(), "--execution-home", lab.path("agent-home").to_str().unwrap()]);
}

/// A worker whose end is proven (here: a cancelled attempt the ticker stops
/// while the project is active) leaves the project admitted after its pane
/// closes: the ownership claim is retired with an audit event and the next
/// task can be drafted. A pane that vanishes under a live worker, with no
/// termination evidence, still pauses the project and keeps the claim.
#[test]
fn a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    // A second queued task on its own resource-free binding.
    lab.ok(&["task", "demo", "add", "next", "--title", "next", "--expected-head", &lab.head().to_string()]);
    let request = lab.path("queue.json");
    lab.ok(&["task", "demo", "queue", "next", "--input-file", request.to_str().unwrap(), "--expected-revision", "1", "--expected-head", &lab.head().to_string()]);
    let id = TaskId::new("next").unwrap();
    let revision = lab.state().tasks.into_iter().find(|t| t.id == id).unwrap().revision;
    let route = RuntimeRoute { socket: lab.socket().display().to_string(), cwd: lab.repo.canonicalize().unwrap().display().to_string(), ..Default::default() };
    let next = runtime::create_binding(&lab.project, Some(&id), Some(revision), lab.head(), &route).unwrap().binding.id;
    lab.resume();
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.stop(ticker);
    let running = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "operator stop"]);
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    // Herdr closes the ended worker's pane; a pass observes its absence and
    // later passes no longer track the retired binding. The observation a
    // pass admits commits after the pass, so wait for each commit. A pass
    // whose 100 ms observation-head read overruns (under load) offers none.
    fs::write(lab.path("lab/vanish"), b"").unwrap();
    lab.run_until(4, &|| !lab.events("runtime.relinquished").is_empty());
    let observed = lab.state().observations.iter().map(|o| o.observed_unix_ms).max().unwrap();
    lab.run_until(4, &|| lab.state().observations.iter().any(|o| o.observed_unix_ms > observed));
    let state = lab.state();
    let control = state.control.clone().unwrap();
    assert_eq!((control.state, control.reconciliation_required), (ProjectState::Active, false), "{:?}", lab.events("project.reconciliation_invalidated"));
    assert!(state.ownership.iter().all(|o| o.binding != lab.binding), "{:?}", state.ownership);
    let retired = lab.events("runtime.relinquished");
    assert_eq!(retired.len(), 1);
    assert_eq!((retired[0].entity.as_str(), &retired[0].payload["termination"]["attempt"]), (lab.binding.as_str(), &json!(attempt.as_str())));
    // The binding and its resource references are retained.
    assert_eq!(state.runtime_bindings.iter().find(|b| b.id == lab.binding).unwrap().identity.pane_id, "w1:p1");
    let selection = lab.selection_for("next", &next, "Next instructions");
    lab.ok(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &lab.head().to_string()]);

    // Without termination evidence the same disappearance is unexplained.
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.stop(ticker);
    fs::write(lab.path("lab/vanish"), b"").unwrap();
    lab.run_for("pane.list", 2);
    let state = lab.state();
    let control = state.control.clone().unwrap();
    assert_eq!((control.state, control.reconciliation_required), (ProjectState::Paused, true));
    assert!(state.ownership.iter().any(|o| o.binding == lab.binding && o.attempt.as_ref() == Some(&attempt)));
    assert!(lab.events("runtime.relinquished").is_empty());
    assert!(state.inbox.iter().all(|item| item.content.kind != "attempt.worker_idle"));
    let live = lab.attempt(&attempt);
    assert!(live.retains_capacity() && !live.termination_observed);
}

/// Another holder of the shared root barrier (a store service, a CLI command)
/// comes and goes while a worker launches. Launch and brief take the exclusive
/// root afresh at each durable stage; each waits out the holder instead of
/// failing the job and backing off, and while such an effect is admitted the
/// ticker's own store services stay off the root. The worker still reaches
/// Running, and no job or service lost the root to the contention.
#[test]
fn a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently() {
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    // Hold the root shared for 300 ms of every 400 ms, as a busy neighbour would.
    let (stop, held) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let holder = {
        let (lock, stop, held) = (lab.path("root/.execution.lock"), stop.clone(), held.clone());
        std::thread::spawn(move || while !stop.load(Ordering::SeqCst) {
            let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock).unwrap();
            if file.try_lock_shared().is_ok() {
                held.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
                file.unlock().unwrap();
            }
            std::thread::sleep(Duration::from_millis(100));
        })
    };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.stop(ticker);
    stop.store(true, Ordering::SeqCst);
    holder.join().unwrap();
    assert!(held.load(Ordering::SeqCst) >= 5, "the holder took the root {} times", held.load(Ordering::SeqCst));
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap();
    assert!(!log.contains(".execution.lock; retry"), "an effect lost the root: {log}");
    assert!(!log.contains("root maintenance or exclusive external operation is active"), "a service contended with an admitted effect: {log}");
}

// ---- Review launch (docs/telemetry/contracts-review.md §11, card D9) ----

/// The reviewed work's identities, planted so a leak into a reviewer's brief shows.
const AUTHOR_ATTEMPT: &str = "author-sentinel-attempt-0001";
const AUTHOR_TITLE: &str = "AUTHOR-TITLE-SENTINEL";
const PROJECT_SENTINEL: &str = "PROJECT-INSTRUCTIONS-SENTINEL";
const AUTHOR_CONFIGURATION_JSON: &str = r#"{"kind":"codex","schema":"agent_configuration.v1","sentinel":"AUTHOR-CONFIGURATION-SENTINEL"}"#;

/// A review world on the lab: task `authored` (title `AUTHOR_TITLE`) with an
/// owner-signed contract, whose attempt `AUTHOR_ATTEMPT` (dispatched as a
/// `codex` configuration) submitted candidate commit C on branch `author`;
/// review opportunity O (`code`, `review-protocol.v1`, budget 600000 ms,
/// prior finding `finding:prior-a`) blindly assigned to the lab's `worker`
/// profile; and the queued task `work` turned into O's review task by a
/// worker snapshot built with `--review-opportunity`. PROJECT.md names the
/// author. Nothing is reserved yet.
struct ReviewWorld { opportunity: String, submission: String, candidate: String, base: String, repository: String, store: String,
    author_configuration: String, author_profile_digest: String, snapshot: Value, selection: PathBuf }

impl Lab {
    fn review_world(&mut self) -> ReviewWorld { self.review_world_setup(true) }
    fn review_world_setup(&mut self, bind: bool) -> ReviewWorld {
        self.prepare_profile();
        let head = self.head().to_string();
        self.ok(&["task", "demo", "add", "authored", "--title", AUTHOR_TITLE, "--expected-head", &head]);
        let base = self.git(&["rev-parse", "HEAD"]);
        self.git(&["checkout", "-qb", "author"]);
        fs::write(self.repo.join("lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
        self.git(&["add", "."]);
        self.git(&["commit", "-qm", "candidate"]);
        let candidate = self.git(&["rev-parse", "HEAD"]);
        self.git(&["checkout", "-q", "-"]);
        let repository = self.repo.canonicalize().unwrap().display().to_string();
        let store = self.project.join(".state/state.db").canonicalize().unwrap().display().to_string();
        let mut document = serde_json::to_vec_pretty(&json!({
            "version": 3, "outputs": [{"path": "lib.rs", "kind": "git_file"}], "scope": {"paths": [{"path": "lib.rs", "access": "write"}]},
            "project_store": store, "expected_head": self.head(), "task_id": "authored", "contract_revision": 1, "deliverable": "answer", "non_goals": "none",
            "acceptance_policies": [{"id": "clean", "text": r#"{"version":1,"checks":["/usr/bin/git","diff","--quiet"]}"#}], "repository": repository, "base_oid": base,
            "object_format": "sha256", "dependencies": [], "capability_flags": [], "profile_kind": "codex", "retry_class": "none", "result_schema_id": "result-v1",
            "route": "verify_only", "authority": authority::policy_reference(&self.project).unwrap()})).unwrap();
        document.push(b'\n');
        let contract = self.path("authored-contract.json");
        fs::write(&contract, &document).unwrap();
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&self.key).args(["-n", authority::CONTRACT_SIGNATURE_NAMESPACE]).arg(&contract).output().unwrap().status.success());
        let installed = self.ok(&["task", "demo", "contract", "put", "--input-file", contract.to_str().unwrap(), "--signature", self.path("authored-contract.json.sig").to_str().unwrap()]);
        // The author attempt ran and ended; its dispatch chose a `codex` configuration.
        let author_configuration = format!("sha256:{:x}", Sha256::digest(AUTHOR_CONFIGURATION_JSON.as_bytes()));
        let author_profile_digest = "e".repeat(64);
        let db = rusqlite::Connection::open(self.project.join(".state/state.db")).unwrap();
        db.execute("INSERT INTO attempts(id,task_id,revision,state,snapshot,reservation,termination_observed) VALUES(?1,'authored',1,'completed',NULL,?1,1)", [AUTHOR_ATTEMPT]).unwrap();
        db.execute("INSERT INTO agent_configurations VALUES(?1,?2,1)", rusqlite::params![author_configuration, AUTHOR_CONFIGURATION_JSON]).unwrap();
        let eligible = json!([{"configuration_id": author_configuration, "probability_ppm": 1_000_000, "profile_digest": author_profile_digest, "status": "chosen"}]).to_string();
        db.execute("INSERT INTO dispatch_decisions(attempt_id,task_id,task_revision,contract_revision,chosen_configuration_id,eligible,chooser_kind,chooser_principal,reason_codes,decided_unix_ms)
            VALUES(?1,'authored',1,1,?2,?3,'operator','operator:cli','[\"unspecified\"]',1)", rusqlite::params![AUTHOR_ATTEMPT, author_configuration, eligible]).unwrap();
        drop(db);
        let objects: Vec<Value> = self.git(&["rev-list", "--objects", "--all"]).lines()
            .map(|line| { let oid = line.split_whitespace().next().unwrap(); json!({"oid": oid, "relative_path": format!("{}/{}", &oid[..2], &oid[2..])}) }).collect();
        let result = self.path("authored-result.json");
        fs::write(&result, json!({"idempotency_key": "authored-key", "task_id": "authored", "contract_revision": 1, "contract_digest": installed["digest"],
            "attempt_id": AUTHOR_ATTEMPT, "repository": repository, "base_oid": base, "candidate_oid": candidate, "object_format": "sha256",
            "artifact_manifest": [{"path": "lib.rs", "oid": candidate}], "claimed_checks": [], "objects": objects}).to_string()).unwrap();
        let submission = self.ok(&["result", "demo", "submit", "--input-file", result.to_str().unwrap()])["submission_id"].as_str().unwrap().to_owned();
        if !bind { return ReviewWorld { opportunity: String::new(), submission, candidate, base, repository, store, author_configuration, author_profile_digest,
            snapshot: Value::Null, selection: self.path("unused-selection.json") }; }
        let opportunity = self.ok(&["telemetry", "demo", "review", "open", &submission, "--protocol", "review-protocol.v1", "--budget-ms", "600000",
            "--prior-finding", "finding:prior-a"])["opportunity"]["opportunity_id"].as_str().unwrap().to_owned();
        let assignment = self.ok(&["telemetry", "demo", "review", "assign", &opportunity, "--blind", "--candidate", "worker"])["assignment"].clone();
        assert_eq!((&assignment["author_attempt_id"], &assignment["reason"], &assignment["same_family"], &assignment["blind"]),
            (&json!(AUTHOR_ATTEMPT), &json!("cross_provider"), &json!(false), &json!(true)));
        // The review task's snapshot: the blind brief replaces PROJECT.md, which names the author.
        fs::write(self.project.join("PROJECT.md"), format!("{PROJECT_SENTINEL} written by {AUTHOR_ATTEMPT}")).unwrap();
        let scope = self.path("review-scope.json");
        fs::write(&scope, json!({"schema_version":1,"task_id":"work","profile":"worker","domains":[],"paths":[],"pinned_keys":[],"sensitivity":"default"}).to_string()).unwrap();
        let snapshot = self.ok(&["memory", "demo", "snapshot", "--task", "work", "--profile", "worker", "--input-file", scope.to_str().unwrap(), "--worker", "--review-opportunity", &opportunity]);
        let selection = self.path("review-selection.json");
        fs::write(&selection, json!({"task":"work","binding":self.binding,"profile":self.profile,
            "knowledge":{"id":snapshot["id"],"revision":1,"digest":snapshot["manifest_hash"]},"repositories":[self.repo.canonicalize().unwrap()]}).to_string()).unwrap();
        ReviewWorld { opportunity, submission, candidate, base, repository, store, author_configuration, author_profile_digest, snapshot, selection }
    }
    /// Draft, owner-sign, import and reserve `selection`; returns (draft, attempt).
    fn reserve_selection(&self, selection: &std::path::Path) -> (Value, AttemptId) {
        let drafted = self.ok(&["launch", "demo", "draft", "--selection", selection.to_str().unwrap(), "--expected-head", &self.head().to_string()]);
        let document = self.path("review-approval.json");
        fs::write(&document, serde_json::to_vec_pretty(&drafted["approval"]).unwrap()).unwrap();
        let _ = fs::remove_file(self.path("review-approval.json.sig"));
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&self.key).args(["-n", authority::SIGNATURE_NAMESPACE]).arg(&document).output().unwrap().status.success());
        let approval = self.ok(&["approval", "demo", "import", document.to_str().unwrap(), self.path("review-approval.json.sig").to_str().unwrap(), "--expected-head", &self.head().to_string()]);
        let reservation = self.ok(&["launch", "demo", "reserve", "--selection", selection.to_str().unwrap(), "--approval-digest", approval["digest"].as_str().unwrap(), "--expected-head", &self.head().to_string()]);
        (drafted, AttemptId::new(reservation["record"]["attempt"].as_str().unwrap()).unwrap())
    }
    /// The CLI as the reviewing worker runs it: `HOME` is the profile's execution home.
    fn worker_cli(&self, args: &[&str]) -> Output {
        Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", self.path("agent-home")).env("PATH", "/usr/bin:/bin")
            .args(["--root", self.path("root").to_str().unwrap()]).args(args).output().unwrap()
    }
    fn review_show(&self, as_of: Option<i64>) -> Value {
        let mut args = vec!["telemetry".to_owned(), "demo".into(), "review".into(), "show".into()];
        if let Some(seq) = as_of { args.extend(["--as-of".into(), seq.to_string()]); }
        self.ok(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// The retained instructions inside a rendered worker prompt.
fn instructions(prompt: &str) -> &str {
    let start = prompt.find("# Project instructions\n\n").unwrap() + "# Project instructions\n\n".len();
    &prompt[start..start + prompt[start..].find("\n\n# Task\n\n").unwrap()]
}

/// O is assigned blindly to `worker`; task `work` becomes its review task
/// through `memory snapshot --review-opportunity O`, whose retained
/// instructions are the blind brief (bound with its digest, prior findings
/// withheld: the protocol is unregistered). A plain PROJECT.md snapshot of
/// the same task cannot launch it. The ordinary `launch draft`/approval/
/// `launch reserve` path reserves the review attempt, and that reservation
/// records the session: recorder `service:launch`, the assigned
/// configuration, ledger `started` at seq 3 (after O's opening at 1 and its
/// assignment at 2). The ticker then launches and
/// briefs the worker on the Herdr stand-in; the delivered prompt is the
/// drafted brief, names O's exact candidate, and carries none of the planted
/// author identities (attempt, configuration, profile digest, the authored
/// task's title, PROJECT.md).
#[test]
fn review_assignment_launches_with_blind_brief_and_records_session() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let world = lab.review_world();
    let bound = &world.snapshot["review_brief"];
    assert_eq!((&bound["opportunity_id"], &bound["task_id"], &bound["snapshot_id"], &bound["prior_disclosure"], &bound["principal"], &bound["replayed"]),
        (&json!(world.opportunity), &json!("work"), &world.snapshot["id"], &json!("withheld"), &json!("operator:cli"), &json!(false)));

    // A snapshot of the review task that is not its blind brief never launches it.
    let plain = lab.path("plain-scope.json");
    fs::write(&plain, json!({"schema_version":1,"task_id":"work","profile":"worker","domains":[],"paths":[],"pinned_keys":[],"sensitivity":"default"}).to_string()).unwrap();
    let project_snapshot = lab.ok(&["memory", "demo", "snapshot", "--task", "work", "--profile", "worker", "--input-file", plain.to_str().unwrap(), "--worker"]);
    let wrong = lab.path("plain-selection.json");
    fs::write(&wrong, json!({"task":"work","binding":lab.binding,"profile":lab.profile,
        "knowledge":{"id":project_snapshot["id"],"revision":1,"digest":project_snapshot["manifest_hash"]},"repositories":[lab.repo.canonicalize().unwrap()]}).to_string()).unwrap();
    let refused = lab.refused(&["launch", "demo", "draft", "--selection", wrong.to_str().unwrap(), "--expected-head", &lab.head().to_string()]);
    assert!(refused.contains("a review task launches only with its blind review brief snapshot"), "{refused}");

    let (drafted, attempt) = lab.reserve_selection(&world.selection);
    let shown = lab.review_show(None);
    let session = shown["opportunities"][0]["sessions"][0].clone();
    assert_eq!(shown["opportunities"][0]["status"], json!("in_progress"));
    let configuration = shown["opportunities"][0]["assignment"]["reviewer_configuration_id"].clone();
    assert_eq!((&session["ordinal"], &session["attempt_id"], &session["recorder_principal"], &session["configuration_id"], &session["matches_assignment"], &session["same_attempt_as_author"], &session["completion"]),
        (&json!(1), &json!(attempt.as_str()), &json!("service:launch"), &configuration, &json!(true), &json!(false), &Value::Null));
    // O's opening (1) and assignment (2) are ledger rows; the start follows them, in the reservation's transaction.
    assert_eq!(lab.review_show(Some(0))["opportunities"], json!([]));
    let at1 = lab.review_show(Some(1))["opportunities"][0].clone();
    assert_eq!((&at1["opened_seq"], &at1["status"], &at1["assignment"]), (&json!(1), &json!("unassigned"), &Value::Null));
    let at2 = lab.review_show(Some(2))["opportunities"][0].clone();
    assert_eq!((&at2["assignment"]["assigned_seq"], &at2["status"], &at2["sessions"]), (&json!(2), &json!("no_session"), &json!([])));
    assert_eq!(lab.review_show(Some(3))["opportunities"][0]["sessions"][0]["session_id"], session["session_id"]);
    let db = rusqlite::Connection::open(lab.project.join(".state/state.db")).unwrap();
    let launched: (String, String) = db.query_row("SELECT l.attempt_id,l.snapshot_id FROM review_session_launches l WHERE l.session_id=?1", [session["session_id"].as_str().unwrap()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((launched.0.as_str(), launched.1.as_str()), (attempt.as_str(), world.snapshot["id"].as_str().unwrap()));
    drop(db);

    lab.serve();
    let briefed = || { let s = lab.state(); s.operations.iter().any(|o| o.kind == "runtime.worker_brief" && s.deliveries.iter().any(|d| d.operation == o.id && d.state == DeliveryState::Confirmed)) };
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &briefed);
    lab.stop(ticker);
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    let text = lab.requests().into_iter().find(|(m, _)| m == "agent.prompt").unwrap().1["text"].as_str().unwrap().to_owned();
    assert_eq!(text, drafted["brief"]["text"].as_str().unwrap(), "the worker receives exactly the drafted brief");
    let brief = instructions(&text);
    assert_eq!(bound["brief_digest"], json!(format!("sha256:{:x}", Sha256::digest(brief.as_bytes()))));
    assert!(brief.starts_with("# Blind review (review_brief.v1)\n\n"), "{brief}");
    let view: Value = serde_json::from_str(&brief[brief.find("{\n").unwrap()..=brief.find("\n}").unwrap() + 1]).unwrap();
    assert_eq!(view, json!({"opportunity_id": world.opportunity, "task_id": "authored", "contract_revision": 1, "repository": world.repository,
        "base_oid": world.base, "candidate_oid": world.candidate, "object_format": "sha256", "scope": "candidate_diff", "kind": "code",
        "protocol": "review-protocol.v1", "budget_ms": 600000}));
    assert!(text.contains(&format!("Attempt: {}", attempt.as_str())) && text.contains("review submit --input-file FILE"), "{text}");
    for sentinel in [AUTHOR_ATTEMPT, AUTHOR_TITLE, PROJECT_SENTINEL, "AUTHOR-CONFIGURATION-SENTINEL", world.author_configuration.as_str(),
        world.author_profile_digest.as_str(), "finding:prior-a", world.submission.as_str()] {
        assert!(!text.contains(sentinel), "the brief names {sentinel}");
    }
}

/// Write `receipt` beside the lab and return its path.
fn receipt_file(lab: &Lab, name: &str, receipt: &Value) -> String {
    let path = lab.path(name);
    fs::write(&path, receipt.to_string()).unwrap();
    path.display().to_string()
}

/// The reviewing worker (HOME = its execution home) finds its launched
/// session and submits its `review_receipt.v1` through `review submit`: a
/// proposal (`trust: proposal`, declared coverage) recorded as
/// `worker:<attempt>`, with its finding a pending submission; the same
/// receipt replays and another is refused. A receipt carrying `accepted` is
/// refused. In that context `review accept`, `review accept draft`, `review
/// complete` and finding triage are refused, writing nothing; the session
/// stays undecided.
#[test]
fn reviewer_worker_submits_proposal_receipt_but_cannot_accept() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let world = lab.review_world();
    let (_, attempt) = lab.reserve_selection(&world.selection);
    let worker = |args: &[&str]| -> Result<Value, String> {
        let out = lab.worker_cli(args);
        if out.status.success() { Ok(serde_json::from_slice(&out.stdout).unwrap()) } else { Err(String::from_utf8_lossy(&out.stderr).into_owned()) }
    };
    let mine = worker(&["telemetry", "demo", "review", "session", "--attempt", attempt.as_str()]).unwrap()["session"].clone();
    let session = mine["session_id"].as_str().unwrap().to_owned();
    assert_eq!(mine, json!({"session_id": session, "opportunity_id": world.opportunity, "ordinal": 1, "submission_id": world.submission,
        "candidate_oid": world.candidate, "completed": false, "receipt_schema": "review_receipt.v1"}));

    let mut receipt = json!({"schema": "review_receipt.v1", "session_id": session, "submission_id": world.submission, "candidate_oid": world.candidate,
        "outcome": "completed", "findings": [{"ref": "finding:w1", "title": "Answer is hard-coded"}], "evidence": [format!("sha256:{}", "c".repeat(64))]});
    let mut claimed = receipt.clone();
    claimed["accepted"] = json!(true);
    let err = worker(&["telemetry", "demo", "review", "submit", "--input-file", &receipt_file(&lab, "claimed.json", &claimed)]).unwrap_err();
    assert!(err.contains("unknown field `accepted`"), "{err}");
    let path = receipt_file(&lab, "receipt.json", &receipt);
    let done = worker(&["telemetry", "demo", "review", "submit", "--input-file", &path]).unwrap()["completion"].clone();
    assert_eq!((&done["outcome"], &done["trust"], &done["coverage_basis"], &done["recorder_principal"], &done["findings_submitted"], &done["replayed"]),
        (&json!("completed"), &json!("proposal"), &json!("declared"), &json!(format!("worker:{}", attempt.as_str())), &json!(1), &json!(false)));
    assert_eq!(worker(&["telemetry", "demo", "review", "submit", "--input-file", &path]).unwrap()["completion"]["replayed"], json!(true));
    receipt["findings"] = json!([]);
    let err = worker(&["telemetry", "demo", "review", "submit", "--input-file", &receipt_file(&lab, "other.json", &receipt)]).unwrap_err();
    assert!(err.contains("already has a different completion"), "{err}");

    // The worker decides nothing: every deciding or owner command refuses its context.
    let before = rusqlite::Connection::open(lab.project.join(".state/state.db")).unwrap()
        .query_row("SELECT (SELECT count(*) FROM review_acceptances)+(SELECT count(*) FROM finding_decisions)+(SELECT count(*) FROM review_log)", [], |r| r.get::<_, i64>(0)).unwrap();
    for args in [vec!["review", "accept", session.as_str(), "--document", path.as_str(), "--signature", path.as_str()],
        vec!["review", "accept", "draft", session.as_str(), "--grant", "sha256:0", "--output", "/dev/null"],
        vec!["review", "complete", "--input-file", path.as_str()],
        vec!["review", "findings", "reject", "1", "--reason", "out_of_scope"]] {
        let mut all = vec!["telemetry", "demo"];
        all.extend(args);
        let err = worker(&all).unwrap_err();
        assert!(err.contains("refuses to run inside a worker execution context: HOME is a worker execution home"), "{all:?}: {err}");
    }
    let after = rusqlite::Connection::open(lab.project.join(".state/state.db")).unwrap()
        .query_row("SELECT (SELECT count(*) FROM review_acceptances)+(SELECT count(*) FROM finding_decisions)+(SELECT count(*) FROM review_log)", [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(after, before, "refused commands write nothing");
    let completion = lab.review_show(None)["opportunities"][0]["sessions"][0]["completion"].clone();
    assert_eq!((&completion["trust"], &completion["acceptance"], &completion["recorder_principal"]),
        (&json!("proposal"), &Value::Null, &json!(format!("worker:{}", attempt.as_str()))));
    let findings = lab.ok(&["telemetry", "demo", "review", "findings", "show"])["findings"]["submissions"].clone();
    assert_eq!((&findings[0]["finding_ref"], &findings[0]["outcome"], &findings[0]["title"]), (&json!("finding:w1"), &json!("pending"), &json!("Answer is hard-coded")));
}

/// Owner-signed grant G makes `reviewer:carol` a delegated reviewer of task
/// `authored`. After the opportunity's opening (seq 1) and assignment (2),
/// the launched session (seq 3) and the worker's receipt (completion seq 4,
/// its finding seq 5), `review accept draft` writes the exact canonical
/// request, byte for byte; carol signs it offline with her own key and
/// `review accept` records the decision at seq 6 of the shared ledger.
/// `review show --as-of` places it: undecided at 5, accepted from 6
/// (`ledger_seq` 6); the owner's later triage takes seq 7, after it.
#[test]
fn acceptance_decision_replays_in_the_ledger_as_of() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let world = lab.review_world();
    let (_, attempt) = lab.reserve_selection(&world.selection);
    let session = lab.review_show(None)["opportunities"][0]["sessions"][0]["session_id"].as_str().unwrap().to_owned();
    let receipt = json!({"schema": "review_receipt.v1", "session_id": session, "submission_id": world.submission, "candidate_oid": world.candidate,
        "outcome": "completed", "findings": ["finding:w1"], "evidence": []});
    let out = lab.worker_cli(&["telemetry", "demo", "review", "submit", "--input-file", &receipt_file(&lab, "receipt.json", &receipt)]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let receipt_digest = serde_json::from_slice::<Value>(&out.stdout).unwrap()["completion"]["receipt_digest"].as_str().unwrap().to_owned();
    assert_eq!(lab.ok(&["telemetry", "demo", "review", "findings", "show"])["findings"]["head_seq"], json!(5));

    let carol = lab.path("carol");
    assert!(Command::new("/usr/bin/ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&carol).output().unwrap().status.success());
    let carol_public = fs::read_to_string(carol.with_extension("pub")).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    let now = jiff::Timestamp::now().as_millisecond();
    let grant = json!({"schema": "code_review_authority.v1", "scope": "code_review", "issuer": "owner", "subject": "reviewer:carol",
        "subject_public_key": carol_public, "subject_configurations": [], "project_store": world.store, "repositories": [world.repository],
        "tasks": [{"task_id": "authored", "contract_revision": 1}], "kinds": ["code"], "review_configurations": [], "actions": ["accept_review_completion"],
        "max_decisions": 1, "valid_from_unix_ms": now - 60_000, "expires_unix_ms": now + 3_600_000,
        "prohibited_effects": ["alter_requirements", "approve_author_attempt", "approve_own_work", "child_delegation", "increase_permissions"],
        "authority": authority::policy_reference(&lab.project).unwrap()});
    let grant_file = lab.path("grant.json");
    fs::write(&grant_file, serde_json::to_vec_pretty(&grant).unwrap()).unwrap();
    assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&lab.key).args(["-n", "code-review-authority@herdr-projects"]).arg(&grant_file).output().unwrap().status.success());
    let grant_id = lab.ok(&["telemetry", "demo", "review", "authority", "import", grant_file.to_str().unwrap(), lab.path("grant.json.sig").to_str().unwrap()])["grant"]["grant_id"].as_str().unwrap().to_owned();

    // The product drafts the exact bytes; it holds no reviewer key and signs nothing.
    let request = lab.path("request.json");
    let drafted = lab.ok(&["telemetry", "demo", "review", "accept", "draft", &session, "--grant", &grant_id, "--output", request.to_str().unwrap()])["draft"].clone();
    let expected = format!(r#"{{"decision":"accepted","grant_id":"{grant_id}","project_store":"{}","reason":null,"receipt_digest":"{receipt_digest}","schema":"review_acceptance.v1","session_id":"{session}","subject":"reviewer:carol"}}"#, world.store);
    assert_eq!(fs::read_to_string(&request).unwrap(), expected);
    assert_eq!((&drafted["request_digest"], &drafted["signer"], &drafted["namespace"]),
        (&json!(format!("sha256:{:x}", Sha256::digest(expected.as_bytes()))), &json!("reviewer:carol"), &json!("review-acceptance@herdr-projects")));
    assert!(!lab.cli(&["telemetry", "demo", "review", "accept", "draft", &session, "--grant", &grant_id, "--output", request.to_str().unwrap()]).status.success(), "the draft never overwrites");
    assert_eq!(lab.ok(&["telemetry", "demo", "review", "findings", "show"])["findings"]["head_seq"], json!(5), "a draft writes nothing");

    assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&carol).args(["-n", "review-acceptance@herdr-projects"]).arg(&request).output().unwrap().status.success());
    let accepted = lab.ok(&["telemetry", "demo", "review", "accept", &session, "--document", request.to_str().unwrap(), "--signature", lab.path("request.json.sig").to_str().unwrap()])["acceptance"].clone();
    assert_eq!((&accepted["decision"], &accepted["authority_principal"], &accepted["ledger_seq"], &accepted["receipt_digest"], &accepted["replayed"]),
        (&json!("accepted"), &json!("reviewer:carol"), &json!(6), &json!(receipt_digest), &json!(false)));

    let at = |seq: i64| lab.review_show(Some(seq));
    let s3 = at(3);
    assert_eq!((&s3["head_seq"], &s3["as_of_seq"], &s3["opportunities"][0]["status"], &s3["opportunities"][0]["sessions"][0]["attempt_id"], &s3["opportunities"][0]["sessions"][0]["completion"]),
        (&json!(6), &json!(3), &json!("in_progress"), &json!(attempt.as_str()), &Value::Null));
    let s5 = at(5)["opportunities"][0]["sessions"][0]["completion"].clone();
    assert_eq!((&s5["outcome"], &s5["acceptance"]), (&json!("completed"), &Value::Null));
    let s6 = at(6)["opportunities"][0]["sessions"][0]["completion"]["acceptance"].clone();
    assert_eq!((&s6["decision"], &s6["grant_id"], &s6["ledger_seq"], &s6["authority"]), (&json!("accepted"), &json!(grant_id), &json!(6), &json!("delegated_code_review.v1")));
    let err = String::from_utf8_lossy(&lab.cli(&["telemetry", "demo", "review", "show", "--as-of", "7"]).stderr).into_owned();
    assert!(err.contains("as-of seq 7 is outside the review history 0..=6"), "{err}");

    // The owner's triage follows the decision in the one ordering.
    let claim = lab.ok(&["telemetry", "demo", "review", "findings", "show"])["findings"]["submissions"][0]["claims"][0]["claim_id"].as_i64().unwrap();
    let event = lab.ok(&["telemetry", "demo", "review", "findings", "reject", &claim.to_string(), "--reason", "intended_behavior"])["event"].clone();
    assert_eq!(event["seq"], json!(7));
    assert_eq!(at(6)["opportunities"][0]["sessions"][0]["completion"]["acceptance"]["decision"], json!("accepted"));
}

/// A worker agent that probes its own filesystem view. It records every
/// secret read, an append to each owner file in `WRITES`, a write and read
/// back of `TMP_WRITE` and the `/tmp` listing, the projects root listing, a connection to the Herdr socket,
/// direct `umount2`/`mount` calls on each hidden path, and the same attempts
/// (plus a bind of a hidden path's parent) from a user and mount namespace it
/// creates itself. It then does ordinary work: writes and commits a file in
/// its worktree, publishes `probe-1.txt`, waits for the owner's `submit.json`
/// and submits it through the product CLI, publishing `probe-2.txt`.
const PROBE_AGENT: &str = r#"
use std::{ffi::{CString, c_char, c_void}, fs, os::unix::net::UnixStream, path::Path, process::Command, time::Duration};
extern "C" { fn umount2(target: *const c_char, flags: i32) -> i32; fn mount(source: *const c_char, target: *const c_char, kind: *const c_char, flags: u64, data: *const c_void) -> i32; }
fn sys(result: i32) -> String { if result == 0 { "OK".into() } else { format!("ERR{}", std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) } }
fn publish(name: &str, text: &str) { fs::write(format!("{name}.tmp"), text).unwrap(); fs::rename(format!("{name}.tmp"), name).unwrap(); }
fn run(program: &str, args: &[&str]) -> (bool, String) {
    match Command::new(program).args(args).output() {
        Ok(out) => (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).replace('\n', "|")),
        Err(error) => (false, error.to_string()),
    }
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    let mut report = String::new();
    for key in ["GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0", "GIT_CONFIG_KEY_1", "GIT_CONFIG_VALUE_1", "GIT_CONFIG_KEY_2", "GIT_CONFIG_VALUE_2"] {
        report += &format!("env {key} {}\n", std::env::var(key).unwrap_or_default());
    }
    for key in ["gc.auto", "gc.autoDetach", "maintenance.auto"] {
        report += &format!("gitconfig {key} {}\n", run("/usr/bin/git", &["config", "--get", key]).1);
    }
    for secret in SECRETS {
        report += &format!("read {secret} {}\n", match fs::read(secret) { Ok(bytes) => format!("OK:{}", String::from_utf8_lossy(&bytes).trim()), Err(error) => format!("ERR:{:?}", error.kind()) });
    }
    for target in WRITES {
        let written = fs::OpenOptions::new().create(true).append(true).open(target).and_then(|mut file| std::io::Write::write_all(&mut file, b"WORKER-WROTE\n"));
        report += &format!("write {target} {}\n", match written { Ok(()) => "OK".into(), Err(error) => format!("ERR:{:?}", error.kind()) });
    }
    report += &format!("tmp {} {}\n", fs::write(TMP_WRITE, "private").is_ok(), fs::read_to_string(TMP_WRITE).unwrap_or_default());
    let list = |dir: &str| { let mut names: Vec<String> = fs::read_dir(dir).map(|d| d.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default(); names.sort(); names.join(",") };
    report += &format!("root {}\n", list(ROOT));
    report += &format!("tmp-list {}\n", list("/tmp"));
    report += &format!("own {}\n", list(OWN));
    report += &format!("socket {}\n", match UnixStream::connect(SOCKET) { Ok(_) => "OK".into(), Err(error) => format!("ERR:{:?}", error.kind()) });
    let tmpfs = CString::new("tmpfs").unwrap();
    for dir in HIDDEN {
        let target = CString::new(*dir).unwrap();
        report += &format!("unmount {dir} {} {}\n", sys(unsafe { umount2(target.as_ptr(), 0) }), sys(unsafe { umount2(target.as_ptr(), 2) }));
        report += &format!("mount {dir} {}\n", sys(unsafe { mount(tmpfs.as_ptr(), target.as_ptr(), tmpfs.as_ptr(), 0, std::ptr::null()) }));
    }
    let (_, nested) = run("/usr/bin/unshare", &["--user", "--map-root-user", "--mount", "/bin/sh", "-c", NESTED]);
    report += &format!("nested {nested}\n");
    let git = |args: &[&str]| run("/usr/bin/git", &[&["-c", "user.name=worker", "-c", "user.email=worker@example.invalid"], args].concat());
    let line = |args: &[&str]| git(args).1.trim_end_matches('|').to_owned();
    fs::write("stray.txt", "unreachable worker object\n").unwrap();
    report += &format!("stray {}\n", line(&["hash-object", "-w", "stray.txt"]));
    fs::remove_file("stray.txt").unwrap();
    fs::write("work.txt", "worker change\n").unwrap();
    let (added, _) = git(&["add", "work.txt"]);
    let (committed, output) = git(&["commit", "-qm", "worker change"]);
    let head = line(&["rev-parse", "HEAD"]);
    report += &format!("commit {} {}\nhead {head}\n", added && committed, output);
    for branch in ["master", "hp-other", "integration"] {
        report += &format!("move {branch} {}\n", git(&["update-ref", &format!("refs/heads/{branch}"), &head]).0);
    }
    report += &format!("objects {}\n", line(&["rev-list", "--objects", &head]).split('|').map(|l| l.split(' ').next().unwrap()).collect::<Vec<_>>().join(","));
    // Git writes outside its own branch, valid for Git: they may succeed in
    // the worker's quarantined view but must never reach the shared
    // repository. The owner's loose object holds `owner object`.
    let common = line(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    fs::write("owner.txt", "owner object\n").unwrap();
    let owned = line(&["hash-object", "owner.txt"]);
    fs::remove_file("owner.txt").unwrap();
    let base = line(&["rev-parse", "HEAD~"]);
    let wt_config = format!("{}/config.worktree", line(&["rev-parse", "--absolute-git-dir"]));
    for (target, text) in [(format!("{common}/objects/{}/{}", &owned[..2], &owned[2..]), "WORKER-WROTE\n".to_owned()), (wt_config, "[core]\n\tfsmonitor = /bin/false\n".into()),
        (format!("{common}/config"), "[core]\n\thooksPath = /tmp\n".into()), (format!("{common}/hooks/pre-commit"), "exit 0\n".into()),
        (format!("{common}/info/exclude"), "work.txt\n".into()), (format!("{common}/packed-refs"), format!("{base} refs/heads/worker-planted\n"))] {
        let written = fs::OpenOptions::new().create(true).append(true).open(&target).and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
        report += &format!("gitwrite {target} {}\n", match written { Ok(()) => "OK".into(), Err(error) => format!("ERR:{:?}", error.kind()) });
    }
    publish("probe-1.txt", &report);
    while !Path::new("submit.json").exists() { std::thread::sleep(Duration::from_millis(50)); }
    let mut last = (false, String::new());
    for _ in 0..200 {
        // Bare: the sandbox puts the product binary first on the agent's PATH.
        last = run("herdr-farm", &["--root", ROOT, "result", "demo", "submit", "--input-file", "submit.json"]);
        if last.0 { break }
        std::thread::sleep(Duration::from_millis(100));
    }
    publish("probe-2.txt", &format!("submitted {} {}\n", last.0, last.1));
    loop { std::thread::park() }
}
"#;

impl Lab {
    /// Replace the lab agent with `PROBE_AGENT`, its paths compiled in.
    fn write_probe_agent(&self, secrets: &[PathBuf], writes: &[PathBuf], tmp: &std::path::Path, hidden: &[PathBuf], nested: &str) {
        let quoted = |paths: &[PathBuf]| paths.iter().map(|p| format!("{:?}", p.to_str().unwrap())).collect::<Vec<_>>().join(",");
        let root = self.path("root").canonicalize().unwrap();
        let source = format!("{PROBE_AGENT}\nconst WRITES: &[&str] = &[{}];\nconst TMP_WRITE: &str = {:?};\nconst SECRETS: &[&str] = &[{}];\nconst HIDDEN: &[&str] = &[{}];\nconst ROOT: &str = {:?};\nconst OWN: &str = {:?};\nconst SOCKET: &str = {:?};\nconst NESTED: &str = {nested:?};\n",
            quoted(writes), tmp.to_str().unwrap(), quoted(secrets), quoted(hidden), root.to_str().unwrap(), self.project.canonicalize().unwrap().join(".state").to_str().unwrap(), self.socket().to_str().unwrap());
        self.build_agent(&source);
    }
    /// Replace the lab agent with the Rust program `source`.
    fn build_agent(&self, source: &str) {
        let (agent, file) = (self.agent(), self.path("bin/probe.rs"));
        fs::write(&file, source).unwrap();
        let built = Command::new("rustc").args(["--edition", "2021", "-o"]).arg(&agent).arg(&file).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    }
    /// Install an owner-signed contract for the queued task `work` whose one
    /// output is `work.txt`, routed `route`; returns (contract digest, base commit).
    fn install_work_contract(&self, route: &str) -> (String, String) {
        self.install_work_contract_policy(route, WORK_POLICY)
    }
    fn install_work_contract_policy(&self, route: &str, policy: &str) -> (String, String) {
        let base = self.git(&["rev-parse", "HEAD"]);
        let store = self.project.join(".state/state.db").canonicalize().unwrap().display().to_string();
        let mut document = serde_json::to_vec_pretty(&json!({
            "version": 3, "outputs": [{"path": "work.txt", "kind": "git_file"}], "scope": {"paths": [{"path": "work.txt", "access": "write"}]},
            "project_store": store, "expected_head": self.head(), "task_id": "work", "contract_revision": 1, "deliverable": "work", "non_goals": "none",
            "acceptance_policies": [{"id": "clean", "text": policy}],
            "repository": self.repo.canonicalize().unwrap().display().to_string(), "base_oid": base,
            "object_format": "sha256", "dependencies": [], "capability_flags": [], "profile_kind": "claude", "retry_class": "none", "result_schema_id": "result-v1",
            "route": route, "authority": authority::policy_reference(&self.project).unwrap()})).unwrap();
        document.push(b'\n');
        let contract = self.path("work-contract.json");
        fs::write(&contract, &document).unwrap();
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&self.key).args(["-n", authority::CONTRACT_SIGNATURE_NAMESPACE]).arg(&contract).output().unwrap().status.success());
        let installed = self.ok(&["task", "demo", "contract", "put", "--input-file", contract.to_str().unwrap(), "--signature", self.path("work-contract.json.sig").to_str().unwrap()]);
        (installed["digest"].as_str().unwrap().to_owned(), base)
    }
}

/// The `work` contract's one acceptance policy.
const WORK_POLICY: &str = r#"{"version":1,"checks":["/usr/bin/git","diff","--quiet"]}"#;

impl Lab {
    /// Whether `git args` succeeds in the lab repository.
    fn git_ok(&self, args: &[&str]) -> bool {
        Command::new("/usr/bin/git").env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("PATH", "/usr/bin:/bin").env("HOME", self.home.path())
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").current_dir(&self.repo).args(args)
            .output().unwrap().status.success()
    }
    /// The attempt branch `launch draft` planned for `attempt`.
    fn attempt_branch(attempt: &AttemptId) -> String { format!("hp-{}", attempt.as_str().strip_prefix("attempt-").unwrap()) }
    /// Where the sandbox keeps the Git quarantine of `worktree`.
    fn quarantine(&self, worktree: &std::path::Path) -> PathBuf {
        let project = self.project.canonicalize().unwrap();
        project.join(".git-quarantine").join(worktree.strip_prefix(project.join(".state/worktrees")).unwrap())
    }
}

/// Plant `text` at `path` as a private file in a private directory.
fn plant(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// A Codex worker stand-in. It records its arguments, then, when `REAL_CODEX`
/// names a real Codex binary, commits in its worktree through Codex's own
/// `workspace-write` sandbox (`codex sandbox`, no model call) twice: once
/// with only the Git common directory as a writable root (`CONTROL`, the live
/// run's configuration) and once with the `-c` overrides the product passed.
/// It publishes `codex-probe.txt` in its worktree and stays up.
const CODEX_AGENT: &str = r#"
use std::{fs, process::Command};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") { println!("codex-cli 0.159.3"); return }
    let mut report: String = args.iter().map(|a| format!("arg {a}\n")).collect();
    let mut overrides = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() && args[i] == "-c" { overrides.push(args[i + 1].clone()); i += 2; }
    report += &format!("path {}\n", std::env::var("PATH").unwrap_or_default());
    if !REAL_CODEX.is_empty() {
        for (name, set) in [("control", vec![CONTROL.to_owned()]), ("product", overrides.clone())] {
            fs::write(name, "x\n").unwrap();
            let mut command = Command::new(REAL_CODEX);
            command.arg("sandbox");
            for o in &set { command.args(["-c", o]); }
            command.args(["--", "/bin/sh", "-c", &format!("git add {name} && git -c user.name=w -c user.email=w@example.invalid commit -qm {name} && echo COMMITTED")]);
            let out = match command.output() { Ok(out) => format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)), Err(e) => e.to_string() };
            let out: Vec<&str> = out.lines().filter(|l| !l.contains("WARNING")).collect();
            report += &format!("commit {name} {}\n", out.join("|"));
        }
    }
    fs::write("codex-probe.txt.tmp", report).unwrap();
    fs::rename("codex-probe.txt.tmp", "codex-probe.txt").unwrap();
    loop { std::thread::park() }
}
"#;

/// F-commit (certificate-live.md §2.2): an isolated Codex attempt gets, before
/// its profile's own arguments, one `-c permissions.herdr-farm-worker.filesystem`
/// override naming the Git common directory, the worktree's administrative
/// directory `<common>/worktrees/<id>` (Codex binds a linked worktree's gitdir
/// read-only unless it is a writable root itself), its spool and its output
/// directory; the start observation still identifies the agent (it runs), and
/// the product binary's directory leads the agent's PATH.
///
/// With `HP_CODEX_SANDBOX_BIN` naming a real Codex 0.159.3 binary, the worker
/// also commits through Codex's own `workspace-write` sandbox inside the
/// product sandbox, no model call: with only the common directory writable
/// (the live run's configuration) Git fails on the administrative
/// directory's `index.lock` (read-only file system); with the product's
/// override the commit succeeds and lands in the attempt's Git quarantine,
/// never in the shared repository.
#[test]
fn an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox() {
    let mut lab = Lab::of_kind("codex", "unknown_usage='allow_with_warning'", |repo| repo.to_owned());
    let real = std::env::var("HP_CODEX_SANDBOX_BIN").unwrap_or_default();
    let common = lab.repo.canonicalize().unwrap().join(".git");
    let control = format!("permissions.herdr-farm-worker.filesystem={{ {:?} = \"write\" }}", common.to_str().unwrap());
    lab.build_agent(&format!("{CODEX_AGENT}\nconst REAL_CODEX: &str = {real:?};\nconst CONTROL: &str = {control:?};\n"));
    let base = lab.git(&["rev-parse", "HEAD"]);
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| worktree.join("codex-probe.txt").exists() && lab.attempt(&attempt).state == AttemptState::Running);
    let report = fs::read_to_string(worktree.join("codex-probe.txt")).unwrap();
    lab.stop(ticker);
    let gitdir = fs::read_to_string(worktree.join(".git")).unwrap().trim().strip_prefix("gitdir: ").unwrap().to_owned();
    assert_eq!(std::path::Path::new(&gitdir).parent().unwrap(), common.join("worktrees"));
    let project = lab.project.canonicalize().unwrap();
    let roots = [common.display().to_string(), gitdir.clone(), format!("{}/.state/spool/{}", project.display(), attempt.as_str()),
        format!("{}/.state/worker-output/{}", project.display(), attempt.as_str())];
    let args: Vec<&str> = report.lines().filter_map(|l| l.strip_prefix("arg ")).collect();
    assert_eq!(args.len(), 2, "{report}");
    assert_eq!(args[0], "-c");
    let settings: toml::Value = toml::from_str(args[1]).unwrap();
    let grants = settings["permissions"]["herdr-farm-worker"]["filesystem"].as_table().unwrap();
    assert_eq!(grants.len(), roots.len());
    for root in roots { assert_eq!(grants[&root].as_str(), Some("write"), "{report}"); }
    let product = std::path::Path::new(BIN).canonicalize().unwrap();
    assert!(report.contains(&format!("\npath {}:/usr/bin:/bin\n", product.parent().unwrap().display())), "{report}");
    if real.is_empty() { return; }
    assert!(report.contains("commit control ") && report.contains("index.lock': Read-only file system"), "{report}");
    assert!(report.contains("commit product COMMITTED\n"), "{report}");
    // The commit landed in the attempt's quarantine only.
    assert_eq!(lab.git(&["rev-parse", &format!("refs/heads/{}", Lab::attempt_branch(&attempt))]), base);
}

/// The owner decision "isolate workers first": a canonical worker launched
/// through the real ticker path cannot read the owner's SSH/GnuPG keys, the
/// owner's own Codex login, the product configuration (owner policy and the
/// reviewer-signer directory), an owner signing key declared in
/// `[worker_isolation]`, another project under the root, the Herdr stand-in's
/// directory or its control socket, an SSH agent directory in `/tmp`, nor lift the hiding with `umount`/`mount`,
/// directly or from a namespace of its own. Its own execution home, worktree
/// commit and `result submit` into its own project (through its submission
/// spool: the project store is read-only to it) still work.
///
/// Git quarantine: the worker's Git writes (loose objects, refs, config,
/// `config.worktree`, hooks) land in its per-attempt quarantine, never in the
/// shared repository: moving `master`, another attempt's branch or the
/// integration target, and overwriting an existing loose object, change
/// nothing there, and its commit stays invisible to the owner until imported.
/// The candidate is imported (re-hashed, fsck-checked) when the ticker
/// ingests the spooled submission, so verification and integration work while
/// the worker runs, and its branch after it ends; an object the branch does
/// not reach is never imported.
#[test]
fn an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'\n[worker_isolation]\nhide=['~/owner']");
    let home = lab.home.path().canonicalize().unwrap();
    lab.ok(&["new", "--legacy", "other"]);
    let secrets = [
        (home.join(".ssh/id_owner"), "SENTINEL-SSH-KEY"),
        (home.join(".gnupg/private-keys-v1.d/key"), "SENTINEL-GNUPG"),
        (home.join(".codex/auth.json"), "SENTINEL-OWNER-CODEX-AUTH"),
        (home.join(".config/herdr-farm/review-signer/reviewer"), "SENTINEL-REVIEW-SIGNER"),
        (home.join(".config/herdr-farm/notes"), "SENTINEL-CONFIG-DIR"),
        (home.join("root/other/SECRET.txt"), "SENTINEL-OTHER-PROJECT"),
        (home.join("lab/secret"), "SENTINEL-HERDR-DIR"),
    ];
    for (path, text) in &secrets { plant(path, text); }
    // An SSH agent directory in /tmp, as ssh-agent creates it.
    let agent_socket_dir = tempfile::Builder::new().prefix("ssh-").tempdir_in("/tmp").unwrap();
    let agent_socket = agent_socket_dir.path().canonicalize().unwrap().join("agent.1");
    plant(&agent_socket, "SENTINEL-SSH-AGENT");
    plant(&home.join("agent-home/.codex/auth.json"), "WORKER-OWN-AUTH");
    fs::set_permissions(home.join("agent-home"), fs::Permissions::from_mode(0o700)).unwrap();
    let owner_key = fs::read_to_string(&lab.key).unwrap();
    let mut reads: Vec<PathBuf> = secrets.iter().map(|(p, _)| p.clone()).collect();
    reads.extend([agent_socket.clone(), lab.key.clone(), home.join(".config/herdr-farm/config.toml"), home.join("agent-home/.codex/auth.json")]);
    // Files the owner's shell, systemd or Git later run, and the repository's
    // working tree: none may change from inside the worker.
    let repo_git = lab.repo.canonicalize().unwrap().join(".git");
    fs::write(home.join(".bashrc"), "OWNER-BASHRC\n").unwrap();
    for dir in [".local/bin", ".config/systemd/user"] { fs::create_dir_all(home.join(dir)).unwrap(); }
    let writes = [home.join(".bashrc"), home.join(".local/bin/x"), home.join(".config/systemd/user/x.service"), lab.repo.canonicalize().unwrap().join("README")];
    // Shared Git files the worker writes through its quarantined view; the
    // base commit is a loose object, `hp-other` stands for another attempt's
    // branch and `integration` is the integration target.
    let base_commit = lab.git(&["rev-parse", "HEAD"]);
    lab.git(&["branch", "hp-other"]);
    lab.git(&["branch", "integration"]);
    fs::write(lab.repo.join("owner.txt"), "owner object\n").unwrap();
    let owned = lab.git(&["hash-object", "-w", "owner.txt"]);
    fs::remove_file(lab.repo.join("owner.txt")).unwrap();
    let git_files = [repo_git.join("hooks/pre-commit"), repo_git.join("config"), repo_git.join("info/exclude"), repo_git.join("packed-refs"),
        repo_git.join(format!("objects/{}/{}", &owned[..2], &owned[2..]))];
    let owner_files = || writes.iter().chain(&git_files).map(|p| fs::read(p).ok()).collect::<Vec<_>>();
    let before = owner_files();
    // The worker's /tmp is private: a host file there is invisible and a file
    // the worker writes there never reaches the host.
    let host_tmp = tempfile::Builder::new().prefix("herdr-farm-host-").tempdir_in("/tmp").unwrap();
    plant(&host_tmp.path().join("secret"), "SENTINEL-HOST-TMP");
    reads.push(host_tmp.path().join("secret"));
    let worker_tmp = PathBuf::from(format!("{}-worker", host_tmp.path().display()));
    let hidden = [home.join(".ssh"), home.join(".codex"), home.join(".config/herdr-farm"), home.join("lab"), home.join("root")];
    let nested = format!("for d in {0}; do umount \"$d\" 2>/dev/null && echo LIFTED-$d; umount -l \"$d\" 2>/dev/null && echo LIFTED-$d; done; \
        mkdir -p nested && mount --bind {1} nested 2>/dev/null && cat nested/.ssh/id_owner; mount --rbind {1} nested 2>/dev/null && cat nested/.ssh/id_owner nested/owner; \
        cat {2} {3}; echo nested-done",
        hidden.iter().map(|p| format!("'{}'", p.display())).collect::<Vec<_>>().join(" "), home.display(), home.join(".ssh/id_owner").display(), home.join("root/other/SECRET.txt").display());
    // Also the project's retained knowledge and the worktree's own `.git`
    // pointer (relative to the worker's working directory, its worktree).
    let project_writes = [lab.project.canonicalize().unwrap().join("PROJECT.md"), PathBuf::from(".git")];
    lab.write_probe_agent(&reads, &[&writes[..], &project_writes[..]].concat(), &worker_tmp, &hidden, &nested);
    let (contract, base) = lab.install_work_contract("verify_then_integrate");
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| worktree.join("probe-1.txt").exists());
    let report = fs::read_to_string(worktree.join("probe-1.txt")).unwrap();
    for expected in ["GIT_CONFIG_COUNT 3", "GIT_CONFIG_KEY_0 gc.auto", "GIT_CONFIG_VALUE_0 0", "GIT_CONFIG_KEY_1 gc.autoDetach", "GIT_CONFIG_VALUE_1 false", "GIT_CONFIG_KEY_2 maintenance.auto", "GIT_CONFIG_VALUE_2 false"] {
        assert!(report.contains(&format!("env {expected}\n")), "{report}");
    }
    for expected in ["gc.auto 0", "gc.autoDetach false", "maintenance.auto false"] {
        assert!(report.contains(&format!("gitconfig {expected}|\n")), "{report}");
    }
    // Nothing secret reached the worker, directly or through its own namespace.
    for sentinel in secrets.iter().map(|(_, s)| *s).chain([owner_key.lines().nth(1).unwrap(), "SENTINEL-SSH-AGENT", "SENTINEL-HOST-TMP", "LIFTED"]) {
        assert!(!report.contains(sentinel), "{sentinel} reached the worker:\n{report}");
    }
    // A hidden directory is an empty read-only tmpfs; a hidden file (the
    // declared owner key) reads as `/dev/null`.
    for path in reads.iter().filter(|p| **p != lab.key && !p.starts_with(home.join("agent-home"))) {
        assert!(report.contains(&format!("read {} ERR:", path.display())), "{} was readable:\n{report}", path.display());
    }
    assert!(report.contains(&format!("read {} OK:\n", lab.key.display())), "{report}");
    assert!(report.contains(&format!("read {} OK:WORKER-OWN-AUTH", home.join("agent-home/.codex/auth.json").display())), "{report}");
    // Every owner write failed and changed nothing; /tmp is private.
    for path in &writes {
        assert!(report.contains(&format!("write {} ERR:", path.display())), "{} was writable:\n{report}", path.display());
    }
    for path in &project_writes {
        assert!(report.contains(&format!("write {} ERR:", path.display())), "{} was writable:\n{report}", path.display());
    }
    assert_eq!(owner_files(), before);
    assert_eq!(fs::read_to_string(lab.project.join("PROJECT.md")).unwrap(), "Retained instructions");
    assert!(!fs::read_to_string(worktree.join(".git")).unwrap().contains("WORKER-WROTE"));
    assert!(report.contains("\ntmp true private\n") && !worker_tmp.exists(), "{report}");
    let listed = report.lines().find_map(|l| l.strip_prefix("tmp-list ")).unwrap();
    for name in [host_tmp.path(), agent_socket_dir.path()].map(|p| p.file_name().unwrap().to_str().unwrap()) {
        assert!(!listed.split(',').any(|n| n == name), "{name} visible in the worker's /tmp: {listed}");
    }
    // Only its own project and the shared root lock remain under the root; the
    // Herdr control socket is unreachable.
    assert!(report.contains("\nroot .execution.lock,demo\n"), "{report}");
    assert!(report.contains("\nown ") && report.contains("state.db"), "{report}");
    assert!(report.contains("\nsocket ERR:"), "{report}");
    // Every direct unmount/mount fails (EPERM) and the nested namespace
    // neither unmounts the locked hiding nor binds around it.
    for dir in &hidden {
        assert!(report.contains(&format!("unmount {} ERR1 ERR1\n", dir.display())), "{}:\n{report}", dir.display());
        assert!(report.contains(&format!("mount {} ERR1\n", dir.display())), "{}:\n{report}", dir.display());
    }
    assert!(report.contains("nested-done"), "{report}");
    // Normal work: a commit on the attempt branch, in the worker's quarantine.
    let candidate = report.lines().find_map(|l| l.strip_prefix("head ")).unwrap().to_owned();
    let stray = report.lines().find_map(|l| l.strip_prefix("stray ")).unwrap().to_owned();
    assert!(report.contains("commit true"), "{report}");
    assert_eq!(fs::read_to_string(worktree.join("work.txt")).unwrap(), "worker change\n");
    // Every other Git write stayed in the quarantine: the shared files, the
    // loose base object and every other branch are unchanged, no
    // `config.worktree` appeared, and the commit is not shared yet.
    let branch = Lab::attempt_branch(&attempt);
    let refs = || ["master", "hp-other", "integration", branch.as_str()].map(|b| lab.git(&["rev-parse", &format!("refs/heads/{b}")]));
    for moved in ["master", "hp-other", "integration"] { assert!(report.contains(&format!("move {moved} true")), "{report}"); }
    for path in &git_files { assert!(report.contains(&format!("gitwrite {} OK", path.display())), "{report}"); }
    assert_eq!(refs(), [base_commit.clone(), base_commit.clone(), base_commit.clone(), base_commit.clone()]);
    let worktree_config = repo_git.join("worktrees").join(worktree.file_name().unwrap()).join("config.worktree");
    assert!(report.contains(&format!("gitwrite {} OK", worktree_config.display())) && !worktree_config.exists(), "{report}");
    assert_eq!(owner_files(), before);
    assert!(!lab.git_ok(&["cat-file", "-e", &candidate]) && !lab.git_ok(&["cat-file", "-e", &stray]));
    // ... and `result submit` from inside the sandbox records a submission of
    // the objects the worker sees (the ticker ingests it from the attempt's
    // spool, importing the candidate from the quarantine first).
    let objects: Vec<Value> = report.lines().find_map(|l| l.strip_prefix("objects ")).unwrap().split(',')
        .map(|oid| json!({"oid": oid, "relative_path": format!("{}/{}", &oid[..2], &oid[2..])})).collect();
    fs::write(worktree.join("submit.json.tmp"), json!({"idempotency_key": "isolated-worker", "task_id": "work", "contract_revision": 1, "contract_digest": contract,
        "attempt_id": attempt.as_str(), "repository": lab.repo.canonicalize().unwrap().display().to_string(), "base_oid": base, "candidate_oid": candidate, "object_format": "sha256",
        "artifact_manifest": [{"path": "work.txt", "oid": candidate}], "claimed_checks": [], "objects": objects}).to_string()).unwrap();
    fs::rename(worktree.join("submit.json.tmp"), worktree.join("submit.json")).unwrap();
    lab.wait(&mut ticker, 60, &|| worktree.join("probe-2.txt").exists());
    let submitted = fs::read_to_string(worktree.join("probe-2.txt")).unwrap();
    assert!(submitted.starts_with("submitted true"), "{submitted}");
    let shown = lab.ok(&["result", "demo", "show"]);
    assert_eq!(shown.as_array().map(|a| (a.len(), a[0]["candidate_oid"].clone(), a[0]["attempt_id"].clone())),
        Some((1, json!(candidate), json!(attempt.as_str()))), "{shown}");
    // To record it, the ticker imported the candidate from the quarantine
    // (re-hashed, fsck-checked): only its closure, never the stray object,
    // and no branch moved.
    assert_eq!(lab.git(&["cat-file", "-p", &format!("{candidate}:work.txt")]), "worker change");
    assert!(!lab.git_ok(&["cat-file", "-e", &stray]));
    assert_eq!(refs(), [base_commit.clone(), base_commit.clone(), base_commit.clone(), base_commit.clone()]);

    // The owner's files are untouched, before and after the submission.
    for (path, text) in &secrets { assert_eq!(&fs::read_to_string(path).unwrap(), text); }
    assert_eq!(owner_files(), before);
    // Verification runs on the submitted objects; integration, with the worker
    // still running, merges the imported candidate. The attempt branch itself
    // does not move yet.
    let submission = shown[0]["submission_id"].as_str().unwrap().to_owned();
    let policy = lab.path("policy.json");
    fs::write(&policy, WORK_POLICY).unwrap();
    let tries = std::cell::Cell::new(0);
    let fresh = |name: &str| { tries.set(tries.get() + 1); lab.path(&format!("{name}-{}", tries.get())).display().to_string() };
    let verified = lab.ok_live(&|| ["result", "demo", "verify", &submission, "--policy-id", "clean", "--policy-file", policy.to_str().unwrap(),
        "--idempotency-key", "isolated-verify", "--work-dir", &fresh("verify")].map(String::from).to_vec());
    assert_eq!(verified["state"], "accepted", "{verified}");
    let repository = lab.repo.canonicalize().unwrap().display().to_string();
    lab.ok_live(&|| ["result", "demo", "configure-integration", "--repository", &repository, "--reference", "refs/heads/integration"].map(String::from).to_vec());
    let result = verified["receipt"]["result_id"].as_str().unwrap().to_owned();
    let integrated = lab.ok_live(&|| ["result", "demo", "integrate", &result, "--repository", &repository, "--idempotency-key", "isolated-integrate",
        "--work-dir", &fresh("integrate")].map(String::from).to_vec());
    assert_eq!(integrated["state"], "integrated", "{integrated}");
    assert_eq!(lab.git(&["rev-parse", "integration^2"]), candidate);
    assert_eq!(lab.git(&["cat-file", "-p", &format!("{candidate}:work.txt")]), "worker change");
    assert_eq!(lab.git(&["rev-parse", &format!("refs/heads/{branch}")]), base_commit);
    // The released agent can finish before the ticker records its brief and
    // moves the attempt to Running (a new revision): cancel only from there,
    // with the attempt's current revision on every retry.
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &lab.attempt(&attempt).revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "probe done"].map(String::from).to_vec());
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    // Once the worker ended, its attempt branch (and only it) was imported;
    // the stray object it never committed was not.
    let imported: Value = serde_json::from_slice(&fs::read(lab.quarantine(&worktree).join("import.json")).unwrap()).unwrap();
    assert_eq!(imported, json!({"state": "imported", "commit": candidate}));
    let merge = lab.git(&["rev-parse", "integration"]);
    assert_eq!(refs(), [base_commit.clone(), base_commit.clone(), merge, candidate.clone()]);
    assert!(!lab.git_ok(&["cat-file", "-e", &stray]));
    assert_eq!(owner_files(), before);
    assert!(!worktree_config.exists());
    lab.git(&["fsck", "--strict", "--no-dangling"]);
}

/// A worker agent that commits `work.txt` and then, in its quarantined view,
/// replaces the committed blob's loose object with the bytes of another
/// valid object (`tampered`), so the branch it leaves names content that does
/// not hash to it. It publishes `probe-1.txt` (`head`, `blob`, `objects`),
/// submits the owner's `submit.json` through its spool (`submit-1.txt`:
/// success, then output) and waits.
const TAMPERING_AGENT: &str = r#"
use std::{fs, path::Path, process::Command, time::Duration};
fn git(args: &[&str]) -> String {
    let out = Command::new("/usr/bin/git").args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    fs::write("work.txt", "worker change\n").unwrap();
    git(&["add", "work.txt"]);
    git(&["commit", "-qm", "worker change"]);
    let (head, blob) = (git(&["rev-parse", "HEAD"]), git(&["rev-parse", "HEAD:work.txt"]));
    let objects = git(&["rev-list", "--objects", "HEAD"]).lines().map(|l| l.split(' ').next().unwrap().to_owned()).collect::<Vec<_>>().join(",");
    fs::write("tampered.txt", "tampered\n").unwrap();
    let tampered = git(&["hash-object", "-w", "tampered.txt"]);
    fs::remove_file("tampered.txt").unwrap();
    let objects = git(&["rev-parse", "--path-format=absolute", "--git-path", "objects"]);
    let path = |oid: &str| format!("{objects}/{}/{}", &oid[..2], &oid[2..]);
    fs::remove_file(path(&blob)).unwrap();
    fs::copy(path(&tampered), path(&blob)).unwrap();
    fs::write("probe-1.tmp", format!("head {head}\nblob {blob}\nobjects {objects}\n")).unwrap();
    fs::rename("probe-1.tmp", "probe-1.txt").unwrap();
    while !Path::new("submit.json").exists() { std::thread::sleep(Duration::from_millis(50)); }
    let out = Command::new(BIN).env("HERDR_FARM_TEST_TIME_SCALE", TEST_TIME_SCALE).args(["--root", ROOT, "result", "demo", "submit", "--input-file", "submit.json"]).output().unwrap();
    fs::write("submit-1.tmp", format!("{}\n{}{}", out.status.success(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))).unwrap();
    fs::rename("submit-1.tmp", "submit-1.txt").unwrap();
    loop { std::thread::park() }
}
"#;

/// A quarantine whose commit reaches a corrupt object is refused: a spooled
/// `result submit` naming that commit is answered with the refusal (and the
/// denial recorded) without any submission, and when the worker ends nothing
/// is imported, the attempt branch stays at its base and the refusal is
/// recorded beside the quarantine.
#[test]
fn a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'\n");
    lab.write_agent(TAMPERING_AGENT, &[("ROOT", lab.path("root").canonicalize().unwrap().display().to_string()), ("BIN", BIN.into())]);
    let (contract, base) = lab.install_work_contract("verify_only");
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| worktree.join("probe-1.txt").exists());
    let report = fs::read_to_string(worktree.join("probe-1.txt")).unwrap();
    let head = report.lines().find_map(|l| l.strip_prefix("head ")).unwrap().to_owned();
    let blob = report.lines().find_map(|l| l.strip_prefix("blob ")).unwrap().to_owned();
    let objects: Vec<Value> = report.lines().find_map(|l| l.strip_prefix("objects ")).unwrap().split(',')
        .map(|oid| json!({"oid": oid, "relative_path": format!("{}/{}", &oid[..2], &oid[2..])})).collect();
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).state == AttemptState::Running);
    fs::write(worktree.join("submit.json.tmp"), json!({"idempotency_key": "tampered", "task_id": "work", "contract_revision": 1, "contract_digest": contract,
        "attempt_id": attempt.as_str(), "repository": lab.repo.canonicalize().unwrap().display().to_string(), "base_oid": base, "candidate_oid": head, "object_format": "sha256",
        "artifact_manifest": [{"path": "work.txt", "oid": head}], "claimed_checks": [], "objects": objects}).to_string()).unwrap();
    fs::rename(worktree.join("submit.json.tmp"), worktree.join("submit.json")).unwrap();
    lab.wait(&mut ticker, 120, &|| worktree.join("submit-1.txt").exists());
    let submitted = fs::read_to_string(worktree.join("submit-1.txt")).unwrap();
    let refusal = format!("the candidate's Git quarantine was refused: objects reachable from {head} fail re-hashing, fsck or connectivity");
    assert!(submitted.starts_with("false\n") && submitted.contains(&format!("submission spool refused the request: {refusal}")), "{submitted}");
    assert_eq!(lab.spool_denials(attempt.as_str()), vec![refusal]);
    assert_eq!(lab.ok(&["result", "demo", "show"]), json!([]));
    assert!(!lab.git_ok(&["cat-file", "-e", &head]) && !lab.git_ok(&["cat-file", "-e", &blob]));
    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &lab.attempt(&attempt).revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "probe done"].map(String::from).to_vec());
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let outcome: Value = serde_json::from_slice(&fs::read(lab.quarantine(&worktree).join("import.json")).unwrap()).unwrap();
    assert_eq!(outcome["state"], "refused", "{outcome}");
    assert!(outcome["reason"].as_str().unwrap().contains(&format!("reachable from {head}")), "{outcome}");
    assert_eq!(lab.git(&["rev-parse", &format!("refs/heads/{}", Lab::attempt_branch(&attempt))]), base);
    assert!(!lab.git_ok(&["cat-file", "-e", &head]) && !lab.git_ok(&["cat-file", "-e", &blob]));
    lab.git(&["fsck", "--strict", "--no-dangling"]);
}

/// The rendered prefix also works before dispatch, using a real sealed attempt.
#[test]
fn rendered_memory_commands_read_the_reserved_attempt() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    let brief = lab.ok(&["memory", "demo", "attempt-brief", "--attempt", attempt.as_str()]);
    let text = brief["text"].as_str().unwrap();
    let card = text.split("## Worker commands").nth(1).unwrap().split("```sh\n").nth(1).unwrap().split("```").next().unwrap();
    let setup = card.split("$M attempt-brief").next().unwrap();
    drop(herdr_farm::telemetry::sidecar::open(&lab.project, true).unwrap());
    herdr_farm::submission_spool::prepare(&lab.project, &attempt).unwrap();
    let spool = lab.project.join(".state/spool").join(attempt.as_str());
    // Model the sandbox's hidden owner config without requiring mount privileges.
    let config = lab.home.path().join(".config/herdr-farm/config.toml");
    let saved_config = fs::read(&config).unwrap();
    fs::remove_file(&config).unwrap();
    for command in ["attempt-brief", "attempt-input"] {
        let denied = lab.cli(&["memory", "demo", command, "--attempt", attempt.as_str()]);
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("worker configuration changed since approval"));
    }
    let script = format!("{setup}\nset -e\n$M attempt-brief --attempt $A\n$M attempt-input --attempt $A\n$M receipts --attempt $A\nif $M attempt-input --attempt; then exit 42; fi\n");
    let out = Command::new("/bin/bash").env_clear()
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .env("HOME", lab.home.path())
        .env("HERDR_FARM_SUBMISSION_SPOOL", &spool)
        .env("HERDR_FARM_WORKER_OUTPUT", lab.project.join(".state/worker-output").join(attempt.as_str()))
        .env("PATH", format!("{}:/usr/bin:/bin", std::path::Path::new(BIN).parent().unwrap().display()))
        .current_dir(&lab.repo).args(["-c", &script]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let values = serde_json::Deserializer::from_slice(&out.stdout).into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0], brief);
    assert_eq!(values[1]["attempt_id"], attempt.as_str());
    assert!(values[1]["text"].as_str().unwrap().contains("Retained instructions"));
    assert!(values[2].is_array());
    fs::write(&config, saved_config).unwrap();
    // Reproduce the former card with the same valid attempt: quotes inside a
    // variable survive word splitting and become part of the root argument.
    let old_prefix = format!("M=\"herdr-farm --root '{}' memory demo\"; A={}; $M attempt-brief --attempt $A",
        lab.path("root").display(), attempt.as_str());
    let old = Command::new("/bin/bash").env_clear()
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .env("HOME", lab.home.path()).env("HERDR_FARM_SUBMISSION_SPOOL", &spool)
        .env("PATH", format!("{}:/usr/bin:/bin", std::path::Path::new(BIN).parent().unwrap().display()))
        .current_dir(&lab.repo).args(["-c", &old_prefix]).output().unwrap();
    assert!(!old.status.success());
    assert!(String::from_utf8_lossy(&old.stderr).contains("uses legacy-markdown memory"),
        "{}", String::from_utf8_lossy(&old.stderr));
    herdr_farm::submission_spool::ingest_with_cli_paths(&lab.project,
        &["memory attempt-brief".into(), "memory attempt-input".into(), "memory receipts".into()]).unwrap();
    let sidecar = rusqlite::Connection::open(herdr_farm::telemetry::sidecar::path(&lab.project)).unwrap();
    let operator_preconditions:i64 = sidecar.query_row("SELECT count(*) FROM cli_invocations WHERE command_path IN ('memory attempt-brief','memory attempt-input') AND caller='operator' AND error_class='precondition' AND outcome='error'", [], |r| r.get(0)).unwrap();
    assert_eq!(operator_preconditions, 2);
    let failures:i64 = sidecar.query_row("SELECT count(*) FROM cli_invocations WHERE command_path='memory attempt-input' AND caller='worker' AND trust='worker_reported' AND error_class='usage' AND outcome='usage_error'", [], |r| r.get(0)).unwrap();
    assert_eq!(failures, 1);
    let missing:i64 = sidecar.query_row("SELECT count(*) FROM cli_invocations WHERE command_path='memory attempt-brief' AND caller='worker' AND trust='worker_reported' AND error_class='precondition' AND outcome='error'", [], |r| r.get(0)).unwrap();
    assert_eq!(missing, 1);
    let successes:i64 = sidecar.query_row("SELECT count(*) FROM cli_invocations WHERE caller='worker' AND trust='worker_reported' AND outcome='ok' AND error_class IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(successes, 3);
}

/// Probe for the submission spool: reports which `.state` paths it can
/// write, copies the host's planted requests (and links) into its own spool
/// as `plan.txt` lists them, then runs `result submit` twice.
const SPOOL_PROBE: &str = r##"
use std::{fs, os::unix::fs::symlink, path::Path, process::Command, time::Duration};
fn publish(name: &str, text: &str) { fs::write(format!("{name}.tmp"), text).unwrap(); fs::rename(format!("{name}.tmp"), name).unwrap(); }
fn create(path: &str) -> String { match fs::OpenOptions::new().create(true).write(true).open(path) { Ok(_) => "OK".into(), Err(e) => format!("ERR:{:?}", e.kind()) } }
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    let spool = std::env::var("HERDR_PROJECTS_SUBMISSION_SPOOL").unwrap_or_default();
    let own = Path::new(&spool).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut report = format!("spool {spool}\n");
    report += &format!("store-write {}\n", match fs::OpenOptions::new().write(true).open(format!("{STATE}/state.db")) { Ok(_) => "OK".into(), Err(e) => format!("ERR:{:?}", e.kind()) });
    for (label, path) in [("state", format!("{STATE}/planted")), ("objects", format!("{STATE}/factory-objects/planted")),
        ("other-spool", format!("{STATE}/spool/{OTHER}/planted")), ("other-output", format!("{STATE}/worker-output/{OTHER}/planted")),
        ("own-output", format!("{STATE}/worker-output/{own}/report.md")), ("own-spool", format!("{spool}/.probe"))] {
        report += &format!("write {label} {}\n", create(&path));
    }
    while !Path::new("memory-card.sh").exists() { std::thread::sleep(Duration::from_millis(50)); }
    let out = Command::new("/bin/bash").env("HERDR_FARM_TEST_TIME_SCALE", TEST_TIME_SCALE).arg("memory-card.sh").output().unwrap();
    publish("memory-card-result.txt", &format!("{}\n{}{}", out.status.success(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    publish("probe-1.txt", &report);
    while !Path::new("plan.txt").exists() { std::thread::sleep(Duration::from_millis(50)); }
    for line in fs::read_to_string("plan.txt").unwrap().lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        let target = format!("{spool}/{}", parts[2]);
        if parts[0] == "link" { symlink(parts[1], &target).unwrap(); continue }
        fs::copy(parts[1], format!("{spool}/.copy")).unwrap();
        fs::rename(format!("{spool}/.copy"), &target).unwrap();
    }
    while !Path::new("submit.json").exists() { std::thread::sleep(Duration::from_millis(50)); }
    for n in 1..=2 {
        let mut last = String::new();
        for _ in 0..60 {
            let out = Command::new(BIN).env("HERDR_FARM_TEST_TIME_SCALE", TEST_TIME_SCALE).args(["--root", ROOT, "result", "demo", "submit", "--input-file", "submit.json"]).output().unwrap();
            last = format!("{}\n{}{}", out.status.success(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            if out.status.success() { break }
            std::thread::sleep(Duration::from_millis(200));
        }
        publish(&format!("submit-{n}.txt"), &last);
    }
    loop { std::thread::park() }
}
"##;

/// Probe for the reviewer's worker channel through the spool: its own
/// session, another attempt's session, its opportunity's blind view, and
/// the same receipt submitted twice.
const REVIEW_PROBE: &str = r##"
use std::{fs, path::Path, process::Command};
fn publish(name: &str, text: &str) { fs::write(format!("{name}.tmp"), text).unwrap(); fs::rename(format!("{name}.tmp"), name).unwrap(); }
fn field(json: &str, key: &str) -> String {
    let marker = format!("\"{key}\": \"");
    json.find(&marker).map(|i| json[i + marker.len()..].split('"').next().unwrap().to_owned()).unwrap_or_default()
}
fn review(args: &[&str]) -> String {
    let out = Command::new(BIN).env("HERDR_FARM_TEST_TIME_SCALE", TEST_TIME_SCALE).args(["--root", ROOT, "telemetry", "demo", "review"]).args(args).output().unwrap();
    format!("{}\n{}{}", out.status.success(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    let spool = std::env::var("HERDR_PROJECTS_SUBMISSION_SPOOL").unwrap_or_default();
    let attempt = Path::new(&spool).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let session = review(&["session", "--attempt", &attempt]);
    publish("session.txt", &session);
    publish("foreign.txt", &review(&["session", "--attempt", FOREIGN]));
    publish("present.txt", &review(&["present", &field(&session, "opportunity_id")]));
    fs::write("receipt.json", format!("{{\"schema\":\"review_receipt.v1\",\"session_id\":\"{}\",\"submission_id\":\"{}\",\"candidate_oid\":\"{}\",\"outcome\":\"completed\",\"findings\":[\"finding:w1\"],\"evidence\":[]}}",
        field(&session, "session_id"), field(&session, "submission_id"), field(&session, "candidate_oid"))).unwrap();
    publish("submit-1.txt", &review(&["submit", "--input-file", "receipt.json"]));
    publish("submit-2.txt", &review(&["submit", "--input-file", "receipt.json"]));
    loop { std::thread::park() }
}
"##;

impl Lab {
    /// Compile `source` (a probe above) as the lab agent, with `consts` appended.
    fn write_agent(&self, source: &str, consts: &[(&str, String)]) {
        let mut text = format!("const TEST_TIME_SCALE: &str = {:?};\n{source}", include_str!("support/time-scale.txt").trim());
        for (name, value) in consts { text += &format!("const {name}: &str = {value:?};\n"); }
        let (agent, file) = (self.agent(), self.path("bin/probe.rs"));
        fs::write(&file, text).unwrap();
        let built = Command::new("rustc").args(["--edition", "2021", "-o"]).arg(&agent).arg(&file).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    }
    /// The `spool.request_denied` reasons recorded for `attempt`.
    fn spool_denials(&self, attempt: &str) -> Vec<String> {
        self.events("spool.request_denied").into_iter().filter(|e| e.entity == attempt).map(|e| e.payload["reason"].as_str().unwrap().to_owned()).collect()
    }
}

/// A spool request exactly as the worker's CLI writes it (canonical field
/// order), and its content-addressed name.
fn spool_request(kind: &str, attempt: &str, document: &str) -> (String, Vec<u8>) {
    let bytes = format!(r#"{{"version":1,"kind":"{kind}","attempt_id":"{attempt}","document":{}}}"#, serde_json::to_string(document).unwrap()).into_bytes();
    (format!("{:x}", Sha256::digest(&bytes)), bytes)
}

/// Residual risk 1 closed: the isolated worker's project `.state` is
/// read-only (it cannot open `state.db` for writing, add objects, or write
/// another attempt's spool or outputs); only its own spool and output
/// directory are writable. `result submit` inside the sandbox goes through
/// the spool: the ticker ingests it through the store's submission path, so
/// the receipt is the one a direct submission prints and the same document
/// replays (`replayed: true`, same submission). Requests the worker plants
/// by hand are refused, answered and recorded (`spool.request_denied`): a
/// malformed one, an oversized one, one naming another attempt, one whose
/// result names another attempt, and a request that is a symbolic link. A
/// symbolic link planted at a receipt name is replaced, never followed: its
/// target (the owner's `~/.bashrc`) is unchanged. After the attempt ends the
/// ticker removes its spool.
#[test]
fn an_isolated_worker_submits_only_through_its_own_spool() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    drop(herdr_farm::telemetry::sidecar::open(&lab.project, true).unwrap());
    let home = lab.home.path().canonicalize().unwrap();
    let state = lab.project.canonicalize().unwrap().join(".state");
    // Another attempt's spool and outputs, as gate release creates them.
    for dir in ["spool/other-attempt", "worker-output/other-attempt", "factory-objects"] { fs::create_dir_all(state.join(dir)).unwrap(); }
    fs::write(home.join(".bashrc"), "OWNER-BASHRC\n").unwrap();
    lab.write_agent(SPOOL_PROBE, &[("ROOT", lab.path("root").canonicalize().unwrap().display().to_string()), ("BIN", BIN.into()),
        ("STATE", state.display().to_string()), ("OTHER", "other-attempt".into())]);
    let (contract, base) = lab.install_work_contract("verify_only");
    let (_, attempt) = lab.reserve("Retained instructions");
    let worktree = lab.planned_worktree(&attempt);
    let spool = state.join("spool").join(attempt.as_str());
    let brief = lab.ok(&["memory", "demo", "attempt-brief", "--attempt", attempt.as_str()]);
    let text = brief["text"].as_str().unwrap();
    let card = text.split("## Worker commands").nth(1).unwrap().split("```sh\n").nth(1).unwrap().split("```").next().unwrap();
    let setup = card.split("$M attempt-brief").next().unwrap();
    let script = format!("{setup}\nset -e\n$M attempt-brief --attempt $A > brief.json\n$M attempt-input --attempt $A > input.json\n$M receipts --attempt $A > receipts.json\nif $M attempt-input --attempt; then exit 42; fi\n");
    lab.serve();
    let mut ticker = lab.spawn();
    // Launch must finish preparing the checkout before the test modifies it.
    lab.wait(&mut ticker, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    fs::write(worktree.join("memory-card.sh"), script).unwrap();
    lab.wait(&mut ticker, 120, &|| worktree.join("probe-1.txt").exists());
    let command_result = fs::read_to_string(worktree.join("memory-card-result.txt")).unwrap();
    assert!(command_result.starts_with("true\n"), "{command_result}");
    let read_json = |name| serde_json::from_slice::<Value>(&fs::read(worktree.join(name)).unwrap()).unwrap();
    assert_eq!(read_json("brief.json"), brief);
    assert_eq!(read_json("input.json")["attempt_id"], attempt.as_str());
    assert!(read_json("input.json")["text"].as_str().unwrap().contains("Retained instructions"));
    assert!(read_json("receipts.json").is_array());
    let report = fs::read_to_string(worktree.join("probe-1.txt")).unwrap();
    assert!(report.starts_with(&format!("spool {}\n", spool.display())), "{report}");
    for label in ["store-write", "write state", "write objects", "write other-spool", "write other-output"] {
        assert!(report.contains(&format!("\n{label} ERR:ReadOnlyFilesystem\n")), "{label}:\n{report}");
    }
    assert!(report.contains("\nwrite own-output OK\n") && report.contains("\nwrite own-spool OK\n"), "{report}");
    assert!(state.join("worker-output").join(attempt.as_str()).join("report.md").is_file());
    assert!(!state.join("planted").exists() && !state.join("spool/other-attempt/planted").exists());

    // Requests the worker plants by hand in its own spool.
    let plan = worktree.join("plan");
    fs::create_dir(&plan).unwrap();
    let mut lines = Vec::new();
    let mut plant = |label: &str, name: String, bytes: &[u8]| -> String {
        fs::write(plan.join(label), bytes).unwrap();
        lines.push(format!("copy\t{}\t{name}.request", plan.join(label).display()));
        name
    };
    let malformed = plant("malformed", format!("{:x}", Sha256::digest(b"not a request\n")), b"not a request\n");
    let big = vec![b'x'; 1024 * 1024 + 1];
    let oversized = plant("oversized", format!("{:x}", Sha256::digest(&big)), &big);
    let (name, bytes) = spool_request("result_submit", "other-attempt", "{}");
    let other_spool = plant("other-spool", name, &bytes);
    let (name, bytes) = spool_request("result_submit", attempt.as_str(), &json!({"attempt_id": "other-attempt"}).to_string());
    let other_result = plant("other-result", name, &bytes);
    let linked = format!("{:x}", Sha256::digest(b"linked"));
    // The receipt name of the malformed request is a link to the owner's file.
    let mut plan_text = format!("link\t{}\t{malformed}.receipt\nlink\t{}\t{linked}.request\n", home.join(".bashrc").display(), home.join(".bashrc").display());
    plan_text += &(lines.join("\n") + "\n");
    fs::write(worktree.join("plan.txt.tmp"), plan_text).unwrap();
    fs::rename(worktree.join("plan.txt.tmp"), worktree.join("plan.txt")).unwrap();
    let answered = |digest: &str| fs::symlink_metadata(spool.join(format!("{digest}.receipt"))).is_ok_and(|m| m.is_file());
    lab.wait(&mut ticker, 120, &|| [&malformed, &oversized, &other_spool, &other_result, &linked].iter().all(|d| answered(d)));
    let receipt = |digest: &str| -> Value { serde_json::from_slice(&fs::read(spool.join(format!("{digest}.receipt"))).unwrap()).unwrap() };
    for (digest, reason) in [(&malformed, "the request is not a canonical spool request"), (&oversized, "the request exceeds 1048576 bytes"),
        (&other_spool, "the request names another attempt than its spool"), (&other_result, "the result submission names attempt other-attempt, not this spool's attempt"),
        (&linked, "the request is a symbolic link")] {
        assert_eq!(receipt(digest)["error"], json!(format!("submission spool refused the request: {reason}")), "{digest}");
        assert!(lab.spool_denials(attempt.as_str()).iter().any(|r| r == reason), "{reason}: {:?}", lab.spool_denials(attempt.as_str()));
    }
    assert_eq!(fs::read_to_string(home.join(".bashrc")).unwrap(), "OWNER-BASHRC\n", "the planted receipt link was followed");
    assert!(lab.state().attempts.iter().all(|a| a.id.as_str() != "other-attempt"));

    // An ordinary `result submit` goes through the spool and replays.
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).state == AttemptState::Running);
    let objects: Vec<Value> = lab.git(&["rev-list", "--objects", "--all"]).lines()
        .map(|line| { let oid = line.split_whitespace().next().unwrap(); json!({"oid": oid, "relative_path": format!("{}/{}", &oid[..2], &oid[2..])}) }).collect();
    fs::write(worktree.join("submit.json.tmp"), json!({"idempotency_key": "spooled", "task_id": "work", "contract_revision": 1, "contract_digest": contract,
        "attempt_id": attempt.as_str(), "repository": lab.repo.canonicalize().unwrap().display().to_string(), "base_oid": base, "candidate_oid": base, "object_format": "sha256",
        "artifact_manifest": [{"path": "work.txt", "oid": base}], "claimed_checks": [], "objects": objects}).to_string()).unwrap();
    fs::rename(worktree.join("submit.json.tmp"), worktree.join("submit.json")).unwrap();
    lab.wait(&mut ticker, 120, &|| worktree.join("submit-2.txt").exists());
    let parse = |n: u32| -> Value {
        let text = fs::read_to_string(worktree.join(format!("submit-{n}.txt"))).unwrap();
        let (status, stdout) = text.split_once('\n').unwrap();
        assert_eq!(status, "true", "{text}");
        serde_json::from_str(stdout).unwrap()
    };
    let (first, second) = (parse(1), parse(2));
    let shown = lab.ok(&["result", "demo", "show"]);
    assert_eq!(shown.as_array().map(|a| (a.len(), a[0]["submission_id"].clone(), a[0]["attempt_id"].clone())),
        Some((1, first["submission_id"].clone(), json!(attempt.as_str()))), "{shown}");
    assert_eq!((&first["idempotency_key"], &first["replayed"], &second["replayed"]), (&json!("spooled"), &json!(false), &json!(true)));
    assert_eq!((&second["submission_id"], &second["payload_digest"]), (&first["submission_id"], &first["payload_digest"]));
    assert_eq!(first["payload_digest"], json!(format!("{:x}", Sha256::digest(fs::read(worktree.join("submit.json")).unwrap()))));

    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &lab.attempt(&attempt).revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "probe done"].map(String::from).to_vec());
    lab.wait(&mut ticker, 90, &|| lab.attempt(&attempt).termination_observed && !spool.exists());
    lab.stop(ticker);
    assert!(state.join("spool/other-attempt").is_dir(), "an unknown attempt's spool is not removed");
    let telemetry=rusqlite::Connection::open_with_flags(herdr_farm::telemetry::sidecar::path(&lab.project),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY|rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW).unwrap();
    let submissions:i64=telemetry.query_row("SELECT count(*) FROM cli_invocations WHERE command_path='result submit' AND caller='worker' AND trust='worker_reported' AND outcome='ok'",[],|r|r.get(0)).unwrap();
    assert_eq!(submissions,2,"both sandbox submissions are observed before ended-attempt spool removal");
    let failures:i64=telemetry.query_row("SELECT count(*) FROM cli_invocations WHERE command_path='memory attempt-input' AND caller='worker' AND trust='worker_reported' AND error_class='usage'",[],|r|r.get(0)).unwrap();
    assert_eq!(failures,1);
}

/// D9's worker channel through the spool: a sandboxed reviewing worker (the
/// ticker launched it; its project store is read-only) prints its own
/// session and its opportunity's blind view, and submits its receipt: a
/// proposal recorded as `worker:<attempt>`, the same receipt replaying.
/// Asking for another attempt's session is refused and recorded.
#[test]
fn a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    lab.write_agent(REVIEW_PROBE, &[("ROOT", lab.path("root").canonicalize().unwrap().display().to_string()), ("BIN", BIN.into()), ("FOREIGN", AUTHOR_ATTEMPT.into())]);
    let world = lab.review_world();
    let (_, attempt) = lab.reserve_selection(&world.selection);
    let worktree = lab.planned_worktree(&attempt);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 180, &|| worktree.join("submit-2.txt").exists());
    lab.stop(ticker);
    let read = |name: &str| -> (String, String) {
        let text = fs::read_to_string(worktree.join(name)).unwrap();
        let (status, rest) = text.split_once('\n').unwrap();
        (status.to_owned(), rest.to_owned())
    };
    let (status, session) = read("session.txt");
    assert_eq!(status, "true", "{session}");
    let session: Value = serde_json::from_str(&session).unwrap();
    assert_eq!((&session["session"]["opportunity_id"], &session["session"]["submission_id"], &session["session"]["candidate_oid"], &session["session"]["completed"]),
        (&json!(world.opportunity), &json!(world.submission), &json!(world.candidate), &json!(false)));
    let (status, foreign) = read("foreign.txt");
    assert_eq!(status, "false");
    assert!(foreign.contains("submission spool refused the request: review session asks for another attempt's session"), "{foreign}");
    assert_eq!(lab.spool_denials(attempt.as_str()), vec!["review session asks for another attempt's session".to_owned()],
        "{}", fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default());
    let (status, present) = read("present.txt");
    assert_eq!(status, "true", "{present}");
    assert_eq!(serde_json::from_str::<Value>(&present).unwrap()["presentation"]["opportunity_id"], json!(world.opportunity));
    let submitted = |n: u32| -> Value {
        let (status, out) = read(&format!("submit-{n}.txt"));
        assert_eq!(status, "true", "{out}");
        serde_json::from_str::<Value>(&out).unwrap()["completion"].clone()
    };
    let (first, second) = (submitted(1), submitted(2));
    assert_eq!((&first["trust"], &first["recorder_principal"], &first["replayed"], &second["replayed"], &second["receipt_digest"]),
        (&json!("proposal"), &json!(format!("worker:{}", attempt.as_str())), &json!(false), &json!(true), &first["receipt_digest"]));
    let completion = lab.review_show(None)["opportunities"][0]["sessions"][0]["completion"].clone();
    assert_eq!((&completion["trust"], &completion["recorder_principal"]), (&json!("proposal"), &json!(format!("worker:{}", attempt.as_str()))));
}

/// Isolation fails closed: an owner-declared hidden path that would cover the
/// profile's own execution home is refused before any worker is created.
#[test]
fn a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'\n[worker_isolation]\nhide=['~/agent-home']");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let log = || fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 90, &|| log().contains("worker isolation would hide"));
    lab.stop(ticker);
    assert_eq!(lab.count("workspace.create_command"), 0, "{:?}", lab.requests());
    assert!(lab.events("runtime.launch_target").is_empty() && lab.events("runtime.launch_started").is_empty());
    assert!(lab.attempt(&attempt).retains_capacity());
}


/// Running, paused and terminated attempts use the retained pane, with explicit
/// deletion on lifecycle changes. Claude usage is unavailable: unknown ○.
#[test]
fn canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let metadata = || lab.requests().into_iter().filter(|(m, _)| m == "pane.report_metadata").map(|(_, p)| p).collect::<Vec<_>>();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| metadata().iter().any(|p| p["tokens"]["telemetry"] == "claude ○"));
    let published = metadata();
    assert!(published.iter().all(|p| p["pane_id"] == "w1:p1" && p["source"] == "herdr-farm" && p["tokens"] == json!({"telemetry":"claude ○"})), "{published:?}");
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
    lab.ok_live(&|| { let s = lab.state(); ["runtime", "demo", "state", "paused", "--expected-revision", &s.control.unwrap().revision.to_string(), "--expected-head", &s.head.to_string()].map(String::from).to_vec() });
    lab.wait(&mut ticker, 90, &|| metadata().iter().any(|p| p["tokens"] == json!({"telemetry":null})));
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
    lab.stop(ticker);
    // Cleanup removed the process-local publish entry. A fresh ticker must
    // not issue another erase for the already cleared paused attempt.
    fs::write(lab.path("lab/requests"), "").unwrap();
    let running = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "sidebar lifecycle"]);
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 90, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Cancelled);
    assert_eq!(metadata().len(), 0, "{:?}", metadata());
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (0, 0));
    // Relinquishment after restart must not discover historical cleanup work.
    let state = lab.state();
    let owner = state.ownership.iter().find(|o| o.binding == lab.binding).unwrap();
    runtime::relinquish(&lab.project, &lab.binding, owner.revision, state.head, "sidebar relinquishment").unwrap();
    fs::write(lab.path("lab/requests"), "").unwrap();
    lab.run_passes(4);
    assert_eq!(metadata().len(), 0, "{:?}", metadata());
    // A public route edit advances the binding generation. Neither the old
    // pane nor the replacement may receive metadata from the retired attempt.
    let state = lab.state();
    let binding = state.runtime_bindings.iter().find(|b| b.id == lab.binding).unwrap();
    let mut route = RuntimeRoute::from_identity(&binding.identity);
    route.pane_id = "w1:p2".into();
    runtime::rebind(&lab.project, &binding.id, binding.revision, state.head, &route).unwrap();
    fs::write(lab.path("lab/requests"), "").unwrap();
    lab.run_passes(4);
    assert_eq!(lab.count("pane.report_metadata"), 0, "{:?}", lab.requests());
}

/// A native terminal replacement at the same pane ID must receive no metadata,
/// including cleanup. The old suffix expires via its native TTL.
#[test]
fn canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.count("pane.report_metadata") > 0);
    lab.stop(ticker);
    fs::write(lab.path("lab/changed-terminal"), "").unwrap();
    fs::write(lab.path("lab/requests"), "").unwrap();
    lab.run_passes(4);
    assert_eq!(lab.count("pane.report_metadata"), 0, "{:?}", lab.requests());
    // Stop only this fixture's worker through the normal public workflow.
    fs::remove_file(lab.path("lab/changed-terminal")).unwrap();
    let running = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "fixture cleanup"]);
    lab.run_until(20, &|| lab.attempt(&attempt).termination_observed);
}


/// The real Codex collector binds the fixture rollout to the launched attempt:
/// 1000 input + 120 output = 1120 tokens, hence complete ●, with no cost.
/// Two persisted blocked observations six seconds apart add exactly `6s`.
#[test]
fn canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting() {
    let mut lab = Lab::of_kind("codex", "unknown_usage='allow_with_warning'", |repo| repo.to_owned());
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let has_token = |token: &str| lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"] == json!({"telemetry":token}));
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| has_token("codex ○"));
    lab.stop(ticker);
    let state = lab.state();
    let binding = state.runtime_bindings.iter().find(|b| b.id == lab.binding).unwrap();
    let start = jiff::Timestamp::now();
    let rollout = lab.path("agent-home/.codex/sessions/2026/09/30/rollout-sidebar.jsonl");
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/telemetry/codex-0.154.0/head.jsonl");
    let text = fs::read_to_string(fixture).unwrap().replace("@SID@", "12345678-1234-1234-1234-123456789abc")
        .replace("@CWD@", &binding.identity.cwd).replace("@TS@", &start.to_string()).replace("@VERSION@", "0.154.0");
    fs::write(rollout, text).unwrap();
    lab.ok(&["telemetry", "demo", "collect"]);
    let projection = lab.ok(&["telemetry", "demo", "attempts", "--json"]);
    let record = projection["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == attempt.as_str()).unwrap();
    assert_eq!(record["usage"]["total_tokens"], 1120, "{record}");
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 90, &|| has_token("codex ●"));
    lab.stop(ticker);
    // The agent now waits on the human: the ticker's own attention sampler
    // (stock Herdr `agent_status`) opens the wait, and the suffix shows how
    // long it has been observed open, never extrapolated to now.
    fs::write(lab.path("lab/agent-status"), "blocked").unwrap();
    let waiting = || lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"]["telemetry"].as_str()
        .and_then(|t| t.strip_prefix("codex ● ")).is_some_and(|w| !w.is_empty() && w.ends_with(['s', 'm'])));
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 150, &waiting);
    fs::remove_file(lab.path("lab/agent-status")).unwrap();
    assert_eq!((lab.count("workspace.create_command"), lab.count("agent.prompt")), (1, 1));
    // Collector revocation keeps historical counters but clears the decoration
    // while the worker and its runtime ownership remain live.
    fs::write(lab.path("lab/requests"), "").unwrap();
    lab.ok_live(&|| ["telemetry", "demo", "collectors", "revoke", attempt.as_str()].map(String::from).to_vec());
    lab.wait(&mut ticker, 90, &|| lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"] == json!({"telemetry":null})));
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
    let running = lab.attempt(&attempt);
    lab.stop(ticker);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "fixture cleanup"]);
    lab.run_until(20, &|| lab.attempt(&attempt).termination_observed);
    let last = lab.requests().into_iter().rev().find(|(m, _)| m == "pane.report_metadata").unwrap().1;
    assert_eq!(last["tokens"], json!({"telemetry":null}));
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Cancelled);
}


/// Termination clears a published suffix independently of pause or revocation.
#[test]
fn canonical_attempt_sidebar_clears_after_termination_in_an_active_project() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"] == json!({"telemetry":"claude ○"})));
    fs::write(lab.path("lab/requests"), "").unwrap();
    lab.ok_live(&|| {let running=lab.attempt(&attempt); ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "sidebar termination"].map(String::from).to_vec()});
    lab.wait(&mut ticker, 90, &|| lab.attempt(&attempt).termination_observed && lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"] == json!({"telemetry":null})));
    assert_eq!(lab.state().control.unwrap().state, ProjectState::Active);
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Cancelled);
    let last = lab.requests().into_iter().rev().find(|(m, _)| m == "pane.report_metadata").unwrap().1;
    assert_eq!((last["pane_id"].clone(), last["tokens"].clone()), (json!("w1:p1"), json!({"telemetry":null})));
    lab.stop(ticker);
}

/// Both running bindings publish, and a retired missing server is harmless
/// across subsequent passes in the same ticker process.
#[test]
fn concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    lab.ok(&["scheduler", "demo", "policy", "--max-active-workers", "2", "--max-attempts-per-task", "3", "--expected-revision", &lab.state().scheduler.unwrap().policy.revision.to_string(), "--expected-head", &lab.head().to_string()]);
    lab.ok(&["task", "demo", "add", "next", "--title", "next", "--expected-head", &lab.head().to_string()]);
    lab.ok(&["task", "demo", "queue", "next", "--input-file", lab.path("queue.json").to_str().unwrap(), "--expected-revision", "1", "--expected-head", &lab.head().to_string()]);
    fs::create_dir(lab.path("next-server")).unwrap();
    let socket = lab.path("next-server/native.sock");
    let id = TaskId::new("next").unwrap();
    let revision = lab.state().tasks.iter().find(|t| t.id == id).unwrap().revision;
    let route = RuntimeRoute { socket: socket.display().to_string(), cwd: lab.repo.canonicalize().unwrap().display().to_string(), ..Default::default() };
    let binding = runtime::create_binding(&lab.project, Some(&id), Some(revision), lab.head(), &route).unwrap().binding.id;
    lab.resume();
    let (_, first) = lab.reserve("First instructions");
    let selection = lab.selection_for("next", &binding, "Second instructions");
    let (_, second) = lab.reserve_selection(&selection);
    lab.serve();
    let mut server = Ticker(Command::new("/usr/bin/python3").args(["-c", SERVER]).arg(&socket).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
    let second_metadata = || fs::read_to_string(lab.path("next-server/requests")).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).any(|v| v["method"] == "pane.report_metadata" && v["params"]["tokens"]["telemetry"] == "claude ○");
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.attempt(&first).state == AttemptState::Running && lab.attempt(&second).state == AttemptState::Running && second_metadata() && lab.requests().iter().any(|(m,p)| m == "pane.report_metadata" && p["tokens"]["telemetry"] == "claude ○"));
    lab.ok_live(&|| { let a = lab.attempt(&second); ["task", "demo", "cancel-attempt", second.as_str(), "--expected-revision", &a.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "missing server cleanup"].map(String::from).to_vec() });
    lab.wait(&mut ticker, 90, &|| lab.attempt(&second).termination_observed);
    server.0.kill().unwrap();
    server.0.wait().unwrap();
    fs::remove_file(&socket).unwrap();
    for _ in 0..3 {
        let observed = lab.state().observations.iter().map(|o| o.observed_unix_ms).max().unwrap();
        lab.wait(&mut ticker, 90, &|| lab.state().observations.iter().any(|o| o.observed_unix_ms > observed));
    }
    lab.stop(ticker);
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
    assert!(!log.contains("attempt token binding changed") && !log.contains("attempt token: No such file") && !log.contains(&format!("attempt token {}:", second.as_str())), "{log}");
}

/// Recurring maintenance and two running attempts cannot starve a stop.
#[test]
fn barrier_stop_gets_a_fair_turn_and_maintenance_keeps_advancing() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    lab.ok(&["scheduler", "demo", "policy", "--max-active-workers", "2", "--max-attempts-per-task", "3", "--expected-revision", &lab.state().scheduler.unwrap().policy.revision.to_string(), "--expected-head", &lab.head().to_string()]);
    lab.ok(&["task", "demo", "add", "next", "--title", "next", "--expected-head", &lab.head().to_string()]);
    lab.ok(&["task", "demo", "queue", "next", "--input-file", lab.path("queue.json").to_str().unwrap(), "--expected-revision", "1", "--expected-head", &lab.head().to_string()]);
    fs::create_dir(lab.path("next-server")).unwrap();
    let socket = lab.path("next-server/native.sock");
    let id = TaskId::new("next").unwrap();
    let revision = lab.state().tasks.iter().find(|t| t.id == id).unwrap().revision;
    let route = RuntimeRoute { socket: socket.display().to_string(), cwd: lab.repo.canonicalize().unwrap().display().to_string(), ..Default::default() };
    let binding = runtime::create_binding(&lab.project, Some(&id), Some(revision), lab.head(), &route).unwrap().binding.id;
    lab.resume();
    let (_, first) = lab.reserve("First instructions");
    let selection = lab.selection_for("next", &binding, "Second instructions");
    let (_, second) = lab.reserve_selection(&selection);
    lab.serve();
    let mut server = Ticker(Command::new("/usr/bin/python3").args(["-c", SERVER]).arg(&socket).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
    let mut ticker = lab.spawn();
    // Only both workers running matters here; sidebar tokens are covered by the
    // concurrent-attempt token lab and depend on the advisory window.
    lab.wait(&mut ticker, 120, &|| lab.attempt(&first).state == AttemptState::Running && lab.attempt(&second).state == AttemptState::Running);
    // Public guard ownership deterministically produces repeated service
    // contention, independent of scheduler timing or probe duration.
    let deadline = Instant::now() + Duration::from_secs(15);
    let guard = loop {
        match herdr_farm::execution_guard::ProjectGuard::acquire(&lab.project) {
            Ok(guard) => break guard,
            Err(error) => { assert!(Instant::now() < deadline, "{error:#}"); std::thread::sleep(Duration::from_millis(20)); }
        }
    };
    lab.wait(&mut ticker, 15, &|| fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default().contains("barrier stop service:") );
    // The logging minute is deliberately unscaled. Wait for a counted
    // summary, which proves multiple actual service failures were suppressed.
    lab.wait(&mut ticker, 75, &|| fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default().lines().any(|line| {
        line.contains("canonical service contention:") && line.contains("barrier stop service:")
            && line.rsplit_once("count=").and_then(|(_,n)|n.strip_suffix(')')).and_then(|n|n.parse::<u64>().ok()).is_some_and(|n|n>1)
    }));
    drop(guard);
    lab.ok_live(&|| { let a = lab.attempt(&second); ["task", "demo", "cancel-attempt", second.as_str(), "--expected-revision", &a.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "fair stop"].map(String::from).to_vec() });
    lab.wait(&mut ticker, 60, &|| lab.attempt(&second).termination_observed);
    assert_eq!(lab.attempt(&first).state, AttemptState::Running);
    for _ in 0..3 {
        let observed = lab.state().observations.iter().map(|o| o.observed_unix_ms).max().unwrap();
        lab.wait(&mut ticker, 60, &|| lab.state().observations.iter().any(|o| o.observed_unix_ms > observed));
    }
    server.0.kill().unwrap();
    server.0.wait().unwrap();
    lab.stop(ticker);
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
    let contention = log.lines().filter(|line| line.contains("canonical service contention:") && line.contains("barrier stop service:")).collect::<Vec<_>>();
    assert!(!contention.is_empty(), "{log}");
    assert!(contention.iter().all(|line| line.contains("count=")), "{log}");
    assert!(contention.len() <= 3, "repeated barrier contention flooded the log: {log}");
}

/// A restarted ticker must not discover historical cleanup work, even when
/// a cancelled worker's retained pane is still present.
#[test]
fn canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    lab.run_until(120, &|| lab.requests().iter().any(|(m, p)| m == "pane.report_metadata" && p["tokens"]["telemetry"] == "claude ○"));
    let running = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "expired sidebar fixture"]);
    lab.run_until(90, &|| lab.attempt(&attempt).termination_observed);
    let control = herdr_farm::store::controlled::ReadControl::new(Instant::now() + Duration::from_secs(10), Default::default());
    let mut store = migration::open_active_scoped(&lab.project, control).unwrap();
    assert!(store.attempt_tokens(&[]).unwrap().entries.is_empty());
    let cleanup = store.attempt_tokens(std::slice::from_ref(&lab.binding)).unwrap();
    assert_eq!(cleanup.entries.len(), 1);
    assert!(!cleanup.entries[0].publishing);
    drop(store);
    fs::write(lab.path("lab/requests"), "").unwrap();
    fs::write(lab.path("lab/request-ids"), "").unwrap();
    lab.run_passes(4);
    let ids = fs::read_to_string(lab.path("lab/request-ids")).unwrap();
    assert!(!ids.lines().any(|id| id.starts_with("attempt-tokens-")), "{ids}");
    assert_eq!(lab.count("pane.report_metadata"), 0, "{:?}", lab.requests());
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Cancelled);
}

#[test]
fn dedicated_worker_refuses_remote_manifest_before_its_brief() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    fs::write(lab.path("lab/manifest-source"), "remote:/fixture/herdr/agent-detection/remote/claude.toml").unwrap();
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 120, &|| lab.count("agent.explain") >= 1);
    lab.stop(ticker);
    assert_eq!(lab.count("agent.prompt"), 0);
    // Readiness is judged on the launched agent's pane, so launch has started;
    // the refusal must stop the brief: no prompt, not running, no delivery.
    assert_ne!(lab.attempt(&attempt).state, AttemptState::Running);
    let state = lab.state();
    assert!(!state.deliveries.iter().any(|d| d.state == DeliveryState::Confirmed
        && state.operations.iter().any(|o| o.id == d.operation && o.kind == "runtime.worker_brief")), "no brief delivery is confirmed");
}

#[test]
fn canonical_inbox_wait_timeout_is_read_only_and_rejects_excessive_timeout() {
    let lab = Lab::new("unknown_usage='allow_with_warning'");
    let before = lab.state();
    assert_eq!(lab.ok(&["inbox", "demo", "wait", "--timeout", "1"]), json!({"items":0,"timed_out":true}));
    assert_eq!(lab.state(), before);
    assert!(!lab.cli(&["inbox", "demo", "wait", "--timeout", "7201"]).status.success());
}

const REVIEW_AUTO_PROBE: &str = r##"
use std::{fs, path::Path, process::Command};
fn field(json: &str, key: &str) -> String {
    let marker = format!("\"{key}\": \"");
    let start = json.find(&marker).unwrap() + marker.len();
    json[start..].split('"').next().unwrap().to_owned()
}
fn review(args: &[&str]) -> String {
    let out = Command::new(BIN).env("HERDR_FARM_TEST_TIME_SCALE", TEST_TIME_SCALE).args(["--root", ROOT, "telemetry", "demo", "review"]).args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("{VERSION}"); return }
    let spool = std::env::var("HERDR_FARM_SUBMISSION_SPOOL").unwrap();
    let attempt = Path::new(&spool).file_name().unwrap().to_str().unwrap();
    let session = review(&["session", "--attempt", attempt]);
    let view = review(&["present", &field(&session, "opportunity_id")]);
    let findings = if field(&view, "kind") == "skeptical" { r#"[{"ref":"finding:new","title":"New edge case"}]"# }
        else { r#"[{"ref":"finding:first","title":"First edge case"},{"ref":"finding:second","title":"Second edge case"}]"# };
    fs::write("receipt.json", format!("{{\"schema\":\"review_receipt.v1\",\"session_id\":\"{}\",\"submission_id\":\"{}\",\"candidate_oid\":\"{}\",\"outcome\":\"completed\",\"findings\":{},\"evidence\":[\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"]}}",
        field(&session,"session_id"), field(&session,"submission_id"), field(&session,"candidate_oid"),findings)).unwrap();
    let receipt = review(&["submit","--input-file","receipt.json"]);
    fs::write("submitted.tmp", receipt).unwrap(); fs::rename("submitted.tmp","submitted.json").unwrap();
    loop { std::thread::park() }
}
"##;

#[test]
fn launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield() { review_auto_spool("claude"); }
#[test]
fn launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield() { review_auto_spool("codex"); }

fn review_auto_spool(kind: &'static str) {
    let mut lab = Lab::of_kind(kind, "unknown_usage='allow_with_warning'\n[worker_isolation]\nshare_login=false", |repo| repo.to_owned());
    lab.write_agent(REVIEW_AUTO_PROBE, &[("ROOT", lab.path("root").display().to_string()), ("BIN", BIN.into()), ("VERSION",lab.version().into())]);
    let world = lab.review_world_setup(false);
    fs::write(lab.project.join("PROJECT.md"), format!("+++\n[[repos]]\npath={:?}\n+++\nRetained project review context.\n", lab.repo)).unwrap();
    let prompt = lab.path("instructions.md");
    fs::write(&prompt, "Check the candidate's edge cases.").unwrap();
    lab.serve();
    let socket = lab.socket();
    for (task, method) in [("review-code","code"),("review-skeptic","skeptical")] {
        let args = ["launch", "demo", "run", "--task", task, "--profile", "worker", "--repository", lab.repo.to_str().unwrap(), "--review-of", "authored",
            "--review-kind", method, "--prompt-file", prompt.to_str().unwrap(), "--herdr-socket", socket.to_str().unwrap(), "--sign-with", lab.key.to_str().unwrap()];
        let report = lab.ok(&args);
        let attempt = AttemptId::new(report["attempt"].as_str().unwrap()).unwrap();
        let before = lab.review_show(None);
        assert_eq!(lab.ok(&args)["attempt"], report["attempt"]);
        assert_eq!(lab.review_show(None), before);
        let worktree = lab.planned_worktree(&attempt);
        let mut ticker = lab.spawn();
        lab.wait(&mut ticker, 180, &|| worktree.join("submitted.json").exists());
        lab.stop(ticker);
        let shown = lab.review_show(None);
        let opportunity = shown["opportunities"].as_array().unwrap().iter().find(|o|o["kind"]==method).unwrap();
        assert_eq!(opportunity["submission_id"], world.submission);
        assert_eq!(opportunity["assignment"]["blind"], false);
        assert_eq!(opportunity["role"], "gate");
        assert_eq!(opportunity["sessions"][0]["completion"]["findings_submitted"], if method=="code" {2} else {1});
        // A launch-target worker receives its brief with the launch command, not
        // as `agent.prompt`: read the retained brief of the launched review session.
        let snapshot: String = rusqlite::Connection::open(lab.project.join(".state/state.db")).unwrap()
            .query_row("SELECT snapshot_id FROM review_session_launches WHERE attempt_id=?1", [attempt.as_str()], |r| r.get(0)).unwrap();
        let delivered = migration::open_active(&lab.project).unwrap().memory_snapshot_inputs(&snapshot).unwrap().instructions;
        assert!(delivered.contains("Retained project review context") && delivered.contains("Review instructions") && delivered.contains("Check the candidate's edge cases."), "{delivered}");
        // Stop this worker before launching the next review; the receipt remains
        // a completion even though this probe intentionally omitted its report.
        let running = lab.attempt(&attempt);
        lab.ok(&["task","demo","cancel-attempt",attempt.as_str(),"--expected-revision",&running.revision.to_string(),"--expected-head",&lab.head().to_string(),"--reason","probe done"]);
        let mut ticker = lab.spawn();
        lab.wait(&mut ticker, 120, &|| lab.attempt(&attempt).termination_observed);
        lab.stop(ticker);
        let inbox = lab.ok(&["inbox","list","demo"]).to_string();
        assert!(inbox.contains("review_receipt_without_result"), "{inbox}");
        // The Herdr stand-in serves one workspace; forget the ended worker.
        fs::write(lab.path("lab/reset-workspace"), "").unwrap();
    }
    let report = lab.ok(&["telemetry","demo","review","report"]);
    assert_eq!(report["metrics"]["M20"]["value"], "2/2");
    assert_eq!(report["metrics"]["M28"]["excluded"]["pending_triage"], 1);
    let findings = lab.ok(&["telemetry","demo","review","findings","show"]);
    let claim = findings["findings"]["submissions"].as_array().unwrap().iter().find(|s|s["finding_ref"]=="finding:new").unwrap()["claims"][0]["claim_id"].as_i64().unwrap().to_string();
    lab.ok(&["telemetry","demo","review","findings","validate",&claim,"--new","--severity","high","--evidence",&format!("sha256:{}","a".repeat(64))]);
    let report = lab.ok(&["telemetry","demo","review","report"]);
    assert_eq!(report["metrics"]["M28"]["denominator"], 1);
    assert_eq!(report["metrics"]["M28"]["value"], "1/1");
    review_auto_fix_round(&mut lab, &prompt, &socket);
}

/// One task repairs both findings of a completed review, with the worker's
/// public result submission and native verification/integration driving links.
fn review_auto_fix_round(lab: &mut Lab, prompt: &std::path::Path, socket: &std::path::Path) {
    let template = lab.path("fix-template.json");
    let source = SUBMITTING_EDITING_AGENT.replace("2.1.0 (Claude Code)", lab.version())
        .replace("for index in 0..2", "for index in 0..1")
        .replace("Command::new(\"herdr-farm\").args", "Command::new(\"herdr-farm\").env(\"HERDR_FARM_TEST_TIME_SCALE\", TEST_TIME_SCALE).args");
    lab.write_agent(&source, &[("ROOT",lab.path("root").display().to_string()),("TEMPLATE_PATH",template.display().to_string())]);
    // A new agent binary is a new profile: re-record its evidence as the lab
    // setup does (the Herdr stand-in cannot serve a live probe).
    lab.prepare_profile();
    // A fix that lands is routed verify-then-integrate by its contract.
    lab.git(&["branch","fix-integration"]);
    let args = ["launch","demo","run","--task","fix-review","--profile","worker","--repository",lab.repo.to_str().unwrap(),
        "--fixes-review","review-code","--fixes","finding:first","--write","work.txt","--output","work.txt","--integration-ref","refs/heads/fix-integration",
        "--prompt-file",prompt.to_str().unwrap(),"--herdr-socket",socket.to_str().unwrap(),"--sign-with",lab.key.to_str().unwrap()];
    let launch = lab.ok(&args);
    let attempt = AttemptId::new(launch["attempt"].as_str().unwrap()).unwrap();
    let fixes = || lab.ok(&["telemetry","demo","review","fixes","show"])["fixes"].clone();
    let opened = fixes();
    assert_eq!(opened["repairs"].as_array().unwrap().len(), 2);
    for repair in opened["repairs"].as_array().unwrap() {
        assert_eq!(repair["assignment"], "configuration");
        assert_eq!(repair["attempts"][0]["attempt_id"], attempt.as_str());
        assert_eq!(repair["proposals"], json!([]));
    }
    assert_eq!(lab.ok(&args)["attempt"], launch["attempt"]);
    assert_eq!(fixes(), opened, "launch replay must not mint findings or repairs twice");
    let triage = lab.ok(&["telemetry","demo","review","findings","show"])["findings"].clone();
    for reference in ["finding:first","finding:second"] {
        let submitted = triage["submissions"].as_array().unwrap().iter().find(|s|s["finding_ref"]==reference).unwrap();
        assert_eq!(submitted["claims"][0]["outcome"], "validated");
    }
    let metrics = lab.ok(&["telemetry","demo","review","report"])["metrics"].clone();
    assert_eq!(metrics["M22"]["value"], "3/3");
    assert_eq!(metrics["M25"]["value"], "0/3");
    lab.ok(&["result","demo","auto","--verify","off","--integrate","off","--expected-head",&lab.head().to_string()]);
    let contract = migration::open_active(&lab.project).unwrap().task_contract_document("fix-review").unwrap().unwrap();
    let digest = runtime::task_contract(&lab.project,&TaskId::new("fix-review").unwrap()).unwrap().unwrap().digest;
    fs::write(&template,json!({"idempotency_key":"KEY","task_id":"fix-review","contract_revision":1,"contract_digest":digest,
        "attempt_id":attempt.as_str(),"repository":lab.repo.canonicalize().unwrap(),"base_oid":contract["base_oid"],"candidate_oid":"CANDIDATE","object_format":"sha256",
        "artifact_manifest":[{"path":"work.txt","oid":"BLOB"}],"claimed_checks":[],"objects":"OBJECTS"}).to_string()).unwrap();
    let worktree = lab.planned_worktree(&attempt);
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker,180,&|| worktree.join("submitted-0").exists() && lab.ok(&["result","demo","show"]).as_array().unwrap().iter().any(|s|s["attempt_id"]==attempt.as_str()));
    lab.stop(ticker);
    let shown = lab.ok(&["result","demo","show"]);
    let submitted = shown.as_array().unwrap().iter().find(|s|s["attempt_id"]==attempt.as_str()).unwrap();
    let submission = submitted["submission_id"].as_str().unwrap();
    let candidate = submitted["candidate_oid"].as_str().unwrap();
    let proposed = fixes();
    for repair in proposed["repairs"].as_array().unwrap() {
        assert_eq!(repair["proposals"].as_array().unwrap().len(),1);
        assert_eq!(repair["proposals"][0]["submission_id"],submission);
        assert_eq!(repair["proposals"][0]["candidate_oid"],candidate);
    }
    assert!(lab.ok(&["result","demo","submit","--input-file",worktree.join("submission.json").to_str().unwrap()])["replayed"].as_bool().unwrap());
    assert_eq!(fixes(),proposed);
    let policy = lab.path("fix-policy.json");
    fs::write(&policy,contract["acceptance_policies"][0]["text"].as_str().unwrap()).unwrap();
    let verify_dir = lab.path("fix-verify");
    let verify = ["result","demo","verify",submission,"--policy-id","output-1","--policy-file",policy.to_str().unwrap(),"--idempotency-key","fix-verify","--work-dir",verify_dir.to_str().unwrap()];
    let verified = lab.ok(&verify);
    assert_eq!(verified["state"],"accepted","{verified}");
    let fixed = fixes();
    for repair in fixed["repairs"].as_array().unwrap() {
        assert_eq!(repair["closure"], "fixed", "{repair}");
        assert_eq!(repair["proposals"][0]["verification"]["run_id"],verified["run_id"]);
    }
    lab.ok(&verify);
    assert_eq!(fixes(),fixed);
    assert_eq!(lab.ok(&["telemetry","demo","review","report"])["metrics"]["M25"]["value"],"2/3");
    let integrate_dir = lab.path("fix-integrate");
    let integrate = ["result","demo","integrate",verified["receipt"]["result_id"].as_str().unwrap(),"--repository",lab.repo.to_str().unwrap(),"--idempotency-key","fix-integrate","--work-dir",integrate_dir.to_str().unwrap()];
    let integrated = lab.ok(&integrate);
    assert_eq!(integrated["state"],"integrated","{integrated}");
    let landed = fixes();
    for repair in landed["repairs"].as_array().unwrap() { assert!(repair["proposals"][0]["integration"].is_object()); }
    lab.ok(&integrate);
    assert_eq!(fixes(),landed);
    // The coordinator uses the ordinary owner-session CLI verbs. Both decisions
    // persist in history even when the duplicate decision supersedes rejection.
    let claim = triage["submissions"].as_array().unwrap().iter().find(|s|s["finding_ref"]=="finding:new").unwrap()["claims"][0]["claim_id"].as_i64().unwrap().to_string();
    let rejected = lab.ok(&["telemetry","demo","review","findings","reject",&claim,"--reason","intended_behavior"]);
    assert_eq!(rejected["event"]["principal"],"operator:cli");
    let canonical = landed["repairs"][0]["finding_id"].as_str().unwrap();
    let duplicate = lab.ok(&["telemetry","demo","review","findings","duplicate",&claim,"--of",canonical]);
    assert_eq!(duplicate["event"]["authority"],"operator_owner.v1");
    let before = lab.state();
    let history = fixes();
    for (flag,reference) in [("--fixes","finding:new"),("--fixes-review","authored")] {
        let refused = lab.cli(&["launch","demo","run","--task","refused-fix","--profile","worker","--repository",lab.repo.to_str().unwrap(),flag,reference,"--write","work.txt","--output","work.txt"]);
        assert!(!refused.status.success());
        assert_eq!(lab.state(),before,"a refused selector writes nothing");
        assert_eq!(fixes(),history);
    }
    // The editing agent stays alive after submitting: stop it, as the review
    // rounds do, then reuse the lab for a re-review.
    let running = lab.attempt(&attempt);
    lab.ok(&["task","demo","cancel-attempt",attempt.as_str(),"--expected-revision",&running.revision.to_string(),"--expected-head",&lab.head().to_string(),"--reason","fix round done"]);
    lab.run_until(120,&|| lab.attempt(&attempt).termination_observed);
    fs::write(lab.path("lab/reset-workspace"),"").unwrap();
    // A distinct build (new digest) is a new profile, re-recorded as at setup.
    let rebuilt = format!("{REVIEW_AUTO_PROBE}\n#[used] static REBUILD: [u8; 9] = *b\"re-review\";\n");
    lab.write_agent(&rebuilt,&[("ROOT",lab.path("root").display().to_string()),("BIN",BIN.into()),("VERSION",lab.version().into())]);
    lab.prepare_profile();
    let rereview = lab.ok(&["launch","demo","run","--task","review-fix","--profile","worker","--repository",lab.repo.to_str().unwrap(),"--review-of","fix-review",
        "--prompt-file",prompt.to_str().unwrap(),"--herdr-socket",socket.to_str().unwrap(),"--sign-with",lab.key.to_str().unwrap()]);
    let rereview_attempt = AttemptId::new(rereview["attempt"].as_str().unwrap()).unwrap();
    let session = lab.ok(&["telemetry","demo","review","session","--attempt",rereview_attempt.as_str()])["session"].clone();
    let reviewed = lab.review_show(None);
    let opportunity = reviewed["opportunities"].as_array().unwrap().iter().find(|o|o["opportunity_id"]==session["opportunity_id"]).unwrap();
    assert_eq!(opportunity["submission_id"],submission);
    let mut expected: Vec<_> = landed["repairs"].as_array().unwrap().iter().map(|r|r["finding_id"].clone()).collect();
    expected.sort_by(|a,b|a.as_str().cmp(&b.as_str()));
    assert_eq!(opportunity["prior_findings"],json!(expected));
    let reserved = lab.attempt(&rereview_attempt);
    lab.ok(&["task","demo","cancel-attempt",rereview_attempt.as_str(),"--expected-revision",&reserved.revision.to_string(),"--expected-head",&lab.head().to_string(),"--reason","re-review binding checked"]);
    lab.run_until(120,&|| lab.attempt(&rereview_attempt).termination_observed);
}

/// Operator recovery uses the retained attempt report without starting an agent.
/// A local Herdr socket fixture supplies preparation inventory; worktree
/// preparation and submission use public entry points.
#[test]
fn submit_captured_retains_remember_from_the_attempt_report_and_replays_once() {
    let mut lab=Lab::new("unknown_usage='allow_with_warning'\n");
    lab.install_work_contract("verify_only");
    let (_,attempt)=lab.reserve("Retained instructions");
    lab.serve();
    lab.ok(&["new", "--legacy", "history"]);
    lab.ok(&["pause", "history"]);
    let history = lab.path("root/history");
    migration::apply(&history, &migration::inspect(&history).unwrap(), true).unwrap();
    inventory_history::seed(&history, 1040);
    let state=lab.state();
    let record=state.attempt_inputs.iter().find(|r|r.attempt==attempt).unwrap();
    let receipts=herdr_farm::worktree_preparation::prepare(&lab.project,&record.operation,1,Instant::now()+Duration::from_secs(45),Default::default()).unwrap();
    fs::write(PathBuf::from(&receipts[0].plan.path).join("work.txt"),"recovered change\n").unwrap();
    let brief=lab.ok(&["memory","demo","attempt-brief","--attempt",attempt.as_str()]);
    assert!(brief["text"].as_str().unwrap().contains("## Remember"));
    let output=PathBuf::from(brief["output_directory"].as_str().unwrap());
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("report.md"),"## Results\nRecovered edit\n\n## Remember\nRetries retain the captured base closure.\n").unwrap();
    let first=lab.ok(&["result","demo","submit-captured",attempt.as_str()]);
    let again=lab.ok(&["result","demo","submit-captured",attempt.as_str()]);
    assert_eq!(again["submission"]["replayed"],true);
    assert_eq!(first["submission"]["submission_id"],again["submission"]["submission_id"]);
    let candidates=lab.ok(&["memory","demo","list"]);
    assert_eq!(candidates.as_array().unwrap().len(),1);
    assert_eq!(candidates[0]["remember"],"Retries retain the captured base closure.");
    assert_eq!(candidates[0]["attempt_id"],attempt.as_str());
    assert_eq!(candidates[0]["submission_id"],first["submission"]["submission_id"]);
    assert_eq!(lab.state().inbox.iter().filter(|i|i.content.kind=="memory.candidate_proposed").count(),1);
    // Empty Remember evidence does not produce a second candidate.
    fs::write(output.join("report.md"),"## Results\nSame recovered edit\n\n## Remember\n\n").unwrap();
    lab.ok(&["result","demo","submit-captured",attempt.as_str()]);
    assert_eq!(lab.ok(&["memory","demo","list"]).as_array().unwrap().len(),1);
}

/// A failed mid-run observation remains a gap even when both lifecycle hooks
/// succeed. All operations use the CLI and an isolated local Herdr fixture.
#[test]
fn attention_mid_run_failure_remains_incomplete_at_termination() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Attention observation lab");
    lab.ok(&["telemetry", "demo", "collect"]);
    lab.serve();
    let mut ticker = lab.spawn_attention_interval("3600");
    lab.wait_for(&mut ticker, "running observation", &attempt, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.stop(ticker);
    fs::write(lab.path("lab/agent-status"), "unknown").unwrap();
    let out = Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim())
        .env("HOME", lab.home.path()).env("PATH", "/usr/bin:/bin").env("HERDR_BIN_PATH", &lab.herdr).env("HERDR_FARM_TELEMETRY_COLLECT_SECS", "3600")
        .args(["--root", lab.path("root").to_str().unwrap(), "telemetry", "demo", "accounting", "observe-attention"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let observed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(observed["gaps"]["state_unknown"], 1);
    fs::remove_file(lab.path("lab/agent-status")).unwrap();
    let running = lab.attempt(&attempt);
    lab.ok(&["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &running.revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "finished"]);
    let mut ticker = lab.spawn_attention_interval("3600");
    lab.wait_for(&mut ticker, "terminal observation", &attempt, 120, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let m31 = lab.ok(&["telemetry", "demo", "report", "--json"])["metrics"]["M31"].clone();
    assert!(m31["value"].is_null());
    assert_eq!(m31["reason"], "empty_denominator");
    assert_eq!(m31["coverage"]["with_gaps"], 1);
    assert_eq!(m31["coverage"]["complete"], 0);
    assert_eq!(m31["coverage"]["not_observed"], 0);
}

#[test]
fn idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    lab.run_until(100, &|| lab.attempt(&attempt).state == AttemptState::Running);
    fs::write(lab.path("lab/agent-status"), "done").unwrap();
    let notices = || lab.state().inbox.into_iter().filter(|i| i.content.kind == "attempt.worker_idle").collect::<Vec<_>>();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| notices().len() == 1);
    lab.ok(&["inbox", "done", "demo", &notices()[0].content.id]);
    // Observe another 30 completed passes (15 scaled seconds) without rearming.
    let metrics = lab.path("root/.ticker-metrics.json");
    let mut last = fs::metadata(&metrics).unwrap().ino();
    for _ in 0..30 {
        lab.wait(&mut ticker, 60, &|| fs::metadata(&metrics).unwrap().ino() != last);
        last = fs::metadata(&metrics).unwrap().ino();
        assert_eq!(notices().len(), 1);
    }
    lab.stop(ticker);
    lab.run_quiet(3);
    let first = notices();
    assert_eq!(first.len(), 1);
    assert!(first[0].content.summary.contains(attempt.as_str()));
    assert!(!lab.attempt(&attempt).termination_observed);
    assert_eq!(lab.attempt(&attempt).state, AttemptState::Running);
    // A restart begins a new stretch even though the worker stayed idle.
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| notices().len() == 2);
    fs::write(lab.path("lab/agent-status"), "working").unwrap();
    let observed = lab.count("agent.list");
    lab.wait(&mut ticker, 60, &|| lab.count("agent.list") >= observed + 3);
    fs::write(lab.path("lab/agent-status"), "idle").unwrap();
    lab.wait(&mut ticker, 60, &|| notices().len() == 3);
    lab.stop(ticker);
    lab.run_quiet(2);
    assert_eq!(notices().len(), 3);
}

/// The real timeout ends the stand-in agent while the root barrier is held.
/// Failed observations remain retryable, including across ticker restart.
#[test]
fn wall_budget_termination_retries_contention_and_notifies_once() {
    wall_budget_termination(true, false);
}

#[test]
fn wall_budget_termination_without_contention() {
    wall_budget_termination(false, false);
}

#[test]
fn wall_budget_termination_with_changed_frozen_definition_keeps_process_exit() {
    wall_budget_termination(false, true);
}

fn wall_budget_termination(contended: bool, change_definition: bool) {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let config = lab.path(".config/herdr-farm/config.toml");
    fs::write(&config, fs::read_to_string(&config).unwrap().replace("max_wall_seconds=600", "max_wall_seconds=15")).unwrap();
    lab.resume();
    let (_, attempt) = lab.reserve("Stay alive until the wall budget ends");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).state == AttemptState::Running);
    if change_definition {
        fs::write(&config, fs::read_to_string(&config).unwrap().replace("max_wall_seconds=15", "max_wall_seconds=600")).unwrap();
    }
    if contended {
        let deadline = Instant::now() + Duration::from_secs(10);
        let guard = loop {
            match herdr_farm::execution_guard::ProjectGuard::acquire(&lab.project) {
                Ok(guard) => break guard,
                Err(error) => {
                    assert!(Instant::now() < deadline, "{error:#}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        let started: LaunchStartedReceipt = serde_json::from_value(lab.events("runtime.launch_started")[0].payload.clone()).unwrap();
        lab.wait(&mut ticker, 60, &|| {
            let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
            log.lines().filter(|line| line.contains("termination recording deferred by contention") && line.contains(attempt.as_str())).count() >= 2
                && herdr_farm::worker_supervision::SupervisorObservation::recover_exited(started.supervisor.as_ref().unwrap()).unwrap()
        });
        assert!(!lab.attempt(&attempt).termination_observed);
        lab.stop(ticker);
        drop(guard);
        ticker = lab.spawn();
    }
    lab.wait(&mut ticker, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let ended = lab.attempt(&attempt);
    assert_eq!(ended.state, AttemptState::Failed);
    assert!(!ended.retains_capacity());
    let events = lab.events("runtime.worker_terminated");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["cause"], if change_definition { "process_exit" } else { "timed_out" });
    let notices = lab.ok(&["inbox", "list", "demo"]);
    assert_eq!(notices.as_array().unwrap().iter().filter(|item| item["content"]["kind"] == "attempt.ended_without_submission").count(), 1, "{notices}");
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap();
    assert!(log.lines().any(|line| line.contains("termination recording succeeded") && line.contains(attempt.as_str())), "{log}");
}

// ---- Termination beside a long isolated check (TERM-RECORD-1, SILENT-DEATH-1) ----

/// A git-protocol fixture on localhost whose acceptance policy's one check,
/// `git ls-remote`, is held open until `release`, as a long test run would be;
/// `started` is the number of checks that have connected. The sandbox root
/// holds only `git`, so a blocking check must be a git command.
struct HeldCheck { policy: String, started: std::sync::Arc<std::sync::atomic::AtomicUsize>, release: std::sync::Arc<std::sync::atomic::AtomicBool> }
impl HeldCheck {
    fn new() -> Self {
        use std::{io::{Read, Write}, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (started, release) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        {
            let (started, release) = (started.clone(), release.clone());
            std::thread::spawn(move || for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                started.fetch_add(1, Ordering::SeqCst);
                let release = release.clone();
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer);
                    while !release.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
                    let _ = stream.write_all(b"0000");
                    let _ = stream.read(&mut buffer);
                });
            });
        }
        let policy = format!(r#"{{"version":1,"checks":["/usr/bin/git","ls-remote","git://127.0.0.1:{port}/fixture"]}}"#);
        HeldCheck { policy, started, release }
    }
    fn started(&self) -> bool { self.started.load(std::sync::atomic::Ordering::SeqCst) >= 1 }
    fn released(&self) -> bool { self.release.load(std::sync::atomic::Ordering::SeqCst) }
    fn release(&self) { self.release.store(true, std::sync::atomic::Ordering::SeqCst); }
}

/// A child CLI command killed when dropped (a failed wait must not leave it behind).
struct Background(Option<Child>);
impl Background {
    fn running(&mut self) -> bool { matches!(self.0.as_mut().unwrap().try_wait(), Ok(None)) }
    /// Its output once it ends on its own.
    fn finish(mut self) -> Output { self.0.take().unwrap().wait_with_output().unwrap() }
}
impl Drop for Background { fn drop(&mut self) { if let Some(child) = &mut self.0 { let _ = child.kill(); let _ = child.wait(); } } }

/// Like `SUBMITTING_EDITING_AGENT`, but one submission, then the process
/// exits (status 0) once the owner marker `die` appears in the worktree.
const SUBMIT_THEN_EXIT_AGENT: &str = r#"
use std::{fs, path::Path, process::Command, time::Duration};
fn git(args: &[&str]) -> String {
    let out = Command::new("/usr/bin/git").args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    let output = std::path::PathBuf::from(std::env::var("HERDR_FARM_WORKER_OUTPUT").unwrap());
    fs::write(output.join("report.md"), "Submitted, then exited\n").unwrap();
    fs::write("work.txt", "worker change\n").unwrap();
    git(&["add", "work.txt"]); git(&["commit", "-qm", "worker change"]);
    let candidate = git(&["rev-parse", "HEAD"]);
    let blob = git(&["rev-parse", "HEAD:work.txt"]);
    let objects = git(&["rev-list", "--objects", "HEAD"]).lines().map(|line| {
        let oid = line.split_whitespace().next().unwrap();
        format!("{{\"oid\":\"{oid}\",\"relative_path\":\"{}/{}\"}}", &oid[..2], &oid[2..])
    }).collect::<Vec<_>>().join(",");
    let template = fs::read_to_string(TEMPLATE_PATH).unwrap();
    let document = template.replace("CANDIDATE", &candidate).replace("BLOB", &blob)
        .replace("\"OBJECTS\"", &format!("[{objects}]")).replace("KEY", "exit-0");
    fs::write("submission.json", document).unwrap();
    let mut submitted = false;
    for _ in 0..200 {
        let out = Command::new("herdr-farm").args(["--root", ROOT, "result", "demo", "submit", "--input-file", "submission.json"]).output().unwrap();
        if out.status.success() { submitted = true; break }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(submitted);
    fs::write("submitted-0", "ok").unwrap();
    while !Path::new("die").exists() { std::thread::sleep(Duration::from_millis(50)); }
    std::process::exit(0)
}
"#;

/// A worker that edits its deliverable without committing or submitting and
/// dies (status 1) once the owner marker `die` appears in the worktree.
const EDIT_THEN_DIE_AGENT: &str = r#"
use std::{fs, path::Path, time::Duration};
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") { println!("2.1.0 (Claude Code)"); return }
    fs::write("work.txt", "worker change\n").unwrap();
    while !Path::new("die").exists() { std::thread::sleep(Duration::from_millis(50)); }
    std::process::exit(1)
}
"#;

impl Lab {
    /// `cli` started in the background with captured output.
    fn spawn_cli(&self, args: &[&str]) -> Background {
        Background(Some(Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim()).env("HOME", self.home.path()).env("PATH", "/usr/bin:/bin")
            .args(["--root", self.path("root").to_str().unwrap()]).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()))
    }
    /// Recorded verification runs, read without taking any lock.
    fn verification_runs(&self) -> u64 {
        let db = rusqlite::Connection::open_with_flags(self.project.join(".state/state.db"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.query_row("SELECT count(*) FROM verification_runs", [], |row| row.get(0)).unwrap()
    }
    /// Another attempt's submission to verify: the ended attempt `AUTHOR_ATTEMPT`
    /// of task `authored`, whose owner-signed contract carries `policy` as
    /// its one acceptance policy `clean`. Returns the submission id.
    fn plant_foreign_submission(&self, policy: &str) -> String {
        let head = self.head().to_string();
        self.ok(&["task", "demo", "add", "authored", "--title", AUTHOR_TITLE, "--expected-head", &head]);
        let base = self.git(&["rev-parse", "HEAD"]);
        self.git(&["checkout", "-qb", "author"]);
        fs::write(self.repo.join("lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
        self.git(&["add", "."]);
        self.git(&["commit", "-qm", "candidate"]);
        let candidate = self.git(&["rev-parse", "HEAD"]);
        self.git(&["checkout", "-q", "-"]);
        let repository = self.repo.canonicalize().unwrap().display().to_string();
        let store = self.project.join(".state/state.db").canonicalize().unwrap().display().to_string();
        let mut document = serde_json::to_vec_pretty(&json!({
            "version": 3, "outputs": [{"path": "lib.rs", "kind": "git_file"}], "scope": {"paths": [{"path": "lib.rs", "access": "write"}]},
            "project_store": store, "expected_head": self.head(), "task_id": "authored", "contract_revision": 1, "deliverable": "answer", "non_goals": "none",
            "acceptance_policies": [{"id": "clean", "text": policy}], "repository": repository, "base_oid": base,
            "object_format": "sha256", "dependencies": [], "capability_flags": [], "profile_kind": "codex", "retry_class": "none", "result_schema_id": "result-v1",
            "route": "verify_only", "authority": authority::policy_reference(&self.project).unwrap()})).unwrap();
        document.push(b'\n');
        let contract = self.path("authored-contract.json");
        fs::write(&contract, &document).unwrap();
        assert!(Command::new("/usr/bin/ssh-keygen").args(["-Y", "sign", "-f"]).arg(&self.key).args(["-n", authority::CONTRACT_SIGNATURE_NAMESPACE]).arg(&contract).output().unwrap().status.success());
        let installed = self.ok(&["task", "demo", "contract", "put", "--input-file", contract.to_str().unwrap(), "--signature", self.path("authored-contract.json.sig").to_str().unwrap()]);
        let db = rusqlite::Connection::open(self.project.join(".state/state.db")).unwrap();
        db.execute("INSERT INTO attempts(id,task_id,revision,state,snapshot,reservation,termination_observed) VALUES(?1,'authored',1,'completed',NULL,?1,1)", [AUTHOR_ATTEMPT]).unwrap();
        drop(db);
        let objects: Vec<Value> = self.git(&["rev-list", "--objects", "--all"]).lines()
            .map(|line| { let oid = line.split_whitespace().next().unwrap(); json!({"oid": oid, "relative_path": format!("{}/{}", &oid[..2], &oid[2..])}) }).collect();
        let result = self.path("authored-result.json");
        fs::write(&result, json!({"idempotency_key": "authored-key", "task_id": "authored", "contract_revision": 1, "contract_digest": installed["digest"],
            "attempt_id": AUTHOR_ATTEMPT, "repository": repository, "base_oid": base, "candidate_oid": candidate, "object_format": "sha256",
            "artifact_manifest": [{"path": "lib.rs", "oid": candidate}], "claimed_checks": [], "objects": objects}).to_string()).unwrap();
        self.ok(&["result", "demo", "submit", "--input-file", result.to_str().unwrap()])["submission_id"].as_str().unwrap().to_owned()
    }
    /// The ticker log lines about `attempt` containing `text`.
    fn log_lines(&self, attempt: &AttemptId, text: &str) -> usize {
        fs::read_to_string(self.path("root/.ticker.log")).unwrap_or_default().lines().filter(|line| line.contains(attempt.as_str()) && line.contains(text)).count()
    }
}

/// A worker's process ends while an isolated check of another attempt's
/// submission runs for minutes, as a Godot test run does. The check holds
/// only the check lock, so the ticker takes the exclusive root beside it and
/// records the termination within its usual retry cadence, with no cancel,
/// no `launch stop --force` and no wait for the check to end.
/// With `submits`, the worker submitted first and then exited (TERM-RECORD-1);
/// otherwise it edited without committing and died (SILENT-DEATH-1): the
/// `attempt.ended_without_submission` notice lands and the uncommitted edit
/// is preserved under `.state/worktree-file-snapshots`.
fn termination_beside_a_foreign_check(submits: bool) {
    let held = HeldCheck::new();
    let policy = held.policy.clone();
    let (mut lab, attempt, worktree) = if submits { editing_submission_lab_with("verify_only", WORK_POLICY, false, false, SUBMIT_THEN_EXIT_AGENT) } else {
        let mut lab = Lab::new("unknown_usage='allow_with_warning'\n");
        lab.write_agent(EDIT_THEN_DIE_AGENT, &[]);
        let (_, attempt) = lab.reserve("Retained instructions");
        let worktree = lab.planned_worktree(&attempt);
        (lab, attempt, worktree)
    };
    let foreign = lab.plant_foreign_submission(&policy);
    let policy_file = lab.path("blocking-policy.json");
    fs::write(&policy_file, &policy).unwrap();
    lab.serve();
    let mut ticker = lab.spawn();
    let edited = || fs::read_to_string(worktree.join("work.txt")).is_ok_and(|t| t == "worker change\n");
    if submits {
        lab.wait_for(&mut ticker, "the worker to run and submit", &attempt, 120, &|| worktree.join("submitted-0").exists() && lab.attempt(&attempt).state == AttemptState::Running);
        lab.wait_for(&mut ticker, "the submission notice", &attempt, 60, &|| lab.state().inbox.iter().any(|i| i.content.kind == "result.submitted"));
    } else {
        lab.wait_for(&mut ticker, "the worker to run and edit work.txt", &attempt, 120, &|| edited() && lab.attempt(&attempt).state == AttemptState::Running);
    }
    let live = lab.attempt(&attempt);
    assert!(live.state == AttemptState::Running && !live.termination_observed, "{live:?}\n{}", fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default());
    // The operator check of the other attempt's submission starts and stays running.
    let mut check = lab.spawn_cli(&["result", "demo", "verify", &foreign, "--policy-id", "clean", "--policy-file", policy_file.to_str().unwrap(),
        "--idempotency-key", "operator-foreign", "--work-dir", lab.path("verify-work").to_str().unwrap(), "--timeout-seconds", "300"]);
    lab.wait_for(&mut ticker, "the foreign check to start", &attempt, 60, &|| held.started());
    assert_eq!(lab.verification_runs(), 0);
    // The worker's process ends under the running check.
    fs::write(worktree.join("die"), b"").unwrap();
    lab.wait_for(&mut ticker, "termination recorded beside the running check", &attempt, 60, &|| lab.attempt(&attempt).termination_observed);
    assert!(!held.released() && lab.verification_runs() == 0, "the check had already ended");
    let ended = lab.attempt(&attempt);
    assert!(!ended.retains_capacity(), "{ended:?}");
    let events = lab.events("runtime.worker_terminated");
    assert_eq!((events.len(), &events[0].payload["cause"]), (1, &json!("process_exit")));
    assert!(lab.log_lines(&attempt, "termination recording succeeded") >= 1);
    assert_ne!(ended.state, AttemptState::Cancelled);
    let notices = lab.ok(&["inbox", "list", "demo"]);
    let ended_without_submission = notices.as_array().unwrap().iter().filter(|item| item["content"]["kind"] == "attempt.ended_without_submission").count();
    if submits {
        assert_eq!(ended_without_submission, 0, "{notices}");
        assert_eq!(ended.state, AttemptState::Failed, "a process exit before a verdict leaves the attempt failed, the submission intact");
    } else {
        assert_eq!(ended_without_submission, 1, "{notices}");
        // The uncommitted edit is preserved with the termination.
        let snapshots = lab.project.join(".state/worktree-file-snapshots").join(attempt.as_str());
        let digests: Vec<PathBuf> = fs::read_dir(&snapshots).unwrap().map(|e| e.unwrap().path()).collect();
        assert!(!digests.is_empty(), "no checkout snapshot under {}", snapshots.display());
        let preserved = digests.iter().any(|dir| fs::read_to_string(dir.join("manifest.json")).is_ok_and(|m| m.contains("work.txt"))
            && fs::read_dir(dir).unwrap().any(|e| fs::read(e.unwrap().path()).is_ok_and(|b| b == b"worker change\n")));
        assert!(preserved, "the uncommitted work.txt is not in any snapshot of {}", snapshots.display());
        assert!(!events[0].payload["repository_snapshots"].as_array().unwrap().is_empty(), "{}", events[0].payload);
    }
    // Only then does the check end, recording its verdict on an unchanged target.
    assert!(check.running(), "the foreign check ended early");
    held.release();
    let out = check.finish();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let verdict: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(verdict["state"], "accepted", "{verdict}");
    lab.stop(ticker);
}

#[test]
fn a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs() {
    termination_beside_a_foreign_check(true);
}

#[test]
fn a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs() {
    termination_beside_a_foreign_check(false);
}

/// A worker exits right after submitting, while the automatic verification
/// of its own submission is still running. Recording the termination at once
/// would move the task revision the check is fenced on and the check would
/// end without a verdict, so the attempt stays open — without contending
/// for the root — until the verdict lands; then it ends on its own.
#[test]
fn a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention() {
    let held = HeldCheck::new();
    let (mut lab, attempt, worktree) = editing_submission_lab_with("verify_only", &held.policy, true, false, SUBMIT_THEN_EXIT_AGENT);
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait_for(&mut ticker, "the submission's own check to start", &attempt, 120, &|| held.started());
    fs::write(worktree.join("die"), b"").unwrap();
    // Several termination observations pass while the check runs: none records, none contends.
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(4) {
        assert!(ticker.0.try_wait().unwrap().is_none(), "ticker exited");
        assert!(!lab.attempt(&attempt).termination_observed, "terminated under its own running check");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(lab.log_lines(&attempt, "termination recording deferred by contention"), 0, "{}", fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default());
    held.release();
    lab.wait_for(&mut ticker, "the verdict", &attempt, 120, &|| lab.state().inbox.iter().any(|i| i.content.kind == "verification.accepted"));
    lab.wait_for(&mut ticker, "termination after the verdict", &attempt, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.stop(ticker);
    let kinds: Vec<String> = lab.state().inbox.iter().map(|i| i.content.kind.clone()).collect();
    assert!(!kinds.iter().any(|k| k == "verification.errored"), "{kinds:?}");
    assert_eq!(lab.verification_runs(), 1);
    // The verdict may have requested completion before the exit was recorded.
    let events = lab.events("runtime.worker_terminated");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].payload["cause"].as_str(), Some("process_exit" | "completion")), "{}", events[0].payload);
    assert_eq!(lab.log_lines(&attempt, "termination recording deferred by contention"), 0);
    assert!(lab.log_lines(&attempt, "termination recording succeeded") >= 1);
}

// ---- Launch stall and automatic pause recovery (LAUNCH-STALL-1, PAUSE-RECOVERY-1) ----

/// Every launch stage retakes the exclusive root with a two-second wait. An
/// operator command that holds the root for most of a minute (a `launch run`
/// preparing the next task refreshes profile evidence under it) blocks the
/// stages after workspace creation; the historical 30 s claim lease then
/// expired before a naming intent was recorded and recovery closed the launch
/// as `start_unnamed`, leaving an unnamed worker holding capacity. With the
/// production lease the stages resume once the holder lets go and the worker
/// reaches Running.
#[test]
fn a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn_with_env("300", &[("HERDR_FARM_LAUNCH_LEASE_SECS", "180")]);
    lab.wait_for(&mut ticker, "workspace creation", &attempt, 120, &|| !lab.events("runtime.launch_creation").is_empty());
    // Queued in the kernel: granted the moment the creating stage lets go.
    let file = fs::OpenOptions::new().read(true).write(true).open(lab.path("root/.execution.lock")).unwrap();
    file.lock().unwrap();
    let held = Instant::now();
    while held.elapsed() < Duration::from_secs(45) {
        assert!(ticker.0.try_wait().unwrap().is_none(), "ticker exited");
        assert_ne!(lab.attempt(&attempt).state, AttemptState::Running, "ran while the root was held exclusively");
        std::thread::sleep(Duration::from_millis(500));
    }
    file.unlock().unwrap();
    lab.wait_for(&mut ticker, "Running once the holder released the root", &attempt, 150, &|| lab.attempt(&attempt).state == AttemptState::Running);
    lab.stop(ticker);
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
    assert!(!log.contains("start_unnamed"), "{log}");
    assert!(log.lines().any(|l| l.contains("canonical-launch:") && l.contains(".execution.lock; retry")), "no launch stage waited for the root: {log}");
    let launch = lab.state().deliveries.into_iter().find(|d| d.operation.as_str().starts_with("launch-")).unwrap();
    assert_eq!((launch.state, launch.attempts), (DeliveryState::Confirmed, 1));
    assert_eq!((lab.events("runtime.launch_started").len(), lab.count("workspace.create_command")), (1, 1));
}

/// A worker's pane and agent vanish while its process lives: the observation
/// pass pauses the project for reconciliation. `context` names the automatic
/// pause and its blockers. The owner cancels the attempt; the ticker stops the
/// worker and records its end, nothing blocks admission any more, and the
/// ticker re-activates the project by itself, logging one line.
#[test]
fn an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear() {
    let mut lab = Lab::new("unknown_usage='allow_with_warning'");
    let (_, attempt) = lab.reserve("Retained instructions");
    lab.serve();
    let mut ticker = lab.spawn();
    lab.wait_for(&mut ticker, "the worker to run", &attempt, 120, &|| lab.attempt(&attempt).state == AttemptState::Running);
    fs::write(lab.path("lab/vanish"), b"").unwrap();
    let paused = || lab.state().control.is_some_and(|c| c.state == ProjectState::Paused && c.reconciliation_required);
    lab.wait_for(&mut ticker, "the automatic pause", &attempt, 60, &|| paused());
    assert!(lab.events("project.reconciliation_invalidated").len() >= 1);
    let instructions = fs::read_to_string(lab.project.join("PROJECT.md")).unwrap();
    fs::write(lab.project.join("PROJECT.md"), format!("+++\nname = \"demo\"\n+++\n{instructions}")).unwrap();
    let context = lab.cli(&["context", "demo"]);
    assert!(context.status.success(), "{}", String::from_utf8_lossy(&context.stderr));
    let text = String::from_utf8(context.stdout).unwrap();
    let control = text.lines().find(|l| l.contains("Paused")).unwrap_or("").to_owned();
    assert!(control.contains("automatic pause") && control.contains("blockers:") && control.contains("attempt termination remains unobserved"), "{text}");
    // Live worker, vanished pane: the pause holds across passes.
    let since = Instant::now();
    while since.elapsed() < Duration::from_secs(3) {
        assert!(paused(), "re-activated while the attempt still retained capacity");
        std::thread::sleep(Duration::from_millis(200));
    }
    let running = lab.attempt(&attempt);
    lab.ok_live(&|| ["task", "demo", "cancel-attempt", attempt.as_str(), "--expected-revision", &lab.attempt(&attempt).revision.to_string(), "--expected-head", &lab.head().to_string(), "--reason", "operator stop"].map(String::from).to_vec());
    assert_eq!(running.state, AttemptState::Running);
    lab.wait_for(&mut ticker, "the termination record", &attempt, 60, &|| lab.attempt(&attempt).termination_observed);
    lab.wait_for(&mut ticker, "automatic re-activation", &attempt, 60, &|| lab.state().control.is_some_and(|c| c.state == ProjectState::Active && !c.reconciliation_required));
    lab.stop(ticker);
    let log = fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default();
    assert_eq!(log.lines().filter(|l| l.contains("control re-activated automatically")).count(), 1, "{log}");
    let last = lab.state().events.into_iter().filter(|e| e.kind.starts_with("project.")).last().unwrap();
    assert_eq!((last.kind.as_str(), last.payload["state"].as_str()), ("project.control_changed", Some("active")));
    let context = lab.ok(&["context", "demo"]);
    let _ = context;
}

/// An owner pause is never lifted by the ticker, even with nothing blocking.
#[test]
fn an_owner_pause_is_not_lifted_by_the_ticker() {
    let lab = Lab::new("unknown_usage='allow_with_warning'");
    let control = lab.state().control.unwrap();
    lab.ok(&["runtime", "demo", "state", "paused", "--expected-revision", &control.revision.to_string(), "--expected-head", &lab.head().to_string()]);
    lab.run_quiet(4);
    let control = lab.state().control.unwrap();
    assert_eq!((control.state, control.reconciliation_required), (ProjectState::Paused, true));
    assert!(!fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default().contains("re-activated automatically"));
    let instructions = fs::read_to_string(lab.project.join("PROJECT.md")).unwrap();
    fs::write(lab.project.join("PROJECT.md"), format!("+++\nname = \"demo\"\n+++\n{instructions}")).unwrap();
    let text = String::from_utf8(lab.cli(&["context", "demo"]).stdout).unwrap();
    assert!(text.lines().any(|l| l.contains("Paused") && l.contains("paused by the owner") && l.contains("runtime demo state active")), "{text}");
}

/// A queued task's dedicated server exists before its attempt does; the sweep
/// leaves it alone. The finished-task case is `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`.
#[test]
fn the_server_sweep_spares_a_queued_task_without_an_attempt() {
    let lab = Lab::new("unknown_usage='allow_with_warning'");
    let dir = lab.path("root/.herdr-run/demo-work/herdr");
    fs::create_dir_all(&dir).unwrap();
    let record = json!({"project":"demo","task":"work","pid":1,"socket":lab.path("lab/never.sock"),"managed_by":"herdr-farm"}).to_string();
    fs::write(dir.join("server.json"), &record).unwrap();
    assert_eq!(lab.state().tasks.iter().find(|t| t.id.as_str() == "work").unwrap().state, TaskState::Queued);
    lab.run_quiet(4);
    assert!(dir.join("server.json").is_file(), "the queued task's server record was retired: {}", fs::read_to_string(lab.path("root/.ticker.log")).unwrap_or_default());
}

// Real worker and acceptance commands, with the policy changed only after the
// worker has started. Both directions must retain the original mapping.
#[test]
fn worker_uid_default_root_and_verification_stay_frozen() {
    worker_uid_workflow(false);
}

#[test]
fn worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced() {
    worker_uid_workflow(true);
}

fn worker_uid_workflow(owner: bool) {
    // Keep the fixture socket below AF_UNIX's path limit even when cargo's
    // mandated TMPDIR is inside a long worktree path.
    let mut lab = Lab::of_kind_in("claude", "unknown_usage='allow_with_warning'", |repo| repo.to_owned(), Some("/tmp"));
    let host = Command::new("/usr/bin/id").arg("-u").output().unwrap();
    let uid = String::from_utf8(host.stdout).unwrap().trim().parse::<u32>().unwrap();
    assert_ne!(uid, 0, "this lab requires a regular owner uid");
    let expected = if owner { uid } else { 0 };
    let hidden = lab.path("secret-dir/secret");
    plant(&hidden, "HIDDEN-SENTINEL");
    let read_only = lab.path("owner-read-only");
    plant(&read_only, "READ-ONLY-SENTINEL");
    let checker = lab.path("bin/uid-check");
    let source = lab.path("uid-check.rs");
    fs::write(&source, r#"
use std::{fs,process::Command};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = Command::new("/usr/bin/id").arg("-u").output().unwrap();
    assert!(out.status.success());
    let uid = String::from_utf8(out.stdout).unwrap();
    assert_eq!(uid.trim(), args[1]);
    if uid.trim() != "0" { assert!(fs::read_link("/proc/1/fd/0").is_err(), "private supervisor descriptors exposed"); }
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for field in ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"] {
        let value = status.lines().find(|line| line.starts_with(field)).unwrap().split_whitespace().nth(1).unwrap();
        assert_eq!(value, "0000000000000000", "check retained {field}");
    }
    assert!(fs::read(&args[2]).is_err(), "host secret exposed");
    assert_eq!(fs::read_to_string(&args[3]).unwrap(), "READ-ONLY-SENTINEL");
    assert!(fs::write(&args[3], "overwrite").is_err(), "toolchain input was writable");
    assert_eq!(fs::read_to_string("work.txt").unwrap(), "uid workflow\n");
    println!("VERIFICATION-UID={}", uid.trim());
}
"#).unwrap();
    assert!(Command::new("rustc").args(["--edition", "2021", "-o"]).arg(&checker).arg(&source).status().unwrap().success());
    let config = lab.path(".config/herdr-farm/config.toml");
    let original = fs::read_to_string(&config).unwrap();
    let mode = if owner { "owner" } else { "root" };
    let settings = format!("{original}\n[worker_isolation]\nhide=[{:?}]\n[verification.toolchains.uid]\npaths=[{:?}, '/usr/bin/id', {:?}]\nnetwork=false\ntimeout_seconds=30\n[safety.{:?}]\nworker_uid={mode:?}\n",
        hidden.parent().unwrap().display().to_string(), checker.display().to_string(), read_only.display().to_string(), lab.project.canonicalize().unwrap().display().to_string());
    let settings = if owner { settings } else { settings.replace("worker_uid=\"root\"\n", "") };
    fs::write(&config, &settings).unwrap();
    lab.resume();
    let policy = herdr_farm::verification::toolchains::policy(&lab.project, "uid", vec![checker.display().to_string(), expected.to_string(), hidden.display().to_string(), read_only.display().to_string()]).unwrap();
    lab.install_work_contract_policy("verify_only", &policy);
    lab.write_agent(r#"
use std::{fs, process::Command, path::Path, time::Duration};
fn main() {
    if std::env::args().nth(1).as_deref()==Some("--version") { println!("2.1.0 (Claude Code)"); return; }
    let output = std::env::var("HERDR_FARM_WORKER_OUTPUT").unwrap();
    let output = Path::new(&output);
    fs::create_dir_all(output).unwrap();
    let sample = || {
        let out = Command::new("/usr/bin/id").arg("-u").output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(fs::read(HIDDEN).is_err());
    assert!(fs::write(READ_ONLY, "overwrite").is_err());
    // Private HOME still maps to this same host owner; Git and token login
    // remain usable without an in-sandbox chown.
    fs::write(Path::new(&std::env::var("HOME").unwrap()).join("uid-home"), "ok").unwrap();
    fs::write("work.txt", "uid workflow\n").unwrap();
    fs::write(output.join("report.md"), "Uid workflow\n").unwrap();
    fs::write(output.join("uid-before"), sample()).unwrap();
    while !output.join("sample-again").exists() { std::thread::sleep(Duration::from_millis(20)); }
    fs::write(output.join("uid-after"), sample()).unwrap();
    loop { std::thread::park(); }
}
"#, &[("HIDDEN", hidden.display().to_string()), ("READ_ONLY", read_only.display().to_string())]);
    let (_, attempt) = lab.reserve("Uid mapping workflow");
    lab.serve();
    let mut ticker = lab.spawn();
    let output = lab.project.join(".state/worker-output").join(attempt.as_str());
    lab.wait(&mut ticker, 120, &|| output.join("uid-before").exists());
    let argv: Vec<String> = serde_json::from_value(lab.requests().into_iter()
        .find(|(method, _)| method == "workspace.create_command").unwrap().1["command"].clone()).unwrap();
    let mapping = if owner {
        format!("exec /usr/bin/unshare --user --map-user={uid} --map-group={} --", unsafe { libc::getgid() })
    } else { "exec /usr/bin/unshare --user --map-root-user --".into() };
    assert_eq!(argv.iter().map(|arg| arg.matches(&mapping).count()).sum::<usize>(), 1,
        "launched sandbox must contain the selected exec mapping");

    assert_eq!(fs::read_to_string(output.join("uid-before")).unwrap().trim(), expected.to_string());
    let replacement = if owner { settings.replace("worker_uid=\"owner\"", "worker_uid=\"root\"") }
        else { format!("{settings}worker_uid=\"owner\"\n") };
    fs::write(&config, replacement).unwrap();
    fs::write(output.join("sample-again"), "go").unwrap();
    lab.wait(&mut ticker, 120, &|| output.join("uid-after").exists());
    assert_eq!(fs::read_to_string(output.join("uid-after")).unwrap().trim(), expected.to_string());
    assert_eq!(fs::read_to_string(&read_only).unwrap(), "READ-ONLY-SENTINEL");
    assert_eq!(fs::read_to_string(&hidden).unwrap(), "HIDDEN-SENTINEL");
    let captured = lab.ok_live(&|| ["result", "demo", "submit-captured", attempt.as_str()].map(String::from).to_vec());
    let submission = captured["submission"]["submission_id"].as_str().unwrap();
    let policy_file = lab.path("uid-policy.json");
    fs::write(&policy_file, policy).unwrap();
    let scratch = lab.path("uid-verification");
    let verified = lab.ok_live(&|| ["result", "demo", "verify", submission, "--policy-id", "clean", "--policy-file", policy_file.to_str().unwrap(),
        "--idempotency-key", "uid-verification", "--work-dir", scratch.to_str().unwrap()].map(String::from).to_vec());
    assert_eq!(verified["state"], "accepted", "{verified}");
    assert!(verified["stdout"].as_str().unwrap().contains(&format!("VERIFICATION-UID={expected}")), "{verified}");
    let shown = lab.ok_live(&|| ["result", "demo", "show"].map(String::from).to_vec());
    assert_eq!(shown[0]["verification_runs"][0]["state"], "accepted", "{shown}");
    lab.stop(ticker);
}

#[test]
fn owner_uid_policy_is_retained_in_approved_launch_inputs_without_a_server() {
    let identity = |flag| {
        let out = Command::new("/usr/bin/id").arg(flag).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().parse::<u32>().unwrap()
    };
    let host = (identity("-u"), identity("-g"));
    for owner in [false, true] {
        let mut lab = Lab::new("unknown_usage='allow_with_warning'");
        let config = lab.path(".config/herdr-farm/config.toml");
        let original = fs::read_to_string(&config).unwrap();
        let policy = format!("\n[safety.{:?}]\nworker_uid={:?}\n", lab.project.canonicalize().unwrap().display().to_string(), if owner { "owner" } else { "root" });
        if owner {
            fs::write(&config, format!("{original}{policy}")).unwrap();
            lab.resume();
        }
        let (_, attempt) = lab.reserve("Frozen identity workflow");
        let retained = lab.state().attempt_inputs.into_iter().find(|input| input.attempt == attempt).unwrap();
        assert_eq!(retained.inputs.worker_uid, owner.then_some(host));
        if !owner { assert!(serde_json::to_value(&retained.inputs).unwrap().get("worker_uid").is_none()); }
        fs::write(&config, if owner { original.clone() } else { format!("{original}{policy}").replace("worker_uid=\"root\"", "worker_uid=\"owner\"") }).unwrap();
        let after = lab.state().attempt_inputs.into_iter().find(|input| input.attempt == attempt).unwrap();
        assert_eq!(after, retained, "owner config edited a retained attempt");
    }
}
