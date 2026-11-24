use serde_json::json;
use soliton::history::HistoryStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn test_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soliton-l06-rotation-{stamp}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn copy_actual_source(source: &Path, destination: &Path) {
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(source, destination).unwrap();
}

#[test]
fn rotated_actual_codex_session_reuses_origin_identity() {
    // The source is an immutable copy of a retained Codex rollout from the
    // lane's populated workspace corpus. The test only relocates that copy;
    // it does not invent conversation content or touch the retained source.
    let actual = PathBuf::from(std::env::var("L06_ACTUAL_CODEX_SOURCE").unwrap());
    assert!(
        actual.is_file(),
        "actual copied Codex source missing: {actual:?}"
    );
    let root = test_root();
    let first_path = root.join("rotated-a/session.jsonl");
    let second_path = root.join("rotated-b/session.jsonl");
    copy_actual_source(&actual, &first_path);
    copy_actual_source(&actual, &second_path);

    let archive = root.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let first = store
        .command(
            "history-import",
            &json!({"path": first_path, "provider": "codex"}),
        )
        .unwrap();
    let first_id = first["record"]["id"].as_str().unwrap().to_owned();
    let first_origin = first["record"]["source_refs"][0]["origin_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let rotated = store
        .command(
            "history-import",
            &json!({"path": second_path, "provider": "codex"}),
        )
        .unwrap();
    assert_eq!(rotated["unchanged"], true);
    assert_eq!(rotated["record"]["id"], first_id);
    assert_eq!(
        rotated["record"]["source_refs"][0]["origin_id"],
        first_origin
    );

    drop(store);
    let mut reopened = HistoryStore::open(&archive).unwrap();
    let list = reopened
        .command("history-list", &json!({"kind": "chat", "limit": 100}))
        .unwrap();
    assert_eq!(list["total"], 1);
    let detail = reopened
        .command("history-detail", &json!({"id": first_id}))
        .unwrap();
    assert!(!detail["segments"].as_array().unwrap().is_empty());
    assert_eq!(detail["revision"], "1");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_voxtype_segments_and_long_audio_reopen_independently() {
    let transcript_only = PathBuf::from(std::env::var("L06_ACTUAL_VOXTYPE_TRANSCRIPT").unwrap());
    let long_recording = PathBuf::from(std::env::var("L06_ACTUAL_VOXTYPE_LONG").unwrap());
    assert!(
        transcript_only.is_dir(),
        "actual copied transcript-only recording missing: {transcript_only:?}"
    );
    assert!(
        long_recording.is_dir(),
        "actual copied long recording missing: {long_recording:?}"
    );

    let root = test_root();
    let archive = root.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let transcript = store
        .command(
            "history-import",
            &json!({"path": transcript_only, "provider": "voxtype"}),
        )
        .unwrap();
    assert_eq!(transcript["record"]["coverage"]["state"], "complete");
    assert_eq!(transcript["record"]["audio"]["state"], "unavailable");
    let segments = transcript["record"]["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 106);
    let mut ids = std::collections::BTreeSet::new();
    for (index, segment) in segments.iter().enumerate() {
        assert_eq!(segment["index"], index);
        assert!(ids.insert(segment["id"].as_str().unwrap().to_owned()));
        assert!(segment["start_ms"].is_number());
        assert!(segment["end_ms"].is_number());
    }
    assert_eq!(segments.iter().filter(|s| s["raw_id"] == 0).count(), 106);

    let long = store
        .command(
            "history-import",
            &json!({"path": long_recording, "provider": "voxtype"}),
        )
        .unwrap();
    assert_eq!(long["record"]["audio"]["state"], "retained");
    assert_eq!(long["record"]["segments"].as_array().unwrap().len(), 93);
    let audio_path = PathBuf::from(long["record"]["audio"]["path"].as_str().unwrap());
    assert_eq!(fs::metadata(audio_path).unwrap().len(), 530_966_478);
    let repeated = store
        .command(
            "history-import",
            &json!({"path": long_recording, "provider": "voxtype"}),
        )
        .unwrap();
    assert_eq!(repeated["unchanged"], true);

    let transcript_id = transcript["record"]["id"].as_str().unwrap().to_owned();
    let long_id = long["record"]["id"].as_str().unwrap().to_owned();
    drop(store);
    let mut reopened = HistoryStore::open(&archive).unwrap();
    let transcript_detail = reopened
        .command("history-detail", &json!({"id": transcript_id}))
        .unwrap();
    let long_detail = reopened
        .command("history-detail", &json!({"id": long_id}))
        .unwrap();
    assert_eq!(transcript_detail["segments"].as_array().unwrap().len(), 106);
    assert_eq!(transcript_detail["audio"]["state"], "unavailable");
    assert_eq!(long_detail["segments"].as_array().unwrap().len(), 93);
    assert_eq!(long_detail["audio"]["state"], "retained");
    fs::remove_dir_all(root).unwrap();
}
