//! One-monitor fallback applied by the compositor itself.
//!
//! A `Fallback` is recorded per set of active outputs (its key) while that
//! state is live: the surviving output, the workspaces chosen to stay on it,
//! and the rule that every other regular workspace, with its windows, moves to
//! the survivor. Soliton writes the records as a Lua block in the user's
//! `monitors.lua`, between `-- BEGIN SOLITON FALLBACK` and
//! `-- END SOLITON FALLBACK`. The block's handlers for `monitor.removed`,
//! `monitor.added` and `monitor.layout_changed` compute the active-output key
//! from the compositor's monitor list and apply the matching record with the
//! workspace dispatcher; when no record matches (both outputs back), they
//! return every displaced workspace to the output it was on before. Applying
//! needs no Soliton process: the compositor runs the handlers.
//!
//! Block form: the first line after the start marker is `-- ` plus the JSON
//! list of records (the only state Soliton reads back); the Lua that follows
//! is generated from it and is never parsed. Bytes outside the markers are
//! never changed.
//!
//! An output is present when the compositor lists it, not disabled and not a
//! mirror; its power state does not count. A screen asleep (DPMS off, blanked
//! by idle) keeps its place and its workspaces, so a sleep or a wake moves
//! nothing; only a disabled or removed output is gone.
//!
//! `decide` is the same decision in Rust, for tests and the page's preview;
//! `restore` is the return decision given the remembered homes.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub const START: &str = "-- BEGIN SOLITON FALLBACK\n";
pub const END: &str = "-- END SOLITON FALLBACK\n";
const MAX_FALLBACKS: usize = 64;
const MAX_STAY: usize = 64;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// One recorded one-output arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fallback {
    /// The active outputs this record applies to, sorted.
    pub key: Vec<String>,
    /// The output every other workspace moves to.
    pub survivor: String,
    /// The chosen workspaces that stay on the survivor; the first is its default.
    pub stay: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    /// Present: listed, not disabled, not a mirror, with a size.
    pub active: bool,
    /// Blanked by its power state; shown to him, never part of presence.
    pub asleep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub monitor: String,
    pub windows: u64,
}

/// The compositor's state as `hyprctl -j monitors all`, `workspaces` read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub outputs: Vec<Output>,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub workspace: i64,
    pub name: String,
    pub from: String,
    pub to: String,
    pub windows: u64,
}

impl Snapshot {
    /// Reads `{"monitors": [...], "workspaces": [...]}` in hyprctl's JSON shape.
    pub fn from_value(value: &Value) -> Result<Self> {
        let list = |key: &str| {
            value[key]
                .as_array()
                .ok_or_else(|| invalid("The display state could not be read"))
        };
        let outputs = list("monitors")?
            .iter()
            .map(|m| {
                let name = m["name"].as_str().unwrap_or_default().to_string();
                let mirror = m["mirrorOf"].as_str().unwrap_or("none");
                let active = !name.is_empty()
                    && !m["disabled"].as_bool().unwrap_or(false)
                    && matches!(mirror, "" | "none")
                    && m["width"].as_u64().unwrap_or(0) > 0
                    && m["height"].as_u64().unwrap_or(0) > 0;
                let asleep = active && !m["dpmsStatus"].as_bool().unwrap_or(true);
                Output {
                    name,
                    active,
                    asleep,
                }
            })
            .collect();
        let workspaces = list("workspaces")?
            .iter()
            .map(|w| Workspace {
                id: w["id"].as_i64().unwrap_or(0),
                name: w["name"].as_str().unwrap_or_default().to_string(),
                monitor: w["monitor"].as_str().unwrap_or_default().to_string(),
                windows: w["windows"].as_u64().unwrap_or(0),
            })
            .collect();
        Ok(Snapshot {
            outputs,
            workspaces,
        })
    }

