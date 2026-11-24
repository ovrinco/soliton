use serde_json::{Value, json};
use soliton::history::HistoryStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = std::env::var_os("SOLITON_KINDS_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = scratch.join(format!("soliton-kinds-{label}-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn import_chat(store: &mut HistoryStore, path: &Path, prompt: &str, messages: usize) -> Value {
    let mut lines = vec![json!({"type":"session_meta","payload":{"id":path.display().to_string(),"timestamp":"2026-10-01T00:00:00Z"}}).to_string()];
    for i in 0..messages {
        lines.push(json!({"timestamp":format!("2026-10-01T{:02}:00:00Z",i),"role":if i%2==0 {"user"}else{"assistant"},"content":if i==0 {prompt.to_owned()}else{"discussion ".repeat(1200)}}).to_string());
    }
    fs::write(path, lines.join("\n") + "\n").unwrap();
    store
        .command("history-import", &json!({"path":path,"provider":"codex"}))
        .unwrap()["record"]
        .clone()
}

#[test]
fn automatic_kinds_use_provenance_and_measured_discussion() {
    let dir = root("roles");
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    for (file, prompt, count, kind, color) in [
        (
            "ordinary.jsonl",
            "What is two plus two?",
            2,
            "ordinary",
            "grey",
        ),
        (
            "qream.jsonl",
            "You are a QREAM sub-agent. Carry out this directive.",
            2,
            "qream",
            "purple",
        ),
        ("zeno.jsonl", "Zeno console session", 2, "zeno", "yellow"),
        (
            "deep.jsonl",
            "Let us discuss the long-term direction.",
            22,
            "deep_discussion",
            "green",
        ),
        (
            "dense.jsonl",
            "Develop a substantial specification with acceptance criteria.",
            22,
            "dense_specification",
            "blue",
        ),
    ] {
        let record = import_chat(&mut store, &dir.join(file), prompt, count);
        assert_eq!(record["chat_kind"], kind);
        assert_eq!(record["chat_color_token"], color);
        let detail = store
            .command("history-detail", &json!({"id":record["id"]}))
            .unwrap();
        assert_eq!(detail["chat_kind"], kind);
    }
    let ambiguous = import_chat(
        &mut store,
        &dir.join("unrelated.jsonl"),
        "What does the word QREAM mean?",
        2,
    );
    assert_eq!(ambiguous["chat_kind"], "ordinary");
    for args in [
        json!({"kind":"chat"}),
        json!({"kind":"chat","q":"discussion"}),
    ] {
        let list = store.command("history-list", &args).unwrap();
        assert!(
            list["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["chat_kind"].is_string())
        );
    }
}

#[test]
fn mapping_color_edit_changes_detail_without_rebuild() {
    let dir = root("palette");
    let mapping = dir.join("chat_kinds.json");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/chat_kinds.json"),
        &mapping,
    )
    .unwrap();
    let mut store = HistoryStore::open_with_kind_mapping(&dir.join("archive"), &mapping).unwrap();
    let config = store.command("history-chat-kinds", &json!({})).unwrap();
    assert_eq!(config["kinds"].as_array().unwrap().len(), 5);
    let record = import_chat(&mut store, &dir.join("ordinary.jsonl"), "How are you?", 2);
    assert_eq!(record["chat_color_token"], "grey");
    let mut map: Value = serde_json::from_slice(&fs::read(&mapping).unwrap()).unwrap();
    map["kinds"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|k| k["kind"] == "ordinary")
        .unwrap()["color_token"] = json!("magenta");
    fs::write(&mapping, serde_json::to_vec(&map).unwrap()).unwrap();
    assert_eq!(
        store
            .command("history-detail", &json!({"id":record["id"]}))
            .unwrap()["chat_color_token"],
        "magenta"
    );
    for args in [json!({"kind":"chat"}), json!({"kind":"chat","q":"How"})] {
        assert_eq!(
            store.command("history-list", &args).unwrap()["items"][0]["chat_color_token"],
            "magenta"
        );
    }
}

#[test]
fn standalone_notes_and_collection_filters_are_truthful() {
    let dir = root("collections");
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    import_chat(&mut store, &dir.join("real.jsonl"), "Authentic question", 2);
    let note = dir.join("ChatGPT export branding.txt");
    fs::write(
        &note,
        "Codex CLI branding\nA standalone note about a ChatGPT export.",
    )
    .unwrap();
    let imported = store
        .command("history-import", &json!({"path":note,"provider":"manual"}))
        .unwrap()["record"]
        .clone();
    assert_eq!(imported["kind"], "source");
    let empty = dir.join("branding.jsonl");
    fs::write(
        &empty,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"branding\"}}\n",
    )
    .unwrap();
    store
        .command("history-import", &json!({"path":empty,"provider":"codex"}))
        .unwrap();
    for kind in ["chat", "voice_note", "run", "source"] {
        for q in ["", "branding"] {
            let list = store
                .command("history-list", &json!({"kind":kind,"q":q}))
                .unwrap();
            assert!(
                list["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["kind"] == kind)
            );
            if kind == "chat" && q == "branding" {
                assert_eq!(list["total"], 0);
            }
            println!("collection {kind} query {q:?}: {}", list["total"]);
        }
    }
    assert_eq!(
        store.command("history-list", &json!({})).unwrap()["total"],
        2
    );
    drop(store);
    for kind in ["run", "voice_note", "note"] {
        let record = json!({"id":kind,"revision":"1","_source_revision":1,"_origin_key":format!("fixture-{kind}"),"kind":kind,"title":format!("Genuine {kind}"),"provider":"fixture","segments":[],"source_refs":[]});
        let revisions = dir.join("archive/records").join(kind).join("revisions");
        fs::create_dir_all(&revisions).unwrap();
        fs::write(
            revisions.join("1.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
    let mut reopened = HistoryStore::open(&dir.join("archive")).unwrap();
    assert_eq!(
        reopened
            .command("history-list", &json!({"kind":"chat","q":"branding"}))
            .unwrap()["total"],
        0
    );
    for kind in ["chat", "voice_note", "run", "source"] {
        let list = reopened
            .command("history-list", &json!({"kind":kind}))
            .unwrap();
        assert_eq!(list["total"], 1);
        assert!(
            list["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["kind"] == kind)
        );
        println!("populated collection {kind}: 1/1");
    }
    assert_eq!(
        reopened.command("history-list", &json!({})).unwrap()["total"],
        4
    );
    assert_eq!(
        reopened
            .command("history-list", &json!({"q":"Genuine"}))
            .unwrap()["total"],
        2
    );
}

#[test]
fn capture_check_facts_track_each_check_and_survive_reenable() {
    let dir = root("check-facts");
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let path = dir.join("source.jsonl");
    import_chat(&mut store, &path, "A real captured question", 2);
    let source = store
        .command(
            "history-capture-add",
            &json!({"path":path,"provider":"codex"}),
        )
        .unwrap();
    let detail = |store: &mut HistoryStore| {
        store
            .command("history-capture-detail", &json!({"id":source["id"]}))
            .unwrap()
    };
    let fresh = detail(&mut store);
    assert_eq!(fresh["last_checked_at"], Value::Null);
    assert_eq!(fresh["last_result"], Value::Null);
    assert!(fresh.get("last_sync").is_none());
    assert_eq!(
        store
            .command("history-capture-sync", &json!({"id":source["id"]}))
            .unwrap()["items"][0]["ok"],
        true
    );
    let checked = detail(&mut store);
    assert!(checked["last_checked_at"].is_string());
    assert_eq!(
        checked["last_result"],
        json!({"imported":0,"unchanged":1,"failed":0})
    );
    fs::write(&path, "invalid complete json\n").unwrap();
    assert_eq!(
        store
            .command("history-capture-sync", &json!({"id":source["id"]}))
            .unwrap()["items"][0]["ok"],
        false
    );
    let failed = detail(&mut store);
    assert!(failed["last_checked_at"].as_str() >= checked["last_checked_at"].as_str());
    assert_eq!(failed["last_result"]["failed"], 1);
    assert!(failed.get("last_sync").is_none());
    store
        .command("history-capture-remove", &json!({"id":source["id"]}))
        .unwrap();
    store
        .command(
            "history-capture-add",
            &json!({"path":path,"provider":"codex"}),
        )
        .unwrap();
    let reenabled = detail(&mut store);
    assert_eq!(reenabled["enabled"], true);
    assert_eq!(reenabled["last_checked_at"], failed["last_checked_at"]);
    assert_eq!(reenabled["last_result"], failed["last_result"]);
    assert!(reenabled.get("last_sync").is_none());
}

#[test]
fn capture_detail_states_only_what_is_switched_on_and_what_a_check_found() {
    let dir = root("capture");
    let mut store = HistoryStore::open(&dir).unwrap();
    fs::write(dir.join("capture-sources.json"), serde_json::to_vec(&json!({"schema":"soliton-history-capture/1","sources":[
        {"id":"disabled","provider":"codex","enabled":false,"last_checked_at":"2026-10-01T00:00:00Z","last_result":{"status":"failed","errors":2}},
        {"id":"past-success","provider":"codex","enabled":false,"last_sync":"2026-09-30T12:00:00Z","last_checked_at":"2026-09-30T12:00:00Z","last_result":{"imported":3,"unchanged":4,"errors":0,"status":"complete"}},
        {"id":"never","provider":"codex","enabled":true}
    ]})).unwrap()).unwrap();
    let detail = store
        .command("history-capture-detail", &json!({"id":"disabled"}))
        .unwrap();
    assert_eq!(detail["enabled"], false);
    assert_eq!(
        detail["last_result"],
        json!({"imported":0,"unchanged":0,"failed":2})
    );
    println!("disabled source detail: {detail}");
    let past = store
        .command("history-capture-detail", &json!({"id":"past-success"}))
        .unwrap();
    assert!(past.get("last_sync").is_none());
    assert_eq!(past["last_checked_at"], "2026-09-30T12:00:00Z");
    assert_eq!(
        past["last_result"],
        json!({"imported":3,"unchanged":4,"failed":0})
    );
    let never = store
        .command("history-capture-detail", &json!({"id":"never"}))
        .unwrap();
    assert_eq!(never["last_checked_at"], Value::Null);
    assert_eq!(never["last_result"], Value::Null);
    assert!(
        store
            .command("history-capture-detail", &json!({"id":"missing"}))
            .is_err()
    );
    assert_eq!(
        store.command("history-capture-list", &json!({})).unwrap()["sources"][0]["enabled"],
        false
    );
}

#[test]
fn every_real_chat_is_classified_and_collections_are_pure() {
    let Ok(copy) = std::env::var("SOLITON_HISTORY_COPY") else {
        return;
    };
    let mut store = HistoryStore::open(Path::new(&copy)).unwrap();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut uncertain = Vec::new();
    for kind in ["chat", "voice_note", "run", "source"] {
        let mut checked = 0;
        let mut total = 1;
        while checked < total {
            let page = store
                .command(
                    "history-list",
                    &json!({"kind":kind,"limit":100,"offset":checked}),
                )
                .unwrap();
            total = page["total"].as_u64().unwrap() as usize;
            for record in page["items"].as_array().unwrap() {
                assert_eq!(record["kind"], kind);
                if kind == "chat" {
                    let role = record["chat_kind"]
                        .as_str()
                        .expect("Every chat needs a kind");
                    *counts.entry(role.to_owned()).or_default() += 1;
                    uncertain.push((
                        record["chat_kind_certainty"].as_f64().unwrap(),
                        record["title"].clone(),
                        record["chat_kind"].clone(),
                        record["chat_kind_reason"].clone(),
                    ));
                }
                checked += 1;
            }
        }
        println!("real collection {kind}: checked {checked}/{total}");
        assert_eq!(checked, total);
    }
    println!(
        "real kind counts: {}",
        serde_json::to_string(&counts).unwrap()
    );
    let mut archived_counts = std::collections::BTreeMap::<String, usize>::new();
    let mut archived_chats = 0;
    for entry in fs::read_dir(Path::new(&copy).join("records")).unwrap() {
        let path = entry.unwrap().path().join("record.json");
        let record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        if record["kind"] == "chat" {
            let kind = record["chat_kind"]
                .as_str()
                .expect("Even excluded native sessions have the ordinary fallback");
            *archived_counts.entry(kind.to_owned()).or_default() += 1;
            archived_chats += 1;
        }
    }
    println!(
        "archived chat classifications: checked {archived_chats}/{archived_chats}; {}",
        serde_json::to_string(&archived_counts).unwrap()
    );
    uncertain.sort_by(|a, b| a.0.total_cmp(&b.0));
    for item in uncertain.iter().take(5) {
        println!(
            "least certain: {}",
            json!({"certainty":item.0,"title":item.1,"kind":item.2,"why":item.3})
        );
    }
}
