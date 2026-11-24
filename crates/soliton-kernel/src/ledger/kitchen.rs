//! The kitchen on the ledger (S4b, OC-12 result line 5): stock lots, equipment,
//! the cook profile and large mise en place plans, as S4_DESIGN.md (T007)
//! fixes them. Every change to a lot, a piece of equipment or a profile entry
//! supersedes its current row; nothing is edited in place. No model is used:
//! lots are typed or come from a purchase line, a lot may carry his picture and
//! its weight, and a plan a local model drafted comes back here as `inferred`
//! dishes for his review (the call itself is the service's, behind his setting).

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{Ledger, NewObservation, Table};
use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::ObjectId;

pub const WRITER: &str = "kitchen";
pub const INVENTORY: &str = "inventory";
pub const EQUIPMENT: &str = "equipment";
pub const COOK_PROFILE: &str = "cook_profile";
pub const BATCH_PLAN: &str = "batch_plan";
pub const MEAL: &str = "meal";
/// The category of a meal cooked at home (Q2 and Q3 count it).
pub const MEAL_HOME: &str = "food:home";
/// The longest look-ahead for what goes off, in days.
pub const MAX_HORIZON_DAYS: i64 = 366;
/// The cook-profile subject whose value lists ingredients a plan never uses,
/// separated by commas ("coriander, peanuts").
pub const EXCLUDE: &str = "exclude";
/// The units a weight is given in; any other unit is a count or a volume.
pub const MASS_UNITS: [&str; 4] = ["g", "kg", "lb", "oz"];
/// Where a lot's picture is kept, under the workspace.
pub const PICTURES_DIR: &str = "kitchen/pictures";
/// The picture types a lot takes.
pub const PICTURE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
pub const MAX_PICTURE_BYTES: u64 = 32 * 1024 * 1024;
/// The longest span of meals one batch is for, in days.
pub const MAX_COVERAGE_DAYS: i64 = 366;
/// The standing of a dish a model drafted, until he takes it.
pub const DRAFTED: &str = "inferred";
/// The category prefix of a food purchase line.
pub const FOOD_CATEGORY: &str = "food:";

/// A unit as written, mass units lower-cased ("KG" is "kg").
fn unit_of(value: &str) -> Result<String> {
    let unit = named(value, "a lot's unit")?;
    Ok(MASS_UNITS
        .iter()
        .find(|mass| mass.eq_ignore_ascii_case(&unit))
        .map_or(unit, |mass| (*mass).to_owned()))
}

/// The picture's extension, lower-cased, when it is one a lot takes.
fn picture_extension(picture: &Path) -> Result<String> {
    picture
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .filter(|e| PICTURE_EXTENSIONS.contains(&e.as_str()))
        .ok_or_else(|| KernelError::Ledger("a lot's picture is a png, jpg or webp file".into()))
}

/// The picture type the bytes are, by their signature, whatever the file is
/// called: png, jpg (jpeg) or webp, else none.
pub fn picture_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Reads the picture a lot is to carry: a regular file, not empty, within the
/// limit, named as a picture and holding one (its signature, not its name). The
/// extension returned is the one its contents are.
fn read_picture(picture: &Path) -> Result<(Vec<u8>, String)> {
    picture_extension(picture)?;
    let meta = fs::metadata(picture)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_PICTURE_BYTES {
        return Err(KernelError::Ledger(format!(
            "a lot's picture is a file of 1 byte to {} MB",
            MAX_PICTURE_BYTES / (1024 * 1024)
        )));
    }
    let bytes = fs::read(picture)?;
    let kind = picture_kind(&bytes)
        .ok_or_else(|| KernelError::Ledger("this file is not a picture".into()))?;
    Ok((bytes, kind.to_owned()))
}

/// Checks a file a lot is to carry, as `read_picture` does, writing nothing:
/// its picture type.
pub fn check_picture(picture: &Path) -> Result<String> {
    read_picture(picture).map(|(_, kind)| kind)
}

/// The ingredients a profile entry `exclude` names, trimmed and lower-cased.
fn excluded(profile: &[ProfileEntry]) -> Vec<String> {
    profile
        .iter()
        .filter(|entry| entry.subject.trim().eq_ignore_ascii_case(EXCLUDE))
        .filter_map(|entry| entry.value.as_deref())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The excluded ingredient a lot's name holds as a whole word, if any.
fn excluded_in<'a>(subject: &str, excluded: &'a [String]) -> Option<&'a str> {
    let subject = subject.to_lowercase();
    let words: Vec<&str> = subject
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    excluded.iter().map(String::as_str).find(|name| {
        let wanted: Vec<&str> = name
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        !wanted.is_empty()
            && words
                .windows(wanted.len())
                .any(|window| window == wanted.as_slice())
    })
}

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

fn amount(value: f64, what: &str) -> Result<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(KernelError::Ledger(format!(
            "{what} is a number of zero or more"
        )))
    }
}

fn named(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(KernelError::Ledger(format!("{what} needs a name")));
    }
    Ok(value.to_owned())
}

/// How a lot came in. A lot of either source may carry a picture; reading what
/// a photo shows needs his local model and is not done here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LotSource {
    Typed,
    Receipt,
}

impl LotSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Receipt => "receipt",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "typed" => Ok(Self::Typed),
            "receipt" => Ok(Self::Receipt),
            _ => Err(KernelError::Ledger(format!(
                "unknown lot source {value:?} (typed or receipt)"
            ))),
        }
    }
}

/// A lot as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct LotSpec {
    pub subject: String,
    pub category: Option<String>,
    pub quantity: f64,
    pub unit: String,
    pub expires_on: Option<CivilDate>,
    pub location: Option<String>,
    pub source: LotSource,
    /// The purchase line it came from, when the source is a receipt.
    pub purchase_line: Option<ObjectId>,
}

/// One current food purchase line no lot has come from yet.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferedLine {
    pub id: ObjectId,
    pub observed_on: String,
    pub subject: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    /// `statement:<bank>:<period>#<line>` or what wrote the line.
    pub source: Option<String>,
}

/// One current lot.
#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    pub id: ObjectId,
    pub subject: String,
    pub category: Option<String>,
    pub quantity: f64,
    pub unit: Option<String>,
    pub expires_on: Option<String>,
    pub location: Option<String>,
    pub source: Option<String>,
    pub observed_on: String,
    /// The lot carries a picture kept under the workspace.
    pub has_picture: bool,
    /// The purchase line the lot came from.
    pub purchase_line: Option<String>,
    extra: Value,
}

