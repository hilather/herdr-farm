//! `launch PROJECT run`: the operator sequence that takes one task on a migrated
//! project from nothing to a reserved canonical attempt, using only the
//! existing library steps. Every step checks the current state first, so a
//! rerun skips what is already done, and the first failing step stops the run
//! with its name. Automatic signing obeys the pinned owner policy and passes
//! the key path to `ssh-keygen`; this program never reads a private key itself.
use crate::paths::Ctx;
use anyhow::{Context, Result, bail, ensure};
use herdr_farm::{
    authority, domain::{AttemptState, ProjectState, RuntimeRoute, TaskId, VersionedReference}, execution_guard::GatedSpawn,
    launch_preparation::{self, LaunchSelection}, migration, runtime,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::{fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt}, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Beside a dedicated server's logs: its process id and socket.
const SERVER_RECORD: &str = "server.json";

pub struct Args {
    pub task: String,
    pub profile: String,
    pub repository: PathBuf,
    pub sign_with: Option<PathBuf>,
    pub validity_seconds: u64,
    pub title: Option<String>,
    /// Planning task: its single deliverable, a Markdown document under `docs/`.
    pub plan_output: Option<String>,
    pub review_of: Option<String>,
    pub fixes_review: Option<String>,
    pub work_item: Option<String>,
    pub role: Option<String>,
    pub supersedes: Option<String>,
    pub fixes: Vec<String>,
    pub review_kind: String,
    pub review_scope: String,
    pub write: Vec<String>,
    pub output: Vec<String>,
    pub accept: Vec<String>,
    pub no_default_accept: bool,
    pub deliverable: Option<String>,
    /// The task's instructions (appended to PROJECT.md in the retained brief).
    pub prompt_file: Option<PathBuf>,
    /// Unsigned contract decisions; product-owned fields are filled before signing.
    pub contract_file: Option<PathBuf>,
    /// Existing, not-checked-out branch that verified results integrate into.
    pub integration_ref: Option<String>,
    pub base: String,
    pub max_active_workers: Option<u32>,
    /// A Herdr server the operator already runs for this task (its control
    /// socket); by default a dedicated server is started.
    pub herdr_socket: Option<PathBuf>,
    /// Stop after the binding is reconciled and the project active, before any
    /// attempt is reserved (see the binding note in `steps`).
    pub prepare_only: bool,
}

struct Step {
    name: &'static str,
    outcome: &'static str,
    detail: Value,
}

struct Run<'a> {
    ctx: &'a Ctx<'a>,
    project: PathBuf,
    slug: String,
    steps: Vec<Step>,
}

impl Run<'_> {
    fn done(&mut self, name: &'static str, detail: Value) {
        self.steps.push(Step { name, outcome: "done", detail });
    }
    fn skipped(&mut self, name: &'static str, detail: Value) {
        self.steps.push(Step { name, outcome: "already_done", detail });
    }
    fn head(&self) -> Result<u64> {
        Ok(runtime::snapshot(&self.project)?.head)
    }
    fn dir(&self, task: &str) -> Result<PathBuf> {
        let dir = self.ctx.root.join(".herdr-run").join(format!("{}-{task}", self.slug));
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        Ok(dir)
    }
}

