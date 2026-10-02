//! The names servers and their tools are known by to the model.

use sha2::Digest as _;
use sha2::Sha256;
use std::collections::HashMap;

/// Providers accept tool names of at most this many characters.
const MAX_TOOL_NAME_LENGTH: usize = 64;

/// Hex characters of the hash that tells colliding names apart.
const HASH_LENGTH: usize = 8;

/// What separates a server's name from a tool's in the name the model sees.
const SEPARATOR: &str = "__";

/// Whether `name` may name a server: letters, digits, `_` and `-`.
pub fn is_valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

/// The prefix every tool of a server shares: `mcp__<server>` with `-` written as `_`, so names
/// that differ only in those two belong to the same server.
pub fn namespace(server: &str) -> String {
    format!("mcp{SEPARATOR}{}", server.replace('-', "_"))
}

/// The name the model calls a server's tool by: `mcp__<server>__<tool>`, with everything but
/// letters, digits and `_` written as `_`. Too long a name is cut short and given a hash suffix.
pub fn tool_name(server: &str, tool: &str) -> String {
    let name = sanitized(server, tool);
    match name.len() <= MAX_TOOL_NAME_LENGTH {
        true => name,
        false => hashed(&name, server, tool),
    }
}

/// The names of all of one server's tools, in the order given. Tools whose names come out the
/// same once written for the model, such as `read-file` and `read_file`, all get a hash suffix,
/// so neither can be mistaken for the other.
pub fn tool_names(server: &str, tools: &[String]) -> Vec<String> {
    let sanitized: Vec<String> = tools.iter().map(|tool| sanitized(server, tool)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in &sanitized {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    sanitized
        .iter()
        .zip(tools)
        .map(
            |(name, tool)| match counts[name.as_str()] > 1 || name.len() > MAX_TOOL_NAME_LENGTH {
                true => hashed(name, server, tool),
                false => name.clone(),
            },
        )
        .collect()
}

fn sanitized(server: &str, tool: &str) -> String {
    format!("mcp{SEPARATOR}{server}{SEPARATOR}{tool}")
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || character == '_' {
                true => character,
                false => '_',
            },
        )
        .collect()
}

fn hashed(name: &str, server: &str, tool: &str) -> String {
    let digest = Sha256::digest(format!("{server}\0{tool}").as_bytes());
    let hash: String = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(HASH_LENGTH)
        .collect();
    let kept: String = name
        .chars()
        .take(MAX_TOOL_NAME_LENGTH - HASH_LENGTH - 1)
        .collect();
    format!("{kept}_{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_become_underscores_in_the_namespace_and_the_tool() {
        assert_eq!(namespace("my-server"), "mcp__my_server");
        assert_eq!(tool_name("my-server", "x"), "mcp__my_server__x");
        assert_eq!(tool_name("docs", "search.v2"), "mcp__docs__search_v2");
    }

    #[test]
    fn tools_that_would_share_a_name_all_get_a_hash() {
        let names = tool_names(
            "fs",
            &[
                "read-file".to_string(),
                "read_file".to_string(),
                "write".to_string(),
            ],
        );
        assert_ne!(names[0], names[1]);
        assert!(names[0].starts_with("mcp__fs__read_file_"), "{names:?}");
        assert!(names[1].starts_with("mcp__fs__read_file_"), "{names:?}");
        assert_eq!(names[2], "mcp__fs__write");
    }

    #[test]
    fn a_long_name_is_cut_and_hashed() {
        let name = tool_name("server", &"x".repeat(80));
        assert_eq!(name.len(), MAX_TOOL_NAME_LENGTH);
        assert_eq!(
            tool_name("server", &"x".repeat(80)),
            name,
            "the same every time"
        );
    }

    #[test]
    fn server_names_are_letters_digits_dashes_and_underscores() {
        assert!(is_valid_server_name("dev-radius_2"));
        assert!(!is_valid_server_name("has space"));
        assert!(!is_valid_server_name(""));
    }
}
