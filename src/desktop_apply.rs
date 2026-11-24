//! The monitor arrangement written where the compositor reads it.
//!
//! One profile, keyed by the set of active outputs, is rendered into the
//! user's `monitors.lua` between Soliton's markers. Every byte outside the
//! markers is kept. An apply records the previous bytes beside the file and a
//! pending confirmation; the change stays only when it is kept inside the
//! confirmation window, otherwise the previous bytes come back exactly.
//!
//! The compositor is touched only when the caller passes `live`, and then only
//! to reload its configuration and to read its configuration errors. Nothing
//! here runs without a caller: a lapsed confirmation is restored by the next
//! `confirm` (the page's countdown, or the guard that outlives it).
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// First line of Soliton's block in `monitors.lua`.
pub const BEGIN_MARKER: &str = "-- BEGIN SOLITON DISPLAYS";
/// Last line of Soliton's block in `monitors.lua`.
pub const END_MARKER: &str = "-- END SOLITON DISPLAYS";
/// The file the compositor reads the arrangement from, inside the config dir.
pub const CONFIG_FILE: &str = "monitors.lua";
/// The exact bytes the file held before the last apply.
pub const PREVIOUS_FILE: &str = ".soliton-displays.previous";
/// The change waiting to be kept or reverted.
pub const PENDING_FILE: &str = ".soliton-displays.pending";
/// Shared with the settings bridge so both writers take turns.
pub const LOCK_FILE: &str = ".soliton-settings.lock";
/// Seconds a change waits for a keep before it is reverted.
pub const CONFIRM_SECONDS: u64 = 18;

const MAX_COORDINATE: i32 = 32768;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Why a preview, apply or confirm did not go ahead.
#[derive(Debug)]
pub enum ApplyError {
    /// Refused before anything was written; the reason is a short plain line.
    Refused(String),
    /// The compositor reported configuration errors; the change was reverted.
    Compositor(Vec<String>),
    /// Reading or writing a file failed.
    Io(io::Error),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => f.write_str(reason),
            Self::Compositor(lines) => write!(f, "Reverted: {}", lines.join("; ")),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<io::Error> for ApplyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ApplyError {
    fn from(error: serde_json::Error) -> Self {
        Self::Io(io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

pub type Result<T> = std::result::Result<T, ApplyError>;

fn refused(reason: &str) -> ApplyError {
    ApplyError::Refused(reason.to_string())
}

/// One output as the compositor reports it (`monitors all`, JSON form).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedOutput {
    pub name: String,
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub serial: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub refresh_rate: f64,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    #[serde(default)]
    pub transform: u8,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default = "yes")]
    pub dpms_status: bool,
    #[serde(default)]
    pub mirror_of: String,
    #[serde(default)]
    pub available_modes: Vec<String>,
}

fn yes() -> bool {
    true
}

impl ObservedOutput {
    /// Enabled, powered, not mirroring and with a real size.
    pub fn is_active(&self) -> bool {
        !self.disabled
            && self.dpms_status
            && matches!(self.mirror_of.as_str(), "" | "none")
            && self.width > 0
            && self.height > 0
    }

    fn identity(&self) -> String {
        [&self.make, &self.model, &self.serial, &self.name]
            .iter()
            .map(|part| part.replace('|', "/"))
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// What the preview reads: the outputs as observed and the file's bytes.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub outputs: Vec<ObservedOutput>,
    pub file: Vec<u8>,
}

impl Snapshot {
    /// The revision a request must carry: the hash of the file's bytes.
    pub fn revision(&self) -> String {
        digest(&self.file)
    }

    /// The key of the active topology: the sorted identities of the active
    /// outputs (make, model, serial, port), so a profile follows the screens.
    pub fn topology_key(&self) -> String {
        let mut identities: Vec<String> = self
            .outputs
            .iter()
            .filter(|output| output.is_active())
            .map(ObservedOutput::identity)
            .collect();
        identities.sort();
        identities.join("||")
    }
}

/// One output's chosen arrangement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputSetting {
    pub name: String,
    /// A mode the output offers, `WIDTHxHEIGHT@RATE` with or without `Hz`.
    pub mode: String,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// 0 normal, 1 to 3 turned 90, 180, 270 degrees, 4 to 7 the same flipped.
    pub transform: u8,
}

impl OutputSetting {
    fn pixels(&self) -> Option<(u32, u32)> {
        let (size, _) = self.mode.split_once('@')?;
        let (width, height) = size.split_once('x')?;
        Some((width.parse().ok()?, height.parse().ok()?))
    }

