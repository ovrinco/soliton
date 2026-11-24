//! The Hearth service commands: the finance intake, the kitchen's pictures,
//! purchases, coverage and draft, and the now view, on the stage-1 fixtures.
use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::*;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("soliton-test").unwrap().to_string());
    fs::create_dir(&p).unwrap();
    p
}

fn fixture(dir: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hearth")
        .join(dir)
        .join(name)
}

fn service() -> (PathBuf, Service) {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    (p, s)
}

/// Drops a fixture into the intake folder and registers it: its document id.
fn drop_in(s: &mut Service, p: &Path, dir: &str, name: &str, kind: &str) -> Value {
    fs::copy(fixture(dir, name), p.join("intake/finance").join(name)).unwrap();
    s.command(
        "finance-document-register",
        &json!({"name":name,"kind":kind,"day":"2026-10-10"}),
    )
    .unwrap()["id"]
        .clone()
}

fn len(v: &Value) -> usize {
    v.as_array().map_or(usize::MAX, Vec::len)
}

const FINANCE_FIXTURES: [(&str, &str); 6] = [
    ("test-statement-bank.csv", "csv"),
    ("test-statement-card.csv", "csv"),
    ("test-purchases-fixed.csv", "csv"),
    ("test-pay-stub.csv", "csv"),
    ("test-receipt-fuel.png", "png"),
    ("test-receipt-email.eml", "eml"),
];

fn mapping(bank: &str, format: &str, debit_negative: bool) -> Value {
    json!({"bank":bank,"date":"Date","date_format":format,"description":"Description","amount":"Amount",
        "currency":"Currency","category":"Type","debit_negative":debit_negative,"categories":{"Groceries":"food:groceries"}})
}

