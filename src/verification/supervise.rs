//! Parent side of isolation. This builds a command; it does not switch the root.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::runner::Cmd;

pub struct Spec {
    pub unshare_program: PathBuf,
    pub timeout: Duration,
    pub checkout: PathBuf,
    pub policy: PathBuf,
    pub scratch: PathBuf,
    pub checks: Vec<String>,
    pub commit: String,
    pub tree: String,
    pub policy_digest: String,
    /// Hidden check inputs, bound read-only at the same paths (replay suite).
    pub hidden: Vec<PathBuf>,
    pub toolchain: Option<super::toolchains::Resolved>,
    pub repository: Option<PathBuf>,
}

pub struct Launch {
    pub cmd: Cmd,
    pub argv: Vec<String>,
}

pub fn launch(spec: &Spec) -> Result<Launch> {
    crate::self_executable::real_path()?;
    if spec.checks.is_empty() || spec.timeout.is_zero() {
        bail!("verification command is empty");
    }
    fs::create_dir_all(&spec.scratch).context("verification scratch directory")?;
    let host_mnt = fs::read_link("/proc/self/ns/mnt").context("mount namespace")?;
    let host_mnt = host_mnt
        .to_str()
        .context("mount namespace is not utf-8")?
        .to_string();
    let image = crate::self_executable::Image::pin()?;
    let program = image.path().display().to_string();
    if program.is_empty() {
        bail!("verification command is empty");
    }
    // The inherited image descriptor survives both user and PID namespaces,
    // including remounting proc. Setup closes it before running policy checks.
    let mut args = vec![
        "--user".into(),
        "--map-root-user".into(),
        "--mount".into(),
        "--propagation".into(),
        "private".into(),
        "--pid".into(),
        "--fork".into(),
        "--mount-proc".into(),
        "--kill-child=KILL".into(),
        "--".into(),
        program,
        "verification-setup".into(),
        "--host-mnt".into(),
        host_mnt,
        "--checkout".into(),
        spec.checkout.display().to_string(),
        "--policy".into(),
        spec.policy.display().to_string(),
        "--git".into(),
        "/usr/bin/git".into(),
    ];
    for hidden in &spec.hidden {
        args.extend(["--hidden".into(), hidden.display().to_string()]);
    }
    if spec.toolchain.as_ref().is_some_and(|r| !r.toolchain.network) {
        args.insert(0, "--net".into());
    }
    if let Some(resolved) = &spec.toolchain {
        let manifest = spec.scratch.with_extension("toolchain.json");
        fs::write(&manifest, serde_json::to_vec(resolved)?)?;
        args.extend(["--toolchain".into(), manifest.display().to_string()]);
    }
    if let Some(repository) = &spec.repository {
        args.extend(["--repository".into(), repository.display().to_string()]);
    }
    args.push("--".into());
    args.extend(spec.checks.iter().cloned());
    let mut argv = vec![spec.unshare_program.display().to_string()];
    argv.extend(args.iter().cloned());
    let mut cmd = Cmd::new(spec.unshare_program.display().to_string(), spec.timeout);
    cmd.args = args;
    cmd.env_clear = true;
    cmd.env = vec![
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
        ("HP_VERIFY_COMMIT".into(), spec.commit.clone()),
        ("HP_VERIFY_TREE".into(), spec.tree.clone()),
        ("HP_VERIFY_POLICY_DIGEST".into(), spec.policy_digest.clone()),
        (
            "HP_VERIFY_SCRATCH".into(),
            spec.scratch.display().to_string(),
        ),
    ];
    cmd.env.push(("HP_VERIFY_IMAGE_FD".into(), image.fd().to_string()));
    cmd.executable_image = Some(image);
    if let Some(resolved) = &spec.toolchain {
        cmd.env.push(("HP_VERIFY_CHECK_ENV".into(), serde_json::to_string(&resolved.toolchain.env)?));
    }
    if spec.toolchain.is_some() {
        cmd.env.push(("HP_VERIFY_DEADLINE_MONOTONIC_MS".into(), (super::repetitions::monotonic_ms().saturating_add(spec.timeout.as_millis() as u64)).to_string()));
    }
    cmd.cwd = Some(spec.checkout.clone());
    let _ = Path::new(&cmd.program);
    Ok(Launch { cmd, argv })
}
