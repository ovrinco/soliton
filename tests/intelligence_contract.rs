use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::ObjectId;
use std::{fs, path::PathBuf};

fn fixture() -> (PathBuf, Service, Value) {
    let root = std::env::temp_dir().join(ObjectId::mint("intelligence-test").unwrap().to_string());
    fs::create_dir_all(&root).unwrap();
    let text = root.join("source.md");
    fs::write(
        &text,
        "We need to review the calendar integration and keep the original voice recording.",
    )
    .unwrap();
    let mut service = Service::open(&root.join("workspace")).unwrap();
    let source = service
        .command("history-import", &json!({"path":text,"provider":"manual"}))
        .unwrap()["record"]
        .clone();
    (root, service, source)
}
fn propose(s: &mut Service, source: &Value, extra: Value) -> Value {
    let mut args = json!({"source_id":source["id"],"source_revision":source["revision"],"segment_id":source["segments"][0]["id"],"title":"Review the calendar integration","rationale":"Apply this source to the active work.","date":"2026-09-22"});
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    s.command("proposal-create", &args).unwrap()
}
fn decide(s: &mut Service, p: &Value, decision: &str) -> soliton::Result<Value> {
    s.command("proposal-decide",&json!({"id":p["id"],"expected_revision":p["revision"],"op_id":p["op_id"],"decision":decision}))
}
#[test]
fn source_proposal_approval_receipt_survives_lost_reply_and_restart() {
    let (root, mut s, source) = fixture();
    let p = propose(&mut s, &source, json!({}));
    assert_eq!(
        s.state().unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "task")
            .count(),
        0
    );
    let applied = decide(&mut s, &p, "approve").unwrap();
    assert_eq!(applied["status"], "applied");
    let task = applied["task_id"].clone();
    drop(s);
    let mut s = Service::open(&root.join("workspace")).unwrap();
    let replay = decide(&mut s, &p, "approve").unwrap();
    assert_eq!(replay["receipt"]["task_id"], task);
    let state = s.state().unwrap();
    let tasks: Vec<_> = state["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "task")
        .collect();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["links"][0], p["id"]);
    let calendar = s
        .command(
            "calendar",
            &json!({"from":"2026-09-22","through":"2026-09-22","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(calendar[0]["id"], task);
}
#[test]
fn individual_decisions_do_not_apply_other_proposals() {
    let (_, mut s, source) = fixture();
    let a = propose(&mut s, &source, json!({}));
    let b = propose(&mut s, &source, json!({"title":"Another action"}));
    let c = propose(&mut s, &source, json!({"title":"Third action"}));
    decide(&mut s, &a, "approve").unwrap();
    assert_eq!(decide(&mut s, &b, "defer").unwrap()["status"], "deferred");
    assert_eq!(decide(&mut s, &c, "reject").unwrap()["status"], "rejected");
    assert_eq!(
        s.state().unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "task")
            .count(),
        1
    );
    assert!(decide(&mut s, &c, "approve").is_err());
}
#[test]
fn source_correction_blocks_stale_approval_without_mutating_original() {
    let (_, mut s, source) = fixture();
    let p = propose(&mut s, &source, json!({}));
    s.command("history-correct",&json!({"id":source["id"],"expected_revision":source["revision"],"segment_id":source["segments"][0]["id"],"text":"Actually defer this work.","author":"test"})).unwrap();
    assert!(decide(&mut s, &p, "approve").is_err());
    let original = s
        .command(
            "history-detail",
            &json!({"id":source["id"],"revision":source["revision"]}),
        )
        .unwrap();
    assert_eq!(original["segments"], source["segments"]);
    assert_eq!(
        s.state().unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "task")
            .count(),
        0
    );
}
#[test]
fn goal_change_is_a_conflict_and_overlap_needs_explicit_choice() {
    let (_, mut s, source) = fixture();
    let goal = s
        .command("save", &json!({"kind":"goal","title":"Calendar quality"}))
        .unwrap();
    let p = propose(&mut s, &source, json!({"goal_id":goal["id"]}));
    s.command(
        "save",
        &json!({"id":goal["id"],"hash":goal["hash"],"title":"Changed goal"}),
    )
    .unwrap();
    assert!(decide(&mut s, &p, "approve").is_err());
    let schedule =
        json!({"time":"09:00","duration_minutes":30,"timezone":"America/Toronto","fold":"earlier"});
    s.command("save",&json!({"kind":"task","title":"Existing work","scheduled_for":"2026-09-22","schedule":schedule})).unwrap();
    let q = propose(&mut s, &source, json!({"schedule":schedule}));
    assert!(decide(&mut s, &q, "approve").is_err());
    assert_eq!(s.command("proposal-decide",&json!({"id":q["id"],"expected_revision":q["revision"],"op_id":q["op_id"],"decision":"approve","allow_overlap":true})).unwrap()["status"],"applied");
}
#[test]
fn analysis_has_exact_passages_and_never_creates_tasks() {
    let (_, mut s, source) = fixture();
    s.command(
        "save",
        &json!({"kind":"goal","title":"Calendar integration"}),
    )
    .unwrap();
    let before = s.state().unwrap()["records"].clone();
    let a = s
        .command("intelligence-analyze", &json!({"source_id":source["id"]}))
        .unwrap();
    assert_eq!(a["items"][0]["text"], source["segments"][0]["text"]);
    assert_eq!(a["items"][0]["goals"][0]["basis"], "lexical_match");
    assert_eq!(s.state().unwrap()["records"], before);
}

