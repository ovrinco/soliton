//! Private mode hides every item marked private (itself, an ancestor or its
//! whole module) from each view's data source, and the repeats too. Each
//! source is checked present with the mode off and absent with it on.
use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::ObjectId;
use std::{fs, path::Path, path::PathBuf};

fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("soliton-privacy").unwrap().to_string());
    fs::create_dir(&p).unwrap();
    p
}
fn save(s: &mut Service, v: Value) -> String {
    s.command("save", &v).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}
fn private_mode(s: &mut Service, on: bool, modules: &[&str], marks: &[&str]) {
    s.command(
        "preferences",
        &json!({"privateMode":on,"privateModules":modules,"privateIds":marks}),
    )
    .unwrap();
}
fn read(s: &mut Service, command: &str, args: &Value) -> Value {
    // The settings in `state` list the marked ids themselves; look at records.
    if command == "state" {
        s.state().unwrap()["records"].clone()
    } else {
        s.command(command, args).unwrap()
    }
}
fn mentions(s: &mut Service, command: &str, args: &Value, text: &str) -> bool {
    read(s, command, args).to_string().contains(text)
}
/// Present with the mode off, absent with it on.
fn hides(
    s: &mut Service,
    command: &str,
    args: &Value,
    text: &str,
    modules: &[&str],
    marks: &[&str],
) {
    private_mode(s, false, modules, marks);
    assert!(
        mentions(s, command, args, text),
        "{command}: present with the mode off"
    );
    private_mode(s, true, modules, marks);
    assert!(
        !mentions(s, command, args, text),
        "{command}: absent with the mode on"
    );
    private_mode(s, false, modules, marks);
}
fn calendar_args() -> Value {
    json!({"from":"2026-10-01","through":"2026-10-10","timezone":"America/Toronto"})
}
fn scheduled(title: &str, extra: Value) -> Value {
    let mut v = json!({"kind":"task","title":title,"scheduled_for":"2026-10-05",
        "schedule":{"time":"09:00","duration_minutes":30,"timezone":"America/Toronto","fold":"earlier"}});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}
