use serde_json::json;
use soliton::history::HistoryStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("soliton-history-{label}-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn empty_search_and_provider_filter_return_no_rows_without_error() {
    let root = temp_root("empty-search");
    let mut store = HistoryStore::open(&root).unwrap();
    for args in [
        json!({"q":"no match"}),
        json!({"provider":"youtube"}),
        json!({"provider":"' OR 1=1 --"}),
    ] {
        let result = store.command("history-list", &args).unwrap();
        assert_eq!(result["items"], json!([]));
        assert_eq!(result["total"], 0);
    }
}

#[test]
fn lifecycle_is_recoverable_revision_checked_and_survives_reimport() {
    let root = temp_root("lifecycle");
    let source = voxtype_fixture(&root);
    let archive = root.join("archive");
    let original = fs::read(source.join("transcript.json")).unwrap();
    let mut store = HistoryStore::open(&archive).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"voxtype"}),
        )
        .unwrap();
    let id = imported["record"]["id"].clone();
    let mut revision = imported["record"]["revision"].clone();
    assert!(
        store
            .command(
                "history-state",
                &json!({"id":id,"state":"deleted","expected_revision":revision})
            )
            .is_err()
    );
    for state in ["archived", "deleted", "active"] {
        let changed = store
            .command(
                "history-state",
                &json!({"id":id,"state":state,"confirmed":true,"expected_revision":revision}),
            )
            .unwrap();
        assert_eq!(changed["record_state"], state);
        assert!(
            store
                .command(
                    "history-state",
                    &json!({"id":id,"state":"active","expected_revision":revision})
                )
                .is_err()
        );
        revision = changed["revision"].clone();
        for q in ["", "first"] {
            let visible = store
                .command("history-list", &json!({"q":q,"state":state}))
                .unwrap();
            assert_eq!(visible["total"], 1);
            if state != "active" {
                assert_eq!(
                    store.command("history-list", &json!({"q":q})).unwrap()["total"],
                    0
                );
            }
        }
    }
    store
        .command(
            "history-state",
            &json!({"id":id,"state":"deleted","confirmed":true,"expected_revision":revision}),
        )
        .unwrap();
    drop(store);
    let mut store = HistoryStore::open(&archive).unwrap();
    assert_eq!(
        store
            .command("history-list", &json!({"state":"deleted","q":"first"}))
            .unwrap()["total"],
        1
    );
    let mut updated: serde_json::Value = serde_json::from_slice(&original).unwrap();
    updated["segments"][0]["text"] = json!("first revised source");
    fs::write(
        source.join("transcript.json"),
        serde_json::to_vec(&updated).unwrap(),
    )
    .unwrap();
    let reimported = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"voxtype"}),
        )
        .unwrap();
    assert_eq!(reimported["record"]["record_state"], "deleted");
    assert_eq!(
        store.command("history-list", &json!({})).unwrap()["total"],
        0
    );
    let revisions = store
        .command("history-revisions", &json!({"id":id}))
        .unwrap();
    assert!(revisions["items"].as_array().unwrap().len() >= 6);
    assert!(source.join("transcript.json").is_file());
}

