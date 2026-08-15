//! Flags micro does not know about.

/// A flag that was written on the command line but not understood by micro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub name: String,
    /// What followed it, when anything did.
    pub value: Option<String>,
}

/// Split arguments into the ones micro's own parser should see and the ones it would not
/// understand.
pub fn split_unknown(
    arguments: impl IntoIterator<Item = String>,
    known: &[&str],
    known_with_value: &[&str],
) -> (Vec<String>, Vec<Given>) {
    let mut kept = Vec::new();
    let mut leftover = Vec::new();
    let mut arguments = arguments.into_iter().peekable();
    
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
            
            if inline.is_none() && known_with_value.contains(&name) {
                if let Some(value) = arguments.peek() {
                    if !value.starts_with("--") {
                        kept.push(arguments.next().unwrap_or_default());
                    }
                }
            }
            continue;
        }

        
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
        assert_eq!(
            kept,
            vec!["micro", "--print", "--model", "opus", "a prompt"]
        );
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

    /// A value is written with an equals sign.
    #[test]
    fn a_value_is_written_with_an_equals_sign() {
        let (_, leftover) = split(&["micro", "--env=staging"]);
        assert_eq!(leftover[0].value.as_deref(), Some("staging"));

        
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
