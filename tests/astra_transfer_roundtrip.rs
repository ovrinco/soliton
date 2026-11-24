use serde_json::{Value, json};
use soliton::service::Service;
use soliton_kernel::Entity;
use std::{fs, path::PathBuf};

#[test]
fn actual_populated_export_roundtrips_proposals_without_importing_workspace_preferences() {
    let root = PathBuf::from(
        std::env::var("ASTRA_COPIED_WORKSPACE").expect("explicit isolated actual workspace"),
    );
    let output =
        PathBuf::from(std::env::var("ASTRA_TRANSFER_OUTPUT").expect("new output directory"));
    fs::create_dir(&output).unwrap();
    let mut service = Service::open(&root).unwrap();
    let originals: Vec<_> = service
        .snapshot
        .entities()
        .filter(|e| !matches!(e, Entity::Document(d) if d.document_type == "workspace"))
        .map(|e| (e.id().to_string(), e.canonical_bytes().unwrap()))
        .collect();
    assert!(
        service
            .snapshot
            .entities()
            .any(|e| matches!(e, Entity::Document(d) if d.document_type == "proposal"))
    );
    let export = output.join("actual.json");
    service.command("export", &json!({"path": export})).unwrap();
    let preview = service
        .command("import_preview", &json!({"path": export}))
        .expect("the application's actual export must preview");
    assert_eq!(preview["count"].as_u64().unwrap() as usize, originals.len());
    let target = output.join("imported");
    soliton::transfer::import(&export, preview["hash"].as_str().unwrap(), &target).unwrap();
    let imported = Service::open(&target).unwrap();
    for (id, bytes) in originals {
        let actual = imported
            .snapshot
            .entities()
            .find(|e| e.id().as_str() == id)
            .unwrap();
        assert_eq!(actual.canonical_bytes().unwrap(), bytes, "{id}");
    }
    let workspace = service
        .snapshot
        .entities()
        .find(|e| matches!(e, Entity::Document(d) if d.document_type == "workspace"))
        .unwrap();
    let mut packet: Value = serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
    packet["objects"].as_array_mut().unwrap().push(json!(
        String::from_utf8(workspace.canonical_bytes().unwrap()).unwrap()
    ));
    let forbidden = output.join("preferences-injected.json");
    fs::write(&forbidden, serde_json::to_vec(&packet).unwrap()).unwrap();
    assert!(
        soliton::transfer::preview(&forbidden).is_err(),
        "workspace preferences must remain non-importable"
    );
}
