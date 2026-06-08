//! Tool results, interpreted for display.

use crate::diff;
use crate::diff::DiffLine;
use crate::diff::LineKind;
use serde_json::Value;

/// Rows shown for a result the reader has expanded.
const MAX_EXPANDED_ROWS: usize = 400;

const COLLAPSED_DIFF_ROWS: usize = 14;
const COLLAPSED_READ_ROWS: usize = 0;
const COLLAPSED_OUTPUT_ROWS: usize = 6;
const COLLAPSED_LIST_ROWS: usize = 8;
const COLLAPSED_ERROR_ROWS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Plain(String),
    /// One line of a diff, laid out but not yet painted.
    Diff(DiffLine),
    /// A file heading in a search result.
    Path {
        path: String,
        count: Option<usize>,
    },
    /// A matching line beneath its file heading.
    Match {
        line: u32,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatches {
    pub path: String,
    pub lines: Vec<(u32, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Empty,
    Diff {
        lines: Vec<DiffLine>,
        /// Columns the line-number gutter needs.
        number_width: usize,
    },
    Text(Vec<String>),
    Matches(Vec<FileMatches>),
    Paths(Vec<String>),
}

impl Body {}

/// A tool result ready to render: what it acted on, how it went, and what it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolView {
    pub subject: String,
    /// A short outcome shown beside the subject: `+3 -1`, `12 lines`, `exit 2`.
    pub detail: Option<String>,
    pub body: Body,
    /// Rows shown while the result is collapsed.
    pub collapsed_rows: usize,
    /// Trailing remarks the tool appended, kept out of the collapsible body.
    pub notes: Vec<String>,
}

impl ToolView {
    /// The rows to draw, and how many were left out.
    pub fn visible(&self, expanded: bool) -> (Vec<Row>, usize) {
        let mut rows = self.rows(expanded);
        let limit = if expanded {
            MAX_EXPANDED_ROWS
        } else {
            self.collapsed_rows
        };
        let hidden = rows.len().saturating_sub(limit);
        rows.truncate(limit);
        (rows, hidden)
    }

    /// The body as rows.
    fn rows(&self, expanded: bool) -> Vec<Row> {
        match &self.body {
            Body::Empty => Vec::new(),
            Body::Text(lines) => lines.iter().cloned().map(Row::Plain).collect(),
            Body::Paths(paths) => paths
                .iter()
                .map(|path| Row::Path {
                    path: path.clone(),
                    count: None,
                })
                .collect(),
            Body::Diff { lines, .. } => lines.iter().cloned().map(Row::Diff).collect(),
            Body::Matches(files) if expanded => files
                .iter()
                .flat_map(|file| {
                    std::iter::once(Row::Path {
                        path: file.path.clone(),
                        count: Some(file.lines.len()),
                    })
                    .chain(file.lines.iter().map(|(line, text)| Row::Match {
                        line: *line,
                        text: text.clone(),
                    }))
                })
                .collect(),
            Body::Matches(files) => files
                .iter()
                .map(|file| Row::Path {
                    path: file.path.clone(),
                    count: Some(file.lines.len()),
                })
                .collect(),
        }
    }
}

/// Interpret one tool result.
pub fn view(name: &str, arguments: &Value, output: Option<&str>, is_error: bool) -> ToolView {
    let subject = subject(name, arguments);

    let Some(output) = output else {
        return ToolView {
            subject,
            detail: None,
            body: Body::Empty,
            collapsed_rows: 0,
            notes: Vec::new(),
        };
    };

    if is_error {
        return failure(name, subject, output);
    }

    match name {
        "edit" | "multi_edit" | "write" => file_change(name, subject, arguments),
        "read" => read(subject, output),
        "bash" => command(subject, output),
        "grep" => search(subject, arguments, output),
        "find" | "ls" => listing(subject, output, if name == "ls" { "entry" } else { "file" }),
        _ => plain(subject, output),
    }
}

/// A failed call shows why, never the diff it did not apply.
fn failure(name: &str, subject: String, output: &str) -> ToolView {
    let (detail, body) = match (name, output.split_once('\n')) {
        ("bash", Some((first, rest))) if first.starts_with("exit code ") => (
            Some(format!("exit {}", first.trim_start_matches("exit code "))),
            rest,
        ),
        ("bash", Some(("terminated by signal", rest))) => (Some("signalled".to_string()), rest),
        _ => (None, output),
    };

    ToolView {
        subject,
        detail,
        body: Body::Text(text_lines(body)),
        collapsed_rows: COLLAPSED_ERROR_ROWS,
        notes: Vec::new(),
    }
}

/// A file change, diffed from the text the model asked to replace.
fn file_change(name: &str, subject: String, arguments: &Value) -> ToolView {
    let mut lines: Vec<DiffLine> = Vec::new();
    let mut number_width = 1;

    let mut record = |old: &str, new: &str| {
        let formatted = diff::format(old, new, diff::DEFAULT_CONTEXT);
        if formatted.is_empty() {
            return;
        }

        if !lines.is_empty() {
            lines.push(DiffLine {
                kind: LineKind::Elision,
                number: None,
                text: "...".to_string(),
            });
        }
        number_width = number_width.max(diff::number_width(old, new));
        lines.extend(formatted);
    };

    match name {
        "write" => record("", field(arguments, "content")),
        "edit" => record(
            field(arguments, "old_string"),
            field(arguments, "new_string"),
        ),
        "multi_edit" => {
            let edits = arguments.get("edits").and_then(Value::as_array);
            for edit in edits.into_iter().flatten() {
                record(field(edit, "old_string"), field(edit, "new_string"));
            }
        }
        _ => {}
    }

    let count = |kind: LineKind| lines.iter().filter(|line| line.kind == kind).count();
    let detail = change_counts(count(LineKind::Added), count(LineKind::Removed));

    ToolView {
        subject,
        detail: Some(detail),
        body: match lines.is_empty() {
            true => Body::Empty,
            false => Body::Diff {
                lines,
                number_width,
            },
        },
        collapsed_rows: COLLAPSED_DIFF_ROWS,
        notes: Vec::new(),
    }
}

fn read(subject: String, output: &str) -> ToolView {
    if let Some(note) = aside(output) {
        return ToolView {
            subject,
            detail: None,
            body: Body::Empty,
            collapsed_rows: 0,
            notes: vec![note],
        };
    }
    let lines = text_lines(output);
    ToolView {
        subject,
        detail: Some(count_of(lines.len(), "line")),
        body: Body::Text(lines),
        collapsed_rows: COLLAPSED_READ_ROWS,
        notes: Vec::new(),
    }
}

fn command(subject: String, output: &str) -> ToolView {
    let lines = match output.trim() {
        "(no output)" => Vec::new(),
        _ => text_lines(output),
    };
    ToolView {
        subject,
        detail: None,
        body: Body::Text(lines),
        collapsed_rows: COLLAPSED_OUTPUT_ROWS,
        notes: Vec::new(),
    }
}

fn search(subject: String, arguments: &Value, output: &str) -> ToolView {
    if let Some(note) = aside(output) {
        return ToolView {
            subject,
            detail: Some("no matches".to_string()),
            body: Body::Empty,
            collapsed_rows: 0,
            notes: vec![note],
        };
    }

    let (body, notes) = match arguments.get("output_mode").and_then(Value::as_str) {
        Some("files") | Some("count") => {
            let (paths, notes) = split_notes(output);
            (Body::Paths(paths), notes)
        }
        _ => {
            let (files, notes) = group_matches(output);
            (Body::Matches(files), notes)
        }
    };

    let detail = match &body {
        Body::Matches(files) => {
            let matches: usize = files.iter().map(|file| file.lines.len()).sum();
            Some(format!(
                "{} in {}",
                count_of(matches, "match"),
                count_of(files.len(), "file")
            ))
        }
        Body::Paths(paths) => Some(count_of(paths.len(), "file")),
        _ => None,
    };

    ToolView {
        subject,
        detail,
        body,
        collapsed_rows: COLLAPSED_LIST_ROWS,
        notes,
    }
}

fn listing(subject: String, output: &str, noun: &str) -> ToolView {
    if let Some(note) = aside(output) {
        return ToolView {
            subject,
            detail: None,
            body: Body::Empty,
            collapsed_rows: 0,
            notes: vec![note],
        };
    }
    let (paths, notes) = split_notes(output);
    ToolView {
        subject,
        detail: Some(count_of(paths.len(), noun)),
        body: Body::Paths(paths),
        collapsed_rows: COLLAPSED_LIST_ROWS,
        notes,
    }
}

fn plain(subject: String, output: &str) -> ToolView {
    let lines = text_lines(output);
    ToolView {
        subject,
        detail: Some(count_of(lines.len(), "line")),
        body: Body::Text(lines),
        collapsed_rows: COLLAPSED_OUTPUT_ROWS,
        notes: Vec::new(),
    }
}

/// The one argument that says what a call is about.
pub fn subject(name: &str, arguments: &Value) -> String {
    subject_text(name, arguments).trim().to_string()
}

fn subject_text(name: &str, arguments: &Value) -> String {
    match name {
        "read" | "write" | "edit" | "multi_edit" | "ls" => field(arguments, "path").to_string(),
        "bash" => {
            let command = field(arguments, "command");
            match command.split_once('\n') {
                Some((first, _)) => format!("{first} …"),
                None => command.to_string(),
            }
        }
        "grep" | "find" => field(arguments, "pattern").to_string(),
        _ => match arguments {
            Value::Null => String::new(),
            Value::Object(map) if map.is_empty() => String::new(),
            other => other.to_string(),
        },
    }
}

fn aside(output: &str) -> Option<String> {
    let trimmed = output.trim();
    let is_aside = trimmed.starts_with("no matches for ")
        || trimmed.starts_with("no files match ")
        || trimmed.ends_with(" is empty");
    is_aside.then(|| trimmed.to_string())
}

/// Split a listing from the trailing remarks the search tools append after a blank line.
fn split_notes(output: &str) -> (Vec<String>, Vec<String>) {
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('…') {
            notes.push(trimmed.trim_matches('…').trim().to_string());
        } else {
            entries.push(line.to_string());
        }
    }
    (entries, notes)
}

