//! The kitchen plan's local draft: the kernel's draft input handed to the
//! small local decision model, the dishes it returns read back for his review.
//!
//! Nothing here runs unless he has turned on both the local model and the
//! kitchen draft. While either is off `draft` returns before it reads a file,
//! starts a process or opens a socket. When on, the model is addressed only at
//! the port the stratum's serve script gives its decision role; the chat
//! gateway's port is refused.

use crate::{Error, Result};
use serde_json::{Value, json};
use soliton_kernel::{ObjectId, ledger::kitchen::DishSpec};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The longest a draft may take, from connect to the last byte.
pub const DRAFT_CAP: Duration = Duration::from_secs(60);
/// The largest answer read back.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;
/// The fixed instruction sent with every draft.
pub const INSTRUCTION: &str = "You plan one batch cook. The user message is JSON: the day, coverage_days \
(how many days of meals the cook must cover), stock (lots with subject, quantity, unit and the day each goes off), \
equipment (name, count, capacity) and profile (standing constraints: diet, exclusions, targets, portions per week, \
ingredients he wants used). Plan dishes a Michelin-trained chef would serve, adapted to this stock and these \
constraints: use what goes off first, never an excluded ingredient, only the listed equipment, enough portions \
for the coverage. Answer with JSON only: {\"dishes\":[{\"name\":text,\"portions\":number,\"phase\":small whole \
number,\"equipment\":[names from the list],\"uses\":[{\"lot\":a lot id from the stock,\"qty\":number in the lot's \
unit}]}]}. If the constraints cannot be met, answer {\"refusal\":\"one plain sentence why\"}.";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// His two switches and where the stratum's serve script is.
#[derive(Clone, Debug)]
pub struct DraftSettings {
    /// The workspace setting `localModel`: the master switch for every model path.
    pub local_model: bool,
    /// The workspace setting `kitchenDraft`, off by default.
    pub kitchen_draft: bool,
    /// The stratum's serve script, read (never run) for the decision role's port.
    pub serve_script: PathBuf,
}

impl DraftSettings {
    /// The stratum's serve script.
    #[must_use]
    pub fn serve_script() -> PathBuf {
        let home = std::env::var_os("HOME").unwrap_or_default();
        Path::new(&home).join("80_stratum/models/serve/llm.sh")
    }
}

/// The decision role as the serve script declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRole {
    /// The model file's name without its extension: the draft rule's model id.
    pub model_id: String,
    pub alias: String,
    pub port: u16,
}

/// Reads the serve script's role table: the first role whose model lives under
/// `decision/`. Callers reach a role on the loopback at its port (a role bound
/// to another loopback address is fronted there by its socket unit). A role on
/// the script's gateway port is refused.
pub fn decision_role(script: &Path) -> Result<DecisionRole> {
    let text = std::fs::read_to_string(script)
        .map_err(|_| invalid("The local decision model is not set up"))?;
    let gateway = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("GATEWAY_PORT=")
            .and_then(|port| port.trim().parse::<u16>().ok())
    });
    for line in text.lines() {
        let line = line.trim();
        let Some(entry) = line.strip_prefix('"').and_then(|l| l.split('"').next()) else {
            continue;
        };
        let fields: Vec<&str> = entry.split('|').collect();
        if fields.len() < 5 || !fields[1].starts_with("decision/") {
            continue;
        }
        let port: u16 = fields[2]
            .rsplit(':')
            .next()
            .and_then(|port| port.parse().ok())
            .ok_or_else(|| invalid("The local decision model has no port"))?;
        if Some(port) == gateway {
            return Err(invalid(
                "The local decision model shares the chat gateway's port",
            ));
        }
        let model_id = Path::new(fields[1])
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("decision")
            .to_owned();
        return Ok(DecisionRole {
            model_id,
            alias: fields[4].to_owned(),
            port,
        });
    }
    Err(invalid("The local decision model is not set up"))
}

/// The rule a drafted plan is written under.
#[must_use]
pub fn rule(role: &DecisionRole) -> String {
    format!("local-draft:{}", role.model_id)
}

