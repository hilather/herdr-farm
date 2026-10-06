//! Coordinator checks through the public CLI, with real processes and stores.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
mod support;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::{Command, Output},
    time::{Duration, Instant},
};
use support::telemetry::{BIN, Fixture};

fn cli(f: &Fixture) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env_clear()
        .env("HOME", f.tmp.path().join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", f.scale)
        .args(["--root", f.root.to_str().unwrap()]);
    cmd
}
fn check(f: &Fixture, extra: &[&str], command: &[&str]) -> Output {
    cli(f)
        .args(["result", "demo", "host-check", "--task", "work"])
        .args(extra)
        .arg("--")
        .args(command)
        .output()
        .unwrap()
}
fn report(f: &Fixture) -> Value {
    f.cli_args(&["host-checks", "--json"]).0
}
fn wait(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(25);
    while !condition() {
        assert!(Instant::now() < until, "observable state did not arrive");
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn running(f: &Fixture) -> bool {
    report(f)["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["outcome"] == "running")
}
fn spool_nonempty(f: &Fixture) -> bool {
    fs::read_dir(f.project.join(".state/host-check-spool")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.metadata().is_ok_and(|m| m.len() > 0))
    })
}
#[test]
fn pass_fail_log_metadata_and_first_run_are_exposed() {
    let f = Fixture::new();
    let input = f.tmp.path().join("check-input");
    let log = f.tmp.path().join("check.log");
    let stdout = b"host stdout payload 8472\n";
    let stderr = b"host stderr payload 9981\n";
    fs::write(&input, stdout).unwrap();
    let script = f.tmp.path().join("check.sh");
    fs::write(
        &script,
        format!(
            "cat '{}'\nprintf 'host stderr payload 9981\\n' >&2\n",
            input.display()
        ),
    )
    .unwrap();
    let out = check(
        &f,
        &[
            "--cwd",
            f.tmp.path().to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--name",
            "local-tests",
        ],
        &["/bin/sh", script.to_str().unwrap()],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, stdout);
    assert_eq!(out.stderr, stderr);
    let bytes = fs::read(&log).unwrap();
    assert_eq!(bytes.len(), stdout.len() + stderr.len());
    assert!(bytes.windows(stdout.len()).any(|b| b == stdout));
    assert!(bytes.windows(stderr.len()).any(|b| b == stderr));
    let result = report(&f);
    let run = &result["runs"][0];
    assert_eq!(run["outcome"], "pass");
    assert_eq!(run["exit_code"], 0);
    assert_eq!(run["name"], "local-tests");
    assert_eq!(run["log_path"], log.to_str().unwrap());
    assert_eq!(run["log_size"], bytes.len());
    assert_eq!(
        run["log_digest"],
        format!("sha256:{:x}", Sha256::digest(&bytes))
    );
    let start = run["started_unix_ms"].as_i64().unwrap();
    let finish = run["finished_unix_ms"].as_i64().unwrap();
    assert!(start > f.decided && finish >= start && finish - start < 25_000);
    assert!(run["duration_ms"].as_u64().unwrap() < 25_000);
    assert_eq!(run["submission_id"], Value::Null);
    assert_eq!(run["submission_reason"], "no_submission");
    assert_eq!(run["caller"], "operator");
    assert_eq!(run["trust"], "local");
    assert!(run["host_load"]["sampled_unix_ms"].is_i64());
    let db = rusqlite::Connection::open(f.project.join(".state/telemetry.db")).unwrap();
    let stored: String = db
        .query_row("SELECT payload FROM host_checks", [], |r| r.get(0))
        .unwrap();
    assert!(!stored.contains("host stdout payload 8472"));
    assert!(!stored.contains("host stderr payload 9981"));
    let out = check(&f, &[], &["/bin/sh", "-c", "exit 3"]);
    assert_eq!(out.status.code(), Some(3));
    let out = check(&f, &[], &["/bin/sh", "-c", "exit 0"]);
    assert_eq!(out.status.code(), Some(0));
    let result = report(&f);
    assert_eq!(result["runs"][1]["outcome"], "fail");
    assert_eq!(result["runs"][1]["exit_code"], 3);
    assert_eq!(result["tasks"]["work"]["runs"], 3);
    assert_eq!(result["tasks"]["work"]["first_run_outcome"], "pass");
    assert_eq!(result["tasks"]["work"]["latest_outcome"], "pass");
    assert!(result["tasks"]["work"]["total_minutes"].as_f64().unwrap() >= 0.0);
    assert_eq!(f.cli_args(&["usage", "--json"]).0["host_checks"], result);
    assert_eq!(
        f.cli_args(&["host-checks", "--task", "absent", "--json"]).0["runs"],
        serde_json::json!([])
    );
}
#[test]
fn timeout_terminates_the_process_group() {
    let f = Fixture::new();
    let pids = f.tmp.path().join("pids");
    let script = f.tmp.path().join("sleep.sh");
    fs::write(
        &script,
        format!(
            "trap '' TERM\nsleep 60 &\necho $$ $! > '{}'\nwait\n",
            pids.display()
        ),
    )
    .unwrap();
    let out = check(
        &f,
        &["--timeout", "1"],
        &["/bin/sh", script.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(137));
    let run = &report(&f)["runs"][0];
    assert_eq!(run["outcome"], "timeout");
    assert_eq!(run["signal"], 9);
    let pids = fs::read_to_string(pids).unwrap();
    for pid in pids.split_whitespace() {
        wait(|| {
            fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| {
                s.rsplit_once(')')
                    .is_some_and(|(_, tail)| tail.split_whitespace().next() == Some("Z"))
            })
        });
    }
}
#[test]
fn unknown_task_and_invalid_submission_do_not_spawn() {
    let f = Fixture::new();
    let marker = f.tmp.path().join("must-not-exist");
    let out = cli(&f)
        .args([
            "result",
            "demo",
            "host-check",
            "--task",
            "unknown",
            "--",
            "/usr/bin/touch",
            marker.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown task"));
    assert!(!marker.exists());
    let out = check(
        &f,
        &["--submission", "unknown"],
        &["/usr/bin/touch", marker.to_str().unwrap()],
    );
    assert!(!out.status.success());
    assert!(!marker.exists());
    assert!(report(&f)["runs"].as_array().unwrap().is_empty());
}
#[test]
fn concurrent_writes_and_busy_finish_replay_are_idempotent() {
    let f = Fixture::new();
    f.cli("collect");
    let release = f.tmp.path().join("release");
    let script = f.tmp.path().join("wait.sh");
    fs::write(
        &script,
        format!(
            "while [ ! -f '{}' ]; do sleep 0.05; done\nexit 3\n",
            release.display()
        ),
    )
    .unwrap();
    let mut child = cli(&f)
        .args([
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--",
            "/bin/sh",
            script.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();
    wait(|| running(&f));
    let before = Instant::now();
    f.cli_args(&["accounting", "sync"]);
    f.cli("collect");
    assert!(before.elapsed() < Duration::from_secs(10));
    assert!(child.try_wait().unwrap().is_none());
    let db = rusqlite::Connection::open(f.project.join(".state/telemetry.db")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    fs::write(release, "").unwrap();
    wait(|| spool_nonempty(&f));
    assert_eq!(child.wait().unwrap().code(), Some(3));
    db.execute_batch("ROLLBACK").unwrap();
    f.cli("collect");
    assert_eq!(report(&f)["runs"][0]["outcome"], "fail");
    assert!(!spool_nonempty(&f));
    f.cli("collect");
    assert_eq!(report(&f)["runs"].as_array().unwrap().len(), 1);
    assert!(
        check(&f, &[], &["/bin/sh", "-c", "exit 0"])
            .status
            .success()
    );
    assert_eq!(report(&f)["runs"].as_array().unwrap().len(), 2);
}
#[test]
fn absent_sidecar_spools_running_and_finished_then_next_check_replays_once() {
    let f = Fixture::new();
    let path = f.project.join(".state/telemetry.db");
    fs::create_dir(&path).unwrap();
    assert_eq!(
        check(&f, &[], &["/bin/sh", "-c", "exit 3"]).status.code(),
        Some(3)
    );
    assert!(spool_nonempty(&f));
    fs::remove_dir(&path).unwrap();
    assert!(
        check(&f, &[], &["/bin/sh", "-c", "exit 0"])
            .status
            .success()
    );
    let result = report(&f);
    assert_eq!(result["runs"].as_array().unwrap().len(), 2);
    assert_eq!(result["tasks"]["work"]["first_run_outcome"], "fail");
    assert!(!spool_nonempty(&f));
    f.cli("collect");
    assert_eq!(report(&f)["runs"].as_array().unwrap().len(), 2);
}
#[test]
fn signals_forward_and_dead_recorders_read_abandoned() {
    use std::os::unix::process::ExitStatusExt;
    let f = Fixture::new();
    f.cli("collect");
    let mut child = cli(&f)
        .args([
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--",
            "/bin/sleep",
            "60",
        ])
        .spawn()
        .unwrap();
    wait(|| running(&f));
    // Wait for the actual child to exec, beyond the durable pre-spawn row.
    wait(|| {
        fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
            .is_ok_and(|s| !s.trim().is_empty())
    });
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert_eq!(child.wait().unwrap().code(), Some(143));
    assert_eq!(report(&f)["runs"][0]["outcome"], "interrupted");
    let mut child = cli(&f)
        .args([
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--",
            "/bin/sleep",
            "60",
        ])
        .spawn()
        .unwrap();
    wait(|| running(&f));
    let mut children = String::new();
    wait(|| {
        children = fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
            .unwrap_or_default();
        !children.trim().is_empty()
    });
    child.kill().unwrap();
    assert_eq!(child.wait().unwrap().signal(), Some(9));
    for pid in children.split_whitespace() {
        unsafe {
            libc::kill(-pid.parse::<i32>().unwrap(), libc::SIGKILL);
        }
    }
    assert_eq!(report(&f)["runs"][0]["outcome"], "abandoned");
}
#[test]
fn spawn_failure_is_recorded_and_returns_nonzero() {
    let f = Fixture::new();
    let out = check(&f, &[], &["/definitely-missing-herdr-check"]);
    assert_eq!(out.status.code(), Some(127));
    assert_eq!(report(&f)["runs"][0]["outcome"], "spawn_failed");
}

#[test]
fn git_head_dirty_state_and_caller_environment_are_recorded() {
    let f = Fixture::new();
    let repo = f.tmp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    ] {
        let out = Command::new("git")
            .env_clear()
            .env("HOME", f.tmp.path().join("home"))
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let expected = Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let out = cli(&f)
        .env("CHECK_ENV", "inherited-value")
        .args([
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--cwd",
            repo.to_str().unwrap(),
            "--",
            "/bin/sh",
            "-c",
            "printf '%s' \"$CHECK_ENV\"",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"inherited-value");
    let first = report(&f);
    assert_eq!(
        first["runs"][0]["commit_oid"],
        String::from_utf8_lossy(&expected.stdout).trim()
    );
    assert_eq!(first["runs"][0]["tree_dirty"], false);
    fs::write(repo.join("new-file"), "untracked").unwrap();
    assert!(
        check(
            &f,
            &["--cwd", repo.to_str().unwrap()],
            &["/bin/sh", "-c", "exit 0"]
        )
        .status
        .success()
    );
    assert_eq!(report(&f)["runs"][0]["tree_dirty"], true);
}

#[test]
fn backup_and_retention_restore_do_not_resurrect_host_checks() {
    let f = Fixture::new();
    assert!(
        check(&f, &[], &["/bin/sh", "-c", "exit 0"])
            .status
            .success()
    );
    let mut row = report(&f)["runs"][0].clone();
    let old = jiff::Timestamp::now().as_millisecond() - 100 * 86_400_000;
    row["started_unix_ms"] = serde_json::json!(old);
    row["finished_unix_ms"] = serde_json::json!(old + 10);
    f.sidecar()
        .execute(
            "UPDATE host_checks SET started_unix_ms=?1,finished_unix_ms=?2,payload=?3",
            rusqlite::params![old, old + 10, row.to_string()],
        )
        .unwrap();
    let out = f.tmp.path().join("backup");
    f.cli_args(&["backup", "create", "--out", out.to_str().unwrap()]);
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["rows"]["host_checks"], 1);
    let plan = f.cli_args(&["maintenance", "plan", "--json"]).0;
    f.cli_args(&[
        "maintenance",
        "apply",
        "--confirm",
        plan["plan_digest"].as_str().unwrap(),
        "--json",
    ]);
    assert!(report(&f)["runs"].as_array().unwrap().is_empty());
    // Replay a previously observed genuine record after the retention tombstone.
    let spool = f.project.join(".state/host-check-spool");
    fs::create_dir_all(&spool).unwrap();
    fs::write(spool.join("retained.jsonl"), format!("{row}\n")).unwrap();
    f.cli("collect");
    assert!(report(&f)["runs"].as_array().unwrap().is_empty());
    f.cli_args(&[
        "backup",
        "restore",
        "--from",
        out.to_str().unwrap(),
        "--force",
    ]);
    assert!(report(&f)["runs"].as_array().unwrap().is_empty());
}

#[test]
fn timeout_still_supervises_descendants_holding_log_pipes() {
    let f = Fixture::new();
    let log = f.tmp.path().join("background.log");
    let out = check(
        &f,
        &["--log", log.to_str().unwrap(), "--timeout", "1"],
        &["/bin/sh", "-c", "sleep 60 & exit 0"],
    );
    // The leader's actual exit code is preserved even if its remaining group
    // needs termination to close inherited output descriptors.
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(report(&f)["runs"][0]["outcome"], "timeout");
}

#[test]
fn latest_submission_binding_and_explicit_older_submission_use_public_results() {
    use serde_json::json;
    use support::replay::Lab;
    let lab = Lab::new("");
    let repo = lab.repo.canonicalize().unwrap();
    let base = lab.git(&["rev-parse", "HEAD"]);
    let contract=lab.install_contract("work",json!({"version":3,"task_id":"work","contract_revision":1,
        "deliverable":"answer","non_goals":"none","acceptance_policies":[{"id":"clean","text":support::replay::CLEAN}],
        "repository":repo,"base_oid":base,"object_format":"sha256","dependencies":[],"capability_flags":[],
        "profile_kind":"claude","retry_class":"none","result_schema_id":"result-v1","route":"verify_then_integrate",
        "scope":{"paths":[{"path":"answer.txt","access":"write"}]},"outputs":[{"path":"answer.txt","kind":"git_file"}]}));
    lab.plant_attempt("work", "host-attempt");
    let candidate = lab.commit_files(
        &repo,
        "answer",
        &base,
        &std::collections::BTreeMap::from([("answer.txt".into(), "answer\n".into())]),
    );
    let older = lab.submit(
        "work",
        "host-attempt",
        &contract,
        &repo,
        &base,
        &candidate,
        &["answer.txt"],
        "older",
    );
    assert!(
        lab.cli(&[
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--",
            "/bin/sh",
            "-c",
            "exit 0"
        ])
        .status
        .success()
    );
    let result = lab.ok(&["telemetry", "demo", "host-checks", "--json"]);
    assert_eq!(result["runs"][0]["submission_id"], older);
    assert_eq!(result["runs"][0]["attempt_id"], "host-attempt");
    assert_eq!(result["runs"][0]["submission_reason"], Value::Null);
    let newer = lab.submit(
        "work",
        "host-attempt",
        &contract,
        &repo,
        &base,
        &candidate,
        &["answer.txt"],
        "newer",
    );
    assert!(
        lab.cli(&[
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--",
            "/bin/sh",
            "-c",
            "exit 0"
        ])
        .status
        .success()
    );
    assert_eq!(
        lab.ok(&["telemetry", "demo", "host-checks", "--json"])["runs"][0]["submission_id"],
        newer
    );
    assert!(
        lab.cli(&[
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--submission",
            &older,
            "--",
            "/bin/sh",
            "-c",
            "exit 0"
        ])
        .status
        .success()
    );
    assert_eq!(
        lab.ok(&["telemetry", "demo", "host-checks", "--json"])["runs"][0]["submission_id"],
        older
    );
    assert_eq!(
        lab.ok(&["telemetry", "demo", "accounting", "host-checks"])["runs"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
