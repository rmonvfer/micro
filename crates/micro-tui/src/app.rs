//! What the interface knows.
//!
//! Every key press becomes an [`Action`], and every action is answered here: the editor, the
//! transcript, whatever overlay is up, and the turn in flight are all state on [`App`]. The
//! event loop owns the terminal and the agent; this owns everything they are showing.
//!
//! Nothing here draws. [`App::refresh_lines`] turns the transcript into wrapped rows and
//! caches them against the width and the transcript's version, so a frame arriving between
//! two streamed tokens reuses what the last one wrapped.

use crate::approval::ApprovalQueue;
use crate::approval::Choice;
use crate::approval::PendingApproval;
use crate::capabilities::ImageProtocol;
use crate::clipboard;
use crate::commands::Applied;
use crate::commands::Commands;
use crate::commands::ConversationState;
use crate::editor::Editor;
use crate::event::Action;
use crate::menu::Menu;
use crate::picker::Picker;
use crate::render::links::Links;
use crate::render::pictures::Pictures;
use crate::render::status::shorten_home;
use crate::render::transcript::Display;
use crate::theme::Theme;
use crate::transcript::NoticeLevel;
use crate::transcript::Transcript;
use micro_commands::MessageKind;
use micro_types::AgentEvent;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::ThinkingLevel;
use ratatui::style::Color;
use ratatui::text::Line;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

/// Rows a page moves, as a share of the visible transcript. A page that moved the whole
/// height would leave nothing in common between the two screens.
const PAGE_OVERLAP: usize = 2;

/// How the interface is set up before it runs.
pub struct TuiOptions {
    /// The workspace the agent is working in, which is also where a `!` command runs.
    pub cwd: PathBuf,
    /// The model as a user would name it, for the footer.
    pub model: String,
    pub context_window: u32,
    pub thinking: ThinkingLevel,
    /// The palette to paint in. Worked out from the terminal when it is left open.
    pub theme: Option<Theme>,
    /// Where tool calls come to be approved, when anything is gating them.
    pub approvals: Option<crate::approval::ApprovalRequests>,
    /// How a slash command is run. Without this every submitted line goes to the model.
    pub commands: Option<Box<dyn Commands + 'static>>,
}

impl Default for TuiOptions {
    fn default() -> Self {
        TuiOptions {
            cwd: std::env::current_dir().unwrap_or_default(),
            model: String::new(),
            context_window: 0,
            thinking: ThinkingLevel::Off,
            theme: None,
            approvals: None,
            commands: None,
        }
    }
}

/// What the event loop should do about a key it has already been handled.
///
/// Everything an action changes on screen is done by the time this comes back; these are
/// only the things the interface cannot do for itself, because they need the terminal, the
/// agent, or the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing further. The action was dealt with here.
    Handled,
    /// Leave.
    Quit,
    /// Stop what is running.
    Interrupt,
    /// Hand the prompt to `$EDITOR`.
    ExternalEditor,
    /// Reason this hard from the next turn on.
    ThinkingChanged(ThinkingLevel),
    /// Step to the next model in the catalog, or the previous one.
    CycleModel(bool),
    /// Drop to the shell.
    Suspend,
}

/// A credential being collected. The key is held here rather than in the editor so it is
/// never drawn, never kept in the prompt's history, and never submitted to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPrompt {
    pub provider: String,
    /// The variables that would supply the key instead, worth naming while asking.
    pub env_names: Vec<String>,
    key: String,
    /// Set when the user pressed enter, so the loop can collect it.
    done: bool,
}

