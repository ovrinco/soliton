use serde_json::{Value, json};
use soliton::desktop_fallback::*;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/desktop_fallback")
}
fn topology(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fixtures().join(format!("{name}.json"))).unwrap())
        .unwrap()
}
fn snapshot(name: &str) -> Snapshot {
    Snapshot::from_value(&topology(name)).unwrap()
}
fn dp1_fallback() -> Fallback {
    Fallback {
        key: vec!["DP-1".into()],
        survivor: "DP-1".into(),
        stay: vec!["3".into(), "4".into(), "5".into()],
    }
}
fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliton-t15-{label}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn lua(program: &str) -> Option<String> {
    let path = Path::new("/usr/bin/lua");
    if !path.exists() {
        println!("skipped: /usr/bin/lua is not installed");
        return None;
    }
    // One file per call: the lua tests run on parallel threads of one process and must not share a script.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let file = scratch("lua").join(format!("run-{n}.lua"));
    fs::write(&file, program).unwrap();
    let out = Command::new(path).arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "lua failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).unwrap())
}

#[test]
fn decide_on_one_output_moves_only_workspaces_one_and_two_to_the_survivor() {
    let moves = decide(&snapshot("one-output-dp1"), &[dp1_fallback()]);
    println!("{}", serde_json::to_string_pretty(&moves).unwrap());
    let ids: Vec<i64> = moves.iter().map(|m| m.workspace).collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(moves.iter().all(|m| m.from == "HDMI-A-1" && m.to == "DP-1"));
    assert_eq!(
        moves.iter().map(|m| m.windows).sum::<u64>(),
        3,
        "the windows travel with their workspaces"
    );
}

#[test]
fn decide_does_nothing_while_both_outputs_are_on() {
    assert!(decide(&snapshot("two-output"), &[dp1_fallback()]).is_empty());
}

#[test]
fn restore_returns_displaced_workspaces_when_the_second_output_comes_back() {
    let homes: BTreeMap<String, String> = [("1", "HDMI-A-1"), ("2", "HDMI-A-1"), ("3", "DP-1")]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let moves = restore(&snapshot("two-output-returned"), &[dp1_fallback()], &homes);
    assert_eq!(
        moves
            .iter()
            .map(|m| (m.workspace, m.to.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "HDMI-A-1"), (2, "HDMI-A-1")]
    );
    assert!(restore(&snapshot("one-output-dp1"), &[dp1_fallback()], &homes).is_empty());
}

#[test]
fn can_record_refuses_while_the_other_output_is_on_and_allows_it_when_off() {
    let refused = can_record(&snapshot("two-output"), "DP-1")
        .unwrap_err()
        .to_string();
    println!("two-output: Err({refused})");
    assert!(!refused.contains('/') && !refused.contains("DP-1") && !refused.contains("HDMI"));
    assert!(can_record(&snapshot("one-output-dp1"), "DP-1").is_ok());
    assert!(
        can_record(&snapshot("one-output-dp1"), "HDMI-A-1").is_err(),
        "an output that is off is not a survivor"
    );
}

