//! Runs: one History record per sealed task of a campaign folder.
//!
//! The campaign folder is the record and QREAM's campaign reader
//! (`qream-record`) reads it: Soliton asks it which campaigns exist, what each
//! task is and which word each task stands at, and keeps only the
//! presentation. Soliton holds no rule of its own for when a task counts as
//! accepted; a task the reader gives no word for shows no result. The facts the
//! reader does not carry for a task (a seal outside its waves, the report's
//! changed files, the first recorded action) are read from the task's own seal
//! and report, never inferred.
//!
//! Records are derived only when the Runs collection is listed, then cached
//! under the history folder (`runs/<id>.json`, and per campaign under
//! `runs/campaigns/`), rewritten only when a record changed; a campaign whose
//! folder is unchanged since it was cached is not asked of the reader again. Nothing here runs at launch, on a timer or for another kind.

use crate::{Error, Result};
use chrono::{DateTime, FixedOffset};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

/// Largest seal, report or controller file read for one record.
const MAX_FILE: u64 = 16 * 1024 * 1024;
/// Largest reader reply accepted (a campaign's reply carries its documents'
/// text; the largest today is about 90 MB).
const MAX_REPLY: usize = 256 * 1024 * 1024;
/// Reader calls made at once when campaigns are asked again.
const READERS: usize = 4;
/// Changed files named in one record before the rest are counted.
const MAX_FILES_NAMED: usize = 12;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// Where the reader and the campaign folders are. Today's settings:
/// `SOLITON_RUN_READER` (default `qream-record` on PATH) and
/// `SOLITON_RUNS_ROOT` (default `~/06_memory/runs`).
#[derive(Debug, Clone)]
pub struct Config {
    pub reader: PathBuf,
    pub root: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let reader = std::env::var_os("SOLITON_RUN_READER")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("qream-record"));
        let root = std::env::var_os("SOLITON_RUNS_ROOT")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join("06_memory/runs")
            });
        Self { reader, root }
    }
}

/// The Runs collection for the History list: derived now, cached under the
/// history folder. A reader that cannot be reached gives an empty collection.
pub fn records(history_root: &Path) -> Vec<Value> {
    records_with(&Config::from_env(), history_root).unwrap_or_default()
}

/// The cached record of one run, for its detail. Reads the cache only.
pub fn detail(history_root: &Path, id: &str) -> Option<Value> {
    if !valid_id(id) {
        return None;
    }
    let bytes = read_limited(&history_root.join("runs").join(format!("{id}.json"))).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(|v| v["id"] == id && v["kind"] == "run")
}

pub fn records_with(config: &Config, history_root: &Path) -> Result<Vec<Value>> {
    let cache = history_root.join("runs");
    let campaigns = cache.join("campaigns");
    private_dir(&cache)?;
    private_dir(&campaigns)?;
    let root = config.root.to_string_lossy().into_owned();
    let found = reader(config, &["discover", &root, "--json"])?;
    let mut work = Vec::new();
    for entry in found.as_array().into_iter().flatten() {
        if entry["kind"] != "campaign" {
            continue;
        }
        let Some(path) = entry["path"].as_str().map(PathBuf::from) else {
            continue;
        };
        if seal_files(&path).is_empty() {
            continue;
        }
        let project = entry["project"].as_str().unwrap_or_default().to_owned();
        work.push((path, project));
    }
    // A campaign's records are asked of the reader again only when its
    // folder changed since they were cached.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..READERS.min(work.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((path, project)) = work.get(i) else {
                        break;
                    };
                    let print = fingerprint(path);
                    let file = campaigns.join(format!("{}.json", &run_id(path, "")[4..]));
                    let cached = read_json(&file)
                        .filter(|c| c["fingerprint"] == print && c["folder"] == json!(path));
                    let records = match cached {
                        Some(c) => c["records"].as_array().cloned().unwrap_or_default(),
                        // A campaign the reader refuses gives no records; the others still list.
                        None => match campaign(config, path, project) {
                            Ok(records) => {
                                let _ = atomic_json(
                                    &file,
                                    &json!({"folder":path,"fingerprint":print,"records":records}),
                                );
                                records
                            }
                            Err(_) => Vec::new(),
                        },
                    };
                    results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(records);
                }
            });
        }
    });
    let mut out = results.into_inner().unwrap_or_else(|e| e.into_inner());
    out.sort_by(|a, b| b["captured_at"].as_str().cmp(&a["captured_at"].as_str()));
    for record in &out {
        let id = record["id"].as_str().unwrap_or_default();
        let path = cache.join(format!("{id}.json"));
        let unchanged = read_json(&path).is_some_and(|cached| &cached == record);
        if !unchanged {
            atomic_json(&path, record)?;
        }
    }
    Ok(out)
}