fn git(repository: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("/usr/bin/git").arg("-C").arg(repository).args(args).output_gated()?;
    ensure!(output.status.success(), "git {} failed in {}", args.join(" "), repository.display());
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Sign `file` for `namespace` into `file.sig` (a key the operator names).
fn sign(key: &Path, namespace: &str, file: &Path) -> Result<PathBuf> {
    let signature = PathBuf::from(format!("{}.sig", file.display()));
    let _ = fs::remove_file(&signature);
    // No controlling terminal: encrypted keys must already be in ssh-agent.
    let status = Command::new("/usr/bin/setsid").arg("/usr/bin/ssh-keygen").env("SSH_ASKPASS_REQUIRE", "never").stdin(Stdio::null()).args(["-Y", "sign", "-n", namespace, "-f"]).arg(key).arg(file).stdout(Stdio::null()).status_gated()
        .context("ssh-keygen could not be started")?;
    ensure!(status.success() && signature.is_file(), "the signing key needs a passphrase; load it into ssh-agent (ssh-keygen did not sign {})", file.display());
    Ok(signature)
}

// A head read before lock acquisition can become stale while ownership waits.
// Snapshot-at-head refusals use HistoryUnavailable; retain the same eight-pass
// head-race policy rather than treating them as lock contention.
fn retry<T>(mut step: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..8 {
        match step() {
            Ok(value) => return Ok(value),
            Err(error) if error.chain().any(|cause|
                matches!(cause.downcast_ref::<herdr_farm::store::StoreError>(), Some(herdr_farm::store::StoreError::Conflict | herdr_farm::store::StoreError::Busy | herdr_farm::store::StoreError::HistoryUnavailable(_)))
                    || cause.to_string().starts_with("project head changed during observation:")) => {
                if attempt == 7 {
                    let detail = error.chain().find_map(|cause| match cause.downcast_ref::<herdr_farm::store::StoreError>() {
                        Some(herdr_farm::store::StoreError::HistoryUnavailable(head)) => Some(format!(" (expected project store head {head} is no longer current)")),
                        _ => None,
                    }).unwrap_or_default();
                    return Err(error.context(format!("the project store head kept changing while launching{detail}; rerun the same command")));
                }
                std::thread::sleep(herdr_farm::timing::retry(Duration::from_millis(50 + 30 * attempt)));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn literal_path(raw: &str, file: bool) -> Result<String> {
    ensure!(!raw.contains(['*', '?', '[']), "scope paths must not contain globs");
    let (path, _) = herdr_farm::domain::normalize_scope_path(raw).map_err(anyhow::Error::msg)?;
    ensure!(!file || !path.ends_with('/'), "--output must name an exact file");
    Ok(path)
}

fn code_paths(args: &Args) -> Result<(Vec<String>, Vec<String>)> {
    ensure!(args.output.is_empty() || !args.write.is_empty(), "--output requires --write: name the paths the task may change");
    ensure!(args.write.len() <= 64, "give between one and 64 --write paths");
    ensure!(args.output.len() <= 8, "give at most 8 --output files");
    ensure!(args.write.is_empty() || !args.output.is_empty(), "--write requires --output: name at least one file the result must contain, an existing file the task changes or a new one such as <dir>/NOTES.md");
    let writes = args.write.iter().map(|p| literal_path(p, false)).collect::<Result<Vec<_>>>()?;
    let outputs = args.output.iter().map(|p| literal_path(p, true)).collect::<Result<Vec<_>>>()?;
    for output in &outputs {
        ensure!(writes.iter().any(|p| if p.ends_with('/') { output.starts_with(p) } else { output == p }), "--output {output} lies outside every --write path; add its write scope");
    }
    Ok((writes, outputs))
}

fn contract_document(args: &Args, repository: &Path, head: u64, kind: &str, project: &Path) -> Result<Vec<u8>> {
    let mut contract: Value = if let Some(path) = &args.contract_file {
        let value: Value = serde_json::from_slice(&migration::read_plan_file(path)?)?;
        ensure!(value["task_id"] == args.task, "contract task_id must equal --task {}", args.task);
        ensure!(value["profile_kind"] == kind, "contract profile_kind must equal the profile's kind {kind}");
        value
    } else if args.plan_output.is_some() {
        serde_json::from_slice(&planning_contract(args, repository, head, kind, project)?)?
    } else {
        let (writes, outputs) = code_paths(args)?;
        let mut policies = outputs.iter().enumerate().map(|(i, output)| {
            json!({"id":format!("output-{}", i + 1),"text":serde_json::to_string(&json!({"version":1,"checks":["/usr/bin/git","grep","--quiet","--no-index","-e",".","--",output]})).expect("JSON policy")})
        }).collect::<Vec<_>>();
        let mut explicit = Vec::new();
        for (index, acceptance) in args.accept.iter().enumerate() {
            let (name, command) = acceptance.split_once(':').context("--accept must be TOOLCHAIN:COMMAND [ARGS]")?;
            let checks = split_command(command)?;
            let text = herdr_farm::verification::toolchains::policy(project, name, checks.clone())?;
            explicit.push((name.to_owned(), checks));
            policies.push(json!({"id":format!("accept-{}", index + 1),"text":text}));
        }
        let mut default_count = 0;
        if !writes.is_empty() && !args.no_default_accept && args.review_of.is_none()
            && args.role.as_deref().is_none_or(|role| matches!(role, "build" | "fix")) {
            let key = project.canonicalize()?.to_string_lossy().into_owned();
            for (index, acceptance) in herdr_farm::verification::toolchains::default_accept(project)?.iter().enumerate() {
                let entry = format!("owner config verification.defaults.{key:?}.accept[{}]", index + 1);
                let (name, checks, text) = (|| -> Result<_> {
                    let (name, command) = acceptance.split_once(':').context("must be TOOLCHAIN:COMMAND [ARGS]")?;
                    let checks = split_command(command)?;
                    let text = herdr_farm::verification::toolchains::policy(project, name, checks.clone())?;
                    Ok((name.to_owned(), checks, text))
                })().with_context(|| entry)?;
                if !explicit.contains(&(name, checks)) {
                    policies.push(json!({"id":format!("accept-default-{}", index + 1),"text":text}));
                    default_count += 1;
                }
            }
        }
        let limit = if args.integration_ref.is_some() { herdr_farm::integration::MAX_POLICIES } else { 32 };
        ensure!(policies.len() <= limit,
            "generated contract has {} acceptance policies ({} outputs + {} defaults + {} explicit accepts), exceeding the {} route limit of {limit}; reduce policies or use --no-default-accept",
            policies.len(), outputs.len(), default_count, explicit.len(), if args.integration_ref.is_some() { "verify_then_integrate" } else { "verify_only" });
        json!({"version":3,"task_id":args.task,"profile_kind":kind,
            "scope":{"paths":writes.iter().map(|p| json!({"path":p,"access":"write"})).collect::<Vec<_>>()},
            "outputs":outputs.iter().map(|p| json!({"path":p,"kind":"git_file"})).collect::<Vec<_>>(),
            "acceptance_policies":policies,
            "deliverable":args.deliverable.clone().unwrap_or_else(|| format!("{}: the change described in the task instructions, inside the write paths.", args.title.as_deref().unwrap_or(&args.task))),
            "non_goals":"No change outside the write paths; no network use; no push.",
            "repository":repository,"base_oid":git(repository, &["rev-parse", "--verify", &format!("{}^{{commit}}", args.base)])?,
            "object_format":git(repository, &["rev-parse", "--show-object-format"])?,
            "dependencies":[],"capability_flags":[],"retry_class":"none","result_schema_id":"result-v1",
            "route":if args.integration_ref.is_some() { "verify_then_integrate" } else { "verify_only" }})
    };
    contract["project_store"] = json!(project.join(".state/state.db").canonicalize()?);
    contract["expected_head"] = json!(head);
    contract["contract_revision"] = json!(runtime::task_contract(project, &TaskId::new(args.task.clone()).map_err(anyhow::Error::msg)?)?.map_or(1, |r| r.revision + 1));
    contract["authority"] = serde_json::to_value(authority::policy_reference(project)?)?;
    Ok(serde_json::to_vec_pretty(&contract)?)
}

// Reruns may omit contract inputs, but supplied inputs must never be ignored.
fn check_requested_contract(args: &Args, repository: &Path, head: u64, kind: &str, project: &Path, stored: &Value) -> Result<()> {
    if args.write.is_empty() && args.output.is_empty() && args.accept.is_empty()
        && args.plan_output.is_none() && args.contract_file.is_none() && !args.no_default_accept {
        return Ok(());
    }
    let requested: Value = serde_json::from_slice(&contract_document(args, repository, head, kind, project)?)?;
    for field in ["scope", "outputs", "acceptance_policies"] {
        ensure!(stored[field] == requested[field],
            "task {} already has a frozen contract; requested {field} differs: old={}, new={}. Use a new task id with --supersedes {}",
            args.task, stored[field], requested[field], args.task);
    }
    Ok(())
}

fn planning_contract(args: &Args, repository: &Path, head: u64, kind: &str, project: &Path) -> Result<Vec<u8>> {
    let output = args.plan_output.as_deref().context("planning output missing")?;
    ensure!(
        output.starts_with("docs/") && output.ends_with(".md") && !output.contains("..") && !output.contains("//")
            && output.len() <= 200 && output.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)),
        "--plan-output must be a Markdown path under docs/ (letters, digits, `._-/`)"
    );
    let format = git(repository, &["rev-parse", "--show-object-format"])?;
    let base = git(repository, &["rev-parse", "--verify", &format!("{}^{{commit}}", args.base)])?;
    let route = if args.integration_ref.is_some() { "verify_then_integrate" } else { "verify_only" };
    let store = project.join(".state/state.db").canonicalize()?;
    // The check passes only for a document that exists and has content.
    let policy = serde_json::to_string(&json!({"version":1,"checks":["/usr/bin/git","grep","--quiet","--no-index","-e",".","--",output]}))?;
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "version":3,"outputs":[{"path":output,"kind":"git_file"}],"scope":{"paths":[{"path":output,"access":"write"}]},
        "project_store":store,"expected_head":head,"task_id":args.task,"contract_revision":1,
        "deliverable":if args.review_of.is_some() { args.deliverable.clone().unwrap_or_else(|| format!("The review report {output}, written in the working tree.")) } else { format!("The planning document {output}, written in the working tree.") },
        "non_goals":"No code, test or configuration change; no file other than the deliverable.",
        "acceptance_policies":[{"id":"document-present","text":policy}],
        "repository":repository,"base_oid":base,"object_format":format,"dependencies":[],"capability_flags":[],
        "profile_kind":kind,"retry_class":"none","result_schema_id":"result-v1","route":route,
        "authority":authority::policy_reference(project)?
    }))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// The closing section of the brief: commit changes inside write scopes and submit
/// the result through the worker's submission spool, with every value the
/// contract fixes already filled in. Without it nothing ever enters
/// verification or integration.
fn finish_instructions(root: &Path, slug: &str, contract: &Value, reference: &VersionedReference) -> Result<String> {
    let text = |key: &str| contract[key].as_str().with_context(|| format!("contract field {key} missing"));
    let outputs: Vec<&str> = contract["outputs"].as_array().context("contract outputs missing")?.iter().filter_map(|o| o["path"].as_str()).collect();
    ensure!(!outputs.is_empty() && outputs.len() <= 8, "a launched task declares between one and eight outputs");
    let writes: Vec<&str> = contract["scope"]["paths"].as_array().context("contract scope paths missing")?.iter()
        .filter(|p| p["access"] == "write").filter_map(|p| p["path"].as_str()).collect();
    ensure!(!writes.is_empty(), "contract write paths missing");
    let safe = |value: &str| !value.is_empty() && !value.contains(['$', '`', '\\', '\'', '"', '\n']);
    let (task, repository, base, format) = (text("task_id")?, text("repository")?, text("base_oid")?, text("object_format")?);
    ensure!([task, repository, base, format, reference.digest.as_str()].iter().all(|v| safe(v)) && safe(&root.display().to_string()) && outputs.iter().chain(writes.iter()).all(|o| safe(o)),
        "a path or value in the task contract contains a character the submission script cannot carry");
    let mut script = String::from("set -eu\nattempt=$(basename \"$HERDR_FARM_SUBMISSION_SPOOL\")\n");
    let patterns = writes.iter().map(|p| if p.ends_with('/') { format!("'{p}'*") } else { format!("'{p}'") }).collect::<Vec<_>>().join("|");
    script.push_str(&format!("check_scope() {{\nwhile IFS= read -r path; do\ncase \"$path\" in\n{patterns}) ;;\n*) printf '%s\\n' \"$path\" >&2; bad=1 ;;\nesac\ndone\n[ \"$bad\" = 0 ] || {{ echo 'Restore the paths above: verification cancels an attempt whose result changes anything outside its scope.' >&2; exit 1; }}\n}}\nbad=0\n{{ git -c core.quotePath=false diff --name-only --no-renames '{base}'; git -c core.quotePath=false ls-files --others --exclude-standard; }} | check_scope\n"));
    for output in &outputs {
        script.push_str(&format!("[ -f '{output}' ] || {{ echo 'declared output missing: {output}' >&2; exit 1; }}\n"));
    }
    for path in &writes {
        // Include tracked deletions even when the declared path is now absent.
        script.push_str(&format!("if [ -e '{path}' ] || [ -L '{path}' ] || [ -n \"$(git ls-files -- '{path}')\" ]; then git add -A -- '{path}'; fi\n"));
    }
    script.push_str("git -c user.name=worker -c user.email=worker@invalid commit -q -m 'Deliverable' || true\n");
    script.push_str(&format!("git -c core.quotePath=false diff --name-only --no-renames '{base}' HEAD | check_scope\ncandidate=$(git rev-parse HEAD)\n[ \"$candidate\" != '{base}' ] || {{ echo 'nothing is committed: write the deliverable first' >&2; exit 1; }}\n"));
    let mut manifest = Vec::new();
    for (index, output) in outputs.iter().enumerate() {
        script.push_str(&format!("blob{index}=$(git rev-parse \"HEAD:{output}\")\n"));
        manifest.push(format!(r#"{{"path":"{output}","oid":"$blob{index}"}}"#));
    }
    // Keep the candidate delta and both commit anchors in step with submit-captured.
    script.push_str(&"list() { { printf '%s\\n' 'BASE' \"$candidate\"; git rev-list --objects --no-object-names \"$candidate\" '^BASE'; } | sort -u; }\n\
n=$(list | wc -l)\n[ \"$n\" -le LIMIT ] || { echo \"the candidate delta and commit anchors hold $n objects; a submission carries at most LIMIT\" >&2; exit 1; }\n\
objects=$(list | while IFS= read -r oid; do prefix=$(printf %s \"$oid\" | cut -c1-2); suffix=$(printf %s \"$oid\" | cut -c3-); printf '{\"oid\":\"%s\",\"relative_path\":\"%s/%s\"}\\n' \"$oid\" \"$prefix\" \"$suffix\"; done | paste -sd, -)\n"
        .replace("BASE", base).replace("LIMIT", &herdr_farm::store::SUBMISSION_OBJECT_LIMIT.to_string()));
    script.push_str("key=result-$(printf %s \"$attempt\" | cut -c1-100)\ndocument=$(mktemp)\ncat > \"$document\" <<EOF\n");
    script.push_str(&format!(
        r#"{{"idempotency_key":"$key","task_id":"{task}","contract_revision":{},"contract_digest":"{}","attempt_id":"$attempt","repository":"{repository}","base_oid":"{base}","candidate_oid":"$candidate","object_format":"{format}","artifact_manifest":[{}],"claimed_checks":[],"objects":[$objects]}}"#,
        reference.revision, reference.digest, manifest.join(",")));
    script.push_str(&format!("\nEOF\nherdr-farm --root '{}' result {slug} submit --input-file \"$document\"\n", root.display()));
    Ok(format!(
        "\n## When the deliverable is complete: submit it\n\nA result that is not submitted is never verified or integrated. Run exactly this script in the current directory. It commits every change inside the write paths on your attempt branch and submits the result through your submission spool (the product's own command; it needs no network and is safe to run again). It must print a submission receipt containing `submission_id`; if it fails, fix what it reports and run it again. Only then stop and reply DONE.\n\n```sh\n{script}```\n"))
}

/// Record current observations and make the project active, unless it already
/// is, needs no reconciliation and acknowledges the owner configuration as it is
/// now (or `force` after a new binding). A configuration edited since control
/// was last made active is re-acknowledged: the signed contract, approvals and
/// reservations all require the active control to carry the current digest.
fn activate(run: &mut Run, name: &'static str, force: bool) -> Result<()> {
    let project = run.project.clone();
    let config = std::path::absolute(run.ctx.config_dir.join("config.toml"))?;
    let current = migration::config_reference(&config)?.digest;
    let snapshot = runtime::snapshot(&project)?;
    let control = snapshot.control.as_ref().context("project has no control state")?;
    ensure!(control.state != ProjectState::Paused || runtime::automatically_paused(&snapshot),
        "project is explicitly paused by the owner; resume it with `runtime state active` after reconciliation");
    if !force && control.state == ProjectState::Active && !control.reconciliation_required && control.config_digest == current {
        run.skipped(name, json!({"state":"active"}));
        return Ok(());
    }
    let reacknowledged = control.state == ProjectState::Active && control.config_digest != current;
    // Shared with `open`; retried because the ticker can move the head between collection and activation.
    retry(|| activate_project(run.ctx, &project, None))?;
    run.done(name, json!({"state":"active","owner_configuration_reacknowledged":reacknowledged}));
    Ok(())
}

/// Shared activation path: collect fresh evidence, then use store admission checks.
pub(crate) fn activate_project(ctx: &Ctx, project: &Path, held: Option<&herdr_farm::execution_guard::ProjectGuard>) -> Result<()> {
    let config = std::path::absolute(ctx.config_dir.join("config.toml"))?;
    let batch = crate::reconcile_live::collect(ctx, project)?;
    if held.is_some() { runtime::record_observations_held(project, &batch)?; }
    else { runtime::record_observations(project, &batch)?; }
    let snapshot = runtime::snapshot(project)?;
    // Open retains a project guard; re-check owner intent after native I/O.
    if snapshot.control.as_ref().is_some_and(|c| c.state == ProjectState::Active
        && !c.reconciliation_required && c.config_digest == migration::config_reference(&config).ok().and_then(|r| r.digest)) {
        return Ok(());
    }
    if held.is_some() && !runtime::automatically_paused(&snapshot) { return Ok(()); }
    if !runtime::automatically_paused(&snapshot)
        && snapshot.control.as_ref().is_some_and(|c| c.state == ProjectState::Paused) {
        bail!("project is explicitly paused by the owner; resume it with `runtime state active` after reconciliation");
    }
    let control = snapshot.control.context("project has no control state")?;
    match held {
        // `open` holds the project guard; a second execution lock would refuse.
        Some(guard) => runtime::set_state_held(project, snapshot.head, control.revision, ProjectState::Active, &config, guard)?,
        None => runtime::set_state(project, snapshot.head, control.revision, ProjectState::Active, &config)?,
    };
    Ok(())
}

fn herdr_server(run: &mut Run, herdr: &Path, task: &str, existing: Option<&Path>) -> Result<PathBuf> {
    use std::os::unix::fs::FileTypeExt;
    if let Some(socket) = existing {
        ensure!(socket.is_absolute() && fs::symlink_metadata(socket).is_ok_and(|m| m.file_type().is_socket()), "--herdr-socket must be the absolute path of an existing Herdr control socket");
        run.done("herdr_server", json!({"socket":socket,"managed_by":"operator"}));
        return Ok(socket.to_owned());
    }
    let directory = run.dir(task)?.join("herdr");
    for part in ["", "home", "runtime"] {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(directory.join(part))?;
    }
    // Reuse the persisted socket even if this invocation's runtime directory
    // changed. Never overwrite the only recovery record for a live old server.
    let old_record: Value = fs::read(directory.join(SERVER_RECORD)).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null);
    if let (Some(pid), Some(socket)) = (old_record["pid"].as_i64(), old_record["socket"].as_str()) {
        let socket = PathBuf::from(socket);
        if pid > 1 && pid <= i32::MAX as i64 && is_our_server(pid as i32, &socket) {
            if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                run.skipped("herdr_server", json!({"socket":socket}));
                return Ok(socket);
            }
            // Preserve its only recovery record. Explicit retirement applies
            // the attempt-retention fence before a later run can replace it.
            bail!("recorded private Herdr server is still running but unreachable; run launch {} stop --task {task} before retrying", run.slug);
        }
    }
    // The socket lives in a short private directory (sun_path is 108 bytes);
    // logs and configuration stay beside the run.
    let socket = herdr_farm::short_socket::stable(&directory)?.join("s");
    herdr_farm::short_socket::check_length(&socket)?;
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        ensure!(directory.join(SERVER_RECORD).is_file(), "private server socket has no recovery record; inspect before relaunching");
        run.skipped("herdr_server", json!({"socket":socket}));
        return Ok(socket);
    }
    close_viewer(run.ctx, &old_record);
    let _ = fs::remove_file(&socket);
    fs::write(
        directory.join("config.toml"),
        "onboarding = false\n[terminal]\ndefault_shell = '/bin/sh'\nshell_mode = 'non_login'\n[update]\nversion_check = false\nmanifest_check = false\n",
    )?;
    let log = fs::File::create(directory.join("server.log"))?;
    // Its own process group: the server outlives this command.
    let child = Command::new(herdr)
        .arg("server")
        .env_clear()
        .env("HOME", directory.join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8")
        .env("XDG_RUNTIME_DIR", directory.join("runtime"))
        .env("HERDR_CONFIG_PATH", directory.join("config.toml"))
        .env("HERDR_SOCKET_PATH", &socket)
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn_gated()
        .context("Herdr server could not be started")?;
    // The record `launch stop` and the ticker's sweep use to find (and prove
    // they have found) this server again.
    fs::write(directory.join(SERVER_RECORD), serde_json::to_vec_pretty(&json!({"pid":child.id(),"socket":socket,"project":run.slug,"task":task}))?)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::os::unix::net::UnixStream::connect(&socket).is_err() {
        ensure!(Instant::now() < deadline, "Herdr server did not open {} (see server.log beside it)", socket.display());
        std::thread::sleep(Duration::from_millis(50));
    }
    run.done("herdr_server", json!({"socket":socket,"log":directory.join("server.log")}));
    Ok(socket)
}

fn viewer_matches(ctx: &Ctx, viewer: &Value) -> Result<bool> {
    let field = |name: &str| viewer[name].as_str().with_context(|| format!("viewer missing {name}"));
    let h = crate::herdr::Herdr::new(ctx.env.herdr_bin(), field("socket")?, ctx.runner);
    Ok(h.tab_matches(field("workspace")?, field("tab")?, field("label")?)?)
}

fn close_viewer(ctx: &Ctx, record: &Value) {
    let viewer = &record["viewer"];
    if viewer_matches(ctx, viewer).unwrap_or(false) {
        let h = crate::herdr::Herdr::new(ctx.env.herdr_bin(), viewer["socket"].as_str().unwrap_or_default(), ctx.runner);
        let _ = h.tab_close(viewer["tab"].as_str().unwrap_or_default());
    }
}

fn open_viewer(ctx: &Ctx, project: &Path, directory: &Path, task: &str, socket: &Path, focus: bool) -> Result<Value> {
    let path = directory.join(SERVER_RECORD);
    let mut record: Value = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({"socket":socket,"managed_by":"operator"}));
    let viewer = &record["viewer"];
    if viewer_matches(ctx, viewer).unwrap_or(false) {
        if focus {
            crate::herdr::Herdr::new(ctx.env.herdr_bin(), viewer["socket"].as_str().context("viewer socket missing")?, ctx.runner)
                .tab_focus(viewer["tab"].as_str().context("viewer tab missing")?)?;
        }
        return Ok(viewer.clone());
    }
    let snapshot = runtime::snapshot(project)?;
    let binding = snapshot.runtime_bindings.iter().find(|b| b.id == "coordinator")
        .context("no coordinator session is bound")?;
    ensure!(binding.identity.machine.is_empty(), "coordinator session is remote");
    ensure!(!binding.identity.workspace_id.is_empty(), "coordinator workspace is missing");
    std::os::unix::net::UnixStream::connect(&binding.identity.socket)
        .context("coordinator session is unreachable")?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)?;
    let config = directory.join("viewer.toml");
    use std::io::Write;
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&config)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(b"onboarding = false\n[update]\nversion_check = false\nmanifest_check = false\n[experimental]\nallow_nested = true\n")?;
    let h = crate::herdr::Herdr::new(ctx.env.herdr_bin(), &binding.identity.socket, ctx.runner);
    let label = format!("worker: {task}");
    let created = h.tab_create(&binding.identity.workspace_id, project, &label, false)?;
    let viewer = json!({"socket":binding.identity.socket,"workspace":created.workspace_id,
        "tab":created.tab_id,"pane":created.pane_id,"label":label,"config":config});
    record["viewer"] = viewer.clone();
    if let Err(error) = fs::write(&path, serde_json::to_vec_pretty(&record)?) {
        close_viewer(ctx, &record);
        return Err(error.into());
    }
    let command = herdr_farm::worker_supervision::posix_command(&[
        "/usr/bin/env".into(), format!("HERDR_CONFIG_PATH={}", config.display()),
        format!("HERDR_SOCKET_PATH={}", socket.display()), ctx.env.herdr_bin().to_string(),
    ])?;
    if let Err(error) = h.pane_run(&created.pane_id, &command) {
        close_viewer(ctx, &record);
        record.as_object_mut().context("invalid server record")?.remove("viewer");
        fs::write(&path, serde_json::to_vec_pretty(&record)?)?;
        return Err(error.into());
    }
    if focus { h.tab_focus(&created.tab_id)?; }
    Ok(viewer)
}

