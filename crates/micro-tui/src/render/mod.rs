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
mod menu;
mod overlay;
pub mod hints;
pub mod links;
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

/// Rows the input may grow to before it scrolls internally.
const MAX_EDITOR_ROWS: usize = 10;
/// The share of the screen an approval prompt may claim. It takes what its content needs up
/// to this, and never so much that the conversation behind it disappears.
const MAX_PROMPT_SHARE: u16 = 3;
/// Rows kept for the spinner: a blank one, then the message.
///
/// Held from the first turn onward, whether or not one is running, so starting one never
/// shifts the interface vertically. Before then they are not held at all: a screen that
/// has done nothing has nothing to say there, and ohm leaves the same space empty.
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

    let output_padding = app.settings().output_padding;
    let editor_padding = app.settings().editor_padding;
    let content_width = content_width(area.width, output_padding);
    // The frame is measured before anything is laid out against it: the transcript wraps to
    // this width, and a page of scrolling moves by the rows the region turns out to have.
    app.set_frame(content_width as usize, area.height);
    let chrome = chrome(app, &theme, content_width);
    let transcript_rows = area.height.saturating_sub(chrome.rows());
    app.set_viewport(transcript_rows as usize);

    app.refresh_lines();

    // Before anything has happened there is no conversation, and the screen introduces
    // itself in the space one would have taken.
    let opening = match app.lines().is_empty() && !app.settings().quiet_startup {
        true => intro(&theme, content_width as usize),
        false => Vec::new(),
    };

    let [transcript_area, _, overlay_area, activity_area, editor_area, menu_area, status_area] =
        Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(chrome.overlay.len() as u16),
            Constraint::Length(activity_rows(app)),
            Constraint::Length(chrome.editor),
            Constraint::Length(chrome.menu.len() as u16),
            Constraint::Length(status::HEIGHT),
        ])
        .areas(area);

    let overlay = chrome.overlay;
    let menu = chrome.menu;

    draw_transcript(frame, transcript_area, app, &opening);
    draw_rows(
        frame,
        overlay_area,
        inset_by(overlay_area, output_padding),
        &overlay,
        &theme,
    );
    draw_activity(frame, inset_by(activity_area, output_padding), app, &theme);
    // An overlay has the keyboard while it is up, so the cursor belongs to it rather than to
    // an input the next keystroke will not reach.
    let level = app.thinking_color();
    editor::draw(
        frame,
        editor_area,
        inset_by(editor_area, editor_padding),
        &app.editor,
        &theme,
        editor::Look {
            level,
            focused: !app.overlay_is_open(),
            hardware_cursor: app.settings().show_hardware_cursor,
        },
    );
    for (offset, line) in menu.iter().take(menu_area.height as usize).enumerate() {
        let content = inset_by(menu_area, editor_padding);
        frame
            .buffer_mut()
            .set_line(content.x, content.y + offset as u16, line, content.width);
    }
    draw_status(frame, inset_by(status_area, output_padding), app, &theme);

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
}

impl Chrome {
    /// The blank row, whatever is open, the spinner's row, the input, its menu, the footer.
    fn rows(&self) -> u16 {
        1 + self.overlay.len() as u16
            + self.activity
            + self.editor
            + self.menu.len() as u16
            + status::HEIGHT
    }
}

fn chrome(app: &App, theme: &Theme, width: u16) -> Chrome {
    Chrome {
        overlay: overlay_lines(app, theme, width as usize),
        menu: app
            .menu()
            .map(|menu| menu::lines(menu, theme, width as usize, app.menu_rows()))
            .unwrap_or_default(),
        // The prompt's own rows, plus the rule above it and the rule below it.
        editor: app.editor.height(width as usize).clamp(1, MAX_EDITOR_ROWS) as u16 + editor::RULES,
        activity: activity_rows(app),
    }
}

/// The rows of whatever overlay is up, in the order of what is blocking on an answer: a
/// credential first, then a list to choose from.
fn overlay_lines(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    // Sized against the screen, not against the region: the region is sized by what this
    // turns out to need, so measuring it against itself would never settle.
    let budget = (app.rows() / MAX_PROMPT_SHARE).max(4) as usize;

    if let Some(prompt) = app.key_prompt() {
        return overlay::key_prompt_lines(prompt, theme, width);
    }
    match app.picker() {
        Some(picker) => overlay::picker_lines(picker, theme, width, budget),
        None => Vec::new(),
    }
}

