use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use soliton_kernel::ledger::habits::{Bundle, CheckIn, Direction, HabitSpec};
use soliton_kernel::ledger::kitchen::{LotSource, LotSpec};
use soliton_kernel::ledger::Ledger;
use soliton_kernel::now::{items, NowList};
use soliton_kernel::{
    CivilDate, ClockTime, DataClass, Entity, GovernedRepository, NowKind, NowSettings, NowWhy,
    ObjectId, ObjectMeta, Priority, Snapshot, SourceKind, Task, TaskStatus,
};

const DAY: &str = "2026-10-28";

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("nowtest").unwrap().to_string());
    fs::create_dir(&path).unwrap();
    path
}

fn day() -> CivilDate {
    DAY.parse().unwrap()
}

fn at(text: &str) -> ClockTime {
    text.parse().unwrap()
}

fn task(title: &str, status: TaskStatus) -> Entity {
    Entity::Task(Task {
        meta: ObjectMeta {
            details: Default::default(),
            id: ObjectId::derive("task", title.as_bytes()).unwrap(),
            title: title.into(),
            data_class: DataClass::Internal,
            revision: 1,
        },
        category_id: None,
        aim_id: None,
        priority: Priority::Medium,
        status,
        scheduled_for: Some(day()),
        due_on: None,
        recurrence: None,
        completed_occurrences: BTreeSet::new(),
    })
}

fn habit(ledger: &mut Ledger, title: &str, bundle: Bundle) -> ObjectId {
    let spec = HabitSpec {
        title: title.into(),
        direction: Direction::Form,
        baseline: 1.0,
        bundle: Some(bundle),
    };
    ledger.define_habit(None, &spec, DAY).unwrap().id
}

/// Two tasks today (one done), a morning bundle of three habits (one checked),
/// an evening bundle of one, and one lot going off tomorrow beside one that keeps.
fn fixture() -> (Snapshot, Ledger) {
    let snapshot = Snapshot::from_entities([
        task("Send the invoice", TaskStatus::Todo),
        task("Book the dentist", TaskStatus::Completed),
    ])
    .unwrap();
    let mut ledger = Ledger::migrate(&root()).unwrap();
    let pray = habit(&mut ledger, "Pray", Bundle::Morning);
    habit(&mut ledger, "Drink water", Bundle::Morning);
    habit(&mut ledger, "Brush teeth", Bundle::Morning);
    habit(&mut ledger, "Wash face", Bundle::Evening);
    ledger.record_check_in(&pray, DAY, CheckIn::Done).unwrap();
    for (subject, expires) in [("Spinach", "2026-10-29"), ("Rice", "2027-03-01")] {
        ledger
            .add_stock(
                &LotSpec {
                    subject: subject.into(),
                    category: Some("food:produce".into()),
                    quantity: 1.0,
                    unit: "kg".into(),
                    expires_on: Some(expires.parse().unwrap()),
                    location: Some("fridge".into()),
                    source: LotSource::Typed,
                    purchase_line: None,
                },
                DAY,
            )
            .unwrap();
    }
    (snapshot, ledger)
}

fn shape(list: &NowList) -> Vec<(NowKind, &str, NowWhy)> {
    list.items
        .iter()
        .map(|item| (item.kind, item.title.as_str(), item.why))
        .collect()
}

