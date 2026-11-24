//! Bounded, append-only archive for imported history sources.
//!
//! The archive deliberately uses inspectable JSON and source snapshots instead
//! for authoritative data. SQLite holds only a rebuildable search projection.
//! Every source and transcript revision is retained below `raw/` and `revisions/`.

use crate::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soliton_kernel::ContentHash;
use std::cmp::Reverse;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "voice_loudness.rs"]
pub mod voice_loudness;

#[path = "chat_kinds.rs"]
pub mod chat_kinds;

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_DB_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SQL_OUTPUT: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;
const MAX_SEGMENTS: usize = 100_000;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_QUERY: usize = 512;
const MAX_PAGE: usize = 100;
const MAX_SNAPSHOT_BYTES: u64 = 1024 * 1024 * 1024;
const AUDIO_UNAVAILABLE_MARKER: &str = "audio-unavailable.json";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn private_dir(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    fs::create_dir_all(path)?;
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
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "Symlink is not accepted: {}",
                    ancestor.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn ensure_regular(path: &Path) -> Result<fs::Metadata> {
    reject_symlink(path)?;
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(invalid(format!(
            "Expected a regular file: {}",
            path.display()
        )));
    }
    if meta.len() > MAX_BYTES {
        return Err(invalid("History source exceeds the archive size limit"));
    }
    Ok(meta)
}

fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let meta = ensure_regular(path)?;
    let f = File::open(path)?;
    let mut bytes = Vec::with_capacity(meta.len().min(MAX_BYTES) as usize);
    f.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    let after = fs::metadata(path)?;
    #[cfg(unix)]
    let replaced = {
        use std::os::unix::fs::MetadataExt;
        after.dev() != meta.dev() || after.ino() != meta.ino()
    };
    #[cfg(not(unix))]
    let replaced = false;
    if replaced
        || bytes.len() as u64 != meta.len()
        || after.len() != meta.len()
        || after.modified().ok() != meta.modified().ok()
    {
        return Err(invalid("History source changed while it was being read"));
    }
    Ok(bytes)
}

fn value_string(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(Value::as_str).map(ToOwned::to_owned))
}

fn value_i64(v: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        v.get(*key)
            .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|n| n as i64)))
    })
}

fn id_hex(bytes: &[u8]) -> String {
    ContentHash::digest(bytes).short_hex(16)
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("Archive object exceeds the size limit"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Archive path has no parent"))?;
    private_dir(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("archive"),
        stamp
    ));
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    private_file(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    private_file(path)?;
    let dir = File::open(parent)?;
    dir.sync_all()?;
    Ok(())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_bytes(path, &bytes)
}

fn check_component_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(invalid("Unsafe archive path component"));
    }
    Ok(())
}

fn copy_snapshot(
    source: &Path,
    destination: &Path,
    count: &mut usize,
    total_bytes: &mut u64,
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        return Err(invalid("History source directory is too deep"));
    }
    reject_symlink(source)?;
    let meta = fs::metadata(source)?;
    if meta.is_file() {
        let bytes = read_limited(source)?;
        *total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("History source size overflow"))?;
        if *total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(invalid("History source snapshot exceeds the size limit"));
        }
        atomic_bytes(destination, &bytes)?;
        *count += 1;
        if *count > 512 {
            return Err(invalid("History source contains too many files"));
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(invalid("History source contains a non-file entry"));
    }
    private_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        check_component_name(&name)?;
        let src = entry.path();
        reject_symlink(&src)?;
        copy_snapshot(&src, &destination.join(name), count, total_bytes, depth + 1)?;
    }
    Ok(())
}

fn parse_text(bytes: &[u8], what: &str) -> Result<String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid(format!("{what} is not UTF-8")))?
        .to_owned();
    if text.len() > MAX_TEXT {
        return Err(invalid(format!("{what} is too large")));
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
fn segment(
    record_id: &str,
    _revision: &str,
    index: usize,
    raw_id: Option<&Value>,
    start: Option<i64>,
    end: Option<i64>,
    text: String,
    speaker: Option<String>,
) -> Value {
    // The archive key includes the source revision and position.  Raw IDs are
    // retained as metadata because Voxtype legitimately repeats id=0.
    let mut obj = json!({"id": format!("{record_id}:s{index}"), "index": index, "start_ms": start, "end_ms": end, "text": text, "speaker": speaker});
    if let Some(raw) = raw_id {
        obj["raw_id"] = raw.clone();
    }
    obj
}

/// Readable paragraphs with UTF-16 source ranges for the existing QML editor.
/// Timing remains that of the original segment, never invented word timing.
pub fn transcript_paragraphs(segments: &[Value]) -> Vec<Value> {
    fn sentence_end(text: &str) -> bool {
        text.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
            .ends_with(['.', '!', '?', '。', '！', '？', '…'])
    }
    fn flush(
        paragraphs: &mut Vec<Value>,
        text: &mut String,
        spans: &mut Vec<Value>,
        chars: &mut usize,
        units: &mut usize,
    ) {
        if text.is_empty() {
            return;
        }
        paragraphs.push(json!({"text":std::mem::take(text),"spans":std::mem::take(spans)}));
        *chars = 0;
        *units = 0;
    }
    let mut paragraphs = Vec::new();
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut paragraph_chars = 0;
    let mut paragraph_units = 0;
    let mut previous: Option<&Value> = None;
    for (index, segment) in segments.iter().enumerate() {
        let raw = segment["text"].as_str().unwrap_or("");
        if raw.trim().is_empty() {
            continue;
        }
        if let Some(previous) = previous {
            let gap = previous["end_ms"]
                .as_i64()
                .zip(segment["start_ms"].as_i64())
                .map_or(0, |(end, start)| start.saturating_sub(end));
            let speaker_change = previous["speaker"]
                .as_str()
                .zip(segment["speaker"].as_str())
                .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && a != b);
            if speaker_change || gap >= 1500 || (gap >= 300 && sentence_end(&text)) {
                flush(
                    &mut paragraphs,
                    &mut text,
                    &mut spans,
                    &mut paragraph_chars,
                    &mut paragraph_units,
                );
            }
        }
        // A capture chunk may contain several sentences (or an entire note).
        // Only cut at sentence punctuation followed by whitespace, or at a
        // retained blank line. Decimal points therefore stay inside words.
        let mut boundaries = vec![0];
        for (offset, ch) in raw.char_indices() {
            if (ch.is_whitespace() && sentence_end(raw[..offset].trim_end()))
                || (ch == '\n' && raw[..offset].ends_with('\n'))
            {
                boundaries.push(offset);
            }
        }
        boundaries.push(raw.len());
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut source_byte = 0;
        let mut source_units = 0;
        for pair in boundaries.windows(2) {
            let part = &raw[pair[0]..pair[1]];
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let start = pair[0] + part.len() - part.trim_start().len();
            let end = start + trimmed.len();
            let unit_start = source_units + raw[source_byte..start].encode_utf16().count();
            let unit_end = unit_start + trimmed.encode_utf16().count();
            source_byte = end;
            source_units = unit_end;
            if (paragraph_chars >= 440 && sentence_end(&text)) || paragraph_chars >= 1200 {
                // Unpunctuated capture fragments still form bounded readable
                // groups. Tracking lengths avoids rescanning the growing text.
                flush(
                    &mut paragraphs,
                    &mut text,
                    &mut spans,
                    &mut paragraph_chars,
                    &mut paragraph_units,
                );
            }
            if !text.is_empty() {
                text.push(' ');
                paragraph_chars += 1;
                paragraph_units += 1;
            }
            let display_start = paragraph_units;
            text.push_str(trimmed);
            paragraph_chars += trimmed.chars().count();
            paragraph_units += unit_end - unit_start;
            spans.push(json!({"segment_id":segment["id"],"segment_index":index,
                "source_start":unit_start,"source_end":unit_end,
                "start":display_start,"end":paragraph_units,
                "start_ms":segment["start_ms"],"end_ms":segment["end_ms"]}));
        }
        previous = Some(segment);
    }
    // Keep a final unfinished sentence with the preceding paragraph rather
    // than presenting a tiny detached capture fragment.
    if !text.is_empty() && !sentence_end(&text) && !paragraphs.is_empty() {
        let last = paragraphs.last_mut().expect("nonempty paragraphs checked");
        let prior = last["text"].as_str().unwrap_or("").to_owned();
        let shift = prior.encode_utf16().count() + 1;
        for span in &mut spans {
            span["start"] = json!(span["start"].as_u64().unwrap_or(0) + shift as u64);
            span["end"] = json!(span["end"].as_u64().unwrap_or(0) + shift as u64);
        }
        last["text"] = json!(format!("{prior} {text}"));
        if let Some(prior_spans) = last["spans"].as_array_mut() {
            prior_spans.extend(spans);
        }
    } else {
        flush(
            &mut paragraphs,
            &mut text,
            &mut spans,
            &mut paragraph_chars,
            &mut paragraph_units,
        );
    }
    paragraphs
}

/// The playback lens of a voice record, read from the original recording.
/// The opening window gives a provisional gain at once; the whole-file lens
/// follows from the background measure. A failed or cancelled measure keeps
/// the provisional gain and, with `retry`, is measured again on this selection.
/// Returns whether the record changed and the file still to be measured whole.
fn prepare_voice(record: &mut Value, retry: bool) -> Result<(bool, Option<(PathBuf, Value)>)> {
    if record["kind"] != "voice_note" || record["audio"]["state"] != "retained" {
        return Ok((false, None));
    }
    let Some(path) = record["audio"]["path"].as_str().map(PathBuf::from) else {
        return Ok((false, None));
    };
    reject_symlink(&path)?;
    if voice_loudness::cache_matches(record, &path) {
        return Ok((false, None));
    }
    let stamp = voice_loudness::stamp(&path)?;
    let provisional = voice_loudness::provisional_matches(record, &stamp);
    match record["loudness"]["state"].as_str() {
        Some("unavailable") if !retry && record["_playback_stamp"] == stamp => {
            return Ok((false, None));
        }
        Some("measuring") if provisional.is_some() => return Ok((false, Some((path, stamp)))),
        _ => {}
    }
    let (lens, gain) = match provisional {
        Some(gain) => (record["loudness"]["provisional"].clone(), gain),
        None => match voice_loudness::measure_opening(&path) {
            Ok(measured) => measured,
            // Playback stays usable at unity until the whole-file lens answers;
            // nothing is cached as final, and the interface shows no notice.
            Err(error) => (
                json!({"state":"unavailable","reason":error.to_string()}),
                1.0,
            ),
        },
    };
    if voice_loudness::stamp(&path)? != stamp {
        return Err(invalid("Recording changed during playback preparation"));
    }
    record["loudness"] = json!({"state":"measuring","provisional":lens});
    record["playback_gain"] = json!(gain);
    record["_playback_stamp"] = stamp.clone();
    Ok((true, Some((path, stamp))))
}

/// A whole-file measure finished in the background, waiting to be recorded.
struct Refined {
    id: String,
    path: PathBuf,
    stamp: Value,
    result: std::result::Result<(Value, f64), String>,
}

/// One background worker measures whole recordings, newest selection first.
/// It hands a finished gain to the player that asked for this record.
#[derive(Default)]
struct Refiner {
    queue: VecDeque<(String, PathBuf, Value)>,
    busy: Option<String>,
    worker: bool,
    closed: bool,
    done: Vec<Refined>,
    player: Option<(String, u32)>,
}

const REFINE_QUEUE: usize = 4;

fn refine(shared: &Arc<Mutex<Refiner>>, id: &str, path: PathBuf, stamp: Value) {
    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
    if state.closed
        || state.busy.as_deref() == Some(id)
        || state.queue.iter().any(|(queued, ..)| queued == id)
        || state
            .done
            .iter()
            .any(|done| done.id == id && done.stamp == stamp)
    {
        return;
    }
    state.queue.push_front((id.to_owned(), path, stamp));
    state.queue.truncate(REFINE_QUEUE);
    if state.worker {
        return;
    }
    state.worker = true;
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        loop {
            let (id, path, stamp) = {
                let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                match state.queue.pop_front().filter(|_| !state.closed) {
                    Some(job) => {
                        state.busy = Some(job.0.clone());
                        job
                    }
                    None => {
                        state.worker = false;
                        return;
                    }
                }
            };
            let result = voice_loudness::measure(&path).map_err(|error| error.to_string());
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            state.busy = None;
            // Under the lock: a request for this record's gain either sees this
            // result or is seen here, so the player always ends on the refined gain.
            if let (Ok((_, gain)), Some((_, pid))) = (
                &result,
                state
                    .player
                    .as_ref()
                    .filter(|(player_id, _)| *player_id == id),
            ) {
                let _ = voice_loudness::apply_playback_gain(*pid, *gain);
            }
            state.done.push(Refined {
                id,
                path,
                stamp,
                result,
            });
        }
    });
}

/// Word intervals in milliseconds. Missing measurements are explicitly estimated.
/// Character weights count Unicode scalar values, excluding whitespace.
pub fn segment_word_timings(segment: &Value) -> Result<Vec<Value>> {
    let words: Vec<&str> = segment["text"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(measured) = segment
        .get("words")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        return measured
            .iter()
            .map(|word| {
                let start = word["start_ms"]
                    .as_i64()
                    .ok_or_else(|| invalid("Word start_ms missing"))?;
                let end = word["end_ms"]
                    .as_i64()
                    .ok_or_else(|| invalid("Word end_ms missing"))?;
                if end < start
                    || word
                        .get("text")
                        .or_else(|| word.get("word"))
                        .and_then(Value::as_str)
                        .is_none()
                {
                    return Err(invalid("Invalid measured word interval"));
                }
                let mut result = word.clone();
                result["estimated"] = json!(word["estimated"].as_bool().unwrap_or(false));
                Ok(result)
            })
            .collect();
    }
    let start = segment["start_ms"]
        .as_i64()
        .ok_or_else(|| invalid("Segment start_ms missing"))?;
    let end = segment["end_ms"]
        .as_i64()
        .ok_or_else(|| invalid("Segment end_ms missing"))?;
    if end < start {
        return Err(invalid("Invalid segment interval"));
    }
    let total: usize = words.iter().map(|word| word.chars().count()).sum();
    let duration = i128::from(end) - i128::from(start);
    let mut used = 0usize;
    Ok(words.into_iter().map(|word| {
        let word_start = i128::from(start) + duration * used as i128 / total as i128;
        used += word.chars().count();
        let word_end = i128::from(start) + duration * used as i128 / total as i128;
        json!({"text":word,"start_ms":word_start as i64,"end_ms":word_end as i64,"estimated":true})
    }).collect())
}

fn normalize_revision(record: &Value, revision: &str) -> Value {
    let mut out = record.clone();
    out["revision"] = json!(revision);
    out
}

#[derive(Clone, Debug)]
struct Imported {
    origin_id: String,
    source_hash: String,
    record: Value,
    snapshot: PathBuf,
}

/// Durable history archive.  The source directory passed to `open` is already
/// private to history; no planner data is read or written here.
pub struct HistoryStore {
    _archive_lock: File,
    root: PathBuf,
    records: BTreeMap<String, Value>,
    origins: BTreeMap<String, String>,
    kind_mapping: PathBuf,
    /// Completed recordings of the recorder's own folder, by folder: its
    /// fingerprint and listing (None when the folder holds no completed one).
    lens: BTreeMap<PathBuf, (String, Option<Value>)>,
    /// Lengths read from recordings, by path and stamp.
    durations: BTreeMap<String, (String, Option<i64>)>,
    refiner: Arc<Mutex<Refiner>>,
}

impl Drop for HistoryStore {
    fn drop(&mut self) {
        // The worker ends after the measure in hand; nothing new starts.
        self.refiner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        let _ = self._archive_lock.unlock();
    }
}