impl KeyPrompt {
    /// How much has been typed, which is all a renderer may know about a secret.
    pub fn len(&self) -> usize {
        self.key.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    /// A prompt with a key already in it, for a renderer's tests. The key is private
    /// everywhere else, so there is no other way to build one part-typed.
    #[cfg(test)]
    pub fn for_test(provider: &str, env_names: Vec<String>, key: &str) -> Self {
        KeyPrompt {
            provider: provider.to_string(),
            env_names,
            key: key.to_string(),
            done: false,
        }
    }
}

/// The turn in flight.
struct Turn {
    /// What to call what is happening, in the activity line.
    label: &'static str,
    started: Instant,
    /// The user asked for it to stop, and it has not stopped yet.
    interrupting: bool,
}

/// The wrapped transcript, kept between frames.
///
/// Wrapping a long conversation is the most expensive thing a frame does, and a streamed
/// answer would otherwise pay for it once per token.
#[derive(Default)]
struct Cache {
    /// What the cached rows were wrapped for. `None` forces the next frame to rebuild.
    pub key: Option<CacheKey>,
    lines: Vec<Line<'static>>,
    links: Links,
    pictures: Pictures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    version: u64,
    width: usize,
    show_thinking: bool,
    focus: Option<usize>,
}

pub struct App {
    pub transcript: Transcript,
    pub editor: Editor,
    pub approvals: ApprovalQueue,
    /// The palette everything is painted in.
    pub theme: Theme,
    /// Whether the model's reasoning is shown alongside its answers.
    pub show_thinking: bool,
    /// How hard the model is reasoning, for the footer and the editor's rules.
    pub thinking: ThinkingLevel,
    pub context_window: u32,
    /// The workspace, shortened the way a shell prompt shortens it.
    pub cwd: String,
    /// The workspace itself, for a command run with `!`.
    pub workspace: PathBuf,
    /// Advanced by the event loop, so the spinner turns while a turn runs.
    pub tick: usize,
    pub should_quit: bool,

    model: String,
    turn: Option<Turn>,
    /// Prompts waiting to be sent, in the order they were written.
    pending: VecDeque<String>,
    /// Lines the interface asked for itself, which go out before anything the user typed.
    injected: VecDeque<String>,
    menu: Option<Menu>,
    picker: Option<Picker>,
    key_prompt: Option<KeyPrompt>,
    /// Images taken off the clipboard, riding with the next prompt.
    attachments: Vec<ContentBlock>,
    /// The tool result the reader has selected, as an index into the transcript.
    focus: Option<usize>,
    /// How far back from the end of the conversation the reader has scrolled.
    scroll: usize,
    /// Rows the transcript has to draw in, which is what a page moves by.
    viewport: usize,
    /// The columns content wraps to, and the rows the whole frame has.
    width: usize,
    rows: u16,
    cache: Cache,
    /// Set by jump-to-char: the next printable key moves the cursor to it.
    jump: Option<bool>,
    hyperlinks: bool,
    images: Option<ImageProtocol>,
}

impl App {
    pub fn new(history: &[Message], options: TuiOptions) -> Self {
        let capabilities = crate::capabilities::detect();
        let workspace = options.cwd;
        App {
            transcript: Transcript::from_messages(history),
            editor: Editor::new(),
            approvals: ApprovalQueue::new(),
            theme: options.theme.unwrap_or_else(Theme::dark),
            show_thinking: false,
            thinking: options.thinking,
            context_window: options.context_window,
            cwd: shorten_home(&workspace.display().to_string()),
            workspace,
            tick: 0,
            should_quit: false,
            model: options.model,
            turn: None,
            pending: VecDeque::new(),
            injected: VecDeque::new(),
            menu: None,
            picker: None,
            key_prompt: None,
            attachments: Vec::new(),
            focus: None,
            scroll: 0,
            viewport: 0,
            width: 80,
            rows: 24,
            cache: Cache::default(),
            jump: None,
            hyperlinks: capabilities.hyperlinks,
            images: capabilities.images,
        }
    }

    /// The model as the footer names it.
    pub fn model_id(&self) -> &str {
        &self.model
    }

    pub fn set_model_label(&mut self, model: String) {
        self.model = model;
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.cache.key = None;
    }

    pub fn set_thinking(&mut self, level: ThinkingLevel) {
        self.thinking = level;
    }

