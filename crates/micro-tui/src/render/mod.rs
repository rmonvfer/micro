//! Frame layout.

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

const EDITOR_SHARE: f32 = 0.3;
const MIN_EDITOR_ROWS: usize = 5;

/// Rows the input may grow to on a screen this tall.
fn max_editor_rows(rows: u16) -> usize {
    ((rows as f32 * EDITOR_SHARE) as usize).max(MIN_EDITOR_ROWS)
}
/// Rows kept for the conversation behind an overlay, so opening one never hides all of it.
const ROWS_BEHIND_OVERLAY: u16 = 1;
/// Rows kept for the spinner: a blank one, then the message.
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
        app.set_placements(Vec::new());
        return;
    }
    let theme = app.theme;

    let content_padding = app.settings().content_padding;
    let content_width = content_width(area.width, content_padding);

    app.set_frame(content_width as usize, area.height);
    let chrome = chrome(app, &theme, area.width, area.height);
    let transcript_rows = area.height.saturating_sub(chrome.rows());
    app.set_viewport(transcript_rows as usize);

    app.refresh_lines();

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
    set_component_overlay_cursor(frame, overlay_area, app);
    draw_activity(frame, inset_by(activity_area, content_padding), app, &theme);
    draw_rows(
        frame,
        widgets_above_area,
        inset_by(widgets_above_area, content_padding),
        &widgets_above,
        &theme,
        false,
    );

    let level = app.thinking_color();
    match app.editor_component_id() {
        Some(_) => editor::draw_component(
            frame,
            editor_area,
            inset_by(editor_area, content_padding),
            app.editor_component_lines(),
            app.editor_component_cursor().map(|(row, byte)| {
                let column = app
                    .editor_component_lines()
                    .get(row)
                    .map(|line| text_width(&line[..byte]))
                    .unwrap_or(0);
                (row, column)
            }),
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

    app.links().apply(frame.buffer_mut(), area);

    let first_visible = app
        .lines()
        .len()
        .saturating_sub(transcript_rows as usize)
        .saturating_sub(app.scroll());
    let shown = app
        .lines()
        .len()
        .saturating_sub(first_visible)
        .min(transcript_area.height as usize);
    let top = transcript_area.y + (transcript_area.height as usize - shown) as u16;

    let placements = app
        .pictures()
        .placements(transcript_area, first_visible, top);
    app.set_placements(placements);
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
    fn stack(&self, height: Option<usize>) -> crate::layout::Stack {
        use crate::layout::{Child, Lines, Spacer, Stack};
        let stack = match height {
            Some(height) => Stack::within(height).with(Child::flexible(Spacer(0), 1)),
            None => Stack::new(),
        };
        stack
            .with(Child::content(Spacer(1)))
            .with(Child::content(Spacer(self.activity as usize)))
            .with(Child::content(Lines(self.overlay.clone())))
            .with(Child::content(Lines(self.widgets_above.clone())))
            .with(Child::content(Spacer(self.editor as usize)))
            .with(Child::content(Lines(self.widgets_below.clone())))
            .with(Child::content(Lines(self.menu.clone())))
            .with(Child::content(Spacer(self.status as usize)))
    }
}

/// How many rows the interface itself needs, apart from the conversation.
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
    let mut widgets_above = queue_lines(app, theme, width as usize);
    widgets_above.extend(widget_lines(app.widgets_above(), theme));
    let widgets_below = widget_lines(app.widgets_below(), theme);

    let editor = match overlay.is_empty() {
        true => {
            let content_rows = match app.editor_component_id() {
                Some(_) => app.editor_component_lines().len().max(1),
                None => app
                    .editor
                    .height(content_width(width, app.settings().content_padding) as usize),
            };
            content_rows.clamp(1, max_editor_rows(height)) as u16 + editor::RULES
        }
        false => 0,
    };
    let status = footer_height(app);

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

fn queue_lines(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let queued = app.queued_messages();
    if queued.is_empty() {
        return Vec::new();
    }

    let mut lines = vec![Line::default()];
    for message in queued {
        let label = match message.kind {
            crate::app::QueueKind::Steering => "Steering",
            crate::app::QueueKind::FollowUp => "Follow-up",
        };
        lines.push(clip(
            Line::styled(
                format!("{label}: {}", message.text),
                Style::new().fg(theme.dim),
            ),
            width,
        ));
    }
    lines.push(Line::from(vec![Span::styled(
        "↳ alt+up to edit all queued messages",
        Style::new().fg(theme.dim),
    )]));
    lines
}

fn widget_lines(widgets: Vec<Vec<String>>, theme: &Theme) -> Vec<Line<'static>> {
    widgets
        .into_iter()
        .flatten()
        .map(|line| Line::styled(line, theme.body()))
        .collect()
}

