//! The part of a request that stands still: what the model is told before the conversation, and
//! what it is allowed to call.

use crate::content_hash;
use crate::Context;
use crate::Message;
use crate::PrefixSpan;
use crate::ToolDefinition;

/// The head of every request a conversation issues.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prefix {
    system_prompt: Option<String>,
    tools: Vec<ToolDefinition>,
    spans: Vec<PrefixSpan>,
    hash: String,
}

impl Prefix {
    /// Assemble a prefix and hash it.
    pub fn new(
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        spans: Vec<PrefixSpan>,
    ) -> Self {
        let hash = hash_of(system_prompt.as_deref(), &tools);
        Prefix {
            system_prompt,
            tools,
            spans,
            hash,
        }
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Where each stretch of the system prompt came from, in the order they were joined.
    pub fn spans(&self) -> &[PrefixSpan] {
        &self.spans
    }

    /// What identifies this prefix: the hash of the prompt and the tool definitions together.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// The same prefix telling the model something else, rehashed.
    pub fn with_system_prompt(&self, prompt: impl Into<String>, spans: Vec<PrefixSpan>) -> Self {
        Prefix::new(Some(prompt.into()), self.tools.clone(), spans)
    }

    /// The same prefix offering a different set of tools, rehashed.
    pub fn with_tools(&self, tools: Vec<ToolDefinition>) -> Self {
        Prefix::new(self.system_prompt.clone(), tools, self.spans.clone())
    }

    /// This prefix in front of a conversation, which is one request.
    pub fn ahead_of(&self, messages: Vec<Message>, cache_key: Option<String>) -> Context {
        Context {
            system_prompt: self.system_prompt.clone(),
            messages,
            tools: self.tools.clone(),
            headers: Vec::new(),
            cache_key,
        }
    }
}

/// The name a prefix is known by: the hash of the prompt followed by the tool definitions as they
/// are serialized.
fn hash_of(system_prompt: Option<&str>, tools: &[ToolDefinition]) -> String {
    let mut bytes = system_prompt.unwrap_or_default().as_bytes().to_vec();
    bytes.extend_from_slice(&serde_json::to_vec(tools).unwrap_or_default());
    content_hash(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventSource;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: "does a thing".into(),
            parameters: serde_json::json!({ "type": "object" }),
            constrained_sampling: None,
        }
    }

    fn spans() -> Vec<PrefixSpan> {
        vec![PrefixSpan {
            source: EventSource::SystemPrompt,
            bytes: 8,
            hash: content_hash(b"be brief"),
        }]
    }

    #[test]
    fn the_same_prompt_and_tools_hash_the_same_every_time() {
        let one = Prefix::new(Some("be brief".into()), vec![tool("read")], spans());
        let again = Prefix::new(Some("be brief".into()), vec![tool("read")], spans());

        assert_eq!(one.hash(), again.hash());
        assert!(!one.hash().is_empty());
    }

    /// Either half moving moves the hash: a provider is told both before it is told anything else,
    /// so both are the cacheable head.
    #[test]
    fn changing_either_half_changes_the_hash() {
        let prefix = Prefix::new(Some("be brief".into()), vec![tool("read")], spans());

        assert_ne!(
            prefix.hash(),
            prefix.with_system_prompt("be thorough", spans()).hash()
        );
        assert_ne!(
            prefix.hash(),
            prefix.with_tools(vec![tool("read"), tool("write")]).hash()
        );
    }

    /// Attribution is not part of what a provider is told, so a prompt described differently is
    /// still the same prompt to a cache.
    #[test]
    fn where_the_prompt_came_from_is_not_part_of_its_hash() {
        let prefix = Prefix::new(Some("be brief".into()), vec![tool("read")], spans());
        let attributed = Prefix::new(
            Some("be brief".into()),
            vec![tool("read")],
            vec![PrefixSpan {
                source: EventSource::ProjectInstructions,
                bytes: 8,
                hash: content_hash(b"be brief"),
            }],
        );

        assert_eq!(prefix.hash(), attributed.hash());
    }

    #[test]
    fn a_prefix_in_front_of_a_conversation_is_a_request() {
        let prefix = Prefix::new(Some("be brief".into()), vec![tool("read")], spans());

        let context = prefix.ahead_of(vec![Message::user("go")], Some("session-1".into()));

        assert_eq!(context.system_prompt.as_deref(), Some("be brief"));
        assert_eq!(context.tools, vec![tool("read")]);
        assert_eq!(context.messages, vec![Message::user("go")]);
        assert_eq!(context.cache_key.as_deref(), Some("session-1"));
    }
}
