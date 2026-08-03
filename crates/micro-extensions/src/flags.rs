//! Flags micro does not know about.
//!
//! An extension may declare a command-line flag, and it declares it after the command line
//! has already been read — the extensions are loaded by the run the flags configure. So the
//! arguments are split first: everything micro's own parser understands goes to it, and
//! anything left over is held until the extensions have said what they answer to.
//!
//! A value for an unknown flag is written with an equals sign: `--env=staging`. Nothing
//! else would be safe, because whether a flag takes a value is declared by an extension
//! that has not loaded yet — and guessing wrong would swallow the prompt.
//!
//! Nothing is guessed. A leftover flag nobody claims is reported rather than ignored, so a
//! typo is visible instead of silently doing nothing.

/// A flag that was written on the command line but not understood by micro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub name: String,
    /// What followed it, when anything did. A flag with no value is a flag that is on.
    pub value: Option<String>,
}

/// Split arguments into the ones micro's own parser should see and the ones it would not
/// understand.
///
/// `known` names the long flags micro takes a value for, so `--model opus` keeps its value
/// rather than leaving `opus` stranded.
pub fn split_unknown(
    arguments: impl IntoIterator<Item = String>,
    known: &[&str],
    known_with_value: &[&str],
) -> (Vec<String>, Vec<Given>) {
    let mut kept = Vec::new();
    let mut leftover = Vec::new();
    let mut arguments = arguments.into_iter().peekable();
    // Everything after `--` is a positional argument, whatever it looks like.
    let mut only_positional = false;

    while let Some(argument) = arguments.next() {
        if only_positional || !argument.starts_with("--") || argument == "--" {
            only_positional |= argument == "--";
            kept.push(argument);
            continue;
        }

        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name.trim_start_matches("--"), Some(value.to_string())),
            None => (argument.trim_start_matches("--"), None),
        };

        if known.contains(&name) || known_with_value.contains(&name) {
            kept.push(argument.clone());
            // A known flag that takes a value written apart from it keeps that value.
            if inline.is_none() && known_with_value.contains(&name) {
                if let Some(value) = arguments.peek() {
                    if !value.starts_with("--") {
                        kept.push(arguments.next().unwrap_or_default());
                    }
                }
            }
            continue;
        }

        // Only a value written with an equals sign is taken. Anything after a space
        // belongs to the prompt until an extension says otherwise.
        leftover.push(Given {
            name: name.to_string(),
            value: inline,
        });
    }

    (kept, leftover)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(line: &[&str]) -> (Vec<String>, Vec<Given>) {
        split_unknown(
            line.iter().map(|argument| argument.to_string()),
            &["print", "quiet"],
            &["model", "provider"],
        )
    }

    #[test]
    fn what_micro_understands_is_left_alone() {
        let (kept, leftover) = split(&["micro", "--print", "--model", "opus", "a prompt"]);
        assert_eq!(kept, vec!["micro", "--print", "--model", "opus", "a prompt"]);
        assert!(leftover.is_empty());
    }

    #[test]
    fn a_flag_micro_does_not_know_is_held_back() {
        let (kept, leftover) = split(&["micro", "--verbose", "a prompt"]);
        assert_eq!(kept, vec!["micro", "a prompt"]);
        assert_eq!(
            leftover,
            vec![Given {
                name: "verbose".into(),
                value: None
            }]
        );
    }

    /// A value is written with an equals sign, because whether the flag takes one is not
    /// known until the extension that declared it has loaded.
    #[test]
    fn a_value_is_written_with_an_equals_sign() {
        let (_, leftover) = split(&["micro", "--env=staging"]);
        assert_eq!(leftover[0].value.as_deref(), Some("staging"));

        // Written apart, the word belongs to the prompt rather than to the flag.
        let (kept, leftover) = split(&["micro", "--env", "staging"]);
        assert_eq!(kept, vec!["micro", "staging"]);
        assert_eq!(leftover[0].value, None);
    }

    /// An unknown flag does not eat the flag after it.
    #[test]
    fn an_unknown_flag_leaves_the_next_flag_alone() {
        let (kept, leftover) = split(&["micro", "--verbose", "--model", "opus"]);
        assert_eq!(kept, vec!["micro", "--model", "opus"]);
        assert_eq!(leftover[0].name, "verbose");
        assert_eq!(leftover[0].value, None);
    }

    /// Everything after `--` is a prompt, however it is spelled.
    #[test]
    fn nothing_after_a_bare_double_dash_is_a_flag() {
        let (kept, leftover) = split(&["micro", "--", "--not-a-flag"]);
        assert_eq!(kept, vec!["micro", "--", "--not-a-flag"]);
        assert!(leftover.is_empty());
    }

    #[test]
    fn short_flags_are_micros_own_business() {
        let (kept, leftover) = split(&["micro", "-p", "-m", "opus"]);
        assert_eq!(kept, vec!["micro", "-p", "-m", "opus"]);
        assert!(leftover.is_empty());
    }
}