    /// The size the output takes in the layout: its mode over its scale,
    /// width and height swapped when it is turned a quarter.
    pub fn logical_size(&self) -> Option<(i64, i64)> {
        let (width, height) = self.pixels()?;
        let width = (f64::from(width) / self.scale).round() as i64;
        let height = (f64::from(height) / self.scale).round() as i64;
        Some(if self.transform % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        })
    }
}

/// One workspace placed on one output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub workspace: String,
    pub output: String,
}

/// The change asked for, against the revision the page last read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// The revision the page last read; a request without one is refused.
    #[serde(default)]
    pub revision: String,
    pub outputs: Vec<OutputSetting>,
    pub workspaces: Vec<Assignment>,
}

/// A reviewed change, ready to apply.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Names this change in apply and confirm; derived from the revision.
    pub token: String,
    /// One line of plain words saying what will change.
    pub summary: String,
    /// Hash of the file's bytes the preview was made over.
    pub revision: String,
    /// The active topology the profile is keyed by.
    pub topology_key: String,
    /// The rendered block, markers included.
    pub block: String,
    /// The file's bytes before the change.
    pub original: Vec<u8>,
    /// The file's bytes after the change.
    pub updated: Vec<u8>,
    /// Seconds the change waits for a keep.
    pub confirm_seconds: u64,
}

/// What an apply did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// The token confirm expects.
    pub token: String,
    /// The file now holds the change.
    pub written: bool,
    /// The compositor reloaded with no configuration errors.
    pub applied: bool,
    /// Hash of the file's bytes now.
    pub revision: String,
    /// Hash of the bytes kept in the previous-bytes copy.
    pub previous_revision: String,
    /// Where the previous bytes are kept.
    pub previous: PathBuf,
    /// Seconds left to keep the change.
    pub confirm_seconds: u64,
}

/// What a confirm did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The change stays.
    pub kept: bool,
    /// The previous bytes were written back.
    pub reverted: bool,
    /// The confirmation window had passed.
    pub lapsed: bool,
    /// Hash of the file's bytes now.
    pub revision: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    token: String,
    deadline_ms: u128,
    updated: String,
    previous: String,
    live: bool,
}

/// Render the profile block for one topology.
///
/// Returns the Lua text from `BEGIN_MARKER` to `END_MARKER` inclusive, ending
/// in a newline: the topology key as a comment, one `hl.monitor` line per
/// output (mode, position, scale, transform) and one `hl.workspace_rule` line
/// per assignment, outputs and workspaces in the order given.
pub fn render_profile(
    topology_key: &str,
    outputs: &[OutputSetting],
    workspaces: &[Assignment],
) -> String {
    let mut block = format!("{BEGIN_MARKER}\n-- profile {}\n", comment(topology_key));
    for output in outputs {
        block.push_str(&format!(
            "hl.monitor({{ output = {}, mode = {}, position = {}, scale = {}, transform = {} }})\n",
            lua_string(&output.name),
            lua_string(output.mode.trim_end_matches("Hz")),
            lua_string(&format!("{}x{}", output.x, output.y)),
            output.scale,
            output.transform
        ));
    }
    for assignment in workspaces {
        block.push_str(&format!(
            "hl.workspace_rule({{ workspace = {}, monitor = {} }})\n",
            lua_string(&assignment.workspace),
            lua_string(&assignment.output)
        ));
    }
    block.push_str(END_MARKER);
    block.push('\n');
    block
}

/// Put `block` in place of Soliton's block in `source`.
///
/// Returns the new text: every byte before the begin marker and after the end
/// marker's line is kept as it was; with no block yet, `block` is appended
/// after a blank line. Refuses when the markers are missing one half, repeated
/// or out of order, leaving the source untouched.
pub fn replace_block(source: &str, block: &str) -> Result<String> {
    match block_span(source)? {
        Some((start, end)) => Ok(format!("{}{block}{}", &source[..start], &source[end..])),
        None => {
            let mut text = source.to_string();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
            text.push_str(block);
            Ok(text)
        }
    }
}

