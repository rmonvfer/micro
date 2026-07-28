//! Test doubles for driving the agent loop without a network.
//!
//! [`FakeProvider`] replays a script of [`Turn`]s and records every request it was handed,
//! so a test can assert what the loop actually sent the model. [`FakeTool`] answers with
//! canned output and counts its calls. [`run_agent`] drives an [`micro_agent::Agent`] to
//! completion and hands back both the messages it returned and an [`EventLog`] of
//! everything it emitted.
//!
//! ```
//! use micro_testkit::{run_agent, FakeProvider, FakeTool, Turn};
//! use micro_types::{Message, Model, ThinkingLevel};
//! use std::sync::Arc;
//!
//! # tokio_test_block(async {
//! let provider = FakeProvider::builder()
//!     .turn(Turn::new().with_tool_call("c1", "read", serde_json::json!({ "path": "a.txt" })))
//!     .turn(Turn::text("a.txt says hello"))
//!     .build();
//! let read = FakeTool::new("read").returning("hello");
//!
//! let model = Model {
//!     id: "test-model".into(),
//!     provider: "fake".into(),
//!     base_url: "https://example.invalid".into(),
//!     max_tokens: 1024,
//!     thinking: ThinkingLevel::Off,
//! };
//! let mut agent = micro_agent::Agent::new(
//!     Arc::new(provider.clone()),
//!     vec![Arc::new(read.clone())],
//!     model,
//!     "test-key",
//! );
//!
//! let (messages, events) = run_agent(&mut agent, Message::user("read a.txt")).await;
//!
//! assert_eq!(read.call_count(), 1);
//! assert_eq!(provider.call_count(), 2);
//! assert_eq!(messages.len(), 4);
//! assert_eq!(events.names().last(), Some(&"AgentSettled"));
//! # });
//! # fn tokio_test_block<F: std::future::Future>(f: F) -> F::Output {
//! #     tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
//! # }
//! ```

mod harness;
mod provider;
mod summarizer;
mod tool;

pub use harness::run_agent;
pub use harness::EventLog;
pub use provider::FakeProvider;
pub use provider::FakeProviderBuilder;
pub use provider::RecordedCall;
pub use provider::Turn;
pub use summarizer::FakeSummarizer;
pub use tool::FakeTool;
