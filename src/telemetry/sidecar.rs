//! `<project>/.state/telemetry.db`: analytics only, never read to grant launch.
//! Versioned per stream in `telemetry_streams`: `codex` (the migrations under
//! `migrations/telemetry/`, whose history is also `user_version`) and one
//! stream per lane (`super::LANES`, `migrations/telemetry/<stream>/`). No
//! cross-database transaction with the canonical store (contracts §0).
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const STREAMS_TABLE: &str = "CREATE TABLE IF NOT EXISTS telemetry_streams (stream TEXT PRIMARY KEY, version INTEGER NOT NULL CHECK (version >= 0)) STRICT;";
const CODEX: &[&str] = &[include_str!("../../migrations/telemetry/0001_codex_usage.sql"), include_str!("../../migrations/telemetry/0002_reevaluation.sql"),
    include_str!("../../migrations/telemetry/0003_read_indexes.sql"),
    include_str!("../../migrations/telemetry/0004_compact_native.sql"),
    include_str!("../../migrations/telemetry/0005_usage_schema_unrecognized.sql")];

// Compaction in older binaries could silently remove 0012's capture triggers.
// Check the actual schema even when every migration version is current.
const COMPACT_CAPTURE: &str = include_str!("../../migrations/telemetry/accounting/repair_compact_capture.sql");
const COMPACT_CAPTURE_NAMES: &[&str] = &[
    "accounting_codex_turns_insert",
    "accounting_codex_turns_update",
    "accounting_codex_turns_delete",
    "accounting_codex_rate_limits_insert",
    "accounting_codex_rate_limits_update",
    "accounting_codex_rate_limits_delete",
    "accounting_codex_rate_limit_windows_insert",
    "accounting_codex_rate_limit_windows_update",
    "accounting_codex_rate_limit_windows_delete",
    "accounting_rollout_metadata_insert",
    "accounting_rollout_metadata_update",
    "accounting_rollout_metadata_delete",
    "accounting_rollout_threads_insert",
    "accounting_rollout_threads_update",
    "accounting_rollout_threads_delete",
    "accounting_rollout_forks_insert",
    "accounting_rollout_forks_update",
    "accounting_rollout_forks_delete",
    "accounting_quota_limits_update",
    "accounting_quota_secondary_insert",
    "accounting_quota_secondary_update",
    "accounting_dispositions_update",
];

fn compact_capture_current(db: &Connection) -> Result<bool> {
    let placeholders = vec!["?"; COMPACT_CAPTURE_NAMES.len()].join(",");
    let count: usize = db.query_row(
        &format!("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name IN ({placeholders})"),
        rusqlite::params_from_iter(COMPACT_CAPTURE_NAMES.iter()), |r| r.get(0))?;
    Ok(count == COMPACT_CAPTURE_NAMES.len())
}

/// Every stream and its migrations; index + 1 is the stream version.
fn streams() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("codex", CODEX)).chain(super::LANES.iter().map(|lane| (lane.stream, lane.migrations)))
}

/// Stored stream versions. A sidecar from before streams has only `codex` = `user_version`.
fn versions(db: &Connection) -> Result<BTreeMap<String, usize>> {
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='telemetry_streams')", [], |r| r.get(0))? {
        return Ok(BTreeMap::from([("codex".to_owned(), db.query_row("PRAGMA user_version", [], |r| r.get(0))?)]));
    }
    Ok(db.prepare("SELECT stream,version FROM telemetry_streams")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// Refuse only a stream stored newer than this binary knows (an unknown stream knows 0).
fn check(versions: &BTreeMap<String, usize>) -> Result<()> {
    for (stream, &version) in versions {
        let known = streams().find(|s| s.0 == stream).map_or(0, |s| s.1.len());
        if version > known { bail!("telemetry sidecar stream {stream} version {version} is newer than this binary (knows {known})"); }
    }
    Ok(())
}

/// Stored stream versions, refused when any is newer than this binary knows
/// (TM5.3 backup and restore check a copy with it before exposing it).
pub(crate) fn stream_versions(db: &Connection) -> Result<BTreeMap<String, usize>> {
    let versions = versions(db)?;
    check(&versions)?;
    Ok(versions)
}

/// Whether a sidecar read without migrating has the 0002 `reevaluation` column.
fn reevaluation_column(db: &Connection) -> rusqlite::Result<&'static str> {
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('rollout_sources') WHERE name='reevaluation')", [], |r| r.get(0))?;
    Ok(if present { "s.reevaluation" } else { "NULL" })
}

