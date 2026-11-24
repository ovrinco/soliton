//! The machine's log: what the computer did, one short plain line per kind of event, grouped by day.
//!
//! `collect(range, source)` reads, on request only, the records the machine already keeps: the system and
//! user service managers' job records (with the timer, socket, path or desktop-launcher cause of each), power
//! and boot records, sign-ins, and the package manager's log. Each event becomes about five plain words and a
//! category from a fixed table (never a model); identical lines of one day fold into one line with a count,
//! keeping every time. Each line says whether it was his own act (started through the desktop launcher), the
//! machine's (a timer, socket or path started it), or unknown (nothing on record says who asked).
//!
//! Nothing here runs unless `collect` is called: no thread, child or file handle outlives the call. Every
//! source is bounded in bytes and in time.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Bytes kept from one source (a child's output or one file).
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
/// Time allowed to one child process.
pub const MAX_CHILD_TIME: Duration = Duration::from_secs(2);
const MAX_UNITS_ASKED: usize = 256;
const MAX_TIMES: usize = 5_000;
const MAX_EVIDENCE: usize = 8;
const LAUNCH_FOLD_SECS: i64 = 5;
const MAX_DESCRIPTION_WORDS: usize = 5;
const PACKAGE_LOG: &str = "/var/log/pacman.log";

/// systemd catalog: a unit job finished ("Started …", "Finished …", "Failed to start …").
const JOB_DONE_ID: &str = "39f53479d3a045ac8e11786248231fbf";
/// systemd catalog ids for power and sign-in records, and what each one is.
const POWER_IDS: [(&str, PowerKind); 6] = [
    ("98268866d1d54a499c4e98921d93bc40", PowerKind::Shutdown),
    ("6bbd95ee977941e497c48be27c254128", PowerKind::Sleep),
    ("8811e6df2a8e40f58a94cea26f8ebf14", PowerKind::Wake),
    ("b72ea4a2881545a0b50e200e55b9b071", PowerKind::PowerKey),
    ("8d45620c1a4348dbb17410da57c60c66", PowerKind::SignIn),
    ("3354939424b4456d9802ca8333ed424a", PowerKind::SignOut),
];
const FIELDS: &str = "MESSAGE_ID,UNIT,USER_UNIT,JOB_TYPE,JOB_RESULT,SLEEP,SESSION_ID";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The {what} could not be read: {reason}")]
    Source { what: &'static str, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// The half-open interval `[from, to)` to read, and the zone whose calendar days group it.
#[derive(Clone, Copy, Debug)]
pub struct Range {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub zone: Tz,
}

impl Range {
    /// Whole local days from `first` to `last`, both included.
    pub fn days(zone: Tz, first: NaiveDate, last: NaiveDate) -> Range {
        let start = |day: NaiveDate| {
            let midnight = day
                .and_hms_opt(0, 0, 0)
                .expect("midnight exists as a clock time");
            zone.from_local_datetime(&midnight)
                .earliest()
                .unwrap_or_else(|| zone.from_utc_datetime(&midnight))
                .with_timezone(&Utc)
        };
        let after = last.succ_opt().unwrap_or(last);
        Range {
            from: start(first),
            to: start(after),
            zone,
        }
    }
}

/// Where the records come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// This machine's own records, through these programs and this package log.
    System {
        journalctl: PathBuf,
        systemctl: PathBuf,
        package_log: PathBuf,
    },
    /// Saved records in a folder: `jobs.jsonl`, `power.jsonl` (journal JSON lines), `boots.json`,
    /// `units-system.txt`, `units-user.txt` (`systemctl show` output) and `pacman.log`.
    Recorded(PathBuf),
}

