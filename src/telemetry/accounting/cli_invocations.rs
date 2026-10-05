//! Best-effort product CLI metadata, never command arguments or diagnostics.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};

pub const MAX_ROWS: i64 = 100_000;
pub const MAX_AGE_MS: i64 = 90 * 86_400_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub invocation_id: String,
    pub command_path: String,
    pub outcome: String,
    pub exit_code: i32,
    pub duration_ms: i64,
    pub project_slug: Option<String>,
    pub caller: String,
    pub trust: String,
    pub recorded_unix_ms: i64,
}

/// Open an existing current accounting stream without migration, maintenance
/// admission, a busy wait, or creation (including when the path disappears).
pub fn open_current(project: &Path) -> Result<Option<Connection>> {
    let path = super::super::sidecar::path(project);
    if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return Ok(None);
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    db.busy_timeout(Duration::ZERO)?;
    let version: Option<usize> = db
        .query_row(
            "SELECT version FROM telemetry_streams WHERE stream='accounting'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok((version == Some(super::MIGRATIONS.len())).then_some(db))
}

/// One atomic zero-wait batch. Worker replay is idempotent by invocation ID.
pub fn write(project: &Path, rows: &[Invocation]) -> Result<bool> {
    let Some(mut db) = open_current(project)? else {
        return Ok(false);
    };
    // A shortened retention policy must not be undone by replaying an old
    // append-only worker report. Read only the class cutoff, without waiting.
    let ops_path = super::super::maintenance::store::path(project);
    let before: Option<i64> = if ops_path.try_exists()? {
        let ops = super::super::read_only_nowait(&ops_path)?;
        ops.query_row(
            "SELECT max(before_unix_ms) FROM tombstones WHERE class=?1 AND key_digest IS NULL",
            [super::super::maintenance::CLI],
            |r| r.get(0),
        )?
    } else {
        None
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for row in rows {
        if before.is_some_and(|cutoff| row.recorded_unix_ms < cutoff) {
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO cli_invocations VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                row.invocation_id,
                row.command_path,
                row.outcome,
                row.exit_code,
                row.duration_ms,
                row.project_slug,
                row.caller,
                row.trust,
                row.recorded_unix_ms
            ],
        )?;
    }
    let now = super::super::maintenance::store::now();
    tx.execute(
        "DELETE FROM cli_invocations WHERE recorded_unix_ms < ?1",
        [now - MAX_AGE_MS],
    )?;
    tx.execute("DELETE FROM cli_invocations WHERE invocation_id IN (SELECT invocation_id FROM cli_invocations ORDER BY recorded_unix_ms DESC, invocation_id DESC LIMIT -1 OFFSET ?1)", [MAX_ROWS])?;
    tx.commit()?;
    Ok(true)
}

/// Per-caller totals and caller/command detail; nearest-rank percentiles.
pub fn read(project: &Path) -> Result<Value> {
    let Some(db) = super::super::sidecar::read(project)? else {
        return Ok(json!({"status":"unavailable","reason":"collection_not_run"}));
    };
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cli_invocations' AND type='table')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(json!({"status":"unavailable","reason":"stream_upgrade_required"}));
    }
    let rows = db.prepare("SELECT caller,command_path,outcome,duration_ms FROM cli_invocations WHERE recorded_unix_ms>=?1 ORDER BY caller,command_path")?
        .query_map([super::super::maintenance::store::now() - MAX_AGE_MS], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut callers = BTreeMap::<String, Vec<(String, i64)>>::new();
    let mut commands = BTreeMap::<(String, String), Vec<(String, i64)>>::new();
    for (caller, path, outcome, duration) in rows {
        callers
            .entry(caller.clone())
            .or_default()
            .push((outcome.clone(), duration));
        commands
            .entry((caller, path))
            .or_default()
            .push((outcome, duration));
    }
    let summarize = |rows: &mut Vec<(String, i64)>| {
        rows.sort_by_key(|r| r.1);
        let n = rows.len();
        let count = |k: &str| rows.iter().filter(|r| r.0 == k).count();
        json!({"invocations":n,"help_share":format!("{}/{n}", count("help")),
            "error_share":format!("{}/{n}", count("error")+count("usage_error")),
            "p50_duration_ms":rows[(n*50).div_ceil(100)-1].1,
            "p95_duration_ms":rows[(n*95).div_ceil(100)-1].1})
    };
    Ok(
        json!({"status":"available","callers":callers.iter_mut().map(|(caller,rows)| {
        let mut v=summarize(rows); v["caller"]=json!(caller); v
    }).collect::<Vec<_>>(),"commands":commands.iter_mut().map(|((caller,path),rows)| {
        let mut v=summarize(rows); v["caller"]=json!(caller); v["command_path"]=json!(path); v
    }).collect::<Vec<_>>() }),
    )
}
