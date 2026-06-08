//! Textbook-quality display mathematics, rasterized by Typst for graphical terminals.

use crate::images::Cell;
use ratatui::style::Color;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;
use typst::foundations::Dict;
use typst::foundations::IntoValue;
use typst::layout::PagedDocument;
use typst_as_lib::TypstEngine;
use typst_as_lib::TypstTemplateCollection;

/// The size mathematics is set at, which is what ties it to the size of the words around it.
const TEXT_SIZE_PT: f64 = 12.0;

/// A terminal column is one character of text at that size. Measuring a formula against that is
/// what lands it at the size of the prose it sits in, whatever resolution it happened to be
/// rasterized at.
const COLUMN_POINTS: f64 = TEXT_SIZE_PT * 0.6;

/// Roughly the pixels a terminal cell is across on a display with doubled pixels, for the terminals
/// that will not say what theirs is. A formula is rasterized against that rather than against the
/// cell grid, so it stays sharp on the screens that have the pixels for it without carrying any that
/// nothing can show.
const COLUMN_PIXELS: f64 = 28.0;

/// The expression and the ink are handed to the templates as inputs, so one engine typesets every
/// formula a conversation asks for. The page itself is left unpainted, so the terminal's own
/// background shows through and a formula sits in the transcript rather than on a card.
///
/// The formula on a page that grows around it, which is how much room it asks for.
const MEASURE_ID: &str = "/measure.typ";
const MEASURE: &str = r#"#import sys: inputs
#set page(width: auto, height: auto, margin: 0pt, fill: none)
#set text(size: eval(inputs.size), fill: eval(inputs.ink))
#math.equation(block: true, eval(inputs.source, mode: "math"))
"#;

/// The formula on a page of exactly the cells it will be drawn over, which is the picture the
/// terminal is handed. It sits at the left of that page, where the prose around it starts, with what
/// the rounding up to whole cells left over falling evenly above and below it.
const PAGE_ID: &str = "/math.typ";
const PAGE: &str = r#"#import sys: inputs
#set page(width: eval(inputs.width), height: eval(inputs.height), margin: 0pt, fill: none)
#set text(size: eval(inputs.size), fill: eval(inputs.ink))
#align(left + horizon, math.equation(block: true, eval(inputs.source, mode: "math")))
"#;

/// How tall one row of the terminal is in the points a formula is set in.
///
/// A row is as tall as the terminal's cell is, measured against the width of a column, because a
/// picture is scaled to fill the cells it is given: the box a formula asks for has to be the shape
/// the formula is, or the terminal stretches it into the shape of the box.
fn row_points(cell: Cell) -> f64 {
    COLUMN_POINTS * cell.height as f64 / cell.width as f64
}

/// A formula is rasterized at this much of the resolution it is drawn at. Terminals disagree about
/// whether the size they report is in pixels or in the points a doubled display has two pixels to,
/// so a formula is set at twice either and left to be scaled down, which costs a few kilobytes and
/// keeps it sharp on both.
const SUPERSAMPLE: f64 = 2.0;

/// How finely a formula is rasterized, from the size of the cells it will be drawn over. A terminal
/// that reports no size gets a picture set against a doubled display, which is the one that would
/// otherwise be short of pixels.
fn pixels_per_point(cell: Cell) -> f32 {
    let pixels = match cell.measured {
        true => cell.width as f64 * SUPERSAMPLE,
        false => COLUMN_PIXELS,
    };
    (pixels / COLUMN_POINTS) as f32
}

/// Building an engine searches the system for fonts, which costs far more than typesetting does,
/// so one engine is built and then kept.
fn engine() -> &'static TypstEngine<TypstTemplateCollection> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateCollection>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .with_static_source_file_resolver([(MEASURE_ID, MEASURE), (PAGE_ID, PAGE)])
            .search_fonts_with(Default::default())
            .build()
    })
}

/// A formula, the ink it was set in, and the cell it was set for, which together decide the picture.
type Formula = (String, Color, usize, usize);

