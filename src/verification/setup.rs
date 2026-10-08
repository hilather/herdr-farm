//! Child side of isolation. `unshare` has already started the new namespaces.
//! The same-namespace check returns before any mount.
use crate::execution_guard::GatedSpawn;
use std::{
    ffi::CString,
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use crate::verification::{hidden_digest, parse_checks, parse_hidden};

const EXIT_SAME_NS: i32 = 71;
const EXIT_SETUP: i32 = 72;
const EXIT_TAMPER: i32 = 73;
const EXIT_LEFTOVER: i32 = 74;
const EXIT_POLICY: i32 = 75;
const EXIT_CHECKS: i32 = 76;
const EXIT_CHECK_FAILED: i32 = 77;

pub fn setup_main() -> i32 {
    setup_from_args(&std::env::args().collect::<Vec<_>>())
}

pub fn setup_from_args(args: &[String]) -> i32 {
    let Some(parsed) = parse_args(args) else {
        return fail("args", 0);
    };
    // Mount only after a successful read proves this is a different mount namespace.
    let current = match fs::read_link("/proc/self/ns/mnt") {
        Ok(path) => path.into_os_string().into_string().ok(),
        Err(_) => None,
    };
    if !namespace_is_new(current.as_deref(), &parsed.host_mnt) {
        eprintln!("hp-verify same-namespace");
        return EXIT_SAME_NS;
    }
    // Only the trusted runner can create the sealed inherited image. Do not
    // leave its descriptor visible to untrusted policy commands.
    if let Ok(value) = std::env::var("HP_VERIFY_IMAGE_FD") {
        let Ok(fd) = value.parse::<i32>() else { return fail("image-fd", 0); };
        if fd < 3 { return fail("image-fd", 0); }
        // SAFETY: this is the runner's explicitly inherited executable handle.
        if unsafe { libc::close(fd) } != 0 { return fail("image-fd", 0); }
    }
    enter(&parsed)
}

/// `None` is a failed read or a non-UTF-8 id. That must not mount.
pub(super) fn namespace_is_new(current: Option<&str>, host_mnt: &str) -> bool {
    matches!(current, Some(current) if current != host_mnt)
}

struct Args {
    worker_uid: Option<(u32, u32)>,
    host_mnt: String,
    checkout: PathBuf,
    policy: PathBuf,
    git: PathBuf,
    hidden: Vec<PathBuf>,
    checks: Vec<String>,
    toolchain: Option<super::toolchains::Resolved>,
    repository: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Option<Args> {
    let start = args.iter().position(|arg| arg == "verification-setup")? + 1;
    let mut worker_uid = None;
    let mut host_mnt = None;
    let mut checkout = None;
    let mut policy = None;
    let mut git = None;
    let mut hidden = Vec::new();
    let mut checks = Vec::new();
    let mut toolchain = None;
    let mut repository = None;
    let mut index = start;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            checks.extend(args[index + 1..].iter().cloned());
            break;
        }
        let value = args.get(index + 1)?;
        match arg.as_str() {
            "--worker-uid" => {
                let (uid, gid) = value.split_once(':')?;
                worker_uid = Some((uid.parse().ok()?, gid.parse().ok()?));
            }
            "--host-mnt" => host_mnt = Some(value.clone()),
            "--checkout" => checkout = Some(PathBuf::from(value)),
            "--policy" => policy = Some(PathBuf::from(value)),
            "--git" => git = Some(PathBuf::from(value)),
            "--repository" => repository = Some(PathBuf::from(value)),
            "--toolchain" => toolchain = Some(serde_json::from_slice(&fs::read(value).ok()?).ok()?),
            "--hidden" => hidden.push(PathBuf::from(value)),
            _ => return None,
        }
        index += 2;
    }
    Some(Args {
        worker_uid,
        host_mnt: host_mnt?,
        checkout: checkout?,
        policy: policy?,
        git: git?,
        hidden,
        checks,
        toolchain,
        repository,
    })
}

