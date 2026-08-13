//! Slash commands a user writes for themselves.
//!
//! A markdown file in `prompts/` becomes a command named after the file. Running it
//! substitutes whatever was typed after the name into the file's body, and the result is
//! sent as the prompt. Nothing about it reaches the model until it is run, so a directory
//! of them costs nothing to have.
//!
//! The substitutions are the ones a shell uses, because that is what the syntax looks
//! like and guessing differently would be a trap: `$1` for the first argument, `$@` and
//! `$ARGUMENTS` for all of them, `${1:-default}` for one that may be missing, and
//! `${@:2}` or `${@:2:3}` for a run of them.

mod frontmatter;

pub use frontmatter::parse_frontmatter;
pub use frontmatter::Frontmatter;

use std::path::Path;
use std::path::PathBuf;

/// How long a description taken from the body may be before it is cut short.
const DERIVED_DESCRIPTION_LIMIT: usize = 60;

/// The directory prompts are kept in, under the home directory and under the project's.
pub const PROMPTS_DIR: &str = "prompts";

/// One prompt file, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// What it is invoked as, which is the file's name without its extension.
    pub name: String,
    /// What it does, for the command list.
    pub description: String,
    /// What to type after the name, when the file says.
    pub argument_hint: Option<String>,
    /// The body, before anything is substituted into it.
    pub content: String,
    pub path: PathBuf,
}

impl PromptTemplate {
    /// The prompt to send, with `arguments` substituted into the body.
    pub fn render(&self, arguments: &str) -> String {
        substitute(&self.content, &parse_arguments(arguments))
    }
}

/// Split what was typed after the command name into arguments.
///
/// Quoting works the way it does in a shell, so an argument with a space in it can be
/// written as one.
pub fn parse_arguments(text: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for character in text.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character.is_whitespace() => {
                if !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
            }
            None => current.push(character),
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

/// Replace the argument placeholders in `content`.
///
/// Only the template is rewritten. An argument that itself contains something looking like
/// a placeholder is left as it is, so what a user typed is never re-read as syntax.
pub fn substitute(content: &str, arguments: &[String]) -> String {
    let all = arguments.join(" ");
    let characters: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut index = 0;

    while index < characters.len() {
        if characters[index] != '$' {
            out.push(characters[index]);
            index += 1;
            continue;
        }

        match placeholder_at(&characters, index) {
            Some((replacement, length)) => {
                out.push_str(&resolve(&replacement, arguments, &all));
                index += length;
            }
            // Not a placeholder, so it is just a dollar sign.
            None => {
                out.push('$');
                index += 1;
            }
        }
    }
    out
}

/// What a placeholder asks for.
enum Placeholder {
    /// `$1`, `$2`, …
    Positional(usize),
    /// `$@` or `$ARGUMENTS`
    All,
    /// `${1:-default}`, `${@:-default}`
    WithDefault(Box<Placeholder>, String),
    /// `${@:2}` and `${@:2:3}`
    Slice { start: usize, length: Option<usize> },
}

/// Read the placeholder starting at `start`, and how many characters it occupies.
fn placeholder_at(characters: &[char], start: usize) -> Option<(Placeholder, usize)> {
    let after = start + 1;
    if after >= characters.len() {
        return None;
    }

    if characters[after] == '{' {
        let close = (after + 1..characters.len()).find(|index| characters[*index] == '}')?;
        let inside: String = characters[after + 1..close].iter().collect();
        let placeholder = braced(&inside)?;
        return Some((placeholder, close - start + 1));
    }

    // `$ARGUMENTS`, `$@`, `$1`
    if characters[after] == '@' {
        return Some((Placeholder::All, 2));
    }
    let word: String = characters[after..].iter().take("ARGUMENTS".len()).collect();
    if word == "ARGUMENTS" {
        return Some((Placeholder::All, 1 + word.len()));
    }
    let digits: String = characters[after..]
        .iter()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    let position = digits.parse().ok()?;
    Some((Placeholder::Positional(position), 1 + digits.len()))
}

/// What was written between the braces.
fn braced(inside: &str) -> Option<Placeholder> {
    if let Some((target, fallback)) = inside.split_once(":-") {
        let target = match target {
            "@" | "ARGUMENTS" => Placeholder::All,
            digits => Placeholder::Positional(digits.parse().ok()?),
        };
        return Some(Placeholder::WithDefault(
            Box::new(target),
            fallback.to_string(),
        ));
    }

    let rest = inside.strip_prefix("@:")?;
    let (start, length) = match rest.split_once(':') {
        Some((start, length)) => (start, Some(length.parse().ok()?)),
        None => (rest, None),
    };
    Some(Placeholder::Slice {
        start: start.parse().ok()?,
        length,
    })
}

fn resolve(placeholder: &Placeholder, arguments: &[String], all: &str) -> String {
    match placeholder {
        Placeholder::All => all.to_string(),
        // Written 1-indexed, the way a shell writes them.
        Placeholder::Positional(position) => position
            .checked_sub(1)
            .and_then(|index| arguments.get(index))
            .cloned()
            .unwrap_or_default(),
        Placeholder::WithDefault(target, fallback) => {
            let value = resolve(target, arguments, all);
            match value.is_empty() {
                true => fallback.clone(),
                false => value,
            }
        }
        Placeholder::Slice { start, length } => {
            // A shell counts from 1 here too, and treats 0 as 1.
            let from = start.saturating_sub(1).min(arguments.len());
            let taken = match length {
                Some(length) => arguments[from..].iter().take(*length),
                None => arguments[from..].iter().take(usize::MAX),
            };
            taken.cloned().collect::<Vec<_>>().join(" ")
        }
    }
}

/// Read one prompt file. A file that cannot be read is not a template.
pub fn load(path: &Path) -> Option<PromptTemplate> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed = parse_frontmatter(&raw);
    let body = parsed.body().to_string();

    let name = path.file_stem()?.to_str()?.to_string();
    if name.is_empty() {
        return None;
    }

    // A file that does not describe itself is described by its first line, which is
    // usually a heading and says enough to pick it out of a list.
    let description = match parsed.field("description") {
        Some(description) if !description.trim().is_empty() => description.trim().to_string(),
        _ => derive_description(&body),
    };

    Some(PromptTemplate {
        name,
        description,
        argument_hint: parsed
            .field("argument-hint")
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
            .map(str::to_string),
        content: body,
        path: path.to_path_buf(),
    })
}