/// A typeset formula, and the room it wants on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Math {
    /// The rasterized formula, base64 encoded.
    pub data: String,
    /// The cells it is meant to be drawn over, before the width of the transcript is taken into
    /// account. This comes from the size Typst set it at rather than from the pixels it was
    /// rasterized into, so raising the resolution sharpens a formula without enlarging it.
    pub columns: usize,
    pub rows: usize,
}

/// What each formula came out as, so the transcript can be laid out again as often as a running
/// answer needs without typesetting anything twice.
fn rendered() -> &'static Mutex<HashMap<Formula, Option<Math>>> {
    static RENDERED: OnceLock<Mutex<HashMap<Formula, Option<Math>>>> = OnceLock::new();
    RENDERED.get_or_init(Default::default)
}

/// The text color used to rasterize mathematics.
fn text_fill(foreground: Color) -> String {
    match foreground {
        Color::Rgb(red, green, blue) => {
            format!("rgb(\"#{red:02x}{green:02x}{blue:02x}\")")
        }
        _ => "white".to_string(),
    }
}

/// Render one Typst display-math expression as a PNG.
///
/// A formula is typeset the first time it is seen and remembered after that: the transcript is laid
/// out again on every frame of a running answer, and typesetting is far too slow to sit on that
/// path more than once.
pub fn render_math(source: &str, ink: Color) -> Option<Math> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }

    let cell = crate::images::cell();
    let key = (source.to_string(), ink, cell.width, cell.height);
    if let Some(known) = rendered().lock().ok()?.get(&key) {
        return known.clone();
    }

    let math = typeset(source, ink, cell);
    if let Ok(mut cache) = rendered().lock() {
        cache.insert(key, math.clone());
    }
    math
}

/// How many formulas have actually been typeset, as opposed to answered from the cache.
static TYPESET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Typeset one expression, which is what costs the time the cache above is there to save.
///
/// It is set twice: once to see how much room it wants, and again on a page of exactly the cells it
/// will be drawn over. The second page is what keeps a formula in proportion, since whole cells are
/// all a terminal can be given and it scales a picture to fill them.
fn typeset(source: &str, ink: Color, cell: Cell) -> Option<Math> {
    TYPESET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let measured = compile(MEASURE_ID, source, ink, None)?;
    let size = measured.pages.first()?.frame.size();

    let row_points = row_points(cell);
    let columns = (size.x.to_pt() / COLUMN_POINTS).ceil().max(1.0);
    let rows = (size.y.to_pt() / row_points).ceil().max(1.0);

    let page = Some((columns * COLUMN_POINTS, rows * row_points));
    let document = compile(PAGE_ID, source, ink, page)?;
    let png = typst_render::render(document.pages.first()?, pixels_per_point(cell))
        .encode_png()
        .ok()?;
    Some(Math {
        data: base64(&png),
        columns: columns as usize,
        rows: rows as usize,
    })
}

/// Set one expression, on a page of the given size in points where there is one.
fn compile(
    template: &str,
    source: &str,
    ink: Color,
    page: Option<(f64, f64)>,
) -> Option<PagedDocument> {
    let (width, height) = page.unwrap_or_default();
    let mut inputs = Dict::new();
    inputs.insert("source".into(), source.into_value());
    inputs.insert("ink".into(), text_fill(ink).into_value());
    inputs.insert("size".into(), format!("{TEXT_SIZE_PT}pt").into_value());
    inputs.insert("width".into(), format!("{width}pt").into_value());
    inputs.insert("height".into(), format!("{height}pt").into_value());
    engine().compile_with_input(template, inputs).output.ok()
}

