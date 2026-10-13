//! Stable inbox delivery shared by the CLI and automatic health cadence.
use super::store::{ALERT_COLUMNS, alert_json, slug, tables};
use crate::telemetry::export::external::{self, Destination, Setting};
use anyhow::{Result, bail};
use rusqlite::params;
use serde_json::{Value, json};
use std::path::Path;

pub const ALERTS_CONFIG: Setting = Setting { file: "telemetry-alerts.toml", schema: "telemetry-alerts-config.v1", what: "alert notification",
    disabled: "external_notification_disabled" };
const KIND: &str = "telemetry-health";

fn open_alerts(project: &Path) -> Result<Option<Vec<Value>>> {
    let Some(db) = crate::telemetry::sidecar::read(project)? else { return Ok(None) };
    if !tables(&db)? { return Ok(Some(Vec::new())); }
    Ok(Some(db.prepare(&format!("SELECT {ALERT_COLUMNS} FROM health_alerts WHERE resolved_unix_ms IS NULL ORDER BY alert_id"))?
        .query_map([], alert_json)?.collect::<rusqlite::Result<_>>()?))
}

/// The notice's one-line summary: labels and reason codes only.
fn summary(project: &str, alert: &Value) -> String {
    let labels = &alert["labels"];
    let mut scope = String::new();
    for key in ["service", "role"] { if let Some(v) = labels[key].as_str() { scope += &format!(" {key}={v}"); } }
    let codes: Vec<&str> = alert["reasons"].as_array().into_iter().flatten().filter_map(|r| r["code"].as_str()).collect();
    format!("telemetry health {}: {} [{}{scope}] {}; advisory — `herdr-farm telemetry {project} health alerts`", alert["state"].as_str().unwrap_or(""),
        alert["rule"].as_str().unwrap_or(""), labels["family"].as_str().unwrap_or(""), codes.join(","))
}

/// `health notify`: one inbox notice per open alert not yet noticed.
pub fn inbox(project: &Path) -> Result<Value> {
    deliver(project, None)
}

/// Owner policy: critical by default, warn optionally, or off.
pub fn automatic(project: &Path, config_dir: &Path) -> Result<Value> {
    let setting = crate::telemetry::views::telemetry_setting(config_dir, "health_notify")?;
    let threshold = setting.as_ref().and_then(toml::Value::as_str).unwrap_or("critical");
    anyhow::ensure!(setting.is_none() || matches!(&setting, Some(toml::Value::String(_))), "[telemetry] health_notify must be critical, warn or off");
    anyhow::ensure!(matches!(threshold, "critical" | "warn" | "off"), "[telemetry] health_notify must be critical, warn or off");
    if threshold == "off" { return Ok(json!({"disabled": true})); }
    deliver(project, Some(threshold))
}

