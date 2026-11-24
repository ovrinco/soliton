// The string floor rides the full check: a new offender among the visible strings fails the build.
use std::{fs, path::Path, process::Command};

fn floor(args: &[&str]) -> std::process::Output {
    Command::new("python3")
        .arg("-I")
        .arg("scripts/string_floor.py")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("python3 runs the string floor")
}

#[test]
fn string_floor_holds_on_the_tree() {
    let out = floor(&["--check"]);
    assert!(
        out.status.success(),
        "string floor refused the tree:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn string_floor_refuses_a_loading_label_and_a_refresh_control() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("string-floor-scratch");
    fs::create_dir_all(root.join("ui")).unwrap();
    fs::write(
        root.join("ui/Scratch.qml"),
        "import QtQuick\nItem {\n    LabelText { text: 'Loading…' }\n    SButton { text: 'Refresh' }\n}\n",
    )
    .unwrap();
    let baseline = root.join("baseline.tsv");
    fs::write(&baseline, "").unwrap();
    let out = floor(&[
        "--check",
        "--root",
        root.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
    ]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "the scratch tree passed:\n{printed}");
    assert!(printed.contains("\tloading\tLoading…"), "{printed}");
    assert!(printed.contains("\tcontrol\tRefresh"), "{printed}");
}
