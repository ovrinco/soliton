use rusqlite::Connection;
use serde_json::{json, Value};
use soliton_kernel::ledger::finance::{DateFormat, StatementMapping};
use soliton_kernel::ledger::kitchen::{DishSpec, LotSource, LotSpec};
use soliton_kernel::ledger::{Ledger, MarkScope, Question, LEDGER_FILE, SCHEMA_VERSION};
use soliton_kernel::{CivilDate, KernelError, ObjectId};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("kitchentest").unwrap().to_string());
    fs::create_dir(&path).unwrap();
    path
}

fn day(text: &str) -> CivilDate {
    text.parse().unwrap()
}

fn lot(subject: &str, quantity: f64, unit: &str, expires_on: Option<&str>) -> LotSpec {
    LotSpec {
        subject: subject.into(),
        category: Some("food:produce".into()),
        quantity,
        unit: unit.into(),
        expires_on: expires_on.map(day),
        location: Some("fridge".into()),
        source: LotSource::Typed,
        purchase_line: None,
    }
}

fn mapping() -> StatementMapping {
    StatementMapping {
        bank: "sample-bank".into(),
        date: "Date".into(),
        date_format: DateFormat::DayMonthYear,
        description: "Description".into(),
        amount: "Amount".into(),
        currency: "Currency".into(),
        category: Some("Type".into()),
        debit_negative: true,
        categories: BTreeMap::from([
            ("Groceries".into(), "food:groceries".into()),
            ("Takeaway".into(), "food:eating_out".into()),
        ]),
    }
}

const STATEMENT: &str = "Date,Description,Amount,Currency,Type\r\n\
29/09/2026,\"Market, stall 4\",-4.80,EUR,Groceries\r\n\
30/09/2026,Salary,2500.00,EUR,Income\r\n\
02/10/2026,Grocer,-3.50,EUR,Groceries\r\n\
03/10/2026,Noodle bar,-14.50,EUR,Takeaway\r\n\
03/10/2026,Hardware shop,-9.99,EUR,Home\r\n";

#[test]
fn version_one_ledger_migrates_in_place_to_version_two() {
    let root = root();
    let path = root.join(LEDGER_FILE);
    let v1 = Connection::open(&path).unwrap();
    v1.execute_batch(include_str!("../src/ledger.sql")).unwrap();
    drop(v1);
    let mut ledger = Ledger::open_existing(&root).unwrap().unwrap();
    assert_eq!(ledger.version(), 1);
    assert_eq!(ledger.kinds().unwrap().len(), 4);
    assert_eq!(ledger.upgrade().unwrap(), 1);
    assert_eq!(ledger.version(), SCHEMA_VERSION);
    assert_eq!(ledger.kinds().unwrap().len(), 9);
    assert!(matches!(ledger.upgrade(), Err(KernelError::Conflict(_))));
    drop(ledger);
    let check = Connection::open(&path).unwrap();
    let version: i64 = check
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let marks: i64 = check
        .query_row("SELECT count(*) FROM private_mark WHERE scope='module' AND key IN ('finance','health')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marks, 2);
}

#[test]
fn stock_add_use_adjust_and_expiry_supersede_the_lot() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    let used = ledger.use_stock(&spinach, 100.0, "2026-10-05").unwrap();
    assert!(
        ledger.lot(&spinach).is_err(),
        "the old lot row is no longer current"
    );
    assert_eq!(ledger.lot(&used).unwrap().quantity, 300.0);
    assert!(ledger.use_stock(&used, 301.0, "2026-10-05").is_err());
    let counted = ledger.adjust_stock(&used, 250.0, "2026-10-05").unwrap();
    let later = ledger
        .set_expiry(&counted, day("2026-10-07"), "2026-10-05")
        .unwrap();
    let now = ledger.lot(&later).unwrap();
    assert_eq!(
        (
            now.quantity,
            now.expires_on.as_deref(),
            now.location.as_deref()
        ),
        (250.0, Some("2026-10-07"), Some("fridge"))
    );
    assert_eq!(ledger.stock(false).unwrap().len(), 1);
    assert!(ledger
        .add_stock(&lot("", 1.0, "g", None), "2026-10-05")
        .is_err());
    let mut receipt = lot("beetroot", 1.0, "kg", None);
    receipt.source = LotSource::Receipt;
    assert!(
        ledger.add_stock(&receipt, "2026-10-05").is_err(),
        "a receipt lot names its purchase line"
    );
}

