//! Kind isolation for legacy launch arguments. These checks bind user-supplied
//! argv to one CLI kind; they do not certify vendor flags or protocol support.
use anyhow::{Result,ensure};
pub mod profiles;
pub mod probe;
pub mod resolve;
pub fn arguments<'a>(kind:&str,bound_kind:Option<&str>,args:&'a [String],setting:&str)->Result<&'a [String]> {
    valid_kind(kind)?;
    if let Some(bound)=bound_kind {valid_kind(bound)?;}
    ensure!(args.len()<=128&&args.iter().map(String::len).sum::<usize>()<=65_536&&!args.iter().any(|a|a.contains('\0')),"{setting} exceeds argument limits or contains NUL");
    if !args.is_empty() {
        let bound=bound_kind.ok_or_else(||anyhow::anyhow!("{setting} is not bound to an agent kind; set {setting}_kind to the kind these existing arguments were written for before launching"))?;
        ensure!(bound==kind,"{setting} belongs to agent kind `{bound}`, not requested kind `{kind}`; configure arguments for the requested kind explicitly");
    }
    Ok(args)
}
fn valid_kind(kind:&str)->Result<()> {ensure!(!kind.is_empty()&&kind.len()<=64&&kind.as_bytes()[0].is_ascii_alphabetic()&&kind.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c)),"invalid agent kind identifier");Ok(())}

/// Resolve the launch's main repository without consulting agent configuration.
pub fn worker_arguments(safety: &crate::project::Safety, t: &crate::thread::Thread, runner: &dyn crate::runner::Runner) -> Result<Vec<String>> {
    let mut repository = t.repo.clone();
    if t.agent == "codex" && t.machine.is_empty() && !safety.thread_agent_args_explicit && safety.thread_agent_args.is_empty() {
        let out = runner.run(&crate::runner::Cmd::new("git", std::time::Duration::from_secs(10)).args(["-C", &t.cwd, "rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
        if out.success() {
            let common = std::path::Path::new(out.stdout.trim());
            // A bare repository is its own root; ordinary and linked worktrees
            // share the main worktree's .git directory.
            if common.file_name().is_some_and(|name| name == ".git") {
                repository = common.parent().ok_or_else(|| anyhow::anyhow!("git common directory has no parent"))?.to_string_lossy().into_owned();
            }
        }
    }
    safety.effective_worker_arguments(&t.agent, &t.cwd, &repository)
}
