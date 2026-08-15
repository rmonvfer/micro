//! The interactive terminal interface.
//!
//! [`run`] takes an agent and whatever conversation it should open with, draws inline at
//! wherever the shell left the cursor, and returns the whole conversation when the user
//! leaves.
//!
//! Nothing here takes the terminal away from you. The interface occupies an inline region
//! that is only as tall as it needs to be, and a message it has finished with is printed
//! above that region into the terminal's own scrollback — so your shell history stays where
//! it was, and your terminal's search, selection and wheel reach the conversation the same
//! way they reach any other command's output.
//!
//! The `history` argument is what the scrollback opens on. It is not given to the agent: an
//! agent resuming a session already carries that history, and the caller seeds it there with
//! `Agent::with_history`. The conversation that comes back is the agent's own, which is the
//! only complete account of the run — it holds the history, the turns just taken, and any
//! summary compaction put in place of an older stretch.
//!
//! ```no_run
//! # async fn example(agent: micro_agent::Agent) -> anyhow::Result<()> {
//! let conversation = micro_tui::run(agent, Vec::new()).await?;
//! # let _ = conversation;
//! # Ok(())
//! # }
//! ```

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

// The pieces the interface is assembled from. They hold no terminal state, so a caller that
// wants to show a conversation its own way can reuse them.
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

/// Shortest gap between repaints. Streamed tokens arrive faster than a terminal can usefully
/// redraw, so frames are coalesced instead of drawn one per delta.
const FRAME: Duration = Duration::from_millis(33);
/// How often the spinner advances and a running turn is repainted.
const TICK: Duration = Duration::from_millis(80);

/// Run the interface with default options.
///
/// `history` is what the scrollback opens on; the agent is expected to already carry it.
pub async fn run(agent: Agent, history: Vec<Message>) -> Result<Vec<Message>> {
    run_with(agent, history, TuiOptions::default()).await
}

/// Run the interface, returning the agent's conversation as it stands when the user leaves.
///
/// `history` seeds the scrollback only. The agent keeps the conversation of record — it may
/// already hold this same history, and compaction may have replaced part of it with a
/// summary — so what comes back is the agent's, never the two concatenated.
pub async fn run_with(
    mut agent: Agent,
    history: Vec<Message>,
    mut options: TuiOptions,
) -> Result<Vec<Message>> {
    install_panic_hook();
    let mut screen = Screen::enter(options.tui_mode)?;
    // Asked for here, between taking the terminal and drawing on it: the reply has to be
    // read before the event stream exists, and the answer has to be known before the first
    // frame, or that frame paints the wrong theme and then corrects itself.
    // Asked here, while the event stream has not been built and nothing else is reading the
    // terminal. Whatever it answers is kept, so `/theme auto` later costs no round trip.
    background::prime();
    options.theme = Some(options.theme.unwrap_or_else(background::detect_theme));
    let mode = options.tui_mode;
    let exit_output = options.settings.exit_output;
    let mut said = Vec::new();
    let result = drive(&mut screen, &mut agent, &history, options, &mut said).await;
    leave();
    // The alternate screen takes the conversation with it when it goes. Written out again
    // here, after the terminal is back, it stays where a reader left it — which is what
    // drawing inline gives for nothing and what a full screen has to be asked for.
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
///
/// The conversation is taken from the interface rather than from the agent because it is
/// what was drawn that a reader is being given back: the tool calls folded as they were
/// folded, the answers wrapped as they were wrapped.
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
    // Set while a list of models is open and the providers are being asked what they serve.
    let mut refreshing: Option<tokio::sync::oneshot::Receiver<Listings>> = None;
    // Set while an extension's `getSuggestions` or `applyCompletion` is in flight — see
    // `next_event`, which starts and races these the same way it does `refreshing`.
    let mut suggesting: Option<(String, tokio::sync::oneshot::Receiver<Value>)> = None;
    let mut completing: Option<tokio::sync::oneshot::Receiver<Value>> = None;

    loop {
        screen.render(app)?;
        if app.should_quit {
            return Ok(());
        }

        // An extension let go while the last pass was running took its tools with it. Done
        // here, between turns, because this is where the agent is this loop's to change and
        // the next turn has not yet asked what tools there are.
        let retired = app.take_retired_tools();
        if !retired.is_empty() {
            agent.remove_tools(&retired);
        }

        // A credential finishes collecting the moment the user presses enter on it.
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

        // A question waiting while nothing else is happening is shown now, so an extension
        // that asks between turns is not left until the next keystroke.
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
                // Work finished behind the interface rather than something the user did:
                // the frame is drawn again at the top of the loop and nothing else changes.
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
                        // Cycling is `/model` with a direction, so it goes the same way every
                        // other model change does rather than reaching for the catalog here.
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
                // The terminal went away; there is nothing left to read.
                Next::Ended => return Ok(()),
            },
        }
    }
}

