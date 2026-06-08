use std::collections::BTreeMap;

use crate::canvas::draw_text;
use crate::canvas::Canvas;
use crate::labels::{ascii_lower, clean_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Flows past this and there is nothing left to read as a shape.
const MAX_FLOWS: usize = 256;

/// The widest a bar is drawn.
const BAR: usize = 28;

/// How far a flow is indented under the node it leaves.
const INDENT: usize = 2;

struct Flow {
    source: String,
    target: String,
    value: f64,
}

/// Draw `src` as a sankey diagram, or answer nothing when it is not one.
pub(crate) fn render_sankey(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let mut lines = src.lines().map(str::trim).filter(|line| !line.is_empty());

    let header = lines.next()?;
    if ascii_lower(header.split_whitespace().next()?) != "sankey-beta" {
        return None;
    }

    let mut flows: Vec<Flow> = Vec::new();
    for line in lines {
        let flow = read_flow(line)?;
        if flows.len() >= MAX_FLOWS {
            return None;
        }
        flows.push(flow);
    }

    if flows.is_empty() {
        return None;
    }
    Some(draw(&flows))
}

/// One `source,target,value` row, with quoted fields allowed since a name may hold a comma.
fn read_flow(line: &str) -> Option<Flow> {
    let fields = split_fields(line);
    if fields.len() != 3 {
        return None;
    }
    let value: f64 = fields[2].trim().parse().ok()?;

    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let source = clean_label(&fields[0]);
    let target = clean_label(&fields[1]);
    if source.is_empty() || target.is_empty() {
        return None;
    }
    Some(Flow {
        source,
        target,
        value,
    })
}

/// The comma-separated fields of a row, honouring quotes so a name may hold a comma.
fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;

    for character in line.chars() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(character),
        }
    }
    fields.push(field);
    fields.into_iter().map(|f| f.trim().to_string()).collect()
}

fn draw(flows: &[Flow]) -> Canvas {
    let mut order: Vec<&str> = Vec::new();
    let mut grouped: BTreeMap<&str, Vec<&Flow>> = BTreeMap::new();
    for flow in flows {
        if !grouped.contains_key(flow.source.as_str()) {
            order.push(&flow.source);
        }
        grouped.entry(&flow.source).or_default().push(flow);
    }

    let widest = flows.iter().map(|flow| flow.value).fold(0.0_f64, f64::max);
    let amounts: Vec<String> = flows.iter().map(|flow| trim_number(flow.value)).collect();
    let widest_amount = amounts.iter().map(|a| string_width(a)).max().unwrap_or(0);

    let widest_target = flows
        .iter()
        .map(|flow| INDENT + 2 + string_width(&flow.target))
        .max()
        .unwrap_or(0);
    let widest_source = order
        .iter()
        .map(|name| string_width(name))
        .max()
        .unwrap_or(0);
    let label_width = widest_target.max(widest_source + widest_amount + 3);

    let bar_at = label_width + 1;
    let width = bar_at + BAR + 1 + widest_amount;
    let rows = order.len() + flows.len();
    let mut canvas = Canvas::new(width.max(1), rows);

    let mut y = 0;
    for source in &order {
        let out: f64 = grouped[source].iter().map(|flow| flow.value).sum();

        let heading = format!("{source} ({})", trim_number(out));
        draw_text(&mut canvas, &heading, 0, y, Cls::Title);
        y += 1;

        for flow in &grouped[*source] {
            let target = format!("→ {}", flow.target);
            draw_text(&mut canvas, &target, INDENT, y, Cls::Text);

            let filled = match widest > 0.0 {
                true => ((flow.value / widest) * BAR as f64).round() as usize,
                false => 0,
            }
            .clamp(1, BAR);
            draw_text(&mut canvas, &"█".repeat(filled), bar_at, y, Cls::Border);
            draw_text(
                &mut canvas,
                &trim_number(flow.value),
                bar_at + BAR + 1,
                y,
                Cls::EdgeLabel,
            );
            y += 1;
        }
    }

    canvas
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
        render_sankey(src).expect("it is a sankey").to_lines().plain
    }

    /// Flows are grouped under the node they leave, with that node's total beside it.
    #[test]
    fn flows_are_grouped_under_where_they_leave_from() {
        let rows =
            drawn("sankey-beta\nAgriculture,Food,120\nAgriculture,Waste,30\nFood,Households,90");
        assert!(rows[0].starts_with("Agriculture (150)"), "{rows:?}");
        assert!(rows[1].contains("→ Food"), "{rows:?}");
        assert!(rows[2].contains("→ Waste"), "{rows:?}");
        assert!(rows[3].starts_with("Food (90)"), "{rows:?}");
        assert!(rows[4].contains("→ Households"), "{rows:?}");
    }

    /// The biggest flow fills the bar and the rest are drawn against it, so which path carries the
    /// volume is the first thing seen.
    #[test]
    fn a_bar_is_drawn_in_proportion_to_the_largest_flow() {
        let rows = drawn("sankey-beta\nA,B,100\nA,C,25");
        assert_eq!(rows[1].matches('█').count(), BAR);
        assert_eq!(rows[2].matches('█').count(), BAR / 4);
        assert!(rows[1].trim_end().ends_with("100"), "{rows:?}");
    }

    #[test]
    fn a_quoted_name_may_hold_a_comma() {
        let rows = drawn("sankey-beta\n\"Bread, baked\",Shops,10");
        assert!(rows[0].starts_with("Bread, baked (10)"), "{rows:?}");
    }

    #[test]
    fn what_is_not_a_sankey_is_left_alone() {
        assert!(render_sankey("graph TD\n  A --> B").is_none());
        assert!(render_sankey("sankey-beta").is_none(), "nothing flows");
        assert!(render_sankey("sankey-beta\nA,B").is_none(), "no value");
        assert!(render_sankey("sankey-beta\nA,B,nonsense").is_none());
        assert!(
            render_sankey("sankey-beta\nA,B,0").is_none(),
            "nothing is not a flow"
        );
        assert!(render_sankey("sankey-beta\nA,B,-5").is_none());
    }

    /// A diagram of hundreds of flows has no shape left to read.
    #[test]
    fn too_many_flows_are_refused() {
        let mut source = String::from("sankey-beta\n");
        for index in 0..MAX_FLOWS + 1 {
            source.push_str(&format!("Source,Target {index},1\n"));
        }
        assert!(render_sankey(&source).is_none());
    }
}