#[test]
fn rendered_block_passes_luac_and_names_the_three_events() {
    let block = render_handlers(&[dp1_fallback()]).unwrap();
    assert!(block.starts_with(START) && block.ends_with(END));
    assert_eq!(block.matches("hl.on(").count(), 3);
    for event in ["monitor.removed", "monitor.added", "monitor.layout_changed"] {
        assert!(block.contains(&format!("hl.on(\"{event}\"")), "{event}");
    }
    let file = scratch("luac").join("rendered.lua");
    fs::write(&file, &block).unwrap();
    println!("rendered: {}", file.display());
    if !Path::new("/usr/bin/luac").exists() {
        println!("skipped: /usr/bin/luac is not installed");
        return;
    }
    let out = Command::new("/usr/bin/luac")
        .arg("-p")
        .arg(&file)
        .output()
        .unwrap();
    println!(
        "luac -p exit {:?} {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success());
}

#[test]
fn block_round_trips_through_replace_with_bytes_outside_untouched() {
    let original = fs::read_to_string(fixtures().join("monitors.lua")).unwrap();
    let once = replace(&original, &[dp1_fallback()]).unwrap();
    assert!(once.starts_with(&original));
    assert_eq!(read(&once).unwrap(), vec![dp1_fallback()]);
    let mut wrapped = once.clone();
    wrapped.push_str("-- his own line after the block\n");
    let other = Fallback {
        key: vec!["HDMI-A-1".into()],
        survivor: "HDMI-A-1".into(),
        stay: vec!["1".into()],
    };
    let twice = replace(&wrapped, &[dp1_fallback(), other.clone()]).unwrap();
    let start = twice.find(START).unwrap();
    let end = twice.find(END).unwrap() + END.len();
    assert_eq!(&twice[..start], &wrapped[..wrapped.find(START).unwrap()]);
    assert_eq!(&twice[end..], "-- his own line after the block\n");
    assert_eq!(read(&twice).unwrap(), vec![dp1_fallback(), other]);
    let damaged = format!("{twice}{START}");
    assert!(
        replace(&damaged, &[]).is_err(),
        "a damaged block is refused"
    );
}

#[test]
fn handlers_apply_the_fallback_and_return_the_two_output_arrangement() {
    let block = render_handlers(&[dp1_fallback()]).unwrap();
    let mock = fs::read_to_string(fixtures().join("mock_hl.lua")).unwrap();
    let program = format!(
        r#"{mock}
local both = {{ {{ "HDMI-A-1", true }}, {{ "DP-1", true }} }}
local one = {{ {{ "HDMI-A-1", false, true }}, {{ "DP-1", true }} }}
set_topology(both, {{ {{1,"HDMI-A-1"}}, {{2,"HDMI-A-1"}}, {{3,"DP-1"}}, {{4,"DP-1"}}, {{5,"DP-1"}} }})
{block}
print("loaded " .. placement())
set_topology(one, {{ {{1,"HDMI-A-1"}}, {{2,"HDMI-A-1"}}, {{3,"DP-1"}}, {{4,"DP-1"}}, {{5,"DP-1"}} }})
fire("monitor.removed")
print("off " .. placement() .. " rules " .. enabled_rules())
set_topology(both, {{ {{1,"DP-1"}}, {{2,"DP-1"}}, {{3,"DP-1"}}, {{4,"DP-1"}}, {{5,"DP-1"}} }})
fire("monitor.added")
print("on " .. placement() .. " rules " .. enabled_rules())
set_topology({{ {{ "DP-1", true }} }}, {{ {{1,"DP-1"}}, {{2,"DP-1"}}, {{3,"DP-1"}}, {{4,"DP-1"}}, {{5,"DP-1"}} }})
fire("monitor.removed")
set_topology(both, {{ {{1,"DP-1"}}, {{2,"DP-1"}}, {{3,"DP-1"}}, {{4,"DP-1"}}, {{5,"DP-1"}} }})
fire("monitor.layout_changed")
print("unplugged and back " .. placement())
{block}
fire("monitor.added")
print("reloaded handlers once " .. placement())
"#
    );
    let Some(out) = lua(&program) else { return };
    println!("{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"loaded 1=HDMI-A-1 2=HDMI-A-1 3=DP-1 4=DP-1 5=DP-1"));
    assert!(lines.contains(&"move 1 DP-1") && lines.contains(&"move 2 DP-1"));
    assert!(lines.contains(&"off 1=DP-1 2=DP-1 3=DP-1 4=DP-1 5=DP-1 rules 3@DP-1 4@DP-1 5@DP-1"));
    assert!(lines.contains(&"on 1=HDMI-A-1 2=HDMI-A-1 3=DP-1 4=DP-1 5=DP-1 rules "));
    assert!(lines.contains(&"unplugged and back 1=HDMI-A-1 2=HDMI-A-1 3=DP-1 4=DP-1 5=DP-1"));
    assert!(!out.contains("move 3") && !out.contains("move 4") && !out.contains("move 5"));
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("move ")).count(),
        6,
        "two away, two back, none at unplug, two back"
    );
    assert!(lines.last().unwrap().starts_with("reloaded handlers once"));
}

