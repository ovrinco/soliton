use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soliton::inbox::InboxStore;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("soliton-inbox-{label}-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn op_id(suffix: char) -> String {
    format!("00000000-0000-4000-8000-00000000000{suffix}")
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn begin(store: &mut InboxStore, id: &str, bytes: &[u8], intent: &str) -> Value {
    store
        .command(
            "inbox-begin",
            &json!({
                "op_id": id,
                "filename": "voice note.wav",
                "size": bytes.len(),
                "sha256": hash(bytes),
                "intent": intent
            }),
        )
        .unwrap()
}

fn chunk(store: &mut InboxStore, id: &str, offset: usize, bytes: &[u8]) -> Value {
    store
        .command(
            "inbox-chunk",
            &json!({
                "id": id,
                "offset": offset,
                "hex": bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "sha256": hash(bytes)
            }),
        )
        .unwrap()
}

fn complete(store: &mut InboxStore, id: &str, bytes: &[u8]) -> Value {
    chunk(store, id, 0, bytes);
    store.command("inbox-commit", &json!({"id": id})).unwrap()
}

fn fixture(store: &mut InboxStore, id: &str) -> (Vec<u8>, Value) {
    let bytes = b"original phone bytes\n".to_vec();
    let started = begin(store, id, &bytes, "voice_note");
    (bytes, started)
}

#[test]
fn destinations_and_path_traversal_are_rejected() {
    let root = temp_root("path");
    let mut store = InboxStore::open(&root).unwrap();
    let destinations = store.command("inbox-destinations", &json!({})).unwrap();
    assert_eq!(destinations["destinations"][0]["id"], "inbox");
    assert_eq!(destinations["destinations"][0]["writable"], true);

    let args = json!({
        "op_id": op_id('1'),
        "filename": "../escape.wav",
        "size": 1,
        "sha256": hash(b"x"),
        "intent": "store"
    });
    assert!(store.command("inbox-begin", &args).is_err());
    assert!(!root.join("escape.wav").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn offset_accepts_byte_identical_replay_and_rejects_gaps_or_changes() {
    let root = temp_root("offset");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('2');
    let bytes = b"abcdefgh";
    begin(&mut store, &id, bytes, "store");
    assert_eq!(chunk(&mut store, &id, 0, b"abcd")["offset"], 4);
    let replay = chunk(&mut store, &id, 0, b"abcd");
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["offset"], 4);
    assert!(
        store
            .command(
                "inbox-chunk",
                &json!({"id":id,"offset":0,"hex":"61626365","sha256":hash(b"abce")}),
            )
            .is_err()
    );
    assert!(
        store
            .command(
                "inbox-chunk",
                &json!({"id":id,"offset":6,"hex":"6768","sha256":hash(b"gh")}),
            )
            .is_err()
    );
    assert_eq!(chunk(&mut store, &id, 4, b"ef")["offset"], 6);
    assert_eq!(chunk(&mut store, &id, 6, b"gh")["offset"], 8);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_operation_payload_is_rejected() {
    let root = temp_root("reuse");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('3');
    let bytes = b"same operation";
    begin(&mut store, &id, bytes, "store");
    let changed = json!({
        "op_id": id,
        "filename": "voice note.wav",
        "size": bytes.len(),
        "sha256": hash(bytes),
        "intent": "voice_note"
    });
    assert!(store.command("inbox-begin", &changed).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn full_file_hash_failure_keeps_recoverable_staging() {
    let root = temp_root("hash-failure");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('4');
    let bytes = b"verified content";
    store
        .command(
            "inbox-begin",
            &json!({"op_id":id,"filename":"x.bin","size":bytes.len(),"sha256":hash(b"different"),"intent":"store"}),
        )
        .unwrap();
    chunk(&mut store, &id, 0, bytes);
    assert!(store.command("inbox-commit", &json!({"id": id})).is_err());
    let status = store.command("inbox-status", &json!({"id": id})).unwrap();
    assert_eq!(status["state"], "receiving");
    assert_eq!(status["offset"], bytes.len());
    assert!(!root.join("receipts").join(format!("{id}.json")).exists());
    assert!(
        fs::read_dir(root.join("artifacts"))
            .unwrap()
            .next()
            .is_none()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn full_file_commit_returns_receipt_and_survives_restart() {
    let root = temp_root("restart");
    let id = op_id('5');
    let (bytes, _) = {
        let mut store = InboxStore::open(&root).unwrap();
        fixture(&mut store, &id)
    };
    let mut store = InboxStore::open(&root).unwrap();
    let receipt = complete(&mut store, &id, &bytes);
    assert_eq!(receipt["id"], id);
    assert_eq!(receipt["size"], bytes.len());
    assert_eq!(receipt["source_intent"], "voice_note");
    let path = PathBuf::from(receipt["path"].as_str().unwrap());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    drop(store);
    let mut reopened = InboxStore::open(&root).unwrap();
    let again = reopened
        .command("inbox-status", &json!({"id": id}))
        .unwrap();
    assert_eq!(again["state"], "committed");
    assert_eq!(again["receipt"]["receipt_id"], receipt["receipt_id"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn lost_commit_response_is_idempotent() {
    let root = temp_root("lost-response");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('6');
    let (bytes, _) = fixture(&mut store, &id);
    let first = complete(&mut store, &id, &bytes);
    let second = store.command("inbox-commit", &json!({"id": id})).unwrap();
    assert_eq!(first["receipt_id"], second["receipt_id"]);
    assert_eq!(first["artifact_id"], second["artifact_id"]);
    assert_eq!(
        fs::read(PathBuf::from(first["path"].as_str().unwrap())).unwrap(),
        bytes
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restart_reconciles_artifact_published_before_receipt() {
    let root = temp_root("reconcile");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('7');
    let (bytes, started) = fixture(&mut store, &id);
    chunk(&mut store, &id, 0, &bytes);
    drop(store);

    let metadata_path = root.join("staging").join(&id).join("metadata.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    metadata["state"] = json!("committing");
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    let payload = root.join("staging").join(&id).join("payload.part");
    let artifact = root.join("artifacts").join(format!("artifact-{id}.wav"));
    fs::hard_link(&payload, &artifact).unwrap();

    let mut reopened = InboxStore::open(&root).unwrap();
    let status = reopened
        .command("inbox-status", &json!({"id": id}))
        .unwrap();
    assert_eq!(status["state"], "committed");
    assert_eq!(
        status["receipt"]["path"].as_str().unwrap(),
        artifact.to_str().unwrap()
    );
    assert_eq!(fs::read(&artifact).unwrap(), bytes);
    assert_eq!(started["id"], id);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancel_after_commit_cannot_remove_original() {
    let root = temp_root("cancel-committed");
    let mut store = InboxStore::open(&root).unwrap();
    let id = op_id('8');
    let (bytes, _) = fixture(&mut store, &id);
    let receipt = complete(&mut store, &id, &bytes);
    assert!(store.command("inbox-cancel", &json!({"id": id})).is_err());
    let path = Path::new(receipt["path"].as_str().unwrap());
    assert!(path.is_file());
    assert_eq!(fs::read(path).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}
