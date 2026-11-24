use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::*;
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    let p = std::env::temp_dir().join(ObjectId::mint("soliton-test").unwrap().to_string());
    fs::create_dir(&p).unwrap();
    p
}
/// A statement placed in the finance intake and registered: its document id.
fn intake_statement(s: &mut Service, p: &std::path::Path, name: &str, text: &str) -> Value {
    s.command("finance-intake", &json!({})).unwrap();
    fs::write(p.join("intake/finance").join(name), text).unwrap();
    s.command(
        "finance-document-register",
        &json!({"name":name,"kind":"csv"}),
    )
    .unwrap()["id"]
        .clone()
}
fn tasks_this_week(s: &mut Service) -> Value {
    let a = s
        .command("ledger-question", &json!({"name":"morning-bundle-weeks"}))
        .unwrap();
    assert_eq!(
        a["columns"],
        json!([
            "week",
            "morning_bundle",
            "tasks_finished",
            "food_spend_gbp",
            "bundle_days_kept",
            "bundle_days_recorded"
        ])
    );
    a["rows"]
        .as_array()
        .unwrap()
        .last()
        .map(|r| r[2].clone())
        .unwrap_or(json!(0))
}

#[test]
fn ledger_task_completion_writes_check_in_and_reopen_supersedes_it() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let task = s
        .command("save", &json!({"kind":"task","title":"Ledger check-in"}))
        .unwrap();
    // No ledger: completing writes nothing and creates no file.
    let done = s
        .command("complete", &json!({"id":task["id"],"hash":task["hash"]}))
        .unwrap();
    assert!(!p.join("ledger.sqlite").exists());
    assert!(
        s.command("ledger-question", &json!({"name":"morning-bundle-weeks"}))
            .is_err()
    );
    let reopened = s
        .command("complete", &json!({"id":task["id"],"hash":done["hash"]}))
        .unwrap();

    let migrated = s.command("ledger-migrate", &json!({})).unwrap();
    assert_eq!(migrated["kinds"], 9);
    assert!(s.command("ledger-migrate", &json!({})).is_err());
    let done = s
        .command(
            "complete",
            &json!({"id":task["id"],"hash":reopened["hash"]}),
        )
        .unwrap();
    assert_eq!(done["status"], "completed");
    assert_eq!(tasks_this_week(&mut s), json!(1));
    s.command("complete", &json!({"id":task["id"],"hash":done["hash"]}))
        .unwrap();
    assert_eq!(tasks_this_week(&mut s), json!(0));
    for name in ["sleep-baseline-days", "food-spend-weeks"] {
        assert!(s.command("ledger-question", &json!({"name":name})).is_ok());
    }
    assert!(
        s.command("ledger-question", &json!({"name":"unknown"}))
            .is_err()
    );
}

#[test]
fn ledger_goal_mark_expands_to_goal_items() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    let goal = s
        .command("save", &json!({"kind":"goal","title":"Private goal"}))
        .unwrap();
    let task = s
        .command(
            "save",
            &json!({"kind":"task","title":"Goal item","goal_id":goal["id"]}),
        )
        .unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    s.command("complete", &json!({"id":task["id"],"hash":task["hash"]}))
        .unwrap();
    let marked = s
        .command(
            "ledger-mark-private",
            &json!({"scope":"goal","key":goal["id"]}),
        )
        .unwrap();
    assert_eq!(marked["items"], 1);
    let on = s
        .command(
            "ledger-question",
            &json!({"name":"morning-bundle-weeks","hide_all":true}),
        )
        .unwrap();
    assert_eq!(on["rows"], json!([]));
    assert_eq!(tasks_this_week(&mut s), json!(1));
    assert!(
        s.command(
            "ledger-mark-private",
            &json!({"scope":"goal","key":task["id"]})
        )
        .is_err()
    );
}

