use serde_json::{Value, json};
use soliton::service::{Service, tool_launch::ToolLauncher};
use soliton_kernel::Entity;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "soliton-tool-launch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("bin")).unwrap();
        Self(root)
    }
    fn fake(&self, name: &str) {
        let script = self.0.join("bin").join(name);
        fs::write(&script, format!(
            "#!/bin/sh\n{{ printf 'cwd=%s\\n' \"$PWD\"; printf 'TERM=%s\\n' \"${{TERM-unset}}\"; printf 'argv=%s\\n' \"$@\"; }} > '{}'\nprintf done >> '{}'\n",
            self.0.join(format!("{name}.record")).display(),
            self.0.join(format!("{name}.record")).display()
        )).unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn launcher(&self) -> ToolLauncher {
        ToolLauncher::discover_with(self.0.join("bin").as_os_str(), Some("foot".as_ref()), &[])
    }
    fn service(&self) -> Service {
        Service::open_with_tool_launcher(&self.0.join("data"), self.launcher()).unwrap()
    }
    fn record(&self, name: &str) -> String {
        let start = Instant::now();
        loop {
            let record =
                fs::read_to_string(self.0.join(format!("{name}.record"))).unwrap_or_default();
            if record.ends_with("done") {
                return record;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "fake launch did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn launch(s: &mut Service, tool: &str, context: Value) -> Value {
    s.command("tool-launch", &json!({"tool":tool,"context":context}))
        .unwrap()
}

#[test]
fn plain_terminal_uses_documents_folder_and_no_agent() {
    let f = Fixture::new();
    f.fake("foot");
    let mut s = f.service();
    let r = launch(
        &mut s,
        "terminal",
        json!({"page":"Action Center","view":"Calendar"}),
    );
    let folder = f.0.join("data/documents");
    assert_eq!(r["folder"], folder.to_str().unwrap());
    assert_eq!(r["started"], true);
    let record = f.record("foot");
    assert!(record.contains(&format!("cwd={}\n", folder.display())));
    assert_eq!(
        record
            .lines()
            .filter(|l| l.starts_with("argv="))
            .collect::<Vec<_>>(),
        vec![format!("argv=--working-directory={}", folder.display())]
    );
    println!("PLAIN {record}");
}

#[test]
fn codex_and_default_agent_use_persisted_goal_folder() {
    let f = Fixture::new();
    f.fake("foot");
    f.fake("codex");
    let folder = f.0.join("goal folder");
    fs::create_dir(&folder).unwrap();
    let mut s = f.service();
    let goal = s
        .command(
            "save",
            &json!({"kind":"goal","title":"Launch goal","folder":folder}),
        )
        .unwrap();
    drop(s);
    let mut s = f.service();
    let r = launch(
        &mut s,
        "agent",
        json!({"page":"Strategy","view":"Goals","selected_id":goal["id"]}),
    );
    assert_eq!(r["tool"], "codex");
    assert_eq!(r["folder"], folder.to_str().unwrap());
    let record = f.record("foot");
    assert!(record.contains(&format!("cwd={}\n", folder.display())));
    assert_eq!(
        record
            .lines()
            .filter(|l| l.starts_with("argv="))
            .collect::<Vec<_>>(),
        vec![
            format!("argv=--working-directory={}", folder.display()),
            format!("argv={}", f.0.join("bin/codex").display())
        ]
    );
    assert!(
        !f.0.join("codex.record").exists(),
        "fake emulator must not execute even fake Codex"
    );
    println!("AGENT_DEFAULT={} CODEX {record}", r["tool"]);
    fs::remove_file(f.0.join("foot.record")).unwrap();
    assert_eq!(
        launch(&mut s, "codex", json!({"selected_id":goal["id"]}))["tool"],
        "codex"
    );
    f.record("foot");
}

#[test]
fn qream_and_zeno_have_own_commands_in_context_folder() {
    let f = Fixture::new();
    f.fake("qream");
    f.fake("zeno");
    let mut s = f.service();
    let folder = f.0.join("data/documents");
    assert_eq!(launch(&mut s, "qream", json!({}))["started"], true);
    let record = f.record("qream");
    assert!(record.contains(&format!("cwd={}\n", folder.display())));
    assert_eq!(
        record
            .lines()
            .filter(|l| l.starts_with("argv="))
            .collect::<Vec<_>>(),
        vec!["argv=gui"]
    );
    assert!(!f.0.join("foot.record").exists());
    for emulator in ["foot", "alacritty", "ghostty", "kitty"] {
        let f = Fixture::new();
        f.fake(emulator);
        f.fake("zeno");
        let launcher =
            ToolLauncher::discover_with(f.0.join("bin").as_os_str(), Some(emulator.as_ref()), &[]);
        let mut s = Service::open_with_tool_launcher(&f.0.join("data"), launcher).unwrap();
        let folder = f.0.join("data/documents");
        assert_eq!(launch(&mut s, "zeno", json!({}))["started"], true);
        let record = f.record(emulator);
        assert!(record.contains(&format!("cwd={}\n", folder.display())));
        let mut expected = match emulator {
            "foot" | "ghostty" => vec![format!("argv=--working-directory={}", folder.display())],
            "alacritty" => vec![
                "argv=--working-directory".into(),
                format!("argv={}", folder.display()),
            ],
            "kitty" => vec![
                "argv=--directory".into(),
                format!("argv={}", folder.display()),
            ],
            _ => unreachable!(),
        };
        if matches!(emulator, "alacritty" | "ghostty") {
            expected.push("argv=-e".into());
        }
        expected.push(format!("argv={}", f.0.join("bin/zeno").display()));
        expected.push(format!("argv={}", folder.display()));
        assert_eq!(
            record
                .lines()
                .filter(|l| l.starts_with("argv="))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(!f.0.join("zeno.record").exists());
        println!(
            "ZENO emulator={emulator} cwd={} {}",
            folder.display(),
            expected.join(" ")
        );
    }
}

#[test]
fn note_uses_its_document_folder_and_goal_without_folder_falls_back() {
    let f = Fixture::new();
    f.fake("foot");
    let mut s = f.service();
    let note = s
        .command("save", &json!({"kind":"note","title":"A note"}))
        .unwrap();
    let folder = Path::new(note["path"].as_str().unwrap()).parent().unwrap();
    assert_eq!(
        launch(&mut s, "terminal", json!({"selected_id":note["id"]}))["folder"],
        folder.to_str().unwrap()
    );
    f.record("foot");
    fs::remove_file(f.0.join("foot.record")).unwrap();
    let goal = s
        .command("save", &json!({"kind":"goal","title":"No folder"}))
        .unwrap();
    assert_eq!(
        launch(&mut s, "terminal", json!({"selected_id":goal["id"]}))["folder"],
        f.0.join("data/documents").to_str().unwrap()
    );
    f.record("foot");
}

#[test]
fn availability_is_executable_based_and_absent_launch_is_explained() {
    let f = Fixture::new();
    f.fake("foot");
    f.fake("qream");
    fs::write(f.0.join("bin/codex"), "not executable").unwrap();
    let mut s = f.service();
    let r = s.command("tool-available", &json!({})).unwrap();
    for (tool, expected) in [
        ("terminal", true),
        ("codex", false),
        ("qream", true),
        ("zeno", false),
    ] {
        let entry = r["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tool"] == tool)
            .unwrap();
        assert_eq!(entry["available"], expected);
    }
    let r = launch(&mut s, "codex", json!({}));
    assert_eq!(r["started"], false);
    assert!(r["message"].as_str().unwrap().contains("not installed"));
    assert!(!f.0.join("foot.record").exists());
    let no_terminal = Fixture::new();
    no_terminal.fake("zeno");
    let launcher = no_terminal.launcher();
    let availability = launcher.available();
    let zeno = availability["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tool"] == "zeno")
        .unwrap();
    assert_eq!(zeno["installed"], true);
    assert_eq!(zeno["available"], false);
    let result = launcher.launch("zeno", &no_terminal.0).unwrap();
    assert_eq!(result["started"], false);
    assert!(
        result["message"]
            .as_str()
            .unwrap()
            .contains("terminal emulator")
    );
    assert!(!no_terminal.0.join("zeno.record").exists());
}

#[test]
fn terminal_configuration_priority_and_machine_inventory() {
    let f = Fixture::new();
    f.fake("foot");
    f.fake("kitty");
    let config = f.0.join("terminals.list");
    fs::write(&config, "# configured\nkitty.desktop\n").unwrap();
    let configured = ToolLauncher::discover_with(
        f.0.join("bin").as_os_str(),
        None,
        std::slice::from_ref(&config),
    );
    assert!(
        configured.available()["emulator"]["program"]
            .as_str()
            .unwrap()
            .ends_with("/kitty")
    );
    let environment = ToolLauncher::discover_with(
        f.0.join("bin").as_os_str(),
        Some("foot".as_ref()),
        &[config],
    );
    assert!(
        environment.available()["emulator"]["program"]
            .as_str()
            .unwrap()
            .ends_with("/foot")
    );
    let fallback = ToolLauncher::discover_with(f.0.join("bin").as_os_str(), None, &[]);
    assert!(
        fallback.available()["emulator"]["program"]
            .as_str()
            .unwrap()
            .ends_with("/kitty")
    );
    println!(
        "MACHINE_TOOL_AVAILABLE={}",
        ToolLauncher::discover().available()
    );
}

#[test]
fn goal_folder_codec_is_optional_additive_and_validated() {
    let f = Fixture::new();
    let mut s = f.service();
    let old = s
        .command("save", &json!({"kind":"goal","title":"Old goal"}))
        .unwrap();
    let bytes = fs::read(old["path"].as_str().unwrap()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("folder="));
    let Entity::Aim(old) = Entity::from_canonical_bytes(&bytes).unwrap() else {
        panic!()
    };
    assert_eq!(old.folder, None);
    assert!(
        s.command(
            "save",
            &json!({"kind":"goal","title":"Invalid","folder":"relative/path"})
        )
        .is_err()
    );
    let goal = s
        .command("save", &json!({"kind":"goal","title":"New","folder":f.0}))
        .unwrap();
    let bytes = fs::read(goal["path"].as_str().unwrap()).unwrap();
    let Entity::Aim(new) = Entity::from_canonical_bytes(&bytes).unwrap() else {
        panic!()
    };
    assert_eq!(new.folder.as_deref(), f.0.to_str());
}

#[test]
fn invalid_context_and_missing_folder_cannot_start_a_child() {
    let f = Fixture::new();
    f.fake("foot");
    let mut s = f.service();
    assert!(
        s.command(
            "tool-launch",
            &json!({"tool":"terminal","context":{"selected_id":"invalid"}})
        )
        .is_err()
    );
    let goal = s
        .command(
            "save",
            &json!({"kind":"goal","title":"Missing","folder":f.0.join("missing")}),
        )
        .unwrap();
    assert!(
        s.command(
            "tool-launch",
            &json!({"tool":"terminal","context":{"selected_id":goal["id"]}})
        )
        .is_err()
    );
    assert!(!f.0.join("foot.record").exists());
}