fn viewer_report(result: Result<Value>) -> Value {
    result.unwrap_or_else(|error| json!({"status":"unavailable","reason":format!("{error:#}")}))
}

pub fn view(ctx: &Ctx, slug: &str, task: &str) -> Result<Value> {
    TaskId::new(task.to_owned()).map_err(anyhow::Error::msg)?;
    let project = ctx.root.join(slug).canonicalize()?;
    let directory = ctx.root.join(".herdr-run").join(format!("{slug}-{task}")).join("herdr");
    let record: Value = serde_json::from_slice(&fs::read(directory.join(SERVER_RECORD))?)?;
    let socket = PathBuf::from(record["socket"].as_str().context("server socket missing")?);
    std::os::unix::net::UnixStream::connect(&socket).context("task server is not running")?;
    Ok(json!({"task":task,"viewer":viewer_report(open_viewer(ctx, &project, &directory, task, &socket, true))}))
}

struct ProbeDirectory(PathBuf);
impl Drop for ProbeDirectory {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn resolve_signer(explicit: Option<PathBuf>, dirs: &[PathBuf], public: &str) -> Result<PathBuf> {
    let matches = |key: &Path| -> bool {
        let private = if key.extension().is_some_and(|e|e=="pub") { key.with_extension("") } else { key.to_owned() };
        let pub_path = PathBuf::from(format!("{}.pub", private.display()));
        fs::symlink_metadata(&private).is_ok_and(|m|m.is_file() && m.uid()==unsafe {libc::geteuid()} && m.mode() & 0o077==0)
            && fs::read_to_string(pub_path).is_ok_and(|v|v.split_whitespace().take(2).collect::<Vec<_>>().join(" ")==public)
    };
    if let Some(key) = explicit {
        let agent_public = key.extension().is_some_and(|e|e=="pub") && std::env::var_os("SSH_AUTH_SOCK").is_some()
            && fs::symlink_metadata(&key).is_ok_and(|m|m.is_file() && m.uid()==unsafe {libc::geteuid()} && m.mode() & 0o022==0)
            && fs::read_to_string(&key).is_ok_and(|v|v.split_whitespace().take(2).collect::<Vec<_>>().join(" ")==public);
        ensure!(matches(&key) || agent_public, "signing key does not match the owner approval key or fails file ownership/permissions policy: {}", key.display());
        return Ok(key);
    }
    let mut keys = Vec::new();
    for dir in dirs {
        if let Ok(entries)=fs::read_dir(dir) {
            for entry in entries { let key=entry?.path(); if key.extension().is_some_and(|e|e=="pub") {continue;}
                if matches(&key) {keys.push(key);} }
        }
    }
    keys.sort(); keys.dedup();
    ensure!(!keys.is_empty(), "no signing key matching the owner approval key was found in {}", dirs.iter().map(|d|d.display().to_string()).collect::<Vec<_>>().join(", "));
    ensure!(keys.len()==1, "more than one signing key matches the owner approval key: {}", keys.iter().map(|d|d.display().to_string()).collect::<Vec<_>>().join(", "));
    Ok(keys.remove(0))
}

fn on_path(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    for directory in std::env::split_paths(ctx.env.var("PATH").unwrap_or("/usr/bin:/bin")) {
        let path=directory.join(name);
        if path.is_file() { return Ok(path.canonicalize()?); }
    }
    bail!("{name} unavailable on PATH; run profile verify-interaction with explicit executables")
}

struct ProfilePlan {
    candidate: Option<(VersionedReference, String)>,
    reason: Option<String>,
    herdr: PathBuf,
    refresh: Option<(PathBuf, PathBuf)>,
}

fn profile_plan(ctx: &Ctx, project: &Path, args: &Args, observe: bool) -> Result<ProfilePlan> {
    ensure!(!args.profile.is_empty() && args.profile.len()<=64 && args.profile.as_bytes()[0].is_ascii_alphabetic() && args.profile.bytes().all(|c|c.is_ascii_alphanumeric() || b"_-".contains(&c)), "invalid profile name");
    let mut store=herdr_farm::store::SqliteStore::open(&project.join(".state/state.db"))?;
    let candidate = store.latest_native_profile(&args.profile)?;
    let retained = candidate.as_ref().map(|(r,_)|store.native_profile_report(r)).transpose()?.flatten();
    let frozen = retained.as_ref().map(|v|serde_json::from_value::<herdr_farm::domain::FrozenProfile>(v["preparation"]["profile"].clone())).transpose()?;
    if let Some(f)=&frozen {
        let mut current=f.clone(); current.config=migration::config_reference(Path::new(&f.config.path))?;
        herdr_farm::profile_config::check_worker_login(&current, project)?;
    }
    drop(store);
    let herdr = if let Some(path)=ctx.env.var("HERDR_BIN_PATH") { PathBuf::from(path) }
        else if let Some(f)=&frozen { PathBuf::from(&f.herdr.path) } else { on_path(ctx, "herdr")? };
    ensure!(herdr.is_absolute() && herdr.is_file(), "HERDR_BIN_PATH or profile Herdr must be an absolute real file");
    let reason = match &candidate {
        Some(_) if retained.as_ref().is_some_and(|v|v["preparation"]["launchable"] != true) => Some("latest profile evidence is not launchable".into()),
        Some(_) if frozen.as_ref().is_some_and(|f|Path::new(&f.herdr.path).canonicalize().ok()!=herdr.canonicalize().ok()) => Some("selected Herdr executable changed".into()),
        Some((reference,_)) if observe => herdr_farm::profile_preparation::revalidate(project, reference, Instant::now()+herdr_farm::profile_preparation::BUDGET, Default::default()).err().map(|e|format!("{e:#}")),
        Some(_) => None,
        None => Some(format!("no launchable evidence retained for profile {}", args.profile)),
    };
    let refresh = if reason.is_some() {
        let pinned=migration::status(project)?.plan.config.context("pinned config missing")?;
        let config:toml::Value=toml::from_str(&String::from_utf8(migration::read_plan_file(Path::new(&pinned.path))?)?)?;
        let kind=config.get("profiles").and_then(|p|p.get(&args.profile)).and_then(|p|p.get("kind")).and_then(|v|v.as_str()).context("profile kind missing; run profile verify-interaction")?;
        let agent=if let Some(f)=&frozen && Path::new(&f.agent.path).is_file() {PathBuf::from(&f.agent.path)} else {
            let path=on_path(ctx,kind)?;
            use std::io::Read;
            let mut prefix=[0;2]; fs::File::open(&path)?.read_exact(&mut prefix)?;
            if prefix==*b"#!" {
                let mise=on_path(ctx,"mise").context("script agent requires profile verify-interaction")?;
                let result=Command::new(mise).args(["which",kind]).output_gated()?;
                ensure!(result.status.success(), "run profile verify-interaction to select the agent executable");
                PathBuf::from(String::from_utf8(result.stdout)?.trim()).canonicalize()?
            } else {path}
        };
        let home=frozen.as_ref().and_then(|f|f.execution_home.as_ref()).map(PathBuf::from).unwrap_or_else(||ctx.env.home.join(".herdr-farm-homes").join(&args.profile));
        Some((agent,home))
    } else {None};
    Ok(ProfilePlan{candidate, reason, herdr, refresh})
}

pub fn run(ctx: &Ctx, slug: &str, mut args: Args) -> Result<Value> {
    let project = ctx.root.join(slug).canonicalize().with_context(|| format!("project {slug} not found"))?;
    // File locks do not alter SQLite bytes and span the entire multi-step launch.
    let memory_launch_guard = fs::File::open(project.join(".state/state.db"))?;
    memory_launch_guard.try_lock_shared().context("memory adoption is in progress; retry launch after it finishes")?;
    if args.work_item.is_some() || args.role.is_some() || args.supersedes.is_some() {
        ensure!(runtime::snapshot(&project)?.schema_version >= 72, "launch lineage requires upgrade-store to schema 72");
    }
    if args.fixes_review.is_some() || !args.fixes.is_empty() {
        ensure!(args.role.as_deref().is_none_or(|role| role == "fix"),
            "a fix launch is always role fix; --fixes/--fixes-review cannot be combined with --role other than fix");
    }
    migration::open_active(&project)?.check_fix_run(&args.task, args.fixes_review.as_deref(), &args.fixes, &args.profile)?;
    let role = args.role.clone().unwrap_or_else(|| {
        if args.review_of.is_some() && args.review_kind == "code" { "review" }
        else if args.review_of.is_some() && args.review_kind == "skeptical" { "skeptic" }
        else if args.fixes_review.is_some() || !args.fixes.is_empty() { "fix" }
        else if args.plan_output.is_some() { "plan" } else { "build" }.into()
    });
    if args.review_of.is_some() {
        if args.output.is_empty() && args.write.is_empty() && args.plan_output.is_none() && args.contract_file.is_none() {
            args.plan_output = Some(format!("docs/reviews/{}.md", args.task));
        } else if args.write.is_empty() && !args.output.is_empty() {
            args.write = args.output.clone();
        }
        let mut context = String::from_utf8(migration::read_plan_file(&project.join("PROJECT.md"))?)?;
        if let Some(path) = &args.prompt_file { context.push_str(&String::from_utf8(migration::read_plan_file(path)?)?); }
        context.push_str(args.title.as_deref().unwrap_or(&args.task));
        if let Some(task) = runtime::snapshot(&project)?.tasks.iter().find(|t| t.id.as_str() == args.task) { context.push_str(&task.title); }
        migration::open_active(&project)?.check_review_run(&args.task, args.review_of.as_deref().unwrap(), &args.review_kind, &args.review_scope, &args.profile, &context)?;
    }
    // Like the other preflight checks, hold the store only for this check.
    let work_item = {
        let mut store = migration::open_active(&project)?;
        let work_item = store.resolve_launch_work_item(&args.task, args.work_item.as_deref(), args.review_of.as_deref().or(args.fixes_review.as_deref()), &args.fixes)?;
        store.check_task_lineage(&args.task, &work_item, &role, args.supersedes.as_deref())?;
        work_item
    };
    args.work_item = Some(work_item);
    args.role = Some(role);
    eprintln!("launch run: preflight");
    let pinned = migration::status(&project)?.plan.config.context("migration has no pinned config")?;
    let config: toml::Value = toml::from_str(&String::from_utf8(migration::read_plan_file(Path::new(&pinned.path))?)?)?;
    let cap = config.get("launch").and_then(|v| v.get("max_workers")).map(|v| v.as_integer().context("launch.max_workers must be an integer")).transpose()?.unwrap_or(4);
    ensure!((1..=64).contains(&cap), "launch.max_workers must be in 1..64");
    args.max_active_workers = Some(args.max_active_workers.unwrap_or(cap as u32));
    let mut problems = Vec::new();
    if args.max_active_workers.unwrap() == 0 || args.max_active_workers.unwrap() > cap as u32 { problems.push("owner cap: --max-active-workers must be at least one and cannot exceed launch.max_workers".into()); }
    if config.get("profiles").and_then(|v|v.get(&args.profile)).is_none() { problems.push("owner profiles: profile is absent from current owner configuration".into()); }
    if let Err(error)=code_paths(&args) {problems.push(format!("task inputs: {error:#}"));}
    let snapshot = runtime::snapshot(&project)?;
    if snapshot.control.as_ref().is_some_and(|c| c.state == ProjectState::Paused) && !runtime::automatically_paused(&snapshot) {
        problems.push("project is explicitly paused by the owner; resume it with `runtime state active` after reconciliation".into());
    }
    let held = snapshot.attempts.iter().filter(|a| a.retains_capacity()).count();
    let existing = snapshot.tasks.iter().any(|t|t.id.as_str()==args.task && t.active_attempt.is_some());
    let cap_refusal=format!("owner cap: unfinished attempts plus this launch exceed launch.max_workers ({cap})");
    let above_cap=held + usize::from(!existing) > cap as usize;
    if above_cap { problems.push(cap_refusal.clone()); }
    let repository = args.repository.canonicalize()?;
    let mut unlisted_repository=false;
    match crate::project::parse_project_md(&String::from_utf8(migration::read_plan_file(&project.join("PROJECT.md"))?)?) {
        Ok((settings, _)) if settings.repos.iter().any(|r|r.machine.is_none() && Path::new(&r.path).canonicalize().is_ok_and(|p|p==repository)) => {},
        Ok(_) => { unlisted_repository=true; problems.push("project repositories: canonical repository is not a local repository listed in PROJECT.md".into()); },
        Err(error) => problems.push(format!("PROJECT.md: {error:#}")),
    }
    if let Some(path)=&args.contract_file {
        let document:Value=serde_json::from_slice(&migration::read_plan_file(path)?)?;
        if document["task_id"] != args.task {problems.push(format!("task inputs: contract task_id must equal --task {}", args.task));}
        if let Some(kind)=config.get("profiles").and_then(|p|p.get(&args.profile)).and_then(|p|p.get("kind")).and_then(|v|v.as_str())
            && document["profile_kind"] != kind {problems.push(format!("task inputs: contract profile_kind must equal the profile's kind {kind}"));}
        if !document["repository"].as_str().is_some_and(|p|Path::new(p).canonicalize().is_ok_and(|p|p==repository)) {
            problems.push("project repositories: contract repository must equal the selected project repository".into());
        }
    }
    let mut dirs = vec![Path::new(&pinned.path).parent().context("config directory missing")?.to_owned()];
    if !dirs.contains(&ctx.config_dir) { dirs.push(ctx.config_dir.clone()); }
    let public = config["authority"]["approval_public_key"].as_str().context("owner approval key missing")?;
    let signer = (|| {
        let explicit = if let Some(key)=&args.sign_with {Some(key.clone())}
            else if let Some(value)=config.get("coordinator").and_then(|v|v.get("signing_key")) {
                let key=value.as_str().context("coordinator.signing_key must be an absolute path")?;
                ensure!(Path::new(key).is_absolute(), "coordinator.signing_key must be an absolute path");
                Some(PathBuf::from(key))
            } else {None};
        resolve_signer(explicit, &dirs, public)
    })();
    match signer {
        Ok(key) => {
            let probe = ProbeDirectory(herdr_farm::short_socket::fresh()?);
            let file = probe.0.join("probe");
            let payload = b"herdr-farm automatic launch signing preflight v1\n";
            fs::write(&file, payload)?;
            match sign(&key, "preflight@herdr-projects", &file).and_then(|sig|authority::verify_signing_probe(&project, payload, &fs::read(sig)?)) {
                Ok(()) => args.sign_with = Some(key),
                Err(error) => problems.push(format!("signer: {error:#}")),
            }
        },
        Err(error) => problems.push(format!("signer: {error:#}")),
    }
    let mut store = herdr_farm::store::SqliteStore::open(&project.join(".state/state.db"))?;
    if let Some((reference, _)) = store.latest_native_profile(&args.profile)? {
        let report = store.native_profile_report(&reference)?.context("retained profile missing")?;
        let mut frozen:herdr_farm::domain::FrozenProfile=serde_json::from_value(report["preparation"]["profile"].clone())?;
        frozen.config=migration::config_reference(Path::new(&pinned.path))?;
        if let Err(error)=herdr_farm::profile_config::check_worker_login(&frozen, &project) {problems.push(format!("profile evidence: {error:#}"));}
        if report["preparation"]["profile"]["execution_home"].as_str().is_none() { problems.push("sandboxed workers: profile has no execution home".into()); }
    }
    let kind=config.get("profiles").and_then(|p|p.get(&args.profile)).and_then(|p|p.get("kind")).and_then(|v|v.as_str());
    let request_digest=(|| -> Result<String> {
        let document=match store.task_contract_document(&args.task)? {
            Some(document)=>{
                check_requested_contract(&args, &repository, snapshot.head, kind.context("profile kind missing")?, &project, &document)?;
                document
            },
            None=>serde_json::from_slice(&contract_document(&args,&repository,snapshot.head,kind.context("profile kind missing")?,&project)?)?,
        };
        Ok(herdr_farm::store::owner_requests::contract_digest(document))
    })();
    if let Err(error) = &request_digest { problems.push(format!("contract: {error:#}")); }
    if above_cap && let Ok(digest)=&request_digest
        && store.owner_cap_exemption(&args.task,digest,jiff::Timestamp::now().as_millisecond())? {
        problems.retain(|p|p!=&cap_refusal);
    }
    let needs_activation=snapshot.control.as_ref().is_none_or(|c|c.state!=ProjectState::Active || c.reconciliation_required || c.config_digest.as_deref()!=migration::config_reference(Path::new(&pinned.path)).ok().and_then(|r|r.digest).as_deref());
    if needs_activation {
        for blocker in runtime::admission(&project, Path::new(&pinned.path))?.blockers {
            // Reconciliation supplies fresh observations, but cannot drain work.
            let live_identity=snapshot.runtime_bindings.iter().any(|b|blocker.starts_with(&format!("{}:",b.id)) && (!b.identity.pane_id.is_empty() || !b.identity.worktree_path.is_empty()));
            if !blocker.contains("fresh") || live_identity {problems.push(format!("admission: {blocker}"));}
        }
    }
    eprintln!("launch run: profile_evidence");
    let plan=profile_plan(ctx, &project, &args, problems.is_empty());
    if let Err(error)=&plan {problems.push(format!("profile evidence: {error:#}"));}
    problems.dedup();
    let repository_refusal="project repositories: canonical repository is not a local repository listed in PROJECT.md";
    if !problems.is_empty() && problems.iter().all(|p|p==&cap_refusal || p==repository_refusal) {
        let digest=request_digest.context("cannot bind owner request to contract decisions")?;
        for action in ["cap","repository"] {
            if (action=="cap" && problems.contains(&cap_refusal)) || (action=="repository" && unlisted_repository) {
                let request=store.request_owner(action,&args.task,&digest,&repository.display().to_string(),slug,jiff::Timestamp::now().as_millisecond())?;
                let command=format!("{} owner {slug} approve {} --summary {}",crate::coordinator::current_prefix(&ctx.root)?,request.id,crate::remote::quote(&request.summary));
                problems.push(format!("owner request {}: {command}",request.id));
            }
        }
    }
    ensure!(problems.is_empty(), "launch run preflight refused:\n{}", problems.join("\n"));
    let mut run = Run { ctx, project: project.clone(), slug: slug.to_owned(), steps: Vec::new() };
    match steps(&mut run, &args, plan.context("profile evidence preflight failed")?) {
        Ok(report) => Ok(report),
        Err(error) => {
            let failed = run.steps.len() + 1;
            let done: Vec<String> = run.steps.iter().map(|s| format!("{} ({})", s.name, s.outcome)).collect();
            let reason = format!("{error:#}").split_whitespace().collect::<Vec<_>>().join(" ");
            bail!("launch run stopped at step {failed} (launch failed: refused: {reason}); completed before it: [{}]. Fix the cause and rerun the same command; finished steps are skipped.", done.join(", "))
        }
    }
}

fn steps(run: &mut Run, args: &Args, plan: ProfilePlan) -> Result<Value> {
    let ctx = run.ctx;
    let project = run.project.clone();
    let task_id = TaskId::new(args.task.clone()).map_err(anyhow::Error::msg)?;

    let repository = args.repository.canonicalize().context("--repository must exist")?;
    code_paths(args)?;
    ensure!((usize::from(args.plan_output.is_some()) + usize::from(args.contract_file.is_some()) + usize::from(!args.write.is_empty())) == 1
        || runtime::task_contract(&project, &task_id)?.is_some(),
        "give --plan-output, --write with --output, or --contract-file");

    // 1. Refresh is a separately retained, resumable step.
    let ProfilePlan{candidate,reason,herdr,refresh}=plan;
    let (profile, kind) = if let Some((agent,home))=refresh {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&home)?;
        let verified=herdr_farm::profile_preparation::verify_interaction(&project,&args.profile,&herdr,&agent,&home,Instant::now()+Duration::from_secs(120),Default::default())?;
        let report=serde_json::to_value(&verified)?;
        ensure!(report["preparation"]["launchable"]==true, "profile verify-interaction did not produce launchable evidence");
        let value:(VersionedReference,String)=(serde_json::from_value(report["preparation"]["reference"].clone())?,
            report["preparation"]["profile"]["kind"].as_str().context("verified profile kind missing")?.to_owned());
        verified.retain(&project)?;
        eprintln!("launch run: profile_evidence: refreshed ({})", reason.as_deref().unwrap_or("missing evidence"));
        run.steps.push(Step{name:"profile_evidence",outcome:"refreshed",detail:json!({"profile":value.0,"kind":value.1,"reason":reason})});
        value
    } else {
        let value=candidate.context("profile evidence missing")?;
        run.skipped("profile_evidence",json!({"profile":value.0,"kind":value.1})); value
    };

