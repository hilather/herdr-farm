//! Claude metadata-only turn projection, shared by M34 and attempt counts.
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use super::cost::Dec;

fn trigger(p: &Value, queued: bool, background: bool) -> &'static str {
    let classify = |tag: &str| match tag {
        "scheduled_wakeup" | "scheduled" | "timer" | "wakeup" => Some("scheduled_wakeup"),
        "subagent" | "subagent_notification" => Some("subagent"),
        "task_notification" | "background_task" | "background_task_notification" => Some("background_task"),
        "bootstrap" | "startup" | "init" => Some("bootstrap"),
        "queued_owner" | "queue" | "queued" => Some("queued_owner"),
        "human" | "owner_typed" | "user" | "owner" | "typed" | "cli" => Some("owner_typed"),
        _ => None,
    };
    // Per-turn metadata is stronger evidence than preceding session activity.
    p["turn_origin"].as_str().and_then(classify)
        .or_else(|| p["prompt_source"].as_str().and_then(classify))
        .unwrap_or(if background { "background_task" } else if queued { "queued_owner" } else { "other" })
}

pub(super) fn read(db: &Connection, session: &str, cost: &Value) -> Result<Value> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='claude_turn_lines')", [], |r| r.get(0))?;
    if !exists || !session.starts_with("claude-code:") { return Ok(json!({"status":"unavailable","reason":"turn_metadata_not_collected"})); }
    let rows = db.prepare("SELECT occurred_unix_ms,is_prompt,metadata FROM claude_turn_lines WHERE session_id=?1 ORDER BY occurred_unix_ms,path_digest,byte_offset")?
        .query_map([session], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, bool>(1)?, r.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let entries: BTreeMap<String, String> = db.prepare("SELECT response_id,entry_id FROM usage_entries WHERE session_id=?1 AND response_id IS NOT NULL")?
        .query_map([session], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let valuations: BTreeMap<&str, &Value> = cost["sessions"].as_array().into_iter().flatten().filter(|s| s["session_id"] == session)
        .flat_map(|s| s["entries"].as_array().into_iter().flatten()).filter_map(|e| e["entry_id"].as_str().map(|id| (id, &e["valuation"]))).collect();
    let mut turns = Vec::<Value>::new();
    let (mut queued, mut background) = (false, false);
    let mut messages = BTreeSet::new();
    let mut question_calls = BTreeSet::new();
    let mut request_contexts = BTreeMap::<String, i64>::new();
    for (time, prompt, text) in rows {
        let p: Value = serde_json::from_str(&text)?;
        if p["line_type"] == "queue-operation" { queued = p["operation"] == "enqueue"; }
        if p["background_task_id"].is_string() { background = true; }
        if prompt {
            request_contexts.clear();
            let previous_end = turns.last().and_then(|t| t["ended_unix_ms"].as_i64());
            turns.push(json!({"session_id":session,"turn":turns.len()+1,"trigger_class":trigger(&p,queued,background),
                "started_unix_ms":time,"ended_unix_ms":time,"requests":0,"context_tokens_max":0,"context_tokens_sum":0,
                "request_samples":{},"questions":[],"cache_write_5m_tokens":0,"cache_write_1h_tokens":0,"tool_calls":0,"ask_user_question_wait_ms":0,
                "wall_duration_ms":null,"cost_by_currency":{},"unpriced_requests":0,"stop_reasons":{},
                "idle_gap_ms":time.zip(previous_end).map(|(a,b)| (a-b).max(0)),"full_context_cache_rewrite":false}));
            queued = false; background = false;
        }
        let Some(t) = turns.last_mut() else { continue };
        if let Some(time) = time { t["ended_unix_ms"] = json!(time); }
        if p["line_type"] == "system" && p["duration_ms"].is_number() { t["wall_duration_ms"] = p["duration_ms"].clone(); }
        if let Some(reason) = p["stop_reason"].as_str() {
            let n = t["stop_reasons"][reason].as_u64().unwrap_or(0);
            t["stop_reasons"][reason] = json!(n+1);
        }
        let tools = p["tool_use_ids"].as_array().map_or(0, Vec::len);
        t["tool_calls"] = json!(t["tool_calls"].as_u64().unwrap_or(0)+tools as u64);
        for call in p["tool_use_ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let question: bool = db.query_row("SELECT name='AskUserQuestion' FROM codex_tool_calls WHERE session_id=?1 AND call_id=?2", [session,call], |r| r.get(0)).unwrap_or(false);
            let wait: Option<i64> = db.query_row("SELECT CASE WHEN name='AskUserQuestion' THEN max(0,output_unix_ms-called_unix_ms) END FROM codex_tool_calls WHERE session_id=?1 AND call_id=?2",
                [session,call], |r| r.get(0)).unwrap_or(None);
            t["ended_with_question"] = json!(question);
            if question && question_calls.insert(call.to_owned()) { t["questions"].as_array_mut().unwrap().push(json!({"call_id":call,"called_unix_ms":time,"answer_wait_ms":wait})); }
            t["ask_user_question_wait_ms"] = json!(t["ask_user_question_wait_ms"].as_i64().unwrap_or(0)+wait.unwrap_or(0));
        }
        if p["line_type"] != "assistant" { continue; }
        let Some(id) = p["message_id"].as_str() else { continue };
        if !p["input_tokens"].is_number() || !messages.insert(id.to_owned()) { continue; }
        let counters: Option<Vec<i64>> = ["input_tokens","cache_read_input_tokens","cache_creation_input_tokens"].iter()
            .map(|k| p[k].as_i64().filter(|n| (0..=9007199254740992).contains(n))).collect();
        let Some(counters) = counters else { continue };
        let context = counters.iter().sum::<i64>();
        let request = p["request_id"].as_str().unwrap_or(id).to_owned();
        let previous = request_contexts.insert(request.clone(), context).unwrap_or(0);
        let maximum = previous.max(context);
        request_contexts.insert(request.clone(), maximum);
        let rewrite = counters[1] == 0 && counters[2] > 0;
        if t["request_samples"][&request].is_null() {
            t["request_samples"][&request] = json!({"session_id":session,"request_id":request,"recorded_unix_ms":time,
                "context_tokens":maximum,"rewrite":rewrite,"valuations":[],"idle_gap_ms":t["idle_gap_ms"]});
        }
        let sample = &mut t["request_samples"][&request];
        sample["context_tokens"] = json!(maximum);
        sample["rewrite"] = json!(rewrite || sample["rewrite"] == true);
        sample["valuations"].as_array_mut().unwrap().push(json!(entries.get(id).and_then(|e| valuations.get(e.as_str()))));
        t["requests"] = json!(request_contexts.len());
        t["context_tokens_max"] = json!(t["context_tokens_max"].as_i64().unwrap_or(0).max(context));
        t["context_tokens_sum"] = json!(t["context_tokens_sum"].as_i64().unwrap_or(0).checked_add(maximum-previous).ok_or_else(|| anyhow::anyhow!("turn context sum overflow"))?);
        for key in ["cache_write_5m_tokens","cache_write_1h_tokens"] {
            t[key] = json!(t[key].as_i64().zip(p[key].as_i64().filter(|n| (0..=9007199254740992).contains(n)))
                .and_then(|(sum,n)| sum.checked_add(n)));
        }
        if p["cache_read_input_tokens"].as_i64() == Some(0) && p["cache_creation_input_tokens"].as_i64().is_some_and(|n| n > 0) { t["full_context_cache_rewrite"] = json!(true); }
        let v = entries.get(id).and_then(|e| valuations.get(e.as_str()));
        if let Some(v) = v.filter(|v| v["status"] == "priced") {
            let currency = v["currency"].as_str().unwrap_or_default();
            let amount = Dec::parse(v["amount"].as_str().unwrap_or("0"))?;
            let sum = Dec::parse(t["cost_by_currency"][currency].as_str().unwrap_or("0"))?.add(amount)?;
            t["cost_by_currency"][currency] = json!(sum.to_string());
        } else { t["unpriced_requests"] = json!(t["unpriced_requests"].as_u64().unwrap_or(0)+1); }
    }
    let mut by_trigger = BTreeMap::<String, BTreeMap<String, Dec>>::new();
    for t in &mut turns {
        if t["wall_duration_ms"].is_null() { t["wall_duration_ms"] = json!(t["ended_unix_ms"].as_i64().zip(t["started_unix_ms"].as_i64()).map(|(a,b)| (a-b).max(0))); }
        for (currency, amount) in t["cost_by_currency"].as_object().into_iter().flatten() {
            let sum = by_trigger.entry(t["trigger_class"].as_str().unwrap_or("other").to_owned()).or_default().entry(currency.clone()).or_insert(Dec::ZERO);
            *sum = sum.add(Dec::parse(amount.as_str().unwrap_or("0"))?)?;
        }
        t["no_tool_call"] = json!(t["tool_calls"] == 0);
    }
    let costs: BTreeMap<_, BTreeMap<_,_>> = by_trigger.into_iter().map(|(k,v)| (k,v.into_iter().map(|(c,d)| (c,d.to_string())).collect())).collect();
    Ok(json!({"turns":turns,"cost_by_trigger_class":costs,
        "idle_gaps_before_cache_rewrites":turns.iter().filter(|t| t["full_context_cache_rewrite"] == true && t["idle_gap_ms"].is_number()).collect::<Vec<_>>(),
        "no_tool_call_turns":turns.iter().filter(|t| t["no_tool_call"] == true).collect::<Vec<_>>()}))
}

/// Coordinator-wide summaries preserve currencies and session identity.
pub(super) fn summarize(sessions: &[Value]) -> Result<Value> {
    let mut costs = BTreeMap::<String, BTreeMap<String, Dec>>::new();
    let mut gaps = Vec::new();
    let mut no_ops = Vec::new();
    let mut turns = 0usize;
    let mut requests = 0u64;
    for session in sessions {
        for turn in session["turns"].as_array().into_iter().flatten() {
            turns += 1;
            requests += turn["requests"].as_u64().unwrap_or(0);
        }
        gaps.extend(session["idle_gaps_before_cache_rewrites"].as_array().into_iter().flatten().cloned());
        no_ops.extend(session["no_tool_call_turns"].as_array().into_iter().flatten().cloned());
        for (trigger, currencies) in session["cost_by_trigger_class"].as_object().into_iter().flatten() {
            for (currency, amount) in currencies.as_object().into_iter().flatten() {
                let sum = costs.entry(trigger.clone()).or_default().entry(currency.clone()).or_insert(Dec::ZERO);
                *sum = sum.add(Dec::parse(amount.as_str().unwrap_or("0"))?)?;
            }
        }
    }
    let costs: BTreeMap<_, BTreeMap<_,_>> = costs.into_iter().map(|(k,v)| (k,v.into_iter().map(|(c,d)| (c,d.to_string())).collect())).collect();
    Ok(json!({"turns":turns,"requests":requests,"cost_by_trigger_class":costs,
        "idle_gaps_before_cache_rewrites":gaps,"no_tool_call_turns":no_ops}))
}