    /// The sorted names of the outputs that are present, asleep or awake.
    pub fn active_key(&self) -> Vec<String> {
        let mut key: Vec<String> = self
            .outputs
            .iter()
            .filter(|o| o.active)
            .map(|o| o.name.clone())
            .collect();
        key.sort();
        key.dedup();
        key
    }
}

fn safe_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value.starts_with("special:")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        return Err(invalid("Choose a regular screen and workspace"));
    }
    Ok(())
}

fn validate(fallbacks: &[Fallback]) -> Result<()> {
    if fallbacks.len() > MAX_FALLBACKS {
        return Err(invalid("Too many screen arrangements"));
    }
    for f in fallbacks {
        if f.key.is_empty() || !f.key.contains(&f.survivor) || f.stay.len() > MAX_STAY {
            return Err(invalid("A screen arrangement is incomplete"));
        }
        for name in f.key.iter().chain(&f.stay).chain([&f.survivor]) {
            safe_name(name)?;
        }
    }
    Ok(())
}

/// A one-output arrangement is recorded only while that state is live: the
/// output is on and no other output is present and on (T747-001).
pub fn can_record(snapshot: &Snapshot, output: &str) -> Result<()> {
    if !snapshot
        .outputs
        .iter()
        .any(|o| o.active && o.name == output)
    {
        return Err(invalid("That screen is off"));
    }
    if snapshot
        .outputs
        .iter()
        .any(|o| o.active && o.name != output)
    {
        return Err(invalid(
            "Turn the other screen off first; a one-screen arrangement is set while it is off",
        ));
    }
    Ok(())
}

/// The moves the compositor's handlers make for this snapshot: under a
/// matching record, every regular workspace not on the survivor goes to it.
pub fn decide(snapshot: &Snapshot, fallbacks: &[Fallback]) -> Vec<Move> {
    let key = snapshot.active_key();
    let Some(chosen) = fallbacks.iter().find(|f| f.key == key) else {
        return Vec::new();
    };
    let mut moves: Vec<Move> = snapshot
        .workspaces
        .iter()
        .filter(|w| w.id > 0 && w.monitor != chosen.survivor)
        .map(|w| Move {
            workspace: w.id,
            name: w.name.clone(),
            from: w.monitor.clone(),
            to: chosen.survivor.clone(),
            windows: w.windows,
        })
        .collect();
    moves.sort_by_key(|m| m.workspace);
    moves
}

/// The return moves once no record matches: each displaced workspace goes
/// back to its remembered home when that output is on again.
pub fn restore(
    snapshot: &Snapshot,
    fallbacks: &[Fallback],
    homes: &BTreeMap<String, String>,
) -> Vec<Move> {
    let key = snapshot.active_key();
    if fallbacks.iter().any(|f| f.key == key) {
        return Vec::new();
    }
    let mut moves: Vec<Move> = snapshot
        .workspaces
        .iter()
        .filter_map(|w| {
            let home = homes.get(&w.id.to_string())?;
            (w.id > 0 && key.contains(home) && &w.monitor != home).then(|| Move {
                workspace: w.id,
                name: w.name.clone(),
                from: w.monitor.clone(),
                to: home.clone(),
                windows: w.windows,
            })
        })
        .collect();
    moves.sort_by_key(|m| m.workspace);
    moves
}

/// Reads the records from a `monitors.lua` source; no block means none.
pub fn read(source: &str) -> Result<Vec<Fallback>> {
    if !source.contains(START) && !source.contains(END) {
        return Ok(Vec::new());
    }
    let (Some(start), Some(end)) = (source.find(START), source.find(END)) else {
        return Err(invalid(
            "The screen arrangement block is damaged; nothing was changed",
        ));
    };
    if source.matches(START).count() != 1 || source.matches(END).count() != 1 || end < start {
        return Err(invalid(
            "The screen arrangement block is damaged; nothing was changed",
        ));
    }
    let block = &source[start + START.len()..end];
    let meta = block
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("-- "))
        .ok_or_else(|| invalid("The screen arrangement block is damaged; nothing was changed"))?;
    let fallbacks: Vec<Fallback> = serde_json::from_str(meta)?;
    validate(&fallbacks)?;
    Ok(fallbacks)
}