impl Source {
    pub fn system() -> Source {
        Source::System {
            journalctl: PathBuf::from("journalctl"),
            systemctl: PathBuf::from("systemctl"),
            package_log: PathBuf::from(PACKAGE_LOG),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Category {
    Updates,
    Power,
    Scheduled,
    Started,
    Stopped,
    #[serde(rename = "Signed in")]
    SignedIn,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Updates,
        Category::Power,
        Category::Scheduled,
        Category::Started,
        Category::Stopped,
        Category::SignedIn,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Updates => "Updates",
            Category::Power => "Power",
            Category::Scheduled => "Scheduled",
            Category::Started => "Started",
            Category::Stopped => "Stopped",
            Category::SignedIn => "Signed in",
        }
    }
}

/// Who asked for it. `His` rests on the desktop launcher's record alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    His,
    Machine,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct Log {
    pub zone: String,
    pub from: String,
    pub to: String,
    pub days: Vec<Day>,
    pub sources: Vec<SourceNote>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Day {
    /// The local calendar day, `YYYY-MM-DD`.
    pub date: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Line {
    /// About five plain words; with `count` and `many`, `text()` reads the folded line.
    pub words: String,
    pub category: Category,
    pub actor: Actor,
    pub count: usize,
    pub first: String,
    pub last: String,
    pub times: Vec<String>,
    /// What the line rests on: units, managers, triggers, rules. Not for the plain reading.
    pub evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub many: Option<String>,
}

impl Line {
    /// The line as read: `Opened the file manager`, or `15 file manager openings` when it repeats.
    pub fn text(&self) -> String {
        match (&self.many, self.count) {
            (_, 1) => self.words.clone(),
            (Some(many), count) => format!("{count} {many}"),
            (None, count) => format!("{} ({count} times)", self.words),
        }
    }
}

/// What one source gave and what it could not see.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SourceNote {
    pub name: &'static str,
    pub entries: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
}

impl Log {
    /// The same log with only the lines of one actor; empty days are left out.
    pub fn only(&self, actor: Actor) -> Log {
        let days = self
            .days
            .iter()
            .map(|day| Day {
                date: day.date.clone(),
                lines: day
                    .lines
                    .iter()
                    .filter(|l| l.actor == actor)
                    .cloned()
                    .collect(),
            })
            .filter(|day| !day.lines.is_empty())
            .collect();
        Log {
            days,
            ..self.clone()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// The words table.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerKind {
    Boot,
    BootEnd,
    Shutdown,
    Sleep,
    Wake,
    PowerKey,
    SignIn,
    SignOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Schedule,
    Activation,
    None,
}

/// One event before it is put into words.
#[derive(Clone, Debug)]
pub enum Event {
    /// An application scope; `launcher` when the session launcher made it, otherwise the app made its own.
    Launch {
        app: String,
        launcher: bool,
    },
    /// A service manager job that started a unit.
    Job {
        unit: String,
        failed: bool,
        trigger: Trigger,
        description: Option<String>,
    },
    Power(PowerKind),
    Packages {
        installed: usize,
        upgraded: usize,
        removed: usize,
        finished: bool,
    },
}

/// The plain reading of one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrase {
    pub words: String,
    pub category: Category,
    pub actor: Actor,
    pub many: Option<String>,
}

fn phrase(
    words: impl Into<String>,
    category: Category,
    actor: Actor,
    many: Option<&str>,
) -> Phrase {
    Phrase {
        words: words.into(),
        category,
        actor,
        many: many.map(str::to_owned),
    }
}

/// Applications he opens, by the launcher's program word: the words, and the words for a count.
const APPS: &[(&[&str], &str, &str)] = &[
    (
        &["nemo", "nautilus", "thunar"],
        "Opened the file manager",
        "file manager openings",
    ),
    (
        &[
            "chromium",
            "google-chrome",
            "google-chrome-stable",
            "firefox",
            "brave",
            "brave-browser",
            "zen",
            "zen-browser",
            "omarchy-launch-browser",
        ],
        "Opened the web browser",
        "web browser openings",
    ),
    (
        &[
            "zeditor",
            "zed",
            "dev.zed.zed",
            "code",
            "codium",
            "nvim",
            "gnome-text-editor",
        ],
        "Opened the code editor",
        "code editor openings",
    ),
    (&["voxtype"], "Opened VoxType", "VoxType openings"),
    (
        &["xdg-terminal-exec", "alacritty", "ghostty", "kitty", "foot"],
        "Opened a terminal",
        "terminal openings",
    ),
    (
        &["gtk-launch"],
        "Opened an application",
        "application openings",
    ),
];

/// Units by their stem (name without type, instance or trailing run number): words, category, actor, count words.
const UNITS: &[(&str, &str, Category, Actor, &str)] = &[
    (
        "snapper-cleanup",
        "Old system snapshots cleaned up",
        Category::Scheduled,
        Actor::Machine,
        "snapshot cleanups",
    ),
    (
        "snapper-timeline",
        "System snapshot taken",
        Category::Scheduled,
        Actor::Machine,
        "system snapshots taken",
    ),
    (
        "snapper-boot",
        "Start-up snapshot taken",
        Category::Started,
        Actor::Machine,
        "start-up snapshots",
    ),
    (
        "snapperd",
        "Snapshot service answered a request",
        Category::Started,
        Actor::Machine,
        "snapshot service requests",
    ),
    (
        "man-db",
        "Manual pages index refreshed",
        Category::Scheduled,
        Actor::Machine,
        "manual index refreshes",
    ),
    (
        "plocate-updatedb",
        "File search index refreshed",
        Category::Scheduled,
        Actor::Machine,
        "file search index refreshes",
    ),
    (
        "updatedb",
        "File search index refreshed",
        Category::Scheduled,
        Actor::Machine,
        "file search index refreshes",
    ),
    (
        "shadow",
        "Password files checked",
        Category::Scheduled,
        Actor::Machine,
        "password file checks",
    ),
    (
        "systemd-tmpfiles-clean",
        "Temporary files cleaned up",
        Category::Scheduled,
        Actor::Machine,
        "temporary file cleanups",
    ),
    (
        "logrotate",
        "Old logs tidied away",
        Category::Scheduled,
        Actor::Machine,
        "log tidy-ups",
    ),
    (
        "fstrim",
        "Disk free space tidied",
        Category::Scheduled,
        Actor::Machine,
        "disk tidy-ups",
    ),
    (
        "archlinux-keyring-wkd-sync",
        "Software signing keys refreshed",
        Category::Scheduled,
        Actor::Machine,
        "signing key refreshes",
    ),
    (
        "pacman-filesdb-refresh",
        "Software file list refreshed",
        Category::Scheduled,
        Actor::Machine,
        "software list refreshes",
    ),
    (
        "paccache",
        "Old software downloads removed",
        Category::Scheduled,
        Actor::Machine,
        "download cleanups",
    ),
    (
        "NetworkManager-dispatcher",
        "Network change handled",
        Category::Started,
        Actor::Machine,
        "network changes handled",
    ),
    (
        "systemd-hostnamed",
        "Computer name looked up",
        Category::Started,
        Actor::Machine,
        "computer name lookups",
    ),
    (
        "systemd-rfkill",
        "Wireless switch state saved",
        Category::Started,
        Actor::Machine,
        "wireless switch saves",
    ),
    (
        "systemd-timedated",
        "Clock settings looked up",
        Category::Started,
        Actor::Machine,
        "clock lookups",
    ),
    (
        "systemd-localed",
        "Language settings looked up",
        Category::Started,
        Actor::Machine,
        "language lookups",
    ),
    (
        "fwupd",
        "Firmware update check ran",
        Category::Started,
        Actor::Machine,
        "firmware checks",
    ),
    (
        "polkit-agent-helper",
        "Administrator permission was requested",
        Category::Started,
        Actor::Unknown,
        "permission requests",
    ),
    (
        "user",
        "Personal services started",
        Category::SignedIn,
        Actor::Unknown,
        "personal service starts",
    ),
    (
        "omarchy-sleep-lock",
        "Screen locked before sleep",
        Category::Power,
        Actor::Machine,
        "locks before sleep",
    ),
    (
        "omarchy-browser",
        "Web browser started",
        Category::Started,
        Actor::Unknown,
        "web browser starts",
    ),
    (
        "swarm-load-guard",
        "Machine load guard checked",
        Category::Scheduled,
        Actor::Machine,
        "load guard checks",
    ),
    (
        "voice-note-ledger",
        "New voice notes filed",
        Category::Scheduled,
        Actor::Machine,
        "voice note filings",
    ),
    (
        "tracker-backup",
        "Tracker copy saved",
        Category::Scheduled,
        Actor::Machine,
        "tracker copies",
    ),
    (
        "lorengine-checkpoint",
        "Lorengine checkpoint saved",
        Category::Started,
        Actor::Unknown,
        "Lorengine checkpoints",
    ),
    (
        "ovrin-rgb",
        "Lighting service started",
        Category::Started,
        Actor::Unknown,
        "lighting service starts",
    ),
];

/// Unit types that are states, not events, and units whose event the power records already give.
fn skipped(unit: &str) -> bool {
    const TYPES: [&str; 9] = [
        ".target",
        ".mount",
        ".automount",
        ".device",
        ".slice",
        ".socket",
        ".timer",
        ".path",
        ".swap",
    ];
    const COVERED: [&str; 6] = [
        "systemd-suspend.service",
        "systemd-hibernate.service",
        "systemd-hybrid-sleep.service",
        "systemd-suspend-then-hibernate.service",
        "user-runtime-dir",
        "session-",
    ];
    TYPES.iter().any(|t| unit.ends_with(t)) || COVERED.iter().any(|c| unit.starts_with(c))
}

/// `foo@bar.service` → `foo`; `lorengine-checkpoint-1793….service` → `lorengine-checkpoint`.
fn unit_stem(unit: &str) -> &str {
    let name = unit.rsplit_once('.').map_or(unit, |(name, _)| name);
    let name = name.split_once('@').map_or(name, |(name, _)| name);
    match name.rsplit_once('-') {
        Some((stem, tail))
            if !stem.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) =>
        {
            stem
        }
        _ => name,
    }
}

fn is_hex_run(word: &str) -> bool {
    word.len() >= 8 && word.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A unit's own description, kept only when it reads as plain words: its first clause, at most five words.
pub fn plain_description(description: &str, unit: &str) -> Option<String> {
    let text = description.trim();
    let clause = text
        .split(": ")
        .next()
        .unwrap_or(text)
        .split(" (")
        .next()
        .unwrap_or(text)
        .trim()
        .trim_end_matches('.');
    let lowered = clause.to_ascii_lowercase();
    let jargon = [
        ".service", ".timer", ".scope", ".socket", ".path", ".slice", ".target", ".mount",
    ];
    if clause.is_empty()
        || clause == unit
        || clause.contains(['/', '\\', '=', '@', '[', ']', '{', '}'])
        || jargon.iter().any(|j| lowered.contains(j))
        || clause
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(is_hex_run)
        || clause.split_whitespace().any(|w| w.len() > 24)
    {
        return None;
    }
    let words: Vec<&str> = clause
        .split_whitespace()
        .take(MAX_DESCRIPTION_WORDS)
        .collect();
    let mut out = words.join(" ");
    if let Some(first) = out.get(..1) {
        out = first.to_ascii_uppercase() + &out[1..];
    }
    Some(out)
}

fn app_words(app: &str) -> (String, String) {
    let lower = app.to_ascii_lowercase();
    let lower = lower.trim_end_matches(".desktop");
    if let Some((_, words, many)) = APPS.iter().find(|(names, _, _)| names.contains(&lower)) {
        return ((*words).to_owned(), (*many).to_owned());
    }
    let name = lower
        .rsplit('.')
        .next()
        .unwrap_or(lower)
        .replace(['-', '_'], " ");
    let plain = !name.is_empty()
        && name.split_whitespace().count() <= 3
        && name.bytes().all(|b| b.is_ascii_lowercase() || b == b' ')
        && name.split_whitespace().all(|w| !is_hex_run(w));
    if plain {
        let mut title = name.clone();
        title[..1].make_ascii_uppercase();
        (format!("Opened {title}"), format!("{name} openings"))
    } else {
        (
            "Opened an application".to_owned(),
            "application openings".to_owned(),
        )
    }
}

/// The words table: an event to about five plain words, a category and who asked.
pub fn describe(event: &Event) -> Phrase {
    match event {
        Event::Launch {
            app,
            launcher: true,
        } => {
            let (words, many) = app_words(app);
            phrase(words, Category::Started, Actor::His, Some(&many))
        }
        Event::Launch {
            app,
            launcher: false,
        } => {
            let (words, _) = app_words(app);
            let subject = words.trim_start_matches("Opened ");
            let subject = subject
                .strip_prefix("the ")
                .or_else(|| subject.strip_prefix("a "))
                .or_else(|| subject.strip_prefix("an "))
                .unwrap_or(subject);
            phrase(
                format!("{subject} started on its own"),
                Category::Started,
                Actor::Unknown,
                None,
            )
        }
        Event::Job {
            unit,
            failed,
            trigger,
            description,
        } => {
            let stem = unit_stem(unit);
            let known = UNITS.iter().find(|(name, ..)| *name == stem);
            let plain = description
                .as_deref()
                .and_then(|d| plain_description(d, unit));
            if *failed {
                let subject = known
                    .map(|(_, words, ..)| (*words).to_owned())
                    .or(plain)
                    .unwrap_or_else(|| "A background task".to_owned());
                return phrase(
                    format!("Failed: {subject}"),
                    Category::Stopped,
                    Actor::Unknown,
                    Some("failures"),
                );
            }
            let by_trigger = match trigger {
                Trigger::Schedule => Some((Category::Scheduled, Actor::Machine)),
                Trigger::Activation => Some((Category::Started, Actor::Machine)),
                Trigger::None => None,
            };
            if let Some((_, words, category, actor, many)) = known {
                let (category, actor) = match by_trigger {
                    // A unit the table knows keeps its words; a recorded trigger is the stronger evidence of who.
                    Some((cat, act)) if *category != Category::Power => (cat, act),
                    _ => (*category, *actor),
                };
                return phrase(*words, category, actor, Some(many));
            }
            match (by_trigger, plain) {
                (Some((Category::Scheduled, actor)), Some(plain)) => {
                    phrase(plain, Category::Scheduled, actor, None)
                }
                (Some((Category::Scheduled, actor)), None) => phrase(
                    "A scheduled task ran",
                    Category::Scheduled,
                    actor,
                    Some("scheduled tasks"),
                ),
                (Some((category, actor)), Some(plain)) => {
                    phrase(format!("Started on demand: {plain}"), category, actor, None)
                }
                (Some((category, actor)), None) => phrase(
                    "A service started on demand",
                    category,
                    actor,
                    Some("services started on demand"),
                ),
                (None, Some(plain)) => phrase(
                    format!("Reason unknown: {plain}"),
                    Category::Started,
                    Actor::Unknown,
                    None,
                ),
                (None, None) => phrase("Reason unknown", Category::Started, Actor::Unknown, None),
            }
        }
        Event::Power(kind) => match kind {
            PowerKind::Boot => phrase(
                "Computer started up",
                Category::Power,
                Actor::Unknown,
                Some("start-ups"),
            ),
            PowerKind::BootEnd => phrase(
                "Computer stopped or restarted",
                Category::Stopped,
                Actor::Unknown,
                Some("stops or restarts"),
            ),
            PowerKind::Shutdown => phrase(
                "Computer began shutting down",
                Category::Power,
                Actor::Unknown,
                Some("shutdowns"),
            ),
            PowerKind::Sleep => phrase(
                "Computer went to sleep",
                Category::Power,
                Actor::Unknown,
                Some("sleeps"),
            ),
            PowerKind::Wake => phrase(
                "Computer woke from sleep",
                Category::Power,
                Actor::Unknown,
                Some("wake-ups"),
            ),
            PowerKind::PowerKey => phrase(
                "Power button was pressed",
                Category::Power,
                Actor::Unknown,
                Some("power button presses"),
            ),
            PowerKind::SignIn => phrase(
                "Signed in to the computer",
                Category::SignedIn,
                Actor::Unknown,
                Some("sign-ins"),
            ),
            PowerKind::SignOut => phrase(
                "Signed out of the computer",
                Category::SignedIn,
                Actor::Unknown,
                Some("sign-outs"),
            ),
        },
        Event::Packages {
            installed,
            upgraded,
            removed,
            finished,
        } => {
            if !finished {
                phrase(
                    "Software update did not finish",
                    Category::Updates,
                    Actor::Unknown,
                    Some("unfinished updates"),
                )
            } else if *upgraded > 0 {
                phrase(
                    "Software packages were updated",
                    Category::Updates,
                    Actor::Unknown,
                    Some("software updates"),
                )
            } else if *installed > 0 {
                phrase(
                    "New software was installed",
                    Category::Updates,
                    Actor::Unknown,
                    Some("software installs"),
                )
            } else if *removed > 0 {
                phrase(
                    "Software was removed",
                    Category::Updates,
                    Actor::Unknown,
                    Some("software removals"),
                )
            } else {
                phrase(
                    "Software packages were changed",
                    Category::Updates,
                    Actor::Unknown,
                    Some("software changes"),
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Reading the sources.

/// One event at its time, with what it rests on.
struct Entry {
    time: DateTime<Utc>,
    phrase: Phrase,
    evidence: Vec<String>,
}

struct Job {
    time: DateTime<Utc>,
    manager: &'static str,
    unit: String,
    failed: bool,
}

#[derive(Default, Clone)]
struct UnitFacts {
    triggered_by: Vec<String>,
    description: Option<String>,
}

type Units = BTreeMap<(&'static str, String), UnitFacts>;

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn journal_time(value: &Value) -> Option<DateTime<Utc>> {
    let micros: i64 = field(value, "__REALTIME_TIMESTAMP")?.parse().ok()?;
    Utc.timestamp_opt(
        micros.div_euclid(1_000_000),
        (micros.rem_euclid(1_000_000) * 1_000) as u32,
    )
    .single()
}

fn short(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Undo systemd unit-name escaping (`\x2d` and friends).
fn unescape_unit(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'x'))
            .then(|| raw.get(index + 2..index + 4))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escaped {
            out.push(byte);
            index += 4;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How an application scope was made, from its name alone: `app-<Desktop>-<app>-<8 hex>.scope` by the
/// session launcher (`true`), `app-<reverse.dns.App>-<pid>.scope` by the application itself (`false`).
fn launch_app(unit: &str) -> Option<(String, bool)> {
    let inner = unit.strip_prefix("app-")?.strip_suffix(".scope")?;
    let (rest, tail) = inner.rsplit_once('-')?;
    if tail.len() == 8
        && tail
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        let (desktop, app) = rest.split_once('-')?;
        if !desktop.is_empty() && !desktop.contains('.') && !app.is_empty() {
            return Some((short(&unescape_unit(app), 64), true));
        }
    }
    if !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
        && !rest.contains('-')
        && rest.contains('.')
    {
        let app = unescape_unit(rest.rsplit('.').next().unwrap_or(rest)).to_ascii_lowercase();
        return Some((short(&app, 64), false));
    }
    None
}

fn job_entry(value: &Value) -> Option<Job> {
    if field(value, "MESSAGE_ID") != Some(JOB_DONE_ID) {
        return None;
    }
    if !matches!(field(value, "JOB_TYPE"), Some("start" | "restart")) {
        return None;
    }
    let (manager, unit) = match (field(value, "USER_UNIT"), field(value, "UNIT")) {
        (Some(unit), _) => ("user", unit),
        (None, Some(unit)) => ("system", unit),
        _ => return None,
    };
    Some(Job {
        time: journal_time(value)?,
        manager,
        unit: short(unit, 256),
        failed: field(value, "JOB_RESULT").is_some_and(|r| r != "done"),
    })
}

fn power_kind(value: &Value) -> Option<PowerKind> {
    let id = field(value, "MESSAGE_ID")?;
    POWER_IDS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, kind)| *kind)
}

fn journal_values(bytes: &[u8], note: &mut SourceNote) -> Vec<Value> {
    let mut values = Vec::new();
    let mut malformed = 0usize;
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        match serde_json::from_slice::<Value>(line) {
            Ok(value) => values.push(value),
            Err(_) => malformed += 1,
        }
    }
    note.entries += values.len();
    // The last line of a cut output is expected to be broken; more than that is a gap worth naming.
    if malformed > usize::from(note.truncated) {
        note.gap = Some(format!("{malformed} unreadable records"));
    }
    values
}

fn parse_unit_show(output: &str, manager: &'static str, units: &mut Units) {
    for block in output.split("\n\n") {
        let mut id = None;
        let mut facts = UnitFacts::default();
        let mut loaded = true;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("Id=") {
                id = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("TriggeredBy=") {
                facts.triggered_by = value.split_whitespace().map(|s| short(s, 128)).collect();
            } else if let Some(value) = line.strip_prefix("Description=") {
                facts.description = Some(short(value, 256));
            } else if let Some(value) = line.strip_prefix("LoadState=") {
                loaded = value != "not-found";
            }
        }
        if !loaded {
            facts.description = None;
        }
        if let Some(id) = id {
            units.insert((manager, id), facts);
        }
    }
}

fn read_limited<R: Read>(reader: &mut R, limit: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    Ok((bytes, truncated))
}

fn read_file(path: &Path, limit: u64) -> std::io::Result<(Vec<u8>, bool)> {
    read_limited(&mut File::open(path)?, limit)
}

/// The newest `limit` bytes of a log that only grows, starting at a whole line.
fn read_tail(path: &Path, limit: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if length <= limit {
        return read_limited(&mut file, limit);
    }
    file.seek(SeekFrom::Start(length - limit))?;
    let (mut bytes, _) = read_limited(&mut file, limit)?;
    let cut = bytes
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |i| i + 1);
    Ok((bytes.split_off(cut), true))
}

/// Run a read-only program, bounded in bytes and time. Output cut at the byte bound is kept and marked.
/// The child and both pipe readers end before this returns.
fn run_bounded(
    program: &Path,
    args: &[String],
    limit: u64,
    time: Duration,
) -> std::result::Result<(Vec<u8>, bool), String> {
    let name = program.file_name().map_or_else(
        || program.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{name} could not be started ({error})"))?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{name} gave no output channel"));
    };
    std::thread::scope(|scope| {
        let out = scope.spawn(move || read_limited(&mut stdout, limit));
        let err = scope.spawn(move || read_limited(&mut stderr, 4096));
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < time => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "{name} took longer than {} ms and was ended",
                        time.as_millis()
                    ));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("{name} could not be followed ({error})"));
                }
            }
        };
        let output = out
            .join()
            .map_err(|_| format!("{name} output reader stopped"))?;
        let message = err
            .join()
            .ok()
            .and_then(std::result::Result::ok)
            .map(|(bytes, _)| String::from_utf8_lossy(&bytes).trim().to_owned());
        let status = status?;
        let (bytes, truncated) =
            output.map_err(|error| format!("{name} output could not be read ({error})"))?;
        // Closing the pipe at the byte bound ends the writer early; that exit is expected.
        if !status.success() && !truncated {
            let said = message
                .filter(|m| !m.is_empty())
                .map(|m| format!(": {}", short(m.lines().next().unwrap_or(""), 200)));
            return Err(format!(
                "{name} refused ({status}){}",
                said.unwrap_or_default()
            ));
        }
        Ok((bytes, truncated))
    })
}

