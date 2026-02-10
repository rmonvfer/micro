//! The single error type this crate returns.

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ContextError>;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("cannot locate a home directory; set {env}")]
    NoHome { env: &'static str },

    #[error("invalid compaction config: {0}")]
    InvalidConfig(String),

    /// The history cannot be split: everything in it has to be kept verbatim, so there is
    /// nothing left for a summary to replace.
    #[error("nothing to compact; the whole history fits in the recent window")]
    NothingToCompact,

    /// Raised by a caller's [`crate::Summarizer`], whose own error type this crate does
    /// not know.
    #[error("summarization failed: {0}")]
    Summarizer(String),

    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl ContextError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ContextError::Io {
            path: path.into(),
            source,
        }
    }

    /// Wraps a summarizer's own failure. Implementations of [`crate::Summarizer`] use this
    /// to report anything their provider returned.
    pub fn summarizer(message: impl std::fmt::Display) -> Self {
        ContextError::Summarizer(message.to_string())
    }
}
