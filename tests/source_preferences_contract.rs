use serde_json::json;
use soliton::source_preferences::SourcePreferences;
use soliton_kernel::ObjectId;
use std::fs;

fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("src-pref-contract").unwrap().to_string());
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn global_exclusion_and_goal_save_have_explicit_precedence() {
    let root = root();
    let mut store = SourcePreferences::open(&root).unwrap();
    let excluded = store
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"exclude","matcher":"domain","pattern":"example.test"}),
        )
        .unwrap();
    let denied = store
        .command(
            "source-policy-evaluate",
            &json!({"source_id":"feed-1","url":"https://www.example.test/item","title":"Research"}),
        )
        .unwrap();
    assert_eq!(denied["include"], false);
    assert_eq!(denied["excluded"], true);
    let saved = store.command("source-policy-set", &json!({"scope":"goal","goal_id":"goal-cooking","kind":"save","matcher":"source","pattern":"feed-1"})).unwrap();
    let allowed = store.command("source-policy-evaluate", &json!({"source_id":"feed-1","url":"https://example.test/item","goal_id":"goal-cooking"})).unwrap();
    assert_eq!(allowed["include"], true);
    assert_eq!(
        allowed["reason"],
        "Goal save preference overrides a broader exclusion"
    );
    let reviewed = store.command("source-policy-evaluate", &json!({"source_id":"other","url":"https://example.test/item","override_excluded":true})).unwrap();
    assert_eq!(reviewed["include"], true);
    assert_eq!(reviewed["overridden"], true);
    assert_ne!(excluded["id"], saved["id"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn updates_and_removals_require_revision_and_survive_restart() {
    let root = root();
    let mut store = SourcePreferences::open(&root).unwrap();
    let rule = store
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"save","matcher":"topic","pattern":"rust"}),
        )
        .unwrap();
    assert!(
        store
            .command(
                "source-policy-set",
                &json!({"scope":"global","kind":"save","matcher":"topic","pattern":"rust"})
            )
            .is_err()
    );
    let updated = store.command("source-policy-set", &json!({"scope":"global","kind":"save","matcher":"topic","pattern":"rust","enabled":false,"expected_revision":rule["revision"]})).unwrap();
    assert_eq!(updated["enabled"], false);
    drop(store);
    let mut reopened = SourcePreferences::open(&root).unwrap();
    let listed = reopened
        .command(
            "source-policy-list",
            &json!({"q":"rust","offset":0,"limit":10}),
        )
        .unwrap();
    assert_eq!(listed["total"], 1);
    assert!(
        reopened
            .command(
                "source-policy-remove",
                &json!({"id":updated["id"],"expected_revision":"stale"})
            )
            .is_err()
    );
    reopened
        .command(
            "source-policy-remove",
            &json!({"id":updated["id"],"expected_revision":updated["revision"]}),
        )
        .unwrap();
    assert_eq!(
        reopened.command("source-policy-list", &json!({})).unwrap()["total"],
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn list_is_bounded_and_supports_pagination() {
    let root = root();
    let mut store = SourcePreferences::open(&root).unwrap();
    for index in 0..7 {
        store.command("source-policy-set", &json!({"scope":"global","kind":"save","matcher":"source","pattern":format!("source-{index}")})).unwrap();
    }
    let page = store
        .command("source-policy-list", &json!({"offset":2,"limit":3}))
        .unwrap();
    assert_eq!(page["total"], 7);
    assert_eq!(page["items"].as_array().unwrap().len(), 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_batch_preserves_exclusion_and_goal_exception() {
    let root = root();
    let mut store = SourcePreferences::open(&root).unwrap();
    store
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"exclude","matcher":"domain","pattern":"example.test"}),
        )
        .unwrap();
    store.command("source-policy-set",&json!({"scope":"goal","goal_id":"goal-research","kind":"save","matcher":"url","pattern":"https://example.test/Case"})).unwrap();
    let result = store
        .command(
            "source-policy-evaluate-batch",
            &json!({"items":[
                {"source_id":"a","url":"https://example.test/Case","goal_id":"goal-research"},
                {"source_id":"b","url":"https://example.test/case","goal_id":"goal-research"},
                {"source_id":"c","url":"https://example.test/Case"}
            ]}),
        )
        .unwrap();
    assert_eq!(result["items"][0]["include"], true);
    assert_eq!(result["items"][1]["include"], false);
    assert_eq!(result["items"][2]["include"], false);
    assert!(
        store
            .command(
                "source-policy-evaluate-batch",
                &json!({"items":vec![json!({"source_id":"x"});257]})
            )
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}
