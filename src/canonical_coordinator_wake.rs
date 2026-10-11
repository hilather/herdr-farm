//! Canonical result wakes: retained claims fence lost replies across restarts.
use super::*;
use crate::{
    runner::{Cmd, Output, Runner},
    steps::NotificationRetry,
};
use herdr_farm::notification_claim::{Batch, Claim, Mode, Phase};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};
const JOB: &str = "\0herdr-farm-canonical-wake";
const BUDGET: Duration = Duration::from_secs(45);
#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct State {
    sequence: u64,
    claim: Option<Claim>,
    delivered: BTreeSet<String>,
    retry: NotificationRetry,
    busy_hash: String,
    busy_probes: u32,
    busy_until: Option<i64>,
    // Last 1024 attempts, including the durable pending intent before any send.
    attempts: Vec<Value>,
}
fn load(dir: &Path) -> Result<State> {
    let state: State =
        paths::read_control_text(&dir.join(".state/canonical-wake.json"), 4 * 1024 * 1024)?
            .map(|text| serde_json::from_str(&text))
            .transpose()?
            .unwrap_or_default();
    if let Some(claim) = &state.claim {
        claim.validate(state.sequence)?;
    }
    ensure!(
        state.attempts.len() <= 1024,
        "canonical wake history exceeds bounds"
    );
    Ok(state)
}
fn persist(dir: &Path, state: &State) -> Result<()> {
    if let Some(claim) = &state.claim {
        claim.validate(state.sequence)?;
    }
    let bytes = serde_json::to_vec_pretty(state)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "canonical wake state exceeds bounds"
    );
    let path = dir.join(".state/canonical-wake.json");
    if paths::read_control_text(&path, 4 * 1024 * 1024)?.is_some_and(|old| old.as_bytes() == bytes) {
        return Ok(());
    }
    project::write_atomic(&path, &bytes)?;
    std::fs::File::open(dir.join(".state"))?.sync_all()?;
    Ok(())
}
fn defer_busy(state: &mut State, hash: String) {
    if state.busy_hash != hash {
        state.busy_probes = 0;
        state.busy_hash = hash;
    }
    state.busy_probes = state.busy_probes.saturating_add(1);
    // 5, 10, then 15 seconds: bounded below the idle observation target.
    let seconds = (5_u64 << state.busy_probes.min(3).saturating_sub(1)).min(15);
    state.busy_until = Some(jiff::Timestamp::now().as_millisecond()
        + crate::timing::retry(Duration::from_secs(seconds)).as_millis() as i64);
}
fn items(dir: &Path) -> Result<Vec<String>> {
    Ok(runtime::worker_result_inbox(dir)?
        .into_iter()
        .map(|i| i.content.id)
        .collect())
}
/// Cheap offer; native readiness and all authority are rechecked in the worker.
pub fn needed(dir: &Path) -> Result<bool> {
    if !dir.join(".state/canonical-coordinator.json").is_file() {
        return Ok(false);
    }
    let state = load(dir)?;
    let ids = items(dir)?;
    if state
        .claim
        .as_ref()
        .is_some_and(|c| c.phase == Phase::Pending)
    {
        return Ok(true);
    }
    let md = paths::read_control_text(&dir.join("PROJECT.md"), 1024 * 1024)?
        .context("wake settings missing")?;
    let (settings, _) = project::parse_project_md(&md)?;
    let mode = if settings.nudge {
        Mode::Nudge
    } else {
        Mode::Toast
    };
    let changed_mode = state
        .claim
        .as_ref()
        .is_some_and(|c| c.phase == Phase::Confirmed && c.mode != mode);
    if ids.is_empty() || (!changed_mode && ids.iter().all(|id| state.delivered.contains(id))) {
        return Ok(false);
    }
    if state.claim.as_ref().is_some_and(|c| {
        matches!(c.phase, Phase::Pending | Phase::Uncertain)
            && c.batch
                .as_ref()
                .is_some_and(|b| b.ids.iter().any(|id| ids.contains(id)))
    }) {
        return Ok(false);
    }
    let batch = Batch::new(ids)?;
    if state.busy_hash == batch.hash && state.busy_until.is_some_and(|until| jiff::Timestamp::now().as_millisecond() < until) {
        return Ok(false);
    }
    Ok(state.retry.retry.due(jiff::Timestamp::now()))
}
pub fn run(ctx: &Ctx, dir: &Path) -> Result<()> {
    let guard = herdr_farm::execution_guard::ProjectGuard::acquire(dir)?;
    let locks = guard.inherit_transfer()?;
    let control = crate::source_tree::Control {
        deadline: Instant::now() + BUDGET,
        cancellation: Default::default(),
    };
    let runner = WakeRunner { control: &control, locks: &locks };
    let held_ctx = Ctx {
        env: ctx.env,
        root: ctx.root.clone(),
        config_dir: ctx.config_dir.clone(),
        runner: &runner,
        detached_ticker: ctx.detached_ticker,
    };
    run_held(&held_ctx, dir, &control)
}
/// Contention and interrupted jobs prove nothing about coordinator delivery.
fn transient(error: &anyhow::Error) -> bool {
    use herdr_farm::store::StoreError;
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<StoreError>(),
            Some(StoreError::Busy | StoreError::Conflict | StoreError::Cancelled | StoreError::Deadline)
        ) || matches!(
            cause.downcast_ref::<std::fs::TryLockError>(),
            Some(std::fs::TryLockError::WouldBlock)
        )
    })
}
fn run_held(ctx: &Ctx, dir: &Path, control: &crate::source_tree::Control) -> Result<()> {
    if let Err(error) = run_inner(ctx, dir) {
        if control.cancellation.is_cancelled() || Instant::now() >= control.deadline {
            return Err(error);
        }
        if transient(&error) {
            // Bounded in-memory accounting avoids another contended store write.
            // Log the first and every 16th transient failure across wake jobs.
            use std::sync::atomic::{AtomicU64, Ordering};
            static TRANSIENT_FAILURES: AtomicU64 = AtomicU64::new(0);
            let count = TRANSIENT_FAILURES.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            if count % 16 == 1 {
                eprintln!("canonical wake transient failure #{count} for {}: {error:#}", dir.display());
            }
            return Err(error);
        }
        let mut state = load(dir)?;
        // Before a native call, failure proves no effect and can back off safely.
        if !state
            .claim
            .as_ref()
            .is_some_and(|c| matches!(c.phase, Phase::Pending | Phase::Uncertain))
        {
            let batch = Batch::new(items(dir)?)?;
            state.retry.hash = batch.hash.clone();
            state
                .retry
                .retry
                .reserve(jiff::Timestamp::now(), &batch.hash);
            state.retry.retry.last_error =
                "Wake preflight failed; no prompt or toast was sent".into();
            if state.attempts.len() == 1024 {
                state.attempts.remove(0);
            }
            state.attempts.push(json!({"at_unix_ms":jiff::Timestamp::now().as_millisecond(),"hash":batch.hash,"items":batch.ids.len(),"outcome":"not-sent","reason":"preflight_failed","error":format!("{error:#}")}));
            persist(dir, &state)?;
        }
        return Err(error);
    }
    Ok(())
}
fn run_inner(ctx: &Ctx, dir: &Path) -> Result<()> {
    let mut state = load(dir)?;
    // A crash after reservation cannot establish whether the prompt arrived.
    if let Some(claim) = state.claim.as_mut().filter(|c| c.phase == Phase::Pending) {
        claim.phase = Phase::Uncertain;
        claim.error =
            "Wake reply lost; inspect coordinator before handling the claimed inbox items.".into();
        if let Some(last) = state.attempts.last_mut() {
            last["outcome"] = json!("uncertain");
        }
        persist(dir, &state)?;
    }
    if !needed(dir)? {
        return Ok(());
    }
    let rows = migration::open_active_unchecked(dir)?.coordinator_wake_rows()?;
    if rows.control.state != ProjectState::Active {
        return Ok(());
    }
    let Some(binding) = rows.binding.as_ref() else {
        return Ok(());
    };
    let text =
        paths::read_control_text(&dir.join(".state/canonical-coordinator.json"), 1024 * 1024)?
            .context("wake coordinator journal missing")?;
    let mut journal = read_journal(&text)?;
    if journal.phase != "accepted" {
        return Ok(());
    }
    ensure!(
        RuntimeRoute::from_identity(&binding.identity) == journal.route,
        "wake coordinator binding changed"
    );
    ensure!(
        rows.ownership
            .as_ref()
            .is_some_and(|o| o.binding_revision == binding.revision
                && o.config_digest == journal.config_digest),
        "wake coordinator is not owned"
    );
    let md = paths::read_control_text(&dir.join("PROJECT.md"), 1024 * 1024)?
        .context("wake settings missing")?;
    let (settings, _) = project::parse_project_md(&md)?;
    ensure!(
        settings.coordinator_agent == journal.kind,
        "wake coordinator agent setting changed"
    );
    // Nudge is live policy; changing it does not authorize re-routing a coordinator.
    journal.settings_digest = crate::thread::sha256_hex(md.as_bytes());
    crate::notification_delivery::config(ctx, dir)?;
    let mode = if settings.nudge {
        Mode::Nudge
    } else {
        Mode::Toast
    };
    if state
        .claim
        .as_ref()
        .is_some_and(|c| c.phase == Phase::Confirmed && c.mode != mode)
    {
        state.delivered.clear();
    }
    let h = Herdr::new(ctx.env.herdr_bin(), &journal.socket, ctx.runner);
    if mode == Mode::Nudge {
        let live = agent(&h, &journal, false)?;
        if !matches!(live["agent_status"].as_str(), Some("idle" | "done")) {
            let batch = Batch::new(items(dir)?)?;
            defer_busy(&mut state, batch.hash);
            persist(dir, &state)?;
            return Ok(());
        }
        state.busy_hash.clear();
        state.busy_probes = 0;
        state.busy_until = None;
        persist(dir, &state)?;
        ready(ctx, &h, &journal)?;
    }
    fence_live(dir, ctx, &journal)?;
    let ids = items(dir)?;
    state.delivered.retain(|id| ids.contains(id));
    let batch = Batch::new(ids)?;
    if batch.ids.is_empty() || batch.ids.iter().all(|id| state.delivered.contains(id)) {
        return Ok(());
    }
    let payload = serde_json::to_string(&if mode == Mode::Nudge {
        json!({"target":journal.route.pane_id,"text":crate::steps::NUDGE_TEXT})
    } else {
        json!({"title":format!("herdr-farm: {}",dir.file_name().unwrap_or_default().to_string_lossy()),"body":format!("{} worker-result inbox item(s) waiting on coordinator. Run context.",batch.ids.len())})
    })?;
    state.sequence = state
        .sequence
        .checked_add(1)
        .context("wake sequence exhausted")?;
    let authority = crate::thread::sha256_hex(&serde_json::to_vec(&(
        &journal.route,
        &journal.terminal,
        &journal.session_identity,
        &journal.settings_digest,
        &journal.config_digest,
    ))?);
    state.claim = Some(Claim {
        sequence: state.sequence,
        batch: Some(batch.clone()),
        mode,
        authority,
        payload: payload.clone(),
        phase: Phase::Pending,
        retry_of: None,
        error: String::new(),
    });
    state.retry.hash = batch.hash.clone();
    state
        .retry
        .retry
        .reserve(jiff::Timestamp::now(), &batch.hash);
    if state.attempts.len() == 1024 {
        state.attempts.remove(0);
    }
    state.attempts.push(json!({"sequence":state.sequence,"at_unix_ms":jiff::Timestamp::now().as_millisecond(),"mode":mode,"items":batch.ids.len(),"hash":batch.hash,"outcome":"pending"}));
    persist(dir, &state)?;
    // Busy transitions observed at this last probe defer with proven no send.
    let delivered = (|| -> Result<bool> {
        fence_live(dir, ctx, &journal)?;
        if mode == Mode::Nudge {
            let live = agent(&h, &journal, false)?;
            if !matches!(live["agent_status"].as_str(), Some("idle" | "done")) {
                return Ok(false);
            }
            agent(&h, &journal, true)?;
        }
        ensure!(items(dir)? == batch.ids, "wake inbox changed before send");
        Ok(true)
    })();
    let became_busy = matches!(&delivered, Ok(false));
    let outcome = match delivered {
        Ok(true) => {
            // Any failure after starting this call is ambiguous, never auto-replayed.
            let result = call(
                &h,
                &format!("coordinator-wake-{}", state.sequence),
                if mode == Mode::Nudge {
                    "agent.prompt"
                } else {
                    "notification.show"
                },
                serde_json::from_str(&payload)?,
            );
            match result {
                Ok(reply)
                    if mode == Mode::Nudge
                        && reply["type"] == "agent_prompted"
                        && reply["agent"]["terminal_id"] == journal.terminal
                        && herdr_farm::canonical_worker::validate_native_agent(
                            &reply["agent"],
                            &journal.route,
                            &journal.terminal,
                            &journal.kind,
                            Some(&journal.name),
                            false,
                        )
                        .is_ok() =>
                {
                    "confirmed"
                }
                Ok(reply)
                    if mode == Mode::Toast
                        && reply["type"] == "notification_show"
                        && reply["shown"] == true
                        && reply["reason"] == "shown" =>
                {
                    "confirmed"
                }
                Ok(reply)
                    if mode == Mode::Toast
                        && reply["type"] == "notification_show"
                        && reply["shown"] == false
                        && matches!(
                            reply["reason"].as_str(),
                            Some("disabled" | "rate_limited" | "no_foreground_client" | "busy")
                        ) =>
                {
                    "not-shown"
                }
                _ => "uncertain",
            }
        }
        _ => "deferred",
    };
    let claim = state.claim.as_mut().unwrap();
    match outcome {
        "confirmed" => {
            claim.phase = Phase::Confirmed;
            state.delivered.extend(batch.ids);
            state.retry = Default::default();
        }
        "not-shown" => {
            claim.phase = Phase::NotShown;
            claim.error = "Native toast not shown; retry after backoff".into();
        }
        "deferred" => {
            claim.phase = Phase::Suppressed;
            claim.error =
                "Wake was not sent: readiness or inputs changed; retry after backoff".into();
        }
        _ => {
            claim.phase = Phase::Uncertain;
            claim.error="Wake outcome uncertain; inspect coordinator, then mark claimed inbox items DONE to release the fence".into();
        }
    }
    if became_busy {
        // A proven busy transition sent nothing. Use the same short probe
        // policy rather than the delivery-failure retry reserved above.
        state.retry = Default::default();
        defer_busy(&mut state, batch.hash);
    }
    state.attempts.last_mut().unwrap()["outcome"] = json!(outcome);
    state.attempts.last_mut().unwrap()["completed_unix_ms"] =
        json!(jiff::Timestamp::now().as_millisecond());
    persist(dir, &state)
}
#[derive(Serialize, Deserialize)]
struct Input {
    project: PathBuf,
    identity: (u64, u64),
    home: PathBuf,
    config: PathBuf,
    bin: String,
}
pub fn request(ctx: &Ctx, dir: &Path) -> Result<crate::executor::Request> {
    use std::os::unix::fs::MetadataExt;
    let project = dir.canonicalize()?;
    let metadata = std::fs::metadata(&project)?;
    let input = Input {
        project,
        identity: (metadata.dev(), metadata.ino()),
        home: std::path::absolute(&ctx.env.home)?,
        config: std::path::absolute(&ctx.config_dir)?,
        bin: ctx.env.herdr_bin(),
    };
    let deadline = Instant::now() + BUDGET;
    let mut command = Cmd::new(JOB, BUDGET).stdin(serde_json::to_string(&input)?);
    command.deadline = Some(deadline);
    Ok(crate::executor::Request {
        identity: crate::executor::Identity {
            operation: "canonical-wake:results".into(),
            revision: 1,
            project: input.project.display().to_string(),
            machine: "local".into(),
            terminal: None,
        },
        lane: crate::executor::Lane::Control,
        deadline,
        command,
    })
}
pub struct JobRunner {
    pub inner: Arc<dyn Runner + Send + Sync>,
}
impl Runner for JobRunner {
    fn run(&self, command: &Cmd) -> Result<Output> {
        if command.program != JOB {
            return self.inner.run(command);
        }
        let entered = Instant::now();
        let input: Input =
            serde_json::from_str(command.stdin.as_deref().context("wake input missing")?)?;
        let control = crate::source_tree::Control {
            deadline: command
                .deadline
                .context("wake deadline missing")?
                .min(entered + BUDGET),
            cancellation: command
                .cancellation
                .clone()
                .context("wake cancellation missing")?,
        };
        control.check()?;
        let guard = herdr_farm::execution_guard::ProjectGuard::acquire(&input.project)?;
        let locks = guard.inherit_transfer()?;
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&input.project)?;
        ensure!(
            (metadata.dev(), metadata.ino()) == input.identity
                && input.project.canonicalize()? == input.project,
            "wake project incarnation changed"
        );
        let env = paths::Env::for_observation(&input.home, &input.bin);
        let runner = WakeRunner { control: &control, locks: &locks };
        let ctx = Ctx {
            env: &env,
            root: input.project.parent().unwrap().into(),
            config_dir: input.config,
            runner: &runner,
            detached_ticker: false,
        };
        run_held(&ctx, &input.project, &control)?;
        Ok(Output {
            code: Some(0),
            elapsed: entered.elapsed(),
            ..Default::default()
        })
    }
    fn socket_request(&self, path: &Path, line: &str, timeout: Duration) -> Result<String> {
        self.inner.socket_request(path, line, timeout)
    }
}
struct WakeRunner<'a> {
    control: &'a crate::source_tree::Control,
    locks: &'a [crate::runner::InheritedLock],
}
impl Runner for WakeRunner<'_> {
    fn run(&self, command: &Cmd) -> Result<Output> {
        herdr_farm::supervision::run(
            command.clone(),
            self.control.deadline,
            self.control.cancellation.clone(),
            self.locks,
        )
    }
    fn socket_request(&self, path: &Path, line: &str, timeout: Duration) -> Result<String> {
        self.control.check()?;
        crate::runner::RealRunner.socket_request(
            path,
            line,
            timeout.min(
                self.control
                    .deadline
                    .saturating_duration_since(Instant::now()),
            ),
        )
    }
}
