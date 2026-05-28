//! The interactive terminal interface.

mod app;
mod background;
mod capabilities;
mod clipboard;
mod commands;
mod diff;
mod event;
mod images;
mod latex;
mod layout;
mod markdown;
mod menu;
pub use menu::MenuItem;
mod picker;
pub mod remote;
mod render;
mod tools;
mod typeset;
pub mod ui;
mod wrap;


pub mod editor;
pub mod theme;
pub mod transcript;

pub use app::ResourceSection;
pub use app::Resources;
pub use app::TuiOptions;
pub use commands::Applied;
pub use commands::BashRun;
pub use commands::Commands;
pub use commands::ConversationState;
pub use commands::DoubleEscape;
pub use commands::Listings;
pub use commands::Preferences;
pub use theme::Theme;
pub use ui::host_ask_channel;
pub use ui::terminal_input_channel;
pub use ui::ui_channel;
pub use ui::HostAsk;
pub use ui::HostAsker;
pub use ui::HostAsks;
pub use ui::TerminalInputAsker;
pub use ui::TerminalInputAsks;
pub use ui::UiAsker;
pub use ui::UiRequest;
pub use ui::UiRequests;

use crate::app::App;
use crate::app::Outcome;
use anyhow::Result;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::Event;
use crossterm::event::EventStream;
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use futures::StreamExt;
use micro_agent::Agent;
use micro_commands::CommandOutcome;
use micro_commands::MessageKind;
use micro_types::AgentEvent;
use micro_types::Message;
use ratatui::backend::Backend as _;
use ratatui::backend::CrosstermBackend;
use ratatui::text::Line;
use ratatui::Terminal;
use serde_json::Value;
use std::future::Future;
use std::io::Stdout;
use std::sync::Once;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::MissedTickBehavior;

/// Shortest gap between repaints.
const FRAME: Duration = Duration::from_millis(33);
/// How often the spinner advances and a running turn is repainted.
const TICK: Duration = Duration::from_millis(80);

/// Run the interface with default options.
pub async fn run(agent: Agent, history: Vec<Message>) -> Result<Vec<Message>> {
    run_with(agent, history, TuiOptions::default()).await
}

/// Run the interface, returning the agent's conversation as it stands when the user leaves.
pub async fn run_with(
    mut agent: Agent,
    history: Vec<Message>,
    mut options: TuiOptions,
) -> Result<Vec<Message>> {
    install_panic_hook();
    let mut screen = Screen::enter(options.tui_mode)?;
    
    background::prime();
    options.theme = Some(options.theme.unwrap_or_else(background::detect_theme));
    let mode = options.tui_mode;
    let exit_output = options.settings.exit_output;
    let mut said = Vec::new();
    let result = drive(&mut screen, &mut agent, &history, options, &mut said).await;
    leave();
    
    if mode == TuiMode::Fullscreen && exit_output == commands::ExitOutput::Transcript {
        use std::io::Write as _;
        let mut out = std::io::stdout();
        for line in said {
            let _ = writeln!(out, "{line}");
        }
    }
    result?;

    Ok(agent.messages().to_vec())
}

/// Run the interface, and hand back the conversation as it was left on screen.
async fn drive(
    screen: &mut Screen,
    agent: &mut Agent,
    history: &[Message],
    mut options: TuiOptions,
    said: &mut Vec<String>,
) -> Result<()> {
    let mut questions = options.questions.take();
    let mut commands = options.commands.take();
    let terminal_input = options.terminal_input.take();
    let host_asker = options.host_asker.take();
    let mut remote = options.remote.take();
    let mut app = App::new(history, options);
    let outcome = run_loop(
        screen,
        agent,
        &mut app,
        &mut questions,
        &mut commands,
        &terminal_input,
        &host_asker,
        &mut remote,
        said,
    )
    .await;
    *said = app.plain_lines();
    outcome
}

async fn run_loop(
    screen: &mut Screen,
    agent: &mut Agent,
    app: &mut App,
    questions: &mut Option<crate::ui::UiRequests>,
    commands: &mut Option<Box<dyn Commands + 'static>>,
    terminal_input: &Option<crate::ui::TerminalInputAsker>,
    host_asker: &Option<crate::ui::HostAsker>,
    remote: &mut Option<crate::remote::Remote>,
    _said: &mut Vec<String>,
) -> Result<()> {
    let mut input = EventStream::new();
    
    let mut refreshing: Option<tokio::sync::oneshot::Receiver<Listings>> = None;
    
    
    let mut suggesting: Option<(String, tokio::sync::oneshot::Receiver<Value>)> = None;
    let mut completing: Option<tokio::sync::oneshot::Receiver<Value>> = None;

    loop {
        screen.render(app)?;
        if app.should_quit {
            return Ok(());
        }

        
        let retired = app.take_retired_tools();
        if !retired.is_empty() {
            agent.remove_tools(&retired);
        }

        
        if let Some((provider, key)) = app.take_key_prompt() {
            if let Some(commands) = commands.as_mut() {
                app.busy("signing in");
                let stored = commands.store_api_key(provider, key);
                let applied = await_host(screen, app, &mut input, stored).await?;
                app.idle();
                if let Some(applied) = applied {
                    apply_applied(app, agent, applied);
                }
            }
            continue;
        }

        
        if let Some(question) = questions
            .as_mut()
            .and_then(|questions| questions.try_recv())
        {
            app.ask_question(question);
            continue;
        }

        match app.take_submission() {
            Some(line) => {
                submit(
                    Turn {
                        screen,
                        app,
                        agent,
                        input: &mut input,
                        questions: questions,
                        terminal_input,
                        host_asker,
                        remote,
                    },
                    commands.as_deref_mut(),
                    line,
                )
                .await?
            }
            None => match next_event(
                &mut input,
                &mut refreshing,
                app,
                commands,
                host_asker,
                &mut suggesting,
                &mut completing,
            )
            .await
            {
                
                Next::Redrawn => continue,
                Next::Event(event) => {
                    if offer_component_input(host_asker, app, &event).await {
                        continue;
                    }
                    if offer_terminal_input(terminal_input, app, &event).await {
                        continue;
                    }
                    if offer_editor_component_input(host_asker, app, &event).await {
                        continue;
                    }
                    if offer_shortcut(commands, &event).await {
                        continue;
                    }
                    match handle(app, event) {
                        Outcome::Quit => return Ok(()),
                        Outcome::ExternalEditor => external_editor(screen, app)?,
                        Outcome::ThinkingChanged(level) => {
                            agent.set_thinking(level);
                            if let Some(commands) = commands.as_mut() {
                                commands.thinking_changed(level).await;
                            }
                        }
                        
                        Outcome::CycleModel(forward) => {
                            app.queue_line(match forward {
                                true => "/model next",
                                false => "/model previous",
                            });
                        }
                        Outcome::Suspend => suspend(screen, app)?,
                        _ => {}
                    }
                }
                
                Next::Ended => return Ok(()),
            },
        }
    }
}

