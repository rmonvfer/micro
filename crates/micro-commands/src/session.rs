//! `/sessions`, `/resume`, `/fork`, `/clone`, `/tree`, `/name` and `/session`: moving
//! between conversations, and reporting on the one in hand.

use crate::CommandContext;
use crate::CommandOutcome;
use crate::Picker;
use crate::PickerItem;
use micro_session::SessionMeta;

pub(crate) async fn sessions(context: &CommandContext<'_>) -> CommandOutcome {
    match listing(context).await {
        Ok(sessions) if sessions.is_empty() => {
            CommandOutcome::info("no sessions recorded in this workspace yet")
        }
        Ok(sessions) => CommandOutcome::Choose(session_picker(sessions, context)),
        Err(message) => CommandOutcome::error(message),
    }
}

pub(crate) async fn resume(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let sessions = match listing(context).await {
        Ok(sessions) => sessions,
        Err(message) => return CommandOutcome::error(message),
    };

    let Some(query) = argument else {
        if sessions.is_empty() {
            return CommandOutcome::info("no sessions recorded in this workspace yet");
        }
        return CommandOutcome::Choose(session_picker(sessions, context));
    };

    // Ids are long enough that nobody types one in full, so a unique prefix stands in.
    let matches: Vec<SessionMeta> = sessions
        .into_iter()
        .filter(|meta| meta.id == query || meta.id.starts_with(query))
        .collect();

    match matches.as_slice() {
        [] => CommandOutcome::error(format!(
            "no session in this workspace starts with \"{query}\""
        )),
        [only] => CommandOutcome::Resume {
            session_id: only.id.clone(),
        },
        several => {
            let title = format!("{} sessions start with \"{query}\"", several.len());
            CommandOutcome::Choose(
                Picker::new(title, items(several.to_vec(), context)).searchable(),
            )
        }
    }
}

pub(crate) fn fork(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("nothing to fork: this conversation is not being recorded");
    };

    if context.message_count == 0 {
        return CommandOutcome::error("nothing to fork: the conversation is empty");
    }

    // Forking through the last message copies the conversation whole, which is what a
    // bare `/fork` means.
    let last = context.message_count - 1;
    let through_index = match argument {
        None => last,
        Some(raw) => match raw.parse::<usize>() {
            Ok(index) if index <= last => index,
            Ok(index) => {
                return CommandOutcome::error(format!(
                    "message {index} is past the end of the conversation - pick 0 to {last}"
                ))
            }
            Err(_) => {
                return CommandOutcome::error(format!(
                    "\"{raw}\" is not a message index - pick 0 to {last}"
                ))
            }
        },
    };

    CommandOutcome::Fork {
        session_id: session_id.to_string(),
        through_index,
        whole: false,
    }
}

async fn listing(context: &CommandContext<'_>) -> Result<Vec<SessionMeta>, String> {
    context
        .sessions
        .list_in(context.workspace)
        .await
        .map_err(|error| format!("could not read the session store: {error}"))
}

fn session_picker(sessions: Vec<SessionMeta>, context: &CommandContext<'_>) -> Picker {
    Picker::new("Resume a session", items(sessions, context)).searchable()
}

fn items(sessions: Vec<SessionMeta>, context: &CommandContext<'_>) -> Vec<PickerItem> {
    sessions
        .into_iter()
        .map(|meta| {
            let current = context.session_id == Some(meta.id.as_str());
            PickerItem::new(
                title(&meta),
                format!("{} · {} · {}", meta.id, messages(&meta), meta.model_id),
                format!("/resume {}", meta.id),
            )
            .current(current)
        })
        .collect()
}

/// A session's title, or its id when it was never given one.
fn title(meta: &SessionMeta) -> String {
    if meta.title.trim().is_empty() {
        meta.id.clone()
    } else {
        meta.title.clone()
    }
}

fn messages(meta: &SessionMeta) -> String {
    match meta.message_count {
        1 => "1 message".to_string(),
        count => format!("{count} messages"),
    }
}

