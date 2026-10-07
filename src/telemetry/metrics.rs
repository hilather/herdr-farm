//! Contracts §6 metric subset: a read-only report over `state.db` and the
//! sidecar. Unknown is never 0: a ratio with an empty denominator is `null`
//! with `empty_denominator`; a value without a source is `unavailable`.
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const TERMINAL: [&str; 4] = ["completed", "failed", "cancelled", "lost"];
const NAMES: [(&str, &str); 10] = [("M02", "task_acceptance_rate"), ("M07", "attempt_amplification"), ("M08", "input_tokens"), ("M09", "output_tokens"),
    ("M13", "usage_coverage"), ("M15", "effective_model_coverage"), ("M31", "attention"), ("M32", "attention"), ("M33", "attention"), ("M40", "quota_headroom_at_dispatch")];

struct Attempt { id: String, task: String, state: String, kind: Option<String>, home: Option<String>, decided: Option<i64>, never_running: bool }

/// A reserved lifecycle with no running mark proves no session ran. Pre-log
/// attempts have no such proof and remain eligible for missing-usage coverage.
pub(crate) fn never_running_sql(db: &Connection) -> Result<&'static str> {
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='attempt_lifecycle')", [], |r| r.get(0))?;
    Ok(if present { "EXISTS(SELECT 1 FROM attempt_lifecycle l WHERE l.attempt_id=a.id AND l.state='reserved') AND NOT EXISTS(SELECT 1 FROM attempt_lifecycle l WHERE l.attempt_id=a.id AND l.state='running')" } else { "0" })
}

fn unavailable(reason: &str) -> Value {
    json!({"status": "unavailable", "reason": reason})
}

fn metric(id: &str, mut body: Value) -> Value {
    body["definition"] = json!(if id == "M13" { "M13.slice-v2".to_owned() } else { format!("{id}.slice-v1") });
    body
}

fn ratio(id: &str, numerator: usize, denominator: usize, extra: Value) -> Value {
    let mut body = json!({"numerator": numerator, "denominator": denominator});
    if denominator == 0 { body["value"] = Value::Null; body["reason"] = json!("empty_denominator"); } else { body["value"] = json!(format!("{numerator}/{denominator}")); }
    if let (Value::Object(body), Value::Object(extra)) = (&mut body, extra) { body.extend(extra); }
    metric(id, body)
}

