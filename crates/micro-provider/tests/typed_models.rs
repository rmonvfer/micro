//! Image generation, classification and the llama.cpp router, against services on loopback.

mod support;

use micro_auth::AuthStore;
use micro_auth::Credential;
use micro_models::Catalog;
use micro_models::ModelType;
use micro_provider::llama_cpp::LlamaClient;
use micro_provider::ClassifierAnswer;
use micro_provider::ClassifierContext;
use micro_provider::ClassifyOptions;
use micro_provider::ImagesContent;
use micro_provider::ImagesContext;
use micro_provider::ModelRuntime;
use micro_provider::Outcome;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use support::JsonServer;

fn store(name: &str, keys: &[(&str, &str)]) -> Arc<AuthStore> {
    let dir =
        std::env::temp_dir().join(format!("micro-typed-models-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = AuthStore::open_at(dir.join("auth.json")).unwrap();
    for (provider, key) in keys {
        store.set(provider, Credential::api_key(*key)).unwrap();
    }
    Arc::new(store)
}

fn catalog(json: String) -> Catalog {
    Catalog::from_json(&json).unwrap()
}

/// An image model shares its provider's credential, and what it drew comes back as base64 blocks
/// priced at the model's rates.
#[tokio::test]
async fn an_image_model_draws_with_its_providers_credential() {
    let server = JsonServer::start(|_| {
        (
            200,
            json!({
                "id": "gen-7",
                "choices": [{ "message": {
                    "content": "",
                    "images": [{ "image_url": { "url": "data:image/png;base64,iVBORw0KGgo=" } }],
                }}],
                "usage": { "prompt_tokens": 12, "completion_tokens": 1290 },
            }),
        )
    })
    .await;
    let runtime = ModelRuntime::new(
        catalog(format!(
            r#"{{"providers": {{"openrouter": {{
                "base_url": "{}/api/v1", "api": "openai-completions",
                "models": [
                    {{"id": "acme/chat"}},
                    {{"id": "acme/painter", "api": "openrouter-images", "input": ["text", "image"],
                      "cost": {{"input": 0.3, "output": 30}}}}
                ]
            }}}}}}"#,
            server.url
        )),
        store("images", &[("openrouter", "sk-or-test")]),
    );

    let painter = runtime
        .find_of_type(ModelType::Image, "openrouter", "acme/painter")
        .expect("the image model is listed under its provider");
    assert!(runtime
        .find_of_type(ModelType::Chat, "openrouter", "acme/painter")
        .is_none());

    let answer = runtime
        .generate_images(
            &painter,
            &ImagesContext {
                input: vec![ImagesContent::Text {
                    text: "A red fox in the snow".into(),
                }],
            },
        )
        .await;

    assert_eq!(
        answer.stop_reason,
        Outcome::Stop,
        "{:?}",
        answer.error_message
    );
    assert_eq!(
        answer.output,
        vec![ImagesContent::Image {
            data: "iVBORw0KGgo=".into(),
            mime_type: "image/png".into()
        }]
    );
    let usage = answer.usage.expect("usage is reported");
    assert_eq!(usage.output, 1290);
    assert!((usage.cost.total - (12.0 * 0.3 + 1290.0 * 30.0) / 1e6).abs() < 1e-12);

    let sent = &server.seen()[0];
    assert_eq!(sent.path, "/api/v1/chat/completions");
    assert_eq!(sent.headers["authorization"], "Bearer sk-or-test");
    assert_eq!(sent.body["model"], "acme/painter");
    assert_eq!(sent.body["modalities"], json!(["image"]));
    assert_eq!(
        sent.body["messages"][0]["content"][0]["text"],
        "A red fox in the snow"
    );
}

/// A service that refuses is an answer with an error in it, never a panic or a missing answer.
#[tokio::test]
async fn a_refused_image_request_is_reported_in_the_answer() {
    let server =
        JsonServer::start(|_| (400, json!({ "error": { "message": "no such model" } }))).await;
    let runtime = ModelRuntime::new(
        catalog(format!(
            r#"{{"providers": {{"openrouter": {{"base_url": "{}/v1",
                "models": [{{"id": "acme/painter", "api": "openrouter-images"}}]}}}}}}"#,
            server.url
        )),
        store("images-refused", &[("openrouter", "sk-or-test")]),
    );
    let painter = runtime
        .find_of_type(ModelType::Image, "openrouter", "acme/painter")
        .unwrap();
    let answer = runtime
        .generate_images(&painter, &ImagesContext::default())
        .await;
    assert_eq!(answer.stop_reason, Outcome::Error);
    assert!(answer.error_message.unwrap().contains("no such model"));
}

