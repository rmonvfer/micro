//! Line-based differencing.

use crate::theme::Theme;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;

/// Beyond this many lines the alignment is abandoned in favour of a wholesale replacement.
const MAX_ALIGNED_LINES: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Equal(String),
    Delete(String),
    Insert(String),
}

/// Align `old` against `new`, line by line.
pub fn diff_lines(old: &str, new: &str) -> Vec<Change> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);

    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut changes: Vec<Change> = old_lines[..prefix]
        .iter()
        .map(|line| Change::Equal((*line).to_string()))
        .collect();
    changes.extend(align(
        &old_lines[prefix..old_lines.len() - suffix],
        &new_lines[prefix..new_lines.len() - suffix],
    ));
    changes.extend(
        old_lines[old_lines.len() - suffix..]
            .iter()
            .map(|line| Change::Equal((*line).to_string())),
    );
    changes
}


fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n').collect()
}

fn align(old: &[&str], new: &[&str]) -> Vec<Change> {
    if old.is_empty() {
        return new
            .iter()
            .map(|line| Change::Insert((*line).to_string()))
            .collect();
    }
    if new.is_empty() {
        return old
            .iter()
            .map(|line| Change::Delete((*line).to_string()))
            .collect();
    }
    if old.len() + new.len() > MAX_ALIGNED_LINES {
        let mut changes: Vec<Change> = old
            .iter()
            .map(|line| Change::Delete((*line).to_string()))
            .collect();
        changes.extend(new.iter().map(|line| Change::Insert((*line).to_string())));
        return changes;
    }
    myers(old, new)
}


fn myers(old: &[&str], new: &[&str]) -> Vec<Change> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (old.len() + new.len()) as isize;
    
    let offset = max + 1;
    let mut furthest = vec![0isize; (2 * max + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();

    for steps in 0..=max {
        trace.push(furthest.clone());
        let mut diagonal = -steps;
        while diagonal <= steps {
            let index = (diagonal + offset) as usize;
            let mut x = if diagonal == -steps
                || (diagonal != steps && furthest[index - 1] < furthest[index + 1])
            {
                furthest[index + 1]
            } else {
                furthest[index - 1] + 1
            };
            let mut y = x - diagonal;

            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            furthest[index] = x;

            if x >= n && y >= m {
                return backtrack(&trace, old, new, offset);
            }
            diagonal += 2;
        }
    }

    unreachable!("a path of at most old.len() + new.len() steps always exists")
}

fn backtrack(trace: &[Vec<isize>], old: &[&str], new: &[&str], offset: isize) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut x = old.len() as isize;
    let mut y = new.len() as isize;

    for (steps, furthest) in trace.iter().enumerate().rev() {
        let steps = steps as isize;
        let diagonal = x - y;
        let index = (diagonal + offset) as usize;
        let previous_diagonal = if diagonal == -steps
            || (diagonal != steps && furthest[index - 1] < furthest[index + 1])
        {
            diagonal + 1
        } else {
            diagonal - 1
        };
        let previous_x = furthest[(previous_diagonal + offset) as usize];
        let previous_y = previous_x - previous_diagonal;

        while x > previous_x && y > previous_y {
            changes.push(Change::Equal(old[(x - 1) as usize].to_string()));
            x -= 1;
            y -= 1;
        }

        if steps > 0 {
            if x == previous_x {
                changes.push(Change::Insert(new[(y - 1) as usize].to_string()));
                y -= 1;
            } else {
                changes.push(Change::Delete(old[(x - 1) as usize].to_string()));
                x -= 1;
            }
        }
    }

    changes.reverse();
    changes
}

/// What a rendered diff line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// The gap left where unchanged lines were skipped.
    Elision,
}

/// One line of a diff as it is shown: a marker, a line number, and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub number: Option<usize>,
    pub text: String,
}

/// How many unchanged lines are kept either side of a change.
pub const DEFAULT_CONTEXT: usize = 4;

/// A tab is this many spaces, so a diff of indented code lines up.
const TAB: &str = "   ";


