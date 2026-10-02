use crate::Catalog;
use crate::Modality;
use crate::ModelDef;
use crate::ModelType;
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
        WireApi::OpenrouterImages => "openrouter-images",
        WireApi::TypesafeSystemOne => "typesafe-system-one",
        WireApi::CloudflareWorkersAiSystemOne => "cloudflare-workers-ai-system-one",
        WireApi::LlamaCppClassify => "llama-cpp-classify",
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
    let mut described = json!({
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
    });
    // Chat is the default type, so chat entries leave `type` out as pi-ai's own do.
    if def.kind() != ModelType::Chat {
        described["type"] = json!(def.kind().as_str());
    }
    if def.kind() == ModelType::Image {
        described["output"] = json!(def
            .output
            .iter()
            .copied()
            .map(modality_name)
            .collect::<Vec<_>>());
    }
    described
}

pub fn catalog_json(catalog: &Catalog, provider: Option<&str>) -> Value {
    let providers: Vec<&str> = catalog.providers();
    let models: Vec<Value> = match provider {
        Some(provider) => catalog.by_provider(provider).map(model_json).collect(),
        None => catalog.models().iter().map(model_json).collect(),
    };
    json!({ "models": models, "providers": providers })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image model says what it is and what it returns; a chat model is described as pi-ai
    /// describes one, without a type.
    #[test]
    fn a_typed_model_names_its_type() {
        let catalog = Catalog::from_json(
            r#"{"providers": {"openrouter": {
                "base_url": "https://openrouter.ai/api/v1",
                "api": "openai-completions",
                "models": [
                    {"id": "a/chat"},
                    {"id": "a/painter", "api": "openrouter-images", "output": ["image", "text"]}
                ]
            }}}"#,
        )
        .unwrap();

        let chat = model_json(catalog.get("openrouter", "a/chat").unwrap());
        assert!(chat.get("type").is_none());

        let painter = model_json(
            catalog
                .get_of_type(ModelType::Image, "openrouter", "a/painter")
                .unwrap(),
        );
        assert_eq!(painter["type"], "image");
        assert_eq!(painter["api"], "openrouter-images");
        assert_eq!(painter["output"], json!(["image", "text"]));
    }
}
