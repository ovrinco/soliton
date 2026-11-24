//! Versioned desktop fields, serialized inside each journaled object.
use crate::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Details {
    pub body: String,
    pub order: i64,
    pub archived: bool,
    pub links: Vec<String>,
    pub files: Vec<String>,
    pub tags: Vec<String>,
    pub schedule: Option<Schedule>,
    pub overrides: BTreeMap<String, OccurrenceOverride>,
    /// Original rule/occurrence completion identities at each series edit.
    pub series_history: Vec<SeriesHistory>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub time: String,
    pub duration_minutes: u32,
    pub timezone: String,
    pub start_utc: i64,
    /// earlier, later; recurring gaps shift by the size of the gap.
    pub fold: String,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OccurrenceOverride {
    pub origin_date: String,
    pub date: Option<String>,
    pub schedule: Option<Schedule>,
    pub title: Option<String>,
    pub cancelled: bool,
    pub all_day: bool,
    /// Nonterminal state for this occurrence; completion retains its existing identity set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesHistory {
    pub changed_at: i64,
    pub rule_hash: String,
    pub completed: Vec<String>,
    pub overrides: BTreeMap<String, OccurrenceOverride>,
}

impl Details {
    pub fn validate(&self) -> Result<()> {
        if self.body.len() > 1_000_000
            || self.links.len() > 256
            || self.files.len() > 256
            || self.tags.len() > 128
            || self.overrides.len() > 10000
        {
            return Err(KernelError::InvalidObject(
                "object details exceed supported limits".into(),
            ));
        }
        for o in self.overrides.values().chain(
            self.series_history
                .iter()
                .flat_map(|h| h.overrides.values()),
        ) {
            if o.status
                .as_deref()
                .is_some_and(|s| !["todo", "in_progress"].contains(&s))
            {
                return Err(KernelError::InvalidObject(
                    "invalid occurrence status".into(),
                ));
            }
        }
        for s in self
            .schedule
            .iter()
            .chain(self.overrides.values().filter_map(|o| o.schedule.as_ref()))
        {
            if s.duration_minutes == 0
                || s.duration_minutes > 10080
                || !["earlier", "later"].contains(&s.fold.as_str())
                || s.time.len() != 5
                || s.timezone.is_empty()
            {
                return Err(KernelError::InvalidObject("invalid timed schedule".into()));
            }
        }
        Ok(())
    }
}
