use serde_json::{json, Value};
use soliton_kernel::ledger::habits::{Bundle, CheckIn, Direction, HabitSpec};
use soliton_kernel::ledger::{KindDef, Ledger, MarkScope, NewObservation, Question, LEDGER_FILE};
use soliton_kernel::{DataClass, KernelError, ObjectId};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("ledgertest").unwrap().to_string());
    fs::create_dir(&path).unwrap();
    path
}

#[allow(clippy::too_many_arguments)]
fn row(
    kind: &str,
    day: &str,
    item: Option<&str>,
    category: Option<&str>,
    subject: &str,
    quantity: f64,
    pence: Option<i64>,
    writer: &str,
) -> NewObservation {
    NewObservation {
        kind: kind.into(),
        observed_on: day.into(),
        item_id: item.map(Into::into),
        category: category.map(Into::into),
        subject: subject.into(),
        quantity: Some(quantity),
        amount_minor: pence,
        currency: pence.map(|_| "GBP".into()),
        writer: writer.into(),
        ..NewObservation::default()
    }
}

/// The design's example rows (example.sql), written through the kernel.
fn write_example(ledger: &Ledger) -> ObjectId {
    let cat = "category_38a036a22d051d143d3cdebe906f94f0";
    for (day, kept) in [
        ("2026-09-29", 1.0),
        ("2026-09-30", 1.0),
        ("2026-10-06", 0.0),
    ] {
        ledger
            .append_observation(&row(
                "check_in",
                day,
                None,
                Some(cat),
                "bundle:morning",
                kept,
                None,
                "habits",
            ))
            .unwrap();
    }
    for (day, task) in [
        ("2026-09-30", "task_001fcc426d45bcbe7ccdc124cc24180c"),
        ("2026-10-01", "task_013cec88e2a3db606b0b81a92bf4b4c8"),
        ("2026-10-07", "task_0293d24ee361fe2f5c015d9e4e9302fe"),
    ] {
        ledger
            .append_observation(&row(
                "check_in",
                day,
                Some(task),
                None,
                "task.completed",
                1.0,
                None,
                "planner",
            ))
            .unwrap();
    }
    for (day, category, subject) in [
        ("2026-09-29", "food:home", "lentil batch, portion 1"),
        ("2026-09-30", "food:home", "roast vegetables"),
        ("2026-10-06", "food:out", "takeaway noodles"),
    ] {
        ledger
            .append_observation(&row(
                "meal",
                day,
                None,
                Some(category),
                subject,
                1.0,
                None,
                "kitchen",
            ))
            .unwrap();
    }
    ledger
        .append_observation(&row(
            "purchase_line",
            "2026-09-28",
            None,
            Some("food:groceries"),
            "red lentils 2kg",
            1.0,
            Some(420),
            "finance",
        ))
        .unwrap();
    ledger
        .append_observation(&row(
            "purchase_line",
            "2026-09-28",
            None,
            Some("food:groceries"),
            "vegetables box",
            1.0,
            Some(1250),
            "finance",
        ))
        .unwrap();
    let mut takeaway = row(
        "purchase_line",
        "2026-10-06",
        None,
        Some("food:takeaway"),
        "noodles",
        1.0,
        Some(1180),
        "finance",
    );
    takeaway.private = true;
    let marked = ledger.append_observation(&takeaway).unwrap();
    for (day, hours) in [
        ("2026-09-29", 7.5),
        ("2026-09-30", 7.0),
        ("2026-10-06", 5.5),
    ] {
        ledger
            .append_observation(&row(
                "measurement",
                day,
                None,
                Some("health:sleep"),
                "sleep_hours",
                hours,
                None,
                "health",
            ))
            .unwrap();
    }
    marked
}

fn answer(ledger: &Ledger, question: Question, hide_all: bool) -> Vec<Vec<Value>> {
    ledger.ask(question, hide_all).unwrap().rows
}

#[test]
fn migrate_creates_private_versioned_ledger_with_nine_kinds_and_refuses_twice() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    let mode = fs::metadata(root.join(LEDGER_FILE))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let kinds: Vec<String> = ledger
        .kinds()
        .unwrap()
        .into_iter()
        .map(|k| k.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "batch_plan",
            "check_in",
            "cook_profile",
            "equipment",
            "inventory",
            "meal",
            "measure_definition",
            "measurement",
            "purchase_line"
        ]
    );
    drop(ledger);
    assert!(matches!(
        Ledger::migrate(&root),
        Err(KernelError::Conflict(_))
    ));
    assert!(Ledger::open_existing(&root).unwrap().is_some());
    let empty = self::root();
    assert!(Ledger::open_existing(&empty).unwrap().is_none());
    assert!(
        !empty.join(LEDGER_FILE).exists(),
        "opening never creates the ledger"
    );
}

