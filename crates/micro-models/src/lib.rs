//! The model catalog: which models exist, how to reach them, and what they
//! cost.
//!
//! A catalog is assembled from three layers, each overlaying the last:
//!
//! 1. a catalog compiled into the binary, so the agent works offline with no
//!    setup ([`Catalog::bundled`]);
//! 2. the user's own catalog, `models.json` in micro's configuration directory,
//!    which patches known models and registers new ones ([`Catalog::load`]);
//! 3. live provider listings, so models released since this build show up
//!    without one ([`Catalog::merge_live_listings`]).
//!
//! ```no_run
//! use micro_models::{Catalog, TokenUsage};
//!
//! let catalog = Catalog::load()?;
//! let model = catalog
//!     .resolve("opus")
//!     .model()
//!     .expect("`opus` resolves to exactly one model");
//!
//! let spent = model.price(TokenUsage::new(12_000, 3_000).with_cache(40_000, 0));
//! println!("{} — ${:.4}", model.qualified_id(), spent.total());
//! # Ok::<(), micro_models::Error>(())
//! ```

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
pub use wire_json::model_json;
pub use wire_json::modality_name;
pub use wire_json::wire_api_name;
