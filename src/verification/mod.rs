//! Independent verifier. The parent execs `unshare`; only the child switches root.
//! ```compile_fail
//! use herdr_farm::verification::VerificationReceipt;
//! let _: VerificationReceipt = serde_json::from_str("{}").unwrap();
//! ```
mod checkout;
mod evidence;
pub use evidence::repetition_load as host_load;
mod manifest;
pub mod toolchains;
mod setup;
mod repetitions;
pub(crate) mod supervise;

#[cfg(test)] use crate::execution_guard::GatedSpawn;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    domain::ObjectFormat,
    runner::{Output, RealRunner, Runner},
    store::{
        SqliteStore,
        verification::{RunDraft, VerifyTarget},
    },
};

pub use setup::setup_main;

pub const ISOLATION: &str = "linux-unshare-user-pid-mount-v1";

/// Native verifier receipt. JSON can display it and cannot build one.
#[derive(Debug, Serialize)]
pub struct VerificationReceipt {
    run_id: String,
    result_id: String,
    commit_oid: String,
    tree_oid: String,
    object_format: ObjectFormat,
    policy_digest: String,
    isolation: &'static str,
    exit_status: i32,
    memory_fence: u64,
    #[serde(skip)]
    store_device: i64,
    #[serde(skip)]
    store_inode: i64,
    // Keeps the store inode pinned for the accept transaction.
    #[serde(skip)]
    source_file: fs::File,
}

impl VerificationReceipt {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn result_id(&self) -> &str {
        &self.result_id
    }
    pub fn commit_oid(&self) -> &str {
        &self.commit_oid
    }
    pub fn tree_oid(&self) -> &str {
        &self.tree_oid
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn isolation(&self) -> &'static str {
        self.isolation
    }
    pub fn exit_status(&self) -> i32 {
        self.exit_status
    }
    pub fn memory_fence(&self) -> u64 {
        self.memory_fence
    }
    pub(crate) fn store_device(&self) -> i64 {
        self.store_device
    }
    pub(crate) fn store_inode(&self) -> i64 {
        self.store_inode
    }
    pub(crate) fn digest(&self) -> String {
        sha256(&format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.run_id,
            self.result_id,
            self.commit_oid,
            self.tree_oid,
            self.object_format.as_str(),
            self.policy_digest,
            self.isolation,
            self.exit_status,
            self.memory_fence,
            self.store_device,
            self.store_inode
        ))
    }
    pub(crate) fn binding_ok(&self) -> bool {
        use std::os::unix::fs::MetadataExt;
        self.source_file.metadata().ok().is_some_and(|meta| {
            i64::try_from(meta.dev()).ok() == Some(self.store_device)
                && i64::try_from(meta.ino()).ok() == Some(self.store_inode)
        })
    }
}

pub(crate) fn run_identity(project: &str, key: &str, payload: &str) -> String {
    sha256(&format!("{project}\0{key}\0{payload}"))
}
pub(crate) fn result_identity(run_id: &str, tree: &str, policy_digest: &str) -> String {
    sha256(&format!("{run_id}\0{tree}\0{policy_digest}"))
}
fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    None,
    DirtyTree,
    DropPolicy,
    #[cfg(test)]
    StageChange,
    #[cfg(test)]
    Untracked,
    #[cfg(test)]
    BumpFence,
}

#[derive(Clone)]
pub struct VerifyRequest {
    pub submission_id: String,
    pub policy_id: String,
    pub policy_path: PathBuf,
    /// Use the installed signed policy rather than an operator file.
    pub contract_policy: bool,
    pub idempotency_key: String,
    pub timeout: Duration,
    pub work_dir: PathBuf,
    pub(crate) unshare_program: PathBuf,
    pub(crate) fault: Fault,
    pub(crate) cancellation: Option<crate::runner::Cancellation>,
}

impl VerifyRequest {
    pub fn new(
        submission_id: impl Into<String>,
        policy_id: impl Into<String>,
        policy_path: impl Into<PathBuf>,
        idempotency_key: impl Into<String>,
        timeout: Duration,
        work_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            submission_id: submission_id.into(),
            policy_id: policy_id.into(),
            policy_path: policy_path.into(),
            contract_policy: false,
            idempotency_key: idempotency_key.into(),
            timeout,
            work_dir: work_dir.into(),
            unshare_program: PathBuf::from("/usr/bin/unshare"),
            fault: Fault::None,
            cancellation: None,
        }
    }
}

#[derive(Serialize)]
pub struct VerifyOutcome {
    pub run_id: String,
    pub state: String,
    pub reason: Option<String>,
    pub receipt: Option<VerificationReceipt>,
    pub argv: Vec<String>,
    pub stdout: String,
    pub replayed: bool,
}

#[derive(serde::Deserialize)]
struct PolicyDocument {
    /// Hidden check inputs (replay suite, TM4.6): owner-held files the
    /// candidate never sees, bound read-only into the isolated root at the
    /// same path and pinned by digest. Absent in ordinary policies.
    #[serde(default)]
    hidden: Vec<HiddenInput>,
}

/// One hidden check input: an absolute path under the project's replay
/// check store (`<projects root>/.replay/<slug>/checks/`) and its sha256.
#[derive(serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct HiddenInput {
    pub(crate) path: String,
    pub(crate) sha256: String,
}

