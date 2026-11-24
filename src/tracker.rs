//! The tracker store read in-process through the store's own crate (`crates/tracker-store`), so the sheet a view
//! shows is the record itself. Every read opens the store read-only and closes it with the reply; nothing is read
//! while no sheet view has said it is shown. While a view is shown one read-only connection is held and every
//! commit to the store is noticed on the next tick (today: SQLite's `data_version`, which moves on any other
//! connection's commit, the log's included; the revision is re-read when it moves). Writes go only through the
//! installed `tracker` command, from an explicit act.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tracker_store::{Order, Store};

/// Tabs gathered into a few life categories, Learning first (today's setting; compiled in with the build).
const CATEGORIES: &str = include_str!("../resources/tracker_categories.json");
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 2000;

pub struct Tracker {
    store: PathBuf,
    convention: PathBuf,
    program: PathBuf,
    /// Present only while a sheet view has said it is shown; dropping it closes the store.
    watch: Option<Watch>,
}
/// The connection held for a shown view, the file it was opened on, and what it last saw.
struct Watch {
    store: Store,
    file: Option<u64>,
    version: u64,
    revision: u64,
}
impl Watch {
    fn open(path: &Path) -> Result<Self> {
        let file = identity(path);
        let store = read_only(path)?;
        // `data_version` first: a commit landing before the revision read moves it again for the next look.
        let version = store.data_version().map_err(store_error)?;
        let revision = store.revision().map_err(store_error)?;
        Ok(Self {
            store,
            file,
            version,
            revision,
        })
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}
fn setting(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(default))
}
fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
fn store_error(e: tracker_store::Error) -> Error {
    invalid(e.to_string())
}
fn read_only(path: &Path) -> Result<Store> {
    if !path.is_file() {
        return Err(invalid("The tracker is not available"));
    }
    Store::open_read_only(path).map_err(store_error)
}
/// The store file's identity by its metadata, never an open: a store replaced under its name is opened again.
fn identity(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    return Some(std::os::unix::fs::MetadataExt::ino(&m));
    #[cfg(not(unix))]
    return Some(m.len());
}

