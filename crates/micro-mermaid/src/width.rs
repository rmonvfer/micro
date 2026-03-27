//! Display width, measured in grapheme clusters.
//!
//! A cluster is the unit both of measuring and of painting, so a box is always
//! sized for exactly what gets drawn into it. Sizing by cluster but painting by
//! code point is what would make a family emoji or a flag overflow its border.
//!
//! Clustering comes from `unicode-segmentation` (UAX #29), which already
//! handles ZWJ sequences, skin-tone modifiers, variation selectors, keycaps,
//! flags and Hangul. Per-code-point widths come from `unicode-width`.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// Variation selector 16, which requests emoji presentation for the character
/// before it and forces two columns regardless of that character's own width.
const VS16: char = '\u{fe0f}';

fn is_regional_indicator(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

/// Columns occupied by one grapheme cluster.
///
/// The widest code point wins, so a base plus its combining marks measures as
/// the base. Two adjustments: a variation selector requesting emoji
/// presentation forces two columns, as does a regional indicator pair (a flag).
///
/// Zero is a real answer — a soft hyphen or zero-width space occupies nothing,
/// and callers skip painting such a cluster rather than reserving a cell.
pub fn cluster_width(cluster: &str) -> usize {
    let mut w = 0usize;
    let mut vs16 = false;
    let mut regional = 0u32;
    for ch in cluster.chars() {
        if ch == VS16 {
            vs16 = true;
        }
        if is_regional_indicator(ch) {
            regional += 1;
        }
        let cw = ch.width().unwrap_or(0);
        if cw > w {
            w = cw;
        }
    }
    if vs16 || regional >= 2 {
        2
    } else {
        w
    }
}

/// Iterate clusters paired with their display width.
pub fn measured(s: &str) -> impl Iterator<Item = (&str, usize)> {
    s.graphemes(true).map(|g| (g, cluster_width(g)))
}

/// Display columns of a string.
pub fn string_width(s: &str) -> usize {
    s.graphemes(true).map(cluster_width).sum()
}
