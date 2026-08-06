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
use crate::commands::Preferences;
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
    /// Where questions from extensions arrive, when anything can ask them.
    pub questions: Option<crate::ui::UiRequests>,
    /// How a slash command is run. Without this every submitted line goes to the model.
    pub commands: Option<Box<dyn Commands + 'static>>,
    /// What the user settled in `/settings`.
    pub settings: Preferences,
    /// Something to say before anything else happens, such as that nobody is signed in
    /// to the service serving the chosen model.
    pub notice: Option<String>,
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
            questions: None,
            notice: None,
            commands: None,
            settings: Preferences::default(),
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
    /// What is being asked for, which is the provider when a credential is wanted and the
    /// question itself when an extension is asking.
    pub provider: String,
    /// The variables that would supply the key instead, worth naming while asking.
    pub env_names: Vec<String>,
    key: String,
    /// Set when the user pressed enter, so the loop can collect it.
    done: bool,
    /// Whether what is typed is drawn back. A credential never is; an answer to a question
    /// always is, or the person cannot see what they are writing.
    pub masked: bool,
}

impl KeyPrompt {
    /// How much has been typed, which is all a renderer may know about a secret.
    pub fn len(&self) -> usize {
        self.key.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    /// What has been typed, for a prompt that is not asking for a secret.
    pub fn text(&self) -> &str {
        &self.key
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
            masked: true,
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
    /// A question an extension asked, waiting for whatever overlay is showing it.
    question: Option<crate::ui::UiRequest>,
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
    /// Answers taken in so far, so a cache notice knows there was a cache to read.
    answers: usize,
    hyperlinks: bool,
    images: Option<ImageProtocol>,
    /// What the user settled in `/settings`, honoured wherever it applies.
    settings: Preferences,
}

impl App {
    pub fn new(history: &[Message], options: TuiOptions) -> Self {
        let capabilities = crate::capabilities::detect();
        let workspace = options.cwd;
        let notice = options.notice;
        let mut app = App {
            transcript: Transcript::from_messages(history),
            editor: Editor::new(),
            approvals: ApprovalQueue::new(),
            theme: options.theme.unwrap_or_else(Theme::dark),
            show_thinking: !options.settings.hide_thinking,
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
            question: None,
            attachments: Vec::new(),
            focus: None,
            scroll: 0,
            viewport: 0,
            width: 80,
            rows: 24,
            cache: Cache::default(),
            jump: None,
            answers: 0,
            hyperlinks: capabilities.hyperlinks,
            // A terminal that can draw images still does not when the user would rather
            // read the conversation without them.
            images: match options.settings.show_images {
                true => capabilities.images,
                false => None,
            },
            settings: options.settings,
        };

        // Said before the first frame, so the reason nothing can be sent is on screen
        // rather than waiting to be discovered by sending something.
        if let Some(notice) = notice {
            app.notice(notice, MessageKind::Info);
        }
        app
    }

    /// What the user settled in `/settings`, for the parts of the frame drawn elsewhere.
    pub fn settings(&self) -> &Preferences {
        &self.settings
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

    /// How many completions the menu may show at once.
    pub fn menu_rows(&self) -> usize {
        self.settings.autocomplete_max_items
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
        // A warning nobody wants to see is not shown at all, rather than shown quietly.
        if matches!(kind, MessageKind::Warning) && !self.settings.warnings {
            return;
        }
        let level = match kind {
            MessageKind::Info => NoticeLevel::Info,
            MessageKind::Warning => NoticeLevel::Warning,
            MessageKind::Error => NoticeLevel::Error,
        };
        self.transcript.push_notice(text, level);
    }

    /// Say something that is worth knowing but not worth stopping for.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.notice(text, MessageKind::Warning);
    }

    /// Open a list for the user to choose from.
    pub fn open_picker(&mut self, choices: micro_commands::Picker) {
        self.picker = Some(Picker::new(choices));
    }

    /// Show a question an extension asked, in whatever suits it.
    ///
    /// The question is held until it is answered or closed, so however the overlay ends,
    /// the extension is told something rather than left waiting.
    pub fn ask_question(&mut self, request: crate::ui::UiRequest) {
        match request.method.as_str() {
            // Not a question: a message an extension wants said, which goes into the
            // conversation as though the user had written it.
            "send_user_message" => {
                let mut request = request;
                self.queue_line(request.title.clone());
                request.answer(serde_json::json!({ "queued": true }));
                return;
            }
            // Something an extension drew: the title names it, the options are its lines.
            "custom_message" => {
                let mut request = request;
                self.transcript
                    .push_custom(request.title.clone(), request.options.clone());
                request.answer(serde_json::json!({ "shown": true }));
                return;
            }
            "notify" => {
                let mut request = request;
                self.notice(request.title.clone(), MessageKind::Info);
                request.answer(serde_json::json!({}));
                return;
            }
            "select" => {
                let items = request
                    .options
                    .iter()
                    .map(|option| {
                        micro_commands::PickerItem::new(option.clone(), String::new(), option.clone())
                    })
                    .collect();
                self.open_picker(micro_commands::Picker::new(request.title.clone(), items));
            }
            "confirm" => {
                let detail = request.detail.clone().unwrap_or_default();
                let items = vec![
                    micro_commands::PickerItem::new("Yes", detail.clone(), "yes"),
                    micro_commands::PickerItem::new("No", detail, "no"),
                ];
                self.open_picker(micro_commands::Picker::new(request.title.clone(), items));
            }
            // Anything else is asked in words.
            _ => self.open_input(request.title.clone(), request.detail.clone()),
        }
        self.question = Some(request);
    }

    /// Ask for a credential. What is typed is held apart from the prompt, so it is never
    /// drawn, never remembered, and never sent anywhere but the credential store.
    pub fn open_key_prompt(&mut self, provider: String, env_names: Vec<String>) {
        self.key_prompt = Some(KeyPrompt {
            provider,
            env_names,
            key: String::new(),
            done: false,
            masked: true,
        });
    }

    /// Ask the user something in words, and show what they type.
    ///
    /// The same overlay as a credential prompt, drawn plainly: an extension asking a
    /// question is not asking for a secret.
    pub fn open_input(&mut self, question: String, placeholder: Option<String>) {
        self.key_prompt = Some(KeyPrompt {
            provider: question,
            env_names: placeholder.into_iter().collect(),
            key: String::new(),
            done: false,
            masked: false,
        });
    }

    /// The credential, once the user has finished typing it.
    pub fn take_key_prompt(&mut self) -> Option<(String, String)> {
        let finished = self
            .key_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.done && !prompt.is_empty());
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
        let answers_before = self.answers;
        self.transcript.apply(&event);
        if let AgentEvent::MessageEnd { .. } = &event {
            self.answers = self.answers.saturating_add(1);
            self.report_cache_miss(answers_before);
        }
    }

    /// A turn that wrote the cache without reading any of it paid twice for a context it
    /// already had. Only worth saying after the first answer, when there was a cache to
    /// have read.
    fn report_cache_miss(&mut self, answers_before: usize) {
        if !self.settings.cache_miss_notices || answers_before == 0 {
            return;
        }
        let usage = self.transcript.last_usage();
        if usage.cache_write == 0 || usage.cache_read > 0 {
            return;
        }
        self.warn(format!(
            "Cache miss: {} tokens were written to the cache again",
            usage.cache_write
        ));
    }

    /// Take in what the host did with a command.
    pub fn apply_result(&mut self, applied: Applied) {
        match applied {
            Applied::Nothing => {}
            // Both of these need the agent, which this type does not hold. They are
            // intercepted by the caller before reaching here.
            Applied::Model { .. } | Applied::SystemPrompt { .. } => {}
            Applied::Note { text, kind } => self.notice(text, kind),
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
            self.notice("No agent messages to copy yet.", MessageKind::Error);
            return;
        };
        match clipboard::write_text(&answer) {
            true => self.notice("Copied last agent message to clipboard", MessageKind::Info),
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
                crate::transcript::Entry::Custom { label, lines } => {
                    out.push_str(&format!("## {label}\n\n{}\n\n", lines.join("\n")))
                }
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
                image_width: self.settings.image_width_cells as usize,
                resize_images: self.settings.auto_resize_images,
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
        // Enter takes a completion only when there is something left to complete. A
        // command typed out in full is a command the user meant to send, and swallowing
        // that press to add a space would make every short command need two.
        if self.completion_would_change_the_line() && self.commit_completion() {
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
        if line.trim().is_empty() {
            return Outcome::Handled;
        }
        self.pending.push_back(line);
        // Set to interrupt, a follow-up is meant to replace what is running rather than
        // to wait behind it, so the turn is stopped and the prompt goes next.
        match self.settings.follow_up_interrupts && self.is_running() {
            true => self.interrupt(),
            false => Outcome::Handled,
        }
    }

    /// Tab takes the highlighted completion; with nothing offering one it indents.
    fn complete_or_indent(&mut self) -> Outcome {
        if self.commit_completion() {
            return Outcome::Handled;
        }
        self.editor.insert_str("\t");
        Outcome::Handled
    }

    /// Whether committing would write anything the user has not already typed.
    fn completion_would_change_the_line(&self) -> bool {
        let Some(menu) = self.menu.as_ref() else {
            return false;
        };
        match menu.commit() {
            Some(completed) => completed.trim_end() != menu.prefix(),
            None => false,
        }
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
        // Nothing was open and nothing was scrolled, and the prompt is empty: this is the
        // second escape, which is how ohm offers a way back to an earlier point.
        if self.editor.is_empty() {
            match self.settings.double_escape {
                crate::commands::DoubleEscape::Tree => self.queue_line("/tree"),
                crate::commands::DoubleEscape::Fork => self.queue_line("/fork"),
                crate::commands::DoubleEscape::None => {}
            }
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
            Action::Insert(text) => {
                if let Some(choice) = Choice::from_key(&text) {
                    self.approvals.answer(choice);
                }
            }
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
            Action::Submit => {
                prompt.done = true;
                let said = prompt.text().to_string();
                // A question asked in words is answered here; a credential is collected by
                // the loop, which is what `take_key_prompt` is for.
                if let Some(mut question) = self.question.take() {
                    self.key_prompt = None;
                    question.answer(serde_json::json!({ "value": said }));
                }
            }
            Action::Cancel | Action::Interrupt => {
                self.key_prompt = None;
                if let Some(mut question) = self.question.take() {
                    question.cancel();
                }
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
                match self.question.take() {
                    // The chosen item carries the answer, not a command to run.
                    Some(mut question) => {
                        let answer = match (question.method.as_str(), chosen.as_deref()) {
                            ("confirm", Some(said)) => {
                                serde_json::json!({ "confirmed": said == "yes" })
                            }
                            (_, Some(said)) => serde_json::json!({ "value": said }),
                            (_, None) => serde_json::json!({ "cancelled": true }),
                        };
                        question.answer(answer);
                    }
                    None => {
                        if let Some(line) = chosen {
                            self.queue_line(line);
                        }
                    }
                }
            }
            Action::Cancel | Action::Interrupt => {
                self.picker = None;
                if let Some(mut question) = self.question.take() {
                    question.cancel();
                }
            }
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
        if self.settings.block_images {
            self.notice("Images are turned off in /settings.", MessageKind::Error);
            return Outcome::Handled;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Applied;
    use micro_types::AssistantMessage;
    use micro_types::StopReason;
    use micro_types::Usage;

    fn app() -> App {
        App::new(&[], TuiOptions::default())
    }

    fn type_text(app: &mut App, text: &str) {
        app.handle(Action::Insert(text.to_string()));
    }

    fn transcript_text(app: &mut App) -> String {
        app.set_frame(60, 24);
        app.refresh_lines();
        app.lines()
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn submitting_queues_the_prompt_and_empties_the_editor() {
        let mut app = app();
        type_text(&mut app, "explain this");
        app.handle(Action::Submit);

        assert!(app.editor.is_empty());
        assert_eq!(app.take_submission().as_deref(), Some("explain this"));
        assert_eq!(app.take_submission(), None);
    }

    #[test]
    fn an_empty_prompt_submits_nothing() {
        let mut app = app();
        app.handle(Action::Submit);
        type_text(&mut app, "   ");
        app.handle(Action::Submit);
        assert_eq!(app.take_submission(), None);
    }

    #[test]
    fn a_slash_opens_the_command_menu_and_typing_narrows_it() {
        let mut app = app();
        type_text(&mut app, "/");
        assert_eq!(
            app.menu().map(|menu| menu.items().len()),
            Some(micro_commands::commands().len())
        );

        type_text(&mut app, "c");
        let names: Vec<String> = app
            .menu()
            .unwrap()
            .items()
            .iter()
            .map(|item| item.value.clone())
            .collect();
        assert_eq!(
            names,
            vec!["clone", "changelog", "copy", "compact", "clear", "cwd"]
        );
    }

    #[test]
    fn the_menu_closes_on_escape_without_touching_the_prompt() {
        let mut app = app();
        type_text(&mut app, "/mo");
        assert!(app.menu().is_some());

        app.handle(Action::Cancel);
        assert!(app.menu().is_none());
        assert_eq!(app.editor.text(), "/mo", "the text is left as typed");
    }

    #[test]
    fn the_arrows_move_the_menu_rather_than_the_prompt() {
        let mut app = app();
        type_text(&mut app, "/c");
        assert_eq!(app.menu().unwrap().selected(), 0);

        app.handle(Action::MoveDown);
        assert_eq!(app.menu().unwrap().selected(), 1);
        app.handle(Action::MoveUp);
        assert_eq!(app.menu().unwrap().selected(), 0);
        assert_eq!(app.scroll(), 0, "the transcript did not scroll");
    }

    #[test]
    fn enter_takes_the_highlighted_command_instead_of_submitting() {
        let mut app = app();
        type_text(&mut app, "/c");
        app.handle(Action::MoveDown);
        app.handle(Action::Submit);

        assert_eq!(
            app.editor.text(),
            "/changelog ",
            "the second of the /c commands"
        );
        assert_eq!(app.queued(), 0, "committing is not submitting");
        assert!(app.menu().is_none(), "the space closed the menu");
    }

    #[test]
    fn tab_takes_the_highlighted_command_too() {
        let mut app = app();
        type_text(&mut app, "/mo");
        app.handle(Action::Tab);
        assert_eq!(app.editor.text(), "/model ");
    }

    #[test]
    fn tab_indents_when_no_command_is_being_typed() {
        let mut app = app();
        type_text(&mut app, "plain");
        app.handle(Action::Tab);
        assert_eq!(app.editor.text(), "plain\t");
    }

    /// Committing replaces what was typed toward the command, not the whole prompt: an
    /// argument already written stays where it is.
    #[test]
    fn committing_keeps_what_follows_the_cursor() {
        let mut app = app();
        type_text(&mut app, "/mo");
        app.handle(Action::Tab);
        assert_eq!(app.editor.text(), "/model ");
    }

    #[test]
    fn a_command_that_matches_nothing_closes_the_menu_and_still_submits() {
        let mut app = app();
        type_text(&mut app, "/zzzz");
        assert!(app.menu().is_none());

        app.handle(Action::Submit);
        assert_eq!(
            app.take_submission().as_deref(),
            Some("/zzzz"),
            "dispatch decides it is unknown, not the menu"
        );
    }

    #[test]
    fn submitting_a_command_leaves_no_menu_behind() {
        let mut app = app();
        type_text(&mut app, "/help");
        app.handle(Action::Submit);
        assert!(app.menu().is_none());
        assert_eq!(app.take_submission().as_deref(), Some("/help"));
    }

    fn app_choosing() -> App {
        let mut app = app();
        app.open_picker(micro_commands::Picker::new(
            "Select a model",
            vec![
                micro_commands::PickerItem::new("opus-5", "200k", "/model opus-5"),
                micro_commands::PickerItem::new("sonnet-5", "200k", "/model sonnet-5"),
                micro_commands::PickerItem::new("gemini-2.5-pro", "1M", "/model gemini"),
            ],
        ));
        app
    }

    #[test]
    fn a_picker_filters_on_typing_rather_than_reaching_the_prompt() {
        let mut app = app_choosing();
        app.handle(Action::Insert("gem".into()));

        assert_eq!(app.picker().unwrap().query(), "gem");
        assert_eq!(app.picker().unwrap().matches().len(), 1);
        assert!(app.editor.is_empty(), "typing did not reach the input");

        app.handle(Action::Backspace);
        assert_eq!(app.picker().unwrap().query(), "ge");
    }

    #[test]
    fn choosing_queues_the_line_the_item_carries() {
        let mut app = app_choosing();
        app.handle(Action::MoveDown);
        app.handle(Action::Submit);

        assert!(app.picker().is_none());
        assert_eq!(app.take_submission().as_deref(), Some("/model sonnet-5"));
    }

    #[test]
    fn a_picker_is_dismissed_by_escape() {
        let mut app = app_choosing();
        app.handle(Action::Cancel);
        assert!(app.picker().is_none());
        assert_eq!(app.queued(), 0);
    }

    #[test]
    fn a_key_prompt_collects_without_echoing_and_hands_the_key_over() {
        let mut app = app();
        app.open_key_prompt("anthropic".into(), vec!["ANTHROPIC_API_KEY".into()]);
        app.handle(Action::Insert("sk-secret".into()));

        assert!(app.editor.is_empty());
        assert_eq!(app.key_prompt().unwrap().len(), 9);

        app.handle(Action::Submit);
        let (provider, key) = app.take_key_prompt().expect("a key was typed");
        assert_eq!(provider, "anthropic");
        assert_eq!(key, "sk-secret");
        assert!(app.key_prompt().is_none());
    }

    #[test]
    fn an_empty_key_prompt_hands_nothing_over() {
        let mut app = app();
        app.open_key_prompt("anthropic".into(), Vec::new());
        app.handle(Action::Submit);
        assert!(app.take_key_prompt().is_none());
        assert!(app.key_prompt().is_some(), "it is still waiting");

        app.handle(Action::Cancel);
        assert!(app.key_prompt().is_none());
    }

    #[test]
    fn a_replaced_conversation_rebuilds_the_scrollback() {
        let mut app = app();
        app.transcript.push_user("before");
        assert!(transcript_text(&mut app).contains("before"));

        app.apply_result(Applied::Conversation {
            messages: Vec::new(),
            note: Some("cleared".into()),
        });

        let text = transcript_text(&mut app);
        assert!(!text.contains("before"), "the old conversation is gone");
        assert!(text.contains("cleared"));
    }

    #[test]
    fn a_note_from_the_host_leaves_the_conversation_alone() {
        let mut app = app();
        app.transcript.push_user("kept");
        app.apply_result(Applied::note("now on claude-opus-5"));

        let text = transcript_text(&mut app);
        assert!(text.contains("kept"));
        assert!(text.contains("now on claude-opus-5"));
    }

    /// An attached image rides in front of the text, which is the order every provider
    /// expects, and is handed over exactly once.
    #[test]
    fn an_attached_image_goes_with_the_next_prompt() {
        let mut app = app();
        app.attachments.push(ContentBlock::Image {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        });
        assert_eq!(app.attachments(), 1);

        let Message::User { content, .. } = app.begin_turn("what is this") else {
            panic!("a prompt is a user message");
        };
        assert!(matches!(content[0], ContentBlock::Image { .. }));
        assert_eq!(content[1].as_text(), "what is this");
        assert_eq!(app.attachments(), 0, "handed over exactly once");

        let Message::User { content, .. } = app.begin_turn("and this") else {
            panic!("a prompt is a user message");
        };
        assert_eq!(content.len(), 1, "the image did not ride twice");
    }

    #[test]
    fn a_prompt_is_remembered_and_can_be_recalled() {
        let mut app = app();
        app.begin_turn("the first thing asked");
        app.handle(Action::MoveUp);
        assert_eq!(app.editor.text(), "the first thing asked");
    }

    /// Ctrl+C clears a half-written prompt before it interrupts anything, which is what
    /// makes it safe to press when nothing is running.
    #[test]
    fn interrupting_clears_the_prompt_before_it_stops_anything() {
        let mut app = app();
        type_text(&mut app, "half written");
        assert_eq!(app.handle(Action::Interrupt), Outcome::Handled);
        assert!(app.editor.is_empty());

        assert_eq!(app.handle(Action::Interrupt), Outcome::Interrupt);
    }

    #[test]
    fn interrupting_a_running_turn_marks_it_rather_than_clearing() {
        let mut app = app();
        app.busy("thinking");
        assert_eq!(app.handle(Action::Interrupt), Outcome::Interrupt);
        assert!(app.is_interrupting());
        assert!(app.is_running(), "it has not stopped yet");

        app.finish_turn(true);
        assert!(!app.is_running());
    }

    #[test]
    fn a_follow_up_queues_behind_the_turn_in_flight() {
        let mut app = app();
        app.busy("thinking");
        type_text(&mut app, "and then this");
        app.handle(Action::QueueFollowUp);

        assert_eq!(app.queued(), 1);
        assert!(app.editor.is_empty());
        assert_eq!(app.take_submission().as_deref(), Some("and then this"));
    }

    #[test]
    fn dequeue_pulls_the_last_queued_prompt_back_into_the_editor() {
        let mut app = app();
        type_text(&mut app, "first");
        app.handle(Action::Submit);
        type_text(&mut app, "second");
        app.handle(Action::Submit);

        app.handle(Action::Dequeue);
        assert_eq!(app.editor.text(), "second");
        assert_eq!(app.queued(), 1);
    }

    /// A line the interface asked for itself goes out before anything the user typed: it
    /// is the answer to a key they just pressed.
    #[test]
    fn an_injected_line_goes_before_a_queued_prompt() {
        let mut app = app();
        type_text(&mut app, "typed");
        app.handle(Action::Submit);
        app.queue_line("/model");

        assert_eq!(app.take_submission().as_deref(), Some("/model"));
        assert_eq!(app.take_submission().as_deref(), Some("typed"));
    }

    #[test]
    fn cycling_reasoning_effort_reports_the_new_level() {
        let mut app = app();
        assert_eq!(
            app.handle(Action::CycleThinking),
            Outcome::ThinkingChanged(ThinkingLevel::Low)
        );
        assert_eq!(app.thinking, ThinkingLevel::Low);
        assert_eq!(thinking_name(app.thinking), "low");
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut app = app();
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();

        app.handle(Action::ScrollDown(5));
        assert_eq!(app.scroll(), 0, "the end is as far forward as it goes");

        for _ in 0..100 {
            app.handle(Action::PageUp);
        }
        let furthest = app.lines().len() - 10;
        assert_eq!(app.scroll(), furthest, "the start is as far back as it goes");
    }

    /// Someone reading back through the conversation stays where they are when an answer
    /// arrives beneath them.
    #[test]
    fn arriving_content_does_not_move_what_is_being_read() {
        let mut app = app();
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();
        app.handle(Action::PageUp);

        let reading = app.scroll();
        let before: String = app.lines()[app.lines().len() - reading - 10]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        app.transcript.push_user("something new");
        app.refresh_lines();

        let after: String = app.lines()[app.lines().len() - app.scroll() - 10]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(before, after, "the same line is still at the top");
    }

    #[test]
    fn focus_moves_between_tool_results_and_opens_them() {
        let mut app = app();
        app.apply_event(AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "src/main.rs" }),
        });
        app.apply_event(AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: (0..40).map(|n| format!("line {n}\n")).collect(),
            is_error: false,
        });

        assert_eq!(app.handle(Action::FocusNext), Outcome::Handled);
        let focused = app.transcript.tool_positions()[0];
        assert_eq!(app.focus, Some(focused));

        app.handle(Action::ToggleFocused);
        assert!(!app.transcript.any_collapsed(), "the result opened");
    }

    #[test]
    fn the_conversation_state_counts_what_is_on_screen() {
        let mut app = app();
        app.transcript.push_user("one");
        app.transcript.push_user("two");
        assert_eq!(app.conversation_state().message_count, 2);
    }

    #[test]
    fn copying_with_nothing_to_copy_says_so() {
        let mut app = app();
        app.copy_last_answer();
        assert!(transcript_text(&mut app).contains("No agent messages to copy yet."));
    }

    #[test]
    fn the_last_answer_is_what_gets_copied() {
        let mut app = app();
        app.transcript = Transcript::from_messages(&[Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("the answer")],
            provider: "openrouter".into(),
            model: "gemini-3-pro".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })]);
        assert_eq!(app.transcript.last_answer().as_deref(), Some("the answer"));
    }