fn overlay_lines(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let held = ROWS_BEHIND_OVERLAY + activity_rows(app) + footer_height(app) + 1;
    let budget = app.rows().saturating_sub(held).max(4) as usize;

    if let Some(prompt) = app.key_prompt() {
        return overlay::key_prompt_lines(prompt, theme, width);
    }
    if let Some((title, _text, items, body, selected, detail_open, scroll)) = app.inspection() {
        return overlay::inspection_lines(overlay::Inspection {
            title,
            items,
            body,
            selected,
            detail_open,
            scroll,
            theme,
            width,
            budget,
        });
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

/// Place the terminal cursor where an open custom component requested it.
fn set_component_overlay_cursor(frame: &mut Frame, area: Rect, app: &App) {
    let (Some(lines), Some((row, byte))) = (
        app.component_overlay_lines(),
        app.component_overlay_cursor(),
    ) else {
        return;
    };
    let column = lines
        .get(row)
        .map(|line| text_width(&line[..byte]))
        .unwrap_or(0);
    if row < area.height as usize {
        frame.set_cursor_position((
            area.x + (column as u16).min(area.width.saturating_sub(1)),
            area.y + row as u16,
        ));
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

    let first = rows
        .len()
        .saturating_sub(height)
        .saturating_sub(app.scroll());
    let shown = rows.len().saturating_sub(first).min(height);

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

/// Cut a row at the width.
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

/// The keys worth knowing on the first screen.
const HINTS: [(&str, &str); 5] = [
    ("escape", "interrupt"),
    ("ctrl+c/ctrl+d", "clear/exit"),
    ("/", "commands"),
    ("!", "bash"),
    ("ctrl+o", "more"),
];

/// Every key, for the reader who asked for all of them.
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

    let mut out = vec![Line::default()];
    out.extend(wrap_spans(&logo, width, 0));

    match expanded {
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
    let line = match app.is_compacting() {
        true => status::compaction_line(theme, app.indicator_frame()),
        false => status::activity_line(
            theme,
            app.indicator_frame(),
            app.elapsed(),
            app.is_interrupting(),
            &app.activity(),
        ),
    };
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

    let row = area.y + u16::from(area.height > 1);
    frame.buffer_mut().set_line(area.x, row, &line, area.width);
}

/// Everything the footer reports about the session as it stands.
fn footer_for(app: &App) -> status::Footer<'_> {
    status::Footer {
        cwd: &app.cwd,
        branch: app.branch(),
        session: None,
        total: app.total_usage(),
        last: app.last_usage(),
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

    /// Once `setEditorComponent` has replaced the built-in editor, its lines are what is drawn in
    /// the input's place.
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

    /// A menu opens under the input and can be long.
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

    /// The rows the spinner draws in are not held open until something has been worked on.
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

    /// Drawing inline, the region is only as tall as the interface.
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

    /// Paint the way the real screen does: the whole terminal, with the interface laid out inside
    /// it.
    fn paint(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        screen(&terminal)
    }

    #[test]
    /// The interface takes the whole screen, opening at the top and keeping the input on the last
    /// rows however little there is to show.
    fn the_opening_screen_fills_the_terminal() {
        let mut app = App::new(&[], TuiOptions::default());
        let rows = paint(&mut app, 100, 50);

        assert_eq!(rows.len(), 50, "{rows:#?}");
        let logo = rows
            .iter()
            .position(|row| row.trim().starts_with("micro v"))
            .expect("the logo is drawn");

        assert!(logo > 30, "the opening sits above the input, at row {logo}");
    }

    /// Nothing on the first screen is cut off: a line too long for the terminal takes another row
    /// instead of losing its end.
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

    fn a_short_conversation_sits_above_the_input() {
        let mut app = App::new(&[], TuiOptions::default());
        app.transcript.push_user("hello");
        let rows = paint(&mut app, 80, 40);

        assert_eq!(rows.len(), 40, "{rows:#?}");
        let prompt = rows
            .iter()
            .position(|row| row.trim_start().starts_with("hello"))
            .expect("the prompt is drawn");
        assert!(prompt > 25, "prompt should remain near the input");
    }

    #[test]
    fn a_wrapped_prompt_receives_all_of_its_rows() {
        let mut app = App::new(&[], unpadded());
        app.editor.insert_str("abcdefghijklmnopqrst");

        let rows = paint(&mut app, 20, 12);

        assert!(
            rows.iter().any(|row| row.contains("abcdefghijklmnopqr")),
            "the first wrapped row is visible: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("st")),
            "the final wrapped row is visible: {rows:?}"
        );
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

    /// The input is on the same rows whether or not a turn is running: the status area holds its
    /// two rows either way.
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
