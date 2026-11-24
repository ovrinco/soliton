use serde_json::json;
use soliton::history::HistoryStore;
use soliton_kernel::ObjectId;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("history-scale-test").unwrap().to_string());
    fs::create_dir_all(&path).unwrap();
    path
}

fn rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse().ok())
            })
        })
        .unwrap_or(0)
}

fn write_recording(root: &std::path::Path, number: usize) -> PathBuf {
    let recording = root.join(format!("recording-{number}"));
    fs::create_dir_all(&recording).unwrap();
    fs::write(
        recording.join("metadata.json"),
        format!(
            "{{\"id\":\"scale-{number}\",\"title\":\"Scale {number}\",\"started_at\":\"2026-09-21T00:{number:02}:00Z\",\"status\":\"completed\"}}"
        ),
    )
    .unwrap();
    let mut transcript = String::from("{\"segments\":[");
    for index in 0..100_000usize {
        if index != 0 {
            transcript.push(',');
        }
        let text = if index % 10_000 == 0 {
            format!("record {number} needle segment {index}")
        } else {
            format!("record {number} segment {index}")
        };
        transcript.push_str(&format!(
            "{{\"id\":{},\"start_ms\":{},\"end_ms\":{},\"text\":{}}}",
            index,
            index * 10,
            index * 10 + 9,
            serde_json::to_string(&text).unwrap()
        ));
    }
    transcript.push_str("]}");
    fs::write(recording.join("transcript.json"), transcript).unwrap();
    recording
}

#[test]
fn million_segment_archive_stays_pageable_and_searchable() {
    let root = root();
    let archive = root.join("archive");
    let rss_before = rss_kib();
    let started = Instant::now();
    let mut store = HistoryStore::open(&archive).unwrap();
    let startup_empty = started.elapsed();
    for number in 0..10 {
        let recording = write_recording(&root, number);
        store
            .command(
                "history-import",
                &json!({"path":recording,"provider":"voxtype"}),
            )
            .unwrap();
    }
    let rss_after_import = rss_kib();
    let list_started = Instant::now();
    let page = store
        .command(
            "history-list",
            &json!({"kind":"voice_note","offset":3,"limit":4}),
        )
        .unwrap();
    let list_time = list_started.elapsed();
    assert_eq!(page["total"], 10);
    assert_eq!(page["items"].as_array().unwrap().len(), 4);
    assert!(page["items"][0].get("segments").is_none());
    let search_started = Instant::now();
    let found = store
        .command(
            "history-list",
            &json!({"kind":"voice_note","q":"needle","limit":10}),
        )
        .unwrap();
    let search_time = search_started.elapsed();
    assert_eq!(found["total"], 10);
    assert_eq!(found["items"].as_array().unwrap().len(), 10);
    drop(store);
    let restart_started = Instant::now();
    let mut reopened = HistoryStore::open(&archive).unwrap();
    let restart_time = restart_started.elapsed();
    let rss_after_restart = rss_kib();
    let detail = reopened
        .command("history-detail", &json!({"id":found["items"][0]["id"]}))
        .unwrap();
    assert_eq!(detail["segments"].as_array().unwrap().len(), 100_000);
    eprintln!(
        "history scale: empty_startup={startup_empty:?} list={list_time:?} search={search_time:?} restart={restart_time:?} rss_kib={rss_before}->{rss_after_import}->{rss_after_restart}"
    );
    fs::remove_dir_all(root).unwrap();
}
