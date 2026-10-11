use crate::CommandContext;
use crate::CommandOutcome;
use crate::ForkPoint;
use crate::Picker;
use crate::PickerItem;
use crate::SessionChoices;
use micro_session::SessionMeta;
use micro_types::Message;

pub(crate) async fn sessions(context: &CommandContext<'_>) -> CommandOutcome {
    resume(None, context).await
}

/// `/resume` with no argument offers every recorded session; with an id, or the start of one, it
/// continues that session. `--delete <id>` and `--rename <id> <title>` change a session from the
/// selector, which is offered again afterwards.
pub(crate) async fn resume(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let argument = argument
        .map(str::trim)
        .filter(|argument| !argument.is_empty());
    if let Some(rest) = argument.and_then(|argument| argument.strip_prefix("--delete")) {
        return delete(rest.trim(), context).await;
    }
    if let Some(rest) = argument.and_then(|argument| argument.strip_prefix("--rename")) {
        return rename(rest.trim(), context).await;
    }

    let sessions = match context.sessions.list().await {
        Ok(sessions) => sessions,
        Err(error) => {
            return CommandOutcome::error(format!("could not read the session store: {error}"))
        }
    };

    let Some(query) = argument else {
        if sessions.is_empty() {
            return CommandOutcome::info("no sessions recorded yet");
        }
        return choices(sessions, context);
    };

    let matches: Vec<SessionMeta> = sessions
        .into_iter()
        .filter(|meta| meta.id == query || meta.id.starts_with(query))
        .collect();

    match matches.as_slice() {
        [] => CommandOutcome::error(format!("no session starts with \"{query}\"")),
        [only] => CommandOutcome::Resume {
            session_id: only.id.clone(),
        },
        _ => choices(matches, context),
    }
}

fn choices(sessions: Vec<SessionMeta>, context: &CommandContext<'_>) -> CommandOutcome {
    CommandOutcome::ChooseSession(SessionChoices {
        sessions,
        // Sessions record the workspace as its resolved path, so this one is compared the same way.
        workspace: std::fs::canonicalize(context.workspace)
            .unwrap_or_else(|_| context.workspace.to_path_buf()),
        current: context.session_id.map(str::to_string),
    })
}

/// The selector again, as the store now has it.
async fn choices_now(context: &CommandContext<'_>) -> CommandOutcome {
    match context.sessions.list().await {
        Ok(sessions) => choices(sessions, context),
        Err(error) => CommandOutcome::error(format!("could not read the session store: {error}")),
    }
}

async fn delete(id: &str, context: &CommandContext<'_>) -> CommandOutcome {
    if id.is_empty() {
        return CommandOutcome::error("name the session to delete");
    }
    if context.session_id == Some(id) {
        return CommandOutcome::error("Cannot delete the currently active session");
    }
    match context.sessions.delete(id).await {
        Ok(()) => choices_now(context).await,
        Err(error) => CommandOutcome::error(format!("Failed to delete: {error}")),
    }
}

async fn rename(rest: &str, context: &CommandContext<'_>) -> CommandOutcome {
    let (id, title) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let title = title.trim();
    if id.is_empty() || title.is_empty() {
        return CommandOutcome::error("name the session and the title to give it");
    }
    // The session being written to keeps its title in memory, so it is renamed through it.
    if context.session_id == Some(id) {
        return CommandOutcome::Rename {
            title: title.to_string(),
        };
    }
    match context.sessions.rename(id, title).await {
        Ok(()) => choices_now(context).await,
        Err(error) => CommandOutcome::error(format!("Failed to rename: {error}")),
    }
}