impl HistoryStore {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_kind_mapping(root, &chat_kinds::default_path())
    }

    pub fn open_with_kind_mapping(root: &Path, kind_mapping: &Path) -> Result<Self> {
        let mapping = chat_kinds::ChatKinds::load(kind_mapping)?;
        reject_symlink(root)?;
        if root.exists() && !root.is_dir() {
            return Err(invalid("History archive root is not a directory"));
        }
        private_dir(root)?;
        let lock_path = root.join(".archive.lock");
        reject_symlink(&lock_path)?;
        let archive_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        private_file(&lock_path)?;
        archive_lock
            .try_lock()
            .map_err(|_| invalid("Another process is using the history archive"))?;
        for name in ["records", "raw", "staging"] {
            private_dir(&root.join(name))?;
        }
        let mut records = BTreeMap::new();
        let mut origins = BTreeMap::new();
        let final_index_path = root.join("history-index.sqlite3");
        reject_symlink(&final_index_path)?;
        // The index is disposable. Rebuild into a fresh private file so a
        // corrupt cache cannot prevent recovery from immutable revisions.
        let index_path = root.join(format!(
            ".index-build-{}-{}.sqlite3",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&index_path)?;
        private_file(&index_path)?;
        let mut index_script = String::from(
            "BEGIN; CREATE TABLE IF NOT EXISTS history_index (id TEXT PRIMARY KEY, kind TEXT NOT NULL, captured_at TEXT NOT NULL, search_text TEXT NOT NULL, summary_json TEXT NOT NULL); CREATE INDEX IF NOT EXISTS history_index_captured ON history_index(captured_at); DELETE FROM history_index;",
        );
        for entry in fs::read_dir(root.join("records"))? {
            let entry = entry?;
            reject_symlink(&entry.path())?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid(
                    "History archive contains an unexpected record entry",
                ));
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            check_component_name(&id)?;
            finish_pending(root, &id)?;
            let current = entry.path().join("record.json");
            if !current.exists()
                && !entry
                    .path()
                    .join("revisions")
                    .read_dir()
                    .map(|mut it| it.next().is_some())
                    .unwrap_or(false)
            {
                continue;
            }
            // Immutable revisions are the commit record. A crash may leave the
            // convenience projection valid but stale after a correction.
            let revisions = entry.path().join("revisions");
            reject_symlink(&revisions)?;
            let mut newest: Option<(u64, PathBuf)> = None;
            for revision_entry in fs::read_dir(&revisions)? {
                let revision_entry = revision_entry?;
                let path = revision_entry.path();
                reject_symlink(&path)?;
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let n = path
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .and_then(|n| n.parse::<u64>().ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid(format!("Invalid history revision for {id}")))?;
                if newest.as_ref().is_none_or(|(prior, _)| n > *prior) {
                    newest = Some((n, path));
                }
            }
            let (_, newest_path) = newest
                .ok_or_else(|| invalid(format!("History record {id} has no committed revision")))?;
            let mut value: Value = serde_json::from_slice(&read_limited(&newest_path)?)
                .map_err(|e| invalid(format!("Corrupt history revision for {id}: {e}")))?;
            let revision_number = value
                .get("revision")
                .and_then(Value::as_str)
                .and_then(|n| n.parse::<u64>().ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| invalid(format!("Invalid history revision for {id}")))?;
            if value.get("id").and_then(Value::as_str) != Some(id.as_str())
                || revision_number.to_string()
                    != newest_path
                        .file_stem()
                        .and_then(|x| x.to_str())
                        .unwrap_or_default()
            {
                return Err(invalid(format!("History revision identity mismatch: {id}")));
            }
            let projected = read_limited(&current)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if let Some(projected) = &projected
                && projected["id"] == value["id"]
                && projected["revision"] == value["revision"]
                && projected["audio"] == value["audio"]
                && projected["_source_hash"] == value["_source_hash"]
                && projected["_playback_variants"].is_object()
            {
                value["_playback_variants"] = projected["_playback_variants"].clone();
            }
            // Loudness is a rebuildable playback lens, not a transcript revision.
            // Retain its cache across restarts without rewriting committed sources.
            if let Some(projected) = &projected
                && projected["revision"] == value["revision"]
                && projected["audio"] == value["audio"]
                && projected["_source_hash"] == value["_source_hash"]
                && let Some(path) = public_record_at(root, &value)["audio"]["path"].as_str()
                && voice_loudness::stamp(Path::new(path))
                    .is_ok_and(|stamp| projected["_playback_stamp"] == stamp)
                && projected["loudness"].is_object()
            {
                for key in ["loudness", "playback_gain", "_playback_stamp"] {
                    value[key] = projected[key].clone();
                }
            }
            chat_kinds::recover_session(root, &mut value, projected.as_ref())?;
            mapping.apply(&mut value);
            if projected.as_ref() != Some(&value) {
                atomic_json(&current, &value)?;
            }
            let origin = value
                .get("_origin_key")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("History record {id} lacks origin identity")))?;
            origins.insert(origin.to_owned(), id.clone());
            let insertion = index_insert_sql(&value);
            if index_script.len() + insertion.len() > 8 * 1024 * 1024 && index_script != "BEGIN;" {
                index_script.push_str("COMMIT;");
                sqlite_script(&index_path, &index_script, 192 * 1024 * 1024)?;
                index_script = String::from("BEGIN;");
            }
            index_script.push_str(&insertion);
            records.insert(id, slim_record(&value));
            if records.len() > MAX_RECORDS {
                return Err(invalid("History archive contains too many records"));
            }
        }
        index_script.push_str("COMMIT;");
        sqlite_script(&index_path, &index_script, 192 * 1024 * 1024)?;
        private_file(&index_path)?;
        fs::rename(&index_path, &final_index_path)?;
        File::open(root)?.sync_all()?;
        Ok(Self {
            _archive_lock: archive_lock,
            root: root.to_path_buf(),
            records,
            origins,
            kind_mapping: kind_mapping.to_path_buf(),
            lens: BTreeMap::new(),
            durations: BTreeMap::new(),
            refiner: Arc::default(),
        })
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        self.absorb_refined()?;
        if matches!(
            command,
            "history-detail" | "history-playback-gain" | "history-word-timings"
        ) {
            self.register_from_lens(args)?;
        }
        match command {
            "history-list" => self.list(args),
            "history-revision" => self
                .records
                .get(args["id"].as_str().unwrap_or(""))
                .map(|record| public_record_at(&self.root, record))
                .ok_or_else(|| invalid("History record not found")),
            "history-detail" => self.detail(args),
            "history-playback-gain" => {
                let player_pid = args["player_pid"]
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .ok_or_else(|| invalid("Player process identity required"))?;
                let record = self.prepared_detail(args, false)?;
                let id = record["id"].as_str().unwrap_or("").to_owned();
                // The provisional gain now; the worker hands this player the
                // whole-file gain when it is ready (see `refine`).
                let mut state = self.refiner.lock().unwrap_or_else(|e| e.into_inner());
                state.player = Some((id.clone(), player_pid));
                let refined = state
                    .done
                    .iter()
                    .rev()
                    .find(|done| done.id == id)
                    .and_then(|done| done.result.as_ref().ok().map(|(_, gain)| *gain));
                voice_loudness::apply_playback_gain(
                    player_pid,
                    refined
                        .or_else(|| record["playback_gain"].as_f64())
                        .unwrap_or(1.0),
                )
            }
            "history-word-timings" => {
                let record = self.detail(args)?;
                let transcript = if record["provider"].as_str() == Some("voxtype") {
                    record["archive_path"]
                        .as_str()
                        .map(Path::new)
                        .map(|path| path.join("transcript.json"))
                        .filter(|path| path.exists())
                        .map(|path| {
                            read_limited(&path)
                                .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
                        })
                        .transpose()?
                } else {
                    None
                };
                let mut segments = Vec::new();
                for (index, item) in record["segments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let mut timed = item.clone();
                    // Corrections invalidate source words: only borrow timings for identical text.
                    if timed.get("words").is_none()
                        && let Some(source) =
                            transcript.as_ref().and_then(|v| v["segments"].get(index))
                        && source["text"] == item["text"]
                        && source.get("words").is_some()
                    {
                        timed["words"] = measured_words(&source["words"]);
                    }
                    segments.push(json!({"id":item["id"],"words":segment_word_timings(&timed)?}));
                }
                Ok(json!({"id":record["id"],"revision":record["revision"],"segments":segments}))
            }
            "history-import" => self.import(args),
            "history-import-batch" => {
                let sources = args["sources"]
                    .as_array()
                    .filter(|s| s.len() <= 4096)
                    .ok_or_else(|| invalid("Bounded source array required"))?;
                let mut records = Vec::new();
                for source in sources {
                    let imported = self.import(source)?;
                    records.push(summary(&imported["record"]));
                }
                Ok(json!({"records":records}))
            }
            "history-discover" => self.discover(args),
            "history-correct" => self.correct(args),
            "history-revisions" => self.revisions(args),
            "history-label" => self.label(args),
            "history-state" => self.set_state(args),
            "history-opencode-sessions" => {
                let path = PathBuf::from(
                    args["path"]
                        .as_str()
                        .ok_or_else(|| invalid("Database path required"))?,
                );
                let snapshot = opencode_backup(&path, &self.root)?;
                let offset = args["offset"].as_u64().unwrap_or(0).min(1000000);
                let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 100);
                let items = sqlite_json(
                    &snapshot,
                    &format!(
                        "SELECT id,title,parent_id,time_created,time_updated FROM session ORDER BY time_updated DESC LIMIT {limit} OFFSET {offset}"
                    ),
                )?;
                Ok(json!({"items":items,"offset":offset,"limit":limit}))
            }
            "history-chat-kinds" => Ok(chat_kinds::ChatKinds::load(&self.kind_mapping)?.palette()),
            "history-capture-list" => self.capture_list(),
            "history-capture-detail" => {
                let state = self.capture_list()?;
                let id = args["id"]
                    .as_str()
                    .ok_or_else(|| invalid("Capture source id is required"))?;
                state["sources"]
                    .as_array()
                    .and_then(|sources| sources.iter().find(|source| source["id"] == id))
                    .cloned()
                    .ok_or_else(|| invalid("Capture source not found"))
            }
            "history-capture-add" => self.capture_add(args),
            "history-capture-remove" => self.capture_remove(args),
            "history-capture-bootstrap" => self.capture_bootstrap(args),
            "history-capture-sync" => self.capture_sync(args),
            _ => Err(invalid(format!("Unknown history command: {command}"))),
        }
    }

    /// Record what the background worker measured, in the current record
    /// projection only (never in an immutable revision). A failure keeps the
    /// provisional gain and is measured again on the next selection.
    fn absorb_refined(&mut self) -> Result<()> {
        let done = std::mem::take(&mut self.refiner.lock().unwrap_or_else(|e| e.into_inner()).done);
        for refined in done {
            let Ok(mut current) = self.load_current(&refined.id) else {
                continue;
            };
            let resolved = public_record_at(&self.root, &current);
            let path_text = refined.path.to_string_lossy();
            let variant = id_hex(path_text.as_bytes());
            let target = if resolved["audio"]["path"].as_str() == Some(path_text.as_ref()) {
                &mut current
            } else if current["_playback_variants"].get(&variant).is_some() {
                &mut current["_playback_variants"][&variant]
            } else {
                continue;
            };
            if target["_playback_stamp"] != refined.stamp
                || voice_loudness::stamp(&refined.path).ok().as_ref() != Some(&refined.stamp)
            {
                continue;
            }
            let provisional = target["loudness"]["provisional"].clone();
            match refined.result {
                Ok((mut loudness, gain)) => {
                    loudness["provisional"] = provisional;
                    target["loudness"] = loudness;
                    target["playback_gain"] = json!(gain);
                }
                Err(reason) => {
                    target["loudness"] =
                        json!({"state":"unavailable","reason":reason,"provisional":provisional});
                }
            }
            atomic_json(
                &self
                    .root
                    .join("records")
                    .join(&refined.id)
                    .join("record.json"),
                &current,
            )?;
        }
        Ok(())
    }

    /// Recorder folders listed in place: each enabled or disabled (never
    /// removed) Voxtype capture source, at its meetings folder when it has one.
    fn recorder_roots(&self) -> Vec<PathBuf> {
        let Ok(state) = self.capture_state() else {
            return Vec::new();
        };
        let mut roots: Vec<PathBuf> = state["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|source| source["provider"] == "voxtype" && source["removed_at"].is_null())
            .filter_map(|source| source["path"].as_str().map(PathBuf::from))
            .filter(|path| path.is_dir() && !path.join("metadata.json").is_file())
            .map(|path| {
                if path.join("meetings").is_dir() {
                    path.join("meetings")
                } else {
                    path
                }
            })
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Completed recordings not yet registered, read in place from the
    /// recorder's folders. Reads metadata, transcript and the audio header;
    /// writes nothing anywhere.
    fn lens_entries(&mut self) -> Vec<Value> {
        let mut seen = std::collections::BTreeSet::new();
        for root in self.recorder_roots() {
            let Ok(entries) = fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let folder = entry.path();
                if !entry.file_type().is_ok_and(|kind| kind.is_dir())
                    || !folder.join("metadata.json").is_file()
                {
                    continue;
                }
                let Ok(fingerprint) = capture_directory_fingerprint(&folder) else {
                    continue;
                };
                seen.insert(folder.clone());
                if self
                    .lens
                    .get(&folder)
                    .is_some_and(|(known, _)| *known == fingerprint)
                {
                    continue;
                }
                let listed = lens_summary(&folder);
                self.lens.insert(folder, (fingerprint, listed));
            }
        }
        self.lens.retain(|folder, _| seen.contains(folder));
        self.lens
            .values()
            .filter_map(|(_, listed)| listed.as_ref())
            .filter(|listed| {
                listed["_origin_key"]
                    .as_str()
                    .is_some_and(|key| !self.origins.contains_key(key))
            })
            .cloned()
            .collect()
    }

    /// Opening a recording listed in place registers it: a record holding the
    /// original's locator and the source hash, never a copy of its audio.
    fn register_from_lens(&mut self, args: &Value) -> Result<()> {
        let Some(id) = args["id"].as_str() else {
            return Ok(());
        };
        if self.records.contains_key(id) {
            return Ok(());
        }
        let find = |lens: &BTreeMap<PathBuf, (String, Option<Value>)>| {
            lens.iter()
                .find(|(_, (_, listed))| listed.as_ref().is_some_and(|l| l["id"] == id))
                .map(|(folder, _)| folder.clone())
        };
        let folder = match find(&self.lens) {
            Some(folder) => Some(folder),
            None => {
                self.lens_entries();
                find(&self.lens)
            }
        };
        if let Some(folder) = folder {
            self.import(&json!({"path":folder,"provider":"voxtype"}))?;
        }
        Ok(())
    }

    /// A recording's length read from the file, cached by its stamp.
    fn file_duration(&mut self, path: &str) -> Option<i64> {
        let metadata = fs::metadata(path).ok()?;
        let stamp = capture_metadata_fingerprint(&metadata);
        if let Some((known, ms)) = self.durations.get(path)
            && *known == stamp
        {
            return *ms;
        }
        let ms = voice_loudness::duration_ms(Path::new(path));
        self.durations.insert(path.to_owned(), (stamp, ms));
        ms
    }

    fn with_file_durations(&mut self, items: &mut [Value]) {
        for item in items {
            if item["kind"] != "voice_note" {
                continue;
            }
            if let Some(ms) = item["audio"]["path"]
                .as_str()
                .map(str::to_owned)
                .and_then(|path| self.file_duration(&path))
            {
                item["duration_ms"] = json!(ms);
            }
        }
    }

    fn list(&mut self, args: &Value) -> Result<Value> {
        let mapping = chat_kinds::ChatKinds::load(&self.kind_mapping)?;
        let state = args["state"].as_str().unwrap_or("active");
        if !matches!(state, "active" | "archived" | "deleted") {
            return Err(invalid("Invalid history state"));
        }
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("all");
        if !matches!(kind, "all" | "voice_note" | "chat" | "run" | "source") {
            return Err(invalid("Invalid history kind"));
        }
        let query = args.get("q").and_then(Value::as_str).unwrap_or("");
        if query.len() > MAX_QUERY {
            return Err(invalid("History search query is too long"));
        }
        let query = query.to_lowercase();
        let provider = args.get("provider").and_then(Value::as_str).unwrap_or("");
        if provider.len() > 128 {
            return Err(invalid("History provider filter is too long"));
        }
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(MAX_RECORDS as u64) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, MAX_PAGE as u64) as usize;
        if !query.is_empty() || !provider.is_empty() {
            let (items, total) =
                index_search(&self.root, kind, provider, &query, state, offset, limit)?;
            let mut items = items
                .iter()
                .map(|record| self.classified_summary(record, &mapping))
                .collect::<Result<Vec<_>>>()?;
            self.with_file_durations(&mut items);
            return Ok(
                json!({"items":items,"total":total,"offset":offset.min(total),"limit":limit}),
            );
        }
        let hidden: std::collections::BTreeSet<&str> = args
            .get("hidden")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut rows: Vec<Value> = self
            .records
            .values()
            .filter(|record| chat_kinds::relevant(record))
            .filter(|record| record["record_state"].as_str().unwrap_or("active") == state)
            .filter(|record| {
                !record
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| hidden.contains(id))
            })
            .filter(|record| {
                kind == "all" || record.get("kind").and_then(Value::as_str) == Some(kind)
            })
            .cloned()
            .collect();
        if state == "active" && kind == "run" {
            rows.extend(crate::runs::records(&self.root));
        }
        if state == "active" && matches!(kind, "all" | "voice_note") {
            rows.extend(
                self.lens_entries()
                    .into_iter()
                    .filter(|listed| !listed["id"].as_str().is_some_and(|id| hidden.contains(id))),
            );
        }
        rows.sort_by_key(|v| {
            Reverse(
                v.get("captured_at")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )
        });
        let total = rows.len();
        let mut items = rows
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|record| self.classified_summary(&record, &mapping))
            .collect::<Result<Vec<_>>>()?;
        self.with_file_durations(&mut items);
        Ok(json!({"items":items,"total":total,"offset":offset,"limit":limit}))
    }

    fn classified_summary(&self, record: &Value, mapping: &chat_kinds::ChatKinds) -> Result<Value> {
        let mut value = if record["kind"] == "chat" {
            self.load_current(
                record["id"]
                    .as_str()
                    .ok_or_else(|| invalid("History record id is missing"))?,
            )?
        } else {
            record.clone()
        };
        mapping.apply(&mut value);
        Ok(public_record_at(&self.root, &summary(&value)))
    }

    fn detail(&self, args: &Value) -> Result<Value> {
        crate::runs::detail(&self.root, args["id"].as_str().unwrap_or(""))
            .map_or_else(|| self.prepared_detail(args, true), Ok)
    }

    fn prepared_detail(&self, args: &Value, retry: bool) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("History id is required"))?;
        if !self.records.contains_key(id) {
            return Err(invalid("History record not found"));
        }
        let mut current = self.load_current(id)?;
        let resolved_current = public_record_at(&self.root, &current);
        let mut value = if let Some(revision) = args.get("revision").and_then(Value::as_str) {
            self.load_revision(id, revision)?
        } else {
            resolved_current.clone()
        };
        // A transcript revision shares the recording's lens. Keep the cache
        // in the current record projection, never in immutable revisions.
        let same_audio = value["audio"]["path"] == resolved_current["audio"]["path"];
        if same_audio {
            for key in ["loudness", "playback_gain", "_playback_stamp"] {
                if let Some(cached) = current.get(key) {
                    value[key] = cached.clone();
                }
            }
        }
        let cache_key = value["audio"]["path"]
            .as_str()
            .map(|path| id_hex(path.as_bytes()));
        if !same_audio
            && let Some(key) = &cache_key
            && let Some(cached) = current["_playback_variants"].get(key)
        {
            for field in ["loudness", "playback_gain", "_playback_stamp"] {
                value[field] = cached[field].clone();
            }
        }
        let (changed, whole) = prepare_voice(&mut value, retry)?;
        if let Some((path, stamp)) = whole {
            refine(&self.refiner, id, path, stamp);
        }
        if changed {
            if same_audio {
                for key in ["loudness", "playback_gain", "_playback_stamp"] {
                    current[key] = value[key].clone();
                }
            } else if let Some(key) = cache_key {
                current["_playback_variants"][key] = json!({"loudness":value["loudness"],
                    "playback_gain":value["playback_gain"],"_playback_stamp":value["_playback_stamp"]});
            }
            atomic_json(
                &self.root.join("records").join(id).join("record.json"),
                &current,
            )?;
        }
        chat_kinds::ChatKinds::load(&self.kind_mapping)?.apply(&mut value);
        let mut value = public_record(&value);
        if value["kind"] == "voice_note"
            && let Some(ms) = value["audio"]["path"]
                .as_str()
                .and_then(|path| voice_loudness::duration_ms(Path::new(path)))
        {
            value["duration_ms"] = json!(ms);
        }
        let segments = value["segments"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        value["paragraphs"] = json!(transcript_paragraphs(segments));
        value["relationships"] = self.relationships(&value);
        Ok(value)
    }

    fn relationships(&self, record: &Value) -> Value {
        let provider = record["provider"].as_str().unwrap_or("");
        let origin = record["source_refs"][0]["origin_id"]
            .as_str()
            .unwrap_or("")
            .rsplit('\0')
            .next()
            .unwrap_or("");
        let parent = record["source_refs"][0]["parent_id"].as_str().unwrap_or("");
        let mut parent_record = Value::Null;
        let mut children = Vec::new();
        for candidate in self.records.values().filter(|r| r["provider"] == provider) {
            let candidate_origin = candidate["source_refs"][0]["origin_id"]
                .as_str()
                .unwrap_or("")
                .rsplit('\0')
                .next()
                .unwrap_or("");
            if !parent.is_empty() && candidate_origin == parent {
                parent_record = json!({"id":candidate["id"],"title":candidate["title"]});
            }
            if !origin.is_empty() && candidate["source_refs"][0]["parent_id"] == origin {
                children.push(json!({"id":candidate["id"],"title":candidate["title"]}));
            }
        }
        json!({"parent_origin_id":if parent.is_empty(){Value::Null}else{json!(parent)},"parent":parent_record,"children":children,"basis":"Explicit provider metadata only"})
    }
    fn load_current(&self, id: &str) -> Result<Value> {
        if !self.records.contains_key(id) {
            return Err(invalid("History record not found"));
        }
        let path = self.root.join("records").join(id).join("record.json");
        let value: Value = serde_json::from_slice(&read_limited(&path)?)
            .map_err(|e| invalid(format!("Corrupt current history record: {e}")))?;
        if value.get("id").and_then(Value::as_str) != Some(id) {
            return Err(invalid("Current history record identity mismatch"));
        }
        Ok(value)
    }

    fn load_revision(&self, id: &str, revision: &str) -> Result<Value> {
        if revision.len() > 64 || revision.contains('/') || revision == "." || revision == ".." {
            return Err(invalid("Invalid history revision"));
        }
        let path = self
            .root
            .join("records")
            .join(id)
            .join("revisions")
            .join(format!("{revision}.json"));
        let bytes = match read_limited(&path) {
            Ok(bytes) => bytes,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(invalid("History revision not found"));
            }
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("Corrupt history revision: {e}")))?;
        Ok(public_record_at(&self.root, &value))
    }

    fn import(&mut self, args: &Value) -> Result<Value> {
        let path_string = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("History import path is required"))?;
        if path_string.len() > 4096 {
            return Err(invalid("History import path is too long"));
        }
        let path = PathBuf::from(path_string);
        reject_symlink(&path)?;
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("manual");
        if !matches!(
            provider,
            "voxtype"
                | "codex"
                | "console"
                | "antigravity"
                | "opencode"
                | "chatgpt"
                | "manual"
                | "normalized"
        ) {
            return Err(invalid("Unsupported history provider"));
        }
        if provider == "chatgpt" {
            return self.import_chatgpt(&path);
        }
        if provider == "opencode" {
            return self.publish(parse_opencode(
                &path,
                &self.root,
                args["session_id"].as_str(),
            )?);
        }
        // Capture first, then interpret exactly the captured bytes. A source
        // changing during collection can never mismatch its parsed revision.
        let capture = self.root.join("staging").join(format!(
            "capture-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        private_dir(&capture)?;
        let captured = if provider == "voxtype" {
            capture.join("recording")
        } else {
            capture.join(
                path.file_name()
                    .ok_or_else(|| invalid("Source filename is missing"))?,
            )
        };
        let result = (|| {
            let mut count = 0;
            let mut total = 0;
            if provider == "voxtype" {
                copy_voxtype_snapshot(&path, &captured, &mut count, &mut total)?;
            } else {
                ensure_regular(&path)?;
                copy_snapshot(&path, &captured, &mut count, &mut total, 0)?;
            }
            let mut imported = match provider {
                "voxtype" => parse_voxtype(&path)?,
                "codex" => parse_codex(&captured)?,
                "console" => parse_console(&captured)?,
                "antigravity" => parse_antigravity(&captured)?,
                "normalized" => parse_normalized(&captured)?,
                _ => parse_manual(&captured)?,
            };
            let original = fs::canonicalize(&path)?.to_string_lossy().into_owned();
            if provider == "manual" || imported.origin_id == captured.to_string_lossy() {
                imported.origin_id = original.clone();
            }
            if provider == "antigravity" {
                imported.origin_id = antigravity_origin(&path);
            }
            // Provider session/brain IDs are the durable origin identity. The
            // source path remains provenance, but must not be folded into the
            // identity: providers may rotate or relocate a session file while
            // retaining the same session. The earlier fallback above still
            // uses the canonical path when no provider identity was present.
            let source_origin = if matches!(provider, "codex" | "console" | "antigravity")
                && imported.origin_id != original
            {
                format!("{}\0{}", original, imported.origin_id)
            } else {
                imported.origin_id.clone()
            };
            if let Some(refs) = imported.record["source_refs"].as_array_mut()
                && let Some(first) = refs.first_mut()
            {
                first["path"] = json!(original);
                first["origin_id"] = json!(source_origin);
            }
            self.publish(imported)
        })();
        // Only our freshly created capture directory is removed. Published raw
        // bytes have their own independent copy and durable record.
        let cleanup = fs::remove_dir_all(&capture);
        match result {
            Ok(value) => {
                cleanup?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    fn import_chatgpt(&mut self, path: &Path) -> Result<Value> {
        reject_symlink(path)?;
        ensure_regular(path)?;
        let bytes = read_limited(path)?;
        let original = fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let capture = self.root.join("staging").join(format!("chatgpt-{stamp}"));
        private_dir(&capture)?;
        let captured = capture.join(
            path.file_name()
                .ok_or_else(|| invalid("ChatGPT export filename is missing"))?,
        );
        atomic_bytes(&captured, &bytes)?;
        let parsed = if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("html"))
        {
            parse_chatgpt_html(&captured, &original)?
        } else {
            chatgpt_json(&captured, &original)?
        };
        let mut imported = Vec::new();
        let mut unchanged = Vec::new();
        let mut first_record = None;
        let result = (|| {
            for mut item in parsed {
                item.snapshot = captured.clone();
                let value = self.publish(item)?;
                if first_record.is_none() {
                    first_record = Some(value["record"].clone());
                }
                let summary_value = summary(&value["record"]);
                if value["unchanged"].as_bool().unwrap_or(false) {
                    unchanged.push(summary_value);
                } else {
                    imported.push(summary_value);
                }
            }
            Ok::<(), Error>(())
        })();
        let cleanup = fs::remove_dir_all(&capture);
        result?;
        cleanup?;
        let first = first_record
            .ok_or_else(|| invalid("ChatGPT export contains no visible conversations"))?;
        Ok(json!({
            "record": first,
            "records": imported,
            "unchanged_records": unchanged,
            "unchanged": imported.is_empty(),
            "provider": "chatgpt",
        }))
    }

    fn capture_file(&self) -> PathBuf {
        self.root.join("capture-sources.json")
    }

    fn capture_state(&self) -> Result<Value> {
        let path = self.capture_file();
        if !path.exists() {
            return Ok(json!({"schema":"soliton-history-capture/1","sources":[]}));
        }
        let bytes = read_limited(&path)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("Capture source registry is corrupt: {e}")))?;
        if value["schema"] != "soliton-history-capture/1" || !value["sources"].is_array() {
            return Err(invalid("Unsupported capture source registry"));
        }
        Ok(value)
    }

    fn capture_list(&self) -> Result<Value> {
        let state = self.capture_state()?;
        let sources = state["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .map(capture_facts)
            .collect::<Vec<_>>();
        Ok(json!({
            "sources": sources,
            "coverage": {
                "opt_in": true,
                "supported": ["antigravity", "chatgpt", "codex", "opencode", "console", "voxtype"],
                "retention": "Text sources are kept as archive snapshots; recordings are read in place and never copied; source files are never modified.",
                "capture": "Only explicitly enabled local paths are checked; no web API, cookie or account scraping is used."
            }
        }))
    }

    fn capture_add(&mut self, args: &Value) -> Result<Value> {
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Capture provider is required"))?;
        if !matches!(
            provider,
            "antigravity" | "chatgpt" | "codex" | "opencode" | "console" | "voxtype"
        ) {
            return Err(invalid("Unsupported capture provider"));
        }
        let path_string = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Capture source path is required"))?;
        let path = PathBuf::from(path_string);
        reject_symlink(&path)?;
        if !path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(invalid(
                "Capture source path must be absolute and normalized",
            ));
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| invalid("Capture source path does not exist"))?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(invalid(
                "Capture source must be a regular file or directory",
            ));
        }
        let canonical = fs::canonicalize(&path)?.to_string_lossy().into_owned();
        let id = format!(
            "capture-{}",
            id_hex(format!("{provider}\0{canonical}").as_bytes())
        );
        let mut state = self.capture_state()?;
        let sources = state["sources"]
            .as_array_mut()
            .ok_or_else(|| invalid("Capture registry sources are malformed"))?;
        let mut entry = json!({
            "id": id,
            "provider": provider,
            "path": canonical,
            "enabled": true,
            "opt_in": true,
            "created_at": now(),
            "last_checked_at": Value::Null,
            "last_result": Value::Null,
            "last_error": Value::Null,
            "watermarks": {}
        });
        if let Some(existing) = sources.iter_mut().find(|value| value["id"] == entry["id"]) {
            entry["created_at"] = existing["created_at"].clone();
            for key in ["last_checked_at", "last_result", "last_error", "watermarks"] {
                if let Some(value) = existing.get(key) {
                    entry[key] = value.clone();
                }
            }
            *existing = entry.clone();
        } else {
            if sources.len() >= 64 {
                return Err(invalid("Capture source limit reached"));
            }
            sources.push(entry.clone());
        }
        atomic_json(&self.capture_file(), &state)?;
        Ok(entry)
    }

    fn capture_bootstrap(&mut self, args: &Value) -> Result<Value> {
        let home = args
            .get("home")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let codex_home = args
            .get("codex_home")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
            .or_else(|| home.as_ref().map(|path| path.join(".codex")));
        let targets = [
            (
                "codex",
                args.get("codex_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .or_else(|| codex_home.map(|path| path.join("sessions"))),
            ),
            (
                "antigravity",
                args.get("antigravity_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .or_else(|| {
                        home.as_ref()
                            .map(|path| path.join(".gemini/antigravity/brain"))
                    }),
            ),
            (
                "voxtype",
                args.get("voxtype_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .or_else(|| home.map(|path| path.join(".local/share/voxtype/meetings"))),
            ),
        ];
        let mut state = self.capture_state()?;
        let sources = state["sources"]
            .as_array_mut()
            .ok_or_else(|| invalid("Capture registry sources are malformed"))?;
        let mut added = Vec::new();
        let mut skipped = Vec::new();
        for (provider, candidate) in targets {
            let Some(path) = candidate else {
                skipped
                    .push(json!({"provider":provider,"reason":"No bootstrap path was supplied"}));
                continue;
            };
            if sources.iter().any(|source| source["provider"] == provider) {
                skipped.push(
                    json!({"provider":provider,"reason":"Provider already has a registry entry"}),
                );
                continue;
            }
            if !path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
            {
                return Err(invalid(
                    "Bootstrap source path must be absolute and normalized",
                ));
            }
            reject_symlink(&path)?;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() || metadata.is_file() => metadata,
                Ok(_) => {
                    skipped.push(json!({"provider":provider,"path":path,"reason":"Bootstrap path is not a file or directory"}));
                    continue;
                }
                Err(_) => {
                    skipped.push(json!({"provider":provider,"path":path,"reason":"Bootstrap path does not exist"}));
                    continue;
                }
            };
            let _ = metadata;
            let canonical = fs::canonicalize(&path)?.to_string_lossy().into_owned();
            let id = format!(
                "capture-{}",
                id_hex(format!("{provider}\0{canonical}").as_bytes())
            );
            let entry = json!({
                "id": id,
                "provider": provider,
                "path": canonical,
                "enabled": true,
                "opt_in": true,
                "auto_discovered": true,
                "created_at": now(),
                "last_checked_at": Value::Null,
                "last_result": Value::Null,
                "last_error": Value::Null,
                "watermarks": {}
            });
            if sources.len() >= 64 {
                return Err(invalid("Capture source limit reached"));
            }
            sources.push(entry.clone());
            added.push(entry);
        }
        if !added.is_empty() {
            atomic_json(&self.capture_file(), &state)?;
        }
        Ok(json!({"added":added,"skipped":skipped,"sources":state["sources"]}))
    }

    fn capture_remove(&mut self, args: &Value) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Capture source id is required"))?;
        let mut state = self.capture_state()?;
        let source = state["sources"]
            .as_array_mut()
            .and_then(|items| items.iter_mut().find(|value| value["id"] == id))
            .ok_or_else(|| invalid("Capture source not found"))?;
        source["enabled"] = json!(false);
        source["removed_at"] = json!(now());
        let result = source.clone();
        atomic_json(&self.capture_file(), &state)?;
        Ok(result)
    }

    fn capture_sync(&mut self, args: &Value) -> Result<Value> {
        let requested = args.get("id").and_then(Value::as_str);
        let state = self.capture_state()?;
        let entries = state["sources"]
            .as_array()
            .ok_or_else(|| invalid("Capture registry sources are malformed"))?
            .iter()
            .filter(|source| {
                source["enabled"] == true && requested.is_none_or(|id| source["id"] == id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut results = Vec::new();
        for entry in entries {
            let id = entry["id"].as_str().unwrap_or("").to_owned();
            let provider = entry["provider"].as_str().unwrap_or("").to_owned();
            let path = entry["path"].as_str().unwrap_or("").to_owned();
            let result = if provider == "voxtype" {
                self.capture_voxtype_source(&id, Path::new(&path))
            } else if matches!(provider.as_str(), "antigravity" | "codex" | "console")
                && Path::new(&path).is_dir()
            {
                self.capture_directory(&id, &provider, Path::new(&path))
            } else if provider == "chatgpt" && Path::new(&path).is_dir() {
                self.discover(&json!({"path":path,"provider":"chatgpt","limit":100}))
            } else {
                self.import(&json!({"path":path,"provider":provider}))
            };
            let mut updated = self.capture_state()?;
            if let Some(source) = updated["sources"]
                .as_array_mut()
                .and_then(|items| items.iter_mut().find(|value| value["id"] == id))
            {
                source["last_checked_at"] = json!(now());
                match &result {
                    Ok(value) => {
                        let error_count = value["errors"].as_array().map_or(0, Vec::len);
                        source["last_error"] = if error_count == 0 {
                            Value::Null
                        } else {
                            json!(format!(
                                "Partial capture: {error_count} source item(s) could not be imported"
                            ))
                        };
                        source["last_result"] = json!({
                            "imported": value["imported"].as_array().map_or(0, Vec::len),
                            "unchanged": result_count(&value["unchanged_records"], &value["unchanged"]),
                            "failed": error_count
                        });
                    }
                    Err(error) => {
                        source["last_error"] = json!(error.to_string());
                        source["last_result"] = json!({"imported":0,"unchanged":0,"failed":1});
                    }
                }
                *source = capture_facts(source);
                atomic_json(&self.capture_file(), &updated)?;
            }
            results.push(match result {
                Ok(value) => {
                    let error_count = value["errors"].as_array().map_or(0, Vec::len);
                    let status = if error_count == 0 { "complete" } else { "partial" };
                    json!({
                        "id":id,
                        "provider":provider,
                        "ok":error_count == 0,
                        "status":status,
                        "imported": value["imported"].as_array().map_or(0, Vec::len),
                        "unchanged": result_count(&value["unchanged_records"], &value["unchanged"]),
                        "records": value["imported"].clone(),
                        "errors": value["errors"].clone()
                    })
                }
                Err(error) => {
                    json!({"id":id,"provider":provider,"ok":false,"status":"failed","error":error.to_string()})
                }
            });
        }
        Ok(json!({"items":results,"coverage":self.capture_list()?}))
    }

    fn capture_directory(&mut self, id: &str, provider: &str, root: &Path) -> Result<Value> {
        let mut files = Vec::new();
        collect_capture_files_for_provider(root, &mut files, 0, Some(provider))?;
        let mut state = self.capture_state()?;
        let source = state["sources"]
            .as_array_mut()
            .and_then(|items| items.iter_mut().find(|value| value["id"] == id))
            .ok_or_else(|| invalid("Capture source not found"))?;
        let mut watermarks = source["watermarks"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut imported = Vec::new();
        let mut unchanged = Vec::new();
        let mut errors = Vec::new();
        for path in files {
            let path_text = path.to_string_lossy().into_owned();
            let fingerprint = capture_fingerprint(&path)?;
            if watermarks.get(&path_text).and_then(Value::as_str) == Some(fingerprint.as_str()) {
                unchanged.push(json!({"path":path_text,"reason":"watermark"}));
                continue;
            }
            match self.import(&json!({"path":path_text,"provider":provider})) {
                Ok(value) => {
                    watermarks.insert(path_text.clone(), json!(fingerprint));
                    if value["unchanged"].as_bool().unwrap_or(false) {
                        unchanged.push(summary(&value["record"]));
                    } else {
                        imported.push(summary(&value["record"]));
                    }
                }
                Err(error) => errors.push(json!({"path":path_text,"error":error.to_string()})),
            }
        }
        source["watermarks"] = Value::Object(watermarks);
        source["last_result"] =
            json!({"imported":imported.len(),"unchanged":unchanged.len(),"failed":errors.len()});
        atomic_json(&self.capture_file(), &state)?;
        Ok(json!({"imported":imported,"unchanged":unchanged,"errors":errors}))
    }

    fn capture_voxtype_source(&mut self, id: &str, root: &Path) -> Result<Value> {
        reject_symlink(root)?;
        if !root.is_dir() {
            return Err(invalid(
                "Voxtype capture source must be an existing directory",
            ));
        }
        let mut candidates: Vec<(PathBuf, String)> = Vec::new();
        if root.is_dir() {
            if root.join("metadata.json").is_file() {
                candidates.push((root.to_path_buf(), String::new()));
            } else {
                let search_root = if root.join("meetings").is_dir() {
                    root.join("meetings")
                } else {
                    root.to_path_buf()
                };
                for entry in fs::read_dir(search_root)? {
                    let entry = entry?;
                    let path = entry.path();
                    if entry.file_type()?.is_dir() && path.join("metadata.json").is_file() {
                        let captured = read_limited(&path.join("metadata.json"))
                            .ok()
                            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                            .and_then(|value| {
                                value_string(&value, &["started_at", "captured_at", "created_at"])
                            })
                            .unwrap_or_default();
                        candidates.push((path, captured));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));

        let state = self.capture_state()?;
        let mut watermarks = state["sources"]
            .as_array()
            .and_then(|sources| sources.iter().find(|source| source["id"] == id))
            .and_then(|source| source["watermarks"].as_object())
            .cloned()
            .unwrap_or_default();
        let mut imported = Vec::new();
        let mut unchanged = Vec::new();
        let mut errors = Vec::new();
        let omitted = candidates.len().saturating_sub(MAX_PAGE);
        for (candidate, _) in candidates.into_iter().take(MAX_PAGE) {
            let path_text = candidate.to_string_lossy().into_owned();
            let fingerprint = match capture_directory_fingerprint(&candidate) {
                Ok(value) => value,
                Err(error) => {
                    errors.push(json!({"path":path_text,"error":error.to_string()}));
                    continue;
                }
            };
            if watermarks.get(&path_text).and_then(Value::as_str) == Some(fingerprint.as_str()) {
                unchanged.push(json!({"path":path_text,"reason":"watermark"}));
                continue;
            }
            match self.import(&json!({"path":path_text,"provider":"voxtype"})) {
                Ok(value) => {
                    match capture_directory_fingerprint(&candidate) {
                        Ok(current) if current == fingerprint => {
                            watermarks.insert(path_text.clone(), json!(fingerprint));
                            if value["unchanged"].as_bool().unwrap_or(false) {
                                unchanged.push(summary(&value["record"]));
                            } else {
                                imported.push(summary(&value["record"]));
                            }
                        }
                        Ok(_) => errors.push(json!({"path":path_text,"error":"Voxtype recording changed while it was being captured"})),
                        Err(error) => errors.push(json!({"path":path_text,"error":error.to_string()})),
                    }
                }
                Err(error) => errors.push(json!({"path":path_text,"error":error.to_string()})),
            }
        }
        if omitted > 0 {
            errors.push(json!({"path":root.to_string_lossy(),"error":format!("Voxtype capture limit reached; {omitted} recording(s) omitted") }));
        }
        let mut updated = self.capture_state()?;
        if let Some(source) = updated["sources"]
            .as_array_mut()
            .and_then(|sources| sources.iter_mut().find(|source| source["id"] == id))
        {
            source["watermarks"] = Value::Object(watermarks);
        }
        atomic_json(&self.capture_file(), &updated)?;
        Ok(json!({"imported":imported,"unchanged":unchanged,"errors":errors}))
    }

    #[allow(clippy::collapsible_if)]
    fn publish(&mut self, imported: Imported) -> Result<Value> {
        let id = self
            .origins
            .get(&format!(
                "{}\0{}",
                imported.record["provider"].as_str().unwrap_or(""),
                imported.origin_id
            ))
            .cloned()
            .unwrap_or_else(|| {
                format!(
                    "history-{}",
                    id_hex(
                        format!(
                            "{}\0{}",
                            imported.record["provider"].as_str().unwrap_or(""),
                            imported.origin_id
                        )
                        .as_bytes()
                    )
                )
            });
        let origin_key = format!(
            "{}\0{}",
            imported.record["provider"].as_str().unwrap_or(""),
            imported.origin_id
        );
        let existing = self.records.get(&id).cloned();
        if existing.is_none() && self.records.len() >= MAX_RECORDS {
            return Err(invalid(
                "History archive record limit reached; existing records are preserved",
            ));
        }
        if let Some(current) = existing.as_ref() {
            if current.get("_source_hash").and_then(Value::as_str)
                == Some(imported.source_hash.as_str())
                && current["_adapter_version"] == 2
            {
                let mut current = self.load_current(&id)?;
                chat_kinds::ChatKinds::load(&self.kind_mapping)?.apply(&mut current);
                return Ok(
                    json!({"record":public_record_at(&self.root, &current),"unchanged":true}),
                );
            }
        }
        let revision_number = existing
            .as_ref()
            .and_then(|r| r.get("_revision_number").and_then(Value::as_u64))
            .or_else(|| {
                existing
                    .as_ref()
                    .and_then(|r| r.get("_source_revision").and_then(Value::as_u64))
            })
            .unwrap_or(0)
            + 1;
        let source_revision = existing
            .as_ref()
            .and_then(|r| r.get("_source_revision").and_then(Value::as_u64))
            .unwrap_or(0)
            + 1;
        let revision = revision_number.to_string();
        let mut record = normalize_revision(&imported.record, &revision);
        chat_kinds::ChatKinds::load(&self.kind_mapping)?.apply(&mut record);
        record["id"] = json!(id.clone());
        record["_adapter_version"] = json!(2);
        if let Some(prior) = &existing {
            if let Some(state) = prior.get("record_state") {
                record["record_state"] = state.clone();
                record["lifecycle"] = prior["lifecycle"].clone();
            }
            if prior.get("annotation").is_some() {
                record["title"] = prior["title"].clone();
                record["purpose"] = prior["purpose"].clone();
                record["annotation"] = prior["annotation"].clone();
            }
            if prior.get("correction").is_some() {
                record["prior_correction_revision"] = prior["revision"].clone();
            }
        }
        if record.get("purpose").is_none() {
            record["purpose"] = json!("Unclassified");
        }
        if let Some(segments) = record.get_mut("segments").and_then(Value::as_array_mut) {
            for (index, item) in segments.iter_mut().enumerate() {
                item["id"] = if let Some(key) = item["_stable_key"].as_str() {
                    json!(format!("{id}:e{}", id_hex(key.as_bytes())))
                } else {
                    json!(format!("{id}:s{index}"))
                };
                if let Some(object) = item.as_object_mut() {
                    object.remove("_stable_key");
                }
            }
        }
        record["imported_at"] = json!(now());
        record["_origin_key"] = json!(origin_key.clone());
        record["_source_hash"] = json!(imported.source_hash.clone());
        record["_source_revision"] = json!(source_revision);
        record["_revision_number"] = json!(revision_number);
        let mut refs = imported.record["source_refs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if refs.is_empty() {
            refs.push(json!({}));
        }
        refs[0]["provider"] = record["provider"].clone();
        if refs[0].get("origin_id").is_none() {
            refs[0]["origin_id"] = json!(imported.origin_id);
        }
        record["source_refs"] = json!(refs);
        let rec_dir = self.root.join("records").join(&id);
        private_dir(&self.root.join("raw").join(&id))?;
        if !source_matches(&imported)? {
            return Err(invalid(
                "History source changed before it could be archived",
            ));
        }
        // Snapshot publication happens before the normalized record points at it.
        let raw_dir = self.root.join("raw").join(&id).join(&revision);
        let stage_name = format!(
            "{}-{}-{}",
            id,
            revision,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let staging_dir = self.root.join("staging").join(&stage_name);
        let mut count = 0;
        let mut total_bytes = 0;
        if imported.record["provider"].as_str() == Some("soliton-inbox") {
            private_dir(&staging_dir)?;
            let target = staging_dir.join(
                imported
                    .snapshot
                    .file_name()
                    .ok_or_else(|| invalid("Audio name missing"))?,
            );
            copy_large_file(&imported.snapshot, &target, 1024 * 1024 * 1024)?;
        } else if imported.record["provider"].as_str() == Some("voxtype") {
            copy_voxtype_snapshot(
                &imported.snapshot,
                &staging_dir,
                &mut count,
                &mut total_bytes,
            )?;
        } else if imported.record["provider"].as_str() == Some("opencode") {
            copy_opencode_snapshot(
                &imported.snapshot,
                &staging_dir,
                &mut count,
                &mut total_bytes,
            )?;
        } else {
            copy_snapshot(
                &imported.snapshot,
                &staging_dir.join(
                    imported
                        .snapshot
                        .file_name()
                        .ok_or_else(|| invalid("Source filename is missing"))?,
                ),
                &mut count,
                &mut total_bytes,
                0,
            )?;
        }
        if !source_matches(&imported)? {
            return Err(invalid(
                "History source changed while it was being archived",
            ));
        }
        if record["audio"]["state"].as_str() == Some("retained")
            && record["provider"].as_str() != Some("voxtype")
        {
            if let Some(name) = record["audio"]["path"]
                .as_str()
                .and_then(|p| Path::new(p).file_name())
                .and_then(|n| n.to_str())
            {
                record["audio"]["path"] = json!(raw_dir.join(name));
            }
        }
        record["archive_path"] = json!(raw_dir);
        if existing.is_some() {
            let prior = self.load_current(&id)?;
            for key in [
                "loudness",
                "playback_gain",
                "_playback_stamp",
                "_playback_variants",
            ] {
                if let Some(value) = prior.get(key) {
                    record[key] = value.clone();
                }
            }
        }
        let snapshot_hash = snapshot_digest(&staging_dir)?;
        private_dir(&rec_dir.join("revisions"))?;
        atomic_json(
            &rec_dir.join("pending.json"),
            &json!({"record":record,"stage":stage_name,"snapshot_hash":snapshot_hash}),
        )?;
        finish_pending(&self.root, &id)?;
        // The loudness lens is measured when the record is selected, never
        // for every recording a capture brings in.
        self.records.insert(id.clone(), slim_record(&record));
        self.origins.insert(origin_key, id);
        index_upsert(&self.root, &record)?;
        Ok(json!({"record":public_record_at(&self.root, &record),"unchanged":false}))
    }

    fn discover(&mut self, args: &Value) -> Result<Value> {
        let path = PathBuf::from(
            args.get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("History discovery path is required"))?,
        );
        let provider = args.get("provider").and_then(Value::as_str).unwrap_or("");
        if provider == "opencode" {
            if !path.is_file() {
                return Err(invalid(
                    "History discovery source must be a file for this provider",
                ));
            }
            let imported_record = self.publish(parse_opencode(
                &path,
                &self.root,
                args["session_id"].as_str(),
            )?)?;
            let key = if imported_record["unchanged"].as_bool().unwrap_or(false) {
                "unchanged"
            } else {
                "imported"
            };
            return Ok(
                json!({"imported":if key == "imported" { vec![summary(&imported_record["record"])] } else { Vec::<Value>::new() },"unchanged":if key == "unchanged" { vec![summary(&imported_record["record"])] } else { Vec::<Value>::new() },"errors":[]}),
            );
        }
        if provider == "codex" || provider == "console" || provider == "antigravity" {
            let mut candidates = Vec::new();
            if path.is_dir() {
                collect_capture_files_for_provider(&path, &mut candidates, 0, Some(provider))?;
            } else if path.is_file() {
                candidates.push(path.clone());
            } else {
                return Err(invalid(
                    "History discovery source must be a file or directory",
                ));
            }
            let mut imported = Vec::new();
            let mut unchanged = Vec::new();
            let mut errors = Vec::new();
            for candidate in candidates {
                match self.import(&json!({"path":candidate,"provider":provider})) {
                    Ok(value) if value["unchanged"].as_bool().unwrap_or(false) => {
                        unchanged.push(summary(&value["record"]))
                    }
                    Ok(value) => imported.push(summary(&value["record"])),
                    Err(error) => errors.push(json!({"path":candidate,"error":error.to_string()})),
                }
            }
            return Ok(json!({"imported":imported,"unchanged":unchanged,"errors":errors}));
        }
        if provider == "chatgpt" {
            reject_symlink(&path)?;
            let candidate = if path.is_dir() {
                ["conversations.json", "chat.html"]
                    .iter()
                    .map(|name| path.join(name))
                    .find(|candidate| candidate.is_file())
                    .ok_or_else(|| {
                        invalid("ChatGPT directory has no conversations.json or chat.html")
                    })?
            } else {
                path
            };
            let result = self.import(&json!({"path":candidate,"provider":"chatgpt"}))?;
            let imported = result["records"].clone();
            let unchanged = result["unchanged_records"].clone();
            return Ok(json!({"imported":imported,"unchanged":unchanged,"errors":[]}));
        }
        if provider != "voxtype" {
            return Err(invalid(
                "Discovery supports providers antigravity, chatgpt, voxtype, console, and opencode",
            ));
        }
        reject_symlink(&path)?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(MAX_PAGE as u64) as usize;
        let mut candidates: Vec<(PathBuf, String)> = Vec::new();
        if path.is_dir() {
            if path.join("metadata.json").is_file() {
                candidates.push((path.clone(), String::new()));
            } else {
                let search_root = if path.join("meetings").is_dir() {
                    path.join("meetings")
                } else {
                    path.clone()
                };
                for entry in fs::read_dir(search_root)? {
                    let entry = entry?;
                    let p = entry.path();
                    if entry.file_type()?.is_dir() && p.join("metadata.json").is_file() {
                        let captured = fs::read(p.join("metadata.json"))
                            .ok()
                            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                            .and_then(|value| {
                                value_string(&value, &["started_at", "captured_at", "created_at"])
                            })
                            .unwrap_or_default();
                        candidates.push((p, captured));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        let mut imported = Vec::new();
        let mut unchanged = Vec::new();
        let mut errors = Vec::new();
        for (candidate, _) in candidates.into_iter().take(limit) {
            match self.import(&json!({"path":candidate,"provider":"voxtype"})) {
                Ok(v) => {
                    if v["unchanged"].as_bool().unwrap_or(false) {
                        unchanged.push(summary(&v["record"]));
                    } else {
                        imported.push(summary(&v["record"]));
                    }
                }
                Err(e) => errors.push(json!({"path":candidate,"error":e.to_string()})),
            }
        }
        Ok(json!({"imported":imported,"unchanged":unchanged,"errors":errors}))
    }

    fn correct(&mut self, args: &Value) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("History id is required"))?;
        let expected = args
            .get("expected_revision")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Expected history revision is required"))?;
        let current = self.load_current(id)?;
        if current.get("revision").and_then(Value::as_str) != Some(expected) {
            return Err(invalid("History revision conflict"));
        }
        // Keep the existing single-segment contract, and accept a bounded
        // atomic batch for one inline edit crossing capture chunks.
        let edits = if let Some(edits) = args.get("edits") {
            edits
                .as_array()
                .cloned()
                .ok_or_else(|| invalid("Correction edits must be an array"))?
        } else {
            vec![json!({"segment_id":args.get("segment_id"),"text":args.get("text")})]
        };
        if edits.is_empty() || edits.len() > 256 {
            return Err(invalid("Correction requires between one and 256 segments"));
        }
        let old_segments = current
            .get("segments")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("History record has no segments"))?;
        let mut replacements = BTreeMap::new();
        let mut total_bytes = 0;
        for edit in &edits {
            let segment_id = edit
                .get("segment_id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Segment id is required"))?;
            let text = edit
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Corrected text is required"))?;
            total_bytes += text.len();
            if total_bytes > MAX_TEXT {
                return Err(invalid("Corrected text is too long"));
            }
            if !old_segments
                .iter()
                .any(|s| s.get("id").and_then(Value::as_str) == Some(segment_id))
            {
                return Err(invalid("History segment not found"));
            }
            if replacements
                .insert(segment_id.to_owned(), text.to_owned())
                .is_some()
            {
                return Err(invalid("Duplicate correction segment"));
            }
        }
        // Validate the entire batch before publishing a revision. Source
        // anchors remain the original coarse capture intervals, never inferred
        // word timestamps; the receipt names every touched source interval.
        let source_intervals: Vec<Value> = old_segments
            .iter()
            .filter(|s| replacements.contains_key(s["id"].as_str().unwrap_or("")))
            .map(|s| json!({"segment_id":s["id"],"start_ms":s["start_ms"],"end_ms":s["end_ms"]}))
            .collect();
        let number = current
            .get("_revision_number")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                current
                    .get("_source_revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
            })
            + 1;
        let revision = number.to_string();
        let mut record = current.clone();
        record["revision"] = json!(revision.clone());
        record["_revision_number"] = json!(number);
        let mut segments = Vec::with_capacity(old_segments.len());
        for old in old_segments {
            let mut next = old.clone();
            if let Some(text) = replacements.get(old["id"].as_str().unwrap_or("")) {
                next["text"] = json!(text);
            }
            segments.push(next);
        }
        record["segments"] = Value::Array(segments);
        record["updated_at"] = json!(now());
        record["correction"] = json!({"author":args.get("author").cloned().unwrap_or(Value::Null),"reason":args.get("reason").cloned().unwrap_or(Value::Null),"parent_revision":expected,"created_at":now(),"source_segment_ids":source_intervals.iter().map(|s|s["segment_id"].clone()).collect::<Vec<_>>(),"source_intervals":source_intervals});
        let rec_dir = self.root.join("records").join(id).join("revisions");
        atomic_json(&rec_dir.join(format!("{revision}.json")), &record)?;
        atomic_json(
            &self.root.join("records").join(id).join("record.json"),
            &record,
        )?;
        self.records.insert(id.to_owned(), slim_record(&record));
        index_upsert(&self.root, &record)?;
        Ok(public_record_at(&self.root, &record))
    }

    fn revisions(&self, args: &Value) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("History id is required"))?;
        let current = self.load_current(id)?;
        let dir = self.root.join("records").join(id).join("revisions");
        let mut items = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_slice(&read_limited(&entry.path())?)?;
            items.push(json!({"revision":value["revision"],"created_at":value.get("updated_at").cloned().unwrap_or(value["imported_at"].clone()),"parent_revision":value.get("correction").and_then(|c| c.get("parent_revision")).cloned().unwrap_or(Value::Null),"source_hash":value.get("_source_hash").cloned().unwrap_or(Value::Null)}));
        }
        items.sort_by_key(|v| {
            v.get("revision")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0)
        });
        Ok(json!({"id":id,"current_revision":current["revision"],"items":items}))
    }
}

