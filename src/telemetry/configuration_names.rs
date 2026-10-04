//! Display evidence only: never changes comparison identities or populations.
use std::path::Path;
use serde_json::{Value, json};

pub(crate) fn read(project: &Path) -> anyhow::Result<Vec<Value>> {
    let db = super::read_only(&project.join(".state/state.db"))?;
    read_db(&db)
}

pub(crate) fn read_db(db: &rusqlite::Connection) -> anyhow::Result<Vec<Value>> {
    let exists: bool = db.query_row("SELECT count(*)=3 FROM sqlite_master WHERE type='table' AND name IN ('dispatch_decisions','attempt_inputs','native_profiles')", [], |r| r.get(0))?;
    if !exists { return Ok(Vec::new()); }
    let mut statement = db.prepare("SELECT d.chosen_configuration_id,json_extract(i.payload,'$.inputs.effective_profile.name'),json_extract(i.payload,'$.inputs.effective_profile.kind'),json_extract(n.report,'$.evidence.interaction.pinned.model'),json_extract(n.report,'$.evidence.interaction.pinned.reasoning_effort'),min(d.decided_unix_ms),max(d.decided_unix_ms) FROM dispatch_decisions d JOIN attempt_inputs i ON i.attempt_id=d.attempt_id LEFT JOIN native_profiles n ON n.profile_digest=json_extract(i.payload,'$.inputs.profile.digest') GROUP BY 1,2,3,4,5 ORDER BY 2,1")?;
    let rows = statement.query_map([], |r| {
        let id: String = r.get(0)?;
        let profile: Option<String> = r.get(1)?;
        let kind: Option<String> = r.get(2)?;
        let model: Option<String> = r.get(3)?;
        let effort: Option<String> = r.get(4)?;
        fn known(v: &Option<String>) -> &str { v.as_deref().unwrap_or("unknown") }
        let label = format!("{} ({} {} {})", known(&profile), known(&kind), known(&model), known(&effort));
        Ok(json!({"configuration_id": id, "label": label, "profile": profile, "agent_kind": kind, "model": model, "reasoning_effort": effort, "first_use_unix_ms": r.get::<_, i64>(5)?, "last_use_unix_ms": r.get::<_, i64>(6)?}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn text(rows: &[Value]) -> String {
    let mut out = String::new();
    for row in rows {
        let id = row["configuration_id"].as_str().unwrap_or("unknown");
        out += &format!("  {} {} first_use={} last_use={}\n", row["label"].as_str().unwrap_or("unknown"), id.get(..19).unwrap_or(id), row["first_use_unix_ms"], row["last_use_unix_ms"]);
    }
    out
}

pub(crate) fn project_text(project: &Path) -> anyhow::Result<String> { Ok(text(&read(project)?)) }
