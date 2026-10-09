//! Bounded advancement of an explicitly selected, durably prepared launch.
use super::*;

/// Continue creation, release and naming using their one-use durable boundaries.
/// A missing resource returns `None` and retains capacity. Errors leave the last
/// committed boundary intact; a subsequent call observes it instead of replaying
/// an uncertain request. Callers must fetch the current delivery revision again.
/// This service does not admit tasks, fabricate profiles or enable scheduling.
fn advance_launch_once(
    project: &Path,
    operation: &OperationId,
    expected_revision: u64,
    deadline: Instant,
    cancellation: Cancellation,
) -> Result<Option<LaunchStartedReceipt>> {
    use crate::operations::DeliveryState;
    let deadline = deadline.min(Instant::now() + Duration::from_secs(45));
    check(deadline, &cancellation)?;
    let project = project.canonicalize()?;
    let select = |revision| -> Result<_> {
        let control = crate::store::controlled::ReadControl::new(deadline, cancellation.clone());
        let mut db = crate::migration::open_active_scoped(&project, control)?;
        Ok(db.launch_advancement_selection(operation, revision)?)
    };
    let state = select(expected_revision).context("launch phase: selection")?;
    let delivery = &state.delivery;
    if delivery.state == DeliveryState::Confirmed {
        return super::reconcile_start(
            &project,
            operation,
            expected_revision,
            deadline,
            cancellation,
        )
        .map(Some);
    }
    let revision = if delivery.state == DeliveryState::Pending && delivery.attempts == 0 {
        let profile = state.record.inputs.effective_profile.as_ref().context("launch profile missing")?;
        super::resources::validate_execution_home(profile, &project)?;
        super::create_resource(
            &project,
            operation,
            expected_revision,
            deadline,
            cancellation.clone(),
        ).context("launch phase: worktree and resource creation")?;
        // Creation takes exactly one claim. Never adopt an unrelated new revision.
        expected_revision
            .checked_add(1)
            .context("launch revision overflow")?
    } else {
        ensure!(
            delivery.attempts == 1
                && matches!(
                    delivery.state,
                    DeliveryState::Claimed | DeliveryState::Ambiguous
                ),
            "launch has no recoverable original claim"
        );
        if delivery.state==DeliveryState::Claimed
            && state.kinds.contains("runtime.worktrees_creation")
            && !state.kinds.contains("runtime.launch_creation") {
            super::create_resource(&project,operation,expected_revision,deadline,cancellation.clone())?;
        }
        if delivery.state == DeliveryState::Claimed
            && state.kinds.contains("runtime.launch_workspace")
            && !["runtime.launch_layout", "runtime.launch_target", "runtime.launch_started"].iter().any(|kind| state.kinds.contains(*kind))
        {
            super::continue_workspace_layout(
                &project,
                operation,
                expected_revision,
                deadline,
                cancellation.clone(),
            )?;
        }
        if super::reconcile_resource(
            &project,
            operation,
            expected_revision,
            deadline,
            cancellation.clone(),
        )?
        .is_none()
        {
            return Ok(None);
        }
        expected_revision
    };
    check(deadline, &cancellation)?;
    let state = select(revision).context("launch phase: release selection")?;
    let delivery = &state.delivery;
    let has = |kind: &str| state.kinds.contains(kind);
    if !has("runtime.launch_release") {
        super::release_gate(
            &project,
            operation,
            revision,
            deadline,
            cancellation.clone(),
        ).context("launch phase: gate release / brief preparation")?;
    }
    check(deadline, &cancellation)?;
    // The naming service performs an effect only for an unnamed exact worker
    // under current authority, and refuses any prior naming intent. An uncertain
    // naming request, or expired authority, takes the observation-only path.
    if has("runtime.launch_name")
        || delivery.state != DeliveryState::Claimed
        || delivery.lease_until_ms.is_none_or(|until| until <= now())
    {
        super::reconcile_start(&project, operation, revision, deadline, cancellation).context("launch phase: start observation").map(Some)
    } else {
        super::name_started_agent(&project, operation, revision, deadline, cancellation).context("launch phase: naming and start observation").map(Some)
    }
}

/// Retry revision races only by reselecting the durable one-use boundaries.
pub fn advance_launch(project: &Path, operation: &OperationId, mut revision: u64, deadline: Instant, cancellation: Cancellation) -> Result<Option<LaunchStartedReceipt>> {
    for pass in 0..3 {
        match advance_launch_once(project, operation, revision, deadline, cancellation.clone()) {
            Ok(value) => return Ok(value),
            Err(error) => {
                let conflict = error.chain().any(|e| matches!(e.downcast_ref::<crate::store::StoreError>(), Some(crate::store::StoreError::Conflict)));
                if !conflict { return Err(error); }
                if pass < 2 && check(deadline, &cancellation).is_ok() {
                    let db=crate::migration::open_active_scoped(project,crate::store::controlled::ReadControl::new(deadline,cancellation.clone()))?;
                    revision=db.current_delivery_revision(operation)?;
                    continue;
                }
                let _guard = crate::execution_guard::RootGuard::exclusive_by(project.parent().context("project root missing")?, Instant::now() + Duration::from_secs(5), &Default::default())?;
                let phase = error.chain().filter_map(|e| e.to_string().strip_prefix("launch phase: ").map(str::to_owned)).last().unwrap_or_else(||"launch advancement".into());
                crate::migration::open_active(project)?.record_launch_failure(operation, &phase, &format!("{error:#}"), true, now())?;
                return Err(error.context("launch_conflict: bounded retries exhausted; coordinator notified"));
            }
        }
    }
    unreachable!()
}
