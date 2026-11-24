use serde_json::json;
use soliton::collection::CollectionStore;
use soliton_kernel::ObjectId;
use std::{fs, thread, time::Duration};

fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let root =
        std::env::temp_dir().join(ObjectId::mint("collection-contract").unwrap().to_string());
    fs::create_dir_all(&root).unwrap();
    let transcript = root.join("lesson.srt");
    fs::write(
        &transcript,
        "1\n00:00:00,000 --> 00:00:01,000\nHello\n\n2\n00:00:05,000 --> 00:00:06,000\nWorld\n",
    )
    .unwrap();
    (root, transcript)
}

fn wait_for_result(store: &mut CollectionStore) -> serde_json::Value {
    for _ in 0..100 {
        let result = store.command("collection-poll", &json!({})).unwrap();
        if result["completed"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
            || result["failed"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        {
            return result;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("collection worker did not report a result")
}

#[test]
fn transcript_is_normalized_with_explicit_partial_gaps() {
    let (root, transcript) = fixture();
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    let source = store
        .command(
            "collection-add",
            &json!({"type":"transcript","path":transcript,"cadence_hours":12}),
        )
        .unwrap();
    assert_eq!(source["capabilities"]["state"], "available");
    let result = wait_for_result(&mut store);
    assert!(result["failed"].as_array().unwrap().is_empty());
    let completed = &result["completed"][0];
    assert_eq!(completed["provider"], "normalized");
    assert_eq!(completed["coverage"]["state"], "partial");
    assert_eq!(completed["coverage"]["gaps"].as_array().unwrap().len(), 1);
    let normalized: serde_json::Value =
        serde_json::from_slice(&fs::read(completed["normalized_path"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(normalized["schema"], "soliton-source/1");
    assert_eq!(normalized["provider"], "transcript");
    let original = fs::read(&transcript).unwrap();
    fs::remove_file(&transcript).unwrap();
    assert_eq!(
        fs::read(completed["raw_path"].as_str().unwrap()).unwrap(),
        original
    );
    assert_eq!(
        store
            .command("collection-pending-imports", &json!({}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    store
        .command(
            "collection-ack",
            &json!({"job_id":completed["job_id"],"history_ids":["history-1"]}),
        )
        .unwrap();
    assert!(
        store
            .command("collection-pending-imports", &json!({}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_feed_is_a_visible_failed_job() {
    let (root, _) = fixture();
    let feed = root.join("broken.xml");
    fs::write(&feed, "<rss><channel><item><title>incomplete").unwrap();
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    store
        .command("collection-add", &json!({"type":"feed","path":feed}))
        .unwrap();
    let result = wait_for_result(&mut store);
    assert!(result["completed"].as_array().unwrap().is_empty());
    let error = result["failed"][0]["error"].as_str().unwrap();
    assert!(error.contains("incomplete") || error.contains("malformed"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn queue_is_durable_and_check_does_not_duplicate_pending_source() {
    let (root, transcript) = fixture();
    let store_root = root.join("store");
    let mut store = CollectionStore::open(&store_root).unwrap();
    store
        .command(
            "collection-add",
            &json!({"type":"transcript","path":transcript}),
        )
        .unwrap();
    store.command("collection-check", &json!({})).unwrap();
    let status = store.command("collection-status", &json!({})).unwrap();
    assert_eq!(status["jobs"].as_array().unwrap().len(), 1);
    drop(store);

    let mut reopened = CollectionStore::open(&store_root).unwrap();
    let status = reopened.command("collection-status", &json!({})).unwrap();
    assert_eq!(status["jobs"].as_array().unwrap().len(), 1);
    // A status read drains the worker. If the original job completed before
    // restart, an explicit check legitimately creates a new run. Assert the
    // durable identity and the one-pending-run invariant, not worker timing.
    let original = status["jobs"][0]["id"].clone();
    let finished = !matches!(
        status["jobs"][0]["state"].as_str(),
        Some("queued" | "running")
    );
    reopened.command("collection-check", &json!({})).unwrap();
    let after = reopened.command("collection-status", &json!({})).unwrap();
    let jobs = after["jobs"].as_array().unwrap();
    assert!(jobs.iter().any(|job| job["id"] == original));
    assert_eq!(jobs.len(), if finished { 2 } else { 1 });
    assert!(
        jobs.iter()
            .filter(|job| matches!(job["state"].as_str(), Some("queued" | "running")))
            .count()
            <= 1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_history_uses_readonly_consistent_backup_adapter() {
    let (root, _) = fixture();
    let history = root.join("History");
    let script = "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('create table urls (url text,title text,last_visit_time integer)'); db.execute(\"insert into urls values ('https://example.test','Example',13200000000000000)\"); db.commit()";
    let status = std::process::Command::new("python3")
        .args(["-c", script, history.to_string_lossy().as_ref()])
        .status()
        .unwrap();
    assert!(status.success());
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    // Off until he turns the browser-history intake on.
    assert!(
        store
            .command(
                "collection-add",
                &json!({"type":"browser-history","path":history})
            )
            .unwrap_err()
            .to_string()
            .contains("Browser history intake is off")
    );
    fs::create_dir_all(root.join("source-preferences")).unwrap();
    fs::write(
        root.join("source-preferences/intake.json"),
        r#"{"browser_history":true}"#,
    )
    .unwrap();
    store
        .command(
            "collection-add",
            &json!({"type":"browser-history","path":history}),
        )
        .unwrap();
    let result = wait_for_result(&mut store);
    assert!(result["failed"].as_array().unwrap().is_empty());
    assert_eq!(result["completed"][0]["provider"], "normalized");
    assert_eq!(result["completed"][0]["coverage"]["state"], "complete");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn long_plain_transcript_reports_bounded_partial_coverage() {
    let (root, _) = fixture();
    let transcript = root.join("long.txt");
    let body = (0..20100)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    fs::write(&transcript, body).unwrap();
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    store
        .command(
            "collection-add",
            &json!({"type":"transcript","path":transcript}),
        )
        .unwrap();
    let result = wait_for_result(&mut store);
    let completed = &result["completed"][0];
    assert_eq!(completed["coverage"]["state"], "partial");
    assert!(
        completed["coverage"]["notice"]
            .as_str()
            .unwrap()
            .contains("bounded")
    );
    assert_eq!(completed["segments_total"], 20000);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn briefing_follows_archive_ack_and_forced_check_advances_future_job() {
    let (root, _) = fixture();
    let transcript = root.join("pipeline.txt");
    fs::write(
        &transcript,
        "A useful source with an explicit proposal for consideration.",
    )
    .unwrap();
    let store_root = root.join("store");
    let mut store = CollectionStore::open(&store_root).unwrap();
    let source=store.command("collection-add", &json!({"type":"transcript","path":transcript,"auto_briefing":true,"goal_ids":["goal-one"]})).unwrap();
    let result = wait_for_result(&mut store);
    let job = result["completed"][0]["job_id"].clone();
    let job_status = store
        .command("collection-job-status", &json!({"id":job}))
        .unwrap();
    assert_eq!(job_status["id"], job);
    assert_eq!(job_status["state"], "completed");
    assert!(
        store
            .command("collection-pending-briefings", &json!({}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    store
        .command(
            "collection-ack",
            &json!({"job_id":job,"history_ids":["history-pinned"]}),
        )
        .unwrap();
    let pending = store
        .command("collection-pending-briefings", &json!({}))
        .unwrap();
    assert_eq!(pending["items"][0]["history_ids"][0], "history-pinned");
    assert_eq!(pending["items"][0]["goal_ids"][0], "goal-one");
    store
        .command(
            "collection-mark-briefing",
            &json!({"job_id":job,"briefing":{"job_id":"briefing-one","state":"queued"}}),
        )
        .unwrap();
    drop(store);
    let mut store = CollectionStore::open(&store_root).unwrap();
    assert!(
        store
            .command("collection-pending-briefings", &json!({}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Cadence work runs only with his automatic-capture switch on.
    store.set_automatic_capture(true);
    store.command("collection-poll-due", &json!({})).unwrap();
    let before = store.command("collection-list", &json!({})).unwrap();
    assert!(
        before["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["state"] == "queued")
    );
    store
        .command("collection-check", &json!({"id":source["id"]}))
        .unwrap();
    let second = wait_for_result(&mut store);
    assert_eq!(second["completed"].as_array().unwrap().len(), 1);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn collection_ack_is_idempotent_and_conflicts_are_rejected() {
    let (root, transcript) = fixture();
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    store
        .command(
            "collection-add",
            &json!({"type":"transcript","path":transcript}),
        )
        .unwrap();
    let result = wait_for_result(&mut store);
    let job = result["completed"][0]["job_id"].clone();
    let first = store
        .command(
            "collection-ack",
            &json!({"job_id":job,"history_ids":["history-1"]}),
        )
        .unwrap();
    assert_eq!(first["acked"], true);
    let repeat = store
        .command(
            "collection-ack",
            &json!({"job_id":job,"history_ids":["history-1"]}),
        )
        .unwrap();
    assert_eq!(repeat["already_acknowledged"], true);
    assert!(
        store
            .command(
                "collection-ack",
                &json!({"job_id":job,"history_ids":["history-2"]}),
            )
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn collection_list_and_status_are_pageable() {
    let (root, transcript) = fixture();
    let second = root.join("lesson-2.srt");
    fs::copy(&transcript, &second).unwrap();
    let mut store = CollectionStore::open(&root.join("store")).unwrap();
    for path in [&transcript, &second] {
        store
            .command("collection-add", &json!({"type":"transcript","path":path}))
            .unwrap();
    }
    let page = store
        .command("collection-list", &json!({"limit":1}))
        .unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert!(page["next_cursor"].is_string());
    let status = store
        .command("collection-status", &json!({"limit":1}))
        .unwrap();
    assert_eq!(status["sources"].as_array().unwrap().len(), 1);
    assert_eq!(status["jobs"].as_array().unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

/// A stand-in for yt-dlp that copies a real ffmpeg-made clip, so queue
/// order, failure text, cancellation and restart are exercised without a
/// network.  It records how many downloads were alive at once.
fn fake_downloader(root: &std::path::Path) -> std::path::PathBuf {
    let clip = root.join("clip.mp4");
    let status = std::process::Command::new("/usr/bin/ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:duration=2",
            "-shortest",
            "-y",
        ])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(status.success());
    let state = root.join("fake-state");
    fs::create_dir_all(state.join("active")).unwrap();
    fs::create_dir_all(state.join("ended")).unwrap();
    let script = root.join("yt-dlp");
    fs::write(&script, format!(r#"#!/bin/bash
state='{state}'; clip='{clip}'
dir=""; prev=""; for arg in "$@"; do [ "$prev" = "-P" ] && dir="$arg"; prev="$arg"; url="$arg"; done
for f in "$state"/active/*; do [ -e "$f" ] && [ ! -e "/proc/${{f##*/}}" ] && mv -f "$f" "$state/ended/"; done
touch "$state/active/$$"; ls "$state/active" | wc -l >> "$state/concurrency"; echo "$url" >> "$state/order"
trap 'mv -f "$state/active/$$" "$state/ended/"' EXIT
case "$url" in
  *fail*) echo "ERROR: [generic] Unable to download webpage: HTTP Error 404: Not Found" >&2; exit 1;;
  *slow*) if [ ! -e "$state/slow-once" ]; then touch "$state/slow-once"; echo partial > "$dir/slow.mp4.part"; sleep 30; fi;;
esac
sleep 0.3
name=$(basename "${{url%%\?*}}"); name="${{name%.*}}"; [ -n "$name" ] || name=media
[ -e "$dir/slow.mp4.part" ] && mv -f "$dir/slow.mp4.part" "$state/ended/resumed.part"
cp "$clip" "$dir/$name.mp4"
reported="$url"; case "$url" in *mismatch*) reported="https://example.org/other.mp4";; esac
printf 'soliton-media-expected\t%s\n' "$(stat -c %s "$clip")"
printf 'soliton-media-result\t{{"id":"%s","webpage_url":"%s","original_url":"%s","ext":"mp4","duration":2.0,"title":"%s","filepath":"%s"}}\n' "$name" "$reported" "$reported" "$name" "$dir/$name.mp4"
"#, state = state.display(), clip = clip.display())).unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    script
}

fn wait_media(store: &mut CollectionStore, id: &str, states: &[&str]) -> serde_json::Value {
    let mut item = json!(null);
    for _ in 0..400 {
        item = store
            .command("collection-media-status", &json!({"id":id}))
            .unwrap();
        if states.contains(&item["state"].as_str().unwrap_or("")) {
            return item;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("media job {id} did not reach {states:?}: {item}")
}

#[test]
fn media_queue_downloads_sequentially_and_keeps_failures_cancels_and_restarts() {
    let (root, _) = fixture();
    let script = fake_downloader(&root);
    let store_root = root.join("store");
    let ids;
    {
        let mut store = CollectionStore::open(&store_root).unwrap();
        store.set_media_downloader(&script, 0);
        assert!(
            store
                .command(
                    "collection-media-queue",
                    &json!({"urls":["file:///etc/passwd"]})
                )
                .is_err()
        );
        assert!(
            store
                .command(
                    "collection-media-queue",
                    &json!({"urls":["https://www.youtube.com/@channel"]})
                )
                .is_err()
        );
        let queued = store
            .command(
                "collection-media-queue",
                &json!({"urls":[
                    "https://media.example.org/one.mp4",
                    "https://media.example.org/fail.mp4",
                    "https://media.example.org/mismatch.mp4",
                    "https://media.example.org/two.mp4",
                    "https://media.example.org/one.mp4"
                ]}),
            )
            .unwrap();
        let items = queued["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            4,
            "a duplicate URL in one paste is queued once"
        );
        ids = items
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let one = wait_media(&mut store, &ids[0], &["completed"]);
        assert_eq!(
            one["result"]["bytes"].as_u64().unwrap(),
            fs::metadata(root.join("clip.mp4")).unwrap().len()
        );
        assert_eq!(one["result"]["sha256"].as_str().unwrap().len(), 64);
        assert!(
            one["result"]["streams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["codec_type"] == "video")
        );
        let failed = wait_media(&mut store, &ids[1], &["failed"]);
        assert!(
            failed["error"].as_str().unwrap().contains("HTTP Error 404"),
            "{failed}"
        );
        let mismatch = wait_media(&mut store, &ids[2], &["failed"]);
        assert!(
            mismatch["error"]
                .as_str()
                .unwrap()
                .contains("instead of the requested URL")
        );
        wait_media(&mut store, &ids[3], &["completed"]);
        let counts = fs::read_to_string(root.join("fake-state/concurrency")).unwrap();
        assert!(
            counts.lines().all(|line| line.trim() == "1"),
            "concurrent downloads: {counts}"
        );
        let order = fs::read_to_string(root.join("fake-state/order")).unwrap();
        assert_eq!(
            order.lines().collect::<Vec<_>>(),
            [
                "https://media.example.org/one.mp4",
                "https://media.example.org/fail.mp4",
                "https://media.example.org/mismatch.mp4",
                "https://media.example.org/two.mp4"
            ]
        );
        // Cancel a running job: its own process group stops, partial bytes stay visible.
        let slow = store
            .command(
                "collection-media-queue",
                &json!({"urls":["https://media.example.org/slow.mp4"]}),
            )
            .unwrap();
        let slow_id = slow["items"][0]["id"].as_str().unwrap().to_owned();
        for _ in 0..200 {
            if root.join("fake-state/slow-once").exists() {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        thread::sleep(Duration::from_millis(200));
        store
            .command("collection-media-cancel", &json!({"id":slow_id}))
            .unwrap();
        let cancelled = wait_media(&mut store, &slow_id, &["cancelled"]);
        assert_eq!(cancelled["partial"]["files"][0]["name"], "slow.mp4.part");
        // The killed stand-in and its sleeping child are gone.
        for entry in fs::read_dir(root.join("fake-state/active")).unwrap() {
            let pid = entry.unwrap().file_name();
            assert!(
                !std::path::Path::new("/proc").join(&pid).exists(),
                "process {pid:?} survived cancel"
            );
        }
        let retried = store
            .command("collection-media-retry", &json!({"id":slow_id}))
            .unwrap();
        assert!(matches!(
            retried["state"].as_str(),
            Some("queued" | "running")
        ));
        let done = wait_media(&mut store, &slow_id, &["completed"]);
        assert_eq!(done["attempts"], 2);
        assert!(
            done["history"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["state"] == "cancelled")
        );
        // Open only names a verified completed file.
        let opened = store
            .command("collection-media-open", &json!({"id":ids[0]}))
            .unwrap();
        assert!(opened["file_url"].as_str().unwrap().starts_with("file:///"));
        assert!(
            store
                .command("collection-media-open", &json!({"id":ids[1]}))
                .is_err()
        );
        let listed = store.command("collection-list", &json!({})).unwrap();
        assert_eq!(listed["acquired_media"].as_array().unwrap().len(), 3);
        let late = store
            .command(
                "collection-media-queue",
                &json!({"urls":["https://media.example.org/late.mp4"]}),
            )
            .unwrap();
        assert!(late["items"][0]["id"].is_string());
    }
    // The previous store ended while `late` was queued or running.
    {
        let mut store = CollectionStore::open(&store_root).unwrap();
        store.set_media_downloader(&script, 0);
        let list = store.command("collection-media-list", &json!({})).unwrap();
        let late = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["url"] == "https://media.example.org/late.mp4")
            .unwrap()
            .clone();
        let done = wait_media(&mut store, late["id"].as_str().unwrap(), &["completed"]);
        assert_eq!(done["url"], "https://media.example.org/late.mp4");
        let failed = store
            .command("collection-media-status", &json!({"id":ids[1]}))
            .unwrap();
        assert_eq!(failed["state"], "failed", "failure state is durable");
    }
    fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------------------
// The gate, the format probe and a channel's children (T20).

const POCOCK: &str = "https://www.youtube.com/watch?v=4DhcSPkEbwI";
const HINTON: &str = "https://www.youtube.com/watch?v=BMYvfVk8Ar0";
// The real exclusion probes of the learning-intake inputs, with his local
// category for each (music, cooking, entertainment).
const PROBES: [(&str, &str); 3] = [
    ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "music"),
    ("https://www.youtube.com/watch?v=eLkgILAkqVI", "cooking"),
    (
        "https://www.youtube.com/watch?v=zxYjTTXc-J8",
        "entertainment",
    ),
];

/// A stand-in for yt-dlp covering the format probe (-J), caption collection
/// (--skip-download), a channel listing (--flat-playlist) and a download.
/// Every call is logged with the live count of stand-ins.
fn fake_source_tool(root: &std::path::Path) -> std::path::PathBuf {
    let clip = root.join("clip.mp4");
    let status = std::process::Command::new("/usr/bin/ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:duration=2",
            "-shortest",
            "-y",
        ])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(status.success());
    let state = root.join("tool-state");
    fs::create_dir_all(state.join("active")).unwrap();
    fs::write(state.join("formats.json"), r#"{"id":"4DhcSPkEbwI","formats":[
        {"format_id":"160","ext":"mp4","vcodec":"avc1.4d400c","acodec":"none","height":144,"protocol":"https"},
        {"format_id":"134","ext":"mp4","vcodec":"avc1.4d401e","acodec":"none","height":360,"protocol":"https"},
        {"format_id":"137","ext":"mp4","vcodec":"avc1.640028","acodec":"none","height":1080,"protocol":"https"},
        {"format_id":"140-19","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","abr":129.5,"format_note":"Spanish dubbed","protocol":"https"},
        {"format_id":"140-0","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","abr":129.5,"format_note":"English original (default)","protocol":"https"},
        {"format_id":"251","ext":"webm","vcodec":"none","acodec":"opus","abr":140.0,"protocol":"https"}]}"#).unwrap();
    let script = root.join("yt-dlp");
    fs::write(&script, format!(r#"#!/bin/bash
state='{state}'; clip='{clip}'
mode=download; dir=""; out=""; prev=""
for arg in "$@"; do
  case "$arg" in -J) mode=probe;; --flat-playlist) mode=channel;; --skip-download) [ "$mode" = download ] && mode=captions;; esac
  [ "$prev" = "-P" ] && dir="$arg"; [ "$prev" = "-o" ] && out="$arg"; [ "$prev" = "-f" ] && echo "$arg" >> "$state/formats"
  prev="$arg"; url="$arg"
done
for f in "$state"/active/*; do [ -e "$f" ] && [ ! -e "/proc/${{f##*/}}" ] && rm -f "$f"; done
touch "$state/active/$$"; echo "$mode $(ls "$state/active" | wc -l) $url" >> "$state/calls"
trap 'rm -f "$state/active/$$"' EXIT
id="${{url##*v=}}"
case "$mode" in
  probe) grep -qx "$id" "$state/unavailable" 2>/dev/null && {{ echo "ERROR: [youtube] $id: Video unavailable" >&2; exit 1; }}
         [ -f "$state/meta/$id.json" ] && {{ cat "$state/meta/$id.json"; exit 0; }}
         sed "s/4DhcSPkEbwI/$id/" "$state/formats.json"; exit 0;;
  channel) cat "$state/channel.json"; exit 0;;
  captions) sleep 0.2; base="${{out%%\%*}}"
         printf '{{"id":"%s","title":"Video %s","channel":"Fixture","duration":3}}' "$id" "$id" > "$base$id.info.json"
         printf '{{"events":[{{"tStartMs":0,"dDurationMs":1000,"segs":[{{"utf8":"caption of %s"}}]}}]}}' "$id" > "$base$id.en.json3"; exit 0;;
esac
cp "$clip" "$dir/$id.mp4"
printf 'soliton-media-result\t{{"id":"%s","webpage_url":"%s","original_url":"%s","ext":"mp4","duration":2.0,"title":"%s","filepath":"%s"}}\n' "$id" "$url" "$url" "$id" "$dir/$id.mp4"
"#, state = state.display(), clip = clip.display())).unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    // The page fetcher's stand-in answers the public oEmbed page from
    // tool-state/oembed and refuses everything else, as an offline curl would.
    let curl = root.join("curl");
    fs::write(
        &curl,
        format!(
            r#"#!/bin/bash
state='{state}'
for arg in "$@"; do url="$arg"; done
id="${{url: -11}}"; echo "oembed $id" >> "$state/calls"
[ -f "$state/oembed/$id.json" ] && {{ cat "$state/oembed/$id.json"; exit 0; }}
echo "curl: (22) The requested URL returned error: 404" >&2; exit 22
"#,
            state = state.display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&curl)
        .status()
        .unwrap();
    script
}

/// The probes' and the Pocock video's own metadata as yt-dlp returned it on
/// 16 Oct (trimmed), and the oEmbed page of the cooking probe, which yt-dlp
/// calls unavailable.  Nothing here names his categories.
fn real_metadata(root: &std::path::Path) {
    let real: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/learning_real_metadata.json")).unwrap();
    let state = root.join("tool-state");
    fs::create_dir_all(state.join("meta")).unwrap();
    fs::create_dir_all(state.join("oembed")).unwrap();
    for (key, value) in real.as_object().unwrap() {
        match key.strip_prefix("oembed-") {
            Some(id) => fs::write(
                state.join("oembed").join(format!("{id}.json")),
                value.to_string(),
            )
            .unwrap(),
            None => fs::write(
                state.join("meta").join(format!("{key}.json")),
                value.to_string(),
            )
            .unwrap(),
        }
    }
    fs::write(state.join("unavailable"), "eLkgILAkqVI\n").unwrap();
}

/// His local rules for the probes: each video mapped to its category with
/// the evidence, and the three categories excluded.
fn block_probes(prefs_root: &std::path::Path) {
    let mut prefs = soliton::source_preferences::SourcePreferences::open(prefs_root).unwrap();
    for (url, category) in PROBES {
        prefs
            .command("source-metadata-set", &json!({"locator":"video","pattern":url,"category":category,
                "evidence":"learning-intake inputs/REAL_SOURCES.md: public title page establishes this source"}))
            .unwrap();
    }
    for category in ["music", "cooking", "entertainment"] {
        prefs
            .command(
                "source-policy-set",
                &json!({"scope":"global","kind":"exclude","matcher":"category","pattern":category}),
            )
            .unwrap();
    }
}

fn wait_job(store: &mut CollectionStore, source_url_part: &str) -> serde_json::Value {
    for _ in 0..400 {
        let status = store.command("collection-status", &json!({})).unwrap();
        let sources = status["sources"].as_array().cloned().unwrap_or_default();
        if let Some(source) = sources.iter().find(|s| {
            s["location"]
                .as_str()
                .is_some_and(|l| l.contains(source_url_part))
        }) {
            if let Some(job) = status["jobs"].as_array().unwrap().iter().find(|job| {
                job["source_id"] == source["id"]
                    && matches!(job["state"].as_str(), Some("completed" | "failed"))
            }) {
                return job.clone();
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("no finished job for {source_url_part}")
}

#[test]
fn media_format_is_chosen_from_the_offer_not_a_pin() {
    let (root, _) = fixture();
    let tool = fake_source_tool(&root);
    let mut store = CollectionStore::open(&root.join("collection")).unwrap();
    store.set_media_downloader(&tool, 0);
    let queued = store
        .command("collection-media-queue", &json!({"urls":[POCOCK]}))
        .unwrap();
    let id = queued["items"][0]["id"].as_str().unwrap().to_owned();
    let done = wait_media(&mut store, &id, &["completed"]);
    let chosen = fs::read_to_string(root.join("tool-state/formats")).unwrap();
    println!("format chosen from the offer: {}", chosen.trim());
    assert_eq!(
        chosen.trim(),
        "134+140-0",
        "360p H.264 with the original-language audio, never the dubbed track"
    );
    assert!(
        done["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["detail"].as_str().unwrap().contains("134+140-0"))
    );
    // An unavailable video fails with the tool's own words.
    fs::write(root.join("tool-state/unavailable"), "BMYvfVk8Ar0\n").unwrap();
    let queued = store
        .command("collection-media-queue", &json!({"urls":[HINTON]}))
        .unwrap();
    let failed = wait_media(
        &mut store,
        queued["items"][0]["id"].as_str().unwrap(),
        &["failed"],
    );
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("Video unavailable"),
        "{failed}"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blocked_probes_are_refused_on_every_path_and_pocock_is_admitted() {
    let (root, _) = fixture();
    let tool = fake_source_tool(&root);
    let collection = root.join("collection");
    let prefs_root = root.join("source-preferences");
    // The three probes were configured before his rules existed, and one of
    // them waits in the media queue as a failed download.
    fs::create_dir_all(collection.join("sources")).unwrap();
    for (index, (url, _)) in PROBES.iter().enumerate() {
        let id = format!("collection-youtube-video-probe{index}");
        fs::write(collection.join("sources").join(format!("{id}.json")), json!({"schema":1,"id":id,"kind":"youtube-video","location":url,
            "cadence_hours":12,"enabled":true,"auto_briefing":false,"goal_ids":[],"created_at":"2026-10-30T00:00:00Z",
            "last_success":null,"last_error":null,"next_due_at":0}).to_string()).unwrap();
    }
    let retry_id;
    {
        let mut store = CollectionStore::open(&collection).unwrap();
        store.set_media_downloader(&tool, 0);
        fs::write(root.join("tool-state/unavailable"), "dQw4w9WgXcQ\n").unwrap();
        let queued = store
            .command("collection-media-queue", &json!({"urls":[PROBES[0].0]}))
            .unwrap();
        retry_id = queued["items"][0]["id"].as_str().unwrap().to_owned();
        wait_media(&mut store, &retry_id, &["failed"]);
    }
    block_probes(&prefs_root);
    let mut store = CollectionStore::open(&collection).unwrap();
    store.set_media_downloader(&tool, 0);
    store.set_source_downloader(&tool);
    store.set_page_fetcher(&root.join("curl"));
    store.set_automatic_capture(true);
    let mut refusals = Vec::new();
    for (url, _) in PROBES {
        let error = store
            .command("collection-add", &json!({"type":"youtube-video","url":url}))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Excluded by your local source preferences"),
            "{error}"
        );
        refusals.push(format!("add     {url}: {error}"));
    }
    for index in 0..3 {
        let id = format!("collection-youtube-video-probe{index}");
        let error = store
            .command("collection-check", &json!({"id":id}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Excluded"), "{error}");
        refusals.push(format!("check   {}: {error}", PROBES[index].0));
    }
    let due = store.command("collection-poll-due", &json!({})).unwrap();
    for item in due["excluded_due"].as_array().unwrap() {
        refusals.push(format!(
            "cadence {}: {}",
            item["location"].as_str().unwrap(),
            item["policy"]["reason"].as_str().unwrap()
        ));
    }
    fs::write(root.join("tool-state/channel.json"), json!({"title":"Probe channel","channel":"Mixed",
        "entries":PROBES.iter().map(|(url, _)| json!({"id":&url[url.len()-11..],"title":"probe"})).collect::<Vec<_>>()}).to_string()).unwrap();
    store
        .command(
            "collection-add",
            &json!({"type":"youtube-channel","url":"https://www.youtube.com/@mixedprobes"}),
        )
        .unwrap();
    let channel_job = wait_job(&mut store, "@mixedprobes");
    assert_eq!(channel_job["state"], "completed", "{channel_job}");
    for child in channel_job["refused_children"].as_array().unwrap() {
        refusals.push(format!(
            "child   {}: {}",
            child["url"].as_str().unwrap(),
            child["reason"].as_str().unwrap()
        ));
    }
    let mut extra = Vec::new();
    for (url, _) in PROBES {
        let error = store
            .command("collection-media-queue", &json!({"urls":[url]}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Excluded"), "{error}");
        extra.push(format!("media   {url}"));
    }
    let error = store
        .command("collection-media-retry", &json!({"id":retry_id}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Excluded"), "{error}");
    extra.push(format!("retry   {}", PROBES[0].0));
    for line in &refusals {
        println!("refused {line}");
    }
    println!(
        "refusals on add, check, cadence and as channel children: {} of 12",
        refusals.len()
    );
    println!(
        "also refused on the media queue and its retry: {} of 4",
        extra.len()
    );
    assert_eq!(refusals.len(), 12);
    // Nothing of a probe is left behind: no job, no result, no transcript.
    let status = store.command("collection-status", &json!({})).unwrap();
    let probe_sources = status["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| PROBES.iter().any(|(url, _)| s["location"] == *url))
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let probe_jobs = status["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| probe_sources.iter().any(|id| job["source_id"] == *id))
        .count();
    let results = fs::read_dir(collection.join("results"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let channel_result = fs::read_to_string(
        collection
            .join("results")
            .join(format!("{}.json", channel_job["id"].as_str().unwrap())),
    )
    .unwrap();
    println!("probe jobs: {probe_jobs}; results folder: {results:?}");
    assert_eq!(probe_jobs, 0);
    assert_eq!(
        results.len(),
        1,
        "only the channel's own listing result: {results:?}"
    );
    assert!(
        PROBES
            .iter()
            .all(|(url, _)| !channel_result.contains(&url[url.len() - 11..])),
        "a refused child's line left the listing"
    );
    let calls = fs::read_to_string(root.join("tool-state/calls")).unwrap();
    assert!(PROBES.iter().all(|(url, _)| {
        !calls
            .lines()
            .any(|line| line.starts_with("captions") && line.contains(&url[url.len() - 11..]))
    }));
    // The wanted Pocock video, which YouTube itself files as Entertainment, is admitted.
    let admitted = store
        .command(
            "collection-add",
            &json!({"type":"youtube-video","url":POCOCK}),
        )
        .unwrap();
    assert!(admitted["job_id"].is_string(), "{admitted}");
    let job = wait_job(&mut store, "4DhcSPkEbwI");
    assert_eq!(job["state"], "completed", "{job}");
    println!("Pocock video admitted and collected: {}", job["state"]);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// The three probes already in a store, added before his rules, as on his copy.
fn probe_sources(collection: &std::path::Path) {
    fs::create_dir_all(collection.join("sources")).unwrap();
    for (index, (url, _)) in PROBES.iter().enumerate() {
        let id = format!("collection-youtube-video-probe{index}");
        fs::write(collection.join("sources").join(format!("{id}.json")), json!({"schema":1,"id":id,"kind":"youtube-video","location":url,
            "cadence_hours":12,"enabled":true,"auto_briefing":false,"goal_ids":[],"created_at":"2026-10-30T00:00:00Z",
            "last_success":null,"last_error":null,"next_due_at":0}).to_string()).unwrap();
    }
}

/// With only his three category rules and no hand-entered metadata, the store
/// learns each video's category from the video's own metadata on whichever
/// path meets it first (each path in a fresh store), refuses the probes, and
/// admits the Pocock video that YouTube files as Entertainment.
#[test]
fn categories_are_learned_from_the_source_itself_on_every_path() {
    let (root, _) = fixture();
    let tool = fake_source_tool(&root);
    real_metadata(&root);
    let prefs_root = root.join("source-preferences");
    let mut prefs = soliton::source_preferences::SourcePreferences::open(&prefs_root).unwrap();
    for category in ["music", "cooking", "entertainment"] {
        prefs
            .command(
                "source-policy-set",
                &json!({"scope":"global","kind":"exclude","matcher":"category","pattern":category}),
            )
            .unwrap();
    }
    assert_eq!(
        prefs.command("source-metadata-list", &json!({})).unwrap()["total"],
        0,
        "no hand-entered metadata"
    );
    drop(prefs);
    let open = |name: &str| {
        let collection = root.join(name);
        let mut store = CollectionStore::open(&collection).unwrap();
        store.set_media_downloader(&tool, 0);
        store.set_source_downloader(&tool);
        store.set_page_fetcher(&root.join("curl"));
        store.set_automatic_capture(true);
        (collection, store)
    };
    let mut refusals = Vec::new();
    let (add_root, mut store) = open("c-add");
    for (url, _) in PROBES {
        let error = store
            .command("collection-add", &json!({"type":"youtube-video","url":url}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Excluded"), "{error}");
        refusals.push(format!("add     {url}: {error}"));
    }
    drop(store);
    let (check_root, mut store) = {
        probe_sources(&root.join("c-check"));
        open("c-check")
    };
    for (index, (url, _)) in PROBES.iter().enumerate() {
        let error = store
            .command(
                "collection-check",
                &json!({"id":format!("collection-youtube-video-probe{index}")}),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("Excluded"), "{error}");
        refusals.push(format!("check   {url}: {error}"));
    }
    drop(store);
    let (_, mut store) = {
        probe_sources(&root.join("c-cadence"));
        open("c-cadence")
    };
    let due = store.command("collection-poll-due", &json!({})).unwrap();
    for item in due["excluded_due"].as_array().unwrap() {
        refusals.push(format!(
            "cadence {}: {}",
            item["location"].as_str().unwrap(),
            item["policy"]["reason"].as_str().unwrap()
        ));
    }
    drop(store);
    fs::write(root.join("tool-state/channel.json"), json!({"title":"Probe channel","channel":"Mixed",
        "entries":PROBES.iter().map(|(url, _)| json!({"id":&url[url.len()-11..],"title":"probe"})).chain([json!({"id":"4DhcSPkEbwI","title":"AI Skills"})]).collect::<Vec<_>>()}).to_string()).unwrap();
    let (channel_root, mut store) = open("c-channel");
    store
        .command(
            "collection-add",
            &json!({"type":"youtube-channel","url":"https://www.youtube.com/@mixedprobes"}),
        )
        .unwrap();
    let channel_job = wait_job(&mut store, "@mixedprobes");
    assert_eq!(channel_job["state"], "completed", "{channel_job}");
    for child in channel_job["refused_children"].as_array().unwrap() {
        refusals.push(format!(
            "child   {}: {}",
            child["url"].as_str().unwrap(),
            child["reason"].as_str().unwrap()
        ));
    }
    assert_eq!(
        channel_job["child_sources"].as_array().unwrap().len(),
        1,
        "the Pocock child is admitted: {channel_job}"
    );
    drop(store);
    let (media_root, mut store) = open("c-media");
    let mut extra = Vec::new();
    for (url, _) in PROBES {
        let error = store
            .command("collection-media-queue", &json!({"urls":[url]}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Excluded"), "{error}");
        extra.push(format!("media   {url}"));
    }
    for line in &refusals {
        println!("refused {line}");
    }
    println!(
        "learned-category refusals on add, check, cadence and as channel children: {} of 12",
        refusals.len()
    );
    println!("also refused on the media queue: {} of 3", extra.len());
    assert_eq!(refusals.len(), 12);
    // What was learned, with its evidence, in each path's own store.
    for (path, collection) in [
        ("add", &add_root),
        ("check", &check_root),
        ("channel", &channel_root),
        ("media", &media_root),
    ] {
        let learned: serde_json::Value = serde_json::from_slice(
            &fs::read(collection.join("intake/learned-sources.json")).unwrap(),
        )
        .unwrap();
        for (url, category) in PROBES {
            let record = &learned["items"][&url[url.len() - 11..]];
            assert_eq!(
                record["categories"][0]["category"], category,
                "{path}: {record}"
            );
            println!(
                "{path:7} {} -> {category} | {}",
                &url[url.len() - 11..],
                record["categories"][0]["evidence"].as_str().unwrap()
            );
        }
    }
    // The Pocock video: YouTube says Entertainment, its own words say learning.
    let admitted = store
        .command(
            "collection-add",
            &json!({"type":"youtube-video","url":POCOCK}),
        )
        .unwrap();
    let learned: serde_json::Value =
        serde_json::from_slice(&fs::read(media_root.join("intake/learned-sources.json")).unwrap())
            .unwrap();
    let pocock = &learned["items"]["4DhcSPkEbwI"];
    println!(
        "Pocock: provider {} | learning words {} | categories {}",
        pocock["provider_categories"], pocock["learning_words"], pocock["categories"]
    );
    assert_eq!(pocock["provider_categories"], json!(["entertainment"]));
    assert_eq!(pocock["categories"], json!([]));
    let job = wait_job(&mut store, "4DhcSPkEbwI");
    assert_eq!(job["state"], "completed", "{admitted} {job}");
    let calls = fs::read_to_string(root.join("tool-state/calls")).unwrap();
    assert!(PROBES.iter().all(|(url, _)| {
        !calls
            .lines()
            .any(|line| line.starts_with("captions") && line.contains(&url[url.len() - 11..]))
    }));
    assert!(
        !calls.lines().any(|line| line.starts_with("download")),
        "nothing was downloaded"
    );
    let lookups = calls
        .lines()
        .filter(|line| line.starts_with("probe ") || line.starts_with("oembed "))
        .count();
    println!(
        "metadata lookups: {lookups} (yt-dlp -J and oEmbed); captions for probes: 0; downloads: 0"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn channel_children_pass_the_gate_one_by_one() {
    let (root, _) = fixture();
    let tool = fake_source_tool(&root);
    let collection = root.join("collection");
    let mut prefs =
        soliton::source_preferences::SourcePreferences::open(&root.join("source-preferences"))
            .unwrap();
    prefs.command("source-metadata-set", &json!({"locator":"video","pattern":PROBES[0].0,"category":"music","evidence":"public title page"})).unwrap();
    prefs
        .command(
            "source-policy-set",
            &json!({"scope":"global","kind":"exclude","matcher":"category","pattern":"music"}),
        )
        .unwrap();
    drop(prefs);
    fs::write(root.join("tool-state/channel.json"), json!({"title":"Fixture channel","channel":"Fixture","entries":[
        {"id":"4DhcSPkEbwI","title":"AI Skills"},{"id":"BMYvfVk8Ar0","title":"Evolution of Deep Learning"},{"id":"dQw4w9WgXcQ","title":"Music video"}]}).to_string()).unwrap();
    let mut store = CollectionStore::open(&collection).unwrap();
    store.set_source_downloader(&tool);
    store.set_page_fetcher(&root.join("curl"));
    store
        .command(
            "collection-add",
            &json!({"type":"youtube-channel","url":"https://www.youtube.com/@fixture"}),
        )
        .unwrap();
    let channel = wait_job(&mut store, "@fixture");
    let admitted = channel["child_sources"].as_array().unwrap().len();
    let refused = channel["refused_children"].as_array().unwrap();
    println!(
        "channel children: admitted {admitted}, refused {} ({})",
        refused.len(),
        refused[0]["url"]
    );
    assert_eq!((admitted, refused.len()), (2, 1));
    assert!(refused[0]["url"].as_str().unwrap().contains("dQw4w9WgXcQ"));
    wait_job(&mut store, "4DhcSPkEbwI");
    let hinton = wait_job(&mut store, "BMYvfVk8Ar0");
    assert_eq!(hinton["state"], "completed");
    let transcript = fs::read_to_string(hinton["result_path"].as_str().unwrap()).unwrap();
    assert!(transcript.contains("caption of BMYvfVk8Ar0"));
    let calls = fs::read_to_string(root.join("tool-state/calls")).unwrap();
    let captions = calls
        .lines()
        .filter(|line| line.starts_with("captions"))
        .collect::<Vec<_>>();
    println!("caption collections: {captions:?}");
    assert_eq!(captions.len(), 2);
    assert!(
        captions
            .iter()
            .all(|line| line.split(' ').nth(1) == Some("1")),
        "one collection at a time: {calls}"
    );
    assert!(!calls.contains("captions 1 https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// The one real public download, run by hand under the campaign's limits:
/// `T20_REAL_ROOT=<store> T20_REAL_URL=<url> collection_contract --ignored real_`.
#[test]
#[ignore]
fn real_public_download_completes_and_opens() {
    let root = std::path::PathBuf::from(std::env::var("T20_REAL_ROOT").expect("T20_REAL_ROOT"));
    let url = std::env::var("T20_REAL_URL").expect("T20_REAL_URL");
    let mut store = CollectionStore::open(&root).unwrap();
    let queued = store
        .command("collection-media-queue", &json!({"urls":[url]}))
        .unwrap();
    let id = queued["items"][0]["id"].as_str().unwrap().to_owned();
    let mut item = json!(null);
    for _ in 0..(30 * 60 * 2) {
        item = store
            .command("collection-media-status", &json!({"id":id}))
            .unwrap();
        if matches!(
            item["state"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    println!(
        "state {} attempts {} error {}",
        item["state"], item["attempts"], item["error"]
    );
    for event in item["history"].as_array().unwrap() {
        println!(
            "history {} {} {}",
            event["at"], event["state"], event["detail"]
        );
    }
    assert_eq!(item["state"], "completed", "{item}");
    let opened = store
        .command("collection-media-open", &json!({"id":id}))
        .unwrap();
    println!("open {}", serde_json::to_string(&opened).unwrap());
    assert_eq!(opened["mime"], "video/mp4");
    let listed = store.command("collection-media-list", &json!({})).unwrap();
    println!("acquired {}", listed["acquired"].as_array().unwrap().len());
}
