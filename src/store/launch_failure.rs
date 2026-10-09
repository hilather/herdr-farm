//! Launch failures retain effect evidence in the existing event journal.
use super::*;
use crate::operations::Outcome;

#[derive(Clone, Copy)]
enum LaunchFailureReason {
    LaunchConflict,
    DedicatedServerLost,
    LaunchRecoveryFailed,
}
impl LaunchFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::LaunchConflict => "launch_conflict",
            Self::DedicatedServerLost => "dedicated_server_lost",
            Self::LaunchRecoveryFailed => "launch_recovery_failed",
        }
    }
}

/// A missing endpoint alone is transient. Only an owner-controlled dedicated
/// run directory (or its absence), a matching record when present, and an absent
/// socket directory and process can prove this launch lost its terminal resources.
/// Callers must obtain the socket from this operation's recorded creation intent.
pub(super) fn dedicated_server_gone(record: &AttemptInputRecord, socket: &str) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let project = Path::new(&record.inputs.project_store)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| StoreError::Invalid("launch project path missing".into()))?;
    let slug = project
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| StoreError::Invalid("launch project name missing".into()))?;
    let path = project
        .parent()
        .ok_or_else(|| StoreError::Invalid("launch root missing".into()))?
        .join(".herdr-run")
        .join(format!("{slug}-{}", record.inputs.task.as_str()));
    let uid = unsafe { libc::geteuid() };
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_dir() && m.uid() == uid => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Ok(false),
    }
    let path = path.join("herdr/server.json");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Ok(false),
    };
    if let Some(metadata) = metadata {
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.mode() & 0o022 != 0
        {
            return Ok(false);
        }
        let value: serde_json::Value = match std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => return Ok(false),
        };
        if value["managed_by"] == "operator"
            || value["socket"].as_str() != Some(socket)
            || value["project"].as_str() != Some(slug)
            || value["task"].as_str() != Some(record.inputs.task.as_str())
            || value["pid"].as_i64().is_none_or(|p| p <= 1)
        {
            return Ok(false);
        }
    }
    let Some(directory) = Path::new(socket).parent() else {
        return Ok(false);
    };
    match std::fs::symlink_metadata(directory) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Ok(false),
    }
    let wanted = format!("HERDR_SOCKET_PATH={socket}");
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return Ok(false),
    };
    // A server spawned with env_clear and no privilege change is dumpable and
    // cannot gain permitted capabilities. A setcap Herdr is non-dumpable and
    // consequently excluded by the environ ownership check as well.
    let capabilities = |path: &Path| -> Option<u64> {
        std::fs::read_to_string(path).ok()?.lines()
            .find_map(|line| line.strip_prefix("CapPrm:")
                .and_then(|value| u64::from_str_radix(value.trim(), 16).ok()))
    };
    let Some(own_capabilities) = capabilities(Path::new("/proc/self/status")) else {
        return Ok(false);
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => return Ok(false),
        };
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let environ = entry.path().join("environ");
        let env = match std::fs::read(&environ) {
            Ok(env) => env,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
                || e.raw_os_error() == Some(libc::ESRCH) => continue,
            Err(error) => {
                match std::fs::metadata(&environ) {
                    Ok(m) if m.uid() != uid => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound
                        || e.raw_os_error() == Some(libc::ESRCH) => continue,
                    Err(_) => return Ok(false),
                    _ => {}
                }
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && capabilities(&entry.path().join("status"))
                        .is_some_and(|target| target & !own_capabilities != 0)
                {
                    continue;
                }
                return Ok(false);
            }
        };
        if env.split(|b| *b == 0).any(|e| e == wanted.as_bytes()) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn creation_socket(
    tx: &Connection,
    record: &AttemptInputRecord,
) -> Result<Option<String>> {
    let payload: Option<String> = tx.query_row("SELECT payload FROM events WHERE entity=?1 AND kind='runtime.launch_creation' ORDER BY sequence DESC LIMIT 1", [record.operation.as_str()], |r| r.get(0)).optional()?;
    payload
        .map(|p| {
            serde_json::from_str::<crate::domain::LaunchCreationIntent>(&p)
                .map_err(|e| StoreError::Corrupt(e.to_string()))
                .and_then(|i| {
                    if i.operation != record.operation || i.attempt != record.attempt {
                        return Err(StoreError::Corrupt(
                            "launch creation identity mismatch".into(),
                        ));
                    }
                    i.route.validate().map_err(StoreError::Corrupt)?;
                    Ok(i.route.socket)
                })
        })
        .transpose()
}

