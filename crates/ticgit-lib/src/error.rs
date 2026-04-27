//! Error types for the ticgit library.

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum Error {
    #[error("git-meta error: {0}")]
    GitMeta(#[from] git_meta_lib::Error),

    #[error("ticket {0} not found")]
    NotFound(Uuid),

    #[error("ticket prefix `{0}` is ambiguous: matches {1} tickets")]
    Ambiguous(String, usize),

    #[error("ticket prefix `{0}` matches no ticket")]
    NoMatch(String),

    #[error("invalid ticket state `{0}` (expected one of: open, resolved, invalid, hold)")]
    InvalidState(String),

    #[error("invalid value: {0}")]
    InvalidValue(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("time formatting error: {0}")]
    Time(String),
}

pub type Result<T> = std::result::Result<T, Error>;
