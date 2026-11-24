//! Habits on the ledger (S3, OC-12 result line 4). A habit is a ledger row of
//! kind `habit`, one current definition per habit id, edited by supersession;
//! a bundle's order is one current `bundle.order` row per bundle. Check-ins
//! are `check_in` rows written through `append_observation`. Adding the kind is
//! the design's section 5: one `kind` row, no schema change.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{Ledger, NewObservation};
use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::id::ObjectId;
use crate::model::DataClass;

pub const HABIT_KIND: &str = "habit";
pub const WRITER: &str = "habits";
pub const DEFINITION: &str = "habit.definition";
pub const ORDER: &str = "bundle.order";
pub const CHECK_IN: &str = "habit.check_in";
/// The longest heat-map span, in days.
pub const MAX_SPAN_DAYS: i64 = 366;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// A habit to form: a check-in is done (1) or missed (0).
    Form,
    /// A habit to break: a check-in is a count of times it happened.
    Break,
}

impl Direction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Break => "break",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "form" => Ok(Self::Form),
            "break" => Ok(Self::Break),
            _ => Err(KernelError::Ledger(format!(
                "unknown habit direction {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Bundle {
    Morning,
    Evening,
}

impl Bundle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Evening => "evening",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "morning" => Ok(Self::Morning),
            "evening" => Ok(Self::Evening),
            _ => Err(KernelError::Ledger(format!("unknown bundle {value:?}"))),
        }
    }

    fn category(self) -> String {
        format!("bundle:{}", self.as_str())
    }
}

/// What a habit is, as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct HabitSpec {
    pub title: String,
    pub direction: Direction,
    /// Per day: done 1 for a habit to form; the usual count for one to break.
    pub baseline: f64,
    pub bundle: Option<Bundle>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Habit {
    pub id: ObjectId,
    pub spec: HabitSpec,
    /// The current definition row.
    pub row: ObjectId,
    /// Existing planner references; never creates planner objects.
    pub hierarchy: Value,
}

/// A check-in value: done or missed for a habit to form, a count for one to break.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckIn {
    Done,
    Missed,
    Count(f64),
}

/// One cell of the heat map: the day's value against the habit's baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatCell {
    pub day: CivilDate,
    pub value: Option<f64>,
    /// `above`, `at`, `below`, or `no record`.
    pub against: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeatRow {
    pub habit: Habit,
    pub cells: Vec<HeatCell>,
}

/// A check-in row as `taken_back` reads it: its value, unit, the row it
/// retracts and the value of the row before it.
type TakenBackRow = (Option<f64>, Option<String>, Option<String>, Option<f64>);

fn view(hide_all: bool) -> &'static str {
    if hide_all {
        "visible_observation"
    } else {
        "current_observation"
    }
}

fn parse_extra(text: Option<String>) -> Result<Value> {
    text.map_or(Ok(Value::Null), |text| {
        serde_json::from_str(&text).map_err(|error| KernelError::Codec(error.to_string()))
    })
}

fn validate(spec: &HabitSpec) -> Result<()> {
    if spec.title.trim().is_empty() {
        return Err(KernelError::Ledger("a habit needs a title".into()));
    }
    if !spec.baseline.is_finite() || spec.baseline < 0.0 {
        return Err(KernelError::Ledger(
            "a habit baseline is a number of zero or more".into(),
        ));
    }
    if spec.direction == Direction::Form && spec.baseline > 1.0 {
        return Err(KernelError::Ledger(
            "a habit to form has a baseline between 0 and 1".into(),
        ));
    }
    Ok(())
}

