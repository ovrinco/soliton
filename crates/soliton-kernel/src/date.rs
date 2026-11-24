use std::fmt;
use std::str::FromStr;

use crate::error::{KernelError, Result};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CivilDate {
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self> {
        if !(1..=9999).contains(&year) {
            return Err(KernelError::InvalidDate(
                "year must be in the inclusive range 1..=9999".into(),
            ));
        }
        if !(1..=12).contains(&month) {
            return Err(KernelError::InvalidDate("month must be in 1..=12".into()));
        }
        if day == 0 || day > Self::days_in_month(year, month) {
            return Err(KernelError::InvalidDate(format!(
                "day {day} does not exist in {year:04}-{month:02}"
            )));
        }
        Ok(Self { year, month, day })
    }

    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    #[must_use]
    pub const fn is_leap_year(year: i32) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    }

    #[must_use]
    pub const fn days_in_month(year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if Self::is_leap_year(year) => 29,
            2 => 28,
            _ => 0,
        }
    }

    pub fn add_days(self, days: i64) -> Result<Self> {
        let serial = days_from_civil(self.year, self.month, self.day)
            .checked_add(days)
            .ok_or_else(|| KernelError::InvalidDate("day arithmetic overflowed".into()))?;
        civil_from_days(serial)
    }

    pub fn add_months_clamped(self, months: i64) -> Result<Self> {
        let base = i64::from(self.year)
            .checked_mul(12)
            .and_then(|value| value.checked_add(i64::from(self.month) - 1))
            .ok_or_else(|| KernelError::InvalidDate("month arithmetic overflowed".into()))?;
        let target = base
            .checked_add(months)
            .ok_or_else(|| KernelError::InvalidDate("month arithmetic overflowed".into()))?;
        let year = i32::try_from(target.div_euclid(12))
            .map_err(|_| KernelError::InvalidDate("year is outside supported range".into()))?;
        let month = u8::try_from(target.rem_euclid(12) + 1)
            .map_err(|_| KernelError::InvalidDate("month conversion failed".into()))?;
        let day = self.day.min(Self::days_in_month(year, month));
        Self::new(year, month, day)
    }

    #[must_use]
    pub fn weekday(self) -> Weekday {
        let monday_zero = (days_from_civil(self.year, self.month, self.day) + 3).rem_euclid(7);
        Weekday::from_monday_index(u8::try_from(monday_zero).expect("remainder is 0..=6"))
    }

    pub fn start_of_week(self) -> Result<Self> {
        self.add_days(-i64::from(self.weekday().monday_index()))
    }

    pub fn end_of_week(self) -> Result<Self> {
        self.start_of_week()?.add_days(6)
    }

    pub fn start_of_month(self) -> Result<Self> {
        Self::new(self.year, self.month, 1)
    }

    pub fn end_of_month(self) -> Result<Self> {
        Self::new(
            self.year,
            self.month,
            Self::days_in_month(self.year, self.month),
        )
    }

    pub fn start_of_year(self) -> Result<Self> {
        Self::new(self.year, 1, 1)
    }

    pub fn end_of_year(self) -> Result<Self> {
        Self::new(self.year, 12, 31)
    }

    #[must_use]
    pub fn days_until(self, later: Self) -> i64 {
        days_from_civil(later.year, later.month, later.day)
            - days_from_civil(self.year, self.month, self.day)
    }
}

impl fmt::Display for CivilDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

impl FromStr for CivilDate {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() != 10 || &value[4..5] != "-" || &value[7..8] != "-" {
            return Err(KernelError::InvalidDate(
                "date must use the YYYY-MM-DD form".into(),
            ));
        }
        let year = value[0..4]
            .parse::<i32>()
            .map_err(|_| KernelError::InvalidDate("year is not numeric".into()))?;
        let month = value[5..7]
            .parse::<u8>()
            .map_err(|_| KernelError::InvalidDate("month is not numeric".into()))?;
        let day = value[8..10]
            .parse::<u8>()
            .map_err(|_| KernelError::InvalidDate("day is not numeric".into()))?;
        Self::new(year, month, day)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    #[must_use]
    pub const fn monday_index(self) -> u8 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }

    #[must_use]
    pub const fn from_monday_index(index: u8) -> Self {
        match index % 7 {
            0 => Self::Monday,
            1 => Self::Tuesday,
            2 => Self::Wednesday,
            3 => Self::Thursday,
            4 => Self::Friday,
            5 => Self::Saturday,
            _ => Self::Sunday,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monday => "mon",
            Self::Tuesday => "tue",
            Self::Wednesday => "wed",
            Self::Thursday => "thu",
            Self::Friday => "fri",
            Self::Saturday => "sat",
            Self::Sunday => "sun",
        }
    }
}

impl FromStr for Weekday {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "mon" => Ok(Self::Monday),
            "tue" => Ok(Self::Tuesday),
            "wed" => Ok(Self::Wednesday),
            "thu" => Ok(Self::Thursday),
            "fri" => Ok(Self::Friday),
            "sat" => Ok(Self::Saturday),
            "sun" => Ok(Self::Sunday),
            _ => Err(KernelError::InvalidDate(format!(
                "unknown weekday {value:?}"
            ))),
        }
    }
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let adjusted_year = i64::from(year) - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> Result<CivilDate> {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    CivilDate::new(
        i32::try_from(year)
            .map_err(|_| KernelError::InvalidDate("year is outside supported range".into()))?,
        u8::try_from(month)
            .map_err(|_| KernelError::InvalidDate("month conversion failed".into()))?,
        u8::try_from(day).map_err(|_| KernelError::InvalidDate("day conversion failed".into()))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_year_rules_are_gregorian() {
        assert!(CivilDate::is_leap_year(2000));
        assert!(!CivilDate::is_leap_year(1900));
        assert!(CivilDate::is_leap_year(2024));
    }

    #[test]
    fn day_arithmetic_crosses_year_boundaries() {
        let date = CivilDate::new(2025, 12, 31).unwrap();
        assert_eq!(date.add_days(1).unwrap().to_string(), "2026-01-01");
        assert_eq!(date.add_days(-365).unwrap().to_string(), "2024-12-31");
    }

    #[test]
    fn month_arithmetic_clamps_without_losing_anchor_input() {
        let date = CivilDate::new(2024, 1, 31).unwrap();
        assert_eq!(
            date.add_months_clamped(1).unwrap().to_string(),
            "2024-02-29"
        );
        assert_eq!(
            date.add_months_clamped(2).unwrap().to_string(),
            "2024-03-31"
        );
    }

    #[test]
    fn weekdays_match_known_dates() {
        assert_eq!(
            CivilDate::new(1970, 1, 1).unwrap().weekday(),
            Weekday::Thursday
        );
        assert_eq!(
            CivilDate::new(2026, 8, 1).unwrap().weekday(),
            Weekday::Saturday
        );
    }

    #[test]
    fn parser_rejects_impossible_dates() {
        assert!("2025-02-29".parse::<CivilDate>().is_err());
        assert!("2025-2-01".parse::<CivilDate>().is_err());
    }
}
