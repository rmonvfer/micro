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
mod fuzzy;
mod images;
mod markdown;
mod menu;
mod picker;
mod render;
mod tools;
mod wrap;

// The pieces the interface is assembled from. They hold no terminal state, so a caller that
// wants to show a conversation its own way can reuse them.
pub mod approval;
pub mod editor;
pub mod theme;
pub mod transcript;

pub use app::TuiOptions;
pub use approval::approval_channel;
pub use approval::ApprovalRequests;
pub use commands::Applied;
pub use commands::Commands;
pub use commands::ConversationState;
pub use commands::DoubleEscape;
pub use commands::Preferences;
pub use theme::Theme;

use crate::app::App;
use crate::app::Outcome;
use crate::approval::PendingApproval;
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
    let mut screen = Screen::enter()?;
    // Asked for here, between taking the terminal and drawing on it: the reply has to be
    // read before the event stream exists, and the answer has to be known before the first
    // frame, or that frame paints the wrong theme and then corrects itself.
    options.theme = Some(options.theme.unwrap_or_else(background::detect_theme));
    let result = drive(&mut screen, &mut agent, &history, options).await;
    leave();
    result?;

    Ok(agent.messages().to_vec())
}

async fn drive(
    screen: &mut Screen,
    agent: &mut Agent,
    history: &[Message],
    mut options: TuiOptions,
) -> Result<()> {
    let mut approvals = options.approvals.take();
    let mut commands = options.commands.take();
    let mut app = App::new(history, options);
    let mut input = EventStream::new();

    loop {
        screen.render(&mut app)?;
        if app.should_quit {
            return Ok(());
        }

        // A credential finishes collecting the moment the user presses enter on it.
        if let Some((provider, key)) = app.take_key_prompt() {
            if let Some(commands) = commands.as_mut() {
                app.busy("signing in");
                let stored = commands.store_api_key(provider, key);
                let applied = await_host(screen, &mut app, &mut input, stored).await?;
                app.idle();
                if let Some(applied) = applied {
                    apply_applied(&mut app, agent, applied);
                }
            }
            continue;
        }

        match app.take_submission() {
            Some(line) => {
                submit(
                    screen,
                    &mut app,
                    agent,
                    &mut input,
                    approvals.as_mut(),
                    commands.as_deref_mut(),
                    line,
                )
                .await?
            }
            None => match input.next().await {
                Some(Ok(event)) => match handle(&mut app, event) {
                    Outcome::Quit => return Ok(()),
                    Outcome::ExternalEditor => external_editor(screen, &mut app)?,
                    Outcome::ThinkingChanged(level) => agent.set_thinking(level),
                    // Cycling is `/model` with a direction, so it goes the same way every
                    // other model change does rather than reaching for the catalog here.
                    Outcome::CycleModel(forward) => {
                        app.queue_line(match forward {
                            true => "/model next",
                            false => "/model previous",
                        });
                    }
                    Outcome::Suspend => suspend(screen, &mut app)?,
                    _ => {}
                },
                // The terminal went away; there is nothing left to read.
                Some(Err(_)) | None => return Ok(()),
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
async fn submit(
    screen: &mut Screen,
    app: &mut App,
    agent: &mut Agent,
    input: &mut EventStream,
    approvals: Option<&mut ApprovalRequests>,
    commands: Option<&mut (dyn Commands + 'static)>,
    line: String,
) -> Result<()> {
    // `!` runs a command here instead of asking the model to run one. Its output still
    // joins the conversation, so the model knows what the user just did.
    if let Some(command) = line.strip_prefix('!') {
        return run_bash(screen, app, agent, command.trim()).await;
    }

    let Some(commands) = commands else {
        mark_prompt_submitted();
        let prompt = app.begin_turn(&line);
        return run_turn(screen, app, agent, input, approvals, prompt).await;
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
            run_turn(screen, app, agent, input, approvals, prompt).await
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
    command: &str,
) -> Result<()> {
    if command.is_empty() {
        return Ok(());
    }

    app.push_bash(command);
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
    // anything it ran itself.
    agent.record(Message::user(format!(
        "<bash command=\"{command}\">\n{output}\n</bash>"
    )));
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
        CommandOutcome::Choose(picker) => app.open_picker(picker),
        CommandOutcome::PromptForApiKey {
            provider,
            env_names,
        } => app.open_key_prompt(provider, env_names),

        // Both of these belong to the interface alone. Reasoning effort rides on the model
        // the agent already holds, and a palette never leaves this crate, so neither is
        // worth a round trip through the host.
        CommandOutcome::SetThinking { level } => {
            agent.set_thinking(level);
            app.set_thinking(level);
        }
        // Both of these are the interface's own: it holds the conversation being copied or
        // written out, and neither needs anything the host has.
        CommandOutcome::CopyLastAnswer => app.copy_last_answer(),
        CommandOutcome::Export { path } => app.export(path.as_deref()),

        // Compacting is the agent's own work: it holds the conversation and the model that
        // summarizes it, so nothing about it needs the host.
        CommandOutcome::Compact => {
            app.busy("compacting");
            let compacted = await_work(screen, app, input, agent.compact_now()).await?;
            app.idle();
            match compacted {
                // The summary is the first message of the conversation it left behind, and
                // renders as a compaction marker, so rebuilding from the agent shows it.
                Some(Ok(_)) => app.apply_result(Applied::Conversation {
                    messages: agent.messages().to_vec(),
                    note: None,
                }),
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
            agent.set_messages(messages.clone());
            app.apply_result(Applied::Conversation { messages, note });
        }
        Applied::SystemPrompt { prompt, note } => {
            agent.set_system_prompt(prompt);
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        Applied::Model { swap, note } => {
            app.set_model_label(swap.model.id.clone());
            agent.set_model(*swap);
            if let Some(note) = note {
                app.notice(note, MessageKind::Info);
            }
        }
        other => app.apply_result(other),
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
    approvals: Option<&mut ApprovalRequests>,
    prompt: Message,
) -> Result<()> {
    let (sender, mut receiver) = unbounded_channel::<AgentEvent>();
    let progress = app.settings().terminal_progress;
    report_progress(progress, true);
    let mut approvals = approvals;
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
            Some(pending) = next_approval(&mut approvals) => app.ask_approval(pending),
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
    // A request the abandoned turn had already sent would otherwise open as a prompt during
    // the next one, for a call that is no longer going to run. Collecting it here hands it
    // to `finish_turn`, which refuses everything outstanding.
    if let Some(requests) = approvals {
        while let Some(pending) = requests.try_recv() {
            app.ask_approval(pending);
        }
    }
    app.finish_turn(aborted);
    report_progress(progress, false);
    Ok(())
}

/// The next approval request. With nothing able to ask, this never resolves, which leaves
/// the arm inert rather than closing the select over it.
async fn next_approval(requests: &mut Option<&mut ApprovalRequests>) -> Option<PendingApproval> {
    match requests {
        Some(requests) => requests.recv().await,
        None => std::future::pending().await,
    }
}

fn handle(app: &mut App, event: Event) -> Outcome {
    app.handle(event::action_for(&event))
}

/// The terminal, and the inline region the interface lives in.
///
/// The region is only as tall as the interface needs. When that changes it is rebuilt at the
/// new height from the same row, which is what lets the interface grow downward from where
/// the shell left the cursor instead of taking the screen away.
struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
        terminal.clear()?;
        Ok(Screen { terminal })
    }

    /// Bring the interface up to date: hand whatever has left the live region to the
    /// terminal, size the region to what is left, and paint it.
    fn render(&mut self, app: &mut App) -> Result<()> {
        // The whole screen is ours, so a frame is drawn where it stands: no region to move,
        // nothing to hand to the scrollback, and the input stays on the last rows because
        // the layout puts it there.
        self.terminal.draw(|frame| render::draw(frame, app))?;
        Ok(())
    }

    /// Take the terminal back after another program has had it.
    fn reopen(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        self.terminal.clear()?;
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
        DisableMouseCapture,
        DisableBracketedPaste,
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
