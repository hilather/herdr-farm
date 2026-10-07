#![cfg(feature = "state-store")]

use herdr_farm::{store::SqliteStore, telemetry::{accounting::attention, sidecar}};
use std::{collections::BTreeSet, time::{Duration, Instant}};

// Exercise the public lifecycle telemetry entry point against a real isolated
// canonical store and sidecar. A remote launch records an observable gap without
// needing a local Herdr socket or agent executable.
fn lifecycle_writer_contention(release_after: Option<Duration>) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("demo");
    std::fs::create_dir_all(project.join(".state")).unwrap();
    drop(SqliteStore::create(&project.join(".state/state.db")).unwrap());
    let canonical = rusqlite::Connection::open(project.join(".state/state.db")).unwrap();
    canonical.execute_batch("PRAGMA foreign_keys=OFF;
        INSERT INTO tasks(id,revision,state,title) VALUES('work',1,'running','work');
        INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed)
            VALUES('attempt','work',2,'running','reservation',0);").unwrap();
    canonical.execute("INSERT INTO events(kind,entity,revision,payload_version,payload)
        VALUES('runtime.launch_started','launch',1,1,?1)", [serde_json::json!({
            "attempt": "attempt", "route": {"machine": "remote"},
            "observed_unix_ms": jiff::Timestamp::now().as_millisecond()
        }).to_string()]).unwrap();
    drop(canonical);
    drop(sidecar::open(&project, true).unwrap());
    let mut writer = rusqlite::Connection::open(sidecar::path(&project)).unwrap();
    let lock = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let ids = BTreeSet::from(["attempt".to_owned()]);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let sample_project = project.clone();
    let sampler = std::thread::spawn(move || {
        let start = Instant::now();
        started_tx.send(()).unwrap();
        attention::observe_selected(&sample_project, &ids);
        done_tx.send(start.elapsed()).unwrap();
    });
    started_rx.recv().unwrap();
    let elapsed = if let Some(delay) = release_after {
        assert!(done_rx.recv_timeout(delay).is_err(), "sample must wait for the writer");
        lock.commit().unwrap();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap()
    } else {
        // Keep the write lock until the caller returns: waiting for release would
        // deadlock, and the hook must still return best-effort within its budget.
        let elapsed = done_rx.recv_timeout(Duration::from_millis(500)).unwrap();
        lock.commit().unwrap();
        elapsed
    };
    sampler.join().unwrap();
    assert!(elapsed < Duration::from_millis(500), "probe took {elapsed:?}");
    let rows: Vec<(Option<String>, Option<String>)> = writer.prepare(
        "SELECT state,gap FROM attention_samples WHERE attempt_id='attempt'").unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    if release_after.is_some() {
        assert_eq!(rows, vec![(None, Some("remote_route".to_owned()))]);
    } else {
        assert!(elapsed >= Duration::from_millis(200), "writer wait took {elapsed:?}");
        assert!(rows.is_empty(), "expired write must remain best-effort: {rows:?}");
    }
}

#[test]
fn lifecycle_sample_survives_a_short_sidecar_writer() {
    lifecycle_writer_contention(Some(Duration::from_millis(100)));
}

#[test]
fn lifecycle_sample_bounds_a_persistent_sidecar_writer() {
    lifecycle_writer_contention(None);
}
