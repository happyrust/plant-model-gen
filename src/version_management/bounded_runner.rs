use crate::version_management::hashing::sha256_file;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const KILL_GRACE_MS: u64 = 1500;

#[derive(Clone, Debug)]
pub struct BoundedCommandRunRequest {
    pub run_id: String,
    pub kind: String,
    pub state_dir: PathBuf,
    pub executable: Option<PathBuf>,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub metrics_path: Option<PathBuf>,
    pub timeout_secs: u64,
    pub stale_heartbeat_secs: Option<u64>,
    pub source_db_file: Option<PathBuf>,
    pub expected_source_db_sha256: Option<String>,
    pub poll_interval_ms: u64,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundedRunStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

impl BoundedRunStatus {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundedRunMetricsSnapshot {
    pub path: PathBuf,
    pub exists: bool,
    pub bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub updated_at: Option<String>,
    pub success: Option<bool>,
    pub stage: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundedRunRecord {
    pub run_id: String,
    pub kind: String,
    pub status: BoundedRunStatus,
    pub pid: Option<u32>,
    pub executable: PathBuf,
    pub argv: Vec<String>,
    #[serde(default)]
    pub child_argv: Vec<String>,
    #[serde(default)]
    pub argv_included_executable: bool,
    pub cwd: PathBuf,
    pub env_keys: Vec<String>,
    pub state_path: PathBuf,
    pub cancel_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub metrics_path: Option<PathBuf>,
    pub timeout_secs: u64,
    pub stale_heartbeat_secs: Option<u64>,
    pub submitted_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub elapsed_ms: u128,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub cancel_requested_at: Option<String>,
    pub cancel_reason: Option<String>,
    pub timeout_at: Option<String>,
    pub stale_heartbeat_at: Option<String>,
    pub source_db_file: Option<PathBuf>,
    pub source_db_sha256_before: Option<String>,
    pub source_db_sha256_after: Option<String>,
    pub source_db_hash_unchanged: Option<bool>,
    pub metrics: Option<BoundedRunMetricsSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundedRunCancelResponse {
    pub run_id: String,
    pub cancel_path: PathBuf,
    pub previous_status: Option<BoundedRunStatus>,
    pub pid: Option<u32>,
    pub kill_attempted: bool,
}

pub fn run_state_dir(base: &Path, run_id: &str) -> PathBuf {
    base.join(run_id)
}

pub fn run_state_path(base: &Path, run_id: &str) -> PathBuf {
    run_state_dir(base, run_id).join("run.json")
}

pub fn run_cancel_path(base: &Path, run_id: &str) -> PathBuf {
    run_state_dir(base, run_id).join("cancel.requested.json")
}

pub fn run_bounded_command(request: BoundedCommandRunRequest) -> anyhow::Result<BoundedRunRecord> {
    validate_run_id(&request.run_id)?;
    if request.timeout_secs == 0 {
        bail!("timeout_secs must be greater than 0 for bounded command runs");
    }
    if request.argv.is_empty() {
        bail!("argv must not be empty; pass an explicit argv JSON array");
    }
    if !request.cwd.is_dir() {
        bail!("cwd is not a directory: {}", request.cwd.display());
    }
    if request.poll_interval_ms == 0 {
        bail!("poll_interval_ms must be greater than 0");
    }

    let run_dir = run_state_dir(&request.state_dir, &request.run_id);
    if run_dir.exists() && !request.force {
        bail!(
            "run directory already exists for '{}'; pass --force to overwrite: {}",
            request.run_id,
            run_dir.display()
        );
    }
    if request.force && run_dir.exists() {
        fs::remove_dir_all(&run_dir).with_context(|| {
            format!(
                "remove existing run directory failed: {}",
                run_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("create run directory failed: {}", run_dir.display()))?;

    let state_path = run_state_path(&request.state_dir, &request.run_id);
    let cancel_path = run_cancel_path(&request.state_dir, &request.run_id);
    let stdout_path = request
        .stdout_path
        .clone()
        .unwrap_or_else(|| run_dir.join("stdout.log"));
    let stderr_path = request
        .stderr_path
        .clone()
        .unwrap_or_else(|| run_dir.join("stderr.log"));
    ensure_parent_dir(&stdout_path)?;
    ensure_parent_dir(&stderr_path)?;

    let executable = request.executable.clone().unwrap_or_else(|| {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("aios-database"))
    });
    if !executable.is_file() {
        bail!(
            "executable is missing or not a file: {}",
            executable.display()
        );
    }
    let (child_argv, argv_included_executable) = normalize_child_argv(&request.argv, &executable);

    let source_db_sha256_before = match &request.source_db_file {
        Some(path) => {
            if !path.is_file() {
                bail!(
                    "source_db_file is missing or not a file: {}",
                    path.display()
                );
            }
            let hash = sha256_file(path)?;
            if let Some(expected) = &request.expected_source_db_sha256 {
                if !hash.eq_ignore_ascii_case(expected.trim()) {
                    bail!(
                        "source DB hash mismatch before run: expected {}, got {} for {}",
                        expected,
                        hash,
                        path.display()
                    );
                }
            }
            Some(hash)
        }
        None => None,
    };

    let now = now_rfc3339();
    let mut record = BoundedRunRecord {
        run_id: request.run_id.clone(),
        kind: request.kind.clone(),
        status: BoundedRunStatus::Running,
        pid: None,
        executable: executable.clone(),
        argv: request.argv.clone(),
        child_argv: child_argv.clone(),
        argv_included_executable,
        cwd: request.cwd.clone(),
        env_keys: request.env.keys().cloned().collect(),
        state_path: state_path.clone(),
        cancel_path: cancel_path.clone(),
        stdout_path: stdout_path.clone(),
        stderr_path: stderr_path.clone(),
        metrics_path: request.metrics_path.clone(),
        timeout_secs: request.timeout_secs,
        stale_heartbeat_secs: request.stale_heartbeat_secs,
        submitted_at: now.clone(),
        started_at: None,
        updated_at: now,
        finished_at: None,
        elapsed_ms: 0,
        exit_code: None,
        error: None,
        cancel_requested_at: None,
        cancel_reason: None,
        timeout_at: None,
        stale_heartbeat_at: None,
        source_db_file: request.source_db_file.clone(),
        source_db_sha256_before,
        source_db_sha256_after: None,
        source_db_hash_unchanged: None,
        metrics: request
            .metrics_path
            .as_ref()
            .map(|path| snapshot_metrics(path)),
    };
    write_record_atomic(&record)?;

    let mut child = match spawn_child(
        &request,
        &executable,
        &child_argv,
        &stdout_path,
        &stderr_path,
    ) {
        Ok(child) => child,
        Err(error) => {
            finalize_source_hash(&mut record);
            record.status = BoundedRunStatus::Failed;
            record.error = Some(error.to_string());
            record.updated_at = now_rfc3339();
            record.finished_at = Some(record.updated_at.clone());
            record.metrics = request
                .metrics_path
                .as_ref()
                .map(|path| snapshot_metrics(path));
            write_record_atomic(&record)?;
            return Err(error);
        }
    };
    record.pid = Some(child.id());
    record.started_at = Some(now_rfc3339());
    record.updated_at = now_rfc3339();
    record.metrics = request
        .metrics_path
        .as_ref()
        .map(|path| snapshot_metrics(path));
    write_record_atomic(&record)?;

    let started = Instant::now();
    let timeout = Duration::from_secs(request.timeout_secs);
    let poll = Duration::from_millis(request.poll_interval_ms);
    let mut terminal_reason: Option<BoundedRunStatus> = None;

    loop {
        if cancel_path.exists() {
            let reason = read_cancel_reason(&cancel_path);
            record.cancel_requested_at = Some(now_rfc3339());
            record.cancel_reason = reason;
            kill_child_process_tree(&mut child);
            terminal_reason = Some(BoundedRunStatus::Cancelled);
            break;
        }

        if started.elapsed() >= timeout {
            record.timeout_at = Some(now_rfc3339());
            record.error = Some(format!(
                "command timed out after {} seconds",
                request.timeout_secs
            ));
            kill_child_process_tree(&mut child);
            terminal_reason = Some(BoundedRunStatus::TimedOut);
            break;
        }

        if let Some(stale_secs) = request.stale_heartbeat_secs.filter(|value| *value > 0) {
            if let Some(metrics_path) = &request.metrics_path {
                if metrics_is_stale(metrics_path, stale_secs) {
                    record.stale_heartbeat_at = Some(now_rfc3339());
                    record.error = Some(format!(
                        "metrics heartbeat stale for more than {} seconds",
                        stale_secs
                    ));
                    kill_child_process_tree(&mut child);
                    terminal_reason = Some(BoundedRunStatus::TimedOut);
                    break;
                }
            }
        }

        match child.try_wait().context("poll child process failed")? {
            Some(status) => {
                record.exit_code = status.code();
                if cancel_path.exists() {
                    record.cancel_requested_at = Some(now_rfc3339());
                    record.cancel_reason = read_cancel_reason(&cancel_path);
                    terminal_reason = Some(BoundedRunStatus::Cancelled);
                } else if status.success() {
                    terminal_reason = Some(BoundedRunStatus::Succeeded);
                } else {
                    record.error = Some(format!("command exited with status {}", status));
                    terminal_reason = Some(BoundedRunStatus::Failed);
                }
                break;
            }
            None => {
                record.elapsed_ms = started.elapsed().as_millis();
                record.updated_at = now_rfc3339();
                record.metrics = request
                    .metrics_path
                    .as_ref()
                    .map(|path| snapshot_metrics(path));
                write_record_atomic(&record)?;
                std::thread::sleep(poll);
            }
        }
    }

    if record.exit_code.is_none() {
        if let Ok(Some(status)) = child.try_wait() {
            record.exit_code = status.code();
        }
    }
    finalize_source_hash(&mut record);
    record.status = terminal_reason.unwrap_or(BoundedRunStatus::Failed);
    record.elapsed_ms = started.elapsed().as_millis();
    record.updated_at = now_rfc3339();
    record.finished_at = Some(record.updated_at.clone());
    record.metrics = request
        .metrics_path
        .as_ref()
        .map(|path| snapshot_metrics(path));
    write_record_atomic(&record)?;
    Ok(record)
}

pub fn read_bounded_run_status(state_dir: &Path, run_id: &str) -> anyhow::Result<BoundedRunRecord> {
    validate_run_id(run_id)?;
    let path = run_state_path(state_dir, run_id);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("read run status failed: {}", path.display()))?;
    let mut record: BoundedRunRecord = serde_json::from_str(&content)
        .with_context(|| format!("parse run status JSON failed: {}", path.display()))?;
    if let Some(path) = &record.metrics_path {
        record.metrics = Some(snapshot_metrics(path));
    }
    Ok(record)
}

pub fn request_bounded_run_cancel(
    state_dir: &Path,
    run_id: &str,
    reason: Option<String>,
) -> anyhow::Result<BoundedRunCancelResponse> {
    validate_run_id(run_id)?;
    let cancel_path = run_cancel_path(state_dir, run_id);
    ensure_parent_dir(&cancel_path)?;
    let requested_at = now_rfc3339();
    let payload = serde_json::json!({
        "run_id": run_id,
        "requested_at": requested_at,
        "reason": reason.clone().unwrap_or_else(|| "cancel requested".to_string()),
    });
    fs::write(&cancel_path, serde_json::to_vec_pretty(&payload)?)
        .with_context(|| format!("write cancel request failed: {}", cancel_path.display()))?;

    let status = read_bounded_run_status(state_dir, run_id).ok();
    let mut kill_attempted = false;
    if let Some(record) = &status {
        if !record.status.is_terminal() {
            if let Some(pid) = record.pid {
                kill_process_tree_by_pid(pid);
                kill_attempted = true;
            }
        }
    }

    Ok(BoundedRunCancelResponse {
        run_id: run_id.to_string(),
        cancel_path,
        previous_status: status.as_ref().map(|record| record.status.clone()),
        pid: status.and_then(|record| record.pid),
        kill_attempted,
    })
}

pub fn parse_argv_json(value: &str) -> anyhow::Result<Vec<String>> {
    let argv: Vec<String> =
        serde_json::from_str(value).context("argv-json must be a JSON string array")?;
    if argv.iter().any(|item| item.trim().is_empty()) {
        bail!("argv-json must not contain empty arguments");
    }
    Ok(argv)
}

pub fn parse_env_assignments<I>(values: Option<I>) -> anyhow::Result<BTreeMap<String, String>>
where
    I: IntoIterator<Item = String>,
{
    let mut env = BTreeMap::new();
    let Some(values) = values else {
        return Ok(env);
    };
    for value in values {
        let Some((key, val)) = value.split_once('=') else {
            bail!("env assignment must use KEY=VALUE form: {}", value);
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("env assignment key must not be empty");
        }
        env.insert(key.to_string(), val.to_string());
    }
    Ok(env)
}

fn normalize_child_argv(argv: &[String], executable: &Path) -> (Vec<String>, bool) {
    let Some(first) = argv.first() else {
        return (Vec::new(), false);
    };
    if first_arg_matches_executable(first, executable) {
        (argv.iter().skip(1).cloned().collect(), true)
    } else {
        (argv.to_vec(), false)
    }
}

fn first_arg_matches_executable(first: &str, executable: &Path) -> bool {
    let first = first.trim();
    if first.is_empty() {
        return false;
    }
    let first_path = Path::new(first);
    let first_name = first_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(first)
        .to_ascii_lowercase();
    let first_stem = Path::new(&first_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(first_name.as_str())
        .to_ascii_lowercase();

    let exe_name = executable
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let exe_stem = executable
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    (!exe_name.is_empty() && first_name == exe_name)
        || (!exe_stem.is_empty() && first_stem == exe_stem)
}

fn spawn_child(
    request: &BoundedCommandRunRequest,
    executable: &Path,
    child_argv: &[String],
    stdout_path: &Path,
    stderr_path: &Path,
) -> anyhow::Result<Child> {
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stdout_path)
        .with_context(|| format!("open stdout log failed: {}", stdout_path.display()))?;
    let stderr = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stderr_path)
        .with_context(|| format!("open stderr log failed: {}", stderr_path.display()))?;
    let mut command = Command::new(executable);
    command
        .args(child_argv)
        .envs(&request.env)
        .current_dir(&request.cwd)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_process_group(&mut command);
    command
        .spawn()
        .with_context(|| format!("spawn command failed: {}", executable.display()))
}

fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
}

fn kill_child_process_tree(child: &mut Child) {
    kill_process_tree_by_pid(child.id());
    let _ = child.wait();
}

fn kill_process_tree_by_pid(pid: u32) {
    #[cfg(unix)]
    {
        if !killpg_group(pid, libc::SIGTERM) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
        std::thread::sleep(Duration::from_millis(KILL_GRACE_MS));
        if !killpg_group(pid, libc::SIGKILL) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .output();
        std::thread::sleep(Duration::from_millis(KILL_GRACE_MS));
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

#[cfg(unix)]
fn killpg_group(pid: u32, sig: libc::c_int) -> bool {
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid <= 0 {
        return false;
    }
    unsafe { libc::killpg(pgid, sig) == 0 }
}

fn write_record_atomic(record: &BoundedRunRecord) -> anyhow::Result<()> {
    ensure_parent_dir(&record.state_path)?;
    let tmp = record
        .state_path
        .with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(record)?)
        .with_context(|| format!("write temporary run status failed: {}", tmp.display()))?;
    fs::rename(&tmp, &record.state_path)
        .with_context(|| format!("replace run status failed: {}", record.state_path.display()))?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory failed: {}", parent.display()))?;
    }
    Ok(())
}

fn finalize_source_hash(record: &mut BoundedRunRecord) {
    let Some(path) = &record.source_db_file else {
        return;
    };
    match sha256_file(path) {
        Ok(hash) => {
            record.source_db_hash_unchanged = record
                .source_db_sha256_before
                .as_ref()
                .map(|before| before.eq_ignore_ascii_case(&hash));
            record.source_db_sha256_after = Some(hash);
        }
        Err(err) => {
            record.error = Some(match &record.error {
                Some(existing) => format!("{existing}; source hash after run failed: {err}"),
                None => format!("source hash after run failed: {err}"),
            });
        }
    }
}

fn snapshot_metrics(path: &Path) -> BoundedRunMetricsSnapshot {
    let metadata = fs::metadata(path).ok();
    let modified_at = metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .map(system_time_to_rfc3339);
    let value = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok());
    BoundedRunMetricsSnapshot {
        path: path.to_path_buf(),
        exists: metadata.is_some(),
        bytes: metadata.as_ref().map(|meta| meta.len()),
        modified_at,
        updated_at: find_string_key(&value, "updated_at"),
        success: find_bool_key(&value, "success"),
        stage: find_string_key(&value, "stage"),
    }
}

fn metrics_is_stale(path: &Path, stale_secs: u64) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(elapsed) = modified.elapsed() else {
        return false;
    };
    elapsed >= Duration::from_secs(stale_secs)
}

fn find_string_key(value: &Option<Value>, key: &str) -> Option<String> {
    fn walk(value: &Value, key: &str) -> Option<String> {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key).and_then(Value::as_str) {
                    return Some(found.to_string());
                }
                map.values().find_map(|value| walk(value, key))
            }
            Value::Array(values) => values.iter().find_map(|value| walk(value, key)),
            _ => None,
        }
    }
    value.as_ref().and_then(|value| walk(value, key))
}

fn find_bool_key(value: &Option<Value>, key: &str) -> Option<bool> {
    fn walk(value: &Value, key: &str) -> Option<bool> {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key).and_then(Value::as_bool) {
                    return Some(found);
                }
                map.values().find_map(|value| walk(value, key))
            }
            Value::Array(values) => values.iter().find_map(|value| walk(value, key)),
            _ => None,
        }
    }
    value.as_ref().and_then(|value| walk(value, key))
}

fn read_cancel_reason(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&content)
        .ok()
        .and_then(|value| {
            value
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn validate_run_id(run_id: &str) -> anyhow::Result<()> {
    let trimmed = run_id.trim();
    if trimmed.is_empty() {
        bail!("run_id must not be empty");
    }
    if trimmed.len() > 128 {
        bail!("run_id must be <= 128 characters");
    }
    if trimmed.contains("..")
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!(
            "run_id must be path-safe ASCII using only letters, numbers, dash, underscore, or dot"
        );
    }
    Ok(())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Utc> = time.into();
    datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