fn voxtype_fixture(root: &Path) -> PathBuf {
    let source = root.join("recording");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("metadata.json"),
        serde_json::to_vec(&json!({
            "id": "voice-1",
            "title": "A real note",
            "started_at": "2026-09-21T20:40:52Z",
            "duration_secs": 90,
            "status": "completed",
            "chunk_count": 212,
            "audio_retained": false
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        source.join("transcript.json"),
        serde_json::to_vec(&json!({
            "segments": [
                {"id": 0, "start_ms": 0, "end_ms": 30000, "text": "first", "speaker_id": "You", "chunk_id": 0},
                {"id": 0, "start_ms": 30000, "end_ms": 60000, "text": "second", "speaker_id": "You", "chunk_id": 2},
                {"id": 0, "start_ms": 60000, "end_ms": 90000, "text": "third", "speaker_id": "You", "chunk_id": 4}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    source
}

#[test]
fn voxtype_import_preserves_duplicate_raw_ids_and_is_idempotent() {
    let root = temp_root("duplicate-id");
    let source = voxtype_fixture(&root);
    let archive = root.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let first = store
        .command(
            "history-import",
            &json!({"path": source, "provider": "voxtype"}),
        )
        .unwrap();
    let record = &first["record"];
    assert_eq!(record["kind"], "voice_note");
    assert_eq!(record["audio"]["state"], "unavailable");
    assert_eq!(record["segments"].as_array().unwrap().len(), 3);
    assert_ne!(record["segments"][0]["id"], record["segments"][1]["id"]);
    assert_eq!(record["segments"][0]["raw_id"], 0);
    let second = store
        .command(
            "history-import",
            &json!({"path": root.join("recording"), "provider": "voxtype"}),
        )
        .unwrap();
    assert_eq!(second["unchanged"], true);
    assert_eq!(second["record"]["revision"], record["revision"]);
    assert!(archive.join("raw").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn correction_survives_restart_and_retains_original_revision() {
    let root = temp_root("correction");
    let source = voxtype_fixture(&root);
    let archive = root.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path": source, "provider": "voxtype"}),
        )
        .unwrap()["record"]
        .clone();
    let corrected = store
        .command(
            "history-correct",
            &json!({
                "id": imported["id"],
                "expected_revision": imported["revision"],
                "segment_id": imported["segments"][0]["id"],
                "text": "corrected",
                "author": "ian",
                "reason": "name spelling"
            }),
        )
        .unwrap();
    assert_eq!(corrected["revision"], "2");
    drop(store);
    fs::remove_file(
        archive
            .join("records")
            .join(corrected["id"].as_str().unwrap())
            .join("record.json"),
    )
    .unwrap();
    let mut store = HistoryStore::open(&archive).unwrap();
    let current = store
        .command("history-detail", &json!({"id": corrected["id"]}))
        .unwrap();
    assert_eq!(current["segments"][0]["text"], "corrected");
    let original = store
        .command(
            "history-detail",
            &json!({"id": corrected["id"], "revision": "1"}),
        )
        .unwrap();
    assert_eq!(original["segments"][0]["text"], "first");
    assert_eq!(
        store
            .command("history-revisions", &json!({"id": corrected["id"]}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_source_fails_and_partial_codex_tail_is_visible() {
    let root = temp_root("malformed-partial");
    let archive = root.join("archive");
    let malformed = root.join("bad.txt");
    fs::write(&malformed, [0xff, 0xfe]).unwrap();
    let mut store = HistoryStore::open(&archive).unwrap();
    assert!(
        store
            .command("history-import", &json!({"path": malformed}))
            .is_err()
    );
    let codex = root.join("session.jsonl");
    fs::write(
        &codex,
        "{\"session_id\":\"s-1\",\"role\":\"user\",\"content\":\"hello\"}\n{\"role\":\"assistant\",\"content\":\"unfinished",
    )
    .unwrap();
    let result = store
        .command(
            "history-import",
            &json!({"path": codex, "provider": "codex"}),
        )
        .unwrap();
    assert_eq!(result["record"]["coverage"]["state"], "partial");
    assert_eq!(result["record"]["segments"].as_array().unwrap().len(), 1);
    let console = root.join("console.jsonl");
    fs::write(
        &console,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"console-1\",\"timestamp\":\"2026-09-21T00:00:00Z\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<keep-leading\"}]}}\n",
    )
    .unwrap();
    let console_result = store
        .command(
            "history-import",
            &json!({"path": console, "provider": "console"}),
        )
        .unwrap();
    assert_eq!(console_result["record"]["provider"], "console");
    assert_eq!(
        console_result["record"]["segments"][0]["text"],
        "<keep-leading"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_capture_directory_recurses_and_watermarks_each_file() {
    let root = temp_root("codex-directory");
    let archive = root.join("archive");
    let sessions = root.join("sessions");
    fs::create_dir_all(sessions.join("nested")).unwrap();
    for (path, id) in [
        (sessions.join("one.jsonl"), "one"),
        (sessions.join("nested/two.jsonl"), "two"),
    ] {
        fs::write(
            path,
            format!("{{\"session_id\":\"{id}\",\"role\":\"user\",\"content\":\"hello {id}\"}}\n"),
        )
        .unwrap();
    }
    let mut store = HistoryStore::open(&archive).unwrap();
    let capture = store
        .command(
            "history-capture-add",
            &json!({"provider":"codex","path":sessions}),
        )
        .unwrap();
    let first = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    assert_eq!(first["items"][0]["imported"], 2);
    let second = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    assert_eq!(second["items"][0]["imported"], 0);
    assert_eq!(second["items"][0]["unchanged"], 2);
    fs::write(
        sessions.join("nested/two.jsonl"),
        "{\"session_id\":\"two\",\"role\":\"user\",\"content\":\"changed\"}\n",
    )
    .unwrap();
    let third = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    assert_eq!(third["items"][0]["imported"], 1);
    assert_eq!(third["items"][0]["unchanged"], 1);
    // The capture list states what is switched on, when a check last ran and
    // what it found; it never states a sync.
    let listed = store.command("history-capture-list", &json!({})).unwrap();
    let source = &listed["sources"][0];
    assert_eq!(source["enabled"], true);
    assert!(source["last_checked_at"].is_string());
    assert_eq!(
        source["last_result"],
        json!({"imported":1,"unchanged":1,"failed":0})
    );
    assert!(source.get("last_sync").is_none());
    assert!(!listed.to_string().contains("sync"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_sqlite_import_archives_consistent_visible_messages() {
    let root = temp_root("opencode");
    let db = root.join("opencode.db");
    let sql = r#"
      PRAGMA journal_mode=WAL;
      CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, title TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL);
      CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
      CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
      INSERT INTO session VALUES ('ses-test', NULL, 'Fixture', 1700000000000, 1700000001000);
      INSERT INTO message VALUES ('msg-user', 'ses-test', 1700000000000, '{"role":"user"}');
      INSERT INTO message VALUES ('msg-assistant', 'ses-test', 1700000001000, '{"role":"assistant"}');
      INSERT INTO part VALUES ('part-user', 'msg-user', 'ses-test', 1700000000000, '{"type":"text","text":"<visible leading marker"}');
      INSERT INTO part VALUES ('part-reasoning', 'msg-assistant', 'ses-test', 1700000001000, '{"type":"reasoning","text":"hidden reasoning"}');
      INSERT INTO part VALUES ('part-assistant', 'msg-assistant', 'ses-test', 1700000001000, '{"type":"text","text":"answer"}');
    "#;
    let created = Command::new("sqlite3").arg(&db).arg(sql).status().unwrap();
    assert!(created.success());
    let archive = root.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path": db, "provider": "opencode"}),
        )
        .unwrap()["record"]
        .clone();
    assert_eq!(imported["provider"], "opencode");
    assert_eq!(imported["segments"][0]["text"], "<visible leading marker");
    assert_eq!(imported["segments"].as_array().unwrap().len(), 2);
    assert_eq!(imported["coverage"]["state"], "partial");
    let id = imported["id"].as_str().unwrap().to_owned();
    fs::remove_file(&db).unwrap();
    drop(store);
    let mut store = HistoryStore::open(&archive).unwrap();
    let reopened = store.command("history-detail", &json!({"id": id})).unwrap();
    assert_eq!(reopened["segments"][0]["text"], "<visible leading marker");
    fs::remove_dir_all(root).unwrap();
}
