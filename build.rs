// Refuses a build in which the interface asks for an icon the family lacks:
// every literal name given to SIcon, iconName or symbol must be in a table of
// ui/SIcon.qml, and every glyph in those tables must ship under assets/icons.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn qml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            qml_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "qml") {
            out.push(path);
        }
    }
}

fn quoted(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['\'', '"']) {
        let quote = rest.as_bytes()[start] as char;
        let tail = &rest[start + 1..];
        let Some(end) = tail.find(quote) else { break };
        names.push(tail[..end].to_string());
        rest = &tail[end + 1..];
    }
    names
}

// The expression bound to `key:` up to the end of its statement, with
// comparison operands removed (`x === 'youtube' ? 'play' : 'file'`).
fn bound_names(line: &str, key: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut from = 0;
    while let Some(pos) = line[from..].find(key) {
        let at = from + pos;
        from = at + key.len();
        let before = line[..at].chars().last();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_') {
            continue;
        }
        let expr = &line[from..];
        let expr = &expr[..expr.find([';', '}']).unwrap_or(expr.len())];
        let mut cleaned = String::new();
        let mut parts = expr.split("==");
        if let Some(first) = parts.next() {
            cleaned.push_str(first);
        }
        for part in parts {
            let part = part.trim_start_matches('=').trim_start();
            let skip = if part.starts_with(['\'', '"']) {
                let q = part.as_bytes()[0] as char;
                part[1..].find(q).map_or(part.len(), |e| e + 2)
            } else {
                0
            };
            cleaned.push_str(&part[skip..]);
        }
        let cleaned = cleaned.replace("!'", "'");
        names.extend(quoted(&cleaned).into_iter().filter(|n| !n.is_empty()));
    }
    names
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon_source = root.join("ui/SIcon.qml");
    let table = fs::read_to_string(&icon_source).expect("ui/SIcon.qml");
    let mut mapped = BTreeSet::new();
    for line in table.lines() {
        let row = line.trim();
        if let Some((name, glyph)) = row.trim_end_matches(',').split_once(": ") {
            let (name, glyph) = (quoted(name), quoted(glyph));
            if name.len() == 1 && glyph.len() == 1 && row.starts_with('\'') {
                let svg = root.join("assets/icons").join(format!("{}.svg", glyph[0]));
                if !svg.is_file() {
                    panic!(
                        "icon '{}' maps to '{}', which is not shipped under assets/icons",
                        name[0], glyph[0]
                    );
                }
                mapped.insert(name[0].clone());
            }
        }
    }

    let mut files = vec![root.join("shell.qml")];
    qml_files(&root.join("ui"), &mut files);
    qml_files(&root.join("console"), &mut files);
    let mut missing = Vec::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            let mut names = bound_names(line, "iconName:");
            names.extend(bound_names(line, "symbol:"));
            if line.contains("SIcon") {
                names.extend(bound_names(line, "name:"));
            }
            for name in names {
                if !mapped.contains(&name.to_lowercase()) {
                    missing.push(format!(
                        "{}:{}: '{}'",
                        file.strip_prefix(&root).unwrap_or(file).display(),
                        number + 1,
                        name
                    ));
                }
            }
        }
    }
    println!("cargo:rerun-if-changed={}", icon_source.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("assets/icons").display()
    );
    if !missing.is_empty() {
        panic!(
            "icon names without a glyph in ui/SIcon.qml:\n  {}",
            missing.join("\n  ")
        );
    }
}
