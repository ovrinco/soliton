//! Private mode: one owner setting that hides everything marked private from
//! every view's data source, including where it repeats (calendar entries,
//! reports, briefings, previews). Hiding is a filter on read results; nothing
//! stored is changed and `data_class` is not consulted.
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Modules the owner can mark private as a whole.
pub const MODULES: [&str; 14] = [
    "planner",
    "history",
    "inbox",
    "collection",
    "briefing",
    "proposals",
    "sheets",
    "qroc",
    "modules",
    "recovery",
    "habits",
    "kitchen",
    "finance",
    "health",
];

/// `sheets-owner` actions that only read. Every other action writes or
/// starts work, and its reply is passed through untouched.
pub const SHEETS_READS: [&str; 15] = [
    "list",
    "audit",
    "chat-state",
    "documents",
    "document-status",
    "jobs",
    "job-status",
    "plugins",
    "preview",
    "skills",
    "skill-content",
    "snapshots",
    "inspect",
    "validate",
    "verify",
];

/// Where each module key meets the ledger `writer` names of the data plane
/// (work/T001/DATA_PLANE.md, section 7). `None`: no ledger writer yet.
pub const LEDGER_WRITERS: [(&str, Option<&str>); 14] = [
    ("planner", Some("planner")),
    ("history", None),
    ("inbox", None),
    ("collection", None),
    ("briefing", None),
    ("proposals", Some("planner")),
    ("sheets", None),
    ("qroc", None),
    ("modules", None),
    ("recovery", None),
    ("habits", Some("habits")),
    ("kitchen", Some("kitchen")),
    ("finance", Some("finance")),
    ("health", Some("health")),
];

/// The module a view's read call belongs to, or `None` for writes and calls
/// that feed no view of owner content.
#[must_use]
pub fn view_module(command: &str, args: &Value) -> Option<&'static str> {
    Some(match command {
        "sheets-owner"
            if args["action"]
                .as_str()
                .is_some_and(|action| SHEETS_READS.contains(&action)) =>
        {
            "sheets"
        }
        "state" | "calendar" | "history" => "planner",
        "history-list"
        | "history-detail"
        | "history-revisions"
        | "history-capture-list"
        | "history-opencode-sessions" => "history",
        "inbox-list" | "inbox-status" => "inbox",
        "collection-list" | "collection-status" => "collection",
        "briefing-list" | "briefing-status" | "briefing-chunk" => "briefing",
        "proposal-list" | "proposal-detail" | "proposal-calendar" => "proposals",
        "sheets-history-preview" | "tracker-tabs" | "tracker-rows" | "tracker-row" => "sheets",
        "qroc-project-list" | "qroc-project-detail" => "qroc",
        "modules-list" | "modules-status" => "modules",
        "recovery-list" => "recovery",
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct Privacy {
    /// The one control: private mode on or off.
    pub on: bool,
    /// Whole modules the owner marked private.
    pub modules: BTreeSet<String>,
    /// Planner objects hidden by a marker on themselves or an ancestor.
    pub ids: BTreeSet<String>,
}

impl Privacy {
    #[must_use]
    pub fn hides_planner(&self, id: &str) -> bool {
        self.on && (self.modules.contains("planner") || self.ids.contains(id))
    }

    /// Hide a read result. A private module yields its result with every
    /// list emptied, every text cleared and every count zeroed. Otherwise a
    /// list entry that names a hidden object anywhere inside it is dropped
    /// and the listing's counts lowered to match.
    #[must_use]
    pub fn filter(&self, module: &str, mut value: Value) -> Value {
        if !self.on {
            return value;
        }
        if self.modules.contains(module) {
            empty(&mut value);
            return value;
        }
        if self.ids.is_empty() {
            return value;
        }
        // A single item (it has its own identity, or no list of entries) that
        // names a hidden object is hidden whole.
        let item = value
            .get("id")
            .or(value.get("job_id"))
            .is_some_and(Value::is_string);
        if value.is_object() && (item || !has_list(&value)) && self.cites(&value) {
            return Value::Object(Default::default());
        }
        self.drop_citing(&mut value);
        value
    }

    /// Briefing list entries do not carry the goals they were written for;
    /// read each entry's job file (read only) and drop those that cite a
    /// hidden object.
    #[must_use]
    pub fn drop_briefings(&self, jobs: &Path, mut value: Value) -> Value {
        if self.ids.is_empty() {
            return value;
        }
        if let Some(items) = value["items"].as_array_mut() {
            let before = items.len();
            items.retain(|item| {
                let Some(id) = item["job_id"]
                    .as_str()
                    .filter(|id| !id.contains(['/', '.']))
                else {
                    return true;
                };
                std::fs::read(jobs.join(format!("{id}.json")))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_none_or(|job| !self.cites(&job["goals"]))
            });
            let dropped = before - items.len();
            reduce_total(&mut value, dropped);
        }
        value
    }

    fn cites(&self, value: &Value) -> bool {
        match value {
            Value::String(text) => self.ids.iter().any(|id| text.contains(id.as_str())),
            Value::Array(items) => items.iter().any(|v| self.cites(v)),
            Value::Object(map) => map
                .iter()
                .any(|(k, v)| self.ids.contains(k) || self.cites(v)),
            _ => false,
        }
    }

    fn drop_citing(&self, value: &mut Value) {
        match value {
            Value::Array(items) => items.retain(|v| !self.cites(v)),
            Value::Object(map) => {
                map.retain(|k, _| !self.ids.contains(k));
                // Only lists of entries count; a list of identifiers is a field.
                let mut dropped = 0;
                for v in map.values_mut() {
                    if v.is_string() && self.cites(v) {
                        *v = Value::String(String::new());
                    } else if let Some(items) = v
                        .as_array()
                        .filter(|items| items.iter().any(Value::is_object))
                    {
                        let before = items.len();
                        self.drop_citing(v);
                        dropped += before - v.as_array().map_or(0, Vec::len);
                    } else {
                        self.drop_citing(v);
                    }
                }
                reduce_total(value, dropped);
            }
            _ => {}
        }
    }
}

/// A listing holds entries (objects) in a list; a list of plain identifiers
/// belongs to a detail.
fn has_list(value: &Value) -> bool {
    value.as_object().is_some_and(|m| {
        m.values().any(|v| {
            v.as_array()
                .is_some_and(|items| items.iter().any(Value::is_object))
        })
    })
}

fn is_count(key: &str) -> bool {
    key == "total" || key == "count" || key.ends_with("_total") || key.ends_with("_count")
}

/// Lower a listing's counts by the entries dropped from it, so a count never
/// shows that something is hidden.
fn reduce_total(value: &mut Value, dropped: usize) {
    if dropped == 0 {
        return;
    }
    if let Some(map) = value.as_object_mut() {
        for (key, count) in map.iter_mut() {
            if is_count(key)
                && let Some(n) = count.as_u64()
            {
                *count = Value::from(n.saturating_sub(dropped as u64));
            }
        }
    }
}

fn empty(value: &mut Value) {
    match value {
        Value::Array(items) => items.clear(),
        Value::String(text) => text.clear(),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if is_count(key) && v.is_number() {
                    *v = Value::from(0);
                } else {
                    empty(v);
                }
            }
        }
        _ => {}
    }
}
