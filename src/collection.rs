//! Bounded Learning Center source collection and a durable, poll-driven queue.
//!
//! The store owns source configuration and collection jobs, while the parent
//! service remains the only history writer.  A single worker parses one local
//! transcript or feed at a time and returns normalized source metadata to the
//! service through `collection-poll`.  No imported text is executed.
use crate::{Error, Result};
use chrono::Utc;
use serde_json::{Value, json};
use soliton_kernel::{ContentHash, Entity, ObjectId};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ITEMS: usize = 20_000;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn captured_at(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339())
        .unwrap_or_else(|_| Utc::now().to_rfc3339())
}

fn private_dir(path: &Path) -> Result<()> {
    reject_symlink_components(path, false)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err(invalid("Collection store path is not a directory"));
    }
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid("Collection store path is not a directory"));
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

fn reject_path_syntax(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(invalid("Choose an absolute source path"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid("Source path must not contain . or .. components"));
    }
    Ok(())
}

fn reject_symlink_components(path: &Path, include_final: bool) -> Result<()> {
    reject_path_syntax(path)?;
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let final_component = current == path;
        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() && (include_final || !final_component) =>
            {
                return Err(invalid("Collection source symlink is not accepted"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && final_component => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("Collection source exceeds its size limit"));
    }
    Ok(bytes)
}

fn read_source(path: &Path) -> Result<Vec<u8>> {
    reject_symlink_components(path, true)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            invalid("Collection source is missing")
        } else {
            error.into()
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(invalid("Collection source symlink is not accepted"));
    }
    if !metadata.is_file() {
        return Err(invalid("Collection source must be a regular file"));
    }
    read_bounded(path, MAX_SOURCE_BYTES)
}

fn safe_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:.".contains(&byte))
    {
        return Err(invalid(format!("Invalid {label}")));
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Collection record has no parent"))?;
    private_dir(parent)?;
    let stamp = format!("{}-{}", now_nanos(), std::process::id());
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        stamp
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    private_file(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    sync_dir(parent)?;
    Ok(())
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Collection archive has no parent"))?;
    private_dir(parent)?;
    let stamp = format!("{}-{}", now_nanos(), std::process::id());
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        stamp
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    private_file(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    sync_dir(parent)?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn hash(bytes: &[u8]) -> String {
    ContentHash::digest(bytes).to_string()
}

fn youtube_video_id(location: &str) -> Option<String> {
    let (scheme, rest) = location.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let (authority, path_query) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let candidate = if host == "youtu.be" {
        path.trim_matches('/').split('/').next().unwrap_or("")
    } else if matches!(
        host.as_str(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com"
    ) {
        query.split('&').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (key == "v").then_some(value)
        })?
    } else {
        return None;
    };
    if candidate.len() == 11
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn canonical_location(kind: &str, location: &str) -> String {
    if kind == "youtube-video"
        && let Some(video_id) = youtube_video_id(location)
    {
        return format!("https://www.youtube.com/watch?v={video_id}");
    }
    location.to_owned()
}

fn source_id(kind: &str, location: &str) -> String {
    let stable_location = canonical_location(kind, location);
    format!(
        "collection-{}-{}",
        kind,
        ContentHash::digest(stable_location.as_bytes()).short_hex(16)
    )
}

fn source_id_for_child(child: &Value) -> Result<String> {
    let kind = child["type"]
        .as_str()
        .ok_or_else(|| invalid("Child source kind missing"))?;
    let url = child["url"]
        .as_str()
        .ok_or_else(|| invalid("Child source URL missing"))?;
    if kind != "youtube-video" {
        return Err(invalid("Unsupported channel child source"));
    }
    Ok(source_id(kind, url))
}

fn job_id(source_id: &str) -> String {
    format!(
        "job-{}-{}",
        ContentHash::digest(format!("{source_id}\0{}", now_nanos()).as_bytes()).short_hex(16),
        now_nanos()
    )
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn parse_time(value: &str) -> Option<i64> {
    let value = value.trim().replace(',', ".");
    let parts = value.split(':').collect::<Vec<_>>();
    if !(1..=3).contains(&parts.len()) {
        return None;
    }
    let seconds = parts.last()?.parse::<f64>().ok()?;
    let minutes = if parts.len() >= 2 {
        parts[parts.len() - 2].parse::<f64>().ok()?
    } else {
        0.0
    };
    let hours = if parts.len() == 3 {
        parts[0].parse::<f64>().ok()?
    } else {
        0.0
    };
    Some(((hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0).round() as i64)
}

fn parse_cues(text: &str) -> Result<(Vec<Value>, Value)> {
    let mut segments = Vec::new();
    let mut gaps = Vec::new();
    let mut truncated = false;
    let blocks = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut previous_end = None;
    for block in blocks.split("\n\n") {
        if segments.len() >= MAX_ITEMS {
            truncated = true;
            break;
        }
        let lines = block.lines().map(str::trim).collect::<Vec<_>>();
        let Some(timestamp_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let times = lines[timestamp_index]
            .split("-->")
            .map(str::trim)
            .collect::<Vec<_>>();
        if times.len() != 2 {
            continue;
        }
        let start = parse_time(times[0].split_whitespace().next().unwrap_or(""));
        let end = parse_time(times[1].split_whitespace().next().unwrap_or(""));
        let Some(start) = start else { continue };
        let Some(end) = end.filter(|end| *end >= start) else {
            continue;
        };
        if let Some(previous_end) = previous_end.filter(|previous_end| start > *previous_end + 1000)
        {
            gaps.push(json!({"start_ms":previous_end,"end_ms":start}));
        }
        let cue = lines[timestamp_index + 1..]
            .iter()
            .copied()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if cue.is_empty() {
            continue;
        }
        let index = segments.len();
        segments.push(json!({"id":format!("s{index}"),"index":index,"start_ms":start,"end_ms":end,"text":cue,"speaker":Value::Null}));
        previous_end = Some(end);
    }
    if segments.is_empty() {
        return Err(invalid("Transcript contains no timed cues"));
    }
    let coverage = json!({
        "state":if gaps.is_empty() && !truncated{"complete"}else{"partial"},
        "segments_examined":segments.len(),
        "covered_ranges":segments.iter().map(|segment|json!({"start_ms":segment["start_ms"],"end_ms":segment["end_ms"]})).collect::<Vec<_>>(),
        "gaps":gaps,
        "notice":if truncated{"Transcript was bounded at the maximum segment count"}else if segments.iter().any(|segment|segment["start_ms"].is_null()){"Some timestamps are unavailable"}else{"Timed cue coverage"}
    });
    Ok((segments, coverage))
}

fn parse_json_transcript(value: &Value) -> Result<(Vec<Value>, Value)> {
    let array = value
        .as_array()
        .or_else(|| value.get("segments").and_then(Value::as_array))
        .or_else(|| value.get("chunks").and_then(Value::as_array))
        .ok_or_else(|| invalid("Transcript JSON has no segments array"))?;
    let mut segments = Vec::new();
    let mut gaps = Vec::new();
    let mut previous_end = None;
    let truncated = array.len() > MAX_ITEMS;
    for item in array.iter().take(MAX_ITEMS) {
        let Some(raw_text) = text(item, &["text", "content", "transcript"]) else {
            continue;
        };
        let raw_text = raw_text.trim();
        if raw_text.is_empty() {
            continue;
        }
        let start = value_time(item, &["start_ms", "start", "from"]);
        let end = value_time(item, &["end_ms", "end", "to"]);
        if let (Some(previous_end), Some(start)) = (previous_end, start)
            && start > previous_end + 1000
        {
            gaps.push(json!({"start_ms":previous_end,"end_ms":start}));
        }
        previous_end = end.or(start).or(previous_end);
        let index = segments.len();
        segments.push(json!({"id":text(item,&["id","key"]).unwrap_or_else(||format!("s{index}")),"index":index,"start_ms":start,"end_ms":end,"text":raw_text,"speaker":text(item,&["speaker","role"])}));
    }
    if segments.is_empty() {
        return Err(invalid("Transcript JSON contains no text segments"));
    }
    let state = if gaps.is_empty()
        && !truncated
        && segments
            .iter()
            .all(|segment| segment["start_ms"].is_number())
    {
        "complete"
    } else {
        "partial"
    };
    Ok((
        segments.clone(),
        json!({"state":state,"segments_examined":segments.len(),"covered_ranges":segments.iter().filter_map(|segment|if segment["start_ms"].is_number(){Some(json!({"start_ms":segment["start_ms"],"end_ms":segment["end_ms"]}))}else{None}).collect::<Vec<_>>(),"gaps":gaps,"notice":if truncated{"Transcript was bounded at the maximum segment count"}else if state=="partial"{"Missing or non-contiguous timestamps remain visible"}else{"Timed segment coverage"}}),
    ))
}

fn value_time(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        let number = value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()));
        if let Some(number) = number {
            if !number.is_finite() || number < 0.0 {
                return None;
            }
            return Some((number * if key.ends_with("_ms") { 1.0 } else { 1000.0 }).round() as i64);
        }
        value.as_str().and_then(parse_time)
    })
}

fn parse_transcript(path: &Path, bytes: &[u8]) -> Result<(Value, Value)> {
    let source_hash = hash(bytes);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text_value =
        std::str::from_utf8(bytes).map_err(|_| invalid("Transcript source must be UTF-8"))?;
    let (segments, coverage) = if extension == "json" {
        parse_json_transcript(
            &serde_json::from_str(text_value)
                .map_err(|_| invalid("Transcript JSON is malformed"))?,
        )?
    } else if extension == "srt" || extension == "vtt" || text_value.contains("-->") {
        parse_cues(text_value)?
    } else {
        let lines = text_value
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let segments = lines.iter().take(MAX_ITEMS).enumerate().map(|(index,line)|json!({"id":format!("s{index}"),"index":index,"start_ms":Value::Null,"end_ms":Value::Null,"text":line,"speaker":Value::Null})).collect::<Vec<_>>();
        if segments.is_empty() {
            return Err(invalid("Transcript contains no text"));
        }
        (
            segments.clone(),
            json!({"state":"partial","segments_examined":segments.len(),"covered_ranges":[],"gaps":[],"notice":if lines.len()>MAX_ITEMS{"Plain text was bounded at the maximum segment count"}else{"Plain text has no timestamps; coverage is partial"}}),
        )
    };
    let normalized = json!({"schema":"soliton-source/1","id":format!("collection-transcript-{}",ContentHash::digest(path.to_string_lossy().as_bytes()).short_hex(16)),"kind":"transcript","provider":"transcript","captured_at":captured_at(path),"source_path":path,"source_hash":source_hash,"title":path.file_stem().and_then(|value|value.to_str()).unwrap_or("Transcript"),"segments":segments,"coverage":coverage,"source_refs":[{"path":path,"origin_id":source_hash}],"import_ready":{"provider":"normalized","path":path}});
    Ok((
        normalized.clone(),
        json!({"kind":"transcript","provider":"normalized","source_path":path,"source_hash":normalized["source_hash"],"coverage":normalized["coverage"],"segments_total":normalized["segments"].as_array().map_or(0,Vec::len),"normalized":normalized}),
    ))
}

fn parse_feed_safe(path: &Path, bytes: &[u8]) -> Result<(Value, Value)> {
    let script = r#"import json, sys, xml.etree.ElementTree as ET
try:
    data=sys.stdin.buffer.read(33554433)
    if len(data)>33554432: raise ValueError('feed exceeds size limit')
    root=ET.fromstring(data)
except Exception as error:
    print(str(error), file=sys.stderr)
    raise SystemExit(2)
def local(tag): return tag.rsplit('}',1)[-1].lower()
def text(node, names):
    for child in list(node):
        if local(child.tag) in names:
            value=''.join(child.itertext()).strip()
            if value: return value[:65536]
    return None
def deeptext(node, names):
    for candidate in node.iter():
        if local(candidate.tag) in names:
            value=''.join(candidate.itertext()).strip()
            if value: return value[:65536]
    return None
items=[]
for node in root.iter():
    if local(node.tag) not in ('item','entry'): continue
    link=text(node, ('link',))
    for child in list(node):
        if local(child.tag)=='link' and child.attrib.get('href'):
            link=child.attrib['href'][:8192]; break
    guid=link or text(node, ('guid','id'))
    title=text(node, ('title',)) or 'Untitled feed item'
    summary=text(node, ('description','summary','content'))
    published=text(node, ('pubdate','published','updated'))
    items.append({'id':guid or 'item-'+str(len(items)),'title':title,'url':link,'published_at':published,'summary':summary})
truncated=len(items)>4096
print(json.dumps({'title':deeptext(root,('title',)),'items':items[:4096],'truncated':truncated},separators=(',',':')))
"#;
    let mut child = Command::new("python3")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| invalid(format!("RSS/Atom parser unavailable: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| invalid("RSS/Atom parser input unavailable"))?
        .write_all(bytes)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "RSS/Atom XML is malformed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let parsed: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| invalid("RSS/Atom parser returned malformed JSON"))?;
    let items = parsed["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return Err(invalid("Feed contains no entries"));
    }
    let segments = items.iter().enumerate().map(|(index, item)| json!({"id":item["id"],"index":index,"start_ms":Value::Null,"end_ms":Value::Null,"text":format!("{}: {}",item["title"].as_str().unwrap_or("Untitled"),item["summary"].as_str().unwrap_or("")),"url":item["url"],"speaker":Value::Null})).collect::<Vec<_>>();
    let refs = items
        .iter()
        .map(|item| json!({"url":item["url"],"path":path,"origin_id":item["id"]}))
        .collect::<Vec<_>>();
    let source_hash = hash(bytes);
    let normalized = json!({"schema":"soliton-source/1","id":format!("collection-feed-{}",ContentHash::digest(path.to_string_lossy().as_bytes()).short_hex(16)),"kind":"feed","provider":"feed","captured_at":captured_at(path),"source_path":path,"source_hash":source_hash,"title":parsed["title"].as_str().unwrap_or_else(||path.file_stem().and_then(|value|value.to_str()).unwrap_or("Feed")),"items":items,"segments":segments,"coverage":{"state":if parsed["truncated"].as_bool().unwrap_or(false){"partial"}else{"complete"},"items_examined":segments.len(),"notice":if parsed["truncated"].as_bool().unwrap_or(false){"Feed was bounded at the maximum item count"}else{"RSS/Atom metadata and text fields parsed locally"}},"source_refs":refs,"import_ready":{"provider":"normalized","path":path}});
    Ok((
        normalized.clone(),
        json!({"kind":"feed","provider":"normalized","source_path":path,"source_hash":source_hash,"coverage":normalized["coverage"],"items_total":normalized["items"].as_array().map_or(0,Vec::len),"normalized":normalized}),
    ))
}

fn parse_feed(path: &Path, bytes: &[u8]) -> Result<(Value, Value)> {
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        return parse_news_cache(path, bytes);
    }
    parse_feed_safe(path, bytes)
}

/// The Waybar news cache (`{"v":..,"items":[{"id","sid","title","link",..}]}`)
/// as a feed: every item keeps its cache id and its link as its source.
fn parse_news_cache(path: &Path, bytes: &[u8]) -> Result<(Value, Value)> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("News cache JSON is malformed"))?;
    let rows = value["items"]
        .as_array()
        .ok_or_else(|| invalid("News cache has no items array"))?;
    let truncated = rows.len() > 4096;
    let items = rows
        .iter()
        .take(4096)
        .filter_map(|row| {
            let id = row["id"].as_str()?.chars().take(512).collect::<String>();
            let url = row["link"].as_str().filter(|url| url.starts_with("http://") || url.starts_with("https://"))?;
            let published = row["publishedMs"].as_i64().and_then(chrono::DateTime::<Utc>::from_timestamp_millis).map(|time| time.to_rfc3339());
            Some(json!({"id":id,"sid":row["sid"],"title":row["title"].as_str().unwrap_or("Untitled news item").chars().take(4096).collect::<String>(),"url":url.chars().take(8192).collect::<String>(),"comments_url":row["comments"],"domain":row["domain"],"published_at":published,"summary":row["summary"].as_str().map(|text| text.chars().take(65536).collect::<String>())}))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err(invalid("News cache contains no linked items"));
    }
    let segments = items.iter().enumerate().map(|(index, item)| json!({"id":item["id"],"index":index,"start_ms":Value::Null,"end_ms":Value::Null,"text":format!("{}: {}",item["title"].as_str().unwrap_or("Untitled"),item["summary"].as_str().unwrap_or("")),"url":item["url"],"speaker":Value::Null})).collect::<Vec<_>>();
    let refs = items
        .iter()
        .map(|item| json!({"url":item["url"],"path":path,"origin_id":item["id"]}))
        .collect::<Vec<_>>();
    let source_hash = hash(bytes);
    let normalized = json!({"schema":"soliton-source/1","id":format!("collection-feed-{}",ContentHash::digest(path.to_string_lossy().as_bytes()).short_hex(16)),"kind":"feed","provider":"feed","captured_at":captured_at(path),"source_path":path,"source_hash":source_hash,"title":"Waybar news","items":items,"segments":segments,"coverage":{"state":if truncated{"partial"}else{"complete"},"items_examined":segments.len(),"notice":if truncated{"News cache was bounded at the maximum item count"}else{"Waybar news cache items with their cache ids and links; titles and summaries only"}},"source_refs":refs,"import_ready":{"provider":"normalized","path":path}});
    Ok((
        normalized.clone(),
        json!({"kind":"feed","provider":"normalized","source_path":path,"source_hash":source_hash,"coverage":normalized["coverage"],"items_total":normalized["items"].as_array().map_or(0,Vec::len),"normalized":normalized}),
    ))
}

fn kill_process_group(child: &mut std::process::Child) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{}", child.id())])
        .status();
    let _ = child.kill();
}

fn run_bounded_process(
    command: Command,
    timeout: std::time::Duration,
    stop: &AtomicBool,
    max_output: u64,
    label: &str,
) -> Result<Vec<u8>> {
    run_bounded_logged(command, timeout, stop, max_output, label, None)
}

/// As `run_bounded_process`, with the tool's own error output kept in `log`
/// so a failure carries the tool's words.
fn run_bounded_logged(
    mut command: Command,
    timeout: std::time::Duration,
    stop: &AtomicBool,
    max_output: u64,
    label: &str,
    log: Option<&Path>,
) -> Result<Vec<u8>> {
    let stderr = match log {
        Some(path) => Stdio::from(File::create(path)?),
        None => Stdio::null(),
    };
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|error| invalid(format!("{label} adapter unavailable: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid(format!("{label} adapter returned no output pipe")))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(max_output.saturating_add(1))
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = std::time::Instant::now();
    let bytes = loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(result) => break result?,
            Err(mpsc::RecvTimeoutError::Timeout)
                if !stop.load(Ordering::Acquire) && started.elapsed() < timeout => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_process_group(&mut child);
                let _ = child.wait();
                return Err(invalid(format!(
                    "{label} operation timed out or was cancelled"
                )));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                kill_process_group(&mut child);
                let _ = child.wait();
                return Err(invalid(format!("{label} adapter output reader stopped")));
            }
        }
    };
    if bytes.len() as u64 > max_output {
        kill_process_group(&mut child);
        let _ = child.wait();
        return Err(invalid(format!("{label} output exceeds its size limit")));
    }
    let status = child.wait()?;
    if !status.success() {
        let detail = log
            .and_then(last_error_line)
            .map(|line| format!(": {line}"))
            .unwrap_or_default();
        return Err(invalid(format!(
            "{label} operation failed with status {status}{detail}"
        )));
    }
    Ok(bytes)
}

