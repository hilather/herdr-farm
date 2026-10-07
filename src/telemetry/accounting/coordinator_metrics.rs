//! Metadata-only coordinator and owner experience, MET-COORD-1.
use crate::telemetry::accounting::cost::Dec;
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn unavailable(reason: &str) -> Value {
    json!({"status":"unavailable","reason":reason})
}
fn distribution(mut samples: Vec<i64>) -> Value {
    samples.sort_unstable();
    if samples.is_empty() {
        return unavailable("no_timed_samples");
    }
    let n = samples.len();
    json!({"samples":n,"median":samples[n.div_ceil(2)-1],"p90":samples[(n*90).div_ceil(100)-1],"max":samples[n-1],"total":samples.iter().map(|n| i128::from(*n)).sum::<i128>()})
}
fn ratio(n: usize, d: usize) -> Value {
    if d == 0 {
        unavailable("empty_denominator")
    } else {
        json!({"numerator":n,"denominator":d,"ratio":format!("{n}/{d}")})
    }
}
fn costs<'a>(rows: impl Iterator<Item = &'a Value>) -> Result<Value> {
    let mut sums = BTreeMap::<String, Dec>::new();
    let mut unpriced = 0;
    for v in rows {
        if v["status"] != "priced" {
            unpriced += 1;
            continue;
        }
        let currency = v["currency"].as_str().unwrap_or_default().to_owned();
        let sum = sums.entry(currency).or_insert(Dec::ZERO);
        *sum = sum.add(Dec::parse(v["amount"].as_str().unwrap_or("0"))?)?;
    }
    let status = if unpriced > 0 && sums.is_empty() {
        "unavailable"
    } else if unpriced > 0 {
        "partial"
    } else {
        "available"
    };
    let zero = sums.is_empty() && unpriced == 0;
    let mut cost = json!({"by_currency":sums.into_iter().map(|(k,v)|(k,v.to_string())).collect::<BTreeMap<_,_>>(),
        "unpriced_requests":unpriced,"status":status,"reason":if unpriced>0 {json!("requests_unpriced")} else {Value::Null}});
    if zero { cost["amount"] = json!("0"); }
    Ok(cost)
}

