//! The `models` global of `codemode` scripts: the catalog, and classification and image
//! generation run with the session's credentials.

use async_trait::async_trait;
use micro_codemode::GlobalSpend;
use micro_codemode::ScriptGlobals;
use micro_models::ModelDef;
use micro_models::ModelType;
use micro_provider::ClassifierContext;
use micro_provider::ClassifyOptions;
use micro_provider::ImagesContext;
use micro_provider::ModelRuntime;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// How many classifier and image requests one script may have in flight; more wait their turn.
const CONCURRENT_MODEL_CALLS: usize = 4;

const GET_MODELS_OF_TYPE: &str = "models.getModelsOfType";
const GET_AVAILABLE_OF_TYPE: &str = "models.getAvailableOfType";
const GET_MODEL_OF_TYPE: &str = "models.getModelOfType";
const CLASSIFY: &str = "models.classify";
const GENERATE_IMAGES: &str = "models.generateImages";

const TYPES: &str = "\"chat\", \"image\" or \"classifier\"";

const CLASSIFY_SHAPE: &str = "models.classify(model, { state: { ... }, questions: { id: { type: \"choice\", instructions, criteria: { label: meaning } } | { type: \"score\", instructions, criteria: [lowest, ..., highest] } | { type: \"bool\", instructions, criteria: { true, false } } } })";

const GENERATE_SHAPE: &str = "models.generateImages(model, { input: [{ type: \"text\", text }, { type: \"image\", data, mimeType }] })";

/// The `models` namespace scripts call.
pub struct ModelGlobals {
    models: ModelRuntime,
    slots: Arc<Semaphore>,
}

impl ModelGlobals {
    pub fn new(models: ModelRuntime) -> ModelGlobals {
        ModelGlobals {
            models,
            slots: Arc::new(Semaphore::new(CONCURRENT_MODEL_CALLS)),
        }
    }

    fn model_type(&self, called: &str, value: Option<&Value>) -> Result<ModelType, String> {
        value
            .and_then(Value::as_str)
            .and_then(ModelType::parse)
            .ok_or_else(|| format!("{called}() expects a model type first: {TYPES}"))
    }

    /// The models a listing names, optionally of one provider, as scripts see them.
    fn listed(&self, models: Vec<ModelDef>, provider: Option<&Value>) -> Result<Value, String> {
        let provider = match provider {
            None | Some(Value::Null) => None,
            Some(Value::String(provider)) => Some(provider.as_str()),
            Some(_) => return Err("the provider, when given, is a string".to_string()),
        };
        Ok(Value::Array(
            models
                .iter()
                .filter(|model| provider.is_none_or(|wanted| model.provider == wanted))
                .map(micro_models::model_json)
                .collect(),
        ))
    }

    /// The model a call names by `{ provider, id }`, which must be of `kind`.
    fn runnable(
        &self,
        called: &str,
        kind: ModelType,
        value: Option<&Value>,
    ) -> Result<ModelDef, String> {
        let provider = value
            .and_then(|model| model.get("provider"))
            .and_then(Value::as_str);
        let id = value
            .and_then(|model| model.get("id"))
            .and_then(Value::as_str);
        let (Some(provider), Some(id)) = (provider, id) else {
            return Err(format!(
                "{called}() expects a model first, such as one from models.getModelOfType(\"{}\", provider, id), or {{ provider, id }}",
                kind.as_str()
            ));
        };
        if let Some(found) = self.models.find_of_type(kind, provider, id) {
            return Ok(found);
        }
        let other = [ModelType::Chat, ModelType::Image, ModelType::Classifier]
            .into_iter()
            .find(|other| {
                *other != kind && self.models.find_of_type(*other, provider, id).is_some()
            });
        Err(match other {
            Some(other) => format!(
                "{provider}/{id} is {} model, not {} model. models.getAvailableOfType(\"{}\") lists the {} models that work with your credentials.",
                with_article(other),
                with_article(kind),
                kind.as_str(),
                kind.as_str()
            ),
            None => format!(
                "There is no {} model {provider}/{id}. models.getAvailableOfType(\"{}\") lists the ones that work with your credentials.",
                kind.as_str(),
                kind.as_str()
            ),
        })
    }

