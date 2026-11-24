//! The tracker store read in-process (T46): rows equal to `tracker get`, every tab placed once, a change through
//! the `tracker` command reaching the engine's state event within a second (every one of 50), and an idle engine
//! never opening the store. Every case works on a private copy of the store (`TRACKER_CONTRACT_SOURCE`, else the
//! live store, copied with SQLite's read-only backup); the copies stay in the build's scratch folder.
use serde_json::{Value, json};
use soliton::tracker::Tracker;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}
fn tracker_bin() -> PathBuf {
    home().join(".local/bin/tracker")
}
fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("T46-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("scratch folder");
    dir
}
/// A private copy of the store.
struct Copy(PathBuf);
impl Copy {
    fn new(dir: &Path) -> Self {
        let source = std::env::var_os("TRACKER_CONTRACT_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("06_memory/tracker/live/store/tracker.sqlite"));
        let copy = dir.join("tracker.sqlite");
        let status = Command::new("sqlite3")
            .arg("-readonly")
            .arg(&source)
            .arg(format!(".backup '{}'", copy.display()))
            .status()
            .expect("sqlite3 runs");
        assert!(status.success() && copy.is_file(), "store copied");
        Self(copy)
    }
}
fn cli(store: &Path, args: &[&str]) -> String {
    let out = Command::new(tracker_bin())
        .arg("--db")
        .arg(store)
        .args(args)
        .output()
        .expect("tracker command runs");
    assert!(
        out.status.success(),
        "tracker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn ten_rows_equal_tracker_get() {
    let dir = scratch("rows");
    let copy = Copy::new(&dir);
    let mut tracker = Tracker::with_store(copy.0.clone());
    let mut ids = Vec::new();
    // Tabs whose first column is a unique key (Outcomes' first block keys on its charter, shared by its lines).
    for (tab, n) in [("Soliton", 5), ("Decisions", 3), ("Faults", 2)] {
        let page = tracker
            .command("tracker-rows", &json!({"tab": tab, "limit": n}))
            .expect("rows read");
        assert!(page["convention"]["entries"].is_array() || page["convention"].is_null());
        let key = |r: &Value| -> String {
            let table = r["_table"].as_str().unwrap_or("");
            let section = page["sections"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["table"] == table))
                .expect("row's section");
            let first = section["columns"][0].as_str().expect("key column");
            r[first].as_str().expect("key text").to_string()
        };
        for r in page["rows"].as_array().expect("rows") {
            ids.push((key(r), r.clone()));
        }
    }
    assert_eq!(ids.len(), 10, "ten rows read");
    let mut differences = 0;
    for (id, listed) in &ids {
        let ours = tracker
            .command("tracker-row", &json!({"id": id}))
            .expect("row read")["row"]
            .clone();
        let theirs: Value = serde_json::from_str(&cli(&copy.0, &["get", id])).expect("get JSON");
        if ours != theirs {
            differences += 1;
            eprintln!("{id}: tracker-row differs from tracker get");
        }
        // The listed row holds the store's cells as `get` prints them.
        for (k, v) in listed.as_object().expect("object") {
            if k != "sheet" && theirs.get(k).is_some_and(|t| t != v) {
                differences += 1;
                eprintln!("{id}.{k}: tracker-rows differs from tracker get");
            }
        }
    }
    println!(
        "ten-row comparison: {differences} differences over {} rows",
        ids.len()
    );
    assert_eq!(differences, 0);
}

#[test]
fn every_tab_appears_once_learning_first() {
    let dir = scratch("tabs");
    let copy = Copy::new(&dir);
    let mut tracker = Tracker::with_store(copy.0.clone());
    let reply = tracker
        .command("tracker-tabs", &json!({}))
        .expect("tabs read");
    let categories = reply["categories"].as_array().expect("categories");
    assert_eq!(categories[0]["name"], "Learning");
    let mut seen: Vec<String> = Vec::new();
    for c in categories {
        for t in c["tabs"].as_array().expect("tabs") {
            seen.push(t["name"].as_str().expect("name").into());
        }
    }
    let count: Value = serde_json::from_str(&cli(
        &copy.0,
        &[
            "query",
            "SELECT count(*) AS n FROM sheet",
            "--format",
            "json",
        ],
    ))
    .expect("query JSON");
    let n = count[0]["n"].as_u64().expect("count") as usize;
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    println!(
        "tabs: {} listed, {} distinct, {} in the store; categories {:?}",
        seen.len(),
        unique.len(),
        n,
        categories
            .iter()
            .map(|c| c["name"].clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(seen.len(), n);
    assert_eq!(unique.len(), n);
    // A filter, an order and the column check work on the store's own columns.
    let page = tracker
        .command(
            "tracker-rows",
            &json!({"tab":"Soliton","filter":{"column":"status","text":"said"},"order":{"column":"id","direction":"desc"},"limit":3}),
        )
        .expect("filtered rows");
    let rows = page["rows"].as_array().expect("rows");
    assert!(
        rows.iter()
            .all(|r| r["status"].as_str().unwrap_or("").contains("said"))
    );
    assert!(
        rows.windows(2)
            .all(|w| w[0]["id"].as_str() >= w[1]["id"].as_str())
    );
    assert!(
        tracker
            .command(
                "tracker-rows",
                &json!({"tab":"Soliton","order":{"column":"no such"}})
            )
            .is_err()
    );
}

struct Engine {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<(Instant, Value)>,
}
impl Engine {
    fn start(workspace: &Path, store: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_soliton"))
            .arg(workspace)
            .env("SOLITON_TRACKER_STORE", store)
            .env("SOLITON_AUTO_CAPTURE", "0")
            .env_remove("SOLITON_COLLECTION_AUTOSTART")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("engine starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str(&line)
                    && tx.send((Instant::now(), v)).is_err()
                {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }
    fn ask(&mut self, id: u64, command: &str, args: Value) -> Value {
        writeln!(
            self.stdin,
            "{}",
            json!({"id": id, "command": command, "args": args})
        )
        .expect("send");
        loop {
            let (_, v) = self
                .lines
                .recv_timeout(Duration::from_secs(20))
                .expect("reply");
            if v["id"] == id {
                assert_eq!(v["ok"], true, "{command}: {v}");
                return v["result"].clone();
            }
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({"id": 0, "command": "shutdown"}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const WRITES: usize = 50;

#[test]
fn every_cli_change_reaches_the_state_event_within_a_second() {
    let dir = scratch("event");
    let copy = Copy::new(&dir);
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace folder");
    let mut engine = Engine::start(&workspace, &copy.0);
    // The window asks for state first, as it does at start (the engine's identity is read once, then).
    engine.ask(0, "state", json!({}));
    let found = engine.ask(
        1,
        "tracker-rows",
        json!({"tab":"Soliton","filter":{"column":"status","text":"implemented"},"limit":1}),
    );
    let id = found["rows"][0]["id"]
        .as_str()
        .expect("an implemented row")
        .to_string();
    let watching = engine.ask(2, "tracker-watch", json!({"on": true}));
    let mut before = watching["revision"].as_u64().expect("revision");
    let mut waits = Vec::new();
    for i in 0..WRITES {
        // Writes land at every phase of the engine's tick: a different pause before each.
        std::thread::sleep(Duration::from_millis((i as u64 * 97) % 700));
        let status = if i % 2 == 0 { "said" } else { "implemented" };
        cli(
            &copy.0,
            &[
                "--author",
                "t46-contract",
                "set",
                &id,
                &format!("STATUS={status}"),
            ],
        );
        let written = Instant::now();
        let revision = loop {
            let Ok((at, v)) = engine.lines.recv_timeout(Duration::from_secs(5)) else {
                break None;
            };
            if v["event"] == "state"
                && let Some(r) = v["state"]["trackerRevision"].as_u64()
                && r > before
            {
                break Some((at, r));
            }
        };
        let wait = revision.map(|(at, _)| at.saturating_duration_since(written));
        println!(
            "write {:2} status {status:11} revision {before} -> {}: {}",
            i + 1,
            revision.map_or("none".into(), |(_, r)| r.to_string()),
            wait.map_or("no event in 5 s".into(), |w| format!(
                "{} ms",
                w.as_millis()
            ))
        );
        if let Some((_, r)) = revision {
            before = r;
        }
        waits.push(wait);
    }
    let within = waits
        .iter()
        .filter(|w| w.is_some_and(|w| w < Duration::from_secs(1)))
        .count();
    let longest = waits.iter().flatten().max().copied().unwrap_or_default();
    println!(
        "{within} of {WRITES} writes reached the state event within 1 s; longest {} ms",
        longest.as_millis()
    );
    assert_eq!(within, WRITES);
    let row = engine.ask(3, "tracker-row", json!({"id": id}));
    assert_eq!(row["row"]["status"], "implemented");
    engine.ask(4, "tracker-watch", json!({"on": false}));
}

/// Opens of the store copy seen by inotify while `act` runs against an engine on it.
fn opens_during(label: &str, act: impl FnOnce(&mut Engine)) -> usize {
    let dir = scratch(label);
    let copy = Copy::new(&dir);
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace folder");
    let mut watcher = Command::new("inotifywait")
        .args(["-m", "-e", "open", "--format", "%e %w"])
        .arg(&copy.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("inotifywait runs");
    let mut ready = String::new();
    let mut err = BufReader::new(watcher.stderr.take().expect("stderr"));
    while !ready.contains("Watches established") {
        ready.clear();
        assert!(err.read_line(&mut ready).expect("inotifywait speaks") > 0);
    }
    let out = watcher.stdout.take().expect("stdout");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    {
        let mut engine = Engine::start(&workspace, &copy.0);
        engine.ask(1, "state", json!({}));
        act(&mut engine);
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = watcher.kill();
    let _ = watcher.wait();
    rx.try_iter().filter(|l| l.contains("OPEN")).count()
}

#[test]
fn an_engine_with_no_sheet_view_never_opens_the_store() {
    let idle = opens_during("idle", |_| std::thread::sleep(Duration::from_secs(3)));
    let watching_unchanged = opens_during("watch", |e| {
        e.ask(2, "tracker-watch", json!({"on": true}));
        let after_watch = Instant::now();
        std::thread::sleep(Duration::from_secs(3));
        assert!(after_watch.elapsed() >= Duration::from_secs(3));
    });
    let read = opens_during("read", |e| {
        e.ask(2, "tracker-revision", json!({}));
    });
    println!(
        "store opens: idle 3 s {idle}; a watch on an unchanged store, 3 s {watching_unchanged} (its one opening read included); one read {read}"
    );
    assert_eq!(idle, 0);
    assert!(read >= 1, "the method sees an open");
    assert_eq!(
        watching_unchanged, read,
        "a watch on an unchanged store reads once, at its start"
    );
}
