use serde_json::{Value, json};
use soliton::{history::HistoryStore, service::Service};
use soliton_kernel::{ContentHash, ObjectId};
use std::{fs, path::PathBuf};
fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("retention-test").unwrap().to_string());
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn pending_snapshot_recovers_before_and_after_raw_publication() {
    for raw_published in [false, true] {
        let dir = root();
        let archive = dir.join("archive");
        let source = dir.join("note.md");
        fs::write(&source, "We need a reliable archive.").unwrap();
        let mut store = HistoryStore::open(&archive).unwrap();
        let r = store
            .command(
                "history-import",
                &json!({"path":source,"provider":"manual"}),
            )
            .unwrap()["record"]
            .clone();
        drop(store);
        let id = r["id"].as_str().unwrap();
        let rec = archive.join("records").join(id);
        let raw = archive.join("raw").join(id).join("1");
        let record: Value =
            serde_json::from_slice(&fs::read(rec.join("record.json")).unwrap()).unwrap();
        let entries = vec![(
            "note.md".to_owned(),
            ContentHash::digest(&fs::read(raw.join("note.md")).unwrap()).to_string(),
        )];
        let digest = ContentHash::digest(&serde_json::to_vec(&entries).unwrap()).to_string();
        fs::write(
            rec.join("pending.json"),
            serde_json::to_vec(
                &json!({"record":record,"stage":"recover-test","snapshot_hash":digest}),
            )
            .unwrap(),
        )
        .unwrap();
        fs::remove_file(rec.join("record.json")).unwrap();
        fs::remove_file(rec.join("revisions/1.json")).unwrap();
        fs::remove_file(&source).unwrap();
        if !raw_published {
            fs::rename(&raw, archive.join("staging/recover-test")).unwrap();
        }
        let mut store = HistoryStore::open(&archive).unwrap();
        let recovered = store.command("history-detail", &json!({"id":id})).unwrap();
        assert_eq!(recovered["segments"], r["segments"]);
        assert!(raw.join("note.md").is_file());
        assert!(!rec.join("pending.json").exists());
    }
}