/// Shell integration: the markers a terminal uses to tell a prompt from its output.
mod osc133 {
    /// A prompt begins here.
    pub const PROMPT: &str = "\x1b]133;A\x07";
    /// The prompt ends and what was typed begins.
    pub const INPUT: &str = "\x1b]133;B\x07";
    /// The command was accepted; its output follows.
    pub const OUTPUT: &str = "\x1b]133;C\x07";
}

/// Progress, as a terminal that shows it in the tab or the dock expects to be told.
mod osc94 {
    pub const BUSY: &str = "\x1b]9;4;3;0\x07";
    pub const DONE: &str = "\x1b]9;4;0;0\x07";
}

/// Say whether something is running, for a terminal that draws progress of its own.
fn report_progress(enabled: bool, running: bool) {
    if !enabled {
        return;
    }
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = out.write_all(
        match running {
            true => osc94::BUSY,
            false => osc94::DONE,
        }
        .as_bytes(),
    );
    let _ = out.flush();
}

/// Set the terminal's window/tab title.
fn set_terminal_title(title: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]0;{title}\x07");
    let _ = out.flush();
}

/// Tell the terminal a prompt was submitted and its answer is about to arrive.
fn mark_prompt_submitted() {
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = write!(out, "{}{}{}", osc133::PROMPT, osc133::INPUT, osc133::OUTPUT);
    let _ = out.flush();
}

/// Drop to the shell, and pick the interface back up when the user returns.
#[cfg(unix)]
fn suspend(screen: &mut Screen, app: &mut App) -> Result<()> {
    leave();
    // SAFETY: raising SIGTSTP on our own process is what ctrl+z does in any shell job.
    unsafe {
        libc::raise(libc::SIGTSTP);
    }
    screen.reopen()?;
    let _ = app;
    Ok(())
}

/// Suspending is a job-control idea, and Windows has no equivalent to raise.
#[cfg(not(unix))]
fn suspend(_screen: &mut Screen, app: &mut App) -> Result<()> {
    app.notice(
        "Suspending is not supported on this platform.",
        MessageKind::Info,
    );
    Ok(())
}

/// The next thing the loop has to answer: a key, or a refresh coming back.
enum Next {
    /// Something the user did.
    Event(Event),
    /// Something finished behind the interface.
    Redrawn,
    /// The input ended, so the session is over.
    Ended,
}

async fn next_event(
    input: &mut EventStream,
    refreshing: &mut Option<tokio::sync::oneshot::Receiver<Listings>>,
    app: &mut App,
    commands: &mut Option<Box<dyn Commands + 'static>>,
    host_asker: &Option<crate::ui::HostAsker>,
    suggesting: &mut Option<(String, tokio::sync::oneshot::Receiver<Value>)>,
    completing: &mut Option<tokio::sync::oneshot::Receiver<Value>>,
) -> Next {
    
    if refreshing.is_none() && app.picker_mut().is_some_and(|open| open.refreshes()) {
        if let Some(commands) = commands.as_deref_mut() {
            *refreshing = commands.begin_model_refresh();
        }
    }
    
    
    if suggesting.is_none() {
        if let (Some(request), Some(asker)) = (app.take_pending_suggestion_request(), host_asker) {
            *suggesting = Some(ask_for_suggestions(asker.clone(), request));
        }
    }
    
    if completing.is_none() {
        if let (Some(request), Some(asker)) = (app.take_pending_completion_request(), host_asker) {
            *completing = Some(ask_to_apply_completion(asker.clone(), request));
        }
    }

    tokio::select! {
        biased;
        event = input.next() => arrived(event),
        listings = async { refreshing.as_mut().unwrap().await }, if refreshing.is_some() => {
            *refreshing = None;
            let listings = listings.unwrap_or_default();
            let errors = listings.errors.clone();
            let rebuilt = match commands.as_deref_mut() {
                Some(commands) => commands.apply_model_refresh(listings).await,
                None => None,
            };
            if let Some(open) = app.picker_mut() {
                
                if let Some(rebuilt) = rebuilt {
                    open.replace_items(rebuilt);
                }
                match errors.len() {
                    0 => open.set_status("Model catalogs refreshed.", true),
                    1 => open.set_status(
                        format!("Could not refresh: {}; showing what is known.", errors[0]),
                        false,
                    ),
                    count => open.set_status(
                        format!("Could not refresh {count} model catalogs; showing what is known."),
                        false,
                    ),
                }
            }
            Next::Redrawn
        }
        answer = async {
            let (_, receiver) = suggesting.as_mut().unwrap();
            receiver.await
        }, if suggesting.is_some() => {
            
            let (prefix, _) = suggesting.take().expect("guarded by is_some");
            if let Ok(answer) = answer {
                let items = answer
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                app.apply_extension_suggestions(&prefix, items);
            }
            Next::Redrawn
        }
        answer = async { completing.as_mut().unwrap().await }, if completing.is_some() => {
            *completing = None;
            if let Ok(answer) = answer {
                apply_extension_completion_answer(app, &answer);
            }
            Next::Redrawn
        }
    }
}

/// Start the `getSuggestions` question `sync_menu` raised, off the render path.
fn ask_for_suggestions(
    asker: crate::ui::HostAsker,
    request: crate::app::SuggestionRequest,
) -> (String, tokio::sync::oneshot::Receiver<Value>) {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let prefix = request.prefix.clone();
    tokio::spawn(async move {
        let answer = asker
            .ask(
                "get_suggestions",
                serde_json::json!({
                    "lines": request.lines,
                    "cursorLine": request.cursor_line,
                    "cursorCol": request.cursor_col,
                }),
            )
            .await;
        let _ = sender.send(answer);
    });
    (prefix, receiver)
}

/// Start the `applyCompletion` question committing an extension's menu item raised, off the render
/// path.
fn ask_to_apply_completion(
    asker: crate::ui::HostAsker,
    request: crate::app::CompletionRequest,
) -> tokio::sync::oneshot::Receiver<Value> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let answer = asker
            .ask(
                "apply_completion",
                serde_json::json!({
                    "lines": request.lines,
                    "cursorLine": request.cursor_line,
                    "cursorCol": request.cursor_col,
                    "item": request.item,
                    "prefix": request.prefix,
                }),
            )
            .await;
        let _ = sender.send(answer);
    });
    receiver
}

