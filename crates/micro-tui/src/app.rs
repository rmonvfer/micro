//! What the interface knows.

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
use crate::render::pictures::Placement;
use crate::render::status::shorten_home;
use crate::render::transcript::Display;
use crate::render::transcript::Rendered;
use crate::theme::Theme;
use crate::transcript::NoticeLevel;
use crate::transcript::Transcript;
use micro_commands::InspectionItem;
use micro_commands::MessageKind;
use micro_types::AgentEvent;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::ThinkingLevel;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::text::Span;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

/// Rows a page moves, as a share of the visible transcript.
const PAGE_OVERLAP: usize = 2;

/// How the interface is set up before it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceSection {
    /// What the section is called, written as `[Context]`, `[Skills]` and so on.
    pub name: String,
    /// What is in it, by name.
    pub names: Vec<String>,
    /// Where each of them was read from, in the same order.
    pub paths: Vec<String>,
}

/// Everything the first screen says was loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resources {
    pub sections: Vec<ResourceSection>,
}

impl Resources {
    /// Add a section, unless nothing was found for it.
    pub fn add(&mut self, name: &str, names: Vec<String>, paths: Vec<String>) {
        if names.is_empty() {
            return;
        }
        self.sections.push(ResourceSection {
            name: name.to_string(),
            names,
            paths,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

pub struct TuiOptions {
    /// The workspace the agent is working in, which is also where a `!` command runs.
    pub cwd: PathBuf,
    /// The model as a user would name it, for the footer.
    pub model: String,
    pub context_window: u32,
    pub thinking: ThinkingLevel,
    /// The palette to paint in.
    pub theme: Option<Theme>,
    /// Where tool calls come to be approved, when anything is gating them.
    pub questions: Option<crate::ui::UiRequests>,
    /// How a slash command is run.
    pub commands: Option<Box<dyn Commands + 'static>>,
    /// What the user settled in `/settings`.
    pub settings: Preferences,
    /// Something to say before anything else happens, such as that nobody is signed in to the
    /// service serving the chosen model.
    pub notice: Option<String>,
    /// The provider serving the model, named in the footer when the model does not say.
    pub provider: String,

    pub subscription: bool,
    /// Whether the conversation summarizes itself once it fills the window.
    pub auto_compact: bool,
    /// What the model charges, for the session's running cost.
    pub price: Option<micro_models::ModelCost>,
    /// Durable cost read from the session ledger.
    pub session_cost: Option<f64>,
    /// Durable all-turn and latest-turn usage read from the session ledger.
    pub session_usage: Option<(micro_types::Usage, micro_types::Usage)>,
    /// Whether this run has experimental behavior turned on, which is worth showing because it
    /// changes what micro does.
    pub experimental: bool,
    /// How much of the terminal to take.
    pub tui_mode: crate::TuiMode,
    /// What was loaded before the session started, named on the first screen.
    pub resources: Resources,

    pub terminal_input: Option<crate::ui::TerminalInputAsker>,
    /// Where the interface asks the host something off the render path.
    pub host_asker: Option<crate::ui::HostAsker>,
    /// Names of tools whose `render_shell` asked for `"self"`.
    pub self_framed_tools: HashSet<String>,
    /// Commands supplied by extensions and skills, offered beside the built-in ones.
    pub extension_commands: Vec<crate::menu::MenuItem>,
    /// Where a phone reaches this session, once one has been handed it.
    pub remote: Option<crate::remote::Remote>,
}

impl Default for TuiOptions {
    fn default() -> Self {
        TuiOptions {
            cwd: std::env::current_dir().unwrap_or_default(),
            model: String::new(),
            context_window: 0,
            thinking: ThinkingLevel::Off,
            theme: None,
            questions: None,
            terminal_input: None,
            host_asker: None,
            self_framed_tools: HashSet::new(),
            extension_commands: Vec::new(),
            remote: None,
            notice: None,
            provider: String::new(),
            subscription: false,
            auto_compact: true,
            price: None,
            session_cost: None,
            session_usage: None,
            experimental: false,
            tui_mode: Default::default(),
            resources: Default::default(),
            commands: None,
            settings: Preferences::default(),
        }
    }
}

/// What the event loop should do about a key it has already been handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing further.
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

/// A credential being collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPrompt {
    /// What is being asked for, which is the provider when a credential is wanted and the question
    /// itself when an extension is asking.
    pub provider: String,
    /// The variables that would supply the key instead, worth naming while asking.
    pub env_names: Vec<String>,
    key: String,
    /// Set when the user pressed enter, so the loop can collect it.
    done: bool,
    /// Whether what is typed is drawn back.
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

    /// A prompt with a key already in it, for a renderer's tests.
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

/// A prompt held until the current work can accept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedMessage {
    pub text: String,
    pub kind: QueueKind,
}

/// Why a prompt is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Steering,
    FollowUp,
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
#[derive(Default)]
struct Cache {
    shape: Option<Shape>,
    rendered: Rendered,
    /// Where each entry's rows begin, so a conversation can be redrawn from the middle.
    starts: Vec<usize>,
    /// Where each entry's links and images begin, for the same reason.
    links_from: Vec<usize>,
    pictures_from: Vec<usize>,
}

/// What every entry's rows depend on beyond the entry itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    width: usize,
    show_thinking: bool,
    focus: Option<usize>,
}

pub struct App {
    pub transcript: Transcript,
    pub editor: Editor,
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

    worked: bool,
    /// Whether the previous action was escape, so two escapes can clear the prompt.
    escape_pending: bool,
    /// Whether ctrl+c has been pressed once on an empty prompt, so the next one leaves.
    quitting: bool,
    pub should_quit: bool,

    model: String,
    turn: Option<Turn>,
    /// Whether the active operation is compacting the conversation.
    compacting: bool,
    /// Prompts waiting to be sent, in the order they were written.
    pending: VecDeque<QueuedMessage>,
    /// Lines the interface asked for itself, which go out before anything the user typed.
    injected: VecDeque<String>,
    menu: Option<Menu>,
    picker: Option<Picker>,
    key_prompt: Option<KeyPrompt>,
    /// A question an extension asked, waiting for whatever overlay is showing it.
    question: Option<crate::ui::UiRequest>,
    /// What each extension is reporting, by the key it reports under.
    pub extension_status: std::collections::BTreeMap<String, String>,
    /// The workspace's files, for completing a name after `@`.
    file_index: Option<Vec<String>>,

    handed_over: usize,

    pub subscription: bool,
    /// Whether the conversation summarizes itself once it fills the window.
    pub auto_compact: bool,
    /// The provider serving the model, named in the footer when the model does not say.
    pub provider: String,
    /// What a million tokens costs, for the running total.
    pub price: Option<micro_models::ModelCost>,
    session_cost: Option<f64>,
    session_usage: Option<(micro_types::Usage, micro_types::Usage)>,
    /// Images taken off the clipboard, riding with the next prompt.
    attachments: Vec<ContentBlock>,
    /// The tool result the reader has selected, as an index into the transcript.
    focus: Option<usize>,
    /// How far back from the end of the conversation the reader has scrolled.
    scroll: usize,

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
    /// Where the images go on the frame just laid out, for the terminal to draw once the rows they
    /// sit on have landed.
    placements: Vec<Placement>,
    /// What the user settled in `/settings`, honoured wherever it applies.
    settings: Preferences,
    /// What was loaded before the session started.
    resources: Resources,
    /// How much of the terminal this run took, which decides whether the spinner's rows are held
    /// open while nothing is running.
    tui_mode: crate::TuiMode,
    /// The branch the workspace is on, for the footer.
    branch: Option<String>,

    startup_expanded: bool,
    /// What an extension asked the activity line to call what is happening, in place of the turn's
    /// own word for it.
    working_message: Option<String>,
    /// Whether the spinner's row is drawn at all, apart from whether its rows are held open.
    working_visible: bool,
    /// The spinner's animation, in place of the built-in braille frames.
    working_indicator: Option<WorkingIndicator>,
    /// The label a folded reasoning block collapses to.
    hidden_thinking_label: Option<String>,
    /// Lines an extension asked shown above or below the input, by the key it asked under.
    widgets: std::collections::BTreeMap<String, Widget>,
    /// Whether an extension is listening to every key before the interface decides what to do with
    /// it.
    wants_terminal_input: bool,
    /// A terminal title an extension asked for since the last frame, taken and written once by
    /// whoever owns the terminal.
    title_change: Option<String>,
    /// Lines standing in for the opening screen, from `setHeader`.
    header_override: Option<Vec<String>>,
    /// Lines standing in for the footer, from `setFooter`.
    footer_override: Option<Vec<String>>,
    /// What a live component's lines are standing in for, by the id it was registered under.
    component_slots: std::collections::HashMap<String, ComponentSlot>,
    /// A live component shown with keyboard focus, from `custom()`.
    component_overlay: Option<ComponentOverlay>,
    /// A local, read-only ledger view opened by `/bill`, `/why-miss`, or `/request`.
    inspection: Option<InspectionOverlay>,
    /// The component `setEditorComponent` replaced the built-in editor with, and what it last
    /// looked like.
    editor_component: Option<ComponentOverlay>,

    drawn: Drawn,
    /// Tools a deactivated extension provided, waiting to be taken off the agent.
    retired_tools: Vec<String>,
    /// Names of tools whose `render_shell` asked for `"self"`, kept so a conversation rebuilt mid-
    /// run.
    self_framed_tools: HashSet<String>,

    extension_commands: Vec<crate::menu::MenuItem>,
    /// A multi-line editor shown for an extension's `editor()`, holding the keyboard until it
    /// submits or is cancelled.
    extension_editor: Option<ExtensionEditorOverlay>,

    autocomplete_triggers: Vec<char>,
    /// A `getSuggestions` question `sync_menu` owes an extension, taken once by the event loop.
    pending_suggestion_request: Option<SuggestionRequest>,
    /// An `applyCompletion` question queued by committing an extension's own menu item, taken once
    /// by the event loop.
    pending_completion_request: Option<CompletionRequest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SuggestionRequest {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub prefix: String,
}

/// What the interface asks an extension's `applyCompletion` about, once the reader commits one of
/// its items: the buffer, the cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionRequest {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub item: serde_json::Value,
    pub prefix: String,
}

/// A live component holding keyboard focus in place of something built in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ComponentOverlay {
    component_id: String,
    lines: Vec<String>,
    cursor: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectionOverlay {
    title: String,
    text: String,
    items: Vec<InspectionItem>,
    selected: usize,
    detail_open: bool,
    scroll: usize,
    /// The selected detail or standalone text, wrapped once for the current frame width.
    body: Vec<Line<'static>>,
    body_width: usize,
    body_detail: Option<usize>,
}

/// The data needed to render an open inspection overlay.
pub type Inspection<'a> = (
    &'a str,
    &'a str,
    &'a [InspectionItem],
    &'a [Line<'static>],
    usize,
    bool,
    usize,
);

/// A real [`Editor`] shown for an extension's `editor()`, unlike [`ComponentOverlay`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExtensionEditorOverlay {
    title: String,
    editor: Editor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ComponentSlot {
    Header,
    Footer,
    Widget(String),
}

/// Which extension put each lasting thing on the screen.
#[derive(Debug, Default)]
struct Drawn {
    /// The extension behind each widget key, and each status key.
    widgets: std::collections::BTreeMap<String, String>,
    status: std::collections::BTreeMap<String, String>,
    header: Option<String>,
    footer: Option<String>,
    editor: Option<String>,
}

impl Drawn {
    /// Note that this extension is now behind this key, and forget whoever was before.
    fn by(map: &mut std::collections::BTreeMap<String, String>, key: &str, owner: Option<&String>) {
        match owner {
            Some(owner) => {
                map.insert(key.to_string(), owner.clone());
            }
            None => {
                map.remove(key);
            }
        }
    }

    /// Every key this extension is behind, so what it drew can be taken back by key.
    fn keys_of(map: &std::collections::BTreeMap<String, String>, extension: &str) -> Vec<String> {
        map.iter()
            .filter(|(_, owner)| owner.as_str() == extension)
            .map(|(key, _)| key.clone())
            .collect()
    }
}

