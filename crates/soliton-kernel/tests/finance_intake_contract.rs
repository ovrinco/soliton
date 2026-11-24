//! The finance intake (soliton-whole T11, design/finance_intake.md): the intake
//! folder, documents, and the five ways money comes in, each line pointing at its
//! document or at cash. Fixtures are test files under tests/fixtures/hearth/finance.

use rusqlite::Connection;
use serde_json::{json, Value};
use soliton_kernel::ledger::finance::{
    self, category_for, intake_dir, DateFormat, DocumentKind, LineFilter, PayStub, ReceiptLine,
    StatementMapping, CATEGORIES, OTHER,
};
use soliton_kernel::ledger::{Ledger, Question, LEDGER_FILE};
use soliton_kernel::{CivilDate, KernelError, ObjectId};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(ObjectId::mint("financeintake").unwrap().to_string());
    fs::create_dir(&path).unwrap();
    path
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hearth/finance")
        .join(name)
}

fn day(text: &str) -> CivilDate {
    text.parse().unwrap()
}

/// Drops a fixture into the intake folder and registers it.
fn drop_in(
    ledger: &Ledger,
    root: &Path,
    name: &str,
    kind: DocumentKind,
    arrived: &str,
) -> ObjectId {
    let target = intake_dir(root).join(name);
    fs::copy(fixture(name), &target).unwrap();
    ledger
        .register_document(&target, kind, day(arrived))
        .unwrap()
}

fn bank(name: &str, date_format: DateFormat, debit_negative: bool) -> StatementMapping {
    StatementMapping {
        bank: name.into(),
        date: "Date".into(),
        date_format,
        description: "Description".into(),
        amount: "Amount".into(),
        currency: "Currency".into(),
        category: Some("Type".into()),
        debit_negative,
        categories: BTreeMap::from([("Groceries".into(), "food:groceries".into())]),
    }
}

fn sql(root: &Path, query: &str) -> Vec<Vec<Value>> {
    let connection = Connection::open(root.join(LEDGER_FILE)).unwrap();
    let mut statement = connection.prepare(query).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|i| {
                    Ok(match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Integer(n) => json!(n),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        rusqlite::types::ValueRef::Real(n) => json!(n),
                        _ => Value::Null,
                    })
                })
                .collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn intake_folder_is_made_private_on_first_list_and_tells_waiting_from_in() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    assert!(
        !intake_dir(&root).exists(),
        "nothing makes the folder before it is listed"
    );
    assert!(ledger.list_intake().unwrap().is_empty());
    let mode = fs::metadata(intake_dir(&root))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
    for name in [
        "test-statement-bank.csv",
        "test-receipt-fuel.png",
        "test-receipt-email.eml",
    ] {
        fs::copy(fixture(name), intake_dir(&root).join(name)).unwrap();
    }
    let waiting = ledger.list_intake().unwrap();
    assert_eq!(waiting.len(), 3);
    assert!(waiting
        .iter()
        .all(|f| f.document.is_none() && f.bytes > 0 && f.sha256.starts_with("sha256:")));
    let kinds: Vec<_> = waiting.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        [
            Some(DocumentKind::Eml),
            Some(DocumentKind::Png),
            Some(DocumentKind::Csv)
        ],
        "by name"
    );
    let picture = intake_dir(&root).join("test-receipt-fuel.png");
    let id = ledger
        .register_document(&picture, DocumentKind::Png, day("2026-10-11"))
        .unwrap();
    let listed = ledger.list_intake().unwrap();
    let fuel = listed
        .iter()
        .find(|f| f.name == "test-receipt-fuel.png")
        .unwrap();
    assert_eq!(
        (fuel.document.as_ref(), fuel.day.as_str()),
        (Some(&id), "2026-10-11")
    );
    assert_eq!(listed.iter().filter(|f| f.document.is_none()).count(), 2);
    // The same file twice, or its bytes under another name, is refused with the first document's id.
    let again = ledger
        .register_document(&picture, DocumentKind::Png, day("2026-10-12"))
        .unwrap_err();
    assert!(
        matches!(&again, KernelError::Conflict(m) if m.contains(id.as_str())),
        "{again}"
    );
    let copy = intake_dir(&root).join("test-receipt-fuel-copy.png");
    fs::copy(&picture, &copy).unwrap();
    let twin = ledger
        .register_document(&copy, DocumentKind::Png, day("2026-10-12"))
        .unwrap_err();
    assert!(twin.to_string().contains(id.as_str()), "{twin}");
    // Only files in the folder, under the kind their name carries.
    assert!(ledger
        .register_document(
            &fixture("test-pay-stub.csv"),
            DocumentKind::Csv,
            day("2026-10-12")
        )
        .is_err());
    let email = intake_dir(&root).join("test-receipt-email.eml");
    assert!(ledger
        .register_document(&email, DocumentKind::Pdf, day("2026-10-12"))
        .is_err());
    let rows = sql(&root, "SELECT subject, category, writer, data_class, json_extract(extra,'$.kind'), json_extract(extra,'$.bytes') FROM current_observation WHERE kind='document'");
    assert_eq!(
        rows,
        [vec![
            json!("test-receipt-fuel.png"),
            json!("png"),
            json!("finance"),
            json!("personal_sensitive"),
            json!("png"),
            json!(fs::metadata(&picture).unwrap().len())
        ]]
    );
}