/// Drafts the dishes of one plan from the kernel's draft input. Off unless
/// both switches are on; then one bounded request to the decision role.
pub fn draft(input: &Value, settings: &DraftSettings) -> Result<(Vec<DishSpec>, DecisionRole)> {
    if !(settings.local_model && settings.kitchen_draft) {
        return Err(invalid("Drafting is off"));
    }
    let role = decision_role(&settings.serve_script)?;
    let body = request_body(input, &role);
    let answer = post(role.port, &body)?;
    Ok((dishes_from_answer(&answer)?, role))
}

/// The request: the fixed instruction and the draft input, as JSON.
#[must_use]
pub fn request_body(input: &Value, role: &DecisionRole) -> Value {
    json!({
        "model": role.alias,
        "messages": [
            {"role": "system", "content": INSTRUCTION},
            {"role": "user", "content": input.to_string()},
        ],
        "temperature": 0.2,
        "max_tokens": 4096,
        "stream": false,
        "response_format": {"type": "json_object"},
    })
}

fn post(port: u16, body: &Value) -> Result<Value> {
    let unreachable = || invalid("The local decision model did not answer");
    let started = Instant::now();
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, DRAFT_CAP).map_err(|_| unreachable())?;
    stream.set_write_timeout(Some(DRAFT_CAP))?;
    let payload = body.to_string();
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| unreachable())?;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let left = DRAFT_CAP
            .checked_sub(started.elapsed())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| invalid("The local decision model took longer than a minute"))?;
        stream.set_read_timeout(Some(left))?;
        let read = stream
            .read(&mut buffer)
            .map_err(|_| invalid("The local decision model took longer than a minute"))?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.len() > MAX_ANSWER_BYTES {
            return Err(invalid("The local decision model's answer is too long"));
        }
    }
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(unreachable)?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    if !head.starts_with("http/1.1 200") && !head.starts_with("http/1.0 200") {
        return Err(unreachable());
    }
    let mut payload = raw[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload).ok_or_else(unreachable)?;
    }
    serde_json::from_slice(&payload).map_err(|_| unreachable())
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = body.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&body[..end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        body = &body[end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

/// The dishes in a chat completion's answer, or the model's refusal in its words.
pub fn dishes_from_answer(answer: &Value) -> Result<Vec<DishSpec>> {
    let content = answer["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| invalid("The local decision model gave no plan"))?
        .trim();
    let content = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))
        .map_or(content, |inner| inner.trim_end().trim_end_matches("```"))
        .trim();
    let parsed: Value = serde_json::from_str(content).map_err(|_| refusal(content))?;
    if let Some(reason) = parsed["refusal"].as_str() {
        return Err(refusal(reason));
    }
    match parsed["dishes"].as_array() {
        Some(dishes) if !dishes.is_empty() => dishes_from(&parsed["dishes"]),
        _ => Err(refusal(content)),
    }
}

fn refusal(text: &str) -> Error {
    let text: String = text.trim().chars().take(300).collect();
    if text.is_empty() {
        invalid("The local decision model gave no plan")
    } else {
        invalid(text)
    }
}

/// Dishes as the window and the model write them: `[{"name", "portions",
/// "phase"?, "equipment"?: [names], "uses"?: [{"lot", "qty"}]}]`.
pub fn dishes_from(value: &Value) -> Result<Vec<DishSpec>> {
    let mut dishes = Vec::new();
    for dish in value.as_array().map(Vec::as_slice).unwrap_or_default() {
        let mut uses = Vec::new();
        for used in dish["uses"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let lot: ObjectId = used["lot"]
                .as_str()
                .ok_or_else(|| invalid("A dish's use names its lot"))?
                .parse()?;
            let qty = used["qty"]
                .as_f64()
                .ok_or_else(|| invalid("A dish's use gives its quantity"))?;
            uses.push((lot, qty));
        }
        dishes.push(DishSpec {
            name: dish["name"].as_str().unwrap_or_default().to_owned(),
            portions: dish["portions"]
                .as_f64()
                .ok_or_else(|| invalid("A dish gives its portions"))?,
            phase: u32::try_from(dish["phase"].as_u64().unwrap_or(0))
                .map_err(|_| invalid("A phase is a small whole number"))?,
            equipment: dish["equipment"]
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            uses,
        });
    }
    Ok(dishes)
}