    async fn classify(&self, arguments: &[Value]) -> Result<Option<Value>, String> {
        let model = self.runnable(CLASSIFY, ModelType::Classifier, arguments.first())?;
        let context: ClassifierContext = arguments
            .get(1)
            .cloned()
            .ok_or_else(|| format!("expected {CLASSIFY_SHAPE}"))
            .and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|error| format!("{error}; expected {CLASSIFY_SHAPE}"))
            })?;
        if !context.state.is_object() {
            return Err(format!(
                "state must be an object; expected {CLASSIFY_SHAPE}"
            ));
        }
        if context.questions.is_empty() {
            return Err(format!(
                "ask at least one question; expected {CLASSIFY_SHAPE}"
            ));
        }
        let options: ClassifyOptions = match arguments.get(2) {
            None | Some(Value::Null) => ClassifyOptions::default(),
            Some(options) => serde_json::from_value(options.clone())
                .map_err(|error| format!("the options are {{ temperature }}: {error}"))?,
        };
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|error| error.to_string())?;
        let result = self.models.classify(&model, &context, options).await;
        Ok(Some(json!(result)))
    }

    async fn generate_images(&self, arguments: &[Value]) -> Result<Option<Value>, String> {
        let model = self.runnable(GENERATE_IMAGES, ModelType::Image, arguments.first())?;
        let context: ImagesContext = arguments
            .get(1)
            .cloned()
            .ok_or_else(|| format!("expected {GENERATE_SHAPE}"))
            .and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|error| format!("{error}; expected {GENERATE_SHAPE}"))
            })?;
        if context.input.is_empty() {
            return Err(format!(
                "give the model a prompt; expected {GENERATE_SHAPE}"
            ));
        }
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|error| error.to_string())?;
        let result = self.models.generate_images(&model, &context).await;
        Ok(Some(json!(result)))
    }
}

#[async_trait]
impl ScriptGlobals for ModelGlobals {
    fn names(&self) -> Vec<String> {
        [
            GET_MODELS_OF_TYPE,
            GET_AVAILABLE_OF_TYPE,
            GET_MODEL_OF_TYPE,
            CLASSIFY,
            GENERATE_IMAGES,
        ]
        .iter()
        .map(|name| name.to_string())
        .collect()
    }

