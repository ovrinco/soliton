//! The machine's log on a recorded day of this machine (26 Oct 23:00 to 28 Oct 01:00, Toronto time).
//! The slice was recorded read only by `tests/fixtures/machine_log/record.py`.

use soliton::machine_log;

use chrono::NaiveDate;
use chrono_tz::Tz;
use machine_log::{
    Actor, Category, Event, Log, PowerKind, Range, Source, Trigger, collect, describe,
};
use serde_json::Value;
use std::path::PathBuf;

fn slice() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/machine_log/2026-10-27")
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn recorded(zone: Tz) -> Log {
    collect(
        Range::days(zone, day("2026-10-26"), day("2026-10-28")),
        &Source::Recorded(slice()),
    )
    .unwrap()
}

fn lines_of<'a>(log: &'a Log, date: &str) -> &'a [machine_log::Line] {
    &log.days
        .iter()
        .find(|d| d.date == date)
        .unwrap_or_else(|| panic!("no day {date}"))
        .lines
}

fn line<'a>(log: &'a Log, date: &str, words: &str) -> &'a machine_log::Line {
    lines_of(log, date)
        .iter()
        .find(|l| l.words == words)
        .unwrap_or_else(|| panic!("no line {words:?} on {date}"))
}

/// Raw job records of one unit on one local day, counted straight from the recorded file.
fn raw_starts(unit: &str, date: &str, zone: Tz) -> usize {
    std::fs::read_to_string(slice().join("jobs.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| {
            v["UNIT"].as_str().or(v["USER_UNIT"].as_str()) == Some(unit) && v["JOB_TYPE"] == "start"
        })
        .filter(|v| {
            let micros: i64 = v["__REALTIME_TIMESTAMP"].as_str().unwrap().parse().unwrap();
            let time = chrono::DateTime::from_timestamp_micros(micros)
                .unwrap()
                .with_timezone(&zone);
            time.format("%Y-%m-%d").to_string() == date
        })
        .count()
}

#[test]
fn the_words_table_covers_every_category_and_the_unknown_case() {
    let job = |unit: &str, trigger, description: Option<&str>| Event::Job {
        unit: unit.into(),
        failed: false,
        trigger,
        description: description.map(str::to_owned),
    };
    let cases = [
        (
            Event::Packages {
                installed: 0,
                upgraded: 12,
                removed: 0,
                finished: true,
            },
            "Software packages were updated",
            Category::Updates,
        ),
        (
            Event::Power(PowerKind::Sleep),
            "Computer went to sleep",
            Category::Power,
        ),
        (
            job("snapper-cleanup.service", Trigger::Schedule, None),
            "Old system snapshots cleaned up",
            Category::Scheduled,
        ),
        (
            job("systemd-hostnamed.service", Trigger::Activation, None),
            "Computer name looked up",
            Category::Started,
        ),
        (
            Event::Power(PowerKind::BootEnd),
            "Computer stopped or restarted",
            Category::Stopped,
        ),
        (
            Event::Power(PowerKind::SignIn),
            "Signed in to the computer",
            Category::SignedIn,
        ),
        (
            Event::Launch {
                app: "nemo".into(),
                launcher: true,
            },
            "Opened the file manager",
            Category::Started,
        ),
        (
            job(
                "mystery-thing.service",
                Trigger::None,
                Some("Frobnicate the widgets daily"),
            ),
            "Reason unknown: Frobnicate the widgets daily",
            Category::Started,
        ),
        (
            job(
                "run-u77.service",
                Trigger::None,
                Some("/usr/bin/sh -c secret"),
            ),
            "Reason unknown",
            Category::Started,
        ),
        (
            job("run-u77.service", Trigger::None, None),
            "Reason unknown",
            Category::Started,
        ),
        (
            job(
                "odd.service",
                Trigger::Schedule,
                Some("Nightly mirror refresh"),
            ),
            "Nightly mirror refresh",
            Category::Scheduled,
        ),
    ];
    for (event, words, category) in &cases {
        let phrase = describe(event);
        assert_eq!(
            (phrase.words.as_str(), phrase.category),
            (*words, *category),
            "{event:?}"
        );
    }
    let covered: Vec<Category> = cases.iter().map(|(_, _, c)| *c).collect();
    for category in Category::ALL {
        assert!(
            covered.contains(&category),
            "{} has no case",
            category.label()
        );
    }
    let failed = describe(&Event::Job {
        unit: "man-db.service".into(),
        failed: true,
        trigger: Trigger::Schedule,
        description: None,
    });
    assert_eq!(
        (failed.words.as_str(), failed.category),
        ("Failed: Manual pages index refreshed", Category::Stopped)
    );
    assert_eq!(
        describe(&Event::Launch {
            app: "nemo".into(),
            launcher: true
        })
        .actor,
        Actor::His
    );
    assert_eq!(
        describe(&Event::Launch {
            app: "chromium".into(),
            launcher: false
        })
        .actor,
        Actor::Unknown
    );
    println!(
        "table cases: {} of {} categories covered, unknown cases 3",
        Category::ALL.len(),
        Category::ALL.len()
    );
}

