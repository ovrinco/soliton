//! Every item shows its sheet row (T25, trimmed into T54): the row a record names reads the same as `tracker get`
//! prints it, and a record tagged `row:<ID>` carries the row's id while an untagged one carries none. The store
//! case works on a private copy (`TRACKER_CONTRACT_SOURCE`, else the live store, copied with SQLite's read-only
//! backup) under the build's scratch folder; nothing is deleted (the lot's rule), the copies stay there.
use serde_json::{Value, json};
use soliton::service::Service;
use soliton::tracker::Tracker;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}
fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("T54-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("scratch folder");
    dir
}
/// A private copy of the store, left in the scratch folder.
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
#[test]
fn three_rows_equal_tracker_get() {
    let dir = scratch("rows");
    let copy = Copy::new(&dir);
    let mut tracker = Tracker::with_store(copy.0.clone());
    let fields = ["id", "status", "purpose", "dod", "outcome_id"];
    let mut differences = 0;
    for id in ["T003-081", "T973-001", "T1018-009"] {
        let ours = tracker
            .command("tracker-row", &json!({"id": id}))
            .expect("row read")["row"]
            .clone();
        let out = Command::new(home().join(".local/bin/tracker"))
            .arg("--db")
            .arg(&copy.0)
            .args(["get", id])
            .output()
            .expect("tracker command runs");
        assert!(out.status.success(), "tracker get {id}");
        let theirs: Value = serde_json::from_slice(&out.stdout).expect("get JSON");
        for f in fields {
            if ours[f] != theirs[f] {
                differences += 1;
                eprintln!("{id}.{f}: {} != {}", ours[f], theirs[f]);
            }
        }
        assert_eq!(ours["id"], json!(id), "the row asked for");
    }
    eprintln!("three-row comparison: {differences} differences over 3 rows x 5 fields");
    assert_eq!(differences, 0);
}

#[test]
fn a_record_tagged_with_a_row_carries_its_id() {
    let root = scratch("record");
    let mut service = Service::open(&root).expect("workspace");
    let linked = service
        .command(
            "save",
            &json!({"kind": "task", "title": "Privacy toggle", "tags": ["row:T003-081", "privacy"]}),
        )
        .expect("linked task saved");
    let plain = service
        .command(
            "save",
            &json!({"kind": "task", "title": "Plain task", "tags": ["row:"]}),
        )
        .expect("plain task saved");
    let state = service.command("state", &json!({})).expect("state");
    let find = |id: &Value| {
        state["records"]
            .as_array()
            .expect("records")
            .iter()
            .find(|r| &r["id"] == id)
            .cloned()
            .expect("record listed")
    };
    eprintln!(
        "linked row_id {}; plain row_id {}",
        find(&linked["id"])["row_id"],
        find(&plain["id"])["row_id"]
    );
    assert_eq!(find(&linked["id"])["row_id"], json!("T003-081"));
    assert!(find(&plain["id"]).get("row_id").is_none());
}
