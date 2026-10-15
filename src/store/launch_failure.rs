//! Launch failures retain effect evidence in the existing event journal.
use super::*;
use crate::operations::Outcome;

#[derive(Clone, Copy)]
enum LaunchFailureReason {
    LaunchConflict,
    BriefAuthorityStale,
    BriefApprovalExpired,
    KnowledgeSnapshotStale,
    BriefNotReady,
    BriefAuthorityInvalid,
    DedicatedServerLost,
    LaunchRecoveryFailed,
    LaunchLeaseExpired,
    LaunchCreationTimedOut,
}
impl LaunchFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::BriefApprovalExpired => "brief_approval_expired",
            Self::KnowledgeSnapshotStale => "knowledge_snapshot_stale",
            Self::BriefNotReady => "brief_not_ready",
            Self::BriefAuthorityInvalid => "brief_authority_invalid",
            Self::BriefAuthorityStale => "brief_authority_stale",
            Self::LaunchConflict => "launch_conflict",
            Self::DedicatedServerLost => "dedicated_server_lost",
            Self::LaunchLeaseExpired => "launch_lease_expired",
            Self::LaunchCreationTimedOut => "launch_creation_timed_out",
            Self::LaunchRecoveryFailed => "launch_recovery_failed",
        }
    }
}

