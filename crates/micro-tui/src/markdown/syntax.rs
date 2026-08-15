//! Syntax highlighting for fenced code blocks.
//!
//! Scopes are named as highlight.js names them — `keyword`, `built_in`, `title`, and the
//! rest — and each one lands on a theme color. The lexing is hand-written rather than pulled
//! from a grammar library, because the Rust equivalents of highlight.js cost megabytes of
//! binary for what is a cosmetic feature. What that trades away is breadth: this knows the
//! handful of languages that actually turn up in a coding agent's output rather than every
//! language a grammar library ships.
//!
//! Two rules hold whatever the language:
//!
//! Text is never changed. Every byte of a line ends up in exactly one token, in order, so
//! the text recovered from the tokens is the line that went in. A language that is not
//! recognized produces no tokens at all and the caller renders the line as it always did.
//!
//! Lexing is line at a time, with only the state a block comment or a triple-quoted string
//! needs carried across. That keeps a half-streamed response legible, which is the whole
//! reason the interface renders markdown a line at a time in the first place.

use crate::theme::Theme;
use ratatui::style::Style;

/// A highlight scope, named as highlight.js names it.
///
/// Only the scopes this lexer emits are listed. Several wider highlight.js scopes collapse
/// onto these: `built_in` and `class` join `Type`, `title` joins `Function`, `attr` and
/// `params` join `Variable`, and `literal` joins `Number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Comment,
    Keyword,
    Function,
    Variable,
    String,
    Number,
    Type,
    Operator,
    Punctuation,
    /// Attributes, decorators, preprocessor lines. These are painted `muted` rather than
    /// with a syntax color of their own.
    Meta,
}

impl Scope {
    /// The style this scope is painted in.
    pub fn style(self, theme: &Theme) -> Style {
        let color = match self {
            Scope::Comment => theme.syntax_comment,
            Scope::Keyword => theme.syntax_keyword,
            Scope::Function => theme.syntax_function,
            Scope::Variable => theme.syntax_variable,
            Scope::String => theme.syntax_string,
            Scope::Number => theme.syntax_number,
            Scope::Type => theme.syntax_type,
            Scope::Operator => theme.syntax_operator,
            Scope::Punctuation => theme.syntax_punctuation,
            Scope::Meta => theme.muted,
        };
        Style::new().fg(color)
    }
}

/// A run of characters that share a scope. `scope` is `None` for text that carries none,
/// which the caller paints with the code block's own color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub scope: Option<Scope>,
    pub text: String,
}

/// What a language looks like, as far as this lexer cares.
struct Syntax {
    names: &'static [&'static str],
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that open a string ending at the matching quote.
    quotes: &'static [char],
    /// Whether a run of three quotes opens a string that spans lines, as Python's does.
    triple_quotes: bool,
    /// Whether a backslash inside a string escapes the next character.
    escapes: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    /// Characters that start a variable, such as shell's `$`.
    sigils: &'static [char],
    /// Characters that begin a line-level annotation, such as Rust's `#[` or a decorator.
    meta: &'static [&'static str],
}

const RUST: Syntax = Syntax {
    names: &["rust", "rs"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    triple_quotes: false,
    escapes: true,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "union", "unsafe", "use", "where", "while",
    ],
    types: &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
        "u16", "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box", "Rc",
        "Arc", "HashMap", "HashSet", "Some", "None", "Ok", "Err",
    ],
    sigils: &[],
    meta: &["#[", "#!["],
};

const TYPESCRIPT: Syntax = Syntax {
    names: &[
        "typescript",
        "ts",
        "tsx",
        "javascript",
        "js",
        "jsx",
        "mjs",
        "cjs",
    ],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\'', '`'],
    triple_quotes: false,
    escapes: true,
    keywords: &[
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "get",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "of",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "set",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "yield",
    ],
    types: &[
        "Array", "Boolean", "Date", "Error", "Function", "JSON", "Map", "Math", "Number", "Object",
        "Promise", "RegExp", "Set", "String", "Symbol", "any", "boolean", "never", "number",
        "object", "string", "unknown",
    ],
    sigils: &[],
    meta: &["@"],
};

