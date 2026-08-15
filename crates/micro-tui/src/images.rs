//! Drawing an image in a terminal that can do it.
//!
//! Two protocols cover the terminals that manage this at all: kitty's, which Ghostty,
//! WezTerm and Warp also speak, and iTerm2's own. Both take the image base64 encoded and
//! placed at the cursor, and both are escape sequences rather than characters — so they go
//! onto a cell after layout, the same way a hyperlink does, and the rows they occupy are
//! reserved beforehand so nothing is drawn underneath them.

use crate::capabilities::ImageProtocol;

/// Kitty accepts at most this much base64 per escape, so a larger image is split.
const CHUNK: usize = 4096;

/// A terminal cell is about this many pixels, which is what turns an image's size into a
/// number of rows. Terminals that report their real cell size are rare enough that a fixed
/// figure is what gets used.
pub const CELL_WIDTH_PX: usize = 9;
pub const CELL_HEIGHT_PX: usize = 18;

/// The escape sequence that draws `data` at the cursor, sized to `columns` by `rows`.
pub fn encode(protocol: ImageProtocol, data: &str, columns: usize, rows: usize) -> String {
    match protocol {
        ImageProtocol::Kitty => encode_kitty(data, columns, rows),
        ImageProtocol::ITerm2 => encode_iterm2(data, columns, rows),
    }
}

/// Kitty's graphics protocol.
///
/// `a=T` places the image immediately, `f=100` says the payload is a PNG rather than raw
/// pixels, and `q=2` asks the terminal not to reply — a reply would land in the input stream
/// and be read as keystrokes.
fn encode_kitty(data: &str, columns: usize, rows: usize) -> String {
    let params = format!("a=T,f=100,q=2,c={columns},r={rows}");
    if data.len() <= CHUNK {
        return format!("\x1b_G{params};{data}\x1b\\");
    }

    // Split across escapes, `m=1` on every one that has more to come and `m=0` on the last.
    let mut out = String::new();
    let bytes = data.as_bytes();
    let mut offset = 0;
    let mut first = true;
    while offset < bytes.len() {
        let end = (offset + CHUNK).min(bytes.len());
        let chunk = &data[offset..end];
        let last = end == bytes.len();
        match (first, last) {
            (true, _) => out.push_str(&format!("\x1b_G{params},m=1;{chunk}\x1b\\")),
            (false, true) => out.push_str(&format!("\x1b_Gm=0;{chunk}\x1b\\")),
            (false, false) => out.push_str(&format!("\x1b_Gm=1;{chunk}\x1b\\")),
        }
        first = false;
        offset = end;
    }
    out
}

/// iTerm2's inline image escape, sized in cells.
fn encode_iterm2(data: &str, columns: usize, rows: usize) -> String {
    format!("\x1b]1337;File=inline=1;width={columns};height={rows}:{data}\x07")
}

/// Free every image the terminal is holding for us.
///
/// Kitty keeps an image until it is told otherwise, so a conversation that scrolls away
/// would otherwise leave them all in memory.
pub fn forget_all(protocol: ImageProtocol) -> Option<&'static str> {
    match protocol {
        ImageProtocol::Kitty => Some("\x1b_Ga=d,d=A,q=2\x1b\\"),
        ImageProtocol::ITerm2 => None,
    }
}

/// How many cells an image should occupy, keeping its shape.
///
/// Bounded by the columns available and, when given, by a number of rows — so an image never
/// takes the whole screen and never stretches.
pub fn cell_size(
    width_px: usize,
    height_px: usize,
    max_columns: usize,
    max_rows: Option<usize>,
) -> (usize, usize) {
    let width_px = width_px.max(1) as f64;
    let height_px = height_px.max(1) as f64;
    let max_columns = max_columns.max(1);

    let by_width = (max_columns * CELL_WIDTH_PX) as f64 / width_px;
    let scale = match max_rows {
        Some(rows) => by_width.min((rows.max(1) * CELL_HEIGHT_PX) as f64 / height_px),
        None => by_width,
    };
    // Never enlarge: an image smaller than the space keeps its own size.
    let scale = scale.min(1.0);

    let columns = ((width_px * scale) / CELL_WIDTH_PX as f64).round().max(1.0) as usize;
    let rows = ((height_px * scale) / CELL_HEIGHT_PX as f64)
        .round()
        .max(1.0) as usize;
    (columns.min(max_columns), rows.max(1))
}

/// The pixel size of a PNG, read from its header.
///
/// Only PNG is measured: it is what every clipboard on every platform hands over, and its
/// dimensions sit at a fixed offset. Anything else is drawn at a default size rather than
/// decoded.
pub fn png_size(data: &[u8]) -> Option<(usize, usize)> {
    const SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 24 || !data.starts_with(SIGNATURE) {
        return None;
    }
    // IHDR is the first chunk, and its width and height are big-endian at bytes 16 and 20.
    let width = u32::from_be_bytes(data[16..20].try_into().ok()?) as usize;
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?) as usize;
    match width > 0 && height > 0 {
        true => Some((width, height)),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_image_is_one_kitty_escape() {
        let encoded = encode_kitty("Zm9v", 10, 5);
        assert!(encoded.starts_with("\x1b_Ga=T,f=100,q=2,c=10,r=5;"));
        assert!(encoded.ends_with("\x1b\\"));
        assert_eq!(encoded.matches("\x1b_G").count(), 1);
    }

    /// Kitty takes at most 4096 bytes per escape, and every chunk but the last says more is
    /// coming.
    #[test]
    fn a_large_image_is_split_and_marked() {
        let data = "a".repeat(CHUNK * 2 + 10);
        let encoded = encode_kitty(&data, 10, 5);

        assert_eq!(encoded.matches("\x1b_G").count(), 3);
        assert_eq!(
            encoded.matches("m=1;").count(),
            2,
            "two chunks have more to come"
        );
        assert_eq!(encoded.matches("m=0;").count(), 1, "and the last says so");
    }

    #[test]
    fn every_byte_survives_being_split() {
        let data = "b".repeat(CHUNK * 2 + 7);
        let encoded = encode_kitty(&data, 1, 1);
        let payload: String = encoded
            .split("\x1b\\")
            .filter_map(|part| part.split_once(';').map(|(_, rest)| rest.to_string()))
            .collect();
        assert_eq!(payload, data);
    }

    #[test]
    fn iterm_takes_its_size_in_cells() {
        let encoded = encode_iterm2("Zm9v", 8, 4);
        assert_eq!(encoded, "\x1b]1337;File=inline=1;width=8;height=4:Zm9v\x07");
    }

    #[test]
    fn an_image_keeps_its_shape_when_it_is_scaled_down() {
        // Twice as wide as it is tall, in a space too narrow for it.
        let (columns, rows) = cell_size(900, 450, 50, None);
        assert_eq!(columns, 50);
        assert_eq!(
            rows, 13,
            "half the width in pixels, and cells are twice as tall"
        );
    }

    #[test]
    fn a_small_image_is_never_enlarged() {
        let (columns, rows) = cell_size(18, 18, 100, None);
        assert_eq!((columns, rows), (2, 1));
    }

    #[test]
    fn a_row_limit_is_honoured_too() {
        let (_, rows) = cell_size(900, 900, 80, Some(10));
        assert!(rows <= 10);
    }

    #[test]
    fn a_png_reports_its_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend(320u32.to_be_bytes());
        png.extend(240u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((320, 240)));
    }

    #[test]
    fn something_that_is_not_a_png_reports_nothing() {
        assert_eq!(png_size(b"not an image at all...."), None);
        assert_eq!(png_size(&[]), None);
    }
}
