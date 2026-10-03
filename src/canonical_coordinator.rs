//! Canonical coordinator adapter. A versioned effect journal survives CLI death;
//! SQLite remains the sole owner of routing, observations and runtime ownership.
use crate::{
    coordinator::{self, OpenOptions},
    herdr::Herdr,
    paths::{self, Ctx},
    project::{self, Project},
};
use anyhow::{Context, Result, bail, ensure};
use herdr_farm::{
    domain::{ProjectState, RuntimeRoute},
    migration, runtime, timing,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/canonical-coordinator/0001.json"),
    include_str!("../migrations/canonical-coordinator/0002.json"),
];
fn read_journal(text: &str) -> Result<Journal> {
    let mut value: Value = serde_json::from_str(text)?;
    let version = value["version"].as_u64().context("coordinator journal version missing")? as usize;
    ensure!((1..=MIGRATIONS.len()).contains(&version), "unsupported coordinator journal version");
    for patch in &MIGRATIONS[version..] {
        let patch: Value = serde_json::from_str(patch)?;
        for (key, field) in patch.as_object().context("invalid coordinator migration")? {
            value[key] = field.clone();
        }
    }
    Ok(serde_json::from_value(value)?)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    socket: String,
    session_identity: herdr_farm::domain::ResourceIdentity,
    route: RuntimeRoute,
    terminal: String,
    kind: String,
    name: String,
    settings_digest: String,
    config_digest: Option<String>,
    phase: String,
    deliveries: u32,
    /// Retained operator authorization to replace this absent coordinator.
    replace_missing: bool,
    #[serde(default)]
    priming_manifest: Option<Value>,
    #[serde(default)]
    permission_file: Option<String>,
}
fn save(dir: &Path, journal: &Journal) -> Result<()> {
    project::write_atomic(
        &dir.join(".state/canonical-coordinator.json"),
        &serde_json::to_vec_pretty(journal)?,
    )?;
    std::fs::File::open(dir.join(".state"))?.sync_all()?;
    Ok(())
}
fn call(h: &Herdr<'_>, id: &str, method: &str, params: Value) -> Result<Value> {
    let out = crate::runner::Runner::run(
        &crate::runner::RealRunner,
        &h.cmd(crate::herdr::CALL_TIMEOUT)
            .arg("remote-api-bridge")
            .stdin(
                serde_json::to_string(&json!({"id":id,"method":method,"params":params}))? + "\n",
            ),
    )?;
    ensure!(out.success(), "coordinator API request failed");
    let reply: Value = serde_json::from_str(&out.stdout)?;
    ensure!(
        reply["id"].as_str() == Some(id) && reply.get("error").is_none(),
        "coordinator API response mismatch or rejection"
    );
    reply
        .get("result")
        .cloned()
        .context("coordinator API response missing")
}
fn agent(h: &Herdr<'_>, j: &Journal, ready: bool) -> Result<Value> {
    let list = call(h, "coordinator-agent", "agent.list", json!({}))?;
    let agents = list["agents"]
        .as_array()
        .context("coordinator inventory missing")?;
    ensure!(agents.len() <= 256, "coordinator inventory exceeds bounds");
    let matching: Vec<_> = agents
        .iter()
        .filter(|a| a["pane_id"].as_str() == Some(&j.route.pane_id))
        .collect();
    ensure!(matching.len() == 1, "coordinator agent absent or ambiguous");
    herdr_farm::canonical_worker::validate_native_agent(
        matching[0],
        &j.route,
        &j.terminal,
        &j.kind,
        Some(&j.name),
        ready,
    )?;
    Ok(matching[0].clone())
}
fn allow_local_manifest(config_dir: &Path) -> Result<bool> {
    let Some(text) = paths::read_control_text(&config_dir.join("config.toml"), 1024 * 1024)? else {
        return Ok(false);
    };
    let config: toml::Value = toml::from_str(&text)?;
    config
        .get("coordinator")
        .and_then(|c| c.get("allow_local_manifest_override"))
        .map(|v| {
            v.as_bool()
                .context("coordinator.allow_local_manifest_override must be a boolean")
        })
        .unwrap_or(Ok(false))
}