const MAX_HIDDEN: usize = 8;
const MAX_HIDDEN_BYTES: u64 = 65_536;

/// The policy's hidden inputs, bounded: absolute normal paths, hex digests, unique.
pub(crate) fn parse_hidden(bytes: &[u8]) -> Result<Vec<HiddenInput>> {
    let document: PolicyDocument = serde_json::from_slice(bytes).context("policy document is invalid")?;
    if document.hidden.len() > MAX_HIDDEN { bail!("policy hidden inputs exceed bounds"); }
    let mut seen = std::collections::BTreeSet::new();
    for input in &document.hidden {
        let path = Path::new(&input.path);
        if input.path.len() > 1024 || input.path.contains('\0') || !path.is_absolute()
            || path.components().any(|c| !matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)))
            || input.sha256.len() != 64 || !input.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !seen.insert(input.path.clone()) {
            bail!("policy hidden input is invalid");
        }
    }
    Ok(document.hidden)
}

/// sha256 of a regular, non-symlink hidden file of at most 64 KiB.
pub(crate) fn hidden_digest(path: &Path) -> Option<String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_HIDDEN_BYTES { return None; }
    let mut bytes = Vec::new();
    file.take(MAX_HIDDEN_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_HIDDEN_BYTES { return None; }
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Every hidden input lies in the project's own replay check store, is
/// canonical and still has its pinned digest.
fn hidden_inputs_ok(project_store: &Path, hidden: &[HiddenInput]) -> bool {
    let Some(project) = project_store.parent().and_then(Path::parent) else { return false };
    let (Some(root), Some(slug)) = (project.parent(), project.file_name()) else { return false };
    let Ok(store) = root.join(".replay").join(slug).join("checks").canonicalize() else { return false };
    hidden.iter().all(|input| {
        let path = Path::new(&input.path);
        path.canonicalize().is_ok_and(|c| c == path && c.starts_with(&store)) && hidden_digest(path).as_deref() == Some(input.sha256.as_str())
    })
}

pub(crate) fn parse_checks(bytes: &[u8]) -> Result<Vec<String>> {
    Ok(crate::domain::verification_policy::ExecutionPolicy::parse(bytes)?.checks)
}

pub(crate) fn program_allowed(program: &str, checkout: &Path) -> bool {
    let path = Path::new(program);
    if program == "/usr/bin/git" {
        return true;
    }
    let Ok(program) = path.canonicalize() else {
        return false;
    };
    let Ok(checkout) = checkout.canonicalize() else {
        return false;
    };
    program.starts_with(checkout)
}

/// Owner/operator CLI ingress. Work is isolated in a newly created directory;
/// existing caller files are never reused or removed by this entry point.
/// Runtime ownership covers the load and the record; the isolated check holds
/// only the shared root and the work directory's fence.
pub fn verify_project(project: &Path, request: &VerifyRequest) -> Result<VerifyOutcome> {
    use std::os::unix::fs::DirBuilderExt;
    if request.timeout < Duration::from_secs(1) || request.timeout > Duration::from_secs(3600) {
        bail!("verification timeout must be between 1 and 3600 seconds");
    }
    if !request.work_dir.is_absolute() { bail!("verification work directory must be absolute"); }
    let scratch = crate::execution_guard::Resource::new("scratch", request.work_dir.display().to_string())?;
    let guard = crate::migration::runtime_mutation(project)?;
    let mut store = crate::migration::open_active(project)?;
    fs::DirBuilder::new().mode(0o700).create(&request.work_dir)
        .context("verification work directory must be new and have an existing parent")?;
    struct Work(Option<PathBuf>);
    impl Drop for Work { fn drop(&mut self) { if let Some(path) = &self.0 { let _ = fs::remove_dir_all(path); } } }
    let mut work = Work(Some(request.work_dir.clone()));
    // The work directory is this call's own, so it is removed with or without ownership.
    let mut ownership = OperatorOwnership::new(guard, project, scratch, request.timeout);
    let outcome = verify_owned(&mut store, request, Some(&mut ownership));
    let cleanup = fs::remove_dir_all(&request.work_dir);
    if cleanup.is_ok() { work.0 = None; }
    let outcome = outcome?;
    cleanup.context("verification recorded but scratch cleanup failed")?;
    Ok(outcome)
}

enum OperatorSlot { Project(crate::migration::Maintenance), Check(crate::execution_guard::CheckGuard), Lost }
/// Operator runtime ownership, handed to the verifier or integrator around its isolated check.
pub(crate) struct OperatorOwnership { slot: OperatorSlot, project: PathBuf, scratch: crate::execution_guard::Resource, wait: Duration }
impl OperatorOwnership {
    /// `scratch` fences the check's work directory; `wait` bounds regaining ownership.
    pub(crate) fn new(guard: crate::migration::Maintenance, project: &Path, scratch: crate::execution_guard::Resource, wait: Duration) -> Self {
        Self { slot: OperatorSlot::Project(guard), project: project.to_path_buf(), scratch, wait }
    }
}
impl CheckOwnership for OperatorOwnership {
    fn release(&mut self) -> Result<()> {
        let OperatorSlot::Project(guard) = std::mem::replace(&mut self.slot, OperatorSlot::Lost) else { bail!("the check does not hold project ownership") };
        match guard.fence(&self.scratch) {
            Ok(fence) => { self.slot = OperatorSlot::Check(guard.narrow(fence)?); Ok(()) }
            Err(error) => { self.slot = OperatorSlot::Project(guard); Err(error) }
        }
    }
    fn reacquire(&mut self, _: &mut SqliteStore) -> Result<()> {
        let OperatorSlot::Check(check) = std::mem::replace(&mut self.slot, OperatorSlot::Lost) else { bail!("the check is not narrowed") };
        self.slot = OperatorSlot::Project(crate::migration::Maintenance::widen(check, &self.project, self.wait)?);
        Ok(())
    }
    fn held(&self) -> bool { matches!(self.slot, OperatorSlot::Project(_)) }
}

/// Legacy automatic timeout. Toolchain policies use their owner-declared
/// timeout; the automatic job lease covers 3600 seconds plus preparation.
pub const AUTO_TIMEOUT: Duration = Duration::from_secs(240);

/// One automatic verification job. The policy is the stored signed body, never
/// an operator file, and the key is the job's operation id.
pub struct StoredJob<'a> {
    pub submission_id: &'a str,
    pub policy_id: &'a str,
    pub policy_digest: &'a str,
    pub key: &'a str,
    /// Deterministic per job; any leftover from an earlier attempt is removed first.
    pub scratch: &'a Path,
    pub cancellation: crate::runner::Cancellation,
}

