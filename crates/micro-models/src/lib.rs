//! The model catalog: which models exist, how to reach them, and what they cost.

mod bundled;
mod catalog;
mod compat;
mod cost;
mod error;
pub mod fuzzy;
mod overrides;
mod remote;
mod resolve;
mod wire_json;

pub use catalog::{Catalog, Modality, ModelCost, ModelDef, WireApi};
pub use compat::CompatOverrides;
pub use cost::{RequestCost, TokenUsage};
pub use error::{Error, Result};
pub use overrides::{config_dir, user_catalog_path, USER_CATALOG_FILE};
pub use remote::{
    fetch_copilot, fetch_openrouter, parse_copilot, parse_openrouter, CopilotCredentials,
    COPILOT_BASE_URL, COPILOT_PROVIDER, OPENROUTER_BASE_URL, OPENROUTER_PROVIDER,
};
pub use resolve::Resolution;
pub use wire_json::catalog_json;
pub use wire_json::modality_name;
pub use wire_json::model_json;
pub use wire_json::wire_api_name;
