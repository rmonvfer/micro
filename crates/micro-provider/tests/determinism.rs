//! The same request, assembled twice, is the same bytes.

use micro_models::WireApi;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::ThinkingLevel;
use micro_types::ToolDefinition;
use micro_types::Usage;

/// How many times each body is rebuilt before it is believed.
const REPEATS: usize = 100;

/// Every protocol, with a service that speaks it and a model to speak about.
fn protocols() -> Vec<(WireApi, &'static str, Model)> {
    vec![
        (
            WireApi::AnthropicMessages,
            "anthropic",
            model("claude-opus-5", "anthropic", "https://api.anthropic.com/v1"),
        ),
        (
            WireApi::OpenaiCompletions,
            "openai",
            model("gpt-5", "openai", "https://api.openai.com/v1"),
        ),
        (
            WireApi::OpenaiResponses,
            "openai-codex",
            model(
                "gpt-5-codex",
                "openai-codex",
                "https://chatgpt.com/backend-api",
            ),
        ),
        (
            WireApi::GoogleGenerativeAi,
            "google",
            model(
                "gemini-3-pro",
                "google",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
        ),
        (
            WireApi::BedrockConverseStream,
            "bedrock",
            model(
                "anthropic.claude-opus-5",
                "bedrock",
                "https://bedrock-runtime.us-east-1.amazonaws.com",
            ),
        ),
        (
            WireApi::GoogleVertex,
            "vertex",
            model(
                "gemini-3-pro",
                "vertex",
                "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1",
            ),
        ),
    ]
}

fn model(id: &str, provider: &str, base_url: &str) -> Model {
    Model {
        id: id.into(),
        provider: provider.into(),
        base_url: base_url.into(),
        max_tokens: 4_096,
        thinking: ThinkingLevel::Medium,
        reasoning: true,
        compat: Default::default(),
        headers: Default::default(),
    }
}

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: format!("the {name} tool"),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "where" },
                "recursive": { "type": "boolean" },
                "depth": { "type": "integer" },
            },
            "required": ["path"],
        }),
        constrained_sampling: None,
    }
}

/// A conversation with one of everything a request can carry.
fn conversation() -> Vec<Message> {
    vec![
        Message::User {
            content: vec![
                ContentBlock::text("look at this"),
                ContentBlock::Image {
                    data: "iVBORw0KGgoAAAANSUhEUg==".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 1_700_000_000_000,
        },
        Message::Assistant(AssistantMessage {
            content: vec![
                ContentBlock::Thinking {
                    thinking: "the file is probably under src".into(),
                    signature: Some("signature-from-the-provider".into()),
                },
                ContentBlock::text("reading it"),
                ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({
                        "path": "src/main.rs",
                        "recursive": false,
                        "depth": 2,
                    }),
                    signature: None,
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage {
                input: 100,
                output: 20,
                cache_read: 0,
                cache_write: 0,
            },
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 1_700_000_001_000,
        }),
        Message::tool_result("call_1", "read", "fn main() {}", false),
        Message::user("thanks"),
    ]
}

/// Every context worth asserting the property over, named for the failure it would report.
fn contexts() -> Vec<(&'static str, Context)> {
    let bare = Context {
        system_prompt: None,
        messages: vec![Message::user("hello")],
        tools: Vec::new(),
        headers: Vec::new(),
        cache_key: None,
    };
    let with_tools = Context {
        system_prompt: Some("be brief".into()),
        messages: vec![Message::user("count the files")],
        tools: vec![tool("read"), tool("write"), tool("bash")],
        headers: Vec::new(),
        cache_key: Some("session-1".into()),
    };
    let full = Context {
        system_prompt: Some("be brief\n\nthe project says: run the tests".into()),
        messages: conversation(),
        tools: vec![tool("read"), tool("write"), tool("bash")],
        headers: vec![("x-something".into(), "value".into())],
        cache_key: Some("session-1".into()),
    };

    vec![
        ("a bare request", bare),
        ("a request carrying tools", with_tools),
        ("a request carrying everything", full),
    ]
}

/// The same context, handed to the same provider a hundred times, produces the same body.
#[test]
fn one_context_serializes_to_one_body_however_often_it_is_asked_for() {
    for (api, provider, model) in protocols() {
        let client = micro_provider::client_for(api, provider);
        for (named, context) in contexts() {
            let first = serde_json::to_vec(&client.payload(&model, &context)).unwrap();
            assert!(
                first.len() > 2,
                "{provider} built no body at all for {named}"
            );
            for repeat in 1..REPEATS {
                let again = serde_json::to_vec(&client.payload(&model, &context)).unwrap();
                assert_eq!(
                    first,
                    again,
                    "{provider} changed {named} on rebuild {repeat}: {} became {}",
                    String::from_utf8_lossy(&first),
                    String::from_utf8_lossy(&again),
                );
            }
        }
    }
}

#[test]
fn two_equal_contexts_serialize_to_equal_bodies() {
    for (api, provider, model) in protocols() {
        let client = micro_provider::client_for(api, provider);
        for ((named, one), (_, other)) in contexts().into_iter().zip(contexts()) {
            assert_eq!(
                serde_json::to_vec(&client.payload(&model, &one)).unwrap(),
                serde_json::to_vec(&client.payload(&model, &other)).unwrap(),
                "{provider} built two different bodies for {named}"
            );
        }
    }
}

/// Adding to the conversation leaves what came before it untouched.
#[test]
fn a_longer_conversation_opens_with_the_same_prefix() {
    for (api, provider, model) in protocols() {
        let client = micro_provider::client_for(api, provider);
        let mut context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![Message::user("first")],
            tools: vec![tool("read"), tool("write")],
            headers: Vec::new(),
            cache_key: Some("session-1".into()),
        };
        let first = client.payload(&model, &context);

        context.messages.push(Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("done")],
            provider: provider.into(),
            model: model.id.clone(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 1_700_000_002_000,
        }));
        context.messages.push(Message::user("second"));
        let second = client.payload(&model, &context);

        for field in [
            "system",
            "instructions",
            "tools",
            "toolConfig",
            "system_instruction",
        ] {
            assert_eq!(
                first.get(field),
                second.get(field),
                "{provider} moved `{field}` when the conversation grew"
            );
        }
    }
}
