//! The desktop as the engine reads and changes it.
//!
//! The outputs, workspaces and bindings come from the compositor (`hyprctl -j`),
//! or from a recorded state when `SOLITON_DESKTOP_RECORD` names one. A binding
//! resolves to its literal `o.bind` line in the desktop's defaults or the
//! user's override folder; only such a binding is editable. A changed binding
//! is written where it lives: the previous bytes of the user's `bindings.lua`
//! are kept beside it, an unbind of the old chord and a bind of the new one
//! with the same action are appended, and the compositor reloads when `live`.
//!
//! Commands: `desktop-monitors`, `desktop-bindings`, `desktop-binding-set`,
//! `desktop-arrangement-preview|apply|confirm` (through `desktop_apply`) and
//! `desktop-fallback-read|preview|record` (through `desktop_fallback`).
use crate::{Error, Result, desktop_apply, desktop_fallback};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// The override file the user's bindings live in, inside the config dir.
pub const BINDINGS_FILE: &str = "bindings.lua";
/// The exact bytes `bindings.lua` held before the last binding change.
pub const BINDINGS_PREVIOUS: &str = ".soliton-bindings.previous";
/// The desktop's own defaults, read for literal binding sources.
pub const DEFAULTS_DIR: &str = "/usr/share/omarchy/default/hypr";

const MODIFIERS: [(u64, &str); 4] = [(64, "SUPER"), (8, "ALT"), (4, "CTRL"), (1, "SHIFT")];
const MAX_FILE_BYTES: u64 = 1024 * 1024;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// Where the desktop is read and written.
#[derive(Debug, Clone)]
pub struct Desktop {
    /// The user's compositor configuration folder.
    pub config: PathBuf,
    /// The desktop's default configuration folder (read only).
    pub defaults: PathBuf,
    /// Reload the compositor after a write.
    pub live: bool,
    /// A recorded compositor state used in place of `hyprctl`.
    pub record: Option<PathBuf>,
}

