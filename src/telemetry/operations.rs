//! MET-NOW-B: descriptive flow and operations metadata. Never grants work.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::{BTreeMap, BTreeSet}, io::Read, path::Path};

pub const STREAM: &str = "operations";
pub const MIGRATIONS: &[&str] = &[include_str!("../../migrations/telemetry/operations/0001_samples.sql"), include_str!("../../migrations/telemetry/operations/0002_launch_metrics.sql")];
pub mod launch;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
pub const STORAGE_ROWS: i64 = 2160; // 90 days at hourly cadence

fn table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |r| r.get(0))?)
}
fn unavailable(reason: &str) -> Value { json!({"status":"unavailable","reason":reason}) }
fn summary(mut values: Vec<i64>) -> Value {
    values.sort_unstable();
    let rank = |p: usize| values.get((values.len() * p).div_ceil(100).saturating_sub(1)).copied();
    if values.is_empty() { return json!({"status":"unavailable","reason":"no_phase_samples","count":0,"median_ms":null,"p90_ms":null,"total_ms":null}); }
    json!({"count":values.len(),"median_ms":rank(50),"p90_ms":rank(90),"total_ms":values.iter().map(|v| i128::from(*v)).sum::<i128>()})
}

/// Best effort, existing current sidecar only, zero SQLite wait. The first
/// launch-start sample is immutable; recovery cannot replace it with later load.
pub(crate) fn sample_launch(project: &Path, attempt: &str) {
    let capture = || -> Result<()> {
        let path = super::sidecar::path(project);
        if !path.is_file() { return Ok(()); }
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
        db.busy_timeout(std::time::Duration::ZERO)?;
        if !table(&db, "operation_launch_load")? { return Ok(()); }
        let version: Option<i64> = db.query_row("SELECT version FROM telemetry_streams WHERE stream='operations'", [], |r| r.get(0)).optional()?;
        if version != Some(MIGRATIONS.len() as i64) { return Ok(()); }
        let at = jiff::Timestamp::now().as_millisecond();
        let mut text = String::new();
        let load = std::fs::File::open("/proc/loadavg").ok().and_then(|file| {
            file.take(4097).read_to_string(&mut text).ok()?;
            if text.len() > 4096 { return None; }
            let fields: Vec<String> = text.split_whitespace().take(3).map(str::to_owned).collect();
            (fields.len() == 3 && fields.iter().all(|s| s.parse::<f64>().is_ok_and(|v| v.is_finite() && v >= 0.0))).then_some(fields)
        });
        db.execute("INSERT OR IGNORE INTO operation_launch_load VALUES(?1,?2,?3,?4,?5,?6)",
            params![attempt,at,load.as_ref().map(|v| &v[0]),load.as_ref().map(|v| &v[1]),load.as_ref().map(|v| &v[2]),load.is_none().then_some("unreadable_or_invalid")])?;
        Ok(())
    };
    let _ = capture();
}

/// Sum regular-file logical bytes without reading content or following links.
/// Missing trees are zero; unreadable/incomplete scans are unknown. Bound the
/// traversal so an hourly sample cannot monopolize the telemetry worker.
fn bytes(path: &Path, budget: &mut usize) -> Option<i64> {
    if *budget == 0 { return None; }
    *budget -= 1;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(0),
        Err(_) => return None,
    };
    if meta.is_symlink() { return Some(0); }
    if meta.is_file() { return i64::try_from(meta.len()).ok(); }
    if !meta.is_dir() { return Some(0); }
    let mut sum = 0i64;
    for entry in std::fs::read_dir(path).ok()? {
        sum = sum.checked_add(bytes(&entry.ok()?.path(), budget)?)?;
    }
    Some(sum)
}

