//! Retained legacy evidence is data, never a live route or an execution request.
use super::*;
use serde_json::Value;

// Memory Markdown stays backup-only: the separate memory importer owns its
// provenance and signed cutover. Review obligations retain their legacy links.
pub(super) fn retained(path: &str) -> bool {
    path.starts_with(".state/artifacts/")
        || path.starts_with(".state/memory-review-evidence/")
        || path.starts_with(".state/resolved-cleanup-") && path.ends_with(".json")
        || path.starts_with("threads/") && path.ends_with(".md")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupJournal {
    version: u32,
    generation: u64,
    execution: String,
    step: String,
    #[serde(rename = "note")]
    _note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    thread: String,
    generation: u64,
    source: String,
    #[serde(default)]
    machine: String,
    entries: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    directory: bool,
    bytes: u64,
    sha256: String,
}

pub(super) fn validate(project: &Path, source: &Source, bytes: &[u8]) -> Result<()> {
    if let Some(id) = source.path.strip_prefix(".state/resolved-cleanup-").and_then(|s| s.strip_suffix(".json")) {
        ensure!(safe_relative(id) && !id.contains('/'), "invalid cleanup thread identity");
        let journal: CleanupJournal = serde_json::from_slice(bytes)?;
        let record: toml::Value = toml::from_str(std::str::from_utf8(&read(&safe_join(project, &format!("threads/{id}.toml"))?)?)?)?;
        ensure!(record.get("id").and_then(|v| v.as_str()) == Some(id) && record.get("machine").and_then(|v| v.as_str()).unwrap_or("").is_empty() && journal.version == 1
            && Some(journal.generation) == record.get("lifecycle_generation").and_then(|v| v.as_integer()).map(|n| n as u64)
            && journal.execution.len() == 64 && journal.execution.bytes().all(|b| b.is_ascii_hexdigit())
            && matches!(journal.step.as_str(), "ready" | "stop" | "pane" | "workspace" | "remove" | "complete" | "skipped"), "invalid cleanup journal");
        let mut execution_record = record.clone();
        if let Some(removal) = record.get("removal")
            && record.get("worktree_path").and_then(|v| v.as_str()) == Some("") {
                let path = removal.get("path").and_then(|v| v.as_str()).context("missing cleanup path")?;
                execution_record["worktree_path"] = toml::Value::String(path.into());
                execution_record["cwd"] = toml::Value::String(path.into());
        }
        ensure!(crate::operations::receipts::legacy_execution_fingerprint(&execution_record).as_deref()
            == Some(journal.execution.as_str()), "cleanup execution identity mismatch");
        return Ok(());
    }
    if source.path == ".state/memory-review.json" {
        let state: Value = serde_json::from_slice(bytes)?;
        super::validate_runtime(&source.path, &state)?;
        for item in state["obligations"]
            .as_array()
            .context("missing obligations")?
        {
            if let Some(candidate) = item["candidate"].as_str() {
                ensure!(
                    !candidate.is_empty()
                        && Path::new(candidate).components().count() == 1
                        && safe_relative(candidate),
                    "unsafe candidate id"
                );
                let bytes = read(&safe_join(
                    project,
                    &format!("memory/candidates/{candidate}.md"),
                )?)?;
                ensure!(
                    item["candidate_digest"].as_str() == Some(hash(&bytes).as_str()),
                    "candidate digest mismatch"
                );
            }
        }
        return Ok(());
    }
    if !source.path.starts_with(".state/artifacts/") || !source.path.ends_with("/manifest.json") {
        return Ok(());
    }
    let parts: Vec<_> = source.path.split('/').collect();
    // Nested library JSON, including a file named manifest.json, is opaque.
    if parts.len() != 5 {
        return Ok(());
    }
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    ensure!(
        manifest.schema == 1 && manifest.thread == parts[2] && hash(bytes) == parts[3],
        "invalid artifact identity"
    );
    ensure!(
        manifest.machine.is_empty(),
        "remote artifact identity requires reconciliation"
    );
    let thread: toml::Value = toml::from_str(std::str::from_utf8(&read(&safe_join(
        project,
        &format!("threads/{}.toml", manifest.thread),
    )?)?)?)?;
    ensure!(
        thread
            .get("lifecycle_generation")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as u64
            >= manifest.generation,
        "artifact generation mismatch"
    );
    ensure!(thread.get("id").and_then(|v| v.as_str()) == Some(manifest.thread.as_str())
        && thread.get("machine").and_then(|v| v.as_str()).unwrap_or("").is_empty(),
        "artifact thread or machine mismatch");
    // Cleanup retains thread_dir; an older receipt must still name that exact
    // recorded source. A changed source without historical evidence is rejected.
    ensure!(
        thread
            .get("thread_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            == manifest.source,
        "artifact source mismatch"
    );
    let root = project.join(source.path.strip_suffix("/manifest.json").unwrap());
    let mut paths = BTreeSet::new();
    for entry in manifest.entries {
        ensure!(
            safe_relative(&entry.path)
                && entry.path != "manifest.json"
                && paths.insert(entry.path.clone()),
            "invalid artifact entry"
        );
        let path = safe_join(&root, &entry.path)?;
        if entry.directory {
            ensure!(
                fs::symlink_metadata(path)?.is_dir(),
                "artifact directory missing"
            );
        } else {
            let bytes = read(&path)?;
            ensure!(
                bytes.len() as u64 == entry.bytes && hash(&bytes) == entry.sha256,
                "artifact bytes changed"
            );
        }
    }
    Ok(())
}

/// Repeated by inspect/plan and exact-plan validation under maintenance locks.
/// Unreachable sessions are uncertainty, never absence. Idle UI alone is insufficient.
pub(super) fn quiesced(project: &Path, record: &Value) -> Result<()> {
    ensure!(
        record
            .get("machine")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty(),
        "remote writer"
    );
    let coordinator: Value =
        serde_json::from_slice(&read(&project.join(".state/coordinator.json"))?)?;
    let socket = coordinator
        .get("socket")
        .and_then(Value::as_str)
        .context("recorded socket missing")?;
    ensure!(
        Path::new(socket).is_absolute(),
        "recorded socket must be absolute"
    );
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let runner = crate::runner::RealRunner;
    ensure!(
        crate::herdr::version(&bin, &runner)? >= crate::herdr::MIN_VERSION,
        "unsupported Herdr"
    );
    let herdr = crate::herdr::Herdr::new(bin, socket, &runner);
    let panes = herdr.pane_list()?;
    let agents = herdr.agent_list()?;
    let pane = record
        .get("pane_id")
        .and_then(Value::as_str)
        .context("missing pane")?;
    ensure!(
        !agents.iter().any(|a| a.pane_id == pane),
        "live agent blocks migration"
    );
    if let Some(found) = panes.iter().find(|p| p.pane_id == pane) {
        for (wanted, actual) in [
            ("workspace_id", found.workspace_id.as_str()),
            ("tab_id", found.tab_id.as_str()),
            ("cwd", found.cwd.as_str()),
        ] {
            ensure!(
                record.get(wanted).and_then(Value::as_str) == Some(actual) && !actual.is_empty(),
                "pane identity mismatch"
            );
        }
    }
    writer_paths(project, record)
}

pub(super) fn writer_paths(project: &Path, record: &Value) -> Result<()> {
    for field in ["cwd", "worktree_path", "thread_dir"] {
        if let Some(path) = record
            .get(field)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            ensure!(
                Path::new(path).is_absolute(),
                "writer path must be absolute"
            );
            crate::writer_quiescence::no_process_references(Path::new(path))?;
        }
    }
    crate::writer_quiescence::no_process_references(project)?;
    Ok(())
}