/// What a campaign folder looks like on the surface: the names, sizes and
/// times of its seals, its reports, reviews and controller files, and its
/// wave folders. Any change there asks the reader again.
fn fingerprint(folder: &Path) -> String {
    let mut digest = Sha256::new();
    let mut stamp = |path: &Path| {
        if let Ok(meta) = fs::symlink_metadata(path) {
            let time = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            digest.update(format!("{}\0{}\0{time}\n", path.display(), meta.len()));
        }
    };
    stamp(folder);
    for (_, seal) in seal_files(folder) {
        stamp(&seal);
    }
    for sub in ["reports", "reviews", "controller", "state/replay"] {
        let dir = folder.join(sub);
        stamp(&dir);
        let mut names: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        names.sort();
        for name in names.iter().take(4096) {
            stamp(name);
        }
    }
    let mut waves: Vec<PathBuf> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| file_name(p).starts_with("wave"))
        .collect();
    waves.sort();
    for wave in &waves {
        stamp(wave);
    }
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The records of one campaign folder, one per seal.
pub fn campaign(config: &Config, folder: &Path, project: &str) -> Result<Vec<Value>> {
    let shown = reader(config, &["show", &folder.to_string_lossy(), "--json"])?;
    let record = &shown["record"];
    let words = &shown["state"]["tasks"];
    let mut known = std::collections::BTreeMap::new();
    for wave in record["waves"].as_array().into_iter().flatten() {
        for task in wave["tasks"].as_array().into_iter().flatten() {
            if let Some(id) = task["id"].as_str() {
                known.insert(id.to_owned(), task.clone());
            }
        }
    }
    let reports: Vec<PathBuf> = record["reports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect();
    let campaign_name = record["name"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| file_name(folder));
    let project = if project.is_empty() {
        folder.parent().map(file_name).unwrap_or_default()
    } else {
        project.to_owned()
    };
    let mut out = Vec::new();
    for (task_id, seal_path) in seal_files(folder) {
        let task = known.get(&task_id).cloned().unwrap_or(Value::Null);
        let seal = if task["seal"].is_object() {
            task["seal"].clone()
        } else {
            read_json(&seal_path).unwrap_or(Value::Null)
        };
        if !seal.is_object() {
            continue;
        }
        let report = task["reports"]
            .as_array()
            .and_then(|r| r.first())
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .or_else(|| task_report(&reports, &task_id));
        let report_text = report
            .as_deref()
            .and_then(|p| read_limited(p).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let directive = task["directive"]["path"].as_str().map(PathBuf::from);
        let title = task["title"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| heading(&report_text))
            .unwrap_or_else(|| task_id.clone());
        let word = words[&task_id].as_str().unwrap_or_default().to_owned();
        out.push(build(Facts {
            folder,
            project: &project,
            campaign: &campaign_name,
            task: &task_id,
            title: &title,
            seal: &seal,
            seal_path: &seal_path,
            report: report.as_deref(),
            report_text: &report_text,
            directive: directive.as_deref(),
            result: &word,
        }));
    }
    Ok(out)
}

struct Facts<'a> {
    folder: &'a Path,
    project: &'a str,
    campaign: &'a str,
    task: &'a str,
    title: &'a str,
    seal: &'a Value,
    seal_path: &'a Path,
    report: Option<&'a Path>,
    report_text: &'a str,
    directive: Option<&'a Path>,
    result: &'a str,
}

fn build(f: Facts) -> Value {
    let candidate = f.seal["candidate"].as_str().map(Path::new);
    let place = candidate.map(candidate_words).unwrap_or_default();
    let campaign = campaign_words(f.campaign);
    let where_ = [f.project, campaign.as_str(), place.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ");
    let changed = changed_files(f.report_text);
    let plain_title = plain(&strip_task_prefix(f.title, f.task));
    let what = match &changed {
        Changed::Files(files, more) => {
            let mut list = files.join(", ");
            if *more > 0 {
                list.push_str(&format!(" and {more} more"));
            }
            format!("{plain_title}. Changed: {list}")
        }
        Changed::Words(words) => format!("{plain_title}. {words}"),
        Changed::Nothing => plain_title.clone(),
    };
    let sealed = f.seal["sealed_at"].as_str().and_then(parse_time);
    let started = started_at(f.folder, f.task, candidate);
    let duration_ms = match (started, sealed) {
        (Some(a), Some(b)) if b >= a => Some((b - a).num_milliseconds()),
        _ => None,
    };
    let stamp = |t: Option<DateTime<FixedOffset>>| t.map(|t| t.to_rfc3339()).unwrap_or_default();
    let span = time_span(started, sealed, duration_ms);
    let mut lines = vec![
        [
            campaign.as_str(),
            f.task,
            &duration_words(duration_ms),
            f.result,
        ]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · "),
        format!("Where: {where_}"),
        format!("What: {what}"),
    ];
    if !span.is_empty() {
        lines.push(format!("When: {span}"));
    }
    if !f.result.is_empty() {
        lines.push(format!("Result: {}", f.result));
    }
    let segments: Vec<Value> = lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| json!({"id": format!("{}", i + 1), "text": text}))
        .collect();
    let mut refs = Vec::new();
    if let Some(d) = f.directive {
        refs.push(json!({"kind":"directive","path":d}));
    }
    if let Some(r) = f.report {
        refs.push(json!({"kind":"report","path":r}));
    }
    refs.push(json!({"kind":"freeze","path":f.seal_path}));
    let files = match &changed {
        Changed::Files(files, _) => json!(files),
        _ => json!([]),
    };
    json!({
        "id": run_id(f.folder, f.task),
        "kind": "run",
        "record_state": "active",
        "revision": "1",
        "title": f.title,
        "task": f.task,
        "where": where_,
        "what": what,
        "files_changed": files,
        "started_at": stamp(started),
        "sealed_at": stamp(sealed),
        "captured_at": stamp(sealed),
        "duration_ms": duration_ms,
        "result": f.result,
        "seal_version": f.seal["version"],
        "segments": segments,
        "source_refs": refs,
    })
}

fn run_id(folder: &Path, task: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(folder.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(task.as_bytes());
    let hex: String = digest
        .finalize()
        .iter()
        .take(10)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("run-{hex}")
}

fn valid_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("run-") && id[4..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `artifacts/TNN/freeze.json` of a folder, in task order.
fn seal_files(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(folder.join("artifacts")) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let path = entry.path().join("freeze.json");
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            out.push((entry.file_name().to_string_lossy().into_owned(), path));
        }
    }
    out.sort();
    out
}

fn task_report(reports: &[PathBuf], task: &str) -> Option<PathBuf> {
    let prefix = format!("{task}_");
    reports
        .iter()
        .find(|p| file_name(p).starts_with(&prefix))
        .cloned()
}

/// When the task began: the controller's start record when it holds a time,
/// else the first recorded action of the task, else the directive's file time.
fn started_at(
    folder: &Path,
    task: &str,
    candidate: Option<&Path>,
) -> Option<DateTime<FixedOffset>> {
    if let Some(starts) = read_json(&folder.join("controller/starts.json")) {
        let mut found = None;
        for entry in starts.as_object().into_iter().flat_map(|o| o.values()) {
            let value = &entry[task];
            let text = value
                .as_str()
                .or_else(|| value["started_at"].as_str())
                .or_else(|| value["at"].as_str());
            if let Some(t) = text.and_then(parse_time) {
                found = Some(found.map_or(t, |f: DateTime<FixedOffset>| f.min(t)));
            }
        }
        if found.is_some() {
            return found;
        }
    }
    let replay = folder.join("state/replay").join(format!("{task}.jsonl"));
    if let Ok(file) = fs::File::open(&replay) {
        use std::io::BufRead;
        let first = std::io::BufReader::new(file.take_limited())
            .lines()
            .map_while(|l| l.ok())
            .find(|l| !l.trim().is_empty());
        if let Some(t) = first
            .and_then(|l| serde_json::from_str::<Value>(&l).ok())
            .and_then(|v| v["at"].as_str().and_then(parse_time))
        {
            return Some(t);
        }
    }
    // The candidate folder is made when the task is launched. (A directive's
    // own file time is when it was written, often days before the run.)
    let made = fs::symlink_metadata(candidate?).ok()?.created().ok()?;
    let secs = made.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let utc = DateTime::from_timestamp(secs, 0)?;
    Some(utc.with_timezone(&chrono::Local).fixed_offset())
}

trait TakeLimited: std::io::Read + Sized {
    fn take_limited(self) -> std::io::Take<Self> {
        self.take(MAX_FILE)
    }
}
impl TakeLimited for fs::File {}

fn parse_time(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z"))
        .ok()
}

enum Changed {
    Files(Vec<String>, usize),
    Words(String),
    Nothing,
}

/// The report's first `files changed` lines: the file names they name, or the
/// line itself in plain words when it names none.
fn changed_files(report: &str) -> Changed {
    let mut section = Vec::new();
    let mut inside = false;
    for line in report.lines() {
        let trimmed = line.trim();
        let bare = trimmed.trim_start_matches(['-', '*', ' ']).trim_start();
        if inside {
            if trimmed.starts_with('#') {
                break;
            }
            section.push(trimmed.to_owned());
            continue;
        }
        let lower = bare.to_lowercase();
        if trimmed.starts_with('#') && lower.contains("files") {
            inside = true;
        } else if let Some(rest) = lower
            .strip_prefix("files changed:")
            .or_else(|| lower.strip_prefix("files written:"))
        {
            let start = bare.len() - rest.len();
            section.push(bare[start..].trim().to_owned());
            break;
        }
    }
    let text = section.join("\n");
    let mut files: Vec<String> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || ",;()`|[]".contains(c)) {
        let token = token.trim_matches(|c: char| ".:'\"*".contains(c));
        let name = token.rsplit('/').next().unwrap_or_default();
        if is_file_name(name) && !files.iter().any(|f| f == name) {
            files.push(name.to_owned());
        }
    }
    if !files.is_empty() {
        let more = files.len().saturating_sub(MAX_FILES_NAMED);
        files.truncate(MAX_FILES_NAMED);
        return Changed::Files(files, more);
    }
    let first = section
        .iter()
        .map(|l| l.trim_start_matches(['-', '*', ' ']).trim())
        .find(|l| !l.is_empty())
        .map(plain)
        .unwrap_or_default();
    if first.is_empty() {
        Changed::Nothing
    } else {
        Changed::Words(first)
    }
}

