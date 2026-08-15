//! Display width, measured in grapheme clusters.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// Variation selector 16.
const VS16: char = '\u{fe0f}';

fn is_regional_indicator(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

/// Columns occupied by one grapheme cluster.
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
