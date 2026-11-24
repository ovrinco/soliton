use serde_json::{Value, json};
use soliton::briefing::{BriefingStore, StoreTools, map_relevance, model_setting};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("soliton-briefing-{stamp}"));
    fs::create_dir_all(&path).unwrap();
    path
}

/// The store every test opens unless it brings its own client: its client is a
/// fixture that records what it was asked to send and sends nothing, so no test
/// can reach his gateway on 8080 or 8092 whatever setting it writes.
fn fixture_store(dir: &Path) -> BriefingStore {
    let client = dir.join("fixture-client.sh");
    fs::write(
        &client,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in http*) echo \"refused $a\" >> '{}';; esac; done\nexit 7\n",
            dir.join("fixture-client.log").display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&client).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&client, permissions).unwrap();
    let tools = StoreTools {
        client,
        stack_map: stack_fixture(dir),
    };
    BriefingStore::open_with(dir, tools).unwrap()
}

fn source() -> serde_json::Value {
    json!({
        "id":"source-one",
        "revision":"revision-one",
        "title":"Bounded notes",
        "segments":[{"id":"segment-one","text":"Review the selected note."}]
    })
}

#[test]
fn selected_segment_outside_snapshot_is_rejected_before_queueing() {
    let dir = root();
    let mut store = fixture_store(&dir);
    let result = store.command(
        "briefing-start",
        &json!({"source":source(),"goals":[],"selected_segment_ids":["forged-segment"]}),
    );
    assert!(result.is_err());
    assert!(fs::read_dir(dir.join("jobs")).unwrap().next().is_none());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn status_rejects_untrusted_job_id_and_cancel_is_durable() {
    let dir = root();
    let mut store = fixture_store(&dir);
    assert!(
        store
            .command("briefing-status", &json!({"job_id":"../job"}))
            .is_err()
    );
    let queued = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"selected_segment_ids":["segment-one"]}),
        )
        .unwrap();
    let id = queued["job_id"].as_str().unwrap();
    let cancelled = store
        .command("briefing-cancel", &json!({"job_id":id}))
        .unwrap();
    assert!(matches!(
        cancelled["state"].as_str(),
        Some("cancel_requested") | Some("cancelled") | Some("failed")
    ));
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn list_returns_latest_durable_job_for_selected_source() {
    let dir = root();
    let mut store = fixture_store(&dir);
    let queued = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"selected_segment_ids":["segment-one"]}),
        )
        .unwrap();
    let listed = store
        .command(
            "briefing-list",
            &json!({"source_id":"source-one","limit":1}),
        )
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["items"][0]["job_id"], queued["job_id"]);
    // No model setting: the job is recorded and waits; nothing is sent.
    assert_eq!(listed["items"][0]["state"], "waiting");
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn request_key_reuses_exact_job_and_rejects_conflicting_payload() {
    let dir = root();
    let mut store = fixture_store(&dir);
    let first = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"request_key":"history-sync-42"}),
        )
        .unwrap();
    let second = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"request_key":"history-sync-42"}),
        )
        .unwrap();
    assert_eq!(first["job_id"], second["job_id"]);
    assert_eq!(second["reused"], true);
    let mut changed = source();
    changed["revision"] = json!("revision-two");
    let conflict = store.command(
        "briefing-start",
        &json!({"source":changed,"goals":[],"request_key":"history-sync-42"}),
    );
    assert!(conflict.is_err());
    let durable_jobs = fs::read_dir(dir.join("jobs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(
        durable_jobs, 1,
        "a request-key conflict must not enqueue a second job"
    );
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn status_keeps_long_source_payload_metadata_only() {
    let dir = root();
    let mut store = fixture_store(&dir);
    let mut snapshot = source();
    snapshot["segments"] = (0..500)
        .map(|index| json!({"id":format!("segment-{index}"),"text":"A source statement."}))
        .collect();
    let queued = store
        .command("briefing-start", &json!({"source":snapshot,"goals":[]}))
        .unwrap();
    let status = store
        .command("briefing-status", &json!({"job_id":queued["job_id"]}))
        .unwrap();
    assert!(status["source"].get("segments").is_none());
    assert_eq!(status["source"]["selected_segments"], 500);
    assert_eq!(status["coverage"]["selected_segments"], 500);
    assert!(status["chunks"][0].get("result").is_none());
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restart_resumes_persisted_long_source_checkpoint_without_replaying_done_chunks() {
    let dir = root();
    let mut snapshot = source();
    snapshot["segments"] = (0..500)
        .map(|index| json!({"id":format!("segment-{index}"),"text":"A source statement."}))
        .collect();
    let queued = {
        let mut store = fixture_store(&dir);
        store
            .command("briefing-start", &json!({"source":snapshot,"goals":[]}))
            .unwrap()
    };
    let job_path = dir
        .join("jobs")
        .join(format!("{}.json", queued["job_id"].as_str().unwrap()));
    let mut job: serde_json::Value = serde_json::from_slice(&fs::read(&job_path).unwrap()).unwrap();
    let chunk_count = job["chunks"].as_array().unwrap().len();
    for chunk in job["chunks"].as_array_mut().unwrap() {
        chunk["state"] = json!("complete");
        chunk["result"] = json!({"summary":"checkpointed chunk","candidate_actions":[]});
    }
    job["state"] = json!("running");
    job["coverage"]["chunks_complete"] = json!(chunk_count);
    fs::write(&job_path, serde_json::to_vec_pretty(&job).unwrap()).unwrap();
    let mut reopened = fixture_store(&dir);
    thread::sleep(Duration::from_millis(100));
    let status = reopened
        .command("briefing-status", &json!({"job_id":queued["job_id"]}))
        .unwrap();
    assert_eq!(status["coverage"]["chunks_total"], chunk_count);
    assert_eq!(status["coverage"]["chunks_complete"], chunk_count);
    assert!(matches!(
        status["state"].as_str(),
        Some("complete") | Some("paused") | Some("waiting")
    ));
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn resume_command_requeues_failed_job_and_chunk_detail_keeps_checkpoint_visible() {
    let dir = root();
    let queued = {
        let mut store = fixture_store(&dir);
        store
            .command("briefing-start", &json!({"source":source(),"goals":[]}))
            .unwrap()
    };
    let id = queued["job_id"].as_str().unwrap();
    let path = dir.join("jobs").join(format!("{id}.json"));
    let mut job: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    job["state"] = json!("failed");
    job["error"] = json!("temporary gateway error");
    job["chunks"][0]["state"] = json!("complete");
    job["chunks"][0]["result"] = json!({"summary":"checkpoint","candidate_actions":[]});
    fs::write(&path, serde_json::to_vec_pretty(&job).unwrap()).unwrap();
    let mut store = fixture_store(&dir);
    let resumed = store
        .command("briefing-resume", &json!({"job_id":id}))
        .unwrap();
    // Resumed without his setting: it waits and the checkpoint stays.
    assert_eq!(resumed["state"], "waiting");
    let detail = store
        .command("briefing-chunk", &json!({"job_id":id,"index":0}))
        .unwrap();
    assert_eq!(detail["state"], "complete");
    assert_eq!(detail["result"]["summary"], "checkpoint");
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

/// A client that writes every request it is asked to send to a trace, refuses
/// the live gateway's ports (8080, 8092) outright, and otherwise runs the real client.
fn traced_client(dir: &Path) -> (PathBuf, PathBuf) {
    let trace = dir.join("connect-trace.log");
    let client = dir.join("client.sh");
    fs::write(
        &client,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in http*) echo \"connect $a\" >> '{}';; esac; done\nfor a in \"$@\"; do case \"$a\" in *:8080*|*:8092*) exit 7;; esac; done\nexec /usr/bin/curl \"$@\"\n",
            trace.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&client).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&client, permissions).unwrap();
    (client, trace)
}

fn trace_lines(trace: &Path) -> Vec<String> {
    fs::read_to_string(trace)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stack_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("stack.json");
    fs::write(
        &path,
        json!({"projects":{
            "soliton":{"owner":"ovrin"},"lorengine":{"owner":"zavent"},
            "qroc":{"owner":"citium"},"zavent_sheets":{"owner":"zavent"}
        }})
        .to_string(),
    )
    .unwrap();
    path
}

/// A stand-in gateway on a private port: answers each request with one
/// fixed analysis and counts the connections it accepted.
fn stub_gateway(content: Value) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            seen.fetch_add(1, Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            let mut expect = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                expect |= lower.starts_with("expect: 100-continue");
            }
            if expect {
                stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["model"], "stub-model");
            let reply =
                json!({"choices":[{"message":{"content":content.to_string()}}]}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
    });
    (port, count)
}