    #[test]
    fn a_bash_line_joins_the_conversation_where_it_was_run() {
        let mut app = app();
        app.push_bash("ls -la");
        assert!(transcript_text(&mut app).contains("! ls -la"));
    }

    /// The arrow keys stop at the edge of the prompt rather than wrapping around it.
    #[test]
    fn the_arrow_keys_stop_at_the_edge_of_the_prompt() {
        let mut app = app();
        type_text(&mut app, "abc");
        for _ in 0..10 {
            app.handle(Action::MoveLeft);
        }
        assert_eq!(app.editor.cursor(), (0, 0));

        for _ in 0..10 {
            app.handle(Action::MoveRight);
        }
        assert_eq!(app.editor.cursor(), (0, 3));
    }

    /// Up at the top of a multi-line prompt moves through the rows before it reaches back
    /// for what was sent earlier.
    #[test]
    fn moving_up_walks_the_prompt_before_the_history() {
        let mut app = app();
        app.set_frame(60, 24);
        app.begin_turn("an earlier prompt");
        type_text(&mut app, "second line");
        app.handle(Action::Newline);
        type_text(&mut app, "third line");

        app.handle(Action::MoveUp);
        assert!(
            app.editor.text().contains("second line"),
            "still in the prompt"
        );
        app.handle(Action::MoveUp);
        app.handle(Action::MoveUp);
        assert_eq!(app.editor.text(), "an earlier prompt");
    }