fn fetch_feed(url: &str, stop: &AtomicBool) -> Result<Vec<u8>> {
    let mut command = Command::new("setsid");
    command.args([
        "curl",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "10",
        "--max-filesize",
        "33554432",
        "--",
        url,
    ]);
    run_bounded_process(
        command,
        std::time::Duration::from_secs(12),
        stop,
        MAX_SOURCE_BYTES,
        "HTTP feed",
    )
}

fn parse_browser(path: &Path, stop: &AtomicBool) -> Result<(Value, Value, Vec<u8>)> {
    let snapshot_path = std::env::temp_dir().join(format!(
        "soliton-browser-{}-{}.sqlite",
        std::process::id(),
        now_nanos()
    ));
    let script = r#"import json, sqlite3, sys
p=sys.argv[1]
snapshot=sys.argv[2]
src=sqlite3.connect('file:'+p+'?mode=ro', uri=True)
db=sqlite3.connect(snapshot)
src.backup(db)
rows=db.execute('select url, title, last_visit_time from urls order by last_visit_time desc limit 4096').fetchall()
total=db.execute('select count(*) from urls').fetchone()[0]
out=[]
for i,(url,title,visit) in enumerate(rows):
    url=(url or '')[:8192]; title=(title or '')[:4096]
    # Chromium timestamps are microseconds since 1601-01-01.
    start=int((visit or 0)/1000-11644473600000) if visit else None
    out.append({'id':'visit-'+str(i),'url':url,'title':title,'start_ms':start})
print(json.dumps({'rows':out,'truncated':total>4096},separators=(',',':')))
"#;
    let mut command = Command::new("setsid");
    command.args([
        "python3",
        "-c",
        script,
        path.to_string_lossy().as_ref(),
        snapshot_path.to_string_lossy().as_ref(),
    ]);
    let output = match run_bounded_process(
        command,
        std::time::Duration::from_secs(20),
        stop,
        MAX_SOURCE_BYTES,
        "Browser history",
    ) {
        Ok(output) => output,
        Err(error) => {
            let _ = fs::remove_file(&snapshot_path);
            return Err(error);
        }
    };
    let parsed: Value = serde_json::from_slice(&output)
        .map_err(|_| invalid("Browser history adapter returned malformed JSON"))?;
    let rows = parsed["rows"].as_array().cloned().unwrap_or_default();
    let truncated = parsed["truncated"].as_bool().unwrap_or(false);
    let snapshot = read_bounded(&snapshot_path, MAX_SOURCE_BYTES);
    let _ = fs::remove_file(&snapshot_path);
    let snapshot = snapshot?;
    let source_hash = hash(&snapshot);
    let segments = rows.iter().enumerate().map(|(index, row)| json!({"id":row["id"],"index":index,"start_ms":row["start_ms"],"end_ms":Value::Null,"text":format!("{} — {}",row["title"].as_str().unwrap_or("Untitled"),row["url"].as_str().unwrap_or("")),"speaker":Value::Null})).collect::<Vec<_>>();
    let normalized = json!({"schema":"soliton-source/1","id":format!("collection-browser-{}",ContentHash::digest(path.to_string_lossy().as_bytes()).short_hex(16)),"kind":"browser-history","provider":"browser","captured_at":captured_at(path),"source_path":path,"source_hash":source_hash,"title":"Browser history","segments":segments,"visits":rows,"coverage":{"state":if truncated{"partial"}else{"complete"},"segments_examined":segments.len(),"notice":if truncated{"Browser history was bounded at the maximum visit count"}else{"Read-only consistent SQLite backup; URLs and visit times only"}},"source_refs":[{"path":path,"origin_id":source_hash}],"import_ready":{"provider":"normalized","path":path}});
    Ok((
        normalized.clone(),
        json!({"kind":"browser-history","provider":"normalized","source_path":path,"source_hash":normalized["source_hash"],"coverage":normalized["coverage"],"segments_total":normalized["segments"].as_array().map_or(0,Vec::len),"normalized":normalized}),
        snapshot,
    ))
}

fn learning_helper(
    action: &str,
    url: &str,
    stop: Option<&AtomicBool>,
    tools: &Value,
) -> Result<(Value, Vec<u8>)> {
    // A private process group lets cancellation stop Python and its curl/yt-dlp
    // children together; source text is data on stdin, never shell code.
    let mut child = Command::new("setsid")
        .args([
            "python3",
            "-c",
            include_str!("../scripts/learning_sources.py"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut input) = child.stdin.take() {
        let mut request = json!({"action":action,"url":url});
        for key in ["ytdlp", "curl", "learn", "known"] {
            if !tools[key].is_null() {
                request[key] = tools[key].clone();
            }
        }
        input.write_all(&serde_json::to_vec(&request)?)?;
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("Source worker has no output"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = std::time::Instant::now();
    // A channel whose children are each looked up takes longer than one
    // video; a metadata lookup alone is short.
    let timeout = match action {
        "catalog" => 5,
        "youtube-meta" => 90,
        "youtube-channel" if tools["learn"] == true => 600,
        _ => 180,
    };
    let bytes = loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(result) => break result?,
            Err(mpsc::RecvTimeoutError::Timeout)
                if !stop.is_some_and(|stop| stop.load(Ordering::Acquire))
                    && started.elapsed().as_secs() < timeout =>
            {
                continue;
            }
            _ => {
                let _ = Command::new("kill")
                    .args(["-TERM", "--", &format!("-{}", child.id())])
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                return Err(invalid(
                    "Source collection interrupted; its durable job can be retried",
                ));
            }
        }
    };
    if bytes.len() > 64 * 1024 * 1024 {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .status();
        let _ = child.kill();
        let _ = child.wait();
        return Err(invalid("Source worker output exceeds the archive bound"));
    }
    let status = child.wait()?;
    let value: Value = serde_json::from_slice(&bytes)?;
    if !status.success() {
        return Err(invalid(
            value["error"]
                .as_str()
                .unwrap_or("Source collection failed"),
        ));
    }
    Ok((value, bytes))
}

fn collect_web_document(
    kind: &str,
    location: &str,
    stop: &AtomicBool,
    tools: &Value,
) -> Result<(Value, Vec<u8>)> {
    let (mut value, raw) = learning_helper(kind, location, Some(stop), tools)?;
    let source_hash = hash(&raw);
    let normalized = json!({"schema":"soliton-source/1","id":source_id(kind,location),"kind":if kind=="article"{"article"}else{"transcript"},"provider":if kind=="article"{"web"}else{"youtube"},
        "title":value["title"],"url":location,"captured_at":Utc::now().to_rfc3339(),"source_hash":source_hash,
        "segments":value["segments"].take(),"coverage":value["coverage"],"channel":value["channel"],"duration_seconds":value["duration_seconds"],
        "source_refs":[{"url":location,"origin_id":source_hash}],"import_ready":{"provider":"normalized"}});
    Ok((
        json!({"kind":kind,"provider":"normalized","source_hash":source_hash,"coverage":normalized["coverage"],"segments_total":normalized["segments"].as_array().map_or(0,Vec::len),"normalized":normalized,"child_sources":value["child_sources"]}),
        raw,
    ))
}

fn collect_source(source: &Value, stop: &AtomicBool) -> Result<(Value, Vec<u8>)> {
    let kind = source["kind"].as_str().unwrap_or("");
    let location = source["location"].as_str().unwrap_or("");
    if matches!(kind, "article" | "youtube-video" | "youtube-channel") {
        let tools = json!({"ytdlp":source["_ytdlp"],"curl":source["_curl"],"learn":source["_learn"],"known":source["_known"]});
        return collect_web_document(kind, location, stop, &tools);
    }
    if location.starts_with("http://") || location.starts_with("https://") {
        let bytes = fetch_feed(location, stop)?;
        let (_, result) = parse_feed(Path::new(location), &bytes)?;
        return Ok((result, bytes));
    }
    if kind == "browser-history" {
        let path = Path::new(location);
        let _ = read_source(path)?;
        let (_, result, snapshot) = parse_browser(path, stop)?;
        return Ok((result, snapshot));
    }
    let path = PathBuf::from(location);
    let bytes = read_source(&path)?;
    let (_, result) = if kind == "transcript" {
        parse_transcript(&path, &bytes)?
    } else if kind == "feed" {
        parse_feed(&path, &bytes)?
    } else {
        return Err(invalid("Unsupported collection source type"));
    };
    Ok((result, bytes))
}

/// Whether his workspace setting `automaticCapture` is on.  Cadence work
/// (sources due again, channels checked for new uploads) runs only then;
/// an explicit add, check or queue is always immediate.  The setting is read
/// from the workspace record the engine keeps; missing or unreadable is off,
/// and `SOLITON_AUTO_CAPTURE=0` forces it off as it does for capture.
fn automatic_capture_on(workspace: &Path) -> bool {
    if std::env::var("SOLITON_AUTO_CAPTURE").as_deref() == Ok("0") {
        return false;
    }
    let Ok(id) = ObjectId::derive("document", b"soliton-desktop-workspace-v1") else {
        return false;
    };
    let path = workspace
        .join("objects")
        .join("document")
        .join(format!("{id}.soliton"));
    let Ok(bytes) = read_bounded(&path, MAX_RECORD_BYTES) else {
        return false;
    };
    let Ok(entity) = Entity::from_canonical_bytes(&bytes) else {
        return false;
    };
    serde_json::from_str::<Value>(&entity.meta().details.body)
        .is_ok_and(|prefs| prefs["automaticCapture"] == true)
}

/// His local intake switches (`source-preferences/intake.json`, written by
/// the settings face).  Absent or unreadable means every switch is off.
fn intake_switch(prefs_root: &Path, key: &str) -> bool {
    let path = prefs_root.join("intake.json");
    if fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return false;
    }
    read_bounded(&path, 64 * 1024)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value[key] == true)
}

