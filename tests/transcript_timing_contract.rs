use serde_json::{Value, json};
use soliton::history::{HistoryStore, segment_word_timings};
use std::path::Path;

#[test]
fn real_word_timings_pass_through_unchanged() {
    let measured = json!([{"text":"one","start_ms":12,"end_ms":37,"estimated":false},
        {"text":"two","start_ms":49,"end_ms":91,"estimated":false}]);
    assert_eq!(
        json!(segment_word_timings(&json!({"text":"one two","words":measured})).unwrap()),
        measured
    );
}

#[test]
fn estimated_words_fill_segment_exactly_by_character_length() {
    let words =
        segment_word_timings(&json!({"text":"a éé xyz","start_ms":17,"end_ms":118})).unwrap();
    assert_eq!(words[0]["start_ms"], 17);
    assert_eq!(words[0]["end_ms"], 33);
    assert_eq!(words[1]["end_ms"], 67);
    assert_eq!(words[2]["end_ms"], 118);
    for pair in words.windows(2) {
        assert_eq!(pair[0]["end_ms"], pair[1]["start_ms"]);
    }
    assert!(words.iter().all(|word| word["estimated"] == true));
}

#[test]
fn empty_segment_returns_no_words() {
    assert!(
        segment_word_timings(&json!({"text":" \n "}))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn real_data_copy_word_timing_call() {
    let Ok(root) = std::env::var("T011_DATA_COPY") else {
        return;
    };
    let mut history = HistoryStore::open(&Path::new(&root).join("history")).unwrap();
    let list = history
        .command("history-list", &json!({"provider":"voxtype","limit":100}))
        .unwrap();
    let record = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "voice_note")
        .unwrap();
    let result = history
        .command("history-word-timings", &json!({"id":record["id"]}))
        .unwrap();
    let words = result["segments"][0]["words"].as_array().unwrap();
    assert!(!words.is_empty());
    let redacted: Vec<Value> = words.iter().map(|w| json!({"length":w.get("text").or_else(||w.get("word")).and_then(Value::as_str).unwrap().chars().count(),"start_ms":w["start_ms"],"end_ms":w["end_ms"],"estimated":w["estimated"]})).collect();
    println!(
        "T011_TIMINGS {}",
        json!({"record_id":record["id"],"words":redacted,"estimated":words.iter().filter(|w|w["estimated"]==true).count(),"total":words.len()})
    );
}
