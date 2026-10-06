//! A typed clap-schema projection. No argv value reaches the telemetry row.
use clap::{Arg, ArgAction, Command};
use herdr_farm::telemetry::accounting::cli_invocations::{self, Invocation};
use std::{path::PathBuf, time::Instant};

const INTERNAL: &[&str] = &[
    "launch-exec",
    "report-hash",
    "artifact-stream",
    "verification-setup",
    "build-info",
];

fn excluded(path: &str) -> bool {
    INTERNAL.contains(&path.split(' ').next().unwrap_or("")) || path == "ticker run"
}

/// The exact registered command paths, also used to validate untrusted spool rows.
pub fn paths() -> &'static [String] {
    static PATHS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    PATHS.get_or_init(|| {
        fn visit(command: &Command, prefix: &str, out: &mut Vec<String>) {
            for child in command.get_subcommands() {
                let path = if prefix.is_empty() {
                    child.get_name().to_owned()
                } else {
                    format!("{prefix} {}", child.get_name())
                };
                if !excluded(&path) {
                    out.push(path.clone());
                    visit(child, &path, out);
                }
            }
        }
        let mut out = vec![String::new()];
        visit(&crate::cli::invocation_schema(), "", &mut out);
        out
    })
}

fn worker_spool() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        herdr_farm::submission_spool::worker_spool()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub struct Capture {
    started: Instant,
    project: Option<PathBuf>,
    row: Option<Invocation>,
    target: (Option<String>, Option<String>, bool),
}

impl Capture {
    pub fn caller(&self) -> (&str, &str) {
        self.row.as_ref().map_or(("operator", "local"), |r| (r.caller.as_str(), r.trust.as_str()))
    }

    pub fn start() -> Self {
        let started = Instant::now();
        // Disable clap's early exits only in this metadata projection. Its
        // partial matches retain typed positional IDs and canonical names.
        fn partial(command: &mut Command) {
            *command = command
                .clone()
                .disable_help_flag(true)
                .disable_version_flag(true)
                .disable_help_subcommand(true)
                .ignore_errors(true);
            for child in command.get_subcommands_mut() {
                partial(child);
            }
        }
        let mut schema = crate::cli::invocation_schema();
        partial(&mut schema);
        schema = schema
            .arg(
                Arg::new("capture_help")
                    .long("help")
                    .short('h')
                    .global(true)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("capture_version")
                    .long("version")
                    .short('V')
                    .global(true)
                    .action(ArgAction::SetTrue),
            );
        let Ok(matches) = schema.try_get_matches_from(std::env::args_os()) else {
            return Self {
                started,
                project: None,
                row: None,
                target: (None, None, false),
            };
        };
        let root = matches
            .try_get_one::<PathBuf>("root")
            .ok()
            .flatten()
            .cloned();
        let mut current = &matches;
        let mut names = Vec::new();
        let mut slug = None;
        loop {
            if let Some(value) = current.try_get_one::<String>("slug").ok().flatten()
                && crate::project::validate_slug(value).is_ok()
            {
                slug = Some(value.clone());
            }
            let Some((name, child)) = current.subcommand() else {
                break;
            };
            names.push(name);
            current = child;
        }
        let path = names.join(" ");
        if excluded(&path) {
            return Self {
                started,
                project: None,
                row: None,
                target: (None, None, false),
            };
        }
        let project = slug.as_ref().and_then(|slug| {
            let root = if let Some(root) = root.as_deref() {
                std::path::absolute(root).ok()?
            } else {
                // Help must still work with an unrelated non-UTF-8 environment
                // value; Env::from_process uses the Unicode-only env iterator.
                if std::env::vars_os()
                    .any(|(name, value)| name.to_str().is_none() || value.to_str().is_none())
                {
                    return None;
                }
                let env = crate::paths::Env::from_process().ok()?;
                crate::paths::resolve_root(None, &env, &env.config_dir()).ok()?
            };
            Some(root.join(slug))
        });
        let worker = worker_spool().is_some();
        let caller = if worker {
            "worker"
        } else if project
            .as_ref()
            .is_some_and(|p| crate::canonical_coordinator::caller_is_coordinator(p))
        {
            "coordinator"
        } else if std::env::vars_os().any(|(name, _)| {
            name.to_str()
                .is_some_and(|s| s.starts_with("HERDR_PLUGIN_"))
        }) {
            "plugin"
        } else if std::env::var_os("HERDR_FARM_TICKER_CHILD").is_some() {
            "ticker"
        } else {
            "operator"
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        use sha2::{Digest, Sha256};
        let id = format!(
            "{:x}",
            Sha256::digest(format!("{}:{}", std::process::id(), stamp.as_nanos()))
        );
        let target_id = |name: &str| current.try_get_one::<String>(name).ok().flatten()
            .filter(|s| !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))).cloned();
        let target = if matches!(path.as_str(), "launch run" | "launch stop" | "task cancel-attempt") {
            (target_id("task"), target_id("attempt"), current.try_get_one::<bool>("force").ok().flatten().copied().unwrap_or(false))
        } else { (None, None, false) };
        Self {
            started,
            project,
            target,
            row: Some(Invocation {
                invocation_id: id,
                command_path: path,
                outcome: "ok".into(),
                error_class: None,
                exit_code: 0,
                duration_ms: 0,
                project_slug: slug,
                caller: caller.into(),
                trust: if worker { "worker_reported" } else { "local" }.into(),
                recorded_unix_ms: stamp.as_millis().min(i64::MAX as u128) as i64,
            }),
        }
    }

    pub fn failure(&mut self, error: &anyhow::Error) {
        if let Some(row) = &mut self.row {
            row.error_class = Some(cli_invocations::ErrorClass::classify(error));
        }
    }

    /// Called only after command execution has unwound its project guards;
    /// parse exits call this before clap exits, having acquired no guards.
    pub fn finish(&mut self, outcome: &str, code: i32) {
        let Some(mut row) = self.row.take() else {
            return;
        };
        row.outcome = outcome.into();
        if outcome == "usage_error" {
            row.error_class = Some(cli_invocations::ErrorClass::Usage);
        } else if code != 0 && row.error_class.is_none() {
            row.error_class = Some(cli_invocations::ErrorClass::Internal);
        }
        row.exit_code = code;
        row.duration_ms = self.started.elapsed().as_millis().min(i64::MAX as u128) as i64;
        #[cfg(target_os = "linux")]
        if let Some(spool) = worker_spool() {
            let _ = herdr_farm::submission_spool::append_cli_invocation(&spool, &row);
            return;
        }
        if let Some(project) = &self.project {
            let targeted = matches!(row.command_path.as_str(), "launch run" | "launch stop" | "task cancel-attempt");
            let invocation = row.invocation_id.clone();
            if cli_invocations::write(project, &[row]).is_ok() && targeted {
                let _ = herdr_farm::telemetry::operations::launch::cli_target(project, &invocation, self.target.0.as_deref(), self.target.1.as_deref(), self.target.2);
            }
        }
    }
}