#[test]
fn commands_read_preview_and_record_into_the_given_file() {
    let config = scratch("command").join("monitors.lua");
    let original = fs::read_to_string(fixtures().join("monitors.lua")).unwrap();
    fs::write(&config, &original).unwrap();
    let two =
        json!({"snapshot": topology("two-output"), "output": "DP-1", "stay": ["3", "4", "5"]});
    let one =
        json!({"snapshot": topology("one-output-dp1"), "output": "DP-1", "stay": ["3", "4", "5"]});
    let read_two = command("desktop-fallback-read", &two, &config).unwrap();
    assert_eq!(read_two["canRecord"], false);
    assert!(command("desktop-fallback-record", &two, &config).is_err());
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        original,
        "a refused record writes nothing"
    );
    let read_one = command("desktop-fallback-read", &one, &config).unwrap();
    assert_eq!(read_one["offered"], json!(["DP-1"]));
    assert_eq!(read_one["canRecord"], true);
    let preview = command("desktop-fallback-preview", &one, &config).unwrap();
    assert_eq!(preview["moves"].as_array().unwrap().len(), 2);
    command("desktop-fallback-record", &one, &config).unwrap();
    let written = fs::read_to_string(&config).unwrap();
    assert!(written.starts_with(&original));
    assert_eq!(read(&written).unwrap(), vec![dp1_fallback()]);
    let after = command("desktop-fallback-read", &one, &config).unwrap();
    assert_eq!(after["current"]["survivor"], "DP-1");
    assert_eq!(after["moves"].as_array().unwrap().len(), 2);
}
#[test]
fn a_sleeping_screen_stays_present_and_keeps_its_workspaces() {
    let asleep = snapshot("two-output-dp2-asleep");
    println!("asleep key {:?}", asleep.active_key());
    let moves = decide(&asleep, &[dp1_fallback()]);
    println!("asleep moves {}", moves.len());
    assert_eq!(
        asleep.active_key(),
        vec!["DP-1".to_string(), "HDMI-A-1".to_string()],
        "a sleeping screen is still present"
    );
    assert!(moves.is_empty(), "a sleeping screen keeps its workspaces");
}

#[test]
fn a_disabled_screen_is_gone_and_its_workspaces_go_to_the_survivor() {
    let disabled = snapshot("two-output-dp2-disabled");
    let moves = decide(&disabled, &[dp1_fallback()]);
    println!(
        "disabled key {:?} moves {}",
        disabled.active_key(),
        serde_json::to_string(&moves).unwrap()
    );
    assert_eq!(disabled.active_key(), vec!["DP-1".to_string()]);
    assert_eq!(
        moves
            .iter()
            .map(|m| (m.workspace, m.to.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "DP-1"), (2, "DP-1")]
    );
}

#[test]
fn can_record_refuses_while_the_other_screen_only_sleeps() {
    let refused = can_record(&snapshot("two-output-dp2-asleep"), "DP-1")
        .unwrap_err()
        .to_string();
    println!("asleep: Err({refused})");
    assert!(can_record(&snapshot("two-output-dp2-disabled"), "DP-1").is_ok());
}