fn is_file_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=6).contains(&ext.len())
        && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && !stem.chars().all(|c| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

/// Text for a display field: no backticks, a path reduced to its last name,
/// no hash-like token, at most 240 characters.
fn plain(text: &str) -> String {
    let words: Vec<String> = text
        .replace('`', "")
        .split_whitespace()
        .filter_map(|word| {
            let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if core.len() >= 7
                && core.chars().all(|c| c.is_ascii_hexdigit())
                && core.chars().any(|c| c.is_ascii_digit())
            {
                return None;
            }
            if word.contains('/') {
                let trail: String = word
                    .chars()
                    .rev()
                    .take_while(|c| ".,;:)".contains(*c))
                    .collect();
                let name = word
                    .trim_end_matches(|c: char| ".,;:)".contains(c))
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                return (!name.is_empty()).then(|| name + &trail.chars().rev().collect::<String>());
            }
            Some(word.to_owned())
        })
        .collect();
    let joined = words.join(" ");
    if joined.chars().count() > 240 {
        joined.chars().take(239).collect::<String>() + "…"
    } else {
        joined
    }
}

fn heading(report: &str) -> Option<String> {
    report
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim().to_owned())
        .filter(|l| !l.is_empty())
}

fn strip_task_prefix(title: &str, task: &str) -> String {
    let rest = title.strip_prefix(task).unwrap_or(title);
    let rest = rest.trim_start_matches([' ', '-', '—', '–', ':']);
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => title.to_owned(),
    }
}