impl SqliteStore {
    /// Timed progress of capacity-retaining launches, selected without unrelated
    /// project history. Error observations never renew this progress timestamp.
    pub fn launch_progress_health(&self) -> Result<Vec<(AttemptId, i64, bool)>> {
        let mut statement=self.connection.prepare("SELECT a.id,
            coalesce((SELECT max(json_extract(e.payload,'$.observed_unix_ms')) FROM events e
                WHERE e.entity=i.operation_id AND e.kind IN ('runtime.worktree_progress','runtime.launch_progress','runtime.launch_target','runtime.launch_started','runtime.launch_release','runtime.launch_name')),o.due_unix_ms),
            EXISTS(SELECT 1 FROM events e WHERE e.entity=i.operation_id AND e.kind='runtime.launch_recovery_error' AND json_extract(e.payload,'$.terminal')=1)
            FROM attempts a JOIN attempt_inputs i ON i.attempt_id=a.id JOIN operations o ON o.id=i.operation_id
            WHERE a.termination_observed=0 AND a.state IN ('reserved','launching','failed') ORDER BY a.id")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, last, stopped) = row?;
            Ok((
                AttemptId::new(id).map_err(StoreError::Corrupt)?,
                last,
                stopped,
            ))
        })
        .collect()
    }

    /// A refused atomic reservation has no attempt to close, but its task still
    /// receives one coordinator-visible outcome and a supported relaunch action.
    pub fn notify_unreserved_launch_conflict(
        &mut self,
        task: &TaskId,
        phase: &str,
        diagnostic: &str,
        now: i64,
    ) -> Result<()> {
        super::delivery::now_check(now)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        super::inbox::result_notice(
            &tx,
            "attempt.launch_failure",
            &format!("unreserved-{}", task.as_str()),
            task.as_str(),
            "not_reserved",
            phase,
            &format!(
                "launch_conflict; phase {phase}; error {diagnostic}; recovery: relaunch (no attempt was reserved)"
            ),
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Record a bounded launch failure. Effects are never replayed here. A
    /// terminal delivery with unproven resources retains capacity for cancellation.
    pub fn record_launch_failure(
        &mut self,
        operation: &OperationId,
        phase: &str,
        diagnostic: &str,
        conflict: bool,
        now: i64,
    ) -> Result<bool> {
        super::delivery::now_check(now)?;
        let diagnostic: String = diagnostic
            .chars()
            .take(2000)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = super::reservations::read_input(&tx, operation, None)?;
        let mut attempt = read_attempt(&tx, &record.attempt)?;
        if !attempt.retains_capacity() {
            return Ok(true);
        }
        let delivery = super::delivery::delivery(&tx, operation)?;
        let socket = creation_socket(&tx, &record).unwrap_or(None);
        let gone = socket
            .as_deref()
            .map(|s| dedicated_server_gone(&record, s))
            .transpose()?
            .unwrap_or(false);
        let no_worker: bool = tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM events WHERE entity=?1 AND kind IN ('runtime.launch_creation','runtime.launch_workspace','runtime.launch_layout','runtime.launch_target','runtime.launch_started','runtime.launch_release','runtime.launch_name')) AND NOT EXISTS(SELECT 1 FROM runtime_ownership WHERE attempt_id=?2)", params![operation.as_str(),attempt.id.as_str()], |r| r.get(0))?;
        let stopped: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE entity=?1 AND kind='runtime.launch_recovery_error' AND json_extract(payload,'$.terminal')=1)", [operation.as_str()], |r| r.get(0))?;
        if stopped && !gone {
            return Ok(true);
        }
        let count: u64 = tx.query_row("SELECT count(*) FROM events WHERE entity=?1 AND kind='runtime.launch_recovery_error' AND json_extract(payload,'$.error')=?2", params![operation.as_str(), diagnostic], |r| r.get(0))?;
        let terminal = gone || conflict || count >= 2;
        let reason = if gone {
            LaunchFailureReason::DedicatedServerLost
        } else if conflict {
            LaunchFailureReason::LaunchConflict
        } else {
            LaunchFailureReason::LaunchRecoveryFailed
        }
        .as_str();
        let effects: Vec<(u64,String)> = tx.prepare("SELECT sequence,kind FROM events WHERE entity=?1 AND kind IN ('runtime.worktrees_creation','runtime.worktrees_ready','runtime.launch_creation','runtime.launch_workspace','runtime.launch_target','runtime.launch_release','runtime.launch_started') ORDER BY sequence")?
            .query_map([operation.as_str()],|r| Ok((r.get(0)?,r.get(1)?)))?.collect::<std::result::Result<_,_>>()?;
        let payload = serde_json::json!({"attempt":attempt.id,"phase":phase,"error":diagnostic,"reason":reason,"terminal":terminal,"socket":socket,"observed_unix_ms":now,"worktrees_retained":effects.iter().any(|(_,kind)|kind=="runtime.worktrees_creation"),"retained_effect_events":effects});
        tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('runtime.launch_recovery_error',?1,?2,1,?3)", params![operation.as_str(), integer(delivery.revision)?, payload.to_string()])?;
        if terminal {
            if !matches!(
                delivery.state,
                crate::operations::DeliveryState::Confirmed
                    | crate::operations::DeliveryState::PermanentFailure
            ) {
                super::delivery::update_outcome(
                    &tx,
                    &delivery,
                    &Outcome::PermanentFailure {
                        diagnostic: format!("{reason}: {phase}: {diagnostic}"),
                    },
                    now,
                    "launch.failure",
                )?;
            }
            let briefs: Vec<String> = tx.prepare("SELECT id FROM operations WHERE kind='runtime.worker_brief' AND json_extract(payload,'$.attempt')=?1")?
                .query_map([attempt.id.as_str()], |r| r.get(0))?.collect::<std::result::Result<_,_>>()?;
            for brief in briefs {
                let id = OperationId::new(brief).map_err(StoreError::Corrupt)?;
                let old = super::delivery::delivery(&tx, &id)?;
                if !matches!(
                    old.state,
                    crate::operations::DeliveryState::Confirmed
                        | crate::operations::DeliveryState::PermanentFailure
                ) {
                    super::delivery::update_outcome(
                        &tx,
                        &old,
                        &Outcome::PermanentFailure {
                            diagnostic: format!(
                                "{reason}: {phase}: {diagnostic}; prior prompt effect retained"
                            ),
                        },
                        now,
                        "launch.failure",
                    )?;
                }
            }
            attempt.state = if gone {
                AttemptState::Lost
            } else {
                AttemptState::Failed
            };
            attempt.termination_observed = gone || no_worker;
            attempt.revision = attempt
                .revision
                .checked_add(1)
                .ok_or_else(|| StoreError::Invalid("attempt revision exhausted".into()))?;
            tx.execute(
                "UPDATE attempts SET revision=?2,state=?3,termination_observed=?4 WHERE id=?1",
                params![
                    attempt.id.as_str(),
                    integer(attempt.revision)?,
                    attempt.state.as_str(),
                    attempt.termination_observed
                ],
            )?;
            if attempt.termination_observed {
                tx.execute("UPDATE tasks SET revision=revision+1,state='failed',active_attempt=NULL WHERE id=?1 AND active_attempt=?2",params![attempt.task.as_str(),attempt.id.as_str()])?;
            }
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('attempt.launch_failed',?1,?2,1,?3)", params![attempt.id.as_str(),integer(attempt.revision)?,payload.to_string()])?;
            super::active_work::invalidate(&tx)?;
            let task = read_task(&tx, attempt.task.as_str())?;
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('task.changed',?1,?2,1,?3)",params![task.id.as_str(),integer(task.revision)?,serde_json::to_string(&task).map_err(|e| StoreError::Invalid(e.to_string()))?])?;
            super::dispatch_log::mark(&tx, &attempt, now, "launch.failure")?;
            super::consumer_bindings::reconcile_task(&tx, attempt.task.as_str(), None)?;
            super::inbox::result_notice(
                &tx,
                "attempt.launch_failure",
                operation.as_str(),
                attempt.task.as_str(),
                attempt.id.as_str(),
                phase,
                &format!(
                    "{reason}; phase {phase}; error {diagnostic}; worktrees retained; recovery: relaunch after released capacity, or task cancel-attempt with fresh revisions for retained resources"
                ),
            )?;
        }
        tx.commit()?;
        Ok(terminal)
    }

    /// Observe disappearance without treating a temporarily missing socket as proof.
    pub fn reconcile_missing_launch_server(
        &mut self,
        operation: &OperationId,
        now: i64,
    ) -> Result<bool> {
        let record = super::reservations::read_input(&self.connection, operation, None)?;
        if let Some(socket) = creation_socket(&self.connection, &record).unwrap_or(None)
            && dedicated_server_gone(&record, &socket)?
        {
            return self.record_launch_failure(operation, "launch_creation recovery", "dedicated Herdr server socket directory absent; no server process holds recorded route", false, now);
        }
        Ok(false)
    }
}