/// Fold `path:line:text` output into one entry per file, keeping the file order.
fn group_matches(output: &str) -> (Vec<FileMatches>, Vec<String>) {
    let mut files: Vec<FileMatches> = Vec::new();
    let mut notes = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.trim_start().starts_with('…') {
            notes.push(
                trimmed
                    .trim_matches(|c: char| c == '…' || c.is_whitespace())
                    .to_string(),
            );
            continue;
        }

        match parse_match(trimmed) {
            Some((path, number, text)) => match files.last_mut() {
                Some(last) if last.path == path => last.lines.push((number, text)),
                _ => files.push(FileMatches {
                    path,
                    lines: vec![(number, text)],
                }),
            },

            None => notes.push(trimmed.to_string()),
        }
    }

    (files, notes)
}

fn parse_match(line: &str) -> Option<(String, u32, String)> {
    let (path, rest) = line.split_once(':')?;
    let (number, text) = rest.split_once(':')?;
    let number = number.parse::<u32>().ok()?;
    Some((path.to_string(), number, text.to_string()))
}

fn field<'a>(arguments: &'a Value, key: &str) -> &'a str {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn text_lines(text: &str) -> Vec<String> {
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('\n').map(str::to_string).collect()
}

fn change_counts(added: usize, removed: usize) -> String {
    match (added, removed) {
        (0, 0) => "no change".to_string(),
        (added, 0) => format!("+{added}"),
        (0, removed) => format!("-{removed}"),
        (added, removed) => format!("+{added} -{removed}"),
    }
}

