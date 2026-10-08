#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)] // Test-only spawns outside the library may skip the spawn gate.
//! Worker profile preparation and native verification through the compiled
//! CLI: `profile prepare`, `profile verify-native` and `profile retained`
//! over a disposable migrated project. herdr and the agent are shell
//! fixtures that only answer `--version` and log every invocation.
use herdr_farm::{authority, domain::*, migration, runtime};
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::{Command, Output}};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");

struct Lab { home: tempfile::TempDir, project: PathBuf, config: PathBuf }

impl Lab {
    /// A paused migrated project whose `worker` profile passes `extra_args`.
    fn new(extra_args: &str) -> Self { Self::with_budget(extra_args, "[profiles.worker.budget]\nmax_wall_seconds=60\nunknown_usage='allow_with_warning'\n") }
    /// As `new`, with `budget` as the rest of the profile definition.
    fn with_budget(extra_args: &str, budget: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, format!("[authority]\nversion=1\nrevision=1\napproval_public_key='ssh-ed25519 {}'\n[profiles.worker]\nkind='claude'\n\
permission_policy='interactive'\nextra_args={extra_args}\n{budget}", "A".repeat(48))).unwrap();
        let lab = Lab { project: home.path().join("root/demo"), config, home };
        lab.ok(&["new", "--legacy", "demo"]);
        lab.ok(&["pause", "demo"]);
        migration::apply(&lab.project, &migration::inspect_with_config(&lab.project, &lab.config).unwrap(), true).unwrap();
        fs::create_dir(lab.path("agent-home")).unwrap();
        for (name, version) in [("herdr", "herdr 0.9.1"), ("claude", "2.1.0-preview.1 (Claude Code)")] {
            let log = lab.path(&format!("{name}-calls"));
            fs::write(lab.path(name), format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n\
[ \"$#\" = 1 ] && [ \"$1\" = --version ] && [ -z \"$HOME\" ] && [ \"$PATH\" = /usr/bin:/bin ] || exit 3\nprintf '%s\\n' '{version}'\n", log.display())).unwrap();
            fs::set_permissions(lab.path(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        lab
    }
    fn path(&self, name: &str) -> PathBuf { self.home.path().join(name) }
    fn cli_command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command.env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", self.home.path()).env("PATH", "/usr/bin:/bin")
            .args(["--root", self.path("root").to_str().unwrap()]).args(args);
        command
    }
    fn cli(&self, args: &[&str]) -> Output { self.cli_command(args).output().unwrap() }
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.cli(args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }
    /// `profile <verb> demo worker` over the fixture executables.
    fn profile_command(&self, verb: &str, extra: &[&str]) -> Command {
        let (herdr, agent, home) = (self.path("herdr"), self.path("claude"), self.path("agent-home"));
        let mut args = vec!["profile", verb, "demo", "worker", "--herdr-executable", herdr.to_str().unwrap(),
            "--agent-executable", agent.to_str().unwrap(), "--execution-home", home.to_str().unwrap()];
        args.extend(extra);
        self.cli_command(&args)
    }
    fn profile(&self, verb: &str, extra: &[&str]) -> Output { self.profile_command(verb, extra).output().unwrap() }
    /// A refused `profile <verb>` that changed no project state; returns its stderr.
    fn refused(&self, verb: &str, extra: &[&str]) -> String {
        let before = runtime::snapshot(&self.project).unwrap();
        let out = self.profile(verb, extra);
        assert!(!out.status.success(), "{verb} accepted: {}", String::from_utf8_lossy(&out.stdout));
        let mut after = runtime::snapshot(&self.project).unwrap();
        // Probe failures append an audit event but do not change project state.
        if verb != "prepare" {
            assert_eq!(after.head, before.head + 1);
            assert_eq!(after.events.len(), before.events.len() + 1);
            let failure = after.events.pop().unwrap();
            assert_eq!(failure.kind, "profile.native_failed");
            assert_eq!(failure.entity, "worker");
            assert_eq!(failure.sequence, before.head + 1);
            after.head = before.head;
        }
        assert_eq!(after, before, "{verb} refusal changed project state");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
    fn calls(&self, name: &str) -> Vec<String> { fs::read_to_string(self.path(&format!("{name}-calls"))).unwrap_or_default().lines().map(str::to_owned).collect() }
}

/// Replaces `production_preparation_binds_inputs_without_fabricating_capabilities_or_authority`.
#[test]
fn prepare_freezes_the_installation_without_capabilities_authority_or_secrets() {
    let lab = Lab::new("['PRIVATE_ARG']");
    let before = runtime::snapshot(&lab.project).unwrap();
    let out = lab.profile("prepare", &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("PRIVATE_ARG"), "{text}");
    let prepared: Value = serde_json::from_str(&text).unwrap();
    let profile: FrozenProfile = serde_json::from_value(prepared["profile"].clone()).unwrap();
    assert_eq!(prepared["reference"], serde_json::to_value(profile.reference().unwrap()).unwrap());
    assert_eq!((profile.name.as_str(), profile.kind.as_str()), ("worker", "claude"));
    assert_eq!((profile.agent.version.as_str(), profile.herdr.version.as_str()), ("2.1.0-preview.1", "0.9.1"));
    assert_eq!(profile.execution_home.as_deref(), lab.path("agent-home").to_str());
    assert_eq!(profile.permission_policy, authority::policy_reference(&lab.project).unwrap());
    // Nothing is fabricated: no capability, launchability or certificate.
    let c = &profile.capabilities;
    for evidence in [&c.launch, &c.readiness_observation, &c.prompt_submission, &c.stop, &c.checkpoint_acknowledgment, &c.structured_usage, &c.resume] {
        assert_eq!(evidence, &CapabilityEvidence::Unknown);
    }
    assert!(profile.workflow_certificate.is_none() && profile.validate_for_launch().is_err());
    assert_eq!((&prepared["launchable"], &prepared["protocol_capable"], &prepared["certified"]), (&Value::Bool(false), &Value::Bool(false), &Value::Bool(false)));
    // Preparation writes nothing, retains nothing, and repeats to the same reference.
    assert_eq!(runtime::snapshot(&lab.project).unwrap(), before);
    let digest = prepared["reference"]["digest"].as_str().unwrap();
    let retained = lab.cli(&["profile", "retained", "demo", digest]);
    assert!(!retained.status.success() && String::from_utf8_lossy(&retained.stderr).contains("retained native profile not found"));
    let again: Value = serde_json::from_slice(&lab.profile("prepare", &[]).stdout).unwrap();
    assert_eq!(again["reference"], prepared["reference"]);
    // The executables were only asked for their versions.
    assert!(lab.calls("claude").iter().chain(&lab.calls("herdr")).all(|c| c == "--version"));
}

/// Replaces `native_verification_refuses_unmapped_startup_arguments_before_launch`.
#[test]
fn verify_native_refuses_unmapped_startup_arguments_before_launching_anything() {
    let lab = Lab::new("['PRIVATE_ARG']");
    let error = lab.refused("verify-native", &["--retain"]);
    assert!(error.contains("empty extra_args"), "{error}");
    assert!(!error.contains("PRIVATE_ARG"), "{error}");
    assert!(lab.calls("claude").iter().all(|c| c == "--version"), "{:?}", lab.calls("claude"));
    assert!(lab.calls("herdr").iter().all(|c| c == "--version"), "{:?}", lab.calls("herdr"));
}

/// Replaces `native_verification_does_not_promote_version_only_helpers`.
#[test]
fn verify_native_does_not_retain_a_helper_that_only_answers_its_version() {
    let lab = Lab::new("[]");
    let digest = lab.ok(&["profile", "prepare", "demo", "worker", "--herdr-executable", lab.path("herdr").to_str().unwrap(),
        "--agent-executable", lab.path("claude").to_str().unwrap(), "--execution-home", lab.path("agent-home").to_str().unwrap()])["reference"]["digest"]
        .as_str().unwrap().to_owned();
    let error = lab.refused("verify-native", &["--retain"]);
    // The helper cannot serve the disposable probe session, so nothing is observed.
    assert!(error.contains("native probe server exited"), "{error}");
    let inspected = lab.ok(&["profile", "inspect", "worker", "--project", "demo"]);
    let failures = inspected["probe_failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["agent_version"], "2.1.0-preview.1");
    assert_eq!(failures[0]["failure_class"], "verification_failed");
    assert_eq!(failures[0]["step"], "native_verification");
    assert!(failures[0]["finished_unix_ms"].as_i64().unwrap() >= failures[0]["started_unix_ms"].as_i64().unwrap());
    assert_eq!(herdr_farm::store::SqliteStore::open(&lab.project.join(".state/state.db")).unwrap()
        .native_probe_failures("worker").unwrap(), *failures);
    assert_eq!(lab.calls("herdr").last().map(String::as_str), Some("server"));
    let retained = lab.cli(&["profile", "retained", "demo", &digest]);
    assert!(!retained.status.success() && String::from_utf8_lossy(&retained.stderr).contains("retained native profile not found"));
    assert!(lab.calls("claude").iter().all(|c| c == "--version"), "{:?}", lab.calls("claude"));
}

/// Replaces `preparation_checks_whole_brief_and_supported_budget_policy`.
///
/// A worker profile is prepared only with a wall deadline of one second to
/// one week and positive token limits whose character estimate fits; the
/// refusal writes nothing. (A brief over the input budget and a profile that
/// blocks on unknown usage are refused in tests/canonical_worker.rs.)
#[test]
fn prepare_refuses_worker_budgets_outside_the_supported_bounds() {
    let budget = |fields: &str| format!("[profiles.worker.budget]\n{fields}unknown_usage='allow_with_warning'\n");
    for (definition, error) in [
        (String::new(), "worker profile requires a wall deadline"),
        (budget(""), "worker profile requires a wall deadline"),
        (budget("max_wall_seconds=0\n"), "profile budget limits must be positive bounded integers"),
        (budget("max_wall_seconds=604801\n"), "worker wall deadline exceeds supported bounds"),
        (budget(&format!("max_wall_seconds=60\nsoft_input_tokens={}\n", i64::MAX)), "profile token budget exceeds character conversion bounds"),
        (budget("max_wall_seconds=60\nsoft_output_tokens=0\n"), "profile budget limits must be positive bounded integers"),
    ] {
        let lab = Lab::with_budget("[]", &definition);
        let refused = lab.refused("prepare", &[]);
        assert!(refused.contains(error), "{definition}: {refused}");
    }
    let lab = Lab::with_budget("[]", &budget("max_wall_seconds=604800\nsoft_input_tokens=100\n"));
    let out = lab.profile("prepare", &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

impl Lab {
    /// A migrated project whose `worker` profile has `kind` and `fields` (extra TOML
    /// lines such as pins) and whose agent fixture answers `version` (Codex is
    /// probed with an execution-home `HOME`, Claude without one).
    fn pinned(kind: &str, fields: &str, version: &str) -> Self {
        let lab = Lab::with_budget("[]", &format!("{fields}\n[profiles.worker.budget]\nmax_wall_seconds=60\nunknown_usage='allow_with_warning'\n"));
        let config = fs::read_to_string(&lab.config).unwrap();
        // The kind is part of the pinned configuration: rebuild the project over it.
        drop(lab);
        let home = tempfile::tempdir().unwrap();
        let config_path = home.path().join(".config/herdr-farm/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, config.replace("kind='claude'", &format!("kind='{kind}'"))).unwrap();
        let lab = Lab { project: home.path().join("root/demo"), config: config_path, home };
        lab.ok(&["new", "--legacy", "demo"]);
        lab.ok(&["pause", "demo"]);
        migration::apply(&lab.project, &migration::inspect_with_config(&lab.project, &lab.config).unwrap(), true).unwrap();
        fs::create_dir(lab.path("agent-home")).unwrap();
        fs::write(lab.path("herdr"), "#!/bin/sh\n[ \"$1\" = --version ] && echo 'herdr 0.9.1' && exit 0\nexit 3\n").unwrap();
        fs::write(lab.path("claude"), format!("#!/bin/sh\n[ \"$1\" = --version ] && echo '{version}' && exit 0\nexit 3\n")).unwrap();
        for name in ["herdr", "claude"] { fs::set_permissions(lab.path(name), fs::Permissions::from_mode(0o700)).unwrap(); }
        lab
    }
}

/// Model and reasoning effort are validated profile fields for Codex and Claude
/// (no passthrough arguments): preparation freezes them into the profile
/// identity and `profile inspect` shows what will be pinned.
#[test]
fn model_and_effort_pins_are_profile_fields_that_inspect_shows_and_identity_binds() {
    for (kind, model, version) in [("codex", "gpt-6.1-sol", "codex-cli 0.154.0"), ("claude", "claude-sonnet-5-5", "2.1.0 (Claude Code)")] {
        let pinned = Lab::pinned(kind, &format!("model='{model}'\nreasoning_effort='low'"), version);
        let inspected = pinned.ok(&["profile", "inspect", "worker"]);
        assert_eq!((inspected["kind"].as_str(), inspected["pinned_model"].as_str(), inspected["pinned_reasoning_effort"].as_str()), (Some(kind), Some(model), Some("low")), "{inspected}");
        assert_eq!(inspected["extra_argument_count"], 0);
        assert!(!inspected["blockers"].to_string().contains("model request") && !inspected["blockers"].to_string().contains("reasoning effort"), "{inspected}");
        let prepared = pinned.profile("prepare", &[]);
        assert!(prepared.status.success(), "{kind}: {}", String::from_utf8_lossy(&prepared.stderr));
        let prepared: Value = serde_json::from_slice(&prepared.stdout).unwrap();
        let profile: FrozenProfile = serde_json::from_value(prepared["profile"].clone()).unwrap();
        assert_eq!(profile.kind, kind);
        // No passthrough argument carries the pin; the definition identity does.
        assert_eq!(profile.arguments_digest, "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945", "empty extra_args");
        let plain = Lab::pinned(kind, "", version);
        let other: Value = serde_json::from_slice(&plain.profile("prepare", &[]).stdout).unwrap();
        assert_ne!(other["profile"]["definition_digest"], prepared["profile"]["definition_digest"], "the pins are part of the profile identity");
        // Pins write nothing by themselves: the home is prepared by verification and launch.
        assert!(fs::read_dir(pinned.path("agent-home")).unwrap().next().is_none());
    }
}

/// Anything that is not a plain lowercase model or effort name is refused
/// before any work, without echoing the value.
#[test]
fn invalid_pins_are_refused_without_echoing_them() {
    for fields in ["model='Private Model!'", "reasoning_effort='PRIVATE'", "model='gpt-6.1-sol'\nreasoning_effort=''"] {
        let lab = Lab::pinned("codex", fields, "codex-cli 0.154.0");
        let before = runtime::snapshot(&lab.project).unwrap();
        let refused = lab.profile("prepare", &[]);
        assert!(!refused.status.success(), "{fields}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(!stderr.contains("Private") && !stderr.contains("PRIVATE"), "{stderr}");
        assert_eq!(runtime::snapshot(&lab.project).unwrap(), before);
    }
}

/// Real public native probe: delayed detector evidence, live argv title races,
/// and sanitized persisted failures inspected in a separate CLI process.
fn readiness_scenario(mode: &str, delay: u64, wall: u64, failure_class: Option<&str>) {
    let lab = Lab::new("[]");
    let config = fs::read_to_string(&lab.config).unwrap().replace("kind='claude'", "kind='codex'")
        .replace("max_wall_seconds=60", &format!("max_wall_seconds={wall}"));
    let config = format!("{config}\n[worker_isolation]\nshare_login=false\n");
    fs::write(&lab.config, config).unwrap();
    fs::write(lab.path("agent-home/mode"), mode).unwrap();
    fs::write(lab.path("delay"), delay.to_string()).unwrap();
    fs::write(lab.path("setup-delay"), if mode == "w" { "true" } else { "false" }).unwrap();
    fs::write(lab.path("server.py"), include_str!("fixtures/probe_readiness_server.py")).unwrap();
    fs::write(lab.path("herdr"), format!("#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n", lab.path("server.py").display())).unwrap();
    let compiled = Command::new("/usr/bin/cc").args(["-O0", "-o"]).arg(lab.path("claude"))
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/probe_readiness_agent.c")).output().unwrap();
    assert!(compiled.status.success(), "{}", String::from_utf8_lossy(&compiled.stderr));
    let result = lab.profile("verify-interaction", &[]);
    if let Ok(diagnostic) = fs::read_to_string(lab.path("socket-error")) {
        panic!("{diagnostic}");
    }
    match failure_class {
        None => {
            assert!(result.status.success(), "mode {mode}, delay {delay}: {}", String::from_utf8_lossy(&result.stderr));
            let proof: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(proof["preparation"]["launchable"], true);
            if mode == "w" {
                assert!(lab.path("agent-home/.hp-verify-work/setup-pending").exists(),
                    "setup wrapper must have run");
            }
            assert!(lab.ok(&["profile", "inspect", "worker", "--project", "demo"])["probe_failures"].as_array().unwrap().is_empty());
        }
        Some(failure_class) => {
            assert!(!result.status.success(), "unready or changed agent must fail");
            let text = String::from_utf8_lossy(&result.stderr);
            if failure_class == "transient_readiness" {
                assert!(text.contains("worker lacks verified visible prompt readiness"), "{text}");
                assert!(text.contains("trust") && text.contains("network"), "{text}");
            } else { assert!(text.contains("process_changed"), "{text}"); }
            assert!(!text.contains("PRIVATE_SCREEN_TOKEN"), "{text}");
            let inspected = lab.ok(&["profile", "inspect", "worker", "--project", "demo"]);
            let record = &inspected["probe_failures"][0];
            assert_eq!(record["failure_class"], failure_class);
            assert_eq!(record["step"], "prompt_readiness");
            assert_eq!(record["agent_version"], "0.154.0");
            if failure_class == "transient_readiness" {
                assert!(record["reason"].as_str().unwrap().contains("trust"));
            }
            assert!(!inspected.to_string().contains("PRIVATE_SCREEN_TOKEN"));
            let telemetry = lab.ok(&["telemetry", "demo", "collectors", "capabilities", "--json"]);
            let codex = telemetry["adapters"].as_array().unwrap().iter()
                .find(|adapter| adapter["adapter"] == "codex").unwrap();
            assert_eq!(codex["probe_failures"][0], *record);
        }
    }
}

#[test]
fn native_readiness_waits_for_setup_wrapper_exec() { readiness_scenario("w", 0, 120, None); }
#[test]
fn native_readiness_retries_empty_process_title() { readiness_scenario("e", 4, 120, None); }
#[test]
fn native_readiness_retries_non_terminated_process_title() { readiness_scenario("n", 4, 120, None); }
#[test]
fn native_readiness_accepts_cold_start_beyond_thirty_seconds() { readiness_scenario("0", 32, 120, None); }
#[test]
fn native_readiness_timeout_retains_categories_visible_in_cli() { readiness_scenario("0", 999, 60, Some("transient_readiness")); }

#[test]
fn native_readiness_rejects_changed_arguments() { readiness_scenario("a", 999, 120, Some("process_changed")); }
#[test]
fn native_readiness_rejects_changed_executable() { readiness_scenario("x", 999, 120, Some("process_changed")); }

/// An isolated, unwritable runtime directory fails before native process creation;
/// the public CLI must still retain a bounded, redacted setup observation.
#[test]
fn native_probe_setup_failure_is_retained_without_starting_executables() {
    let lab = Lab::new("[]");
    let runtime = tempfile::Builder::new().prefix("hp-probe-").tempdir_in("/tmp").unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = lab.profile_command("verify-native", &[]).env("XDG_RUNTIME_DIR", runtime.path()).output().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot create"));
    let inspection = lab.ok(&["profile", "inspect", "worker", "--project", "demo"]);
    let record = &inspection["probe_failures"][0];
    assert_eq!(record["step"], "probe_setup");
    assert_eq!(record["failure_class"], "verification_failed");
    assert!(record["kind"].is_null() && record["agent_version"].is_null());
    assert!(record.to_string().len() <= 4096);
    assert!(lab.calls("herdr").is_empty() && lab.calls("claude").is_empty());
}