    /// The colour reasoning effort is marked in, which ohm draws on the input's rules.
    pub fn thinking_color(&self) -> Color {
        match self.thinking {
            ThinkingLevel::Off => self.theme.thinking_off,
            ThinkingLevel::Low => self.theme.thinking_low,
            ThinkingLevel::Medium => self.theme.thinking_medium,
            ThinkingLevel::High => self.theme.thinking_high,
        }
    }

    /// How many images are riding with the next prompt.
    pub fn attachments(&self) -> usize {
        self.attachments.len()
    }

    /// The frame the next repaint will be laid out in.
    pub fn set_frame(&mut self, width: usize, rows: u16) {
        self.width = width.max(1);
        self.rows = rows;
    }

    /// How many rows the transcript has to draw in.
    pub fn set_viewport(&mut self, rows: usize) {
        self.viewport = rows;
        self.clamp_scroll();
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn lines(&self) -> &[Line<'static>] {
        &self.cache.lines
    }

    pub fn links(&self) -> &Links {
        &self.cache.links
    }

    pub fn pictures(&self) -> &Pictures {
        &self.cache.pictures
    }

    pub fn menu(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }

    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    pub fn key_prompt(&self) -> Option<&KeyPrompt> {
        self.key_prompt.as_ref()
    }

    /// Whether something is holding the keyboard, which is also what decides where the
    /// cursor is drawn: an input the next keystroke will not reach must not blink.
    pub fn overlay_is_open(&self) -> bool {
        self.approvals.is_open() || self.key_prompt.is_some() || self.picker.is_some()
    }

    pub fn is_running(&self) -> bool {
        self.turn.is_some()
    }

    pub fn is_interrupting(&self) -> bool {
        self.turn
            .as_ref()
            .is_some_and(|turn| turn.interrupting)
    }

    pub fn elapsed(&self) -> Duration {
        self.turn
            .as_ref()
            .map(|turn| turn.started.elapsed())
            .unwrap_or_default()
    }

    /// What the activity line calls what is happening.
    pub fn activity(&self) -> &'static str {
        self.turn
            .as_ref()
            .map(|turn| turn.label)
            .unwrap_or("working")
    }

    /// How many prompts are waiting behind the one in flight.
    pub fn queued(&self) -> usize {
        self.pending.len()
    }

    /// State a command needs to know about the conversation it runs against.
    pub fn conversation_state(&self) -> ConversationState {
        ConversationState {
            message_count: self.transcript.entries().len(),
            usage: self.transcript.total_usage(),
        }
    }

    /// Say that something is happening, so the activity line has a word for it.
    pub fn busy(&mut self, label: &'static str) {
        self.turn = Some(Turn {
            label,
            started: Instant::now(),
            interrupting: false,
        });
    }

    pub fn idle(&mut self) {
        self.turn = None;
    }

    /// Put a line of the interface's own into the conversation.
    pub fn notice(&mut self, text: impl Into<String>, kind: MessageKind) {
        let level = match kind {
            MessageKind::Info => NoticeLevel::Info,
            MessageKind::Error => NoticeLevel::Error,
        };
        self.transcript.push_notice(text, level);
    }

    /// Open a list for the user to choose from.
    pub fn open_picker(&mut self, choices: micro_commands::Picker) {
        self.picker = Some(Picker::new(choices));
    }

    /// Ask for a credential. What is typed is held apart from the prompt, so it is never
    /// drawn, never remembered, and never sent anywhere but the credential store.
    pub fn open_key_prompt(&mut self, provider: String, env_names: Vec<String>) {
        self.key_prompt = Some(KeyPrompt {
            provider,
            env_names,
            key: String::new(),
            done: false,
        });
    }

    /// The credential, once the user has finished typing it.
    pub fn take_key_prompt(&mut self) -> Option<(String, String)> {
        let finished = self
            .key_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.done && !prompt.key.is_empty());
        match finished {
            false => None,
            true => self
                .key_prompt
                .take()
                .map(|prompt| (prompt.provider, prompt.key)),
        }
    }

