use serde_json::{Value, json};
use soliton::service::Service;
use std::{
    io::{self, BufRead, Read, Write},
    path::Path,
};
const USAGE: &str = "Usage: soliton DATA_DIRECTORY\n\nServes one existing Soliton workspace over standard input and output.\n  --help     print this text\n  --version  print the version and the release this build belongs to";

fn main() {
    // Answered before anything touches the disk; the engine never creates a workspace.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return;
        }
        [flag] if flag == "--version" || flag == "-V" => {
            let record = soliton::service::release_record();
            let name = record["release"]
                .as_str()
                .filter(|n| !n.is_empty())
                .unwrap_or("unreleased");
            println!(
                "soliton {} ({name}) workspace format {}",
                env!("CARGO_PKG_VERSION"),
                soliton::service::WORKSPACE_FORMAT
            );
            return;
        }
        [path] if !path.starts_with('-') => path.clone(),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2)
        }
    };
    if !Path::new(&path).is_dir() {
        eprintln!("No Soliton workspace at {path}");
        std::process::exit(1)
    }
    let mut service = match Service::open(Path::new(&path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    };
    // One bounded reader keeps the single writer responsive to scheduled work
    // even when no UI command arrives. A closed input channel still ends the service.
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            let mut bytes = Vec::new();
            let Ok(n) = Read::by_ref(&mut input)
                .take(2_000_002)
                .read_until(b'\n', &mut bytes)
            else {
                break;
            };
            if n == 0 {
                break;
            }
            if bytes.len() > 2_000_000 && bytes.last() != Some(&b'\n') {
                while let Ok(part) = input.fill_buf() {
                    if part.is_empty() {
                        break;
                    }
                    let end = part.iter().position(|b| *b == b'\n');
                    let count = end.map_or(part.len(), |i| i + 1);
                    input.consume(count);
                    if end.is_some() {
                        break;
                    }
                }
            }
            if sender.send(bytes).is_err() {
                break;
            }
        }
    });
    let mut out = io::stdout().lock();
    let mut last_background_error = String::new();
    loop {
        if let Err(error) = service.background_tick() {
            let message = error.to_string();
            if message != last_background_error {
                eprintln!("Background collection: {message}");
                last_background_error = message;
            }
        }
        if let Ok(Some(state)) = service.check_external_freshness() {
            let _ = writeln!(out, "{}", json!({"event":"state","ok":true,"state":state}));
            let _ = out.flush();
        }
        let bytes = match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(bytes) => bytes,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if bytes.len() > 2_000_000 {
            let _ = writeln!(
                out,
                "{}",
                json!({"ok":false,"error":"Request exceeds size limit"})
            );
            let _ = out.flush();
            continue;
        }
        let line = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                let _ = writeln!(
                    out,
                    "{}",
                    json!({"ok":false,"error":"Request must be UTF-8"})
                );
                let _ = out.flush();
                continue;
            }
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok":false,"error":e.to_string()}));
                continue;
            }
        };
        let command = request["command"].as_str().unwrap_or("");
        if command == "shutdown" {
            let _ = writeln!(out, "{}", json!({"id":request["id"],"ok":true,"result":{}}));
            let _ = out.flush();
            break;
        }
        let result = service.command(command, &request["args"]);
        let reply = match result {
            Ok(r) => {
                json!({"id":request["id"],"ok":true,"result":r,"state":if matches!(command,"reorder"|"save"|"complete"|"occurrence"|"restore"|"promote"|"duplicate"|"preferences"|"proposal-create"|"proposal-revise"|"proposal-decide"|"planner-trash"|"planner-trash-restore"){service.state().ok()}else{None}})
            }
            Err(e) => json!({"id":request["id"],"ok":false,"error":e.to_string()}),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