/// Project ownership for an automatic check. The job loads and records under
/// it, and gives it up only while the isolated check runs in its own scratch.
pub trait CheckOwnership {
    /// Narrow ownership to the check's scratch fence and the shared root.
    fn release(&mut self) -> Result<()>;
    /// Take project ownership again and recheck the job's own fences; a
    /// [`FenceChanged`] error means nothing may be recorded.
    fn reacquire(&mut self, store: &mut SqliteStore) -> Result<()>;
    /// Whether project ownership is held now.
    fn held(&self) -> bool;
}

/// The job's inputs changed while the check ran; no verdict was recorded.
#[derive(Debug)]
pub struct FenceChanged(pub String);
impl std::fmt::Display for FenceChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "verification inputs changed during the check: {}", self.0)
    }
}
impl std::error::Error for FenceChanged {}

/// Controller ingress. The caller holds project ownership and a live claim;
/// `ownership` is released only around the isolated check. The scratch
/// directory is removed on every return path it can reach under ownership.
pub fn verify_stored(store: &mut SqliteStore, job: &StoredJob<'_>, ownership: &mut dyn CheckOwnership) -> Result<VerifyOutcome> {
    use std::os::unix::fs::DirBuilderExt;
    clear_scratch(job.scratch)?;
    let target = store.load_verify_target(job.submission_id, job.policy_id)?;
    if sha256(&target.policy_body) != job.policy_digest {
        bail!("stored acceptance policy changed since the job was enqueued");
    }
    let policy = target.policy_body.clone();
    drop(target);
    let parent = job.scratch.parent().context("verification scratch has no parent")?;
    if let Err(error) = fs::DirBuilder::new().mode(0o700).create(parent) {
        if error.kind() != std::io::ErrorKind::AlreadyExists { return Err(error.into()); }
    }
    if !fs::symlink_metadata(parent)?.is_dir() { bail!("verification scratch parent is not a directory"); }
    fs::DirBuilder::new().mode(0o700).create(job.scratch).context("verification scratch directory")?;
    let outcome = (|| {
        let policy_path = job.scratch.join("policy.json");
        fs::write(&policy_path, policy.as_bytes())?;
        let mut request = VerifyRequest::new(job.submission_id, job.policy_id, policy_path, job.key, AUTO_TIMEOUT, job.scratch.join("work"));
        request.cancellation = Some(job.cancellation.clone());
        fs::DirBuilder::new().mode(0o700).create(&request.work_dir)?;
        verify_owned(store, &request, Some(&mut *ownership))
    })();
    // Without ownership, recovery (observation by key) removes the scratch.
    if !ownership.held() { return outcome; }
    let cleanup = clear_scratch(job.scratch);
    let outcome = outcome?;
    cleanup.context("verification recorded but scratch cleanup failed")?;
    Ok(outcome)
}