#[test]
fn equipment_and_cook_profile_are_one_current_row_per_name() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger
        .set_equipment("sheet pan", 2.0, Some("half sheet"), "2026-10-04")
        .unwrap();
    ledger
        .set_equipment("sheet pan", 3.0, Some("half sheet"), "2026-10-05")
        .unwrap();
    ledger
        .set_equipment("dutch oven", 1.0, Some("6.5 l"), "2026-10-05")
        .unwrap();
    let pieces = ledger.equipment(false).unwrap();
    assert_eq!(
        pieces
            .iter()
            .map(|e| (e.name.as_str(), e.count))
            .collect::<Vec<_>>(),
        [("dutch oven", 1.0), ("sheet pan", 3.0)]
    );
    ledger
        .set_cook_profile("diet", None, None, Some("plant-based"), "2026-10-04")
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(12.0),
            Some("portion"),
            None,
            "2026-10-04",
        )
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(14.0),
            Some("portion"),
            None,
            "2026-10-05",
        )
        .unwrap();
    let profile = ledger.cook_profile(false).unwrap();
    assert_eq!(profile.len(), 2);
    assert_eq!(profile[1].quantity, Some(14.0));
    assert!(ledger
        .set_cook_profile("diet", None, None, None, "2026-10-05")
        .is_err());
}

#[test]
fn batch_plan_checks_stock_and_equipment_and_reads_q5() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    let ginger = ledger
        .add_stock(&lot("ginger", 150.0, "g", Some("2026-10-07")), "2026-10-04")
        .unwrap();
    ledger
        .add_stock(
            &lot("chickpeas dried", 2000.0, "g", Some("2027-06-01")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .set_equipment("dutch oven", 1.0, None, "2026-10-04")
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(14.0),
            Some("portion"),
            None,
            "2026-10-04",
        )
        .unwrap();
    let stew = |phase| DishSpec {
        name: "spinach stew".into(),
        portions: 6.0,
        phase,
        equipment: vec!["dutch oven".into()],
        uses: vec![(spinach.clone(), 300.0)],
    };
    // Two dishes in one dutch oven in the same phase, an unknown piece, too much spinach: refused.
    assert!(ledger
        .write_batch_plan(day("2026-10-05"), 3, &[stew(1), stew(1)])
        .is_err());
    let mut wok = stew(1);
    wok.equipment = vec!["wok".into()];
    assert!(ledger
        .write_batch_plan(day("2026-10-05"), 3, &[wok])
        .is_err());
    let mut greedy = stew(1);
    greedy.uses = vec![(spinach.clone(), 401.0)];
    assert!(ledger
        .write_batch_plan(day("2026-10-05"), 3, &[greedy])
        .is_err());
    assert!(ledger.write_batch_plan(day("2026-10-05"), 3, &[]).is_err());

    let plan = ledger
        .write_batch_plan(day("2026-10-05"), 3, &[stew(1)])
        .unwrap();
    assert_eq!(plan.plan_id, "plan_2026-10-05");
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(plan.constraints.len(), 1);
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("6 portions planned against 14 a week")));
    assert!(
        plan.notes.iter().any(|n| n.starts_with("ginger 150")),
        "{:?}",
        plan.notes
    );
    assert_eq!(
        plan.goes_off.columns,
        [
            "subject",
            "on_hand",
            "unit",
            "expires_on",
            "planned",
            "unplanned"
        ]
    );
    assert_eq!(
        plan.goes_off.rows,
        [
            vec![
                json!("spinach"),
                json!(400.0),
                json!("g"),
                json!("2026-10-06"),
                json!(300.0),
                json!(100.0)
            ],
            vec![
                json!("ginger"),
                json!(150.0),
                json!("g"),
                json!("2026-10-07"),
                json!(0),
                json!(150.0)
            ],
        ]
    );
    assert!(matches!(
        ledger.write_batch_plan(day("2026-10-05"), 3, &[stew(2)]),
        Err(KernelError::Conflict(_))
    ));
    // A second plan cannot draw the spinach the first already holds.
    let mut more = stew(1);
    more.uses = vec![(spinach.clone(), 200.0)];
    assert!(ledger
        .write_batch_plan(day("2026-10-06"), 3, &[more])
        .is_err());
    let _ = ginger;
}

