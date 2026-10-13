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
mod inventory_history { include!("support/inventory_history.rs"); }
const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");
// A deterministic fake Herdr and fake agent. The listener provides a genuine
// session incarnation; the fake CLI implements the public Herdr command/API
// boundary and persists the simulated agent's observable state between calls.
const HERDR: &str = include_str!("support/coordinator-herdr.py");
struct Lab {
    home: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    herdr: PathBuf,
}
impl Lab {
    fn new() -> Self { Self::configured(false) }
    fn configured(defaults: bool) -> Self {
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
        // Exact inbox and prompt workflows are independent of automatic health delivery.
        let config = lab.home.path().join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "[telemetry]\nhealth_notify = 'off'\n").unwrap();
        lab.ok(&["new", "demo"]);
        lab.ok(&["pause", "demo"]);
        let plan = if defaults {
            let config = lab.home.path().join(".config/herdr-farm/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, format!("[telemetry]\nhealth_notify = 'off'\n[verification.defaults.{:?}]\naccept=['godot:./tools/run-tests.sh', 'shell:./tools/check.sh --fast']\n", lab.project.canonicalize().unwrap().to_string_lossy())).unwrap();
            migration::inspect_with_config(&lab.project, &config).unwrap()
        } else {
            migration::inspect(&lab.project).unwrap()
        };
        migration::apply(&lab.project, &plan, true).unwrap();
        lab
    }
    fn cli(&self, args: &[&str]) -> Output {
        let mut legacy_args = args.to_vec();
        if args.first() == Some(&"new") { legacy_args.insert(1, "--legacy"); }
        let args = legacy_args.as_slice();
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
    /// Like `cli`, but retries the transient lock refusal a command can meet
    /// while the accelerated test ticker holds the root for a pass, so the
    /// outcome reflects the command's own decision.
    fn settled(&self, args: &[&str]) -> Output {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let out = self.cli(args);
            let busy = String::from_utf8_lossy(&out.stderr).contains("lock acquisition failed");
            if !busy || std::time::Instant::now() >= until { return out; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    /// `ok` for a command that can meet the accelerated ticker's pass lock.
    fn settled_ok(&self, args: &[&str]) -> String {
        let o = self.settled(args);
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
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
        let args = ["ticker", "stop"];
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let out = self.cli(&args);
            if out.status.success() { return; }
            // A stop requested mid-pass can outlast the scaled CLI wait.
            assert!(std::time::Instant::now() < until,
                "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
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
        "--accept TOOLCHAIN:COMMAND",
        "Acceptance toolchains are owner-declared",
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
            "[telemetry]\nhealth_notify = 'off'\n[safety.{:?}]\ncoordinator_agent_args=['--fixture-safe']\ncoordinator_agent_args_kind='claude'\n",
            l.project.to_string_lossy()
        ),
    )
    .unwrap();
    let owner_settings = l.project.join(".claude/settings.local.json");
    fs::create_dir_all(owner_settings.parent().unwrap()).unwrap();
    let owner_bytes = b"{\"permissions\":{\"allow\":[\"Bash(echo:*)\"]}}\n";
    fs::write(&owner_settings, owner_bytes).unwrap();
    let alias = l.home.path().join("git/herdr-projects");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    l.settled_ok(&["open", "demo"]);
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
    let settings_path = l.project.join(".state/coordinator/claude-settings.json");
    let args = state["args"].as_array().unwrap();
    let settings_arg = args.iter().position(|v| v == "--settings").unwrap();
    assert_eq!(args[settings_arg + 1], settings_path.to_string_lossy().as_ref());
    assert_eq!(fs::metadata(&settings_path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(settings_path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    let permissions: Value = serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
    let prefix = format!("{} --root {}", BIN, l.root.display());
    for verb in ["skill", "context demo", "inbox list", "inbox done", "inbox demo wait", "task demo list", "task demo show", "task demo add", "task demo rename",
        "launch demo run", "launch demo stop", "result demo show", "result demo jobs", "result demo capture", "result demo submit-captured",
        "scheduler demo inspect", "operations demo inspect", "runtime demo inspect", "telemetry demo usage", "memory-review demo propose", "memory demo record", "doctor",
        "thread prompt", "safety requests", "telemetry demo review show", "telemetry demo review findings show",
        "telemetry demo review findings validate", "telemetry demo review findings reject", "telemetry demo review findings duplicate"] {
        assert!(permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({prefix} {verb}:*)"))), "missing allow {verb}");
    }
    let memory_rules=permissions["permissions"]["allow"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|rule|rule.starts_with(&format!("Bash({prefix} memory demo "))).collect::<Vec<_>>();
    assert_eq!(memory_rules.len(),5);
    for verb in ["list","show","approve","reject","record"] {
        assert!(memory_rules.contains(&format!("Bash({prefix} memory demo {verb}:*)").as_str()));
    }
    for verb in ["merge", "unmerge", "split", "restore", "reset"] {
        assert!(!permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({prefix} telemetry demo review findings {verb}:*)"))));
    }
    for verb in ["accept", "protocols", "experiments"] {
        assert!(!permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({prefix} telemetry demo review {verb}:*)"))));
    }
    assert!(!permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({BIN}:*)"))));
    assert!(permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({prefix} reconcile demo --plan)"))));
    for verb in ["owner demo approve", "owner demo reject", "safety approve", "safety reject", "safety revoke", "approval", "delegation", "budget", "routine-store", "migration", "archive", "delete",
        "runtime * state", "runtime * rebind", "runtime * adopt", "runtime * relinquish", "task * cancel-attempt", "task * complete"] {
        assert!(permissions["permissions"]["ask"].as_array().unwrap().contains(&json!(format!("Bash({prefix} {verb}:*)"))), "missing ask {verb}");
    }
    for path in ["~/.config/herdr-projects/**".into(), "~/.config/herdr-farm/**".into(), "~/.ssh/**".into(),
        format!("/{}", settings_path.display()), format!("/{}/.claude/**", l.project.display())] {
        for tool in ["Read", "Edit", "Write"] {
            assert!(permissions["permissions"]["deny"].as_array().unwrap().contains(&json!(format!("{tool}({path})"))));
        }
    }
    for rule in ["Bash(ssh-keygen:*)", "Bash(env *)", "Bash(sh -c *)", "Bash(bash -c *)", "Bash(bash -lc *)", "Bash(zsh -c *)", "Bash(eval *)", "Bash(command *)", "Bash(exec *)", "Bash(xargs *)", "Bash(direnv exec *)", "Bash(devbox run *)", "Bash(mise exec *)", "Bash(npx *)", "Bash(docker exec *)", "Bash(nocorrect *)", "Bash(watch *)", "Bash(setsid *)", "Bash(ionice *)", "Bash(flock *)"] {
        assert!(permissions["permissions"]["deny"].as_array().unwrap().contains(&json!(rule)));
    }
    assert_eq!(serde_json::from_str::<Value>(&l.ok(&["runtime", "demo", "inspect"])).unwrap()["control"]["state"], "active");
    let context = l.ok(&["context", "demo", "--peek"]);
    assert!(context.contains("Launchable retained profile evidence: none"));
    assert!(context.contains("Control state: Active"));
    assert!(context.contains("claude-settings.json"));
    // A focus-only open re-adopts changed owner config and resumes automatic pause.
    let mut changed = fs::read_to_string(&config).unwrap();
    changed.push_str("# owner configuration changed\n");
    fs::write(&config, changed).unwrap();
    l.settled_ok(&["open", "demo"]);
    l.stop();
    assert_eq!(serde_json::from_str::<Value>(&l.ok(&["runtime", "demo", "inspect"])).unwrap()["control"]["state"], "active");
    let adopted = runtime::snapshot(&l.project).unwrap();
    assert!(adopted.ownership.iter().any(|o| o.binding == "coordinator" && o.config_digest == migration::config_reference(&config).unwrap().digest));
    let control = adopted.control.unwrap();
    l.settled_ok(&["runtime", "demo", "state", "paused", "--expected-revision", &control.revision.to_string(), "--expected-head", &adopted.head.to_string()]);
    l.settled_ok(&["open", "demo"]);
    l.stop();
    assert_eq!(serde_json::from_str::<Value>(&l.ok(&["runtime", "demo", "inspect"])).unwrap()["control"]["state"], "paused");
    // Restore active for the replacement workflow below.
    let paused = runtime::snapshot(&l.project).unwrap();
    l.settled_ok(&["runtime", "demo", "state", "active", "--expected-revision", &paused.control.unwrap().revision.to_string(), "--expected-head", &paused.head.to_string()]);
    assert_eq!(fs::read(owner_settings).unwrap(), owner_bytes);
    let alias_prefix = format!("{} --root {}", alias.display(), l.root.display());
    assert!(permissions["permissions"]["allow"].as_array().unwrap().contains(&json!(format!("Bash({alias_prefix} task demo add:*)"))));
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
    l.settled_ok(&["open", "demo"]);
    l.stop();
    assert_eq!(l.state()["starts"], 1);
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(), 2);
    let refused = l.settled(&["open", "demo", "--reprime"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not ready"));
    let mut done = l.state();
    done["status"] = json!("done");
    done["accepted"] = json!(false);
    fs::write(l.home.path().join("herdr-state.json"), serde_json::to_vec(&done).unwrap()).unwrap();
    // Append-only launch history across the root must not stall reprime.
    l.ok(&["new", "history"]);
    l.ok(&["pause", "history"]);
    let history = l.root.join("history");
    migration::apply(&history, &migration::inspect(&history).unwrap(), true).unwrap();
    // Three counted records per launch: 350 * 3 = 1050 exceeds the old
    // 1024 controller-hint candidate budget without slowing the lab stop pass.
    inventory_history::seed(&history, 350);
    l.settled_ok(&["open", "demo", "--reprime"]);
    l.stop();
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(), 3);
    // Older effect journals migrate through a public focus-only open without
    // inventing startup provenance or restarting the accepted agent.
    let journal_path = l.project.join(".state/canonical-coordinator.json");
    let mut old: Value = serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    old["version"] = json!(1);
    old.as_object_mut().unwrap().remove("permission_file");
    fs::write(&journal_path, serde_json::to_vec(&old).unwrap()).unwrap();
    l.settled_ok(&["open", "demo"]);
    l.stop();
    let upgraded: Value = serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    assert_eq!(upgraded["version"], 2);
    assert!(upgraded["permission_file"].is_null());
    assert_eq!(l.state()["starts"], 1);
    assert!(l.ok(&["context", "demo", "--peek"]).contains("no generated file recorded"));
    let retained = runtime::snapshot(&l.project).unwrap().ownership;
    let mut state = l.state();
    state["live"] = json!(false);
    // An agent without its pane is uncertainty, not proof of absence.
    fs::write(l.home.path().join("herdr-state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(!l.settled(&["open", "demo", "--reprime"]).status.success());
    assert_eq!(l.state()["creates"], 1);
    assert_eq!(runtime::snapshot(&l.project).unwrap().ownership, retained);
    state["agent"] = json!(false);
    state["fail_inventory"] = json!(true);
    fs::write(l.home.path().join("herdr-state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(!l.settled(&["open", "demo", "--reprime"]).status.success());
    assert_eq!(l.state()["creates"], 1);
    assert_eq!(runtime::snapshot(&l.project).unwrap().ownership, retained);
    state["fail_inventory"] = json!(false);
    fs::write(
        l.home.path().join("herdr-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    l.ok(&["ticker", "start"]);
    let reopening = std::time::Instant::now();
    let reopened = l.settled(&["open", "demo", "--reprime"]);
    assert!(reopening.elapsed() < std::time::Duration::from_secs(5),
        "foreground reprime starved while ticker was running: {:?}", reopening.elapsed());
    assert!(reopened.status.success(), "{}", String::from_utf8_lossy(&reopened.stderr));
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
    assert!(!l.settled(&["open", "demo", "--reprime"]).status.success());
    let sent = l.state()["prompts"].as_array().unwrap().len();
    l.settled_ok(&["open", "demo"]);
    l.stop();
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(), sent);
}

#[test]
fn socket_coordinator_remote_manifest_and_explicit_local_override_policy() {
    for mode in ["remote", "remote-shadow", "local", "outside", "symlink", "relative"] {
        let l = Lab::new();
        let _listener = UnixListener::bind(l.home.path().join("s"))
            .expect("socket fixture: sandbox may deny Unix sockets");
        let state_dir = l.home.path().join(".local/state/herdr/agent-detection/remote");
        fs::create_dir_all(&state_dir).unwrap();
        let manifest = state_dir.join("claude.toml");
        fs::write(&manifest, "fixture").unwrap();
        let outside = l.home.path().join("outside.toml");
        fs::write(&outside, "fixture").unwrap();
        let link = state_dir.join("link.toml");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let source = match mode {
            "local" => format!("local:{}", manifest.display()),
            "outside" => format!("remote:{}", outside.display()),
            "symlink" => format!("remote:{}", link.display()),
            "relative" => "remote:relative.toml".into(),
            _ => format!("remote:{}", manifest.display()),
        };
        fs::write(l.home.path().join("herdr-state.json"), serde_json::to_vec(&json!({
            "creates":0,"starts":0,"prompts":[],"live":false,"agent":false,"accepted":false,
            "manifest_source":source,"shadow":mode == "local" || mode == "remote-shadow"
        })).unwrap()).unwrap();
        let out = l.settled(&["open", "demo"]);
        if mode != "remote" {
            assert!(!out.status.success());
            let error = String::from_utf8_lossy(&out.stderr);
            assert!(error.contains(if mode == "local" || mode == "remote-shadow" { "local manifest override shadowing remote refused" }
                else if mode == "relative" { "must be absolute" } else { "outside owner's Herdr state dir" }), "{error}");
            assert!(l.state()["prompts"].as_array().unwrap().is_empty());
            let doctor = l.cli(&["doctor"]);
            assert!(String::from_utf8_lossy(&doctor.stdout).contains("refused:"));
            if mode != "local" && mode != "remote-shadow" { continue; }
            let config = l.home.path().join(".config/herdr-farm/config.toml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(config, "[telemetry]\nhealth_notify = 'off'\n[coordinator]\nallow_local_manifest_override = true\n").unwrap();
            l.ok(&["open", "demo", "--reprime"]);
        } else {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        l.stop();
        assert_eq!(l.state()["prompts"].as_array().unwrap().len(), 1);
        let journal: Value = serde_json::from_slice(&fs::read(l.project.join(".state/canonical-coordinator.json")).unwrap()).unwrap();
        assert_eq!(journal["priming_manifest"]["source"], source);
        assert_eq!(journal["priming_manifest"]["version"], "fixture-1");
        assert_eq!(journal["phase"], "accepted");
        assert!(runtime::snapshot(&l.project).unwrap().ownership.iter().any(|o| o.binding == "coordinator"));
        let doctor = l.cli(&["doctor"]);
        let report = String::from_utf8_lossy(&doctor.stdout);
        assert!(report.contains("coordinator priming manifest:") && report.contains(&source), "{report}");
        assert!(report.contains("manifest policy accepted"), "{report}");
    }
}

#[test]
fn unchanged_ticker_observations_preserve_head_and_refresh_coordinator_admission() {
    let l = Lab::new();
    let _listener = UnixListener::bind(l.home.path().join("s"))
        .expect("socket fixture: sandbox may deny Unix sockets");
    fs::write(l.home.path().join("herdr-state.json"), serde_json::to_vec(&json!({
        "creates":1,"starts":1,"prompts":[],"live":true,"agent":true,"accepted":false,"pane":"w1:p1","cwd":l.project
    })).unwrap()).unwrap();
    let route = l.home.path().join("route.json");
    fs::write(&route, serde_json::to_vec(&json!({"socket":l.home.path().join("s"),"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1","cwd":l.project})).unwrap()).unwrap();
    let initial: Value = serde_json::from_str(&l.ok(&["runtime", "demo", "inspect"])).unwrap();
    l.ok(&["runtime", "demo", "create", "--route", route.to_str().unwrap(), "--expected-head", &initial["head"].to_string()]);
    l.ok(&["reconcile", "demo", "--record"]);
    let bound = runtime::snapshot(&l.project).unwrap();
    l.ok(&["runtime", "demo", "adopt", "coordinator", "--expected-revision", &bound.runtime_bindings[0].revision.to_string(), "--expected-head", &bound.head.to_string()]);
    let first: Value = serde_json::from_str(&l.ok(&["runtime", "demo", "inspect"])).unwrap();
    let started = std::time::Instant::now();
    // The admission freshness window is 30 real seconds. Keep collecting
    // beyond it so activation must use the refreshed row, not the first event.
    while started.elapsed() <= std::time::Duration::from_secs(31) {
        l.ok(&["ticker", "run", "--passes", "10"]);
    }
    let last: Value = serde_json::from_str(&l.ok(&["runtime", "demo", "inspect"])).unwrap();
    assert_eq!(last["head"], first["head"]);
    assert!(last["observations"][0]["observed_unix_ms"].as_i64().unwrap() - first["observations"][0]["observed_unix_ms"].as_i64().unwrap() > 30_000);
    let control = runtime::snapshot(&l.project).unwrap().control.unwrap();
    l.ok(&["runtime", "demo", "state", "active", "--expected-revision", &control.revision.to_string(), "--expected-head", &first["head"].to_string()]);
}

#[test]
fn owner_settings_arguments_are_refused_before_coordinator_effects() {
    for argument in ["--settings", "--settings=/tmp/owner.json"] {
        let l = Lab::new();
        let config = l.home.path().join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, format!("[telemetry]\nhealth_notify = 'off'\n[safety.{:?}]\ncoordinator_agent_args=[{argument:?}]\ncoordinator_agent_args_kind='claude'\n", l.project.to_string_lossy())).unwrap();
        let out = l.cli(&["open", "demo"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("must not contain --settings"));
        assert!(!l.project.join(".state/coordinator/claude-settings.json").exists());
        assert!(!l.home.path().join("herdr-state.json").exists());
    }
}

#[test]
fn explicit_pause_of_already_paused_control_retains_owner_intent() {
    let l = Lab::new();
    let initial = l.ok(&["context", "demo", "--peek"]);
    assert!(initial.contains("run `open demo` to re-activate"));
    let s = runtime::snapshot(&l.project).unwrap();
    l.ok(&["runtime", "demo", "state", "paused", "--expected-revision", &s.control.unwrap().revision.to_string(), "--expected-head", &s.head.to_string()]);
    let paused = l.ok(&["context", "demo", "--peek"]);
    assert!(paused.contains("Control state: Paused"));
    assert!(!paused.contains("run `open demo` to re-activate"));
    let s = runtime::snapshot(&l.project).unwrap();
    assert!(s.events.iter().any(|e| e.kind == "project.control_changed" && e.payload["state"] == "paused"));
}

#[test]
fn non_claude_context_and_doctor_explain_permissions_are_not_generated() {
    let l = Lab::new();
    let md = l.project.join("PROJECT.md");
    let text = fs::read_to_string(&md).unwrap().replace("coordinator_agent = \"claude\"", "coordinator_agent = \"codex\"");
    fs::write(md, text).unwrap();
    let context = l.ok(&["context", "demo", "--peek"]);
    assert!(context.contains("none generated for codex (Claude Code only)"));
    let out = l.cli(&["doctor"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("none generated for codex (Claude Code only)"));
    assert!(!l.project.join(".state/coordinator/claude-settings.json").exists());
}

#[test]
fn inbox_wait_wakes_for_a_lost_attempt_without_marking_the_notice_seen() {
    use herdr_farm::domain::{Attempt, AttemptId, AttemptState, TaskState, Commit, Mutation};
    let lab = Lab::new();
    let mut db = migration::open_active(&lab.project).unwrap();
    let head = db.current_head().unwrap();
    lab.ok(&["task", "demo", "add", "lost-work", "--title", "Lost worker", "--expected-head", &head.to_string()]);
    let mut task = db.read_snapshot(None).unwrap().tasks.into_iter().find(|t| t.id.as_str() == "lost-work").unwrap();
    let mut attempt = Attempt { id: AttemptId::new("lost-attempt").unwrap(), task: task.id.clone(), revision: 1,
        state: AttemptState::Running, snapshot: None, reservation: "lost-reservation".into(), termination_observed: false };
    task.revision += 1;
    task.state = TaskState::Running;
    task.active_attempt = Some(attempt.id.clone());
    db.commit(Commit { expected_head: db.current_head().unwrap(), mutations: vec![
        Mutation::Task { expected: Some(task.revision - 1), next: task.clone() },
        Mutation::Attempt { expected: None, next: attempt.clone() },
    ] }).unwrap();
    let mut waiter = Command::new(BIN).env_clear().env("HOME", lab.home.path()).env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .args(["--root", lab.root.to_str().unwrap(), "inbox", "demo", "wait", "--timeout", "60"])
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    assert!(waiter.try_wait().unwrap().is_none());
    attempt.state = AttemptState::Lost;
    attempt.revision += 1;
    db.commit(Commit { expected_head: db.current_head().unwrap(), mutations: vec![
        Mutation::Attempt { expected: Some(attempt.revision - 1), next: attempt.clone() },
    ] }).unwrap();
    let output = waiter.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let wake: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items: Value = serde_json::from_str(&lab.ok(&["inbox", "list", "demo"])).unwrap();
    let notice = items.as_array().unwrap().iter().find(|i| i["content"]["kind"] == "attempt.ended_without_submission").unwrap();
    assert_eq!(wake, json!({"items":1,"ids":[notice["content"]["id"]]}));
    assert_eq!((notice["seen"].clone(), notice["done"].clone()), (json!(false), json!(false)));
    let context = lab.ok(&["context", "demo"]);
    assert!(context.contains(notice["content"]["id"].as_str().unwrap()));
    let after_context: Value = serde_json::from_str(&lab.ok(&["inbox", "demo", "wait", "--timeout", "1"])).unwrap();
    assert_eq!(after_context, wake, "SEEN does not consume a worker-result wake");
    let health:Value=serde_json::from_str(&lab.ok(&["telemetry","demo","health","evaluate","--json"])).unwrap();
    let waiting=health["states"].as_array().unwrap().iter().find(|s|s["rule"]=="result_waiting_on_coordinator").unwrap();
    assert_eq!(waiting["evidence"]["waiting_items"],1);
    assert!(waiting["evidence"]["age_ms"].as_i64().is_some_and(|ms|ms>=0));
    lab.ok(&["inbox", "done", "demo", notice["content"]["id"].as_str().unwrap()]);
    let timeout: Value = serde_json::from_str(&lab.ok(&["inbox", "demo", "wait", "--timeout", "1"])).unwrap();
    assert_eq!(timeout, json!({"items":0,"timed_out":true}));
    let health:Value=serde_json::from_str(&lab.ok(&["telemetry","demo","health","evaluate","--json"])).unwrap();
    let waiting=health["states"].as_array().unwrap().iter().find(|s|s["rule"]=="result_waiting_on_coordinator").unwrap();
    assert_eq!(waiting["evidence"]["waiting_items"],0);
    assert_eq!(waiting["state"],"ok");
    assert!(db.unseen_inbox().unwrap().is_empty());
}

#[test]
fn legacy_inbox_wait_times_out_without_creating_seen_state() {
    let lab = Lab::new();
    lab.ok(&["new", "legacy"]);
    let project = lab.root.join("legacy");
    let timeout: Value = serde_json::from_str(&lab.ok(&["inbox", "legacy", "wait", "--timeout", "1"])).unwrap();
    assert_eq!(timeout, json!({"items":0,"timed_out":true}));
    assert!(!project.join(".state/inbox-seen.json").exists());
    assert!(fs::read_dir(project.join("inbox")).unwrap().flatten().all(|e| e.path().is_dir()));
}

// A separate local process owns the same OS guard as a ticker pass. Readiness
// comes from its stdout, so the CLI always encounters an already-held lock.
fn hold_lock(path: &std::path::Path, seconds: &str) -> std::process::Child {
    use std::io::BufRead;
    let mut child = Command::new("python3")
        .args(["-c", "import fcntl,sys,time; f=open(sys.argv[1],'a'); fcntl.flock(f,fcntl.LOCK_EX); print('held',flush=True); time.sleep(float(sys.argv[2]))"])
        .arg(path).arg(seconds)
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "held");
    child
}

#[test]
fn open_waits_for_brief_project_and_root_contention() {
    let l = Lab::new();
    let _socket = UnixListener::bind(l.home.path().join("s")).unwrap();
    for path in [l.project.join(".state/effect.lock"), l.root.join(".execution.lock")] {
        let mut holder = hold_lock(&path, "0.2");
        l.ok(&["open", "demo"]);
        assert!(holder.wait().unwrap().success());
        l.stop();
        assert_eq!(runtime::snapshot(&l.project).unwrap().runtime_bindings.iter()
            .find(|b| b.id == "coordinator").unwrap().identity.pane_id, "w1:p1");
    }
}

#[test]
fn open_reports_existing_refusal_after_bounded_contention() {
    let l = Lab::new();
    for path in [l.project.join(".state/effect.lock"), l.root.join(".execution.lock")] {
        let mut holder = hold_lock(&path, "5");
        let started = std::time::Instant::now();
        let out = l.cli(&["open", "demo"]);
        let elapsed = started.elapsed();
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert!(!out.status.success());
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains("retry") && error.contains("operation would block"), "{error}");
        let bound = 30.0 * include_str!("support/time-scale.txt").trim().parse::<f64>().unwrap();
        assert!(elapsed.as_secs_f64() >= bound, "{elapsed:?}");
        assert!(elapsed.as_secs_f64() < bound + 3.0, "{elapsed:?}");
        assert!(!l.home.path().join("herdr-state.json").exists());
        assert!(runtime::snapshot(&l.project).unwrap().runtime_bindings.iter().all(|b| b.id != "coordinator"));
    }
}

#[test]
fn migrated_doctor_reports_canonical_binding_despite_stale_legacy_record() {
    let l = Lab::new();
    let _socket = UnixListener::bind(l.home.path().join("s")).unwrap();
    l.ok(&["open", "demo"]);
    l.stop();
    fs::write(l.project.join(".state/coordinator.json"), json!({
        "socket":l.home.path().join("s"), "pane_id":"w1R:p1",
        "workspace_id":"w1R", "tab_id":"w1R:t1", "agent_name":"coordinator"
    }).to_string()).unwrap();
    let out = l.cli(&["doctor"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("canonical coordinator binding: pane w1:p1"), "{text}");
    assert!(text.contains("canonical coordinator journal: pane w1:p1"), "{text}");
    assert!(!text.contains("w1R:p1"), "{text}");
}

#[test]
fn migrated_doctor_uses_public_store_binding_without_a_coordinator_journal() {
    let l = Lab::new();
    let head = runtime::snapshot(&l.project).unwrap().head;
    runtime::create_binding(&l.project, None, None, head, &herdr_farm::domain::RuntimeRoute {
        socket: l.home.path().join("absent-session").display().to_string(),
        pane_id: "canonical-pane".into(), workspace_id: "canonical-workspace".into(),
        tab_id: "canonical-tab".into(), cwd: l.project.display().to_string(),
        ..Default::default()
    }).unwrap();
    fs::write(l.project.join(".state/coordinator.json"), json!({
        "socket":l.home.path().join("old-session"), "pane_id":"w1R:p1",
        "workspace_id":"w1R", "tab_id":"w1R:t1", "agent_name":"coordinator"
    }).to_string()).unwrap();
    let out = l.cli(&["doctor"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("canonical coordinator binding: pane canonical-pane"), "{text}");
    assert!(!text.contains("w1R:p1") && !text.contains("old-session"), "{text}");
}

#[test]
fn legacy_open_waits_then_reports_bounded_root_refusal() {
    let l = Lab::new();
    l.ok(&["new", "legacy"]);
    let mut holder = hold_lock(&l.root.join(".execution.lock"), "5");
    let started = std::time::Instant::now();
    let out = l.cli(&["open", "legacy"]);
    let elapsed = started.elapsed();
    holder.kill().unwrap();
    holder.wait().unwrap();
    let bound = 30.0 * include_str!("support/time-scale.txt").trim().parse::<f64>().unwrap();
    assert!(elapsed.as_secs_f64() >= bound && elapsed.as_secs_f64() < bound + 3.0, "{elapsed:?}");
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("another operation owns lock") && error.contains("operation would block"), "{error}");
    assert!(!l.home.path().join("herdr-state.json").exists());
}

/// Public store intake plus ticker/launch/inbox CLI: no live services required.
#[test]
fn restarted_ticker_sweeps_ended_server_but_retains_active_worker_server() {
    use herdr_farm::domain::{Attempt, AttemptId, AttemptState, TaskState, Commit, Mutation};
    struct Server(std::process::Child);
    impl Drop for Server { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    let lab = Lab::new();
    let mut db = migration::open_active(&lab.project).unwrap();
    lab.ok(&["task", "demo", "add", "private", "--title", "Private worker", "--expected-head", &db.current_head().unwrap().to_string()]);
    let mut task = db.read_snapshot(None).unwrap().tasks.into_iter().find(|t| t.id.as_str() == "private").unwrap();
    let mut attempt = Attempt { id: AttemptId::new("private-attempt").unwrap(), task: task.id.clone(), revision: 1, state: AttemptState::Running, snapshot: None, reservation: "private-reservation".into(), termination_observed: false };
    task.revision += 1; task.state = TaskState::Running; task.active_attempt = Some(attempt.id.clone());
    db.commit(Commit { expected_head: db.current_head().unwrap(), mutations: vec![Mutation::Task { expected: Some(task.revision - 1), next: task }, Mutation::Attempt { expected: None, next: attempt.clone() }] }).unwrap();
    let directory = lab.root.join(".herdr-run/demo-private/herdr");
    fs::create_dir_all(&directory).unwrap();
    let socket_dir = lab.home.path().join("rprivate");
    fs::create_dir(&socket_dir).unwrap();
    let socket = socket_dir.join("s");
    fs::write(&socket, b"fixture socket marker").unwrap();
    let mut server = Server(Command::new("/usr/bin/python3").args(["-c", "import time; time.sleep(300)", "server"]).env("HERDR_SOCKET_PATH", &socket).spawn().unwrap());
    fs::write(directory.join("server.json"), json!({"pid":server.0.id(),"socket":socket}).to_string()).unwrap();
    // An orphaned record must also be retired even without a task row.
    let orphan_dir = lab.root.join(".herdr-run/demo-missing/herdr");
    fs::create_dir_all(&orphan_dir).unwrap();
    let orphan_socket = lab.home.path().join("rorphan/s");
    fs::create_dir_all(orphan_socket.parent().unwrap()).unwrap();
    fs::write(&orphan_socket, b"orphan socket marker").unwrap();
    let mut orphan = Server(Command::new("/usr/bin/python3").args(["-c", "import time; time.sleep(300)", "server"]).env("HERDR_SOCKET_PATH", &orphan_socket).spawn().unwrap());
    fs::write(orphan_dir.join("server.json"), json!({"pid":orphan.0.id(),"socket":orphan_socket}).to_string()).unwrap();
    lab.ok(&["ticker", "run", "--passes", "1"]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = orphan.0.try_wait().unwrap() { assert!(status.code().is_none()); break; }
        assert!(std::time::Instant::now() < deadline, "orphaned server was not stopped");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!orphan_dir.join("server.json").exists());
    assert!(!orphan_socket.exists());
    assert!(server.0.try_wait().unwrap().is_none());
    assert!(directory.join("server.json").exists());
    assert!(!lab.cli(&["launch", "demo", "stop", "--task", "private"]).status.success());
    // End through the public store while the ticker is down. Leave the old
    // task pointer in place: retirement must use attempt state, not that pointer.
    attempt.revision += 1; attempt.state = AttemptState::Lost; attempt.termination_observed = true;
    db.commit(Commit { expected_head: db.current_head().unwrap(), mutations: vec![Mutation::Attempt { expected: Some(attempt.revision - 1), next: attempt }] }).unwrap();
    lab.ok(&["ticker", "run", "--passes", "1"]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while server.0.try_wait().unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!directory.join("server.json").exists());
    assert!(!socket.exists());
}

#[test]
fn idle_observations_deliver_durable_inbox_advisories_without_stopping_worker() {
    use herdr_farm::domain::{Attempt, AttemptId, AttemptState, TaskState, Commit, Mutation};
    let lab = Lab::new();
    let mut db = migration::open_active(&lab.project).unwrap();
    lab.ok(&["task", "demo", "add", "quiet", "--title", "Quiet worker", "--expected-head", &db.current_head().unwrap().to_string()]);
    let mut task = db.read_snapshot(None).unwrap().tasks.into_iter().find(|t| t.id.as_str() == "quiet").unwrap();
    let attempt = Attempt { id: AttemptId::new("quiet-attempt").unwrap(), task: task.id.clone(), revision: 1, state: AttemptState::Running, snapshot: None, reservation: "quiet-reservation".into(), termination_observed: false };
    task.revision += 1; task.state = TaskState::Running; task.active_attempt = Some(attempt.id.clone());
    db.commit(Commit { expected_head: db.current_head().unwrap(), mutations: vec![Mutation::Task { expected: Some(task.revision - 1), next: task }, Mutation::Attempt { expected: None, next: attempt.clone() }] }).unwrap();
    let now = jiff::Timestamp::now().as_millisecond();
    use herdr_farm::domain::ReminderOutcome;
    assert_eq!(db.notify_worker_idle(db.current_head().unwrap(), &attempt.id, 1, now, now + 600_000).unwrap(), Some(ReminderOutcome::Delivered));
    drop(db);
    let mut db = migration::open_active(&lab.project).unwrap();
    assert_eq!(db.notify_worker_idle(db.current_head().unwrap(), &attempt.id, 1, now, now + 700_000).unwrap(), Some(ReminderOutcome::AlreadyDelivered));
    let rows: Value = serde_json::from_str(&lab.ok(&["inbox", "list", "demo"])).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["content"]["summary"], "attempt quiet-attempt: worker idle for 10 min without submitting");
    lab.ok(&["inbox", "done", "demo", rows[0]["content"]["id"].as_str().unwrap()]);
    assert_eq!(db.notify_worker_idle(db.current_head().unwrap(), &attempt.id, 1, now, now + 800_000).unwrap(), Some(ReminderOutcome::AlreadyDelivered));
    assert_eq!(db.notify_worker_idle(db.current_head().unwrap(), &attempt.id, 1, now + 1_000_000, now + 1_600_000).unwrap(), Some(ReminderOutcome::Delivered));
    let snapshot = runtime::snapshot(&lab.project).unwrap();
    assert_eq!(snapshot.inbox.len(), 2);
    assert_eq!(snapshot.attempts[0], attempt);
}

#[test]
fn canonical_command_list_names_owner_default_acceptance() {
    let lab = Lab::configured(true);
    let text = lab.ok(&["context", "demo", "--peek"]);
    assert!(text.contains("Code launches automatically carry owner acceptance defaults: godot:./tools/run-tests.sh, shell:./tools/check.sh --fast"), "{text}");
    assert!(text.contains("--no-default-accept opts out for one launch"), "{text}");
    assert!(!text.contains("--accept TOOLCHAIN:COMMAND"), "{text}");
}

/// Public lifecycle API produces the actual lost-worker notice, rather than
/// planting an inbox row or preconstructing a delivery receipt.
fn lose_worker(lab:&Lab, name:&str) -> String {
    use herdr_farm::domain::{Attempt,AttemptId,AttemptState,TaskState,Commit,Mutation};
    let mut db=migration::open_active(&lab.project).unwrap();
    lab.ok(&["task","demo","add",name,"--title","Lost worker","--expected-head",&db.current_head().unwrap().to_string()]);
    let mut task=db.read_snapshot(None).unwrap().tasks.into_iter().find(|t|t.id.as_str()==name).unwrap();
    let mut attempt=Attempt{id:AttemptId::new(format!("{name}-attempt")).unwrap(),task:task.id.clone(),revision:1,state:AttemptState::Running,snapshot:None,reservation:format!("{name}-reservation"),termination_observed:false};
    task.revision+=1;task.state=TaskState::Running;task.active_attempt=Some(attempt.id.clone());
    db.commit(Commit{expected_head:db.current_head().unwrap(),mutations:vec![Mutation::Task{expected:Some(task.revision-1),next:task},Mutation::Attempt{expected:None,next:attempt.clone()}]}).unwrap();
    attempt.revision+=1;attempt.state=AttemptState::Lost;
    db.commit(Commit{expected_head:db.current_head().unwrap(),mutations:vec![Mutation::Attempt{expected:Some(attempt.revision-1),next:attempt}]}).unwrap();
    db.worker_result_inbox().unwrap().into_iter().find(|i|i.content.subject==name).unwrap().content.id
}
fn wake_passes(lab:&Lab, passes:&str) {lab.ok(&["ticker","run","--passes",passes]);}
fn wake_history(lab:&Lab)->Value {
    let health:Value=serde_json::from_str(&lab.ok(&["telemetry","demo","health","--json"])).unwrap();
    health["states"].as_array().unwrap().iter().find(|s|s["rule"]=="result_waiting_on_coordinator").unwrap()["evidence"].clone()
}
#[test]
fn canonical_toast_backoff_and_live_nudge_policy_survive_context_reads() {
    let l=Lab::new();
    // This health-notice wake workflow deliberately retains the default policy.
    fs::write(l.home.path().join(".config/herdr-farm/config.toml"), "").unwrap();
    let _listener=UnixListener::bind(l.home.path().join("s")).expect("socket fixture: sandbox may deny Unix sockets");
    l.ok(&["open","demo"]);l.stop();
    let initial=l.state()["prompts"].as_array().unwrap().len();
    let id=lose_worker(&l,"toast-work");
    l.ok(&["context","demo"]);
    let mut native=l.state();native["toast_busy"]=json!(true);
    fs::write(l.home.path().join("herdr-state.json"),serde_json::to_vec(&native).unwrap()).unwrap();
    wake_passes(&l,"8");
    let rejected=wake_history(&l);
    assert!(rejected["wake"]["attempts"].as_array().unwrap().iter().any(|a|a["outcome"]=="not-shown"),"{rejected}");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial);
    let mut native=l.state();native["toast_busy"]=json!(false);
    fs::write(l.home.path().join("herdr-state.json"),serde_json::to_vec(&native).unwrap()).unwrap();
    wake_passes(&l,"30");
    assert_eq!(wake_history(&l)["wake"]["claim"]["phase"],"confirmed");
    let before=wake_history(&l)["wake"]["attempts"].as_array().unwrap().len();
    wake_passes(&l,"5");
    assert_eq!(wake_history(&l)["wake"]["attempts"].as_array().unwrap().len(),before);
    let md=l.project.join("PROJECT.md");
    fs::write(&md,fs::read_to_string(&md).unwrap().replace("nudge = false","nudge = true")).unwrap();
    fs::write(l.home.path().join("coordinator-status"),"done").unwrap();
    wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+1,"enabling nudge prompts an already toasted unfinished result");
    l.ok(&["inbox","done","demo",&id]);
    assert_eq!(wake_history(&l)["waiting_items"],0);
    // A critical health notice uses the same idle wake and retained claim.
    let mut store=migration::open_active(&l.project).unwrap();
    let notice=herdr_farm::domain::InboxContent {id:"telemetry-health-1-1".into(),kind:"telemetry-health".into(),subject:"launch_stalled".into(),created:String::new(),summary:"telemetry health critical: launch_stalled; inspect launch progress, then relaunch / cancel-attempt".into(),body:String::new()};
    store.deliver_telemetry_notice(store.current_head().unwrap(),&notice,jiff::Timestamp::now().as_millisecond()).unwrap();
    drop(store);
    wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+2);
    assert_eq!(wake_history(&l)["wake"]["claim"]["phase"],"confirmed");
    wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+2);
}
#[test]
fn canonical_wake_lost_reply_fences_replay_until_done_and_releases_new_results() {
    let l=Lab::new();
    let _listener=UnixListener::bind(l.home.path().join("s")).expect("socket fixture: sandbox may deny Unix sockets");
    let md=l.project.join("PROJECT.md");
    fs::write(&md,fs::read_to_string(&md).unwrap().replace("nudge = false","nudge = true")).unwrap();
    l.ok(&["open","demo"]);l.stop();
    fs::write(l.home.path().join("coordinator-status"),"idle").unwrap();
    let initial=l.state()["prompts"].as_array().unwrap().len();
    let id=lose_worker(&l,"lost-reply");
    fs::write(l.home.path().join("lose-prompt-reply"),"").unwrap();
    wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+1);
    assert_eq!(wake_history(&l)["wake"]["claim"]["phase"],"uncertain");
    l.ok(&["context","demo"]);wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+1);
    lose_worker(&l,"new-result");wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+1,"new results cannot bypass the uncertain item's fence");
    l.ok(&["inbox","done","demo",&id]);
    fs::write(l.home.path().join("coordinator-status"),"done").unwrap();
    wake_passes(&l,"8");
    assert_eq!(l.state()["prompts"].as_array().unwrap().len(),initial+2);
    assert_eq!(wake_history(&l)["wake"]["claim"]["phase"],"confirmed");
}

/// Concurrent arrivals and a competing reader cannot consume unread notices.
#[test]
fn context_seen_retries_a_stale_head_without_consuming_concurrent_arrivals() {
    use herdr_farm::domain::InboxContent;
    let lab=Lab::new();
    let mut db=migration::open_active(&lab.project).unwrap();
    let notice=|id:&str|InboxContent {id:format!("memory-review-{id}"),kind:"memory-review".into(),subject:id.into(),created:String::new(),summary:format!("Review {id}"),body:String::new()};
    let first=notice("first");
    db.deliver_memory_review_reminder(db.current_head().unwrap(),&first,jiff::Timestamp::now().as_millisecond()).unwrap();
    let rendered=db.read_snapshot(None).unwrap();
    let second=notice("second");
    db.deliver_memory_review_reminder(db.current_head().unwrap(),&second,jiff::Timestamp::now().as_millisecond()).unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(30);
    assert_eq!(db.mark_context_seen(rendered.head,&[first.id.clone(),first.id.clone()],deadline).unwrap(),1);
    assert_eq!(db.mark_context_seen(rendered.head,std::slice::from_ref(&first.id),deadline).unwrap(),0);
    let rows=db.read_snapshot(None).unwrap();
    assert_eq!(rows.inbox.len(),2);
    assert!(rows.inbox.iter().find(|i|i.content.id==first.id).unwrap().seen);
    assert!(!rows.inbox.iter().find(|i|i.content.id==second.id).unwrap().seen);
    assert_eq!(rows.events.iter().filter(|e|e.kind=="inbox.seen"&&e.entity==first.id).count(),1);
    // The public per-turn command renders and marks the remaining arrival.
    lab.ok(&["context","demo"]);
    let head=db.current_head().unwrap();
    for _ in 0..6 {lab.ok(&["context","demo"]);assert_eq!(db.current_head().unwrap(),head);}
    assert!(db.read_snapshot(None).unwrap().inbox.iter().all(|i|i.seen&&!i.done));
    db.update_inbox(head,std::slice::from_ref(&first.id),true).unwrap();
    assert_eq!(db.mark_context_seen(head,std::slice::from_ref(&first.id),deadline).unwrap(),0);
    let final_rows=db.read_snapshot(None).unwrap().inbox;
    assert!(final_rows.iter().find(|i|i.content.id==first.id).unwrap().done);
    assert!(!final_rows.iter().find(|i|i.content.id==second.id).unwrap().done);
}
