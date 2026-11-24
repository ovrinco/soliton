//! Launch the operator's installed terminal tools without owning their sessions.
use serde_json::{Value, json};
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("Unknown terminal tool {0}")]
    UnknownTool(String),
    #[error("The launch folder is not an existing directory: {0}")]
    Folder(PathBuf),
    #[error("Could not start {program} in {folder}: {source}")]
    Start {
        program: PathBuf,
        folder: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

type Result<T> = std::result::Result<T, LaunchError>;

#[derive(Clone, Copy)]
enum EmulatorKind {
    Foot,
    Alacritty,
    Ghostty,
    Kitty,
}
impl EmulatorKind {
    fn of(program: &Path) -> Option<Self> {
        match program.file_name()?.to_str()? {
            "foot" => Some(Self::Foot),
            "alacritty" => Some(Self::Alacritty),
            "ghostty" => Some(Self::Ghostty),
            "kitty" => Some(Self::Kitty),
            _ => None,
        }
    }
    fn arguments(self, folder: &Path, command: Option<&Path>) -> Vec<OsString> {
        let mut args = match self {
            Self::Foot | Self::Ghostty => {
                let mut arg = OsString::from("--working-directory=");
                arg.push(folder);
                vec![arg]
            }
            Self::Alacritty => vec!["--working-directory".into(), folder.into()],
            Self::Kitty => vec!["--directory".into(), folder.into()],
        };
        if let Some(command) = command {
            if matches!(self, Self::Alacritty | Self::Ghostty) {
                args.push("-e".into());
            }
            args.push(command.into());
        }
        args
    }
}

/// The four fixed entry points. Only discovery supplies executable paths;
/// launch requests cannot supply a program or shell command.
pub struct ToolLauncher {
    emulator: Option<(PathBuf, EmulatorKind)>,
    terminal_source: String,
    codex: Option<PathBuf>,
    qream: Option<PathBuf>,
    zeno: Option<PathBuf>,
}
fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn find_program(path: &OsStr, name: &OsStr) -> Option<PathBuf> {
    let program = Path::new(name);
    if program.components().count() > 1 || program.is_absolute() {
        return executable(program).then(|| program.to_path_buf());
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(program))
        .find(|p| executable(p))
}
impl ToolLauncher {
    /// Capture the emulator choice once, when the service opens. Environment,
    /// Omarchy's terminal preference files, then installed fallback order.
    pub fn discover() -> Self {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let terminal = std::env::var_os("TERMINAL");
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        let files = [
            config.join("hyprland-xdg-terminals.list"),
            config.join("xdg-terminals.list"),
            PathBuf::from("/etc/xdg/hyprland-xdg-terminals.list"),
            PathBuf::from("/etc/xdg/xdg-terminals.list"),
            PathBuf::from(
                "/usr/share/omarchy/default/xdg-terminal-exec/hyprland-xdg-terminals.list",
            ),
        ];
        Self::discover_with(&path, terminal.as_deref(), &files)
    }
    /// Explicit discovery inputs let contract tests substitute executable
    /// recorders without changing process-wide environment or live settings.
    pub fn discover_with(
        path: &OsStr,
        terminal: Option<&OsStr>,
        preference_files: &[PathBuf],
    ) -> Self {
        let candidate = |name: &OsStr| {
            let program = find_program(path, name)?;
            Some((program.clone(), EmulatorKind::of(&program)?))
        };
        let mut emulator = terminal.filter(|v| !v.is_empty()).and_then(candidate);
        let mut source = if emulator.is_some() {
            "TERMINAL".to_string()
        } else {
            String::new()
        };
        if emulator.is_none() {
            for file in preference_files {
                let Ok(text) = fs::read_to_string(file) else {
                    continue;
                };
                for line in text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('-'))
                {
                    // The supported emulator desktop ids match their command names.
                    let name = line.strip_suffix(".desktop").unwrap_or(line);
                    if let Some(found) = candidate(OsStr::new(name)) {
                        emulator = Some(found);
                        source = format!("Omarchy preference {}", file.display());
                        break;
                    }
                }
                if emulator.is_some() {
                    break;
                }
            }
        }
        if emulator.is_none() {
            for name in ["alacritty", "ghostty", "kitty", "foot"] {
                if let Some(found) = candidate(OsStr::new(name)) {
                    emulator = Some(found);
                    source = "installed fallback: alacritty, ghostty, kitty, foot".into();
                    break;
                }
            }
        }
        Self {
            emulator,
            terminal_source: source,
            codex: find_program(path, OsStr::new("codex")),
            qream: find_program(path, OsStr::new("qream")),
            zeno: find_program(path, OsStr::new("zeno")),
        }
    }
    pub fn available(&self) -> Value {
        let terminal = self.emulator.as_ref().map(|(p, _)| p);
        let entries = [
            ("terminal", terminal, terminal.is_some()),
            (
                "codex",
                self.codex.as_ref(),
                self.codex.is_some() && terminal.is_some(),
            ),
            ("qream", self.qream.as_ref(), self.qream.is_some()),
            (
                "zeno",
                self.zeno.as_ref(),
                self.zeno.is_some() && terminal.is_some(),
            ),
        ];
        json!({"default_agent":"codex","emulator":{"program":terminal,"available":terminal.is_some(),"source":self.terminal_source},
            "tools":entries.into_iter().map(|(tool,program,available)|json!({
                "tool":tool,"program":program,"installed":program.is_some(),"available":available,
                "message":if available {format!("{tool} is available")} else if program.is_none(){format!("{tool} is not installed")}else{"No supported terminal emulator is installed".into()}
            })).collect::<Vec<_>>()})
    }
    pub fn launch(&self, tool: &str, folder: &Path) -> Result<Value> {
        let tool = if tool == "agent" { "codex" } else { tool };
        let required = match tool {
            "terminal" => self.emulator.as_ref().map(|(p, _)| p),
            "codex" => self.codex.as_ref(),
            "qream" => self.qream.as_ref(),
            "zeno" => self.zeno.as_ref(),
            _ => return Err(LaunchError::UnknownTool(tool.into())),
        };
        let Some(required) = required else {
            return Ok(
                json!({"tool":tool,"started":false,"folder":folder,"message":format!("{tool} is not installed on this machine")}),
            );
        };
        let (program, args) = match tool {
            "terminal" | "codex" | "zeno" => {
                let Some((emulator, kind)) = &self.emulator else {
                    return Ok(
                        json!({"tool":tool,"started":false,"folder":folder,"message":"No supported terminal emulator is installed on this machine"}),
                    );
                };
                let mut args =
                    kind.arguments(folder, (tool != "terminal").then_some(required.as_path()));
                if tool == "zeno" {
                    args.push(folder.as_os_str().to_owned());
                }
                (emulator, args)
            }
            "qream" => (required, vec![OsString::from("gui")]),
            _ => unreachable!("tool was validated above"),
        };
        if !folder.is_dir() {
            return Err(LaunchError::Folder(folder.into()));
        }
        let mut child = Command::new(program)
            .args(&args)
            .current_dir(folder)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|source| LaunchError::Start {
                program: program.clone(),
                folder: folder.into(),
                source,
            })?;
        let pid = child.id();
        // Reap in the background; the engine returns immediately and never
        // holds a child's stdin or takes responsibility for its session.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(
            json!({"tool":tool,"started":true,"program":program,"folder":folder,"pid":pid,
            "message":format!("Started {tool} using {} in {}",program.display(),folder.display())}),
        )
    }
}