    /// A trailing backslash continues the line instead of sending it, which is how a
    /// prompt gets a newline on a keyboard that cannot send shift+enter.
    #[test]
    fn a_trailing_backslash_continues_the_line() {
        let mut app = app();
        type_text(&mut app, "first\\");
        app.handle(Action::Submit);

        assert_eq!(app.take_submission(), None, "nothing was sent");
        assert_eq!(app.editor.text(), "first\n");
    }

    #[test]
    fn escape_backs_out_of_one_thing_at_a_time() {
        let mut app = app();
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();

        type_text(&mut app, "/mo");
        app.handle(Action::Cancel);
        assert!(app.menu().is_none(), "the menu goes first");

        app.handle(Action::PageUp);
        assert!(app.scroll() > 0);
        app.handle(Action::Cancel);
        assert_eq!(app.scroll(), 0, "then where the reader had scrolled to");
    }

    /// Jump-to-char takes the next key as a destination rather than as text.
    #[test]
    fn jump_to_char_moves_the_cursor_instead_of_typing() {
        let mut app = app();
        type_text(&mut app, "alpha beta");
        app.handle(Action::MoveLineStart);

        app.handle(Action::ArmJump { forward: true });
        app.handle(Action::Insert("b".into()));

        assert_eq!(app.editor.text(), "alpha beta", "the key was not typed");
        assert_eq!(app.editor.cursor().1, 6, "the cursor moved to it");
    }

