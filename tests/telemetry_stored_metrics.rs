//! MET-NOW-A CLI workflows over isolated canonical and rollout fixtures.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)] // Shared CLI fixtures spawn outside the library.
mod support;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use support::telemetry::*;

fn fixture_rollout(f: &Fixture, name: &str, fixture: &str) {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/telemetry/accounting")
            .join(fixture),
    )
    .unwrap()
    .replace(
        "@TS@",
        &jiff::Timestamp::from_millisecond(f.decided + 1)
            .unwrap()
            .to_string(),
    )
    .replace("@CWD@", &f.worktree())
    .replace("@VERSION@", "0.154.0")
    .replace("@SID@", SID);
    let dir = f.home.join(".codex/sessions/2026/09/28");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("rollout-{name}.jsonl")), text).unwrap();
}
fn unavailable(reason: &str) -> Value {
    json!({"status":"unavailable","reason":reason})
}

#[test]
fn usage_shares_first_request_and_long_context_are_reported_by_profile() {
    let f = Fixture::new();
    let empty = f.report()["metrics"].clone();
    for id in ["M55", "M56", "M57", "M58"] {
        assert_eq!(empty[id]["value"], unavailable("collection_not_run"));
    }
    assert_eq!(empty["M52"]["value"], unavailable("no_reviews"));
    assert_eq!(empty["M53"]["value"], unavailable("no_reviews"));
    assert_eq!(empty["M51"]["reason"], "empty_denominator");
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    for id in ["M55", "M56", "M57", "M58"] {
        assert_eq!(f.report()["metrics"][id]["value"], unavailable("no_usage"));
    }
    fixture_rollout(&f, "parent", "parent.jsonl");
    f.cli("collect");
    assert_eq!(
        f.report()["metrics"]["M55"]["value"],
        unavailable("accounting_sync_required")
    );
    f.cli_args(&["accounting", "sync"]);
    let no_children = f.report()["metrics"].clone();
    assert_eq!(
        no_children["M55"]["by_profile"]["codex"]["value"],
        unavailable("no_children")
    );
    assert_eq!(no_children["M58"]["by_profile"]["codex"]["value"], "80");
    fixture_rollout(&f, "child", "spawned.jsonl");
    // A large request followed by a small request, with a duplicate response that must not count.
    let dir = f.home.join(".codex/sessions/2026/09/28");
    let parent = dir.join("rollout-parent.jsonl");
    let mut text = fs::read_to_string(&parent).unwrap();
    let at = jiff::Timestamp::from_millisecond(f.decided + 2)
        .unwrap()
        .to_string();
    let usage = json!({"timestamp":at,"type":"token_usage_record","payload":{"turn_id":"turn-1","response_id":"large","usage":{
        "input_tokens":300000,"cached_input_tokens":100000,"cache_write_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":40,"total_tokens":300100}}}).to_string()+"\n";
    text.push_str(&usage);
    text.push_str(&usage);
    // The published boundary is strict: exactly 272000 is not above it.
    text.push_str(&(json!({"timestamp":at,"type":"token_usage_record","payload":{"turn_id":"turn-1","response_id":"boundary","usage":{
        "input_tokens":272000,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":272010}}}).to_string()+"\n"));
    fs::write(parent, text).unwrap();
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let m = f.report()["metrics"].clone();
    assert_eq!(m["M55"]["by_profile"]["codex"]["value"], "30/572240");
    assert_eq!(
        m["M56"]["by_profile"]["codex"]["reasoning_share"]["value"],
        "40/135"
    );
    assert_eq!(
        m["M56"]["by_profile"]["codex"]["cache_share"]["value"],
        "100000/572105"
    );
    assert_eq!(m["M57"]["by_profile"]["codex"]["value"], "300000/572105");
    assert_eq!(m["M58"]["by_profile"]["codex"]["value"], "80");
    let query = f
        .cli_args(&["query", "--metric", "M55,M56,M57,M58", "--json"])
        .0;
    assert_eq!(
        query["results"][0]["detail"]["by_profile"]["codex"]["value"],
        "30/572240"
    );
    let registry = f.cli_args(&["metrics", "registry", "--json"]).0;
    for n in 51..=58 {
        let id = format!("M{n}");
        let metric = registry["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap();
        assert_eq!(metric["certification"]["status"], "fixture");
    }
    for n in 51..=58 {
        assert!(f.text(&["report"]).contains(&format!("M{n} ")));
    }
    f.cli_args(&["analytics", "refresh"]);
    assert_eq!(
        f.cli_args(&["analytics", "rebuild", "--verify"]).0["identical"],
        true
    );
    // A new setup with a new first request contributes a separate cell.
    f.readmit("other");
    fixture_rollout(&f, "other", "record.jsonl");
    // fixture_rollout uses the original worktree; replace it with the newly admitted one.
    let current: String = rusqlite::Connection::open(f.project.join(".state/state.db"))
        .unwrap()
        .query_row("SELECT id FROM attempts WHERE state='reserved'", [], |r| {
            r.get(0)
        })
        .unwrap();
    rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap().execute("INSERT INTO collector_bindings(attempt_id,revision,state,collector,execution_home,unix_ms,source) VALUES(?1,1,'active','codex',?2,?3,'apply_launch_started')",rusqlite::params![current,f.tmp.path().join("other-home").display().to_string(),unix_ms()]).unwrap();
    let path = dir.join("rollout-other.jsonl");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace(
            &jiff::Timestamp::from_millisecond(f.decided + 1)
                .unwrap()
                .to_string(),
            &jiff::Timestamp::from_millisecond(unix_ms())
                .unwrap()
                .to_string(),
        )
        .replace(
            &f.worktree(),
            &format!("{}/.state/worktrees/{current}/repo-00", f.project.display()),
        );
    // The 'other' profile owns its own execution home.
    let other_dir = f.tmp.path().join("other-home/.codex/sessions");
    fs::create_dir_all(&other_dir).unwrap();
    fs::write(other_dir.join("rollout-other.jsonl"), text).unwrap();
    fs::remove_file(path).unwrap();
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let other = f.report()["metrics"]["M56"]["by_profile"]["other"].clone();
    assert_eq!(other["reasoning_share"]["value"], "80/300");
    assert_eq!(other["cache_share"]["value"], "200/1000");
    assert_eq!(
        f.report()["metrics"]["M58"]["by_profile"]["other"]["value"],
        "1000"
    );
}

fn submission(f: &Fixture, db: &rusqlite::Connection, c: char, at: i64, policy: &str) {
    let id = c.to_string().repeat(64);
    let oid = c.to_string().repeat(40);
    let digest = format!("{:x}", Sha256::digest(policy.as_bytes()));
    db.execute("INSERT OR IGNORE INTO task_contracts(task_id,contract_revision,plan_revision,project_store,expected_head,repository,base_oid,object_format,memory_snapshot_id,route,raw_bytes,raw_digest,installed_seq)
        VALUES('work',1,NULL,'store',0,'/repo',?1,'sha1',NULL,'verify_only',x'61',?2,(SELECT max(sequence) FROM events))", rusqlite::params!["b".repeat(40), "c".repeat(64)]).unwrap();
    db.execute("INSERT INTO acceptance_policies(task_id,contract_revision,policy_id,body) VALUES('work',1,?1,?2)", rusqlite::params![id,policy]).unwrap();
    db.execute("INSERT INTO result_submissions(submission_id,project_store,idempotency_key,payload_digest,payload,task_id,contract_revision,contract_digest,attempt_id,repository,base_oid,candidate_oid,object_format,artifact_manifest,claimed_checks,created_unix_ms)
        VALUES(?1,'store',?1,?1,'{}','work',1,?1,?2,'/repo',?3,?4,'sha1','[]','[]',?5)",rusqlite::params![id,f.attempt,"b".repeat(40),oid,at]).unwrap();
    db.execute("INSERT INTO verification_runs(run_id,project_store,idempotency_key,payload_digest,submission_id,task_id,contract_revision,contract_digest,attempt_id,policy_id,policy_digest,
        commit_oid,tree_oid,object_format,memory_fence,isolation,argv,library_manifest,state,reason,exit_status,receipt_digest,store_device,store_inode,created_unix_ms)
        VALUES(?1,'store',?1,?1,?1,'work',1,?1,?2,?1,?3,?4,?4,'sha1',0,'linux-unshare-user-pid-mount-v1','[\"x\"]','[]','accepted',NULL,0,?1,1,1,?5)",rusqlite::params![id,f.attempt,digest,oid,at]).unwrap();
    db.execute("INSERT INTO verified_results(result_id,run_id,submission_id,commit_oid,tree_oid,object_format,policy_digest,receipt_digest,isolation,memory_fence,created_unix_ms)
        VALUES(?1,?1,?1,?2,?2,'sha1',?3,?1,'linux-unshare-user-pid-mount-v1',0,?4)",rusqlite::params![id,oid,digest,at]).unwrap();
}

#[test]
fn signed_policy_strength_and_weakening_health_warn_and_resolve() {
    let f = Fixture::new();
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let now = unix_ms();
    submission(
        &f,
        &db,
        '1',
        now - 1000,
        r#"{"version":1,"checks":["/usr/bin/git","grep","--quiet","--no-index","-e",".","--","tests/a.rs"]}"#,
    );
    submission(
        &f,
        &db,
        '2',
        now - 500,
        r#"{"version":1,"checks":["/usr/bin/cargo","test"]}"#,
    );
    submission(
        &f,
        &db,
        '3',
        now - 200,
        r#"{"version":2,"toolchain":"check","checks":["./check"]}"#,
    );
    drop(db);
    let m = f.report()["metrics"]["M51"].clone();
    assert_eq!(m["value"], "2/3");
    assert_eq!(m["file_presence_only"], 1);
    assert_eq!(m["by_profile"]["codex"]["value"], "2/3");
    let from = (now - 400).to_string();
    assert_eq!(
        f.cli_args(&["report", "--since", &from, "--json"]).0["metrics"]["M51"]["value"],
        "1/1"
    );
    f.cli("collect");
    assert_eq!(
        f.report()["metrics"]["M54"]["value"],
        unavailable("no_observed_submissions")
    );
    let side = f.sidecar();
    for (id, weakening) in [('1', "clear"), ('2', "flagged"), ('3', "clear")] {
        let id = id.to_string().repeat(64);
        side.execute("INSERT INTO proxy_signals(kind,task_id,submission_id,attempt_id,run_id,policy_digest,base_oid,candidate_oid,ci_state,verified_unix_ms,
            tests_added_lines,tests_deleted_lines,tests_binary_files,weakening,weakening_reason,weakening_rule,source_trust,observed_unix_ms)
            VALUES('first_candidate_ci',?1,?2,?3,?2,?2,?4,?4,'accepted',?5,0,2,0,?6,NULL,'tests-net-removal.v1','proxy_observed',?5)",
            rusqlite::params![format!("t{id}"),id,f.attempt,"b".repeat(40),now,weakening]).unwrap();
    }
    drop(side);
    assert_eq!(f.report()["metrics"]["M54"]["value"], "1/3");
    let health = f.cli_args(&["health", "evaluate", "--json"]).0;
    let state = health["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["rule"] == "test_weakening")
        .unwrap();
    assert_eq!(state["state"], "warn");
    let alerts = f.cli_args(&["health", "alerts", "--json"]).0;
    assert!(
        alerts["open"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["rule"] == "test_weakening")
    );
    // Move the flagged fixture outside the 24-hour window using the fixture's timestamp, never wall-clock waits.
    let side = f.sidecar();
    side.execute(
        "UPDATE proxy_signals SET observed_unix_ms=?1 WHERE weakening='flagged'",
        [now - 2 * 86400000],
    )
    .unwrap();
    drop(side);
    let health = f.cli_args(&["health", "evaluate", "--json"]).0;
    assert_eq!(
        health["states"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["rule"] == "test_weakening")
            .unwrap()["state"],
        "ok"
    );
    let alerts = f
        .cli_args(&["health", "alerts", "--since", "0", "--json"])
        .0;
    assert!(
        !alerts["open"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["rule"] == "test_weakening")
    );
    assert!(
        alerts["recent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["rule"] == "test_weakening")
    );
}

#[test]
fn reviewed_submission_density_and_review_task_spend_use_unique_findings() {
    let f = Fixture::new();
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let now = unix_ms();
    submission(
        &f,
        &db,
        '1',
        now - 1000,
        r#"{"version":1,"checks":["/usr/bin/cargo","test"]}"#,
    );
    for id in ["review-one", "review-two"] {
        db.execute(
            "INSERT INTO tasks(id,revision,state,title) VALUES(?1,1,'running',?1)",
            [id],
        )
        .unwrap();
        db.execute("INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed) VALUES(?1,?1,1,'completed',?1,1)",[id]).unwrap();
        db.execute("INSERT INTO memory_snapshots(id,task_id,task_revision,profile_name,profile_digest,selection_policy_version,estimator,sequence,required_bytes,optional_bytes,budget_bytes,omitted_optional_count,manifest_hash,scope_digest)
            VALUES(?1,?1,1,'codex',?2,1,'bytes',1,0,0,0,0,?2,?2)",rusqlite::params![id,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO operations(id,task_id,kind,target,payload_version,payload,payload_hash,expected_revision,due_unix_ms,idempotency_key) VALUES(?1,?1,'runtime.launch','binding',1,'{}',?2,1,0,?1)",rusqlite::params![id,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO dispatch_decisions(attempt_id,task_id,task_revision,contract_revision,chosen_configuration_id,eligible,chooser_kind,chooser_principal,reason_codes,decided_unix_ms)
            SELECT ?1,?1,1,NULL,chosen_configuration_id,'[\"x\"]','operator','operator:cli','[\"x\"]',?2 FROM dispatch_decisions WHERE attempt_id=?3",rusqlite::params![id,now-1,f.attempt]).unwrap();
        let inputs=json!({"inputs":{"version":2,"effective_profile":{"kind":"codex","name":"reviewer","execution_home":f.home.display().to_string()}}}).to_string();
        db.execute("INSERT INTO attempt_inputs(attempt_id,operation_id,payload,payload_hash) VALUES(?1,?1,?2,?3)",rusqlite::params![id,inputs,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO collector_bindings(attempt_id,revision,state,collector,execution_home,unix_ms,source) VALUES(?1,1,'active','codex',?2,?3,'apply_launch_started')",rusqlite::params![id,f.home.display().to_string(),now]).unwrap();
    }
    drop(db);
    let sub = "1".repeat(64);
    for (n, (attempt, kind)) in [("review-one", "code"), ("review-two", "skeptical")]
        .into_iter()
        .enumerate()
    {
        let op = f
            .cli_args(&[
                "review",
                "open",
                &sub,
                "--kind",
                kind,
                "--protocol",
                "review-protocol.v1",
            ])
            .0["opportunity"]["opportunity_id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.cli_args(&["review", "assign", &op, "--reviewer", "codex"]);
        let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
        db.execute("INSERT INTO review_briefs(snapshot_id,opportunity_id,task_id,brief_schema,brief_digest,prior_disclosure,principal,recorded_unix_ms) VALUES(?1,?2,?1,'review_brief.v1',?3,'withheld','operator:cli',?4)",rusqlite::params![attempt,op,format!("sha256:{}","a".repeat(64)),now]).unwrap();
        drop(db);
        let session = f
            .cli_args(&["review", "start", &op, "--attempt", attempt])
            .0["session"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let receipt = f.tmp.path().join(format!("{attempt}-receipt.json"));
        fs::write(&receipt,json!({"schema":"review_receipt.v1","session_id":session,"submission_id":sub,"candidate_oid":"1".repeat(40),"outcome":"completed","findings":[format!("finding:report-{n}")],"evidence":[]}).to_string()).unwrap();
        f.cli_args(&[
            "review",
            "complete",
            "--input-file",
            receipt.to_str().unwrap(),
        ]);
        let dir = f.home.join(".codex/sessions");
        fs::create_dir_all(&dir).unwrap();
        let at = jiff::Timestamp::from_millisecond(now).unwrap().to_string();
        let (input, output, model) = if n == 0 {
            (1000, 500, "gpt-5.5")
        } else {
            (2000, 1000, "gpt-5.5-mini")
        };
        let rows = [
            json!({"timestamp":at,"type":"session_meta","payload":{"id":format!("00000000-0000-4000-8000-0000000ab00{n}"),"timestamp":at,"cwd":format!("{}/.state/worktrees/{attempt}/repo-00",f.project.display()),"originator":"codex_exec","cli_version":"0.154.0","source":"exec"}}),
            json!({"timestamp":at,"type":"turn_context","payload":{"turn_id":"turn-1","model":model}}),
            json!({"timestamp":at,"type":"token_usage_record","payload":{"turn_id":"turn-1","response_id":"response","usage":{"input_tokens":input,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":output,"reasoning_output_tokens":0,"total_tokens":input+output}}}),
        ];
        fs::write(
            dir.join(format!("rollout-{attempt}.jsonl")),
            rows.iter()
                .map(|r| r.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
    }
    // Two reports of the same defect: owner validation mints one unique root.
    f.cli_args(&[
        "review",
        "findings",
        "validate",
        "1",
        "--new",
        "--severity",
        "high",
        "--evidence",
        &format!("sha256:{}", "a".repeat(64)),
    ]);
    let finding =
        f.cli_args(&["review", "findings", "show"]).0["findings"]["findings"][0]["finding_id"]
            .as_str()
            .unwrap()
            .to_owned();
    f.cli_args(&["review", "findings", "duplicate", "2", "--of", &finding]);
    let m = f.report()["metrics"].clone();
    assert_eq!(m["M52"]["value"], "1/1");
    assert_eq!(
        m["M52"]["by_profile"]["codex"]["by_severity"]["high"]["value"],
        "1/1"
    );
    assert_eq!(
        m["M52"]["by_profile"]["codex"]["by_severity"]["low"]["value"],
        "0/1"
    );
    assert_eq!(m["M53"]["value"], unavailable("collection_not_run"));
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(
        f.report()["metrics"]["M53"]["value"],
        unavailable("not_priced")
    );
    for (file, boundary) in [("rates-v1.json", "4102444800000"), ("rates-eur.json", "0")] {
        let card = f.tmp.path().join(file);
        fs::write(
            &card,
            fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/telemetry/accounting")
                    .join(file),
            )
            .unwrap()
            .replace("@BOUNDARY@", boundary),
        )
        .unwrap();
        f.cli_args(&["accounting", "import-rate-card", card.to_str().unwrap()]);
        if file == "rates-v1.json" {
            f.cli_args(&["accounting", "reprice"]);
            let partial = f.report()["metrics"]["M53"].clone();
            assert_eq!(
                partial["by_currency"]["USD"]["value"],
                json!({"status":"partial","reason":"review_cost_incomplete","priced_amount":"0.004","denominator":1})
            );
            assert!(f.text(&["report"]).contains("M53 cost_per_validated_finding currency=USD partial 0.004/1 (review_cost_incomplete)"));
        }
    }
    f.cli_args(&["accounting", "reprice"]);
    let cost = f.report()["metrics"]["M53"].clone();
    assert_eq!(cost["by_currency"]["USD"]["value"], "0.004/1");
    assert_eq!(cost["by_currency"]["EUR"]["value"], "0.006/1");
    assert_eq!(cost["attempts_without_usage"], 0);
    // Correcting validation removes the unique denominator without removing observed spend.
    f.cli_args(&["review", "findings", "reset", "1"]);
    let cost = f.report()["metrics"]["M53"].clone();
    assert_eq!(cost["value"], Value::Null);
    assert_eq!(cost["reason"], "empty_denominator");
    assert_eq!(cost["by_currency"]["USD"]["value"], Value::Null);
}

#[test]
fn first_request_median_uses_two_attempts_in_one_setup() {
    let f = Fixture::new();
    fixture_rollout(&f, "first", "parent.jsonl");
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    f.readmit("codex");
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let current: String = db
        .query_row("SELECT id FROM attempts WHERE state='reserved'", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute("INSERT INTO collector_bindings(attempt_id,revision,state,collector,execution_home,unix_ms,source) VALUES(?1,1,'active','codex',?2,?3,'apply_launch_started')",
        rusqlite::params![current, f.home.display().to_string(), unix_ms()]).unwrap();
    drop(db);
    let original = f
        .home
        .join(".codex/sessions/2026/09/28/rollout-first.jsonl");
    let mut rows: Vec<Value> = fs::read_to_string(original)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let at = jiff::Timestamp::from_millisecond(unix_ms())
        .unwrap()
        .to_string();
    for row in &mut rows {
        row["timestamp"] = json!(at);
    }
    rows[0]["payload"]["id"] = json!(SID);
    rows[0]["payload"]["timestamp"] = json!(at);
    rows[0]["payload"]["cwd"] = json!(format!(
        "{}/.state/worktrees/{current}/repo-00",
        f.project.display()
    ));
    rows[2]["payload"]["usage"]["input_tokens"] = json!(81);
    rows[2]["payload"]["usage"]["total_tokens"] = json!(101);
    fs::write(
        f.home
            .join(".codex/sessions/2026/09/28/rollout-second.jsonl"),
        rows.iter()
            .map(|r| r.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let median = f.report()["metrics"]["M58"]["by_profile"]["codex"].clone();
    assert_eq!(median["value"], "80.5");
    assert_eq!(median["samples"], 2);
}

#[test]
fn policy_classification_counts_only_executed_checks_and_exposes_missing_evidence() {
    let f = Fixture::new();
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    for (id, policy) in [
        (
            '1',
            json!({"version":1,"checks":["/usr/bin/test","-s","tests/a"]}),
        ),
        (
            '2',
            json!({"version":2,"checks":["/usr/bin/test","-s","tests/a"],"named_checks":{"unused":["/usr/bin/cargo","test"]}}),
        ),
        (
            '3',
            json!({"version":2,"checks":["/usr/bin/test","-s","tests/a"],"named_checks":{"suite":["/usr/bin/cargo","test"]},"stress":{"checks":["suite"],"repetitions":1,"concurrency":1}}),
        ),
    ] {
        submission(&f, &db, id, unix_ms(), &policy.to_string());
    }
    drop(db);
    let strength = f.report()["metrics"]["M51"].clone();
    assert_eq!(strength["value"], "1/3");
    assert_eq!(strength["file_presence_only"], 2);
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    submission(&f, &db, '4', unix_ms(), "malformed-policy-fixture");
    drop(db);
    let unknown = f.report()["metrics"]["M51"].clone();
    assert_eq!(
        unknown["value"],
        unavailable("acceptance_policy_unavailable")
    );
    assert_eq!(unknown["policy_unavailable"], 1);
    assert_eq!(unknown["denominator"], 4);
}