#[test]
fn q5_goes_off_within_n_days_leaves_out_later_and_empty_lots() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let soon = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .add_stock(
            &lot("chickpeas dried", 2000.0, "g", Some("2027-06-01")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .add_stock(&lot("salt", 500.0, "g", None), "2026-10-04")
        .unwrap();
    let table = ledger.goes_off_within(day("2026-10-05"), 3, false).unwrap();
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0][0], json!("spinach"));
    assert!(ledger
        .goes_off_within(day("2026-10-05"), 0, false)
        .unwrap()
        .rows
        .is_empty());
    ledger.use_stock(&soon, 400.0, "2026-10-05").unwrap();
    assert!(ledger
        .goes_off_within(day("2026-10-05"), 3, false)
        .unwrap()
        .rows
        .is_empty());
    assert!(ledger
        .goes_off_within(day("2026-10-05"), 400, false)
        .is_err());
}

#[test]
fn statement_import_maps_once_refuses_duplicates_and_answers_q6() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    assert!(
        ledger.import_statement("sample-bank", STATEMENT).is_err(),
        "no mapping yet"
    );
    ledger.set_statement_mapping(&mapping()).unwrap();
    assert_eq!(
        ledger.statement_mapping("sample-bank").unwrap(),
        Some(mapping())
    );
    let first = ledger.import_statement("sample-bank", STATEMENT).unwrap();
    assert_eq!(
        (first.added.len(), first.duplicates, first.credits),
        (4, 0, 1)
    );
    assert_eq!(first.period, "2026-09-29..2026-10-03");
    let again = ledger.import_statement("sample-bank", STATEMENT).unwrap();
    assert_eq!((again.added.len(), again.duplicates), (0, 4));
    let q6 = ledger.ask(Question::SpendByCategoryWeeks, false).unwrap();
    assert_eq!(
        q6.columns,
        ["week", "category", "spent", "currency", "lines"]
    );
    let rows: Vec<Vec<Value>> = q6.rows;
    // One week (Monday 2026-09-28): groceries 8.30 over 2 lines, eating out 14.50, uncategorised 9.99.
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        vec![
            json!("2026-09-28"),
            json!("food:eating_out"),
            json!("14.50"),
            json!("EUR"),
            json!(1)
        ]
    );
    assert_eq!(
        rows[1],
        vec![
            json!("2026-09-28"),
            json!("food:groceries"),
            json!("8.30"),
            json!("EUR"),
            json!(2)
        ]
    );
    assert_eq!(rows[2][1], json!("uncategorised"));
}

#[test]
fn statement_import_refuses_a_line_without_a_currency_and_writes_nothing() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.set_statement_mapping(&mapping()).unwrap();
    let bad = "Date,Description,Amount,Currency,Type\n01/10/2026,Grocer,-3.50,EUR,Groceries\n02/10/2026,Grocer,-1.00,,Groceries\n";
    assert!(ledger.import_statement("sample-bank", bad).is_err());
    assert!(ledger
        .ask(Question::SpendByCategoryWeeks, false)
        .unwrap()
        .rows
        .is_empty());
    let mut missing = mapping();
    missing.currency = " ".into();
    assert!(ledger.set_statement_mapping(&missing).is_err());
}

