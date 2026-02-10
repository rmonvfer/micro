//! The single error type this crate returns.

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PolicyError>;

#[derive(Debug, Error)]
pub enum PolicyError {
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

impl PolicyError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        PolicyError::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        PolicyError::Json {
            path: path.into(),
            source,
        }
    }
}
