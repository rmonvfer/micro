//! Codebase search: file contents by regex, file names by glob.

use crate::required_str;
use crate::resolve_path;
use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use ignore::overrides::OverrideBuilder;
use ignore::Walk;
use ignore::WalkBuilder;
use micro_types::ToolDefinition;
use regex::Regex;
use regex::RegexBuilder;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Files past this size are skipped. Searching one costs more memory than the match is
/// worth, and files this large are generated rather than written.
const MAX_SEARCHED_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// How much of a file is inspected for the NUL byte that marks it binary.
const BINARY_SNIFF_BYTES: usize = 8192;

/// Matches reported before a search stops early.
const MAX_MATCHES: usize = 500;

const DEFAULT_FIND_LIMIT: usize = 100;
const MAX_FIND_LIMIT: usize = 1000;

pub struct Grep {
    root: PathBuf,
}

impl Grep {
    pub fn new(root: PathBuf) -> Self {
        Grep { root }
    }
}

#[async_trait]
impl Tool for Grep {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".into(),
            description: "Search file contents with a regular expression. Respects .gitignore \
                          and skips binary files."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression to search for" },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search, defaults to the workspace root",
                    },
                    "glob": {
                        "type": "string",
                        "description": "Only search files matching this glob, for example *.rs",
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Match regardless of case, default false",
                    },
                    "literal": {
                        "type": "boolean",
                        "description": "Treat pattern as a literal string instead of a regex \
                                        (default: false)",
                    },
                    "context": {
                        "type": "number",
                        "description": "Number of lines to show before and after each match \
                                        (default: 0)",
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files", "count"],
                        "description": "content returns path:line:text, files returns matching \
                                        paths, count returns path:matches. Default content",
                    },
                },
                "required": ["pattern"],
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let pattern = required_str(arguments, "pattern")?;
        let requested = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let case_insensitive = arguments
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = match arguments.get("output_mode").and_then(Value::as_str) {
            Some(raw) => OutputMode::parse(raw)?,
            None => OutputMode::Content,
        };

        let literal = arguments
            .get("literal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let context = arguments
            .get("context")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;

        // A literal pattern is escaped rather than compiled as written, so a dot means a
        // dot and a bracket means a bracket.
        let expression = match literal {
            true => regex::escape(&pattern),
            false => pattern.clone(),
        };

        let search = Search {
            root: self.root.clone(),
            start: resolve_path(&self.root, requested)?,
            regex: RegexBuilder::new(&expression)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|error| format!("invalid pattern {pattern}: {error}"))?,
            context,
            glob: arguments
                .get("glob")
                .and_then(Value::as_str)
                .map(str::to_string),
            pattern,
            mode,
        };

        // Walking a tree and reading every file blocks; keep it off the async runtime.
        tokio::task::spawn_blocking(move || search.run())
            .await
            .map_err(|error| format!("search failed: {error}"))?
    }
}

pub struct Find {
    root: PathBuf,
}

impl Find {
    pub fn new(root: PathBuf) -> Self {
        Find { root }
    }
}