#[test]
fn ledger_supersede_keeps_old_row_and_answers_from_the_correction() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let first = ledger
        .append_observation(&row(
            "purchase_line",
            "2026-09-28",
            None,
            Some("food:groceries"),
            "lentils",
            1.0,
            Some(420),
            "finance",
        ))
        .unwrap();
    let fixed = row(
        "purchase_line",
        "2026-09-28",
        None,
        Some("food:groceries"),
        "lentils",
        1.0,
        Some(400),
        "finance",
    );
    let second = ledger.supersede(&first, &fixed).unwrap();
    assert!(matches!(
        ledger.supersede(&first, &fixed),
        Err(KernelError::Conflict(_))
    ));
    let rows = answer(&ledger, Question::FoodSpendWeeks, false);
    assert_eq!(
        rows,
        vec![vec![
            json!("2026-09-28"),
            json!("food:groceries"),
            json!("4.00"),
            json!(0),
            json!(0)
        ]]
    );
    assert_ne!(first, second);
    assert!(first.as_str().starts_with("obs_") && first.as_str().len() == 36);
}

#[test]
fn ledger_data_class_defaults_to_kind_and_cannot_be_lowered() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    let mut meal = row(
        "meal",
        "2026-09-29",
        None,
        Some("food:home"),
        "soup",
        1.0,
        None,
        "kitchen",
    );
    meal.data_class = Some(DataClass::Internal);
    assert!(matches!(
        ledger.append_observation(&meal),
        Err(KernelError::Ledger(_))
    ));
    meal.data_class = Some(DataClass::Restricted);
    ledger.append_observation(&meal).unwrap();
    let mut bad_day = meal.clone();
    bad_day.observed_on = "2026-02-30".into();
    assert!(ledger.append_observation(&bad_day).is_err());
    let mut unknown = meal;
    unknown.kind = "no_such_kind".into();
    assert!(ledger.append_observation(&unknown).is_err());
}

#[test]
fn ledger_three_questions_answer_on_rows_written_through_the_kernel() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    write_example(&ledger);
    assert_eq!(
        answer(&ledger, Question::MorningBundleWeeks, false),
        vec![
            vec![
                json!("2026-09-28"),
                json!("kept"),
                json!(2),
                json!("16.70"),
                json!(2),
                json!(2)
            ],
            vec![
                json!("2026-10-05"),
                json!("not kept"),
                json!(1),
                json!("11.80"),
                json!(0),
                json!(1)
            ],
        ]
    );
    assert_eq!(
        answer(&ledger, Question::SleepBaselineDays, false),
        vec![
            vec![
                json!("2026-09-29"),
                json!(7.5),
                json!("above"),
                json!(1.0),
                json!(1)
            ],
            vec![
                json!("2026-09-30"),
                json!(7.0),
                json!("above"),
                json!(1.0),
                json!(1)
            ],
            vec![
                json!("2026-10-06"),
                json!(5.5),
                json!("below"),
                json!(0.0),
                json!(0)
            ],
        ]
    );
    assert_eq!(
        answer(&ledger, Question::FoodSpendWeeks, false),
        vec![
            vec![
                json!("2026-09-28"),
                json!("food:groceries"),
                json!("16.70"),
                json!(2),
                json!(0)
            ],
            vec![
                json!("2026-10-05"),
                json!("food:takeaway"),
                json!("11.80"),
                json!(0),
                json!(1)
            ],
        ]
    );
    for question in Question::ALL {
        assert_eq!(Question::parse(question.name()).unwrap(), question);
    }
}