/// Carry out what `applyCompletion` answered, when it is shaped the way one should be.
fn apply_extension_completion_answer(app: &mut App, answer: &Value) {
    let (Some(lines), Some(cursor_line), Some(cursor_col)) = (
        answer.get("lines").and_then(Value::as_array),
        answer.get("cursorLine").and_then(Value::as_u64),
        answer.get("cursorCol").and_then(Value::as_u64),
    ) else {
        return;
    };
    let lines = lines
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    app.apply_extension_completion(lines, cursor_line as usize, cursor_col as usize);
}

/// What the event stream handed back, as the loop reads it.
fn arrived(event: Option<std::result::Result<Event, std::io::Error>>) -> Next {
    match event {
        Some(Ok(event)) => Next::Event(event),
        Some(Err(_)) | None => Next::Ended,
    }
}

/// Hand the prompt to `$EDITOR`, and take back whatever it was left as.
fn external_editor(screen: &mut Screen, app: &mut App) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let path = std::env::temp_dir().join(format!("micro-prompt-{}.md", std::process::id()));
    if std::fs::write(&path, app.editor.text()).is_err() {
        app.notice("Could not write the prompt to a file.", MessageKind::Error);
        return Ok(());
    }

    leave();
    let status = std::process::Command::new(&editor).arg(&path).status();
    screen.reopen()?;

    match status {
        Ok(status) if status.success() => match std::fs::read_to_string(&path) {
            Ok(text) => app.editor.set_text(text.trim_end_matches('\n')),
            Err(_) => app.notice("Could not read the prompt back.", MessageKind::Error),
        },
        Ok(_) => app.notice(
            format!("{editor} exited without saving."),
            MessageKind::Info,
        ),
        Err(error) => app.notice(
            format!("Could not run {editor}: {error}"),
            MessageKind::Error,
        ),
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

/// Send one submitted line where it belongs: to the shell, to a command, or to the model.
struct Turn<'a> {
    screen: &'a mut Screen,
    app: &'a mut App,
    agent: &'a mut Agent,
    input: &'a mut EventStream,
    questions: &'a mut Option<crate::ui::UiRequests>,
    terminal_input: &'a Option<crate::ui::TerminalInputAsker>,
    host_asker: &'a Option<crate::ui::HostAsker>,
    remote: &'a mut Option<crate::remote::Remote>,
}

