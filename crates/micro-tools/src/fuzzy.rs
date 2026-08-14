//! Finding the text an edit meant, when it is not quite the text that is there.
//!
//! A model writes what it read, and what it read came back through a terminal, a diff, or
//! its own tokenizer. Along the way a straight quote can become a curly one, a hyphen an
//! en-dash, a space a non-breaking space, and trailing whitespace can vanish. The file is
//! unchanged; only the description of it drifted.
//!
//! An exact match is always preferred. When there is none, both sides are put into a form
//! where those differences do not count, and the match is looked for again. What gets
//! written back is the normalized text, so the file ends up saying what the edit meant.

use unicode_normalization::UnicodeNormalization as _;

/// Where an edit's target was found, and in which text.
pub struct Match {
    /// Byte offset of the match within [`Match::haystack`].
    pub start: usize,
    /// Byte length of the match within [`Match::haystack`].
    pub length: usize,
    /// The text the offsets refer to: the original when the match was exact, the
    /// normalized form when it was not.
    pub haystack: String,
    /// Whether the differences had to be set aside to find it.
    pub fuzzy: bool,
}

/// Put text into a form where presentational differences do not count.
pub fn normalize(text: &str) -> String {
    let composed: String = text.nfkc().collect();
    let mut out = String::with_capacity(composed.len());

    for (index, line) in composed.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mapped: String = line
            .chars()
            .map(|character| match character {
                // Smart single quotes.
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
                // Smart double quotes.
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
                // Hyphen, non-breaking hyphen, figure dash, en dash, em dash,
                // horizontal bar, minus sign.
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => '-',
                // Spaces that are not the ordinary one.
                '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
                other => other,
            })
            .collect();
        out.push_str(mapped.trim_end());
    }

    out
}

/// Find `needle` in `haystack`, exactly if it is there and forgivingly if it is not.
pub fn find(haystack: &str, needle: &str) -> Option<Match> {
    if let Some(start) = haystack.find(needle) {
        return Some(Match {
            start,
            length: needle.len(),
            haystack: haystack.to_string(),
            fuzzy: false,
        });
    }

    let normalized_haystack = normalize(haystack);
    let normalized_needle = normalize(needle);
    let start = normalized_haystack.find(&normalized_needle)?;
    Some(Match {
        start,
        length: normalized_needle.len(),
        haystack: normalized_haystack,
        fuzzy: true,
    })
}

/// How many times `needle` occurs in `haystack`, counting the way [`find`] looks.
///
/// Counting happens in whichever text a match was found in, so an exact match counts
/// exact occurrences and a forgiving one counts forgiving occurrences. Mixing the two
/// would report a count for text that is not the text being edited.
pub fn count(haystack: &str, needle: &str) -> usize {
    let exact = haystack.matches(needle).count();
    if exact > 0 {
        return exact;
    }
    normalize(haystack).matches(&normalize(needle)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_match_is_taken_as_it_is() {
        let found = find("let a = 1;", "a = 1").expect("it is there");
        assert!(!found.fuzzy);
        assert_eq!(
            &found.haystack[found.start..found.start + found.length],
            "a = 1"
        );
    }

    /// A curly quote in the edit against a straight one in the file, which is what comes
    /// back when text has been through a renderer.
    #[test]
    fn a_smart_quote_still_finds_a_straight_one() {
        let file = "let name = \"micro\";";
        let asked = "let name = \u{201C}micro\u{201D};";

        let found = find(file, asked).expect("the quotes do not count");
        assert!(found.fuzzy);
        assert_eq!(
            &found.haystack[found.start..found.start + found.length],
            "let name = \"micro\";"
        );
    }

    #[test]
    fn a_dash_that_is_not_a_hyphen_still_matches() {
        let found = find("--flag", "\u{2013}-flag").expect("a dash is a dash");
        assert!(found.fuzzy);
    }

    #[test]
    fn a_non_breaking_space_still_matches() {
        let found = find("a b", "a\u{00A0}b").expect("a space is a space");
        assert!(found.fuzzy);
    }

    #[test]
    fn trailing_whitespace_does_not_count() {
        let found = find("let a = 1;\nlet b = 2;", "let a = 1;   \nlet b = 2;")
            .expect("trailing space does not count");
        assert!(found.fuzzy);
    }

    #[test]
    fn text_that_is_genuinely_absent_is_not_found() {
        assert!(find("let a = 1;", "let c = 3;").is_none());
    }

    #[test]
    fn occurrences_are_counted_the_way_they_are_matched() {
        assert_eq!(count("a a a", "a"), 3);
        // No exact occurrence, so the forgiving reading is what counts.
        assert_eq!(count("\"x\" and \"x\"", "\u{201C}x\u{201D}"), 2);
        assert_eq!(count("let a = 1;", "nope"), 0);
    }
}