    /// The next prompt to send, if one has been submitted.
    pub fn take_submission(&mut self) -> Option<String> {
        self.injected
            .pop_front()
            .or_else(|| self.pending.pop_front())
    }

    /// Send a line as though the user had typed it. Used where the interface knows the
    /// command it wants rather than making the user find it.
    pub fn queue_line(&mut self, line: impl Into<String>) {
        self.injected.push_back(line.into());
    }

    /// Record a command the user ran themselves, so the transcript shows it where it
    /// happened rather than only in what the model was told.
    pub fn push_bash(&mut self, command: &str) {
        self.transcript.push_user(format!("! {command}"));
    }

    /// Take a submitted line and make the message the agent will be given.
    ///
    /// Attachments ride in front of the text, which is the order every provider expects,
    /// and are handed over exactly once.
    pub fn begin_turn(&mut self, line: &str) -> Message {
        self.transcript.push_user(line);
        let attached = std::mem::take(&mut self.attachments);
        for block in &attached {
            if let ContentBlock::Image { data, mime_type } = block {
                self.transcript.push_image(data.clone(), mime_type.clone());
            }
        }
        self.editor.remember(line);
        self.busy("thinking");

        let mut content = attached;
        content.push(ContentBlock::text(line));
        Message::User {
            content,
            timestamp: micro_types::now_ms(),
        }
    }

    /// The turn is over, however it ended.
    ///
    /// An abandoned turn leaves requests nobody is going to answer, so they are refused
    /// here rather than opening as prompts during the next one.
    pub fn finish_turn(&mut self, aborted: bool) {
        self.transcript.close();
        self.turn = None;
        if aborted {
            self.approvals.deny_all(crate::approval::INTERRUPTED);
            self.notice("Interrupted", MessageKind::Info);
        }
    }

    /// Take in something the agent reported.
    pub fn apply_event(&mut self, event: AgentEvent) {
        self.transcript.apply(&event);
    }

    /// Take in what the host did with a command.
    pub fn apply_result(&mut self, applied: Applied) {
        match applied {
            Applied::Nothing => {}
            // Both of these need the agent, which this type does not hold. They are
            // intercepted by the caller before reaching here.
            Applied::Model { .. } | Applied::SystemPrompt { .. } => {}
            Applied::Note { text, error } => self.notice(
                text,
                match error {
                    true => MessageKind::Error,
                    false => MessageKind::Info,
                },
            ),
            Applied::Conversation { messages, note } => {
                // The conversation is the host's to define; the scrollback is rebuilt from
                // whatever it says the conversation now is.
                self.transcript = Transcript::from_messages(&messages);
                self.cache.key = None;
                self.focus = None;
                // A replaced conversation is read from its end, like any new one.
                self.scroll = 0;
                if let Some(note) = note {
                    self.notice(note, MessageKind::Info);
                }
            }
        }
    }

    /// A tool call is waiting on an answer.
    pub fn ask_approval(&mut self, pending: PendingApproval) {
        self.approvals.push(pending);
    }

    /// Put the last answer on the system clipboard.
    pub fn copy_last_answer(&mut self) {
        let Some(answer) = self.transcript.last_answer() else {
            self.notice("Nothing to copy yet.", MessageKind::Error);
            return;
        };
        match clipboard::write_text(&answer) {
            true => self.notice("Copied last message to clipboard", MessageKind::Info),
            false => self.notice("No clipboard on this system.", MessageKind::Error),
        }
    }

