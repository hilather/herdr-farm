//! Claude Code native metadata adapter. All sources are isolated execution homes
//! from canonical Claude attempts, plus the explicitly scoped coordinator source.
use super::*;

pub const FIXTURE_VERSIONS: &[&str] = &["2.1.3", "2.1.286"];
/// Versions with a recorded steward live reconciliation (owner-approved).
pub const LIVE_VERSIONS: &[&str] = &["2.1.286"];
const LIVE_FIELDS: &[&str] = &["sessionId", "timestamp", "cwd", "version", "type", "message.model", "message.id",
    "message.usage.input_tokens", "message.usage.output_tokens", "message.usage.cache_creation_input_tokens",
    "message.usage.cache_read_input_tokens", "effort", "message.usage.output_tokens_details.thinking_tokens",
    "message.usage.cache_creation.ephemeral_5m_input_tokens", "message.usage.cache_creation.ephemeral_1h_input_tokens"];

pub(super) fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    if !root.parent().is_some_and(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
        || !std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir()) { return; }
    let Ok(dirs) = std::fs::read_dir(root) else { return };
    for dir in dirs.flatten() {
        if !dir.file_type().is_ok_and(|t| t.is_dir()) { continue; }
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for file in files.flatten() {
            if file.file_type().is_ok_and(|t| t.is_file()) && file.path().extension().is_some_and(|s| s == "jsonl") {
                out.push(file.path());
            }
        }
    }
}

pub fn capabilities() -> Value {
    use sanitize::Class::*;
    let source_fields = [("sessionId", Id), ("timestamp", Text), ("cwd", Path), ("version", Text), ("type", Tag),
        ("turnOrigin", EnumTag), ("promptSource", EnumTag), ("turnPosition.promptIndex", Number), ("turnPosition.turnIndex", Number),
        ("promptId", Id), ("requestId", Id), ("message.stop_reason", EnumTag), ("subtype", EnumTag), ("durationMs", Number),
        ("toolUseResult.backgroundTaskId", Id), ("operation", EnumTag), ("effort", Tag), ("message.usage.output_tokens_details.thinking_tokens", Number),
        ("message.usage.cache_creation.ephemeral_5m_input_tokens", Number), ("message.usage.cache_creation.ephemeral_1h_input_tokens", Number),
        ("message.model", ModelId), ("message.id", Id), ("isSidechain", Bool),
        ("message.usage.input_tokens", Number), ("message.usage.output_tokens", Number),
        ("message.usage.cache_creation_input_tokens", Number), ("message.usage.cache_read_input_tokens", Number),
        ("message.content.tool_use.id", Id), ("message.content.tool_use.name", Tag),
        ("message.content.tool_result.tool_use_id", Id), ("message.content.tool_result.is_error", Bool)];
    let fields = source_fields.into_iter().map(|(field, class)| json!({"kind": "line", "field": field,
        "available": true, "basis": match class { Text | Tag => "reported_excerpt", Path => "binding_only_home_redacted", _ => "reported" },
        // Live run of 2.1.286 (docs/telemetry/claude-live-2.1.286.md) observed the
        // binding inputs, line type, model, message id, effort, thinking and cache tiers.
        // Tool and sidechain fields stay fixture: the live run used no tools.
        "certified": if LIVE_FIELDS.contains(&field) { "live" } else { "fixture" },
        "live_versions": if LIVE_FIELDS.contains(&field) { LIVE_VERSIONS } else { &[] as &[&str] },
        "caveat": if field == "cwd" { Some("binding_only") } else { None }})).chain(
        ["message.text", "message.thinking", "tool_use.input", "tool_result.content", "toolUseResult.* (except backgroundTaskId)", "summary", "user_prompt"].map(|field|
            json!({"kind": "line", "field": field, "available": false, "basis": "unavailable", "certified": "none", "reason": "content_forbidden"})));
    json!({"adapter": "claude-code", "interface": "session_jsonl", "certified_versions": LIVE_VERSIONS, "fixture_versions": FIXTURE_VERSIONS,
        "accepted_versions": LIVE_VERSIONS, "version_rule":"at_or_above_lowest_live_certified", "certification": "live", "uncertified_version": "cli_version_uncertified",
        "fields": fields.collect::<Vec<_>>(), "profiles": [], "live_certification": "separate_owner_gated_step"})
}