/// Review a change against what is on screen and in the file.
///
/// Returns a `Plan` whose `summary` is one plain line and whose `token` is tied
/// to the file's revision. Refuses, with a short reason, a revision that is not
/// the file's, outputs that are not exactly the active ones, a mode the output
/// does not offer, a scale that leaves fractional pixels, an orientation
/// outside 0 to 7, screens that overlap, or a workspace placed twice or on an
/// output that is not active.
pub fn preview(snapshot: &Snapshot, request: &Request) -> Result<Plan> {
    let revision = snapshot.revision();
    if request.revision != revision {
        return Err(refused("The display settings changed; review them again"));
    }
    let active: Vec<&ObservedOutput> = snapshot.outputs.iter().filter(|o| o.is_active()).collect();
    if active.is_empty() {
        return Err(refused("No active display"));
    }
    let mut asked: Vec<&str> = request.outputs.iter().map(|o| o.name.as_str()).collect();
    let mut seen: Vec<&str> = active.iter().map(|o| o.name.as_str()).collect();
    asked.sort_unstable();
    seen.sort_unstable();
    if asked != seen {
        return Err(refused("Arrange exactly the active displays"));
    }
    for setting in &request.outputs {
        let observed = active.iter().find(|o| o.name == setting.name).unwrap();
        let offered = observed
            .available_modes
            .iter()
            .any(|mode| mode.trim_end_matches("Hz") == setting.mode.trim_end_matches("Hz"));
        if !offered {
            return Err(refused("Choose a mode this display offers"));
        }
        if !setting.scale.is_finite() || !(0.5..=4.0).contains(&setting.scale) {
            return Err(refused("Choose a scale from 0.5 to 4"));
        }
        let (width, height) = setting
            .pixels()
            .ok_or_else(|| refused("Choose a mode this display offers"))?;
        if [width, height].iter().any(|size| {
            let logical = f64::from(*size) / setting.scale;
            (logical - logical.round()).abs() > 0.01
        }) {
            return Err(refused("That scale leaves part of a pixel"));
        }
        if setting.transform > 7 {
            return Err(refused("Choose an orientation"));
        }
        if setting.x.abs() > MAX_COORDINATE || setting.y.abs() > MAX_COORDINATE {
            return Err(refused("A display is placed too far away"));
        }
    }
    for (index, first) in request.outputs.iter().enumerate() {
        for second in &request.outputs[index + 1..] {
            if overlaps(first, second) {
                return Err(refused("Two displays overlap"));
            }
        }
    }
    let mut workspaces: Vec<&str> = Vec::new();
    for assignment in &request.workspaces {
        if !valid_workspace(&assignment.workspace) {
            return Err(refused("A workspace name is not usable"));
        }
        if workspaces.contains(&assignment.workspace.as_str()) {
            return Err(refused("A workspace is placed twice"));
        }
        if !seen.contains(&assignment.output.as_str()) {
            return Err(refused("A workspace is placed on a display that is off"));
        }
        workspaces.push(&assignment.workspace);
    }
    let source = std::str::from_utf8(&snapshot.file)
        .map_err(|_| refused("The display settings file is not text"))?;
    let topology_key = snapshot.topology_key();
    let block = render_profile(&topology_key, &request.outputs, &request.workspaces);
    let updated = replace_block(source, &block)?.into_bytes();
    let summary = summary(&active, &request.outputs, &request.workspaces);
    let token = digest(format!("{revision}\n{}\n{}", digest(&updated), now_ms()).as_bytes())[..32]
        .to_string();
    Ok(Plan {
        token,
        summary,
        revision,
        topology_key,
        block,
        original: snapshot.file.clone(),
        updated,
        confirm_seconds: CONFIRM_SECONDS,
    })
}

/// Write a reviewed change.
///
/// Under the settings lock: refuses when the file changed since the preview
/// or another change still waits for a keep; writes the previous bytes to
/// `PREVIOUS_FILE`, the change to `CONFIG_FILE` and the pending confirmation
/// beside them. When `live`, the compositor reloads and its configuration
/// errors are read; any error reverts at once and is returned as
/// `ApplyError::Compositor` with the compositor's lines.
pub fn apply(plan: &Plan, config_dir: &Path, live: bool) -> Result<Receipt> {
    apply_with(
        plan,
        config_dir,
        live.then_some(&Hyprland as &dyn Compositor),
    )
}