/// `2026-10-13_work_pages_reproof` reads `work pages reproof`.
fn campaign_words(name: &str) -> String {
    let bytes = name.as_bytes();
    let dated = bytes.len() > 11
        && bytes[..10].iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        && bytes[10] == b'_';
    let rest = if dated { &name[11..] } else { name };
    rest.replace(['_', '-'], " ")
}

/// `…/work/T07/candidate` reads `T07 candidate`.
fn candidate_words(path: &Path) -> String {
    let last = file_name(path);
    match path.parent().map(file_name) {
        Some(parent) if !parent.is_empty() && last == "candidate" => format!("{parent} {last}"),
        _ => last,
    }
}

fn duration_words(ms: Option<i64>) -> String {
    let Some(ms) = ms else { return String::new() };
    let minutes = (ms + 30_000) / 60_000;
    match minutes {
        0 => "under a minute".into(),
        1 => "1 minute".into(),
        m if m < 60 => format!("{m} minutes"),
        m => {
            let (h, m) = (m / 60, m % 60);
            let hours = if h == 1 {
                "1 hour".into()
            } else {
                format!("{h} hours")
            };
            match m {
                0 => hours,
                1 => format!("{hours} 1 minute"),
                m => format!("{hours} {m} minutes"),
            }
        }
    }
}