async fn submit(
    turn: Turn<'_>,
    mut commands: Option<&mut (dyn Commands + 'static)>,
    line: String,
) -> Result<()> {
    let Turn {
        screen,
        app,
        agent,
        input,
        questions,
        terminal_input,
        host_asker,
        remote,
    } = turn;
    
    let line = match commands {
        Some(ref mut commands) => match commands.submitted(line).await {
            Some(line) => line,
            None => return Ok(()),
        },
        None => line,
    };

    
    if let Some(rest) = line.strip_prefix('!') {
        let (command, shared) = match rest.strip_prefix('!') {
            Some(private) => (private, false),
            None => (rest, true),
        };
        return run_bash(screen, app, agent, commands, command.trim(), shared).await;
    }

    let Some(commands) = commands else {
        mark_prompt_submitted();
        let prompt = app.begin_turn(&line);
        return run_turn(
            screen,
            app,
            agent,
            input,
            questions,
            terminal_input,
            host_asker,
            remote,
            None,
            prompt,
        )
        .await;
    };

    let state = app.conversation_state();
    app.busy("running");
    let dispatched = commands.dispatch(&line, state);
    let outcome = await_command(screen, app, input, dispatched).await?;
    app.idle();

    match outcome {
        
        None => Ok(()),
        Some(None) => {
            mark_prompt_submitted();
            let prompt = app.begin_turn(&line);
            run_turn(
                screen,
                app,
                agent,
                input,
                questions,
                terminal_input,
                host_asker,
                remote,
                Some(commands),
                prompt,
            )
            .await
        }
        
        Some(Some(CommandOutcome::Send { prompt })) => {
            mark_prompt_submitted();
            let prompt = app.begin_turn(&prompt);
            run_turn(
                screen,
                app,
                agent,
                input,
                questions,
                terminal_input,
                host_asker,
                remote,
                Some(commands),
                prompt,
            )
            .await
        }
        Some(Some(outcome)) => apply_outcome(screen, app, agent, input, commands, outcome).await,
    }
}

/// Run a shell command on the user's behalf and put what it printed into the conversation.
async fn run_bash(
    screen: &mut Screen,
    app: &mut App,
    agent: &mut Agent,
    mut commands: Option<&mut (dyn Commands + 'static)>,
    command: &str,
    shared: bool,
) -> Result<()> {
    if command.is_empty() {
        return Ok(());
    }

    app.push_bash(command, shared);
    app.busy("running");
    screen.render(app)?;

    
    let overridden = match commands.as_deref_mut() {
        Some(commands) => {
            commands
                .before_bash(command, !shared, &app.workspace.display().to_string())
                .await
        }
        None => None,
    };

    let (output, failed) = match overridden {
        Some(run) => (run.output, run.failed),
        None => {
            let finished = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&app.workspace)
                .stdin(std::process::Stdio::null())
                .output()
                .await;

            match finished {
                Ok(result) => {
                    let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
                    let errors = String::from_utf8_lossy(&result.stderr);
                    if !errors.is_empty() {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str(&errors);
                    }
                    (text.trim_end().to_string(), !result.status.success())
                }
                Err(error) => (format!("cannot run the command: {error}"), true),
            }
        }
    };
    app.idle();

    let shown = match output.is_empty() {
        true => "(no output)".to_string(),
        false => output.clone(),
    };
    app.notice(
        shown,
        match failed {
            true => MessageKind::Error,
            false => MessageKind::Info,
        },
    );

    
    
    if shared {
        agent.record(Message::user(format!(
            "<bash command=\"{command}\">\n{output}\n</bash>"
        )));
    }
    Ok(())
}

/// Carry out one command outcome, drawing whatever the interface owns and handing the rest to the
/// host.
async fn apply_outcome(
    screen: &mut Screen,
    app: &mut App,
    agent: &mut Agent,
    input: &mut EventStream,
    commands: &mut (dyn Commands + 'static),
    outcome: CommandOutcome,
) -> Result<()> {
    match outcome {
        CommandOutcome::Message { kind, text } => app.notice(text, kind),
        CommandOutcome::Inspect { title, text, items } => app.open_inspection(title, text, items),
        CommandOutcome::Quit => app.should_quit = true,
        CommandOutcome::Choose(picker) => {
            
            let asks = picker.refreshes;
            app.open_picker(picker);
            if asks {
                if let Some(open) = app.picker_mut() {
                    open.set_status("Refreshing model catalogs…", false);
                }
            }
        }
        CommandOutcome::PromptForApiKey {
            provider,
            env_names,
        } => app.open_key_prompt(provider, env_names),

        
        CommandOutcome::DeviceLogin { pending } => {
            app.notice(
                format!(
                    "Open {} and enter the code: {}",
                    pending.verification_uri(),
                    pending.user_code()
                ),
                MessageKind::Info,
            );
            app.busy("waiting for authorization");
            screen.render(app)?;

            let work = commands.finish_device_login(pending);
            let applied = await_host(screen, app, input, work).await?;
            app.idle();
            match applied {
                Some(applied) => apply_applied(app, agent, applied),
                None => app.notice("Sign-in cancelled", MessageKind::Error),
            }
        }

        
        CommandOutcome::SetThinking { level } => {
            agent.set_thinking(level);
            app.set_thinking(level);
            commands.thinking_changed(level).await;
        }
        
        
        CommandOutcome::CopyLastAnswer => app.copy_last_answer(),
        CommandOutcome::Export { path } => app.export(path.as_deref()),

        
        CommandOutcome::Compact if !commands.compacting().await => {
            app.notice("An extension stopped the compaction", MessageKind::Error);
        }
        CommandOutcome::Compact => {
            app.busy("compacting");
            let compacted = await_work(screen, app, input, agent.compact_now()).await?;
            app.idle();
            match compacted {
                
                Some(Ok(summary)) => {
                    commands.compacted(&summary_text(&summary)).await;
                    app.apply_result(Applied::Conversation {
                        messages: agent.messages().to_vec(),
                        note: None,
                    });
                }
                Some(Err(refusal)) => app.notice(refusal.to_string(), MessageKind::Error),
                None => app.notice("Compaction cancelled", MessageKind::Error),
            }
        }

        CommandOutcome::SetTheme { theme } => app.set_theme(match theme {
            micro_commands::ThemeChoice::Dark => Theme::dark(),
            micro_commands::ThemeChoice::Light => Theme::light(),
            micro_commands::ThemeChoice::Auto => background::detect_theme(),
        }),

        CommandOutcome::SetTuiMode { mode } => {
            let mode = match mode {
                micro_config::TuiMode::Regular => TuiMode::Inline,
                micro_config::TuiMode::Fullscreen => TuiMode::Fullscreen,
            };
            screen.set_mode(mode)?;
            app.set_tui_mode(mode);
            app.notice(
                match mode {
                    TuiMode::Inline => "tui_mode is now regular.",
                    TuiMode::Fullscreen => "tui_mode is now fullscreen.",
                },
                MessageKind::Info,
            );
        }

        outcome => {
            app.busy(label_for(&outcome));
            let work = commands.apply(outcome);
            let applied = await_host(screen, app, input, work).await?;
            app.idle();
            if let Some(applied) = applied {
                apply_applied(app, agent, applied);
            }
        }
    }
    Ok(())
}

/// Take in what the host did.
fn apply_applied(app: &mut App, agent: &mut Agent, applied: Applied) {
    match applied {
        Applied::Conversation { messages, note } => {
            
            app.forget_scrolled_out();
            agent.set_messages(messages.clone());
            app.apply_result(Applied::Conversation { messages, note });
        }
        
        Applied::SystemPrompt { note, .. } => {
            
            app.forget_workspace_files();
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        Applied::Model { swap, note } => {
            
            app.set_model_label(swap.model.id.clone());
            app.context_window = swap.context_window as u32;
            app.set_price(swap.cost.clone());
            app.set_thinking(swap.model.thinking);
            agent.set_model(*swap);
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        other => app.apply_result(other),
    }
}

/// What a summary says, for whoever is told the conversation was compacted.
fn summary_text(message: &Message) -> String {
    match message {
        Message::Assistant(assistant) => assistant.text(),
        Message::User { content, .. } => content
            .iter()
            .map(micro_types::ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join(""),
        Message::ToolResult { .. } => String::new(),
    }
}

/// What the activity line says while the host is carrying an outcome out.
fn label_for(outcome: &CommandOutcome) -> &'static str {
    match outcome {
        CommandOutcome::Resume { .. } | CommandOutcome::Fork { .. } => "loading",
        CommandOutcome::DeviceLogin { .. } => "waiting for sign-in",
        _ => "working",
    }
}

/// Await the host while the interface keeps painting and ctrl+c stays live.
async fn await_host<F>(
    screen: &mut Screen,
    app: &mut App,
    input: &mut EventStream,
    work: F,
) -> Result<Option<Applied>>
where
    F: Future<Output = Applied>,
{
    await_work(screen, app, input, work).await
}

async fn await_command<F>(
    screen: &mut Screen,
    app: &mut App,
    input: &mut EventStream,
    work: F,
) -> Result<Option<Option<CommandOutcome>>>
where
    F: Future<Output = Option<CommandOutcome>>,
{
    await_work(screen, app, input, work).await
}

async fn await_work<T, F>(
    screen: &mut Screen,
    app: &mut App,
    input: &mut EventStream,
    work: F,
) -> Result<Option<T>>
where
    F: Future<Output = T>,
{
    let mut work = Box::pin(work);
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut painted = Instant::now() - FRAME;

    loop {
        if painted.elapsed() >= FRAME {
            screen.render(app)?;
            painted = Instant::now();
        }

        tokio::select! {
            biased;
            event = input.next() => match event {
                Some(Ok(event)) => match handle(app, event) {
                    
                    Outcome::ExternalEditor => {}
                    
                    Outcome::ThinkingChanged(_) => {}
                    
                    Outcome::CycleModel(_) | Outcome::Suspend => {}
                    Outcome::Quit => {
                        app.should_quit = true;
                        return Ok(None);
                    }
                    Outcome::Interrupt => return Ok(None),
                    Outcome::Handled => {}
                },
                Some(Err(_)) | None => {
                    app.should_quit = true;
                    return Ok(None);
                }
            },
            done = &mut work => return Ok(Some(done)),
            _ = ticker.tick() => app.tick = app.tick.wrapping_add(1),
        }
    }
}

/// Drive one exchange, keeping the interface live while the agent works.
async fn run_turn(
    screen: &mut Screen,
    app: &mut App,
    agent: &mut Agent,
    input: &mut EventStream,
    questions: &mut Option<crate::ui::UiRequests>,
    terminal_input: &Option<crate::ui::TerminalInputAsker>,
    host_asker: &Option<crate::ui::HostAsker>,
    remote: &mut Option<crate::remote::Remote>,
    mut commands: Option<&mut (dyn Commands + 'static)>,
    prompt: Message,
) -> Result<()> {
    
    if let Some(remote) = remote.as_ref() {
        remote.report_running(true);
    }
    let (sender, mut receiver) = unbounded_channel::<AgentEvent>();
    let progress = app.settings().terminal_progress;
    report_progress(progress, true);
    let mut turn = Box::pin(agent.run(prompt, &sender));
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut painted = Instant::now() - FRAME;
    let mut aborted = false;
    
    
    let mut suggesting: Option<(String, tokio::sync::oneshot::Receiver<Value>)> = None;
    let mut completing: Option<tokio::sync::oneshot::Receiver<Value>> = None;

    loop {
        if painted.elapsed() >= FRAME {
            screen.render(app)?;
            painted = Instant::now();
        }
        if suggesting.is_none() {
            if let (Some(request), Some(asker)) =
                (app.take_pending_suggestion_request(), host_asker)
            {
                suggesting = Some(ask_for_suggestions(asker.clone(), request));
            }
        }
        if completing.is_none() {
            if let (Some(request), Some(asker)) =
                (app.take_pending_completion_request(), host_asker)
            {
                completing = Some(ask_to_apply_completion(asker.clone(), request));
            }
        }

        
        tokio::select! {
            biased;
            event = input.next() => match event {
                Some(Ok(event)) if offer_component_input(host_asker, app, &event).await => {}
                Some(Ok(event)) if offer_terminal_input(terminal_input, app, &event).await => {}
                Some(Ok(event)) if offer_editor_component_input(host_asker, app, &event).await => {}
                Some(Ok(event)) => match handle(app, event) {
                    Outcome::ExternalEditor => {}
                    Outcome::ThinkingChanged(_) => {}
                    Outcome::CycleModel(_) | Outcome::Suspend => {}
                    Outcome::Quit => {
                        app.should_quit = true;
                        aborted = true;
                        break;
                    }
                    Outcome::Interrupt => {
                        aborted = true;
                        break;
                    }
                    Outcome::Handled => {}
                },
                Some(Err(_)) | None => {
                    app.should_quit = true;
                    aborted = true;
                    break;
                }
            },
            Some(question) = next_question(questions) => {
                app.ask_question(question);
                
                if app.is_interrupting() {
                    aborted = true;
                    break;
                }
            }
            Some(event) = receiver.recv() => {
                app.apply_event(event);
                while let Ok(next) = receiver.try_recv() {
                    app.apply_event(next);
                }
            }
            answer = async {
                let (_, receiver) = suggesting.as_mut().unwrap();
                receiver.await
            }, if suggesting.is_some() => {
                let (prefix, _) = suggesting.take().expect("guarded by is_some");
                if let Ok(answer) = answer {
                    let items = answer
                        .get("items")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    app.apply_extension_suggestions(&prefix, items);
                }
            }
            answer = async { completing.as_mut().unwrap().await }, if completing.is_some() => {
                completing = None;
                if let Ok(answer) = answer {
                    apply_extension_completion_answer(app, &answer);
                }
            }
            _ = &mut turn => break,
            _ = ticker.tick() => app.tick = app.tick.wrapping_add(1),
        }
    }

    
    drop(turn);
    while let Ok(event) = receiver.try_recv() {
        app.apply_event(event);
    }
    app.finish_turn(aborted);
    if let Some(commands) = commands.as_mut() {
        let observed = commands.session_observability().await;
        app.set_session_observability(observed);
    }
    report_progress(progress, false);
    Ok(())
}

/// The next question from an extension.
async fn next_question(
    requests: &mut Option<crate::ui::UiRequests>,
) -> Option<crate::ui::UiRequest> {
    match requests {
        Some(requests) => requests.recv().await,
        None => std::future::pending().await,
    }
}

fn handle(app: &mut App, event: Event) -> Outcome {
    app.handle(event::action_for(&event))
}

/// A key nothing built in wanted, offered to whatever registered shortcuts.
async fn offer_shortcut(commands: &mut Option<Box<dyn Commands + 'static>>, event: &Event) -> bool {
    if event::action_for(event) != event::Action::Ignored {
        return false;
    }
    let Some(name) = event::key_name(event) else {
        return false;
    };
    match commands {
        Some(commands) => commands.shortcut(&name).await,
        None => false,
    }
}

/// Offer a key to `ctx.ui.onTerminalInput` before the interface does anything with it itself.
async fn offer_terminal_input(
    terminal_input: &Option<crate::ui::TerminalInputAsker>,
    app: &App,
    event: &Event,
) -> bool {
    if !app.wants_terminal_input() {
        return false;
    }
    let Some(terminal_input) = terminal_input else {
        return false;
    };
    let Some(data) = event::key_to_data(event) else {
        return false;
    };
    terminal_input
        .ask(data)
        .await
        .get("consume")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Hand a key to the component a `custom()` overlay has open, and redraw it with what it looked
/// like afterward.
async fn offer_component_input(
    host_asker: &Option<crate::ui::HostAsker>,
    app: &mut App,
    event: &Event,
) -> bool {
    let Some(component_id) = app.component_overlay_id() else {
        return false;
    };
    if matches!(
        event::action_for(event),
        event::Action::Cancel | event::Action::Interrupt
    ) {
        return false;
    }
    let Some(host_asker) = host_asker else {
        return false;
    };
    let Some(data) = event::key_to_data(event) else {
        return false;
    };
    let component_id = component_id.to_string();
    let answer = host_asker
        .ask(
            "component_input",
            serde_json::json!({ "componentId": component_id, "data": data }),
        )
        .await;
    let lines = answer
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    app.set_component_overlay_lines(&component_id, lines);
    true
}

/// Offer a key to the component `setEditorComponent` replaced the built-in editor with, before the
/// built-in editor sees it.
async fn offer_editor_component_input(
    host_asker: &Option<crate::ui::HostAsker>,
    app: &mut App,
    event: &Event,
) -> bool {
    let Some(component_id) = app.editor_component_id() else {
        return false;
    };
    let Some(host_asker) = host_asker else {
        return false;
    };
    let Some(data) = event::key_to_data(event) else {
        return false;
    };
    let component_id = component_id.to_string();
    
    let text = app.editor_text();
    let answer = host_asker
        .ask(
            "component_input",
            serde_json::json!({ "componentId": component_id, "data": data, "text": text }),
        )
        .await;
    let lines = answer
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    app.set_editor_component_lines(&component_id, lines);
    answer
        .get("consume")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// The terminal, and the inline region the interface lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiMode {
    /// A region at the cursor, as tall as the interface needs.
    Inline,
    /// The whole screen, which leaves the scrollback untouched and scrolls internally.
    #[default]
    Fullscreen,
}

/// How tall the inline region starts.
const INLINE_ROWS: u16 = 8;

/// Alternate scroll.
const ENABLE_ALTERNATE_SCROLL: &str = "\x1b[?1007h";
const DISABLE_ALTERNATE_SCROLL: &str = "\x1b[?1007l";

/// The terminal, with the one question micro can answer itself answered here.
struct Anchored<B> {
    inner: B,
    /// The row to anchor the next inline region at.
    anchor: Option<u16>,
}

/// The backend micro draws through, which is crossterm with that one question answered.
type Backing = Anchored<CrosstermBackend<Stdout>>;

impl Backing {
    fn new(anchor: Option<u16>) -> Self {
        Anchored::over(CrosstermBackend::new(std::io::stdout()), anchor)
    }
}

impl<B> Anchored<B> {
    fn over(inner: B, anchor: Option<u16>) -> Self {
        Anchored { inner, anchor }
    }
}

impl<B: ratatui::backend::Backend> ratatui::backend::Backend for Anchored<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), B::Error>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        self.inner.draw(content)
    }

    fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, B::Error> {
        let Some(row) = self.anchor else {
            return self.inner.get_cursor_position();
        };
        
        let position = ratatui::layout::Position { x: 0, y: row };
        self.inner.set_cursor_position(position)?;
        Ok(position)
    }

    fn append_lines(&mut self, lines: u16) -> Result<(), B::Error> {
        self.inner.append_lines(lines)
    }

    fn hide_cursor(&mut self) -> Result<(), B::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), B::Error> {
        self.inner.show_cursor()
    }

    fn set_cursor_position<P: Into<ratatui::layout::Position>>(
        &mut self,
        position: P,
    ) -> Result<(), B::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), B::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> Result<(), B::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<ratatui::layout::Size, B::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, B::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), B::Error> {
        self.inner.flush()
    }
}