pub fn format(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let parts = parts(&diff_lines(old, new));
    let mut lines = Vec::new();
    let mut old_number = 1;
    let mut new_number = 1;
    let mut after_change = false;

    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Added(texts) => {
                for text in texts {
                    lines.push(DiffLine {
                        kind: LineKind::Added,
                        number: Some(new_number),
                        text: text.clone(),
                    });
                    new_number += 1;
                }
                after_change = true;
            }
            Part::Removed(texts) => {
                for text in texts {
                    lines.push(DiffLine {
                        kind: LineKind::Removed,
                        number: Some(old_number),
                        text: text.clone(),
                    });
                    old_number += 1;
                }
                after_change = true;
            }
            Part::Equal(texts) => {
                let before_change = matches!(
                    parts.get(index + 1),
                    Some(Part::Added(_)) | Some(Part::Removed(_))
                );
                let keep = |range: std::ops::Range<usize>,
                            lines: &mut Vec<DiffLine>,
                            old_number: &mut usize,
                            new_number: &mut usize| {
                    for text in &texts[range] {
                        lines.push(DiffLine {
                            kind: LineKind::Context,
                            number: Some(*old_number),
                            text: text.clone(),
                        });
                        *old_number += 1;
                        *new_number += 1;
                    }
                };

                match (after_change, before_change) {
                    (true, true) if texts.len() <= context * 2 => {
                        keep(0..texts.len(), &mut lines, &mut old_number, &mut new_number);
                    }
                    (true, true) => {
                        keep(0..context, &mut lines, &mut old_number, &mut new_number);
                        let skipped = texts.len() - context * 2;
                        lines.push(elision());
                        old_number += skipped;
                        new_number += skipped;
                        keep(
                            texts.len() - context..texts.len(),
                            &mut lines,
                            &mut old_number,
                            &mut new_number,
                        );
                    }
                    (true, false) => {
                        let shown = context.min(texts.len());
                        keep(0..shown, &mut lines, &mut old_number, &mut new_number);
                        let skipped = texts.len() - shown;
                        if skipped > 0 {
                            lines.push(elision());
                            old_number += skipped;
                            new_number += skipped;
                        }
                    }
                    (false, true) => {
                        let skipped = texts.len().saturating_sub(context);
                        if skipped > 0 {
                            lines.push(elision());
                            old_number += skipped;
                            new_number += skipped;
                        }
                        keep(
                            skipped..texts.len(),
                            &mut lines,
                            &mut old_number,
                            &mut new_number,
                        );
                    }
                    
                    (false, false) => {
                        old_number += texts.len();
                        new_number += texts.len();
                    }
                }
                after_change = false;
            }
        }
    }

    lines
}

fn elision() -> DiffLine {
    DiffLine {
        kind: LineKind::Elision,
        number: None,
        text: "...".to_string(),
    }
}

/// Consecutive changes of one kind, which is the unit the context rules work on.
enum Part {
    Equal(Vec<String>),
    Added(Vec<String>),
    Removed(Vec<String>),
}

fn parts(changes: &[Change]) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for change in changes {
        match (change, parts.last_mut()) {
            (Change::Equal(text), Some(Part::Equal(texts)))
            | (Change::Insert(text), Some(Part::Added(texts)))
            | (Change::Delete(text), Some(Part::Removed(texts))) => texts.push(text.clone()),
            (Change::Equal(text), _) => parts.push(Part::Equal(vec![text.clone()])),
            (Change::Insert(text), _) => parts.push(Part::Added(vec![text.clone()])),
            (Change::Delete(text), _) => parts.push(Part::Removed(vec![text.clone()])),
        }
    }
    parts
}

/// The width the line-number column needs, from the longer of the two files.
pub fn number_width(old: &str, new: &str) -> usize {
    let count = |text: &str| text.split('\n').count();
    count(old).max(count(new)).to_string().len()
}

/// The text of one line, gutter included: the marker, the number padded to `width`, a space, and
/// the content with tabs expanded.
pub fn gutter(line: &DiffLine, width: usize) -> String {
    let marker = match line.kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        LineKind::Context | LineKind::Elision => ' ',
    };
    let number = match line.number {
        Some(number) => format!("{number:>width$}"),
        None => " ".repeat(width),
    };
    format!("{marker}{number} {}", line.text.replace('\t', TAB))
}