pub fn path(project: &Path) -> PathBuf {
    project.join(".state").join("telemetry.db")
}

/// Open and migrate the sidecar. Absent and `create` false → `None`. Created mode 0600.
pub fn open(project: &Path, create: bool) -> Result<Option<Connection>> {
    let path = path(project);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() => bail!("telemetry sidecar is not a regular file"),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(None);
            }
            match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW).open(&path) {
                Ok(_) => {},
                // Another first collector won creation after our metadata read.
                // Join its store; retain the regular-file and no-follow checks.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !std::fs::symlink_metadata(&path)?.is_file() { bail!("telemetry sidecar is not a regular file"); }
                }
                Err(error) => return Err(error).context("create telemetry sidecar"),
            }
        }
        Err(error) => return Err(error.into()),
    }
    let mut db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    // New stores can enable page reclamation without a rewriting VACUUM.
    // Existing stores retain their mode; switching them remains an operator
    // maintenance operation because it rewrites the entire file.
    let pages: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    if pages == 0 { db.pragma_update(None, "auto_vacuum", "INCREMENTAL")?; }
    // Changing journal mode takes a read lock before upgrading it. Racing
    // first openers can therefore get SQLITE_BUSY without the busy handler
    // running. Retry the standalone pragma after its read lock is released;
    // migrations themselves acquire their write lock with BEGIN IMMEDIATE.
    let started = std::time::Instant::now();
    loop {
        match db.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => break,
            Err(error) if matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
                && started.elapsed() < std::time::Duration::from_secs(5) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                },
            Err(error) => return Err(error).context("configure telemetry WAL"),
        }
    }
    // Retain FULL durability: valuation revisions and live attention samples
    // are not reproducible from rollouts (certificate-core.md R4).
    db.pragma_update(None, "synchronous", "FULL")?;
    migrate(&mut db).context("migrate telemetry sidecar")?;
    Ok(Some(db))
}