fn enter(parsed: &Args) -> i32 {
    let commit = std::env::var("HP_VERIFY_COMMIT").unwrap_or_default();
    let tree = std::env::var("HP_VERIFY_TREE").unwrap_or_default();
    let policy_digest = std::env::var("HP_VERIFY_POLICY_DIGEST").unwrap_or_default();
    let scratch = std::env::var("HP_VERIFY_SCRATCH").unwrap_or_default();
    if commit.is_empty() || tree.is_empty() || policy_digest.len() != 64 || scratch.is_empty() {
        return fail("env", 0);
    }
    let scratch = PathBuf::from(scratch);
    if parsed.toolchain.as_ref().is_some_and(|r| !super::toolchains::unchanged(r)) { return EXIT_POLICY; }
    if parsed.toolchain.as_ref().is_some_and(|r| !r.toolchain.network)
        && let Err(error) = enable_loopback()
    {
        return fail("loopback", error);
    }
    // Keep a private reference to the writable setup proc mount. Checks see
    // only the read-only proc mounted below; map writes happen before exec.
    let mapping_proc = if parsed.worker_uid.is_some() {
        use std::os::unix::fs::OpenOptionsExt;
        match fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC).open("/proc") {
            Ok(file) => Some(file),
            Err(error) => return fail("mapping-proc", error.raw_os_error().unwrap_or(0)),
        }
    } else { None };
    let libraries = match Command::new("/usr/bin/ldd").arg(&parsed.git).output_gated() {
        Ok(output) if output.status.success() => {
            super::manifest::parse_ldd(&String::from_utf8_lossy(&output.stdout))
        }
        _ => return fail("ldd", 0),
    };
    if libraries.is_empty() {
        return fail("ldd", 0);
    }
    if let Err(errno) = switch_root(&scratch, parsed, &libraries) {
        return fail("root", errno);
    }
    if !drop_privileges() { return fail("drop-privileges", 0); }
    if mapping_proc.is_some() {
        // A check must never reopen the parent's private proc descriptor via
        // /proc/1/fd. A descendant user namespace cannot override this fence.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 { return fail("mapping-fence", 0); }
    }
    use std::os::fd::AsRawFd;
    let worker_identity = parsed.worker_uid.map(|(uid, gid)| (uid, gid, mapping_proc.as_ref().expect("owner mapping proc").as_raw_fd()));
    let bytes = match fs::read(&parsed.policy) {
        Ok(bytes) => bytes,
        Err(error) => return fail("policy", error.raw_os_error().unwrap_or(0)),
    };
    if sha256(&bytes) != policy_digest {
        return EXIT_POLICY;
    }
    let checks = match parse_checks(&bytes) {
        Ok(checks) if checks == parsed.checks && super::toolchains::command_allowed(&checks[0], &parsed.checkout, parsed.toolchain.as_ref()) => {
            checks
        }
        Ok(_) => return EXIT_CHECKS,
        Err(_) => return EXIT_POLICY,
    };
    // Hidden inputs: exactly the policy's, bound read-only, each still at its pinned digest.
    match parse_hidden(&bytes) {
        Ok(hidden) if hidden.len() == parsed.hidden.len()
            && hidden.iter().zip(&parsed.hidden).all(|(input, path)| Path::new(&input.path) == path
                && hidden_digest(path).as_deref() == Some(input.sha256.as_str())) => {}
        _ => return EXIT_POLICY,
    }
    let seen_commit = match git_line(&parsed.git, &parsed.checkout, &["rev-parse", "HEAD"]) {
        Some(value) => value,
        None => return fail("rev-parse", 0),
    };
    let seen_tree = match git_line(
        &parsed.git,
        &parsed.checkout,
        &["rev-parse", &format!("{commit}^{{tree}}")],
    ) {
        Some(value) => value,
        None => return fail("rev-parse", 0),
    };
    eprintln!("hp-verify commit={seen_commit}");
    eprintln!("hp-verify tree={seen_tree}");
    if seen_commit != commit || seen_tree != tree {
        clean_tree(&parsed.git, &parsed.checkout);
        return EXIT_TAMPER;
    }
    // Both the index and worktree must match HEAD before executing checks.
    match clean_tree(&parsed.git, &parsed.checkout) {
        Some(true) => {}
        Some(false) => return EXIT_TAMPER,
        None => return fail("diff", 0),
    }
    // The copied checkout is the check cwd. Signed argv cannot name that path.
    // Stream to the parent supervisor, which bounds capture. Do not buffer
    // arbitrary test output in this isolated process.
    let policy = match crate::domain::verification_policy::ExecutionPolicy::parse(&bytes) {
        Ok(policy) => policy,
        Err(_) => return EXIT_POLICY,
    };
    if policy.toolchain.as_deref() != parsed.toolchain.as_ref().map(|r| r.name.as_str())
        || policy.toolchain_digest.as_deref() != parsed.toolchain.as_ref().map(|r| r.digest.as_str()) {
        return EXIT_POLICY;
    }
    if !policy.commands().all(|args| super::toolchains::command_allowed(&args[0], &parsed.checkout, parsed.toolchain.as_ref())) {
        return EXIT_CHECKS;
    }
    let code = if policy.version == 2 {
        match super::repetitions::execute(&policy, &parsed.checkout, worker_identity) {
            Ok(code) => code,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return 78,
            Err(error) => return fail("exec", error.raw_os_error().unwrap_or(0)),
        }
    } else {
        let status = match super::check_command(&checks[0], worker_identity)
            .args(&checks[1..])
            .current_dir(&parsed.checkout)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status_gated()
        {
            Ok(output) => output,
            Err(error) => return fail("exec", error.raw_os_error().unwrap_or(0)),
        };
        status.code()
    };
    match code {
        Some(code) => eprintln!("hp-verify checks={code}"),
        None => eprintln!("hp-verify checks=signal"),
    }
    reap();
    if leftover() {
        return EXIT_LEFTOVER;
    }
    // Checks run against a writable disposable copy. Successful execution is
    // not proof that HEAD, the index, and tracked inputs still match the pin.
    if git_line(&parsed.git,&parsed.checkout,&["rev-parse","HEAD"]).as_deref()!=Some(commit.as_str())
        || git_line(&parsed.git,&parsed.checkout,&["rev-parse","HEAD^{tree}"]).as_deref()!=Some(tree.as_str()) {
        clean_tree(&parsed.git, &parsed.checkout);
        return EXIT_TAMPER;
    }
    match clean_tree(&parsed.git,&parsed.checkout) {
        Some(true)=>{},
        Some(false)=>return EXIT_TAMPER,
        None=>return fail("post-check-diff",0),
    }
    // Child exit codes may overlap supervisor failures (71..77); classify the
    // check outcome through a distinct supervisor code and retain its real code.
    if code==Some(0) {0} else {EXIT_CHECK_FAILED}
}

