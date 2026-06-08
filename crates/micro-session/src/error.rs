//! The single error type every store operation returns.

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("no session with id {0}")]
    NotFound(String),

    #[error("invalid session id: {0}")]
    InvalidId(String),

    #[error("{path}: invalid session graph: {reason}")]
    InvalidGraph { path: PathBuf, reason: String },

    #[error("message index {index} is out of range; the session holds {len} messages")]
    IndexOutOfRange { index: usize, len: usize },

    #[error("session {id} recorded no turn {turn}")]
    NoSuchTurn { id: String, turn: u64 },

    /// A fact names content by hash, and the content is not beside the log any more.
    #[error("session {id} is missing the content {hash} one of its records names")]
    MissingBlob { id: String, hash: String },

    #[error("cannot locate a home directory; set {env}")]
    NoHome { env: &'static str },

    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{}: {source}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl SessionError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        SessionError::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        SessionError::Json {
            path: path.into(),
            source,
        }
    }
}