/// Upgrade a writable sidecar, including a private backup copy before retention.
pub(crate) fn migrate(db: &mut Connection) -> Result<()> {
    // Current stores need no write lock. An upgrade still rechecks versions
    // under IMMEDIATE so concurrent openers cannot apply a migration twice.
    let current = stream_versions(db)?;
    if streams().all(|(stream, migrations)| current.get(stream).copied().unwrap_or(0) == migrations.len())
        && super::analytics::inputs::installation_current(db)?
        && compact_capture_current(db)? {
        return Ok(());
    }
    // One transaction: a pre-streams sidecar gains `telemetry_streams` with
    // `codex` = `user_version`, then each stream migrates from its version.
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let versions = versions(&tx)?;
    check(&versions)?;
    tx.execute_batch(STREAMS_TABLE)?;
    let mut upgraded = false;
    for (stream, migrations) in streams() {
        let from = versions.get(stream).copied().unwrap_or(0);
        // A lost logical inventory can rerun older table rebuilds. Their row
        // shape predates the optional Claude columns; retain those columns in
        // TEMP storage inside this same transaction and restore them afterwards.
        let native_extensions = stream == "codex" && from < 5 && has_column(&tx, "codex_usage", "reasoning_reported")?;
        let mut ledger_extensions = false;
        if native_extensions {
            tx.execute_batch("CREATE TEMP TABLE claude_native_extensions AS SELECT session_id,ordinal,cache_write_5m_tokens,cache_write_1h_tokens,reasoning_reported FROM codex_usage;
                CREATE UNIQUE INDEX claude_native_extensions_key ON claude_native_extensions(session_id,ordinal);
                ALTER TABLE codex_usage DROP COLUMN cache_write_5m_tokens;
                ALTER TABLE codex_usage DROP COLUMN cache_write_1h_tokens;
                ALTER TABLE codex_usage DROP COLUMN reasoning_reported;")?;
        }
        if from > 0 { tx.execute("INSERT OR IGNORE INTO telemetry_streams(stream,version) VALUES(?1,?2)", rusqlite::params![stream, from])?; }
        for (index, migration) in migrations.iter().enumerate().skip(from) {
            upgraded = true;
            // Wait until preceding migrations recreate a missing accounting
            // frontier: SQLite ALTER validates triggers referencing that table.
            if !ledger_extensions && stream == "accounting" && matches!(index + 1, 13 | 14 | 18 | 19 | 20)
                && has_column(&tx, "usage_entries", "cache_write_5m_tokens")? {
                ledger_extensions = true;
                tx.execute_batch("CREATE TEMP TABLE claude_ledger_extensions AS SELECT entry_id,cache_write_5m_tokens,cache_write_1h_tokens FROM usage_entries;
                    CREATE UNIQUE INDEX claude_ledger_extensions_key ON claude_ledger_extensions(entry_id);
                    ALTER TABLE usage_entries DROP COLUMN cache_write_5m_tokens;
                    ALTER TABLE usage_entries DROP COLUMN cache_write_1h_tokens;")?;
            }
            // These storage migrations are already installed when a legacy
            // stream inventory was lost. Keep the logical views and their rows.
            let compact = match (stream, index + 1) {
                ("ingest", 12) => Some("source_observations"),
                ("analytics", 4) => Some("analytics_lineage"),
                _ => None,
            };
            let installed = matches!((stream, index + 1), ("ingest", 14)) && has_column(&tx, "codex_usage", "reasoning_reported")?
                || matches!((stream, index + 1), ("accounting", 23)) && has_column(&tx, "usage_entries", "cache_write_5m_tokens")?
                || compact.map(|name| tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='view' AND name=?1)",
                [name], |r| r.get::<_, bool>(0))).transpose()?.unwrap_or(false);
            if !installed {
                // Rebuilding native tables drops their triggers. Preserve the
                // original accounting/frontier SQL, then reinstall it on the
                // replacement tables in this same migration transaction.
                let triggers: Vec<(String, String)> = if matches!((stream, index + 1), ("codex", 4 | 5) | ("ingest", 12) | ("accounting", 17)) {
                    tx.prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND sql IS NOT NULL")?
                        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
                } else { Vec::new() };
                // OTLP capture writes across normalized tables. Suspend those
                // triggers while compact migrations temporarily replace targets.
                for (name, _) in &triggers {
                    if name.starts_with("accounting_otlp_") {
                        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))?;
                    }
                }
                tx.execute_batch(migration)?;
                for (name, sql) in triggers {
                    if name.starts_with("analytics_input_source_observations_") { continue; }
                    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1)", [&name], |r| r.get(0))?;
                    if !exists { tx.execute_batch(&sql)?; }
                }
            }
            tx.execute("INSERT INTO telemetry_streams(stream,version) VALUES(?1,?2) ON CONFLICT(stream) DO UPDATE SET version=excluded.version",
                rusqlite::params![stream, index + 1])?;
        }
        if native_extensions {
            tx.execute_batch("ALTER TABLE codex_usage ADD COLUMN cache_write_5m_tokens INTEGER CHECK (cache_write_5m_tokens BETWEEN 0 AND 9007199254740992);
                ALTER TABLE codex_usage ADD COLUMN cache_write_1h_tokens INTEGER CHECK (cache_write_1h_tokens BETWEEN 0 AND 9007199254740992);
                ALTER TABLE codex_usage ADD COLUMN reasoning_reported INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_reported IN (0,1));
                UPDATE codex_usage SET (cache_write_5m_tokens,cache_write_1h_tokens,reasoning_reported)=(SELECT cache_write_5m_tokens,cache_write_1h_tokens,reasoning_reported FROM claude_native_extensions x WHERE x.session_id=codex_usage.session_id AND x.ordinal=codex_usage.ordinal) WHERE EXISTS(SELECT 1 FROM claude_native_extensions x WHERE x.session_id=codex_usage.session_id AND x.ordinal=codex_usage.ordinal);
                DROP TABLE claude_native_extensions;")?;
        }
        if ledger_extensions {
            tx.execute_batch("UPDATE usage_entries SET (cache_write_5m_tokens,cache_write_1h_tokens)=(SELECT cache_write_5m_tokens,cache_write_1h_tokens FROM claude_ledger_extensions x WHERE x.entry_id=usage_entries.entry_id);
                DROP TABLE claude_ledger_extensions;")?;
        }
    }
    if !compact_capture_current(&tx)? {
        tx.execute_batch(COMPACT_CAPTURE)?;
        // Inputs committed while capture was absent cannot be reconstructed
        // from the dirty queue. Replay once before trusting incremental sync.
        super::accounting::ledger::invalidate(&tx, "capture_repaired")?;
    }
    if upgraded { super::accounting::ledger::invalidate(&tx, "schema_upgrade")?; }
    super::analytics::inputs::install(&tx)?;
    tx.commit()?;
    Ok(())
}

