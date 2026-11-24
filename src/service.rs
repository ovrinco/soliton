use crate::{Error, Result, time};
use chrono::{Datelike, Duration, Utc};
use serde_json::{Value, json};
use soliton_kernel::{
    details::{Details, SeriesHistory},
    ledger::finance::{
        CATEGORIES, DocumentKind, LineFilter, ReceiptLine, StatementMapping, category_for,
        intake_dir, read_statement_header,
    },
    ledger::habits::{Bundle, CheckIn, Direction, Habit, HabitSpec},
    ledger::kitchen::{
        DishSpec, Lot, LotSource, LotSpec, PlanOutcome, ProfileEntry, check_picture,
    },
    ledger::{LEDGER_FILE, SCHEMA_VERSION},
    *,
};
use std::{
    collections::{BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[path = "intelligence.rs"]
mod intelligence;
#[path = "privacy.rs"]
pub mod privacy;
#[path = "tool_launch.rs"]
pub mod tool_launch;
#[path = "zeno_settings.rs"]
pub mod zeno_settings;

type HistoryJobResult = (Option<crate::history::HistoryStore>, Result<Value>);
const HISTORY_RECEIPT_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Recovery's id prefix for a record past its Trash window.
const EXPIRED_RECORD: &str = "expired-record-";

/// The word the Trash shows for each kind it holds.
fn trash_word(kind: &str) -> &'static str {
    match kind {
        "goal" => "Goal",
        "category" => "Category",
        "kitchen-profile" => "Profile",
        "collection-source" => "Source",
        _ => "Task",
    }
}

/// A record's module is the privacy scope that hides it.
fn hides_record(privacy: &privacy::Privacy, kind: &str, id: &str) -> bool {
    let module = if kind == "kitchen-profile" {
        "kitchen"
    } else {
        "collection"
    };
    privacy.on && (privacy.modules.contains(module) || privacy.ids.contains(id))
}

/// A cook-profile entry's values as the Trash keeps them (its row id aside).
fn profile_bytes(entry: &ProfileEntry) -> Vec<u8> {
    serde_json::to_vec(&json!({"subject":entry.subject,"quantity":entry.quantity,
        "unit":entry.unit,"value":entry.value}))
    .unwrap_or_default()
}

/// The preferences key that reports the ledger's own module marks (read only).
const LEDGER_MARKS_KEY: &str = "ledgerPrivateModules";

pub struct Service {
    tool_launcher: tool_launch::ToolLauncher,
    pub repo: GovernedRepository,
    pub snapshot: Snapshot,
    trash: soliton_kernel::trash::TrashIndex,
    records: soliton_kernel::trash::RecordTrash,
    _lock: File,
    pub recovered: u32,
    history: Option<crate::history::HistoryStore>,
    history_job: Option<(String, std::sync::mpsc::Receiver<HistoryJobResult>)>,
    history_job_receipts: std::collections::BTreeMap<String, Value>,
    history_job_command: String,
    history_receipt_bytes: std::collections::BTreeMap<String, usize>,
    history_receipt_order: VecDeque<String>,
    recovery: Option<crate::recovery::RecoveryStore>,
    inbox: Option<crate::inbox::InboxStore>,
    modules: Option<crate::modules::ModulesStore>,
    inference: crate::inference::InferenceStatus,
    briefing: Option<crate::briefing::BriefingStore>,
    /// The programs the briefing worker uses; tests set a fixture client.
    briefing_tools: crate::briefing::StoreTools,
    collection: Option<crate::collection::CollectionStore>,
    source_preferences: Option<crate::source_preferences::SourcePreferences>,
    collection_import: Option<(String, String)>,
    collection_import_error: Option<(std::time::Instant, String)>,
    background_due: std::time::Instant,
    /// Due time of the next automatic history capture; `None` while the
    /// workspace setting `automaticCapture` is off (the default).
    capture_due: Option<std::time::Instant>,
    /// The workspace setting `localModel`: off by default, and while off no
    /// path of Soliton calls or starts the local model gateway.
    local_model: bool,
    /// The stratum's serve script the kitchen draft reads its port from; tests set a stand-in.
    decision_serve: PathBuf,
    journal_signature: Option<(u64, std::time::SystemTime, u64, u64)>,
    /// The workspace's mark is older than this engine's format: written with the
    /// first change to the workspace, never on open or idle.
    format_mark_due: bool,
    last_freshness_check: std::time::Instant,
    /// The data plane ledger, present only when the workspace has `ledger.sqlite`.
    /// Its presence is the one switch for the planner's ledger writes.
    ledger: Option<Ledger>,
    /// The tracker store, read in-process only while asked or while a sheet view is shown.
    tracker: crate::tracker::Tracker,
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self._lock.unlock();
    }
}
/// An already registered bootstrap changes neither capture nor archive state.
/// Keep this one no-op on the small registry; real history operations still open
/// and recover the archive normally. Other payloads retain the full bootstrap.
fn existing_capture_bootstrap(root: &Path, args: &Value) -> Option<Value> {
    if !args.as_object()?.is_empty() || std::env::var_os("HOME").is_none() {
        return None;
    }
    let path = root.join("capture-sources.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 2_000_000 {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(2_000_001)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 2_000_000 {
        return None;
    }
    let registry: Value = serde_json::from_slice(&bytes).ok()?;
    if registry["schema"] != "soliton-history-capture/1" {
        return None;
    }
    let sources = registry["sources"].as_array()?;
    let providers = ["codex", "antigravity", "voxtype"];
    if !providers
        .iter()
        .all(|provider| sources.iter().any(|source| source["provider"] == *provider))
    {
        return None;
    }
    let skipped: Vec<Value> = providers
        .iter()
        .map(|provider| json!({"provider":provider,"reason":"Provider already has a registry entry"}))
        .collect();
    Some(json!({"added":[],"skipped":skipped,"sources":sources}))
}

fn duration(v: &Value) -> Result<u32> {
    let n = v["duration_minutes"]
        .as_u64()
        .ok_or_else(|| invalid("Duration must be a whole number of minutes"))?;
    if !(1..=10080).contains(&n) {
        return Err(invalid("Duration must be 1–10080 minutes"));
    }
    Ok(n as u32)
}
fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}

fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn env_disabled(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| value == "0")
}

/// `machine-log`: whole local days `first` to `last` (`YYYY-MM-DD`, both
/// today when absent) in `zone` (else the machine's zone), each line with its
/// folded reading in `text`. `SOLITON_MACHINE_LOG_RECORD` names a recorded
/// source folder in place of the machine's own records.
fn machine_log_command(args: &Value) -> Result<Value> {
    use crate::machine_log::{Range, Source, collect};
    let zone_name = args["zone"].as_str().map(String::from).or_else(|| {
        fs::read_link("/etc/localtime").ok().and_then(|p| {
            p.to_str()
                .and_then(|p| p.split_once("zoneinfo/"))
                .map(|(_, z)| z.to_string())
        })
    });
    let zone: chrono_tz::Tz = zone_name
        .as_deref()
        .unwrap_or("UTC")
        .parse()
        .map_err(|_| invalid("Unknown time zone"))?;
    let today = Utc::now().with_timezone(&zone).date_naive();
    let day = |key: &str| -> Result<chrono::NaiveDate> {
        match args[key].as_str() {
            Some(text) => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| invalid("Days are written YYYY-MM-DD")),
            None => Ok(today),
        }
    };
    let (first, last) = (day("first")?, day("last")?);
    if last < first || (last - first).num_days() > 31 {
        return Err(invalid(
            "Choose up to a month of days, the first before the last",
        ));
    }
    let source = std::env::var_os("SOLITON_MACHINE_LOG_RECORD")
        .filter(|v| !v.is_empty())
        .map_or_else(Source::system, |dir| Source::Recorded(PathBuf::from(dir)));
    let log = collect(Range::days(zone, first, last), &source)
        .map_err(|error| invalid(error.to_string()))?;
    let texts: Vec<Vec<String>> = log
        .days
        .iter()
        .map(|d| d.lines.iter().map(|l| l.text()).collect())
        .collect();
    let mut value = serde_json::to_value(&log)?;
    if let Some(days) = value["days"].as_array_mut() {
        for (day, texts) in days.iter_mut().zip(texts) {
            if let Some(lines) = day["lines"].as_array_mut() {
                for (line, text) in lines.iter_mut().zip(texts) {
                    line["text"] = json!(text);
                }
            }
        }
    }
    Ok(value)
}

/// Wait between turning automatic capture on (or opening with it on) and the
/// first sync. `SOLITON_CAPTURE_DELAY_SECS` is a test setting.
fn capture_delay() -> std::time::Duration {
    std::time::Duration::from_secs(
        std::env::var("SOLITON_CAPTURE_DELAY_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(60),
    )
}

const LOCAL_MODEL_OFF: &str = "Local model is off. Turn it on in Settings to use it.";
fn value_weight(value: &Value) -> usize {
    match value {
        Value::Null => 8,
        Value::Bool(_) => 8,
        Value::Number(_) => 24,
        Value::String(text) => 24usize.saturating_add(text.len()),
        Value::Array(items) => items
            .iter()
            .fold(24, |total, item| total.saturating_add(value_weight(item))),
        Value::Object(items) => items.iter().fold(32, |total, (key, item)| {
            total
                .saturating_add(key.len())
                .saturating_add(value_weight(item))
        }),
    }
}
fn retain_history_receipt(
    receipts: &mut std::collections::BTreeMap<String, Value>,
    receipt_bytes: &mut std::collections::BTreeMap<String, usize>,
    receipt_order: &mut VecDeque<String>,
    id: String,
    value: Value,
) {
    let weight = value_weight(&value);
    if receipts.remove(&id).is_some() {
        receipt_bytes.remove(&id);
        receipt_order.retain(|key| key != &id);
    }
    receipts.insert(id.clone(), value);
    receipt_bytes.insert(id.clone(), weight);
    receipt_order.push_back(id.clone());
    let mut total = receipt_bytes.values().copied().sum::<usize>();
    while (receipts.len() > 32 || total > HISTORY_RECEIPT_MAX_BYTES) && receipts.len() > 1 {
        let Some(oldest) = receipt_order.pop_front() else {
            break;
        };
        if oldest == id {
            receipt_order.push_back(oldest);
            break;
        }
        receipts.remove(&oldest);
        total = total.saturating_sub(receipt_bytes.remove(&oldest).unwrap_or(0));
    }
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}
fn optional_id(v: &Value, key: &str) -> Result<Option<ObjectId>> {
    if s(v, key).is_empty() {
        Ok(None)
    } else {
        Ok(Some(s(v, key).parse()?))
    }
}
fn optional_date(v: &Value, key: &str) -> Result<Option<CivilDate>> {
    if s(v, key).is_empty() {
        Ok(None)
    } else {
        Ok(Some(s(v, key).parse()?))
    }
}
fn meta(kind: &str, title: &str) -> Result<ObjectMeta> {
    let now = Utc::now().timestamp();
    Ok(ObjectMeta {
        id: ObjectId::mint(kind)?,
        title: title.into(),
        revision: 1,
        data_class: DataClass::PersonalSensitive,
        details: Details {
            created_at: now,
            updated_at: now,
            ..Default::default()
        },
    })
}
fn with_hidden(args: &Value, ids: &std::collections::BTreeSet<String>) -> Value {
    let mut args = if args.is_object() {
        args.clone()
    } else {
        json!({})
    };
    args["hidden"] = json!(ids.iter().collect::<Vec<_>>());
    args
}

fn workspace_id() -> ObjectId {
    ObjectId::derive("document", b"soliton-desktop-workspace-v1").unwrap()
}

/// The workspace format this engine reads and writes. A workspace records the
/// format of the newest engine that wrote it in `journal/format.json`; one without
/// the mark is format 0 (every workspace before 0.5.24), which this engine reads.
pub const WORKSPACE_FORMAT: u64 = 1;
const FORMAT_MARK: &str = "journal/format.json";

/// The format a workspace records, read without writing; 0 when it has no mark.
pub fn workspace_format(root: &Path) -> Result<u64> {
    let path = root.join(FORMAT_MARK);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_file() => return Err(invalid("Workspace format mark is not a file")),
        Ok(_) => {}
    }
    let mark: Value = serde_json::from_slice(&fs::read(&path)?)
        .map_err(|_| invalid("Workspace format mark is unreadable; nothing was changed"))?;
    mark["format"]
        .as_u64()
        .ok_or_else(|| invalid("Workspace format mark is unreadable; nothing was changed"))
}

/// The release record beside the running build: SOLITON_RESOURCE_ROOT, else
/// the folder above this executable's bin/. Reads only; never writes.
pub fn release_record() -> Value {
    let root = std::env::var_os("SOLITON_RESOURCE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()?
                .parent()
                .map(Path::to_path_buf)
        });
    root.and_then(|r| fs::read(r.join("RELEASE.json")).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Which build runs and whether it is the build its record names on disk.
/// Hashed once per process: the executable does not change under a running engine.
pub fn release_identity() -> Value {
    static IDENTITY: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            use sha2::{Digest, Sha256};
            let record = release_record();
            let version = env!("CARGO_PKG_VERSION");
            let exe = std::env::current_exe().ok();
            let hash = exe
                .as_ref()
                .and_then(|p| File::open(p).ok())
                .and_then(|mut f| {
                    let mut hasher = Sha256::new();
                    std::io::copy(&mut f, &mut hasher).ok()?;
                    Some(format!("{:x}", hasher.finalize()))
                });
            let name = record["release"].as_str().unwrap_or("").to_string();
            // An install record spells its version as the head of the release name.
            let recorded = record["version"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| name.split('-').next().unwrap_or("").to_string());
            let built_on = record["built_on"].as_str().map(str::to_string).or_else(|| {
                let modified = fs::metadata(exe.as_ref()?).ok()?.modified().ok()?;
                Some(
                    chrono::DateTime::<Utc>::from(modified)
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string(),
                )
            });
            let matches_disk = hash.is_some()
                && record["binary_sha256"].as_str() == hash.as_deref()
                && recorded == version;
            json!({"name":name,"version":version,"built_on":built_on,
            "source_commit":record["source_commit"],"matches_disk":matches_disk})
        })
        .clone()
}