impl Desktop {
    /// `SOLITON_SETTINGS_CONFIG` (else `~/.config/hypr`), `SOLITON_SETTINGS_LIVE=1`,
    /// `SOLITON_DESKTOP_RECORD`, `SOLITON_DESKTOP_DEFAULTS`.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Desktop {
            config: var("SOLITON_SETTINGS_CONFIG").unwrap_or_else(|| home.join(".config/hypr")),
            defaults: var("SOLITON_DESKTOP_DEFAULTS")
                .unwrap_or_else(|| PathBuf::from(DEFAULTS_DIR)),
            live: std::env::var("SOLITON_SETTINGS_LIVE").is_ok_and(|v| v == "1"),
            record: var("SOLITON_DESKTOP_RECORD"),
        }
    }

    /// One compositor reading: `monitors`, `workspaces`, `binds` or `devices`.
    fn observe(&self, what: &str) -> Result<Value> {
        if let Some(record) = &self.record {
            let value: Value = serde_json::from_slice(&fs::read(record)?)?;
            return Ok(match (what, value.get(what)) {
                (_, Some(found)) => found.clone(),
                ("devices", None) => json!({}),
                (_, None) => json!([]),
            });
        }
        let args: &[&str] = match what {
            "monitors" => &["-j", "monitors", "all"],
            "workspaces" => &["-j", "workspaces"],
            "binds" => &["-j", "binds"],
            "devices" => &["-j", "devices"],
            _ => return Err(invalid("Unknown desktop reading")),
        };
        let output = Command::new("hyprctl").args(args).output()?;
        if !output.status.success() {
            return Err(invalid("The compositor could not be read"));
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// A user file inside the config folder, refused when it is a link.
    fn file(&self, name: &str) -> Result<PathBuf> {
        let path = self.config.join(name);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(invalid("The configuration file is a link; nothing written"));
        }
        Ok(path)
    }

    /// Writes refused when not live and the folder is the user's real one:
    /// a candidate or a test changes a copy, never his desktop by accident.
    fn may_write(&self) -> Result<()> {
        if self.live {
            return Ok(());
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let real = fs::canonicalize(home.join(".config/hypr")).ok();
        if real.is_some() && fs::canonicalize(&self.config).ok() == real {
            return Err(invalid("System changes are disabled in this candidate"));
        }
        Ok(())
    }

    /// The outputs and workspaces as the page reads them, with the revision of
    /// `monitors.lua` a change must carry and the recorded one-output fallbacks.
    pub fn monitors(&self) -> Result<Value> {
        let all = self.observe("monitors")?;
        let workspaces = self.observe("workspaces")?;
        let items: Vec<Value> = all
            .as_array()
            .map(|rows| rows.iter().filter(|row| is_active(row)).cloned().collect())
            .unwrap_or_default();
        let mut known: Vec<String> = all
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let mut configured = Vec::new();
        for path in lua_files(&self.config) {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (number, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("--") || !line.contains("hl.workspace_rule") {
                    continue;
                }
                if let (Some(workspace), Some(monitor)) =
                    (field(line, "workspace"), field(line, "monitor"))
                {
                    known.push(monitor.clone());
                    configured.push(
                        json!({"workspace": workspace, "monitor": monitor, "line": number + 1}),
                    );
                }
            }
        }
        known.sort();
        known.dedup();
        let file = read_small(&self.file(desktop_apply::CONFIG_FILE)?)?;
        let snapshot = desktop_apply::Snapshot {
            outputs: observed_outputs(&all),
            file: file.clone(),
        };
        let source = String::from_utf8_lossy(&file);
        Ok(json!({
            "items": items,
            "allItems": all,
            "activeKey": snapshot.topology_key(),
            "knownOutputs": known,
            "configuredWorkspaces": configured,
            "workspaces": workspaces,
            "fallbacks": desktop_fallback::read(&source)?,
            "revision": snapshot.revision(),
            "liveApply": self.live,
        }))
    }

    /// Every binding the compositor holds, each with its chord, its literal
    /// source line when it has one, and whether it can be changed in place.
    pub fn bindings(&self) -> Result<Value> {
        let rows = self.observe("binds")?;
        let sources = self.binding_sources();
        let items: Vec<Value> = rows
            .as_array()
            .map(|rows| rows.iter().map(|row| binding_row(row, &sources)).collect())
            .unwrap_or_default();
        let keyboards = self.observe("devices")?["keyboards"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let keyboard = keyboards
            .iter()
            .find(|k| k["main"].as_bool() == Some(true))
            .or(keyboards.first())
            .cloned()
            .unwrap_or_else(|| json!({}));
        Ok(json!({
            "items": items,
            "revision": digest(&read_small(&self.file(BINDINGS_FILE)?)?),
            "liveApply": self.live,
            "keyboard": keyboard,
        }))
    }

    /// Literal single-line `o.bind` sources, keyed by (chord parts, description);
    /// the user's folder is read after the defaults, so its lines win.
    fn binding_sources(&self) -> BTreeMap<(Vec<String>, String), Source> {
        let mut sources = BTreeMap::new();
        for root in [&self.defaults, &self.config] {
            for path in lua_files(root) {
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                for (number, line) in text.lines().enumerate() {
                    if let Some((combo, description, expression)) = parse_bind(line) {
                        sources.insert(
                            (chord_parts(&combo), description.clone()),
                            Source {
                                path: path.clone(),
                                line: number + 1,
                                expression,
                            },
                        );
                    }
                }
            }
        }
        sources
    }

    /// Changes one binding where it lives; see [`set_binding`].
    pub fn set_binding(&self, request: &BindingRequest) -> Result<BindingReceipt> {
        self.may_write()?;
        let _lock = lock(&self.config)?;
        let path = self.file(BINDINGS_FILE)?;
        let original = read_small(&path)?;
        if request.revision != digest(&original) {
            return Err(invalid("The bindings changed; review them again"));
        }
        let snapshot = self.bindings()?;
        let rows = snapshot["items"].as_array().cloned().unwrap_or_default();
        let old = chord_parts(&request.old);
        let row = rows
            .iter()
            .find(|r| {
                chord_parts(s(r, "combo")) == old && s(r, "description") == request.description
            })
            .filter(|r| r["editable"] == true)
            .ok_or_else(|| invalid("This binding has no safely editable source"))?;
        let new = normalize_chord(&request.new)?;
        let wanted = chord_parts(&new);
        if let Some(other) = rows.iter().find(|r| chord_parts(s(r, "combo")) == wanted) {
            return Err(invalid(format!(
                "{new} is already bound to {}",
                action_of(other)
            )));
        }
        let lines = [
            format!("hl.unbind({})", lua_string(s(row, "combo"))),
            format!(
                "o.bind({}, {}, {})",
                lua_string(&new),
                lua_string(&request.description),
                s(row, "expression")
            ),
        ];
        let mut updated = original.clone();
        if !updated.is_empty() && !updated.ends_with(b"\n") {
            updated.push(b'\n');
        }
        updated.extend_from_slice(format!("\n{}\n{}\n", lines[0], lines[1]).as_bytes());
        check_lua(&updated)?;
        let previous = self.config.join(BINDINGS_PREVIOUS);
        write_atomic(&previous, &original, None)?;
        write_atomic(&path, &updated, file_mode(&path))?;
        if self.live {
            let errors = reload_and_read_errors();
            if !errors.is_empty() {
                write_atomic(&path, &original, file_mode(&path))?;
                let _ = reload_and_read_errors();
                return Err(invalid(format!("Reverted: {}", errors.join("; "))));
            }
        }
        Ok(BindingReceipt {
            written: true,
            applied: self.live,
            summary: format!("{}: {} → {new}", request.description, s(row, "combo")),
            lines: lines.to_vec(),
            revision: digest(&read_small(&path)?),
            previous_revision: digest(&read_small(&previous)?),
        })
    }

    /// The arrangement request in either the page's map form
    /// (`outputs: {name: {mode, scale, x, y, transform}}`, `workspaces: {name: output}`),
    /// the list form of `desktop_apply::Request`, or one output's change
    /// (`name, scale, x, y, transform`) with the rest as observed.
    fn arrangement(
        &self,
        args: &Value,
    ) -> Result<(desktop_apply::Snapshot, desktop_apply::Request)> {
        let all = self.observe("monitors")?;
        let workspaces = self.observe("workspaces")?;
        let path = self.file(desktop_apply::CONFIG_FILE)?;
        let snapshot = desktop_apply::Snapshot {
            outputs: observed_outputs(&all),
            file: read_small(&path)?,
        };
        let active: Vec<&Value> = all
            .as_array()
            .map(|rows| rows.iter().filter(|r| is_active(r)).collect())
            .unwrap_or_default();
        let mut outputs: Vec<desktop_apply::OutputSetting> = match &args["outputs"] {
            Value::Array(_) => serde_json::from_value(args["outputs"].clone())?,
            Value::Object(map) => map
                .iter()
                .map(|(name, choice)| setting(name, choice))
                .collect::<Result<_>>()?,
            _ => active.iter().map(|row| observed_setting(row)).collect(),
        };
        if let Some(name) = args["name"].as_str() {
            let one = outputs
                .iter_mut()
                .find(|o| o.name == name)
                .ok_or_else(|| invalid("Select a connected, enabled display"))?;
            one.scale = number(&args["scale"]).unwrap_or(one.scale);
            one.x = number(&args["x"]).map_or(one.x, |v| v as i32);
            one.y = number(&args["y"]).map_or(one.y, |v| v as i32);
            one.transform = number(&args["transform"]).map_or(one.transform, |v| v as u8);
        }
        let assignments = match &args["workspaces"] {
            Value::Array(_) => serde_json::from_value(args["workspaces"].clone())?,
            Value::Object(map) => map
                .iter()
                .map(|(workspace, output)| desktop_apply::Assignment {
                    workspace: workspace.clone(),
                    output: output.as_str().unwrap_or_default().to_string(),
                })
                .collect(),
            _ => workspaces
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter(|w| w["id"].as_i64().unwrap_or(0) > 0)
                        .map(|w| desktop_apply::Assignment {
                            workspace: value_text(&w["name"]),
                            output: s(w, "monitor").to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let request = desktop_apply::Request {
            revision: s(args, "revision").to_string(),
            outputs,
            workspaces: assignments,
        };
        Ok((snapshot, request))
    }

    /// The fallback commands with the compositor's state filled in when the
    /// caller sends none.
    fn fallback(&self, command: &str, args: &Value) -> Result<Value> {
        if command == "desktop-fallback-record" {
            self.may_write()?;
        }
        let mut args = args.clone();
        if args["snapshot"].is_null() {
            let snapshot = json!({"monitors": self.observe("monitors")?, "workspaces": self.observe("workspaces")?});
            if let Value::Object(map) = &mut args {
                map.insert("snapshot".into(), snapshot);
            } else {
                args = json!({"snapshot": snapshot});
            }
        }
        desktop_fallback::command(command, &args, &self.file(desktop_apply::CONFIG_FILE)?)
    }

    /// One desktop command of the service's line protocol.
    pub fn command(&self, command: &str, args: &Value) -> Result<Value> {
        let refused = |e: desktop_apply::ApplyError| invalid(e.to_string());
        match command {
            "desktop-monitors" => self.monitors(),
            "desktop-bindings" => self.bindings(),
            "desktop-binding-set" => {
                let request: BindingRequest = serde_json::from_value(args.clone())
                    .map_err(|_| invalid("Name the binding, its action and the new chord"))?;
                Ok(serde_json::to_value(self.set_binding(&request)?)?)
            }
            "desktop-arrangement-preview" => {
                let (snapshot, request) = self.arrangement(args)?;
                let plan = desktop_apply::preview(&snapshot, &request).map_err(refused)?;
                Ok(json!({
                    "token": plan.token,
                    "summary": plan.summary,
                    "change": plan.block.trim_end(),
                    "revision": plan.revision,
                    "confirmSeconds": plan.confirm_seconds,
                    "liveApply": self.live,
                }))
            }
            "desktop-arrangement-apply" => {
                self.may_write()?;
                let (snapshot, request) = self.arrangement(args)?;
                let plan = desktop_apply::preview(&snapshot, &request).map_err(refused)?;
                let receipt =
                    desktop_apply::apply(&plan, &self.config, self.live).map_err(refused)?;
                Ok(serde_json::to_value(receipt)?)
            }
            "desktop-arrangement-confirm" => {
                let confirmation =
                    desktop_apply::confirm(&self.config, s(args, "token"), args["keep"] == true)
                        .map_err(refused)?;
                Ok(serde_json::to_value(confirmation)?)
            }
            "desktop-fallback-read" | "desktop-fallback-preview" | "desktop-fallback-record" => {
                self.fallback(command, args)
            }
            _ => Err(invalid("Unknown command")),
        }
    }
}

/// `desktop-*` commands against the desktop the environment names.
pub fn command(command: &str, args: &Value) -> Result<Value> {
    Desktop::from_env().command(command, args)
}

/// The change asked for: the binding by its chord and action, the new chord,
/// and the revision of `bindings.lua` the page last read.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BindingRequest {
    pub old: String,
    pub description: String,
    pub new: String,
    #[serde(default)]
    pub revision: String,
}

/// What a binding change did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BindingReceipt {
    /// The override file holds the change.
    pub written: bool,
    /// The compositor reloaded with no configuration errors.
    pub applied: bool,
    /// One line of plain words: action, old chord, new chord.
    pub summary: String,
    /// The two lines appended: the unbind and the bind.
    pub lines: Vec<String>,
    /// Hash of the override file's bytes now.
    pub revision: String,
    /// Hash of the kept previous bytes.
    pub previous_revision: String,
}

/// Changes one binding where it lives: refuses an unknown modifier, a chord
/// already bound anywhere (naming that binding's action), a binding with no
/// literal source and a changed revision; keeps the previous bytes in
/// `BINDINGS_PREVIOUS`; appends the unbind and the bind; reloads when `live`
/// and reads the compositor's configuration errors, reverting on any.
pub fn set_binding(
    request: &BindingRequest,
    config_dir: &Path,
    live: bool,
) -> Result<BindingReceipt> {
    Desktop {
        config: config_dir.to_path_buf(),
        live,
        ..Desktop::from_env()
    }
    .set_binding(request)
}

#[derive(Debug, Clone)]
struct Source {
    path: PathBuf,
    line: usize,
    expression: String,
}

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), String::from)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|t| t.trim().parse().ok()))
}