#[test]
fn every_path_writes_lines_that_point_at_their_document_or_cash_and_q6_answers_across_them() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.list_intake().unwrap();
    // A bank statement and a card statement, each in its bank's columns.
    ledger
        .set_statement_mapping(&bank("test-bank", DateFormat::DayMonthYear, true))
        .unwrap();
    ledger
        .set_statement_mapping(&bank("test-card", DateFormat::Iso, false))
        .unwrap();
    let bank_doc = drop_in(
        &ledger,
        &root,
        "test-statement-bank.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    let card_doc = drop_in(
        &ledger,
        &root,
        "test-statement-card.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    let first = ledger
        .import_statement_document(&bank_doc, "test-bank")
        .unwrap();
    assert_eq!((first.added.len(), first.credits), (2, 1));
    assert_eq!(
        ledger
            .import_statement_document(&card_doc, "test-card")
            .unwrap()
            .added
            .len(),
        2
    );
    let repeat = ledger
        .import_statement_document(&bank_doc, "test-bank")
        .unwrap();
    assert_eq!((repeat.added.len(), repeat.duplicates), (0, 2));
    // The prototype's fixed-header CSV.
    let fixed = drop_in(
        &ledger,
        &root,
        "test-purchases-fixed.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    assert_eq!(
        ledger.import_fixed_csv(&fixed, "CAD").unwrap().added.len(),
        3
    );
    assert!(
        ledger.import_fixed_csv(&fixed, "CAD").is_err(),
        "a document is read once"
    );
    // A receipt picture (his fuel example) and an e-mailed receipt, typed once against each.
    let picture = drop_in(
        &ledger,
        &root,
        "test-receipt-fuel.png",
        DocumentKind::Png,
        "2026-10-12",
    );
    let line = |item: &str, quantity: f64, unit: &str, amount: i64, category: &str, on: &str| {
        ReceiptLine {
            merchant: "Canadian Tire Gas+".into(),
            day: day(on),
            item: item.into(),
            quantity,
            unit: unit.into(),
            amount_minor: amount,
            currency: "CAD".into(),
            category: category.into(),
        }
    };
    let fuel = ledger
        .record_receipt_lines(
            &picture,
            &[
                line("Premium", 42.0, "L", 7430, "Fuel", "2026-10-11"),
                line("Washer fluid", 1.0, "jug", 699, "Car care", "2026-10-11"),
            ],
        )
        .unwrap();
    assert_eq!(fuel.len(), 2);
    assert!(ledger
        .record_receipt_lines(
            &picture,
            &[line("Premium", 42.0, "L", 7430, "Fuel", "2026-10-11")]
        )
        .is_err());
    let email = drop_in(
        &ledger,
        &root,
        "test-receipt-email.eml",
        DocumentKind::Eml,
        "2026-10-12",
    );
    ledger
        .record_receipt_lines(
            &email,
            &[line(
                "Regular unleaded",
                38.2,
                "L",
                6150,
                "gas",
                "2026-10-10",
            )],
        )
        .unwrap();
    // A pay stub, read from its file.
    let stub = drop_in(
        &ledger,
        &root,
        "test-pay-stub.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    let income = ledger.import_pay_stub_csv(&stub).unwrap();
    assert!(
        ledger.import_pay_stub_csv(&stub).is_err(),
        "a pay stub is read once"
    );
    // Cash by hand.
    ledger
        .record_cash(
            day("2026-10-07"),
            "TEST Farmers market",
            1200,
            "CAD",
            "Groceries",
        )
        .unwrap();

    let sources = sql(&root, "SELECT source_ref FROM current_observation WHERE kind IN ('purchase_line','income_line') ORDER BY source_ref");
    assert_eq!(sources.len(), 2 + 2 + 3 + 2 + 1 + 1 + 1);
    let documents: Vec<String> = [&bank_doc, &card_doc, &fixed, &picture, &email, &stub]
        .iter()
        .map(|d| format!("document:{d}#"))
        .collect();
    for row in &sources {
        let source = row[0].as_str().unwrap();
        assert!(
            source == "cash" || documents.iter().any(|d| source.starts_with(d.as_str())),
            "{source}"
        );
    }
    let income_lines = ledger.income_lines(false).unwrap();
    assert_eq!(income_lines.len(), 1);
    assert_eq!(
        (
            &income_lines[0].id,
            income_lines[0].net_minor,
            income_lines[0].gross_minor,
            income_lines[0].employer.as_str()
        ),
        (&income, 185_000, 240_000, "TEST Employer Ltd")
    );
    assert_eq!(
        income_lines[0].source.as_deref(),
        Some(format!("document:{stub}#1").as_str())
    );
    assert!(
        ledger.income_lines(true).unwrap().is_empty(),
        "income is private with the module"
    );

    // Q6 answers over statement, fixed CSV, receipt, e-mail and cash lines in one table.
    let q6 = ledger.ask(Question::SpendByCategoryWeeks, false).unwrap();
    let cells: Vec<(String, String, String)> = q6
        .rows
        .iter()
        .map(|r| {
            (
                r[0].as_str().unwrap().into(),
                r[1].as_str().unwrap().into(),
                r[2].as_str().unwrap().into(),
            )
        })
        .collect();
    let fuel_week = cells
        .iter()
        .find(|(w, c, _)| w == "2026-10-05" && c == "transport:fuel")
        .unwrap();
    assert_eq!(
        fuel_week.2, "135.80",
        "fuel from the picture and the e-mail"
    );
    let groceries: Vec<_> = cells
        .iter()
        .filter(|(_, c, _)| c == "food:groceries")
        .collect();
    assert_eq!(
        groceries
            .iter()
            .map(|(w, _, s)| (w.as_str(), s.as_str()))
            .collect::<Vec<_>>(),
        [("2026-09-28", "23.40"), ("2026-10-05", "16.99")],
        "statement; fixed CSV + cash"
    );
    let captures = sql(&root, "SELECT DISTINCT json_extract(extra,'$.capture') FROM current_observation WHERE kind='purchase_line' ORDER BY 1");
    assert_eq!(
        captures,
        [
            vec![json!("cash")],
            vec![json!("email_receipt")],
            vec![json!("fixed_csv")],
            vec![json!("receipt")],
            vec![json!("statement")]
        ]
    );
    let standings = sql(&root, "SELECT DISTINCT json_extract(extra,'$.standing') FROM current_observation WHERE kind IN ('purchase_line','income_line')");
    assert_eq!(standings, [vec![json!("observed")]]);
    // A picture's lines name the picture as their source in the lines read.
    let fuel_lines = ledger
        .purchase_lines(
            &LineFilter {
                week: None,
                category: Some("transport:fuel".into()),
            },
            false,
        )
        .unwrap();
    assert_eq!(fuel_lines.len(), 2);
    assert!(fuel_lines
        .iter()
        .any(|l| l.source.as_deref() == Some(format!("document:{picture}#1").as_str())));
}

#[test]
fn every_category_word_is_kept_an_unknown_one_under_other_and_recategorise_moves_it() {
    assert_eq!(category_for("Groceries"), "food:groceries");
    assert_eq!(category_for("fuel"), "transport:fuel");
    assert_eq!(category_for("Gas"), "transport:fuel");
    assert_eq!(category_for("Toiletries"), "household:toiletries");
    assert_eq!(category_for("Property tax"), "household:property_tax");
    assert_eq!(category_for("household:utilities"), "household:utilities");
    assert_eq!(category_for("Decor"), OTHER);
    assert_eq!(category_for(" "), finance::UNCATEGORISED);
    let keys: Vec<&str> = CATEGORIES.iter().map(|(k, _)| *k).collect();
    for key in [
        "food:groceries",
        "transport:fuel",
        "household:toiletries",
        "household:property_tax",
        "other",
    ] {
        assert!(keys.contains(&key), "{key}");
    }
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger.list_intake().unwrap();
    let fixed = drop_in(
        &ledger,
        &root,
        "test-purchases-fixed.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    ledger.import_fixed_csv(&fixed, "CAD").unwrap();
    let decor = sql(&root, "SELECT id, category, json_extract(extra,'$.category_word') FROM current_observation WHERE subject='Picture frame'");
    assert_eq!(
        (&decor[0][1], &decor[0][2]),
        (&json!("other"), &json!("Decor")),
        "kept under other with its word, never refused"
    );
    let tax = ledger
        .record_cash(
            day("2026-10-08"),
            "TEST Town office",
            98_000,
            "CAD",
            "Property taxes",
        )
        .unwrap();
    let id: ObjectId = decor[0][0].as_str().unwrap().parse().unwrap();
    let moved = ledger.recategorise_line(&id, "household:other").unwrap();
    let after = sql(&root, &format!("SELECT category, source_ref, json_extract(extra,'$.category_word') FROM current_observation WHERE id='{moved}'"));
    assert_eq!(
        after[0],
        vec![
            json!("household:other"),
            json!(format!("document:{fixed}#3")),
            json!("Decor")
        ]
    );
    let tax_row = sql(
        &root,
        &format!("SELECT category, source_ref FROM current_observation WHERE id='{tax}'"),
    );
    assert_eq!(
        tax_row[0],
        vec![json!("household:property_tax"), json!("cash")]
    );
}

#[test]
fn a_statement_from_text_keeps_its_old_source_and_a_changed_document_is_refused() {
    let root = root();
    let mut ledger = Ledger::migrate(&root).unwrap();
    ledger
        .set_statement_mapping(&bank("test-bank", DateFormat::DayMonthYear, true))
        .unwrap();
    let text = fs::read_to_string(fixture("test-statement-bank.csv")).unwrap();
    ledger.import_statement("test-bank", &text).unwrap();
    let lines = ledger
        .purchase_lines(&LineFilter::default(), false)
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l
        .source
        .as_deref()
        .is_some_and(|s| s.starts_with("statement:test-bank:2026-10-01..2026-10-03#"))));
    // The unmapped bank word waits for his pick, its word kept.
    let books = sql(&root, "SELECT category, json_extract(extra,'$.category_word') FROM current_observation WHERE subject='TEST Bookshop'");
    assert_eq!(books[0], vec![json!("uncategorised"), json!("Books")]);
    ledger.list_intake().unwrap();
    let doc = drop_in(
        &ledger,
        &root,
        "test-pay-stub.csv",
        DocumentKind::Csv,
        "2026-10-12",
    );
    fs::write(
        intake_dir(&root).join("test-pay-stub.csv"),
        "field,value\nemployer,changed\n",
    )
    .unwrap();
    assert!(matches!(
        ledger.import_pay_stub_csv(&doc),
        Err(KernelError::Conflict(_))
    ));
    let stub =
        PayStub::from_csv(&fs::read_to_string(fixture("test-pay-stub.csv")).unwrap()).unwrap();
    assert_eq!(
        (stub.gross_minor, stub.net_minor, stub.deductions.len()),
        (240_000, 185_000, 2)
    );
}