#[test]
fn finance_intake_imports_every_kind_and_lists_its_documents() {
    let (p, mut s) = service();
    let intake = s.command("finance-intake", &json!({})).unwrap();
    assert_eq!(intake["files"], json!([]));
    let mode = fs::metadata(p.join("intake/finance"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
    for (name, _) in FINANCE_FIXTURES {
        fs::copy(
            fixture("finance", name),
            p.join("intake/finance").join(name),
        )
        .unwrap();
    }
    let waiting = s.command("finance-intake", &json!({})).unwrap()["files"].clone();
    assert_eq!(len(&waiting), 6);
    assert!(
        waiting
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["state"] == "waiting")
    );

    let mut ids = std::collections::BTreeMap::new();
    for (name, kind) in FINANCE_FIXTURES {
        let r = s
            .command(
                "finance-document-register",
                &json!({"name":name,"kind":kind,"day":"2026-10-10"}),
            )
            .unwrap();
        ids.insert(name, r["id"].as_str().unwrap().to_owned());
    }
    assert!(
        s.command(
            "finance-document-register",
            &json!({"name":"../ledger.sqlite","kind":"csv"})
        )
        .is_err()
    );
    let doc = |name: &str| ids[name].clone();

    s.command(
        "finance-statement-mapping",
        &mapping("bank", "DD/MM/YYYY", true),
    )
    .unwrap();
    s.command(
        "finance-statement-mapping",
        &mapping("card", "YYYY-MM-DD", false),
    )
    .unwrap();
    let bank = s
        .command(
            "finance-import",
            &json!({"document":doc("test-statement-bank.csv"),"bank":"bank"}),
        )
        .unwrap();
    assert_eq!(
        (
            bank["kind"].as_str(),
            bank["added"].as_u64(),
            bank["credits"].as_u64()
        ),
        (Some("statement"), Some(2), Some(1))
    );
    let card = s
        .command(
            "finance-statement-import",
            &json!({"document":doc("test-statement-card.csv"),"bank":"card"}),
        )
        .unwrap();
    assert_eq!(card["added"], 2);
    assert!(
        s.command(
            "finance-statement-import",
            &json!({"bank":"card","path":fixture("finance","test-statement-card.csv")})
        )
        .is_err()
    );
    let fixed_args = json!({"document":doc("test-purchases-fixed.csv")});
    assert!(
        s.command("finance-import", &fixed_args).is_err(),
        "a fixed file needs its currency"
    );
    let fixed = s
        .command(
            "finance-import",
            &json!({"document":doc("test-purchases-fixed.csv"),"currency":"CAD"}),
        )
        .unwrap();
    assert_eq!(
        (fixed["kind"].as_str(), fixed["added"].as_u64()),
        (Some("fixed_csv"), Some(3))
    );
    let stub = s
        .command(
            "finance-import",
            &json!({"document":doc("test-pay-stub.csv")}),
        )
        .unwrap();
    assert_eq!(
        (stub["kind"].as_str(), stub["added"].as_u64()),
        (Some("pay_stub"), Some(1))
    );
    assert!(
        s.command(
            "finance-import",
            &json!({"document":doc("test-receipt-fuel.png")})
        )
        .is_err()
    );
    let fuel = s
        .command("finance-receipt-lines", &json!({"document":doc("test-receipt-fuel.png"),"lines":[
            {"merchant":"TEST Fuel","day":"2026-10-08","item":"Regular fuel","quantity":32.5,"unit":"l","amount":"54.90","currency":"CAD","category":"Gas"}]}))
        .unwrap();
    assert_eq!(len(&fuel["lines"]), 1);
    s.command("finance-receipt-lines", &json!({"document":doc("test-receipt-email.eml"),"lines":[
        {"merchant":"TEST Online","day":"2026-10-07","item":"Fuel card top-up","quantity":1,"unit":"item","amount":25,"currency":"CAD","category":"fuel"}]}))
        .unwrap();
    let cash = s.command("finance-cash", &json!({"day":"2026-10-09","merchant":"TEST Market","amount":"7.25","currency":"cad","category":"Groceries"})).unwrap();
    assert_eq!(
        (
            cash["amount"].as_str(),
            cash["category"].as_str(),
            cash["currency"].as_str()
        ),
        (Some("7.25"), Some("food:groceries"), Some("CAD"))
    );

    let documents = s.command("finance-documents", &json!({})).unwrap()["documents"].clone();
    let counts: std::collections::BTreeMap<String, u64> = documents
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["name"].as_str().unwrap().to_owned(),
                d["lines"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        counts.values().collect::<Vec<_>>(),
        [&1, &3, &1, &1, &2, &2]
    );
    assert!(
        s.command("finance-intake", &json!({})).unwrap()["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["state"] == "in")
    );
    let lines = s.command("finance-lines", &json!({})).unwrap()["lines"].clone();
    assert_eq!(len(&lines), 10);
    let fuel_line = lines
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["merchant"] == "TEST Fuel")
        .unwrap();
    assert_eq!(
        fuel_line["source"],
        format!("document:{}#1", doc("test-receipt-fuel.png"))
    );
    assert_eq!(fuel_line["category"], "transport:fuel");
    assert!(
        lines
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["source"] == "cash")
    );
    let categories = s.command("finance-categories", &json!({})).unwrap()["categories"].clone();
    assert!(
        categories
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == &json!({"key":"transport:fuel","word":"Fuel"}))
    );

    // Hide-all with finance marked private: every read empty, writes kept.
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["finance"]}),
    )
    .unwrap();
    for (command, field) in [
        ("finance-intake", "files"),
        ("finance-documents", "documents"),
        ("finance-lines", "lines"),
        ("finance-banks", "banks"),
    ] {
        let r = s.command(command, &json!({})).unwrap();
        assert_eq!(
            (r["hide_all"].clone(), r[field].clone()),
            (json!(true), json!([])),
            "{command}"
        );
    }
    s.command("finance-cash", &json!({"day":"2026-10-09","merchant":"TEST Kiosk","amount":"2.00","currency":"CAD","category":"other"})).unwrap();
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(
        len(&s.command("finance-lines", &json!({})).unwrap()["lines"]),
        11
    );
    // The ledger's own module mark hides the same reads.
    s.command(
        "ledger-mark-private",
        &json!({"scope":"module","key":"finance"}),
    )
    .unwrap();
    assert_eq!(
        s.command("finance-documents", &json!({"hide_all":true}))
            .unwrap()["documents"],
        json!([])
    );
}

