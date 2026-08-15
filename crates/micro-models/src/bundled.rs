/// The catalog compiled into the binary, so the agent has a usable set of models with no network
/// access and no configuration.
pub(crate) const CATALOG_JSON: &str = include_str!("../data/catalog.json");