    let worker_wall_seconds = crate::agents::resolve::resolve(&args.profile, &ctx.config_dir.join("config.toml"), None)?
        .budget.max_wall_seconds.context("profile max_wall_seconds missing")?;

    if let Some(path) = &args.contract_file {
        let value: Value = serde_json::from_slice(&migration::read_plan_file(path)?)?;
        ensure!(value["task_id"] == args.task, "contract task_id must equal --task {}", args.task);
        ensure!(value["profile_kind"] == kind, "contract profile_kind must equal the profile's kind {kind}");
    }

    // 2. The owner configuration must be acknowledged by an active project
    // before a signed contract can be installed.
    eprintln!("launch run: project_control");
    activate(run, "project_control", false)?;

    // 3. The task and its signed contract.
    eprintln!("launch run: task");
    let snapshot = runtime::snapshot(&project)?;
    if snapshot.tasks.iter().any(|t| t.id == task_id) {
        run.skipped("task", json!({"task":args.task}));
    } else {
        let head = retry(|| runtime::add_task(&project, task_id.clone(), args.title.clone().unwrap_or_else(|| args.task.clone()), run.head()?))?;
        run.done("task", json!({"task":args.task,"head":head}));
    }
    eprintln!("launch run: contract");
    if let Some(installed) = runtime::task_contract(&project, &task_id)? {
        let document = migration::open_active(&project)?.task_contract_document(&args.task)?.context("installed contract document missing")?;
        check_requested_contract(args, &repository, run.head()?, &kind, &project, &document)?;
        run.skipped("contract", json!({"installed":installed}));
    } else {
        let dir = run.dir(&args.task)?;
        let document = dir.join("contract.json");
        // Resolve the base once, including across retries of the signing ingress.
        let contract: Value = serde_json::from_slice(&contract_document(args, &repository, run.head()?, &kind, &project)?)?;
        fs::write(&document, serde_json::to_vec_pretty(&contract)?)?;
        let key = args.sign_with.as_deref().with_context(|| format!("the contract needs the owner's signature: pass --sign-with KEY, or sign {} with `ssh-keygen -Y sign -n {} -f KEY` and install it with `task contract put`", document.display(), authority::CONTRACT_SIGNATURE_NAMESPACE))?;
        let installed = retry(|| {
            let mut contract = contract.clone();
            contract["expected_head"] = json!(run.head()?);
            let bytes = serde_json::to_vec_pretty(&contract)?;
            fs::write(&document, &bytes)?;
            let signature = sign(key, authority::CONTRACT_SIGNATURE_NAMESPACE, &document)?;
            authority::import_contract(&project, &document, &signature)
        })?;
        run.done("contract", json!({"task":installed.task_id,"revision":installed.contract_revision,"digest":installed.digest}));
    }

