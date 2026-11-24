//! Durable, local relevance preferences for Learning Center sources.
//!
//! Rules are Soliton-owned preferences. They never call a provider, mutate a
//! like/save state, or imply that a browser visit proves that a source was
//! viewed. The service can use `source-policy-evaluate` before lexical
//! analysis and pass `override_excluded` only for an explicit read-only review.
use crate::{Error, Result};
use chrono::Utc;
use serde_json::{Value, json};
use soliton_kernel::ContentHash;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_RULES: usize = 4096;
const MAX_RECORD_BYTES: u64 = 256 * 1024;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
fn safe(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
    {
        return Err(invalid(format!("Invalid {label}")));
    }
    Ok(())
}
fn reject_root(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            "Source preference root must be an absolute path without . or ..",
        ));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(invalid("Source preference root contains a symlink"));
        }
    }
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    reject_root(path)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err(invalid("Source preference path is not a directory"));
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Preference record has no parent"))?;
    private_dir(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(invalid("Source preference target is a symlink"));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id(),
        stamp
    ));
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
fn hash(value: &Value) -> String {
    ContentHash::digest(&serde_json::to_vec(value).unwrap_or_default()).to_string()
}
fn canonical(value: &Value) -> String {
    value.to_string()
}
fn domain(url: &str) -> String {
    let value = url.trim().to_lowercase();
    let host = value
        .split_once("://")
        .map_or(value.as_str(), |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or("");
    host.split('@')
        .next_back()
        .unwrap_or(host)
        .split(':')
        .next()
        .unwrap_or(host)
        .trim_start_matches("www.")
        .to_owned()
}
/// Single YouTube video identity from a public watch, short or embed URL.
pub fn youtube_video_id(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
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
    let path = path.split('#').next().unwrap_or("");
    let candidate = if host == "youtu.be" {
        path.trim_matches('/').split('/').next().unwrap_or("")
    } else if matches!(
        host.as_str(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com"
    ) {
        match path.trim_matches('/').split_once('/') {
            Some(("shorts" | "embed" | "live", id)) => id.split('/').next().unwrap_or(""),
            _ => query
                .split('#')
                .next()
                .unwrap_or("")
                .split('&')
                .find_map(|part| {
                    let (key, value) = part.split_once('=')?;
                    (key == "v").then_some(value)
                })?,
        }
    } else {
        return None;
    };
    (candidate.len() == 11
        && candidate
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
    .then(|| candidate.to_owned())
}
fn fold(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
/// Words in a source's own title, tags, channel or description that mark it
/// as learning material.  Any one of them keeps the source out of the
/// local categories below, whatever the provider files it under.
const LEARNING_WORDS: &[&str] = &[
    "lecture",
    "lectures",
    "course",
    "courses",
    "tutorial",
    "tutorials",
    "educator",
    "education",
    "educational",
    "engineer",
    "engineering",
    "engineers",
    "programming",
    "programmer",
    "software",
    "developer",
    "developers",
    "coding",
    "ai",
    "science",
    "scientist",
    "research",
    "mathematics",
    "maths",
    "physics",
    "computer",
    "algorithm",
    "algorithms",
    "typescript",
    "python",
    "rust",
    "linux",
    "pytorch",
    "neural",
    "learning",
    "lesson",
    "lessons",
    "university",
    "professor",
];
const MUSIC_PHRASES: &[&str] = &[
    "official video",
    "official music video",
    "music video",
    "lyric video",
    "lyrics",
    "official audio",
];
const COOKING_WORDS: &[&str] = &[
    "recipe",
    "recipes",
    "cooking",
    "cookery",
    "chef",
    "chefs",
    "masterchef",
    "kitchen",
    "baking",
    "cuisine",
];

/// The local mapping from a video's own metadata (as the downloader or the
/// public oEmbed page returns it) to his category names: music, cooking and
/// entertainment.  The provider's category is one hint among the source's
/// own words and never decides alone: a learning word anywhere in the
/// source keeps it uncategorised, so a lecture YouTube files under
/// Entertainment is still admitted.  Returns the facts with their evidence.
pub fn learn_categories(meta: &Value) -> Value {
    let text = |key: &str| meta[key].as_str().unwrap_or("").to_owned();
    let title = text("title");
    let channel = [text("channel"), text("uploader"), text("author_name")]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .unwrap_or_default();
    let tags = meta["tags"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .take(64)
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let description = text("description").chars().take(1500).collect::<String>();
    let provider = meta["categories"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(fold)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let words = |value: &str| {
        value
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let own = words(&format!("{title} {tags} {channel}"));
    let learning = own
        .union(&words(&description))
        .filter(|word| LEARNING_WORDS.contains(&word.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let via = meta["via"].as_str().unwrap_or("source metadata");
    let mut categories = Vec::new();
    if learning.is_empty() {
        let low = format!("{title} {tags}").to_lowercase();
        let mut push = |category: &str, why: String| {
            categories.push(json!({"category":category,"evidence":format!("learned from {via}: {why}; no learning words")}));
        };
        if provider.iter().any(|c| c == "music") {
            push("music", "YouTube category music".into());
        } else if let Some(phrase) = MUSIC_PHRASES.iter().find(|phrase| low.contains(*phrase)) {
            push("music", format!("\"{phrase}\" in the title or tags"));
        }
        if let Some(word) = COOKING_WORDS.iter().find(|word| own.contains(**word)) {
            push(
                "cooking",
                format!("\"{word}\" in the title, tags or channel"),
            );
        }
        if let Some(category) = provider
            .iter()
            .find(|c| matches!(c.as_str(), "entertainment" | "comedy"))
        {
            push("entertainment", format!("YouTube category {category}"));
        }
    }
    json!({"title":title,"channel":channel,"provider_categories":provider,"learning_words":learning,
        "categories":categories,"via":via})
}

/// What is known about a source before a rule is applied: its channel and
/// categories, each from caller-verified metadata or the local mapping.
struct Facts {
    channel: String,
    categories: Vec<Value>,
}
fn matcher_matches(rule: &Value, source_id: &str, url: &str, title: &str, facts: &Facts) -> bool {
    let pattern = rule["pattern"].as_str().unwrap_or("").to_lowercase();
    match rule["matcher"].as_str().unwrap_or("source") {
        "source" => pattern == source_id.to_lowercase(),
        "url" => rule["pattern"].as_str().unwrap_or("") == url,
        "domain" => domain(url) == pattern.trim_start_matches("www."),
        "topic" => title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == pattern),
        "channel" => !facts.channel.is_empty() && fold(&facts.channel) == fold(&pattern),
        "category" => facts
            .categories
            .iter()
            .any(|known| known["category"].as_str() == Some(fold(&pattern).as_str())),
        _ => false,
    }
}

pub struct SourcePreferences {
    root: PathBuf,
    rules: BTreeMap<String, Value>,
    /// Locally configured source metadata: a video's channel, or the
    /// category of a video, channel or domain, each with its evidence.
    metadata: BTreeMap<String, Value>,
}

fn load_dir(dir: &Path, label: &str) -> Result<BTreeMap<String, Value>> {
    let mut records = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
            return Err(invalid(format!(
                "{label} archive contains an unexpected entry"
            )));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = name
            .strip_suffix(".json")
            .ok_or_else(|| invalid(format!("{label} archive contains an unexpected file")))?;
        safe(id, label)?;
        let file = File::open(entry.path())?;
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid(format!("{label} record exceeds its size limit")));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("Corrupt {label}: {error}")))?;
        if value["id"].as_str() != Some(id) || value["schema"] != 1 {
            return Err(invalid(format!("Corrupt {label} identity")));
        }
        let mut unsigned = value.clone();
        let revision = unsigned["revision"]
            .as_str()
            .ok_or_else(|| invalid(format!("Corrupt {label} revision")))?
            .to_owned();
        if let Some(object) = unsigned.as_object_mut() {
            object.remove("revision");
        }
        if hash(&unsigned) != revision {
            return Err(invalid(format!("Corrupt {label} revision hash")));
        }
        records.insert(id.to_owned(), value);
        if records.len() > MAX_RULES {
            return Err(invalid(format!("Too many {label} records")));
        }
    }
    Ok(records)
}

impl SourcePreferences {
    pub fn open(root: &Path) -> Result<Self> {
        private_dir(root)?;
        private_dir(&root.join("rules"))?;
        private_dir(&root.join("metadata"))?;
        Ok(Self {
            root: root.to_owned(),
            rules: load_dir(&root.join("rules"), "source preference")?,
            metadata: load_dir(&root.join("metadata"), "source metadata")?,
        })
    }
    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "source-policy-set" => self.set(args),
            "source-policy-remove" => self.remove(args),
            "source-policy-list" => self.list(args),
            "source-metadata-set" => self.metadata_set(args),
            "source-metadata-remove" => self.metadata_remove(args),
            "source-metadata-list" => Ok(
                json!({"items":self.metadata.values().cloned().collect::<Vec<_>>(),"total":self.metadata.len()}),
            ),
            "source-policy-evaluate" => self.evaluate(args),
            "source-policy-evaluate-batch" => {
                let items = args["items"]
                    .as_array()
                    .ok_or_else(|| invalid("items must be an array"))?;
                if items.len() > 256 {
                    return Err(invalid("At most 256 source policies per batch"));
                }
                let values = items
                    .iter()
                    .map(|item| self.evaluate(item))
                    .collect::<Result<Vec<_>>>()?;
                Ok(json!({"items":values}))
            }
            _ => Err(invalid(format!(
                "Unknown source preference command: {command}"
            ))),
        }
    }
    fn set(&mut self, args: &Value) -> Result<Value> {
        let scope = args["scope"].as_str().unwrap_or("global");
        if !matches!(scope, "global" | "goal") {
            return Err(invalid("Preference scope must be global or goal"));
        }
        let goal_id = args["goal_id"].as_str().unwrap_or("");
        if scope == "goal" {
            safe(goal_id, "goal id")?;
            if goal_id.is_empty() {
                return Err(invalid("Goal preference requires goal_id"));
            }
        }
        let kind = args["kind"].as_str().unwrap_or("");
        if !matches!(kind, "save" | "exclude") {
            return Err(invalid("Preference kind must be save or exclude"));
        }
        let matcher = args
            .get("matcher")
            .or_else(|| args.get("pattern_type"))
            .and_then(Value::as_str)
            .unwrap_or("source");
        if !matches!(
            matcher,
            "source" | "url" | "domain" | "topic" | "channel" | "category"
        ) {
            return Err(invalid(
                "Preference matcher must be source, URL, domain, topic, channel or category",
            ));
        }
        let pattern = args["pattern"].as_str().unwrap_or("").trim();
        if pattern.is_empty() || pattern.len() > 512 {
            return Err(invalid("Preference pattern is required and bounded"));
        }
        let identity = format!(
            "{scope}\0{goal_id}\0{kind}\0{matcher}\0{}",
            if matcher == "url" {
                pattern.to_owned()
            } else {
                pattern.to_lowercase()
            }
        );
        let id = format!(
            "policy-{}",
            ContentHash::digest(identity.as_bytes()).short_hex(24)
        );
        let prior = self.rules.get(&id);
        if prior.is_none() && self.rules.len() >= MAX_RULES {
            return Err(invalid("Too many source preference rules"));
        }
        if let Some(expected) = args["expected_revision"].as_str() {
            if prior.map(|value| value["revision"].as_str().unwrap_or("")) != Some(expected) {
                return Err(invalid("Source preference changed. Reload before editing."));
            }
        } else if prior.is_some() {
            return Err(invalid(
                "Existing source preference requires expected_revision",
            ));
        }
        let created_at = prior
            .and_then(|value| value["created_at"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut rule = json!({"schema":1,"id":id,"scope":scope,"goal_id":if scope=="goal"{json!(goal_id)}else{Value::Null},"kind":kind,"matcher":matcher,"pattern":pattern,"enabled":args["enabled"].as_bool().unwrap_or(true),"created_at":if created_at.is_empty(){json!(now())}else{json!(created_at)},"updated_at":now()});
        rule["revision"] = json!(hash(&rule));
        atomic_json(&self.root.join("rules").join(format!("{id}.json")), &rule)?;
        self.rules.insert(id, rule.clone());
        Ok(rule)
    }
    fn remove(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Preference id is required"))?;
        safe(id, "source preference id")?;
        let rule = self
            .rules
            .get(id)
            .ok_or_else(|| invalid("Source preference not found"))?;
        if args["expected_revision"].as_str() != rule["revision"].as_str() {
            return Err(invalid(
                "Source preference changed. Reload before removing.",
            ));
        }
        fs::remove_file(self.root.join("rules").join(format!("{id}.json")))?;
        self.rules.remove(id);
        #[cfg(unix)]
        {
            File::open(self.root.join("rules"))?.sync_all()?;
        }
        Ok(json!({"removed":id}))
    }
    fn list(&self, args: &Value) -> Result<Value> {
        let query = args["q"].as_str().unwrap_or("").to_lowercase();
        let scope = args["scope"].as_str().unwrap_or("");
        let goal_id = args["goal_id"].as_str().unwrap_or("");
        let mut items = self
            .rules
            .values()
            .filter(|rule| {
                (scope.is_empty() || rule["scope"] == scope)
                    && (goal_id.is_empty() || rule["goal_id"] == goal_id)
                    && (query.is_empty() || canonical(rule).to_lowercase().contains(&query))
            })
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a["updated_at"].as_str().cmp(&b["updated_at"].as_str()));
        let total = items.len();
        let offset = args["offset"].as_u64().unwrap_or(0).min(total as u64) as usize;
        let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
        Ok(
            json!({"items":items.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":total,"offset":offset,"limit":limit}),
        )
    }
    fn metadata_set(&mut self, args: &Value) -> Result<Value> {
        let locator = args["locator"].as_str().unwrap_or("");
        let raw = args["pattern"].as_str().unwrap_or("").trim();
        let pattern = match locator {
            "video" => youtube_video_id(raw)
                .or_else(|| youtube_video_id(&format!("https://youtu.be/{raw}")))
                .ok_or_else(|| invalid("A video locator is a YouTube video id or URL"))?,
            "channel" | "domain" if !raw.is_empty() && raw.len() <= 512 => fold(raw),
            _ => {
                return Err(invalid(
                    "Locator must be video, channel or domain with a pattern",
                ));
            }
        };
        let category = fold(args["category"].as_str().unwrap_or(""));
        let channel = args["channel"].as_str().unwrap_or("").trim();
        if category.len() > 64 || channel.len() > 512 || (category.is_empty() && channel.is_empty())
        {
            return Err(invalid(
                "Give a category, or for a video its channel, within bounds",
            ));
        }
        if !channel.is_empty() && locator != "video" {
            return Err(invalid("Only a video locator records its channel"));
        }
        let evidence = args["evidence"].as_str().unwrap_or("").trim();
        if evidence.is_empty() || evidence.len() > 2000 {
            return Err(invalid(
                "Record where this metadata was verified (at most 2000 bytes)",
            ));
        }
        let id = format!(
            "meta-{}",
            ContentHash::digest(format!("{locator}\0{pattern}").as_bytes()).short_hex(24)
        );
        let prior = self.metadata.get(&id);
        if prior.is_none() && self.metadata.len() >= MAX_RULES {
            return Err(invalid("Too many source metadata records"));
        }
        if let Some(expected) = args["expected_revision"].as_str() {
            if prior.map(|value| value["revision"].as_str().unwrap_or("")) != Some(expected) {
                return Err(invalid("Source metadata changed. Reload before editing."));
            }
        } else if prior.is_some() {
            return Err(invalid(
                "Existing source metadata requires expected_revision",
            ));
        }
        let created_at = prior
            .and_then(|value| value["created_at"].as_str())
            .map_or_else(now, str::to_owned);
        let mut record = json!({"schema":1,"id":id,"locator":locator,"pattern":pattern,
            "category":if category.is_empty(){Value::Null}else{json!(category)},
            "channel":if channel.is_empty(){Value::Null}else{json!(channel)},
            "evidence":evidence,"created_at":created_at,"updated_at":now()});
        record["revision"] = json!(hash(&record));
        atomic_json(
            &self.root.join("metadata").join(format!("{id}.json")),
            &record,
        )?;
        self.metadata.insert(id, record.clone());
        Ok(record)
    }
    fn metadata_remove(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Metadata id is required"))?;
        safe(id, "source metadata id")?;
        let record = self
            .metadata
            .get(id)
            .ok_or_else(|| invalid("Source metadata not found"))?;
        if args["expected_revision"].as_str() != record["revision"].as_str() {
            return Err(invalid("Source metadata changed. Reload before removing."));
        }
        fs::remove_file(self.root.join("metadata").join(format!("{id}.json")))?;
        self.metadata.remove(id);
        Ok(json!({"removed":id}))
    }
    /// Whether an enabled rule could match on what is learned about a
    /// source (its category, channel or title words); without one nothing
    /// is fetched to learn it.
    pub fn needs_facts(&self) -> bool {
        self.rules.values().any(|rule| {
            rule["enabled"] != false
                && matches!(
                    rule["matcher"].as_str(),
                    Some("category" | "channel" | "topic")
                )
        })
    }
    fn mapped(&self, locator: &str, pattern: &str) -> Option<&Value> {
        self.metadata
            .values()
            .find(|record| record["locator"] == locator && record["pattern"] == pattern)
    }
    /// Channel and categories from the request's verified metadata, the
    /// local mapping and what the engine learned from the source itself
    /// (`learn_categories`). Nothing is inferred from viewing history, and
    /// YouTube's own category never decides alone: it also labels wanted
    /// learning videos.
    fn facts(&self, args: &Value, url: &str) -> Facts {
        let mut channel = args["channel"].as_str().unwrap_or("").trim().to_owned();
        let mut categories = Vec::new();
        let mut push = |category: &str, via: &str, record: Option<&Value>| {
            let category = fold(category);
            if !category.is_empty() && category.len() <= 64 {
                categories.push(json!({"category":category,"via":via,
                    "metadata_id":record.map_or(Value::Null,|r|r["id"].clone()),
                    "evidence":record.map_or(Value::Null,|r|r["evidence"].clone())}));
            }
        };
        if let Some(category) = args["category"].as_str() {
            push(category, "request", None);
        }
        let video = youtube_video_id(url);
        if let Some(record) = video.as_deref().and_then(|id| self.mapped("video", id)) {
            if channel.is_empty() {
                channel = record["channel"].as_str().unwrap_or("").to_owned();
            }
            if let Some(category) = record["category"].as_str() {
                push(category, "video", Some(record));
            }
        }
        // What the engine learned from the source itself; his own mapping of
        // the video, when it names a category, wins over it.
        let learned = &args["learned"];
        if channel.is_empty() {
            channel = learned["channel"].as_str().unwrap_or("").trim().to_owned();
        }
        let mapped_video = video
            .as_deref()
            .and_then(|id| self.mapped("video", id))
            .is_some_and(|record| record["category"].is_string());
        let mut learned_categories = Vec::new();
        if !mapped_video && let Some(items) = learned["categories"].as_array() {
            for item in items.iter().take(8) {
                let category = fold(item["category"].as_str().unwrap_or(""));
                if !category.is_empty() && category.len() <= 64 {
                    learned_categories.push(
                        json!({"category":category,"via":"learned","metadata_id":Value::Null,
                        "evidence":item["evidence"].clone()}),
                    );
                }
            }
        }
        if !channel.is_empty()
            && let Some(record) = self.mapped("channel", &fold(&channel))
            && let Some(category) = record["category"].as_str()
        {
            push(category, "channel", Some(record));
        }
        let host = domain(url);
        if !host.is_empty()
            && let Some(record) = self.mapped("domain", &host)
            && let Some(category) = record["category"].as_str()
        {
            push(category, "domain", Some(record));
        }
        categories.extend(learned_categories);
        Facts {
            channel,
            categories,
        }
    }
    fn evaluate(&self, args: &Value) -> Result<Value> {
        let source_id = args["source_id"].as_str().unwrap_or("");
        if source_id.is_empty() {
            return Err(invalid("source_id is required"));
        }
        let url = args["url"].as_str().unwrap_or("");
        let title = args["title"].as_str().unwrap_or("");
        let goal_id = args["goal_id"].as_str().unwrap_or("");
        let facts = self.facts(args, url);
        let matched = self
            .rules
            .values()
            .filter(|rule| {
                rule["enabled"] != false
                    && matcher_matches(rule, source_id, url, title, &facts)
                    && (rule["scope"] == "global"
                        || (rule["scope"] == "goal" && rule["goal_id"] == goal_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        let explicit_save = matched.iter().any(|rule| {
            rule["kind"] == "save" && rule["scope"] == "goal" && rule["goal_id"] == goal_id
        });
        let excluded = matched.iter().any(|rule| rule["kind"] == "exclude");
        let overridden = args["override_excluded"].as_bool().unwrap_or(false) && excluded;
        let include = !excluded || explicit_save || overridden;
        let reason = if overridden {
            "Explicit read-only override of a local exclusion"
        } else if explicit_save {
            "Goal save preference overrides a broader exclusion"
        } else if excluded {
            "Excluded by a local source preference"
        } else {
            "No matching exclusion; lexical analysis may proceed"
        };
        Ok(
            json!({"include":include,"excluded":excluded,"overridden":overridden,"reason":reason,"matched":matched,"source_id":source_id,"goal_id":if goal_id.is_empty(){Value::Null}else{json!(goal_id)},
                "video_id":youtube_video_id(url),"channel":if facts.channel.is_empty(){Value::Null}else{json!(facts.channel)},
                "category_state":if facts.categories.is_empty(){"unknown"}else{"known"},"categories":facts.categories}),
        )
    }
}