fn count_of(count: usize, noun: &str) -> String {
    if count == 1 {
        return format!("1 {noun}");
    }
    let plural = match noun {
        "entry" => "entries".to_string(),
        "match" => "matches".to_string(),
        other => format!("{other}s"),
    };
    format!("{count} {plural}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rows_of(view: &ToolView, expanded: bool) -> Vec<Row> {
        view.visible(expanded).0
    }

    fn gutters(view: &ToolView, expanded: bool) -> Vec<String> {
        let number_width = match &view.body {
            Body::Diff { number_width, .. } => *number_width,
            _ => 0,
        };
        rows_of(view, expanded)
            .iter()
            .filter_map(|row| match row {
                Row::Diff(line) => Some(diff::gutter(line, number_width)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_running_call_has_no_body() {
        let view = view("read", &json!({ "path": "a.rs" }), None, false);
        assert_eq!(view.subject, "a.rs");
        assert_eq!(view.body, Body::Empty);
        assert_eq!(view.detail, None);
    }

    #[test]
    fn an_edit_becomes_a_diff_of_its_arguments() {
        let view = view(
            "edit",
            &json!({
                "path": "src/main.rs",
                "old_string": "fn main() {\n    old();\n}",
                "new_string": "fn main() {\n    new();\n}",
            }),
            Some("Edited src/main.rs"),
            false,
        );

        assert_eq!(view.subject, "src/main.rs");
        assert_eq!(view.detail.as_deref(), Some("+1 -1"));

        assert_eq!(
            gutters(&view, true),
            vec![" 1 fn main() {", "-2     old();", "+2     new();", " 3 }"]
        );
    }

    #[test]
    fn a_multi_edit_shows_each_edit_as_its_own_block() {
        let view = view(
            "multi_edit",
            &json!({
                "path": "a.rs",
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" },
                    { "old_string": "omega", "new_string": "OMEGA" },
                ],
            }),
            Some("Edited a.rs (2 edits)"),
            false,
        );

        assert_eq!(view.detail.as_deref(), Some("+2 -2"));

        assert_eq!(
            gutters(&view, true),
            vec!["-1 alpha", "+1 ALPHA", "   ...", "-1 omega", "+1 OMEGA"]
        );
    }

    #[test]
    fn a_write_is_shown_as_an_addition() {
        let view = view(
            "write",
            &json!({ "path": "new.rs", "content": "one\ntwo\n" }),
            Some("Wrote new.rs (8 bytes)"),
            false,
        );
        assert_eq!(view.detail.as_deref(), Some("+2"));
        assert_eq!(gutters(&view, true), vec!["+1 one", "+2 two"]);
    }

    #[test]
    fn a_read_collapses_to_its_header() {
        let output = "     1\tone\n     2\ttwo\n     3\tthree\n";
        let view = view("read", &json!({ "path": "a.rs" }), Some(output), false);

        assert_eq!(view.detail.as_deref(), Some("3 lines"));
        let (rows, hidden) = view.visible(false);
        assert!(rows.is_empty(), "a collapsed read shows nothing");
        assert_eq!(hidden, 3);
        assert_eq!(view.visible(true).0.len(), 3);
    }

    #[test]
    fn an_empty_file_reads_as_a_note() {
        let view = view(
            "read",
            &json!({ "path": "a.rs" }),
            Some("/tmp/a.rs is empty"),
            false,
        );
        assert_eq!(view.notes, vec!["/tmp/a.rs is empty"]);
        assert_eq!(view.body, Body::Empty);
    }

    #[test]
    fn a_command_keeps_its_output_and_collapses_the_tail() {
        let output = (1..=20)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let view = view(
            "bash",
            &json!({ "command": "cargo test" }),
            Some(&output),
            false,
        );

        assert_eq!(view.subject, "cargo test");
        assert_eq!(view.detail, None);
        let (rows, hidden) = view.visible(false);
        assert_eq!(rows.len(), COLLAPSED_OUTPUT_ROWS);
        assert_eq!(hidden, 20 - COLLAPSED_OUTPUT_ROWS);
    }

    #[test]
    fn a_failed_command_shows_its_exit_status() {
        let view = view(
            "bash",
            &json!({ "command": "cargo test" }),
            Some("exit code 101\nthread 'main' panicked\nnote: run with RUST_BACKTRACE=1"),
            true,
        );
        assert_eq!(view.detail.as_deref(), Some("exit 101"));
        assert_eq!(
            rows_of(&view, false),
            vec![
                Row::Plain("thread 'main' panicked".into()),
                Row::Plain("note: run with RUST_BACKTRACE=1".into()),
            ]
        );
    }

    #[test]
    fn a_multi_line_command_is_summarized_by_its_first_line() {
        let view = view(
            "bash",
            &json!({ "command": "cd src\nls -la" }),
            Some("ok"),
            false,
        );
        assert_eq!(view.subject, "cd src …");
    }

    #[test]
    fn grep_groups_matches_under_their_files() {
        let output = "src/a.rs:12:fn one()\nsrc/a.rs:40:fn two()\nsrc/b.rs:3:fn three()";
        let view = view("grep", &json!({ "pattern": "fn " }), Some(output), false);

        assert_eq!(view.detail.as_deref(), Some("3 matches in 2 files"));
        assert_eq!(
            rows_of(&view, false),
            vec![
                Row::Path {
                    path: "src/a.rs".into(),
                    count: Some(2)
                },
                Row::Path {
                    path: "src/b.rs".into(),
                    count: Some(1)
                },
            ]
        );

        let expanded = rows_of(&view, true);
        assert_eq!(expanded.len(), 5, "two headings and three matches");
        assert_eq!(
            expanded[1],
            Row::Match {
                line: 12,
                text: "fn one()".into()
            }
        );
    }

    #[test]
    fn a_grep_cap_becomes_a_note_rather_than_a_match() {
        let output = "src/a.rs:1:hit\n\n… stopped at 200 results; narrow the pattern …";
        let view = view("grep", &json!({ "pattern": "x" }), Some(output), false);
        assert_eq!(rows_of(&view, false).len(), 1);
        assert_eq!(view.notes.len(), 1);
        assert!(view.notes[0].contains("stopped at 200 results"));
    }

    #[test]
    fn grep_in_files_mode_is_a_path_list() {
        let view = view(
            "grep",
            &json!({ "pattern": "x", "output_mode": "files" }),
            Some("src/a.rs\nsrc/b.rs"),
            false,
        );
        assert_eq!(view.detail.as_deref(), Some("2 files"));
        assert_eq!(
            rows_of(&view, false),
            vec![
                Row::Path {
                    path: "src/a.rs".into(),
                    count: None
                },
                Row::Path {
                    path: "src/b.rs".into(),
                    count: None
                },
            ]
        );
    }

    #[test]
    fn an_empty_search_says_so_without_a_body() {
        let view = view(
            "grep",
            &json!({ "pattern": "zzz" }),
            Some("no matches for zzz"),
            false,
        );
        assert_eq!(view.detail.as_deref(), Some("no matches"));
        assert!(rows_of(&view, true).is_empty());
        assert_eq!(view.notes, vec!["no matches for zzz"]);
    }

    #[test]
    fn find_lists_paths_and_keeps_its_cap_as_a_note() {
        let view = view(
            "find",
            &json!({ "pattern": "**/*.rs" }),
            Some("a.rs\nb.rs\n\n… 12 more files match; raise limit …"),
            false,
        );
        assert_eq!(view.detail.as_deref(), Some("2 files"));
        assert_eq!(rows_of(&view, false).len(), 2);
        assert!(view.notes[0].contains("12 more files"));
    }

    #[test]
    fn a_long_list_collapses_with_a_hidden_count() {
        let output = (0..30)
            .map(|index| format!("file{index}.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        let view = view("find", &json!({ "pattern": "*" }), Some(&output), false);
        let (rows, hidden) = view.visible(false);
        assert_eq!(rows.len(), COLLAPSED_LIST_ROWS);
        assert_eq!(hidden, 30 - COLLAPSED_LIST_ROWS);
        assert_eq!(view.visible(true).1, 0);
    }

    #[test]
    fn an_unknown_tool_shows_its_output_as_text() {
        let view = view("mystery", &json!({ "a": 1 }), Some("some output"), false);
        assert_eq!(view.subject, r#"{"a":1}"#);
        assert_eq!(
            rows_of(&view, false),
            vec![Row::Plain("some output".into())]
        );
    }

    #[test]
    fn an_expanded_body_is_still_capped() {
        let output = (0..1_000)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let view = view("bash", &json!({ "command": "yes" }), Some(&output), false);
        let (rows, hidden) = view.visible(true);
        assert_eq!(rows.len(), MAX_EXPANDED_ROWS);
        assert_eq!(hidden, 1_000 - MAX_EXPANDED_ROWS);
    }
}