fn history_session(s: &mut Service, root: &Path) -> String {
    let source = root.join("recording");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("metadata.json"),
        json!({"id":"voice-1","title":"A real note",
        "started_at":"2026-09-21T20:40:52Z","duration_secs":30,"status":"completed",
        "chunk_count":2,"audio_retained":false})
        .to_string(),
    )
    .unwrap();
    fs::write(
        source.join("transcript.json"),
        json!({"segments":[
        {"id":0,"start_ms":0,"end_ms":30000,"text":"first","speaker_id":"You","chunk_id":0}]})
        .to_string(),
    )
    .unwrap();
    let imported = s
        .command(
            "history-import",
            &json!({"path":source,"provider":"voxtype"}),
        )
        .unwrap();
    imported["record"]["id"].as_str().unwrap().to_string()
}
/// Every planner object file except the workspace settings document.
fn object_bytes(root: &Path, skip: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for kind in fs::read_dir(root.join("objects")).unwrap() {
        for file in fs::read_dir(kind.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if !path.to_string_lossy().contains(skip) {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn privacy_default_off_persisted_and_validated() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    assert!(!s.privacy().unwrap().on);
    assert!(
        s.command("preferences", &json!({"privateMode":"yes"}))
            .is_err()
    );
    assert!(
        s.command("preferences", &json!({"privateModules":["nowhere"]}))
            .is_err()
    );
    assert!(
        s.command("preferences", &json!({"privateIds":[1]}))
            .is_err()
    );
    private_mode(&mut s, true, &[], &[]);
    drop(s);
    assert!(Service::open(&p).unwrap().privacy().unwrap().on);
}

#[test]
fn privacy_marking_leaves_object_bytes_unchanged() {
    // Rollback safety: the release before this one must read every object.
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let task = save(&mut s, scheduled("Secret", json!({})));
    let goal = save(&mut s, json!({"kind":"goal","title":"Goal"}));
    let workspace = s.state().unwrap()["records"].as_array().unwrap().len();
    assert_eq!(workspace, 2);
    let settings = ObjectId::derive("document", b"soliton-desktop-workspace-v1")
        .unwrap()
        .to_string();
    let before = object_bytes(&p, &settings);
    private_mode(&mut s, true, &["history"], &[&task, &goal]);
    let after = object_bytes(&p, &settings);
    assert_eq!(before.len(), 2);
    assert_eq!(before, after);
}

#[test]
fn privacy_item_mark_hides_state_calendar_and_versions() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let secret = save(&mut s, scheduled("Secret", json!({})));
    let open = save(&mut s, scheduled("Open", json!({})));
    for (command, args) in [
        ("state", json!({})),
        ("calendar", calendar_args()),
        ("history", json!({"id":secret})),
    ] {
        hides(&mut s, command, &args, &secret, &[], &[&secret]);
    }
    private_mode(&mut s, true, &[], &[&secret]);
    for (command, args) in [("state", json!({})), ("calendar", calendar_args())] {
        assert!(
            mentions(&mut s, command, &args, &open),
            "{command} keeps the rest"
        );
    }
}

#[test]
fn privacy_ancestor_mark_hides_children_and_reduces_goal_counts() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let category = save(&mut s, json!({"kind":"category","title":"Health"}));
    let goal = save(&mut s, json!({"kind":"goal","title":"Quiet goal"}));
    let in_category = save(
        &mut s,
        scheduled("Under category", json!({"category_id":category})),
    );
    let under_goal = save(&mut s, scheduled("Under goal", json!({"goal_id":goal})));
    let note = save(
        &mut s,
        json!({"kind":"note","title":"Cites goal","links":[goal]}),
    );
    let marks = [category.as_str(), goal.as_str()];
    for id in [&category, &goal, &in_category, &under_goal, &note] {
        hides(&mut s, "state", &json!({}), id, &[], &marks);
    }
    hides(
        &mut s,
        "calendar",
        &calendar_args(),
        &in_category,
        &[],
        &marks,
    );
    hides(
        &mut s,
        "calendar",
        &calendar_args(),
        &under_goal,
        &[],
        &marks,
    );
    // Counts: a goal that stays shown counts only the tasks still shown.
    let shown = save(&mut s, json!({"kind":"goal","title":"Shown goal"}));
    save(&mut s, scheduled("Visible task", json!({"goal_id":shown})));
    let private_task = save(&mut s, scheduled("Private task", json!({"goal_id":shown})));
    let count = |s: &mut Service| {
        s.state().unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == shown.as_str())
            .unwrap()["task_count"]
            .clone()
    };
    private_mode(&mut s, false, &[], &[&private_task]);
    assert_eq!(count(&mut s), 2);
    private_mode(&mut s, true, &[], &[&private_task]);
    assert_eq!(count(&mut s), 1);
}

#[test]
fn privacy_history_module_hides_list_detail_and_total() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let task = save(&mut s, scheduled("Ordinary", json!({})));
    let history = history_session(&mut s, &p);
    hides(
        &mut s,
        "history-list",
        &json!({}),
        &history,
        &["history"],
        &[],
    );
    hides(
        &mut s,
        "history-detail",
        &json!({"id":history}),
        "first",
        &["history"],
        &[],
    );
    private_mode(&mut s, true, &["history"], &[]);
    assert_eq!(
        s.command("history-list", &json!({})).unwrap()["total"],
        0,
        "count follows what is shown"
    );
    assert!(
        mentions(&mut s, "state", &json!({}), &task),
        "other modules untouched"
    );
    private_mode(&mut s, true, &["planner"], &[]);
    assert!(!mentions(&mut s, "state", &json!({}), &task));
    assert!(mentions(&mut s, "history-list", &json!({}), &history));
}

