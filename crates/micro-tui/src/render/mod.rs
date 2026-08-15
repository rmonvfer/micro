//! Frame layout.
//!
//! Flat and borderless: the transcript sits on the terminal's own background, and the input
//! and footer are separated from it by a blank row and a background tint rather than a box.
//!
//! The frame is only as tall as it needs to be. [`plan`] measures it against the terminal
//! and hands back both the height to ask for and whatever no longer fits, which the caller
//! prints above the region into the terminal's own history. [`draw`] then lays the same
//! bands out inside exactly that many rows, so the two always agree on where things go.

mod editor;
pub mod hints;
pub mod links;
mod menu;
mod overlay;
pub mod pictures;
pub mod status;
mod tool;
pub mod transcript;

use crate::app::App;
use crate::theme::Theme;
use crate::wrap::text_width;
use crate::wrap::wrap_spans;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::Frame;

/// The share of the screen the input may grow to before it scrolls internally, and the
/// fewest rows it may have whatever the screen's height.
const EDITOR_SHARE: f32 = 0.3;
const MIN_EDITOR_ROWS: usize = 5;

/// Rows the input may grow to on a screen this tall.
fn max_editor_rows(rows: u16) -> usize {
    ((rows as f32 * EDITOR_SHARE) as usize).max(MIN_EDITOR_ROWS)
}
/// Rows kept for the conversation behind an overlay, so opening one never hides all of it.
///
/// One. Everything else on the screen is the overlay's to use: a list is what the reader is
/// working in while it is open, and one squeezed into a corner shows three or four rows
/// wrapped in as much chrome again — an interface made mostly of margins.
const ROWS_BEHIND_OVERLAY: u16 = 1;
/// Rows kept for the spinner: a blank one, then the message.
///
/// Held from the first turn onward, whether or not one is running, so starting one never
/// shifts the interface vertically. Before then they are not held at all: a screen that
/// has done nothing has nothing to say there.
const ACTIVITY_ROWS: u16 = 2;

/// The rows the spinner is entitled to right now.
fn activity_rows(app: &App) -> u16 {
    match app.reserves_activity_rows() {
        true => ACTIVITY_ROWS,
        false => 0,
    }
}
/// What this is called on its own first screen.
const APP_NAME: &str = "micro";

/// Lay the whole interface out on the frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = margin(frame.area(), app.settings().interface_padding);
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = app.theme;

    let content_padding = app.settings().content_padding;
    let content_width = content_width(area.width, content_padding);
    // The frame is measured before anything is laid out against it: the transcript wraps to
    // this width, and a page of scrolling moves by the rows the region turns out to have.
    app.set_frame(content_width as usize, area.height);
    let chrome = chrome(app, &theme, area.width, area.height);
    let transcript_rows = area.height.saturating_sub(chrome.rows());
    app.set_viewport(transcript_rows as usize);

    app.refresh_lines();

    // Before anything has happened there is no conversation, and the screen introduces
    // itself in the space one would have taken — an extension's own header in place of the
    // built-in one, when `setHeader` gave it one.
    let opening = match app.lines().is_empty() && !app.settings().quiet_startup {
        true => match app.header_override() {
            Some(lines) => lines
                .iter()
                .map(|line| Line::styled(line.clone(), theme.body()))
                .collect(),
            None => intro(
                &theme,
                content_width as usize,
                app.startup_expanded(),
                app.resources(),
            ),
        },
        false => Vec::new(),
    };

    // The engine says how the rows divide; ratatui draws into what it decided. One
    // calculation whether the interface has the whole screen or a region of it.
    let rows = chrome.stack(Some(area.height as usize)).allocation(0);
    let [transcript_area, _, activity_area, overlay_area, widgets_above_area, editor_area, widgets_below_area, menu_area, status_area] =
        Layout::vertical(rows.iter().map(|rows| Constraint::Length(*rows as u16))).areas(area);

    let overlay = chrome.overlay;
    let menu = chrome.menu;
    let widgets_above = chrome.widgets_above;
    let widgets_below = chrome.widgets_below;

    draw_transcript(frame, transcript_area, app, &opening, &theme);
    draw_rows(
        frame,
        overlay_area,
        overlay_area,
        &overlay,
        &theme,
        app.picker().is_none(),
    );
    draw_activity(frame, inset_by(activity_area, content_padding), app, &theme);
    draw_rows(
        frame,
        widgets_above_area,
        inset_by(widgets_above_area, content_padding),
        &widgets_above,
        &theme,
        false,
    );
    // An overlay has the keyboard while it is up, so the cursor belongs to it rather than to
    // an input the next keystroke will not reach.
    let level = app.thinking_color();
    match app.editor_component_id() {
        Some(_) => editor::draw_component(
            frame,
            editor_area,
            inset_by(editor_area, content_padding),
            app.editor_component_lines(),
            &theme,
            level,
        ),
        None => editor::draw(
            frame,
            editor_area,
            inset_by(editor_area, content_padding),
            &app.editor,
            &theme,
            editor::Look {
                level,
                focused: !app.overlay_is_open(),
                hardware_cursor: app.settings().show_hardware_cursor,
            },
        ),
    }
    draw_rows(
        frame,
        widgets_below_area,
        inset_by(widgets_below_area, content_padding),
        &widgets_below,
        &theme,
        false,
    );
    for (offset, line) in menu.iter().take(menu_area.height as usize).enumerate() {
        let content = inset_by(menu_area, content_padding);
        frame
            .buffer_mut()
            .set_line(content.x, content.y + offset as u16, line, content.width);
    }
    draw_status(frame, inset_by(status_area, content_padding), app, &theme);

    // Last, once every line has been placed and its columns are settled: a hyperlink costs
    // no width, so it can only go on after everything that measures width has finished.
    app.links().apply(frame.buffer_mut(), area);
    // Images are placed the same way and for the same reason: an escape that occupies no
    // columns can only go on once every column is settled.
    let first_visible = app
        .lines()
        .len()
        .saturating_sub(transcript_rows as usize)
        .saturating_sub(app.scroll());
    app.pictures()
        .apply(frame.buffer_mut(), transcript_area, first_visible);
}