/// The raw records, from whichever source.
#[derive(Default)]
struct Raw {
    jobs: Vec<Value>,
    power: Vec<Value>,
    boots: Option<Value>,
    units: Units,
    package_log: Vec<u8>,
    notes: Vec<SourceNote>,
}

fn journal_args(range: &Range, matches: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--no-pager".into(),
        "--output=json".into(),
        // Newest first, so a cut at the byte bound drops the oldest part of the range.
        "--reverse".into(),
        format!("--since=@{}", range.from.timestamp()),
        format!("--until=@{}", range.to.timestamp()),
        format!("--output-fields={FIELDS}"),
    ];
    args.extend(matches.iter().cloned());
    args
}

fn read_system(
    range: &Range,
    journalctl: &Path,
    systemctl: &Path,
    package_log: &Path,
) -> Result<Raw> {
    let mut raw = Raw::default();
    let journal = |what: &'static str, matches: &[String]| -> Result<(Vec<Value>, SourceNote)> {
        let (bytes, truncated) = run_bounded(
            journalctl,
            &journal_args(range, matches),
            MAX_SOURCE_BYTES,
            MAX_CHILD_TIME,
        )
        .map_err(|reason| Error::Source { what, reason })?;
        let mut note = SourceNote {
            name: what,
            truncated,
            ..SourceNote::default()
        };
        let values = journal_values(&bytes, &mut note);
        Ok((values, note))
    };
    let (jobs, note) = journal(
        "service managers' job records",
        &[format!("MESSAGE_ID={JOB_DONE_ID}")],
    )?;
    raw.jobs = jobs;
    raw.notes.push(note);
    let mut power_match = Vec::new();
    for (index, (id, _)) in POWER_IDS.iter().enumerate() {
        if index > 0 {
            power_match.push("+".to_owned());
        }
        power_match.push(format!("MESSAGE_ID={id}"));
    }
    let (power, note) = journal("power and sign-in records", &power_match)?;
    raw.power = power;
    raw.notes.push(note);
    let boots = ["--no-pager", "--list-boots", "--output=json"].map(str::to_owned);
    let (bytes, truncated) = run_bounded(journalctl, &boots, MAX_SOURCE_BYTES, MAX_CHILD_TIME)
        .map_err(|reason| Error::Source {
            what: "boot list",
            reason,
        })?;
    let mut note = SourceNote {
        name: "boot list",
        truncated,
        ..SourceNote::default()
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => {
            note.entries = value.as_array().map_or(0, Vec::len);
            raw.boots = Some(value);
        }
        Err(_) => note.gap = Some("the boot list was unreadable".into()),
    }
    raw.notes.push(note);

    // Ask each service manager what triggers the units seen, and how each describes itself.
    let jobs: Vec<Job> = raw.jobs.iter().filter_map(job_entry).collect();
    let mut note = SourceNote {
        name: "unit causes",
        ..SourceNote::default()
    };
    for manager in ["system", "user"] {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for job in jobs.iter().filter(|job| {
            job.manager == manager && !job.unit.ends_with(".scope") && !skipped(&job.unit)
        }) {
            *seen.entry(job.unit.as_str()).or_default() += 1;
        }
        // Most frequent first, so a cut at the bound leaves out one-off units.
        let mut units: Vec<(&str, usize)> = seen.into_iter().collect();
        units.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        if units.len() > MAX_UNITS_ASKED {
            note.truncated = true;
            units.truncate(MAX_UNITS_ASKED);
        }
        if units.is_empty() {
            continue;
        }
        let mut args: Vec<String> = Vec::new();
        if manager == "user" {
            args.push("--user".into());
        }
        args.extend(
            [
                "show",
                "-p",
                "Id",
                "-p",
                "TriggeredBy",
                "-p",
                "Description",
                "-p",
                "LoadState",
                "--",
            ]
            .map(str::to_owned),
        );
        args.extend(units.iter().map(|(unit, _)| (*unit).to_owned()));
        match run_bounded(systemctl, &args, MAX_SOURCE_BYTES, MAX_CHILD_TIME) {
            Ok((bytes, truncated)) => {
                note.truncated |= truncated;
                note.entries += units.len();
                parse_unit_show(&String::from_utf8_lossy(&bytes), manager, &mut raw.units);
            }
            Err(reason) => {
                note.gap = Some(format!(
                    "causes from the {manager} service manager are missing: {reason}"
                ))
            }
        }
    }
    raw.notes.push(note);

    let mut note = SourceNote {
        name: "package log",
        ..SourceNote::default()
    };
    match read_tail(package_log, MAX_SOURCE_BYTES) {
        Ok((bytes, truncated)) => {
            note.truncated = truncated;
            raw.package_log = bytes;
        }
        Err(error) => note.gap = Some(format!("the package log could not be read ({error})")),
    }
    raw.notes.push(note);
    Ok(raw)
}

