#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
use herdr_farm::{agent_home, execution_guard::GatedSpawn, profile_config, worker_supervision::{Isolation, isolated_gated_command}};
use std::{fs, io::Write, os::unix::fs::PermissionsExt, path::Path, process::{Command, Stdio}};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn run(agent: &Path, cwd: &Path, home: &Path, isolation: &Isolation) {
    let argv = isolated_gated_command(agent, &[], 30, "release", home, isolation).unwrap();
    assert!(!argv.iter().any(|a| a.contains("fixture-secret-token")));
    let mut child = Command::new(&argv[0]).args(&argv[1..]).current_dir(cwd)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn_gated().unwrap();
    child.stdin.take().unwrap().write_all(b"release\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "sandbox probe: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn shared_git_thread_and_tab_are_contained_and_restarts_discard_worker_settings() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(base).unwrap();
    let lab = tempfile::tempdir_in(base).unwrap();
    let top = lab.path().canonicalize().unwrap();
    let owner = top.join("owner");
    for rel in [".ssh/key", ".claude/secret", ".codex/secret", "plain"] {
        fs::create_dir_all(owner.join(rel).parent().unwrap()).unwrap();
        fs::write(owner.join(rel), "secret").unwrap();
    }
    // One test owns this process's fixture environment; no real owner data is accessed.
    unsafe { std::env::set_var("HERDR_PROJECTS_OWNER_HOME", &owner); }
    fs::write(owner.join(".gitconfig"), "[user]\nname = Sandbox Owner\nemail = sandbox-owner@example.invalid\n").unwrap();
    for dir in [".cargo", ".rustup"] { fs::create_dir(owner.join(dir)).unwrap(); }
    let project = top.join("root/demo");
    fs::create_dir_all(project.join("threads")).unwrap();
    fs::create_dir_all(project.join("homes/other")).unwrap();
    fs::write(project.join("homes/other/transcript"), "private").unwrap();
    fs::write(project.join("threads/t-0001.toml"), "state").unwrap();
    fs::create_dir_all(top.join("root/other")).unwrap();
    fs::write(top.join("root/other/secret"), "other").unwrap();
    fs::write(top.join("root/.execution.lock"), "").unwrap();
    let config = owner.join("config.toml");
    fs::write(&config, "private config").unwrap();
    let repo = top.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("initial"), "initial").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "initial"]);
    git(&repo, &["config", "--unset", "user.name"]);
    git(&repo, &["config", "--unset", "user.email"]);
    git(&repo, &["gc"]);
    assert!(fs::read_dir(repo.join(".git/objects/pack")).unwrap().next().is_some());
    let wt = top.join("worktree");
    git(&repo, &["worktree", "add", "-b", "hp/demo/t-0001/work", wt.to_str().unwrap()]);
    let common = repo.join(".git");
    let admin = common.join("worktrees/worktree");
    let home = project.join("homes/t-0001");
    let agent = top.join("probe");
    fs::write(&agent, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
    let build = || Isolation::for_thread(&project, &home, &wt, &agent, Some((&wt, &admin, &common)), "hp/demo/t-0001", Some("hp/demo/t-0001/work"), Some(&config), None, &[]);
    assert!(Isolation::for_thread(&project, &home, &wt, &agent, Some((&wt, &admin, &common)), "hp/demo/t-0001", Some("hp/demo/old"), Some(&config), None, &[]).unwrap_err().to_string().contains("restart from a new thread"));
    let pointer = fs::read_to_string(wt.join(".git")).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", common.display())).unwrap();
    assert!(build().is_err());
    fs::write(wt.join(".git"), pointer).unwrap();
    let own_refs = common.join("refs/heads/hp/demo/t-0001");
    let saved_refs = common.join("saved-refs");
    fs::rename(&own_refs, &saved_refs).unwrap();
    std::os::unix::fs::symlink(&saved_refs, &own_refs).unwrap();
    assert!(build().unwrap_err().to_string().contains("symlink"));
    assert!(!home.exists(), "ref refusal must precede owner-side effects");
    fs::remove_file(&own_refs).unwrap();
    fs::rename(&saved_refs, &own_refs).unwrap();
    let initial_config = fs::read(common.join("config")).unwrap();
    git(&repo, &["config", "extensions.refStorage", "reftable"]);
    assert!(build().is_err());
    fs::write(common.join("config"), initial_config).unwrap();
    git(&repo, &["config", "extensions.worktreeConfig", "true"]);
    let parsed = profile_config::parse_isolation(&toml::Value::Table(Default::default())).unwrap();
    assert!(profile_config::claude_token_file(&parsed).unwrap_err().to_string().contains("claude_token_file"));
    let token = top.join("token");
    fs::write(&token, "fixture-secret-token\n").unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    let unauthed = build().unwrap();
    assert_eq!(fs::metadata(&home).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(admin.join("config.worktree").is_file());
    assert!(isolated_gated_command(&agent, &[], 30, "release", &home, &unauthed).unwrap_err().to_string().contains("claude_token_file"));
    let parsed = profile_config::parse_isolation(&toml::from_str(&format!("[worker_isolation.login]\nclaude_token_file = {:?}\n", token)).unwrap()).unwrap();
    let isolation = unauthed.with_thread_login(&parsed).unwrap().with_thread_env(&["RUST_BACKTRACE=1".into()]).unwrap();
    assert!(isolated_gated_command(&agent, &["--settings".into(), "planted.json".into()], 30, "release", &home, &isolation).unwrap_err().to_string().contains("--settings"));
    let writable = [&wt, &common.join("objects"), &common.join("refs/heads/hp/demo/t-0001"), &common.join("logs/refs/heads/hp/demo/t-0001"), &admin, &home].map(|p| p.to_string_lossy().into_owned());
    agent_home::prepare_claude_thread(&home, &wt, &writable, false).unwrap();
    fs::write(home.join(".claude/settings.json"), r#"{"permissions":{"allow":["Bash", "WebFetch", "Read(/planted/**)"]},"hooks":{"PreToolUse":[]}}"#).unwrap();
    agent_home::prepare_claude_thread(&home, &wt, &writable, false).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
    assert!(settings.get("hooks").is_none());
    assert_eq!(settings["permissions"]["allow"], serde_json::json!(["Bash", "Read", "Edit", "Write", "Glob", "Grep"]));
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], true);
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    assert_eq!(settings["sandbox"]["filesystem"]["allowWrite"], serde_json::json!(writable));
    assert_eq!(settings["permissions"]["deny"], serde_json::json!(["WebFetch", "WebSearch"]));
    assert_eq!(settings["sandbox"]["network"]["allowedDomains"], serde_json::json!([]));
    let trust: serde_json::Value = serde_json::from_str(&fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(trust["projects"][wt.to_str().unwrap()]["hasTrustDialogAccepted"], true);
    agent_home::prepare_claude_thread(&home, &wt, &writable, true).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
    assert!(settings["sandbox"].get("network").is_none());
    assert!(settings["permissions"].get("deny").is_none());
    agent_home::prepare_claude_thread(&home, &wt, &writable, false).unwrap();
    let mut script = String::from("#!/bin/sh\nset -eu\n[ \"$RUST_BACKTRACE\" = 1 ]\n[ \"$CLAUDE_CODE_OAUTH_TOKEN\" = fixture-secret-token ]\n[ ! -e /proc/self/fd/9 ]\nprintf changed > edit\ngit add edit\ngit commit -m sandbox-commit\n");
    script.push_str(&format!("[ \"$CARGO_HOME\" = '{}' ]\n[ \"$RUSTUP_HOME\" = '{}' ]\n[ \"$XDG_CACHE_HOME\" = \"$HOME/.cache\" ]\n", owner.join(".cargo").display(), owner.join(".rustup").display()));
    for path in [
        common.join("objects/pack/attack"),
        common.join("config"), common.join("hooks/pre-commit"), common.join("info/exclude"),
        common.join("packed-refs"), common.join("refs/heads/main"),
        common.join("refs/heads/hp/demo/other/work"), wt.join(".git"),
        admin.join("commondir"), admin.join("gitdir"), admin.join("config.worktree"), common.join("objects/info/alternates"),
        project.join("homes/other/transcript"), project.join("threads/t-0001.toml"), owner.join("plain"),
    ] {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
        script.push_str(&format!("if (printf attack > '{}') 2>/dev/null; then echo 'write escaped: {}' >&2; exit 1; fi\n", path.display(), path.display()));
    }
    for path in [owner.join(".ssh/key"), owner.join(".claude/secret"), owner.join(".codex/secret"), config.clone(), top.join("root/other/secret"), project.join("homes/other/transcript"), token.clone()] {
        script.push_str(&format!("if [ -n \"$(cat '{}' 2>/dev/null || true)\" ]; then echo 'read escaped: {}' >&2; exit 1; fi\n", path.display(), path.display()));
    }
    script.push_str("printf success > \"$HOME/report\"\n");
    fs::write(&agent, script).unwrap();
    run(&agent, &wt, &home, &isolation);
    assert_eq!(fs::read_to_string(wt.join("edit")).unwrap(), "changed");
    assert_eq!(git(&repo, &["show", "hp/demo/t-0001/work:edit"]), "changed");
    assert_eq!(fs::read_to_string(home.join("report")).unwrap(), "success");
    assert_eq!(git(&wt, &["log", "-1", "--format=%an <%ae>"]), "Sandbox Owner <sandbox-owner@example.invalid>");
    let overrides = isolation.clone().with_thread_env(&["CARGO_HOME=/tmp/cargo-override".into(), "RUSTUP_HOME=/tmp/rustup-override".into(), "XDG_CACHE_HOME=/tmp/cache-override".into()]).unwrap();
    fs::write(&agent, "#!/bin/sh\nset -eu\n[ \"$CARGO_HOME\" = /tmp/cargo-override ]\n[ \"$RUSTUP_HOME\" = /tmp/rustup-override ]\n[ \"$XDG_CACHE_HOME\" = /tmp/cache-override ]\n").unwrap();
    run(&agent, &wt, &home, &overrides);
    assert!(!fs::read_dir(&home).unwrap().any(|e| e.unwrap().file_name() == "token"));
    let tab = project.join("threads/t-0002");
    fs::create_dir(&tab).unwrap();
    let tab_home = project.join("homes/t-0002");
    let isolation = Isolation::for_thread(&project, &tab_home, &tab, &agent, None, "hp/demo/t-0002", None, Some(&config), None, &[]).unwrap().with_login_token_file(&token).unwrap();
    fs::write(&agent, format!("#!/bin/sh\nset -eu\nprintf tab > edit\nif (printf bad > '{}') 2>/dev/null; then exit 1; fi\n", project.join("threads/t-0001.toml").display())).unwrap();
    run(&agent, &tab, &tab_home, &isolation);
    assert_eq!(fs::read_to_string(tab.join("edit")).unwrap(), "tab");
    let redirected_tab = project.join("threads/t-0003");
    std::os::unix::fs::symlink(&project, &redirected_tab).unwrap();
    assert!(Isolation::for_thread(&project, &project.join("homes/t-0003"), &redirected_tab, &agent, None, "hp/demo/t-0003", None, Some(&config), None, &[]).unwrap_err().to_string().contains("symlink"));
    assert!(!project.join("homes/t-0003").exists());
}

#[test]
fn owner_safety_settings_are_reported_and_validated_through_cli() {
    let lab = tempfile::tempdir().unwrap();
    let root = lab.path().join("root");
    let config_dir = lab.path().join(".config/herdr-farm");
    fs::create_dir_all(&config_dir).unwrap();
    let cli = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_herdr-farm"))
        .env_clear().env("HOME", lab.path()).env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .arg("--root").arg(&root).args(args).output().unwrap();
    let out = cli(&["new", "--legacy", "demo"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = format!("[safety.\"{}\"]\nthread_sandbox = false\nthread_wall_hours = 12\nthread_env = [\"RUST_BACKTRACE=1\"]\n", root.join("demo").display());
    fs::write(config_dir.join("config.toml"), &text).unwrap();
    let out = cli(&["safety", "show", "demo"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let output = String::from_utf8(out.stdout).unwrap();
    assert!(output.contains("thread_sandbox = false"), "{output}");
    assert!(output.contains("Claude threads are unsandboxed"), "{output}");
    assert!(output.contains("thread_wall_hours = 12"), "{output}");
    assert!(output.contains("RUST_BACKTRACE=1"), "{output}");
    let sandboxed = format!("[safety.\"{}\"]\nthread_sandbox = true\nthread_allowed_commands = [\"tool test:*\"]\nthread_agent_args_kind = \"claude\"\nthread_agent_args = [\"--model\", \"sonnet\"]\n", root.join("demo").display());
    fs::write(config_dir.join("config.toml"), &sandboxed).unwrap();
    let out = cli(&["safety", "show", "demo"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let output = String::from_utf8(out.stdout).unwrap();
    let arguments = output.lines().find(|line| line.contains("sandboxed Claude worker arguments:")).unwrap();
    assert_eq!(arguments.trim(), r#"sandboxed Claude worker arguments: ["--model", "sonnet", "--setting-sources", "user"]"#);
    fs::write(config_dir.join("config.toml"), sandboxed.replace("sonnet", "--settings")).unwrap();
    let out = cli(&["safety", "show", "demo"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8(out.stdout).unwrap().contains("sandboxed Claude worker arguments: refused: sandboxed Claude thread refuses --settings"));
    for invalid in ["thread_wall_hours = 0", "thread_wall_hours = 169", "thread_env = [\"GIT_AUTHOR_NAME=bad\"]", "thread_env = [\"LD_PRELOAD=bad\"]"] {
        fs::write(config_dir.join("config.toml"), format!("[safety.\"{}\"]\n{invalid}\n", root.join("demo").display())).unwrap();
        assert!(!cli(&["safety", "show", "demo"]).status.success());
    }
}