impl Lot {
    /// The lot's weight and its unit, when its quantity is given in a mass unit.
    #[must_use]
    pub fn weight(&self) -> Option<(f64, &str)> {
        let unit = self.unit.as_deref()?;
        MASS_UNITS.contains(&unit).then_some((self.quantity, unit))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Equipment {
    pub id: ObjectId,
    pub name: String,
    pub count: f64,
    pub capacity: Option<String>,
}

/// One standing constraint of the cook.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileEntry {
    pub id: ObjectId,
    pub subject: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub value: Option<String>,
}

/// One dish of a plan, as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct DishSpec {
    pub name: String,
    pub portions: f64,
    /// 0 is the mise en place, then cooking phases.
    pub phase: u32,
    pub equipment: Vec<String>,
    /// Lots drawn on and the amount of each, in the lot's unit.
    pub uses: Vec<(ObjectId, f64)>,
}

/// What writing a plan did and what it read.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanOutcome {
    pub plan_id: String,
    pub rows: Vec<ObjectId>,
    /// The days of meals the batch is for, when he chose them.
    pub coverage_days: Option<i64>,
    pub portions: f64,
    /// The cook profile the plan was read against.
    pub constraints: Vec<ProfileEntry>,
    /// Plain-word notes: lots going off that the plan leaves unplanned, and
    /// portions against the profile's portions per week.
    pub notes: Vec<String>,
    /// Q5 after the plan was written.
    pub goes_off: Table,
    /// Each lot the plan draws on, soonest to go off first (no date last).
    pub ingredients: Vec<PlanIngredient>,
}

/// One lot a written plan draws on, summed over its dishes.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanIngredient {
    pub lot: ObjectId,
    pub subject: String,
    pub quantity: f64,
    pub unit: Option<String>,
    pub expires_on: Option<String>,
    /// The lot goes off within the plan's look-ahead.
    pub goes_off_soon: bool,
}

/// One dish of a written plan, as the plans read gives it back.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedDish {
    pub id: ObjectId,
    pub name: String,
    pub portions: f64,
    pub phase: u32,
    pub equipment: Vec<String>,
    pub uses: Vec<(ObjectId, f64)>,
    /// The meal written when the dish was cooked.
    pub cooked: Option<ObjectId>,
    /// `observed` when he wrote it, `inferred` when a model drafted it.
    pub standing: String,
    /// The rule that drafted the dish, when a model did.
    pub derived_by: Option<String>,
}

/// One meal: a dish cooked from a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Meal {
    pub id: ObjectId,
    pub day: String,
    pub name: String,
    pub portions: f64,
    pub category: Option<String>,
    pub plan_id: Option<String>,
}

/// What cooking a dish wrote: the meal, the dish's new plan row and each lot
/// drawn on as (the row superseded, the row now current).
#[derive(Clone, Debug, PartialEq)]
pub struct CookOutcome {
    pub meal: ObjectId,
    pub dish: ObjectId,
    pub lots: Vec<(ObjectId, ObjectId)>,
}

/// One current plan: the `batch_plan` rows under one plan id.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchPlan {
    pub plan_id: String,
    pub day: String,
    pub portions: f64,
    /// The days of meals the batch is for, when he chose them.
    pub coverage_days: Option<i64>,
    pub dishes: Vec<PlannedDish>,
}

// S4a Q5 (T007 queries.sql), with today and N as parameters.
const Q5: &str = "WITH p(today, n) AS (VALUES (?1, ?2)),
planned AS (
  SELECT json_extract(u.value,'$.lot') AS lot, sum(json_extract(u.value,'$.qty')) AS qty
  FROM current_observation b, json_each(b.extra,'$.uses') u
  WHERE b.kind = 'batch_plan' AND json_extract(b.extra,'$.cooked') IS NULL GROUP BY lot)
SELECT i.subject, i.quantity AS on_hand, i.unit, json_extract(i.extra,'$.expires_on') AS expires_on,
       coalesce(planned.qty, 0) AS planned, i.quantity - coalesce(planned.qty, 0) AS unplanned
FROM current_observation i JOIN p LEFT JOIN planned ON planned.lot = i.id
WHERE i.kind = 'inventory' AND i.quantity > 0
  AND json_extract(i.extra,'$.expires_on') <= date(p.today, '+' || p.n || ' days')
ORDER BY expires_on, i.subject";

impl Ledger {
    fn lot_from_row(
        id: String,
        subject: String,
        category: Option<String>,
        quantity: Option<f64>,
        unit: Option<String>,
        observed_on: String,
        extra: Option<String>,
    ) -> Result<Lot> {
        let extra = parse_extra(extra)?;
        let text = |key: &str| extra[key].as_str().map(str::to_owned);
        Ok(Lot {
            id: id.parse()?,
            subject,
            category,
            quantity: quantity.unwrap_or(0.0),
            unit,
            expires_on: text("expires_on"),
            location: text("location"),
            source: text("source"),
            observed_on,
            has_picture: extra["picture"].is_string(),
            purchase_line: text("purchase_line"),
            extra,
        })
    }

    /// The current lots with something left, soonest to go off first.
    pub fn stock(&self, hide_all: bool) -> Result<Vec<Lot>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, subject, category, quantity, unit, observed_on, extra FROM {} \
             WHERE kind = ?1 AND quantity > 0 \
             ORDER BY coalesce(json_extract(extra,'$.expires_on'),'9999-12-31'), subject, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map([INVENTORY], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?;
        let mut lots = Vec::new();
        for row in rows {
            let (id, subject, category, quantity, unit, day, extra) = row?;
            lots.push(Self::lot_from_row(
                id, subject, category, quantity, unit, day, extra,
            )?);
        }
        Ok(lots)
    }

