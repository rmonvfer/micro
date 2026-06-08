//! Drawing an image in a terminal that can do it.

use crate::capabilities::ImageProtocol;
use crate::render::pictures::Band;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// Kitty accepts at most this much base64 per escape, so a larger image is split.
const CHUNK: usize = 4096;

/// How large one cell of the terminal's grid is, in pixels.
///
/// A picture is scaled to fill the cells it is given, so this is what ties the shape an image has on
/// the screen to the shape it has in pixels: get it wrong and everything drawn is stretched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub width: usize,
    pub height: usize,
    /// Whether the terminal reported this, as against it being the shape assumed of one that will
    /// not say.
    pub measured: bool,
}

/// The cell of a terminal that reports none: about the shape a monospaced font has.
const ASSUMED: Cell = Cell {
    width: 9,
    height: 18,
    measured: false,
};

/// What the terminal last said its cell was, or zero while it has said nothing.
static CELL_WIDTH_PX: AtomicUsize = AtomicUsize::new(0);
static CELL_HEIGHT_PX: AtomicUsize = AtomicUsize::new(0);

/// How large a cell is, as the terminal last reported it.
pub fn cell() -> Cell {
    let width = CELL_WIDTH_PX.load(Ordering::Relaxed);
    let height = CELL_HEIGHT_PX.load(Ordering::Relaxed);
    match width > 0 && height > 0 {
        true => Cell {
            width,
            height,
            measured: true,
        },
        false => ASSUMED,
    }
}

/// Take the terminal's word for the size of a cell, worked out from the size of its window in
/// pixels and in cells. A terminal that reports neither leaves the assumed shape standing.
pub fn note_cell_size(pixels: (u16, u16), cells: (u16, u16)) {
    let Some(cell) = cell_from(pixels, cells) else {
        return;
    };
    CELL_WIDTH_PX.store(cell.width, Ordering::Relaxed);
    CELL_HEIGHT_PX.store(cell.height, Ordering::Relaxed);
}

/// The cell a window of that many pixels and that many cells is made of, when the two agree on
/// something a cell of text could be: one is never wider than it is tall, and terminals that report
/// no pixels at all report a shape that is not one.
fn cell_from(pixels: (u16, u16), cells: (u16, u16)) -> Option<Cell> {
    let width = (pixels.0 as usize).checked_div(cells.0 as usize)?;
    let height = (pixels.1 as usize).checked_div(cells.1 as usize)?;
    let believable = (4..=64).contains(&width) && (4..=128).contains(&height) && height >= width;
    match believable {
        true => Some(Cell {
            width,
            height,
            measured: true,
        }),
        false => None,
    }
}

/// The number the terminal files an image under, taken from the image itself so the same picture
/// keeps its number for as long as the conversation holds it.
pub fn image_id(data: &str) -> u32 {
    const OFFSET: u32 = 2_166_136_261;
    const PRIME: u32 = 16_777_619;
    let mut hash = OFFSET;
    for byte in data.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(PRIME);
    }

    hash.max(1)
}

/// Hand an image to the terminal to hold, without drawing anything. It is drawn afterwards by
/// number, so the picture itself crosses the wire once however often it is redrawn.
pub fn transmit(protocol: ImageProtocol, data: &str, id: u32) -> String {
    match protocol {
        ImageProtocol::Kitty => transmit_kitty(data, id),

        ImageProtocol::ITerm2 => String::new(),
    }
}

/// The escape that draws an image at the cursor over `columns` by `rows` cells, leaving the cursor
/// where it found it. A `band` draws that slice of the picture rather than the whole of it.
pub fn place(
    protocol: ImageProtocol,
    data: &str,
    id: u32,
    columns: usize,
    rows: usize,
    band: Option<Band>,
) -> String {
    match protocol {
        ImageProtocol::Kitty => {
            let slice = match band {
                Some(band) => format!(",y={},h={}", band.top, band.height),
                None => String::new(),
            };
            format!("\x1b_Ga=p,i={id},q=2,c={columns},r={rows}{slice},C=1\x1b\\")
        }
        ImageProtocol::ITerm2 => {
            format!("\x1b]1337;File=inline=1;width={columns};height={rows}:{data}\x07")
        }
    }
}