/// Remove a job's scratch directory, refusing to follow a symlink.
pub fn clear_scratch(scratch: &Path) -> Result<()> {
    match fs::symlink_metadata(scratch) {
        Ok(meta) if meta.is_dir() => Ok(fs::remove_dir_all(scratch)?),
        Ok(_) => bail!("verification scratch is not a directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The run recorded under `key`, if any: (run id, state). Only store rows count.
pub fn recorded_run(store: &mut SqliteStore, project_store: &str, key: &str) -> Result<Option<(String, String)>> {
    Ok(store.lookup_verification(project_store, key)?.map(|run| (run.run_id, run.state)))
}

/// Automatic runs never fall back to an unsandboxed check. Probe the same
/// namespaces the verifier uses before any claim is taken.
pub fn isolation_available() -> Result<()> {
    isolation_available_with_uid(None)
}

/// Probe the identity sealed for this submission, including the inner step.
pub fn isolation_available_for(store: &mut SqliteStore, submission: &str, policy: &str) -> Result<()> {
    let target = store.load_verify_target(submission, policy)?;
    let identity = store.attempt_worker_uid(&target.attempt_id)?;
    isolation_available_with_uid(identity)
}

/// Integration probes the attempt that produced the verified result.
pub fn isolation_available_for_result(store: &SqliteStore, result: &str) -> Result<()> {
    isolation_available_with_uid(store.integration_worker_uid(result)?)
}

fn isolation_available_with_uid(identity: Option<(u32, u32)>) -> Result<()> {
    crate::self_executable::real_path()?;
    let unshare = Path::new("/usr/bin/unshare");
    if !unshare_ready(unshare) {
        bail!("isolation unavailable: /usr/bin/unshare is missing or not root-owned");
    }
    let mut probe = crate::runner::Cmd::new(unshare.display().to_string(), Duration::from_secs(5));
    probe.args = ["--user", "--map-root-user", "--mount", "--propagation", "private", "--pid", "--fork", "--mount-proc", "--kill-child=KILL", "--", "/bin/true"]
        .into_iter().map(String::from).collect();
    if let Some((uid, gid)) = identity {
        probe.args.pop();
        probe.args.extend(["/usr/bin/unshare".into(), "--user".into(), format!("--map-user={uid}"), format!("--map-group={gid}"), "--".into(),
            "/bin/sh".into(), "-c".into(), format!("[ $(/usr/bin/id -u) = {uid} ] && [ $(/usr/bin/id -g) = {gid} ]")]);
    }
    probe.env_clear = true;
    let output = RealRunner.run(&probe).context("isolation unavailable")?;
    if !output.success() {
        bail!("isolation unavailable: unshare probe failed: {}", output.stderr.trim().chars().take(512).collect::<String>());
    }
    Ok(())
}

fn read_policy(path: &Path) -> Result<Vec<u8>> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).context("policy file is unreadable")?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 4_000 { bail!("policy file must be a regular file of at most 4000 bytes"); }
    let mut bytes = Vec::new();
    file.take(4_001).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 4_000 { bail!("policy document exceeds bounds"); }
    Ok(bytes)
}

pub fn verify(store: &mut SqliteStore, request: &VerifyRequest) -> Result<VerifyOutcome> {
    verify_owned(store, request, None)
}

fn verify_owned(store: &mut SqliteStore, request: &VerifyRequest, ownership: Option<&mut dyn CheckOwnership>) -> Result<VerifyOutcome> {
    let target = store.load_verify_target(&request.submission_id, &request.policy_id)?;
    let mut resolved_request = request.clone();
    if request.contract_policy {
        resolved_request.policy_path = request.work_dir.join("signed-policy.json");
    }
    let request = &resolved_request;
    let policy_bytes = if request.contract_policy { target.policy_body.as_bytes().to_vec() } else { read_policy(&request.policy_path)? };
    let payload_digest = sha256(&format!(
        "{}\0{}\0{}\0{}",
        target.submission_id,
        target.policy_id,
        sha256(&String::from_utf8_lossy(&policy_bytes)),
        target.candidate_oid
    ));
    let replay = |store: &mut SqliteStore, target: &VerifyTarget| -> Result<Option<VerifyOutcome>> {
        let Some(existing) = store.lookup_verification(&target.project_store, &request.idempotency_key)? else { return Ok(None) };
        if existing.payload_digest != payload_digest {
            bail!("verification idempotency conflict");
        }
        Ok(Some(VerifyOutcome {
            run_id: existing.run_id,
            state: existing.state,
            reason: existing.reason,
            receipt: None,
            argv: existing.argv,
            stdout: String::new(),
            replayed: true,
        }))
    };
    if let Some(replayed) = replay(store, &target)? {
        return Ok(replayed);
    }
    if policy_bytes != target.policy_body.as_bytes() {
        bail!("policy_digest_mismatch: supplied digest {:x} differs from signed contract digest {}; omit --policy-file to use the signed policy, or export exact bytes with result <slug> policy --submission {} --policy-id {} --out FILE",
            Sha256::digest(&policy_bytes), target.policy_digest, target.submission_id, target.policy_id);
    }
    crate::self_executable::real_path()?;
    let checks = parse_checks(&policy_bytes)?;
    let toolchain = match toolchains::for_policy(Path::new(&target.project_store).parent().and_then(Path::parent).context("project path")?, &policy_bytes) {
        Ok(resolved) => resolved,
        Err(_) => return persist(store, &target, request, payload_digest, Vec::new(), Vec::new(), None, Some("toolchain_identity_mismatch"), None, String::new(), None, None),
    };
    let timeout = toolchains::timeout(toolchain.as_ref(), request.timeout);
    let hidden = parse_hidden(&policy_bytes)?;
    if !hidden.is_empty() && !hidden_inputs_ok(Path::new(&target.project_store), &hidden) {
        return persist(store, &target, request, payload_digest, Vec::new(), Vec::new(), None, Some("hidden_check_unavailable"), None, String::new(), None, None);
    }
    let checkout = checkout::materialize(
        &request.work_dir,
        Path::new(&target.project_store),
        &target.objects,
        &target.candidate_oid,
        &target.object_format,
        Some((Path::new(&target.repository), &target.base_oid)),
    )?;
    if request.contract_policy { fs::write(&request.policy_path, &policy_bytes)?; }
    let diff_counts = checkout::diff_counts(&checkout.path, &target.base_oid, &target.candidate_oid);
    if let Some(project) = Path::new(&target.project_store).parent().and_then(Path::parent) {
        let counts = diff_counts;
        let _ = crate::telemetry::operations::launch::submission_diff(project, &target.submission_id, counts);
    }
    if let Some(scopes) = &target.write_scopes {
        let reason = match checkout::changed_paths(&checkout.path, &target.base_oid, &target.candidate_oid) {
            Ok(paths) if paths.iter().any(|path| !crate::domain::in_write_scope(scopes, path)) => Some("scope_violation"),
            Ok(_) => None,
            Err(_) => Some("scope_diff_unavailable"),
        };
        if let Some(reason) = reason {
            return persist(store, &target, request, payload_digest, Vec::new(), Vec::new(),
                Some(checkout.tree), Some(reason), None, String::new(), None, None);
        }
    }
    // Inspect the materialized candidate, never the worker's artifact claims.
    // Disallow symlink ancestors as well as symlink final entries.
    if target.required_outputs.iter().any(|output| {
        let mut path = checkout.path.clone();
        let parts: Vec<_> = output.split('/').collect();
        parts.iter().enumerate().any(|(index, part)| {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(meta) => meta.file_type().is_symlink()
                    || if index + 1 == parts.len() { !meta.is_file() } else { !meta.is_dir() },
                Err(_) => true,
            }
        })
    }) {
        return persist(store, &target, request, payload_digest, Vec::new(), Vec::new(),
            Some(checkout.tree), Some("required_output_missing"), None, String::new(), None, None);
    }
    if !crate::domain::verification_policy::ExecutionPolicy::parse(&policy_bytes)?
        .commands().all(|args| toolchains::command_allowed(&args[0], &checkout.path, toolchain.as_ref())) {
        return persist(
            store,
            &target,
            request,
            payload_digest,
            Vec::new(),
            Vec::new(),
            Some(checkout.tree),
            Some("checks_not_allowlisted"),
            None,
            String::new(),
            None,
            None,
        );
    }
    if request.fault == Fault::DirtyTree {
        fs::write(checkout.path.join("src/file.txt"), b"tampered\n")
            .context("could not dirty the checkout")?;
    }
    #[cfg(test)]
    if request.fault == Fault::StageChange {
        fs::write(checkout.path.join("src/file.txt"), b"staged\n")
            .context("could not stage the checkout")?;
        let status = std::process::Command::new("/usr/bin/git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "add",
                "--",
                "src/file.txt",
            ])
            .current_dir(&checkout.path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status_gated()
            .context("git add")?;
        if !status.success() {
            bail!("could not stage the checkout change");
        }
    }
    #[cfg(test)]
    if request.fault == Fault::Untracked {
        fs::write(checkout.path.join("untracked.txt"), b"extra\n")
            .context("could not add an untracked file")?;
    }
    if request.fault == Fault::DropPolicy {
        fs::remove_file(&request.policy_path).context("could not remove the policy file")?;
    }
    let libraries = match manifest::git_libraries(Path::new("/usr/bin/git")) {
        Ok(libraries) => libraries,
        Err(_) => {
            return persist(
                store,
                &target,
                request,
                payload_digest,
                Vec::new(),
                Vec::new(),
                Some(checkout.tree),
                Some("isolation_setup_failed"),
                None,
                String::new(),
                None,
                None,
            );
        }
    };
    let worker_uid = store.attempt_worker_uid(&target.attempt_id)?;
    let mut launch = supervise::launch(&supervise::Spec {
        worker_uid,
        unshare_program: request.unshare_program.clone(),
        timeout,
        checkout: checkout.path.clone(),
        policy: request.policy_path.clone(),
        scratch: request.work_dir.join("ns-root"),
        checks: checks.clone(),
        commit: checkout.commit.clone(),
        tree: checkout.tree.clone(),
        policy_digest: target.policy_digest.clone(),
        toolchain: toolchain.clone(),
        repository: Some(PathBuf::from(&target.repository)),
        hidden: hidden.iter().map(|input| PathBuf::from(&input.path)).collect(),
    })?;
    if !unshare_ready(&request.unshare_program) {
        return persist(
            store,
            &target,
            request,
            payload_digest,
            launch.argv,
            libraries,
            Some(checkout.tree),
            Some("isolation_setup_failed"),
            None,
            String::new(),
            None,
            None,
        );
    }
    launch.cmd.capture_limit = 2 * 1024 * 1024 + 1;
    launch.cmd.cancellation = request.cancellation.clone();
    let mut execution = evidence::Execution::start(Path::new(&target.project_store));
    launch.cmd.env.push(("HP_VERIFY_DEADLINE_MONOTONIC_MS".into(),
        (repetitions::monotonic_ms().saturating_add(timeout.as_millis() as u64)).to_string()));
    // The check touches only its own scratch; record only if the inputs still hold.
    let (ran, target) = match ownership {
        Some(ownership) => {
            ownership.release()?;
            let ran = RealRunner.run(&launch.cmd);
            execution.completed();
            ownership.reacquire(store)?;
            let fresh = store.load_verify_target(&request.submission_id, &request.policy_id)
                .map_err(|error| FenceChanged(format!("{error}")))?;
            if !fresh.same_job(&target) {
                return Err(FenceChanged("task, submission, contract, policy or attempt changed".into()).into());
            }
            // Another caller may have recorded under the same key meanwhile.
            if let Some(replayed) = replay(store, &fresh)? {
                return Ok(replayed);
            }
            (ran, fresh)
        }
        None => {
            let ran = RealRunner.run(&launch.cmd);
            execution.completed();
            (ran, target)
        },
    };
    crate::self_executable::real_path()?;
    let output = match ran {
        // A cancelled check is no verdict: record nothing and let the caller retry.
        Ok(output) if output.cancelled => bail!("verification cancelled before a verdict"),
        Ok(output) => output,
        Err(_) => {
            return persist(
                store,
                &target,
                request,
                payload_digest,
                launch.argv,
                libraries,
                Some(checkout.tree),
                Some("isolation_setup_failed"),
                None,
                String::new(),
                None,
                None,
            );
        }
    };
    let mut report = classify(&output, &checkout.commit, &checkout.tree);
    if toolchain.as_ref().is_some_and(|r| !toolchains::unchanged(r)) {
        report.success = false;
        report.reason = Some("toolchain_identity_mismatch");
    }
    #[cfg(test)]
    if request.fault == Fault::BumpFence {
        store.testing_append_event()?;
    }
    let receipt = if report.success {
        Some(issue_receipt(
            &target,
            &payload_digest,
            &request.idempotency_key,
            &checkout.commit,
            report.tree.as_deref().unwrap_or_default(),
        )?)
    } else {
        None
    };
    let mut metadata = execution.metadata(&report.stdout, output.stdout_truncated);
    if report.reason == Some("tampered_tree") {
        let changes = protocol(&output.stderr);
        metadata["tree_changes"] = serde_json::json!({"count": changes.change_count,
            "truncated": changes.change_count > changes.paths.len() || changes.path_truncated,
            "paths": changes.paths});
    }
    metadata["diff_size"] = match diff_counts { Some((a,d,b)) => serde_json::json!({"added":a,"removed":d,"binary_files":b}), None => serde_json::json!({"status":"unavailable","reason":"diff_unavailable"}) };
    if let Some(resolved) = &toolchain { metadata["toolchain"] = serde_json::json!({"name":resolved.name,"digest":resolved.digest,"paths":resolved.identities,"network":resolved.toolchain.network,"timeout_seconds":resolved.toolchain.timeout_seconds}); }
    if crate::domain::verification_policy::ExecutionPolicy::parse(&policy_bytes)?.version == 2 {
        metadata["version"] = serde_json::json!("verification-metadata.v2");
        let observations = evidence::repetitions(&output.stderr, output.timed_out || output.code == Some(78));
        // The primary command's tests remain distinct from repetition output.
        if let Some(first) = observations.first() { metadata["tests"] = first["tests"].clone(); }
        metadata["observations"] = serde_json::json!(observations);
    }
    persist(
        store,
        &target,
        request,
        payload_digest,
        launch.argv,
        libraries,
        report.tree.or(Some(checkout.tree)),
        report.reason,
        receipt,
        report.stdout,
        report.exit_status,
        Some(metadata),
    )
}

