//! Large pastes, held aside and stood in for by a marker.

use std::collections::BTreeMap;


const MAX_LINES: usize = 10;
/// So is one longer than this many characters.
const MAX_CHARS: usize = 1_000;
/// A tab in pasted text becomes this many spaces.
const TAB: &str = "    ";

/// Pastes held aside, by the number in their marker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PasteStore {
    entries: BTreeMap<usize, String>,
    /// Numbers are never reused within a prompt, so a marker always means one paste.
    counter: usize,
}

impl PasteStore {
    pub fn new() -> Self {
        PasteStore::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: usize) -> Option<&str> {
        self.entries.get(&id).map(String::as_str)
    }

    /// Hold a paste aside and return the marker that stands in for it.
    pub fn store(&mut self, text: &str) -> String {
        self.counter += 1;
        let id = self.counter;
        let marker = marker_for(id, text);
        self.entries.insert(id, text.to_string());
        marker
    }

    /// Forget one paste and close the gap its number left.
    pub fn remove(&mut self, id: usize) -> Vec<(usize, usize)> {
        if self.entries.remove(&id).is_none() {
            return Vec::new();
        }
        let moved: Vec<(usize, usize)> = self
            .entries
            .keys()
            .filter(|key| **key > id)
            .map(|key| (*key, *key - 1))
            .collect();
        for (from, to) in &moved {
            if let Some(text) = self.entries.remove(from) {
                self.entries.insert(*to, text);
            }
        }
        self.counter = self.counter.saturating_sub(1);
        moved
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.counter = 0;
    }

    /// Put every held-aside paste back where its marker stands.
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(found) = find_marker(rest) {
            out.push_str(&rest[..found.start]);
            match self.get(found.id) {
                Some(paste) => out.push_str(paste),
                
                None => out.push_str(&rest[found.start..found.end]),
            }
            rest = &rest[found.end..];
        }
        out.push_str(rest);
        out
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub start: usize,
    pub end: usize,
    pub id: usize,
}

/// True when a paste is too big to put in the prompt as it is.
pub fn is_large(text: &str) -> bool {
    text.lines().count() > MAX_LINES || text.chars().count() > MAX_CHARS
}

/// The marker for a paste: lines when there are too many of them, characters otherwise.
pub fn marker_for(id: usize, text: &str) -> String {
    let lines = text.split('\n').count();
    match lines > MAX_LINES {
        true => format!("[paste #{id} +{lines} lines]"),
        false => format!("[paste #{id} {} chars]", text.chars().count()),
    }
}

/// Tidy pasted text into something that can go in a prompt.
pub fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', TAB)
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .collect()
}

/// Whether a space is needed before pasting, so a path does not fuse onto a word.
pub fn needs_separator(before: &str, paste: &str) -> bool {
    let starts_path = paste.starts_with(['/', '~', '.']);
    let after_word = before
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_');
    starts_path && after_word
}

/// The first marker in `text`, if there is one.
pub fn find_marker(text: &str) -> Option<Marker> {
    let mut from = 0;
    while let Some(offset) = text[from..].find("[paste #") {
        let start = from + offset;
        match parse_marker(&text[start..]) {
            Some((length, id)) => {
                return Some(Marker {
                    start,
                    end: start + length,
                    id,
                })
            }
            None => from = start + "[paste #".len(),
        }
    }
    None
}


pub fn marker_ending_at(text: &str, index: usize) -> Option<Marker> {
    let mut from = 0;
    while let Some(marker) = find_marker(&text[from..]) {
        let marker = Marker {
            start: from + marker.start,
            end: from + marker.end,
            id: marker.id,
        };
        if marker.end == index {
            return Some(marker);
        }
        if marker.end >= index {
            return None;
        }
        from = marker.end;
    }
    None
}

/// The marker beginning exactly at `index`, which forward motion steps over.
pub fn marker_starting_at(text: &str, index: usize) -> Option<Marker> {
    if index >= text.len() {
        return None;
    }
    find_marker(&text[index..])
        .filter(|marker| marker.start == 0)
        .map(|marker| Marker {
            start: index,
            end: index + marker.end,
            id: marker.id,
        })
}

/// The marker containing `index`, for motion that must step over one whole.
pub fn marker_containing(text: &str, index: usize) -> Option<Marker> {
    let mut from = 0;
    while let Some(marker) = find_marker(&text[from..]) {
        let marker = Marker {
            start: from + marker.start,
            end: from + marker.end,
            id: marker.id,
        };
        if marker.start < index && index < marker.end {
            return Some(marker);
        }
        if marker.start >= index {
            return None;
        }
        from = marker.end;
    }
    None
}