/// Columns left for content once both margins are taken off.
fn content_width(width: u16, padding: u16) -> u16 {
    width.saturating_sub(padding * 2).max(1)
}

/// Everything below the transcript, and how many rows it takes.
struct Chrome {
    overlay: Vec<Line<'static>>,
    menu: Vec<Line<'static>>,
    editor: u16,
    /// The rows held for the spinner, which is none until something has been worked on.
    activity: u16,
    /// The rows the footer needs, which grows when an extension has something to say.
    status: u16,
    /// What `setWidget` asked shown just above the input, already laid out.
    widgets_above: Vec<Line<'static>>,
    /// What `setWidget` asked shown just below the input, already laid out.
    widgets_below: Vec<Line<'static>>,
}

impl Chrome {
    /// The blank row, whatever is open, the spinner's row, the input, its menu, the footer.
    fn rows(&self) -> u16 {
        use crate::layout::Component as _;
        self.stack(None).height(0) as u16
    }

    /// Every region of the interface, in the order they are drawn.
    ///
    /// Given a screen's height the conversation is the first region and takes whatever the
    /// others leave. Without one the stack is the chrome alone, which is what says how tall
    /// the interface is before there is anywhere to put it — what drawing inline needs,
    /// since there the region is only as tall as the interface turns out to be.
    ///
    /// One construction either way, so the height the caller reserves and the rows it draws
    /// into can never disagree.
    fn stack(&self, height: Option<usize>) -> crate::layout::Stack {
        use crate::layout::{Child, Lines, Spacer, Stack};
        let stack = match height {
            // The conversation gives way first when there is not enough: the prompt is
            // what a reader is using.
            Some(height) => Stack::within(height).with(Child::flexible(Spacer(0), 1)),
            None => Stack::new(),
        };
        stack
            // The blank row above whatever is open.
            .with(Child::content(Spacer(1)))
            .with(Child::content(Spacer(self.activity as usize)))
            // An overlay stands where the prompt stands rather than above it: it is what
            // the next keystroke reaches, so it is what occupies the place a reader is
            // already looking at. The prompt is not drawn behind it.
            .with(Child::content(Lines(self.overlay.clone())))
            .with(Child::content(Lines(self.widgets_above.clone())))
            .with(Child::content(Spacer(self.editor as usize)))
            .with(Child::content(Lines(self.widgets_below.clone())))
            .with(Child::content(Lines(self.menu.clone())))
            .with(Child::content(Spacer(self.status as usize)))
    }
}

/// How many rows the interface itself needs, apart from the conversation.
///
/// What drawing inline asks for: the region is only as tall as the prompt, the footer and
/// whatever is open above them, and the conversation goes to the terminal's own scrollback.
pub fn interface_rows(app: &App, theme: &Theme, width: u16, height: u16) -> u16 {
    let width = margin(
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        app.settings().interface_padding,
    )
    .width;
    chrome(app, theme, width, height).rows()
}

fn chrome(app: &App, theme: &Theme, width: u16, height: u16) -> Chrome {
    let overlay = overlay_lines(app, theme, width as usize);
    let activity = activity_rows(app);
    let widgets_above = widget_lines(app.widgets_above(), theme);
    let widgets_below = widget_lines(app.widgets_below(), theme);
    // The prompt's own rows, plus the rule above it and the rule below it. None at all
    // while an overlay is up: the overlay has taken its place, and a prompt drawn below one
    // that the keyboard does not reach is a second input that does nothing.
    let editor = match overlay.is_empty() {
        true => {
            let content_rows = match app.editor_component_id() {
                Some(_) => app.editor_component_lines().len().max(1),
                None => app.editor.height(width as usize),
            };
            content_rows.clamp(1, max_editor_rows(height)) as u16 + editor::RULES
        }
        false => 0,
    };
    let status = footer_height(app);

    // A menu opens under the prompt, so it can only have rows nothing else is using. It is
    // the one part of the interface that is there to be scrolled, and the prompt it belongs
    // to has to stay whole: an input without its rules is not a smaller input, it is a
    // different one.
    let held = 1
        + overlay.len() as u16
        + activity
        + widgets_above.len() as u16
        + editor
        + widgets_below.len() as u16
        + status;
    let free = height.saturating_sub(held) as usize;

    Chrome {
        overlay,
        menu: app
            .menu()
            .map(|menu| menu::lines(menu, theme, width as usize, app.menu_rows().min(free)))
            .unwrap_or_default(),
        editor,
        activity,
        status,
        widgets_above,
        widgets_below,
    }
}