/// Each source names a task in the snapshot, a habit or lot on the ledger, or
/// a bundle that holds the item's habit. Returns resolved and total.
fn resolve(list: &NowList, snapshot: &Snapshot, ledger: &Ledger) -> (usize, usize) {
    let mut resolved = 0;
    let mut total = 0;
    for item in &list.items {
        for source in &item.sources {
            total += 1;
            let found = match source.kind {
                SourceKind::Task => source
                    .id
                    .parse::<ObjectId>()
                    .ok()
                    .and_then(|id| snapshot.get(&id))
                    .is_some_and(|stored| stored.entity.meta().title == source.title),
                SourceKind::Habit => source
                    .id
                    .parse::<ObjectId>()
                    .ok()
                    .and_then(|id| ledger.habit(&id).ok())
                    .is_some_and(|habit| habit.spec.title == source.title),
                SourceKind::Bundle => Bundle::parse(&source.id).is_ok_and(|bundle| {
                    ledger
                        .bundle_order(bundle)
                        .unwrap()
                        .iter()
                        .any(|id| id.as_str() == item.id)
                }),
                SourceKind::Lot => source
                    .id
                    .parse::<ObjectId>()
                    .ok()
                    .and_then(|id| ledger.lot(&id).ok())
                    .is_some_and(|lot| lot.subject == source.title),
            };
            resolved += usize::from(found);
        }
    }
    (resolved, total)
}

#[test]
fn now_empty_snapshot_and_empty_ledger_give_an_empty_view() {
    let snapshot = Snapshot::default();
    let ledger = Ledger::migrate(&root()).unwrap();
    for ledger in [None, Some(&ledger)] {
        let list = items(
            &snapshot,
            ledger,
            day(),
            at("09:00"),
            &NowSettings::default(),
        )
        .unwrap();
        assert!(list.items.is_empty());
        assert_eq!(list.left_out, 0);
    }
}

#[test]
fn now_fixture_at_nine_gives_one_task_two_habits_one_lot_in_the_default_order() {
    let (snapshot, ledger) = fixture();
    let list = items(
        &snapshot,
        Some(&ledger),
        day(),
        at("09:00"),
        &NowSettings::default(),
    )
    .unwrap();
    assert_eq!(
        shape(&list),
        [
            (NowKind::Task, "Send the invoice", NowWhy::ScheduledNow),
            (NowKind::Habit, "Drink water", NowWhy::BundleOpen),
            (NowKind::Habit, "Brush teeth", NowWhy::BundleOpen),
            (NowKind::Lot, "Spinach", NowWhy::GoesOff),
        ]
    );
    assert_eq!(
        list.items.iter().map(|i| i.rank).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(list.left_out, 0);
}

#[test]
fn now_bound_two_returns_two_and_counts_two_left_out() {
    let (snapshot, ledger) = fixture();
    let settings = NowSettings {
        bound: 2,
        ..NowSettings::default()
    };
    let list = items(&snapshot, Some(&ledger), day(), at("09:00"), &settings).unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.left_out, 2);
    assert_eq!(list.items[1].title, "Drink water");
}

#[test]
fn now_habit_windows_follow_the_time_of_day() {
    let (snapshot, ledger) = fixture();
    let habits = |time: &str| {
        items(
            &snapshot,
            Some(&ledger),
            day(),
            at(time),
            &NowSettings::default(),
        )
        .unwrap()
        .items
        .into_iter()
        .filter(|item| item.kind == NowKind::Habit)
        .map(|item| item.title)
        .collect::<Vec<_>>()
    };
    assert_eq!(habits("09:00"), ["Drink water", "Brush teeth"]);
    assert_eq!(habits("19:00"), ["Wash face"]);
    assert!(habits("14:00").is_empty());
}

#[test]
fn now_order_is_a_setting_and_a_timed_task_comes_before_an_all_day_one() {
    let (_, ledger) = fixture();
    let mut timed = task("Call the bank", TaskStatus::InProgress);
    if let Entity::Task(task) = &mut timed {
        task.meta.details.schedule = Some(soliton_kernel::details::Schedule {
            time: "15:30".into(),
            duration_minutes: 30,
            timezone: "America/New_York".into(),
            start_utc: 0,
            fold: "earlier".into(),
        });
    }
    let snapshot =
        Snapshot::from_entities([task("Send the invoice", TaskStatus::Todo), timed]).unwrap();
    let settings = NowSettings {
        order: vec![NowKind::Lot, NowKind::Habit, NowKind::Task],
        ..NowSettings::default()
    };
    let list = items(&snapshot, Some(&ledger), day(), at("09:00"), &settings).unwrap();
    let kinds: Vec<_> = list
        .items
        .iter()
        .map(|i| (i.kind, i.title.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (NowKind::Lot, "Spinach"),
            (NowKind::Habit, "Drink water"),
            (NowKind::Habit, "Brush teeth"),
            (NowKind::Task, "Call the bank"),
            (NowKind::Task, "Send the invoice"),
        ]
    );
}