/// Repair ownership left by the pre-hotfix proven-server-loss transaction.
pub(super) fn proven_lost_launch(db: &Connection, owned: &RuntimeOwnership, budget: Option<&read_budget::ReadBudget>) -> Result<Option<serde_json::Value>> {
    let Some(id) = owned.attempt.as_ref().filter(|_| owned.origin == "launched") else { return Ok(None); };
    // Most claims belong to live workers. Do not decode their attempt or sealed
    // launch input on every observation pass. Ordinary cancelled workers also
    // stop here: their completed launch delivery is confirmed, not failed.
    let state = read_budget::optional(db, "SELECT state FROM attempts a WHERE id=?1 AND termination_observed=1 AND (state='lost' OR (state='cancelled' AND EXISTS(SELECT 1 FROM attempt_inputs i JOIN operation_delivery d ON d.operation_id=i.operation_id WHERE i.attempt_id=a.id AND d.state='permanent_failure')))", [id.as_str()], budget, &[], |row| row.get::<_, String>(0))?;
    let Some(state) = state else { return Ok(None); };
    let proof = if state == "lost" {
        let payload = read_budget::optional(db, "SELECT payload FROM events WHERE kind='attempt.launch_failed' AND entity=?1 AND json_extract(payload,'$.reason')='dedicated_server_lost' AND json_extract(payload,'$.terminal')=1 ORDER BY sequence DESC LIMIT 1", [id.as_str()], budget, &[(0,1)], |row| row.get::<_,String>(0))?;
        let Some(payload) = payload else { return Ok(None); };
        let proof: serde_json::Value = serde_json::from_str(&payload).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if proof["attempt"] != id.as_str() { return Ok(None); }
        proof
    } else {
        // Pre-fix cancellation persisted its proof under the sealed operation,
        // rather than emitting attempt.launch_failed. Require both journal entries.
        let payload = read_budget::optional(db, "SELECT c.payload FROM events c WHERE c.kind='attempt.cancellation_requested' AND c.entity=?1 AND json_extract(c.payload,'$.released')=1 AND json_extract(c.payload,'$.request.attempt')=?1 AND json_extract(c.payload,'$.attempt.id')=?1 AND json_extract(c.payload,'$.attempt.state')='cancelled' AND json_extract(c.payload,'$.attempt.termination_observed')=1 AND EXISTS(SELECT 1 FROM events o WHERE o.kind='operation.outcome' AND o.entity=(SELECT operation_id FROM attempt_inputs WHERE attempt_id=?1) AND o.sequence<c.sequence AND json_extract(o.payload,'$.actor')='operator.cancellation' AND json_extract(o.payload,'$.outcome.kind')='permanent_failure' AND json_extract(o.payload,'$.outcome.diagnostic')='dedicated_server_lost: launch_creation recovery; recorded socket directory and server absent: ' || json_extract(c.payload,'$.request.reason')) ORDER BY c.sequence DESC LIMIT 1", [id.as_str()], budget, &[(0,1)], |row| row.get::<_,String>(0))?;
        let Some(payload) = payload else { return Ok(None); };
        let cancellation: serde_json::Value = serde_json::from_str(&payload).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        serde_json::json!({"attempt":id,"phase":"launch_creation recovery","reason":"dedicated_server_lost","terminal":true,"observed_unix_ms":cancellation["request"]["requested_unix_ms"],"cancellation":cancellation["request"]["reason"]})
    };
    // Only proven loss candidates pay for the retained input decode. The proof
    // and claim must refer to the same sealed attempt and binding.
    let input = super::reservations::read_attempt_input(db, id.as_str(), budget)?;
    if input.inputs.binding != owned.binding { return Ok(None); }
    Ok(Some(proof))
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
            coalesce((SELECT min(json_extract(e.payload,'$.observed_unix_ms')) FROM events e WHERE e.entity=i.operation_id AND e.kind='runtime.launch_started'),
            (SELECT max(json_extract(e.payload,'$.observed_unix_ms')) FROM events e
                WHERE e.entity=i.operation_id AND e.kind IN ('runtime.worktree_progress','runtime.launch_progress','runtime.launch_target','runtime.launch_started','runtime.launch_release','runtime.launch_name')),o.due_unix_ms),
            EXISTS(SELECT 1 FROM events e WHERE e.entity=i.operation_id AND e.kind='runtime.launch_recovery_error' AND json_extract(e.payload,'$.terminal')=1)
            FROM attempts a JOIN attempt_inputs i ON i.attempt_id=a.id JOIN operations o ON o.id=i.operation_id
            WHERE a.termination_observed=0 AND a.state IN ('reserved','launching','failed')
            AND NOT EXISTS(SELECT 1 FROM operations b JOIN operation_delivery d ON d.operation_id=b.id
                WHERE b.kind='runtime.worker_brief' AND json_extract(b.payload,'$.attempt')=a.id AND d.state='confirmed') ORDER BY a.id")?;
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

    /// Retain background failures in the sidecar without changing canonical state or progress.
    pub fn record_launch_job_error(&mut self, job: &str, diagnostic: &str, now: i64) -> Result<()> {
        super::delivery::now_check(now)?;
        let Some(operation) = job.strip_prefix("canonical-worker:").or_else(|| job.strip_prefix("canonical-launch:")) else {
            return Ok(());
        };
        let operation = operation.strip_prefix("recover:").or_else(|| operation.strip_prefix("worker-exit-watch:")).unwrap_or(operation);
        let attempt: Option<String> = if let Some(id) = operation.strip_prefix("prepare-brief-") {
            self.connection
                .query_row("SELECT id FROM attempts WHERE id=?1", [id], |r| r.get(0))
                .optional()?
        } else {
            self.connection
                .query_row(
                    "SELECT coalesce(json_extract(o.payload,'$.attempt'),i.attempt_id) FROM operations o LEFT JOIN attempt_inputs i ON i.operation_id=o.id WHERE o.id=?1",
                    [operation],
                    |r| r.get(0),
                )
                .optional()?
                .flatten()
        };
        if let Some(attempt) = attempt {
            let project = self.connection.path().and_then(|p| std::path::Path::new(p).parent()).and_then(|p| p.parent())
                .ok_or_else(|| StoreError::Invalid("launch evidence requires a project database".into()))?;
            crate::telemetry::health::store::record_launch_job_error(project, &attempt, job, diagnostic, now)
                .map_err(|error| StoreError::Invalid(format!("launch evidence: {error:#}")))?;
        }
        Ok(())
    }

    /// Bounded sidecar retry evidence; never reads or writes canonical events.
    pub fn launch_job_errors(&self, attempt: &AttemptId) -> Result<serde_json::Value> {
        let project = self.connection.path().and_then(|p| std::path::Path::new(p).parent()).and_then(|p| p.parent())
            .ok_or_else(|| StoreError::Invalid("launch evidence requires a project database".into()))?;
        crate::telemetry::health::store::launch_job_errors(project, attempt.as_str())
            .map_err(|error| StoreError::Invalid(format!("launch evidence: {error:#}")))
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
        // Accepted work is evidence of receipt, even when an old ticker left
        // its brief ambiguous. Expiry cannot retroactively erase that effect.
        if attempt.state == AttemptState::Launching {
            let accepted:Option<String> = tx.query_row(&format!("{} ORDER BY s.created_unix_ms,s.submission_id LIMIT 1", super::worker_termination::ACCEPTED_SUBMISSION), params![attempt.task.as_str(),attempt.id.as_str()], |row| row.get(0)).optional()?;
            if let Some(submission) = accepted {
                super::worker_brief::submitted(&tx, &attempt.id, &submission, now)?;
                tx.commit()?;
                return Ok(true);
            }
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
        let terminal = gone || conflict || matches!(phase, "launch_lease_expired" | "launch_creation_timed_out") || count >= 2;
        let reason = if phase == "launch_creation_timed_out" {
            LaunchFailureReason::LaunchCreationTimedOut
        } else if gone {
            LaunchFailureReason::DedicatedServerLost
        } else if phase == "launch_lease_expired" {
            LaunchFailureReason::LaunchLeaseExpired
        } else if phase == "brief_approval_expired" {
            LaunchFailureReason::BriefApprovalExpired
        } else if phase == "knowledge_snapshot_stale" {
            LaunchFailureReason::KnowledgeSnapshotStale
        } else if phase == "brief_not_ready" {
            LaunchFailureReason::BriefNotReady
        } else if phase == "brief_authority_invalid" {
            LaunchFailureReason::BriefAuthorityInvalid
        } else if phase == "brief_authority_stale" {
            LaunchFailureReason::BriefAuthorityStale
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
                for owned in super::ownership::read_all(&tx)?.into_iter().filter(|o| o.attempt.as_ref() == Some(&attempt.id)) {
                    super::ownership::relinquish_ended(&tx, &owned, reason, payload.clone())?;
                }
                tx.execute("UPDATE tasks SET revision=revision+1,state='failed',active_attempt=NULL WHERE id=?1 AND active_attempt=?2",params![attempt.task.as_str(),attempt.id.as_str()])?;
            }
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('attempt.launch_failed',?1,?2,1,?3)", params![attempt.id.as_str(),integer(attempt.revision)?,payload.to_string()])?;
            super::active_work::invalidate(&tx)?;
            let task = read_task(&tx, attempt.task.as_str())?;
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('task.changed',?1,?2,1,?3)",params![task.id.as_str(),integer(task.revision)?,serde_json::to_string(&task).map_err(|e| StoreError::Invalid(e.to_string()))?])?;
            super::dispatch_log::mark(&tx, &attempt, now, "launch.failure")?;
            super::consumer_bindings::reconcile_task(&tx, attempt.task.as_str(), None)?;
            let project = Path::new(&record.inputs.project_store).parent().and_then(Path::parent)
                .and_then(Path::file_name).and_then(|name| name.to_str()).unwrap_or("PROJECT");
            super::inbox::result_notice(
                &tx,
                "attempt.launch_failure",
                operation.as_str(),
                attempt.task.as_str(),
                attempt.id.as_str(),
                phase,
                &format!(
                    "{reason}; phase {phase}; error {diagnostic}; worktrees retained; recovery: launch {project} run with a fresh approval after released capacity, or task cancel-attempt with fresh revisions for retained resources"
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

/// Persisted identity of a retired private server, independent of task lineage.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DedicatedServerRetirement {
    pub task: crate::domain::TaskId,
    pub pid: i32,
    pub record_identity: String,
    pub socket: String,
    pub viewer: serde_json::Value,
}

impl SqliteStore {
    /// Retry-safe audit of the external retirement, using the existing journal.
    pub fn record_dedicated_server_retirement(&mut self, retirement: &DedicatedServerRetirement) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let payload = serde_json::to_string(retirement).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let recorded: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE kind='runtime.dedicated_server_retired' AND entity=?1 AND payload=?2)",
            params![retirement.task.as_str(), payload], |row| row.get(0))?;
        if !recorded {
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('runtime.dedicated_server_retired',?1,1,1,?2)",
                params![retirement.task.as_str(), payload])?;
        }
        tx.commit()?;
        Ok(())
    }
}