impl Service {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_tool_launcher(root, tool_launch::ToolLauncher::discover())
    }
    pub fn open_with_tool_launcher(
        root: &Path,
        tool_launcher: tool_launch::ToolLauncher,
    ) -> Result<Self> {
        // A workspace a newer engine wrote is refused before anything here writes.
        let format = workspace_format(root)?;
        if format > WORKSPACE_FORMAT {
            return Err(invalid(format!(
                "This workspace was written by a newer Soliton (format {format}); this build reads format {WORKSPACE_FORMAT} and changed nothing"
            )));
        }
        // Check path confinement/symlinks before opening the OS lock file.
        let repo = GovernedRepository::open(root)?;
        let lock_path = repo.root().join(".desktop.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(invalid("Workspace lock is a symlink"));
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock().map_err(|_|invalid("This workspace is already open in another Soliton process. Close that instance first."))?;
        let trash = soliton_kernel::trash::TrashIndex::open(repo.root())?;
        let records = soliton_kernel::trash::RecordTrash::open(repo.root())?;
        let recovered = repo.recover()?.committed;
        let snapshot = repo.rebuild()?;
        // A broken ledger never stops the planner: it is reported and left unused.
        let ledger = Ledger::open_existing(repo.root()).unwrap_or_else(|error| {
            eprintln!("Ledger unavailable: {error}");
            None
        });
        let mut service = Self {
            tool_launcher,
            repo,
            snapshot,
            trash,
            records,
            _lock: lock,
            recovered,
            history: None,
            history_job: None,
            history_job_receipts: Default::default(),
            history_job_command: String::new(),
            history_receipt_bytes: Default::default(),
            history_receipt_order: Default::default(),
            recovery: None,
            inbox: None,
            modules: None,
            inference: crate::inference::InferenceStatus::default(),
            briefing: None,
            briefing_tools: Default::default(),
            collection: None,
            source_preferences: None,
            collection_import: None,
            collection_import_error: None,
            background_due: std::time::Instant::now(),
            // Set from the workspace preferences on every refresh.
            capture_due: None,
            local_model: false,
            decision_serve: crate::hearth_draft::DraftSettings::serve_script(),
            journal_signature: None,
            format_mark_due: format < WORKSPACE_FORMAT,
            last_freshness_check: std::time::Instant::now(),
            ledger,
            tracker: crate::tracker::Tracker::from_env(),
        };
        service
            .trash
            .purge(&service.repo, &service.snapshot, Utc::now().timestamp())?;
        service.records.expire(Utc::now().timestamp())?;
        service.refresh()?;
        if service.snapshot.get(&workspace_id()).is_none() {
            let mut m = meta("document", "Soliton workspace")?;
            m.id = workspace_id();
            m.details.body=json!({"schema":1,"page":"Action Center","view":"Calendar","calendarMode":"Week","timezone":"America/Toronto","startHour":5,"endHour":24,"hourHeight":58,"weekStart":0,"savedViews":[],"drafts":{},"consoleOpen":false,"diskOpen":false,"navCollapsed":false}).to_string();
            service.repo.create(Entity::Document(Document {
                meta: m,
                document_type: "workspace".into(),
            }))?;
            service.refresh()?;
        }
        service.preferences()?;
        // A media download the previous engine left running is queued again at start, not at the first collection command.
        if media_left_running(service.repo.root()) {
            service.collection =
                crate::collection::CollectionStore::open(&service.repo.root().join("collection"))
                    .map_err(|e| eprintln!("Media queue recovery: {e}"))
                    .ok();
        }
        Ok(service)
    }
    fn refresh(&mut self) -> Result<()> {
        self.snapshot = self.repo.rebuild()?;
        self.journal_signature = self.owner_journal_signature();
        self.apply_preference_gates();
        Ok(())
    }

    /// Reads the two opt-in settings after every rebuild, so a change saved
    /// here or by another writer takes effect at once. Opening Soliton does no
    /// work: capture runs only when `automaticCapture` is on (and the
    /// `SOLITON_AUTO_CAPTURE=0` force-off is not set), the local model only
    /// when `localModel` is on. A missing or unreadable workspace reads as off.
    fn apply_preference_gates(&mut self) {
        let prefs = self.preferences().unwrap_or(Value::Null);
        let capture = prefs["automaticCapture"] == true && !env_disabled("SOLITON_AUTO_CAPTURE");
        self.local_model = prefs["localModel"] == true;
        if !capture {
            self.capture_due = None;
        } else if self.capture_due.is_none() {
            // Let the UI and service become responsive before the first
            // automatic source check. Subsequent checks remain once per
            // minute, and explicit capture-sync is still immediate.
            self.capture_due = Some(std::time::Instant::now() + capture_delay());
        }
    }

    /// Moves the automatic capture clock forward, as if `by` had passed.
    /// Tests use it to prove the capture gate without waiting.
    #[doc(hidden)]
    pub fn advance_capture_clock(&mut self, by: std::time::Duration) {
        if let Some(due) = self.capture_due {
            self.capture_due = Some(due.checked_sub(by).unwrap_or_else(std::time::Instant::now));
        }
    }

    /// Points the kitchen draft at another serve script. Tests use it to
    /// address a stand-in server.
    #[doc(hidden)]
    pub fn set_decision_serve(&mut self, script: &Path) {
        self.decision_serve = script.to_path_buf();
    }

    fn require_local_model(&self) -> Result<()> {
        if self.local_model {
            Ok(())
        } else {
            Err(invalid(LOCAL_MODEL_OFF))
        }
    }

    /// The governed repository's journal is the durable change boundary for
    /// every entity kind. Owner mutations append a prepare/commit pair after
    /// staging the new object and atomically installing it. Watching this one
    /// file covers documents, reminders and routines as well as planning
    /// objects without scanning every object file on each background tick.
    fn owner_journal_signature(&self) -> Option<(u64, std::time::SystemTime, u64, u64)> {
        let path = self.repo.root().join("journal/operations.log");
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.len(), modified, metadata.dev(), metadata.ino()))
        }
        #[cfg(not(unix))]
        {
            Some((metadata.len(), modified, 0, 0))
        }
    }

    pub fn check_external_freshness(&mut self) -> Result<Option<Value>> {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_freshness_check) < std::time::Duration::from_millis(200) {
            return Ok(None);
        }
        self.last_freshness_check = now;

        let current = self.owner_journal_signature();
        let changed = current != self.journal_signature;
        // A sheet view is shown: a change to the tracker store reaches it as one state event.
        let tracker_changed = self.tracker.poll().unwrap_or_else(|e| {
            eprintln!("Tracker watch: {e}");
            None
        });
        if tracker_changed.is_some() && !changed {
            return Ok(Some(self.state()?));
        }

        if changed {
            if let Err(e) = self.refresh() {
                eprintln!("Freshness check refresh error: {e}");
                return Ok(None);
            }
            return Ok(Some(self.state()?));
        }
        Ok(None)
    }
    /// Point the briefing worker at other programs (a test's fixture client).
    /// Takes effect when the briefing store is first opened.
    pub fn set_briefing_tools(&mut self, tools: crate::briefing::StoreTools) {
        self.briefing_tools = tools;
    }

    pub fn preferences(&self) -> Result<Value> {
        let body = &self
            .snapshot
            .get(&workspace_id())
            .ok_or_else(|| invalid("Workspace record missing"))?
            .entity
            .meta()
            .details
            .body;
        let v: Value = serde_json::from_str(body)?;
        if v["schema"] != 1 {
            return Err(invalid("Unsupported workspace schema; source preserved"));
        }
        Ok(v)
    }
    /// The preferences as Settings reads them: the stored settings plus
    /// `ledgerPrivateModules`, the modules the ledger itself keeps private
    /// (the finance and health marks written by the version 2 migration).
    /// Reported, never mirrored: a read writes no mark, and the key is never
    /// stored back from a save.
    pub fn preferences_read(&self) -> Result<Value> {
        let mut prefs = self.preferences()?;
        let marked = match &self.ledger {
            Some(ledger) => ledger.private_modules()?,
            None => Vec::new(),
        };
        let modules: Vec<&str> = privacy::MODULES
            .iter()
            .copied()
            .filter(|m| marked.iter().any(|k| k == m))
            .collect();
        prefs[LEDGER_MARKS_KEY] = json!(modules);
        Ok(prefs)
    }
    fn get(&self, id: &str) -> Result<&StoredEntity> {
        self.snapshot
            .get(&id.parse()?)
            .ok_or_else(|| invalid("Object no longer exists"))
    }
    fn effective_category(&self, e: &Entity) -> String {
        match e {
            Entity::Task(t) => t
                .aim_id
                .as_ref()
                .and_then(|id| self.snapshot.get(id))
                .and_then(|g| {
                    if let Entity::Aim(a) = &g.entity {
                        a.category_id.as_ref()
                    } else {
                        None
                    }
                })
                .or(t.category_id.as_ref())
                .map(ToString::to_string)
                .unwrap_or_default(),
            Entity::Aim(g) => g
                .category_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
    fn hidden(&self, e: &Entity) -> bool {
        if self.trash.contains(e, &self.snapshot) || e.meta().details.archived {
            return true;
        }
        let category = self.effective_category(e);
        if !category.is_empty()
            && self
                .get(&category)
                .is_ok_and(|r| r.entity.meta().details.archived)
        {
            return true;
        }
        if let Entity::Task(t) = e
            && let Some(id) = &t.aim_id
            && self
                .get(id.as_str())
                .is_ok_and(|r| r.entity.meta().details.archived)
        {
            return true;
        }
        matches!(e,Entity::Task(t) if t.status==TaskStatus::Archived)
            || matches!(e,Entity::Aim(a) if a.state==AimState::Archived)
    }
    pub fn record(&self, e: &Entity) -> Value {
        let m = e.meta();
        let d = &m.details;
        let kind = match e {
            Entity::Aim(_) => "goal",
            Entity::Document(v) => v.document_type.as_str(),
            _ => e.kind().as_str(),
        };
        let mut v = json!({"id":m.id.as_str(),"kind":kind,"title":m.title,"body":d.body,"hash":e.content_hash().map(|h|h.to_string()).unwrap_or_default(),"revision":m.revision,"order":d.order,"archived":d.archived,"hidden":self.hidden(e),"links":d.links,"files":d.files,"tags":d.tags,"schedule":d.schedule,"overrides":d.overrides,"series_history":d.series_history,"created_at":d.created_at,"updated_at":d.updated_at,"category_id":self.effective_category(e),"goal_id":"","status":"active","priority":"medium","scheduled_for":"","due_on":"","target_on":"","recurrence":"none","interval":1,"repeat_until":"","color":"#c8a96b","path":self.repo.root().join("objects").join(e.kind().as_str()).join(format!("{}.soliton",e.id())).to_string_lossy()});
        // A record linked to a tracker row carries the row's id, from its tag `row:<ID>`.
        if let Some(row) = d
            .tags
            .iter()
            .find_map(|t| t.strip_prefix("row:"))
            .filter(|r| !r.is_empty())
        {
            v["row_id"] = json!(row);
        }
        match e {
            Entity::Category(c) => {
                v["color"] = json!(c.color);
            }
            Entity::Aim(a) => {
                v["folder"] = json!(a.folder);
                v["status"] = json!(a.state.as_str());
                v["target_on"] = json!(a.target_on.map(|d| d.to_string()).unwrap_or_default());
                let tasks: Vec<_> = self
                    .snapshot
                    .entities()
                    .filter(|e| {
                        matches!(e,Entity::Task(t) if t.aim_id.as_ref()==Some(&a.meta.id))
                            && !self.hidden(e)
                    })
                    .collect();
                v["task_count"] = json!(tasks.len());
                v["completed_count"] = json!(
                    tasks
                        .iter()
                        .filter(|e| matches!(e,Entity::Task(t) if t.status==TaskStatus::Completed))
                        .count()
                );
            }
            Entity::Task(t) => {
                v["goal_id"] = json!(
                    t.aim_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                );
                v["status"] = json!(t.status.as_str());
                v["priority"] = json!(t.priority.as_str());
                v["scheduled_for"] =
                    json!(t.scheduled_for.map(|d| d.to_string()).unwrap_or_default());
                v["due_on"] = json!(t.due_on.map(|d| d.to_string()).unwrap_or_default());
                v["completed_count"] = json!(t.completed_occurrences.len());
                if let Some(r) = &t.recurrence {
                    v["recurrence"] = json!(cadence_label(r));
                    v["interval"] = json!(r.interval);
                    v["repeat_until"] = json!(r.ends_on.map(|d| d.to_string()).unwrap_or_default());
                    v["repeat_start"] = json!(r.starts_on.to_string());
                }
            }
            _ => {}
        }
        let category = s(&v, "category_id").to_string();
        if !category.is_empty()
            && let Ok(c) = self.get(&category)
            && let Entity::Category(c) = &c.entity
        {
            v["color"] = json!(c.color.clone().unwrap_or_else(|| "#c8a96b".into()));
            v["category_title"] = json!(c.meta.title);
        }
        if let Ok(g) = self.get(s(&v, "goal_id")) {
            v["goal_title"] = json!(g.entity.meta().title);
        }
        v
    }
    pub fn state(&self) -> Result<Value> {
        let privacy = self.privacy()?;
        let records: Vec<_> = self
            .snapshot
            .entities()
            .filter(|e| e.id() != &workspace_id())
            .filter(|e| !self.trash.contains(e, &self.snapshot))
            .filter(|e| !privacy.hides_planner(e.id().as_str()))
            .map(|e| {
                let mut record = self.record(e);
                if privacy.on && matches!(e, Entity::Aim(_)) {
                    // A goal's counts cover only the tasks still shown.
                    let tasks: Vec<_> = self.snapshot.entities().filter(|t| {
                        matches!(t, Entity::Task(task) if task.aim_id.as_ref() == Some(e.id()))
                            && !self.hidden(t) && !privacy.hides_planner(t.id().as_str())
                    }).collect();
                    record["task_count"] = json!(tasks.len());
                    record["completed_count"] = json!(tasks.iter().filter(|t| matches!(t, Entity::Task(task) if task.status == TaskStatus::Completed)).count());
                }
                record
            })
            .collect();
        Ok(
            json!({"records":records,"ui":self.preferences_read()?,"root":self.repo.root(),"recovered":self.recovered,"schema":2,"kernel":"CORE-007 r03 + desktop v2","app_version":env!("CARGO_PKG_VERSION"),"release":release_identity(),"now":Utc::now().timestamp(),"trackerRevision":self.tracker.revision()}),
        )
    }
    fn expected(&self, args: &Value) -> Result<ContentHash> {
        let hash = s(args, "hash");
        if hash.is_empty() {
            return Err(invalid(
                "Edit token missing. Reload the object before saving.",
            ));
        }
        Ok(hash.parse()?)
    }
    fn commit(&mut self, mut entity: Entity, args: &Value, creating: bool) -> Result<Value> {
        if entity.meta().title.trim().is_empty() {
            return Err(invalid("Give this object a title"));
        }
        if entity.meta().title.len() > 1000 {
            return Err(invalid("Title is too long"));
        }
        entity.meta_mut().details.updated_at = Utc::now().timestamp();
        let stored = if creating {
            self.repo.create(entity)?
        } else {
            self.repo.update(entity, self.expected(args)?)?
        };
        self.refresh()?;
        Ok(self.record(&stored.entity))
    }
    pub fn save(&mut self, args: &Value) -> Result<Value> {
        let creating = s(args, "id").is_empty();
        if !creating
            && self
                .trash
                .contains(&self.get(s(args, "id"))?.entity, &self.snapshot)
        {
            return Err(invalid("Restore this item from Trash before editing it."));
        }
        let existing = if creating {
            None
        } else {
            Some(self.get(s(args, "id"))?.entity.clone())
        };
        let mut fields=existing.as_ref().map(|e|self.record(e)).unwrap_or_else(||json!({"kind":s(args,"kind"),"title":"","status":"todo","priority":"medium","recurrence":"none","interval":1,"schedule":null,"order":self.snapshot.entities().map(|e|e.meta().details.order).max().unwrap_or(0)+1_048_576}));
        if !args.is_object() {
            return Err(invalid("Save requires an object"));
        }
        for (k, v) in args.as_object().unwrap() {
            fields[k] = v.clone();
        }
        let kind = s(&fields, "kind");
        let mut m = existing.as_ref().map(|e| e.meta().clone()).unwrap_or(meta(
            match kind {
                "goal" => "aim",
                "category" | "task" => kind,
                _ => "document",
            },
            s(&fields, "title"),
        )?);
        m.title = s(&fields, "title").trim().into();
        if !creating {
            m.revision += 1;
        }
        m.details.body = s(&fields, "body").into();
        m.details.order = fields["order"].as_i64().unwrap_or(m.details.order);
        m.details.archived = fields["archived"].as_bool().unwrap_or(false);
        m.details.links = strings(&fields["links"]);
        m.details.files = strings(&fields["files"]);
        m.details.tags = strings(&fields["tags"]);
        for link in &m.details.links {
            if link == m.id.as_str() {
                return Err(invalid("An object cannot link to itself"));
            }
            self.get(link)?;
        }
        let e = match kind {
            "category" => {
                let color = s(&fields, "color");
                if !color.is_empty()
                    && !(color.len() == 7
                        && color.starts_with('#')
                        && color[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    return Err(invalid("Use a color such as #c8a96b"));
                }
                Entity::Category(Category {
                    meta: m,
                    color: Some(if color.is_empty() { "#c8a96b" } else { color }.into()),
                    sort_order: 0,
                })
            }
            "goal" => Entity::Aim(Aim {
                meta: m,
                category_id: optional_id(&fields, "category_id")?,
                state: AimState::parse(if s(&fields, "status") == "todo" {
                    "active"
                } else {
                    s(&fields, "status")
                })?,
                target_on: optional_date(&fields, "target_on")?,
                folder: match fields.get("folder") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(folder)) if folder.is_empty() => None,
                    Some(Value::String(folder)) => Some(folder.clone()),
                    _ => return Err(invalid("Goal folder must be an absolute path or empty")),
                },
            }),
            "task" => {
                let goal = optional_id(&fields, "goal_id")?;
                let category = if goal.is_some() {
                    None
                } else {
                    optional_id(&fields, "category_id")?
                };
                let scheduled_for = optional_date(&fields, "scheduled_for")?;
                let recurrence = rule(&fields)?;
                if recurrence.is_some() && s(&fields, "status") == "completed" {
                    return Err(invalid(
                        "Complete an occurrence in the calendar. To end the series, set its end date or archive it.",
                    ));
                }
                let mut completed = match &existing {
                    Some(Entity::Task(t)) => t.completed_occurrences.clone(),
                    _ => BTreeSet::new(),
                };
                if let Some(Entity::Task(old)) = &existing
                    && old.recurrence != recurrence
                {
                    if let Some(r) = &old.recurrence {
                        m.details.series_history.push(SeriesHistory {
                            changed_at: Utc::now().timestamp(),
                            rule_hash: r.content_hash().to_string(),
                            completed: completed
                                .iter()
                                .map(|o| {
                                    format!("{}|{}|{}", o.scheduled_for, o.sequence, o.opaque_key())
                                })
                                .collect(),
                            overrides: m.details.overrides.clone(),
                        });
                    }
                    completed.clear();
                    m.details.overrides.clear();
                }
                m.details.schedule = if fields["schedule"].is_object() {
                    let schedule = &fields["schedule"];
                    let day = scheduled_for
                        .ok_or_else(|| invalid("A timed task needs a scheduled date"))?
                        .to_string();
                    Some(time::scheduled(
                        &day,
                        s(schedule, "time"),
                        duration(schedule)?,
                        s(schedule, "timezone"),
                        s(schedule, "fold"),
                    )?)
                } else {
                    None
                };
                Entity::Task(Task {
                    meta: m,
                    category_id: category,
                    aim_id: goal,
                    priority: Priority::parse(s(&fields, "priority"))?,
                    status: TaskStatus::parse(if s(&fields, "status") == "active" {
                        "todo"
                    } else {
                        s(&fields, "status")
                    })?,
                    scheduled_for,
                    due_on: optional_date(&fields, "due_on")?,
                    recurrence,
                    completed_occurrences: completed,
                })
            }
            "note" | "strategy" | "learning" => Entity::Document(Document {
                meta: m,
                document_type: kind.into(),
            }),
            _ => return Err(invalid("Unsupported object type")),
        };
        self.commit(e, args, creating)
    }
    fn reorder(&mut self, args: &Value) -> Result<Value> {
        let current = self.get(s(args, "id"))?.clone();
        if current.content_hash != self.expected(args)? {
            return Err(invalid("This card changed. Reload it before moving it."));
        }
        let r = self.record(&current.entity);
        let target = self.record(&self.get(s(args, "target"))?.entity);
        if r["id"] == target["id"] {
            return Ok(r);
        }
        if r["kind"] != target["kind"] || !matches!(s(&r, "kind"), "category" | "goal") {
            return Err(invalid(
                "Move a category beside a category, or a goal beside a goal",
            ));
        }
        let mut peers: Vec<Value> = self
            .snapshot
            .entities()
            .map(|e| self.record(e))
            .filter(|v| v["kind"] == target["kind"] && v["hidden"] == target["hidden"])
            .collect();
        peers.sort_by(|a, b| {
            a["order"]
                .as_i64()
                .cmp(&b["order"].as_i64())
                .then_with(|| s(a, "id").cmp(s(b, "id")))
        });
        let after = args["after"].as_bool().unwrap_or(false);
        // Only sparse-rank exhaustion rewrites neighbours. Move the last card
        // first into unused high ranks so every interrupted prefix preserves
        // their relative order. Each metadata write has journal/history/CAS.
        for attempt in 0..2 {
            let remaining: Vec<_> = peers.iter().filter(|v| v["id"] != r["id"]).collect();
            let i = remaining
                .iter()
                .position(|v| v["id"] == target["id"])
                .ok_or_else(|| invalid("Target card disappeared"))?
                + usize::from(after);
            let left = if i > 0 {
                remaining[i - 1]["order"].as_i64()
            } else {
                None
            };
            let right = remaining.get(i).and_then(|v| v["order"].as_i64());
            let order = match (left, right) {
                (Some(l), Some(h)) if (h as i128 - l as i128) > 1 => {
                    Some((l as i128 + (h as i128 - l as i128) / 2) as i64)
                }
                (Some(l), None) => l.checked_add(1_048_576),
                (None, Some(h)) => h.checked_sub(1_048_576),
                _ => None,
            };
            if let Some(order) = order {
                let hash = self.get(s(&r, "id"))?.content_hash.to_string();
                return self.save(&json!({"id":r["id"],"hash":hash,"order":order}));
            }
            if attempt == 1 {
                break;
            }
            let base = peers
                .iter()
                .filter_map(|v| v["order"].as_i64())
                .max()
                .unwrap_or(0);
            base.checked_add((peers.len() as i64 + 2) * 1_048_576)
                .ok_or_else(|| invalid("Ordering range exceeded"))?;
            for i in (0..peers.len()).rev() {
                let v = &peers[i];
                peers[i] = self.save(
                    &json!({"id":v["id"],"hash":v["hash"],"order":base+(i as i64+1)*1_048_576}),
                )?;
            }
        }
        Err(invalid("Could not place this card"))
    }
    /// Advances durable collectors and explicitly opted-in history sources while
    /// retaining the service as the only canonical/history writer.
    pub fn background_tick(&mut self) -> Result<()> {
        if std::time::Instant::now() < self.background_due {
            return Ok(());
        }
        self.background_due = std::time::Instant::now() + std::time::Duration::from_secs(2);
        self.poll_history_job();
        if env_enabled("SOLITON_COLLECTION_AUTOSTART") {
            let _ = self.tick_collection(true)?;
        } else if self.collection.is_some() {
            // Explicit collection-add/check owns execution. Once that store
            // exists, the normal service loop may drain its active result and
            // archive it, but it must not enqueue any other due source.
            let _ = self.tick_collection(false)?;
        }
        if self.history_job.is_none()
            && self
                .capture_due
                .is_some_and(|due| std::time::Instant::now() >= due)
        {
            self.capture_due = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
            if self
                .repo
                .root()
                .join("history/capture-sources.json")
                .exists()
            {
                let _ =
                    self.start_history_job(&json!({"command":"history-capture-sync","args":{}}))?;
            }
        }
        Ok(())
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        let before = self.format_mark_due.then(|| self.owner_journal_signature());
        let result = self.dispatch(command, args);
        if let Some(before) = before
            && self.owner_journal_signature() != before
        {
            self.write_format_mark()?;
        }
        result
    }
    /// Records this engine's format once the workspace has been written in it.
    fn write_format_mark(&mut self) -> Result<()> {
        let path = self.repo.root().join(FORMAT_MARK);
        let temporary = path.with_extension("json.pending");
        let mark = json!({"format":WORKSPACE_FORMAT,"written_by":env!("CARGO_PKG_VERSION")});
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)?;
        file.write_all(mark.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        self.format_mark_due = false;
        Ok(())
    }
    fn dispatch(&mut self, command: &str, args: &Value) -> Result<Value> {
        let privacy = self.privacy()?;
        if let Some(ledger) = self.ledger.as_ref() {
            let writers: Vec<&str> = privacy::LEDGER_WRITERS
                .iter()
                .filter(|(module, _)| privacy.modules.contains(*module))
                .filter_map(|(_, writer)| *writer)
                .collect();
            let mut ids = privacy.ids.clone();
            ids.extend(strings(&self.preferences()?["privateIds"]));
            ledger.set_screen_marks(
                &ids.iter().map(String::as_str).collect::<Vec<_>>(),
                &writers,
            )?;
        }
        // History pages after it drops what is hidden, so every page's total
        // agrees with the records it can show.
        let hidden_args;
        let args = if privacy.on && !privacy.ids.is_empty() && command == "history-list" {
            hidden_args = with_hidden(args, &privacy.ids);
            &hidden_args
        } else if privacy.on
            && !privacy.ids.is_empty()
            && command == "history-job-start"
            && args["command"] == "history-list"
        {
            let mut a = args.clone();
            a["args"] = with_hidden(&args["args"], &privacy.ids);
            hidden_args = a;
            &hidden_args
        } else {
            args
        };
        let result = self.command_unfiltered(command, args)?;
        let privacy = self.privacy()?;
        let Some(module) = privacy::view_module(command, args).filter(|_| privacy.on) else {
            return Ok(result);
        };
        let result = if command == "briefing-list" {
            privacy.drop_briefings(&self.repo.root().join("briefings/jobs"), result)
        } else {
            result
        };
        Ok(privacy.filter(module, result))
    }
    /// Private-mode state from the workspace settings, with the identifiers
    /// of every planner object hidden by a mark on itself, its category, its
    /// goal, or an object it links to. Marks live in the workspace setting
    /// `privateIds`, never in object bytes, so a rollback reads every object.
    pub fn privacy(&self) -> Result<privacy::Privacy> {
        let prefs = self.preferences()?;
        let on = prefs["privateMode"].as_bool().unwrap_or(false);
        let modules = strings(&prefs["privateModules"]).into_iter().collect();
        let marks: std::collections::BTreeSet<String> =
            strings(&prefs["privateIds"]).into_iter().collect();
        // A mark on something that is not a planner object (a History record)
        // hides that object itself.
        let mut ids = if on {
            marks.clone()
        } else {
            std::collections::BTreeSet::new()
        };
        if on && !marks.is_empty() {
            let marked = |id: &str| marks.contains(id);
            for e in self.snapshot.entities() {
                let goal = match e {
                    Entity::Task(t) => t
                        .aim_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let category = self.effective_category(e);
                let own_category = match e {
                    Entity::Task(t) => t
                        .category_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                if marked(e.id().as_str())
                    || [goal, category, own_category]
                        .iter()
                        .any(|id| !id.is_empty() && marked(id))
                    || e.meta().details.links.iter().any(|id| marked(id))
                {
                    ids.insert(e.id().to_string());
                }
            }
        }
        Ok(privacy::Privacy { on, modules, ids })
    }
    /// Context carries page/view and an optional selected planner id; folders
    /// are derived from stored entities, never from an arbitrary launch cwd.
    fn tool_folder(&self, context: &Value) -> Result<PathBuf> {
        if !context.is_null() && !context.is_object() {
            return Err(invalid(
                "Terminal context must contain page, view and selected_id",
            ));
        }
        let selected = context
            .get("selected_id")
            .or_else(|| context.get("selectedId"))
            .or_else(|| context.get("id"));
        if let Some(selected) = selected
            && !selected.is_null()
        {
            let id = selected
                .as_str()
                .ok_or_else(|| invalid("Selected item must be an identifier"))?;
            if !id.is_empty() {
                let entity = &self.get(id)?.entity;
                if self.privacy()?.hides_planner(id) || self.trash.contains(entity, &self.snapshot)
                {
                    return Err(invalid("The selected item is not available in this view"));
                }
                match entity {
                    Entity::Aim(goal) => {
                        if let Some(folder) = &goal.folder {
                            return Ok(PathBuf::from(folder));
                        }
                    }
                    Entity::Document(_) => {
                        return Ok(self.repo.root().join("objects/document"));
                    }
                    _ => {}
                }
            }
        }
        let documents = self.repo.root().join("documents");
        fs::create_dir_all(&documents).map_err(|error| {
            invalid(format!(
                "Could not prepare documents folder {}: {error}",
                documents.display()
            ))
        })?;
        Ok(documents)
    }

    fn command_unfiltered(&mut self, command: &str, args: &Value) -> Result<Value> {
        if command == "tool-available" {
            return Ok(self.tool_launcher.available());
        }
        if command == "tool-launch" {
            let tool = args.get("tool").and_then(Value::as_str).unwrap_or("agent");
            let folder = self.tool_folder(&args["context"])?;
            return self
                .tool_launcher
                .launch(tool, &folder)
                .map_err(|error| invalid(error.to_string()));
        }

        if matches!(command, "zeno-settings-get" | "zeno-settings-set") {
            return zeno_settings::command(command, args);
        }
        // The desktop, the machine's log and its processes: read from the
        // compositor and the system, never from the workspace, so they carry
        // no record content (privacy::view_module answers None for them).
        if command.starts_with("desktop-") {
            return crate::desktop::command(command, args);
        }
        if command == "machine-log" {
            return machine_log_command(args);
        }
        if matches!(command, "process-list" | "process-stop") {
            return crate::processes::command(command, args)
                .map_err(|error| invalid(error.to_string()));
        }
        if command == "planner-trash-list" {
            let privacy = self.privacy()?;
            let entries = self
                .trash
                .entries()
                .iter()
                .filter(|entry| !privacy.hides_planner(&entry.id))
                .map(|entry| {
                    let mut value = serde_json::to_value(entry)?;
                    let title = if let Ok(stored) = self.get(&entry.id) {
                        stored.entity.meta().title.clone()
                    } else {
                        self.repo
                            .expired_trash()?
                            .into_iter()
                            .find(|stored| stored.entity.id().as_str() == entry.id)
                            .ok_or_else(|| invalid("Retained Trash item is unavailable"))?
                            .entity
                            .meta()
                            .title
                            .clone()
                    };
                    value["title"] = json!(title);
                    value["word"] = json!(trash_word(&entry.kind));
                    Ok(value)
                })
                .collect::<Result<Vec<Value>>>()?;
            let records = self
                .records
                .entries()
                .iter()
                .filter(|entry| !hides_record(&privacy, &entry.kind, &entry.id))
                .map(|entry| {
                    json!({"kind":entry.kind,"id":entry.id,"title":self.record_title(entry),
                        "deleted_at":entry.deleted_at,"word":trash_word(&entry.kind)})
                });
            return Ok(json!(
                entries.into_iter().chain(records).collect::<Vec<_>>()
            ));
        }
        if matches!(command, "planner-trash" | "planner-trash-restore")
            && soliton_kernel::trash::is_record_kind(s(args, "kind"))
        {
            return if command == "planner-trash" {
                self.record_trash(s(args, "kind"), s(args, "id"))
            } else {
                self.record_restore(s(args, "kind"), s(args, "id"))
            };
        }
        if command == "collection-remove" {
            return self.record_trash("collection-source", s(args, "id"));
        }
        if command == "planner-trash" || command == "planner-trash-restore" {
            if command == "planner-trash" {
                self.trash.trash(
                    self.repo.root(),
                    &self.snapshot,
                    s(args, "kind"),
                    s(args, "id"),
                    Utc::now().timestamp(),
                )?;
            } else if self.get(s(args, "id")).is_err()
                && self
                    .trash
                    .entries()
                    .iter()
                    .any(|entry| entry.kind == s(args, "kind") && entry.id == s(args, "id"))
            {
                self.repo.restore_expired_trash(&s(args, "id").parse()?)?;
                self.trash = soliton_kernel::trash::TrashIndex::open(self.repo.root())?;
                self.refresh()?;
            } else {
                self.trash
                    .restore(self.repo.root(), s(args, "kind"), s(args, "id"))?;
            }
            // The CLI does not attach state to new commands yet. Return it
            // explicitly for existing UI callers to refresh immediately.
            return Ok(json!({"kind":args["kind"],"id":args["id"],"state":self.state()?}));
        }
        if matches!(
            command,
            "complete" | "occurrence" | "restore" | "reorder" | "promote" | "duplicate"
        ) && self
            .trash
            .contains(&self.get(s(args, "id"))?.entity, &self.snapshot)
        {
            return Err(invalid("Restore this item from Trash before changing it."));
        }
        if command.starts_with("modules-") {
            if self.modules.is_none() {
                self.modules = Some(crate::modules::ModulesStore::open(
                    &self.repo.root().join("modules"),
                )?);
            }
            return self.modules.as_mut().unwrap().command(command, args);
        }
        self.poll_history_job();
        if command.starts_with("inference-") {
            self.require_local_model()?;
            return self.inference.command(command);
        }
        if command.starts_with("briefing-") {
            return self.briefing_command(command, args);
        }
        if command.starts_with("source-policy-") {
            return self.source_policy_command(command, args);
        }
        if command == "collection-tick" {
            return self.tick_collection(true);
        }
        if command.starts_with("tracker-") {
            return self.tracker.command(command, args);
        }
        if command.starts_with("collection-") {
            if self.collection.is_none() {
                self.collection = Some(crate::collection::CollectionStore::open(
                    &self.repo.root().join("collection"),
                )?);
            }
            return self.collection.as_mut().unwrap().command(command, args);
        }
        if command == "history-job-start" {
            return self.start_history_job(args);
        }
        if command == "history-job-status" {
            if let Some(value) = args["id"]
                .as_str()
                .and_then(|id| self.history_job_receipts.get(id))
            {
                // A read run in the background is filtered as it is read, so a
                // mark or the mode applies to it as to the same read made directly.
                let mut value = value.clone();
                let privacy = self.privacy()?;
                if let Some(module) =
                    privacy::view_module(s(&value, "command"), &Value::Null).filter(|_| privacy.on)
                    && value["ok"] == true
                {
                    value["result"] = privacy.filter(module, value["result"].take());
                }
                return Ok(value);
            }
            if let Some((id, _)) = &self.history_job
                && args["id"] == *id
            {
                return Ok(json!({"id":id,"state":"running"}));
            }
            return Err(invalid(
                "History receipt expired from the bounded cache; retry the operation. Archived sources are deduplicated.",
            ));
        }
        if command.starts_with("history-") && self.history_job.is_some() {
            return Err(invalid(
                "History is importing a source. Existing views are preserved; retry when it finishes.",
            ));
        }
        if command == "history-from-inbox" {
            if self.inbox.is_none() {
                self.inbox = Some(crate::inbox::InboxStore::open(
                    &self.repo.root().join("inbox"),
                )?);
            }
            let status = self.inbox.as_mut().unwrap().command("inbox-status", args)?;
            if status["state"] != "committed" || status["source_intent"] != "voice_note" {
                return Err(invalid("Select a committed voice-note transfer"));
            }
            if self.history.is_none() {
                self.history = Some(crate::history::HistoryStore::open(
                    &self.repo.root().join("history"),
                )?);
            }
            return self
                .history
                .as_mut()
                .unwrap()
                .register_inbox(&status["receipt"]);
        }
        if command.starts_with("history-") {
            return self.history_command(command, args);
        }
        if command.starts_with("inbox-") {
            if self.inbox.is_none() {
                self.inbox = Some(crate::inbox::InboxStore::open(
                    &self.repo.root().join("inbox"),
                )?);
            }
            return self.inbox.as_mut().unwrap().command(command, args);
        }
        if matches!(command, "recovery-preview" | "recovery-restore")
            && s(args, "id").starts_with(EXPIRED_RECORD)
        {
            return self.expired_record(command, args);
        }
        if command.starts_with("recovery-") {
            if self.recovery.is_none() {
                self.recovery = Some(crate::recovery::RecoveryStore::open(
                    &self.repo.root().join("recovery"),
                )?);
            }
            let mut result = self.recovery.as_mut().unwrap().command(command, args)?;
            if command == "recovery-list" {
                let items = self.expired_record_items(args["path"].as_str())?;
                if let Some(list) = result["items"].as_array_mut() {
                    list.extend(items);
                }
            }
            if command == "recovery-restore"
                && args["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("expired-trash-"))
            {
                self.trash = soliton_kernel::trash::TrashIndex::open(self.repo.root())?;
                self.refresh()?;
            }
            return Ok(result);
        }
        if command.starts_with("proposal-")
            || command == "intelligence-analyze"
            || command == "intelligence-stages"
        {
            return self.intelligence_command(command, args);
        }
        match command {
            "state" => self.state(),
            "save" => self.save(args),
            "preferences" => {
                let stored = self.get(workspace_id().as_str())?.clone();
                let mut entity = stored.entity;
                let mut prefs = self.preferences()?;
                if !args.is_object() {
                    return Err(invalid("Preferences must be an object"));
                }
                // While the mode is on every read shows the marks filtered out
                // (an empty list), so a caller saving its settings back would
                // wipe them all. Then an empty list leaves the marks as they
                // are, unless the caller writes the marks themselves from an
                // unfiltered copy (`replaceMarks`), as lifting the last mark does.
                let guarded = prefs["privateMode"].as_bool().unwrap_or(false)
                    && args["replaceMarks"].as_bool() != Some(true);
                for (k, v) in args.as_object().unwrap() {
                    let wipe = guarded
                        && ["privateIds", "privateModules"].contains(&k.as_str())
                        && v.as_array().is_some_and(Vec::is_empty);
                    if k != "schema" && k != LEDGER_MARKS_KEY && k != "replaceMarks" && !wipe {
                        prefs[k] = v.clone();
                    }
                }
                time::zone(s(&prefs, "timezone"))?;
                for k in ["startHour", "endHour"] {
                    let hour = prefs[k]
                        .as_i64()
                        .ok_or_else(|| invalid("Planning hours must be whole numbers"))?;
                    if !(0..=24).contains(&hour) {
                        return Err(invalid("Planning hours must be 0–24"));
                    }
                }
                let h = prefs["hourHeight"]
                    .as_i64()
                    .ok_or_else(|| invalid("Calendar density must be a whole number"))?;
                if !(32..=120).contains(&h) {
                    return Err(invalid("Calendar density must be 32–120"));
                }
                if prefs
                    .get("reducedMotion")
                    .is_some_and(|value| !value.is_boolean())
                {
                    return Err(invalid("Reduce motion must be enabled or disabled"));
                }
                if prefs
                    .get("privateMode")
                    .is_some_and(|value| !value.is_boolean())
                {
                    return Err(invalid("Private mode must be on or off"));
                }
                for (key, name) in [
                    ("automaticCapture", "Automatic capture"),
                    ("localModel", "Local model"),
                    ("kitchenDraft", "Kitchen draft"),
                ] {
                    if prefs.get(key).is_some_and(|value| !value.is_boolean()) {
                        return Err(invalid(format!("{name} must be on or off")));
                    }
                }
                if let Some(modules) = prefs.get("privateModules")
                    && !modules.as_array().is_some_and(|list| {
                        list.iter()
                            .all(|m| m.as_str().is_some_and(|m| privacy::MODULES.contains(&m)))
                    })
                {
                    return Err(invalid("Private modules must be a list of known modules"));
                }
                if let Some(marks) = prefs.get("privateIds")
                    && !marks.as_array().is_some_and(|list| {
                        list.len() <= 100_000
                            && list.iter().all(|id| {
                                id.as_str()
                                    .is_some_and(|id| !id.is_empty() && id.len() <= 160)
                            })
                    })
                {
                    return Err(invalid("Private items must be a list of identifiers"));
                }
                entity.meta_mut().details.body = prefs.to_string();
                entity.meta_mut().revision += 1;
                self.repo.update(entity, stored.content_hash)?;
                self.refresh()?;
                self.preferences_read()
            }
            "complete" => {
                let mut entity = self.get(s(args, "id"))?.entity.clone();
                let check_in;
                if let Entity::Task(t) = &mut entity {
                    if t.recurrence.is_some() {
                        let occ = validate_occurrence(t, args)?;
                        let done = !t.completed_occurrences.remove(&occ);
                        if done {
                            t.completed_occurrences.insert(occ.clone());
                        }
                        check_in = (t.clone(), done, Some(occ));
                    } else {
                        t.status = if t.status == TaskStatus::Completed {
                            TaskStatus::Todo
                        } else {
                            TaskStatus::Completed
                        };
                        check_in = (t.clone(), t.status == TaskStatus::Completed, None);
                    }
                } else {
                    return Err(invalid("Only tasks can be completed here"));
                }
                entity.meta_mut().revision += 1;
                let record = self.commit(entity, args, false)?;
                self.ledger_task_check_in(&check_in.0, check_in.1, check_in.2.as_ref());
                Ok(record)
            }
            "occurrence" => {
                let mut entity = self.get(s(args, "id"))?.entity.clone();
                let mut check_in = None;
                if let Entity::Task(t) = &mut entity {
                    let occ = validate_occurrence(t, args)?;
                    let was_done = t.completed_occurrences.contains(&occ);
                    let mut ov = t
                        .meta
                        .details
                        .overrides
                        .get(occ.opaque_key().as_str())
                        .cloned()
                        .unwrap_or_default();
                    ov.origin_date = occ.scheduled_for.to_string();
                    if let Some(day) = args["date"].as_str() {
                        time::date(day)?;
                        ov.date = Some(day.into());
                    }
                    if let Some(title) = args["title"].as_str() {
                        if title.trim().is_empty() {
                            return Err(invalid("Title cannot be empty"));
                        }
                        ov.title = Some(title.into());
                    }
                    if let Some(c) = args["cancelled"].as_bool() {
                        ov.cancelled = c;
                    }
                    if let Some(value) = args.get("status") {
                        let status = TaskStatus::parse(
                            value
                                .as_str()
                                .ok_or_else(|| invalid("Occurrence status must be text"))?,
                        )?;
                        match status {
                            TaskStatus::Completed => {
                                t.completed_occurrences.insert(occ.clone());
                            }
                            TaskStatus::Todo | TaskStatus::InProgress => {
                                t.completed_occurrences.remove(&occ);
                                ov.status = Some(status.as_str().into());
                            }
                            TaskStatus::Archived => {
                                return Err(invalid("Archive the task, not an occurrence"));
                            }
                        }
                    }
                    if args["schedule"].is_object() {
                        let z = &args["schedule"];
                        ov.all_day = false;
                        ov.schedule = Some(time::scheduled(
                            ov.date.as_deref().unwrap_or(&occ.scheduled_for.to_string()),
                            s(z, "time"),
                            duration(z)?,
                            s(z, "timezone"),
                            s(z, "fold"),
                        )?);
                    } else if args.get("schedule").is_some_and(Value::is_null) {
                        ov.schedule = None;
                        ov.all_day = true;
                    }
                    let done = t.completed_occurrences.contains(&occ);
                    t.meta
                        .details
                        .overrides
                        .insert(occ.opaque_key().to_string(), ov);
                    if done != was_done {
                        check_in = Some((t.clone(), done, occ));
                    }
                } else {
                    return Err(invalid("Not a recurring task"));
                }
                entity.meta_mut().revision += 1;
                let record = self.commit(entity, args, false)?;
                if let Some((task, done, occ)) = check_in {
                    self.ledger_task_check_in(&task, done, Some(&occ));
                }
                Ok(record)
            }
            "ledger-question" => self.ledger_question(args),
            "ledger-mark-private" => self.ledger_mark_private(args),
            "ledger-unmark-private" => self.ledger_unmark_private(args),
            "habit-define" => self.habit_define(args),
            "habit-list" => self.habit_list(args),
            "habit-link" => self.habit_link(args),
            "habit-actions" => self.habit_actions(args),
            "habit-action-set" => self.habit_action_set(args),
            "habit-bundle-order" => self.habit_bundle_order(args),
            "habit-check-in" => self.habit_check_in(args),
            "habit-correct" => self.habit_correct(args),
            "habit-retract" => self.habit_retract(args),
            "habit-restore" => self.habit_restore(args),
            "habit-heat-map" => self.habit_heat_map(args),
            "kitchen-stock-add" => self.kitchen_stock_add(args),
            "kitchen-stock-use" => self.kitchen_stock_change(args, "use"),
            "kitchen-stock-adjust" => self.kitchen_stock_change(args, "adjust"),
            "kitchen-stock-expiry" => self.kitchen_stock_change(args, "expiry"),
            "kitchen-stock-list" => self.kitchen_stock_list(args),
            "kitchen-equipment-set" => self.kitchen_equipment_set(args),
            "kitchen-equipment-list" => self.kitchen_equipment_list(args),
            "kitchen-profile-set" => self.kitchen_profile_set(args),
            "kitchen-profile-list" => self.kitchen_profile_list(args),
            "kitchen-goes-off" => self.kitchen_goes_off(args),
            "kitchen-batch-plan" => self.kitchen_batch_plan(args),
            "kitchen-plans" => self.kitchen_plans(args),
            "kitchen-cook" => self.kitchen_cook(args),
            "kitchen-meals" => self.kitchen_meals(args),
            "kitchen-profile-remove" => self.kitchen_profile_remove(args),
            "kitchen-lot-move" => self.kitchen_lot_move(args),
            "kitchen-stock-picture" => self.kitchen_stock_picture(args),
            "kitchen-picture-check" => self.kitchen_picture_check(args),
            "kitchen-from-purchases" => self.kitchen_from_purchases(args),
            "kitchen-lot-from-purchase" => self.kitchen_lot_from_purchase(args),
            "kitchen-draft-input" => self.kitchen_draft_input(args),
            "kitchen-draft-accept" => self.kitchen_draft_accept(args),
            "kitchen-draft" => self.kitchen_draft(args),
            "finance-statement-header" => self.finance_statement_header(args),
            "finance-banks" => self.finance_banks(args),
            "finance-statement-mapping" => self.finance_statement_mapping(args),
            "finance-statement-import" => self.finance_statement_import(args),
            "finance-lines" => self.finance_lines(args),
            "finance-income" => self.finance_income(args),
            "finance-recategorise" => self.finance_recategorise(args),
            "finance-intake" => self.finance_intake(args),
            "finance-document-register" => self.finance_document_register(args),
            "finance-import" => self.finance_import(args),
            "finance-receipt-lines" => self.finance_receipt_lines(args),
            "finance-cash" => self.finance_cash(args),
            "finance-documents" => self.finance_documents(args),
            "finance-categories" => self.finance_categories(args),
            "now-view" => self.now_view(args),
            // A new ledger is created at the current version; a version 1
            // ledger is migrated in place (additive, one transaction).
            "ledger-migrate" => {
                let from = match self.ledger.as_mut() {
                    Some(ledger) if ledger.version() < SCHEMA_VERSION => Some(ledger.upgrade()?),
                    Some(_) => return Err(invalid("This workspace already has a ledger")),
                    None => {
                        self.ledger = Some(Ledger::migrate(self.repo.root())?);
                        None
                    }
                };
                let kinds = self.ledger_ref()?.kinds()?.len();
                Ok(
                    json!({"ledger":LEDGER_FILE,"schema_version":SCHEMA_VERSION,"from_version":from,"kinds":kinds}),
                )
            }
            "calendar" => self.calendar(s(args, "from"), s(args, "through"), s(args, "timezone")),
            "clock" => {
                Ok(json!({"local":time::local_string(Utc::now().timestamp(),s(args,"timezone"))?}))
            }
            "history" => {
                let e = self.get(s(args, "id"))?;
                Ok(json!(
                    self.repo
                        .versions(e.entity.id())?
                        .iter()
                        .map(|v| self.record(&v.entity))
                        .collect::<Vec<_>>()
                ))
            }
            "restore" => {
                let current = self.get(s(args, "id"))?.clone();
                let mut old = self
                    .repo
                    .versions(current.entity.id())?
                    .into_iter()
                    .find(|v| v.content_hash.to_string() == s(args, "version"))
                    .ok_or_else(|| invalid("Version no longer available"))?
                    .entity;
                old.meta_mut().revision = current.entity.meta().revision + 1;
                self.commit(old, args, false)
            }
            "promote" => {
                let source = self.get(s(args, "id"))?.entity.clone();
                // New task links back to the idea. The source is unchanged, so
                // the entire promotion is one journaled creation.
                self.save(&json!({"kind":"task","title":source.meta().title,"body":source.meta().details.body,"goal_id":s(args,"goal_id"),"links":[source.id().as_str()]}))
            }
            "duplicate" => {
                let mut record = self.record(&self.get(s(args, "id"))?.entity);
                record["id"] = json!("");
                record["title"] = json!(format!("{} (copy)", s(&record, "title")));
                record["archived"] = json!(false);
                self.save(&record)
            }
            "import_preview" => crate::transfer::preview(Path::new(s(args, "path"))),
            "import_workspace" => crate::transfer::import(
                Path::new(s(args, "path")),
                s(args, "hash"),
                Path::new(s(args, "target")),
            ),
            "source" => read_source(s(args, "path")),
            "reorder" => self.reorder(args),
            "export" => {
                let path = PathBuf::from(s(args, "path"));
                if !path.is_absolute() {
                    return Err(invalid("Choose an absolute export path"));
                }
                let packet = json!({"format":"soliton-export/2","exported_at":Utc::now().to_rfc3339(),"objects":self.snapshot.entities().filter(|e|e.id()!=&workspace_id()).map(|e|String::from_utf8(e.canonical_bytes().unwrap()).unwrap()).collect::<Vec<_>>()});
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(serde_json::to_string_pretty(&packet)?.as_bytes())?;
                file.sync_all()?;
                Ok(json!({"path":path}))
            }
            _ => Err(invalid(format!("Unknown command: {command}"))),
        }
    }
    /// Poll explicit collection work and archive it. `dispatch_due` is true
    /// only for the cadence scheduler or an explicit collection-tick command;
    /// the ordinary heartbeat drains an already-running explicit job without
    /// starting any other due source or auto-briefing the result.
    /// The planner's one ledger write: a `task.completed` check-in when a task
    /// or occurrence is completed, superseded by a 0 when it is reopened. The
    /// task file is already committed; a ledger fault is reported, never fatal.
    fn ledger_task_check_in(&mut self, task: &Task, done: bool, occ: Option<&OccurrenceId>) {
        let Some(ledger) = self.ledger.as_mut() else {
            return;
        };
        let occurrence = occ.map(|o| o.opaque_key().to_string());
        let mut extra = serde_json::Map::new();
        if let Some(key) = &occurrence {
            extra.insert("occurrence".into(), json!(key));
        }
        if let Some(goal) = &task.aim_id {
            extra.insert("goal_id".into(), json!(goal.to_string()));
        }
        let now = chrono::Local::now();
        let row = NewObservation {
            kind: "check_in".into(),
            observed_on: now.format("%Y-%m-%d").to_string(),
            observed_at: Some(now.format("%H:%M").to_string()),
            item_id: Some(task.meta.id.to_string()),
            category: task.category_id.as_ref().map(ToString::to_string),
            subject: "task.completed".into(),
            quantity: Some(if done { 1.0 } else { 0.0 }),
            unit: Some("done".into()),
            writer: "planner".into(),
            extra: (!extra.is_empty()).then_some(Value::Object(extra)),
            ..NewObservation::default()
        };
        let result = (|| -> soliton_kernel::Result<()> {
            if done {
                ledger.append_observation(&row)?;
                return Ok(());
            }
            let open = ledger
                .current_for_item(task.meta.id.as_str(), "task.completed")?
                .into_iter()
                .find(|(_, quantity, extra)| {
                    *quantity == 1.0
                        && extra
                            .as_ref()
                            .and_then(|e| e["occurrence"].as_str())
                            .map(str::to_owned)
                            == occurrence
                });
            if let Some((id, _, _)) = open {
                ledger.supersede(&id, &row)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Ledger check-in for {}: {error}", task.meta.id);
        }
    }

    /// The one backend call for a cross-module question:
    /// `{"name": "<question>", "hide_all": bool}`.
    fn ledger_question(&self, args: &Value) -> Result<Value> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))?;
        let question = Question::parse(s(args, "name"))?;
        let answer = ledger.ask(
            question,
            self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false),
        )?;
        Ok(
            json!({"name":answer.question.name(),"hide_all":answer.hide_all,"columns":answer.columns,"rows":answer.rows}),
        )
    }

    /// Hide-all marks: `{"scope": "item"|"category"|"goal"|"module", "key": "..."}`.
    /// A goal mark is expanded here to the goal's items.
    fn ledger_mark_private(&mut self, args: &Value) -> Result<Value> {
        let scope = MarkScope::parse(s(args, "scope"))?;
        let key = s(args, "key").to_owned();
        let items: Vec<ObjectId> = if scope == MarkScope::Goal {
            let goal: ObjectId = key.parse()?;
            if !matches!(
                self.snapshot.get(&goal).map(|g| &g.entity),
                Some(Entity::Aim(_))
            ) {
                return Err(invalid("No such goal"));
            }
            self.snapshot
                .entities()
                .filter_map(|e| match e {
                    Entity::Task(t) if t.aim_id.as_ref() == Some(&goal) => Some(t.meta.id.clone()),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let ledger = self
            .ledger
            .as_mut()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))?;
        if scope == MarkScope::Goal {
            ledger.mark_goal_private(&key.parse()?, &items)?;
        } else {
            ledger.mark_private(scope, &key)?;
        }
        Ok(json!({"scope":scope.as_str(),"key":key,"items":items.len()}))
    }

    /// `{"scope", "key"}`: lifts a persistent ledger mark. A goal's mark also
    /// marked each of its items, which a single lift cannot undo; refused.
    fn ledger_unmark_private(&mut self, args: &Value) -> Result<Value> {
        let scope = MarkScope::parse(s(args, "scope"))?;
        if scope == MarkScope::Goal {
            return Err(invalid("A goal's mark is lifted item by item"));
        }
        let key = s(args, "key").to_owned();
        self.ledger_mut()?.unmark_private(scope, &key)?;
        Ok(json!({"scope":scope.as_str(),"key":key}))
    }

    fn ledger_ref(&self) -> Result<&Ledger> {
        self.ledger
            .as_ref()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))
    }

    /// `{"subject", "quantity", "unit", "category"?, "expires_on"?, "location"?,
    /// "source"?: "typed"|"receipt", "purchase_line"?, "picture"?, "day"?}`. A
    /// picture is a file on his disk or in the workspace; the kernel keeps a copy.
    fn kitchen_stock_add(&mut self, args: &Value) -> Result<Value> {
        let spec = LotSpec {
            subject: s(args, "subject").to_owned(),
            category: args["category"].as_str().map(str::to_owned),
            quantity: number(args, "quantity")?,
            unit: s(args, "unit").to_owned(),
            expires_on: args["expires_on"].as_str().map(str::parse).transpose()?,
            location: args["location"].as_str().map(str::to_owned),
            source: LotSource::parse(args["source"].as_str().unwrap_or("typed"))?,
            purchase_line: optional_id(args, "purchase_line")?,
        };
        let day = habit_day(args)?;
        let picture = self.workspace_picture(args)?;
        let ledger = self.ledger_mut()?;
        let id = match picture {
            Some(picture) => ledger.add_stock_with_picture(&spec, &picture, &day)?,
            None => ledger.add_stock(&spec, &day)?,
        };
        Ok(lot_json(&ledger.lot(&id)?))
    }

    /// The `picture` argument, when given: a file on his disk chosen in the
    /// window (an absolute path), or one inside the workspace (relative to it;
    /// the finance intake is part of it). The kernel checks it is a picture
    /// and keeps its own copy, so the original is only read.
    fn workspace_picture(&self, args: &Value) -> Result<Option<PathBuf>> {
        let Some(picture) = args["picture"].as_str().filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        let root = self.repo.root().canonicalize()?;
        let given = Path::new(picture);
        let path = if given.is_absolute() {
            given.to_path_buf()
        } else {
            root.join(given)
        };
        match path.canonicalize() {
            Ok(found) if found.is_file() && (given.is_absolute() || found.starts_with(&root)) => {
                Ok(Some(found))
            }
            _ => Err(invalid("Choose a picture file")),
        }
    }

    /// `{"lot", "picture", "day"?}`: gives a lot its picture (the lot is superseded).
    fn kitchen_stock_picture(&mut self, args: &Value) -> Result<Value> {
        let lot: ObjectId = s(args, "lot").parse()?;
        let day = habit_day(args)?;
        let picture = self
            .workspace_picture(args)?
            .ok_or_else(|| invalid("Choose a picture"))?;
        let ledger = self.ledger_mut()?;
        let id = ledger.set_picture(&lot, &picture, &day)?;
        let mut row = lot_json(&ledger.lot(&id)?);
        row["supersedes"] = json!(lot.to_string());
        Ok(row)
    }

    /// `{"picture"}`: whether a file can be a lot's picture (a real png, jpg or
    /// webp, wherever it lies), writing nothing; the window asks before it saves.
    fn kitchen_picture_check(&self, args: &Value) -> Result<Value> {
        let picture = self
            .workspace_picture(args)?
            .ok_or_else(|| invalid("Choose a picture"))?;
        Ok(json!({"kind": check_picture(&picture)?}))
    }

    /// `{"hide_all"?}`: the food purchase lines not yet in stock.
    fn kitchen_from_purchases(&self, args: &Value) -> Result<Value> {
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let lines: Vec<Value> = self
            .ledger_ref()?
            .lots_from_purchases(hide_all)?
            .iter()
            .map(|l| {
                json!({"line":l.id.to_string(),"day":l.observed_on,"subject":l.subject,"category":l.category,
                    "amount":money(l.amount_minor),"amount_minor":l.amount_minor,"currency":l.currency,"source":l.source})
            })
            .collect();
        Ok(json!({"hide_all":hide_all,"lines":lines}))
    }

    /// `{"line", "quantity", "unit", "expires_on"?, "location"?, "day"?}`: one
    /// food purchase line becomes one lot, once.
    fn kitchen_lot_from_purchase(&mut self, args: &Value) -> Result<Value> {
        let line: ObjectId = s(args, "line").parse()?;
        let day = habit_day(args)?;
        let quantity = number(args, "quantity")?;
        let expires_on = args["expires_on"].as_str().map(str::parse).transpose()?;
        let ledger = self.ledger_mut()?;
        let id = ledger.lot_from_purchase(
            &line,
            quantity,
            s(args, "unit"),
            expires_on,
            args["location"].as_str(),
            &day,
        )?;
        Ok(lot_json(&ledger.lot(&id)?))
    }

    /// `{"day"?, "coverage_days"}`: what a draft would be given (nothing is called).
    fn kitchen_draft_input(&self, args: &Value) -> Result<Value> {
        let day: CivilDate = habit_day(args)?.parse()?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        Ok(self
            .ledger_ref()?
            .draft_input(day, coverage_days(args)?, hide_all)?)
    }

    /// `{"day"?, "coverage_days", "dishes", "rule"}`: writes a drafted plan,
    /// each dish inferred under its rule, for his review.
    fn kitchen_draft_accept(&mut self, args: &Value) -> Result<Value> {
        let day: CivilDate = habit_day(args)?.parse()?;
        let coverage = coverage_days(args)?;
        let dishes = crate::hearth_draft::dishes_from(&args["dishes"])?;
        let plan = self
            .ledger_mut()?
            .accept_draft(day, coverage, &dishes, s(args, "rule"))?;
        let mut row = plan_json(&plan, coverage);
        row["rule"] = json!(s(args, "rule").trim());
        Ok(row)
    }

    /// `{"day"?, "coverage_days"}`: drafts a plan with the local decision
    /// model and writes it as inferred. Off unless `localModel` and
    /// `kitchenDraft` are both on; while off nothing is read or started.
    fn kitchen_draft(&mut self, args: &Value) -> Result<Value> {
        let prefs = self.preferences()?;
        let settings = crate::hearth_draft::DraftSettings {
            local_model: self.local_model,
            kitchen_draft: prefs["kitchenDraft"] == true,
            serve_script: self.decision_serve.clone(),
        };
        if !(settings.local_model && settings.kitchen_draft) {
            return Err(invalid("Drafting is off"));
        }
        let input = self.kitchen_draft_input(args)?;
        let (dishes, role) = crate::hearth_draft::draft(&input, &settings)?;
        let day: CivilDate = habit_day(args)?.parse()?;
        let coverage = coverage_days(args)?;
        let rule = crate::hearth_draft::rule(&role);
        let plan = self
            .ledger_mut()?
            .accept_draft(day, coverage, &dishes, &rule)?;
        let mut row = plan_json(&plan, coverage);
        row["rule"] = json!(rule);
        Ok(row)
    }

    /// `{"lot", "amount"}` (use), `{"lot", "quantity"}` (adjust) or
    /// `{"lot", "expires_on"}` (expiry), each with `"day"?`. The lot is superseded.
    fn kitchen_stock_change(&mut self, args: &Value, change: &str) -> Result<Value> {
        let lot: ObjectId = s(args, "lot").parse()?;
        let day = habit_day(args)?;
        let ledger = self.ledger_mut()?;
        let id = match change {
            "use" => ledger.use_stock(&lot, number(args, "amount")?, &day)?,
            "adjust" => ledger.adjust_stock(&lot, number(args, "quantity")?, &day)?,
            _ => ledger.set_expiry(&lot, s(args, "expires_on").parse()?, &day)?,
        };
        let mut row = lot_json(&ledger.lot(&id)?);
        row["supersedes"] = json!(lot.to_string());
        Ok(row)
    }

    /// `{"hide_all"?}`: the lots with something left, soonest to go off first.
    fn kitchen_stock_list(&self, args: &Value) -> Result<Value> {
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let ledger = self.ledger_ref()?;
        let mut lots = Vec::new();
        for lot in ledger.stock(hide_all)? {
            let mut row = lot_json(&lot);
            // For the window to show the picture; never shown as text.
            row["picture"] = json!(ledger.picture_path(&lot)?);
            lots.push(row);
        }
        Ok(json!({"hide_all":hide_all,"lots":lots}))
    }

    /// `{"name", "count", "capacity"?, "day"?}`: adds a piece or supersedes its row.
    fn kitchen_equipment_set(&mut self, args: &Value) -> Result<Value> {
        let day = habit_day(args)?;
        let count = number(args, "count")?;
        let id = self.ledger_mut()?.set_equipment(
            s(args, "name"),
            count,
            args["capacity"].as_str(),
            &day,
        )?;
        Ok(json!({"id":id.to_string(),"name":s(args, "name").trim(),"count":count}))
    }

    /// `{"hide_all"?}`.
    fn kitchen_equipment_list(&self, args: &Value) -> Result<Value> {
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let pieces: Vec<Value> = self
            .ledger_ref()?
            .equipment(hide_all)?
            .iter()
            .map(|e| json!({"id":e.id.to_string(),"name":e.name,"count":e.count,"capacity":e.capacity}))
            .collect();
        Ok(json!({"hide_all":hide_all,"equipment":pieces}))
    }

    /// `{"subject", "quantity"?, "unit"?, "value"?, "day"?}`: one standing constraint.
    fn kitchen_profile_set(&mut self, args: &Value) -> Result<Value> {
        let day = habit_day(args)?;
        let id = self.ledger_mut()?.set_cook_profile(
            s(args, "subject"),
            args["quantity"].as_f64(),
            args["unit"].as_str(),
            args["value"].as_str(),
            &day,
        )?;
        Ok(json!({"id":id.to_string(),"subject":s(args, "subject").trim()}))
    }

    /// `{"hide_all"?}`.
    fn kitchen_profile_list(&self, args: &Value) -> Result<Value> {
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let entries: Vec<Value> = self
            .ledger_ref()?
            .cook_profile(hide_all)?
            .iter()
            .map(profile_json)
            .collect();
        Ok(json!({"hide_all":hide_all,"profile":entries}))
    }

    /// Q5: `{"days", "today"?, "hide_all"?}`.
    fn kitchen_goes_off(&self, args: &Value) -> Result<Value> {
        let today: CivilDate = habit_day(&json!({"day":args["today"]}))?.parse()?;
        let days = args["days"]
            .as_i64()
            .ok_or_else(|| invalid("A look-ahead needs a whole number of days"))?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let table = self.ledger_ref()?.goes_off_within(today, days, hide_all)?;
        Ok(
            json!({"today":today.to_string(),"days":days,"hide_all":hide_all,"columns":table.columns,"rows":table.rows}),
        )
    }

    /// `{"day"?, "horizon_days"?, "dishes": [{"name", "portions", "phase"?,
    /// "equipment"?: [names], "uses"?: [{"lot", "qty"}]}]}`.
    fn kitchen_batch_plan(&mut self, args: &Value) -> Result<Value> {
        let day: CivilDate = habit_day(args)?.parse()?;
        let horizon = args["horizon_days"].as_i64().unwrap_or(3);
        let mut dishes = Vec::new();
        for dish in args["dishes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let mut uses = Vec::new();
            for used in dish["uses"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                uses.push((s(used, "lot").parse()?, number(used, "qty")?));
            }
            dishes.push(DishSpec {
                name: s(dish, "name").to_owned(),
                portions: number(dish, "portions")?,
                phase: u32::try_from(dish["phase"].as_u64().unwrap_or(0))
                    .map_err(|_| invalid("A phase is a small whole number"))?,
                equipment: strings(&dish["equipment"]),
                uses,
            });
        }
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let plan = if args.get("coverage_days").is_some_and(|c| !c.is_null()) {
            let coverage = coverage_days(args)?;
            self.ledger_mut()?
                .write_batch_plan_covering(day, horizon, coverage, &dishes, hide_all)?
        } else {
            self.ledger_mut()?
                .write_batch_plan_for_screen(day, horizon, &dishes, hide_all)?
        };
        Ok(plan_json(&plan, horizon))
    }

    /// `{"hide_all"?}`: the finance intake folder's files, each waiting or in.
    fn finance_intake(&self, args: &Value) -> Result<Value> {
        let (hide_all, hidden) = self.finance_hidden(args)?;
        let files: Vec<Value> = if hidden {
            Vec::new()
        } else {
            self.ledger_ref()?
                .list_intake()?
                .iter()
                .map(|f| {
                    json!({"name":f.name,"bytes":f.bytes,"kind":f.kind.map(|k| k.as_str()),"day":f.day,
                        "state":if f.document.is_some() { "in" } else { "waiting" },
                        "document":f.document.as_ref().map(ToString::to_string)})
                })
                .collect()
        };
        Ok(json!({"hide_all":hide_all,"files":files}))
    }

    /// Hide-all, and whether finance reads come back empty: hide-all with the
    /// finance module marked private in Settings or in the ledger.
    fn finance_hidden(&self, args: &Value) -> Result<(bool, bool)> {
        let privacy = self.privacy()?;
        let hide_all = privacy.on || args["hide_all"].as_bool().unwrap_or(false);
        let hidden = hide_all
            && (privacy.modules.contains("finance")
                || self
                    .ledger_ref()?
                    .is_marked_private(MarkScope::Module, "finance")?);
        Ok((hide_all, hidden))
    }

    /// `{"name", "kind": "csv"|"pdf"|"jpg"|"png"|"eml", "day"?}`: a file in
    /// the intake folder comes in as a document.
    fn finance_document_register(&mut self, args: &Value) -> Result<Value> {
        let name = s(args, "name").trim();
        if name.is_empty() || name.contains('/') || name.starts_with('.') {
            return Err(invalid("Choose a file from the intake folder"));
        }
        let kind = DocumentKind::parse(s(args, "kind"))?;
        let day: CivilDate = habit_day(args)?.parse()?;
        let path = intake_dir(self.repo.root()).join(name);
        let id = self.ledger_ref()?.register_document(&path, kind, day)?;
        Ok(json!({"id":id.to_string(),"name":name,"kind":kind.as_str(),"day":day.to_string()}))
    }

    /// `{"document", "bank"?, "currency"?}`: reads a CSV document: a statement
    /// through its bank's mapping when `bank` is given, else the fixed purchase
    /// header (with `currency`) or a pay stub's field,value rows, by its header.
    fn finance_import(&mut self, args: &Value) -> Result<Value> {
        let document: ObjectId = s(args, "document").parse()?;
        let intake = intake_dir(self.repo.root());
        let ledger = self.ledger_mut()?;
        let file = ledger
            .list_intake()?
            .into_iter()
            .find(|f| f.document.as_ref() == Some(&document))
            .ok_or_else(|| invalid("That document is no longer in the intake folder"))?;
        if file.kind != Some(DocumentKind::Csv) {
            return Err(invalid("A receipt's lines are entered against its picture"));
        }
        let bank = s(args, "bank").trim();
        let (kind, outcome) = if !bank.is_empty() {
            (
                "statement",
                ledger.import_statement_document(&document, bank)?,
            )
        } else {
            let header = csv_header(&intake.join(&file.name))?;
            if header.first().is_some_and(|c| c == "field") {
                let id = ledger.import_pay_stub_csv(&document)?;
                (
                    "pay_stub",
                    soliton_kernel::ledger::finance::ImportOutcome {
                        added: vec![id],
                        ..Default::default()
                    },
                )
            } else if soliton_kernel::ledger::finance::FIXED_HEADER
                .iter()
                .all(|c| header.iter().any(|h| h == c))
            {
                let currency = s(args, "currency");
                if currency.trim().is_empty() {
                    return Err(invalid("Choose the currency of these purchases"));
                }
                ("fixed_csv", ledger.import_fixed_csv(&document, currency)?)
            } else {
                return Err(invalid("Choose the bank this statement is from"));
            }
        };
        Ok(json!({
            "document": document.to_string(),
            "kind": kind,
            "added": outcome.added.len(),
            "duplicates": outcome.duplicates,
            "credits": outcome.credits,
            "period": outcome.period,
        }))
    }

    /// `{"document", "lines": [{"merchant", "day", "item", "quantity", "unit",
    /// "amount", "currency", "category"}]}`: a receipt's lines, typed once,
    /// against its picture, PDF or saved e-mail.
    fn finance_receipt_lines(&mut self, args: &Value) -> Result<Value> {
        let document: ObjectId = s(args, "document").parse()?;
        let mut lines = Vec::new();
        for line in args["lines"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            lines.push(ReceiptLine {
                merchant: s(line, "merchant").to_owned(),
                day: s(line, "day").parse()?,
                item: s(line, "item").to_owned(),
                quantity: number(line, "quantity")?,
                unit: s(line, "unit").to_owned(),
                amount_minor: minor_amount(&line["amount"])?,
                currency: s(line, "currency").to_owned(),
                category: s(line, "category").to_owned(),
            });
        }
        let ids = self.ledger_mut()?.record_receipt_lines(&document, &lines)?;
        Ok(
            json!({"document":document.to_string(),"lines":ids.iter().map(ToString::to_string).collect::<Vec<_>>()}),
        )
    }

    /// `{"day", "merchant", "amount", "currency", "category"}`: cash spent, by hand.
    fn finance_cash(&mut self, args: &Value) -> Result<Value> {
        let day: CivilDate = habit_day(args)?.parse()?;
        let amount = minor_amount(&args["amount"])?;
        let id = self.ledger_mut()?.record_cash(
            day,
            s(args, "merchant"),
            amount,
            s(args, "currency"),
            s(args, "category"),
        )?;
        Ok(
            json!({"id":id.to_string(),"day":day.to_string(),"merchant":s(args, "merchant").trim(),
            "amount":money(amount),"currency":s(args, "currency").trim().to_ascii_uppercase(),
            "category":category_for(s(args, "category"))}),
        )
    }

    /// `{"hide_all"?}`: the documents in, each with its day and line count.
    fn finance_documents(&self, args: &Value) -> Result<Value> {
        let (hide_all, hidden) = self.finance_hidden(args)?;
        if hidden {
            return Ok(json!({"hide_all":hide_all,"documents":[]}));
        }
        let ledger = self.ledger_ref()?;
        let mut sources: Vec<String> = ledger
            .purchase_lines(&LineFilter::default(), hide_all)?
            .into_iter()
            .filter_map(|l| l.source)
            .collect();
        sources.extend(
            ledger
                .income_lines(hide_all)?
                .into_iter()
                .filter_map(|l| l.source),
        );
        let documents: Vec<Value> = ledger
            .list_intake()?
            .into_iter()
            .filter_map(|f| {
                let id = f.document?;
                let prefix = format!("document:{id}#");
                let lines = sources.iter().filter(|s| s.starts_with(&prefix)).count();
                Some(json!({"id":id.to_string(),"name":f.name,"kind":f.kind.map(|k| k.as_str()),"day":f.day,"lines":lines}))
            })
            .collect();
        Ok(json!({"hide_all":hide_all,"documents":documents}))
    }

    /// The spending categories by key, each with its word.
    fn finance_categories(&self, _args: &Value) -> Result<Value> {
        let categories: Vec<Value> = CATEGORIES
            .iter()
            .map(|(key, word)| json!({"key":key,"word":word}))
            .collect();
        Ok(json!({"categories":categories}))
    }

    /// `{"day"?, "time"?: "HH:MM"}`: today's few items by his objective, each
    /// with its why and sources, and how many were left out. Reads `nowBound`,
    /// `nowOrder`, `nowMorningUntil`, `nowEveningFrom`; private items stay out.
    fn now_view(&self, args: &Value) -> Result<Value> {
        let prefs = self.preferences()?;
        let privacy = self.privacy()?;
        let mut settings = NowSettings::default();
        if let Some(bound) = prefs["nowBound"].as_u64().filter(|b| (1..=50).contains(b)) {
            settings.bound = usize::try_from(bound).unwrap_or(settings.bound);
        }
        let order: Vec<NowKind> = strings(&prefs["nowOrder"])
            .iter()
            .filter_map(|k| match k.as_str() {
                "task" => Some(NowKind::Task),
                "habit" => Some(NowKind::Habit),
                "lot" => Some(NowKind::Lot),
                _ => None,
            })
            .collect();
        if !order.is_empty() {
            settings.order = order;
        }
        if let Some(t) = prefs["nowMorningUntil"]
            .as_str()
            .and_then(|t| t.parse().ok())
        {
            settings.morning_until = t;
        }
        if let Some(t) = prefs["nowEveningFrom"]
            .as_str()
            .and_then(|t| t.parse().ok())
        {
            settings.evening_from = t;
        }
        settings.hide_all = privacy.on;
        let bound = settings.bound;
        // Rank everything, then take out what Settings hides, then bound.
        settings.bound = usize::MAX;
        let day: CivilDate = habit_day(args)?.parse()?;
        let time: ClockTime = match args["time"].as_str() {
            Some(t) => t.parse()?,
            None => chrono::Local::now().format("%H:%M").to_string().parse()?,
        };
        let list =
            soliton_kernel::now::items(&self.snapshot, self.ledger.as_ref(), day, time, &settings)?;
        let module_of = |kind: NowKind| match kind {
            NowKind::Task => "planner",
            NowKind::Habit => "habits",
            NowKind::Lot => "kitchen",
        };
        let shown: Vec<_> = list
            .items
            .into_iter()
            .filter(|item| {
                !(privacy.on
                    && (privacy.modules.contains(module_of(item.kind))
                        || privacy.ids.contains(&item.id)
                        || item.sources.iter().any(|s| privacy.ids.contains(&s.id))))
            })
            .collect();
        let left_out = list.left_out + shown.len().saturating_sub(bound);
        let items: Vec<Value> = shown
            .into_iter()
            .take(bound)
            .enumerate()
            .map(|(index, mut item)| {
                item.rank = index + 1;
                item.to_json()
            })
            .collect();
        Ok(
            json!({"day":day.to_string(),"time":time.to_string(),"hide_all":privacy.on,"items":items,"left_out":left_out}),
        )
    }

    /// `{"day"?, "hide_all"?}`: the current batch plans by day, each with its
    /// dishes, phases, equipment and uses.
    fn kitchen_plans(&self, args: &Value) -> Result<Value> {
        let day: Option<CivilDate> = args["day"].as_str().map(str::parse).transpose()?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let plans: Vec<Value> = self
            .ledger_ref()?
            .batch_plans(day, hide_all)?
            .iter()
            .map(|plan| {
                json!({
                    "plan_id": plan.plan_id,
                    "day": plan.day,
                    "portions": plan.portions,
                    "coverage_days": plan.coverage_days,
                    "dishes": plan.dishes.iter().map(|d| json!({
                        "id": d.id.to_string(),
                        "name": d.name,
                        "portions": d.portions,
                        "phase": d.phase,
                        "equipment": d.equipment,
                        "uses": d.uses.iter().map(|(lot, qty)| json!({"lot":lot.to_string(),"qty":qty})).collect::<Vec<_>>(),
                        "meal": d.cooked.as_ref().map(ToString::to_string),
                        "standing": d.standing,
                        "derived_by": d.derived_by,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"hide_all":hide_all,"plans":plans}))
    }

    /// `{"plan", "dish", "day"?}`: cooks one dish of a plan (`dish` is its row
    /// id or name): writes the meal, takes the dish's uses from stock.
    fn kitchen_cook(&mut self, args: &Value) -> Result<Value> {
        let day: CivilDate = habit_day(args)?.parse()?;
        let cooked = self
            .ledger_mut()?
            .cook_dish(s(args, "plan"), s(args, "dish"), day)?;
        Ok(json!({
            "meal": cooked.meal.to_string(),
            "dish": cooked.dish.to_string(),
            "day": day.to_string(),
            "lots": cooked.lots.iter().map(|(old, new)| json!({"id":new.to_string(),"supersedes":old.to_string()})).collect::<Vec<_>>(),
        }))
    }

    /// `{"day"?, "hide_all"?}`: the current meals, oldest first.
    fn kitchen_meals(&self, args: &Value) -> Result<Value> {
        let day: Option<CivilDate> = args["day"].as_str().map(str::parse).transpose()?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let meals: Vec<Value> = self
            .ledger_ref()?
            .meals(day, hide_all)?
            .iter()
            .map(|m| json!({"id":m.id.to_string(),"day":m.day,"name":m.name,"portions":m.portions,"category":m.category,"plan_id":m.plan_id}))
            .collect();
        Ok(json!({"hide_all":hide_all,"meals":meals}))
    }

    /// `{"subject", "day"?}`: removes a cook-profile entry (supersede to
    /// nothing); the entry goes to the Trash first.
    fn kitchen_profile_remove(&mut self, args: &Value) -> Result<Value> {
        let day = habit_day(args)?;
        let subject = s(args, "subject").trim();
        let entry = self
            .ledger_ref()?
            .cook_profile(false)?
            .into_iter()
            .find(|e| e.subject == subject)
            .ok_or_else(|| invalid(format!("The cook profile has no entry {subject:?}")))?;
        let kept = profile_bytes(&entry);
        let original = entry.id.to_string();
        self.records.put(
            "kitchen-profile",
            &original,
            &entry.subject,
            &kept,
            Utc::now().timestamp(),
        )?;
        match self.ledger_mut()?.remove_cook_profile(subject, &day) {
            Ok(id) => Ok(
                json!({"id":id.to_string(),"subject":subject,"removed":true,"trash":{"kind":"kitchen-profile","id":original}}),
            ),
            Err(error) => {
                self.records.restored("kitchen-profile", &original)?;
                Err(error.into())
            }
        }
    }

    /// Moves a kitchen profile entry or a collection source to the Trash:
    /// its bytes are kept, then it leaves its own store.
    fn record_trash(&mut self, kind: &str, id: &str) -> Result<Value> {
        if kind == "kitchen-profile" {
            let entry = self
                .ledger_ref()?
                .cook_profile(false)?
                .into_iter()
                .find(|e| e.id.as_str() == id)
                .ok_or_else(|| invalid("This item is no longer available."))?;
            return self.kitchen_profile_remove(&json!({"subject":entry.subject}));
        }
        if self.collection.is_none() {
            self.collection = Some(crate::collection::CollectionStore::open(
                &self.repo.root().join("collection"),
            )?);
        }
        let (bytes, title) = self.collection.as_ref().unwrap().source_bytes(id)?;
        self.records
            .put(kind, id, &title, &bytes, Utc::now().timestamp())?;
        let removed = self
            .collection
            .as_mut()
            .unwrap()
            .command("collection-remove", &json!({"id":id}));
        match removed {
            Ok(mut value) => {
                value["trash"] = json!({"kind":kind,"id":id});
                Ok(value)
            }
            Err(error) => {
                self.records.restored(kind, id)?;
                Err(error)
            }
        }
    }

    /// Puts a record from the Trash, or from Recovery after its window, back
    /// into its own store exactly as it was.
    fn record_restore(&mut self, kind: &str, id: &str) -> Result<Value> {
        let bytes = self.records.bytes(kind, id)?;
        let restored = if kind == "kitchen-profile" {
            let row = self.ledger_mut()?.restore_cook_profile(&id.parse()?)?;
            json!({"id":row.to_string()})
        } else {
            if self.collection.is_none() {
                self.collection = Some(crate::collection::CollectionStore::open(
                    &self.repo.root().join("collection"),
                )?);
            }
            self.collection
                .as_mut()
                .unwrap()
                .restore_source(id, &bytes)?
        };
        self.records.restored(kind, id)?;
        Ok(json!({"kind":kind,"id":id,"restored":restored,"state":self.state()?}))
    }

    /// Records past their Trash window, as Recovery lists them.
    fn expired_record_items(&self, filter: Option<&str>) -> Result<Vec<Value>> {
        let privacy = self.privacy()?;
        let mut items = Vec::new();
        for entry in self.records.expired() {
            if hides_record(&privacy, &entry.kind, &entry.id) {
                continue;
            }
            let path = self.record_path(&entry.kind, &entry.id);
            if filter.is_some_and(|filter| Some(filter) != path.to_str()) {
                continue;
            }
            let bytes = self.records.bytes(&entry.kind, &entry.id)?;
            items.push(json!({"id":format!("{EXPIRED_RECORD}{}-{}", entry.kind, entry.id),
                "kind":"checkpoint","path":path,"hash":ContentHash::digest(&bytes).to_string(),
                "bytes":bytes.len(),"reason":"expired-trash","title":self.record_title(entry),
                "captured_at":chrono::DateTime::from_timestamp(entry.deleted_at, 0).map(|t| t.to_rfc3339())}));
        }
        Ok(items)
    }

    /// A Trash entry's title; a source removed before titles were kept takes
    /// its name from the bytes the Trash holds.
    fn record_title(&self, entry: &soliton_kernel::trash::RecordEntry) -> String {
        if !entry.title.is_empty() || entry.kind != "collection-source" {
            return entry.title.clone();
        }
        self.records
            .bytes(&entry.kind, &entry.id)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .map(|source| crate::collection::source_name(&source))
            .unwrap_or_default()
    }

    fn record_path(&self, kind: &str, id: &str) -> std::path::PathBuf {
        self.repo
            .root()
            .join("trash/records")
            .join(kind)
            .join(format!("{id}.bin"))
    }

    /// `recovery-preview` and `recovery-restore` for a record past its window.
    fn expired_record(&mut self, command: &str, args: &Value) -> Result<Value> {
        let rest = &s(args, "id")[EXPIRED_RECORD.len()..];
        let (kind, id) = soliton_kernel::trash::RECORD_KINDS
            .iter()
            .find_map(|kind| {
                rest.strip_prefix(kind)
                    .and_then(|id| id.strip_prefix('-'))
                    .map(|id| (*kind, id))
            })
            .ok_or_else(|| invalid("Invalid expired Trash id"))?;
        if self
            .records
            .expired()
            .iter()
            .all(|e| e.kind != kind || e.id != id)
        {
            return Err(invalid("Expired Trash item is unavailable"));
        }
        let bytes = self.records.bytes(kind, id)?;
        let token = ContentHash::digest(&bytes).to_string();
        let path = self.record_path(kind, id);
        if command == "recovery-preview" {
            return Ok(
                json!({"id":args["id"],"op_id":format!("restore-{token}"),"path":path,
                "current_hash":token,"checkpoint_hash":token,"current_state":"missing",
                "missing":true,"conflict":false,"affected":[],
                "diff":{"kind":"text","before":String::from_utf8_lossy(&bytes),"current":""},
                "local_only":true}),
            );
        }
        if args["expected_current_hash"].as_str() != Some(&token) {
            return Err(invalid(
                "Recovery conflict: preview the expired Trash item again",
            ));
        }
        self.record_restore(kind, id)?;
        Ok(
            json!({"id":args["id"],"status":"applied","path":path,"restored_hash":token,"restored_count":1}),
        )
    }

    /// `{"lot", "location", "day"?}`: where a lot is kept now; the lot is superseded.
    fn kitchen_lot_move(&mut self, args: &Value) -> Result<Value> {
        let lot: ObjectId = s(args, "lot").parse()?;
        let day = habit_day(args)?;
        let ledger = self.ledger_mut()?;
        let id = ledger.move_lot(&lot, s(args, "location"), &day)?;
        let mut row = lot_json(&ledger.lot(&id)?);
        row["supersedes"] = json!(lot.to_string());
        Ok(row)
    }

    /// `{"path"}`: the column names on a chosen statement's first line. Only
    /// that line is read; nothing is written.
    fn finance_statement_header(&self, args: &Value) -> Result<Value> {
        let columns = read_statement_header(Path::new(s(args, "path")))?;
        Ok(json!({"columns":columns}))
    }

    /// `{"hide_all"?}`: the banks set up and their mappings. Empty while
    /// hide-all is on and finance is marked private (in the ledger or in Settings).
    fn finance_banks(&self, args: &Value) -> Result<Value> {
        let privacy = self.privacy()?;
        let hide_all = privacy.on || args["hide_all"].as_bool().unwrap_or(false);
        let ledger = self.ledger_ref()?;
        let hidden = hide_all
            && (privacy.modules.contains("finance")
                || ledger.is_marked_private(MarkScope::Module, "finance")?);
        let banks = if hidden {
            Vec::new()
        } else {
            ledger.statement_mappings()?
        };
        Ok(json!({"hide_all":hide_all,"banks":banks}))
    }

    /// `{"week"?, "category"?, "hide_all"?}`: the current purchase lines, by
    /// week then day; `week` is any day of the ISO week.
    fn finance_lines(&self, args: &Value) -> Result<Value> {
        let filter = LineFilter {
            week: args["week"].as_str().map(str::parse).transpose()?,
            category: args["category"].as_str().map(str::to_owned),
        };
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let ledger = self.ledger_ref()?;
        let stocked = ledger.stocked_purchases(hide_all)?;
        let lines: Vec<Value> = ledger
            .purchase_lines(&filter, hide_all)?
            .iter()
            .map(|l| {
                json!({
                    "id": l.id.to_string(),
                    "date": l.observed_on,
                    "week": l.week,
                    "merchant": l.merchant,
                    "amount": format!("{}{}.{:02}", if l.amount_minor < 0 { "-" } else { "" }, l.amount_minor.abs() / 100, l.amount_minor.abs() % 100),
                    "amount_minor": l.amount_minor,
                    "currency": l.currency,
                    "category": l.category,
                    "category_word": l.category_word,
                    "item": l.item,
                    "in_stock": stocked.contains_key(l.id.as_str()),
                    "source": l.source,
                })
            })
            .collect();
        Ok(json!({"hide_all":hide_all,"lines":lines}))
    }

    /// `{"hide_all"?}`: the current income lines by day, each with its pay
    /// period; nothing while finance is private and hide-all is on.
    fn finance_income(&self, args: &Value) -> Result<Value> {
        let (hide_all, hidden) = self.finance_hidden(args)?;
        if hidden {
            return Ok(json!({"hide_all":hide_all,"lines":[]}));
        }
        let money = |minor: i64| {
            format!(
                "{}{}.{:02}",
                if minor < 0 { "-" } else { "" },
                minor.abs() / 100,
                minor.abs() % 100
            )
        };
        let lines: Vec<Value> = self
            .ledger_ref()?
            .income_lines(hide_all)?
            .iter()
            .map(|l| {
                json!({
                    "id": l.id.to_string(),
                    "day": l.observed_on,
                    "employer": l.employer,
                    "net": money(l.net_minor),
                    "gross": money(l.gross_minor),
                    "currency": l.currency,
                    "period_start": l.period_from,
                    "period_end": l.period_to,
                    "source": l.source,
                })
            })
            .collect();
        Ok(json!({"hide_all":hide_all,"lines":lines}))
    }

    /// `{"line", "category"}`: supersedes the line with the new category.
    fn finance_recategorise(&mut self, args: &Value) -> Result<Value> {
        let line: ObjectId = s(args, "line").parse()?;
        let id = self
            .ledger_mut()?
            .recategorise_line(&line, s(args, "category"))?;
        Ok(
            json!({"id":id.to_string(),"supersedes":line.to_string(),"category":s(args, "category").trim()}),
        )
    }

    /// The bank's column mapping, set once: `{"bank", "date", "date_format":
    /// "YYYY-MM-DD"|"DD/MM/YYYY"|"MM/DD/YYYY", "description", "amount", "currency",
    /// "category"?, "debit_negative", "categories"?: {bank name: ledger category}}`.
    fn finance_statement_mapping(&mut self, args: &Value) -> Result<Value> {
        let mapping: StatementMapping = serde_json::from_value(args.clone())
            .map_err(|error| invalid(format!("Statement mapping: {error}")))?;
        self.ledger_ref()?.set_statement_mapping(&mapping)?;
        Ok(json!({"bank":mapping.bank.trim()}))
    }

    /// `{"bank", "document"}`: reads one statement that came in through the
    /// intake folder into purchase lines. A file chosen elsewhere is refused.
    fn finance_statement_import(&mut self, args: &Value) -> Result<Value> {
        if args.get("path").is_some() {
            return Err(invalid("A statement comes in through the intake folder"));
        }
        let document: ObjectId = s(args, "document").parse()?;
        let outcome = self
            .ledger_mut()?
            .import_statement_document(&document, s(args, "bank"))?;
        Ok(json!({
            "bank": s(args, "bank"),
            "period": outcome.period,
            "added": outcome.added.len(),
            "duplicates": outcome.duplicates,
            "credits": outcome.credits,
        }))
    }

    fn ledger_mut(&mut self) -> Result<&mut Ledger> {
        self.ledger
            .as_mut()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))
    }

    /// `{"id"?, "title", "direction": "form"|"break", "baseline", "bundle"?: "morning"|"evening"}`.
    /// Without an id a habit is defined; with one its definition is superseded.
    fn habit_define(&mut self, args: &Value) -> Result<Value> {
        let id = optional_id(args, "id")?;
        let spec = HabitSpec {
            title: s(args, "title").to_owned(),
            direction: Direction::parse(s(args, "direction"))?,
            baseline: args["baseline"]
                .as_f64()
                .ok_or_else(|| invalid("A habit needs a numeric baseline"))?,
            bundle: args["bundle"].as_str().map(Bundle::parse).transpose()?,
        };
        let day = habit_day(args)?;
        let habit = self.ledger_mut()?.define_habit(id.as_ref(), &spec, &day)?;
        Ok(habit_json(&habit))
    }

    /// `{"hide_all"?}`: the current habits in bundle order, and each bundle's order.
    fn habit_list(&self, args: &Value) -> Result<Value> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let day = habit_day(args)?;
        let mut habits = Vec::new();
        for habit in ledger.habits(hide_all)? {
            let mut row = habit_json(&habit);
            row["hierarchy_status"] = json!(self.habit_hierarchy_status(&habit.hierarchy));
            row["private"] = json!(ledger.is_marked_private(MarkScope::Item, habit.id.as_str())?);
            row["check_in"] = json!(
                ledger
                    .day_check_in(&habit.id, &day, hide_all)?
                    .map(|id| id.to_string())
            );
            row["restorable"] = json!(
                ledger
                    .day_restorable(&habit.id, &day, hide_all)?
                    .map(|id| id.to_string())
            );
            habits.push(row);
        }
        let shown: BTreeSet<String> = habits
            .iter()
            .filter_map(|h| h["id"].as_str().map(str::to_owned))
            .collect();
        let mut bundles = serde_json::Map::new();
        for bundle in [Bundle::Morning, Bundle::Evening] {
            let order: Vec<String> = ledger
                .bundle_order(bundle)?
                .iter()
                .map(ToString::to_string)
                .filter(|id| shown.contains(id))
                .collect();
            bundles.insert(bundle.as_str().into(), json!(order));
        }
        Ok(json!({"hide_all":hide_all,"day":day,"habits":habits,"bundles":bundles}))
    }

    /// A saved binding stays intact when a parent is removed or moved; callers
    /// can repair it explicitly rather than receiving a fabricated replacement.
    fn habit_hierarchy_status(&self, links: &Value) -> &'static str {
        let category = links["category_id"].as_str();
        let goal = links["goal_id"].as_str();
        let tasks = links["task_ids"].as_array();
        if category.is_none() && goal.is_none() && tasks.is_none_or(Vec::is_empty) {
            return "unlinked";
        }
        if let Some(id) = category {
            match self.get(id) {
                Ok(r) if matches!(&r.entity, Entity::Category(_)) && !self.hidden(&r.entity) => {}
                _ => return "removed-parent",
            }
        }
        if let Some(id) = goal {
            match self.get(id) {
                Ok(r) if !self.hidden(&r.entity) => match &r.entity {
                    Entity::Aim(a) if a.category_id.as_ref().map(ObjectId::as_str) == category => {}
                    Entity::Aim(_) => return "moved-parent",
                    _ => return "removed-parent",
                },
                _ => return "removed-parent",
            }
        }
        for id in tasks.into_iter().flatten().filter_map(Value::as_str) {
            match self.get(id) {
                Ok(r) if !self.hidden(&r.entity) => match &r.entity {
                    Entity::Task(t)
                        if t.aim_id.as_ref().map(ObjectId::as_str) == goal
                            && Some(self.effective_category(&r.entity).as_str())
                                .filter(|s| !s.is_empty())
                                == category => {}
                    Entity::Task(_) => return "moved-parent",
                    _ => return "removed-parent",
                },
                _ => return "removed-parent",
            }
        }
        "linked"
    }

    /// `{id: habit, category_id: id|null, goal_id: id|null, task_ids: [ids], day?}`.
    /// Actions retain caller order. An empty binding explicitly unlinks a habit.
    fn habit_link(&mut self, args: &Value) -> Result<Value> {
        let id: ObjectId = s(args, "id").parse()?;
        for field in ["category_id", "goal_id"] {
            if !args[field].is_null() && !args[field].is_string() {
                return Err(invalid(format!("{field} must be an id or null")));
            }
        }
        let category = optional_id(args, "category_id")?;
        let goal = optional_id(args, "goal_id")?;
        let tasks = args["task_ids"]
            .as_array()
            .ok_or_else(|| invalid("task_ids must be an ordered list"))?;
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        for task in tasks {
            let id: ObjectId = task
                .as_str()
                .ok_or_else(|| invalid("task_ids must contain ids"))?
                .parse()?;
            if !seen.insert(id.clone()) {
                return Err(invalid("task_ids must name each action once"));
            }
            ids.push(id.to_string());
        }
        let links = json!({"category_id":category.map(|id| id.to_string()), "goal_id":goal.map(|id| id.to_string()), "task_ids":ids});
        let status = self.habit_hierarchy_status(&links);
        if status != "linked" && status != "unlinked" {
            return Err(invalid(format!(
                "Habit hierarchy is {status}; select existing matching parents and tasks"
            )));
        }
        let day = habit_day(args)?;
        let habit = self.ledger_mut()?.bind_habit(&id, &links, &day)?;
        let mut result = habit_json(&habit);
        result["hierarchy_status"] = json!(status);
        Ok(result)
    }

    /// `{id: habit, day?, timezone?}`. Only actual task events enter the calendar;
    /// unscheduled one-off actions are returned here with on_timeline=false.
    fn habit_actions(&self, args: &Value) -> Result<Value> {
        let id: ObjectId = s(args, "id").parse()?;
        let ledger = self.ledger_ref()?;
        let privacy = self.privacy()?;
        let hide_all = privacy.on || args["hide_all"].as_bool().unwrap_or(false);
        let habit = ledger
            .habits(hide_all)?
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| invalid("Habit is unavailable"))?;
        let day = habit_day(args)?;
        let zone = if s(args, "timezone").is_empty() {
            "America/Toronto"
        } else {
            s(args, "timezone")
        };
        let status = self.habit_hierarchy_status(&habit.hierarchy);
        let calendar = self.calendar(&day, &day, zone)?;
        let mut actions = Vec::new();
        if let Some(ids) = habit.hierarchy["task_ids"].as_array() {
            for id in ids.iter().filter_map(Value::as_str) {
                let Ok(record) = self.get(id) else { continue };
                if self.hidden(&record.entity) || privacy.hides_planner(id) {
                    continue;
                }
                for event in calendar
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["id"] == id && v["event_kind"] != "due")
                {
                    let mut event = event.clone();
                    event["on_timeline"] = json!(true);
                    event["habit_ids"] = json!([habit.id.to_string()]);
                    actions.push(event);
                }
                if let Entity::Task(t) = &record.entity
                    && t.recurrence.is_none()
                    && t.scheduled_for.is_none()
                {
                    let mut action = self.record(&record.entity);
                    action["event_key"] = json!(id);
                    action["on_timeline"] = json!(false);
                    actions.push(action);
                }
            }
        }
        Ok(
            json!({"id":habit.id.to_string(), "day":day, "hierarchy":habit.hierarchy, "hierarchy_status":status, "actions":actions}),
        )
    }

    /// `{id: habit, task_id, day?, occurrence?, occurrence_date?, done: bool, hash}`.
    /// Setting a desired state makes retries safe. Completion belongs to the
    /// task occurrence, separate from the habit's daily aggregate check-in.
    fn habit_action_set(&mut self, args: &Value) -> Result<Value> {
        let actions = self.habit_actions(args)?;
        if actions["hierarchy_status"] != "linked" {
            return Err(invalid(
                "Repair the habit hierarchy before changing an action",
            ));
        }
        let done = args["done"]
            .as_bool()
            .ok_or_else(|| invalid("done must be true or false"))?;
        let task_id = s(args, "task_id");
        let action = actions["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| {
                a["id"] == task_id
                    && s(a, "occurrence") == s(args, "occurrence")
                    && (s(a, "occurrence").is_empty()
                        || a["occurrence_date"] == args["occurrence_date"])
            })
            .ok_or_else(|| invalid("Task occurrence is unavailable; reload habit actions"))?;
        if (action["status"] == "completed") == done {
            return Ok(actions);
        }
        let mut command = args.clone();
        command["id"] = json!(task_id);
        if s(action, "occurrence").is_empty() {
            self.command("complete", &command)?;
        } else {
            command["status"] = json!(if done { "completed" } else { "todo" });
            self.command("occurrence", &command)?;
        }
        self.habit_actions(args)
    }

    /// `{"bundle": "morning"|"evening", "order": [habit ids]}`.
    fn habit_bundle_order(&mut self, args: &Value) -> Result<Value> {
        let bundle = Bundle::parse(s(args, "bundle"))?;
        let order = strings(&args["order"])
            .iter()
            .map(|id| id.parse())
            .collect::<soliton_kernel::Result<Vec<ObjectId>>>()?;
        let day = habit_day(args)?;
        self.ledger_mut()?.set_bundle_order(bundle, &order, &day)?;
        Ok(json!({"bundle":bundle.as_str(),"order":strings(&args["order"])}))
    }

    /// `{"id": habit, "day"?, "done": bool}` for a habit to form,
    /// `{"id": habit, "day"?, "count": n}` for one to break.
    fn habit_check_in(&mut self, args: &Value) -> Result<Value> {
        let habit: ObjectId = s(args, "id").parse()?;
        let value = check_in_value(args)?;
        let day = habit_day(args)?;
        let row = self.ledger_mut()?.record_check_in(&habit, &day, value)?;
        Ok(json!({"check_in":row.to_string(),"habit":habit.to_string(),"day":day}))
    }

    /// `{"check_in": obs id, "done"|"count"}`: supersedes a check-in.
    fn habit_correct(&mut self, args: &Value) -> Result<Value> {
        let old: ObjectId = s(args, "check_in").parse()?;
        let value = check_in_value(args)?;
        let row = self.ledger_mut()?.correct_check_in(&old, value)?;
        Ok(json!({"check_in":row.to_string(),"supersedes":old.to_string()}))
    }

    /// `{"check_in": obs id}`: takes a check-in back, so its day has no entry.
    fn habit_retract(&mut self, args: &Value) -> Result<Value> {
        let old: ObjectId = s(args, "check_in").parse()?;
        let row = self.ledger_mut()?.retract_check_in(&old)?;
        Ok(json!({"retraction":row.to_string(),"retracts":old.to_string()}))
    }

    /// `{"retraction": obs id}`: takes a take-back back (a retraction, or a
    /// done unticked; habit-list names it as `restorable`), so the day reads
    /// as checked in again with the earlier row whole (one more ledger row).
    fn habit_restore(&mut self, args: &Value) -> Result<Value> {
        let retraction: ObjectId = s(args, "retraction").parse()?;
        let row = self.ledger_mut()?.restore_check_in(&retraction)?;
        Ok(json!({"check_in":row.to_string(),"restores":retraction.to_string()}))
    }

    /// `{"from", "through", "hide_all"?}`: per habit and day, the value against its baseline.
    fn habit_heat_map(&self, args: &Value) -> Result<Value> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or_else(|| invalid("This workspace has no ledger yet"))?;
        let from: CivilDate = s(args, "from").parse()?;
        let through: CivilDate = s(args, "through").parse()?;
        let hide_all = self.privacy()?.on || args["hide_all"].as_bool().unwrap_or(false);
        let rows: Vec<Value> = ledger
            .heat_map(from, through, hide_all)?
            .iter()
            .map(|row| {
                let mut habit = habit_json(&row.habit);
                habit["days"] = row
                    .cells
                    .iter()
                    .map(|c| json!({"day":c.day.to_string(),"value":c.value,"against":c.against}))
                    .collect();
                habit
            })
            .collect();
        Ok(
            json!({"from":from.to_string(),"through":through.to_string(),"hide_all":hide_all,"habits":rows}),
        )
    }

    fn tick_collection(&mut self, dispatch_due: bool) -> Result<Value> {
        if self.collection.is_none() {
            self.collection = Some(crate::collection::CollectionStore::open(
                &self.repo.root().join("collection"),
            )?);
        }
        let poll_command = if dispatch_due {
            "collection-poll-due"
        } else {
            "collection-poll"
        };
        let mut status = self
            .collection
            .as_mut()
            .unwrap()
            .command(poll_command, &json!({}))?;
        if let Some((job, ticket)) = &self.collection_import
            && let Some(result) = self.history_job_receipts.get(ticket)
        {
            if result["ok"] == true {
                let ids: Vec<_> = result["result"]["records"]
                    .as_array()
                    .map(|rs| rs.iter().map(|r| r["id"].clone()).collect())
                    .unwrap_or_default();
                self.collection
                    .as_mut()
                    .unwrap()
                    .command("collection-ack", &json!({"job_id":job,"history_ids":ids}))?;
                self.collection_import_error = None;
            } else {
                self.collection_import_error =
                    Some((std::time::Instant::now(), s(result, "error").into()));
            }
            self.collection_import = None;
        }
        if let Some((_, ticket)) = &self.collection_import
            && self.history_job.is_none()
            && !self.history_job_receipts.contains_key(ticket)
        {
            // The durable collection item remains pending; allow the next
            // tick to retry instead of waiting forever on an evicted receipt.
            self.collection_import = None;
            self.collection_import_error = Some((
                std::time::Instant::now(),
                "History import receipt expired; retrying the durable collection item".into(),
            ));
        }
        if self.history_job.is_none()
            && self.collection_import.is_none()
            && self
                .collection_import_error
                .as_ref()
                .is_none_or(|(time, _)| time.elapsed().as_secs() >= 60)
        {
            let pending = self
                .collection
                .as_mut()
                .unwrap()
                .command("collection-pending-imports", &json!({}))?;
            if let Some(job) = pending["items"].as_array().and_then(|items| items.first()) {
                let sources = job["outputs"].as_array().cloned().unwrap_or_else(|| {
                    vec![json!({"path":job["result_path"],"provider":"normalized"})]
                });
                let ticket = self.start_history_job(
                    &json!({"command":"history-import-batch","args":{"sources":sources}}),
                )?;
                self.collection_import = Some((s(job, "job_id").into(), s(&ticket, "id").into()));
            }
        }
        if dispatch_due && self.local_model && self.history_job.is_none() {
            let pending = self
                .collection
                .as_mut()
                .unwrap()
                .command("collection-pending-briefings", &json!({}))?;
            if let Some(job) = pending["items"].as_array().and_then(|items| items.first())
                && let Some(source_id) = job["history_ids"].as_array().and_then(|ids| ids.first())
            {
                let outcome=self.briefing_command("briefing-start",&json!({"source_id":source_id,"goal_ids":job["goal_ids"],"request_key":job["job_id"]}));
                let receipt = match outcome {
                    Ok(receipt) => receipt,
                    Err(error) => json!({"state":"failed","error":error.to_string()}),
                };
                self.collection.as_mut().unwrap().command(
                    "collection-mark-briefing",
                    &json!({"job_id":job["job_id"],"briefing":receipt}),
                )?;
            }
        }
        status["archive_import_running"] = json!(self.collection_import.is_some());
        status["archive_import_error"] = self
            .collection_import_error
            .as_ref()
            .map(|(_, error)| json!(error))
            .unwrap_or(Value::Null);
        Ok(status)
    }

    fn store_history_receipt(&mut self, id: String, value: Value) {
        retain_history_receipt(
            &mut self.history_job_receipts,
            &mut self.history_receipt_bytes,
            &mut self.history_receipt_order,
            id,
            value,
        );
    }

    fn poll_history_job(&mut self) {
        let Some((id, receiver)) = &self.history_job else {
            return;
        };
        match receiver.try_recv() {
            Ok((store, result)) => {
                let id = id.clone();
                self.history = store;
                let receipt = match result {
                    Ok(mut value) => {
                        if value.get("segments").is_some()
                            && let Ok(related) =
                                self.history_related(s(&value, "id"), s(&value, "revision"))
                        {
                            value["related"] = related;
                        }
                        json!({"id":id,"state":"complete","ok":true,"command":self.history_job_command,"result":value})
                    }
                    Err(e) => {
                        json!({"id":id,"state":"failed","ok":false,"error":e.to_string()})
                    }
                };
                self.history_job = None;
                self.store_history_receipt(id, receipt);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let id = id.clone();
                self.history_job = None;
                self.store_history_receipt(
                    id.clone(),
                    json!({"id":id,"state":"failed","ok":false,"error":"History worker stopped. Retry to recover the pending archive."}),
                );
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    fn briefing_command(&mut self, command: &str, args: &Value) -> Result<Value> {
        // Whether he has chosen a model for analysis (briefings/model.json): a
        // file read only, so the views can say an analysis waits for his model
        // instead of calling it queued. Nothing is sent and no model is started.
        if command == "briefing-model" {
            return Ok(
                match crate::briefing::model_setting(&self.repo.root().join("briefings")) {
                    Ok(setting) => json!({"set": true, "model": setting.model}),
                    Err(_) => json!({"set": false, "model": null}),
                },
            );
        }
        if matches!(command, "briefing-start" | "briefing-resume") {
            self.require_local_model()?;
        }
        if self.briefing.is_none() {
            self.briefing = Some(crate::briefing::BriefingStore::open_with(
                &self.repo.root().join("briefings"),
                self.briefing_tools.clone(),
            )?);
        }
        if command == "briefing-resume" {
            let status = self
                .briefing
                .as_mut()
                .unwrap()
                .command("briefing-status", args)?;
            let source_id = status["source_id"]
                .as_str()
                .ok_or_else(|| invalid("Briefing has no pinned source"))?;
            let current = self.history_command("history-revision", &json!({"id":source_id}))?;
            if current["revision"] != status["source_revision"] {
                return Err(invalid(
                    "The source changed. Start a new briefing instead of resuming an older revision.",
                ));
            }
        }
        if command == "briefing-start" {
            let source =
                self.history_command("history-detail", &json!({"id":args["source_id"]}))?;
            let selected = args["goal_ids"].as_array().cloned().unwrap_or_default();
            if selected.len() > 32 {
                return Err(invalid("Choose at most 32 goals for a briefing"));
            }
            let policy=self.source_policy_command("source-policy-evaluate",&json!({"source_id":source["id"],"title":source["title"],"url":source["source_refs"][0]["url"],"goal_id":if selected.len()==1{selected[0].clone()}else{Value::Null}}))?;
            if policy["include"] != true {
                return Err(invalid(
                    "This source is excluded by your local preferences. Change the relevant preference before generating a briefing.",
                ));
            }
            let goals = self
                .snapshot
                .entities()
                .filter(|e| {
                    matches!(e, Entity::Aim(_))
                        && !self.hidden(e)
                        && (selected.is_empty() || selected.iter().any(|id| id == e.id().as_str()))
                })
                .map(|e| json!({"id":e.id().as_str(),"title":e.meta().title}))
                .collect::<Vec<_>>();
            if !selected.is_empty() && goals.len() != selected.len() {
                return Err(invalid(
                    "One or more selected goals changed; choose current active goals",
                ));
            }
            if goals.len() > 32 {
                return Err(invalid(
                    "Choose a focused set of at most 32 goals for this briefing",
                ));
            }
            return self.briefing.as_mut().unwrap().command(
                command,
                &json!({"source":source,"goals":goals,"selected_segment_ids":args["segment_ids"],"request_key":args["request_key"]}),
            );
        }
        let mut result = self.briefing.as_mut().unwrap().command(command, args)?;
        if command == "briefing-status" {
            let id = result["source_id"].as_str().unwrap_or("");
            if !id.is_empty() {
                let current = self.history_command("history-revision", &json!({"id":id}));
                result["stale"] = match current {
                    Ok(record) => json!(record["revision"] != result["source_revision"]),
                    Err(_) => Value::Null,
                };
            }
        }
        Ok(result)
    }
    pub(super) fn source_policy_command(&mut self, command: &str, args: &Value) -> Result<Value> {
        if self.source_preferences.is_none() {
            self.source_preferences = Some(crate::source_preferences::SourcePreferences::open(
                &self.repo.root().join("source-preferences"),
            )?);
        }
        self.source_preferences
            .as_mut()
            .unwrap()
            .command(command, args)
    }
    fn start_history_job(&mut self, args: &Value) -> Result<Value> {
        if self.history_job.is_some() {
            return Ok(json!({"state":"busy","reason":"Another history operation is running"}));
        }
        let command = s(args, "command").to_owned();
        if !matches!(
            command.as_str(),
            "history-import"
                | "history-import-batch"
                | "history-list"
                | "history-detail"
                | "history-word-timings"
                | "history-revisions"
                | "history-correct"
                | "history-label"
                | "history-state"
                | "history-capture-list"
                | "history-capture-add"
                | "history-capture-remove"
                | "history-capture-bootstrap"
                | "history-capture-sync"
                | "history-discover"
                | "history-opencode-sessions"
                | "history-from-inbox"
        ) {
            return Err(invalid("Unsupported background history operation"));
        }
        let mut payload = args["args"].clone();
        if command == "history-from-inbox" {
            if self.inbox.is_none() {
                self.inbox = Some(crate::inbox::InboxStore::open(
                    &self.repo.root().join("inbox"),
                )?);
            }
            let status = self
                .inbox
                .as_mut()
                .unwrap()
                .command("inbox-status", &payload)?;
            if status["state"] != "committed" || status["source_intent"] != "voice_note" {
                return Err(invalid("Select a committed voice-note transfer"));
            }
            payload = status["receipt"].clone();
        }
        let id = ObjectId::mint("history-job")?.to_string();
        let result_id = id.clone();
        self.history_job_command = command.clone();
        let store = self.history.take();
        let root = self.repo.root().join("history");
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if command == "history-capture-bootstrap"
                && let Some(value) = existing_capture_bootstrap(&root, &payload)
            {
                let _ = sender.send((store, Ok(value)));
                return;
            }
            let opened = match store {
                Some(store) => Ok(store),
                None => crate::history::HistoryStore::open(&root),
            };
            let result = match opened {
                Ok(mut store) => {
                    let value = if command == "history-from-inbox" {
                        store.register_inbox(&payload)
                    } else {
                        store.command(&command, &payload)
                    };
                    (Some(store), value)
                }
                Err(e) => (None, Err(e)),
            };
            let _ = sender.send(result);
        });
        self.history_job = Some((id, receiver));
        Ok(json!({"id":result_id,"state":"running"}))
    }

    fn history_command(&mut self, command: &str, args: &Value) -> Result<Value> {
        if self.history_job.is_some() {
            return Err(invalid(
                "History import is running; retry when it completes",
            ));
        }
        if self.history.is_none() {
            self.history = Some(crate::history::HistoryStore::open(
                &self.repo.root().join("history"),
            )?);
        }
        let mut result = self.history.as_mut().unwrap().command(command, args)?;
        if command == "history-detail" {
            let links = self.history_related(s(&result, "id"), s(&result, "revision"))?;
            let related = result["related"].as_array().cloned().unwrap_or_default();
            result["related"] = json!(
                related
                    .into_iter()
                    .chain(links.as_array().unwrap().iter().cloned())
                    .collect::<Vec<_>>()
            );
        }
        Ok(result)
    }
    pub fn calendar(&self, from: &str, through: &str, timezone: &str) -> Result<Value> {
        let start: CivilDate = from.parse()?;
        let end: CivilDate = through.parse()?;
        if time::date(through)? - time::date(from)? > Duration::days(740) || start > end {
            return Err(invalid("Calendar range must be within two years"));
        }
        time::zone(timezone)?;
        let privacy = self.privacy()?;
        let mut items = Vec::new();
        for e in self.snapshot.entities() {
            if self.hidden(e) || privacy.hides_planner(e.id().as_str()) {
                continue;
            }
            let Entity::Task(t) = e else { continue };
            let base = self.record(e);
            if let Some(rule) = &t.recurrence {
                // Include origins of moved occurrences even when outside the
                // requested range, then filter by the resulting interval.
                let mut occurrences = materialize_occurrences(
                    &t.meta.id,
                    rule,
                    start.add_days(-7)?,
                    end.add_days(1)?,
                )?;
                for (key, overridden) in &t.meta.details.overrides {
                    if overridden.origin_date.is_empty() {
                        continue;
                    }
                    let origin: CivilDate = overridden.origin_date.parse()?;
                    let all = materialize_occurrences(&t.meta.id, rule, origin, origin)?;
                    if let Some(o) = all.into_iter().find(|o| o.id.opaque_key().as_str() == key)
                        && !occurrences.iter().any(|v| v.id == o.id)
                    {
                        occurrences.push(o);
                    }
                }
                for o in occurrences {
                    let key = o.id.opaque_key().to_string();
                    let ov = t.meta.details.overrides.get(&key);
                    if ov.is_some_and(|v| v.cancelled) {
                        continue;
                    }
                    let day = ov
                        .and_then(|v| v.date.as_deref())
                        .unwrap_or(&o.on.to_string())
                        .to_owned();
                    let mut v = base.clone();
                    v["occurrence"] = json!(key);
                    v["occurrence_date"] = json!(o.on.to_string());
                    v["occurrence_sequence"] = json!(o.id.sequence);
                    v["status"] = json!(if t.completed_occurrences.contains(&o.id) {
                        "completed"
                    } else {
                        ov.and_then(|v| v.status.as_deref())
                            .unwrap_or(t.status.as_str())
                    });
                    if let Some(title) = ov.and_then(|v| v.title.as_ref()) {
                        v["title"] = json!(title);
                    }
                    let schedule = if ov.is_some_and(|v| v.all_day) {
                        None
                    } else {
                        ov.and_then(|v| v.schedule.as_ref())
                            .or(t.meta.details.schedule.as_ref())
                    };
                    append_event(&mut items, v, &day, schedule, start, end, timezone)?;
                }
            } else if let Some(day) = t.scheduled_for {
                append_event(
                    &mut items,
                    base.clone(),
                    &day.to_string(),
                    t.meta.details.schedule.as_ref(),
                    start,
                    end,
                    timezone,
                )?;
            }
            if let Some(due) = t.due_on
                && (start..=end).contains(&due)
            {
                let mut v = base.clone();
                v["event_kind"] = json!("due");
                v["date"] = json!(due.to_string());
                v["event_key"] = json!(format!("{}:due", e.id()));
                items.push(v);
            }
        }
        if let Some(ledger) = &self.ledger {
            let habits = ledger.habits(privacy.on)?;
            for event in &mut items {
                let ids: Vec<String> = habits
                    .iter()
                    .filter(|h| {
                        self.habit_hierarchy_status(&h.hierarchy) == "linked"
                            && h.hierarchy["task_ids"]
                                .as_array()
                                .is_some_and(|tasks| tasks.iter().any(|id| id == &event["id"]))
                    })
                    .map(|h| h.id.to_string())
                    .collect();
                event["habit_ids"] = json!(ids);
            }
        }
        items.sort_by(|a, b| {
            s(a, "date")
                .cmp(s(b, "date"))
                .then_with(|| s(a, "local_start").cmp(s(b, "local_start")))
        });
        Ok(json!(items))
    }
}

