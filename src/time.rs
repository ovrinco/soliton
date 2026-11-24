use crate::{Error, Result};
use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset,
    TimeZone, Utc,
};
use chrono_tz::Tz;
use soliton_kernel::details::Schedule;

pub fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| Error::Invalid("Use a valid date in YYYY-MM-DD format".into()))
}
pub fn zone(s: &str) -> Result<Tz> {
    s.parse()
        .map_err(|_| Error::Invalid(format!("Unknown timezone: {s}")))
}

pub fn resolve(day: &str, time: &str, timezone: &str, fold: &str, recurring: bool) -> Result<i64> {
    let local = NaiveDateTime::new(
        date(day)?,
        NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| Error::Invalid("Use a time in HH:MM format".into()))?,
    );
    let tz = zone(timezone)?;
    match tz.from_local_datetime(&local) {
        LocalResult::Single(dt) => Ok(dt.timestamp()),
        LocalResult::Ambiguous(a, b) => match fold {
            "earlier" => Ok(a.timestamp().min(b.timestamp())),
            "later" => Ok(a.timestamp().max(b.timestamp())),
            _ => Err(Error::Invalid(
                "This time occurs twice. Choose Earlier or Later for the repeated hour.".into(),
            )),
        },
        LocalResult::None if recurring => {
            // Find the adjacent valid offsets, then move by the actual gap.
            let before = (1..=1440).find_map(|m| {
                tz.from_local_datetime(&(local - Duration::minutes(m)))
                    .earliest()
            });
            let after = (1..=1440).find_map(|m| {
                tz.from_local_datetime(&(local + Duration::minutes(m)))
                    .earliest()
            });
            match (before, after) {
                (Some(a), Some(b)) => {
                    let gap =
                        b.offset().fix().local_minus_utc() - a.offset().fix().local_minus_utc();
                    tz.from_local_datetime(&(local + Duration::seconds(i64::from(gap))))
                        .earliest()
                        .map(|d| d.timestamp())
                        .ok_or_else(|| {
                            Error::Invalid("Could not resolve recurrence timezone gap".into())
                        })
                }
                _ => Err(Error::Invalid(
                    "Could not resolve recurrence timezone gap".into(),
                )),
            }
        }
        LocalResult::None => Err(Error::Invalid(
            "This local time does not exist because clocks move forward. Choose a valid time."
                .into(),
        )),
    }
}
pub fn scheduled(
    day: &str,
    time: &str,
    duration: u32,
    timezone: &str,
    fold: &str,
) -> Result<Schedule> {
    if duration == 0 || duration > 10080 {
        return Err(Error::Invalid("Duration must be 1–10080 minutes".into()));
    }
    let start_utc = resolve(day, time, timezone, fold, false)?;
    Ok(Schedule {
        time: time.into(),
        duration_minutes: duration,
        timezone: timezone.into(),
        start_utc,
        fold: if fold == "later" { "later" } else { "earlier" }.into(),
    })
}
pub fn local_string(timestamp: i64, timezone: &str) -> Result<String> {
    Ok(DateTime::<Utc>::from_timestamp(timestamp, 0)
        .ok_or_else(|| Error::Invalid("Timestamp out of range".into()))?
        .with_timezone(&zone(timezone)?)
        .format("%Y-%m-%dT%H:%M")
        .to_string())
}
pub fn weekday(s: &str) -> Result<u32> {
    Ok(date(s)?.weekday().num_days_from_monday())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dst_gap_and_fold_are_explicit() {
        assert!(resolve("2026-03-08", "02:30", "America/Toronto", "", false).is_err());
        let shifted = resolve("2026-03-08", "02:30", "America/Toronto", "earlier", true).unwrap();
        assert_eq!(
            local_string(shifted, "America/Toronto").unwrap(),
            "2026-03-08T03:30"
        );
        assert!(resolve("2026-11-01", "01:30", "America/Toronto", "", false).is_err());
        assert_eq!(
            resolve("2026-11-01", "01:30", "America/Toronto", "later", false).unwrap()
                - resolve("2026-11-01", "01:30", "America/Toronto", "earlier", false).unwrap(),
            3600
        );
    }
    #[test]
    fn recurring_wall_clock_follows_offset() {
        let a = resolve("2026-03-07", "09:00", "America/Toronto", "earlier", true).unwrap();
        let b = resolve("2026-03-08", "09:00", "America/Toronto", "earlier", true).unwrap();
        assert_eq!(b - a, 23 * 3600);
    }
}
