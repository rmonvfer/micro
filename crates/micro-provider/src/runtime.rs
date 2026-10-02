//! Every model micro knows, of every type, and the operations each type accepts, with the
//! credential each request needs resolved at the time it is made.
//!
//! This is what extensions reach through `ctx.modelRegistry`, and what a script runtime calls
//! for `models.generateImages()` and `models.classify()`.

use crate::typed::AssistantImages;
use crate::typed::ClassifierContext;
use crate::typed::ClassifierResult;
use crate::typed::ClassifyOptions;
use crate::typed::ImagesContext;
use micro_auth::AuthStore;
use micro_models::Catalog;
use micro_models::ModelDef;
use micro_models::ModelType;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;

/// The catalog and the credentials, shared by everything in a run that calls a model.
#[derive(Clone)]
pub struct ModelRuntime {
    catalog: Arc<RwLock<Catalog>>,
    store: Arc<AuthStore>,
    /// Keys that came with a provider an extension declared, which the store does not keep.
    keys: Arc<RwLock<BTreeMap<String, String>>>,
    http: reqwest::Client,
}

impl std::fmt::Debug for ModelRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRuntime").finish_non_exhaustive()
    }
}

/// The ledger entry that bills one image or classifier request to the session, at the rates of
/// the model that served it.
pub fn model_call_event(
    operation: &str,
    requested_by: &str,
    model: &ModelDef,
    usage: &crate::typed::PricedUsage,
) -> micro_types::LedgerEvent {
    micro_types::LedgerEvent::ModelCall {
        operation: operation.to_string(),
        requested_by: requested_by.to_string(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        usage: micro_types::Usage {
            input: usage.input.min(u32::MAX as u64) as u32,
            output: usage.output.min(u32::MAX as u64) as u32,
            cache_read: usage.cache_read.min(u32::MAX as u64) as u32,
            cache_write: usage.cache_write.min(u32::MAX as u64) as u32,
        },
        pricing: Some((&model.cost).into()),
    }
}

impl ModelRuntime {
    pub fn new(catalog: Catalog, store: Arc<AuthStore>) -> ModelRuntime {
        ModelRuntime {
            catalog: Arc::new(RwLock::new(catalog)),
            store,
            keys: Arc::default(),
            http: crate::http_client(),
        }
    }

    /// Use these keys for these providers ahead of anything stored.
    pub fn with_keys(self, keys: BTreeMap<String, String>) -> ModelRuntime {
        if let Ok(mut held) = self.keys.write() {
            held.extend(keys);
        }
        self
    }

    /// A copy of the catalog as it stands.
    pub fn catalog(&self) -> Catalog {
        self.catalog
            .read()
            .map(|catalog| catalog.clone())
            .unwrap_or_default()
    }

    /// Change the catalog in place, for a listing that arrived after the run started.
    pub fn update_catalog(&self, change: impl FnOnce(&mut Catalog)) {
        if let Ok(mut catalog) = self.catalog.write() {
            change(&mut catalog);
        }
    }

    pub fn store(&self) -> &Arc<AuthStore> {
        &self.store
    }

    /// The models of one type.
    pub fn models_of_type(&self, kind: ModelType) -> Vec<ModelDef> {
        self.read(|catalog| catalog.models_of_type(kind).into_iter().cloned().collect())
    }

    /// Every model of every type.
    pub fn all_models(&self) -> Vec<ModelDef> {
        self.read(|catalog| catalog.all_models().cloned().collect())
    }

    /// One model of one type.
    pub fn find_of_type(&self, kind: ModelType, provider: &str, id: &str) -> Option<ModelDef> {
        self.read(|catalog| catalog.get_of_type(kind, provider, id).cloned())
    }

    /// The models of one type whose provider has a credential.
    pub fn available_of_type(&self, kind: ModelType) -> Vec<ModelDef> {
        self.models_of_type(kind)
            .into_iter()
            .filter(|model| self.has_credential(&model.provider))
            .collect()
    }

    /// Every model whose provider has a credential, of every type.
    pub fn all_available(&self) -> Vec<ModelDef> {
        self.all_models()
            .into_iter()
            .filter(|model| self.has_credential(&model.provider))
            .collect()
    }

    /// Whether a request to `provider` has something to authenticate it with, without asking the
    /// network.
    pub fn has_credential(&self, provider: &str) -> bool {
        if crate::llama_cpp::is_keyless(provider) {
            return true;
        }
        let declared = self
            .keys
            .read()
            .map(|keys| keys.get(provider).is_some_and(|key| !key.trim().is_empty()))
            .unwrap_or(false);
        declared
            || self.store.get(provider).is_some()
            || micro_auth::env_names(provider).iter().any(|name| {
                std::env::var(name)
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty())
            })
    }

    /// The credential to send to `provider` now, refreshed if it has expired. A provider that
    /// needs none answers `None`.
    pub async fn credential(&self, provider: &str) -> Result<Option<String>, String> {
        let declared = self
            .keys
            .read()
            .ok()
            .and_then(|keys| keys.get(provider).cloned());
        if let Some(key) = declared.filter(|key| !key.trim().is_empty()) {
            return Ok(Some(key));
        }
        if crate::llama_cpp::is_keyless(provider) {
            return Ok(crate::llama_cpp::api_key(&self.store));
        }
        self.store
            .resolve(provider)
            .await
            .map(|credential| Some(credential.token().to_string()))
            .map_err(|error| error.to_string())
    }

    /// Generate images with an image model, authenticated as its provider.
    pub async fn generate_images(
        &self,
        model: &ModelDef,
        context: &ImagesContext,
    ) -> AssistantImages {
        match self.credential(&model.provider).await {
            Ok(key) => {
                crate::images::generate_images(
                    &self.http,
                    model,
                    context,
                    key.as_deref().unwrap_or_default(),
                )
                .await
            }
            Err(error) => AssistantImages::empty(model).failed(error),
        }
    }

    /// Answer typed questions with a classifier model, authenticated as its provider.
    pub async fn classify(
        &self,
        model: &ModelDef,
        context: &ClassifierContext,
        options: ClassifyOptions,
    ) -> ClassifierResult {
        match self.credential(&model.provider).await {
            Ok(key) => {
                crate::classify::classify(&self.http, model, context, options, key.as_deref()).await
            }
            Err(error) => ClassifierResult::empty(model).failed(error),
        }
    }

    fn read<T: Default>(&self, read: impl FnOnce(&Catalog) -> T) -> T {
        self.catalog
            .read()
            .map(|catalog| read(&catalog))
            .unwrap_or_default()
    }
}
