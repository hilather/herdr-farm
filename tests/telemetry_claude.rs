//! DG4b end-to-end: public admission/store workflow and CLI, synthetic homes only.
#![cfg(all(feature = "state-store", target_os = "linux"))]
#![allow(clippy::disallowed_methods)]
mod support;
use std::{fs, path::PathBuf};
use serde_json::{Value, json};
use support::telemetry::*;

fn claude() -> Fixture { claude_with_pins(None) }

fn claude_with_pins(effort: Option<&str>) -> Fixture {
    let mut f = Fixture::new();
    let home = f.tmp.path().join("claude-execution-home");
    let mut profile = codex_profile(&f.config, "claude", "claude", Some(&home));
    profile.agent.version = "2.1.286".into();
    let path = f.project.join(".state/state.db");
    plant_profile_with_pins(&path, profile, None, effort);
    f.readmit("claude");
    let db = rusqlite::Connection::open(path).unwrap();
    (f.attempt, f.decided) = db.query_row("SELECT a.id,d.decided_unix_ms FROM attempts a JOIN dispatch_decisions d ON d.attempt_id=a.id WHERE a.state='reserved'",
        [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    f.home = home;
    db.execute("INSERT INTO collector_bindings(attempt_id,revision,state,collector,execution_home,unix_ms,source)
        VALUES(?1,1,'active','claude',?2,?3,'apply_launch_started')",
        rusqlite::params![f.attempt, f.home.display().to_string(), unix_ms()]).unwrap();
    f
}
fn transcript(f: &Fixture, sid: &str, cwd: &str, version: &str, time: i64) -> PathBuf {
    let dir = f.home.join(".claude/projects").join(cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>());
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{sid}.jsonl"));
    let text = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/claude-code/session.jsonl")).unwrap();
    fs::write(&path, text.replace("@SID@", sid).replace("@CWD@", cwd).replace("@VERSION@", version)
        .replace("@TS@", &jiff::Timestamp::from_millisecond(time).unwrap().to_string())).unwrap();
    path
}
fn attempt_usage(f: &Fixture) -> Value {
    f.cli_args(&["usage", "--json"]).0["attempts"].as_array().unwrap().iter()
        .find(|a| a["attempt_id"] == f.attempt).unwrap()["usage"].clone()
}
fn no_secrets(f: &Fixture) {
    for name in ["telemetry.db", "telemetry.db-wal", "telemetry.db-shm"] {
        if let Ok(bytes) = fs::read(f.project.join(".state").join(name)) {
            assert!(!bytes.windows(b"CLAUDE_SECRET_".len()).any(|w| w == b"CLAUDE_SECRET_"), "content leaked to {name}");
        }
    }
}
#[test]
fn native_claude_usage_tools_sidechains_and_privacy() {
    let f = claude();
    transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    let collected = f.cli("collect").0;
    assert_eq!(collected["collected"]["records"], 2);
    assert_eq!(attempt_usage(&f), json!({"input_tokens":382,"cached_input_tokens":240,"cache_write_input_tokens":32,
        "output_tokens":25,"reasoning_output_tokens":{"status":"unavailable","reason":"reasoning_tokens_not_reported"},"total_tokens":407,"records":2}));
    let attempts = f.cli_args(&["attempts", "--json"]).0;
    let observed = attempts["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == f.attempt).unwrap();
    assert_eq!(observed["usage"]["total_tokens"], 407);
    f.cli_args(&["accounting", "sync"]);
    let ledger = f.cli_args(&["accounting", "entries"]).0;
    let entries = ledger["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["normalization_version"], "claude-code-v2");
    let normalized: Vec<Value> = entries.iter().map(|e| e["normalized"].clone()).collect();
    assert!(normalized.contains(&json!({"input_tokens":330,"cache_read_tokens":200,"new_input_tokens":100,"cache_write_tokens":30,
        "output_tokens":20,"reasoning_tokens":0,"total_tokens":350})));
    let tools = f.cli_args(&["accounting", "tools", "--json"]).0;
    assert_eq!(tools["sessions"][0]["tools"]["issued"], 3);
    assert_eq!(tools["sessions"][0]["tools"]["executed"], 3);
    assert_eq!(tools["sessions"][0]["tools"]["succeeded"], 2);
    assert_eq!(tools["sessions"][0]["tools"]["failed"], 1);
    assert_eq!(tools["metrics"]["M17"]["value"], "2/3");
    assert_eq!(tools["metrics"]["M16"]["collaboration"]["sidechain_turns"], 2);
    let report = f.report();
    assert_eq!(report["metrics"]["M08"]["value"], 382);
    assert_eq!(report["metrics"]["M09"]["value"], 25);
    assert_eq!(report["metrics"]["M09"]["reasoning_output_tokens"]["reason"], "reasoning_tokens_not_reported");
    assert_eq!(report["metrics"]["M15"]["value"], "2/2");
    assert_eq!(f.count("claude_messages"), 2);
    let db = f.sidecar();
    assert_eq!(db.query_row("SELECT count(*) FROM source_observations WHERE json_extract(provenance,'$.adapter')='claude-code'", [], |r| r.get::<_, i64>(0)).unwrap(), 8);
    let unknown: String = db.query_row("SELECT json_extract(payload,'$.unmapped_keys') FROM source_observations WHERE json_extract(payload,'$.line_type')='future-line'", [], |r| r.get(0)).unwrap();
    assert!(unknown.contains("futureField") && unknown.contains("type:future-line"));
    assert_eq!(db.query_row("SELECT json_extract(payload,'$.unmapped_count') FROM source_observations WHERE json_extract(payload,'$.line_type')='future-line'",
        [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    f.cli("collect");
    assert_eq!(f.count("claude_messages"), 2);
    assert_eq!(attempt_usage(&f)["total_tokens"], 407);
    let capabilities = f.cli_args(&["collectors", "capabilities", "--json"]).0;
    let adapter = capabilities["adapters"].as_array().unwrap().iter().find(|a| a["adapter"] == "claude-code").unwrap();
    assert_eq!(adapter["fixture_versions"], json!(["2.1.3", "2.1.286"]));
    // Live only for the fields the 2.1.286 live run observed; tools stay fixture.
    assert_eq!(adapter["certified_versions"], json!(["2.1.286"]));
    let live: Vec<&str> = adapter["fields"].as_array().unwrap().iter().filter(|f| f["certified"] == "live")
        .map(|f| f["field"].as_str().unwrap()).collect();
    assert_eq!(live, ["sessionId", "timestamp", "cwd", "version", "type", "effort", "message.usage.output_tokens_details.thinking_tokens",
        "message.usage.cache_creation.ephemeral_5m_input_tokens", "message.usage.cache_creation.ephemeral_1h_input_tokens", "message.model", "message.id",
        "message.usage.input_tokens", "message.usage.output_tokens", "message.usage.cache_creation_input_tokens",
        "message.usage.cache_read_input_tokens"]);
    assert!(adapter["fields"].as_array().unwrap().iter()
        .filter(|f| f["field"].as_str().unwrap().contains("tool") || f["field"] == "isSidechain")
        .all(|f| f["certified"] != "live"));
    no_secrets(&f);
}

#[test]
fn native_claude_partial_lines_and_truncation_replay() {
    let f = claude();
    let path = transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    let text = fs::read_to_string(&path).unwrap();
    let split = text.find("\n{\"type\":\"assistant\",\"uuid\":\"side-1\"").unwrap() + 1;
    fs::write(&path, &text[..split + 30]).unwrap();
    f.cli("collect");
    assert_eq!(attempt_usage(&f)["total_tokens"], 350);
    assert_eq!(f.sidecar().query_row("SELECT byte_offset FROM collect_offsets", [], |r| r.get::<_, i64>(0)).unwrap(), split as i64);
    fs::write(&path, &text).unwrap();
    f.cli("collect");
    assert_eq!(attempt_usage(&f)["total_tokens"], 407);
    // A replacement/truncation replays zero without double counting native message ids.
    fs::write(&path, &text[..split]).unwrap();
    f.cli("collect");
    fs::write(&path, &text).unwrap();
    f.cli("collect");
    assert_eq!(attempt_usage(&f)["total_tokens"], 407);
    assert_eq!(f.count("codex_quarantine"), 0);
    no_secrets(&f);
}

#[test]
fn native_claude_unbound_and_uncertified_stay_unknown() {
    let f = claude();
    transcript(&f, "unbound", "/tmp/synthetic-unrelated-project", "2.1.286", f.decided + 1000);
    transcript(&f, "too-early", &f.worktree(), "2.1.286", f.decided - 1);
    transcript(&f, "uncertified", &f.worktree(), "2.1.2", f.decided + 1000);
    f.cli("collect");
    assert_eq!(attempt_usage(&f)["reason"], "cli_version_uncertified");
    let db = f.sidecar();
    assert_eq!(db.query_row("SELECT count(*) FROM rollout_sources WHERE binding='unbound'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(db.query_row("SELECT count(*) FROM codex_usage WHERE session_id='claude-code:uncertified' AND accepted=0 AND total_tokens IS NULL AND reason='cli_version_uncertified'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    no_secrets(&f);
}

#[test]
fn codex_and_claude_native_sources_keep_separate_identity() {
    let f = claude();
    // The preceding real Codex attempt retains its own home and canonical binding.
    let codex_home = f.tmp.path().join("codex-home");
    let db = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    let (old, decided): (String, i64) = db.query_row("SELECT attempt_id,decided_unix_ms FROM dispatch_decisions WHERE attempt_id<>?1", [&f.attempt], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let cwd = format!("{}/.state/worktrees/{old}/repo-00", f.project.display());
    f.rollout(&codex_home, SID, &["head.jsonl", "tail.jsonl"], &cwd, decided + 1000, "0.154.0");
    transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    f.cli("collect");
    let usage = f.cli_args(&["usage", "--json"]).0;
    let old_usage = &usage["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == old).unwrap()["usage"];
    assert_eq!(old_usage["input_tokens"], 1500);
    assert_eq!(attempt_usage(&f)["input_tokens"], 382);
    let observed = f.report();
    assert_eq!(observed["metrics"]["M08"]["value"], 1882);
    f.cli_args(&["accounting", "sync"]);
    let aggregated = f.report();
    for id in ["M08", "M09", "M15", "M16", "M17", "M18"] {
        assert_eq!(aggregated["metrics"][id], observed["metrics"][id], "mixed adapters {id}");
    }
    f.cli_args(&["query", "--metric", "M08,M09,M16,M17,M18", "--json"]);
    f.cli_args(&["analytics", "refresh"]);
    assert_eq!(f.cli_args(&["analytics", "rebuild", "--verify"]).0["identical"], true);
    no_secrets(&f);
}

#[test]
fn native_claude_backup_restore_retention_and_tombstones() {
    let f = claude();
    transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let turn_rows = f.count("claude_turn_lines");
    assert!(turn_rows > 0);
    let backup = f.tmp.path().join("synthetic-claude-backup");
    f.cli_args(&["backup", "create", "--out", backup.to_str().unwrap()]);
    for name in ["telemetry.db", "telemetry.db-wal", "telemetry.db-shm"] {
        let _ = fs::remove_file(f.project.join(".state").join(name));
    }
    let restored = f.cli_args(&["backup", "restore", "--from", backup.to_str().unwrap()]).0;
    assert_eq!(restored["rows"]["claude_turn_lines"], turn_rows);
    assert_eq!(f.count("claude_turn_lines"), turn_rows);
    assert_eq!(restored["rows"]["claude_messages"], 2);
    assert_eq!(restored["rows"]["claude_tool_results"], 3);
    assert_eq!(f.cli("collect").0["collected"]["records"], 0);
    assert_eq!(attempt_usage(&f)["total_tokens"], 407);
    f.cancel_reserved();
    f.cli_args(&["accounting", "sync"]);
    f.sidecar().execute("UPDATE rollout_sources SET observed_unix_ms=observed_unix_ms-?1", [91_i64 * 86_400_000]).unwrap();
    let plan = f.cli_args(&["maintenance", "plan", "--json"]).0;
    let digest = plan["plan_digest"].as_str().unwrap();
    f.cli_args(&["maintenance", "apply", "--confirm", digest, "--json"]);
    assert_eq!(f.count("claude_turn_lines"), 0);
    assert_eq!(f.count("claude_messages"), 0);
    assert_eq!(f.count("claude_tool_results"), 0);
    assert_eq!(f.count("codex_usage"), 0);
    for table in ["accounting_usage_totals", "accounting_native_totals",
        "accounting_source_summary", "accounting_tool_summary"] {
        assert_eq!(f.count(table), 0, "retention must purge {table}");
    }
    f.cli("collect");
    assert_eq!(f.count("claude_turn_lines"), 0);
    assert_eq!(f.count("claude_messages"), 0);
    f.cli_args(&["accounting", "sync"]);
    let purged = f.report();
    assert_eq!(purged["metrics"]["M08"]["value"]["reason"], "no_certified_source");
    assert_eq!(purged["metrics"]["M16"]["coverage"]["sessions"], 0);
    f.cli_args(&["backup", "restore", "--from", backup.to_str().unwrap(), "--force"]);
    assert_eq!(f.count("claude_turn_lines"), 0);
    assert_eq!(f.count("claude_messages"), 0);
    assert_eq!(f.count("claude_tool_results"), 0);
    no_secrets(&f);
}

#[test]
fn claude_upgrade_preserves_an_existing_codex_ledger() {
    let f = Fixture::new();
    f.rollout(&f.home, SID, &["head.jsonl", "tail.jsonl"], &f.worktree(), f.decided + 1000, "0.154.0");
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let before = f.cli_args(&["accounting", "entries"]).1;
    // Reconstruct the historical v12 ledger constraints with real collected rows.
    let db = f.sidecar();
    db.execute_batch("CREATE TEMP TABLE saved_entries AS SELECT entry_id,source,session_id,basis,scope,normalization_version,precedence,position,response_id,model,native,input_tokens,cache_read_tokens,new_input_tokens,cache_write_tokens,output_tokens,reasoning_tokens,total_tokens FROM usage_entries;
        CREATE TEMP TABLE saved_dispositions AS SELECT * FROM usage_dispositions;
        DELETE FROM usage_dispositions; DROP TABLE usage_entries;").unwrap();
    db.execute_batch(include_str!("../migrations/telemetry/accounting/0001_usage_ledger.sql")).unwrap();
    db.execute_batch("INSERT INTO usage_entries SELECT * FROM saved_entries;
        INSERT INTO usage_dispositions SELECT * FROM saved_dispositions;
        UPDATE telemetry_streams SET version=12 WHERE stream='accounting';").unwrap();
    drop(db);
    // A writable public command upgrades; every Codex byte visible in the ledger stays.
    f.cli("collect");
    assert_eq!(f.cli_args(&["accounting", "status"]).0["version"], 27);
    assert_eq!(f.cli_args(&["accounting", "entries"]).1, before);
    assert_eq!(attempt_usage(&f)["total_tokens"], 1680);
}

#[test]
fn unreported_claude_tool_outcome_is_unknown_in_its_own_scope() {
    let f = claude();
    let path = transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    fs::write(&path, fs::read_to_string(&path).unwrap().replace("\"is_error\":true,", "")).unwrap();
    f.cli("collect");
    let tools = f.cli_args(&["accounting", "tools", "--json"]).0;
    assert_eq!(tools["sessions"][0]["tools"]["executed"], 3);
    assert_eq!(tools["sessions"][0]["tools"]["failed"], 0);
    assert_eq!(tools["sessions"][0]["tools"]["unknown"], 1);
    let metric = &tools["metrics"]["M17"];
    assert_eq!(metric["value"], "2/2");
    assert_eq!(metric["by_scope"]["claude-code"]["unknown"]["executions"], 1);
    assert_eq!(metric["by_scope"]["command_execution"]["unknown"]["executions"], 0);
    no_secrets(&f);
}

#[test]
fn claude_aggregate_reads_match_replay_and_verified_rebuild() {
    let f = claude();
    // A terminal planted attempt also exercises the maintained diagnostic.
    f.cancel_reserved();
    let terminated = unix_ms();
    rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap().execute(
        "INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('runtime.worker_terminated',?1,2,1,?2)",
        rusqlite::params![f.attempt, json!({"version": 1, "attempt": f.attempt,
            "cause": "cancellation", "observed_unix_ms": terminated}).to_string()]).unwrap();
    transcript(&f, SID, &f.worktree(), "2.1.286", terminated + 1000);
    f.cli("collect");
    let reads = || {
        let report = f.report();
        let metrics: Vec<Value> = ["M08", "M09", "M15", "M16", "M17", "M18"]
            .iter().map(|id| report["metrics"][id].clone()).collect();
        (metrics, report["after_termination"].clone(),
            f.cli_args(&["accounting", "tools", "--json"]).0,
            f.cli_args(&["view", "cost", "--json"]).0["rows"].as_array().unwrap().iter()
                .map(|r| json!({"metric_id": r["metric_id"], "value": r["value"],
                    "coverage": r["coverage"], "status": r["status"], "basis": r["basis"],
                    "digest": r["projection"]["content_digest"]})).collect::<Vec<_>>())
    };
    let replay = reads();
    assert_eq!(replay.0[0]["value"], 382);
    assert_eq!(replay.0[1]["value"], 25);
    assert_eq!(replay.0[3]["executed"]["by_scope"]["claude-code"], 3);
    assert_eq!(replay.0[3]["collaboration"]["sidechain_turns"], 2);
    assert_eq!(replay.0[4]["value"], "2/3");
    assert_eq!(replay.1.as_array().unwrap().len(), 1);
    f.cli_args(&["accounting", "sync"]);
    for table in ["accounting_usage_totals", "accounting_native_totals",
        "accounting_source_summary", "accounting_tool_summary"] {
        assert_eq!(f.count(table), 1, "Claude must populate {table}");
    }
    assert_eq!(reads(), replay, "maintained reads equal original derivation");
    f.cli_args(&["accounting", "reprice"]);
    let cost = f.cli_args(&["accounting", "cost", "--json"]).1;
    let cost_metrics = (f.report()["metrics"]["M12"].clone(), f.report()["metrics"]["M14"].clone());

    f.cli_args(&["query", "--metric", "M08,M09,M12,M14,M16,M17,M18", "--json"]);
    f.cli_args(&["analytics", "refresh"]);
    let pinned = f.cli_args(&["analytics", "snapshot"]).1;
    assert_eq!(f.cli_args(&["analytics", "rebuild", "--verify"]).0["identical"], true);
    f.cli_args(&["analytics", "rebuild"]);
    assert_eq!(f.cli_args(&["analytics", "snapshot"]).1, pinned);
    // A tool-only fixture correction must invalidate both the read summary
    // and analytics dependency even without another usage record.
    f.sidecar().execute("UPDATE claude_tool_results SET is_error=NULL WHERE call_id='call-2'", []).unwrap();
    let corrected = reads();
    assert_eq!(corrected.0[4]["value"], "2/2");
    assert_eq!(corrected.0[4]["by_scope"]["claude-code"]["unknown"]["executions"], 1);
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(reads(), corrected);
    let ledger = f.cli_args(&["accounting", "entries"]).1;
    // Removing the public projection marker forces the full sync derivation.
    f.sidecar().execute("DELETE FROM usage_ledger", []).unwrap();
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(f.cli_args(&["accounting", "entries"]).1, ledger);
    assert_eq!(reads(), corrected);
    assert_eq!(f.cli_args(&["accounting", "cost", "--json"]).1, cost);
    assert_eq!((f.report()["metrics"]["M12"].clone(), f.report()["metrics"]["M14"].clone()), cost_metrics);
    f.cli_args(&["analytics", "refresh"]);
    assert_eq!(f.cli_args(&["analytics", "rebuild", "--verify"]).0["identical"], true);
}

#[test]
fn claude_2_1_286_two_turns_in_lossy_project_directory() {
    let f = claude();
    let cwd = f.worktree();
    assert!(cwd.contains("/.state/"));
    let slug: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    assert!(slug.contains("--state-"));
    let dir = f.home.join(".claude/projects").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let text = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/claude-2.1.286/live-two-turn.jsonl")).unwrap();
    let lines: Vec<String> = text.lines().map(|line| {
        let mut v: Value = serde_json::from_str(line).unwrap();
        // Free-text sentinels exercise every sanitized subtree in the real skeleton.
        v = serde_json::from_str(&v.to_string().replace("<str>", "CLAUDE_SECRET_LIVE")).unwrap();
        if v.get("cwd").is_some() { v["cwd"] = json!(cwd); }
        if v.get("version").is_some() { v["version"] = json!("2.1.286"); }
        if v.get("timestamp").is_some() {
            v["timestamp"] = json!(jiff::Timestamp::from_millisecond(f.decided + 1000).unwrap().to_string());
        }
        v.to_string()
    }).collect();
    fs::write(dir.join("ID000.jsonl"), format!("{}\n", lines.join("
"))).unwrap();
    assert_eq!(f.cli("collect").0["collected"]["records"], 2);
    assert_eq!(f.binding(), ("bound".into(), Some(f.attempt.clone())));
    f.cli_args(&["accounting", "sync"]);
    let ledger = f.cli_args(&["accounting", "entries"]).0;
    let entries = ledger["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for (output, write, read) in [(87, 6928, 0), (41, 151, 6928)] {
        let e = entries.iter().find(|e| e["normalized"]["output_tokens"] == output).unwrap();
        assert_eq!(e["model"], "claude-haiku-4-5-20251001");
        assert_eq!(e["normalized"]["new_input_tokens"], 10);
        assert_eq!(e["normalized"]["cache_write_tokens"], write);
        assert_eq!(e["normalized"]["cache_read_tokens"], read);
    }
    let db = f.sidecar();
    for kind in ["queue-operation", "attachment", "atis-latch", "last-prompt", "cost-state", "mode"] {
        assert!(db.query_row("SELECT count(*) FROM source_observations WHERE json_extract(payload,'$.line_type')=?1 AND json_extract(payload,'$.unmapped_count')>0", [kind], |r| r.get::<_, i64>(0)).unwrap() > 0);
    }
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(f.cli_args(&["accounting", "entries"]).0["entries"].as_array().unwrap().len(), 2);
    // A distinct cwd with the same lossy slug must not inherit the binding.
    let collision = cwd.replace("/.state/", "/_state/");
    let collision_slug: String = collision.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    assert_eq!(dir.file_name().unwrap().to_str().unwrap(), collision_slug);
    fs::write(dir.join("collision.jsonl"), format!("{}\n", json!({"type":"user", "sessionId":"collision", "cwd":collision,
        "version":"2.1.286", "timestamp":jiff::Timestamp::from_millisecond(f.decided + 1000).unwrap().to_string(),
        "message":{"content":"CLAUDE_SECRET_COLLISION"}}))).unwrap();
    f.cli("collect");
    assert_eq!(f.sidecar().query_row("SELECT binding FROM rollout_sources WHERE session_id='claude-code:collision'", [], |r| r.get::<_, String>(0)).unwrap(), "unbound");
    assert_eq!(attempt_usage(&f)["records"], 2);
    no_secrets(&f);
}

#[test]
fn claude_model_identifiers_and_planted_secret_conformance() {
    let f = claude();
    let path = transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    let models = [
        "claude-haiku-4-5-20251001",
        "claude-sonnet-4-5-20250929",
        "claude-opus-4-1",
        "claude-future-9-7-20301231",
        "Bearer sk-ant-CLAUDE_SECRET_MODEL_SPACES",
        "sk-ant-CLAUDE_SECRET_MODEL_TOKEN",
    ];
    let mut lines = String::new();
    for (i, model) in models.iter().enumerate() {
        lines.push_str(&format!("{}\n", json!({"type":"assistant", "sessionId":SID,
            "cwd":f.worktree(), "version":"2.1.286",
            "timestamp":jiff::Timestamp::from_millisecond(f.decided + 1000).unwrap().to_string(),
            "message":{"id":format!("model-{i}"), "model":model,
                "usage":{"input_tokens":10,"output_tokens":1},
                "content":[{"type":"text","text":"CLAUDE_SECRET_MODEL_CONTENT"}]}})));
    }
    fs::write(path, lines).unwrap();
    assert_eq!(f.cli("collect").0["collected"]["records"], 6);
    f.cli_args(&["accounting", "sync"]);
    let ledger = f.cli_args(&["accounting", "entries"]).0;
    let entries = ledger["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 6);
    for model in &models[..4] {
        assert!(entries.iter().any(|entry| entry["model"] == *model), "{ledger}");
    }
    for expected in ["Bearer [redacted]", "[redacted]"] {
        assert!(entries.iter().any(|entry| entry["model"] == expected), "{ledger}");
    }
    let db = f.sidecar();
    for (i, model) in models.iter().take(4).enumerate() {
        let stored: String = db.query_row("SELECT json_extract(payload,'$.model') FROM source_observations WHERE json_extract(payload,'$.message_id')=?1",
            [format!("model-{i}")], |r| r.get(0)).unwrap();
        assert_eq!(stored, *model);
    }
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(f.cli_args(&["accounting", "entries"]).0["entries"].as_array().unwrap().len(), 6);
    no_secrets(&f);
}

#[test]
fn otlp_request_fallback_yields_to_later_native_attempt_source() {
    use herdr_farm::telemetry::otlp;
    for native_first in [false, true] {
        let f = claude();
        f.cli("collect");
        f.cli_args(&["accounting", "sync"]);
        if native_first {
            transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
            f.cli("collect");
        }
        let mut request: Value = serde_json::from_str(&fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/otlp/claude-logs.json")).unwrap().replace("@ATTEMPT@", &f.attempt)).unwrap();
        request["resourceLogs"][0]["resource"]["attributes"].as_array_mut().unwrap().push(json!({"key":"service.version","value":{"stringValue":"2.1.286"}}));
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(otlp::ingest(&f.project, "/v1/logs", &bytes).unwrap(), 2);
        assert_eq!(otlp::ingest(&f.project, "/v1/logs", &bytes).unwrap(), 0);
        f.cli_args(&["accounting", "sync"]);
        if !native_first {
            assert_eq!(attempt_usage(&f)["total_tokens"], 23);
            assert_eq!(accepted_delta_entries(&f.cli_args(&["accounting", "entries"]).0).len(), 1);
            transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
            f.cli("collect");
            assert_eq!(attempt_usage(&f)["total_tokens"], 407, "live read applies precedence before sync");
        }
        f.cli_args(&["accounting", "sync"]);
        assert_eq!(f.cli_args(&["accounting", "status"]).0["sync"]["mode"], "incremental");
        assert_eq!(attempt_usage(&f)["total_tokens"], 407);
        assert_eq!(f.report()["metrics"]["M08"]["value"], 382);
        let ledger = f.cli_args(&["accounting", "entries"]).0;
        assert_eq!(accepted_delta_entries(&ledger).len(), 2);
        let loser = ledger["entries"].as_array().unwrap().iter().find(|e| e["source"] == "otlp:claude-code").unwrap();
        assert_eq!(loser["provenance"][0]["disposition"], "duplicate");
        assert_eq!(loser["provenance"][0]["reason"], "native_surface_precedence");
        let replay = aggregate_read_snapshot(&f);
        verify_aggregate_replay(&f, &replay);
        assert_eq!(f.cli_args(&["accounting", "entries"]).0, ledger);
        no_secrets(&f);
    }
}

#[test]
fn accepted_otlp_cannot_hide_an_uncertified_native_surface() {
    use herdr_farm::telemetry::otlp;
    let f = claude();
    transcript(&f, SID, &f.worktree(), "2.1.99", f.decided + 1000);
    f.cli("collect");
    let mut request: Value = serde_json::from_str(&fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/otlp/claude-logs.json")).unwrap().replace("@ATTEMPT@", &f.attempt)).unwrap();
    request["resourceLogs"][0]["resource"]["attributes"].as_array_mut().unwrap().push(json!({"key":"service.version","value":{"stringValue":"2.1.286"}}));
    otlp::ingest(&f.project, "/v1/logs", &serde_json::to_vec(&request).unwrap()).unwrap();
    assert_eq!(attempt_usage(&f)["reason"], "cli_version_uncertified");
    assert_eq!(f.report()["metrics"]["M08"]["value"]["reason"], "no_certified_source");
    f.cli_args(&["accounting", "sync"]);
    let ledger = f.cli_args(&["accounting", "entries"]).0;
    assert!(accepted_delta_entries(&ledger).is_empty());
    assert_eq!(f.report()["metrics"]["M08"]["value"]["reason"], "no_certified_source");
    assert!(ledger["entries"].as_array().unwrap().iter().any(|e| e["provenance"][0]["reason"] == "native_surface_precedence"));
    f.sidecar().execute("DELETE FROM usage_ledger", []).unwrap();
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(f.cli_args(&["accounting", "entries"]).0, ledger);
    no_secrets(&f);
}

/// DG5: Claude cache writes are priced at the card's `cache_write` rate and
/// refused when the card has none; both stay explicit, never 0. The cards are
/// INVENTED synthetic rates (not provider prices) with product `claude-code`.
#[test]
fn claude_cache_writes_are_priced_by_a_cache_write_rate_or_refused() {
    let f = claude();
    transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    f.cli("collect");
    f.cli_args(&["accounting", "sync"]);
    let card = |id: &str, model: &str, write: bool| {
        let mut rates = vec![json!({"category":"input","rate":"3"}), json!({"category":"cache_read","rate":"0.30"}), json!({"category":"output","rate":"15"})];
        if write { rates.push(json!({"category":"cache_write","rate":"3.75"})); }
        let path = f.tmp.path().join(format!("{id}.json"));
        fs::write(&path, serde_json::to_vec(&json!({"card_id":id,"version":1,"provider":"anthropic","product":"claude-code","models":[model],
            "currency":"USD","rate_unit":1000000,"effective_from_unix_ms":0,"effective_to_unix_ms":null,
            "includes":{"discounts":false,"taxes":false,"fees":false},"source":"INVENTED synthetic test rates; not a provider price","rates":rates})).unwrap()).unwrap();
        f.cli_args(&["accounting", "import-rate-card", path.to_str().unwrap()]);
    };
    card("synthetic-claude-sonnet", "claude-fixture-sonnet", true);
    card("synthetic-claude-haiku", "claude-fixture-haiku", false);
    f.cli_args(&["accounting", "reprice"]);
    let cost = f.cli_args(&["accounting", "cost", "--json"]).0;
    let entries: Vec<Value> = cost["sessions"].as_array().unwrap().iter().flat_map(|s| s["entries"].as_array().unwrap().clone()).collect();
    assert_eq!(entries.len(), 2);
    let by_model = |model: &str| entries.iter().filter(|e| e["model"] == model).map(|e| e["valuation"].clone()).collect::<Vec<_>>();
    // 100 × 3 + 200 × 0.30 + 30 × 3.75 + 20 × 15 per 10^6, disjoint quantities.
    let priced = json!({"status":"priced","basis":"published_rate_estimate","rate_card":{"card_id":"synthetic-claude-sonnet","version":1},
        "currency":"USD","amount":"0.0007725","provider_check":"matched",
        "components":{"input":"0.0003","cache_read":"0.00006","cache_write":"0.0001125","output":"0.0003"}});
    let sonnet = by_model("claude-fixture-sonnet");
    assert_eq!(sonnet.len(), 1);
    for v in &sonnet {
        for key in ["status", "basis", "rate_card", "currency", "amount", "components"] { assert_eq!(v[key], priced[key], "{v}"); }
    }
    for v in by_model("claude-fixture-haiku") {
        assert_eq!((&v["status"], &v["reason"]), (&json!("unavailable"), &json!("cache_write_convention_unknown")), "{v}");
    }
    let session = &cost["sessions"][0];
    assert_eq!(session["coverage"], json!({"entries": 2, "priced": 1, "unpriced": {"cache_write_convention_unknown": 1}}));
    assert_eq!(session["estimate"], json!({"status":"partial","reason":"unpriced_entries","currency":"USD","priced_amount":"0.0007725"}));
    let m12 = f.report()["metrics"]["M12"].clone();
    assert_eq!(m12["coverage"], json!({"entries": 2, "priced": 1, "unpriced": {"cache_write_convention_unknown": 1}}), "{m12}");
    let again = f.cli_args(&["accounting", "reprice"]).0;
    assert_eq!(again["appended"], false, "repricing is idempotent: {again}");
    assert_eq!(f.cli_args(&["accounting", "cost", "--json"]).0, cost);
}

#[test]
fn newer_claude_usage_and_historical_refusal_are_recollected() {
    let f = claude();
    transcript(&f, SID, &f.worktree(), "2.1.300", f.decided + 1000);
    f.cli("collect");
    let usage = attempt_usage(&f);
    assert_eq!(usage["total_tokens"], 407);
    assert_eq!(usage["certification"], "newer_than_certified");
    assert_eq!(usage["nearest_certified_version"], "2.1.286");
    for command in [vec!["collect"], vec!["accounting", "sync"]] {
        f.sidecar().execute_batch("UPDATE codex_usage SET accepted=0,reason='cli_version_uncertified',cache_write_input_tokens=NULL,cached_input_tokens=NULL,input_tokens=NULL,output_tokens=NULL,reasoning_output_tokens=NULL,total_tokens=NULL;").unwrap();
        assert_eq!(attempt_usage(&f)["reason"], "cli_version_uncertified");
        f.cli_args(&command);
        assert_eq!(attempt_usage(&f), usage);
        assert_eq!(f.count("codex_usage"), 2);
    }
    plant_aggregate_termination(&f);
    assert_eq!(f.report()["metrics"]["M13"]["coverage"]["newer_than_certified"], 1);
    assert_eq!(f.report()["metrics"]["M15"]["coverage"]["newer_than_certified"], 1);
    no_secrets(&f);
}

#[test]
fn newer_claude_schema_drift_is_never_partial_usage() {
    for missing in [false, true] {
        let f = claude();
        let path = transcript(&f, SID, &f.worktree(), "2.1.300", f.decided + 1000);
        let mut changed = false;
        let lines: Vec<String> = fs::read_to_string(&path).unwrap().lines().map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            if !changed && value["type"] == "assistant" && value["message"]["usage"].is_object() {
                changed = true;
                if missing { value["message"]["usage"].as_object_mut().unwrap().remove("output_tokens"); }
                else { value["message"]["usage"]["output_tokens"] = json!("changed"); }
            }
            value.to_string()
        }).collect();
        assert!(changed);
        fs::write(path, format!("{}\n", lines.join("
"))).unwrap();
        f.cli("collect");
        assert_eq!(attempt_usage(&f)["reason"], "schema_unrecognized");
        assert_eq!(attempt_usage(&f)["cli_version"], "claude-code/2.1.300");
        assert!(attempt_usage(&f).get("total_tokens").is_none());
        assert!(f.sidecar().query_row("SELECT EXISTS(SELECT 1 FROM source_observations WHERE json_extract(measurement,'$.reason')='schema_unrecognized' AND json_extract(measurement,'$.coverage')='unavailable' AND json_extract(provenance,'$.adapter_version')='claude-code/2.1.300')", [], |r| r.get::<_, bool>(0)).unwrap());
        f.cli_args(&["accounting", "sync"]);
        assert_eq!(attempt_usage(&f)["reason"], "schema_unrecognized");
        no_secrets(&f);
    }
}

#[test]
fn owner_claude_coordinator_is_scoped_private_and_optional() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::{io::Read, os::fd::{AsRawFd, FromRawFd}};
    for enabled in [true, false] {
        let mut f = claude();
        fs::write(f.project.join("PROJECT.md"), "+++\ncoordinator_agent = 'claude'\n+++\n").unwrap();
        let owner = f.tmp.path().join("home");
        let config = owner.join(".config/herdr-farm");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("config.toml"), if enabled { "[telemetry]\n" } else { "[telemetry]\ncollect_coordinator_usage = false\n" }).unwrap();
        transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
        f.cli("collect");
        let worker = attempt_usage(&f);
        let m35 = f.cli_args(&["accounting", "fleet", "--json"]).0["metrics"]["M35"].clone();
        let execution_home = f.home.clone();
        f.home = owner.clone();
        let cwd = f.project.display().to_string();
        let path = transcript(&f, "owner-coordinator", &cwd, "2.1.286", f.decided + 1000);
        // Different session and API ids preserve two independent observations.
        let text = fs::read_to_string(&path).unwrap().replace("msg-", "owner-msg-");
        fs::write(&path, text).unwrap();
        let sibling = transcript(&f, "sibling-sentinel", "/synthetic/sibling", "2.1.286", f.decided + 1000);
        fs::set_permissions(&sibling, fs::Permissions::from_mode(0o000)).unwrap();
        let credentials = owner.join(".claude/credentials.json");
        fs::write(&credentials, "CLAUDE_SECRET_CREDENTIAL_SENTINEL").unwrap();
        // Watch forbidden paths for real opens, so a collector silently skipping
        // unreadable files cannot hide a boundary violation.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0);
        let mut forbidden_opens = unsafe { fs::File::from_raw_fd(fd) };
        for watched in [sibling.parent().unwrap(), &credentials] {
            let name = std::ffi::CString::new(watched.as_os_str().as_encoded_bytes()).unwrap();
            assert!(unsafe { libc::inotify_add_watch(forbidden_opens.as_raw_fd(), name.as_ptr(), libc::IN_OPEN) } >= 0);
        }
        // A linked source inside the permitted directory must also be ignored.
        symlink(&sibling, path.parent().unwrap().join("linked.jsonl")).unwrap();
        f.home = execution_home;
        f.cli("collect");
        let mut events = [0u8; 4096];
        assert_eq!(forbidden_opens.read(&mut events).unwrap_err().kind(), std::io::ErrorKind::WouldBlock,
            "sibling projects and credentials must never be opened");
        assert_eq!(attempt_usage(&f), worker, "coordinator does not enter worker totals");
        assert_eq!(f.sidecar().query_row("SELECT count(*) FROM rollout_sources WHERE session_id='claude-code:sibling-sentinel'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        f.cli_args(&["accounting", "sync"]);
        let card = f.tmp.path().join("coordinator-rates.json");
        fs::write(&card, json!({"card_id":"synthetic-coordinator","version":1,"provider":"anthropic","product":"claude-code",
            "models":["claude-fixture-sonnet","claude-fixture-haiku"],"currency":"USD","rate_unit":1000000,"effective_from_unix_ms":0,
            "includes":{"discounts":false,"taxes":false,"fees":false},"source":"INVENTED synthetic fixture rates",
            "rates":[{"category":"input","rate":"1"},{"category":"output","rate":"1"},{"category":"cache_read","rate":"1"},{"category":"cache_write","rate":"1"}]}).to_string()).unwrap();
        f.cli_args(&["accounting", "import-rate-card", card.to_str().unwrap()]);
        f.cli_args(&["accounting", "reprice"]);
        let fleet = f.cli_args(&["accounting", "fleet", "--json"]).0;
        assert_eq!(fleet["metrics"]["M35"], m35);
        let m34 = &fleet["metrics"]["M34"];
        assert_eq!(m34["scope"], "coordinator-scope-v2");
        if enabled {
            assert_eq!(m34["coordinator"]["sessions"], 1);
            assert_eq!(m34["coordinator"]["coverage"]["entries"], 2);
            assert_eq!(m34["value"], "1/2");
            assert_eq!(f.sidecar().query_row("SELECT binding FROM rollout_sources WHERE session_id='claude-code:owner-coordinator'", [], |r| r.get::<_, String>(0)).unwrap(), "unbound");
        } else {
            assert_eq!(m34["value"]["reason"], "coordinator_usage_not_observed");
        }
        let cost = f.cli_args(&["view", "cost", "--json"]).0;
        let row = cost["rows"].as_array().unwrap().iter().find(|r| r["metric_id"] == "M34").unwrap();
        assert_eq!(row["value"], fleet["metrics"]["M34"]["value"]);
        no_secrets(&f);
        fs::write(f.project.join(".state/format.json"), r#"{"memory":"sqlite-v1","runtime":"sqlite-v1"}"#).unwrap();
        let doctor = std::process::Command::new(BIN).env_clear().env("HOME", &owner).env("PATH", "/usr/bin:/bin")
            .env("HERDR_FARM_TEST_TIME_SCALE", f.scale).args(["--root", f.root.to_str().unwrap(), "doctor"]).output().unwrap();
        let output = String::from_utf8(doctor.stdout).unwrap();
        assert!(output.contains(&format!("coordinator usage: {} ({})", if enabled { "collected" } else { "disabled" }, path.parent().unwrap().display())), "{output}");
    }
}

/// Public collection, attempts, ledger and pricing over one isolated native transcript.
#[test]
fn claude_v2_thinking_effort_and_cache_tiers_survive_rebuild() {
    let f = claude_with_pins(Some("medium"));
    let path = transcript(&f, SID, &f.worktree(), "2.1.286", f.decided + 1000);
    let lines: Vec<String> = fs::read_to_string(&path).unwrap().lines().map(|line| {
        let mut v: Value = serde_json::from_str(line).unwrap();
        if v["type"] == "assistant" && v["message"]["usage"].is_object() {
            v["effort"] = json!("high");
            v["message"]["usage"]["output_tokens_details"] = json!({"thinking_tokens":3});
            let writes = v["message"]["usage"]["cache_creation_input_tokens"].as_i64().unwrap();
            v["message"]["usage"]["cache_creation"] = json!({"ephemeral_5m_input_tokens":writes-1,"ephemeral_1h_input_tokens":1});
        }
        v.to_string()
    }).collect();
    fs::write(&path, format!("{}\n", lines.join("
"))).unwrap();
    f.cli("collect");
    assert_eq!(attempt_usage(&f)["reasoning_output_tokens"], 6);
    assert_eq!(f.report()["metrics"]["M09"]["reasoning_output_tokens"], 6);
    let attempts = f.cli_args(&["attempts", "--json"]).0;
    let attempt = attempts["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == f.attempt).unwrap();
    assert_eq!(attempt["effort_observed"], "high");
    assert_eq!(attempt["reasoning_effort"], "medium");
    f.cli_args(&["accounting", "sync"]);
    let ledger = f.cli_args(&["accounting", "entries"]).0;
    assert_eq!(ledger["entries"].as_array().unwrap().iter().map(|e| e["normalized"]["cache_write_5m_tokens"].as_i64().unwrap()).sum::<i64>(), 30);
    assert_eq!(ledger["entries"].as_array().unwrap().iter().map(|e| e["normalized"]["cache_write_1h_tokens"].as_i64().unwrap()).sum::<i64>(), 2);
    for (version, rates, expected) in [
        (1, json!([{"category":"input","rate":"3"},{"category":"output","rate":"15"},{"category":"cache_read","rate":"0.3"},{"category":"cache_write","cache_tier":"5m","rate":"3.75"},{"category":"cache_write","cache_tier":"1h","rate":"6"}]), "0.0009015"),
        (2, json!([{"category":"input","rate":"3"},{"category":"output","rate":"15"},{"category":"cache_read","rate":"0.3"},{"category":"cache_write","rate":"3.75"}]), "0.000897"),
    ] {
        let card = f.tmp.path().join("tier-card.json");
        fs::write(&card, json!({"card_id":"synthetic-tiers","version":version,"provider":"anthropic","product":"claude-code",
            "models":["claude-fixture-sonnet","claude-fixture-haiku"],"currency":"USD","rate_unit":1000000,"effective_from_unix_ms":0,
            "includes":{"discounts":false,"taxes":false,"fees":false},"source":"INVENTED synthetic rates", "rates":rates}).to_string()).unwrap();
        f.cli_args(&["accounting", "import-rate-card", card.to_str().unwrap()]);
        f.cli_args(&["accounting", "reprice"]);
        let cost = f.cli_args(&["accounting", "cost", "--json"]).0;
        assert_eq!(cost["sessions"][0]["estimate"]["amount"], expected, "{cost}");
    }
    f.cli("collect");
    assert_eq!(f.count("codex_quarantine"), 0);
    // Reconstruct the persisted v1 layout and old digest after real collection.
    // Upgrade through the CLI must discard only disposable mappings and replay.
    let db = f.sidecar();
    db.execute_batch("ALTER TABLE codex_usage DROP COLUMN cache_write_5m_tokens;
        ALTER TABLE codex_usage DROP COLUMN cache_write_1h_tokens;
        ALTER TABLE codex_usage DROP COLUMN reasoning_reported;
        UPDATE codex_usage SET effort=NULL,reasoning_output_tokens=0,payload_digest='sha256:v1';").unwrap();
    let old_envelopes = db.prepare("SELECT producer_epoch,producer_sequence,payload FROM source_observations WHERE event_kind='claude-code.claude_line.v1'").unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert!(!old_envelopes.is_empty());
    for (source, sequence, payload) in old_envelopes {
        let mut payload: Value = serde_json::from_str(&payload).unwrap();
        for key in ["effort", "thinking_tokens", "cache_write_5m_tokens", "cache_write_1h_tokens"] { payload.as_object_mut().unwrap().remove(key); }
        let payload = payload.to_string();
        let digest = format!("sha256:{:x}", <sha2::Sha256 as sha2::Digest>::digest(payload.as_bytes()));
        db.execute("UPDATE source_observations SET payload=?3,payload_digest=?4,measurement=json_set(measurement,'$.normalization_version',1) WHERE producer_epoch=?1 AND producer_sequence=?2",
            rusqlite::params![source, sequence, payload, digest]).unwrap();
    }
    db.execute("UPDATE telemetry_streams SET version=13 WHERE stream='ingest'", []).unwrap();
    drop(db);
    f.cli("collect");
    assert_eq!(f.count("codex_quarantine"), 0);
    assert_eq!(f.count("ingest_quarantine"), 0);
    assert_eq!(attempt_usage(&f)["reasoning_output_tokens"], 6);
    f.cli_args(&["accounting", "sync"]);
    assert_eq!(f.cli_args(&["accounting", "entries"]).0, ledger);
    no_secrets(&f);
}

#[test]
fn claude_turn_metadata_workers_and_coordinator_detail() {
    let f = claude();
    let path = transcript(&f, SID, &f.worktree(), "2.1.286", f.decided+1000);
    let fixture = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/telemetry/claude-code/turns.jsonl")).unwrap();
    let render = |sid: &str, cwd: &str| fixture.replace("@SID@",sid).replace("@CWD@",cwd)
        .replace("@TS@",&jiff::Timestamp::from_millisecond(f.decided+1000).unwrap().to_string());
    fs::write(&path,render(SID,&f.worktree())).unwrap();
    f.cli("collect");
    let attempts = f.cli_args(&["attempts","--json"]).0;
    let a = attempts["attempts"].as_array().unwrap().iter().find(|a| a["attempt_id"] == f.attempt).unwrap();
    assert_eq!(f.report()["metrics"]["M80"]["value"]["reason"], "coordinator_turn_metadata_missing");
    assert_eq!(a["turns"],8);
    assert_eq!(a["stop_reasons"]["end_turn"],8);
    let db = f.sidecar();
    let invalid: String = db.query_row("SELECT json_extract(metadata,'$.turn_origin') FROM claude_turn_lines WHERE json_extract(metadata,'$.prompt_id')='prompt-7'",[],|r|r.get(0)).unwrap();
    assert_eq!(invalid,"other");
    assert_eq!(f.count("claude_turn_lines"),32);
    f.cli("collect");
    assert_eq!(f.count("claude_turn_lines"),32);
    no_secrets(&f);
    // A project-root transcript is a coordinator session, isolated from workers.
    let owner_path = transcript(&f,"turn-coordinator",&f.project.display().to_string(),"2.1.286",f.decided+1000);
    let mut rows: Vec<Value> = render("turn-coordinator",&f.project.display().to_string()).lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    for (index, r) in rows.iter_mut().enumerate() {
        r["timestamp"] = json!(jiff::Timestamp::from_millisecond(f.decided+1000+index as i64*1000).unwrap().to_string());
        if r["type"] == "assistant" {
            r["message"]["usage"]["cache_read_input_tokens"] = json!(0);
            r["message"]["usage"]["cache_creation_input_tokens"] = json!(50);
            r["message"]["usage"]["cache_creation"]["ephemeral_5m_input_tokens"] = json!(45);
        }
        if r["type"] == "assistant" && r["message"]["id"] == "turn-msg-3" {
            r["message"]["content"].as_array_mut().unwrap().retain(|b| b["type"] != "tool_use");
        }
        if r["type"] == "assistant" && r["message"]["id"] == "turn-msg-0" {
            r["message"]["content"][1]["name"] = json!("AskUserQuestion");
        }
    }
    // Isolate tag classification from previous background-result evidence.
    for r in &mut rows { if r["type"] == "user" && r["toolUseResult"].is_object() { r["toolUseResult"].as_object_mut().unwrap().remove("backgroundTaskId"); } }
    fs::write(&owner_path,format!("{}\n",rows[..8].iter().map(Value::to_string).collect::<Vec<_>>().join("\n"))).unwrap();
    f.cli("collect");
    let early = f.cli_args(&["accounting","coordinator"]).0;
    assert_eq!(early["sessions"][0]["turns"].as_array().unwrap().len(),2);
    assert_eq!(early["sessions"][0]["turns"][0]["context_tokens_sum"],60);
    assert_eq!(early["sessions"][0]["turns"][0]["unpriced_requests"],1);
    assert_eq!(f.report()["metrics"]["M81"]["value"]["unpriced_requests"], 2);
    assert_eq!(early["sessions"][0]["turns"][1]["idle_gap_ms"],1000);
    fs::write(owner_path,format!("{}\n",rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"))).unwrap();
    // Worker execution homes are scanned but the root cwd still scopes coordinator.
    f.cli("collect");
    f.cli_args(&["accounting","sync"]);
    let card = f.tmp.path().join("turn-rates.json");
    fs::write(&card,json!({"card_id":"turn-synthetic","version":1,"provider":"anthropic","product":"claude-code",
        "models":["claude-fixture-sonnet"],"currency":"USD","rate_unit":1000000,"effective_from_unix_ms":0,
        "includes":{"discounts":false,"taxes":false,"fees":false},"source":"INVENTED fixture rates",
        "rates":[{"category":"input","rate":"1"},{"category":"output","rate":"1"},{"category":"cache_read","rate":"1"},{"category":"cache_write","rate":"1"}]}).to_string()).unwrap();
    f.cli_args(&["accounting","import-rate-card",card.to_str().unwrap()]);
    f.cli_args(&["accounting","reprice"]);
    let fleet = f.cli_args(&["accounting","fleet","--json"]).0;
    let view = &fleet["metrics"]["M34"]["coordinator"]["turn_sessions"][0];
    let direct = f.cli_args(&["accounting","coordinator"]).0;
    assert_eq!(direct["sessions"][0],*view);
    assert_eq!(direct["summary"]["turns"],8);
    assert_eq!(direct["summary"]["requests"],8);
    assert_eq!(direct["summary"]["cost_by_trigger_class"]["other"]["USD"],"0.000124");
    let turns = view["turns"].as_array().unwrap();
    assert_eq!(turns.len(),8,"{fleet}");
    for (t,trigger) in turns.iter().zip(["owner_typed","queued_owner","background_task","scheduled_wakeup","subagent","bootstrap","other","other"]) {
        assert_eq!(t["trigger_class"],trigger);
        assert_eq!(t["requests"],1);
        assert_eq!(t["context_tokens_max"],60);
        assert_eq!(t["context_tokens_sum"],60);
        assert_eq!(t["cache_write_5m_tokens"],45);
        assert_eq!(t["cache_write_1h_tokens"],5);
        assert_eq!(t["wall_duration_ms"],123);
        assert_eq!(t["cost_by_currency"]["USD"],"0.000062");
        assert_eq!(t["full_context_cache_rewrite"],true);
    }
    assert_eq!(turns[0]["ask_user_question_wait_ms"],1000);
    assert_eq!(turns[1]["idle_gap_ms"],1000);
    assert_eq!(view["no_tool_call_turns"].as_array().unwrap().len(),1);
    assert_eq!(view["no_tool_call_turns"][0]["trigger_class"],"scheduled_wakeup");
    assert_eq!(view["cost_by_trigger_class"]["other"]["USD"],"0.000124");
    assert_eq!(view["idle_gaps_before_cache_rewrites"].as_array().unwrap().len(),7);
    let metrics = f.report()["metrics"].clone();
    assert_eq!(metrics["M80"]["value"], json!({"samples":8,"median":60,"p90":60,"max":60,"total":480}));
    assert_eq!(metrics["M81"]["value"]["by_currency"]["USD"], "0.000496");
    assert_eq!(metrics["M81"]["rewrites"].as_array().unwrap().len(), 8);
    assert_eq!(metrics["M82"]["value"]["other"]["by_currency"]["USD"], "0.000124");
    assert_eq!(metrics["M82"]["no_tool_call_turns"], 1);
    assert_eq!(metrics["M82"]["no_tool_call_cost"]["by_currency"]["USD"], "0.000062");
    assert_eq!(metrics["M83"]["value"]["ratio"], "1/1");
    assert_eq!(metrics["M84"]["ask_user_question_answer_time_ms"]["median"], 1000);
    assert_eq!(metrics["M84"]["value"]["reason"], "no_timed_samples");
    assert_eq!(metrics["M85"]["prompts"], 1);
    assert_eq!(metrics["M85"]["value"]["reason"], "empty_denominator");
    assert_eq!(metrics["M86"]["value"]["reason"], "no_timed_samples");
    assert_eq!(metrics["M87"]["value"]["reason"], "empty_denominator");
    let next_owner = json!({"sessionId":"turn-coordinator","cwd":f.project.display().to_string(),
        "version":"2.1.286","type":"user","turnOrigin":"owner_typed","promptSource":"owner_typed",
        "timestamp":jiff::Timestamp::from_millisecond(f.decided+40000).unwrap().to_string(),
        "message":{"content":[{"type":"text","text":"CLAUDE_SECRET_NEXT_OWNER"}]}});
    let owner_path = transcript(&f,"turn-coordinator",&f.project.display().to_string(),"2.1.286",f.decided+1000);
    let text = rows.iter().chain(std::iter::once(&next_owner)).map(Value::to_string).collect::<Vec<_>>().join("\n");
    fs::write(&owner_path,format!("{text}\n")).unwrap();
    f.cli("collect");
    let metrics = f.report()["metrics"].clone();
    assert_eq!(metrics["M83"]["value"]["ratio"], "2/2");
    assert_eq!(metrics["M84"]["value"], json!({"samples":1,"median":36000,"p90":36000,"max":36000,"total":36000}));
    assert_eq!(metrics["M86"]["launches_per_owner_turn"]["ratio"], "0/2");
    let canonical = rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();

    // Synthetic lifecycle and acceptance records are inputs; assertions go
    // through the real report CLI and exercise joins to coordinator timestamps.
    canonical.execute("INSERT INTO attempt_lifecycle VALUES(?1,'running',2,?2,'fixture')",
        rusqlite::params![f.attempt,f.decided+5000]).unwrap();
    canonical.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    canonical.execute("INSERT INTO task_contracts(task_id,contract_revision,plan_revision,project_store,expected_head,repository,base_oid,object_format,memory_snapshot_id,route,raw_bytes,raw_digest,installed_seq)
        VALUES('work',1,NULL,'store',0,'/repo',?1,'sha1',NULL,'verify_only',x'61',?2,(SELECT max(sequence) FROM events))",rusqlite::params!["b".repeat(40),"c".repeat(64)]).unwrap();
    canonical.execute("INSERT INTO result_submissions(submission_id,project_store,idempotency_key,payload_digest,payload,task_id,contract_revision,contract_digest,attempt_id,repository,base_oid,candidate_oid,object_format,artifact_manifest,claimed_checks,created_unix_ms)
        VALUES(?1,'store',?1,?2,'{}','work',1,?2,?3,'/repo',?4,?4,'sha1','[]','[]',?5)",rusqlite::params!["1".repeat(64),"d".repeat(64),f.attempt,"b".repeat(40),f.decided+35000]).unwrap();
    canonical.execute("INSERT INTO verified_results(result_id,run_id,submission_id,commit_oid,tree_oid,object_format,policy_digest,receipt_digest,isolation,memory_fence,created_unix_ms)
        VALUES(?1,?1,?2,?3,?3,'sha1',?4,?4,'linux-unshare-user-pid-mount-v1',0,?5)",rusqlite::params!["2".repeat(64),"1".repeat(64),"b".repeat(40),"7".repeat(64),f.decided+36000]).unwrap();
    let metrics=f.report()["metrics"].clone();
    assert_eq!(metrics["M85"]["value"]["ratio"],"1/1");
    assert_eq!(metrics["M86"]["value"]["max"],35000);
    assert_eq!(metrics["M86"]["launches_per_owner_turn"]["ratio"],"1/2");
    assert_eq!(metrics["M87"]["value"]["ratio"],"2/1");
    canonical.execute("INSERT INTO owner_requests(id,action,task,contract_digest,repository,summary,expires,status) VALUES('fixture-permission','cap','work',?1,'/repo','synthetic permission',?2,'pending')",rusqlite::params!["c".repeat(64),unix_ms()+100000]).unwrap();
    canonical.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('owner.requested','fixture-permission',1,1,'{}')",[]).unwrap();
    let prompts=f.report()["metrics"]["M85"].clone();
    assert_eq!(prompts["prompts"],2);
    assert_eq!(prompts["value"]["ratio"],"2/1");
    let query=f.cli_args(&["query","--json","--metric","M80,M87"]).0;
    assert_eq!(query["results"][0]["detail"]["value"],metrics["M80"]["value"]);
    assert_eq!(query["results"][1]["detail"]["value"],metrics["M87"]["value"]);
    assert_eq!(f.cli_args(&["accounting","coordinator"]).0["metrics"]["M87"]["value"],metrics["M87"]["value"]);

    canonical.execute_batch("DROP TRIGGER attempt_lifecycle_no_delete; DELETE FROM attempt_lifecycle WHERE state='reserved'").unwrap();
    let incomplete=f.report()["metrics"].clone();
    assert_eq!(incomplete["M83"]["value"]["reason"],"historical_activity_times_missing");
    assert_eq!(incomplete["M86"]["launches_per_owner_turn"]["reason"],"historical_activity_times_missing");
    canonical.execute_batch("DROP TRIGGER events_record_time; DROP TABLE event_times").unwrap();
    assert_eq!(f.report()["metrics"]["M83"]["value"]["reason"], "event_times_not_recorded");


    no_secrets(&f);
}

#[test]
fn coordinator_request_percentiles_and_idle_owner_waits() {
    use herdr_farm::{domain::AttemptId, store::SqliteStore};
    let f = claude();
    let mut store = SqliteStore::open(&f.project.join(".state/state.db")).unwrap();
    let snapshot = store.read_snapshot(None).unwrap();
    let attempt = snapshot.attempts.iter().find(|a|a.id.as_str()==f.attempt).unwrap();
    store.cancel_attempt(&AttemptId::new(&f.attempt).unwrap(),attempt.revision,snapshot.head,"fixture complete",f.decided+500).unwrap();
    drop(store);
    let cwd=f.project.display().to_string();
    let path=transcript(&f,"percentiles",&cwd,"2.1.286",f.decided+1000);
    let mut rows=Vec::new();
    for (i,context) in [10,20,100].into_iter().enumerate() {
        for (offset,kind) in [(0,"user"),(100,"assistant")] {
            let mut row=json!({"sessionId":"percentiles","cwd":cwd,"version":"2.1.286","type":kind,
                "timestamp":jiff::Timestamp::from_millisecond(f.decided+(i as i64+1)*1000+offset).unwrap().to_string()});
            if kind=="user" {row["turnOrigin"]=json!("owner_typed"); row["message"]=json!({"content":"CLAUDE_SECRET_PROMPT"});}
            else {row["requestId"]=json!(format!("r-{i}"));row["message"]=json!({"id":format!("m-{i}"),"model":"claude-fixture-sonnet","stop_reason":"end_turn",
                "usage":{"input_tokens":context-5,"cache_read_input_tokens":5,"cache_creation_input_tokens":0,"output_tokens":1},"content":[]});}
            rows.push(row.to_string());
        }
    }
    fs::write(path,format!("{}\n",rows.join("\n"))).unwrap();
    f.cli("collect");
    let metrics=f.report()["metrics"].clone();
    assert_eq!(metrics["M80"]["value"],json!({"samples":3,"median":20,"p90":100,"max":100,"total":130}));
    assert_eq!(metrics["M81"]["rewrites"],json!([]));
    assert_eq!(metrics["M82"]["no_tool_call_turns"],3);
    assert_eq!(metrics["M82"]["value"]["owner_typed"]["unpriced_requests"],3);
    assert_eq!(metrics["M83"]["value"]["ratio"],"0/3");
    assert_eq!(metrics["M84"]["value"],json!({"samples":2,"median":900,"p90":900,"max":900,"total":1800}));
    assert_eq!(metrics["M84"]["open_censored"],1);
    assert_eq!(metrics["M85"]["prompts"],0);
    let canonical=rusqlite::Connection::open(f.project.join(".state/state.db")).unwrap();
    canonical.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.delivered','worker-result-fixture',1,1,'{}')",[]).unwrap();
    let delivered:i64=canonical.query_row("SELECT recorded_unix_ms FROM event_times ORDER BY sequence DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    let next=unix_ms().max(f.decided+3100)+1000;
    rows.push(json!({"sessionId":"percentiles","cwd":cwd,"version":"2.1.286","type":"user","turnOrigin":"owner_typed",
        "timestamp":jiff::Timestamp::from_millisecond(next).unwrap().to_string(),"message":{"content":"CLAUDE_SECRET_NOTICE_PULL"}}).to_string());
    let path=transcript(&f,"percentiles",&cwd,"2.1.286",f.decided+1000);
    fs::write(&path,format!("{}\n",rows.join("\n"))).unwrap();
    f.cli("collect");
    let metrics=f.report()["metrics"].clone();
    let pulled=(1..=3).filter(|i|f.decided+i*1000>=delivered).count()+1;
    assert_eq!(metrics["M83"]["value"]["ratio"],format!("{pulled}/4"));
    assert_eq!(metrics["M86"]["value"]["max"],next-delivered.max(f.decided+3000));
    canonical.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.seen','worker-result-fixture',2,1,'{}')",[]).unwrap();
    let seen:i64=canonical.query_row("SELECT recorded_unix_ms FROM event_times ORDER BY sequence DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    // Recollect the final owner turn after the actual seen mark; metadata
    // replay must consume the notice and keep the preceding owner count.
    let final_time=seen.max(next)+1000;
    let mut owner:Value=serde_json::from_str(rows.last().unwrap()).unwrap();
    owner["timestamp"]=json!(jiff::Timestamp::from_millisecond(final_time).unwrap().to_string());
    rows.push(owner.to_string());
    fs::write(path,format!("{}\n",rows.join("\n"))).unwrap();
    f.cli("collect");
    let metrics=f.report()["metrics"].clone();
    // The transcript uses future fixture times, so a real seen event can
    // precede both appended owner turns. Replaying both is deterministic.
    let expected=(1..=3).filter(|i|f.decided+i*1000>=delivered && f.decided+i*1000<seen).count()+usize::from(next<seen);
    assert_eq!(metrics["M83"]["value"]["ratio"],format!("{expected}/5"));
    no_secrets(&f);
}