#[allow(clippy::too_many_arguments)]
fn persist(
    store: &mut SqliteStore,
    target: &VerifyTarget,
    request: &VerifyRequest,
    payload_digest: String,
    argv: Vec<String>,
    libraries: Vec<PathBuf>,
    tree_oid: Option<String>,
    reason: Option<&str>,
    receipt: Option<VerificationReceipt>,
    stdout: String,
    exit_status: Option<i32>,
    metadata: Option<serde_json::Value>,
) -> Result<VerifyOutcome> {
    let libraries = libraries
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let (stored, receipt) = store.commit_verification_metadata(
        target,
        RunDraft {
            idempotency_key: request.idempotency_key.clone(),
            payload_digest,
            argv,
            libraries,
            tree_oid,
            exit_status,
            reason: reason.map(str::to_string),
            receipt,
        },
        metadata.as_ref(),
    )?;
    Ok(VerifyOutcome {
        run_id: stored.run_id,
        state: stored.state,
        reason: stored.reason,
        receipt,
        argv: stored.argv,
        stdout,
        replayed: false,
    })
}

#[cfg(test)]
pub(crate) fn testing_receipt(target: &VerifyTarget, payload: &str, key: &str, commit: &str, tree: &str) -> VerificationReceipt {
    issue_receipt(target, payload, key, commit, tree).unwrap()
}

