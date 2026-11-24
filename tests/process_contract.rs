//! The process module against two children this test owns: both are listed
//! with a known reason, one is stopped by its chosen identity while the other
//! keeps running, a stale identity and the system manager are never
//! signalled, and the test ends its own children.
use soliton::processes;

use processes::{Outcome, Process, Signal};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// The marker in each child's argv[0]; `pgrep -f sw-t17-child` after the run
/// shows a leftover.
const MARKER: &str = "sw-t17-child";

/// A child owned by this test, ended and reaped when dropped.
struct Owned(Child);

impl Owned {
    fn start(label: &str) -> Owned {
        let script = format!("exec -a {MARKER}-{label} /usr/bin/sleep 297.25");
        Owned(
            Command::new("/usr/bin/bash")
                .args(["-c", &script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .expect("bash and sleep are on every machine this runs on"),
        )
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }

    fn running(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn row(list: &[Process], pid: u32) -> Process {
    list.iter()
        .find(|row| row.pid == pid)
        .cloned()
        .unwrap_or_else(|| panic!("pid {pid} missing from the list"))
}

/// Waits until bash has become sleep, so the listed name is the child's own.
fn settled(child: &Owned) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if processes::read_proc(child.pid()).is_some_and(|p| p.name == "sleep") {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("child {} did not become sleep", child.pid());
}

#[test]
fn both_children_are_listed_with_a_known_reason_and_one_stops() {
    let mut a = Owned::start("a");
    let mut b = Owned::start("b");
    settled(&a);
    settled(&b);
    let list = processes::list();
    let (ra, rb) = (row(&list, a.pid()), row(&list, b.pid()));
    for r in [&ra, &rb] {
        println!("listed: {}", serde_json::to_string(r).unwrap());
        assert_eq!(r.name, "sleep");
        assert_eq!(r.executable.as_deref(), Some("sleep"));
        assert_eq!(r.parent.pid, std::process::id());
        assert!(r.reason.known, "{:?}", r.reason);
        assert!(
            r.reason.words.starts_with("Started by "),
            "{}",
            r.reason.words
        );
        assert!(
            r.reason
                .evidence
                .iter()
                .any(|e| e.kind == "parent" && e.value.contains(&std::process::id().to_string()))
        );
        assert!(r.can_stop && r.refusal.is_none());
        assert!(r.started.is_some());
        let text = serde_json::to_string(r).unwrap();
        assert!(!text.contains("297.25"), "arguments never appear: {text}");
        assert!(!text.contains(MARKER), "argv[0] never appears: {text}");
    }
    // Listing keeps both: nothing was signalled.
    assert!(a.running() && b.running());

    let report =
        processes::stop_report(ra.pid, ra.start_ticks, Signal::Term, Duration::from_secs(2));
    println!("stop: {}", serde_json::to_string(&report).unwrap());
    assert_eq!(report.outcome, Outcome::Stopped);
    assert!(report.signal_sent);
    assert_eq!(
        report.affected.as_ref().map(|i| (i.pid, i.start_ticks)),
        Some((ra.pid, ra.start_ticks))
    );
    assert!(!a.running(), "the chosen child ended");
    assert!(b.running(), "the other child keeps running");
    let after = processes::list();
    assert_eq!(row(&after, b.pid()).start_ticks, rb.start_ticks);
    assert!(
        after
            .iter()
            .all(|r| r.pid != ra.pid || r.start_ticks != ra.start_ticks)
    );
}

#[test]
fn a_stale_identity_is_never_signalled() {
    let mut child = Owned::start("stale");
    settled(&child);
    let now = processes::read_proc(child.pid()).unwrap();
    let report = processes::stop_report(
        child.pid(),
        now.start_ticks + 1,
        Signal::Kill,
        Duration::from_millis(200),
    );
    println!("stale: {}", serde_json::to_string(&report).unwrap());
    assert_eq!(report.outcome, Outcome::Stale);
    assert!(!report.signal_sent && report.affected.is_none());
    std::thread::sleep(Duration::from_millis(100));
    assert!(child.running(), "the child survives a stale stop");
}

#[test]
fn the_system_manager_is_not_permitted_and_not_signalled() {
    let init = processes::read_proc(1).expect("pid 1 is readable");
    let report = processes::stop_report(
        1,
        init.start_ticks,
        Signal::Term,
        Duration::from_millis(100),
    );
    println!("init: {}", serde_json::to_string(&report).unwrap());
    assert_eq!(report.outcome, Outcome::NotPermitted);
    assert!(!report.signal_sent);
    assert_eq!(
        processes::stop(1, init.start_ticks, Signal::Term, Duration::ZERO),
        Outcome::NotPermitted
    );
}

#[test]
fn the_command_seam_lists_and_refuses_bad_requests() {
    let reply = processes::command("process-list", &serde_json::json!({})).unwrap();
    assert_eq!(
        reply["count"].as_u64(),
        reply["processes"].as_array().map(|p| p.len() as u64)
    );
    let bad = processes::command("process-stop", &serde_json::json!({"pid": 5}));
    assert!(matches!(
        bad,
        Err(processes::ProcessError::MissingStartTicks)
    ));
    let bad = processes::command(
        "process-stop",
        &serde_json::json!({"pid": 5, "start_ticks": 1, "signal": "hup"}),
    );
    assert!(matches!(bad, Err(processes::ProcessError::BadSignal(_))));
}

/// Item 3: one list on this machine, its time and how many reasons are known.
#[test]
fn one_list_is_measured() {
    let started = Instant::now();
    let list = processes::list();
    let took = started.elapsed();
    let known = list.iter().filter(|p| p.reason.known).count();
    println!(
        "MEASURE list_ms={} known={known} total={}",
        took.as_millis(),
        list.len()
    );
    for p in list.iter().filter(|p| !p.reason.known).take(5) {
        println!("UNKNOWN {} {}: {}", p.pid, p.name, p.reason.words);
    }
    for p in list
        .iter()
        .filter(|p| p.reason.known && !p.reason.words.starts_with("Part of"))
        .take(12)
    {
        println!("KNOWN {} {}: {}", p.pid, p.name, p.reason.words);
    }
    assert!(
        list.iter()
            .all(|p| !p.name.is_empty() && !p.reason.words.is_empty())
    );
    assert!(
        list.iter()
            .filter(|p| !p.reason.known)
            .all(|p| p.reason.words.starts_with("Reason unknown"))
    );
}
