//! Versioned project-local permission stream. All mutations use the project lock;
//! owner configuration is never rewritten. This stream is independent of state.db.
use crate::{
    paths::Ctx,
    project::{self, Project, Safety},
    runner::{Cmd, Runner},
    thread,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Write},
    path::{Component, Path},
    time::Duration,
};

const MIGRATIONS: &[&str] = &[include_str!("../migrations/worker-permissions/0001.json")];
pub const TOOLS: &[&str] = &[
    "godot --headless",
    "cargo test",
    "cargo build",
    "cargo check",
    "cargo nextest run",
    "npm test",
    "npm run test",
    "pnpm test",
    "yarn test",
    "pytest",
    "go test",
    "make test",
    "make check",
];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub prefix: String,
    pub reason: String,
    pub status: String,
    pub principal: String,
    pub rule: String,
    pub repository: String,
    pub script: String,
    pub blob: String,
    pub created: String,
    pub decision_by: String,
    pub decided: String,
    pub decision_reason: String,
}
#[derive(Serialize, Deserialize)]
pub struct State {
    version: usize,
    pub records: Vec<Record>,
    pub pending_restarts: BTreeMap<String, Vec<String>>,
    pub restart_history: BTreeMap<String, String>,
    pub restart_generations: BTreeMap<String, u64>,
}
pub fn load(project: &Project) -> Result<State> {
    let path = project.state_dir().join("worker-permissions.json");
    let state: State = match crate::paths::read_control_text(&path, 4 * 1024 * 1024)? {
        Some(text) => serde_json::from_str(&text).context("invalid worker permission state")?,
        None => serde_json::from_str(MIGRATIONS[0])?,
    };
    ensure!(
        state.version == MIGRATIONS.len(),
        "unsupported worker permission stream version"
    );
    Ok(state)
}
fn save(project: &Project, state: &State) -> Result<()> {
    ensure!(
        state.records.len() <= 4096,
        "worker permission history limit reached"
    );
    let encoded = serde_json::to_vec_pretty(state)?;
    ensure!(
        encoded.len() <= 4 * 1024 * 1024,
        "worker permission state exceeds read limit"
    );
    project::write_atomic(
        &project.state_dir().join("worker-permissions.json"),
        &encoded,
    )
}
fn git(runner: &dyn Runner, repo: &str, args: &[&str]) -> Result<String> {
    let out = runner.run(
        &Cmd::new("git", Duration::from_secs(10))
            .args(["-C", repo])
            .args(args.iter().copied()),
    )?;
    ensure!(out.success(), "cannot verify repository target");
    Ok(out.stdout.trim().into())
}
fn script_blob(project: &Project, repo: &str, script: &str, runner: &dyn Runner) -> Result<String> {
    ensure!(
        !Path::new(script).is_absolute()
            && Path::new(script)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "script must be inside repository"
    );
    let (settings, _) = project.read_project_md()?;
    ensure!(
        settings
            .repos
            .iter()
            .any(|r| r.machine.is_none() && r.path == repo),
        "repository is not an owner-configured local repository"
    );
    let target = if settings.integration_target.is_empty() {
        git(
            runner,
            repo,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        )
        .or_else(|_| {
            let head = git(runner, repo, &["symbolic-ref", "--quiet", "HEAD"])?;
            ensure!(
                matches!(head.as_str(), "refs/heads/main" | "refs/heads/master"),
                "configure integration_target to verify default branch"
            );
            Ok::<_, anyhow::Error>(head)
        })?
    } else {
        settings.integration_target
    };
    let target = git(
        runner,
        repo,
        &[
            "rev-parse",
            "--symbolic-full-name",
            "--verify",
            "--end-of-options",
            &target,
        ],
    )?;
    ensure!(
        target.starts_with("refs/heads/") || target.starts_with("refs/remotes/"),
        "target must be a branch"
    );
    ensure!(
        !thread::list(project)
            .iter()
            .any(|t| target == format!("refs/heads/{}", t.branch)),
        "target cannot be a worker branch"
    );
    let commit = git(
        runner,
        repo,
        &["rev-parse", "--verify", &format!("{target}^{{commit}}")],
    )?;
    let tree = git(runner, repo, &["ls-tree", &commit, "--", script])?;
    let (metadata, path) = tree
        .split_once('\t')
        .context("script is not committed on target")?;
    let fields: Vec<_> = metadata.split_whitespace().collect();
    ensure!(
        path == script
            && fields.len() == 3
            && matches!(fields[0], "100644" | "100755")
            && fields[1] == "blob",
        "script must be an exact regular tracked file"
    );
    Ok(fields[2].into())
}
fn classify(
    project: &Project,
    safety: &Safety,
    prefix: &str,
    runner: &dyn Runner,
) -> Result<(String, String, String, String)> {
    let command = prefix.strip_suffix(":*").context("prefix must end in :*")?;
    let words: Vec<_> = command.split_whitespace().collect();
    let executable = words.first().context("empty command")?;
    let name = executable.rsplit('/').next().unwrap_or(executable);
    ensure!(
        !matches!(
            name,
            "git"
                | "curl"
                | "wget"
                | "ssh"
                | "scp"
                | "sftp"
                | "rsync"
                | "ftp"
                | "nc"
                | "ncat"
                | "netcat"
                | "socat"
                | "http"
                | "httpie"
                | "npx"
                | "corepack"
                | "sudo"
                | "busybox"
                | "env"
                | "xargs"
                | "find"
                | "sed"
                | "rg"
        ) && !command.starts_with("git "),
        "command requires owner approval"
    );
    ensure!(
        !words
            .iter()
            .any(|w| w.starts_with('/') || w.split('/').any(|p| p == "..")),
        "outside repository"
    );
    if TOOLS.contains(&command) {
        return Ok((
            "build/test tool".into(),
            String::new(),
            String::new(),
            String::new(),
        ));
    }
    let interpreter = matches!(
        name,
        "python"
            | "python2"
            | "python3"
            | "node"
            | "nodejs"
            | "bash"
            | "sh"
            | "dash"
            | "zsh"
            | "fish"
            | "ksh"
            | "perl"
            | "ruby"
            | "lua"
            | "php"
            | "tclsh"
            | "Rscript"
    );
    ensure!(
        !matches!(
            name,
            "csh"
                | "tcsh"
                | "awk"
                | "gawk"
                | "mawk"
                | "nawk"
                | "wish"
                | "R"
                | "deno"
                | "bun"
                | "pwsh"
                | "powershell"
                | "exec"
                | "eval"
                | "command"
                | "nohup"
        ) && (!(["python", "perl", "ruby", "lua", "php", "node"]
            .iter()
            .any(|base| name.starts_with(base)))
            || interpreter),
        "general interpreter requires owner approval"
    );
    if interpreter {
        ensure!(
            *executable == name && words.len() == 2 && !words[1].starts_with('-'),
            "interpreter requires one committed script argument"
        );
    }
    let script = if interpreter { words[1] } else { *executable };
    let script = script.strip_prefix("./").unwrap_or(script);
    if words.len() == 1 || interpreter {
        let (settings, _) = project.read_project_md()?;
        for repo in settings.repos.iter().filter(|r| r.machine.is_none()) {
            if let Ok(blob) = script_blob(project, &repo.path, script, runner) {
                return Ok((
                    "committed project script".into(),
                    repo.path.clone(),
                    script.into(),
                    blob,
                ));
            }
        }
    }
    ensure!(
        !interpreter && !executable.contains('/'),
        "script is not committed on target"
    );
    ensure!(
        safety
            .grantable_commands
            .iter()
            .any(|extra| extra == prefix),
        "no owner grantable rule"
    );
    Ok((
        "owner grantable_commands".into(),
        String::new(),
        String::new(),
        String::new(),
    ))
}
fn schedule(project: &Project, state: &mut State, id: &str) -> Result<()> {
    let (threads, diagnostics) = thread::list_with_diagnostics(project);
    ensure!(
        diagnostics.is_empty(),
        "invalid worker inventory: {}",
        diagnostics.join("; ")
    );
    for t in threads
        .into_iter()
        // A stopped (or stopping) worker was stopped deliberately; a grant never revives it.
        .filter(|t| !matches!(t.status, thread::Status::Resolved | thread::Status::Stopped | thread::Status::Stopping) && t.kind != thread::Kind::Adopted)
    {
        state
            .restart_generations
            .entry(t.id.clone())
            .or_insert(t.lifecycle_generation);
        state
            .pending_restarts
            .entry(t.id)
            .or_default()
            .push(id.into());
    }
    Ok(())
}
pub fn grant(ctx: &Ctx, slug: &str, prefix: &str, reason: &str) -> Result<()> {
    let project = Project::load(&ctx.root, slug)?;
    ensure!(
        !prefix.is_empty() && prefix.len() <= 256 && !prefix.chars().any(char::is_control),
        "invalid permission request prefix"
    );
    ensure!(reason.len() <= 4096, "reason too long");
    let safety = project.safety(&ctx.config_dir)?;
    let syntax = Safety {
        thread_allowed_commands: vec![prefix.into()],
        ..Safety::default()
    }
    .validate_thread_allowed_commands();
    let (matched, escalation) = if safety.worker_permissions == "owner" {
        (None, "worker_permissions=owner".to_string())
    } else if let Err(error) = syntax {
        (None, error.to_string())
    } else {
        match classify(&project, &safety, prefix, ctx.runner) {
            Ok(rule) => (Some(rule), String::new()),
            Err(error) => (None, error.to_string()),
        }
    };
    let _lock = project.lock()?;
    ensure!(
        project.safety(&ctx.config_dir)? == safety,
        "owner safety policy changed; retry grant"
    );
    let mut state = load(&project)?;
    if let Some(existing) = state
        .records
        .iter()
        .rev()
        .find(|r| r.prefix == prefix && matches!(r.status.as_str(), "granted" | "requested"))
    {
        println!("{} {}", existing.status, existing.id);
        return Ok(());
    }
    let id = format!("permission-{}", state.records.len() + 1);
    let (rule, repository, script, blob) = matched
        .clone()
        .unwrap_or_else(|| (escalation, String::new(), String::new(), String::new()));
    let record = Record {
        id: id.clone(),
        prefix: prefix.into(),
        reason: reason.into(),
        status: if matched.is_some() {
            "granted"
        } else {
            "requested"
        }
        .into(),
        principal: "coordinator".into(),
        rule,
        repository,
        script,
        blob,
        created: project::now(),
        decision_by: String::new(),
        decided: String::new(),
        decision_reason: String::new(),
    };
    if matched.is_some() {
        schedule(&project, &mut state, &id)?;
    }
    let status = record.status.clone();
    state.records.push(record);
    save(&project, &state)?;
    drop(_lock);
    deliver_requests(&project)?;
    println!("{status} {id}");
    Ok(())
}
fn request_summary(request:&Record)->String {
    format!("Owner permission requested: {} ({})",request.prefix,request.reason).chars().map(|c|if c.is_control() {' '} else {c}).collect()
}
pub fn deliver_requests(project: &Project) -> Result<()> {
    for r in load(project)?
        .records
        .iter()
        .filter(|r| r.status == "requested")
    {
        crate::inbox::write_once(
            project,
            &r.id,
            "permission-request",
            &r.id,
            &request_summary(r),
            &format!(
                "Exact prefix: {}\nReason: {}\nOwner: safety approve {} {} or safety reject {} {} --reason …",
                r.prefix, r.reason, project.slug, r.id, project.slug, r.id
            ),
        )?;
    }
    Ok(())
}
fn owner_confirm(action: &str, exact: &str) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal(),
        "safety {action} requires the owner at a terminal"
    );
    print!("{action}: {exact}\nType the exact value to confirm: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    ensure!(line.trim() == exact, "not confirmed");
    Ok(())
}
pub fn decide(ctx: &Ctx, slug: &str, id: &str, approve: bool, reason: &str) -> Result<()> {
    decide_inner(ctx,slug,id,approve,reason,None)
}
pub fn decide_ask(ctx:&Ctx,slug:&str,id:&str,summary:&str,approve:bool)->Result<()> {
    decide_inner(ctx,slug,id,approve,"",Some(summary))
}
fn decide_inner(ctx:&Ctx,slug:&str,id:&str,approve:bool,reason:&str,summary:Option<&str>)->Result<()> {
    let project = Project::load(&ctx.root, slug)?;
    let state = load(&project)?;
    let request = state
        .records
        .iter()
        .find(|r| r.id == id && r.status == "requested")
        .context("no pending permission request")?;
    ensure!(reason.len() <= 4096, "reason too long");
    if approve {
        Safety {
            thread_allowed_commands: vec![request.prefix.clone()],
            ..Safety::default()
        }
        .validate_thread_allowed_commands()?;
    }
    println!(
        "{}: {}\nReason: {}",
        request.id, request.prefix, request.reason
    );
    let attribution=if let Some(summary)=summary {
        ensure!(summary==request_summary(request),"--summary must be byte-identical to the stored summary");
        "owner:claude-code-ask"
    } else {owner_confirm(if approve { "approve" } else { "reject" }, id)?; "owner:terminal"};
    let _lock = project.lock()?;
    let mut state = load(&project)?;
    let r = state
        .records
        .iter_mut()
        .find(|r| r.id == id && r.status == "requested")
        .context("request changed")?;
    ensure!(r == request, "request changed during owner confirmation");
    if approve {
        r.principal = attribution.into();
    }
    r.status = if approve { "granted" } else { "rejected" }.into();
    r.decision_by = attribution.into();
    r.decided = project::now();
    r.decision_reason = reason.into();
    r.rule = if approve {
        "owner approval"
    } else {
        "owner rejection"
    }
    .into();
    if approve {
        schedule(&project, &mut state, id)?;
    }
    save(&project, &state)?;
    drop(_lock);
    crate::inbox::done(&project, &[id.to_string()], false)?;
    println!("{} {id}", if approve { "approved" } else { "rejected" });
    Ok(())
}
pub fn revoke(ctx: &Ctx, slug: &str, prefix: &str) -> Result<()> {
    owner_confirm("revoke", prefix)?;
    let project = Project::load(&ctx.root, slug)?;
    let _lock = project.lock()?;
    let mut state = load(&project)?;
    for r in state
        .records
        .iter_mut()
        .filter(|r| r.prefix == prefix && r.status == "granted")
    {
        r.status = "revoked".into();
        r.decision_by = "owner:terminal".into();
        r.decided = project::now();
    }
    save(&project, &state)?;
    println!("revoked {prefix}");
    Ok(())
}
pub fn effective(
    project: &Project,
    safety: &Safety,
    repository: &str,
    cwd: &str,
    runner: &dyn Runner,
) -> Result<Safety> {
    let mut safety = safety.clone();
    for r in load(project)?
        .records
        .iter()
        .filter(|r| r.status == "granted")
    {
        if !r.script.is_empty() {
            if r.repository != repository
                || script_blob(project, &r.repository, &r.script, runner).is_err()
            {
                continue;
            }
            // Never launch a granted script through a worker-created link outside
            // the checkout. Missing scripts simply remain unavailable there.
            let root = std::fs::canonicalize(cwd)?;
            if !std::fs::canonicalize(root.join(&r.script))
                .is_ok_and(|path| path.starts_with(&root))
            {
                continue;
            }
        }
        safety.active_worker_grants.push(r.prefix.clone());
        if !safety.thread_allowed_commands.contains(&r.prefix) {
            safety.thread_allowed_commands.push(r.prefix.clone());
        }
    }
    Ok(safety)
}
pub fn summary(project: &Project, safety: &Safety, runner: &dyn Runner) -> Result<String> {
    let (settings, _) = project.read_project_md()?;
    let mut summaries = Vec::new();
    for repo in settings.repos.iter().filter(|r| r.machine.is_none()) {
        let effective = effective(project, safety, &repo.path, &repo.path, runner)?;
        summaries.push(format!("{}: {}", repo.path, effective.worker_summary()));
    }
    if summaries.is_empty() {
        summaries.push(
            effective(
                project,
                safety,
                "",
                &project.canonical_dir().to_string_lossy(),
                runner,
            )?
            .worker_summary(),
        );
    }
    Ok(summaries.join("; "))
}
pub fn show(project: &Project) -> Result<()> {
    for r in load(project)?.records {
        println!(
            "  {} {} source={} rule={:?} repository={:?} script={:?} blob={} time={} decision={} {} reason={:?}",
            r.status,
            r.prefix,
            r.principal,
            r.rule,
            r.repository,
            r.script,
            r.blob,
            r.created,
            r.decision_by,
            r.decided,
            r.reason
        );
    }
    Ok(())
}

