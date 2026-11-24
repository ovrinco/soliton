//! Running processes, each named with the reason it runs, and a stop bound to
//! the exact process that was chosen.
//!
//! Identity comes from /proc only: the short name, the executable's file
//! name, the owner, the parent, the start time and the control group. A
//! process's command arguments are never read: they can carry private paths
//! and secrets.
//!
//! The reason comes from evidence, in this order: a kernel thread; the system
//! manager; the main process of a service (with its timer or socket when one
//! started it); the first process of its own launch (a desktop launch or a
//! login session); the parent that started it. A process whose starter has
//! ended and that only hangs under a service manager has an unknown reason,
//! and the words say so together with what is known. The words use the
//! unit's own description where it has one and never a raw unit name alone;
//! the raw names stay in the evidence.
//!
//! A stop names the process by pid and start time (clock ticks since boot).
//! The pair is checked twice, once before the ownership check and again
//! immediately before the signal, so a reused pid is reported as stale and
//! never signalled. Only processes of the caller's own user are signalled.
//! Keeping a process is the absence of a stop: nothing here signals a process
//! except `stop`. Nothing here keeps a child or a thread running: the two
//! programs it calls (the unit query and the signal) are waited for at once.
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Linux reports start times in USER_HZ, which is 100 on every architecture
/// this runs on.
const TICKS_PER_SECOND: u64 = 100;
const PF_KTHREAD: u64 = 0x0020_0000;
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const TIMEOUT: &str = "/usr/bin/timeout";
const KILL: &str = "/usr/bin/kill";
/// The bound on one unit query, in seconds, for `timeout`.
const QUERY_SECONDS: &str = "3";
const UNIT_SUFFIXES: [&str; 4] = [".service", ".scope", ".slice", ".socket"];
const DEFAULT_WAIT: Duration = Duration::from_millis(2000);
const LONGEST_WAIT: Duration = Duration::from_millis(5000);

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("unknown process command {0}")]
    UnknownCommand(String),
    #[error("a process stop needs the chosen pid")]
    MissingPid,
    #[error("a process stop needs the chosen start_ticks")]
    MissingStartTicks,
    #[error("signal must be term or kill, not {0}")]
    BadSignal(String),
}

/// `process-list` and `process-stop` for the service line protocol.
pub fn command(command: &str, args: &Value) -> Result<Value, ProcessError> {
    match command {
        "process-list" => {
            let processes = list();
            let unknown = processes.iter().filter(|p| !p.reason.known).count();
            Ok(json!({
                "count": processes.len(),
                "reasons_unknown": unknown,
                "processes": processes,
            }))
        }
        "process-stop" => {
            let pid = args["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid > 0)
                .ok_or(ProcessError::MissingPid)?;
            let start_ticks = args["start_ticks"]
                .as_u64()
                .ok_or(ProcessError::MissingStartTicks)?;
            let signal = match args["signal"].as_str().unwrap_or("term") {
                "term" => Signal::Term,
                "kill" => Signal::Kill,
                other => return Err(ProcessError::BadSignal(other.to_owned())),
            };
            let wait = args["wait_ms"]
                .as_u64()
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_WAIT);
            Ok(json!(stop_report(pid, start_ticks, signal, wait)))
        }
        other => Err(ProcessError::UnknownCommand(other.to_owned())),
    }
}