#[async_trait]
impl Tool for Find {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".into(),
            description: "Find files by glob pattern, most recently modified first. Respects \
                          .gitignore."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob to match against the path, for example **/*.rs",
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search, defaults to the workspace root",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "How many paths to return, default 100, max 1000",
                    },
                },
                "required": ["pattern"],
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let pattern = required_str(arguments, "pattern")?;
        let requested = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_FIND_LIMIT)
            .clamp(1, MAX_FIND_LIMIT);

        let lookup = Lookup {
            root: self.root.clone(),
            start: resolve_path(&self.root, requested)?,
            pattern,
            limit,
        };

        tokio::task::spawn_blocking(move || lookup.run())
            .await
            .map_err(|error| format!("find failed: {error}"))?
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Content,
    Files,
    Count,
}

impl OutputMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "content" => Ok(OutputMode::Content),
            "files" => Ok(OutputMode::Files),
            "count" => Ok(OutputMode::Count),
            other => Err(format!(
                "unknown output_mode {other}; expected content, files or count"
            )),
        }
    }
}

struct Search {
    root: PathBuf,
    start: PathBuf,
    pattern: String,
    regex: Regex,
    glob: Option<String>,
    mode: OutputMode,
    /// Lines to show either side of a match. Zero shows only the matching line.
    context: usize,
}

impl Search {
    fn run(&self) -> Result<String, String> {
        let mut hits: Vec<String> = Vec::new();
        let mut capped = false;

        for entry in walker(&self.start, self.glob.as_deref())? {
            // An unreadable directory is worth stepping over, not failing the search.
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() > MAX_SEARCHED_FILE_BYTES)
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            if is_binary(&bytes) {
                continue;
            }

            let relative = relative_to(&self.root, entry.path());
            let text = String::from_utf8_lossy(&bytes);
            let mut file_matches = 0usize;

            let lines: Vec<&str> = text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if !self.regex.is_match(line) {
                    continue;
                }
                file_matches += 1;
                match self.mode {
                    OutputMode::Content => {
                        let first = index.saturating_sub(self.context);
                        let last = (index + self.context).min(lines.len().saturating_sub(1));
                        for around in first..=last {
                            // A match is separated from its surroundings by the marker, the
                            // way a context-carrying search has always shown it.
                            let marker = match around == index {
                                true => ':',
                                false => '-',
                            };
                            hits.push(format!(
                                "{relative}{marker}{}{marker}{}",
                                around + 1,
                                lines[around].trim_end()
                            ));
                        }
                        if hits.len() >= MAX_MATCHES {
                            capped = true;
                            break;
                        }
                    }
                    // The listing modes only need to know that the file matched at all.
                    OutputMode::Files => break,
                    OutputMode::Count => {}
                }
            }

            if file_matches > 0 {
                match self.mode {
                    OutputMode::Content => {}
                    OutputMode::Files => hits.push(relative),
                    OutputMode::Count => hits.push(format!("{relative}:{file_matches}")),
                }
            }
            if capped || hits.len() >= MAX_MATCHES {
                capped = true;
                break;
            }
        }

        if hits.is_empty() {
            return Ok(format!("no matches for {}", self.pattern));
        }

        let mut output = hits.join("\n");
        if capped {
            output.push_str(&format!(
                "\n\n… stopped at {MAX_MATCHES} results; narrow the pattern or scope the path …"
            ));
        }
        Ok(truncate(&output))
    }
}

struct Lookup {
    root: PathBuf,
    start: PathBuf,
    pattern: String,
    limit: usize,
}

impl Lookup {
    fn run(&self) -> Result<String, String> {
        let mut found: Vec<(SystemTime, String)> = Vec::new();

        for entry in walker(&self.start, Some(&self.pattern))? {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            found.push((modified, relative_to(&self.root, entry.path())));
        }

        if found.is_empty() {
            return Ok(format!("no files match {}", self.pattern));
        }

        // Path breaks ties so two files written in the same instant keep a stable order.
        found.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

        let total = found.len();
        found.truncate(self.limit);
        let mut output = found
            .into_iter()
            .map(|(_, path)| path)
            .collect::<Vec<_>>()
            .join("\n");
        if total > self.limit {
            output.push_str(&format!(
                "\n\n… {} more files match; raise limit or narrow the pattern …",
                total - self.limit
            ));
        }
        Ok(truncate(&output))
    }
}

/// A walker that honours the workspace's ignore files.
///
/// Global git configuration is left out so a search depends only on the workspace rather
/// than on the machine, and `require_git` is off so a `.gitignore` still counts in a
/// directory that is not a checkout. Hidden files are searched: a dotfile is where a
/// project keeps its configuration, and leaving it out means answering a question about
/// the workspace with only part of it while appearing to have read all of it.
fn walker(start: &Path, glob: Option<&str>) -> Result<Walk, String> {
    let mut builder = WalkBuilder::new(start);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .sort_by_file_path(|left, right| left.cmp(right));

    if let Some(glob) = glob {
        let mut overrides = OverrideBuilder::new(start);
        overrides
            .add(glob)
            .map_err(|error| format!("invalid glob {glob}: {error}"))?;
        builder.overrides(
            overrides
                .build()
                .map_err(|error| format!("invalid glob {glob}: {error}"))?,
        );
    }

    Ok(builder.build())
}

/// A NUL byte among the leading bytes is how a binary file gives itself away. Searching
/// one produces noise rather than information.
fn is_binary(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .take(BINARY_SNIFF_BYTES)
        .any(|byte| *byte == b'\0')
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-search-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn content_mode_prefixes_every_match_with_its_path_and_line() {
        let root = scratch("content");
        write(&root, "src/main.rs", "fn main() {}\nlet needle = 1;\n");
        write(&root, "src/other.rs", "nothing here\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle" }))
            .await
            .unwrap();
        assert_eq!(output, "src/main.rs:2:let needle = 1;");
    }

    #[tokio::test]
    async fn files_mode_lists_each_matching_path_once() {
        let root = scratch("files-mode");
        write(&root, "a.rs", "needle\nneedle\nneedle\n");
        write(&root, "b.rs", "needle\n");
        write(&root, "c.rs", "nothing\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(output, "a.rs\nb.rs");
    }

    #[tokio::test]
    async fn count_mode_reports_matches_per_file() {
        let root = scratch("count-mode");
        write(&root, "a.rs", "needle\nneedle\n");
        write(&root, "b.rs", "needle\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "count" }))
            .await
            .unwrap();
        assert_eq!(output, "a.rs:2\nb.rs:1");
    }

    #[tokio::test]
    async fn a_gitignored_file_is_not_searched() {
        let root = scratch("grep-gitignore");
        write(&root, ".gitignore", "target/\nsecrets.txt\n");
        write(&root, "target/build.rs", "needle in generated output\n");
        write(&root, "secrets.txt", "needle in a secret\n");
        write(&root, "src/keep.rs", "needle in tracked source\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(output, "src/keep.rs");
    }

    #[tokio::test]
    async fn a_binary_file_is_skipped() {
        let root = scratch("grep-binary");
        let mut binary = b"needle".to_vec();
        binary.push(0);
        binary.extend_from_slice(b"needle again");
        write(&root, "blob.bin", binary);
        write(&root, "notes.txt", "needle in text\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(output, "notes.txt");
    }

    #[tokio::test]
    async fn a_glob_filters_which_files_are_searched() {
        let root = scratch("grep-glob");
        write(&root, "src/lib.rs", "needle\n");
        write(&root, "README.md", "needle\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "glob": "*.rs", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(output, "src/lib.rs");
    }

    #[tokio::test]
    async fn a_path_scopes_the_search_to_one_subtree() {
        let root = scratch("grep-scope");
        write(&root, "src/lib.rs", "needle\n");
        write(&root, "docs/guide.md", "needle\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "path": "docs", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(output, "docs/guide.md");
    }

    #[tokio::test]
    async fn case_insensitivity_is_opt_in() {
        let root = scratch("grep-case");
        write(&root, "a.rs", "Needle\n");
        let tool = Grep::new(root);

        let sensitive = tool
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();
        assert_eq!(sensitive, "no matches for needle");

        let insensitive = tool
            .execute(&json!({
                "pattern": "needle",
                "case_insensitive": true,
                "output_mode": "files",
            }))
            .await
            .unwrap();
        assert_eq!(insensitive, "a.rs");
    }

    #[tokio::test]
    async fn a_regular_expression_matches_more_than_a_literal() {
        let root = scratch("grep-regex");
        write(
            &root,
            "a.rs",
            "fn alpha() {}\nfn beta() {}\nstruct Gamma;\n",
        );

        let output = Grep::new(root)
            .execute(&json!({ "pattern": r"^fn (\w+)" }))
            .await
            .unwrap();
        assert_eq!(output, "a.rs:1:fn alpha() {}\na.rs:2:fn beta() {}");
    }

    #[tokio::test]
    async fn an_unparseable_pattern_is_reported_rather_than_searched() {
        let root = scratch("grep-bad-pattern");
        let error = Grep::new(root)
            .execute(&json!({ "pattern": "unclosed(" }))
            .await
            .unwrap_err();
        assert!(error.contains("invalid pattern"));
    }

    #[tokio::test]
    async fn an_unknown_output_mode_is_rejected() {
        let root = scratch("grep-bad-mode");
        let error = Grep::new(root)
            .execute(&json!({ "pattern": "x", "output_mode": "json" }))
            .await
            .unwrap_err();
        assert!(error.contains("unknown output_mode"));
    }

    #[tokio::test]
    async fn searching_outside_the_workspace_is_refused() {
        let root = scratch("grep-escape");
        let error = Grep::new(root)
            .execute(&json!({ "pattern": "x", "path": "../.." }))
            .await
            .unwrap_err();
        assert!(error.contains("escapes the workspace"));
    }

    #[tokio::test]
    async fn a_capped_search_says_it_stopped_early() {
        let root = scratch("grep-cap");
        let many = "needle\n".repeat(MAX_MATCHES + 50);
        write(&root, "a.rs", many);

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle" }))
            .await
            .unwrap();
        assert!(output.contains("stopped at"));
        assert_eq!(
            output.lines().filter(|line| line.contains(":1:")).count(),
            1
        );
    }

    #[tokio::test]
    async fn find_returns_paths_matching_the_glob() {
        let root = scratch("find-glob");
        write(&root, "src/main.rs", "");
        write(&root, "src/lib.rs", "");
        write(&root, "README.md", "");

        let output = Find::new(root)
            .execute(&json!({ "pattern": "**/*.rs" }))
            .await
            .unwrap();
        let mut paths: Vec<&str> = output.lines().collect();
        paths.sort();
        assert_eq!(paths, vec!["src/lib.rs", "src/main.rs"]);
    }

    #[tokio::test]
    async fn find_orders_by_modification_time_newest_first() {
        let root = scratch("find-order");
        write(&root, "old.rs", "");
        write(&root, "middle.rs", "");
        write(&root, "new.rs", "");

        let base = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        for (name, offset) in [("old.rs", 0), ("middle.rs", 60), ("new.rs", 120)] {
            let file = std::fs::File::options()
                .write(true)
                .open(root.join(name))
                .unwrap();
            file.set_modified(base + std::time::Duration::from_secs(offset))
                .unwrap();
        }

        let output = Find::new(root)
            .execute(&json!({ "pattern": "*.rs" }))
            .await
            .unwrap();
        assert_eq!(output, "new.rs\nmiddle.rs\nold.rs");
    }

    #[tokio::test]
    async fn find_skips_gitignored_files() {
        let root = scratch("find-gitignore");
        write(&root, ".gitignore", "target/\n");
        write(&root, "target/generated.rs", "");
        write(&root, "src/main.rs", "");

        let output = Find::new(root)
            .execute(&json!({ "pattern": "**/*.rs" }))
            .await
            .unwrap();
        assert_eq!(output, "src/main.rs");
    }

    #[tokio::test]
    async fn find_caps_results_and_says_how_many_were_left_out() {
        let root = scratch("find-limit");
        for index in 0..10 {
            write(&root, &format!("file{index}.rs"), "");
        }

        let output = Find::new(root)
            .execute(&json!({ "pattern": "*.rs", "limit": 3 }))
            .await
            .unwrap();
        assert_eq!(
            output.lines().filter(|line| line.ends_with(".rs")).count(),
            3
        );
        assert!(output.contains("7 more files match"));
    }

    #[tokio::test]
    async fn find_reports_an_empty_result_rather_than_failing() {
        let root = scratch("find-empty");
        write(&root, "README.md", "");

        let output = Find::new(root)
            .execute(&json!({ "pattern": "**/*.rs" }))
            .await
            .unwrap();
        assert_eq!(output, "no files match **/*.rs");
    }
}

