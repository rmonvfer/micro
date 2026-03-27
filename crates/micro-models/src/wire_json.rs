//! The catalog, in the shape pi's own `getBuiltinModel`/`getBuiltinModels`/
//! `getBuiltinProviders` expect: camelCase fields and the api id strings pi-ai itself uses,
//! rather than this crate's own snake_case `Serialize` derive.
//!
//! Two callers need exactly this shape and must never quietly disagree about it: micro-cli's
//! `model_catalog` wire request answers it live, per turn, and micro-extensions' compat
//! layer writes the same shape once, to a static file a pi extension's own synchronous
//! `getBuiltinModel` reads directly rather than asking over the wire. One function here is
//! what keeps those two answers identical.

use crate::Catalog;
use crate::Modality;
use crate::ModelDef;
use crate::WireApi;
use serde_json::json;
use serde_json::Value;

/// The pi-ai `Api` id a wire protocol is named by — every `OpenaiResponses` model in
/// micro's own catalog reports as `"openai-responses"` regardless of whether resolving it
/// would route to a Codex or Azure client underneath.
pub fn wire_api_name(api: WireApi) -> &'static str {
    match api {
        WireApi::AnthropicMessages => "anthropic-messages",
        WireApi::OpenaiCompletions => "openai-completions",
        WireApi::OpenaiResponses => "openai-responses",
        WireApi::GoogleGenerativeAi => "google-generative-ai",
        WireApi::GoogleVertex => "google-vertex",
        WireApi::BedrockConverseStream => "bedrock-converse-stream",
    }
}

pub fn modality_name(modality: Modality) -> &'static str {
    match modality {
        Modality::Text => "text",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Pdf => "pdf",
    }
}

/// One model, in the shape pi-ai's own catalog entries take.
pub fn model_json(def: &ModelDef) -> Value {
    json!({
        "id": def.id,
        "name": def.name,
        "provider": def.provider,
        "api": wire_api_name(def.api),
        "baseUrl": def.base_url,
        "contextWindow": def.context_window,
        "maxTokens": def.max_output_tokens,
        "reasoning": def.reasoning,
        "input": def.input.iter().copied().map(modality_name).collect::<Vec<_>>(),
        "cost": {
            "input": def.cost.input,
            "output": def.cost.output,
            "cacheRead": def.cost.cache_read,
            "cacheWrite": def.cost.cache_write,
        },
    })
}

/// The catalog `getBuiltinModel`/`getBuiltinModels`/`getBuiltinProviders` are built from:
/// `providers` always lists every provider the catalog carries, so a caller can tell "no
/// provider by that name" apart from "that provider has no models"; `models` is every
/// model, or only one provider's when `provider` narrows it.
pub fn catalog_json(catalog: &Catalog, provider: Option<&str>) -> Value {
    let providers: Vec<&str> = catalog.providers();
    let models: Vec<Value> = match provider {
        Some(provider) => catalog.by_provider(provider).map(model_json).collect(),
        None => catalog.models().iter().map(model_json).collect(),
    };
    json!({ "models": models, "providers": providers })
}