/// Rewrite the numbers in markers after a paste was removed.
pub fn renumber(text: &str, moved: &[(usize, usize)]) -> String {
    let mut out = text.to_string();
    
    let mut moved = moved.to_vec();
    moved.sort_by_key(|(from, _)| *from);
    for (from, to) in moved {
        out = out.replace(&format!("[paste #{from} "), &format!("[paste #{to} "));
    }
    out
}

/// Parse a marker at the start of `text`, returning its length and number.
fn parse_marker(text: &str) -> Option<(usize, usize)> {
    let rest = text.strip_prefix("[paste #")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let id = digits.parse().ok()?;
    let after = &rest[digits.len()..];
    let close = after.find(']')?;
    
    let body = &after[..close];
    let valid = body.is_empty()
        || (body.starts_with(" +") && body.ends_with(" lines"))
        || (body.starts_with(' ') && body.ends_with(" chars"));
    match valid {
        true => Some(("[paste #".len() + digits.len() + close + 1, id)),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_paste_goes_in_as_it_is() {
        assert!(!is_large("one\ntwo"));
        assert!(!is_large(&"x".repeat(MAX_CHARS)));
    }

    #[test]
    fn a_paste_over_either_limit_is_held_aside() {
        assert!(is_large(&"line\n".repeat(MAX_LINES + 1)));
        assert!(is_large(&"x".repeat(MAX_CHARS + 1)));
    }

    #[test]
    fn the_line_form_wins_when_a_paste_is_over_both_limits() {
        let text = "a".repeat(MAX_CHARS + 1) + &"\nb".repeat(MAX_LINES + 1);
        assert!(marker_for(1, &text).ends_with("lines]"));
    }

    #[test]
    fn a_marker_says_what_it_stands_for() {
        assert_eq!(marker_for(1, &"x\n".repeat(11)), "[paste #1 +12 lines]");
        assert_eq!(marker_for(2, &"y".repeat(1500)), "[paste #2 1500 chars]");
    }

    #[test]
    fn cleaning_normalizes_line_endings_and_tabs() {
        assert_eq!(clean("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(clean("a\tb"), "a    b");
    }

    #[test]
    fn cleaning_drops_control_characters_but_keeps_newlines() {
        assert_eq!(clean("a\u{1b}[31mb\nc"), "a[31mb\nc");
        assert_eq!(clean("a\u{7}b"), "ab");
    }

    #[test]
    fn a_path_pasted_after_a_word_needs_a_space_first() {
        assert!(needs_separator("cat", "/etc/hosts"));
        assert!(needs_separator("open", "~/notes"));
        assert!(needs_separator("run", "./script"));
        assert!(!needs_separator("cat ", "/etc/hosts"));
        assert!(!needs_separator("cat", "hello"));
    }

    #[test]
    fn a_stored_paste_comes_back_where_its_marker_stands() {
        let mut store = PasteStore::new();
        let marker = store.store("the whole thing");
        let text = format!("before {marker} after");
        assert_eq!(store.expand(&text), "before the whole thing after");
    }

    #[test]
    fn a_marker_with_nothing_behind_it_is_left_as_text() {
        let store = PasteStore::new();
        assert_eq!(store.expand("[paste #7 12 chars]"), "[paste #7 12 chars]");
    }

    #[test]
    fn removing_a_paste_closes_the_gap_its_number_left() {
        let mut store = PasteStore::new();
        store.store("first");
        store.store("second");
        store.store("third");

        let moved = store.remove(1);
        assert_eq!(moved, vec![(2, 1), (3, 2)]);
        assert_eq!(store.get(1), Some("second"));
        assert_eq!(store.get(2), Some("third"));
        assert_eq!(store.get(3), None);
    }

    #[test]
    fn renumbering_rewrites_the_markers_to_match() {
        let text = "[paste #2 5 chars] and [paste #3 6 chars]";
        assert_eq!(
            renumber(text, &[(2, 1), (3, 2)]),
            "[paste #1 5 chars] and [paste #2 6 chars]"
        );
    }

    #[test]
    fn a_marker_is_found_by_where_it_ends() {
        let text = "x [paste #1 9 chars] y";
        let marker = marker_ending_at(text, 20).expect("a marker ends there");
        assert_eq!((marker.start, marker.id), (2, 1));
        assert!(marker_ending_at(text, 19).is_none());
    }

    #[test]
    fn a_position_inside_a_marker_reports_the_whole_marker() {
        let text = "[paste #1 9 chars]";
        assert!(marker_containing(text, 5).is_some());
        assert!(
            marker_containing(text, 0).is_none(),
            "the edge is not inside"
        );
        assert!(marker_containing(text, 18).is_none());
    }

    #[test]
    fn text_that_merely_looks_like_a_marker_is_not_one() {
        assert!(find_marker("[paste #] nothing").is_none());
        assert!(find_marker("[paste #1 something else]").is_none());
        assert!(find_marker("[paste #1]").is_some());
    }
}