fn lot_json(lot: &Lot) -> Value {
    json!({
        "id": lot.id.to_string(),
        "subject": lot.subject,
        "category": lot.category,
        "quantity": lot.quantity,
        "unit": lot.unit,
        "expires_on": lot.expires_on,
        "location": lot.location,
        "source": lot.source,
        "day": lot.observed_on,
        "has_picture": lot.has_picture,
        "weight": lot.weight().map(|(quantity, unit)| json!({"quantity":quantity,"unit":unit})),
        "purchase_line": lot.purchase_line,
    })
}

/// A written plan as the window reads it.
fn plan_json(plan: &PlanOutcome, horizon: i64) -> Value {
    json!({
        "plan_id": plan.plan_id,
        "rows": plan.rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "portions": plan.portions,
        "coverage_days": plan.coverage_days,
        "constraints": plan.constraints.iter().map(profile_json).collect::<Vec<_>>(),
        "notes": plan.notes,
        "goes_off": {"days":horizon,"columns":plan.goes_off.columns,"rows":plan.goes_off.rows},
        "ingredients": plan.ingredients.iter().map(|i| json!({
            "lot": i.lot.to_string(),
            "subject": i.subject,
            "quantity": i.quantity,
            "unit": i.unit,
            "expires_on": i.expires_on,
            "goes_off_soon": i.goes_off_soon,
        })).collect::<Vec<_>>(),
    })
}