/// Runs under the ticker's root execution lease. Finishing never closes a pane
/// or removes a worktree; restart delegates to the existing preservation path.
pub fn restart_pass(ctx: &Ctx, project: &Project, herdr: &crate::herdr::Herdr) -> Result<()> {
    deliver_requests(project)?;
    let pending = load(project)?.pending_restarts;
    for (id, batches) in pending {
        let t = thread::load(project, &id)?;
        if matches!(t.status, thread::Status::Resolved | thread::Status::Stopped | thread::Status::Stopping) || t.kind == thread::Kind::Adopted {
            complete(project, &id, &batches, "no longer restartable")?;
            continue;
        }
        if load(project)?
            .restart_generations
            .get(&id)
            .is_some_and(|generation| t.lifecycle_generation > *generation)
        {
            complete(
                project,
                &id,
                &batches,
                "restart recorded; brief delivery follows thread state",
            )?;
            continue;
        }
        // A live launch/delivery is allowed to drain before its agent is touched.
        if t.launch_claim
            .as_ref()
            .is_some_and(|c| c.phase == thread::launch_delivery::Phase::Pending)
            || t.prompt_claim
                .as_ref()
                .is_some_and(|c| c.phase == thread::prompt_delivery::Phase::Pending)
            || t.pending_live_copy.is_some()
            || t.pending_final_copy.is_some()
        {
            continue;
        }
        let route = herdr.on_machine(&t.machine);
        let agents = route.agent_list()?;
        if t.prompt_pending && !agents.iter().any(|a| thread::agent_matches(&t, a)) {
            continue;
        }
        if let Some(agent) = agents.iter().find(|a| a.pane_id == t.pane_id) {
            ensure!(
                thread::agent_matches(&t, agent),
                "permission restart pane has a foreign agent"
            );
            if !agent.ready() && agent.agent_status != "blocked" {
                continue;
            }
            // Validate all restart constraints before interrupting the agent.
            let panes = route.pane_list()?;
            let mut live = thread::live_state(&t, &agents, &panes, jiff::Timestamp::now());
            live.agent_state = None;
            if crate::threads::restart_plan(&t, &live, false, jiff::Timestamp::now()).is_err() {
                continue;
            }
            // Shared panes must never have their other owner's agent interrupted.
            for slug in project::list_slugs(&ctx.root) {
                let other = Project::load(&ctx.root, &slug)?;
                if other.coordinator().is_none_or(|c| {
                    project
                        .coordinator()
                        .is_none_or(|own| c.socket != own.socket)
                }) {
                    continue;
                }
                let (records, diagnostics) = thread::list_with_diagnostics(&other);
                ensure!(
                    diagnostics.is_empty(),
                    "cannot verify exclusive restart pane ownership"
                );
                ensure!(
                    !records.iter().any(|r| r.status != thread::Status::Resolved
                        && r.machine == t.machine
                        && r.pane_id == t.pane_id
                        && (other.slug != project.slug || r.id != t.id)),
                    "permission restart pane is shared"
                );
            }
            route.agent_finish(&t.pane_id)?;
            if route.agent_list()?.iter().any(|a| a.pane_id == t.pane_id) {
                continue;
            }
        }
        crate::threads::restart_owned(ctx, &project.slug, &id)?;
        complete(
            project,
            &id,
            &batches,
            &format!(
                "restarted for {} at {}; conversation context lost; brief resent",
                batches.join(","),
                project::now()
            ),
        )?;
    }
    Ok(())
}
fn complete(project: &Project, id: &str, batches: &[String], note: &str) -> Result<()> {
    let _lock = project.lock()?;
    let mut state = load(project)?;
    if let Some(pending) = state.pending_restarts.get_mut(id) {
        pending.retain(|batch| !batches.contains(batch));
        if pending.is_empty() {
            state.pending_restarts.remove(id);
            state.restart_generations.remove(id);
        }
    }
    if state.pending_restarts.contains_key(id) {
        state
            .restart_generations
            .insert(id.into(), thread::load(project, id)?.lifecycle_generation);
    }
    state.restart_history.insert(id.into(), note.into());
    save(project, &state)
}
pub fn restart_note(project: &Project, id: &str) -> Result<String> {
    let state = load(project)?;
    let mut note = state.restart_history.get(id).cloned().unwrap_or_default();
    if let Some(batches) = state.pending_restarts.get(id) {
        note.push_str(&format!(
            "; permission restart pending: {}",
            batches.join(",")
        ));
    }
    Ok(note)
}
