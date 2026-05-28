//! User journeys, drawn as sections of tasks scored out of five.

use crate::canvas::draw_text;
use crate::canvas::Canvas;
use crate::labels::{clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;


const MAX_TASKS: usize = 64;

/// Mermaid scores a journey out of five.
const SCALE: usize = 5;

enum Row {
    Section(String),
    Task {
        label: String,
        score: usize,
        actors: Vec<String>,
    },
}

/// Draw `src` as a user journey, or answer nothing when it is not one.
pub(crate) fn render_journey(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|line| !line.is_empty());

    if lines.next()? != "journey" {
        return None;
    }

    let mut title = None;
    let mut rows: Vec<Row> = Vec::new();
    let mut tasks = 0usize;

    for line in lines {
        if let Some(named) = line.strip_prefix("title ") {
            title = Some(clean_label(named.trim()));
            continue;
        }
        if let Some(named) = line.strip_prefix("section ") {
            let name = clean_label(named.trim());
            if name.is_empty() {
                return None;
            }
            rows.push(Row::Section(name));
            continue;
        }

        
        let mut parts = line.split(':').map(str::trim);
        let label = clean_label(parts.next()?);
        let score: usize = parts.next()?.trim().parse().ok()?;
        if label.is_empty() || score > SCALE {
            return None;
        }
        let actors = parts
            .next()
            .map(|named| {
                named
                    .split(',')
                    .map(str::trim)
                    .filter(|actor| !actor.is_empty())
                    .map(clean_label)
                    .collect()
            })
            .unwrap_or_default();

        tasks += 1;
        if tasks > MAX_TASKS {
            return None;
        }
        rows.push(Row::Task {
            label,
            score,
            actors,
        });
    }

    if tasks == 0 {
        return None;
    }
    Some(draw(title.as_deref(), &rows))
}

fn draw(title: Option<&str>, rows: &[Row]) -> Canvas {
    
    const INDENT: usize = 2;

    let widest_label = rows
        .iter()
        .map(|row| match row {
            Row::Task { label, .. } => INDENT + string_width(label),
            Row::Section(name) => string_width(name),
        })
        .max()
        .unwrap_or(0);

    let actors: Vec<String> = rows
        .iter()
        .map(|row| match row {
            Row::Task { actors, .. } => actors.join(", "),
            Row::Section(_) => String::new(),
        })
        .collect();
    let widest_actors = actors.iter().map(|a| string_width(a)).max().unwrap_or(0);

    let score_at = widest_label + 1;
    let actors_at = score_at + SCALE + 1;
    let width = (actors_at + widest_actors).max(string_width(title.unwrap_or("")));
    let top = usize::from(title.is_some());
    let mut canvas = Canvas::new(width.max(1), rows.len() + top);

    if let Some(title) = title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }

    for (index, row) in rows.iter().enumerate() {
        let y = index + top;
        match row {
            Row::Section(name) => draw_text(&mut canvas, name, 0, y, Cls::Title),
            Row::Task { label, score, .. } => {
                draw_text(&mut canvas, label, INDENT, y, Cls::Text);

                
                let marks: String = (0..SCALE)
                    .map(|mark| match mark < *score {
                        true => '●',
                        false => '○',
                    })
                    .collect();
                draw_text(&mut canvas, &marks, score_at, y, Cls::EdgeLabel);

                if !actors[index].is_empty() {
                    draw_text(&mut canvas, &actors[index], actors_at, y, Cls::Text);
                }
            }
        }
    }

    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_journey(src)
            .expect("it is a journey")
            .to_lines()
            .plain
    }

    /// A task's score is drawn against a fixed scale, so two tasks compare by eye.
    #[test]
    fn a_task_is_scored_against_a_fixed_scale() {
        let rows = drawn("journey\n  title Signing up\n  section Start\n    Find the page: 5: Me\n    Fill the form: 2: Me, You");
        assert_eq!(rows[0], "Signing up");
        assert_eq!(rows[1], "Start");
        assert!(rows[2].contains("●●●●●"), "a five is full: {rows:?}");
        assert!(rows[3].contains("●●○○○"), "a two is not: {rows:?}");
        assert!(rows[3].trim_end().ends_with("Me, You"), "{rows:?}");
    }

    /// Tasks sit under their section, so the shape reads before the words do.
    #[test]
    fn tasks_are_indented_under_their_section() {
        let rows = drawn("journey\n  section One\n    A task: 3: Me");
        assert!(rows[0].starts_with("One"), "{rows:?}");
        assert!(rows[1].starts_with("  A task"), "{rows:?}");
    }

    /// Nobody has to be named for a step to have gone well or badly.
    #[test]
    fn a_task_with_nobody_named_still_has_its_score() {
        let rows = drawn("journey\n  Alone: 4");
        assert!(rows[0].contains("●●●●○"), "{rows:?}");
    }

    
    #[test]
    fn what_is_not_a_journey_is_left_alone() {
        assert!(render_journey("graph TD\n  A --> B").is_none());
        assert!(
            render_journey("journey").is_none(),
            "a journey with no steps"
        );
        assert!(render_journey("journey\n  section Only").is_none());
        assert!(render_journey("journey\n  No score here").is_none());
        assert!(
            render_journey("journey\n  Too good: 9: Me").is_none(),
            "the scale is five, so nine is not a score"
        );
    }

    
    #[test]
    fn too_many_tasks_are_refused() {
        let mut source = String::from("journey\n");
        for index in 0..MAX_TASKS + 1 {
            source.push_str(&format!("  Step {index}: 3: Me\n"));
        }
        assert!(render_journey(&source).is_none());
    }
}
