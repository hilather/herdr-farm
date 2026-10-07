//! MET-WORKER-1: metadata-only projections, with no retained state.
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn unavailable(reason: &str) -> Value {
    json!({"status":"unavailable","reason":reason})
}
fn share(n: usize, d: usize) -> Value {
    if d == 0 {
        json!({"value":null,"reason":"empty_denominator","numerator":n,"denominator":d})
    } else {
        json!({"value":format!("{n}/{d}"),"numerator":n,"denominator":d})
    }
}
fn distribution(mut samples: Vec<f64>, reason: &str, tail: usize) -> Value {
    samples.sort_by(f64::total_cmp);
    if samples.is_empty() {
        return unavailable(reason);
    }
    let rank = |p: usize| samples[(samples.len() * p).div_ceil(100) - 1].to_string();
    json!({"samples":samples.len(),"p50":rank(50),format!("p{tail}"):rank(tail)})
}
fn workers(records: &[&Value], id: &str) -> Value {
    let observed: Vec<_> = records
        .iter()
        .filter(|a| a["session"]["status"] == "observed")
        .copied()
        .collect();
    let never_running = records.iter().filter(|a| a["session"]["end_state"] == "never_running").count();
    let eligible = records.len() - never_running;
    let mut body = match id {
        "M70" => {
            let mut states = BTreeMap::from([
                ("submitted", 0),
                ("never_running", 0),
                ("ended_without_submission", 0),
                ("stopped", 0),
                ("timed_out", 0),
                ("unknown", 0),
            ]);
            for a in records {
                *states
                    .entry(a["session"]["end_state"].as_str().unwrap_or("unknown"))
                    .or_default() += 1;
            }
            json!({"value":states})
        }
        "M71" => {
            let samples: Vec<_> = observed
                .iter()
                .filter_map(|a| a["session"]["lingering_ms"].as_i64())
                .collect();
            json!({"value":distribution(samples.iter().map(|n| *n as f64).collect(),"session_end_or_terminal_time_missing",95),"over_10_min":samples.iter().filter(|n| **n>600_000).count(),"missing_samples":eligible-samples.len()})
        }
        "M72" => {
            let commands: usize = observed
                .iter()
                .filter_map(|a| a["session"]["commands"].as_u64())
                .sum::<u64>() as usize;
            let failed = observed
                .iter()
                .filter_map(|a| a["session"]["failed_commands"].as_u64())
                .sum::<u64>() as usize;
            let unknown = observed
                .iter()
                .filter_map(|a| a["session"]["commands_exit_unknown"].as_u64())
                .sum::<u64>();
            let mut classes = BTreeMap::<String, usize>::new();
            for a in &observed {
                for (k, v) in a["session"]["failed_commands_by_class"]
                    .as_object()
                    .into_iter()
                    .flatten()
                {
                    *classes.entry(k.clone()).or_default() += v.as_u64().unwrap_or(0) as usize;
                }
            }
            let mut b = share(failed, commands);
            b["commands"] = json!(commands);
            b["failed_commands"] = json!(failed);
            b["exit_unknown"] = json!(unknown);
            b["by_class"] = json!(
                classes
                    .into_iter()
                    .map(|(k, n)| (k, share(n, commands)))
                    .collect::<BTreeMap<_, _>>()
            );
            if unknown > 0 {
                b["status"] = json!("partial");
                b["reason"] = json!("exit_code_unknown");
            }
            b
        }
        "M75" => {
            let per_attempt: BTreeMap<_, _> = observed
                .iter()
                .map(|a| {
                    (
                        a["attempt_id"].as_str().unwrap_or_default(),
                        a["session"]["unanswered_user_input_requests"].clone(),
                    )
                })
                .collect();
            let n = per_attempt.values().filter_map(Value::as_u64).sum::<u64>();
            let mut b = share(n as usize, observed.len());
            b["unanswered"] = json!(n);
            b["per_attempt"] = json!(per_attempt);
            b
        }
        "M76" => {
            let samples: Vec<f64> = observed
                .iter()
                .filter_map(|a| {
                    a["session"]["max_context_window_fill"]
                        .as_str()?
                        .parse()
                        .ok()
                })
                .collect();
            json!({"value":distribution(samples.clone(),"context_window_or_input_tokens_missing",95),"over_80_percent":share(samples.iter().filter(|n|**n>0.8).count(),samples.len()),"missing_samples":eligible-samples.len()})
        }
        _ => unreachable!(),
    };
    if body["missing_samples"].as_u64().is_some_and(|n| n > 0) {
        body["status"] = json!(if body["value"]["status"] == "unavailable" {
            "unavailable"
        } else {
            "partial"
        });
        body["reason"] = json!(if id == "M71" {
            "session_end_or_terminal_time_missing"
        } else {
            "context_window_or_input_tokens_missing"
        });
    }
    body["coverage"] = json!({"observed_attempts":observed.len(),"unavailable_attempts":eligible-observed.len(),"never_running_attempts":never_running});
    if id != "M70" && observed.is_empty() && eligible > 0 {
        body["value"] = unavailable("session_metadata_not_collected");
    } else if observed.len() < eligible {
        body["status"] = json!("partial");
        body["reason"] = json!("session_metadata_not_collected");
    }
    body
}