fn summary(record: &Value) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(fields) = record.as_object() {
        for (key, value) in fields {
            if key != "segments" && !key.starts_with('_') {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    let preview = record
        .get("segments")
        .and_then(Value::as_array)
        .and_then(|segments| {
            segments
                .iter()
                .find_map(|segment| segment.get("text").and_then(Value::as_str))
        })
        .map(|text| text.chars().take(200).collect::<String>())
        .unwrap_or_else(|| record["preview"].as_str().unwrap_or("").to_owned());
    object.insert("preview".into(), json!(preview));
    Value::Object(object)
}

/// What a capture source states: whether it is switched on, when it was last
/// checked and what that check found. A check is never reported as a sync.
fn capture_facts(source: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "id",
        "provider",
        "path",
        "enabled",
        "opt_in",
        "auto_discovered",
        "created_at",
        "removed_at",
        "last_checked_at",
        "last_error",
        "watermarks",
    ] {
        if let Some(value) = source.get(key) {
            out.insert(key.to_owned(), value.clone());
        }
    }
    out.entry("last_checked_at").or_insert(Value::Null);
    let previous = &source["last_result"];
    let count = |key: &str| previous[key].as_u64();
    let result = if previous.is_object() {
        json!({"imported":count("imported").unwrap_or(0),
            "unchanged":count("unchanged").unwrap_or(0),
            "failed":count("failed").or_else(|| count("errors")).unwrap_or(0)})
    } else {
        Value::Null
    };
    out.insert("last_result".into(), result);
    Value::Object(out)
}

/// Measured word intervals only: the text and its times. Any score a
/// recorder attaches is a placeholder and never enters a record.
fn measured_words(words: &Value) -> Value {
    match words.as_array() {
        Some(words) => Value::Array(
            words
                .iter()
                .map(|word| {
                    let mut kept = serde_json::Map::new();
                    for key in ["text", "word", "start_ms", "end_ms", "estimated"] {
                        if let Some(value) = word.get(key) {
                            kept.insert(key.to_owned(), value.clone());
                        }
                    }
                    Value::Object(kept)
                })
                .collect(),
        ),
        None => words.clone(),
    }
}

/// The listing of one completed recording read in place, with the identity
/// its record will have once opened. None when it is not completed.
fn lens_summary(folder: &Path) -> Option<Value> {
    let metadata: Value =
        serde_json::from_slice(&read_limited(&folder.join("metadata.json")).ok()?).ok()?;
    let status = value_string(&metadata, &["status", "state"])?.to_lowercase();
    if !matches!(status.as_str(), "complete" | "completed" | "finished") {
        return None;
    }
    let origin_id = value_string(&metadata, &["id", "recording_id"])
        .unwrap_or_else(|| folder.to_string_lossy().into_owned());
    let origin_key = format!("voxtype\0{origin_id}");
    let id = format!("history-{}", id_hex(origin_key.as_bytes()));
    let declared = metadata
        .get("audio_path")
        .or_else(|| metadata.get("audio"))
        .or_else(|| metadata.get("audio_file"))
        .and_then(Value::as_str)
        .map(|name| folder.join(name))
        .or_else(|| {
            fs::read_dir(folder).ok()?.flatten().find_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                (is_audio_file(&name) && entry.file_type().ok()?.is_file()).then(|| entry.path())
            })
        });
    let audio = declared
        .filter(|path| {
            path.starts_with(folder) && fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
        })
        .and_then(|path| {
            let stat = fs::metadata(&path).ok()?;
            let modified = stat
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            Some(json!({"state":"retained","path":path.to_string_lossy(),
                "source_size":stat.len(),"source_mtime_ns":modified.as_nanos().to_string()}))
        })
        .unwrap_or_else(|| json!({"state":"unavailable","reason":"Source did not retain audio"}));
    let preview = read_limited(&folder.join("transcript.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|transcript| {
            transcript["segments"]
                .as_array()?
                .iter()
                .find_map(|segment| {
                    value_string(segment, &["text", "transcript"])
                        .filter(|text| !text.trim().is_empty())
                })
        })
        .map(|text| text.chars().take(200).collect::<String>())
        .unwrap_or_default();
    let duration_ms = audio["path"]
        .as_str()
        .and_then(|path| voice_loudness::duration_ms(Path::new(path)))
        .or_else(|| value_i64(&metadata, &["duration_ms"]))
        .or_else(|| value_i64(&metadata, &["duration_secs"]).map(|x| x * 1000));
    Some(json!({
        "id": id,
        "kind": "voice_note",
        "provider": "voxtype",
        "title": value_string(&metadata, &["title", "name"]).unwrap_or_else(|| {
            folder.file_name().and_then(|x| x.to_str()).unwrap_or("Voice note").to_owned()
        }),
        "purpose": "Unclassified",
        "captured_at": value_string(&metadata, &["started_at", "captured_at", "created_at"]).unwrap_or_default(),
        "duration_ms": duration_ms,
        "audio": audio,
        "preview": preview,
        "source_refs": [{"path":folder.to_string_lossy(),"origin_id":origin_id,"provider":"voxtype"}],
        "_origin_key": origin_key,
    }))
}

