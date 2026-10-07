//! MET-REWORK-1: metadata-only work-item projections; no inferred lineage.
use super::cost::Dec;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn missing(reason: &str) -> Value {
    json!({"value":{"status":"unavailable","reason":reason}})
}
fn distribution(mut samples: Vec<i64>) -> Value {
    samples.sort_unstable();
    let n = samples.len();
    if n == 0 {
        return missing("no_samples");
    }
    let median = if n % 2 == 1 {
        json!(samples[n / 2])
    } else {
        let sum = i128::from(samples[n / 2 - 1]) + i128::from(samples[n / 2]);
        if sum % 2 == 0 {
            json!((sum / 2) as i64)
        } else {
            json!(format!("{}.5", sum / 2))
        }
    };
    json!({"value":{"median":median,"p90":samples[(9*n).div_ceil(10)-1],"max":samples[n-1]},"samples":n})
}

// Each required independent verification policy needs its own immutable receipt.
// The candidate time and acceptance completion time have separate meanings.
fn green(db: &Connection, work: &str) -> Result<Option<(i64, i64)>> {
    // Earliest receipt per policy, then the last required policy's completion.
    // Later reruns must not move the first green time.
    let rows = db.prepare("SELECT s.submission_id,s.created_unix_ms,p.policy_id,p.body,r.policy_digest,v.created_unix_ms
        FROM result_submissions s JOIN task_lineage l ON l.task_id=s.task_id
        JOIN acceptance_policies p ON p.task_id=s.task_id AND p.contract_revision=s.contract_revision
        LEFT JOIN verification_runs r ON r.submission_id=s.submission_id AND r.policy_id=p.policy_id AND r.state='accepted'
        LEFT JOIN verified_results v ON v.run_id=r.run_id AND v.submission_id=s.submission_id AND v.policy_digest=r.policy_digest
        WHERE l.work_item=?1 AND l.role IN ('build','fix') ORDER BY s.submission_id,p.policy_id")?
        .query_map([work], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<i64>>(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut candidates = BTreeMap::<String, (i64, BTreeMap<String, Option<i64>>)>::new();
    for (id, created, policy, body, digest, time) in rows {
        let expected = format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(body.as_bytes())
        );
        let policies = &mut candidates.entry(id).or_insert((created, BTreeMap::new())).1;
        let first = policies.entry(policy).or_default();
        if digest.as_ref() == Some(&expected)
            && let Some(time) = time
        {
            *first = Some(first.map_or(time, |prior| prior.min(time)));
        }
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(id, (created, policies))| {
            let times = policies.values().copied().collect::<Option<Vec<_>>>()?;
            Some((times.into_iter().max()?, created, id))
        })
        .min()
        .map(|(accepted, created, _)| (created, accepted)))
}

#[derive(Default)]
struct Spend {
    currencies: BTreeMap<String, (Dec, Dec)>,
    missing: usize,
}
impl Spend {
    fn add(&mut self, estimate: &Value, coverage: &Value, selected: bool) -> Result<()> {
        if coverage["entries"].as_u64().unwrap_or(0) == 0
            || coverage["priced"] != coverage["entries"]
        {
            self.missing += 1;
        }
        let mut amounts = BTreeMap::new();
        if let (Some(c), Some(a)) = (
            estimate["currency"].as_str(),
            estimate["amount"]
                .as_str()
                .or_else(|| estimate["priced_amount"].as_str()),
        ) {
            amounts.insert(c, a);
        }
        for (c, a) in estimate["priced_by_currency"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if let Some(a) = a.as_str() {
                amounts.insert(c, a);
            }
        }
        for (c, a) in amounts {
            let amount = Dec::parse(a)?;
            let sums = self
                .currencies
                .entry(c.into())
                .or_insert((Dec::ZERO, Dec::ZERO));
            sums.1 = sums.1.add(amount)?;
            if selected {
                sums.0 = sums.0.add(amount)?;
            }
        }
        Ok(())
    }
    fn body(&self) -> Value {
        let cells: BTreeMap<_,_> = self.currencies.iter().map(|(c,(n,d))| {
            let mut body = json!({"value":format!("{n}/{d}"),"numerator":n.to_string(),"denominator":d.to_string()});
            if *d == Dec::ZERO { body["value"] = Value::Null; body["reason"] = json!("empty_denominator"); }
            else if self.missing > 0 { body["value"] = json!({"status":"partial","reason":"work_item_cost_incomplete","numerator":n.to_string(),"priced_amount":n.to_string(),"denominator":d.to_string()}); }
            (c.clone(),body)
        }).collect();
        let mut body = if cells.is_empty() {
            missing("cost_not_observed")
        } else if self.missing > 0 {
            json!({"value":{"status":"partial","reason":"work_item_cost_incomplete"}})
        } else {
            json!({"value":"per_currency"})
        };
        body["by_currency"] = json!(cells);
        body["attempts_without_complete_cost"] = json!(self.missing);
        body["basis"] = json!("published_rate_estimate");
        body
    }
}

pub(crate) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let _snapshot = db.unchecked_transaction()?;
    let history = super::work_items::read_snapshot(project, &db)?;
    let mut out: BTreeMap<String, Value> = (65..=69)
        .map(|n| (format!("M{n}"), missing("lineage_not_recorded")))
        .collect();
    let Some(items) = history["work_items"].as_array() else {
        let count: i64 = db.query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))?;
        for body in out.values_mut() {
            body["excluded"] = json!({"lineage_not_recorded":count});
        }
        return Ok(out);
    };
    let unbound = history["tasks_without_lineage"].as_i64().unwrap_or(0);
    if items.is_empty() {
        for body in out.values_mut() {
            body["excluded"] = json!({"lineage_not_recorded":unbound});
        }
        return Ok(out);
    }
    let (mut rounds, mut leads) = (Vec::new(), Vec::new());
    let mut rounds_by = BTreeMap::<String, Vec<i64>>::new();
    let (mut rework, mut waste) = (Spend::default(), Spend::default());
    let mut rework_by = BTreeMap::<String, Spend>::new();
    let mut accepted = BTreeMap::new();
    let mut excluded = BTreeMap::<String, i64>::from([("lineage_not_recorded".into(), unbound)]);
    let (mut not_green, mut censored) = (0, 0);
    for item in items {
        let work = item["work_item"].as_str().unwrap_or_default();
        let Some(first) = item["first_launch_unix_ms"].as_i64() else {
            *excluded.entry("launch_time_unknown".into()).or_default() += 1;
            continue;
        };
        if since.is_some_and(|s| first < s) {
            *excluded.entry("outside_window".into()).or_default() += 1;
            continue;
        }
        let attempts: Vec<&Value> = item["attempts"].as_array().into_iter().flatten().collect();
        let roles: BTreeMap<&str, &str> = item["tasks_by_role"]
            .as_object()
            .into_iter()
            .flatten()
            .flat_map(|(role, tasks)| {
                tasks
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(move |t| Some((t.as_str()?, role.as_str())))
            })
            .collect();
        let launches: BTreeMap<&str, i64> = roles
            .keys()
            .filter_map(|task| {
                attempts
                    .iter()
                    .filter(|a| a["task_id"] == *task)
                    .filter_map(|a| a["launch_unix_ms"].as_i64())
                    .min()
                    .map(|at| (*task, at))
            })
            .collect();
        let profile: String = db.query_row("SELECT json_extract(i.payload,'$.inputs.effective_profile.name') FROM task_lineage l JOIN attempts a ON a.task_id=l.task_id
            JOIN attempt_lifecycle m ON m.attempt_id=a.id AND m.state IN ('launching','running') LEFT JOIN attempt_inputs i ON i.attempt_id=a.id
            WHERE l.work_item=?1 AND l.role='build' ORDER BY m.unix_ms,a.id LIMIT 1",[work],|r| r.get::<_,Option<String>>(0)).optional()?.flatten().unwrap_or_else(|| "unknown".into());
        if let Some((candidate, at)) = green(&db, work)? {
            accepted.insert(work.to_owned(), at);
            let count = launches
                .iter()
                .filter(|(task, start)| roles.get(*task) == Some(&"fix") && **start <= candidate)
                .count() as i64;
            rounds.push(count);
            rounds_by.entry(profile.clone()).or_default().push(count);
        } else {
            not_green += 1;
        }
        let integration: Option<i64> = db.query_row("SELECT min(c.created_unix_ms) FROM task_lineage l JOIN result_submissions s ON s.task_id=l.task_id
            JOIN verified_results v USING(submission_id) JOIN integration_operations o ON o.verified_result_id=v.result_id JOIN integrated_commits c USING(operation_id)
            WHERE l.work_item=?1 AND l.role IN ('build','fix')",[work],|r| r.get(0))?;
        let end = integration.or_else(|| {
            (item["censored"] == false)
                .then(|| item["last_terminal_unix_ms"].as_i64())
                .flatten()
        });
        if let Some(end) = end.filter(|end| *end >= first) {
            leads.push(end - first);
        } else {
            censored += 1;
        }
        let superseded: BTreeSet<&str> = item["superseded_tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for a in &attempts {
            if a["launch_unix_ms"].is_null() {
                continue;
            }
            let task = a["task_id"].as_str().unwrap_or_default();
            let role = roles.get(task).copied().unwrap_or("other");
            let rer = matches!(role, "review" | "skeptic")
                && launches.get(task).is_some_and(|at| {
                    attempts.iter().any(|prior| {
                        let prior_task = prior["task_id"].as_str().unwrap_or_default();
                        prior_task != task
                            && roles.get(prior_task) == Some(&role)
                            && prior["terminal_unix_ms"]
                                .as_i64()
                                .is_some_and(|end| end <= *at)
                    })
                });
            let selected = matches!(role, "fix" | "recheck") || rer;
            rework.add(&a["cost_estimate"], &a["cost_coverage"], selected)?;
            let attempt_profile: Option<String> = db.query_row("SELECT json_extract(payload,'$.inputs.effective_profile.name') FROM attempt_inputs WHERE attempt_id=?1",[a["attempt_id"].as_str().unwrap_or_default()],|r|r.get(0)).optional()?.flatten();
            rework_by
                .entry(attempt_profile.unwrap_or_else(|| "unknown".into()))
                .or_default()
                .add(&a["cost_estimate"], &a["cost_coverage"], selected)?;
            waste.add(
                &a["cost_estimate"],
                &a["cost_coverage"],
                matches!(a["state"].as_str(), Some("failed" | "cancelled"))
                    || superseded.contains(task),
            )?;
        }
    }
    let mut escaped = 0;
    if let Some(state) = crate::store::fix_state(&db, None)? {
        for f in state
            .findings
            .iter()
            .filter(|f| f.status == "validated" && !f.seeded_evaluation)
        {
            let claim = state
                .triage
                .findings
                .iter()
                .find(|g| g.finding_id == f.finding_id)
                .and_then(|g| g.discovery_claim);
            let submission = claim.and_then(|id| {
                state
                    .triage
                    .submissions
                    .iter()
                    .find(|s| s.claims.iter().any(|c| c.claim_id == id))
            });
            if let Some(s) = submission {
                let work: Option<String> = db.query_row("SELECT l.work_item FROM review_opportunities o JOIN task_lineage l ON l.task_id=o.task_id WHERE o.opportunity_id=?1",[&s.opportunity_id],|r|r.get(0)).optional()?;
                if work
                    .as_ref()
                    .and_then(|w| accepted.get(w))
                    .zip(f.discovered_unix_ms)
                    .is_some_and(|(at, discovered)| {
                        discovered > *at && since.is_none_or(|s| discovered >= s)
                    })
                {
                    escaped += 1;
                }
            }
        }
    }
    let mut m65 = distribution(rounds);
    m65["by_profile"] = json!(
        rounds_by
            .into_iter()
            .map(|(p, v)| (p, distribution(v)))
            .collect::<BTreeMap<_, _>>()
    );
    m65["not_yet_green"] = json!(not_green);
    out.insert("M65".into(), m65);
    let mut m66 = rework.body();
    m66["by_profile"] = json!(
        rework_by
            .into_iter()
            .map(|(p, s)| (p, s.body()))
            .collect::<BTreeMap<_, _>>()
    );
    out.insert("M66".into(), m66);
    out.insert("M67".into(),json!({"value":if accepted.is_empty() { Value::Null } else { json!(format!("{escaped}/{}",accepted.len())) },"numerator":escaped,"denominator":accepted.len(),"count":escaped}));
    if accepted.is_empty() {
        out.get_mut("M67").unwrap()["reason"] = json!("empty_denominator");
    }
    let mut m68 = distribution(leads);
    m68["censored"] = json!(censored);
    out.insert("M68".into(), m68);
    out.insert("M69".into(), waste.body());
    for body in out.values_mut() {
        body["excluded"] = json!(excluded);
    }
    out.get_mut("M65").unwrap()["excluded"]["not_yet_green"] = json!(not_green);
    out.get_mut("M68").unwrap()["excluded"]["open_or_terminal_time_unknown"] = json!(censored);
    Ok(out)
}
