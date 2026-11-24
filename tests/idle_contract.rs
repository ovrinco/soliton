use serde_json::json;
use soliton::service::Service;
use soliton_kernel::{Entity, GovernedRepository};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn scratch(label: &str) -> PathBuf {
    let root = PathBuf::from(std::env::var("T02_SCRATCH").expect("isolated scratch required"))
        .join(format!(
            "{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn unchanged_requests_and_idle_ticks_do_not_write_or_emit_state() {
    let root = scratch("unchanged");
    let mut service = Service::open(&root).unwrap();
    service
        .command(
            "save",
            &json!({"kind":"task","title":"Idle contract","scheduled_for":"2026-10-12"}),
        )
        .unwrap();
    let journal = root.join("journal/operations.log");
    let before = fs::read(&journal).unwrap();
    let args = json!({"from":"2026-01-01","through":"2026-12-31","timezone":"America/Toronto"});
    let calendar = service.command("calendar", &args).unwrap();
    let proposals = service.command("proposal-calendar", &args).unwrap();
    for _ in 0..5 {
        assert_eq!(service.command("calendar", &args).unwrap(), calendar);
        assert_eq!(
            service.command("proposal-calendar", &args).unwrap(),
            proposals
        );
        assert!(service.command("clock", &args).unwrap()["local"].is_string());
        service.background_tick().unwrap();
        std::thread::sleep(Duration::from_millis(210));
        assert!(service.check_external_freshness().unwrap().is_none());
    }
    assert_eq!(fs::read(journal).unwrap(), before);
    assert!(!root.join("collection").exists());
}

#[test]
fn real_mutation_refreshes_calendar_and_external_owner_update_pushes_once() {
    let root = scratch("refresh");
    let mut service = Service::open(&root).unwrap();
    let args = json!({"from":"2026-10-12","through":"2026-10-12","timezone":"America/Toronto"});
    assert!(
        service
            .command("calendar", &args)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let saved = service
        .command(
            "save",
            &json!({"kind":"task","title":"Before","scheduled_for":"2026-10-12"}),
        )
        .unwrap();
    assert_eq!(
        service
            .command("calendar", &args)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let id = saved["id"].as_str().unwrap().parse().unwrap();
    let prior = service.snapshot.get(&id).unwrap().clone();
    let mut changed = prior.entity.clone();
    changed.meta_mut().title = "After".into();
    changed.meta_mut().revision += 1;
    GovernedRepository::open(&root)
        .unwrap()
        .update(changed, prior.content_hash)
        .unwrap();
    std::thread::sleep(Duration::from_millis(210));
    assert!(service.check_external_freshness().unwrap().is_some());
    assert_eq!(
        service.command("calendar", &args).unwrap()[0]["title"],
        "After"
    );
    std::thread::sleep(Duration::from_millis(210));
    assert!(service.check_external_freshness().unwrap().is_none());
}

#[test]
fn copied_real_data_owner_change_reaches_engine_state_event() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, Command, Stdio};
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = PathBuf::from(std::env::var("T02_REAL_COPY").expect("copied real data required"));
    let repo = GovernedRepository::open(&root).unwrap();
    let snapshot = repo.rebuild().unwrap();
    let task = snapshot
        .entities()
        .find(|e| matches!(e, Entity::Task(_)))
        .expect("real task");
    let prior = snapshot.get(task.id()).unwrap().clone();
    let mut changed = prior.entity.clone();
    changed.meta_mut().title = "T02 copied-data refresh proof".into();
    changed.meta_mut().revision += 1;
    let mut child = Owned(
        Command::new(std::env::var("T02_ENGINE").expect("pinned copied engine"))
            .arg(&root)
            .env("SOLITON_AUTO_CAPTURE", "0")
            .env("SOLITON_COLLECTION_AUTOSTART", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let input = child.0.stdin.as_mut().unwrap();
    writeln!(
        input,
        "{}",
        json!({"id":1,"command":"clock","args":{"timezone":"America/Toronto"}})
    )
    .unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["ok"],
        true
    );
    let start = Instant::now();
    repo.update(changed, prior.content_hash).unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let elapsed = start.elapsed();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "state");
    let record = event["state"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == prior.entity.id().as_str())
        .unwrap();
    assert_eq!(record["title"], "T02 copied-data refresh proof");
    assert_eq!(record["revision"], prior.entity.meta().revision + 1);
    println!(
        "copied-real-owner-to-engine-state-event latency_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    assert!(elapsed < Duration::from_secs(2));
    writeln!(input, "{}", json!({"id":2,"command":"shutdown","args":{}})).unwrap();
    input.flush().unwrap();
    assert!(child.0.wait().unwrap().success());
}

#[test]
fn copied_owner_change_reaches_running_qml_records() {
    use std::process::Command;
    let pid = std::env::var("T02_UI_PID").expect("owned QML runtime required");
    let root = PathBuf::from(std::env::var("T02_UI_COPY").expect("owned UI workspace"));
    let record_id = std::env::var("T02_RECORD_ID").expect("visible copied task id");
    let repo = GovernedRepository::open(&root).unwrap();
    let snapshot = repo.rebuild().unwrap();
    let id = record_id.parse().unwrap();
    let prior = snapshot.get(&id).expect("copied visible task").clone();
    let mut changed = prior.entity.clone();
    let title = "T02 copied owner visible latency proof";
    changed.meta_mut().title = title.into();
    changed.meta_mut().revision += 1;
    let start = Instant::now();
    repo.update(changed, prior.content_hash).unwrap();
    let commit_elapsed = start.elapsed();
    loop {
        let result = Command::new("/usr/bin/qs")
            .args([
                "ipc",
                "--pid",
                &pid,
                "call",
                "soliton-input",
                "recordProbe",
                &record_id,
            ])
            .output()
            .expect("owned QML probe");
        assert!(result.status.success(), "owned IPC failed");
        let probe: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        if probe["record"]["title"] == title {
            let result = Command::new("/usr/bin/qs")
                .args(["ipc", "--pid", &pid, "call", "soliton-input", "items"])
                .output()
                .expect("owned visible item probe");
            assert!(result.status.success(), "owned visible IPC failed");
            let items: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let visible = items.as_array().unwrap().iter().any(|item| {
                item["text"] == title
                    && item["visibleRect"]["width"].as_f64().unwrap_or(0.0) > 0.0
                    && item["visibleRect"]["height"].as_f64().unwrap_or(0.0) > 0.0
            });
            if !visible {
                assert!(
                    start.elapsed() < Duration::from_secs(2),
                    "copied title did not reach visible QML text"
                );
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let elapsed = start.elapsed();
            println!(
                "copied-owner-to-qml-visible-record latency_ms={:.3} commit_ms={:.3}",
                elapsed.as_secs_f64() * 1000.0,
                commit_elapsed.as_secs_f64() * 1000.0
            );
            assert!(elapsed < Duration::from_secs(2));
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "copied owner update did not reach QML"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn repeated_registered_bootstrap_preserves_registry_without_opening_archive() {
    let root = scratch("registered-bootstrap");
    let mut service = Service::open(&root).unwrap();
    let history = root.join("history");
    fs::create_dir_all(&history).unwrap();
    let sources = json!([
        {"provider":"codex","enabled":false,"path":"/unchanged/codex"},
        {"provider":"antigravity","enabled":true,"path":"/unchanged/antigravity"},
        {"provider":"voxtype","enabled":true,"path":"/unchanged/voxtype"}
    ]);
    let registry = history.join("capture-sources.json");
    let before =
        serde_json::to_vec(&json!({"schema":"soliton-history-capture/1","sources":sources}))
            .unwrap();
    fs::write(&registry, &before).unwrap();
    for _ in 0..3 {
        let job = service
            .command(
                "history-job-start",
                &json!({"command":"history-capture-bootstrap","args":{}}),
            )
            .unwrap();
        let start = Instant::now();
        loop {
            let status = service
                .command("history-job-status", &json!({"id":job["id"]}))
                .unwrap();
            if status["state"] != "running" {
                assert_eq!(status["ok"], true);
                assert_eq!(status["result"]["sources"], sources);
                assert_eq!(status["result"]["added"], json!([]));
                assert_eq!(status["result"]["skipped"].as_array().unwrap().len(), 3);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(fs::read(registry).unwrap(), before);
    assert!(!history.join("history-index.sqlite3").exists());
    assert!(!history.join(".archive.lock").exists());
    // A real archive query still uses the normal recovery/index path.
    service.command("history-list", &json!({})).unwrap();
    assert!(history.join("history-index.sqlite3").exists());
}

#[test]
fn incomplete_or_corrupt_bootstrap_keeps_the_normal_archive_path() {
    for corrupt in [false, true] {
        let root = scratch("bootstrap-fallback");
        let mut service = Service::open(&root).unwrap();
        let history = root.join("history");
        fs::create_dir_all(&history).unwrap();
        let schema = if corrupt {
            "unsupported"
        } else {
            "soliton-history-capture/1"
        };
        fs::write(history.join("capture-sources.json"), serde_json::to_vec(&json!({
            "schema":schema,
            "sources":[{"provider":"codex","enabled":false},{"provider":"antigravity","enabled":true}]
        })).unwrap()).unwrap();
        let job = service
            .command(
                "history-job-start",
                &json!({"command":"history-capture-bootstrap","args":{}}),
            )
            .unwrap();
        let start = Instant::now();
        loop {
            let status = service
                .command("history-job-status", &json!({"id":job["id"]}))
                .unwrap();
            if status["state"] != "running" {
                assert_eq!(status["ok"], !corrupt);
                if !corrupt {
                    assert_eq!(status["result"]["sources"][0]["enabled"], false);
                    assert!(
                        status["result"]["skipped"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|s| s["provider"] == "voxtype")
                    );
                }
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(history.join("history-index.sqlite3").exists());
    }
}

fn socket_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
        .filter(|target| target.to_string_lossy().starts_with("socket:"))
        .count()
}

#[test]
fn local_model_off_refuses_briefing_and_opens_no_socket() {
    let root = scratch("local-model-off");
    let mut service = Service::open(&root).unwrap();
    assert!(service.preferences().unwrap()["localModel"].is_null());
    let sockets = socket_count();
    for command in [
        "briefing-start",
        "briefing-resume",
        "inference-status",
        "inference-start",
    ] {
        let error = service
            .command(command, &json!({"source_id":"any","job_id":"any"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Local model is off"), "{command}: {error}");
    }
    assert_eq!(
        socket_count(),
        sockets,
        "a refused model path opens no socket"
    );
    assert!(!root.join("briefings").exists());
}