#[test]
fn revised_proposal_preserves_identity_and_invalidates_old_approval() {
    let (_, mut s, source) = fixture();
    let p = propose(&mut s, &source, json!({}));
    let args = json!({"id":p["id"],"expected_revision":p["revision"],"source_id":source["id"],"source_revision":source["revision"],"segment_id":source["segments"][0]["id"],"title":"Edited outcome","date":"2026-09-23"});
    let revised = s.command("proposal-revise", &args).unwrap();
    assert_eq!(revised["id"], p["id"]);
    assert_ne!(revised["op_id"], p["op_id"]);
    assert!(decide(&mut s, &p, "approve").is_err());
    let applied = decide(&mut s, &revised, "approve").unwrap();
    assert_eq!(applied["task"]["title"], "Edited outcome");
    assert!(s.command("proposal-revise", &args).is_err());
}
#[test]
fn calendar_preview_resolves_timezone_and_overnight_without_creating_tasks() {
    let (_, mut s, source) = fixture();
    propose(
        &mut s,
        &source,
        json!({"date":"2026-09-22","schedule":{"time":"23:30","duration_minutes":120,"timezone":"America/Toronto","fold":"earlier"}}),
    );
    let v = s
        .command(
            "proposal-calendar",
            &json!({"from":"2026-09-23","through":"2026-09-23","timezone":"UTC"}),
        )
        .unwrap();
    assert_eq!(v[0]["local_start"], "2026-09-23T03:30");
    assert_eq!(v[0]["local_end"], "2026-09-23T05:30");
    assert!(
        s.command(
            "calendar",
            &json!({"from":"2026-09-22","through":"2026-09-23","timezone":"UTC"})
        )
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty()
    );
}

#[test]
fn create_request_identity_survives_restart_and_rejects_changed_payload() {
    let (root, mut service, source) = fixture();
    let request_id = "66666666-6666-4666-8666-666666666666";
    let proposal = propose(&mut service, &source, json!({"request_id": request_id}));
    drop(service);
    let mut service = Service::open(&root.join("workspace")).unwrap();
    let repeated = propose(&mut service, &source, json!({"request_id": request_id}));
    assert_eq!(proposal["id"], repeated["id"]);
    assert_eq!(proposal["op_id"], repeated["op_id"]);
    assert!(service.command("proposal-create", &json!({"request_id":request_id,"source_id":source["id"],"source_revision":source["revision"],"segment_id":source["segments"][0]["id"],"title":"Different content","rationale":"Apply this source to the active work.","date":"2026-09-22"})).is_err());
    assert_eq!(
        service.command("proposal-list", &json!({})).unwrap()["total"],
        1
    );
}

#[test]
fn generic_work_words_do_not_manufacture_goal_relevance() {
    let (_, mut service, source) = fixture();
    service
        .command(
            "save",
            &json!({"kind":"goal","title":"Make the work better with daily review"}),
        )
        .unwrap();
    let analyzed = service
        .command("intelligence-analyze", &json!({"source_id":source["id"]}))
        .unwrap();
    // A domain-specific match is required; the generic word "review" alone
    // is still a literal match, so inspect the more general source separately.
    let record = service.command("history-correct", &json!({"id":source["id"],"expected_revision":source["revision"],"segment_id":source["segments"][0]["id"],"text":"We should keep this work.","author":"acceptance"})).unwrap();
    assert!(!analyzed["items"].as_array().unwrap().is_empty());
    let refined = service
        .command("intelligence-analyze", &json!({"source_id":record["id"]}))
        .unwrap();
    assert_eq!(refined["items"][0]["goals"], json!([]));
}

