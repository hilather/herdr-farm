#![allow(clippy::disallowed_methods)] // The build script holds no execution locks.
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// No rerun-if-changed lines on purpose: cargo then reruns this script whenever
// any file in the package changes, so a rebuilt binary always gets a new build id.
fn main() {
    // Embed one shared fault fixture in E2E binaries, so archived CI tests need
    // neither a compiler nor a per-test build in their isolated labs.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref()==Ok("linux") {
        let fixture=std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("control-slow-read.so");
        let output=Command::new("cc").args(["-shared","-fPIC","-o"]).arg(&fixture)
            .args(["tests/fixtures/control-publication/slow-read.c","-ldl"]).output()
            .expect("C compiler required for shared E2E fault fixture");
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    }
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nogit".to_string());
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=HP_BUILD_ID={hash}.{secs}");
}