#[test]
fn now_hide_all_leaves_out_private_rows_and_marked_items() {
    let (snapshot, ledger) = fixture();
    let invoice = ObjectId::derive("task", b"Send the invoice").unwrap();
    ledger
        .mark_private(soliton_kernel::MarkScope::Item, invoice.as_str())
        .unwrap();
    ledger
        .mark_private(soliton_kernel::MarkScope::Module, "kitchen")
        .unwrap();
    let hidden = NowSettings {
        hide_all: true,
        ..NowSettings::default()
    };
    let list = items(&snapshot, Some(&ledger), day(), at("09:00"), &hidden).unwrap();
    assert!(list.items.iter().all(|i| i.kind == NowKind::Habit));
    let shown = items(
        &snapshot,
        Some(&ledger),
        day(),
        at("09:00"),
        &NowSettings::default(),
    )
    .unwrap();
    assert_eq!(shown.items.len(), 4);
}

#[test]
fn now_every_source_resolves_in_the_snapshot_or_the_ledger() {
    let (snapshot, ledger) = fixture();
    for time in ["09:00", "14:00", "19:00"] {
        let list = items(
            &snapshot,
            Some(&ledger),
            day(),
            at(time),
            &NowSettings::default(),
        )
        .unwrap();
        assert!(list.items.iter().all(|item| !item.sources.is_empty()));
        let (resolved, total) = resolve(&list, &snapshot, &ledger);
        assert!(total > 0);
        assert_eq!(resolved, total, "{time}");
    }
}

/// Runs the view once on a workspace copy named by SOLITON_NOW_WORKSPACE at
/// SOLITON_NOW_DAY and SOLITON_NOW_TIME, and prints it as JSON.
#[test]
#[ignore = "runs on a workspace copy named in the environment"]
fn now_real_copy_run() {
    let workspace = PathBuf::from(std::env::var("SOLITON_NOW_WORKSPACE").unwrap());
    let day: CivilDate = std::env::var("SOLITON_NOW_DAY").unwrap().parse().unwrap();
    let time = at(&std::env::var("SOLITON_NOW_TIME").unwrap());
    let snapshot = GovernedRepository::open(&workspace)
        .unwrap()
        .rebuild()
        .unwrap();
    let ledger = Ledger::open_existing(&workspace).unwrap().unwrap();
    let list = items(&snapshot, Some(&ledger), day, time, &NowSettings::default()).unwrap();
    let (resolved, sources) = resolve(&list, &snapshot, &ledger);
    let resolved_items = list
        .items
        .iter()
        .filter(|item| {
            let one = NowList {
                items: vec![(*item).clone()],
                left_out: 0,
            };
            let (r, t) = resolve(&one, &snapshot, &ledger);
            r == t
        })
        .count();
    let report = serde_json::json!({
        "day": day.to_string(),
        "time": time.to_string(),
        "planner_objects": snapshot.len(),
        "count": list.items.len(),
        "left_out": list.left_out,
        "kinds": list.items.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(),
        "reasons": list.items.iter().map(|i| i.why.as_str()).collect::<Vec<_>>(),
        "items_resolved": resolved_items,
        "sources_resolved": resolved,
        "sources": sources,
        "items": list.items.iter().map(soliton_kernel::NowItem::to_json).collect::<Vec<_>>(),
    });
    println!("NOW_JSON {}", serde_json::to_string(&report).unwrap());
    assert_eq!(resolved_items, list.items.len());
}
