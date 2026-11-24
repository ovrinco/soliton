//! The built service answers the desktop, machine-log and process commands
//! over its line protocol, on a temp workspace with the recorded two-output
//! desktop (`SOLITON_DESKTOP_RECORD`) and the recorded machine-log day; a
//! scratch config folder stands in for the user's, and nothing is live.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

struct Engine {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
}

impl Engine {
    fn start(workspace: &Path, config: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_soliton"))
            .arg(workspace)
            .env(
                "SOLITON_DESKTOP_RECORD",
                fixtures().join("desktop/two-outputs-2026-10-28.json"),
            )
            .env("SOLITON_DESKTOP_DEFAULTS", config.join("no-defaults"))
            .env("SOLITON_SETTINGS_CONFIG", config)
            .env(
                "SOLITON_MACHINE_LOG_RECORD",
                fixtures().join("machine_log/2026-10-27"),
            )
            .env("SOLITON_AUTO_CAPTURE", "0")
            .env_remove("SOLITON_SETTINGS_LIVE")
            .env_remove("SOLITON_COLLECTION_AUTOSTART")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("engine starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    break;
                }
            }
        });
        Engine {
            child,
            stdin,
            lines,
        }
    }

    /// The reply to one request and how long it took to arrive.
    fn ask(&mut self, id: u64, command: &str, args: Value) -> (Value, Duration) {
        let started = Instant::now();
        writeln!(
            self.stdin,
            "{}",
            json!({"id": id, "command": command, "args": args})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        loop {
            let reply = self
                .lines
                .recv_timeout(Duration::from_secs(10))
                .expect("a reply");
            if reply["id"] == id {
                return (reply, started.elapsed());
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({"id": 0, "command": "shutdown"}));
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch(label: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("t78-protocol-{label}-{}", std::process::id()));
    let workspace = dir.join("workspace");
    let config = dir.join("hypr");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(config.join("no-defaults")).unwrap();
    fs::copy(
        fixtures().join("desktop_apply/two-outputs-2026-10-28/monitors.lua"),
        config.join("monitors.lua"),
    )
    .unwrap();
    fs::write(
        config.join("bindings.lua"),
        "o.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh your-server\")\no.bind(\"SUPER + SHIFT + Y\", \"Notes\", \"notes\")\n",
    )
    .unwrap();
    (workspace, config)
}

#[test]
fn the_five_reads_answer_with_their_fields_in_under_300_ms() {
    let (workspace, config) = scratch("reads");
    let mut engine = Engine::start(&workspace, &config);
    // The engine's own start is not what is timed: one cheap request first.
    assert_eq!(engine.ask(100, "tool-available", json!({})).0["ok"], true);
    let reads: [(&str, Value, &[&str]); 5] = [
        (
            "desktop-monitors",
            json!({}),
            &[
                "items",
                "allItems",
                "activeKey",
                "workspaces",
                "knownOutputs",
                "fallbacks",
                "revision",
                "liveApply",
            ],
        ),
        (
            "desktop-bindings",
            json!({}),
            &["items", "revision", "keyboard", "liveApply"],
        ),
        (
            "desktop-fallback-read",
            json!({}),
            &["fallbacks", "active", "asleep", "canRecord", "moves"],
        ),
        (
            "machine-log",
            json!({"first": "2026-10-27", "last": "2026-10-27", "zone": "America/Toronto"}),
            &["zone", "from", "to", "days", "sources"],
        ),
        (
            "process-list",
            json!({}),
            &["count", "reasons_unknown", "processes"],
        ),
    ];
    let mut slow = Vec::new();
    for (n, (command, args, fields)) in reads.into_iter().enumerate() {
        let (reply, took) = engine.ask(n as u64 + 1, command, args);
        println!("{command}: {} ms", took.as_millis());
        assert_eq!(reply["ok"], true, "{command}: {reply}");
        for field in fields {
            assert!(!reply["result"][*field].is_null(), "{command} has {field}");
        }
        if took >= Duration::from_millis(300) {
            slow.push(command);
        }
    }
    assert!(slow.is_empty(), "over 300 ms: {slow:?}");
}

#[test]
fn the_recorded_desktop_is_what_the_reads_report() {
    let (workspace, config) = scratch("recorded");
    let mut engine = Engine::start(&workspace, &config);
    let (monitors, _) = engine.ask(1, "desktop-monitors", json!({}));
    let names: Vec<&str> = monitors["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(
        monitors["result"]["workspaces"].as_array().unwrap().len(),
        4
    );
    let (log, _) = engine.ask(
        2,
        "machine-log",
        json!({"first": "2026-10-27", "last": "2026-10-27", "zone": "America/Toronto"}),
    );
    let lines = log["result"]["days"][0]["lines"].as_array().unwrap();
    assert!(
        !lines.is_empty() && lines.iter().all(|l| l["text"].is_string()),
        "each line carries its reading"
    );
    let (stop, _) = engine.ask(3, "process-stop", json!({}));
    assert_eq!(
        stop["ok"], false,
        "a stop with no process is refused: {stop}"
    );
}

#[test]
fn a_binding_is_changed_over_the_protocol_and_a_conflict_is_refused() {
    let (workspace, config) = scratch("binding");
    let mut engine = Engine::start(&workspace, &config);
    let (read, _) = engine.ask(1, "desktop-bindings", json!({}));
    let revision = read["result"]["revision"].clone();
    let (refused, _) = engine.ask(2, "desktop-binding-set", json!({"old": "SUPER + SHIFT + R", "description": "SSH", "new": "SUPER + SHIFT + Y", "revision": revision}));
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"].as_str().unwrap().contains("Notes"),
        "{refused}"
    );
    let (set, _) = engine.ask(3, "desktop-binding-set", json!({"old": "SUPER + SHIFT + R", "description": "SSH", "new": "SUPER + SHIFT + T", "revision": revision}));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(
        set["result"]["lines"][1],
        "o.bind(\"SUPER + SHIFT + T\", \"SSH\", \"alacritty -e ssh your-server\")"
    );
    let text = fs::read_to_string(config.join("bindings.lua")).unwrap();
    assert!(text.ends_with("\nhl.unbind(\"SUPER + SHIFT + R\")\no.bind(\"SUPER + SHIFT + T\", \"SSH\", \"alacritty -e ssh your-server\")\n"));
}
