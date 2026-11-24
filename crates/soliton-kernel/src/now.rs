//! The now view's engine (OC-12, OD-2026-10-10-04-R09): today's bounded set
//! of items assembled from what is already on the machine, ranked by his
//! objective as a setting, each carrying the reason it is there and the
//! objects or ledger rows it stands on. An empty day is a valid result.
//!
//! Today's sources are the planner's tasks due or scheduled today, the habit
//! bundle whose window is open and its habits without a check-in, and the
//! lots that go off within the kitchen's span. Later feeds join as further
//! kinds and reasons; nothing stands in for them. The function is pure: it
//! reads the snapshot and the ledger and writes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde_json::Value;

use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::ledger::habits::Bundle;
use crate::ledger::{Ledger, MarkScope};
use crate::model::{Entity, EntityKind, Task, TaskStatus};
use crate::projection::{calendar_between, list_projection};
use crate::recurrence::materialize_occurrences;
use crate::repository::Snapshot;

/// A time of day, minutes after midnight, local to him.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClockTime(u16);

impl ClockTime {
    pub fn new(hour: u8, minute: u8) -> Result<Self> {
        if hour > 23 || minute > 59 {
            return Err(KernelError::InvalidObject(format!(
                "a time of day is 00:00 to 23:59, not {hour:02}:{minute:02}"
            )));
        }
        Ok(Self(u16::from(hour) * 60 + u16::from(minute)))
    }

    #[must_use]
    pub const fn minutes(self) -> u16 {
        self.0
    }
}

impl FromStr for ClockTime {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        let invalid = || KernelError::InvalidObject(format!("invalid time of day {value:?}"));
        let (hour, minute) = value.split_once(':').ok_or_else(invalid)?;
        if hour.len() != 2 || minute.len() != 2 {
            return Err(invalid());
        }
        Self::new(
            hour.parse().map_err(|_| invalid())?,
            minute.parse().map_err(|_| invalid())?,
        )
    }
}

impl fmt::Display for ClockTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

/// What a now item stands on.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NowKind {
    Task,
    Habit,
    Lot,
}

impl NowKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Habit => "habit",
            Self::Lot => "lot",
        }
    }
}

/// Why an item is in the view; the window turns each into a word or two.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NowWhy {
    DueToday,
    ScheduledNow,
    BundleOpen,
    GoesOff,
}

impl NowWhy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DueToday => "due-today",
            Self::ScheduledNow => "scheduled-now",
            Self::BundleOpen => "bundle-open",
            Self::GoesOff => "goes-off",
        }
    }
}

/// The kind of thing a source names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    /// A planner task, by object id.
    Task,
    /// A habit definition on the ledger, by habit id.
    Habit,
    /// A habit bundle, by its name (`morning`, `evening`).
    Bundle,
    /// A kitchen lot on the ledger, by row id.
    Lot,
}

impl SourceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Habit => "habit",
            Self::Bundle => "bundle",
            Self::Lot => "lot",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NowSource {
    pub kind: SourceKind,
    pub id: String,
    /// The human title the window shows.
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NowItem {
    pub kind: NowKind,
    /// The task or its occurrence, the habit, or the lot.
    pub id: String,
    pub title: String,
    pub why: NowWhy,
    /// One-based place in the view.
    pub rank: usize,
    pub sources: Vec<NowSource>,
    /// The task's time of day; none for an all-day task and other kinds.
    pub at: Option<ClockTime>,
    /// The lot's date; none for other kinds.
    pub on: Option<CivilDate>,
}

/// The view: at most `bound` items and how many were left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowList {
    pub items: Vec<NowItem>,
    pub left_out: usize,
}

/// His objective as settings; the defaults are today's order and bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NowSettings {
    pub bound: usize,
    pub order: Vec<NowKind>,
    /// The morning bundle is open from the day's start until this time.
    pub morning_until: ClockTime,
    /// The evening bundle is open from this time to the day's end.
    pub evening_from: ClockTime,
    /// The kitchen's look-ahead for lots going off.
    pub goes_off_days: i64,
    /// The hide-all control: private rows, items, categories, goals and modules stay out.
    pub hide_all: bool,
}

