//! Packet diagrams: a bit-field table in the style of an RFC wire-format figure, wrapping at 32
//! bits per row.

use crate::canvas::{draw_text, Canvas, D, L, R, U};
use crate::labels::{ascii_lower, clean_label, fit_label, strip_controls};
use crate::parse::statements_of;
use crate::types::Cls;
use crate::width::string_width;


const ROW_BITS: u32 = 32;


const BIT_W: usize = 4;


const MAX_FIELDS: usize = 128;


const MAX_BITS: u32 = 512;

struct Field {
    start: u32,
    end: u32,
    label: String,
}

/// Draw `src` as a packet diagram, or answer nothing when it is not one.
pub(crate) fn render_packet(src: &str) -> Option<Canvas> {
    let src = strip_controls(src);
    let statements = statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "packet-beta" {
        return None;
    }

    let mut title = None;
    let mut declared: Vec<Field> = Vec::new();
    for st in &statements[1..] {
        if let Some(rest) = st.strip_prefix("title") {
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                return None;
            }
            title = Some(clean_label(rest.trim()));
            continue;
        }
        if declared.len() >= MAX_FIELDS {
            return None;
        }
        declared.push(read_field(st)?);
    }

    if declared.is_empty() {
        return None;
    }

    
    let mut fields: Vec<Field> = Vec::new();
    let mut cursor = 0u32;
    for field in declared {
        if field.start < cursor || field.end >= MAX_BITS {
            return None;
        }
        if field.start > cursor {
            fields.push(Field {
                start: cursor,
                end: field.start - 1,
                label: String::new(),
            });
        }
        cursor = field.end + 1;
        fields.push(field);
    }

    Some(draw(title.as_deref(), &fields, cursor))
}

/// `0-15: "Source Port"`, or a single bit `6: "Flag"`.
fn read_field(st: &str) -> Option<Field> {
    let (bits, label) = st.split_once(':')?;
    let bits = bits.trim();
    let (start, end) = match bits.split_once('-') {
        Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?),
        None => {
            let bit: u32 = bits.parse().ok()?;
            (bit, bit)
        }
    };
    if start > end {
        return None;
    }
    Some(Field {
        start,
        end,
        label: clean_label(label.trim()),
    })
}

fn draw(title: Option<&str>, fields: &[Field], total_bits: u32) -> Canvas {
    let row_count = total_bits.div_ceil(ROW_BITS).max(1) as usize;
    let top = usize::from(title.is_some());
    
    let row_height = 4;
    let height = top + row_count * row_height + row_count.saturating_sub(1);

    let widest_row_bits = (0..row_count)
        .map(|r| row_bit_count(r as u32, total_bits))
        .max()
        .unwrap_or(0);
    
    let width = (widest_row_bits as usize * BIT_W + 1).max(string_width(title.unwrap_or("")));

    let mut canvas = Canvas::new(width.max(1), height.max(1));
    if let Some(title) = title {
        draw_text(&mut canvas, title, 0, 0, Cls::Title);
    }

    for r in 0..row_count {
        let row = r as u32;
        let bit_count = row_bit_count(row, total_bits);
        let row_w = bit_count as usize * BIT_W;
        let y0 = top + r * (row_height + 1);
        draw_row(&mut canvas, y0, row, bit_count, row_w, fields);
    }

    canvas.finalize_mask();
    canvas
}


fn row_bit_count(row: u32, total_bits: u32) -> u32 {
    let row_start = row * ROW_BITS;
    total_bits.saturating_sub(row_start).min(ROW_BITS)
}

fn draw_row(
    canvas: &mut Canvas,
    y0: usize,
    row: u32,
    bit_count: u32,
    row_w: usize,
    fields: &[Field],
) {
    let y_header = y0;
    let y_top = y0 + 1;
    let y_label = y0 + 2;
    let y_bottom = y0 + 3;
    let row_start = row * ROW_BITS;
    let row_end = row_start + bit_count - 1;

    for i in 0..bit_count {
        let text = (row_start + i).to_string();
        let x = i as usize * BIT_W;
        let tx = x + BIT_W.saturating_sub(string_width(&text));
        draw_text(canvas, &text, tx, y_header, Cls::EdgeLabel);
    }

    seg_h_border(canvas, y_top, 0, row_w);
    seg_h_border(canvas, y_bottom, 0, row_w);
    seg_v_border(canvas, 0, y_top, y_bottom);
    seg_v_border(canvas, row_w, y_top, y_bottom);

    for field in fields {
        if field.end < row_start || field.start > row_end {
            continue;
        }
        let seg_start = field.start.max(row_start) - row_start;
        let seg_end = field.end.min(row_end) - row_start;
        let x0 = seg_start as usize * BIT_W;
        let x1 = (seg_end + 1) as usize * BIT_W;
        seg_v_border(canvas, x0, y_top, y_bottom);
        seg_v_border(canvas, x1, y_top, y_bottom);

        let inner = x1.saturating_sub(x0 + 1);
        if inner == 0 || field.label.is_empty() {
            continue;
        }
        let text = fit_label(&field.label, inner);
        let tx = x0 + 1 + inner.saturating_sub(string_width(&text)) / 2;
        draw_text(canvas, &text, tx, y_label, Cls::Text);
    }
}

