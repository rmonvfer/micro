//! Images placed on the frame, after the rows they occupy are known.

use crate::capabilities::ImageProtocol;
use crate::images;
use ratatui::layout::Rect;

/// One image and the space it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picture {
    data: String,
    /// The number the terminal files this image under.
    id: u32,
    columns: usize,
    rows: usize,
    /// How tall the picture itself is, for working out which part of it a partly visible block
    /// should show.
    height_px: usize,
    /// Row in the transcript where the image begins.
    row: usize,
}

/// Where one image goes on the screen, for the frame about to be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The number the terminal files this image under.
    pub id: u32,
    pub column: u16,
    pub row: u16,
    pub columns: usize,
    pub rows: usize,
    /// The band of the picture to draw, when the transcript has room for only part of it.
    pub band: Option<Band>,
}

/// The slice of a picture that the region can show, in pixels down the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub top: usize,
    pub height: usize,
}

/// Images drawn on one frame.
#[derive(Debug, Clone, Default)]
pub struct Pictures {
    protocol: Option<ImageProtocol>,
    pictures: Vec<Picture>,
    /// The widest a picture may be drawn, in cells.
    max_columns: usize,
    /// Whether a picture wider than the room it has is shrunk to fit, or left at its own size and
    /// cut off by the region it is drawn in.
    resize: bool,
    /// The columns of ground the transcript's own text starts after, which a picture starts after
    /// too so that the two line up.
    indent: usize,
}

impl Pictures {
    pub fn new(protocol: Option<ImageProtocol>) -> Self {
        Pictures {
            protocol,
            pictures: Vec::new(),
            max_columns: DEFAULT_MAX_COLUMNS,
            resize: true,
            indent: 0,
        }
    }

    /// How wide a picture may be drawn, and whether one that does not fit is shrunk.
    pub fn sized(mut self, max_columns: usize, resize: bool) -> Self {
        self.max_columns = max_columns.max(1);
        self.resize = resize;
        self
    }

    /// Where the transcript's text begins, which is where a picture begins as well.
    pub fn indented(mut self, columns: usize) -> Self {
        self.indent = columns;
        self
    }

    /// Claim the rows an image needs, returning how many.
    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    /// Forget every image reserved after `kept`, whose rows are being drawn again.
    pub fn truncate(&mut self, kept: usize) {
        self.pictures.truncate(kept);
    }

    /// Reserve an image at `row` in the transcript, returning how many rows it occupies.
    ///
    /// The picture's own pixel size decides how much room it asks for, which is what a photograph
    /// or a screenshot wants: one image pixel to one screen pixel, shrunk if it will not fit.
    pub fn reserve(&mut self, data: &str, width: usize, row: usize) -> Option<usize> {
        let room = self.room(width);

        let (columns, rows) = match images::pixel_size(data) {
            Some((width_px, height_px)) => {
                images::cell_size(width_px, height_px, room, Some(MAX_ROWS))
            }
            None => (room.min(DEFAULT_MAX_COLUMNS), 10),
        };
        self.push(data, columns, rows, row)
    }

    /// Reserve an image that already knows how much room it wants, shrinking it if the transcript
    /// is too narrow for that. Typeset mathematics comes this way: it is rasterized far above the
    /// size it is drawn at, so its pixels say nothing about how large it should appear.
    pub fn reserve_sized(
        &mut self,
        data: &str,
        wanted: (usize, usize),
        width: usize,
        row: usize,
    ) -> Option<usize> {
        let room = self.room(width);
        let (columns, rows) = fitted(wanted.0, wanted.1, room, MAX_ROWS);
        self.push(data, columns, rows, row)
    }

    /// How wide a picture may be drawn in a transcript `width` cells across.
    fn room(&self, width: usize) -> usize {
        match self.resize {
            true => width.saturating_sub(self.indent).min(self.max_columns),

            false => self.max_columns,
        }
        .max(1)
    }

    /// Take an image at the size it will be drawn, returning the rows it occupies.
    fn push(&mut self, data: &str, columns: usize, rows: usize, row: usize) -> Option<usize> {
        self.protocol?;
        self.pictures.push(Picture {
            id: images::image_id(data),
            data: data.to_string(),
            columns,
            rows,
            height_px: images::pixel_size(data)
                .map(|(_, height)| height)
                .unwrap_or(0),
            row,
        });
        Some(rows)
    }

    /// How this terminal draws an image, if it draws one at all.
    pub fn protocol(&self) -> Option<ImageProtocol> {
        self.protocol
    }

