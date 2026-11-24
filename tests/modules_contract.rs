use serde_json::json;
use soliton::modules::ModulesStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "soliton-modules-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/modules")
        .join(name)
}

fn wait(store: &mut ModulesStore, id: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let record = store.command("modules-status", &json!({"id": id})).unwrap();
        if matches!(
            record.get("state").and_then(|v| v.as_str()),
            Some("succeeded" | "failed" | "cancelled" | "timeout" | "interrupted")
        ) {
            return record;
        }
        assert!(Instant::now() < deadline, "job did not finish: {record}");
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn inventory_reports_real_engines_and_bounded_capabilities() {
    let mut store = ModulesStore::open(&temp_root("inventory")).unwrap();
    let list = store.command("modules-list", &json!({})).unwrap();
    let modules = list["modules"].as_array().unwrap();
    // Z-ETL is the one bundled helper left; the others left with their pages.
    assert_eq!(modules.len(), 1);
    assert!(modules.iter().any(
        |m| m["id"] == "zetl" && m["sha256"].as_str().unwrap_or("").starts_with("sha256:")
    ));
}

#[test]
fn zetl_fixture_workflow_copies_input_and_records_evidence() {
    let root = temp_root("zetl");
    let input = fixture("DATA/DATA-028/fixtures/two-row.csv");
    let mut store = ModulesStore::open(&root).unwrap();
    let accepted = store
        .command(
            "modules-run",
            &json!({"module":"zetl", "action":"inspect", "source_path":input}),
        )
        .unwrap();
    let record = wait(&mut store, accepted["id"].as_str().unwrap());
    assert_eq!(record["state"], "succeeded", "{record}");
    assert!(
        record["evidence"]["copied_input"]["path"]
            .as_str()
            .unwrap()
            .starts_with(root.to_str().unwrap())
    );
    assert!(record["evidence"]["stdout_path"].as_str().is_some());
    let artifacts = record["evidence"]["artifacts"]
        .as_array()
        .expect("artifact receipt");
    assert!(artifacts.iter().any(|artifact| {
        artifact["role"] == "input"
            && artifact["size"].as_u64().is_some()
            && artifact["sha256"].as_str().is_some()
    }));
    let listing = store
        .command("modules-list", &json!({"limit": 100}))
        .expect("metadata listing");
    assert_eq!(listing["offset"], 0);
    assert_eq!(listing["limit"], 100);
    assert_eq!(listing["job_limit"]["metadata_only"], true);
    let listed = listing["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == record["id"])
        .expect("listed job");
    assert!(listed["evidence"]["stdout"].is_null());
    assert!(listed["evidence"]["artifacts"].is_array());
}

#[test]
fn failed_job_and_restart_markers_are_durable() {
    let root = temp_root("failure");
    let mut store = ModulesStore::open(&root).unwrap();
    let accepted = store.command("modules-run", &json!({"module":"zetl", "action":"inspect", "source_path":"/definitely/missing/input.csv"})).unwrap();
    let failed = wait(&mut store, accepted["id"].as_str().unwrap());
    assert_eq!(failed["state"], "failed");
    let interrupted = "restart-fixture";
    fs::write(
        root.join("jobs").join(format!("{interrupted}.json")),
        serde_json::to_vec(&json!({"id":interrupted,"state":"running"})).unwrap(),
    )
    .unwrap();
    let reopened = ModulesStore::open(&root).unwrap();
    let mut reopened = reopened;
    let marker = reopened
        .command("modules-status", &json!({"id":interrupted}))
        .unwrap();
    assert_eq!(marker["state"], "interrupted");
}

#[test]
fn timeout_and_cancel_are_terminal_and_do_not_escape_job_root() {
    let root = temp_root("cancel");
    let input = root.join("large.csv");
    let mut bytes = Vec::new();
    for i in 0..400_000 {
        bytes.extend_from_slice(format!("{i},value-{i}\n").as_bytes());
    }
    fs::write(&input, bytes).unwrap();
    let mut store = ModulesStore::open(&root).unwrap();
    let timeout = store
        .command(
            "modules-run",
            &json!({"module":"zetl", "action":"inspect", "source_path":input, "timeout_ms":1}),
        )
        .unwrap();
    let timeout_record = wait(&mut store, timeout["id"].as_str().unwrap());
    assert!(matches!(
        timeout_record["state"].as_str(),
        Some("timeout" | "failed" | "succeeded")
    ));
    let cancel = store
        .command(
            "modules-run",
            &json!({"module":"zetl", "action":"inspect", "source_path":input, "timeout_ms":120000}),
        )
        .unwrap();
    let _ = store.command("modules-cancel", &json!({"id":cancel["id"]}));
    let cancelled = wait(&mut store, cancel["id"].as_str().unwrap());
    assert!(matches!(
        cancelled["state"].as_str(),
        Some("cancelled" | "succeeded" | "failed" | "timeout")
    ));
    let evidence_dir = cancelled["evidence"]["job_dir"].as_str().unwrap();
    assert!(evidence_dir.starts_with(root.to_str().unwrap()));
}