fn read_recorded(dir: &Path) -> Result<Raw> {
    let mut raw = Raw::default();
    let journal = |what: &'static str, file: &str| -> Result<(Vec<Value>, SourceNote)> {
        let (bytes, truncated) =
            read_file(&dir.join(file), MAX_SOURCE_BYTES).map_err(|error| Error::Source {
                what,
                reason: format!("{file}: {error}"),
            })?;
        let mut note = SourceNote {
            name: what,
            truncated,
            ..SourceNote::default()
        };
        let values = journal_values(&bytes, &mut note);
        Ok((values, note))
    };
    let (jobs, note) = journal("service managers' job records", "jobs.jsonl")?;
    raw.jobs = jobs;
    raw.notes.push(note);
    let (power, note) = journal("power and sign-in records", "power.jsonl")?;
    raw.power = power;
    raw.notes.push(note);
    let mut note = SourceNote {
        name: "boot list",
        ..SourceNote::default()
    };
    match read_file(&dir.join("boots.json"), MAX_SOURCE_BYTES)
        .map(|(b, _)| serde_json::from_slice::<Value>(&b))
    {
        Ok(Ok(value)) => {
            note.entries = value.as_array().map_or(0, Vec::len);
            raw.boots = Some(value);
        }
        _ => note.gap = Some("the boot list is missing".into()),
    }
    raw.notes.push(note);
    let mut note = SourceNote {
        name: "unit causes",
        ..SourceNote::default()
    };
    for manager in ["system", "user"] {
        match read_file(&dir.join(format!("units-{manager}.txt")), MAX_SOURCE_BYTES) {
            Ok((bytes, truncated)) => {
                note.truncated |= truncated;
                let before = raw.units.len();
                parse_unit_show(&String::from_utf8_lossy(&bytes), manager, &mut raw.units);
                note.entries += raw.units.len() - before;
            }
            Err(_) => {
                note.gap = Some(format!(
                    "causes from the {manager} service manager are missing"
                ))
            }
        }
    }
    raw.notes.push(note);
    let mut note = SourceNote {
        name: "package log",
        ..SourceNote::default()
    };
    match read_tail(&dir.join("pacman.log"), MAX_SOURCE_BYTES) {
        Ok((bytes, truncated)) => {
            note.truncated = truncated;
            raw.package_log = bytes;
        }
        Err(_) => note.gap = Some("the package log is missing".into()),
    }
    raw.notes.push(note);
    Ok(raw)
}