#[test]
fn kitchen_lots_carry_pictures_purchases_become_stock_once_and_plans_keep_coverage() {
    let (p, mut s) = service();
    s.command("finance-intake", &json!({})).unwrap();
    fs::copy(
        fixture("kitchen", "test-onions.png"),
        p.join("intake/finance/test-onions.png"),
    )
    .unwrap();
    fs::copy(
        fixture("kitchen", "test-flour.jpg"),
        p.join("intake/finance/test-flour.jpg"),
    )
    .unwrap();
    let onions = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"onions","quantity":1250,"unit":"g","expires_on":"2026-10-20",
            "picture":p.join("intake/finance/test-onions.png"),"day":"2026-10-10"}),
        )
        .unwrap();
    assert_eq!(
        (onions["has_picture"].as_bool(), onions["weight"].clone()),
        (Some(true), json!({"quantity":1250.0,"unit":"g"}))
    );
    // A file that is not a picture, or none at all, is refused; nothing is written.
    assert!(s.command("kitchen-stock-add", &json!({"subject":"x","quantity":1,"unit":"g","picture":fixture("finance","test-purchases-fixed.csv")})).is_err());
    assert!(s.command("kitchen-stock-add", &json!({"subject":"x","quantity":1,"unit":"g","picture":p.join("intake/finance/missing.png")})).is_err());
    assert!(
        s.command(
            "kitchen-stock-add",
            &json!({"subject":"x","quantity":1,"unit":"g","picture":"../outside.png"})
        )
        .is_err()
    );
    let flour = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"flour","quantity":2,"unit":"kg","day":"2026-10-10"}),
        )
        .unwrap();
    assert_eq!(flour["has_picture"], false);
    let pictured = s.command("kitchen-stock-picture", &json!({"lot":flour["id"],"picture":"intake/finance/test-flour.jpg","day":"2026-10-11"})).unwrap();
    assert_eq!(
        (
            pictured["has_picture"].as_bool(),
            pictured["supersedes"].clone()
        ),
        (Some(true), flour["id"].clone())
    );
    let listed = s.command("kitchen-stock-list", &json!({})).unwrap()["lots"].clone();
    let kept = listed
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| {
            l["picture"]
                .as_str()
                .is_some_and(|f| Path::new(f).is_file())
        })
        .count();
    assert_eq!(kept, 2);

    // One food line from the fixed file becomes one lot, once.
    let id = drop_in(&mut s, &p, "finance", "test-purchases-fixed.csv", "csv");
    s.command("finance-import", &json!({"document":id,"currency":"CAD"}))
        .unwrap();
    let offered = s.command("kitchen-from-purchases", &json!({})).unwrap()["lines"].clone();
    assert_eq!(len(&offered), 1);
    let line = offered[0]["line"].clone();
    let lot = s.command("kitchen-lot-from-purchase", &json!({"line":line,"quantity":2,"unit":"l","expires_on":"2026-10-15","day":"2026-10-10"})).unwrap();
    assert_eq!(lot["purchase_line"], line);
    let again = s
        .command(
            "kitchen-lot-from-purchase",
            &json!({"line":line,"quantity":2,"unit":"l"}),
        )
        .unwrap_err()
        .to_string();
    assert!(again.contains(lot["id"].as_str().unwrap()), "{again}");
    assert_eq!(
        s.command("kitchen-from-purchases", &json!({})).unwrap()["lines"],
        json!([])
    );

    // A plan for a chosen coverage, read back with it.
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"dutch oven","count":1,"day":"2026-10-10"}),
    )
    .unwrap();
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"diet","value":"high protein, cottage cheese daily","day":"2026-10-10"}),
    )
    .unwrap();
    let plan = s
        .command("kitchen-batch-plan", &json!({"day":"2026-10-12","coverage_days":14,"dishes":[
            {"name":"onion soup","portions":6,"phase":1,"equipment":["dutch oven"],"uses":[{"lot":onions["id"],"qty":800}]}]}))
        .unwrap();
    assert_eq!(plan["coverage_days"], 14);
    let plans = s
        .command("kitchen-plans", &json!({"day":"2026-10-12"}))
        .unwrap()["plans"]
        .clone();
    assert_eq!(
        (
            plans[0]["coverage_days"].clone(),
            plans[0]["dishes"][0]["standing"].clone()
        ),
        (json!(14), json!("observed"))
    );

    let input = s
        .command(
            "kitchen-draft-input",
            &json!({"day":"2026-10-13","coverage_days":7}),
        )
        .unwrap();
    assert_eq!(
        (
            len(&input["stock"]),
            len(&input["equipment"]),
            len(&input["profile"]),
            input["coverage_days"].clone()
        ),
        (3, 1, 1, json!(7))
    );
    let accepted = s
        .command("kitchen-draft-accept", &json!({"day":"2026-10-13","coverage_days":7,"rule":"local-draft:test","dishes":[
            {"name":"bread","portions":8,"phase":1,"equipment":["dutch oven"],"uses":[{"lot":pictured["id"],"qty":0.5}]}]}))
        .unwrap();
    assert_eq!(accepted["coverage_days"], 7);
    let drafted = &s
        .command("kitchen-plans", &json!({"day":"2026-10-13"}))
        .unwrap()["plans"][0]["dishes"][0];
    assert_eq!(
        (drafted["standing"].as_str(), drafted["derived_by"].as_str()),
        (Some("inferred"), Some("local-draft:test"))
    );
}