fn manifest_policy(env: &paths::Env, config_dir: &Path, explain: &Value) -> Result<()> {
    ensure!(
        explain["manifest_version"]
            .as_str()
            .is_some_and(|v| !v.is_empty() && v.len() <= 96),
        "coordinator manifest version missing or exceeds 96 bytes; update Herdr agent manifests"
    );
    ensure!(
        explain.get("warning").is_none_or(Value::is_null)
            && explain.get("fallback_reason").is_none_or(Value::is_null),
        "coordinator manifest warning or fallback refused; inspect herdr agent explain and update or repair manifests"
    );
    let source = explain["manifest_source"]
        .as_str()
        .context("coordinator manifest source missing")?;
    ensure!(
        source.len() <= 4096,
        "coordinator manifest source exceeds bounds"
    );
    let shadow = explain["local_override_shadowing_remote"]
        .as_bool()
        .context("coordinator manifest override evidence missing")?;
    let opted_in = allow_local_manifest(config_dir)?;
    ensure!(
        !shadow || opted_in,
        "coordinator local manifest override shadowing remote refused; inspect the override, remove it or explicitly set [coordinator] allow_local_manifest_override = true in owner config"
    );
    if source == "bundled" {
        return Ok(());
    }
    let remote = source.strip_prefix("remote:");
    let local = source.strip_prefix("local:");
    ensure!(
        remote.is_some() || (local.is_some() && opted_in),
        "coordinator manifest source refused; use bundled or a remote manifest under the owner's Herdr state dir, or inspect and opt in to a local override"
    );
    if let Some(path) = remote {
        let state = env
            .var("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| env.home.join(".local/state"))
            .join("herdr")
            .canonicalize()
            .context("owner Herdr state dir unavailable")?;
        let path = Path::new(path);
        ensure!(
            path.is_absolute(),
            "coordinator remote manifest path must be absolute"
        );
        let resolved = path
            .canonicalize()
            .context("coordinator remote manifest path unavailable")?;
        ensure!(
            resolved.starts_with(&state) && resolved != state && resolved.is_file(),
            "coordinator remote manifest path is outside owner's Herdr state dir; update Herdr manifests in the owner session"
        );
    }
    Ok(())
}

fn readiness(env: &paths::Env, config_dir: &Path, kind: &str, explain: &Value) -> Result<()> {
    manifest_policy(env, config_dir, explain)?;
    // The worker validator retains all positive visible-idle requirements.
    // Only the coordinator's independently checked manifest policy differs.
    let mut checked = explain.clone();
    checked["manifest_source"] = json!("bundled");
    checked["local_override_shadowing_remote"] = json!(false);
    herdr_farm::canonical_worker::validate_visible_readiness(kind, &checked)
        .context("coordinator lacks verified visible prompt readiness; inspect agent explain for idle state, matched rule, warning/fallback and manifest version before retrying open")
}

fn ready(ctx: &Ctx, h: &Herdr<'_>, j: &Journal) -> Result<Value> {
    agent(h, j, true)?;
    let reply = call(
        h,
        "coordinator-ready",
        "agent.explain",
        json!({"target":j.route.pane_id}),
    )?;
    let explain = &reply["explain"];
    readiness(ctx.env, &ctx.config_dir, &j.kind, explain)?;
    Ok(
        json!({"source":explain["manifest_source"], "version":explain["manifest_version"],
        "local_override_shadowing_remote":explain["local_override_shadowing_remote"]}),
    )
}

pub fn doctor_manifest(
    env: &paths::Env,
    config_dir: &Path,
    dir: &Path,
    runner: &dyn crate::runner::Runner,
) -> Result<Vec<String>> {
    let Some(text) =
        paths::read_control_text(&dir.join(".state/canonical-coordinator.json"), 64 * 1024)?
    else {
        return Ok(vec![permission_status(env, dir, runner)?]);
    };
    let j = read_journal(&text)?;
    let mut lines = vec![permission_status(env, dir, runner)?];
    if let Some(evidence) = &j.priming_manifest {
        lines.push(format!(
            "coordinator priming manifest: {} version {} (override {})",
            evidence["source"], evidence["version"], evidence["local_override_shadowing_remote"]
        ));
    }
    let h = Herdr::new(env.herdr_bin(), &j.socket, runner);
    let reply = match call(
        &h,
        "coordinator-doctor-manifest",
        "agent.explain",
        json!({"target":j.route.pane_id}),
    ) {
        Ok(reply) => reply,
        Err(error) => {
            lines.push(format!("coordinator live manifest unavailable: {error:#}; inspect owner session agent explain"));
            return Ok(lines);
        }
    };
    let e = &reply["explain"];
    if e["manifest_source"].as_str() != Some("bundled")
        || e["local_override_shadowing_remote"].as_bool() != Some(false)
    {
        let decision = manifest_policy(env, config_dir, e);
        lines.push(format!("coordinator live manifest: {} version {}; {}", e["manifest_source"], e["manifest_version"],
            match decision { Ok(()) => "manifest policy accepted; priming also requires verified visible idle readiness".into(), Err(error) => format!("refused: {error:#}") }));
    }
    Ok(lines)
}

fn accepted(h: &Herdr<'_>, j: &Journal) -> Result<bool> {
    let until = Instant::now() + timing::brief_accept_window();
    loop {
        let a = agent(h, j, false)?;
        if matches!(a["agent_status"].as_str(), Some("working" | "blocked")) {
            return Ok(true);
        }
        if let Ok(explain) = call(
            h,
            "coordinator-accepted",
            "agent.explain",
            json!({"target":j.route.pane_id}),
        ) {
            let e = &explain["explain"];
            if e["agent"].as_str() == Some(&j.kind)
                && (e["state"].as_str() == Some("working")
                    || e["visible_working"].as_bool() == Some(true))
            {
                return Ok(true);
            }
        }
        if Instant::now() + timing::brief_accept_poll() >= until {
            return Ok(false);
        }
        std::thread::sleep(timing::brief_accept_poll());
    }
}
fn fence(dir: &Path, ctx: &Ctx, j: &Journal) -> Result<()> {
    migration::open_active(dir)?;
    ensure!(
        crate::reconcile_live::resource_identity(Path::new(&j.socket), true).as_ref()
            == Some(&j.session_identity),
        "coordinator session incarnation changed; inspect before --rebind"
    );
    let h = Herdr::new(ctx.env.herdr_bin(), &j.socket, ctx.runner);
    let panes = call(&h, "coordinator-fence-pane", "pane.list", json!({}))?;
    let matching: Vec<_> = panes["panes"]
        .as_array()
        .context("coordinator fence inventory missing")?
        .iter()
        .filter(|p| p["pane_id"].as_str() == Some(&j.route.pane_id))
        .collect();
    ensure!(
        matching.len() == 1,
        "coordinator pane absent or ambiguous before effect"
    );
    let pane = matching[0];
    ensure!(
        pane["workspace_id"].as_str() == Some(&j.route.workspace_id)
            && pane["tab_id"].as_str() == Some(&j.route.tab_id)
            && pane["cwd"].as_str() == Some(&j.route.cwd)
            && pane["terminal_id"].as_str() == Some(&j.terminal),
        "coordinator terminal incarnation changed before effect"
    );
    let md = paths::read_control_text(&dir.join("PROJECT.md"), 1024 * 1024)?
        .context("PROJECT.md missing")?;
    ensure!(
        crate::thread::sha256_hex(md.as_bytes()) == j.settings_digest
            && migration::config_reference(&ctx.config_dir.join("config.toml"))?.digest
                == j.config_digest,
        "coordinator settings or safety changed; inspect before --reprime"
    );
    Ok(())
}
/// Product-owned Claude settings. Absolute file rules use Claude's `//` syntax:
/// https://code.claude.com/docs/en/permissions#read-and-edit
fn permissions(ctx: &Ctx, dir: &Path, slug: &str) -> Result<String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    use std::io::Write;
    let parent = dir.join(".state/coordinator");
    std::fs::DirBuilder::new().mode(0o700).create(&parent).or_else(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists { Ok(()) } else { Err(e) }
    })?;
    ensure!(std::fs::symlink_metadata(&parent)?.file_type().is_dir(), "coordinator settings parent must be a real directory");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))?;
    let file = parent.join("claude-settings.json");
    let mut prefixes = vec![coordinator::current_prefix(&ctx.root)?];
    let alias = ctx.env.home.join("git/herdr-projects");
    if alias.canonicalize().ok() == Some(std::env::current_exe()?.canonicalize()?) {
        prefixes.push(coordinator::command_prefix(&alias, &ctx.root));
    }
    ensure!(prefixes.iter().all(|p| !p.contains(['*', '\n', '\r'])),
        "coordinator command prefix contains permission-rule wildcards or line breaks");
    let verbs = [
        "skill".into(), format!("context {slug}"), "inbox list".into(), "inbox done".into(),
        format!("task {slug} list"), format!("task {slug} show"), format!("task {slug} add"), format!("task {slug} rename"),
        format!("launch {slug} run"), format!("launch {slug} stop"),
        format!("result {slug} show"), format!("result {slug} jobs"), format!("result {slug} capture"), format!("result {slug} submit-captured"),
        format!("scheduler {slug} inspect"), format!("operations {slug} inspect"),
        format!("runtime {slug} inspect"), "doctor".into(),
    ];
    let mut allow = Vec::new();
    let mut ask = Vec::new();
    for prefix in &prefixes {
        for verb in &verbs { allow.push(format!("Bash({prefix} {verb}:*)")); }
        // Wildcards allow the planning flag in any argument position, never apply.
        allow.push(format!("Bash({prefix} reconcile {slug} --plan)"));
        allow.push(format!("Bash({prefix} reconcile {slug} --plan *)"));
        allow.push(format!("Bash({prefix} reconcile {slug} * --plan*)"));
        ask.push(format!("Bash({prefix} reconcile {slug} *--apply*)"));
        ask.push(format!("Bash({prefix} reconcile {slug} *--record*)"));
        for verb in ["attempts", "usage", "report", "query", "view", "watch", "compare", "recommend", "metrics registry", "workspace show", "workspace digest",
            "collectors status", "collectors bindings", "collectors sessions", "collectors tools", "collectors capabilities",
            "accounting status", "accounting entries", "accounting sessions", "accounting rate-cards", "accounting cost", "accounting charges", "accounting fx", "accounting budget-shadow", "accounting quota", "accounting attention", "accounting tools", "accounting fleet",
            "quality status", "quality flaky", "quality report", "quality groups present", "quality groups report", "quality groups show",
            "review status", "review present", "review show", "review report", "review findings show",
            "review fixes show", "review protocols show", "review experiments show", "review seeds show", "review seeds report", "review authority show", "review signer status",
            "analytics status", "analytics snapshot", "analytics revisions", "analytics plans", "experiments plan", "experiments report",
            "maintenance classes", "maintenance plan", "maintenance hold list", "backup list", "backup verify",
            "policies show", "policies simulate", "policies suggest", "policies shadow", "health alerts", "health rules", "health status"] {
            allow.push(format!("Bash({prefix} telemetry {slug} {verb}:*)"));
        }
        for verb in ["list", "show", "ingest", "propose", "reject", "defer", "remind", "record"] {
            allow.push(format!("Bash({prefix} memory-review {slug} {verb}:*)"));
        }
        // Preserve the documented legacy coordinator command surface.
        for verb in ["list", "overview", "safety show", "safety grant", "safety requests", "routine list", "thread list", "thread show", "thread prompt", "thread ack", "thread restart", "thread stop"] {
            allow.push(format!("Bash({prefix} {verb}:*)"));
        }
        for verb in ["safety approve", "safety reject", "safety revoke", "approval", "delegation", "budget", "routine-store", "migration", "archive", "delete"] {
            ask.push(format!("Bash({prefix} {verb}:*)"));
        }
        for verb in ["state", "rebind", "adopt", "relinquish"] { ask.push(format!("Bash({prefix} runtime * {verb}:*)")); }
        for verb in ["cancel-attempt", "complete"] { ask.push(format!("Bash({prefix} task * {verb}:*)")); }
    }
    let mut deny = vec!["Bash(ssh-keygen:*)".to_string()];
    for path in ["~/.config/herdr-projects/**".to_string(), "~/.config/herdr-farm/**".into(), "~/.ssh/**".into(),
        format!("/{}", file.display()), format!("/{}/.claude/**", dir.display())] {
        for tool in ["Read", "Edit", "Write"] { deny.push(format!("{tool}({path})")); }
    }
    // Publish atomically with restrictive permissions from the first byte;
    // create_new and rename never follow a pre-existing settings symlink.
    let temp = parent.join(format!(".settings-{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        output.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        output.write_all(&serde_json::to_vec_pretty(&json!({"permissions":{"allow":allow,"ask":ask,"deny":deny}}))?)?;
        output.sync_all()?;
        std::fs::rename(&temp, &file)?;
        std::fs::File::open(&parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temp); }
    result?;
    Ok(file.to_string_lossy().into_owned())
}

