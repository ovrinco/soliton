use serde_json::{Value, json};
use soliton::history::HistoryStore;
#[test]
fn prepare_explicit_bounded_canonical_reference_fixture() {
    let path = std::env::var("SOLITON_FINAL_FIXTURE_CONTRACT").expect("explicit fixture contract");
    let contract: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let root = std::path::PathBuf::from(contract["workspace"].as_str().unwrap());
    let mut store = HistoryStore::open(&root.join("history")).unwrap();
    let mut records = vec![];
    for source in contract["voice_sources"].as_array().unwrap() {
        let reply = store
            .command(
                "history-import",
                &json!({"provider":"voxtype","path":source}),
            )
            .unwrap();
        assert_eq!(reply["record"]["coverage"]["state"], "complete", "{reply}");
        let archive = std::path::PathBuf::from(reply["record"]["archive_path"].as_str().unwrap());
        assert!(!archive.join("audio.wav").exists());
        records.push(json!({"id":reply["record"]["id"],"title":reply["record"]["title"],"segments":reply["record"]["segments"].as_array().unwrap().len(),"source":source,"audio":reply["record"]["audio"]}));
    }
    std::fs::write(
        root.parent().unwrap().join("voice-fixture-receipt.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
    assert_eq!(records.len(), 19);
}