fn git_line(git: &Path, checkout: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(git)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .output_gated()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    Some(text.to_string())
}

fn clean_tree(git: &Path, checkout: &Path) -> Option<bool> {
    // Porcelain -z preserves repository paths without Git's quoting rules.
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"]).arg(checkout)
        .args(["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all", "--ignore-submodules=none"])
        .output_gated().ok()?;
    if !output.status.success() { return None; }
    let records = output.stdout.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut count = 0usize;
    for record in records {
        if record.len() < 4 { return None; }
        let status = &record[..2];
        let kind = if status == b"??" { "added-untracked" }
            else if status.contains(&b'D') { "deleted" } else { "modified" };
        count += 1;
        if count <= 50 {
            let path = String::from_utf8_lossy(&record[3..]);
            // Escape protocol separators; report names only, never file contents.
            let path = path.replace('%', "%25").replace('\n', "%0A").replace('\r', "%0D").replace('\t', "%09");
            let mut end = path.len().min(300);
            while !path.is_char_boundary(end) { end -= 1; }
            if end < path.len() { eprintln!("hp-verify changed-truncated=true"); }
            eprintln!("hp-verify changed={kind}\t{}", &path[..end]);
        }
    }
    eprintln!("hp-verify changed-count={count}");
    Some(count == 0)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn fail(step: &str, errno: i32) -> i32 {
    eprintln!("hp-verify setup-error={step} errno={errno}");
    EXIT_SETUP
}

fn switch_root(scratch: &Path, parsed: &Args, libraries: &[PathBuf]) -> Result<(), i32> {
    mount_path(
        Some("none"),
        "/",
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
    )?;
    mount_path(
        Some("tmpfs"),
        &scratch.display().to_string(),
        Some("tmpfs"),
        0,
        Some("mode=755"),
    )?;
    for directory in ["old", "dev", "proc", "tmp"] {
        fs::create_dir_all(scratch.join(directory))
            .map_err(|error| error.raw_os_error().unwrap_or(1))?;
    }
    // Install private /tmp before copying paths: a checkout or toolchain under
    // /tmp must not be hidden by a later mount on its ancestor.
    let tmp = scratch.join("tmp").display().to_string();
    mount_path(Some("tmpfs"), &tmp, Some("tmpfs"), libc::MS_NOSUID | libc::MS_NODEV, Some("mode=1777"))?;
    let proc = scratch.join("proc");
    mount_path(
        Some("proc"),
        &proc.display().to_string(),
        Some("proc"),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | libc::MS_RDONLY,
        None,
    )?;
    device(scratch, "null", 1, 3, "/dev/null")?;
    device(scratch, "urandom", 1, 9, "/dev/urandom")?;
    bind_ro(scratch, &parsed.git)?;
    for library in libraries {
        bind_ro(scratch, library)?;
    }
    if let Some(resolved) = &parsed.toolchain {
        for identity in &resolved.identities {
            if identity.sha256.is_none() && identity.symlink.is_none() {
                fs::create_dir_all(scratch.join(identity.path.strip_prefix("/").map_err(|_| 1)?)).map_err(|e| e.raw_os_error().unwrap_or(1))?;
            }
        }
        for path in &resolved.mounts { bind_toolchain_ro_at(scratch, path, path)?; }
    }
    bind_ro(scratch, &parsed.policy)?;
    for hidden in &parsed.hidden {
        bind_ro(scratch, hidden)?;
    }
    // A private copy, not a bind of the live host directory. Host writes cannot land after the check.
    copy_checkout(scratch, &parsed.checkout)?;
    // Keep only the disposable checkout and /tmp writable.
    let checkout_dest = scratch.join(parsed.checkout.strip_prefix("/").map_err(|_| 1)?);
    let checkout_text = checkout_dest.display().to_string();
    mount_path(Some(&checkout_text), &checkout_text, None, libc::MS_BIND, None)?;
    if let (Some(resolved), Some(repository)) = (&parsed.toolchain, &parsed.repository) {
        for source in &resolved.mounts {
            if let Ok(relative) = source.strip_prefix(repository) {
                // Explicit owner tools inside a repository (e.g. ignored
                // .tools/Godot) are also available relative to its private copy.
                let alias = parsed.checkout.join(relative);
                bind_toolchain_ro_at(scratch, source, &alias)?;
            }
        }
    }
    mount_path(None, &scratch.display().to_string(), None, libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV, None)?;
    let put_old = scratch.join("old");
    pivot(
        &scratch.display().to_string(),
        &put_old.display().to_string(),
    )?;
    if unsafe { libc::chdir(c_str("/")?.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(1));
    }
    if unsafe { libc::umount2(c_str("/old")?.as_ptr(), libc::MNT_DETACH) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(1));
    }
    let _ = fs::remove_dir("/old");
    Ok(())
}

