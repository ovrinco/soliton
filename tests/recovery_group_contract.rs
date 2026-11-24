use serde_json::json;
use soliton::recovery::RecoveryStore;
use soliton_kernel::ObjectId;
use std::fs;
fn setup() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    let dir = std::env::temp_dir().join(ObjectId::mint("group-test").unwrap().to_string());
    fs::create_dir_all(&dir).unwrap();
    let paths = vec![dir.join("a.txt"), dir.join("b.txt")];
    for p in &paths {
        fs::write(p, b"original").unwrap();
    }
    (dir, paths)
}
#[test]
fn reviewed_group_detects_later_edit_before_any_restore_and_retries_once() {
    let (dir, paths) = setup();
    let mut store = RecoveryStore::open(&dir.join("archive")).unwrap();
    let g = store
        .command(
            "recovery-group-checkpoint",
            &json!({"paths":paths,"title":"Before changes"}),
        )
        .unwrap();
    for p in &paths {
        fs::write(p, b"second version").unwrap();
    }
    let preview = store
        .command("recovery-group-preview", &json!({"id":g["id"]}))
        .unwrap();
    fs::write(&paths[1], b"third version").unwrap();
    let args =
        json!({"id":g["id"],"expected_revision":preview["revision"],"op_id":preview["op_id"]});
    assert!(store.command("recovery-group-restore", &args).is_err());
    assert_eq!(fs::read(&paths[0]).unwrap(), b"second version");
    let p = store
        .command("recovery-group-preview", &json!({"id":g["id"]}))
        .unwrap();
    let args = json!({"id":g["id"],"expected_revision":p["revision"],"op_id":p["op_id"]});
    let receipt = store.command("recovery-group-restore", &args).unwrap();
    assert_eq!(receipt["status"], "applied");
    drop(store);
    let mut store = RecoveryStore::open(&dir.join("archive")).unwrap();
    assert_eq!(
        store.command("recovery-group-restore", &args).unwrap(),
        receipt
    );
    for p in &paths {
        assert_eq!(fs::read(p).unwrap(), b"original");
    }
    assert_eq!(
        store.command("recovery-list", &json!({})).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn interrupted_group_reconciles_first_file_and_protects_later_edit() {
    let (dir, paths) = setup();
    let archive = dir.join("archive");
    let mut store = RecoveryStore::open(&archive).unwrap();
    let g = store
        .command("recovery-group-checkpoint", &json!({"paths":paths}))
        .unwrap();
    for p in &paths {
        fs::write(p, b"changed").unwrap();
    }
    let preview = store
        .command("recovery-group-preview", &json!({"id":g["id"]}))
        .unwrap();
    let op = preview["op_id"].as_str().unwrap();
    let entries = preview["entries"].as_array().unwrap();
    let first=store.command("recovery-restore",&json!({"id":entries[0]["id"],"op_id":format!("{op}-0"),"expected_current_hash":entries[0]["current_hash"]})).unwrap();
    fs::create_dir_all(archive.join("group-intents")).unwrap();
    let plan = json!({"schema":1,"id":g["id"],"op_id":op,"expected_revision":preview["revision"],"status":"applying","entries":entries,"outcomes":[]});
    fs::write(
        archive.join("group-intents").join(format!("{op}.json")),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    fs::write(&paths[1], b"later independent edit").unwrap();
    drop(store);
    let mut store = RecoveryStore::open(&archive).unwrap();
    let r = store
        .command(
            "recovery-group-restore",
            &json!({"id":g["id"],"expected_revision":preview["revision"],"op_id":op}),
        )
        .unwrap();
    assert_eq!(r["status"], "partial");
    assert_eq!(r["outcomes"][0], first);
    assert_eq!(fs::read(&paths[1]).unwrap(), b"later independent edit");
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn no_restore_receipt_is_invented_without_an_admitted_intent() {
    let (dir, paths) = setup();
    let mut store = RecoveryStore::open(&dir.join("archive")).unwrap();
    let c = store
        .command("recovery-checkpoint", &json!({"path":paths[0]}))
        .unwrap();
    let result = store.command(
        "recovery-restore",
        &json!({"id":c["id"],"expected_current_hash":"sha256:unrelated","op_id":"unadmitted"}),
    );
    assert!(result.is_err());
    fs::remove_dir_all(dir).unwrap();
}
