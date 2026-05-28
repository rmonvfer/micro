//! Images placed on the frame, after the rows they occupy are known.

use crate::capabilities::ImageProtocol;
use crate::images;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One image and the space it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picture {
    data: String,
    columns: usize,
    rows: usize,
    /// Which reserved row block this is, counted from the top of the transcript.
    order: usize,
}

/// Images drawn on one frame.
#[derive(Debug, Clone, Default)]
pub struct Pictures {
    protocol: Option<ImageProtocol>,
    pictures: Vec<Picture>,
    /// Rows reserved so far, which is how a picture finds its own block again.
    reserved: usize,
    /// The widest a picture may be drawn, in cells.
    max_columns: usize,
    /// Whether a picture wider than the room it has is shrunk to fit, or left at its own size and
    /// cut off by the region it is drawn in.
    resize: bool,
}

impl Pictures {
    pub fn new(protocol: Option<ImageProtocol>) -> Self {
        Pictures {
            protocol,
            pictures: Vec::new(),
            reserved: 0,
            max_columns: DEFAULT_MAX_COLUMNS,
            resize: true,
        }
    }

    /// How wide a picture may be drawn, and whether one that does not fit is shrunk.
    pub fn sized(mut self, max_columns: usize, resize: bool) -> Self {
        self.max_columns = max_columns.max(1);
        self.resize = resize;
        self
    }

    /// Claim the rows an image needs, returning how many.
    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    /// Forget every image reserved after `kept`, whose rows are being drawn again.
    pub fn truncate(&mut self, kept: usize) {
        
        if let Some(first) = self.pictures.get(kept) {
            self.reserved = first.order;
        }
        self.pictures.truncate(kept);
    }

    pub fn reserve(&mut self, data: &str, width: usize) -> Option<usize> {
        let protocol = self.protocol?;
        let _ = protocol;

        
        let room = match self.resize {
            true => width.min(self.max_columns),
            
            false => self.max_columns,
        }
        .max(1);

        
        let (columns, rows) = match decoded_size(data) {
            Some((width_px, height_px)) => {
                images::cell_size(width_px, height_px, room, Some(MAX_ROWS))
            }
            None => (room.min(DEFAULT_MAX_COLUMNS), 10),
        };

        self.pictures.push(Picture {
            data: data.to_string(),
            columns,
            rows,
            order: self.reserved,
        });
        self.reserved += rows;
        Some(rows)
    }

    /// Draw every image into the rows it was given.
    pub fn apply(&self, buffer: &mut Buffer, area: Rect, first_visible: usize) {
        let Some(protocol) = self.protocol else {
            return;
        };
        for picture in &self.pictures {
            let Some(offset) = picture.order.checked_sub(first_visible) else {
                continue;
            };
            let y = area.y + offset as u16;
            if offset >= area.height as usize || y >= area.bottom() {
                continue;
            }
            let escape = images::encode(protocol, &picture.data, picture.columns, picture.rows);
            let cell = &mut buffer[(area.x, y)];
            let symbol = format!("{escape}{}", cell.symbol());
            cell.set_symbol(&symbol);
        }
    }
}

/// No image is given more of the screen than this, however large it is.
const MAX_ROWS: usize = 20;

/// The widest a picture is drawn when nothing says otherwise.
const DEFAULT_MAX_COLUMNS: usize = 60;

/// The pixel size of a base64 image, when it is a PNG and the header can be read.
fn decoded_size(data: &str) -> Option<(usize, usize)> {
    let head: String = data.chars().take(64).collect();
    let bytes = decode_base64(&head)?;
    images::png_size(&bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A base64 PNG header describing an image of the given size.
    fn png(width: u32, height: u32) -> String {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend([0, 0, 0, 13]);
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        encode_base64(&bytes)
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let block = match chunk.len() {
                1 => (chunk[0] as u32) << 16,
                2 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8),
                _ => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32,
            };
            out.push(ALPHABET[(block >> 18 & 63) as usize] as char);
            out.push(ALPHABET[(block >> 12 & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(block >> 6 & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(block & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn a_terminal_that_draws_no_images_reserves_nothing() {
        let mut pictures = Pictures::new(None);
        assert_eq!(pictures.reserve(&png(100, 100), 80), None);
    }

    #[test]
    fn an_image_is_given_the_rows_its_shape_asks_for() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        
        let rows = pictures.reserve(&png(900, 450), 50).expect("reserved");
        assert_eq!(rows, 13);
    }

    #[test]
    fn no_image_takes_more_than_its_share_of_the_screen() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let rows = pictures.reserve(&png(1000, 5000), 80).expect("reserved");
        assert!(rows <= MAX_ROWS, "{rows} rows");
    }

    #[test]
    fn something_that_is_not_a_png_still_gets_a_block() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let rows = pictures.reserve("bm90IGFuIGltYWdl", 80).expect("reserved");
        assert_eq!(rows, 10);
    }

    #[test]
    fn the_escape_lands_on_the_first_row_it_reserved() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(&png(90, 18), 80);

        let area = Rect::new(0, 0, 20, 5);
        let mut buffer = Buffer::empty(area);
        pictures.apply(&mut buffer, area, 0);

        assert!(buffer[(0, 0)].symbol().starts_with("\x1b_G"));
    }

    /// A block that has scrolled above the region is not drawn into whatever is there now.
    #[test]
    fn an_image_scrolled_out_of_the_region_is_skipped() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(&png(90, 18), 80);

        let area = Rect::new(0, 0, 20, 5);
        let mut buffer = Buffer::empty(area);
        pictures.apply(&mut buffer, area, 4);

        assert_eq!(buffer[(0, 0)].symbol(), " ");
    }

    #[test]
    fn two_images_keep_their_own_rows() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let first = pictures.reserve(&png(90, 18), 80).expect("reserved");
        pictures.reserve(&png(90, 18), 80);

        let area = Rect::new(0, 0, 20, 10);
        let mut buffer = Buffer::empty(area);
        pictures.apply(&mut buffer, area, 0);

        assert!(buffer[(0, 0)].symbol().starts_with("\x1b_G"));
        assert!(buffer[(0, first as u16)].symbol().starts_with("\x1b_G"));
    }

    #[test]
    fn base64_decodes_far_enough_to_read_a_header() {
        assert_eq!(decode_base64("Zm9vYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(decode_base64(""), None);
    }
}