fn device(root: &Path, name: &str, major: u32, minor: u32, source: &str) -> Result<(), i32> {
    let dest = root.join("dev").join(name);
    let path = c_str(&dest.display().to_string())?;
    let dev = libc::makedev(major, minor);
    let created = unsafe { libc::mknod(path.as_ptr(), libc::S_IFCHR | 0o666, dev) };
    if created == 0 {
        return Ok(());
    }
    bind_ro(root, Path::new(source))
}

fn copy_checkout(scratch: &Path, checkout: &Path) -> Result<(), i32> {
    let relative = checkout.strip_prefix("/").map_err(|_| 1)?;
    let dest = scratch.join(relative);
    if dest.starts_with(checkout) || checkout.starts_with(&dest) {
        return Err(1);
    }
    copy_snapshot(checkout, &dest)
}

fn copy_snapshot(source: &Path, dest: &Path) -> Result<(), i32> {
    let meta = fs::symlink_metadata(source).map_err(|error| error.raw_os_error().unwrap_or(1))?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(source).map_err(|error| error.raw_os_error().unwrap_or(1))?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|error| error.raw_os_error().unwrap_or(1))?;
        }
        std::os::unix::fs::symlink(&target, dest)
            .map_err(|error| error.raw_os_error().unwrap_or(1))?;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dest).map_err(|error| error.raw_os_error().unwrap_or(1))?;
        for entry in fs::read_dir(source).map_err(|error| error.raw_os_error().unwrap_or(1))? {
            let entry = entry.map_err(|error| error.raw_os_error().unwrap_or(1))?;
            copy_snapshot(&entry.path(), &dest.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if meta.is_file() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|error| error.raw_os_error().unwrap_or(1))?;
        }
        fs::copy(source, dest).map_err(|error| error.raw_os_error().unwrap_or(1))?;
        return Ok(());
    }
    Err(1)
}

