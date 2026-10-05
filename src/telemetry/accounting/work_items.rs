//! Read-only work-item history and published-rate spend, canonical schema 72.
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub fn read(project: &Path) -> Result<Value> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let tx = db.unchecked_transaction()?;
    let present: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_lineage')",
        [],
        |r| r.get(0),
    )?;
    if !present {
        return Ok(super::unavailable("lineage_not_recorded"));
    }
    let costs = match crate::telemetry::sidecar::read(project)? {
        Some(sidecar) => super::cost::cost(&sidecar, None, None)?,
        None => super::unavailable("collection_not_run"),
    };
    let estimates: BTreeMap<&str, &Value> = costs["attempts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["attempt_id"].as_str()?, &a["estimate"])))
        .collect();
    let rows: Vec<(String,String,String,Option<String>)> = tx.prepare("SELECT task_id,work_item,role,supersedes_task FROM task_lineage ORDER BY work_item,task_id")?
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut items = BTreeMap::<String, Vec<(String, String, Option<String>)>>::new();
    for (task, work, role, supersedes) in rows {
        items
            .entry(work)
            .or_default()
            .push((task, role, supersedes));
    }
    let mut detail = Vec::new();
    for (work, tasks) in items {
        let mut by_role = BTreeMap::<String, Vec<String>>::new();
        let mut attempts = Vec::new();
        let mut superseded = std::collections::BTreeSet::new();
        for (task, role, supersedes) in &tasks {
            by_role.entry(role.clone()).or_default().push(task.clone());
            if let Some(old) = supersedes {
                superseded.insert(old.clone());
            }
            attempts.extend(task_attempts(&tx, task, &estimates)?);
        }
        let first = attempts
            .iter()
            .filter_map(|a| a["launch_unix_ms"].as_i64())
            .min();
        let last = attempts
            .iter()
            .filter_map(|a| a["terminal_unix_ms"].as_i64())
            .max();
        // Reuse accounting's decimal summation and never add currencies together.
        let mut entries = Vec::new();
        for attempt in &attempts {
            let estimate = &attempt["cost_estimate"];
            if let Some(amount) = estimate["amount"]
                .as_str()
                .or_else(|| estimate["priced_amount"].as_str())
            {
                entries.push(json!({"valuation":{"status":"priced","amount":amount,"currency":estimate["currency"]}}));
            }
            for (currency, amount) in estimate["priced_by_currency"]
                .as_object()
                .into_iter()
                .flatten()
            {
                entries.push(
                    json!({"valuation":{"status":"priced","amount":amount,"currency":currency}}),
                );
            }
            if estimate["status"] != "complete" {
                entries.push(json!({"valuation":{"status":"unavailable","reason":estimate["reason"].as_str().unwrap_or("cost_not_observed")}}));
            }
        }
        let (spend, coverage) = super::cost::summarize(&entries.iter().collect::<Vec<_>>())?;
        let open = attempts.iter().any(|a| a["terminal_unix_ms"].is_null());
        detail.push(json!({"work_item":work,"tasks_by_role":by_role,"task_count":tasks.len(),
            "fix_rounds":tasks.iter().filter(|(_,role,_)| role=="fix").count(),
            "attempt_count":attempts.len(),"attempts":attempts,"first_launch_unix_ms":first,"last_terminal_unix_ms":last,
            "elapsed_ms": first.zip(last).map(|(start,end)| end-start),"censored":open,
            "spend":{"basis":super::cost::BASIS,"estimate":spend,"coverage":coverage},
            "superseded_tasks":superseded,"superseded_count":superseded.len()}));
    }
    let unbound: i64 = tx.query_row(
        "SELECT count(*) FROM tasks WHERE id NOT IN (SELECT task_id FROM task_lineage)",
        [],
        |r| r.get(0),
    )?;
    Ok(json!({"work_items":detail,"tasks_without_lineage":unbound}))
}
fn task_attempts(
    db: &Connection,
    task: &str,
    costs: &BTreeMap<&str, &Value>,
) -> Result<Vec<Value>> {
    let rows: Vec<(String,String,Option<i64>,Option<i64>)> = db.prepare("SELECT a.id,a.state,
        (SELECT min(unix_ms) FROM attempt_lifecycle WHERE attempt_id=a.id AND state IN ('launching','running')),
        (SELECT max(unix_ms) FROM attempt_lifecycle WHERE attempt_id=a.id AND state IN ('completed','failed','cancelled','lost'))
        FROM attempts a WHERE a.task_id=?1 ORDER BY a.rowid")?
        .query_map([task], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows.into_iter().map(|(attempt,state,launch,terminal)| {
        let estimate = costs.get(attempt.as_str()).copied().cloned().unwrap_or_else(|| super::unavailable("cost_not_observed"));
        json!({"attempt_id":attempt,"task_id":task,"state":state,"launch_unix_ms":launch,"terminal_unix_ms":terminal,"cost_estimate":estimate})
    }).collect())
}
pub fn text(value: &Value) -> String {
    let mut out = String::new();
    for item in value["work_items"].as_array().into_iter().flatten() {
        out += &format!(
            "{} tasks={} attempts={} fix_rounds={} superseded={} elapsed_ms={} spend={}\n",
            item["work_item"].as_str().unwrap_or_default(),
            item["task_count"],
            item["attempt_count"],
            item["fix_rounds"],
            item["superseded_count"],
            item["elapsed_ms"],
            item["spend"]["estimate"]
        );
    }
    if out.is_empty() {
        out = format!("{value}\n");
    }
    out
}