/// One item of evidence for a reason: its kind (`unit`, `description`,
/// `slice`, `timer`, `socket`, `wanted-by`, `launch`, `parent`,
/// `kernel-thread`) and its raw value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub kind: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub known: bool,
    /// Plain words; an unknown reason starts "Reason unknown".
    pub words: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Parent {
    pub pid: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Process {
    pub pid: u32,
    /// With `pid`, the identity a stop is bound to.
    pub start_ticks: u64,
    /// The kernel's short name, at most 15 characters.
    pub name: String,
    /// The executable's file name; none when /proc withholds it.
    pub executable: Option<String>,
    pub uid: Option<u32>,
    pub user: Option<String>,
    /// RFC 3339 local time.
    pub started: Option<String>,
    /// One letter from /proc (R, S, D, Z, ...).
    pub state: String,
    pub memory_kib: Option<u64>,
    pub parent: Parent,
    pub unit: Option<String>,
    /// The unit's own words, only when the unit accounts for this process.
    pub unit_description: Option<String>,
    pub reason: Reason,
    pub can_stop: bool,
    pub refusal: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    Term,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Signalled and gone within the wait.
    Stopped,
    /// Already ended (or a zombie) before any signal.
    Exited,
    /// The pid now belongs to another start: nothing signalled.
    Stale,
    NotPermitted,
    /// Signalled and still alive after the wait.
    StillRunning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StopReport {
    pub outcome: Outcome,
    pub message: &'static str,
    pub selected: Identity,
    /// The process signalled, else none.
    pub affected: Option<Identity>,
    pub signal: Signal,
    pub signal_sent: bool,
}

/// What /proc says about one process at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub ppid: u32,
    pub start_ticks: u64,
    pub kernel: bool,
}

/// Parses /proc/<pid>/stat. The short name sits in parentheses and may itself
/// hold spaces and parentheses, so the fields after it are found from the
/// last closing parenthesis.
pub fn parse_stat(pid: u32, text: &str) -> Option<Proc> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text.get(open + 1..close)?.to_owned();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // rest[0] is field 3 (state); field n is rest[n - 3].
    let state = rest.first()?.chars().next()?;
    let ppid = rest.get(1)?.parse().ok()?;
    let flags: u64 = rest.get(6)?.parse().ok()?;
    let start_ticks = rest.get(19)?.parse().ok()?;
    Some(Proc {
        pid,
        name,
        state,
        ppid,
        start_ticks,
        kernel: flags & PF_KTHREAD != 0,
    })
}

pub fn read_proc(pid: u32) -> Option<Proc> {
    parse_stat(pid, &fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

fn status_fields(pid: u32) -> (Option<u32>, Option<u64>) {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return (None, None);
    };
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
    };
    (
        field("Uid:").and_then(|uid| uid.parse().ok()),
        field("VmRSS:").and_then(|kib| kib.parse().ok()),
    )
}

fn own_uid() -> Option<u32> {
    status_fields(std::process::id()).0
}

/// The systemd unit a control group path belongs to, and the manager that
/// owns it: the deepest unit in the path, user units below `user@N.service`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub unit: Option<String>,
    pub slice: Option<String>,
    pub user_manager: Option<u32>,
}

pub fn parse_cgroup(text: &str) -> Placement {
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap_or("");
    let mut placement = Placement {
        unit: None,
        slice: None,
        user_manager: None,
    };
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if let Some(uid) = part
            .strip_prefix("user@")
            .and_then(|rest| rest.strip_suffix(".service"))
            .and_then(|uid| uid.parse().ok())
        {
            placement.user_manager = Some(uid);
            placement.unit = Some(part.to_owned());
            continue;
        }
        if part.ends_with(".slice") {
            placement.slice = Some(part.to_owned());
        } else if UNIT_SUFFIXES.iter().any(|suffix| part.ends_with(suffix)) {
            placement.unit = Some(part.to_owned());
        }
    }
    // The user manager's own unit is a system unit; units below it are the
    // user manager's.
    if placement
        .unit
        .as_deref()
        .is_some_and(|unit| unit.starts_with("user@"))
    {
        placement.user_manager = None;
    }
    placement
}

/// A unit's own words. A transient unit made by systemd-run carries the full
/// command line as its description; only the program's file name is kept.
pub fn safe_description(description: &str) -> String {
    match description.strip_prefix("[systemd-run]") {
        Some(rest) => {
            let program = rest.split_whitespace().next().unwrap_or("");
            program.rsplit('/').next().unwrap_or(program).to_owned()
        }
        None => description.to_owned(),
    }
}

