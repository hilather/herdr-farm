//! Read-only local writer checkpoint shared by cleanup and migration.
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

#[cfg(target_os = "linux")]
fn mapped_path(value: &str) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = value.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            decoded.push(
                ((bytes[i + 1] - b'0') as u16 * 64
                    + (bytes[i + 2] - b'0') as u16 * 8
                    + (bytes[i + 3] - b'0') as u16) as u8,
            );
            i += 4;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    std::ffi::OsString::from_vec(decoded).into()
}
#[cfg(target_os = "linux")]
pub fn no_process_references(path: &Path) -> Result<()> {
    // Unrelated same-user processes can exit between /proc reads under load.
    // Retry that incomplete snapshot only; a discovered writer still fails closed.
    for attempt in 0..8 {
        match inspect_process_references(path) {
            Err(error) if attempt < 7 && error.to_string() == "process identity changed during writer inspection; retry the checkpoint" => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            result => return result,
        }
    }
    unreachable!()
}
#[cfg(target_os = "linux")]
fn inspect_process_references(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    // A checkpoint observes all visible same-user processes, not merely Herdr's
    // idle indicator. The operator separately confirms other known writers stopped.
    // SAFETY: geteuid takes no pointers and has no preconditions.
    let uid = unsafe { libc::geteuid() };
    for process in fs::read_dir("/proc")? {
        let process = process?;
        if !process
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let proc = process.path();
        let meta = match fs::metadata(&proc) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.uid() != uid {
            continue;
        }
        let check = || -> Result<()> {
            let cwd = fs::read_link(proc.join("cwd"))
                .with_context(|| format!("cannot inspect {}", proc.join("cwd").display()))?;
            ensure!(
                !cwd.starts_with(path),
                "process {} still has its cwd in the worktree",
                process.file_name().to_string_lossy()
            );
            for fd in fs::read_dir(proc.join("fd"))
                .with_context(|| format!("cannot inspect {}", proc.join("fd").display()))?
            {
                match fs::read_link(fd?.path()) {
                    Ok(target)
                        if process.file_name().to_string_lossy()
                            == std::process::id().to_string()
                            && target == path.join(".state/lock") => {}
                    Ok(target) => ensure!(
                        !target.starts_with(path),
                        "process {} still holds a worktree descriptor",
                        process.file_name().to_string_lossy()
                    ),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let maps = fs::read_to_string(proc.join("maps"))
                .with_context(|| format!("cannot inspect {}", proc.join("maps").display()))?;
            ensure!(
                !maps.lines().any(|line| line
                    .find('/')
                    .is_some_and(|start| mapped_path(&line[start..]).starts_with(path))),
                "process still maps worktree content"
            );
            Ok(())
        };
        match check() {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ESRCH)) =>
            {
                let gone = !proc.try_exists()?;
                let zombie = fs::read_to_string(proc.join("status")).is_ok_and(|s| {
                    s.lines()
                        .any(|l| l.starts_with("State:") && l.contains("Z (zombie)"))
                }) && fs::read_dir(proc.join("task"))
                    .is_ok_and(|tasks| tasks.count() == 1);
                ensure!(
                    gone || zombie,
                    "process identity changed during writer inspection; retry the checkpoint"
                );
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
                    && uid != 0
                    && privileged_or_defunct(&proc) => {}
            Err(error) => return Err(error).context("cannot establish writer quiescence"),
        }
    }
    Ok(())
}

/// The kernel refuses same-user inspection of zombies, non-dumpable processes
/// (their /proc links become root-owned) and processes holding capabilities the
/// caller lacks, such as `systemd --user` and `(sd-pam)`. A worker Herdr launches
/// is an ordinary dumpable process with the caller's capabilities, so none of
/// these is one; any other refusal still fails closed.
#[cfg(target_os = "linux")]
fn privileged_or_defunct(proc: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let field = |status: &str, key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
    };
    let Ok(status) = fs::read_to_string(proc.join("status")) else {
        return false;
    };
    if field(&status, "State:").is_some_and(|state| state.starts_with('Z')) {
        return true;
    }
    if fs::symlink_metadata(proc.join("cwd")).is_ok_and(|link| link.uid() == 0) {
        return true;
    }
    let permitted =
        |status: &str| field(status, "CapPrm:").and_then(|v| u64::from_str_radix(&v, 16).ok());
    let own = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| permitted(&s));
    matches!((permitted(&status), own), (Some(theirs), Some(own)) if theirs & !own != 0)
}
#[cfg(not(target_os = "linux"))]
pub fn no_process_references(_: &Path) -> Result<()> {
    anyhow::bail!("writer quiescence inspection is currently supported on Linux only")
}
