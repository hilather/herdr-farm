//! Append-only setup for the owner's first canonical project.
use crate::execution_guard::GatedSpawn;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    process::Command,
};

pub fn prepare(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let _lock = crate::execution_guard::exclusive_file(&dir.join(".owner-setup.lock"))?;
    let path = dir.join("config.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let config: toml::Table = toml::from_str(&text).context("invalid owner config")?;
    let mut append = String::new();
    if !config.contains_key("authority") {
        let private = dir.join("owner-approval");
        let public = dir.join("owner-approval.pub");
        // A crash after key generation must not overwrite the owner's key.
        if !private.exists() && !public.exists() {
            let output = Command::new("/usr/bin/ssh-keygen")
                .args([
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-C",
                    "owner-approval",
                    "-f",
                ])
                .arg(&private)
                .output_gated()?;
            ensure!(
                output.status.success(),
                "owner approval key generation failed"
            );
        }
        ensure!(
            private.is_file() && public.is_file(),
            "incomplete owner-approval key pair; preserve and inspect {}",
            dir.display()
        );
        fs::set_permissions(&private, fs::Permissions::from_mode(0o600))?;
        fs::set_permissions(&public, fs::Permissions::from_mode(0o644))?;
        let key = fs::read_to_string(&public)?;
        let fields: Vec<_> = key.split_whitespace().collect();
        ensure!(
            fields.len() >= 2 && fields[0] == "ssh-ed25519",
            "invalid owner approval public key"
        );
        let key = format!("{} {}", fields[0], fields[1]);
        append.push_str(&format!(
            "\n[authority]\nversion = 1\nrevision = 1\napproval_public_key = {}\n",
            toml::Value::String(key)
        ));
        println!(
            "created owner approval authority; private key: {}",
            private.display()
        );
    }
    if match config.get("profiles") {
        None => true,
        Some(toml::Value::Table(profiles)) => profiles.is_empty(),
        _ => false,
    } {
        for kind in ["codex", "claude"] {
            let found = std::env::var_os("PATH").is_some_and(|paths| {
                std::env::split_paths(&paths).any(|p| {
                    fs::metadata(p.join(kind))
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
            });
            if found {
                append.push_str(&format!("\n[profiles.{kind}]\nkind = \"{kind}\"\npermission_policy = \"interactive\"\n[profiles.{kind}.budget]\nmax_wall_seconds = 3600\nunknown_usage = \"allow_with_warning\"\n"));
                println!("added starter profile {kind} (agent default model)");
            }
        }
    }
    if !append.is_empty() {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(append.as_bytes())?;
        file.sync_all()?;
        fs::File::open(dir)?.sync_all()?;
    }
    println!("Claude workers need `claude setup-token` once.");
    Ok(())
}
