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
fn released_root_history_does_not_block_panes_but_live_and_uncertain_launches_do() {
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "current");
    let other = project(root.path(), "other");
    history::seed(&current, 520);
    let operation = history::seed(&other, 520);
    let route = RuntimeRoute {
        socket: other.join("historical.sock").display().to_string(),
        pane_id: "history-pane".into(),
        ..Default::default()
    };
    let check = || {
        canonical_worker::check_advisory_pane_aliases(
            &current,
            "coordinator",
            &route,
            Instant::now() + Duration::from_secs(10),
            Default::default(),
        )
    };
    check().unwrap();
    // These public readers share the same root budget as open --reprime.
    let mut shared = budget(1024);
    for project in [&current, &other] {
        assert!(
            migration::read_identity_inventory(project, &mut shared)
                .unwrap()
                .is_empty()
        );
        assert!(
            migration::read_launch_target_inventory(project, &mut shared)
                .unwrap()
                .is_empty()
        );
        assert!(
            migration::read_worktree_inventory(project, &mut shared)
                .unwrap()
                .is_empty()
        );
    }
    let db = rusqlite::Connection::open(other.join(".state/state.db")).unwrap();
    db.execute("UPDATE attempts SET termination_observed=0 WHERE id=(SELECT attempt_id FROM attempt_inputs WHERE operation_id=?1)", [&operation]).unwrap();
    assert!(check().unwrap_err().to_string().contains("another binding"));
    assert_eq!(
        migration::read_launch_target_inventory(&other, &mut budget(MAX_INVENTORY_RECORDS))
            .unwrap()
            .len(),
        2
    );
    db.execute("UPDATE attempts SET termination_observed=1 WHERE id=(SELECT attempt_id FROM attempt_inputs WHERE operation_id=?1)", [&operation]).unwrap();
    db.execute(
        "DELETE FROM events WHERE entity=?1 AND kind='runtime.launch_release'",
        [&operation],
    )
    .unwrap();
    assert!(check().unwrap_err().to_string().contains("another binding"));
    assert_eq!(
        migration::read_launch_target_inventory(&other, &mut budget(MAX_INVENTORY_RECORDS))
            .unwrap()
            .len(),
        2
    );
    let error = migration::read_launch_target_inventory(&other, &mut budget(1))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("launch target inventory")
            && error.contains("other")
            && error.contains("1 records"),
        "{error}"
    );
    assert!(
        migration::read_worktree_inventory(&other, &mut budget(MAX_INVENTORY_RECORDS)).is_err(),
        "unreleased creation provenance must still be validated"
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
fn relinquishment_retires_only_the_owned_binding_revision() {
    use herdr_farm::reconcile::{ResourceState, RuntimeObservation};
    let root = tempfile::tempdir().unwrap();
    let current = project(root.path(), "current");
    let other = project(root.path(), "other");
    let route = RuntimeRoute {
        socket: other.join("live.sock").display().to_string(),
        pane_id: "owned-pane".into(),
        workspace_id: "workspace".into(),
        tab_id: "tab".into(),
        cwd: other.display().to_string(),
        ..Default::default()
    };
    let head = runtime::snapshot(&other).unwrap().head;
    let changed = runtime::create_binding(&other, None, None, head, &route).unwrap();
    let mut store = migration::open_active(&other).unwrap();
    let now = jiff::Timestamp::now().as_millisecond();
    let head = store
        .record_observations(
            changed.head,
            &[RuntimeObservation {
                binding: "coordinator".into(),
                binding_revision: changed.binding.revision,
                observed_unix_ms: now,
                collector: "herdr-git-v2".into(),
                pane: ResourceState::Present,
                agent_present: true,
                session_identity: Some(ResourceIdentity {
                    device: 1,
                    inode: 2,
                    born_secs: 1,
                    born_nanos: 0,
                }),
                agent_identity: Some(AgentIdentity {
                    kind: "fixture".into(),
                    name: "owner".into(),
                }),
                ..Default::default()
            }],
        )
        .unwrap();
    let owned = store
        .adopt_runtime("coordinator", changed.binding.revision, head, now, None)
        .unwrap();
    let check = || {
        canonical_worker::check_advisory_pane_aliases(
            &current,
            "coordinator",
            &route,
            Instant::now() + Duration::from_secs(10),
            Default::default(),
        )
    };
    assert!(check().is_err());
    let head = store
        .relinquish_runtime(
            "coordinator",
            owned.ownership.revision,
            owned.head,
            "return ownership",
        )
        .unwrap();
    check().unwrap();
    assert!(
        migration::read_identity_inventory(&other, &mut budget(1024))
            .unwrap()
            .is_empty()
    );
    // The receipt is retained, but must not suppress a later rebind.
    let new_route = RuntimeRoute {
        pane_id: "new-pane".into(),
        ..route.clone()
    };
    store
        .rebind_runtime("coordinator", changed.binding.revision, head, &new_route)
        .unwrap();
    assert!(
        canonical_worker::check_advisory_pane_aliases(
            &current,
            "coordinator",
            &new_route,
            Instant::now() + Duration::from_secs(10),
            Default::default()
        )
        .is_err()
    );
    assert_eq!(
        migration::read_identity_inventory(&other, &mut budget(1024))
            .unwrap()
            .len(),
        1
    );
}
