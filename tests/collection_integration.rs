use serde_json::json;
use soliton::service::Service;
use soliton_kernel::ObjectId;
use std::{fs, thread, time::Duration};

fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(
        ObjectId::mint("collection-integration")
            .unwrap()
            .to_string(),
    );
    fs::create_dir_all(&path).unwrap();
    path
}

fn wait_for_completed(service: &mut Service, source_id: &str) {
    for _ in 0..200 {
        let status = service.command("collection-status", &json!({})).unwrap();
        if status["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["source_id"] == source_id && job["state"] == "completed")
        {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("collection worker did not complete")
}

fn tick_until_archived(service: &mut Service) {
    for _ in 0..300 {
        let status = service.command("collection-tick", &json!({})).unwrap();
        if status["archive_import_running"] != true
            && service
                .command("collection-pending-imports", &json!({}))
                .unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("collection history import did not finish")
}

#[test]
fn service_tick_archives_then_acknowledges_and_refresh_is_idempotent() {
    let dir = root();
    let source = dir.join("lesson.txt");
    fs::write(&source, "A local learning note.\n").unwrap();
    let workspace = dir.join("workspace");
    let source_id;
    {
        let mut service = Service::open(&workspace).unwrap();
        let configured = service
            .command(
                "collection-add",
                &json!({"type":"transcript","path":source}),
            )
            .unwrap();
        source_id = configured["id"].as_str().unwrap().to_owned();
        wait_for_completed(&mut service, &source_id);
        // Drop after collection completion but before collection-tick starts
        // the history worker: the durable unacknowledged job must be replayed.
    }
    {
        let mut service = Service::open(&workspace).unwrap();
        tick_until_archived(&mut service);
        let first = service.command("history-list", &json!({})).unwrap();
        let count = first["items"].as_array().unwrap().len();
        assert!(count >= 1);
        for _ in 0..3 {
            let _ = service.command("collection-tick", &json!({})).unwrap();
        }
        let second = service.command("history-list", &json!({})).unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), count);
        assert!(
            service
                .command("collection-pending-imports", &json!({}))
                .unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_collection_job_is_archived_by_idle_loop_without_due_scheduler() {
    if std::env::var("SOLITON_COLLECTION_AUTOSTART").is_ok() {
        return;
    }
    let dir = root();
    let source = dir.join("explicit.txt");
    fs::write(&source, "An explicitly requested local learning note.\n").unwrap();
    let workspace = dir.join("workspace");
    let mut service = Service::open(&workspace).unwrap();
    let configured = service
        .command(
            "collection-add",
            &json!({"type":"transcript","path":source}),
        )
        .unwrap();
    let source_id = configured["id"].as_str().unwrap();
    for _ in 0..600 {
        // This is the normal service heartbeat. With collection autostart off
        // it drains the explicitly started worker but never enqueues another
        // due source or starts the briefing model.
        service.background_tick().unwrap();
        let status = service.command("collection-status", &json!({})).unwrap();
        let completed = status["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["source_id"] == source_id && job["state"] == "completed");
        let pending = service
            .command("collection-pending-imports", &json!({}))
            .unwrap();
        let history_ready = service
            .command("history-list", &json!({"limit": 20}))
            .map(|value| {
                value["items"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            })
            .unwrap_or(false);
        if completed && pending["items"].as_array().is_some_and(Vec::is_empty) && history_ready {
            drop(service);
            fs::remove_dir_all(dir).unwrap();
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("explicit collection result was not archived by the idle loop");
}

#[test]
fn explicit_sources_advance_in_order_without_dispatching_unrelated_due_work() {
    if std::env::var("SOLITON_COLLECTION_AUTOSTART").is_ok() {
        return;
    }
    let dir = root();
    let first = dir.join("first.txt");
    let second = dir.join("second.txt");
    fs::write(&first, "The first explicitly requested note.\n").unwrap();
    fs::write(&second, "The second explicitly requested note.\n").unwrap();
    let unrelated = dir.join("unrelated.txt");
    fs::write(&unrelated, "A persisted but unrelated due note.\n").unwrap();
    let workspace = dir.join("workspace");
    let (first_id, second_id, unrelated_id) = {
        let mut service = Service::open(&workspace).unwrap();
        let first_id = service
            .command("collection-add", &json!({"type":"transcript","path":first}))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second_id = service
            .command(
                "collection-add",
                &json!({"type":"transcript","path":second}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        // Persist a source without requesting it. Its queued due job is
        // installed below after restart, so open starts with an empty
        // explicit allowlist and must leave this source untouched.
        let unrelated_id = service
            .command(
                "collection-add",
                &json!({"type":"transcript","path":unrelated,"enabled":false}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        drop(service);
        (first_id, second_id, unrelated_id)
    };
    let jobs = workspace.join("collection").join("jobs");
    fs::write(
        jobs.join("job-unrelated.json"),
        serde_json::to_vec_pretty(&json!({
            "schema": 1,
            "id": "job-unrelated",
            "source_id": unrelated_id,
            "state": "queued",
            "requested_at": "test",
            "due_at": 0,
            "attempts": 0,
            "auto_briefing": false,
            "goal_ids": []
        }))
        .unwrap(),
    )
    .unwrap();
    let mut service = Service::open(&workspace).unwrap();
    // Reassert both sources while the first worker is active. The explicit
    // allowlist must carry the queued second job through later poll(false)
    // calls, without broadening to the persisted unrelated due job.
    service
        .command("collection-check", &json!({"id":first_id}))
        .unwrap();
    service
        .command("collection-check", &json!({"id":second_id}))
        .unwrap();

    for _ in 0..900 {
        service.background_tick().unwrap();
        let status = service.command("collection-status", &json!({})).unwrap();
        let completed = status["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|job| {
                (job["source_id"] == first_id || job["source_id"] == second_id)
                    && job["state"] == "completed"
            })
            .count();
        let pending = service
            .command("collection-pending-imports", &json!({}))
            .unwrap();
        let unrelated_queued = status["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["source_id"] == unrelated_id && job["state"] == "queued");
        if completed == 2
            && unrelated_queued
            && pending["items"].as_array().is_some_and(Vec::is_empty)
            && service
                .command("history-list", &json!({"limit":20}))
                .unwrap()["items"]
                .as_array()
                .is_some_and(|items| items.len() >= 2)
        {
            drop(service);
            fs::remove_dir_all(dir).unwrap();
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("explicit collection sources did not both archive in order");
}

#[test]
fn news_cache_items_keep_ids_and_links_and_acquired_media_is_listed() {
    let dir = root();
    let cache = dir.join("cache.json");
    fs::write(&cache, r#"{"v":2,"lastRefreshMs":1,"items":[
        {"id":"hn|hn:1","sid":"hn","title":"First","link":"https://example.org/a","comments":"https://news.ycombinator.com/item?id=1","publishedMs":1792144495000,"summary":"","domain":"example.org"},
        {"id":"hn|hn:2","sid":"hn","title":"Second","link":"https://example.org/b","summary":"Body"}]}"#).unwrap();
    let workspace = dir.join("workspace");
    let mut service = Service::open(&workspace).unwrap();
    let configured = service
        .command(
            "collection-add",
            &json!({"type":"feed","path":cache,"auto_briefing":false}),
        )
        .unwrap();
    let source_id = configured["id"].as_str().unwrap().to_owned();
    wait_for_completed(&mut service, &source_id);
    let status = service.command("collection-status", &json!({})).unwrap();
    let job = status["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["source_id"] == source_id)
        .unwrap()
        .clone();
    let normalized: serde_json::Value =
        serde_json::from_slice(&fs::read(job["result_path"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(normalized["items"][0]["id"], "hn|hn:1");
    assert_eq!(normalized["items"][0]["url"], "https://example.org/a");
    assert_eq!(normalized["source_refs"][1]["origin_id"], "hn|hn:2");
    assert_eq!(normalized["source_refs"][1]["url"], "https://example.org/b");
    let listed = service
        .command("collection-media-list", &json!({"limit":10}))
        .unwrap();
    assert!(listed["acquired"].as_array().unwrap().is_empty());
    assert!(
        service
            .command("collection-pending-briefings", &json!({}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(service);
    fs::remove_dir_all(dir).unwrap();
}

fn sha256(path: &std::path::Path) -> String {
    let out = std::process::Command::new("sha256sum")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
}

/// The real Waybar news cache (a copy; the original is only hashed): every
/// item becomes a feed record with its own id and link.
#[test]
fn real_waybar_news_items_become_records_with_ids_and_links() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let real = std::path::Path::new(&home).join(".local/state/omarchy-news/cache.json");
    if !real.is_file() {
        println!("news cache absent; nothing to prove here");
        return;
    }
    let before = sha256(&real);
    let dir = root();
    let cache = dir.join("cache.json");
    fs::copy(&real, &cache).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    let total = raw["items"].as_array().unwrap().len();
    let mut service = Service::open(&dir.join("workspace")).unwrap();
    let source = service
        .command("collection-add", &json!({"type":"feed","path":cache}))
        .unwrap();
    let source_id = source["id"].as_str().unwrap().to_owned();
    wait_for_completed(&mut service, &source_id);
    let status = service.command("collection-status", &json!({})).unwrap();
    let job = status["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["source_id"] == source_id)
        .unwrap()
        .clone();
    let normalized: serde_json::Value =
        serde_json::from_slice(&fs::read(job["result_path"].as_str().unwrap()).unwrap()).unwrap();
    let items = normalized["items"].as_array().unwrap();
    let kept = items
        .iter()
        .zip(raw["items"].as_array().unwrap())
        .filter(|(item, row)| {
            item["id"] == row["id"]
                && item["url"] == row["link"]
                && item["url"].as_str().is_some_and(|u| u.starts_with("http"))
        })
        .count();
    println!("news items with id and url: {kept} of {total}");
    assert_eq!(kept, total);
    drop(service);
    fs::remove_dir_all(dir).unwrap();
    assert_eq!(
        sha256(&real),
        before,
        "the live news cache is never written"
    );
}

fn chromium_fixture(path: &std::path::Path) {
    let script = "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('create table urls (id integer primary key, url text, title text, visit_count integer, last_visit_time integer)'); \
        db.executemany('insert into urls (url,title,visit_count,last_visit_time) values (?,?,?,?)', [\
        ('https://www.youtube.com/watch?v=4DhcSPkEbwI','AI Skills with Matt Pocock',3,13405000000000000),\
        ('https://www.youtube.com/watch?v=4DhcSPkEbwI&t=60s','AI Skills with Matt Pocock',1,13405000100000000),\
        ('https://www.youtube.com/watch?v=BMYvfVk8Ar0','Evolution of Deep Learning',2,13404000000000000),\
        ('https://www.youtube.com/watch?v=dQw4w9WgXcQ','Music video',5,13403000000000000),\
        ('https://example.org/article','An article',9,13405000000000000)]); db.commit()";
    assert!(
        std::process::Command::new("python3")
            .args(["-c", script])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

/// Watched videos from local browser history are intake candidates only
/// when his switch is on; with no switch nothing is read at all.
#[test]
fn browser_history_yields_watch_entries_only_when_his_switch_is_on() {
    let dir = root();
    let history = dir.join("History");
    chromium_fixture(&history);
    let before = sha256(&history);
    let workspace = dir.join("workspace");
    let mut service = Service::open(&workspace).unwrap();
    let off = service
        .command("collection-watch-history", &json!({"path":history}))
        .unwrap();
    println!("switch absent: {off}");
    assert_eq!(
        (off["enabled"].clone(), off["read"].clone()),
        (json!(false), json!(false))
    );
    assert!(
        !workspace.join("collection/cache").exists(),
        "nothing was copied"
    );
    assert!(
        !workspace.join("collection/intake").exists(),
        "nothing was recorded"
    );
    let mut prefs =
        soliton::source_preferences::SourcePreferences::open(&workspace.join("source-preferences"))
            .unwrap();
    prefs.command("source-metadata-set", &json!({"locator":"video","pattern":"dQw4w9WgXcQ","category":"music","evidence":"public title page"})).unwrap();
    prefs
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"exclude","matcher":"category","pattern":"music"}),
        )
        .unwrap();
    drop(prefs);
    fs::write(
        workspace.join("source-preferences/intake.json"),
        r#"{"browser_history":true}"#,
    )
    .unwrap();
    let on = service
        .command("collection-watch-history", &json!({"path":history}))
        .unwrap();
    let items = on["items"].as_array().unwrap();
    for item in items {
        println!(
            "watch entry: {} | {} | count {} | {}",
            item["video_id"], item["title"], item["visit_count"], item["last_watched_at"]
        );
    }
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["video_id"], "4DhcSPkEbwI");
    assert_eq!(
        items[0]["visit_count"], 4,
        "two visits of one video count as one entry"
    );
    assert_eq!(items[0]["signal"], "watched");
    assert_eq!(on["refused"], 1, "his music rule keeps the music video out");
    assert_eq!(
        fs::read_dir(workspace.join("collection/cache"))
            .unwrap()
            .count(),
        0,
        "the copy is removed"
    );
    assert_eq!(sha256(&history), before, "the history file is only read");
    // A candidate can be picked for review.
    let picked = service
        .command(
            "collection-add",
            &json!({"type":"youtube-video","url":items[1]["url"],"enabled":false}),
        )
        .unwrap();
    assert_eq!(picked["kind"], "youtube-video");
    drop(service);
    fs::remove_dir_all(dir).unwrap();
}

/// Cadence work waits for his automatic-capture switch: with it off, thirty
/// ticks of the window's two-second collection timer (sixty seconds) make
/// no request; with it on, the due source is checked once.
#[test]
fn cadence_runs_only_with_automatic_capture_on_and_then_once() {
    if std::env::var("SOLITON_AUTO_CAPTURE").as_deref() == Ok("0") {
        return;
    }
    let dir = root();
    let feed = dir.join("feed.json");
    fs::write(
        &feed,
        r#"{"v":2,"items":[{"id":"hn|hn:7","title":"Seven","link":"https://example.org/7"}]}"#,
    )
    .unwrap();
    let workspace = dir.join("workspace");
    let sources = workspace.join("collection/sources");
    fs::create_dir_all(&sources).unwrap();
    fs::write(sources.join("collection-feed-due.json"), json!({"schema":1,"id":"collection-feed-due","kind":"feed","location":feed,
        "cadence_hours":12,"enabled":true,"auto_briefing":false,"goal_ids":[],"created_at":"2026-10-30T00:00:00Z",
        "last_success":null,"last_error":null,"next_due_at":0}).to_string()).unwrap();
    let mut service = Service::open(&workspace).unwrap();
    let jobs = |service: &mut Service| {
        service.command("collection-status", &json!({})).unwrap()["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|job| job["source_id"] == "collection-feed-due")
            .count()
    };
    for _ in 0..30 {
        service.command("collection-tick", &json!({})).unwrap();
    }
    let off = jobs(&mut service);
    println!("automatic capture off, 30 ticks: cadence requests {off}");
    assert_eq!(off, 0);
    service
        .command("preferences", &json!({"automaticCapture": true}))
        .unwrap();
    for _ in 0..30 {
        service.command("collection-tick", &json!({})).unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    wait_for_completed(&mut service, "collection-feed-due");
    for _ in 0..30 {
        service.command("collection-tick", &json!({})).unwrap();
    }
    let on = service.command("collection-status", &json!({})).unwrap()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["source_id"] == "collection-feed-due" && job["state"] != "queued")
        .count();
    println!("automatic capture on, 60 ticks: cadence requests {on}");
    assert_eq!(on, 1, "checked once; the next check is twelve hours on");
    drop(service);
    fs::remove_dir_all(dir).unwrap();
}
