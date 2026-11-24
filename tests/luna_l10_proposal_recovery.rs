use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::{Entity, ObjectId};
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("luna-l10-proposal").unwrap().to_string());
    fs::create_dir_all(&path).unwrap();
    path
}

fn force_pending_proposal(service: &mut Service, id: &str) {
    let object_id: ObjectId = id.parse().unwrap();
    let stored = service.snapshot.get(&object_id).unwrap().clone();
    let mut entity = stored.entity;
    let details = &mut entity.meta_mut().details.body;
    let mut proposal: Value = serde_json::from_str(details).unwrap();
    proposal["status"] = json!("applying");
    proposal["receipt"] = Value::Null;
    *details = serde_json::to_string(&proposal).unwrap();
    assert!(matches!(entity, Entity::Document(_)));
    service.repo.update(entity, stored.content_hash).unwrap();
}

#[test]
fn lost_ack_retry_persists_applied_receipt_after_task_commit() {
    let root = root();
    let source_path = root.join("source.md");
    fs::write(
        &source_path,
        "Review this retained source passage and make one concrete task.",
    )
    .unwrap();

    let (saved, task_path) = {
        let mut service = Service::open(&root.join("workspace")).unwrap();
        let source = service
            .command(
                "history-import",
                &json!({"path":source_path,"provider":"manual"}),
            )
            .unwrap()["record"]
            .clone();
        let saved = service
            .command(
                "proposal-create",
                &json!({
                    "source_id":source["id"],
                    "source_revision":source["revision"],
                    "segment_id":source["segments"][0]["id"],
                    "title":"Review the retained source",
                    "rationale":"Create one task from the cited passage.",
                    "date":""
                }),
            )
            .unwrap();
        let approved = service
            .command(
                "proposal-decide",
                &json!({
                    "id":saved["id"],
                    "expected_revision":saved["revision"],
                    "op_id":saved["op_id"],
                    "decision":"approve",
                    "allow_overlap":false
                }),
            )
            .unwrap();
        assert_eq!(approved["status"], "applied");
        let task_path = root
            .join("workspace/objects/task")
            .join(format!("{}.soliton", saved["task_id"].as_str().unwrap()));
        assert!(task_path.is_file());
        force_pending_proposal(&mut service, saved["id"].as_str().unwrap());
        (saved, task_path)
    };

    let mut reopened = Service::open(&root.join("workspace")).unwrap();
    let pending = reopened
        .command("proposal-detail", &json!({"id":saved["id"]}))
        .unwrap();
    // The effective view already derives applied from the existing task. The
    // persisted receipt is the missing part of the lost-ack recovery.
    assert_eq!(pending["status"], "applied");
    assert!(pending["receipt"].is_null());

    let reconciled = reopened
        .command(
            "proposal-decide",
            &json!({
                "id":saved["id"],
                "expected_revision":pending["revision"],
                "op_id":saved["op_id"],
                "decision":"approve",
                "allow_overlap":false
            }),
        )
        .unwrap();
    assert_eq!(reconciled["status"], "applied");
    assert_eq!(reconciled["receipt"]["status"], "applied");

    let persisted = reopened
        .command("proposal-detail", &json!({"id":saved["id"]}))
        .unwrap();
    assert_eq!(persisted["status"], "applied");
    assert_eq!(persisted["receipt"]["status"], "applied");
    assert!(task_path.is_file());
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}