/// `/tree` with no argument offers the conversation's entries to choose from; with an
/// entry id it continues from that entry.
///
/// Nothing is thrown away by continuing from an earlier point. What came after stays in
/// the log as another branch, which is what makes it safe to go back and try again.
pub(crate) async fn tree(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    if let Some(entry_id) = argument {
        return CommandOutcome::Branch {
            entry_id: entry_id.trim().to_string(),
        };
    }

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };

    let outline = loaded.session.tree().outline();
    if outline.is_empty() {
        return CommandOutcome::info("No entries in session");
    }

    // The tree opens showing what the reader asked it to: the whole of it, the shape of
    // the conversation without the tool calls that fill it out, or only what they wrote.
    let shown: Vec<&micro_session::Row<'_>> = outline
        .iter()
        .filter(|row| keeps(context.tree_filter, row))
        .collect();
    if shown.is_empty() {
        return CommandOutcome::info(
            "Nothing matches the tree filter. `/set tree_filter_mode all` shows everything.",
        );
    }

    let items = shown
        .iter()
        .map(|row| {
            // Depth is drawn into the label, so a branch reads as one at a glance and the
            // list stays a flat thing to move through.
            let label = format!("{}{}", "  ".repeat(row.depth), summary(&row.entry.message));
            PickerItem::new(label, where_it_sits(row), format!("/tree {}", row.entry.id))
                .current(row.is_head)
        })
        .collect();

    CommandOutcome::Choose(Picker::new("Session Tree", items).searchable())
}

/// Whether an entry is one the reader asked to see.
fn keeps(filter: micro_config::TreeFilter, row: &micro_session::Row<'_>) -> bool {
    use micro_config::TreeFilter;
    let is_tool = matches!(row.entry.message, micro_types::Message::ToolResult { .. });
    let is_user = matches!(row.entry.message, micro_types::Message::User { .. });

    match filter {
        TreeFilter::All => true,
        // What the conversation is made of, which is everything but the tools filling it out.
        TreeFilter::Default | TreeFilter::NoTools => !is_tool,
        TreeFilter::UserOnly => is_user,
        TreeFilter::LabeledOnly => row.label.is_some(),
    }
}