#[test]
fn habit_commands_define_order_check_in_correct_and_heat_map_with_hide_all() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    assert!(
        s.command(
            "habit-define",
            &json!({"title":"Water","direction":"form","baseline":1.0,"bundle":"morning"})
        )
        .is_err()
    );
    s.command("ledger-migrate", &json!({})).unwrap();
    let mut ids = Vec::new();
    for title in ["Water", "Stretch", "Plan the day"] {
        let h = s
            .command("habit-define", &json!({"title":title,"direction":"form","baseline":1.0,"bundle":"morning","day":"2026-09-28"}))
            .unwrap();
        ids.push(h["id"].as_str().unwrap().to_owned());
    }
    let phone = s
        .command(
            "habit-define",
            &json!({"title":"Phone in bed","direction":"break","baseline":2.0,"day":"2026-09-28"}),
        )
        .unwrap();
    let edited = s
        .command("habit-define", &json!({"id":phone["id"],"title":"Phone in bed","direction":"break","baseline":1.0,"day":"2026-09-28"}))
        .unwrap();
    assert_eq!(edited["baseline"], 1.0);
    let reversed: Vec<&String> = ids.iter().rev().collect();
    s.command(
        "habit-bundle-order",
        &json!({"bundle":"morning","order":reversed}),
    )
    .unwrap();
    let list = s.command("habit-list", &json!({})).unwrap();
    assert_eq!(list["bundles"]["morning"], json!(reversed));
    assert_eq!(list["habits"].as_array().unwrap().len(), 4);
    let c = s
        .command(
            "habit-check-in",
            &json!({"id":ids[0],"day":"2026-09-28","done":false}),
        )
        .unwrap();
    assert!(
        s.command(
            "habit-check-in",
            &json!({"id":ids[0],"day":"2026-09-28","done":true})
        )
        .is_err()
    );
    s.command(
        "habit-correct",
        &json!({"check_in":c["check_in"],"done":true}),
    )
    .unwrap();
    s.command(
        "habit-check-in",
        &json!({"id":phone["id"],"day":"2026-09-28","count":0}),
    )
    .unwrap();
    assert!(
        s.command(
            "habit-check-in",
            &json!({"id":phone["id"],"day":"2026-09-29","done":true})
        )
        .is_err()
    );
    let map = s
        .command(
            "habit-heat-map",
            &json!({"from":"2026-09-28","through":"2026-09-30"}),
        )
        .unwrap();
    let habits = map["habits"].as_array().unwrap();
    assert_eq!(habits.len(), 4);
    assert_eq!(
        habits[2]["days"][0],
        json!({"day":"2026-09-28","value":1.0,"against":"at"})
    );
    assert_eq!(habits[3]["days"][0]["against"], "below");
    s.command(
        "ledger-mark-private",
        &json!({"scope":"item","key":phone["id"]}),
    )
    .unwrap();
    let hidden = s
        .command(
            "habit-heat-map",
            &json!({"from":"2026-09-28","through":"2026-09-30","hide_all":true}),
        )
        .unwrap();
    assert_eq!(hidden["habits"].as_array().unwrap().len(), 3);
    assert_eq!(
        s.command("habit-list", &json!({"hide_all":true})).unwrap()["habits"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn kitchen_and_finance_commands_plan_import_once_and_hide_all() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    assert!(
        s.command(
            "kitchen-stock-add",
            &json!({"subject":"spinach","quantity":400,"unit":"g"})
        )
        .is_err()
    );
    let migrated = s.command("ledger-migrate", &json!({})).unwrap();
    assert_eq!(migrated["schema_version"], 2);
    let spinach = s
        .command("kitchen-stock-add", &json!({"subject":"spinach","category":"food:produce","quantity":400,"unit":"g","expires_on":"2026-10-06","location":"fridge","day":"2026-10-04"}))
        .unwrap();
    let ginger = s
        .command("kitchen-stock-add", &json!({"subject":"ginger","category":"food:produce","quantity":150,"unit":"g","expires_on":"2026-10-09","day":"2026-10-04"}))
        .unwrap();
    assert!(
        s.command(
            "kitchen-stock-add",
            &json!({"subject":"leek","quantity":1,"unit":"item","source":"photo"})
        )
        .is_err()
    );
    let used = s
        .command(
            "kitchen-stock-use",
            &json!({"lot":spinach["id"],"amount":50,"day":"2026-10-05"}),
        )
        .unwrap();
    assert_eq!(
        (used["quantity"].as_f64(), &used["supersedes"]),
        (Some(350.0), &spinach["id"])
    );
    let counted = s
        .command(
            "kitchen-stock-adjust",
            &json!({"lot":used["id"],"quantity":340,"day":"2026-10-05"}),
        )
        .unwrap();
    let ginger = s
        .command(
            "kitchen-stock-expiry",
            &json!({"lot":ginger["id"],"expires_on":"2026-10-07","day":"2026-10-05"}),
        )
        .unwrap();
    assert_eq!(ginger["expires_on"], "2026-10-07");
    assert_eq!(
        s.command("kitchen-stock-list", &json!({})).unwrap()["lots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"dutch oven","count":1,"capacity":"6.5 l","day":"2026-10-04"}),
    )
    .unwrap();
    assert_eq!(
        s.command("kitchen-equipment-list", &json!({})).unwrap()["equipment"][0]["name"],
        "dutch oven"
    );
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"session_hours","quantity":5,"unit":"h","day":"2026-10-04"}),
    )
    .unwrap();
    assert_eq!(
        s.command("kitchen-profile-list", &json!({})).unwrap()["profile"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let plan = s
        .command("kitchen-batch-plan", &json!({"day":"2026-10-05","horizon_days":3,"dishes":[
            {"name":"spinach ginger soup","portions":8,"phase":1,"equipment":["dutch oven"],"uses":[{"lot":counted["id"],"qty":300},{"lot":ginger["id"],"qty":60}]}]}))
        .unwrap();
    assert_eq!(plan["plan_id"], "plan_2026-10-05");
    assert_eq!(plan["goes_off"]["rows"].as_array().unwrap().len(), 2);
    let q5 = s
        .command("kitchen-goes-off", &json!({"today":"2026-10-05","days":3}))
        .unwrap();
    assert_eq!(
        q5["rows"][0],
        json!(["spinach", 340.0, "g", "2026-10-06", 300.0, 40.0])
    );

    let csv = intake_statement(
        &mut s,
        &p,
        "sample-statement.csv",
        "Date,Description,Amount,Currency\n2026-10-01,Grocer,-3.50,GBP\n2026-10-02,Refund,2.00,GBP\n",
    );
    assert!(
        s.command(
            "finance-statement-import",
            &json!({"bank":"sample","document":csv})
        )
        .is_err()
    );
    s.command("finance-statement-mapping", &json!({"bank":"sample","date":"Date","date_format":"YYYY-MM-DD","description":"Description","amount":"Amount","currency":"Currency","debit_negative":true})).unwrap();
    let first = s
        .command(
            "finance-statement-import",
            &json!({"bank":"sample","document":csv}),
        )
        .unwrap();
    assert_eq!(
        (first["added"].as_u64(), first["credits"].as_u64()),
        (Some(1), Some(1))
    );
    let again = s
        .command(
            "finance-statement-import",
            &json!({"bank":"sample","document":csv}),
        )
        .unwrap();
    assert_eq!(
        (again["added"].as_u64(), again["duplicates"].as_u64()),
        (Some(0), Some(1))
    );
    let q6 = s
        .command(
            "ledger-question",
            &json!({"name":"spend-by-category-weeks"}),
        )
        .unwrap();
    assert_eq!(
        q6["rows"],
        json!([["2026-09-28", "uncategorised", "3.50", "GBP", 1]])
    );
    let hidden = s
        .command(
            "ledger-question",
            &json!({"name":"spend-by-category-weeks","hide_all":true}),
        )
        .unwrap();
    assert_eq!(hidden["rows"], json!([]));
    s.command(
        "ledger-mark-private",
        &json!({"scope":"module","key":"kitchen"}),
    )
    .unwrap();
    assert_eq!(
        s.command(
            "kitchen-goes-off",
            &json!({"today":"2026-10-05","days":3,"hide_all":true})
        )
        .unwrap()["rows"],
        json!([])
    );
    assert_eq!(
        s.command("kitchen-stock-list", &json!({"hide_all":true}))
            .unwrap()["lots"],
        json!([])
    );
}

fn private_read(command: &str, args: Value, field: &str) {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    s.command("habit-define", &json!({"title":"Private habit","direction":"form","baseline":1.0,"bundle":"morning","day":"2026-10-05"})).unwrap();
    s.command("kitchen-stock-add", &json!({"subject":"spinach","quantity":400,"unit":"g","expires_on":"2026-10-06","day":"2026-10-05"})).unwrap();
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"oven","count":1,"day":"2026-10-05"}),
    )
    .unwrap();
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"hours","quantity":5,"unit":"h","day":"2026-10-05"}),
    )
    .unwrap();
    let csv = intake_statement(
        &mut s,
        &p,
        "private-statement.csv",
        "Date,Description,Amount,Currency\n2026-10-05,Grocer,-3.50,GBP\n2026-10-06,Market,-2.00,GBP\n",
    );
    s.command("finance-statement-mapping", &json!({"bank":"private-test","date":"Date","date_format":"YYYY-MM-DD","description":"Description","amount":"Amount","currency":"Currency","debit_negative":true})).unwrap();
    assert_eq!(
        s.command(
            "finance-statement-import",
            &json!({"bank":"private-test","document":csv})
        )
        .unwrap()["added"],
        2
    );
    let baseline = s.command(command, &args).unwrap();
    assert!(!baseline[field].as_array().unwrap().is_empty());
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["habits","kitchen","finance","health"]}),
    )
    .unwrap();
    let result = s.command(command, &args).unwrap();
    assert_eq!(result["hide_all"], true);
    assert_eq!(result[field], json!([]));
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(s.command(command, &args).unwrap()["hide_all"], false);
}
#[test]
fn private_mode_ledger_question() {
    private_read(
        "ledger-question",
        json!({"name":"spend-by-category-weeks","hide_all":false}),
        "rows",
    );
}
fn habit_service() -> (Service, Value) {
    let mut s = Service::open(&root()).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    let h = s
        .command("habit-define", &json!({"title":"Water","direction":"form","baseline":1.0,"bundle":"morning","day":"2026-10-05"}))
        .unwrap();
    (s, h)
}

