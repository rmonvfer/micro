//! Kanban boards: columns of cards, side by side.
//!
//! A board is read left to right, column by column, so that is how it is
//! drawn: each column a heading over a stack of its own cards, the columns
//! placed side by side in the order they were declared. A card is a small
//! bordered box of its own rather than a plain line of text, because a
//! kanban board is a board of physical-feeling cards even when it is text.

use crate::canvas::{draw_text, Canvas, D, L, R, U};
use crate::labels::{clean_label, fit_label, strip_controls, WRAP_WIDTH};
use crate::types::Cls;
use crate::width::string_width;

/// Columns past this and the board is refused: this many side by side no
/// longer fits a terminal at a width worth reading.
const MAX_COLUMNS: usize = 12;
/// Cards past this in total and the board is refused, the same reasoning as
/// `graph::MAX_NODES`.
const MAX_TASKS: usize = 128;
/// Columns between one column and the next.
const GAP: usize = 2;

struct Task {
    label: String,
    assigned: Option<String>,
    priority: Option<String>,
}

struct Column {
    label: String,
    tasks: Vec<Task>,
}

/// Draw `src` as a kanban board, or answer nothing when it is not one.
pub(crate) fn render_kanban(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines();
    if lines.next()?.trim() != "kanban" {
        return None;
    }

    let mut columns: Vec<Column> = Vec::new();
    // The indentation of the first column line is what every later column
    // line is compared against; anything indented past it is a task
    // belonging to whichever column came before it.
    let mut column_indent: Option<usize> = None;
    let mut task_count = 0usize;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        let is_task = column_indent.is_some_and(|ci| indent > ci);
        if is_task {
            let column = columns.last_mut()?;
            task_count += 1;
            if task_count > MAX_TASKS {
                return None;
            }
            column.tasks.push(read_task(trimmed)?);
            continue;
        }

        column_indent.get_or_insert(indent);
        if columns.len() >= MAX_COLUMNS {
            return None;
        }
        columns.push(read_column(trimmed)?);
    }

    if columns.is_empty() {
        return None;
    }
    Some(draw(&columns))
}

/// `id` or `id[Title]`; the id itself is discarded once the label it stands
/// for is known, since nothing here is ever referenced back by id.
fn read_column(line: &str) -> Option<Column> {
    let (id, title) = read_id_and_label(line)?;
    Some(Column {
        label: title.unwrap_or(id),
        tasks: Vec::new(),
    })
}

/// `id[Task text]`, optionally followed by `@{ assigned: 'x', priority:
/// 'High' }`. Metadata keys this does not recognise are read past rather
/// than rejected, so a board carrying keys beyond these two still draws.
fn read_task(line: &str) -> Option<Task> {
    let open = line.find('[')?;
    let after_open = &line[open + 1..];
    let close = after_open.find(']')?;
    let label = clean_label(&after_open[..close]);
    if label.is_empty() {
        return None;
    }

    let mut assigned = None;
    let mut priority = None;
    let rest = after_open[close + 1..].trim();
    if !rest.is_empty() {
        let meta = rest
            .strip_prefix('@')?
            .trim_start()
            .strip_prefix('{')?
            .strip_suffix('}')?;
        for pair in meta.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once(':')?;
            let value = clean_label(value.trim());
            match key.trim() {
                "assigned" => assigned = Some(value),
                "priority" => priority = Some(value),
                _ => {}
            }
        }
    }
    Some(Task {
        label,
        assigned,
        priority,
    })
}

fn read_id_and_label(line: &str) -> Option<(String, Option<String>)> {
    match line.find('[') {
        Some(open) => {
            let id = line[..open].trim();
            if id.is_empty() {
                return None;
            }
            let rest = &line[open + 1..];
            let close = rest.rfind(']')?;
            Some((id.to_string(), Some(clean_label(&rest[..close]))))
        }
        None => {
            let id = line.trim();
            if id.is_empty() {
                None
            } else {
                Some((id.to_string(), None))
            }
        }
    }
}

