#![cfg(all(target_os = "linux", feature = "state-store"))]
#![allow(clippy::disallowed_methods)]
//! Socket-free operator acknowledgement through the compiled CLI and store.
use herdr_farm::{domain::InboxContent, execution_guard::RootGuard, migration, runtime};
use std::{fs, process::{Command, Stdio}, time::{Duration, Instant}};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-farm");

#[test]
fn inbox_done_waits_for_maintenance_and_refreshes_an_unrelated_head() {
    let lab = tempfile::tempdir().unwrap();
    let root = lab.path().join("root");
    let project = root.join("demo");
    for dir in [".state", "threads", "inbox"] {
        fs::create_dir_all(project.join(dir)).unwrap();
    }
    fs::write(project.join("PROJECT.md"), "+++\nname='Contention lab'\n+++\n").unwrap();
    fs::write(project.join("TASKS.md"), "").unwrap();
    fs::write(project.join("MEMORY.md"), "").unwrap();
    fs::write(project.join(".state/project.json"), r#"{"status":"paused"}"#).unwrap();
    let plan = migration::inspect(&project).unwrap();
    migration::apply(&project, &plan, true).unwrap();
    let mut db = migration::open_active(&project).unwrap();
    let notice = InboxContent {
        id: "telemetry-health-output-check".into(), kind: "telemetry-health".into(),
        subject: "G1-SKEPTIC".into(), summary: "Review the output check".into(), ..Default::default()
    };
    db.deliver_telemetry_notice(db.current_head().unwrap(), &notice, jiff::Timestamp::now().as_millisecond()).unwrap();
    let held = RootGuard::exclusive(&root).unwrap();
    let started = Instant::now();
    let mut command = Command::new(BIN);
    command.env_clear().env("HOME", lab.path()).env("PATH", "/usr/bin:/bin")
        .env("HERDR_FARM_TEST_TIME_SCALE", include_str!("support/time-scale.txt").trim())
        .env("HERDR_FARM_LOCK_WAIT_SECS", "3")
        .args(["--root", root.to_str().unwrap(), "inbox", "done", "demo", &notice.id])
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    // The hold outlasts transient probes and gives the CLI a real wait window.
    while started.elapsed() < Duration::from_millis(350) {
        assert!(child.try_wait().unwrap().is_none(), "acknowledgement did not wait for maintenance");
        std::thread::sleep(Duration::from_millis(10));
    }
    let unrelated = InboxContent { id: "telemetry-health-unrelated".into(), subject: "other".into(), ..notice.clone() };
    db.deliver_telemetry_notice(db.current_head().unwrap(), &unrelated, jiff::Timestamp::now().as_millisecond()).unwrap();
    let held_ms=started.elapsed().as_millis();
    drop(held);
    let deadline = started + Duration::from_secs(4);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "acknowledgement exceeded its bound");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    eprintln!("maintenance_hold_ms={held_ms}; acknowledgement_elapsed_ms={}",started.elapsed().as_millis());
    let state = runtime::snapshot(&project).unwrap();
    assert!(state.inbox.iter().find(|item| item.content.id == notice.id).unwrap().done);
    assert!(!state.inbox.iter().find(|item| item.content.id == unrelated.id).unwrap().done);
    assert_eq!(state.events.iter().filter(|event| event.kind == "inbox.done").count(), 1);
}