#[test]
fn hide_all_hides_finance_from_the_first_line_and_private_kitchen_rows() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.set_statement_mapping(&mapping()).unwrap();
    ledger.import_statement("sample-bank", STATEMENT).unwrap();
    assert_eq!(
        ledger
            .ask(Question::SpendByCategoryWeeks, false)
            .unwrap()
            .rows
            .len(),
        3
    );
    assert!(
        ledger
            .ask(Question::SpendByCategoryWeeks, true)
            .unwrap()
            .rows
            .is_empty(),
        "finance is private from the first capture"
    );
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .add_stock(&lot("ginger", 150.0, "g", Some("2026-10-07")), "2026-10-04")
        .unwrap();
    ledger
        .set_cook_profile("diet", None, None, Some("plant-based"), "2026-10-04")
        .unwrap();
    assert_eq!(
        ledger
            .goes_off_within(day("2026-10-05"), 3, true)
            .unwrap()
            .rows
            .len(),
        2
    );
    ledger
        .mark_private(MarkScope::Category, "food:produce")
        .unwrap();
    assert!(ledger
        .goes_off_within(day("2026-10-05"), 3, true)
        .unwrap()
        .rows
        .is_empty());
    assert!(ledger.stock(true).unwrap().is_empty());
    assert_eq!(ledger.stock(false).unwrap().len(), 2);
    ledger.mark_private(MarkScope::Module, "kitchen").unwrap();
    assert!(ledger.cook_profile(true).unwrap().is_empty());
    assert!(ledger.equipment(true).unwrap().is_empty());
    let _ = spinach;
}

#[test]
fn cooking_a_dish_writes_a_meal_takes_its_uses_and_q3_counts_it() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let rice = ledger
        .add_stock(&lot("rice", 1000.0, "g", None), "2031-03-03")
        .unwrap();
    let leek = ledger
        .add_stock(&lot("leek", 4.0, "item", Some("2031-03-05")), "2031-03-03")
        .unwrap();
    let dishes = [DishSpec {
        name: "leek risotto".into(),
        portions: 4.0,
        phase: 1,
        equipment: vec![],
        uses: vec![(leek.clone(), 2.0), (rice.clone(), 300.0)],
    }];
    ledger
        .write_batch_plan(day("2031-03-03"), 3, &dishes)
        .unwrap();
    // The lot is corrected after planning; cooking draws on the row now current.
    let leek = ledger
        .set_expiry(&leek, day("2031-03-06"), "2031-03-03")
        .unwrap();
    let cooked = ledger
        .cook_dish("plan_2031-03-03", "leek risotto", day("2031-03-04"))
        .unwrap();
    assert_eq!(cooked.lots.len(), 2);
    let stock: BTreeMap<String, f64> = ledger
        .stock(false)
        .unwrap()
        .into_iter()
        .map(|l| (l.subject, l.quantity))
        .collect();
    assert_eq!(
        stock,
        BTreeMap::from([("leek".into(), 2.0), ("rice".into(), 700.0)])
    );
    assert!(ledger.lot(&leek).is_err(), "the lot row used is superseded");
    let meals = ledger.meals(None, false).unwrap();
    assert_eq!(meals.len(), 1);
    assert_eq!(
        (
            meals[0].id.clone(),
            meals[0].name.as_str(),
            meals[0].portions,
            meals[0].category.as_deref(),
            meals[0].plan_id.as_deref()
        ),
        (
            cooked.meal.clone(),
            "leek risotto",
            4.0,
            Some("food:home"),
            Some("plan_2031-03-03")
        )
    );
    assert_eq!(
        ledger.meals(Some(day("2031-03-05")), false).unwrap(),
        vec![]
    );
    // The cooked dish no longer counts as planned in Q5.
    let q5 = ledger.goes_off_within(day("2031-03-04"), 3, false).unwrap();
    assert_eq!(
        q5.rows[0],
        vec![
            json!("leek"),
            json!(2.0),
            json!("item"),
            json!("2031-03-06"),
            json!(0),
            json!(2.0)
        ]
    );
    let plan = &ledger.batch_plans(None, false).unwrap()[0];
    assert_eq!(
        (plan.dishes[0].id.clone(), plan.dishes[0].cooked.clone()),
        (cooked.dish.clone(), Some(cooked.meal.clone()))
    );
    assert!(matches!(
        ledger.cook_dish("plan_2031-03-03", "leek risotto", day("2031-03-04")),
        Err(KernelError::Conflict(_))
    ));
    // Q3 counts the meal beside the week's food spending.
    let csv = "Date,Description,Amount,Currency,Type\n04/03/2031,Grocer,-3.50,EUR,Groceries\n";
    ledger.set_statement_mapping(&mapping()).unwrap();
    ledger.import_statement("sample-bank", csv).unwrap();
    let q3 = ledger.ask(Question::FoodSpendWeeks, false).unwrap();
    assert_eq!(
        q3.rows,
        vec![vec![
            json!("2031-03-03"),
            json!("food:groceries"),
            json!("3.50"),
            json!(1),
            json!(0)
        ]]
    );
    ledger.mark_private(MarkScope::Module, "kitchen").unwrap();
    assert_eq!(ledger.meals(None, true).unwrap(), vec![]);
    assert_eq!(ledger.meals(None, false).unwrap().len(), 1);
}

