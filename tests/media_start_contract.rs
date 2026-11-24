//! A media download the previous engine left running is queued again when the engine starts, before any
//! collection command (the service hook carried in from T20).
use serde_json::{Value, json};
use soliton::service::Service;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("T46-{label}-{nanos}"));
    fs::create_dir_all(&dir).expect("scratch folder");
    dir
}

#[test]
fn a_media_job_left_running_is_queued_again_at_start() {
    let root = scratch("media-start");
    drop(Service::open(&root).expect("workspace opens"));
    let jobs = root.join("collection/media/jobs");
    fs::create_dir_all(&jobs).expect("jobs folder");
    let job = json!({"id":"m-left","state":"running","stage":"downloading","pid":null,
        "url":"https://www.youtube.com/watch?v=aaaaaaaaaaa","history":[]});
    fs::write(jobs.join("m-left.json"), job.to_string()).expect("job written");

    let service = Service::open(&root).expect("workspace reopens");
    let after: Value =
        serde_json::from_slice(&fs::read(jobs.join("m-left.json")).expect("job read"))
            .expect("job JSON");
    println!(
        "after start, with no collection command: state {} stage {}",
        after["state"], after["stage"]
    );
    assert_eq!(after["state"], "queued");
    assert_eq!(after["stage"], "queued");
    drop(service);

    // A workspace with no job left running starts as before: no collection store is made.
    let quiet = scratch("media-quiet");
    drop(Service::open(&quiet).expect("workspace opens"));
    drop(Service::open(&quiet).expect("workspace reopens"));
    assert!(!quiet.join("collection").exists());
}