#[test]
fn the_two_kinds_are_added_once_and_write_no_observation() {
    let root = root();
    let ledger = Ledger::migrate(&root).unwrap();
    let before = ledger.kinds().unwrap().len();
    ledger.ensure_finance_kinds().unwrap();
    ledger.ensure_finance_kinds().unwrap();
    assert_eq!(ledger.kinds().unwrap().len(), before + 2);
    assert_eq!(
        sql(&root, "SELECT count(*) FROM observation"),
        [vec![json!(0)]]
    );
}

/// The proof on a copy of his workspace (never the live one): the folder is
/// absent until the first list, made 0700 by it; the kinds go 10 to 12 and no
/// observation is written. Run with `SOLITON_T11_WORKSPACE=<copy> --ignored`.
#[test]
#[ignore = "runs only on a workspace copy named by SOLITON_T11_WORKSPACE"]
fn workspace_copy_first_list() {
    let Ok(copy) = std::env::var("SOLITON_T11_WORKSPACE") else {
        return;
    };
    let copy = PathBuf::from(copy);
    assert!(
        !copy.starts_with(dirs_home().join(".local/share/soliton")),
        "never the live workspace"
    );
    let existed = intake_dir(&copy).exists();
    let ledger = Ledger::open_existing(&copy).unwrap().unwrap();
    let kinds_before = ledger.kinds().unwrap().len();
    let rows_before = sql(&copy, "SELECT count(*) FROM observation");
    let listed = ledger.list_intake().unwrap();
    let mode = fs::metadata(intake_dir(&copy))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    println!(
        "T11-COPY existed_before={existed} mode={mode:o} listed={} kinds_before={kinds_before} kinds_after={} observations_before={} observations_after={}",
        listed.len(),
        ledger.kinds().unwrap().len(),
        rows_before[0][0],
        sql(&copy, "SELECT count(*) FROM observation")[0][0]
    );
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Every fixture into a copy of his real ledger (a folder holding only a copied
/// `ledger.sqlite`, named by SOLITON_T11_LEDGER_COPY): each line points at a
/// document or cash, and a second registration names the first document.
#[test]
#[ignore = "runs only on a ledger copy named by SOLITON_T11_LEDGER_COPY"]
fn fixtures_into_a_ledger_copy() {
    let Ok(copy) = std::env::var("SOLITON_T11_LEDGER_COPY") else {
        return;
    };
    let root = PathBuf::from(copy);
    assert!(
        !root.starts_with(dirs_home().join(".local/share/soliton")),
        "never the live workspace"
    );
    let mut ledger = Ledger::open_existing(&root).unwrap().unwrap();
    ledger.list_intake().unwrap();
    ledger
        .set_statement_mapping(&bank("test-bank", DateFormat::DayMonthYear, true))
        .unwrap();
    ledger
        .set_statement_mapping(&bank("test-card", DateFormat::Iso, false))
        .unwrap();
    let bank_doc = drop_in(
        &ledger,
        &root,
        "test-statement-bank.csv",
        DocumentKind::Csv,
        "2026-10-28",
    );
    ledger
        .import_statement_document(&bank_doc, "test-bank")
        .unwrap();
    let card_doc = drop_in(
        &ledger,
        &root,
        "test-statement-card.csv",
        DocumentKind::Csv,
        "2026-10-28",
    );
    ledger
        .import_statement_document(&card_doc, "test-card")
        .unwrap();
    let fixed = drop_in(
        &ledger,
        &root,
        "test-purchases-fixed.csv",
        DocumentKind::Csv,
        "2026-10-28",
    );
    ledger.import_fixed_csv(&fixed, "CAD").unwrap();
    let line = |item: &str, amount: i64, on: &str| ReceiptLine {
        merchant: "Canadian Tire Gas+".into(),
        day: day(on),
        item: item.into(),
        quantity: 1.0,
        unit: "item".into(),
        amount_minor: amount,
        currency: "CAD".into(),
        category: "Fuel".into(),
    };
    let picture = drop_in(
        &ledger,
        &root,
        "test-receipt-fuel.png",
        DocumentKind::Png,
        "2026-10-28",
    );
    ledger
        .record_receipt_lines(&picture, &[line("Premium", 7430, "2026-10-11")])
        .unwrap();
    let email = drop_in(
        &ledger,
        &root,
        "test-receipt-email.eml",
        DocumentKind::Eml,
        "2026-10-28",
    );
    ledger
        .record_receipt_lines(&email, &[line("Regular unleaded", 6150, "2026-10-10")])
        .unwrap();
    let stub = drop_in(
        &ledger,
        &root,
        "test-pay-stub.csv",
        DocumentKind::Csv,
        "2026-10-28",
    );
    ledger.import_pay_stub_csv(&stub).unwrap();
    ledger
        .record_cash(
            day("2026-10-07"),
            "TEST Farmers market",
            1200,
            "CAD",
            "Groceries",
        )
        .unwrap();
    let again = ledger
        .register_document(
            &intake_dir(&root).join("test-receipt-fuel.png"),
            DocumentKind::Png,
            day("2026-10-28"),
        )
        .unwrap_err();
    println!("T11-DUPLICATE {again}");
    println!("T11-PICTURE {picture}");
    for row in sql(&root, "SELECT kind, source_ref, category FROM current_observation WHERE kind IN ('purchase_line','income_line') ORDER BY kind, source_ref") {
        println!("T11-LINE {} {} {}", row[0], row[1], row[2]);
    }
    println!(
        "T11-KINDS {}",
        sql(&root, "SELECT count(*) FROM kind")[0][0]
    );
}