/// Shell integration: the markers a terminal uses to tell a prompt from its output.
///
/// With these a terminal can jump between prompts, fold output, and mark where a command
/// began — the same affordances it gives a shell. They are zero-width and move no cursor,
/// so they are written straight to the terminal rather than through the cell grid, which
/// has nowhere to put a character that occupies no column.
mod osc133 {
    /// A prompt begins here.
    pub const PROMPT: &str = "\x1b]133;A\x07";
    /// The prompt ends and what was typed begins.
    pub const INPUT: &str = "\x1b]133;B\x07";
    /// The command was accepted; its output follows.
    pub const OUTPUT: &str = "\x1b]133;C\x07";
}

/// Progress, as a terminal that shows it in the tab or the dock expects to be told.
///
/// OSC 9;4 takes a state and a percentage. Indeterminate is what an answer is: there is
/// no way to know how much of it is left until it arrives.
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

/// Set the terminal's window/tab title. OSC 0 is the one every terminal still honors,
/// including the ones that dropped OSC 2's narrower "just the tab" form.
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
///
/// The terminal has to be handed back before stopping, or the shell inherits raw mode and
/// the user's next command types into nothing.
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
///
/// Waiting on both at once is what lets a list of models be drawn from what is known and
/// then corrected in place, rather than either blocking on the network or never hearing
/// back from it.
/// What the loop got back from waiting.
enum Next {
    /// Something the user did.
    Event(Event),
    /// Something finished behind the interface. Draw again and carry on waiting.
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
    // Nothing in flight, and a list open that wants asking about: start now.
    if refreshing.is_none() && app.picker_mut().is_some_and(|open| open.refreshes()) {
        if let Some(commands) = commands.as_deref_mut() {
            *refreshing = commands.begin_model_refresh();
        }
    }
    // Same story for an extension's own completion menu: `sync_menu` raised a question the
    // moment the reader's cursor landed in a trigger-prefixed word, and this is the first
    // place since with anywhere to send it — started here, off the render path, rather than
    // holding up the keystroke that raised it.
    if suggesting.is_none() {
        if let (Some(request), Some(asker)) = (app.take_pending_suggestion_request(), host_asker) {
            *suggesting = Some(ask_for_suggestions(asker.clone(), request));
        }
    }
    // And for committing one of that menu's items: `App::handle` queued the question the
    // moment the reader pressed enter or tab on it, in place of writing a fixed splice the
    // way it would for a built-in menu.
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
                // Only a list still open is worth correcting; one closed in the meantime is
                // not brought back.
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
            // Taken whether or not the extension host is still there to have answered:
            // either way this exact question is settled and nothing should ask it again.
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

/// Start the `getSuggestions` question `sync_menu` raised, off the render path. The prefix
/// travels with the receiver rather than being re-read from `app` once the answer lands,
/// since the menu it was asked about may by then be a different one, or none — the same
/// staleness [`crate::menu::Menu::set_extension_items`] checks either way.
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

/// Start the `applyCompletion` question committing an extension's menu item raised, off the
/// render path.
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
/// Shaped wrong — an extension that raised instead of returning, say — changes nothing
/// rather than writing a broken edit.
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
///
/// The terminal has to be given up entirely while another program owns it — raw mode off,
/// bracketed paste off — and taken back afterwards, or the editor and the interface fight
/// over the same input. Anything that goes wrong is reported into the transcript rather
/// than ending the session: a missing `$EDITOR` is not a reason to lose a conversation.
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
///
/// Whether it is a command comes from dispatching it, never from the leading slash, so a
/// prompt that happens to start with a path still reaches the model.
/// Everything a turn needs besides the line: the terminal, the state, the agent, and the
/// two places something can arrive from while it runs.
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
    // What was typed is offered to whoever runs commands before anything is done with it:
    // it may come back changed, or not come back at all.
    let line = match commands {
        Some(ref mut commands) => match commands.submitted(line).await {
            Some(line) => line,
            None => return Ok(()),
        },
        None => line,
    };

