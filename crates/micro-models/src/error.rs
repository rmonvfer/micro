use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to parse model catalog: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A user-defined model that names a provider the catalog has never seen,
    /// without supplying the endpoint details needed to reach it.
    #[error("model `{provider}/{id}` must declare both `api` and `base_url`")]
    IncompleteModel { provider: String, id: String },

    #[error("could not locate the home directory; set MICRO_DIR to choose a config directory")]
    NoHomeDir,

    #[error("request to {url} failed")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{provider} model listing returned HTTP {status}")]
    ListingStatus { provider: &'static str, status: u16 },

    #[error("unexpected {provider} model listing response: {reason}")]
    ListingShape {
        provider: &'static str,
        reason: String,
    },
}