    fn describe(&self) -> String {
        "`models`: `getModelsOfType(type, provider?)`, `getAvailableOfType(type, provider?)` and \
         `getModelOfType(type, provider, id)` read the catalog (type is \"chat\", \"image\" or \
         \"classifier\"); `classify(model, { state, questions })` answers typed questions with a \
         classifier and `generateImages(model, { input })` returns base64 image blocks to show with \
         `image()`. Neither throws on provider errors: check `stopReason` and `errorMessage`. Their \
         usage counts toward the session cost."
            .to_string()
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<Option<Value>, String> {
        let arguments = arguments.as_array().cloned().unwrap_or_default();
        match name {
            GET_MODELS_OF_TYPE => {
                let kind = self.model_type(name, arguments.first())?;
                self.listed(self.models.models_of_type(kind), arguments.get(1))
                    .map(Some)
            }
            GET_AVAILABLE_OF_TYPE => {
                let kind = self.model_type(name, arguments.first())?;
                self.listed(self.models.available_of_type(kind), arguments.get(1))
                    .map(Some)
            }
            GET_MODEL_OF_TYPE => {
                let kind = self.model_type(name, arguments.first())?;
                let (Some(provider), Some(id)) = (
                    arguments.get(1).and_then(Value::as_str),
                    arguments.get(2).and_then(Value::as_str),
                ) else {
                    return Err(format!(
                        "{name}(type, provider, id) expects a provider and an id"
                    ));
                };
                Ok(self
                    .models
                    .find_of_type(kind, provider, id)
                    .map(|model| micro_models::model_json(&model)))
            }
            CLASSIFY => self.classify(&arguments).await,
            GENERATE_IMAGES => self.generate_images(&arguments).await,
            other => Err(format!("Unknown global \"{other}\"")),
        }
    }

    fn spent(&self, name: &str, result: &Value) -> Option<GlobalSpend> {
        let operation = match name {
            CLASSIFY => "classify",
            GENERATE_IMAGES => "generateImages",
            _ => return None,
        };
        let images = result
            .get("output")
            .and_then(Value::as_array)
            .map(|output| {
                output
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
                    .count()
            })
            .unwrap_or(0);
        let usage = result.get("usage");
        if usage.is_none() && images == 0 {
            return None;
        }
        let count = |key: &str| {
            usage
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64)
                .map(|count| count.min(u32::MAX as u64) as u32)
                .unwrap_or(0)
        };
        Some(GlobalSpend {
            label: format!(
                "{operation} {}/{}",
                result
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                result
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ),
            usage: micro_types::Usage {
                input: count("input"),
                output: count("output"),
                cache_read: count("cacheRead"),
                cache_write: count("cacheWrite"),
            },
            cost: usage
                .and_then(|usage| usage.pointer("/cost/total"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            images,
        })
    }
}

/// "an image model", "a chat model".
fn with_article(kind: ModelType) -> String {
    match kind {
        ModelType::Image => "an image".to_string(),
        other => format!("a {}", other.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals() -> ModelGlobals {
        let dir =
            std::env::temp_dir().join(format!("micro-codemode-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = micro_auth::AuthStore::open_at(dir.join("auth.json")).unwrap();
        store
            .set("opencode", micro_auth::Credential::api_key("oc-key"))
            .unwrap();
        ModelGlobals::new(ModelRuntime::new(
            micro_models::Catalog::bundled(),
            Arc::new(store),
        ))
    }

    #[tokio::test]
    async fn scripts_read_the_catalog_by_type() {
        let globals = globals();
        let jev = globals
            .call(
                GET_MODEL_OF_TYPE,
                json!(["classifier", "typesafe", "jev-latest"]),
            )
            .await
            .unwrap()
            .expect("TypeSafe's Jev is listed");
        assert_eq!(jev["type"], "classifier");

        let available = globals
            .call(GET_AVAILABLE_OF_TYPE, json!(["classifier", "opencode"]))
            .await
            .unwrap()
            .unwrap();
        let ids: Vec<&str> = available
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["id"].as_str())
            .collect();
        assert!(ids.contains(&"jev-1.13"), "{ids:?}");

        assert_eq!(
            globals
                .call(
                    GET_MODEL_OF_TYPE,
                    json!(["image", "typesafe", "jev-latest"])
                )
                .await
                .unwrap(),
            None
        );
    }

    /// A malformed call says what was expected and how to find a model that works.
    #[tokio::test]
    async fn malformed_calls_say_how_to_recover() {
        let globals = globals();
        let wrong_type = globals
            .call(GET_MODELS_OF_TYPE, json!(["llm"]))
            .await
            .unwrap_err();
        assert!(wrong_type.contains("\"classifier\""), "{wrong_type}");

        let unknown = globals
            .call(
                CLASSIFY,
                json!([{ "provider": "typesafe", "id": "jev-9" }, {}]),
            )
            .await
            .unwrap_err();
        assert!(
            unknown.contains("getAvailableOfType(\"classifier\")"),
            "{unknown}"
        );

        let wrong_kind = globals
            .call(
                GENERATE_IMAGES,
                json!([{ "provider": "typesafe", "id": "jev-latest" }, { "input": [] }]),
            )
            .await
            .unwrap_err();
        assert!(
            wrong_kind.contains("is a classifier model, not an image model"),
            "{wrong_kind}"
        );

        let shapeless = globals
            .call(
                CLASSIFY,
                json!([{ "provider": "typesafe", "id": "jev-latest" }, { "state": {} }]),
            )
            .await
            .unwrap_err();
        assert!(shapeless.contains("questions"), "{shapeless}");
    }

    #[test]
    fn a_model_call_reports_what_it_spent_and_drew() {
        let globals = globals();
        let spent = globals
            .spent(
                GENERATE_IMAGES,
                &json!({
                    "provider": "openrouter",
                    "model": "acme/painter",
                    "output": [{ "type": "image", "data": "x", "mimeType": "image/png" }],
                    "usage": { "input": 10, "output": 1000, "totalTokens": 1010, "cost": { "total": 0.03 } },
                }),
            )
            .unwrap();
        assert_eq!(spent.label, "generateImages openrouter/acme/painter");
        assert_eq!(spent.usage.output, 1000);
        assert_eq!(spent.images, 1);
        assert!((spent.cost - 0.03).abs() < 1e-12);
        assert!(globals.spent(GET_MODELS_OF_TYPE, &json!([])).is_none());
    }
}