#[test]
fn cooking_refuses_a_dish_short_of_stock_and_writes_nothing() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let rice = ledger
        .add_stock(&lot("rice", 500.0, "g", None), "2031-03-03")
        .unwrap();
    let dishes = [DishSpec {
        name: "rice".into(),
        portions: 2.0,
        phase: 1,
        equipment: vec![],
        uses: vec![(rice.clone(), 400.0)],
    }];
    ledger
        .write_batch_plan(day("2031-03-03"), 3, &dishes)
        .unwrap();
    let rice = ledger.use_stock(&rice, 200.0, "2031-03-03").unwrap();
    assert!(ledger
        .cook_dish("plan_2031-03-03", "rice", day("2031-03-04"))
        .is_err());
    assert!(ledger
        .cook_dish("plan_2031-03-03", "no such dish", day("2031-03-04"))
        .is_err());
    assert_eq!(ledger.lot(&rice).unwrap().quantity, 300.0);
    assert_eq!(ledger.meals(None, false).unwrap(), vec![]);
}

#[test]
fn a_profile_entry_is_removed_by_supersede_and_set_again() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger
        .set_cook_profile("diet", None, None, Some("vegetarian"), "2031-03-03")
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(10.0),
            Some("portion"),
            None,
            "2031-03-03",
        )
        .unwrap();
    let gone = ledger.remove_cook_profile("diet", "2031-03-04").unwrap();
    let subjects: Vec<String> = ledger
        .cook_profile(false)
        .unwrap()
        .into_iter()
        .map(|e| e.subject)
        .collect();
    assert_eq!(subjects, vec!["portions_per_week".to_owned()]);
    assert!(ledger.remove_cook_profile("diet", "2031-03-04").is_err());
    assert!(ledger
        .remove_cook_profile("session_hours", "2031-03-04")
        .is_err());
    let again = ledger
        .set_cook_profile("diet", None, None, Some("pescatarian"), "2031-03-05")
        .unwrap();
    assert_ne!(gone, again);
    let path = Ledger::path_in(&root);
    let raw = Connection::open(path).unwrap();
    let rows: i64 = raw
        .query_row(
            "SELECT count(*) FROM observation WHERE kind='cook_profile' AND subject='diet'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 3, "nothing is deleted");
    assert_eq!(
        ledger
            .cook_profile(false)
            .unwrap()
            .iter()
            .find(|e| e.subject == "diet")
            .unwrap()
            .value
            .as_deref(),
        Some("pescatarian")
    );
}

#[test]
fn a_lot_moves_by_supersede() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let rice = ledger
        .add_stock(&lot("rice", 500.0, "g", Some("2031-04-01")), "2031-03-03")
        .unwrap();
    let moved = ledger.move_lot(&rice, " pantry ", "2031-03-04").unwrap();
    let now = ledger.lot(&moved).unwrap();
    assert_eq!(
        (
            now.location.as_deref(),
            now.quantity,
            now.expires_on.as_deref()
        ),
        (Some("pantry"), 500.0, Some("2031-04-01"))
    );
    assert!(ledger.lot(&rice).is_err());
    let cleared = ledger.move_lot(&moved, "", "2031-03-04").unwrap();
    assert_eq!(ledger.lot(&cleared).unwrap().location, None);
}