    /// One current lot, whatever its quantity.
    pub fn lot(&self, id: &ObjectId) -> Result<Lot> {
        let found = self
            .connection
            .query_row(
                "SELECT id, subject, category, quantity, unit, observed_on, extra FROM current_observation \
                 WHERE id = ?1 AND kind = ?2",
                params![id.as_str(), INVENTORY],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let (id, subject, category, quantity, unit, day, extra) =
            found.ok_or_else(|| KernelError::Ledger(format!("no current lot {id}")))?;
        Self::lot_from_row(id, subject, category, quantity, unit, day, extra)
    }

    /// Adds one lot to stock.
    pub fn add_stock(&self, spec: &LotSpec, day: &str) -> Result<ObjectId> {
        let row = self.lot_row(spec, day)?;
        self.append_observation(&row)
    }

    /// Adds one lot to stock carrying his picture: the picture is copied to
    /// `<workspace>/kitchen/pictures/<lot id>.<ext>` (0600) and the lot keeps
    /// its name there and its sha256. The weight is the lot's quantity in a
    /// mass unit. All or nothing: no lot without its copy, no copy without its lot.
    pub fn add_stock_with_picture(
        &mut self,
        spec: &LotSpec,
        picture: &Path,
        day: &str,
    ) -> Result<ObjectId> {
        let (bytes, extension) = read_picture(picture)?;
        let row = self.lot_row(spec, day)?;
        let workspace = self.workspace()?;
        let transaction = self.connection.transaction()?;
        let id = Self::insert(&transaction, &row)?;
        let kept = attach_picture(&transaction, &workspace, &id, &bytes, &extension)?;
        if let Err(error) = transaction.commit() {
            let _ = fs::remove_file(&kept);
            return Err(error.into());
        }
        Ok(id)
    }

    /// Gives a lot a picture, or a new one, by superseding it; the earlier
    /// picture stays with the row that carried it.
    pub fn set_picture(&mut self, lot: &ObjectId, picture: &Path, day: &str) -> Result<ObjectId> {
        let (bytes, extension) = read_picture(picture)?;
        let lot = self.lot(lot)?;
        let source_ref: Option<String> = self.connection.query_row(
            "SELECT source_ref FROM observation WHERE id = ?1",
            [lot.id.as_str()],
            |row| row.get(0),
        )?;
        let mut extra = if lot.extra.is_object() {
            lot.extra.clone()
        } else {
            json!({})
        };
        if let Some(map) = extra.as_object_mut() {
            map.remove("picture");
            map.remove("sha256");
        }
        let row = NewObservation {
            kind: INVENTORY.into(),
            observed_on: day.into(),
            category: lot.category.clone(),
            subject: lot.subject.clone(),
            quantity: Some(lot.quantity),
            unit: lot.unit.clone(),
            writer: WRITER.into(),
            source_ref,
            extra: Some(extra),
            ..NewObservation::default()
        };
        let workspace = self.workspace()?;
        let transaction = self.connection.transaction()?;
        let id = supersede_in(&transaction, lot.id.as_str(), &row)?;
        let kept = attach_picture(&transaction, &workspace, &id, &bytes, &extension)?;
        if let Err(error) = transaction.commit() {
            let _ = fs::remove_file(&kept);
            return Err(error.into());
        }
        Ok(id)
    }

    /// Where a lot's picture is kept, for the window to show; never shown as text.
    pub fn picture_path(&self, lot: &Lot) -> Result<Option<PathBuf>> {
        match lot.extra["picture"].as_str() {
            Some(name) => Ok(Some(self.workspace()?.join(name))),
            None => Ok(None),
        }
    }

    /// The workspace the ledger file sits in.
    fn workspace(&self) -> Result<PathBuf> {
        self.path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| KernelError::Ledger("the ledger has no workspace".into()))
    }