fn wait_for(store: &mut BriefingStore, id: &Value, state: &str) -> Value {
    for _ in 0..100 {
        let status = store
            .command("briefing-status", &json!({"job_id":id}))
            .unwrap();
        if status["state"] == state {
            return status;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("briefing never reached {state}");
}

fn mark_queued(dir: &Path, id: &Value) {
    let path = dir
        .join("jobs")
        .join(format!("{}.json", id.as_str().unwrap()));
    let mut job: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    job["state"] = json!("queued");
    fs::write(&path, serde_json::to_vec_pretty(&job).unwrap()).unwrap();
}

#[test]
fn nothing_is_sent_without_his_setting_and_only_his_start_sends_with_it() {
    let dir = root();
    let (client, trace) = traced_client(&dir);
    let tools = StoreTools {
        client,
        stack_map: stack_fixture(&dir),
    };

    // No model.json: his start, an automatic start keyed to a collection, a
    // job found queued after a restart and a resume all wait; nothing is sent.
    let mut store = BriefingStore::open_with(&dir, tools.clone()).unwrap();
    let clicked = store
        .command("briefing-start", &json!({"source":source(),"goals":[]}))
        .unwrap();
    assert_eq!(clicked["state"], "waiting");
    assert_eq!(clicked["waiting_reason"], "No local model is set");
    let automatic = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"request_key":"job-collect-1"}),
        )
        .unwrap();
    assert_eq!(automatic["state"], "waiting");
    drop(store);
    mark_queued(&dir, &clicked["job_id"]);
    let mut store = BriefingStore::open_with(&dir, tools.clone()).unwrap();
    let requeued = store
        .command("briefing-status", &json!({"job_id":clicked["job_id"]}))
        .unwrap();
    assert_eq!(requeued["state"], "waiting");
    let resumed = store
        .command("briefing-resume", &json!({"job_id":clicked["job_id"]}))
        .unwrap();
    assert_eq!(resumed["state"], "waiting");
    thread::sleep(Duration::from_millis(500));
    let absent = trace_lines(&trace);
    eprintln!(
        "connect trace, setting absent ({} lines): {absent:?}",
        absent.len()
    );
    assert!(absent.is_empty());

    // A setting that names anything but this computer is not a setting.
    for gateway in [
        "https://api.example.com/v1",
        "http://10.0.0.2:8080",
        "http://127.0.0.1",
    ] {
        fs::write(
            dir.join("model.json"),
            json!({"enabled":true,"model":"m","gateway":gateway}).to_string(),
        )
        .unwrap();
        assert!(model_setting(&dir).is_err(), "{gateway}");
    }

    // His setting on, pointing at a stand-in on a private port.
    let analysis = json!({"summary":"Two ideas from the note.","candidate_actions":[
        {"title":"Index the voice notes in Lorengine","rationale":"Find old notes again.","segment_ids":["segment-one"],"goal_id":null},
        {"title":"Try the pasta recipe","rationale":"A dinner idea.","segment_ids":["segment-one"],"goal_id":null}]});
    let (port, accepted) = stub_gateway(analysis);
    assert_ne!(port, 8080);
    fs::write(
        dir.join("model.json"),
        json!({"enabled":true,"model":"stub-model","gateway":format!("http://127.0.0.1:{port}")})
            .to_string(),
    )
    .unwrap();
    let keyed = store
        .command(
            "briefing-start",
            &json!({"source":source(),"goals":[],"request_key":"job-collect-2"}),
        )
        .unwrap();
    assert_eq!(keyed["state"], "waiting", "an automatic start never sends");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(accepted.load(Ordering::SeqCst), 0);

    let started = store
        .command("briefing-start", &json!({"source":source(),"goals":[]}))
        .unwrap();
    assert_eq!(started["state"], "queued");
    let done = wait_for(&mut store, &started["job_id"], "complete");
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "one request for one chunk"
    );
    let result = &done["result"];
    assert_eq!(result["model"], "stub-model");
    assert_eq!(result["candidate_actions"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["candidate_actions"][0]["layers"][0]["project"],
        "lorengine"
    );
    assert_eq!(
        result["candidate_actions"][0]["layers"][0]["layer"],
        "zavent"
    );
    assert_eq!(result["not_relevant"][0]["title"], "Try the pasta recipe");
    assert_eq!(result["relevance"]["state"], "relevant");

    // A restart with the setting on pauses queued work for his resume.
    drop(store);
    mark_queued(&dir, &keyed["job_id"]);
    let mut store = BriefingStore::open_with(&dir, tools).unwrap();
    let paused = store
        .command("briefing-status", &json!({"job_id":keyed["job_id"]}))
        .unwrap();
    assert_eq!(paused["state"], "paused");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(accepted.load(Ordering::SeqCst), 1);

    let lines = trace_lines(&trace);
    eprintln!(
        "connect trace, setting on ({} lines): {lines:?}",
        lines.len()
    );
    assert_eq!(
        lines,
        vec![format!(
            "connect http://127.0.0.1:{port}/v1/chat/completions"
        )]
    );
    assert!(lines.iter().all(|line| !line.contains(":8080")));
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn long_sources_partition_from_ten_minutes_to_ten_hours() {
    let dir = root();
    let mut store = fixture_store(&dir);
    // Caption-sized passages, one every four seconds.
    for (label, minutes) in [("ten minutes", 10), ("one hour", 60), ("ten hours", 600)] {
        let count = minutes * 60 / 4;
        let mut snapshot = source();
        snapshot["segments"] = (0..count)
            .map(|index| json!({"id":format!("s{index}"),"text":format!("Passage {index}: a spoken line of ordinary caption length about the work."),"start_ms":index*4000}))
            .collect();
        let queued = store
            .command("briefing-start", &json!({"source":snapshot,"goals":[]}))
            .unwrap();
        assert_eq!(queued["state"], "waiting");
        let path = dir
            .join("jobs")
            .join(format!("{}.json", queued["job_id"].as_str().unwrap()));
        let job: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let ids: Vec<String> = job["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|chunk| chunk["segment_ids"].as_array().unwrap().clone())
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        let expected: Vec<String> = (0..count).map(|index| format!("s{index}")).collect();
        assert_eq!(ids, expected, "{label}: every passage once, in order");
        eprintln!(
            "{label}: {count} passages in {} chunks",
            job["chunks"].as_array().unwrap().len()
        );
    }
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn results_map_to_goals_and_layers_and_the_irrelevant_are_dropped() {
    let dir = root();
    let stack: Vec<(String, String)> = {
        let manifest: Value =
            serde_json::from_slice(&fs::read(stack_fixture(&dir)).unwrap()).unwrap();
        manifest["projects"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(project, entry)| (project.clone(), entry["owner"].as_str().unwrap().to_owned()))
            .collect()
    };
    let goals = vec![json!({"id":"goal-sleep","title":"Sleep schedule before midnight"})];
    let segments = vec![
        json!({"id":"a","text":"Move the sleep alarm earlier."}),
        json!({"id":"b","text":"Zavent sheets could hold the price table."}),
        json!({"id":"c","text":"This video is sponsored by a game."}),
    ];
    let mut result = json!({"summary":"s","candidate_actions":[
        {"title":"Set the alarm","rationale":"","segment_ids":["a"],"goal_id":null},
        {"title":"Price table","rationale":"","segment_ids":["b"],"goal_id":null},
        {"title":"Watch the game","rationale":"","segment_ids":["c"],"goal_id":null}]});
    map_relevance(&mut result, &goals, &segments, &stack);
    let kept = result["candidate_actions"].as_array().unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0]["goals"][0]["id"], "goal-sleep");
    assert_eq!(kept[1]["layers"][0]["project"], "zavent_sheets");
    assert!(
        kept[1]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["layer"] == "zavent")
    );
    assert_eq!(result["not_relevant"][0]["title"], "Watch the game");
    assert!(
        result["not_relevant"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("none of your goals")
    );

    let mut nothing = json!({"summary":"s","candidate_actions":[
        {"title":"Watch the game","rationale":"","segment_ids":["c"],"goal_id":null}]});
    map_relevance(&mut nothing, &goals, &segments, &stack);
    assert_eq!(nothing["candidate_actions"], json!([]));
    assert_eq!(nothing["relevance"]["state"], "not_relevant");
    fs::remove_dir_all(dir).unwrap();
}
