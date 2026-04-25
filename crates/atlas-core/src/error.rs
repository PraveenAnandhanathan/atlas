//! Shared error type for ATLAS crates.

use thiserror::Error;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(String),

    #[error("integrity check failed: expected {expected}, got {actual}")]
    Integrity { expected: String, actual: String },

    #[error("malformed hash: {0}")]
    BadHash(String),

    #[error("malformed path: {0}")]
    BadPath(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid argument: {0}")]
    Invalid(String),

    #[error("backend error: {0}")]
    Backend(String),
}

impl From<bincode::Error> for Error {
    fn from(e: bincode::Error) -> Self {
        Self::Serde(e.to_string())
    }
}