fn issue_receipt(
    target: &VerifyTarget,
    payload_digest: &str,
    key: &str,
    commit: &str,
    tree: &str,
) -> Result<VerificationReceipt> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let path = Path::new(&target.project_store);
    let source_file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(path)
        .context("project store could not be pinned")?;
    let meta = source_file
        .metadata()
        .context("project store could not be pinned")?;
    let format = match target.object_format.as_str() {
        "sha1" => ObjectFormat::Sha1,
        "sha256" => ObjectFormat::Sha256,
        _ => bail!("unsupported object format"),
    };
    let run_id = run_identity(&target.project_store, key, payload_digest);
    let result_id = result_identity(&run_id, tree, &target.policy_digest);
    Ok(VerificationReceipt {
        run_id,
        result_id,
        commit_oid: commit.to_string(),
        tree_oid: tree.to_string(),
        object_format: format,
        policy_digest: target.policy_digest.clone(),
        isolation: ISOLATION,
        exit_status: 0,
        memory_fence: target.memory_fence,
        store_device: i64::try_from(meta.dev()).context("store device does not fit")?,
        store_inode: i64::try_from(meta.ino()).context("store inode does not fit")?,
        source_file,
    })
}

struct ChildReport {
    success: bool,
    reason: Option<&'static str>,
    tree: Option<String>,
    stdout: String,
    exit_status: Option<i32>,
}