    // 4. Capacity, queue and the integration target.
    let snapshot = runtime::snapshot(&project)?;
    eprintln!("launch run: queue");
    let scheduler = snapshot.scheduler.as_ref().context("project has no scheduler state: migrate it first")?;
    if snapshot.scheduler.as_ref().is_some_and(|s| s.queue.iter().any(|q| q.task == task_id) && snapshot.tasks.iter().any(|t| t.id == task_id && (t.state == herdr_farm::domain::TaskState::Queued || t.active_attempt.is_some()))) {
        run.skipped("queue", json!({"task":args.task}));
    } else {
        let request = herdr_farm::domain::QueueRequest { priority: 0, dependencies: vec![] };
        let head = retry(|| {
            let fresh = runtime::snapshot(&project)?;
            let revision = fresh.tasks.iter().find(|t| t.id == task_id).context("task missing")?.revision;
            runtime::queue_task(&project, &task_id, revision, fresh.head, &request)
        })?;
        run.done("queue", json!({"head":head}));
    }
    eprintln!("launch run: scheduler_capacity");
    let scheduler_policy = &scheduler.policy;
    if scheduler_policy.max_active_workers == args.max_active_workers.unwrap() {
        run.skipped("scheduler_capacity", json!({"max_active_workers":scheduler_policy.max_active_workers}));
    } else {
        let head = retry(|| {
            let fresh = runtime::snapshot(&project)?;
            let policy = fresh.scheduler.context("scheduler missing")?.policy;
            runtime::scheduler_policy(&project, fresh.head, policy.revision, args.max_active_workers.unwrap(), policy.max_attempts_per_task.max(1))
        })?;
        run.done("scheduler_capacity", json!({"max_active_workers":args.max_active_workers,"head":head}));
    }
    retry(|| herdr_farm::store::set_project_result_automation(&project, run.head()?, Some(true), None))?;
    eprintln!("launch run: integration_target");
    if let Some(reference) = &args.integration_ref {
        retry(|| herdr_farm::integration::configure_project(&project, &repository, reference))?;
        retry(|| herdr_farm::store::set_project_result_automation(&project, run.head()?, Some(true), Some(true)))?;
        run.done("integration_target", json!({"repository":repository,"reference":reference,"automation":"verify and integrate on"}));
    }