const PYTHON: Syntax = Syntax {
    names: &["python", "py"],
    line_comment: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    triple_quotes: true,
    escapes: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
        "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
        "try", "while", "with", "yield",
    ],
    types: &[
        "bool",
        "bytes",
        "dict",
        "float",
        "frozenset",
        "int",
        "list",
        "set",
        "str",
        "tuple",
        "object",
        "type",
    ],
    sigils: &[],
    meta: &["@"],
};

const JSON: Syntax = Syntax {
    names: &["json", "jsonc"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"'],
    triple_quotes: false,
    escapes: true,
    keywords: &["true", "false", "null"],
    types: &[],
    sigils: &[],
    meta: &[],
};

const TOML: Syntax = Syntax {
    names: &["toml"],
    line_comment: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    triple_quotes: false,
    escapes: true,
    keywords: &["true", "false"],
    types: &[],
    sigils: &[],
    meta: &[],
};

const SHELL: Syntax = Syntax {
    names: &["bash", "sh", "shell", "zsh", "console"],
    line_comment: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    triple_quotes: false,
    escapes: true,
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
        "in", "local", "readonly", "return", "select", "then", "until", "while",
    ],
    types: &[
        "cat", "cd", "echo", "exit", "grep", "kill", "ls", "mkdir", "mv", "printf", "pwd", "read",
        "rm", "sed", "set", "source", "test", "unset",
    ],
    sigils: &['$'],
    meta: &["#!"],
};

const SYNTAXES: &[&Syntax] = &[&RUST, &TYPESCRIPT, &PYTHON, &JSON, &TOML, &SHELL];

/// Languages this highlights, beyond those the table covers.
const MARKDOWN: &[&str] = &["markdown", "md"];

fn syntax_for(language: &str) -> Option<&'static Syntax> {
    let language = language.trim().to_ascii_lowercase();
    SYNTAXES
        .iter()
        .find(|syntax| syntax.names.contains(&language.as_str()))
        .copied()
}

/// Lexes one code block, a line at a time.
pub struct Highlighter {
    kind: Kind,
    state: State,
}

enum Kind {
    Table(&'static Syntax),
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Normal,
    /// Inside a block comment that opened on an earlier line.
    Block,
    /// Inside a triple-quoted string that opened on an earlier line.
    Triple(char),
    /// Inside a markdown fence nested in a markdown block.
    Fence,
}

impl Highlighter {
    /// A highlighter for `language`, or nothing when the language is not recognized.
    pub fn new(language: &str) -> Option<Self> {
        let lowered = language.trim().to_ascii_lowercase();
        if MARKDOWN.contains(&lowered.as_str()) {
            return Some(Highlighter {
                kind: Kind::Markdown,
                state: State::Normal,
            });
        }
        syntax_for(&lowered).map(|syntax| Highlighter {
            kind: Kind::Table(syntax),
            state: State::Normal,
        })
    }

    /// Lex one line. The tokens concatenate back to exactly the line that went in.
    pub fn line(&mut self, line: &str) -> Vec<Token> {
        let tokens = match self.kind {
            Kind::Table(syntax) => self.table_line(syntax, line),
            Kind::Markdown => self.markdown_line(line),
        };
        merge(tokens)
    }