#[test]
fn a_lot_takes_a_picture_from_anywhere_on_disk_and_keeps_its_own_copy() {
    let (p, mut s) = service();
    // A folder outside the workspace and its intake, as the window's file choice gives it.
    let elsewhere = p.with_extension("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let chosen = elsewhere.join("onions on the counter.png");
    fs::copy(fixture("kitchen", "test-onions.png"), &chosen).unwrap();
    let rice = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"rice","quantity":900,"unit":"g","day":"2026-10-30"}),
        )
        .unwrap();
    let pictured = s
        .command(
            "kitchen-stock-picture",
            &json!({"lot":rice["id"],"picture":chosen,"day":"2026-10-30"}),
        )
        .unwrap();
    assert_eq!(pictured["has_picture"], true);
    let kept = |s: &mut Service| {
        s.command("kitchen-stock-list", &json!({})).unwrap()["lots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["subject"] == "rice")
            .unwrap()["picture"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let copy = kept(&mut s);
    assert!(
        Path::new(&copy).starts_with(&p),
        "{copy} lies in the workspace"
    );
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&chosen).unwrap());
    // The original is only read; the lot keeps its picture across a reopen.
    assert!(chosen.is_file());
    drop(s);
    let mut s = Service::open(&p).unwrap();
    assert_eq!(kept(&mut s), copy);
    println!("picture kept at {copy}");
}

/// The pictures a lot keeps, as files in the workspace.
fn kept_pictures(p: &Path) -> usize {
    fs::read_dir(p.join("kitchen/pictures")).map_or(0, |d| d.count())
}

