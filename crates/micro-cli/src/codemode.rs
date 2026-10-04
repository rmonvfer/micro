//! The `codemode` tool, with its `store()` values kept in the session.

use async_trait::async_trait;
use micro_codemode::Codemode;
use micro_codemode::ScriptStore;
use micro_codemode::StoreWrites;
use micro_codemode::CODEMODE_TOOL_NAME;
use micro_codemode::STORE_ENTRY_TYPE;
use micro_session::Session;
use serde_json::Map;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Keeps each successful script's writes as a custom entry, so a resumed session keeps them and
/// each branch sees only the values written along it.
pub struct SessionScriptStore {
    session: Arc<Mutex<Session>>,
}

#[async_trait]
impl ScriptStore for SessionScriptStore {
    async fn load(&self) -> Map<String, Value> {
        let held = self.session.lock().await;
        let mut store = Map::new();
        for custom in held
            .tree()
            .customs_on_path()
            .into_iter()
            .filter(|custom| custom.custom_type == STORE_ENTRY_TYPE)
        {
            micro_codemode::apply_store_entry(&mut store, &custom.data);
        }
        store
    }

    async fn append(&self, writes: &StoreWrites) -> Result<(), String> {
        self.session
            .lock()
            .await
            .append_custom(STORE_ENTRY_TYPE, micro_codemode::store_entry(writes))
            .await
            .map_err(|error| error.to_string())
    }
}

/// The `codemode` tool as the settings describe it, keeping its values in `session`.
pub fn tool(
    settings: &micro_config::Settings,
    session: Arc<Mutex<Session>>,
    models: micro_provider::ModelRuntime,
) -> Arc<dyn micro_tools::Tool> {
    let mode = match settings.codemode_mode {
        micro_config::CodemodeMode::On => micro_codemode::Mode::On,
        micro_config::CodemodeMode::Only => micro_codemode::Mode::Only,
    };
    let codemode = Codemode::new()
        .with_mode(mode)
        .with_inline_budget(settings.codemode_inline_budget)
        .with_store(Arc::new(SessionScriptStore { session }));
    let globals = Arc::new(crate::codemode_models::ModelGlobals::new(models));
    match codemode.with_globals(globals) {
        Ok(codemode) => Arc::new(codemode),
        Err(error) => unreachable!("the models global names are valid: {error}"),
    }
}

/// Whether `codemode` is offered: in every session whose tool selection neither narrows the
/// tools nor withholds it. A selection that names other tools without it leaves it out.
pub fn wanted(allowed: &[String], excluded: &[String]) -> bool {
    let named = |names: &[String]| names.iter().any(|name| name == CODEMODE_TOOL_NAME);
    if named(excluded) {
        return false;
    }
    allowed.is_empty() || named(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codemode_is_offered_unless_the_selection_leaves_it_out() {
        let codemode = vec![CODEMODE_TOOL_NAME.to_string()];
        assert!(wanted(&codemode, &[]));
        assert!(wanted(&[], &[]), "offered in a default session");
        assert!(
            !wanted(&["read".to_string()], &[]),
            "a selection that leaves it out leaves it out"
        );
        assert!(!wanted(&[], &codemode), "withheld");
        assert!(!wanted(
            &["read".to_string(), CODEMODE_TOOL_NAME.to_string()],
            &codemode
        ));
    }
}
