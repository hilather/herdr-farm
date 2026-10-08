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