#[test]
fn a_file_named_as_a_picture_that_is_not_one_is_refused_on_every_path() {
    let (p, mut s) = service();
    s.command("finance-intake", &json!({})).unwrap();
    // A renamed text file: fourteen bytes, a picture's name.
    let elsewhere = p.with_extension("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let notes = elsewhere.join("notes.png");
    fs::write(&notes, b"not a picture\n").unwrap();
    assert_eq!(fs::metadata(&notes).unwrap().len(), 14);
    // The window's check before it writes.
    let why = s
        .command("kitchen-picture-check", &json!({"picture":notes}))
        .unwrap_err()
        .to_string();
    assert!(why.contains("not a picture"), "{why}");
    // The engine protocol: a new lot with it, and a picture for a lot.
    s.command(
        "kitchen-stock-add",
        &json!({"subject":"oats","quantity":500,"unit":"g","picture":notes,"day":"2026-10-30"}),
    )
    .unwrap_err();
    let oats = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"oats","quantity":500,"unit":"g","day":"2026-10-30"}),
        )
        .unwrap();
    s.command(
        "kitchen-stock-picture",
        &json!({"lot":oats["id"],"picture":notes,"day":"2026-10-30"}),
    )
    .unwrap_err();
    // The intake folder: the same file dropped there.
    fs::copy(&notes, p.join("intake/finance/notes.png")).unwrap();
    s.command(
        "kitchen-stock-picture",
        &json!({"lot":oats["id"],"picture":"intake/finance/notes.png","day":"2026-10-30"}),
    )
    .unwrap_err();
    // Nothing kept: one lot (the one without a picture, not superseded), no copy.
    let lots = s.command("kitchen-stock-list", &json!({})).unwrap()["lots"].clone();
    assert_eq!(lots.as_array().unwrap().len(), 1);
    assert_eq!(lots[0]["id"], oats["id"]);
    assert_eq!(lots[0]["has_picture"], false);
    assert_eq!(kept_pictures(&p), 0);
    // Real pictures still are: png, jpg and webp, by their contents.
    let webp = elsewhere.join("pixel.webp");
    fs::write(
        &webp,
        [
            0x52, 0x49, 0x46, 0x46, 0x1a, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x4c, 0x0d, 0x00, 0x00, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x10, 0x07, 0x10, 0x11,
            0x11, 0x88, 0x88, 0xfe, 0x07, 0x00,
        ],
    )
    .unwrap();
    for (file, kind) in [
        (fixture("kitchen", "test-onions.png"), "png"),
        (fixture("kitchen", "test-flour.jpg"), "jpg"),
        (webp, "webp"),
    ] {
        let checked = s
            .command("kitchen-picture-check", &json!({"picture":file}))
            .unwrap();
        assert_eq!(checked["kind"], kind);
        let lot = s
            .command(
                "kitchen-stock-add",
                &json!({"subject":kind,"quantity":1,"unit":"pieces","picture":file,"day":"2026-10-30"}),
            )
            .unwrap();
        assert_eq!(lot["has_picture"], true, "{kind}");
    }
    assert_eq!(kept_pictures(&p), 3);
    println!("refused 4 of 4 paths; accepted png, jpg, webp");
}

#[test]
fn kitchen_draft_off_calls_nothing() {
    let (_p, mut s) = service();
    for prefs in [
        json!({}),
        json!({"localModel":true}),
        json!({"localModel":false,"kitchenDraft":true}),
    ] {
        s.command("preferences", &prefs).unwrap();
        let started = Instant::now();
        let error = s
            .command(
                "kitchen-draft",
                &json!({"day":"2026-10-13","coverage_days":7}),
            )
            .unwrap_err();
        let took = started.elapsed();
        assert_eq!(error.to_string(), "Drafting is off");
        assert!(took < Duration::from_millis(10), "{took:?}");
    }
}

/// A one-request stand-in for the decision model: returns what it was sent.
fn stub(answer: Value) -> (u16, std::thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut raw = Vec::new();
        let mut buffer = [0_u8; 8192];
        let body = loop {
            let n = stream.read(&mut buffer).unwrap();
            raw.extend_from_slice(&buffer[..n]);
            let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if raw.len() >= split + 4 + length {
                break raw[split + 4..split + 4 + length].to_vec();
            }
        };
        let reply =
            json!({"choices":[{"message":{"role":"assistant","content":answer.to_string()}}]})
                .to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}", reply.len()).unwrap();
        serde_json::from_slice(&body).unwrap()
    });
    (port, handle)
}

