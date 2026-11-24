//! The kitchen's stock as he described it (T12): a lot carries a picture and a
//! weight, a food purchase becomes one lot once, a plan carries the coverage he
//! chooses, and a drafted plan comes back as `inferred` dishes. Every lot here
//! is a test lot in a temporary workspace.

use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use soliton_kernel::ledger::finance::{DateFormat, StatementMapping};
use soliton_kernel::ledger::kitchen::{DishSpec, LotSource, LotSpec, PICTURES_DIR};
use soliton_kernel::ledger::{Ledger, Question, LEDGER_FILE};
use soliton_kernel::{CivilDate, KernelError, ObjectId};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("kitchenstock").unwrap().to_string());
    fs::create_dir(&path).unwrap();
    path
}

fn day(text: &str) -> CivilDate {
    text.parse().unwrap()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hearth/kitchen")
        .join(name)
}

fn sha256(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn lot(subject: &str, quantity: f64, unit: &str) -> LotSpec {
    LotSpec {
        subject: subject.into(),
        category: Some("food:produce".into()),
        quantity,
        unit: unit.into(),
        expires_on: None,
        location: Some("pantry".into()),
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
            ("Home".into(), "home:hardware".into()),
        ]),
    }
}

const TWO_FOOD_LINES: &str = "Date,Description,Amount,Currency,Type\n\
05/10/2026,Market onions,-4.80,EUR,Groceries\n\
06/10/2026,Mill flour,-3.50,EUR,Groceries\n\
06/10/2026,Hardware shop,-9.99,EUR,Home\n";

