#![cfg(feature = "state-store")]
mod history {
    include!("support/inventory_history.rs");
}
use herdr_farm::{
    canonical_worker,
    domain::*,
    migration, runtime,
    store::identity_inventory::{Budget, MAX_INVENTORY_RECORDS},
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn project(root: &Path, name: &str) -> std::path::PathBuf {
    let project = root.join(name);
    for dir in [".state", "threads", "inbox"] {
        fs::create_dir_all(project.join(dir)).unwrap();
    }
    fs::write(
        project.join("PROJECT.md"),
        "+++\nname='Inventory lab'\n+++\n",
    )
    .unwrap();
    fs::write(project.join("TASKS.md"), "").unwrap();
    fs::write(project.join("MEMORY.md"), "").unwrap();
    fs::write(
        project.join(".state/project.json"),
        r#"{"status":"paused"}"#,
    )
    .unwrap();
    let plan = migration::inspect(&project).unwrap();
    migration::apply(&project, &plan, true).unwrap();
    project.canonicalize().unwrap()
}
fn budget(records: usize) -> Budget {
    Budget::new(
        50 * 1024 * 1024,
        records,
        Instant::now() + Duration::from_secs(10),
        Default::default(),
    )
    .unwrap()
}

#[test]
fn complete_history_passes_all_public_inventories_under_the_root_cap() {
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "current");
    let other = project(root.path(), "other");
    history::seed(&other, 1040);
    let mut shared = budget(MAX_INVENTORY_RECORDS);
    for project in [&current, &other] {
        let expected = if project == &other { 1040 } else { 0 };
        assert_eq!(
            migration::read_worktree_inventory(project, &mut shared).unwrap().len(),
            expected
        );
        assert_eq!(
            migration::read_launch_target_inventory(project, &mut shared).unwrap().len(),
            expected
        );
        assert_eq!(
            migration::read_identity_inventory(project, &mut shared).unwrap().len(),
            expected
        );
    }
}

#[test]
fn terminated_root_history_remains_counted_under_the_stopgap_budget() {
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "current");
    let other = project(root.path(), "other");
    history::seed(&current, 520);
    history::seed(&other, 520);
    let mut shared = budget(MAX_INVENTORY_RECORDS);
    let mut bindings = 0;
    let mut workspaces = 0;
    for project in [&current, &other] {
        bindings += migration::read_identity_inventory(project, &mut shared)
            .unwrap()
            .len();
        workspaces += migration::read_launch_target_inventory(project, &mut shared)
            .unwrap()
            .len();
    }
    assert_eq!(bindings, 1040);
    assert_eq!(workspaces, 1040);
    // The public pane check uses the retained pane projection without retiring
    // workspace ownership at the start-gate release.
    canonical_worker::check_advisory_pane_aliases(
        &current,
        "coordinator",
        &RuntimeRoute {
            socket: other.join("fresh.sock").display().to_string(),
            pane_id: "history-pane".into(),
            ..Default::default()
        },
        Instant::now() + Duration::from_secs(10),
        Default::default(),
    )
    .unwrap();
    let error = migration::read_launch_target_inventory(&other, &mut budget(1))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("launch target inventory")
            && error.contains("other")
            && error.contains("1 records"),
        "{error}"
    );
    // Repeated public scans consume one shared budget; they cannot reset it.
    let mut shared = budget(MAX_INVENTORY_RECORDS);
    let error = loop {
        if let Err(error) = migration::read_identity_inventory(&other, &mut shared) {
            break error.to_string();
        }
    };
    assert!(
        error.contains("binding inventory")
            && error.contains("other")
            && error.contains("16384 records"),
        "{error}"
    );
}

