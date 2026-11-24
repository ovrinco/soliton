use serde_json::json;
use soliton::recovery::RecoveryStore;
use soliton_kernel::ObjectId;
use std::fs;

fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(ObjectId::mint("recovery-contract").unwrap().to_string());
    fs::create_dir(&root).unwrap();
    let file = root.join("note.txt");
    fs::write(&file, b"before\n").unwrap();
    (root, file)
}

#[test]
fn checkpoint_preview_conflict_and_restore_are_hash_gated() {
    let (root, file) = fixture();
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let checkpoint = store
        .command("recovery-checkpoint", &json!({"path":file}))
        .unwrap();
    let id = checkpoint["id"].as_str().unwrap().to_owned();
    let original_hash = checkpoint["hash"].as_str().unwrap().to_owned();
    let preview = store
        .command("recovery-preview", &json!({"id":id}))
        .unwrap();
    assert_eq!(preview["conflict"], false);
    assert_eq!(preview["current_hash"], original_hash);
    assert_eq!(preview["diff"]["changed"], false);

    fs::write(&file, b"newer\n").unwrap();
    let changed = store
        .command("recovery-preview", &json!({"id":id}))
        .unwrap();
    assert_eq!(changed["conflict"], true);
    let expected_newer = soliton_kernel::ContentHash::digest(b"newer\n").to_string();
    assert!(
        store
            .command(
                "recovery-restore",
                &json!({"id":id,"expected_current_hash":original_hash}),
            )
            .is_err()
    );
    assert_eq!(fs::read(&file).unwrap(), b"newer\n");

    drop(store);
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let receipt = store
        .command(
            "recovery-restore",
            &json!({"id":id,"expected_current_hash":expected_newer}),
        )
        .unwrap();
    assert_eq!(receipt["status"], "applied");
    assert_eq!(fs::read(&file).unwrap(), b"before\n");
    let list = store
        .command("recovery-list", &json!({"path":file}))
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_and_symlink_targets_are_explicit_states_or_errors() {
    let (root, file) = fixture();
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let checkpoint = store
        .command("recovery-checkpoint", &json!({"path":file}))
        .unwrap();
    let id = checkpoint["id"].as_str().unwrap();
    fs::remove_file(&file).unwrap();
    let preview = store
        .command("recovery-preview", &json!({"id":id}))
        .unwrap();
    assert_eq!(preview["current_state"], "missing");
    assert_eq!(preview["missing"], true);
    store
        .command(
            "recovery-restore",
            &json!({"id":id,"expected_current_hash":"missing"}),
        )
        .unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"before\n");

    let target = root.join("target.txt");
    fs::write(&target, b"target").unwrap();
    let link = root.join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        store
            .command("recovery-checkpoint", &json!({"path":link}))
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retry_reconciles_replaced_file_without_second_checkpoint() {
    let (root, file) = fixture();
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let checkpoint = store
        .command("recovery-checkpoint", &json!({"path":file}))
        .unwrap();
    let id = checkpoint["id"].as_str().unwrap();
    fs::write(&file, b"newer\n").unwrap();
    let before_checkpoint = store
        .command("recovery-checkpoint", &json!({"path":file}))
        .unwrap();
    let preview = store
        .command("recovery-preview", &json!({"id":id}))
        .unwrap();
    // Simulate the durable intent written after the before-restore checkpoint,
    // followed by a process crash after atomic replacement but before receipt.
    let op_id = preview["op_id"].as_str().unwrap();
    fs::write(
        root.join("archive")
            .join("intents")
            .join(format!("{op_id}.json")),
        serde_json::to_vec(&json!({
            "schema":1,
            "op_id":op_id,
            "status":"prepared",
            "checkpoint_id":id,
            "path":file,
            "expected_current_hash":preview["current_hash"],
            "before_checkpoint_id":before_checkpoint["id"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(&file, b"before\n").unwrap();
    drop(store);
    let mut store = RecoveryStore::open(&root.join("archive")).unwrap();
    let receipt = store
        .command(
            "recovery-restore",
            &json!({
                "id":id,
                "op_id":preview["op_id"],
                "expected_current_hash":preview["current_hash"]
            }),
        )
        .unwrap();
    assert_eq!(receipt["reconciled"], true);
    assert_eq!(fs::read(&file).unwrap(), b"before\n");
    assert_eq!(
        store
            .command("recovery-list", &json!({"path":file}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::remove_dir_all(root).unwrap();
}
