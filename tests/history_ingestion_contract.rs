use serde_json::json;
use soliton::history::HistoryStore;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("soliton-history-ingestion-{label}-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn voxtype_recording(root: &Path, id: &str, audio_bytes: u64) -> PathBuf {
    let recording = root.join(id);
    fs::create_dir_all(&recording).unwrap();
    fs::write(
        recording.join("metadata.json"),
        serde_json::to_vec(&json!({
            "id": id,
            "title": format!("{id} note"),
            "started_at": "2026-09-23T20:49:00Z",
            "duration_secs": 90,
            "status": "completed",
            "audio_retained": true
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        recording.join("transcript.json"),
        br#"{"segments":[{"id":0,"start_ms":0,"end_ms":90000,"text":"A bounded audio fixture."}]}"#,
    )
    .unwrap();
    let audio = File::create(recording.join("audio.wav")).unwrap();
    audio.set_len(audio_bytes).unwrap();
    recording
}

#[test]
fn observed_large_voxtype_audio_sizes_stream_and_retain() {
    let dir = root("large-audio");
    let archive = dir.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    for (index, size) in [244_560_078_u64, 194_822_478, 530_966_478]
        .into_iter()
        .enumerate()
    {
        let source = voxtype_recording(&dir, &format!("recording-{index}"), size);
        let imported = store
            .command(
                "history-import",
                &json!({"path": source, "provider": "voxtype"}),
            )
            .unwrap()["record"]
            .clone();
        assert_eq!(imported["audio"]["state"], "retained");
        let archived = PathBuf::from(imported["audio"]["path"].as_str().unwrap());
        assert_eq!(fs::metadata(archived).unwrap().len(), size);
        let repeated = store
            .command(
                "history-import",
                &json!({"path": dir.join(format!("recording-{index}")), "provider": "voxtype"}),
            )
            .unwrap();
        assert_eq!(repeated["unchanged"], true);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oversized_audio_does_not_drop_valid_transcript() {
    let dir = root("oversized-audio");
    let source = voxtype_recording(&dir, "oversized", 1_073_741_825);
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path": source, "provider": "voxtype"}),
        )
        .unwrap()["record"]
        .clone();
    assert_eq!(imported["segments"][0]["text"], "A bounded audio fixture.");
    assert_eq!(imported["audio"]["state"], "unavailable");
    assert!(
        imported["audio"]["reason"]
            .as_str()
            .unwrap()
            .contains("bounded")
    );
    let repeated = store
        .command(
            "history-import",
            &json!({"path": dir.join("oversized"), "provider": "voxtype"}),
        )
        .unwrap();
    assert_eq!(repeated["unchanged"], true);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn antigravity_schema_keeps_visible_steps_and_excludes_reasoning() {
    let dir = root("antigravity");
    let source = dir
        .join("brain")
        .join("conversation-visible-1")
        .join(".system_generated")
        .join("logs");
    fs::create_dir_all(&source).unwrap();
    let path = source.join("transcript.jsonl");
    let lines = [
        json!({"step_index":1,"type":"USER_INPUT","created_at":"2026-09-24T08:00:00-04:00","content":"<USER_REQUEST>Keep the visible request</USER_REQUEST><ADDITIONAL_METADATA>private runtime details</ADDITIONAL_METADATA>"}),
        json!({"step_index":2,"type":"PLANNER_RESPONSE","created_at":"2026-09-24T08:00:01-04:00","content":"Visible answer","thinking":"private chain of thought","tool_calls":[{"name":"secret_tool"}]}),
        json!({"step_index":3,"type":"GENERIC","created_at":"2026-09-24T08:00:02-04:00","content":"tool output\npassword=do-not-retain"}),
        json!({"step_index":4,"type":"CHECKPOINT","content":"hidden prior context"}),
    ];
    let mut file = File::create(&path).unwrap();
    for line in lines {
        writeln!(file, "{}", serde_json::to_string(&line).unwrap()).unwrap();
    }
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path": path, "provider": "antigravity"}),
        )
        .unwrap()["record"]
        .clone();
    assert_eq!(imported["provider"], "antigravity");
    assert!(
        imported["source_refs"][0]["origin_id"]
            .as_str()
            .unwrap()
            .ends_with("\0conversation-visible-1")
    );
    assert_eq!(imported["segments"].as_array().unwrap().len(), 3);
    assert_eq!(imported["segments"][0]["speaker"], "user");
    assert_eq!(imported["segments"][1]["speaker"], "assistant");
    assert_eq!(imported["segments"][2]["speaker"], "tool");
    assert_eq!(imported["segments"][0]["raw_id"], 1);
    assert_eq!(imported["segments"][1]["raw_id"], 2);
    assert!(
        imported["segments"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Keep the visible request")
    );
    assert!(
        !imported["segments"][0]["text"]
            .as_str()
            .unwrap()
            .contains("private runtime")
    );
    assert!(
        !imported["segments"][2]["text"]
            .as_str()
            .unwrap()
            .contains("password=do-not-retain")
    );
    assert_eq!(imported["coverage"]["state"], "partial");
    let repeated = store
        .command(
            "history-import",
            &json!({"path": dir.join("brain/conversation-visible-1/.system_generated/logs/transcript.jsonl"), "provider": "antigravity"}),
        )
        .unwrap();
    assert_eq!(repeated["unchanged"], true);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_rollout_schema_uses_response_messages_once() {
    let dir = root("codex-rollout");
    let path = dir.join("rollout.jsonl");
    let lines = [
        json!({"type":"session_meta","payload":{"id":"rollout-visible-1","timestamp":"2026-09-24T08:00:00Z"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","id":"msg-user","content":[{"type":"input_text","text":"Visible request"}]}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"event-echo","content":[{"type":"text","text":"Visible request"}]}}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"private tool payload"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","id":"msg-assistant","content":[{"type":"output_text","text":"Visible answer"}]}}),
    ];
    let mut file = File::create(&path).unwrap();
    for line in lines {
        writeln!(file, "{}", serde_json::to_string(&line).unwrap()).unwrap();
    }
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let imported = store
        .command(
            "history-import",
            &json!({"path": path, "provider": "codex"}),
        )
        .unwrap()["record"]
        .clone();
    assert!(
        imported["source_refs"][0]["origin_id"]
            .as_str()
            .unwrap()
            .ends_with("\0rollout-visible-1")
    );
    assert_eq!(imported["segments"].as_array().unwrap().len(), 2);
    assert_eq!(imported["segments"][0]["raw_id"], "msg-user");
    assert_eq!(imported["segments"][1]["raw_id"], "msg-assistant");
    assert!(
        !serde_json::to_string(&imported)
            .unwrap()
            .contains("private tool payload")
    );
    let repeated = store
        .command(
            "history-import",
            &json!({"path": dir.join("rollout.jsonl"), "provider": "codex"}),
        )
        .unwrap();
    assert_eq!(repeated["unchanged"], true);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn capture_sync_reports_nested_failures_as_partial() {
    let dir = root("partial-capture");
    let source = dir.join("codex");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("valid.jsonl"),
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"valid\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"visible\"}}\n",
    )
    .unwrap();
    fs::write(
        source.join("broken.jsonl"),
        "{\"type\":\"message\"}\n{broken\n",
    )
    .unwrap();
    let archive = dir.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    store
        .command(
            "history-capture-add",
            &json!({"path": source, "provider": "codex"}),
        )
        .unwrap();
    let synced = store.command("history-capture-sync", &json!({})).unwrap();
    assert_eq!(synced["items"][0]["status"], "partial");
    assert_eq!(synced["items"][0]["ok"], false);
    assert_eq!(synced["items"][0]["errors"].as_array().unwrap().len(), 1);
    assert_eq!(synced["coverage"]["sources"][0]["last_result"]["failed"], 1);
    assert!(
        synced["coverage"]["sources"][0]["last_error"]
            .as_str()
            .is_some()
    );
    assert_eq!(
        store
            .command("history-list", &json!({"kind":"chat"}))
            .unwrap()["total"],
        1
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn voxtype_capture_watermarks_skip_unchanged_audio_and_retry_changes() {
    let dir = root("voxtype-capture-watermark");
    let recordings = dir.join("meetings");
    let recording = voxtype_recording(&recordings, "recording-1", 8 * 1024 * 1024);
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let capture = store
        .command(
            "history-capture-add",
            &json!({"path":recordings,"provider":"voxtype"}),
        )
        .unwrap();

    let first_started = Instant::now();
    let first = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    let first_elapsed = first_started.elapsed();
    assert_eq!(first["items"][0]["imported"], 1);

    // An unchanged recording must be recognized from file metadata alone, so
    // the large attachment is not opened again by this sync.
    let second_started = Instant::now();
    let second = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    let second_elapsed = second_started.elapsed();
    assert_eq!(second["items"][0]["imported"], 0);
    assert_eq!(second["items"][0]["unchanged"], 1);
    eprintln!(
        "voxtype capture watermark: first={first_elapsed:?} second={second_elapsed:?} audio=8MiB"
    );

    fs::write(
        recording.join("transcript.json"),
        br#"{"segments":[{"id":0,"start_ms":0,"end_ms":90000,"text":"Changed transcript."}]}"#,
    )
    .unwrap();
    let third = store
        .command("history-capture-sync", &json!({"id":capture["id"]}))
        .unwrap();
    assert_eq!(third["items"][0]["imported"], 1);
    assert_eq!(third["items"][0]["errors"].as_array().unwrap().len(), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_bootstrap_is_idempotent_and_preserves_disabled_providers() {
    let dir = root("bootstrap");
    let home = dir.join("home");
    let codex = home.join("codex-sessions");
    let antigravity = home.join("antigravity-brain");
    let voxtype = home.join("voxtype-meetings");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&antigravity).unwrap();
    fs::create_dir_all(&voxtype).unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let first = store
        .command(
            "history-capture-bootstrap",
            &json!({
                "codex_path": codex,
                "antigravity_path": antigravity,
                "voxtype_path": voxtype
            }),
        )
        .unwrap();
    assert_eq!(first["added"].as_array().unwrap().len(), 3);
    let second = store
        .command("history-capture-bootstrap", &json!({"home": home}))
        .unwrap();
    assert_eq!(second["added"].as_array().unwrap().len(), 0);
    assert_eq!(second["sources"].as_array().unwrap().len(), 3);
    let codex_id = first["added"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "codex")
        .unwrap()["id"]
        .clone();
    store
        .command("history-capture-remove", &json!({"id": codex_id}))
        .unwrap();
    let third = store
        .command("history-capture-bootstrap", &json!({"home": home}))
        .unwrap();
    assert_eq!(third["added"].as_array().unwrap().len(), 0);
    assert_eq!(
        third["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["provider"] == "codex")
            .unwrap()["enabled"],
        false
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn antigravity_directory_prunes_non_transcripts_before_candidate_bound() {
    let dir = root("antigravity-prune");
    let brain = dir.join("brain");
    let logs = brain
        .join("conversation-1")
        .join(".system_generated")
        .join("logs");
    fs::create_dir_all(&logs).unwrap();
    for index in 0..2_100 {
        fs::write(brain.join(format!("artifact-{index}.json")), b"{}").unwrap();
    }
    fs::write(
        logs.join("transcript.jsonl"),
        format!(
            "{}\n",
            serde_json::to_string(&json!({
                "step_index": 1,
                "type": "USER_INPUT",
                "content": "<USER_REQUEST>visible</USER_REQUEST>"
            }))
            .unwrap()
        ),
    )
    .unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let capture = store
        .command(
            "history-capture-add",
            &json!({"path": brain, "provider": "antigravity"}),
        )
        .unwrap();
    let synced = store
        .command("history-capture-sync", &json!({"id": capture["id"]}))
        .unwrap();
    assert_eq!(synced["items"][0]["status"], "complete");
    assert_eq!(synced["items"][0]["imported"], 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn antigravity_ignores_unrelated_deep_artifact_trees() {
    let dir = root("antigravity-deep-artifacts");
    let brain = dir.join("brain");
    let logs = brain
        .join("conversation-1")
        .join(".system_generated")
        .join("logs");
    fs::create_dir_all(&logs).unwrap();
    let deep_artifacts = brain.join("unrelated").join("a/b/c/d/e/f/g/h/i");
    fs::create_dir_all(&deep_artifacts).unwrap();
    fs::write(deep_artifacts.join("private.json"), b"{}").unwrap();
    fs::write(
        logs.join("transcript.jsonl"),
        format!(
            "{}\n",
            serde_json::to_string(&json!({
                "step_index": 1,
                "type": "USER_INPUT",
                "content": "<USER_REQUEST>visible transcript</USER_REQUEST>"
            }))
            .unwrap()
        ),
    )
    .unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let capture = store
        .command(
            "history-capture-add",
            &json!({"path": brain, "provider": "antigravity"}),
        )
        .unwrap();
    let synced = store
        .command("history-capture-sync", &json!({"id": capture["id"]}))
        .unwrap();
    assert_eq!(synced["items"][0]["status"], "complete");
    assert_eq!(synced["items"][0]["imported"], 1);
    fs::remove_dir_all(dir).unwrap();
}

fn wav(path: &Path, seconds: u32) {
    let rate: u32 = 16_000 * 2;
    let mut bytes = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16_000u32.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((rate * seconds).to_le_bytes());
    bytes.extend(vec![0u8; (rate * seconds) as usize]);
    fs::write(path, bytes).unwrap();
}

fn digest(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

/// A note saved by the recorder appears at once, read in place, with its real
/// length; opening it registers its own identity and words, never a copy.
#[test]
fn recorder_notes_are_listed_in_place_and_registered_on_opening() {
    let dir = root("recorder-lens");
    let meetings = dir.join("voxtype").join("meetings");
    let saved = voxtype_recording(&meetings, "saved-note", 0);
    wav(&saved.join("audio.wav"), 175);
    let older = voxtype_recording(&meetings, "older-note", 0);
    wav(&older.join("audio.wav"), 3);
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(older.join("metadata.json")).unwrap()).unwrap();
    metadata["started_at"] = json!("2026-09-01T08:00:00Z");
    metadata["duration_secs"] = json!(60);
    fs::write(older.join("metadata.json"), metadata.to_string()).unwrap();
    let recording = voxtype_recording(&meetings, "still-recording", 0);
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(recording.join("metadata.json")).unwrap()).unwrap();
    metadata["status"] = json!("active");
    fs::write(recording.join("metadata.json"), metadata.to_string()).unwrap();
    let files = ["audio.wav", "metadata.json", "transcript.json"];
    let before: Vec<String> = files.iter().map(|f| digest(&saved.join(f))).collect();

    let archive = dir.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    assert_eq!(
        store
            .command("history-list", &json!({"kind":"voice_note"}))
            .unwrap()["total"],
        0
    );
    store
        .command(
            "history-capture-add",
            &json!({"path":dir.join("voxtype"),"provider":"voxtype"}),
        )
        .unwrap();
    let listed = store
        .command("history-list", &json!({"kind":"voice_note"}))
        .unwrap();
    let items = listed["items"].as_array().unwrap();
    assert_eq!(listed["total"], 2, "{listed}");
    assert_eq!(items[0]["title"], "saved-note note");
    assert_eq!(items[0]["duration_ms"], 175_000);
    assert_eq!(items[1]["duration_ms"], 3_000);
    assert_eq!(items[0]["preview"], "A bounded audio fixture.");
    assert_eq!(fs::read_dir(archive.join("records")).unwrap().count(), 0);
    let all = store.command("history-list", &json!({})).unwrap();
    assert_eq!(all["total"], 2);

    let id = items[0]["id"].clone();
    let opened = store.command("history-detail", &json!({"id":id})).unwrap();
    assert_eq!(opened["id"], id);
    assert_eq!(opened["source_refs"][0]["origin_id"], "saved-note");
    assert_eq!(opened["segments"][0]["text"], "A bounded audio fixture.");
    assert_eq!(opened["duration_ms"], 175_000);
    assert_eq!(opened["audio"]["path"], json!(saved.join("audio.wav")));
    let raw = archive.join("raw").join(id.as_str().unwrap());
    let mut archived: Vec<String> = fs::read_dir(raw.join("1"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    archived.sort();
    assert_eq!(archived, ["metadata.json", "transcript.json"]);
    let after_open = store
        .command("history-list", &json!({"kind":"voice_note"}))
        .unwrap();
    assert_eq!(after_open["total"], 2);
    assert_eq!(after_open["items"][0]["id"], id);
    let after: Vec<String> = files.iter().map(|f| digest(&saved.join(f))).collect();
    assert_eq!(before, after);
    // A removed source is no longer read.
    let source =
        store.command("history-capture-list", &json!({})).unwrap()["sources"][0]["id"].clone();
    store
        .command("history-capture-remove", &json!({"id":source}))
        .unwrap();
    assert_eq!(
        store
            .command("history-list", &json!({"kind":"voice_note"}))
            .unwrap()["total"],
        1
    );
    println!(
        "RECORDER_LENS {}",
        json!({"listed":items.len(),"opened":id,"archived":archived,"hashes":before})
    );
}