/// `/fork` with no argument offers the conversation's prompts to branch from; with an entry id it
/// starts a session holding everything before that prompt, with the prompt back in the editor.
/// `/fork <entry> --at` starts one holding everything up to and including any entry.
pub(crate) async fn fork(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("nothing to fork: this conversation is not being recorded");
    };

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(micro_session::SessionError::NotFound(_)) => {
            return CommandOutcome::info("No messages to fork from")
        }
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };
    let tree = loaded.session.tree();

    let argument = argument
        .map(str::trim)
        .filter(|argument| !argument.is_empty());
    let (argument, at) = match argument.and_then(|argument| argument.strip_suffix("--at")) {
        Some(entry_id) => (Some(entry_id.trim()), true),
        None => (argument, false),
    };
    let Some(entry_id) = argument else {
        let points: Vec<ForkPoint> = tree
            .entries()
            .iter()
            .filter_map(|entry| {
                prompt_text(&entry.message).map(|text| ForkPoint {
                    entry_id: entry.id.clone(),
                    text,
                })
            })
            .collect();
        if points.is_empty() {
            return CommandOutcome::info("No messages to fork from");
        }
        return CommandOutcome::ChooseForkPoint { points };
    };

    let Some(entry) = tree.entry(entry_id) else {
        return CommandOutcome::error(format!("no message {entry_id} in this session"));
    };
    if at {
        return CommandOutcome::Fork {
            session_id: session_id.to_string(),
            entry_id: entry.id.clone(),
            leaf: Some(entry.id.clone()),
            draft: None,
            whole: false,
        };
    }
    let Some(text) = prompt_text(&entry.message) else {
        return CommandOutcome::error(format!(
            "message {entry_id} is not a prompt; fork from something you wrote"
        ));
    };

    CommandOutcome::Fork {
        session_id: session_id.to_string(),
        entry_id: entry.id.clone(),
        leaf: entry.parent_id.clone(),
        draft: Some(text),
        whole: false,
    }
}

/// What a prompt said, when the message is a prompt with something to say.
fn prompt_text(message: &Message) -> Option<String> {
    let Message::User { content, .. } = message else {
        return None;
    };
    if micro_context::is_summary(message) {
        return None;
    }
    let text: String = content
        .iter()
        .filter_map(|block| match block {
            micro_types::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// `/tree` with no argument offers the conversation's entries to choose from; with an entry id it
/// continues from that entry.
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
        Err(micro_session::SessionError::NotFound(_)) => {
            return CommandOutcome::info("No entries in session")
        }
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };

    let outline = loaded.session.tree().outline();
    if outline.is_empty() {
        return CommandOutcome::info("No entries in session");
    }

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

        TreeFilter::Default | TreeFilter::NoTools => !is_tool,
        TreeFilter::UserOnly => is_user,
        TreeFilter::LabeledOnly => row.label.is_some(),
    }
}

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

/// `/name` gives the session a title of its own, in place of the one taken from the first thing
/// that was asked.
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

    let title = requested
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    CommandOutcome::Rename { title }
}

