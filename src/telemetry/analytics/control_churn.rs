//! Control fencing churn from retained canonical events; identities stay evidence.
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn metrics(project: &Path, since: Option<i64>) -> Result<BTreeMap<String, Value>> {
    let db = crate::telemetry::read_only(&project.join(".state/state.db"))?;
    let now = jiff::Timestamp::now().as_millisecond();
    let mut count = 0_i64;
    let mut legacy = 0_i64;
    let mut causes = BTreeMap::<(String, String), i64>::new();
    let mut stmt = db.prepare("SELECT payload FROM events WHERE entity='project' AND kind IN ('project.control_changed','project.reconciliation_invalidated','project.launch_run_paused') ORDER BY sequence")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let payload: Value = serde_json::from_str(&row.get::<_, String>(0)?)?;
        let Some(at) = payload["control_unix_ms"].as_i64() else { legacy += 1; continue; };
        if at < since.unwrap_or(i64::MIN) || at > now { continue; }
        count += 1;
        if let Some(cause) = payload["invalidation_reason"].as_str().or_else(|| payload["cause"].as_str()) {
            let bindings = payload["invalidated_bindings"].as_array();
            if let Some(bindings) = bindings.filter(|b| !b.is_empty()) {
                for binding in bindings.iter().filter_map(Value::as_str) {
                    *causes.entry((cause.into(), binding.into())).or_default() += 1;
                }
            } else { *causes.entry((cause.into(), String::new())).or_default() += 1; }
        }
    }
    let dominant = causes.into_iter().max_by(|a,b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|((cause,binding),occurrences)| json!({"cause":cause,"binding":binding,"occurrences":occurrences}));
    Ok(BTreeMap::from([("M105".into(), json!({"definition":"M105.v1", "value":count,"numerator":count,
        "dominant_invalidation":dominant,"legacy_events_without_time":legacy,"excluded":{"missing_control_time":legacy}}))]))
}