/// The sidecar opened strictly read-only (`super::read_only`); absent → `None`.
pub(crate) fn read(project: &Path) -> Result<Option<super::ReadOnly>> {
    let path = path(project);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_file() => bail!("telemetry sidecar is not a regular file"),
        Ok(_) => {}
    }
    let db = super::read_only(&path)?;
    let versions = versions(&db)?;
    check(&versions)?;
    if versions.get("codex").is_none_or(|&version| version == 0) {
        bail!("telemetry sidecar schema 0 is not readable by this binary");
    }
    Ok(Some(db))
}

/// `telemetry <slug> <lane> status`: the stream's stored version. Read-only.
pub fn status(project: &Path, stream: &str) -> Result<Value> {
    let version = match read(project)? {
        None => unavailable("collection_not_run"),
        Some(db) => json!(versions(&db)?.get(stream).copied().unwrap_or(0)),
    };
    Ok(json!({"stream": stream, "version": version}))
}

/// Per-attempt usage (contracts §4 `usage`) and per-rollout metadata. Read-only.
pub fn report(project: &Path) -> Result<Value> {
    let attempts = super::codex::canonical_attempts(project)?;
    let Some(db) = read(project)? else {
        let attempts = attempts.iter().map(|a| json!({"attempt_id": a.id, "after_termination": if a.terminated_unix_ms().is_some() { unavailable("collection_not_run") } else { Value::Null }, "usage": unavailable(if a.supported() { "collection_not_run" } else { "adapter_absent" })}));
        return Ok(json!({"attempts": attempts.collect::<Vec<_>>(), "sessions": []}));
    };
    let mut sessions = Vec::new();
    let mut stmt = db.prepare(&format!("SELECT s.session_id,s.binding,s.attempt_id,s.cli_version,s.records,s.cwd,s.originator,s.source,
        (SELECT count(*) FROM codex_usage u WHERE u.session_id=s.session_id AND u.accepted=1),
        EXISTS(SELECT 1 FROM codex_quarantine q WHERE q.session_id=s.session_id),{}
        FROM rollout_sources s ORDER BY s.session_id,s.path_digest", reevaluation_column(&db)?))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?,
        r.get::<_, i64>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, Option<String>>(7)?, r.get::<_, i64>(8)?, r.get::<_, bool>(9)?,
        r.get::<_, Option<String>>(10)?)))?;
    for row in rows {
        let (session, binding, attempt, version, records, cwd, originator, source, accepted, quarantined, reevaluation) = row?;
        let mut row = json!({"session_id": session, "binding": binding, "attempt_id": attempt, "cli_version": version,
            "certified": super::codex::accepted_version(&version), "records": records, "accepted": accepted, "quarantined": quarantined,
            "cwd": cwd, "originator": originator, "source": source, "reevaluation": reevaluation});
        if originator.as_deref() == Some("opencode") {
            row["adapter"] = json!("opencode");
            row["certification"] = json!(if super::codex::accepted_version(&version) { "fixture" } else { "none" });
        }
        if originator.as_deref() == Some("claude-code") {
            row["adapter"] = json!("claude-code");
            row["certification"] = json!(if super::codex::accepted_version(&version) { "fixture" } else { "none" });
        }
        if super::version::nearest(&version).is_some() {
            for (k, v) in super::version::provenance(&version).as_object().unwrap() { row[k] = v.clone(); }
        }
        sessions.push(row);
    }
    let mut out = Vec::new();
    let children = child_index(&db)?;
    for attempt in &attempts {
        let usage = if attempt.supported() || (attempt.grok() && db.query_row("SELECT EXISTS(SELECT 1 FROM rollout_sources WHERE attempt_id=?1 AND originator IN ('otlp:grok','otlp:devin') AND binding='bound')", [&attempt.id], |r| r.get::<_, bool>(0))?) { attempt_usage_with(&db, &attempt.id, &children)? } else { unavailable("adapter_absent") };
        let after_termination = after_termination(&db, &attempt.id, attempt.terminated_unix_ms())?;
        out.push(json!({"attempt_id": attempt.id, "usage": usage, "after_termination": after_termination}));
    }
    Ok(json!({"attempts": out, "sessions": sessions}))
}