fn bind_ro(root: &Path, source: &Path) -> Result<(), i32> {
    bind_ro_at(root, source, source)
}

// libmount applies read-only attributes recursively, including nested mounts.
// A plain bind remount only protects the top mount.
fn bind_toolchain_ro_at(root: &Path, source: &Path, target: &Path) -> Result<(), i32> {
    bind_ro_at(root, source, target)?;
    let status = Command::new("/usr/bin/mount")
        .args(["--rbind", "-o", "ro=recursive"])
        .arg(source)
        .arg(root.join(target.strip_prefix("/").map_err(|_| 1)?))
        .status_gated()
        .map_err(|error| error.raw_os_error().unwrap_or(1))?;
    if status.success() { Ok(()) } else { Err(libc::EIO) }
}

fn bind_ro_at(root: &Path, source: &Path, target: &Path) -> Result<(), i32> {
    let relative = target.strip_prefix("/").map_err(|_| 1)?;
    let dest = root.join(relative);
    // A worker checkout can contain symlink ancestors. Never follow them while
    // still in the host root, including when exposing ignored repository tools.
    let mut ancestor = root.to_path_buf();
    for component in relative.components() {
        ancestor.push(component);
        if fs::symlink_metadata(&ancestor).is_ok_and(|m| m.file_type().is_symlink()) { return Err(libc::ELOOP); }
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|error| error.raw_os_error().unwrap_or(1))?;
    }
    match fs::symlink_metadata(&dest) {
        Ok(meta) if meta.is_file() => {},
        Ok(_) => return Err(libc::EINVAL),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::OpenOptions::new().write(true).create_new(true).mode(0o644)
                .custom_flags(libc::O_NOFOLLOW).open(&dest)
                .map_err(|error| error.raw_os_error().unwrap_or(1))?;
        }
        Err(error) => return Err(error.raw_os_error().unwrap_or(1)),
    }
    let source_text = source.display().to_string();
    let dest_text = dest.display().to_string();
    mount_path(Some(&source_text), &dest_text, None, libc::MS_BIND, None)?;
    // Device nodes reject a read-only remount in this user namespace; files do not.
    if source.starts_with("/dev/") {
        return Ok(());
    }
    // This kernel rejects a read-only bind remount unless the implicit nosuid/nodev flags are restated.
    mount_path(
        None,
        &dest_text,
        None,
        libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
        None,
    )
}

