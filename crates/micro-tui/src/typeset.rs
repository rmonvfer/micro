//! Textbook-quality display mathematics, rasterized by Typst for graphical terminals.

use ratatui::style::Color;
use typst::layout::PagedDocument;
use typst_as_lib::TypstEngine;

const PIXELS_PER_POINT: f32 = 3.0;

/// The page color used to rasterize mathematics.
fn page_fill(background: Color) -> String {
    match background {
        Color::Rgb(red, green, blue) => {
            format!("rgb(\"#{red:02x}{green:02x}{blue:02x}\")")
        }
        Color::Indexed(index) => format!("luma({})", index as f32 / 255.0),
        Color::Black => "black".to_string(),
        Color::Red => "red".to_string(),
        Color::Green => "green".to_string(),
        Color::Yellow => "yellow".to_string(),
        Color::Blue => "blue".to_string(),
        Color::Magenta => "magenta".to_string(),
        Color::Cyan => "cyan".to_string(),
        Color::Gray => "gray".to_string(),
        Color::DarkGray => "rgb(#{555555})".to_string(),
        Color::LightRed => "rgb(#{ff5555})".to_string(),
        Color::LightGreen => "rgb(#{55ff55})".to_string(),
        Color::LightYellow => "rgb(#{ffff55})".to_string(),
        Color::LightBlue => "rgb(#{5555ff})".to_string(),
        Color::LightMagenta => "rgb(#{ff55ff})".to_string(),
        Color::LightCyan => "rgb(#{55ffff})".to_string(),
        Color::White => "white".to_string(),
        Color::Reset => "none".to_string(),
    }
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
pub fn render_math(source: &str, foreground: Color, background: Color) -> Option<String> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }

    let document = format!(
        "#set page(width: auto, height: auto, margin: 4pt, fill: {})\n#set text(size: 12pt, fill: {})\n$ {source} $",
        page_fill(background),
        text_fill(foreground),
    );
    let engine = TypstEngine::builder()
        .main_file(document)
        .search_fonts_with(Default::default())
        .build();
    let document: PagedDocument = engine.compile().output.ok()?;
    let page = document.pages.first()?;
    let png = typst_render::render(page, PIXELS_PER_POINT)
        .encode_png()
        .ok()?;
    Some(base64(&png))
}

/// Encode bytes for the terminal image protocols without bringing a second base64 dependency.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
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

    #[test]
    fn typesets_math_to_a_png() {
        let png = render_math(
            "frac(-b plus.minus sqrt(b^2 - 4 a c), 2 a)",
            Color::White,
            Color::Black,
        )
        .expect("it renders");
        assert!(png.starts_with("iVBOR"), "PNG data: {png}");
    }
}