/// `Canvas::seg_h` accumulates its bits as `Cls::Edge`.
fn seg_h_border(canvas: &mut Canvas, y: usize, x0: usize, x1: usize) {
    let (a, b) = (x0.min(x1), x0.max(x1));
    for x in a..=b {
        let mut bits = 0;
        if x > a {
            bits |= L;
        }
        if x < b {
            bits |= R;
        }
        canvas.add_bits(x, y, bits, Cls::Border);
    }
}

fn seg_v_border(canvas: &mut Canvas, x: usize, y0: usize, y1: usize) {
    let (a, b) = (y0.min(y1), y0.max(y1));
    for y in a..=b {
        let mut bits = 0;
        if y > a {
            bits |= U;
        }
        if y < b {
            bits |= D;
        }
        canvas.add_bits(x, y, bits, Cls::Border);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_packet(src)
            .expect("it is a packet diagram")
            .to_lines()
            .plain
    }

    /// A field spans exactly the columns its bits occupy, with the bit numbers for that span along
    /// the top.
    #[test]
    fn a_field_spans_its_bit_range() {
        let rows = drawn(
            "packet-beta\ntitle IP header\n0-15: \"Source Port\"\n16-31: \"Destination Port\"",
        );
        assert_eq!(rows[0], "IP header");
        assert!(rows.iter().any(|r| r.contains("Source Port")), "{rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("Destination Port")),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.trim_start().starts_with('0')),
            "{rows:?}"
        );
        assert!(rows.iter().any(|r| r.contains("31")), "{rows:?}");
        
        let top_border = rows
            .iter()
            .find(|r| r.starts_with('┌'))
            .expect("a top border");
        assert!(top_border.ends_with('┐'), "{top_border:?}");
        let bottom_border = rows
            .iter()
            .find(|r| r.starts_with('└'))
            .expect("a bottom border");
        assert!(bottom_border.ends_with('┘'), "{bottom_border:?}");
    }

    
    #[test]
    fn a_gap_between_fields_gets_an_unlabelled_cell() {
        let rows = drawn("packet-beta\n0-3: \"Version\"\n8-15: \"Length\"");
        let border_row = rows
            .iter()
            .find(|r| r.starts_with('┌'))
            .expect("a border row");
        
        assert_eq!(border_row.matches('┬').count(), 2, "{border_row:?}");
    }

    /// A field wider than 32 bits is split across the rows it actually falls in, each keeping its
    /// own copy of the label.
    #[test]
    fn a_field_crossing_a_row_boundary_is_split_across_rows() {
        let rows = drawn("packet-beta\n0-39: \"Checksum\"");
        let label_rows: Vec<&String> = rows.iter().filter(|r| r.contains("Checksum")).collect();
        assert_eq!(label_rows.len(), 2, "{rows:?}");
    }

    /// A single bit is a field of width one, same as any other.
    #[test]
    fn a_single_bit_is_a_field_of_width_one() {
        
        let rows = drawn("packet-beta\n0: \"Bit\"");
        assert!(rows.iter().any(|r| r.contains("Bit")), "{rows:?}");
    }

    
    #[test]
    fn what_is_not_a_packet_diagram_is_left_alone() {
        assert!(render_packet("graph TD\n A --> B").is_none());
        assert!(render_packet("packet-beta").is_none(), "no fields at all");
        assert!(render_packet("packet-beta\n0-15 no colon").is_none());
        assert!(
            render_packet("packet-beta\n0-15: \"A\"\n10-20: \"B\"").is_none(),
            "overlapping fields"
        );
    }

    /// A header this wide is a dump, not a diagram, so it is refused.
    #[test]
    fn too_many_bits_are_refused() {
        let src = format!("packet-beta\n0-{MAX_BITS}: \"Huge\"");
        assert!(render_packet(&src).is_none());
    }
}