#[test]
fn kitchen_draft_on_sends_the_input_and_writes_inferred_dishes() {
    let (p, mut s) = service();
    let lot = s.command("kitchen-stock-add", &json!({"subject":"chickpeas","quantity":900,"unit":"g","expires_on":"2026-10-16","day":"2026-10-12"})).unwrap();
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"pressure cooker","count":1,"day":"2026-10-12"}),
    )
    .unwrap();
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"exclude","value":"pork","day":"2026-10-12"}),
    )
    .unwrap();
    let dishes = json!({"dishes":[{"name":"Chickpeas, preserved lemon, charred onion","portions":10,"phase":1,
        "equipment":["pressure cooker"],"uses":[{"lot":lot["id"],"qty":600}]}]});
    let (port, server) = stub(dishes);
    let script = p.join("llm.sh");
    fs::write(&script, format!("GATEWAY_PORT=1\nMODELS=(\n  \"scorer|decision/test/Test-Decision-Q4.gguf|127.0.0.2:{port}|8192|test-alias|-|cpu|0|--threads 2\"\n)\n")).unwrap();
    s.set_decision_serve(&script);
    s.command(
        "preferences",
        &json!({"localModel":true,"kitchenDraft":true}),
    )
    .unwrap();
    let plan = s
        .command(
            "kitchen-draft",
            &json!({"day":"2026-10-13","coverage_days":7}),
        )
        .unwrap();
    let sent = server.join().unwrap();
    let input: Value =
        serde_json::from_str(sent["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["stock"][0]["subject"], "chickpeas");
    assert_eq!(input["equipment"][0]["name"], "pressure cooker");
    assert_eq!(input["profile"][0]["value"], "pork");
    assert_eq!(input["coverage_days"], 7);
    assert!(
        sent["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Michelin")
    );
    assert_eq!(
        (plan["rule"].as_str(), plan["coverage_days"].clone()),
        (Some("local-draft:Test-Decision-Q4"), json!(7))
    );
    let dish = &s
        .command("kitchen-plans", &json!({"day":"2026-10-13"}))
        .unwrap()["plans"][0]["dishes"][0];
    assert_eq!(
        (dish["standing"].as_str(), dish["derived_by"].as_str()),
        (Some("inferred"), Some("local-draft:Test-Decision-Q4"))
    );

    // A refusal comes back in the model's words; nothing is written.
    let (port, server) = stub(json!({"refusal":"There is not enough stock for seven days"}));
    fs::write(&script, format!("GATEWAY_PORT=1\nMODELS=(\n  \"scorer|decision/test/Test-Decision-Q4.gguf|{port}|8192|test-alias|-|cpu|0|\"\n)\n")).unwrap();
    let error = s
        .command(
            "kitchen-draft",
            &json!({"day":"2026-10-20","coverage_days":7}),
        )
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error.to_string(),
        "There is not enough stock for seven days"
    );
    assert_eq!(
        s.command("kitchen-plans", &json!({"day":"2026-10-20"}))
            .unwrap()["plans"],
        json!([])
    );
    // A decision role on the gateway's port is refused before any request.
    fs::write(
        &script,
        "GATEWAY_PORT=4\nMODELS=(\n  \"scorer|decision/test/T.gguf|4|8192|a|-|cpu|0|\"\n)\n",
    )
    .unwrap();
    assert!(
        s.command(
            "kitchen-draft",
            &json!({"day":"2026-10-21","coverage_days":7})
        )
        .is_err()
    );
}