    // 5. Herdr server, binding, reconciliation and activation.
    eprintln!("launch run: herdr_server");
    let socket = herdr_server(run, &herdr, &args.task, args.herdr_socket.as_deref())?;
    let directory = run.ctx.root.join(".herdr-run").join(format!("{}-{}", run.slug, args.task)).join("herdr");
    // A server the operator supplied (`--herdr-socket`) is already a session he
    // watches; a viewer would only nest it inside itself.
    let viewer = if args.herdr_socket.is_some() {
        json!({"status":"unavailable","reason":"workers run in the operator-supplied Herdr session"})
    } else {
        viewer_report(open_viewer(run.ctx, &project, &directory, &args.task, &socket, false))
    };
    run.done("viewer", viewer);
    eprintln!("launch run: binding");
    let snapshot = runtime::snapshot(&project)?;
    if let Some(old) = snapshot.runtime_bindings.iter().find(|b| b.task.as_ref() == Some(&task_id)) {
        let history: Vec<_> = snapshot.attempts.iter().filter(|a| a.task == task_id).collect();
        if !history.is_empty() && history.iter().all(|a| !a.retains_capacity()
            && matches!(a.state, AttemptState::Completed | AttemptState::Failed | AttemptState::Cancelled | AttemptState::Lost))
            && (!old.identity.pane_id.is_empty() || !old.identity.worktree_path.is_empty() || old.identity.socket != socket.display().to_string()) {
            retry(|| {
                let fresh = runtime::snapshot(&project)?;
                runtime::pause_for_relaunch(&project, fresh.head, fresh.control.context("control missing")?.revision)
            })?;
            if let Some(owned) = snapshot.ownership.iter().find(|o| o.binding == old.id) {
                retry(|| {
                    let fresh = runtime::snapshot(&project)?;
                    let revision = fresh.ownership.iter().find(|o| o.binding == old.id).map_or(owned.revision, |o| o.revision);
                    runtime::relinquish(&project, &old.id, revision, fresh.head, "relaunch after observed worker termination; retain old worktree")
                })?;
            }
            let route = RuntimeRoute { socket: socket.display().to_string(), cwd: repository.display().to_string(), ..Default::default() };
            retry(|| {
                let fresh = runtime::snapshot(&project)?;
                let revision = fresh.runtime_bindings.iter().find(|b| b.id == old.id).context("binding missing")?.revision;
                runtime::rebind(&project, &old.id, revision, fresh.head, &route)
            })?;
        }
    }
    let snapshot = runtime::snapshot(&project)?;
    let existing = snapshot.runtime_bindings.iter().find(|b| b.task.as_ref() == Some(&task_id) && b.identity.socket == socket.display().to_string());
    let (binding, created) = match existing {
        Some(binding) => {
            run.skipped("binding", json!({"binding":binding.id}));
            (binding.id.clone(), false)
        }
        None => {
            let route = RuntimeRoute { socket: socket.display().to_string(), cwd: repository.display().to_string(), ..Default::default() };
            let change = retry(|| {
                let fresh = runtime::snapshot(&project)?;
                let revision = fresh.tasks.iter().find(|t| t.id == task_id).context("task missing")?.revision;
                runtime::create_binding(&project, Some(&task_id), Some(revision), fresh.head, &route)
            })?;
            run.done("binding", json!({"binding":change.binding.id,"socket":socket}));
            (change.binding.id, true)
        }
    };
    eprintln!("launch run: reconcile_and_activate");
    activate(run, "reconcile_and_activate", created)?;

    migration::open_active(&project)?.prepare_task_lineage(&args.task, args.work_item.as_deref().context("work item missing")?, args.role.as_deref().context("role missing")?, args.supersedes.as_deref())?;
    if args.fixes_review.is_some() || !args.fixes.is_empty() {
        migration::open_active(&project)?.prepare_fix_run(&args.task, args.fixes_review.as_deref(), &args.fixes, &args.profile, jiff::Timestamp::now().as_millisecond())?;
    }
    if args.prepare_only {
        return Ok(report(run, &args.task, &profile, &kind, &herdr, &socket, None, None, worker_wall_seconds));
    }