/// The `coverage_days` of a plan: how many days of meals the batch covers.
fn coverage_days(args: &Value) -> Result<i64> {
    args["coverage_days"]
        .as_i64()
        .ok_or_else(|| invalid("Choose how many days the batch covers"))
}

/// Minor units as the window shows money: `12.34`, `-3.50`.
fn money(minor: i64) -> String {
    format!(
        "{}{}.{:02}",
        if minor < 0 { "-" } else { "" },
        minor.abs() / 100,
        minor.abs() % 100
    )
}

/// An amount given as `12.34` (text or number) in minor units.
fn minor_amount(value: &Value) -> Result<i64> {
    let text = match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => return Err(invalid("An amount is a number like 12.34")),
    };
    let wrong = || invalid(format!("{text:?} is not an amount like 12.34"));
    let (negative, digits) = text
        .strip_prefix('-')
        .map_or((false, text.as_str()), |d| (true, d));
    let (whole, cents) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty()
        || cents.len() > 2
        || !whole
            .chars()
            .chain(cents.chars())
            .all(|c| c.is_ascii_digit())
    {
        return Err(wrong());
    }
    let whole: i64 = whole.parse().map_err(|_| wrong())?;
    let cents: i64 = format!("{cents:0<2}").parse().map_err(|_| wrong())?;
    let minor = whole
        .checked_mul(100)
        .and_then(|w| w.checked_add(cents))
        .ok_or_else(wrong)?;
    Ok(if negative { -minor } else { minor })
}