#[test]
fn every_line_stays_inside_its_local_day_and_the_day_edge_follows_the_zone() {
    let toronto = recorded(chrono_tz::America::Toronto);
    let dates: Vec<&str> = toronto.days.iter().map(|d| d.date.as_str()).collect();
    assert_eq!(dates, ["2026-10-26", "2026-10-27", "2026-10-28"]);
    let mut checked = 0;
    for d in &toronto.days {
        for l in &d.lines {
            for t in l.times.iter().chain([&l.first, &l.last]) {
                assert!(
                    t.starts_with(&d.date) && t.ends_with("-04:00"),
                    "{t} outside {}",
                    d.date
                );
                checked += 1;
            }
        }
    }
    // The same records grouped by UTC days: the last hour of 26 Oct in Toronto is already 27 Oct.
    let utc = recorded(chrono_tz::UTC);
    assert!(utc.days.iter().all(|d| d.date != "2026-10-26"));
    let total = |log: &Log| {
        log.days
            .iter()
            .flat_map(|d| &d.lines)
            .map(|l| l.count)
            .sum::<usize>()
    };
    assert_eq!(total(&toronto), total(&utc));
    println!(
        "times checked inside their day: {checked} of {checked}; events {} in both zones",
        total(&toronto)
    );
}

#[test]
fn repeated_runs_fold_into_one_line_with_every_time() {
    let zone = chrono_tz::America::Toronto;
    let log = recorded(zone);
    let snapper = line(&log, "2026-10-27", "Old system snapshots cleaned up");
    let raw = raw_starts("snapper-cleanup.service", "2026-10-27", zone);
    assert_eq!((snapper.count, snapper.times.len()), (raw, raw));
    let guard = line(&log, "2026-10-27", "Machine load guard checked");
    let raw = raw_starts("swarm-load-guard.service", "2026-10-27", zone);
    assert_eq!((guard.count, guard.times.len()), (raw, raw));
    let mut checked = 0;
    for d in &log.days {
        let mut seen = std::collections::BTreeSet::new();
        for l in &d.lines {
            assert_eq!(l.count, l.times.len(), "{}", l.words);
            assert_eq!(
                (l.times.first(), l.times.last()),
                (Some(&l.first), Some(&l.last))
            );
            assert!(
                seen.insert((l.words.clone(), l.actor)),
                "{} appears twice on {}",
                l.words,
                d.date
            );
            checked += 1;
        }
    }
    println!(
        "snapshot cleanups {} = raw {}; load guard {} = raw {}; lines checked {checked} of {checked}",
        snapper.count,
        snapper.times.len(),
        guard.count,
        guard.times.len()
    );
}

#[test]
fn his_lines_are_launcher_launches_and_match_the_launcher_requests() {
    let zone = chrono_tz::America::Toronto;
    let log = recorded(zone);
    let his = log.only(Actor::His);
    let mut checked = 0;
    for l in his.days.iter().flat_map(|d| &d.lines) {
        assert!(
            l.evidence
                .iter()
                .any(|e| e == "rule: desktop launcher scope"),
            "{}",
            l.words
        );
        assert!(l.words.starts_with("Opened "), "{}", l.words);
        checked += 1;
    }
    assert!(checked > 0);
    for l in log
        .only(Actor::Machine)
        .days
        .iter()
        .chain(log.only(Actor::Unknown).days.iter())
        .flat_map(|d| &d.lines)
    {
        assert!(
            !l.evidence
                .iter()
                .any(|e| e == "rule: desktop launcher scope"),
            "{}",
            l.words
        );
    }
    // The launcher's own request log is an independent record of what he opened.
    let requests = std::fs::read_to_string(slice().join("launcher-requests.tsv")).unwrap();
    let asked = |program: &str| {
        requests
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter(|(micros, p)| {
                let t = chrono::DateTime::from_timestamp_micros(micros.parse().unwrap())
                    .unwrap()
                    .with_timezone(&zone);
                *p == program && t.format("%Y-%m-%d").to_string() == "2026-10-27"
            })
            .count()
    };
    let nemo = line(&his, "2026-10-27", "Opened the file manager");
    assert_eq!(nemo.count, asked("nemo"));
    let browser = line(&his, "2026-10-27", "Opened the web browser");
    assert_eq!(browser.count, asked("chromium"));
    // The browser's helper service a moment before its launch is the same launch, not a second line.
    assert!(
        lines_of(&log, "2026-10-27")
            .iter()
            .all(|l| l.words != "Web browser started")
    );
    assert!(
        log.only(Actor::Machine)
            .days
            .iter()
            .flat_map(|d| &d.lines)
            .any(|l| l.category == Category::Scheduled)
    );
    println!(
        "his lines {checked} of {checked} launcher-scoped; file manager {} = requests {}; browser {} = requests {}",
        nemo.count,
        asked("nemo"),
        browser.count,
        asked("chromium")
    );
}

