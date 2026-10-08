// Retained historical launch fixtures, consumed through public workflows.
use herdr_farm::domain::*;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn seed(project: &Path, count: usize) -> String {
    let mut db = rusqlite::Connection::open(project.join(".state/state.db")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let tx = db.transaction().unwrap();
    // Simulate retained v1 launches from before v2 became mandatory. Restore
    // the insertion guard before committing; production schema is unchanged.
    let guard: String = tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='attempt_inputs_effective_profile'", [], |row| row.get(0)).unwrap();
    tx.execute_batch("DROP TRIGGER attempt_inputs_effective_profile")
        .unwrap();
    let mut last = String::new();
    for n in 0..count {
        let task = TaskId::new(format!("history-{n}")).unwrap();
        let mut inputs: LaunchInputs =
            serde_json::from_str(include_str!("../fixtures/launch-inputs-v1.json")).unwrap();
        inputs.task = task.clone();
        inputs.binding = format!("task:{}", task.as_str());
        inputs.project_store = project.join(".state/state.db").display().to_string();
        inputs.repositories = vec![RepositoryInput {
            repository: project.display().to_string(),
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        }];
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&inputs).unwrap()));
        let record = AttemptInputRecord {
            attempt: AttemptId::new(format!("attempt-{digest}")).unwrap(),
            operation: OperationId::new(format!("launch-{digest}")).unwrap(),
            inputs,
        };
        let payload = serde_json::to_string(&record).unwrap();
        let hash = format!("{:x}", Sha256::digest(payload.as_bytes()));
        tx.execute("INSERT INTO tasks(id,revision,state,title) VALUES(?1,4,'cancelled','Historical worker')", [task.as_str()]).unwrap();
        tx.execute("INSERT INTO attempts(id,task_id,revision,state,reservation,termination_observed) VALUES(?1,?2,2,'cancelled',?1,1)", params![record.attempt.as_str(), task.as_str()]).unwrap();
        tx.execute("INSERT INTO operations(id,task_id,kind,target,payload_version,payload,payload_hash,expected_revision,due_unix_ms,idempotency_key) VALUES(?1,?2,'runtime.launch',?3,1,?4,?5,4,0,?1)", params![record.operation.as_str(), task.as_str(), record.inputs.binding, payload, hash]).unwrap();
        tx.execute(
            "INSERT INTO attempt_inputs VALUES(?1,?2,?3,?4)",
            params![
                record.attempt.as_str(),
                record.operation.as_str(),
                payload,
                hash
            ],
        )
        .unwrap();
        let target = LaunchTarget {
            version: 1,
            operation: record.operation.clone(),
            attempt: record.attempt.clone(),
            route: RuntimeRoute {
                socket: project.join("historical.sock").display().to_string(),
                workspace_id: "history-workspace".into(),
                tab_id: "history-tab".into(),
                pane_id: "history-pane".into(),
                cwd: project.display().to_string(),
                ..Default::default()
            },
            terminal: "history-terminal".into(),
            session: ResourceIdentity {
                device: 1,
                inode: 2,
                born_secs: 1,
                born_nanos: 0,
            },
            supervisor: None,
            observed_unix_ms: 0,
        };
        let identity = RuntimeIdentity {
            socket: target.route.socket.clone(),
            workspace_id: target.route.workspace_id.clone(),
            tab_id: target.route.tab_id.clone(),
            pane_id: target.route.pane_id.clone(),
            cwd: target.route.cwd.clone(),
            ..Default::default()
        };
        let binding = RuntimeBinding {
            id: record.inputs.binding.clone(),
            task: Some(task.clone()),
            revision: 1,
            source_path: None,
            source_digest: None,
            session_source_digest: None,
            verification: RuntimeVerification::Unverified,
            identity,
        };
        let payload = serde_json::to_string(&binding).unwrap();
        tx.execute("INSERT INTO runtime_bindings(id,task_id,revision,payload,payload_hash) VALUES(?1,?2,1,?3,?4)", params![binding.id, task.as_str(), payload, format!("{:x}",Sha256::digest(payload.as_bytes()))]).unwrap();
        let owned = RuntimeOwnership {
            binding: binding.id.clone(),
            revision: 1,
            binding_revision: 1,
            identity_digest: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&binding.identity).unwrap())
            ),
            origin: "launched".into(),
            attempt: Some(record.attempt.clone()),
            session: Some(target.session.clone()),
            worktree: None,
            agent: Some(AgentIdentity {
                kind: "fixture".into(),
                name: "historical-worker".into(),
            }),
            config_digest: None,
            observed_unix_ms: 0,
        };
        let payload = serde_json::to_string(&owned).unwrap();
        tx.execute(
            "INSERT INTO runtime_ownership VALUES(?1,1,1,?2,?3,?4)",
            params![
                binding.id,
                record.attempt.as_str(),
                payload,
                format!("{:x}", Sha256::digest(payload.as_bytes()))
            ],
        )
        .unwrap();
        let creation = WorktreeCreation {
            version: 1,
            operation: record.operation.clone(),
            attempt: record.attempt.clone(),
            plans: worktree_plans(&record.inputs, &record.attempt).unwrap(),
            token: "c".repeat(64),
        };
        let started = LaunchStartedReceipt {
            version: 1,
            attempt: target.attempt.clone(),
            operation: target.operation.clone(),
            route: target.route.clone(),
            terminal: target.terminal.clone(),
            session: target.session.clone(),
            agent: AgentIdentity {
                kind: "fixture".into(),
                name: "historical-worker".into(),
            },
            supervisor: None,
            observed_unix_ms: 0,
        };
        for (kind, value) in [
            (
                "runtime.launch_started",
                serde_json::to_value(&started).unwrap(),
            ),
            (
                "runtime.launch_workspace",
                serde_json::to_value(&target).unwrap(),
            ),
            (
                "runtime.launch_target",
                serde_json::to_value(&target).unwrap(),
            ),
            (
                "runtime.worktrees_creation",
                serde_json::to_value(&creation).unwrap(),
            ),
            (
                "runtime.launch_release",
                serde_json::json!({"version":1,"target":target,"observed_unix_ms":0}),
            ),
        ] {
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES(?1,?2,1,1,?3)", params![kind, record.operation.as_str(), value.to_string()]).unwrap();
        }
        last = record.operation.as_str().to_owned();
    }
    tx.execute_batch(&guard).unwrap();
    tx.commit().unwrap();
    last
}
