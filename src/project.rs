//! Project folders under the root: slugs, settings, status, the per-project
//! lock and the coordinator record.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const MAX_SLUG: usize = 40;
pub const BODY_WARN_CHARS: usize = 16_000;

/// A slug matches `[a-z0-9][a-z0-9-]*` and is at most 40 characters. Every
/// subcommand validates the slug it is given before building any path from it.
pub fn validate_slug(slug: &str) -> Result<()> {
    let mut chars = slug.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !first_ok || !rest_ok || slug.len() > MAX_SLUG {
        bail!("`{slug}` is not a valid slug (lower-case letters, digits and hyphens, at most {MAX_SLUG} characters)");
    }
    Ok(())
}

/// Lower-cases and turns each run of other characters into one hyphen. Used for
/// project names and for thread titles in branch names.
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.chars().take(MAX_SLUG).collect();
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// The slug `new` gives a project name, refusing names that look like paths.
pub fn slug_from_name(name: &str) -> Result<String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        bail!("a project name may not contain `/`, `\\` or `..`");
    }
    let slug = slugify(name);
    if slug.is_empty() {
        bail!("`{name}` has no letters or digits to make a slug from");
    }
    validate_slug(&slug)?;
    Ok(slug)
}

/// Writes through a temporary file in the same directory plus a rename. It never
/// creates parent directories: only `new` creates a project's directories.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    let name = path.file_name().context("path has no file name")?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("could not write {}", path.display()))
}

pub fn now() -> String {
    jiff::Timestamp::now()
        .round(jiff::Unit::Second)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Repo {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `PROJECT.md` front matter. `repos` is last so the TOML tables follow the
/// plain keys when `new` serializes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    pub goal: String,
    pub coordinator_agent: String,
    pub thread_agent: String,
    pub max_parallel_threads: u32,
    pub auto_resolve_days: u32,
    pub nudge: bool,
    /// Optional Git ref used by guarded legacy integrated resolution.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub integration_target: String,
    pub repos: Vec<Repo>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: String::new(),
            goal: String::new(),
            coordinator_agent: "claude".into(),
            thread_agent: "claude".into(),
            max_parallel_threads: 3,
            auto_resolve_days: 7,
            // Off by default: on herdr 0.9.1 a prompt merges with, and submits,
            // text the user has half-typed (docs/herdr-notes.md, stage 2). With
            // `false` the ticker shows a herdr notification instead.
            nudge: false,
            integration_target: String::new(),
            repos: Vec::new(),
        }
    }
}