trait Fresh: ratatui::backend::Backend + Sized {
    fn fresh(&self) -> Self;
}

impl Fresh for CrosstermBackend<Stdout> {
    /// Another handle on the process's stdout, which is the same terminal.
    fn fresh(&self) -> Self {
        CrosstermBackend::new(std::io::stdout())
    }
}

#[cfg(test)]
impl Fresh for ratatui::backend::TestBackend {
    /// A copy carries the whole simulated screen.
    fn fresh(&self) -> Self {
        self.clone()
    }
}

struct Screen<B: ratatui::backend::Backend = CrosstermBackend<Stdout>> {
    terminal: Terminal<Anchored<B>>,
    mode: TuiMode,
    /// How tall the inline region is now, so a change in what the interface needs can be noticed
    /// and the region resized to match.
    rows: u16,
    /// The row an inline region starts at.
    anchor: Option<u16>,
    /// The terminal's size when the region was last built.
    size: ratatui::layout::Size,
}

/// Build an inline region `rows` tall at the backend's anchor.
fn inline_region<B>(backend: Anchored<B>, rows: u16) -> Result<Terminal<Anchored<B>>>
where
    B: ratatui::backend::Backend,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let mut terminal = Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(rows),
        },
    )?;
    let landed = Some(terminal.get_frame().area().y);
    terminal.backend_mut().anchor = landed;
    Ok(terminal)
}