pub fn allowlist() -> Vec<(String, sanitize::Class)> {
    use sanitize::Class::*;
    [("session_id", Id), ("timestamp", Text), ("version", Text), ("line_type", Tag), ("model", ModelId), ("message_id", Id),
        ("turn_origin", EnumTag), ("prompt_source", EnumTag), ("prompt_index", Number), ("turn_index", Number), ("prompt_id", Id),
        ("request_id", Id), ("stop_reason", EnumTag), ("subtype", EnumTag), ("duration_ms", Number), ("background_task_id", Id), ("operation", EnumTag),
        ("effort", Tag), ("thinking_tokens", Number), ("cache_write_5m_tokens", Number), ("cache_write_1h_tokens", Number),
        ("isSidechain", Bool), ("input_tokens", Number), ("output_tokens", Number), ("cache_creation_input_tokens", Number),
        ("cache_read_input_tokens", Number), ("tool_use_ids", IdList), ("tool_names", IdList), ("tool_result_ids", IdList),
        ("tool_result_errors", Number), ("unmapped_count", Number), ("unmapped_keys", IdList)].into_iter().map(|(k, c)| (k.to_owned(), c)).collect()
}

/// Only field names are reported for drift. Forbidden subtrees are never walked.
fn unmapped(raw: &Value, prefix: &str, known: &[&str], out: &mut Vec<String>, count: &mut u64) {
    if let Some(object) = raw.as_object() {
        for key in object.keys().filter(|k| !known.contains(&k.as_str())) {
            *count += 1;
            if out.len() < 128 { out.push(format!("{prefix}{key}")); }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_line(tx: &Transaction, ledger: &ingest::Ledger, at: u64, line: &[u8], _file: &Path, key: &str, home: &str,
    worktrees: &str, cursor: &mut Cursor, now: i64, done: &mut Collected) -> Result<bool> {
    let Ok(raw) = serde_json::from_slice::<Value>(line) else { return Ok(false) };
    let Some(kind) = raw["type"].as_str() else { return Ok(false) };
    let field = |value: &Value, name: &str, class| sanitize::field(value, name, class);
    let id = |value: &Value, name: &str| field(value, name, sanitize::Class::Id).as_str().map(str::to_owned);
    let timestamp = raw["timestamp"].as_str();
    if cursor.session.is_none() {
        let (Some(session), Some(cwd), Some(version)) = (id(&raw, "sessionId"), raw["cwd"].as_str(), raw["version"].as_str()) else { return Ok(true) };
        // Project slugs are lossy (every non-alphanumeric becomes a dash).
        // Walk all project directories and bind only from the reported absolute cwd.
        if Path::new(cwd).is_relative() || cwd.split('/').any(|p| p == "." || p == "..") { return Ok(false); }
        let version = field(&json!({"version": version}), "version", sanitize::Class::Text);
        let meta = json!({"type": "session_meta", "payload": {"id": format!("claude-code:{session}"), "cwd": cwd,
            "timestamp": timestamp, "cli_version": format!("claude-code/{}", version.as_str().unwrap_or("unknown")),
            "originator": "claude-code", "source": "user", "model_provider": "anthropic"}});
        apply(tx, ledger, at, meta, key, home, worktrees, cursor, now, done)?;
    }
    let Some((session, mut version, started)) = cursor.session.clone() else { return Ok(true) };
    // Mixed-session lines never inherit a different session's binding.
    if id(&raw, "sessionId").is_some_and(|s| format!("claude-code:{s}") != session) { return Ok(false); }
    if let Some(reported) = raw["version"].as_str() {
        let sanitized = field(&json!({"version": reported}), "version", sanitize::Class::Text);
        let reported = format!("claude-code/{}", sanitized.as_str().unwrap_or("unknown"));
        if reported != version {
            version = reported;
            tx.execute("UPDATE rollout_sources SET cli_version=?2 WHERE path_digest=?1", params![key, version])?;
            cursor.session = Some((session.clone(), version.clone(), started));
        }
    }
    let message = &raw["message"];
    let mut unknown = Vec::new();
    let mut unmapped_count = 0;
    unmapped(&raw, "", &["type", "sessionId", "timestamp", "cwd", "version", "isSidechain", "effort", "message", "toolUseResult", "summary", "turnOrigin", "promptSource", "turnPosition", "promptId", "requestId", "subtype", "durationMs", "operation"], &mut unknown, &mut unmapped_count);
    unmapped(message, "message.", &["id", "model", "usage", "content", "stop_reason"], &mut unknown, &mut unmapped_count);
    let usage = &message["usage"];
    unmapped(usage, "message.usage.", &["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens_details", "cache_creation"], &mut unknown, &mut unmapped_count);
    unmapped(&usage["output_tokens_details"], "message.usage.output_tokens_details.", &["thinking_tokens"], &mut unknown, &mut unmapped_count);
    unmapped(&usage["cache_creation"], "message.usage.cache_creation.", &["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"], &mut unknown, &mut unmapped_count);
    if !matches!(kind, "assistant" | "user" | "summary" | "system" | "queue-operation") { unknown.push(format!("type:{kind}")); unmapped_count += 1; }
    let mut payload = json!({"session_id": session, "timestamp": timestamp, "version": raw["version"], "line_type": kind,
        "model": field(message, "model", sanitize::Class::ModelId), "message_id": id(message, "id"), "isSidechain": raw["isSidechain"].as_bool(),
        "tool_use_ids": [], "tool_names": [], "tool_result_ids": [], "tool_result_errors": 0});
    for counter in ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"] {
        payload[counter] = field(usage, counter, sanitize::Class::Number);
    }
    payload["effort"] = field(&raw, "effort", sanitize::Class::Tag);
    payload["thinking_tokens"] = field(&usage["output_tokens_details"], "thinking_tokens", sanitize::Class::Number);
    payload["cache_write_5m_tokens"] = field(&usage["cache_creation"], "ephemeral_5m_input_tokens", sanitize::Class::Number);
    payload["cache_write_1h_tokens"] = field(&usage["cache_creation"], "ephemeral_1h_input_tokens", sanitize::Class::Number);
    cursor.effort = payload["effort"].as_str().map(str::to_owned);
    cursor.model = payload["model"].as_str().map(str::to_owned);
    if kind == "assistant" && (usage.is_object() || super::super::version::nearest(&version).is_some()) {
        let reported_id = id(message, "id");
        let message_id = reported_id.clone().unwrap_or_else(|| format!("unmapped:{key}:{at}"));
        let ordinal: Option<i64> = tx.query_row("SELECT ordinal FROM claude_messages WHERE session_id=?1 AND message_id=?2",
            params![session, message_id], |r| r.get(0)).optional()?;
        let ordinal = match ordinal {
            Some(ordinal) => ordinal,
            None => {
                let ordinal: i64 = tx.query_row("SELECT coalesce(max(ordinal),0)+1 FROM claude_messages WHERE session_id=?1", [&session], |r| r.get(0))?;
                tx.execute("INSERT INTO claude_messages(session_id,message_id,ordinal,path_digest,byte_offset) VALUES(?1,?2,?3,?4,?5)",
                    params![session, message_id, ordinal, key, at as i64])?;
                ordinal
            }
        };
        let first: (String, i64) = tx.query_row("SELECT path_digest,byte_offset FROM claude_messages WHERE session_id=?1 AND message_id=?2",
            params![session, message_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if first == (key.to_owned(), at as i64) {
            let number = |k: &str| payload[k].as_i64().filter(|v| (0..=(1 << 53)).contains(v));
            let input = reported_id.as_ref().and_then(|_| number("input_tokens")).zip(number("cache_read_input_tokens")).zip(number("cache_creation_input_tokens"))
                .and_then(|((i, r), w)| i.checked_add(r)?.checked_add(w));
            let output = number("output_tokens");
            let record = json!({"type": "token_usage_record", "timestamp": timestamp, "payload": {"response_id": message_id,
                "usage": {"input_tokens": input, "cached_input_tokens": number("cache_read_input_tokens"),
                    "cache_write_input_tokens": number("cache_creation_input_tokens"), "output_tokens": output,
                    "reasoning_output_tokens": number("thinking_tokens").unwrap_or(0),
                    "reasoning_reported": number("thinking_tokens").is_some(),
                    "cache_write_5m_tokens": number("cache_write_5m_tokens"), "cache_write_1h_tokens": number("cache_write_1h_tokens"), "total_tokens": input.zip(output).and_then(|(i,o)| i.checked_add(o))}}});
            cursor.records = ordinal - 1;
            apply(tx, ledger, at, record, key, home, worktrees, cursor, now, done)?;
        }
        cursor.records = tx.query_row("SELECT count(*) FROM claude_messages WHERE path_digest=?1", [key], |r| r.get(0))?;
    }
    if raw["isSidechain"].as_bool() == Some(true) {
        let activity = format!("sidechain:{key}:{at}");
        tx.execute("INSERT OR IGNORE INTO codex_agent_items(session_id,item_type,item_id,agent_thread_id,completed_unix_ms)
            VALUES(?1,'SubAgentActivity',?2,NULL,?3)", params![session, activity, ms(timestamp)])?;
    }
    for block in message["content"].as_array().into_iter().flatten() {
        let block_kind = block["type"].as_str().unwrap_or("unknown");
        let allowed: &[&str] = match block_kind {
            "tool_use" => &["type", "id", "name", "input"],
            "tool_result" => &["type", "tool_use_id", "is_error", "content"],
            "text" => &["type", "text"], "thinking" => &["type", "thinking", "signature"], _ => &["type"],
        };
        unmapped(block, "message.content.", allowed, &mut unknown, &mut unmapped_count);
        if !matches!(block_kind, "tool_use" | "tool_result" | "text" | "thinking") { unknown.push(format!("block_type:{block_kind}")); unmapped_count += 1; }
        if kind == "assistant" && block_kind == "tool_use" && let Some(call) = id(block, "id") {
            let name = field(block, "name", sanitize::Class::Tag);
            payload["tool_use_ids"].as_array_mut().unwrap().push(json!(call));
            payload["tool_names"].as_array_mut().unwrap().push(name.clone());
            apply(tx, ledger, at, json!({"type": "response_item", "timestamp": timestamp, "payload": {
                "type": "function_call", "call_id": call, "name": name}}), key, home, worktrees, cursor, now, done)?;
        }
        if kind == "user" && block_kind == "tool_result" && let Some(call) = id(block, "tool_use_id") {
            payload["tool_result_ids"].as_array_mut().unwrap().push(json!(call));
            let error = block["is_error"].as_bool();
            if error == Some(true) { payload["tool_result_errors"] = json!(payload["tool_result_errors"].as_i64().unwrap_or(0) + 1); }
            apply(tx, ledger, at, json!({"type": "response_item", "timestamp": timestamp, "payload": {
                "type": "function_call_output", "call_id": call}}), key, home, worktrees, cursor, now, done)?;
            // Preserve reported boolean outcomes without inventing shell exit codes.
            tx.execute("INSERT OR IGNORE INTO claude_tool_results(session_id,call_id,is_error,completed_unix_ms) VALUES(?1,?2,?3,?4)",
                params![session, call, error, ms(timestamp)])?;
        }
    }
    // Read these leaves only in their approved line classes. Never walk toolUseResult.
    let tool_result = message["content"].as_array().is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"));
    if kind == "user" && !tool_result {
        payload["turn_origin"] = field(&raw, "turnOrigin", sanitize::Class::EnumTag);
        payload["prompt_source"] = field(&raw, "promptSource", sanitize::Class::EnumTag);
        payload["prompt_id"] = field(&raw, "promptId", sanitize::Class::Id);
        payload["prompt_index"] = field(&raw["turnPosition"], "promptIndex", sanitize::Class::Number);
        payload["turn_index"] = field(&raw["turnPosition"], "turnIndex", sanitize::Class::Number);
    }
    if kind == "assistant" {
        payload["request_id"] = field(&raw, "requestId", sanitize::Class::Id);
        payload["stop_reason"] = field(message, "stop_reason", sanitize::Class::EnumTag);
    }
    if kind == "system" {
        payload["subtype"] = field(&raw, "subtype", sanitize::Class::EnumTag);
        payload["duration_ms"] = field(&raw, "durationMs", sanitize::Class::Number);
    }
    if tool_result { payload["background_task_id"] = field(&raw["toolUseResult"], "backgroundTaskId", sanitize::Class::Id); }
    if kind == "queue-operation" { payload["operation"] = field(&raw, "operation", sanitize::Class::EnumTag); }
    if matches!(kind, "user" | "assistant" | "system" | "queue-operation") {
        tx.execute("INSERT OR IGNORE INTO claude_turn_lines(session_id,path_digest,byte_offset,occurred_unix_ms,line_type,is_prompt,metadata)
            VALUES(?1,?2,?3,?4,?5,?6,?7)", params![session, key, at as i64, ms(timestamp), kind,
            kind == "user" && !tool_result, serde_json::to_string(&sanitize::payload(&allowlist(), &payload))?])?;
    }
    payload["unmapped_count"] = json!(unmapped_count);
    payload["unmapped_keys"] = json!(unknown);
    ledger.observe(tx, at, ingest::Record { kind: "claude_line", payload: &payload, occurred_unix_ms: ms(timestamp), session: &session,
        adapter_version: &version, certified: accepted_version(&version) }, now)?;
    cursor.uncertified |= !accepted_version(&version);
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn apply(tx: &Transaction, ledger: &ingest::Ledger, at: u64, value: Value, key: &str, home: &str, worktrees: &str,
    cursor: &mut Cursor, now: i64, done: &mut Collected) -> Result<()> {
    let line = serde_json::to_vec(&value)?;
    let tag = serde_json::from_slice::<Tag>(&line)?;
    anyhow::ensure!(record(tx, ledger, at, &tag, &line, key, home, worktrees, cursor, now, done, &mut None)?, "invalid Claude metadata mapping");
    Ok(())
}

/// Owner-approved coordinator boundary: compute one directory, never enumerate
/// the owner's projects or inspect any other Claude files.
pub(super) fn coordinator_source(project: &Path) -> Result<Option<(PathBuf, bool)>> {
    let text = match std::fs::read_to_string(project.join("PROJECT.md")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let Some(front) = text.strip_prefix("+++\n").and_then(|s| s.split_once("\n+++").map(|(f, _)| f)) else { return Ok(None) };
    let settings: toml::Table = toml::from_str(front)?;
    if settings.get("coordinator_agent").and_then(toml::Value::as_str).unwrap_or("claude") != "claude" { return Ok(None); }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Ok(None) };
    let enabled = super::super::views::telemetry_switch(&crate::product_environment::config_dir_for_home(&home), "collect_coordinator_usage")?;
    let project = std::fs::canonicalize(project)?;
    let encoded: String = project.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Ok(Some((home.join(".claude/projects").join(encoded), enabled)))
}

/// The enabled coordinator directory, only when it exists as a real directory:
/// a project whose coordinator never ran has nothing to collect, so it must
/// not become a telemetry source (and get a sidecar) by default.
pub(super) fn coordinator_dir(project: &Path) -> Result<Option<PathBuf>> {
    Ok(coordinator_source(project)?.filter(|(path, enabled)| *enabled && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())).map(|(path, _)| path))
}

/// Open every component relative to a pinned directory descriptor. Neither
/// discovery nor tail reads follow symlinks, including intermediate directories.
pub(super) fn open_scoped(path: &Path, directory: bool) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "absolute source path required"));
    }
    let mut parent = std::fs::File::open("/")?;
    let parts: Vec<_> = path.components().filter_map(|c| match c { std::path::Component::Normal(s) => Some(s), _ => None }).collect();
    for (i, part) in parts.iter().enumerate() {
        let name = std::ffi::CString::new(part.as_encoded_bytes())?;
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
            | if directory || i + 1 < parts.len() { libc::O_DIRECTORY } else { 0 };
        // SAFETY: parent is live, name is NUL-terminated; the returned fd is owned.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 { return Err(std::io::Error::last_os_error()); }
        parent = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(parent)
}

pub(super) fn coordinator_files(root: &Path) -> Vec<PathBuf> {
    use std::os::fd::AsRawFd;
    let Ok(dir) = open_scoped(root, true) else { return Vec::new() };
    let Ok(files) = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd())) else { return Vec::new() };
    files.flatten().filter(|f| f.file_type().is_ok_and(|t| t.is_file()) && f.path().extension().is_some_and(|s| s == "jsonl"))
        .map(|f| root.join(f.file_name())).collect()
}