#[cfg(unix)]
#[test]
fn pending_snapshot_rejects_symlinked_raw_or_staging_roots() {
    for (index, raw_case) in [true, false].into_iter().enumerate() {
        let dir = std::env::temp_dir().join(format!(
            "soliton-history-pending-symlink-{}-{}",
            std::process::id(),
            index
        ));
        let _ = fs::remove_dir_all(&dir);
        let archive = dir.join("archive");
        let source = dir.join("note.md");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&source, "immutable source").unwrap();
        let mut store = HistoryStore::open(&archive).unwrap();
        let record = store
            .command(
                "history-import",
                &json!({"path":source,"provider":"manual"}),
            )
            .unwrap()["record"]
            .clone();
        drop(store);
        let id = record["id"].as_str().unwrap();
        let rec = archive.join("records").join(id);
        let raw = archive.join("raw").join(id).join("1");
        let outside = dir.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("note.md"), "immutable source").unwrap();
        fs::write(outside.join("marker"), b"must remain unchanged").unwrap();
        let entries = vec![(
            "note.md".to_owned(),
            ContentHash::digest(b"immutable source").to_string(),
        )];
        let digest = ContentHash::digest(&serde_json::to_vec(&entries).unwrap()).to_string();
        fs::write(
            rec.join("pending.json"),
            serde_json::to_vec(&json!({
                "record":record,
                "stage":"symlink-stage",
                "snapshot_hash":digest
            }))
            .unwrap(),
        )
        .unwrap();
        fs::remove_file(rec.join("record.json")).unwrap();
        fs::remove_file(rec.join("revisions/1.json")).unwrap();
        fs::remove_dir_all(&raw).unwrap();
        if raw_case {
            std::os::unix::fs::symlink(&outside, &raw).unwrap();
        } else {
            std::os::unix::fs::symlink(&outside, archive.join("staging/symlink-stage")).unwrap();
        }
        assert!(HistoryStore::open(&archive).is_err());
        assert_eq!(
            fs::read(outside.join("marker")).unwrap(),
            b"must remain unchanged"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn collected_source_keeps_url_and_parent_reference() {
    let dir = root();
    let source = dir.join("feed.json");
    fs::write(&source,serde_json::to_vec(&json!({"schema":"soliton-source/1","id":"https://example.test/item/1","provider":"feed","title":"Research release","url":"https://example.test/item/1","captured_at":"2026-09-21T00:00:00Z","coverage":{"state":"complete"},"segments":[{"text":"We should review the new release."}],"source_refs":[{"url":"https://example.test/item/1","parent_id":"feed-one"}]})).unwrap()).unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let r = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"normalized"}),
        )
        .unwrap();
    assert_eq!(r["record"]["kind"], "source");
    assert_eq!(r["record"]["source_refs"][0]["parent_id"], "feed-one");
    assert_eq!(
        r["record"]["source_refs"][0]["url"],
        "https://example.test/item/1"
    );
}
#[test]
fn history_worker_leaves_planner_responsive_and_returns_real_archive() {
    let dir = root();
    let source = dir.join("note.txt");
    fs::write(&source, "We need to keep this original.").unwrap();
    let mut service = Service::open(&dir.join("workspace")).unwrap();
    let job = service
        .command(
            "history-job-start",
            &json!({"command":"history-import","args":{"path":source,"provider":"manual"}}),
        )
        .unwrap();
    let task = service
        .command(
            "save",
            &json!({"kind":"task","title":"Work while archive runs"}),
        )
        .unwrap();
    assert_eq!(task["title"], "Work while archive runs");
    for _ in 0..200 {
        let status = service
            .command("history-job-status", &json!({"id":job["id"]}))
            .unwrap();
        if status["state"] != "running" {
            assert_eq!(status["ok"], true);
            assert_eq!(
                status["result"]["record"]["segments"][0]["text"],
                "We need to keep this original."
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("History worker did not finish");
}
#[test]
fn received_audio_is_archived_independently_and_repeat_import_is_idempotent() {
    let dir = root();
    let mut service = Service::open(&dir.join("workspace")).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let bytes = b"RIFF-test-WAVE-original-audio";
    let hash = ContentHash::digest(bytes).to_string();
    let hash = hash.trim_start_matches("sha256:");
    service.command("inbox-begin",&json!({"op_id":id,"filename":"Walk note.wav","size":bytes.len(),"sha256":hash,"intent":"voice_note"})).unwrap();
    service.command("inbox-chunk",&json!({"id":id,"offset":0,"hex":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>(),"sha256":hash})).unwrap();
    let receipt = service.command("inbox-commit", &json!({"id":id})).unwrap();
    let note = service
        .command("history-from-inbox", &json!({"id":id}))
        .unwrap();
    assert_eq!(note["record"]["audio"]["state"], "retained");
    assert_eq!(note["record"]["coverage"]["state"], "unavailable");
    assert_eq!(
        service
            .command("history-from-inbox", &json!({"id":id}))
            .unwrap()["unchanged"],
        true
    );
    let archived = PathBuf::from(note["record"]["audio"]["path"].as_str().unwrap());
    fs::write(receipt["path"].as_str().unwrap(), b"Changed original file").unwrap();
    assert_eq!(fs::read(archived).unwrap(), bytes);
}

#[test]
fn valid_but_stale_projection_recovers_latest_correction() {
    let dir = root();
    let archive = dir.join("archive");
    let source = dir.join("note.txt");
    fs::write(&source, "Original wording").unwrap();
    let mut store = HistoryStore::open(&archive).unwrap();
    let first = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"manual"}),
        )
        .unwrap()["record"]
        .clone();
    let projection = archive
        .join("records")
        .join(first["id"].as_str().unwrap())
        .join("record.json");
    let old = fs::read(&projection).unwrap();
    let corrected = store.command("history-correct", &json!({"id":first["id"],"expected_revision":"1","segment_id":first["segments"][0]["id"],"text":"Corrected wording","author":"test","reason":"spelling"})).unwrap();
    drop(store);
    fs::write(projection, old).unwrap();
    let mut store = HistoryStore::open(&archive).unwrap();
    assert_eq!(
        store
            .command("history-detail", &json!({"id":first["id"]}))
            .unwrap()["revision"],
        corrected["revision"]
    );
}