#[test]
fn privacy_proposals_citing_marked_goal_are_dropped_with_total() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let goal = save(&mut s, json!({"kind":"goal","title":"Private goal"}));
    let history = history_session(&mut s, &p);
    let detail = s.command("history-detail", &json!({"id":history})).unwrap();
    let revision = match &detail["revision"] {
        Value::String(r) => r.clone(),
        other => other.to_string(),
    };
    let segment = detail["segments"][0]["id"].clone();
    let base = json!({"source_id":history,"source_revision":revision,"segment_id":segment});
    let mut cited = base.clone();
    cited["title"] = json!("For the private goal");
    cited["goal_id"] = json!(goal);
    let cited = s.command("proposal-create", &cited).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut plain = base;
    plain["title"] = json!("Unrelated");
    s.command("proposal-create", &plain).unwrap();
    let list = json!({"status":"all"});
    hides(&mut s, "proposal-list", &list, &cited, &[], &[&goal]);
    hides(
        &mut s,
        "proposal-detail",
        &json!({"id":cited}),
        &goal,
        &[],
        &[&goal],
    );
    private_mode(&mut s, false, &[], &[&goal]);
    assert_eq!(s.command("proposal-list", &list).unwrap()["total"], 2);
    private_mode(&mut s, true, &[], &[&goal]);
    let shown = s.command("proposal-list", &list).unwrap();
    assert_eq!(shown["items"].as_array().unwrap().len(), 1);
    assert_eq!(shown["total"], 1, "count follows what is shown");
}

#[test]
fn privacy_briefing_citing_marked_goal_is_dropped() {
    // Briefing jobs as BriefingStore writes them (briefing.rs `start`). The
    // store itself is not opened: its worker calls a local model.
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let goal = save(&mut s, json!({"kind":"goal","title":"Private goal"}));
    let jobs = p.join("briefings/jobs");
    fs::create_dir_all(&jobs).unwrap();
    for (id, goals) in [
        ("brief-a", json!([{"id":goal,"title":"Private goal"}])),
        ("brief-b", json!([])),
    ] {
        fs::write(
            jobs.join(format!("{id}.json")),
            json!({"schema":"soliton-briefing-job/1","job_id":id,
            "state":"complete","source":{"id":"src","revision":"1","title":"Talk"},"goals":goals,
            "result":{"summary":"a summary"}})
            .to_string(),
        )
        .unwrap();
    }
    let list = json!({"items":[
        {"job_id":"brief-a","state":"complete","source_id":"src","title":"Talk"},
        {"job_id":"brief-b","state":"complete","source_id":"src","title":"Talk"}]});
    private_mode(&mut s, false, &[], &[&goal]);
    assert_eq!(
        s.privacy().unwrap().drop_briefings(&jobs, list.clone())["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    private_mode(&mut s, true, &[], &[&goal]);
    let privacy = s.privacy().unwrap();
    let shown = privacy.filter("briefing", privacy.drop_briefings(&jobs, list));
    assert_eq!(
        shown["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["job_id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("brief-b")]
    );
    let status: Value =
        serde_json::from_slice(&fs::read(jobs.join("brief-a.json")).unwrap()).unwrap();
    assert_eq!(
        privacy.filter("briefing", status),
        json!({}),
        "briefing-status of the cited job"
    );
    private_mode(&mut s, true, &["briefing"], &[]);
    let privacy = s.privacy().unwrap();
    let status: Value =
        serde_json::from_slice(&fs::read(jobs.join("brief-b.json")).unwrap()).unwrap();
    assert!(
        !privacy
            .filter("briefing", status)
            .to_string()
            .contains("a summary")
    );
}

#[test]
fn privacy_inbox_module_hides_captures() {
    use sha2::{Digest, Sha256};
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let bytes = b"original phone bytes\n";
    let id = "00000000-0000-4000-8000-000000000001";
    let sha = format!("{:x}", Sha256::digest(bytes));
    let begun = s.command("inbox-begin", &json!({"op_id":id,"filename":"private voice.wav","size":bytes.len(),"sha256":sha,"intent":"voice_note"})).unwrap();
    let transfer = begun["id"].as_str().unwrap_or(id).to_string();
    s.command("inbox-chunk", &json!({"id":transfer,"offset":0,"hex":bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),"sha256":sha})).unwrap();
    s.command("inbox-commit", &json!({"id":transfer})).unwrap();
    hides(
        &mut s,
        "inbox-list",
        &json!({}),
        "private voice",
        &["inbox"],
        &[],
    );
    hides(
        &mut s,
        "inbox-status",
        &json!({"id":transfer}),
        "private voice",
        &["inbox"],
        &[],
    );
}

#[test]
fn privacy_collection_module_hides_sources() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let transcript = p.join("private-talk.txt");
    fs::write(&transcript, "A transcript.\n").unwrap();
    s.command(
        "collection-add",
        &json!({"type":"transcript","path":transcript}),
    )
    .unwrap();
    hides(
        &mut s,
        "collection-list",
        &json!({}),
        "private-talk",
        &["collection"],
        &[],
    );
    hides(
        &mut s,
        "collection-status",
        &json!({}),
        "private-talk",
        &["collection"],
        &[],
    );
}