/// The kitchen's span the window offers by default (`kitchenSpan`).
pub const DEFAULT_GOES_OFF_DAYS: i64 = 3;

impl Default for NowSettings {
    fn default() -> Self {
        Self {
            bound: 7,
            order: vec![NowKind::Task, NowKind::Habit, NowKind::Lot],
            morning_until: ClockTime(12 * 60),
            evening_from: ClockTime(18 * 60),
            goes_off_days: DEFAULT_GOES_OFF_DAYS,
            hide_all: false,
        }
    }
}

impl NowSettings {
    /// The bundle whose window holds `local_time`, if any.
    #[must_use]
    pub fn open_bundle(&self, local_time: ClockTime) -> Option<Bundle> {
        if local_time < self.morning_until {
            Some(Bundle::Morning)
        } else if local_time >= self.evening_from {
            Some(Bundle::Evening)
        } else {
            None
        }
    }
}

/// Today's now items, ranked and bounded. Without a ledger (the data plane is
/// off) only planner items appear.
pub fn items(
    snapshot: &Snapshot,
    ledger: Option<&Ledger>,
    day: CivilDate,
    local_time: ClockTime,
    settings: &NowSettings,
) -> Result<NowList> {
    let mut found = task_items(snapshot, ledger, day, settings.hide_all)?;
    if let Some(ledger) = ledger {
        if let Some(bundle) = settings.open_bundle(local_time) {
            found.extend(habit_items(ledger, bundle, day, settings.hide_all)?);
        }
        found.extend(lot_items(ledger, day, settings)?);
    }
    let place = |kind: NowKind| {
        settings
            .order
            .iter()
            .position(|k| *k == kind)
            .unwrap_or(settings.order.len())
    };
    // Habits arrive in bundle order and lots in date order; a stable sort keeps both.
    found.sort_by(|(a, a_step), (b, b_step)| {
        place(a.kind)
            .cmp(&place(b.kind))
            .then_with(|| match (a.at, b.at) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.on.cmp(&b.on))
            .then_with(|| a_step.cmp(b_step))
    });
    let left_out = found.len().saturating_sub(settings.bound);
    let items = found
        .into_iter()
        .take(settings.bound)
        .enumerate()
        .map(|(index, (mut item, _))| {
            item.rank = index + 1;
            item
        })
        .collect();
    Ok(NowList { items, left_out })
}

