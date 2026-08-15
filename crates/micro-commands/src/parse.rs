//! Turning a typed line into a command, or leaving it alone.

use crate::find;
use crate::Command;

/// How far apart two names can be and still be worth suggesting.
const MAX_SUGGESTION_DISTANCE: usize = 2;

/// A line the user submitted, understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input<'a> {
    /// Ordinary text, to be sent to the model.
    Prompt(&'a str),
    /// A command micro knows, with whatever followed its name.
    Command {
        command: &'static Command,
        argument: Option<&'a str>,
    },
    /// A slash followed by something that reads like a command but is not one.
    Unknown {
        name: &'a str,
        suggestion: Option<&'static str>,
    },
}

/// Read a submitted line.
pub fn parse(line: &str) -> Input<'_> {
    let trimmed = line.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Input::Prompt(trimmed);
    };

    let (name, argument) = match rest.find(char::is_whitespace) {
        Some(split) => (&rest[..split], rest[split..].trim()),
        None => (rest, ""),
    };

    
    if !is_command_name(name) {
        return Input::Prompt(trimmed);
    }

    let argument = (!argument.is_empty()).then_some(argument);

    match find(name) {
        Some(command) => Input::Command { command, argument },
        None => Input::Unknown {
            name,
            suggestion: suggest(name),
        },
    }
}

/// A command name is a letter followed by letters, digits, or dashes.
fn is_command_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// The known command closest to `name`, when one is close enough to be worth offering.
pub fn suggest(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    let first = name.chars().next();

    crate::commands()
        .iter()
        .map(|command| {
            let same_start = command.name.chars().next() == first;
            (
                distance(&name, command.name),
                usize::from(!same_start),
                command.name,
            )
        })
        .filter(|(distance, ..)| *distance <= MAX_SUGGESTION_DISTANCE)
        .min_by_key(|(distance, start, _)| (*distance, *start))
        .map(|(.., name)| name)
}

/// Levenshtein distance, over one row of the matrix.
fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();

    for (i, left_character) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;

        for (j, right_character) in right.iter().enumerate() {
            let substitution = diagonal + usize::from(left_character != *right_character);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }

    row[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_name(input: &Input<'_>) -> &'static str {
        match input {
            Input::Command { command, .. } => command.name,
            other => panic!("expected a command, got {other:?}"),
        }
    }

    fn argument<'a>(input: &Input<'a>) -> Option<&'a str> {
        match input {
            Input::Command { argument, .. } => *argument,
            other => panic!("expected a command, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_command_has_no_argument() {
        let input = parse("/help");
        assert_eq!(command_name(&input), "help");
        assert_eq!(argument(&input), None);
    }

    #[test]
    fn everything_after_the_name_is_the_argument() {
        let input = parse("/model claude opus 5");
        assert_eq!(command_name(&input), "model");
        assert_eq!(argument(&input), Some("claude opus 5"));
    }

    #[test]
    fn surrounding_and_inner_whitespace_is_tolerated() {
        assert_eq!(argument(&parse("   /model    opus   ")), Some("opus"));
        assert_eq!(argument(&parse("/model\topus")), Some("opus"));
        assert_eq!(argument(&parse("/help   ")), None);
    }

    #[test]
    fn a_command_name_is_matched_whatever_its_case() {
        assert_eq!(command_name(&parse("/HELP")), "help");
        assert_eq!(command_name(&parse("/Model opus")), "model");
    }

    #[test]
    fn plain_text_is_left_for_the_model() {
        assert_eq!(
            parse("what does this do?"),
            Input::Prompt("what does this do?")
        );
        assert_eq!(parse("  spaced  "), Input::Prompt("spaced"));
        assert_eq!(parse(""), Input::Prompt(""));
    }

    #[test]
    fn a_bare_slash_is_not_a_command() {
        assert_eq!(parse("/"), Input::Prompt("/"));
        assert_eq!(parse("  /  "), Input::Prompt("/"));
    }

    #[test]
    fn a_line_that_opens_with_a_path_reaches_the_model() {
        assert_eq!(
            parse("/usr/bin/env python"),
            Input::Prompt("/usr/bin/env python")
        );
        assert_eq!(parse("/etc/hosts"), Input::Prompt("/etc/hosts"));
        assert_eq!(parse("//comment"), Input::Prompt("//comment"));
        assert_eq!(parse("/2fa"), Input::Prompt("/2fa"));
    }

    #[test]
    fn an_unknown_command_offers_the_nearest_one() {
        assert_eq!(
            parse("/modl opus"),
            Input::Unknown {
                name: "modl",
                suggestion: Some("model"),
            }
        );
        assert_eq!(
            parse("/quti"),
            Input::Unknown {
                name: "quti",
                suggestion: Some("quit"),
            }
        );
        assert_eq!(
            parse("/helpp"),
            Input::Unknown {
                name: "helpp",
                suggestion: Some("help"),
            }
        );
    }

    #[test]
    fn an_unknown_command_with_nothing_close_suggests_nothing() {
        assert_eq!(
            parse("/xyzzy"),
            Input::Unknown {
                name: "xyzzy",
                suggestion: None,
            }
        );
    }

    #[test]
    fn distances_count_edits() {
        assert_eq!(distance("model", "model"), 0);
        assert_eq!(distance("modl", "model"), 1);
        assert_eq!(distance("quti", "quit"), 2);
        assert_eq!(distance("", "quit"), 4);
        assert_eq!(distance("quit", ""), 4);
    }
}
