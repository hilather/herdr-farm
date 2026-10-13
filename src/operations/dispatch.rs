//! One durable attempt. Production adapters must retain resource ownership guards
//! and enforce lifecycle/reconciliation policy; this service never grants either.
use anyhow::{Context, Result};
use crate::{domain::{Operation, OperationId}, store::SqliteStore};
use super::{Claim, Delivery, Outcome};

/// Temporary diagnostics for isolated delivery labs; disabled by default.
pub fn trace_delivery(step: &str, detail: impl std::fmt::Debug) {
    if std::env::var_os("HERDR_FARM_TRACE_DELIVERY").as_deref() == Some(std::ffi::OsStr::new("1")) {
        eprintln!("delivery trace {step}: {detail:?}");
    }
}

pub trait DeliveryAdapter {
    type Prepared: PreparedDelivery;
    /// No external effect. Validate kind/version, lifecycle, authority and target
    /// identity, and acquire an exclusive resource guard retained by Prepared.
    fn prepare(&mut self, operation: &Operation) -> Result<Self::Prepared>;
}

pub trait PreparedDelivery {
    /// Recheck policy/liveness after durable claim. Must not perform the effect.
    fn revalidate(&mut self, operation: &Operation) -> Result<()>;
    /// Bound receipt persistence by the same control deadline as delivery.
    fn recording_allowed(&self) -> Result<()> { Ok(()) }
    fn recording_deadline(&self) -> Option<std::time::Instant> { None }
    /// Runs outside a SQLite transaction. A bare error is always ambiguous.
    /// Retryable requires proof of no effect; success requires a receipt/identity.
    /// Adapter timeout must fit the remaining lease. Keep ownership until dropped.
    fn deliver(&mut self, operation: &Operation, claim: &Claim) -> Result<Outcome>;
}

pub struct DispatchRequest<'a> {
    pub operation: &'a OperationId,
    pub expected_revision: u64,
    pub owner: &'a str,
    pub lease_ms: i64,
}

#[derive(Debug)]
pub enum DispatchResult {
    Recorded(Delivery),
    /// The effect may already exist. Do not call deliver again. Expiry and
    /// explicit observation must resolve this claim, including after restart.
    Unrecorded { claim: Claim, error: String },
}

pub fn dispatch_one<A: DeliveryAdapter>(
    store: &mut SqliteStore,
    request: DispatchRequest<'_>,
    adapter: &mut A,
    mut clock: impl FnMut() -> i64,
) -> Result<DispatchResult> {
    let (_, rows) = store.operation_rows(request.operation, None)?;
    let (operation, _) = rows.context("operation not found")?;
    trace_delivery("prepare begin", &operation.id);
    let mut prepared = adapter.prepare(&operation)?;
    trace_delivery("prepare complete", &operation.id);
    let claim = store.claim_operation(request.operation, request.expected_revision,
        request.owner, clock(), request.lease_ms)?;
    trace_delivery("claimed", &claim);
    let outcome = if prepared.revalidate(&operation).is_err() {
        Outcome::Retryable { no_effect_evidence: "adapter authorization withdrawn before deliver was called".into() }
    } else {
        // Revalidation may take time or allow another writer to change the task.
        if let Err(error) = store.validate_claim(&claim, clock()) {
            trace_delivery("claim validation failed", &error);
            return Ok(DispatchResult::Unrecorded { claim, error: error.to_string() });
        }
        trace_delivery("deliver begin", &claim);
        prepared.deliver(&operation, &claim).unwrap_or_else(|error| {
            trace_delivery("deliver error", format!("{error:#}"));
            Outcome::Ambiguous { observation_required: "adapter failed after delivery began; observe effect before retry (details withheld)".into() } })
    };
    trace_delivery("deliver outcome", &outcome);
    // Retry persistence only: never repeat the external effect. Keep ownership
    // while a competing SQLite writer finishes, bounded by lease and control.
    let lease_deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(claim.lease_until_ms.saturating_sub(clock()).max(0) as u64);
    let recording_deadline = prepared.recording_deadline().map_or(lease_deadline, |deadline| deadline.min(lease_deadline));
    let mut first_attempt = true;
    let result = loop {
        if !first_attempt && let Err(error) = prepared.recording_allowed() {
            break DispatchResult::Unrecorded { claim, error: format!("outcome recording control: {error:#}") };
        }
        first_attempt = false;
        // Cancellation forbids retries, but must not discard the cancellation
        // outcome. Give its first persistence attempt a short bounded window.
        let now = clock();
        let attempt_deadline = recording_deadline.min(std::time::Instant::now() + std::time::Duration::from_millis(20));
        trace_delivery("record begin", (&claim, now));
        match store.finish_operation_before(&claim, outcome.clone(), now, attempt_deadline) {
            Ok(delivery) => { trace_delivery("record complete", &delivery); break DispatchResult::Recorded(delivery); },
            Err(error) => {
                trace_delivery("record error", &error);
                if let Err(control_error) = prepared.recording_allowed() {
                    break DispatchResult::Unrecorded { claim, error: format!("outcome recording control: {control_error:#}; persistence: {error}") };
                }
                let transient = matches!(error, crate::store::StoreError::Busy | crate::store::StoreError::Io(_) | crate::store::StoreError::Deadline);
                if !transient || now >= claim.lease_until_ms || std::time::Instant::now() >= recording_deadline {
                    break DispatchResult::Unrecorded { claim, error: error.to_string() };
                }
                std::thread::sleep(std::time::Duration::from_millis(20).min(recording_deadline.saturating_duration_since(std::time::Instant::now())));
            }
        }
    };
    // Keep the adapter's resource guard alive through receipt persistence.
    drop(prepared);
    Ok(result)
}

#[cfg(test)]
mod tests;
