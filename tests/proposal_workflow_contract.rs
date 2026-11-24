use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::ObjectId;
use std::fs;
use std::path::PathBuf;

fn fixture() -> (PathBuf, Service, Value) {
    let root = std::env::temp_dir().join(ObjectId::mint("proposal-page-test").unwrap().to_string());
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("source.md");
    fs::write(&source_path, "We need to review this source passage.").unwrap();
    let mut service = Service::open(&root.join("workspace")).unwrap();
    let source = service
        .command(
            "history-import",
            &json!({"path":source_path,"provider":"manual"}),
        )
        .unwrap()["record"]
        .clone();
    (root, service, source)
}

fn proposal(service: &mut Service, source: &Value, title: &str) -> Value {
    service
        .command(
            "proposal-create",
            &json!({
                "source_id":source["id"],
                "source_revision":source["revision"],
                "segment_id":source["segments"][0]["id"],
                "title":title,
                "rationale":"Review the cited passage.",
                "date":""
            }),
        )
        .unwrap()
}

#[test]
fn proposal_list_is_bounded_and_reports_total_for_pages() {
    let (root, mut service, source) = fixture();
    for title in ["First proposal", "Second proposal", "Third proposal"] {
        proposal(&mut service, &source, title);
    }
    let first = service
        .command(
            "proposal-list",
            &json!({"status":"all","offset":0,"limit":2}),
        )
        .unwrap();
    assert_eq!(first["total"], 3);
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let second = service
        .command(
            "proposal-list",
            &json!({"status":"all","offset":2,"limit":2}),
        )
        .unwrap();
    assert_eq!(second["total"], 3);
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_edit_requires_current_source_revision_before_revise() {
    let (root, mut service, source) = fixture();
    let saved = proposal(&mut service, &source, "Review source");
    service
        .command(
            "history-correct",
            &json!({
                "id":source["id"],
                "expected_revision":source["revision"],
                "segment_id":source["segments"][0]["id"],
                "text":"The current source says to defer this review.",
                "author":"test"
            }),
        )
        .unwrap();
    let current = service
        .command("history-detail", &json!({"id":source["id"]}))
        .unwrap();
    assert_ne!(current["revision"], saved["source_revision"]);
    assert_ne!(current["segments"][0]["text"], saved["excerpt"]);
    let stale = service.command(
        "proposal-revise",
        &json!({
            "id":saved["id"],
            "expected_revision":saved["revision"],
            "source_id":source["id"],
            "source_revision":saved["source_revision"],
            "segment_id":saved["segment_id"],
            "title":"Edited against stale source",
            "rationale":"Must be rejected until reviewed.",
            "date":""
        }),
    );
    assert!(stale.is_err());
    let revised = service
        .command(
            "proposal-revise",
            &json!({
                "id":saved["id"],
                "expected_revision":saved["revision"],
                "source_id":source["id"],
                "source_revision":current["revision"],
                "segment_id":current["segments"][0]["id"],
                "title":"Edited after current review",
                "rationale":"Reviewed against the current source.",
                "date":""
            }),
        )
        .unwrap();
    assert_eq!(revised["source_revision"], current["revision"]);
    assert_eq!(revised["excerpt"], current["segments"][0]["text"]);
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

fn settle(service: &mut Service, done: impl Fn(&mut Service) -> bool) {
    for _ in 0..240 {
        service.background_tick().unwrap();
        if done(service) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("collection did not settle");
}

/// A real transcript (passages of a public video, each carrying its URL),
/// collected and archived through the collection path.
fn archived_origin(root: &std::path::Path, service: &mut Service) -> (PathBuf, Value) {
    let origin = root.join("Zs3faMCDYNs.json");
    fs::write(&origin, r#"{"title":"Turn Claude Code Into Your AI Operating System (4 Layers)","segments":[
      {"id":"t164800-74","index":0,"text":"So, a bot never marks a task done. Only I tick it.","start_ms":164800,"end_ms":169680,"url":"https://www.youtube.com/watch?v=Zs3faMCDYNs"},
      {"id":"t665839-297","index":1,"text":"The morning digest lands in my calendar before I'm awake.","start_ms":665839,"end_ms":670000,"url":"https://www.youtube.com/watch?v=Zs3faMCDYNs"}]}"#).unwrap();
    service
        .command(
            "collection-add",
            &json!({"type":"transcript","path":origin}),
        )
        .unwrap();
    let find = |service: &mut Service| {
        service
            .command("history-list", &json!({"kind":"source","limit":100}))
            .ok()
            .and_then(|list| {
                list["items"]
                    .as_array()?
                    .iter()
                    .find(|item| item["title"] == "Zs3faMCDYNs")
                    .map(|item| item["id"].clone())
            })
    };
    settle(service, |service| find(service).is_some());
    let id = find(service).unwrap();
    let source = service
        .command("history-detail", &json!({"id":id}))
        .unwrap();
    (origin, source)
}

fn referenced(
    service: &mut Service,
    source: &Value,
    index: usize,
    url: &str,
) -> Result<Value, String> {
    service
        .command(
            "proposal-create",
            &json!({
                "source_id":source["id"],"source_revision":source["revision"],
                "segment_id":source["segments"][index]["id"],
                "title":format!("Insight {index}"),"rationale":"Selected passage.","date":"",
                "reference":{"url":url}
            }),
        )
        .map_err(|error| error.to_string())
}

#[test]
fn source_backed_proposals_keep_checked_references_and_wait_for_individual_approval() {
    let (root, mut service, _) = fixture();
    let (origin, source) = archived_origin(&root, &mut service);
    let watch = "https://youtu.be/Zs3faMCDYNs";
    let first = referenced(&mut service, &source, 0, watch).unwrap();
    let second = referenced(&mut service, &source, 1, watch).unwrap();
    assert_eq!(first["status"], "ready");
    assert_eq!(
        first["reference"]["locator"],
        "https://www.youtube.com/watch?v=Zs3faMCDYNs&t=164s"
    );
    assert_eq!(
        first["reference"]["checked_against"]["kind"],
        "origin_segment"
    );
    assert_eq!(
        second["reference"]["checked_against"]["origin_segment_id"],
        "t665839-297"
    );
    let mismatch = referenced(
        &mut service,
        &source,
        0,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    )
    .unwrap_err();
    assert!(mismatch.contains("does not match"), "{mismatch}");

    // Each waits in the list for its own decision.
    let listed = service
        .command("proposal-list", &json!({"status":"ready"}))
        .unwrap();
    assert_eq!(listed["total"], 2);
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["reference"]["locator"].is_string())
    );

    let tasks = |service: &mut Service| {
        service.command("state", &json!({})).unwrap()["records"]
            .as_array()
            .map_or(0, |records| {
                records.iter().filter(|r| r["kind"] == "task").count()
            })
    };
    let before = tasks(&mut service);
    let approve = json!({"id":first["id"],"expected_revision":first["revision"],"decision":"approve","op_id":first["op_id"]});
    let applied = service.command("proposal-decide", &approve).unwrap();
    assert_eq!(applied["status"], "applied");
    assert_eq!(tasks(&mut service), before + 1);
    // The task cites the source line it came from.
    let body = applied["task"]["body"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let notes = if body.is_empty() {
        applied["task"]["notes"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    } else {
        body
    };
    assert!(
        notes.contains("Source: Zs3faMCDYNs — https://www.youtube.com/watch?v=Zs3faMCDYNs&t=164s"),
        "{}",
        applied["task"]
    );
    // The same operation again rewrites nothing.
    let retried = service.command("proposal-decide", &approve).unwrap();
    assert_eq!(retried["receipt"], applied["receipt"]);
    assert_eq!(retried["revision"], applied["revision"]);
    assert_eq!(tasks(&mut service), before + 1);
    // The sibling still waits, untouched.
    let sibling = service
        .command("proposal-detail", &json!({"id":second["id"]}))
        .unwrap();
    assert_eq!(sibling["status"], "ready");
    assert_eq!(sibling["revision"], second["revision"]);

    // Stale origin evidence is refused before any downstream write.
    fs::write(&origin, "{}").unwrap();
    let stale = service
        .command("proposal-decide", &json!({"id":second["id"],"expected_revision":second["revision"],"decision":"approve","op_id":second["op_id"]}))
        .unwrap_err()
        .to_string();
    assert!(stale.contains("stale"), "{stale}");
    assert_eq!(tasks(&mut service), before + 1);
    assert_eq!(
        service
            .command("proposal-detail", &json!({"id":second["id"]}))
            .unwrap()["status"],
        "ready"
    );
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

/// The proposal record exactly as release 0.5.19 reads it (its struct,
/// unknown fields denied). A record that carries a reference must still
/// parse, so rolling back keeps every proposal readable.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Proposal0519 {
    schema: u32,
    #[serde(default)]
    request_id: String,
    source_id: String,
    source_revision: String,
    segment_id: String,
    excerpt: String,
    source_title: String,
    title: String,
    rationale: String,
    goal_id: String,
    goal_hash: String,
    date: String,
    schedule: Value,
    status: String,
    op_id: String,
    task_id: String,
    receipt: Value,
    allow_overlap: bool,
}

#[test]
fn a_referenced_and_approved_proposal_stays_readable_by_the_previous_release() {
    let (root, mut service, _) = fixture();
    let (_, source) = archived_origin(&root, &mut service);
    let saved = referenced(&mut service, &source, 0, "https://youtu.be/Zs3faMCDYNs").unwrap();
    assert!(saved["reference"]["locator"].is_string());
    let body = |service: &Service, id: &Value| {
        let id: ObjectId = id.as_str().unwrap().parse().unwrap();
        service
            .snapshot
            .get(&id)
            .unwrap()
            .entity
            .meta()
            .details
            .body
            .clone()
    };
    let old: Proposal0519 = serde_json::from_str(&body(&service, &saved["id"])).unwrap();
    assert_eq!(old.status, "ready");
    service
        .command("proposal-decide", &json!({"id":saved["id"],"expected_revision":saved["revision"],"decision":"approve","op_id":saved["op_id"]}))
        .unwrap();
    let old: Proposal0519 = serde_json::from_str(&body(&service, &saved["id"])).unwrap();
    assert_eq!(old.status, "applied");
    drop(service);
    fs::remove_dir_all(root).unwrap();
}
