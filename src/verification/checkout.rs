//! Fresh checkout from the signed owner base and retained untrusted objects.
//! Hooks are disabled; the only fetch is from the local owner repository.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::{
    runner::{Cmd, RealRunner, Runner},
    store::verification::RetainedObject,
};

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Delete only a canonical checkout that does not contain the open database.
fn reset_checkout(checkout: &Path, store_file: &Path, store_dir: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(checkout) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("checkout path"),
    };
    if meta.file_type().is_symlink() {
        bail!("verification checkout must stay outside the project store");
    }
    let canonical = checkout.canonicalize().context("checkout path")?;
    if overlaps(&canonical, store_file) || overlaps(&canonical, store_dir) {
        bail!("verification checkout must stay outside the project store");
    }
    fs::remove_dir_all(&canonical).context("reset checkout")?;
    Ok(())
}

pub struct Checkout {
    pub path: PathBuf,
    pub commit: String,
    pub tree: String,
}

pub fn materialize(
    work: &Path,
    store_file: &Path,
    objects: &[RetainedObject],
    oid: &str,
    object_format: &str,
    trusted_base: Option<(&Path, &str)>,
) -> Result<Checkout> {
    materialize_inner(work, store_file, objects, oid, object_format, trusted_base, false)
}

/// Materialize an operator checkout with hardening isolated from verification.
pub(super) fn materialize_project(
    work: &Path,
    store_file: &Path,
    objects: &[RetainedObject],
    oid: &str,
    object_format: &str,
    trusted_base: Option<(&Path, &str)>,
) -> Result<Checkout> {
    let checkout = materialize_inner(work, store_file, objects, oid, object_format, trusted_base, true)?;
    fs::create_dir_all(checkout.path.join(".git/hooks"))?;
    checkout_git(&checkout.path, &["config".into(), "core.hooksPath".into(), ".git/hooks".into()], Some(&checkout.path))?;
    checkout_git(&checkout.path, &["config".into(), "submodule.recurse".into(), "false".into()], Some(&checkout.path))?;
    Ok(checkout)
}

fn materialize_inner(
    work: &Path,
    store_file: &Path,
    objects: &[RetainedObject],
    oid: &str,
    object_format: &str,
    trusted_base: Option<(&Path, &str)>,
    operator_checkout: bool,
) -> Result<Checkout> {
    let git = |cwd: &Path, args: &[String], dir: Option<&Path>| {
        if operator_checkout { checkout_git(cwd, args, dir) } else { git(cwd, args, dir) }
    };
    if !matches!(object_format, "sha1" | "sha256") {
        bail!("unsupported Git object format");
    }
    let work = work.canonicalize().context("verification work directory")?;
    let store_file = store_file.canonicalize().context("project store")?;
    let store_dir = store_file
        .parent()
        .context("project store directory")?
        .canonicalize()
        .context("project store directory")?;
    // Either nesting can make a later delete or a bind include the open database.
    if overlaps(&work, &store_file) || overlaps(&work, &store_dir) {
        bail!("verification checkout must stay outside the project store");
    }
    let path = work.join("checkout");
    if overlaps(&path, &store_file) || overlaps(&path, &store_dir) {
        bail!("verification checkout must stay outside the project store");
    }
    reset_checkout(&path, &store_file, &store_dir)?;
    let template = work.join("template");
    fs::create_dir_all(&template).context("git template")?;
    git(
        &work,
        &[
            "init".into(),
            format!("--object-format={object_format}"),
            "--template".into(),
            template.display().to_string(),
            path.display().to_string(),
        ],
        None,
    )?;
    if let Some((repository, base)) = trusted_base {
        // Only the signed contract's base is imported. No alternate remains,
        // and no candidate is fetched from an attempt or quarantine.
        git(&path, &[
            "-c".into(), "protocol.file.allow=always".into(),
            "-c".into(), "fetch.fsckObjects=true".into(),
            "-c".into(), "core.hooksPath=/dev/null".into(),
            "fetch".into(), "--no-tags".into(), "--no-write-fetch-head".into(),
            "--".into(), repository.display().to_string(), base.into(),
        ], Some(&path)).context("owner repository base is unavailable or invalid")?;
    }
    for object in objects {
        if object
            .relative_path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            bail!("retained object path escapes the checkout");
        }
        let dest = path.join(".git/objects").join(&object.relative_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).context("git object directory")?;
        }
        fs::write(&dest, &object.bytes).with_context(|| format!("write {}", object.oid))?;
    }
    // fsck checks both object identity and complete connectivity before any
    // candidate files are exposed to acceptance checks.
    git(&path, &["fsck".into(), "--full".into(), "--strict".into(),
        "--no-reflogs".into(), "--no-dangling".into(), oid.into()], Some(&path))
        .context("candidate object hash mismatch or missing referenced object")?;
    git(
        &path,
        &[
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            "checkout".into(),
            "--detach".into(),
            oid.into(),
        ],
        Some(&path),
    )?;
    let commit = git_text(&path, &["rev-parse".into(), "HEAD".into()])?;
    let tree = git_text(&path, &["rev-parse".into(), format!("{oid}^{{tree}}")])?;
    if commit != oid {
        bail!("checkout did not land on the retained commit");
    }
    let _ = fs::remove_dir_all(path.join(".git/hooks"));
    Ok(Checkout { path, commit, tree })
}

