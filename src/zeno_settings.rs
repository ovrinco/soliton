//! Zeno profile settings, read from their owner and changed without launching it.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const ROUTES: [&str; 3] = ["zeno", "zeno-local", "zeno-openrouter"];
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
enum SettingsError {
    #[error("{0}")]
    Invalid(String),
    #[error("Zeno settings file {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Zeno settings contain invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}
type Result<T> = std::result::Result<T, SettingsError>;
fn invalid(message: impl Into<String>) -> SettingsError {
    SettingsError::Invalid(message.into())
}
fn io<T>(path: &Path, result: std::io::Result<T>) -> Result<T> {
    result.map_err(|source| SettingsError::Io {
        path: path.to_owned(),
        source,
    })
}

struct Paths {
    profiles: PathBuf,
    catalogue: PathBuf,
    home: PathBuf,
    copied: bool,
}
impl Paths {
    fn from_args(args: &Value) -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| invalid("Home folder is unavailable"))?;
        if let Some(root) = args.get("test_root") {
            let root = PathBuf::from(
                root.as_str()
                    .ok_or_else(|| invalid("test_root must be an absolute folder"))?,
            );
            safe_path(&root)?;
            Ok(Self {
                profiles: root.join("profiles"),
                catalogue: root.join("zeno-models.json"),
                home,
                copied: true,
            })
        } else {
            let profiles = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            Ok(Self {
                profiles,
                catalogue: home.join(".local/lib/zeno/current/config/zeno-models.json"),
                home,
                copied: false,
            })
        }
    }
    fn profile(&self, route: &str) -> Result<PathBuf> {
        if !ROUTES.contains(&route) {
            return Err(invalid(
                "Choose the zeno, zeno-local or zeno-openrouter profile",
            ));
        }
        let path = self.profiles.join(format!("{route}.config.toml"));
        safe_path(&path)?;
        Ok(path)
    }
}
fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid(
            "Zeno settings path must be absolute without . or ..",
        ));
    }
    let mut prefix = PathBuf::new();
    for part in path.components() {
        prefix.push(part.as_os_str());
        match fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(invalid(
                    "Zeno settings path contains a symlink; use a real copy",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return io(&prefix, Err(error)),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<String> {
    let metadata = io(path, fs::metadata(path))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(invalid(format!(
            "Zeno settings {} must be a regular file smaller than 2 MiB",
            path.display()
        )));
    }
    io(path, fs::read_to_string(path))
}

struct Entry {
    key: String,
    value: Value,
    line: usize,
}
// This preserves the installed profiles' scalar TOML dialect. Unsupported
// syntax fails explicitly, so a profile is never partly read or rewritten.
fn entries(text: &str) -> Result<Vec<Entry>> {
    let mut section = String::new();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (line, raw) in text.lines().enumerate() {
        let text = without_comment(raw).trim();
        if text.is_empty() {
            continue;
        }
        if text.starts_with('[') && text.ends_with(']') && !text.starts_with("[[") {
            section = text[1..text.len() - 1].trim().to_owned();
            continue;
        }
        let (key, encoded) = text
            .split_once('=')
            .ok_or_else(|| invalid(format!("Unsupported profile syntax at line {}", line + 1)))?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid(format!(
                "Unsupported profile key at line {}",
                line + 1
            )));
        }
        let encoded = encoded.trim();
        let value: Value = if encoded.starts_with('\'')
            && encoded.ends_with('\'')
            && encoded.len() >= 2
        {
            json!(&encoded[1..encoded.len() - 1])
        } else {
            serde_json::from_str(encoded).map_err(|_| invalid(format!("Unsupported profile value at line {}; expected a quoted string, boolean, number or array", line+1)))?
        };
        if value.is_null() || value.is_object() {
            return Err(invalid(format!(
                "Unsupported profile value at line {}",
                line + 1
            )));
        }
        let key = if section.is_empty() {
            key.to_owned()
        } else {
            format!("{section}.{key}")
        };
        if !seen.insert(key.clone()) {
            return Err(invalid(format!("Duplicate Zeno setting {key}")));
        }
        result.push(Entry { key, value, line });
    }
    Ok(result)
}
fn without_comment(text: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '"' | '\'') {
            quote = Some(ch);
        } else if quote.is_none() && ch == '#' {
            return &text[..index];
        }
    }
    text
}
fn kind(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "string",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => "null",
    }
}
fn allowed(key: &str, value: &Value) -> Value {
    match key.rsplit('.').next().unwrap_or(key) {
        "model_reasoning_effort" => json!(["none", "minimal", "low", "medium", "high", "xhigh"]),
        "approval_policy" => json!(["untrusted", "on-failure", "on-request", "never"]),
        "sandbox_mode" => json!(["read-only", "workspace-write", "danger-full-access"]),
        "trust_level" => json!(["trusted", "untrusted"]),
        "web_search" => json!(["disabled", "cached", "live"]),
        "wire_api" => json!(["responses"]),
        _ if value.is_boolean() => json!([false, true]),
        _ if value.is_string() => json!(["nonempty text"]),
        _ if value.is_number() => json!(["nonnegative number"]),
        _ if value.is_array() => json!(["array"]),
        _ => json!([value]),
    }
}
fn description(key: &str) -> String {
    let leaf = key.rsplit(['.', '/']).next().unwrap_or(key);
    let text = match leaf {
        "model_reasoning_effort" | "default_reasoning_level" | "effort" => {
            "How much reasoning the model is asked to do"
        }
        "approval_policy" => "When Zeno asks before running a command",
        "sandbox_mode" => "Which files and operations Zeno may access",
        "developer_instructions" => {
            "Profile instructions; the launcher replaces these with live alignment files"
        }
        "model" | "slug" => "The model name sent to the selected provider",
        "model_provider" => "The provider used for this route",
        "model_catalog_json" => "File containing the model capability descriptions",
        "web_search" => "Whether searches are disabled, cached or live",
        "trust_level" => "Whether this project folder is trusted",
        "name" | "display_name" => "The readable name shown for this provider or model",
        "base_url" => "The provider's API address; changing it starts no server",
        "env_key" => {
            "Name of the environment variable holding the API key; the key is never returned"
        }
        "wire_api" => "The API format used to talk to the provider",
        "requires_openai_auth" => "Whether the provider needs native OpenAI authentication",
        "description" => "Explanation of the model or reasoning choice",
        "shell_type" => "How shell commands are sent to the model",
        "visibility" => "Where the model appears in the model picker",
        "minimal_client_version" => "Oldest client version that can use the model",
        "supported_in_api" => "Whether the model is available through the API",
        "priority" => "The order in which the model appears",
        "context_window" | "max_context_window" => "Maximum number of tokens held in model context",
        "auto_compact_token_limit" => {
            "Token count that triggers context compaction; null uses the client default"
        }
        "mode" => "How older tool output is shortened",
        "limit" => "Maximum tool output retained before shortening",
        "input_modalities" => "The kinds of input the model accepts",
        "apply_patch_tool_type" => "The patch format the model can use to edit files",
        "supports_search_tool" => "Whether the model supports the search tool",
        "supports_parallel_tool_calls" => "Whether several tools can be requested together",
        "prefer_websockets" => "Whether the client prefers WebSocket transport",
        "support_verbosity" => "Whether response length can be configured",
        "supports_reasoning_summaries" | "supports_reasoning_summary_parameter" => {
            "Whether the model supports a summary of its reasoning"
        }
        "default_reasoning_summary" => "Default reasoning summary mode",
        "experimental_supported_tools" => "Additional experimental tools the model accepts",
        "base_instructions" => "Base instructions supplied to this model by the client",
        _ => return format!("Additional Zeno setting: {}", leaf.replace('_', " ")),
    };
    text.to_owned()
}
fn field(id: &str, key: &str, value: Value, path: &Path, editable: bool) -> Value {
    json!({"field":id,"kind":kind(&value),"allowed_values":allowed(key,&value),"current_value":value,"description":description(key),"file":path,"editable":editable})
}
fn catalogue_fields(value: &Value, pointer: &str, path: &Path, output: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                catalogue_fields(value, &format!("{pointer}/{key}"), path, output);
            }
        }
        Value::Array(array) if array.iter().any(|v| v.is_object() || v.is_array()) => {
            for (index, value) in array.iter().enumerate() {
                catalogue_fields(value, &format!("{pointer}/{index}"), path, output);
            }
        }
        _ => {
            let mut item = field(
                &format!("catalogue{pointer}"),
                pointer,
                value.clone(),
                path,
                false,
            );
            item["allowed_values"] = json!([value]);
            item["read_only_reason"] =
                json!("Installed model metadata is maintained by Zeno's release owner");
            output.push(item);
        }
    }
}
fn schema(paths: &Paths) -> Result<Value> {
    let mut fields = Vec::new();
    for route in ROUTES {
        let path = paths.profile(route)?;
        for entry in entries(&read(&path)?)? {
            let mut item = field(
                &format!("{route}/{}", entry.key),
                &entry.key,
                entry.value,
                &path,
                entry.key != "developer_instructions",
            );
            item["line"] = json!(entry.line + 1);
            if entry.key == "developer_instructions" {
                item["read_only_reason"] =
                    json!("The launcher replaces this text with live alignment files");
            }
            fields.push(item);
        }
    }
    let catalogue: Value = serde_json::from_str(&read(&paths.catalogue)?)?;
    catalogue_fields(&catalogue, "", &paths.catalogue, &mut fields);
    for (key, value, explanation, choices) in [
        (
            "home_folder",
            json!(paths.home),
            "Default working folder; pass DIR or here to Zeno to change it for a launch",
            json!(["absolute folder supplied as DIR", "here"]),
        ),
        (
            "route",
            json!("zeno"),
            "Command argument selecting the subscription, local or OpenRouter route",
            json!(ROUTES),
        ),
        (
            "state_home",
            json!(paths.profiles),
            "Native profile and history folder, selected with CODEX_HOME",
            json!(["absolute folder"]),
        ),
        (
            "harness",
            json!(
                std::env::var_os("ZENO_HARNESS")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths.home.join(".local/lib/zeno/current/bin/zeno-harness"))
            ),
            "Executable selected with ZENO_HARNESS; otherwise the adjacent installed harness",
            json!(["absolute executable"]),
        ),
        (
            "agent_folder",
            json!(
                std::env::var_os("ZENO_AGENT_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths.home.join(".config/zeno/agent"))
            ),
            "Folder of live alignment files, selected with ZENO_AGENT_DIR",
            json!(["absolute folder"]),
        ),
        (
            "remote_address",
            json!("127.0.0.1:8797"),
            "Loopback WebSocket address supplied with --ws",
            json!(["127.0.0.1:PORT"]),
        ),
        (
            "remote_runtime",
            json!(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)),
            "Native runtime folder holding Zeno's socket and token files",
            json!(["absolute folder"]),
        ),
        (
            "openrouter_key_present",
            json!(std::env::var_os("OPENROUTER_API_KEY").is_some()),
            "Whether an OpenRouter API key is supplied; its contents are never read or returned",
            json!([false, true]),
        ),
        (
            "remote_token_present",
            json!(std::env::var_os("ZENO_REMOTE_TOKEN").is_some()),
            "Whether a remote access token is supplied; its contents are never read or returned",
            json!([false, true]),
        ),
        (
            "native_arguments",
            json!([]),
            "Additional native arguments passed after -- for this launch",
            json!(["argument array"]),
        ),
    ] {
        fields.push(json!({"field":format!("launcher/{key}"),"kind":kind(&value),"allowed_values":choices,"current_value":value,"description":explanation,"editable":false,"read_only_reason":"Launch argument or environment value; Zeno has no persistent file for this setting"}));
    }
    Ok(
        json!({"schema":1,"routes":ROUTES,"fields":fields,"test_copy":paths.copied,"effects":"Reads settings only; starts no process or model"}),
    )
}
fn validate(key: &str, old: &Value, new: &Value) -> Result<()> {
    let choices = allowed(key, old);
    let choices = choices
        .as_array()
        .ok_or_else(|| invalid("Allowed values are unavailable"))?;
    let valid = kind(old) == kind(new)
        && match choices.first().and_then(Value::as_str) {
            Some("nonempty text") => new
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && !s.contains('\0') && s.len() <= 65536),
            Some("nonnegative number") => new.as_f64().is_some_and(|n| n >= 0.0),
            Some("array") => new.is_array(),
            _ => choices.contains(new),
        };
    if !valid {
        return Err(invalid(format!(
            "Value refused for {key}. Allowed values: {} ({} required)",
            Value::Array(choices.clone()),
            kind(old)
        )));
    }
    if key.ends_with("base_url")
        && !new
            .as_str()
            .is_some_and(|s| s.starts_with("https://") || s.starts_with("http://"))
    {
        return Err(invalid(
            "Provider address must begin with http:// or https://",
        ));
    }
    if key == "model_catalog_json" && !new.as_str().is_some_and(|s| Path::new(s).is_absolute()) {
        return Err(invalid("Model catalogue file must be an absolute path"));
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8], permissions: fs::Permissions) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = io(path, options.open(path))?;
    io(path, file.write_all(bytes))?;
    io(path, file.set_permissions(permissions))?;
    io(path, file.sync_all())
}
fn set(paths: &Paths, args: &Value) -> Result<Value> {
    let id = args["field"]
        .as_str()
        .ok_or_else(|| invalid("A settings field is required"))?;
    let (route, key) = id
        .split_once('/')
        .ok_or_else(|| invalid("Field must include its route and name"))?;
    if matches!(route, "catalogue" | "launcher") {
        return Err(invalid(
            "This field is read only: model metadata belongs to Zeno's release; launch arguments and environment are supplied at launch",
        ));
    }
    if key == "developer_instructions" {
        return Err(invalid(
            "The launcher replaces profile instructions with live alignment files",
        ));
    }
    let path = paths.profile(route)?;
    let lock_path = path.with_file_name(format!(".{route}.config.toml.soliton.lock"));
    safe_path(&lock_path)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = io(&lock_path, options.open(&lock_path))?;
    io(&lock_path, lock.lock())?;
    safe_path(&path)?;
    let previous = read(&path)?;
    let entry = entries(&previous)?
        .into_iter()
        .find(|entry| entry.key == key)
        .ok_or_else(|| invalid(format!("Unknown Zeno settings field: {id}")))?;
    let value = args
        .get("value")
        .ok_or_else(|| invalid("A new settings value is required"))?;
    validate(key, &entry.value, value)?;
    let mut lines: Vec<String> = previous.split_inclusive('\n').map(str::to_owned).collect();
    let line = &lines[entry.line];
    let equals = line
        .find('=')
        .ok_or_else(|| invalid("Setting assignment is unavailable"))?;
    let suffix = &line[without_comment(line).len()..];
    let comment = if suffix.starts_with('#') {
        format!(" {suffix}")
    } else if line.ends_with('\n') {
        "\n".to_owned()
    } else {
        String::new()
    };
    lines[entry.line] = format!(
        "{}= {}{comment}",
        &line[..equals],
        serde_json::to_string(value)?
    );
    let updated = lines.concat();
    entries(&updated)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("Clock is before the Unix epoch"))?
        .as_nanos();
    let backup = path.with_file_name(format!(
        "{route}.config.toml.before-{stamp}-{}",
        std::process::id()
    ));
    let temporary = path.with_file_name(format!(".{route}.config.toml.{stamp}.tmp"));
    let permissions = io(&path, fs::metadata(&path))?.permissions();
    write_new(&backup, previous.as_bytes(), permissions.clone())?;
    write_new(&temporary, updated.as_bytes(), permissions)?;
    if io(&path, fs::read(&path))? != previous.as_bytes() {
        io(&temporary, fs::remove_file(&temporary))?;
        return Err(invalid(
            "Zeno settings changed during the edit. Reload and try again; previous bytes are kept",
        ));
    }
    io(&path, fs::rename(&temporary, &path))?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Settings file has no parent folder"))?;
    io(parent, io(parent, File::open(parent))?.sync_all())?;
    Ok(
        json!({"field":id,"current_value":value,"file":path,"backup_path":backup,"effects":"One profile field saved; no process or model started"}),
    )
}
pub fn command(command: &str, args: &Value) -> crate::Result<Value> {
    let result = Paths::from_args(args).and_then(|paths| match command {
        "zeno-settings-get" => schema(&paths),
        "zeno-settings-set" => set(&paths, args),
        _ => Err(invalid("Unknown Zeno settings command")),
    });
    result.map_err(|error| crate::Error::Invalid(error.to_string()))
}