impl Screen {
    /// Take the terminal, without asking it to report the mouse.
    fn enter(mode: TuiMode) -> Result<Self> {
        enable_raw_mode()?;
        
        let anchor = crossterm::cursor::position().ok().map(|(_, row)| row);
        match mode {
            TuiMode::Fullscreen => {
                execute!(
                    std::io::stdout(),
                    EnterAlternateScreen,
                    EnableBracketedPaste,
                    Print(ENABLE_ALTERNATE_SCROLL)
                )?;
                let mut terminal = Terminal::new(Anchored::new(anchor))?;
                terminal.clear()?;
                let size = terminal.size()?;
                Ok(Screen {
                    terminal,
                    mode,
                    rows: 0,
                    anchor,
                    size,
                })
            }
            TuiMode::Inline => {
                execute!(std::io::stdout(), EnableBracketedPaste)?;
                
                let terminal = inline_region(Anchored::new(anchor), INLINE_ROWS)?;
                let size = terminal.size()?;
                let mut screen = Screen {
                    terminal,
                    mode,
                    rows: 0,
                    anchor: None,
                    size,
                };
                screen.settle();
                Ok(screen)
            }
        }
    }

    /// Switch between the terminal's scrollback and an alternate screen without ending the session.
    fn set_mode(&mut self, mode: TuiMode) -> Result<()> {
        if mode == self.mode {
            return Ok(());
        }

        match mode {
            TuiMode::Fullscreen => {
                
                self.anchor = Some(self.terminal.get_frame().area().y);
                execute!(
                    std::io::stdout(),
                    EnterAlternateScreen,
                    EnableBracketedPaste,
                    Print(ENABLE_ALTERNATE_SCROLL)
                )?;
                let mut terminal = Terminal::new(Anchored::new(self.anchor))?;
                terminal.clear()?;
                self.size = terminal.size()?;
                self.terminal = terminal;
                self.rows = 0;
            }
            TuiMode::Inline => {
                execute!(
                    std::io::stdout(),
                    LeaveAlternateScreen,
                    EnableBracketedPaste,
                    Print(DISABLE_ALTERNATE_SCROLL)
                )?;
                
                self.size = self.terminal.size()?;
                self.rebuild(INLINE_ROWS, true)?;
            }
        }
        self.mode = mode;
        Ok(())
    }

    /// Take the terminal back after another program has had it.
    fn reopen(&mut self) -> Result<()> {
        enable_raw_mode()?;
        match self.mode {
            TuiMode::Fullscreen => {
                execute!(
                    std::io::stdout(),
                    EnterAlternateScreen,
                    EnableBracketedPaste,
                    Print(ENABLE_ALTERNATE_SCROLL)
                )?;
                self.terminal.clear()?;
            }
            TuiMode::Inline => {
                execute!(std::io::stdout(), EnableBracketedPaste)?;
                
                if let Ok((_, row)) = crossterm::cursor::position() {
                    self.anchor = Some(row);
                }
                self.size = self.terminal.size()?;
                self.rebuild(self.rows.max(1), true)?;
            }
        }
        Ok(())
    }
}