/// TypeSafe's System One: a yes-or-no question travels as a `noul`, and the answers and the
/// tokens come back priced.
#[tokio::test]
async fn a_system_one_classifier_answers_every_question() {
    let server = JsonServer::start(|_| {
        (
            200,
            json!({
                "answers": {
                    "approved": { "type": "noul", "noul": 0.93 },
                    "tone": { "type": "choice", "choice": "warm",
                              "probabilities": { "warm": 0.8, "cold": 0.2 }, "confidence": 0.6 },
                },
                "usage": { "input_tokens": 1000, "output_tokens": 0 },
            }),
        )
    })
    .await;
    let runtime = ModelRuntime::new(
        catalog(format!(
            r#"{{"providers": {{"opencode": {{"base_url": "{}/zen/v1",
                "models": [{{"id": "jev-1.13", "api": "typesafe-system-one",
                             "cost": {{"input": 0.042}}}}]}}}}}}"#,
            server.url
        )),
        store("system-one", &[("opencode", "oc-key")]),
    );
    let jev = runtime
        .find_of_type(ModelType::Classifier, "opencode", "jev-1.13")
        .unwrap();
    let context: ClassifierContext = serde_json::from_str(
        r#"{"state": {"message": "The change works, thanks."},
            "questions": {
                "tone": {"type": "choice", "instructions": "Tone?", "criteria": {"warm": "", "cold": ""}},
                "approved": {"type": "bool", "instructions": "Approved?",
                             "criteria": {"true": "Approval", "false": "No approval"}}
            }}"#,
    )
    .unwrap();

    let result = runtime
        .classify(&jev, &context, ClassifyOptions::default())
        .await;

    assert_eq!(
        result.stop_reason,
        Outcome::Stop,
        "{:?}",
        result.error_message
    );
    assert_eq!(
        result.answers.keys().collect::<Vec<_>>(),
        vec!["tone", "approved"]
    );
    assert_eq!(
        result.answers.get("approved"),
        Some(&ClassifierAnswer::Bool { probability: 0.93 })
    );
    let usage = result.usage.unwrap();
    assert!((usage.cost.input - 1000.0 * 0.042 / 1e6).abs() < 1e-15);

    let sent = &server.seen()[0];
    assert_eq!(sent.path, "/zen/v1/systemone");
    assert_eq!(sent.headers["authorization"], "Bearer oc-key");
    assert_eq!(sent.body["model"], "jev-1.13");
    assert_eq!(sent.body["questions"]["approved"]["type"], "noul");
    assert_eq!(sent.body["state"]["message"], "The change works, thanks.");
}

/// A llama.cpp model answers from the probabilities of the label tokens it would emit next, one
/// question at a time, without generating anything.
#[tokio::test]
async fn a_llama_cpp_model_classifies_from_label_probabilities() {
    let completions = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&completions);
    let server = JsonServer::start(move |seen| match seen.path.as_str() {
        "/tokenize" => {
            let tokens = match seen.body["content"].as_str().unwrap() {
                "\n" => json!([10]),
                "\nYes" => json!([10, 100]),
                "\nNo" => json!([10, 101]),
                "\nA" => json!([10, 200]),
                "\nB" => json!([10, 201]),
                other => panic!("unexpected tokenization of {other:?}"),
            };
            (200, json!({ "tokens": tokens }))
        }
        "/apply-template" => {
            assert_eq!(seen.body["chat_template_kwargs"]["enable_thinking"], false);
            (200, json!({ "prompt": "<prompt><think>" }))
        }
        "/completion" => {
            counted.fetch_add(1, Ordering::SeqCst);
            assert_eq!(seen.body["prompt"], "<prompt><think></think>");
            assert_eq!(seen.body["n_predict"], 1);
            (
                200,
                json!({ "completion_probabilities": [{ "top_logprobs": [
                    { "id": 100, "logprob": -0.1 },
                    { "id": 101, "logprob": -2.5 },
                    { "id": 200, "logprob": -1.0 },
                    { "id": 201, "logprob": -1.0 },
                ]}]}),
            )
        }
        other => (
            404,
            json!({ "error": { "message": format!("no route {other}") } }),
        ),
    })
    .await;

    let runtime = ModelRuntime::new(
        catalog(format!(
            r#"{{"providers": {{"llama.cpp": {{"base_url": "{}/v1",
                "models": [{{"id": "qwen3", "api": "llama-cpp-classify"}}]}}}}}}"#,
            server.url
        )),
        store("llama-classify", &[]),
    );
    assert!(
        runtime.has_credential("llama.cpp"),
        "a local router needs no credential"
    );
    let qwen = runtime
        .find_of_type(ModelType::Classifier, "llama.cpp", "qwen3")
        .unwrap();
    let context: ClassifierContext = serde_json::from_value(json!({
        "state": { "message": "Looks good" },
        "questions": {
            "approved": { "type": "bool", "instructions": "Approved?",
                          "criteria": { "true": "", "false": "" } },
            "tone": { "type": "choice", "instructions": "Tone?",
                      "criteria": [["warm", ""], ["cold", ""]] },
        },
    }))
    .unwrap();

    let result = runtime
        .classify(&qwen, &context, ClassifyOptions::default())
        .await;
    assert_eq!(
        result.stop_reason,
        Outcome::Stop,
        "{:?}",
        result.error_message
    );
    assert_eq!(
        completions.load(Ordering::SeqCst),
        2,
        "one readout per question"
    );

    let Some(ClassifierAnswer::Bool { probability }) = result.answers.get("approved") else {
        panic!("a bool answer: {:?}", result.answers);
    };
    let expected = 1.0 / (1.0 + (-2.4f64).exp());
    assert!((probability - expected).abs() < 1e-12);

    let Some(ClassifierAnswer::Choice {
        choice, confidence, ..
    }) = result.answers.get("tone")
    else {
        panic!("a choice answer");
    };
    assert_eq!(choice, "warm", "a tie goes to the first option");
    assert_eq!(*confidence, 0.0);
}

