// T08: one Rust-owned task record behind every planner view, recurrence in his local time.
// Rows T001-166 (one record, every view), T001-167 (three states), T001-168 (consecutive local
// dates across the March and November changes in America/Toronto), T001-169 (one instance edited
// leaves its neighbours).
use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::*;
use std::{fs, path::PathBuf};

const ZONE: &str = "America/Toronto";

fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("soliton-test").unwrap().to_string());
    fs::create_dir(&p).unwrap();
    p
}
fn save(s: &mut Service, v: Value) -> Value {
    s.command("save", &v).unwrap()
}
fn calendar(s: &mut Service, from: &str, through: &str) -> Vec<Value> {
    s.command(
        "calendar",
        &json!({"from":from,"through":through,"timezone":ZONE}),
    )
    .unwrap()
    .as_array()
    .unwrap()
    .clone()
}
fn record(s: &Service, id: &Value) -> Value {
    s.state().unwrap()["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| &v["id"] == id)
        .unwrap()
        .clone()
}
// What one occurrence shows: everything in its projection except the series' own version stamp
// (hash, revision, override map), which moves whenever any part of the series is saved.
fn instance(v: &Value) -> String {
    let mut v = v.clone();
    for k in ["hash", "revision", "overrides"] {
        v.as_object_mut().unwrap().remove(k);
    }
    serde_json::to_string(&v).unwrap()
}
fn timed() -> Value {
    json!({"time":"09:00","duration_minutes":30,"timezone":ZONE,"fold":"earlier"})
}

#[test]
fn editing_one_weekly_instance_leaves_its_neighbours() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Weekly review","scheduled_for":"2026-11-01",
        "recurrence":"weekly","interval":1,"schedule":timed()}),
    );
    let before = calendar(&mut s, "2026-11-01", "2026-11-15");
    let dates: Vec<&str> = before
        .iter()
        .map(|v| v["occurrence_date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2026-11-01", "2026-11-08", "2026-11-15"]);
    let middle = &before[1];
    s.command(
        "occurrence",
        &json!({"id":t["id"],"hash":t["hash"],"occurrence":middle["occurrence"],
        "occurrence_date":middle["occurrence_date"],"title":"Weekly review, moved",
        "date":"2026-11-10","status":"in_progress",
        "schedule":{"time":"14:00","duration_minutes":45,"timezone":ZONE,"fold":"earlier"}}),
    )
    .unwrap();
    let after = calendar(&mut s, "2026-11-01", "2026-11-15");
    assert_eq!(after.len(), 3);
    let edited = after
        .iter()
        .find(|v| v["occurrence"] == middle["occurrence"])
        .unwrap();
    assert_eq!(edited["title"], "Weekly review, moved");
    assert_eq!(edited["local_start"], "2026-11-10T14:00");
    assert_eq!(edited["status"], "in_progress");
    for n in [&before[0], &before[2]] {
        let now = after
            .iter()
            .find(|v| v["occurrence"] == n["occurrence"])
            .unwrap();
        assert_eq!(instance(n), instance(now), "a neighbour changed");
        println!(
            "neighbour {} unchanged: {} bytes equal",
            n["occurrence_date"],
            instance(now).len()
        );
    }
    // The series record itself is untouched by an instance edit.
    let series = record(&s, &t["id"]);
    assert_eq!(series["title"], "Weekly review");
    assert_eq!(series["status"], "todo");
    drop(s);
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn daily_instances_fall_on_consecutive_local_dates_across_both_changes() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let t = save(
        &mut s,
        json!({"kind":"task","title":"Daily practice","scheduled_for":"2026-03-05",
        "recurrence":"daily","interval":1,"schedule":timed()}),
    );
    let all_day = save(
        &mut s,
        json!({"kind":"task","title":"Daily note","scheduled_for":"2026-03-05",
        "recurrence":"daily","interval":1}),
    );
    // 2026: clocks go forward Sunday 8 March, back Sunday 1 November in America/Toronto.
    for (from, through, change, hours) in [
        ("2026-03-05", "2026-03-11", "2026-03-08", 23),
        ("2026-10-29", "2026-11-04", "2026-11-01", 25),
    ] {
        let events = calendar(&mut s, from, through);
        for (id, label) in [(&t["id"], "timed"), (&all_day["id"], "all-day")] {
            let mine: Vec<&Value> = events.iter().filter(|v| &v["id"] == id).collect();
            let dates: Vec<String> = mine
                .iter()
                .map(|v| v["occurrence_date"].as_str().unwrap().to_owned())
                .collect();
            let mut want = Vec::new();
            let mut d: CivilDate = from.parse().unwrap();
            let end: CivilDate = through.parse().unwrap();
            while d <= end {
                want.push(d.to_string());
                d = d.add_days(1).unwrap();
            }
            assert_eq!(dates, want, "{label} instances skip or repeat a local date");
            println!("{label} {from}..{through}: {}", dates.join(" "));
            if label == "timed" {
                for v in &mine {
                    let day = v["occurrence_date"].as_str().unwrap();
                    assert_eq!(
                        v["local_start"],
                        format!("{day}T09:00"),
                        "local time drifted"
                    );
                }
                for w in mine.windows(2) {
                    let gap =
                        w[1]["start_utc"].as_i64().unwrap() - w[0]["start_utc"].as_i64().unwrap();
                    let expect = if w[1]["occurrence_date"] == change {
                        hours
                    } else {
                        24
                    };
                    assert_eq!(
                        gap,
                        expect * 3600,
                        "wrong gap before {}",
                        w[1]["occurrence_date"]
                    );
                }
                println!(
                    "timed: 09:00 local every day; {hours} h between the instances either side of {change}"
                );
            }
        }
    }
    drop(s);
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn one_record_is_every_view_and_offers_three_states() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let g = save(&mut s, json!({"kind":"goal","title":"Goal"}));
    let t = save(
        &mut s,
        json!({"kind":"task","title":"One task","goal_id":g["id"],"scheduled_for":"2026-11-03"}),
    );
    let mut hash = t["hash"].clone();
    for status in ["in_progress", "completed", "todo"] {
        let saved = save(
            &mut s,
            json!({"id":t["id"],"hash":hash,"status":status,"title":format!("One task {status}")}),
        );
        hash = saved["hash"].clone();
        let list = record(&s, &t["id"]);
        // Day, Week, Month and Year are ranges over the same projection.
        for (from, through) in [
            ("2026-11-03", "2026-11-03"),
            ("2026-11-01", "2026-11-07"),
            ("2026-11-01", "2026-11-30"),
            ("2026-01-01", "2026-12-31"),
        ] {
            let view = calendar(&mut s, from, through);
            let ev = view.iter().find(|v| v["id"] == t["id"]).unwrap();
            for k in ["hash", "title", "status", "goal_id"] {
                assert_eq!(
                    ev[k], list[k],
                    "{k} differs between the list and {from}..{through}"
                );
            }
        }
        assert_eq!(list["status"], status);
    }
    for other in ["blocked", "paused", "waiting", "done"] {
        assert!(
            s.command("save", &json!({"id":t["id"],"hash":hash,"status":other}))
                .is_err(),
            "{other} accepted as a task state"
        );
    }
    drop(s);
    fs::remove_dir_all(p).unwrap();
}
