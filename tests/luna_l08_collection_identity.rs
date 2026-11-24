use serde_json::json;
use soliton::collection::CollectionStore;
use soliton_kernel::ObjectId;
use std::{fs, path::PathBuf};

// Both URLs are derived from the retained source URL
// https://youtu.be/tn2v3AgAyFc?si=wOjaOvxEMuA6Vf44 in the copied history corpus.
// The explicit disabled flag keeps this admission/identity regression offline;
// it does not fabricate captions or invoke the provider.
const RETAINED_SHORT_URL: &str = "https://youtu.be/tn2v3AgAyFc?si=wOjaOvxEMuA6Vf44";
const RETAINED_WATCH_URL: &str = "https://www.youtube.com/watch?v=tn2v3AgAyFc";

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(
        ObjectId::mint("luna-l08-collection-id")
            .unwrap()
            .to_string(),
    );
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn equivalent_retained_youtube_urls_have_one_admission_identity() {
    let dir = root();
    let mut store = CollectionStore::open(&dir.join("store")).unwrap();
    let first = store
        .command(
            "collection-add",
            &json!({"type":"youtube-video","url":RETAINED_SHORT_URL,"enabled":false}),
        )
        .unwrap();
    let second = store
        .command(
            "collection-add",
            &json!({"type":"youtube-video","url":RETAINED_WATCH_URL,"enabled":false}),
        )
        .unwrap();
    let status = store.command("collection-status", &json!({})).unwrap();

    assert_eq!(first["kind"], "youtube-video");
    assert_eq!(second["already_present"], true);
    assert_eq!(second["id"], first["id"]);
    assert_eq!(status["sources"].as_array().unwrap().len(), 1);
    assert_eq!(status["jobs"].as_array().unwrap().len(), 0);
    assert_eq!(status["sources"][0]["location"], RETAINED_SHORT_URL);
    fs::remove_dir_all(dir).unwrap();
}