/// The router's loaded models are chat models and classifiers, and loading one waits for the
/// router to say it is serving.
#[tokio::test]
async fn the_llama_cpp_router_lists_and_loads_models() {
    let lists = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&lists);
    let server = JsonServer::start(
        move |seen| match (seen.method.as_str(), seen.path.as_str()) {
            ("GET", "/models") => {
                let polls = counted.fetch_add(1, Ordering::SeqCst);
                let gemma = match polls {
                    0 => json!({ "value": "loading" }),
                    _ => json!({ "value": "loaded", "args": ["-c", "32768"] }),
                };
                (
                    200,
                    json!({ "data": [
                        { "id": "gemma", "status": gemma, "meta": { "n_ctx_train": 131072 } },
                        { "id": "llama", "status": { "value": "unloaded" }, "source": "dir" },
                    ]}),
                )
            }
            ("POST", "/models/load") => (200, json!({ "success": true })),
            ("GET", path) if path.starts_with("/props") => {
                (200, json!({ "chat_template": "plain" }))
            }
            (method, path) => (
                404,
                json!({ "error": { "message": format!("{method} {path}") } }),
            ),
        },
    )
    .await;

    let client = LlamaClient::new(&format!("{}/v1/", server.url), None).unwrap();
    assert_eq!(client.server_url(), server.url);

    let mut reported = Vec::new();
    let loaded = client
        .load_and_wait("gemma", |progress| reported.push(progress.message))
        .await
        .unwrap();
    assert!(loaded.is_loaded());
    assert_eq!(reported, vec!["Loading model"]);
    assert_eq!(server.seen()[0].body, json!({ "model": "gemma" }));

    let models = client.catalog_models(&Default::default()).await.unwrap();
    let ids: Vec<(String, ModelType)> = models
        .iter()
        .map(|model| (model.id.clone(), model.kind()))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("gemma".to_string(), ModelType::Chat),
            ("gemma".to_string(), ModelType::Classifier),
        ],
        "an unloaded model outside a preset is not offered"
    );
    assert_eq!(models[0].context_window, 32768);
    assert_eq!(models[0].base_url, format!("{}/v1", server.url));
}

/// Only models whose provider has a credential are available.
#[tokio::test]
async fn availability_follows_the_credentials() {
    let runtime = ModelRuntime::new(
        Catalog::bundled(),
        store("availability", &[("opencode", "oc-key")]),
    );
    let available: Vec<String> = runtime
        .available_of_type(ModelType::Classifier)
        .into_iter()
        .map(|model| model.qualified_id())
        .collect();
    assert!(
        available.contains(&"opencode/jev-1.13".to_string()),
        "{available:?}"
    );
    assert!(
        runtime
            .models_of_type(ModelType::Classifier)
            .iter()
            .any(|model| model.qualified_id() == "typesafe/jev-latest"),
        "TypeSafe's own Jev is in the catalog"
    );
    assert!(runtime
        .models_of_type(ModelType::Image)
        .iter()
        .all(|model| model.provider == "openrouter"));
}