#[test]
fn ledger_hide_all_hides_private_rows_and_marked_goals_items_and_modules() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    write_example(&ledger);
    let on = answer(&ledger, Question::MorningBundleWeeks, true);
    assert_eq!(
        on[1],
        vec![
            json!("2026-10-05"),
            json!("not kept"),
            json!(1),
            json!("0.00"),
            json!(0),
            json!(1)
        ]
    );
    let goal: ObjectId = "aim_00000000000000000000000000000001".parse().unwrap();
    let task: ObjectId = "task_0293d24ee361fe2f5c015d9e4e9302fe".parse().unwrap();
    ledger
        .mark_goal_private(&goal, std::slice::from_ref(&task))
        .unwrap();
    assert_eq!(
        answer(&ledger, Question::MorningBundleWeeks, true)[1][2],
        json!(0)
    );
    assert_eq!(
        answer(&ledger, Question::MorningBundleWeeks, false)[1][2],
        json!(1)
    );
    ledger.mark_private(MarkScope::Module, "finance").unwrap();
    assert_eq!(
        answer(&ledger, Question::FoodSpendWeeks, true),
        Vec::<Vec<Value>>::new()
    );
    ledger.unmark_private(MarkScope::Module, "finance").unwrap();
    assert_eq!(answer(&ledger, Question::FoodSpendWeeks, true).len(), 1);
}

#[test]
fn ledger_new_kind_registered_in_the_test_alone_touches_no_other_module() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    write_example(&ledger);
    let before: Vec<_> = Question::ALL
        .iter()
        .map(|q| answer(&ledger, *q, false))
        .collect();
    ledger
        .register_kind(&KindDef {
            kind: "test_mood".into(),
            writer: "test_journal".into(),
            default_class: DataClass::Restricted,
            unit: Some("score".into()),
            description: "A test-only kind".into(),
        })
        .unwrap();
    let mut mood = row(
        "test_mood",
        "2026-09-29",
        None,
        Some("mind:mood"),
        "mood",
        4.0,
        None,
        "test_journal",
    );
    mood.extra = Some(json!({"note_words": 3}));
    ledger.append_observation(&mood).unwrap();
    let mut lowered = mood.clone();
    lowered.data_class = Some(DataClass::PersonalSensitive);
    assert!(ledger.append_observation(&lowered).is_err());
    let after: Vec<_> = Question::ALL
        .iter()
        .map(|q| answer(&ledger, *q, false))
        .collect();
    assert_eq!(before, after, "the other modules' answers do not change");
    assert_eq!(ledger.kinds().unwrap().len(), 10);
}

fn spec(title: &str, direction: Direction, baseline: f64, bundle: Option<Bundle>) -> HabitSpec {
    HabitSpec {
        title: title.into(),
        direction,
        baseline,
        bundle,
    }
}