/// A card's second line, `priority · assigned`, whichever of the two were
/// given. `None` when neither was, so a plain task keeps a plain card.
fn task_meta(task: &Task) -> Option<String> {
    let parts: Vec<&str> = [task.priority.as_deref(), task.assigned.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn card_height(task: &Task) -> usize {
    // Border, label, border — plus a fourth row for the metadata line when
    // there is one.
    3 + usize::from(task_meta(task).is_some())
}

fn column_height(col: &Column) -> usize {
    let mut h = 1;
    for (i, task) in col.tasks.iter().enumerate() {
        h += card_height(task);
        if i + 1 < col.tasks.len() {
            h += 1;
        }
    }
    h
}

fn column_width(col: &Column) -> usize {
    let mut inner = string_width(&fit_label(&col.label, WRAP_WIDTH));
    for task in &col.tasks {
        inner = inner.max(string_width(&fit_label(&task.label, WRAP_WIDTH)));
        if let Some(meta) = task_meta(task) {
            inner = inner.max(string_width(&fit_label(&meta, WRAP_WIDTH)));
        }
    }
    (inner + 4).max(5)
}

fn draw(columns: &[Column]) -> Canvas {
    let widths: Vec<usize> = columns.iter().map(column_width).collect();
    let content_h = columns.iter().map(column_height).max().unwrap_or(0);
    let total_w: usize = widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1);

    let mut canvas = Canvas::new(total_w.max(1), content_h.max(1));

    let mut x = 0;
    for (col, &w) in columns.iter().zip(&widths) {
        draw_text(
            &mut canvas,
            &fit_label(&col.label, WRAP_WIDTH),
            x,
            0,
            Cls::Text,
        );
        let mut y = 1;
        for task in &col.tasks {
            draw_card(&mut canvas, x, y, w, task);
            y += card_height(task) + 1;
        }
        x += w + GAP;
    }

    canvas.finalize_mask();
    canvas
}

fn draw_card(canvas: &mut Canvas, x: usize, y: usize, w: usize, task: &Task) {
    let meta = task_meta(task);
    let h = card_height(task);
    let right = x + w - 1;
    let bottom = y + h - 1;

    canvas.set(x, y, "┌", Cls::Border);
    canvas.set(right, y, "┐", Cls::Border);
    canvas.set(x, bottom, "└", Cls::Border);
    canvas.set(right, bottom, "┘", Cls::Border);
    for cx in x + 1..right {
        canvas.add_bits(cx, y, L | R, Cls::Border);
        canvas.add_bits(cx, bottom, L | R, Cls::Border);
    }
    for cy in y + 1..bottom {
        canvas.add_bits(x, cy, U | D, Cls::Border);
        canvas.add_bits(right, cy, U | D, Cls::Border);
    }

    // One cell of padding inside the border on every side, the same as any
    // other bordered box this crate draws.
    let inner = w.saturating_sub(4);
    draw_text(
        canvas,
        &fit_label(&task.label, inner),
        x + 2,
        y + 1,
        Cls::Text,
    );
    if let Some(meta) = meta {
        draw_text(
            canvas,
            &fit_label(&meta, inner),
            x + 2,
            y + 2,
            Cls::EdgeLabel,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_kanban(src)
            .expect("it is a kanban board")
            .to_lines()
            .plain
    }

    /// A task is drawn as its own bordered card under its column's heading.
    #[test]
    fn a_task_is_drawn_as_a_card_under_its_column() {
        let rows = drawn("kanban\n  todo[To Do]\n    t1[Write docs]");
        assert_eq!(rows[0], "To Do");
        assert!(rows[1].starts_with('┌'), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("Write docs")), "{rows:?}");
        assert!(rows.last().unwrap().starts_with('└'), "{rows:?}");
    }

    /// Priority and assignee draw as a second line on the card.
    #[test]
    fn priority_and_assignee_draw_as_the_cards_second_line() {
        let rows =
            drawn("kanban\n  todo[To Do]\n    t1[Ship it]@{ assigned: 'Alice', priority: 'High' }");
        assert!(rows.iter().any(|r| r.contains("High · Alice")), "{rows:?}");
    }

    /// Columns are drawn side by side, left to right in declaration order.
    #[test]
    fn columns_are_drawn_side_by_side() {
        let rows = drawn("kanban\n  todo[To Do]\n    t1[A]\n  done[Done]\n    t2[B]");
        assert_eq!(rows[0], "To Do      Done");
        let todo_col = rows[0].find("To Do").unwrap();
        let done_col = rows[0].find("Done").unwrap();
        assert!(todo_col < done_col, "{rows:?}");
    }

    /// An empty column still draws its heading with nothing under it.
    #[test]
    fn an_empty_column_still_draws_its_heading() {
        let rows = drawn("kanban\n  todo[To Do]\n  done[Done]");
        assert_eq!(rows, vec!["To Do      Done"]);
    }

    /// Anything that is not a kanban board, or is one but malformed, is
    /// refused rather than guessed at.
    #[test]
    fn what_is_not_a_kanban_board_is_left_alone() {
        assert!(render_kanban("graph TD\n A --> B").is_none());
        assert!(render_kanban("kanban").is_none(), "no columns at all");
        assert!(
            render_kanban("kanban\n  [Column with no id]").is_none(),
            "a column with no id"
        );
        assert!(render_kanban("kanban\n  todo[To Do]\n    t1[No closing bracket").is_none());
    }

    /// A board this wide is refused rather than squeezed into a terminal.
    #[test]
    fn too_many_columns_are_refused() {
        let mut source = String::from("kanban\n");
        for index in 0..MAX_COLUMNS + 1 {
            source.push_str(&format!("  c{index}[Column {index}]\n"));
        }
        assert!(render_kanban(&source).is_none());
    }
}