#[test]
fn banks_list_their_mappings_and_a_header_is_the_first_line_only() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    assert_eq!(ledger.statement_mappings().unwrap(), vec![]);
    let mut other = mapping();
    other.bank = "another-bank".into();
    ledger.set_statement_mapping(&mapping()).unwrap();
    ledger.set_statement_mapping(&other).unwrap();
    let banks: Vec<String> = ledger
        .statement_mappings()
        .unwrap()
        .into_iter()
        .map(|m| m.bank)
        .collect();
    assert_eq!(
        banks,
        vec!["another-bank".to_owned(), "sample-bank".to_owned()]
    );
    assert_eq!(ledger.statement_mappings().unwrap()[1], mapping());
    let csv = root.join("statement.csv");
    fs::write(
        &csv,
        "\u{feff}Date,\"Description, long\",Amount,Currency\r\n\"unterminated\n",
    )
    .unwrap();
    let header = soliton_kernel::ledger::finance::read_statement_header(&csv).unwrap();
    assert_eq!(
        header,
        vec!["Date", "Description, long", "Amount", "Currency"]
    );
    fs::write(&csv, "").unwrap();
    assert!(soliton_kernel::ledger::finance::read_statement_header(&csv).is_err());
    assert_eq!(
        ledger.private_modules().unwrap(),
        vec!["finance".to_owned(), "health".to_owned()]
    );
}

#[test]
fn plan_notes_name_no_lot_the_plan_uses_as_unplanned() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let leek = ledger
        .add_stock(
            &lot("sample leek", 4.0, "item", Some("2031-03-05")),
            "2031-03-03",
        )
        .unwrap();
    ledger
        .add_stock(
            &lot("sample kale", 2.0, "item", Some("2031-03-04")),
            "2031-03-03",
        )
        .unwrap();
    // The plan uses part of the leek lot only; some of it is left over.
    let dishes = [DishSpec {
        name: "sample leek soup".into(),
        portions: 2.0,
        phase: 1,
        equipment: vec![],
        uses: vec![(leek, 1.0)],
    }];
    let plan = ledger
        .write_batch_plan(day("2031-03-03"), 3, &dishes)
        .unwrap();
    assert!(
        plan.notes.iter().all(|n| !n.contains("sample leek")),
        "{:?}",
        plan.notes
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.starts_with("sample kale") && n.ends_with("is not in any plan")),
        "{:?}",
        plan.notes
    );
}

fn plan_count(ledger: &Ledger) -> usize {
    ledger.batch_plans(None, false).unwrap().len()
}

#[test]
fn plan_ingredients_come_in_goes_off_order_marked_soon() {
    let mut ledger = Ledger::migrate(&root()).unwrap();
    let onions = ledger
        .add_stock(
            &lot("onions", 1500.0, "g", Some("2026-10-20")),
            "2026-10-04",
        )
        .unwrap();
    let beans = ledger
        .add_stock(&lot("beans dried", 2000.0, "g", None), "2026-10-04")
        .unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    let ginger = ledger
        .add_stock(&lot("ginger", 150.0, "g", Some("2026-10-07")), "2026-10-04")
        .unwrap();
    let dish = |name: &str, uses: Vec<(ObjectId, f64)>| DishSpec {
        name: name.into(),
        portions: 4.0,
        phase: 0,
        equipment: vec![],
        uses,
    };
    let plan = ledger
        .write_batch_plan(
            day("2026-10-05"),
            3,
            &[
                dish(
                    "chilli",
                    vec![
                        (onions.clone(), 300.0),
                        (beans, 500.0),
                        (ginger.clone(), 20.0),
                    ],
                ),
                dish("saag", vec![(spinach, 300.0), (ginger, 40.0)]),
            ],
        )
        .unwrap();
    let order: Vec<(&str, f64, bool)> = plan
        .ingredients
        .iter()
        .map(|i| (i.subject.as_str(), i.quantity, i.goes_off_soon))
        .collect();
    assert_eq!(
        order,
        [
            ("spinach", 300.0, true),
            ("ginger", 60.0, true),
            ("onions", 300.0, false),
            ("beans dried", 500.0, false)
        ]
    );
    let _ = onions;
}