fn jargon(text: &str) -> Option<&'static str> {
    if text.contains(".service") {
        return Some(".service");
    }
    if text.contains(".timer") {
        return Some(".timer");
    }
    if text.contains('/') {
        return Some("/");
    }
    let mut run = 0;
    for b in text.bytes() {
        run = if b.is_ascii_digit() || (b'a'..=b'f').contains(&b) {
            run + 1
        } else {
            0
        };
        if run >= 8 {
            return Some("hex id");
        }
    }
    None
}

#[test]
fn every_line_reads_as_a_few_plain_words() {
    let log = recorded(chrono_tz::America::Toronto);
    let mut checked = 0;
    for l in log.days.iter().flat_map(|d| &d.lines) {
        assert!(l.words.split_whitespace().count() < 8, "{}", l.words);
        assert_eq!(jargon(&l.words), None, "{}", l.words);
        assert_eq!(jargon(&l.text()), None, "{}", l.text());
        checked += 1;
    }
    let power = line(&log, "2026-10-27", "Computer went to sleep");
    assert_eq!((power.count, power.category), (2, Category::Power));
    assert_eq!(
        line(&log, "2026-10-27", "Computer woke from sleep").count,
        2
    );
    assert_eq!(
        line(&log, "2026-10-27", "New software was installed").category,
        Category::Updates
    );
    assert_eq!(
        line(&log, "2026-10-27", "Opened the file manager").text(),
        format!(
            "{} file manager openings",
            line(&log, "2026-10-27", "Opened the file manager").count
        )
    );
    println!("lines read as plain words: {checked} of {checked}");
}

#[test]
fn the_folded_day_reads_in_plain_lines() {
    let log = recorded(chrono_tz::America::Toronto);
    for d in &log.days {
        println!("== {}", d.date);
        for l in &d.lines {
            println!(
                "{} | {} | {:?} | {}..{}",
                l.text(),
                l.category.label(),
                l.actor,
                l.first,
                l.last
            );
        }
    }
}

#[test]
fn a_missing_or_refusing_journal_is_an_error_with_its_reason() {
    let range = Range::days(
        chrono_tz::America::Toronto,
        day("2026-10-27"),
        day("2026-10-27"),
    );
    let missing = Source::System {
        journalctl: "/nonexistent/journalctl".into(),
        systemctl: "systemctl".into(),
        package_log: "/var/log/pacman.log".into(),
    };
    let error = collect(range, &missing).unwrap_err().to_string();
    assert!(error.contains("journalctl could not be started"), "{error}");
    let refusing = Source::System {
        journalctl: "false".into(),
        systemctl: "systemctl".into(),
        package_log: "/var/log/pacman.log".into(),
    };
    let error = collect(range, &refusing).unwrap_err().to_string();
    assert!(error.contains("false refused"), "{error}");
    let empty = Source::Recorded(PathBuf::from("/nonexistent/recording"));
    let error = collect(range, &empty).unwrap_err().to_string();
    assert!(error.contains("job records"), "{error}");
    println!("errors with reasons: 3 of 3");
}

fn threads_and_children() -> (usize, usize) {
    let mut threads = 0;
    let mut children = 0;
    for task in std::fs::read_dir("/proc/self/task").unwrap() {
        threads += 1;
        let path = task.unwrap().path().join("children");
        children += std::fs::read_to_string(path)
            .unwrap_or_default()
            .split_whitespace()
            .count();
    }
    (threads, children)
}

/// Run alone in its own process by the next test, so no other test's threads move the counts.
#[test]
#[ignore]
fn idle_probe() {
    let before = threads_and_children();
    let now = chrono::Utc::now();
    let range = Range {
        from: now - chrono::Duration::hours(1),
        to: now,
        zone: chrono_tz::America::Toronto,
    };
    let log = collect(range, &Source::system()).unwrap();
    let after = threads_and_children();
    assert!(log.sources.iter().any(|s| s.entries > 0));
    println!(
        "IDLE before threads={} children={} after threads={} children={}",
        before.0, before.1, after.0, after.1
    );
    assert_eq!(before, after);
}

#[test]
fn nothing_outlives_collect() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "idle_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find_map(|l| l.find("IDLE").map(|at| &l[at..]))
        .unwrap_or_else(|| panic!("no probe line: {text}"));
    println!("{line}");
    assert!(output.status.success(), "{text}");
}
