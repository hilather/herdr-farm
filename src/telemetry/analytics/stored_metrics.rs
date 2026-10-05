//! MET-NOW-A: read-only projections of existing policy, review, quality and usage metadata.
use crate::telemetry::{
    accounting::{cost, graph, ledger},
    sidecar,
};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Published long-context price boundary; a future definition may configure it per model.
const LONG_CONTEXT_INPUT_TOKENS: i64 = 272_000;

fn unavailable(reason: &str) -> Value {
    json!({"status": "unavailable", "reason": reason})
}
fn ratio(n: i64, d: i64) -> Value {
    if d == 0 {
        json!({"value": null, "reason": "empty_denominator", "numerator": n, "denominator": d})
    } else {
        json!({"value": format!("{n}/{d}"), "numerator": n, "denominator": d})
    }
}
fn missing(reason: &str) -> Value {
    json!({"value": unavailable(reason)})
}
fn table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )?)
}
struct Attempt {
    profile: String,
    decided: Option<i64>,
}
fn attempts(db: &Connection) -> Result<BTreeMap<String, Attempt>> {
    Ok(db.prepare("SELECT a.id,json_extract(i.payload,'$.inputs.effective_profile.name'),d.decided_unix_ms
        FROM attempts a LEFT JOIN attempt_inputs i ON i.attempt_id=a.id LEFT JOIN dispatch_decisions d ON d.attempt_id=a.id")?
        .query_map([], |r| Ok((r.get(0)?, Attempt { profile: r.get::<_, Option<String>>(1)?.unwrap_or_else(|| "unknown".into()), decided: r.get(2)? })))?
        .collect::<rusqlite::Result<_>>()?)
}
fn profile<'a>(attempts: &'a BTreeMap<String, Attempt>, id: &str) -> &'a str {
    attempts.get(id).map_or("unknown", |a| a.profile.as_str())
}

pub(super) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let _snapshot = db.unchecked_transaction()?;
    let attempts = attempts(&db)?;
    let side = sidecar::read(project)?;
    let _side_snapshot = side
        .as_ref()
        .map(|db| db.unchecked_transaction())
        .transpose()?;
    let mut out = BTreeMap::new();
    out.insert("M51".into(), verification(&db, &attempts, since)?);
    let (density, findings) = density(&db, &attempts, since)?;
    out.insert("M52".into(), density);
    out.insert(
        "M53".into(),
        finding_cost(&db, side.as_deref(), since, findings)?,
    );
    out.insert("M54".into(), weakening(&db, side.as_deref(), since)?);
    out.extend(consumption(side.as_deref(), &attempts, since)?);
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