#[test]
fn privacy_modules_hide_jobs_and_workspace_activity() {
    let p = root();
    let jobs = p.join("modules/jobs");
    fs::create_dir_all(&jobs).unwrap();
    fs::write(jobs.join("job-private.json"), json!({"id":"job-private","module":"zetl","action":"inspect",
        "state":"succeeded","created_at":"2026-10-05T12:00:00Z","completed_at":"2026-10-05T12:00:01Z",
        "cancel_requested":false,"evidence":{"stdout":"inspected private ledger"}}).to_string()).unwrap();
    let mut s = Service::open(&p).unwrap();
    hides(
        &mut s,
        "modules-status",
        &json!({"id":"job-private"}),
        "job-private",
        &["modules"],
        &[],
    );
    hides(
        &mut s,
        "modules-list",
        &json!({}),
        "job-private",
        &["modules"],
        &[],
    );
}

#[test]
fn privacy_recovery_module_hides_checkpoints() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let file = p.join("private-note.txt");
    fs::write(&file, b"before\n").unwrap();
    s.command("recovery-checkpoint", &json!({"path":file}))
        .unwrap();
    hides(
        &mut s,
        "recovery-list",
        &json!({"path":file}),
        "private-note",
        &["recovery"],
        &[],
    );
}

#[test]
fn privacy_item_mark_hides_one_history_record_and_keeps_its_neighbours() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let secret = history_session(&mut s, &p);
    let neighbour = {
        let source = p.join("recording-2");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("metadata.json"),
            json!({"id":"voice-2","title":"Another note",
            "started_at":"2026-09-22T08:00:00Z","duration_secs":30,"status":"completed",
            "chunk_count":2,"audio_retained":false})
            .to_string(),
        )
        .unwrap();
        fs::write(
            source.join("transcript.json"),
            json!({"segments":[
            {"id":0,"start_ms":0,"end_ms":30000,"text":"second","speaker_id":"You","chunk_id":0}]})
            .to_string(),
        )
        .unwrap();
        s.command(
            "history-import",
            &json!({"path":source,"provider":"voxtype"}),
        )
        .unwrap()["record"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_ne!(secret, neighbour);
    let total = |s: &mut Service| {
        s.command("history-list", &json!({})).unwrap()["total"]
            .as_u64()
            .unwrap()
    };
    let before = total(&mut s);
    hides(&mut s, "history-list", &json!({}), &secret, &[], &[&secret]);
    hides(
        &mut s,
        "history-detail",
        &json!({"id":secret}),
        "first",
        &[],
        &[&secret],
    );
    private_mode(&mut s, true, &[], &[&secret]);
    assert!(
        mentions(&mut s, "history-list", &json!({}), &neighbour),
        "the rest of History stays"
    );
    assert!(mentions(
        &mut s,
        "history-detail",
        &json!({"id":neighbour}),
        "second"
    ));
    assert_eq!(total(&mut s), before - 1, "count follows what is shown");
    private_mode(&mut s, false, &[], &[&secret]);
    assert!(
        mentions(&mut s, "history-list", &json!({}), &secret),
        "returns with the mode off"
    );
    assert_eq!(total(&mut s), before);
    private_mode(&mut s, true, &["history"], &[&secret]);
    assert!(
        !mentions(&mut s, "history-list", &json!({}), &neighbour),
        "a module mark still hides everything"
    );
    assert_eq!(total(&mut s), 0);
}

