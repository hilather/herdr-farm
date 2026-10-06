//! Process-local test acceleration. Production durations are returned unchanged.
//! Never use this for external execution budgets or approval validity.
use std::{sync::OnceLock, time::Duration};

pub const ENV: &str = "HERDR_FARM_TEST_TIME_SCALE";
static SCALE: OnceLock<Option<f64>> = OnceLock::new();

/// Only the actual process environment is consulted, never project configuration.
/// Invalid, zero, negative, infinite and greater-than-one values are ignored.
pub fn scale() -> Option<f64> {
    *SCALE.get_or_init(|| {
        std::env::var(ENV)
            .ok()?
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
    })
}

fn scaled(duration: Duration, floor: Duration) -> Duration {
    match scale() {
        Some(factor) if !duration.is_zero() => duration.mul_f64(factor).max(floor),
        _ => duration,
    }
}

/// Controller passes and observation intervals never fall below 50 ms.
pub fn pass(duration: Duration) -> Duration {
    scaled(duration, Duration::from_millis(50))
}
/// Queue backoffs and loop polling never fall below 20 ms.
pub fn retry(duration: Duration) -> Duration {
    scaled(duration, Duration::from_millis(20))
}
/// Related deadlines share a factor, including their smallest member's floor.
/// This preserves production ordering instead of flooring each deadline alone.
fn family(duration: Duration, smallest: Duration, floor: Duration) -> Duration {
    match scale() {
        Some(factor) if !duration.is_zero() => duration.mul_f64(factor.max(floor.as_secs_f64() / smallest.as_secs_f64())),
        _ => duration,
    }
}

/// Main passes, debounce, polling and background cooldowns share 30 s -> 1 s.
/// Whole-second history cannot represent a smaller debounce. Quantize the
/// factor once, so 30/60/120/300 s retain their ratios after rounding.
pub fn cadence(duration: Duration) -> Duration {
    match scale() {
        Some(factor) => duration.mul_f64((factor * 30.0).ceil() / 30.0),
        None => duration,
    }
}

/// The claim lease of one canonical launch, across all its durable stages.
/// Each stage retakes the exclusive root with a short wait; an operator
/// command that holds the root for its whole preparation (profile evidence,
/// minutes) must not outlast the lease, or the naming intent is never
/// recorded (`start_unnamed`). A dead launch is still recovered at expiry.
/// Labs keep the historical 30 s; `HERDR_FARM_LAUNCH_LEASE_SECS` overrides
/// explicitly (1..=300 s).
pub fn launch_lease() -> Duration {
    if let Some(seconds) = std::env::var("HERDR_FARM_LAUNCH_LEASE_SECS").ok().and_then(|v| v.parse::<u64>().ok()).filter(|s| (1..=300).contains(s)) {
        return Duration::from_secs(seconds);
    }
    scaled(Duration::from_secs(180), Duration::from_secs(30))
}
/// Second-resolution historical timestamps use the shared cadence quantum.
pub fn seconds(seconds: i64) -> i64 {
    let duration = cadence(Duration::from_secs(seconds.max(0) as u64));
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0)) as i64
}
pub fn tick() -> Duration {
    cadence(Duration::from_secs(15))
}
pub fn stop_wait() -> Duration {
    pass(Duration::from_secs(60))
}
pub fn idle_exit() -> Duration {
    pass(Duration::from_secs(300))
}
pub fn lock_retry() -> Duration {
    // Status/start probes hold a real OS lock across process scheduling.
    // Preserve a practical window even when the nominal retry scales to 20 ms.
    retry(Duration::from_secs(1)).max(Duration::from_millis(250))
}

/// Scale a freshness lease while preserving time for unscaled external work.
/// Production values are exact; accelerated leases always outlive one pass.
pub fn lease(duration: Duration, execution: Duration) -> Duration {
    if scale().is_none() {
        return duration;
    }
    pass(duration).max(execution + tick())
}