    /// Write the conversation to a file beside the workspace.
    pub fn export(&mut self, path: Option<&str>) {
        let default = format!(
            "micro-conversation-{}.md",
            micro_types::now_ms() / 1000
        );
        let target = self.workspace.join(path.unwrap_or(&default));

        let mut out = String::new();
        for entry in self.transcript.entries() {
            match entry {
                crate::transcript::Entry::User(text) => {
                    out.push_str(&format!("## Prompt\n\n{text}\n\n"))
                }
                crate::transcript::Entry::Assistant(assistant) => {
                    if !assistant.text.trim().is_empty() {
                        out.push_str(&format!("## Answer\n\n{}\n\n", assistant.text));
                    }
                }
                crate::transcript::Entry::Tool(tool) => out.push_str(&format!(
                    "### Tool: {}\n\n```\n{}\n```\n\n",
                    tool.name,
                    tool.output.as_deref().unwrap_or("(still running)")
                )),
                crate::transcript::Entry::Compaction { summary, .. } => {
                    out.push_str(&format!("## Compacted\n\n{summary}\n\n"))
                }
                crate::transcript::Entry::Image { mime_type, .. } => {
                    out.push_str(&format!("_({mime_type} attached)_\n\n"))
                }
                crate::transcript::Entry::Notice { .. } => {}
            }
        }

        match std::fs::write(&target, out) {
            Ok(()) => self.notice(
                format!("Exported to {}", target.display()),
                MessageKind::Info,
            ),
            Err(error) => self.notice(
                format!("Could not export the conversation: {error}"),
                MessageKind::Error,
            ),
        }
    }

    /// Wrap the transcript for the frame about to be drawn, reusing the last frame's rows
    /// when nothing that affects them has changed.
    pub fn refresh_lines(&mut self) {
        let key = CacheKey {
            version: self.transcript.version(),
            width: self.width,
            show_thinking: self.show_thinking,
            focus: self.focus,
        };
        if self.cache.key == Some(key) {
            return;
        }

        let rendered = crate::render::transcript::lines(
            &self.transcript,
            &self.theme,
            &Display {
                width: self.width,
                show_thinking: self.show_thinking,
                focus: self.focus,
                from: 0,
                hyperlinks: self.hyperlinks,
                images: self.images,
            },
        );
        // A reader who has scrolled back stays over the same lines when more arrive
        // below them, rather than being carried along by the conversation growing.
        let before = self.cache.lines.len();
        self.cache.key = Some(key);
        self.cache.lines = rendered.lines;
        if self.scroll > 0 {
            self.scroll += self.cache.lines.len().saturating_sub(before);
        }
        self.cache.links = rendered.links;
        self.cache.pictures = rendered.pictures;
        self.clamp_scroll();
    }