/// Encode bytes for the terminal image protocols without bringing a second base64 dependency.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The cell the tests set their mathematics for, so a shape the terminal happens to report
    /// never decides what they measure.
    const CELL: Cell = Cell {
        width: 9,
        height: 18,
        measured: true,
    };

    #[test]
    fn typesets_math_to_a_png() {
        let math = render_math("frac(-b plus.minus sqrt(b^2 - 4 a c), 2 a)", Color::White)
            .expect("it renders");
        assert!(math.data.starts_with("iVBOR"), "PNG data: {}", math.data);
    }

    /// The ink is the reader's, so the same formula in two themes is two pictures.
    #[test]
    fn the_palette_reaches_the_page() {
        let dark = render_math("x^2", Color::Rgb(0xee, 0xee, 0xee)).expect("it renders");
        let light = render_math("x^2", Color::Rgb(0x11, 0x11, 0x11)).expect("it renders");
        assert_ne!(dark.data, light.data);
    }

    /// A formula is drawn at the size of the words around it, not at the size it was rasterized
    /// into: a two-line fraction is a handful of rows, whatever resolution it was rendered at.
    #[test]
    fn a_formula_asks_for_the_room_its_own_size_needs() {
        let math = typeset(
            "frac(-b plus.minus sqrt(b^2 - 4 a c), 2 a)",
            Color::White,
            CELL,
        )
        .expect("it renders");

        assert!(
            (2..=5).contains(&math.rows),
            "a stacked fraction wants a few rows, asked for {}",
            math.rows
        );
        assert!(
            (10..=40).contains(&math.columns),
            "and about as many columns as it has characters, asked for {}",
            math.columns
        );
    }

    /// Typesetting is far too slow to sit on the path a running answer redraws, so a formula that
    /// has been seen comes back without being typeset again.
    #[test]
    fn a_formula_is_only_typeset_once() {
        use std::sync::atomic::Ordering;

        let source = "sum_(k=1)^n k = (n(n+1))/2";
        let first = render_math(source, Color::White).expect("it renders");
        let after_first = TYPESET.load(Ordering::Relaxed);

        let again = render_math(source, Color::White).expect("it renders");

        assert_eq!(first, again);
        assert_eq!(
            TYPESET.load(Ordering::Relaxed),
            after_first,
            "the second ask went to Typst rather than to the cache"
        );
    }

    /// The terminal scales a formula to fill the cells it asked for, so the picture has to be the
    /// shape of those cells: a picture of any other shape is drawn stretched.
    #[test]
    fn a_formula_is_the_shape_of_the_cells_it_asks_for() {
        for source in [
            "e^(i pi) + 1 = 0",
            "frac(-b plus.minus sqrt(b^2 - 4 a c), 2 a)",
            "integral_(-infinity)^infinity e^(-x^2) dif x = sqrt(pi)",
            "sum_(n=1)^infinity 1/n^2 = pi^2/6",
        ] {
            let math = typeset(source, Color::White, CELL).expect("it renders");
            let (width, height) = crate::images::pixel_size(&math.data).expect("a png");

            let picture = width as f64 / height as f64;
            let cells = (math.columns * CELL.width) as f64 / (math.rows * CELL.height) as f64;
            assert!(
                (picture / cells - 1.0).abs() < 0.01,
                "{source} is {picture:.3} across for every one down, in cells {cells:.3}"
            );
        }
    }

    /// A formula is set for the terminal it will be drawn in, so one whose cells are a different
    /// shape gets a different picture rather than the same one stretched.
    #[test]
    fn the_shape_of_the_terminal_reaches_the_page() {
        let tall = typeset(
            "x^2",
            Color::White,
            Cell {
                width: 10,
                height: 30,
                measured: true,
            },
        )
        .expect("it renders");
        let square = typeset(
            "x^2",
            Color::White,
            Cell {
                width: 10,
                height: 10,
                measured: true,
            },
        )
        .expect("it renders");

        assert!(
            tall.rows < square.rows,
            "taller cells, fewer of them: {} against {}",
            tall.rows,
            square.rows
        );
        assert_ne!(tall.data, square.data);
    }

    #[test]
    fn something_that_is_not_maths_at_all_draws_nothing() {
        assert_eq!(render_math("   ", Color::White), None);
        assert_eq!(render_math("frac(1,", Color::White), None);
    }
}