fn slim_record(record: &Value) -> Value {
    let mut out = summary(record);
    if let Some(object) = out.as_object_mut() {
        for key in [
            "_origin_key",
            "_source_hash",
            "_source_revision",
            "_revision_number",
            "_adapter_version",
            "_history_relevant",
            "_playback_stamp",
        ] {
            if let Some(value) = record.get(key) {
                object.insert(key.to_owned(), value.clone());
            }
        }
    }
    out
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn index_insert_sql(record: &Value) -> String {
    let id = record.get("id").and_then(Value::as_str).unwrap_or("");
    if !chat_kinds::relevant(record) {
        return format!("DELETE FROM history_index WHERE id={};", sql_quote(id));
    }
    let kind = record.get("kind").and_then(Value::as_str).unwrap_or("");
    let captured = record
        .get("captured_at")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut search_text = ["title", "purpose", "provider", "kind"]
        .iter()
        .filter_map(|key| record[*key].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(segments) = record["segments"].as_array() {
        for segment in segments {
            if let Some(text) = segment["text"].as_str() {
                search_text.push('\n');
                search_text.push_str(text);
            }
        }
    }
    let search_text = search_text.to_lowercase().replace('\0', " ");
    let summary_json = serde_json::to_string(&summary(record)).unwrap_or_else(|_| "{}".into());
    format!(
        "INSERT OR REPLACE INTO history_index(id,kind,captured_at,search_text,summary_json) VALUES ({},{},{},{},{});",
        sql_quote(id),
        sql_quote(kind),
        sql_quote(captured),
        sql_quote(&search_text),
        sql_quote(&summary_json)
    )
}

fn index_upsert(root: &Path, record: &Value) -> Result<()> {
    let path = root.join("history-index.sqlite3");
    reject_symlink(&path)?;
    let script = format!(
        "BEGIN; CREATE TABLE IF NOT EXISTS history_index (id TEXT PRIMARY KEY, kind TEXT NOT NULL, captured_at TEXT NOT NULL, search_text TEXT NOT NULL, summary_json TEXT NOT NULL); {}; COMMIT;",
        index_insert_sql(record)
    );
    sqlite_script(&path, &script, 192 * 1024 * 1024)?;
    private_file(&path)
}

fn index_search(
    root: &Path,
    kind: &str,
    provider: &str,
    query: &str,
    state: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<Value>, usize)> {
    let path = root.join("history-index.sqlite3");
    let kind_clause = if kind == "all" {
        String::new()
    } else {
        format!(" AND kind={}", sql_quote(kind))
    };
    let provider_clause = if provider.is_empty() {
        String::new()
    } else {
        format!(
            " AND json_extract(summary_json, '$.provider')={}",
            sql_quote(provider)
        )
    };
    let match_clause = format!(
        "kind IS NOT NULL{}{} AND instr(search_text, {}) > 0 AND COALESCE(json_extract(summary_json, '$.record_state'), 'active') = {}",
        kind_clause,
        provider_clause,
        sql_quote(&query.to_lowercase().replace('\0', " ")),
        sql_quote(state)
    );
    let count = sqlite_json(
        &path,
        &format!("SELECT COUNT(*) AS total FROM history_index WHERE {match_clause}"),
    )?;
    let total = count
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let rows = sqlite_json(
        &path,
        &format!(
            "SELECT summary_json FROM history_index WHERE {match_clause} ORDER BY captured_at DESC, id ASC LIMIT {limit} OFFSET {offset}"
        ),
    )?;
    let mut items = Vec::new();
    for row in rows
        .as_array()
        .ok_or_else(|| invalid("History index returned malformed rows"))?
    {
        let text = row
            .get("summary_json")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("History index row is missing its summary"))?;
        let summary: Value = serde_json::from_str(text)
            .map_err(|e| invalid(format!("Corrupt history index summary: {e}")))?;
        items.push(public_record_at(root, &summary));
    }
    Ok((items, total))
}

fn public_record(record: &Value) -> Value {
    let mut out = record.clone();
    if let Some(object) = out.as_object_mut() {
        object.retain(|key, _| !key.starts_with('_'));
    }
    out
}

/// Resolve paths into the archive copied for this HistoryStore without
/// rewriting source_refs. Source provenance intentionally remains the path
/// that was captured; runtime playback and replay must use the durable local
/// snapshot instead of the source workspace from which it was imported.
fn rebase_archive_path(root: &Path, value: &str) -> String {
    let marker = "/history/raw/";
    if let Some(suffix) = value.split_once(marker).map(|(_, suffix)| suffix) {
        return root.join("raw").join(suffix).to_string_lossy().into_owned();
    }
    if let Some(suffix) = value.strip_prefix("raw/") {
        return root.join("raw").join(suffix).to_string_lossy().into_owned();
    }
    value.to_owned()
}

fn public_record_at(root: &Path, record: &Value) -> Value {
    let mut out = public_record(record);
    if let Some(object) = out.as_object_mut() {
        if let Some(audio) = object.get_mut("audio").and_then(Value::as_object_mut)
            && let Some(path) = audio.get("path").and_then(Value::as_str).map(str::to_owned)
        {
            let mut resolved = rebase_archive_path(root, &path);
            // An archived recording copy that is absent here (a copy of the
            // workspace without it) is read from the original in place.
            if !Path::new(&resolved).is_file()
                && record["provider"] == "voxtype"
                && let Some(original) = record["source_refs"][0]["path"]
                    .as_str()
                    .zip(Path::new(&path).file_name())
                    .map(|(folder, name)| Path::new(folder).join(name))
                    .filter(|original| {
                        fs::symlink_metadata(original).is_ok_and(|m| m.file_type().is_file())
                    })
            {
                resolved = original.to_string_lossy().into_owned();
            }
            audio.insert("path".into(), json!(resolved));
            if audio.get("source_size").is_some() {
                let current = fs::metadata(&resolved).ok();
                let modified = current
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|t| t.as_nanos().to_string());
                let changed = current
                    .as_ref()
                    .is_some_and(|m| Some(m.len()) != audio["source_size"].as_u64())
                    || modified.as_deref() != audio["source_mtime_ns"].as_str();
                if current.is_none() || changed {
                    audio.insert("state".into(), json!("unavailable"));
                    audio.insert(
                        "reason".into(),
                        json!(if current.is_none() {
                            "Canonical audio is missing"
                        } else {
                            "Canonical audio changed since import; re-import to verify it"
                        }),
                    );
                    audio.remove("path");
                }
            }
        }
        if let Some(path) = object
            .get("archive_path")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            object.insert(
                "archive_path".into(),
                json!(rebase_archive_path(root, &path)),
            );
        }
    }
    out
}

