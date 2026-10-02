//! Versioned legacy cleanup journal. Each external action is submitted once;
//! recovery observes its result, never repeats an uncertain destructive action.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::{cleanup, paths::Ctx, project::{self, Project}, runner::Cmd, thread::{self, Kind, Status, Thread}, threads};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    generation: u64,
    execution: String,
    step: String,
    note: String,
}
fn path(project: &Project, record: &Thread) -> PathBuf {
    project.state_dir().join(format!("resolved-cleanup-{}.json", record.id))
}
fn load(project: &Project, record: &Thread) -> Result<Option<Journal>> {
    let Some(text) = crate::paths::read_control_text(&path(project, record), 64 * 1024)? else { return Ok(None) };
    let journal: Journal = serde_json::from_str(&text)?;
    ensure!(journal.version == 1, "unsupported resolved cleanup journal version");
    Ok((journal.generation == record.lifecycle_generation).then_some(journal))
}
fn save(project: &Project, record: &Thread, journal: &Journal) -> Result<()> {
    project::write_atomic(&path(project, record), &serde_json::to_vec(journal)?)?;
    std::fs::File::open(project.state_dir())?.sync_all()?;
    Ok(())
}
pub fn note(project: &Project, record: &Thread) -> String {
    match load(project, record) {
        Ok(Some(journal)) if !journal.note.is_empty() => format!("; {}", journal.note),
        Err(error) => format!("; cleanup journal unreadable: {error:#}; inspect with `thread show {} {}`", project.slug, record.id),
        _ => String::new(),
    }
}
fn clean_git(ctx: &Ctx, project: &Project, record: &Thread) -> Result<()> {
    let run = |args: &[&str]| -> Result<String> {
        let out = ctx.runner.run(&Cmd::new("git", std::time::Duration::from_secs(5)).args(["-C", &record.worktree_path]).args(args.iter().copied()))?;
        ensure!(out.success(), "cannot inspect worktree: {}", out.error_text());
        Ok(out.stdout)
    };
    ensure!(run(&["status", "--porcelain", "--untracked-files=all"])?.is_empty(), "dirty worktree: uncommitted or untracked changes were not preserved");
    let work = std::fs::canonicalize(&record.worktree_path)?;
    let source = std::fs::canonicalize(&record.thread_dir)?;
    ensure!(source.starts_with(&work), "artifact source is outside the worktree");
    for file in run(&["ls-files", "--others", "--ignored", "--exclude-standard", "-z"])?.split('\0').filter(|s| !s.is_empty()) {
        let file = work.join(file);
        if file == source.join("brief.md") {
            let brief = crate::paths::read_control_text(&file, 1024 * 1024)?.context("brief disappeared")?;
            let retained = crate::paths::read_control_text(&project.dir().join("threads").join(format!("{}.brief.md", record.id)), 1024 * 1024)?;
            ensure!(retained.as_deref() == Some(brief.as_str()), "generated brief was not preserved or changed after final copy");
        } else {
            ensure!(file == source.join("report.md") || file.starts_with(source.join("library")), "ignored worktree content outside the preserved artifacts");
        }
    }
    Ok(())
}
/// Caller holds the root lifecycle lease. At most one thread and one external
/// action per project per pass. Skipped journals are durable, visible and quiet.
pub fn pass(ctx: &Ctx, project: &Project) -> Result<()> {
    if project.safety(&ctx.config_dir)?.cleanup_resolved != "auto" { return Ok(()) }
    let (records, diagnostics) = thread::list_with_diagnostics(project);
    ensure!(diagnostics.is_empty(), "cannot establish cleanup ownership: {}", diagnostics.join("; "));
    for record in records.into_iter().filter(|t| t.status == Status::Resolved && (!t.worktree_path.is_empty() || t.removal.is_some() || !t.pane_id.is_empty())) {
        let existing = load(project, &record)?;
        if record.worktree_path.is_empty() && record.removal.is_some() && existing.is_none() { continue; }
        if existing.as_ref().is_some_and(|j| matches!(j.step.as_str(), "skipped" | "complete")) { continue; }
        let mut journal = existing.unwrap_or(Journal { version: 1, generation: record.lifecycle_generation, execution: thread::execution_fingerprint(&record), step: "ready".into(), note: String::new() });
        match advance(ctx, project, &record, &mut journal) {
            Ok(()) => save(project, &record, &journal)?,
            Err(error) => {
                journal.step = "skipped".into();
                journal.note = format!("cleanup kept: {error:#}; inspect `thread show {} {}`; after stopping writers and closing its pane/workspace, run `thread resolve {} {} --remove-worktree --writers-stopped`", project.slug, record.id, project.slug, record.id);
                save(project, &record, &journal)?;
                anyhow::bail!("{}: {}", record.id, journal.note);
            }
        }
        break;
    }
    Ok(())
}
fn advance(ctx: &Ctx, project: &Project, record: &Thread, journal: &mut Journal) -> Result<()> {
    if journal.step != "remove" {
        ensure!(journal.execution == thread::execution_fingerprint(record), "cleanup execution identity changed");
    }
    ensure!(record.kind == Kind::Worktree && !record.is_remote(), "shared, adopted or remote workspace needs owner cleanup");
    ensure!(!record.prompt_pending && record.launch_claim.is_none() && record.prompt_claim.is_none() && record.pending_live_copy.is_none() && record.pending_final_copy.is_none(), "pending agent delivery or artifact projection");
    if journal.step == "remove" || record.removal.is_some() {
        cleanup::acknowledge_absent(ctx, project, record)?;
        journal.step = "complete".into();
        journal.note = "cleanup complete; branch kept".into();
        return Ok(());
    }
    ensure!(!record.worktree_path.is_empty(), "no recorded owned worktree; inspect the pane before closing it");
    let worktree = threads::cleanup_ownership(ctx, project, record)?;
    cleanup::verify_registration(ctx, record)?;
    let manifest = crate::artifacts::load(project, record, &record.artifact_snapshot).context("final copy has no verified preservation receipt")?;
    crate::artifacts::verify_source(record, &manifest)?;
    clean_git(ctx, project, record)?;
    let view = threads::session_view(ctx, project).context("Herdr session is unreachable")?;
    // Refuse duplicate/foreign claims, including coordinator panes, before keys.
    for slug in project::list_slugs(&ctx.root) {
        let owner = Project::load(&ctx.root, &slug)?;
        ensure!(owner.coordinator().is_none_or(|c| c.pane_id != record.pane_id && c.workspace_id != record.workspace_id), "workspace belongs to a coordinator");
        let (others, diagnostics) = thread::list_with_diagnostics(&owner);
        ensure!(diagnostics.is_empty(), "cannot read all terminal ownership records");
        for other in others {
            if owner.canonical_dir() == project.canonical_dir() && other.id == record.id { continue; }
            ensure!(other.is_remote() || (other.pane_id != record.pane_id && other.workspace_id != record.workspace_id), "workspace is also owned by {} in {slug}", other.id);
        }
    }
    ensure!(!view.agents.iter().any(|a| a.pane_id != record.pane_id && (a.workspace_id == record.workspace_id || std::fs::canonicalize(&a.cwd).is_ok_and(|p| p.starts_with(&worktree)))), "worktree still has another managed agent");
    let agents: Vec<_> = view.agents.iter().filter(|a| a.pane_id == record.pane_id).collect();
    ensure!(agents.len() <= 1 && agents.iter().all(|a| thread::agent_matches(record, a) && a.ready()), "live agent is working, blocked, unknown or foreign");
    ensure!(view.panes.iter().filter(|p| p.workspace_id == record.workspace_id).all(|p| p.pane_id == record.pane_id && thread::pane_matches(record, p)), "workspace has another or foreign pane");
    match journal.step.as_str() {
        "ready" if !agents.is_empty() => {
            journal.step = "stop".into();
            save(project, record, journal)?;
            view.herdr.agent_finish(&record.pane_id)?;
        }
        "ready" | "stop" => {
            ensure!(agents.is_empty(), "agent shutdown could not be verified; graceful stop was submitted once");
            cleanup::no_process_references(&worktree)?;
            journal.step = "pane".into();
            save(project, record, journal)?;
            if view.panes.iter().any(|p| p.pane_id == record.pane_id) {
                view.herdr.pane_close(&record.pane_id)?;
            }
        }
        "pane" => {
            ensure!(!view.panes.iter().any(|p| p.pane_id == record.pane_id), "pane close outcome is uncertain");
            ensure!(!view.panes.iter().any(|p| p.workspace_id == record.workspace_id), "workspace is no longer empty");
            cleanup::no_process_references(&worktree)?;
            journal.step = "workspace".into();
            save(project, record, journal)?;
            if view.herdr.workspace_exists(&record.workspace_id)? {
                view.herdr.workspace_close(&record.workspace_id)?;
            }
        }
        "workspace" => {
            ensure!(!view.herdr.workspace_exists(&record.workspace_id)?, "workspace close outcome is uncertain");
            journal.step = "remove".into();
            save(project, record, journal)?;
            threads::remove_worktree(ctx, project, record, true)?;
            cleanup::acknowledge_absent(ctx, project, &thread::load(project, &record.id)?)?;
            journal.step = "complete".into();
            journal.note = "cleanup complete; branch kept".into();
        }
        _ => anyhow::bail!("unknown cleanup checkpoint"),
    }
    Ok(())
}