pub fn permission_status(env: &paths::Env, dir: &Path, runner: &dyn crate::runner::Runner) -> Result<String> {
    let md = paths::read_control_text(&dir.join("PROJECT.md"), 1024 * 1024)?.context("PROJECT.md missing")?;
    let (settings, _) = project::parse_project_md(&md)?;
    if settings.coordinator_agent != "claude" {
        return Ok(format!("Coordinator permissions: none generated for {} (Claude Code only).", settings.coordinator_agent));
    }
    let journal: Option<Journal> = paths::read_control_text(&dir.join(".state/canonical-coordinator.json"), 64 * 1024)?
        .map(|s| read_journal(&s)).transpose()?;
    let status = match journal {
        Some(j) if j.permission_file.is_some() => {
            let h = Herdr::new(env.herdr_bin(), &j.socket, runner);
            let live = h.agent_list().map(|agents| agents.iter().any(|a|
                a.pane_id == j.route.pane_id && a.workspace_id == j.route.workspace_id
                    && a.tab_id == j.route.tab_id && a.cwd == j.route.cwd && a.name == j.name));
            match live {
                Ok(true) => format!("generated file {} in effect for running coordinator (supplied at start)", j.permission_file.unwrap()),
                Ok(false) => "no running coordinator observed; generated settings retained for the previous start".into(),
                Err(_) => "running coordinator unconfirmed; generated settings were supplied at the recorded start".into(),
            }
        }
        _ => "no generated file recorded for the running coordinator".into(),
    };
    Ok(format!("Coordinator permissions: {status}; picked up only when open starts the coordinator agent."))
}