/// Central reports need only the diagnostic after-termination rows, not the
/// complete native usage projection for every retained attempt.
pub(crate) fn after_termination_report(project: &Path) -> Result<Vec<Value>> {
    let Some(db) = read(project)? else { return Ok(Vec::new()); };
    if db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='accounting_termination_summary')", [], |r| r.get::<_, bool>(0))?
        && let Some(generations) = super::analytics::inputs::generations(&db)? {
        let stamp = super::analytics::inputs::stamp("diagnostics", &super::analytics::inputs::canonical(project)?, &generations);
        let body: Option<String> = db.query_row("SELECT body FROM accounting_termination_summary WHERE inputs=?1", [&stamp], |r| r.get(0)).optional()?;
        if let Some(body) = body { return Ok(serde_json::from_str(&body)?); }
    }
    termination_summary_raw(project, &db)
}

fn termination_summary_raw(project: &Path, db: &Connection) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for attempt in super::codex::canonical_attempts(project)? {
        let diagnostic = after_termination(db, &attempt.id, attempt.terminated_unix_ms())?;
        if diagnostic["records"].as_i64().is_some_and(|n| n > 0) {
            out.push(json!({"attempt_id": attempt.id, "after_termination": diagnostic, "accounting": "still counted in M08"}));
        }
    }
    Ok(out)
}