/// A sidebar batch uses unscaled native calls and yields to the next pass.
pub fn advisory_pass() -> Duration {
    lease(Duration::from_secs(15), Duration::from_secs(15))
}

/// Attention's gap threshold must cover the cadence that actually samples it.
/// Otherwise the preserved native-call window manufactures not_observed gaps.
pub fn collection_interval(duration: Duration) -> Duration {
    if scale().is_none() { return duration; }
    pass(duration).max(advisory_pass())
}

/// A worker idle stretch is sampled on resting passes, so after an advisory
/// batch it is next sampled only once that quiet window has passed. The notice
/// threshold must outlive the window and the passes around it, or a restarted
/// ticker's first re-sample would already qualify.
pub fn worker_idle_threshold(duration: Duration) -> Duration {
    if scale().is_none() { return duration; }
    pass(duration).max(advisory_pass() + 2 * tick())
}

// Default policies live here so CLI and ticker comparisons cannot drift.
pub const STARTING_TIMEOUT_SECS: i64 = 300;
pub const BLOCKED_DEBOUNCE_SECS: i64 = 30;
pub const NOT_READY_SECS: i64 = 60;
pub const PR_INTERVAL_SECS: i64 = 120;
pub const REMOTE_INTERVAL: Duration = Duration::from_secs(60);
pub const REMOTE_RETRY_DELAY: Duration = Duration::from_secs(120);
pub const TELEMETRY_COLLECT_SECS: u64 = 300;
pub const ANALYTICS_INTERVAL_MS: i64 = 60_000;
pub const HEALTH_INTERVAL_MS: i64 = 300_000;

pub fn canonical_pass() -> Duration {
    pass(Duration::from_millis(250))
}
pub fn lock_poll() -> Duration {
    retry(Duration::from_millis(25))
}
pub fn stop_poll() -> Duration {
    pass(Duration::from_millis(250))
}
pub fn wake_poll() -> Duration {
    retry(Duration::from_millis(500))
}
pub fn shutdown_poll() -> Duration {
    retry(Duration::from_millis(50))
}
pub fn queue_retention() -> Duration {
    pass(Duration::from_secs(180))
}
pub fn launch_retry() -> Duration {
    family(Duration::from_secs(1), Duration::from_millis(250), Duration::from_millis(50))
}
pub fn worker_retry() -> Duration {
    family(Duration::from_secs(2), Duration::from_millis(250), Duration::from_millis(50))
}
pub fn job_retry() -> Duration {
    cadence(Duration::from_secs(30))
}
pub fn worker_recovery_retry() -> Duration {
    cadence(Duration::from_secs(15))
}
pub fn routine_retention() -> Duration {
    cadence(Duration::from_secs(120))
}
pub fn observation_lease(execution: Duration) -> Duration {
    lease(Duration::from_secs(60), execution)
}
pub fn pr_retention() -> Duration {
    cadence(Duration::from_secs(120))
}
pub fn pr_pending_retention() -> Duration {
    lease(Duration::from_secs(120), Duration::from_secs(30))
}
pub fn brief_accept_window() -> Duration {
    pass(Duration::from_secs(6))
}
pub fn brief_accept_poll() -> Duration {
    retry(Duration::from_millis(500))
}

/// Store delivery retries use actual UTC deadlines at millisecond precision.
pub fn delivery_backoff(attempts: u32) -> Duration {
    family(Duration::from_millis(
        (1_000_u64 * (1_u64 << attempts.saturating_sub(1).min(8))).min(300_000),
    ), Duration::from_secs(1), Duration::from_millis(20))
}
pub fn legacy_delivery_backoff(attempts: u32, jitter: u64) -> Duration {
    cadence(Duration::from_secs(
        (15_u64 * (1_u64 << attempts.saturating_sub(1).min(5)) + jitter).min(300),
    ))
}