/// A unit name with systemd's `\xNN` escapes decoded, for reading.
pub fn unescape(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find("\\x") {
        out.push_str(&rest[..at]);
        let code = rest
            .get(at + 2..at + 4)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .filter(u8::is_ascii);
        match code {
            Some(code) => {
                out.push(code as char);
                rest = &rest[at + 4..];
            }
            None => {
                out.push('\\');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A unit's name made readable when it has no description of its own: the
/// escapes decoded, the type suffix, the launcher prefix and the trailing
/// launch number dropped (`app-Hyprland-xdg\x2dterminal\x2dexec-38c4.scope`
/// reads `xdg-terminal-exec`).
pub fn readable_unit(name: &str) -> String {
    let mut text = unescape(name);
    if let Some(suffix) = UNIT_SUFFIXES.iter().find(|suffix| text.ends_with(*suffix)) {
        text.truncate(text.len() - suffix.len());
    }
    let mut text = text.as_str();
    for prefix in ["app-Hyprland-", "app-", "run-"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest;
            break;
        }
    }
    if let Some((head, tail)) = text.rsplit_once('-')
        && !head.is_empty()
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_hexdigit())
    {
        text = head;
    }
    // A transient run unit named only by its number (`run-u42`) keeps it.
    text.to_owned()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Unit {
    pub description: String,
    pub main_pid: u32,
    pub wanted_by: Vec<String>,
    pub triggered_by: Vec<String>,
}

impl Unit {
    /// The unit's own words, or its readable name when it has none (systemd
    /// fills an empty description with the unit's name).
    fn words(&self, name: &str) -> String {
        if self.description.is_empty() || self.description == name {
            readable_unit(name)
        } else {
            self.description.clone()
        }
    }
}

/// Parses `systemctl show -p Id,Description,MainPID,WantedBy,TriggeredBy`.
pub fn parse_units(text: &str) -> BTreeMap<String, Unit> {
    let mut found = BTreeMap::new();
    for block in text.split("\n\n") {
        let mut id = None;
        let mut unit = Unit::default();
        for line in block.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let list = || value.split_whitespace().map(str::to_owned).collect();
            match key {
                "Id" => id = Some(value.to_owned()),
                "Description" => unit.description = safe_description(value),
                "MainPID" => unit.main_pid = value.parse().unwrap_or(0),
                "WantedBy" => unit.wanted_by = list(),
                "TriggeredBy" => unit.triggered_by = list(),
                _ => {}
            }
        }
        if let Some(id) = id {
            found.insert(id, unit);
        }
    }
    found
}

/// Reads unit properties, one call per manager, bounded by `timeout` and
/// waited for here. A manager that does not answer leaves its units without
/// words rather than failing the list.
fn query_units(user: bool, units: &BTreeSet<String>) -> BTreeMap<String, Unit> {
    if units.is_empty() {
        return BTreeMap::new();
    }
    let mut command = Command::new(TIMEOUT);
    command.args(["-k", "1", QUERY_SECONDS, SYSTEMCTL]);
    if user {
        command.arg("--user");
    }
    let output = command
        .args([
            "show",
            "--no-pager",
            "-p",
            "Id,Description,MainPID,WantedBy,TriggeredBy",
            "--",
        ])
        .args(units)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) => parse_units(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => BTreeMap::new(),
    }
}

/// Why a process is running, from the evidence there is. `adopted` is true
/// when a service manager (the system's or a user's) is its parent and it is
/// not the eldest process of its launch: its starter has ended.
pub fn reason(
    proc: &Proc,
    placement: &Placement,
    unit: Option<&Unit>,
    parent: Option<&Proc>,
    parent_placement: Option<&Placement>,
    adopted: bool,
) -> Reason {
    let mut evidence = Vec::new();
    let mut add = |kind: &'static str, value: String| evidence.push(Evidence { kind, value });
    let unit_name = placement.unit.as_deref();
    if let Some(name) = unit_name {
        add("unit", name.to_owned());
    }
    if let Some(slice) = &placement.slice {
        add("slice", slice.clone());
    }
    let within = unit_name.map(|name| match unit {
        Some(unit) => unit.words(name),
        None => readable_unit(name),
    });
    let known = |words: String, evidence| Reason {
        known: true,
        words,
        evidence,
    };
    if proc.kernel {
        add("kernel-thread", proc.name.clone());
        return known("Part of the running kernel".into(), evidence);
    }
    if proc.pid == 1 {
        return known("The system's service manager".into(), evidence);
    }
    // The main process of a service, with what started the service.
    if let (Some(name), Some(unit)) = (unit_name, unit)
        && name.ends_with(".service")
        && unit.main_pid == proc.pid
    {
        if !unit.description.is_empty() {
            add("description", unit.description.clone());
        }
        let timer = unit.triggered_by.iter().find(|t| t.ends_with(".timer"));
        let socket = unit.triggered_by.iter().find(|t| t.ends_with(".socket"));
        for by in &unit.triggered_by {
            let kind = if by.ends_with(".timer") {
                "timer"
            } else {
                "socket"
            };
            add(kind, by.clone());
        }
        if !unit.wanted_by.is_empty() {
            add("wanted-by", unit.wanted_by.join(" "));
        }
        let words = unit.words(name);
        let words = if timer.is_some() {
            format!("Scheduled task: {words}")
        } else if socket.is_some() {
            format!("Started on demand: {words}")
        } else if placement.user_manager.is_some() {
            format!("Your background service: {words}")
        } else {
            format!("System service: {words}")
        };
        return known(words, evidence);
    }
    // The first process of its own launch: the parent sits elsewhere. A
    // launch whose first process a manager adopted is not its own launch.
    let inherited = parent_placement.is_some_and(|parent| parent == placement);
    if let Some(name) = unit_name
        && name.ends_with(".scope")
        && !inherited
    {
        let what = within.clone().unwrap_or_default();
        let words = if let Some(session) = name
            .strip_prefix("session-")
            .and_then(|rest| rest.strip_suffix(".scope"))
        {
            Some(format!("Login session {session}"))
        } else if name == "init.scope"
            && let Some(uid) = placement.user_manager
        {
            Some(format!("Service manager for user {uid}"))
        } else if adopted {
            None
        } else if name.starts_with("app-Hyprland-") {
            Some(format!("Opened from the desktop: {what}"))
        } else {
            Some(format!("Launched on its own: {what}"))
        };
        if let Some(words) = words {
            add("launch", name.to_owned());
            return known(words, evidence);
        }
    }
    let Some(parent) = parent else {
        add("parent", format!("pid {}", proc.ppid));
        let words = match &within {
            Some(within) => format!("Reason unknown: its starter is not visible; within {within}"),
            None => "Reason unknown: its starter is not visible".to_owned(),
        };
        return Reason {
            known: false,
            words,
            evidence,
        };
    };
    add("parent", format!("{} (pid {})", parent.name, parent.pid));
    if adopted {
        // Adopted by a service manager after its starter ended.
        let words = match &within {
            Some(within) => format!("Reason unknown: its starter has ended; within {within}"),
            None => "Reason unknown: its starter has ended, outside any service".to_owned(),
        };
        return Reason {
            known: false,
            words,
            evidence,
        };
    }
    let words = match &within {
        Some(within) => format!(
            "Started by {} (pid {}), within {within}",
            parent.name, parent.pid
        ),
        None => format!("Started by {} (pid {})", parent.name, parent.pid),
    };
    known(words, evidence)
}

/// Why a process will not be signalled, or none when it may.
pub fn refusal(proc: &Proc, uid: Option<u32>, own: Option<u32>) -> Option<&'static str> {
    if proc.kernel {
        Some("Kernel threads cannot be stopped")
    } else if proc.pid == 1 {
        Some("The system manager is never stopped")
    } else if proc.pid == std::process::id() {
        Some("Soliton does not stop itself")
    } else if own.is_none() || uid != own {
        Some("This process belongs to another user")
    } else {
        None
    }
}