fn directory_source_hash(path: &Path) -> Result<String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        reject_symlink(&entry.path())?;
        if entry.file_type()?.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            check_component_name(&name)?;
            if voxtype_file_admitted(&name) {
                files.push((name, entry.path()));
            }
        }
    }
    files.sort_by(|a, b| {
        is_audio_file(&a.0)
            .cmp(&is_audio_file(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut digest = Sha256::new();
    let mut total = 0u64;
    for (name, file) in files {
        let metadata = fs::metadata(&file)?;
        if is_audio_file(&name) {
            // Media identity is a bounded stat fingerprint. Never read a full
            // WAV merely to decide whether the transcript changed.
            digest.update(name.as_bytes());
            digest.update(metadata.len().to_le_bytes());
            let stamp = metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            digest.update(stamp.as_nanos().to_le_bytes());
            continue;
        }
        let next_total = total
            .checked_add(metadata.len())
            .ok_or_else(|| invalid("History source size overflow"))?;
        if next_total > MAX_SNAPSHOT_BYTES {
            if is_audio_file(&name) {
                continue;
            }
            return Err(invalid("History source snapshot exceeds the size limit"));
        }
        total = next_total;
        let mut file_digest = Sha256::new();
        hash_file_into(&file, &mut file_digest, MAX_BYTES)?;
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(file_digest.finalize());
        digest.update([0]);
    }
    Ok(ContentHash::from(<[u8; 32]>::from(digest.finalize())).to_string())
}

fn voxtype_file_admitted(name: &str) -> bool {
    if matches!(name, "metadata.json" | "transcript.json") {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    [".wav", ".mp3", ".ogg", ".flac", ".m4a", ".opus"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn is_audio_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".wav", ".mp3", ".ogg", ".flac", ".m4a", ".opus"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn collect_capture_files_for_provider(
    root: &Path,
    files: &mut Vec<PathBuf>,
    depth: usize,
    provider: Option<&str>,
) -> Result<()> {
    // Antigravity's brain directory is an artifact tree. Its retained source
    // is deliberately limited to brain/<conversation>/.system_generated/logs/
    // transcript.jsonl; walking every artifact first lets unrelated deep
    // trees fail the generic nesting bound before the useful source is seen.
    // Keep this allowlist structural so a large or private artifact tree is
    // never traversed as part of capture.
    if provider == Some("antigravity") {
        return collect_antigravity_transcripts(root, files);
    }
    if depth > 8 {
        return Err(invalid("Capture directory nesting exceeds the bound"));
    }
    reject_symlink(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid("Capture directories cannot contain symlinks"));
        }
        if kind.is_dir() {
            collect_capture_files_for_provider(&path, files, depth + 1, provider)?;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let admitted = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "jsonl" | "ndjson" | "json" | "log" | "txt"
                )
            });
        if admitted && provider.is_none_or(|name| capture_file_admitted(name, &path)) {
            files.push(path);
            if files.len() > 2_000 {
                return Err(invalid(
                    "Capture directory contains more than 2,000 candidate files",
                ));
            }
        }
    }
    files.sort();
    Ok(())
}

fn collect_antigravity_transcripts(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    reject_symlink(root)?;
    let mut candidates = Vec::new();

    // Accept callers that point directly at a transcript, a conversation
    // directory, or the brain directory. The brain case only examines its
    // immediate conversation children and never descends into artifact trees.
    candidates.push(root.join("transcript.jsonl"));
    candidates.push(root.join(".system_generated/logs/transcript.jsonl"));
    let mut children = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(invalid("Capture directories cannot contain symlinks"));
        }
        if kind.is_dir() {
            candidates.push(path.join(".system_generated/logs/transcript.jsonl"));
        }
    }

    candidates.sort();
    candidates.dedup();
    for candidate in candidates {
        reject_symlink(&candidate)?;
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => {
                files.push(candidate);
                if files.len() > 2_000 {
                    return Err(invalid(
                        "Capture directory contains more than 2,000 candidate files",
                    ));
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn result_count(items: &Value, flag: &Value) -> usize {
    items.as_array().map_or_else(
        || {
            flag.as_array()
                .map_or_else(|| usize::from(flag.as_bool().unwrap_or(false)), Vec::len)
        },
        Vec::len,
    )
}

fn capture_fingerprint(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos());
    Ok(format!("{}:{}", metadata.len(), modified))
}

fn capture_metadata_fingerprint(metadata: &fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}:{}:{}",
            metadata.len(),
            modified,
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    }
    #[cfg(not(unix))]
    format!("{}:{}", metadata.len(), modified)
}

/// Return a cheap recording fingerprint for the automatic capture pass.
///
/// The full source digest is still computed when a recording changes and is
/// still used for publication idempotence. Automatic checks only need to know
/// whether any admitted file could have changed, so metadata avoids rereading
/// large retained audio attachments on every timer tick.
fn capture_directory_fingerprint(path: &Path) -> Result<String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !voxtype_file_admitted(&name) {
            continue;
        }
        reject_symlink(&entry.path())?;
        let metadata = fs::metadata(entry.path())?;
        if !metadata.is_file() {
            continue;
        }
        entries.push(format!(
            "{name}:{}",
            capture_metadata_fingerprint(&metadata)
        ));
    }
    entries.sort();
    Ok(entries.join("\0"))
}