fn derive_description(body: &str) -> String {
    let Some(line) = body.lines().find(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let line = line.trim();
    match line.chars().count() > DERIVED_DESCRIPTION_LIMIT {
        true => {
            let cut: String = line.chars().take(DERIVED_DESCRIPTION_LIMIT).collect();
            format!("{cut}...")
        }
        false => line.to_string(),
    }
}

/// Every prompt available here, by name.
///
/// The user's own are read first and the project's are laid over them, so a project can
/// offer a command of its own without the user losing theirs elsewhere. The project's are
/// read only once it is trusted: a prompt is text put in front of the model.
pub fn discover(root: &Path, home: &Path, trusted: bool) -> Vec<PromptTemplate> {
    let mut found: std::collections::BTreeMap<String, PromptTemplate> = Default::default();

    let mut directories = vec![home.join(PROMPTS_DIR)];
    if trusted {
        directories.push(root.join(micro_config::PROJECT_DIR).join(PROMPTS_DIR));
    }

    for directory in directories {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect();
        paths.sort();

        for path in paths {
            if let Some(template) = load(&path) {
                found.insert(template.name.clone(), template);
            }
        }
    }

    found.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        parse_arguments(text)
    }

    #[test]
    fn arguments_split_on_whitespace() {
        assert_eq!(args("one two three"), vec!["one", "two", "three"]);
        assert_eq!(args("   spaced   out  "), vec!["spaced", "out"]);
        assert!(args("").is_empty());
    }

    #[test]
    fn quoting_keeps_an_argument_whole() {
        assert_eq!(args(r#"one "two three" four"#), vec!["one", "two three", "four"]);
        assert_eq!(args("'single quoted'"), vec!["single quoted"]);
    }

    #[test]
    fn a_positional_argument_is_substituted() {
        assert_eq!(substitute("fix $1 please", &args("auth")), "fix auth please");
        // One that was not given leaves nothing behind.
        assert_eq!(substitute("fix $2", &args("auth")), "fix ");
    }

    #[test]
    fn all_arguments_can_be_taken_together() {
        assert_eq!(substitute("review $@", &args("a b c")), "review a b c");
        assert_eq!(
            substitute("review $ARGUMENTS", &args("a b c")),
            "review a b c"
        );
    }

    #[test]
    fn a_default_fills_in_for_a_missing_argument() {
        assert_eq!(substitute("on ${1:-main}", &args("")), "on main");
        assert_eq!(substitute("on ${1:-main}", &args("dev")), "on dev");
        assert_eq!(substitute("all ${@:-nothing}", &args("")), "all nothing");
    }

    #[test]
    fn a_run_of_arguments_can_be_taken() {
        assert_eq!(substitute("${@:2}", &args("a b c d")), "b c d");
        assert_eq!(substitute("${@:2:2}", &args("a b c d")), "b c");
        // A shell counts from one, and treats zero as one.
        assert_eq!(substitute("${@:0}", &args("a b")), "a b");
        assert_eq!(substitute("${@:9}", &args("a b")), "");
    }

    /// What the user typed is text, not more template. A `$1` inside an argument stays a
    /// `$1` rather than being read again.
    #[test]
    fn an_argument_is_not_substituted_into_twice() {
        assert_eq!(substitute("say $1", &args(r#""$2 and $@""#)), "say $2 and $@");
    }

    #[test]
    fn a_dollar_that_is_not_a_placeholder_is_left_alone() {
        assert_eq!(substitute("costs $5.00", &args("")), "costs .00");
        assert_eq!(substitute("100% $ done", &args("")), "100% $ done");
        assert_eq!(substitute("shell $VAR", &args("")), "shell $VAR");
    }

    #[test]
    fn a_template_describes_itself_or_its_first_line() {
        let described = "---\ndescription: Review a pull request\nargument-hint: <number>\n---\nDo the thing\n";
        let parsed = parse_frontmatter(described);
        assert_eq!(parsed.field("description"), Some("Review a pull request"));

        assert_eq!(derive_description("# Fix the failing tests\n\nmore"), "# Fix the failing tests");
        assert_eq!(derive_description("   \n\n"), "");
        let long = "x".repeat(80);
        assert!(derive_description(&long).ends_with("..."));
        assert_eq!(derive_description(&long).chars().count(), DERIVED_DESCRIPTION_LIMIT + 3);
    }
}

#[cfg(test)]
mod discovery {
    use super::*;

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("micro-prompts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("project");
        let home = base.join("home");
        std::fs::create_dir_all(root.join(micro_config::PROJECT_DIR).join(PROMPTS_DIR)).unwrap();
        std::fs::create_dir_all(home.join(PROMPTS_DIR)).unwrap();
        (root, home)
    }

    /// A markdown file becomes a command named after it, and running it substitutes what
    /// was typed into the body.
    #[test]
    fn a_file_becomes_a_command() {
        let (root, home) = scratch("basic");
        std::fs::write(
            home.join(PROMPTS_DIR).join("review.md"),
            "---\ndescription: Review a pull request\nargument-hint: <number>\n---\nReview PR $1 on ${2:-main}.\n",
        )
        .unwrap();

        let found = discover(&root, &home, true);
        assert_eq!(found.len(), 1);
        let template = &found[0];
        assert_eq!(template.name, "review");
        assert_eq!(template.description, "Review a pull request");
        assert_eq!(template.argument_hint.as_deref(), Some("<number>"));
        assert_eq!(template.render("42").trim_end(), "Review PR 42 on main.");
        assert_eq!(template.render("42 dev").trim_end(), "Review PR 42 on dev.");
    }

    /// A project's own prompt is offered only once the project is trusted.
    #[test]
    fn an_untrusted_project_offers_no_prompts() {
        let (root, home) = scratch("trust");
        std::fs::write(
            root.join(micro_config::PROJECT_DIR).join(PROMPTS_DIR).join("deploy.md"),
            "Ship it.\n",
        )
        .unwrap();

        assert!(discover(&root, &home, false).is_empty());
        assert_eq!(discover(&root, &home, true).len(), 1);
    }

    /// A project may offer a command of its own name; the user keeps theirs elsewhere.
    #[test]
    fn a_project_prompt_wins_over_the_users_by_the_same_name() {
        let (root, home) = scratch("shadow");
        std::fs::write(home.join(PROMPTS_DIR).join("ship.md"), "user version\n").unwrap();
        std::fs::write(
            root.join(micro_config::PROJECT_DIR).join(PROMPTS_DIR).join("ship.md"),
            "project version\n",
        )
        .unwrap();

        let found = discover(&root, &home, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].render("").trim_end(), "project version");
    }

    /// Anything that is not a markdown file is not a prompt.
    #[test]
    fn only_markdown_files_count() {
        let (root, home) = scratch("kinds");
        std::fs::write(home.join(PROMPTS_DIR).join("notes.txt"), "not a prompt").unwrap();
        std::fs::write(home.join(PROMPTS_DIR).join("real.md"), "a prompt").unwrap();

        let found = discover(&root, &home, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "real");
    }
}