fn hidden_task(task: &Task, ledger: Option<&Ledger>, hide_all: bool) -> Result<bool> {
    let Some(ledger) = ledger.filter(|_| hide_all) else {
        return Ok(false);
    };
    if ledger.is_marked_private(MarkScope::Item, task.meta.id.as_str())? {
        return Ok(true);
    }
    if let Some(category) = &task.category_id {
        if ledger.is_marked_private(MarkScope::Category, category.as_str())? {
            return Ok(true);
        }
    }
    if let Some(aim) = &task.aim_id {
        if ledger.is_marked_private(MarkScope::Goal, aim.as_str())? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn time_of(value: &str) -> Option<ClockTime> {
    value.parse().ok()
}

fn task_item(
    task: &Task,
    id: String,
    title: String,
    why: NowWhy,
    at: Option<ClockTime>,
) -> NowItem {
    NowItem {
        kind: NowKind::Task,
        id,
        title,
        why,
        rank: 0,
        sources: vec![NowSource {
            kind: SourceKind::Task,
            id: task.meta.id.to_string(),
            title: task.meta.title.clone(),
        }],
        at,
        on: None,
    }
}

/// Open tasks due or scheduled today, and today's open occurrences of
/// recurring ones (a cancelled one left out, a moved one on its new day).
fn task_items(
    snapshot: &Snapshot,
    ledger: Option<&Ledger>,
    day: CivilDate,
    hide_all: bool,
) -> Result<Vec<(NowItem, usize)>> {
    let open = |status: &str| {
        status != TaskStatus::Completed.as_str() && status != TaskStatus::Archived.as_str()
    };
    let task_of = |id: &crate::id::ObjectId| match snapshot.get(id).map(|stored| &stored.entity) {
        Some(Entity::Task(task)) => Some(task),
        _ => None,
    };
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let calendar = calendar_between(snapshot, day, day)?;
    let listed = list_projection(snapshot);
    let candidates = calendar.items.iter().chain(listed.iter().filter(|item| {
        item.kind == EntityKind::Task
            && task_of(&item.source_id)
                .is_some_and(|task| task.recurrence.is_none() && task.due_on == Some(day))
    }));
    for item in candidates {
        if item.kind != EntityKind::Task || !open(&item.state) {
            continue;
        }
        let Some(task) = task_of(&item.source_id) else {
            continue;
        };
        if hidden_task(task, ledger, hide_all)? {
            continue;
        }
        let details = &task.meta.details;
        match &item.occurrence_id {
            None => {
                if !seen.insert(task.meta.id.to_string()) {
                    continue;
                }
                let why = if task.due_on == Some(day) {
                    NowWhy::DueToday
                } else {
                    NowWhy::ScheduledNow
                };
                let at = details.schedule.as_ref().and_then(|s| time_of(&s.time));
                found.push(task_item(
                    task,
                    task.meta.id.to_string(),
                    task.meta.title.clone(),
                    why,
                    at,
                ));
            }
            Some(occurrence) => {
                let key = occurrence.opaque_key().to_string();
                let overridden = details.overrides.get(&key);
                if overridden.is_some_and(|o| {
                    o.cancelled || o.date.as_deref().is_some_and(|d| d != day.to_string())
                }) {
                    continue;
                }
                if overridden
                    .and_then(|o| o.status.as_deref())
                    .is_some_and(|s| !open(s))
                {
                    continue;
                }
                if seen.insert(key.clone()) {
                    found.push(occurrence_item(task, key, overridden));
                }
            }
        }
    }
    // Occurrences moved onto today from another day.
    for entity in snapshot.entities() {
        let Entity::Task(task) = entity else { continue };
        let Some(rule) = &task.recurrence else {
            continue;
        };
        if !open(task.status.as_str()) || hidden_task(task, ledger, hide_all)? {
            continue;
        }
        for (key, overridden) in &task.meta.details.overrides {
            if overridden.cancelled
                || overridden.origin_date.is_empty()
                || overridden.date.as_deref() != Some(day.to_string().as_str())
                || seen.contains(key)
                || overridden.status.as_deref().is_some_and(|s| !open(s))
            {
                continue;
            }
            let origin: CivilDate = overridden.origin_date.parse()?;
            let Some(occurrence) = materialize_occurrences(&task.meta.id, rule, origin, origin)?
                .into_iter()
                .find(|o| o.id.opaque_key().as_str() == key)
            else {
                continue;
            };
            if task.completed_occurrences.contains(&occurrence.id) {
                continue;
            }
            seen.insert(key.clone());
            found.push(occurrence_item(task, key.clone(), Some(overridden)));
        }
    }
    Ok(found.into_iter().map(|item| (item, 0)).collect())
}

fn occurrence_item(
    task: &Task,
    key: String,
    overridden: Option<&crate::details::OccurrenceOverride>,
) -> NowItem {
    let schedule = if overridden.is_some_and(|o| o.all_day) {
        None
    } else {
        overridden
            .and_then(|o| o.schedule.as_ref())
            .or(task.meta.details.schedule.as_ref())
    };
    let title = overridden
        .and_then(|o| o.title.clone())
        .unwrap_or_else(|| task.meta.title.clone());
    task_item(
        task,
        key,
        title,
        NowWhy::ScheduledNow,
        schedule.and_then(|s| time_of(&s.time)),
    )
}

/// The open bundle's habits with no check-in today, in bundle order.
fn habit_items(
    ledger: &Ledger,
    bundle: Bundle,
    day: CivilDate,
    hide_all: bool,
) -> Result<Vec<(NowItem, usize)>> {
    let today = day.to_string();
    let bundle_source = NowSource {
        kind: SourceKind::Bundle,
        id: bundle.as_str().to_owned(),
        title: match bundle {
            Bundle::Morning => "Morning",
            Bundle::Evening => "Evening",
        }
        .to_owned(),
    };
    let mut found = Vec::new();
    // `habits` returns them in bundle order; the check-in is read whatever its
    // mark, so a privately checked habit never shows as open.
    for (step, habit) in ledger
        .habits(hide_all)?
        .into_iter()
        .filter(|habit| habit.spec.bundle == Some(bundle))
        .enumerate()
    {
        if ledger.day_check_in(&habit.id, &today, false)?.is_some() {
            continue;
        }
        found.push((
            NowItem {
                kind: NowKind::Habit,
                id: habit.id.to_string(),
                title: habit.spec.title.clone(),
                why: NowWhy::BundleOpen,
                rank: 0,
                sources: vec![
                    NowSource {
                        kind: SourceKind::Habit,
                        id: habit.id.to_string(),
                        title: habit.spec.title.clone(),
                    },
                    bundle_source.clone(),
                ],
                at: None,
                on: None,
            },
            step,
        ));
    }
    Ok(found)
}

/// The lots going off within the kitchen's span (the kitchen's Q5), each tied
/// to its lot row: Q5 names a lot by subject and date, and the lot with that
/// subject and date in stock is its row.
fn lot_items(
    ledger: &Ledger,
    day: CivilDate,
    settings: &NowSettings,
) -> Result<Vec<(NowItem, usize)>> {
    let table = ledger.goes_off_within(day, settings.goes_off_days, settings.hide_all)?;
    let column =
        |name: &str| {
            table.columns.iter().position(|c| c == name).ok_or_else(|| {
                KernelError::Ledger(format!("the goes-off table has no {name} column"))
            })
        };
    let (subject_at, expires_at) = (column("subject")?, column("expires_on")?);
    let mut stock: BTreeMap<(String, String), Vec<crate::ledger::kitchen::Lot>> = BTreeMap::new();
    for lot in ledger.stock(settings.hide_all)? {
        if let Some(expires) = lot.expires_on.clone() {
            stock
                .entry((lot.subject.clone(), expires))
                .or_default()
                .push(lot);
        }
    }
    let mut found = Vec::new();
    for (step, row) in table.rows.iter().enumerate() {
        let subject = row[subject_at].as_str().unwrap_or_default().to_owned();
        let expires = row[expires_at].as_str().unwrap_or_default().to_owned();
        let lot = stock
            .get_mut(&(subject.clone(), expires.clone()))
            .and_then(|lots| (!lots.is_empty()).then(|| lots.remove(0)))
            .ok_or_else(|| {
                KernelError::Ledger(format!("no lot in stock for {subject} on {expires}"))
            })?;
        found.push((
            NowItem {
                kind: NowKind::Lot,
                id: lot.id.to_string(),
                title: subject.clone(),
                why: NowWhy::GoesOff,
                rank: 0,
                sources: vec![NowSource {
                    kind: SourceKind::Lot,
                    id: lot.id.to_string(),
                    title: subject,
                }],
                at: None,
                on: expires.parse().ok(),
            },
            step,
        ));
    }
    Ok(found)
}

impl NowItem {
    /// The item as plain values, for the service and the report.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "kind": self.kind.as_str(),
            "id": self.id,
            "title": self.title,
            "why": self.why.as_str(),
            "rank": self.rank,
            "at": self.at.map(|t| t.to_string()),
            "on": self.on.map(|d| d.to_string()),
            "sources": self.sources.iter().map(|s| serde_json::json!({
                "kind": s.kind.as_str(), "id": s.id, "title": s.title
            })).collect::<Vec<_>>(),
        })
    }
}
