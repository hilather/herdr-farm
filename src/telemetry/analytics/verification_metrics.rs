//! MET-VERIFY-1: retained metadata only, with cohort windows before run selection.
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
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
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
fn share(n: i64, d: i64) -> Value {
    json!({"value":ratio(n,d),"numerator":n,"denominator":d})
}
struct Run {
    id: String,
    submission: Option<String>,
    attempt: String,
    task: String,
    at: i64,
    source: &'static str,
    outcome: String,
    duration: Option<i64>,
}
fn executable(id: &str) -> bool {
    id.strip_prefix("accept-default-")
        .or_else(|| id.strip_prefix("accept-"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
fn counted(r: &Run) -> bool {
    matches!(r.outcome.as_str(), "pass" | "fail")
}
pub(super) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let _snapshot = db.unchecked_transaction()?;
    let host = crate::telemetry::accounting::host_checks::verification_metadata(project)?;
    let mut runs = Vec::new();
    for r in &host {
        runs.push(Run {
            id: r["run_id"].as_str().unwrap_or_default().into(),
            submission: r["submission_id"].as_str().map(str::to_owned),
            attempt: r["attempt_id"].as_str().unwrap_or_default().into(),
            task: r["task"].as_str().unwrap_or_default().into(),
            at: r["started_unix_ms"].as_i64().unwrap_or(0),
            source: "host",
            outcome: r["outcome"].as_str().unwrap_or("unknown").into(),
            duration: r["duration_ms"].as_i64(),
        });
    }
    let host_count = runs.len();
    let mut old = 0;
    let mut old_tasks = BTreeMap::<String, i64>::new();
    let mut old_reasons = BTreeMap::<String, BTreeMap<String, i64>>::new();
    let mut old_submissions = BTreeMap::<String, (i64, String)>::new();
    let policy_ids = db.prepare("SELECT policy_id FROM acceptance_policies WHERE CASE WHEN json_valid(body) THEN json_extract(body,'$.toolchain') END IS NOT NULL")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let declared_executable = policy_ids.iter().any(|id| executable(id));
    for row in db.prepare("SELECT run_id,submission_id,attempt_id,task_id,created_unix_ms,policy_id,state,reason,exit_status,metadata FROM verification_runs ORDER BY created_unix_ms,run_id")?.query_map([], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<i64>>(8)?,r.get::<_,Option<String>>(9)?)))? {
        let (id,submission,attempt,task,at,policy,state,reason,exit,metadata)=row?;
        if !executable(&policy) { continue; }
        let meta: Value = metadata.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(Value::Null);
        if meta["toolchain"].is_null() {
            old += 1;
            *old_tasks.entry(task).or_default() += 1;
            if let Some(reason @ ("timeout" | "interrupted" | "spawn_failed" | "abandoned"))=reason.as_deref() {
                *old_reasons.entry(submission).or_default().entry(reason.into()).or_default()+=1;
                continue;
            }
            if state!="accepted" && exit.is_none() {continue;}
            old_submissions.entry(submission).and_modify(|prior| {if (at,&id)<(prior.0,&prior.1) {*prior=(at,id.clone());}}).or_insert((at,id));
            continue;
        }
        let at = meta["load"]["sampled_unix_ms"].as_i64().unwrap_or(at);
        let outcome = if state=="accepted" { "pass" } else if reason.as_deref()==Some("timeout") { "timeout" } else if matches!(reason.as_deref(),Some("interrupted" | "spawn_failed" | "abandoned")) {reason.as_deref().unwrap_or("abandoned")} else if exit.is_some() { "fail" } else { reason.as_deref().unwrap_or("abandoned") };
        runs.push(Run{id,submission:Some(submission),attempt,task,at,source:"sandbox",outcome:outcome.into(),duration:meta["duration_ms"].as_i64()});
    }
    runs.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    let sandbox_count = runs.len() - host_count;
    let rows = db.prepare("SELECT s.submission_id,s.attempt_id,json_array_length(s.claimed_checks),coalesce(json_extract(i.payload,'$.inputs.effective_profile.name'),'unknown'),coalesce(l.role,'unknown'),s.created_unix_ms FROM result_submissions s LEFT JOIN attempt_inputs i ON i.attempt_id=s.attempt_id LEFT JOIN task_lineage l ON l.task_id=s.task_id WHERE ?1 IS NULL OR s.created_unix_ms>=?1 ORDER BY s.submission_id")?.query_map([since],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut green, mut observed, mut no_run, mut claims, mut disputed, mut entries) =
        (0, 0, 0, 0, 0, 0);
    let mut profiles = BTreeMap::<String, [i64; 2]>::new();
    let mut roles = BTreeMap::<String, [i64; 2]>::new();
    let mut sources = BTreeMap::<String, [i64; 2]>::new();
    let mut excluded = BTreeMap::<String, i64>::new();
    let mut excluded_ids = BTreeSet::new();
    let mut details = Vec::new();
    let (mut uncertain, mut uncertain_claims) = (0, 0);
    for (submission, attempt, checks, profile, role, _submitted) in &rows {
        let linked = |r: &&Run| {
            r.submission.as_ref() == Some(submission)
                || (r.submission.is_none() && r.attempt == *attempt)
        };
        let candidates = runs.iter().filter(linked).collect::<Vec<_>>();
        if let Some(reasons) = old_reasons.get(submission) {
            for (reason, n) in reasons {
                *excluded.entry(reason.clone()).or_default() += n;
            }
        }
        for r in candidates.iter().filter(|r| !counted(r)) {
            if excluded_ids.insert(&r.id) {
                *excluded.entry(r.outcome.clone()).or_default() += 1;
            }
        }
        let first = candidates.into_iter().find(|r| counted(r));
        let missing_first = old_submissions
            .get(submission)
            .is_some_and(|(at, id)| first.is_none_or(|r| (*at, id) < (r.at, &r.id)));
        uncertain += i64::from(missing_first);
        uncertain_claims += i64::from(missing_first && *checks > 0);
        let ok = first.is_some_and(|r| r.outcome == "pass");
        if let Some(r) = first {
            observed += 1;
            green += i64::from(ok);
            for cell in [
                profiles.entry(profile.clone()).or_default(),
                roles.entry(role.clone()).or_default(),
                sources.entry(r.source.into()).or_default(),
            ] {
                cell[0] += i64::from(ok);
                cell[1] += 1;
            }
        } else {
            no_run += 1;
        }
        entries += checks;
        if *checks > 0 {
            claims += 1;
            disputed += i64::from(!ok);
        }
        details.push(json!({"submission_id":submission,"first_run_source":first.map(|r|r.source),"first_run_outcome":first.map(|r|&r.outcome),"claimed_check_count":checks}));
    }
    let breakdown = |cells: BTreeMap<String, [i64; 2]>| {
        cells
            .into_iter()
            .map(|(k, [n, d])| (k, share(n, d)))
            .collect::<BTreeMap<_, _>>()
    };
    let mut first = share(green, observed);
    first["no_independent_run"] = json!(no_run);
    first["excluded_runs_by_reason"] = json!(excluded);
    first["by_profile"] = json!(breakdown(profiles));
    first["by_role"] = json!(breakdown(roles));
    first["by_source"] = json!(breakdown(sources));
    first["submissions"] = json!(details);
    first["coverage"] = json!({"host":if host_count==0 {absent("no_host_checks_recorded")}else{json!(host_count)},"sandbox":if sandbox_count==0 {absent(if old>0 {"executable_run_metadata_missing"}else if declared_executable {"no_executable_runs"}else{"no_executable_policies"})}else{json!(sandbox_count)},"executable_runs_without_metadata":old});
    if observed == 0 && !rows.is_empty() {
        first["value"] = absent("no_independent_runs");
    }
    if uncertain > 0 {
        first["observed_share"] = first["value"].clone();
        first["value"] = absent("first_run_metadata_missing");
        first["uncertain_submissions"] = json!(uncertain);
    }
    let mut quality = json!({"value":ratio(claims,rows.len() as i64),"numerator":claims,"denominator":rows.len(),"claimed_check_entries":entries,"claimed_without_agreement":share(disputed,claims)});
    if uncertain_claims > 0 {
        quality["claimed_without_agreement"]["value"] = absent("first_run_metadata_missing");
        quality["uncertain_claims"] = json!(uncertain_claims);
    }
    let accepted=db.prepare("SELECT s.task_id,min(CASE WHEN c.route='verify_only' THEN v.created_unix_ms ELSE k.created_unix_ms END) FROM verified_results v JOIN result_submissions s USING(submission_id) JOIN task_contracts c ON c.task_id=s.task_id AND c.contract_revision=s.contract_revision LEFT JOIN integration_operations i ON i.verified_result_id=v.result_id LEFT JOIN integrated_commits k ON k.operation_id=i.operation_id WHERE c.contract_revision=(SELECT max(contract_revision) FROM task_contracts WHERE task_id=s.task_id) AND (c.route='verify_only' OR k.created_unix_ms IS NOT NULL) GROUP BY s.task_id HAVING ?1 IS NULL OR min(CASE WHEN c.route='verify_only' THEN v.created_unix_ms ELSE k.created_unix_ms END)>=?1")?.query_map([since],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut host_ms, mut sandbox_ms, mut missing) = (0i64, 0i64, 0);
    let mut samples = Vec::new();
    let mut tasks_without_runs = 0;
    let (mut host_missing, mut sandbox_missing) = (0, 0);
    for task in &accepted {
        let mut total = 0;
        let mut complete = true;
        if !runs.iter().any(|r| &r.task == task) && !old_tasks.contains_key(task) {
            tasks_without_runs += 1;
            complete = false;
        }
        for r in runs.iter().filter(|r| &r.task == task) {
            if let Some(ms) = r.duration.filter(|n| *n >= 0) {
                total += ms;
                if r.source == "host" {
                    host_ms += ms;
                } else {
                    sandbox_ms += ms;
                }
            } else {
                missing += 1;
                if r.source == "host" {
                    host_missing += 1;
                } else {
                    sandbox_missing += 1;
                }
                complete = false;
            }
        }
        // Eligible executable runs with old metadata cannot silently become zero minutes.
        let unknown = old_tasks.get(task).copied().unwrap_or(0);
        sandbox_missing += unknown;
        missing += unknown;
        complete &= unknown == 0;
        if complete {
            samples.push(total);
        }
    }
    samples.sort_unstable();
    let median = if samples.is_empty() {
        absent("no_complete_duration_samples")
    } else if samples.len().is_multiple_of(2) {
        let total =
            i128::from(samples[samples.len() / 2 - 1]) + i128::from(samples[samples.len() / 2]);
        json!(format!("{total}/120000"))
    } else {
        json!(format!("{}/60000", samples[samples.len() / 2]))
    };
    let p90 = if samples.is_empty() {
        absent("no_complete_duration_samples")
    } else {
        json!(format!(
            "{}/60000",
            samples[(samples.len() * 90).div_ceil(100) - 1]
        ))
    };
    let denom = accepted.len() as i64 * 60000;
    let minutes = json!({"value":if missing>0 {absent("verification_duration_missing")}else if tasks_without_runs>0 {absent("verification_not_observed")}else{ratio(host_ms+sandbox_ms,denom)},"accepted_tasks":accepted.len(),"host":if host_missing>0 {absent("verification_duration_missing")}else{ratio(host_ms,denom)},"sandbox":if sandbox_missing>0 {absent("verification_duration_missing")}else{ratio(sandbox_ms,denom)},"total_host_ms":host_ms,"total_sandbox_ms":sandbox_ms,"missing_duration_runs":missing,"tasks_without_runs":tasks_without_runs,"distribution":{"median":median,"p90":p90,"samples":samples.len()},"coverage":first["coverage"]});
    let self_check = self_checks(project, &db, since)?;
    let mut out: BTreeMap<String, Value> = BTreeMap::from([
        ("M100".into(), first),
        ("M101".into(), self_check),
        ("M102".into(), minutes),
        ("M103".into(), quality),
    ]);
    for (id, body) in &mut out {
        body["definition"] = json!(format!("{id}.v1"));
        body["name"] = json!(super::registry::find(id).map(|m| m.name));
    }
    Ok(out)
}
fn self_checks(project: &Path, db: &Connection, since: Option<i64>) -> Result<Value> {
    let Some(side) = crate::telemetry::sidecar::read(project)? else {
        return Ok(json!({"value":absent("session_metadata_not_collected")}));
    };
    if !table(&side, "codex_exec_classes")? {
        return Ok(json!({"value":absent("worker_session_stream_before_v17")}));
    }
    for required in ["codex_session_clock", "codex_exec_items", "rollout_sources"] {
        if !table(&side, required)? {
            return Ok(json!({"value":absent("session_metadata_not_collected")}));
        }
    }
    let history=db.prepare("SELECT a.id,min(s.created_unix_ms) FROM attempts a LEFT JOIN result_submissions s ON s.attempt_id=a.id GROUP BY a.id ORDER BY a.id")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let without_submission_time = history.iter().filter(|(_, at)| at.is_none()).count();
    let outside_window = history
        .iter()
        .filter(|(_, at)| since.is_some_and(|since| at.is_some_and(|at| at < since)))
        .count();
    let attempts = history
        .into_iter()
        .filter(|(_, at)| since.is_none_or(|since| at.is_some_and(|at| at >= since)))
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let (mut yes, mut observed, mut unknown) = (0, 0, 0);
    for attempt in &attempts {
        let (known,check):(bool,bool)=side.query_row("SELECT EXISTS(SELECT 1 FROM codex_session_clock c JOIN rollout_sources s USING(session_id) WHERE s.attempt_id=?1 AND s.binding='bound') AND NOT EXISTS(SELECT 1 FROM codex_exec_items e JOIN rollout_sources s USING(session_id) LEFT JOIN codex_exec_classes c USING(session_id,item_id) WHERE s.attempt_id=?1 AND s.binding='bound' AND coalesce(c.classification_available,0)=0), EXISTS(SELECT 1 FROM codex_exec_classes c JOIN codex_exec_items e USING(session_id,item_id) JOIN rollout_sources s USING(session_id) WHERE s.attempt_id=?1 AND s.binding='bound' AND c.class='project_test' AND e.exit_code IS NOT NULL)",params![attempt],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if known || check {
            observed += 1;
            yes += i64::from(check);
        } else {
            unknown += 1;
        }
    }
    Ok(
        json!({"value":if unknown>0 {absent("project_test_classification_incomplete")} else {ratio(yes,observed)},"numerator":yes,"denominator":attempts.len(),"observed_attempts":observed,"unavailable_attempts":unknown,"observed_share":ratio(yes,observed),"without_submission_time":without_submission_time,"excluded":{"outside_window":outside_window,"submission_time_unknown":if since.is_some(){without_submission_time}else{0}}}),
    )
}