fn day_check_in(s: &mut Service, day: &str) -> Value {
    s.command("habit-list", &json!({"day":day})).unwrap()["habits"][0]["check_in"].clone()
}

#[test]
fn habit_list_returns_the_days_check_in_id() {
    let (mut s, h) = habit_service();
    assert_eq!(
        s.command("habit-list", &json!({"day":"2026-10-05"}))
            .unwrap()["day"],
        "2026-10-05"
    );
    assert_eq!(day_check_in(&mut s, "2026-10-05"), Value::Null);
    let c = s
        .command(
            "habit-check-in",
            &json!({"id":h["id"],"day":"2026-10-05","done":true}),
        )
        .unwrap();
    assert_eq!(day_check_in(&mut s, "2026-10-05"), c["check_in"]);
    let fixed = s
        .command(
            "habit-correct",
            &json!({"check_in":c["check_in"],"done":false}),
        )
        .unwrap();
    assert_eq!(day_check_in(&mut s, "2026-10-05"), fixed["check_in"]);
    assert_eq!(day_check_in(&mut s, "2026-10-06"), Value::Null);
}

#[test]
fn habit_retract_then_re_enter_on_the_same_day() {
    let (mut s, h) = habit_service();
    let c = s
        .command(
            "habit-check-in",
            &json!({"id":h["id"],"day":"2026-10-05","done":true}),
        )
        .unwrap();
    let r = s
        .command("habit-retract", &json!({"check_in":c["check_in"]}))
        .unwrap();
    assert_eq!(r["retracts"], c["check_in"]);
    assert_eq!(day_check_in(&mut s, "2026-10-05"), Value::Null);
    let map = s
        .command(
            "habit-heat-map",
            &json!({"from":"2026-10-05","through":"2026-10-05"}),
        )
        .unwrap();
    assert_eq!(map["habits"][0]["days"][0]["against"], "no record");
    let q1 = s
        .command("ledger-question", &json!({"name":"morning-bundle-weeks"}))
        .unwrap();
    assert!(
        !q1.to_string().contains("\"bundle_days_recorded\":1"),
        "{q1}"
    );
    let again = s
        .command(
            "habit-check-in",
            &json!({"id":h["id"],"day":"2026-10-05","done":false}),
        )
        .unwrap();
    assert_eq!(day_check_in(&mut s, "2026-10-05"), again["check_in"]);
    let map = s
        .command(
            "habit-heat-map",
            &json!({"from":"2026-10-05","through":"2026-10-05"}),
        )
        .unwrap();
    assert_eq!(map["habits"][0]["days"][0]["against"], "below");
}

#[test]
fn habit_retract_works_under_private_mode_and_stays_hidden() {
    let (mut s, h) = habit_service();
    let c = s
        .command(
            "habit-check-in",
            &json!({"id":h["id"],"day":"2026-10-05","done":true}),
        )
        .unwrap();
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["habits"]}),
    )
    .unwrap();
    s.command("habit-retract", &json!({"check_in":c["check_in"]}))
        .unwrap();
    assert_eq!(
        s.command("habit-list", &json!({"day":"2026-10-05"}))
            .unwrap()["habits"],
        json!([])
    );
    let map = s
        .command(
            "habit-heat-map",
            &json!({"from":"2026-10-05","through":"2026-10-05"}),
        )
        .unwrap();
    assert_eq!(map["habits"], json!([]));
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(day_check_in(&mut s, "2026-10-05"), Value::Null);
}