    /// The picture filed under `id`, for the first time it is sent to the terminal.
    pub fn data(&self, id: u32) -> Option<&str> {
        self.pictures
            .iter()
            .find(|picture| picture.id == id)
            .map(|picture| picture.data.as_str())
    }

    /// Where every image the region can show goes on this frame.
    ///
    /// A picture half scrolled past the edge of the transcript is drawn as the half that fits, so
    /// nothing is ever painted over the input or the footer below it.
    pub fn placements(&self, area: Rect, first_visible: usize, top: u16) -> Vec<Placement> {
        let Some(protocol) = self.protocol else {
            return Vec::new();
        };
        let mut placements = Vec::new();
        for picture in &self.pictures {
            let start = top as i64 + picture.row as i64 - first_visible as i64;
            let end = start + picture.rows as i64;

            let visible_top = start.max(area.y as i64);
            let visible_bottom = end.min(area.bottom() as i64);
            if visible_bottom <= visible_top {
                continue;
            }

            let cut_above = (visible_top - start) as usize;
            let rows = (visible_bottom - visible_top) as usize;
            let whole = cut_above == 0 && rows == picture.rows;

            if !whole && !protocol.crops() {
                continue;
            }

            placements.push(Placement {
                id: picture.id,
                column: area.x + self.indent.min(area.width.saturating_sub(1) as usize) as u16,
                row: visible_top as u16,
                columns: picture.columns,
                rows,
                band: match whole {
                    true => None,
                    false => Some(picture.band(cut_above, rows)),
                },
            });
        }
        placements
    }
}

impl Picture {
    /// The slice of the picture covering `rows` of it, starting `cut_above` rows down.
    fn band(&self, cut_above: usize, rows: usize) -> Band {
        let per_row = self.height_px as f64 / self.rows.max(1) as f64;
        let top = (cut_above as f64 * per_row).round() as usize;
        let height = (rows as f64 * per_row).round().max(1.0) as usize;
        Band {
            top,
            height: height.min(self.height_px.saturating_sub(top)).max(1),
        }
    }
}

/// Shrink a size to the room it has, keeping its shape.
fn fitted(columns: usize, rows: usize, room: usize, max_rows: usize) -> (usize, usize) {
    let columns = columns.max(1);
    let rows = rows.max(1);
    let scale = (room as f64 / columns as f64)
        .min(max_rows as f64 / rows as f64)
        .min(1.0);
    (
        ((columns as f64 * scale).round() as usize).clamp(1, room),
        ((rows as f64 * scale).round() as usize).clamp(1, max_rows),
    )
}

/// No image is given more of the screen than this, however large it is.
const MAX_ROWS: usize = 20;