/// His local category and source rules for one source, decided before the
/// source can leave a job, a result or a transcript.  `Some(policy)` means
/// refused.  The decision uses his rules, his local mapping (video to
/// channel, channel or domain to category) and, for a YouTube video, what the
/// engine learned from the video's own metadata through the local mapping of
/// `learn_categories`, where the provider's category never decides alone.
/// `fetch` allows one metadata lookup when nothing is learned yet; what
/// cannot be learned stays unknown and is admitted.
#[allow(clippy::too_many_arguments)]
fn source_refusal(
    prefs_root: &Path,
    collection_root: &Path,
    tools: &Value,
    fetch: bool,
    kind: &str,
    location: &str,
    title: &str,
    channel: &str,
) -> Result<Option<Value>> {
    if !prefs_root.join("rules").is_dir() {
        return Ok(None);
    }
    let mut prefs = crate::source_preferences::SourcePreferences::open(prefs_root)?;
    let mut args = json!({"source_id":source_id(kind,location),"url":location,"title":title});
    if !channel.trim().is_empty() {
        args["channel"] = json!(channel);
    }
    if kind == "youtube-video"
        && prefs.needs_facts()
        && let Some(learned) = learned_facts(collection_root, tools, location, fetch)?
    {
        if title.trim().is_empty() {
            args["title"] = learned["title"].clone();
        }
        args["learned"] = learned;
    }
    let policy = prefs.command("source-policy-evaluate", &args)?;
    Ok((policy["include"] == false).then_some(policy))
}

const MAX_LEARNED_SOURCES: usize = 4096;
/// A video whose metadata could not be read is asked again after this long.
const LEARN_RETRY_SECS: i64 = 12 * 3600;

fn learned_path(collection_root: &Path) -> PathBuf {
    collection_root.join("intake").join("learned-sources.json")
}

fn load_learned(collection_root: &Path) -> BTreeMap<String, Value> {
    read_bounded(&learned_path(collection_root), MAX_SOURCE_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["items"].as_object().cloned())
        .map(|items| items.into_iter().collect())
        .unwrap_or_default()
}

/// Keeps what was learned about one video (or that it could not be read),
/// with its evidence, so later checks and cadence runs ask nobody again.
fn remember_learned(collection_root: &Path, video_id: &str, meta: Result<Value>) -> Result<Value> {
    let mut record = match &meta {
        Ok(meta) => crate::source_preferences::learn_categories(meta),
        Err(_) => json!({"categories":[]}),
    };
    record["video_id"] = json!(video_id);
    record["state"] = json!(if meta.is_ok() { "known" } else { "unknown" });
    record["learned_at"] = json!(Utc::now().to_rfc3339());
    record["learned_at_secs"] = json!(now_secs());
    if let Err(error) = &meta {
        record["error"] = json!(error.to_string().chars().take(500).collect::<String>());
    }
    let mut items = load_learned(collection_root);
    items.insert(video_id.to_owned(), record.clone());
    while items.len() > MAX_LEARNED_SOURCES {
        let oldest = items
            .iter()
            .min_by_key(|(_, item)| item["learned_at_secs"].as_i64().unwrap_or(0))
            .map(|(id, _)| id.clone());
        match oldest {
            Some(id) => items.remove(&id),
            None => break,
        };
    }
    atomic_json(
        &learned_path(collection_root),
        &json!({"schema":1,"items":items}),
    )?;
    Ok(record)
}

/// What is known about a YouTube video's channel and category: the stored
/// record, else (when `fetch`) one metadata lookup through the downloader,
/// falling back to the public oEmbed page for a video it cannot open.
fn learned_facts(
    collection_root: &Path,
    tools: &Value,
    url: &str,
    fetch: bool,
) -> Result<Option<Value>> {
    let Some(video_id) = youtube_video_id(url) else {
        return Ok(None);
    };
    if let Some(record) = load_learned(collection_root).remove(&video_id) {
        if record["state"] == "known" {
            return Ok(Some(record));
        }
        if now_secs() - record["learned_at_secs"].as_i64().unwrap_or(0) < LEARN_RETRY_SECS {
            return Ok(None);
        }
    }
    if !fetch {
        return Ok(None);
    }
    let watch = format!("https://www.youtube.com/watch?v={video_id}");
    let meta = learning_helper("youtube-meta", &watch, None, tools).map(|(value, _)| value);
    let record = remember_learned(collection_root, &video_id, meta)?;
    Ok((record["state"] == "known").then_some(record))
}

fn prefs_need_facts(prefs_root: &Path) -> bool {
    prefs_root.join("rules").is_dir()
        && crate::source_preferences::SourcePreferences::open(prefs_root)
            .is_ok_and(|prefs| prefs.needs_facts())
}

