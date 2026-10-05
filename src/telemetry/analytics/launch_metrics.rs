//! MET-LAUNCH-1: descriptive metadata, never admission or acceptance inputs.
use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn absent(reason: &str) -> Value {
    json!({"status":"unavailable","reason":reason})
}
fn table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1 AND type='table')",
        [name],
        |r| r.get(0),
    )?)
}
fn ratio(n: i64, d: i64) -> Value {
    if d == 0 {
        absent("empty_denominator")
    } else {
        json!(format!("{n}/{d}"))
    }
}
struct Invocation {
    path: String,
    at: i64,
    task: Option<String>,
    attempt: Option<String>,
    force: bool,
}
fn invocations(side: Option<&Connection>, since: Option<i64>) -> Result<Option<Vec<Invocation>>> {
    let Some(db) = side else {
        return Ok(None);
    };
    if !table(db, "operation_cli_targets")? {
        return Ok(None);
    }
    let rows = db.prepare("SELECT c.invocation_id,c.command_path,c.recorded_unix_ms,t.task_id,t.attempt_id,coalesce(t.force,0) FROM cli_invocations c LEFT JOIN operation_cli_targets t USING(invocation_id) WHERE c.caller IN ('operator','coordinator') AND c.outcome NOT IN ('help','version','usage_error') AND c.command_path IN ('launch run','launch stop','task cancel-attempt') AND c.recorded_unix_ms>=?1 ORDER BY c.recorded_unix_ms,c.invocation_id")?
        .query_map([since.unwrap_or(0).max(crate::telemetry::maintenance::store::now()-crate::telemetry::accounting::cli_invocations::MAX_AGE_MS)], |r| Ok(Invocation {path:r.get(1)?,at:r.get(2)?,task:r.get(3)?,attempt:r.get(4)?,force:r.get(5)?}))?.collect::<rusqlite::Result<_>>()?;
    Ok(Some(rows))
}
fn launches(db: &Connection, rows: Option<&[Invocation]>) -> Result<Value> {
    let Some(rows) = rows else {
        return Ok(json!({"value":absent("launch_invocations_not_collected")}));
    };
    let launches: Vec<_> = rows.iter().filter(|r| r.path == "launch run").collect();
    let running = db.prepare("SELECT a.task_id,l.unix_ms,(SELECT max(unix_ms) FROM attempt_lifecycle terminal WHERE terminal.attempt_id=a.id AND terminal.state IN ('completed','failed','cancelled','lost')) FROM attempts a JOIN attempt_lifecycle l ON l.attempt_id=a.id WHERE l.state='running' AND NOT EXISTS(SELECT 1 FROM replay_candidates r WHERE r.task_id=a.task_id)")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<i64>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reached = 0;
    let mut by_task = BTreeMap::<String, Value>::new();
    let mut missing = 0;
    let mut first_running = BTreeMap::<&str, i64>::new();
    for (task, at, _) in &running {
        first_running
            .entry(task)
            .and_modify(|old| *old = (*old).min(*at))
            .or_insert(*at);
    }
    for (index, row) in launches.iter().enumerate() {
        let Some(task) = row.task.as_deref() else {
            missing += 1;
            continue;
        };
        let next = launches[index + 1..]
            .iter()
            .find(|r| r.task.as_deref() == Some(task))
            .map(|r| r.at);
        let success = running.iter().any(|(t, at, end)| {
            t == task
                && ((*at >= row.at && next.is_none_or(|n| *at < n))
                    || (*at < row.at && end.is_none_or(|end| end >= row.at)))
        });
        reached += i64::from(success);
        let item = by_task
            .entry(task.into())
            .or_insert(json!({"launches":0,"tries_before_running":0,"reached_running":false}));
        item["launches"] = json!(item["launches"].as_i64().unwrap_or(0) + 1);
        if first_running.get(task).is_none_or(|at| row.at <= *at) {
            item["tries_before_running"] =
                json!(item["tries_before_running"].as_i64().unwrap_or(0) + 1);
        }
        item["reached_running"] = json!(first_running.contains_key(task));
    }
    let d = launches.len() as i64;
    let mut out = json!({"value":ratio(reached,d),"numerator":reached,"denominator":d,"by_task":by_task,"unattributed_invocations":missing});
    if missing > 0 {
        out["status"] = json!("partial");
        out["reason"] = json!("launch_target_not_recorded");
        if missing == launches.len() {
            out["value"] = absent("launch_target_not_recorded");
        }
    }
    Ok(out)
}
fn interventions(db: &Connection, rows: Option<&[Invocation]>) -> Result<Value> {
    let Some(rows) = rows else {
        return Ok(json!({"value":absent("launch_invocations_not_collected")}));
    };
    let mut days = BTreeMap::<i64, i64>::new();
    let mut attempts = BTreeMap::<String, i64>::new();
    let mut missing = 0;
    let unknown_force = rows
        .iter()
        .filter(|r| r.path == "launch stop" && r.task.is_none() && r.attempt.is_none())
        .count();
    for row in rows
        .iter()
        .filter(|r| r.path == "task cancel-attempt" || (r.path == "launch stop" && r.force))
    {
        *days.entry(row.at.div_euclid(86_400_000)).or_default() += 1;
        let attempt = if let Some(attempt) = &row.attempt {
            Some(attempt.clone())
        } else if let Some(task) = &row.task {
            use rusqlite::OptionalExtension;
            db.query_row("SELECT a.id FROM attempts a JOIN attempt_lifecycle l ON l.attempt_id=a.id AND l.state='reserved' WHERE a.task_id=?1 AND l.unix_ms<=?2 ORDER BY l.unix_ms DESC,a.id LIMIT 1", params![task,row.at], |r|r.get::<_,String>(0)).optional()?
        } else {
            None
        };
        if let Some(id) = attempt {
            *attempts.entry(id).or_default() += 1;
        } else {
            missing += 1;
        }
    }
    let mut out = json!({"value":days.values().sum::<i64>(),"per_day_utc":days,"per_attempt":attempts,"unattributed_invocations":missing,"force_flag_unknown":unknown_force});
    if missing > 0 || unknown_force > 0 {
        out["status"] = json!("partial");
        out["reason"] = json!("intervention_attempt_unknown");
    }
    Ok(out)
}
fn ticker(side: Option<&Connection>, since: Option<i64>) -> Result<Value> {
    let Some(db) = side else {
        return Ok(json!({"value":absent("ticker_errors_not_observed")}));
    };
    if !table(db, "operation_ticker_errors")? {
        return Ok(json!({"value":absent("ticker_errors_not_observed")}));
    }
    let rows=db.prepare("SELECT sampled_unix_ms,lock_contention,expired_inventory,ambiguous_outcome,permanent_failure,other,session,pass FROM operation_ticker_errors WHERE sampled_unix_ms>=?1 ORDER BY sampled_unix_ms,session,pass")?.query_map([since.unwrap_or(0)],|r|Ok((r.get::<_,i64>(0)?,[r.get::<_,i64>(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?],r.get::<_,String>(6)?,r.get::<_,i64>(7)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(json!({"value":absent("ticker_errors_not_observed")}));
    }
    let classes = [
        "lock_contention",
        "expired_inventory",
        "ambiguous_outcome",
        "permanent_failure",
        "other",
    ];
    let mut hours = BTreeMap::<i64, BTreeMap<&str, i64>>::new();
    let mut previous = BTreeMap::<&str, i64>::new();
    let mut missing = 0;
    for (at, counts, session, pass) in &rows {
        let prior = previous.insert(session, *pass);
        if let Some(prior) = prior {
            missing += (*pass - prior - 1).max(0);
        } else if since.is_none() {
            missing += (*pass - 1).max(0);
        }
        let hour = hours
            .entry(at.div_euclid(3_600_000))
            .or_insert_with(|| classes.into_iter().map(|c| (c, 0)).collect());
        for (c, n) in classes.iter().zip(counts) {
            *hour.get_mut(c).unwrap() += n;
        }
    }
    let mut body = json!({"value":hours,"observed_passes":rows.len(),"missing_passes":missing,"scope":"ticker_root","partial_hours":true,"rate_basis":"counts_per_utc_hour_bucket","coverage":{"first_pass_unix_ms":rows.first().map(|r|r.0),"last_pass_unix_ms":rows.last().map(|r|r.0)}});
    if missing > 0 {
        body["status"] = json!("partial");
        body["reason"] = json!("ticker_passes_not_observed");
    }
    Ok(body)
}
fn union(intervals: &mut [(i64, i64)]) -> i64 {
    intervals.sort_unstable();
    let mut end = i64::MIN;
    let mut total = 0;
    for &(a, b) in intervals.iter() {
        if b > a {
            total += b - a.max(end).min(b);
            end = end.max(b);
        }
    }
    total
}
fn time_breakdown(project: &Path, since: Option<i64>) -> Result<Value> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    if !table(&db, "task_lineage")? {
        return Ok(json!({"value":absent("lineage_not_recorded")}));
    }
    let rows = db.prepare("SELECT t.work_item,a.id,a.state,
        (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=a.id AND state='reserved'),
        (SELECT unix_ms FROM attempt_lifecycle WHERE attempt_id=a.id AND state='running'),
        (SELECT max(unix_ms) FROM attempt_lifecycle WHERE attempt_id=a.id AND state IN ('completed','failed','cancelled','lost'))
        FROM task_lineage t LEFT JOIN attempts a ON a.task_id=t.task_id
        WHERE NOT EXISTS(SELECT 1 FROM replay_candidates r WHERE r.task_id=t.task_id) ORDER BY t.work_item,a.id")?
        .query_map([],|r|Ok(json!({"lineage":{"work_item":r.get::<_,String>(0)?},"attempt_id":r.get::<_,Option<String>>(1)?,"terminal_state":r.get::<_,Option<String>>(2)?,"reserved_unix_ms":r.get::<_,Option<i64>>(3)?,"running_unix_ms":r.get::<_,Option<i64>>(4)?,"terminal_unix_ms":r.get::<_,Option<i64>>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let report = json!({"attempts":rows});
    let mut groups = BTreeMap::<&str, Vec<&Value>>::new();
    let unbound: i64 = db.query_row(
        "SELECT count(*) FROM tasks WHERE id NOT IN (SELECT task_id FROM task_lineage)",
        [],
        |r| r.get(0),
    )?;
    for a in report["attempts"].as_array().into_iter().flatten() {
        if let Some(work) = a["lineage"]["work_item"].as_str() {
            groups.entry(work).or_default().push(a);
        }
    }
    let mut items = Vec::new();
    for (work, attempts) in groups {
        let first = attempts
            .iter()
            .filter_map(|a| a["reserved_unix_ms"].as_i64())
            .min();
        if since.is_some_and(|s| first.is_none_or(|at| at < s)) {
            continue;
        }
        let mut active = Vec::new();
        let mut overhead = Vec::new();
        let mut spans = Vec::new();
        let mut unknown = 0;
        let mut censored = 0;
        let mut never_running = 0;
        for a in &attempts {
            let (start, end) = (
                a["reserved_unix_ms"].as_i64(),
                a["terminal_unix_ms"].as_i64(),
            );
            if !matches!(
                a["terminal_state"].as_str(),
                Some("completed" | "failed" | "cancelled" | "lost")
            ) {
                censored += 1;
            }
            let Some((start, end)) = start.zip(end).filter(|(s, e)| e >= s) else {
                unknown += 1;
                continue;
            };
            spans.push((start, end));
            match a["running_unix_ms"].as_i64() {
                Some(run) if run >= start && run <= end => {
                    active.push((run, end));
                    overhead.push((start, run));
                }
                None if a["running_unix_ms"].is_null() => never_running += 1,
                _ => unknown += 1,
            }
        }
        let last = spans.iter().map(|i| i.1).max();
        let total = first.zip(last).map(|(s, e)| e - s);
        let running_attempt_ms = active.iter().map(|(a, b)| b - a).sum::<i64>();
        let running = union(&mut active);
        let occupied = union(&mut spans);
        // Overlapping attempts use wall-time unions; running takes priority over
        // launch overhead so the three shares partition elapsed wall time.
        let mut all = active.clone();
        all.extend(overhead);
        let launch = union(&mut all) - running;
        let value = if let Some(total) = total.filter(|_| unknown == 0) {
            let waiting = total - occupied;
            let mut value = json!({"worker_running_ms":running,"worker_attempt_ms":running_attempt_ms,"launch_overhead_ms":launch,"waiting_between_attempts_ms":waiting,"total_elapsed_ms":total,"shares":{"worker_running":ratio(running,total),"launch_overhead":ratio(launch,total),"waiting_between_attempts":ratio(waiting,total)}});
            if never_running > 0 {
                value["status"] = json!("partial");
                value["reason"] = json!("never_running_attempt");
                value["known_launch_overhead_ms"] = json!(launch);
                value["launch_overhead_ms"] = absent("never_running_attempt");
                value["shares"]["launch_overhead"] = absent("never_running_attempt");
            }
            value
        } else {
            absent(if censored > 0 {
                "open_work_item"
            } else {
                "lifecycle_time_missing"
            })
        };
        items.push(json!({"work_item":work,"value":value,"attempts":attempts.iter().filter(|a| !a["attempt_id"].is_null()).count(),"never_running_attempts":never_running,"missing_attempt_times":unknown,"censored_attempts":censored}));
    }
    let missing = items
        .iter()
        .filter(|item| {
            matches!(
                item["value"]["status"].as_str(),
                Some("unavailable" | "partial")
            )
        })
        .count();
    let value = if items.is_empty() {
        absent("no_work_item_time_samples")
    } else {
        json!(items)
    };
    let mut body = json!({"value":value,"tasks_without_lineage":unbound,"time_basis":"wall_time_union_running_priority","incomplete_work_items":missing});
    if missing > 0 {
        body["status"] = json!("partial");
        body["reason"] = json!("work_item_time_incomplete");
    }
    Ok(body)
}
fn diffs(db: &Connection, side: Option<&Connection>, since: Option<i64>) -> Result<Value> {
    let rows=db.prepare("SELECT s.submission_id,s.attempt_id,json_extract(i.payload,'$.inputs.effective_profile.name') FROM result_submissions s LEFT JOIN attempt_inputs i ON i.attempt_id=s.attempt_id WHERE s.created_unix_ms>=?1 AND NOT EXISTS(SELECT 1 FROM replay_candidates r WHERE r.task_id=s.task_id) ORDER BY s.created_unix_ms,s.submission_id")?.query_map([since.unwrap_or(0)],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let available = side
        .map(|s| table(s, "operation_submission_diff"))
        .transpose()?
        .unwrap_or(false);
    let costs = if let Some(s) = side {
        crate::telemetry::accounting::cost::cost(s, None, None)?
    } else {
        Value::Null
    };
    let mut submissions = Vec::new();
    let mut profiles = BTreeMap::<String, (i64, BTreeSet<String>, usize)>::new();
    let mut missing = 0;
    let mut binary_submissions = 0;
    for (id, attempt, profile) in rows {
        use rusqlite::OptionalExtension;
        let retained: Option<String> = db.query_row("SELECT json_extract(metadata,'$.diff_size') FROM verification_runs WHERE submission_id=?1 AND json_type(metadata,'$.diff_size')='object' ORDER BY created_unix_ms,run_id LIMIT 1", [&id], |r|r.get(0)).optional()?;
        let counts = if let Some(retained) = retained {
            let v: Value = serde_json::from_str(&retained)?;
            Some((
                v["added"].as_i64(),
                v["removed"].as_i64(),
                v["binary_files"].as_i64(),
                v["reason"].as_str().map(str::to_owned),
            ))
        } else if available {
            side.unwrap().query_row("SELECT added,removed,binary_files,reason FROM operation_submission_diff WHERE submission_id=?1",[&id],|r|Ok((r.get::<_,Option<i64>>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?))).optional()?
        } else {
            None
        };
        let value = match counts {
            Some((Some(a), Some(d), Some(b), None)) => {
                json!({"added":a,"removed":d,"changed_lines":a+d,"binary_files":b,"status":if b>0 {"partial"}else{"available"},"reason":if b>0 {Some("binary_lines_unknown")}else{None}})
            }
            Some(_) => absent("diff_unavailable"),
            None => absent("submission_diff_not_observed"),
        };
        let group = profiles
            .entry(profile.clone().unwrap_or_else(|| "unknown".into()))
            .or_default();
        if let Some(lines) = value["changed_lines"].as_i64() {
            group.0 += lines;
            group.1.insert(attempt.clone());
            if value["binary_files"].as_i64().unwrap_or(0) > 0 {
                group.2 += 1;
                binary_submissions += 1;
            }
        } else {
            missing += 1;
            group.2 += 1;
        }
        submissions
            .push(json!({"submission_id":id,"attempt_id":attempt,"profile":profile,"value":value}));
    }
    let mut by_profile = BTreeMap::new();
    for (profile, (lines, attempts, unknown)) in profiles {
        let entries:Vec<_>=attempts.iter().map(|id| {
            let cost=costs["attempts"].as_array().into_iter().flatten().find(|c|c["attempt_id"].as_str()==Some(id));
            let estimate=cost.map(|c|&c["estimate"]);
            match estimate {Some(e) if e["status"]=="complete"=>json!({"valuation":{"status":"priced","amount":e["amount"],"currency":e["currency"]}}),_=>json!({"valuation":{"status":"unavailable","reason":"cost_not_observed"}})}
        }).collect();
        let (spend, coverage) =
            crate::telemetry::accounting::cost::summarize(&entries.iter().collect::<Vec<_>>())?;
        let cost_per_line = if unknown > 0 {
            absent("diff_coverage_incomplete")
        } else if lines == 0 {
            absent("empty_denominator")
        } else if spend["amount"].as_str().is_some() {
            json!({"amount_per_changed_line":format!("{}/{}",spend["amount"].as_str().unwrap(),lines),"currency":spend["currency"],"cost_coverage":coverage})
        } else {
            absent("cost_not_observed")
        };
        by_profile.insert(profile,json!({"changed_lines":lines,"attempts":attempts.len(),"missing_or_binary_submissions":unknown,"cost_per_changed_line":cost_per_line,"spend":spend}));
    }
    let mut out = json!({"value":if submissions.is_empty(){absent("no_submissions")}else{json!(submissions)},"by_profile":by_profile,"missing_submissions":missing,"binary_submissions":binary_submissions});
    if missing > 0 || binary_submissions > 0 {
        out["status"] = json!("partial");
        out["reason"] = json!(if missing > 0 {
            "submission_diff_not_observed"
        } else {
            "binary_lines_unknown"
        });
    }
    Ok(out)
}
fn memory(db: &Connection, since: Option<i64>) -> Result<Value> {
    if !table(db, "memory_proposals")? {
        return Ok(json!({"value":absent("memory_history_not_recorded")}));
    }
    let (proposed,accepted,rejected):(i64,i64,i64)=db.query_row("SELECT count(*),coalesce(sum(EXISTS(SELECT 1 FROM memory_promotions m WHERE m.proposal_id=p.id)),0),coalesce(sum(NOT EXISTS(SELECT 1 FROM memory_promotions m WHERE m.proposal_id=p.id) AND (p.review_state='rejected' OR (SELECT d.decision FROM review_decisions d WHERE d.proposal_id=p.id ORDER BY d.created_unix_ms DESC,d.rowid DESC LIMIT 1)='reject')),0) FROM memory_proposals p WHERE p.created_unix_ms>=?1",[since.unwrap_or(0)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let briefs = if table(db, "consumer_bindings")? {
        let (count,facts,omitted):(i64,i64,i64)=db.query_row("WITH delivered AS (
            SELECT DISTINCT entity,json_extract(payload,'$.intent.knowledge.id') AS snapshot_id
            FROM events WHERE kind='runtime.worker_brief_delivered'
            AND json_extract(payload,'$.observed_unix_ms')>=?1)
            SELECT count(*),coalesce(sum((SELECT count(*) FROM snapshot_entries e WHERE e.snapshot_id=s.id)),0),coalesce(sum(s.omitted_optional_count),0)
            FROM delivered d JOIN memory_snapshots s ON s.id=d.snapshot_id",[since.unwrap_or(0)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let prepared:i64 = db.query_row("SELECT count(*) FROM consumer_bindings b WHERE b.attempt_id IS NOT NULL AND b.created_unix_ms>=?1 AND NOT EXISTS(SELECT 1 FROM events e WHERE e.kind='runtime.worker_brief_delivered' AND json_extract(e.payload,'$.intent.attempt')=b.attempt_id AND json_extract(e.payload,'$.intent.knowledge.id')=b.snapshot_id)",[since.unwrap_or(0)],|r|r.get(0))?;
        let mut value = json!({"briefs":count,"facts_delivered":facts,"omitted_for_budget":omitted,"prepared_without_delivery":prepared,"basis":"acknowledged_worker_briefs"});
        if count == 0 {
            value["status"] = json!("unavailable");
            value["reason"] = json!("no_brief_delivery_samples");
            value["facts_delivered"] = absent("no_brief_delivery_samples");
            value["omitted_for_budget"] = absent("no_brief_delivery_samples");
        }
        value
    } else {
        absent("brief_delivery_history_not_recorded")
    };
    let remember = if table(db, "result_memory_candidates")? {
        json!(db.query_row("SELECT count(*) FROM result_memory_candidates c JOIN result_submissions s USING(submission_id) WHERE s.created_unix_ms>=?1",[since.unwrap_or(0)],|r|r.get::<_,i64>(0))?)
    } else {
        absent("remember_history_not_recorded")
    };
    Ok(
        json!({"value":{"candidates_proposed":proposed,"candidates_accepted":accepted,"candidates_rejected":rejected,"briefs":briefs,"remember_sections_written":remember},"remember_basis":"distinct_canonical_attempt_content_candidates","legacy_remember":absent("legacy_remember_not_observed")}),
    )
}
pub(super) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let side = crate::telemetry::sidecar::read(project)?;
    let rows = invocations(side.as_deref(), since)?;
    let mut out: BTreeMap<String, Value> = BTreeMap::from([
        ("M90".into(), launches(&db, rows.as_deref())?),
        ("M91".into(), interventions(&db, rows.as_deref())?),
        ("M92".into(), ticker(side.as_deref(), since)?),
        ("M93".into(), time_breakdown(project, since)?),
        (
            "M94".into(),
            diffs(&db, side.as_deref(), since)?,
        ),
        ("M95".into(), memory(&db, since)?),
    ]);
    for (id, body) in &mut out {
        body["definition"] = json!(format!("{id}.v1"));
        body["name"] = json!(super::registry::find(id).map(|m| m.name));
    }
    Ok(out)
}