/// `apply` with the compositor given; `None` writes the file only.
pub fn apply_with(
    plan: &Plan,
    config_dir: &Path,
    compositor: Option<&dyn Compositor>,
) -> Result<Receipt> {
    let live = compositor.is_some();
    let _lock = lock(config_dir)?;
    let path = config_path(config_dir)?;
    let pending_path = config_dir.join(PENDING_FILE);
    if pending_path.exists() {
        return Err(refused("Keep or revert the display change first"));
    }
    let current = read_file(&path)?;
    if digest(&current) != plan.revision || current != plan.original {
        return Err(refused("The display settings changed; review them again"));
    }
    let previous = config_dir.join(PREVIOUS_FILE);
    write_atomic(&previous, &plan.original, None)?;
    let pending = Pending {
        token: plan.token.clone(),
        deadline_ms: now_ms() + u128::from(plan.confirm_seconds) * 1000,
        updated: digest(&plan.updated),
        previous: plan.revision.clone(),
        live,
    };
    write_atomic(&pending_path, &serde_json::to_vec_pretty(&pending)?, None)?;
    write_atomic(&path, &plan.updated, file_mode(&path))?;
    if let Some(compositor) = compositor {
        let errors = reload_and_read_errors(compositor);
        if !errors.is_empty() {
            restore(&path, &previous, &pending.updated)?;
            if let Err(error) = compositor.reload() {
                return Err(ApplyError::Compositor(vec![error.to_string()]));
            }
            fs::remove_file(&pending_path)?;
            return Err(ApplyError::Compositor(errors));
        }
    }
    Ok(Receipt {
        token: plan.token.clone(),
        written: true,
        applied: live,
        revision: digest(&read_file(&path)?),
        previous_revision: digest(&read_file(&previous)?),
        previous,
        confirm_seconds: plan.confirm_seconds,
    })
}

/// Keep or revert the change that waits for confirmation.
///
/// With the matching `token` and `keep` inside the window the change stays.
/// Without a keep, or once the window has passed (whatever the token), the
/// previous bytes are written back, when the file still holds this change,
/// and the compositor reloads if the apply was live. Returns what happened
/// and the file's revision now; refuses a token that names no waiting change.
pub fn confirm(config_dir: &Path, token: &str, keep: bool) -> Result<Confirmation> {
    confirm_with(config_dir, token, keep, &Hyprland)
}

/// `confirm` with the compositor given; it reloads only after a live apply.
pub fn confirm_with(
    config_dir: &Path,
    token: &str,
    keep: bool,
    compositor: &dyn Compositor,
) -> Result<Confirmation> {
    let _lock = lock(config_dir)?;
    let path = config_path(config_dir)?;
    let pending_path = config_dir.join(PENDING_FILE);
    let pending: Pending = match fs::read(&pending_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(refused("No display change is waiting"));
        }
        Err(error) => return Err(error.into()),
    };
    let lapsed = now_ms() > pending.deadline_ms;
    if !lapsed && pending.token != token {
        return Err(refused("No display change is waiting"));
    }
    let kept = keep && !lapsed;
    let mut reverted = false;
    if !kept {
        reverted = restore(&path, &config_dir.join(PREVIOUS_FILE), &pending.updated)?;
        if reverted && pending.live {
            compositor.reload()?;
        }
    }
    fs::remove_file(&pending_path)?;
    Ok(Confirmation {
        kept,
        reverted,
        lapsed,
        revision: digest(&read_file(&path)?),
    })
}

fn reload_and_read_errors(compositor: &dyn Compositor) -> Vec<String> {
    if let Err(error) = compositor.reload() {
        return vec![error.to_string()];
    }
    match compositor.config_errors() {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        Err(error) => vec![error.to_string()],
    }
}

/// The two things asked of the compositor after a live write.
pub trait Compositor {
    /// Read the configuration again.
    fn reload(&self) -> Result<()>;
    /// The configuration errors it reports, one per line; empty when none.
    fn config_errors(&self) -> Result<String>;
}