#[test]
fn privacy_settings_saved_back_while_on_keep_every_mark() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let task = save(&mut s, scheduled("Secret", json!({})));
    save(&mut s, scheduled("Open", json!({})));
    private_mode(&mut s, true, &["history"], &[&task, "record-x"]);
    let shown = s.command("state", &json!({})).unwrap()["ui"].clone();
    assert_eq!(
        shown["privateIds"],
        json!([]),
        "the read shows no marks while the mode is on"
    );
    s.command("preferences", &shown).unwrap();
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    let kept = s.command("state", &json!({})).unwrap()["ui"].clone();
    assert_eq!(kept["privateIds"], json!([task, "record-x"]));
    assert_eq!(kept["privateModules"], json!(["history"]));
    s.command("preferences", &json!({"privateIds":[]})).unwrap();
    assert_eq!(
        s.command("state", &json!({})).unwrap()["ui"]["privateIds"],
        json!([]),
        "with the mode off a clear is a clear"
    );
}

/// The window reads History through the background queue; a mark hides the
/// record there as it does in a direct read.
#[test]
fn privacy_item_mark_hides_a_history_record_read_through_the_queue() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let secret = history_session(&mut s, &p);
    let queued = |s: &mut Service| {
        let id = s
            .command(
                "history-job-start",
                &json!({"command":"history-list","args":{}}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        for _ in 0..500 {
            let status = s.command("history-job-status", &json!({"id":id})).unwrap();
            if status["state"] == "complete" {
                return status["result"].to_string();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("history job did not finish");
    };
    assert!(queued(&mut s).contains(&secret));
    private_mode(&mut s, true, &[], &[&secret]);
    assert!(!queued(&mut s).contains(&secret), "hidden with the mode on");
    private_mode(&mut s, false, &[], &[&secret]);
    assert!(queued(&mut s).contains(&secret), "back with the mode off");
}

/// A page of History that does not hold the hidden record still counts
/// without it.
#[test]
fn privacy_item_mark_lowers_the_total_on_every_history_page() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let secret = history_session(&mut s, &p);
    let source = p.join("recording-2");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("metadata.json"),
        json!({"id":"voice-2","title":"Another note",
        "started_at":"2030-01-01T08:00:00Z","duration_secs":30,"status":"completed",
        "chunk_count":2,"audio_retained":false})
        .to_string(),
    )
    .unwrap();
    fs::write(
        source.join("transcript.json"),
        json!({"segments":[
        {"id":0,"start_ms":0,"end_ms":30000,"text":"second","speaker_id":"You","chunk_id":0}]})
        .to_string(),
    )
    .unwrap();
    s.command(
        "history-import",
        &json!({"path":source,"provider":"voxtype"}),
    )
    .unwrap();
    let page = |s: &mut Service, offset: u64| {
        s.command("history-list", &json!({"offset":offset,"limit":1}))
            .unwrap()
    };
    assert_eq!(page(&mut s, 0)["total"], 2);
    private_mode(&mut s, true, &[], &[&secret]);
    let first = page(&mut s, 0);
    assert_eq!(
        first["total"], 1,
        "the first page counts without the hidden record"
    );
    assert!(!first.to_string().contains(&secret));
    assert!(!page(&mut s, 1).to_string().contains(&secret));
}

#[test]
fn privacy_last_mark_lifts_with_the_mode_on_but_a_plain_save_never_wipes() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let id = save(&mut s, json!({"kind":"task","title":"Marked task"}));
    private_mode(&mut s, true, &["kitchen"], &[&id]);
    let marks = |s: &mut Service| {
        let r = s
            .command("preferences", &json!({"privateMode":true}))
            .unwrap();
        (r["privateModules"].clone(), r["privateIds"].clone())
    };
    // A plain save of filtered settings (empty lists) keeps every mark.
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":[],"privateIds":[],"timezone":"UTC"}),
    )
    .unwrap();
    assert_eq!(
        marks(&mut s),
        (json!(["kitchen"]), json!([id])),
        "a plain save with the mode on keeps the marks"
    );
    // The marks written themselves may lift the last one.
    s.command(
        "preferences",
        &json!({"replaceMarks":true,"privateModules":[]}),
    )
    .unwrap();
    s.command("preferences", &json!({"replaceMarks":true,"privateIds":[]}))
        .unwrap();
    assert_eq!(
        marks(&mut s),
        (json!([]), json!([])),
        "the last marks lift with the mode on"
    );
    let stored = s.command("preferences", &json!({})).unwrap();
    assert!(
        stored.get("replaceMarks").is_none(),
        "the flag is not stored"
    );
}