fn job_entries(raw: &Raw, range: &Range, entries: &mut Vec<Entry>) {
    let mut jobs: Vec<Job> = raw
        .jobs
        .iter()
        .filter_map(job_entry)
        .filter(|job| job.time >= range.from && job.time < range.to && !skipped(&job.unit))
        .collect();
    jobs.sort_by_key(|job| job.time);
    // One launch from the desktop launcher can leave companions a moment before or after it: the browser's own
    // scope, or the browser helper service. They are the same launch, so they fold into it.
    let mut folded = vec![false; jobs.len()];
    let mut own_scope: BTreeMap<usize, String> = BTreeMap::new();
    let launches: Vec<(usize, String)> = jobs
        .iter()
        .enumerate()
        .filter_map(|(index, job)| match launch_app(&job.unit) {
            Some((app, true)) => Some((index, app_words(&app).0)),
            _ => None,
        })
        .collect();
    for (index, job) in jobs.iter().enumerate() {
        let companion = match launch_app(&job.unit) {
            Some((app, false)) => Some(app_words(&app).0),
            None if unit_stem(&job.unit) == "omarchy-browser" => Some(app_words("chromium").0),
            _ => None,
        };
        let Some(words) = companion else { continue };
        let near = launches.iter().find(|(launch, launch_words)| {
            *launch_words == words
                && !own_scope.contains_key(launch)
                && (job.time - jobs[*launch].time).num_seconds().abs() <= LAUNCH_FOLD_SECS
        });
        if let Some((launch, _)) = near {
            own_scope.insert(*launch, job.unit.clone());
            folded[index] = true;
        }
    }
    for (index, job) in jobs.iter().enumerate() {
        if folded[index] {
            continue;
        }
        let manager = format!("{} service manager: {}", job.manager, job.unit);
        let mut evidence = vec![manager];
        let event = if let Some((app, launcher)) = launch_app(&job.unit) {
            evidence.push(if launcher {
                "rule: desktop launcher scope".into()
            } else {
                "rule: scope the application made itself".into()
            });
            if let Some(scope) = own_scope.get(&index) {
                evidence.push(format!("companion folded in: {scope}"));
            }
            Event::Launch { app, launcher }
        } else {
            let facts = raw
                .units
                .get(&(job.manager, job.unit.clone()))
                .cloned()
                .unwrap_or_default();
            let trigger = if facts.triggered_by.iter().any(|t| t.ends_with(".timer")) {
                Trigger::Schedule
            } else if facts
                .triggered_by
                .iter()
                .any(|t| t.ends_with(".socket") || t.ends_with(".path"))
            {
                Trigger::Activation
            } else {
                Trigger::None
            };
            match trigger {
                Trigger::None => evidence.push("rule: no trigger recorded".into()),
                _ => evidence.push(format!("triggered by: {}", facts.triggered_by.join(" "))),
            }
            if job.failed {
                evidence.push("result: failed".into());
            }
            Event::Job {
                unit: job.unit.clone(),
                failed: job.failed,
                trigger,
                description: facts.description,
            }
        };
        entries.push(Entry {
            time: job.time,
            phrase: describe(&event),
            evidence,
        });
    }
}