fn refusal_text(policy: &Value) -> String {
    let rules = policy["matched"]
        .as_array()
        .map(|rules| {
            rules
                .iter()
                .filter(|rule| rule["kind"] == "exclude")
                .map(|rule| {
                    format!(
                        "{} {}",
                        rule["matcher"].as_str().unwrap_or("source"),
                        rule["pattern"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let categories = policy["categories"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    format!(
                        "{} (via {})",
                        item["category"].as_str().unwrap_or(""),
                        item["via"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!(
        "Excluded by your local source preferences ({rules}{}). Nothing was collected or analysed.",
        if categories.is_empty() {
            String::new()
        } else {
            format!("; categories: {categories}")
        }
    )
}

struct WorkerRequest {
    job_id: String,
    source: Value,
    cancel: Arc<AtomicBool>,
}
struct WorkerResult {
    job_id: String,
    result: Result<Value>,
    raw: Option<Vec<u8>>,
}
enum WorkerMessage {
    Run(WorkerRequest),
    Stop,
}
struct Worker {
    tx: Sender<WorkerMessage>,
    rx: Receiver<WorkerResult>,
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

pub struct CollectionStore {
    root: PathBuf,
    sources: BTreeMap<String, Value>,
    jobs: BTreeMap<String, Value>,
    // Sources explicitly requested by the user are allowed to advance one at
    // a time through the normal observational poll loop. This keeps a due
    // source that was merely persisted on disk out of an explicit check.
    explicit_sources: std::collections::BTreeSet<String>,
    worker: Worker,
    active_job: Option<String>,
    active_cancel: Option<Arc<AtomicBool>>,
    media: MediaQueue,
    // His source rules and intake switches, beside the store in the workspace.
    prefs_root: PathBuf,
    // The caption/channel tool handed to the source worker (None: yt-dlp).
    source_program: Option<String>,
    // The page fetcher for the oEmbed fallback (None: curl).
    page_fetcher: Option<String>,
    // Tests settle the workspace's `automaticCapture` without a workspace.
    cadence_override: Option<bool>,
}

impl Drop for CollectionStore {
    fn drop(&mut self) {
        if let Some(cancel) = &self.active_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.worker.stop.store(true, Ordering::Release);
        let _ = self.worker.tx.send(WorkerMessage::Stop);
        if let Some(handle) = self.worker.handle.take() {
            let _ = handle.join();
        }
    }
}

impl CollectionStore {
    pub fn open(root: &Path) -> Result<Self> {
        reject_path_syntax(root)?;
        private_dir(root)?;
        private_dir(&root.join("sources"))?;
        private_dir(&root.join("jobs"))?;
        private_dir(&root.join("results"))?;
        let mut sources = load_records(&root.join("sources"), "source")?;
        for source in sources.values_mut() {
            source["capabilities"] = capabilities(
                source["kind"].as_str().unwrap_or(""),
                source["location"].as_str().unwrap_or(""),
            );
        }
        let prefs_root = root
            .parent()
            .map(|workspace| workspace.join("source-preferences"))
            .unwrap_or_else(|| root.join("source-preferences"));
        let mut jobs = load_records(&root.join("jobs"), "job")?;
        for job in jobs.values_mut() {
            if job["state"] == "running" {
                job["state"] = json!("queued");
            }
        }
        let (request_tx, request_rx) = mpsc::channel::<WorkerMessage>();
        let (result_tx, result_rx) = mpsc::channel::<WorkerResult>();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("soliton-collection".into())
            .spawn(move || {
                while let Ok(message) = request_rx.recv() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let WorkerMessage::Run(request) = message else {
                        break;
                    };
                    let collected = collect_source(&request.source, &request.cancel);
                    let (result, raw) = match collected {
                        Ok((value, bytes)) => (Ok(value), Some(bytes)),
                        Err(error) => (Err(error), None),
                    };
                    if result_tx
                        .send(WorkerResult {
                            job_id: request.job_id,
                            result,
                            raw,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| invalid(format!("Could not start collection worker: {error}")))?;
        let store = Self {
            root: root.to_path_buf(),
            sources,
            jobs,
            explicit_sources: Default::default(),
            worker: Worker {
                tx: request_tx,
                rx: result_rx,
                handle: Some(handle),
                stop,
            },
            active_job: None,
            active_cancel: None,
            media: MediaQueue::open(root, &prefs_root)?,
            prefs_root,
            source_program: None,
            page_fetcher: None,
            cadence_override: None,
        };
        store.persist_all_jobs()?;
        Ok(store)
    }

    /// Downloader and free-space reserve used by the media queue (the
    /// installed yt-dlp and 16 GiB by default); tests substitute a stand-in.
    pub fn set_media_downloader(&mut self, program: &Path, free_reserve: u64) {
        self.media.program = program.to_path_buf();
        self.media.free_reserve = free_reserve;
    }

    /// Caption and channel-listing tool for collected sources (yt-dlp by
    /// default); tests substitute a stand-in.
    pub fn set_source_downloader(&mut self, program: &Path) {
        self.source_program = Some(program.to_string_lossy().into_owned());
    }

    /// Page fetcher for the public oEmbed fallback (curl by default); tests
    /// substitute a stand-in.
    #[doc(hidden)]
    pub fn set_page_fetcher(&mut self, program: &Path) {
        self.page_fetcher = Some(program.to_string_lossy().into_owned());
    }

    fn tools(&self) -> Value {
        // A lookup uses the caption tool when one is named, else the media
        // queue's downloader: both are yt-dlp in his install.
        let ytdlp = self
            .source_program
            .clone()
            .unwrap_or_else(|| self.media.program.to_string_lossy().into_owned());
        json!({"ytdlp":ytdlp,"curl":self.page_fetcher})
    }

    /// Settles the cadence switch for a store opened outside a workspace.
    #[doc(hidden)]
    pub fn set_automatic_capture(&mut self, on: bool) {
        self.cadence_override = Some(on);
    }

    fn cadence_allowed(&self) -> bool {
        match self.cadence_override {
            Some(on) => on && std::env::var("SOLITON_AUTO_CAPTURE").as_deref() != Ok("0"),
            None => self.root.parent().is_some_and(automatic_capture_on),
        }
    }

    /// The gate every admission path passes: explicit add, check, cadence
    /// dispatch, a channel's children and the media queue and its retry.
    /// A YouTube video's category is learned here when it is not yet known.
    fn refusal(
        &self,
        kind: &str,
        location: &str,
        title: &str,
        channel: &str,
    ) -> Result<Option<Value>> {
        source_refusal(
            &self.prefs_root,
            &self.root,
            &self.tools(),
            true,
            kind,
            location,
            title,
            channel,
        )
    }

    /// The same gate deciding only on what is already learned, for paths
    /// that must not wait on a lookup (watch history, a channel's listing).
    fn known_refusal(
        &self,
        kind: &str,
        location: &str,
        title: &str,
        channel: &str,
    ) -> Result<Option<Value>> {
        source_refusal(
            &self.prefs_root,
            &self.root,
            &self.tools(),
            false,
            kind,
            location,
            title,
            channel,
        )
    }

    fn source_refusal_for(&self, source: &Value, fetch: bool) -> Result<Option<Value>> {
        source_refusal(
            &self.prefs_root,
            &self.root,
            &self.tools(),
            fetch,
            source["kind"].as_str().unwrap_or(""),
            source["location"].as_str().unwrap_or(""),
            source["title"].as_str().unwrap_or(""),
            source["channel"].as_str().unwrap_or(""),
        )
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "collection-discover" => {
                learning_helper("catalog", "", None, &Value::Null).map(|(value, _)| value)
            }
            "collection-pending-briefings" => self.pending_briefings(),
            "collection-mark-briefing" => self.mark_briefing(args),
            "collection-add" => self.add(args),
            "collection-media-queue" => {
                let urls = args["urls"]
                    .as_array()
                    .cloned()
                    .or_else(|| args["url"].as_str().map(|url| vec![json!(url)]))
                    .unwrap_or_default();
                for url in urls.iter().filter_map(Value::as_str).map(str::trim) {
                    if let Some(policy) = self.refusal(media_gate_kind(url), url, "", "")? {
                        return Err(invalid(format!("{url}: {}", refusal_text(&policy))));
                    }
                }
                self.media.command(command, args)
            }
            "collection-media-retry" => {
                let job = self.media.job(args)?;
                let url = job["url"].as_str().unwrap_or("");
                if let Some(policy) = self.refusal(media_gate_kind(url), url, "", "")? {
                    return Err(invalid(refusal_text(&policy)));
                }
                self.media.command(command, args)
            }
            "collection-watch-history" => self.watch_history(args),
            command if command.starts_with("collection-media-") => {
                self.media.command(command, args)
            }
            "collection-remove" => self.remove(args),
            "collection-list" => {
                let _ = self.poll(false)?;
                self.list(args)
            }
            "collection-check" => self.check(args),
            // Polling an active worker is observational. Cadence dispatch is
            // reserved for the service scheduler's explicit tick command.
            "collection-poll" => self.poll(false),
            "collection-poll-due" => self.poll(true),
            "collection-pending-imports" => self.pending_imports(),
            "collection-job-status" => self.job_status(args),
            "collection-ack" => self.ack(args),
            "collection-status" => {
                let _ = self.poll(false)?;
                self.status(args)
            }
            _ => Err(invalid(format!("Unknown collection command: {command}"))),
        }
    }

    fn add(&mut self, args: &Value) -> Result<Value> {
        let kind = args
            .get("type")
            .or_else(|| args.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(
            kind,
            "transcript"
                | "feed"
                | "browser-history"
                | "article"
                | "youtube-video"
                | "youtube-channel"
        ) {
            return Err(invalid(
                "Choose transcript, feed, browser-history, article or youtube-video",
            ));
        }
        let location = args
            .get("path")
            .or_else(|| args.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if location.starts_with("http://") || location.starts_with("https://") {
            if !matches!(
                kind,
                "feed" | "article" | "youtube-video" | "youtube-channel"
            ) {
                return Err(invalid("This source type requires a local file"));
            }
            if kind == "youtube-video" && youtube_video_id(location).is_none() {
                return Err(invalid("Choose a single YouTube video URL"));
            }
        } else {
            if matches!(kind, "article" | "youtube-video" | "youtube-channel") {
                return Err(invalid(
                    "Article and video sources require an HTTP or HTTPS URL",
                ));
            }
            let path = Path::new(location);
            reject_symlink_components(path, true)?;
            let metadata =
                fs::symlink_metadata(path).map_err(|_| invalid("Collection source is missing"))?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid("Collection source must be a regular file"));
            }
        }
        let cadence_hours = args["cadence_hours"].as_i64().unwrap_or(24);
        if !matches!(cadence_hours, 12 | 24) {
            return Err(invalid("Cadence must be 12 or 24 hours"));
        }
        let goals = args["goal_ids"].as_array().cloned().unwrap_or_default();
        if goals.len() > 32
            || goals
                .iter()
                .any(|id| id.as_str().is_none_or(|id| id.is_empty() || id.len() > 160))
        {
            return Err(invalid("Choose at most 32 valid goal identities"));
        }
        if kind == "browser-history" && !intake_switch(&self.prefs_root, "browser_history") {
            return Err(invalid(
                "Browser history intake is off. Turn it on in Settings to use it.",
            ));
        }
        let title = args["title"].as_str().unwrap_or("").trim();
        let channel = args["channel"].as_str().unwrap_or("").trim();
        if title.len() > 4096 || channel.len() > 512 {
            return Err(invalid("Source title or channel exceeds its bound"));
        }
        if let Some(policy) = self.refusal(kind, location, title, channel)? {
            return Err(invalid(refusal_text(&policy)));
        }
        let id = source_id(kind, location);
        if let Some((existing_id, existing)) = self.sources.iter().find(|(_, source)| {
            source["kind"].as_str() == Some(kind)
                && canonical_location(kind, source["location"].as_str().unwrap_or(""))
                    == canonical_location(kind, location)
        }) {
            let mut result = existing.clone();
            result["already_present"] = json!(true);
            result["job_id"] = self
                .jobs
                .values()
                .find(|job| {
                    job["source_id"] == *existing_id
                        && matches!(job["state"].as_str(), Some("queued" | "running"))
                })
                .map(|job| job["id"].clone())
                .unwrap_or(Value::Null);
            return Ok(result);
        }
        let source = json!({"schema":1,"id":id,"kind":kind,"location":location,"cadence_hours":cadence_hours,"enabled":args["enabled"].as_bool().unwrap_or(true),"auto_briefing":args["auto_briefing"].as_bool().unwrap_or(false),"goal_ids":goals,"title":if title.is_empty(){Value::Null}else{json!(title)},"channel":if channel.is_empty(){Value::Null}else{json!(channel)},"created_at":Utc::now().to_rfc3339(),"last_success":Value::Null,"last_error":Value::Null,"next_due_at":now_secs(),"capabilities":capabilities(kind,location)});
        atomic_json(
            &self.root.join("sources").join(format!("{id}.json")),
            &source,
        )?;
        self.sources.insert(id.clone(), source);
        self.enqueue_source(&id, true)?;
        self.explicit_sources.insert(id.clone());
        let requested = self.explicit_sources.clone();
        self.dispatch_due_for(Some(&requested))?;
        let mut result = self.sources[&id].clone();
        result["job_id"] = self
            .jobs
            .values()
            .find(|job| {
                job["source_id"] == id
                    && matches!(job["state"].as_str(), Some("queued" | "running"))
            })
            .map(|job| job["id"].clone())
            .unwrap_or(Value::Null);
        Ok(result)
    }

    fn remove(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Collection source id is required"))?;
        safe_component(id, "collection source id")?;
        if self.sources.remove(id).is_none() {
            return Err(invalid("Collection source not found"));
        }
        let _ = fs::remove_file(self.root.join("sources").join(format!("{id}.json")));
        for job in self.jobs.values_mut() {
            if job["source_id"] == id && (job["state"] == "queued" || job["state"] == "running") {
                if self.active_job.as_deref() == job["id"].as_str()
                    && let Some(cancel) = &self.active_cancel
                {
                    cancel.store(true, Ordering::Release);
                }
                job["state"] = json!("cancelled");
            }
        }
        self.persist_all_jobs()?;
        Ok(json!({"removed":id}))
    }

    /// The stored bytes of a source and a title for it, read before the source
    /// is removed so the Trash keeps it exactly.
    pub fn source_bytes(&self, id: &str) -> Result<(Vec<u8>, String)> {
        safe_component(id, "collection source id")?;
        let source = self
            .sources
            .get(id)
            .ok_or_else(|| invalid("Collection source not found"))?;
        let title = source_name(source);
        let bytes = read_bounded(
            &self.root.join("sources").join(format!("{id}.json")),
            MAX_RECORD_BYTES,
        )?;
        Ok((bytes, title))
    }

    /// Puts a removed source back from the bytes the Trash kept, unchanged.
    /// Jobs cancelled by the removal stay as they were; the source is due
    /// again on its own schedule.
    pub fn restore_source(&mut self, id: &str, bytes: &[u8]) -> Result<Value> {
        safe_component(id, "collection source id")?;
        if self.sources.contains_key(id) {
            return Err(invalid(
                "This source is already collected again; the copy in Trash stays there",
            ));
        }
        let mut source: Value = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("Corrupt collection source {id}: {error}")))?;
        if source["id"].as_str() != Some(id) {
            return Err(invalid(format!("Corrupt collection source {id}")));
        }
        atomic_bytes(&self.root.join("sources").join(format!("{id}.json")), bytes)?;
        source["capabilities"] = capabilities(
            source["kind"].as_str().unwrap_or(""),
            source["location"].as_str().unwrap_or(""),
        );
        self.sources.insert(id.to_owned(), source.clone());
        Ok(source)
    }

    fn check(&mut self, args: &Value) -> Result<Value> {
        let requested = args["id"].as_str();
        let mut refused = Vec::new();
        let ids = if let Some(id) = requested {
            let source = self
                .sources
                .get(id)
                .ok_or_else(|| invalid("Collection source not found"))?;
            if let Some(policy) = self.source_refusal_for(source, true)? {
                return Err(invalid(refusal_text(&policy)));
            }
            vec![id.to_owned()]
        } else {
            let mut ids = Vec::new();
            for (id, source) in &self.sources {
                match self.source_refusal_for(source, true)? {
                    Some(policy) => refused.push(
                        json!({"source_id":id,"location":source["location"],"policy":policy}),
                    ),
                    None => ids.push(id.clone()),
                }
            }
            ids
        };
        for id in &ids {
            self.enqueue_source(id, true)?;
        }
        self.explicit_sources.extend(ids.iter().cloned());
        let explicit_requested = self.explicit_sources.clone();
        self.dispatch_due_for(Some(&explicit_requested))?;
        let status = self.status(&json!({}))?;
        let job_ids = self
            .jobs
            .values()
            .filter(|job| {
                (requested.is_none() || job["source_id"].as_str() == requested)
                    && matches!(job["state"].as_str(), Some("queued" | "running"))
            })
            .filter_map(|job| job["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        Ok(json!({"queued":true,"job_ids":job_ids,"refused":refused,"status":status}))
    }

    fn enqueue_source(&mut self, id: &str, force: bool) -> Result<()> {
        if !self
            .sources
            .get(id)
            .is_some_and(|source| source["enabled"].as_bool().unwrap_or(false))
        {
            return Ok(());
        }
        if let Some(existing) = self
            .jobs
            .values()
            .find(|job| {
                job["source_id"] == id
                    && matches!(job["state"].as_str(), Some("queued" | "running"))
            })
            .cloned()
        {
            if force && existing["state"] == "queued" {
                let mut job = existing;
                job["due_at"] = json!(now_secs());
                let job_id = job["id"].as_str().unwrap().to_owned();
                atomic_json(&self.root.join("jobs").join(format!("{job_id}.json")), &job)?;
                self.jobs.insert(job_id, job);
            }
            return Ok(());
        }
        let source = self
            .sources
            .get(id)
            .ok_or_else(|| invalid("Collection source not found"))?;
        let due = if force {
            now_secs()
        } else {
            source["next_due_at"].as_i64().unwrap_or(now_secs())
        };
        // Keep the next cadence run durable even before it is due.  A future
        // queued job is the scheduler's persisted reservation; dispatch_due
        // will leave it queued until its due_at.  This also gives a forced
        // check a single existing job to advance instead of creating a
        // duplicate alongside the scheduled run.
        let id_job = job_id(id);
        let job = json!({"schema":1,"id":id_job,"source_id":id,"state":"queued","requested_at":Utc::now().to_rfc3339(),"due_at":due,"attempts":0,"auto_briefing":source["auto_briefing"]==true&&source["kind"]!="youtube-channel","goal_ids":source["goal_ids"]});
        atomic_json(&self.root.join("jobs").join(format!("{id_job}.json")), &job)?;
        self.jobs.insert(id_job, job);
        Ok(())
    }

    fn dispatch_due(&mut self) -> Result<()> {
        self.dispatch_due_for(None)
    }

    fn dispatch_due_for(
        &mut self,
        requested: Option<&std::collections::BTreeSet<String>>,
    ) -> Result<()> {
        if self.active_job.is_some() {
            return Ok(());
        }
        let now = now_secs();
        let id = self
            .jobs
            .values()
            .filter(|job| {
                job["state"] == "queued"
                    && job["due_at"].as_i64().unwrap_or(i64::MAX) <= now
                    && requested.is_none_or(|ids| {
                        job["source_id"]
                            .as_str()
                            .is_some_and(|source_id| ids.contains(source_id))
                    })
            })
            .min_by_key(|job| job["due_at"].as_i64().unwrap_or(i64::MAX))
            .and_then(|job| job["id"].as_str())
            .map(str::to_owned);
        let Some(id) = id else {
            return Ok(());
        };
        let source_id = self.jobs[&id]["source_id"]
            .as_str()
            .ok_or_else(|| invalid("Collection job has no source"))?
            .to_owned();
        let mut source = self
            .sources
            .get(&source_id)
            .ok_or_else(|| invalid("Collection source was removed"))?
            .clone();
        let mut job = self.jobs[&id].clone();
        if let Some(policy) = self.source_refusal_for(&source, true)? {
            // A rule set after the job was queued: the job ends refused and
            // nothing is collected.
            job["state"] = json!("cancelled");
            job["error"] = json!(refusal_text(&policy));
            job["finished_at"] = json!(Utc::now().to_rfc3339());
            atomic_json(&self.root.join("jobs").join(format!("{id}.json")), &job)?;
            self.jobs.insert(id, job);
            return Ok(());
        }
        if let Some(program) = &self.source_program {
            source["_ytdlp"] = json!(program);
        }
        if let Some(program) = &self.page_fetcher {
            source["_curl"] = json!(program);
        }
        // A channel's listing looks up each video's own metadata in the
        // worker, so each child is judged without the store waiting on it.
        if source["kind"] == "youtube-channel" && prefs_need_facts(&self.prefs_root) {
            source["_learn"] = json!(true);
            source["_known"] = json!(
                load_learned(&self.root)
                    .into_iter()
                    .filter(|(_, record)| record["state"] == "known")
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>()
            );
        }
        job["state"] = json!("running");
        job["started_at"] = json!(Utc::now().to_rfc3339());
        job["attempts"] = json!(job["attempts"].as_i64().unwrap_or(0) + 1);
        atomic_json(&self.root.join("jobs").join(format!("{id}.json")), &job)?;
        self.jobs.insert(id.clone(), job);
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker
            .tx
            .send(WorkerMessage::Run(WorkerRequest {
                job_id: id.clone(),
                source,
                cancel: Arc::clone(&cancel),
            }))
            .map_err(|_| invalid("Collection worker stopped"))?;
        self.active_cancel = Some(cancel);
        self.active_job = Some(id);
        Ok(())
    }

    /// Cadence admission: each due source passes his rules before a job is
    /// written; a refused one is reported and leaves nothing behind.
    fn enqueue_due(&mut self) -> Result<Vec<Value>> {
        let ids = self.sources.keys().cloned().collect::<Vec<_>>();
        let mut excluded = Vec::new();
        for id in ids {
            let source = &self.sources[&id];
            if source["enabled"] == false {
                continue;
            }
            // Only a source due now may wait on a lookup; the rest are judged
            // on what is already learned and again when they fall due.
            let due = source["next_due_at"].as_i64().unwrap_or(0) <= now_secs();
            if let Some(policy) = self.source_refusal_for(source, due)? {
                excluded
                    .push(json!({"source_id":id,"location":source["location"],"policy":policy}));
                continue;
            }
            self.enqueue_source(&id, false)?;
        }
        Ok(excluded)
    }

    fn poll(&mut self, dispatch_due: bool) -> Result<Value> {
        // Cadence work waits for his automatic-capture switch; with it off a
        // tick only drains what he asked for himself.
        let dispatch_due = dispatch_due && self.cadence_allowed();
        let excluded_due = if dispatch_due {
            self.enqueue_due()?
        } else {
            Vec::new()
        };
        let watch_history = if dispatch_due {
            self.watch_history_due()?
        } else {
            Value::Null
        };
        let mut completed = Vec::new();
        let mut failed = Vec::new();
        loop {
            match self.worker.rx.try_recv() {
                Ok(result) => {
                    self.active_job = None;
                    self.active_cancel = None;
                    let mut job = self
                        .jobs
                        .get(&result.job_id)
                        .cloned()
                        .ok_or_else(|| invalid("Collection worker returned an unknown job"))?;
                    let source_id = job["source_id"].as_str().unwrap_or("").to_owned();
                    if job["state"] == "cancelled" {
                        job["finished_at"] = json!(Utc::now().to_rfc3339());
                        atomic_json(
                            &self
                                .root
                                .join("jobs")
                                .join(format!("{}.json", result.job_id)),
                            &job,
                        )?;
                        self.jobs.insert(result.job_id, job);
                        continue;
                    }
                    match result.result {
                        Ok(value) => {
                            let mut normalized = value
                                .get("normalized")
                                .cloned()
                                .unwrap_or_else(|| value.clone());
                            let mut raw = result.raw;
                            if let Some(children) = value["child_sources"].as_array() {
                                let parent_source =
                                    self.sources.get(&source_id).cloned().unwrap_or(Value::Null);
                                let mut children_ids = Vec::new();
                                let mut refused_children = Vec::new();
                                let mut new_children = Vec::new();
                                for child in children.iter().take(20) {
                                    let id = source_id_for_child(child)?;
                                    let url = child["url"].as_str().unwrap_or("");
                                    let channel = child["channel"]
                                        .as_str()
                                        .filter(|value| !value.is_empty())
                                        .or_else(|| parent_source["channel"].as_str())
                                        .or_else(|| value["normalized"]["channel"].as_str())
                                        .unwrap_or("");
                                    let title = child["title"].as_str().unwrap_or("");
                                    // The worker read this video's own
                                    // metadata; it is kept before the gate.
                                    if child["meta"].is_object()
                                        && let Some(video_id) = youtube_video_id(url)
                                    {
                                        let meta = if child["meta"]["error"].is_string() {
                                            Err(invalid(
                                                child["meta"]["error"].as_str().unwrap_or(""),
                                            ))
                                        } else {
                                            Ok(child["meta"].clone())
                                        };
                                        remember_learned(&self.root, &video_id, meta)?;
                                    }
                                    // Each child passes his rules on its own:
                                    // an admitted channel never carries a
                                    // refused video into the pipeline.
                                    if let Some(policy) =
                                        self.known_refusal("youtube-video", url, title, channel)?
                                    {
                                        refused_children.push(json!({"source_id":id,"url":url,"title":title,"reason":refusal_text(&policy)}));
                                        continue;
                                    }
                                    if !self.sources.contains_key(&id) {
                                        let mut args = child.clone();
                                        if let Some(args) = args.as_object_mut() {
                                            args.remove("meta");
                                        }
                                        args["channel"] = json!(channel);
                                        args["cadence_hours"] =
                                            parent_source["cadence_hours"].clone();
                                        args["auto_briefing"] =
                                            parent_source["auto_briefing"].clone();
                                        args["goal_ids"] = parent_source["goal_ids"].clone();
                                        let _ = self.add(&args)?;
                                        new_children.push(id.clone());
                                    }
                                    children_ids.push(id);
                                }
                                job["child_sources"] = json!(children_ids);
                                job["new_child_sources"] = json!(new_children);
                                job["refused_children"] = json!(refused_children);
                            }
                            if job["refused_children"]
                                .as_array()
                                .is_some_and(|items| !items.is_empty())
                            {
                                // A refused child leaves nothing behind: its
                                // listing line leaves the channel's result and
                                // the provider's raw listing is not kept.
                                let refused = job["refused_children"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .filter_map(|item| item["url"].as_str().map(str::to_owned))
                                    .collect::<std::collections::BTreeSet<_>>();
                                if let Some(segments) = normalized["segments"].as_array_mut() {
                                    segments.retain(|segment| {
                                        !segment["url"]
                                            .as_str()
                                            .is_some_and(|url| refused.contains(url))
                                    });
                                }
                                raw = None;
                            }
                            let result_path = self
                                .root
                                .join("results")
                                .join(format!("{}.json", result.job_id));
                            atomic_json(&result_path, &normalized)?;
                            let raw_path = if let Some(raw) = raw.as_deref() {
                                let path = self
                                    .root
                                    .join("results")
                                    .join(format!("{}.source", result.job_id));
                                atomic_bytes(&path, raw)?;
                                Some(path)
                            } else {
                                None
                            };
                            job["state"] = json!("completed");
                            job["result_path"] = json!(result_path);
                            job["raw_path"] =
                                raw_path.clone().map_or(Value::Null, |path| json!(path));
                            job["acked"] = json!(false);
                            let segments_total =
                                normalized["segments"].as_array().map_or(0, Vec::len);
                            job["result"] = json!({"source_path":normalized["source_path"],"provider":"normalized","kind":normalized["kind"],"coverage":normalized["coverage"],"source_hash":normalized["source_hash"],"normalized_path":result_path,"raw_path":raw_path,"segments_total":segments_total});
                            completed.push(json!({"job_id":result.job_id,"source_id":source_id,"source_path":normalized["source_path"],"provider":"normalized","kind":normalized["kind"],"coverage":normalized["coverage"],"normalized_path":result_path,"raw_path":raw_path,"segments_total":segments_total}));
                            if let Some(source) = self.sources.get_mut(&source_id) {
                                source["last_success"] = json!(Utc::now().to_rfc3339());
                                source["last_error"] = Value::Null;
                                source["next_due_at"] = json!(
                                    now_secs()
                                        + source["cadence_hours"].as_i64().unwrap_or(24) * 3600
                                );
                                let source_copy = source.clone();
                                self.persist_source(&source_copy)?;
                            }
                        }
                        Err(error) => {
                            job["state"] = json!("failed");
                            job["error"] = json!(error.to_string());
                            job["finished_at"] = json!(Utc::now().to_rfc3339());
                            failed.push(json!({"job_id":result.job_id,"source_id":source_id,"error":error.to_string()}));
                            if let Some(source) = self.sources.get_mut(&source_id) {
                                source["last_error"] = json!(error.to_string());
                                source["next_due_at"] = json!(
                                    now_secs()
                                        + source["cadence_hours"].as_i64().unwrap_or(24) * 3600
                                );
                                let source_copy = source.clone();
                                self.persist_source(&source_copy)?;
                            }
                        }
                    }
                    job["finished_at"] = json!(Utc::now().to_rfc3339());
                    atomic_json(
                        &self
                            .root
                            .join("jobs")
                            .join(format!("{}.json", result.job_id)),
                        &job,
                    )?;
                    self.jobs.insert(result.job_id, job);
                    let active_sources = self
                        .jobs
                        .values()
                        .filter(|other| {
                            matches!(other["state"].as_str(), Some("queued" | "running"))
                        })
                        .filter_map(|other| other["source_id"].as_str().map(str::to_owned))
                        .collect::<std::collections::BTreeSet<_>>();
                    self.explicit_sources
                        .retain(|source_id| active_sources.contains(source_id));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(invalid("Collection worker stopped"));
                }
            }
        }
        if dispatch_due {
            self.dispatch_due()?;
        } else if !self.explicit_sources.is_empty() {
            let requested = self.explicit_sources.clone();
            self.dispatch_due_for(Some(&requested))?;
        }
        let media = self.media.poll()?;
        let pending_imports = self.pending_imports()?["items"].clone();
        Ok(
            json!({"completed":completed,"failed":failed,"pending":self.jobs.values().filter(|job|job["state"]=="queued"||job["state"]=="running").count(),"pending_imports":pending_imports,"media":media,"excluded_due":excluded_due,"watch_history":watch_history}),
        )
    }

    /// The local browser history file: an explicit one, else Chromium's or
    /// the first Firefox profile's, whichever exists.
    fn history_file(args: &Value) -> Option<(PathBuf, &'static str)> {
        if let Some(path) = args["path"].as_str() {
            let path = PathBuf::from(path);
            let flavour = if path.file_name().is_some_and(|name| name == "places.sqlite") {
                "firefox"
            } else {
                "chromium"
            };
            return Some((path, flavour));
        }
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let chromium = home.join(".config/chromium/Default/History");
        if chromium.is_file() {
            return Some((chromium, "chromium"));
        }
        let profiles = fs::read_dir(home.join(".mozilla/firefox")).ok()?;
        let mut places = profiles
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("places.sqlite"))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        places.sort();
        places.into_iter().next().map(|path| (path, "firefox"))
    }

    /// YouTube videos he watched, from his local browser history, as intake
    /// candidates he can pick for review.  Nothing is read unless his
    /// browser-history switch is on.  The live file is never opened: it is
    /// copied into the store's cache, read there as an immutable database
    /// and the copy is removed.  Each candidate passes his rules; a refused
    /// one is counted, not listed.
    fn watch_history(&mut self, args: &Value) -> Result<Value> {
        if !intake_switch(&self.prefs_root, "browser_history") {
            return Ok(json!({"enabled":false,"read":false,"items":[],"refused":0}));
        }
        let Some((path, flavour)) = Self::history_file(args) else {
            return Ok(
                json!({"enabled":true,"read":false,"items":[],"refused":0,"error":"No local browser history was found"}),
            );
        };
        reject_symlink_components(&path, true)?;
        private_dir(&self.root.join("cache"))?;
        let cache = self
            .root
            .join("cache")
            .join(format!("history-{}", now_nanos()));
        private_dir(&cache)?;
        let copy = cache.join("history.sqlite");
        let read = (|| -> Result<Value> {
            fs::copy(&path, &copy)?;
            let wal = PathBuf::from(format!("{}-wal", path.display()));
            if wal.is_file() {
                fs::copy(&wal, cache.join("history.sqlite-wal"))?;
            }
            let script = r#"import json, sqlite3, sys, urllib.parse
copy, flavour = sys.argv[1], sys.argv[2]
db = sqlite3.connect('file:' + urllib.parse.quote(copy) + '?immutable=1', uri=True)
if flavour == 'firefox':
    rows = db.execute("select url, title, visit_count, last_visit_date from moz_places where url like '%youtube.com/watch?%' order by last_visit_date desc limit 2000").fetchall()
    rows = [(u, t, c, (v or 0) // 1000) for u, t, c, v in rows]
else:
    rows = db.execute("select url, title, visit_count, last_visit_time from urls where url like '%youtube.com/watch?%' order by last_visit_time desc limit 2000").fetchall()
    # Chromium counts microseconds from 1601-01-01.
    rows = [(u, t, c, (v or 0) // 1000 - 11644473600000) for u, t, c, v in rows]
print(json.dumps([{'url': (u or '')[:4096], 'title': (t or '')[:1024], 'visit_count': c or 0, 'last_visit_ms': v} for u, t, c, v in rows], separators=(',', ':')))
"#;
            let mut command = Command::new("python3");
            command.args(["-I", "-c", script]).arg(&copy).arg(flavour);
            let output = run_bounded_process(
                command,
                std::time::Duration::from_secs(20),
                &AtomicBool::new(false),
                MAX_SOURCE_BYTES,
                "Browser history",
            )?;
            serde_json::from_slice(&output)
                .map_err(|_| invalid("Browser history reader returned malformed JSON"))
        })();
        for name in ["history.sqlite", "history.sqlite-wal"] {
            let _ = fs::remove_file(cache.join(name));
        }
        let _ = fs::remove_dir(&cache);
        let rows = read?;
        let mut by_video: BTreeMap<String, Value> = BTreeMap::new();
        for row in rows.as_array().into_iter().flatten() {
            let Some(video_id) = youtube_video_id(row["url"].as_str().unwrap_or("")) else {
                continue;
            };
            let entry = by_video.entry(video_id.clone()).or_insert_with(|| {
                json!({"video_id":video_id,"kind":"youtube-video","url":format!("https://www.youtube.com/watch?v={video_id}"),
                    "title":row["title"],"visit_count":0,"last_visit_ms":row["last_visit_ms"],
                    "signal":"watched","evidence":"local browser history"})
            });
            entry["visit_count"] = json!(
                entry["visit_count"].as_i64().unwrap_or(0)
                    + row["visit_count"].as_i64().unwrap_or(0)
            );
            if row["last_visit_ms"].as_i64() > entry["last_visit_ms"].as_i64() {
                entry["last_visit_ms"] = row["last_visit_ms"].clone();
                entry["title"] = row["title"].clone();
            }
        }
        let mut items = Vec::new();
        let mut refused = 0;
        for mut item in by_video.into_values() {
            let url = item["url"].as_str().unwrap_or("").to_owned();
            let title = item["title"].as_str().unwrap_or("").to_owned();
            if self
                .known_refusal("youtube-video", &url, &title, "")?
                .is_some()
            {
                refused += 1;
                continue;
            }
            item["last_watched_at"] = item["last_visit_ms"]
                .as_i64()
                .and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
                .map_or(Value::Null, |time| json!(time.to_rfc3339()));
            item["collected"] = json!(self.sources.contains_key(&source_id("youtube-video", &url)));
            items.push(item);
        }
        items.sort_by(|left, right| {
            right["last_visit_ms"]
                .as_i64()
                .cmp(&left["last_visit_ms"].as_i64())
        });
        let record = json!({"schema":1,"read_at":Utc::now().to_rfc3339(),"read_at_secs":now_secs(),"browser":flavour,"items":items,"refused":refused});
        atomic_json(
            &self.root.join("intake").join("watch-history.json"),
            &record,
        )?;
        Ok(
            json!({"enabled":true,"read":true,"browser":flavour,"items":record["items"],"refused":refused,"read_at":record["read_at"]}),
        )
    }

    /// At his cadence the watched-video list is refreshed with the channel
    /// checks, when his browser-history switch is on (twice a day at most).
    fn watch_history_due(&mut self) -> Result<Value> {
        if !intake_switch(&self.prefs_root, "browser_history") {
            return Ok(Value::Null);
        }
        let last = read_bounded(
            &self.root.join("intake").join("watch-history.json"),
            MAX_SOURCE_BYTES,
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|record| record["read_at_secs"].as_i64());
        if last.is_some_and(|last| now_secs() - last < 12 * 3600) {
            return Ok(Value::Null);
        }
        let result = self.watch_history(&json!({}))?;
        Ok(
            json!({"items":result["items"].as_array().map_or(0,Vec::len),"refused":result["refused"]}),
        )
    }

    fn pending_briefings(&self) -> Result<Value> {
        Ok(
            json!({"items":self.jobs.values().filter(|job|job["acked"]==true&&job["auto_briefing"]==true&&job["briefing"].is_null()).take(8).map(|job|json!({"job_id":job["id"],"history_ids":job["history_ids"],"goal_ids":job["goal_ids"]})).collect::<Vec<_>>()}),
        )
    }
    fn mark_briefing(&mut self, args: &Value) -> Result<Value> {
        let id = args["job_id"]
            .as_str()
            .ok_or_else(|| invalid("Collection job id required"))?;
        let mut job = self
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| invalid("Collection job not found"))?;
        if job["acked"] != true {
            return Err(invalid("Source evidence must be archived before briefing"));
        }
        job["briefing"] = args["briefing"].clone();
        atomic_json(&self.root.join("jobs").join(format!("{id}.json")), &job)?;
        self.jobs.insert(id.into(), job.clone());
        Ok(job)
    }

    fn pending_imports(&self) -> Result<Value> {
        let items = self.jobs.values().filter(|job| job["state"] == "completed" && job["acked"] != true).map(|job| {
            json!({"job_id":job["id"],"source_id":job["source_id"],"result_path":job["result_path"],"result":job["result"]})
        }).collect::<Vec<_>>();
        Ok(json!({"items":items}))
    }

    fn job_status(&self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Collection job id is required"))?;
        safe_component(id, "collection job id")?;
        let job = self
            .jobs
            .get(id)
            .ok_or_else(|| invalid("Collection job not found"))?;
        Ok(Self::compact_job(job))
    }

    fn ack(&mut self, args: &Value) -> Result<Value> {
        let id = args["job_id"]
            .as_str()
            .ok_or_else(|| invalid("Collection job id is required"))?;
        safe_component(id, "collection job id")?;
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| invalid("Collection job not found"))?;
        if job["state"] != "completed" {
            return Err(invalid(
                "Only completed collection jobs can be acknowledged",
            ));
        }
        let history_ids = args
            .get("history_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("collection-ack requires a history_ids array"))?;
        if history_ids.len() > 1024
            || history_ids.iter().any(|id| {
                id.as_str()
                    .is_none_or(|value| value.is_empty() || value.len() > 160)
            })
        {
            return Err(invalid(
                "collection-ack history_ids are invalid or too numerous",
            ));
        }
        if job["acked"] == true {
            if job["history_ids"] != Value::Array(history_ids.clone()) {
                return Err(invalid(
                    "Collection job is already acknowledged with different history_ids",
                ));
            }
            return Ok(
                json!({"job_id":id,"acked":true,"history_ids":job["history_ids"],"already_acknowledged":true}),
            );
        }
        job["acked"] = json!(true);
        job["history_ids"] = Value::Array(history_ids.clone());
        let copy = job.clone();
        atomic_json(&self.root.join("jobs").join(format!("{id}.json")), &copy)?;
        Ok(json!({"job_id":id,"acked":true,"history_ids":copy["history_ids"]}))
    }

    fn page(args: &Value, total: usize) -> (usize, usize, Option<String>) {
        let offset = args
            .get("cursor")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<usize>().ok())
            .or_else(|| {
                args.get("offset")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
            })
            .unwrap_or(0)
            .min(total);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 100) as usize;
        let end = offset.saturating_add(limit).min(total);
        (offset, end, (end < total).then(|| end.to_string()))
    }
    fn compact_job(job: &Value) -> Value {
        json!({
            "id": job["id"], "source_id": job["source_id"], "state": job["state"],
            "requested_at": job["requested_at"], "due_at": job["due_at"],
            "started_at": job["started_at"], "finished_at": job["finished_at"],
            "attempts": job["attempts"], "auto_briefing": job["auto_briefing"],
            "goal_ids": job["goal_ids"], "acked": job["acked"],
            "history_ids": job["history_ids"], "briefing": job["briefing"],
            "error": job["error"], "result": job["result"],
            "result_path": job["result_path"], "raw_path": job["raw_path"],
            "child_sources": job["child_sources"], "new_child_sources": job["new_child_sources"],
            "refused_children": job["refused_children"]
        })
    }
    fn list(&self, args: &Value) -> Result<Value> {
        let sources = self.sources.values().cloned().collect::<Vec<_>>();
        let jobs = self
            .jobs
            .values()
            .map(Self::compact_job)
            .collect::<Vec<_>>();
        let (source_start, source_end, source_next) = Self::page(args, sources.len());
        let (job_start, job_end, job_next) = Self::page(args, jobs.len());
        Ok(json!({
            "items": sources[source_start..source_end].to_vec(),
            "jobs": jobs[job_start..job_end].to_vec(),
            "limit": args.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 100),
            "next_cursor": source_next.or(job_next),
            "acquired_media": acquired_media(&self.root)?,
            "scope": "Only explicitly configured sources are checked on their selected cadence"
        }))
    }
    fn status(&self, args: &Value) -> Result<Value> {
        let sources = self.sources.values().cloned().collect::<Vec<_>>();
        let jobs = self
            .jobs
            .values()
            .map(Self::compact_job)
            .collect::<Vec<_>>();
        let (source_start, source_end, source_next) = Self::page(args, sources.len());
        let (job_start, job_end, job_next) = Self::page(args, jobs.len());
        Ok(json!({
            "sources": sources[source_start..source_end].to_vec(),
            "jobs": jobs[job_start..job_end].to_vec(),
            "queued": self.jobs.values().filter(|job| job["state"] == "queued").count(),
            "running": self.jobs.values().filter(|job| job["state"] == "running").count(),
            "worker_busy": self.active_job.is_some(),
            "next_cursor": source_next.or(job_next),
            "scope": "Only explicitly configured sources are checked on their selected cadence"
        }))
    }
    fn persist_source(&self, source: &Value) -> Result<()> {
        let id = source["id"]
            .as_str()
            .ok_or_else(|| invalid("Collection source has no id"))?;
        atomic_json(
            &self.root.join("sources").join(format!("{id}.json")),
            source,
        )
    }
    fn persist_all_jobs(&self) -> Result<()> {
        for job in self.jobs.values() {
            let id = job["id"]
                .as_str()
                .ok_or_else(|| invalid("Collection job has no id"))?;
            atomic_json(&self.root.join("jobs").join(format!("{id}.json")), job)?;
        }
        Ok(())
    }
}

fn load_records(root: &Path, label: &str) -> Result<BTreeMap<String, Value>> {
    let mut records = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
            return Err(invalid(format!(
                "Collection {label} archive contains an unexpected entry"
            )));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = name.strip_suffix(".json").ok_or_else(|| {
            invalid(format!(
                "Collection {label} archive contains an unexpected file"
            ))
        })?;
        safe_component(id, &format!("{label} id"))?;
        let bytes = read_bounded(&entry.path(), MAX_RECORD_BYTES)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("Corrupt collection {label} {id}: {error}")))?;
        if value["id"].as_str() != Some(id) {
            return Err(invalid(format!("Corrupt collection {label} {id}")));
        }
        records.insert(id.to_owned(), value);
        if records.len() > MAX_ITEMS {
            return Err(invalid("Collection store contains too many records"));
        }
    }
    Ok(records)
}

/// The name a source goes by where no title is shown: its own title or
/// channel, else what its location names (a file's name without its folder or
/// extension, a web address's site). Never a folder path.
#[must_use]
pub fn source_name(source: &Value) -> String {
    if let Some(name) = [&source["title"], &source["channel"]]
        .into_iter()
        .find_map(|v| v.as_str().map(str::trim).filter(|v| !v.is_empty()))
    {
        return name.to_owned();
    }
    let location = source["location"].as_str().unwrap_or("").trim();
    if let Some(rest) = location
        .strip_prefix("https://")
        .or_else(|| location.strip_prefix("http://"))
    {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        return host.strip_prefix("www.").unwrap_or(host).to_owned();
    }
    let file = Path::new(location);
    file.file_stem()
        .or_else(|| file.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn capabilities(kind: &str, location: &str) -> Value {
    json!({"discovery":true,"full_text":matches!(kind,"transcript"|"article"|"youtube-video"),"feed_text":kind=="feed","article_fetch":kind=="article","incremental":false,"attachments":false,"signals":if kind=="browser-history"{"visited_url_and_time_only"}else{"explicit_source"},"network":location.starts_with("http://")||location.starts_with("https://"),"state":"available"})
}

// ---------------------------------------------------------------------------
// Media acquisition queue.
//
// Public audio/video URLs are queued durably and downloaded one at a time by
// the installed yt-dlp in its own process group.  A job completes only after
// the child exited successfully, its readback names the requested source, the
// file lies in the job's own folder and ffprobe reads audio or video from it.
// Acquired files stay where they were downloaded; listings refer to them.

const MEDIA_MAX_QUEUE: usize = 20;
const MEDIA_MAX_JOBS: usize = 2_000;
const MEDIA_YOUTUBE_MAX_BYTES: u64 = 300 * 1024 * 1024;
const MEDIA_DIRECT_MAX_BYTES: u64 = 600 * 1024 * 1024;
const MEDIA_TIMEOUT_SECS: u64 = 1_800;
const MEDIA_FREE_RESERVE: u64 = 16 * 1024 * 1024 * 1024;
const MEDIA_PROBE_TIMEOUT_SECS: u64 = 120;
const MEDIA_RESULT_TAG: &str = "soliton-media-result\t";
const MEDIA_EXPECT_TAG: &str = "soliton-media-expected\t";

fn media_url_kind(url: &str) -> Result<(&'static str, Option<String>)> {
    if url.len() > 4096 || url.chars().any(char::is_control) {
        return Err(invalid(
            "Media URL is too long or contains control characters",
        ));
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(invalid("Media URLs must be public HTTP or HTTPS links"));
    }
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err(invalid("Media URL has no public host"));
    }
    if let Some(video_id) = youtube_video_id(url) {
        return Ok(("youtube", Some(video_id)));
    }
    let host = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    if host.ends_with("youtube.com") || host == "youtu.be" {
        return Err(invalid("Choose a single YouTube video URL"));
    }
    Ok(("direct", None))
}

/// The gate's source kind for a media URL: a YouTube video is judged as the
/// same source the collection store would hold for it.
fn media_gate_kind(url: &str) -> &'static str {
    if youtube_video_id(url).is_some() {
        "youtube-video"
    } else {
        "media"
    }
}

/// Picks the download from the formats the source offers today, never from a
/// fixed id: a progressive H.264 file of at most 360 lines, else H.264 video
/// of at most 360 lines with the original-language AAC audio, else that audio
/// alone.  Dubbed and audio-description tracks are passed over while an
/// original one exists.
fn choose_youtube_format(info: &Value) -> Result<String> {
    let formats = info["formats"].as_array().cloned().unwrap_or_default();
    let codec = |format: &Value, key: &str| format[key].as_str().unwrap_or("none").to_owned();
    let height = |format: &Value| format["height"].as_u64().unwrap_or(0);
    let id = |format: &Value| format["format_id"].as_str().unwrap_or("").to_owned();
    let usable = |format: &&Value| {
        !id(format).is_empty()
            && format["protocol"]
                .as_str()
                .is_none_or(|protocol| !protocol.contains("m3u8") && protocol != "mhtml")
    };
    let progressive = formats
        .iter()
        .filter(usable)
        .filter(|format| {
            codec(format, "vcodec") != "none"
                && codec(format, "acodec") != "none"
                && format["ext"] == "mp4"
                && (1..=360).contains(&height(format))
        })
        .max_by_key(|format| height(format));
    if let Some(format) = progressive {
        return Ok(id(format));
    }
    let audio = formats
        .iter()
        .filter(usable)
        .filter(|format| {
            codec(format, "vcodec") == "none"
                && codec(format, "acodec") != "none"
                && format["ext"] == "m4a"
        })
        .collect::<Vec<_>>();
    let note = |format: &Value| {
        format["format_note"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let original = audio
        .iter()
        .copied()
        .filter(|format| note(format).contains("original"))
        .collect::<Vec<_>>();
    let plain = audio
        .iter()
        .copied()
        .filter(|format| {
            !id(format).contains('-')
                && !note(format).contains("dubbed")
                && !note(format).contains("descri")
        })
        .collect::<Vec<_>>();
    let pool = if !original.is_empty() {
        original
    } else if !plain.is_empty() {
        plain
    } else {
        audio
    };
    let rate = |format: &Value| format["abr"].as_f64().unwrap_or(0.0);
    let chosen_audio = pool
        .iter()
        .copied()
        .filter(|format| rate(format) <= 160.0)
        .max_by(|left, right| rate(left).total_cmp(&rate(right)))
        .or_else(|| {
            pool.iter()
                .copied()
                .min_by(|left, right| rate(left).total_cmp(&rate(right)))
        });
    let video = formats
        .iter()
        .filter(usable)
        .filter(|format| {
            codec(format, "vcodec").starts_with("avc1")
                && codec(format, "acodec") == "none"
                && format["ext"] == "mp4"
                && (1..=360).contains(&height(format))
        })
        .max_by_key(|format| height(format));
    match (video, chosen_audio) {
        (Some(video), Some(audio)) => Ok(format!("{}+{}", id(video), id(audio))),
        (None, Some(audio)) => Ok(id(audio)),
        _ => Err(invalid(format!(
            "The source offers no H.264 video or AAC audio to download ({} formats listed)",
            formats.len()
        ))),
    }
}

/// Asks the downloader which formats the video offers now, in the job's own
/// process group, bounded in time and honouring cancellation.
fn probe_youtube_formats(request: &MediaRequest, url: &str) -> Result<String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(&request.program);
    command
        .args([
            "--ignore-config",
            "--no-playlist",
            "--no-cookies",
            "--no-cookies-from-browser",
            "--socket-timeout",
            "30",
            "-J",
            "--",
            url,
        ])
        .stdin(Stdio::null())
        .process_group(0);
    let output = run_bounded_logged(
        command,
        std::time::Duration::from_secs(MEDIA_PROBE_TIMEOUT_SECS),
        &request.cancel,
        MAX_SOURCE_BYTES,
        "Format probe",
        Some(&request.dir.join("yt-dlp-probe.log")),
    )?;
    let info: Value = serde_json::from_slice(&output)
        .map_err(|_| invalid("The format probe returned malformed JSON"))?;
    choose_youtube_format(&info)
}

fn media_canonical(url: &str) -> String {
    youtube_video_id(url)
        .map(|id| format!("https://www.youtube.com/watch?v={id}"))
        .unwrap_or_else(|| url.to_owned())
}

fn media_event(job: &mut Value, state: &str, detail: impl Into<String>) {
    let entry = json!({"at":Utc::now().to_rfc3339(),"state":state,"detail":detail.into()});
    if let Some(history) = job["history"].as_array_mut() {
        if history.len() >= 64 {
            history.remove(0);
        }
        history.push(entry);
    } else {
        job["history"] = json!([entry]);
    }
}

fn free_bytes(path: &Path) -> Result<u64> {
    let output = Command::new("df")
        .args(["--output=avail", "-B1", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| invalid(format!("Free space could not be read: {error}")))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .and_then(|line| line.trim().parse::<u64>().ok())
        .ok_or_else(|| invalid("Free space could not be read"))
}

/// Bytes of in-progress or abandoned partial files in a job folder.
fn partial_files(dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.ends_with(".part") || name.contains(".part-Frag") || name.ends_with(".ytdl"))
                .then(
                    || json!({"name":name,"bytes":entry.metadata().map(|m| m.len()).unwrap_or(0)}),
                )
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    items
}

fn last_error_line(log: &Path) -> Option<String> {
    let bytes = fs::read(log).ok()?;
    let text =
        String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(64 * 1024)..]).into_owned();
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines
        .iter()
        .rev()
        .find(|line| line.starts_with("ERROR:"))
        .or_else(|| lines.last())
        .map(|line| line.chars().take(1000).collect())
}

fn sha256_file(path: &Path, stop: &AtomicBool) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(invalid("Media validation was cancelled"));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn probe_media(path: &Path) -> Result<Value> {
    let output = Command::new("/usr/bin/ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=format_name,duration,size:stream=codec_type,codec_name,width,height",
            "-of",
            "json",
            "--",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| invalid(format!("ffprobe unavailable: {error}")))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "ffprobe could not read the downloaded file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| invalid("ffprobe returned malformed JSON"))?;
    let streams = value["streams"].as_array().cloned().unwrap_or_default();
    let has_media = streams
        .iter()
        .any(|stream| matches!(stream["codec_type"].as_str(), Some("audio" | "video")));
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0);
    if !has_media || duration.is_none() {
        return Err(invalid(
            "Downloaded file has no playable audio or video stream",
        ));
    }
    Ok(json!({
        "format_name": value["format"]["format_name"],
        "duration_seconds": duration,
        "streams": streams,
    }))
}

struct MediaRequest {
    program: PathBuf,
    free_reserve: u64,
    job: Value,
    dir: PathBuf,
    cancel: Arc<AtomicBool>,
}
struct MediaOutcome {
    job_id: String,
    pid: Option<u32>,
    result: Result<Value>,
}
enum MediaMessage {
    Run(MediaRequest),
    Format(String, String),
    Started(String, u32),
    Done(MediaOutcome),
}

fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Runs one download to the end: child exit, readback, identity and probe.
fn run_media_job(
    request: &MediaRequest,
    started: &Sender<MediaMessage>,
) -> (Option<u32>, Result<Value>) {
    use std::os::unix::process::CommandExt;
    let job = &request.job;
    let id = job["id"].as_str().unwrap_or("").to_owned();
    let url = job["url"].as_str().unwrap_or("").to_owned();
    let max_bytes = job["max_bytes"].as_u64().unwrap_or(MEDIA_DIRECT_MAX_BYTES);
    let timeout =
        std::time::Duration::from_secs(job["timeout_secs"].as_u64().unwrap_or(MEDIA_TIMEOUT_SECS));
    if let Err(error) = private_dir(&request.dir) {
        return (None, Err(error));
    }
    match free_bytes(&request.dir) {
        Ok(free) if free >= request.free_reserve + max_bytes => {}
        Ok(free) => {
            return (
                None,
                Err(invalid(format!(
                    "Not enough free space: {free} bytes available, {} required",
                    request.free_reserve + max_bytes
                ))),
            );
        }
        Err(error) => return (None, Err(error)),
    }
    let out_path = request.dir.join("yt-dlp.out");
    let log_path = request.dir.join("yt-dlp.log");
    let (Ok(out), Ok(log)) = (File::create(&out_path), File::create(&log_path)) else {
        return (None, Err(invalid("Media job log could not be created")));
    };
    let mut command = Command::new(&request.program);
    command
        .args([
            "--ignore-config",
            "--no-playlist",
            "--no-cookies",
            "--no-cookies-from-browser",
            "--no-mtime",
            "--no-overwrites",
            "--continue",
            "--socket-timeout",
            "30",
            "--retries",
            "3",
            "--max-filesize",
        ])
        .arg(max_bytes.to_string())
        .arg("-P")
        .arg(&request.dir)
        .args(["-o", "%(id).80B.%(ext)s", "--no-simulate"]);
    if job["source_kind"] == "youtube" {
        let format = match probe_youtube_formats(request, &url) {
            Ok(format) => format,
            Err(error) => return (None, Err(error)),
        };
        let _ = started.send(MediaMessage::Format(id.clone(), format.clone()));
        command.args([
            "-f",
            &format,
            "--merge-output-format",
            "mp4",
            "--ffmpeg-location",
            "/usr/bin/ffmpeg",
        ]);
    }
    command
        .arg("--print")
        .arg(format!("before_dl:{MEDIA_EXPECT_TAG}%(filesize,filesize_approx|0)d"))
        .arg("--print")
        .arg(format!(
            "after_move:{MEDIA_RESULT_TAG}%(.{{id,webpage_url,original_url,extractor_key,format_id,language,ext,title,channel,duration,filesize,filesize_approx,filepath}})j"
        ))
        .arg("--")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(log)
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return (None, Err(invalid(format!("yt-dlp unavailable: {error}")))),
    };
    let pid = child.id();
    let _ = started.send(MediaMessage::Started(id.clone(), pid));
    let begun = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                kill_group(pid);
                let _ = child.wait();
                return (Some(pid), Err(error.into()));
            }
        }
        let cancelled = request.cancel.load(Ordering::Acquire);
        let oversized = partial_files(&request.dir)
            .iter()
            .map(|file| file["bytes"].as_u64().unwrap_or(0))
            .sum::<u64>()
            > max_bytes;
        if cancelled || oversized || begun.elapsed() >= timeout {
            kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            let reason = if cancelled {
                "Cancelled; the owned download process group was stopped"
            } else if oversized {
                "Download exceeded its size limit and was stopped"
            } else {
                "Download exceeded its time limit and was stopped"
            };
            return (Some(pid), Err(invalid(reason)));
        }
        thread::sleep(std::time::Duration::from_millis(200));
    };
    if !status.success() {
        let detail = last_error_line(&log_path).unwrap_or_else(|| "no error text".into());
        return (
            Some(pid),
            Err(invalid(format!("yt-dlp failed ({status}): {detail}"))),
        );
    }
    (Some(pid), validate_media(request, &out_path, &url))
}

fn validate_media(request: &MediaRequest, out_path: &Path, url: &str) -> Result<Value> {
    let job = &request.job;
    let output = read_bounded(out_path, 1024 * 1024)?;
    let text = String::from_utf8_lossy(&output);
    let readback: Value = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(MEDIA_RESULT_TAG))
        .and_then(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| {
            invalid(format!(
                "yt-dlp exited without producing a file: {}",
                last_error_line(&request.dir.join("yt-dlp.log"))
                    .or_else(|| text.lines().last().map(str::to_owned))
                    .unwrap_or_else(|| "no readback".into())
            ))
        })?;
    // Identity: the readback must name the requested source.
    if job["source_kind"] == "youtube" {
        if readback["id"].as_str() != job["video_id"].as_str() {
            return Err(invalid(format!(
                "Downloaded video {} does not match the requested video {}",
                readback["id"], job["video_id"]
            )));
        }
    } else {
        let requested = media_canonical(url);
        let named = [&readback["original_url"], &readback["webpage_url"]]
            .iter()
            .filter_map(|value| value.as_str())
            .any(|value| value == requested);
        if !named {
            return Err(invalid(format!(
                "Downloaded readback names {} instead of the requested URL",
                readback["webpage_url"]
            )));
        }
    }
    let path = PathBuf::from(
        readback["filepath"]
            .as_str()
            .ok_or_else(|| invalid("yt-dlp readback has no file path"))?,
    );
    let dir = fs::canonicalize(&request.dir)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| invalid("Downloaded file named by yt-dlp is missing"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(invalid("Downloaded file is not a non-empty regular file"));
    }
    if path.parent().map(fs::canonicalize).transpose()?.as_deref() != Some(dir.as_path()) {
        return Err(invalid("Downloaded file is outside its job folder"));
    }
    let max_bytes = job["max_bytes"].as_u64().unwrap_or(MEDIA_DIRECT_MAX_BYTES);
    if metadata.len() > max_bytes {
        return Err(invalid("Downloaded file exceeds its size limit"));
    }
    let probe = probe_media(&path)?;
    let probed = probe["duration_seconds"].as_f64().unwrap_or(0.0);
    if let Some(expected) = readback["duration"].as_f64().filter(|value| *value > 0.0)
        && (probed - expected).abs() > (expected * 0.01).max(2.0)
    {
        return Err(invalid(format!(
            "Downloaded duration {probed:.2} s does not match the source's {expected:.2} s"
        )));
    }
    let sha256 = sha256_file(&path, &request.cancel)?;
    Ok(json!({
        "path": path,
        "file_name": path.file_name().map(|name| name.to_string_lossy().into_owned()),
        "bytes": metadata.len(),
        "sha256": sha256,
        "format_name": probe["format_name"],
        "duration_seconds": probe["duration_seconds"],
        "streams": probe["streams"],
        "readback": {
            "id": readback["id"], "webpage_url": readback["webpage_url"],
            "original_url": readback["original_url"], "extractor": readback["extractor_key"],
            "format_id": readback["format_id"], "language": readback["language"],
            "ext": readback["ext"], "title": readback["title"], "channel": readback["channel"],
            "duration": readback["duration"],
        },
    }))
}

fn spawn_media_worker() -> Result<(Sender<MediaMessage>, Receiver<MediaMessage>, JoinHandle<()>)> {
    let (request_tx, request_rx) = mpsc::channel::<MediaMessage>();
    let (result_tx, result_rx) = mpsc::channel::<MediaMessage>();
    let handle = thread::Builder::new()
        .name("soliton-media".into())
        .spawn(move || {
            // One request at a time: the queue is sequential by construction.
            while let Ok(MediaMessage::Run(request)) = request_rx.recv() {
                let job_id = request.job["id"].as_str().unwrap_or("").to_owned();
                let (pid, result) = run_media_job(&request, &result_tx);
                if result_tx
                    .send(MediaMessage::Done(MediaOutcome {
                        job_id,
                        pid,
                        result,
                    }))
                    .is_err()
                {
                    break;
                }
            }
        })
        .map_err(|error| invalid(format!("Could not start media worker: {error}")))?;
    Ok((request_tx, result_rx, handle))
}

/// Whether a durable media job waits to run; lets the service resume the
/// queue after a restart without opening the store otherwise.
pub fn media_work_pending(root: &Path) -> bool {
    load_media_jobs(&root.join("media").join("jobs"))
        .map(|jobs| {
            jobs.values()
                .any(|job| matches!(job["state"].as_str(), Some("queued" | "running")))
        })
        .unwrap_or(false)
}

fn load_media_jobs(dir: &Path) -> Result<BTreeMap<String, Value>> {
    if !dir.exists() {
        return Ok(BTreeMap::new());
    }
    load_records(dir, "media job")
}

fn media_item(job: &Value, dir: &Path) -> Value {
    let partial = if job["state"] == "completed" {
        Vec::new()
    } else {
        partial_files(dir)
    };
    let downloaded = partial
        .iter()
        .map(|file| file["bytes"].as_u64().unwrap_or(0))
        .sum::<u64>();
    let expected = fs::read(dir.join("yt-dlp.out"))
        .ok()
        .and_then(|bytes| {
            String::from_utf8_lossy(&bytes).lines().find_map(|line| {
                line.strip_prefix(MEDIA_EXPECT_TAG)
                    .and_then(|v| v.trim().parse::<u64>().ok())
            })
        })
        .filter(|value| *value > 0);
    json!({
        "id": job["id"], "seq": job["seq"], "url": job["url"], "source_kind": job["source_kind"],
        "video_id": job["video_id"], "state": job["state"], "stage": job["stage"],
        "requested_at": job["requested_at"], "started_at": job["started_at"],
        "finished_at": job["finished_at"], "attempts": job["attempts"], "error": job["error"],
        "max_bytes": job["max_bytes"], "history": job["history"], "result": job["result"],
        "partial": {"files": partial, "bytes": downloaded, "expected_bytes": expected},
    })
}

/// Completed downloads as read-only media entries for the Media and inbox
/// listings.  Nothing is moved or copied; the entry names the acquired file.
pub fn acquired_media(root: &Path) -> Result<Vec<Value>> {
    let jobs = load_media_jobs(&root.join("media").join("jobs"))?;
    let mut items = jobs
        .values()
        .filter(|job| job["state"] == "completed")
        .map(|job| {
            let result = &job["result"];
            let present = result["path"].as_str().is_some_and(|path| {
                fs::symlink_metadata(path)
                    .is_ok_and(|meta| meta.is_file() && Some(meta.len()) == result["bytes"].as_u64())
            });
            json!({
                "id": job["id"], "kind": "acquired-media", "source_intent": "media",
                "state": if present {"available"} else {"missing"},
                "url": job["url"], "title": result["readback"]["title"],
                "original_filename": result["file_name"], "filename": result["file_name"],
                "size": result["bytes"], "sha256": result["sha256"],
                "format_name": result["format_name"], "duration_seconds": result["duration_seconds"],
                "acquired_at": job["finished_at"],
                "receipt": {"path": result["path"], "url": job["url"], "job_id": job["id"]},
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right["acquired_at"]
            .as_str()
            .cmp(&left["acquired_at"].as_str())
    });
    Ok(items)
}

struct MediaQueue {
    root: PathBuf,
    jobs: BTreeMap<String, Value>,
    tx: Sender<MediaMessage>,
    rx: Receiver<MediaMessage>,
    handle: Option<JoinHandle<()>>,
    active: Option<(String, Arc<AtomicBool>)>,
    program: PathBuf,
    free_reserve: u64,
    prefs_root: PathBuf,
}

impl MediaQueue {
    fn open(root: &Path, prefs_root: &Path) -> Result<Self> {
        let root = root.join("media");
        private_dir(&root)?;
        private_dir(&root.join("jobs"))?;
        private_dir(&root.join("files"))?;
        let mut jobs = load_media_jobs(&root.join("jobs"))?;
        for job in jobs.values_mut() {
            if job["state"] == "running" {
                // The previous service ended mid-download.  Its process group
                // is stopped only when /proc still shows our own yt-dlp
                // writing into this job's folder; the job runs again.
                if let Some(pid) = job["pid"].as_u64() {
                    let dir = root.join("files").join(job["id"].as_str().unwrap_or(""));
                    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                    let cmdline = String::from_utf8_lossy(&cmdline);
                    if cmdline.contains("yt-dlp")
                        && cmdline.contains(dir.to_string_lossy().as_ref())
                    {
                        kill_group(pid as u32);
                    }
                }
                job["state"] = json!("queued");
                job["stage"] = json!("queued");
                job["pid"] = Value::Null;
                media_event(
                    job,
                    "queued",
                    "Recovered after restart; partial files are kept and resumed",
                );
            }
        }
        let (tx, rx, handle) = spawn_media_worker()?;
        let queue = Self {
            root,
            jobs,
            tx,
            rx,
            handle: Some(handle),
            active: None,
            program: PathBuf::from("/usr/bin/yt-dlp"),
            free_reserve: MEDIA_FREE_RESERVE,
            prefs_root: prefs_root.to_path_buf(),
        };
        for job in queue.jobs.values() {
            queue.persist(job)?;
        }
        Ok(queue)
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join("files").join(id)
    }

    fn persist(&self, job: &Value) -> Result<()> {
        let id = job["id"]
            .as_str()
            .ok_or_else(|| invalid("Media job has no id"))?;
        atomic_json(&self.root.join("jobs").join(format!("{id}.json")), job)
    }

    fn job(&self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Media job id is required"))?;
        safe_component(id, "media job id")?;
        self.jobs
            .get(id)
            .cloned()
            .ok_or_else(|| invalid("Media job not found"))
    }

    fn queue(&mut self, args: &Value) -> Result<Value> {
        let urls = args["urls"]
            .as_array()
            .cloned()
            .or_else(|| args["url"].as_str().map(|url| vec![json!(url)]))
            .ok_or_else(|| invalid("Paste one or more media URLs"))?;
        if urls.is_empty() || urls.len() > MEDIA_MAX_QUEUE {
            return Err(invalid(format!(
                "Queue between 1 and {MEDIA_MAX_QUEUE} media URLs at a time"
            )));
        }
        if self.jobs.len() + urls.len() > MEDIA_MAX_JOBS {
            return Err(invalid("The media queue is full"));
        }
        // Validate every URL before any is queued, so a bad paste queues nothing.
        let mut parsed = Vec::new();
        for url in &urls {
            let url = url
                .as_str()
                .map(str::trim)
                .ok_or_else(|| invalid("Media URLs must be text"))?;
            let (kind, video_id) = media_url_kind(url)?;
            parsed.push((url.to_owned(), kind, video_id));
        }
        let mut seq = self
            .jobs
            .values()
            .filter_map(|job| job["seq"].as_u64())
            .max()
            .unwrap_or(0);
        let mut items = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (url, kind, video_id) in parsed {
            let canonical = media_canonical(&url);
            if !seen.insert(canonical.clone()) {
                continue;
            }
            if let Some(existing) = self.jobs.values().find(|job| {
                media_canonical(job["url"].as_str().unwrap_or("")) == canonical
                    && matches!(
                        job["state"].as_str(),
                        Some("queued" | "running" | "completed")
                    )
            }) {
                let mut item =
                    media_item(existing, &self.dir(existing["id"].as_str().unwrap_or("")));
                item["already_present"] = json!(true);
                items.push(item);
                continue;
            }
            seq += 1;
            let id = format!(
                "media-{}-{}",
                ContentHash::digest(format!("{canonical}\0{}", now_nanos()).as_bytes())
                    .short_hex(8),
                now_nanos()
            );
            let max_bytes = if kind == "youtube" {
                MEDIA_YOUTUBE_MAX_BYTES
            } else {
                MEDIA_DIRECT_MAX_BYTES
            };
            let mut job = json!({
                "schema": 1, "id": id, "seq": seq, "url": url, "source_kind": kind,
                "video_id": video_id, "state": "queued", "stage": "queued",
                "requested_at": Utc::now().to_rfc3339(), "started_at": null, "finished_at": null,
                "attempts": 0, "error": null, "pid": null, "max_bytes": max_bytes,
                "timeout_secs": MEDIA_TIMEOUT_SECS,
                "format": if kind == "youtube" { Value::Null } else { json!("source") },
                "result": null, "history": [],
            });
            media_event(&mut job, "queued", "Queued for sequential download");
            self.persist(&job)?;
            items.push(media_item(&job, &self.dir(&id)));
            self.jobs.insert(id, job);
        }
        self.dispatch()?;
        Ok(
            json!({"items": items, "queued": self.count("queued"), "running": self.count("running")}),
        )
    }

    fn count(&self, state: &str) -> usize {
        self.jobs
            .values()
            .filter(|job| job["state"] == state)
            .count()
    }

    fn dispatch(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let Some(id) = self
            .jobs
            .values()
            .filter(|job| job["state"] == "queued")
            .min_by_key(|job| job["seq"].as_u64().unwrap_or(u64::MAX))
            .and_then(|job| job["id"].as_str())
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let mut job = self.jobs[&id].clone();
        let url = job["url"].as_str().unwrap_or("").to_owned();
        let collection_root = self.root.parent().unwrap_or(&self.root).to_path_buf();
        let tools = json!({"ytdlp":self.program.to_string_lossy()});
        if let Some(policy) = source_refusal(
            &self.prefs_root,
            &collection_root,
            &tools,
            true,
            media_gate_kind(&url),
            &url,
            "",
            "",
        )? {
            // A rule set while it waited (or before a restart): nothing runs.
            job["state"] = json!("cancelled");
            job["stage"] = json!("cancelled");
            job["error"] = json!(refusal_text(&policy));
            job["finished_at"] = json!(Utc::now().to_rfc3339());
            media_event(&mut job, "cancelled", refusal_text(&policy));
            self.persist(&job)?;
            self.jobs.insert(id, job);
            return self.dispatch();
        }
        job["state"] = json!("running");
        job["stage"] = json!("downloading");
        job["started_at"] = json!(Utc::now().to_rfc3339());
        job["finished_at"] = Value::Null;
        job["error"] = Value::Null;
        job["attempts"] = json!(job["attempts"].as_u64().unwrap_or(0) + 1);
        media_event(&mut job, "running", "Download started");
        self.persist(&job)?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.tx
            .send(MediaMessage::Run(MediaRequest {
                program: self.program.clone(),
                free_reserve: self.free_reserve,
                job: job.clone(),
                dir: self.dir(&id),
                cancel: Arc::clone(&cancel),
            }))
            .map_err(|_| invalid("Media worker stopped"))?;
        self.jobs.insert(id.clone(), job);
        self.active = Some((id, cancel));
        Ok(())
    }

    fn poll(&mut self) -> Result<Value> {
        let mut completed = Vec::new();
        let mut failed = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(MediaMessage::Format(id, format)) => {
                    if let Some(mut job) = self.jobs.get(&id).cloned() {
                        job["format"] = json!(format);
                        media_event(
                            &mut job,
                            "running",
                            format!("Format {format} chosen from the source's offer"),
                        );
                        self.persist(&job)?;
                        self.jobs.insert(id, job);
                    }
                }
                Ok(MediaMessage::Started(id, pid)) => {
                    if let Some(mut job) = self.jobs.get(&id).cloned() {
                        job["pid"] = json!(pid);
                        self.persist(&job)?;
                        self.jobs.insert(id, job);
                    }
                }
                Ok(MediaMessage::Done(outcome)) => {
                    let cancelled = self.active.as_ref().is_some_and(|(id, cancel)| {
                        *id == outcome.job_id && cancel.load(Ordering::Acquire)
                    });
                    self.active = None;
                    let Some(mut job) = self.jobs.get(&outcome.job_id).cloned() else {
                        continue;
                    };
                    job["pid"] = Value::Null;
                    job["last_pid"] = outcome.pid.map_or(Value::Null, |pid| json!(pid));
                    job["finished_at"] = json!(Utc::now().to_rfc3339());
                    match outcome.result {
                        Ok(result) => {
                            job["state"] = json!("completed");
                            job["stage"] = json!("completed");
                            job["result"] = result;
                            media_event(
                                &mut job,
                                "completed",
                                "Downloaded, identity checked and probed",
                            );
                            completed.push(job["id"].clone());
                        }
                        Err(error) if cancelled => {
                            job["state"] = json!("cancelled");
                            job["stage"] = json!("cancelled");
                            job["error"] = json!(error.to_string());
                            media_event(&mut job, "cancelled", error.to_string());
                        }
                        Err(error) => {
                            job["state"] = json!("failed");
                            job["stage"] = json!("failed");
                            job["error"] = json!(error.to_string());
                            media_event(&mut job, "failed", error.to_string());
                            failed.push(json!({"id":job["id"],"error":job["error"]}));
                        }
                    }
                    self.persist(&job)?;
                    self.jobs.insert(outcome.job_id, job);
                }
                Ok(MediaMessage::Run(_)) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(invalid("Media worker stopped")),
            }
        }
        self.dispatch()?;
        Ok(
            json!({"completed": completed, "failed": failed, "queued": self.count("queued"), "running": self.count("running")}),
        )
    }

    fn list(&self, args: &Value) -> Result<Value> {
        let mut jobs = self.jobs.values().collect::<Vec<_>>();
        jobs.sort_by_key(|job| job["seq"].as_u64().unwrap_or(0));
        let (start, end, next) = CollectionStore::page(args, jobs.len());
        let items = jobs[start..end]
            .iter()
            .map(|job| media_item(job, &self.dir(job["id"].as_str().unwrap_or(""))))
            .collect::<Vec<_>>();
        Ok(json!({
            "items": items,
            "acquired": acquired_media(self.root.parent().unwrap_or(&self.root))?,
            "queued": self.count("queued"), "running": self.count("running"),
            "active_job": self.active.as_ref().map(|(id, _)| json!(id)).unwrap_or(Value::Null),
            "next_cursor": next,
            "limits": {"youtube_max_bytes": MEDIA_YOUTUBE_MAX_BYTES, "direct_max_bytes": MEDIA_DIRECT_MAX_BYTES,
                       "timeout_secs": MEDIA_TIMEOUT_SECS, "youtube_format": "chosen from the source's offer",
                       "concurrent_downloads": 1},
        }))
    }

    fn cancel(&mut self, args: &Value) -> Result<Value> {
        let mut job = self.job(args)?;
        let id = job["id"].as_str().unwrap_or("").to_owned();
        match job["state"].as_str() {
            Some("queued") => {
                job["state"] = json!("cancelled");
                job["stage"] = json!("cancelled");
                job["finished_at"] = json!(Utc::now().to_rfc3339());
                media_event(&mut job, "cancelled", "Cancelled before it started");
                self.persist(&job)?;
                self.jobs.insert(id.clone(), job.clone());
            }
            Some("running") => {
                if let Some((active, cancel)) = &self.active
                    && *active == id
                {
                    cancel.store(true, Ordering::Release);
                }
                job["stage"] = json!("cancelling");
                media_event(&mut job, "running", "Cancellation requested");
                self.persist(&job)?;
                self.jobs.insert(id.clone(), job.clone());
            }
            _ => {
                return Err(invalid(
                    "Only queued or running media jobs can be cancelled",
                ));
            }
        }
        Ok(media_item(&job, &self.dir(&id)))
    }

    fn retry(&mut self, args: &Value) -> Result<Value> {
        let mut job = self.job(args)?;
        if !matches!(job["state"].as_str(), Some("failed" | "cancelled")) {
            return Err(invalid(
                "Only failed or cancelled media jobs can be retried",
            ));
        }
        let id = job["id"].as_str().unwrap_or("").to_owned();
        let seq = self
            .jobs
            .values()
            .filter_map(|job| job["seq"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        job["state"] = json!("queued");
        job["stage"] = json!("queued");
        job["seq"] = json!(seq);
        job["previous_error"] = job["error"].take();
        job["error"] = Value::Null;
        job["finished_at"] = Value::Null;
        media_event(
            &mut job,
            "queued",
            "Retry requested; partial files are resumed",
        );
        self.persist(&job)?;
        self.jobs.insert(id.clone(), job.clone());
        self.dispatch()?;
        Ok(media_item(&self.jobs[&id], &self.dir(&id)))
    }

    fn open_item(&self, args: &Value) -> Result<Value> {
        let job = self.job(args)?;
        if job["state"] != "completed" {
            return Err(invalid("Only completed downloads can be opened"));
        }
        let result = &job["result"];
        let path = PathBuf::from(
            result["path"]
                .as_str()
                .ok_or_else(|| invalid("Media file path missing"))?,
        );
        reject_symlink_components(&path, true)?;
        let dir = fs::canonicalize(self.dir(job["id"].as_str().unwrap_or("")))?;
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| invalid("The downloaded file is missing"))?;
        if !metadata.is_file()
            || path.parent().map(fs::canonicalize).transpose()?.as_deref() != Some(dir.as_path())
        {
            return Err(invalid("The downloaded file is not in its job folder"));
        }
        if Some(metadata.len()) != result["bytes"].as_u64() {
            return Err(invalid(
                "The downloaded file changed size since it was checked",
            ));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = match ext.as_str() {
            "mp4" | "m4v" => "video/mp4",
            "webm" => "video/webm",
            "mkv" => "video/x-matroska",
            "m4a" => "audio/mp4",
            "mp3" => "audio/mpeg",
            "opus" | "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        };
        let encoded = path
            .to_string_lossy()
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect::<String>();
        Ok(json!({
            "id": job["id"], "path": path, "file_url": format!("file://{encoded}"), "mime": mime,
            "title": result["readback"]["title"], "source_url": job["url"], "bytes": metadata.len(),
            "sha256": result["sha256"], "duration_seconds": result["duration_seconds"],
        }))
    }

    fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "collection-media-queue" => self.queue(args),
            "collection-media-list" => {
                self.poll()?;
                self.list(args)
            }
            "collection-media-status" => {
                self.poll()?;
                let job = self.job(args)?;
                Ok(media_item(
                    &job,
                    &self.dir(job["id"].as_str().unwrap_or("")),
                ))
            }
            "collection-media-cancel" => self.cancel(args),
            "collection-media-retry" => self.retry(args),
            "collection-media-open" => self.open_item(args),
            _ => Err(invalid(format!("Unknown collection command: {command}"))),
        }
    }
}

impl Drop for MediaQueue {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        // Dropping the sender ends the worker loop after its current job.
        let (dead, _) = mpsc::channel();
        self.tx = dead;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
