//! Owner-only mounts and signed identities for executable acceptance policies.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub network: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    600
}
#[derive(Default, Deserialize)]
struct Verification {
    #[serde(default)]
    toolchains: BTreeMap<String, Toolchain>,
    #[serde(default)]
    defaults: BTreeMap<String, Defaults>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    #[serde(default)]
    accept: Vec<String>,
}
#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    verification: Verification,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub mode: u32,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub sha256: Option<String>,
    pub symlink: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resolved {
    pub name: String,
    pub toolchain: Toolchain,
    pub mounts: Vec<PathBuf>,
    pub identities: Vec<Identity>,
    pub digest: String,
}
fn normal(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
fn collect(path: &Path, seen: &mut BTreeSet<PathBuf>) -> Result<()> {
    ensure!(
        normal(path) && path != Path::new("/"),
        "invalid toolchain path"
    );
    if !seen.insert(path.to_path_buf()) {
        return Ok(());
    }
    ensure!(seen.len() <= 100_000, "toolchain exceeds path limit");
    let meta = fs::symlink_metadata(path).context("toolchain path unavailable")?;
    if meta.file_type().is_symlink() {
        let canonical = fs::canonicalize(path)?;
        collect(&canonical, seen)?;
        if fs::metadata(path)?.is_dir() {
            for entry in fs::read_dir(path)? {
                collect(&path.join(entry?.file_name()), seen)?;
            }
        }
    } else if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(&entry?.path(), seen)?;
        }
    } else if meta.is_file() {
        for dependency in crate::worker_supervision::executable_dependencies(path) {
            collect(&dependency, seen)?;
        }
    } else {
        bail!("toolchain paths must be regular files or directories");
    }
    Ok(())
}
fn identity(path: &Path) -> Result<Identity> {
    let meta = fs::symlink_metadata(path)?;
    let sha256 = if meta.is_file() {
        let mut file = fs::File::open(path)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
        Some(format!("{:x}", digest.finalize()))
    } else {
        None
    };
    Ok(Identity {
        path: path.to_path_buf(),
        device: meta.dev(),
        inode: meta.ino(),
        size: meta.len(),
        mode: meta.mode(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        sha256,
        symlink: if meta.file_type().is_symlink() {
            Some(fs::read_link(path)?)
        } else {
            None
        },
    })
}
/// Resolves only the external config path pinned by the project's migration.
fn owner_config(project: &Path) -> Result<Option<Config>> {
    let Some(reference) = crate::migration::status(project)?.plan.config else {
        return Ok(None);
    };
    let path = Path::new(&reference.path);
    let canonical = path.canonicalize()?;
    let meta = fs::metadata(&canonical)?;
    ensure!(
        !canonical.starts_with(project.canonicalize()?)
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o022 == 0,
        "toolchains require external owner configuration"
    );
    let bytes = crate::migration::read_plan_file(path)?;
    Ok(Some(toml::from_str(std::str::from_utf8(&bytes)?)?))
}

/// Owner defaults use the same canonical path string as the safety table.
pub fn default_accept(project: &Path) -> Result<Vec<String>> {
    let Some(mut config) = owner_config(project)? else { return Ok(Vec::new()); };
    let key = project.canonicalize()?.to_string_lossy().into_owned();
    let defaults = config.verification.defaults.remove(&key).unwrap_or_default();
    ensure!(defaults.accept.len() <= 4, "verification.defaults.{key:?}.accept allows at most 4 entries");
    Ok(defaults.accept)
}

pub fn resolve(project: &Path, name: &str) -> Result<Resolved> {
    let config = owner_config(project)?.context("project has no owner configuration")?;
    let toolchain = config
        .verification
        .toolchains
        .get(name)
        .context("undeclared verification toolchain")?
        .clone();
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "invalid toolchain name"
    );
    ensure!(
        !toolchain.paths.is_empty()
            && toolchain.paths.len() <= 32
            && (1..=3600).contains(&toolchain.timeout_seconds),
        "toolchain exceeds bounds"
    );
    crate::worker_supervision::validate_thread_env(&toolchain.env)?;
    let mut paths = BTreeSet::new();
    for path in &toolchain.paths {
        collect(path, &mut paths)?;
    }
    for path in &paths {
        if !fs::metadata(path)?.is_file() {
            continue;
        }
        let mut head = Vec::new();
        fs::File::open(path)?.take(256).read_to_end(&mut head)?;
        if head.starts_with(b"#!") {
            let text = String::from_utf8_lossy(&head[2..]);
            let interpreter = Path::new(
                text.split_whitespace()
                    .next()
                    .context("empty toolchain interpreter")?,
            );
            ensure!(
                toolchain
                    .paths
                    .iter()
                    .any(|p| p == interpreter || p.is_dir() && interpreter.starts_with(p)),
                "toolchain script interpreter must be explicitly declared"
            );
        }
    }
    let identities = paths
        .iter()
        .map(|p| identity(p))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        serde_json::to_vec(&identities)?.len() <= 524_288,
        "toolchain identities exceed evidence limit"
    );
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&toolchain, &identities))?)
    );
    // Mount individual files rather than directories: no nested writable mounts,
    // host directory additions, or accidental exposure of unrelated files.
    let mounts = identities
        .iter()
        .filter(|i| i.sha256.is_some())
        .map(|i| i.path.clone())
        .chain(
            identities
                .iter()
                .filter(|i| i.symlink.is_some())
                .filter_map(|i| {
                    fs::canonicalize(&i.path)
                        .ok()
                        .filter(|p| p.is_file())
                        .map(|_| i.path.clone())
                }),
        )
        .collect();
    Ok(Resolved {
        name: name.to_owned(),
        toolchain,
        mounts,
        identities,
        digest,
    })
}
/// Builds the exact policy bytes to sign. Contracts may select a toolchain but
/// cannot supply mounts, environment, networking or execution budgets.
pub fn policy(project: &Path, name: &str, checks: Vec<String>) -> Result<String> {
    let resolved = resolve(project, name)?;
    let body = serde_json::to_string(
        &serde_json::json!({"version":2,"toolchain":name,"toolchain_digest":resolved.digest,"checks":checks}),
    )?;
    crate::domain::verification_policy::ExecutionPolicy::parse(body.as_bytes())?;
    Ok(body)
}
pub(crate) fn for_policy(project: &Path, bytes: &[u8]) -> Result<Option<Resolved>> {
    let policy = crate::domain::verification_policy::ExecutionPolicy::parse(bytes)?;
    let Some(name) = &policy.toolchain else {
        return Ok(None);
    };
    let resolved = resolve(project, name)?;
    ensure!(
        policy.toolchain_digest.as_deref() == Some(&resolved.digest),
        "toolchain identity changed since signing (or signed digest missing)"
    );
    Ok(Some(resolved))
}
pub(crate) fn timeout(resolved: Option<&Resolved>, fallback: Duration) -> Duration {
    resolved.map_or(fallback, |r| {
        Duration::from_secs(r.toolchain.timeout_seconds)
    })
}
pub(crate) fn command_allowed(program: &str, checkout: &Path, resolved: Option<&Resolved>) -> bool {
    if resolved.is_none() {
        return crate::verification::program_allowed(program, checkout);
    }
    if let Some(relative) = program.strip_prefix("./") {
        let relative = Path::new(relative);
        return resolved.is_some()
            && !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            && crate::verification::program_allowed(
                &checkout.join(relative).display().to_string(),
                checkout,
            );
    }
    crate::verification::program_allowed(program, checkout)
        || resolved.is_some_and(|r| {
            r.toolchain.paths.iter().any(|path| {
                path == Path::new(program) || path.is_dir() && Path::new(program).starts_with(path)
            }) && r.mounts.iter().any(|path| path == Path::new(program))
        })
}
pub(crate) fn unchanged(resolved: &Resolved) -> bool {
    serde_json::to_vec(&(&resolved.toolchain, &resolved.identities))
        .is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == resolved.digest)
        && resolved.mounts.iter().all(|path| {
            resolved
                .identities
                .iter()
                .any(|i| &i.path == path && (i.sha256.is_some() || i.symlink.is_some()))
        })
        && resolved
            .identities
            .iter()
            .all(|i| identity(&i.path).is_ok_and(|current| current == *i))
}

/// Parse declared acceptance argv without shell expansion; shared by launch and telemetry.
pub fn split_accept_command(raw: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    let mut started = false;
    for c in raw.chars() {
        if escape {
            word.push(c);
            escape = false;
            started = true;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escape = true;
            started = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    ensure!(
        quote.is_none() && !escape,
        "unclosed quote or escape in --accept"
    );
    if started {
        args.push(word);
    }
    ensure!(!args.is_empty(), "empty --accept command");
    Ok(args)
}