/// Paint a laid-out diff, one styled line per input line.
pub fn paint(lines: &[DiffLine], width: usize, theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let mut painted = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        if lines[index].kind != LineKind::Removed {
            painted.push(plain(&lines[index], width, theme));
            index += 1;
            continue;
        }

        let removed_end = run_end(lines, index, LineKind::Removed);
        let added_end = run_end(lines, removed_end, LineKind::Added);
        let removed = &lines[index..removed_end];
        let added = &lines[removed_end..added_end];

        if removed.len() == 1 && added.len() == 1 {
            let (left, right) = intra_line(&removed[0], &added[0], width, theme);
            painted.push(left);
            painted.push(right);
        } else {
            painted.extend(removed.iter().map(|line| plain(line, width, theme)));
            painted.extend(added.iter().map(|line| plain(line, width, theme)));
        }
        index = added_end;
    }

    painted
}

fn run_end(lines: &[DiffLine], from: usize, kind: LineKind) -> usize {
    let mut end = from;
    while end < lines.len() && lines[end].kind == kind {
        end += 1;
    }
    end
}

fn color(kind: LineKind, theme: &Theme) -> Color {
    match kind {
        LineKind::Added => theme.tool_diff_added,
        LineKind::Removed => theme.tool_diff_removed,
        LineKind::Context | LineKind::Elision => theme.tool_diff_context,
    }
}

fn plain(line: &DiffLine, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    vec![Span::styled(
        gutter(line, width),
        Style::new().fg(color(line.kind, theme)),
    )]
}

/// The gutter alone: marker, padded number, and the space before the content.
fn prefix(line: &DiffLine, width: usize) -> String {
    let text = gutter(line, width);
    text.chars().take(width + 2).collect()
}

fn intra_line(
    removed: &DiffLine,
    added: &DiffLine,
    width: usize,
    theme: &Theme,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let old_text = removed.text.replace('\t', TAB);
    let new_text = added.text.replace('\t', TAB);

    let removed_style = Style::new().fg(theme.tool_diff_removed);
    let added_style = Style::new().fg(theme.tool_diff_added);
    let mut left = vec![Span::styled(prefix(removed, width), removed_style)];
    let mut right = vec![Span::styled(prefix(added, width), added_style)];
    let mut first_removed = true;
    let mut first_added = true;

    for change in diff_words(&old_text, &new_text) {
        match change {
            Change::Delete(text) => {
                push_changed(&mut left, &text, removed_style, &mut first_removed)
            }
            Change::Insert(text) => push_changed(&mut right, &text, added_style, &mut first_added),
            Change::Equal(text) => {
                left.push(Span::styled(text.clone(), removed_style));
                right.push(Span::styled(text, added_style));
            }
        }
    }

    (left, right)
}

/// Adds a run of changed words in inverse video.
fn push_changed(spans: &mut Vec<Span<'static>>, text: &str, style: Style, first: &mut bool) {
    let mut text = text;
    if *first {
        let lead = text.len() - text.trim_start().len();
        if lead > 0 {
            spans.push(Span::styled(text[..lead].to_string(), style));
        }
        text = &text[lead..];
        *first = false;
    }
    if !text.is_empty() {
        spans.push(Span::styled(
            text.to_string(),
            style.add_modifier(Modifier::REVERSED),
        ));
    }
}


fn diff_words(old: &str, new: &str) -> Vec<Change> {
    let old_tokens = word_tokens(old);
    let new_tokens = word_tokens(new);
    let old_refs: Vec<&str> = old_tokens.iter().map(String::as_str).collect();
    let new_refs: Vec<&str> = new_tokens.iter().map(String::as_str).collect();
    align(&old_refs, &new_refs)
}

