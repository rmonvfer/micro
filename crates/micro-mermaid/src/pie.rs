//! Pie charts, drawn as bars.

use crate::canvas::draw_text;
use crate::canvas::Canvas;
use crate::labels::{clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

const MAX_SLICES: usize = 64;

/// The widest a bar is drawn, so a chart stays inside a terminal without being told its width.
const BAR: usize = 32;

struct Slice {
    label: String,
    value: f64,
}

/// Draw `src` as a pie chart, or answer nothing when it is not one.
pub(crate) fn render_pie(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|line| !line.is_empty());

    let opening = lines.next()?;

    let mut words = opening.split_whitespace();
    if words.next()? != "pie" {
        return None;
    }
    let mut title = None;
    if let Some(rest) = opening.strip_prefix("pie") {
        let rest = rest.trim().trim_start_matches("showData").trim();
        if let Some(named) = rest.strip_prefix("title") {
            title = Some(clean_label(named.trim()));
        }
    }

    let mut slices: Vec<Slice> = Vec::new();
    for line in lines {
        if let Some(named) = line.strip_prefix("title") {
            title = Some(clean_label(named.trim()));
            continue;
        }
        let slice = read_slice(line)?;
        if slices.len() >= MAX_SLICES {
            return None;
        }
        slices.push(slice);
    }

    if slices.is_empty() {
        return None;
    }
    draw(title.as_deref(), &slices)
}

/// One `"Label" : 42` row.
fn read_slice(line: &str) -> Option<Slice> {
    let (label, value) = line.rsplit_once(':')?;
    let label = label.trim().trim_matches('"').trim();
    let value: f64 = value.trim().parse().ok()?;

    if label.is_empty() || !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(Slice {
        label: clean_label(label),
        value,
    })
}

fn draw(title: Option<&str>, slices: &[Slice]) -> Option<Canvas> {
    let total: f64 = slices.iter().map(|slice| slice.value).sum();
    if total <= 0.0 {
        return None;
    }
    let widest = slices
        .iter()
        .map(|slice| slice.value)
        .fold(0.0_f64, f64::max);

    let labels = slices
        .iter()
        .map(|slice| string_width(&slice.label))
        .max()
        .unwrap_or(0);
    let amounts: Vec<String> = slices
        .iter()
        .map(|slice| {
            format!(
                "{} ({:.1}%)",
                trim_number(slice.value),
                share(slice.value, total)
            )
        })
        .collect();
    let widest_amount = amounts
        .iter()
        .map(|text| string_width(text))
        .max()
        .unwrap_or(0);

    let width = labels + 1 + BAR + 1 + widest_amount;
    let top = usize::from(title.is_some());
    let mut canvas = Canvas::new(
        width.max(string_width(title.unwrap_or(""))),
        slices.len() + top,
    );

    if let Some(title) = title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }

    for (row, (slice, amount)) in slices.iter().zip(&amounts).enumerate() {
        let y = row + top;
        draw_text(&mut canvas, &slice.label, 0, y, Cls::Text);

        let filled = match widest > 0.0 {
            true => ((slice.value / widest) * BAR as f64).round() as usize,
            false => 0,
        }
        .clamp(usize::from(slice.value > 0.0), BAR);
        let bar: String = "█".repeat(filled);
        draw_text(&mut canvas, &bar, labels + 1, y, Cls::Border);
        draw_text(&mut canvas, amount, labels + 1 + BAR + 1, y, Cls::EdgeLabel);
    }

    Some(canvas)
}

fn share(value: f64, total: f64) -> f64 {
    (value / total) * 100.0
}

/// A value written the way it was meant: whole numbers without a decimal point.
fn trim_number(value: f64) -> String {
    match value.fract() == 0.0 && value.abs() < 1e15 {
        true => format!("{}", value as i64),
        false => format!("{value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_pie(src).expect("it is a pie chart").to_lines().plain
    }

    /// Each slice is a bar against the largest, with its value and share beside it.
    #[test]
    fn a_pie_is_drawn_as_bars_in_proportion() {
        let rows = drawn("pie title Pets\n  \"Dogs\" : 75\n  \"Cats\" : 25");
        assert_eq!(rows[0], "Pets");
        assert!(rows[1].starts_with("Dogs "), "{rows:?}");
        assert!(rows[1].contains("75 (75.0%)"), "{rows:?}");
        assert!(rows[2].contains("25 (25.0%)"), "{rows:?}");

        assert_eq!(rows[1].matches('█').count(), BAR);
        assert_eq!(
            rows[2].matches('█').count(),
            11,
            "a third of {BAR}, rounded"
        );
    }

    /// A slice worth nothing still has a row, because it was written down.
    #[test]
    fn a_slice_of_nothing_keeps_its_row() {
        let rows = drawn("pie\n  \"None\" : 0\n  \"All\" : 10");
        assert!(rows[0].contains("0 (0.0%)"), "{rows:?}");
        assert!(!rows[0].contains('█'), "and no bar: {rows:?}");
    }

    /// `showData` asks for the values, which are always written, so it changes nothing but must not
    /// be read as a title.
    #[test]
    fn show_data_is_understood_and_is_not_a_title() {
        let rows = drawn("pie showData\n  \"One\" : 1");
        assert!(rows[0].starts_with("One "), "{rows:?}");
    }

    #[test]
    fn what_is_not_a_pie_is_left_alone() {
        assert!(render_pie("graph TD\n A --> B").is_none());
        assert!(render_pie("pie").is_none(), "a chart with no slices");
        assert!(render_pie("pie\n  \"Bad\" : nonsense").is_none());
        assert!(render_pie("pie\n  \"Negative\" : -5").is_none());
        assert!(
            render_pie("pie\n  \"Nothing\" : 0").is_none(),
            "nothing to divide"
        );
    }

    /// A chart of hundreds of slices says nothing, so it is not drawn.
    #[test]
    fn too_many_slices_are_refused() {
        let mut source = String::from("pie\n");
        for index in 0..MAX_SLICES + 1 {
            source.push_str(&format!("  \"Slice {index}\" : 1\n"));
        }
        assert!(render_pie(&source).is_none());
    }
}