fn empty_hierarchy() -> Value {
    json!({"category_id": null, "goal_id": null, "task_ids": []})
}
impl Ledger {
    /// Registers the `habit` kind once; a ledger migrated before S3 lacks it.
    fn ensure_habit_kind(&self) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO kind (kind, writer, default_class, unit, description) VALUES (?1, ?2, ?3, NULL, ?4)",
            params![
                HABIT_KIND,
                WRITER,
                DataClass::Internal.as_str(),
                "A habit's definition (direction, baseline, bundle) or a bundle's order; corrected by supersession"
            ],
        )?;
        Ok(())
    }

    /// The current habits, in bundle order (morning, evening, then unbundled by title).
    pub fn habits(&self, hide_all: bool) -> Result<Vec<Habit>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, item_id, extra FROM {} WHERE kind = ?1 AND subject = ?2",
            view(hide_all)
        ))?;
        let rows = statement.query_map(params![HABIT_KIND, DEFINITION], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut habits = Vec::new();
        for row in rows {
            let (row_id, habit_id, extra) = row?;
            let extra = parse_extra(extra)?;
            habits.push(Habit {
                id: habit_id.parse()?,
                row: row_id.parse()?,
                hierarchy: extra
                    .get("hierarchy")
                    .cloned()
                    .unwrap_or_else(empty_hierarchy),
                spec: HabitSpec {
                    title: extra["title"].as_str().unwrap_or_default().to_owned(),
                    direction: Direction::parse(extra["direction"].as_str().unwrap_or_default())?,
                    baseline: extra["baseline"].as_f64().unwrap_or(0.0),
                    bundle: extra["bundle"].as_str().map(Bundle::parse).transpose()?,
                },
            });
        }
        let mut rank: BTreeMap<String, usize> = BTreeMap::new();
        for bundle in [Bundle::Morning, Bundle::Evening] {
            for (step, id) in self.bundle_order(bundle)?.into_iter().enumerate() {
                rank.insert(id.to_string(), step);
            }
        }
        habits.sort_by(|a, b| {
            let key = |h: &Habit| {
                (
                    h.spec.bundle.map_or(2, |b| b as u8),
                    rank.get(h.id.as_str()).copied().unwrap_or(usize::MAX),
                    h.spec.title.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        Ok(habits)
    }

    pub fn habit(&self, id: &ObjectId) -> Result<Habit> {
        self.habits(false)?
            .into_iter()
            .find(|habit| &habit.id == id)
            .ok_or_else(|| KernelError::Ledger(format!("no habit {id}")))
    }

    fn order_row(&self, bundle: Bundle) -> Result<Option<(ObjectId, Vec<ObjectId>)>> {
        let found: Option<(String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT id, extra FROM current_observation WHERE kind = ?1 AND subject = ?2 AND category = ?3",
                params![HABIT_KIND, ORDER, bundle.category()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((row, extra)) = found else {
            return Ok(None);
        };
        let extra = parse_extra(extra)?;
        let order = extra["order"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::parse)
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Some((row.parse()?, order)))
    }

    /// The bundle's habits in their set order.
    pub fn bundle_order(&self, bundle: Bundle) -> Result<Vec<ObjectId>> {
        Ok(self
            .order_row(bundle)?
            .map(|(_, order)| order)
            .unwrap_or_default())
    }

    fn write_order(&mut self, bundle: Bundle, order: &[ObjectId], day: &str) -> Result<()> {
        let row = NewObservation {
            kind: HABIT_KIND.into(),
            observed_on: day.into(),
            category: Some(bundle.category()),
            subject: ORDER.into(),
            writer: WRITER.into(),
            extra: Some(
                json!({"bundle": bundle.as_str(), "order": order.iter().map(ObjectId::as_str).collect::<Vec<_>>()}),
            ),
            ..NewObservation::default()
        };
        match self.order_row(bundle)? {
            Some((old, _)) => self.supersede(&old, &row)?,
            None => self.append_observation(&row)?,
        };
        Ok(())
    }

    /// Sets a bundle's order. `order` must name each habit of the bundle once.
    pub fn set_bundle_order(
        &mut self,
        bundle: Bundle,
        order: &[ObjectId],
        day: &str,
    ) -> Result<()> {
        let mut members: Vec<String> = self
            .habits(false)?
            .into_iter()
            .filter(|h| h.spec.bundle == Some(bundle))
            .map(|h| h.id.to_string())
            .collect();
        let mut given: Vec<String> = order.iter().map(ToString::to_string).collect();
        members.sort();
        given.sort();
        if members != given {
            return Err(KernelError::Ledger(format!(
                "the {} bundle order must name each of its {} habits once",
                bundle.as_str(),
                members.len()
            )));
        }
        self.write_order(bundle, order, day)
    }

    fn definition_extra(&self, row: &ObjectId) -> Result<(Value, bool)> {
        let (extra, private): (Option<String>, bool) = self.connection.query_row(
            "SELECT extra, coalesce(private, 0) FROM observation WHERE id = ?1",
            [row.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((parse_extra(extra)?, private))
    }

    /// Supersedes a definition with validated planner references. The service
    /// validates existence and parent relationships against its planner snapshot.
    /// An exact retry leaves the current definition identity unchanged.
    pub fn bind_habit(&mut self, id: &ObjectId, hierarchy: &Value, day: &str) -> Result<Habit> {
        day.parse::<CivilDate>()?;
        let habit = self.habit(id)?;
        for field in ["category_id", "goal_id"] {
            if !hierarchy[field].is_null() {
                hierarchy[field]
                    .as_str()
                    .ok_or_else(|| KernelError::Ledger(format!("{field} must be an id or null")))?
                    .parse::<ObjectId>()?;
            }
        }
        let tasks = hierarchy["task_ids"]
            .as_array()
            .ok_or_else(|| KernelError::Ledger("task_ids must be an ordered list".into()))?;
        let mut seen = std::collections::BTreeSet::new();
        for task in tasks {
            let id: ObjectId = task
                .as_str()
                .ok_or_else(|| KernelError::Ledger("task_ids must contain ids".into()))?
                .parse()?;
            if !seen.insert(id) {
                return Err(KernelError::Ledger(
                    "task_ids must name each action once".into(),
                ));
            }
        }
        if habit.hierarchy == *hierarchy {
            return Ok(habit);
        }
        let (mut extra, private) = self.definition_extra(&habit.row)?;
        extra["hierarchy"] = hierarchy.clone();
        let row = NewObservation {
            kind: HABIT_KIND.into(),
            observed_on: day.into(),
            item_id: Some(id.to_string()),
            category: habit.spec.bundle.map(Bundle::category),
            subject: DEFINITION.into(),
            quantity: Some(habit.spec.baseline),
            writer: WRITER.into(),
            extra: Some(extra),
            private,
            ..NewObservation::default()
        };
        self.supersede(&habit.row, &row)?;
        self.habit(id)
    }

    /// Defines a new habit (`id` none) or edits one by superseding its definition.
    /// A habit joining a bundle goes last in it; leaving one removes it there.
    pub fn define_habit(
        &mut self,
        id: Option<&ObjectId>,
        spec: &HabitSpec,
        day: &str,
    ) -> Result<Habit> {
        validate(spec)?;
        self.ensure_habit_kind()?;
        let previous = id.map(|id| self.habit(id)).transpose()?;
        let habit_id = match id {
            Some(id) => id.clone(),
            None => ObjectId::mint("habit")?,
        };
        let (mut extra, private) = previous
            .as_ref()
            .map(|h| self.definition_extra(&h.row))
            .transpose()?
            .unwrap_or((json!({}), false));
        extra["title"] = json!(spec.title.trim());
        extra["direction"] = json!(spec.direction.as_str());
        extra["baseline"] = json!(spec.baseline);
        extra["bundle"] = json!(spec.bundle.map(Bundle::as_str));
        let hierarchy = extra
            .get("hierarchy")
            .cloned()
            .unwrap_or_else(empty_hierarchy);
        let row = NewObservation {
            kind: HABIT_KIND.into(),
            observed_on: day.into(),
            item_id: Some(habit_id.to_string()),
            category: spec.bundle.map(Bundle::category),
            subject: DEFINITION.into(),
            quantity: Some(spec.baseline),
            writer: WRITER.into(),
            extra: Some(extra),
            private,
            ..NewObservation::default()
        };
        let row_id = match &previous {
            Some(old) => self.supersede(&old.row, &row)?,
            None => self.append_observation(&row)?,
        };
        let old_bundle = previous.as_ref().and_then(|h| h.spec.bundle);
        if old_bundle != spec.bundle {
            if let Some(bundle) = old_bundle {
                let order: Vec<ObjectId> = self
                    .bundle_order(bundle)?
                    .into_iter()
                    .filter(|h| h != &habit_id)
                    .collect();
                self.write_order(bundle, &order, day)?;
            }
            if let Some(bundle) = spec.bundle {
                let mut order = self.bundle_order(bundle)?;
                order.push(habit_id.clone());
                self.write_order(bundle, &order, day)?;
            }
        }
        Ok(Habit {
            id: habit_id,
            spec: HabitSpec {
                title: spec.title.trim().to_owned(),
                ..spec.clone()
            },
            row: row_id,
            hierarchy,
        })
    }

    fn check_in_row(habit: &Habit, day: &str, value: CheckIn) -> Result<NewObservation> {
        let quantity = match (habit.spec.direction, value) {
            (Direction::Form, CheckIn::Done) => 1.0,
            (Direction::Form, CheckIn::Missed) => 0.0,
            (Direction::Break, CheckIn::Count(n)) if n.is_finite() && n >= 0.0 => n,
            (Direction::Form, _) => {
                return Err(KernelError::Ledger(
                    "a habit to form is checked in as done or missed".into(),
                ))
            }
            (Direction::Break, _) => {
                return Err(KernelError::Ledger(
                    "a habit to break is checked in as a count of zero or more".into(),
                ))
            }
        };
        Ok(NewObservation {
            kind: "check_in".into(),
            observed_on: day.into(),
            item_id: Some(habit.id.to_string()),
            category: habit.spec.bundle.map(Bundle::category),
            subject: CHECK_IN.into(),
            quantity: Some(quantity),
            unit: Some(match habit.spec.direction {
                Direction::Form => "done".into(),
                Direction::Break => "count".into(),
            }),
            writer: WRITER.into(),
            extra: Some(json!({
                "direction": habit.spec.direction.as_str(),
                "bundle": habit.spec.bundle.map(Bundle::as_str),
            })),
            ..NewObservation::default()
        })
    }

    /// Records one check-in for a habit on a day. A second one for the same day
    /// is refused: a check-in is corrected with `correct_check_in`.
    /// A day whose check-in was retracted takes a new entry as the next row of
    /// that chain.
    pub fn record_check_in(
        &mut self,
        habit_id: &ObjectId,
        day: &str,
        value: CheckIn,
    ) -> Result<ObjectId> {
        let habit = self.habit(habit_id)?;
        let existing: Option<(String, Option<f64>)> = self
            .connection
            .query_row(
                "SELECT id, quantity FROM current_observation WHERE kind = 'check_in' AND subject = ?1 AND item_id = ?2 AND observed_on = ?3",
                params![CHECK_IN, habit_id.as_str(), day],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let row = Self::check_in_row(&habit, day, value)?;
        match existing {
            Some((retraction, None)) => self.supersede(&retraction.parse()?, &row),
            Some((existing, Some(_))) => Err(KernelError::Conflict(format!(
                "{habit_id} already has check-in {existing} on {day}; correct it instead"
            ))),
            None => self.append_observation(&row),
        }
    }

    /// Takes back a current check-in: one more row in its chain with no value
    /// and `extra.retracts` naming it, so the day reads as no entry. A row
    /// already corrected or taken back is refused.
    pub fn retract_check_in(&mut self, check_in: &ObjectId) -> Result<ObjectId> {
        type HabitObservationRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            bool,
            Option<f64>,
        );
        let found: Option<HabitObservationRow> = self
            .connection
            .query_row(
                "SELECT item_id, observed_on, category, superseded_by, coalesce(private, 0), quantity FROM observation \
                 WHERE id = ?1 AND kind = 'check_in' AND subject = ?2",
                params![check_in.as_str(), CHECK_IN],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()?;
        let (habit_id, day, category, superseded_by, private, quantity) =
            found.ok_or_else(|| KernelError::Ledger(format!("no habit check-in {check_in}")))?;
        if superseded_by.is_some() {
            return Err(KernelError::Conflict(
                "this check-in has already been changed, so it cannot be taken back".into(),
            ));
        }
        if quantity.is_none() {
            return Err(KernelError::Conflict(
                "this check-in has already been taken back".into(),
            ));
        }
        let habit = self.habit(&habit_id.parse()?)?;
        let row = NewObservation {
            kind: "check_in".into(),
            observed_on: day,
            item_id: Some(habit_id),
            category,
            subject: CHECK_IN.into(),
            quantity: None,
            unit: Some(match habit.spec.direction {
                Direction::Form => "done".into(),
                Direction::Break => "count".into(),
            }),
            writer: WRITER.into(),
            private,
            extra: Some(json!({"retracts": check_in.to_string()})),
            ..NewObservation::default()
        };
        self.supersede(check_in, &row)
    }

    /// The check-in a taken-back row hides: for a retraction the row it
    /// retracts; for a habit to form unticked (done corrected to missed) the
    /// done row before it. Any other row hides nothing.
    fn taken_back(&self, row: &ObjectId) -> Result<Option<String>> {
        let found: Option<TakenBackRow> = self
            .connection
            .query_row(
                "SELECT r.quantity, r.unit, json_extract(r.extra,'$.retracts'), \
                 (SELECT p.quantity FROM observation p WHERE p.superseded_by = r.id) \
                 FROM observation r WHERE r.id = ?1 AND r.kind = 'check_in' AND r.subject = ?2",
                params![row.as_str(), CHECK_IN],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((quantity, unit, retracts, before)) = found else {
            return Err(KernelError::Ledger(format!("no habit check-in {row}")));
        };
        Ok(match (quantity, retracts) {
            (None, Some(retracted)) => Some(retracted),
            (Some(q), None)
                if q == 0.0 && unit.as_deref() == Some("done") && before == Some(1.0) =>
            {
                self.connection
                    .query_row(
                        "SELECT id FROM observation WHERE superseded_by = ?1",
                        [row.as_str()],
                        |r| r.get(0),
                    )
                    .optional()?
            }
            _ => None,
        })
    }

    /// The day's current check-in of a habit when it is a take-back that
    /// `restore_check_in` can bring back (a retraction, or a done unticked).
    pub fn day_restorable(
        &self,
        habit_id: &ObjectId,
        day: &str,
        hide_all: bool,
    ) -> Result<Option<ObjectId>> {
        let found: Option<String> = self
            .connection
            .query_row(
                &format!(
                    "SELECT id FROM {} WHERE kind = 'check_in' AND subject = ?1 AND item_id = ?2 AND observed_on = ?3",
                    view(hide_all)
                ),
                params![CHECK_IN, habit_id.as_str(), day],
                |row| row.get(0),
            )
            .optional()?;
        match found {
            Some(id) => {
                let id: ObjectId = id.parse()?;
                Ok(self.taken_back(&id)?.map(|_| id))
            }
            None => Ok(None),
        }
    }

    /// Takes back a take-back: the check-in it hid (the retracted one, or the
    /// done before an untick) returns whole as the next row of its chain, so
    /// the day reads as before. Only the day's current row can be taken back;
    /// no row is deleted.
    pub fn restore_check_in(&mut self, retraction: &ObjectId) -> Result<ObjectId> {
        let superseded_by: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT superseded_by FROM observation WHERE id = ?1 AND kind = 'check_in' AND subject = ?2",
                params![retraction.as_str(), CHECK_IN],
                |row| row.get(0),
            )
            .optional()?;
        let superseded_by = superseded_by
            .ok_or_else(|| KernelError::Ledger(format!("no habit check-in {retraction}")))?;
        let Some(retracted) = self.taken_back(retraction)? else {
            return Err(KernelError::Conflict(
                "this check-in was not taken back".into(),
            ));
        };
        if superseded_by.is_some() {
            return Err(KernelError::Conflict(
                "this day has a newer entry, so the retraction cannot be taken back".into(),
            ));
        }
        let row = self.copy_of(&retracted.parse()?)?;
        self.supersede(retraction, &row)
    }

    /// The current check-in of a habit on a day, if there is one. With
    /// `hide_all`, a private check-in is not returned.
    pub fn day_check_in(
        &self,
        habit_id: &ObjectId,
        day: &str,
        hide_all: bool,
    ) -> Result<Option<ObjectId>> {
        let found: Option<String> = self
            .connection
            .query_row(
                &format!(
                    "SELECT id FROM {} WHERE kind = 'check_in' AND subject = ?1 AND item_id = ?2 AND observed_on = ?3 \
                     AND quantity IS NOT NULL",
                    view(hide_all)
                ),
                params![CHECK_IN, habit_id.as_str(), day],
                |row| row.get(0),
            )
            .optional()?;
        found.map(|id| id.parse()).transpose()
    }

    /// Corrects a current habit check-in by supersession; the day and habit stay.
    pub fn correct_check_in(&mut self, check_in: &ObjectId, value: CheckIn) -> Result<ObjectId> {
        let found: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT item_id, observed_on FROM observation WHERE id = ?1 AND kind = 'check_in' AND subject = ?2",
                params![check_in.as_str(), CHECK_IN],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (habit_id, day) =
            found.ok_or_else(|| KernelError::Ledger(format!("no habit check-in {check_in}")))?;
        let habit = self.habit(&habit_id.parse()?)?;
        let row = Self::check_in_row(&habit, &day, value)?;
        self.supersede(check_in, &row)
    }

    /// Per habit and day from `from` through `through`, the day's value against
    /// the habit's baseline. With `hide_all`, private habits and rows are left out.
    pub fn heat_map(
        &self,
        from: CivilDate,
        through: CivilDate,
        hide_all: bool,
    ) -> Result<Vec<HeatRow>> {
        let span = from.days_until(through) + 1;
        if !(1..=MAX_SPAN_DAYS).contains(&span) {
            return Err(KernelError::Ledger(format!(
                "a heat-map span is 1 to {MAX_SPAN_DAYS} days, not {span}"
            )));
        }
        let mut statement = self.connection.prepare(&format!(
            "SELECT observed_on, quantity FROM {} WHERE kind = 'check_in' AND subject = ?1 AND item_id = ?2 \
             AND observed_on BETWEEN ?3 AND ?4",
            view(hide_all)
        ))?;
        let mut rows = Vec::new();
        for habit in self.habits(hide_all)? {
            let mut values: BTreeMap<String, f64> = BTreeMap::new();
            let found = statement.query_map(
                params![
                    CHECK_IN,
                    habit.id.as_str(),
                    from.to_string(),
                    through.to_string()
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)),
            )?;
            for row in found {
                let (day, quantity) = row?;
                if let Some(quantity) = quantity {
                    *values.entry(day).or_insert(0.0) += quantity;
                }
            }
            let mut cells = Vec::new();
            for offset in 0..span {
                let day = from.add_days(offset)?;
                let value = values.get(&day.to_string()).copied();
                let against = match value {
                    None => "no record",
                    Some(v) if (v - habit.spec.baseline).abs() < 1e-9 => "at",
                    Some(v) if v > habit.spec.baseline => "above",
                    Some(_) => "below",
                };
                cells.push(HeatCell {
                    day,
                    value,
                    against,
                });
            }
            rows.push(HeatRow { habit, cells });
        }
        Ok(rows)
    }
}