    /// The row a new lot is written as, after its checks.
    fn lot_row(&self, spec: &LotSpec, day: &str) -> Result<NewObservation> {
        let subject = named(&spec.subject, "a lot")?;
        let unit = unit_of(&spec.unit)?;
        let quantity = amount(spec.quantity, "a lot's quantity")?;
        if spec.source == LotSource::Receipt && spec.purchase_line.is_none() {
            return Err(KernelError::Ledger(
                "a lot from a receipt names its purchase line".into(),
            ));
        }
        if let Some(line) = &spec.purchase_line {
            let kind: Option<String> = self
                .connection
                .query_row(
                    "SELECT kind FROM observation WHERE id = ?1",
                    [line.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if kind.as_deref() != Some("purchase_line") {
                return Err(KernelError::Ledger(format!("no purchase line {line}")));
            }
            if let Some(first) = self.lot_of_purchase(line)? {
                return Err(KernelError::Conflict(format!(
                    "that purchase is already in stock as {first}"
                )));
            }
        }
        let mut extra = json!({"source": spec.source.as_str()});
        if let Some(date) = spec.expires_on {
            extra["expires_on"] = json!(date.to_string());
        }
        if let Some(location) = spec
            .location
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            extra["location"] = json!(location);
        }
        if let Some(line) = &spec.purchase_line {
            extra["purchase_line"] = json!(line.as_str());
            let statement: Option<String> = self.connection.query_row(
                "SELECT source_ref FROM observation WHERE id = ?1",
                [line.as_str()],
                |row| row.get(0),
            )?;
            if let Some(statement) = statement {
                extra["purchase_source"] = json!(statement);
            }
        }
        Ok(NewObservation {
            kind: INVENTORY.into(),
            observed_on: day.into(),
            category: spec.category.clone(),
            subject,
            quantity: Some(quantity),
            unit: Some(unit),
            writer: WRITER.into(),
            source_ref: spec.purchase_line.as_ref().map(ToString::to_string),
            extra: Some(extra),
            ..NewObservation::default()
        })
    }

    /// The current lot that came from a purchase line, by the line's id or,
    /// for a line recategorised since, by its statement source.
    pub fn lot_of_purchase(&self, line: &ObjectId) -> Result<Option<ObjectId>> {
        let found: Option<String> = self
            .connection
            .query_row(
                "SELECT i.id FROM current_observation i, observation p WHERE p.id = ?2 AND i.kind = ?1                    AND (json_extract(i.extra,'$.purchase_line') = p.id                         OR (p.source_ref IS NOT NULL AND json_extract(i.extra,'$.purchase_source') = p.source_ref))                  ORDER BY i.recorded_at, i.id LIMIT 1",
                params![INVENTORY, line.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        found.map(|id| id.parse()).transpose()
    }

    /// The current food purchase lines (category `food:*`) that no current
    /// lot names as its source: what Finance offers the stock. Read through
    /// the visibility view while hide-all is on.
    pub fn lots_from_purchases(&self, hide_all: bool) -> Result<Vec<OfferedLine>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT p.id, p.observed_on, coalesce(json_extract(p.extra,'$.merchant'), p.subject), p.category,                     coalesce(p.amount_minor, 0), coalesce(p.currency, ''), p.source_ref FROM {} p              WHERE p.kind = 'purchase_line' AND substr(p.category, 1, ?2) = ?3                AND NOT EXISTS (SELECT 1 FROM current_observation i WHERE i.kind = ?1                    AND (json_extract(i.extra,'$.purchase_line') = p.id                         OR (p.source_ref IS NOT NULL AND json_extract(i.extra,'$.purchase_source') = p.source_ref)))              ORDER BY p.observed_on, p.source_ref, p.id",
            view(hide_all)
        ))?;
        let rows = statement.query_map(
            params![
                INVENTORY,
                i64::try_from(FOOD_CATEGORY.len()).unwrap_or(5),
                FOOD_CATEGORY
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )?;
        let mut offered = Vec::new();
        for row in rows {
            let (id, observed_on, subject, category, amount_minor, currency, source) = row?;
            offered.push(OfferedLine {
                id: id.parse()?,
                observed_on,
                subject,
                category,
                amount_minor,
                currency,
                source,
            });
        }
        Ok(offered)
    }

    /// Makes one lot from a food purchase line: the lot takes the line's item
    /// (the merchant only when the line names no item)
    /// and category and names the line as its source, so the food is entered
    /// once and seen in both. A second lot for the same line is refused with
    /// the first lot's id.
    pub fn lot_from_purchase(
        &self,
        line: &ObjectId,
        quantity: f64,
        unit: &str,
        expires_on: Option<CivilDate>,
        location: Option<&str>,
        day: &str,
    ) -> Result<ObjectId> {
        let found: Option<(String, Option<String>, String)> = self
            .connection
            .query_row(
                "SELECT coalesce(nullif(json_extract(extra,'$.item'), ''), json_extract(extra,'$.merchant'), subject), \
                 category, kind FROM observation WHERE id = ?1",
                [line.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (subject, category) = match found {
            Some((subject, category, kind)) if kind == "purchase_line" => (subject, category),
            _ => return Err(KernelError::Ledger(format!("no purchase line {line}"))),
        };
        if !category
            .as_deref()
            .is_some_and(|c| c.starts_with(FOOD_CATEGORY))
        {
            return Err(KernelError::Ledger(
                "only a food purchase becomes stock".into(),
            ));
        }
        self.add_stock(
            &LotSpec {
                subject,
                category,
                quantity,
                unit: unit.into(),
                expires_on,
                location: location.map(str::to_owned),
                source: LotSource::Receipt,
                purchase_line: Some(line.clone()),
            },
            day,
        )
    }

    /// Which food purchase lines are in stock, by line id, with the lot each became.
    pub fn stocked_purchases(&self, hide_all: bool) -> Result<BTreeMap<String, ObjectId>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT json_extract(extra,'$.purchase_line'), id FROM {} WHERE kind = ?1                AND json_extract(extra,'$.purchase_line') IS NOT NULL ORDER BY recorded_at, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map([INVENTORY], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut stocked = BTreeMap::new();
        for row in rows {
            let (line, lot) = row?;
            stocked.entry(line).or_insert(lot.parse()?);
        }
        Ok(stocked)
    }

    /// Writes the lot again with a new quantity and extra, superseding it.
    fn rewrite_lot(
        &mut self,
        lot: &Lot,
        quantity: f64,
        extra: Value,
        day: &str,
    ) -> Result<ObjectId> {
        let source_ref: Option<String> = self.connection.query_row(
            "SELECT source_ref FROM observation WHERE id = ?1",
            [lot.id.as_str()],
            |row| row.get(0),
        )?;
        let row = NewObservation {
            kind: INVENTORY.into(),
            observed_on: day.into(),
            category: lot.category.clone(),
            subject: lot.subject.clone(),
            quantity: Some(quantity),
            unit: lot.unit.clone(),
            writer: WRITER.into(),
            source_ref,
            extra: Some(extra),
            ..NewObservation::default()
        };
        self.supersede(&lot.id, &row)
    }

    /// Takes `used` from a lot. More than is on hand is refused.
    pub fn use_stock(&mut self, lot: &ObjectId, used: f64, day: &str) -> Result<ObjectId> {
        let used = amount(used, "an amount used")?;
        let lot = self.lot(lot)?;
        if used > lot.quantity + 1e-9 {
            return Err(KernelError::Ledger(format!(
                "{} has {} on hand; {used} cannot be used",
                lot.subject, lot.quantity
            )));
        }
        let left = (lot.quantity - used).max(0.0);
        let extra = lot.extra.clone();
        self.rewrite_lot(&lot, left, extra, day)
    }

    /// Sets a lot's quantity after a count.
    pub fn adjust_stock(&mut self, lot: &ObjectId, quantity: f64, day: &str) -> Result<ObjectId> {
        let quantity = amount(quantity, "a lot's quantity")?;
        let lot = self.lot(lot)?;
        let extra = lot.extra.clone();
        self.rewrite_lot(&lot, quantity, extra, day)
    }

    /// Sets when a lot goes off.
    pub fn set_expiry(
        &mut self,
        lot: &ObjectId,
        expires_on: CivilDate,
        day: &str,
    ) -> Result<ObjectId> {
        let lot = self.lot(lot)?;
        let mut extra = if lot.extra.is_object() {
            lot.extra.clone()
        } else {
            json!({})
        };
        extra["expires_on"] = json!(expires_on.to_string());
        self.rewrite_lot(&lot, lot.quantity, extra, day)
    }

    /// Adds a piece of equipment, or supersedes the current row of that name.
    pub fn set_equipment(
        &mut self,
        name: &str,
        count: f64,
        capacity: Option<&str>,
        day: &str,
    ) -> Result<ObjectId> {
        let name = named(name, "a piece of equipment")?;
        let count = amount(count, "an equipment count")?;
        let row = NewObservation {
            kind: EQUIPMENT.into(),
            observed_on: day.into(),
            category: Some("kitchen:equipment".into()),
            subject: name.clone(),
            quantity: Some(count),
            unit: Some("item".into()),
            writer: WRITER.into(),
            extra: Some(json!({"capacity": capacity.map(str::trim).filter(|c| !c.is_empty())})),
            ..NewObservation::default()
        };
        match self.current_named(EQUIPMENT, &name)? {
            Some(old) => self.supersede(&old, &row),
            None => self.append_observation(&row),
        }
    }

    fn current_named(&self, kind: &str, subject: &str) -> Result<Option<ObjectId>> {
        let found: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM current_observation WHERE kind = ?1 AND subject = ?2",
                params![kind, subject],
                |row| row.get(0),
            )
            .optional()?;
        found.map(|id| id.parse()).transpose()
    }

    /// The current equipment by name, pieces with a count of zero left out.
    pub fn equipment(&self, hide_all: bool) -> Result<Vec<Equipment>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, subject, quantity, json_extract(extra,'$.capacity') FROM {} \
             WHERE kind = ?1 AND quantity > 0 ORDER BY subject",
            view(hide_all)
        ))?;
        let rows = statement.query_map([EQUIPMENT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (id, name, count, capacity) = row?;
            found.push(Equipment {
                id: id.parse()?,
                name,
                count,
                capacity,
            });
        }
        Ok(found)
    }