impl<B> Screen<B>
where
    B: Fresh,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    /// Record where the region actually is.
    fn settle(&mut self) {
        let area = self.terminal.get_frame().area();
        self.rows = area.height;
        self.anchor = Some(area.y);
    }

    /// Replace the region with one `rows` tall at the kept anchor.
    fn rebuild(&mut self, rows: u16, clear_below: bool) -> Result<()> {
        if clear_below {
            if let Some(row) = self.anchor {
                let backend = self.terminal.backend_mut();
                backend.set_cursor_position(ratatui::layout::Position { x: 0, y: row })?;
                backend.clear_region(ratatui::backend::ClearType::AfterCursor)?;
            }
        }
        let backend = Anchored::over(self.terminal.backend().inner.fresh(), self.anchor);
        self.terminal = inline_region(backend, rows)?;
        self.settle();
        Ok(())
    }

    /// Blank every row the region stands on, leaving the rest of the screen alone.
    fn blank_footprint(&mut self) -> Result<()> {
        let area = self.terminal.get_frame().area();
        let backend = self.terminal.backend_mut();
        for row in area.top()..area.bottom() {
            backend.set_cursor_position(ratatui::layout::Position { x: 0, y: row })?;
            backend.clear_region(ratatui::backend::ClearType::CurrentLine)?;
        }
        backend.flush()?;
        Ok(())
    }

    /// Follow the terminal through a resize, before ratatui tries to.
    fn fit_screen(&mut self) -> Result<()> {
        let size = self.terminal.size()?;
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        let rows = self.rows.clamp(1, size.height.max(1));
        self.anchor = self
            .anchor
            .map(|row| row.min(size.height.saturating_sub(rows)));
        
        self.rebuild(rows, true)
    }

    /// Hand what the conversation has finished with to the terminal, above the region.
    fn insert(&mut self, finished: &[Line<'_>]) -> Result<()> {
        if finished.is_empty() {
            return Ok(());
        }
        let rows = finished.len() as u16;
        self.terminal.insert_before(rows, |buffer| {
            for (offset, line) in finished.iter().enumerate() {
                buffer.set_line(0, offset as u16, line, buffer.area.width);
            }
        })?;
        
        self.anchor = Some(self.terminal.get_frame().area().y);
        self.terminal.backend_mut().anchor = self.anchor;
        Ok(())
    }

    /// Make the region as tall as the interface has turned out to be.
    fn fit_rows(&mut self, wanted: u16, clear_on_shrink: bool) -> Result<()> {
        if wanted == self.rows {
            return Ok(());
        }
        match wanted < self.rows && !clear_on_shrink {
            true => {
                self.blank_footprint()?;
                self.rebuild(wanted, false)
            }
            false => self.rebuild(wanted, true),
        }
    }

    
    fn render(&mut self, app: &mut App) -> Result<()> {
        if self.mode == TuiMode::Inline {
            self.fit_screen()?;
            
            let finished = app.take_scrolled_out();
            self.insert(&finished)?;
            let wanted = render::interface_rows(app, &app.theme, self.size.width, self.rows).max(1);
            self.fit_rows(wanted, app.settings().clear_on_shrink)?;
        }
        self.terminal.draw(|frame| render::draw(frame, app))?;
        if let Some(title) = app.take_title_change() {
            set_terminal_title(&title);
        }
        Ok(())
    }
}