#[test]
fn habit_retract_of_a_superseded_row_is_refused_plainly() {
    let (mut s, h) = habit_service();
    let c = s
        .command(
            "habit-check-in",
            &json!({"id":h["id"],"day":"2026-10-05","done":true}),
        )
        .unwrap();
    s.command(
        "habit-correct",
        &json!({"check_in":c["check_in"],"done":false}),
    )
    .unwrap();
    let error = s
        .command("habit-retract", &json!({"check_in":c["check_in"]}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("already been changed"), "{error}");
    assert!(!error.contains(c["check_in"].as_str().unwrap()), "{error}");
}

#[test]
fn private_mode_habit_list() {
    private_read("habit-list", json!({}), "habits");
}
#[test]
fn private_mode_habit_heat_map() {
    private_read(
        "habit-heat-map",
        json!({"from":"2026-10-05","through":"2026-10-06"}),
        "habits",
    );
}
#[test]
fn private_mode_stock_list() {
    private_read("kitchen-stock-list", json!({}), "lots");
}
#[test]
fn private_mode_equipment_list() {
    private_read("kitchen-equipment-list", json!({}), "equipment");
}
#[test]
fn private_mode_profile_list() {
    private_read("kitchen-profile-list", json!({}), "profile");
}
#[test]
fn private_mode_goes_off() {
    private_read(
        "kitchen-goes-off",
        json!({"today":"2026-10-05","days":3}),
        "rows",
    );
}

#[test]
fn private_mode_batch_plan_hides_private_lot_and_profile_then_whole_module() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    for (subject, private) in [("secret spinach", true), ("public ginger", false)] {
        s.command("kitchen-stock-add", &json!({"subject":subject,"category":if private {"food:private"} else {"food:public"},"quantity":400,"unit":"g","expires_on":"2026-10-08","day":"2026-10-05"})).unwrap();
        if private {
            s.command(
                "ledger-mark-private",
                &json!({"scope":"category","key":"food:private"}),
            )
            .unwrap();
        }
    }
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"portions_per_week","quantity":17,"unit":"portion","day":"2026-10-05"}),
    )
    .unwrap();
    s.command(
        "ledger-mark-private",
        &json!({"scope":"category","key":"kitchen:profile"}),
    )
    .unwrap();
    let plan_args =
        |day: &str| json!({"day":day,"horizon_days":3,"dishes":[{"name":"soup","portions":2}]});
    let baseline = s
        .command("kitchen-batch-plan", &plan_args("2026-10-05"))
        .unwrap();
    assert_eq!(baseline["constraints"].as_array().unwrap().len(), 1);
    assert_eq!(baseline["goes_off"]["rows"].as_array().unwrap().len(), 2);
    assert!(baseline["notes"].to_string().contains("secret spinach"));
    s.command("preferences", &json!({"privateMode":true}))
        .unwrap();
    let hidden = s
        .command("kitchen-batch-plan", &plan_args("2026-10-06"))
        .unwrap();
    assert_eq!(hidden["constraints"], json!([]));
    assert_eq!(hidden["goes_off"]["rows"].as_array().unwrap().len(), 1);
    assert!(!hidden.to_string().contains("secret spinach"));
    assert!(!hidden["notes"].to_string().contains("17"));
    assert!(hidden["notes"].to_string().contains("public ginger"));
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["kitchen"]}),
    )
    .unwrap();
    let module_hidden = s
        .command("kitchen-batch-plan", &plan_args("2026-10-07"))
        .unwrap();
    assert_eq!(module_hidden["constraints"], json!([]));
    assert_eq!(module_hidden["notes"], json!([]));
    assert_eq!(module_hidden["goes_off"]["rows"], json!([]));
}

#[test]
fn habit_mark_is_listed_and_lifted_and_private_mode_follows_it() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    let kept = s
        .command(
            "habit-define",
            &json!({"title":"Water","direction":"form","baseline":1.0,"day":"2026-09-28"}),
        )
        .unwrap();
    let secret = s
        .command(
            "habit-define",
            &json!({"title":"Secret","direction":"form","baseline":1.0,"day":"2026-09-28"}),
        )
        .unwrap();
    s.command(
        "habit-check-in",
        &json!({"id":secret["id"],"day":"2026-09-28","done":true}),
    )
    .unwrap();
    let private_of = |list: &Value, id: &Value| -> Option<Value> {
        list["habits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| &h["id"] == id)
            .map(|h| h["private"].clone())
    };
    let day = json!({"day":"2026-09-28"});
    let map_args = json!({"from":"2026-09-28","through":"2026-09-28"});
    let list = s.command("habit-list", &day).unwrap();
    assert_eq!(
        (
            private_of(&list, &kept["id"]),
            private_of(&list, &secret["id"])
        ),
        (Some(json!(false)), Some(json!(false)))
    );
    s.command(
        "ledger-mark-private",
        &json!({"scope":"item","key":secret["id"]}),
    )
    .unwrap();
    let list = s.command("habit-list", &day).unwrap();
    assert_eq!(private_of(&list, &secret["id"]), Some(json!(true)));
    assert_eq!(private_of(&list, &kept["id"]), Some(json!(false)));
    s.command("preferences", &json!({"privateMode":true}))
        .unwrap();
    let list = s.command("habit-list", &day).unwrap();
    assert_eq!(
        (
            private_of(&list, &secret["id"]),
            private_of(&list, &kept["id"]).is_some()
        ),
        (None, true)
    );
    assert!(
        !s.command("habit-heat-map", &map_args)
            .unwrap()
            .to_string()
            .contains(secret["id"].as_str().unwrap())
    );
    assert!(
        s.command(
            "ledger-unmark-private",
            &json!({"scope":"goal","key":secret["id"]})
        )
        .is_err()
    );
    let lifted = s
        .command(
            "ledger-unmark-private",
            &json!({"scope":"item","key":secret["id"]}),
        )
        .unwrap();
    assert_eq!(lifted["key"], secret["id"]);
    let list = s.command("habit-list", &day).unwrap();
    assert_eq!(private_of(&list, &secret["id"]), Some(json!(false)));
    assert!(
        list["habits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["id"] == secret["id"] && h["check_in"].is_string()),
        "its check-in returns"
    );
    let map = s.command("habit-heat-map", &map_args).unwrap();
    let row = map["habits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"] == secret["id"])
        .cloned()
        .unwrap();
    assert_eq!(row["days"][0]["value"], 1.0);
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(
        private_of(&s.command("habit-list", &day).unwrap(), &secret["id"]),
        Some(json!(false))
    );
}