/// An extension's widgets, laid out one line per row in the order they were set — the same
/// order a `BTreeMap` keeps them in, which is by the key each was set under.
fn widget_lines(widgets: Vec<Vec<String>>, theme: &Theme) -> Vec<Line<'static>> {
    widgets
        .into_iter()
        .flatten()
        .map(|line| Line::styled(line, theme.body()))
        .collect()
}

/// The rows of whatever overlay is up, in the order of what is blocking on an answer: a
/// credential first, then a list to choose from.
fn overlay_lines(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    // Sized against the screen, not against the region: the region is sized by what this
    // turns out to need, so measuring it against itself would never settle. What is left
    // after the footer, the spinner's rows and a glimpse of the conversation is the
    // overlay's; the conversation gives way, which is what the layout does anyway.
    let held = ROWS_BEHIND_OVERLAY + activity_rows(app) + footer_height(app) + 1;
    let budget = app.rows().saturating_sub(held).max(4) as usize;

    if let Some(prompt) = app.key_prompt() {
        return overlay::key_prompt_lines(prompt, theme, width);
    }
    if let Some((title, text, scroll)) = app.inspection() {
        return overlay::inspection_lines(title, text, scroll, theme, width, budget);
    }
    if let Some(lines) = app.component_overlay_lines() {
        return lines
            .iter()
            .take(budget)
            .map(|line| Line::styled(line.clone(), theme.body()))
            .collect();
    }
    if let (Some(title), Some(editor)) = (app.extension_editor_title(), app.extension_editor()) {
        return overlay::extension_editor_lines(title, editor, theme, width, budget);
    }
    match app.picker() {
        Some(picker) => overlay::picker_lines(picker, theme, width, budget),
        None => Vec::new(),
    }
}

