//! Durable, allow-listed engine jobs for the bundled Z-ETL helper.
//!
//! The other engines this store once ran (Lorengine, Phod, QREAM, the Q-ROC
//! draft and the Sheets owner) left with their pages; their code is kept in
//! archive/organ-stand-ins-2026-10-30/src/modules.rs.
//!
//! The planner command loop only validates a small job envelope and starts a
//! worker.  Workers own all file copying and subprocess work, so a slow
//! index, ETL scan, or semantic check cannot hold the service loop hostage.
//! Every input is copied below `root`, and every result is evidence below the
//! corresponding job directory.

use crate::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 512 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_MODULE_WORKERS: usize = 4;
const MAX_QUEUED_MODULES: usize = 64;

const ZETL_SHA256: &str = "532bb7c2ee640c3228587d0086e91a918dc21de092e99dc8e10342ef93bdcd4a";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn private_dir(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    fs::create_dir_all(path)?;
    if !fs::metadata(path)?.is_dir() {
        return Err(invalid("Module path is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid(format!(
            "Symlink is not accepted: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(invalid(format!(
                "Symlink path component is not accepted: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Module record has no parent"))?;
    private_dir(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        stamp()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    private_file(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    private_file(path)?;
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    reject_symlink(path)?;
    let bytes = fs::read(path)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid("Module job record is too large"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn safe_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(invalid(format!("Invalid {label}")));
    }
    Ok(())
}

fn safe_input_name(value: &OsStr) -> Result<()> {
    let mut components = Path::new(value).components();
    if matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        Ok(())
    } else {
        Err(invalid("Module input contains an unsafe path component"))
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    total: &mut u64,
    files: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > 12 {
        return Err(invalid("Module input is too deeply nested"));
    }
    reject_symlink(source)?;
    let meta = fs::metadata(source)?;
    if meta.is_file() {
        *total = total
            .checked_add(meta.len())
            .ok_or_else(|| invalid("Module input size overflow"))?;
        if *total > MAX_INPUT_BYTES {
            return Err(invalid("Module input exceeds 256 MiB"));
        }
        *files += 1;
        if *files > 10_000 {
            return Err(invalid("Module input contains too many files"));
        }
        private_dir(
            destination
                .parent()
                .ok_or_else(|| invalid("Module input has no parent"))?,
        )?;
        let mut from = File::open(source)?;
        let mut to = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)?;
        private_file(destination)?;
        std::io::copy(&mut from, &mut to)?;
        to.sync_all()?;
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(invalid(
            "Module input must contain files and directories only",
        ));
    }
    private_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        safe_input_name(&name)?;
        copy_tree(
            &entry.path(),
            &destination.join(&name),
            total,
            files,
            depth + 1,
        )?;
    }
    Ok(())
}

fn copy_input(source: &Path, destination: &Path) -> Result<Value> {
    if !source.is_absolute() {
        return Err(invalid("Module input path must be absolute"));
    }
    reject_symlink_components(source)?;
    if source == destination || source.starts_with(destination) {
        return Err(invalid("Module input cannot be inside the job"));
    }
    let mut total = 0;
    let mut files = 0;
    copy_tree(source, destination, &mut total, &mut files, 0)?;
    let hash = if fs::metadata(destination)?.is_file() {
        Some(hash_file(destination)?)
    } else {
        None
    };
    Ok(
        json!({"path": destination, "source_path": source, "bytes": total, "files": files, "sha256": hash}),
    )
}

fn arg_string(args: &Value, key: &str, max: usize) -> Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("modules-run requires string {key}")))?;
    if value.is_empty() || value.len() > max {
        return Err(invalid(format!("Invalid {key}")));
    }
    Ok(value.to_owned())
}

fn runtime_binary(name: &str) -> PathBuf {
    let mut directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_owned));
    while let Some(path) = directory {
        let bundled = path.join("ui/modules/bin").join(name);
        if binary_available(&bundled) {
            return bundled;
        }
        directory = path.parent().map(Path::to_owned);
    }
    PathBuf::from(format!("ui/modules/bin/{name}"))
}