/// Take one image off the screen wherever it was drawn, leaving what the terminal holds in place so
/// it can be drawn again without being sent a second time.
///
/// Only the images this conversation drew are named, so anything the terminal was showing before
/// the session started is left alone.
pub fn remove(protocol: ImageProtocol, id: u32) -> String {
    match protocol {
        ImageProtocol::Kitty => format!("\x1b_Ga=d,d=i,i={id},q=2\x1b\\"),

        ImageProtocol::ITerm2 => String::new(),
    }
}

/// Send an image for the terminal to hold under `id`, split into the chunks kitty accepts.
fn transmit_kitty(data: &str, id: u32) -> String {
    let params = format!("a=t,f=100,i={id},q=2");
    if data.len() <= CHUNK {
        return format!("\x1b_G{params};{data}\x1b\\");
    }

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

/// Free every image the terminal is holding for us.
pub fn forget_all(protocol: ImageProtocol) -> Option<&'static str> {
    match protocol {
        ImageProtocol::Kitty => Some("\x1b_Ga=d,d=A,q=2\x1b\\"),
        ImageProtocol::ITerm2 => None,
    }
}

/// How many cells an image should occupy, keeping its shape.
pub fn cell_size(
    width_px: usize,
    height_px: usize,
    max_columns: usize,
    max_rows: Option<usize>,
) -> (usize, usize) {
    let cell = cell();
    let width_px = width_px.max(1) as f64;
    let height_px = height_px.max(1) as f64;
    let max_columns = max_columns.max(1);

    let by_width = (max_columns * cell.width) as f64 / width_px;
    let scale = match max_rows {
        Some(rows) => by_width.min((rows.max(1) * cell.height) as f64 / height_px),
        None => by_width,
    };

    let scale = scale.min(1.0);

    let columns = ((width_px * scale) / cell.width as f64).round().max(1.0) as usize;
    let rows = ((height_px * scale) / cell.height as f64).round().max(1.0) as usize;
    (columns.min(max_columns), rows.max(1))
}

/// The pixel size of a base64 image, when it is a PNG and the header can be read.
pub fn pixel_size(data: &str) -> Option<(usize, usize)> {
    let head: String = data.chars().take(64).collect();
    png_size(&decode_base64(&head)?)
}

/// Decode base64, stopping at anything that is not part of the alphabet.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for character in text.chars() {
        let Some(value) = sextet(character) else {
            break;
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    match out.is_empty() {
        true => None,
        false => Some(out),
    }
}

fn sextet(character: char) -> Option<u8> {
    match character {
        'A'..='Z' => Some(character as u8 - b'A'),
        'a'..='z' => Some(character as u8 - b'a' + 26),
        '0'..='9' => Some(character as u8 - b'0' + 52),
        '+' => Some(62),
        '/' => Some(63),
        _ => None,
    }
}

