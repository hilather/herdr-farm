//! Observational coordinator host checks. No output bytes enter the sidecar.
use crate::execution_guard::GatedSpawn;
use anyhow::{Context, Result, ensure};
use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::OpenOptionsExt,
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    pub task: String,
    /// Bind this submission; otherwise resolve the task's unique latest submission.
    #[arg(long, value_name = "ID")]
    pub submission: Option<String>,
    /// Check name (defaults to the command's basename).
    #[arg(long)]
    pub name: Option<String>,
    /// Working directory (defaults to the caller's current directory).
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// Send TERM to the command group at this deadline, then KILL after two seconds.
    #[arg(long, value_name="SECONDS", value_parser=clap::value_parser!(u64).range(1..))]
    pub timeout: Option<u64>,
    /// Replace FILE with combined stdout/stderr; only path, size and SHA-256 are stored.
    #[arg(long, value_name = "FILE")]
    pub log: Option<PathBuf>,
    #[arg(required = true, last = true)]
    pub command: Vec<String>,
}
fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn spool(project: &Path) -> PathBuf {
    project.join(".state/host-check-spool")
}
fn locked_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "host-check spool must be a regular file"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(file)
}
fn write_row(project: &Path, row: &Value) -> Result<()> {
    let mut db =
        super::super::sidecar::open_nowait(project, true)?.context("sidecar unavailable")?;
    // One short IMMEDIATE transaction; no handle survives this call.
    db.busy_timeout(Duration::from_secs(5))?;
    let cutoff = super::super::maintenance::store::path(project);
    if cutoff.exists() {
        let ops = super::super::read_only_nowait(&cutoff)?;
        let before: Option<i64> = ops.query_row(
            "SELECT max(before_unix_ms) FROM tombstones WHERE class=?1 AND key_digest IS NULL",
            [super::super::maintenance::CLI],
            |r| r.get(0),
        )?;
        if before.is_some_and(|before| row["started_unix_ms"].as_i64().unwrap_or(0) < before) {
            return Ok(());
        }
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("INSERT INTO host_checks(run_id,task,started_unix_ms,finished_unix_ms,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(run_id) DO UPDATE SET finished_unix_ms=excluded.finished_unix_ms,payload=excluded.payload WHERE host_checks.finished_unix_ms IS NULL", params![row["run_id"].as_str(),row["task"].as_str(),row["started_unix_ms"].as_i64(),row["finished_unix_ms"].as_i64(),row.to_string()])?;
    tx.commit()?;
    Ok(())
}
fn record(project: &Path, row: &Value) {
    if let Err(error) = write_row(project, row) {
        let append = || -> Result<()> {
            fs::create_dir_all(spool(project))?;
            let id = row["run_id"].as_str().context("run id")?;
            let mut file = locked_file(&spool(project).join(format!("{id}.jsonl")))?;
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::End(0))?;
            writeln!(file, "{row}")?;
            file.sync_all()?;
            Ok(())
        };
        if let Err(spool_error) = append() {
            eprintln!("host-check metadata unavailable: {error:#}; spool: {spool_error:#}");
        } else {
            eprintln!("host-check metadata spooled: {error:#}");
        }
    }
}
/// Per-run spool locks protect append/replay only, never execution. Keep empty
/// files so an appender cannot race unlink and write to an unlinked inode.
pub fn ingest(project: &Path) {
    let Ok(entries) = fs::read_dir(spool(project)) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let replay = || -> Result<()> {
            let text = {
                let mut file = locked_file(&entry.path())?;
                let mut text = String::new();
                file.read_to_string(&mut text)?;
                text
            };
            // SQLite writes happen after releasing the file lock. A finishing
            // check can append while replay waits for a busy sidecar.
            for line in text.lines() {
                let row: Value = serde_json::from_str(line)?;
                write_row(project, &row)?;
            }
            let mut file = locked_file(&entry.path())?;
            let mut current = String::new();
            file.read_to_string(&mut current)?;
            if current == text {
                file.set_len(0)?;
                file.sync_all()?;
            }
            // Concurrent appends keep the whole file; already replayed lines
            // dedupe on the next pass, so no newly appended data is discarded.
            Ok(())
        };
        if let Err(error) = replay() {
            eprintln!("host-check spool replay deferred: {error:#}");
        }
    }
}
fn process_start(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(')')?.1;
    if tail.split_whitespace().next() == Some("Z") {
        return None;
    }
    tail.split_whitespace().nth(19).map(str::to_owned)
}
fn alive(row: &Value) -> bool {
    row["pid"]
        .as_u64()
        .and_then(|pid| process_start(pid as u32))
        .is_some_and(|start| row["process_start"] == start)
}
pub fn read(project: &Path, task: Option<&str>) -> Result<Value> {
    let mut runs = Vec::<Value>::new();
    if let Some(db) = super::super::sidecar::read(project)? {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='host_checks')",
            [],
            |r| r.get(0),
        )?;
        if exists {
            let mut stmt = db.prepare("SELECT payload FROM host_checks WHERE (?1 IS NULL OR task=?1) ORDER BY started_unix_ms DESC,run_id DESC")?;
            for text in stmt.query_map([task], |r| r.get::<_, String>(0))? {
                let mut row: Value = serde_json::from_str(&text?)?;
                if row["outcome"] == "running" && !alive(&row) {
                    row["outcome"] = json!("abandoned");
                }
                runs.push(row);
            }
        }
    }
    let mut totals = std::collections::BTreeMap::<String, Value>::new();
    for run in &runs {
        let total = totals.entry(run["task"].as_str().unwrap_or("").into()).or_insert_with(|| json!({"runs":0,"first_run_outcome":null,"total_minutes":0.0,"latest_outcome":run["outcome"]}));
        total["runs"] = json!(total["runs"].as_u64().unwrap_or(0) + 1);
        total["first_run_outcome"] = run["outcome"].clone();
        total["total_minutes"] = json!(
            total["total_minutes"].as_f64().unwrap_or(0.0)
                + run["duration_ms"].as_f64().unwrap_or(0.0) / 60_000.0
        );
    }
    Ok(json!({"runs":runs,"tasks":totals}))
}
static SIGNAL: AtomicI32 = AtomicI32::new(0);
extern "C" fn received(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::SeqCst);
}
struct Signals {
    int: libc::sigaction,
    term: libc::sigaction,
}
impl Signals {
    fn install() -> Result<Self> {
        SIGNAL.store(0, Ordering::SeqCst);
        // SAFETY: sigaction structures are initialized; handler only stores an atomic.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = received as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut previous = Self {
                int: std::mem::zeroed(),
                term: std::mem::zeroed(),
            };
            ensure!(
                libc::sigaction(libc::SIGINT, &action, &mut previous.int) == 0,
                "install SIGINT handler"
            );
            if libc::sigaction(libc::SIGTERM, &action, &mut previous.term) != 0 {
                libc::sigaction(libc::SIGINT, &previous.int, std::ptr::null_mut());
                anyhow::bail!("install SIGTERM handler");
            }
            Ok(previous)
        }
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGINT, &self.int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.term, std::ptr::null_mut());
        }
    }
}
fn kill_group(pid: u32, signal: i32) {
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}
fn tee(
    mut source: impl Read + Send + 'static,
    stderr: bool,
    log: Arc<Mutex<File>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match source.read(&mut buf) {
                Ok(0) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    eprintln!("host-check output read: {error}");
                    break;
                }
                Ok(n) => n,
            };
            let bytes = &buf[..n];
            if stderr {
                let _ = std::io::stderr().lock().write_all(bytes);
            } else {
                let _ = std::io::stdout().lock().write_all(bytes);
            }
            if let Ok(mut file) = log.lock()
                && let Err(error) = file.write_all(bytes)
            {
                eprintln!("host-check log write: {error}");
            }
        }
    })
}
fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output_gated()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
/// Returns the checked program's shell-compatible exit code.
#[cfg(target_os = "linux")]
pub fn run(project: &Path, args: Args, caller: &str, trust: &str) -> Result<i32> {
    ensure!(!args.command.is_empty(), "host-check command is required");
    // Validate through public canonical APIs and release their ownership first.
    let mut store = crate::store::SqliteStore::open_read_only(&project.join(".state/state.db"))
        .context("read host-check project tasks")?;
    let binding = store.host_check_binding(&args.task, args.submission.as_deref())?;
    drop(store);
    let cwd =
        fs::canonicalize(args.cwd.unwrap_or(std::env::current_dir()?)).context("host-check cwd")?;
    ensure!(cwd.is_dir(), "host-check cwd must be a directory");
    let log_path = args.log.map(std::path::absolute).transpose()?;
    let log = log_path
        .as_ref()
        .map(|p| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(p)
        })
        .transpose()
        .context("open host-check log")?;
    let _signals = Signals::install()?;
    ingest(project);
    let started = now();
    let pid = std::process::id();
    let run_id = digest(
        format!(
            "{pid}:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )
        .as_bytes(),
    )
    .replace("sha256:", "");
    let argv = serde_json::to_vec(&args.command)?;
    let head = git(&cwd, &["rev-parse", "--verify", "HEAD"]);
    let dirty = git(&cwd, &["rev-parse", "--is-inside-work-tree"])
        .filter(|inside| inside == "true")
        .and_then(|_| git(&cwd, &["status", "--porcelain"]).map(|s| !s.is_empty()));
    let mut row = json!({"run_id":run_id,"project":project.file_name().and_then(|s|s.to_str()),"task":args.task,
        "submission_id":binding["submission_id"],"attempt_id":binding["attempt_id"],
        "submission_reason":binding["submission_reason"],
        "commit_oid":head,"tree_dirty":dirty,"cwd":cwd,"name":args.name.unwrap_or_else(|| Path::new(&args.command[0]).file_name().unwrap_or_default().to_string_lossy().into_owned()),
        "argv_digest":digest(&argv),"argv":args.command,"started_unix_ms":started,"finished_unix_ms":null,"duration_ms":null,
        "exit_code":null,"signal":null,"outcome":"running","log_path":log_path,"log_size":null,"log_digest":null,
        "caller":caller,"trust":trust,"host_load":crate::verification::host_load(),"product_version":env!("CARGO_PKG_VERSION"),"pid":pid,"process_start":process_start(pid)});
    record(project, &row);
    let clock = Instant::now();
    let mut command = Command::new(&args.command[0]);
    command
        .args(&args.command[1..])
        .current_dir(&cwd)
        .process_group(0);
    if log.is_some() {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = match command.spawn_gated() {
        Ok(child) => child,
        Err(error) => {
            row["outcome"] = json!("spawn_failed");
            row["finished_unix_ms"] = json!(now());
            row["duration_ms"] = json!(clock.elapsed().as_millis() as u64);
            if log_path.is_some() {
                row["log_size"] = json!(0);
                row["log_digest"] = json!(digest(b""));
            }
            record(project, &row);
            eprintln!("host-check spawn failed: {error}");
            return Ok(127);
        }
    };
    let threads = log
        .map(|file| {
            let log = Arc::new(Mutex::new(file));
            vec![
                tee(child.stdout.take().unwrap(), false, log.clone()),
                tee(child.stderr.take().unwrap(), true, log),
            ]
        })
        .unwrap_or_default();
    let mut stopped = None;
    let mut exited = None;
    let status = loop {
        let signal = SIGNAL.swap(0, Ordering::SeqCst);
        if signal != 0 {
            kill_group(child.id(), signal);
            if stopped.is_none() {
                row["outcome"] = json!("interrupted");
                stopped = Some(Instant::now());
            }
        }
        if stopped.is_none()
            && args
                .timeout
                .is_some_and(|seconds| clock.elapsed() >= Duration::from_secs(seconds))
        {
            row["outcome"] = json!("timeout");
            kill_group(child.id(), libc::SIGTERM);
            stopped = Some(Instant::now());
        }
        if stopped.is_some_and(|start| start.elapsed() >= Duration::from_secs(2)) {
            kill_group(child.id(), libc::SIGKILL);
        }
        if exited.is_none() {
            match child.try_wait() {
                Ok(status) => exited = status,
                Err(error) => {
                    kill_group(child.id(), libc::SIGKILL);
                    let _ = child.wait();
                    row["outcome"] = json!("abandoned");
                    row["finished_unix_ms"] = json!(now());
                    row["duration_ms"] = json!(clock.elapsed().as_millis() as u64);
                    record(project, &row);
                    anyhow::bail!("wait host-check: {error}");
                }
            }
        }
        // Keep supervising while inherited log pipes drain; descendants must
        // remain subject to the timeout and forwarded signals after leader exit.
        if let Some(status) = exited
            && threads.iter().all(|t| t.is_finished())
        {
            break status;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A leader can exit on TERM before a descendant; finish killing the group
    // after the grace period, including descendants that still own log pipes.
    if let Some(start) = stopped {
        while start.elapsed() < Duration::from_secs(2) {
            let signal = SIGNAL.swap(0, Ordering::SeqCst);
            if signal != 0 {
                kill_group(child.id(), signal);
            }
            std::thread::sleep(
                Duration::from_millis(20)
                    .min(Duration::from_secs(2).saturating_sub(start.elapsed())),
            );
        }
        kill_group(child.id(), libc::SIGKILL);
    }
    for thread in threads {
        let _ = thread.join();
    }
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    if stopped.is_none() {
        row["outcome"] = json!(if status.success() { "pass" } else { "fail" });
    }
    row["exit_code"] = json!(status.code());
    row["signal"] = json!(status.signal());
    row["finished_unix_ms"] = json!(now());
    row["duration_ms"] = json!(clock.elapsed().as_millis() as u64);
    if let Some(path) = log_path {
        let hashed = || -> Result<(u64, String)> {
            let mut file = File::open(path)?;
            let mut hash = Sha256::new();
            let mut size = 0;
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                hash.update(&buf[..n]);
            }
            Ok((size, format!("sha256:{:x}", hash.finalize())))
        };
        match hashed() {
            Ok((size, hash)) => {
                row["log_size"] = json!(size);
                row["log_digest"] = json!(hash);
            }
            Err(error) => eprintln!("host-check log metadata: {error:#}"),
        }
    }
    record(project, &row);
    Ok(code)
}