    // 6. Already reserved? Then stop here: a rerun never reserves a second attempt.
    let snapshot = runtime::snapshot(&project)?;
    if let Some(attempt) = snapshot.tasks.iter().find(|t| t.id == task_id).and_then(|t| t.active_attempt.clone()) {
        run.skipped("reserve", json!({"attempt":attempt}));
        return Ok(report(run, &args.task, &profile, &kind, &herdr, &socket, Some(attempt.as_str().to_owned()), None, worker_wall_seconds));
    }

    // 7. Knowledge snapshot, draft, owner approval, import, reservation.
    let dir = run.dir(&args.task)?;
    let mut instructions = String::from_utf8(migration::read_plan_file(&project.join("PROJECT.md"))?).map_err(|_| anyhow::anyhow!("project instructions are not UTF-8"))?;
    if let Some(path) = &args.prompt_file {
        let text = String::from_utf8(migration::read_plan_file(path)?).map_err(|_| anyhow::anyhow!("--prompt-file is not UTF-8"))?;
        if args.review_of.is_some() { instructions.push_str(&format!("\n\n## Review instructions\n\n{text}\n")); }
        else { instructions.push_str(&format!("\n\n---\n\n# Task {}\n\n{text}\n", args.task)); }
    }
    if let Some(output) = &args.plan_output {
        instructions.push_str(&format!(
            "\n## Deliverable\n\nWrite the document `{output}` in the current directory (a disposable git worktree), with the {} as its content. Do not change any other file, do not push and do not use the network.\n", if args.review_of.is_some() { "review report" } else { "plan" }));
    }
    let contract = migration::open_active(&project)?.task_contract_document(task_id.as_str())?.context("the task has no installed contract document")?;
    let reference = runtime::task_contract(&project, &task_id)?.context("the task has no installed contract")?;
    instructions.push_str(&finish_instructions(&ctx.root, run.slug.as_str(), &contract, &reference)?);
    eprintln!("launch run: knowledge_snapshot");
    let knowledge = retry(|| {
        let _guard = herdr_farm::memory::mutation_guard(&project)?;
        let resolved = crate::agents::resolve::resolve(&args.profile, &ctx.config_dir.join("config.toml"), None)?;
        let request: herdr_farm::domain::SnapshotRequest = serde_json::from_value(json!({
            "schema_version":1,"task_id":args.task,"profile":args.profile,"domains":[],"paths":[],"pinned_keys":[],"sensitivity":"default"}))?;
        let now = jiff::Timestamp::now().as_millisecond();
        let created = if let Some(reviewed) = &args.review_of {
            migration::open_active(&project)?.create_review_run_snapshot(herdr_farm::domain::SnapshotPlan {
                coordinator: false, session_id: None, request, profile_name: resolved.name.clone(), profile_digest: resolved.definition_digest.clone(),
                config_digest: Some(resolved.config_digest.clone()), budget_chars: resolved.budget.soft_input_chars,
                estimator: "char-count-worker-brief-v3".into(), instructions: instructions.clone(), now_unix_ms: now, expected_heads_digest: None,
            }, &project, reviewed, &args.review_kind, &args.review_scope, &ctx.root.display().to_string(), &run.slug)?
        } else {
            let mut memory = herdr_farm::memory::MemoryStore::from_sqlite(migration::open_active(&project)?, project.join(".state/objects"));
            memory.create_worker_snapshot(request, &resolved.name, &resolved.definition_digest, Some(&resolved.config_digest), resolved.budget.soft_input_chars, &instructions, now, None)?
        };
        let selected = created.entries.iter().map(|e| &e.record_id).collect::<Vec<_>>();
        let omitted = migration::open_active(&project)?.active_facts(now)?.into_iter()
            .filter(|f| f.record.scope_id == "project" && f.record.kind != herdr_farm::domain::MemoryKind::TaskLocal && !selected.contains(&&f.record.id))
            .map(|f| f.record.record_key).collect::<Vec<_>>();
        let mut value = serde_json::to_value(&created)?;
        value["omitted_for_budget"] = json!(omitted);
        Ok(value)
    })?;
    let selection = LaunchSelection {
        task: task_id.clone(),
        binding,
        profile: profile.clone(),
        knowledge: VersionedReference { id: knowledge["id"].as_str().context("snapshot id missing")?.to_owned(), revision: 1, digest: knowledge["manifest_hash"].as_str().context("snapshot digest missing")?.to_owned() },
        repositories: vec![repository.clone()],
        reason: None,
        note: None,
    };
    run.done("knowledge_snapshot", json!({"snapshot":selection.knowledge,"omitted_for_budget":knowledge["omitted_for_budget"]}));
    let deadline = Instant::now() + herdr_farm::profile_preparation::BUDGET;
    eprintln!("launch run: draft");
    let drafted = retry(|| launch_preparation::draft(&project, &selection, run.head()?, Duration::from_secs(args.validity_seconds), deadline, Default::default()))?;
    let approval = dir.join("approval.json");
    fs::write(&approval, serde_json::to_vec_pretty(&drafted.approval)?)?;
    run.done("draft", json!({"approval_document":approval,"brief_chars":drafted.brief.prompt_chars}));
    let key = args.sign_with.as_deref().with_context(|| format!("the launch approval needs the owner's signature: pass --sign-with KEY, or sign {} with `ssh-keygen -Y sign -n {} -f KEY`, then `approval import` and `launch reserve`", approval.display(), authority::SIGNATURE_NAMESPACE))?;
    eprintln!("launch run: approval_import");
    let imported = retry(|| {
        let signature = sign(key, authority::SIGNATURE_NAMESPACE, &approval)?;
        authority::import_signed(&project, &approval, &signature, run.head()?)
    })?;
    run.done("approval_import", json!({"approval":imported}));
    let approval_reference = VersionedReference { id: format!("approval-{}", imported.digest), revision: 1, digest: imported.digest.clone() };
    eprintln!("launch run: reserve");
    let reservation = retry(|| launch_preparation::reserve(&project, &selection, &approval_reference, run.head()?, deadline, Default::default()))?;
    anyhow::ensure!(reservation.record.inputs == drafted.inputs, "the reservation does not carry the drafted inputs");
    let attempt = reservation.record.attempt.clone();
    let worktree = herdr_farm::domain::worktree_plans(&drafted.inputs, &attempt).map_err(anyhow::Error::msg)?.into_iter().next().map(|p| p.path);
    run.done("reserve", json!({"attempt":attempt}));
    Ok(report(run, &args.task, &profile, &kind, &herdr, &socket, Some(attempt.as_str().to_owned()), worktree, worker_wall_seconds))
}

#[allow(clippy::too_many_arguments)]
fn report(run: &Run, task: &str, profile: &VersionedReference, kind: &str, herdr: &Path, socket: &Path, attempt: Option<String>, worktree: Option<String>, worker_wall_seconds: u64) -> Value {
    let contract = migration::open_active(&run.project).ok().and_then(|store| store.task_contract_document(task).ok().flatten()).unwrap_or(Value::Null);
    let reference = TaskId::new(task.to_owned()).ok().and_then(|id| runtime::task_contract(&run.project, &id).ok().flatten());
    if attempt.is_some() {
        eprintln!("launch run: attempt reserved; worker wall budget {worker_wall_seconds} seconds");
    }
    json!({
        "acceptance_policies":contract["acceptance_policies"].as_array().map(|policies| policies.iter().map(|policy| {
            let mut summary = json!({"id":policy["id"]});
            if policy["id"].as_str().is_some_and(|id| id.starts_with("accept"))
                && let Ok(text) = serde_json::from_str::<Value>(policy["text"].as_str().unwrap_or("")) {
                summary["toolchain"] = text["toolchain"].clone();
                summary["command"] = text["checks"].clone();
            }
            summary
        }).collect::<Vec<_>>()),
        "worker_wall_seconds":worker_wall_seconds,
        "contract_digest":reference.map(|r| r.digest),
        "write_paths":contract["scope"]["paths"].as_array().map(|paths| paths.iter().filter(|p| p["access"] == "write").map(|p| p["path"].clone()).collect::<Vec<_>>()),
        "outputs":contract["outputs"].as_array().map(|outputs| outputs.iter().map(|o| o["path"].clone()).collect::<Vec<_>>()),
        "project":run.slug,"task":task,"kind":kind,"profile":profile,"attempt":attempt,"worktree":worktree,
        "herdr_socket":socket,
        "viewer":run.steps.iter().find(|s| s.name == "viewer").map(|s| &s.detail),
        "steps":run.steps.iter().map(|s| json!({"step":s.name,"outcome":s.outcome,"detail":s.detail})).collect::<Vec<_>>(),
        "next":[
            format!("The ticker launches the worker using the verified profile Herdr {}.", herdr.display()),
            format!("Watch it with `scheduler {} inspect` and `operations {} inspect`. The worker's brief ends with the submission it must make; if it finishes without submitting, `result {} submit-captured <attempt>` captures its worktree and records the submission, and automatic verification and integration take it from there.", run.slug, run.slug, run.slug),
            format!("When the task is finished the ticker stops its dedicated Herdr server; `launch {} stop --task {task}` does it explicitly.", run.slug)
        ]
    })
}

/// Whether `pid` is still the dedicated server `launch run` started on `socket`:
/// a `herdr server` process of ours whose environment names that socket. A
/// recycled process id never matches.
fn is_our_server(pid: i32, socket: &Path) -> bool {
    let read = |name: &str| fs::read(format!("/proc/{pid}/{name}")).unwrap_or_default();
    let cmdline = read("cmdline");
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).filter(|a| !a.is_empty()).collect();
    let wanted = format!("HERDR_SOCKET_PATH={}", socket.display());
    args.last() == Some(&b"server".as_slice()) && read("environ").split(|b| *b == 0).any(|e| e == wanted.as_bytes())
}

fn signal(pid: i32, signal: i32) -> bool {
    // SAFETY: kill(2) on a process id this module just proved to be its server.
    unsafe { libc::kill(pid, signal) == 0 }
}