fn extra_of(root: &Path, id: &ObjectId) -> Value {
    let check = Connection::open(root.join(LEDGER_FILE)).unwrap();
    let text: String = check
        .query_row(
            "SELECT extra FROM observation WHERE id = ?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_lot_carries_a_picture_and_a_weight_after_reopening() {
    let root = root();
    let picture = fixture("test-onions.png");
    let before = sha256(&picture);
    let mut ledger = Ledger::migrate(&root).unwrap();
    let onions = ledger
        .add_stock_with_picture(&lot("test onions", 1250.0, "G"), &picture, "2026-10-06")
        .unwrap();
    drop(ledger);
    let ledger = Ledger::open_existing(&root).unwrap().unwrap();
    let read = ledger.lot(&onions).unwrap();
    assert!(read.has_picture);
    assert_eq!(
        read.weight(),
        Some((1250.0, "g")),
        "the weight is the quantity in a mass unit"
    );
    let kept = root.join(PICTURES_DIR).join(format!("{onions}.png"));
    assert_eq!(ledger.picture_path(&read).unwrap(), Some(kept.clone()));
    let names: Vec<String> = fs::read_dir(root.join(PICTURES_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [format!("{onions}.png")]);
    assert_eq!(
        fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let extra = extra_of(&root, &onions);
    assert_eq!(extra["sha256"], json!(sha256(&kept)));
    assert_eq!(
        extra["picture"],
        json!(format!("kitchen/pictures/{onions}.png"))
    );
    assert_eq!(sha256(&picture), before, "the fixture is untouched");
    assert_eq!(
        ledger
            .stock(false)
            .unwrap()
            .iter()
            .filter(|l| l.has_picture)
            .count(),
        1
    );
}

#[test]
fn a_picture_is_replaced_by_supersede_and_kept_through_use() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let flour = ledger
        .add_stock(&lot("test flour", 2.0, "kg"), "2026-10-06")
        .unwrap();
    assert!(!ledger.lot(&flour).unwrap().has_picture);
    let pictured = ledger
        .set_picture(&flour, &fixture("test-flour.jpg"), "2026-10-06")
        .unwrap();
    assert!(
        ledger.lot(&flour).is_err(),
        "the lot without a picture is superseded"
    );
    let used = ledger.use_stock(&pictured, 0.5, "2026-10-07").unwrap();
    let now = ledger.lot(&used).unwrap();
    assert!(
        now.has_picture,
        "the picture stays with the lot through use"
    );
    assert_eq!(now.weight(), Some((1.5, "kg")));
    assert_eq!(
        ledger.picture_path(&now).unwrap(),
        Some(root.join(PICTURES_DIR).join(format!("{pictured}.jpg")))
    );
    let eggs = ledger
        .add_stock(&lot("test eggs", 12.0, "item"), "2026-10-06")
        .unwrap();
    assert_eq!(
        ledger.lot(&eggs).unwrap().weight(),
        None,
        "a count stays a count"
    );
    // A picture of another type, or no file, is refused and nothing is written.
    let lots = ledger.stock(false).unwrap().len();
    assert!(ledger
        .add_stock_with_picture(
            &lot("test oats", 500.0, "g"),
            &root.join(LEDGER_FILE),
            "2026-10-06"
        )
        .is_err());
    assert!(ledger
        .add_stock_with_picture(
            &lot("test oats", 500.0, "g"),
            &root.join("absent.png"),
            "2026-10-06"
        )
        .is_err());
    assert_eq!(ledger.stock(false).unwrap().len(), lots);
    assert_eq!(fs::read_dir(root.join(PICTURES_DIR)).unwrap().count(), 1);
}

#[test]
fn a_food_purchase_becomes_one_lot_once() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.set_statement_mapping(&mapping()).unwrap();
    assert_eq!(
        ledger
            .import_statement("sample-bank", TWO_FOOD_LINES)
            .unwrap()
            .added
            .len(),
        3
    );
    let offered = ledger.lots_from_purchases(false).unwrap();
    assert_eq!(
        offered
            .iter()
            .map(|l| l.subject.as_str())
            .collect::<Vec<_>>(),
        ["Market onions", "Mill flour"],
        "food lines only"
    );
    let line = offered[0].id.clone();
    let first = ledger
        .lot_from_purchase(
            &line,
            1500.0,
            "g",
            Some(day("2026-10-20")),
            Some("pantry"),
            "2026-10-06",
        )
        .unwrap();
    let left = ledger.lots_from_purchases(false).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].subject, "Mill flour");
    match ledger.lot_from_purchase(&line, 1500.0, "g", None, None, "2026-10-06") {
        Err(KernelError::Conflict(message)) => {
            assert!(message.contains(first.as_str()), "{message}")
        }
        other => panic!("a second lot for one line: {other:?}"),
    }
    let check = Connection::open(root.join(LEDGER_FILE)).unwrap();
    let count = |check: &Connection| -> i64 {
        check
            .query_row(
                "SELECT count(*) FROM current_observation WHERE kind='inventory' AND json_extract(extra,'$.purchase_line') = ?1",
                [line.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(count(&check), 1);
    let read = ledger.lot(&first).unwrap();
    assert_eq!(
        (
            read.subject.as_str(),
            read.category.as_deref(),
            read.source.as_deref()
        ),
        ("Market onions", Some("food:groceries"), Some("receipt"))
    );
    assert_eq!(read.purchase_line.as_deref(), Some(line.as_str()));
    // The line shows that it is in stock, and the link holds through use and recategorising.
    assert_eq!(
        ledger.stocked_purchases(false).unwrap().get(line.as_str()),
        Some(&first)
    );
    let used = ledger.use_stock(&first, 500.0, "2026-10-07").unwrap();
    assert_eq!(ledger.lot_of_purchase(&line).unwrap(), Some(used));
    let moved = ledger.recategorise_line(&line, "food:produce").unwrap();
    assert_eq!(
        ledger.lots_from_purchases(false).unwrap().len(),
        1,
        "a recategorised line is still in stock"
    );
    assert!(ledger
        .lot_from_purchase(&moved, 1.0, "kg", None, None, "2026-10-07")
        .is_err());
    assert_eq!(count(&check), 1);
    let hardware = ledger
        .purchase_lines(&Default::default(), false)
        .unwrap()
        .into_iter()
        .find(|l| l.category == "home:hardware")
        .unwrap();
    assert!(
        ledger
            .lot_from_purchase(&hardware.id, 1.0, "item", None, None, "2026-10-07")
            .is_err(),
        "only food becomes stock"
    );
}

#[test]
fn a_plan_carries_the_coverage_he_chooses_and_notes_short_portions() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let rice = ledger
        .add_stock(&lot("test rice", 5000.0, "g"), "2026-10-06")
        .unwrap();
    ledger
        .set_equipment("dutch oven", 2.0, None, "2026-10-06")
        .unwrap();
    ledger
        .set_cook_profile(
            "portions_per_week",
            Some(14.0),
            Some("portion"),
            None,
            "2026-10-06",
        )
        .unwrap();
    let dish = |name: &str, portions: f64| DishSpec {
        name: name.into(),
        portions,
        phase: 1,
        equipment: vec!["dutch oven".into()],
        uses: vec![(rice.clone(), 500.0)],
    };
    let short = ledger
        .write_batch_plan_covering(day("2026-10-07"), 3, 14, &[dish("test pilaf", 10.0)], false)
        .unwrap();
    assert_eq!(short.coverage_days, Some(14));
    assert_eq!(
        short.notes,
        ["10 portions planned; 14 days at 14 a week wants 28"]
    );
    let check = Connection::open(root.join(LEDGER_FILE)).unwrap();
    let coverages: Vec<i64> = check
        .prepare("SELECT json_extract(extra,'$.coverage_days') FROM current_observation WHERE kind='batch_plan' AND json_extract(extra,'$.plan_id')='plan_2026-10-07'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(coverages, [14]);
    let met = ledger
        .write_batch_plan_covering(
            day("2026-10-08"),
            3,
            14,
            &[dish("test congee", 14.0), dish("test kedgeree", 14.0)],
            false,
        )
        .unwrap();
    assert!(
        met.notes.is_empty(),
        "no portions line when the plan covers the span: {:?}",
        met.notes
    );
    let plans = ledger.batch_plans(None, false).unwrap();
    assert_eq!(
        plans
            .iter()
            .map(|p| (p.plan_id.as_str(), p.coverage_days, p.dishes.len()))
            .collect::<Vec<_>>(),
        [
            ("plan_2026-10-07", Some(14), 1),
            ("plan_2026-10-08", Some(14), 2)
        ]
    );
    assert!(ledger
        .write_batch_plan_covering(day("2026-10-09"), 3, 0, &[dish("test soup", 1.0)], false)
        .is_err());
    // A plan written without a coverage reads none.
    ledger
        .write_batch_plan(day("2026-10-10"), 3, &[dish("test soup", 2.0)])
        .unwrap();
    assert_eq!(
        ledger.batch_plans(Some(day("2026-10-10")), false).unwrap()[0].coverage_days,
        None
    );
}

#[test]
fn the_draft_input_is_built_and_a_draft_comes_back_inferred() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    let mut beans = lot("test black beans", 2000.0, "g");
    beans.expires_on = Some(day("2027-01-01"));
    let beans = ledger.add_stock(&beans, "2026-10-06").unwrap();
    let onions = ledger
        .add_stock(&lot("test onions", 1000.0, "g"), "2026-10-06")
        .unwrap();
    ledger
        .set_equipment("cast iron pan", 1.0, Some("30 cm"), "2026-10-06")
        .unwrap();
    ledger
        .set_equipment("microwave", 1.0, None, "2026-10-06")
        .unwrap();
    ledger
        .set_cook_profile("diet", None, None, Some("high protein"), "2026-10-06")
        .unwrap();
    // A diet plan adjusted from what the records show is one profile write.
    ledger
        .set_cook_profile(
            "diet",
            None,
            None,
            Some("high protein, cottage cheese daily"),
            "2026-10-07",
        )
        .unwrap();
    let input = ledger.draft_input(day("2026-10-07"), 10, false).unwrap();
    assert_eq!(input["coverage_days"], json!(10));
    assert_eq!(input["stock"].as_array().unwrap().len(), 2);
    assert_eq!(
        input["stock"][0],
        json!({"lot": beans.as_str(), "subject": "test black beans", "quantity": 2000.0, "unit": "g", "goes_off": "2027-01-01"})
    );
    assert_eq!(
        input["equipment"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["cast iron pan", "microwave"]
    );
    assert_eq!(
        input["profile"][0]["value"],
        json!("high protein, cottage cheese daily")
    );
    assert!(ledger.draft_input(day("2026-10-07"), 0, false).is_err());
    let draft = [
        DishSpec {
            name: "test charred onions".into(),
            portions: 4.0,
            phase: 0,
            equipment: vec!["cast iron pan".into()],
            uses: vec![(onions.clone(), 600.0)],
        },
        DishSpec {
            name: "test black bean stew".into(),
            portions: 8.0,
            phase: 1,
            equipment: vec![],
            uses: vec![(beans.clone(), 800.0), (onions.clone(), 200.0)],
        },
    ];
    assert!(
        ledger
            .accept_draft(day("2026-10-07"), 10, &draft, " ")
            .is_err(),
        "a draft names its rule"
    );
    let written = ledger
        .accept_draft(day("2026-10-07"), 10, &draft, "kitchen.draft.local-model")
        .unwrap();
    assert_eq!(written.rows.len(), 2);
    let plan = &ledger.batch_plans(None, false).unwrap()[0];
    assert_eq!(plan.coverage_days, Some(10));
    assert_eq!(
        plan.dishes
            .iter()
            .map(|d| (d.standing.as_str(), d.derived_by.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("inferred", Some("kitchen.draft.local-model")),
            ("inferred", Some("kitchen.draft.local-model"))
        ]
    );
    for row in &written.rows {
        assert_eq!(extra_of(&root, row)["standing"], json!("inferred"));
    }
    // A dish he writes himself reads observed.
    ledger
        .write_batch_plan(
            day("2026-10-08"),
            3,
            &[DishSpec {
                name: "test rice".into(),
                portions: 1.0,
                phase: 1,
                equipment: vec![],
                uses: vec![],
            }],
        )
        .unwrap();
    assert_eq!(
        ledger.batch_plans(Some(day("2026-10-08")), false).unwrap()[0].dishes[0].standing,
        "observed"
    );
}

#[test]
fn a_draft_sets_the_standing_column_where_his_ledger_has_one() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    drop(ledger);
    let alter = Connection::open(root.join(LEDGER_FILE)).unwrap();
    alter
        .execute_batch("ALTER TABLE observation ADD COLUMN standing TEXT NOT NULL DEFAULT 'observed' CHECK (standing IN ('said','observed','inferred','verified'));")
        .unwrap();
    drop(alter);
    ledger = Ledger::open_existing(&root).unwrap().unwrap();
    let dish = DishSpec {
        name: "test omelette".into(),
        portions: 2.0,
        phase: 1,
        equipment: vec![],
        uses: vec![],
    };
    let written = ledger
        .accept_draft(
            day("2026-10-07"),
            7,
            &[dish.clone()],
            "kitchen.draft.local-model",
        )
        .unwrap();
    let typed = ledger
        .write_batch_plan(day("2026-10-08"), 3, &[dish])
        .unwrap();
    let check = Connection::open(root.join(LEDGER_FILE)).unwrap();
    let standing = |id: &ObjectId| -> String {
        check
            .query_row(
                "SELECT standing FROM observation WHERE id = ?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(standing(&written.rows[0]), "inferred");
    assert_eq!(standing(&typed.rows[0]), "observed");
}

#[test]
fn meals_cooked_and_food_bought_read_in_one_table() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.set_statement_mapping(&mapping()).unwrap();
    ledger
        .import_statement("sample-bank", TWO_FOOD_LINES)
        .unwrap();
    let line = ledger.lots_from_purchases(false).unwrap()[0].id.clone();
    let onions = ledger
        .lot_from_purchase(&line, 1500.0, "g", None, None, "2026-10-06")
        .unwrap();
    let dish = DishSpec {
        name: "test onion soup".into(),
        portions: 4.0,
        phase: 1,
        equipment: vec![],
        uses: vec![(onions, 700.0)],
    };
    ledger
        .write_batch_plan_covering(day("2026-10-06"), 3, 7, &[dish], false)
        .unwrap();
    ledger
        .cook_dish("plan_2026-10-06", "test onion soup", day("2026-10-07"))
        .unwrap();
    let q3 = ledger.ask(Question::FoodSpendWeeks, false).unwrap();
    assert_eq!(
        q3.columns,
        ["week", "category", "spend_gbp", "meals_home", "meals_out"]
    );
    assert_eq!(
        q3.rows,
        vec![vec![
            json!("2026-10-05"),
            json!("food:groceries"),
            json!("8.30"),
            json!(1),
            json!(0)
        ]]
    );
}

/// On a copy of his real ledger (named by SOLITON_T12_REAL_WORKSPACE): no lot,
/// one piece of equipment, and the reads write nothing. Skipped without the copy.
#[test]
fn his_ledger_copy_reads_without_a_write() {
    let Ok(workspace) = std::env::var("SOLITON_T12_REAL_WORKSPACE") else {
        eprintln!("SOLITON_T12_REAL_WORKSPACE not set; skipped");
        return;
    };
    let workspace = PathBuf::from(workspace);
    let file = workspace.join(LEDGER_FILE);
    let before = sha256(&file);
    let ledger = Ledger::open_existing(&workspace).unwrap().unwrap();
    let lots = ledger.stock(false).unwrap();
    let pieces = ledger.equipment(false).unwrap();
    let offered = ledger.lots_from_purchases(false).unwrap();
    let input = ledger.draft_input(day("2026-10-28"), 7, false).unwrap();
    drop(ledger);
    assert_eq!(lots.len(), 0);
    assert_eq!(
        pieces.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["food scale"]
    );
    assert!(offered.is_empty());
    assert_eq!(input["equipment"].as_array().unwrap().len(), 1);
    assert_eq!(
        sha256(&file),
        before,
        "nothing is written to his ledger copy"
    );
    eprintln!(
        "real copy: {} lots, {} equipment ({}), {} purchase lines offered",
        lots.len(),
        pieces.len(),
        pieces[0].name,
        offered.len()
    );
}
