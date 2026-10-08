#![allow(clippy::disallowed_methods)] // Test-only spawns outside the library may skip the spawn gate.
//! Project creation, discovery, PROJECT.md settings and per-project safety
//! overrides through the compiled CLI: `new`, `list`, `context` and
//! `safety show`, asserting what they print and the files they leave.
use std::{fs, path::PathBuf, process::{Command, Output}};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");

struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Self { Home(tempfile::tempdir().unwrap()) }
    fn root(&self) -> PathBuf { self.0.path().join("root") }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(BIN).env_clear().env("HERDR_FARM_TEST_TIME_SCALE", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/time-scale.txt")).trim()).env("HOME", self.0.path()).env("PATH", "/usr/bin:/bin")
            .args(["--root", self.root().to_str().unwrap()]).args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let out = self.cli(args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }
    fn refused(&self, args: &[&str]) -> String {
        let out = self.cli(args);
        assert!(!out.status.success(), "{args:?} was accepted: {}", String::from_utf8_lossy(&out.stdout));
        String::from_utf8(out.stderr).unwrap()
    }
    fn listing(&self) -> Vec<String> {
        self.ok(&["list"]).lines().map(|l| l.split('\t').next().unwrap().to_string()).collect()
    }
    fn tree(&self) -> Vec<PathBuf> {
        let mut all = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                if entry.path().is_dir() { stack.push(entry.path()); }
                all.push(entry.path());
            }
        }
        all.sort();
        all
    }
}