/// Stop the dedicated Herdr server of `task` and remove its socket directory.
/// Refuses while the task's attempt still holds its worker, unless `force`.
/// A task with no dedicated server (an operator-managed `--herdr-socket`) is
/// left alone. Idempotent.
pub fn stop(ctx: &Ctx, slug: &str, task: &str, force: bool) -> Result<Value> {
    let project = ctx.root.join(slug).canonicalize().with_context(|| format!("project {slug} not found"))?;
    let task_id = TaskId::new(task.to_owned()).map_err(anyhow::Error::msg)?;
    let directory = ctx.root.join(".herdr-run").join(format!("{slug}-{task}")).join("herdr");
    let record = directory.join(SERVER_RECORD);
    let Ok(text) = fs::read_to_string(&record) else {
        return Ok(json!({"task":task,"stopped":false,"reason":"no dedicated Herdr server is recorded for this task"}));
    };
    let record_value: Value = serde_json::from_str(&text).context("server record is unreadable")?;
    if let Some(project) = record_value["project"].as_str() {
        ensure!(project == slug && record_value["task"].as_str() == Some(task), "private server record belongs to another project/task");
    }
    // Historical directory names concatenate slug and task with a hyphen.
    // Protect every possible project/task interpretation before retiring one.
    if !force {
        let name = format!("{slug}-{task}");
        for other in crate::project::list_slugs(&ctx.root).into_iter().filter(|other| other != slug) {
            if let Some(other_task) = name.strip_prefix(&format!("{other}-")) {
                let other_snapshot = migration::open_active_read_only(&ctx.root.join(&other))?.read_snapshot(None)?;
                ensure!(!other_snapshot.attempts.iter().any(|a| a.task.as_str() == other_task && a.retains_capacity()), "private server may belong to an active attempt in project {other}");
            }
        }
    }
    // Fence launch preparation as well as reservation: its server is created
    // before its attempt exists, and must not be swept mid-launch.
    let launch_guard = fs::File::open(project.join(".state/state.db"))?;
    launch_guard.try_lock().context("launch preparation or memory adoption is in progress; retry server retirement")?;
    let _guard = herdr_farm::execution_guard::ProjectGuard::acquire(&project)?;
    let snapshot = migration::open_active_read_only(&project)?.read_snapshot(None)?;
    let server_socket = record_value["socket"].as_str();
    let held = snapshot.attempts.iter().find(|a| a.retains_capacity() && (a.task == task_id ||
        snapshot.ownership.iter().any(|o| o.attempt.as_ref() == Some(&a.id) &&
            snapshot.runtime_bindings.iter().any(|b| b.id == o.binding && Some(b.identity.socket.as_str()) == server_socket))));
    if let (Some(attempt), false) = (held, force) {
        bail!("attempt {} of task {task} still holds its worker; stop it first (or pass --force)", attempt.id.as_str());
    }
    close_viewer(ctx, &record_value);
    if record_value["managed_by"] == "operator" {
        fs::remove_file(&record)?;
        return Ok(json!({"task":task,"stopped":false,"reason":"operator-managed server left running"}));
    }
    let pid = i32::try_from(record_value["pid"].as_i64().context("server record has no pid")?).context("server pid is out of range")?;
    let socket = PathBuf::from(record_value["socket"].as_str().context("server record has no socket")?);
    let mut stopped = false;
    if pid > 1 && is_our_server(pid, &socket) {
        ensure!(signal(pid, libc::SIGTERM) || !is_our_server(pid, &socket), "could not signal private Herdr server; retain its recovery record");
        let until = Instant::now() + Duration::from_secs(5);
        while Path::new(&format!("/proc/{pid}")).exists() && is_our_server(pid, &socket) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        if is_our_server(pid, &socket) {
            ensure!(signal(pid, libc::SIGKILL) || !is_our_server(pid, &socket), "could not kill private Herdr server; retain its recovery record");
            let until = Instant::now() + Duration::from_secs(2);
            while is_our_server(pid, &socket) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            ensure!(!is_our_server(pid, &socket), "private Herdr server is still running; retain its recovery record");
        }
        stopped = true;
    }
    // The socket directory is the short private one named by the socket itself.
    if let Some(dir) = socket.parent().filter(|d| d.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('r')) && socket.file_name().is_some_and(|n| n == "s")) {
        let _ = fs::remove_file(&socket);
        let _ = fs::remove_dir(dir);
    }
    fs::remove_file(&record)?;
    Ok(json!({"task":task,"stopped":stopped,"socket":socket,"socket_directory_removed":!socket.exists()}))
}

/// Stop the dedicated server of every task of `slug` whose attempts are all
/// finished (worker termination observed). The ticker calls this each pass; it
/// does nothing unless a server record exists. One line per stopped server or
/// failure, for the ticker log.
pub fn sweep_servers(ctx: &Ctx, slug: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let prefix = format!("{slug}-");
    let Ok(entries) = fs::read_dir(ctx.root.join(".herdr-run")) else { return lines };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(task) = name.strip_prefix(&prefix) else { continue };
        if !entry.path().join("herdr").join(SERVER_RECORD).is_file() {
            continue;
        }
        let Ok(project) = ctx.root.join(slug).canonicalize() else { continue };
        let Ok(snapshot) = migration::open_active_read_only(&project).and_then(|mut db| db.read_snapshot(None).map_err(Into::into)) else { continue };
        // Attempt retention is authoritative even if the task pointer changed.
        // Orphaned and never-reserved private servers have no worker to retain.
        if snapshot.attempts.iter().any(|a| a.task.as_str() == task && a.retains_capacity()) {
            continue;
        }
        // A launch in flight has a server before it has an attempt: a queued
        // task or one whose launch delivery is still pending keeps its server.
        // The task's active-attempt pointer can be stale after an end, so
        // retention uses attempt state (above) and deliveries, never it.
        // Cancelled, finished and superseded tasks lose theirs as before.
        let launching = snapshot.tasks.iter().any(|t| t.id.as_str() == task && t.state == herdr_farm::domain::TaskState::Queued)
            || snapshot.operations.iter().any(|o| o.kind == "runtime.launch" && o.task.as_ref().is_some_and(|t| t.as_str() == task)
                && snapshot.deliveries.iter().any(|d| d.operation == o.id && matches!(d.state, herdr_farm::operations::DeliveryState::Pending | herdr_farm::operations::DeliveryState::Claimed | herdr_farm::operations::DeliveryState::Ambiguous)));
        if launching { continue; }
        match stop(ctx, slug, task, false) {
            Ok(report) if report["stopped"] == true => lines.push(format!("stopped the dedicated Herdr server of finished task {task}")),
            Ok(_) => {}
            Err(error) => lines.push(format!("dedicated Herdr server of task {task} not stopped: {error:#}")),
        }
    }
    lines
}

fn split_command(raw: &str) -> Result<Vec<String>> {
    herdr_farm::verification::toolchains::split_accept_command(raw)
}

/// Owner convenience command shares launch's signer discovery and spawn gate.
pub fn adopt_memory(ctx: &Ctx, project: &Path, dry_run: bool, explicit: Option<PathBuf>) -> Result<Value> {
    let marker = migration::read_format(project).context("memory adopt requires a canonical project")?;
    migration::open_active(project).context("memory adopt requires an active canonical store")?;
    if marker.memory == "sqlite-v1" {
        if !dry_run { authority::recover_memory_adopt(project)?; }
        return Ok(json!({"outcome":"already_adopted"}));
    }
    let plan = herdr_farm::memory::adopt_plan(project)?;
    if dry_run { return Ok(serde_json::to_value(plan)?); }
    ensure!(plan.blockers.is_empty(), "adopt refused: {}", serde_json::to_string(&plan)?);
    let key = memory_signer(ctx, project, explicit)?;
    let temp = ProbeDirectory(herdr_farm::short_socket::fresh()?);
    let document = temp.0.join("memory.json");
    let result = authority::adopt_memory(project, &plan, |bytes| {
        fs::write(&document, bytes)?;
        fs::read(sign(&key, authority::MEMORY_SIGNATURE_NAMESPACE, &document)?).map_err(Into::into)
    })?;
    Ok(json!({"journal":result,"outcomes":plan.outcomes}))
}

fn memory_signer(ctx: &Ctx, project: &Path, explicit: Option<PathBuf>) -> Result<PathBuf> {
    let config = migration::status(project)?.plan.config.context("pinned owner config missing")?;
    let path = Path::new(&config.path);
    let value: toml::Value = toml::from_str(&String::from_utf8(migration::read_plan_file(path)?)?)?;
    let selected = if let Some(key) = explicit { Some(key) }
        else if let Some(configured) = value.get("coordinator").and_then(|v|v.get("signing_key")) {
            let key = configured.as_str().context("coordinator.signing_key must be an absolute path")?;
            ensure!(Path::new(key).is_absolute(), "coordinator.signing_key must be an absolute path");
            Some(PathBuf::from(key))
        } else { None };
    resolve_signer(selected, &[path.parent().context("config parent missing")?.to_owned(), ctx.config_dir.clone()], value["authority"]["approval_public_key"].as_str().context("owner approval key missing")?)
}

pub fn record_memory(ctx: &Ctx, project: &Path, title: &str, provenance: &str, body_file: &Path) -> Result<Value> {
    ensure!(migration::read_format(project)?.memory == "sqlite-v1", "legacy-markdown memory: use `memory-review PROJECT record` to record owner decisions");
    let (title, body, provenance, name) = crate::memory_review::owner_decision_inputs(title, body_file, provenance)?;
    let key = memory_signer(ctx, project, None)?;
    let temp = ProbeDirectory(herdr_farm::short_socket::fresh()?);
    let document = temp.0.join("memory.json");
    authority::record_memory(project, &title, &format!("memory/{name}"), &body, &provenance, |bytes| {
        fs::write(&document, bytes)?;
        fs::read(sign(&key, authority::MEMORY_SIGNATURE_NAMESPACE, &document)?).map_err(Into::into)
    })
}