/// `/session` reports what is known about the conversation in hand: where it is written, what it
/// holds, and what it has cost.
pub(crate) async fn info(context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(micro_session::SessionError::NotFound(_)) => {
            return CommandOutcome::info(format!(
                "Session Info\n\nID: {session_id}\nNothing said yet: the session is written with \
                 the first message."
            ))
        }
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

    if let Some(warming) = &context.cache_warming {
        out.push_str("\nCache Warming\n");
        out.push_str(&format!("Mode: {}\n", warming.mode));
        out.push_str(&format!("Status: {}\n", warming.status));
        if let Some((miss_cost, warm_cost)) = warming.costs {
            out.push_str(&format!("Cache miss penalty: ${miss_cost:.3}\n"));
            out.push_str(&format!("Refresh cost: ${warm_cost:.3}\n"));
        }
    }

    // What the ledger billed, model by model: a virtual model's turns are billed to the physical
    // models it routed them to, and extensions' image and classifier calls to theirs.
    let billed = crate::bill::bill(context.sessions, context.catalog, session_id)
        .await
        .ok()
        .filter(|bill| {
            bill.from_ledger || !bill.model_calls.is_empty() || !bill.tool_costs.is_empty()
        });
    if let Some(bill) = billed {
        let spent = bill.by_model();
        if bill.total > 0.0 {
            out.push_str("\nCost\n");
            out.push_str(&format!("Total: ${:.3}\n", bill.total));
            for (model, amount) in spent.iter().filter(|(_, amount)| *amount > 0.0) {
                out.push_str(&format!("  {model}: ${amount:.3}\n"));
            }
        }
    } else if let Some(model) = context.model {
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
pub(crate) fn thousands(count: u64) -> String {
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

/// `/clone` copies the conversation as it stands into a session of its own, leaving this one
/// untouched.
pub(crate) async fn clone(context: &CommandContext<'_>) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("this conversation is not being recorded");
    };

    let loaded = match context.sessions.load(session_id).await {
        Ok(loaded) => loaded,
        Err(micro_session::SessionError::NotFound(_)) => {
            return CommandOutcome::info("Nothing to clone yet")
        }
        Err(error) => return CommandOutcome::error(format!("cannot read the session: {error}")),
    };
    let Some(head) = loaded.session.tree().head() else {
        return CommandOutcome::info("Nothing to clone yet");
    };

    CommandOutcome::Fork {
        session_id: session_id.to_string(),
        entry_id: head.to_string(),
        leaf: Some(head.to_string()),
        draft: None,
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
    async fn session_info_says_what_the_cache_warmer_is_doing() {
        let harness = Harness::new("session-warming");
        let id = record(&harness, 1).await;
        let context = CommandContext {
            session_id: Some(&id),
            cache_warming: Some(crate::CacheWarmingReport {
                mode: "idle".to_string(),
                status: "Decision in 4m 30s (100% continuation probability)".to_string(),
                costs: Some((0.345, 0.0301)),
            }),
            ..harness.context()
        };

        let outcome = dispatch("/session", &context).await.unwrap();
        let said = text(&outcome);
        assert!(
            said.contains(
                "Cache Warming\nMode: idle\nStatus: Decision in 4m 30s (100% continuation \
                 probability)\nCache miss penalty: $0.345\nRefresh cost: $0.030"
            ),
            "{said}"
        );
    }

    #[tokio::test]
    async fn an_empty_store_says_so_rather_than_offering_nothing() {
        let harness = Harness::new("sessions-empty");
        let outcome = dispatch("/sessions", &harness.context()).await.unwrap();

        assert!(!outcome.is_error());
        assert!(text(&outcome).contains("no sessions"));
    }

    fn chosen(outcome: &CommandOutcome) -> &SessionChoices {
        match outcome {
            CommandOutcome::ChooseSession(choices) => choices,
            other => panic!("expected the session selector, got {other:?}"),
        }
    }

    /// The selector is handed every workspace's sessions, and narrows them to this one itself.
    #[tokio::test]
    async fn resume_offers_every_recorded_session() {
        let harness = Harness::new("sessions-list");
        let first = record(&harness, 2).await;
        let elsewhere = harness.workspace.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let mut other = harness
            .sessions
            .create(&elsewhere, "anthropic/claude-opus-5")
            .await
            .unwrap();
        other.append(&Message::user("over there")).await.unwrap();

        let context = CommandContext {
            session_id: Some(&first),
            ..harness.context()
        };
        let outcome = dispatch("/resume", &context).await.unwrap();
        let choices = chosen(&outcome);

        assert_eq!(choices.sessions.len(), 2);
        assert_eq!(
            choices.workspace,
            std::fs::canonicalize(&harness.workspace).unwrap()
        );
        assert_eq!(choices.current.as_deref(), Some(first.as_str()));
        assert_eq!(
            chosen(&dispatch("/sessions", &context).await.unwrap())
                .sessions
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn a_session_is_deleted_from_the_selector_and_offered_again() {
        let harness = Harness::new("sessions-delete");
        let kept = record(&harness, 1).await;
        let dropped = record(&harness, 1).await;

        let outcome = dispatch(&format!("/resume --delete {dropped}"), &harness.context())
            .await
            .unwrap();
        let ids: Vec<&str> = chosen(&outcome)
            .sessions
            .iter()
            .map(|meta| meta.id.as_str())
            .collect();
        assert_eq!(ids, [kept.as_str()]);
    }

    #[tokio::test]
    async fn the_session_being_written_to_cannot_be_deleted() {
        let harness = Harness::new("sessions-delete-current");
        let current = record(&harness, 1).await;
        let context = CommandContext {
            session_id: Some(&current),
            ..harness.context()
        };

        let outcome = dispatch(&format!("/resume --delete {current}"), &context)
            .await
            .unwrap();
        assert!(outcome.is_error());
        assert!(harness.sessions.exists(&current).await.unwrap());
    }

    #[tokio::test]
    async fn a_session_is_renamed_from_the_selector() {
        let harness = Harness::new("sessions-rename");
        let other = record(&harness, 1).await;
        let current = record(&harness, 1).await;
        let context = CommandContext {
            session_id: Some(&current),
            ..harness.context()
        };

        let outcome = dispatch(&format!("/resume --rename {other} the plan"), &context)
            .await
            .unwrap();
        let renamed = chosen(&outcome)
            .sessions
            .iter()
            .find(|meta| meta.id == other)
            .unwrap();
        assert_eq!(renamed.title, "the plan");
        assert!(renamed.named);

        let outcome = dispatch(&format!("/resume --rename {current} mine"), &context)
            .await
            .unwrap();
        assert!(
            matches!(&outcome, CommandOutcome::Rename { title } if title == "mine"),
            "{outcome:?}"
        );
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
    async fn forking_needs_a_recorded_conversation() {
        let harness = Harness::new("fork-unrecorded");
        let outcome = dispatch("/fork", &harness.context()).await.unwrap();

        assert!(outcome.is_error());
        assert!(text(&outcome).contains("not being recorded"));
    }

    /// Record a conversation of prompts and answers, and return its id.
    async fn conversation(harness: &Harness) -> String {
        let mut session = harness
            .sessions
            .create(&harness.workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        session
            .append_all(&[
                Message::user("first prompt"),
                Message::Assistant(micro_types::AssistantMessage {
                    content: vec![micro_types::ContentBlock::text("an answer")],
                    provider: "anthropic".into(),
                    model: "claude-opus-5".into(),
                    usage: micro_types::Usage::default(),
                    charged: None,
                    stop_reason: micro_types::StopReason::Stop,
                    error: None,
                    timestamp: 0,
                }),
                Message::user("second prompt"),
            ])
            .await
            .unwrap();
        session.id().to_string()
    }

    #[tokio::test]
    async fn a_bare_fork_offers_the_prompts_to_branch_from() {
        let harness = Harness::new("fork-points");
        let id = conversation(&harness).await;
        let context = CommandContext {
            session_id: Some(&id),
            ..harness.context()
        };

        let outcome = dispatch("/fork", &context).await.unwrap();
        let CommandOutcome::ChooseForkPoint { points } = outcome else {
            panic!("expected the prompts to choose from, got {outcome:?}");
        };
        let offered: Vec<(&str, &str)> = points
            .iter()
            .map(|point| (point.entry_id.as_str(), point.text.as_str()))
            .collect();
        assert_eq!(offered, [("1", "first prompt"), ("3", "second prompt")]);
    }

    #[tokio::test]
    async fn forking_from_a_prompt_keeps_what_came_before_it_and_hands_it_back() {
        let harness = Harness::new("fork-prompt");
        let id = conversation(&harness).await;
        let context = CommandContext {
            session_id: Some(&id),
            ..harness.context()
        };

        let outcome = dispatch("/fork 3", &context).await.unwrap();
        let CommandOutcome::Fork {
            session_id,
            entry_id,
            leaf,
            draft,
            whole,
        } = outcome
        else {
            panic!("expected a fork, got {outcome:?}");
        };
        assert_eq!(session_id, id);
        assert_eq!(entry_id, "3");
        assert_eq!(leaf.as_deref(), Some("2"));
        assert_eq!(draft.as_deref(), Some("second prompt"));
        assert!(!whole);

        let outcome = dispatch("/fork 1", &context).await.unwrap();
        assert!(
            matches!(&outcome, CommandOutcome::Fork { leaf: None, .. }),
            "the first prompt forks into an empty session: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn only_a_prompt_can_be_forked_from() {
        let harness = Harness::new("fork-answer");
        let id = conversation(&harness).await;
        let context = CommandContext {
            session_id: Some(&id),
            ..harness.context()
        };

        let outcome = dispatch("/fork 2", &context).await.unwrap();
        assert!(outcome.is_error());
        assert!(text(&outcome).contains("not a prompt"), "{outcome:?}");

        let outcome = dispatch("/fork 2 --at", &context).await.unwrap();
        assert!(
            matches!(&outcome, CommandOutcome::Fork { leaf: Some(leaf), draft: None, .. } if leaf == "2"),
            "any entry can be forked at: {outcome:?}"
        );

        let outcome = dispatch("/fork 9", &context).await.unwrap();
        assert!(outcome.is_error());
    }

    #[tokio::test]
    async fn clone_copies_the_whole_conversation() {
        let harness = Harness::new("fork-clone");
        let id = conversation(&harness).await;
        let context = CommandContext {
            session_id: Some(&id),
            ..harness.context()
        };

        let outcome = dispatch("/clone", &context).await.unwrap();
        assert!(
            matches!(
                &outcome,
                CommandOutcome::Fork { leaf: Some(leaf), draft: None, whole: true, .. }
                    if leaf == "3"
            ),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn forking_a_conversation_never_written_is_refused() {
        let harness = Harness::new("fork-empty");
        let context = CommandContext {
            session_id: Some("abc"),
            ..harness.context()
        };

        let outcome = dispatch("/fork", &context).await.unwrap();
        assert!(text(&outcome).contains("No messages to fork from"));
    }
}