fn is_active(row: &Value) -> bool {
    !row["disabled"].as_bool().unwrap_or(false)
        && row["dpmsStatus"].as_bool().unwrap_or(true)
        && matches!(row["mirrorOf"].as_str().unwrap_or(""), "" | "none")
        && row["width"].as_u64().unwrap_or(0) > 0
        && row["height"].as_u64().unwrap_or(0) > 0
}

fn observed_outputs(all: &Value) -> Vec<desktop_apply::ObservedOutput> {
    all.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| serde_json::from_value(row.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The output's current setting, with the offered mode nearest its rate.
fn observed_setting(row: &Value) -> desktop_apply::OutputSetting {
    let prefix = format!("{}x{}@", row["width"], row["height"]);
    let rate = row["refreshRate"].as_f64().unwrap_or(0.0);
    let mode = row["availableModes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|m| m.starts_with(&prefix))
        .min_by(|a, b| {
            let off = |m: &str| {
                (m[prefix.len()..]
                    .trim_end_matches("Hz")
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    - rate)
                    .abs()
            };
            off(a).total_cmp(&off(b))
        })
        .map_or_else(|| format!("{prefix}{rate:.2}Hz"), String::from);
    desktop_apply::OutputSetting {
        name: s(row, "name").to_string(),
        mode,
        x: row["x"].as_i64().unwrap_or(0) as i32,
        y: row["y"].as_i64().unwrap_or(0) as i32,
        scale: row["scale"].as_f64().unwrap_or(1.0),
        transform: row["transform"].as_u64().unwrap_or(0) as u8,
    }
}

fn setting(name: &str, choice: &Value) -> Result<desktop_apply::OutputSetting> {
    let bad = || invalid("Invalid display arrangement");
    Ok(desktop_apply::OutputSetting {
        name: name.to_string(),
        mode: s(choice, "mode").to_string(),
        x: number(&choice["x"]).ok_or_else(bad)? as i32,
        y: number(&choice["y"]).ok_or_else(bad)? as i32,
        scale: number(&choice["scale"]).ok_or_else(bad)?,
        transform: number(&choice["transform"]).ok_or_else(bad)? as u8,
    })
}

/// `key = "value"` inside a Lua table line.
fn field(line: &str, key: &str) -> Option<String> {
    let mut rest = line;
    while let Some(at) = rest.find(key) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + key.len()..].trim_start();
        rest = &rest[at + key.len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(value) = after
            .strip_prefix('=')
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('"'))
        {
            return value.find('"').map(|end| value[..end].to_string());
        }
    }
    None
}

/// `o.bind("CHORD", "Description", expression)` on one line, as written.
fn parse_bind(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start().strip_prefix("o.bind(\"")?;
    let (combo, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(',')?.trim_start().strip_prefix('"')?;
    let (description, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(',')?.trim_start();
    let rest = rest.trim_end();
    let rest = rest.strip_suffix(';').map_or(rest, str::trim_end);
    let expression = rest.strip_suffix(')')?;
    (!combo.is_empty() && !expression.is_empty()).then(|| {
        (
            combo.to_string(),
            description.to_string(),
            expression.to_string(),
        )
    })
}

/// A chord's parts, upper-cased and sorted, so spelling order does not matter.
fn chord_parts(chord: &str) -> Vec<String> {
    let mut parts: Vec<String> = chord
        .split('+')
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
        .collect();
    parts.sort();
    parts
}

/// `SUPER + SHIFT + T` from any spelling; refuses an unknown modifier, a
/// repeated part or a key that is not a plain symbol.
fn normalize_chord(chord: &str) -> Result<String> {
    let parts: Vec<String> = chord.split('+').map(|p| p.trim().to_uppercase()).collect();
    let (key, modifiers) = parts.split_last().ok_or_else(|| invalid("Choose a key"))?;
    let mut seen = parts.clone();
    seen.sort();
    seen.dedup();
    if key.is_empty()
        || seen.len() != parts.len()
        || modifiers
            .iter()
            .any(|m| !MODIFIERS.iter().any(|(_, name)| name == m))
    {
        return Err(invalid(
            "Use modifiers and one key, for example SUPER + SHIFT + A",
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_:.-".contains(c))
    {
        return Err(invalid("Choose a plain key"));
    }
    Ok(parts.join(" + "))
}

/// The page's row for one compositor binding.
fn binding_row(row: &Value, sources: &BTreeMap<(Vec<String>, String), Source>) -> Value {
    let description = s(row, "description").to_string();
    let modmask = row["modmask"].as_u64().unwrap_or(0);
    let mods: Vec<&str> = MODIFIERS
        .iter()
        .filter(|(bit, _)| modmask & bit != 0)
        .map(|(_, n)| *n)
        .collect();
    let mut key = s(row, "key").to_string();
    if key.is_empty() {
        key = format!("code:{}", row["keycode"]);
        if row["keycode"].as_u64().unwrap_or(0) == 0 {
            // A keycode binding comes back without its key; the literal source
            // with the same action and modifiers names it.
            let mut wanted: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
            wanted.sort();
            let named = sources
                .keys()
                .find(|(parts, desc)| {
                    let mut others: Vec<String> = parts
                        .iter()
                        .filter(|p| MODIFIERS.iter().any(|(_, n)| n == *p))
                        .cloned()
                        .collect();
                    others.sort();
                    desc == &description && others == wanted
                })
                .and_then(|(parts, _)| {
                    parts
                        .iter()
                        .find(|p| !MODIFIERS.iter().any(|(_, n)| n == *p))
                });
            if let Some(found) = named {
                key = if found.starts_with("CODE:") {
                    found.to_lowercase()
                } else {
                    found.clone()
                };
            }
        }
    }
    let combo = mods
        .iter()
        .map(|m| m.to_string())
        .chain([key])
        .collect::<Vec<_>>()
        .join(" + ");
    let source = sources.get(&(chord_parts(&combo), description));
    let flagged = ["release", "repeat", "mouse", "non_consuming"]
        .iter()
        .any(|f| row[*f] == true);
    let mut out: Map<String, Value> = row.as_object().cloned().unwrap_or_default();
    out.insert("combo".into(), json!(combo));
    out.insert(
        "source".into(),
        json!(
            source
                .map(|s| s.path.display().to_string())
                .unwrap_or_default()
        ),
    );
    out.insert("line".into(), json!(source.map_or(0, |s| s.line)));
    out.insert(
        "expression".into(),
        json!(source.map(|s| s.expression.clone()).unwrap_or_default()),
    );
    out.insert(
        "editable".into(),
        json!(source.is_some() && s(row, "submap").is_empty() && !flagged),
    );
    Value::Object(out)
}

fn action_of(row: &Value) -> String {
    let description = s(row, "description");
    if !description.is_empty() {
        return description.to_string();
    }
    [s(row, "dispatcher"), s(row, "arg")]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn lua_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Every `.lua` file under `root`, links not followed, in name order.
fn lua_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "lua") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A missing file reads as empty; a large one is refused.
fn read_small(path: &Path) -> Result<Vec<u8>> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE_BYTES => {
            Err(invalid("The configuration file is too large"))
        }
        Ok(_) => Ok(fs::read(path)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The updated file still parses as Lua, when the checker is installed.
fn check_lua(bytes: &[u8]) -> Result<()> {
    let checker = Path::new("/usr/bin/luac");
    if !checker.exists() {
        return Ok(());
    }
    let mut child = Command::new(checker)
        .args(["-p", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(bytes))
        .transpose()?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(invalid("The change would not parse; nothing written"));
    }
    Ok(())
}

fn reload_and_read_errors() -> Vec<String> {
    match Command::new("hyprctl").arg("reload").output() {
        Ok(out) if out.status.success() => {}
        Ok(_) => return vec!["The compositor did not reload".into()],
        Err(e) => return vec![e.to_string()],
    }
    match Command::new("hyprctl").arg("configerrors").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        Err(e) => vec![e.to_string()],
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn lock(config_dir: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config_dir.join(desktop_apply::LOCK_FILE))?;
    file.lock()?;
    Ok(file)
}

fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("No folder for the settings file"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.{stamp}.tmp"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(mode.unwrap_or(0o600)))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    Ok(())
}