fn power_entries(raw: &Raw, range: &Range, entries: &mut Vec<Entry>) {
    for value in &raw.power {
        let (Some(kind), Some(time)) = (power_kind(value), journal_time(value)) else {
            continue;
        };
        if time < range.from || time >= range.to {
            continue;
        }
        let mut evidence = vec![format!(
            "record: {}",
            field(value, "MESSAGE_ID").unwrap_or_default()
        )];
        if let Some(sleep) = field(value, "SLEEP") {
            evidence.push(format!("sleep kind: {}", short(sleep, 32)));
        }
        entries.push(Entry {
            time,
            phrase: describe(&Event::Power(kind)),
            evidence,
        });
    }
    let Some(list) = raw.boots.as_ref().and_then(Value::as_array) else {
        return;
    };
    let newest = list.iter().filter_map(|boot| boot["index"].as_i64()).max();
    let stamp = |value: &Value| {
        value
            .as_i64()
            .and_then(|micros| Utc.timestamp_opt(micros.div_euclid(1_000_000), 0).single())
    };
    for boot in list {
        let id = boot["boot_id"]
            .as_str()
            .map(|id| short(id, 8))
            .unwrap_or_default();
        let mut marks = vec![(stamp(&boot["first_entry"]), PowerKind::Boot)];
        if boot["index"].as_i64() != newest {
            marks.push((stamp(&boot["last_entry"]), PowerKind::BootEnd));
        }
        for (time, kind) in marks {
            let Some(time) = time.filter(|t| *t >= range.from && *t < range.to) else {
                continue;
            };
            let evidence = vec![
                format!("boot list: boot {id}"),
                "rule: first or last record of a boot".into(),
            ];
            entries.push(Entry {
                time,
                phrase: describe(&Event::Power(kind)),
                evidence,
            });
        }
    }
}

