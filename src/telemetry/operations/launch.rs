//! Metadata-only launch producers. Best effort, zero wait, existing sidecars only.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;

fn writer(project: &Path, table: &str) -> Result<Option<Connection>> {
    let path = crate::telemetry::sidecar::path(project);
    if !path.is_file() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    db.busy_timeout(std::time::Duration::ZERO)?;
    let present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1 AND type='table')",
        [table],
        |r| r.get(0),
    )?;
    Ok(present.then_some(db))
}

/// Typed identifiers only, captured by the local CLI after execution. Worker
/// spool metadata never supplies these trusted launch associations.
pub fn cli_target(
    project: &Path,
    invocation: &str,
    task: Option<&str>,
    attempt: Option<&str>,
    force: bool,
) -> Result<()> {
    let Some(db) = writer(project, "operation_cli_targets")? else {
        return Ok(());
    };
    db.execute("INSERT OR IGNORE INTO operation_cli_targets SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM cli_invocations WHERE invocation_id=?1)", params![invocation, task, attempt, force])?;
    db.execute("DELETE FROM operation_cli_targets WHERE invocation_id NOT IN (SELECT invocation_id FROM cli_invocations)", [])?;
    Ok(())
}

/// One root-wide snapshot per pass, replicated to existing project sidecars.
/// Saturating fixed counters: lock, inventory, ambiguous, permanent, other.
/// Root-wide values must not be summed across projects.
pub fn ticker_errors(
    project: &Path,
    session: &str,
    pass: u64,
    at: i64,
    counts: [u64; 5],
) -> Result<()> {
    anyhow::ensure!(at >= 0 && session.len() <= 128, "invalid ticker sample");
    let Some(mut db) = writer(project, "operation_ticker_errors")? else {
        return Ok(());
    };
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let n = counts.map(|n| n.min(i64::MAX as u64) as i64);
    tx.execute(
        "INSERT OR IGNORE INTO operation_ticker_errors VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            session,
            pass.min(i64::MAX as u64) as i64,
            at,
            n[0],
            n[1],
            n[2],
            n[3],
            n[4]
        ],
    )?;
    tx.execute("DELETE FROM operation_ticker_errors WHERE sampled_unix_ms<?1 OR (session,pass) NOT IN (SELECT session,pass FROM operation_ticker_errors ORDER BY sampled_unix_ms DESC,session,pass DESC LIMIT 100000)", [at.saturating_sub(90*86_400_000)])?;
    tx.commit()?;
    Ok(())
}

/// Persist only a verifier's aggregate numstat, never paths or diff contents.
pub fn submission_diff(
    project: &Path,
    submission: &str,
    counts: Option<(i64, i64, i64)>,
) -> Result<()> {
    let Some(db) = writer(project, "operation_submission_diff")? else {
        return Ok(());
    };
    db.execute("INSERT INTO operation_submission_diff VALUES(?1,?2,?3,?4,?5) ON CONFLICT(submission_id) DO UPDATE SET added=excluded.added,removed=excluded.removed,binary_files=excluded.binary_files,reason=excluded.reason WHERE operation_submission_diff.reason IS NOT NULL", params![submission,counts.map(|n|n.0),counts.map(|n|n.1),counts.map(|n|n.2),counts.is_none().then_some("diff_unavailable")])?;
    Ok(())
}