fn capture_file_admitted(provider: &str, path: &Path) -> bool {
    if provider == "antigravity" {
        return path.file_name().and_then(|name| name.to_str()) == Some("transcript.jsonl");
    }
    true
}

fn copy_voxtype_snapshot(
    source: &Path,
    destination: &Path,
    count: &mut usize,
    total_bytes: &mut u64,
) -> Result<()> {
    private_dir(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by(|a, b| {
        let a_name = a.file_name().to_string_lossy().into_owned();
        let b_name = b.file_name().to_string_lossy().into_owned();
        is_audio_file(&a_name)
            .cmp(&is_audio_file(&b_name))
            .then_with(|| a_name.cmp(&b_name))
    });
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        check_component_name(&name)?;
        reject_symlink(&entry.path())?;
        if !entry.file_type()?.is_file() || !voxtype_file_admitted(&name) {
            continue;
        }
        let metadata = fs::metadata(entry.path())?;
        let target = destination.join(&name);
        if is_audio_file(&name) {
            // The original remains the audio owner. Staging and archive only
            // contain the small transcript and capture metadata.
            continue;
        } else {
            let bytes = read_limited(&entry.path())?;
            atomic_bytes(&target, &bytes)?;
        }
        *total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| invalid("History source size overflow"))?;
        if *total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(invalid("History source snapshot exceeds the size limit"));
        }
        *count += 1;
        if *count > 512 {
            return Err(invalid("History source contains too many files"));
        }
    }
    Ok(())
}

fn copy_opencode_snapshot(
    source: &Path,
    destination: &Path,
    count: &mut usize,
    total_bytes: &mut u64,
) -> Result<()> {
    private_dir(destination)?;
    reject_symlink(source)?;
    *total_bytes = fs::metadata(source)?.len();
    if *total_bytes > MAX_DB_BYTES {
        return Err(invalid("Database snapshot exceeds limit"));
    }
    // A completed SQLite backup is immutable and already private. Link its
    // bytes instead of copying the full database once per conversation.
    fs::hard_link(source, destination.join("opencode.db"))?;
    File::open(destination.join("opencode.db"))?.sync_all()?;
    File::open(destination)?.sync_all()?;
    *count = 1;
    Ok(())
}

fn source_matches(imported: &Imported) -> Result<bool> {
    let hash = if imported.record["provider"].as_str() == Some("soliton-inbox") {
        file_digest(&imported.snapshot, 1024 * 1024 * 1024)?
    } else if imported.record["provider"].as_str() == Some("opencode") {
        file_digest(&imported.snapshot, MAX_DB_BYTES)?
    } else if imported.snapshot.is_dir() {
        directory_source_hash(&imported.snapshot)?
    } else {
        ContentHash::digest(&read_limited(&imported.snapshot)?).to_string()
    };
    Ok(hash
        == imported.record["_raw_database_hash"]
            .as_str()
            .or_else(|| imported.record["_normalized_file_hash"].as_str())
            .unwrap_or(&imported.source_hash))
}

fn parse_voxtype(path: &Path) -> Result<Imported> {
    if !path.is_dir() {
        return Err(invalid(
            "Voxtype import must be a completed recording directory",
        ));
    }
    let metadata_path = path.join("metadata.json");
    let transcript_path = path.join("transcript.json");
    let metadata_before = fs::metadata(&metadata_path)?;
    let transcript_before = fs::metadata(&transcript_path)?;
    let metadata_bytes = read_limited(&metadata_path)?;
    let transcript_bytes = read_limited(&transcript_path)?;
    let metadata_after = fs::metadata(&metadata_path)?;
    let transcript_after = fs::metadata(&transcript_path)?;
    if metadata_before.len() != metadata_after.len()
        || transcript_before.len() != transcript_after.len()
        || metadata_before.modified().ok() != metadata_after.modified().ok()
        || transcript_before.modified().ok() != transcript_after.modified().ok()
    {
        return Err(invalid(
            "Voxtype recording changed while it was being archived",
        ));
    }
    let metadata: Value = serde_json::from_slice(&metadata_bytes)
        .map_err(|e| invalid(format!("Malformed Voxtype metadata: {e}")))?;
    let transcript: Value = serde_json::from_slice(&transcript_bytes)
        .map_err(|e| invalid(format!("Malformed Voxtype transcript: {e}")))?;
    let status = value_string(&metadata, &["status", "state"])
        .unwrap_or_default()
        .to_lowercase();
    if !status.is_empty() && !matches!(status.as_str(), "complete" | "completed" | "finished") {
        return Err(invalid("Voxtype recording is not complete"));
    }
    let segments_in = transcript
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Voxtype transcript has no segments array"))?;
    if segments_in.len() > MAX_SEGMENTS {
        return Err(invalid("Voxtype transcript has too many segments"));
    }
    let origin_id = value_string(&metadata, &["id", "recording_id"])
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("voxtype\0{origin_id}").as_bytes())
    );
    let revision = "1";
    let mut segments = Vec::with_capacity(segments_in.len());
    for (index, item) in segments_in.iter().enumerate() {
        let text = value_string(item, &["text", "transcript"]).unwrap_or_default();
        if text.len() > MAX_TEXT {
            return Err(invalid("Voxtype segment is too large"));
        }
        segments.push(segment(
            &record_id_hint,
            revision,
            index,
            item.get("id"),
            value_i64(item, &["start_ms", "start"]),
            value_i64(item, &["end_ms", "end"]),
            text,
            value_string(item, &["speaker_id", "speaker"]),
        ));
        if let Some(words) = item.get("words")
            && let Some(last) = segments.last_mut()
        {
            last["words"] = measured_words(words);
        }
    }
    let metadata_duration_ms = value_i64(&metadata, &["duration_ms"])
        .or_else(|| value_i64(&metadata, &["duration_secs"]).map(|x| x * 1000));
    let title = value_string(&metadata, &["title", "name"]).unwrap_or_else(|| {
        path.file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("Voice note")
            .to_owned()
    });
    let captured_at =
        value_string(&metadata, &["started_at", "captured_at", "created_at"]).unwrap_or_else(now);
    let declared_audio = metadata
        .get("audio_path")
        .or_else(|| metadata.get("audio"))
        .or_else(|| metadata.get("audio_file"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let detected_audio = declared_audio.or_else(|| {
        fs::read_dir(path)
            .ok()?
            .filter_map(|entry| entry.ok())
            .find_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().ok()?.is_file()
                    && voxtype_file_admitted(&name)
                    && name != "metadata.json"
                    && name != "transcript.json"
                {
                    Some(entry.path())
                } else {
                    None
                }
            })
    });
    let unavailable_marker = fs::read(path.join(AUDIO_UNAVAILABLE_MARKER))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|value| value["state"] == "unavailable");
    let audio = if let Some(candidate) = detected_audio {
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            path.join(candidate)
        };
        let source_root = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let candidate_allowed = fs::canonicalize(&candidate)
            .ok()
            .is_some_and(|resolved| resolved.starts_with(&source_root));
        let too_large = fs::metadata(&candidate)
            .ok()
            .is_some_and(|metadata| metadata.len() > MAX_AUDIO_BYTES);
        if too_large {
            json!({"state":"unavailable","reason":"Audio attachment exceeds the bounded archive limit"})
        } else if candidate.is_file()
            && !fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink())
            && candidate_allowed
            && File::open(&candidate).is_ok()
        {
            let stamp = fs::metadata(&candidate)?;
            let modified = stamp
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            json!({"state":"retained","path":candidate.to_string_lossy(),
                "source_size":stamp.len(),"source_mtime_ns":modified.as_nanos().to_string()})
        } else {
            json!({"state":"unavailable","reason":"Voxtype declared audio retention but the audio file is missing or unreadable"})
        }
    } else if let Some(marker) = unavailable_marker {
        marker
    } else if metadata.get("audio_retained").and_then(Value::as_bool) == Some(true) {
        json!({"state":"unavailable","reason":"Voxtype declared audio retention but the audio file is missing or unreadable"})
    } else {
        json!({"state":"unavailable","reason":"Source did not retain audio"})
    };
    let has_gap = segments.windows(2).any(|pair| {
        match (
            pair[0].get("end_ms").and_then(Value::as_i64),
            pair[1].get("start_ms").and_then(Value::as_i64),
        ) {
            (Some(end), Some(start)) => start > end,
            _ => false,
        }
    });
    let coverage = if segments.is_empty() {
        json!({"state":"partial","reason":"Transcript contains no segments"})
    } else if has_gap {
        json!({"state":"partial","reason":"Transcript contains a time gap between recorded segment ranges"})
    } else {
        json!({"state":"complete"})
    };
    // The recording's own length; the recorder's rounded figure only without one.
    let duration_ms = audio["path"]
        .as_str()
        .and_then(|path| voice_loudness::duration_ms(Path::new(path)))
        .or(metadata_duration_ms);
    let record = json!({"id":record_id_hint,"revision":revision,"title":title,"kind":"voice_note","provider":"voxtype","captured_at":captured_at,"imported_at":now(),"duration_ms":duration_ms,"audio":audio,"coverage":coverage,"segments":segments,"source_refs":[{"path":path.to_string_lossy(),"origin_id":origin_id}],"related":[]});
    Ok(Imported {
        origin_id,
        source_hash: directory_source_hash(path)?,
        record,
        snapshot: path.to_path_buf(),
    })
}

fn parse_manual(path: &Path) -> Result<Imported> {
    let bytes = read_limited(path)?;
    let text = parse_text(&bytes, "Manual history source")?;
    let origin_id = fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("manual\0{origin_id}").as_bytes())
    );
    let title = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|x| x.trim_start_matches('#').trim().to_owned())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("Manual import")
                .to_owned()
        });
    let segments = vec![segment(
        &record_id_hint,
        "1",
        0,
        None,
        Some(0),
        None,
        text,
        None,
    )];
    let record = json!({"id":record_id_hint,"revision":"1","title":title,"kind":"chat","provider":"manual","captured_at":now(),"imported_at":now(),"duration_ms":null,"audio":{"state":"unavailable","reason":"Manual source has no audio"},"coverage":{"state":"complete"},"segments":segments,"source_refs":[{"path":path.to_string_lossy(),"origin_id":origin_id}],"related":[]});
    Ok(Imported {
        origin_id,
        source_hash: ContentHash::digest(&bytes).to_string(),
        record,
        snapshot: path.to_path_buf(),
    })
}

fn chatgpt_timestamp(value: Option<&Value>) -> Option<String> {
    let seconds = value.and_then(|value| value.as_f64())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let whole = seconds.trunc() as i64;
    let nanos = ((seconds.fract() * 1_000_000_000.0).round() as u32).min(999_999_999);
    chrono::DateTime::<chrono::Utc>::from_timestamp(whole, nanos)
        .map(|date| date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn chatgpt_content(value: &Value) -> String {
    let content = value.get("content").unwrap_or(&Value::Null);
    let parts = content
        .get("parts")
        .cloned()
        .unwrap_or_else(|| content.clone());
    match parts {
        Value::String(text) => text,
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                Value::Object(object) => {
                    let kind = object.get("content_type").and_then(Value::as_str);
                    if matches!(
                        kind,
                        Some("thoughts" | "reasoning" | "execution_output" | "tool_result")
                    ) {
                        None
                    } else {
                        object.get("text").and_then(Value::as_str)
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    }
}

fn chatgpt_json(path: &Path, original: &str) -> Result<Vec<Imported>> {
    let bytes = read_limited(path)?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| invalid(format!("Malformed ChatGPT conversations JSON: {e}")))?;
    let conversations = value
        .as_array()
        .or_else(|| value.get("conversations").and_then(Value::as_array))
        .ok_or_else(|| invalid("ChatGPT export must contain a conversations array"))?;
    if conversations.len() > MAX_RECORDS {
        return Err(invalid("ChatGPT export contains too many conversations"));
    }
    let source_hash = ContentHash::digest(&bytes).to_string();
    let mut output = Vec::new();
    for conversation in conversations {
        let conversation_id = value_string(conversation, &["conversation_id", "id"])
            .ok_or_else(|| invalid("ChatGPT conversation has no stable id"))?;
        let record_id_hint = format!(
            "history-{}",
            id_hex(format!("chatgpt\0{original}\0{conversation_id}").as_bytes())
        );
        let title = value_string(conversation, &["title", "name"])
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "ChatGPT conversation".to_owned());
        let mapping = conversation
            .get("mapping")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("ChatGPT conversation has no mapping"))?;
        if mapping.len() > MAX_SEGMENTS.saturating_mul(2) {
            return Err(invalid("ChatGPT conversation mapping is too large"));
        }
        let mut nodes = mapping
            .iter()
            .filter_map(|(node_id, node)| {
                let message = node.get("message")?;
                let role = message.get("author")?.get("role")?.as_str()?;
                if !matches!(role, "user" | "assistant") {
                    return None;
                }
                let text = chatgpt_content(message);
                if text.is_empty() {
                    return None;
                }
                Some((
                    node_id.clone(),
                    node.get("parent").cloned().unwrap_or(Value::Null),
                    node.get("children").cloned().unwrap_or_else(|| json!([])),
                    role.to_owned(),
                    text,
                    chatgpt_timestamp(message.get("create_time")),
                    value_string(message, &["id"]),
                ))
            })
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.5.cmp(&right.5).then_with(|| left.0.cmp(&right.0)));
        if nodes.len() > MAX_SEGMENTS {
            return Err(invalid(
                "ChatGPT conversation has too many visible messages",
            ));
        }
        if nodes.is_empty() {
            continue;
        }
        let mut segments = Vec::with_capacity(nodes.len());
        for (index, (node_id, parent, children, role, text, timestamp, message_id)) in
            nodes.iter().enumerate()
        {
            let mut item = segment(
                &record_id_hint,
                "1",
                index,
                Some(&Value::String(
                    message_id.clone().unwrap_or_else(|| node_id.clone()),
                )),
                None,
                None,
                text.clone(),
                Some(role.clone()),
            );
            item["node_id"] = json!(node_id);
            item["parent_id"] = parent.clone();
            item["child_ids"] = children.clone();
            if let Some(time) = timestamp {
                item["captured_at"] = json!(time);
            }
            segments.push(item);
        }
        let skipped = mapping.len().saturating_sub(nodes.len());
        let coverage = if skipped > 0 {
            json!({"state":"partial","reason":format!("{} non-visible or empty ChatGPT mapping nodes were omitted; raw export is retained", skipped)})
        } else {
            json!({"state":"complete"})
        };
        let captured_at = value_string(conversation, &["create_time", "update_time"])
            .or_else(|| chatgpt_timestamp(conversation.get("create_time")))
            .unwrap_or_else(now);
        let purpose = conversation
            .get("purpose")
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 128)
            .unwrap_or("Unclassified");
        let record = json!({
            "id": record_id_hint,
            "revision": "1",
            "title": title,
            "purpose": purpose,
            "kind": "chat",
            "provider": "chatgpt",
            "captured_at": captured_at,
            "imported_at": now(),
            "duration_ms": null,
            "audio": {"state":"unavailable","reason":"Chat source has no audio"},
            "coverage": coverage,
            "segments": segments,
            "source_refs": [{"path":original,"origin_id":conversation_id}],
            "related": []
        });
        let mut imported = Imported {
            origin_id: format!("{original}\0{conversation_id}"),
            source_hash: source_hash.clone(),
            record,
            snapshot: path.to_path_buf(),
        };
        imported.record["_normalized_file_hash"] = json!(source_hash);
        output.push(imported);
    }
    if output.is_empty() {
        return Err(invalid("ChatGPT export contains no visible conversations"));
    }
    Ok(output)
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