#[test]
fn finance_lines_read_after_import_and_recategorise_moves_q6() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    let csv = intake_statement(
        &mut s,
        &p,
        "lines-statement.csv",
        "Date,Description,Amount,Currency\n2026-10-01,Grocer,-3.50,GBP\n2026-10-02,Bookshop,-12.00,GBP\n2026-10-06,Grocer,-4.25,GBP\n2026-10-03,Refund,2.00,GBP\n",
    );
    s.command("finance-statement-mapping", &json!({"bank":"lines","date":"Date","date_format":"YYYY-MM-DD","description":"Description","amount":"Amount","currency":"Currency","debit_negative":true})).unwrap();
    assert_eq!(
        s.command(
            "finance-statement-import",
            &json!({"bank":"lines","document":csv})
        )
        .unwrap()["added"],
        3
    );
    let all = s.command("finance-lines", &json!({})).unwrap();
    let lines = all["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        (
            &lines[0]["date"],
            &lines[0]["merchant"],
            &lines[0]["amount"],
            &lines[0]["currency"],
            &lines[0]["category"]
        ),
        (
            &json!("2026-10-01"),
            &json!("Grocer"),
            &json!("3.50"),
            &json!("GBP"),
            &json!("uncategorised")
        )
    );
    assert_eq!(
        lines[0]["source"],
        format!("document:{}#1", csv.as_str().unwrap())
    );
    let week = s
        .command("finance-lines", &json!({"week":"2026-10-04"}))
        .unwrap();
    assert_eq!(week["lines"].as_array().unwrap().len(), 2);
    let book = lines[1]["id"].clone();
    let moved = s
        .command(
            "finance-recategorise",
            &json!({"line":book,"category":"leisure:books"}),
        )
        .unwrap();
    assert_eq!(moved["supersedes"], book);
    assert!(
        s.command(
            "finance-recategorise",
            &json!({"line":book,"category":"other"})
        )
        .is_err()
    );
    assert!(
        s.command(
            "finance-recategorise",
            &json!({"line":moved["id"],"category":" "})
        )
        .is_err()
    );
    let q6 = s
        .command(
            "ledger-question",
            &json!({"name":"spend-by-category-weeks"}),
        )
        .unwrap();
    assert_eq!(
        q6["rows"],
        json!([
            ["2026-09-28", "leisure:books", "12.00", "GBP", 1],
            ["2026-09-28", "uncategorised", "3.50", "GBP", 1],
            ["2026-10-05", "uncategorised", "4.25", "GBP", 1]
        ])
    );
    let books = s
        .command("finance-lines", &json!({"category":"leisure:books"}))
        .unwrap();
    assert_eq!(books["lines"].as_array().unwrap().len(), 1);
    assert_eq!(books["lines"][0]["id"], moved["id"]);
    // The recategorised line still counts as already in on a second import.
    let again = s
        .command(
            "finance-statement-import",
            &json!({"bank":"lines","document":csv}),
        )
        .unwrap();
    assert_eq!(
        (again["added"].as_u64(), again["duplicates"].as_u64()),
        (Some(0), Some(3))
    );
    assert_eq!(
        s.command("finance-lines", &json!({"hide_all":true}))
            .unwrap()["lines"],
        json!([])
    );
}

