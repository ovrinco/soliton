use serde_json::json;
use soliton::modules::ModulesStore;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soliton-module-workspace-{label}-{stamp}"));
    fs::create_dir_all(&root).expect("temporary module root");
    root
}

#[test]
fn inventory_exposes_only_real_module_actions_and_deep_zetl_verify() {
    let mut store = ModulesStore::open(&temp_root("inventory")).expect("store");
    let inventory = store
        .command("modules-list", &json!({}))
        .expect("inventory");
    let zetl = inventory["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|module| module["id"] == "zetl")
        .expect("zetl descriptor");
    let actions = zetl["actions"].as_array().expect("zetl actions");
    assert!(actions.iter().any(|action| action == "inspect"));
    assert!(actions.iter().any(|action| action == "validate"));
    assert!(actions.iter().any(|action| action == "run"));
    assert!(actions.iter().any(|action| action == "verify"));
    assert!(actions.iter().any(|action| action == "resume"));
    assert_eq!(inventory["job_limit"]["active"], 4);
    assert_eq!(inventory["job_limit"]["queued"], 64);
    let outside = temp_root("outside").join("manifest.json");
    fs::write(&outside, "{}").expect("outside manifest");
    let error = store
        .command(
            "modules-run",
            &json!({"module":"zetl", "action":"verify", "manifest_path":outside}),
        )
        .expect("verify job is accepted before worker validation");
    let id = error["id"].as_str().expect("verify id");
    for _ in 0..200 {
        let record = store
            .command("modules-status", &json!({"id": id}))
            .expect("status");
        if record["state"] == "failed" {
            assert!(
                record["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("owned module job")
            );
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("verify worker did not reject external manifest");
}
