//! What `ctx.modelRegistry` asks micro: the catalog, and image generation and classification with
//! the session's credentials, billed to the session.

use micro_models::ModelDef;
use micro_models::ModelType;
use micro_provider::ClassifierContext;
use micro_provider::ClassifyOptions;
use micro_provider::ImagesContext;
use micro_provider::ModelRuntime;
use micro_provider::PricedUsage;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Every model of every type, each marked with whether its provider has a credential.
pub(crate) fn catalog(models: Option<&ModelRuntime>) -> Value {
    let Some(models) = models else {
        return json!({ "models": [] });
    };
    let described: Vec<Value> = models
        .all_models()
        .iter()
        .map(|model| {
            let mut described = micro_models::model_json(model);
            described["available"] = json!(models.has_credential(&model.provider));
            described
        })
        .collect();
    json!({ "models": described })
}

/// The model a request names by `{ provider, id }`, of the type the operation needs.
fn named(models: &ModelRuntime, payload: &Value, kind: ModelType) -> Result<ModelDef, String> {
    let reference = payload.get("model").unwrap_or(&Value::Null);
    let provider = reference.get("provider").and_then(Value::as_str);
    let id = reference.get("id").and_then(Value::as_str);
    let (Some(provider), Some(id)) = (provider, id) else {
        return Err("name a model by its provider and id".to_string());
    };
    models.find_of_type(kind, provider, id).ok_or_else(|| {
        format!(
            "no {} model {provider}/{id}; ctx.modelRegistry.getAvailableOfType(\"{}\") lists them",
            kind.as_str(),
            kind.as_str()
        )
    })
}

/// Generate images, billing what the request used to the session.
pub(crate) async fn generate_images(
    models: Option<&ModelRuntime>,
    payload: &Value,
    requested_by: &str,
    session: &Arc<Mutex<micro_session::Session>>,
) -> Value {
    let Some(models) = models else {
        return json!({ "error": "no models are available to this run" });
    };
    let model = match named(models, payload, ModelType::Image) {
        Ok(model) => model,
        Err(error) => return json!({ "error": error }),
    };
    let context: ImagesContext = match serde_json::from_value(
        payload.get("context").cloned().unwrap_or(Value::Null),
    ) {
        Ok(context) => context,
        Err(error) => {
            return json!({
                "error": format!(
                    "generateImages expects {{ input: [{{ type: \"text\", text }} | {{ type: \"image\", data, mimeType }}] }}: {error}"
                ),
            })
        }
    };
    let answer = models.generate_images(&model, &context).await;
    bill(
        session,
        "generate_images",
        requested_by,
        &model,
        answer.usage.as_ref(),
    )
    .await;
    json!({ "result": answer })
}

/// Classify, billing what the request used to the session.
pub(crate) async fn classify(
    models: Option<&ModelRuntime>,
    payload: &Value,
    requested_by: &str,
    session: &Arc<Mutex<micro_session::Session>>,
) -> Value {
    let Some(models) = models else {
        return json!({ "error": "no models are available to this run" });
    };
    let model = match named(models, payload, ModelType::Classifier) {
        Ok(model) => model,
        Err(error) => return json!({ "error": error }),
    };
    let context: ClassifierContext = match serde_json::from_value(
        payload.get("context").cloned().unwrap_or(Value::Null),
    ) {
        Ok(context) => context,
        Err(error) => {
            return json!({
                "error": format!(
                    "classify expects {{ state: {{...}}, questions: {{ id: {{ type: \"choice\" | \"score\" | \"bool\", instructions, criteria }} }} }}: {error}"
                ),
            })
        }
    };
    let options: ClassifyOptions =
        serde_json::from_value(payload.get("options").cloned().unwrap_or(json!({})))
            .unwrap_or_default();
    let result = models.classify(&model, &context, options).await;
    bill(
        session,
        "classify",
        requested_by,
        &model,
        result.usage.as_ref(),
    )
    .await;
    json!({ "result": result })
}

/// Put what a model call used in the ledger, so it counts toward the session's cost.
async fn bill(
    session: &Arc<Mutex<micro_session::Session>>,
    operation: &str,
    requested_by: &str,
    model: &ModelDef,
    usage: Option<&PricedUsage>,
) {
    let Some(usage) = usage.filter(|usage| usage.total_tokens > 0) else {
        return;
    };
    let event = micro_provider::model_call_event(operation, requested_by, model, usage);
    if let Err(error) = session.lock().await.append_event(event).await {
        eprintln!(
            "note: what {requested_by} spent on {} was not recorded: {error}",
            model.qualified_id()
        );
    }
}
