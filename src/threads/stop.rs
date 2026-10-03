//! Durable worker shutdown. Submitted terminal actions are observed on retry,
//! never blindly repeated after an ambiguous transport result or a crash.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub execution: String,
    pub step: String,
    pub reason: String,
    pub requested_at: String,
    pub requested_by: String,
}

fn checkpoint(project: &Project, id: &str, journal: &Journal, step: &str) -> Result<Journal> {
    let mut next = journal.clone();
    next.step = step.into();
    thread::update_checked(project, id, |t| {
        anyhow::ensure!(
            t.stop_journal.as_ref() == Some(journal)
                && thread::execution_fingerprint(t) == journal.execution,
            "stop identity changed"
        );
        t.stop_journal = Some(next.clone());
        Ok(())
    })?;
    std::fs::File::open(project.dir().join("threads"))?.sync_all()?;
    Ok(next)
}

fn ownership(ctx: &Ctx, project: &Project, record: &Thread, view: &SessionView) -> Result<bool> {
    let socket = project
        .coordinator()
        .context("coordinator session missing")?
        .socket;
    #[cfg(feature = "state-store")]
    crate::runtime_ownership::check_conflicts(
        ctx,
        &project.dir(),
        Some(&format!("thread:{}", record.id)),
        &herdr_farm::domain::RuntimeIdentity {
            socket: socket.clone(),
            pane_id: record.pane_id.clone(),
            machine: record.machine.clone(),
            ..Default::default()
        },
    )?;
    let mut exclusive_workspace = true;
    for slug in project::list_slugs(&ctx.root) {
        let other = Project::load(&ctx.root, &slug)?;
        if other.coordinator().is_none_or(|c| c.socket != socket) {
            continue;
        }
        if let Some(c) = other.coordinator() {
            if !record.is_remote() && c.workspace_id == record.workspace_id {
                exclusive_workspace = false;
            }
            anyhow::ensure!(
                record.is_remote() || c.pane_id != record.pane_id,
                "pane belongs to a coordinator"
            );
        }
        let (records, diagnostics) = thread::list_with_diagnostics(&other);
        anyhow::ensure!(
            diagnostics.is_empty(),
            "cannot establish terminal ownership: {}",
            diagnostics.join("; ")
        );
        for t in records {
            if other.canonical_dir() == project.canonical_dir() && t.id == record.id {
                continue;
            }
            if t.machine == record.machine && t.workspace_id == record.workspace_id {
                exclusive_workspace = false;
            }
            anyhow::ensure!(
                t.machine != record.machine
                    || record.pane_id.is_empty()
                    || t.pane_id != record.pane_id,
                "pane is also claimed by {} in {slug}",
                t.id
            );
        }
    }
    let (agents, panes) = lists_for(view, record)?;
    anyhow::ensure!(
        agents
            .iter()
            .filter(|a| a.pane_id == record.pane_id)
            .all(|a| thread::agent_matches(record, a)),
        "pane has a foreign agent"
    );
    anyhow::ensure!(
        panes
            .iter()
            .filter(|p| p.pane_id == record.pane_id)
            .all(|p| thread::pane_matches(record, p)),
        "pane identity changed"
    );
    // Only worktree placement owns a workspace. Tabs/adopted panes share it.
    Ok(exclusive_workspace
        && record.kind == Kind::Worktree
        && !record.workspace_id.is_empty()
        && project
            .coordinator()
            .is_none_or(|c| record.is_remote() || c.workspace_id != record.workspace_id))
}