impl Tracker {
    /// The store, the colour convention and the `tracker` command from the launcher's settings. Touches nothing.
    pub fn from_env() -> Self {
        Self {
            store: setting(
                "SOLITON_TRACKER_STORE",
                "06_memory/tracker/live/store/tracker.sqlite",
            ),
            convention: setting(
                "SOLITON_TRACKER_CONVENTION",
                ".local/lib/zavent-sheets/current/convention.json",
            ),
            program: setting("SOLITON_TRACKER_BIN", ".local/bin/tracker"),
            watch: None,
        }
    }
    /// The launcher's settings with the store given here (a test's copy).
    pub fn with_store(store: PathBuf) -> Self {
        Self {
            store,
            ..Self::from_env()
        }
    }
    /// The revision last read while a view is shown; `None` while none is.
    pub fn revision(&self) -> Option<u64> {
        self.watch.as_ref().map(|w| w.revision)
    }
    fn open(&self) -> Result<Store> {
        read_only(&self.store)
    }
    /// While a view is shown: the new revision when the store changed since the last look, else `None`.
    /// With no view shown it does nothing at all.
    pub fn poll(&mut self) -> Result<Option<u64>> {
        let Some(watch) = self.watch.as_mut() else {
            return Ok(None);
        };
        if identity(&self.store) != watch.file {
            let before = watch.revision;
            *watch = Watch::open(&self.store)?;
            return Ok((watch.revision != before).then_some(watch.revision));
        }
        let version = watch.store.data_version().map_err(store_error)?;
        if version == watch.version {
            return Ok(None);
        }
        watch.version = version;
        let revision = watch.store.revision().map_err(store_error)?;
        if revision == watch.revision {
            return Ok(None);
        }
        watch.revision = revision;
        Ok(Some(revision))
    }
    fn convention(&self) -> Value {
        std::fs::read(&self.convention)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null)
    }
    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "tracker-revision" => {
                Ok(json!({"revision": self.open()?.revision().map_err(store_error)?}))
            }
            "tracker-tabs" => self.tabs(),
            "tracker-rows" => self.rows(args),
            "tracker-row" => {
                let id = args["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("A row id is required"))?;
                let store = self.open()?;
                Ok(json!({
                    "row": store.get(id).map_err(store_error)?,
                    "revision": store.revision().map_err(store_error)?,
                    "convention": self.convention(),
                }))
            }
            "tracker-watch" => {
                if args["on"].as_bool().unwrap_or(false) {
                    if self.watch.is_none() {
                        self.watch = Some(Watch::open(&self.store)?);
                    }
                } else {
                    self.watch = None;
                }
                Ok(json!({"watching": self.watch.is_some(), "revision": self.revision()}))
            }
            "tracker-set" => self.set(args),
            _ => Err(invalid(format!("Unknown command {command}"))),
        }
    }
    fn tabs(&self) -> Result<Value> {
        let store = self.open()?;
        let tabs = store.tabs().map_err(store_error)?;
        Ok(json!({
            "categories": categorise(&tabs)?,
            "revision": store.revision().map_err(store_error)?,
        }))
    }
    fn rows(&self, args: &Value) -> Result<Value> {
        let tab = args["tab"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("A tab is required"))?;
        let filter = match &args["filter"] {
            Value::Null => None,
            f => Some((
                f["column"]
                    .as_str()
                    .ok_or_else(|| invalid("A filter names its column"))?,
                f["text"].as_str().unwrap_or(""),
            )),
        }
        .filter(|(_, text)| !text.is_empty());
        let order = match &args["order"] {
            Value::Null => None,
            o => Some(Order {
                column: o["column"]
                    .as_str()
                    .ok_or_else(|| invalid("An order names its column"))?,
                descending: match o["direction"].as_str().unwrap_or("asc") {
                    "asc" => false,
                    "desc" => true,
                    _ => return Err(invalid("An order runs asc or desc")),
                },
            }),
        };
        let limit = args["limit"]
            .as_u64()
            .map_or(DEFAULT_LIMIT, |n| n as usize)
            .min(MAX_LIMIT);
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let store = self.open()?;
        let page = store
            .tab_page(tab, filter, order, limit, offset)
            .map_err(store_error)?;
        let mut reply = serde_json::to_value(page)?;
        reply["revision"] = json!(store.revision().map_err(store_error)?);
        reply["convention"] = self.convention();
        Ok(reply)
    }
    /// One field of one row through the installed `tracker` command, from an explicit act; the engine itself
    /// never opens the store for writing and never sets a status on its own.
    fn set(&self, args: &Value) -> Result<Value> {
        let text = |key: &str| {
            args[key]
                .as_str()
                .ok_or_else(|| invalid(format!("tracker-set needs {key}")))
        };
        let (id, field, value, expect) =
            (text("id")?, text("field")?, text("value")?, text("expect")?);
        if id.is_empty() || field.is_empty() || field.contains('=') {
            return Err(invalid("tracker-set needs a row id and a field name"));
        }
        let output = std::process::Command::new(&self.program)
            .arg("--db")
            .arg(&self.store)
            .args(["--author", "soliton", "set", id])
            .arg(format!("{field}={value}"))
            .arg("--expect")
            .arg(format!("{field}={expect}"))
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|_| invalid("The tracker command is not available"))?;
        let said = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
        if !output.status.success() {
            return Err(invalid(said(&output.stderr)));
        }
        Ok(json!({"id": id, "changed": said(&output.stdout) != "unchanged"}))
    }
}

/// Each category of the setting in order with its tabs present in the store, in the setting's order; a tab the
/// setting does not place lands under a category of its own name, after the others, in the store's order.
fn categorise(tabs: &[tracker_store::Tab]) -> Result<Value> {
    let setting: Value = serde_json::from_str(CATEGORIES)?;
    let mut placed = std::collections::BTreeSet::new();
    let entry = |t: &tracker_store::Tab| json!({"name": t.name, "kind": t.kind, "rows": t.rows});
    let mut out = Vec::new();
    for c in setting["categories"].as_array().into_iter().flatten() {
        let members: Vec<Value> = c["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|name| tabs.iter().find(|t| t.name == name))
            .filter(|t| placed.insert(t.name.clone()))
            .map(entry)
            .collect();
        if !members.is_empty() {
            out.push(json!({"name": c["name"], "tabs": members}));
        }
    }
    for t in tabs.iter().filter(|t| !placed.contains(&t.name)) {
        out.push(json!({"name": t.name, "tabs": [entry(t)]}));
    }
    Ok(Value::Array(out))
}