pub(super) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let attempts = crate::telemetry::outcome::attempts(project)?;
    let records: Vec<_> = attempts["attempts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| since.is_none_or(|s| a["reserved_unix_ms"].as_i64().is_some_and(|at| at >= s)))
        .collect();
    let mut profiles = BTreeMap::<&str, Vec<&Value>>::new();
    for a in &records {
        profiles
            .entry(a["profile"].as_str().unwrap_or("unknown"))
            .or_default()
            .push(a);
    }
    let mut out = BTreeMap::new();
    for id in ["M70", "M71", "M72", "M75", "M76"] {
        let mut b = workers(&records, id);
        b["by_profile"] = json!(
            profiles
                .iter()
                .map(|(p, r)| (*p, workers(r, id)))
                .collect::<BTreeMap<_, _>>()
        );
        out.insert(id.into(), b);
    }
    let side = crate::telemetry::sidecar::read(project)?;
    let exists = side
        .as_ref()
        .map(|db| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cli_invocations')",
                [],
                |r| r.get::<_, bool>(0),
            )
        })
        .transpose()?
        .unwrap_or(false);
    let rows = if exists {
        side.as_ref().unwrap().prepare("SELECT caller,command_path,outcome,recorded_unix_ms FROM cli_invocations WHERE recorded_unix_ms>=?1 ORDER BY recorded_unix_ms")?.query_map([since.unwrap_or(0).max(crate::telemetry::maintenance::store::now()-crate::telemetry::accounting::cli_invocations::MAX_AGE_MS)],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    for (id, help) in [("M73", true), ("M74", false)] {
        let summarize = |selected: Vec<&(String, String, String, i64)>| {
            share(
                selected
                    .iter()
                    .filter(|r| {
                        if help {
                            r.2 == "help"
                        } else {
                            r.2 == "error" || r.2 == "usage_error"
                        }
                    })
                    .count(),
                selected.len(),
            )
        };
        let mut b = summarize(rows.iter().collect());
        b["by_caller"] = json!(
            ["worker", "coordinator", "operator", "plugin", "ticker"]
                .into_iter()
                .map(|c| (c, summarize(rows.iter().filter(|r| r.0 == c).collect())))
                .collect::<BTreeMap<_, _>>()
        );
        if !help {
            let mut paths = BTreeMap::<&str, Vec<_>>::new();
            for row in &rows {
                paths.entry(&row.1).or_default().push(row);
            }
            let mut top: Vec<_> = paths
                .into_iter()
                .map(|(p, r)| {
                    let errors = r
                        .iter()
                        .filter(|r| r.2 == "error" || r.2 == "usage_error")
                        .count();
                    json!({"command_path":p,"errors":errors,"share":summarize(r)})
                })
                .collect();
            top.sort_by(|a, b| {
                b["errors"]
                    .as_u64()
                    .cmp(&a["errors"].as_u64())
                    .then_with(|| a["command_path"].as_str().cmp(&b["command_path"].as_str()))
            });
            top.truncate(10);
            b["top_command_paths"] = json!(top);
        }
        if rows.is_empty() {
            b["value"] = unavailable(if !exists {
                "cli_invocations_not_collected"
            } else {
                "no_cli_invocations"
            });
        }
        out.insert(id.into(), b);
    }
    let canonical = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    // Invocation rows are time ordered. Binary search avoids scanning the
    // retained CLI history once for every notice in a large canonical log.
    let coordinator_times: Vec<i64> = rows
        .iter()
        .filter(|r| r.0 == "coordinator")
        .map(|r| r.3)
        .collect();
    let reaction = if coordinator_times.is_empty() {
        unavailable("coordinator_cli_invocations_missing")
    } else {
        let notices=canonical.prepare("SELECT json_extract(payload,'$.created') FROM inbox_items WHERE id LIKE 'worker-result-%'")?.query_map([],|r|r.get::<_,Option<String>>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut samples = Vec::new();
        let mut missing = 0;
        for created in notices {
            let at = created.and_then(|s| s.parse::<jiff::Timestamp>().ok()).map(|t| t.as_millisecond());
            if let Some(at) = at {
                if since.is_some_and(|s| at < s) {
                    continue;
                }
                if let Some(next) =
                    coordinator_times.get(coordinator_times.partition_point(|next| *next < at))
                {
                    samples.push((*next - at) as f64);
                } else {
                    missing += 1;
                }
            } else {
                missing += 1;
            }
        }
        let has_samples = !samples.is_empty();
        let mut b = distribution(samples, "worker_result_delivery_times_missing", 90);
        b["unmatched_notices"] = json!(missing);
        if missing > 0 {
            b["status"] = json!(if has_samples {
                "partial"
            } else {
                "unavailable"
            });
            b["reason"] = json!("notice_time_or_next_cli_missing");
        }
        b
    };
    out.insert("M77".into(), json!({"value":reaction}));
    for (id, body) in &mut out {
        body["definition"] = json!(format!("{id}.v1"));
        body["name"] = json!(
            super::registry::METRICS
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.name)
        );
    }
    Ok(out)
}