/// Contracts §6 `T`/`A` evidence: every task `(id, state, accepted)`, where
/// accepted means every policy of one current-contract submission passed,
/// with integration evidence when required. Shared by this report and the attention M31 cohort.
pub(crate) fn task_evidence(db: &Connection) -> Result<Vec<(String, String, bool)>> {
    let mut tasks = db.prepare("SELECT t.id,t.state,EXISTS(SELECT 1 FROM task_contracts c JOIN result_submissions s ON s.task_id=c.task_id AND s.contract_revision=c.contract_revision
        JOIN verified_results r ON r.submission_id=s.submission_id WHERE c.task_id=t.id AND c.contract_revision=(SELECT max(contract_revision) FROM task_contracts WHERE task_id=t.id)
        AND (c.route='verify_only' OR EXISTS(SELECT 1 FROM integration_operations i JOIN integrated_commits k ON k.operation_id=i.operation_id WHERE i.verified_result_id=r.result_id)))
        FROM tasks t ORDER BY t.id")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<(String, String, bool)>>>()?;
    for (task, _, accepted) in &mut tasks {
        if !*accepted { continue; }
        let revision: i64 = db.query_row("SELECT max(contract_revision) FROM task_contracts WHERE task_id=?1", [task.as_str()], |r| r.get(0))?;
        let submissions = db.prepare("SELECT s.submission_id FROM result_submissions s JOIN task_contracts c ON c.task_id=s.task_id AND c.contract_revision=s.contract_revision
            WHERE s.task_id=?1 AND s.contract_revision=?2 AND (c.route='verify_only' OR EXISTS(SELECT 1 FROM verified_results r JOIN integration_operations i ON i.verified_result_id=r.result_id JOIN integrated_commits k ON k.operation_id=i.operation_id WHERE r.submission_id=s.submission_id))")?
            .query_map(rusqlite::params![task.as_str(), revision], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        *accepted = false;
        for submission in submissions {
            if super::analytics::lifecycle::candidate_verdict(db, task, revision, &submission)?.1 == "accepted" { *accepted = true; break; }
        }
    }
    Ok(tasks)
}

/// `herdr-farm telemetry <slug> report`. `since` bounds the activity window (Unix ms).
/// Assembled by the query service (`super::analytics::query::report`), the one
/// read path shared with `telemetry query`, the fleet pane and exports.
pub fn report(project: &Path, since: Option<i64>) -> Result<Value> { super::analytics::query::report(project, since) }

/// The central slice metrics (contracts §6) and the `tasks` summary, before
/// the lane providers (`super::LANES`) add theirs.
pub(crate) fn central(project: &Path, since: Option<i64>) -> Result<(BTreeMap<String, Value>, Value)> {
    if let Some(db) = super::sidecar::read(project)? && let Some(generations) = super::analytics::inputs::generations(&db)? {
        let stamp = super::analytics::inputs::stamp("central", &super::analytics::inputs::canonical(project)?, &generations);
        if let Some(body) = super::analytics::inputs::cached(&db, "central", since, &stamp)? {
            let mut metrics: BTreeMap<String, Value> = match body {
                Value::Object(values) => values.into_iter().collect(),
                body => serde_json::from_value(body)?,
            };
            if let Some(tasks) = metrics.remove("_tasks") {
                if !metrics.contains_key("M30") {
                    let state = super::read_only(&project.join(".state/state.db"))?;
                    let evidence = task_evidence(&state)?;
                    let replay = replay_tasks(&state)?;
                    metrics.insert("M30".to_owned(), super::analytics::lifecycle::first_candidate_report(&state, &evidence, &replay, since)?);
                }
                // Operating facts change every ticker pass. Read M03 separately
                // so they neither stale this body nor invalidate M13/M40 caches.
                metrics.insert("M03".to_owned(),super::operating::evaluate(project,since,None)?);
                return Ok((metrics, tasks));
            }
        }
    }
    central_uncached(project, since, true, true)
}

pub(crate) fn central_uncached(project: &Path, since: Option<i64>, aggregates: bool, first_candidates: bool) -> Result<(BTreeMap<String, Value>, Value)> {
    let path = project.join(".state/state.db");
    let db = super::read_only(&path)?;
    let table = |name: &str| db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |r| r.get::<_, bool>(0));
    let decided = if table("dispatch_decisions")? { "(SELECT decided_unix_ms FROM dispatch_decisions d WHERE d.attempt_id=a.id)" } else { "NULL" };
    let never = never_running_sql(&db)?;
    let attempts: Vec<Attempt> = db.prepare(&format!("SELECT a.id,a.task_id,a.state,json_extract(i.payload,'$.inputs.effective_profile.kind'),
        json_extract(i.payload,'$.inputs.effective_profile.execution_home'),{decided},{never} FROM attempts a LEFT JOIN attempt_inputs i ON i.attempt_id=a.id ORDER BY a.rowid"))?
        .query_map([], |r| Ok(Attempt { id: r.get(0)?, task: r.get(1)?, state: r.get(2)?, kind: r.get(3)?, home: r.get(4)?, decided: r.get(5)?, never_running: r.get(6)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let in_window = |a: &Attempt| since.is_none_or(|since| a.decided.is_some_and(|at| at >= since));

    // Contracts §6 `T` and `A`: evidence for the task's current contract revision.
    // Replay candidates are evaluation artefacts (M49 only), never in `T`.
    let tasks = task_evidence(&db)?;
    let replay = replay_tasks(&db)?;
    let (mut terminal, mut accepted, mut open, mut without_evidence, mut outside, mut replayed) = (BTreeSet::new(), 0, 0, 0, 0, 0);
    for (task, state, evidence) in &tasks {
        if replay.contains(task) { replayed += 1; continue; }
        if since.is_some() && !attempts.iter().any(|a| &a.task == task && in_window(a)) { outside += 1; continue; }
        if *evidence || ["succeeded", "failed", "cancelled"].contains(&state.as_str()) {
            terminal.insert(task.as_str());
            if *evidence { accepted += 1; } else if state == "succeeded" { without_evidence += 1; }
        } else { open += 1; }
    }
    let cohort: Vec<&Attempt> = attempts.iter().filter(|a| terminal.contains(a.task.as_str())).collect();
    let mut metrics = BTreeMap::new();
    metrics.insert("M03", super::operating::evaluate_evidence(project,&tasks,&replay,since,None)?);
    if first_candidates { metrics.insert("M30", super::analytics::lifecycle::first_candidate_report(&db, &tasks, &replay, since)?); }
    metrics.insert("M02", ratio("M02", accepted, terminal.len(), json!({"excluded": {"open": open, "outside_window": outside, "replay_candidate": replayed}})));
    metrics.insert("M07", ratio("M07", cohort.len(), accepted, json!({"attempts_without_decision": cohort.iter().filter(|a| a.decided.is_none()).count(),
        "excluded": {"replay_candidate": replayed}})));

    let sidecar = super::sidecar::read(project)?;
    let _snapshot = sidecar.as_deref().filter(|db| db.is_autocommit()).map(|db| db.unchecked_transaction()).transpose()?;
    usage_metrics(sidecar.as_deref(), &attempts, since, &in_window, &mut metrics, aggregates)?;
    for id in ["M31", "M32", "M33"] { metrics.insert(id, metric(id, json!({"value": unavailable("attention_not_collected")}))); }
    metrics.insert("M40", headroom(project, sidecar.as_deref(), &attempts, &in_window, aggregates)?);
    #[cfg(target_os = "linux")]
    metrics.insert("M49", crate::replay::m49(&db, since)?);
    for (id, name) in NAMES { if let Some(m) = metrics.get_mut(id) { m["name"] = json!(name); } }
    let metrics: BTreeMap<String, Value> = metrics.into_iter().map(|(id, m)| (id.to_owned(), m)).collect();
    Ok((metrics, json!({"accepted": accepted, "open": open, "succeeded_without_evidence": without_evidence, "terminal": terminal.len(), "replay_candidates": replayed})))
}

fn replay_tasks(db: &Connection) -> Result<BTreeSet<String>> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='replay_candidates')", [], |r| r.get(0))?;
    Ok(if exists { db.prepare("SELECT task_id FROM replay_candidates")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()? } else { BTreeSet::new() })
}

/// M08, M09, M15 over certified bound sessions (activity window by session
/// start) and M13 over terminated attempts decided in the window.
fn usage_metrics(sidecar: Option<&Connection>, attempts: &[Attempt], since: Option<i64>, in_window: &dyn Fn(&Attempt) -> bool, metrics: &mut BTreeMap<&str, Value>, aggregates: bool) -> Result<()> {
    let terminated: Vec<&Attempt> = attempts.iter().filter(|a| TERMINAL.contains(&a.state.as_str()) && in_window(a)).collect();
    let grok_bound: BTreeSet<String> = if let Some(db) = sidecar {
        db.prepare("SELECT DISTINCT attempt_id FROM rollout_sources WHERE originator IN ('otlp:grok','otlp:devin') AND binding='bound'")?
            .query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    } else { BTreeSet::new() };
    let adapter_absent = terminated.iter().filter(|a| a.kind.as_deref().is_some_and(|k| !matches!(k, "codex" | "claude" | "opencode" | "muse")) && !grok_bound.contains(&a.id)).count();
    let codex: Vec<&&Attempt> = terminated.iter().filter(|a| matches!(a.kind.as_deref(), Some("codex" | "claude" | "opencode" | "muse")) || grok_bound.contains(&a.id)).collect();
    let Some(db) = sidecar else {
        for id in ["M08", "M09", "M15"] { metrics.insert(id, metric(id, json!({"value": unavailable("no_certified_source")}))); }
        metrics.insert("M13", metric("M13", json!({"value": unavailable("collection_not_run"), "adapter_absent": adapter_absent})));
        return Ok(());
    };
    // (session, binding, attempt, certified, quarantined, records, accepted records, session start)
    type Source = (String, String, Option<String>, bool, bool, i64, i64, Option<i64>, bool);
    // Records collected before their version was certified keep NULL counters:
    // such a source stays uncertified (see `sidecar::attempt_usage`).
    let sql = if aggregates && super::accounting::ledger::aggregates_current(db)? {
        "SELECT s.session_id,s.binding,s.attempt_id,CASE WHEN a.uncertified THEN '' ELSE s.cli_version END,
        a.quarantined,s.records,a.accepted_records,s.session_unix_ms FROM rollout_sources s
        JOIN accounting_source_summary a USING(path_digest) ORDER BY s.path_digest"
    } else {
        "SELECT s.session_id,s.binding,s.attempt_id,
        CASE WHEN EXISTS(SELECT 1 FROM codex_usage u WHERE u.path_digest=s.path_digest AND u.reason='cli_version_uncertified') THEN '' ELSE s.cli_version END,EXISTS(SELECT 1 FROM codex_quarantine q WHERE q.session_id=s.session_id),
        s.records,(SELECT count(*) FROM codex_usage u WHERE u.path_digest=s.path_digest AND u.accepted=1),s.session_unix_ms FROM rollout_sources s ORDER BY s.path_digest"
    };
    let sources: Vec<Source> = db.prepare(sql)?
        .query_map([], |r| {
            let session: String = r.get(0)?;
            let version: String = r.get(3)?;
            let quarantined = r.get::<_, bool>(4)? || super::sidecar::malformed_newer(db, &session, &version)?;
            let incomplete = super::sidecar::incomplete_newer(db, &session, &version)?;
            Ok((session, r.get(1)?, r.get(2)?, super::codex::accepted_version(&version), quarantined, r.get(5)?, r.get(6)?, r.get(7)?, incomplete))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let known: BTreeSet<&str> = attempts.iter().map(|a| a.id.as_str()).collect();
    let losing = if sources.iter().any(|s| s.0.starts_with("otlp:")) { super::accounting::otlp::losing_sessions(db)? } else { BTreeSet::new() };
    let version_rows: Vec<(String, String)> = db.prepare("SELECT DISTINCT session_id,cli_version FROM rollout_sources")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let newer_versions: BTreeSet<String> = version_rows.iter().filter(|(_, v)| super::version::nearest(v).is_some()).map(|(s, _)| s.clone()).collect();
    let incomplete_sessions: BTreeSet<&str> = sources.iter().filter(|s| s.8).map(|s| s.0.as_str()).collect();
    let (mut certified, mut excluded) = (BTreeSet::new(), BTreeMap::<&str, usize>::new());
    for s in sources.iter().filter(|s| since.is_none_or(|since| s.7.is_some_and(|at| at >= since))) {
        let reason = if s.1 != "bound" { s.1.as_str() }
            else if !s.2.as_deref().is_some_and(|a| known.contains(a)) { "orphan" }
            else if losing.contains(&s.0) { "native_surface_precedence" }
            else if s.4 { "quarantined" }
            else if !s.3 { "cli_version_uncertified" }
            else if incomplete_sessions.contains(s.0.as_str()) { "records_not_accepted" }
            else { certified.insert(s.0.as_str()); continue; };
        *excluded.entry(reason).or_default() += 1;
    }
    let newer_sessions: BTreeSet<String> = newer_versions.iter().filter(|s| certified.contains(s.as_str())).cloned().collect();
    let mut coverage = json!({"certified_sessions": certified.len(), "excluded": excluded});
    let newer_provenance: BTreeMap<&str, Value> = version_rows.iter().filter(|(s, _)| newer_sessions.contains(s))
        .map(|(_, v)| (v.as_str(), super::version::provenance(v))).collect();
    if !newer_sessions.is_empty() {
        coverage["newer_than_certified"] = json!(newer_sessions.len());
        coverage["versions"] = json!(newer_provenance);
    }
    if certified.is_empty() {
        for id in ["M08", "M09", "M15"] { metrics.insert(id, metric(id, json!({"value": unavailable("no_certified_source"), "coverage": coverage}))); }
    } else {
        let mut sums = [0i64; 5];
        let sql = if aggregates && super::accounting::ledger::aggregates_current(db)? {
            "SELECT input_tokens,output_tokens,reasoning_tokens,records,models FROM accounting_native_totals WHERE session_id=?1"
        } else {
            "SELECT coalesce(sum(input_tokens),0),coalesce(sum(output_tokens),0),coalesce(sum(reasoning_output_tokens),0),count(*),count(model)
                FROM codex_usage WHERE session_id=?1 AND accepted=1 AND NOT EXISTS(SELECT 1 FROM codex_usage e WHERE e.session_id=codex_usage.session_id AND e.accepted=1 AND e.response_id IS NOT NULL AND e.response_id=codex_usage.response_id AND unhex(substr(e.payload_digest,8)) IS unhex(substr(codex_usage.payload_digest,8)) AND e.payload_digest=codex_usage.payload_digest AND e.ordinal<codex_usage.ordinal)"
        };
        let mut totals = db.prepare(sql)?;
        for session in &certified {
            let row: [i64; 5] = totals.query_row([session], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]))?;
            for (sum, value) in sums.iter_mut().zip(row) { *sum += value; }
        }
        metrics.insert("M08", metric("M08", json!({"value": sums[0], "coverage": coverage})));
        let reasoning = if certified.iter().map(|s| crate::telemetry::sidecar::reasoning_missing(db, s)).collect::<rusqlite::Result<Vec<_>>>()?.into_iter().any(|missing| missing) { unavailable("reasoning_tokens_not_reported") } else { json!(sums[2]) };
        metrics.insert("M09", metric("M09", json!({"value": sums[1], "reasoning_output_tokens": reasoning, "coverage": coverage})));
        metrics.insert("M15", ratio("M15", sums[4] as usize, sums[3] as usize, json!({"coverage": coverage})));
    }
    // Group once: searching all rollouts separately for each retained attempt
    // makes coverage quadratic in the binding count.
    let mut by_attempt = BTreeMap::<&str, Vec<&Source>>::new();
    for source in &sources {
        if source.1 == "bound" && let Some(attempt) = source.2.as_deref() { by_attempt.entry(attempt).or_default().push(source); }
    }
    let mut incomplete = BTreeMap::<&str, usize>::new();
    let mut excluded = 0;
    for a in &codex {
        let bound = by_attempt.get(a.id.as_str()).map(Vec::as_slice).unwrap_or_default();
        if bound.is_empty() && a.never_running { excluded += 1; continue; }
        let reason = if bound.is_empty() { "not_bound" } else if bound.iter().any(|s| s.4) { "quarantined" }
            else if bound.iter().any(|s| !s.3) { "cli_version_uncertified" } else if bound.iter().any(|s| s.5 != s.6 || s.8) { "records_not_accepted" } else { continue };
        *incomplete.entry(reason).or_default() += 1;
    }
    let complete = codex.len() - excluded - incomplete.values().sum::<usize>();
    let mut detail = json!({"adapter_absent": adapter_absent, "incomplete": incomplete, "excluded": {"never_running": excluded}});
    let newer_attempts = codex.iter().filter(|a| by_attempt.get(a.id.as_str()).is_some_and(|sources|
        sources.iter().any(|s| newer_sessions.contains(&s.0)) && sources.iter().all(|s| s.3 && !s.4 && s.5 == s.6 && !s.8))).count();
    if newer_attempts > 0 { detail["coverage"] = json!({"newer_than_certified": newer_attempts, "versions":newer_provenance}); }
    metrics.insert("M13", ratio("M13", complete, codex.len() - excluded, detail));
    Ok(())
}

/// Extended M40 (contracts-accounting §5, `M40.quota-windows-v2`): per
/// decision in the window and per limit window, the latest trusted remaining
/// value from the quota tables the last `accounting sync` built, never summed
/// across accounts, limits or services. Each entry equals the one `accounting
/// quota` prints; before a sync every Codex decision is `ledger_not_synced`.
fn headroom(project: &Path, sidecar: Option<&Connection>, attempts: &[Attempt], in_window: &dyn Fn(&Attempt) -> bool, aggregates: bool) -> Result<Value> {
    use super::accounting::quota;
    let synced = match sidecar { Some(db) => quota::synced(db)?, None => false };
    let aggregate = match sidecar { Some(db) => aggregates && quota::dispatch_current(project, db)?, None => false };
    let mut decisions = Vec::new();
    for a in attempts.iter().filter(|a| in_window(a)) {
        let Some(decided) = a.decided else { continue };
        let mut entry = match (a.kind.as_deref(), &a.home, sidecar) {
            (Some(kind), ..) if kind != "codex" => json!({"value": unavailable("adapter_absent")}),
            (_, None, _) => json!({"value": unavailable("execution_home_unknown")}),
            (.., None) => json!({"value": unavailable("collection_not_run")}),
            _ if !synced => json!({"value": unavailable("ledger_not_synced")}),
            (_, Some(home), Some(db)) => match if aggregate { quota::stored_headroom(db, &a.id, home, decided)? } else { None } {
                Some(body) => body,
                None => quota::headroom(db, home, decided)?,
            },
        };
        entry["attempt_id"] = json!(a.id);
        entry["decided_unix_ms"] = json!(decided);
        entry["service"] = json!("codex");
        decisions.push(entry);
    }
    let mut m40 = metric("M40", json!({"decisions": decisions, "stale_after_ms": quota::STALE_AFTER_MS}));
    m40["definition"] = json!("M40.quota-windows-v2");
    if synced && let Some(db) = sidecar { m40["not_reported"] = json!(quota::unreported(db)?); }
    Ok(m40)
}

/// A structured metric value as one line of text, or `None` when it is not
/// one (an unavailable value carries a `reason`). M16 (`M16.tools-v1`):
/// `issued N, accepted K <status> (U unknown), executed E`, as `accounting
/// tools` prints it, never `n/a` while the counts are known.
pub fn structured_text(value: &Value) -> Option<String> {
    let o = value.as_object()?;
    if value["status"] == "partial" && o.contains_key("denominator") {
        let subtotal = value["priced_amount"].as_str().map(str::to_owned).unwrap_or_else(|| value["tokens"].to_string());
        let reason = value["reason"].as_str().unwrap_or("unknown");
        let missing = value.get("attempts_without_usage").map(|n| format!(": {n} attempts without usage")).unwrap_or_default();
        return Some(format!("partial {subtotal}/{} ({reason}{missing})", value["denominator"]));
    }
    if o.contains_key("reason") { return None; }
    if o.contains_key("state_db") {
        let sizes = ["state_db", "telemetry_db", "worktrees", "worker_output"].map(|name| {
            let category = &value[name];
            let last = category["samples"].as_array().and_then(|rows| rows.last());
            let bytes = last.and_then(|row| row["bytes"].as_i64()).map_or_else(|| "n/a (scan_incomplete)".to_owned(), |bytes| bytes.to_string());
            let growth = &category["growth_bytes_per_day"];
            let growth = growth.as_str().map(str::to_owned).unwrap_or_else(|| format!("n/a ({})", growth["reason"].as_str().unwrap_or("unknown")));
            format!("{name} bytes={bytes} growth_bytes_per_day={growth}")
        });
        return Some(sizes.join("; "));
    }
    if o.contains_key("candidates_proposed") {
        let observed = |v: &Value| v.as_i64().map_or_else(||format!("n/a ({})",v["reason"].as_str().unwrap_or("unknown")),|n|n.to_string());
        return Some(format!("proposed={} accepted={} rejected={} facts={} omitted={} remember={}",
            value["candidates_proposed"],value["candidates_accepted"],value["candidates_rejected"],
            observed(&value["briefs"]["facts_delivered"]),observed(&value["briefs"]["omitted_for_budget"]),observed(&value["remember_sections_written"])));
    }
    if o.contains_key("median") || o.contains_key("median_ms") || o.contains_key("reserved_to_launching") || o.contains_key("launch_samples") || o.contains_key("small") { return Some(value.to_string()); }
    let issued = o.get("issued")?.as_u64()?;
    let accepted = &value["accepted"];
    Some(format!("issued {issued}, accepted {} {} ({} unknown), executed {}", accepted["count"], accepted["status"].as_str().unwrap_or("unknown"),
        accepted["unknown"], value["executed"]))
}

/// One line per metric (M40: one per decision and limit window); anything unknown reads `n/a`, never 0.
pub fn text(report: &Value) -> String {
    let show = |m: &Value| match &m["value"] {
        Value::Null => format!("n/a ({})", m["reason"].as_str().unwrap_or("unknown")),
        Value::Object(o) => structured_text(&m["value"]).unwrap_or_else(|| o.get("reason").and_then(Value::as_str).map_or_else(|| serde_json::to_string(o).unwrap_or_default(), |reason| format!("n/a ({reason})"))),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = String::new();
    // Every metric in registry order; a provider's own name wins.
    let Some(metrics) = report["metrics"].as_object() else { return out };
    for registered in super::analytics::registry::METRICS {
        let id = registered.id;
        let Some(m) = metrics.get(id) else { continue };
        let name = m["name"].as_str().unwrap_or(registered.name);
        let show_metric = |cell: &Value| {
            let mut text = show(cell);
            if matches!(id, "M70" | "M71" | "M72" | "M73" | "M74" | "M75" | "M76" | "M77") {
                if cell["value"].get("samples").is_some() { text = cell["value"].to_string(); }
                for key in ["over_10_min", "commands", "failed_commands", "exit_unknown", "unanswered", "missing_samples", "over_80_percent", "by_class", "by_caller", "top_command_paths"] {
                    if let Some(value) = cell.get(key) { text += &format!(" {key}={value}"); }
                }
                if cell["status"] == "partial" { text += &format!(" partial:{}", cell["reason"].as_str().unwrap_or("unknown")); }
            }
            text
        };
        for w in m["not_reported"].as_array().into_iter().flatten() {
            out += &format!("limit {} {} {} n/a ({})\n", w["account"].as_str().unwrap_or(""), w["limit_id"].as_str().unwrap_or(""), w["window_kind"].as_str().unwrap_or(""), w["reason"].as_str().unwrap_or("not_reported"));
        }
        for (dimension, cells) in [("profile", &m["by_profile"]), ("currency", &m["by_currency"])] {
            for (label, cell) in cells.as_object().into_iter().flatten() {
                if id == "M56" {
                    for component in ["reasoning_share", "cache_share"] {
                        out += &format!("{id} {name} {dimension}={label} {component} {}\n", show(&cell[component]));
                    }
                } else if id == "M66" && dimension == "profile" && cell["by_currency"].as_object().is_some_and(|currencies| !currencies.is_empty()) {
                    for (currency, amount) in cell["by_currency"].as_object().into_iter().flatten() {
                        out += &format!("{id} {name} profile={label} currency={currency} {}\n", show(amount));
                    }
                } else {
                    out += &format!("{id} {name} {dimension}={label} {}\n", show_metric(cell));
                    for (severity, value) in cell["by_severity"].as_object().into_iter().flatten() {
                        out += &format!("{id} {name} {dimension}={label} severity={severity} {}\n", show(value));
                    }
                }
            }
        }
        match m["decisions"].as_array() {
            Some(list) if list.is_empty() => out += &format!("{id} {name} n/a (no_decisions)\n"),
            // M40: one line per decision and limit window, or per decision without windows.
            Some(list) => for d in list {
                let attempt = d["attempt_id"].as_str().unwrap_or("");
                let Some(windows) = d["windows"].as_array() else { out += &format!("{id} {name} {attempt} {}\n", show(d)); continue };
                for w in windows {
                    let age = w["age_ms"].as_i64().map(|age| format!(" age_ms={age}")).unwrap_or_default();
                    let value = match w["value"].as_str() {
                        Some(remaining) => format!("remaining {remaining}%{age} {}", w["freshness"].as_str().unwrap_or("")),
                        None => format!("{}{age}", show(w)),
                    };
                    out += &format!("{id} {name} {attempt} {} {} {value}\n", w["limit_id"].as_str().unwrap_or(""), w["window_kind"].as_str().unwrap_or(""));
                }
            },
            None if id == "M67" && m.get("count").is_some() => out += &format!("{id} {name} count={} {}\n", m["count"], show_metric(m)),
            None => out += &format!("{id} {name} {}\n", show_metric(m)),
        }
    }
    for a in report["after_termination"].as_array().into_iter().flatten() {
        out += &format!("attempt {} after_termination records={} first_unix_ms={} terminated_unix_ms={}; still counted in M08\n",
            a["attempt_id"].as_str().unwrap_or("-"), a["after_termination"]["records"], a["after_termination"]["first_unix_ms"], a["after_termination"]["terminated_unix_ms"]);
    }
    let t = &report["tasks"];
    out + &format!("tasks terminal={} accepted={} open={} succeeded_without_evidence={}\n", t["terminal"], t["accepted"], t["open"], t["succeeded_without_evidence"])
}