/// Paint a block of already-wrapped rows, filling the region behind them.
fn draw_rows(
    frame: &mut Frame,
    area: Rect,
    content: Rect,
    rows: &[Line<'static>],
    theme: &Theme,
    surface: bool,
) {
    if area.height == 0 || content.width == 0 {
        return;
    }
    if surface {
        frame
            .buffer_mut()
            .set_style(area, Style::new().bg(theme.surface));
    }
    for (offset, line) in rows.iter().take(area.height as usize).enumerate() {
        frame
            .buffer_mut()
            .set_line(content.x, content.y + offset as u16, line, content.width);
    }
}

fn draw_transcript(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    opening: &[Line<'static>],
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let height = area.height as usize;
    let rows = match app.lines().is_empty() {
        true => opening,
        false => app.lines(),
    };

    // The newest lines are the ones worth seeing, so the window sits at the end of what the
    // conversation holds and moves back only as far as the reader has scrolled.
    let first = rows
        .len()
        .saturating_sub(height)
        .saturating_sub(app.scroll());
    let shown = rows.len().saturating_sub(first).min(height);

    // A conversation shorter than the region rests on the bottom of it, so it grows upward
    // out of the input rather than hanging from the top of the screen with a gap beneath it.
    let top = area.y + (height - shown) as u16;
    for (offset, line) in rows.iter().skip(first).take(height).enumerate() {
        frame
            .buffer_mut()
            .set_line(area.x, top + offset as u16, line, area.width);
    }

    draw_scrollbar(
        frame,
        area,
        rows.len(),
        first,
        theme,
        app.settings().scrollbar,
    );
}

/// How far through the conversation the window is, drawn down the right edge.
///
/// Shown only where there is more than fits, unless the reader asked for it always. It sits
/// in the last column of the region, over whatever is there: a conversation wraps to the
/// content width, which already leaves that column clear.
fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    first: usize,
    theme: &Theme,
    when: crate::commands::Scrollbar,
) {
    use crate::commands::Scrollbar;

    let height = area.height as usize;
    let overflows = total > height;
    let draw = match when {
        Scrollbar::Hidden => false,
        Scrollbar::Auto => overflows,
        Scrollbar::Always => true,
    };
    if !draw || area.width == 0 || height == 0 {
        return;
    }

    // The thumb is as tall a share of the track as the window is of the conversation, and
    // never shorter than one row: a mark that is not there says nothing about where you are.
    let thumb = match overflows {
        true => (height * height / total).max(1),
        false => height,
    };
    let furthest = total.saturating_sub(height);
    let at = match furthest {
        0 => 0,
        _ => first * (height - thumb) / furthest,
    };

    let column = area.x + area.width - 1;
    for offset in 0..height {
        let (glyph, color) = match (offset >= at && offset < at + thumb, overflows) {
            (true, _) => ("│", theme.border_accent),
            (false, true) => ("│", theme.border_muted),
            // Nothing to scroll, so the track is drawn without a road to travel down.
            (false, false) => (" ", theme.border_muted),
        };
        frame.buffer_mut().set_string(
            column,
            area.y + offset as u16,
            glyph,
            Style::new().fg(color),
        );
    }
}

/// Lay a background across a whole row, exactly one row wide.
///
/// A tinted row is how this interface marks a region — the input, the footer, a code block,
/// an approval prompt — so the background has to reach the edge rather than stopping where
/// the text does. Anything past the edge is cut, since a tinted row that overruns would
/// wrap and leave the block ragged.
fn tint(line: Line<'static>, width: usize, background: Color) -> Line<'static> {
    let mut line = clip(line, width);
    let used: usize = line
        .spans
        .iter()
        .map(|span| text_width(&span.content))
        .sum();
    for span in &mut line.spans {
        span.style = span.style.bg(background);
    }
    if used < width {
        line.spans.push(Span::styled(
            " ".repeat(width - used),
            Style::new().bg(background),
        ));
    }
    line
}

/// Cut a row at the width. A row that overruns would wrap and push everything below it out
/// of place, so every row assembled by hand goes through here before it is drawn.
fn clip(line: Line<'static>, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;

    for mut span in line.spans {
        if used >= width {
            break;
        }
        if used + text_width(&span.content) > width {
            span.content = crate::wrap::truncate(&span.content, width - used).into();
        }
        used += text_width(&span.content);
        spans.push(span);
    }
    Line::from(spans)
}

/// The keys worth knowing on the first screen. A key is dim and what it does is muted,
/// which is how every hint in the interface is written.
const HINTS: [(&str, &str); 5] = [
    ("escape", "interrupt"),
    ("ctrl+c/ctrl+d", "clear/exit"),
    ("/", "commands"),
    ("!", "bash"),
    ("ctrl+o", "more"),
];

/// Every key, for the reader who asked for all of them. One per row rather than one row of
/// all of them: this is a list to look something up in, not a line to glance at.
const ALL_HINTS: [(&str, &str); 19] = [
    ("escape", "to interrupt"),
    ("ctrl+c", "to clear"),
    ("ctrl+c twice", "to exit"),
    ("ctrl+d", "to exit (empty)"),
    ("ctrl+z", "to suspend"),
    ("ctrl+k", "to delete to end"),
    ("shift+tab", "to cycle thinking level"),
    ("ctrl+p/ctrl+shift+p", "to cycle models"),
    ("ctrl+l", "to select model"),
    ("ctrl+o", "to expand tools"),
    ("ctrl+t", "to expand thinking"),
    ("ctrl+g", "for external editor"),
    ("/", "for commands"),
    ("!", "to run bash"),
    ("!!", "to run bash (no context)"),
    ("alt+enter", "to queue follow-up"),
    ("alt+up", "to edit all queued messages"),
    ("ctrl+v", "to paste image (with text fallback)"),
    ("drop files", "to attach"),
];

/// What the first screen says before anything has happened: what this is, the keys worth
/// knowing, what was loaded, and that it can describe itself. Where the session is belongs
/// to the footer, which says it on every screen rather than only on this one.
///
/// Two depths of it. Closed, the five keys worth knowing and each shelf by name, which is
/// what someone starting work needs. Opened with `ctrl+o`, every key and the file each
/// resource was read from, which is what someone asking why a skill did or did not load
/// needs. The same key opens every tool result, so there is one way to ask for more.
fn intro(
    theme: &Theme,
    width: usize,
    expanded: bool,
    resources: &crate::app::Resources,
) -> Vec<Line<'static>> {
    let dim = Style::new().fg(theme.dim);

    let logo = [
        Span::styled(
            APP_NAME.to_string(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" v{}", env!("CARGO_PKG_VERSION")), dim),
    ];

    let onboarding = [Span::styled(
        format!(
            "{APP_NAME} can explain its own features and look up its docs. \
             Ask it how to use or extend {APP_NAME}."
        ),
        dim,
    )];

    // No blank row at the end: the transcript's own gap row is directly below, and a second
    // one would leave the opening screen floating above the input.
    let mut out = vec![Line::default()];
    out.extend(wrap_spans(&logo, width, 0));

    match expanded {
        // One shape for every hint in the interface, so a key is described the same way
        // here as it is anywhere else — and reads as `option` on a Mac.
        false => {
            out.extend(wrap_spans(&hints::hints(&HINTS, theme), width, 0));
            out.extend(wrap_spans(
                &[Span::styled(
                    format!(
                        "Press {} to show full startup help and loaded resources.",
                        hints::key_text("ctrl+o")
                    ),
                    dim,
                )],
                width,
                0,
            ));
        }
        true => {
            for (keys, description) in ALL_HINTS {
                out.extend(wrap_spans(&hints::hint(keys, description, theme), width, 0));
            }
        }
    }

    out.push(Line::default());
    out.extend(wrap_spans(&onboarding, width, 0));
    out.extend(resource_lines(resources, theme, width, expanded));
    out
}

/// How far a section's contents sit in from its heading.
const LISTING_INDENT: &str = "  ";

/// What was loaded, section by section.
///
/// Closed, a section is its names on one wrapped row, which says what is available without
/// saying where any of it lives. Opened, it is one row per file, because the answer to
/// "which of the two skills called this did I get" is a path and nothing else.
fn resource_lines(
    resources: &crate::app::Resources,
    theme: &Theme,
    width: usize,
    expanded: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for section in &resources.sections {
        out.push(Line::default());
        out.extend(wrap_spans(
            &[Span::styled(
                format!("[{}]", section.name),
                Style::new().fg(theme.md_heading),
            )],
            width,
            0,
        ));

        // Indented in, and only on the row it starts on: a list long enough to wrap reads
        // as one run of names rather than as a column with a ragged left edge.
        let dim = Style::new().fg(theme.dim);
        match expanded {
            false => out.extend(wrap_spans(
                &[Span::styled(
                    format!("{LISTING_INDENT}{}", section.names.join(", ")),
                    dim,
                )],
                width,
                0,
            )),
            true => {
                for path in &section.paths {
                    out.extend(wrap_spans(
                        &[Span::styled(format!("{LISTING_INDENT}{path}"), dim)],
                        width,
                        0,
                    ));
                }
            }
        }
    }
    out
}

fn draw_activity(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.width == 0 || area.height == 0 || !app.is_running() || !app.working_visible() {
        return;
    }
    let line = status::activity_line(
        theme,
        app.indicator_frame(),
        app.elapsed(),
        app.is_interrupting(),
        &app.activity(),
    );
    let line = match app.queued() {
        0 => line,
        count => {
            let mut line = line;
            line.spans.push(Span::styled(
                format!("  ·  {count} queued"),
                Style::new().fg(theme.dim),
            ));
            line
        }
    };
    // The first of the two rows stays blank; the message sits on the second, which is what
    // keeps the gap above it the same whether or not a turn is running.
    let row = area.y + u16::from(area.height > 1);
    frame.buffer_mut().set_line(area.x, row, &line, area.width);
}

/// Everything the footer reports about the session as it stands.
///
/// Built in one place so the rows drawn and the rows reserved for them are always the
/// same rows.
fn footer_for(app: &App) -> status::Footer<'_> {
    status::Footer {
        cwd: &app.cwd,
        branch: app.branch(),
        session: None,
        total: app.transcript.total_usage(),
        last: app.transcript.last_usage(),
        context_window: app.context_window,
        model: app.model_id(),
        thinking: Some(crate::app::thinking_name(app.thinking)),
        attachments: app.attachments(),
        cost: app.session_cost(),
        subscription: app.subscription,
        auto_compact: app.auto_compact,
        provider: app.footer_provider(),
        experimental: false,
        extension_status: app
            .extension_status
            .iter()
            .map(|(key, text)| (key.clone(), text.clone()))
            .collect(),
    }
}