    // `!` runs a command here instead of asking the model to run one. Its output joins the
    // conversation, so the model knows what the user just did. `!!` runs it the same way
    // and tells nobody: for a command whose answer is for the user, and which would only
    // take up room in what the model is reading.
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
        // Interrupted before it answered: nothing ran, so there is nothing to report.
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
        // A prompt written for the purpose becomes the turn, in place of the line that
        // asked for it.
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
///
/// The model is not asked anything, but it is told: the command and its output are recorded
/// so the next turn knows what the user just did.
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

    // Whatever is listening is asked first, and may decide what running this amounted to
    // itself — `user_bash` is a moment the shell answers to, not one it is merely told
    // about.
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

    // Tagged rather than pasted in raw, so the model can tell the user's own command from
    // anything it ran itself. A command kept back is never recorded, so it is absent from
    // the next turn and from the session on disk — not merely hidden.
    if shared {
        agent.record(Message::user(format!(
            "<bash command=\"{command}\">\n{output}\n</bash>"
        )));
    }
    Ok(())
}

/// Carry out one command outcome, drawing whatever the interface owns and handing the rest
/// to the host.
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
        CommandOutcome::Inspect { title, text } => app.open_inspection(title, text),
        CommandOutcome::Quit => app.should_quit = true,
        CommandOutcome::Choose(picker) => {
            // A list of models is drawn from what is already known and asked about at the
            // same time, so it is there the moment it was asked for and right shortly after.
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

        // Where to go and what to type has to be on screen before the waiting starts: the
        // code expires with the wait, so showing it afterwards shows nothing useful.
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

        // Both of these belong to the interface alone. Reasoning effort rides on the model
        // the agent already holds, and a palette never leaves this crate, so neither is
        // worth a round trip through the host.
        CommandOutcome::SetThinking { level } => {
            agent.set_thinking(level);
            app.set_thinking(level);
            commands.thinking_changed(level).await;
        }
        // Both of these are the interface's own: it holds the conversation being copied or
        // written out, and neither needs anything the host has.
        CommandOutcome::CopyLastAnswer => app.copy_last_answer(),
        CommandOutcome::Export { path } => app.export(path.as_deref()),

        // Compacting is the agent's own work: it holds the conversation and the model that
        // summarizes it, so nothing about it needs the host.
        CommandOutcome::Compact if !commands.compacting().await => {
            app.notice("An extension stopped the compaction", MessageKind::Error);
        }
        CommandOutcome::Compact => {
            app.busy("compacting");
            let compacted = await_work(screen, app, input, agent.compact_now()).await?;
            app.idle();
            match compacted {
                // The summary is the first message of the conversation it left behind, and
                // renders as a compaction marker, so rebuilding from the agent shows it.
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
///
/// Two outcomes need the agent, which the interface holds and the host does not. A model
/// swap is resolved by the host, because it holds the catalog and the credentials, and
/// applied here. A replaced conversation is decided by the host, because it holds the
/// session log, and has to reach the agent as well as the screen: a branch the model
/// cannot see is not a branch. Everything else is the interface's own business.
fn apply_applied(app: &mut App, agent: &mut Agent, applied: Applied) {
    match applied {
        Applied::Conversation { messages, note } => {
            // A resumed session or a fork is not the conversation the terminal was handed,
            // so what it was given is forgotten rather than carried across.
            app.forget_scrolled_out();
            agent.set_messages(messages.clone());
            app.apply_result(Applied::Conversation { messages, note });
        }
        // What the model is told before the conversation is the host's to change, and it
        // has already asked the agent for it: the prompt reaches the next turn hashed and
        // recorded, rather than being installed from here without either.
        Applied::SystemPrompt { note, .. } => {
            // Reloading re-reads the workspace, so the file listing behind `@` is read
            // again too rather than describing the workspace as it was at startup.
            app.forget_workspace_files();
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        Applied::Model { swap, note } => {
            // Everything the footer says about the model changes with it: the name, how
            // much room there is, which is what the context share is measured against, and
            // what a token costs, which is what the running total is worked out from.
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
///
/// `None` means the user gave up on it. The work is abandoned rather than cancelled, since
/// only the host knows how to stop what it started.
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
                    // Not while something is running: handing the terminal away mid-answer
                    // would leave the stream painting into another program's screen.
                    Outcome::ExternalEditor => {}
                    // The level applies from the next turn; this one is already in flight.
                    Outcome::ThinkingChanged(_) => {}
                    // Neither is safe mid-answer: swapping the model would change what is
                    // replying to itself, and suspending would leave the stream painting
                    // into a terminal this process no longer holds.
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
    // The phone shows a stop button in place of a send button while a turn runs, and
    // refuses to start a second turn inside this one, so it is told at both ends.
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
    // The reader can still type — and still trigger an extension's own completion menu —
    // while a turn runs, so the same two in-flight slots `next_event` races for `run_loop`
    // are raced here too.
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

        // Input is polled first so a key press is never starved by a fast stream, and the
        // agent's events are drained before its future is polled so nothing is lost when
        // the turn finishes with events still queued.
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
                // An extension's `ctx.abort()` reaches this far as an "abort" question,
                // answered by `App::ask_question` calling the same `interrupt()` a Ctrl+C
                // keypress does. Answering it already set `turn.interrupting`; stopping
                // the turn because of it is this loop's job, the same as it is for a key.
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

    // Dropping the future abandons the turn; anything it already reported still belongs in
    // the transcript.
    drop(turn);
    while let Ok(event) = receiver.try_recv() {
        app.apply_event(event);
    }
    app.finish_turn(aborted);
    if let Some(commands) = commands.as_mut() {
        app.set_session_cost(commands.session_cost().await);
    }
    report_progress(progress, false);
    Ok(())
}

/// The next question from an extension. With nothing able to ask, this never resolves,
/// which leaves the arm inert rather than closing the select over it.
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
///
/// Only keys the interface itself ignored are offered, so an extension can never take a
/// key out from under the editor.
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

/// Offer a key to `ctx.ui.onTerminalInput` before the interface does anything with it
/// itself. `true` means an extension consumed it: nothing built in sees this one, not even
/// a registered shortcut.
///
/// Every key is checked for whether anything is listening before anything is sent —
/// `app.wants_terminal_input()` reads a `bool` already held in memory — so a session with
/// no extension asked for this pays nothing beyond that read for every key it does not
/// consume.
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

/// Hand a key to the component a `custom()` overlay has open, and redraw it with what it
/// looked like afterward. `true` for every key while such an overlay is open except escape
/// and interrupt, which close it locally instead — see [`App::handle_component_overlay`] —
/// so those two fall through and are handled the ordinary way.
///
/// Checked before [`offer_terminal_input`]: an overlay has the keyboard while it is up, the
/// same as a picker or a credential prompt already do, and a key an extension is merely
/// listening for is not what decides whether the overlay in front of it sees this one.
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

/// Offer a key to the component `setEditorComponent` replaced the built-in editor with,
/// before the built-in editor sees it. `true` only when the component says it consumed the
/// key — anything it declines falls through to the built-in editor, the same "handle what
/// you want, call through for the rest" contract pi's own `CustomEditor` documents, which is
/// why this checks the answer's `consume` field rather than capturing every key outright
/// the way [`offer_component_input`] does for a `custom()` overlay.
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
    // The buffer this key is about to fall through to if the component does not consume it
    // — carried alongside so the component can read what its own `handleInput` cannot see
    // any other way, the same buffer pi's `CustomEditor` sees for free by inheriting it.
    // `custom()`'s overlay has no such relationship to a buffer, so only this ask carries it.
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
///
/// The region is only as tall as the interface needs. When that changes it is rebuilt at the
/// new height from the same row, which is what lets the interface grow downward from where
/// the shell left the cursor instead of taking the screen away.
/// How much of the terminal the interface takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiMode {
    /// A region at the cursor, as tall as the interface needs. What the conversation has
    /// finished with goes into the terminal's own scrollback, so the shell's history stays
    /// where it was and the terminal's search and selection reach the conversation.
    Inline,
    /// The whole screen, which leaves the scrollback untouched and scrolls internally.
    #[default]
    Fullscreen,
}

/// How tall the inline region starts. It grows to whatever the interface needs on the
/// first frame; this is only what is reserved before there is anything to measure.
const INLINE_ROWS: u16 = 8;

/// Alternate scroll, which is how the wheel reaches a program that has not claimed the
/// mouse: with the alternate screen up, the terminal turns a wheel tick into a cursor key.
///
/// Terminals differ on whether it is on to begin with — Ghostty starts with it, iTerm2
/// waits to be asked — so it is asked for rather than hoped for, and given back on the
/// way out because it is the terminal's setting and not micro's.
const ENABLE_ALTERNATE_SCROLL: &str = "\x1b[?1007h";
const DISABLE_ALTERNATE_SCROLL: &str = "\x1b[?1007l";

/// The terminal, with the one question micro can answer itself answered here.
///
/// ratatui anchors an inline region to the cursor, and asks the terminal where that is.
/// crossterm cannot answer while its event stream is running: the stream's reader holds
/// the terminal until a key arrives, so the question waits for a keystroke and then
/// reports that it timed out. Once a region exists its top row is not a mystery — it is
/// where the region already starts — so it is answered from here.
///
/// Answering it here is also the more accurate answer. A real reading returns wherever the
/// last frame left the cursor, which is a row *inside* the region, so a region rebuilt
/// from it starts partway down the one it replaces and the interface walks down the screen.
///
/// An answer given rather than read has to be made true before it is given. ratatui does not
/// only record the row it is told — it reserves a region's rows by printing that many newlines,
/// and newlines come out wherever the cursor actually is. Told one row while the cursor sits on
/// another, it reserves the rows from the second and counts the scrolling from the first, so the
/// region it goes on to draw into is not the one the terminal made room for.
struct Anchored<B> {
    inner: B,
    /// The row to anchor the next inline region at. Unset before there is a region, when
    /// only the terminal knows where the shell left the cursor.
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
        // The cursor is put on the row that is about to be reported, so that whatever the
        // caller does from there — reserving a region's rows, most of all — happens on the row
        // it was told about rather than on the one the last frame happened to end at.
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

/// A backend equivalent to this one, for a rebuilt `Terminal` to draw through.
///
/// ratatui cannot change an inline region's height in place, so a new height means a new
/// `Terminal` — and a `Terminal` will not give its backend back, so the replacement needs
/// one of its own that reaches the same screen.
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
    /// A copy carries the whole simulated screen — cells, cursor and scrollback — so the
    /// replacement continues from the state the original reached.
    fn fresh(&self) -> Self {
        self.clone()
    }
}

struct Screen<B: ratatui::backend::Backend = CrosstermBackend<Stdout>> {
    terminal: Terminal<Anchored<B>>,
    mode: TuiMode,
    /// How tall the inline region is now, so a change in what the interface needs can be
    /// noticed and the region resized to match.
    rows: u16,
    /// The row an inline region starts at. Read from the terminal once, while nothing else
    /// is reading it, and kept in step from then on — [`Anchored`] says why it is never
    /// read again. Unset when the terminal would not say, which leaves ratatui to ask.
    anchor: Option<u16>,
    /// The terminal's size when the region was last built, so a resize can be noticed
    /// before ratatui notices it: its own answer to a resized inline region moves the
    /// region by the cursor's offset inside it, which is not where the region is.
    size: ratatui::layout::Size,
}

/// Build an inline region `rows` tall at the backend's anchor.
///
/// A region with less room below its anchor than it needs scrolls the terminal to make room,
/// which moves the row it starts at. The row it landed on is written back onto the backend,
/// whose answer the *next* region's rows are reserved from: a backend still holding the row
/// this one was asked for would reserve them from a row the region no longer starts at.
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
    ///
    /// A terminal that is reporting the mouse hands drags to the program instead of
    /// selecting text with them, and selecting text is worth more than a scroll wheel that
    /// keys already cover.
    ///
    /// Whether the whole screen is taken or only a region of it is the caller's choice:
    /// see [`TuiMode`]. Taking the whole screen leaves the shell's scrollback exactly as
    /// it was; drawing inline puts the conversation into it, where the terminal's own
    /// search and selection reach it.
    fn enter(mode: TuiMode) -> Result<Self> {
        enable_raw_mode()?;
        // The one moment the terminal can be asked where the cursor is: raw mode is on and
        // nothing else is reading input yet. A full screen is asked before the alternate
        // one is entered, because the row it answers with is the row to come back to.
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
                // The region is only as tall as the interface needs; what leaves it is
                // handed to the terminal's own scrollback rather than being redrawn.
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

    /// Switch between the terminal's scrollback and an alternate screen without ending the
    /// session. The terminal remains in raw mode throughout, so keyboard input stays live.
    fn set_mode(&mut self, mode: TuiMode) -> Result<()> {
        if mode == self.mode {
            return Ok(());
        }

        match mode {
            TuiMode::Fullscreen => {
                // Where the region stood, which is where leaving the alternate screen puts
                // the cursor back and so where a region built afterwards belongs.
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
                // The normal screen comes back showing whatever the region drew before the
                // alternate screen went up, and the region being built now may be shorter
                // than that was — rebuilding clears below the anchor so nothing of it is
                // left standing.
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
                // The program that had the terminal drew whatever it drew, wherever it
                // pleased, and left the cursor at the end of it — the row a region built
                // now belongs at, exactly as at [`Screen::enter`]. It can be asked for the
                // same reason it could then: the loan suspends the event stream, so nothing
                // else is reading the answer. When the terminal will not say, the row the
                // region last stood at is the only answer left.
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
    /// Record where the region actually is, which after any rebuild is the only version
    /// of the truth: ratatui clamps a region taller than the screen and moves one that had
    /// to scroll, so neither the height asked for nor the row asked for can be trusted.
    fn settle(&mut self) {
        let area = self.terminal.get_frame().area();
        self.rows = area.height;
        self.anchor = Some(area.y);
    }

    /// Replace the region with one `rows` tall at the kept anchor.
    ///
    /// ratatui has no call that changes an inline viewport's height in place —
    /// `Terminal::resize` only ever re-centers the height it was already given — so a new
    /// height means a new `Terminal`, drawing through a fresh backend on the same screen.
    ///
    /// A fresh `Terminal` starts with an empty record of what is on the screen, and a frame
    /// drawn against that record writes only the cells that are not blank — a blank cell is
    /// already what the record claims is there. Whatever the screen shows where the new
    /// frame is blank therefore survives, which is how a filtered menu ends up woven
    /// through the text of the one it replaced. Every rebuild must paint onto rows that
    /// really are blank; `clear_below` blanks everything from the anchor to the foot of the
    /// screen, and the one caller that passes `false` has already blanked what it needed.
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
    ///
    /// ratatui's own answer to a resized inline region re-anchors it to the cursor's
    /// offset within the old one, which is a row *inside* the region — the region jumps up
    /// by most of its own height and the redraw eats the conversation above it. Noticing
    /// the resize first and rebuilding at the kept row, pulled up only as far as the new
    /// screen requires, keeps the region where the reader last saw it.
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
        // The terminal rewrapped or clipped what it was showing, so what stands below the
        // anchor is not anything the rebuilt region can account for.
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
        // Lines going in above the region push it down the screen, or scroll the screen
        // out from under it once it is at the foot. Either way it starts at a different
        // row than it did, and that row is what the next region is built from.
        self.anchor = Some(self.terminal.get_frame().area().y);
        self.terminal.backend_mut().anchor = self.anchor;
        Ok(())
    }

    /// Make the region as tall as the interface has turned out to be.
    ///
    /// A region fixed at the height it started with is either too small for a menu or too
    /// tall for a bare prompt, so it follows what is drawn. The old region's rows have to
    /// be blanked whichever way the height moved — [`Screen::rebuild`] says why — and a
    /// shrinking region has given some of those rows up: they are below the new region,
    /// where nothing will ever draw again, so what the old frame left there would stand
    /// forever. They are blanked with the rest of the footprint; `clear_on_shrink` widens
    /// that to the foot of the screen for whoever prefers everything below swept.
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

    /// Bring the interface up to date: follow a resize, hand whatever has left the live
    /// region to the terminal, size the region to what is left, and paint it.
    fn render(&mut self, app: &mut App) -> Result<()> {
        if self.mode == TuiMode::Inline {
            self.fit_screen()?;
            // Whatever the conversation has finished with leaves the live region and
            // becomes part of the terminal's own scrollback, where its search and
            // selection reach it.
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
    // Kitty holds an image until it is told to let go, so a session that drew any frees
    // them rather than leaving them in the terminal's memory.
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

    /// Waiting answers three different ways, and two of them are not the end of the
    /// session. Work finishing behind an open list once read as the input stream closing,
    /// which quit micro the moment the catalogs came back.
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

    /// The row [`Anchored`] reports is the row the cursor is left on. ratatui reserves an
    /// inline region's rows by printing newlines, which come out at the cursor rather than at
    /// the row it was told about, so an answer that is not made true reserves the rows
    /// somewhere other than where it then says the region is.
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

    /// A region rebuilt while the cursor sits at the foot of the one it replaces — which is
    /// where drawing a frame leaves it — reserves its rows from the row it starts at. Reserving
    /// them from the cursor instead scrolls the screen by rows ratatui does not count, and the
    /// region it records is then not the region the terminal made room for.
    #[test]
    fn a_rebuilt_region_is_where_it_says_it_is() {
        use ratatui::backend::Backend as _;

        let screen = [
            "row0", "row1", "row2", "row3", "row4", "row5", "row6", "row7", "row8", "row9",
        ];
        let mut backend =
            Anchored::over(ratatui::backend::TestBackend::with_lines(screen), Some(2));
        // Where a drawn frame leaves the cursor: the foot of the region, not its top.
        backend
            .set_cursor_position(ratatui::layout::Position { x: 3, y: 9 })
            .unwrap();

        let mut terminal = inline_region(backend, 8).unwrap();

        // Eight rows anchored at row two reach exactly the foot of a ten-row screen, so there
        // was room to reserve them and nothing moved.
        assert_eq!(
            terminal.get_frame().area(),
            ratatui::layout::Rect::new(0, 2, 4, 8)
        );
        assert_eq!(terminal.backend().anchor, Some(2));
        terminal.backend().inner.assert_buffer_lines(screen);
    }

    /// Where a region does not fit below its anchor the terminal scrolls to make room, and the
    /// row it lands on is the row both the [`Screen`] and the backend go on to build from.
    #[test]
    fn a_region_that_scrolled_reports_where_it_landed() {
        let backend = Anchored::over(ratatui::backend::TestBackend::new(20, 10), Some(6));

        // Ten rows anchored at row six need four more than the screen has below it.
        let mut terminal = inline_region(backend, 10).unwrap();

        assert_eq!(terminal.get_frame().area().y, 0);
        assert_eq!(terminal.backend().anchor, Some(0));
    }

    /// A [`Screen`] over the simulated terminal, standing where a real one would after
    /// [`Screen::enter`]: a region at the anchor, sized to what was asked.
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

    /// Paint every row of the region as `glyph` and its offset, left-aligned: the blank
    /// cells to the right are the point, because a frame's blank cells are where anything
    /// left standing on the screen shows through.
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

    /// The slash-menu session, played out on a simulated screen: the menu opens (grow),
    /// filters down (shrink), and reopens (grow). After every repaint the screen must hold
    /// exactly the conversation plus the one current frame — any glyph from an earlier
    /// frame, on rows the region kept or rows it gave up, is the interleaving and stacking
    /// this machinery exists to prevent.
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

        // The menu opens: nine rows do not fit below row eight, so the screen scrolls five
        // and the region lands at row three. The five oldest history rows are the
        // terminal's now.
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

        // Typing filters the menu down to four rows. The five rows the region gives up
        // are below it, where nothing will ever draw again — they have to leave with it.
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

        // The menu opens again over the rows just vacated.
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

    /// The same shrink with `clear_on_shrink` set: the sweep reaches the foot of the
    /// screen instead of stopping at the footprint, and the result is the same one frame.
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

    /// Finished conversation lines go in above the region; the region moves for them, and
    /// the next height change builds from the row it moved to.
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

        // The region now starts two rows lower; a shrink built from the row it stood at
        // before would put the new region on top of the inserted lines.
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

    /// A resized terminal gets a rebuilt region on the screen that remains, rather than
    /// ratatui's own answer, which re-anchors the region to a row inside the old one and
    /// paints over the conversation.
    #[test]
    fn a_resize_rebuilds_the_region_on_the_screen_that_remains() {
        let backend = ratatui::backend::TestBackend::new(10, 12);
        let mut screen = inline_screen(backend, 4, 8);
        paint(&mut screen, 'A');

        // The terminal loses half its height: an eight-row region only fits anchored at
        // the top of a six-row screen.
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

        // Room comes back: the region keeps its row, and its height stays what it was
        // clamped to until the next fit asks for more.
        screen.terminal.backend_mut().inner.resize(10, 12);
        screen.fit_screen().unwrap();
        paint(&mut screen, 'C');
        assert_eq!(screen.anchor, Some(0));
        assert_eq!(screen.rows, 6);
    }

    /// A key offered to `setEditorComponent`'s replacement carries the built-in editor's
    /// text along with it — the buffer a key it does not consume is about to fall through
    /// to, which the component otherwise has no way to read. `custom()`'s overlay has no
    /// such buffer to report, which is why this asks specifically about the editor
    /// replacement rather than [`offer_component_input`].
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
