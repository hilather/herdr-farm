//! CLI workflows: failed launch retries, cancellation, persisted timing and missing history.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
mod support;
use serde_json::{Value, json};
use std::process::Command;
use support::telemetry::*;
fn product(f: &Fixture, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .env_clear()
        .env("HOME", f.tmp.path().join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", f.scale)
        .args(["--root", f.root.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}
fn query(f: &Fixture, id: &str) -> Value {
    f.cli_args(&["query", "--metric", id, "--json"]).0["results"][0].clone()
}
#[test]
fn retry_and_cancel_are_counted_from_real_commands_and_closed_lifecycle() {
    let f = Fixture::with_lineage("launch-item");
    assert_eq!(
        query(&f, "M90")["reason"],
        "launch_invocations_not_collected"
    );
    assert_eq!(query(&f, "M92")["reason"], "ticker_errors_not_observed");
    assert_eq!(query(&f, "M94")["reason"], "no_submissions");
    assert_eq!(
        query(&f, "M93")["detail"]["value"][0]["value"]["reason"],
        "open_work_item"
    );
    f.cli("collect");
    for _ in 0..2 {
        let out = product(
            &f,
            &[
                "launch",
                "demo",
                "run",
                "--task",
                "work",
                "--profile",
                "missing",
                "--repository",
                f.tmp.path().to_str().unwrap(),
            ],
        );
        assert!(!out.status.success());
    }
    // Help is not a launch attempt.
    assert!(
        product(&f, &["launch", "demo", "run", "--help"])
            .status
            .success()
    );
    let m = query(&f, "M90");
    assert_eq!(m["value"], "0/2");
    assert_eq!(m["detail"]["by_task"]["work"]["tries_before_running"], 2);
    assert_eq!(m["detail"]["unattributed_invocations"], 0);
    let mut store =
        herdr_farm::store::SqliteStore::open(&f.project.join(".state/state.db")).unwrap();
    let snapshot = store.read_snapshot(None).unwrap();
    let revision = snapshot
        .attempts
        .iter()
        .find(|a| a.id.as_str() == f.attempt)
        .unwrap()
        .revision
        .to_string();
    let head = snapshot.head.to_string();
    drop(store);
    let out = product(
        &f,
        &[
            "task",
            "demo",
            "cancel-attempt",
            &f.attempt,
            "--expected-revision",
            &revision,
            "--expected-head",
            &head,
            "--reason",
            "fixture intervention",
        ],
    );
    // This isolated store fixture has no migration publication journal, so
    // CLI interventions are refused but remain attempts to intervene.
    assert!(!out.status.success());
    let mut store =
        herdr_farm::store::SqliteStore::open(&f.project.join(".state/state.db")).unwrap();
    store
        .cancel_attempt(
            &herdr_farm::domain::AttemptId::new(f.attempt.clone()).unwrap(),
            revision.parse().unwrap(),
            head.parse().unwrap(),
            "fixture intervention",
            unix_ms(),
        )
        .unwrap();
    drop(store);
    // An unsuccessful retry is a separate error but shares the first intervention.
    let out = product(
        &f,
        &[
            "task",
            "demo",
            "cancel-attempt",
            &f.attempt,
            "--expected-revision",
            &revision,
            "--expected-head",
            &head,
            "--reason",
            "fixture intervention",
        ],
    );
    assert!(!out.status.success());
    // Forced stops count; ordinary stops do not, even when no server exists.
    let _ = product(&f, &["launch", "demo", "stop", "--task", "work"]);
    let cancellations = query(&f, "M91");
    assert_eq!(cancellations["value"], 1);
    assert_eq!(cancellations["detail"]["breakdown"]["stuck"], 1);
    assert_eq!(cancellations["detail"]["errored_calls"], 2);
    let _ = product(&f, &["launch", "demo", "stop", "--task", "work", "--force"]);
    let m = query(&f, "M91");
    assert_eq!(m["value"], 1);
    assert_eq!(m["detail"]["breakdown"]["stuck"], 1);
    assert_eq!(m["detail"]["errored_calls"], 2);
    assert_eq!(m["detail"]["per_attempt"][&f.attempt], 1);
    assert_eq!(
        m["detail"]["per_day_utc"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_i64().unwrap())
            .sum::<i64>(),
        1
    );
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let elapsed:i64=db.query_row("SELECT (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=?1 AND state='cancelled')-(SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=?1 AND state='reserved')",[&f.attempt],|r|r.get(0)).unwrap();
    let m = query(&f, "M93");
    let item = &m["detail"]["value"][0]["value"];
    assert_eq!(item["total_elapsed_ms"], elapsed);
    assert_eq!(item["worker_running_ms"], 0);
    assert_eq!(
        item["launch_overhead_ms"]["reason"],
        "never_running_attempt"
    );
    assert_eq!(item["waiting_between_attempts_ms"], 0);
    assert_eq!(
        item["shares"]["launch_overhead"]["reason"],
        "never_running_attempt"
    );
    let m = query(&f, "M95");
    assert_eq!(m["value"]["candidates_proposed"], 0);
    assert_eq!(m["value"]["remember_sections_written"], 0);
    assert_eq!(
        m["detail"]["legacy_remember"]["reason"],
        "legacy_remember_not_observed"
    );
    let registry = f.cli_args(&["metrics", "registry", "--json"]).0;
    assert_eq!(registry["registry"], "analytics-registry.v12");
    for id in ["M90", "M91", "M92", "M93", "M94", "M95"] {
        assert_eq!(
            registry["metrics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == id)
                .unwrap()["certification"]["status"],
            "fixture"
        );
    }
    let report = f.cli_args(&["report", "--json"]).0;
    assert_eq!(report["metrics"]["M90"]["value"], json!("0/2"));
}

#[test]
fn ticker_passes_publish_metadata_and_disabled_collection_stays_unobserved() {
    let f = Fixture::reserved();
    f.cli("collect");
    std::fs::write(f.project.join("PROJECT.md"), "# Isolated ticker fixture\n").unwrap();
    let out = product(&f, &["ticker", "run", "--passes", "2"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let m = query(&f, "M92");
    assert_eq!(
        m["detail"]["observed_passes"].as_i64().unwrap()
            + m["detail"]["missing_passes"].as_i64().unwrap(),
        2
    );
    assert_eq!(m["detail"]["scope"], "ticker_root");
    let db = f.sidecar();
    let counts:(i64,i64)=db.query_row("SELECT count(*),sum(lock_contention+expired_inventory+ambiguous_outcome+permanent_failure+other) FROM operation_ticker_errors",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!((1..=2).contains(&counts.0));
    assert_eq!(m["detail"]["observed_passes"], counts.0);
    let total: i64 = m["value"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|v| v.as_object().unwrap().values())
        .map(|v| v.as_i64().unwrap())
        .sum();
    assert_eq!(total, counts.1);
    let g = Fixture::reserved();
    g.cli("collect");
    std::fs::write(g.project.join("PROJECT.md"), "# Isolated ticker fixture\n").unwrap();
    let out = Command::new(BIN)
        .env_clear()
        .env("HOME", g.tmp.path().join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", g.scale)
        .env("HERDR_FARM_TELEMETRY_COLLECT_SECS", "0")
        .args([
            "--root",
            g.root.to_str().unwrap(),
            "ticker",
            "run",
            "--passes",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(query(&g, "M92")["reason"], "ticker_errors_not_observed");
    assert_eq!(g.count("operation_ticker_errors"), 0);
}

#[test]
fn already_terminal_attempt_is_other_and_errors_remain_visible() {
    let f = Fixture::reserved();
    f.cancel_reserved();
    f.cli("collect");
    let out = product(&f, &[
        "task", "demo", "cancel-attempt", &f.attempt,
        "--expected-revision", "1", "--expected-head", "0",
        "--reason", "already exited",
    ]);
    assert!(!out.status.success());
    let m = query(&f, "M91");
    assert_eq!(m["value"], 0);
    assert_eq!(m["detail"]["breakdown"], json!({
        "stuck": 0, "cleanup_after_acceptance": 0, "other": 1
    }));
    assert_eq!(m["detail"]["errored_calls"], 1);
    assert_eq!(m["detail"]["per_attempt_classes"][&f.attempt]["other"], 1);
}
