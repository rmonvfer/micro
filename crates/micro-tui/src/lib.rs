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
use crate::render::pictures::Placement;
use anyhow::Result;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::EventStream;
use crossterm::execute;
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
use std::collections::HashSet;
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

/// Ensures remote observers see an interrupted turn as settled even if rendering returns an error.
struct RemoteTurnGuard(Option<tokio::sync::mpsc::UnboundedSender<crate::remote::ToPhone>>);

impl Drop for RemoteTurnGuard {
    fn drop(&mut self) {
        if let Some(outgoing) = self.0.as_ref() {
            let _ = outgoing.send(crate::remote::ToPhone::Running(false));
        }
    }
}

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
    let mut interface = Interface {
        screen,
        agent,
        app: &mut app,
        questions: &mut questions,
        commands: &mut commands,
        terminal_input: &terminal_input,
        host_asker: &host_asker,
        remote: &mut remote,
    };
    let outcome = run_loop(&mut interface).await;
    *said = app.plain_lines();
    outcome
}

/// The shared state needed to drive the interactive interface.
struct Interface<'a> {
    screen: &'a mut Screen,
    agent: &'a mut Agent,
    app: &'a mut App,
    questions: &'a mut Option<crate::ui::UiRequests>,
    commands: &'a mut Option<Box<dyn Commands + 'static>>,
    terminal_input: &'a Option<crate::ui::TerminalInputAsker>,
    host_asker: &'a Option<crate::ui::HostAsker>,
    remote: &'a mut Option<crate::remote::Remote>,
}

/// Work that can complete while the interface is waiting for an event.
#[derive(Default)]
struct PendingWork {
    refreshing: Option<tokio::sync::oneshot::Receiver<Listings>>,
    suggesting: Option<(String, tokio::sync::oneshot::Receiver<Value>)>,
    completing: Option<tokio::sync::oneshot::Receiver<Value>>,
}