/// Public producer for deterministic ticker labs. Never creates a sidecar.
/// Admission is repeated in the writer transaction to prevent racing samples.
pub fn sample_storage(project: &Path, at: i64) -> Result<()> {
    anyhow::ensure!(at >= 0, "invalid storage sample time");
    let Some(db) = super::sidecar::read(project)? else { return Ok(()); };
    if !table(&db, "operation_storage_samples")? { return Ok(()); }
    let _maintenance = super::maintenance::lock(project, false)?;
    let tombstones = super::maintenance::Tombstones::of(project)?;
    if tombstones.before.get(super::maintenance::STORAGE).is_some_and(|before| at < *before) { return Ok(()); }
    let last: Option<i64> = db.query_row("SELECT max(sampled_unix_ms) FROM operation_storage_samples", [], |r| r.get(0))?;
    if last.is_some_and(|last| at.saturating_sub(last) < HOUR_MS) { return Ok(()); }
    drop(db);
    let sizes: Vec<_> = ["state.db", "telemetry.db", "worktrees", "worker-output"].into_iter()
        .map(|name| bytes(&project.join(".state").join(name), &mut 100_000)).collect();
    let Some(mut db) = super::sidecar::open(project, false)? else { return Ok(()); };
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let last: Option<i64> = tx.query_row("SELECT max(sampled_unix_ms) FROM operation_storage_samples", [], |r| r.get(0))?;
    if last.is_some_and(|last| at.saturating_sub(last) < HOUR_MS) { return Ok(()); }
    tx.execute("INSERT INTO operation_storage_samples VALUES(?1,?2,?3,?4,?5)", params![at,sizes[0],sizes[1],sizes[2],sizes[3]])?;
    tx.execute("DELETE FROM operation_storage_samples WHERE sampled_unix_ms < ?1 OR sampled_unix_ms NOT IN (SELECT sampled_unix_ms FROM operation_storage_samples ORDER BY sampled_unix_ms DESC LIMIT ?2)", params![at.saturating_sub(90 * DAY_MS),STORAGE_ROWS])?;
    tx.commit()?;
    Ok(())
}

pub fn tick(project: &Path, _: super::codex::Budget) -> Result<()> {
    sample_storage(project, jiff::Timestamp::now().as_millisecond())
}