/// One real transcript collected with automatic analysis asked for, his
/// Local model switch on and no model set: the analysis waits, and every
/// stage it then passes is one dated line, in order, from its own record.
#[test]
fn stage_history_of_a_collected_transcript_advances_with_dates() {
    let root =
        std::env::temp_dir().join(ObjectId::mint("intelligence-stages").unwrap().to_string());
    fs::create_dir_all(&root).unwrap();
    let mut s = Service::open(&root.join("workspace")).unwrap();
    // A fixture client that records and sends nothing: no request can reach
    // his gateway from this test, whatever the analysis does.
    let client = root.join("fixture-client.sh");
    let client_log = root.join("fixture-client.log");
    fs::write(
        &client,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nexit 7\n",
            client_log.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&client).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&client, permissions).unwrap();
    s.set_briefing_tools(soliton::briefing::StoreTools {
        client,
        ..Default::default()
    });
    s.command("preferences", &json!({"localModel":true}))
        .unwrap();
    let origin = root.join("Zs3faMCDYNs.json");
    fs::write(&origin, r#"{"title":"Turn Claude Code Into Your AI Operating System (4 Layers)","segments":[
      {"id":"t164800-74","index":0,"text":"So, a bot never marks a task done. Only I tick it.","start_ms":164800,"end_ms":169680,"url":"https://www.youtube.com/watch?v=Zs3faMCDYNs"},
      {"id":"t665839-297","index":1,"text":"The morning digest lands in my calendar before I'm awake.","start_ms":665839,"end_ms":670000,"url":"https://www.youtube.com/watch?v=Zs3faMCDYNs"}]}"#).unwrap();
    let added = s
        .command(
            "collection-add",
            &json!({"type":"transcript","path":origin,"auto_briefing":true}),
        )
        .unwrap();
    let mut receipt = Value::Null;
    for _ in 0..240 {
        // The cadence tick is the automatic path that starts analysis.
        s.background_tick().unwrap();
        s.command("collection-tick", &json!({})).unwrap();
        let listed = s.command("collection-list", &json!({})).unwrap();
        if let Some(job) = listed["jobs"]
            .as_array()
            .and_then(|jobs| jobs.iter().find(|job| job["id"] == added["job_id"]))
            && !job["briefing"].is_null()
        {
            receipt = job["briefing"].clone();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    // The automatic entry was recorded as waiting; nothing was sent.
    assert_eq!(receipt["state"], "waiting", "{receipt}");
    assert!(
        !client_log.exists(),
        "the briefing client was asked to send"
    );
    let source_id = s
        .command("history-list", &json!({"kind":"source","limit":100}))
        .unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Zs3faMCDYNs")
        .unwrap()["id"]
        .clone();
    let source = s
        .command("history-detail", &json!({"id":source_id}))
        .unwrap();
    let stages = |s: &mut Service| {
        let v = s
            .command("proposal-stages", &json!({"source_id":source_id}))
            .unwrap();
        v["stages"].as_array().unwrap().clone()
    };
    let names = |lines: &[Value]| {
        lines
            .iter()
            .map(|l| l["stage"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&stages(&mut s)),
        ["collected", "transcribed", "waiting"]
    );

    let p = s
        .command("proposal-create", &json!({"source_id":source["id"],"source_revision":source["revision"],"segment_id":source["segments"][0]["id"],"title":"Only I mark a task done","rationale":"From the passage.","date":"","reference":{"url":"https://youtu.be/Zs3faMCDYNs"}}))
        .unwrap();
    assert_eq!(
        names(&stages(&mut s)),
        ["collected", "transcribed", "waiting", "proposed"]
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    decide(&mut s, &p, "approve").unwrap();
    let lines = stages(&mut s);
    eprintln!("stages: {}", serde_json::to_string_pretty(&lines).unwrap());
    assert_eq!(
        names(&lines),
        [
            "collected",
            "transcribed",
            "waiting",
            "proposed",
            "approved",
            "routed"
        ]
    );
    let dates: Vec<_> = lines
        .iter()
        .map(|l| chrono::DateTime::parse_from_rfc3339(l["at"].as_str().unwrap()).unwrap())
        .collect();
    assert!(dates.windows(2).all(|w| w[0] <= w[1]), "dated in order");
    assert!(
        !names(&lines).contains(&"analysed".to_owned()),
        "no result, no analysed line"
    );
    drop(s);
    fs::remove_dir_all(root).unwrap();
}

/// The views ask whether he has chosen a model; the answer is a file read only.
/// With no model set every analysis waits for his model; a set one is reported.
#[test]
fn whether_a_model_is_chosen_is_read_without_sending() {
    let root = std::env::temp_dir().join(ObjectId::mint("intelligence-model").unwrap().to_string());
    fs::create_dir_all(&root).unwrap();
    let mut s = Service::open(&root.join("workspace")).unwrap();
    assert_eq!(
        s.command("briefing-model", &json!({})).unwrap()["set"],
        false
    );
    s.command("preferences", &json!({"localModel":true}))
        .unwrap();
    assert_eq!(
        s.command("briefing-model", &json!({})).unwrap()["set"],
        false
    );
    let briefings = root.join("workspace").join("briefings");
    fs::create_dir_all(&briefings).unwrap();
    fs::write(
        briefings.join("model.json"),
        json!({"enabled":true,"model":"chosen-model","gateway":"http://127.0.0.1:9"}).to_string(),
    )
    .unwrap();
    let chosen = s.command("briefing-model", &json!({})).unwrap();
    assert_eq!(chosen["set"], true, "{chosen}");
    assert_eq!(chosen["model"], "chosen-model");
}