pub fn word_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut trailing = false;

    for character in text.chars() {
        if character.is_whitespace() {
            current.push(character);
            trailing = true;
        } else {
            if trailing {
                tokens.push(std::mem::take(&mut current));
                trailing = false;
            }
            current.push(character);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    use ratatui::text::Span;

    fn text(change: &Change) -> &str {
        match change {
            Change::Equal(text) | Change::Delete(text) | Change::Insert(text) => text,
        }
    }

    fn rendered(changes: &[Change]) -> Vec<String> {
        changes
            .iter()
            .map(|change| match change {
                Change::Equal(text) => format!(" {text}"),
                Change::Delete(text) => format!("-{text}"),
                Change::Insert(text) => format!("+{text}"),
            })
            .collect()
    }

    /// Applying the deletions and insertions to `old` must reproduce `new` exactly.
    fn counts(changes: &[Change]) -> (usize, usize) {
        changes
            .iter()
            .fold((0, 0), |(added, removed), change| match change {
                Change::Insert(_) => (added + 1, removed),
                Change::Delete(_) => (added, removed + 1),
                Change::Equal(_) => (added, removed),
            })
    }

    fn reconstructs(old: &str, new: &str) {
        let changes = diff_lines(old, new);
        let rebuilt: Vec<&str> = changes
            .iter()
            .filter(|change| !matches!(change, Change::Delete(_)))
            .map(text)
            .collect();
        let original: Vec<&str> = changes
            .iter()
            .filter(|change| !matches!(change, Change::Insert(_)))
            .map(text)
            .collect();
        assert_eq!(rebuilt, split_lines(new), "insertions do not rebuild new");
        assert_eq!(original, split_lines(old), "deletions do not rebuild old");
    }

    #[test]
    fn identical_text_produces_only_context() {
        let changes = diff_lines("one\ntwo\n", "one\ntwo\n");
        assert!(changes
            .iter()
            .all(|change| matches!(change, Change::Equal(_))));
        assert_eq!(counts(&changes), (0, 0));
    }

    #[test]
    fn a_replaced_line_is_a_delete_then_an_insert() {
        let changes = diff_lines("one\ntwo\nthree\n", "one\nTWO\nthree\n");
        assert_eq!(rendered(&changes), vec![" one", "-two", "+TWO", " three"]);
        assert_eq!(counts(&changes), (1, 1));
    }

    #[test]
    fn an_insertion_keeps_the_surrounding_lines() {
        let changes = diff_lines("one\nthree\n", "one\ntwo\nthree\n");
        assert_eq!(rendered(&changes), vec![" one", "+two", " three"]);
    }

    #[test]
    fn a_deletion_keeps_the_surrounding_lines() {
        let changes = diff_lines("one\ntwo\nthree\n", "one\nthree\n");
        assert_eq!(rendered(&changes), vec![" one", "-two", " three"]);
    }

    #[test]
    fn writing_a_new_file_is_all_insertions() {
        let changes = diff_lines("", "one\ntwo\n");
        assert_eq!(rendered(&changes), vec!["+one", "+two"]);
        assert_eq!(counts(&changes), (2, 0));
    }

    #[test]
    fn emptying_a_file_is_all_deletions() {
        let changes = diff_lines("one\ntwo\n", "");
        assert_eq!(counts(&changes), (0, 2));
    }

    #[test]
    fn a_trailing_newline_does_not_add_an_empty_line() {
        assert_eq!(
            diff_lines("one", "one\n"),
            vec![Change::Equal("one".into())]
        );
    }

    #[test]
    fn moved_blocks_are_aligned_rather_than_replaced_wholesale() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "a\nc\nd\nx\ne\nf\n";
        let changes = diff_lines(old, new);
        assert_eq!(counts(&changes), (1, 1));
        reconstructs(old, new);
    }

    #[test]
    fn the_alignment_reconstructs_both_sides() {
        reconstructs("one\ntwo\nthree\nfour\n", "one\nthree\nfour\nfive\n");
        reconstructs("", "");
        reconstructs("same\n", "same\n");
        reconstructs("a\nb\nc\n", "c\nb\na\n");
        reconstructs("x\n", "y\n");
    }

    #[test]
    fn a_change_too_large_to_align_becomes_a_replacement() {
        let old: String = (0..3_000).map(|index| format!("old {index}\n")).collect();
        let new: String = (0..3_000).map(|index| format!("new {index}\n")).collect();
        let changes = diff_lines(&old, &new);
        assert_eq!(counts(&changes), (3_000, 3_000));
    }

    fn laid_out(old: &str, new: &str, context: usize) -> Vec<String> {
        let width = number_width(old, new);
        format(old, new, context)
            .iter()
            .map(|line| gutter(line, width))
            .collect()
    }

    fn numbered(count: usize, from: usize) -> String {
        (from..from + count)
            .map(|n| format!("line {n}\n"))
            .collect()
    }

    #[test]
    fn the_gutter_carries_a_marker_and_the_right_file_s_line_number() {
        let old = "one\ntwo\nthree\n";
        let new = "one\nTWO\nthree\n";
        assert_eq!(
            laid_out(old, new, 4),
            vec![" 1 one", "-2 two", "+2 TWO", " 3 three"]
        );
    }

    #[test]
    fn line_numbers_are_padded_to_the_widest_in_the_file() {
        let old = numbered(12, 1);
        let new = old.replace("line 12", "line twelve");
        let lines = laid_out(&old, &new, 1);
        
        assert!(lines.iter().any(|line| line == " 11 line 11"));
        assert!(lines.iter().any(|line| line == "-12 line 12"));
        assert!(lines.iter().any(|line| line == "+12 line twelve"));
    }

    #[test]
    fn context_is_trimmed_to_the_lines_either_side_of_a_change() {
        let old = numbered(20, 1);
        let new = old.replace("line 10", "line ten");
        let lines = laid_out(&old, &new, 2);

        assert_eq!(
            lines,
            vec![
                "    ...",
                "  8 line 8",
                "  9 line 9",
                "-10 line 10",
                "+10 line ten",
                " 11 line 11",
                " 12 line 12",
                "    ...",
            ]
        );
    }

    #[test]
    fn a_short_gap_between_two_changes_is_shown_whole() {
        let old = numbered(8, 1);
        let new = old
            .replace("line 1\n", "LINE 1\n")
            .replace("line 4", "LINE 4");
        let lines = laid_out(&old, &new, 4);

        
        assert!(!lines.iter().any(|line| line.ends_with("...")));
        assert!(lines.iter().any(|line| line == " 2 line 2"));
        assert!(lines.iter().any(|line| line == " 3 line 3"));
    }

    #[test]
    fn a_long_gap_between_two_changes_elides_its_middle() {
        let old = numbered(30, 1);
        let new = old
            .replace("line 2\n", "LINE 2\n")
            .replace("line 25", "LINE 25");
        let lines = laid_out(&old, &new, 2);

        
        assert_eq!(lines.iter().filter(|line| line.ends_with("...")).count(), 2);
        assert!(lines.iter().any(|line| line == "  4 line 4"));
        assert!(lines.iter().any(|line| line == " 23 line 23"));
        assert!(
            !lines.iter().any(|line| line.contains("line 13")),
            "the middle of the gap should be gone"
        );
        
        assert!(lines.contains(&"    ...".to_string()));
    }

    #[test]
    fn an_elision_carries_no_number() {
        let old = numbered(30, 1);
        let new = old.replace("line 15", "LINE 15");
        let elisions: Vec<_> = format(&old, &new, 2)
            .into_iter()
            .filter(|line| line.kind == LineKind::Elision)
            .collect();

        assert!(!elisions.is_empty());
        assert!(elisions.iter().all(|line| line.number.is_none()));
        assert!(elisions.iter().all(|line| line.text == "..."));
    }

    #[test]
    fn numbering_survives_an_elision() {
        let old = numbered(30, 1);
        let new = old.replace("line 25", "LINE 25");
        let lines = laid_out(&old, &new, 2);

        
        assert!(lines.iter().any(|line| line == "-25 line 25"));
        assert!(lines.iter().any(|line| line == "+25 LINE 25"));
    }

    #[test]
    fn a_tab_becomes_three_spaces() {
        let lines = laid_out("\tindented\n", "\tchanged\n", 4);
        assert_eq!(lines[0], "-1    indented");
        assert_eq!(lines[1], "+1    changed");
    }

    #[test]
    fn a_pure_insertion_numbers_against_the_new_file() {
        assert_eq!(laid_out("", "added\n", 4), vec!["+1 added"]);
    }

    #[test]
    fn a_pure_deletion_numbers_against_the_old_file() {
        assert_eq!(laid_out("gone\n", "", 4), vec!["-1 gone"]);
    }

    fn theme() -> crate::theme::Theme {
        crate::theme::Theme::dark()
    }

    fn painted_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn inverted(spans: &[Span<'static>]) -> Vec<String> {
        spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.to_string())
            .collect()
    }

    #[test]
    fn each_kind_of_line_takes_its_own_color() {
        let theme = theme();
        let old = "one\ntwo\nthree\n";
        let new = "one\nTWO\nthree\n";
        let lines = format(old, new, 4);
        let painted = paint(&lines, number_width(old, new), &theme);

        assert_eq!(painted[0][0].style.fg, Some(theme.tool_diff_context));
        assert_eq!(painted[1][0].style.fg, Some(theme.tool_diff_removed));
        assert_eq!(painted[2][0].style.fg, Some(theme.tool_diff_added));
        assert_eq!(painted[3][0].style.fg, Some(theme.tool_diff_context));
    }

    #[test]
    fn an_elision_is_painted_as_context() {
        let theme = theme();
        let old = numbered(30, 1);
        let new = old.replace("line 15", "LINE 15");
        let lines = format(&old, &new, 2);
        let painted = paint(&lines, number_width(&old, &new), &theme);

        let elision = painted
            .iter()
            .find(|spans| painted_text(spans).ends_with("..."))
            .expect("an elision");
        assert_eq!(elision[0].style.fg, Some(theme.tool_diff_context));
    }

    #[test]
    fn a_one_for_one_replacement_marks_only_the_words_that_changed() {
        let theme = theme();
        let old = "let value = compute(a);\n";
        let new = "let value = compute(b);\n";
        let lines = format(old, new, 4);
        let painted = paint(&lines, number_width(old, new), &theme);

        assert_eq!(painted.len(), 2);
        assert_eq!(painted_text(&painted[0]), "-1 let value = compute(a);");
        assert_eq!(painted_text(&painted[1]), "+1 let value = compute(b);");
        
        assert_eq!(inverted(&painted[0]), vec!["compute(a);"]);
        assert_eq!(inverted(&painted[1]), vec!["compute(b);"]);
    }

    #[test]
    fn indentation_ahead_of_a_change_is_not_lit_up() {
        let theme = theme();
        let old = "    alpha\n";
        let new = "    beta\n";
        let lines = format(old, new, 4);
        let painted = paint(&lines, number_width(old, new), &theme);

        assert_eq!(inverted(&painted[0]), vec!["alpha"]);
        assert_eq!(inverted(&painted[1]), vec!["beta"]);
        
        assert_eq!(painted_text(&painted[0]), "-1     alpha");
    }

    #[test]
    fn a_block_rewrite_is_shown_whole_rather_than_word_by_word() {
        let theme = theme();
        let old = "one\ntwo\n";
        let new = "three\nfour\n";
        let lines = format(old, new, 4);
        let painted = paint(&lines, number_width(old, new), &theme);

        assert_eq!(painted.len(), 4);
        
        assert_eq!(painted_text(&painted[0]), "-1 one");
        assert_eq!(painted_text(&painted[1]), "-2 two");
        assert_eq!(painted_text(&painted[2]), "+1 three");
        assert_eq!(painted_text(&painted[3]), "+2 four");
        assert!(painted.iter().all(|spans| inverted(spans).is_empty()));
    }

    #[test]
    fn a_standalone_insertion_is_not_word_diffed() {
        let theme = theme();
        let old = "one\ntwo\n";
        let new = "one\ninserted\ntwo\n";
        let lines = format(old, new, 4);
        let painted = paint(&lines, number_width(old, new), &theme);

        let added = painted
            .iter()
            .find(|spans| painted_text(spans).starts_with('+'))
            .expect("an added line");
        assert!(inverted(added).is_empty());
    }

    #[test]
    fn words_split_on_their_trailing_space() {
        assert_eq!(word_tokens("a b"), vec!["a ", "b"]);
        assert_eq!(word_tokens("  lead"), vec!["  ", "lead"]);
        assert_eq!(word_tokens(""), Vec::<String>::new());
        assert_eq!(word_tokens("one"), vec!["one"]);
    }

    #[test]
    fn painting_an_empty_diff_produces_nothing() {
        assert!(paint(&[], 2, &theme()).is_empty());
        assert!(format("same\n", "same\n", 4).is_empty());
    }
}