/// How many rows the footer takes: an extension's own count, from `setFooter`, or micro's.
fn footer_height(app: &App) -> u16 {
    match app.footer_override() {
        Some(lines) => lines.len() as u16,
        None => footer_for(app).height(),
    }
}

/// The footer's rows: an extension's own, from `setFooter`, or micro's.
fn footer_rows(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    match app.footer_override() {
        Some(lines) => lines
            .iter()
            .map(|line| Line::styled(line.clone(), theme.body()))
            .collect(),
        None => footer_for(app).rows(theme, width),
    }
}

fn draw_status(frame: &mut Frame, content: Rect, app: &App, theme: &Theme) {
    if content.width == 0 || content.height == 0 {
        return;
    }
    for (offset, line) in footer_rows(app, theme, content.width as usize)
        .iter()
        .take(content.height as usize)
        .enumerate()
    {
        frame
            .buffer_mut()
            .set_line(content.x, content.y + offset as u16, line, content.width);
    }
}

/// Pull an area in by the horizontal padding.
/// Pull an area in by a chosen number of columns on each side.
/// The interface's own area: the terminal, less the margin it keeps around itself.
///
/// Everything is laid out inside this, so the margin is there whatever is drawn — the
/// rules above and below the input, a tinted band, the footer. A terminal too small to
/// spare the room keeps as much of it as it can.
fn margin(area: Rect, padding: u16) -> Rect {
    let horizontal = padding.min(area.width.saturating_sub(1) / 2);
    let vertical = padding.min(area.height.saturating_sub(1) / 2);
    Rect {
        x: area.x + horizontal,
        y: area.y + vertical,
        width: area.width.saturating_sub(horizontal * 2),
        height: area.height.saturating_sub(vertical * 2),
    }
}