    fn table_line(&mut self, syntax: &'static Syntax, line: &str) -> Vec<Token> {
        let characters: Vec<char> = line.chars().collect();
        let mut tokens = Vec::new();
        let mut index = 0;

        // A block comment or triple-quoted string left open by an earlier line swallows
        // whatever comes before its terminator.
        match self.state.clone() {
            State::Block => {
                let (end, closed) = scan_until(&characters, 0, syntax.block_comment.unwrap().1);
                push(&mut tokens, Scope::Comment, &characters[..end]);
                if !closed {
                    return tokens;
                }
                self.state = State::Normal;
                index = end;
            }
            State::Triple(quote) => {
                let closer: String = std::iter::repeat_n(quote, 3).collect();
                let (end, closed) = scan_until(&characters, 0, &closer);
                push(&mut tokens, Scope::String, &characters[..end]);
                if !closed {
                    return tokens;
                }
                self.state = State::Normal;
                index = end;
            }
            _ => {}
        }

        // A line-level annotation claims the rest of the line.
        if index == 0 {
            let leading: String = characters.iter().collect();
            let trimmed = leading.trim_start();
            if syntax
                .meta
                .iter()
                .any(|marker| trimmed.starts_with(marker) && !trimmed.starts_with("#!/usr/bin/env"))
                && !trimmed.is_empty()
            {
                let lead = characters.len() - trimmed.chars().count();
                push(&mut tokens, Scope::Punctuation, &characters[..0]);
                if lead > 0 {
                    tokens.push(Token {
                        scope: None,
                        text: characters[..lead].iter().collect(),
                    });
                }
                push(&mut tokens, Scope::Meta, &characters[lead..]);
                return tokens;
            }
        }

        while index < characters.len() {
            let rest: String = characters[index..].iter().collect();

            if let Some(marker) = syntax
                .line_comment
                .iter()
                .find(|marker| rest.starts_with(**marker))
            {
                let _ = marker;
                push(&mut tokens, Scope::Comment, &characters[index..]);
                return tokens;
            }

            if let Some((open, close)) = syntax.block_comment {
                if rest.starts_with(open) {
                    let (end, closed) =
                        scan_until(&characters, index + open.chars().count(), close);
                    push(&mut tokens, Scope::Comment, &characters[index..end]);
                    if !closed {
                        self.state = State::Block;
                        return tokens;
                    }
                    index = end;
                    continue;
                }
            }

            let character = characters[index];

            if syntax.triple_quotes && syntax.quotes.contains(&character) {
                let triple: String = std::iter::repeat_n(character, 3).collect();
                if rest.starts_with(&triple) {
                    let (end, closed) = scan_until(&characters, index + 3, &triple);
                    push(&mut tokens, Scope::String, &characters[index..end]);
                    if !closed {
                        self.state = State::Triple(character);
                        return tokens;
                    }
                    index = end;
                    continue;
                }
            }

            if syntax.quotes.contains(&character) {
                let end = scan_string(&characters, index, character, syntax.escapes);
                // A double-quoted shell string still expands what it holds, so the names
                // inside it are variables rather than more string.
                if character == '"' && !syntax.sigils.is_empty() {
                    push_interpolated(&mut tokens, &characters[index..end], syntax);
                } else {
                    push(&mut tokens, Scope::String, &characters[index..end]);
                }
                index = end;
                continue;
            }

            if syntax.sigils.contains(&character) {
                let mut end = index + 1;
                while end < characters.len() && is_word(characters[end]) {
                    end += 1;
                }
                // A lone sigil is punctuation; one that names something is a variable.
                let scope = if end > index + 1 {
                    Scope::Variable
                } else {
                    Scope::Punctuation
                };
                push(&mut tokens, scope, &characters[index..end]);
                index = end;
                continue;
            }

            if character.is_ascii_digit() {
                let end = scan_number(&characters, index);
                push(&mut tokens, Scope::Number, &characters[index..end]);
                index = end;
                continue;
            }

            if is_word_start(character) {
                let mut end = index;
                while end < characters.len() && is_word(characters[end]) {
                    end += 1;
                }
                let word: String = characters[index..end].iter().collect();
                let scope = word_scope(&word, &characters, end, syntax);
                push(&mut tokens, scope, &characters[index..end]);
                index = end;
                continue;
            }

            if is_operator(character) {
                let mut end = index;
                while end < characters.len() && is_operator(characters[end]) {
                    end += 1;
                }
                push(&mut tokens, Scope::Operator, &characters[index..end]);
                index = end;
                continue;
            }

            if is_punctuation(character) {
                push(
                    &mut tokens,
                    Scope::Punctuation,
                    &characters[index..index + 1],
                );
                index += 1;
                continue;
            }

            // Whitespace and anything unrecognized carry no scope.
            let start = index;
            while index < characters.len()
                && !is_word_start(characters[index])
                && !characters[index].is_ascii_digit()
                && !is_operator(characters[index])
                && !is_punctuation(characters[index])
                && !syntax.quotes.contains(&characters[index])
                && !syntax.sigils.contains(&characters[index])
            {
                index += 1;
            }
            tokens.push(Token {
                scope: None,
                text: characters[start..index].iter().collect(),
            });
        }

        tokens
    }