#[test]
fn large_conversation_keeps_visible_messages_without_exposing_tool_payloads() {
    let dir = root();
    let source = dir.join("conversation.jsonl");
    let mut data = serde_json::to_string(&json!({"type":"session_meta","payload":{"id":"large-chat","parent_id":"parent-chat","timestamp":"2026-09-21T00:00:00Z"}})).unwrap() + "\n";
    for _ in 0..6 {
        data.push_str(&serde_json::to_string(&json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(1_000_000)}})).unwrap());
        data.push('\n');
    }
    data.push_str(&serde_json::to_string(&json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Keep this visible message"}]}})).unwrap());
    data.push('\n');
    fs::write(&source, data).unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let r = store
        .command("history-import", &json!({"path":source,"provider":"codex"}))
        .unwrap();
    assert_eq!(r["record"]["segments"].as_array().unwrap().len(), 1);
    assert_eq!(r["record"]["source_refs"][0]["parent_id"], "parent-chat");
    assert_eq!(r["record"]["coverage"]["state"], "partial");
}

#[test]
fn labeling_and_reimport_keep_full_transcript_and_explicit_subagent_relationships() {
    let dir = root();
    let source = dir.join("child.jsonl");
    fs::write(&source,format!("{}\n{}\n",json!({"type":"session_meta","payload":{"id":"child-one","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-one","agent_path":"/root/child","agent_nickname":"Child"}}}}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":"Visible source text"}}))).unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let first = store
        .command("history-import", &json!({"path":source,"provider":"codex"}))
        .unwrap()["record"]
        .clone();
    assert_eq!(first["source_refs"][0]["parent_id"], "parent-one");
    assert_eq!(first["agent"]["path"], "/root/child");
    let labeled=store.command("history-label",&json!({"id":first["id"],"expected_revision":first["revision"],"title":"A useful label","purpose":"Code review"})).unwrap();
    assert_eq!(labeled["segments"], first["segments"]);
    let repeated = store
        .command("history-import", &json!({"path":source,"provider":"codex"}))
        .unwrap();
    assert_eq!(repeated["unchanged"], true);
    assert_eq!(repeated["record"]["segments"], first["segments"]);
    fs::write(
        dir.join("archive/history-index.sqlite3"),
        b"Corrupted cache",
    )
    .unwrap();
    drop(store);
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let detail = store
        .command("history-detail", &json!({"id":first["id"]}))
        .unwrap();
    assert_eq!(detail["title"], "A useful label");
    assert_eq!(detail["segments"], first["segments"]);
    assert_eq!(detail["relationships"]["parent_origin_id"], "parent-one");
}

#[test]
fn collected_feed_refresh_is_idempotent_and_entry_ids_survive_reordering() {
    let dir = root();
    let path = dir.join("feed.json");
    let mut source = json!({"schema":"soliton-source/1","id":"feed-one","provider":"feed","title":"Feed","captured_at":"first","coverage":{"state":"complete"},"segments":[{"id":"https://example.test/a","text":"First article"},{"id":"https://example.test/b","text":"Second article"}],"source_refs":[{"url":"https://example.test/feed"}]});
    fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let one = store
        .command(
            "history-import",
            &json!({"provider":"normalized","path":path}),
        )
        .unwrap()["record"]
        .clone();
    source["captured_at"] = json!("second");
    fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    assert_eq!(
        store
            .command(
                "history-import",
                &json!({"provider":"normalized","path":path})
            )
            .unwrap()["unchanged"],
        true
    );
    source["segments"].as_array_mut().unwrap().swap(0, 1);
    fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    let two = store
        .command(
            "history-import",
            &json!({"provider":"normalized","path":path}),
        )
        .unwrap()["record"]
        .clone();
    assert_eq!(one["segments"][0]["id"], two["segments"][1]["id"]);
    let corrected=store.command("history-correct",&json!({"id":two["id"],"expected_revision":two["revision"],"segment_id":two["segments"][1]["id"],"text":"Corrected first article"})).unwrap();
    assert_eq!(corrected["segments"][1]["id"], one["segments"][0]["id"]);
}

#[test]
fn later_history_jobs_do_not_erase_an_uncollected_reply() {
    let dir = root();
    let mut service = Service::open(&dir.join("workspace")).unwrap();
    let mut completed = Vec::new();
    for number in 0..3 {
        let source = dir.join(format!("source-{number}.txt"));
        fs::write(&source, format!("Archived source {number}")).unwrap();
        let job = service
            .command(
                "history-job-start",
                &json!({"command":"history-import","args":{"path":source,"provider":"manual"}}),
            )
            .unwrap();
        let mut result = None;
        for _ in 0..200 {
            let status = service
                .command("history-job-status", &json!({"id":job["id"]}))
                .unwrap();
            if status["state"] != "running" {
                result = Some(status);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let result = result.expect("bounded worker must complete");
        assert_eq!(result["ok"], true);
        completed.push((job["id"].clone(), result));
    }
    for (id, prior) in completed {
        assert_eq!(
            service
                .command("history-job-status", &json!({"id":id}))
                .unwrap(),
            prior
        );
    }
}

#[test]
fn reimport_repairs_legacy_incomplete_interpretation_without_erasing_raw() {
    let dir = root();
    let source = dir.join("original.txt");
    fs::write(&source, "The original passage must remain readable.").unwrap();
    let archive = dir.join("archive");
    let mut history = HistoryStore::open(&archive).unwrap();
    let saved = history
        .command(
            "history-import",
            &json!({"path":source,"provider":"manual"}),
        )
        .unwrap()["record"]
        .clone();
    drop(history);
    let folder = archive.join("records").join(saved["id"].as_str().unwrap());
    let projection = folder.join("record.json");
    let mut legacy: Value = serde_json::from_slice(&fs::read(&projection).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("_adapter_version");
    legacy.as_object_mut().unwrap().remove("segments");
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&projection, &bytes).unwrap();
    fs::write(folder.join("revisions/1.json"), &bytes).unwrap();
    let mut history = HistoryStore::open(&archive).unwrap();
    let repaired = history
        .command(
            "history-import",
            &json!({"path":source,"provider":"manual"}),
        )
        .unwrap();
    assert_eq!(repaired["unchanged"], false);
    assert_eq!(repaired["record"]["id"], saved["id"]);
    assert_eq!(
        repaired["record"]["segments"][0]["text"],
        saved["segments"][0]["text"]
    );
    assert_eq!(fs::read(folder.join("revisions/1.json")).unwrap(), bytes);
    assert!(
        archive
            .join("raw")
            .join(saved["id"].as_str().unwrap())
            .join("1/original.txt")
            .is_file()
    );
}