#[cfg(test)]
mod hidden {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-hidden-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// A dotfile is part of the workspace, and a search that skipped it would answer with
    /// only part of the project while looking like it had read all of it.
    #[tokio::test]
    async fn grep_reads_hidden_files() {
        let root = scratch("grep");
        write(&root, ".env.example", "TOKEN=needle\n");
        write(&root, ".config/settings.toml", "key = \"needle\"\n");
        write(&root, "visible.txt", "needle\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();

        assert!(output.contains(".env.example"), "{output}");
        assert!(output.contains("settings.toml"), "{output}");
        assert!(output.contains("visible.txt"), "{output}");
    }

    #[tokio::test]
    async fn find_lists_hidden_files() {
        let root = scratch("find");
        write(&root, ".gitignore", "target\n");
        write(&root, "README.md", "hello\n");

        let output = Find::new(root)
            .execute(&json!({ "pattern": "**/*" }))
            .await
            .unwrap();

        assert!(output.contains(".gitignore"), "{output}");
        assert!(output.contains("README.md"), "{output}");
    }

    /// What a workspace ignores is still ignored: reading hidden files is not the same as
    /// ignoring `.gitignore`.
    #[tokio::test]
    async fn an_ignored_path_stays_ignored() {
        let root = scratch("ignored");
        write(&root, ".gitignore", "secret.txt\n");
        write(&root, "secret.txt", "needle\n");
        write(&root, "kept.txt", "needle\n");

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "output_mode": "files" }))
            .await
            .unwrap();

        assert!(output.contains("kept.txt"), "{output}");
        assert!(!output.contains("secret.txt"), "{output}");
    }
}

