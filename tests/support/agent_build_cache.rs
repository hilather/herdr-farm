//! Content-addressed Rust stand-ins; every lab still owns a separate executable.
#![allow(dead_code)] // Included by integration crates that use different support modules.
#![allow(clippy::disallowed_methods)] // Test-only compiler spawn.

use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

pub fn copy_agent(source: &str, destination: &Path) {
    let flags = ["--edition", "2021"];
    let mut digest = Sha256::new();
    for part in std::iter::once(source).chain(flags) {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("agents")
        .join(format!("{:x}", digest.finalize()));
    fs::create_dir_all(&directory).unwrap();
    // Keep the lock inode permanently: removing it could split waiting processes
    // between different locks. OS locks release automatically after a crash.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("build.lock"))
        .unwrap();
    lock.lock().unwrap();
    let executable = directory.join("agent");
    if !executable.is_file() {
        let build = tempfile::Builder::new()
            .prefix("build-")
            .tempdir_in(&directory)
            .unwrap();
        let input = build.path().join("agent.rs");
        let output = build.path().join("agent");
        fs::write(&input, source).unwrap();
        let built = Command::new("rustc")
            .args(flags)
            .arg("-o")
            .arg(&output)
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        fs::rename(&output, &executable).unwrap();
    }
    // Copy, rather than link, so lab replacements and chmods cannot alter the
    // cache or another lab. Rename also permits replacing a running executable.
    let copy = tempfile::NamedTempFile::new_in(destination.parent().unwrap()).unwrap();
    fs::copy(&executable, copy.path()).unwrap();
    fs::set_permissions(copy.path(), fs::Permissions::from_mode(0o700)).unwrap();
    copy.persist(destination).unwrap();
}
