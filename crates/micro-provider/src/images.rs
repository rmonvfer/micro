//! Image generation over OpenRouter's chat completions endpoint.

use crate::typed::AssistantImages;
use crate::typed::ImagesContent;
use crate::typed::ImagesContext;
use crate::typed::PricedUsage;
use micro_models::Modality;
use micro_models::ModelDef;
use micro_models::TokenUsage;
use micro_models::WireApi;
use serde_json::json;
use serde_json::Value;

/// Ask an image model for images. A failure is reported in the answer, never as an error, so the
/// caller always has the model and the stop reason to show.
pub async fn generate_images(
    http: &reqwest::Client,
    model: &ModelDef,
    context: &ImagesContext,
    api_key: &str,
) -> AssistantImages {
    let answer = AssistantImages::empty(model);
    if model.api != WireApi::OpenrouterImages {
        return answer.failed(format!("{} is not an image model", model.qualified_id()));
    }
    if api_key.trim().is_empty() {
        return answer.failed(format!("No API key for provider: {}", model.provider));
    }

    let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
    let body =
        match crate::request::post_json(http, model, &url, Some(api_key), &payload(model, context))
            .await
        {
            Ok(body) => body,
            Err(error) => return answer.failed(error),
        };
    read_answer(answer, model, &body)
}

/// The request: one user message carrying the prompt and any input images.
pub(crate) fn payload(model: &ModelDef, context: &ImagesContext) -> Value {
    let content: Vec<Value> = context
        .input
        .iter()
        .map(|item| match item {
            ImagesContent::Text { text } => json!({ "type": "text", "text": text }),
            ImagesContent::Image { data, mime_type } => json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime_type};base64,{data}") },
            }),
        })
        .collect();
    let modalities = match model.output.contains(&Modality::Text) {
        true => json!(["image", "text"]),
        false => json!(["image"]),
    };
    json!({
        "model": model.id,
        "messages": [{ "role": "user", "content": content }],
        "stream": false,
        "modalities": modalities,
    })
}

fn read_answer(mut answer: AssistantImages, model: &ModelDef, body: &Value) -> AssistantImages {
    answer.response_id = body.get("id").and_then(Value::as_str).map(str::to_string);
    if let Some(usage) = body.get("usage").filter(|usage| usage.is_object()) {
        answer.usage = Some(PricedUsage::priced(model, completion_tokens(usage)));
    }

    let Some(message) = body.pointer("/choices/0/message") else {
        return answer;
    };
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            answer.output.push(ImagesContent::Text {
                text: text.to_string(),
            });
        }
    }
    for image in message
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let url = image.get("image_url").and_then(|url| {
            url.as_str()
                .or_else(|| url.get("url").and_then(Value::as_str))
        });
        if let Some((mime_type, data)) = url.and_then(data_url) {
            answer.output.push(ImagesContent::Image {
                data: data.to_string(),
                mime_type: mime_type.to_string(),
            });
        }
    }
    answer
}

/// The media type and base64 payload of a `data:` URL.
fn data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (mime_type, data) = rest.split_once(";base64,")?;
    (!mime_type.is_empty() && !data.is_empty()).then_some((mime_type, data))
}

/// Token counts from an OpenAI-shaped `usage` object, with cache reads and writes taken out of the
/// prompt count they are included in.
pub(crate) fn completion_tokens(usage: &Value) -> TokenUsage {
    let count = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    let prompt = count("/prompt_tokens");
    let reported_cached = count("/prompt_tokens_details/cached_tokens");
    let cache_write = count("/prompt_tokens_details/cache_write_tokens");
    let cache_read = match cache_write > 0 {
        true => reported_cached.saturating_sub(cache_write),
        false => reported_cached,
    };
    let input = prompt.saturating_sub(cache_read + cache_write);
    TokenUsage::new(input, count("/completion_tokens")).with_cache(cache_read, cache_write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painter(output: Vec<Modality>) -> ModelDef {
        let mut model = micro_models::Catalog::from_json(
            r#"{"providers": {"openrouter": {
                "base_url": "https://openrouter.ai/api/v1",
                "models": [{"id": "a/painter", "api": "openrouter-images",
                            "cost": {"input": 1, "output": 30}}]
            }}}"#,
        )
        .unwrap()
        .get_of_type(micro_models::ModelType::Image, "openrouter", "a/painter")
        .unwrap()
        .clone();
        model.output = output;
        model
    }

    /// A model that can also write is asked for text as well as images.
    #[test]
    fn the_request_asks_for_what_the_model_returns() {
        let context = ImagesContext {
            input: vec![
                ImagesContent::Text {
                    text: "a fox".into(),
                },
                ImagesContent::Image {
                    data: "AAAA".into(),
                    mime_type: "image/png".into(),
                },
            ],
        };

        let both = payload(&painter(vec![Modality::Image, Modality::Text]), &context);
        assert_eq!(both["modalities"], json!(["image", "text"]));
        assert_eq!(
            both["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(both["stream"], false);

        let only = payload(&painter(vec![Modality::Image]), &context);
        assert_eq!(only["modalities"], json!(["image"]));
    }

    #[test]
    fn images_and_text_are_read_out_of_the_answer() {
        let model = painter(vec![Modality::Image, Modality::Text]);
        let body = json!({
            "id": "gen-1",
            "choices": [{ "message": {
                "content": "Here it is",
                "images": [
                    { "image_url": { "url": "data:image/png;base64,iVBOR" } },
                    { "image_url": "data:image/jpeg;base64,/9j/" },
                    { "image_url": { "url": "https://elsewhere.example/x.png" } },
                ],
            }}],
            "usage": { "prompt_tokens": 10, "completion_tokens": 1000 },
        });

        let answer = read_answer(AssistantImages::empty(&model), &model, &body);
        assert_eq!(answer.response_id.as_deref(), Some("gen-1"));
        assert_eq!(
            answer.output,
            vec![
                ImagesContent::Text {
                    text: "Here it is".into()
                },
                ImagesContent::Image {
                    data: "iVBOR".into(),
                    mime_type: "image/png".into()
                },
                ImagesContent::Image {
                    data: "/9j/".into(),
                    mime_type: "image/jpeg".into()
                },
            ]
        );
        let usage = answer.usage.unwrap();
        assert_eq!(usage.input, 10);
        assert_eq!(usage.output, 1000);
        assert!((usage.cost.total - (10.0 * 1.0 + 1000.0 * 30.0) / 1e6).abs() < 1e-12);
    }

    #[test]
    fn cached_prompt_tokens_are_not_counted_twice() {
        let tokens = completion_tokens(&json!({
            "prompt_tokens": 100,
            "completion_tokens": 5,
            "prompt_tokens_details": { "cached_tokens": 60, "cache_write_tokens": 20 },
        }));
        assert_eq!(tokens.input, 40);
        assert_eq!(tokens.cache_read, 40);
        assert_eq!(tokens.cache_write, 20);
    }
}
