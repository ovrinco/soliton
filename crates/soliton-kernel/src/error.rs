use std::fmt;
use std::io;

/// The fail-closed error surface shared by kernel modules.
#[derive(Debug)]
pub enum KernelError {
    InvalidId(String),
    InvalidDate(String),
    InvalidRecurrence(String),
    InvalidObject(String),
    Codec(String),
    Io(io::Error),
    Conflict(String),
    Integrity(String),
    PathDenied(String),
    Registry(String),
    Authorization(String),
    Recovery(String),
    Ledger(String),
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(message) => write!(formatter, "invalid id: {message}"),
            Self::InvalidDate(message) => write!(formatter, "invalid date: {message}"),
            Self::InvalidRecurrence(message) => write!(formatter, "invalid recurrence: {message}"),
            Self::InvalidObject(message) => write!(formatter, "invalid object: {message}"),
            Self::Codec(message) => write!(formatter, "codec error: {message}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Conflict(message) => write!(formatter, "write conflict: {message}"),
            Self::Integrity(message) => write!(formatter, "integrity failure: {message}"),
            Self::PathDenied(message) => write!(formatter, "path denied: {message}"),
            Self::Registry(message) => write!(formatter, "registry error: {message}"),
            Self::Authorization(message) => write!(formatter, "authorization denied: {message}"),
            Self::Recovery(message) => write!(formatter, "recovery failed: {message}"),
            Self::Ledger(message) => write!(formatter, "ledger error: {message}"),
        }
    }
}

impl std::error::Error for KernelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for KernelError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, KernelError>;