/// The lower-cased cells of a CSV file's first line (at most 64 KiB read).
fn csv_header(path: &Path) -> Result<Vec<String>> {
    let mut first = String::new();
    File::open(path)?
        .take(64 * 1024)
        .read_to_string(&mut first)?;
    let line = first.lines().next().unwrap_or_default();
    Ok(line
        .trim_start_matches('\u{feff}')
        .split(',')
        .map(|c| c.trim().trim_matches('"').to_ascii_lowercase())
        .collect())
}

fn profile_json(entry: &ProfileEntry) -> Value {
    json!({
        "id": entry.id.to_string(),
        "subject": entry.subject,
        "quantity": entry.quantity,
        "unit": entry.unit,
        "value": entry.value,
    })
}

fn number(args: &Value, key: &str) -> Result<f64> {
    args[key]
        .as_f64()
        .ok_or_else(|| invalid(format!("{key} must be a number")))
}

fn habit_json(h: &Habit) -> Value {
    json!({
        "id": h.id.to_string(),
        "title": h.spec.title,
        "direction": h.spec.direction.as_str(),
        "baseline": h.spec.baseline,
        "bundle": h.spec.bundle.map(Bundle::as_str),
        "hierarchy": h.hierarchy,
        "definition": h.row.to_string(),
    })
}

