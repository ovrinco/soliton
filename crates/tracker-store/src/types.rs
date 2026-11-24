use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type Fields = Map<String, Value>;
pub type Row = Fields;
pub type Event = Row;

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub rows: Vec<Row>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(default)]
    pub seq: i64,
    pub campaign: String,
    pub wave: i64,
    pub task: String,
    pub directive: String,
    #[serde(default)]
    pub row_id: String,
    pub part: i64,
    pub state: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub author: String,
}
pub const REQ_COLS: &[&str] = &[
    "id",
    "tab",
    "item_type",
    "status",
    "purpose",
    "intent",
    "dod",
    "notes",
    "overlap",
    "goal_deadline",
    "surface",
    "source",
    "said_on",
    "exemplar",
    "implemented_in",
    "age",
    "outcome",
    "outcome_id",
];
pub const REQ_HEADERS: &[&str] = &[
    "ID",
    "TAB",
    "ITEM TYPE",
    "STATUS",
    "PURPOSE",
    "INTENT",
    "D.O.D.",
    "NOTES",
    "OVERLAP",
    "GOAL/DEADLINE",
    "SURFACE",
    "SOURCE",
    "SAID ON",
    "EXEMPLAR",
    "IMPLEMENTED IN",
    "AGE",
    "OUTCOME",
    "OUTCOME ID",
];
pub const CAMP_COLS: &[&str] = &[
    "campaign",
    "campaign_state",
    "campaign_evidence",
    "campaign_remaining",
    "campaign_updated_at",
];
pub const CAMP_HEADERS: &[&str] = &[
    "CAMPAIGN",
    "CAMPAIGN STATE",
    "CAMPAIGN EVIDENCE",
    "CAMPAIGN REMAINING",
    "CAMPAIGN UPDATED AT",
];

pub(crate) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub(crate) fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