/// Splits `+++` TOML front matter from the body.
pub fn parse_project_md(text: &str) -> Result<(Settings, String)> {
    let rest = text
        .strip_prefix("+++\n")
        .context("PROJECT.md must start with a `+++` line")?;
    let (front, body) = match rest.split_once("\n+++\n") {
        Some(parts) => parts,
        None => rest
            .strip_suffix("\n+++")
            .map(|front| (front, ""))
            .context("PROJECT.md front matter has no closing `+++` line")?,
    };
    let value: toml::Value = toml::from_str(front).context("PROJECT.md front matter does not parse")?;
    herdr_farm::profile_config::refuse_project_worker_uid(&value)?;
    let settings: Settings = toml::from_str(front).context("PROJECT.md front matter does not parse")?;
    Ok((settings, body.trim_start_matches('\n').to_string()))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Active,
    Paused,
    Archived,
    /// Unreadable or malformed persisted lifecycle state; never permits launch.
    #[serde(skip)]
    Invalid,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Status::Active => "active",
            Status::Paused => "paused",
            Status::Archived => "archived",
            Status::Invalid => "invalid (repair .state/project.json)",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub(crate) struct ProjectState {
    status: Status,
}

/// The coordinator's pane and the session the project belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Coordinator {
    pub socket: String,
    /// Empty when the session was chosen by socket path alone.
    pub session: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub agent_name: String,
    pub cwd: String,
    pub prime_pending: bool,
    pub prime_request:u64,
    pub prime_sequence:u64,
    pub prime_claim:Option<herdr_farm::coordinator_prime::Claim>,
    pub launch_sequence:u64,
    pub launch_claim:Option<herdr_farm::launch_claim::Claim>,
    pub launch_attempts: u32,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Safety {
    pub start_threads: String,
    pub cleanup_resolved: String,
    pub resolve_threads: String,
    pub coordinator_agent_args: Vec<String>,
    pub coordinator_agent_args_kind: Option<String>,
    pub thread_agent_args: Vec<String>,
    pub thread_agent_args_kind: Option<String>,
    pub thread_allowed_commands: Vec<String>,
    pub worker_permissions: String,
    pub worker_uid: String,
    pub grantable_commands: Vec<String>,
    pub thread_network: bool,
    pub thread_sandbox: bool,
    pub thread_wall_hours: u64,
    pub thread_env: Vec<String>,
    #[serde(skip)]
    pub thread_agent_args_explicit: bool,
    #[serde(skip)]
    pub active_worker_grants: Vec<String>,
    pub routine_commands: bool,
}

// Unsandboxed Claude launches only (owner opt-out or remote threads): PERM-1
// grants and thread_allowed_commands extend these Bash prefix permission rules.
// Sandboxed threads allow commands without prompts inside the sandbox boundary.
const CLAUDE_WORKER_COMMANDS: &[&str] = &[
    "git status", "git log", "git diff", "git show", "git branch",
    "git checkout", "git switch", "git add", "git commit", "git merge",
    "git rebase", "git cherry-pick", "git restore", "git rev-parse",
    "git ls-files", "git worktree list", "ls", "cat", "head", "tail", "wc",
    "grep",
    // Not `find` (-exec, -delete), `sed` (GNU `e` executes commands) or `rg`
    // (--pre runs a preprocessor): each can run an arbitrary command under an
    // allowed prefix.
];

impl Safety {
    pub fn worker_arguments(&self,kind:&str)->Result<&[String]> {crate::agents::arguments(kind,self.thread_agent_args_kind.as_deref(),&self.thread_agent_args,"thread_agent_args")}
    pub fn effective_worker_arguments(&self, kind: &str, cwd: &str, repository: &str) -> Result<Vec<String>> {
        let explicit = self.worker_arguments(kind)?;
        if self.thread_agent_args_explicit || !explicit.is_empty() {
            let mut args = explicit.to_vec();
            if kind == "claude" && !self.active_worker_grants.is_empty() {
                args.push("--allowedTools".into());
                args.extend(self.active_worker_grants.iter().map(|prefix| format!("Bash({prefix})")));
            }
            return Ok(args);
        }
        let mut args = Vec::new();
        match kind {
            "codex" => {
                let mut projects = toml::map::Map::new();
                for path in [cwd, repository] {
                    if path.is_empty() { continue; }
                    projects.insert(path.into(), toml::Value::Table(toml::map::Map::from_iter([("trust_level".into(), toml::Value::String("trusted".into()))])));
                }
                // Codex 0.159 splits override keys on every dot, ignoring quotes.
                // Keep paths in the TOML value, whose quoted keys are parsed correctly.
                args.extend(["-c".into(), format!("projects={}", toml::Value::Table(projects))]);
                args.extend(["--sandbox", "workspace-write", "--ask-for-approval", "on-request"].map(String::from));
                if self.thread_network { args.extend(["-c".into(), "sandbox_workspace_write.network_access=true".into()]); }
            }
            "claude" => {
                self.validate_thread_allowed_commands()?;
                args.extend(["--permission-mode", "acceptEdits", "--allowedTools"].map(String::from));
                args.extend(CLAUDE_WORKER_COMMANDS.iter().map(|prefix| format!("Bash({prefix}:*)")));
                args.extend(self.thread_allowed_commands.iter().map(|prefix| format!("Bash({prefix})")));
            }
            _ => {}
        }
        Ok(args)
    }
    /// Arguments for the sandboxed Claude launch path.
    #[allow(dead_code)] // Launch integration connects the legacy launcher to this prepared API.
    pub fn sandboxed_claude_arguments(&self, _cwd: &str, _repository: &str) -> Result<Vec<String>> {
        let explicit = self.worker_arguments("claude")?;
        crate::agents::validate_sandboxed_claude_arguments(explicit)?;
        let mut args = explicit.to_vec();
        args.extend(["--setting-sources".into(), "user".into()]);
        Ok(args)
    }
    pub(crate) fn validate_thread_allowed_commands(&self) -> Result<()> {
        anyhow::ensure!(self.thread_allowed_commands.len() <= 64, "thread_allowed_commands accepts at most 64 entries");
        for command in &self.thread_allowed_commands {
            let prefix = command.strip_suffix(":*").unwrap_or("");
            let executable = prefix.split_whitespace().next().unwrap_or("");
            anyhow::ensure!(
                command.len() <= 256 && !prefix.is_empty() && prefix.trim() == prefix
                    && prefix.bytes().all(|c| c.is_ascii_alphanumeric() || b" /._-".contains(&c))
                    && !executable.starts_with('-')
                    && executable.rsplit('/').next() != Some("sudo"),
                "invalid thread_allowed_commands entry {command:?}: use a command prefix ending in :*, at most 256 bytes, without shell metacharacters or leading sudo"
            );
        }
        Ok(())
    }
    pub fn worker_summary(&self) -> String {
        let mut kinds = vec!["codex", "claude", "other"];
        if let Some(kind) = self.thread_agent_args_kind.as_deref()
            && !kinds.contains(&kind) { kinds.push(kind); }
        kinds.into_iter().map(|kind| {
            let origin = if self.thread_agent_args_explicit || !self.thread_agent_args.is_empty() {
                "configured"
            } else if matches!(kind, "codex" | "claude") {
                "built-in defaults"
            } else {
                "no built-in arguments"
            };
            match self.effective_worker_arguments(kind, "<launch directory>", "<repository root>") {
                Ok(args) => format!("{kind}: {args:?} ({origin})"),
                Err(e) => format!("{kind}: {e}"),
            }
        }).collect::<Vec<_>>().join("; ")
    }
    pub fn coordinator_arguments(&self,kind:&str)->Result<&[String]> {crate::agents::arguments(kind,self.coordinator_agent_args_kind.as_deref(),&self.coordinator_agent_args,"coordinator_agent_args")}
}

impl Default for Safety {
    fn default() -> Self {
        Safety {
            start_threads: "propose".into(),
            cleanup_resolved: "auto".into(),
            resolve_threads: "propose".into(),
            coordinator_agent_args: Vec::new(),
            coordinator_agent_args_kind: None,
            thread_agent_args: Vec::new(),
            thread_agent_args_kind: None,
            thread_allowed_commands: Vec::new(),
            worker_permissions: "coordinator".into(),
            worker_uid: "root".into(),
            grantable_commands: Vec::new(),
            thread_network: false,
            thread_sandbox: true,
            thread_wall_hours: 168,
            thread_env: Vec::new(),
            thread_agent_args_explicit: false,
            active_worker_grants: Vec::new(),
            routine_commands: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub slug: String,
}

/// Held while reading and rewriting anything under `threads/`, `inbox/` or
/// `.state/`. Never held across a herdr, git, gh, ssh or scp call.
pub struct ProjectLock {
    _file: File,
}

impl Drop for ProjectLock {
    /// Unlock explicitly rather than by closing: a child forked by any other
    /// thread shares the open file description until it execs, so a bare close
    /// would leave the flock held for that window and a following
    /// non-blocking `.state/lock` taker (migration, runtime) would fail with
    /// "another operation owns lock". See `execution_guard::LockFile`.
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

impl Project {
    /// An existing project. Validates the slug before building any path.
    pub fn load(root: &Path, slug: &str) -> Result<Project> {
        validate_slug(slug)?;
        let project = Project {
            root: root.to_path_buf(),
            slug: slug.to_string(),
        };
        if !project.project_md().is_file() {
            bail!("no project `{slug}` in {}", root.display());
        }
        ensure_legacy(&project.dir())?;
        Ok(project)
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.slug)
    }

    pub fn project_md(&self) -> PathBuf {
        self.dir().join("PROJECT.md")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.dir().join(".state")
    }

    /// The canonical folder (symlinks resolved): the key of the project's
    /// `[safety]` table and of its routine approvals.
    pub fn canonical_dir(&self) -> PathBuf {
        std::fs::canonicalize(self.dir()).unwrap_or_else(|_| self.dir())
    }

    /// Takes the per-project lock. The lock file is opened without creating
    /// parent directories, and the project is re-checked afterwards, so a
    /// `delete` that lands mid-operation cannot be resurrected by a writer.
    pub fn lock(&self) -> Result<ProjectLock> {
        let path = self.state_dir().join("lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("project `{}` is gone ({})", self.slug, path.display()))?;
        file.lock()?;
        if !self.project_md().is_file() {
            bail!("project `{}` is gone", self.slug);
        }
        ensure_legacy(&self.dir())?;
        Ok(ProjectLock { _file: file })
    }

    pub fn read_project_md(&self) -> Result<(Settings, String)> {
        let text = std::fs::read_to_string(self.project_md())
            .with_context(|| format!("could not read {}", self.project_md().display()))?;
        parse_project_md(&text)
    }

    pub fn status(&self) -> Status { self.try_status().unwrap_or(Status::Invalid) }

    pub fn try_status(&self) -> Result<Status> {
        ensure_legacy(&self.dir())?;
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.state_dir().join("project.json");
        let file = match File::options().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Status::Active), // legacy default
            Err(error) => return Err(error).with_context(|| format!("cannot read {}; preserve and repair it", path.display())),
        };
        anyhow::ensure!(file.metadata()?.is_file(), "{} is not a regular lifecycle record", path.display());
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 65_536, "{} exceeds lifecycle record limit", path.display());
        let record: ProjectState = serde_json::from_slice(&bytes).with_context(|| format!("{} is invalid; preserve and repair it before resuming", path.display()))?;
        Ok(record.status)
    }

    pub fn set_status(&self, status: Status) -> Result<()> {
        let _lock = self.lock()?;
        self.try_status()?; // ordinary lifecycle commands must not erase corruption
        anyhow::ensure!(status != Status::Invalid, "invalid is a diagnostic, not a writable status");
        write_json(&self.state_dir().join("project.json"), &ProjectState { status })
    }

    pub fn coordinator(&self) -> Option<Coordinator> {
        read_json(&self.state_dir().join("coordinator.json"))
    }

    /// Read-modify-write of `coordinator.json` under the lock: re-reads the
    /// file, lets `change` touch only the fields its step owns, writes.
    pub fn update_coordinator(&self, change: impl FnOnce(&mut Coordinator)) -> Result<Coordinator> {
        self.update_coordinator_checked(|record|{change(record);Ok(())})
    }
    pub fn try_coordinator(&self)->Result<Option<Coordinator>> {
        let text=crate::paths::read_control_text(&self.state_dir().join("coordinator.json"),16*1024*1024)?;
        text.map(|text|serde_json::from_str(&text).context("invalid coordinator record")).transpose()
    }
    pub fn update_coordinator_checked(&self,change:impl FnOnce(&mut Coordinator)->Result<()>)->Result<Coordinator> {
        let _lock=self.lock()?;let mut record=self.try_coordinator()?.unwrap_or_default();change(&mut record)?;
        record.updated=now();let text=serde_json::to_string_pretty(&record)?+"\n";
        anyhow::ensure!(text.len()<=16*1024*1024,"coordinator update exceeds record limit");
        write_atomic(&self.state_dir().join("coordinator.json"),text.as_bytes())?;
        std::fs::File::open(self.state_dir())?.sync_all()?;Ok(record)
    }

    pub fn safety(&self, config_dir: &Path) -> Result<Safety> {
        load_safety(config_dir, &self.canonical_dir())
    }
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// The effective safety settings: `[safety."<canonical project path>"]` in
/// `<config_dir>/config.toml`, with defaults for an absent table or key.
pub fn load_safety(config_dir: &Path, canonical_project_dir: &Path) -> Result<Safety> {
    let file=config_dir.join("config.toml");
    let Some(text)=crate::paths::read_root_config(&file).with_context(||format!("cannot load safety settings from {}",file.display()))? else {return Ok(Safety::default());};
    herdr_farm::profile_config::validate_worker_uid_source(&toml::from_str(&text)?, &file, canonical_project_dir)?;
    parse_safety(&text,canonical_project_dir).with_context(||format!("invalid safety settings in {}",file.display()))
}

/// Parse an already bounded config snapshot without reopening its path.
pub fn parse_safety(text:&str,canonical_project_dir:&Path)->Result<Safety> {
    #[derive(Deserialize, Default)]
    struct Config {
        #[serde(default)]
        safety: std::collections::BTreeMap<String, Safety>,
    }
    let mut config: Config =
        toml::from_str(text).context("safety configuration does not parse")?;
    let mut safety = config
        .safety
        .remove(&*canonical_project_dir.to_string_lossy())
        .unwrap_or_default();
    let raw: toml::Value = toml::from_str(text)?;
    safety.thread_agent_args_explicit = raw.get("safety").and_then(|v| v.get(canonical_project_dir.to_string_lossy().as_ref())).and_then(|v| v.get("thread_agent_args")).is_some();
    if !matches!(safety.start_threads.as_str(), "propose" | "auto") {
        bail!(
            "start_threads must be \"propose\" or \"auto\", not {:?}",
            safety.start_threads
        );
    }
    anyhow::ensure!(matches!(safety.cleanup_resolved.as_str(), "auto" | "keep"), "cleanup_resolved must be auto or keep");
    anyhow::ensure!(matches!(safety.resolve_threads.as_str(), "propose" | "auto"), "resolve_threads must be propose or auto");
    anyhow::ensure!(matches!(safety.worker_permissions.as_str(), "coordinator" | "owner"), "worker_permissions must be coordinator or owner");
    herdr_farm::profile_config::parse_worker_uid(&raw, canonical_project_dir)?;
    anyhow::ensure!((1..=168).contains(&safety.thread_wall_hours), "thread_wall_hours must be between 1 and 168");
    herdr_farm::worker_supervision::validate_thread_env(&safety.thread_env)?;
    safety.validate_thread_allowed_commands()?;
    let extras = Safety { thread_allowed_commands: safety.grantable_commands.clone(), ..Safety::default() };
    extras.validate_thread_allowed_commands().context("invalid grantable_commands")?;
    Ok(safety)
}

/// Slugs of the projects in `root`: folders that contain `PROJECT.md`. Entries
/// whose names start with a dot are ignored. A missing root has no projects.
pub fn list_slugs(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut slugs: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.') && validate_slug(name).is_ok())
        .filter(|name| !is_creating(&root.join(name)) && root.join(name).join("PROJECT.md").is_file())
        .collect();
    slugs.sort();
    slugs
}

/// `PATH[@MACHINE]` as given to `new --repo`.
pub fn parse_repo_arg(arg: &str) -> Repo {
    if let Some((path, machine)) = arg.rsplit_once('@') {
        let label_like = !machine.is_empty()
            && machine
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if label_like && !path.is_empty() {
            return Repo {
                path: path.to_string(),
                machine: Some(machine.to_string()),
            };
        }
    }
    Repo {
        path: arg.to_string(),
        machine: None,
    }
}

const TASKS_TEMPLATE: &str = "# Tasks\n\n## Backlog\n";

const INSTRUCTIONS_TEMPLATE: &str = "\
# Instructions

Standing instructions for this project. Every thread starts from this text and
from the project's memory. Replace this paragraph with how you want work done:
conventions, what to check before finishing, what never to do.

The settings above, between the `+++` lines, are yours to edit. `nudge = true`
lets the ticker prompt the coordinator when something changed; it is off by
default because a prompt that arrives while you are typing in the coordinator
is merged with, and submits, your half-typed text. With it off you get a herdr
notification instead.
";

/// Creates the folder and skeleton files. The only code path that creates a
/// project's directories. Fails if the slug exists.
pub fn create(root: &Path, name: &str, goal: &str, repos: Vec<Repo>) -> Result<Project> {
    create_skeleton(root, name, goal, repos, false)
}

/// Canonical creation shared by the CLI and the plugin's new-project action.
pub fn create_canonical(root: &Path, config_dir: &Path, name: &str, goal: &str, repos: Vec<Repo>) -> Result<Project> {
    #[cfg(not(feature="state-store"))]
    { let _ = (root, config_dir, name, goal, repos); bail!("canonical creation requires the default state-store build; use new --legacy"); }
    #[cfg(feature="state-store")]
    {
        herdr_farm::owner_setup::prepare(config_dir)?;
        let project = create_skeleton(root, name, goal, repos, true)?;
        herdr_farm::migration::initialize_new_with_memory(&project.dir(), &std::path::absolute(config_dir.join("config.toml"))?, |dir| crate::launch_run::initialize_memory(config_dir, dir))?;
        Ok(project)
    }
}

pub fn is_creating(dir: &Path) -> bool {
    !matches!(std::fs::symlink_metadata(dir.join(".creating")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

pub fn creating_slugs(root: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(root).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| validate_slug(n).is_ok() && is_creating(&root.join(n))).collect();
    names.sort(); names
}

pub(crate) fn create_skeleton(root: &Path, name: &str, goal: &str, repos: Vec<Repo>, canonical: bool) -> Result<Project> {
    let slug = slug_from_name(name)?;
    let project = Project {
        root: root.to_path_buf(),
        slug: slug.clone(),
    };
    let dir = project.dir();
    if is_creating(&dir) {
        bail!("`{slug}` is creating; creation was interrupted or is still running. After confirming no new command is running, remove {} and run new again", dir.display());
    }
    if dir.exists() {
        bail!("`{slug}` already exists in {}", root.display());
    }
    let repos = repos
        .into_iter()
        .map(|repo| match repo.machine {
            // A remote path is stored as it is on its own machine.
            Some(_) => repo,
            None => Repo {
                path: std::fs::canonicalize(&repo.path)
                    .or_else(|_| std::path::absolute(&repo.path))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(repo.path),
                machine: None,
            },
        })
        .collect();
    let settings = Settings {
        name: name.to_string(),
        goal: goal.to_string(),
        repos,
        ..Settings::default()
    };
    let front = toml::to_string(&settings)?;

    std::fs::create_dir_all(root)?;
    std::fs::create_dir(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    if canonical { write_atomic(&dir.join(".creating"), b"canonical creation in progress\n")?; }
    for sub in ["memory", "scratch", "routines", "threads", "inbox", "inbox/done", "library", ".state"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    write_atomic(
        &dir.join("MEMORY.md"),
        b"# Memory\n\nOne line per memory file: `- [title](memory/file.md): what it holds`.\n",
    )?;
    write_atomic(&dir.join("TASKS.md"), TASKS_TEMPLATE.as_bytes())?;
    write_json(&project.state_dir().join("project.json"), &ProjectState { status: if canonical { Status::Paused } else { Status::Active } })?;
    // PROJECT.md last: a folder without it is not a project, so a half-made
    // skeleton is never picked up by `list` or the ticker.
    write_atomic(
        &project.project_md(),
        format!("+++\n{front}+++\n\n{INSTRUCTIONS_TEMPLATE}").as_bytes(),
    )?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writers_drop_their_write_when_project_md_is_gone() {
        let root = tempfile::tempdir().unwrap();
        let project = create(root.path(), "demo", "", vec![]).unwrap();
        std::fs::remove_file(project.project_md()).unwrap();
        assert!(project.update_coordinator(|c| c.pane_id = "w1:p1".into()).is_err());
        assert!(project.coordinator().is_none());

        // A deleted folder is not recreated by taking the lock.
        std::fs::remove_dir_all(project.dir()).unwrap();
        assert!(project.lock().is_err());
        assert!(!project.dir().exists());
    }
}

/// Always compiled, including legacy-only binaries. Never treat an unreadable,
/// newer, or interrupted ownership marker as permission to use legacy records.
pub fn ensure_legacy(dir: &Path) -> Result<()> {
    if is_creating(dir) { bail!("project is creating; wait for new to finish or rerun new for recovery instructions"); }
    for relative in [".state/format.json", ".state/migration/journal.json", ".state/migration/memory-journal.json"] {
        match std::fs::symlink_metadata(dir.join(relative)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e).with_context(|| format!("cannot inspect ownership marker {relative}")),
            Ok(_) => bail!("project uses migration/store maintenance; legacy runtime is disabled; use `migration status`, `recover` or `export` with the state-store build; reconciliation is required"),
        }
    }
    Ok(())
}