fn user_names() -> BTreeMap<u32, String> {
    fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_owned()))
        })
        .collect()
}

fn boot_time() -> Option<i64> {
    fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse().ok())
}

fn started(boot: Option<i64>, ticks: u64) -> Option<String> {
    let seconds = i64::try_from(ticks / TICKS_PER_SECOND).ok()?;
    DateTime::from_timestamp(boot? + seconds, 0).map(|time| time.with_timezone(&Local).to_rfc3339())
}

fn exe_name(pid: u32) -> Option<String> {
    let target = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// Every process now running, each with its reason, in pid order. One pass
/// over /proc and at most two unit queries (system and this user's).
pub fn list() -> Vec<Process> {
    let own = own_uid();
    let names = user_names();
    let boot = boot_time();
    let procs: BTreeMap<u32, Proc> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .filter_map(|pid| Some((pid, read_proc(pid)?)))
        .collect();
    let placements: BTreeMap<u32, Placement> = procs
        .keys()
        .map(|pid| {
            let text = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
            (*pid, parse_cgroup(&text))
        })
        .collect();
    let mut system_units = BTreeSet::new();
    let mut user_units = BTreeSet::new();
    for placement in placements.values() {
        if let Some(unit) = &placement.unit {
            match placement.user_manager {
                Some(uid) if Some(uid) == own => user_units.insert(unit.clone()),
                Some(_) => false,
                None => system_units.insert(unit.clone()),
            };
        }
    }
    let system = query_units(false, &system_units);
    let user = query_units(true, &user_units);
    let is_manager = |pid: u32| {
        pid == 1
            || placements.get(&pid).is_some_and(|placement| {
                placement.unit.as_deref() == Some("init.scope") && placement.user_manager.is_some()
            })
    };
    // The eldest process of each launch: a desktop launcher hands its
    // program to the user's manager and exits, so the program's parent is
    // the manager while the launch is still its own.
    let mut eldest: BTreeMap<(Option<u32>, &str), (u64, u32)> = BTreeMap::new();
    for (pid, placement) in &placements {
        if let Some(unit) = placement.unit.as_deref().filter(|u| u.ends_with(".scope")) {
            let key = (procs[pid].start_ticks, *pid);
            let entry = eldest.entry((placement.user_manager, unit)).or_insert(key);
            *entry = (*entry).min(key);
        }
    }
    procs
        .values()
        .map(|proc| {
            let placement = &placements[&proc.pid];
            let is_eldest = placement.unit.as_deref().is_some_and(|unit| {
                eldest.get(&(placement.user_manager, unit)).map(|e| e.1) == Some(proc.pid)
            });
            let unit = placement
                .unit
                .as_ref()
                .and_then(|name| match placement.user_manager {
                    Some(_) => user.get(name),
                    None => system.get(name),
                });
            let parent = procs.get(&proc.ppid);
            let reason = reason(
                proc,
                placement,
                unit,
                parent,
                placements.get(&proc.ppid),
                is_manager(proc.ppid) && !is_eldest,
            );
            // A unit's description is lent only to a process the unit
            // accounts for (its main process or its launch).
            let accounts = reason
                .evidence
                .iter()
                .any(|e| matches!(e.kind, "description" | "launch"));
            let (uid, memory_kib) = status_fields(proc.pid);
            let refused = refusal(proc, uid, own);
            Process {
                pid: proc.pid,
                start_ticks: proc.start_ticks,
                name: proc.name.clone(),
                executable: exe_name(proc.pid),
                uid,
                user: uid.and_then(|uid| names.get(&uid).cloned()),
                started: started(boot, proc.start_ticks),
                state: proc.state.to_string(),
                memory_kib,
                parent: Parent {
                    pid: proc.ppid,
                    name: parent.map(|p| p.name.clone()),
                },
                unit: placement.unit.clone(),
                unit_description: unit
                    .filter(|_| accounts)
                    .map(|u| u.description.clone())
                    .filter(|d| !d.is_empty()),
                reason,
                can_stop: refused.is_none(),
                refusal: refused,
            }
        })
        .collect()
}

/// True when the chosen pid no longer runs the chosen process.
fn gone_or_replaced(pid: u32, start_ticks: u64) -> bool {
    match read_proc(pid) {
        None => true,
        Some(now) => now.start_ticks != start_ticks || matches!(now.state, 'Z' | 'X'),
    }
}

/// Ends the chosen process on his act and says what happened in one word.
pub fn stop(pid: u32, start_ticks: u64, signal: Signal, wait: Duration) -> Outcome {
    stop_report(pid, start_ticks, signal, wait).outcome
}

/// `stop` with what was chosen, what was signalled and a plain sentence.
pub fn stop_report(pid: u32, start_ticks: u64, signal: Signal, wait: Duration) -> StopReport {
    let selected = Identity { pid, start_ticks };
    let answer = |outcome, message, affected, signal_sent| StopReport {
        outcome,
        message,
        selected: selected.clone(),
        affected,
        signal,
        signal_sent,
    };
    let ended = || answer(Outcome::Exited, "It had already ended", None, false);
    let stale = || {
        answer(
            Outcome::Stale,
            "That number now belongs to another process; nothing was stopped",
            None,
            false,
        )
    };
    // The identity as chosen, checked before anything else.
    let Some(proc) = read_proc(pid) else {
        return ended();
    };
    if proc.start_ticks != start_ticks {
        return stale();
    }
    if matches!(proc.state, 'Z' | 'X') {
        return ended();
    }
    if let Some(why) = refusal(&proc, status_fields(pid).0, own_uid()) {
        return answer(Outcome::NotPermitted, why, None, false);
    }
    // Last check of the pair right before the signal.
    match read_proc(pid) {
        Some(now) if now.start_ticks != start_ticks => return stale(),
        Some(now) if !matches!(now.state, 'Z' | 'X') => {}
        _ => return ended(),
    }
    let name = match signal {
        Signal::Term => "TERM",
        Signal::Kill => "KILL",
    };
    let sent = Command::new(KILL)
        .args(["-s", name, "--", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let output = match sent {
        Ok(output) => output,
        Err(_) => return answer(Outcome::Failed, "The stop could not be sent", None, false),
    };
    if !output.status.success() {
        if gone_or_replaced(pid, start_ticks) {
            return ended();
        }
        let refused = String::from_utf8_lossy(&output.stderr).contains("not permitted");
        let outcome = if refused {
            Outcome::NotPermitted
        } else {
            Outcome::Failed
        };
        return answer(outcome, "The system refused the stop", None, false);
    }
    let affected = Some(Identity {
        pid: proc.pid,
        start_ticks: proc.start_ticks,
    });
    let deadline = Instant::now() + wait.min(LONGEST_WAIT);
    loop {
        if gone_or_replaced(pid, start_ticks) {
            return answer(Outcome::Stopped, "It stopped", affected, true);
        }
        if Instant::now() >= deadline {
            return answer(
                Outcome::StillRunning,
                "Asked to stop, still running",
                affected,
                true,
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(pid: u32, name: &str, ppid: u32, flags: u64) -> Proc {
        let text =
            format!("{pid} ({name}) S {ppid} 1 1 0 -1 {flags} 0 0 0 0 0 0 0 0 20 0 1 0 77 0 0");
        parse_stat(pid, &text).unwrap()
    }

    fn unit(description: &str, main_pid: u32, triggered_by: &[&str]) -> Unit {
        Unit {
            description: description.into(),
            main_pid,
            wanted_by: vec![],
            triggered_by: triggered_by.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn stat_name_may_hold_parentheses_and_spaces() {
        let text = "42 (a) b (c)) S 7 42 42 0 -1 4194304 1 0 0 0 0 0 0 0 20 0 1 0 12345 0 0";
        let proc = parse_stat(42, text).unwrap();
        assert_eq!(proc.name, "a) b (c)");
        assert_eq!((proc.state, proc.ppid, proc.start_ticks), ('S', 7, 12345));
        assert!(!proc.kernel);
        assert!(stat(2, "kthreadd", 0, 2129984).kernel);
    }

    #[test]
    fn cgroup_names_the_deepest_unit_and_its_manager() {
        let user = parse_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/qream-daemon.service\n",
        );
        assert_eq!(user.unit.as_deref(), Some("qream-daemon.service"));
        assert_eq!(user.slice.as_deref(), Some("app.slice"));
        assert_eq!(user.user_manager, Some(1000));
        let manager = parse_cgroup("0::/user.slice/user-1000.slice/user@1000.service/init.scope");
        assert_eq!(manager.unit.as_deref(), Some("init.scope"));
        assert_eq!(manager.user_manager, Some(1000));
        let system = parse_cgroup("0::/system.slice/systemd-journald.service");
        assert_eq!(system.user_manager, None);
        let root = parse_cgroup("0::/");
        assert_eq!((root.unit, root.slice), (None, None));
    }

    #[test]
    fn unit_names_read_plainly_and_runs_keep_only_the_program() {
        assert_eq!(
            safe_description("[systemd-run] /home/x/bin/qreamd --token secret"),
            "qreamd"
        );
        assert_eq!(
            readable_unit("app-Hyprland-xdg\\x2dterminal\\x2dexec-38c420fd.scope"),
            "xdg-terminal-exec"
        );
        assert_eq!(readable_unit("app-cursor-2320213.scope"), "cursor");
        assert_eq!(readable_unit("pipewire.service"), "pipewire");
        assert_eq!(unescape("a\\xzz"), "a\\xzz");
    }

    #[test]
    fn units_are_read_from_systemctl_show() {
        let units = parse_units(
            "Id=a.service\nDescription=Alpha\nMainPID=5\nWantedBy=default.target\nTriggeredBy=a.timer\n\nId=b.scope\nDescription=b.scope\nMainPID=0\n",
        );
        assert_eq!(
            units["a.service"],
            Unit {
                description: "Alpha".into(),
                main_pid: 5,
                wanted_by: vec!["default.target".into()],
                triggered_by: vec!["a.timer".into()],
            }
        );
        assert_eq!(units["b.scope"].words("b.scope"), "b");
    }

    #[test]
    fn reasons_read_in_plain_words_from_their_evidence() {
        let system = parse_cgroup("0::/system.slice/backup.service");
        let main = stat(50, "backup", 1, 0);
        let timed = unit("Nightly backup", 50, &["backup.timer"]);
        let why = reason(&main, &system, Some(&timed), None, None, true);
        assert_eq!(
            (why.known, why.words.as_str()),
            (true, "Scheduled task: Nightly backup")
        );
        assert!(why.evidence.contains(&Evidence {
            kind: "timer",
            value: "backup.timer".into()
        }));
        let socket = unit("", 50, &["backup.socket"]);
        let why = reason(&main, &system, Some(&socket), None, None, true);
        assert_eq!(why.words, "Started on demand: backup");
        let plain = unit("Journal", 50, &[]);
        assert_eq!(
            reason(&main, &system, Some(&plain), None, None, true).words,
            "System service: Journal"
        );

        let app = parse_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-Hyprland-nemo-73f995b4.scope",
        );
        let hypr = parse_cgroup("0::/user.slice/user-1000.slice/session-2.scope");
        let launcher = stat(60, "Hyprland", 9, 0);
        let first = stat(61, "nemo", 60, 0);
        let why = reason(&first, &app, None, Some(&launcher), Some(&hypr), false);
        assert_eq!(
            (why.known, why.words.as_str()),
            (true, "Opened from the desktop: nemo")
        );

        // A child of a known parent in the same launch is started by it.
        let child = stat(62, "bash", 61, 0);
        let why = reason(&child, &app, None, Some(&first), Some(&app), false);
        assert_eq!(
            (why.known, why.words.as_str()),
            (true, "Started by nemo (pid 61), within nemo")
        );

        // Adopted by a service manager after its starter ended: unknown,
        // with what is known, and never a raw unit name alone.
        let orphan = stat(63, "worker", 9, 0);
        let manager = stat(9, "systemd", 1, 0);
        let manager_placement =
            parse_cgroup("0::/user.slice/user-1000.slice/user@1000.service/init.scope");
        let why = reason(
            &orphan,
            &app,
            None,
            Some(&manager),
            Some(&manager_placement),
            true,
        );
        assert!(!why.known);
        assert_eq!(
            why.words,
            "Reason unknown: its starter has ended; within nemo"
        );
        assert!(!why.words.contains(".scope"));
        // The eldest of a desktop launch whose launcher handed it to the
        // manager is still its own launch.
        let why = reason(
            &orphan,
            &app,
            None,
            Some(&manager),
            Some(&manager_placement),
            false,
        );
        assert_eq!(
            (why.known, why.words.as_str()),
            (true, "Opened from the desktop: nemo")
        );
        let loose = reason(&orphan, &parse_cgroup("0::/"), None, None, None, false);
        assert_eq!(loose.words, "Reason unknown: its starter is not visible");
    }

    #[test]
    fn kernel_threads_and_managers_are_known_and_never_stoppable() {
        let root = parse_cgroup("0::/");
        let kernel = stat(3, "kworker/0:1", 2, PF_KTHREAD);
        assert_eq!(
            reason(&kernel, &root, None, None, None, false).words,
            "Part of the running kernel"
        );
        assert!(refusal(&kernel, Some(0), Some(0)).is_some());
        let init = stat(1, "systemd", 0, 0);
        assert!(
            reason(
                &init,
                &parse_cgroup("0::/init.scope"),
                None,
                None,
                None,
                false
            )
            .known
        );
        assert!(refusal(&init, Some(0), Some(0)).is_some());
        let user = parse_cgroup("0::/user.slice/user-1000.slice/user@1000.service/init.scope");
        let manager = stat(9, "systemd", 1, 0);
        let why = reason(
            &manager,
            &user,
            None,
            Some(&init),
            Some(&parse_cgroup("0::/init.scope")),
            true,
        );
        assert_eq!(why.words, "Service manager for user 1000");
        let mine = stat(70, "sleep", 9, 0);
        assert_eq!(
            refusal(&mine, Some(1000), Some(2000)),
            Some("This process belongs to another user")
        );
        assert_eq!(refusal(&mine, Some(1000), Some(1000)), None);
    }
}