fn html_text(value: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for character in value.chars() {
        if character == '<' {
            in_tag = true;
        } else if character == '>' {
            in_tag = false;
            text.push(' ');
        } else if !in_tag {
            text.push(character);
        }
    }
    html_unescape(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_chatgpt_html(path: &Path, original: &str) -> Result<Vec<Imported>> {
    let bytes = read_limited(path)?;
    let source = parse_text(&bytes, "ChatGPT HTML export")?;
    let lower = source.to_ascii_lowercase();
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = lower[cursor..]
        .find("<article")
        .or_else(|| lower[cursor..].find("<div"))
    {
        let start = cursor + relative;
        let tag_end = match lower[start..].find('>') {
            Some(value) => start + value,
            None => break,
        };
        let opening = &lower[start..=tag_end];
        let role = if opening.contains("data-message-author-role=\"user\"")
            || opening.contains("data-role=\"user\"")
            || opening.contains("class=\"user\"")
        {
            "user"
        } else if opening.contains("data-message-author-role=\"assistant\"")
            || opening.contains("data-role=\"assistant\"")
            || opening.contains("class=\"assistant\"")
        {
            "assistant"
        } else {
            cursor = tag_end + 1;
            continue;
        };
        let tag_name = if opening.starts_with("<article") {
            "</article>"
        } else {
            "</div>"
        };
        let body_start = tag_end + 1;
        let body_end = match lower[body_start..].find(tag_name) {
            Some(value) => body_start + value,
            None => return Err(invalid("ChatGPT HTML message block is incomplete")),
        };
        let body = html_text(&source[body_start..body_end]);
        if !body.is_empty() {
            segments.push((role.to_owned(), body));
        }
        cursor = body_end + tag_name.len();
        if segments.len() > MAX_SEGMENTS {
            return Err(invalid("ChatGPT HTML export has too many visible messages"));
        }
    }
    if segments.is_empty() {
        return Err(invalid(
            "ChatGPT HTML has no explicit visible user/assistant message blocks; use conversations.json",
        ));
    }
    let id = format!("html-{}", id_hex(original.as_bytes()));
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("chatgpt\0{original}\0{id}").as_bytes())
    );
    let title = source
        .split_once("<title>")
        .and_then(|(_, tail)| {
            tail.split_once("</title>")
                .map(|(title, _)| html_text(title))
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "ChatGPT HTML export".to_owned());
    let normalized = ContentHash::digest(&bytes).to_string();
    let values = segments
        .into_iter()
        .enumerate()
        .map(|(index, (role, text))| {
            segment(
                &record_id_hint,
                "1",
                index,
                None,
                None,
                None,
                text,
                Some(role),
            )
        })
        .collect::<Vec<_>>();
    let mut record = json!({
        "id":record_id_hint,
        "revision":"1",
        "title":title,
        "purpose":"Unclassified",
        "kind":"chat",
        "provider":"chatgpt",
        "captured_at":now(),
        "imported_at":now(),
        "duration_ms":null,
        "audio":{"state":"unavailable","reason":"Chat source has no audio"},
        "coverage":{"state":"partial","reason":"HTML export has visible message blocks but no provider timestamps or hidden-node coverage"},
        "segments":values,
        "source_refs":[{"path":original,"origin_id":id}],
        "related":[]
    });
    record["_normalized_file_hash"] = json!(normalized.clone());
    Ok(vec![Imported {
        origin_id: format!("{original}\0{id}"),
        source_hash: normalized,
        record,
        snapshot: path.to_path_buf(),
    }])
}

fn sqlite_json(path: &Path, sql: &str) -> Result<Value> {
    let output = sqlite_output(
        &[
            "-readonly",
            "-json",
            path.to_str()
                .ok_or_else(|| invalid("Database path is not UTF-8"))?,
            sql,
        ],
        MAX_SQL_OUTPUT,
    )?;
    // sqlite3 -json prints no bytes for a successful SELECT with zero rows.
    // The process status was already checked by sqlite_output; this is not
    // malformed data or permission failure.
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!([]));
    }
    serde_json::from_slice(&output)
        .map_err(|e| invalid(format!("History SQLite query returned malformed JSON: {e}")))
}

fn opencode_backup(source: &Path, root: &Path) -> Result<PathBuf> {
    reject_symlink(source)?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", source.to_string_lossy(), suffix));
        if sidecar.exists() {
            reject_symlink(&sidecar)?;
        }
    }
    let meta = fs::metadata(source)?;
    if !meta.is_file() || meta.len() > MAX_DB_BYTES {
        return Err(invalid(
            "OpenCode database is missing or exceeds the size limit",
        ));
    }
    let backup_dir = root.join("staging").join("opencode");
    private_dir(&backup_dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = backup_dir.join(format!("snapshot-{stamp}.db"));
    let command = format!(".backup '{}'", backup.to_string_lossy().replace('\'', "''"));
    sqlite_output(
        &[
            "-readonly",
            source
                .to_str()
                .ok_or_else(|| invalid("Database path is not UTF-8"))?,
            &command,
        ],
        65536,
    )?;
    let metadata = fs::metadata(&backup)?;
    if !metadata.is_file() || metadata.len() > MAX_DB_BYTES {
        return Err(invalid("OpenCode snapshot exceeds size limit"));
    }
    private_file(&backup)?;
    File::open(&backup)?.sync_all()?;
    let digest = file_digest(&backup, MAX_DB_BYTES)?;
    let shared = root.join("databases");
    private_dir(&shared)?;
    let target = shared.join(format!("{}.db", digest.trim_start_matches("sha256:")));
    if target.exists() {
        if file_digest(&target, MAX_DB_BYTES)? != digest {
            return Err(invalid("Archived database hash mismatch"));
        }
        fs::remove_file(&backup)?;
    } else {
        fs::rename(&backup, &target)?;
        File::open(&shared)?.sync_all()?;
    }
    Ok(target)
}

fn sqlite_time(ms: Option<i64>) -> String {
    ms.and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(now)
}

fn parse_opencode(path: &Path, root: &Path, session_id: Option<&str>) -> Result<Imported> {
    let snapshot = opencode_backup(path, root)?;
    let sql = "WITH target AS (SELECT id, parent_id, title, time_created, time_updated FROM session ORDER BY time_updated DESC LIMIT 1) SELECT t.id AS session_id, t.parent_id, t.title, t.time_created AS session_created, t.time_updated AS session_updated, m.id AS message_id, m.time_created AS message_time, json_extract(m.data,'$.role') AS role, json_extract(p.data,'$.text') AS text, EXISTS (SELECT 1 FROM part px WHERE px.message_id=m.id AND json_extract(px.data,'$.type') <> 'text') AS omitted FROM target t JOIN message m ON m.session_id=t.id JOIN part p ON p.message_id=m.id WHERE json_extract(m.data,'$.role') IN ('user','assistant') AND json_extract(p.data,'$.type')='text' ORDER BY m.time_created, p.time_created";
    let selected = if let Some(id) = session_id {
        if id.len() > 256 || id.is_empty() {
            return Err(invalid("Invalid OpenCode session id"));
        }
        sql.replace(
            "ORDER BY time_updated DESC LIMIT 1",
            &format!("WHERE id='{}' LIMIT 1", id.replace('\'', "''")),
        )
    } else {
        sql.to_owned()
    };
    let rows = sqlite_json(&snapshot, &selected)?;
    let rows = rows
        .as_array()
        .ok_or_else(|| invalid("OpenCode SQLite result is not an array"))?;
    if rows.len() > MAX_SEGMENTS {
        return Err(invalid("OpenCode conversation exceeds the segment limit"));
    }
    let first = rows
        .first()
        .ok_or_else(|| invalid("OpenCode database has no visible conversation messages"))?;
    let session_id = first
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("OpenCode session has no stable id"))?;
    let origin = format!(
        "{}\0{}",
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display(),
        session_id
    );
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("opencode\0{origin}").as_bytes())
    );
    let mut segments = Vec::new();
    let mut first_time = None;
    let mut last_time = None;
    let mut omitted = false;
    for (index, row) in rows.iter().enumerate() {
        let text = row.get("text").and_then(Value::as_str).unwrap_or("");
        if text.len() > MAX_TEXT {
            return Err(invalid("OpenCode message is too large"));
        }
        let role = row
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let raw = row.get("message_id").cloned().unwrap_or(Value::Null);
        let time = row.get("message_time").and_then(Value::as_i64);
        first_time = first_time.or(time);
        last_time = time.or(last_time);
        omitted |= row
            .get("omitted")
            .and_then(|value| value.as_bool().or_else(|| value.as_i64().map(|n| n != 0)))
            .unwrap_or(false);
        segments.push(segment(
            &record_id_hint,
            "1",
            index,
            Some(&raw),
            None,
            None,
            text.to_owned(),
            Some(role),
        ));
    }
    let parent = first.get("parent_id").cloned().unwrap_or(Value::Null);
    let title = first
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("OpenCode conversation");
    let coverage = if omitted {
        json!({"state":"partial","reason":"OpenCode reasoning and tool parts are preserved in the raw snapshot but omitted from visible messages"})
    } else {
        json!({"state":"complete"})
    };
    let mut record = json!({"id":record_id_hint,"revision":"1","title":title,"kind":"chat","provider":"opencode","captured_at":sqlite_time(first_time.or_else(|| first.get("session_created").and_then(Value::as_i64))),"imported_at":now(),"duration_ms":first_time.zip(last_time).map(|(a,b)| b.saturating_sub(a)),"audio":{"state":"unavailable","reason":"Chat source has no audio"},"coverage":coverage,"segments":segments,"source_refs":[{"path":path.to_string_lossy(),"origin_id":session_id,"parent_id":parent}],"related":[]});
    record["_raw_database_hash"] = json!(file_digest(&snapshot, MAX_DB_BYTES)?);
    Ok(Imported {
        origin_id: origin,
        source_hash: ContentHash::digest(&serde_json::to_vec(rows)?).to_string(),
        record,
        snapshot,
    })
}

fn parse_codex(path: &Path) -> Result<Imported> {
    parse_jsonl(path, "codex")
}

fn parse_console(path: &Path) -> Result<Imported> {
    parse_jsonl(path, "console")
}

fn antigravity_origin(path: &Path) -> String {
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        if component.as_os_str() == "brain"
            && let Some(id) = components.get(index + 1)
        {
            return id.as_os_str().to_string_lossy().into_owned();
        }
    }
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn redact_antigravity_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let sensitive = [
            "authorization:",
            "bearer ",
            "api_key=",
            "api-key=",
            "apikey=",
            "password=",
            "secret=",
            "token=",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        if sensitive {
            output.push_str("[redacted sensitive tool output]");
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    if !text.ends_with('\n') {
        output.pop();
    }
    output
}

fn antigravity_user_text(content: &str) -> String {
    let body = content
        .split_once("<USER_REQUEST>")
        .map(|(_, value)| {
            value
                .split_once("</USER_REQUEST>")
                .map_or(value, |(body, _)| body)
        })
        .unwrap_or(content);
    body.split_once("<ADDITIONAL_METADATA>")
        .map_or_else(|| body.to_owned(), |(body, _)| body.to_owned())
        .trim()
        .to_owned()
}

fn parse_antigravity(path: &Path) -> Result<Imported> {
    let bytes = read_limited(path)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| invalid("Antigravity transcript is not UTF-8"))?;
    let origin_id = antigravity_origin(path);
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("antigravity\0{origin_id}").as_bytes())
    );
    let mut segments = Vec::new();
    let mut partial_tail = false;
    let mut omitted_hidden = false;
    let mut captured_at = None;
    let mut first_user = None;
    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let has_newline = line.ends_with('\n');
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) if !has_newline => {
                partial_tail = true;
                break;
            }
            Err(error) => {
                return Err(invalid(format!(
                    "Malformed complete Antigravity transcript record on line {}: {error}",
                    line_no + 1
                )));
            }
        };
        captured_at = captured_at.or_else(|| value_string(&value, &["created_at"]));
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let (role, content) = match kind {
            "USER_INPUT" => (
                "user",
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .map(antigravity_user_text),
            ),
            "PLANNER_RESPONSE" => {
                omitted_hidden |=
                    value.get("thinking").is_some() || value.get("tool_calls").is_some();
                (
                    "assistant",
                    value
                        .get("content")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                )
            }
            "GENERIC" => (
                "tool",
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .map(redact_antigravity_text),
            ),
            "ERROR_MESSAGE" => (
                "tool",
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(redact_antigravity_text),
            ),
            "CHECKPOINT" | "SYSTEM_MESSAGE" => {
                omitted_hidden = true;
                continue;
            }
            _ => {
                omitted_hidden = true;
                continue;
            }
        };
        let Some(content) = content.filter(|content| !content.trim().is_empty()) else {
            continue;
        };
        if content.len() > MAX_TEXT {
            return Err(invalid(
                "Antigravity transcript message exceeds the per-message size limit",
            ));
        }
        if role == "user" && first_user.is_none() {
            first_user = Some(content.clone());
        }
        let raw_id = value.get("step_index").cloned();
        let mut item = segment(
            &record_id_hint,
            "1",
            segments.len(),
            raw_id.as_ref(),
            None,
            None,
            content,
            Some(role.to_owned()),
        );
        item["_stable_key"] = json!(format!(
            "step:{}",
            value
                .get("step_index")
                .map(Value::to_string)
                .unwrap_or_else(|| line_no.to_string())
        ));
        segments.push(item);
        if segments.len() > MAX_SEGMENTS {
            return Err(invalid(
                "Antigravity transcript has too many visible messages",
            ));
        }
    }
    if segments.is_empty() && partial_tail {
        return Err(invalid(
            "Antigravity transcript contains only a partial tail",
        ));
    }
    if segments.is_empty() {
        return Err(invalid(
            "Antigravity transcript contains no visible messages",
        ));
    }
    let title = first_user
        .as_deref()
        .map(|body| {
            body.lines()
                .next()
                .unwrap_or(body)
                .chars()
                .take(120)
                .collect()
        })
        .unwrap_or_else(|| "Antigravity conversation".to_owned());
    let coverage = if partial_tail {
        json!({"state":"partial","reason":"The final Antigravity transcript record is incomplete and will be retried"})
    } else if omitted_hidden {
        json!({"state":"partial","reason":"Hidden planner reasoning, system records and non-visible metadata were omitted; the raw transcript is retained"})
    } else {
        json!({"state":"complete"})
    };
    let record = json!({
        "id":record_id_hint,
        "revision":"1",
        "title":title,
        "purpose":"Unclassified",
        "kind":"chat",
        "provider":"antigravity",
        "agent":{"name":"Antigravity"},
        "captured_at":captured_at.unwrap_or_else(now),
        "imported_at":now(),
        "duration_ms":null,
        "audio":{"state":"unavailable","reason":"Chat source has no audio"},
        "coverage":coverage,
        "segments":segments,
        "source_refs":[{"path":path.to_string_lossy(),"origin_id":origin_id}],
        "related":[]
    });
    Ok(Imported {
        origin_id,
        source_hash: ContentHash::digest(&bytes).to_string(),
        record,
        snapshot: path.to_path_buf(),
    })
}