/// Only the program name and its option flags: package names and other arguments stay out.
fn command_flags(command: &str) -> String {
    let mut words = command.split_whitespace();
    let program = words
        .next()
        .map_or("unknown", |word| word.rsplit('/').next().unwrap_or(word));
    std::iter::once(program)
        .chain(words.filter(|w| w.starts_with('-')))
        .map(|w| short(w, 32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn package_entries(raw: &Raw, range: &Range, entries: &mut Vec<Entry>, note: &mut SourceNote) {
    struct Open {
        time: DateTime<Utc>,
        command: Option<String>,
        counts: BTreeMap<&'static str, usize>,
        packages: Vec<String>,
    }
    let finish = |txn: Open, finished: bool, entries: &mut Vec<Entry>| {
        if txn.time < range.from || txn.time >= range.to {
            return;
        }
        let count = |key| txn.counts.get(key).copied().unwrap_or(0);
        let event = Event::Packages {
            installed: count("installed"),
            upgraded: count("upgraded"),
            removed: count("removed"),
            finished,
        };
        let mut evidence = vec![format!(
            "package log: {}",
            txn.counts
                .iter()
                .map(|(k, n)| format!("{n} {k}"))
                .collect::<Vec<_>>()
                .join(", ")
        )];
        if let Some(command) = txn.command {
            evidence.push(format!("run as: {command}"));
        }
        if !txn.packages.is_empty() {
            evidence.push(format!("packages: {}", txn.packages.join(" ")));
        }
        entries.push(Entry {
            time: txn.time,
            phrase: describe(&event),
            evidence,
        });
    };
    let mut command = None;
    let mut open: Option<Open> = None;
    for line in String::from_utf8_lossy(&raw.package_log).lines() {
        let Some((stamp, rest)) = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] "))
        else {
            continue;
        };
        let Ok(time) = DateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%z") else {
            continue;
        };
        let time = time.with_timezone(&Utc);
        note.entries += 1;
        if let Some(cmd) = rest.strip_prefix("[PACMAN] Running '") {
            command = Some(command_flags(cmd.trim_end_matches('\'')));
        } else if let Some(alpm) = rest.strip_prefix("[ALPM] ") {
            match alpm {
                "transaction started" => {
                    if let Some(txn) = open.take() {
                        finish(txn, false, entries);
                    }
                    open = Some(Open {
                        time,
                        command: command.take(),
                        counts: BTreeMap::new(),
                        packages: Vec::new(),
                    });
                }
                "transaction completed" => {
                    if let Some(txn) = open.take() {
                        finish(txn, true, entries);
                    }
                }
                "transaction failed" | "transaction interrupted" => {
                    if let Some(txn) = open.take() {
                        finish(txn, false, entries);
                    }
                }
                _ => {
                    if let Some(txn) = open.as_mut() {
                        for verb in [
                            "installed",
                            "upgraded",
                            "removed",
                            "downgraded",
                            "reinstalled",
                        ] {
                            if let Some(name) =
                                alpm.strip_prefix(verb).and_then(|r| r.strip_prefix(' '))
                            {
                                *txn.counts.entry(verb).or_default() += 1;
                                if txn.packages.len() < 32 {
                                    txn.packages
                                        .push(short(name.split(' ').next().unwrap_or(name), 64));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(txn) = open.take() {
        finish(txn, false, entries);
    }
}

fn local(time: DateTime<Utc>, zone: Tz) -> String {
    time.with_timezone(&zone)
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

/// Group by local day, then fold identical lines of a day, keeping each time.
fn fold(mut entries: Vec<Entry>, zone: Tz) -> Vec<Day> {
    entries.sort_by_key(|entry| entry.time);
    let mut days: BTreeMap<String, Vec<Line>> = BTreeMap::new();
    let mut index: BTreeMap<(String, String, Category, Actor), usize> = BTreeMap::new();
    for entry in entries {
        let date = entry
            .time
            .with_timezone(&zone)
            .format("%Y-%m-%d")
            .to_string();
        let at = local(entry.time, zone);
        let key = (
            date.clone(),
            entry.phrase.words.clone(),
            entry.phrase.category,
            entry.phrase.actor,
        );
        let lines = days.entry(date).or_default();
        match index.get(&key) {
            Some(&i) => {
                let line = &mut lines[i];
                line.count += 1;
                line.last = at.clone();
                if line.times.len() < MAX_TIMES {
                    line.times.push(at);
                }
                for item in entry.evidence {
                    if line.evidence.len() < MAX_EVIDENCE && !line.evidence.contains(&item) {
                        line.evidence.push(item);
                    }
                }
            }
            None => {
                index.insert(key, lines.len());
                let mut evidence = entry.evidence;
                evidence.truncate(MAX_EVIDENCE);
                lines.push(Line {
                    words: entry.phrase.words,
                    category: entry.phrase.category,
                    actor: entry.phrase.actor,
                    count: 1,
                    first: at.clone(),
                    last: at.clone(),
                    times: vec![at],
                    evidence,
                    many: entry.phrase.many,
                });
            }
        }
    }
    days.into_iter()
        .map(|(date, lines)| Day { date, lines })
        .collect()
}

/// Read the machine's records for `range` from `source` and put them in plain lines, by day.
/// Runs only while called; a missing or refusing journal is an error with its reason.
pub fn collect(range: Range, source: &Source) -> Result<Log> {
    let mut raw = match source {
        Source::System {
            journalctl,
            systemctl,
            package_log,
        } => read_system(&range, journalctl, systemctl, package_log)?,
        Source::Recorded(dir) => read_recorded(dir)?,
    };
    let mut entries = Vec::new();
    job_entries(&raw, &range, &mut entries);
    power_entries(&raw, &range, &mut entries);
    let mut package_note = raw.notes.pop().unwrap_or_default();
    package_entries(&raw, &range, &mut entries, &mut package_note);
    raw.notes.push(package_note);
    Ok(Log {
        zone: range.zone.name().to_owned(),
        from: local(range.from, range.zone),
        to: local(range.to, range.zone),
        days: fold(entries, range.zone),
        sources: raw.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_stems_drop_type_instance_and_run_number() {
        assert_eq!(
            unit_stem("polkit-agent-helper@31-122914-3882906_16491137-1000.service"),
            "polkit-agent-helper"
        );
        assert_eq!(
            unit_stem("lorengine-checkpoint-1793107481707305630.service"),
            "lorengine-checkpoint"
        );
        assert_eq!(unit_stem("man-db.service"), "man-db");
        assert_eq!(unit_stem("user@1000.service"), "user");
    }

    #[test]
    fn descriptions_with_paths_names_or_ids_are_not_words() {
        assert_eq!(
            plain_description("Swarm load guard: confines the swarm", "x.service").as_deref(),
            Some("Swarm load guard")
        );
        assert_eq!(
            plain_description("/usr/bin/foo --token=1", "x.service"),
            None
        );
        assert_eq!(
            plain_description("omarchy-browser-17.service", "omarchy-browser-17.service"),
            None
        );
        assert_eq!(
            plain_description("Run deadbeefcafe0011 now", "x.service"),
            None
        );
        assert_eq!(
            plain_description("one two three four five six seven", "x").as_deref(),
            Some("One two three four five")
        );
    }

    #[test]
    fn launcher_scope_names_decide_origin() {
        assert_eq!(
            launch_app("app-Hyprland-nemo-1a2b3c4d.scope"),
            Some(("nemo".into(), true))
        );
        assert_eq!(
            launch_app("app-Hyprland-xdg\\x2dterminal\\x2dexec-0a1b2c3d.scope"),
            Some(("xdg-terminal-exec".into(), true))
        );
        assert_eq!(
            launch_app("app-org.chromium.Chromium-12345.scope"),
            Some(("chromium".into(), false))
        );
        assert_eq!(launch_app("run-u12.scope"), None);
    }

    #[test]
    fn a_child_that_runs_too_long_is_ended_with_a_reason() {
        let started = Instant::now();
        let error = run_bounded(
            Path::new("sleep"),
            &["5".into()],
            1024,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(error.contains("took longer"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn output_past_the_byte_bound_is_kept_and_marked() {
        let (bytes, truncated) = run_bounded(Path::new("yes"), &[], 4096, MAX_CHILD_TIME).unwrap();
        assert_eq!(bytes.len(), 4096);
        assert!(truncated);
    }
}
