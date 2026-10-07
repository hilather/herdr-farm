//! CLI projections over retained verification fixtures and real host checks / collection.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use support::telemetry::*;
fn metric(f: &Fixture, id: &str) -> Value {
    f.cli_args(&["query", "--metric", id, "--json"]).0["results"][0].clone()
}
// Canonical imported-history fixture; tests exercise CLI projections, not SQL helpers.
fn submissions(f: &Fixture) -> Vec<String> {
    submissions_in(f, "/repo", &"b".repeat(40))
}
fn submissions_in(f: &Fixture, repository: &str, oid: &str) -> Vec<String> {
    submissions_with_policy(f, repository, oid, &json!({"version":2,"toolchain":"fixture","checks":["/bin/true"]}).to_string())
}
fn submissions_with_policy(f: &Fixture, repository: &str, oid: &str, policy: &str) -> Vec<String> {
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    db.execute("INSERT INTO task_contracts(task_id,contract_revision,project_store,expected_head,repository,base_oid,object_format,route,raw_bytes,raw_digest,installed_seq) VALUES('work',1,?3,0,'/repo',?1,'sha1','verify_only',x'61',?2,(SELECT max(sequence) FROM events))",rusqlite::params!["b".repeat(40),"c".repeat(64),f.project.join(".state/state.db").canonicalize().unwrap().display().to_string()]).unwrap();
    db.execute(
        "INSERT INTO acceptance_policies VALUES('work',1,'accept-1',?1)",
        [policy],
    )
    .unwrap();
    (0..4).map(|i| {
        let id=format!("{:064x}",i+1);
        db.execute("INSERT INTO result_submissions(submission_id,project_store,idempotency_key,payload_digest,payload,task_id,contract_revision,contract_digest,attempt_id,repository,base_oid,candidate_oid,object_format,artifact_manifest,claimed_checks,created_unix_ms) VALUES(?1,?6,?1,?1,'{}','work',1,?1,?2,?7,?3,?3,'sha1','[]',?4,?5)",rusqlite::params![id,f.attempt,oid,if i==2 {"[]"}else{"[\"PRIVATE CLAIM TEXT\"]"},f.decided+i,f.project.join(".state/state.db").canonicalize().unwrap().display().to_string(),repository]).unwrap();id
    }).collect()
}
fn sandbox(
    f: &Fixture,
    sub: &str,
    key: usize,
    at: i64,
    outcome: &str,
    ms: Option<i64>,
    metadata: bool,
) {
    let digest = format!("{:064x}", key + 100);
    sandbox_policy(f, sub, key, at, outcome, ms, metadata, "accept-1", &digest);
}
#[allow(clippy::too_many_arguments)]
fn sandbox_policy(
    f: &Fixture,
    sub: &str,
    key: usize,
    at: i64,
    outcome: &str,
    ms: Option<i64>,
    metadata: bool,
    policy: &str,
    digest: &str,
) {
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let (task, attempt, oid): (String, String, String) = db
        .query_row(
            "SELECT task_id,attempt_id,candidate_oid FROM result_submissions WHERE submission_id=?1",
            [sub],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let id = format!("{:064x}", key + 100);
    let pass = outcome == "pass";
    let meta =
        metadata.then(|| json!({"toolchain":{"name":"fixture"},"duration_ms":ms}).to_string());
    db.execute("INSERT INTO verification_runs(run_id,project_store,idempotency_key,payload_digest,submission_id,task_id,contract_revision,contract_digest,attempt_id,policy_id,policy_digest,commit_oid,tree_oid,object_format,memory_fence,isolation,argv,library_manifest,state,reason,exit_status,receipt_digest,store_device,store_inode,created_unix_ms,metadata) VALUES(?1,'store',?1,?1,?2,?11,1,?1,?3,?12,?13,?4,?4,'sha1',0,'linux-unshare-user-pid-mount-v1','[\"x\"]','[]',?5,?6,?7,?8,1,1,?9,?10)",rusqlite::params![id,sub,attempt,oid,if pass {"accepted"}else{"rejected"},if pass {None}else{Some(if outcome=="red"{"checks_failed"}else{outcome})},if pass {Some(0)}else if outcome=="red"{Some(1)}else{None},pass.then_some(&id),at,meta,task,policy,digest]).unwrap();
    if pass {
        db.execute("INSERT INTO verified_results VALUES(?1,?1,?2,?3,?3,'sha1',?5,?1,'linux-unshare-user-pid-mount-v1',0,?4)",rusqlite::params![id,sub,oid,at,digest]).unwrap();
    }
}
fn host(f: &Fixture, sub: &str, command: &[&str], extra: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .env_clear()
        .env("HOME", f.tmp.path().join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", f.scale)
        .args([
            "--root",
            f.root.to_str().unwrap(),
            "result",
            "demo",
            "host-check",
            "--task",
            "work",
            "--submission",
            sub,
        ])
        .args(extra)
        .arg("--")
        .args(command)
        .output()
        .unwrap()
}
#[test]
fn first_verdict_claims_sources_duration_and_windows_through_cli() {
    let f = Fixture::with_lineage("verify-work");
    let ids = submissions(&f);
    assert_eq!(metric(&f, "M100")["reason"], "no_independent_runs");
    assert_eq!(
        metric(&f, "M100")["detail"]["coverage"]["host"]["reason"],
        "no_host_checks_recorded"
    );
    assert_eq!(
        metric(&f, "M100")["detail"]["coverage"]["sandbox"]["reason"],
        "no_executable_runs"
    );
    f.cli("collect");
    // Earlier red beats later green, including a cross-source retry.
    sandbox(&f, &ids[0], 1, f.decided + 10, "red", Some(60000), true);
    assert!(host(&f, &ids[0], &["/bin/true"], &[]).status.success());
    // A timeout is excluded before the first completed result.
    sandbox(
        &f,
        &ids[1],
        2,
        f.decided + 11,
        "timeout",
        Some(120000),
        true,
    );
    sandbox(&f, &ids[1], 3, f.decided + 12, "pass", Some(180000), true);
    assert_eq!(
        host(&f, &ids[2], &["/bin/false"], &[]).status.code(),
        Some(1)
    );
    assert!(host(&f, &ids[2], &["/bin/true"], &[]).status.success());
    let report = f.report();
    let m = &report["metrics"]["M100"];
    assert_eq!(m["value"], "1/3");
    assert_eq!(m["no_independent_run"], 1);
    assert_eq!(m["excluded_runs_by_reason"]["timeout"], 1);
    assert_eq!(m["by_profile"]["codex"]["value"], "1/3");
    assert_eq!(m["by_role"]["build"]["value"], "1/3");
    assert_eq!(m["by_source"]["sandbox"]["value"], "1/2");
    assert_eq!(m["by_source"]["host"]["value"], "0/1");
    let q = &report["metrics"]["M103"];
    assert_eq!(q["value"], "3/4");
    assert_eq!(q["claimed_check_entries"], 3);
    assert_eq!(q["claimed_without_agreement"]["value"], "2/3");
    let durations = f.cli_args(&["host-checks", "--json"]).0["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["duration_ms"].as_i64().unwrap())
        .sum::<i64>();
    let m = &report["metrics"]["M102"];
    assert_eq!(m["accepted_tasks"], 1);
    assert_eq!(m["sandbox"], "360000/60000");
    assert_eq!(m["host"], format!("{durations}/60000"));
    assert_eq!(m["value"], format!("{}/60000", 360000 + durations));
    assert_eq!(m["distribution"]["median"], m["value"]);
    assert_eq!(m["distribution"]["p90"], m["value"]);
    for id in ["M100", "M101", "M102", "M103"] {
        assert_eq!(metric(&f, id)["detail"], report["metrics"][id]);
    }
    let window = f
        .cli_args(&["report", "--since", &(f.decided + 3).to_string(), "--json"])
        .0;
    assert_eq!(window["metrics"]["M100"]["no_independent_run"], 1);
    assert_eq!(window["metrics"]["M103"]["value"], "1/1");
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("PRIVATE CLAIM TEXT")
    );
    // Two distinct accepted tasks exercise the even median and nearest-rank tail.
    let mut store =
        herdr_farm::store::SqliteStore::open(&f.project.join(".state/state.db")).unwrap();
    use herdr_farm::domain::*;
    let task = TaskId::new("second").unwrap();
    let attempt = AttemptId::new("second-attempt").unwrap();
    store
        .commit(Commit {
            expected_head: store.current_head().unwrap(),
            mutations: vec![
                Mutation::Task {
                    expected: None,
                    next: Task {
                        id: task.clone(),
                        revision: 1,
                        state: TaskState::Running,
                        title: "second".into(),
                        active_attempt: Some(attempt.clone()),
                    },
                },
                Mutation::Attempt {
                    expected: None,
                    next: Attempt {
                        id: attempt.clone(),
                        task,
                        revision: 1,
                        state: AttemptState::Running,
                        snapshot: None,
                        reservation: "second-fixture".into(),
                        termination_observed: false,
                    },
                },
            ],
        })
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    db.execute("INSERT INTO task_contracts(task_id,contract_revision,project_store,expected_head,repository,base_oid,object_format,route,raw_bytes,raw_digest,installed_seq) SELECT 'second',1,project_store,0,repository,base_oid,object_format,route,raw_bytes,raw_digest,installed_seq FROM task_contracts WHERE task_id='work'",[]).unwrap();
    db.execute("INSERT INTO acceptance_policies SELECT 'second',1,policy_id,body FROM acceptance_policies WHERE task_id='work'",[]).unwrap();
    let second = "e".repeat(64);
    db.execute("INSERT INTO result_submissions(submission_id,project_store,idempotency_key,payload_digest,payload,task_id,contract_revision,contract_digest,attempt_id,repository,base_oid,candidate_oid,object_format,artifact_manifest,claimed_checks,created_unix_ms) SELECT ?1,project_store,?1,?1,payload,'second',1,contract_digest,'second-attempt',repository,base_oid,candidate_oid,object_format,artifact_manifest,'[]',?2 FROM result_submissions WHERE submission_id=?3",rusqlite::params![second,f.decided+4,ids[0]]).unwrap();
    sandbox(&f, &second, 5, f.decided + 21, "pass", Some(120000), true);
    let m = metric(&f, "M102");
    let sum = 480000 + durations;
    assert_eq!(m["value"], format!("{sum}/120000"));
    assert_eq!(
        m["detail"]["distribution"]["median"],
        format!("{sum}/120000")
    );
    assert_eq!(
        m["detail"]["distribution"]["p90"],
        format!("{}/60000", 360000 + durations)
    );
    assert_eq!(m["detail"]["distribution"]["samples"], 2);
    sandbox(&f, &ids[3], 4, f.decided + 20, "pass", None, false);
    assert_eq!(
        metric(&f, "M102")["reason"],
        "verification_duration_missing"
    );
    assert_eq!(
        metric(&f, "M100")["detail"]["coverage"]["executable_runs_without_metadata"],
        1
    );
}
#[test]
fn session_classifier_collects_only_class_and_observed_exit_status() {
    let f = Fixture::new();
    assert_eq!(
        metric(&f, "M101")["reason"],
        "session_metadata_not_collected"
    );
    assert_eq!(
        metric(&f, "M100")["detail"]["coverage"]["sandbox"]["reason"],
        "no_executable_policies"
    );
    submissions(&f);
    let path = f.rollout(
        &f.home,
        "classification",
        &["head.jsonl"],
        &f.worktree(),
        unix_ms(),
        "0.154.0",
    );
    let timestamp = jiff::Timestamp::from_millisecond(unix_ms())
        .unwrap()
        .to_string();
    let mut text = fs::read_to_string(&path).unwrap();
    for (id, command, exit) in [
        ("not-test", json!(["/bin/echo", "PRIVATE COMMAND"]), Some(0)),
        ("test", json!(["/bin/true"]), None),
    ] {
        text.push_str(&(json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":id,"command":command,"exit_code":exit,"stdout":"PRIVATE OUTPUT"}}}).to_string()+"\n"));
    }
    // One public collect classifies a large same-attempt rollout, including
    // exact-match misses; declarations and raw commands never enter the sidecar.
    for index in 0..256 {
        let command = if index % 2 == 0 { json!(["/bin/true"]) }
            else { json!(["/bin/true", "PRIVATE COMMAND"]) };
        text.push_str(&(json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":format!("bulk-{index:03}"),"command":command}}}).to_string()+"\n"));
    }
    fs::write(&path, &text).unwrap();
    f.cli("collect");
    // Repeated collection preserves all classifications and native identities.
    f.cli("collect");
    assert_eq!(metric(&f, "M101")["value"], "0/1");
    text.push_str(&(json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"test-result","command":["/bin/true"],"exit_code":1}}}).to_string()+"\n"));
    fs::write(&path, text).unwrap();
    f.cli("collect");
    assert_eq!(metric(&f, "M101")["value"], "1/1");
    let db = f.sidecar();
    let rows: Vec<(Option<String>, i64)> = db
        .prepare("SELECT class,classification_available FROM codex_exec_classes ORDER BY item_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut expected: Vec<_> = (0..256).map(|index|
        ((index % 2 == 0).then(|| "project_test".into()), 1)).collect();
    expected.extend([(None, 1), (Some("project_test".into()), 1), (Some("project_test".into()), 1)]);
    assert_eq!(rows, expected);
    for file in [
        f.project.join(".state/telemetry.db"),
        f.project.join(".state/telemetry.db-wal"),
    ] {
        if let Ok(bytes) = fs::read(file) {
            for marker in [b"PRIVATE COMMAND".as_slice(), b"PRIVATE OUTPUT".as_slice()] {
                assert!(!bytes.windows(marker.len()).any(|w| w == marker));
            }
        }
    }
    // Read-only old stream fixture exposes precise absence rather than a fabricated zero.
    db.execute("DROP TABLE codex_exec_classes", []).unwrap();
    assert_eq!(
        metric(&f, "M101")["reason"],
        "worker_session_stream_before_v17"
    );
    db.execute(
        "UPDATE telemetry_streams SET version=16 WHERE stream='ingest'",
        [],
    )
    .unwrap();
    f.cli("collect");
    assert_eq!(metric(&f, "M101")["value"], "1/1");
    assert_eq!(f.count("codex_exec_classes"), 259);
    db.execute("DROP TABLE codex_exec_items", []).unwrap();
    assert_eq!(
        metric(&f, "M101")["reason"],
        "session_metadata_not_collected"
    );
    assert_eq!(
        f.report()["metrics"]["M101"]["value"]["reason"],
        "session_metadata_not_collected"
    );
}

#[test]
fn old_timeout_is_excluded_but_an_earlier_unknown_verdict_is_unavailable() {
    let f = Fixture::new();
    let ids = submissions(&f);
    sandbox(&f, &ids[0], 1, f.decided + 10, "timeout", None, false);
    sandbox(&f, &ids[0], 2, f.decided + 20, "pass", Some(60000), true);
    let m = metric(&f, "M100");
    assert_eq!(m["value"], "1/1");
    assert_eq!(m["detail"]["excluded_runs_by_reason"]["timeout"], 1);
    assert_eq!(m["detail"]["no_independent_run"], 3);
    assert_eq!(
        metric(&f, "M102")["reason"],
        "verification_duration_missing"
    );
    sandbox(&f, &ids[0], 3, f.decided + 5, "red", None, false);
    assert_eq!(metric(&f, "M100")["reason"], "first_run_metadata_missing");
    assert_eq!(
        metric(&f, "M103")["detail"]["claimed_without_agreement"]["value"]["reason"],
        "first_run_metadata_missing"
    );
}

#[test]
fn attempt_only_host_history_includes_pre_submission_checks_without_duplicating_reasons() {
    let f = Fixture::new();
    let ids = submissions(&f);
    f.cli("collect");
    let timed = host(&f, &ids[0], &["/bin/sleep", "2"], &["--timeout", "1"]);
    assert!(!timed.status.success());
    assert!(host(&f, &ids[0], &["/bin/true"], &[]).status.success());
    // Imported host history may carry only the attempt, and precede submission.
    let db = f.sidecar();
    db.execute("UPDATE host_checks SET started_unix_ms=?1,payload=json_set(payload,'$.submission_id',NULL,'$.started_unix_ms',?1)", [f.decided-100]).unwrap();
    let m = metric(&f, "M100");
    assert_eq!(m["value"], "4/4");
    assert_eq!(m["detail"]["no_independent_run"], 0);
    assert_eq!(m["detail"]["excluded_runs_by_reason"]["timeout"], 1);
    assert_eq!(m["detail"]["by_source"]["host"]["value"], "4/4");
    assert_eq!(
        metric(&f, "M103")["detail"]["claimed_without_agreement"]["value"],
        "0/3"
    );
}

/// Imported canonical history is projected through the CLI, including collection
/// before the final policy verdict and refresh of already settled proxy rows.
#[test]
fn acceptance_and_ci_proxy_require_every_policy_of_one_candidate() {
    for (two_policies, pass, wrong_digest, legacy_policy, expected) in [
        (true, false, false, false, 0),
        (true, true, false, false, 1),
        (false, true, false, false, 1),
        // Executable documents require the body digest even with a receipt.
        (false, true, true, false, 0),
        // Opaque legacy text has no executable body-digest contract.
        (false, true, true, true, 1),
    ] {
        let f = Fixture::new();
        let repo = f.tmp.path().join("candidate-repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("deliverable.txt"), "candidate output\n").unwrap();
        for args in [vec!["init", "-q"], vec!["add", "deliverable.txt"], vec!["-c", "user.name=fixture", "-c", "user.email=f@f", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "candidate"]] {
            assert!(Command::new("git").env_clear().env("PATH", "/usr/bin:/bin").env("HERDR_FARM_TEST_TIME_SCALE", f.scale).current_dir(&repo).args(args).status().unwrap().success());
        }
        let oid = Command::new("git").env_clear().env("PATH", "/usr/bin:/bin").env("HERDR_FARM_TEST_TIME_SCALE", f.scale).current_dir(&repo).args(["rev-parse", "HEAD"]).output().unwrap();
        let oid = String::from_utf8(oid.stdout).unwrap().trim().to_owned();
        let ids = if legacy_policy {
            submissions_with_policy(&f, repo.to_str().unwrap(), &oid, "cargo test")
        } else {
            submissions_in(&f, repo.to_str().unwrap(), &oid)
        };
        let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
        db.execute("UPDATE tasks SET state='failed' WHERE id='work'", []).unwrap();
        let body: String = db.query_row("SELECT body FROM acceptance_policies WHERE task_id='work'", [], |r| r.get(0)).unwrap();
        let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(body.as_bytes()));
        if two_policies {
            let output = json!({"version":1,"checks":["/bin/test","-s","deliverable.txt"]}).to_string();
            let output_digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(output.as_bytes()));
            db.execute("INSERT INTO acceptance_policies VALUES('work',1,'output-1',?1)", [&output]).unwrap();
            sandbox_policy(&f, &ids[0], 900, f.decided+1, "pass", None, false, "output-1", &output_digest);
            f.cli_args(&["quality", "collect"]);
            // Historical collectors persisted the green file-presence run
            // even while the candidate's toolchain policy was still pending.
            f.sidecar().execute("INSERT INTO proxy_signals(kind,task_id,submission_id,attempt_id,run_id,policy_digest,base_oid,candidate_oid,ci_state,verified_unix_ms,tests_added_lines,tests_deleted_lines,tests_binary_files,weakening,weakening_rule,source_trust,observed_unix_ms)
                VALUES('first_candidate_ci','work',?1,?2,?3,?4,?5,?5,'accepted',?6,0,0,0,'clear','tests-net-removal.v1','proxy_observed',?6)",
                rusqlite::params![ids[0], f.attempt, format!("{:064x}",1000), output_digest, oid, f.decided+1]).unwrap();
            f.cli_args(&["quality", "collect"]);
            assert_eq!(f.count("proxy_signals"), 0);
            let pending = f.cli_args(&["quality", "report"]).0["metrics"]["M45"].clone();
            assert_eq!(pending["pending"], 1);
            assert_eq!(pending["denominator"], 0);
        }
        if !two_policies && !wrong_digest {
            sandbox_policy(&f, &ids[0], 899, f.decided+1, "isolation_setup_failed", None, false, "accept-1", &digest);
            f.cli_args(&["quality", "collect"]);
        }
        let run_digest = if wrong_digest { "f".repeat(64) } else { digest.clone() };
        sandbox_policy(&f, &ids[0], 901, f.decided+2, if pass {"pass"} else {"red"}, None, false, "accept-1", &run_digest);
        if two_policies && !pass {
            // A different submission's green toolchain run cannot complete
            // the first submission's accepted file-presence policy.
            sandbox_policy(&f, &ids[1], 902, f.decided+3, "pass", None, false, "accept-1", &digest);
        }
        f.cli_args(&["quality", "collect"]);
        if two_policies && !pass {
            // Upgrade fixture: older collectors settled the file-presence
            // run as accepted. Normal collection must repair that derived row.
            f.sidecar().execute("UPDATE proxy_signals SET ci_state='accepted'", []).unwrap();
            f.cli_args(&["quality", "collect"]);
        }
        f.rollout(&f.home, "priced", &[concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/accounting/priced-before.jsonl")], &f.worktree(), f.decided, "0.154.0");
        f.cli("collect");
        f.cli_args(&["accounting", "sync"]);
        let card = f.tmp.path().join("rates.json");
        fs::write(&card, fs::read_to_string(Path::new(FIXTURES).parent().unwrap().join("accounting/rates-v1.json")).unwrap().replace("@BOUNDARY@", &(unix_ms()+60_000).to_string())).unwrap();
        f.cli_args(&["accounting", "import-rate-card", card.to_str().unwrap()]);
        f.cli_args(&["accounting", "reprice"]);
        let report = f.report();
        assert_eq!(report["tasks"]["accepted"], expected);
        assert_eq!(report["metrics"]["M01"]["value"], expected);
        assert_eq!(report["metrics"]["M02"]["numerator"], expected);
        assert_eq!(report["metrics"]["M02"]["denominator"], 1);
        assert_eq!(report["metrics"]["M02"]["value"], format!("{expected}/1"));
        assert_eq!(report["metrics"]["M04"]["denominator"], expected);
        assert_eq!(report["metrics"]["M04"]["value"], if expected == 1 { json!("0.004/1") } else { Value::Null });
        let proxy = f.cli_args(&["quality", "report"]).0["metrics"]["M45"].clone();
        assert_eq!(proxy["numerator"], expected);
        assert_eq!(proxy["denominator"], if wrong_digest && !legacy_policy { 0 } else { 1 });
        if wrong_digest && !legacy_policy {
            assert_eq!(proxy["pending"], 1);
            assert_eq!(f.count("proxy_signals"), 0);
            continue;
        }
        assert_eq!(proxy["value"], format!("{expected}/1"));
        assert_eq!(f.sidecar().query_row("SELECT ci_state FROM proxy_signals", [], |r| r.get::<_,String>(0)).unwrap(), if pass {"accepted"} else {"rejected"});
    }
}
