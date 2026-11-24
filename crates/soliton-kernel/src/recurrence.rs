use std::collections::BTreeSet;

use crate::date::{CivilDate, Weekday};
use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::{ObjectId, OccurrenceId};

const MAX_SCANNED_SLOTS: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeekOrdinal {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    Last,
}

impl WeekOrdinal {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Second => "second",
            Self::Third => "third",
            Self::Fourth => "fourth",
            Self::Fifth => "fifth",
            Self::Last => "last",
        }
    }

    #[must_use]
    pub const fn one_based(self) -> Option<u8> {
        match self {
            Self::First => Some(1),
            Self::Second => Some(2),
            Self::Third => Some(3),
            Self::Fourth => Some(4),
            Self::Fifth => Some(5),
            Self::Last => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MonthlyPattern {
    DayOfMonth(u8),
    NthWeekday {
        ordinal: WeekOrdinal,
        weekday: Weekday,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecurrenceCadence {
    Daily,
    Weekly { weekdays: BTreeSet<Weekday> },
    Monthly { pattern: MonthlyPattern },
    Yearly { month: u8, day: u8 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecurrenceRule {
    pub starts_on: CivilDate,
    pub cadence: RecurrenceCadence,
    pub interval: u16,
    pub ends_on: Option<CivilDate>,
    pub max_occurrences: Option<u32>,
    pub exclusions: BTreeSet<CivilDate>,
}

impl RecurrenceRule {
    pub fn validate(&self) -> Result<()> {
        if self.interval == 0 {
            return Err(KernelError::InvalidRecurrence(
                "interval must be at least one".into(),
            ));
        }
        if self.max_occurrences == Some(0) {
            return Err(KernelError::InvalidRecurrence(
                "max_occurrences must be at least one when present".into(),
            ));
        }
        if self.ends_on.is_some_and(|end| end < self.starts_on) {
            return Err(KernelError::InvalidRecurrence(
                "ends_on precedes starts_on".into(),
            ));
        }
        match &self.cadence {
            RecurrenceCadence::Weekly { weekdays } if weekdays.is_empty() => {
                return Err(KernelError::InvalidRecurrence(
                    "weekly recurrence requires at least one weekday".into(),
                ));
            }
            RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::DayOfMonth(day),
            } if !(1..=31).contains(day) => {
                return Err(KernelError::InvalidRecurrence(
                    "monthly day must be in 1..=31".into(),
                ));
            }
            RecurrenceCadence::Yearly { month, day }
                if !(1..=12).contains(month)
                    || *day == 0
                    || *day > CivilDate::days_in_month(2000, *month) =>
            {
                return Err(KernelError::InvalidRecurrence(
                    "yearly month/day is not a possible Gregorian date".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Occurrence {
    pub id: OccurrenceId,
    pub on: CivilDate,
}

/// Durable, monotonic materialization state for one recurrence series.
///
/// The cursor is deliberately separate from canonical entities: it records
/// which occurrences a consumer has applied while the rule and entity remain
/// the source of truth. Exclusions consume their scheduled sequence, so a
/// valid cursor may contain sequence gaps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecurrenceCursor {
    pub series_id: ObjectId,
    pub rule_hash: ContentHash,
    /// Opaque caller provenance retained with the durable cursor. The rule
    /// hash remains the semantic binding; provenance records which admitted
    /// application path supplied the cursor update.
    pub rule_provenance: String,
    pub last_materialized_date: Option<CivilDate>,
    pub last_sequence: Option<u32>,
    pub next_window_start: CivilDate,
    pub applied: BTreeSet<OccurrenceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecurrenceCursorStatus {
    Missing,
    Ready(RecurrenceCursor),
    Hold(String),
}

impl RecurrenceCursor {
    pub fn initial(series_id: ObjectId, rule: &RecurrenceRule) -> Result<Self> {
        rule.validate()?;
        Ok(Self {
            series_id,
            rule_hash: rule.content_hash(),
            rule_provenance: String::new(),
            last_materialized_date: None,
            last_sequence: None,
            next_window_start: rule.starts_on,
            applied: BTreeSet::new(),
        })
    }

    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        ContentHash::digest(&self.to_bytes())
    }

    pub fn validate_against_rule(&self, series_id: &ObjectId, rule: &RecurrenceRule) -> Result<()> {
        rule.validate()?;
        if &self.series_id != series_id {
            return Err(KernelError::Integrity(format!(
                "recurrence cursor series {} does not match {}",
                self.series_id, series_id
            )));
        }
        let expected_hash = rule.content_hash();
        if self.rule_hash != expected_hash {
            return Err(KernelError::Conflict(
                "recurrence cursor rule hash differs from the supplied rule".into(),
            ));
        }
        match (self.last_materialized_date, self.last_sequence) {
            (None, None) if self.next_window_start == rule.starts_on => {
                if !self.applied.is_empty() {
                    return Err(KernelError::Integrity(
                        "initial recurrence cursor must have empty applied history".into(),
                    ));
                }
            }
            (Some(last_date), Some(last_sequence)) => {
                if self.next_window_start != last_date.add_days(1)? {
                    return Err(KernelError::Integrity(
                        "recurrence cursor next window is inconsistent with its last date".into(),
                    ));
                }
                if self.applied.iter().any(|occurrence| {
                    occurrence.series_id != *series_id
                        || occurrence.scheduled_for > last_date
                        || occurrence.sequence > last_sequence
                }) {
                    return Err(KernelError::Integrity(
                        "recurrence cursor applied history exceeds its durable boundary".into(),
                    ));
                }
                self.validate_applied_history(rule, last_date)?;
            }
            _ => {
                return Err(KernelError::Integrity(
                    "recurrence cursor must have both or neither last boundary field".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_applied_history(
        &self,
        rule: &RecurrenceRule,
        through: CivilDate,
    ) -> Result<()> {
        let expected = materialize_occurrences(&self.series_id, rule, rule.starts_on, through)?
            .into_iter()
            .map(|occurrence| occurrence.id)
            .collect::<BTreeSet<_>>();
        if self.applied != expected {
            return Err(KernelError::Integrity(
                "applied history does not equal all materialized occurrences through the durable boundary".into(),
            ));
        }
        let Some(last) = expected.iter().max_by(|left, right| {
            left.scheduled_for
                .cmp(&right.scheduled_for)
                .then(left.sequence.cmp(&right.sequence))
        }) else {
            return Err(KernelError::Integrity(
                "non-initial recurrence cursor has no materialized applied occurrence".into(),
            ));
        };
        if last.scheduled_for != through || Some(last.sequence) != self.last_sequence {
            return Err(KernelError::Integrity(
                "recurrence cursor boundary does not match its last applied occurrence".into(),
            ));
        }
        Ok(())
    }

    /// Bind the opaque provenance supplied by a repository-owned application
    /// path. A loaded cursor may only be reused by the same provenance.
    pub fn bind_rule_provenance(&mut self, provenance: impl Into<String>) -> Result<()> {
        let provenance = provenance.into();
        if provenance.trim().is_empty()
            || provenance
                .bytes()
                .any(|byte| matches!(byte, b'\n' | b'\r' | b'='))
        {
            return Err(KernelError::Integrity(
                "recurrence rule provenance is empty or malformed".into(),
            ));
        }
        if self.rule_provenance.is_empty() {
            self.rule_provenance = provenance;
            return Ok(());
        }
        if self.rule_provenance != provenance {
            return Err(KernelError::Conflict(
                "recurrence rule provenance differs from the durable cursor".into(),
            ));
        }
        Ok(())
    }

    pub fn apply_occurrence(
        &mut self,
        occurrence: &Occurrence,
        rule: &RecurrenceRule,
    ) -> Result<bool> {
        self.validate_against_rule(&self.series_id.clone(), rule)?;
        let expected =
            materialize_occurrences(&self.series_id, rule, occurrence.on, occurrence.on)?
                .into_iter()
                .find(|candidate| candidate.id == occurrence.id);
        if expected.is_none() {
            return Err(KernelError::InvalidRecurrence(
                "occurrence is not produced by the bound rule".into(),
            ));
        }
        if self.applied.contains(&occurrence.id) {
            return Ok(false);
        }
        if self
            .last_materialized_date
            .is_some_and(|last| occurrence.on <= last)
            || self
                .last_sequence
                .is_some_and(|last| occurrence.id.sequence <= last)
        {
            return Err(KernelError::Conflict(
                "recurrence cursor rejects out-of-order or colliding occurrence".into(),
            ));
        }
        self.applied.insert(occurrence.id.clone());
        self.last_materialized_date = Some(occurrence.on);
        self.last_sequence = Some(occurrence.id.sequence);
        self.next_window_start = occurrence.on.add_days(1)?;
        Ok(true)
    }

    /// Stable v2 bytes. The rule hash intentionally excludes the cursor's
    /// mutable fields; this prevents circular hashing while binding the rule.
    pub fn to_bytes(&self) -> Vec<u8> {
        let applied = self
            .applied
            .iter()
            .map(|occurrence| {
                format!(
                    "{}|{}|{}",
                    occurrence.series_id, occurrence.scheduled_for, occurrence.sequence
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "version=2\nseries_id={}\nrule_hash={}\nrule_provenance={}\nlast_materialized_date={}\nlast_sequence={}\nnext_window_start={}\napplied={}\n",
            self.series_id,
            self.rule_hash,
            self.rule_provenance,
            self.last_materialized_date.map_or_else(String::new, |v| v.to_string()),
            self.last_sequence.map_or_else(String::new, |v| v.to_string()),
            self.next_window_start,
            applied
        )
        .into_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut fields = std::collections::BTreeMap::new();
        for line in std::str::from_utf8(bytes)
            .map_err(|_| KernelError::Codec("cursor is not UTF-8".into()))?
            .lines()
        {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| KernelError::Codec("cursor line lacks '='".into()))?;
            if fields.insert(key, value).is_some() {
                return Err(KernelError::Codec(
                    "cursor contains duplicate fields".into(),
                ));
            }
        }
        let version = fields
            .remove("version")
            .ok_or_else(|| KernelError::Codec("cursor lacks version".into()))?;
        if version != "1" && version != "2" {
            return Err(KernelError::Codec("unsupported cursor version".into()));
        }
        let series_id = fields
            .remove("series_id")
            .ok_or_else(|| KernelError::Codec("cursor lacks series_id".into()))?
            .parse()?;
        let rule_hash = fields
            .remove("rule_hash")
            .ok_or_else(|| KernelError::Codec("cursor lacks rule_hash".into()))?
            .parse()?;
        let rule_provenance = if version == "2" {
            fields
                .remove("rule_provenance")
                .ok_or_else(|| KernelError::Codec("cursor lacks rule provenance".into()))?
        } else {
            fields.remove("rule_provenance").unwrap_or_default()
        }
        .to_string();
        let last_materialized_date = fields
            .remove("last_materialized_date")
            .ok_or_else(|| KernelError::Codec("cursor lacks last_materialized_date".into()))?;
        let last_materialized_date = if last_materialized_date.is_empty() {
            None
        } else {
            Some(last_materialized_date.parse()?)
        };
        let last_sequence = fields
            .remove("last_sequence")
            .ok_or_else(|| KernelError::Codec("cursor lacks last_sequence".into()))?;
        let last_sequence = if last_sequence.is_empty() {
            None
        } else {
            Some(
                last_sequence
                    .parse()
                    .map_err(|_| KernelError::Codec("cursor last_sequence is not a u32".into()))?,
            )
        };
        let next_window_start = fields
            .remove("next_window_start")
            .ok_or_else(|| KernelError::Codec("cursor lacks next_window_start".into()))?
            .parse()?;
        let encoded = fields
            .remove("applied")
            .ok_or_else(|| KernelError::Codec("cursor lacks applied history".into()))?;
        let mut applied = BTreeSet::new();
        if !encoded.is_empty() {
            for value in encoded.split(',') {
                let mut parts = value.split('|');
                let occurrence = OccurrenceId {
                    series_id: parts
                        .next()
                        .ok_or_else(|| KernelError::Codec("cursor occurrence lacks series".into()))?
                        .parse()?,
                    scheduled_for: parts
                        .next()
                        .ok_or_else(|| KernelError::Codec("cursor occurrence lacks date".into()))?
                        .parse()?,
                    sequence: parts
                        .next()
                        .ok_or_else(|| {
                            KernelError::Codec("cursor occurrence lacks sequence".into())
                        })?
                        .parse()
                        .map_err(|_| KernelError::Codec("cursor sequence is not a u32".into()))?,
                };
                if parts.next().is_some() || !applied.insert(occurrence) {
                    return Err(KernelError::Codec(
                        "cursor occurrence is malformed or duplicated".into(),
                    ));
                }
            }
        }
        if !fields.is_empty() {
            return Err(KernelError::Codec("cursor contains unknown fields".into()));
        }
        Ok(Self {
            series_id,
            rule_hash,
            rule_provenance,
            last_materialized_date,
            last_sequence,
            next_window_start,
            applied,
        })
    }
}

impl RecurrenceRule {
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let weekdays = match &self.cadence {
            RecurrenceCadence::Weekly { weekdays } => weekdays
                .iter()
                .map(|weekday| weekday.as_str())
                .collect::<Vec<_>>()
                .join(","),
            _ => String::new(),
        };
        let cadence = match &self.cadence {
            RecurrenceCadence::Daily => "daily".to_owned(),
            RecurrenceCadence::Weekly { .. } => format!("weekly:{weekdays}"),
            RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::DayOfMonth(day),
            } => format!("monthly_day:{day}"),
            RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::NthWeekday { ordinal, weekday },
            } => format!("monthly_nth:{}:{}", ordinal.as_str(), weekday.as_str()),
            RecurrenceCadence::Yearly { month, day } => format!("yearly:{month}:{day}"),
        };
        let exclusions = self
            .exclusions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!("v1\nstarts_on={}\ncadence={cadence}\ninterval={}\nends_on={}\nmax_occurrences={}\nexclusions={exclusions}\n", self.starts_on, self.interval, self.ends_on.map_or_else(String::new, |v| v.to_string()), self.max_occurrences.map_or_else(String::new, |v| v.to_string())).into_bytes()
    }

    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        ContentHash::digest(&self.canonical_bytes())
    }
}

/// Materializes occurrences in an inclusive window.
///
/// `sequence` counts scheduled slots from the beginning of the series. An
/// exclusion suppresses a slot but still consumes its sequence and any
/// `max_occurrences` budget, so excluding a date never silently extends a series.
pub fn materialize_occurrences(
    series_id: &ObjectId,
    rule: &RecurrenceRule,
    from: CivilDate,
    through: CivilDate,
) -> Result<Vec<Occurrence>> {
    rule.validate()?;
    if from > through {
        return Err(KernelError::InvalidRecurrence(
            "materialization window starts after it ends".into(),
        ));
    }

    let effective_end = rule.ends_on.map_or(through, |end| end.min(through));
    if effective_end < rule.starts_on || effective_end < from {
        return Ok(Vec::new());
    }

    let candidates = candidates_through(rule, effective_end)?;
    let mut output = Vec::new();
    for (zero_based, candidate) in candidates.into_iter().enumerate() {
        let sequence = u32::try_from(zero_based + 1).map_err(|_| {
            KernelError::InvalidRecurrence("occurrence sequence exceeds u32".into())
        })?;
        if rule
            .max_occurrences
            .is_some_and(|maximum| sequence > maximum)
        {
            break;
        }
        if candidate < from || rule.exclusions.contains(&candidate) {
            continue;
        }
        output.push(Occurrence {
            id: OccurrenceId {
                series_id: series_id.clone(),
                scheduled_for: candidate,
                sequence,
            },
            on: candidate,
        });
    }
    Ok(output)
}

fn candidates_through(rule: &RecurrenceRule, through: CivilDate) -> Result<Vec<CivilDate>> {
    match &rule.cadence {
        RecurrenceCadence::Daily => daily_candidates(rule, through),
        RecurrenceCadence::Weekly { weekdays } => weekly_candidates(rule, weekdays, through),
        RecurrenceCadence::Monthly { pattern } => monthly_candidates(rule, pattern, through),
        RecurrenceCadence::Yearly { month, day } => yearly_candidates(rule, *month, *day, through),
    }
}

fn daily_candidates(rule: &RecurrenceRule, through: CivilDate) -> Result<Vec<CivilDate>> {
    let mut output = Vec::new();
    let mut slot = 0_u32;
    loop {
        enforce_scan_bound(slot)?;
        let offset = i64::from(slot)
            .checked_mul(i64::from(rule.interval))
            .ok_or_else(|| KernelError::InvalidRecurrence("daily offset overflowed".into()))?;
        let candidate = rule.starts_on.add_days(offset)?;
        if candidate > through {
            break;
        }
        output.push(candidate);
        slot = slot.saturating_add(1);
    }
    Ok(output)
}

fn weekly_candidates(
    rule: &RecurrenceRule,
    weekdays: &BTreeSet<Weekday>,
    through: CivilDate,
) -> Result<Vec<CivilDate>> {
    let week_zero = rule.starts_on.start_of_week()?;
    let total_days = rule.starts_on.days_until(through);
    if total_days > i64::from(MAX_SCANNED_SLOTS) * 7 {
        return Err(KernelError::InvalidRecurrence(
            "weekly recurrence exceeds the materialization scan bound".into(),
        ));
    }
    let mut output = Vec::new();
    let mut candidate = rule.starts_on;
    while candidate <= through {
        let weeks = week_zero.days_until(candidate).div_euclid(7);
        if weeks.rem_euclid(i64::from(rule.interval)) == 0
            && weekdays.contains(&candidate.weekday())
        {
            output.push(candidate);
        }
        candidate = candidate.add_days(1)?;
    }
    Ok(output)
}

fn monthly_candidates(
    rule: &RecurrenceRule,
    pattern: &MonthlyPattern,
    through: CivilDate,
) -> Result<Vec<CivilDate>> {
    let mut output = Vec::new();
    let mut slot = 0_u32;
    loop {
        enforce_scan_bound(slot)?;
        let months = i64::from(slot)
            .checked_mul(i64::from(rule.interval))
            .ok_or_else(|| KernelError::InvalidRecurrence("monthly offset overflowed".into()))?;
        let month_anchor = CivilDate::new(rule.starts_on.year(), rule.starts_on.month(), 1)?
            .add_months_clamped(months)?;
        if month_anchor > through {
            break;
        }
        if let Some(candidate) =
            monthly_candidate(month_anchor.year(), month_anchor.month(), pattern)?
        {
            if candidate >= rule.starts_on && candidate <= through {
                output.push(candidate);
            }
        }
        slot = slot.saturating_add(1);
    }
    Ok(output)
}

fn monthly_candidate(year: i32, month: u8, pattern: &MonthlyPattern) -> Result<Option<CivilDate>> {
    match pattern {
        MonthlyPattern::DayOfMonth(day) => Ok(Some(CivilDate::new(
            year,
            month,
            (*day).min(CivilDate::days_in_month(year, month)),
        )?)),
        MonthlyPattern::NthWeekday { ordinal, weekday } => {
            nth_weekday_of_month(year, month, *ordinal, *weekday)
        }
    }
}

fn nth_weekday_of_month(
    year: i32,
    month: u8,
    ordinal: WeekOrdinal,
    weekday: Weekday,
) -> Result<Option<CivilDate>> {
    if let Some(one_based) = ordinal.one_based() {
        let first = CivilDate::new(year, month, 1)?;
        let shift =
            (7 + i16::from(weekday.monday_index()) - i16::from(first.weekday().monday_index())) % 7;
        let day = 1 + u8::try_from(shift).expect("weekday shift is 0..=6") + 7 * (one_based - 1);
        if day > CivilDate::days_in_month(year, month) {
            return Ok(None);
        }
        return Ok(Some(CivilDate::new(year, month, day)?));
    }

    let last_day = CivilDate::days_in_month(year, month);
    let last = CivilDate::new(year, month, last_day)?;
    let shift =
        (7 + i16::from(last.weekday().monday_index()) - i16::from(weekday.monday_index())) % 7;
    let day = last_day - u8::try_from(shift).expect("weekday shift is 0..=6");
    Ok(Some(CivilDate::new(year, month, day)?))
}

fn yearly_candidates(
    rule: &RecurrenceRule,
    month: u8,
    day: u8,
    through: CivilDate,
) -> Result<Vec<CivilDate>> {
    let mut output = Vec::new();
    let mut slot = 0_u32;
    loop {
        enforce_scan_bound(slot)?;
        let year_offset = i32::from(rule.interval)
            .checked_mul(i32::try_from(slot).map_err(|_| {
                KernelError::InvalidRecurrence("yearly occurrence count overflowed".into())
            })?)
            .ok_or_else(|| KernelError::InvalidRecurrence("yearly offset overflowed".into()))?;
        let year = rule
            .starts_on
            .year()
            .checked_add(year_offset)
            .ok_or_else(|| KernelError::InvalidRecurrence("yearly date overflowed".into()))?;
        let candidate =
            CivilDate::new(year, month, day.min(CivilDate::days_in_month(year, month)))?;
        if candidate > through {
            break;
        }
        if candidate >= rule.starts_on {
            output.push(candidate);
        }
        slot = slot.saturating_add(1);
    }
    Ok(output)
}

fn enforce_scan_bound(slot: u32) -> Result<()> {
    if slot >= MAX_SCANNED_SLOTS {
        return Err(KernelError::InvalidRecurrence(
            "recurrence exceeds the one-million-slot materialization bound".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> ObjectId {
        ObjectId::derive("routine", b"test-series").unwrap()
    }

    fn daily(start: CivilDate) -> RecurrenceRule {
        RecurrenceRule {
            starts_on: start,
            cadence: RecurrenceCadence::Daily,
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        }
    }

    #[test]
    fn daily_window_boundaries_are_inclusive() {
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let result = materialize_occurrences(
            &id(),
            &rule,
            CivilDate::new(2026, 8, 2).unwrap(),
            CivilDate::new(2026, 8, 3).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.iter().map(|item| item.on.day()).collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(result[0].id.sequence, 2);
    }

    #[test]
    fn interval_skips_days_deterministically() {
        let mut rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        rule.interval = 3;
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.iter().map(|item| item.on.day()).collect::<Vec<_>>(),
            [1, 4, 7, 10]
        );
    }

    #[test]
    fn weekly_multiple_days_respect_interval_weeks() {
        let weekdays = [Weekday::Monday, Weekday::Wednesday].into_iter().collect();
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2026, 8, 3).unwrap(),
            cadence: RecurrenceCadence::Weekly { weekdays },
            interval: 2,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.iter().map(|item| item.on.day()).collect::<Vec<_>>(),
            [3, 5, 17, 19, 31]
        );
    }

    #[test]
    fn monthly_day_preserves_anchor_and_clamps_each_month() {
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2024, 1, 31).unwrap(),
            cadence: RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::DayOfMonth(31),
            },
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2024, 4, 30).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.on.to_string())
                .collect::<Vec<_>>(),
            ["2024-01-31", "2024-02-29", "2024-03-31", "2024-04-30"]
        );
    }

    #[test]
    fn fifth_weekday_skips_months_without_one() {
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2026, 1, 1).unwrap(),
            cadence: RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::NthWeekday {
                    ordinal: WeekOrdinal::Fifth,
                    weekday: Weekday::Monday,
                },
            },
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 5, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.on.to_string())
                .collect::<Vec<_>>(),
            ["2026-03-30"]
        );
    }

    #[test]
    fn last_weekday_is_calculated_from_month_end() {
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2026, 2, 1).unwrap(),
            cadence: RecurrenceCadence::Monthly {
                pattern: MonthlyPattern::NthWeekday {
                    ordinal: WeekOrdinal::Last,
                    weekday: Weekday::Friday,
                },
            },
            interval: 1,
            ends_on: None,
            max_occurrences: Some(2),
            exclusions: BTreeSet::new(),
        };
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 3, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.on.to_string())
                .collect::<Vec<_>>(),
            ["2026-02-27", "2026-03-27"]
        );
    }

    #[test]
    fn yearly_february_29_clamps_then_returns_to_leap_day() {
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2024, 2, 29).unwrap(),
            cadence: RecurrenceCadence::Yearly { month: 2, day: 29 },
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2028, 3, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.on.to_string())
                .collect::<Vec<_>>(),
            [
                "2024-02-29",
                "2025-02-28",
                "2026-02-28",
                "2027-02-28",
                "2028-02-29"
            ]
        );
    }

    #[test]
    fn end_date_is_inclusive_and_max_is_enforced() {
        let mut rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        rule.ends_on = Some(CivilDate::new(2026, 8, 4).unwrap());
        rule.max_occurrences = Some(3);
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.last().unwrap().on.day(), 3);
    }

    #[test]
    fn exclusions_do_not_extend_a_max_limited_series() {
        let mut rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        rule.max_occurrences = Some(3);
        rule.exclusions.insert(CivilDate::new(2026, 8, 2).unwrap());
        let result = materialize_occurrences(
            &id(),
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.iter().map(|item| item.on.day()).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(result[1].id.sequence, 3);
    }

    #[test]
    fn repeated_materialization_is_idempotent() {
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let through = CivilDate::new(2026, 8, 5).unwrap();
        assert_eq!(
            materialize_occurrences(&id(), &rule, rule.starts_on, through).unwrap(),
            materialize_occurrences(&id(), &rule, rule.starts_on, through).unwrap()
        );
    }

    #[test]
    fn malformed_rules_fail_closed() {
        let mut rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        rule.interval = 0;
        assert!(materialize_occurrences(&id(), &rule, rule.starts_on, rule.starts_on).is_err());
    }

    #[test]
    fn cursor_accepts_exclusion_sequence_gaps_and_duplicate_replay() {
        let series = id();
        let mut rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        rule.exclusions.insert(CivilDate::new(2026, 8, 2).unwrap());
        let occurrences = materialize_occurrences(
            &series,
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 4).unwrap(),
        )
        .unwrap();
        let mut cursor = RecurrenceCursor::initial(series.clone(), &rule).unwrap();
        assert!(cursor.apply_occurrence(&occurrences[0], &rule).unwrap());
        assert!(cursor.apply_occurrence(&occurrences[1], &rule).unwrap());
        assert!(!cursor.apply_occurrence(&occurrences[0], &rule).unwrap());
        assert_eq!(cursor.last_sequence, Some(3));
        cursor.validate_against_rule(&series, &rule).unwrap();
    }

    #[test]
    fn cursor_rejects_missing_history_and_nonempty_initial_state() {
        let series = id();
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let occurrences = materialize_occurrences(
            &series,
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 3).unwrap(),
        )
        .unwrap();
        let mut cursor = RecurrenceCursor::initial(series.clone(), &rule).unwrap();
        cursor.applied.insert(occurrences[0].id.clone());
        assert!(matches!(
            cursor.validate_against_rule(&series, &rule),
            Err(KernelError::Integrity(_))
        ));

        let mut cursor = RecurrenceCursor::initial(series.clone(), &rule).unwrap();
        for occurrence in &occurrences {
            cursor.apply_occurrence(occurrence, &rule).unwrap();
        }
        cursor.applied.remove(&occurrences[0].id);
        assert!(matches!(
            cursor.validate_against_rule(&series, &rule),
            Err(KernelError::Integrity(_))
        ));
    }

    #[test]
    fn cursor_provenance_round_trips_and_cannot_be_rebound() {
        let series = id();
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let mut cursor = RecurrenceCursor::initial(series, &rule).unwrap();
        cursor.bind_rule_provenance("review").unwrap();
        assert!(cursor.bind_rule_provenance("review").is_ok());
        assert!(matches!(
            cursor.bind_rule_provenance("other"),
            Err(KernelError::Conflict(_))
        ));
        let restored = RecurrenceCursor::from_bytes(&cursor.to_bytes()).unwrap();
        assert_eq!(restored.rule_provenance, "review");
    }

    #[test]
    fn cursor_rejects_changed_rule_series_and_corrupt_boundary() {
        let series = id();
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let mut cursor = RecurrenceCursor::initial(series.clone(), &rule).unwrap();
        let mut changed = rule.clone();
        changed.interval = 2;
        assert!(matches!(
            cursor.validate_against_rule(&series, &changed),
            Err(KernelError::Conflict(_))
        ));
        cursor.next_window_start = CivilDate::new(2026, 8, 9).unwrap();
        assert!(matches!(
            cursor.validate_against_rule(&series, &rule),
            Err(KernelError::Integrity(_))
        ));
        let other = id();
        assert!(matches!(
            cursor.validate_against_rule(&other, &rule),
            Err(KernelError::Integrity(_))
        ));
    }

    #[test]
    fn cursor_round_trip_is_stable() {
        let series = id();
        let rule = daily(CivilDate::new(2026, 8, 1).unwrap());
        let mut cursor = RecurrenceCursor::initial(series, &rule).unwrap();
        let occurrence = materialize_occurrences(
            &cursor.series_id,
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 1).unwrap(),
        )
        .unwrap()
        .pop()
        .unwrap();
        cursor.apply_occurrence(&occurrence, &rule).unwrap();
        let restored = RecurrenceCursor::from_bytes(&cursor.to_bytes()).unwrap();
        assert_eq!(restored, cursor);
    }
}
