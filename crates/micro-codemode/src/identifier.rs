//! The names scripts use for tools.

/// The identifier a script uses for a tool: characters that are not valid in a JavaScript
/// identifier become `_`. `mcp__docs__search` stays as it is, `my-tool` becomes `my_tool`.
pub fn identifier(name: &str) -> String {
    let mut identifier = String::with_capacity(name.len());
    for character in name.chars() {
        let valid = match identifier.is_empty() {
            true => character.is_ascii_alphabetic() || character == '_' || character == '$',
            false => character.is_ascii_alphanumeric() || character == '_' || character == '$',
        };
        identifier.push(if valid { character } else { '_' });
    }
    match identifier.is_empty() {
        true => "_".to_string(),
        false => identifier,
    }
}

/// Whether `name` can be used as it is, as an identifier or a property name.
pub fn is_identifier(name: &str) -> bool {
    !name.is_empty() && identifier(name) == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_characters_become_underscores() {
        assert_eq!(identifier("mcp__docs__search"), "mcp__docs__search");
        assert_eq!(identifier("my-tool"), "my_tool");
        assert_eq!(identifier("9lives"), "_lives");
        assert_eq!(identifier("ünï"), "_n_");
        assert_eq!(identifier(""), "_");
    }

    #[test]
    fn identifiers_are_recognized() {
        assert!(is_identifier("read"));
        assert!(!is_identifier("read-file"));
        assert!(!is_identifier(""));
    }
}