/// The running compositor; used only when the caller passes `live`.
struct Hyprland;

impl Compositor for Hyprland {
    fn reload(&self) -> Result<()> {
        let output = Command::new("hyprctl").arg("reload").output()?;
        if !output.status.success() {
            return Err(refused("The compositor did not reload"));
        }
        Ok(())
    }

    fn config_errors(&self) -> Result<String> {
        let output = Command::new("hyprctl").arg("configerrors").output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Write the previous bytes back only while the file still holds this change,
/// so a later edit by hand is never overwritten.
fn restore(path: &Path, previous: &Path, updated_digest: &str) -> Result<bool> {
    if digest(&read_file(path)?) != updated_digest {
        return Ok(false);
    }
    let bytes = read_file(previous)?;
    write_atomic(path, &bytes, file_mode(path))?;
    Ok(true)
}

fn block_span(source: &str) -> Result<Option<(usize, usize)>> {
    let begins = line_starts(source, BEGIN_MARKER);
    let ends = line_starts(source, END_MARKER);
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([start], [end]) if end > start => {
            let after = source[*end..]
                .find('\n')
                .map(|offset| end + offset + 1)
                .unwrap_or(source.len());
            Ok(Some((*start, after)))
        }
        _ => Err(refused("The display block in the settings file is broken")),
    }
}

fn line_starts(source: &str, marker: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim_end() == marker {
            starts.push(offset);
        }
        offset += line.len();
    }
    starts
}

fn overlaps(first: &OutputSetting, second: &OutputSetting) -> bool {
    let (Some((w1, h1)), Some((w2, h2))) = (first.logical_size(), second.logical_size()) else {
        return false;
    };
    let (x1, y1, x2, y2) = (
        i64::from(first.x),
        i64::from(first.y),
        i64::from(second.x),
        i64::from(second.y),
    );
    x1 < x2 + w2 && x2 < x1 + w1 && y1 < y2 + h2 && y2 < y1 + h1
}

fn valid_workspace(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.chars().any(char::is_control)
        && name.trim() == name
}

fn summary(
    active: &[&ObservedOutput],
    outputs: &[OutputSetting],
    workspaces: &[Assignment],
) -> String {
    let screens: Vec<String> = outputs
        .iter()
        .map(|setting| {
            let size = setting.mode.split('@').next().unwrap_or_default();
            let mut words = format!("{} {} at {},{}", setting.name, size, setting.x, setting.y);
            if (setting.scale - 1.0).abs() > f64::EPSILON {
                words.push_str(&format!(" scale {}", setting.scale));
            }
            if setting.transform != 0 {
                words.push(' ');
                words.push_str(orientation(setting.transform));
            }
            words
        })
        .collect();
    let mut line = screens.join("; ");
    let placed: Vec<String> = active
        .iter()
        .filter_map(|output| {
            let names: Vec<&str> = workspaces
                .iter()
                .filter(|a| a.output == output.name)
                .map(|a| a.workspace.as_str())
                .collect();
            (!names.is_empty()).then(|| format!("{} on {}", names.join(", "), output.name))
        })
        .collect();
    if !placed.is_empty() {
        line.push_str("; workspaces ");
        line.push_str(&placed.join(", "));
    }
    line
}

fn orientation(transform: u8) -> &'static str {
    match transform {
        1 => "turned right",
        2 => "upside down",
        3 => "turned left",
        4 => "flipped",
        5 => "flipped, turned right",
        6 => "flipped, upside down",
        7 => "flipped, turned left",
        _ => "normal",
    }
}

fn lua_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            c if c.is_control() => quoted.push_str(&format!("\\{}", c as u32)),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

fn comment(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}

fn config_path(config_dir: &Path) -> Result<PathBuf> {
    let path = config_dir.join(CONFIG_FILE);
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        return Err(refused(
            "The display settings file is a link; nothing written",
        ));
    }
    Ok(path)
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    if fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err(refused("The display settings file is too large"));
    }
    Ok(fs::read(path)?)
}

fn file_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn lock(config_dir: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config_dir.join(LOCK_FILE))?;
    file.lock()?;
    Ok(file)
}

fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| refused("No folder for the settings file"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = parent.join(format!(".{name}.{}.tmp", now_ms()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode.unwrap_or(0o600)))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