#[test]
fn kitchen_plans_read_back_the_plan_as_written() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    assert_eq!(
        s.command("kitchen-plans", &json!({})).unwrap()["plans"],
        json!([])
    );
    let leek = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"leek","quantity":4,"unit":"item","day":"2026-10-05"}),
        )
        .unwrap();
    let rice = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"rice","quantity":1000,"unit":"g","day":"2026-10-05"}),
        )
        .unwrap();
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"dutch oven","count":1,"day":"2026-10-05"}),
    )
    .unwrap();
    s.command(
        "kitchen-equipment-set",
        &json!({"name":"board","count":2,"day":"2026-10-05"}),
    )
    .unwrap();
    let dishes = json!([
        {"name":"leek risotto","portions":4,"phase":1,"equipment":["dutch oven"],"uses":[{"lot":leek["id"],"qty":2.0},{"lot":rice["id"],"qty":320.0}]},
        {"name":"chop leeks","portions":1,"phase":0,"equipment":["board"],"uses":[]}]);
    let written = s
        .command(
            "kitchen-batch-plan",
            &json!({"day":"2026-10-05","dishes":dishes}),
        )
        .unwrap();
    s.command("kitchen-batch-plan", &json!({"day":"2026-10-06","dishes":[{"name":"plain rice","portions":2,"uses":[{"lot":rice["id"],"qty":200.0}]}]})).unwrap();
    let all = s.command("kitchen-plans", &json!({})).unwrap();
    assert_eq!(all["plans"].as_array().unwrap().len(), 2);
    let read = s
        .command("kitchen-plans", &json!({"day":"2026-10-05"}))
        .unwrap();
    let plan = &read["plans"][0];
    assert_eq!(
        (&plan["plan_id"], &plan["day"], plan["portions"].as_f64()),
        (&written["plan_id"], &json!("2026-10-05"), Some(5.0))
    );
    let ids: Vec<&Value> = plan["dishes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["id"])
        .collect();
    let mut wrote: Vec<&Value> = written["rows"].as_array().unwrap().iter().collect();
    wrote.reverse(); // phase order: the mise en place first
    assert_eq!(ids, wrote);
    let strip = |d: &Value| json!({"name":d["name"],"portions":d["portions"].as_f64(),"phase":d["phase"],"equipment":d["equipment"],"uses":d["uses"]});
    let back: Vec<Value> = plan["dishes"]
        .as_array()
        .unwrap()
        .iter()
        .map(strip)
        .collect();
    let mut given: Vec<Value> = dishes.as_array().unwrap().iter().map(strip).collect();
    given.reverse();
    assert_eq!(back, given);
    assert_eq!(
        s.command("kitchen-plans", &json!({"day":"2026-10-07"}))
            .unwrap()["plans"],
        json!([])
    );
}
#[test]
fn private_mode_finance_lines() {
    private_read("finance-lines", json!({}), "lines");
}
#[test]
fn private_mode_kitchen_plans() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    s.command(
        "kitchen-batch-plan",
        &json!({"day":"2026-10-05","dishes":[{"name":"soup","portions":2}]}),
    )
    .unwrap();
    assert_eq!(
        s.command("kitchen-plans", &json!({})).unwrap()["plans"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["kitchen"]}),
    )
    .unwrap();
    let hidden = s.command("kitchen-plans", &json!({})).unwrap();
    assert_eq!(
        (&hidden["hide_all"], &hidden["plans"]),
        (&json!(true), &json!([]))
    );
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(
        s.command("kitchen-plans", &json!({})).unwrap()["plans"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn kitchen_cook_writes_a_meal_and_meals_read_honours_private_mode() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    let rice = s
        .command(
            "kitchen-stock-add",
            &json!({"subject":"rice","quantity":1000,"unit":"g","day":"2031-03-03"}),
        )
        .unwrap();
    s.command("kitchen-batch-plan", &json!({"day":"2031-03-03","dishes":[{"name":"plain rice","portions":2,"uses":[{"lot":rice["id"],"qty":200.0}]}]})).unwrap();
    let dish = s.command("kitchen-plans", &json!({})).unwrap()["plans"][0]["dishes"][0].clone();
    assert_eq!(dish["meal"], Value::Null);
    let cooked = s
        .command(
            "kitchen-cook",
            &json!({"plan":"plan_2031-03-03","dish":dish["id"],"day":"2031-03-04"}),
        )
        .unwrap();
    assert_eq!(cooked["lots"][0]["supersedes"], rice["id"]);
    assert_eq!(
        s.command("kitchen-stock-list", &json!({})).unwrap()["lots"][0]["quantity"].as_f64(),
        Some(800.0)
    );
    assert_eq!(
        s.command("kitchen-plans", &json!({})).unwrap()["plans"][0]["dishes"][0]["meal"],
        cooked["meal"]
    );
    let meals = s
        .command("kitchen-meals", &json!({"day":"2031-03-04"}))
        .unwrap();
    assert_eq!(
        meals["meals"],
        json!([{"id":cooked["meal"],"day":"2031-03-04","name":"plain rice","portions":2.0,"category":"food:home","plan_id":"plan_2031-03-03"}])
    );
    assert!(
        s.command(
            "kitchen-cook",
            &json!({"plan":"plan_2031-03-03","dish":"plain rice"})
        )
        .is_err()
    );
    assert_eq!(
        s.command("kitchen-meals", &json!({"hide_all":true}))
            .unwrap()["meals"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    s.command(
        "preferences",
        &json!({"privateMode":true,"privateModules":["kitchen"]}),
    )
    .unwrap();
    let hidden = s.command("kitchen-meals", &json!({})).unwrap();
    assert_eq!(
        (&hidden["hide_all"], &hidden["meals"]),
        (&json!(true), &json!([]))
    );
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(
        s.command("kitchen-meals", &json!({})).unwrap()["meals"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn settings_read_reports_the_ledger_finance_mark_and_never_stores_it() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    assert_eq!(
        s.command("preferences", &json!({})).unwrap()["ledgerPrivateModules"],
        json!([])
    );
    s.command("ledger-migrate", &json!({})).unwrap();
    let read = s.command("preferences", &json!({})).unwrap();
    assert_eq!(read["ledgerPrivateModules"], json!(["finance", "health"]));
    assert_eq!(
        s.command("state", &json!({})).unwrap()["ui"]["ledgerPrivateModules"],
        json!(["finance", "health"])
    );
    // A save carrying the key back neither stores it nor writes a mark.
    s.command("preferences", &json!({"ledgerPrivateModules":["kitchen"]}))
        .unwrap();
    assert!(
        s.preferences()
            .unwrap()
            .get("ledgerPrivateModules")
            .is_none()
    );
    assert_eq!(
        s.command("preferences", &json!({})).unwrap()["ledgerPrivateModules"],
        json!(["finance", "health"])
    );
    s.command(
        "ledger-unmark-private",
        &json!({"scope":"module","key":"finance"}),
    )
    .unwrap();
    assert_eq!(
        s.command("preferences", &json!({})).unwrap()["ledgerPrivateModules"],
        json!(["health"])
    );
}
#[test]
fn finance_banks_and_statement_header_reads() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    assert_eq!(
        s.command("finance-banks", &json!({})).unwrap()["banks"],
        json!([])
    );
    let csv = p.join("sample-2031.csv");
    fs::write(
        &csv,
        "Date,Description,Amount,Currency\n2031-03-03,Grocer,-3.50,GBP\n",
    )
    .unwrap();
    assert_eq!(
        s.command("finance-statement-header", &json!({"path":csv}))
            .unwrap()["columns"],
        json!(["Date", "Description", "Amount", "Currency"])
    );
    assert!(
        s.command(
            "finance-statement-header",
            &json!({"path":p.join("missing.csv")})
        )
        .is_err()
    );
    s.command("finance-statement-mapping", &json!({"bank":"sample","date":"Date","date_format":"YYYY-MM-DD","description":"Description","amount":"Amount","currency":"Currency","debit_negative":true})).unwrap();
    let banks = s.command("finance-banks", &json!({})).unwrap();
    assert_eq!(
        (
            banks["banks"][0]["bank"].as_str(),
            banks["banks"][0]["date_format"].as_str()
        ),
        (Some("sample"), Some("YYYY-MM-DD"))
    );
    // The ledger's finance mark hides the banks while hide-all is on.
    assert_eq!(
        s.command("finance-banks", &json!({"hide_all":true}))
            .unwrap()["banks"],
        json!([])
    );
    s.command("preferences", &json!({"privateMode":true}))
        .unwrap();
    assert_eq!(
        s.command("finance-banks", &json!({})).unwrap()["banks"],
        json!([])
    );
    s.command("preferences", &json!({"privateMode":false}))
        .unwrap();
    assert_eq!(
        s.command("finance-banks", &json!({})).unwrap()["banks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn kitchen_profile_remove_and_lot_move() {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    s.command(
        "kitchen-profile-set",
        &json!({"subject":"diet","value":"vegetarian","day":"2031-03-03"}),
    )
    .unwrap();
    s.command(
        "kitchen-profile-remove",
        &json!({"subject":"diet","day":"2031-03-04"}),
    )
    .unwrap();
    assert_eq!(
        s.command("kitchen-profile-list", &json!({})).unwrap()["profile"],
        json!([])
    );
    let rice = s.command("kitchen-stock-add", &json!({"subject":"rice","quantity":500,"unit":"g","location":"fridge","day":"2031-03-03"})).unwrap();
    let moved = s
        .command(
            "kitchen-lot-move",
            &json!({"lot":rice["id"],"location":"pantry","day":"2031-03-04"}),
        )
        .unwrap();
    assert_eq!(
        (
            &moved["location"],
            &moved["supersedes"],
            moved["quantity"].as_f64()
        ),
        (&json!("pantry"), &rice["id"], Some(500.0))
    );
}

// T02: fixtures are synthetic isolated contract cases, never personal history.
fn hierarchy_fixture() -> (PathBuf, Service, Value, Value, Value, Vec<Value>) {
    let p = root();
    let mut s = Service::open(&p).unwrap();
    s.command("ledger-migrate", &json!({})).unwrap();
    let category = s
        .command(
            "save",
            &json!({"kind":"category","title":"T02 fixture category"}),
        )
        .unwrap();
    let goal = s
        .command(
            "save",
            &json!({"kind":"goal","title":"T02 fixture goal","category_id":category["id"]}),
        )
        .unwrap();
    let mut tasks = Vec::new();
    for title in ["T02 action one", "T02 action two"] {
        tasks.push(s.command("save", &json!({"kind":"task","title":title,"goal_id":goal["id"],"scheduled_for":"2036-10-13","recurrence":"daily","interval":1})).unwrap());
    }
    let h = s.command("habit-define", &json!({"title":"T02 fixture habit","direction":"form","baseline":1.0,"bundle":"morning","day":"2036-10-13"})).unwrap();
    (p, s, h, category, goal, tasks)
}
fn link_args(h: &Value, c: &Value, g: &Value, tasks: &[Value]) -> Value {
    json!({"id":h["id"],"category_id":c["id"],"goal_id":g["id"],"task_ids":tasks.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(),"day":"2036-10-13"})
}
fn action_args(h: &Value, a: &Value, done: bool) -> Value {
    json!({"id":h["id"],"task_id":a["id"],"day":"2036-10-13","occurrence":a["occurrence"],"occurrence_date":a["occurrence_date"],"done":done,"hash":a["hash"]})
}

#[test]
fn habit_hierarchy_retry_edit_restart_and_individual_occurrences() {
    let (p, mut s, h, c, g, tasks) = hierarchy_fixture();
    let link = link_args(&h, &c, &g, &tasks);
    let bound = s.command("habit-link", &link).unwrap();
    assert_eq!(
        s.command("habit-link", &link).unwrap()["definition"],
        bound["definition"]
    );
    assert_eq!(bound["id"], h["id"]);
    let edited = s.command("habit-define", &json!({"id":h["id"],"title":"T02 edited","direction":"form","baseline":1.0,"bundle":"evening","day":"2036-10-13"})).unwrap();
    assert_eq!(edited["hierarchy"], bound["hierarchy"]);
    let request = json!({"id":h["id"],"day":"2036-10-13"});
    let before = s.command("habit-actions", &request).unwrap();
    assert_eq!(before["actions"].as_array().unwrap().len(), 2);
    assert_eq!(before["actions"][0]["id"], tasks[0]["id"]);
    let done = action_args(&h, &before["actions"][0], true);
    let first = s.command("habit-action-set", &done).unwrap();
    assert_eq!(first["actions"][0]["status"], "completed");
    assert_eq!(first["actions"][1]["status"], "todo");
    // Retrying the same desired state even with the original CAS hash is a no-op.
    assert_eq!(s.command("habit-action-set", &done).unwrap(), first);
    assert_eq!(
        first["actions"][0]["event_key"],
        before["actions"][0]["event_key"]
    );
    drop(s);
    let mut s = Service::open(&p).unwrap();
    let resumed = s.command("habit-actions", &request).unwrap();
    assert_eq!(resumed, first);
    let list = s
        .command("habit-list", &json!({"day":"2036-10-13"}))
        .unwrap();
    assert_eq!(list["bundles"]["evening"], json!([h["id"]]));
    assert_eq!(list["bundles"]["morning"], json!([]));
    let reopened = s
        .command(
            "habit-action-set",
            &action_args(&h, &resumed["actions"][0], false),
        )
        .unwrap();
    assert_eq!(reopened["actions"][0]["status"], "todo");
    assert_eq!(reopened["actions"][1], before["actions"][1]);
    let cal = s
        .command(
            "calendar",
            &json!({"from":"2036-10-13","through":"2036-10-14","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert_eq!(cal.as_array().unwrap().len(), 4);
    assert!(
        cal.as_array().unwrap().iter().all(|a| a["kind"] == "task"
            && a["id"] != h["id"]
            && a["habit_ids"] == json!([h["id"]]))
    );
    assert_ne!(cal[0]["event_key"], cal[2]["event_key"]);
}

#[test]
fn habit_hierarchy_validates_ids_types_duplicates_and_parents_without_writes() {
    let (_, mut s, h, c, g, tasks) = hierarchy_fixture();
    let before = s.command("habit-list", &json!({})).unwrap();
    let link = link_args(&h, &c, &g, &tasks);
    let bad = [
        json!({"id":h["id"],"category_id":g["id"],"goal_id":g["id"],"task_ids":[tasks[0]["id"]]}),
        json!({"id":h["id"],"category_id":42,"task_ids":[]}),
        json!({"id":h["id"],"task_ids":[false]}),
        json!({"id":h["id"],"category_id":c["id"],"goal_id":g["id"],"task_ids":[tasks[0]["id"],tasks[0]["id"]]}),
        json!({"id":h["id"],"category_id":c["id"],"goal_id":g["id"],"task_ids":["task_missing"]}),
        json!({"id":h["id"],"category_id":c["id"],"goal_id":g["id"],"task_ids":[c["id"]]}),
        json!({"id":h["id"],"category_id":null,"goal_id":g["id"],"task_ids":[tasks[0]["id"]]}),
    ];
    for args in bad {
        assert!(s.command("habit-link", &args).is_err());
    }
    assert_eq!(s.command("habit-list", &json!({})).unwrap(), before);
    assert!(s.command("habit-link", &link).is_ok());
}

#[test]
fn habit_hierarchy_moved_and_removed_parents_remain_visible_after_restart() {
    let (p, mut s, h, c, g, tasks) = hierarchy_fixture();
    let link = link_args(&h, &c, &g, &tasks);
    let original = s.command("habit-link", &link).unwrap();
    let other = s
        .command(
            "save",
            &json!({"kind":"category","title":"T02 other category"}),
        )
        .unwrap();
    s.command(
        "save",
        &json!({"id":g["id"],"hash":g["hash"],"category_id":other["id"]}),
    )
    .unwrap();
    drop(s);
    let mut s = Service::open(&p).unwrap();
    let request = json!({"id":h["id"],"day":"2036-10-13"});
    let moved = s.command("habit-actions", &request).unwrap();
    assert_eq!(moved["hierarchy_status"], "moved-parent");
    assert_eq!(moved["hierarchy"], original["hierarchy"]);
    assert!(
        s.command(
            "habit-action-set",
            &action_args(&h, &moved["actions"][0], true)
        )
        .is_err()
    );
    s.command("planner-trash", &json!({"kind":"goal","id":g["id"]}))
        .unwrap();
    drop(s);
    let mut s = Service::open(&p).unwrap();
    let removed = s.command("habit-list", &json!({})).unwrap();
    assert_eq!(removed["habits"][0]["hierarchy_status"], "removed-parent");
    assert_eq!(removed["habits"][0]["hierarchy"], original["hierarchy"]);
    assert!(s.command("habit-link", &link).is_err());
    let cleared = s
        .command("habit-link", &json!({"id":h["id"],"task_ids":[]}))
        .unwrap();
    assert_eq!(cleared["hierarchy_status"], "unlinked");
    assert_eq!(cleared["id"], h["id"]);
}

#[test]
fn habit_action_rejects_stale_and_unbound_occurrences_and_keeps_other_days() {
    let (_, mut s, h, c, g, tasks) = hierarchy_fixture();
    s.command("habit-link", &link_args(&h, &c, &g, &tasks))
        .unwrap();
    let request = json!({"id":h["id"],"day":"2036-10-13"});
    let before = s.command("habit-actions", &request).unwrap();
    let mut bad = action_args(&h, &before["actions"][0], true);
    bad["hash"] = json!("0".repeat(64));
    assert!(s.command("habit-action-set", &bad).is_err());
    bad = action_args(&h, &before["actions"][0], true);
    bad["occurrence"] = json!("stale");
    assert!(s.command("habit-action-set", &bad).is_err());
    bad = action_args(&h, &before["actions"][0], true);
    bad["task_id"] = c["id"].clone();
    assert!(s.command("habit-action-set", &bad).is_err());
    s.command(
        "habit-action-set",
        &action_args(&h, &before["actions"][0], true),
    )
    .unwrap();
    let next = s
        .command("habit-actions", &json!({"id":h["id"],"day":"2036-10-14"}))
        .unwrap();
    assert!(
        next["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["status"] == "todo")
    );
    assert_ne!(
        next["actions"][0]["event_key"],
        before["actions"][0]["event_key"]
    );
}

#[test]
fn habit_actions_privacy_and_unscheduled_task_do_not_emit_habit_events() {
    let (_, mut s, h, c, g, _) = hierarchy_fixture();
    let t = s
        .command(
            "save",
            &json!({"kind":"task","title":"T02 unscheduled","goal_id":g["id"]}),
        )
        .unwrap();
    s.command("habit-link", &link_args(&h, &c, &g, &[t]))
        .unwrap();
    let actions = s
        .command("habit-actions", &json!({"id":h["id"],"day":"2036-10-13"}))
        .unwrap();
    assert_eq!(actions["actions"].as_array().unwrap().len(), 1);
    assert_eq!(actions["actions"][0]["on_timeline"], false);
    s.command(
        "habit-action-set",
        &action_args(&h, &actions["actions"][0], true),
    )
    .unwrap();
    let cal = json!({"from":"2036-10-13","through":"2036-10-13","timezone":"America/Toronto"});
    assert!(
        !s.command("calendar", &cal)
            .unwrap()
            .to_string()
            .contains(h["id"].as_str().unwrap())
    );
    s.command(
        "ledger-mark-private",
        &json!({"scope":"item","key":h["id"]}),
    )
    .unwrap();
    assert!(
        s.command(
            "habit-actions",
            &json!({"id":h["id"],"day":"2036-10-13","hide_all":true})
        )
        .is_err()
    );
}

#[test]
fn habit_action_projection_does_not_expose_other_private_habit_links() {
    let (_, mut s, h, c, g, tasks) = hierarchy_fixture();
    s.command("habit-link", &link_args(&h, &c, &g, &tasks))
        .unwrap();
    let secret = s
        .command(
            "habit-define",
            &json!({"title":"T02 hidden fixture","direction":"form","baseline":1.0}),
        )
        .unwrap();
    s.command("habit-link", &link_args(&secret, &c, &g, &tasks))
        .unwrap();
    s.command(
        "ledger-mark-private",
        &json!({"scope":"item","key":secret["id"]}),
    )
    .unwrap();
    let actions = s
        .command(
            "habit-actions",
            &json!({"id":h["id"],"day":"2036-10-13","hide_all":true}),
        )
        .unwrap();
    assert!(!actions.to_string().contains(secret["id"].as_str().unwrap()));
    s.command("preferences", &json!({"privateMode":true}))
        .unwrap();
    let calendar = s
        .command(
            "calendar",
            &json!({"from":"2036-10-13","through":"2036-10-13","timezone":"America/Toronto"}),
        )
        .unwrap();
    assert!(
        !calendar
            .to_string()
            .contains(secret["id"].as_str().unwrap())
    );
}
