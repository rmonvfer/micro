//! What goes into the model's context and what comes back out of it when it fills up.

mod compaction;
mod error;
mod instructions;

pub use compaction::estimate_context_tokens;
pub use compaction::estimate_message;
pub use compaction::estimate_tokens;
pub use compaction::find_cut;
pub use compaction::is_self_contained;
pub use compaction::is_summary;
pub use compaction::render_transcript;
pub use compaction::summary_message;
pub use compaction::summary_text;
pub use compaction::Compacted;
pub use compaction::CompactionConfig;
pub use compaction::Compactor;
pub use compaction::Summarizer;
pub use compaction::Summary;
pub use compaction::CHARS_PER_TOKEN;
pub use compaction::COMPACTION_PROMPT;
pub use compaction::SUMMARY_CLOSE;
pub use compaction::SUMMARY_OPEN;
pub use error::ContextError;
pub use error::Result;
pub use instructions::InstructionLoader;
pub use instructions::Instructions;
pub use instructions::DEFAULT_MAX_IMPORT_DEPTH;
pub use instructions::INSTRUCTION_FILE_NAMES;
