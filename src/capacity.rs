//! Owner-visible canonical capacity; reporting never changes policy.
use anyhow::Result;
use std::path::Path;

pub fn report(project: &Path, slug: &str) -> Result<(String, bool)> {
    let queue = herdr_farm::runtime::queue_report(project)?;
    let text = crate::paths::read_control_text(&project.join("PROJECT.md"), 1024 * 1024)?
        .ok_or_else(|| anyhow::anyhow!("PROJECT.md missing"))?;
    let (settings, _) = crate::project::parse_project_md(&text)?;
    let policy = &queue.policy;
    let mismatch = settings.max_parallel_threads != policy.max_active_workers;
    let mut out = format!("Canonical capacity: max_active_workers={} active={} available_slots={} (scheduler policy revision {}). Admission is atomic; submit launches without a coordinator capacity self-check.", policy.max_active_workers, queue.retained_attempts, queue.available_slots, policy.revision);
    if mismatch {
        let prefix = crate::coordinator::current_prefix(project.parent().ok_or_else(|| anyhow::anyhow!("project root missing"))?)?;
        out.push_str(&format!("\nCapacity warning: PROJECT.md max_parallel_threads={} is advisory for canonical projects; the enforced cap is {}. Existing policy is preserved. To align the enforced cap with PROJECT.md, run `{prefix} scheduler {slug} policy --max-active-workers {} --max-attempts-per-task {} --expected-revision {} --expected-head {}` (refresh `{prefix} scheduler {slug} inspect` if stale). To keep the enforced cap, edit PROJECT.md max_parallel_threads to {}.", settings.max_parallel_threads, policy.max_active_workers, settings.max_parallel_threads, policy.max_attempts_per_task, policy.revision, queue.head, policy.max_active_workers));
    }
    Ok((out, mismatch))
}