/// A slot as `register_component_slot`'s detail names it: `"header"`, `"footer"`, or
/// `"widget:<key>"`.
fn component_slot_named(name: Option<&str>) -> Option<ComponentSlot> {
    match name? {
        "header" => Some(ComponentSlot::Header),
        "footer" => Some(ComponentSlot::Footer),
        rest => rest
            .strip_prefix("widget:")
            .map(|key| ComponentSlot::Widget(key.to_string())),
    }
}

/// A spinner's animation, asked for in place of the built-in one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingIndicator {
    pub frames: Vec<String>,
    /// How long each frame is shown before the next.
    pub interval_ms: u64,
}

/// Where a widget's lines are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WidgetPlacement {
    Above,
    Below,
}

/// One widget an extension is showing beside the input.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Widget {
    lines: Vec<String>,
    placement: WidgetPlacement,
}

const MAX_WIDGET_LINES: usize = 10;

/// The zero-width marker pi's own components emit at the cursor position when they want a hardware
/// cursor shown.
const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// A live component's lines, with the cursor marker taken out of whichever one carried it.
fn strip_cursor_marker(lines: Vec<String>) -> (Vec<String>, Option<(usize, usize)>) {
    for (row, line) in lines.iter().enumerate() {
        if let Some(at) = line.find(CURSOR_MARKER) {
            let mut stripped = lines.clone();
            stripped[row] = format!("{}{}", &line[..at], &line[at + CURSOR_MARKER.len()..]);
            return (stripped, Some((row, at)));
        }
    }
    (lines, None)
}

/// The branch a workspace is on, or nothing when it is not a repository.
fn git_branch(workspace: &std::path::Path) -> Option<String> {
    let asked = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !asked.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&asked.stdout).trim().to_string();

    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

impl App {
    pub fn new(history: &[Message], options: TuiOptions) -> Self {
        let capabilities = crate::capabilities::detect();
        let workspace = options.cwd;
        let branch = git_branch(&workspace);
        let notice = options.notice;
        let self_framed_tools = options.self_framed_tools;
        let extension_commands = options.extension_commands;
        let mut transcript = Transcript::from_messages(history);
        transcript.set_self_framed_tools(self_framed_tools.clone());
        let mut app = App {
            transcript,
            extension_commands,
            editor: Editor::new(),
            theme: options.theme.unwrap_or_else(Theme::dark),
            show_thinking: !options.settings.hide_thinking,
            thinking: options.thinking,
            context_window: options.context_window,
            worked: false,
            quitting: false,
            escape_pending: false,
            cwd: shorten_home(&workspace.display().to_string()),
            workspace,
            tick: 0,
            should_quit: false,
            model: options.model,
            turn: None,
            compacting: false,
            pending: VecDeque::new(),
            injected: VecDeque::new(),
            menu: None,
            picker: None,
            key_prompt: None,
            question: None,
            extension_status: Default::default(),
            file_index: None,
            handed_over: 0,
            subscription: options.subscription,
            auto_compact: options.auto_compact,
            provider: options.provider,
            price: options.price,
            session_cost: options.session_cost,
            session_usage: options.session_usage,
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

            images: match options.settings.show_images {
                true => capabilities.images,
                false => None,
            },
            placements: Vec::new(),
            settings: options.settings,
            resources: options.resources,
            tui_mode: options.tui_mode,
            branch,
            startup_expanded: false,
            working_message: None,
            working_visible: true,
            working_indicator: None,
            hidden_thinking_label: None,
            widgets: Default::default(),
            wants_terminal_input: false,
            title_change: None,
            header_override: None,
            footer_override: None,
            component_slots: Default::default(),
            drawn: Drawn::default(),
            retired_tools: Vec::new(),
            component_overlay: None,
            inspection: None,
            editor_component: None,
            self_framed_tools,
            extension_editor: None,
            autocomplete_triggers: Vec::new(),
            pending_suggestion_request: None,
            pending_completion_request: None,
        };

        if let Some(notice) = notice {
            app.notice(notice, MessageKind::Info);
        }
        app
    }

    /// What the user settled in `/settings`, for the parts of the frame drawn elsewhere.
    pub fn settings(&self) -> &Preferences {
        &self.settings
    }

    /// Whether the two rows the spinner draws in are held open.
    pub fn reserves_activity_rows(&self) -> bool {
        match self.tui_mode {
            crate::TuiMode::Inline => self.worked,

            crate::TuiMode::Fullscreen => self.is_running(),
        }
    }

    /// The model as the footer names it.
    pub fn model_id(&self) -> &str {
        &self.model
    }

    pub fn set_model_label(&mut self, model: String) {
        self.model = model;
    }

    /// Price the session against a different model's rates.
    pub fn set_price(&mut self, price: micro_models::ModelCost) {
        self.price = Some(price);
    }

    pub fn set_tui_mode(&mut self, mode: crate::TuiMode) {
        self.tui_mode = mode;
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.cache.shape = None;
        self.refresh_inspection_body();
    }

    pub fn set_thinking(&mut self, level: ThinkingLevel) {
        self.thinking = level;
    }

    /// The colour reasoning effort is marked in, drawn on the input's rules.
    pub fn thinking_color(&self) -> Color {
        match self.thinking {
            ThinkingLevel::Off => self.theme.thinking_off,
            ThinkingLevel::Minimal => self.theme.thinking_low,
            ThinkingLevel::Low => self.theme.thinking_low,
            ThinkingLevel::Medium => self.theme.thinking_medium,
            ThinkingLevel::High => self.theme.thinking_high,
            ThinkingLevel::XHigh => self.theme.thinking_high,
            ThinkingLevel::Max => self.theme.thinking_high,
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
        self.refresh_inspection_body();
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
        &self.cache.rendered.lines
    }

    /// The conversation the terminal should keep, which the live region no longer needs.
    pub fn take_scrolled_out(&mut self) -> Vec<Line<'static>> {
        if self.turn.is_some() {
            return Vec::new();
        }
        let settled = self.cache.rendered.lines.len();
        if settled <= self.handed_over {
            return Vec::new();
        }
        let taken = self.cache.rendered.lines[self.handed_over..settled].to_vec();
        self.handed_over = settled;
        taken
    }

    pub fn forget_scrolled_out(&mut self) {
        self.handed_over = 0;
    }

    pub fn links(&self) -> &Links {
        &self.cache.rendered.links
    }

    pub fn pictures(&self) -> &Pictures {
        &self.cache.rendered.pictures
    }

    /// Say where the images go on the frame that has just been laid out.
    pub fn set_placements(&mut self, placements: Vec<Placement>) {
        self.placements = placements;
    }

    /// Where the images go on the frame that has just been laid out.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
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

    /// The built-in editor's text as it stands right now.
    pub fn editor_text(&self) -> String {
        self.editor.text()
    }

    pub fn key_prompt(&self) -> Option<&KeyPrompt> {
        self.key_prompt.as_ref()
    }

    /// Whether something is holding the keyboard.
    pub fn overlay_is_open(&self) -> bool {
        self.key_prompt.is_some()
            || self.picker.is_some()
            || self.component_overlay.is_some()
            || self.inspection.is_some()
            || self.extension_editor.is_some()
    }

    pub fn is_running(&self) -> bool {
        self.turn.is_some()
    }

    pub fn is_interrupting(&self) -> bool {
        self.turn.as_ref().is_some_and(|turn| turn.interrupting)
    }

    pub fn elapsed(&self) -> Duration {
        self.turn
            .as_ref()
            .map(|turn| turn.started.elapsed())
            .unwrap_or_default()
    }

    /// What the activity line calls what is happening.
    pub fn activity(&self) -> String {
        match &self.working_message {
            Some(message) => message.clone(),
            None => self
                .turn
                .as_ref()
                .map(|turn| turn.label)
                .unwrap_or("working")
                .to_string(),
        }
    }

    /// Whether the activity line is drawn at all right now.
    pub fn working_visible(&self) -> bool {
        self.working_visible
    }

    pub fn indicator_frame(&self) -> &str {
        match &self.working_indicator {
            Some(indicator) if indicator.frames.is_empty() => "",
            Some(indicator) => {
                let interval = indicator.interval_ms.max(1) as u128;
                let index =
                    (self.elapsed().as_millis() / interval) as usize % indicator.frames.len();
                &indicator.frames[index]
            }
            None => crate::render::status::spinner_frame(self.tick),
        }
    }

    /// The label a folded reasoning block collapses to.
    pub fn hidden_thinking_label(&self) -> &str {
        self.hidden_thinking_label
            .as_deref()
            .unwrap_or("Thinking...")
    }

    /// Lines an extension asked shown above the input, each already cut to [`MAX_WIDGET_LINES`]
    /// with a note when it was.
    pub fn widgets_above(&self) -> Vec<Vec<String>> {
        self.widgets_for(WidgetPlacement::Above)
    }

    /// Lines an extension asked shown below the input, each already cut to [`MAX_WIDGET_LINES`]
    /// with a note when it was.
    pub fn widgets_below(&self) -> Vec<Vec<String>> {
        self.widgets_for(WidgetPlacement::Below)
    }

    fn widgets_for(&self, placement: WidgetPlacement) -> Vec<Vec<String>> {
        self.widgets
            .values()
            .filter(|widget| widget.placement == placement)
            .map(|widget| match widget.lines.len() > MAX_WIDGET_LINES {
                true => {
                    let mut shown: Vec<String> = widget.lines[..MAX_WIDGET_LINES].to_vec();
                    shown.push("... (widget truncated)".to_string());
                    shown
                }
                false => widget.lines.clone(),
            })
            .collect()
    }

    /// Whether an extension is listening to every key before the interface itself acts on it.
    pub fn wants_terminal_input(&self) -> bool {
        self.wants_terminal_input
    }

    /// A terminal title an extension asked for since this was last called.
    pub fn take_title_change(&mut self) -> Option<String> {
        self.title_change.take()
    }

    pub fn header_override(&self) -> Option<&[String]> {
        self.header_override.as_deref()
    }

    pub fn footer_override(&self) -> Option<&[String]> {
        self.footer_override.as_deref()
    }

    pub fn component_overlay_id(&self) -> Option<&str> {
        self.component_overlay
            .as_ref()
            .map(|overlay| overlay.component_id.as_str())
    }

    /// The lines a `custom()` overlay is currently drawn with.
    pub fn component_overlay_lines(&self) -> Option<&[String]> {
        self.component_overlay
            .as_ref()
            .map(|overlay| overlay.lines.as_slice())
    }

    /// The hardware cursor requested by the open component, measured in its rendered lines.
    pub fn component_overlay_cursor(&self) -> Option<(usize, usize)> {
        self.component_overlay
            .as_ref()
            .and_then(|overlay| overlay.cursor)
    }

