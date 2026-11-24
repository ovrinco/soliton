//! Reloadable classification lens; native records and revisions remain intact.
use crate::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct ChatKinds {
    schema: String,
    default_kind: String,
    kinds: Vec<Kind>,
}

#[derive(Deserialize)]
struct Kind {
    kind: String,
    label: String,
    color_token: String,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default)]
    path_markers: Vec<String>,
    #[serde(default)]
    prompt_markers: Vec<String>,
    #[serde(default)]
    text_markers: Vec<String>,
    #[serde(default)]
    min_messages: u64,
    #[serde(default)]
    min_text_chars: u64,
    #[serde(default)]
    min_duration_ms: u64,
}

pub fn default_path() -> PathBuf {
    // Installed binaries live in bin/. Development tests use the source asset.
    if let Ok(executable) = std::env::current_exe()
        && let Some(root) = executable.parent().and_then(Path::parent)
    {
        let path = root.join("assets/chat_kinds.json");
        if path.is_file() {
            return path;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/chat_kinds.json")
}

impl ChatKinds {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = super::read_limited(path).map_err(|error| {
            Error::Invalid(format!(
                "Cannot read chat-kind mapping {}: {error}",
                path.display()
            ))
        })?;
        let mapping: Self = serde_json::from_slice(&bytes).map_err(|error| {
            Error::Invalid(format!(
                "Invalid chat-kind mapping {}: {error}",
                path.display()
            ))
        })?;
        if mapping.schema != "soliton-chat-kinds/1"
            || mapping.kinds.is_empty()
            || mapping.kinds.len() > 64
            || !mapping
                .kinds
                .iter()
                .any(|kind| kind.kind == mapping.default_kind)
        {
            return Err(Error::Invalid("Unsupported chat-kind mapping".into()));
        }
        let mut ids = std::collections::BTreeSet::new();
        for kind in &mapping.kinds {
            if kind.kind.is_empty()
                || kind.label.is_empty()
                || !ids.insert(&kind.kind)
                || !matches!(
                    kind.color_token.as_str(),
                    "grey" | "purple" | "green" | "yellow" | "blue" | "magenta"
                )
                || kind
                    .providers
                    .iter()
                    .chain(&kind.path_markers)
                    .chain(&kind.prompt_markers)
                    .chain(&kind.text_markers)
                    .any(|marker| marker.is_empty())
            {
                return Err(Error::Invalid("Invalid chat-kind entry".into()));
            }
        }
        Ok(mapping)
    }

    pub fn palette(&self) -> Value {
        json!({"schema":self.schema,"default_kind":self.default_kind,"kinds":self.kinds.iter().map(|kind|json!({"kind":kind.kind,"label":kind.label,"color_token":kind.color_token})).collect::<Vec<_>>()})
    }

    pub fn apply(&self, record: &mut Value) {
        if record["kind"] != "chat" {
            return;
        }
        let facts = record.get("segments").and_then(Value::as_array).map(|segments| {
            let dialogue: Vec<_> = segments.iter().filter(|segment| {
                matches!(segment["speaker"].as_str(), Some("user" | "assistant"))
                    && segment["text"].as_str().is_some_and(|text| !text.trim().is_empty())
            }).collect();
            let text = dialogue.iter().filter_map(|segment| segment["text"].as_str()).collect::<Vec<_>>().join("\n");
            let prompt = dialogue.iter().find(|segment| segment["speaker"] == "user"
                && !segment["text"].as_str().unwrap_or("").trim_start().starts_with("# AGENTS.md instructions"))
                .and_then(|segment| segment["text"].as_str()).unwrap_or("");
            json!({"message_count":dialogue.len(),"text_chars":text.chars().count(),
                "first_prompt":prompt.to_lowercase(),"text_markers":self.kinds.iter().flat_map(|kind| &kind.text_markers).filter(|marker|text.to_lowercase().contains(&marker.to_lowercase())).collect::<Vec<_>>()})
        }).unwrap_or_else(|| record["_chat_facts"].clone());
        let messages = facts["message_count"].as_u64().unwrap_or(0);
        // Unattributed text belongs to Sources; a metadata-only session is no record.
        if messages == 0 {
            let has_text = record["segments"].as_array().is_some_and(|segments| {
                segments.iter().any(|segment| {
                    segment["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
                })
            });
            record["_history_relevant"] = json!(has_text);
            if has_text {
                record["kind"] = json!("source");
                return;
            }
        } else {
            record["_history_relevant"] = json!(true);
        }
        let provider = record["provider"].as_str().unwrap_or("");
        let paths = record["source_refs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|reference| reference["path"].as_str())
            .chain(record["session_path"].as_str())
            .chain(record["agent"]["path"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        let prompt = facts["first_prompt"].as_str().unwrap_or("");
        let duration = record["duration_ms"].as_u64();
        let mut selected = None;
        for kind in &self.kinds {
            if kind.kind == self.default_kind || messages == 0 {
                continue;
            }
            let provider_match = kind.providers.iter().any(|candidate| candidate == provider);
            let path_match = kind
                .path_markers
                .iter()
                .any(|marker| paths.contains(&marker.to_lowercase()));
            let prompt_match = kind
                .prompt_markers
                .iter()
                .any(|marker| prompt.contains(&marker.to_lowercase()));
            let provenance = !kind.providers.is_empty()
                || !kind.path_markers.is_empty()
                || !kind.prompt_markers.is_empty();
            if provenance && !(provider_match || path_match || prompt_match) {
                continue;
            }
            if messages < kind.min_messages
                || facts["text_chars"].as_u64().unwrap_or(0) < kind.min_text_chars
                || (kind.min_duration_ms > 0 && duration.is_none_or(|ms| ms < kind.min_duration_ms))
            {
                continue;
            }
            if !kind.text_markers.is_empty()
                && !kind.text_markers.iter().any(|marker| {
                    facts["text_markers"].as_array().is_some_and(|markers| {
                        markers
                            .iter()
                            .any(|found| found.as_str() == Some(marker.as_str()))
                    })
                })
            {
                continue;
            }
            let reason = if provider_match {
                "Recorded provider"
            } else if path_match {
                "Recorded session location"
            } else if prompt_match {
                "Explicit opening instruction"
            } else {
                "Measured conversation length and duration"
            };
            selected = Some((kind, reason, if provenance { 0.95 } else { 0.7 }));
            break;
        }
        let (kind, reason, certainty) = selected.unwrap_or_else(|| {
            let kind = self
                .kinds
                .iter()
                .find(|kind| kind.kind == self.default_kind)
                .expect("Validated default kind");
            if messages == 0 {
                (kind, "No visible conversation was recorded", 0.0)
            } else if duration.is_none() && messages >= 20 {
                (
                    kind,
                    "Ordinary fallback: session duration was not recorded",
                    0.25,
                )
            } else {
                (kind, "Ordinary fallback: no stronger kind evidence", 0.5)
            }
        });
        record["chat_kind"] = json!(kind.kind);
        record["chat_kind_label"] = json!(kind.label);
        record["chat_color_token"] = json!(kind.color_token);
        record["chat_kind_reason"] = json!(reason);
        record["chat_kind_certainty"] = json!(certainty);
        record["_chat_facts"] = facts;
    }
}

pub fn relevant(record: &Value) -> bool {
    matches!(
        record["kind"].as_str(),
        Some("chat" | "voice_note" | "run" | "source")
    ) && record["_history_relevant"] != false
}

/// Recover timing and console provenance from retained native event logs.
/// This reads snapshots only; source bytes and committed revisions are untouched.
pub fn recover_session(root: &Path, record: &mut Value, projected: Option<&Value>) -> Result<()> {
    if record["kind"] != "chat" || !matches!(record["provider"].as_str(), Some("codex" | "console"))
    {
        return Ok(());
    }
    if let Some(prior) = projected
        && prior["_chat_session_hash"] == record["_source_hash"]
        && prior["_chat_session_hash"].is_string()
    {
        record["duration_ms"] = prior["duration_ms"].clone();
        record["session_path"] = prior["session_path"].clone();
        record["_chat_session_hash"] = prior["_chat_session_hash"].clone();
        return Ok(());
    }
    if record["duration_ms"].is_number() {
        return Ok(());
    }
    let Some(id) = record["id"].as_str() else {
        return Ok(());
    };
    super::check_component_name(id)?;
    let Some(filename) = record["source_refs"][0]["path"]
        .as_str()
        .and_then(|path| Path::new(path).file_name())
    else {
        return Ok(());
    };
    let path = root
        .join("raw")
        .join(id)
        .join(record["_source_revision"].as_u64().unwrap_or(1).to_string())
        .join(filename);
    if !path.is_file() {
        return Ok(());
    }
    super::ensure_regular(&path)?;
    let reader = BufReader::new(std::fs::File::open(&path)?);
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = None;
    for line in reader.lines() {
        let line = line?;
        // Native JSONL places its timestamp in the header. Read the scalar,
        // rather than allocating every retained tool response again.
        if let Some((_, rest)) = line.split_once("\"timestamp\"")
            && let Some((_, rest)) = rest.split_once(':')
            && let Some(rest) = rest.trim_start().strip_prefix('"')
            && let Some((time, _)) = rest.split_once('"')
            && let Ok(time) = chrono::DateTime::parse_from_rfc3339(time)
        {
            let ms = time.timestamp_millis();
            first = Some(first.map_or(ms, |prior| prior.min(ms)));
            last = Some(last.map_or(ms, |prior| prior.max(ms)));
        }
        if line.contains("\"session_meta\"") {
            let meta: Value = serde_json::from_str(&line)?;
            if let Some(path) = meta["payload"]["cwd"].as_str() {
                record["session_path"] = json!(path);
            }
        }
    }
    record["duration_ms"] = json!(
        first
            .zip(last)
            .map(|(first, last)| last.saturating_sub(first))
    );
    record["_chat_session_hash"] = record["_source_hash"].clone();
    Ok(())
}