pub(crate) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let view = crate::telemetry::accounting::fleet::coordinator_turns_metadata(project)?;
    let turns: Vec<&Value> = view["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["turns"].as_array().into_iter().flatten())
        .filter(|t| since.is_none_or(|s| t["started_unix_ms"].as_i64().is_some_and(|at| at >= s)))
        .collect();
    let mut out = BTreeMap::new();
    if turns.is_empty() {
        for id in 80..=87 {
            out.insert(format!("M{id}"), json!({"value":unavailable(view["reason"].as_str().unwrap_or("coordinator_turn_metadata_missing"))}));
        }
    } else {
        let requests: Vec<&Value> = turns
            .iter()
            .flat_map(|t| {
                t["request_samples"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(_, v)| v)
            })
            .collect();
        out.insert("M80".into(),json!({"value":distribution(requests.iter().filter_map(|r|r["context_tokens"].as_i64()).collect())}));
        let rewrites: Vec<&Value> = requests
            .iter()
            .copied()
            .filter(|r| r["rewrite"] == true)
            .collect();
        out.insert(
            "M81".into(),
            json!({"value":costs(rewrites.iter().flat_map(|r|r["valuations"].as_array().into_iter().flatten()))?,"rewrites":rewrites}),
        );
        let no_ops: Vec<&Value> = turns
            .iter()
            .copied()
            .filter(|t| t["no_tool_call"] == true)
            .collect();
        let mut triggers = BTreeMap::<String, Vec<&Value>>::new();
        for t in &turns {
            triggers
                .entry(t["trigger_class"].as_str().unwrap_or("other").into())
                .or_default()
                .push(t);
        }
        let turn_cost = |ts: &[&Value]| {
            costs(ts.iter().flat_map(|t| {
                t["request_samples"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .flat_map(|(_, r)| r["valuations"].as_array().into_iter().flatten())
            }))
        };
        let mut by_trigger = BTreeMap::new();
        for (k, ts) in triggers {
            by_trigger.insert(k, turn_cost(&ts)?);
        }
        out.insert("M82".into(),json!({"value":by_trigger,"no_tool_call_turns":no_ops.len(),"no_tool_call_cost":turn_cost(&no_ops)?}));
        if requests
            .iter()
            .flat_map(|r| r["valuations"].as_array().into_iter().flatten())
            .any(|v| v["status"] != "priced")
        {
            out.get_mut("M82").unwrap()["status"] = json!(if requests
                .iter()
                .flat_map(|r| r["valuations"].as_array().into_iter().flatten())
                .all(|v| v["status"] != "priced")
            {
                "unavailable"
            } else {
                "partial"
            });
            out.get_mut("M82").unwrap()["reason"] = json!("requests_unpriced");
        }
        let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
        let accepted = crate::telemetry::metrics::task_evidence(&db)?
            .iter()
            .filter(|(_, _, a)| *a)
            .count();
        let owners: Vec<i64> = turns
            .iter()
            .filter(|t| t["trigger_class"] == "owner_typed")
            .filter_map(|t| t["started_unix_ms"].as_i64())
            .collect();
        let owner_count = turns
            .iter()
            .filter(|t| t["trigger_class"] == "owner_typed")
            .count();
        let mut owners = owners;
        owners.sort_unstable();
        out.insert("M87".into(), json!({"value":ratio(owner_count,accepted)}));
        let questions: Vec<&Value> = turns
            .iter()
            .flat_map(|t| t["questions"].as_array().into_iter().flatten())
            .collect();
        let has_times: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='event_times')",
            [],
            |r| r.get(0),
        )?;
        if !has_times {
            for id in 83..=86 {
                out.insert(
                    format!("M{id}"),
                    json!({"value":unavailable("event_times_not_recorded")}),
                );
            }
        } else {
            // The first retained timed sequence fences history that was never timed.
            let epoch: i64 = db.query_row("SELECT coalesce(min(sequence),0) FROM event_times", [], |r| r.get(0))?;
            let historical_events: usize = db.query_row("SELECT count(*) FROM events e LEFT JOIN event_times t USING(sequence) WHERE e.sequence<?1 AND t.sequence IS NULL AND e.kind IN ('inbox.seen','inbox.done') AND e.entity LIKE 'worker-result-%'", [epoch], |r| r.get(0))?;
            let mut events = db.prepare("SELECT e.kind,e.entity,t.recorded_unix_ms FROM events e LEFT JOIN event_times t USING(sequence) WHERE e.kind IN ('inbox.seen','inbox.done') AND (t.sequence IS NOT NULL OR e.sequence>=?1) ORDER BY e.sequence")?
                .query_map([epoch],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<i64>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let notices = db.prepare("SELECT id,json_extract(payload,'$.created') FROM inbox_items i WHERE id LIKE 'worker-result-%' AND NOT EXISTS(SELECT 1 FROM events e LEFT JOIN event_times t USING(sequence) WHERE e.entity=i.id AND e.kind IN ('inbox.seen','inbox.done') AND e.sequence<?1 AND t.sequence IS NULL)")?
                .query_map([epoch], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            events.extend(notices.into_iter().map(|(id, created)| {
                let at = created.and_then(|s| s.parse::<jiff::Timestamp>().ok()).map(|t| t.as_millisecond());
                ("inbox.delivered".to_owned(), id, at)
            }));
            events.sort_by_key(|(kind, _, at)| (*at, kind != "inbox.delivered"));
            let owner_requests = db.prepare("SELECT o.id,min(t.recorded_unix_ms) FROM owner_requests o LEFT JOIN events e ON e.entity=o.id AND e.kind='owner.requested' LEFT JOIN event_times t USING(sequence) GROUP BY o.id")?
                .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            events.extend(
                owner_requests
                    .into_iter()
                    .map(|(id, time)| ("owner.requested".to_owned(), id, time)),
            );
            let mut intervals = db.prepare("SELECT a.id,min(CASE WHEN l.state='reserved' THEN l.unix_ms END),min(CASE WHEN l.state IN ('completed','failed','cancelled','lost') THEN l.unix_ms END),min(CASE WHEN l.state='running' THEN l.unix_ms END) FROM attempts a LEFT JOIN attempt_lifecycle l ON l.attempt_id=a.id GROUP BY a.id")?
                .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<i64>>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let historical_attempts = db.prepare("SELECT a.id FROM attempts a WHERE NOT EXISTS(SELECT 1 FROM attempt_lifecycle l WHERE l.attempt_id=a.id AND l.state='reserved') AND EXISTS(SELECT 1 FROM events e WHERE e.entity=a.id AND e.kind IN ('attempt.reserved','attempt.changed') AND e.revision=1 AND e.sequence<?1)")?
                .query_map([epoch], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
            intervals.retain(|(id, _, end, _)| !historical_attempts.contains(id) && !since.is_some_and(|s| end.is_some_and(|e| e < s)));
            let pending = |at: i64| {
                let mut unread = std::collections::BTreeSet::new();
                for (kind, id, time) in &events {
                    if time.is_some_and(|t| t <= at) && id.starts_with("worker-result-") {
                        if kind == "inbox.delivered" {
                            unread.insert(id);
                        } else {
                            unread.remove(id);
                        }
                    }
                }
                !unread.is_empty()
                    || intervals.iter().any(|(_, start, end, _)| {
                        start.is_some_and(|s| s <= at) && end.is_none_or(|e| e > at)
                    })
            };
            let missing = events
                .iter()
                .any(|(_, id, t)| id.starts_with("worker-result-") && t.is_none())
                || intervals.iter().any(|(_, s, _, _)| s.is_none())
                || owner_count != owners.len()
                || turns.iter().any(|t| t["ended_unix_ms"].is_null());
            out.insert("M83".into(),json!({"value":if missing {unavailable("historical_activity_times_missing")} else {ratio(owners.iter().filter(|at|pending(**at)).count(),owner_count)}}));
            let mut waits = Vec::new();
            let mut censored = 0;
            for t in &turns {
                if let Some(end) = t["ended_unix_ms"].as_i64()
                    && (t["ended_with_question"] == true || !pending(end))
                {
                    if let Some(next) = owners.get(owners.partition_point(|at| *at <= end)) {
                        waits.push(*next - end);
                    } else {
                        censored += 1;
                    }
                }
            }
            out.insert("M84".into(),json!({"value":if missing {unavailable("historical_activity_times_missing")} else {distribution(waits)},"open_censored":censored,"ask_user_question_answer_time_ms":distribution(questions.iter().filter_map(|q|q["answer_wait_ms"].as_i64()).collect())}));
            let mut days = BTreeMap::<i64, usize>::new();
            for at in events
                .iter()
                .filter(|(k, _, _)| k == "owner.requested")
                .filter_map(|(_, _, t)| *t)
                .chain(
                    questions
                        .iter()
                        .filter_map(|q| q["called_unix_ms"].as_i64()),
                )
            {
                if since.is_none_or(|s| at >= s) {
                    *days.entry(at.div_euclid(86_400_000)).or_default() += 1;
                }
            }
            let count = days.values().sum();
            out.insert("M85".into(),json!({"value":ratio(count,accepted),"per_utc_day":days,"prompts":count,"missing_event_times":events.iter().filter(|(k,_,t)|k=="owner.requested" && t.is_none()).count()}));
            let missing_prompt_times = events
                .iter()
                .filter(|(k, _, at)| k == "owner.requested" && at.is_none())
                .count()
                + questions
                    .iter()
                    .filter(|q| q["called_unix_ms"].is_null())
                    .count();
            if missing_prompt_times > 0 {
                let body = out.get_mut("M85").unwrap();
                body["status"] = json!("partial");
                body["reason"] = json!("permission_prompt_times_missing");
            }
            let first = turns
                .iter()
                .filter_map(|t| t["started_unix_ms"].as_i64())
                .min()
                .unwrap_or(0);
            let last = turns
                .iter()
                .filter_map(|t| t["ended_unix_ms"].as_i64())
                .max()
                .unwrap_or(first);
            let mut boundaries = vec![first];
            boundaries.extend(owners.iter().copied());
            boundaries.push(last);
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut notice_spans = Vec::new();
            let mut unread = BTreeMap::<String, i64>::new();
            for (kind, id, time) in &events {
                if !id.starts_with("worker-result-") {
                    continue;
                }
                if let Some(at) = time {
                    if kind == "inbox.delivered" {
                        unread.entry(id.clone()).or_insert(*at);
                    } else if let Some(start) = unread.remove(id) {
                        notice_spans.push((start, *at));
                    }
                }
            }
            notice_spans.extend(unread.into_values().map(|start| (start, last)));
            let mut stretches = Vec::new();
            for pair in boundaries.windows(2) {
                let mut spans: Vec<(i64, i64)> = intervals
                    .iter()
                    .filter_map(|(_, _, end, start)| {
                        start.map(|s| (s.max(pair[0]), end.unwrap_or(last).min(pair[1])))
                    })
                    .filter(|(a, b)| b > a)
                    .collect();
                spans.extend(
                    notice_spans
                        .iter()
                        .map(|(a, b)| ((*a).max(pair[0]), (*b).min(pair[1])))
                        .filter(|(a, b)| b > a),
                );
                spans.sort_unstable();
                let mut merged: Vec<(i64, i64)> = Vec::new();
                for (a, b) in spans {
                    if let Some(prev) = merged.last_mut()
                        && a <= prev.1
                    {
                        prev.1 = prev.1.max(b);
                    } else {
                        merged.push((a, b));
                    }
                }
                stretches.extend(merged.into_iter().map(|(a, b)| b - a));
            }
            let launches = intervals
                .iter()
                .filter(|(_, _, _, s)| s.is_some_and(|at| at >= first && at <= last))
                .count();
            out.insert("M86".into(),json!({"value":if missing {unavailable("historical_activity_times_missing")} else {distribution(stretches)},"launches_per_owner_turn":if missing {unavailable("historical_activity_times_missing")} else {ratio(launches,owner_count)},"observation_end_unix_ms":last}));
            for id in ["M83", "M84", "M86"] {
                let body = out.get_mut(id).unwrap();
                body["historical_censored"] = json!({"events":historical_events,"attempts":historical_attempts.len()});
            }
        }
    }
    for (id, b) in &mut out {
        b["definition"] = json!(format!("{id}.v1"));
        b["certification"] = json!("fixture");
    }
    Ok(out)
}
