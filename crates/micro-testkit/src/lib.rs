//! Test doubles for driving the agent loop without a network.

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