/// Paint a block of already-wrapped rows, filling the region behind them.
fn draw_rows(frame: &mut Frame, area: Rect, content: Rect, rows: &[Line<'static>], theme: &Theme) {
    if area.height == 0 || content.width == 0 {
        return;
    }
    frame
        .buffer_mut()
        .set_style(area, Style::new().bg(theme.surface));
    for (offset, line) in rows.iter().take(area.height as usize).enumerate() {
        frame
            .buffer_mut()
            .set_line(content.x, content.y + offset as u16, line, content.width);
    }
}

fn draw_transcript(frame: &mut Frame, area: Rect, app: &App, opening: &[Line<'static>]) {
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

/// The keys worth knowing on the first screen, in ohm's order and ohm's words. A key is
/// dim and what it does is muted, which is how every hint in the interface is written.
const HINTS: [(&str, &str); 5] = [
    ("escape", "interrupt"),
    ("ctrl+c/ctrl+d", "clear/exit"),
    ("/", "commands"),
    ("!", "bash"),
    ("ctrl+o", "more"),
];

/// What the first screen says before anything has happened: what this is, the keys worth
/// knowing, and that it can describe itself. Where the session is belongs to the footer,
/// which says it on every screen rather than only on this one.
fn intro(theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let dim = Style::new().fg(theme.dim);

    let logo = [
        Span::styled(
            APP_NAME.to_string(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" v{}", env!("CARGO_PKG_VERSION")), dim),
    ];

    // One shape for every hint in the interface, so a key is described the same way here as
    // it is anywhere else — and reads as `option` on a Mac.
    let hints = hints::hints(&HINTS, theme);

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
    out.extend(wrap_spans(&hints, width, 0));
    out.push(Line::default());
    out.extend(wrap_spans(&onboarding, width, 0));
    out
}

fn draw_activity(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.width == 0 || area.height == 0 || !app.is_running() {
        return;
    }
    let line = status::activity_line(
        theme,
        app.tick,
        app.elapsed(),
        app.is_interrupting(),
        app.activity(),
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

fn draw_status(frame: &mut Frame, content: Rect, app: &App, theme: &Theme) {
    if content.width == 0 || content.height == 0 {
        return;
    }
    let footer = status::Footer {
        cwd: &app.cwd,
        total: app.transcript.total_usage(),
        last: app.transcript.last_usage(),
        context_window: app.context_window,
        model: app.model_id(),
        thinking: Some(crate::app::thinking_name(app.thinking)),
        attachments: app.attachments(),
        ..status::Footer::default()
    };
    for (offset, line) in footer
        .rows(theme, content.width as usize)
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

        assert_eq!(gap(&paint(&mut app, 40, 12)), 1, "nothing has been worked on");

        app.busy("thinking");
        app.finish_turn(false);
        assert_eq!(
            gap(&paint(&mut app, 40, 12)),
            3,
            "the spinner's two rows are held from the first turn onward"
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
        let rows = paint(&mut app, 40, 12);

        assert!(rows.first().is_some_and(|row| row.is_empty()), "{rows:?}");
        assert!(rows.last().is_some_and(|row| row.is_empty()), "{rows:?}");
        for row in rows.iter().filter(|row| !row.is_empty()) {
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

    #[test]
    fn the_opening_screen_wraps_rather_than_being_cut() {
        let mut app = App::new(&[], unpadded());
        let rows = paint(&mut app, 80, 50);

        // Two lines that do not fit at this width take a second row each.
        let logo = rows
            .iter()
            .position(|row| row.trim().starts_with("micro v"))
            .expect("the logo is drawn");
        assert_eq!(rows[logo + 2].trim(), "more");
        assert_eq!(rows[logo + 5].trim(), "extend micro.");
    }

    #[test]
    fn the_opening_screen_offers_ohms_hints() {
        let mut app = App::new(&[], TuiOptions::default());
        let rows = paint(&mut app, 100, 30);
        assert!(
            rows.iter().any(|row| row.trim()
                == "escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more"),
            "{rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("can explain its own features")),
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
        assert_eq!(rows.len(), 24, "and fills it, with the input on the last rows");
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