    /// An approval owns the keyboard while it is up: a key that would reach the prompt is
    /// an answer instead.
    #[test]
    fn an_approval_takes_the_keyboard_while_it_is_open() {
        let (approver, mut requests) = crate::approval::approval_channel();
        let mut app = app();

        let asking = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        asking.block_on(async {
            let request = micro_policy::ApprovalRequest {
                tool: "bash".into(),
                subject: Some("rm -rf /".into()),
                arguments: serde_json::json!({ "command": "rm -rf /" }),
                reason: "the policy cannot vouch for this".into(),
                key: "bash:rm -rf /".into(),
            };
            let answering =
                tokio::spawn(async move { approver.approve(&request).await });
            let pending = requests.recv().await.expect("a request");
            app.ask_approval(pending);

            assert!(app.overlay_is_open());
            app.handle(Action::Insert("x".into()));
            assert!(app.editor.is_empty(), "typing did not reach the prompt");

            app.handle(Action::Cancel);
            assert!(!app.overlay_is_open(), "answering closed it");
            assert!(matches!(
                answering.await.unwrap(),
                micro_policy::Approval::Denied(_)
            ));
        });
    }

    /// An abandoned turn refuses every request it left behind, so nothing opens as a
    /// prompt during the next one.
    #[test]
    fn an_interrupted_turn_refuses_what_it_left_behind() {
        let (approver, mut requests) = crate::approval::approval_channel();
        let mut app = app();
        app.busy("thinking");

        let asking = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        asking.block_on(async {
            let request = micro_policy::ApprovalRequest {
                tool: "write".into(),
                subject: Some("src/main.rs".into()),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
                reason: "the policy cannot vouch for this".into(),
                key: "write:src/main.rs".into(),
            };
            let answering =
                tokio::spawn(async move { approver.approve(&request).await });
            let pending = requests.recv().await.expect("a request");
            app.ask_approval(pending);

            app.finish_turn(true);
            assert!(!app.overlay_is_open(), "nothing is left asking");
            assert!(matches!(
                answering.await.unwrap(),
                micro_policy::Approval::Denied(_)
            ));
        });
    }