    pub fn inspection(&self) -> Option<Inspection<'_>> {
        self.inspection.as_ref().map(|overlay| {
            (
                overlay.title.as_str(),
                overlay.text.as_str(),
                overlay.items.as_slice(),
                overlay.body.as_slice(),
                overlay.selected,
                overlay.detail_open,
                overlay.scroll,
            )
        })
    }

    pub fn open_inspection(&mut self, title: String, text: String, items: Vec<InspectionItem>) {
        self.inspection = Some(InspectionOverlay {
            title,
            text,
            items,
            selected: 0,
            detail_open: false,
            scroll: 0,
            body: Vec::new(),
            body_width: 0,
            body_detail: None,
        });
        self.refresh_inspection_body();
    }

    /// Wrap the open inspection's scrollable text once, rather than once per frame.
    fn refresh_inspection_body(&mut self) {
        let Some(overlay) = self.inspection.as_mut() else {
            return;
        };
        let detail = overlay.detail_open.then_some(overlay.selected);
        if overlay.body_width == self.width && overlay.body_detail == detail {
            return;
        }
        let text = match detail {
            Some(selected) => overlay
                .items
                .get(selected)
                .map(|item| item.detail.as_str())
                .unwrap_or_default(),
            None => overlay.text.as_str(),
        };
        let style = self.theme.body();
        overlay.body.clear();
        for source in text.lines() {
            overlay.body.extend(crate::wrap::wrap_spans(
                &[Span::styled(source.to_string(), style)],
                self.width,
                2,
            ));
        }
        overlay.body_width = self.width;
        overlay.body_detail = detail;
    }

    /// Redraw the open `custom()` overlay with what its component looked like after handling a key.
    pub fn set_component_overlay_lines(&mut self, component_id: &str, lines: Vec<String>) {
        if let Some(overlay) = self.component_overlay.as_mut() {
            if overlay.component_id == component_id {
                (overlay.lines, overlay.cursor) = strip_cursor_marker(lines);
            }
        }
    }

    /// The component id `setEditorComponent` replaced the built-in editor with.
    pub fn editor_component_id(&self) -> Option<&str> {
        self.editor_component
            .as_ref()
            .map(|component| component.component_id.as_str())
    }

    /// What the editor's replacement is currently drawn with.
    pub fn editor_component_lines(&self) -> &[String] {
        self.editor_component
            .as_ref()
            .map(|component| component.lines.as_slice())
            .unwrap_or_default()
    }

    /// The hardware cursor requested by the editor replacement, measured in its rendered lines.
    pub fn editor_component_cursor(&self) -> Option<(usize, usize)> {
        self.editor_component
            .as_ref()
            .and_then(|component| component.cursor)
    }

    /// Redraw the editor's replacement with what it looked like after handling a key.
    pub fn set_editor_component_lines(&mut self, component_id: &str, lines: Vec<String>) {
        if let Some(component) = self.editor_component.as_mut() {
            if component.component_id == component_id {
                (component.lines, component.cursor) = strip_cursor_marker(lines);
            }
        }
    }

    /// The title an extension's `editor()` opened with, so `render::overlay` can label the dialog.
    pub fn extension_editor_title(&self) -> Option<&str> {
        self.extension_editor
            .as_ref()
            .map(|overlay| overlay.title.as_str())
    }

    /// The editor an extension's `editor()` is currently driving, so `render::overlay` can draw it.
    pub fn extension_editor(&self) -> Option<&Editor> {
        self.extension_editor
            .as_ref()
            .map(|overlay| &overlay.editor)
    }

    /// Prompts waiting to be sent, with the state that made each wait.
    pub fn queued_messages(&self) -> Vec<QueuedMessage> {
        self.pending.iter().cloned().collect()
    }

    pub fn is_compacting(&self) -> bool {
        self.compacting
    }

    /// Start showing compaction progress while keeping the editor available for queued messages.
    pub fn begin_compaction(&mut self) {
        self.compacting = true;
        self.busy("compacting context");
    }

    /// Stop showing compaction progress.
    pub fn finish_compaction(&mut self) {
        self.compacting = false;
        self.idle();
    }

    /// The count of prompts waiting to be sent.
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
        self.worked = true;
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

    pub fn warn(&mut self, text: impl Into<String>) {
        self.notice(text, MessageKind::Warning);
    }

    pub fn open_picker(&mut self, choices: micro_commands::Picker) {
        self.picker = Some(Picker::new(choices));
    }

    /// The open list, for whoever is keeping it up to date.
    pub fn picker_mut(&mut self) -> Option<&mut Picker> {
        self.picker.as_mut()
    }

    /// What the persisted ledger says the complete session has cost so far.
    pub fn session_cost(&self) -> Option<f64> {
        self.session_cost
    }

    pub fn set_session_observability(
        &mut self,
        observed: Option<(Option<f64>, micro_types::Usage, micro_types::Usage)>,
    ) {
        if let Some((cost, total, last)) = observed {
            self.session_cost = cost;
            self.session_usage = Some((total, last));
        }
    }

    pub fn total_usage(&self) -> micro_types::Usage {
        self.session_usage
            .map(|(total, _)| total)
            .unwrap_or_else(|| self.transcript.total_usage())
    }

    pub fn last_usage(&self) -> micro_types::Usage {
        self.session_usage
            .map(|(_, last)| last)
            .unwrap_or_else(|| self.transcript.last_usage())
    }

    /// The provider to name in the footer, which is nothing when the model already says which one
    /// is serving it.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn footer_provider(&self) -> Option<&str> {
        let provider = self.provider.trim();
        (!provider.is_empty()).then_some(provider)
    }

    /// The tools a deactivated extension provided, taken once so they are removed once.
    pub fn take_retired_tools(&mut self) -> Vec<String> {
        std::mem::take(&mut self.retired_tools)
    }

    /// Take back everything one extension drew.
    fn drop_drawings_of(&mut self, extension: &str) {
        for key in Drawn::keys_of(&self.drawn.widgets, extension) {
            self.widgets.remove(&key);
            self.drawn.widgets.remove(&key);
            self.component_slots
                .retain(|_, slot| *slot != ComponentSlot::Widget(key.clone()));
        }
        for key in Drawn::keys_of(&self.drawn.status, extension) {
            self.extension_status.remove(&key);
            self.drawn.status.remove(&key);
        }
        if self.drawn.header.as_deref() == Some(extension) {
            self.header_override = None;
            self.drawn.header = None;
            self.component_slots
                .retain(|_, slot| *slot != ComponentSlot::Header);
        }
        if self.drawn.footer.as_deref() == Some(extension) {
            self.footer_override = None;
            self.drawn.footer = None;
            self.component_slots
                .retain(|_, slot| *slot != ComponentSlot::Footer);
        }
        if self.drawn.editor.as_deref() == Some(extension) {
            self.editor_component = None;
            self.drawn.editor = None;
        }
    }

    /// Show a question an extension asked, in whatever suits it.
    pub fn ask_question(&mut self, request: crate::ui::UiRequest) {
        match request.method.as_str() {
            "send_user_message" => {
                let mut request = request;
                self.queue_line(request.title.clone());
                request.answer(serde_json::json!({ "queued": true }));
                return;
            }

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

            "tool_call_rendered" => {
                let mut request = request;
                self.transcript.set_tool_call_render(
                    &request.title,
                    request.detail.clone().unwrap_or_default(),
                    request.options.clone(),
                );
                request.answer(serde_json::json!({}));
                return;
            }
            "tool_result_rendered" => {
                let mut request = request;
                self.transcript.set_tool_result_render(
                    &request.title,
                    request.detail.clone().unwrap_or_default(),
                    request.options.clone(),
                );
                request.answer(serde_json::json!({}));
                return;
            }

            "component_changed" => {
                let mut request = request;
                let handled = self
                    .transcript
                    .tool_component_changed(&request.title, request.options.clone());
                if !handled {
                    match self.component_slots.get(&request.title) {
                        Some(ComponentSlot::Header) => {
                            self.header_override = Some(request.options.clone());
                        }
                        Some(ComponentSlot::Footer) => {
                            self.footer_override = Some(request.options.clone());
                        }
                        Some(ComponentSlot::Widget(key)) => {
                            if let Some(widget) = self.widgets.get_mut(key) {
                                widget.lines = request.options.clone();
                            }
                        }
                        None => {}
                    }
                }
                request.answer(serde_json::json!({}));
                return;
            }

            "deactivate_extension" => {
                let mut request = request;
                self.drop_drawings_of(&request.title);
                self.retired_tools.extend(request.options.clone());
                request.answer(serde_json::json!({ "ok": true }));
                return;
            }

            "set_status" => {
                let mut request = request;
                match request.detail.clone() {
                    Some(text) if !text.trim().is_empty() => {
                        self.extension_status.insert(request.title.clone(), text);
                        Drawn::by(
                            &mut self.drawn.status,
                            &request.title,
                            request.extension.as_ref(),
                        );
                    }
                    _ => {
                        self.extension_status.remove(&request.title);
                        Drawn::by(&mut self.drawn.status, &request.title, None);
                    }
                }
                request.answer(serde_json::json!({ "ok": true }));
                return;
            }
            "select" | "sandbox_access" => {
                let items = request
                    .options
                    .iter()
                    .map(|option| {
                        micro_commands::PickerItem::new(
                            option.clone(),
                            String::new(),
                            option.clone(),
                        )
                    })
                    .collect();
                self.open_picker(
                    micro_commands::Picker::new(request.title.clone(), items).titled(),
                );
            }
            "confirm" => {
                let detail = request.detail.clone().unwrap_or_default();
                let items = vec![
                    micro_commands::PickerItem::new("Yes", detail.clone(), "yes"),
                    micro_commands::PickerItem::new("No", detail, "no"),
                ];
                self.open_picker(
                    micro_commands::Picker::new(request.title.clone(), items).titled(),
                );
            }

            "editor" => {
                let mut editor = Editor::new();
                if let Some(prefill) = request.detail.clone() {
                    editor.set_text(&prefill);
                }
                self.extension_editor = Some(ExtensionEditorOverlay {
                    title: request.title.clone(),
                    editor,
                });
            }

            "custom" => {
                let (lines, cursor) = strip_cursor_marker(request.options.clone());
                self.component_overlay = Some(ComponentOverlay {
                    component_id: request.title.clone(),
                    lines,
                    cursor,
                });
            }

            "custom_done" => {
                let mut request = request;
                self.component_overlay = None;
                if let Some(mut question) = self.question.take() {
                    let value: serde_json::Value = request
                        .detail
                        .as_deref()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or(serde_json::Value::Null);
                    question.answer(serde_json::json!({ "value": value }));
                }
                request.answer(serde_json::json!({}));
                return;
            }

            "set_title" => {
                let mut request = request;
                self.title_change = Some(request.title.clone());
                request.answer(serde_json::json!({}));
                return;
            }
            "set_working_message" => {
                let mut request = request;
                self.working_message = request.detail.clone();
                request.answer(serde_json::json!({}));
                return;
            }
            "set_working_visible" => {
                let mut request = request;
                self.working_visible = request.title == "true";
                request.answer(serde_json::json!({}));
                return;
            }

            "set_working_indicator" => {
                let mut request = request;
                self.working_indicator = match request.title.as_str() {
                    "reset" => None,
                    _ => Some(WorkingIndicator {
                        frames: request.options.clone(),
                        interval_ms: request
                            .detail
                            .as_deref()
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(80),
                    }),
                };
                request.answer(serde_json::json!({}));
                return;
            }
            "set_hidden_thinking_label" => {
                let mut request = request;
                self.hidden_thinking_label = request.detail.clone();
                request.answer(serde_json::json!({}));
                return;
            }

            "set_widget" => {
                let mut request = request;
                match request.options.is_empty() {
                    true => {
                        self.widgets.remove(&request.title);
                        Drawn::by(&mut self.drawn.widgets, &request.title, None);
                    }
                    false => {
                        let placement = match request.detail.as_deref() {
                            Some("belowEditor") => WidgetPlacement::Below,
                            _ => WidgetPlacement::Above,
                        };
                        self.widgets.insert(
                            request.title.clone(),
                            Widget {
                                lines: request.options.clone(),
                                placement,
                            },
                        );
                        Drawn::by(
                            &mut self.drawn.widgets,
                            &request.title,
                            request.extension.as_ref(),
                        );
                    }
                }
                request.answer(serde_json::json!({}));
                return;
            }

            "set_header" => {
                let mut request = request;
                self.header_override =
                    (!request.options.is_empty()).then(|| request.options.clone());
                self.drawn.header = self
                    .header_override
                    .is_some()
                    .then(|| request.extension.clone())
                    .flatten();
                request.answer(serde_json::json!({}));
                return;
            }
            "set_footer" => {
                let mut request = request;
                self.footer_override =
                    (!request.options.is_empty()).then(|| request.options.clone());
                self.drawn.footer = self
                    .footer_override
                    .is_some()
                    .then(|| request.extension.clone())
                    .flatten();
                request.answer(serde_json::json!({}));
                return;
            }

            "register_component_slot" => {
                let mut request = request;
                if let Some(slot) = component_slot_named(request.detail.as_deref()) {
                    self.component_slots.retain(|_, existing| *existing != slot);
                    self.component_slots.insert(request.title.clone(), slot);
                }
                request.answer(serde_json::json!({}));
                return;
            }
            "set_editor_text" => {
                let mut request = request;
                self.editor
                    .set_text(request.detail.as_deref().unwrap_or_default());
                request.answer(serde_json::json!({}));
                return;
            }

            "paste_to_editor" => {
                let mut request = request;
                self.editor
                    .paste(request.detail.as_deref().unwrap_or_default());
                request.answer(serde_json::json!({}));
                return;
            }

            "set_editor_component" => {
                let mut request = request;
                let (lines, cursor) = strip_cursor_marker(request.options.clone());
                self.editor_component = (!request.title.is_empty()).then(|| ComponentOverlay {
                    component_id: request.title.clone(),
                    lines,
                    cursor,
                });
                self.drawn.editor = self
                    .editor_component
                    .is_some()
                    .then(|| request.extension.clone())
                    .flatten();
                request.answer(serde_json::json!({}));
                return;
            }

            "set_theme" => {
                let mut request = request;
                let colors = request
                    .detail
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
                let resolved = match colors {
                    Some(colors) => {
                        let wrapped =
                            serde_json::json!({ "name": request.title, "colors": colors })
                                .to_string();
                        Theme::from_json(&wrapped)
                    }
                    None => Theme::named(&request.title)
                        .or_else(|| Theme::from_user_file(&request.title).ok())
                        .ok_or_else(|| format!("no theme named {}", request.title)),
                };
                match resolved {
                    Ok(theme) => {
                        self.set_theme(theme);
                        request.answer(serde_json::json!({ "ok": true }));
                    }
                    Err(error) => {
                        request.answer(serde_json::json!({ "ok": false, "error": error }))
                    }
                }
                return;
            }
            "set_tools_expanded" => {
                let mut request = request;
                self.transcript.set_all_expanded(request.title == "true");
                request.answer(serde_json::json!({}));
                return;
            }

            "abort" => {
                let mut request = request;
                let interrupted = matches!(self.interrupt(), Outcome::Interrupt);
                request.answer(serde_json::json!({ "interrupted": interrupted }));
                return;
            }

            "watch_terminal_input" => {
                let mut request = request;
                self.wants_terminal_input = true;
                request.answer(serde_json::json!({}));
                return;
            }
            "unwatch_terminal_input" => {
                let mut request = request;
                self.wants_terminal_input = false;
                request.answer(serde_json::json!({}));
                return;
            }

            "watch_autocomplete" => {
                let mut request = request;
                self.autocomplete_triggers = request
                    .options
                    .iter()
                    .filter_map(|trigger| trigger.chars().next())
                    .collect();
                request.answer(serde_json::json!({}));
                return;
            }

            _ => self.open_input(request.title.clone(), request.detail.clone()),
        }
        self.question = Some(request);
    }

    /// Ask for a credential.
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
        if let Some(line) = self.injected.pop_front() {
            return Some(line);
        }
        if !self.settings.steer_all_at_once || self.pending.len() < 2 {
            return self.pending.pop_front().map(|message| message.text);
        }
        let all: Vec<String> = self.pending.drain(..).map(|message| message.text).collect();
        Some(all.join("\n\n"))
    }

    /// Send a line as though the user had typed it.
    pub fn queue_line(&mut self, line: impl Into<String>) {
        self.injected.push_back(line.into());
    }

    /// Queue a line after the active turn, as though it was entered with the follow-up shortcut.
    pub fn queue_follow_up_line(&mut self, line: impl Into<String>) -> Outcome {
        let line = line.into();
        if line.trim().is_empty() {
            return Outcome::Handled;
        }
        self.pending.push_back(QueuedMessage {
            text: line,
            kind: QueueKind::FollowUp,
        });
        match self.settings.follow_up_interrupts && self.is_running() && !self.compacting {
            true => self.interrupt(),
            false => Outcome::Handled,
        }
    }

    /// Record a command the user ran themselves.
    pub fn push_bash(&mut self, command: &str, shared: bool) {
        self.transcript.push_bash(command, shared);
    }

    /// Take a submitted line and make the message the agent will be given.
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
    pub fn finish_turn(&mut self, aborted: bool) {
        self.transcript.close();
        self.turn = None;
        if aborted {
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

    /// A turn that wrote the cache without reading any of it paid twice for a context it already
    /// had.
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
            Applied::RunGrantedCommand { command } => self.queue_line(format!("!{command}")),
            Applied::Nothing => {}

            Applied::Model { .. } | Applied::SystemPrompt { .. } => {}
            Applied::Note { text, kind } => self.notice(text, kind),
            Applied::Conversation { messages, note } => {
                self.transcript = Transcript::from_messages(&messages);
                self.transcript
                    .set_self_framed_tools(self.self_framed_tools.clone());
                self.cache.shape = None;
                self.focus = None;

                self.scroll = 0;
                if let Some(note) = note {
                    self.notice(note, MessageKind::Info);
                }
            }
        }
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
        let default = format!("micro-conversation-{}.md", micro_types::now_ms() / 1000);
        let target = self.workspace.join(path.unwrap_or(&default));

        let mut out = String::new();
        for entry in self.transcript.entries() {
            match entry {
                crate::transcript::Entry::User(text) => {
                    out.push_str(&format!("## Prompt\n\n{text}\n\n"))
                }
                crate::transcript::Entry::Bash { command, .. } => {
                    out.push_str(&format!("## Command\n\n```\n$ {command}\n```\n\n"))
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

    /// Wrap the transcript for the frame about to be drawn, reusing the last frame's rows when
    /// nothing that affects them has changed.
    pub fn refresh_lines(&mut self) {
        let shape = Shape {
            width: self.width,
            show_thinking: self.show_thinking,
            focus: self.focus,
        };

        let from = match self.cache.shape == Some(shape) {
            true => self.transcript.dirty_from().min(self.cache.starts.len()),
            false => 0,
        };
        if self.cache.shape == Some(shape) && from >= self.transcript.entries().len() {
            self.transcript.settled();
            return;
        }

        let was = self.cache.rendered.lines.len();

        let kept_rows = self
            .cache
            .starts
            .get(from)
            .copied()
            .unwrap_or(self.cache.rendered.lines.len());
        let kept_links = self
            .cache
            .links_from
            .get(from)
            .copied()
            .unwrap_or(self.cache.rendered.links.len());
        let kept_pictures = self
            .cache
            .pictures_from
            .get(from)
            .copied()
            .unwrap_or(self.cache.rendered.pictures.len());
        let mut rendered = match from {
            0 => Rendered {
                lines: Vec::new(),
                links: match self.hyperlinks {
                    true => Links::new(),
                    false => Links::disabled(),
                },
                pictures: Pictures::new(self.images)
                    .sized(
                        self.settings.image_width_cells as usize,
                        self.settings.auto_resize_images,
                    )
                    .indented(crate::render::transcript::PADDING),
            },
            _ => {
                let mut kept = std::mem::take(&mut self.cache.rendered);
                kept.lines.truncate(kept_rows);
                kept.links.truncate(kept_links);
                kept.pictures.truncate(kept_pictures);
                kept
            }
        };
        self.cache.starts.truncate(from);
        self.cache.links_from.truncate(from);
        self.cache.pictures_from.truncate(from);

        crate::render::transcript::append(
            &self.transcript,
            &self.theme,
            &Display {
                width: self.width,
                show_thinking: self.show_thinking,
                focus: self.focus,
                from,
                hyperlinks: self.hyperlinks,
                images: self.images,
                image_width: self.settings.image_width_cells as usize,
                resize_images: self.settings.auto_resize_images,
                mermaid: self.settings.mermaid,
                hidden_thinking_label: std::borrow::Cow::Owned(
                    self.hidden_thinking_label().to_string(),
                ),
            },
            &mut rendered,
            &mut self.cache.starts,
        );

        while self.cache.links_from.len() < self.cache.starts.len() {
            self.cache.links_from.push(rendered.links.len());
            self.cache.pictures_from.push(rendered.pictures.len());
        }

        let grew = rendered.lines.len().saturating_sub(was);
        self.cache.shape = Some(shape);
        self.cache.rendered = rendered;
        if self.scroll > 0 {
            self.scroll += grew;
        }
        self.transcript.settled();
        self.clamp_scroll();
    }

    /// Answer one action.
    pub fn handle(&mut self, action: Action) -> Outcome {
        if !matches!(action, Action::Cancel) {
            self.escape_pending = false;
        }

        if !matches!(action, Action::Interrupt | Action::Resize | Action::Ignored) {
            self.quitting = false;
        }

        if self.key_prompt.is_some() {
            return self.handle_key_prompt(action);
        }
        if self.picker.is_some() {
            return self.handle_picker(action);
        }
        if self.component_overlay.is_some() {
            return self.handle_component_overlay(action);
        }
        if self.inspection.is_some() {
            return self.handle_inspection(action);
        }
        if self.extension_editor.is_some() {
            return self.handle_extension_editor(action);
        }

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
                self.cache.shape = None;
                Outcome::Handled
            }

            Action::QuitOrDelete => {
                if self.editor.is_empty() {
                    Outcome::Quit
                } else {
                    self.edit(|editor| editor.delete())
                }
            }
            Action::Interrupt => self.interrupt(),

            Action::Submit => self.submit(),
            Action::QueueFollowUp => self.queue_follow_up(),
            Action::Dequeue => {
                if let Some(message) = self.pending.pop_back() {
                    self.editor.set_text(&message.text);
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
            Action::ScrollUp => {
                self.scroll_by(3);
                Outcome::Handled
            }
            Action::ScrollDown => {
                self.scroll_by(-3);
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

    /// Ctrl+C.
    fn interrupt(&mut self) -> Outcome {
        if self.compacting {
            return Outcome::Interrupt;
        }
        if let Some(turn) = self.turn.as_mut() {
            turn.interrupting = true;
            return Outcome::Interrupt;
        }
        if !self.editor.is_empty() {
            self.editor.clear();
            self.menu = None;
            return Outcome::Handled;
        }
        if self.quitting {
            return Outcome::Quit;
        }
        self.quitting = true;
        self.notice("Press ctrl+c again to exit", MessageKind::Info);
        Outcome::Handled
    }

    /// Enter.
    fn submit(&mut self) -> Outcome {
        if self.queue_extension_completion() {
            return Outcome::Handled;
        }

        if self.completion_would_change_the_line() && self.commit_completion() {
            return Outcome::Handled;
        }

        if self.editor.escapes_submit() {
            self.editor.escape_newline();
            return Outcome::Handled;
        }

        let line = self.editor.take();
        self.menu = None;
        if line.trim().is_empty() {
            return Outcome::Handled;
        }
        self.pending.push_back(QueuedMessage {
            text: line,
            kind: QueueKind::Steering,
        });
        self.scroll = 0;
        Outcome::Handled
    }

    fn queue_follow_up(&mut self) -> Outcome {
        let line = self.editor.take();
        self.menu = None;
        if line.trim().is_empty() {
            return Outcome::Handled;
        }
        self.pending.push_back(QueuedMessage {
            text: line,
            kind: QueueKind::FollowUp,
        });

        match self.settings.follow_up_interrupts && self.is_running() && !self.compacting {
            true => self.interrupt(),
            false => Outcome::Handled,
        }
    }

    /// Tab takes the highlighted completion; with nothing offering one it indents.
    fn complete_or_indent(&mut self) -> Outcome {
        if self.queue_extension_completion() {
            return Outcome::Handled;
        }
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

    /// Take the highlighted completion, replacing what was typed toward it and leaving anything
    /// past the cursor where it is.
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

    fn move_up(&mut self) -> Outcome {
        if let Some(menu) = self.menu.as_mut() {
            menu.select_previous();
            return Outcome::Handled;
        }
        if self.editor.move_up(self.width) {
            return Outcome::Handled;
        }

        if self.editor.is_empty() && !self.editor.is_browsing_history() && self.scroll_by(1) {
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
        if self.editor.is_empty() && !self.editor.is_browsing_history() && self.scroll_by(-1) {
            return Outcome::Handled;
        }
        self.editor.history_next();
        Outcome::Handled
    }

    /// Escape.
    fn cancel(&mut self) -> Outcome {
        if self.menu.take().is_some() {
            self.escape_pending = false;
            return Outcome::Handled;
        }
        if self.compacting {
            return Outcome::Interrupt;
        }
        if self.is_running() {
            self.escape_pending = false;
            return self.interrupt();
        }
        if self.focus.take().is_some() {
            self.escape_pending = false;
            return Outcome::Handled;
        }
        if self.scroll != 0 {
            self.scroll = 0;
            self.escape_pending = false;
            return Outcome::Handled;
        }
        if self.escape_pending {
            self.escape_pending = false;
            if !self.editor.is_empty() {
                self.editor.clear();
                self.menu = None;
                return Outcome::Handled;
            }
            match self.settings.double_escape {
                crate::commands::DoubleEscape::Tree => self.queue_line("/tree"),
                crate::commands::DoubleEscape::Fork => self.queue_line("/fork"),
                crate::commands::DoubleEscape::None => {}
            }
        } else {
            self.escape_pending = true;
        }
        Outcome::Handled
    }

    /// The command menu belongs to whatever is being typed.
    fn sync_menu(&mut self) {
        let (row, column) = self.editor.cursor();
        let line = self.editor.lines().get(row).cloned().unwrap_or_default();

        if row == 0 {
            if let Some(menu) = Menu::open_for(&line, column, &self.extension_commands) {
                self.menu = Some(menu);
                return;
            }
        }

        if wants_file_menu(&line, column) {
            self.menu = Menu::files_for(&line, column, self.workspace_files());
            return;
        }

        self.menu = Menu::extension_for(&line, column, &self.autocomplete_triggers);
        if let Some(menu) = &self.menu {
            self.pending_suggestion_request = Some(SuggestionRequest {
                lines: self.editor.lines().to_vec(),
                cursor_line: row,
                cursor_col: char_offset(&line, column),
                prefix: menu.prefix().to_string(),
            });
        }
    }

    /// The `getSuggestions` question `sync_menu` raised, if any, and only once.
    pub fn take_pending_suggestion_request(&mut self) -> Option<SuggestionRequest> {
        self.pending_suggestion_request.take()
    }

    /// Fill the open extension menu with what `getSuggestions` answered, when it still belongs to
    /// the prefix it was asked about.
    pub fn apply_extension_suggestions(&mut self, prefix: &str, items: Vec<serde_json::Value>) {
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let items = items
            .into_iter()
            .filter_map(crate::menu::MenuItem::from_extension_item)
            .collect();
        menu.set_extension_items(prefix, items);
    }

    /// Queue the `applyCompletion` question committing an open extension menu's highlighted item
    /// raises.
    fn queue_extension_completion(&mut self) -> bool {
        let Some(menu) = self.menu.as_ref() else {
            return false;
        };
        if menu.offering() != crate::menu::Offering::Extension {
            return false;
        }
        let Some(item) = menu.selected_item().and_then(|item| item.raw.clone()) else {
            return false;
        };
        let (row, column) = self.editor.cursor();
        let line = self.editor.lines().get(row).cloned().unwrap_or_default();
        self.pending_completion_request = Some(CompletionRequest {
            lines: self.editor.lines().to_vec(),
            cursor_line: row,
            cursor_col: char_offset(&line, column),
            item,
            prefix: menu.prefix().to_string(),
        });
        true
    }

    /// The `applyCompletion` question committing an extension's menu item raised, if any, and only
    /// once.
    pub fn take_pending_completion_request(&mut self) -> Option<CompletionRequest> {
        self.pending_completion_request.take()
    }

    pub fn apply_extension_completion(
        &mut self,
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
    ) {
        self.menu = None;
        let text = lines.join("\n");
        let row = cursor_line.min(lines.len().saturating_sub(1));
        let col = lines
            .get(row)
            .map(|line| byte_offset(line, cursor_col))
            .unwrap_or(0);
        self.editor.set_text_with_cursor(&text, row, col);
        self.sync_menu();
    }

    fn workspace_files(&mut self) -> &[String] {
        if self.file_index.is_none() {
            self.file_index = Some(walk_workspace(&self.workspace));
        }
        self.file_index.as_deref().unwrap_or_default()
    }

    /// Forget the file listing, so the next completion walks the workspace again.
    pub fn forget_workspace_files(&mut self) {
        self.file_index = None;
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

            Action::QuitOrDelete => return Outcome::Quit,
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

            Action::Tab => picker.toggle_scope(),
            Action::Insert(text) => picker.push(&text),
            Action::Backspace => picker.backspace(),
            Action::Submit => {
                let chosen = picker.commit();
                self.picker = None;
                match self.question.take() {
                    Some(mut question) => {
                        let answer = match (question.method.as_str(), chosen.as_deref()) {
                            ("confirm", Some(said)) => {
                                serde_json::json!({ "confirmed": said == "yes" })
                            }
                            ("sandbox_access", Some(said)) => {
                                serde_json::json!({ "value": said })
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

            Action::QuitOrDelete => return Outcome::Quit,
            _ => {}
        }
        Outcome::Handled
    }

    /// Back out of a `custom()` overlay.
    fn handle_component_overlay(&mut self, action: Action) -> Outcome {
        match action {
            Action::Cancel | Action::Interrupt => {
                self.component_overlay = None;
                if let Some(mut question) = self.question.take() {
                    question.cancel();
                }
                Outcome::Handled
            }

            Action::QuitOrDelete => Outcome::Quit,
            _ => Outcome::Handled,
        }
    }

    fn handle_inspection(&mut self, action: Action) -> Outcome {
        if matches!(action, Action::Cancel | Action::Interrupt) {
            if self
                .inspection
                .as_ref()
                .is_some_and(|overlay| overlay.detail_open)
            {
                if let Some(overlay) = self.inspection.as_mut() {
                    overlay.detail_open = false;
                    overlay.scroll = 0;
                }
            } else {
                self.inspection = None;
            }
            self.refresh_inspection_body();
            return Outcome::Handled;
        }
        if matches!(action, Action::QuitOrDelete) {
            return Outcome::Quit;
        }
        let page = self.rows as usize / 2;
        let Some(overlay) = self.inspection.as_mut() else {
            return Outcome::Handled;
        };
        if !overlay.items.is_empty() && !overlay.detail_open {
            let mut body_changed = false;
            match action {
                Action::MoveUp | Action::ScrollUp => {
                    overlay.selected = overlay.selected.saturating_sub(1)
                }
                Action::MoveDown | Action::ScrollDown => {
                    overlay.selected = (overlay.selected + 1).min(overlay.items.len() - 1)
                }
                Action::PageUp => overlay.selected = overlay.selected.saturating_sub(page),
                Action::PageDown => {
                    overlay.selected = (overlay.selected + page).min(overlay.items.len() - 1)
                }
                Action::MoveLineStart => overlay.selected = 0,
                Action::MoveLineEnd => overlay.selected = overlay.items.len() - 1,
                Action::Submit => {
                    overlay.detail_open = true;
                    overlay.scroll = 0;
                    body_changed = true;
                }
                _ => {}
            }
            if body_changed {
                self.refresh_inspection_body();
            }
            return Outcome::Handled;
        }
        match action {
            Action::MoveUp | Action::ScrollUp => overlay.scroll = overlay.scroll.saturating_sub(1),
            Action::MoveDown | Action::ScrollDown => {
                overlay.scroll = overlay.scroll.saturating_add(1)
            }
            Action::PageUp => overlay.scroll = overlay.scroll.saturating_sub(page),
            Action::PageDown => overlay.scroll = overlay.scroll.saturating_add(page),
            Action::MoveLineStart => overlay.scroll = 0,
            _ => {}
        }
        Outcome::Handled
    }

    /// Drive an extension's `editor()` overlay.
    fn handle_extension_editor(&mut self, action: Action) -> Outcome {
        let Some(overlay) = self.extension_editor.as_mut() else {
            return Outcome::Handled;
        };
        match action {
            Action::Insert(text) => overlay.editor.insert_str(&text),
            Action::Paste(text) => overlay.editor.paste(&text),
            Action::Newline => overlay.editor.insert_newline(),
            Action::Backspace => overlay.editor.backspace(),
            Action::Delete => overlay.editor.delete(),
            Action::DeleteWordBefore => overlay.editor.delete_word_before(),
            Action::DeleteWordAfter => overlay.editor.delete_word_after(),
            Action::DeleteToLineStart => overlay.editor.delete_to_line_start(),
            Action::DeleteToLineEnd => overlay.editor.delete_to_line_end(),
            Action::MoveLeft => overlay.editor.move_left(),
            Action::MoveRight => overlay.editor.move_right(),
            Action::MoveWordLeft => overlay.editor.move_word_left(),
            Action::MoveWordRight => overlay.editor.move_word_right(),
            Action::MoveLineStart => overlay.editor.move_line_start(),
            Action::MoveLineEnd => overlay.editor.move_line_end(),
            Action::MoveUp => {
                overlay.editor.move_up(self.width);
            }
            Action::MoveDown => {
                overlay.editor.move_down(self.width);
            }
            Action::Undo => {
                overlay.editor.undo();
            }
            Action::Yank => overlay.editor.yank(),
            Action::YankPop => {
                overlay.editor.yank_pop();
            }
            Action::Submit => {
                let text = overlay.editor.text();
                self.extension_editor = None;
                if let Some(mut question) = self.question.take() {
                    question.answer(serde_json::json!({ "value": text }));
                }
            }
            Action::Cancel | Action::Interrupt => {
                self.extension_editor = None;
                if let Some(mut question) = self.question.take() {
                    question.cancel();
                }
            }

            Action::QuitOrDelete => return Outcome::Quit,
            _ => {}
        }
        Outcome::Handled
    }

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
        self.cache.shape = None;
        Outcome::Handled
    }

    /// Open or close what is selected; with nothing selected, everything at once.
    fn toggle_focused(&mut self) -> Outcome {
        match self.focus {
            Some(index) => {
                self.transcript.toggle_expanded(index);
            }
            None => {
                let opening = self.transcript.any_collapsed() || !self.startup_expanded;
                self.transcript.set_all_expanded(opening);
                self.startup_expanded = opening;
            }
        }
        Outcome::Handled
    }

    /// The conversation as it was drawn, as plain rows.
    pub fn plain_lines(&mut self) -> Vec<String> {
        self.refresh_lines();
        self.cache
            .rendered
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// What was loaded before the session started, for the first screen.
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Whether the first screen is showing everything it knows.
    pub fn startup_expanded(&self) -> bool {
        self.startup_expanded
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

    /// Move the window back through the conversation.
    fn scroll_by(&mut self, lines: isize) -> bool {
        let before = self.scroll;
        let scroll = self.scroll as isize + lines;
        self.scroll = scroll.max(0) as usize;
        self.clamp_scroll();
        self.scroll != before
    }

    /// Scrolling stops at the first line: past it there is nothing to show, and the window would
    /// drift off the top of the conversation.
    fn clamp_scroll(&mut self) {
        let furthest = self
            .cache
            .rendered
            .lines
            .len()
            .saturating_sub(self.viewport);
        self.scroll = self.scroll.min(furthest);
    }
}

/// The next reasoning level, wrapping at the top.
fn next_level(level: ThinkingLevel) -> ThinkingLevel {
    match level {
        ThinkingLevel::Off => ThinkingLevel::Minimal,
        ThinkingLevel::Minimal => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::XHigh,
        ThinkingLevel::XHigh => ThinkingLevel::Max,
        ThinkingLevel::Max => ThinkingLevel::Off,
    }
}

/// What a reasoning level is called where a user reads it.
pub fn thinking_name(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
        ThinkingLevel::Max => "max",
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

/// Whether the word under the cursor is asking for a file.
fn wants_file_menu(line: &str, cursor: usize) -> bool {
    let Some(typed) = line.get(..cursor) else {
        return false;
    };
    let start = typed
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    typed.get(start..).is_some_and(|word| word.starts_with('@'))
}

/// A byte offset into `line`, in characters instead.
fn char_offset(line: &str, byte_col: usize) -> usize {
    line.get(..byte_col)
        .map(|typed| typed.chars().count())
        .unwrap_or_else(|| line.chars().count())
}

/// The byte offset in `line` that is `char_col` characters in.
fn byte_offset(line: &str, char_col: usize) -> usize {
    line.char_indices()
        .nth(char_col)
        .map(|(index, _)| index)
        .unwrap_or(line.len())
}

/// How many files are worth offering to complete against.
const MAX_INDEXED_FILES: usize = 20_000;

/// Every file in the workspace, as paths relative to it.
fn walk_workspace(root: &std::path::Path) -> Vec<String> {
    let mut paths = BTreeSet::new();
    collect_workspace_paths(root, true, false, &mut paths);
    // A document is useful conversational context even when its repository ignores it. Keeping
    // this pass document-only avoids turning an ignored build tree into completion candidates.
    collect_workspace_paths(root, false, true, &mut paths);

    paths.into_iter().take(MAX_INDEXED_FILES).collect()
}

fn collect_workspace_paths(
    root: &std::path::Path,
    use_git_ignores: bool,
    documents_only: bool,
    paths: &mut BTreeSet<String>,
) {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(use_git_ignores)
        .git_exclude(use_git_ignores)
        .git_global(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git");

    for entry in builder.build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if documents_only && !is_document(entry.path()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        if let Some(path) = relative.to_str() {
            paths.insert(path.to_string());
        }
        if paths.len() >= MAX_INDEXED_FILES {
            break;
        }
    }
}

fn is_document(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("md" | "mdx" | "txt" | "rst" | "pdf" | "doc" | "docx" | "odt")
    )
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

    #[test]
    fn inspection_items_open_a_detail_before_escape_closes_the_overlay() {
        let mut app = app();
        app.open_inspection(
            "Session bill".to_string(),
            "summary".to_string(),
            vec![
                InspectionItem {
                    label: "turn 1".to_string(),
                    detail: "first".to_string(),
                },
                InspectionItem {
                    label: "turn 2".to_string(),
                    detail: "second".to_string(),
                },
            ],
        );

        app.handle(Action::MoveDown);
        app.handle(Action::Submit);
        let (_, _, _, _, selected, detail_open, _) = app.inspection().unwrap();
        assert_eq!(selected, 1);
        assert!(detail_open);

        app.handle(Action::Cancel);
        assert!(!app.inspection().unwrap().5);
        app.handle(Action::Cancel);
        assert!(app.inspection().is_none());
    }

    #[test]
    fn inspection_navigation_stops_at_the_last_item() {
        let mut app = app();
        let items = (0..40)
            .map(|index| InspectionItem {
                label: format!("turn {index}"),
                detail: format!("detail {index}"),
            })
            .collect();
        app.open_inspection("Session bill".to_string(), "summary".to_string(), items);

        for _ in 0..100 {
            app.handle(Action::MoveDown);
        }

        let (_, _, items, _, selected, detail_open, _) = app.inspection().expect("still open");
        assert_eq!(selected, items.len() - 1);
        assert!(!detail_open);
    }

    fn press_up_until_history(app: &mut App) {
        for _ in 0..500 {
            app.handle(Action::MoveUp);
            if !app.editor.text().is_empty() {
                return;
            }
        }
        panic!("up never reached the history");
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
    fn a_loaded_skill_is_offered_by_the_slash_menu() {
        let mut app = App::new(
            &[],
            TuiOptions {
                extension_commands: vec![crate::menu::MenuItem {
                    value: "review-changes".to_string(),
                    description: "skill — Review the current changes.".to_string(),
                    raw: None,
                }],
                ..Default::default()
            },
        );

        type_text(&mut app, "/review");

        let item = app.menu().unwrap().items().first().unwrap();
        assert_eq!(item.value, "review-changes");
        assert!(item.description.starts_with("skill —"));
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

    fn watch_autocomplete(app: &mut App, triggers: &[&str]) {
        let (request, _answered) = crate::ui::UiRequest::for_test(
            "watch_autocomplete",
            "",
            None,
            triggers.iter().map(|trigger| trigger.to_string()).collect(),
        );
        app.ask_question(request);
    }

    /// A trigger character nobody registered opens nothing, the way `@` would if nothing under it
    /// named a file.
    #[test]
    fn an_unregistered_trigger_opens_nothing() {
        let mut app = app();
        type_text(&mut app, "#tag");
        assert!(app.menu().is_none());
    }

    /// A registered trigger opens the menu empty and queues the `getSuggestions` question the event
    /// loop owes the extension.
    #[test]
    fn a_registered_trigger_opens_an_empty_menu_and_queues_a_suggestion_request() {
        let mut app = app();
        watch_autocomplete(&mut app, &["#"]);
        type_text(&mut app, "#tag");

        assert!(app.menu().unwrap().items().is_empty());
        let request = app
            .take_pending_suggestion_request()
            .expect("a suggestion request");
        assert_eq!(request.prefix, "#tag");
        assert_eq!(request.cursor_line, 0);
        assert_eq!(request.cursor_col, 4);
        assert!(
            app.take_pending_suggestion_request().is_none(),
            "taken once"
        );
    }

    /// What `getSuggestions` answers lands in the menu the reader is still looking at, shown by its
    /// label.
    #[test]
    fn an_extension_answer_fills_the_open_menu() {
        let mut app = app();
        watch_autocomplete(&mut app, &["#"]);
        type_text(&mut app, "#tag");
        app.take_pending_suggestion_request();

        app.apply_extension_suggestions(
            "#tag",
            vec![serde_json::json!({ "value": "v1", "label": "#tagged" })],
        );

        let items = app.menu().unwrap().items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "#tagged");
    }

    /// An answer for a prefix the reader has since typed past does not land on the menu that
    /// replaced it.
    #[test]
    fn a_stale_extension_answer_is_ignored() {
        let mut app = app();
        watch_autocomplete(&mut app, &["#"]);
        type_text(&mut app, "#tag");

        app.apply_extension_suggestions(
            "#ta",
            vec![serde_json::json!({ "value": "v1", "label": "stale" })],
        );

        assert!(app.menu().unwrap().items().is_empty());
    }

    /// Enter on an extension's own item queues `applyCompletion` instead of committing a fixed
    /// splice.
    #[test]
    fn enter_on_an_extension_item_queues_apply_completion_instead_of_submitting() {
        let mut app = app();
        watch_autocomplete(&mut app, &["#"]);
        type_text(&mut app, "#tag");
        app.apply_extension_suggestions(
            "#tag",
            vec![serde_json::json!({ "value": "v1", "label": "#tagged" })],
        );

        app.handle(Action::Submit);

        assert_eq!(
            app.editor.text(),
            "#tag",
            "nothing is written until applyCompletion answers"
        );
        assert_eq!(app.queued(), 0);
        let request = app
            .take_pending_completion_request()
            .expect("a completion request");
        assert_eq!(request.prefix, "#tag");
        assert_eq!(
            request.item,
            serde_json::json!({ "value": "v1", "label": "#tagged" })
        );
    }

    /// What `applyCompletion` answers replaces the buffer and places the cursor exactly where it
    /// says to, and closes the menu.
    #[test]
    fn apply_extension_completion_writes_what_was_answered() {
        let mut app = app();
        watch_autocomplete(&mut app, &["#"]);
        type_text(&mut app, "#tag");
        app.apply_extension_suggestions(
            "#tag",
            vec![serde_json::json!({ "value": "v1", "label": "#tagged" })],
        );
        app.handle(Action::Submit);
        app.take_pending_completion_request();

        app.apply_extension_completion(vec!["#tagged ".to_string()], 0, 8);

        assert_eq!(app.editor.text(), "#tagged ");
        assert_eq!(app.editor.cursor(), (0, 8));
        assert!(app.menu().is_none());
    }

    #[test]
    fn tab_indents_when_no_command_is_being_typed() {
        let mut app = app();
        type_text(&mut app, "plain");
        app.handle(Action::Tab);
        assert_eq!(app.editor.text(), "plain\t");
    }

    /// Committing replaces what was typed toward the command, not the whole prompt: an argument
    /// already written stays where it is.
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

    /// An attached image rides in front of the text, which is the order every provider expects, and
    /// is handed over exactly once.
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
        for index in 0..20 {
            app.transcript.push_user(format!("message number {index}"));
        }
        app.begin_turn("the first thing asked");
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();

        app.handle(Action::MoveUp);
        assert!(app.scroll() > 0);
        assert_eq!(app.editor.text(), "");

        press_up_until_history(&mut app);
        assert_eq!(app.editor.text(), "the first thing asked");
    }

    #[test]
    fn interrupting_an_empty_prompt_twice_leaves() {
        let mut app = app();

        assert_eq!(app.handle(Action::Interrupt), Outcome::Handled);
        assert_eq!(app.handle(Action::Interrupt), Outcome::Quit);
    }

    /// Typing something in between takes the question back, so a stray press cannot combine with a
    /// later one to close the session.
    #[test]
    fn a_press_between_the_two_takes_the_question_back() {
        let mut app = app();

        assert_eq!(app.handle(Action::Interrupt), Outcome::Handled);
        app.handle(Action::Insert("a".into()));
        app.handle(Action::Interrupt);
        assert_eq!(
            app.handle(Action::Interrupt),
            Outcome::Handled,
            "the count starts again"
        );
    }

    /// What is written is cleared before anything else is considered.
    #[test]
    fn interrupting_clears_the_prompt_before_it_stops_anything() {
        let mut app = app();
        type_text(&mut app, "half written");
        assert_eq!(app.handle(Action::Interrupt), Outcome::Handled);
        assert!(app.editor.is_empty());

        assert_eq!(app.handle(Action::Interrupt), Outcome::Handled);
        assert_eq!(app.handle(Action::Interrupt), Outcome::Quit);
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

    /// A line the interface asked for itself goes out before anything the user typed: it is the
    /// answer to a key they just pressed.
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
            Outcome::ThinkingChanged(ThinkingLevel::Minimal)
        );
        assert_eq!(app.thinking, ThinkingLevel::Minimal);
        assert_eq!(thinking_name(app.thinking), "minimal");
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

        app.handle(Action::PageDown);
        assert_eq!(app.scroll(), 0, "the end is as far forward as it goes");

        for _ in 0..100 {
            app.handle(Action::PageUp);
        }
        let furthest = app.lines().len() - 10;
        assert_eq!(
            app.scroll(),
            furthest,
            "the start is as far back as it goes"
        );
    }

    #[test]
    fn wheel_and_arrow_keys_scroll_the_conversation() {
        let mut app = app();
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();

        app.handle(Action::ScrollUp);
        assert_eq!(app.scroll(), 3);
        app.handle(Action::ScrollUp);
        assert_eq!(app.scroll(), 6);

        app.handle(Action::MoveUp);
        assert_eq!(app.scroll(), 7);
        app.handle(Action::MoveUp);
        assert_eq!(app.scroll(), 8);

        app.handle(Action::MoveDown);
        assert_eq!(app.scroll(), 7);
        app.handle(Action::ScrollDown);
        assert_eq!(app.scroll(), 4);
    }

    #[test]
    fn a_typed_prompt_still_browses_history() {
        let mut app = app();
        app.begin_turn("the first thing asked");
        type_text(&mut app, "half written");

        app.handle(Action::MoveUp);
        assert_eq!(app.editor.text(), "the first thing asked");
        assert_eq!(app.scroll(), 0, "typing kept the arrows in history");
    }

    #[test]
    fn arrows_with_a_menu_open_still_choose_from_it() {
        let mut app = app();
        type_text(&mut app, "/c");

        app.handle(Action::MoveDown);
        assert_eq!(app.menu().unwrap().selected(), 1);
        app.handle(Action::MoveUp);
        assert_eq!(app.menu().unwrap().selected(), 0);
    }

    /// Someone reading back through the conversation stays where they are when an answer arrives
    /// beneath them.
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
    fn a_long_conversation_redraws_only_what_changed() {
        let conversation = |count: usize| {
            let mut app = App::new(&[], TuiOptions::default());
            for n in 0..count {
                app.transcript.push_user(format!("prompt {n}"));
                app.apply_event(AgentEvent::ToolStart {
                    id: format!("c{n}"),
                    name: "edit".into(),
                    arguments: serde_json::json!({
                        "path": format!("src/f{n}.rs"),
                        "old_string": "one\ntwo\nthree\n",
                        "new_string": "one\ntwo!\nthree\n",
                    }),
                });
                app.apply_event(AgentEvent::ToolEnd {
                    id: format!("c{n}"),
                    name: "edit".into(),
                    output: "Edited".into(),
                    is_error: false,
                });
            }
            app.set_frame(100, 40);
            app.refresh_lines();
            app
        };

        let mut short = conversation(5);
        let mut long = conversation(80);
        assert!(
            long.lines().len() > short.lines().len() * 5,
            "the long conversation really is longer"
        );

        let update = |app: &mut App, count: usize| {
            app.apply_event(AgentEvent::ToolUpdate {
                id: format!("c{}", count - 1),
                name: "edit".into(),
                output: "still going".into(),
            });
            let started = std::time::Instant::now();
            app.refresh_lines();
            started.elapsed()
        };

        let _ = update(&mut short, 5);
        let _ = update(&mut long, 80);
        let brief: std::time::Duration = (0..20).map(|_| update(&mut short, 5)).sum();
        let lengthy: std::time::Duration = (0..20).map(|_| update(&mut long, 80)).sum();

        assert!(
            lengthy < brief * 8,
            "redrawing a conversation of 80 took {lengthy:?} against {brief:?} for one of 5, \
             which means it is being drawn again from the beginning"
        );
    }

    #[test]
    fn a_bash_line_joins_the_conversation_where_it_was_run() {
        let mut app = app();
        app.push_bash("ls -la", true);
        assert!(transcript_text(&mut app).contains("! ls -la"));
    }

    #[test]
    fn a_command_kept_back_is_marked_as_one() {
        let mut app = app();
        app.push_bash("cat ~/.ssh/config", false);
        assert!(transcript_text(&mut app).contains("!! cat ~/.ssh/config"));
    }

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

    /// Up at the top of a multi-line prompt moves through the rows before it reaches back for what
    /// was sent earlier.
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

    /// A trailing backslash continues the line instead of sending it.
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

    /// Wrapping is the most expensive thing a frame does, so it is not redone when nothing that
    /// affects it has changed.
    #[test]
    fn the_wrapped_transcript_is_kept_between_frames() {
        let mut app = app();
        app.transcript.push_user("something to wrap");
        app.set_frame(60, 24);
        app.refresh_lines();
        let key = app.cache.shape;

        app.refresh_lines();
        assert_eq!(
            app.cache.shape, key,
            "nothing changed, so nothing rewrapped"
        );

        app.set_frame(40, 24);
        app.refresh_lines();
        assert_ne!(app.cache.shape, key, "a new width wraps again");
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

    #[test]
    fn hiding_thinking_starts_it_folded_away() {
        assert!(
            !app_with(Preferences {
                hide_thinking: true,
                ..Preferences::default()
            })
            .show_thinking
        );

        assert!(
            app_with(Preferences {
                hide_thinking: false,
                ..Preferences::default()
            })
            .show_thinking
        );
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
    fn double_escape_clears_written_input() {
        let mut app = app_with(Preferences {
            double_escape: crate::commands::DoubleEscape::Tree,
            ..Preferences::default()
        });
        type_text(&mut app, "discard this");

        app.handle(Action::Cancel);
        app.handle(Action::Cancel);

        assert!(app.editor.is_empty());
        assert_eq!(app.take_submission(), None);
    }

    #[test]
    fn a_second_escape_does_what_the_setting_says() {
        let mut app = app_with(Preferences {
            double_escape: crate::commands::DoubleEscape::Tree,
            ..Preferences::default()
        });
        app.handle(Action::Cancel);
        app.handle(Action::Cancel);
        assert_eq!(app.take_submission().as_deref(), Some("/tree"));

        let mut app = app_with(Preferences {
            double_escape: crate::commands::DoubleEscape::None,
            ..Preferences::default()
        });
        app.handle(Action::Cancel);
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

    /// A terminal that can draw images still does not when the reader would rather it did not.
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

    /// A credential is handed over only once enter has been pressed on it, so a key that is still
    /// being typed is never sent anywhere.
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
        assert!(app.cache.shape.is_some());

        app.handle(Action::Resize);
        assert!(app.cache.shape.is_none(), "the next frame rewraps");
    }

    #[test]
    fn browsing_history_walks_back_through_what_was_sent() {
        let mut app = app();
        for index in 0..20 {
            app.transcript.push_user(format!("message number {index}"));
        }
        app.begin_turn("first thing");
        app.begin_turn("second thing");
        app.set_frame(60, 24);
        app.set_viewport(10);
        app.refresh_lines();

        press_up_until_history(&mut app);
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
        for _ in 0..7 {
            seen.push(app.thinking_color());
            app.handle(Action::CycleThinking);
        }
        assert_eq!(app.thinking, ThinkingLevel::Off, "seven steps wraps around");

        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(
            unique.len(),
            4,
            "levels share the available thinking colours"
        );
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

    /// Build a request the way an extension's ask arrives, hand it to `ask_question`, and read back
    /// whatever it answered.
    fn ask(
        app: &mut App,
        method: &str,
        title: &str,
        detail: Option<&str>,
        options: Vec<&str>,
    ) -> serde_json::Value {
        let (request, mut answered) = crate::ui::UiRequest::for_test(
            method,
            title,
            detail.map(str::to_string),
            options.into_iter().map(str::to_string).collect(),
        );
        app.ask_question(request);
        answered.try_recv().expect("ask_question answers at once")
    }

    /// The same, from a named extension, so what it leaves behind can be attributed to it.
    fn ask_from(
        app: &mut App,
        extension: &str,
        method: &str,
        title: &str,
        detail: Option<&str>,
        options: Vec<&str>,
    ) -> serde_json::Value {
        let (request, mut answered) = crate::ui::UiRequest::for_test_from(
            Some(extension.to_string()),
            method,
            title,
            detail.map(str::to_string),
            options.into_iter().map(str::to_string).collect(),
        );
        app.ask_question(request);
        answered.try_recv().expect("ask_question answers at once")
    }

    /// Letting an extension go takes back everything it drew.
    #[test]
    fn deactivating_an_extension_takes_back_only_what_it_drew() {
        let mut app = app();
        ask_from(
            &mut app,
            "/x/leaving.ts",
            "set_widget",
            "mine",
            None,
            vec!["a line"],
        );
        ask_from(
            &mut app,
            "/x/leaving.ts",
            "set_status",
            "mine",
            Some("busy"),
            Vec::new(),
        );
        ask_from(
            &mut app,
            "/x/leaving.ts",
            "set_header",
            "",
            None,
            vec!["a header"],
        );
        ask_from(
            &mut app,
            "/x/leaving.ts",
            "set_footer",
            "",
            None,
            vec!["a footer"],
        );
        ask_from(
            &mut app,
            "/x/staying.ts",
            "set_widget",
            "theirs",
            None,
            vec!["still here"],
        );
        ask_from(
            &mut app,
            "/x/staying.ts",
            "set_status",
            "theirs",
            Some("fine"),
            Vec::new(),
        );

        assert_eq!(app.widgets.len(), 2);
        assert_eq!(app.extension_status.len(), 2);
        assert!(app.header_override().is_some());
        assert!(app.footer_override().is_some());

        ask_from(
            &mut app,
            "/x/leaving.ts",
            "deactivate_extension",
            "/x/leaving.ts",
            None,
            vec!["gone-tool"],
        );

        assert!(!app.widgets.contains_key("mine"));
        assert!(
            app.widgets.contains_key("theirs"),
            "another extension's widget stays"
        );
        assert!(!app.extension_status.contains_key("mine"));
        assert_eq!(
            app.extension_status.get("theirs").map(String::as_str),
            Some("fine")
        );
        assert!(app.header_override().is_none());
        assert!(app.footer_override().is_none());
        assert_eq!(app.take_retired_tools(), vec!["gone-tool".to_string()]);
        assert!(
            app.take_retired_tools().is_empty(),
            "taken once, so they are removed once"
        );
    }

    #[test]
    fn an_abort_question_interrupts_a_running_turn_and_says_so() {
        let mut app = app();
        app.busy("thinking");
        assert!(!app.is_interrupting());

        let answered = ask(&mut app, "abort", "", None, Vec::new());

        assert_eq!(answered["interrupted"], true);
        assert!(app.is_interrupting());
    }

    #[test]
    fn an_abort_question_with_nothing_running_interrupts_nothing() {
        let mut app = app();
        assert!(!app.is_interrupting());

        let answered = ask(&mut app, "abort", "", None, Vec::new());

        assert_eq!(answered["interrupted"], false);
        assert!(!app.is_interrupting());
    }

    #[test]
    fn setting_the_title_is_taken_once_and_only_once() {
        let mut app = app();
        ask(&mut app, "set_title", "a new title", None, Vec::new());
        assert_eq!(app.take_title_change().as_deref(), Some("a new title"));
        assert_eq!(app.take_title_change(), None);
    }

    #[test]
    fn a_header_can_be_set_and_restored() {
        let mut app = app();
        assert!(app.header_override().is_none());
        ask(&mut app, "set_header", "", None, vec!["custom header"]);
        assert_eq!(
            app.header_override(),
            Some(["custom header".to_string()].as_slice())
        );
        ask(&mut app, "set_header", "", None, Vec::new());
        assert!(app.header_override().is_none());
    }

    #[test]
    fn a_footer_can_be_set_and_restored() {
        let mut app = app();
        assert!(app.footer_override().is_none());
        ask(&mut app, "set_footer", "", None, vec!["custom footer"]);
        assert_eq!(
            app.footer_override(),
            Some(["custom footer".to_string()].as_slice())
        );
        ask(&mut app, "set_footer", "", None, Vec::new());
        assert!(app.footer_override().is_none());
    }

    /// Once a component has registered for a slot, a `component_changed` push naming its id updates
    /// whichever slot that was.
    #[test]
    fn a_component_changed_push_reaches_the_slot_it_registered_for() {
        let mut app = app();

        ask(
            &mut app,
            "register_component_slot",
            "c-header",
            Some("header"),
            Vec::new(),
        );
        ask(
            &mut app,
            "component_changed",
            "c-header",
            None,
            vec!["fresh header"],
        );
        assert_eq!(
            app.header_override(),
            Some(["fresh header".to_string()].as_slice())
        );

        ask(
            &mut app,
            "register_component_slot",
            "c-footer",
            Some("footer"),
            Vec::new(),
        );
        ask(
            &mut app,
            "component_changed",
            "c-footer",
            None,
            vec!["fresh footer"],
        );
        assert_eq!(
            app.footer_override(),
            Some(["fresh footer".to_string()].as_slice())
        );

        ask(
            &mut app,
            "set_widget",
            "status",
            Some("aboveEditor"),
            vec!["first"],
        );
        ask(
            &mut app,
            "register_component_slot",
            "c-widget",
            Some("widget:status"),
            Vec::new(),
        );
        ask(
            &mut app,
            "component_changed",
            "c-widget",
            None,
            vec!["second"],
        );
        assert_eq!(app.widgets_above(), vec![vec!["second".to_string()]]);

        let answer = ask(
            &mut app,
            "component_changed",
            "nobody-registered-this",
            None,
            vec!["x"],
        );
        assert_eq!(answer, serde_json::json!({}));
    }

    /// Registering a new id for a slot retires whichever id was there before.
    #[test]
    fn registering_a_slot_again_retires_the_previous_id() {
        let mut app = app();
        ask(
            &mut app,
            "register_component_slot",
            "c-old",
            Some("header"),
            Vec::new(),
        );
        ask(
            &mut app,
            "register_component_slot",
            "c-new",
            Some("header"),
            Vec::new(),
        );

        ask(&mut app, "component_changed", "c-old", None, vec!["stale"]);
        assert!(
            app.header_override().is_none(),
            "the old id no longer reaches the header"
        );

        ask(
            &mut app,
            "component_changed",
            "c-new",
            None,
            vec!["current"],
        );
        assert_eq!(
            app.header_override(),
            Some(["current".to_string()].as_slice())
        );
    }

    #[test]
    fn a_working_message_overrides_the_turns_own_label() {
        let mut app = app();
        app.busy("thinking");
        assert_eq!(app.activity(), "thinking");
        ask(
            &mut app,
            "set_working_message",
            "",
            Some("cooking up an answer"),
            Vec::new(),
        );
        assert_eq!(app.activity(), "cooking up an answer");

        ask(&mut app, "set_working_message", "", None, Vec::new());
        assert_eq!(app.activity(), "thinking");
    }

    #[test]
    fn working_visible_hides_and_restores_the_activity_line() {
        let mut app = app();
        assert!(app.working_visible());
        ask(&mut app, "set_working_visible", "false", None, Vec::new());
        assert!(!app.working_visible());
        ask(&mut app, "set_working_visible", "true", None, Vec::new());
        assert!(app.working_visible());
    }

    #[test]
    fn a_working_indicator_can_be_set_hidden_and_reset() {
        let mut app = app();

        assert_eq!(
            app.indicator_frame(),
            crate::render::status::spinner_frame(0)
        );

        ask(
            &mut app,
            "set_working_indicator",
            "set",
            Some("1000"),
            vec!["*"],
        );
        assert_eq!(app.indicator_frame(), "*");

        ask(&mut app, "set_working_indicator", "set", None, Vec::new());
        assert_eq!(app.indicator_frame(), "");

        ask(&mut app, "set_working_indicator", "reset", None, Vec::new());
        assert_eq!(
            app.indicator_frame(),
            crate::render::status::spinner_frame(0)
        );
    }

    #[test]
    fn a_hidden_thinking_label_can_be_set_and_reset() {
        let mut app = app();
        assert_eq!(app.hidden_thinking_label(), "Thinking...");
        ask(
            &mut app,
            "set_hidden_thinking_label",
            "",
            Some("Reasoning"),
            Vec::new(),
        );
        assert_eq!(app.hidden_thinking_label(), "Reasoning");
        ask(&mut app, "set_hidden_thinking_label", "", None, Vec::new());
        assert_eq!(app.hidden_thinking_label(), "Thinking...");
    }

    #[test]
    fn a_widget_is_set_placed_and_taken_out_by_an_empty_set() {
        let mut app = app();
        assert!(app.widgets_above().is_empty());

        ask(
            &mut app,
            "set_widget",
            "status",
            Some("aboveEditor"),
            vec!["line one", "line two"],
        );
        assert_eq!(
            app.widgets_above(),
            vec![vec!["line one".to_string(), "line two".to_string()]]
        );
        assert!(app.widgets_below().is_empty());

        ask(
            &mut app,
            "set_widget",
            "status",
            Some("aboveEditor"),
            Vec::new(),
        );
        assert!(app.widgets_above().is_empty());
    }

    #[test]
    fn a_widget_placed_below_the_editor_does_not_appear_above_it() {
        let mut app = app();
        ask(
            &mut app,
            "set_widget",
            "footer-note",
            Some("belowEditor"),
            vec!["hello"],
        );
        assert!(app.widgets_above().is_empty());
        assert_eq!(app.widgets_below(), vec![vec!["hello".to_string()]]);
    }

    #[test]
    fn a_long_widget_is_cut_off_with_a_note() {
        let mut app = app();
        let lines: Vec<&str> = (0..15).map(|_| "x").collect();
        ask(&mut app, "set_widget", "long", Some("aboveEditor"), lines);
        let shown = &app.widgets_above()[0];
        assert_eq!(shown.len(), MAX_WIDGET_LINES + 1);
        assert_eq!(shown.last().unwrap(), "... (widget truncated)");
    }

    fn only_tool(app: &App) -> &crate::transcript::ToolEntry {
        match app.transcript.entries().first() {
            Some(crate::transcript::Entry::Tool(tool)) => tool,
            other => panic!("expected a tool entry, got {other:?}"),
        }
    }

    /// A tool's renderCall answer lands on the call it was asked about, by id, and is read straight
    /// off the entry.
    #[test]
    fn a_tools_rendercall_answer_is_stored_on_its_entry() {
        let mut app = app();
        app.apply_event(micro_types::AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "weather".into(),
            arguments: serde_json::json!({}),
        });

        ask(
            &mut app,
            "tool_call_rendered",
            "call_1",
            Some("component-0"),
            vec!["lima: sunny"],
        );

        let tool = only_tool(&app);
        assert_eq!(tool.call_component_id.as_deref(), Some("component-0"));
        assert_eq!(tool.call_lines, Some(vec!["lima: sunny".to_string()]));
        assert!(tool.has_custom_render());
    }

    #[test]
    fn a_tools_renderresult_answer_joins_its_rendercall_answer() {
        let mut app = app();
        app.apply_event(micro_types::AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "weather".into(),
            arguments: serde_json::json!({}),
        });
        ask(
            &mut app,
            "tool_call_rendered",
            "call_1",
            Some("component-0"),
            vec!["lima:"],
        );
        ask(
            &mut app,
            "tool_result_rendered",
            "call_1",
            Some("component-1"),
            vec!["sunny, 18°C"],
        );

        let tool = only_tool(&app);
        assert_eq!(tool.render_lines(), vec!["lima:", "sunny, 18°C"]);
    }

    /// A component pushing its own change.
    #[test]
    fn a_pushed_component_change_updates_whichever_renderer_registered_it() {
        let mut app = app();
        app.apply_event(micro_types::AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "weather".into(),
            arguments: serde_json::json!({}),
        });
        ask(
            &mut app,
            "tool_call_rendered",
            "call_1",
            Some("component-0"),
            vec!["checking..."],
        );

        ask(
            &mut app,
            "component_changed",
            "component-0",
            None,
            vec!["lima: sunny"],
        );

        let tool = only_tool(&app);
        assert_eq!(tool.call_lines, Some(vec!["lima: sunny".to_string()]));
    }

    #[test]
    fn editor_text_can_be_set_and_a_paste_goes_through_the_editors_own_handling() {
        let mut app = app();
        ask(&mut app, "set_editor_text", "", Some("hello"), Vec::new());
        assert_eq!(app.editor.text(), "hello");

        let large = "x".repeat(5000);
        ask(&mut app, "paste_to_editor", "", Some(&large), Vec::new());
        assert!(app.editor.text().contains("hello"));
        assert_ne!(app.editor.expanded_text().len(), app.editor.text().len());
    }

    #[test]
    fn tools_expanded_folds_and_unfolds_every_entry() {
        let mut app = app();
        app.transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "a.rs" }),
        });
        assert!(app.transcript.any_collapsed());
        ask(&mut app, "set_tools_expanded", "true", None, Vec::new());
        assert!(!app.transcript.any_collapsed());
        ask(&mut app, "set_tools_expanded", "false", None, Vec::new());
        assert!(app.transcript.any_collapsed());
    }

    #[test]
    fn watching_terminal_input_toggles_on_and_off() {
        let mut app = app();
        assert!(!app.wants_terminal_input());
        ask(&mut app, "watch_terminal_input", "", None, Vec::new());
        assert!(app.wants_terminal_input());
        ask(&mut app, "unwatch_terminal_input", "", None, Vec::new());
        assert!(!app.wants_terminal_input());
    }

    #[test]
    fn a_theme_named_by_a_reader_is_switched_to() {
        let mut app = app();
        app.set_theme(Theme::light());
        let answer = ask(&mut app, "set_theme", "dark", None, Vec::new());
        assert_eq!(answer["ok"], true);
        assert_eq!(app.theme.name, "dark");
    }

    #[test]
    fn a_theme_that_does_not_exist_is_answered_rather_than_applied() {
        let mut app = app();
        let before = app.theme;
        let answer = ask(&mut app, "set_theme", "nocturne", None, Vec::new());
        assert_eq!(answer["ok"], false);
        assert_eq!(app.theme, before);
    }

    /// The shape `getTheme` hands an extension back.
    #[test]
    fn a_theme_snapshot_can_be_set_back() {
        let mut app = app();
        let mut colors = serde_json::Map::new();
        for token in Theme::TOKEN_NAMES {
            colors.insert((*token).to_string(), serde_json::json!("#123456"));
        }
        let answer = ask(
            &mut app,
            "set_theme",
            "custom",
            Some(&serde_json::Value::Object(colors).to_string()),
            Vec::new(),
        );
        assert_eq!(answer["ok"], true);
        assert_eq!(
            app.theme.accent,
            ratatui::style::Color::Rgb(0x12, 0x34, 0x56)
        );
    }

    #[test]
    fn a_custom_overlay_opens_with_its_first_lines_and_takes_the_keyboard() {
        let mut app = app();
        assert!(app.component_overlay_id().is_none());
        assert!(!app.overlay_is_open());

        let (request, _answered) = crate::ui::UiRequest::for_test(
            "custom",
            "component-1",
            None,
            vec!["hello from the component".to_string()],
        );
        app.ask_question(request);
        assert_eq!(app.component_overlay_id(), Some("component-1"));
        assert_eq!(
            app.component_overlay_lines(),
            Some(["hello from the component".to_string()].as_slice())
        );
        assert!(app.overlay_is_open());
    }

    #[test]
    fn a_component_cursor_marker_is_kept_for_rendering_but_not_shown() {
        let mut app = app();
        let (request, _answered) = crate::ui::UiRequest::for_test(
            "custom",
            "component-1",
            None,
            vec![format!("hello{CURSOR_MARKER} world")],
        );
        app.ask_question(request);

        assert_eq!(
            app.component_overlay_lines(),
            Some(["hello world".to_string()].as_slice())
        );
        assert_eq!(app.component_overlay_cursor(), Some((0, 5)));
    }

    #[test]
    fn file_completion_walks_the_workspace_not_its_display_label() {
        let workspace = std::env::temp_dir().join(format!(
            "micro-tui-completion-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir(&workspace).expect("workspace");
        std::fs::write(workspace.join("note.md"), "note").expect("workspace file");

        let options = TuiOptions {
            cwd: workspace.clone(),
            ..Default::default()
        };
        let mut app = App::new(&[], options);
        app.cwd = "~".to_string();
        assert!(app.workspace_files().iter().any(|path| path == "note.md"));

        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn file_completion_keeps_ignored_documents_available_for_citation() {
        let workspace = std::env::temp_dir().join(format!(
            "micro-tui-document-completion-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let private = workspace.join("private");
        std::fs::create_dir_all(&private).expect("workspace");
        std::fs::write(workspace.join(".gitignore"), "private/\n").expect("ignore rule");
        std::fs::write(private.join("brief.md"), "brief").expect("document");

        assert!(walk_workspace(&workspace)
            .iter()
            .any(|path| path == "private/brief.md"));

        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn file_completion_ignores_git_directory() {
        let workspace = std::env::temp_dir().join(format!(
            "micro-tui-git-completion-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let git_dir = workspace.join(".git");
        std::fs::create_dir_all(&git_dir).expect("workspace git dir");
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("git file");
        std::fs::write(workspace.join("main.rs"), "fn main() {}\n").expect("source file");

        let files = walk_workspace(&workspace);
        assert!(files.iter().any(|path| path == "main.rs"));
        assert!(!files.iter().any(|path| path.starts_with(".git")));

        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    /// Escape backs out of the overlay itself, the same as it does for a picker.
    #[test]
    fn escape_closes_a_custom_overlay_and_cancels_the_question() {
        let mut app = app();
        let (request, mut answered) =
            crate::ui::UiRequest::for_test("custom", "component-1", None, vec!["hi".to_string()]);
        app.ask_question(request);
        assert!(app.component_overlay_id().is_some());

        assert_eq!(app.handle(Action::Cancel), Outcome::Handled);
        assert!(app.component_overlay_id().is_none());
        assert_eq!(
            answered.try_recv().expect("cancelled at once")["cancelled"],
            true
        );
    }

    /// The component finishing on its own.
    #[test]
    fn a_component_finishing_itself_closes_the_overlay_with_its_result() {
        let mut app = app();
        let (request, mut answered) =
            crate::ui::UiRequest::for_test("custom", "component-1", None, vec!["hi".to_string()]);
        app.ask_question(request);

        ask(
            &mut app,
            "custom_done",
            "",
            Some(r#"{"picked": "yes"}"#),
            Vec::new(),
        );
        assert!(app.component_overlay_id().is_none());
        assert_eq!(answered.try_recv().unwrap()["value"]["picked"], "yes");
    }

    #[test]
    fn a_key_pushed_answer_redraws_the_open_overlay_and_ignores_a_stale_one() {
        let mut app = app();
        let (request, _answered) =
            crate::ui::UiRequest::for_test("custom", "component-1", None, vec!["v0".to_string()]);
        app.ask_question(request);
        app.set_component_overlay_lines("component-1", vec!["v1".to_string()]);
        assert_eq!(
            app.component_overlay_lines(),
            Some(["v1".to_string()].as_slice())
        );

        app.set_component_overlay_lines("some-other-component", vec!["stale".to_string()]);
        assert_eq!(
            app.component_overlay_lines(),
            Some(["v1".to_string()].as_slice())
        );
    }

    #[test]
    fn an_editor_component_can_be_set_and_restored() {
        let mut app = app();
        assert!(app.editor_component_id().is_none());

        ask(
            &mut app,
            "set_editor_component",
            "component-1",
            None,
            vec!["> "],
        );
        assert_eq!(app.editor_component_id(), Some("component-1"));
        assert_eq!(app.editor_component_lines(), ["> ".to_string()]);

        ask(&mut app, "set_editor_component", "", None, Vec::new());
        assert!(app.editor_component_id().is_none());
        assert!(app.editor_component_lines().is_empty());
    }

    #[test]
    fn a_pushed_answer_redraws_the_editor_component_and_ignores_a_stale_one() {
        let mut app = app();
        ask(
            &mut app,
            "set_editor_component",
            "component-1",
            None,
            vec!["v0"],
        );
        app.set_editor_component_lines("component-1", vec!["v1".to_string()]);
        assert_eq!(app.editor_component_lines(), ["v1".to_string()]);

        app.set_editor_component_lines("some-other-component", vec!["stale".to_string()]);
        assert_eq!(app.editor_component_lines(), ["v1".to_string()]);
    }
}
