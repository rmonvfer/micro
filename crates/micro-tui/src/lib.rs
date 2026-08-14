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
mod clipboard;
mod background;
mod capabilities;
mod commands;
mod diff;
mod event;
mod images;
mod latex;
mod layout;
mod markdown;
mod menu;
mod picker;
mod render;
mod tools;
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
pub use commands::Listings;
pub use commands::Commands;
pub use commands::ConversationState;
pub use commands::DoubleEscape;
pub use commands::Preferences;
pub use theme::Theme;
pub use ui::ui_channel;
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
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
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
    let mut app = App::new(history, options);
    let outcome = run_loop(
        screen,
        agent,
        &mut app,
        &mut questions,
        &mut commands,
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
    _said: &mut Vec<String>,
) -> Result<()> {
    let mut input = EventStream::new();
    // Set while a list of models is open and the providers are being asked what they serve.
    let mut refreshing: Option<tokio::sync::oneshot::Receiver<Listings>> = None;

    loop {
        screen.render(app)?;
        if app.should_quit {
            return Ok(());
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
        if let Some(question) = questions.as_mut().and_then(|questions| questions.try_recv()) {
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
                    },
                    commands.as_deref_mut(),
                    line,
                )
                .await?
            }
            None => match next_event(&mut input, &mut refreshing, app, commands).await {
                // Work finished behind the interface rather than something the user did:
                // the frame is drawn again at the top of the loop and nothing else changes.
                Next::Redrawn => continue,
                Next::Event(event) => {
                    if offer_shortcut(commands, &event).await {
                        continue;
                    }
                    match handle(app, event) {
                    Outcome::Quit => return Ok(()),
                    Outcome::ExternalEditor => external_editor(screen, app)?,
                    Outcome::ThinkingChanged(level) => agent.set_thinking(level),
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
) -> Next {
    // Nothing in flight, and a list open that wants asking about: start now.
    if refreshing.is_none() && app.picker_mut().is_some_and(|open| open.refreshes()) {
        if let Some(commands) = commands.as_deref_mut() {
            *refreshing = commands.begin_model_refresh();
        }
    }

    let Some(pending) = refreshing.as_mut() else {
        return arrived(input.next().await);
    };

    tokio::select! {
        biased;
        event = input.next() => arrived(event),
        listings = pending => {
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
    }
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
        Ok(_) => app.notice(format!("{editor} exited without saving."), MessageKind::Info),
        Err(error) => app.notice(format!("Could not run {editor}: {error}"), MessageKind::Error),
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
        return run_turn(screen, app, agent, input, questions, prompt).await;
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
            run_turn(screen, app, agent, input, questions, prompt).await
        }
        // A prompt written for the purpose becomes the turn, in place of the line that
        // asked for it.
        Some(Some(CommandOutcome::Send { prompt })) => {
            mark_prompt_submitted();
            let prompt = app.begin_turn(&prompt);
            run_turn(screen, app, agent, input, questions, prompt).await
        }
        Some(Some(outcome)) => apply_outcome(screen, app, agent, input, commands, outcome).await,
    }
}

/// Run a shell command on the user's behalf and put what it printed into the conversation.
///
/// The model is not asked anything, but it is told: the command and its output are recorded
/// so the next turn knows what the user just did, the same way ohm treats a `!` line.
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

    let finished = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&app.workspace)
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    app.idle();

    let (output, failed) = match finished {
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
    };

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
    if let Some(commands) = commands {
        commands.ran_bash(command, &output, failed).await;
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
        Applied::SystemPrompt { prompt, note } => {
            // Reloading re-reads the workspace, so the file listing behind `@` is read
            // again too rather than describing the workspace as it was at startup.
            app.forget_workspace_files();
            agent.set_system_prompt(prompt);
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        Applied::Model { swap, note } => {
            // Everything the footer says about the model changes with it: the name, and
            // how much room there is, which is what the context share is measured against.
            app.set_model_label(swap.model.id.clone());
            app.context_window = swap.context_window as u32;
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
    prompt: Message,
) -> Result<()> {
    let (sender, mut receiver) = unbounded_channel::<AgentEvent>();
    let progress = app.settings().terminal_progress;
    report_progress(progress, true);
    let mut turn = Box::pin(agent.run(prompt, &sender));
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut painted = Instant::now() - FRAME;
    let mut aborted = false;

    loop {
        if painted.elapsed() >= FRAME {
            screen.render(app)?;
            painted = Instant::now();
        }

        // Input is polled first so a key press is never starved by a fast stream, and the
        // agent's events are drained before its future is polled so nothing is lost when
        // the turn finishes with events still queued.
        tokio::select! {
            biased;
            event = input.next() => match event {
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
            Some(question) = next_question(questions) => app.ask_question(question),
            Some(event) = receiver.recv() => {
                app.apply_event(event);
                while let Ok(next) = receiver.try_recv() {
                    app.apply_event(next);
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
    report_progress(progress, false);
    Ok(())
}

/// The next question from an extension. With nothing able to ask, this never resolves,
/// which leaves the arm inert rather than closing the select over it.
async fn next_question(requests: &mut Option<crate::ui::UiRequests>) -> Option<crate::ui::UiRequest> {
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
async fn offer_shortcut(
    commands: &mut Option<Box<dyn Commands + 'static>>,
    event: &Event,
) -> bool {
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

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    mode: TuiMode,
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
        match mode {
            TuiMode::Fullscreen => {
                execute!(
                    std::io::stdout(),
                    EnterAlternateScreen,
                    EnableBracketedPaste
                )?;
                let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
                terminal.clear()?;
                Ok(Screen { terminal, mode })
            }
            TuiMode::Inline => {
                execute!(std::io::stdout(), EnableBracketedPaste)?;
                // The region is only as tall as the interface needs; what leaves it is
                // handed to the terminal's own scrollback rather than being redrawn.
                let terminal = Terminal::with_options(
                    CrosstermBackend::new(std::io::stdout()),
                    ratatui::TerminalOptions {
                        viewport: ratatui::Viewport::Inline(INLINE_ROWS),
                    },
                )?;
                Ok(Screen { terminal, mode })
            }
        }
    }

    /// Bring the interface up to date: hand whatever has left the live region to the
    /// terminal, size the region to what is left, and paint it.
    fn render(&mut self, app: &mut App) -> Result<()> {
        // Whatever the conversation has finished with leaves the live region and becomes
        // part of the terminal's own scrollback, where its search and selection reach it.
        if self.mode == TuiMode::Inline {
            let finished = app.take_scrolled_out();
            if !finished.is_empty() {
                let rows = finished.len() as u16;
                self.terminal.insert_before(rows, |buffer| {
                    for (offset, line) in finished.iter().enumerate() {
                        buffer.set_line(0, offset as u16, line, buffer.area.width);
                    }
                })?;
            }
        }
        self.terminal.draw(|frame| render::draw(frame, app))?;
        Ok(())
    }

    /// Take the terminal back after another program has had it.
    fn reopen(&mut self) -> Result<()> {
        enable_raw_mode()?;
        match self.mode {
            TuiMode::Fullscreen => {
                execute!(std::io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
                self.terminal.clear()?;
            }
            TuiMode::Inline => execute!(std::io::stdout(), EnableBracketedPaste)?,
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
}