    /// Sets one standing constraint (diet, session_hours, portions_per_week, ...),
    /// superseding the current entry of that subject.
    pub fn set_cook_profile(
        &mut self,
        subject: &str,
        quantity: Option<f64>,
        unit: Option<&str>,
        value: Option<&str>,
        day: &str,
    ) -> Result<ObjectId> {
        let subject = named(subject, "a cook profile entry")?;
        let quantity = quantity
            .map(|q| amount(q, "a cook profile quantity"))
            .transpose()?;
        let value = value.map(str::trim).filter(|v| !v.is_empty());
        if quantity.is_none() && value.is_none() {
            return Err(KernelError::Ledger(
                "a cook profile entry needs a quantity or a value".into(),
            ));
        }
        let row = NewObservation {
            kind: COOK_PROFILE.into(),
            observed_on: day.into(),
            category: Some("kitchen:profile".into()),
            subject: subject.clone(),
            quantity,
            unit: unit
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_owned),
            writer: WRITER.into(),
            extra: value.map(|v| json!({"value": v})),
            ..NewObservation::default()
        };
        match self.current_named(COOK_PROFILE, &subject)? {
            Some(old) => self.supersede(&old, &row),
            None => self.append_observation(&row),
        }
    }

    pub fn cook_profile(&self, hide_all: bool) -> Result<Vec<ProfileEntry>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, subject, quantity, unit, json_extract(extra,'$.value') FROM {} \
             WHERE kind = ?1 AND json_extract(extra,'$.removed') IS NULL ORDER BY subject",
            view(hide_all)
        ))?;
        let rows = statement.query_map([COOK_PROFILE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (id, subject, quantity, unit, value) = row?;
            found.push(ProfileEntry {
                id: id.parse()?,
                subject,
                quantity,
                unit,
                value,
            });
        }
        Ok(found)
    }

    /// Q5: the lots going off within `days` of `today`, on hand against what
    /// the current plans use and what is left unplanned.
    pub fn goes_off_within(&self, today: CivilDate, days: i64, hide_all: bool) -> Result<Table> {
        if !(0..=MAX_HORIZON_DAYS).contains(&days) {
            return Err(KernelError::Ledger(format!(
                "a look-ahead is 0 to {MAX_HORIZON_DAYS} days, not {days}"
            )));
        }
        let sql = if hide_all {
            Q5.replace("current_observation", "visible_observation")
        } else {
            Q5.to_owned()
        };
        self.table(&sql, params![today.to_string(), days])
    }

    /// What the current plans already draw from each lot.
    fn planned_by_lot(&self) -> Result<BTreeMap<String, f64>> {
        let mut statement = self.connection.prepare(
            "SELECT json_extract(u.value,'$.lot'), sum(json_extract(u.value,'$.qty')) \
             FROM current_observation b, json_each(b.extra,'$.uses') u \
             WHERE b.kind = ?1 AND json_extract(b.extra,'$.cooked') IS NULL GROUP BY 1",
        )?;
        let rows = statement.query_map([BATCH_PLAN], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<f64>>(1)?,
            ))
        })?;
        let mut planned = BTreeMap::new();
        for row in rows {
            if let (Some(lot), Some(qty)) = row? {
                planned.insert(lot, qty);
            }
        }
        Ok(planned)
    }

    /// Writes one large mise en place plan: one `batch_plan` row per dish under
    /// one plan id, after checking it against stock (lots current, enough left
    /// after other plans), the equipment (each piece named exists, and no phase
    /// asks for more of one than there are) and the cook profile. Then reads Q5
    /// over `horizon_days` from `day` and notes what goes off unplanned.
    /// Nothing is taken from stock: that happens when a dish is cooked.
    pub fn write_batch_plan(
        &mut self,
        day: CivilDate,
        horizon_days: i64,
        dishes: &[DishSpec],
    ) -> Result<PlanOutcome> {
        self.write_batch_plan_for_screen(day, horizon_days, dishes, false)
    }

    /// Writes against the full stock for correctness, but derives every
    /// owner-visible constraint and note through the screen's visibility view.
    pub fn write_batch_plan_for_screen(
        &mut self,
        day: CivilDate,
        horizon_days: i64,
        dishes: &[DishSpec],
        hide_all: bool,
    ) -> Result<PlanOutcome> {
        self.write_plan(day, horizon_days, None, dishes, hide_all, None)
    }

    /// Writes one plan for a batch cook that covers `coverage_days` of meals,
    /// his choice; the look-ahead of what goes off stays `horizon_days`. Every
    /// row of the plan carries the coverage, and the notes say in plain words
    /// when the dishes' portions fall short of the profile's portions per week
    /// over that span; a short plan is still written.
    pub fn write_batch_plan_covering(
        &mut self,
        day: CivilDate,
        horizon_days: i64,
        coverage_days: i64,
        dishes: &[DishSpec],
        hide_all: bool,
    ) -> Result<PlanOutcome> {
        self.write_plan(
            day,
            horizon_days,
            Some(coverage_days),
            dishes,
            hide_all,
            None,
        )
    }

    /// The draft's input, the exact text the service hands a local model when
    /// his setting is on: the stock (with what goes off), the equipment, the
    /// cook profile and the coverage, as one JSON value. Nothing is called here.
    pub fn draft_input(&self, day: CivilDate, coverage_days: i64, hide_all: bool) -> Result<Value> {
        let coverage_days = coverage(coverage_days)?;
        let stock: Vec<Value> = self
            .stock(hide_all)?
            .into_iter()
            .map(|lot| {
                json!({
                    "lot": lot.id.as_str(),
                    "subject": lot.subject,
                    "quantity": lot.quantity,
                    "unit": lot.unit,
                    "goes_off": lot.expires_on,
                })
            })
            .collect();
        let equipment: Vec<Value> = self
            .equipment(hide_all)?
            .into_iter()
            .map(|piece| json!({"name": piece.name, "count": piece.count, "capacity": piece.capacity}))
            .collect();
        let profile: Vec<Value> = self
            .cook_profile(hide_all)?
            .into_iter()
            .map(|entry| json!({"subject": entry.subject, "quantity": entry.quantity, "unit": entry.unit, "value": entry.value}))
            .collect();
        Ok(json!({
            "day": day.to_string(),
            "coverage_days": coverage_days,
            "stock": stock,
            "equipment": equipment,
            "profile": profile,
        }))
    }

    /// Takes a drafted plan back: the dishes are checked and written like any
    /// plan, over the coverage (which is also the look-ahead), each row with
    /// standing `inferred` and `derived_by` the rule that drafted it, for his review.
    pub fn accept_draft(
        &mut self,
        day: CivilDate,
        coverage_days: i64,
        dishes: &[DishSpec],
        rule: &str,
    ) -> Result<PlanOutcome> {
        let rule = named(rule, "a draft's rule")?;
        self.write_plan(
            day,
            coverage_days,
            Some(coverage_days),
            dishes,
            false,
            Some(&rule),
        )
    }

    fn write_plan(
        &mut self,
        day: CivilDate,
        horizon_days: i64,
        coverage_days: Option<i64>,
        dishes: &[DishSpec],
        hide_all: bool,
        derived_by: Option<&str>,
    ) -> Result<PlanOutcome> {
        if dishes.is_empty() {
            return Err(KernelError::Ledger("a plan needs at least one dish".into()));
        }
        let coverage_days = coverage_days.map(coverage).transpose()?;
        let plan_id = format!("plan_{day}");
        let taken: i64 = self.connection.query_row(
            "SELECT count(*) FROM current_observation WHERE kind = ?1 AND json_extract(extra,'$.plan_id') = ?2",
            params![BATCH_PLAN, plan_id],
            |row| row.get(0),
        )?;
        if taken > 0 {
            return Err(KernelError::Conflict(format!(
                "{plan_id} is already written"
            )));
        }
        let equipment: BTreeMap<String, f64> = self
            .equipment(false)?
            .into_iter()
            .map(|e| (e.name, e.count))
            .collect();
        let mut planned = self.planned_by_lot()?;
        let mut busy: BTreeMap<(u32, String), f64> = BTreeMap::new();
        let mut lots: BTreeMap<String, Lot> = BTreeMap::new();
        // Exclusions hold whatever the screen hides: a private one still applies.
        let excluded = excluded(&self.cook_profile(false)?);
        for dish in dishes {
            named(&dish.name, "a dish")?;
            if !(dish.portions.is_finite() && dish.portions > 0.0) {
                return Err(KernelError::Ledger(format!(
                    "{} needs a number of portions above zero",
                    dish.name
                )));
            }
            for piece in &dish.equipment {
                let have = equipment.get(piece.trim()).copied().ok_or_else(|| {
                    KernelError::Ledger(format!(
                        "{} needs a {piece}, which is not in the equipment list",
                        dish.name
                    ))
                })?;
                let using = busy
                    .entry((dish.phase, piece.trim().to_owned()))
                    .or_insert(0.0);
                *using += 1.0;
                if *using > have {
                    return Err(KernelError::Ledger(format!(
                        "phase {} asks for {using} of {piece}; there are {have}",
                        dish.phase
                    )));
                }
            }
            for (lot, qty) in &dish.uses {
                let qty = amount(*qty, "an amount planned")?;
                if !lots.contains_key(lot.as_str()) {
                    lots.insert(lot.to_string(), self.lot(lot)?);
                }
                if let Some(name) = excluded_in(&lots[lot.as_str()].subject, &excluded) {
                    return Err(KernelError::Ledger(format!(
                        "{} uses {}, which the cook profile excludes",
                        dish.name.trim(),
                        name
                    )));
                }
                let on_hand = lots[lot.as_str()].quantity;
                let total = planned.entry(lot.to_string()).or_insert(0.0);
                *total += qty;
                if *total > on_hand + 1e-9 {
                    return Err(KernelError::Ledger(format!(
                        "{} has {on_hand} {} on hand; the plans would use {}",
                        lots[lot.as_str()].subject,
                        lots[lot.as_str()].unit.as_deref().unwrap_or(""),
                        total
                    )));
                }
            }
        }
        let constraints = self.cook_profile(hide_all)?;
        let horizon_end = day
            .add_days(horizon_days.clamp(0, MAX_HORIZON_DAYS))?
            .to_string();
        let visible: Option<Vec<String>> = if hide_all {
            Some(
                self.stock(true)?
                    .into_iter()
                    .map(|l| l.id.to_string())
                    .collect(),
            )
        } else {
            None
        };
        let mut ingredients: Vec<PlanIngredient> = Vec::new();
        for dish in dishes {
            for (lot, qty) in &dish.uses {
                if visible
                    .as_ref()
                    .is_some_and(|v| !v.contains(&lot.to_string()))
                {
                    continue;
                }
                match ingredients.iter_mut().find(|i| &i.lot == lot) {
                    Some(found) => found.quantity += qty,
                    None => {
                        let read = &lots[lot.as_str()];
                        ingredients.push(PlanIngredient {
                            lot: lot.clone(),
                            subject: read.subject.clone(),
                            quantity: *qty,
                            unit: read.unit.clone(),
                            expires_on: read.expires_on.clone(),
                            goes_off_soon: read
                                .expires_on
                                .as_deref()
                                .is_some_and(|e| e <= horizon_end.as_str()),
                        });
                    }
                }
            }
        }
        ingredients.sort_by(|a, b| match (&a.expires_on, &b.expires_on) {
            (Some(x), Some(y)) => x.cmp(y).then_with(|| a.subject.cmp(&b.subject)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.subject.cmp(&b.subject),
        });
        let has_standing = derived_by.is_some() && self.has_standing_column()?;
        let transaction = self.connection.transaction()?;
        let mut rows = Vec::new();
        for dish in dishes {
            let uses: Vec<Value> = dish
                .uses
                .iter()
                .map(|(lot, qty)| json!({"lot": lot.as_str(), "qty": qty}))
                .collect();
            let mut extra = json!({
                "plan_id": plan_id,
                "phase": dish.phase,
                "equipment": dish.equipment.iter().map(|e| e.trim()).collect::<Vec<_>>(),
                "uses": uses,
            });
            if let Some(days) = coverage_days {
                extra["coverage_days"] = json!(days);
            }
            if let Some(rule) = derived_by {
                extra["standing"] = json!(DRAFTED);
                extra["derived_by"] = json!(rule);
            }
            let id = Self::insert(
                &transaction,
                &NewObservation {
                    kind: BATCH_PLAN.into(),
                    observed_on: day.to_string(),
                    category: Some("kitchen:plan".into()),
                    subject: dish.name.trim().to_owned(),
                    quantity: Some(dish.portions),
                    unit: Some("portion".into()),
                    writer: WRITER.into(),
                    extra: Some(extra),
                    ..NewObservation::default()
                },
            )?;
            if has_standing {
                // The record's own standing column, where the ledger has one.
                transaction.execute(
                    "UPDATE observation SET standing = ?2 WHERE id = ?1",
                    params![id.as_str(), DRAFTED],
                )?;
            }
            rows.push(id);
        }
        transaction.commit()?;
        let portions: f64 = dishes.iter().map(|d| d.portions).sum();
        let mut notes = Vec::new();
        let per_week = constraints
            .iter()
            .find(|c| c.subject == "portions_per_week")
            .and_then(|c| c.quantity);
        match (per_week, coverage_days) {
            (Some(week), Some(days)) => {
                #[allow(clippy::cast_precision_loss)]
                let wanted = (week * days as f64 / 7.0 * 10.0).round() / 10.0;
                if portions + 1e-9 < wanted {
                    notes.push(format!(
                        "{portions} portions planned; {days} days at {week} a week wants {wanted}"
                    ));
                }
            }
            (Some(week), None) => {
                notes.push(format!("{portions} portions planned against {week} a week"))
            }
            (None, _) => {}
        }
        let goes_off = self.goes_off_within(day, horizon_days, hide_all)?;
        for row in &goes_off.rows {
            // A lot any current plan draws from, this one included, is in a
            // plan even when some of it is left over.
            let in_a_plan = row[4].as_f64().unwrap_or(0.0) > 0.0;
            if !in_a_plan && row[5].as_f64().unwrap_or(0.0) > 0.0 {
                notes.push(format!(
                    "{} {} {} goes off {} and is not in any plan",
                    row[0].as_str().unwrap_or(""),
                    row[5],
                    row[2].as_str().unwrap_or(""),
                    row[3].as_str().unwrap_or("")
                ));
            }
        }
        Ok(PlanOutcome {
            plan_id,
            rows,
            coverage_days,
            portions,
            constraints,
            notes,
            goes_off,
            ingredients,
        })
    }
}