fn leave() {
    
    if let Some(protocol) = capabilities::detect().images {
        if let Some(escape) = images::forget_all(protocol) {
            use std::io::Write as _;
            let mut out = std::io::stdout();
            let _ = out.write_all(escape.as_bytes());
        }
    }
    let _ = execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        Print(DISABLE_ALTERNATE_SCROLL),
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

/// A panic would otherwise leave the terminal in raw mode with the message swallowed.
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            leave();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waiting answers three different ways, and two of them are not the end of the session.
    #[test]
    fn only_the_input_ending_ends_the_session() {
        assert!(matches!(arrived(None), Next::Ended));
        assert!(matches!(
            arrived(Some(Err(std::io::Error::other("gone")))),
            Next::Ended
        ));
        assert!(matches!(
            arrived(Some(Ok(Event::FocusGained))),
            Next::Event(_)
        ));
    }

    /// The row [`Anchored`] reports is the row the cursor is left on.
    #[test]
    fn the_reported_row_is_the_row_the_cursor_is_left_on() {
        use ratatui::backend::Backend as _;

        let mut backend = Anchored::over(ratatui::backend::TestBackend::new(20, 10), Some(4));
        backend
            .set_cursor_position(ratatui::layout::Position { x: 7, y: 9 })
            .unwrap();

        let reported = backend.get_cursor_position().unwrap();

        assert_eq!(reported, ratatui::layout::Position { x: 0, y: 4 });
        assert_eq!(backend.inner.cursor_position(), reported);
    }

    /// A region rebuilt while the cursor sits at the foot of the one it replaces.
    #[test]
    fn a_rebuilt_region_is_where_it_says_it_is() {
        use ratatui::backend::Backend as _;

        let screen = [
            "row0", "row1", "row2", "row3", "row4", "row5", "row6", "row7", "row8", "row9",
        ];
        let mut backend =
            Anchored::over(ratatui::backend::TestBackend::with_lines(screen), Some(2));
        
        backend
            .set_cursor_position(ratatui::layout::Position { x: 3, y: 9 })
            .unwrap();

        let mut terminal = inline_region(backend, 8).unwrap();

        
        assert_eq!(
            terminal.get_frame().area(),
            ratatui::layout::Rect::new(0, 2, 4, 8)
        );
        assert_eq!(terminal.backend().anchor, Some(2));
        terminal.backend().inner.assert_buffer_lines(screen);
    }

    
    #[test]
    fn a_region_that_scrolled_reports_where_it_landed() {
        let backend = Anchored::over(ratatui::backend::TestBackend::new(20, 10), Some(6));

        
        let mut terminal = inline_region(backend, 10).unwrap();

        assert_eq!(terminal.get_frame().area().y, 0);
        assert_eq!(terminal.backend().anchor, Some(0));
    }

    
    fn inline_screen(
        backend: ratatui::backend::TestBackend,
        anchor: u16,
        rows: u16,
    ) -> Screen<ratatui::backend::TestBackend> {
        let terminal = inline_region(Anchored::over(backend, Some(anchor)), rows).unwrap();
        let size = terminal.size().unwrap();
        let mut screen = Screen {
            terminal,
            mode: TuiMode::Inline,
            rows: 0,
            anchor: None,
            size,
        };
        screen.settle();
        screen
    }

    /// Paint every row of the region as `glyph` and its offset, left-aligned: the blank cells to
    /// the right are the point.
    fn paint(screen: &mut Screen<ratatui::backend::TestBackend>, glyph: char) {
        screen
            .terminal
            .draw(|frame| {
                let area = frame.area();
                for (offset, y) in (area.top()..area.bottom()).enumerate() {
                    frame.buffer_mut().set_string(
                        area.x,
                        y,
                        format!("{glyph}{offset}"),
                        ratatui::style::Style::default(),
                    );
                }
            })
            .unwrap();
    }

    /// The slash-menu session, played out on a simulated screen: the menu opens (grow), filters
    /// down (shrink), and reopens (grow).
    #[test]
    fn a_menu_opening_filtering_and_reopening_leaves_only_the_current_frame() {
        let backend = ratatui::backend::TestBackend::with_lines([
            "history-00",
            "history-01",
            "history-02",
            "history-03",
            "history-04",
            "history-05",
            "history-06",
            "history-07",
            "          ",
            "          ",
            "          ",
            "          ",
        ]);
        let mut screen = inline_screen(backend, 8, 3);
        paint(&mut screen, 'A');
        screen.terminal.backend().inner.assert_buffer_lines([
            "history-00",
            "history-01",
            "history-02",
            "history-03",
            "history-04",
            "history-05",
            "history-06",
            "history-07",
            "A0        ",
            "A1        ",
            "A2        ",
            "          ",
        ]);

        
        screen.fit_rows(9, false).unwrap();
        paint(&mut screen, 'B');
        assert_eq!(screen.anchor, Some(3));
        assert_eq!(screen.rows, 9);
        screen.terminal.backend().inner.assert_buffer_lines([
            "history-05",
            "history-06",
            "history-07",
            "B0        ",
            "B1        ",
            "B2        ",
            "B3        ",
            "B4        ",
            "B5        ",
            "B6        ",
            "B7        ",
            "B8        ",
        ]);
        screen.terminal.backend().inner.assert_scrollback_lines([
            "history-00",
            "history-01",
            "history-02",
            "history-03",
            "history-04",
        ]);

        
        screen.fit_rows(4, false).unwrap();
        paint(&mut screen, 'C');
        assert_eq!(screen.anchor, Some(3));
        screen.terminal.backend().inner.assert_buffer_lines([
            "history-05",
            "history-06",
            "history-07",
            "C0        ",
            "C1        ",
            "C2        ",
            "C3        ",
            "          ",
            "          ",
            "          ",
            "          ",
            "          ",
        ]);

        
        screen.fit_rows(6, false).unwrap();
        paint(&mut screen, 'D');
        screen.terminal.backend().inner.assert_buffer_lines([
            "history-05",
            "history-06",
            "history-07",
            "D0        ",
            "D1        ",
            "D2        ",
            "D3        ",
            "D4        ",
            "D5        ",
            "          ",
            "          ",
            "          ",
        ]);
    }

    
    #[test]
    fn clear_on_shrink_sweeps_to_the_foot_of_the_screen() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let mut screen = inline_screen(backend, 2, 9);
        paint(&mut screen, 'A');

        screen.fit_rows(3, true).unwrap();
        paint(&mut screen, 'B');
        screen.terminal.backend().inner.assert_buffer_lines([
            "          ",
            "          ",
            "B0        ",
            "B1        ",
            "B2        ",
            "          ",
            "          ",
            "          ",
            "          ",
            "          ",
            "          ",
            "          ",
        ]);
    }

    
    #[test]
    fn inserted_lines_land_above_the_region_and_the_region_follows() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let mut screen = inline_screen(backend, 4, 4);
        paint(&mut screen, 'A');

        screen
            .insert(&[Line::raw("said-00"), Line::raw("said-01")])
            .unwrap();
        paint(&mut screen, 'B');
        screen.terminal.backend().inner.assert_buffer_lines([
            "          ",
            "          ",
            "          ",
            "          ",
            "said-00   ",
            "said-01   ",
            "B0        ",
            "B1        ",
            "B2        ",
            "B3        ",
            "          ",
            "          ",
        ]);

        
        screen.fit_rows(2, false).unwrap();
        paint(&mut screen, 'C');
        screen.terminal.backend().inner.assert_buffer_lines([
            "          ",
            "          ",
            "          ",
            "          ",
            "said-00   ",
            "said-01   ",
            "C0        ",
            "C1        ",
            "          ",
            "          ",
            "          ",
            "          ",
        ]);
    }

    
    #[test]
    fn a_resize_rebuilds_the_region_on_the_screen_that_remains() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let mut screen = inline_screen(backend, 4, 8);
        paint(&mut screen, 'A');

        
        screen.terminal.backend_mut().inner.resize(10, 6);
        screen.fit_screen().unwrap();
        paint(&mut screen, 'B');
        assert_eq!(screen.size, ratatui::layout::Size::new(10, 6));
        assert_eq!(screen.anchor, Some(0));
        assert_eq!(screen.rows, 6);
        screen.terminal.backend().inner.assert_buffer_lines([
            "B0        ",
            "B1        ",
            "B2        ",
            "B3        ",
            "B4        ",
            "B5        ",
        ]);

        
        screen.terminal.backend_mut().inner.resize(10, 12);
        screen.fit_screen().unwrap();
        paint(&mut screen, 'C');
        assert_eq!(screen.anchor, Some(0));
        assert_eq!(screen.rows, 6);
    }

    /// A key offered to `setEditorComponent`'s replacement carries the built-in editor's text along
    /// with it.
    #[tokio::test]
    async fn a_key_offered_to_the_editor_replacement_carries_the_buffers_text() {
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyModifiers;

        let (asker, mut asks) = crate::ui::host_ask_channel();
        let mut app = App::new(&[], TuiOptions::default());
        let (request, _answered) = crate::ui::UiRequest::for_test(
            "set_editor_component",
            "component-1",
            None,
            vec!["> ".to_string()],
        );
        app.ask_question(request);
        app.editor.insert_str("draft");
        assert_eq!(app.editor_text(), "draft");

        let event = Event::Key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
        let asking = tokio::spawn(async move {
            offer_editor_component_input(&Some(asker), &mut app, &event).await
        });

        let mut ask = asks.recv().await.expect("a component_input ask");
        assert_eq!(ask.event, "component_input");
        assert_eq!(ask.payload["text"], "draft");
        ask.answer(serde_json::json!({ "consume": false, "lines": ["> "] }));

        assert!(
            !asking.await.unwrap(),
            "the component did not consume the key"
        );
    }
}
