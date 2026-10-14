//! M104: metadata-only timing collected in the existing incremental native pass.
use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

const LIMIT: i64 = 4096;
fn timestamp(raw: &Value) -> Option<i64> {
    raw["timestamp"]
        .as_str()?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_millisecond())
}
fn model_id(raw: &Value) -> Option<String> {
    crate::telemetry::sanitize::field(raw, "model", crate::telemetry::sanitize::Class::ModelId)
        .as_str()
        .filter(|m| m.len() <= 160)
        .map(str::to_owned)
}
fn counter(v: &Value) -> Option<i64> {
    v.as_i64()
        .filter(|n| (0..=9_007_199_254_740_991).contains(n))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    tx: &Transaction,
    key: &str,
    session: &str,
    offset: u64,
    raw: &Value,
    model: Option<&str>,
    ordinal: Option<i64>,
    claude: bool,
) -> Result<()> {
    let model = model.and_then(|m| model_id(&json!({"model":m})));
    let offset = offset as i64;
    let previous: Option<i64> = tx
        .query_row(
            "SELECT byte_offset FROM model_time_state WHERE path_digest=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    if previous.is_some_and(|p| p >= offset) {
        return Ok(());
    }
    let mut time = timestamp(raw);
    if claude && raw["type"] == "result" && time.is_none() {
        let span = counter(&raw["duration_ms"]);
        let start: Option<i64> = tx
            .query_row(
                "SELECT first_ms FROM model_time_state WHERE path_digest=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        time = start.zip(span).and_then(|(s, d)| s.checked_add(d));
        if time.is_none() {
            time = tx
                .query_row(
                    "SELECT last_ms FROM model_time_state WHERE path_digest=?1",
                    [key],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
        }
    }
    tx.execute("INSERT INTO model_time_state(path_digest,session_id,byte_offset,first_ms,last_ms,model) VALUES(?1,?2,?3,coalesce((SELECT session_unix_ms FROM rollout_sources WHERE path_digest=?1),?4),?4,?5)
        ON CONFLICT(path_digest) DO UPDATE SET byte_offset=excluded.byte_offset, first_ms=coalesce(first_ms,excluded.first_ms),
        last_ms=CASE WHEN excluded.last_ms IS NULL THEN last_ms ELSE max(coalesce(last_ms,excluded.last_ms),excluded.last_ms) END, model=coalesce(excluded.model,model)", params![key,session,offset,time,model])?;
    let kind = raw["type"].as_str().unwrap_or("");
    let payload = &raw["payload"];
    if !claude {
        let trigger = kind == "turn_context"
            || (kind == "response_item"
                && (matches!(
                    payload["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                ) || (payload["type"] == "message" && payload["role"] == "user")))
            || (kind == "event_msg" && matches!(payload["type"].as_str(), Some("user_message")));
        if trigger {
            let reported_model = model_id(payload);
            tx.execute("UPDATE model_time_state SET trigger_ms=?2,model=CASE WHEN ?4 THEN ?3 ELSE coalesce(?3,model) END WHERE path_digest=?1", params![key,time,reported_model,kind == "turn_context"])?;
        }
        if kind == "token_usage_record" {
            tx.execute(
                "UPDATE model_time_state SET pending_ms=?2,pending_ordinal=?3 WHERE path_digest=?1",
                params![key, time, ordinal],
            )?;
        }
        if kind == "event_msg" && payload["type"] == "token_count" {
            let pair: (Option<i64>,Option<i64>,Option<i64>) = tx.query_row("SELECT trigger_ms,pending_ms,pending_ordinal FROM model_time_state WHERE path_digest=?1", [key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            if let (Some(start), Some(end), Some(tokens)) = (
                pair.0,
                pair.1,
                counter(&payload["info"]["last_token_usage"]["output_tokens"]),
            ) {
                sample(
                    tx,
                    key,
                    session,
                    offset,
                    pair.2,
                    end,
                    end.saturating_sub(start),
                    tokens,
                    false,
                )?;
            }
            if pair.1.is_some() {
                tx.execute("UPDATE model_time_state SET pending_ms=NULL,pending_ordinal=NULL,trigger_ms=NULL WHERE path_digest=?1", [key])?;
            }
        }
    } else {
        // Claude's result duration is API time, not tool or whole-turn time.
        // Per-request durations are used when explicitly exposed by an assistant.
        let duration = counter(&raw["duration_api_ms"]);
        let tokens = if kind == "result" {
            counter(&raw["usage"]["output_tokens"])
        } else {
            counter(&raw["message"]["usage"]["output_tokens"])
        };
        if kind == "assistant"
            && let Some(id) = raw["message"]["id"].as_str()
        {
            let seen: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM claude_messages WHERE session_id=?1 AND message_id=?2)", params![session,id], |r| r.get(0))?;
            if seen {
                return Ok(());
            }
        }
        let reported_model = model_id(&raw["message"]).or_else(|| model_id(raw));
        if kind == "result" && reported_model.is_none() {
            let models: i64 = tx.query_row(
                "SELECT count(DISTINCT model) FROM codex_usage WHERE session_id=?1 AND accepted=1",
                [session],
                |r| r.get(0),
            )?;
            if models > 1 {
                tx.execute(
                    "UPDATE model_time_state SET model=NULL WHERE path_digest=?1",
                    [key],
                )?;
            }
        }
        if let Some(model) = reported_model {
            tx.execute(
                "UPDATE model_time_state SET model=?2 WHERE path_digest=?1",
                params![key, model],
            )?;
        }
        if kind == "user"
            && !raw["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
        {
            tx.execute(
                "UPDATE model_time_state SET pending_ordinal=?2 WHERE path_digest=?1",
                params![key, offset],
            )?;
        }
        if matches!(kind, "result" | "assistant")
            && let (Some(duration), Some(tokens), Some(end)) = (duration, tokens, time)
        {
            // A result summarizes requests; prefer it only when no per-request
            // samples were exposed, so the same output is never counted twice.
            let count: i64 = tx.query_row("SELECT count(*) FROM model_time_samples WHERE path_digest=?1 AND byte_offset > coalesce((SELECT pending_ordinal FROM model_time_state WHERE path_digest=?1),-1)", [key], |r| r.get(0))?;
            if kind != "result" || count == 0 {
                let request_ordinal = if kind == "assistant" {
                    Some(tx.query_row("SELECT coalesce(max(ordinal),0)+1 FROM claude_messages WHERE session_id=?1", [session], |r| r.get::<_,i64>(0))?)
                } else {
                    None
                };
                sample(
                    tx,
                    key,
                    session,
                    offset,
                    request_ordinal,
                    end,
                    duration,
                    tokens,
                    kind == "result",
                )?;
            }
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn sample(
    tx: &Transaction,
    key: &str,
    session: &str,
    offset: i64,
    ordinal: Option<i64>,
    end: i64,
    duration: i64,
    tokens: i64,
    aggregate: bool,
) -> Result<()> {
    if duration <= 0 {
        return Ok(());
    }
    tx.execute("INSERT OR IGNORE INTO model_time_samples(path_digest,session_id,byte_offset,ordinal,model,ended_ms,duration_ms,output_tokens,aggregate)
        SELECT path_digest,session_id,?2,?3,model,?4,?5,?6,?9 FROM model_time_state WHERE path_digest=?1
        AND (SELECT count(*) FROM model_time_samples WHERE session_id=?7) < ?8", params![key,offset,ordinal,end,duration,tokens,session,LIMIT,aggregate])?;
    Ok(())
}

fn decimal(n: f64) -> String {
    format!("{n:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
#[derive(Default)]
struct Group {
    tokens: i64,
    duration: i64,
    wall: i64,
    rates: Vec<f64>,
    missing: i64,
    capped: bool,
    aggregated: bool,
}
impl Group {
    fn value(mut self) -> Value {
        if self.rates.is_empty() {
            return json!({"status":"unavailable","reason":"model_duration_not_reported"});
        }
        self.rates.sort_by(f64::total_cmp);
        let percentile =
            |p: usize| decimal(self.rates[(self.rates.len() * p).div_ceil(100).saturating_sub(1)]);
        json!({"status":if self.missing>0 || self.capped {"partial"} else {"available"},
            "reason":if self.capped {json!("response_sample_limit")} else if self.missing>0 {json!("model_duration_not_reported")} else {Value::Null},
            "output_tokens":self.tokens,"model_time_ms":self.duration,"responses":self.rates.len(),"missing_responses":self.missing,
            "tokens_per_second":decimal(self.tokens as f64*1000.0/self.duration as f64),
            "distribution":if self.aggregated {json!({"status":"unavailable","reason":"per_request_durations_not_reported"})} else {json!({"median":percentile(50),"p10":percentile(10),"p90":percentile(90)})},
            "session_wall_time_ms":self.wall,"model_wall_time_share":if self.wall>0 {json!(decimal(self.duration as f64/self.wall as f64))} else {json!({"status":"unavailable","reason":"session_wall_time_not_reported"})}})
    }
}

pub(crate) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let mut total = Group::default();
    let mut coordinator = Group::default();
    let mut models = BTreeMap::<String, Group>::new();
    let mut profiles = BTreeMap::<String, Group>::new();
    if let Some(db) = crate::telemetry::sidecar::read(project)?
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='model_time_samples')",
            [],
            |r| r.get::<_, bool>(0),
        )?
    {
        let state = crate::telemetry::read_only(&project.join(".state/state.db"))?;
        let profile_names: BTreeMap<String,String> = state.prepare("SELECT attempt_id,coalesce(json_extract(payload,'$.inputs.effective_profile.name'),'unknown') FROM attempt_inputs")?
            .query_map([], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let sessions = db.prepare("SELECT s.session_id,s.attempt_id,min(coalesce(t.first_ms,s.session_unix_ms)),max(coalesce(max(c.last_record_unix_ms,t.last_ms),t.last_ms,c.last_record_unix_ms)),count(DISTINCT u.ordinal) FROM rollout_sources s
            LEFT JOIN model_time_state t ON t.path_digest=s.path_digest LEFT JOIN codex_session_clock c ON c.session_id=s.session_id LEFT JOIN codex_usage u ON u.session_id=s.session_id AND u.accepted=1
            WHERE s.binding='bound' OR (s.session_id LIKE 'claude-code:%' AND s.binding='unbound' AND s.cwd_attempt IS NULL AND s.cwd IN (?1,?2)) GROUP BY s.session_id")?.query_map(params![project.to_string_lossy(),crate::telemetry::sanitize::home_prefix(&project.to_string_lossy())], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,i64>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (session, attempt, start, end, count) in sessions {
            let samples = db.prepare("SELECT coalesce(m.model,'unknown'),m.duration_ms,m.output_tokens,m.aggregate FROM model_time_samples m
                WHERE m.session_id=?1 AND m.ended_ms>=?2 AND (m.ordinal IS NULL OR EXISTS(SELECT 1 FROM codex_usage u WHERE u.session_id=m.session_id AND u.ordinal=m.ordinal AND u.accepted=1))")?
                .query_map(params![session,since.unwrap_or(i64::MIN)], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if end.is_some_and(|e| since.is_some_and(|s| e < s)) {
                continue;
            }
            let profile = attempt
                .as_ref()
                .and_then(|a| profile_names.get(a))
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            let wall = start
                .zip(end)
                .map_or(0, |(s, e)| e.saturating_sub(s.max(since.unwrap_or(s))));
            let missing = if session.starts_with("claude-code:") && samples.iter().any(|s| s.3) {
                0
            } else if since.is_none() {
                count.saturating_sub(samples.len() as i64).max(0)
            } else {
                0
            };
            for g in [&mut total, profiles.entry(profile.clone()).or_default()] {
                g.wall += wall;
                g.missing += missing;
                g.capped |= samples.len() as i64 >= LIMIT;
            }
            if attempt.is_none() {
                coordinator.wall += wall;
                coordinator.missing += missing;
                coordinator.capped |= count >= LIMIT;
                for (_, duration, tokens, aggregate) in &samples {
                    coordinator.tokens += tokens;
                    coordinator.duration += duration;
                    coordinator
                        .rates
                        .push(*tokens as f64 * 1000.0 / *duration as f64);
                    coordinator.aggregated |= *aggregate;
                }
            }
            if missing > 0 {
                let gaps = db.prepare("SELECT coalesce(u.model,'unknown'),count(*) FROM codex_usage u
                    WHERE u.session_id=?1 AND u.accepted=1 AND NOT EXISTS(SELECT 1 FROM model_time_samples m WHERE m.session_id=u.session_id AND m.ordinal=u.ordinal)
                    GROUP BY u.model")?.query_map([&session], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                for (model, n) in gaps {
                    models.entry(model).or_default().missing += n;
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            for (model, duration, tokens, aggregate) in samples {
                let g = models.entry(model.clone()).or_default();
                if seen.insert(model) {
                    g.wall += wall;
                    g.capped |= count >= LIMIT;
                }
                for g in [
                    &mut total,
                    profiles.get_mut(&profile).expect("inserted profile"),
                    g,
                ] {
                    g.tokens += tokens;
                    g.duration += duration;
                    g.rates.push(tokens as f64 * 1000.0 / duration as f64);
                    g.aggregated |= aggregate;
                }
            }
        }
    }
    let value = total.value();
    let mut coordinator = coordinator.value();
    if coordinator["status"] == "unavailable" {
        coordinator["reason"] = json!("coordinator_request_duration_not_reported");
    }
    let body = json!({"definition":"M104.v1","value":value,"by_model":models.into_iter().map(|(k,v)|(k,v.value())).collect::<BTreeMap<_,_>>(),
        "by_profile":profiles.into_iter().map(|(k,v)|(k,v.value())).collect::<BTreeMap<_,_>>(),
        "coordinator":coordinator});
    Ok(BTreeMap::from([("M104".into(), body)]))
}

/// Group summaries shared by the report and models view, without new reads.
pub(crate) fn text(body: &Value) -> String {
    let display = |v: &Value| {
        crate::telemetry::metrics::structured_text(v)
            .unwrap_or_else(|| format!("n/a ({})", v["reason"].as_str().unwrap_or("unknown")))
    };
    let mut out = String::new();
    for (dimension, key) in [("model", "by_model"), ("profile", "by_profile")] {
        for (label, value) in body[key].as_object().into_iter().flatten() {
            out += &format!("    M104 {dimension}={label}: {}\n", display(value));
        }
    }
    if body.get("coordinator").is_some() {
        out += &format!("    M104 coordinator: {}\n", display(&body["coordinator"]));
    }
    out
}
