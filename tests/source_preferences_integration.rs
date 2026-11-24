use serde_json::json;
use soliton::service::Service;
use soliton_kernel::ObjectId;
use std::fs;

fn root() -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(ObjectId::mint("src-pref-integration").unwrap().to_string());
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn service_analyze_honors_exclusion_override_and_goal_focus() {
    let dir = root();
    let source = dir.join("learning.txt");
    fs::write(
        &source,
        "We should review this cooking plan and build a recipe.",
    )
    .unwrap();
    let mut service = Service::open(&dir.join("workspace")).unwrap();
    let imported = service
        .command(
            "history-import",
            &json!({"path":source,"provider":"manual"}),
        )
        .unwrap();
    let source_id = imported["record"]["id"].as_str().unwrap().to_owned();
    let excluded = service
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"exclude","matcher":"source","pattern":source_id}),
        )
        .unwrap();
    let denied = service
        .command("intelligence-analyze", &json!({"source_id":source_id}))
        .unwrap();
    assert_eq!(denied["policy"]["excluded"], true);
    assert!(denied["items"].as_array().unwrap().is_empty());
    let reviewed = service
        .command(
            "intelligence-analyze",
            &json!({"source_id":source_id,"override_excluded":true}),
        )
        .unwrap();
    assert_eq!(reviewed["policy"]["overridden"], true);
    assert!(!reviewed["items"].as_array().unwrap().is_empty());

    let goal = service
        .command("save", &json!({"kind":"goal","title":"Cooking practice"}))
        .unwrap();
    let goal_id = goal["id"].as_str().unwrap().to_owned();
    let focused = service
        .command(
            "intelligence-analyze",
            &json!({"source_id":source_id,"goal_id":goal_id,"override_excluded":true}),
        )
        .unwrap();
    assert!(
        focused["items"][0]["goals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] == goal_id)
    );
    assert_eq!(excluded["kind"], "exclude");
    fs::remove_dir_all(dir).unwrap();
}
