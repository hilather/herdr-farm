//! Identity-aware paths for the running product, never a replacement binary.
use std::{path::PathBuf, os::unix::fs::MetadataExt};
use anyhow::{Context, Result};

#[derive(Debug)]
pub struct Unavailable;
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("herdr-farm binary was deleted or replaced since this process started; restart the ticker")
    }
}
impl std::error::Error for Unavailable {}

/// A pathname for delayed shell launches and filesystem layouts. Check inode
/// identity against the executing image, including atomic replacements.
pub fn real_path() -> Result<PathBuf> {
    let path = std::env::current_exe().map_err(|_| Unavailable)?;
    let live = std::fs::metadata("/proc/self/exe").map_err(|_| Unavailable)?;
    let named = std::fs::metadata(&path).map_err(|_| Unavailable)?;
    if (live.dev(), live.ino()) != (named.dev(), named.ino()) {
        return Err(Unavailable.into());
    }
    path.canonicalize().map_err(|_| Unavailable.into())
}

/// Pin re-exec/bind to this process's image. `self` would refer to an
/// intervening launcher (e.g. unshare), so qualify the proc path by our PID.
pub fn reexec() -> Result<PathBuf> {
    let path = PathBuf::from(format!("/proc/{}/exe", std::process::id()));
    std::fs::metadata(&path).context(Unavailable)?;
    Ok(path)
}

/// A sealed reference to this executable, inherited only by the selected child.
/// Parent proc links cannot be dereferenced across a new user namespace.
#[derive(Debug, Clone)]
pub struct Image(std::sync::Arc<std::fs::File>);
impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool { std::sync::Arc::ptr_eq(&self.0, &other.0) }
}
impl Image {
    pub(crate) fn pin() -> Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let file = std::fs::File::open("/proc/self/exe").context(Unavailable)?;
        // Keep it above stdio, CLOEXEC in the parent and every unrelated spawn.
        // SAFETY: this duplicates a live owned descriptor without accessing Rust memory.
        let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        if fd < 0 { return Err(std::io::Error::last_os_error()).context(Unavailable); }
        // SAFETY: fcntl returned a new owned descriptor, checked above.
        Ok(Self(std::sync::Arc::new(unsafe { std::fs::File::from_raw_fd(fd) })))
    }
    pub(crate) fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd;
        self.0.as_raw_fd()
    }
    pub(crate) fn path(&self) -> PathBuf { PathBuf::from(format!("/proc/self/fd/{}", self.fd())) }
}
