use crate::Catalog;
use crate::Modality;
use crate::ModelDef;
use crate::WireApi;
use serde_json::json;
use serde_json::Value;

/// The pi-ai `Api` id a wire protocol is named by.
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

pub fn catalog_json(catalog: &Catalog, provider: Option<&str>) -> Value {
    let providers: Vec<&str> = catalog.providers();
    let models: Vec<Value> = match provider {
        Some(provider) => catalog.by_provider(provider).map(model_json).collect(),
        None => catalog.models().iter().map(model_json).collect(),
    };
    json!({ "models": models, "providers": providers })
}