    /// Answer one action.
    pub fn handle(&mut self, action: Action) -> Outcome {
        // An overlay owns the keyboard while it is up, in the order of what is blocking on
        // an answer: a tool call first, then a credential, then a list to choose from.
        if self.approvals.is_open() {
            return self.handle_approval(action);
        }
        if self.key_prompt.is_some() {
            return self.handle_key_prompt(action);
        }
        if self.picker.is_some() {
            return self.handle_picker(action);
        }

        // Armed by jump-to-char: the next printable key is a destination, not text.
        if let Some(forward) = self.jump {
            if let Action::Insert(text) = &action {
                self.jump = None;
                if let Some(target) = text.chars().next() {
                    self.editor.jump_to_char(target, forward);
                }
                return Outcome::Handled;
            }
            self.jump = None;
        }

        match action {
            Action::Ignored => Outcome::Handled,
            Action::Resize => {
                self.cache.key = None;
                Outcome::Handled
            }

            Action::Quit => Outcome::Quit,
            Action::Interrupt => self.interrupt(),

            Action::Submit => self.submit(),
            Action::QueueFollowUp => self.queue_follow_up(),
            Action::Dequeue => {
                if let Some(line) = self.pending.pop_back() {
                    self.editor.set_text(&line);
                    self.sync_menu();
                }
                Outcome::Handled
            }

            Action::Insert(text) => {
                self.editor.insert_str(&text);
                self.sync_menu();
                Outcome::Handled
            }
            Action::Paste(text) => {
                self.editor.paste(&text);
                self.sync_menu();
                Outcome::Handled
            }
            Action::Newline => {
                self.editor.insert_newline();
                self.sync_menu();
                Outcome::Handled
            }
            Action::Tab => self.complete_or_indent(),

            Action::Backspace => self.edit(|editor| editor.backspace()),
            Action::Delete => self.edit(|editor| editor.delete()),
            Action::DeleteWordBefore => self.edit(|editor| editor.delete_word_before()),
            Action::DeleteWordAfter => self.edit(|editor| editor.delete_word_after()),
            Action::DeleteToLineStart => self.edit(|editor| editor.delete_to_line_start()),
            Action::DeleteToLineEnd => self.edit(|editor| editor.delete_to_line_end()),
            Action::Yank => self.edit(|editor| editor.yank()),
            Action::YankPop => {
                self.editor.yank_pop();
                self.sync_menu();
                Outcome::Handled
            }
            Action::Undo => {
                self.editor.undo();
                self.sync_menu();
                Outcome::Handled
            }

            Action::MoveLeft => self.edit(|editor| editor.move_left()),
            Action::MoveRight => self.edit(|editor| editor.move_right()),
            Action::MoveWordLeft => self.edit(|editor| editor.move_word_left()),
            Action::MoveWordRight => self.edit(|editor| editor.move_word_right()),
            Action::MoveLineStart => self.edit(|editor| editor.move_line_start()),
            Action::MoveLineEnd => self.edit(|editor| editor.move_line_end()),
            Action::MoveUp => self.move_up(),
            Action::MoveDown => self.move_down(),

            Action::PageUp => {
                self.scroll_by(self.page() as isize);
                Outcome::Handled
            }
            Action::PageDown => {
                self.scroll_by(-(self.page() as isize));
                Outcome::Handled
            }
            Action::ScrollUp(lines) => {
                self.scroll_by(lines as isize);
                Outcome::Handled
            }
            Action::ScrollDown(lines) => {
                self.scroll_by(-(lines as isize));
                Outcome::Handled
            }

            Action::FocusPrevious => self.move_focus(false),
            Action::FocusNext => self.move_focus(true),
            Action::ToggleFocused => self.toggle_focused(),

            Action::ToggleThinking => {
                self.show_thinking = !self.show_thinking;
                Outcome::Handled
            }
            Action::CycleThinking => {
                let level = next_level(self.thinking);
                self.thinking = level;
                Outcome::ThinkingChanged(level)
            }

            Action::CycleModel { forward } => Outcome::CycleModel(forward),
            Action::SelectModel => {
                self.queue_line("/model");
                Outcome::Handled
            }
            Action::CopyMessage => {
                self.copy_last_answer();
                Outcome::Handled
            }
            Action::PasteImage => self.paste_image(),

            Action::ArmJump { forward } => {
                self.jump = Some(forward);
                Outcome::Handled
            }
            Action::ExternalEditor => Outcome::ExternalEditor,
            Action::Suspend => Outcome::Suspend,
            Action::Cancel => self.cancel(),
        }
    }

    /// Run an edit and keep the command menu in step with what the prompt now says.
    fn edit(&mut self, change: impl FnOnce(&mut Editor)) -> Outcome {
        change(&mut self.editor);
        self.sync_menu();
        Outcome::Handled
    }

    /// Ctrl+C. What it stops depends on what is going on: a turn, then a half-written
    /// prompt, and with neither there is nothing to interrupt but the wait itself.
    fn interrupt(&mut self) -> Outcome {
        if let Some(turn) = self.turn.as_mut() {
            turn.interrupting = true;
            return Outcome::Interrupt;
        }
        if !self.editor.is_empty() {
            self.editor.clear();
            self.menu = None;
            return Outcome::Handled;
        }
        Outcome::Interrupt
    }