/// The check-in day: `args.day` when given, else today.
fn habit_day(args: &Value) -> Result<String> {
    match args["day"].as_str() {
        Some(day) => Ok(day.parse::<CivilDate>()?.to_string()),
        None => Ok(chrono::Local::now().format("%Y-%m-%d").to_string()),
    }
}

fn check_in_value(args: &Value) -> Result<CheckIn> {
    match (args["done"].as_bool(), args["count"].as_f64()) {
        (Some(true), None) => Ok(CheckIn::Done),
        (Some(false), None) => Ok(CheckIn::Missed),
        (None, Some(n)) => Ok(CheckIn::Count(n)),
        _ => Err(invalid(
            "A check-in is either done (true or false) or a count",
        )),
    }
}

fn cadence_label(r: &RecurrenceRule) -> &'static str {
    match &r.cadence {
        RecurrenceCadence::Daily => "daily",
        RecurrenceCadence::Weekly { weekdays } if weekdays.len() == 5 => "weekdays",
        RecurrenceCadence::Weekly { .. } => "weekly",
        RecurrenceCadence::Monthly { .. } => "monthly",
        RecurrenceCadence::Yearly { .. } => "yearly",
    }
}
fn rule(v: &Value) -> Result<Option<RecurrenceRule>> {
    let kind = s(v, "recurrence");
    if kind.is_empty() || kind == "none" {
        return Ok(None);
    }
    let starts = if !s(v, "repeat_start").is_empty() {
        s(v, "repeat_start")
    } else {
        s(v, "scheduled_for")
    };
    let d: CivilDate = starts.parse()?;
    let date = time::date(starts)?;
    let cadence = match kind {
        "daily" => RecurrenceCadence::Daily,
        "weekdays" => RecurrenceCadence::Weekly {
            weekdays: [
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
            ]
            .into_iter()
            .collect(),
        },
        "weekly" | "biweekly" => RecurrenceCadence::Weekly {
            weekdays: [d.weekday()].into_iter().collect(),
        },
        "monthly" => RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::DayOfMonth(date.day() as u8),
        },
        "yearly" => RecurrenceCadence::Yearly {
            month: date.month() as u8,
            day: date.day() as u8,
        },
        _ => return Err(invalid("Unknown recurrence pattern")),
    };
    let interval = if kind == "biweekly" {
        2
    } else {
        v["interval"]
            .as_u64()
            .ok_or_else(|| invalid("Repeat interval must be a whole number"))?
    };
    if interval == 0 || interval > 1000 {
        return Err(invalid("Repeat interval must be 1–1000"));
    }
    let r = RecurrenceRule {
        starts_on: d,
        cadence,
        interval: interval as u16,
        ends_on: optional_date(v, "repeat_until")?,
        max_occurrences: None,
        exclusions: BTreeSet::new(),
    };
    r.validate()?;
    Ok(Some(r))
}
fn validate_occurrence(t: &Task, args: &Value) -> Result<OccurrenceId> {
    let day: CivilDate = s(args, "occurrence_date").parse()?;
    let rule = t
        .recurrence
        .as_ref()
        .ok_or_else(|| invalid("This task does not repeat"))?;
    materialize_occurrences(&t.meta.id, rule, day, day)?
        .into_iter()
        .find(|o| o.id.opaque_key().as_str() == s(args, "occurrence"))
        .map(|o| o.id)
        .ok_or_else(|| invalid("Occurrence changed. Reload the calendar before editing."))
}
fn append_event(
    items: &mut Vec<Value>,
    mut v: Value,
    day: &str,
    schedule: Option<&soliton_kernel::details::Schedule>,
    from: CivilDate,
    through: CivilDate,
    tz: &str,
) -> Result<()> {
    let recurring = !s(&v, "occurrence").is_empty();
    let key = if s(&v, "occurrence").is_empty() {
        s(&v, "id")
    } else {
        s(&v, "occurrence")
    }
    .to_string();
    v["event_key"] = json!(key);
    v["date"] = json!(day);
    if let Some(z) = schedule {
        let start = if recurring {
            time::resolve(day, &z.time, &z.timezone, &z.fold, true)?
        } else {
            z.start_utc
        };
        let end = start + i64::from(z.duration_minutes) * 60;
        let local_start = time::local_string(start, tz)?;
        let local_end = time::local_string(end, tz)?;
        let first: CivilDate = local_start[..10].parse()?;
        let last: CivilDate = time::local_string(end - 1, tz)?[..10].parse()?;
        if first > through || last < from {
            return Ok(());
        }
        v["event_kind"] = json!("timed");
        v["start_utc"] = json!(start);
        v["end_utc"] = json!(end);
        v["local_start"] = json!(local_start);
        v["local_end"] = json!(local_end);
        v["date"] = json!(first.to_string());
        v["last_date"] = json!(last.to_string());
        v["schedule"] = json!(z);
    } else {
        v["schedule"] = Value::Null;
        let d: CivilDate = day.parse()?;
        if !(from..=through).contains(&d) {
            return Ok(());
        }
        v["event_kind"] = json!("all_day");
    }
    items.push(v);
    Ok(())
}
/// Whether a media job file in the workspace's collection says it was running when its engine ended. Reads only.
fn media_left_running(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root.join("collection/media/jobs")) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.path().extension().is_some_and(|x| x == "json")
            && fs::metadata(e.path()).is_ok_and(|m| m.is_file() && m.len() <= 1_000_000)
            && fs::read(e.path())
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .is_some_and(|job| job["state"] == "running")
    })
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(invalid("Choose an absolute source path"));
    }
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("Choose a regular file"));
    }
    if file.metadata()?.len() > limit {
        return Err(invalid("This source exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("Source grew beyond the size limit"));
    }
    Ok(bytes)
}
fn read_source(path: &str) -> Result<Value> {
    let bytes = bounded_read(Path::new(path), 2 * 1024 * 1024)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| invalid("This file is not UTF-8 text. Open it in its native application."))?;
    Ok(
        json!({"path":path,"text":text,"bytes":bytes.len(),"hash":ContentHash::digest(&bytes).to_string()}),
    )
}