/// The pixel size of a PNG, read from its header.
pub fn png_size(data: &[u8]) -> Option<(usize, usize)> {
    const SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 24 || !data.starts_with(SIGNATURE) {
        return None;
    }

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
        let sent = transmit_kitty("Zm9v", 7);
        assert!(sent.starts_with("\x1b_Ga=t,f=100,i=7,q=2;"));
        assert!(sent.ends_with("\x1b\\"));
        assert_eq!(sent.matches("\x1b_G").count(), 1);
    }

    /// Kitty takes at most 4096 bytes per escape, and every chunk but the last says more is coming.
    #[test]
    fn a_large_image_is_split_and_marked() {
        let data = "a".repeat(CHUNK * 2 + 10);
        let sent = transmit_kitty(&data, 7);

        assert_eq!(sent.matches("\x1b_G").count(), 3);
        assert_eq!(
            sent.matches("m=1;").count(),
            2,
            "two chunks have more to come"
        );
        assert_eq!(sent.matches("m=0;").count(), 1, "and the last says so");
    }

    #[test]
    fn every_byte_survives_being_split() {
        let data = "b".repeat(CHUNK * 2 + 7);
        let sent = transmit_kitty(&data, 1);
        let payload: String = sent
            .split("\x1b\\")
            .filter_map(|part| part.split_once(';').map(|(_, rest)| rest.to_string()))
            .collect();
        assert_eq!(payload, data);
    }

    /// The cursor is where the interface is drawn from, so an image must not carry it away.
    #[test]
    fn a_kitty_placement_leaves_the_cursor_alone_and_carries_no_payload() {
        let placed = place(ImageProtocol::Kitty, "Zm9v", 7, 8, 4, None);
        assert_eq!(placed, "\x1b_Ga=p,i=7,q=2,c=8,r=4,C=1\x1b\\");
        assert!(!placed.contains("Zm9v"), "the terminal already holds it");
    }

    /// Drawing the next frame starts by taking the last frame's images off the screen, keeping
    /// what the terminal holds so nothing has to be sent twice.
    #[test]
    fn an_image_is_taken_off_the_screen_without_being_dropped() {
        let removed = remove(ImageProtocol::Kitty, 7);
        assert_eq!(removed, "\x1b_Ga=d,d=i,i=7,q=2\x1b\\");
        assert!(
            !removed.contains("d=I"),
            "the picture stays where the terminal put it"
        );
    }

    /// Whatever the terminal was showing before the session started is not ours to take away.
    #[test]
    fn only_the_images_this_session_drew_are_named() {
        assert!(remove(ImageProtocol::Kitty, 7).contains("i=7"));
        assert!(!remove(ImageProtocol::Kitty, 7).contains("d=a"));
    }

    #[test]
    fn iterm_takes_its_size_in_cells() {
        let placed = place(ImageProtocol::ITerm2, "Zm9v", 7, 8, 4, None);
        assert_eq!(placed, "\x1b]1337;File=inline=1;width=8;height=4:Zm9v\x07");
    }

    /// iTerm2 has nowhere to keep an image, so it travels with every placement instead.
    #[test]
    fn iterm_holds_nothing_between_frames() {
        assert_eq!(transmit(ImageProtocol::ITerm2, "Zm9v", 7), "");
        assert_eq!(remove(ImageProtocol::ITerm2, 7), "");
    }

    #[test]
    fn an_image_keeps_its_number_and_never_takes_zero() {
        assert_eq!(image_id("Zm9v"), image_id("Zm9v"));
        assert_ne!(image_id("Zm9v"), image_id("YmFy"));
        assert_ne!(image_id(""), 0, "zero means no image at all");
    }

    /// A terminal that reports the size of its window in both pixels and cells has said how large
    /// one cell is, which is what everything drawn is measured against.
    #[test]
    fn a_window_reported_in_pixels_and_cells_gives_the_cell() {
        assert_eq!(
            cell_from((1440, 900), (160, 50)),
            Some(Cell {
                width: 9,
                height: 18,
                measured: true
            })
        );
    }

    /// A terminal that reports no pixels, or a shape no cell of text has, leaves the assumed shape
    /// standing rather than stretching everything drawn.
    #[test]
    fn a_report_that_is_not_a_cell_of_text_is_not_believed() {
        assert_eq!(cell_from((0, 0), (160, 50)), None);
        assert_eq!(cell_from((1440, 900), (0, 0)), None);
        assert_eq!(
            cell_from((1440, 400), (160, 50)),
            None,
            "wider than it is tall"
        );
        assert_eq!(cell_from((320, 900), (160, 50)), None, "two pixels across");
    }

    #[test]
    fn an_image_keeps_its_shape_when_it_is_scaled_down() {
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