#[test]
fn now_view_ranks_with_why_and_keeps_private_items_out() {
    let (_p, mut s) = service();
    let pray = s.command("habit-define", &json!({"title":"Pray","direction":"form","baseline":1.0,"bundle":"morning","day":"2026-10-05"})).unwrap();
    s.command("habit-define", &json!({"title":"Drink water","direction":"form","baseline":1.0,"bundle":"morning","day":"2026-10-05"})).unwrap();
    s.command("habit-define", &json!({"title":"Wash face","direction":"form","baseline":1.0,"bundle":"evening","day":"2026-10-05"})).unwrap();
    s.command("kitchen-stock-add", &json!({"subject":"spinach","quantity":300,"unit":"g","expires_on":"2026-10-13","day":"2026-10-10"})).unwrap();

    let view = s
        .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
        .unwrap();
    let items = view["items"].as_array().unwrap();
    let summary: Vec<(String, String, String)> = items
        .iter()
        .map(|i| {
            (
                i["kind"].as_str().unwrap().into(),
                i["title"].as_str().unwrap().into(),
                i["why"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("habit".into(), "Pray".into(), "bundle-open".into()),
            ("habit".into(), "Drink water".into(), "bundle-open".into()),
            ("lot".into(), "spinach".into(), "goes-off".into()),
        ]
    );
    assert!(items.iter().all(|i| {
        !i["sources"].as_array().unwrap().is_empty()
            && i["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["title"].is_string())
    }));
    assert_eq!(view["left_out"], 0);
    let afternoon = s
        .command("now-view", &json!({"day":"2026-10-01","time":"14:00"}))
        .unwrap();
    assert_eq!(
        (afternoon["items"].clone(), afternoon["left_out"].clone()),
        (json!([]), json!(0))
    );

    s.command(
        "preferences",
        &json!({"nowBound":1,"nowOrder":["lot","habit"]}),
    )
    .unwrap();
    let bounded = s
        .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
        .unwrap();
    assert_eq!(
        (
            len(&bounded["items"]),
            bounded["items"][0]["kind"].clone(),
            bounded["left_out"].clone()
        ),
        (1, json!("lot"), json!(2))
    );
    s.command("preferences", &json!({"nowBound":7,"nowOrder":["task","habit","lot"],"nowEveningFrom":"08:00","nowMorningUntil":"07:00"})).unwrap();
    let evening = s
        .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
        .unwrap();
    assert_eq!(evening["items"][0]["title"], "Wash face");
    s.command(
        "preferences",
        &json!({"nowEveningFrom":"18:00","nowMorningUntil":"12:00"}),
    )
    .unwrap();

    // A marked habit leaves the view while private mode is on.
    s.command(
        "ledger-mark-private",
        &json!({"scope":"item","key":pray["id"]}),
    )
    .unwrap();
    assert_eq!(
        len(&s
            .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
            .unwrap()["items"]),
        3
    );
    s.command("preferences", &json!({"privateMode":true}))
        .unwrap();
    let private = s
        .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
        .unwrap();
    assert!(
        private["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["title"] != "Pray")
    );
    assert_eq!(len(&private["items"]), 2);
    s.command("preferences", &json!({"privateModules":["kitchen"]}))
        .unwrap();
    let no_kitchen = s
        .command("now-view", &json!({"day":"2026-10-12","time":"09:00"}))
        .unwrap();
    assert_eq!(
        (
            len(&no_kitchen["items"]),
            no_kitchen["items"][0]["rank"].clone()
        ),
        (1, json!(1))
    );
}

/// Check 17 on a copy of his workspace named by `SOLITON_T71_COPY`: the intake
/// folder is made private, the fixtures dropped there wait, and one imported
/// reads back with its line count.
#[test]
#[ignore = "runs on a workspace copy named in the environment"]
fn finance_intake_on_a_workspace_copy() {
    let p = PathBuf::from(std::env::var("SOLITON_T71_COPY").expect("a workspace copy"));
    let existed = p.join("intake/finance").exists();
    let mut s = Service::open(&p).unwrap();
    assert_eq!(
        s.command("finance-intake", &json!({})).unwrap()["files"],
        json!([])
    );
    let mode = fs::metadata(p.join("intake/finance"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    for (name, _) in FINANCE_FIXTURES {
        fs::copy(
            fixture("finance", name),
            p.join("intake/finance").join(name),
        )
        .unwrap();
    }
    let files = s.command("finance-intake", &json!({})).unwrap()["files"].clone();
    let waiting = files
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["state"] == "waiting")
        .count();
    let id = s
        .command(
            "finance-document-register",
            &json!({"name":"test-purchases-fixed.csv","kind":"csv"}),
        )
        .unwrap()["id"]
        .clone();
    s.command("finance-import", &json!({"document":id,"currency":"CAD"}))
        .unwrap();
    let documents = s.command("finance-documents", &json!({})).unwrap()["documents"].clone();
    println!(
        "T71-COPY existed_before={existed} mode={mode:o} listed={} waiting={waiting} documents={documents}",
        len(&files)
    );
    assert_eq!(
        (
            mode,
            waiting,
            len(&documents),
            documents[0]["lines"].clone()
        ),
        (0o700, 6, 1, json!(3))
    );
}

// Finance reads for T74 (added by T78, which holds service.rs in batch B).
#[test]
fn finance_income_gives_each_stub_with_its_period() {
    let (p, mut s) = service();
    s.command("finance-intake", &json!({})).unwrap();
    let id = drop_in(&mut s, &p, "finance", "test-pay-stub.csv", "csv");
    s.command("finance-import", &json!({"document":id}))
        .unwrap();
    let income = s.command("finance-income", &json!({})).unwrap()["lines"].clone();
    assert_eq!(len(&income), 1);
    let line = &income[0];
    assert_eq!(
        (
            line["day"].as_str(),
            line["employer"].as_str(),
            line["net"].as_str(),
            line["gross"].as_str()
        ),
        (
            Some("2026-10-09"),
            Some("TEST Employer Ltd"),
            Some("1850.00"),
            Some("2400.00")
        )
    );
    assert_eq!(
        (
            line["period_start"].as_str(),
            line["period_end"].as_str(),
            line["currency"].as_str()
        ),
        (Some("2026-09-26"), Some("2026-10-09"), Some("CAD"))
    );
    assert!(line["id"].is_string() && line["source"].is_string());
}

#[test]
fn finance_lines_carry_their_word_item_and_stock_mark() {
    let (p, mut s) = service();
    s.command("finance-intake", &json!({})).unwrap();
    let id = drop_in(&mut s, &p, "finance", "test-purchases-fixed.csv", "csv");
    s.command("finance-import", &json!({"document":id,"currency":"CAD"}))
        .unwrap();
    let lines = s.command("finance-lines", &json!({})).unwrap()["lines"].clone();
    let oat = lines
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["item"] == "Oat milk")
        .unwrap()
        .clone();
    assert_eq!(
        (oat["category_word"].as_str(), oat["in_stock"].as_bool()),
        (Some("Groceries"), Some(false))
    );
    s.command(
        "kitchen-lot-from-purchase",
        &json!({"line":oat["id"],"quantity":2,"unit":"l","day":"2026-10-10"}),
    )
    .unwrap();
    let lines = s.command("finance-lines", &json!({})).unwrap()["lines"].clone();
    let marks: Vec<(String, bool)> = lines
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["item"].as_str().unwrap().to_string(),
                l["in_stock"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        marks.iter().filter(|(_, stocked)| *stocked).count(),
        1,
        "{marks:?}"
    );
    assert!(marks.contains(&("Oat milk".into(), true)), "{marks:?}");
}

#[test]
fn a_lot_from_a_purchase_takes_the_item_name_not_the_merchant() {
    let (p, mut s) = service();
    s.command("finance-intake", &json!({})).unwrap();
    let id = drop_in(&mut s, &p, "finance", "test-purchases-fixed.csv", "csv");
    s.command("finance-import", &json!({"document":id,"currency":"CAD"}))
        .unwrap();
    let line = s.command("kitchen-from-purchases", &json!({})).unwrap()["lines"][0]["line"].clone();
    let lot = s
        .command(
            "kitchen-lot-from-purchase",
            &json!({"line":line,"quantity":2,"unit":"l","day":"2026-10-10"}),
        )
        .unwrap();
    assert_eq!(lot["subject"], "Oat milk", "{lot}");
}