pub(crate) fn store_termination_summary(project: &Path, db: &Connection) -> Result<()> {
    let generations = super::analytics::inputs::generations(db)?.unwrap_or_default();
    let stamp = super::analytics::inputs::stamp("diagnostics", &super::analytics::inputs::canonical(project)?, &generations);
    if db.query_row("SELECT EXISTS(SELECT 1 FROM accounting_termination_summary WHERE inputs=?1)", [&stamp], |r| r.get::<_, bool>(0))? { return Ok(()); }
    let body = termination_summary_raw(project, db)?;
    db.execute("INSERT INTO accounting_termination_summary VALUES(1,?1,?2)
        ON CONFLICT(singleton) DO UPDATE SET inputs=excluded.inputs,body=excluded.body", rusqlite::params![stamp, serde_json::to_string(&body)?])?;
    Ok(())
}

/// `report` for the terminal: one line per attempt, then one per rollout.
pub fn text(report: &Value) -> String {
    let word = |v: &Value| v.as_str().map_or_else(|| if v.is_null() { "-".to_owned() } else { v.to_string() }, str::to_owned);
    let mut out = String::new();
    for attempt in report["attempts"].as_array().into_iter().flatten() {
        let usage = &attempt["usage"];
        let shown = if usage.get("total_tokens").is_some() {
            format!("in={} out={} total={} records={}", usage["input_tokens"], usage["output_tokens"], usage["total_tokens"], usage["records"])
        } else {
            format!("{}:{}", word(&usage["status"]), word(&usage["reason"]))
        };
        out += &format!("{} usage={shown} after_termination={}\n", word(&attempt["attempt_id"]), attempt["after_termination"]);
    }
    for s in report["sessions"].as_array().into_iter().flatten() {
        out += &format!("session {} binding={} attempt={} cli={} certified={} records={} accepted={} quarantined={}", word(&s["session_id"]), word(&s["binding"]),
            word(&s["attempt_id"]), word(&s["cli_version"]), s["certified"], s["records"], s["accepted"], s["quarantined"]);
        if !s["reevaluation"].is_null() { out += &format!(" reevaluation={}", word(&s["reevaluation"])); }
        out += "\n";
    }
    out
}

/// Diagnostic only: every usage record on a bound rollout remains in M08.
pub(super) fn after_termination(db: &Connection, attempt: &str, terminated: Option<i64>) -> Result<Value> {
    let Some(at) = terminated else { return Ok(Value::Null) };
    let timed: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='codex_usage_times')", [], |r| r.get(0))?;
    if !timed { return Ok(unavailable("predates_collection")); }
    let (records, first): (i64, Option<i64>) = db.query_row("SELECT count(*),min(t.record_unix_ms) FROM codex_usage u JOIN codex_usage_times t USING(session_id,ordinal)
        WHERE t.record_unix_ms>?2 AND EXISTS(SELECT 1 FROM rollout_sources s WHERE s.path_digest=u.path_digest AND s.binding='bound' AND s.attempt_id=?1)",
        rusqlite::params![attempt, at], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(if records == 0 { Value::Null } else { json!({"records": records, "first_unix_ms": first, "terminated_unix_ms": at}) })
}

/// Health's diagnostic count, using the same per-attempt after-termination
/// rule without constructing unrelated usage/session JSON for all history.
pub(crate) fn after_termination_summary(project: &Path) -> Result<Value> {
    let attempts = super::codex::canonical_attempts(project)?;
    let Some(db) = read(project)? else {
        return Ok(json!({"records": 0, "attempts": 0, "missing_reason": "collection_not_run"}));
    };
    let (mut records, mut affected, mut missing) = (0i64, 0usize, None::<String>);
    for attempt in attempts {
        let value = after_termination(&db, &attempt.id, attempt.terminated_unix_ms())?;
        let n = value["records"].as_i64().unwrap_or(0);
        records += n;
        affected += usize::from(n > 0);
        if value["status"] == "unavailable" && missing.is_none() {
            missing = value["reason"].as_str().map(str::to_owned);
        }
    }
    Ok(json!({"records": records, "attempts": affected, "missing_reason": missing}))
}

/// A newer schema that failed typed ingestion cannot yield a complete sum.
pub(crate) fn malformed_newer(db: &Connection, session: &str, version: &str) -> rusqlite::Result<bool> {
    if super::version::nearest(version).is_none() { return Ok(false); }
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ingest_quarantine')", [], |r| r.get(0))?;
    if !present { return Ok(false); }
    db.query_row("SELECT EXISTS(SELECT 1 FROM ingest_quarantine q JOIN rollout_sources s ON s.path_digest=q.source
        WHERE s.session_id=?1 AND q.reason IN ('line_malformed','record_malformed'))", [session], |r| r.get(0))
}

/// All usage records of a compatible newer source must be recognized.
pub(crate) fn incomplete_newer(db: &Connection, session: &str, version: &str) -> rusqlite::Result<bool> {
    if super::version::nearest(version).is_none() { return Ok(false); }
    if malformed_newer(db, session, version)? { return Ok(true); }
    db.query_row("SELECT NOT EXISTS(SELECT 1 FROM codex_usage WHERE session_id=?1)
        OR EXISTS(SELECT 1 FROM codex_usage WHERE session_id=?1 AND accepted=0)", [session], |r| r.get(0))
}

const USAGE_KEYS: [&str; 7] = ["records", "input_tokens", "cached_input_tokens", "cache_write_input_tokens", "output_tokens", "reasoning_output_tokens", "total_tokens"];

fn zero_usage() -> Value {
    json!({"input_tokens": 0, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": 0, "records": 0})
}

fn add_usage(total: &mut Value, part: &Value) {
    for key in USAGE_KEYS {
        total[key] = match (total[key].as_i64(), part[key].as_i64()) { (Some(a), Some(b)) => json!(a + b), _ => unavailable("incomplete") };
    }
}

/// Child usage of every attempt of a project, built once: native membership,
/// session roles and the delta ledger are each read a single time, so a
/// report over every attempt does not rebuild the graph per attempt.
pub(crate) struct ChildIndex {
    /// attempt -> (primary sessions' ledger usage, included children, excluded children)
    by_attempt: BTreeMap<String, (Value, Value, Vec<Value>)>,
    empty: bool,
}

pub(crate) fn child_index(db: &Connection) -> Result<ChildIndex> {
    let membership = super::accounting::graph::attempt_membership(db)?;
    if membership.is_empty() { return Ok(ChildIndex { by_attempt: BTreeMap::new(), empty: true }); }
    let roles: BTreeMap<String, String> = db.prepare("SELECT session_id, role FROM session_graph_nodes")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let ledger = super::accounting::ledger::read(db)?;
    let mut by_session: BTreeMap<&str, Value> = BTreeMap::new();
    for entry in ledger["entries"].as_array().into_iter().flatten().filter(|e| e["basis"] == "delta") {
        if !entry["provenance"].as_array().into_iter().flatten().any(|p| p["disposition"] == "accepted") { continue; }
        let Some(session) = entry["session_id"].as_str() else { continue };
        let usage = by_session.entry(session).or_insert_with(zero_usage);
        for (key, native) in [("input_tokens", "input_tokens"), ("cached_input_tokens", "cache_read_tokens"),
            ("cache_write_input_tokens", "cache_write_tokens"), ("output_tokens", "output_tokens"),
            ("reasoning_output_tokens", "reasoning_tokens"), ("total_tokens", "total_tokens")] {
            usage[key] = match (usage[key].as_i64(), entry["normalized"][native].as_i64()) { (Some(a), Some(b)) => json!(a + b), _ => unavailable("incomplete") };
        }
        usage["records"] = json!(usage["records"].as_i64().unwrap_or(0) + 1);
    }
    let mut by_attempt: BTreeMap<String, (Value, Value, Vec<Value>)> = BTreeMap::new();
    for (session, (owner, reason)) in membership {
        let Some(owner) = owner else { continue };
        let slot = by_attempt.entry(owner).or_insert_with(|| (zero_usage(), zero_usage(), Vec::new()));
        let role = roles.get(&session).cloned().unwrap_or_default();
        if let Some(reason) = reason { slot.2.push(json!({"session_id": session, "role": role, "reason": reason})); continue; }
        let usage = by_session.get(session.as_str()).cloned().unwrap_or_else(zero_usage);
        if role == "primary" { add_usage(&mut slot.0, &usage); } else { add_usage(&mut slot.1, &usage); }
    }
    Ok(ChildIndex { by_attempt, empty: false })
}

pub(super) fn attempt_usage(db: &Connection, attempt: &str) -> Result<Value> {
    attempt_usage_with(db, attempt, &child_index(db)?)
}

/// [`attempt_usage`] with a [`ChildIndex`] built once for the project.
pub(crate) fn attempt_usage_with(db: &Connection, attempt: &str, index: &ChildIndex) -> Result<Value> {
    let mut primary = primary_attempt_usage(db, attempt)?;
    // An unavailable usage (e.g. `schema_unrecognized`) stays exactly as it is.
    if index.empty || primary["status"] == "unavailable" { return Ok(primary); }
    // Attempts without child sessions keep exactly the primary-only shape.
    let Some((own, children, excluded)) = index.by_attempt.get(attempt).cloned() else { return Ok(primary) };
    if children["records"] == 0 && excluded.is_empty() { return Ok(primary); }
    let mut total = own;
    add_usage(&mut total, &children);
    primary["children"] = children;
    primary["including_children"] = total;
    primary["excluded_children"] = json!(excluded);
    Ok(primary)
}

fn primary_attempt_usage(db: &Connection, attempt: &str) -> Result<Value> {
    // Records collected before their version was certified keep NULL counters
    // until a collect re-reads their rollout, so the session stays uncertified
    // rather than summing to 0; `detail` says when that rollout is gone.
    let bound: Vec<(String, String, bool, Option<String>)> = db.prepare_cached(&format!("SELECT DISTINCT session_id,
        CASE WHEN EXISTS(SELECT 1 FROM codex_usage u WHERE u.session_id=s.session_id AND u.reason='cli_version_uncertified') THEN '' ELSE cli_version END,
        EXISTS(SELECT 1 FROM codex_quarantine q WHERE q.session_id=s.session_id),{}
        FROM rollout_sources s WHERE binding='bound' AND attempt_id=?1", reevaluation_column(db)?))?
        .query_map([attempt], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    if bound.is_empty() {
        return Ok(unavailable("not_bound"));
    }
    for (session, version, ..) in &bound {
        if malformed_newer(db, session, version)? {
            let mut usage = unavailable("schema_unrecognized");
            usage["cli_version"] = json!(version);
            for (k, v) in super::version::provenance(version).as_object().unwrap() { usage[k] = v.clone(); }
            return Ok(usage);
        }
    }
    if bound.iter().any(|s| s.2) {
        return Ok(unavailable("quarantined"));
    }
    if bound.iter().any(|s| !super::codex::accepted_version(&s.1)) {
        let mut usage = unavailable("cli_version_uncertified");
        if let Some(detail) = bound.iter().filter(|s| !super::codex::accepted_version(&s.1)).find_map(|s| s.3.clone()) { usage["detail"] = json!(detail); }
        return Ok(usage);
    }
    for (session, version, ..) in bound.iter().filter(|s| super::version::nearest(&s.1).is_some()) {
        let has_usage: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM codex_usage WHERE session_id=?1)", [session], |r| r.get(0))?;
        if !has_usage {
            let mut usage = unavailable("no_usage_records");
            usage["cli_version"] = json!(version);
            for (k, v) in super::version::provenance(version).as_object().unwrap() { usage[k] = v.clone(); }
            return Ok(usage);
        }
    }
    // A record that failed validation keeps no counters: summing the rest
    // would present an unknown total as known (contracts §0).
    let rejected: bool = db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM codex_usage WHERE accepted=0 AND session_id IN ({}))",
        vec!["?"; bound.len()].join(",")), rusqlite::params_from_iter(bound.iter().map(|s| &s.0)), |r| r.get(0))?;
    if rejected {
        for (session, version, ..) in &bound {
            if db.query_row("SELECT EXISTS(SELECT 1 FROM codex_usage WHERE session_id=?1 AND reason='schema_unrecognized')", [session], |r| r.get::<_, bool>(0))? {
                let mut value = unavailable("schema_unrecognized");
                value["cli_version"] = json!(version);
                for (k, v) in super::version::provenance(version).as_object().unwrap() { value[k] = v.clone(); }
                return Ok(value);
            }
        }
        return Ok(unavailable("records_not_accepted"));
    }
    let losing = if bound.iter().any(|s| s.0.starts_with("otlp:")) {
        super::accounting::otlp::losing_sessions(db)?
    } else { std::collections::BTreeSet::new() };
    let mut sums = [0i64; 6];
    let mut records = 0;
    for (session, ..) in bound.iter().filter(|s| !losing.contains(&s.0)) {
        let row: Option<[i64; 7]> = db.prepare_cached("SELECT count(*),sum(input_tokens),sum(cached_input_tokens),sum(cache_write_input_tokens),
            sum(output_tokens),sum(reasoning_output_tokens),sum(total_tokens) FROM codex_usage WHERE session_id=?1 AND accepted=1 AND NOT EXISTS(SELECT 1 FROM codex_usage e WHERE e.session_id=codex_usage.session_id AND e.accepted=1 AND e.response_id IS NOT NULL AND e.response_id=codex_usage.response_id AND unhex(substr(e.payload_digest,8)) IS unhex(substr(codex_usage.payload_digest,8)) AND e.payload_digest=codex_usage.payload_digest AND e.ordinal<codex_usage.ordinal)")?.query_row([session],
            |r| Ok([r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                r.get::<_, Option<i64>>(4)?.unwrap_or(0), r.get::<_, Option<i64>>(5)?.unwrap_or(0), r.get::<_, Option<i64>>(6)?.unwrap_or(0)])).optional()?;
        if let Some(row) = row {
            records += row[0];
            for (sum, value) in sums.iter_mut().zip(&row[1..]) { *sum += value; }
        }
    }
    let reasoning = if bound.iter().filter(|s| !losing.contains(&s.0)).map(|s| reasoning_missing(db, &s.0)).collect::<rusqlite::Result<Vec<_>>>()?.into_iter().any(|missing| missing) { unavailable("reasoning_tokens_not_reported") } else { json!(sums[4]) };
    let mut usage = json!({"input_tokens": sums[0], "cached_input_tokens": sums[1], "cache_write_input_tokens": sums[2],
        "output_tokens": sums[3], "reasoning_output_tokens": reasoning, "total_tokens": sums[5], "records": records});
    if let Some((_, version, ..)) = bound.iter().find(|s| super::version::nearest(&s.1).is_some()) {
        for (k, v) in super::version::provenance(version).as_object().unwrap() { usage[k] = v.clone(); }
    }
    Ok(usage)
}

fn unavailable(reason: &str) -> Value {
    json!({"status": "unavailable", "reason": reason})
}

/// Missing reasoning metadata is not a reported zero, including legacy sidecars.
pub(crate) fn reasoning_missing(db: &Connection, session: &str) -> rusqlite::Result<bool> {
    if session.starts_with("otlp:claude-code:") { return Ok(true); }
    if !session.starts_with("claude-code:") { return Ok(false); }
    if !has_column(db, "codex_usage", "reasoning_reported")? { return Ok(true); }
    db.query_row("SELECT EXISTS(SELECT 1 FROM codex_usage WHERE session_id=?1 AND accepted=1 AND reasoning_reported=0)", [session], |r| r.get(0))
}

/// Legacy sidecars remain readable without a writable schema upgrade.
pub(crate) fn has_column(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)", [table, column], |r| r.get(0))
}