    /// Enter. A menu takes it before the prompt does, so a completion is committed rather
    /// than a half-typed command being sent.
    fn submit(&mut self) -> Outcome {
        if self.commit_completion() {
            return Outcome::Handled;
        }
        // A trailing backslash means the line is being continued, not finished.
        if self.editor.escapes_submit() {
            self.editor.escape_newline();
            return Outcome::Handled;
        }

        let line = self.editor.take();
        self.menu = None;
        if line.trim().is_empty() {
            return Outcome::Handled;
        }
        self.pending.push_back(line);
        self.scroll = 0;
        Outcome::Handled
    }

    /// Alt+Enter: send this after whatever is running rather than instead of it.
    fn queue_follow_up(&mut self) -> Outcome {
        let line = self.editor.take();
        self.menu = None;
        if !line.trim().is_empty() {
            self.pending.push_back(line);
        }
        Outcome::Handled
    }

    /// Tab takes the highlighted completion; with nothing offering one it indents.
    fn complete_or_indent(&mut self) -> Outcome {
        if self.commit_completion() {
            return Outcome::Handled;
        }
        self.editor.insert_str("  ");
        Outcome::Handled
    }

    /// Take the highlighted completion, replacing what was typed toward it and leaving
    /// anything past the cursor where it is.
    fn commit_completion(&mut self) -> bool {
        let Some(menu) = self.menu.as_ref() else {
            return false;
        };
        let Some(completed) = menu.commit() else {
            return false;
        };
        let typed = menu.prefix().len();
        self.editor.replace_before_cursor(typed, &completed);
        self.menu = None;
        true
    }

    /// Up: through the menu when one is open, through the prompt while it has rows above
    /// the cursor, and through what was typed before once it does not.
    fn move_up(&mut self) -> Outcome {
        if let Some(menu) = self.menu.as_mut() {
            menu.select_previous();
            return Outcome::Handled;
        }
        if self.editor.move_up(self.width) {
            return Outcome::Handled;
        }
        self.editor.history_previous();
        Outcome::Handled
    }

    fn move_down(&mut self) -> Outcome {
        if let Some(menu) = self.menu.as_mut() {
            menu.select_next();
            return Outcome::Handled;
        }
        if self.editor.move_down(self.width) {
            return Outcome::Handled;
        }
        self.editor.history_next();
        Outcome::Handled
    }

    /// Escape. It backs out of one thing at a time, nearest first.
    fn cancel(&mut self) -> Outcome {
        if self.menu.take().is_some() {
            return Outcome::Handled;
        }
        if self.is_running() {
            return self.interrupt();
        }
        if self.focus.take().is_some() {
            return Outcome::Handled;
        }
        if self.scroll != 0 {
            self.scroll = 0;
            return Outcome::Handled;
        }
        Outcome::Handled
    }

    /// The command menu belongs to whatever is being typed, so it is rebuilt from the
    /// prompt after every change rather than opened and closed by hand.
    fn sync_menu(&mut self) {
        let (row, column) = self.editor.cursor();
        let line = self
            .editor
            .lines()
            .get(row)
            .cloned()
            .unwrap_or_default();
        // Only the first line can hold a command, and only when nothing precedes it.
        self.menu = match row {
            0 => Menu::open_for(&line, column),
            _ => None,
        };
    }

    fn handle_approval(&mut self, action: Action) -> Outcome {
        match action {
            Action::MoveUp | Action::FocusPrevious => self.approvals.select_previous(),
            Action::MoveDown | Action::FocusNext => self.approvals.select_next(),
            Action::Submit => self.approvals.confirm(),
            Action::Cancel => self.approvals.answer(Choice::Deny),
            // A choice can be answered by its key without moving to it first, which is how
            // ohm lets an approval be answered in one press.
            Action::Insert(text) => match Choice::from_key(&text) {
                Some(choice) => self.approvals.answer(choice),
                None => {}
            },
            Action::Interrupt => {
                self.approvals.answer(Choice::Deny);
                return Outcome::Handled;
            }
            Action::Quit => return Outcome::Quit,
            _ => {}
        }
        Outcome::Handled
    }