/// `new --legacy` derives the slug from the name, writes the whole skeleton with
/// PROJECT.md settings, keeps `@` inside a path, and refuses a second project
/// of the same slug and names that would escape the root.
#[test]
fn new_writes_the_skeleton_and_refuses_duplicates_and_escapes() {
    let home = Home::new();
    // Nothing exists yet: an absent root lists no projects.
    assert_eq!(home.ok(&["list"]), "");
    let out = home.ok(&["new", "--legacy", "My Demo  Project!", "--goal", "Ship \"it\"", "--repo", "/srv/app@box", "--repo", "/a@b/c", "--repo", "/no/such/repo"]);
    let dir = home.root().join("my-demo-project");
    assert!(out.starts_with(&format!("created `my-demo-project` at {}\n", dir.display())), "{out}");
    for sub in ["memory", "scratch", "routines", "threads", "inbox/done", "library", ".state"] {
        assert!(dir.join(sub).is_dir(), "{sub}");
    }
    assert!(fs::read_to_string(dir.join("MEMORY.md")).unwrap().starts_with("# Memory\n"));
    assert!(dir.join("TASKS.md").is_file() && dir.join(".state/project.json").is_file());
    let md = fs::read_to_string(dir.join("PROJECT.md")).unwrap();
    let (front, body) = md.strip_prefix("+++\n").unwrap().split_once("+++\n").unwrap();
    let settings: toml::Value = toml::from_str(front).unwrap();
    let expected: toml::Value = toml::from_str(r#"
        name = "My Demo  Project!"
        goal = 'Ship "it"'
        coordinator_agent = "claude"
        thread_agent = "claude"
        max_parallel_threads = 3
        auto_resolve_days = 7
        nudge = false
        repos = [{ path = "/srv/app", machine = "box" }, { path = "/a@b/c" }, { path = "/no/such/repo" }]
    "#).unwrap();
    assert_eq!(settings, expected);
    assert!(body.trim_start().starts_with("# Instructions"));
    let context = home.ok(&["context", "my-demo-project"]);
    for line in ["Project: my-demo-project (active)", "Goal: Ship \"it\"", "Repo: /srv/app (machine box)", "Repo: /a@b/c\n", "Repo: /no/such/repo"] {
        assert!(context.contains(line), "missing {line:?} in {context}");
    }

    // Folding to the same slug is a duplicate; the original is untouched.
    let before = home.tree();
    assert!(home.refused(&["new", "--legacy", "my-demo-project"]).contains("`my-demo-project` already exists"));
    assert_eq!(fs::read_to_string(dir.join("PROJECT.md")).unwrap(), md);
    for (name, error) in [("../x", "may not contain"), ("a/b", "may not contain"), ("a\\b", "may not contain"), ("..", "may not contain"),
                          ("!!!", "no letters or digits"), ("", "no letters or digits")] {
        assert!(home.refused(&["new", "--legacy", name]).contains(error), "{name:?}");
    }
    assert_eq!(home.tree(), before, "a refused name created something");

    // Non-ASCII letters drop out and long names are cut to 40 characters.
    assert!(home.ok(&["new", "--legacy", "  Ünï 42 "]).starts_with("created `n-42`"));
    let long = home.ok(&["new", "--legacy", &"x".repeat(60)]);
    assert!(long.starts_with(&format!("created `{}`", "x".repeat(40))), "{long}");
    assert_eq!(home.listing(), ["my-demo-project", "n-42", &"x".repeat(40)]);

    // The default uses canonical storage; explicit legacy creation keeps the skeleton above.
    home.ok(&["new", "canonical-demo"]);
    assert!(home.root().join("canonical-demo/.state/state.db").is_file());
    assert!(!dir.join(".state/state.db").exists());
    assert!(home.ok(&["context", "canonical-demo"]).contains("Runtime owner: SQLite;"));
    let runtime: serde_json::Value = serde_json::from_str(&home.ok(&["runtime", "canonical-demo", "inspect"])).unwrap();
    assert_eq!(runtime["control"]["state"], "paused");

}

/// Only valid slugs are projects: `list` skips dot folders, folders without
/// PROJECT.md and invalid names, and every command refuses an invalid slug.
#[test]
fn list_and_commands_accept_only_folders_with_project_md_and_valid_slugs() {
    let home = Home::new();
    home.ok(&["new", "--legacy", "b"]);
    for name in ["a", "0x", "demo-2", ".trash", "Not_A_Slug", "empty", &"a".repeat(41)] {
        fs::create_dir_all(home.root().join(name)).unwrap();
        if name != "empty" { fs::write(home.root().join(name).join("PROJECT.md"), "").unwrap(); }
    }
    assert_eq!(home.listing(), ["0x", "a", "b", "demo-2"]);
    // Forty characters is a well-formed slug that is looked up; forty-one is refused.
    assert!(home.refused(&["safety", "show", &"a".repeat(40)]).contains(&format!("no project `{}`", "a".repeat(40))));
    for bad in ["", "-a", "A", "a_b", "a/b", "../x", "a b", ".", "..", &"a".repeat(41)] {
        assert!(home.refused(&["safety", "show", "--", bad]).contains("is not a valid slug"), "{bad:?}");
    }
    assert!(home.refused(&["safety", "show", "zz"]).contains("no project `zz`"));
    assert!(home.ok(&["safety", "show", "b"]).starts_with("Effective safety settings for `b`:"));
}

/// Settings come from the TOML between the `+++` lines; the body may itself
/// contain `+++`. Malformed front matter is reported rather than guessed at.
#[test]
fn context_reads_front_matter_and_reports_malformed_project_md() {
    let home = Home::new();
    home.ok(&["new", "--legacy", "demo"]);
    let md = home.root().join("demo/PROJECT.md");
    fs::write(&md, "+++\nname = \"X\"\nnudge = true\n+++\n\nBody\n+++\nmore\n").unwrap();
    let context = home.ok(&["context", "demo"]);
    assert!(context.contains("Name: X\n"), "{context}");
    assert!(context.contains("Settings: thread_agent=claude max_parallel_threads=3 auto_resolve_days=7 nudge=true"), "{context}");
    assert!(context.contains("14 characters)\nBody\n+++\nmore\n"), "{context}");

    fs::write(&md, "+++\nname = \"X\"\n+++").unwrap();
    assert!(home.ok(&["context", "demo"]).contains("; 0 characters)"));

    for (text, error) in [("no front matter", "must start with a `+++` line"), ("+++\nname = \n+++\n", "front matter does not parse"),
                          ("+++\nname = \"X\"\n", "front matter has no closing `+++` line")] {
        fs::write(&md, text).unwrap();
        let context = home.ok(&["context", "demo"]);
        assert!(context.contains(&format!("config-error: PROJECT.md: PROJECT.md {error}")), "{text:?}: {context}");
        assert!(!context.contains("Name: X"), "{text:?}");
    }
}

/// Safety overrides live in the user's config.toml, keyed by the project's
/// canonical path; other projects keep the defaults and a bad value is refused.
#[test]
fn safety_overrides_are_keyed_by_canonical_project_path() {
    let home = Home::new();
    home.ok(&["new", "--legacy", "demo"]);
    home.ok(&["new", "--legacy", "other"]);
    let defaults = "  start_threads = \"propose\"\n  coordinator_agent_args = []\n  thread_agent_args = []\n  coordinator_agent_args_kind = None\n  thread_agent_args_kind = None\n  routine_commands = false\n  cleanup_resolved = \"auto\"\n  resolve_threads = \"propose\"\n";
    assert!(home.ok(&["safety", "show", "demo"]).contains(defaults));
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(shown.contains("--sandbox") && shown.contains("workspace-write") && shown.contains("on-request") && shown.contains("acceptEdits") && shown.contains("built-in defaults"), "{shown}");
    assert!(!shown.contains("network_access=true"), "{shown}");

    let canonical = fs::canonicalize(home.root().join("demo")).unwrap();
    let config = home.0.path().join(".config/herdr-farm");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.toml"), format!("[safety.{:?}]\nstart_threads = \"auto\"\nthread_agent_args = [\"--x\"]\n", canonical.to_str().unwrap())).unwrap();
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(shown.contains("  start_threads = \"auto\"\n  coordinator_agent_args = []\n  thread_agent_args = [\"--x\"]\n"), "{shown}");
    assert!(shown.contains("  routine_commands = false\n"), "{shown}");
    assert!(shown.ends_with(&format!("[safety.{:?}]\n", canonical.to_str().unwrap())), "{shown}");
    assert!(home.ok(&["context", "demo"]).contains("Safety: start_threads=auto routine_commands=false thread_agent_args=[\"--x\"]"));
    assert!(home.ok(&["safety", "show", "other"]).contains(defaults));

    fs::write(config.join("config.toml"), format!("[safety.{:?}]\nthread_network = true\n", canonical.to_str().unwrap())).unwrap();
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(shown.contains("network_access=true"), "{shown}");
    fs::write(config.join("config.toml"), format!("[safety.{:?}]\nthread_agent_args = []\nthread_network = true\n", canonical.to_str().unwrap())).unwrap();
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(shown.contains("codex: [] (configured)") && shown.contains("claude: [] (configured)"), "{shown}");
    assert!(!shown.contains("network_access=true"), "{shown}");

    fs::write(config.join("config.toml"), format!("[safety.{:?}]\nstart_threads = \"yolo\"\n", canonical.to_str().unwrap())).unwrap();
    let error = home.refused(&["safety", "show", "demo"]);
    assert!(error.contains("start_threads must be \"propose\" or \"auto\", not \"yolo\""), "{error}");
}