fn git(cwd: &Path, args: &[String], dir: Option<&Path>) -> Result<()> {
    let output = run(cwd, args, dir)?;
    if !output.success() {
        bail!("git checkout failed: {}", output.stderr.trim());
    }
    Ok(())
}

fn git_text(cwd: &Path, args: &[String]) -> Result<String> {
    let output = run(cwd, args, Some(cwd))?;
    if !output.success() {
        bail!("git rev-parse failed: {}", output.stderr.trim());
    }
    let text = output.stdout.trim();
    if text.is_empty() || text.contains('\n') || !text.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("git rev-parse returned an unexpected oid");
    }
    Ok(text.to_string())
}

fn run(cwd: &Path, args: &[String], dir: Option<&Path>) -> Result<crate::runner::Output> {
    let mut command = Cmd::new("/usr/bin/git", Duration::from_secs(30));
    command.args = args.to_vec();
    command.cwd = Some(dir.unwrap_or(cwd).to_path_buf());
    command.env_clear = true;
    command.env = git_env();
    RealRunner.run(&command).context("git failed")
}

fn git_env() -> Vec<(String, String)> {
    vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOME".into(), "/".into()),
        ("LANG".into(), "C".into()),
        ("LC_ALL".into(), "C".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ("GIT_AUTHOR_NAME".into(), "verifier".into()),
        ("GIT_AUTHOR_EMAIL".into(), "verifier@example.com".into()),
        ("GIT_COMMITTER_NAME".into(), "verifier".into()),
        ("GIT_COMMITTER_EMAIL".into(), "verifier@example.com".into()),
    ]
}

/// Compare complete trees without rename folding so both old and new names of
/// a moved file must be authorized. NUL framing preserves arbitrary Git paths.
pub(super) fn changed_paths(checkout: &Path, base: &str, candidate: &str) -> Result<Vec<Vec<u8>>> {
    let output = run(checkout, &[
        "diff-tree".into(), "--no-commit-id".into(), "--name-only".into(),
        "--no-renames".into(), "--no-ext-diff".into(), "--no-textconv".into(),
        "-r".into(), "-z".into(), base.into(), candidate.into(), "--".into(),
    ], Some(checkout))?;
    if !output.success() { bail!("candidate scope diff is unavailable or exceeds capture bounds"); }
    let bytes = output.stdout_bytes;
    if !bytes.is_empty() && bytes.last() != Some(&0) { bail!("candidate scope diff has invalid framing"); }
    let paths: Vec<_> = bytes.split(|byte| *byte == 0).filter(|path| !path.is_empty()).map(Vec::from).collect();
    if paths.len() > 10_000 { bail!("candidate scope diff exceeds 10000 files"); }
    Ok(paths)
}

/// Aggregate the whole submitted range in the isolated verification checkout.
/// RealRunner bounds capture and routes spawning through GatedSpawn.
pub(super) fn diff_counts(checkout: &Path, base: &str, candidate: &str) -> Option<(i64,i64,i64)> {
    let output = run(checkout, &["diff".into(), "--numstat".into(), "-z".into(),
        "--no-renames".into(), "--no-ext-diff".into(), "--no-textconv".into(),
        "--end-of-options".into(), base.into(), candidate.into(), "--".into()], Some(checkout)).ok()?;
    if !output.success() || output.stdout_truncated || (!output.stdout_bytes.is_empty() && output.stdout_bytes.last() != Some(&0)) { return None; }
    let mut counts = (0i64,0i64,0i64);
    for row in output.stdout_bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let mut fields = row.splitn(3, |b| *b == b'\t');
        let a = fields.next()?; let d = fields.next()?; fields.next()?;
        if a == b"-" && d == b"-" { counts.2 = counts.2.checked_add(1)?; }
        else {
            let a = std::str::from_utf8(a).ok()?.parse::<i64>().ok().filter(|n| *n >= 0)?;
            let d = std::str::from_utf8(d).ok()?.parse::<i64>().ok().filter(|n| *n >= 0)?;
            counts.0 = counts.0.checked_add(a)?; counts.1 = counts.1.checked_add(d)?;
        }
    }
    Some(counts)
}

/// Only operator checkouts use private hooks and disable attributes/submodules.
fn checkout_git(cwd: &Path, args: &[String], dir: Option<&Path>) -> Result<()> {
    let hooks = Scratch::new(&std::env::temp_dir())?;
    let mut hardened = vec!["-c".into(), format!("core.hooksPath={}", hooks.0.display()),
        "-c".into(), "submodule.recurse=false".into(), "-c".into(), "core.attributesFile=/dev/null".into()];
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        // Replace verification's per-command hook setting only for this path.
        if arg == "-c" && args.peek().is_some_and(|next| next.as_str() == "core.hooksPath=/dev/null") {
            args.next();
            continue;
        }
        hardened.push(arg.clone());
        if arg == "fetch" { hardened.push("--no-recurse-submodules".into()); }
    }
    git(cwd, &hardened, dir)
}

/// Private scratch directories created exclusively; cleanup only our own tree.
pub(super) struct Scratch(pub PathBuf);
impl Scratch {
    pub(super) fn new(parent: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        for ordinal in 0..100 {
            let name = format!(".result-checkout-{}-{}-{ordinal}", std::process::id(), jiff::Timestamp::now().as_nanosecond());
            let path = parent.join(name);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        bail!("cannot reserve checkout scratch directory")
    }
}
impl Drop for Scratch { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