#[cfg(test)]
mod history_receipt_tests {
    use super::*;

    #[test]
    fn receipt_cache_keeps_newest_oversized_value_without_serializing_it() {
        let mut receipts = std::collections::BTreeMap::new();
        let mut sizes = std::collections::BTreeMap::new();
        let mut order = VecDeque::new();
        let oversized = "x".repeat(HISTORY_RECEIPT_MAX_BYTES + 1024);
        retain_history_receipt(
            &mut receipts,
            &mut sizes,
            &mut order,
            "newest".into(),
            Value::String(oversized),
        );
        assert!(receipts.contains_key("newest"));
        assert_eq!(receipts.len(), 1);
        assert!(sizes["newest"] > HISTORY_RECEIPT_MAX_BYTES);
    }

    #[test]
    fn receipt_cache_evicts_oldest_until_count_and_bytes_are_bounded() {
        let mut receipts = std::collections::BTreeMap::new();
        let mut sizes = std::collections::BTreeMap::new();
        let mut order = VecDeque::new();
        for index in 0..40 {
            retain_history_receipt(
                &mut receipts,
                &mut sizes,
                &mut order,
                format!("job-{index}"),
                json!({"text":"small"}),
            );
        }
        assert_eq!(receipts.len(), 32);
        assert!(receipts.contains_key("job-39"));
        assert!(!receipts.contains_key("job-0"));
        assert!(sizes.values().sum::<usize>() <= HISTORY_RECEIPT_MAX_BYTES);
    }
}