    /// Markdown inside a fence: the structure a reader scans for, and nothing more.
    fn markdown_line(&mut self, line: &str) -> Vec<Token> {
        let characters: Vec<char> = line.chars().collect();
        let trimmed = line.trim_start();
        let lead = characters.len() - trimmed.chars().count();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            self.state = if self.state == State::Fence {
                State::Normal
            } else {
                State::Fence
            };
            return vec![Token {
                scope: Some(Scope::Punctuation),
                text: line.to_string(),
            }];
        }
        if self.state == State::Fence {
            return vec![Token {
                scope: None,
                text: line.to_string(),
            }];
        }

        let mut tokens = Vec::new();
        if lead > 0 {
            tokens.push(Token {
                scope: None,
                text: characters[..lead].iter().collect(),
            });
        }

        if trimmed.starts_with('#') {
            push(&mut tokens, Scope::Keyword, &characters[lead..]);
            return tokens;
        }
        if trimmed.starts_with('>') {
            push(&mut tokens, Scope::Comment, &characters[lead..]);
            return tokens;
        }
        if let Some(marker) = ["- ", "* ", "+ "]
            .iter()
            .find(|marker| trimmed.starts_with(**marker))
        {
            push(&mut tokens, Scope::Punctuation, &characters[lead..lead + 2]);
            let _ = marker;
            tokens.extend(markdown_inline(&characters[lead + 2..]));
            return tokens;
        }

        tokens.extend(markdown_inline(&characters[lead..]));
        tokens
    }
}

/// Inline markdown: code spans and link targets, which are the parts worth picking out.
fn markdown_inline(characters: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut plain = String::new();

    while index < characters.len() {
        if characters[index] == '`' {
            if let Some(end) = (index + 1..characters.len()).find(|at| characters[*at] == '`') {
                if !plain.is_empty() {
                    tokens.push(Token {
                        scope: None,
                        text: std::mem::take(&mut plain),
                    });
                }
                push(&mut tokens, Scope::String, &characters[index..=end]);
                index = end + 1;
                continue;
            }
        }
        if characters[index] == '(' {
            if let Some(end) = (index + 1..characters.len()).find(|at| characters[*at] == ')') {
                if !plain.is_empty() {
                    tokens.push(Token {
                        scope: None,
                        text: std::mem::take(&mut plain),
                    });
                }
                push(&mut tokens, Scope::Variable, &characters[index..=end]);
                index = end + 1;
                continue;
            }
        }
        plain.push(characters[index]);
        index += 1;
    }

    if !plain.is_empty() {
        tokens.push(Token {
            scope: None,
            text: plain,
        });
    }
    tokens
}

/// What a bare word is, decided by what surrounds it.
fn word_scope(word: &str, characters: &[char], end: usize, syntax: &Syntax) -> Scope {
    if syntax.keywords.contains(&word) {
        return Scope::Keyword;
    }
    if syntax.types.contains(&word) {
        return Scope::Type;
    }
    // A word with a call's parentheses after it is a function, which is what highlight.js
    // marks `title.function`. Whitespace between the two still counts as a call.
    let next = characters[end..].iter().find(|c| !c.is_whitespace());
    if next == Some(&'(') {
        return Scope::Function;
    }
    // A quoted key is JSON's `attr`; an unquoted one before `=` is TOML's.
    if next == Some(&':') || next == Some(&'=') {
        return Scope::Variable;
    }
    // Leading capital reads as a type in every language here that has them.
    if word.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Scope::Type;
    }
    Scope::Variable
}