fn binary_available(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn executable_hash(path: &Path) -> Option<String> {
    binary_available(path)
        .then(|| hash_file(path).ok())
        .flatten()
}

fn verify_binary(path: &Path, expected: &str) -> Result<()> {
    if executable_hash(path).as_deref() != Some(&format!("sha256:{expected}")) {
        return Err(invalid(format!(
            "Allowlisted module binary is missing or changed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn engine_descriptor(
    id: &str,
    name: &str,
    purpose: &str,
    limits: &str,
    binary: &Path,
    expected_sha256: &str,
    state: &str,
    actions: &[&str],
    source: &str,
) -> Value {
    // Inventory is polled by the UI. Hashing the 64–75 MiB helpers on every
    // refresh made a visible tab consume a core; launch workers recheck the
    // digest immediately before spawning. The expected digest remains in the
    // response and `current_sha256` is deliberately omitted until a run.
    let available = binary_available(binary);
    let version = match id {
        "zetl" => "0.1.0 (Avx2 scan backend)",
        _ => "unknown",
    };
    json!({"id": id, "name": name, "version": version, "purpose": purpose, "limits": limits, "binary": binary, "sha256": format!("sha256:{expected_sha256}"), "current_sha256": Value::Null, "state": if available { state } else { "unavailable" }, "actions": actions, "source": source})
}

fn inventory() -> Value {
    let zetl = runtime_binary("z-etl");
    json!({"modules": [
        engine_descriptor("zetl", "Z-ETL", "Inspect, validate, run, resume, or deep-verify a bounded delimited-data pipeline.", "Input and generated output stay under the private job directory; no arbitrary pipeline stages are accepted. Verification only reads an owned manifest and artifact; resume only accepts an owned prior config.", &zetl, ZETL_SHA256, "available", &["inspect", "validate", "run", "verify", "resume"], "DATA-028 r03"),
    ]})
}

fn update(
    map: &Arc<Mutex<BTreeMap<String, Value>>>,
    root: &Path,
    id: &str,
    patch: impl FnOnce(&mut Value),
) -> Result<()> {
    let value = {
        let mut guard = map
            .lock()
            .map_err(|_| invalid("Module job state lock poisoned"))?;
        let record = guard
            .get_mut(id)
            .ok_or_else(|| invalid("Module job not found"))?;
        patch(record);
        record.clone()
    };
    atomic_json(&root.join("jobs").join(format!("{id}.json")), &value)
}

fn owned_artifacts(evidence: &Value, job_dir: &Path) -> Vec<Value> {
    let mut artifacts = Vec::new();
    let mut seen = BTreeMap::new();
    let fields = [
        ("stdout_path", "diagnostic"),
        ("stderr_path", "diagnostic"),
        ("config", "config"),
        ("output", "output"),
        ("manifest", "manifest"),
        ("checkpoint", "checkpoint"),
        ("database", "database"),
        ("original_path", "source"),
        ("candidate_path", "candidate"),
        ("proposal_path", "proposal"),
        ("diff_path", "diff"),
    ];
    for (field, role) in fields {
        let Some(path) = evidence
            .get(field)
            .and_then(Value::as_str)
            .map(PathBuf::from)
        else {
            continue;
        };
        add_owned_artifact(&mut artifacts, &mut seen, &path, role, job_dir);
    }
    if let Some(path) = evidence
        .get("copied_input")
        .and_then(|value| value.get("path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
    {
        add_owned_artifact(&mut artifacts, &mut seen, &path, "input", job_dir);
    }
    artifacts
}

fn add_owned_artifact(
    artifacts: &mut Vec<Value>,
    seen: &mut BTreeMap<PathBuf, ()>,
    path: &Path,
    role: &str,
    job_dir: &Path,
) {
    if !path.is_absolute()
        || !path.starts_with(job_dir)
        || seen.contains_key(path)
        || reject_symlink_components(path).is_err()
    {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
        return;
    }
    let Ok(sha256) = hash_file(path) else { return };
    seen.insert(path.to_owned(), ());
    artifacts.push(json!({
        "path": path,
        "sha256": sha256,
        "size": metadata.len(),
        "role": role,
    }));
}

struct CaptureJob {
    root: PathBuf,
    map: Arc<Mutex<BTreeMap<String, Value>>>,
    id: String,
    cancel: Arc<AtomicBool>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    timeout: Duration,
    evidence: Value,
}

fn capture(
    mut child: Child,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    job: CaptureJob,
) {
    let CaptureJob {
        root,
        map,
        id,
        cancel,
        cancels,
        timeout,
        evidence,
    } = job;
    let stdout_thread = thread::spawn(move || read_output(stdout));
    let stderr_thread = thread::spawn(move || read_output(stderr));
    let started = std::time::Instant::now();
    let mut outcome = "succeeded";
    let mut error: Option<String> = None;
    loop {
        if outcome == "succeeded" && cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            outcome = "cancelled";
            error = Some("Cancellation requested".into());
        }
        if outcome == "succeeded" && started.elapsed() > timeout {
            let _ = child.kill();
            outcome = "timeout";
            error = Some("Module exceeded its timeout".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if outcome == "succeeded" && !status.success() {
                    outcome = "failed";
                    error = Some(format!("Module exited with {status}"));
                }
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                outcome = "failed";
                error = Some(e.to_string());
                break;
            }
        }
    }
    let (out, out_truncated) = stdout_thread
        .join()
        .unwrap_or_else(|_| (String::new(), false));
    let (err, err_truncated) = stderr_thread
        .join()
        .unwrap_or_else(|_| (String::new(), false));
    let mut evidence: Value = evidence;
    evidence["stdout"] = json!(out);
    evidence["stderr"] = json!(err);
    evidence["stdout_truncated"] = json!(out_truncated);
    evidence["stderr_truncated"] = json!(err_truncated);
    if let Some(job_dir) = evidence.get("job_dir").and_then(Value::as_str) {
        let directory = Path::new(job_dir);
        let stdout_path = directory.join("stdout.txt");
        let stderr_path = directory.join("stderr.txt");
        if fs::write(&stdout_path, out.as_bytes()).is_ok() {
            let _ = private_file(&stdout_path);
            evidence["stdout_path"] = json!(stdout_path);
        }
        if fs::write(&stderr_path, err.as_bytes()).is_ok() {
            let _ = private_file(&stderr_path);
            evidence["stderr_path"] = json!(stderr_path);
        }
    }
    if let Some(copied) = evidence.get("copied_input").cloned() {
        evidence["input_receipt"] = copied;
    }
    if let Some(job_dir) = evidence.get("job_dir").and_then(Value::as_str) {
        let artifacts = owned_artifacts(&evidence, Path::new(job_dir));
        evidence["artifacts"] = json!(artifacts);
    }
    let _ = update(&map, &root, &id, |record| {
        record["state"] = json!(outcome);
        record["completed_at"] = json!(now());
        record["evidence"] = evidence.clone();
        if let Some(error) = &error {
            record["error"] = json!(error);
        }
    });
    if let Ok(mut active) = cancels.lock() {
        active.remove(&id);
    }
}

fn read_output(stream: Option<impl Read>) -> (String, bool) {
    let Some(stream) = stream else {
        return (String::new(), false);
    };
    let mut stream = stream;
    let mut bytes = Vec::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    while let Ok(read) = stream.read(&mut buffer) {
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if bytes.len() < MAX_OUTPUT_BYTES as usize {
            let remaining = MAX_OUTPUT_BYTES as usize - bytes.len();
            bytes.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    let truncated = total > MAX_OUTPUT_BYTES;
    (String::from_utf8_lossy(&bytes).into_owned(), truncated)
}

#[allow(clippy::too_many_arguments)]
fn start_process(
    root: &Path,
    map: Arc<Mutex<BTreeMap<String, Value>>>,
    id: &str,
    mut command: Command,
    cancel: Arc<AtomicBool>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    timeout_ms: u64,
    evidence: Value,
) -> Result<()> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let root = root.to_owned();
    let map2 = Arc::clone(&map);
    let id = id.to_owned();
    thread::spawn(move || {
        capture(
            child,
            stdout,
            stderr,
            CaptureJob {
                root,
                map: map2,
                id,
                cancel,
                cancels,
                timeout: Duration::from_millis(timeout_ms),
                evidence,
            },
        )
    });
    Ok(())
}

fn build_and_start(
    root: PathBuf,
    map: Arc<Mutex<BTreeMap<String, Value>>>,
    id: String,
    args: Value,
    cancel: Arc<AtomicBool>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
) -> Result<()> {
    let module = arg_string(&args, "module", 32)?;
    let action = arg_string(&args, "action", 32)?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1, MAX_TIMEOUT_MS);
    update(&map, &root, &id, |record| {
        record["state"] = json!("running");
        record["started_at"] = json!(now());
    })?;
    let job_dir = root.join("jobs").join(&id);
    private_dir(&job_dir)?;
    let expected_binary = match module.as_str() {
        "zetl" => (runtime_binary("z-etl"), ZETL_SHA256),
        _ => return Err(invalid("Module or action is not available in this panel")),
    };
    verify_binary(&expected_binary.0, expected_binary.1)?;
    let mut evidence = json!({"job_dir": job_dir, "module": module, "action": action});
    let mut command;
    match (module.as_str(), action.as_str()) {
        ("zetl", action @ ("inspect" | "validate" | "run")) => {
            let source = PathBuf::from(arg_string(&args, "source_path", 4096)?);
            let input = job_dir.join("input");
            let copied = copy_input(&source, &input)?;
            evidence["copied_input"] = copied;
            command = Command::new(&expected_binary.0);
            if action == "inspect" {
                command.args(["inspect", "--input", input.to_str().unwrap(), "--json"]);
                command.stdin(Stdio::null());
            } else {
                let config = job_dir.join("pipeline.yaml");
                let output = job_dir.join("output.csv");
                let manifest = job_dir.join("manifest.json");
                let config_text = format!(
                    "input:\n  path: {}\n  delimiter: \",\"\n  header: true\noutput:\n  path: {}\n  delimiter: \",\"\n  format: csv\n  typed: false\nmanifest: {}\ncheckpoint:\n  path: {}\n  every: 100000\nexecution:\n  workers: 1\n",
                    input.display(),
                    output.display(),
                    manifest.display(),
                    job_dir.join("checkpoint.json").display()
                );
                fs::write(&config, config_text.as_bytes())?;
                private_file(&config)?;
                if action == "validate" {
                    command.args(["validate", "--config", config.to_str().unwrap(), "--json"]);
                } else {
                    command.args([
                        "run",
                        "--config",
                        config.to_str().unwrap(),
                        "--json",
                        "--quiet",
                    ]);
                }
                evidence["config"] = json!(config);
                evidence["output"] = json!(output);
                evidence["manifest"] = json!(manifest);
                evidence["checkpoint"] = json!(job_dir.join("checkpoint.json"));
                command.stdin(Stdio::null());
            }
        }
        ("zetl", "resume") => {
            let config = PathBuf::from(arg_string(&args, "config_path", 4096)?);
            if !config.is_absolute() {
                return Err(invalid("config_path must be absolute"));
            }
            if !config.starts_with(&root) {
                return Err(invalid("config_path must refer to an owned module job"));
            }
            reject_symlink_components(&config)?;
            if !fs::metadata(&config)?.is_file() {
                return Err(invalid("config_path must be a regular file"));
            }
            command = Command::new(&expected_binary.0);
            command.args([
                "run",
                "--config",
                config.to_str().unwrap(),
                "--resume",
                "--json",
                "--quiet",
            ]);
            command.stdin(Stdio::null());
            evidence["config"] = json!(config);
            evidence["resumed"] = json!(true);
        }
        ("zetl", "verify") => {
            let manifest = PathBuf::from(arg_string(&args, "manifest_path", 4096)?);
            if !manifest.is_absolute() {
                return Err(invalid("manifest_path must be absolute"));
            }
            if !manifest.starts_with(&root) {
                return Err(invalid("manifest_path must refer to an owned module job"));
            }
            reject_symlink_components(&manifest)?;
            if !fs::metadata(&manifest)?.is_file() {
                return Err(invalid("manifest_path must be a regular file"));
            }
            // The verifier reads the manifest and the artifact named by it;
            // those files are immutable copies below this store. Optional
            // input/output paths are likewise constrained to the store so a
            // UI supplied path can never turn verification into a file probe.
            command = Command::new(&expected_binary.0);
            command.args(["verify", "-m", manifest.to_str().unwrap(), "--deep"]);
            if let Some(input) = args.get("input_path").and_then(Value::as_str) {
                let input = PathBuf::from(input);
                if !input.is_absolute() || !input.starts_with(&root) {
                    return Err(invalid("input_path must refer to an owned module job"));
                }
                reject_symlink_components(&input)?;
                command.args(["-i", input.to_str().unwrap()]);
                evidence["input"] = json!(input);
            }
            if let Some(output) = args.get("output_path").and_then(Value::as_str) {
                let output = PathBuf::from(output);
                if !output.is_absolute() || !output.starts_with(&root) {
                    return Err(invalid("output_path must refer to an owned module job"));
                }
                reject_symlink_components(&output)?;
                command.args(["-o", output.to_str().unwrap()]);
                evidence["output"] = json!(output);
            }
            command.stdin(Stdio::null());
            evidence["manifest"] = json!(manifest);
            evidence["verification"] = json!("deep");
        }
        _ => return Err(invalid("Module or action is not available in this panel")),
    }
    start_process(
        &root, map, &id, command, cancel, cancels, timeout_ms, evidence,
    )
}

struct QueuedModule {
    root: PathBuf,
    map: Arc<Mutex<BTreeMap<String, Value>>>,
    id: String,
    args: Value,
    cancel: Arc<AtomicBool>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
}

struct ModuleScheduler {
    queue: Mutex<VecDeque<QueuedModule>>,
    wake: Condvar,
    stop: AtomicBool,
}

fn wait_for_terminal(map: &Arc<Mutex<BTreeMap<String, Value>>>, id: &str) {
    loop {
        let terminal = map
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(id).cloned())
            .is_some_and(|record| {
                matches!(
                    record.get("state").and_then(Value::as_str),
                    Some("succeeded" | "failed" | "cancelled" | "timeout" | "interrupted")
                )
            });
        if terminal {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn metadata_job(record: &Value) -> Value {
    let mut result = record.clone();
    if let Some(evidence) = result.get_mut("evidence").and_then(Value::as_object_mut) {
        evidence.remove("stdout");
        evidence.remove("stderr");
        // Runtime receipts can contain a full sessions/approvals/history
        // response.  The list endpoint is a paged metadata view; callers
        // fetch the selected job through modules-status before rendering the
        // result.  Keep the small project/config/manifest/checkpoint and
        // artifact summaries used by the domain selectors.
        evidence.remove("result");
    }
    result
}

fn scheduler_worker(scheduler: Arc<ModuleScheduler>) {
    loop {
        let job = {
            let mut queue = match scheduler.queue.lock() {
                Ok(queue) => queue,
                Err(_) => return,
            };
            loop {
                if let Some(job) = queue.pop_front() {
                    break Some(job);
                }
                if scheduler.stop.load(Ordering::Acquire) {
                    break None;
                }
                queue = match scheduler.wake.wait(queue) {
                    Ok(queue) => queue,
                    Err(_) => return,
                };
            }
        };
        let Some(job) = job else { return };
        if job.cancel.load(Ordering::Acquire) {
            let _ = update(&job.map, &job.root, &job.id, |record| {
                record["state"] = json!("cancelled");
                record["completed_at"] = json!(now());
                record["error"] = json!("Cancellation requested before the worker started");
            });
            if let Ok(mut active) = job.cancels.lock() {
                active.remove(&job.id);
            }
            continue;
        }
        let result = build_and_start(
            job.root.clone(),
            Arc::clone(&job.map),
            job.id.clone(),
            job.args,
            Arc::clone(&job.cancel),
            Arc::clone(&job.cancels),
        );
        if let Err(error) = result {
            let _ = update(&job.map, &job.root, &job.id, |record| {
                record["state"] = json!("failed");
                record["completed_at"] = json!(now());
                record["error"] = json!(error.to_string());
            });
            if let Ok(mut active) = job.cancels.lock() {
                active.remove(&job.id);
            }
        } else {
            // Process-backed jobs hand their child to a bounded capture
            // thread. Keep this scheduler slot occupied until that receipt is
            // durable, so four workers really means four active jobs.
            wait_for_terminal(&job.map, &job.id);
        }
    }
}

/// Durable OVRIN module runner.  The parent service should call `command` on
/// its normal loop; subprocesses and input copies are delegated to workers.
pub struct ModulesStore {
    root: PathBuf,
    jobs: Arc<Mutex<BTreeMap<String, Value>>>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    scheduler: Arc<ModuleScheduler>,
}

impl ModulesStore {
    pub fn open(root: &Path) -> Result<Self> {
        private_dir(root)?;
        private_dir(&root.join("jobs"))?;
        let mut jobs = BTreeMap::new();
        for entry in fs::read_dir(root.join("jobs"))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let mut record = read_json(&path)?;
            if matches!(
                record.get("state").and_then(Value::as_str),
                Some("queued" | "running")
            ) {
                record["state"] = json!("interrupted");
                record["completed_at"] = json!(now());
                record["error"] = json!("Module receiver restarted while the job was running");
                let _ = atomic_json(&path, &record);
            }
            if let Some(id) = record.get("id").and_then(Value::as_str) {
                jobs.insert(id.to_owned(), record);
            }
        }
        let jobs = Arc::new(Mutex::new(jobs));
        let cancels = Arc::new(Mutex::new(BTreeMap::new()));
        let scheduler = Arc::new(ModuleScheduler {
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        for _ in 0..MAX_MODULE_WORKERS {
            let scheduler_for_worker = Arc::clone(&scheduler);
            thread::spawn(move || scheduler_worker(scheduler_for_worker));
        }
        Ok(Self {
            root: root.to_owned(),
            jobs,
            cancels,
            scheduler,
        })
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "modules-list" => {
                let mut result = inventory();
                let offset = args
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(10_000) as usize;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(100)
                    .clamp(1, 100) as usize;
                let include_evidence = args
                    .get("include_evidence")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let jobs = self
                    .jobs
                    .lock()
                    .map_err(|_| invalid("Module job state lock poisoned"))?;
                let mut records = jobs.values().cloned().collect::<Vec<_>>();
                records.sort_by(|left, right| {
                    right
                        .get("created_at")
                        .and_then(Value::as_str)
                        .cmp(&left.get("created_at").and_then(Value::as_str))
                });
                let total = records.len();
                let records = records
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|record| {
                        if include_evidence {
                            record
                        } else {
                            metadata_job(&record)
                        }
                    })
                    .collect::<Vec<_>>();
                result["jobs"] = json!(records);
                result["total_jobs"] = json!(total);
                result["offset"] = json!(offset);
                result["limit"] = json!(limit);
                result["job_limit"] = json!({"active": MAX_MODULE_WORKERS, "queued": MAX_QUEUED_MODULES, "returned": limit, "metadata_only": !include_evidence});
                Ok(result)
            }
            "modules-status" => {
                let id = arg_string(args, "id", 160)?;
                let jobs = self
                    .jobs
                    .lock()
                    .map_err(|_| invalid("Module job state lock poisoned"))?;
                jobs.get(&id)
                    .cloned()
                    .ok_or_else(|| invalid("Module job not found"))
            }
            "modules-cancel" => {
                let id = arg_string(args, "id", 160)?;
                let state = self
                    .jobs
                    .lock()
                    .map_err(|_| invalid("Module job state lock poisoned"))?
                    .get(&id)
                    .and_then(|record| record.get("state").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_owned();
                if !matches!(state.as_str(), "queued" | "running") {
                    return Err(invalid("Module job is no longer running"));
                }
                let flag = self
                    .cancels
                    .lock()
                    .map_err(|_| invalid("Module cancellation lock poisoned"))?
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| invalid("Module job is not running"))?;
                flag.store(true, Ordering::Release);
                update(&self.jobs, &self.root, &id, |record| {
                    record["cancel_requested"] = json!(true);
                })?;
                Ok(json!({"id": id, "cancel_requested": true}))
            }
            "modules-run" => {
                let module = arg_string(args, "module", 32)?;
                let action = arg_string(args, "action", 32)?;
                let id = format!("module-{}-{}", std::process::id(), stamp());
                safe_component(&id, "job id")?;
                if self
                    .jobs
                    .lock()
                    .map_err(|_| invalid("Module job state lock poisoned"))?
                    .contains_key(&id)
                    || self.root.join("jobs").join(format!("{id}.json")).exists()
                {
                    return Err(invalid(
                        "Generated module job id collides with an existing job",
                    ));
                }
                let record = json!({"id":id,"module":module,"action":action,"state":"queued","created_at":now(),"cancel_requested":false,"evidence":{"job_dir":self.root.join("jobs").join(&id)}});
                atomic_json(&self.root.join("jobs").join(format!("{id}.json")), &record)?;
                self.jobs
                    .lock()
                    .map_err(|_| invalid("Module job state lock poisoned"))?
                    .insert(id.clone(), record.clone());
                let flag = Arc::new(AtomicBool::new(false));
                self.cancels
                    .lock()
                    .map_err(|_| invalid("Module cancellation lock poisoned"))?
                    .insert(id.clone(), Arc::clone(&flag));
                let mut queue = self
                    .scheduler
                    .queue
                    .lock()
                    .map_err(|_| invalid("Module scheduler lock poisoned"))?;
                if queue.len() >= MAX_QUEUED_MODULES {
                    drop(queue);
                    if let Ok(mut map) = self.cancels.lock() {
                        map.remove(&id);
                    }
                    let _ = fs::remove_file(self.root.join("jobs").join(format!("{id}.json")));
                    if let Ok(mut jobs) = self.jobs.lock() {
                        jobs.remove(&id);
                    }
                    return Err(invalid(
                        "Module busy: active workers and the 64-job queue are full",
                    ));
                }
                queue.push_back(QueuedModule {
                    root: self.root.clone(),
                    map: Arc::clone(&self.jobs),
                    id: id.clone(),
                    args: args.clone(),
                    cancel: flag,
                    cancels: Arc::clone(&self.cancels),
                });
                drop(queue);
                self.scheduler.wake.notify_one();
                Ok(json!({"accepted":true,"id":id,"state":"queued"}))
            }
            _ => Err(invalid("Unknown module command")),
        }
    }
}

impl Drop for ModulesStore {
    fn drop(&mut self) {
        self.scheduler.stop.store(true, Ordering::Release);
        self.scheduler.wake.notify_all();
        if let Ok(active) = self.cancels.lock() {
            for flag in active.values() {
                flag.store(true, Ordering::Release);
            }
        }
    }
}
