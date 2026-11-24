//! The arrangement written between Soliton's markers, kept or reverted, on a
//! scratch copy of the recorded two-output topology. Nothing here reaches the
//! running compositor: applies are not live, or go to a recorded stand-in.
use soliton::desktop_apply;

use desktop_apply::*;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/desktop_apply/two-outputs-2026-10-28"
);

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("desktop_apply_contract")
        .join(format!("{name}-{}-{}", std::process::id(), nanos()));
    fs::create_dir_all(&dir).unwrap();
    fs::copy(
        Path::new(FIXTURE).join("monitors.lua"),
        dir.join(CONFIG_FILE),
    )
    .unwrap();
    dir
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn sha(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn recorded(dir: &Path) -> Snapshot {
    let outputs: Vec<ObservedOutput> =
        serde_json::from_slice(&fs::read(Path::new(FIXTURE).join("monitors.json")).unwrap())
            .unwrap();
    Snapshot {
        outputs,
        file: fs::read(dir.join(CONFIG_FILE)).unwrap(),
    }
}

fn request(snapshot: &Snapshot) -> Request {
    let mut request: Request =
        serde_json::from_slice(&fs::read(Path::new(FIXTURE).join("request.json")).unwrap())
            .unwrap();
    request.revision = snapshot.revision();
    request
}

/// Reads the block back the way the Monitors page reads the file: the
/// outputs from the monitor lines, the assignments from the workspace rules.
fn read_back(source: &str) -> (Vec<OutputSetting>, Vec<Assignment>) {
    let start = source.find(BEGIN_MARKER).expect("begin marker");
    let end = source.find(END_MARKER).expect("end marker");
    let field = |line: &str, key: &str| -> String {
        let rest = &line[line.find(&format!("{key} = ")).unwrap() + key.len() + 3..];
        let rest = rest.trim_start();
        if let Some(quoted) = rest.strip_prefix('"') {
            quoted[..quoted.find('"').unwrap()].to_string()
        } else {
            rest[..rest.find([',', ' ', '}']).unwrap()].to_string()
        }
    };
    let mut outputs = Vec::new();
    let mut workspaces = Vec::new();
    for line in source[start..end].lines() {
        if line.starts_with("hl.monitor(") {
            let position = field(line, "position");
            let (x, y) = position.split_once('x').unwrap();
            outputs.push(OutputSetting {
                name: field(line, "output"),
                mode: field(line, "mode"),
                x: x.parse().unwrap(),
                y: y.parse().unwrap(),
                scale: field(line, "scale").parse().unwrap(),
                transform: field(line, "transform").parse().unwrap(),
            });
        } else if line.starts_with("hl.workspace_rule(") {
            workspaces.push(Assignment {
                workspace: field(line, "workspace"),
                output: field(line, "monitor"),
            });
        }
    }
    (outputs, workspaces)
}

#[test]
fn round_trip_gives_back_two_outputs_and_five_assignments() {
    let dir = scratch("round-trip");
    let snapshot = recorded(&dir);
    let request = request(&snapshot);
    let block = render_profile(
        &snapshot.topology_key(),
        &request.outputs,
        &request.workspaces,
    );
    let source = String::from_utf8(snapshot.file.clone()).unwrap();
    let written = replace_block(&source, &block).unwrap();
    let (outputs, workspaces) = read_back(&written);
    let normal = |list: &[OutputSetting]| -> Vec<OutputSetting> {
        list.iter()
            .map(|o| OutputSetting {
                mode: o.mode.trim_end_matches("Hz").to_string(),
                ..o.clone()
            })
            .collect()
    };
    println!("input outputs:      {:?}", normal(&request.outputs));
    println!("read-back outputs:  {:?}", outputs);
    println!("input workspaces:     {:?}", request.workspaces);
    println!("read-back workspaces: {:?}", workspaces);
    assert_eq!(outputs.len(), 2);
    assert_eq!(workspaces.len(), 5);
    assert_eq!(outputs, normal(&request.outputs));
    assert_eq!(workspaces, request.workspaces);
    assert!(block.contains("-- profile "));
    assert!(block.contains(&snapshot.topology_key()));
}

#[test]
fn nothing_outside_the_markers_changes() {
    let before = "-- his first line\nlocal x = 1\n\n";
    let after = "-- his last line\nhl.env(\"A\", \"b\")\n";
    let old = render_profile(
        "old",
        &[],
        &[Assignment {
            workspace: "9".into(),
            output: "DP-1".into(),
        }],
    );
    let source = format!("{before}{old}{after}");
    let snapshot = recorded(&scratch("outside"));
    let request = request(&snapshot);
    let block = render_profile("new", &request.outputs, &request.workspaces);
    let written = replace_block(&source, &block).unwrap();
    assert_eq!(&written.as_bytes()[..before.len()], before.as_bytes());
    assert_eq!(
        &written.as_bytes()[written.len() - after.len()..],
        after.as_bytes()
    );
    assert_eq!(written, format!("{before}{block}{after}"));
    // With no block yet the source is kept whole as the prefix.
    let fresh = replace_block(before, &block).unwrap();
    assert!(fresh.as_bytes().starts_with(before.as_bytes()));
    assert!(fresh.ends_with(&block));
    // A broken block is refused and the source is untouched.
    let broken = format!("{before}{BEGIN_MARKER}\n{after}");
    assert!(matches!(
        replace_block(&broken, &block),
        Err(ApplyError::Refused(_))
    ));
}

#[test]
fn apply_not_live_writes_the_file_and_the_previous_bytes() {
    let dir = scratch("apply");
    let original = fs::read(dir.join(CONFIG_FILE)).unwrap();
    let snapshot = recorded(&dir);
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    println!("summary: {}", plan.summary);
    assert!(!plan.summary.contains('\n'));
    let receipt = apply(&plan, &dir, false).unwrap();
    assert!(receipt.written && !receipt.applied);
    assert_eq!(fs::read(dir.join(CONFIG_FILE)).unwrap(), plan.updated);
    assert_eq!(fs::read(dir.join(PREVIOUS_FILE)).unwrap(), original);
    assert_eq!(receipt.previous_revision, plan.revision);
    assert!(dir.join(PENDING_FILE).exists());
    // A second change waits until this one is kept or reverted.
    let next = preview(
        &Snapshot {
            file: plan.updated.clone(),
            ..snapshot.clone()
        },
        &Request {
            revision: Snapshot {
                file: plan.updated.clone(),
                ..snapshot.clone()
            }
            .revision(),
            ..request(&snapshot)
        },
    )
    .unwrap();
    assert!(matches!(
        apply(&next, &dir, false),
        Err(ApplyError::Refused(_))
    ));
}

#[test]
fn a_lapsed_confirmation_restores_the_previous_bytes_exactly() {
    let dir = scratch("lapsed");
    let path = dir.join(CONFIG_FILE);
    let before = sha(&path);
    let snapshot = recorded(&dir);
    let mut plan = preview(&snapshot, &request(&snapshot)).unwrap();
    plan.confirm_seconds = 0;
    apply(&plan, &dir, false).unwrap();
    let during = sha(&path);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let result = confirm(&dir, &plan.token, true).unwrap();
    let after = sha(&path);
    println!("sha256 before {before}\nsha256 during {during}\nsha256 after  {after}");
    assert!(result.lapsed && result.reverted && !result.kept);
    assert_ne!(before, during);
    assert_eq!(before, after);
    assert!(!dir.join(PENDING_FILE).exists());
}

#[test]
fn keep_keeps_and_revert_restores() {
    let dir = scratch("keep");
    let snapshot = recorded(&dir);
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    apply(&plan, &dir, false).unwrap();
    assert!(matches!(
        confirm(&dir, "not-the-token", true),
        Err(ApplyError::Refused(_))
    ));
    let kept = confirm(&dir, &plan.token, true).unwrap();
    assert!(kept.kept && !kept.reverted);
    assert_eq!(fs::read(dir.join(CONFIG_FILE)).unwrap(), plan.updated);

    let dir = scratch("revert");
    let original = sha(&dir.join(CONFIG_FILE));
    let snapshot = recorded(&dir);
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    apply(&plan, &dir, false).unwrap();
    let reverted = confirm(&dir, &plan.token, false).unwrap();
    assert!(!reverted.kept && reverted.reverted);
    assert_eq!(sha(&dir.join(CONFIG_FILE)), original);
}

#[test]
fn a_stale_revision_is_refused() {
    let dir = scratch("stale");
    let snapshot = recorded(&dir);
    let mut stale = request(&snapshot);
    stale.revision = "0".repeat(64);
    assert!(matches!(
        preview(&snapshot, &stale),
        Err(ApplyError::Refused(_))
    ));
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    fs::write(dir.join(CONFIG_FILE), b"-- edited by hand\n").unwrap();
    match apply(&plan, &dir, false) {
        Err(ApplyError::Refused(reason)) => println!("refused: {reason}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        fs::read(dir.join(CONFIG_FILE)).unwrap(),
        b"-- edited by hand\n"
    );
    assert!(!dir.join(PREVIOUS_FILE).exists());
}

struct Recorded {
    errors: &'static str,
    reloads: Cell<u32>,
}

impl Compositor for Recorded {
    fn reload(&self) -> desktop_apply::Result<()> {
        self.reloads.set(self.reloads.get() + 1);
        Ok(())
    }
    fn config_errors(&self) -> desktop_apply::Result<String> {
        Ok(self.errors.to_string())
    }
}

#[test]
fn configuration_errors_revert_at_once_with_the_compositor_lines() {
    let dir = scratch("errors");
    let original = sha(&dir.join(CONFIG_FILE));
    let snapshot = recorded(&dir);
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    let compositor = Recorded {
        errors: "Config error in file monitors.lua at line 20: bad mode\n\n",
        reloads: Cell::new(0),
    };
    match apply_with(&plan, &dir, Some(&compositor)) {
        Err(ApplyError::Compositor(lines)) => {
            println!("compositor lines: {lines:?}");
            assert_eq!(
                lines,
                vec!["Config error in file monitors.lua at line 20: bad mode"]
            );
        }
        other => panic!("expected the compositor's lines, got {other:?}"),
    }
    assert_eq!(sha(&dir.join(CONFIG_FILE)), original);
    assert_eq!(compositor.reloads.get(), 2);
    assert!(!dir.join(PENDING_FILE).exists());

    // A clean reload applies and a revert reloads once more.
    let dir = scratch("clean");
    let snapshot = recorded(&dir);
    let plan = preview(&snapshot, &request(&snapshot)).unwrap();
    let compositor = Recorded {
        errors: "\n\n",
        reloads: Cell::new(0),
    };
    let receipt = apply_with(&plan, &dir, Some(&compositor)).unwrap();
    assert!(receipt.applied);
    confirm_with(&dir, &plan.token, false, &compositor).unwrap();
    assert_eq!(compositor.reloads.get(), 2);
}

#[test]
fn a_rotated_output_swaps_width_and_height_in_the_position_check() {
    let dir = scratch("rotated");
    let snapshot = recorded(&dir);
    let mut request = request(&snapshot);
    request.outputs[0].transform = 1;
    request.outputs[1].x = 1080;
    assert_eq!(request.outputs[0].logical_size(), Some((1080, 1920)));
    let plan = preview(&snapshot, &request).unwrap();
    println!("rotated summary: {}", plan.summary);
    assert!(plan.block.contains("position = \"1080x0\""));
    assert!(plan.block.contains("transform = 1"));
    request.outputs[0].transform = 0;
    assert_eq!(request.outputs[0].logical_size(), Some((1920, 1080)));
    match preview(&snapshot, &request) {
        Err(ApplyError::Refused(reason)) => println!("unrotated at 1080: {reason}"),
        other => panic!("expected an overlap refusal, got {other:?}"),
    }
}

#[test]
fn requests_outside_the_active_outputs_are_refused() {
    let dir = scratch("refusals");
    let snapshot = recorded(&dir);
    type Change = Box<dyn Fn(&mut Request)>;
    let cases: Vec<Change> = vec![
        Box::new(|r| r.outputs[0].mode = "800x600@60.00Hz".into()),
        Box::new(|r| r.outputs[0].scale = 1.7),
        Box::new(|r| r.outputs[0].transform = 8),
        Box::new(|r| r.outputs.pop().map(drop).unwrap_or_default()),
        Box::new(|r| r.workspaces[0].output = "HDMI-A-9".into()),
        Box::new(|r| r.workspaces[1].workspace = "1".into()),
    ];
    for (index, change) in cases.iter().enumerate() {
        let mut asked = request(&snapshot);
        change(&mut asked);
        match preview(&snapshot, &asked) {
            Err(ApplyError::Refused(reason)) => println!("case {}: {reason}", index + 1),
            other => panic!("case {} not refused: {other:?}", index + 1),
        }
    }
}
