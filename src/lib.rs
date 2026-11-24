pub mod collection;
pub mod desktop;
pub mod desktop_apply;
pub mod history;
pub mod inbox;
pub mod machine_log;
pub mod modules;
pub mod processes;
pub mod recovery;
pub mod runs;
pub mod service;
pub mod time;
pub mod tracker;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Kernel(#[from] soliton_kernel::KernelError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

pub mod transfer;

pub mod source_preferences;

pub mod briefing;

pub mod inference;

pub mod desktop_fallback;

pub mod hearth_draft;