#[test]
fn claude_command_prefixes_are_validated_and_visible_through_cli() {
    let home = Home::new();
    home.ok(&["new", "--legacy", "demo"]);
    let project = home.root().join("demo").canonicalize().unwrap();
    let config = home.0.path().join(".config/herdr-farm/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let write = |entries: Vec<String>| fs::write(&config, format!("[safety.{:?}]\nthread_allowed_commands={}\n", project.display().to_string(), serde_json::to_string(&entries).unwrap())).unwrap();
    write(vec!["godot --headless:*".into(), "tools/run_tests.sh:*".into()]);
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(shown.contains("Bash(git merge:*)") && !shown.contains("Bash(sed -n:*)") && !shown.contains("Bash(find:*)") && !shown.contains("Bash(rg:*)") && shown.contains("Bash(godot --headless:*)") && shown.contains("Bash(tools/run_tests.sh:*)"), "{shown}");
    for entries in [
        vec!["echo ok; curl evil:*".into()], vec!["sudo ls:*".into()],
        vec!["/usr/bin/sudo ls:*".into()], vec!["cat $(whoami):*".into()],
        vec!["rg | sh:*".into()], vec!["ls\ncat:*".into()], vec!["ls*".into()],
        vec![format!("{}:*", "a".repeat(255))], vec!["ls:*".into(); 65],
    ] {
        write(entries);
        let error = home.refused(&["safety", "show", "demo"]);
        assert!(error.contains("thread_allowed_commands"), "{error}");
    }
}

/// Public permission workflow: committed target scripts, interpreter narrowing,
/// owner policy, escalation inbox, attribution, rejection and revocation.
#[test]
fn coordinator_permissions_follow_target_and_owner_policy() {
    use std::io::Write;
    use std::process::Stdio;
    let home = Home::new();
    let repo = home.0.path().join("repo");
    fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.0.path())
            .args(["-C", repo.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    fs::create_dir(repo.join("tools")).unwrap();
    fs::write(repo.join("tools/run-tests.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::write(repo.join("tools/check.py"), "print('checked')\n").unwrap();
    fs::write(repo.join("-c"), "print('fixture')\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "reviewed scripts"]);
    let blob = git(&["rev-parse", "main:tools/run-tests.sh"]);
    home.ok(&["new", "--legacy", "demo", "--repo", repo.to_str().unwrap()]);
    let project = home.root().join("demo");
    let grant = |prefix: &str| {
        home.ok(&[
            "safety",
            "grant",
            "demo",
            "--allow",
            prefix,
            "--reason",
            "worker blocked on test",
        ])
    };
    assert!(grant("tools/run-tests.sh:*").starts_with("granted permission-1"));
    assert!(grant("python3 tools/check.py:*").starts_with("granted permission-2"));
    assert!(grant("python3:*").starts_with("requested permission-3"));
    assert!(grant("curl:*").starts_with("requested permission-4"));
    assert!(grant("cargo test:*").starts_with("granted permission-5"));
    let shown = home.ok(&["safety", "show", "demo"]);
    assert!(
        shown.contains("worker_permissions = \"coordinator\"")
            && shown.contains("source=coordinator")
            && shown.contains("committed project script")
            && shown.contains(blob.trim()),
        "{shown}"
    );
    assert!(
        home.ok(&["safety", "requests", "demo"])
            .contains("permission-3\tpython3:*")
    );
    assert!(
        home.ok(&["context", "demo"])
            .contains("Owner permission requested: curl:*")
    );
    assert!(
        home.refused(&["safety", "approve", "demo", "permission-3"])
            .contains("owner at a terminal")
    );
    assert!(
        home.refused(&["safety", "revoke", "demo", "tools/run-tests.sh:*"])
            .contains("owner at a terminal")
    );
    // A real owner terminal exercises the same interactive entry point used in production.
    let owner = |args: &[&str], confirmation: &str| {
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
        let root = home.root();
        let command = std::iter::once(BIN)
            .chain(["--root", root.to_str().unwrap()])
            .chain(args.iter().copied())
            .map(quote)
            .collect::<Vec<_>>()
            .join(" ");
        let mut child = Command::new("script")
            .env_clear()
            .env("HOME", home.0.path())
            .env("PATH", "/usr/bin:/bin")
            .env(
                "HERDR_FARM_TEST_TIME_SCALE",
                include_str!("support/time-scale.txt").trim(),
            )
            .args(["-q", "-e", "-c", &command, "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{confirmation}").unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "owner {args:?}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    owner(
        &["safety", "approve", "demo", "permission-3"],
        "permission-3",
    );
    let approved = home.ok(&["safety", "show", "demo"]);
    assert!(approved.contains("granted python3:*") && approved.contains("decision=owner:terminal"));
    owner(
        &[
            "safety",
            "reject",
            "demo",
            "permission-4",
            "--reason",
            "network forbidden",
        ],
        "permission-4",
    );
    assert!(!home.ok(&["safety", "requests", "demo"]).contains("curl:*"));
    fs::write(
        repo.join("tools/run-tests.sh"),
        "#!/bin/sh\n# merged update\nexit 0\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "merged script change"]);
    let effective = home.ok(&["safety", "show", "demo"]);
    assert!(
        effective
            .lines()
            .find(|l| l.contains("effective worker arguments:"))
            .unwrap()
            .contains("Bash(tools/run-tests.sh:*)")
    );
    fs::remove_file(repo.join("tools/run-tests.sh")).unwrap();
    git(&["add", "-u"]);
    git(&["commit", "-qm", "remove merged script"]);
    let effective = home.ok(&["safety", "show", "demo"]);
    assert!(
        !effective
            .lines()
            .find(|l| l.contains("effective worker arguments:"))
            .unwrap()
            .contains("Bash(tools/run-tests.sh:*)")
    );
    owner(
        &["safety", "revoke", "demo", "tools/run-tests.sh:*"],
        "tools/run-tests.sh:*",
    );
    assert!(
        home.ok(&["safety", "show", "demo"])
            .contains("revoked tools/run-tests.sh:*")
    );
    assert!(grant("python3 -c:*").starts_with("requested"));
    let md = fs::read_to_string(project.join("PROJECT.md")).unwrap();
    fs::write(
        project.join("PROJECT.md"),
        md.replacen("+++\n", "+++\nintegration_target = 'main'\n", 1),
    )
    .unwrap();
    git(&["checkout", "-qb", "worker"]);
    fs::write(repo.join("tools/worker-only.sh"), "exit 0\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "worker only"]);
    assert!(grant("tools/worker-only.sh:*").starts_with("requested"));
    let config = home.0.path().join(".config/herdr-farm/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        format!(
            "[safety.{:?}]\nworker_permissions='owner'\n",
            project.display().to_string()
        ),
    )
    .unwrap();
    assert!(grant("cargo check:*").starts_with("requested"));
    assert!(grant("python3 tools/check.py:*").starts_with("granted")); // existing grants survive policy edits
    let before = fs::read(&config).unwrap();
    assert!(grant("tools/check.py:*").starts_with("requested"));
    assert_eq!(
        fs::read(&config).unwrap(),
        before,
        "grant rewrote owner policy"
    );
    fs::write(
        &config,
        format!(
            "[safety.{:?}]\ngrantable_commands=['custom-check:*','curl:*','python3:*','git:*']\n",
            project.display().to_string()
        ),
    )
    .unwrap();
    assert!(grant("custom-check:*").starts_with("granted"));
    assert!(grant("curl:*").starts_with("requested"));
    assert!(grant("bash:*").starts_with("requested"));
    assert!(grant("git:*").starts_with("requested"));
    assert!(grant("echo ok; curl evil:*").starts_with("requested"));
    assert!(grant("/tmp/outside:*").starts_with("requested"));
    fs::write(&config,format!("[safety.{:?}]\nthread_agent_args=['--vendor-option']\nthread_agent_args_kind='claude'\n",project.display().to_string())).unwrap();
    let explicit = home.ok(&["safety", "show", "demo"]);
    let arguments = explicit
        .lines()
        .find(|line| line.contains("effective worker arguments:"))
        .unwrap();
    assert!(arguments.contains("--vendor-option") && arguments.contains("Bash(custom-check:*)"));
    fs::write(
        &config,
        format!(
            "[safety.{:?}]\nworker_permissions='invalid'\n",
            project.display().to_string()
        ),
    )
    .unwrap();
    assert!(
        home.refused(&["safety", "show", "demo"])
            .contains("worker_permissions")
    );
    let state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.join(".state/worker-permissions.json")).unwrap(),
    )
    .unwrap();
    assert!(
        state["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "rejected" && r["decision_reason"] == "network forbidden")
    );
}

/// New canonical memory is immediately usable; signer-less creation retains
/// Markdown and can later use the ordinary adoption command.
#[cfg(feature = "state-store")]
#[test]
fn canonical_creation_installs_memory_or_explains_signer_fallback() {
    let home = Home::new();
    home.ok(&["new", "signed"]);
    let inspect = |name: &str| -> serde_json::Value {
        serde_json::from_str(&home.ok(&["memory", name, "inspect"])).unwrap()
    };
    let memory = inspect("signed");
    assert_eq!(memory["authority"], "sqlite-v1");
    assert_eq!(memory["policies"].as_array().unwrap().len(), 1);
    let index = fs::read(home.root().join("signed/MEMORY.md")).unwrap();
    let doctor = home.cli(&["doctor"]);
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("memory=sqlite-v1"));
    let body = home.0.path().join("decision.md");
    fs::write(&body, "Keep all builds offline.\n").unwrap();
    home.ok(&["memory", "signed", "record", "--title", "Offline builds", "--provenance", "owner decision", "--body-file", body.to_str().unwrap()]);
    assert_eq!(inspect("signed")["records"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(home.root().join("signed/MEMORY.md")).unwrap(), index);

    // A busy root ticker lock cannot block setup of an unpublished project.
    let ticker = fs::File::create(home.root().join(".ticker.lock")).unwrap();
    ticker.lock().unwrap();
    home.ok(&["new", "busy-root"]);
    assert_eq!(inspect("busy-root")["authority"], "sqlite-v1");
    ticker.unlock().unwrap();

    let config = home.0.path().join(".config/herdr-farm");
    let key = config.join("owner-approval");
    let saved = home.0.path().join("saved-key");
    fs::rename(&key, &saved).unwrap();
    let output = home.ok(&["new", "unsigned"]);
    assert_eq!(output.lines().filter(|line| line.contains("owner signer unavailable")).count(), 1);
    assert!(output.contains("herdr-farm memory unsigned adopt"));
    let memory = inspect("unsigned");
    assert_eq!(memory["authority"], "legacy-markdown");
    assert!(memory["policies"].as_array().unwrap().is_empty());
    fs::rename(saved, key).unwrap();
    home.ok(&["memory", "unsigned", "adopt"]);
    assert_eq!(inspect("unsigned")["authority"], "sqlite-v1");
    assert_eq!(inspect("signed")["records"].as_array().unwrap().len(), 1);

    home.ok(&["new", "--legacy", "legacy"]);
    assert!(!home.root().join("legacy/.state/format.json").exists());
    assert!(!home.root().join("legacy/.state/state.db").exists());
    assert_eq!(fs::read(home.root().join("legacy/MEMORY.md")).unwrap(), index);
}
