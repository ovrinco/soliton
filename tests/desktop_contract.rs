use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::*;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
};
fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("soliton-test").unwrap().to_string());
    fs::create_dir(&p).unwrap();
    p
}
fn save(s: &mut Service, v: Value) -> Value {
    s.command("save", &v).unwrap()
}
fn latest(s: &Service, id: &str) -> Value {
    s.state().unwrap()["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap()
        .clone()
}
#[test]
fn recurring_three_states_are_exact_persistent_and_conflict_checked() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let task = save(
        &mut s,
        json!({"kind":"task","title":"Occurrence states",
        "scheduled_for":"2026-03-07","recurrence":"daily","repeat_until":"2026-03-10",
        "schedule":{"time":"09:00","duration_minutes":30,"timezone":"America/Toronto","fold":"earlier"}}),
    );
    let events = s
        .calendar("2026-03-07", "2026-03-10", "America/Toronto")
        .unwrap();
    let chosen = events[1].clone();
    let status_args = |hash: &Value, status: Value| {
        json!({"id":task["id"],"hash":hash,
        "occurrence":chosen["occurrence"],"occurrence_date":chosen["occurrence_date"],"status":status})
    };
    let progress = s
        .command(
            "occurrence",
            &status_args(&task["hash"], json!("in_progress")),
        )
        .unwrap();
    assert_eq!(progress["status"], "todo");
    assert!(
        s.command(
            "occurrence",
            &status_args(&task["hash"], json!("completed"))
        )
        .is_err()
    );
    for invalid in [
        json!("archived"),
        json!("unknown"),
        json!(null),
        json!(true),
    ] {
        assert!(
            s.command("occurrence", &status_args(&progress["hash"], invalid))
                .is_err()
        );
        assert_eq!(
            latest(&s, task["id"].as_str().unwrap())["hash"],
            progress["hash"]
        );
    }
    let completed = s
        .command(
            "occurrence",
            &status_args(&progress["hash"], json!("completed")),
        )
        .unwrap();
    drop(s);
    let mut s = Service::open(&p).unwrap();
    let events = s
        .calendar("2026-03-07", "2026-03-10", "America/Toronto")
        .unwrap();
    assert_eq!(events[1]["status"], "completed");
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .all(|(i, e)| i == 1 || e["status"] == "todo")
    );
    let reopened = s
        .command(
            "complete",
            &json!({"id":task["id"],"hash":completed["hash"],
        "occurrence":chosen["occurrence"],"occurrence_date":chosen["occurrence_date"]}),
        )
        .unwrap();
    assert_eq!(
        s.calendar("2026-03-07", "2026-03-10", "America/Toronto")
            .unwrap()[1]["status"],
        "in_progress"
    );
    s.command("occurrence", &status_args(&reopened["hash"], json!("todo")))
        .unwrap();
    assert!(
        s.calendar("2026-03-07", "2026-03-10", "America/Toronto")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["status"] == "todo")
    );
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn one_object_survives_reparenting_status_time_and_restart() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let health = save(
        &mut s,
        json!({"kind":"category","title":"Health","color":"#83b6a0"}),
    );
    let business = save(
        &mut s,
        json!({"kind":"category","title":"Business","color":"#85a9d2"}),
    );
    let g = save(
        &mut s,
        json!({"kind":"goal","title":"A goal","category_id":health["id"]}),
    );
    let t = save(
        &mut s,
        json!({"kind":"task","title":"One task","goal_id":g["id"],"body":"Original","scheduled_for":"2026-09-21","schedule":{"time":"23:30","duration_minutes":120,"timezone":"America/Toronto","fold":"earlier"}}),
    );
    let changed = save(
        &mut s,
        json!({"id":t["id"],"hash":t["hash"],"status":"in_progress","body":"Edited"}),
    );
    assert!(
        s.command(
            "save",
            &json!({"id":t["id"],"hash":t["hash"],"title":"Stale"})
        )
        .is_err()
    );
    save(
        &mut s,
        json!({"id":g["id"],"hash":g["hash"],"category_id":business["id"]}),
    );
    assert_eq!(
        latest(&s, t["id"].as_str().unwrap())["category_id"],
        business["id"]
    );
    let calendar = s
        .command(
            "calendar",
            &json!({"from":"2026-09-21","through":"2026-09-22","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(calendar[0]["id"], t["id"]);
    assert_eq!(calendar[0]["local_end"], "2026-09-22T01:30");
    let notes = save(
        &mut s,
        json!({"kind":"note","title":"Context","body":"Real note","links":[t["id"]]}),
    );
    s.command(
        "preferences",
        &json!({"drafts":{"unfinished":{"title":"Not yet saved"}},"selectedId":t["id"]}),
    )
    .unwrap();
    drop(s);
    let s = Service::open(&p).unwrap();
    assert_eq!(latest(&s, t["id"].as_str().unwrap())["body"], "Edited");
    assert_eq!(
        latest(&s, notes["id"].as_str().unwrap())["links"][0],
        t["id"]
    );
    assert_eq!(
        s.preferences().unwrap()["drafts"]["unfinished"]["title"],
        "Not yet saved"
    );
    assert_eq!(
        latest(&s, t["id"].as_str().unwrap())["hash"],
        changed["hash"]
    );
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn recurrence_completion_override_and_rule_history_are_separate() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Practice","scheduled_for":"2026-03-07","recurrence":"daily","schedule":{"time":"09:00","duration_minutes":30,"timezone":"America/Toronto","fold":"earlier"}}),
    );
    let calendar = s
        .command(
            "calendar",
            &json!({"from":"2026-03-07","through":"2026-03-09","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(calendar.as_array().unwrap().len(), 3);
    assert_eq!(
        calendar[1]["start_utc"].as_i64().unwrap() - calendar[0]["start_utc"].as_i64().unwrap(),
        23 * 3600
    );
    let occurrence = &calendar[1];
    let done=s.command("complete",&json!({"id":t["id"],"hash":t["hash"],"occurrence":occurrence["occurrence"],"occurrence_date":occurrence["occurrence_date"]})).unwrap();
    assert_eq!(done["completed_count"], 1);
    let moved=s.command("occurrence",&json!({"id":t["id"],"hash":done["hash"],"occurrence":occurrence["occurrence"],"occurrence_date":occurrence["occurrence_date"],"date":"2026-03-10","schedule":{"time":"11:15","duration_minutes":45,"timezone":"America/Toronto","fold":"earlier"}})).unwrap();
    let calendar = s
        .command(
            "calendar",
            &json!({"from":"2026-03-07","through":"2026-03-10","timezone":"America/Toronto"}),
        )
        .unwrap();
    let item = calendar
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["occurrence"] == occurrence["occurrence"])
        .unwrap();
    assert_eq!(item["local_start"], "2026-03-10T11:15");
    assert_eq!(item["status"], "completed");
    let revised = save(
        &mut s,
        json!({"id":t["id"],"hash":moved["hash"],"recurrence":"weekly"}),
    );
    assert_eq!(
        revised["series_history"][0]["completed"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        revised["series_history"][0]["overrides"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !p.join("recurrence")
            .join(format!("{}.cursor", t["id"].as_str().unwrap()))
            .exists()
    );
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn interrupted_extended_write_recovers_all_fields_and_history_restores() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Before","body":"Body before"}),
    );
    let id: ObjectId = t["id"].as_str().unwrap().parse().unwrap();
    let stored = s.snapshot.get(&id).unwrap();
    let mut next = stored.entity.clone();
    next.meta_mut().title = "After".into();
    next.meta_mut().details.body = "Body after".into();
    next.meta_mut().revision += 1;
    let pending = s
        .repo
        .prepare(next, MutationPrecondition::Matches(stored.content_hash))
        .unwrap();
    drop(pending);
    drop(s);
    let mut s = Service::open(&p).unwrap();
    assert_eq!(s.recovered, 1);
    let after = latest(&s, id.as_str());
    assert_eq!(after["title"], "After");
    assert_eq!(after["body"], "Body after");
    let restored = s
        .command(
            "restore",
            &json!({"id":id.as_str(),"hash":after["hash"],"version":t["hash"]}),
        )
        .unwrap();
    assert_eq!(restored["body"], "Body before");
    assert_eq!(restored["revision"], 3);
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn actual_second_process_cannot_write_and_lock_releases_after_exit() {
    let p = root();
    let exe = env!("CARGO_BIN_EXE_soliton");
    let mut first = Command::new(exe)
        .arg(&p)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    first
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"id\":1,\"command\":\"state\",\"args\":{}}\n")
        .unwrap();
    let mut line = String::new();
    BufReader::new(first.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&line).unwrap()["ok"]
            .as_bool()
            .unwrap()
    );
    let second = Command::new(exe)
        .arg(&p)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already open"));
    first.kill().unwrap();
    first.wait().unwrap();
    let third = Command::new(exe)
        .arg(&p)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(third.status.success());
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn due_only_archiving_and_corruption_do_not_invent_or_erase_work() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let c = save(&mut s, json!({"kind":"category","title":"Health"}));
    let g = save(
        &mut s,
        json!({"kind":"goal","title":"Walk","category_id":c["id"]}),
    );
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Deadline","goal_id":g["id"],"due_on":"2026-09-21"}),
    );
    let cal = s
        .command(
            "calendar",
            &json!({"from":"2026-09-21","through":"2026-09-21","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(cal.as_array().unwrap().len(), 1);
    assert_eq!(cal[0]["event_kind"], "due");
    save(
        &mut s,
        json!({"id":c["id"],"hash":c["hash"],"archived":true}),
    );
    assert_eq!(latest(&s, t["id"].as_str().unwrap())["hidden"], true);
    let file = PathBuf::from(t["path"].as_str().unwrap());
    drop(s);
    fs::write(&file, b"damaged").unwrap();
    assert!(Service::open(&p).is_err());
    assert_eq!(fs::read(file).unwrap(), b"damaged");
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn occurrence_can_become_all_day_and_move_in_from_outside_the_calendar_range() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Repeat","scheduled_for":"2026-01-01","recurrence":"daily","schedule":{"time":"09:00","duration_minutes":60,"timezone":"America/Toronto","fold":"earlier"}}),
    );
    let cal = s
        .command(
            "calendar",
            &json!({"from":"2026-12-01","through":"2026-12-01","timezone":"America/Toronto"}),
        )
        .unwrap();
    let o = &cal[0];
    s.command("occurrence",&json!({"id":t["id"],"hash":t["hash"],"occurrence":o["occurrence"],"occurrence_date":o["occurrence_date"],"date":"2026-01-15","schedule":null})).unwrap();
    let cal = s
        .command(
            "calendar",
            &json!({"from":"2026-01-15","through":"2026-01-15","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(cal.as_array().unwrap().len(), 2);
    let moved = cal
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["occurrence"] == o["occurrence"])
        .unwrap();
    assert_eq!(moved["event_kind"], "all_day");
    assert!(moved["schedule"].is_null());
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn invalid_numbers_and_whole_series_completion_do_not_mutate() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    for duration in [json!(-5), json!(4_294_967_356u64), json!(2.5)] {
        assert!(s.command("save",&json!({"kind":"task","title":"Bad duration","scheduled_for":"2026-09-21","schedule":{"time":"09:00","duration_minutes":duration,"timezone":"America/Toronto","fold":"earlier"}})).is_err());
    }
    assert!(s.command("save",&json!({"kind":"task","title":"Bad repeat","scheduled_for":"2026-09-21","recurrence":"daily","interval":-1})).is_err());
    assert!(s.command("save",&json!({"kind":"task","title":"Wrong completion","scheduled_for":"2026-09-21","recurrence":"daily","status":"completed"})).is_err());
    assert_eq!(s.state().unwrap()["records"].as_array().unwrap().len(), 0);
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn transfer_preserves_ids_links_revisions_and_never_replaces_a_destination() {
    let p = root();
    let mut s = Service::open(&p.join("source")).unwrap();
    let c = save(&mut s, json!({"kind":"category","title":"Health"}));
    let g = save(
        &mut s,
        json!({"kind":"goal","title":"Walk","category_id":c["id"]}),
    );
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Before","goal_id":g["id"]}),
    );
    let t = save(
        &mut s,
        json!({"id":t["id"],"hash":t["hash"],"title":"After"}),
    );
    save(
        &mut s,
        json!({"kind":"note","title":"Context","links":[t["id"]]}),
    );
    let file = p.join("export.json");
    s.command("export", &json!({"path":file})).unwrap();
    let before = fs::read(&file).unwrap();
    let preview = s.command("import_preview", &json!({"path":file})).unwrap();
    let dest = p.join("destination");
    assert!(
        s.command(
            "import_workspace",
            &json!({"path":file,"hash":"stale","target":dest})
        )
        .is_err()
    );
    assert!(!dest.exists());
    s.command(
        "import_workspace",
        &json!({"path":file,"hash":preview["hash"],"target":dest}),
    )
    .unwrap();
    let restored = Service::open(&dest).unwrap();
    assert_eq!(
        latest(&restored, t["id"].as_str().unwrap())["hash"],
        t["hash"]
    );
    assert_eq!(
        restored.state().unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    drop(restored);
    assert!(
        s.command(
            "import_workspace",
            &json!({"path":file,"hash":preview["hash"],"target":dest})
        )
        .is_err()
    );
    assert_eq!(before, fs::read(&file).unwrap());
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn oversized_input_is_rejected_and_the_next_request_still_works() {
    let p = root();
    let mut child = Command::new(env!("CARGO_BIN_EXE_soliton"))
        .arg(&p)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&vec![b'x'; 2_000_020]).unwrap();
    input
        .write_all(b"\n{\"id\":2,\"command\":\"state\",\"args\":{}}\n")
        .unwrap();
    drop(input);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["ok"], false);
    assert_eq!(replies[1]["id"], 2);
    assert_eq!(replies[1]["ok"], true);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn unsupported_workspace_schema_fails_before_serving_and_preserves_source() {
    let p = root();
    let s = Service::open(&p).unwrap();
    let stored = s
        .snapshot
        .entities()
        .find(|e| matches!(e,Entity::Document(d) if d.document_type=="workspace"))
        .unwrap();
    let mut future = stored.clone();
    future.meta_mut().details.body = "{\"schema\":99}".into();
    future.meta_mut().revision += 1;
    s.repo
        .update(future, stored.content_hash().unwrap())
        .unwrap();
    drop(s);
    assert!(Service::open(&p).is_err());
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn card_reordering_preserves_categories_and_survives_dense_ranks_and_restart() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let a = save(&mut s, json!({"kind":"category","title":"A","order":1}));
    let b = save(&mut s, json!({"kind":"category","title":"B","order":2}));
    let c = save(&mut s, json!({"kind":"category","title":"C","order":3}));
    let moved = s
        .command(
            "reorder",
            &json!({"id":c["id"],"hash":c["hash"],"target":a["id"],"after":true}),
        )
        .unwrap();
    let aa = latest(&s, a["id"].as_str().unwrap());
    let bb = latest(&s, b["id"].as_str().unwrap());
    assert!(aa["order"].as_i64().unwrap() < moved["order"].as_i64().unwrap());
    assert!(moved["order"].as_i64().unwrap() < bb["order"].as_i64().unwrap());
    assert!(
        s.command(
            "reorder",
            &json!({"id":c["id"],"hash":c["hash"],"target":a["id"]})
        )
        .is_err()
    );
    let ga = save(
        &mut s,
        json!({"kind":"goal","title":"Goal A","category_id":a["id"]}),
    );
    let gb = save(
        &mut s,
        json!({"kind":"goal","title":"Goal B","category_id":b["id"]}),
    );
    let task = save(
        &mut s,
        json!({"kind":"task","title":"Attached","goal_id":ga["id"],"body":"Preserve me"}),
    );
    s.command(
        "reorder",
        &json!({"id":ga["id"],"hash":ga["hash"],"target":gb["id"]}),
    )
    .unwrap();
    assert_eq!(
        latest(&s, task["id"].as_str().unwrap())["category_id"],
        a["id"]
    );
    assert_eq!(
        latest(&s, task["id"].as_str().unwrap())["body"],
        "Preserve me"
    );
    drop(s);
    let s = Service::open(&p).unwrap();
    assert_eq!(
        latest(&s, ga["id"].as_str().unwrap())["category_id"],
        a["id"]
    );
    drop(s);
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn external_owner_updates_reconcile_documents_through_operations_log() {
    let p = root();
    let mut service = Service::open(&p).unwrap();
    let _workspace_id = service
        .snapshot
        .entities()
        .find_map(|entity| match entity {
            Entity::Document(document) if document.document_type == "workspace" => {
                Some(document.meta.id.clone())
            }
            _ => None,
        })
        .unwrap();
    let note = save(
        &mut service,
        json!({"kind":"note","title":"External note","body":"Before"}),
    );
    let note_id: ObjectId = note["id"].as_str().unwrap().parse().unwrap();
    let before = service.snapshot.get(&note_id).unwrap().clone();
    let mut external = before.entity.clone();
    external.meta_mut().details.body = external
        .meta()
        .details
        .body
        .replace("Action Center", "Externally reconciled");
    external.meta_mut().revision += 1;

    // GovernedRepository is the owner contract used by the separate writer;
    // its commit stages and atomically installs the same canonical object path
    // before appending journal/operations.log.
    let owner = GovernedRepository::open(&p).unwrap();
    owner.update(external, before.content_hash).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    let pushed = service.check_external_freshness().unwrap().unwrap();
    let pushed_record = pushed["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == note_id.as_str())
        .unwrap();
    assert_eq!(pushed_record["revision"], 2);
    assert_eq!(pushed["ui"]["view"], "Calendar");

    let latest = service.snapshot.get(&note_id).unwrap().clone();
    let mut second = latest.entity.clone();
    second.meta_mut().details.body = second
        .meta()
        .details
        .body
        .replace("Externally reconciled", "Atomic replacement observed");
    second.meta_mut().revision += 1;
    owner.update(second, latest.content_hash).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    let replaced = service.check_external_freshness().unwrap().unwrap();
    let replaced_record = replaced["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == note_id.as_str())
        .unwrap();
    assert_eq!(replaced_record["revision"], 3);
    drop(owner);
    drop(service);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn help_and_version_touch_nothing_and_a_missing_workspace_is_refused() {
    let scratch = root();
    let exe = env!("CARGO_BIN_EXE_soliton");
    let run = |args: &[&str]| {
        Command::new(exe)
            .args(args)
            .current_dir(&scratch)
            .env_remove("SOLITON_RESOURCE_ROOT")
            .output()
            .unwrap()
    };
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("Usage: soliton DATA_DIRECTORY"));
    let version = run(&["--version"]);
    assert!(version.status.success());
    let version = String::from_utf8_lossy(&version.stdout).to_string();
    assert!(version.starts_with(&format!("soliton {}", env!("CARGO_PKG_VERSION"))));
    assert!(
        version.trim_end().ends_with(&format!(
            "workspace format {}",
            soliton::service::WORKSPACE_FORMAT
        )),
        "{version}"
    );
    let missing = scratch.join("not-a-workspace");
    let refused = run(&[missing.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&refused.stderr).lines().count(), 1);
    let unknown = run(&["--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(
        fs::read_dir(&scratch).unwrap().count(),
        0,
        "nothing created beside the call"
    );
    fs::remove_dir_all(scratch).unwrap();
}
fn state_release(resource_root: &std::path::Path, workspace: &std::path::Path) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_soliton"))
        .arg(workspace)
        .env("SOLITON_RESOURCE_ROOT", resource_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"id\":1,\"command\":\"state\",\"args\":{}}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let reply: Value = serde_json::from_str(
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    reply["result"]["release"].clone()
}
#[test]
fn state_names_the_running_build_and_whether_it_is_the_build_on_disk() {
    use sha2::{Digest, Sha256};
    let exe = env!("CARGO_BIN_EXE_soliton");
    let hash = format!("{:x}", Sha256::digest(fs::read(exe).unwrap()));
    let version = env!("CARGO_PKG_VERSION");
    let (release, ws) = (root(), root());
    let write =
        |record: Value| fs::write(release.join("RELEASE.json"), record.to_string()).unwrap();
    write(
        json!({"release":format!("{version}-wave3-20261030-abc1234"),"status":"INSTALLED",
        "source_commit":"abc1234","binary_sha256":hash,"built_on":"2026-10-30"}),
    );
    let identity = state_release(&release, &ws);
    for key in [
        "name",
        "version",
        "built_on",
        "source_commit",
        "matches_disk",
    ] {
        assert!(identity.get(key).is_some(), "release.{key} missing");
    }
    assert_eq!(
        identity["name"],
        format!("{version}-wave3-20261030-abc1234")
    );
    assert_eq!(identity["version"], version);
    assert_eq!(identity["built_on"], "2026-10-30");
    assert_eq!(identity["source_commit"], "abc1234");
    assert_eq!(identity["matches_disk"], true);
    // Another binary's hash, or a record of another version, is not this build.
    write(json!({"release":format!("{version}-x"),"binary_sha256":"0".repeat(64)}));
    assert_eq!(state_release(&release, &ws)["matches_disk"], false);
    write(json!({"release":"0.0.1-x","binary_sha256":hash}));
    assert_eq!(state_release(&release, &ws)["matches_disk"], false);
    // The source tree's template claims no install and matches no binary.
    write(json!({"release":"source","version":version,"status":"SOURCE"}));
    let source = state_release(&release, &ws);
    assert_eq!(source["matches_disk"], false);
    assert!(
        source["built_on"].is_string(),
        "the executable's date stands in for an unrecorded build date"
    );
    fs::remove_dir_all(release).unwrap();
    fs::remove_dir_all(ws).unwrap();
}
#[test]
fn opening_a_workspace_makes_no_backup_and_no_checkpoint() {
    fn names(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            out.push(entry.file_name().to_string_lossy().to_lowercase());
            if entry.file_type().unwrap().is_dir() {
                names(&entry.path(), out);
            }
        }
    }
    let p = root();
    for _ in 0..2 {
        let mut s = Service::open(&p).unwrap();
        save(&mut s, json!({"kind":"task","title":"Opened twice"}));
        s.state().unwrap();
        drop(s);
    }
    let mut all = Vec::new();
    names(&p, &mut all);
    assert!(
        !all.iter()
            .any(|n| n.contains("backup") || n.contains("checkpoint")),
        "{all:?}"
    );
    assert!(
        !p.join("recovery").exists(),
        "recovery opens only when asked"
    );
    fs::remove_dir_all(p).unwrap();
}
fn tree(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            let name = path.strip_prefix(dir).unwrap().display().to_string();
            if entry.file_type().unwrap().is_dir() {
                out.push((name + "/", Vec::new()));
                stack.push(path);
            } else {
                out.push((name, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}
#[test]
fn a_workspace_of_a_newer_format_is_refused_in_one_line_and_left_byte_identical() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    save(
        &mut s,
        json!({"kind":"task","title":"Written by a newer build"}),
    );
    drop(s);
    let future = soliton::service::WORKSPACE_FORMAT + 1;
    fs::write(
        p.join("journal/format.json"),
        json!({"format":future,"written_by":"9.9.9"}).to_string(),
    )
    .unwrap();
    let before = tree(&p);
    let modified = fs::metadata(p.join("journal/operations.log"))
        .unwrap()
        .modified()
        .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_soliton"))
        .arg(&p)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.contains("newer Soliton") && stderr.contains("changed nothing"),
        "{stderr}"
    );
    assert!(out.stdout.is_empty());
    assert!(Service::open(&p).is_err());
    assert_eq!(tree(&p), before, "the refused workspace is byte-identical");
    assert_eq!(
        fs::metadata(p.join("journal/operations.log"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn the_format_mark_comes_with_the_first_write_never_with_open_or_idle() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    save(&mut s, json!({"kind":"task","title":"Before the mark"}));
    drop(s);
    // A workspace from before the mark: reads as format 0 and opens.
    let _ = fs::remove_file(p.join("journal/format.json"));
    assert_eq!(soliton::service::workspace_format(&p).unwrap(), 0);
    let mut s = Service::open(&p).unwrap();
    s.state().unwrap();
    s.command("state", &json!({})).unwrap();
    assert!(
        !p.join("journal/format.json").exists(),
        "open, a read and idle write no mark"
    );
    save(&mut s, json!({"kind":"task","title":"The first write"}));
    let mark: Value =
        serde_json::from_slice(&fs::read(p.join("journal/format.json")).unwrap()).unwrap();
    assert_eq!(mark["format"], soliton::service::WORKSPACE_FORMAT);
    assert_eq!(mark["written_by"], env!("CARGO_PKG_VERSION"));
    drop(s);
    // A marked workspace of this format opens and is not stamped again per launch.
    let stamped = fs::metadata(p.join("journal/format.json"))
        .unwrap()
        .modified()
        .unwrap();
    let mut s = Service::open(&p).unwrap();
    save(&mut s, json!({"kind":"task","title":"After the mark"}));
    drop(s);
    assert_eq!(
        fs::metadata(p.join("journal/format.json"))
            .unwrap()
            .modified()
            .unwrap(),
        stamped
    );
    fs::remove_dir_all(p).unwrap();
}

// The desktop module: a binding changed where it lives, on a scratch
// bindings.lua and the recorded desktop; nothing reaches the compositor.
mod binding {
    use sha2::{Digest, Sha256};
    use soliton::desktop::{BINDINGS_PREVIOUS, BindingRequest, Desktop};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    const PLANTED: &str = "-- scratch override\no.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh your-server\")\no.bind(\"SUPER + SHIFT + Y\", \"Notes\", \"notes\")\n";

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    fn scratch(label: &str) -> Desktop {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("t78-binding-{label}-{}", std::process::id()));
        let config = dir.join("hypr");
        let defaults = dir.join("defaults");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&defaults).unwrap();
        fs::write(config.join("bindings.lua"), PLANTED).unwrap();
        Desktop {
            config,
            defaults,
            live: false,
            record: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/desktop/two-outputs-2026-10-28.json"),
            ),
        }
    }
    fn request(desktop: &Desktop, new: &str) -> BindingRequest {
        BindingRequest {
            old: "SUPER + SHIFT + R".into(),
            description: "SSH".into(),
            new: new.into(),
            revision: desktop.bindings().unwrap()["revision"]
                .as_str()
                .unwrap()
                .into(),
        }
    }

    #[test]
    fn the_bindings_read_names_each_chord_and_its_literal_source() {
        let desktop = scratch("read");
        let read = desktop.bindings().unwrap();
        let items = read["items"].as_array().unwrap();
        let row = |action: &str| {
            items
                .iter()
                .find(|r| r["description"] == action)
                .unwrap()
                .clone()
        };
        assert_eq!(row("SSH")["combo"], "SUPER + SHIFT + R");
        assert_eq!(row("SSH")["editable"], true);
        assert_eq!(row("SSH")["line"], 2);
        assert_eq!(
            row("Volume up")["editable"],
            false,
            "no literal source, a repeat key"
        );
        assert_eq!(read["keyboard"]["name"], "recorded keyboard");
        assert_eq!(read["revision"], sha(PLANTED.as_bytes()));
    }

    #[test]
    fn a_changed_chord_appends_the_unbind_and_the_bind_and_keeps_the_previous_bytes() {
        let desktop = scratch("change");
        let path = desktop.config.join("bindings.lua");
        let original = fs::read(&path).unwrap();
        let receipt = desktop
            .set_binding(&request(&desktop, "super + shift + t"))
            .unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        let appended = &updated[original.len()..];
        println!("appended:\n{appended}");
        assert!(
            updated.as_bytes().starts_with(&original),
            "every earlier byte is kept"
        );
        assert_eq!(
            appended,
            "\nhl.unbind(\"SUPER + SHIFT + R\")\no.bind(\"SUPER + SHIFT + T\", \"SSH\", \"alacritty -e ssh your-server\")\n"
        );
        let previous = fs::read(desktop.config.join(BINDINGS_PREVIOUS)).unwrap();
        println!("original {} previous {}", sha(&original), sha(&previous));
        assert_eq!(sha(&previous), sha(&original));
        assert_eq!(receipt.previous_revision, sha(&original));
        assert_eq!(receipt.revision, sha(updated.as_bytes()));
        assert!(!receipt.applied, "not live: the compositor is not touched");
        assert_eq!(
            receipt.summary,
            "SSH: SUPER + SHIFT + R → SUPER + SHIFT + T"
        );
    }

    #[test]
    fn a_chord_bound_to_another_action_is_refused_with_that_action_and_nothing_written() {
        let desktop = scratch("conflict");
        let path = desktop.config.join("bindings.lua");
        let error = desktop
            .set_binding(&request(&desktop, "SHIFT + SUPER + Y"))
            .unwrap_err()
            .to_string();
        println!("refused: {error}");
        assert!(error.contains("Notes"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), PLANTED);
        assert!(!desktop.config.join(BINDINGS_PREVIOUS).exists());
    }

    #[test]
    fn an_unknown_modifier_a_stale_revision_and_a_binding_with_no_source_are_refused() {
        let desktop = scratch("refusals");
        let path = desktop.config.join("bindings.lua");
        let hyper = desktop
            .set_binding(&request(&desktop, "HYPER + T"))
            .unwrap_err()
            .to_string();
        assert!(hyper.contains("modifiers"), "{hyper}");
        let mut stale = request(&desktop, "SUPER + SHIFT + T");
        stale.revision = sha(b"another file");
        assert!(
            desktop
                .set_binding(&stale)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        let mut volume = request(&desktop, "SUPER + V");
        volume.old = "XF86AudioRaiseVolume".into();
        volume.description = "Volume up".into();
        assert!(
            desktop
                .set_binding(&volume)
                .unwrap_err()
                .to_string()
                .contains("editable")
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            PLANTED,
            "3 of 3 refusals wrote nothing"
        );
    }
}