#[test]
fn a_power_change_writes_nothing_and_applies_nothing() {
    let config = scratch("dpms").join("monitors.lua");
    let original = fs::read_to_string(fixtures().join("monitors.lua")).unwrap();
    fs::write(&config, replace(&original, &[dp1_fallback()]).unwrap()).unwrap();
    let before = (
        fs::read(&config).unwrap(),
        fs::metadata(&config).unwrap().modified().unwrap(),
    );
    let asleep = command(
        "desktop-fallback-read",
        &json!({"snapshot": topology("two-output-dp2-asleep")}),
        &config,
    )
    .unwrap();
    let preview = command(
        "desktop-fallback-preview",
        &json!({"snapshot": topology("two-output-dp2-asleep")}),
        &config,
    )
    .unwrap();
    let disabled = command(
        "desktop-fallback-read",
        &json!({"snapshot": topology("two-output-dp2-disabled")}),
        &config,
    )
    .unwrap();
    let after = (
        fs::read(&config).unwrap(),
        fs::metadata(&config).unwrap().modified().unwrap(),
    );
    println!(
        "asleep: active {} asleep {} moves {} preview {}; disabled: moves {}; file unchanged {} mtime unchanged {}",
        asleep["active"],
        asleep["asleep"],
        asleep["moves"].as_array().unwrap().len(),
        preview["moves"].as_array().unwrap().len(),
        disabled["moves"].as_array().unwrap().len(),
        before.0 == after.0,
        before.1 == after.1
    );
    assert_eq!(asleep["asleep"], json!(["HDMI-A-1"]));
    assert_eq!(asleep["moves"].as_array().unwrap().len(), 0);
    assert_eq!(preview["moves"].as_array().unwrap().len(), 0);
    assert_eq!(
        disabled["moves"].as_array().unwrap().len(),
        2,
        "the saved one-screen arrangement applies"
    );
    assert_eq!(before, after, "zero writes to monitors.lua");
}

fn trace_lua() -> String {
    let steps: Vec<Value> = fs::read_to_string(fixtures().join("sleep-wake-trace.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(steps.len(), 5);
    let mut lua = String::new();
    for step in &steps {
        let outputs: Vec<String> = step["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                format!(
                    "{{ \"{}\", {}, {} }}",
                    o["name"].as_str().unwrap(),
                    o["dpmsStatus"],
                    o["disabled"]
                )
            })
            .collect();
        lua.push_str(&format!(
            "local before = MOVES\nset_outputs({{ {} }})\n",
            outputs.join(", ")
        ));
        if let Some(event) = step["event"].as_str() {
            lua.push_str(&format!("fire(\"{event}\")\n"));
        }
        lua.push_str(&format!(
            "print(\"step {} moves \" .. (MOVES - before) .. \" \" .. placement())\n",
            step["step"].as_str().unwrap()
        ));
    }
    lua
}

fn replay(fallbacks: &[Fallback]) -> Option<Vec<String>> {
    let mock = fs::read_to_string(fixtures().join("mock_hl.lua")).unwrap();
    let block = render_handlers(fallbacks).unwrap();
    let program = format!(
        "{mock}\nset_topology({{ {{ \"HDMI-A-1\", true }}, {{ \"DP-1\", true }} }}, {{ {{1,\"HDMI-A-1\"}}, {{2,\"HDMI-A-1\"}}, {{3,\"DP-1\"}}, {{4,\"DP-1\"}}, {{5,\"DP-1\"}} }})\n{block}\ndo\n{}end\n",
        trace_lua()
    );
    let out = lua(&program)?;
    println!("{out}");
    Some(
        out.lines()
            .filter(|l| l.starts_with("step "))
            .map(str::to_string)
            .collect(),
    )
}

#[test]
fn trace_sleep_and_wake_move_nothing_unplug_moves_the_stranded_ones() {
    let start = "1=HDMI-A-1 2=HDMI-A-1 3=DP-1 4=DP-1 5=DP-1";
    let Some(steps) = replay(&[dp1_fallback()]) else {
        return;
    };
    assert_eq!(steps[0], format!("step start moves 0 {start}"));
    assert_eq!(steps[1], format!("step sleep moves 0 {start}"));
    assert_eq!(
        steps[2],
        format!("step wake moves 0 {start}"),
        "after sleep and wake the list equals the start"
    );
    assert_eq!(
        steps[3], "step unplug moves 2 1=DP-1 2=DP-1 3=DP-1 4=DP-1 5=DP-1",
        "the stranded workspaces are on the survivor"
    );
    assert_eq!(
        steps[4],
        format!("step replug moves 2 {start}"),
        "T15's accepted return: displaced workspaces go home"
    );
    let Some(bare) = replay(&[]) else { return };
    println!("no saved arrangement: {bare:?}");
    assert!(
        bare.iter().all(|l| l.contains(" moves 0 ")),
        "with no saved arrangement the handlers move nothing at any step"
    );
}