pub fn stop(ctx: &Ctx, slug: &str, id: &str, reason: Option<&str>) -> Result<()> {
    let _lease = crate::cleanup::lease(&ctx.root)?;
    let project = Project::load(&ctx.root, slug)?;
    let mut record = thread::load(&project, id)?;
    if record.status == Status::Stopped {
        println!("{id} is already stopped.");
        return Ok(());
    }
    anyhow::ensure!(
        record.status != Status::Resolved,
        "{id} is resolved; stop is for unresolved threads"
    );
    anyhow::ensure!(
        record.pending_live_copy.is_none()
            && record.pending_final_copy.is_none()
            && record.removal.is_none(),
        "recover the pending projection or removal before stopping"
    );
    let view = require_session(ctx, &project)?;
    let owned_workspace = ownership(ctx, &project, &record, &view)?;
    if record.stop_journal.is_none() {
        record = thread::update_checked(&project, id, |t| {
            thread::invalidate_finalization(t)?;
            t.status = Status::Stopping;
            t.prompt_pending = false;
            t.launch_claim = None;
            t.prompt_claim = None;
            t.stop_journal = Some(Journal {
                version: 1,
                execution: thread::execution_fingerprint(t),
                step: "copy".into(),
                reason: reason.unwrap_or("manual").into(),
                requested_at: project::now(),
                requested_by: ctx
                    .env
                    .var("USER")
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("uid:{}", unsafe { libc::geteuid() })),
            });
            Ok(())
        })?;
        std::fs::File::open(project.dir().join("threads"))?.sync_all()?;
    }
    let mut journal = record
        .stop_journal
        .clone()
        .context("stop journal missing")?;
    anyhow::ensure!(
        journal.version == 1 && journal.execution == thread::execution_fingerprint(&record),
        "unsupported or stale stop journal"
    );
    if journal.step == "copy" {
        match final_copy(ctx, &project, &record).outcome {
            CopyOutcome::Complete => {}
            CopyOutcome::Partial(notes) => {
                println!("the final copy was partial: {}", notes.join("; "))
            }
            CopyOutcome::Failed(error) => {
                // A failed placement may never have created its artifact source.
                // Nothing is deleted by stop; retain and report this missing copy.
                let missing = !record.is_remote()
                    && (record.thread_dir.is_empty()
                        || std::fs::symlink_metadata(&record.thread_dir)
                            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound));
                if missing {
                    println!("the final copy was partial: artifact source missing ({error})");
                } else {
                    bail!("final copy failed: {error}; retry thread stop");
                }
            }
        }
        journal = checkpoint(&project, id, &journal, "ready")?;
    }
    let herdr = view.herdr.on_machine(&record.machine);
    if journal.step == "ready" {
        let fresh = require_session(ctx, &project)?;
        ownership(ctx, &project, &record, &fresh)?;
        let agents = herdr.agent_list()?;
        journal = checkpoint(&project, id, &journal, "agent")?;
        if agents.iter().any(|a| a.pane_id == record.pane_id) {
            herdr.agent_finish(&record.pane_id)?;
        }
    }
    if journal.step == "agent" {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if !herdr
                .agent_list()?
                .iter()
                .any(|a| a.pane_id == record.pane_id)
            {
                break;
            }
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "agent shutdown unverified; graceful stop was submitted once; retry thread stop after inspecting the pane"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        if !record.is_remote() && record.kind == Kind::Worktree && !record.worktree_path.is_empty()
        {
            crate::cleanup::no_process_references(Path::new(&record.worktree_path))?;
        }
        let fresh = require_session(ctx, &project)?;
        ownership(ctx, &project, &record, &fresh)?;
        anyhow::ensure!(
            !herdr
                .agent_list()?
                .iter()
                .any(|a| a.pane_id == record.pane_id),
            "agent reappeared before pane close"
        );
        journal = checkpoint(&project, id, &journal, "pane")?;
        if herdr
            .pane_list()?
            .iter()
            .any(|p| p.pane_id == record.pane_id)
        {
            herdr.pane_close(&record.pane_id)?;
        }
    }
    if journal.step == "pane" {
        let panes = herdr.pane_list()?;
        anyhow::ensure!(
            !panes.iter().any(|p| p.pane_id == record.pane_id),
            "pane close unverified; retry thread stop after reconciliation"
        );
        journal = checkpoint(&project, id, &journal, "workspace")?;
        if owned_workspace
            && !panes.iter().any(|p| p.workspace_id == record.workspace_id)
            && herdr.workspace_exists(&record.workspace_id)?
        {
            herdr.workspace_close(&record.workspace_id)?;
        }
    }
    if journal.step == "workspace" {
        let panes = herdr.pane_list()?;
        anyhow::ensure!(
            !panes.iter().any(|p| p.pane_id == record.pane_id)
                && !herdr
                    .agent_list()?
                    .iter()
                    .any(|a| a.pane_id == record.pane_id),
            "thread terminal reappeared; stop remains unfinished"
        );
        anyhow::ensure!(
            !owned_workspace
                || panes.iter().any(|p| p.workspace_id == record.workspace_id)
                || !herdr.workspace_exists(&record.workspace_id)?,
            "workspace close unverified; retry thread stop after reconciliation"
        );
        thread::update_checked(&project, id, |t| {
            anyhow::ensure!(
                t.stop_journal.as_ref() == Some(&journal)
                    && thread::execution_fingerprint(t) == journal.execution,
                "stop identity changed"
            );
            t.status = Status::Stopped;
            t.stopped_reason = journal.reason.clone();
            t.stopped_at = project::now();
            t.stopped_by = journal.requested_by.clone();
            t.stop_journal = None;
            t.last_state.clear();
            t.last_group = Group::Stopped.token().into();
            Ok(())
        })?;
        std::fs::File::open(project.dir().join("threads"))?.sync_all()?;
    } else {
        bail!("unknown stop checkpoint: {}", journal.step);
    }
    println!("{id} stopped; branch and worktree kept.");
    Ok(())
}