#[test]
fn excluded_ingredient_refuses_the_dish_by_name_and_writes_nothing() {
    let mut ledger = Ledger::migrate(&root()).unwrap();
    let coriander = ledger
        .add_stock(
            &lot("coriander fresh", 60.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .set_cook_profile(
            "exclude",
            None,
            None,
            Some("Coriander, peanuts"),
            "2026-10-04",
        )
        .unwrap();
    let mut dish = DishSpec {
        name: "green curry".into(),
        portions: 4.0,
        phase: 0,
        equipment: vec![],
        uses: vec![(spinach.clone(), 200.0), (coriander, 30.0)],
    };
    match ledger.write_batch_plan(day("2026-10-05"), 3, &[dish.clone()]) {
        Err(KernelError::Ledger(text)) => assert_eq!(
            text,
            "green curry uses coriander, which the cook profile excludes"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(plan_count(&ledger), 0);
    dish.uses = vec![(spinach, 200.0)];
    let plan = ledger
        .write_batch_plan(day("2026-10-05"), 3, &[dish])
        .unwrap();
    assert!(plan
        .ingredients
        .iter()
        .all(|i| !i.subject.contains("coriander")));
    assert_eq!(plan_count(&ledger), 1);
}

#[test]
fn equipment_not_listed_is_refused_by_name_then_the_same_plan_is_written() {
    let mut ledger = Ledger::migrate(&root()).unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    let dish = DishSpec {
        name: "saag".into(),
        portions: 4.0,
        phase: 1,
        equipment: vec!["wok".into()],
        uses: vec![(spinach, 300.0)],
    };
    match ledger.write_batch_plan(day("2026-10-05"), 3, &[dish.clone()]) {
        Err(KernelError::Ledger(text)) => assert!(
            text.contains("wok") && text.contains("not in the equipment list"),
            "{text}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(plan_count(&ledger), 0);
    ledger
        .set_equipment("wok", 1.0, None, "2026-10-05")
        .unwrap();
    let plan = ledger
        .write_batch_plan(day("2026-10-05"), 3, &[dish])
        .unwrap();
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(plan_count(&ledger), 1);
}

#[test]
fn plan_result_returns_the_cook_profile_entries() {
    let mut ledger = Ledger::migrate(&root()).unwrap();
    let spinach = ledger
        .add_stock(
            &lot("spinach", 400.0, "g", Some("2026-10-06")),
            "2026-10-04",
        )
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(14.0),
            Some("portion"),
            None,
            "2026-10-04",
        )
        .unwrap();
    ledger
        .set_cook_profile("session_hours", Some(4.0), Some("hour"), None, "2026-10-04")
        .unwrap();
    ledger
        .set_cook_profile("diet", None, None, Some("vegetarian"), "2026-10-04")
        .unwrap();
    let dish = DishSpec {
        name: "saag".into(),
        portions: 4.0,
        phase: 0,
        equipment: vec![],
        uses: vec![(spinach, 300.0)],
    };
    let plan = ledger
        .write_batch_plan(day("2026-10-05"), 3, &[dish])
        .unwrap();
    let read: Vec<(&str, Option<f64>, Option<&str>)> = plan
        .constraints
        .iter()
        .map(|c| (c.subject.as_str(), c.quantity, c.value.as_deref()))
        .collect();
    assert_eq!(
        read,
        [
            ("diet", None, Some("vegetarian")),
            ("portions_per_week", Some(14.0), None),
            ("session_hours", Some(4.0), None)
        ]
    );
}