/// What an entry is to the conversation as it stands: where it continues from, what it is
/// on the way to, or a branch that was left behind.
fn where_it_sits(row: &micro_session::Row<'_>) -> &'static str {
    match (row.is_head, row.on_path) {
        (true, _) => "current",
        (_, true) => "on this branch",
        _ => "another branch",
    }
}

/// One line describing a message, for the tree view.
fn summary(message: &micro_types::Message) -> String {
    let (who, text) = match message {
        micro_types::Message::User { content, .. } => ("user", text_of(content)),
        micro_types::Message::Assistant(assistant) => ("assistant", assistant.text()),
        micro_types::Message::ToolResult { tool_name, .. } => ("tool", tool_name.clone()),
    };
    let text = text.replace('\n', " ");
    let text: String = text.chars().take(56).collect();
    format!("{who}: {text}")
}

fn text_of(content: &[micro_types::ContentBlock]) -> String {
    content
        .iter()
        .map(micro_types::ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// `/name` gives the session a title of its own, in place of the one taken from the first
/// thing that was asked. With no argument it reports the title the session already has.
pub(crate) async fn name(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    let requested = argument.map(str::trim).filter(|title| !title.is_empty());
    let Some(requested) = requested else {
        let current = context
            .sessions
            .meta(session_id)
            .await
            .map(|meta| meta.title)
            .unwrap_or_default();
        return match current.is_empty() {
            true => CommandOutcome::error("Usage: /name <name>"),
            false => CommandOutcome::info(format!("Session name: {current}")),
        };
    };

    // A title is one line, because that is how it is listed.
    let title = requested
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    CommandOutcome::Rename { title }
}

/// `/session` reports what is known about the conversation in hand: where it is written,
/// what it holds, and what it has cost.
pub(crate) async fn info(context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };

    let meta = loaded.session.meta();
    let mut out = String::from("Session Info\n\n");
    if !meta.title.is_empty() {
        out.push_str(&format!("Name: {}\n", meta.title));
    }
    out.push_str(&format!("File: {}\n", loaded.session.path().display()));
    out.push_str(&format!("ID: {}\n", meta.id));
    out.push_str(&format!("Model: {}\n", meta.model_id));

    let counts = Counts::of(&loaded.messages);
    out.push_str("\nMessages\n");
    out.push_str(&format!(
        "Total: {}\n",
        thousands(loaded.messages.len() as u64)
    ));
    out.push_str(&format!("User: {}\n", thousands(counts.user)));
    out.push_str(&format!("Assistant: {}\n", thousands(counts.assistant)));
    out.push_str(&format!(
        "Tools: {} calls, {} results\n",
        thousands(counts.tool_calls),
        thousands(counts.tool_results)
    ));

    let usage = context.usage;
    out.push_str("\nTokens\n");
    out.push_str(&format!("Input: {}\n", thousands(usage.input as u64)));
    let read_from_cache = usage.cache_read as u64;
    let billed_input = usage.input as u64 + read_from_cache;
    if read_from_cache > 0 {
        let share = (read_from_cache as f64 / billed_input.max(1) as f64) * 100.0;
        out.push_str(&format!(
            "  Cached: {} ({share:.1}%)\n",
            thousands(read_from_cache)
        ));
        out.push_str(&format!(
            "  Uncached: {} ({} written to cache)\n",
            thousands(usage.input as u64),
            thousands(usage.cache_write as u64)
        ));
    }
    out.push_str(&format!("Output: {}\n", thousands(usage.output as u64)));
    out.push_str(&format!(
        "Total: {}\n",
        thousands(usage.total_tokens() as u64)
    ));

    // Priced against the model in use, since that is what the tokens were spent on. A
    // subscription-backed provider reports no prices, and so no cost section.
    if let Some(model) = context.model {
        let spent = model
            .price(
                micro_models::TokenUsage::new(usage.input as u64, usage.output as u64)
                    .with_cache(usage.cache_read as u64, usage.cache_write as u64),
            )
            .total();
        if spent > 0.0 {
            out.push_str("\nCost\n");
            out.push_str(&format!("Total: ${spent:.3}\n"));
            out.push_str(&format!("  {}: ${spent:.3}\n", model.qualified_id()));
        }
    }

    CommandOutcome::info(out.trim_end().to_string())
}

/// How many messages of each kind a conversation holds.
struct Counts {
    user: u64,
    assistant: u64,
    tool_calls: u64,
    tool_results: u64,
}

impl Counts {
    fn of(messages: &[micro_types::Message]) -> Self {
        let mut counts = Counts {
            user: 0,
            assistant: 0,
            tool_calls: 0,
            tool_results: 0,
        };
        for message in messages {
            match message {
                micro_types::Message::User { .. } => counts.user += 1,
                micro_types::Message::Assistant(assistant) => {
                    counts.assistant += 1;
                    counts.tool_calls += assistant
                        .content
                        .iter()
                        .filter(|block| matches!(block, micro_types::ContentBlock::ToolCall { .. }))
                        .count() as u64;
                }
                micro_types::Message::ToolResult { .. } => counts.tool_results += 1,
            }
        }
        counts
    }
}

/// `1234567` as `1,234,567`, which is how a count this long is read.
fn thousands(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `/clone` copies the conversation as it stands into a session of its own, leaving this
/// one untouched. What was branched away from is not copied: a clone is the conversation
/// in hand, not every conversation the log holds.
pub(crate) async fn clone(context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };
    if loaded.messages.is_empty() {
        return CommandOutcome::info("Nothing to clone yet");
    }

    CommandOutcome::Fork {
        session_id: session_id.to_string(),
        through_index: loaded.messages.len() - 1,
        whole: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch;
    use crate::testing::*;
    use crate::CommandContext;
    use micro_types::Message;

    /// Record a session in the harness's workspace and return its id.
    async fn record(harness: &Harness, messages: usize) -> String {
        let mut session = harness
            .sessions
            .create(&harness.workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        for index in 0..messages {
            session
                .append(&Message::user(format!("message {index}")))
                .await
                .unwrap();
        }
        session.id().to_string()
    }

    #[tokio::test]
    async fn an_empty_store_says_so_rather_than_offering_nothing() {
        let harness = Harness::new("sessions-empty");
        let outcome = dispatch("/sessions", &harness.context()).await.unwrap();

        assert!(!outcome.is_error());
        assert!(text(&outcome).contains("no sessions"));
    }

    #[tokio::test]
    async fn sessions_offers_the_ones_from_this_workspace() {
        let harness = Harness::new("sessions-list");
        let first = record(&harness, 2).await;
        let second = record(&harness, 1).await;

        let outcome = dispatch("/sessions", &harness.context()).await.unwrap();
        let picker = picker(&outcome);

        assert_eq!(picker.items.len(), 2);
        let commands: Vec<&str> = picker
            .items
            .iter()
            .map(|item| item.command.as_str())
            .collect();
        assert!(commands.contains(&format!("/resume {first}").as_str()));
        assert!(commands.contains(&format!("/resume {second}").as_str()));
        assert!(picker
            .items
            .iter()
            .any(|item| item.detail.contains("1 message")));
        assert!(picker
            .items
            .iter()
            .any(|item| item.detail.contains("2 messages")));
    }

    #[tokio::test]
    async fn a_session_from_another_workspace_is_not_offered() {
        let harness = Harness::new("sessions-elsewhere");
        let elsewhere = harness.workspace.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        harness
            .sessions
            .create(&elsewhere, "anthropic/claude-opus-5")
            .await
            .unwrap();

        let outcome = dispatch("/sessions", &harness.context()).await.unwrap();
        assert!(text(&outcome).contains("no sessions"));
    }

    #[tokio::test]
    async fn resume_takes_a_full_id() {
        let harness = Harness::new("resume-full");
        let id = record(&harness, 1).await;

        let outcome = dispatch(&format!("/resume {id}"), &harness.context())
            .await
            .unwrap();
        let CommandOutcome::Resume { session_id } = outcome else {
            panic!("expected a resume");
        };
        assert_eq!(session_id, id);
    }

    #[tokio::test]
    async fn resume_takes_a_unique_prefix() {
        let harness = Harness::new("resume-prefix");
        let id = record(&harness, 1).await;
        let prefix = &id[..8];

        let outcome = dispatch(&format!("/resume {prefix}"), &harness.context())
            .await
            .unwrap();
        let CommandOutcome::Resume { session_id } = outcome else {
            panic!("expected a resume");
        };
        assert_eq!(session_id, id);
    }

    #[tokio::test]
    async fn an_id_that_matches_nothing_is_reported() {
        let harness = Harness::new("resume-unknown");
        record(&harness, 1).await;

        let outcome = dispatch("/resume zzzzzz", &harness.context())
            .await
            .unwrap();
        assert!(outcome.is_error());
        assert!(text(&outcome).contains("zzzzzz"));
    }

    #[tokio::test]
    async fn resume_with_no_argument_offers_the_same_list_as_sessions() {
        let harness = Harness::new("resume-picker");
        record(&harness, 1).await;

        let outcome = dispatch("/resume", &harness.context()).await.unwrap();
        assert_eq!(picker(&outcome).items.len(), 1);
    }

    #[tokio::test]
    async fn forking_needs_a_recorded_conversation() {
        let harness = Harness::new("fork-unrecorded");
        let outcome = dispatch("/fork", &harness.context()).await.unwrap();

        assert!(outcome.is_error());
        assert!(text(&outcome).contains("not being recorded"));
    }

    #[tokio::test]
    async fn a_bare_fork_branches_at_the_last_message() {
        let harness = Harness::new("fork-last");
        let context = CommandContext {
            session_id: Some("abc"),
            message_count: 5,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
            ..harness.context()
        };

        let outcome = dispatch("/fork", &context).await.unwrap();
        let CommandOutcome::Fork {
            session_id,
            through_index,
            ..
        } = outcome
        else {
            panic!("expected a fork");
        };
        assert_eq!(session_id, "abc");
        assert_eq!(through_index, 4);
    }

    #[tokio::test]
    async fn forking_at_an_index_keeps_everything_through_it() {
        let harness = Harness::new("fork-index");
        let context = CommandContext {
            session_id: Some("abc"),
            message_count: 5,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
            ..harness.context()
        };

        let outcome = dispatch("/fork 2", &context).await.unwrap();
        assert!(
            matches!(
                outcome,
                CommandOutcome::Fork {
                    through_index: 2,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn an_index_past_the_end_names_the_range() {
        let harness = Harness::new("fork-range");
        let context = CommandContext {
            session_id: Some("abc"),
            message_count: 3,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
            ..harness.context()
        };

        let outcome = dispatch("/fork 9", &context).await.unwrap();
        assert!(outcome.is_error());
        assert!(text(&outcome).contains("0 to 2"), "{outcome:?}");

        let outcome = dispatch("/fork later", &context).await.unwrap();
        assert!(outcome.is_error());
        assert!(
            text(&outcome).contains("not a message index"),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn forking_an_empty_conversation_is_refused() {
        let harness = Harness::new("fork-empty");
        let context = CommandContext {
            session_id: Some("abc"),
            ..harness.context()
        };

        let outcome = dispatch("/fork", &context).await.unwrap();
        assert!(outcome.is_error());
        assert!(text(&outcome).contains("empty"));
    }
}