    #[test]
    fn the_theme_can_be_changed_without_touching_the_conversation() {
        let mut app = app();
        app.transcript.push_user("kept");
        app.set_theme(Theme::light());

        assert_eq!(app.theme.name, Theme::light().name);
        assert!(transcript_text(&mut app).contains("kept"));
    }

    #[test]
    fn the_model_label_follows_a_swap() {
        let mut app = app();
        app.set_model_label("google/gemini-3-pro".into());
        assert_eq!(app.model_id(), "google/gemini-3-pro");
    }

    /// Reasoning is hidden until it is asked for, and asking rewraps the transcript.
    #[test]
    fn thinking_can_be_shown_and_hidden() {
        let mut app = app();
        assert!(!app.show_thinking);
        app.handle(Action::ToggleThinking);
        assert!(app.show_thinking);
        app.handle(Action::ToggleThinking);
        assert!(!app.show_thinking);
    }

    #[test]
    fn a_streamed_answer_lands_in_the_transcript() {
        let mut app = app();
        app.apply_event(AgentEvent::MessageDelta {
            event: micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "half an ".into(),
            },
        });
        app.apply_event(AgentEvent::MessageDelta {
            event: micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "answer".into(),
            },
        });
        assert!(transcript_text(&mut app).contains("half an answer"));
    }

    /// Wrapping is the most expensive thing a frame does, so it is not redone when
    /// nothing that affects it has changed.
    #[test]
    fn the_wrapped_transcript_is_kept_between_frames() {
        let mut app = app();
        app.transcript.push_user("something to wrap");
        app.set_frame(60, 24);
        app.refresh_lines();
        let key = app.cache.key;

        app.refresh_lines();
        assert_eq!(app.cache.key, key, "nothing changed, so nothing rewrapped");

        app.set_frame(40, 24);
        app.refresh_lines();
        assert_ne!(app.cache.key, key, "a new width wraps again");
    }

    fn app_with(settings: Preferences) -> App {
        App::new(
            &[],
            TuiOptions {
                settings,
                ..TuiOptions::default()
            },
        )
    }

    /// Every preference the interface takes changes what it does, so a setting is a
    /// setting rather than a row in a menu.
    #[test]
    fn hiding_thinking_starts_it_folded_away() {
        assert!(!app_with(Preferences {
            hide_thinking: true,
            ..Preferences::default()
        })
        .show_thinking);

        assert!(app_with(Preferences {
            hide_thinking: false,
            ..Preferences::default()
        })
        .show_thinking);
    }

    #[test]
    fn blocking_images_refuses_to_attach_one() {
        let mut app = app_with(Preferences {
            block_images: true,
            ..Preferences::default()
        });
        app.handle(Action::PasteImage);

        assert_eq!(app.attachments(), 0);
        assert!(transcript_text(&mut app).contains("Images are turned off in /settings."));
    }

    #[test]
    fn turning_warnings_off_leaves_them_unsaid() {
        let mut app = app_with(Preferences {
            warnings: false,
            ..Preferences::default()
        });
        app.warn("something worth knowing");
        assert!(!transcript_text(&mut app).contains("something worth knowing"));

        let mut app = app_with(Preferences {
            warnings: true,
            ..Preferences::default()
        });
        app.warn("something worth knowing");
        assert!(transcript_text(&mut app).contains("something worth knowing"));
    }

    #[test]
    fn a_second_escape_does_what_the_setting_says() {
        let mut app = app_with(Preferences {
            double_escape: crate::commands::DoubleEscape::Tree,
            ..Preferences::default()
        });
        app.handle(Action::Cancel);
        assert_eq!(app.take_submission().as_deref(), Some("/tree"));

        let mut app = app_with(Preferences {
            double_escape: crate::commands::DoubleEscape::None,
            ..Preferences::default()
        });
        app.handle(Action::Cancel);
        assert_eq!(app.take_submission(), None);
    }

    #[test]
    fn a_follow_up_can_interrupt_instead_of_waiting() {
        let mut app = app_with(Preferences {
            follow_up_interrupts: true,
            ..Preferences::default()
        });
        app.busy("thinking");
        type_text(&mut app, "instead, do this");
        assert_eq!(app.handle(Action::QueueFollowUp), Outcome::Interrupt);
        assert!(app.is_interrupting());
        assert_eq!(app.queued(), 1);
    }

    #[test]
    fn the_menu_offers_as_many_rows_as_asked_for() {
        let app = app_with(Preferences {
            autocomplete_max_items: 3,
            ..Preferences::default()
        });
        assert_eq!(app.menu_rows(), 3);
    }

    /// A terminal that can draw images still does not when the reader would rather it
    /// did not.
    #[test]
    fn turning_images_off_stops_them_being_drawn() {
        let app = app_with(Preferences {
            show_images: false,
            ..Preferences::default()
        });
        assert!(app.images.is_none());
    }

    #[test]
    fn a_cache_miss_is_only_reported_when_asked_for() {
        let mut app = app_with(Preferences {
            cache_miss_notices: false,
            ..Preferences::default()
        });
        app.answers = 1;
        app.report_cache_miss(1);
        assert!(!transcript_text(&mut app).contains("Cache miss"));
    }

    /// A credential is handed over only once enter has been pressed on it, so a key that
    /// is still being typed is never sent anywhere.
    #[test]
    fn a_key_is_only_handed_over_once_it_is_finished() {
        let mut app = app();
        app.open_key_prompt("openrouter".into(), Vec::new());
        app.handle(Action::Insert("sk-or".into()));
        assert!(app.take_key_prompt().is_none(), "still being typed");

        app.handle(Action::Backspace);
        assert_eq!(app.key_prompt().unwrap().len(), 4);
        app.handle(Action::Submit);
        assert_eq!(
            app.take_key_prompt(),
            Some(("openrouter".to_string(), "sk-o".to_string()))
        );
    }

    #[test]
    fn queued_prompts_come_back_in_the_order_they_were_written() {
        let mut app = app();
        for line in ["first", "second", "third"] {
            type_text(&mut app, line);
            app.handle(Action::Submit);
        }
        assert_eq!(app.queued(), 3);
        assert_eq!(app.take_submission().as_deref(), Some("first"));
        assert_eq!(app.take_submission().as_deref(), Some("second"));
        assert_eq!(app.take_submission().as_deref(), Some("third"));
        assert_eq!(app.queued(), 0);
    }

    /// A conversation shorter than the screen has nowhere to scroll to.
    #[test]
    fn a_short_conversation_does_not_scroll() {
        let mut app = app();
        app.transcript.push_user("one line");
        app.set_frame(60, 24);
        app.set_viewport(20);
        app.refresh_lines();

        app.handle(Action::PageUp);
        assert_eq!(app.scroll(), 0);
    }

    /// Making the window taller cannot leave the reader scrolled past the start.
    #[test]
    fn a_taller_window_pulls_the_reader_back_inside_the_conversation() {
        let mut app = app();
        for index in 0..30 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(5);
        app.refresh_lines();
        for _ in 0..50 {
            app.handle(Action::PageUp);
        }
        assert!(app.scroll() > 0);

        app.set_viewport(1000);
        assert_eq!(app.scroll(), 0);
    }

    /// The menu belongs to what is being typed, so finishing the command word closes it.
    #[test]
    fn the_menu_closes_once_the_command_word_is_finished() {
        let mut app = app();
        type_text(&mut app, "/model");
        assert!(app.menu().is_some());
        type_text(&mut app, " ");
        assert!(app.menu().is_none(), "an argument is not a command");
    }

    /// A menu only belongs to the first line: a slash further down is text.
    #[test]
    fn a_slash_on_a_later_line_is_not_a_command() {
        let mut app = app();
        type_text(&mut app, "look at");
        app.handle(Action::Newline);
        type_text(&mut app, "/etc/hosts");
        assert!(app.menu().is_none());
    }

    #[test]
    fn a_resize_makes_the_next_frame_wrap_again() {
        let mut app = app();
        app.transcript.push_user("something to wrap");
        app.set_frame(60, 24);
        app.refresh_lines();
        assert!(app.cache.key.is_some());

        app.handle(Action::Resize);
        assert!(app.cache.key.is_none(), "the next frame rewraps");
    }

    #[test]
    fn browsing_history_walks_back_through_what_was_sent() {
        let mut app = app();
        app.begin_turn("first thing");
        app.begin_turn("second thing");

        app.handle(Action::MoveUp);
        assert_eq!(app.editor.text(), "second thing");
        app.handle(Action::MoveUp);
        assert_eq!(app.editor.text(), "first thing");
        app.handle(Action::MoveDown);
        assert_eq!(app.editor.text(), "second thing");
    }

    #[test]
    fn the_conversation_is_written_out_where_it_was_asked_for() {
        let mut app = app();
        app.workspace = std::env::temp_dir().join(format!("micro-export-{}", std::process::id()));
        std::fs::create_dir_all(&app.workspace).unwrap();
        app.transcript.push_user("a question");

        app.export(Some("conversation.md"));
        let written = std::fs::read_to_string(app.workspace.join("conversation.md")).unwrap();
        assert!(written.contains("## Prompt\n\na question"), "{written}");
        assert!(transcript_text(&mut app).contains("Exported to"));
    }

    #[test]
    fn an_export_that_cannot_be_written_says_so() {
        let mut app = app();
        app.workspace = std::path::PathBuf::from("/nowhere-that-exists");
        app.export(Some("conversation.md"));
        assert!(transcript_text(&mut app).contains("Could not export the conversation"));
    }

    /// Reasoning effort is marked on the input's rules, so each level has its own colour.
    #[test]
    fn every_reasoning_level_has_its_own_colour() {
        let mut app = app();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(app.thinking_color());
            app.handle(Action::CycleThinking);
        }
        assert_eq!(app.thinking, ThinkingLevel::Off, "four steps wraps around");

        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "no two levels look the same");
    }

    #[test]
    fn a_turn_reports_that_it_is_running_and_what_it_is_doing() {
        let mut app = app();
        assert!(!app.is_running());
        assert_eq!(app.activity(), "working");

        app.begin_turn("do the thing");
        assert!(app.is_running());
        assert_eq!(app.activity(), "thinking");
        assert!(!app.is_interrupting());

        app.busy("compacting");
        assert_eq!(app.activity(), "compacting");
        app.idle();
        assert!(!app.is_running());
    }

    /// An approval can be answered by its own key without moving to it first.
    #[test]
    fn an_approval_takes_the_answer_by_its_key() {
        let (approver, mut requests) = crate::approval::approval_channel();
        let mut app = app();

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let request = micro_policy::ApprovalRequest {
                    tool: "read".into(),
                    subject: Some("src/main.rs".into()),
                    arguments: serde_json::json!({ "path": "src/main.rs" }),
                    reason: "the policy cannot vouch for this".into(),
                    key: "read:src/main.rs".into(),
                };
                let answering = tokio::spawn(async move { approver.approve(&request).await });
                app.ask_approval(requests.recv().await.expect("a request"));

                app.handle(Action::Insert("a".into()));
                assert!(!app.overlay_is_open());
                assert_eq!(answering.await.unwrap(), micro_policy::Approval::Session);
            });
    }

    /// A prompt sent while the conversation is scrolled back brings the reader to the end,
    /// since what they just asked is the thing worth watching.
    #[test]
    fn sending_a_prompt_returns_to_the_end_of_the_conversation() {
        let mut app = app();
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();
        app.handle(Action::PageUp);
        assert!(app.scroll() > 0);

        type_text(&mut app, "the next thing");
        app.handle(Action::Submit);
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn the_usage_a_command_is_told_about_is_what_the_answers_cost() {
        let mut app = app();
        app.transcript = Transcript::from_messages(&[Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("done")],
            provider: "openrouter".into(),
            model: "gemini-3-pro".into(),
            usage: Usage {
                input: 100,
                output: 20,
                cache_read: 5,
                cache_write: 3,
            },
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })]);

        let state = app.conversation_state();
        assert_eq!(state.message_count, 1);
        assert_eq!(state.usage.input, 100);
        assert_eq!(state.usage.output, 20);
    }

    #[test]
    fn a_byte_count_reads_as_a_size() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(5 * 1_048_576), "5.0 MB");
    }
}