/// A coverage of 1 to `MAX_COVERAGE_DAYS` days.
fn coverage(days: i64) -> Result<i64> {
    if (1..=MAX_COVERAGE_DAYS).contains(&days) {
        Ok(days)
    } else {
        Err(KernelError::Ledger(format!(
            "a batch covers 1 to {MAX_COVERAGE_DAYS} days of meals, not {days}"
        )))
    }
}

impl Ledger {
    /// Whether the observation table carries the record's standing column
    /// (added to his ledger by the personal record's spine).
    fn has_standing_column(&self) -> Result<bool> {
        let found: i64 = self.connection.query_row(
            "SELECT count(*) FROM pragma_table_info('observation') WHERE name = 'standing'",
            [],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    /// The current batch plans by day, oldest first (one day when `day` is
    /// given), each with its dishes in phase order; read through the
    /// visibility view while hide-all is on.
    pub fn batch_plans(&self, day: Option<CivilDate>, hide_all: bool) -> Result<Vec<BatchPlan>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, observed_on, json_extract(extra,'$.plan_id'), subject, coalesce(quantity, 0), extra FROM {} \
             WHERE kind = ?1 AND (?2 IS NULL OR observed_on = ?2) \
             ORDER BY observed_on, json_extract(extra,'$.plan_id'), coalesce(json_extract(extra,'$.phase'), 0), recorded_at, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map(params![BATCH_PLAN, day.map(|d| d.to_string())], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut plans: Vec<BatchPlan> = Vec::new();
        for row in rows {
            let (id, observed_on, plan_id, name, portions, extra) = row?;
            let extra = parse_extra(extra)?;
            let plan_id = plan_id.unwrap_or_else(|| format!("plan_{observed_on}"));
            let phase = u32::try_from(extra["phase"].as_u64().unwrap_or(0))
                .map_err(|_| KernelError::Ledger(format!("{id} has a phase out of range")))?;
            let equipment = extra["equipment"]
                .as_array()
                .map(|pieces| {
                    pieces
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let mut uses = Vec::new();
            for used in extra["uses"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let lot = used["lot"]
                    .as_str()
                    .ok_or_else(|| KernelError::Ledger(format!("{id} has a use without a lot")))?;
                uses.push((lot.parse()?, used["qty"].as_f64().unwrap_or(0.0)));
            }
            let cooked = extra["cooked"].as_str().map(str::parse).transpose()?;
            let coverage_days = extra["coverage_days"].as_i64();
            let standing = extra["standing"].as_str().unwrap_or("observed").to_owned();
            let derived_by = extra["derived_by"].as_str().map(str::to_owned);
            let dish = PlannedDish {
                id: id.parse()?,
                name,
                portions,
                phase,
                equipment,
                uses,
                cooked,
                standing,
                derived_by,
            };
            match plans.last_mut() {
                Some(plan) if plan.plan_id == plan_id => {
                    plan.portions += portions;
                    plan.coverage_days = plan.coverage_days.or(coverage_days);
                    plan.dishes.push(dish);
                }
                _ => plans.push(BatchPlan {
                    plan_id,
                    day: observed_on,
                    portions,
                    coverage_days,
                    dishes: vec![dish],
                }),
            }
        }
        Ok(plans)
    }
}

/// Copies a lot's picture to `<workspace>/kitchen/pictures/<lot id>.<ext>`
/// (0600, the folders 0700) and writes its name and sha256 into the lot row
/// `id`, which `transaction` has just written. Returns the copy's path.
fn attach_picture(
    transaction: &rusqlite::Transaction<'_>,
    workspace: &Path,
    id: &ObjectId,
    bytes: &[u8],
    extension: &str,
) -> Result<PathBuf> {
    let folder = workspace.join(PICTURES_DIR);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)?;
    let name = format!("{PICTURES_DIR}/{id}.{extension}");
    let path = workspace.join(&name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    let recorded = written.map_err(KernelError::from).and_then(|()| {
        transaction
            .execute(
                "UPDATE observation SET extra = json_set(coalesce(extra,'{}'), '$.picture', ?2, '$.sha256', ?3)                  WHERE id = ?1 AND superseded_by IS NULL",
                params![id.as_str(), name, ContentHash::digest(bytes).short_hex(32)],
            )
            .map_err(KernelError::from)
    });
    match recorded {
        Ok(1) => Ok(path),
        Ok(_) => {
            let _ = fs::remove_file(&path);
            Err(KernelError::Conflict(format!("{id} is no longer current")))
        }
        Err(error) => {
            let _ = fs::remove_file(&path);
            Err(error)
        }
    }
}

/// Writes `row` as the correction of the current row `old` inside `transaction`.
fn supersede_in(
    transaction: &rusqlite::Transaction<'_>,
    old: &str,
    row: &NewObservation,
) -> Result<ObjectId> {
    let id = Ledger::insert(transaction, row)?;
    let changed = transaction.execute(
        "UPDATE observation SET superseded_by = ?1 WHERE id = ?2 AND superseded_by IS NULL",
        params![id.as_str(), old],
    )?;
    if changed != 1 {
        return Err(KernelError::Conflict(format!("{old} is no longer current")));
    }
    Ok(id)
}

impl Ledger {
    /// The row now current in the line of corrections that starts at `id`.
    fn current_in_line(&self, id: &str) -> Result<String> {
        let mut at = id.to_owned();
        for _ in 0..10_000 {
            let next: Option<Option<String>> = self
                .connection
                .query_row(
                    "SELECT superseded_by FROM observation WHERE id = ?1",
                    [at.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            match next {
                None => return Err(KernelError::Ledger(format!("no observation {at}"))),
                Some(None) => return Ok(at),
                Some(Some(by)) => at = by,
            }
        }
        Err(KernelError::Ledger(format!(
            "{id} has no end to its corrections"
        )))
    }

    /// Cooks one dish of a plan: writes a `meal` row (home, source_ref the
    /// plan id), takes each use from the lot now current in that lot's line,
    /// and supersedes the dish's plan row with `cooked` set so the plans no
    /// longer count it as planned. All or nothing; more than is on hand is
    /// refused, and a dish is cooked once. `dish` is the plan row id or name.
    pub fn cook_dish(&mut self, plan_id: &str, dish: &str, day: CivilDate) -> Result<CookOutcome> {
        let (plan_id, dish) = (plan_id.trim(), dish.trim());
        let mut statement = self.connection.prepare(
            "SELECT id, observed_on, category, subject, quantity, unit, extra FROM current_observation \
             WHERE kind = ?1 AND json_extract(extra,'$.plan_id') = ?2 AND (id = ?3 OR subject = ?3)",
        )?;
        let found = statement
            .query_map(params![BATCH_PLAN, plan_id, dish], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let (old_id, plan_day, category, name, portions, unit, extra) = match found.len() {
            0 => {
                return Err(KernelError::Ledger(format!(
                    "{plan_id} has no dish {dish:?}"
                )))
            }
            1 => found.into_iter().next().unwrap_or_else(|| unreachable!()),
            _ => {
                return Err(KernelError::Ledger(format!(
                    "{plan_id} has more than one dish named {dish:?}; give its id"
                )))
            }
        };
        let mut extra = parse_extra(extra)?;
        if let Some(meal) = extra["cooked"].as_str() {
            return Err(KernelError::Conflict(format!(
                "{name} is already cooked ({meal})"
            )));
        }
        let mut drawn: BTreeMap<String, f64> = BTreeMap::new();
        for used in extra["uses"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let lot = used["lot"]
                .as_str()
                .ok_or_else(|| KernelError::Ledger(format!("{old_id} has a use without a lot")))?;
            *drawn.entry(self.current_in_line(lot)?).or_insert(0.0) +=
                amount(used["qty"].as_f64().unwrap_or(0.0), "an amount planned")?;
        }
        let mut lots = Vec::new();
        for (lot, qty) in &drawn {
            let lot = self.lot(&lot.parse()?)?;
            if *qty > lot.quantity + 1e-9 {
                return Err(KernelError::Ledger(format!(
                    "{} has {} {} on hand; {name} uses {qty}",
                    lot.subject,
                    lot.quantity,
                    lot.unit.as_deref().unwrap_or("")
                )));
            }
            let source_ref: Option<String> = self.connection.query_row(
                "SELECT source_ref FROM observation WHERE id = ?1",
                [lot.id.as_str()],
                |row| row.get(0),
            )?;
            lots.push((lot, *qty, source_ref));
        }
        let day = day.to_string();
        let transaction = self.connection.transaction()?;
        let meal = Self::insert(
            &transaction,
            &NewObservation {
                kind: MEAL.into(),
                observed_on: day.clone(),
                category: Some(MEAL_HOME.into()),
                subject: name.clone(),
                quantity: portions,
                unit: Some("portion".into()),
                writer: WRITER.into(),
                source_ref: Some(plan_id.to_owned()),
                extra: Some(json!({
                    "plan_id": plan_id,
                    "dish": old_id,
                    "uses": drawn.iter().map(|(lot, qty)| json!({"lot": lot, "qty": qty})).collect::<Vec<_>>(),
                })),
                ..NewObservation::default()
            },
        )?;
        let mut moved = Vec::new();
        for (lot, qty, source_ref) in lots {
            let row = NewObservation {
                kind: INVENTORY.into(),
                observed_on: day.clone(),
                category: lot.category.clone(),
                subject: lot.subject.clone(),
                quantity: Some((lot.quantity - qty).max(0.0)),
                unit: lot.unit.clone(),
                writer: WRITER.into(),
                source_ref,
                extra: Some(lot.extra.clone()),
                ..NewObservation::default()
            };
            let new = supersede_in(&transaction, lot.id.as_str(), &row)?;
            moved.push((lot.id, new));
        }
        extra["cooked"] = json!(meal.as_str());
        let dish_row = supersede_in(
            &transaction,
            &old_id,
            &NewObservation {
                kind: BATCH_PLAN.into(),
                observed_on: plan_day,
                category,
                subject: name,
                quantity: portions,
                unit,
                writer: WRITER.into(),
                extra: Some(extra),
                ..NewObservation::default()
            },
        )?;
        transaction.commit()?;
        Ok(CookOutcome {
            meal,
            dish: dish_row,
            lots: moved,
        })
    }

    /// The current meals, oldest first (one day when `day` is given); read
    /// through the visibility view while hide-all is on.
    pub fn meals(&self, day: Option<CivilDate>, hide_all: bool) -> Result<Vec<Meal>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, observed_on, subject, coalesce(quantity, 0), category, json_extract(extra,'$.plan_id') FROM {} \
             WHERE kind = ?1 AND (?2 IS NULL OR observed_on = ?2) ORDER BY observed_on, recorded_at, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map(params![MEAL, day.map(|d| d.to_string())], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut meals = Vec::new();
        for row in rows {
            let (id, day, name, portions, category, plan_id) = row?;
            meals.push(Meal {
                id: id.parse()?,
                day,
                name,
                portions,
                category,
                plan_id,
            });
        }
        Ok(meals)
    }

    /// Removes a cook-profile entry by superseding it with an empty row
    /// (`removed` in extra), which the profile read leaves out. Setting the
    /// subject again supersedes the empty row.
    pub fn remove_cook_profile(&mut self, subject: &str, day: &str) -> Result<ObjectId> {
        let subject = named(subject, "a cook profile entry")?;
        let current: Option<(String, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT id, json_extract(extra,'$.removed') FROM current_observation WHERE kind = ?1 AND subject = ?2",
                params![COOK_PROFILE, subject],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let old = match current {
            Some((id, None)) => id.parse()?,
            _ => {
                return Err(KernelError::Ledger(format!(
                    "the cook profile has no entry {subject:?}"
                )))
            }
        };
        self.supersede(
            &old,
            &NewObservation {
                kind: COOK_PROFILE.into(),
                observed_on: day.into(),
                category: Some("kitchen:profile".into()),
                subject,
                writer: WRITER.into(),
                extra: Some(json!({"removed": true})),
                ..NewObservation::default()
            },
        )
    }

    /// Puts back a removed cook-profile entry: a copy of its values supersedes
    /// the empty row that removed it, which must still be current for its
    /// subject.
    pub fn restore_cook_profile(&mut self, original: &ObjectId) -> Result<ObjectId> {
        let found: Option<(String, String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT kind, subject, superseded_by FROM observation WHERE id = ?1",
                [original.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (_, subject, by) = match found {
            Some(found) if found.0 == COOK_PROFILE => found,
            _ => {
                return Err(KernelError::Ledger(format!(
                    "no cook profile entry {original}"
                )))
            }
        };
        let removal = by.ok_or_else(|| {
            KernelError::Conflict("this cook profile entry was not removed".into())
        })?;
        let removed: Option<i64> = self.connection.query_row(
            "SELECT json_extract(extra,'$.removed') FROM observation WHERE id = ?1",
            [removal.as_str()],
            |row| row.get(0),
        )?;
        if removed.is_none()
            || self
                .current_named(COOK_PROFILE, &subject)?
                .as_ref()
                .map(ObjectId::as_str)
                != Some(removal.as_str())
        {
            return Err(KernelError::Conflict(format!(
                "a newer entry for {subject:?} replaced this one, so it stays where it is"
            )));
        }
        let row = self.copy_of(original)?;
        self.supersede(&removal.parse()?, &row)
    }

    /// Moves a lot to where it is now kept; an empty location clears it.
    pub fn move_lot(&mut self, lot: &ObjectId, location: &str, day: &str) -> Result<ObjectId> {
        let lot = self.lot(lot)?;
        let mut extra = if lot.extra.is_object() {
            lot.extra.clone()
        } else {
            json!({})
        };
        match location.trim() {
            "" => {
                if let Some(map) = extra.as_object_mut() {
                    map.remove("location");
                }
            }
            place => extra["location"] = json!(place),
        }
        self.rewrite_lot(&lot, lot.quantity, extra, day)
    }
}