/// The widest a picture is drawn when nothing says otherwise.
const DEFAULT_MAX_COLUMNS: usize = 60;

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
        assert_eq!(pictures.reserve(&png(100, 100), 80, 0), None);
    }

    #[test]
    fn an_image_is_given_the_rows_its_shape_asks_for() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));

        let rows = pictures.reserve(&png(900, 450), 50, 0).expect("reserved");
        assert_eq!(rows, 13);
    }

    #[test]
    fn no_image_takes_more_than_its_share_of_the_screen() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let rows = pictures.reserve(&png(1000, 5000), 80, 0).expect("reserved");
        assert!(rows <= MAX_ROWS, "{rows} rows");
    }

    #[test]
    fn something_that_is_not_a_png_still_gets_a_block() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let rows = pictures
            .reserve("bm90IGFuIGltYWdl", 80, 0)
            .expect("reserved");
        assert_eq!(rows, 10);
    }

    #[test]
    fn an_image_is_placed_on_its_transcript_row() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(&png(90, 18), 80, 2);

        let area = Rect::new(0, 0, 20, 5);
        let placements = pictures.placements(area, 0, 0);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].row, 2);
        assert_eq!(placements[0].column, 0);
    }

    /// Every line of the transcript starts a column in from the edge, and a picture starts there
    /// too rather than hanging off the left of the words around it.
    #[test]
    fn a_picture_begins_where_the_words_do() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty)).indented(1);
        pictures.reserve(&png(90, 18), 80, 0);

        let placements = pictures.placements(Rect::new(4, 0, 20, 5), 0, 0);
        assert_eq!(placements[0].column, 5);
    }

    /// A block that has scrolled above the region is not drawn over whatever is there now.
    #[test]
    fn an_image_scrolled_out_of_the_region_is_skipped() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(&png(90, 18), 80, 0);

        let area = Rect::new(0, 0, 20, 5);
        assert!(pictures.placements(area, 4, 0).is_empty());
    }

    #[test]
    fn an_image_is_placed_after_bottom_aligned_transcript_rows() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve(&png(90, 18), 80, 3);

        let area = Rect::new(0, 0, 20, 5);
        let placements = pictures.placements(area, 3, 3);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].row, 3);
    }

    #[test]
    fn two_images_keep_their_own_rows() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let first = pictures.reserve(&png(90, 18), 80, 0).expect("reserved");
        pictures.reserve(&png(90, 20), 80, first);

        let area = Rect::new(0, 0, 20, 10);
        let placements = pictures.placements(area, 0, 0);

        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].row, 0);
        assert_eq!(placements[1].row, first as u16);
        assert_ne!(
            placements[0].id, placements[1].id,
            "two pictures, two numbers"
        );
    }

    /// The same picture keeps its number, so redrawing it never sends it again.
    #[test]
    fn a_picture_is_found_again_by_its_number() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let image = png(90, 18);
        pictures.reserve(&image, 80, 0);

        let area = Rect::new(0, 0, 20, 5);
        let placements = pictures.placements(area, 0, 0);
        assert_eq!(pictures.data(placements[0].id), Some(image.as_str()));
    }

    #[test]
    fn a_terminal_that_draws_no_images_places_none() {
        let pictures = Pictures::new(None);
        assert!(pictures.placements(Rect::new(0, 0, 20, 5), 0, 0).is_empty());
    }

    /// A picture is drawn over the cells it asks for, not the ones its pixels imply: typeset
    /// mathematics is rasterized far above the size it is meant to appear at.
    #[test]
    fn a_sized_picture_takes_the_room_it_asked_for() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));

        let rows = pictures
            .reserve_sized(&png(1600, 400), (24, 3), 80, 0)
            .expect("reserved");

        assert_eq!(rows, 3);
        let placements = pictures.placements(Rect::new(0, 0, 80, 20), 0, 0);
        assert_eq!(placements[0].columns, 24);
        assert_eq!(placements[0].rows, 3);
    }

    /// The transcript may be narrower than the formula would like, and then it shrinks.
    #[test]
    fn a_sized_picture_still_shrinks_to_a_narrow_transcript() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        let rows = pictures
            .reserve_sized(&png(1600, 400), (40, 4), 20, 0)
            .expect("reserved");

        assert!(rows < 4, "it kept its shape, taking {rows} rows");
        let placements = pictures.placements(Rect::new(0, 0, 20, 20), 0, 0);
        assert!(placements[0].columns <= 20);
    }

    /// Nothing is ever painted below the transcript, where the input and the footer are: a picture
    /// hanging over the edge is drawn as the part of it that fits.
    #[test]
    fn a_picture_hanging_below_the_region_is_cut_to_what_fits() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));

        pictures.reserve_sized(&png(180, 144), (20, 8), 80, 0);

        let area = Rect::new(0, 0, 80, 5);
        let placements = pictures.placements(area, 0, 0);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].row, 0);
        assert_eq!(placements[0].rows, 5, "only the rows the region has");
        assert_eq!(
            placements[0].band,
            Some(Band { top: 0, height: 90 }),
            "the top five eighths of the picture"
        );
    }

    /// Scrolling a picture up past the top of the transcript shows its lower part rather than
    /// making the whole thing vanish.
    #[test]
    fn a_picture_scrolled_past_the_top_shows_the_rest_of_itself() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve_sized(&png(180, 144), (20, 8), 80, 0);

        let area = Rect::new(0, 0, 80, 20);
        let placements = pictures.placements(area, 3, 0);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].row, 0);
        assert_eq!(placements[0].rows, 5);
        assert_eq!(
            placements[0].band,
            Some(Band {
                top: 54,
                height: 90
            })
        );
    }

    /// Once it has scrolled away entirely there is nothing left to draw.
    #[test]
    fn a_picture_scrolled_right_out_of_the_region_is_skipped() {
        let mut pictures = Pictures::new(Some(ImageProtocol::Kitty));
        pictures.reserve_sized(&png(180, 144), (20, 8), 80, 0);

        assert!(pictures
            .placements(Rect::new(0, 0, 80, 20), 8, 0)
            .is_empty());
    }

    /// iTerm2 cannot draw part of a picture, so one that does not fit waits until it does rather
    /// than spilling over the input.
    #[test]
    fn a_terminal_that_cannot_crop_waits_for_the_room() {
        let mut pictures = Pictures::new(Some(ImageProtocol::ITerm2));
        pictures.reserve_sized(&png(180, 144), (20, 8), 80, 0);

        assert!(pictures.placements(Rect::new(0, 0, 80, 5), 0, 0).is_empty());
        assert_eq!(pictures.placements(Rect::new(0, 0, 80, 20), 0, 0).len(), 1);
    }
}