/// Advances past a string opened at `start`, stopping after its closing quote or at the end
/// of the line. An unterminated string simply runs to the line's end rather than consuming
/// what follows it.
fn scan_string(characters: &[char], start: usize, quote: char, escapes: bool) -> usize {
    let mut index = start + 1;
    while index < characters.len() {
        if escapes && characters[index] == '\\' {
            index += 2;
            continue;
        }
        if characters[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    characters.len()
}

/// Advances to just past `closer`, or to the end of the line if it is not there.
fn scan_until(characters: &[char], from: usize, closer: &str) -> (usize, bool) {
    let closer: Vec<char> = closer.chars().collect();
    let mut index = from;
    while index + closer.len() <= characters.len() {
        if characters[index..index + closer.len()] == closer[..] {
            return (index + closer.len(), true);
        }
        index += 1;
    }
    (characters.len(), false)
}

fn scan_number(characters: &[char], start: usize) -> usize {
    let mut index = start;
    if characters[index] == '0' && matches!(characters.get(index + 1), Some('x' | 'X' | 'b' | 'o'))
    {
        index += 2;
    }
    while index < characters.len() {
        let character = characters[index];
        let exponent = matches!(character, '+' | '-')
            && index > start
            && matches!(characters[index - 1], 'e' | 'E');
        if character.is_ascii_alphanumeric() || character == '_' || character == '.' || exponent {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn is_word_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn is_operator(character: char) -> bool {
    matches!(
        character,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?'
    )
}

fn is_punctuation(character: char) -> bool {
    matches!(
        character,
        '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '.' | '@' | '#' | '\\'
    )
}

/// Splits a string that expands what it holds into the parts that are string and the parts
/// that name something. Every character still lands in exactly one token.
fn push_interpolated(tokens: &mut Vec<Token>, characters: &[char], syntax: &Syntax) {
    let mut index = 0;
    let mut plain = 0;

    while index < characters.len() {
        if !syntax.sigils.contains(&characters[index]) {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        // `${name}` is as common as `$name`, and both name the same thing.
        let braced = characters.get(end) == Some(&'{');
        if braced {
            end += 1;
            while end < characters.len() && characters[end] != '}' {
                end += 1;
            }
            end = (end + 1).min(characters.len());
        } else {
            while end < characters.len() && is_word(characters[end]) {
                end += 1;
            }
        }

        if end == index + 1 {
            index = end;
            continue;
        }

        push(tokens, Scope::String, &characters[plain..index]);
        push(tokens, Scope::Variable, &characters[index..end]);
        plain = end;
        index = end;
    }

    push(tokens, Scope::String, &characters[plain..]);
}

fn push(tokens: &mut Vec<Token>, scope: Scope, characters: &[char]) {
    if characters.is_empty() {
        return;
    }
    tokens.push(Token {
        scope: Some(scope),
        text: characters.iter().collect(),
    });
}

/// Joins neighbouring tokens that share a scope, so a line becomes as few spans as it can.
fn merge(tokens: Vec<Token>) -> Vec<Token> {
    let mut merged: Vec<Token> = Vec::with_capacity(tokens.len());
    for token in tokens {
        match merged.last_mut() {
            Some(last) if last.scope == token.scope => last.text.push_str(&token.text),
            _ => merged.push(token),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(language: &str, code: &str) -> Vec<(Option<Scope>, String)> {
        let mut highlighter = Highlighter::new(language).expect("a known language");
        code.split('\n')
            .flat_map(|line| highlighter.line(line))
            .map(|token| (token.scope, token.text))
            .collect()
    }

    /// The text a language paints with `scope`, joined.
    fn painted(language: &str, code: &str, scope: Scope) -> Vec<String> {
        scopes(language, code)
            .into_iter()
            .filter(|(found, _)| *found == Some(scope))
            .map(|(_, text)| text)
            .collect()
    }

    /// Every line's tokens concatenated, which must equal the line.
    fn recovered(language: &str, code: &str) -> String {
        let mut highlighter = Highlighter::new(language).expect("a known language");
        code.split('\n')
            .map(|line| {
                highlighter
                    .line(line)
                    .iter()
                    .map(|token| token.text.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    const SAMPLES: &[(&str, &str)] = &[
        (
            "rust",
            "// count things\n#[derive(Debug)]\npub fn main() {\n    let total: u32 = 0x1f + 2;\n    println!(\"hi {total}\");\n    /* a block\n       comment */\n}",
        ),
        (
            "typescript",
            "// a note\nimport { readFile } from \"fs\";\nexport async function main(): Promise<void> {\n  const total: number = 1_000;\n  await readFile(`./x`);\n}",
        ),
        (
            "python",
            "# a note\n@decorator\ndef main(count: int) -> None:\n    total = 0xff\n    text = 'hi'\n    doc = \"\"\"spans\n    lines\"\"\"\n    return None",
        ),
        (
            "json",
            "{\n  \"name\": \"micro\",\n  \"count\": 12,\n  \"ok\": true,\n  \"missing\": null\n}",
        ),
        (
            "toml",
            "# a note\n[package]\nname = \"micro\"\nversion = \"0.1.0\"\nedition = 2021\nstrict = true",
        ),
        (
            "bash",
            "#!/bin/sh\n# a note\nfor file in *.rs; do\n  echo \"$file\"\n  grep -n 'needle' \"$file\"\ndone",
        ),
        (
            "markdown",
            "# Title\n\n- an item with `code`\n> a quote\n\n[text](https://example.com)",
        ),
    ];

    #[test]
    fn every_supported_language_recovers_its_input_exactly() {
        for (language, code) in SAMPLES {
            assert_eq!(
                recovered(language, code),
                *code,
                "{language} did not survive the round trip"
            );
        }
    }

    #[test]
    fn recovery_holds_for_awkward_input() {
        let awkward = [
            "",
            "\n",
            "\n\n\n",
            "   ",
            "\t\ttabbed",
            "\"unterminated",
            "'unterminated",
            "/* unterminated",
            "let x = \"a\\\"b\";",
            "emoji 🎨 and ünïcode",
            "0x1f 1_000 1.5e-3 .5",
            "a(((b)))",
            "$ $$ $VAR",
            "```",
            "#",
            "@",
            "----",
        ];
        for (language, _) in SAMPLES {
            for line in awkward {
                assert_eq!(
                    recovered(language, line),
                    line,
                    "{language} mangled {line:?}"
                );
            }
        }
    }

    #[test]
    fn an_unknown_language_produces_no_highlighter() {
        for language in ["", "brainfuck", "cobol", "text", "plain", "rustacean"] {
            assert!(Highlighter::new(language).is_none(), "{language}");
        }
    }

    #[test]
    fn a_language_tag_is_matched_regardless_of_case_or_spacing() {
        for language in ["RUST", " rust ", "Rust", "TypeScript", "PY"] {
            assert!(Highlighter::new(language).is_some(), "{language}");
        }
    }

    #[test]
    fn rust_recognizes_each_category() {
        let code = SAMPLES[0].1;
        assert!(painted("rust", code, Scope::Comment)
            .iter()
            .any(|text| text.contains("count things")));
        assert!(painted("rust", code, Scope::Keyword).contains(&"pub".to_string()));
        assert!(painted("rust", code, Scope::Keyword).contains(&"fn".to_string()));
        assert!(painted("rust", code, Scope::Function).contains(&"main".to_string()));
        assert!(painted("rust", code, Scope::Type).contains(&"u32".to_string()));
        assert!(painted("rust", code, Scope::Number).contains(&"0x1f".to_string()));
        assert!(painted("rust", code, Scope::String)
            .iter()
            .any(|text| text.contains("hi {total}")));
        assert!(painted("rust", code, Scope::Meta)
            .iter()
            .any(|text| text.starts_with("#[derive")));
        assert!(!painted("rust", code, Scope::Operator).is_empty());
        assert!(!painted("rust", code, Scope::Punctuation).is_empty());
    }

    #[test]
    fn typescript_recognizes_each_category() {
        let code = SAMPLES[1].1;
        assert!(painted("typescript", code, Scope::Keyword).contains(&"import".to_string()));
        assert!(painted("typescript", code, Scope::Keyword).contains(&"async".to_string()));
        assert!(painted("typescript", code, Scope::Function).contains(&"main".to_string()));
        assert!(painted("typescript", code, Scope::Type).contains(&"Promise".to_string()));
        assert!(painted("typescript", code, Scope::Type).contains(&"number".to_string()));
        assert!(painted("typescript", code, Scope::Number).contains(&"1_000".to_string()));
        // Backticks open a string, as they do in the language.
        assert!(painted("typescript", code, Scope::String).contains(&"`./x`".to_string()));
        assert!(painted("typescript", code, Scope::Comment).contains(&"// a note".to_string()));
    }

    #[test]
    fn python_recognizes_each_category() {
        let code = SAMPLES[2].1;
        assert!(painted("python", code, Scope::Comment).contains(&"# a note".to_string()));
        assert!(painted("python", code, Scope::Meta).contains(&"@decorator".to_string()));
        assert!(painted("python", code, Scope::Keyword).contains(&"def".to_string()));
        assert!(painted("python", code, Scope::Function).contains(&"main".to_string()));
        assert!(painted("python", code, Scope::Type).contains(&"int".to_string()));
        assert!(painted("python", code, Scope::Number).contains(&"0xff".to_string()));
        assert!(painted("python", code, Scope::String).contains(&"'hi'".to_string()));
    }

    #[test]
    fn a_triple_quoted_string_spans_lines() {
        let painted = painted("python", SAMPLES[2].1, Scope::String);
        assert!(painted.iter().any(|text| text.contains("spans")));
        assert!(painted.iter().any(|text| text.contains("lines\"\"\"")));
    }

    #[test]
    fn json_marks_keys_apart_from_values() {
        let code = SAMPLES[3].1;
        assert!(painted("json", code, Scope::Keyword).contains(&"true".to_string()));
        assert!(painted("json", code, Scope::Keyword).contains(&"null".to_string()));
        assert!(painted("json", code, Scope::Number).contains(&"12".to_string()));
        assert!(painted("json", code, Scope::String).contains(&"\"micro\"".to_string()));
    }

    #[test]
    fn toml_recognizes_its_shape() {
        let code = SAMPLES[4].1;
        assert!(painted("toml", code, Scope::Comment).contains(&"# a note".to_string()));
        assert!(painted("toml", code, Scope::Variable).contains(&"name".to_string()));
        assert!(painted("toml", code, Scope::String).contains(&"\"micro\"".to_string()));
        assert!(painted("toml", code, Scope::Number).contains(&"2021".to_string()));
        assert!(painted("toml", code, Scope::Keyword).contains(&"true".to_string()));
    }

    #[test]
    fn shell_recognizes_its_shape() {
        let code = SAMPLES[5].1;
        assert!(painted("bash", code, Scope::Meta).contains(&"#!/bin/sh".to_string()));
        assert!(painted("bash", code, Scope::Comment).contains(&"# a note".to_string()));
        assert!(painted("bash", code, Scope::Keyword).contains(&"for".to_string()));
        assert!(painted("bash", code, Scope::Keyword).contains(&"done".to_string()));
        assert!(painted("bash", code, Scope::Type).contains(&"echo".to_string()));
        assert!(painted("bash", code, Scope::Variable).contains(&"$file".to_string()));
        assert!(painted("bash", code, Scope::String).contains(&"'needle'".to_string()));
    }

    #[test]
    fn markdown_recognizes_its_shape() {
        let code = SAMPLES[6].1;
        assert!(painted("markdown", code, Scope::Keyword).contains(&"# Title".to_string()));
        assert!(painted("markdown", code, Scope::Comment).contains(&"> a quote".to_string()));
        assert!(painted("markdown", code, Scope::String).contains(&"`code`".to_string()));
        assert!(painted("markdown", code, Scope::Variable)
            .contains(&"(https://example.com)".to_string()));
    }

    #[test]
    fn a_block_comment_carries_across_lines_and_closes() {
        let mut highlighter = Highlighter::new("rust").unwrap();
        let first = highlighter.line("/* open");
        assert_eq!(first[0].scope, Some(Scope::Comment));

        let middle = highlighter.line("still inside");
        assert_eq!(middle[0].scope, Some(Scope::Comment));

        let last = highlighter.line("done */ let x = 1;");
        assert_eq!(last[0].scope, Some(Scope::Comment));
        assert_eq!(last[0].text, "done */");
        assert!(last.iter().any(|token| token.scope == Some(Scope::Keyword)));
    }

    #[test]
    fn an_unterminated_block_comment_does_not_run_away() {
        let mut highlighter = Highlighter::new("rust").unwrap();
        for line in ["/* open", "one", "two", "three"] {
            let tokens = highlighter.line(line);
            assert_eq!(tokens.len(), 1);
            assert_eq!(tokens[0].scope, Some(Scope::Comment));
            assert_eq!(tokens[0].text, line);
        }
    }

    #[test]
    fn an_unterminated_string_stops_at_the_end_of_its_line() {
        let mut highlighter = Highlighter::new("rust").unwrap();
        let tokens = highlighter.line("let x = \"never closed");
        assert_eq!(tokens.last().unwrap().scope, Some(Scope::String));
        assert_eq!(tokens.last().unwrap().text, "\"never closed");

        // The next line is lexed normally rather than as more string.
        let next = highlighter.line("let y = 1;");
        assert_eq!(next[0].scope, Some(Scope::Keyword));
    }

    #[test]
    fn an_escaped_quote_does_not_close_its_string() {
        let tokens = scopes("rust", r#"let x = "a\"b";"#);
        let strings: Vec<_> = tokens
            .iter()
            .filter(|(scope, _)| *scope == Some(Scope::String))
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(strings, vec![r#""a\"b""#]);
    }

    #[test]
    fn neighbouring_tokens_of_one_scope_become_one() {
        let tokens = scopes("rust", "a.b.c");
        // The dots are punctuation and the names are variables, so nothing collapses; but
        // no two neighbours ever share a scope.
        for pair in tokens.windows(2) {
            assert_ne!(pair[0].0, pair[1].0, "{pair:?} should have been merged");
        }
    }

    #[test]
    fn every_scope_maps_to_a_theme_color() {
        let theme = Theme::dark();
        for (scope, expected) in [
            (Scope::Comment, theme.syntax_comment),
            (Scope::Keyword, theme.syntax_keyword),
            (Scope::Function, theme.syntax_function),
            (Scope::Variable, theme.syntax_variable),
            (Scope::String, theme.syntax_string),
            (Scope::Number, theme.syntax_number),
            (Scope::Type, theme.syntax_type),
            (Scope::Operator, theme.syntax_operator),
            (Scope::Punctuation, theme.syntax_punctuation),
            // Meta goes to `muted`, not to a syntax color.
            (Scope::Meta, theme.muted),
        ] {
            assert_eq!(scope.style(&theme).fg, Some(expected), "{scope:?}");
        }
    }

    #[test]
    fn lexing_a_long_line_stays_quick() {
        let line = "let value = compute(alpha, beta) + \"text\"; // note ".repeat(400);
        let mut highlighter = Highlighter::new("rust").unwrap();
        let started = std::time::Instant::now();
        let tokens = highlighter.line(&line);
        assert!(started.elapsed().as_millis() < 200, "lexing was slow");
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.text.clone())
                .collect::<String>(),
            line
        );
    }
}