    fn handle_key_prompt(&mut self, action: Action) -> Outcome {
        let Some(prompt) = self.key_prompt.as_mut() else {
            return Outcome::Handled;
        };
        match action {
            Action::Insert(text) | Action::Paste(text) => {
                prompt.key.push_str(text.trim_end_matches(['\r', '\n']))
            }
            Action::Backspace => {
                prompt.key.pop();
            }
            Action::Submit => prompt.done = true,
            Action::Cancel | Action::Interrupt => {
                self.key_prompt = None;
            }
            Action::Quit => return Outcome::Quit,
            _ => {}
        }
        Outcome::Handled
    }

    fn handle_picker(&mut self, action: Action) -> Outcome {
        let Some(picker) = self.picker.as_mut() else {
            return Outcome::Handled;
        };
        match action {
            Action::MoveUp => picker.select_previous(),
            Action::MoveDown => picker.select_next(),
            Action::Insert(text) => picker.push(&text),
            Action::Backspace => picker.backspace(),
            Action::Submit => {
                let chosen = picker.commit();
                self.picker = None;
                if let Some(line) = chosen {
                    self.queue_line(line);
                }
            }
            Action::Cancel | Action::Interrupt => self.picker = None,
            Action::Quit => return Outcome::Quit,
            _ => {}
        }
        Outcome::Handled
    }

    /// Move the reader's selection between tool results, which is what can be opened up.
    fn move_focus(&mut self, forward: bool) -> Outcome {
        let positions = self.transcript.tool_positions();
        if positions.is_empty() {
            return Outcome::Handled;
        }

        let next = match (self.focus, forward) {
            (None, true) => positions[0],
            (None, false) => positions[positions.len() - 1],
            (Some(current), true) => positions
                .iter()
                .find(|position| **position > current)
                .copied()
                .unwrap_or(positions[0]),
            (Some(current), false) => positions
                .iter()
                .rev()
                .find(|position| **position < current)
                .copied()
                .unwrap_or(positions[positions.len() - 1]),
        };
        self.focus = Some(next);
        self.cache.key = None;
        Outcome::Handled
    }

    /// Open or close what is selected; with nothing selected, everything at once.
    fn toggle_focused(&mut self) -> Outcome {
        match self.focus {
            Some(index) => {
                self.transcript.toggle_expanded(index);
            }
            None => {
                let opening = self.transcript.any_collapsed();
                self.transcript.set_all_expanded(opening);
            }
        }
        Outcome::Handled
    }

    fn paste_image(&mut self) -> Outcome {
        let Some(image) = clipboard::read_image() else {
            self.notice("No image on the clipboard.", MessageKind::Error);
            return Outcome::Handled;
        };
        self.attachments.push(ContentBlock::Image {
            data: image.data,
            mime_type: image.mime_type,
        });
        self.notice("Image attached to the next message", MessageKind::Info);
        Outcome::Handled
    }

    /// Rows a page moves: what is on screen, less enough to read across the seam.
    fn page(&self) -> usize {
        self.viewport.saturating_sub(PAGE_OVERLAP).max(1)
    }

    /// Move the window back through the conversation. Positive is toward the start.
    fn scroll_by(&mut self, lines: isize) {
        let scroll = self.scroll as isize + lines;
        self.scroll = scroll.max(0) as usize;
        self.clamp_scroll();
    }

    /// Scrolling stops at the first line: past it there is nothing to show, and the window
    /// would drift off the top of the conversation.
    fn clamp_scroll(&mut self) {
        let furthest = self.cache.lines.len().saturating_sub(self.viewport);
        self.scroll = self.scroll.min(furthest);
    }
}

/// The next reasoning level, wrapping at the top.
fn next_level(level: ThinkingLevel) -> ThinkingLevel {
    match level {
        ThinkingLevel::Off => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::Off,
    }
}

/// What a reasoning level is called where a user reads it.
pub fn thinking_name(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
    }
}

/// A byte count as a person would write it.
pub(crate) fn human_size(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}