fn classify(output: &Output, commit: &str, tree: &str) -> ChildReport {
    let stdout = output.stdout.clone();
    if output.timed_out || output.code == Some(78) {
        return ChildReport {
            success: false,
            reason: Some("timeout"),
            tree: None,
            stdout,
            exit_status: None,
        };
    }
    let protocol = protocol(&output.stderr);
    let seen_tree = protocol.tree.clone();
    let exit_status=protocol.checks.filter(|code|(0..=255).contains(code));
    match output.code {
        Some(73) => ChildReport {
            success: false,
            reason: Some("tampered_tree"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
        Some(74) => ChildReport {
            success: false,
            reason: Some("leftover_child"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
        Some(75) => ChildReport {
            success: false,
            reason: Some("policy_digest_mismatch"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
        Some(76) => ChildReport {
            success: false,
            reason: Some("checks_not_allowlisted"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
        Some(0)
            if protocol.commit.as_deref() == Some(commit)
                && protocol.tree.as_deref() == Some(tree)
                && protocol.checks == Some(0) =>
        {
            ChildReport {
                success: true,
                reason: None,
                tree: seen_tree,
                stdout,
                exit_status,
            }
        }
        Some(0) => ChildReport {
            success: false,
            reason: Some("tampered_tree"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
        Some(code) if ((1..71).contains(&code) || code==77) && protocol.commit.as_deref() == Some(commit) => {
            ChildReport {
                success: false,
                reason: Some("checks_failed"),
                tree: seen_tree,
                stdout,
                exit_status,
            }
        }
        _ => ChildReport {
            success: false,
            reason: Some("isolation_setup_failed"),
            tree: seen_tree,
            stdout,
            exit_status,
        },
    }
}

struct Protocol {
    change_count: usize,
    paths: Vec<serde_json::Value>,
    path_truncated: bool,
    commit: Option<String>,
    tree: Option<String>,
    checks: Option<i32>,
}

fn protocol(stderr: &str) -> Protocol {
    let mut parsed = Protocol {
        change_count: 0, paths: Vec::new(), path_truncated: false,
        commit: None,
        tree: None,
        checks: None,
    };
    for line in stderr.lines() {
        if line == "hp-verify changed-truncated=true" {
            parsed.path_truncated = true;
        } else if let Some(value) = line.strip_prefix("hp-verify changed-count=") {
            parsed.change_count = value.parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("hp-verify changed=") {
            if let Some((kind, path)) = value.split_once('\t')
                && matches!(kind, "modified" | "deleted" | "added-untracked")
                && parsed.paths.len() < 50 && path.len() <= 300 {
                let path = path.replace("%0A", "\n").replace("%0D", "\r").replace("%09", "\t").replace("%25", "%");
                parsed.paths.push(serde_json::json!({"kind":kind,"path":path}));
            }
        } else if let Some(value) = line.strip_prefix("hp-verify commit=") {
            parsed.commit = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("hp-verify tree=") {
            parsed.tree = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("hp-verify checks=") {
            parsed.checks = value.trim().parse().ok();
        }
    }
    parsed
}

pub(crate) fn isolated_check_ok(output: &Output, commit: &str, tree: &str) -> bool {
    classify(output, commit, tree).success
}

pub(crate) fn unshare_ready(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(root) = fs::metadata("/") else {
        return false;
    };
    meta.is_file()
        && meta.uid() == root.uid()
        && meta.mode() & 0o022 == 0
        && meta.mode() & 0o111 != 0
}


/// Toolchain checks receive only the owner-declared environment. Legacy checks
/// retain the verifier environment, as before toolchain policies.
pub(super) fn check_command(program: &str, identity: Option<(u32, u32, i32)>) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(program);
    if let Some((uid, gid, proc_fd)) = identity {
        // Equivalent to unshare --user --map-user=uid --map-group=gid. The
        // parent mapped the host owner to 0. Use its private writable proc
        // mount for these writes, without making the check's /proc writable.
        let uid_map = format!("{uid} 0 1\n");
        let gid_map = format!("{gid} 0 1\n");
        // SAFETY: the gated spawn runs this in its child before exec. All
        // strings are allocated before fork; the hook uses only syscalls.
        // proc_fd remains live in the parent, closes here before check exec,
        // and is CLOEXEC as an additional fence. Parent dumpability is off.
        unsafe { command.pre_exec(move || {
            if libc::unshare(libc::CLONE_NEWUSER) != 0 { return Err(std::io::Error::last_os_error()); }
            // Mapping proc inodes must belong to this child, not global root.
            if libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0) != 0 { return Err(std::io::Error::last_os_error()); }
            for (path, bytes) in [(c"self/setgroups", b"deny".as_slice()),
                (c"self/uid_map", uid_map.as_bytes()), (c"self/gid_map", gid_map.as_bytes())] {
                let fd = libc::openat(proc_fd, path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                if fd < 0 { return Err(std::io::Error::last_os_error()); }
                let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
                let error = std::io::Error::last_os_error();
                libc::close(fd);
                if written != bytes.len() as isize { return Err(error); }
            }
            libc::close(proc_fd);
            Ok(())
        }); }
    }
    let Ok(raw) = std::env::var("HP_VERIFY_CHECK_ENV") else { return command; };
    command.env_clear().env("PATH", "/usr/bin:/bin").env("HOME", "/tmp").env("TMPDIR", "/tmp")
        .env("LANG", "C").env("LC_ALL", "C").env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1").env("GIT_NO_REPLACE_OBJECTS", "1").env("GIT_OPTIONAL_LOCKS", "0");
    if let Ok(env) = serde_json::from_str::<Vec<String>>(&raw) {
        for entry in env { if let Some((name, value)) = entry.split_once('=') { command.env(name, value); } }
    }
    command
}

/// Materialize a submission for an operator's host inspection. Not acceptance.
pub fn checkout_project(project: &Path, submission: Option<&str>, attempt: Option<&str>, into: &Path) -> Result<serde_json::Value> {
    anyhow::ensure!(crate::submission_spool::worker_spool().is_none(), "result checkout refuses a worker execution context");
    crate::telemetry::review::refuse_owner_cli_in_worker_context(project, "result checkout")?;
    anyhow::ensure!(submission.is_some() != attempt.is_some(), "select either submission or attempt");
    let mut db = crate::migration::open_active(project)?;
    let mut views = db.show_results(submission)?;
    if let Some(attempt) = attempt { views.retain(|v| v.attempt_id == attempt); }
    anyhow::ensure!(views.len() == 1, "checkout requires exactly one submission; select --submission explicitly");
    let view = views.remove(0);
    let objects = db.checkout_objects(&view.submission_id)?;
    let parent = into.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).canonicalize()?;
    let destination = parent.join(into.file_name().context("checkout directory name missing")?);
    anyhow::ensure!(!destination.starts_with(project.canonicalize()?) && !destination.starts_with(Path::new(&view.repository).canonicalize()?), "checkout must be outside the project and source repository");
    if let Ok(meta) = fs::symlink_metadata(&destination) {
        anyhow::ensure!(meta.is_dir() && !meta.file_type().is_symlink() && fs::read_dir(&destination)?.next().is_none(), "checkout destination must be an empty directory");
    } else { fs::create_dir(&destination)?; }
    let scratch = checkout::Scratch::new(&parent)?;
    let checkout = checkout::materialize_project(&scratch.0, &project.join(".state/state.db"), &objects,
        &view.candidate_oid, &view.object_format, Some((Path::new(&view.repository), &view.base_oid)))?;
    // Rename into the reserved empty directory; never remove caller contents.
    fs::rename(&checkout.path, &destination)?;
    Ok(serde_json::json!({"candidate_oid":view.candidate_oid,"base_oid":view.base_oid,
        "attempt":view.attempt_id,"task":view.task_id,"path":destination,"outputs":view.artifact_manifest}))
}

/// Bounded report observation; declared review document paths are available in
/// result show's artifact_manifest and checkout's outputs.
pub fn attempt_report(project: &Path, attempt: &str) -> Result<String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    crate::domain::AttemptId::new(attempt.to_owned()).map_err(anyhow::Error::msg)?;
    let mut db = crate::migration::open_active(project)?;
    anyhow::ensure!(db.read_snapshot(None)?.attempts.iter().any(|a| a.id.as_str() == attempt), "attempt missing");
    let relative = format!(".state/worker-output/{attempt}/report.md");
    let path = crate::migration::safe_join(project, &relative)?;
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    anyhow::ensure!(file.metadata()?.is_file(), "report is not a regular file");
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "report exceeds 1 MiB");
    Ok(String::from_utf8(bytes)?)
}

#[cfg(test)]
mod tests;

/// Export the installed signed contract text verbatim, without a newline.
pub fn export_policy(project: &Path, submission: &str, policy: &str, out: &Path) -> Result<()> {
    let mut store = crate::migration::open_active_unchecked(project)?;
    export_policy_from_store(&mut store, submission, policy, out)
}

/// Public store ingress for exact signed policy export.
pub fn export_policy_from_store(store: &mut SqliteStore, submission: &str, policy: &str, out: &Path) -> Result<()> {
    use std::io::Write;
    let target = store.load_verify_target(submission, policy)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(out)?;
    file.write_all(target.policy_body.as_bytes())?;
    Ok(())
}