#[test]
fn root_budget_accepts_more_than_1024_current_bindings() {
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "current");
    let other = project(root.path(), "other");
    // Seed retained canonical history, then exercise the public conflict readers.
    for project in [&current, &other] {
        let db = rusqlite::Connection::open(project.join(".state/state.db")).unwrap();
        let binding = RuntimeBinding {
            id: "coordinator".into(),
            task: None,
            revision: 1,
            source_path: None,
            source_digest: None,
            session_source_digest: None,
            verification: RuntimeVerification::Unverified,
            identity: Default::default(),
        };
        let tx = db.unchecked_transaction().unwrap();
        for n in 0..520 {
            let mut binding = binding.clone();
            let task = TaskId::new(format!("bound-{n}")).unwrap();
            binding.id = format!("task:{}", task.as_str());
            binding.task = Some(task.clone());
            tx.execute("INSERT INTO tasks(id,revision,state,title) VALUES(?1,1,'queued','Unreleased binding')", [task.as_str()]).unwrap();
            let payload = serde_json::to_string(&binding).unwrap();
            use sha2::{Digest, Sha256};
            tx.execute("INSERT INTO runtime_bindings(id,task_id,revision,payload,payload_hash) VALUES(?1,?2,1,?3,?4)", rusqlite::params![binding.id, task.as_str(), payload, format!("{:x}",Sha256::digest(payload.as_bytes()))]).unwrap();
        }
        tx.commit().unwrap();
    }
    let mut shared = budget(MAX_INVENTORY_RECORDS);
    let mut count = 0;
    for project in [&current, &other] {
        count += migration::read_identity_inventory(project, &mut shared)
            .unwrap()
            .len();
    }
    assert!(count > 1024);
    // A public ownership operation must still refuse a conflicting live route.
    let snapshot = runtime::snapshot(&other).unwrap();
    runtime::create_binding(
        &other,
        None,
        None,
        snapshot.head,
        &RuntimeRoute {
            socket: other.join("live.sock").display().to_string(),
            pane_id: "live-pane".into(),
            workspace_id: "workspace".into(),
            tab_id: "tab".into(),
            cwd: other.display().to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    let result = canonical_worker::check_advisory_pane_aliases(
        &current,
        "coordinator",
        &RuntimeRoute {
            socket: other.join("live.sock").display().to_string(),
            pane_id: "live-pane".into(),
            workspace_id: "workspace".into(),
            tab_id: "tab".into(),
            cwd: other.display().to_string(),
            ..Default::default()
        },
        Instant::now() + Duration::from_secs(10),
        Default::default(),
    );
    assert!(result.unwrap_err().to_string().contains("another binding"));
}

#[test]
fn head_deadlines_are_typed_and_the_next_public_read_recovers() {
    use herdr_farm::store::StoreError;
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "deadline");
    let expected = migration::read_observation_head(&current, &mut budget(0)).unwrap();
    let mut expired = budget(0);
    expired.deadline = Instant::now();
    let error = migration::read_observation_head(&current, &mut expired).unwrap_err();
    assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<StoreError>(), Some(StoreError::Deadline))), "{error:#}");
    assert!(error.to_string().contains("observation head"), "{error:#}");
    assert!(error.to_string().contains(&current.display().to_string()), "{error:#}");
    assert!(error.to_string().contains("after 0 records and 0 bytes"), "{error:#}");
    let mut cancelled = budget(0);
    cancelled.cancellation.cancel();
    let error = migration::read_observation_head(&current, &mut cancelled).unwrap_err();
    assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<StoreError>(), Some(StoreError::Cancelled))), "{error:#}");
    assert_eq!(migration::read_observation_head(&current, &mut budget(0)).unwrap(), expected);
    // Admission and controller selection use the same public inventory contract.
    let mut expired = budget(1024);
    expired.deadline = Instant::now();
    let error = migration::read_routine_execution_hint(&current, &mut expired, None, 0).unwrap_err();
    assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<StoreError>(), Some(StoreError::Deadline))), "{error:#}");
    assert!(migration::read_routine_execution_hint(&current, &mut budget(1024), None, 0).unwrap().is_none());
    let mut expired = budget(1024);
    expired.deadline = Instant::now();
    let error = migration::read_controller_dispatch_hint(&current, &mut expired, 0, 0, true).unwrap_err();
    assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<StoreError>(), Some(StoreError::Deadline))), "{error:#}");
    migration::read_controller_dispatch_hint(&current, &mut budget(1024), 0, 0, true).unwrap();

}

#[test]
fn large_history_copy_head_reads_stay_within_the_production_allowance() {
    let root = tempfile::tempdir().unwrap();
    let original = project(root.path(), "original");
    let db = rusqlite::Connection::open(original.join(".state/state.db")).unwrap();
    db.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<55000) INSERT INTO events(kind,entity,revision,payload_version,payload) SELECT 'lab.history', 'history-'||x, 1, 1, '{}' FROM n").unwrap();
    drop(db);
    let expected = migration::read_observation_head(&original, &mut budget(0)).unwrap();
    // Preserve the publication's project identity while replacing this isolated
    // lab's database with an offline copy of its large store.
    let copy = root.path().join("large-copy.db");
    fs::copy(original.join(".state/state.db"), &copy).unwrap();
    fs::copy(&copy, original.join(".state/state.db")).unwrap();
    let mut times = Vec::new();
    for _ in 0..20 {
        let allowance = herdr_farm::store::identity_inventory::short_read_timeout();
        let mut read = Budget::new(2*1024*1024, 0, Instant::now()+allowance, Default::default()).unwrap();
        let start = Instant::now();
        assert_eq!(migration::read_observation_head(&original, &mut read).unwrap(), expected);
        times.push(start.elapsed());
    }
    times.sort();
    eprintln!("55k-event copy head read: median {:?}, max {:?}, adaptive bound {:?}", times[10], times[19], herdr_farm::store::identity_inventory::short_read_timeout());
}

#[test]
fn sql_inventory_deadline_interrupts_a_query_and_a_later_read_recovers() {
    use herdr_farm::store::StoreError;
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "sql-deadline");
    let head = migration::read_observation_head(&current, &mut budget(0)).unwrap();
    runtime::create_binding(&current, None, None, head, &RuntimeRoute::default()).unwrap();
    let expected = migration::read_identity_inventory(&current, &mut budget(1024)).unwrap();
    let db = rusqlite::Connection::open(current.join(".state/state.db")).unwrap();
    // A disposable local fixture forces SQL work before the first row. No
    // external service or private reader hook is involved.
    db.execute_batch("ALTER TABLE runtime_bindings RENAME TO original_bindings; CREATE VIEW runtime_bindings AS SELECT * FROM original_bindings WHERE (WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n)>0;").unwrap();
    let mut read = Budget::new(50*1024*1024, 1024, Instant::now()+Duration::from_secs(1), Default::default()).unwrap();
    let start = Instant::now();
    let error = migration::read_identity_inventory(&current, &mut read).unwrap_err();
    assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<StoreError>(), Some(StoreError::Deadline))), "{error:#}");
    assert!(start.elapsed() < Duration::from_secs(5));
    db.execute_batch("DROP VIEW runtime_bindings; ALTER TABLE original_bindings RENAME TO runtime_bindings;").unwrap();
    assert_eq!(migration::read_identity_inventory(&current, &mut budget(1024)).unwrap(), expected);
}
