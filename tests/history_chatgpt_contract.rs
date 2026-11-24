use serde_json::json;
use soliton::history::HistoryStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("soliton-chatgpt-{label}-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn export(path: &Path) {
    fs::write(
        path,
        serde_json::to_vec(&json!([
            {
                "conversation_id":"conv-a",
                "title":"Visible export",
                "create_time":1726900000.25,
                "mapping":{
                    "root":{"id":"root","parent":null,"children":["u"],"message":null},
                    "u":{"id":"u","parent":"root","children":["a"],"message":{"id":"m-u","author":{"role":"user"},"create_time":1726900001.0,"content":{"content_type":"text","parts":["<keep-leading"]}}},
                    "a":{"id":"a","parent":"u","children":[],"message":{"id":"m-a","author":{"role":"assistant"},"create_time":1726900002.0,"content":{"content_type":"text","parts":["answer"]}}},
                    "system":{"id":"system","parent":"a","children":[],"message":{"author":{"role":"system"},"content":{"content_type":"text","parts":["hidden system"]}}}
                }
            },
            {
                "conversation_id":"conv-b",
                "title":"Second conversation",
                "mapping":{
                    "u":{"id":"u","parent":null,"children":[],"message":{"author":{"role":"user"},"content":{"content_type":"text","parts":["second"]}}}
                }
            }
        ]))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn chatgpt_json_preserves_visible_messages_and_parent_metadata() {
    let dir = root("json");
    let source = dir.join("conversations.json");
    export(&source);
    let mut store = HistoryStore::open(&dir.join("archive")).unwrap();
    let result = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"chatgpt"}),
        )
        .unwrap();
    assert_eq!(result["provider"], "chatgpt");
    assert_eq!(result["records"].as_array().unwrap().len(), 2);
    assert_eq!(result["record"]["segments"].as_array().unwrap().len(), 2);
    assert_eq!(result["record"]["segments"][0]["text"], "<keep-leading");
    assert_eq!(result["record"]["segments"][0]["parent_id"], "root");
    assert_eq!(result["record"]["coverage"]["state"], "partial");
    let second = store
        .command(
            "history-import",
            &json!({"path":source,"provider":"chatgpt"}),
        )
        .unwrap();
    assert_eq!(second["unchanged"], true);
    assert_eq!(second["unchanged_records"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn chatgpt_html_requires_explicit_roles_and_capture_sync_is_opt_in() {
    let dir = root("html-capture");
    let html = dir.join("chat.html");
    fs::write(
        &html,
        r#"<html><head><title>HTML conversation</title></head><body>
        <article data-message-author-role="user">&lt;visible user&gt;</article>
        <article data-message-author-role="assistant">Visible answer</article>
        </body></html>"#,
    )
    .unwrap();
    let archive = dir.join("archive");
    let mut store = HistoryStore::open(&archive).unwrap();
    let capture = store
        .command(
            "history-capture-add",
            &json!({"path":html,"provider":"chatgpt"}),
        )
        .unwrap();
    assert_eq!(capture["enabled"], true);
    assert_eq!(capture["opt_in"], true);
    let list = store.command("history-capture-list", &json!({})).unwrap();
    assert_eq!(list["sources"].as_array().unwrap().len(), 1);
    let synced = store.command("history-capture-sync", &json!({})).unwrap();
    assert_eq!(synced["items"][0]["ok"], true);
    let found = store
        .command(
            "history-list",
            &json!({"kind":"chat","q":"visible user","limit":10}),
        )
        .unwrap();
    assert_eq!(found["total"], 1);
    store
        .command("history-capture-remove", &json!({"id":capture["id"]}))
        .unwrap();
    let after = store.command("history-capture-list", &json!({})).unwrap();
    assert_eq!(after["sources"][0]["enabled"], false);
    fs::remove_dir_all(dir).unwrap();
}
