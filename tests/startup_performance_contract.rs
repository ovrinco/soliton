use serde_json::json;
use soliton::modules::ModulesStore;
use soliton::service::Service;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soliton-startup-{label}-{stamp}"));
    fs::create_dir_all(&root).expect("temporary root");
    root
}

#[test]
fn opening_modules_store_does_not_start_qream_by_default() {
    // An explicit operator opt-in is tested by the ignored runtime contract;
    // this assertion covers the normal launcher environment.
    if std::env::var("SOLITON_QREAM_AUTOSTART").is_ok() {
        return;
    }
    let root = temp_root("modules");
    let store = ModulesStore::open(&root).expect("module store");
    drop(store);
    assert!(
        !root.join("qream").exists(),
        "opening the module panel must not create or start the QREAM runtime"
    );
    fs::remove_dir_all(root).expect("temporary cleanup");
}

#[test]
fn background_tick_is_idle_without_scheduler_opt_in() {
    if std::env::var("SOLITON_COLLECTION_AUTOSTART").is_ok() {
        return;
    }
    let root = temp_root("service");
    let workspace = root.join("workspace");
    let mut service = Service::open(&workspace).expect("service");
    service.background_tick().expect("idle tick");
    assert!(
        !workspace.join("collection").exists(),
        "opening Soliton must not create a collection worker"
    );
    drop(service);
    fs::remove_dir_all(root).expect("temporary cleanup");
}

#[test]
fn background_tick_does_not_requeue_a_persisted_briefing() {
    let root = temp_root("briefing");
    let workspace = root.join("workspace");
    let jobs = workspace.join("briefings/jobs");
    fs::create_dir_all(&jobs).expect("briefing jobs");
    let path = jobs.join("old-job.json");
    let persisted = json!({
        "job_id": "old-job",
        "state": "running",
        "queue_token": "old-token",
        "source": {"id": "source", "revision": "revision"},
        "coverage": {"state": "running"}
    });
    fs::write(&path, serde_json::to_vec(&persisted).expect("json")).expect("job");
    let mut service = Service::open(&workspace).expect("service");
    service.background_tick().expect("idle tick");
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("persisted job")).expect("json");
    assert_eq!(after, persisted, "startup must not resume local model work");
    drop(service);
    fs::remove_dir_all(root).expect("temporary cleanup");
}

fn capture_workspace(label: &str) -> (PathBuf, PathBuf) {
    let root = temp_root(label);
    let workspace = root.join("workspace");
    let history = workspace.join("history");
    fs::create_dir_all(&history).expect("history folder");
    fs::write(
        history.join("capture-sources.json"),
        serde_json::to_vec(&json!({"schema":"soliton-history-capture/1","sources":[]}))
            .expect("json"),
    )
    .expect("registered capture sources");
    (root, workspace)
}

fn history_entries(workspace: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(workspace.join("history"))
        .expect("history folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn automatic_capture_off_starts_no_capture_past_the_old_sync_time() {
    let (root, workspace) = capture_workspace("capture-off");
    let mut service = Service::open(&workspace).expect("service");
    assert!(service.preferences().expect("preferences")["automaticCapture"].is_null());
    let before = history_entries(&workspace);
    // The capture clock is moved past the old 60 s sync time instead of waiting.
    service.advance_capture_clock(std::time::Duration::from_secs(120));
    for _ in 0..2 {
        service.background_tick().expect("idle tick");
        std::thread::sleep(std::time::Duration::from_millis(2_100));
        service.advance_capture_clock(std::time::Duration::from_secs(120));
    }
    assert_eq!(
        history_entries(&workspace),
        before,
        "with automatic capture off no history job may start or write"
    );
    drop(service);
    fs::remove_dir_all(root).expect("temporary cleanup");
}

#[test]
fn automatic_capture_on_starts_the_capture_job() {
    if std::env::var("SOLITON_AUTO_CAPTURE").as_deref() == Ok("0") {
        return;
    }
    let (root, workspace) = capture_workspace("capture-on");
    let mut service = Service::open(&workspace).expect("service");
    service
        .command("preferences", &json!({"automaticCapture": true}))
        .expect("turn automatic capture on");
    service.advance_capture_clock(std::time::Duration::from_secs(120));
    service.background_tick().expect("tick");
    let mut opened = false;
    for _ in 0..50 {
        if workspace.join("history/records").is_dir() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        opened,
        "with automatic capture on the capture sync must start"
    );
    drop(service);
    fs::remove_dir_all(root).expect("temporary cleanup");
}