fn lua_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
    format!("{{ {} }}", quoted.join(", "))
}

const HANDLERS: &str = r#"do
  -- Generated by Soliton from the line above; edit the arrangement in Soliton.
  local fallbacks = {
{RECORDS}  }
  local state = _G.soliton_fallback or { homes = {}, displaced = {}, rules = {}, subscriptions = {} }
  _G.soliton_fallback = state
  for _, subscription in ipairs(state.subscriptions) do
    pcall(function() subscription:remove() end)
  end
  state.subscriptions = {}
  local busy = false

  local function active_outputs()
    local active, names = {}, {}
    for _, monitor in ipairs(hl.get_monitors()) do
      if not monitor.disabled and not monitor.is_mirror and (monitor.width or 0) > 0 and (monitor.height or 0) > 0 then
        active[monitor.name] = monitor
        names[#names + 1] = monitor.name
      end
    end
    table.sort(names)
    return active, table.concat(names, "+")
  end

  local function move(workspace, monitor)
    local ok = pcall(function()
      hl.dispatch(hl.dsp.workspace.move({ workspace = workspace, monitor = monitor }))
    end)
    if not ok then
      hl.dispatch(hl.dsp.workspace.move({ workspace = tostring(workspace.id), monitor = monitor.name }))
    end
  end

  local function clear_rules()
    for _, rule in ipairs(state.rules) do
      pcall(function() rule:set_enabled(false) end)
    end
    state.rules = {}
  end

  local function set_rules(chosen)
    clear_rules()
    for index, name in ipairs(chosen.stay) do
      local ok, rule = pcall(hl.workspace_rule, { workspace = name, monitor = chosen.survivor, default = index == 1 })
      if ok and rule then state.rules[#state.rules + 1] = rule end
    end
  end

  local function reconcile()
    if busy then return end
    busy = true
    local ok, problem = pcall(function()
      local active, key = active_outputs()
      local chosen
      for _, fallback in ipairs(fallbacks) do
        if fallback.key == key then chosen = fallback end
      end
      if chosen then
        local survivor = active[chosen.survivor]
        if not survivor then return end
        set_rules(chosen)
        for _, workspace in ipairs(hl.get_workspaces()) do
          if not workspace.special and workspace.id > 0 then
            local id = tostring(workspace.id)
            local current = workspace.monitor and workspace.monitor.name or ""
            if state.homes[id] == nil and current ~= "" then state.homes[id] = current end
            if current ~= chosen.survivor then
              state.displaced[id] = true
              move(workspace, survivor)
            end
          end
        end
        for id, home in pairs(state.homes) do
          if not active[home] then state.displaced[id] = true end
        end
      else
        clear_rules()
        for _, workspace in ipairs(hl.get_workspaces()) do
          local id = tostring(workspace.id)
          local home = state.homes[id]
          local current = workspace.monitor and workspace.monitor.name or ""
          if state.displaced[id] and home and active[home] and current ~= home then
            move(workspace, active[home])
          end
        end
        state.displaced = {}
        for _, workspace in ipairs(hl.get_workspaces()) do
          if not workspace.special and workspace.id > 0 and workspace.monitor and active[workspace.monitor.name] then
            state.homes[tostring(workspace.id)] = workspace.monitor.name
          end
        end
      end
    end)
    busy = false
    if not ok then error(problem) end
  end

  state.subscriptions = {
    hl.on("monitor.removed", reconcile),
    hl.on("monitor.added", reconcile),
    hl.on("monitor.layout_changed", reconcile),
  }
  pcall(reconcile)
end
"#;

/// Renders the whole block, markers included.
pub fn render_handlers(fallbacks: &[Fallback]) -> Result<String> {
    validate(fallbacks)?;
    let records: String = fallbacks
        .iter()
        .map(|f| {
            format!(
                "    {{ key = \"{}\", survivor = \"{}\", stay = {} }},\n",
                f.key.join("+"),
                f.survivor,
                lua_list(&f.stay)
            )
        })
        .collect();
    let meta = serde_json::to_string(fallbacks)?;
    Ok(format!(
        "{START}-- {meta}\n{}{END}",
        HANDLERS.replace("{RECORDS}", &records)
    ))
}

/// Puts the block into `source`, replacing the old one; bytes outside the
/// markers are kept exactly. A damaged block is refused before any change.
pub fn replace(source: &str, fallbacks: &[Fallback]) -> Result<String> {
    read(source)?;
    let block = render_handlers(fallbacks)?;
    let Some(start) = source.find(START) else {
        let gap = if source.is_empty() || source.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        return Ok(format!("{source}{gap}{block}"));
    };
    let end = source
        .find(END)
        .map(|i| i + END.len())
        .unwrap_or(source.len());
    Ok(format!("{}{block}{}", &source[..start], &source[end..]))
}

/// Records the arrangement for the live one-output state into `source`.
pub fn record(
    source: &str,
    snapshot: &Snapshot,
    output: &str,
    stay: &[String],
) -> Result<(Fallback, String)> {
    can_record(snapshot, output)?;
    let mut stay_unique: Vec<String> = Vec::new();
    for name in stay {
        safe_name(name)?;
        if !stay_unique.contains(name) {
            stay_unique.push(name.clone());
        }
    }
    let fallback = Fallback {
        key: snapshot.active_key(),
        survivor: output.to_string(),
        stay: stay_unique,
    };
    let mut fallbacks: Vec<Fallback> = read(source)?
        .into_iter()
        .filter(|f| f.key != fallback.key)
        .collect();
    fallbacks.push(fallback.clone());
    fallbacks.sort_by(|a, b| a.key.cmp(&b.key));
    let updated = replace(source, &fallbacks)?;
    Ok((fallback, updated))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn load(config: &Path) -> Result<String> {
    match fs::read_to_string(config) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// `desktop-fallback-read`, `desktop-fallback-record`, `desktop-fallback-preview`.
/// `args.snapshot` is the compositor's state; `config` is the `monitors.lua`
/// the block lives in (written only by record, atomically).
pub fn command(name: &str, args: &Value, config: &Path) -> Result<Value> {
    let snapshot = Snapshot::from_value(&args["snapshot"])?;
    let source = load(config)?;
    let fallbacks = read(&source)?;
    match name {
        "desktop-fallback-read" => {
            let key = snapshot.active_key();
            let recordable = key.len() == 1;
            let asleep: Vec<&str> = snapshot
                .outputs
                .iter()
                .filter(|o| o.asleep)
                .map(|o| o.name.as_str())
                .collect();
            Ok(json!({
                "fallbacks": fallbacks,
                "active": key,
                "asleep": asleep,
                "offered": key,
                "canRecord": recordable,
                "current": fallbacks.iter().find(|f| f.key == key),
                "moves": decide(&snapshot, &fallbacks),
            }))
        }
        "desktop-fallback-preview" => {
            let output = args["output"].as_str().unwrap_or_default();
            let proposed = if output.is_empty() {
                fallbacks
            } else {
                can_record(&snapshot, output)?;
                record(&source, &snapshot, output, &strings(&args["stay"]))
                    .map(|(_, updated)| read(&updated))??
            };
            Ok(json!({"moves": decide(&snapshot, &proposed)}))
        }
        "desktop-fallback-record" => {
            let output = args["output"].as_str().unwrap_or_default();
            let (fallback, updated) = record(&source, &snapshot, output, &strings(&args["stay"]))?;
            let temporary = config.with_extension("lua.soliton-fallback");
            fs::write(&temporary, &updated)?;
            fs::rename(&temporary, config)?;
            Ok(json!({"recorded": fallback, "fallbacks": read(&updated)?}))
        }
        _ => Err(invalid("Unknown command")),
    }
}