async fn run_loop(interface: &mut Interface<'_>) -> Result<()> {
    let mut input = EventStream::new();
    let mut pending = PendingWork::default();

    loop {
        interface.screen.render(interface.app)?;
        if interface.app.should_quit {
            return Ok(());
        }

        let retired = interface.app.take_retired_tools();
        if !retired.is_empty() {
            interface.agent.remove_tools(&retired);
        }

        if let Some((provider, key)) = interface.app.take_key_prompt() {
            if let Some(commands) = interface.commands.as_mut() {
                interface.app.busy("signing in");
                let stored = commands.store_api_key(provider, key);
                let applied =
                    await_host(interface.screen, interface.app, &mut input, stored).await?;
                interface.app.idle();
                if let Some(applied) = applied {
                    apply_applied(interface.app, interface.agent, applied);
                }
            }
            continue;
        }

        if let Some(question) = interface
            .questions
            .as_mut()
            .and_then(|questions| questions.try_recv())
        {
            interface.app.ask_question(question);
            continue;
        }

        match interface.app.take_submission() {
            Some(line) => submit(interface, &mut input, line).await?,
            None => match next_event(interface, &mut input, &mut pending).await {
                Next::Redrawn => continue,
                Next::Remote(action) => {
                    let _ = handle_remote(interface.app, action);
                }
                Next::Event(event) => {
                    if offer_component_input(interface.host_asker, interface.app, &event).await {
                        continue;
                    }
                    if offer_terminal_input(interface.terminal_input, interface.app, &event).await {
                        continue;
                    }
                    if offer_editor_component_input(interface.host_asker, interface.app, &event)
                        .await
                    {
                        continue;
                    }
                    if offer_shortcut(interface.commands, &event).await {
                        continue;
                    }
                    match handle(interface.app, event) {
                        Outcome::Quit => return Ok(()),
                        Outcome::ExternalEditor => {
                            external_editor(interface.screen, interface.app)?
                        }
                        Outcome::ThinkingChanged(level) => {
                            interface.agent.set_thinking(level);
                            if let Some(commands) = interface.commands.as_mut() {
                                commands.thinking_changed(level).await;
                            }
                        }

                        Outcome::CycleModel(forward) => {
                            interface.app.queue_line(match forward {
                                true => "/model next",
                                false => "/model previous",
                            });
                        }
                        Outcome::Suspend => suspend(interface.screen, interface.app)?,
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
    /// An action sent from a paired phone.
    Remote(crate::remote::FromPhone),
    /// The input ended, so the session is over.
    Ended,
}

async fn next_event(
    interface: &mut Interface<'_>,
    input: &mut EventStream,
    pending: &mut PendingWork,
) -> Next {
    if pending.refreshing.is_none()
        && interface
            .app
            .picker_mut()
            .is_some_and(|open| open.refreshes())
    {
        if let Some(commands) = interface.commands.as_deref_mut() {
            pending.refreshing = commands.begin_model_refresh();
        }
    }

    if pending.suggesting.is_none() {
        if let (Some(request), Some(asker)) = (
            interface.app.take_pending_suggestion_request(),
            interface.host_asker,
        ) {
            pending.suggesting = Some(ask_for_suggestions(asker.clone(), request));
        }
    }

    if pending.completing.is_none() {
        if let (Some(request), Some(asker)) = (
            interface.app.take_pending_completion_request(),
            interface.host_asker,
        ) {
            pending.completing = Some(ask_to_apply_completion(asker.clone(), request));
        }
    }

    tokio::select! {
        biased;
        event = input.next() => arrived(event),
        listings = async { pending.refreshing.as_mut().unwrap().await }, if pending.refreshing.is_some() => {
            pending.refreshing = None;
            let listings = listings.unwrap_or_default();
            let errors = listings.errors.clone();
            let rebuilt = match interface.commands.as_deref_mut() {
                Some(commands) => commands.apply_model_refresh(listings).await,
                None => None,
            };
            if let Some(open) = interface.app.picker_mut() {

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
            let (_, receiver) = pending.suggesting.as_mut().unwrap();
            receiver.await
        }, if pending.suggesting.is_some() => {

            let (prefix, _) = pending.suggesting.take().expect("guarded by is_some");
            if let Ok(answer) = answer {
                let items = answer
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                interface.app.apply_extension_suggestions(&prefix, items);
            }
            Next::Redrawn
        }
        answer = async { pending.completing.as_mut().unwrap().await }, if pending.completing.is_some() => {
            pending.completing = None;
            if let Ok(answer) = answer {
                apply_extension_completion_answer(interface.app, &answer);
            }
            Next::Redrawn
        }
        action = async { interface.remote.as_mut().unwrap().incoming.recv().await }, if interface.remote.is_some() => {
            match action {
                Some(action) => Next::Remote(action),
                None => {
                    *interface.remote = None;
                    Next::Redrawn
                }
            }
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

    let directory = secure_temp_directory()?;
    let path = directory.join("prompt.md");
    if std::fs::write(&path, app.editor.text()).is_err() {
        app.notice("Could not write the prompt to a file.", MessageKind::Error);
        let _ = std::fs::remove_dir_all(directory);
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
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

/// Make a private directory for an editor buffer so no other account can replace its path.
fn secure_temp_directory() -> Result<std::path::PathBuf> {
    use rand::RngCore as _;

    for _ in 0..16 {
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let path = std::env::temp_dir().join(format!("micro-prompt-{}", hex(&nonce)));
        match std::fs::create_dir(&path) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("could not make a private temporary directory")
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

/// Send one submitted line where it belongs: to the shell, to a command, or to the model.
struct Turn<'a> {
    screen: &'a mut Screen,
    app: &'a mut App,
    agent: &'a mut Agent,
    input: &'a mut EventStream,
    questions: &'a mut Option<crate::ui::UiRequests>,
    commands: &'a mut Option<Box<dyn Commands + 'static>>,
    terminal_input: &'a Option<crate::ui::TerminalInputAsker>,
    host_asker: &'a Option<crate::ui::HostAsker>,
    remote: &'a mut Option<crate::remote::Remote>,
}

async fn submit(
    interface: &mut Interface<'_>,
    input: &mut EventStream,
    line: String,
) -> Result<()> {
    let mut turn = Turn {
        screen: interface.screen,
        app: interface.app,
        agent: interface.agent,
        input,
        questions: interface.questions,
        commands: interface.commands,
        terminal_input: interface.terminal_input,
        host_asker: interface.host_asker,
        remote: interface.remote,
    };

    let line = match turn.commands.as_mut() {
        Some(commands) => match commands.submitted(line).await {
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
        return run_bash(
            turn.screen,
            turn.app,
            turn.agent,
            turn.commands.as_deref_mut(),
            command.trim(),
            shared,
        )
        .await;
    }

    if turn.commands.is_none() {
        mark_prompt_submitted();
        let prompt = turn.app.begin_turn(&line);
        return run_turn(&mut turn, prompt).await;
    }

    let outcome = {
        let state = turn.app.conversation_state();
        turn.app.busy("running");
        let dispatched = turn
            .commands
            .as_mut()
            .expect("checked above")
            .dispatch(&line, state);
        let outcome = await_command(turn.screen, turn.app, turn.input, dispatched).await?;
        turn.app.idle();
        outcome
    };

    match outcome {
        None => Ok(()),
        Some(None) => {
            mark_prompt_submitted();
            let prompt = turn.app.begin_turn(&line);
            run_turn(&mut turn, prompt).await
        }

        Some(Some(CommandOutcome::Send { prompt })) => {
            mark_prompt_submitted();
            let prompt = turn.app.begin_turn(&prompt);
            run_turn(&mut turn, prompt).await
        }
        Some(Some(outcome)) => {
            apply_outcome(
                turn.screen,
                turn.app,
                turn.agent,
                turn.input,
                turn.commands.as_deref_mut().expect("checked above"),
                outcome,
            )
            .await
        }
    }
}

/// Run a shell command on the user's behalf and put what it printed into the conversation.
async fn run_bash(
    screen: &mut Screen,
    app: &mut App,
    agent: &mut Agent,
    commands: Option<&mut (dyn Commands + 'static)>,
    command: &str,
    shared: bool,
) -> Result<()> {
    if command.is_empty() {
        return Ok(());
    }

    app.push_bash(command, shared);
    app.busy("running");
    screen.render(app)?;

    let overridden = match commands {
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
            app.begin_compaction();
            let compacted = await_work(screen, app, input, agent.compact_now()).await?;
            app.finish_compaction();
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
async fn run_turn(turn: &mut Turn<'_>, prompt: Message) -> Result<()> {
    if let Some(remote) = turn.remote.as_ref() {
        remote.report_running(true);
    }
    let _remote_turn = RemoteTurnGuard(turn.remote.as_ref().map(|remote| remote.outgoing.clone()));
    let (sender, mut receiver) = unbounded_channel::<AgentEvent>();
    let progress = turn.app.settings().terminal_progress;
    report_progress(progress, true);
    let mut agent_turn = Box::pin(turn.agent.run(prompt, &sender));
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut painted = Instant::now() - FRAME;
    let mut aborted = false;

    let mut suggesting: Option<(String, tokio::sync::oneshot::Receiver<Value>)> = None;
    let mut completing: Option<tokio::sync::oneshot::Receiver<Value>> = None;

    loop {
        if painted.elapsed() >= FRAME {
            turn.screen.render(turn.app)?;
            painted = Instant::now();
        }
        if suggesting.is_none() {
            if let (Some(request), Some(asker)) =
                (turn.app.take_pending_suggestion_request(), turn.host_asker)
            {
                suggesting = Some(ask_for_suggestions(asker.clone(), request));
            }
        }
        if completing.is_none() {
            if let (Some(request), Some(asker)) =
                (turn.app.take_pending_completion_request(), turn.host_asker)
            {
                completing = Some(ask_to_apply_completion(asker.clone(), request));
            }
        }

        tokio::select! {
            biased;
            event = turn.input.next() => match event {
                Some(Ok(event)) if offer_component_input(turn.host_asker, turn.app, &event).await => {}
                Some(Ok(event)) if offer_terminal_input(turn.terminal_input, turn.app, &event).await => {}
                Some(Ok(event)) if offer_editor_component_input(turn.host_asker, turn.app, &event).await => {}
                Some(Ok(event)) => match handle(turn.app, event) {
                    Outcome::ExternalEditor => {}
                    Outcome::ThinkingChanged(_) => {}
                    Outcome::CycleModel(_) | Outcome::Suspend => {}
                    Outcome::Quit => {
                        turn.app.should_quit = true;
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
                    turn.app.should_quit = true;
                    aborted = true;
                    break;
                }
            },
            Some(question) = next_question(turn.questions) => {
                turn.app.ask_question(question);

                if turn.app.is_interrupting() {
                    aborted = true;
                    break;
                }
            }
            Some(event) = receiver.recv() => {
                turn.app.apply_event(event);
                while let Ok(next) = receiver.try_recv() {
                    turn.app.apply_event(next);
                }
            }
            action = async { turn.remote.as_mut().unwrap().incoming.recv().await }, if turn.remote.is_some() => {
                match action {
                    Some(action) => match handle_remote(turn.app, action) {
                        Outcome::Interrupt => {
                            aborted = true;
                            break;
                        }
                        Outcome::Quit => {
                            turn.app.should_quit = true;
                            aborted = true;
                            break;
                        }
                        _ => {}
                    },
                    None => *turn.remote = None,
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
                    turn.app.apply_extension_suggestions(&prefix, items);
                }
            }
            answer = async { completing.as_mut().unwrap().await }, if completing.is_some() => {
                completing = None;
                if let Ok(answer) = answer {
                    apply_extension_completion_answer(turn.app, &answer);
                }
            }
            _ = &mut agent_turn => break,
            _ = ticker.tick() => turn.app.tick = turn.app.tick.wrapping_add(1),
        }
    }

    drop(agent_turn);
    while let Ok(event) = receiver.try_recv() {
        turn.app.apply_event(event);
    }
    turn.app.finish_turn(aborted);
    if let Some(commands) = turn.commands.as_mut() {
        let observed = commands.session_observability().await;
        turn.app.set_session_observability(observed);
    }
    report_progress(progress, false);
    Ok(())
}

/// Apply a paired phone's request with the same queueing semantics as the local interface.
fn handle_remote(app: &mut App, action: crate::remote::FromPhone) -> Outcome {
    match action {
        crate::remote::FromPhone::Submit(text) => {
            app.queue_line(text);
            Outcome::Handled
        }
        crate::remote::FromPhone::Steer(text) => {
            app.queue_line(text);
            if app.is_running() {
                app.handle(event::Action::Interrupt)
            } else {
                Outcome::Handled
            }
        }
        crate::remote::FromPhone::FollowUp(text) => app.queue_follow_up_line(text),
        crate::remote::FromPhone::Abort => app.handle(event::Action::Interrupt),
    }
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

/// Bracket an escape that moves the cursor, so the interface keeps the one it was drawn with.
const SAVE_CURSOR: &str = "\x1b7";
const RESTORE_CURSOR: &str = "\x1b8";

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
    /// The images the terminal is already holding, so each one is only sent the once.
    held: HashSet<u32>,
    /// Where the images were put last time, so they are only moved when they have moved.
    shown: Vec<Placement>,
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
    /// Take the terminal and enable the input modes the interface handles.
    fn enter(mode: TuiMode) -> Result<Self> {
        enable_raw_mode()?;
        let result = (|| {
            let anchor = crossterm::cursor::position().ok().map(|(_, row)| row);
            match mode {
                TuiMode::Fullscreen => {
                    execute!(
                        std::io::stdout(),
                        EnterAlternateScreen,
                        EnableBracketedPaste,
                        EnableMouseCapture
                    )?;
                    let mut terminal = Terminal::new(Anchored::new(anchor))?;
                    terminal.clear()?;
                    let size = terminal.size()?;
                    let mut screen = Screen {
                        terminal,
                        mode,
                        rows: 0,
                        anchor,
                        size,
                        held: HashSet::new(),
                        shown: Vec::new(),
                    };
                    screen.measure_cells();
                    Ok(screen)
                }
                TuiMode::Inline => {
                    execute!(std::io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;

                    let terminal = inline_region(Anchored::new(anchor), INLINE_ROWS)?;
                    let size = terminal.size()?;
                    let mut screen = Screen {
                        terminal,
                        mode,
                        rows: 0,
                        anchor: None,
                        size,
                        held: HashSet::new(),
                        shown: Vec::new(),
                    };
                    screen.settle();
                    screen.measure_cells();
                    Ok(screen)
                }
            }
        })();
        if result.is_err() {
            leave();
        }
        result
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
                    EnableMouseCapture
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
                    EnableMouseCapture
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
                    EnableMouseCapture
                )?;
                self.terminal.clear()?;
            }
            TuiMode::Inline => {
                execute!(std::io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;

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

    /// Say that the screen no longer holds the images where they were left, so the next frame
    /// puts them back rather than trusting what is already there.
    fn moved(&mut self) {
        self.shown.clear();
    }

    /// Take the terminal's word for how large one of its cells is.
    ///
    /// This is what a picture's shape is worked out against, so it is asked again whenever the
    /// window changes: the reader may have changed the font size rather than the window.
    fn measure_cells(&mut self) {
        if let Ok(window) = self.terminal.backend_mut().window_size() {
            images::note_cell_size(
                (window.pixels.width, window.pixels.height),
                (window.columns_rows.width, window.columns_rows.height),
            );
        }
    }

    /// Replace the region with one `rows` tall at the kept anchor.
    fn rebuild(&mut self, rows: u16, clear_below: bool) -> Result<()> {
        self.moved();
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
        self.moved();
        let area = self.terminal.get_frame().area();
        let backend = self.terminal.backend_mut();
        for row in area.top()..area.bottom() {
            backend.set_cursor_position(ratatui::layout::Position { x: 0, y: row })?;
            backend.clear_region(ratatui::backend::ClearType::CurrentLine)?;
        }
        backend.flush()?;
        Ok(())
    }

    /// Notice a resize that ratatui handles itself, whose redraw still costs the screen its
    /// pictures and may cost the terminal the images it was holding.
    fn notice_resize(&mut self) -> Result<()> {
        let size = self.terminal.size()?;
        if size != self.size {
            self.size = size;
            self.moved();
            self.held.clear();
            self.measure_cells();
        }
        Ok(())
    }

    /// Follow the terminal through a resize, before ratatui tries to.
    fn fit_screen(&mut self) -> Result<()> {
        let size = self.terminal.size()?;
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        // A terminal may drop the images it holds when its window is resized, so every picture
        // is sent again rather than trusted to have survived.
        self.held.clear();
        self.measure_cells();
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
        self.moved();
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
        } else {
            self.notice_resize()?;
        }
        let vacated = match app.pictures().protocol() {
            Some(capabilities::ImageProtocol::ITerm2) => {
                let shown = self.shown.clone();
                let mut vacated = String::new();
                self.terminal.draw(|frame| {
                    render::draw(frame, app);
                    if app.placements() != shown.as_slice() {
                        vacated = vacated_blanks(&shown, frame.buffer_mut());
                    }
                })?;
                vacated
            }
            _ => {
                self.terminal.draw(|frame| render::draw(frame, app))?;
                String::new()
            }
        };
        self.show_pictures(app, &vacated);
        if let Some(title) = app.take_title_change() {
            set_terminal_title(&title);
        }
        Ok(())
    }

    /// Put this frame's images on the screen.
    ///
    /// An image is not a cell and cannot travel in the frame's buffer: the escape that draws one
    /// carries the whole picture and leaves the cursor somewhere the buffer does not expect. So the
    /// images go on afterwards, over the blank rows the transcript held open for them, and the
    /// cursor is put back where the frame left it.
    ///
    /// Every frame that moves an image first takes the last frame's images off the screen, so a
    /// picture is never left behind over rows that have since been redrawn. What the terminal holds
    /// survives that, so each picture is sent once however often it is drawn.
    fn show_pictures(&mut self, app: &App, vacated: &str) {
        let Some(protocol) = app.pictures().protocol() else {
            return;
        };

        if app.placements() == self.shown {
            return;
        }
        let escapes = picture_escapes(
            protocol,
            vacated,
            &self.shown,
            app.placements(),
            app.pictures(),
            &mut self.held,
        );

        use std::io::Write as _;
        let mut stdout = std::io::stdout();
        if stdout
            .write_all(escapes.as_bytes())
            .and_then(|()| stdout.flush())
            .is_ok()
        {
            self.shown = app.placements().to_vec();
        }
    }
}

/// The escapes that take the images in `shown` off the screen and put the ones in `wanted` on,
/// sending any picture the terminal is not holding yet and adding it to `held`.
///
/// A terminal whose images are cell content has nothing to name a picture by once it is drawn, so
/// the cells its pictures vacated are handed in as `vacated` and painted over instead.
fn picture_escapes(
    protocol: capabilities::ImageProtocol,
    vacated: &str,
    shown: &[Placement],
    wanted: &[Placement],
    pictures: &render::pictures::Pictures,
    held: &mut HashSet<u32>,
) -> String {
    let mut out = String::from(SAVE_CURSOR);
    out.push_str(vacated);

    let mut taken_off = HashSet::new();
    for placement in shown {
        if taken_off.insert(placement.id) {
            out.push_str(&images::remove(protocol, placement.id));
        }
    }
    for placement in wanted {
        let Some(data) = pictures.data(placement.id) else {
            continue;
        };
        if held.insert(placement.id) {
            out.push_str(&images::transmit(protocol, data, placement.id));
        }

        out.push_str(&format!(
            "\x1b[{};{}H",
            placement.row + 1,
            placement.column + 1
        ));
        out.push_str(&images::place(
            protocol,
            data,
            placement.id,
            placement.columns,
            placement.rows,
            placement.band,
        ));
    }
    out.push_str(RESTORE_CURSOR);
    out
}

/// The cells iTerm2 images were drawn over, blanked wherever this frame still leaves them blank.
///
/// An iTerm2 image is cell content with no handle to take it off the screen by, so a picture that
/// moved or went away would stay painted: to ratatui the cells it held are spaces, and it repaints
/// only what changed. A row the transcript has since written words over repaints itself, so only
/// the rows the frame left blank are painted over here.
fn vacated_blanks(shown: &[Placement], drawn: &ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for placement in shown {
        for row in 0..placement.rows {
            let y = placement.row as usize + row;
            let Ok(y16) = u16::try_from(y) else {
                continue;
            };
            let blank = (0..placement.columns).all(|column| {
                let x = placement.column as usize + column;
                let Ok(x16) = u16::try_from(x) else {
                    return true;
                };
                drawn
                    .cell(ratatui::layout::Position { x: x16, y: y16 })
                    .is_none_or(|cell| cell.symbol() == " ")
            });
            if blank {
                out.push_str(&format!(
                    "\x1b[{};{}H{}",
                    y + 1,
                    placement.column + 1,
                    " ".repeat(placement.columns)
                ));
            }
        }
    }
    out
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

    use crate::capabilities::ImageProtocol;
    use crate::render::pictures::Pictures;

    #[test]
    fn an_editor_buffer_has_a_private_temporary_directory() {
        let directory = secure_temp_directory().expect("temporary directory");
        let metadata = std::fs::metadata(&directory).expect("directory metadata");
        assert!(metadata.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(metadata.permissions().mode() & 0o077, 0);
        }
        std::fs::remove_dir_all(directory).expect("remove temporary directory");
    }

    #[test]
    fn phone_actions_use_the_local_queue_and_interrupt_rules() {
        let mut app = App::new(&[], TuiOptions::default());
        assert_eq!(
            handle_remote(&mut app, crate::remote::FromPhone::Submit("first".into())),
            Outcome::Handled
        );
        assert_eq!(app.take_submission().as_deref(), Some("first"));

        app.begin_turn("active");
        assert_eq!(
            handle_remote(&mut app, crate::remote::FromPhone::Steer("urgent".into())),
            Outcome::Interrupt
        );
        assert_eq!(app.take_submission().as_deref(), Some("urgent"));
    }

    /// A picture, and where the frame put it.
    fn pictured(data: &str, row: u16) -> (Pictures, Vec<Placement>) {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(data, 40, row as usize).expect("reserved");
        let placements = pictures.placements(ratatui::layout::Rect::new(0, 0, 40, 20), 0, 0);
        (pictures, placements)
    }

    /// A base64 PNG header describing an image of the given size.
    fn png(width: u32, height: u32) -> String {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend([0, 0, 0, 13]);
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        crate::typeset::base64(&bytes)
    }

    /// The interface is drawn from the cursor, so the images must hand it back untouched.
    #[test]
    fn drawing_the_images_leaves_the_cursor_where_the_frame_left_it() {
        let (pictures, placements) = pictured(&png(90, 18), 3);
        let escapes = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &[],
            &placements,
            &pictures,
            &mut HashSet::new(),
        );

        assert!(escapes.starts_with(SAVE_CURSOR));
        assert!(escapes.ends_with(RESTORE_CURSOR));

        assert!(escapes.contains("C=1"));
    }

    /// The rows an image sits on are redrawn as the conversation moves, so a picture comes off the
    /// screen before it goes back on, and is never left behind over what replaced it.
    #[test]
    fn a_picture_comes_off_the_screen_before_it_goes_back_on() {
        let (pictures, placements) = pictured(&png(90, 18), 3);
        let moved = pictures.placements(ratatui::layout::Rect::new(0, 0, 40, 20), 0, 2);
        let escapes = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &placements,
            &moved,
            &pictures,
            &mut HashSet::new(),
        );

        let id = placements[0].id;
        let off = escapes
            .find(&format!("a=d,d=i,i={id}"))
            .expect("taken off the screen");
        let on = escapes.find("a=p").expect("and then put back");
        assert!(off < on, "off before on");
    }

    /// Whatever the terminal was showing before the session started is not ours to take away.
    #[test]
    fn the_first_frame_takes_nothing_off_the_screen() {
        let (pictures, placements) = pictured(&png(90, 18), 3);
        let escapes = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &[],
            &placements,
            &pictures,
            &mut HashSet::new(),
        );

        assert!(!escapes.contains("a=d"), "nothing was ours yet");
    }

    /// A picture crosses the wire once. Redrawing it afterwards costs a handful of bytes, not the
    /// whole image, which is what kept the interface from answering the keyboard.
    #[test]
    fn a_picture_is_only_sent_to_the_terminal_once() {
        let image = png(90, 18);
        let (pictures, placements) = pictured(&image, 3);
        let mut held = HashSet::new();

        let first = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &[],
            &placements,
            &pictures,
            &mut held,
        );
        assert!(first.contains(&image), "the picture itself goes once");

        let again = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &placements,
            &placements,
            &pictures,
            &mut held,
        );
        assert!(!again.contains(&image), "and never again");
        assert!(again.contains("a=p"), "it is still drawn");
        assert!(again.len() < 120, "{} bytes to redraw", again.len());
    }

    /// The escape carries the row and column the transcript left for the picture, counted from one
    /// the way a terminal counts.
    #[test]
    fn a_picture_is_drawn_on_the_row_the_transcript_held_for_it() {
        let (pictures, placements) = pictured(&png(90, 18), 3);
        let escapes = picture_escapes(
            ImageProtocol::Kitty,
            "",
            &[],
            &placements,
            &pictures,
            &mut HashSet::new(),
        );

        assert!(escapes.contains("\x1b[4;1H"), "{escapes:?}");
    }

    /// An iTerm2 image is cell content that nothing can name once it is drawn: when its placement
    /// moves, the cells the frame left blank are painted over, while a row the transcript has
    /// written words over is left to the words.
    #[test]
    fn an_iterm_picture_that_moves_has_its_blank_rows_painted_over() {
        let mut pictures = Pictures::new(Some(ImageProtocol::ITerm2));
        pictures
            .reserve_sized(&png(90, 36), (10, 2), 40, 3)
            .expect("reserved");
        let shown = pictures.placements(ratatui::layout::Rect::new(0, 0, 40, 20), 0, 0);
        assert_eq!(shown[0].rows, 2);

        // The frame after a one-line scroll: words landed on the picture's first row, and its
        // second row is still blank.
        let mut drawn = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 20));
        drawn.set_string(
            0,
            3,
            "words the transcript wrote",
            ratatui::style::Style::default(),
        );

        let vacated = vacated_blanks(&shown, &drawn);
        assert!(
            !vacated.contains("\x1b[4;1H"),
            "the written row keeps its words: {vacated:?}"
        );
        assert!(
            vacated.contains("\x1b[5;1H"),
            "the blank row is painted over: {vacated:?}"
        );
    }

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
            held: HashSet::new(),
            shown: Vec::new(),
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

    /// A terminal may drop the images it holds when its window is resized: what looked stored is
    /// gone, and placing a picture by number alone draws nothing. The frame after a resize must
    /// send every picture again, not merely place it.
    #[test]
    fn a_resize_forgets_what_the_terminal_held_so_pictures_are_sent_again() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let mut screen = inline_screen(backend, 4, 8);
        screen.held.insert(7);
        screen.shown.push(Placement {
            id: 7,
            column: 0,
            row: 0,
            columns: 4,
            rows: 2,
            band: None,
        });

        screen.terminal.backend_mut().inner.resize(10, 6);
        screen.fit_screen().unwrap();

        assert!(screen.held.is_empty(), "the pictures are sent afresh");
        assert!(screen.shown.is_empty(), "and placed afresh");
    }

    /// A fullscreen resize is ratatui's to redraw, but the pictures are still the screen's: the
    /// redraw wipes them, and the terminal may have dropped what it held, so both are forgotten
    /// and the next frame sends and places them again.
    #[test]
    fn a_fullscreen_resize_forgets_the_pictures_so_they_are_drawn_again() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let terminal = Terminal::new(Anchored::over(backend, None)).unwrap();
        let size = terminal.size().unwrap();
        let mut screen = Screen {
            terminal,
            mode: TuiMode::Fullscreen,
            rows: 0,
            anchor: None,
            size,
            held: HashSet::new(),
            shown: Vec::new(),
        };
        screen.held.insert(7);
        screen.shown.push(Placement {
            id: 7,
            column: 0,
            row: 0,
            columns: 4,
            rows: 2,
            band: None,
        });

        screen.notice_resize().unwrap();
        assert_eq!(screen.held.len(), 1, "the same size forgets nothing");

        screen.terminal.backend_mut().inner.resize(10, 6);
        screen.notice_resize().unwrap();

        assert!(screen.held.is_empty(), "the pictures are sent afresh");
        assert!(screen.shown.is_empty(), "and placed afresh");
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