pub fn open(ctx: &Ctx, slug: &str, options: &OpenOptions) -> Result<()> {
    project::validate_slug(slug)?;
    let dir = ctx.root.join(slug).canonicalize()?;
    // Retain operator intent across the short root publication section. The
    // project guard excludes background effects during native I/O without
    // excluding foreground commands in other projects.
    let _intent = herdr_farm::execution_guard::CoordinatorOpenGuard::acquire(&dir)?;
    let guard = herdr_farm::execution_guard::ProjectGuard::acquire(&dir)?;
    let snapshot = runtime::snapshot(&dir).context(
        "legacy runtime is disabled; canonical coordinator requires an active migrated store",
    )?;
    ensure!(
        !matches!(
            snapshot.control.as_ref().map(|c| &c.state),
            Some(ProjectState::Archived)
        ),
        "unarchive canonical control before opening the coordinator"
    );
    let p = Project {
        root: ctx.root.clone(),
        slug: slug.into(),
    };
    let md =
        paths::read_control_text(&p.project_md(), 1024 * 1024)?.context("PROJECT.md missing")?;
    let (settings, _) = project::parse_project_md(&md)?;
    let safety = p.safety(&ctx.config_dir)?;
    let mut args = safety.coordinator_arguments(&settings.coordinator_agent)?.to_vec();
    if settings.coordinator_agent == "claude" {
        ensure!(!args.iter().any(|a| a == "--settings" || a.starts_with("--settings=")),
            "coordinator_agent_args must not contain --settings; open owns coordinator permissions");
    }
    let session = paths::resolve_session(&options.session, ctx.env, ctx.runner)?;
    let socket = session.socket.to_string_lossy().into_owned();
    let session_identity = crate::reconcile_live::resource_identity(&session.socket, true)
        .context("coordinator endpoint must be a live Unix socket")?;
    let h = Herdr::new(ctx.env.herdr_bin(), &session.socket, ctx.runner);
    ensure!(
        crate::herdr::version(&ctx.env.herdr_bin(), ctx.runner)? >= crate::herdr::MIN_VERSION,
        "canonical coordinator requires Herdr 0.9.1 or later"
    );
    let path = dir.join(".state/canonical-coordinator.json");
    let mut journal: Option<Journal> = paths::read_control_text(&path, 64 * 1024)?
        .map(|s| read_journal(&s))
        .transpose()?;
    if let Some(j) = &journal {
        ensure!(j.version == MIGRATIONS.len() as u32, "unsupported coordinator journal version");
        ensure!(
            matches!(
                j.phase.as_str(),
                "create-pending"
                    | "layout-ready"
                    | "start-ready"
                    | "start-pending"
                    | "prime-ready"
                    | "prime-pending"
                    | "accepted"
            ) && j.deliveries <= 3,
            "invalid coordinator journal phase or delivery count"
        );
        j.route.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            j.socket == socket || options.rebind,
            "coordinator journal belongs to a different session"
        );
    }
    let binding = snapshot
        .runtime_bindings
        .iter()
        .find(|b| b.id == "coordinator");
    let old_socket = journal
        .as_ref()
        .map(|j| j.socket.as_str())
        .or_else(|| binding.map(|b| b.identity.socket.as_str()))
        .unwrap_or("");
    if !old_socket.is_empty() && old_socket != socket {
        ensure!(
            !Path::new(old_socket).exists() && options.rebind,
            "coordinator belongs to {old_socket}; use its session, or --rebind after it is gone"
        );
        journal = None;
    }
    let panes = call(&h, "coordinator-panes", "pane.list", json!({}))?;
    let panes = panes["panes"]
        .as_array()
        .context("coordinator pane inventory missing")?;
    ensure!(
        panes.len() <= 256,
        "coordinator pane inventory exceeds bounds"
    );
    let previous = journal
        .as_ref()
        .map(|j| j.route.clone())
        .or_else(|| binding.map(|b| RuntimeRoute::from_identity(&b.identity)));
    let live = previous
        .as_ref()
        .filter(|r| r.socket == socket)
        .and_then(|r| {
            panes
                .iter()
                .find(|v| v["pane_id"].as_str() == Some(&r.pane_id))
        });
    if let (Some(r), Some(pane)) = (&previous, live) {
        ensure!(
            pane["workspace_id"].as_str() == Some(&r.workspace_id)
                && pane["tab_id"].as_str() == Some(&r.tab_id)
                && pane["cwd"].as_str() == Some(&r.cwd),
            "coordinator pane identity changed"
        );
    }
    if journal
        .as_ref()
        .is_some_and(|j| j.phase == "create-pending")
    {
        bail!(
            "coordinator workspace creation was interrupted; inspect Herdr and bind the observed coordinator with `runtime {slug} rebind` before removing the journal; creation is never blindly replayed"
        );
    }
    if live.is_none() && journal.is_some() {
        ensure!(
            options.reprime,
            "coordinator pane closed; run open {slug} --reprime"
        );
        journal = None;
    }
    // Prove absence on the recorded route before any replacement effect can
    // obscure that evidence (Herdr may reuse pane identifiers).
    if (live.is_none() || binding.is_some_and(|b| previous.as_ref().is_some_and(|r|
        *r != RuntimeRoute::from_identity(&b.identity))))
        && (options.reprime || journal.as_ref().is_some_and(|j| j.replace_missing))
        && let Some(b) = binding
        && let Some(owned) = snapshot.ownership.iter().find(|o| o.binding == b.id)
    {
        let batch = crate::reconcile_live::collect(ctx, &dir)?;
        let now = jiff::Timestamp::now().as_millisecond();
        ensure!(
            b.identity.machine.is_empty() && b.identity.worktree_path.is_empty()
                && owned.attempt.is_none() && owned.binding_revision == b.revision
                && batch.observations.iter().any(|o| o.binding == b.id
                    && o.binding_revision == b.revision && o.task_revision.is_none()
                    && o.collector == "herdr-git-v2"
                    && o.pane == herdr_farm::reconcile::ResourceState::Absent
                    && !o.agent_present && o.session_identity.is_some()
                    && o.session_identity == owned.session
                    && now >= o.observed_unix_ms && now - o.observed_unix_ms <= 30_000),
            "relinquish owned resources before rebinding; existing references are retained"
        );
        let head = runtime::record_observations_held(&dir, &batch)?;
        runtime::relinquish_held(&dir, &b.id, owned.revision, head, &guard,
            "open --reprime replaces freshly observed absent coordinator")?;
    }
    if journal.is_none() {
        let mut j = Journal {
            version: MIGRATIONS.len() as u32,
            socket: socket.clone(),
            session_identity,
            route: previous
                .clone()
                .filter(|_| live.is_some())
                .unwrap_or_default(),
            terminal: String::new(),
            kind: settings.coordinator_agent.clone(),
            name: coordinator::agent_name(slug),
            settings_digest: crate::thread::sha256_hex(md.as_bytes()),
            config_digest: migration::config_reference(&ctx.config_dir.join("config.toml"))?.digest,
            phase: "create-pending".into(),
            deliveries: 0,
            replace_missing: options.reprime,
            priming_manifest: None,
            permission_file: None,
        };
        if live.is_none() {
            save(&dir, &j)?;
            let created = h.workspace_create(&dir, &format!("hp-{slug}-coordinator"), true)?;
            j.route = RuntimeRoute {
                socket: socket.clone(),
                workspace_id: created.workspace_id,
                tab_id: created.tab_id,
                pane_id: created.pane_id,
                cwd: dir.to_string_lossy().into_owned(),
                ..Default::default()
            };
        }
        j.phase = "layout-ready".into();
        save(&dir, &j)?;
        let pane = call(&h, "coordinator-pane", "pane.list", json!({}))?;
        let found = pane["panes"]
            .as_array()
            .context("pane inventory missing")?
            .iter()
            .find(|a| a["pane_id"].as_str() == Some(&j.route.pane_id))
            .context("created coordinator pane absent")?;
        j.terminal = found["terminal_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("coordinator terminal missing")?
            .into();
        j.phase = "start-ready".into();
        save(&dir, &j)?;
        journal = Some(j);
    }
    let mut j = journal.unwrap();
    if j.phase == "layout-ready" {
        let pane = call(&h, "coordinator-layout", "pane.list", json!({}))?;
        let found = pane["panes"]
            .as_array()
            .context("pane inventory missing")?
            .iter()
            .find(|a| a["pane_id"].as_str() == Some(&j.route.pane_id))
            .context("coordinator pane absent")?;
        j.terminal = found["terminal_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("coordinator terminal missing")?
            .into();
        j.phase = "start-ready".into();
        save(&dir, &j)?;
    }
    // Owner config edits require fresh adoption, including focus-only opens.
    j.config_digest = migration::config_reference(&ctx.config_dir.join("config.toml"))?.digest;
    save(&dir, &j)?;
    if options.reprime {
        ensure!(
            j.kind == settings.coordinator_agent,
            "coordinator agent kind changed; close its old pane before open --reprime"
        );
        j.settings_digest = crate::thread::sha256_hex(md.as_bytes());
        j.config_digest = migration::config_reference(&ctx.config_dir.join("config.toml"))?.digest;
        save(&dir, &j)?;
    }
    ensure!(
        j.kind == settings.coordinator_agent
            && j.name == coordinator::agent_name(slug)
            && j.route.socket == j.socket
            && j.route.machine.is_empty()
            && j.route.cwd == dir.to_string_lossy()
            && !j.terminal.is_empty()
            && j.terminal.len() <= 256,
        "invalid canonical coordinator execution identity"
    );
    fence(&dir, ctx, &j)?;
    let current = runtime::snapshot(&dir)?;
    if current.runtime_bindings.iter().any(|b|
        b.id == "coordinator" && RuntimeRoute::from_identity(&b.identity) != j.route)
    {
        let batch = crate::reconcile_live::collect(ctx, &dir)?;
        runtime::record_observations_held(&dir, &batch)?;
    }
    // Never upgrade shared ownership. Only root-wide conflict validation and
    // binding publication require exclusivity; no native calls or waits here.
    drop(guard);
    let root_guard = herdr_farm::execution_guard::RootGuard::exclusive_by(
        &ctx.root,
        Instant::now() + timing::job_retry(),
        &crate::runner::Cancellation::default(),
    )?;
    let current = runtime::snapshot(&dir)?;
    crate::runtime_ownership::check_conflicts(
        ctx,
        &dir,
        Some("coordinator"),
        &herdr_farm::domain::RuntimeIdentity {
            socket: j.route.socket.clone(),
            workspace_id: j.route.workspace_id.clone(),
            tab_id: j.route.tab_id.clone(),
            pane_id: j.route.pane_id.clone(),
            cwd: j.route.cwd.clone(),
            ..Default::default()
        },
    )?;
    runtime::bind_coordinator_held(
        &dir,
        current.head,
        &j.route,
        options.reprime || j.replace_missing,
        &root_guard,
    )?;
    drop(root_guard);
    let guard = herdr_farm::execution_guard::ProjectGuard::acquire(&dir)?;
    let agents = call(&h, "coordinator-start-inventory", "agent.list", json!({}))?;
    let matching = agents["agents"]
        .as_array()
        .context("agent inventory missing")?
        .iter()
        .filter(|a| a["pane_id"].as_str() == Some(&j.route.pane_id))
        .count();
    ensure!(matching <= 1, "coordinator agent ambiguous");
    if matching == 0 {
        ensure!(
            j.phase == "start-ready" || options.reprime,
            "coordinator startup was interrupted; inspect before open --reprime"
        );
        j.phase = "start-pending".into();
        save(&dir, &j)?;
        fence(&dir, ctx, &j)?;
        if j.kind == "claude" {
            let file = permissions(ctx, &dir, slug)?;
            args.extend(["--settings".into(), file.clone()]);
            j.permission_file = Some(file);
            save(&dir, &j)?;
        }
        h.agent_start(&j.name, &j.kind, &j.route.pane_id, &args)?;
    }
    agent(&h, &j, false)?;
    if matches!(j.phase.as_str(), "start-ready" | "start-pending") {
        j.phase = "prime-ready".into();
        save(&dir, &j)?;
    }
    // Publish genuine live observations and authorize this operator-created runtime.
    let batch = crate::reconcile_live::collect(ctx, &dir)?;
    let head = runtime::record_observations_held(&dir, &batch)?;
    let s = runtime::snapshot(&dir)?;
    let b = s
        .runtime_bindings
        .iter()
        .find(|b| b.id == "coordinator")
        .context("coordinator binding missing")?;
    if !s.ownership.iter().any(|o| {
        o.binding == "coordinator"
            && o.binding_revision == b.revision
            && o.config_digest == j.config_digest
    }) {
        runtime::adopt_observed(
            &dir,
            "coordinator",
            b.revision,
            head,
            &migration::config_reference(&ctx.config_dir.join("config.toml"))?,
        )?;
    }
    if runtime::automatically_paused(&runtime::snapshot(&dir)?)
        && let Err(error) = crate::launch_run::activate_project(ctx, &dir, true)
    {
        match runtime::admission(&dir, &ctx.config_dir.join("config.toml")) {
            Ok(report) if !report.blockers.is_empty() => {
                for blocker in report.blockers { println!("Activation blocked: {blocker}"); }
            }
            _ => println!("Project remains paused: {}", error.to_string().replace(['\n', '\r'], " ")),
        }
    }
    if options.reprime {
        j.phase = "prime-ready".into();
        j.deliveries = 0;
        save(&dir, &j)?;
    }
    if j.phase == "prime-pending" {
        if accepted(&h, &j)? {
            j.phase = "accepted".into();
            save(&dir, &j)?;
        } else {
            bail!(
                "coordinator priming was interrupted and may have been submitted; inspect before open --reprime"
            );
        }
    }
    if j.phase == "prime-ready" {
        let prompt = coordinator::priming_prompt(&coordinator::current_prefix(&ctx.root)?, slug);
        while j.deliveries < 3 {
            j.priming_manifest = Some(ready(ctx, &h, &j)?);
            fence(&dir, ctx, &j)?;
            j.deliveries += 1;
            j.phase = "prime-pending".into();
            save(&dir, &j)?;
            let reply = call(
                &h,
                "coordinator-prime",
                "agent.prompt",
                json!({"target":j.route.pane_id,"text":prompt}),
            )?;
            ensure!(
                reply["type"].as_str() == Some("agent_prompted"),
                "coordinator priming acknowledgement mismatch"
            );
            herdr_farm::canonical_worker::validate_native_agent(
                &reply["agent"],
                &j.route,
                &j.terminal,
                &j.kind,
                Some(&j.name),
                false,
            )?;
            if accepted(&h, &j)? {
                j.phase = "accepted".into();
                save(&dir, &j)?;
                break;
            }
            j.phase = "prime-ready".into();
            save(&dir, &j)?;
        }
        ensure!(
            j.phase == "accepted",
            "coordinator stayed idle after three primings; inspect before open --reprime"
        );
    }
    let _ = h.agent_focus(&j.route.pane_id);
    coordinator::report_tokens(&h, slug, &j.route.pane_id);
    println!(
        "canonical coordinator is bound and primed in pane {}",
        j.route.pane_id
    );
    drop(guard);
    crate::ticker::start(ctx)?;
    Ok(())
}

pub fn commands(root: &Path, slug: &str) -> Result<String> {
    let p = coordinator::current_prefix(root)?;
    Ok(format!(
        "\nCanonical commands (replace uppercase values with retained IDs/paths):\n\
{p} context {slug}\n\
{p} task {slug} list\n\
{p} task {slug} show TASK\n\
{p} task {slug} add TASK --title TITLE --expected-head HEAD\n\
{p} launch {slug} run --task TASK --profile PROFILE --repository REPO --sign-with OWNER_KEY --plan-output docs/PLAN.md\n\
{p} launch {slug} run --task TASK --profile PROFILE --repository REPO --sign-with OWNER_KEY --write src/ --write tests/ --output src/lib.rs --prompt-file BRIEF\n\
{p} result {slug} show\n\
{p} result {slug} jobs\n\
{p} result {slug} verify SUBMISSION --policy-id POLICY --policy-file FILE --idempotency-key KEY --work-dir SCRATCH\n\
{p} result {slug} integrate RESULT --repository REPO --idempotency-key KEY --work-dir SCRATCH\n\
{p} operations {slug} inspect\n\
{p} inbox list {slug}\n\
{p} inbox done {slug} ITEM\n\
Thread commands are legacy-only. Start requires user authorization and owner-signed contracts and approvals; verification is evidence, integration requires the configured target; cleanup requires canonical finalization and proven worker termination. Never edit TASKS.md or old thread records as live state."
    ))
}

pub fn surface(ctx: &Ctx, slug: &str) -> Result<String> {
    let p = Project {
        root: ctx.root.clone(),
        slug: slug.into(),
    };
    let safety = p.safety(&ctx.config_dir)?;
    let snapshot = runtime::snapshot(&p.dir())?;
    let state = snapshot.control.as_ref().map(|c| format!("{:?}", c.state)).unwrap_or("unknown".into());
    let automatic = if runtime::automatically_paused(&snapshot) { format!("; run `open {slug}` to re-activate") } else { String::new() };
    let config = paths::read_control_text(&ctx.config_dir.join("config.toml"), 1024 * 1024)?
        .map(|s| toml::from_str::<toml::Value>(&s)).transpose()?;
    let mut store = migration::open_active(&p.dir())?;
    let mut profiles = Vec::new();
    if let Some(names) = config.as_ref().and_then(|c| c.get("profiles")).and_then(toml::Value::as_table) {
        for name in names.keys() {
            if store.latest_launchable_native_profile(name)?.is_some() { profiles.push(name.clone()); }
        }
    }
    let evidence = if profiles.is_empty() { "none".into() } else { profiles.join(", ") };
    let readiness = format!("Launchable retained profile evidence: {evidence} (launch revalidates current inputs).\nControl state: {state}{automatic}.\n{}\n", permission_status(ctx.env, &p.dir(), ctx.runner)?);
    Ok(format!(
        "{readiness}Safety: start_threads={} resolve_threads={} cleanup_resolved={}\nWith propose, request owner approval before dispatch or integration. With keep, retain artifacts and worktrees; never run destructive cleanup. Signing uses --sign-with OWNER_KEY or [coordinator].signing_key.\n{}",
        safety.start_threads,
        safety.resolve_threads,
        safety.cleanup_resolved,
        commands(&ctx.root, slug)?
    ))
}
