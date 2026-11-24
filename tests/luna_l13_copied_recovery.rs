use serde_json::json;
use soliton::recovery::RecoveryStore;
use soliton_kernel::ContentHash;
use std::{fs, path::PathBuf};

#[test]
fn copied_real_source_survives_preview_conflict_and_hash_gated_restore() {
    let data_root = PathBuf::from(std::env::var("SOLITON_DATA").expect("SOLITON_DATA"));
    let source = data_root.join("history/capture-sources.json");
    let source_bytes = fs::read(&source).expect("lane copied source must exist");
    assert!(
        !source_bytes.is_empty(),
        "copied source must contain actual bytes"
    );

    let root = std::env::temp_dir().join(format!(
        "soliton-l13-copied-recovery-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let target = root.join("capture-sources.json");
    fs::copy(&source, &target).unwrap();

    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let checkpoint = store
        .command("recovery-checkpoint", &json!({"path": target}))
        .unwrap();
    let id = checkpoint["id"].as_str().unwrap().to_owned();
    let original_hash = ContentHash::digest(&source_bytes).to_string();
    assert_eq!(checkpoint["hash"], original_hash);

    let preview = store
        .command("recovery-preview", &json!({"id": id}))
        .unwrap();
    assert_eq!(preview["conflict"], false);
    assert_eq!(preview["current_hash"], original_hash);

    let mut newer = source_bytes.clone();
    newer.extend_from_slice(b"\n{\"l13_lane_edit\":true}\n");
    fs::write(&target, &newer).unwrap();
    let conflict = store
        .command("recovery-preview", &json!({"id": id}))
        .unwrap();
    let newer_hash = ContentHash::digest(&newer).to_string();
    assert_eq!(conflict["conflict"], true);
    assert_eq!(conflict["current_hash"], newer_hash);
    assert!(
        store
            .command(
                "recovery-restore",
                &json!({"id": id, "expected_current_hash": original_hash}),
            )
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), newer);

    drop(store);
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let receipt = store
        .command(
            "recovery-restore",
            &json!({"id": id, "expected_current_hash": newer_hash}),
        )
        .unwrap();
    assert_eq!(receipt["status"], "applied");
    assert_eq!(fs::read(&target).unwrap(), source_bytes);
    assert_eq!(receipt["path"], target.to_string_lossy().as_ref());
    println!(
        "copied_source={} source_hash={} target={} receipt_status={}",
        source.display(),
        original_hash,
        target.display(),
        receipt["status"]
    );
    fs::remove_dir_all(root).unwrap();
}
