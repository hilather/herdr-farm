//! Build-script fixture embedded in the test archive; no runtime compiler.
use std::{fs,path::{Path,PathBuf},sync::OnceLock};
pub fn slow_read()-> &'static Path {
    static BUILD:OnceLock<(tempfile::TempDir,PathBuf)>=OnceLock::new();
    &BUILD.get_or_init(||{
        let directory=tempfile::tempdir().unwrap();
        let shim=directory.path().join("slow-read.so");
        fs::write(&shim,include_bytes!(concat!(env!("OUT_DIR"),"/control-slow-read.so"))).unwrap();
        (directory,shim)
    }).1
}
