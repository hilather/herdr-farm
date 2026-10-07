//! MET-REWORK-1 CLI workflow: real collection, pricing, verification and review metadata.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
mod support;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use support::telemetry::*;
fn submission(
    task: &str,
    attempt: &str,
    db: &rusqlite::Connection,
    c: char,
    at: i64,
    policy: &str,
) {
    let id = c.to_string().repeat(64);
    let oid = c.to_string().repeat(40);
    let digest = format!("{:x}", Sha256::digest(policy.as_bytes()));
    db.execute(&"INSERT OR IGNORE INTO task_contracts(task_id,contract_revision,plan_revision,project_store,expected_head,repository,base_oid,object_format,memory_snapshot_id,route,raw_bytes,raw_digest,installed_seq)
        VALUES('work',1,NULL,'store',0,'/repo',?1,'sha1',NULL,'verify_only',x'61',?2,(SELECT max(sequence) FROM events))".replace("'work'", &format!("'{task}'")), rusqlite::params!["b".repeat(40), "c".repeat(64)]).unwrap();
    db.execute(&"INSERT INTO acceptance_policies(task_id,contract_revision,policy_id,body) VALUES('work',1,?1,?2)".replace("'work'", &format!("'{task}'")), rusqlite::params![id,policy]).unwrap();
    db.execute(&"INSERT INTO result_submissions(submission_id,project_store,idempotency_key,payload_digest,payload,task_id,contract_revision,contract_digest,attempt_id,repository,base_oid,candidate_oid,object_format,artifact_manifest,claimed_checks,created_unix_ms)
        VALUES(?1,'store',?1,?1,'{}','work',1,?1,?2,'/repo',?3,?4,'sha1','[]','[]',?5)".replace("'work'", &format!("'{task}'")),rusqlite::params![id,attempt,"b".repeat(40),oid,at]).unwrap();
    db.execute(&"INSERT INTO verification_runs(run_id,project_store,idempotency_key,payload_digest,submission_id,task_id,contract_revision,contract_digest,attempt_id,policy_id,policy_digest,
        commit_oid,tree_oid,object_format,memory_fence,isolation,argv,library_manifest,state,reason,exit_status,receipt_digest,store_device,store_inode,created_unix_ms)
        VALUES(?1,'store',?1,?1,?1,'work',1,?1,?2,?1,?3,?4,?4,'sha1',0,'linux-unshare-user-pid-mount-v1','[\"x\"]','[]','accepted',NULL,0,?1,1,1,?5)".replace("'work'", &format!("'{task}'")),rusqlite::params![id,attempt,digest,oid,at]).unwrap();
    db.execute(&"INSERT INTO verified_results(result_id,run_id,submission_id,commit_oid,tree_oid,object_format,policy_digest,receipt_digest,isolation,memory_fence,created_unix_ms)
        VALUES(?1,?1,?1,?2,?2,'sha1',?3,?1,'linux-unshare-user-pid-mount-v1',0,?4)".replace("'work'", &format!("'{task}'")),rusqlite::params![id,oid,digest,at]).unwrap();
}

#[test]
fn two_work_items_rework_cost_escape_and_lead_time() {
    let f = Fixture::with_lineage("item-one");
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let now = unix_ms() - 10000;
    db.execute(
        "UPDATE attempts SET state='completed',termination_observed=1 WHERE id=?1",
        [&f.attempt],
    )
    .unwrap();
    let mut ids = vec![("work".to_owned(), f.attempt.clone(), 0)];
    for (task, role, work, state, offset, supersedes) in [
        ("fix-one", "fix", "item-one", "failed", 1000, None),
        (
            "fix-two",
            "fix",
            "item-one",
            "completed",
            2000,
            Some("work"),
        ),
        ("clean", "build", "item-two", "completed", 4000, None),
        ("review-one", "review", "item-one", "completed", 5000, None),
        ("review-two", "review", "item-one", "completed", 6000, None),
        ("recheck", "recheck", "item-one", "completed", 7000, None),
    ] {
        db.execute(
            "INSERT INTO tasks(id,revision,state,title) VALUES(?1,1,'running',?1)",
            [task],
        )
        .unwrap();
        // Public lineage entry point; insert before the first attempt, as launch does.
        herdr_farm::store::SqliteStore::open(&f.project.join(".state/state.db"))
            .unwrap()
            .prepare_task_lineage(task, work, role, supersedes)
            .unwrap();
        db.execute("INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed) VALUES(?1,?1,1,?2,?1,1)",rusqlite::params![task,state]).unwrap();
        db.execute("INSERT INTO operations(id,task_id,kind,target,payload_version,payload,payload_hash,expected_revision,due_unix_ms,idempotency_key) SELECT ?1,?1,kind,target,payload_version,payload,payload_hash,expected_revision,due_unix_ms,?1 FROM operations WHERE id=(SELECT operation_id FROM attempt_inputs WHERE attempt_id=?2)",rusqlite::params![task,f.attempt]).unwrap();
        db.execute("INSERT INTO attempt_inputs(attempt_id,operation_id,payload,payload_hash) SELECT ?1,?1,payload,payload_hash FROM attempt_inputs WHERE attempt_id=?2",rusqlite::params![task,f.attempt]).unwrap();
        db.execute("INSERT INTO collector_bindings(attempt_id,revision,state,collector,execution_home,unix_ms,source) VALUES(?1,1,'active','codex',?2,?3,'apply_launch_started')",rusqlite::params![task,f.home.display().to_string(),now]).unwrap();
        db.execute("INSERT INTO dispatch_decisions(attempt_id,task_id,task_revision,contract_revision,chosen_configuration_id,eligible,chooser_kind,chooser_principal,reason_codes,decided_unix_ms)
            SELECT ?1,?1,1,NULL,chosen_configuration_id,eligible,chooser_kind,chooser_principal,reason_codes,decided_unix_ms FROM dispatch_decisions WHERE attempt_id=?2",rusqlite::params![task,f.attempt]).unwrap();
        ids.push((task.into(), task.into(), offset));
    }
    db.execute("INSERT INTO tasks(id,revision,state,title) VALUES('never-launched',1,'cancelled','never-launched')", []).unwrap();
    herdr_farm::store::SqliteStore::open(&f.project.join(".state/state.db"))
        .unwrap()
        .prepare_task_lineage("never-launched", "item-one", "fix", None)
        .unwrap();
    db.execute("INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed) VALUES('never-launched','never-launched',1,'cancelled','never-launched',1)", []).unwrap();
    db.execute("INSERT INTO attempt_lifecycle(attempt_id,state,attempt_revision,unix_ms,source) VALUES('never-launched','cancelled',1,?1,'fixture')", [now + 500]).unwrap();
    for (_, attempt, offset) in &ids {
        for (state, at) in [
            ("launching", now + offset),
            ("completed", now + offset + 500),
        ] {
            let state = if attempt == "fix-one" && state == "completed" {
                "failed"
            } else {
                state
            };
            db.execute("INSERT INTO attempt_lifecycle(attempt_id,state,attempt_revision,unix_ms,source) VALUES(?1,?2,1,?3,'fixture')",rusqlite::params![attempt,state,at]).unwrap();
        }
    }
    submission(
        "fix-two",
        "fix-two",
        &db,
        '1',
        now + 2500,
        r#"{"version":1,"checks":["/usr/bin/cargo","test"]}"#,
    );
    submission(
        "clean",
        "clean",
        &db,
        '2',
        now + 4500,
        r#"{"version":1,"checks":["/usr/bin/cargo","test"]}"#,
    );
    // Actual collector input and rate-card CLI produce seven equal published-rate costs.
    for (n, (_, attempt, _offset)) in ids.iter().enumerate() {
        let dir = f.home.join(".codex/sessions");
        fs::create_dir_all(&dir).unwrap();
        let at = jiff::Timestamp::from_millisecond(unix_ms())
            .unwrap()
            .to_string();
        let rows = [
            json!({"timestamp":at,"type":"session_meta","payload":{"id":format!("00000000-0000-4000-8000-00000000000{n}"),"timestamp":at,"cwd":format!("{}/.state/worktrees/{attempt}/repo-00",f.project.display()),"originator":"codex_exec","cli_version":"0.154.0","source":"exec"}}),
            json!({"timestamp":at,"type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-5.5"}}),
            json!({"timestamp":at,"type":"token_usage_record","payload":{"turn_id":"turn-1","response_id":"response","usage":{"input_tokens":1000,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1500}}}),
        ];
        fs::write(
            dir.join(format!("rollout-{n}.jsonl")),
            rows.iter()
                .map(|r| r.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
    }
    drop(db);
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let card = f.tmp.path().join("rates.json");
    fs::write(
        &card,
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/telemetry/accounting/rates-v1.json"),
        )
        .unwrap()
        .replace("@BOUNDARY@", "4102444800000"),
    )
    .unwrap();
    f.cli_args(&["accounting", "import-rate-card", card.to_str().unwrap()]);
    f.cli_args(&["accounting", "reprice"]);
    let sub = "1".repeat(64);
    let op = f
        .cli_args(&[
            "review",
            "open",
            &sub,
            "--kind",
            "code",
            "--protocol",
            "review-protocol.v1",
        ])
        .0["opportunity"]["opportunity_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.cli_args(&["review", "assign", &op, "--reviewer", "codex"]);
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    db.execute("INSERT INTO memory_snapshots(id,task_id,task_revision,profile_name,profile_digest,selection_policy_version,estimator,sequence,required_bytes,optional_bytes,budget_bytes,omitted_optional_count,manifest_hash,scope_digest) VALUES('review-one','review-one',1,'codex',?1,1,'bytes',1,0,0,0,0,?1,?1)",["a".repeat(64)]).unwrap();
    db.execute("INSERT INTO review_briefs(snapshot_id,opportunity_id,task_id,brief_schema,brief_digest,prior_disclosure,principal,recorded_unix_ms) VALUES('review-one',?1,'review-one','review_brief.v1',?2,'withheld','operator:cli',?3)",rusqlite::params![op,format!("sha256:{}","a".repeat(64)),now]).unwrap();
    drop(db);
    let session = f
        .cli_args(&["review", "start", &op, "--attempt", "review-one"])
        .0["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let receipt = f.tmp.path().join("receipt.json");
    fs::write(&receipt,json!({"schema":"review_receipt.v1","session_id":session,"submission_id":sub,"candidate_oid":"1".repeat(40),"outcome":"completed","findings":["finding:escaped"],"evidence":[]}).to_string()).unwrap();
    f.cli_args(&[
        "review",
        "complete",
        "--input-file",
        receipt.to_str().unwrap(),
    ]);
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
    let m = f.report()["metrics"].clone();
    assert_eq!(m["M65"]["value"], json!({"median":1,"p90":2,"max":2}));
    assert_eq!(m["M65"]["by_profile"]["codex"]["samples"], 2);
    assert_eq!(m["M66"]["by_currency"]["USD"]["value"], "0.016/0.028", "metric={} cost={}", m["M66"], f.cli_args(&["accounting", "cost", "--json"]).0);
    assert_eq!(
        m["M66"]["by_profile"]["codex"]["by_currency"]["USD"]["value"],
        "0.016/0.028"
    );
    assert_eq!(m["M66"]["attempts_without_complete_cost"], 0);
    assert_eq!(m["M69"]["attempts_without_complete_cost"], 0);
    assert_eq!(m["M67"]["value"], "1/2");
    assert_eq!(m["M67"]["count"], 1);
    assert_eq!(
        m["M68"]["value"],
        json!({"median":4000,"p90":7500,"max":7500})
    );
    assert_eq!(m["M69"]["by_currency"]["USD"]["value"], "0.008/0.028");
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    db.execute("INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed) VALUES('never-launched','clean',1,'cancelled','never-launched',1)", []).unwrap();
    db.execute("INSERT INTO attempt_lifecycle(attempt_id,state,attempt_revision,unix_ms,source) VALUES('never-launched','cancelled',1,?1,'fixture')", [now+4500]).unwrap();
    let no_launch = f.report()["metrics"].clone();
    for id in ["M66","M69"] {
        assert_eq!(no_launch[id]["attempts_without_complete_cost"],0);
        assert_ne!(no_launch[id]["by_currency"]["USD"]["status"],"partial");
    }
    // Accepted work cancelled during cleanup stays in total spend, outside waste.
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    db.execute("UPDATE attempts SET state='cancelled' WHERE id='clean'", []).unwrap();
    assert_eq!(f.report()["metrics"]["M69"]["by_currency"]["USD"]["value"], "0.008/0.028");
    let fleet = f.cli_args(&["accounting", "fleet", "--json"]).0;
    assert_eq!(fleet["metrics"]["M37"]["buckets"]["accepted_then_cancelled"]["attempts"], 1);
    assert_eq!(fleet["metrics"]["M37"]["buckets"]["accepted_then_cancelled"]["estimate"]["amount"], "0.004");
    assert_eq!(fleet["metrics"]["M37"]["buckets"]["unexplained_abandonment"]["attempts"], 0);
    let query = f
        .cli_args(&["query", "--metric", "M65,M66,M67,M68,M69", "--json"])
        .0;
    for (n, row) in query["results"].as_array().unwrap().iter().enumerate() {
        assert_eq!(row["detail"]["value"], m[format!("M{}", 65 + n)]["value"]);
    }
    for n in 65..=69 {
        assert!(f.text(&["report"]).contains(&format!("M{n} ")));
    }
    let text = f.text(&["report"]);
    assert!(text.contains("M66 rework_cost_share profile=codex currency=USD 0.016/0.028"));
    assert!(text.contains("M67 escaped_defects count=1 1/2"));
    f.cli_args(&["analytics", "refresh"]);
    assert_eq!(
        f.cli_args(&["analytics", "rebuild", "--verify"]).0["identical"],
        true
    );
    // The later clean work item is the sole member of this launch window.
    let window = f
        .cli_args(&["report", "--since", &(now + 4000).to_string(), "--json"])
        .0;
    assert_eq!(window["metrics"]["M65"]["value"]["median"], 0);
    assert_eq!(
        window["metrics"]["M66"]["by_currency"]["USD"]["value"],
        "0/0.004"
    );
    assert_eq!(window["metrics"]["M68"]["value"]["median"], 500);
    // Owner triage correction removes the escaped defect from the current count.
    f.cli_args(&["review", "findings", "reset", "1"]);
    assert_eq!(f.report()["metrics"]["M67"]["value"], "0/2");
    // One passing policy cannot hide pending evidence on another required policy.
    db.execute("INSERT INTO acceptance_policies(task_id,contract_revision,policy_id,body) VALUES('clean',1,'second','opaque')", []).unwrap();
    assert_eq!(f.report()["metrics"]["M69"]["by_currency"]["USD"]["value"], "0.012/0.028");
    let fleet = f.cli_args(&["accounting", "fleet", "--json"]).0;
    assert_eq!(fleet["metrics"]["M37"]["buckets"]["accepted_then_cancelled"]["attempts"], 0);
    assert_eq!(fleet["metrics"]["M37"]["buckets"]["unexplained_abandonment"], json!({"attempts":1,"estimate":{"status":"complete","currency":"USD","amount":"0.004"}}));
}

#[test]
fn missing_lineage_is_excluded_and_unavailable() {
    let f = Fixture::new();
    let report = f.report();
    let registry = f.cli_args(&["metrics", "registry", "--json"]).0;
    let query = f
        .cli_args(&["query", "--metric", "M65,M66,M67,M68,M69", "--json"])
        .0;
    for n in 65..=69 {
        let id = format!("M{n}");
        assert_eq!(
            report["metrics"][&id]["value"],
            json!({"status":"unavailable","reason":"lineage_not_recorded"})
        );
        assert_eq!(
            report["metrics"][&id]["excluded"]["lineage_not_recorded"],
            1
        );
        let declared = registry["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap();
        assert_eq!(declared["certification"]["status"], "fixture");
        assert_eq!(
            query["results"][(n - 65) as usize]["reason"],
            "lineage_not_recorded"
        );
    }
}
