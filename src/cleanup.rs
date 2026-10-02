//! Cooperative local cleanup checkpoint. No force removal and no process killing.
use std::{fs::{self, File}, path::Path};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use crate::{paths::Ctx, project::Project, runner::Cmd, thread::{self, Thread}};

/// Excludes supported launch/prompt/adopt/ticker operations within this root.
/// It is deliberately separate from the short project-record transaction lock.
pub struct Lease { _guard:herdr_farm::execution_guard::RootGuard }
pub fn lease(root: &Path) -> Result<Lease> {
    Ok(Lease{_guard:herdr_farm::execution_guard::RootGuard::exclusive(root)?})
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Removal {
    pub operation: String,
    pub repo: String,
    pub path: String,
    pub branch: String,
    pub head: String,
    pub snapshot: String,
    pub generation: u64,
    pub removed: bool,
}
fn git(ctx: &Ctx, repo: &str, args: &[&str]) -> Result<String> {
    let output = ctx.runner.run(&Cmd::new("git", std::time::Duration::from_secs(15)).args(["-C", repo]).args(args.iter().copied()))?;
    ensure!(output.success(), "git {}: {}", args.join(" "), output.error_text());
    Ok(output.stdout.trim_end_matches('\n').into())
}
fn registered(ctx: &Ctx, record: &Thread, path: &str, head: &str) -> Result<bool> {
    let list = git(ctx, &record.repo, &["worktree", "list", "--porcelain", "-z"])?;
    let wanted = format!("worktree {path}");
    let branch = format!("branch refs/heads/{}", record.branch);
    let head = format!("HEAD {head}");
    for block in list.split("\0\0") {
        let fields: Vec<_> = block.split('\0').collect();
        if fields.contains(&wanted.as_str()) {
            ensure!(fields.contains(&branch.as_str()) && fields.contains(&head.as_str()), "registered worktree branch or commit changed");
            ensure!(!fields.iter().any(|f| f.starts_with("locked") || f.starts_with("prunable")), "worktree is locked or prunable");
            return Ok(true);
        }
    }
    Ok(false)
}
pub use herdr_farm::writer_quiescence::no_process_references;

/// Caller holds the lifecycle lease and has checked all project/pane ownership.
pub fn remove(ctx: &Ctx, project: &Project, record: &Thread, writers_stopped: bool) -> Result<()> {
    anyhow::ensure!((record.pending_live_copy.is_none()&&record.pending_final_copy.is_none()),"recover the pending live projection before cleanup");
    if let crate::ticker::LockState::Held(info) = crate::ticker::lock_state(&ctx.root) {
        ensure!(info.version == crate::VERSION, "running ticker uses a different checkpoint protocol build; stop or restart it before cleanup");
    }
    ensure!(writers_stopped, "writer quiescence requires --writers-stopped after stopping all known artifact writers; keeping the worktree");
    ensure!(!record.is_remote(), "remote cleanup has no writer checkpoint adapter; keeping the worktree");
    let repo = fs::canonicalize(&record.repo)?.to_str().context("non-UTF-8 repository")?.to_string();
    let path = fs::canonicalize(&record.worktree_path)?.to_str().context("non-UTF-8 worktree")?.to_string();
    ensure!(path != repo && !record.branch.is_empty(), "invalid worktree identity");
    #[cfg(feature="state-store")]
    crate::runtime_ownership::check_worktree_references(ctx,&project.dir(),&record.id,Path::new(&path))?;
    let head = git(ctx, &repo, &["rev-parse", "--verify", &format!("refs/heads/{}", record.branch)])?;
    ensure!(registered(ctx, record, &path, &head)?, "worktree is not registered to this repository");
    ensure!(git(ctx, &path, &["rev-parse", "--show-toplevel"])? == path, "worktree root changed");
    no_process_references(Path::new(&path))?;
    let manifest = crate::artifacts::load(project, record, &record.artifact_snapshot)?;
    crate::artifacts::verify_source(record, &manifest)?;
    let removal = Removal { operation: thread::sha256_hex(format!("{}:{}:{}:{}", record.id, record.lifecycle_generation, path, head).as_bytes()), repo, path, branch: record.branch.clone(), head, snapshot: record.artifact_snapshot.clone(), generation: record.lifecycle_generation, removed: false };
    thread::update_checked(project, &record.id, |current| {
        ensure!(thread::execution_fingerprint(current) == thread::execution_fingerprint(record), "thread changed before removal reservation");
        current.removal = Some(removal.clone());
        Ok(())
    })?;
    File::open(project.dir().join("threads"))?.sync_all()?;
    ensure!(project.status() == crate::project::Status::Active, "project became inactive before removal");
    no_process_references(Path::new(&removal.path))?;
    crate::artifacts::verify_source(record, &manifest)?;
    ensure!(registered(ctx, record, &removal.path, &removal.head)?, "worktree registration changed");
    // Git enforces dirty/untracked/submodule protections. Never retry with force.
    git(ctx, &removal.repo, &["worktree", "remove", "--", &removal.path])?;
    thread::update_checked(project, &record.id, |current| {
        ensure!(current.removal.as_ref() == Some(&removal), "removal reservation changed; reconcile before retrying");
        current.removal.as_mut().unwrap().removed = true;
        Ok(())
    })?;
    Ok(())
}

/// A durable reservation also covers a crash after Git removed the worktree but
/// before its acknowledgement was saved. Existing/mismatched registrations block.
pub fn restore(ctx: &Ctx, project: &Project, record: &Thread) -> Result<()> {
    let removal = record.removal.as_ref().context("no intentional removal record; inspect the incomplete creation manually")?;
    ensure!(!record.is_remote() && record.branch == removal.branch, "retained branch identity changed");
    ensure!(fs::canonicalize(&record.repo)? == Path::new(&removal.repo), "repository identity changed");
    ensure!(git(ctx, &record.repo, &["rev-parse", "--verify", &format!("refs/heads/{}", removal.branch)])? == removal.head, "retained branch advanced; inspect before reopening");
    crate::artifacts::load(project, record, &removal.snapshot)?;
    #[cfg(feature="state-store")]
    crate::runtime_ownership::check_worktree_references(ctx,&project.dir(),&record.id,Path::new(&removal.path))?;
    #[cfg(not(feature="state-store"))]
    for slug in crate::project::list_slugs(&ctx.root) {
        let owner = Project::load(&ctx.root, &slug)?;
        let (records, diagnostics) = thread::list_with_diagnostics(&owner);
        ensure!(diagnostics.is_empty(), "cannot establish reopen ownership: {}", diagnostics.join("; "));
        for other in records {
            if other.is_remote() || (owner.canonical_dir() == project.canonical_dir() && other.id == record.id) { continue; }
            for location in [&other.worktree_path, &other.cwd] {
                if location.is_empty() { continue; }
                let location = fs::canonicalize(location).unwrap_or_else(|_| location.into());
                ensure!(!location.starts_with(&removal.path) && !Path::new(&removal.path).starts_with(&location), "reopen worktree is referenced by {} in {slug}", other.id);
            }
        }
    }
    let exists = match fs::symlink_metadata(&removal.path) {
        Ok(meta) => { ensure!(meta.is_dir(), "reopen path is not a real directory"); true },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let parent = Path::new(&removal.path).parent().context("reopen path has no parent")?;
    ensure!(fs::canonicalize(parent)? == parent, "reopen parent identity changed");
    if exists {
        // Idempotent retry after worktree add succeeded and acknowledgement failed.
        ensure!(fs::symlink_metadata(&removal.path)?.is_dir() && fs::canonicalize(&removal.path)? == Path::new(&removal.path), "reopen path was replaced");
        ensure!(registered(ctx, record, &removal.path, &removal.head)?, "reopen path already exists without matching registration");
    } else {
        ensure!(!registered(ctx, record, &removal.path, &removal.head)?, "absent path is still registered; reconcile Git metadata first");
        git(ctx, &record.repo, &["worktree", "add", "--", &removal.path, &removal.branch])?;
    }
    thread::update_checked(project, &record.id, |current| {
        ensure!(current.removal.as_ref() == Some(removal), "removal record changed during reopen");
        current.worktree_path = removal.path.clone();
        current.cwd = removal.path.clone();
        current.removal = None;
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{paths::Env, project, runner::RealRunner, thread::{Kind, Status}};
    fn fixture() -> (tempfile::TempDir, Project, Env, Thread) {
        let root = tempfile::tempdir().unwrap();
        let project = project::create(root.path(), "demo", "", vec![]).unwrap();
        let env = Env::for_test(root.path(), &[]);
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let ctx = Ctx { root: root.path().into(), config_dir: root.path().join("cfg"), env: &env, runner: &RealRunner, detached_ticker: false };
        let r = repo.to_str().unwrap();
        git(&ctx, r, &["init", "--quiet"]).unwrap();
        git(&ctx, r, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "--allow-empty", "-m", "base"]).unwrap();
        let work = root.path().join("worktree");
        git(&ctx, r, &["worktree", "add", "-b", "retained", work.to_str().unwrap()]).unwrap();
        let source = work.join(".herdr-project");
        fs::write(repo.join(".git/info/exclude"), b".herdr-project/\n").unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("report.md"), b"retained report").unwrap();
        let record = thread::allocate(&project, |t| {
            t.kind = Kind::Worktree; t.status = Status::Open; t.repo = r.into(); t.branch = "retained".into();
            t.worktree_path = work.to_str().unwrap().into(); t.thread_dir = source.to_str().unwrap().into();
        }).unwrap();
        let snapshot = crate::artifacts::capture_local(&project, &record).unwrap();
        let record = thread::update(&project, &record.id, |t| t.artifact_snapshot = snapshot.id).unwrap();
        (root, project, env, record)
    }

    #[test]
    fn lease_excludes_concurrent_lifecycle_and_releases_explicitly() {
        let root = tempfile::tempdir().unwrap();
        let held = lease(root.path()).unwrap();
        assert!(lease(root.path()).is_err());
        drop(held);
        assert!(lease(root.path()).is_ok());
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn retained_branch_reopens_and_git_refusal_keeps_source() {
        let (root, project, env, record) = fixture();
        let ctx = Ctx { root: root.path().into(), config_dir: root.path().join("cfg"), env: &env, runner: &RealRunner, detached_ticker: false };
        let _lease = lease(root.path()).unwrap();
        fs::write(Path::new(&record.worktree_path).join("untracked"), b"do not lose").unwrap();
        assert!(remove(&ctx, &project, &record, true).is_err());
        assert!(Path::new(&record.worktree_path).join("untracked").exists());
        fs::remove_file(Path::new(&record.worktree_path).join("untracked")).unwrap();
        remove(&ctx, &project, &record, true).unwrap();
        assert!(!Path::new(&record.worktree_path).exists());
        crate::artifacts::load(&project, &record, &record.artifact_snapshot).unwrap();
        let removed = thread::load(&project, &record.id).unwrap();
        assert!(removed.removal.as_ref().unwrap().removed);
        // Simulate lost acknowledgement after Git removal.
        let ambiguous = thread::update(&project, &record.id, |t| t.removal.as_mut().unwrap().removed = false).unwrap();
        restore(&ctx, &project, &ambiguous).unwrap();
        assert!(Path::new(&record.worktree_path).exists());
        assert_eq!(git(&ctx, &record.repo, &["rev-parse", "retained"]).unwrap(), removed.removal.unwrap().head);
        assert!(thread::load(&project, &record.id).unwrap().removal.is_none());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn open_descriptor_blocks_writer_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let _writer = File::create(dir.path().join("still-writing")).unwrap();
        assert!(no_process_references(dir.path()).is_err());
    }
}

/// Read-only reconciliation after a removal reservation. Never resubmit Git.
pub fn acknowledge_absent(ctx: &Ctx, project: &Project, record: &Thread) -> Result<()> {
    let removal = record.removal.as_ref().context("missing removal checkpoint")?;
    ensure!(record.lifecycle_generation == removal.generation && record.artifact_snapshot == removal.snapshot, "removal generation or preservation changed");
    ensure!(record.worktree_path.is_empty() || record.worktree_path == removal.path, "removal path changed");
    ensure!(record.branch == removal.branch && fs::canonicalize(&record.repo)? == Path::new(&removal.repo), "removal identity changed");
    ensure!(git(ctx, &removal.repo, &["rev-parse", "--verify", &format!("refs/heads/{}", removal.branch)])? == removal.head, "retained branch changed");
    ensure!(!Path::new(&removal.path).try_exists()? && !registered(ctx, record, &removal.path, &removal.head)?, "removal outcome is uncertain; inspect before retrying");
    crate::artifacts::load(project, record, &removal.snapshot)?;
    thread::update_checked(project, &record.id, |current| {
        ensure!(current.removal.as_ref() == Some(removal), "removal checkpoint changed");
        current.removal.as_mut().unwrap().removed = true;
        current.worktree_path.clear();
        current.cwd.clear();
        Ok(())
    })?;
    Ok(())
}

/// Prove this is the registered retained branch before terminal cleanup begins.
pub fn verify_registration(ctx: &Ctx, record: &Thread) -> Result<()> {
    let path = fs::canonicalize(&record.worktree_path)?.to_str().context("non-UTF-8 worktree")?.to_string();
    let head = git(ctx, &record.repo, &["rev-parse", "--verify", &format!("refs/heads/{}", record.branch)])?;
    ensure!(registered(ctx, record, &path, &head)?, "worktree is not registered to this repository and branch");
    Ok(())
}