fn time_span(
    started: Option<DateTime<FixedOffset>>,
    sealed: Option<DateTime<FixedOffset>>,
    ms: Option<i64>,
) -> String {
    let day = "%-d %b %Y %H:%M";
    match (started, sealed) {
        (Some(a), Some(b)) => {
            let (a, b) = (
                a.with_timezone(&chrono::Local),
                b.with_timezone(&chrono::Local),
            );
            let end = if a.date_naive() == b.date_naive() {
                b.format("%H:%M").to_string()
            } else {
                b.format(day).to_string()
            };
            format!("{} to {end}, {}", a.format(day), duration_words(ms))
                .trim_end_matches(", ")
                .to_owned()
        }
        (None, Some(b)) => format!("sealed {}", b.with_timezone(&chrono::Local).format(day)),
        (Some(a), None) => format!("began {}", a.with_timezone(&chrono::Local).format(day)),
        (None, None) => String::new(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn reader(config: &Config, args: &[&str]) -> Result<Value> {
    let output = Command::new(&config.reader)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| invalid(format!("The run reader could not be started: {e}")))?;
    if output.stdout.len() > MAX_REPLY {
        return Err(invalid("The run reader's reply is too large"));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| invalid(format!("The run reader's reply is not readable: {e}")))?;
    if !output.status.success() || value.get("error").is_some() {
        return Err(invalid(
            value["error"]["message"]
                .as_str()
                .unwrap_or("The run reader refused the folder")
                .to_owned(),
        ));
    }
    Ok(value)
}

fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(invalid("Not a regular file"));
    }
    if meta.len() > MAX_FILE {
        return Err(invalid("File is too large"));
    }
    Ok(fs::read(path)?)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&read_limited(path).ok()?).ok()
}

fn private_dir(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("The runs cache is a link"));
    }
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