#[cfg(test)]
mod literal_and_context {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-grep-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A literal pattern means what it says: a dot is a dot, not any character.
    #[tokio::test]
    async fn a_literal_pattern_is_not_a_regex() {
        let root = scratch("literal");
        std::fs::write(root.join("a.txt"), "a.b\naxb\n").unwrap();

        let regex = Grep::new(root.clone())
            .execute(&json!({ "pattern": "a.b" }))
            .await
            .unwrap();
        assert!(
            regex.contains("axb"),
            "as a regex the dot matches x: {regex}"
        );

        let literal = Grep::new(root)
            .execute(&json!({ "pattern": "a.b", "literal": true }))
            .await
            .unwrap();
        assert!(literal.contains("a.b"), "{literal}");
        assert!(
            !literal.contains("axb"),
            "literally, it does not: {literal}"
        );
    }

    /// Context shows the lines around a match, marked apart from it.
    #[tokio::test]
    async fn context_shows_the_lines_around_a_match() {
        let root = scratch("context");
        std::fs::write(root.join("a.txt"), "one\ntwo\nneedle\nfour\nfive\n").unwrap();

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle", "context": 1 }))
            .await
            .unwrap();

        assert!(output.contains("a.txt-2-two"), "{output}");
        assert!(output.contains("a.txt:3:needle"), "{output}");
        assert!(output.contains("a.txt-4-four"), "{output}");
        assert!(
            !output.contains("one"),
            "only one line either side: {output}"
        );
    }

    /// Without context, only the matching line is shown, as before.
    #[tokio::test]
    async fn no_context_shows_only_the_match() {
        let root = scratch("nocontext");
        std::fs::write(root.join("a.txt"), "one\nneedle\nthree\n").unwrap();

        let output = Grep::new(root)
            .execute(&json!({ "pattern": "needle" }))
            .await
            .unwrap();
        assert_eq!(output, "a.txt:2:needle");
    }
}
