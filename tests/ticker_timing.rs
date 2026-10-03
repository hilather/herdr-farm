#![cfg(all(target_os = "linux", feature = "state-store"))]
#![allow(clippy::disallowed_methods)]
//! Timing acceptance through the compiled CLI, without sessions or sockets.
use std::{
    fs,
    os::unix::fs::MetadataExt,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");
const SCALE: &str = include_str!("support/time-scale.txt");
struct Lab(tempfile::TempDir);
impl Lab {
    fn new() -> Self {
        let lab = Self(tempfile::tempdir().unwrap());
        assert!(
            lab.command(Some(SCALE.trim()))
                .args(["new", "--legacy", "demo"])
                .output()
                .unwrap()
                .status
                .success()
        );
        lab
    }
    fn command(&self, scale: Option<&str>) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.env_clear()
            .env("HOME", self.0.path())
            .env("PATH", "/usr/bin:/bin")
            .env("HERDR_BIN_PATH", "/bin/false")
            .arg("--root")
            .arg(self.0.path().join("root"));
        if let Some(scale) = scale {
            cmd.env("HERDR_FARM_TEST_TIME_SCALE", scale);
        }
        cmd
    }
    fn inode(&self) -> Option<u64> {
        fs::metadata(self.0.path().join("root/.ticker-metrics.json"))
            .ok()
            .map(|m| m.ino())
    }
}
struct Ticker(Child);
impl Drop for Ticker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn bounded_real_ticker_scales_passes_and_enforces_the_floor() {
    for (scale, minimum) in [
        (SCALE.trim(), Duration::from_millis(950)),
        ("0.000001", Duration::from_millis(950)),
    ] {
        let lab = Lab::new();
        let start = Instant::now();
        let mut ticker = Ticker(
            lab.command(Some(scale))
                .args(["ticker", "run", "--passes", "3"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        loop {
            if let Some(status) = ticker.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "bounded ticker did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            start.elapsed() >= minimum,
            "pass floor was bypassed at scale {scale}: {:?}",
            start.elapsed()
        );
        let log = fs::read_to_string(lab.0.path().join("root/.ticker.log")).unwrap();
        assert!(log.contains("completed 3 passes; draining shared executor"));
        assert!(lab.inode().is_some());
        let status = lab
            .command(Some(scale))
            .args(["ticker", "status"])
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(String::from_utf8_lossy(&status.stdout).contains("not running"));
    }
}

#[test]
fn unset_and_invalid_scales_keep_the_production_cadence_and_doctor_output() {
    let lab = Lab::new();
    for scale in [
        None,
        Some("invalid"),
        Some("0"),
        Some("-1"),
        Some("NaN"),
        Some("inf"),
        Some("2"),
    ] {
        let doctor = lab.command(scale).arg("doctor").output().unwrap();
        assert!(!String::from_utf8_lossy(&doctor.stdout).contains("test time scale active"));
        let bounded = lab
            .command(scale)
            .args(["ticker", "run", "--passes", "2"])
            .output()
            .unwrap();
        assert!(!bounded.status.success());
    }
    let doctor = lab
        .command(Some(SCALE.trim()))
        .arg("doctor")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&doctor.stdout);
    assert!(text.contains("test time scale active (0.02)"));
    assert!(text.contains("ticker pass 500 ms"));
    let mut ticker = Ticker(
        lab.command(None)
            .args(["ticker", "run"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while lab.inode().is_none() {
        assert!(ticker.0.try_wait().unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    let first = lab.inode();
    let entered = Instant::now();
    while lab.inode() == first {
        assert!(ticker.0.try_wait().unwrap().is_none());
        assert!(entered.elapsed() < Duration::from_secs(25));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        entered.elapsed() >= Duration::from_millis(14_500),
        "production 15-second pass changed"
    );
    // Stop through its public entry point; no process or root outside the lab is touched.
    let stop = lab.command(None).args(["ticker", "stop"]).output().unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
}

/// Separate processes exercise persisted store retry eligibility with matching
/// scales, including reopening and lease expiry through the public store API.
#[test]
fn durable_retry_deadlines_survive_process_restart() {
    for (scale, first, second) in [
        (None, 1000, 2000),
        (Some(SCALE.trim()), 20, 40),
        (Some("0.000001"), 20, 40),
    ] {
        let home = tempfile::tempdir().unwrap();
        for phase in ["write", "resume"] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .env_clear()
                .env("HOME", home.path())
                .env("PATH", "/usr/bin:/bin")
                .env("TEST_TIMING_STORE", home.path().join("state.db"))
                .env("TEST_TIMING_PHASE", phase)
                .env("TEST_TIMING_FIRST", first.to_string())
                .env("TEST_TIMING_SECOND", second.to_string())
                .args(["--exact", "durable_retry_store_child", "--nocapture"]);
            if let Some(scale) = scale {
                child.env("HERDR_FARM_TEST_TIME_SCALE", scale);
            }
            let out = child.output().unwrap();
            assert!(
                out.status.success(),
                "{phase}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

#[test]
fn durable_retry_store_child() {
    use herdr_farm::{
        domain::*,
        operations::{DeliveryState, Outcome},
        store::SqliteStore,
    };
    let Some(path) = std::env::var_os("TEST_TIMING_STORE") else {
        return;
    };
    let path = std::path::Path::new(&path);
    let first: i64 = std::env::var("TEST_TIMING_FIRST").unwrap().parse().unwrap();
    let second: i64 = std::env::var("TEST_TIMING_SECOND")
        .unwrap()
        .parse()
        .unwrap();
    let operation = OperationId::new("notification").unwrap();
    let now = 1_000_000;
    if std::env::var("TEST_TIMING_PHASE").unwrap() == "write" {
        let mut db = SqliteStore::create(path).unwrap();
        let head = db.read_snapshot(None).unwrap().head;
        let task = TaskId::new("task").unwrap();
        db.commit(Commit {
            expected_head: head,
            mutations: vec![
                Mutation::Task {
                    expected: None,
                    next: Task {
                        id: task.clone(),
                        revision: 1,
                        state: TaskState::Ready,
                        title: "deliver notification".into(),
                        active_attempt: None,
                    },
                },
                Mutation::Enqueue(Operation {
                    id: operation.clone(),
                    task: Some(task),
                    kind: "runtime.notification".into(),
                    target: "coordinator".into(),
                    payload_version: 1,
                    payload: serde_json::json!({}),
                    expected_revision: 1,
                    due_unix_ms: now,
                    idempotency_key: "notification".into(),
                }),
            ],
        })
        .unwrap();
        let claim = db
            .claim_operation(&operation, 1, "first process", now, 60_000)
            .unwrap();
        assert_eq!(
            claim.lease_until_ms,
            now + 60_000,
            "execution lease must retain its budget"
        );
        let retry = db
            .finish_operation(
                &claim,
                Outcome::Retryable {
                    no_effect_evidence: "local delivery refused before effect".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(retry.next_due_ms, now + first);
        assert_eq!(retry.state, DeliveryState::Pending);
        assert!(
            db.claim_operation(
                &operation,
                retry.revision,
                "early retry",
                retry.next_due_ms - 1,
                60_000
            )
            .is_err()
        );
    } else {
        let mut db = SqliteStore::open(path).unwrap();
        let retry = db
            .read_snapshot(None)
            .unwrap()
            .deliveries
            .into_iter()
            .find(|d| d.operation == operation)
            .unwrap();
        assert_eq!(retry.next_due_ms, now + first);
        let claim = db
            .claim_operation(
                &operation,
                retry.revision,
                "second process",
                retry.next_due_ms,
                60_000,
            )
            .unwrap();
        let retried = db
            .finish_operation(
                &claim,
                Outcome::Retryable {
                    no_effect_evidence: "still no effect".into(),
                },
                retry.next_due_ms,
            )
            .unwrap();
        assert_eq!(retried.next_due_ms, retry.next_due_ms + second);
        let claim = db
            .claim_operation(
                &operation,
                retried.revision,
                "crashed process",
                retried.next_due_ms,
                60_000,
            )
            .unwrap();
        assert_eq!(db.expire_claims(claim.lease_until_ms - 1).unwrap(), 0);
        assert_eq!(db.expire_claims(claim.lease_until_ms).unwrap(), 1);
        let expired = db
            .read_snapshot(None)
            .unwrap()
            .deliveries
            .into_iter()
            .find(|d| d.operation == operation)
            .unwrap();
        assert_eq!(expired.state, DeliveryState::Ambiguous);
        assert!(
            db.claim_operation(
                &operation,
                expired.revision,
                "unsafe replay",
                claim.lease_until_ms + 1,
                60_000
            )
            .is_err()
        );
    }
}