fn deliver(project: &Path, threshold: Option<&str>) -> Result<Value> {
    let Some(mut alerts) = open_alerts(project)? else { return Ok(json!({"status": "unavailable", "reason": "collection_not_run"})) };
    if threshold.is_some() {
        let Some(db) = crate::telemetry::sidecar::read(project)? else { return Ok(json!({"status": "unavailable", "reason": "collection_not_run"})); };
        if tables(&db)? {
            alerts.extend(db.prepare(&format!("SELECT {ALERT_COLUMNS} FROM health_alerts WHERE resolved_unix_ms IS NOT NULL AND notice_id IS NOT NULL AND notice_id NOT LIKE '%-resolved' ORDER BY alert_id"))?
                .query_map([], alert_json)?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
    }
    let slug = slug(project);
    let (mut delivered, mut already) = (Vec::new(), 0usize);
    for alert in alerts {
        let resolved = !alert["resolved_unix_ms"].is_null() && !alert["notified_unix_ms"].is_null();
        if !resolved && threshold.is_some_and(|t| alert["state"] != "critical" && !(t == "warn" && alert["state"] == "warn")) { continue; }
        if !resolved && !alert["notified_unix_ms"].is_null() { already += 1; continue; }
        let (id, opened) = (alert["alert_id"].as_i64().unwrap_or_default(), alert["opened_unix_ms"].as_i64().unwrap_or_default());
        let notice = format!("{KIND}-{id}-{opened}{}", if resolved { "-resolved" } else { "" });
        let mut text = summary(&slug, &alert);
        if resolved { text = format!("resolved: {text}"); }
        let evidence = serde_json::to_string(&alert["reasons"])?;
        text += &format!("; evidence: {}", evidence.chars().take(512).collect::<String>());
        if alert["rule"] == "launch_stalled" { text += "; next step: inspect launch progress, then relaunch / cancel-attempt"; }
        if let Some(db) = crate::telemetry::sidecar::read(project)? {
            let suppressed: i64 = db.query_row("SELECT coalesce(sum(suppressed),0) FROM health_rule_states WHERE rule_key=?1", [alert["labels"].to_string()], |r| r.get(0))?;
            if suppressed > 0 { text += &format!("; flapping summary: {suppressed} reopen observations suppressed by cooldown"); }
        }
        let content = crate::domain::InboxContent { id: notice.clone(), kind: KIND.into(), subject: alert["rule"].as_str().unwrap_or("health").into(),
            created: String::new(), summary: text, body: String::new() };
        // Retain an opening intent before canonical delivery. This permits retries
        // after resolution without replaying old, never-notified history.
        if !resolved && let Some(db) = crate::telemetry::sidecar::open(project, false)? {
            db.execute("UPDATE health_alerts SET notice_id=?2 WHERE alert_id=?1 AND notice_id IS NULL", params![id, notice])?;
        }
        let mut store = crate::store::SqliteStore::open_scoped(&project.join(".state/state.db"))?;
        let mut outcome = None;
        for _ in 0..3 {
            let now = jiff::Timestamp::now().as_millisecond();
            match store.deliver_telemetry_notice(store.current_head()?, &content, now) {
                Ok(result) => { outcome = Some(result); break; }
                Err(crate::store::StoreError::Conflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        let Some(outcome) = outcome else { bail!("store head moved on every attempt; retry `health notify`") };
        drop(store);
        let now = jiff::Timestamp::now().as_millisecond();
        if let Some(db) = crate::telemetry::sidecar::open(project, false)? {
            db.execute("UPDATE health_alerts SET notified_unix_ms=coalesce(notified_unix_ms,?2),notice_id=?3 WHERE alert_id=?1", params![id, now, notice])?;
        }
        delivered.push(json!({"alert_id": id, "notice_id": notice, "outcome": if outcome == crate::domain::ReminderOutcome::Delivered { "delivered" } else { "already_delivered" }, "summary": content.summary}));
    }
    Ok(json!({"target": "inbox", "delivered": delivered, "already_notified": already}))
}

/// `health notify --external`: every open alert to the configured destination; refused unless enabled.
pub fn external(project: &Path, config_dir: &Path) -> Result<(Value, Option<String>)> {
    let destination = match external::load_setting(config_dir, &ALERTS_CONFIG)? {
        Ok(d) => d,
        Err((code, detail)) => bail!("notify rejected: {}", json!({"code": code, "detail": detail})),
    };
    let Some(alerts) = open_alerts(project)? else { return Ok((json!({"status": "unavailable", "reason": "collection_not_run"}), None)) };
    let slug = slug(project);
    let payload = |a: &Value| json!({"contract": "telemetry-health-alert.v1", "project": slug, "alert": {"alert_id": a["alert_id"], "rule": a["rule"],
        "labels": a["labels"], "state": a["state"], "reasons": a["reasons"], "metric": a["metric"], "evidence_window": a["evidence_window"],
        "evidence": a["evidence"], "rules_version": a["rules_version"], "opened_unix_ms": a["opened_unix_ms"], "last_seen_unix_ms": a["last_seen_unix_ms"],
        "occurrences": a["occurrences"]}});
    match destination {
        Destination::Stdout => {
            let lines: String = alerts.iter().map(|a| payload(a).to_string() + "\n").collect();
            Ok((json!({"target": "external", "destination": "stdout", "alerts": alerts.len()}), Some(lines)))
        }
        Destination::Directory(dir) => {
            let (mut written, mut existing) = (Vec::new(), 0usize);
            for a in &alerts {
                let file = dir.join(format!("{slug}-health-{}-{}.json", a["alert_id"], a["opened_unix_ms"]));
                if std::fs::symlink_metadata(&file).is_ok() { existing += 1; continue; }
                external::write_new(&file, (serde_json::to_string_pretty(&payload(a))? + "\n").as_bytes())?;
                written.push(json!(file.file_name().map(|n| n.to_string_lossy().into_owned())));
            }
            Ok((json!({"target": "external", "destination": "directory", "written": written, "already_written": existing}), None))
        }
    }
}
