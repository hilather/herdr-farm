//! Read-only per-attempt summaries from sanitized Codex metadata.
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) fn attempt(
    db: &Connection,
    canonical: &Connection,
    record: &Value,
) -> anyhow::Result<Value> {
    let id = record["attempt_id"].as_str().unwrap_or_default();
    let available: bool = db.query_row("SELECT count(*)=8 FROM sqlite_master WHERE type='table' AND name IN ('codex_session_clock','codex_session_turns','codex_session_items','codex_tool_calls','codex_exec_items','codex_turns','codex_turn_aborts','codex_usage')", [], |r| r.get(0))?;
    let sessions: Vec<String> = db.prepare("SELECT DISTINCT session_id FROM rollout_sources WHERE attempt_id=?1 AND binding='bound' AND (originator IS NULL OR originator NOT LIKE 'otlp:%')")?
        .query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let never_running = sessions.is_empty() && record["running_unix_ms"].is_null();
    if never_running {
        return Ok(json!({"status":"excluded","reason":"never_running","end_state":"never_running"}));
    }
    if !available {
        let end = if record["session"]["end_state"] == "never_running" { json!("unknown") } else { record["session"]["end_state"].clone() };
        return Ok(json!({"status":"unavailable","reason":"session_metadata_not_collected","end_state":end}));
    }
    let canonical_ended: bool = record["agent_kind"] == "claude" && canonical.query_row(
        "SELECT EXISTS(SELECT 1 FROM inbox_items WHERE json_extract(payload,'$.kind')='attempt.ended_without_submission' AND instr(json_extract(payload,'$.summary'),?1)>0)",
        [format!("attempt {id},")], |r| r.get(0))?;
    let mut commands = 0i64;
    let mut unknown_commands = 0i64;
    let mut failed = BTreeMap::from([
        ("127_not_found", 0i64),
        ("1", 0),
        ("2", 0),
        ("other_nonzero", 0),
        ("signal", 0),
    ]);
    let (
        mut turns,
        mut files,
        mut failed_files,
        mut images,
        mut requests,
        mut answered,
        mut spawned,
    ) = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
    let mut fill: Option<f64> = None;
    let mut last: Option<i64> = None;
    let mut all_completed = !sessions.is_empty();
    let mut observed = false;
    let mut missing_timestamp = false;
    for session in &sessions {
        let clock: Option<(Option<i64>, bool)> = db.query_row("SELECT last_record_unix_ms,last_turn_completed FROM codex_session_clock WHERE session_id=?1", [session], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        observed |= clock.is_some();
        missing_timestamp |= clock.as_ref().is_none_or(|c| c.0.is_none());
        all_completed &= clock.as_ref().is_some_and(|c| c.1);
        if let Some(at) = clock.and_then(|c| c.0) {
            last = Some(last.map_or(at, |old| old.max(at)));
        }
        turns += db.query_row("SELECT count(*) FROM (SELECT turn_id FROM codex_session_turns WHERE session_id=?1 UNION SELECT turn_id FROM codex_turns WHERE session_id=?1 UNION SELECT turn_id FROM codex_turn_aborts WHERE session_id=?1)",[session],|r|r.get::<_,i64>(0))?;
        for code in db
            .prepare("SELECT exit_code FROM codex_exec_items WHERE session_id=?1")?
            .query_map([session], |r| r.get::<_, Option<i64>>(0))?
        {
            commands += 1;
            match code? {
                None => unknown_commands += 1,
                Some(0) => {}
                Some(n) => {
                    let class = match n {
                        127 => "127_not_found",
                        1 => "1",
                        2 => "2",
                        n if n < 0 || (129..=192).contains(&n) => "signal",
                        _ => "other_nonzero",
                    };
                    *failed.get_mut(class).unwrap() += 1;
                }
            }
        }
        let counts: (i64,i64,i64) = db.query_row("SELECT count(*) FILTER(WHERE item_type='FileChange'), count(*) FILTER(WHERE item_type='FileChange' AND status='failed'), count(*) FILTER(WHERE item_type='ImageView') FROM codex_session_items WHERE session_id=?1", [session], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        files += counts.0;
        failed_files += counts.1;
        images += counts.2;
        let calls: (i64,i64,i64) = db.query_row("SELECT count(*) FILTER(WHERE name GLOB 'request_user_input*'), count(*) FILTER(WHERE name GLOB 'request_user_input*' AND (EXISTS(SELECT 1 FROM codex_session_items i WHERE i.session_id=codex_tool_calls.session_id AND i.item_type='UserMessage' AND i.completed_unix_ms>called_unix_ms) OR (name='request_user_input' AND output_kind='function_call_output' AND output_unix_ms>=called_unix_ms))), count(*) FILTER(WHERE name='spawn_agent') FROM codex_tool_calls WHERE session_id=?1 AND call_kind='function_call'", [session], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        requests += calls.0;
        answered += calls.1;
        spawned += calls.2;
        let ratio: Option<f64> = db.query_row("SELECT max(1.0*u.input_tokens/t.model_context_window) FROM codex_usage u JOIN codex_session_turns t ON t.session_id=u.session_id AND t.turn_id=u.turn_id WHERE u.session_id=?1 AND u.accepted=1 AND t.model_context_window>0",[session],|r|r.get(0))?;
        if let Some(n) = ratio {
            fill = Some(fill.map_or(n, |old| old.max(n)));
        }
    }
    let stopped: bool = canonical.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE entity=?1 AND ((kind='runtime.worker_terminated' AND json_extract(payload,'$.cause') IN ('cancellation','completion')) OR kind='runtime.launch_stopped'))",[id],|r|r.get(0))?;
    let timed_out: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM codex_turn_aborts a JOIN codex_session_clock c ON c.session_id=a.session_id AND c.last_turn_id=a.turn_id WHERE a.session_id IN (SELECT session_id FROM rollout_sources WHERE attempt_id=?1 AND binding='bound') AND a.reason IN ('wall_budget','wall_budget_exceeded') AND c.last_record_unix_ms=?2)",rusqlite::params![id,last],|r|r.get(0))?;
    let end = if record["result"]["state"] != "not_submitted" {
        "submitted"
    } else if stopped || record["terminal_state"] == "cancelled" {
        "stopped"
    } else if timed_out {
        "timed_out"
    } else if all_completed || canonical_ended {
        "ended_without_submission"
    } else {
        "unknown"
    };
    if !observed {
        return Ok(
            json!({"status":"unavailable","reason":"session_metadata_not_collected","end_state":end}),
        );
    }
    let last = if missing_timestamp { None } else { last };
    let lingering = record["terminal_unix_ms"]
        .as_i64()
        .zip(last)
        .and_then(|(end, last)| end.checked_sub(last))
        .filter(|n| *n >= 0);
    Ok(
        json!({"status":"observed","turns":turns,"commands":commands,"commands_exit_unknown":unknown_commands,
        "failed_commands":failed.values().sum::<i64>(),"failed_commands_by_class":failed,"file_change_items":files,"failed_file_change_items":failed_files,
        "image_views":images,"user_input_requests":requests,"answered_user_input_requests":answered,"unanswered_user_input_requests":requests-answered,
        "answer_basis":"subsequent_user_message_or_sync_output","sub_agents_spawned":spawned,"max_context_window_fill":fill.map(|n| n.to_string()),
        "lingering_ms":lingering,"end_state":end,"end_state_basis":if canonical_ended && !all_completed && end == "ended_without_submission" {"canonical_notice"} else {"recorded_metadata"}}),
    )
}

pub(crate) fn summary(records: &[Value]) -> Value {
    let mut states: BTreeMap<String, i64> = [
        "submitted",
        "never_running",
        "ended_without_submission",
        "stopped",
        "timed_out",
        "unknown",
    ]
    .into_iter()
    .map(|state| (state.to_owned(), 0))
    .collect();
    let mut profiles = BTreeMap::<String, Value>::new();
    let mut classes = BTreeMap::<String, i64>::new();
    let (mut commands, mut failed, mut unanswered) = (0i64, 0i64, 0i64);
    let mut lingering = Vec::new();
    let mut observed_attempts = 0;
    let never_running = records.iter().filter(|a| a["session"]["end_state"] == "never_running").count();
    let eligible = records.len() - never_running;
    for a in records {
        let s = &a["session"];
        *states
            .entry(s["end_state"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
        if s["status"] != "observed" {
            continue;
        }
        observed_attempts += 1;
        let c = s["commands"].as_i64().unwrap_or(0);
        let f = s["failed_commands"].as_i64().unwrap_or(0);
        commands += c;
        failed += f;
        unanswered += s["unanswered_user_input_requests"].as_i64().unwrap_or(0);
        if let Some(n) = s["lingering_ms"].as_i64() {
            lingering.push(n);
        }
        for (k, v) in s["failed_commands_by_class"]
            .as_object()
            .into_iter()
            .flatten()
        {
            *classes.entry(k.clone()).or_default() += v.as_i64().unwrap_or(0);
        }
        let p = profiles
            .entry(a["profile"].as_str().unwrap_or("unknown").into())
            .or_insert_with(
                || json!({"commands":0,"failed_commands":0,"failed_commands_by_class":{}}),
            );
        p["commands"] = json!(p["commands"].as_i64().unwrap_or(0) + c);
        p["failed_commands"] = json!(p["failed_commands"].as_i64().unwrap_or(0) + f);
        for (k, v) in s["failed_commands_by_class"]
            .as_object()
            .into_iter()
            .flatten()
        {
            p["failed_commands_by_class"][k] = json!(
                p["failed_commands_by_class"][k].as_i64().unwrap_or(0) + v.as_i64().unwrap_or(0)
            );
        }
    }
    let share = |n: i64, d: i64| {
        if d > 0 {
            json!(format!("{n}/{d}"))
        } else {
            Value::Null
        }
    };
    for p in profiles.values_mut() {
        p["failed_command_share"] = share(
            p["failed_commands"].as_i64().unwrap_or(0),
            p["commands"].as_i64().unwrap_or(0),
        );
    }
    let by_class: BTreeMap<_, _> = classes
        .iter()
        .map(|(k, n)| (k.clone(), json!({"count":n,"share":share(*n,commands)})))
        .collect();
    lingering.sort_unstable();
    let percentile = |p: usize| {
        if lingering.is_empty() {
            Value::Null
        } else {
            json!(lingering[(lingering.len() * p).div_ceil(100) - 1])
        }
    };
    json!({"status":if observed_attempts==0 {"unavailable"} else if observed_attempts<eligible {"partial"} else {"observed"},"coverage":{"observed_attempts":observed_attempts,"unavailable_attempts":eligible-observed_attempts,"never_running_attempts":never_running},"end_states":states,"commands":commands,"failed_commands":failed,"failed_command_share":share(failed,commands),"by_class":by_class,"by_profile":profiles,
        "lingering_ms":{"samples":lingering.len(),"p50":percentile(50),"p95":percentile(95)},"unanswered_user_input_requests":unanswered})
}