fn mount_path(
    source: Option<&str>,
    target: &str,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> Result<(), i32> {
    let source = source.map(c_str).transpose()?;
    let target_c = c_str(target)?;
    let fstype = fstype.map(c_str).transpose()?;
    let data = data.map(c_str).transpose()?;
    let rc = unsafe {
        libc::mount(
            source
                .as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(std::ptr::null()),
            target_c.as_ptr(),
            fstype
                .as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(std::ptr::null()),
            flags,
            data.as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(std::ptr::null()) as *const libc::c_void,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(1);
        eprintln!("hp-verify mount-fail target={target} flags={flags:#x} errno={err}");
        Err(err)
    }
}

fn pivot(new_root: &str, put_old: &str) -> Result<(), i32> {
    let new_root = c_str(new_root)?;
    let put_old = c_str(put_old)?;
    let rc = unsafe { libc::syscall(libc::SYS_pivot_root, new_root.as_ptr(), put_old.as_ptr()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(1))
    }
}

fn c_str(text: &str) -> Result<CString, i32> {
    CString::new(text).map_err(|_| 1)
}

fn reap() {
    loop {
        let mut status = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break;
        }
    }
}

fn leftover() -> bool {
    let self_pid = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return true;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.chars().all(|character| character.is_ascii_digit()) {
            if name.parse::<u32>().ok() != Some(self_pid) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[used]
#[unsafe(link_section = ".init_array")]
static VERIFICATION_SETUP_HOOK: unsafe extern "C" fn() = enter_verification_setup_hook;

#[cfg(test)]
unsafe extern "C" fn enter_verification_setup_hook() {
    if std::env::args().any(|arg| arg == "verification-setup") {
        std::process::exit(setup_from_args(&std::env::args().collect::<Vec<_>>()));
    }
}

// A check must not remount owner toolchain binds writable or regain privileges.
fn drop_privileges() -> bool {
    #[repr(C)] struct Header { version: u32, pid: i32 }
    #[repr(C)] #[derive(Clone, Copy)] struct Data { effective: u32, permitted: u32, inheritable: u32 }
    let last = match fs::read_to_string("/proc/sys/kernel/cap_last_cap").ok().and_then(|s| s.trim().parse::<i32>().ok()) {
        Some(last) if (0..=63).contains(&last) => last,
        _ => return false,
    };
    for capability in 0..=last {
        // SAFETY: a numeric Linux capability, with no pointer arguments.
        if unsafe { libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) } != 0 { return false; }
    }
    let header = Header { version: 0x20080522, pid: 0 };
    let data = [Data { effective: 0, permitted: 0, inheritable: 0 }; 2];
    // SAFETY: Linux capset receives two v3 capability records; prctl has no pointers.
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0
            && libc::syscall(libc::SYS_capset, &header, data.as_ptr()) == 0
    }
}

/// Called only in the fresh network namespace, before dropping capabilities.
fn enable_loopback() -> Result<(), i32> {
    // SAFETY: socket has no pointer arguments; ifreq is zeroed and contains
    // a NUL-terminated interface name. ioctl receives a live ifreq pointer.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 { return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)); }
        let mut request: libc::ifreq = std::mem::zeroed();
        request.ifr_name[0] = b'l' as libc::c_char;
        request.ifr_name[1] = b'o' as libc::c_char;
        let result = if libc::ioctl(fd, libc::SIOCGIFFLAGS, &mut request) < 0 {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        } else {
            request.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
            if libc::ioctl(fd, libc::SIOCSIFFLAGS, &request) < 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)) } else { Ok(()) }
        };
        libc::close(fd);
        result
    }
}
