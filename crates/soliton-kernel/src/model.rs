use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use crate::date::{CivilDate, Weekday};
use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::{ObjectId, OccurrenceId};
use crate::recurrence::{MonthlyPattern, RecurrenceCadence, RecurrenceRule, WeekOrdinal};

const FORMAT_HEADER: &str = "soliton-object/1";

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal,)+ }) => {
        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            pub fn parse(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(KernelError::Codec(format!(
                        "unknown {} value {value:?}",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DataClass {
    Public,
    Internal,
    PersonalSensitive,
    Restricted,
}

impl DataClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::PersonalSensitive => "personal_sensitive",
            Self::Restricted => "restricted",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "public" => Ok(Self::Public),
            "internal" => Ok(Self::Internal),
            "personal_sensitive" => Ok(Self::PersonalSensitive),
            "restricted" => Ok(Self::Restricted),
            _ => Err(KernelError::Codec(format!("unknown data_class {value:?}"))),
        }
    }

    #[must_use]
    pub const fn exportable_to_guest(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EntityKind {
    Document,
    Category,
    Aim,
    Task,
    Reminder,
    Routine,
}

impl EntityKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Category => "category",
            Self::Aim => "aim",
            Self::Task => "task",
            Self::Reminder => "reminder",
            Self::Routine => "routine",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "document" => Ok(Self::Document),
            "category" => Ok(Self::Category),
            "aim" => Ok(Self::Aim),
            "task" => Ok(Self::Task),
            "reminder" => Ok(Self::Reminder),
            "routine" => Ok(Self::Routine),
            _ => Err(KernelError::Codec(format!("unknown entity kind {value:?}"))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectMeta {
    pub details: crate::details::Details,
    pub id: ObjectId,
    pub title: String,
    pub data_class: DataClass,
    pub revision: u64,
}

impl ObjectMeta {
    pub fn validate_for(&self, kind: EntityKind) -> Result<()> {
        self.details.validate()?;
        if self.id.kind() != kind.as_str() {
            return Err(KernelError::InvalidObject(format!(
                "{} object has {} identifier",
                kind.as_str(),
                self.id.kind()
            )));
        }
        if self.title.trim().is_empty() {
            return Err(KernelError::InvalidObject("title cannot be empty".into()));
        }
        if self.revision == 0 {
            return Err(KernelError::InvalidObject(
                "revision zero is reserved and cannot be stored".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Category {
    pub meta: ObjectMeta,
    pub color: Option<String>,
    pub sort_order: i32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AimState {
    Active,
    Completed,
    Archived,
}

string_enum!(AimState {
    Active => "active",
    Completed => "completed",
    Archived => "archived",
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Aim {
    pub folder: Option<String>,
    pub meta: ObjectMeta,
    pub category_id: Option<ObjectId>,
    pub state: AimState,
    pub target_on: Option<CivilDate>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Priority {
    Low,
    Medium,
    High,
}

string_enum!(Priority {
    Low => "low",
    Medium => "medium",
    High => "high",
});

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Completed,
    Archived,
}

string_enum!(TaskStatus {
    Todo => "todo",
    InProgress => "in_progress",
    Completed => "completed",
    Archived => "archived",
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub meta: ObjectMeta,
    pub category_id: Option<ObjectId>,
    pub aim_id: Option<ObjectId>,
    pub priority: Priority,
    pub status: TaskStatus,
    pub scheduled_for: Option<CivilDate>,
    pub due_on: Option<CivilDate>,
    pub recurrence: Option<RecurrenceRule>,
    pub completed_occurrences: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReminderState {
    Pending,
    Completed,
    Dismissed,
    Archived,
}

string_enum!(ReminderState {
    Pending => "pending",
    Completed => "completed",
    Dismissed => "dismissed",
    Archived => "archived",
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reminder {
    pub meta: ObjectMeta,
    pub related_to: Option<ObjectId>,
    pub scheduled_for: CivilDate,
    pub state: ReminderState,
    pub recurrence: Option<RecurrenceRule>,
    pub completed_occurrences: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoutineState {
    Active,
    Paused,
    Archived,
}

string_enum!(RoutineState {
    Active => "active",
    Paused => "paused",
    Archived => "archived",
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Routine {
    pub meta: ObjectMeta,
    pub category_id: Option<ObjectId>,
    pub state: RoutineState,
    pub recurrence: RecurrenceRule,
    pub completed_occurrences: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub meta: ObjectMeta,
    pub document_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entity {
    Document(Document),
    Category(Category),
    Aim(Aim),
    Task(Task),
    Reminder(Reminder),
    Routine(Routine),
}

impl Entity {
    #[must_use]
    pub const fn kind(&self) -> EntityKind {
        match self {
            Self::Document(_) => EntityKind::Document,
            Self::Category(_) => EntityKind::Category,
            Self::Aim(_) => EntityKind::Aim,
            Self::Task(_) => EntityKind::Task,
            Self::Reminder(_) => EntityKind::Reminder,
            Self::Routine(_) => EntityKind::Routine,
        }
    }

    #[must_use]
    pub fn meta(&self) -> &ObjectMeta {
        match self {
            Self::Document(value) => &value.meta,
            Self::Category(value) => &value.meta,
            Self::Aim(value) => &value.meta,
            Self::Task(value) => &value.meta,
            Self::Reminder(value) => &value.meta,
            Self::Routine(value) => &value.meta,
        }
    }

    pub fn meta_mut(&mut self) -> &mut ObjectMeta {
        match self {
            Self::Category(v) => &mut v.meta,
            Self::Aim(v) => &mut v.meta,
            Self::Task(v) => &mut v.meta,
            Self::Reminder(v) => &mut v.meta,
            Self::Routine(v) => &mut v.meta,
            Self::Document(v) => &mut v.meta,
        }
    }

    #[must_use]
    pub fn id(&self) -> &ObjectId {
        &self.meta().id
    }

    pub fn validate(&self) -> Result<()> {
        self.meta().validate_for(self.kind())?;
        match self {
            Self::Aim(aim) => {
                if let Some(folder) = &aim.folder {
                    if !std::path::Path::new(folder).is_absolute()
                        || folder.len() > 4096
                        || folder.contains('\0')
                    {
                        return Err(KernelError::InvalidObject(
                            "Goal folder must be an absolute filesystem path".into(),
                        ));
                    }
                }
            }
            Self::Category(_) | Self::Document(_) => {}
            Self::Task(task) => {
                validate_occurrence_series(&task.meta.id, &task.completed_occurrences)?;
                if let Some(rule) = &task.recurrence {
                    rule.validate()?;
                }
            }
            Self::Reminder(reminder) => {
                validate_occurrence_series(&reminder.meta.id, &reminder.completed_occurrences)?;
                if let Some(rule) = &reminder.recurrence {
                    rule.validate()?;
                }
            }
            Self::Routine(routine) => {
                validate_occurrence_series(&routine.meta.id, &routine.completed_occurrences)?;
                routine.recurrence.validate()?;
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let extended =
            self.meta().details != Default::default() || matches!(self, Self::Document(_));
        let mut lines = vec![if extended {
            "soliton-object/2"
        } else {
            FORMAT_HEADER
        }
        .to_owned()];
        push(&mut lines, "kind", self.kind().as_str());
        encode_meta(&mut lines, self.meta());
        if extended {
            push(
                &mut lines,
                "details",
                &serde_json::to_string(&self.meta().details)
                    .map_err(|e| KernelError::Codec(e.to_string()))?,
            );
        }
        match self {
            Self::Document(document) => {
                push(&mut lines, "document_type", &document.document_type);
            }
            Self::Category(category) => {
                push_optional(&mut lines, "color", category.color.as_deref());
                push(&mut lines, "sort_order", &category.sort_order.to_string());
            }
            Self::Aim(aim) => {
                if let Some(folder) = &aim.folder {
                    push(&mut lines, "folder", folder);
                }
                push_id(&mut lines, "category_id", aim.category_id.as_ref());
                push(&mut lines, "state", aim.state.as_str());
                push_date(&mut lines, "target_on", aim.target_on);
            }
            Self::Task(task) => {
                push_id(&mut lines, "category_id", task.category_id.as_ref());
                push_id(&mut lines, "aim_id", task.aim_id.as_ref());
                push(&mut lines, "priority", task.priority.as_str());
                push(&mut lines, "status", task.status.as_str());
                push_date(&mut lines, "scheduled_for", task.scheduled_for);
                push_date(&mut lines, "due_on", task.due_on);
                encode_rule(&mut lines, task.recurrence.as_ref());
                encode_completed(&mut lines, &task.completed_occurrences);
            }
            Self::Reminder(reminder) => {
                push_id(&mut lines, "related_to", reminder.related_to.as_ref());
                push_date(&mut lines, "scheduled_for", Some(reminder.scheduled_for));
                push(&mut lines, "state", reminder.state.as_str());
                encode_rule(&mut lines, reminder.recurrence.as_ref());
                encode_completed(&mut lines, &reminder.completed_occurrences);
            }
            Self::Routine(routine) => {
                push_id(&mut lines, "category_id", routine.category_id.as_ref());
                push(&mut lines, "state", routine.state.as_str());
                encode_rule(&mut lines, Some(&routine.recurrence));
                encode_completed(&mut lines, &routine.completed_occurrences);
            }
        }
        lines.push(String::new());
        Ok(lines.join("\n").into_bytes())
    }

    pub fn content_hash(&self) -> Result<ContentHash> {
        Ok(ContentHash::digest(&self.canonical_bytes()?))
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| KernelError::Codec("object file is not UTF-8".into()))?;
        let mut lines = text.lines();
        let header = lines.next();
        if header != Some(FORMAT_HEADER) && header != Some("soliton-object/2") {
            return Err(KernelError::Codec(format!(
                "missing {FORMAT_HEADER} header"
            )));
        }
        let mut fields = BTreeMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (key, encoded) = line
                .split_once('=')
                .ok_or_else(|| KernelError::Codec(format!("field line lacks '=': {line:?}")))?;
            if fields
                .insert(key.to_owned(), decode_text(encoded)?)
                .is_some()
            {
                return Err(KernelError::Codec(format!("duplicate field {key:?}")));
            }
        }
        let kind = EntityKind::parse(&take_required(&mut fields, "kind")?)?;
        let mut meta = decode_meta(&mut fields)?;
        if header == Some("soliton-object/2") {
            meta.details = serde_json::from_str(&take_required(&mut fields, "details")?)
                .map_err(|e| KernelError::Codec(e.to_string()))?;
        }
        let entity = match kind {
            EntityKind::Document => Self::Document(Document {
                meta,
                document_type: take_required(&mut fields, "document_type")?,
            }),
            EntityKind::Category => Self::Category(Category {
                meta,
                color: take_optional(&mut fields, "color")?,
                sort_order: parse_required(&mut fields, "sort_order")?,
            }),
            EntityKind::Aim => Self::Aim(Aim {
                folder: fields.remove("folder").filter(|value| !value.is_empty()),
                meta,
                category_id: take_id(&mut fields, "category_id")?,
                state: AimState::parse(&take_required(&mut fields, "state")?)?,
                target_on: take_date(&mut fields, "target_on")?,
            }),
            EntityKind::Task => Self::Task(Task {
                meta,
                category_id: take_id(&mut fields, "category_id")?,
                aim_id: take_id(&mut fields, "aim_id")?,
                priority: Priority::parse(&take_required(&mut fields, "priority")?)?,
                status: TaskStatus::parse(&take_required(&mut fields, "status")?)?,
                scheduled_for: take_date(&mut fields, "scheduled_for")?,
                due_on: take_date(&mut fields, "due_on")?,
                recurrence: decode_rule(&mut fields)?,
                completed_occurrences: decode_completed(&mut fields)?,
            }),
            EntityKind::Reminder => Self::Reminder(Reminder {
                meta,
                related_to: take_id(&mut fields, "related_to")?,
                scheduled_for: take_date(&mut fields, "scheduled_for")?.ok_or_else(|| {
                    KernelError::Codec("reminder scheduled_for cannot be empty".into())
                })?,
                state: ReminderState::parse(&take_required(&mut fields, "state")?)?,
                recurrence: decode_rule(&mut fields)?,
                completed_occurrences: decode_completed(&mut fields)?,
            }),
            EntityKind::Routine => Self::Routine(Routine {
                meta,
                category_id: take_id(&mut fields, "category_id")?,
                state: RoutineState::parse(&take_required(&mut fields, "state")?)?,
                recurrence: decode_rule(&mut fields)?.ok_or_else(|| {
                    KernelError::Codec("routine recurrence cannot be absent".into())
                })?,
                completed_occurrences: decode_completed(&mut fields)?,
            }),
        };
        if !fields.is_empty() {
            return Err(KernelError::Codec(format!(
                "unknown fields: {}",
                fields.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        entity.validate()?;
        Ok(entity)
    }
}

fn validate_occurrence_series(
    series_id: &ObjectId,
    occurrences: &BTreeSet<OccurrenceId>,
) -> Result<()> {
    if let Some(foreign) = occurrences
        .iter()
        .find(|occurrence| &occurrence.series_id != series_id)
    {
        return Err(KernelError::InvalidObject(format!(
            "completed occurrence {} belongs to {}, not {series_id}",
            foreign.opaque_key(),
            foreign.series_id
        )));
    }
    Ok(())
}

fn encode_meta(lines: &mut Vec<String>, meta: &ObjectMeta) {
    push(lines, "id", meta.id.as_str());
    push(lines, "title", &meta.title);
    push(lines, "data_class", meta.data_class.as_str());
    push(lines, "revision", &meta.revision.to_string());
}

fn decode_meta(fields: &mut BTreeMap<String, String>) -> Result<ObjectMeta> {
    Ok(ObjectMeta {
        details: Default::default(),
        id: take_required(fields, "id")?.parse()?,
        title: take_required(fields, "title")?,
        data_class: DataClass::parse(&take_required(fields, "data_class")?)?,
        revision: parse_required(fields, "revision")?,
    })
}

fn encode_rule(lines: &mut Vec<String>, rule: Option<&RecurrenceRule>) {
    let Some(rule) = rule else {
        push(lines, "recurrence.kind", "none");
        return;
    };
    let kind = match &rule.cadence {
        RecurrenceCadence::Daily => "daily",
        RecurrenceCadence::Weekly { .. } => "weekly",
        RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::DayOfMonth(_),
        } => "monthly_day",
        RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::NthWeekday { .. },
        } => "monthly_nth_weekday",
        RecurrenceCadence::Yearly { .. } => "yearly",
    };
    push(lines, "recurrence.kind", kind);
    push_date(lines, "recurrence.starts_on", Some(rule.starts_on));
    push(lines, "recurrence.interval", &rule.interval.to_string());
    push_date(lines, "recurrence.ends_on", rule.ends_on);
    push_optional(
        lines,
        "recurrence.max_occurrences",
        rule.max_occurrences
            .map(|value| value.to_string())
            .as_deref(),
    );
    push(
        lines,
        "recurrence.exclusions",
        &rule
            .exclusions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    match &rule.cadence {
        RecurrenceCadence::Daily => {}
        RecurrenceCadence::Weekly { weekdays } => push(
            lines,
            "recurrence.weekdays",
            &weekdays
                .iter()
                .map(|weekday| weekday.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
        RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::DayOfMonth(day),
        } => push(lines, "recurrence.day", &day.to_string()),
        RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::NthWeekday { ordinal, weekday },
        } => {
            push(lines, "recurrence.ordinal", ordinal.as_str());
            push(lines, "recurrence.weekday", weekday.as_str());
        }
        RecurrenceCadence::Yearly { month, day } => {
            push(lines, "recurrence.month", &month.to_string());
            push(lines, "recurrence.day", &day.to_string());
        }
    }
}

fn decode_rule(fields: &mut BTreeMap<String, String>) -> Result<Option<RecurrenceRule>> {
    let kind = take_required(fields, "recurrence.kind")?;
    if kind == "none" {
        return Ok(None);
    }
    let starts_on = take_date(fields, "recurrence.starts_on")?.ok_or_else(|| {
        KernelError::Codec("recurrence.starts_on cannot be empty when recurrence exists".into())
    })?;
    let interval = parse_required(fields, "recurrence.interval")?;
    let ends_on = take_date(fields, "recurrence.ends_on")?;
    let max_occurrences = take_optional(fields, "recurrence.max_occurrences")?
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| KernelError::Codec("recurrence.max_occurrences is not a u32".into()))
        })
        .transpose()?;
    let exclusions = parse_dates(&take_required(fields, "recurrence.exclusions")?)?;
    let cadence = match kind.as_str() {
        "daily" => RecurrenceCadence::Daily,
        "weekly" => {
            let values = take_required(fields, "recurrence.weekdays")?;
            let weekdays = if values.is_empty() {
                BTreeSet::new()
            } else {
                values
                    .split(',')
                    .map(Weekday::from_str)
                    .collect::<Result<BTreeSet<_>>>()?
            };
            RecurrenceCadence::Weekly { weekdays }
        }
        "monthly_day" => RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::DayOfMonth(parse_required(fields, "recurrence.day")?),
        },
        "monthly_nth_weekday" => RecurrenceCadence::Monthly {
            pattern: MonthlyPattern::NthWeekday {
                ordinal: parse_ordinal(&take_required(fields, "recurrence.ordinal")?)?,
                weekday: Weekday::from_str(&take_required(fields, "recurrence.weekday")?)?,
            },
        },
        "yearly" => RecurrenceCadence::Yearly {
            month: parse_required(fields, "recurrence.month")?,
            day: parse_required(fields, "recurrence.day")?,
        },
        _ => {
            return Err(KernelError::Codec(format!(
                "unknown recurrence kind {kind:?}"
            )));
        }
    };
    let rule = RecurrenceRule {
        starts_on,
        cadence,
        interval,
        ends_on,
        max_occurrences,
        exclusions,
    };
    rule.validate()?;
    Ok(Some(rule))
}

fn parse_ordinal(value: &str) -> Result<WeekOrdinal> {
    match value {
        "first" => Ok(WeekOrdinal::First),
        "second" => Ok(WeekOrdinal::Second),
        "third" => Ok(WeekOrdinal::Third),
        "fourth" => Ok(WeekOrdinal::Fourth),
        "fifth" => Ok(WeekOrdinal::Fifth),
        "last" => Ok(WeekOrdinal::Last),
        _ => Err(KernelError::Codec(format!(
            "unknown week ordinal {value:?}"
        ))),
    }
}

fn encode_completed(lines: &mut Vec<String>, occurrences: &BTreeSet<OccurrenceId>) {
    push(
        lines,
        "completed_occurrences",
        &occurrences
            .iter()
            .map(|occurrence| {
                format!(
                    "{}|{}|{}",
                    occurrence.series_id, occurrence.scheduled_for, occurrence.sequence
                )
            })
            .collect::<Vec<_>>()
            .join(","),
    );
}

fn decode_completed(fields: &mut BTreeMap<String, String>) -> Result<BTreeSet<OccurrenceId>> {
    let encoded = take_required(fields, "completed_occurrences")?;
    if encoded.is_empty() {
        return Ok(BTreeSet::new());
    }
    encoded
        .split(',')
        .map(|value| {
            let mut parts = value.split('|');
            let series_id = parts
                .next()
                .ok_or_else(|| KernelError::Codec("completed occurrence lacks series id".into()))?
                .parse()?;
            let scheduled_for = parts
                .next()
                .ok_or_else(|| KernelError::Codec("completed occurrence lacks date".into()))?
                .parse()?;
            let sequence = parts
                .next()
                .ok_or_else(|| KernelError::Codec("completed occurrence lacks sequence".into()))?
                .parse::<u32>()
                .map_err(|_| KernelError::Codec("occurrence sequence is not a u32".into()))?;
            if parts.next().is_some() {
                return Err(KernelError::Codec(
                    "completed occurrence contains extra fields".into(),
                ));
            }
            Ok(OccurrenceId {
                series_id,
                scheduled_for,
                sequence,
            })
        })
        .collect()
}

fn push(lines: &mut Vec<String>, key: &str, value: &str) {
    lines.push(format!("{key}={}", encode_text(value)));
}

fn push_optional(lines: &mut Vec<String>, key: &str, value: Option<&str>) {
    push(lines, key, value.unwrap_or_default());
}

fn push_id(lines: &mut Vec<String>, key: &str, value: Option<&ObjectId>) {
    push_optional(lines, key, value.map(ObjectId::as_str));
}

fn push_date(lines: &mut Vec<String>, key: &str, value: Option<CivilDate>) {
    push_optional(lines, key, value.map(|date| date.to_string()).as_deref());
}

fn take_required(fields: &mut BTreeMap<String, String>, key: &str) -> Result<String> {
    fields
        .remove(key)
        .ok_or_else(|| KernelError::Codec(format!("missing field {key:?}")))
}

fn take_optional(fields: &mut BTreeMap<String, String>, key: &str) -> Result<Option<String>> {
    let value = take_required(fields, key)?;
    Ok((!value.is_empty()).then_some(value))
}

fn parse_required<T>(fields: &mut BTreeMap<String, String>, key: &str) -> Result<T>
where
    T: FromStr,
{
    take_required(fields, key)?.parse::<T>().map_err(|_| {
        KernelError::Codec(format!("field {key:?} does not contain the expected type"))
    })
}

fn take_id(fields: &mut BTreeMap<String, String>, key: &str) -> Result<Option<ObjectId>> {
    take_optional(fields, key)?
        .map(|value| value.parse())
        .transpose()
}

fn take_date(fields: &mut BTreeMap<String, String>, key: &str) -> Result<Option<CivilDate>> {
    take_optional(fields, key)?
        .map(|value| value.parse())
        .transpose()
}

fn parse_dates(value: &str) -> Result<BTreeSet<CivilDate>> {
    if value.is_empty() {
        return Ok(BTreeSet::new());
    }
    value.split(',').map(str::parse).collect()
}

fn encode_text(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'-' | b'_' | b'.' | b':' | b'/') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    output
}

fn decode_text(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(KernelError::Codec("truncated percent escape".into()));
            }
            output.push((decode_nibble(bytes[index + 1])? << 4) | decode_nibble(bytes[index + 2])?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| KernelError::Codec("decoded field is not UTF-8".into()))
}

fn decode_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(KernelError::Codec(
            "percent escape must use uppercase hexadecimal".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(kind: &str, title: &str) -> ObjectMeta {
        ObjectMeta {
            details: Default::default(),
            id: ObjectId::derive(kind, title.as_bytes()).unwrap(),
            title: title.into(),
            data_class: DataClass::Internal,
            revision: 1,
        }
    }

    #[test]
    fn all_entity_types_round_trip_canonically() {
        let recurrence = RecurrenceRule {
            starts_on: CivilDate::new(2026, 8, 1).unwrap(),
            cadence: RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::NthWeekday {
                    ordinal: WeekOrdinal::Last,
                    weekday: Weekday::Friday,
                },
            },
            interval: 2,
            ends_on: Some(CivilDate::new(2027, 8, 1).unwrap()),
            max_occurrences: Some(5),
            exclusions: [CivilDate::new(2026, 10, 30).unwrap()]
                .into_iter()
                .collect(),
        };
        let entities = [
            Entity::Category(Category {
                meta: meta("category", "Life & Work = together"),
                color: Some("#DAA520".into()),
                sort_order: -1,
            }),
            Entity::Aim(Aim {
                folder: None,
                meta: meta("aim", "Ship Ω"),
                category_id: None,
                state: AimState::Active,
                target_on: Some(CivilDate::new(2026, 12, 31).unwrap()),
            }),
            Entity::Task(Task {
                meta: meta("task", "Write\na robust codec"),
                category_id: None,
                aim_id: None,
                priority: Priority::High,
                status: TaskStatus::InProgress,
                scheduled_for: Some(CivilDate::new(2026, 8, 1).unwrap()),
                due_on: None,
                recurrence: Some(recurrence.clone()),
                completed_occurrences: BTreeSet::new(),
            }),
            Entity::Reminder(Reminder {
                meta: meta("reminder", "Review"),
                related_to: None,
                scheduled_for: CivilDate::new(2026, 8, 2).unwrap(),
                state: ReminderState::Pending,
                recurrence: None,
                completed_occurrences: BTreeSet::new(),
            }),
            Entity::Routine(Routine {
                meta: meta("routine", "Walk"),
                category_id: None,
                state: RoutineState::Active,
                recurrence,
                completed_occurrences: BTreeSet::new(),
            }),
        ];
        for entity in entities {
            let encoded = entity.canonical_bytes().unwrap();
            let decoded = Entity::from_canonical_bytes(&encoded).unwrap();
            assert_eq!(decoded, entity);
            assert_eq!(decoded.canonical_bytes().unwrap(), encoded);
        }
    }

    #[test]
    fn aim_has_no_authoritative_progress_field() {
        let aim = Entity::Aim(Aim {
            folder: None,
            meta: meta("aim", "No mutable progress"),
            category_id: None,
            state: AimState::Active,
            target_on: None,
        });
        let encoded = String::from_utf8(aim.canonical_bytes().unwrap()).unwrap();
        assert!(!encoded.lines().any(|line| line.starts_with("progress=")));
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let category = Entity::Category(Category {
            meta: meta("category", "Strict"),
            color: None,
            sort_order: 0,
        });
        let mut encoded = String::from_utf8(category.canonical_bytes().unwrap()).unwrap();
        encoded.push_str("invented=true\n");
        assert!(Entity::from_canonical_bytes(encoded.as_bytes()).is_err());
    }

    #[test]
    fn completed_occurrence_must_belong_to_the_object() {
        let foreign = ObjectId::derive("task", b"foreign").unwrap();
        let task = Entity::Task(Task {
            meta: meta("task", "Owner"),
            category_id: None,
            aim_id: None,
            priority: Priority::Medium,
            status: TaskStatus::Todo,
            scheduled_for: None,
            due_on: None,
            recurrence: None,
            completed_occurrences: [OccurrenceId {
                series_id: foreign,
                scheduled_for: CivilDate::new(2026, 8, 1).unwrap(),
                sequence: 1,
            }]
            .into_iter()
            .collect(),
        });
        assert!(task.validate().is_err());
    }
}
