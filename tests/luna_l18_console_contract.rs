use std::fs;
use std::path::Path;

fn console_file(name: &str) -> String {
    let console = Path::new(env!("CARGO_MANIFEST_DIR")).join("console");
    let path = if name == "Backend.qml" {
        console.join(name)
    } else {
        console.join("archive").join(name)
    };
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn console_surfaces_are_task_focused() {
    let pane = console_file("ConsolePane.qml");
    let inspector = console_file("Inspector.qml");

    assert!(!pane.contains("This launch cannot run models."));
    assert!(!pane.contains("Engine connection and each prompt are explicit actions."));
    assert!(!console_file("ConversationPane.qml").contains("Your work starts here."));
    assert!(!inspector.contains("Zeno owns execution"));
    assert!(!inspector.contains("QREAM owns governed effects"));
    assert!(!inspector.contains("Soliton owns task identity"));
}

#[test]
fn console_keeps_offline_generation_explicit() {
    let pane = console_file("ConversationPane.qml");
    let backend = console_file("Backend.qml");

    assert!(pane.contains("Backend.runtime.mode !== \"offline\""));
    assert!(pane.contains("Backend.isActive(pane.sessionId)"));
    assert!(backend.contains("runtime.mode === \"offline\""));
    assert!(backend.contains("Inference is disabled."));
}