fn inset_by(area: Rect, padding: u16) -> Rect {
    Rect {
        x: area.x + padding.min(area.width),
        y: area.y,
        width: area.width.saturating_sub(padding * 2),
        height: area.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TuiOptions;
    use crate::event::Action;
    use micro_types::AgentEvent;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// An app holding one of everything the transcript can show.
    fn populated() -> App {
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("read the file and explain it");
        app.apply_event(AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "src/main.rs" }),
        });
        app.apply_event(AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: "fn main() {}\n".into(),
            is_error: false,
        });
        app.apply_event(AgentEvent::ToolStart {
            id: "call_2".into(),
            name: "edit".into(),
            arguments: serde_json::json!({
                "path": "src/main.rs",
                "old_string": "fn main() {}",
                "new_string": "fn main() {\n    println!(\"hi\");\n}",
            }),
        });
        app.apply_event(AgentEvent::ToolEnd {
            id: "call_2".into(),
            name: "edit".into(),
            output: "Edited src/main.rs".into(),
            is_error: false,
        });
        app.apply_event(AgentEvent::MessageDelta {
            event: micro_types::StreamEvent::ThinkingDelta {
                index: 0,
                delta: "weighing the options".into(),
            },
        });
        app.apply_event(AgentEvent::MessageDelta {
            event: micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "It is an **empty** program:\n\n```rust\nfn main() {}\n```\n".into(),
            },
        });
        app.handle(Action::Insert("a half written\nprompt".into()));
        app
    }

    /// Painting must stay inside the buffer at any size a terminal can be dragged to.
    #[test]
    fn every_terminal_size_paints_without_panicking() {
        for (width, height) in [(1, 1), (2, 3), (8, 4), (20, 6), (80, 24), (200, 60)] {
            let mut app = populated();
            let mut terminal =
                Terminal::new(TestBackend::new(width, height)).expect("test backend");
            for show in [false, true] {
                app.show_thinking = show;
                if show {
                    app.handle(Action::ToggleFocused);
                }
                terminal
                    .draw(|frame| draw(frame, &mut app))
                    .unwrap_or_else(|error| panic!("{width}x{height} failed: {error}"));
            }
        }
    }

    /// A widget an extension asked shown above the input is drawn where it asked, and one
    /// asked below appears after it rather than instead of it.
    #[test]
    fn extension_widgets_are_drawn_where_they_were_placed() {
        let mut app = App::new(&[], TuiOptions::default());
        let (above, _rx1) = crate::ui::UiRequest::for_test(
            "set_widget",
            "above",
            Some("aboveEditor".to_string()),
            vec!["widget above the input".to_string()],
        );
        app.ask_question(above);
        let (below, _rx2) = crate::ui::UiRequest::for_test(
            "set_widget",
            "below",
            Some("belowEditor".to_string()),
            vec!["widget below the input".to_string()],
        );
        app.ask_question(below);

        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("backend");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
        let rows = screen(&terminal);

        let above_row = rows
            .iter()
            .position(|row| row.contains("widget above the input"))
            .expect("the above-editor widget is drawn");
        let below_row = rows
            .iter()
            .position(|row| row.contains("widget below the input"))
            .expect("the below-editor widget is drawn");
        assert!(above_row < below_row, "{rows:?}");
    }

    /// Once `setEditorComponent` has replaced the built-in editor, its lines are what is
    /// drawn in the input's place — not the built-in editor, even though it is still there
    /// underneath, unseen.
    #[test]
    fn an_editor_component_is_drawn_in_the_inputs_place() {
        let mut app = App::new(&[], TuiOptions::default());
        app.editor.insert_str("this should not be on screen");
        let (request, _answered) = crate::ui::UiRequest::for_test(
            "set_editor_component",
            "component-1",
            None,
            vec!["a custom editor".to_string()],
        );
        app.ask_question(request);

        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("backend");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
        let rows = screen(&terminal);

        assert!(
            rows.iter().any(|row| row.contains("a custom editor")),
            "{rows:?}"
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.contains("this should not be on screen")),
            "{rows:?}"
        );
    }

    /// A menu opens under the input and can be long. It may take rows the conversation is
    /// not using, and no others: an input missing the rules that bound it is not a smaller
    /// input, it is one a reader cannot see the edges of.
    #[test]
    fn opening_the_menu_does_not_take_the_rules_off_the_input() {
        let rules = |terminal: &Terminal<TestBackend>| {
            screen(terminal)
                .iter()
                .filter(|row| row.trim_start().starts_with('\u{2500}'))
                .count()
        };

        for height in [10, 16, 24, 40] {
            let mut app = App::new(&[], TuiOptions::default());
            app.transcript.push_user("something said earlier");
            let mut terminal = Terminal::new(TestBackend::new(60, height)).expect("backend");
            terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
            let closed = rules(&terminal);
            assert_eq!(
                closed, 2,
                "{height} rows: the input is bounded above and below"
            );

            app.handle(Action::Insert("/".into()));
            assert!(app.menu().is_some(), "typing a slash opens the menu");
            terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
            assert_eq!(
                rules(&terminal),
                closed,
                "{height} rows: the menu took rows from the input"
            );
        }
    }

    /// Read the buffer back as rows of text, trailing blanks trimmed.
    fn screen(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// The first screen says what loaded, and `ctrl+o` is what turns a name into the file
    /// it came from — the only way to tell two skills of the same name apart.
    #[test]
    fn the_first_screen_names_what_loaded_and_opens_to_say_where_from() {
        let mut resources = crate::app::Resources::default();
        resources.add(
            "Skills",
            vec!["humanizer".into(), "shadcn".into()],
            vec![
                "~/.micro/skills/humanizer/SKILL.md".into(),
                "~/x/shadcn.md".into(),
            ],
        );
        let mut app = App::new(
            &[],
            TuiOptions {
                resources,
                ..TuiOptions::default()
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(70, 30)).expect("backend");

        terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
        let closed = screen(&terminal).join("\n");
        assert!(closed.contains("[Skills]"), "{closed}");
        assert!(closed.contains("humanizer, shadcn"), "{closed}");
        assert!(
            closed.contains("to show full startup help and loaded resources"),
            "{closed}"
        );
        assert!(!closed.contains("SKILL.md"), "closed it names, not locates");

        app.handle(Action::ToggleFocused);
        terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
        let opened = screen(&terminal).join("\n");
        assert!(
            opened.contains("~/.micro/skills/humanizer/SKILL.md"),
            "{opened}"
        );
        assert!(
            opened.contains("to cycle thinking level"),
            "every key: {opened}"
        );

        // And closes again, so the key is a toggle rather than a one-way door.
        app.handle(Action::ToggleFocused);
        terminal.draw(|frame| draw(frame, &mut app)).expect("draws");
        assert!(screen(&terminal).join("\n").contains("humanizer, shadcn"));
    }

    /// A shelf with nothing on it is left out: an empty heading says less than no heading.
    #[test]
    fn an_empty_shelf_is_not_given_a_heading() {
        let mut resources = crate::app::Resources::default();
        resources.add("Prompts", Vec::new(), Vec::new());
        assert!(resources.is_empty());
    }

    /// The rows the spinner draws in are not held open until something has been worked
    /// on, so a screen that has done nothing sits close to the input; from the first turn
    /// onward they stay held, so the input does not jump as turns come and go.
    #[test]
    fn the_spinner_holds_its_rows_only_once_there_has_been_a_turn() {
        /// Blank rows between the last thing said and the rule above the input.
        fn gap(rows: &[String]) -> usize {
            let rule = rows
                .iter()
                .position(|row| row.trim_start().starts_with('\u{2500}'))
                .expect("the input has a rule above it");
            rows[..rule]
                .iter()
                .rev()
                .take_while(|row| row.trim().is_empty())
                .count()
        }

        // Ending on an answer rather than a question: a question's block carries a blank
        // row of its own beneath it, which would be counted as part of the gap.
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("a question");
        app.apply_event(AgentEvent::MessageDelta {
            event: micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "an answer".into(),
            },
        });

        assert_eq!(
            gap(&paint(&mut app, 40, 12)),
            1,
            "nothing has been worked on"
        );

        app.busy("thinking");
        app.finish_turn(false);
        assert_eq!(
            gap(&paint(&mut app, 40, 12)),
            1,
            "on a screen of its own the rows go back to the conversation between turns"
        );

        // And they stay held while one runs, so nothing shifts when it begins.
        app.busy("thinking");
        let running = paint(&mut app, 40, 12);
        let rule = running
            .iter()
            .position(|row| row.trim_start().starts_with('\u{2500}'))
            .expect("the input has a rule above it");
        assert!(
            running[..rule].iter().any(|row| row.contains("thinking")),
            "the spinner draws in the rows that were held for it: {running:?}"
        );
    }

    /// Drawing inline, the region is only as tall as the interface. Giving the spinner's
    /// rows back between turns would shrink it, and the rows the terminal has already been
    /// handed would scroll away — so once something has run they stay held.
    #[test]
    fn the_spinner_keeps_its_rows_between_turns_when_drawing_inline() {
        let mut app = App::new(
            &[],
            TuiOptions {
                tui_mode: crate::TuiMode::Inline,
                ..TuiOptions::default()
            },
        );
        app.transcript.push_user("a question");
        assert!(!app.reserves_activity_rows(), "nothing has run yet");

        app.busy("thinking");
        app.finish_turn(false);
        assert!(
            app.reserves_activity_rows(),
            "held from the first turn onward"
        );
    }

    /// Options with the interface's margin turned off, for the tests that measure how
    /// content wraps and fills rather than where it sits.
    fn unpadded() -> TuiOptions {
        let mut options = TuiOptions::default();
        options.settings.interface_padding = 0;
        options
    }

    /// The interface keeps clear of the terminal's edges, on every side.
    #[test]
    fn the_interface_keeps_a_margin_around_itself() {
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("a question");
        let rows = paint(&mut app, 40, 14);

        assert!(rows.first().is_some_and(|row| row.is_empty()), "{rows:?}");
        for row in rows.iter().filter(|row| !row.is_empty()) {
            if row.chars().all(|character| character == '─') {
                continue;
            }
            assert!(row.starts_with(' '), "a row reaches the left edge: {row:?}");
            assert!(
                row.chars().count() < 40,
                "a row reaches the right edge: {row:?}"
            );
        }
    }

    /// Paint the way the real screen does: the whole terminal, with the interface laid out
    /// inside it.
    fn paint(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        screen(&terminal)
    }

    #[test]
    /// The interface takes the whole screen, opening at the top and keeping the input on the
    /// last rows however little there is to show.
    fn the_opening_screen_fills_the_terminal() {
        let mut app = App::new(&[], TuiOptions::default());
        let rows = paint(&mut app, 100, 50);

        assert_eq!(rows.len(), 50, "{rows:#?}");
        let logo = rows
            .iter()
            .position(|row| row.trim().starts_with("micro v"))
            .expect("the logo is drawn");
        // It rests just above the input rather than at the top of an empty screen.
        assert!(logo > 30, "the opening sits above the input, at row {logo}");
    }

    /// Nothing on the first screen is cut off: a line too long for the terminal takes
    /// another row instead of losing its end.
    #[test]
    fn the_opening_screen_wraps_rather_than_being_cut() {
        for width in [40u16, 56, 80] {
            let mut app = App::new(&[], unpadded());
            let rows = paint(&mut app, width, 50);
            let said = rows.join(" ");

            assert!(
                rows.iter().all(|row| text_width(row) <= width as usize),
                "a row ran past {width}: {rows:#?}"
            );
            for ending in ["ctrl+o more", "loaded resources.", "extend micro."] {
                assert!(
                    said.contains(ending),
                    "`{ending}` was cut at {width}: {said}"
                );
            }
        }
    }

    #[test]
    fn the_opening_screen_offers_its_hints() {
        let mut app = App::new(&[], TuiOptions::default());
        let rows = paint(&mut app, 100, 30);
        assert!(
            rows.iter().any(|row| {
                row.trim()
                == "escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more"
            }),
            "{rows:#?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("can explain its own features")),
            "{rows:#?}"
        );
    }

    #[test]
    /// A short conversation still fills the screen, and sits at the bottom of the transcript
    /// region so the newest message is the one next to the input.
    fn a_short_conversation_sits_above_the_input() {
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("hello");
        let rows = paint(&mut app, 80, 40);

        assert_eq!(rows.len(), 40, "{rows:#?}");
        let prompt = rows
            .iter()
            .position(|row| row.trim_start().starts_with("hello"))
            .expect("the prompt is drawn");
        assert!(prompt > 25, "it sits near the input rather than at the top");
    }

    #[test]
    fn a_long_conversation_fills_the_screen_and_hands_the_rest_to_the_terminal() {
        let mut app = App::new(&[], unpadded());
        for index in 0..40 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        let rows = paint(&mut app, 80, 24);

        assert!(rows.len() <= 24, "the interface never outgrows the screen");
        assert_eq!(
            rows.len(),
            24,
            "and fills it, with the input on the last rows"
        );
        // The footer takes the last two rows of the region, so the conversation reaches all
        // the way down to the input rather than stopping short of it.
        let footer = rows.len() - 2;
        assert!(
            rows[footer].trim().starts_with('~') || rows[footer].starts_with('/'),
            "{:?}",
            rows[footer]
        );
        assert!(
            rows[..footer - 3]
                .iter()
                .any(|row| row.contains("prompt number")),
            "{rows:#?}"
        );
    }

    /// Every message stays reachable: the conversation is scrolled within the region rather
    /// than handed away, so nothing is shown twice and nothing disappears.
    #[test]
    fn every_message_is_reachable_by_scrolling() {
        let mut app = App::new(&[], TuiOptions::default());
        for index in 0..30 {
            app.transcript.push_user(format!("prompt number {index}"));
        }
        paint(&mut app, 80, 24);
        app.refresh_lines();

        let seen: Vec<String> = app
            .lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect();

        for index in 0..30 {
            let prompt = format!("prompt number {index}");
            assert_eq!(
                seen.iter().filter(|row| **row == prompt).count(),
                1,
                "{prompt} appears exactly once"
            );
        }
    }

    /// The input is on the same rows whether or not a turn is running: the status area
    /// holds its two rows either way, so nothing below it moves when an answer begins.
    #[test]
    fn starting_a_turn_does_not_shift_the_input() {
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("hello");
        let idle = paint(&mut app, 80, 40);

        app.apply_event(AgentEvent::TurnStart);
        let running = paint(&mut app, 80, 40);

        let input = |rows: &[String]| {
            rows.iter()
                .rposition(|row| row.contains("Ask anything"))
                .expect("the prompt is drawn")
        };
        assert_eq!(input(&idle), input(&running));
    }

    #[test]
    fn insetting_never_underflows_a_narrow_area() {
        let narrow = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let inner = inset_by(narrow, 1);
        assert_eq!(inner.width, 0);
        assert!(inner.x <= narrow.x + narrow.width);
    }

    #[test]
    fn insetting_leaves_a_column_on_each_side() {
        let area = Rect {
            x: 0,
            y: 2,
            width: 40,
            height: 5,
        };
        let inner = inset_by(area, 1);
        assert_eq!(inner.x, 1);
        assert_eq!(inner.width, 38);
        assert_eq!(inner.height, 5);
    }
}
