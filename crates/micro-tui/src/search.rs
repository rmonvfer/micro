//! Finding text in the rendered transcript.

use crate::wrap::text_width;
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;

/// One row's part of a match, in columns from the transcript's left edge, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// One occurrence of the query, which may wrap across rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub segments: Vec<Segment>,
}

impl Match {
    pub fn first_row(&self) -> usize {
        self.segments.first().map_or(0, |segment| segment.row)
    }

    pub fn last_row(&self) -> usize {
        self.segments.last().map_or(0, |segment| segment.row)
    }

    /// Where it starts and ends, which stays the same while the rows around it do not move.
    fn key(&self) -> Option<(usize, usize, usize, usize)> {
        let first = self.segments.first()?;
        let last = self.segments.last()?;
        Some((first.row, first.start, last.row, last.end))
    }
}

/// The transcript as one run of lowercase text, every run of whitespace and every row break folded
/// into a single space, with the cell each character was drawn in.
struct Corpus {
    text: Vec<char>,
    cells: Vec<Option<Segment>>,
}

fn corpus(lines: &[Line<'_>]) -> Corpus {
    let mut text = Vec::new();
    let mut cells = Vec::new();
    let mut gap = false;

    for (row, line) in lines.iter().enumerate() {
        let mut column = 0;
        for span in &line.spans {
            for grapheme in span.content.graphemes(true) {
                let width = text_width(grapheme);
                if grapheme.trim().is_empty() {
                    gap = !text.is_empty();
                    column += width;
                    continue;
                }
                if gap {
                    text.push(' ');
                    cells.push(None);
                    gap = false;
                }
                let cell = Segment {
                    row,
                    start: column,
                    end: column + width,
                };
                for character in grapheme.chars().flat_map(char::to_lowercase) {
                    text.push(character);
                    cells.push(Some(cell));
                }
                column += width;
            }
        }
        gap = !text.is_empty();
    }
    Corpus { text, cells }
}

/// The query as it is matched: lowercase, with every run of whitespace a single space.
fn needle(query: &str) -> Vec<char> {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

/// Every place `query` appears in the rendered rows, ignoring case and treating any whitespace,
/// including a wrapped row's break, as one space.
pub fn find(lines: &[Line<'_>], query: &str) -> Vec<Match> {
    let needle = needle(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let corpus = corpus(lines);

    let mut matches = Vec::new();
    let mut at = 0;
    while at + needle.len() <= corpus.text.len() {
        if corpus.text[at..at + needle.len()] != needle[..] {
            at += 1;
            continue;
        }
        let mut segments: Vec<Segment> = Vec::new();
        for cell in corpus.cells[at..at + needle.len()].iter().flatten() {
            match segments.last_mut() {
                Some(last) if last.row == cell.row => last.end = last.end.max(cell.end),
                _ => segments.push(*cell),
            }
        }
        if !segments.is_empty() {
            matches.push(Match { segments });
        }
        at += needle.len();
    }
    matches
}

/// What the next refresh does with the selected match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// Keep it, without moving the transcript.
    Retain,
    /// The query changed: take the first match at or below this row.
    Query { anchor: usize },
    /// Step to the match after it.
    Next,
    /// Step to the match before it.
    Previous,
}

/// A search through the transcript: what is asked, what was found, and which one is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    query: String,
    matches: Vec<Match>,
    selected: Option<usize>,
    selected_key: Option<(usize, usize, usize, usize)>,
    pending: Pending,
    /// The query and transcript version the matches were found for.
    searched: Option<(String, u64)>,
    /// The row the next change of query searches from.
    anchor: usize,
}

impl Search {
    /// A search with nothing asked yet, which will start looking from `anchor`.
    pub fn new(anchor: usize) -> Self {
        Search {
            query: String::new(),
            matches: Vec::new(),
            selected: None,
            selected_key: None,
            pending: Pending::Retain,
            searched: None,
            anchor,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Type onto the end of the query.
    pub fn push(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        self.query.push_str(&text);
        self.query_changed();
    }

    /// Take the last character off the query.
    pub fn pop(&mut self) {
        if self.query.pop().is_some() {
            self.query_changed();
        }
    }

    /// Empty the query.
    pub fn clear(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.query_changed();
        }
    }

    fn query_changed(&mut self) {
        let anchor = self
            .selected
            .and_then(|index| self.matches.get(index))
            .map_or(self.anchor, Match::first_row);
        self.pending = Pending::Query { anchor };
    }

    pub fn next(&mut self) {
        if !self.query.trim().is_empty() {
            self.pending = Pending::Next;
        }
    }

    pub fn previous(&mut self) {
        if !self.query.trim().is_empty() {
            self.pending = Pending::Previous;
        }
    }

    /// Bring the matches up to date with the rows, and say which rows the selected match spans
    /// when it ought to be brought into view.
    pub fn refresh(&mut self, lines: &[Line<'_>], version: u64) -> Option<(usize, usize)> {
        if self.query.trim().is_empty() {
            self.matches.clear();
            self.selected = None;
            self.selected_key = None;
            self.pending = Pending::Retain;
            self.searched = None;
            return None;
        }

        let key = (self.query.clone(), version);
        let changed = self.searched.as_ref() != Some(&key);
        if changed {
            self.matches = find(lines, &self.query);
            self.searched = Some(key);
        }
        if !changed && self.pending == Pending::Retain {
            return None;
        }

        let exact = match changed {
            true => self.selected_key.and_then(|wanted| {
                self.matches
                    .iter()
                    .position(|found| found.key() == Some(wanted))
            }),
            false => self.selected,
        };
        let count = self.matches.len();
        let base = exact.or(self
            .selected
            .map(|index| index.min(count.saturating_sub(1))));
        let selected = match (count, self.pending) {
            (0, _) => None,
            (_, Pending::Query { anchor }) => Some(
                self.matches
                    .iter()
                    .position(|found| found.first_row() >= anchor)
                    .unwrap_or(0),
            ),
            (_, Pending::Next) => Some(base.map_or(0, |index| (index + 1) % count)),
            (_, Pending::Previous) => {
                Some(base.map_or(count - 1, |index| (index + count - 1) % count))
            }
            (_, Pending::Retain) => Some(base.unwrap_or(0)),
        };

        let reveal = self.pending != Pending::Retain;
        self.selected = selected;
        self.selected_key = selected.and_then(|index| self.matches[index].key());
        self.pending = Pending::Retain;

        let shown = &self.matches[selected?];
        reveal.then(|| (shown.first_row(), shown.last_row()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn rows(texts: &[&str]) -> Vec<Line<'static>> {
        texts
            .iter()
            .map(|text| Line::raw(text.to_string()))
            .collect()
    }

    #[test]
    fn a_match_is_found_whatever_its_case() {
        let found = find(&rows(&["Hello World", "hello again"]), "HELLO");
        assert_eq!(
            found,
            vec![
                Match {
                    segments: vec![Segment {
                        row: 0,
                        start: 0,
                        end: 5
                    }]
                },
                Match {
                    segments: vec![Segment {
                        row: 1,
                        start: 0,
                        end: 5
                    }]
                },
            ]
        );
    }

    #[test]
    fn a_phrase_is_found_across_a_wrapped_row() {
        let found = find(&rows(&[" the quick", " brown fox"]), "quick brown");
        assert_eq!(
            found,
            vec![Match {
                segments: vec![
                    Segment {
                        row: 0,
                        start: 5,
                        end: 10
                    },
                    Segment {
                        row: 1,
                        start: 1,
                        end: 6
                    },
                ]
            }]
        );
    }

    #[test]
    fn the_space_inside_a_match_on_one_row_is_part_of_it() {
        let found = find(&rows(&["say hello  world now"]), "hello world");
        assert_eq!(
            found[0].segments,
            vec![Segment {
                row: 0,
                start: 4,
                end: 16
            }]
        );
    }

    #[test]
    fn columns_count_wide_characters_and_follow_the_spans() {
        let line = Line::from(vec![Span::raw("界 "), Span::raw("needle")]);
        let found = find(&[line], "needle");
        assert_eq!(
            found[0].segments,
            vec![Segment {
                row: 0,
                start: 3,
                end: 9
            }]
        );
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        assert!(find(&rows(&["anything"]), "   ").is_empty());
    }

    #[test]
    fn a_query_selects_the_first_match_from_where_it_was_opened() {
        let lines = rows(&["match", "x", "match", "x", "match"]);
        let mut search = Search::new(1);
        search.push("match");
        assert_eq!(search.refresh(&lines, 0), Some((2, 2)));
        assert_eq!(search.selected(), Some(1));
        assert_eq!(search.matches().len(), 3);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let lines = rows(&["match", "match", "match"]);
        let mut search = Search::new(0);
        search.push("match");
        search.refresh(&lines, 0);
        assert_eq!(search.selected(), Some(0));

        search.previous();
        assert_eq!(search.refresh(&lines, 0), Some((2, 2)));
        assert_eq!(search.selected(), Some(2));

        search.next();
        search.refresh(&lines, 0);
        assert_eq!(search.selected(), Some(0));
    }

    #[test]
    fn a_settled_search_does_not_move_the_transcript_again() {
        let lines = rows(&["match"]);
        let mut search = Search::new(0);
        search.push("match");
        assert!(search.refresh(&lines, 0).is_some());
        assert_eq!(search.refresh(&lines, 0), None);
    }

    #[test]
    fn the_selected_match_survives_rows_arriving_below_it() {
        let mut lines = rows(&["match", "match"]);
        let mut search = Search::new(0);
        search.push("match");
        search.refresh(&lines, 0);
        search.next();
        search.refresh(&lines, 0);
        assert_eq!(search.selected(), Some(1));

        lines.push(Line::raw("match"));
        assert_eq!(search.refresh(&lines, 1), None);
        assert_eq!(search.selected(), Some(1));
        assert_eq!(search.matches().len(), 3);
    }

    #[test]
    fn editing_the_query_back_to_nothing_clears_the_matches() {
        let lines = rows(&["ab"]);
        let mut search = Search::new(0);
        search.push("a");
        search.refresh(&lines, 0);
        search.pop();
        assert_eq!(search.refresh(&lines, 0), None);
        assert!(search.matches().is_empty());
        assert_eq!(search.selected(), None);
    }
}