#[test]
fn habit_define_edit_and_bundle_order_supersede_and_keep_order() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let day = "2026-09-28";
    let water = ledger
        .define_habit(
            None,
            &spec("Water", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    let stretch = ledger
        .define_habit(
            None,
            &spec("Stretch", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    let read = ledger
        .define_habit(
            None,
            &spec("Read", Direction::Form, 1.0, Some(Bundle::Evening)),
            day,
        )
        .unwrap();
    assert!(water.id.as_str().starts_with("habit_"));
    assert_eq!(
        ledger.bundle_order(Bundle::Morning).unwrap(),
        vec![water.id.clone(), stretch.id.clone()]
    );
    ledger
        .set_bundle_order(
            Bundle::Morning,
            &[stretch.id.clone(), water.id.clone()],
            day,
        )
        .unwrap();
    assert!(ledger
        .set_bundle_order(Bundle::Morning, std::slice::from_ref(&water.id), day)
        .is_err());
    let titles: Vec<String> = ledger
        .habits(false)
        .unwrap()
        .into_iter()
        .map(|h| h.spec.title)
        .collect();
    assert_eq!(titles, ["Stretch", "Water", "Read"]);
    // Moving a habit from the evening to the morning bundle edits both orders.
    let moved = ledger
        .define_habit(
            Some(&read.id),
            &spec("Read 10 pages", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    assert_eq!(moved.id, read.id);
    assert_ne!(moved.row, read.row);
    assert_eq!(
        ledger.bundle_order(Bundle::Evening).unwrap(),
        Vec::<ObjectId>::new()
    );
    assert_eq!(
        ledger.bundle_order(Bundle::Morning).unwrap().last(),
        Some(&read.id)
    );
    assert_eq!(ledger.habits(false).unwrap().len(), 3);
    assert!(ledger
        .define_habit(None, &spec("", Direction::Form, 1.0, None), day)
        .is_err());
    assert!(ledger
        .define_habit(None, &spec("Bad", Direction::Form, 3.0, None), day)
        .is_err());
    assert_eq!(
        ledger.kinds().unwrap().len(),
        10,
        "the habit kind is registered once"
    );
}

#[test]
fn habit_check_in_refuses_a_second_and_corrects_by_supersede() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let day = "2026-09-28";
    let walk = ledger
        .define_habit(
            None,
            &spec("Walk", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    let snack = ledger
        .define_habit(None, &spec("Late snack", Direction::Break, 2.0, None), day)
        .unwrap();
    let first = ledger
        .record_check_in(&walk.id, day, CheckIn::Missed)
        .unwrap();
    assert!(matches!(
        ledger.record_check_in(&walk.id, day, CheckIn::Done),
        Err(KernelError::Conflict(_))
    ));
    assert!(ledger
        .record_check_in(&walk.id, "2026-09-29", CheckIn::Count(2.0))
        .is_err());
    assert!(ledger
        .record_check_in(&snack.id, day, CheckIn::Done)
        .is_err());
    let fixed = ledger.correct_check_in(&first, CheckIn::Done).unwrap();
    assert!(ledger.correct_check_in(&first, CheckIn::Done).is_err());
    ledger
        .record_check_in(&snack.id, day, CheckIn::Count(3.0))
        .unwrap();
    let from = "2026-09-28".parse().unwrap();
    let map = ledger.heat_map(from, from, false).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(
        (map[0].cells[0].value, map[0].cells[0].against),
        (Some(1.0), "at")
    );
    assert_eq!(
        (map[1].cells[0].value, map[1].cells[0].against),
        (Some(3.0), "above")
    );
    assert_ne!(first, fixed);
}

#[test]
fn habit_morning_bundle_check_ins_answer_question_one_and_the_heat_map() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let day = "2026-09-28";
    let a = ledger
        .define_habit(
            None,
            &spec("A", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    let b = ledger
        .define_habit(
            None,
            &spec("B", Direction::Form, 1.0, Some(Bundle::Morning)),
            day,
        )
        .unwrap();
    let smoke = ledger
        .define_habit(None, &spec("Smoke", Direction::Break, 1.0, None), day)
        .unwrap();
    // Week of 28 Sep: 3 of 4 days kept -> kept. Week of 5 Oct: 0 of 2 days kept -> not kept.
    for (d, ka, kb) in [
        ("2026-09-28", true, true),
        ("2026-09-29", true, true),
        ("2026-09-30", true, false),
        ("2026-10-01", true, true),
        ("2026-10-05", false, true),
        ("2026-10-06", true, false),
    ] {
        for (h, k) in [(&a, ka), (&b, kb)] {
            ledger
                .record_check_in(&h.id, d, if k { CheckIn::Done } else { CheckIn::Missed })
                .unwrap();
        }
        ledger
            .record_check_in(&smoke.id, d, CheckIn::Count(if ka { 0.0 } else { 2.0 }))
            .unwrap();
    }
    let q1 = answer(&ledger, Question::MorningBundleWeeks, false);
    assert_eq!(
        q1,
        vec![
            vec![
                json!("2026-09-28"),
                json!("kept"),
                json!(0),
                json!("0.00"),
                json!(3),
                json!(4)
            ],
            vec![
                json!("2026-10-05"),
                json!("not kept"),
                json!(0),
                json!("0.00"),
                json!(0),
                json!(2)
            ],
        ]
    );
    let from = "2026-09-28".parse().unwrap();
    let through = "2026-10-04".parse().unwrap();
    let map = ledger.heat_map(from, through, false).unwrap();
    assert_eq!(map.len(), 3);
    assert!(map.iter().all(|row| row.cells.len() == 7));
    let against: Vec<&str> = map[2].cells.iter().map(|c| c.against).collect();
    assert_eq!(
        against,
        [
            "below",
            "below",
            "below",
            "below",
            "no record",
            "no record",
            "no record"
        ]
    );
    ledger
        .mark_private(MarkScope::Item, smoke.id.as_str())
        .unwrap();
    assert_eq!(ledger.heat_map(from, through, true).unwrap().len(), 2);
    assert_eq!(ledger.heat_map(from, through, false).unwrap().len(), 3);
    assert!(ledger.heat_map(through, from, false).is_err());
}