struct Attempt {
    id: String, task: String, state: String, reserved: Option<i64>, launching: Option<i64>,
    running: Option<i64>, decided: Option<i64>, prompt: Option<i64>, cause: Option<String>,
}
fn attempts(db: &Connection, since: Option<i64>) -> Result<Vec<Attempt>> {
    Ok(db.prepare("SELECT a.id,a.task_id,a.state,
        (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=a.id AND state='reserved'),
        (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=a.id AND state='launching'),
        (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=a.id AND state='running'),
        (SELECT decided_unix_ms FROM dispatch_decisions WHERE attempt_id=a.id),
        (SELECT json_extract(payload,'$.prompt_chars') FROM operations WHERE kind='runtime.worker_brief' AND json_extract(payload,'$.attempt')=a.id ORDER BY rowid LIMIT 1),
        (SELECT json_extract(payload,'$.cause') FROM events WHERE kind='runtime.worker_terminated' AND entity=a.id ORDER BY sequence DESC LIMIT 1)
        FROM attempts a WHERE NOT EXISTS(SELECT 1 FROM replay_candidates r WHERE r.task_id=a.task_id)
        ORDER BY a.rowid")?.query_map([], |r| Ok(Attempt {id:r.get(0)?,task:r.get(1)?,state:r.get(2)?,reserved:r.get(3)?,launching:r.get(4)?,running:r.get(5)?,decided:r.get(6)?,prompt:r.get(7)?,cause:r.get(8)?}))?
        .collect::<rusqlite::Result<Vec<_>>>()?.into_iter().filter(|a| since.is_none_or(|lo| a.reserved.or(a.decided).is_some_and(|t| t >= lo))).collect())
}
fn latency(attempts: &[Attempt]) -> Value {
    let mut phases = BTreeMap::new();
    let mut rows = Vec::new();
    let mut causes = BTreeMap::<String, usize>::new();
    let mut adjudicated = 0;
    let mut never = 0;
    for a in attempts {
        if a.running.is_some() { adjudicated += 1; }
        else if a.reserved.is_some() && ["completed","failed","cancelled","lost"].contains(&a.state.as_str()) {
            adjudicated += 1; never += 1;
            *causes.entry(a.cause.clone().unwrap_or_else(|| "cause_not_recorded".into())).or_default() += 1;
        }
        let mut row = json!({"attempt_id":a.id,"reached_running":if a.running.is_some() {json!(true)} else if a.reserved.is_some() {json!(false)} else {Value::Null},"terminal_state":a.state,"cause":a.cause});
        for (phase, start, end) in [("decided_to_reserved",a.decided,a.reserved),("reserved_to_launching",a.reserved,a.launching),("launching_to_running",a.launching,a.running)] {
            let value = match (start,end) {
                (Some(start),Some(end)) if end >= start => json!(end-start),
                (Some(_),Some(_)) => unavailable("invalid_phase_order"),
                _ => unavailable(if a.reserved.is_none() {"predates_lifecycle_log"} else if start.is_none() || phase == "decided_to_reserved" {"phase_start_not_recorded"} else {"phase_end_not_recorded"}),
            };
            if let Some(ms) = value.as_i64() { phases.entry(phase).or_insert_with(Vec::new).push(ms); }
            row[phase] = value;
        }
        rows.push(row);
    }
    let phase_summary: BTreeMap<_,_> = ["decided_to_reserved","reserved_to_launching","launching_to_running"].into_iter()
        .map(|phase| (phase,summary(phases.remove(phase).unwrap_or_default()))).collect();
    json!({"value":if attempts.is_empty() {unavailable("no_attempts")} else if phase_summary.values().all(|v| v["count"] == 0) {unavailable("launch_phases_not_recorded")} else {json!(phase_summary)},"attempts":rows,
        "never_running":{"count":never,"denominator":adjudicated,"share":if adjudicated == 0 {Value::Null} else {json!(format!("{never}/{adjudicated}"))},"reason":(adjudicated == 0).then_some("empty_denominator"),"by_cause":causes.iter().map(|(cause,count)| (cause,json!({"count":count,"share":format!("{count}/{adjudicated}")}))).collect::<BTreeMap<_,_>>()},
        "pending":attempts.iter().filter(|a| a.running.is_none() && !["completed","failed","cancelled","lost"].contains(&a.state.as_str())).count()})
}
fn load(db: &Connection, sidecar: Option<&Connection>, attempts: &[Attempt], since: Option<i64>) -> Result<Value> {
    let samples = sidecar.filter(|s| table(s,"operation_launch_load").unwrap_or(false));
    let mut launches = Vec::new();
    let mut known = 0;
    for a in attempts {
        let sample = samples.map(|s| s.query_row("SELECT sampled_unix_ms,load_1m,load_5m,load_15m,reason FROM operation_launch_load WHERE attempt_id=?1", [&a.id], |r| Ok(json!({"sampled_unix_ms":r.get::<_,i64>(0)?,"load_1m":r.get::<_,Option<String>>(1)?,"load_5m":r.get::<_,Option<String>>(2)?,"load_15m":r.get::<_,Option<String>>(3)?,"reason":r.get::<_,Option<String>>(4)?}))).optional()).transpose()?.flatten();
        if sample.as_ref().is_some_and(|v| v["load_1m"].is_string()) { known += 1; }
        launches.push(json!({"attempt_id":a.id,"reached_running":if a.running.is_some() {json!(true)} else if a.reserved.is_some() {json!(false)} else {Value::Null},"outcome":if a.running.is_some() {"running"} else if a.reserved.is_none() {"unknown"} else if ["completed","failed","cancelled","lost"].contains(&a.state.as_str()) {"never_running"} else {"pending"},"load":sample.unwrap_or_else(|| unavailable("launch_load_not_recorded"))}));
    }
    let verifications: Vec<Value> = db.prepare("SELECT run_id,attempt_id,state,created_unix_ms,metadata FROM verification_runs WHERE (?1 IS NULL OR created_unix_ms>=?1) ORDER BY created_unix_ms,run_id")?
        .query_map([since], |r| {
            let metadata: Option<String> = r.get(4)?;
            let metadata = metadata.and_then(|s| serde_json::from_str::<Value>(&s).ok());
            Ok(json!({"run_id":r.get::<_,String>(0)?,"attempt_id":r.get::<_,String>(1)?,"verdict":r.get::<_,String>(2)?,"created_unix_ms":r.get::<_,i64>(3)?,"load":metadata.map(|v| v["load"].clone()).unwrap_or_else(|| unavailable("verification_load_not_recorded"))}))
        })?.collect::<rusqlite::Result<_>>()?;
    let value = if known == 0 && !verifications.iter().any(|v| v["load"]["host_load_1m"].is_string()) {unavailable("load_samples_not_recorded")} else {json!({"launch_samples":known,"verification_samples":verifications.iter().filter(|v| v["load"]["host_load_1m"].is_string()).count()})};
    Ok(json!({"value":value,"launches":launches,"verifications":verifications,"coverage":{"known":known,"expected":attempts.len(),"missing":attempts.len()-known}}))
}
fn storage(sidecar: Option<&Connection>, since: Option<i64>) -> Result<Value> {
    let Some(db) = sidecar.filter(|db| table(db,"operation_storage_samples").unwrap_or(false)) else {return Ok(json!({"value":unavailable("storage_not_sampled")}));};
    let rows: Vec<(i64,[Option<i64>;4])> = db.prepare("SELECT * FROM operation_storage_samples WHERE ?1 IS NULL OR sampled_unix_ms>=?1 ORDER BY sampled_unix_ms")?
        .query_map([since], |r| Ok((r.get(0)?,[r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?])))?.collect::<rusqlite::Result<_>>()?;
    let mut sizes = BTreeMap::new();
    for (i, name) in ["state_db","telemetry_db","worktrees","worker_output"].into_iter().enumerate() {
        let samples: Vec<_> = rows.iter().map(|r| json!({"sampled_unix_ms":r.0,"bytes":r.1[i],"reason":r.1[i].is_none().then_some("scan_incomplete")})).collect();
        let growth = match (rows.first(),rows.last()) {
            (Some(first),Some(last)) if first.0 < last.0 => match (first.1[i],last.1[i]) {
                (Some(a),Some(b)) => json!(format!("{}/{}",(i128::from(b)-i128::from(a))*i128::from(DAY_MS),last.0-first.0)),
                _ => unavailable("scan_incomplete"),
            },
            _ => unavailable("insufficient_storage_samples"),
        };
        sizes.insert(name,json!({"samples":samples,"growth_bytes_per_day":growth}));
    }
    Ok(json!({"value":if rows.is_empty() {unavailable("storage_not_sampled")} else {json!(sizes)},"samples":rows.len()}))
}
fn briefs(db: &Connection, attempts: &[Attempt]) -> Result<Value> {
    let accepted: BTreeSet<String> = super::metrics::task_evidence(db)?.into_iter().filter(|t| t.2).map(|t| t.0).collect();
    let mut buckets = BTreeMap::new();
    for name in ["small","medium","large"] {
        let selected: Vec<_> = attempts.iter().filter(|a| a.prompt.is_some_and(|p| match name {"small"=>p<4000,"medium"=>(4000..16000).contains(&p),_=>p>=16000})).collect();
        let tasks: BTreeSet<_> = selected.iter().map(|a| &a.task).collect();
        let successes = tasks.iter().filter(|task| accepted.contains(task.as_str())).count();
        buckets.insert(name,json!({"attempts":selected.len(),"tasks":tasks.len(),"accepted_tasks":successes,"accepted_share":if tasks.is_empty() {Value::Null} else {json!(format!("{successes}/{}",tasks.len()))},"attempts_per_task":if tasks.is_empty() {Value::Null} else {json!(format!("{}/{}",selected.len(),tasks.len()))},"reason":tasks.is_empty().then_some("empty_denominator")}));
    }
    let missing = attempts.iter().filter(|a| a.prompt.is_none()).count();
    Ok(json!({"value":if missing == attempts.len() {unavailable("brief_size_not_recorded")} else {json!(buckets)},"excluded":{"brief_size_not_recorded":missing},"coverage":{"known":attempts.len()-missing,"expected":attempts.len(),"missing":missing},"attempts":attempts.iter().filter_map(|a|a.prompt.map(|p|json!({"attempt_id":a.id,"task_id":a.task,"prompt_chars":p}))).collect::<Vec<_>>()}))
}

pub fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = super::read_only(&project.join(".state/state.db"))?;
    let sidecar = super::sidecar::read(project)?;
    let attempts = attempts(&db,since)?;
    let mut metrics: BTreeMap<String, Value> = BTreeMap::from([
        ("M60".into(),super::accounting::fleet::idle_gaps(project,since)?),
        ("M61".into(),latency(&attempts)),
        ("M62".into(),load(&db,sidecar.as_deref(),&attempts,since)?),
        ("M63".into(),storage(sidecar.as_deref(),since)?),
        ("M64".into(),briefs(&db,&attempts)?),
    ]);
    for (id, body) in &mut metrics {
        let metric = super::analytics::registry::find(id).expect("operations metric");
        body["name"] = json!(metric.name);
        body["definition"] = json!(metric.versions[0].definition);
    }
    Ok(metrics)
}