fn verification(
    db: &Connection,
    attempts: &BTreeMap<String, Attempt>,
    since: Option<i64>,
) -> Result<Value> {
    let accepted = db.prepare("SELECT s.submission_id,s.attempt_id FROM result_submissions s JOIN verified_results v USING(submission_id)
        GROUP BY s.submission_id HAVING ?1 IS NULL OR min(v.created_unix_ms)>=?1 ORDER BY s.submission_id")?
        .query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut policies = db.prepare("SELECT p.body,r.policy_digest FROM verified_results v JOIN verification_runs r USING(run_id)
        LEFT JOIN acceptance_policies p ON p.task_id=r.task_id AND p.contract_revision=r.contract_revision AND p.policy_id=r.policy_id
        WHERE v.submission_id=?1")?;
    let (mut command, mut presence, mut unknown) = (0, 0, 0);
    let mut by = BTreeMap::<String, [i64; 3]>::new();
    for (submission, attempt) in accepted {
        let rows = policies
            .query_map([submission], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let classes = rows
            .iter()
            .map(|(body, digest)| {
                let body = body.as_ref()?;
                if super::sha256(body.as_bytes()).strip_prefix("sha256:") != Some(digest.as_str()) {
                    return None;
                }
                crate::domain::verification_policy::ExecutionPolicy::parse(body.as_bytes())
                    .ok()
                    .map(|p| p.file_presence_only())
            })
            .collect::<Vec<_>>();
        let cell = by
            .entry(profile(attempts, &attempt).to_owned())
            .or_default();
        if classes.contains(&Some(false)) {
            command += 1;
            cell[0] += 1;
        } else if classes.is_empty() || classes.contains(&None) {
            unknown += 1;
            cell[2] += 1;
        } else {
            presence += 1;
            cell[1] += 1;
        }
    }
    let body = |c: [i64; 3]| {
        let mut b = ratio(c[0], c.iter().sum());
        b["file_presence_only"] = json!(c[1]);
        b["policy_unavailable"] = json!(c[2]);
        if c[2] > 0 {
            b["value"] = unavailable("acceptance_policy_unavailable");
        }
        b
    };
    let mut result = body([command, presence, unknown]);
    result["by_profile"] = json!(
        by.into_iter()
            .map(|(p, c)| (p, body(c)))
            .collect::<BTreeMap<_, _>>()
    );
    Ok(result)
}

/// F is exactly M21's validated, non-seeded roots at the current triage watermark.
fn density(
    db: &Connection,
    attempts: &BTreeMap<String, Attempt>,
    since: Option<i64>,
) -> Result<(Value, i64)> {
    let state = crate::store::fix_state(db, None)?;
    let reviewed = db
        .prepare(
            "SELECT DISTINCT o.submission_id,s.attempt_id FROM review_opportunities o
        JOIN review_sessions r USING(opportunity_id) JOIN review_completions c USING(session_id)
        JOIN result_submissions s ON s.submission_id=o.submission_id
        WHERE c.outcome='completed' AND (?1 IS NULL OR c.completed_unix_ms>=?1)",
        )?
        .query_map([since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let mut by = BTreeMap::<String, (i64, BTreeMap<String, i64>)>::new();
    for attempt in reviewed.values() {
        by.entry(profile(attempts, attempt).to_owned())
            .or_default()
            .0 += 1;
    }
    let mut count = 0;
    if let Some(state) = state {
        let claims: BTreeMap<_, _> = state
            .triage
            .submissions
            .iter()
            .flat_map(|s| s.claims.iter().map(move |c| (c.claim_id, (s, c))))
            .collect();
        let roots: BTreeMap<_, _> = state
            .triage
            .findings
            .iter()
            .map(|f| (f.finding_id.as_str(), f))
            .collect();
        let mut author = db.prepare("SELECT s.attempt_id FROM review_opportunities o JOIN result_submissions s USING(submission_id) WHERE o.opportunity_id=?1")?;
        for finding in state.findings.iter().filter(|f| {
            f.status == "validated"
                && !f.seeded_evaluation
                && since.is_none_or(|s| f.discovered_unix_ms.is_some_and(|at| at >= s))
        }) {
            count += 1;
            let claim = roots
                .get(finding.finding_id.as_str())
                .and_then(|g| g.discovery_claim)
                .and_then(|id| claims.get(&id));
            if let Some((submission, claim)) = claim {
                let attempt: String =
                    author.query_row([&submission.opportunity_id], |r| r.get(0))?;
                *by.entry(profile(attempts, &attempt).to_owned())
                    .or_default()
                    .1
                    .entry(claim.severity.clone().unwrap_or_else(|| "unknown".into()))
                    .or_default() += 1;
            }
        }
    }
    let mut body = if reviewed.is_empty() {
        missing("no_reviews")
    } else {
        ratio(count, reviewed.len() as i64)
    };
    body["validated_unique_findings"] = json!(count);
    body["reviewed_submissions"] = json!(reviewed.len());
    body["by_profile"] = json!(
        by.into_iter()
            .map(|(p, (d, severities))| {
                let mut b = ratio(severities.values().sum(), d);
                b["by_severity"] = json!(
                    ["critical", "high", "medium", "low", "informational"]
                        .into_iter()
                        .map(|severity| (
                            severity,
                            ratio(*severities.get(severity).unwrap_or(&0), d)
                        ))
                        .collect::<BTreeMap<_, _>>()
                );
                (p, b)
            })
            .collect::<BTreeMap<_, _>>()
    );
    Ok((body, count))
}

fn finding_cost(
    db: &Connection,
    side: Option<&Connection>,
    since: Option<i64>,
    findings: i64,
) -> Result<Value> {
    let review_attempts: BTreeSet<String> = db.prepare("SELECT a.id FROM attempts a LEFT JOIN dispatch_decisions d ON d.attempt_id=a.id
        WHERE EXISTS(SELECT 1 FROM review_briefs b WHERE b.task_id=a.task_id) AND (?1 IS NULL OR d.decided_unix_ms>=?1)")?
        .query_map([since], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    if review_attempts.is_empty() {
        return Ok(missing("no_reviews"));
    }
    let Some(side) = side else {
        return Ok(missing("collection_not_run"));
    };
    let costs = cost::cost(side, None, None)?;
    if costs["status"] == "unavailable" {
        return Ok(json!({"value": costs}));
    }
    let entries: Vec<&Value> = costs["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            s["exclusion_reason"].is_null()
                && s["attempt_id"]
                    .as_str()
                    .is_some_and(|a| review_attempts.contains(a))
        })
        .flat_map(|s| s["entries"].as_array().into_iter().flatten())
        .collect();
    let represented: BTreeSet<&str> = costs["attempts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["attempt_id"].as_str())
        .filter(|a| review_attempts.contains(*a))
        .collect();
    let (_, coverage) = cost::summarize(&entries)?;
    let missing_attempts = review_attempts.len() - represented.len();
    let incomplete = missing_attempts > 0 || coverage["priced"] != coverage["entries"];
    let mut currencies = BTreeMap::<String, Vec<&Value>>::new();
    for entry in &entries {
        if let Some(currency) = entry["valuation"]["currency"]
            .as_str()
            .filter(|_| entry["valuation"]["status"] == "priced")
        {
            currencies
                .entry(currency.to_owned())
                .or_default()
                .push(entry);
        }
    }
    let mut by_currency = BTreeMap::new();
    for (currency, entries) in currencies {
        let (estimate, _) = cost::summarize(&entries)?;
        let amount = estimate["amount"].as_str().unwrap_or("0");
        let mut b = json!({"numerator": amount, "denominator": findings, "value": format!("{amount}/{findings}")});
        if findings == 0 {
            b["value"] = Value::Null;
            b["reason"] = json!("empty_denominator");
        } else if incomplete {
            b["value"] = json!({"status": "partial", "reason": "review_cost_incomplete", "priced_amount": amount, "denominator": findings});
        }
        by_currency.insert(currency, b);
    }
    let value = if entries.is_empty() {
        unavailable("no_usage")
    } else if by_currency.is_empty() {
        unavailable("no_priced_entries")
    } else if findings == 0 {
        Value::Null
    } else if incomplete {
        json!({"status": "partial", "reason": "review_cost_incomplete"})
    } else {
        json!("per_currency")
    };
    let mut body = json!({"value": value, "by_currency": by_currency, "validated_unique_findings": findings,
        "coverage": coverage, "attempts_without_usage": missing_attempts, "basis": "published_rate_estimate", "rate_cards": "fixture_only"});
    if findings == 0 {
        body["reason"] = json!("empty_denominator");
    }
    Ok(body)
}

fn weakening(db: &Connection, side: Option<&Connection>, since: Option<i64>) -> Result<Value> {
    let submissions: Vec<String> = db
        .prepare(
            "SELECT submission_id FROM result_submissions WHERE ?1 IS NULL OR created_unix_ms>=?1",
        )?
        .query_map([since], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let Some(side) = side else {
        return Ok(missing("collection_not_run"));
    };
    if !table(side, "proxy_signals")? {
        return Ok(missing("collection_not_run"));
    }
    let signals: BTreeMap<String, (String, Option<String>)> = side.prepare("SELECT submission_id,weakening,weakening_reason FROM proxy_signals WHERE weakening_rule='tests-net-removal.v1'")?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<rusqlite::Result<_>>()?;
    let (mut flagged, mut observed) = (0, 0);
    let mut excluded = BTreeMap::<String, i64>::new();
    for submission in submissions {
        match signals.get(&submission) {
            Some((s, _)) if s == "flagged" || s == "clear" => {
                observed += 1;
                if s == "flagged" {
                    flagged += 1;
                }
            }
            Some((_, reason)) => {
                *excluded
                    .entry(
                        reason
                            .clone()
                            .unwrap_or_else(|| "weakening_unavailable".into()),
                    )
                    .or_default() += 1
            }
            None => *excluded.entry("not_collected".into()).or_default() += 1,
        }
    }
    let mut body = ratio(flagged, observed);
    body["proxy"] = json!(true);
    body["source_trust"] = json!("proxy_observed");
    body["weakening_rule"] = json!("tests-net-removal.v1");
    body["excluded"] = json!(excluded);
    if observed == 0 && !excluded.is_empty() {
        body["value"] = unavailable("no_observed_submissions");
    } else if !excluded.is_empty() {
        body["coverage"] = json!({"state": "partial", "observed": observed, "missing": excluded.values().sum::<i64>()});
    }
    Ok(body)
}

#[derive(Default)]
struct Tokens {
    input: i64,
    output: i64,
    reasoning: i64,
    cache: i64,
    total: i64,
    child: i64,
    long: i64,
    records: i64,
    child_records: i64,
    reasoning_missing: bool,
    firsts: BTreeMap<String, (i64, String, i64, i64)>,
    excluded: BTreeMap<String, i64>,
}
fn consumption(
    side: Option<&Connection>,
    attempts: &BTreeMap<String, Attempt>,
    since: Option<i64>,
) -> Result<BTreeMap<String, Value>> {
    let absent = |reason| {
        (55..=58)
            .map(|n| (format!("M{n}"), missing(reason)))
            .collect()
    };
    let Some(db) = side else {
        return Ok(absent("collection_not_run"));
    };
    if !crate::telemetry::accounting::cache::current(db)? {
        return Ok(absent("accounting_sync_required"));
    }
    let membership = graph::attempt_membership(db)?;
    if membership.is_empty() {
        return Ok(absent("no_usage"));
    }
    let roles: BTreeMap<String, String> = db
        .prepare("SELECT DISTINCT session_id,role FROM session_graph_nodes")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let times: BTreeMap<String, i64> = db.prepare("SELECT session_id,coalesce(min(session_unix_ms),0) FROM rollout_sources GROUP BY session_id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let index = sidecar::child_index(db)?;
    let mut by = BTreeMap::<String, Tokens>::new();
    let mut eligible = BTreeSet::new();
    for (id, a) in attempts
        .iter()
        .filter(|(_, a)| since.is_none_or(|s| a.decided.is_some_and(|at| at >= s)))
    {
        let tally = by.entry(a.profile.clone()).or_default();
        let usage = sidecar::attempt_usage_with(db, id, &index)?;
        if usage["status"] == "unavailable" || usage["records"] == 0 {
            *tally
                .excluded
                .entry(usage["reason"].as_str().unwrap_or("no_usage").to_owned())
                .or_default() += 1;
        } else {
            eligible.insert(id.as_str());
        }
    }
    let mut reasoning = BTreeMap::new();
    for entry in ledger::derive(db)?
        .into_iter()
        .filter(|e| e.basis == "delta")
    {
        let Some((Some(owner), reason)) = membership.get(&entry.session) else {
            continue;
        };
        if !eligible.contains(owner.as_str()) {
            continue;
        }
        let tally = by.entry(profile(attempts, owner).to_owned()).or_default();
        if let Some(reason) = reason {
            *tally.excluded.entry(reason.clone()).or_default() += 1;
            continue;
        }
        if !entry.counted() {
            *tally
                .excluded
                .entry("records_not_accepted".into())
                .or_default() += 1;
            continue;
        }
        let Some(n) = entry.normalized else { continue };
        tally.records += 1;
        tally.input += n[0];
        tally.cache += n[1];
        tally.output += n[4];
        tally.reasoning += n[5];
        tally.total += n[6];
        if let std::collections::btree_map::Entry::Vacant(slot) =
            reasoning.entry(entry.session.clone())
        {
            slot.insert(sidecar::reasoning_missing(db, &entry.session)?);
        }
        tally.reasoning_missing |= reasoning[&entry.session];
        if n[0] > LONG_CONTEXT_INPUT_TOKENS {
            tally.long += n[0];
        }
        if roles.get(&entry.session).is_some_and(|r| r != "primary") {
            tally.child += n[6];
            tally.child_records += 1;
        } else {
            let first = (
                times.get(&entry.session).copied().unwrap_or(0),
                entry.session.clone(),
                entry.position,
                n[0],
            );
            let slot = tally
                .firsts
                .entry(owner.clone())
                .or_insert_with(|| first.clone());
            if first < *slot {
                *slot = first;
            }
        }
    }
    let mut out = BTreeMap::new();
    for n in 55..=58 {
        let mut cells = BTreeMap::new();
        for (p, t) in &by {
            let mut cell = if t.records == 0 {
                missing("no_usage")
            } else {
                match n {
                    55 if t.child_records == 0 => missing("no_children"),
                    55 => ratio(t.child, t.total),
                    56 => {
                        let r = if t.reasoning_missing {
                            missing("reasoning_tokens_not_reported")
                        } else {
                            ratio(t.reasoning, t.output)
                        };
                        json!({"value": "per_component", "reasoning_share": r, "cache_share": ratio(t.cache, t.input)})
                    }
                    57 => ratio(t.long, t.input),
                    _ => {
                        let mut samples = t.firsts.values().map(|s| s.3).collect::<Vec<_>>();
                        samples.sort_unstable();
                        if samples.is_empty() {
                            missing("no_usage")
                        } else {
                            let len = samples.len();
                            let value = if len % 2 == 1 {
                                samples[len / 2].to_string()
                            } else {
                                let sum =
                                    i128::from(samples[len / 2 - 1]) + i128::from(samples[len / 2]);
                                if sum % 2 == 0 {
                                    (sum / 2).to_string()
                                } else {
                                    format!("{}.5", sum / 2)
                                }
                            };
                            json!({"value": value, "samples": len})
                        }
                    }
                }
            };
            cell["excluded"] = json!(t.excluded);
            cell["coverage"] = json!({"state": if t.excluded.is_empty() { "complete" } else { "partial" }, "records": t.records});
            cells.insert(p, cell);
        }
        let value = if cells
            .values()
            .all(|c| c["value"]["status"] == "unavailable")
        {
            unavailable(if by.values().all(|t| t.records == 0) {
                "no_usage"
            } else {
                "no_children"
            })
        } else {
            json!("per_profile")
        };
        let mut body = json!({"value": value, "by_profile": cells});
        if n == 57 {
            body["default_model_threshold_input_tokens"] = json!(LONG_CONTEXT_INPUT_TOKENS);
        }
        out.insert(format!("M{n}"), body);
    }
    Ok(out)
}