fn parse_jsonl(path: &Path, provider: &str) -> Result<Imported> {
    let bytes = read_limited(path)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| invalid("Conversation JSONL is not UTF-8"))?;
    let mut messages = Vec::new();
    let mut partial_tail = false;
    let mut unknown_coverage = false;
    let mut session_identity = None;
    let mut fallback_identity = None;
    let mut captured_at = None;
    let mut parent_id = None;
    let mut agent = Value::Null;
    let mut session_path = Value::Null;
    let mut first_message_at: Option<i64> = None;
    let mut last_message_at: Option<i64> = None;
    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let has_newline = line.ends_with('\n');
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) if !has_newline => {
                partial_tail = true;
                break;
            }
            Err(e) => {
                return Err(invalid(format!(
                    "Malformed complete Codex JSONL record on line {}: {e}",
                    line_no + 1
                )));
            }
        };
        if fallback_identity.is_none() {
            fallback_identity = value_string(
                &value,
                &["session_id", "conversation_id", "thread_id", "id"],
            );
        }
        if parent_id.is_none() {
            parent_id = value_string(&value, &["parent_id", "parentID"]);
        }
        if value.get("type").and_then(Value::as_str) == Some("session_meta")
            && let Some(payload) = value.get("payload")
        {
            session_identity = value_string(payload, &["id", "session_id", "conversation_id"]);
            captured_at = value_string(payload, &["timestamp", "created_at"]);
            session_path = payload["cwd"].clone();
            parent_id =
                value_string(payload, &["parent_id", "parentID", "forked_from_id"]).or(parent_id);
            let spawn = &payload["source"]["subagent"]["thread_spawn"];
            parent_id = value_string(spawn, &["parent_thread_id"]).or(parent_id);
            if spawn.is_object() {
                agent = json!({"path":spawn["agent_path"],"nickname":spawn["agent_nickname"],"depth":spawn["depth"],"relationship_basis":"provider_session_metadata"});
            }
        }
        let message = value
            .get("payload")
            .filter(|payload| payload.get("type").and_then(Value::as_str) == Some("message"))
            .unwrap_or(&value);
        if value.get("type").and_then(Value::as_str) == Some("response_item") && message == &value {
            unknown_coverage = true;
        }
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }
        let body = message
            .get("content")
            .or_else(|| value.get("message").and_then(|m| m.get("content")))
            .cloned()
            .unwrap_or(Value::Null);
        let content = match body {
            Value::String(s) => s,
            Value::Array(a) => a
                .iter()
                .filter_map(|x| {
                    let kind = x.get("type").and_then(Value::as_str);
                    if kind.is_none() || matches!(kind, Some("input_text" | "output_text")) {
                        x.get("text").and_then(Value::as_str).or_else(|| x.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };
        if content.is_empty() {
            continue;
        }
        if let Some(time) = value_string(&value, &["timestamp", "created_at"])
            .or_else(|| value_string(message, &["timestamp", "created_at"]))
            .and_then(|time| chrono::DateTime::parse_from_rfc3339(&time).ok())
            .map(|time| time.timestamp_millis())
        {
            first_message_at = Some(first_message_at.map_or(time, |prior| prior.min(time)));
            last_message_at = Some(last_message_at.map_or(time, |prior| prior.max(time)));
        }
        if content.len() > MAX_TEXT {
            return Err(invalid(
                "Conversation message exceeds the per-message size limit",
            ));
        }
        messages.push((
            role.to_owned(),
            content,
            value_string(message, &["id", "message_id"]),
        ));
        if messages.len() > MAX_SEGMENTS {
            return Err(invalid("Codex history has too many messages"));
        }
    }
    if messages.is_empty() && partial_tail {
        return Err(invalid("Codex JSONL contains only a partial tail"));
    }
    let origin_id = session_identity
        .or(fallback_identity)
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let captured_at = captured_at.unwrap_or_else(now);
    let record_id_hint = format!(
        "history-{}",
        id_hex(format!("{provider}\0{origin_id}").as_bytes())
    );
    let title = messages
        .iter()
        .find(|(r, _, _)| r == "user")
        .map(|(_, s, _)| {
            s.lines()
                .next()
                .unwrap_or(s)
                .chars()
                .take(120)
                .collect::<String>()
        })
        .unwrap_or_else(|| "Codex conversation".into());
    let rev = "1";
    let segments = messages
        .iter()
        .enumerate()
        .map(|(i, (role, body, raw))| {
            let raw_value = raw.as_ref().map(|s| Value::String(s.clone()));
            segment(
                &record_id_hint,
                rev,
                i,
                raw_value.as_ref(),
                None,
                None,
                body.clone(),
                Some(role.clone()),
            )
        })
        .collect::<Vec<_>>();
    let coverage = if partial_tail {
        json!({"state":"partial","reason":"The final JSONL record is incomplete and will be retried"})
    } else if unknown_coverage {
        json!({"state":"partial","reason":"Source contains records outside visible user and assistant message coverage"})
    } else {
        json!({"state":"complete"})
    };
    let record = json!({"id":record_id_hint,"revision":rev,"title":title,"kind":"chat","provider":provider,"agent":agent,"session_path":session_path,"captured_at":captured_at,"imported_at":now(),"duration_ms":first_message_at.zip(last_message_at).map(|(first,last)|last.saturating_sub(first)),"audio":{"state":"unavailable","reason":"Chat source has no audio"},"coverage":coverage,"segments":segments,"source_refs":[{"path":path.to_string_lossy(),"origin_id":origin_id,"parent_id":parent_id}],"related":[]});
    Ok(Imported {
        origin_id,
        source_hash: ContentHash::digest(&bytes).to_string(),
        record,
        snapshot: path.to_path_buf(),
    })
}

fn file_digest(path: &Path, limit: u64) -> Result<String> {
    reject_symlink(path)?;
    let mut digest = Sha256::new();
    hash_file_into(path, &mut digest, limit)?;
    Ok(ContentHash::from(<[u8; 32]>::from(digest.finalize())).to_string())
}

fn hash_file_into(path: &Path, digest: &mut Sha256, limit: u64) -> Result<()> {
    reject_symlink(path)?;
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > limit {
        return Err(invalid("Archive file exceeds size limit"));
    }
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or_else(|| invalid("Archive file size overflow"))?;
        if total > limit {
            return Err(invalid("Archive grew beyond size limit"));
        }
        digest.update(&buffer[..n]);
    }
    let after = fs::metadata(path)?;
    if total != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(invalid("Archive file changed while hashing"));
    }
    Ok(())
}
fn snapshot_digest(path: &Path) -> Result<String> {
    let mut entries = Vec::new();
    let mut total = 0;
    for e in fs::read_dir(path)? {
        let e = e?;
        reject_symlink(&e.path())?;
        if !e.file_type()?.is_file() {
            return Err(invalid("Unexpected nested archive snapshot"));
        }
        let name = e.file_name().to_string_lossy().into_owned();
        check_component_name(&name)?;
        total += e.metadata()?.len();
        if total > 1024 * 1024 * 1024 || entries.len() >= 512 {
            return Err(invalid("Snapshot exceeds archive bounds"));
        }
        entries.push((name, file_digest(&e.path(), 1024 * 1024 * 1024)?));
    }
    entries.sort();
    Ok(ContentHash::digest(&serde_json::to_vec(&entries)?).to_string())
}
fn finish_pending(root: &Path, id: &str) -> Result<()> {
    let rec = root.join("records").join(id);
    let pending = rec.join("pending.json");
    if !pending.exists() {
        return Ok(());
    }
    let v: Value = serde_json::from_slice(&read_limited(&pending)?)?;
    let record = &v["record"];
    let revision = record["revision"]
        .as_str()
        .ok_or_else(|| invalid("Pending archive revision missing"))?;
    check_component_name(revision)?;
    if record["id"] != id {
        return Err(invalid("Pending archive identity mismatch"));
    }
    let stage = v["stage"]
        .as_str()
        .ok_or_else(|| invalid("Pending archive stage missing"))?;
    check_component_name(stage)?;
    let staged = root.join("staging").join(stage);
    let raw = root.join("raw").join(id).join(revision);
    // Recovery must never follow a directory substituted between the pending
    // marker and this replay.  The children are checked by snapshot_digest,
    // but the selected root itself needs the same guard.
    reject_symlink(&raw)?;
    reject_symlink(&staged)?;
    let source = if raw.exists() { &raw } else { &staged };
    if snapshot_digest(source)? != v["snapshot_hash"].as_str().unwrap_or("") {
        return Err(invalid("Pending archive integrity verification failed"));
    }
    if !raw.exists() {
        fs::rename(&staged, &raw)?;
        File::open(raw.parent().unwrap())?.sync_all()?;
    }
    let immutable = rec.join("revisions").join(format!("{revision}.json"));
    if immutable.exists() {
        let old: Value = serde_json::from_slice(&read_limited(&immutable)?)?;
        if old != *record {
            return Err(invalid("Archive revision collision"));
        }
    } else {
        atomic_json(&immutable, record)?;
    }
    atomic_json(&rec.join("record.json"), record)?;
    fs::remove_file(pending)?;
    File::open(rec)?.sync_all()?;
    Ok(())
}

fn copy_large_file(source: &Path, target: &Path, limit: u64) -> Result<()> {
    reject_symlink(source)?;
    let mut input = File::open(source)?;
    let before = input.metadata()?;
    if !before.is_file() || before.len() > limit {
        return Err(invalid("Audio source exceeds file limit"));
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    private_file(target)?;
    let count = std::io::copy(&mut Read::by_ref(&mut input).take(limit + 1), &mut output)?;
    if count > limit {
        return Err(invalid("Source exceeds byte limit"));
    }
    let after = fs::metadata(source)?;
    if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
        return Err(invalid("Audio source changed while it was being archived"));
    }
    output.sync_all()?;
    File::open(target.parent().unwrap())?.sync_all()?;
    Ok(())
}
fn parse_normalized(path: &Path) -> Result<Imported> {
    let bytes = read_limited(path)?;
    let input: Value = serde_json::from_slice(&bytes)?;
    if input["schema"] != "soliton-source/1" {
        return Err(invalid("Unsupported collected-source schema"));
    }
    let provider = input["provider"]
        .as_str()
        .ok_or_else(|| invalid("Source provider missing"))?;
    if !matches!(
        provider,
        "feed" | "transcript" | "browser" | "web" | "youtube"
    ) {
        return Err(invalid("Unsupported collected-source provider"));
    }
    let origin = input["id"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() < 4096)
        .ok_or_else(|| invalid("Source identity missing"))?
        .to_owned();
    let segments = input["segments"]
        .as_array()
        .ok_or_else(|| invalid("Collected source segments missing"))?;
    if segments.len() > MAX_SEGMENTS
        || segments.iter().any(|s| {
            s["text"]
                .as_str()
                .map(|t| t.len() > MAX_TEXT)
                .unwrap_or(true)
        })
    {
        return Err(invalid("Collected source exceeds segment bounds"));
    }
    let coverage = if matches!(
        input["coverage"]["state"].as_str(),
        Some("complete" | "partial" | "unavailable")
    ) {
        input["coverage"].clone()
    } else {
        json!({"state":"partial","reason":"Collector did not supply valid coverage"})
    };
    let title = input["title"]
        .as_str()
        .filter(|s| s.len() <= 1000)
        .unwrap_or("Collected source");
    let mut normalized_segments = segments.clone();
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for (index, segment) in normalized_segments.iter_mut().enumerate() {
        let key = segment
            .get("id")
            .map(Value::to_string)
            .unwrap_or_else(|| format!("index:{index}"));
        let occurrence = seen.entry(key.clone()).or_default();
        segment["raw_id"] = segment["id"].clone();
        segment["_stable_key"] = json!(format!("{provider}\0{key}\0{occurrence}"));
        *occurrence += 1;
    }
    let mut semantic = input.clone();
    if let Some(object) = semantic.as_object_mut() {
        for key in ["captured_at", "imported_at", "import_ready"] {
            object.remove(key);
        }
    }
    let mut record = json!({"title":title,"kind":"source","provider":provider,"captured_at":input["captured_at"].as_str().unwrap_or(""),"url":input["url"],"duration_ms":input["duration_ms"],"coverage":coverage,"audio":{"state":"unavailable","reason":"Text source"},"segments":normalized_segments,"source_refs":input["source_refs"],"related":[]});
    record["_normalized_file_hash"] = json!(ContentHash::digest(&bytes).to_string());
    Ok(Imported {
        origin_id: origin,
        source_hash: ContentHash::digest(&serde_json::to_vec(&semantic)?).to_string(),
        record,
        snapshot: path.to_owned(),
    })
}
impl HistoryStore {
    /// Only Service calls this with a receipt read from its own InboxStore.
    pub(crate) fn register_inbox(&mut self, receipt: &Value) -> Result<Value> {
        let id = receipt["id"]
            .as_str()
            .ok_or_else(|| invalid("Inbox receipt identity missing"))?;
        let path = PathBuf::from(
            receipt["path"]
                .as_str()
                .ok_or_else(|| invalid("Inbox path missing"))?,
        );
        let digest = file_digest(&path, 1024 * 1024 * 1024)?;
        if digest != format!("sha256:{}", receipt["sha256"].as_str().unwrap_or("")) {
            return Err(invalid("Received file changed since its receipt"));
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ![
            "wav", "mp3", "m4a", "mp4", "ogg", "opus", "flac", "aac", "webm",
        ]
        .contains(&ext.as_str())
        {
            return Err(invalid(
                "This received file is not a supported audio format",
            ));
        }
        let record = json!({"title":receipt["filename"],"kind":"voice_note","provider":"soliton-inbox","captured_at":receipt["committed_at"],"duration_ms":null,"coverage":{"state":"unavailable","reason":"Original audio saved; no transcript supplied or generated"},"audio":{"state":"retained","path":path,"format":ext},"segments":[],"source_refs":[{"path":path,"receipt_id":receipt["receipt_id"],"artifact_id":receipt["artifact_id"]}],"related":[]});
        self.publish(Imported {
            origin_id: id.to_owned(),
            source_hash: digest,
            record,
            snapshot: path,
        })
    }
}

fn sqlite_output(args: &[&str], limit: usize) -> Result<Vec<u8>> {
    let mut child = Command::new("sqlite3")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| invalid("sqlite3 is required for database history import"))?;
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    });
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > std::time::Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(invalid(
                "Database operation timed out; retry when source is idle",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let bytes = reader
        .join()
        .map_err(|_| invalid("Database output reader failed"))??;
    if bytes.len() > limit {
        return Err(invalid("Database query exceeds output limit"));
    }
    if !status.success() {
        return Err(invalid(
            "Database query failed; source schema may be unsupported",
        ));
    }
    Ok(bytes)
}

fn sqlite_script(path: &Path, script: &str, limit: usize) -> Result<()> {
    if script.len() > limit {
        return Err(invalid("History index update exceeds the size limit"));
    }
    reject_symlink(path)?;
    let database = path
        .to_str()
        .ok_or_else(|| invalid("History index path is not UTF-8"))?;
    let mut child = Command::new("sqlite3")
        .arg(database)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| invalid("sqlite3 is required for the history search index"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| invalid("History index stdin unavailable"))?;
    let input = script.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > std::time::Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            return Err(invalid("History index update timed out"));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let written = writer
        .join()
        .map_err(|_| invalid("History index writer stopped"))?;
    if !status.success() || written.is_err() {
        return Err(invalid("History search index update failed"));
    }
    File::open(path)?.sync_all()?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

impl HistoryStore {
    fn set_state(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("History id required"))?;
        let state = args["state"].as_str().unwrap_or("");
        if !matches!(state, "active" | "archived" | "deleted") {
            return Err(invalid("Invalid history state"));
        }
        let current = self.load_current(id)?;
        if args["expected_revision"].as_str().is_none()
            || current["revision"] != args["expected_revision"]
        {
            return Err(invalid("History revision conflict"));
        }
        if state == "deleted" && args["confirmed"].as_bool() != Some(true) {
            return Err(invalid("Confirm recoverable record deletion"));
        }
        if current["record_state"].as_str().unwrap_or("active") == state {
            return Ok(public_record_at(&self.root, &current));
        }
        let mut record = current.clone();
        let number = current["_revision_number"].as_u64().unwrap_or(1) + 1;
        record["record_state"] = json!(state);
        record["revision"] = json!(number.to_string());
        record["_revision_number"] = json!(number);
        record["updated_at"] = json!(now());
        record["lifecycle"] = json!({"parent_revision":current["revision"],"previous_state":current["record_state"].as_str().unwrap_or("active"),"state":state,"at":now()});
        let dir = self.root.join("records").join(id);
        atomic_json(
            &dir.join("revisions").join(format!("{number}.json")),
            &record,
        )?;
        atomic_json(&dir.join("record.json"), &record)?;
        self.records.insert(id.to_owned(), slim_record(&record));
        index_upsert(&self.root, &record)?;
        Ok(public_record_at(&self.root, &record))
    }

    fn label(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("History id required"))?;
        let current = self.load_current(id)?;
        if current["revision"] != args["expected_revision"] {
            return Err(invalid(
                "History changed. Reload before changing its label.",
            ));
        }
        let title = args["title"].as_str().unwrap_or("").trim();
        let purpose = args["purpose"].as_str().unwrap_or("").trim();
        if title.is_empty() || title.len() > 1000 || purpose.len() > 1000 {
            return Err(invalid(
                "Title and purpose must be at most 1000 bytes; title is required",
            ));
        }
        let number = current["_revision_number"].as_u64().unwrap_or(1) + 1;
        let mut record = current.clone();
        record["title"] = json!(title);
        record["purpose"] = json!(purpose);
        record["revision"] = json!(number.to_string());
        record["_revision_number"] = json!(number);
        record["updated_at"] = json!(now());
        record["annotation"] = json!({"parent_revision":current["revision"],"updated_at":now()});
        let dir = self.root.join("records").join(id);
        atomic_json(
            &dir.join("revisions").join(format!("{number}.json")),
            &record,
        )?;
        atomic_json(&dir.join("record.json"), &record)?;
        self.records.insert(id.to_owned(), slim_record(&record));
        index_upsert(&self.root, &record)?;
        Ok(public_record_at(&self.root, &record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_ids_keep_repeated_raw_ids() {
        let a = segment(
            "r",
            "1",
            0,
            Some(&json!(0)),
            Some(0),
            Some(1),
            "a".into(),
            None,
        );
        let b = segment(
            "r",
            "1",
            1,
            Some(&json!(0)),
            Some(1),
            Some(2),
            "b".into(),
            None,
        );
        assert_ne!(a["id"], b["id"]);
        assert_eq!(a["raw_id"], 0);
    }
}
