//! What a registered tool contributes to the system prompt, once it is actually offered to
//! the model.
//!
//! micro's own system prompt is a fixed paragraph with the project's instructions and a
//! skills section appended — there is no "Available tools" listing built into it the way
//! pi's is, because micro has no fixed roster of selectable built-in tools to describe that
//! way. An extension can still ask for one: a tool that sets `promptSnippet` earns a line
//! in an `Available tools` section, and every `promptGuideline` it sets is folded into a
//! `Guidelines` section, mirroring the two fields pi's own tools use for the same purpose.

use crate::host::RegisteredTool;
use std::collections::HashSet;

/// The text an extension's tools add to the system prompt, or nothing when none of the
/// tools the model was actually given asked for a line.
///
/// `active` names every tool offered to the model this run — after `--tools` and
/// `--exclude-tools` narrowed it down — so a tool left out of the run does not leave its
/// snippet or guidelines behind in the prompt anyway. A tool without a snippet is left out
/// of `Available tools` entirely: an extension with nothing worth summarising should not
/// clutter the list, which is the whole point of the field being optional.
pub fn prompt_section(tools: &[RegisteredTool], active: &[String]) -> Option<String> {
    let active: HashSet<&str> = active.iter().map(String::as_str).collect();
    let offered = tools.iter().filter(|tool| active.contains(tool.name.as_str()));

    let mut snippets = Vec::new();
    let mut guidelines = Vec::new();
    let mut seen_guidelines = HashSet::new();

    for tool in offered {
        if let Some(snippet) = tool
            .prompt_snippet
            .as_deref()
            .map(str::trim)
            .filter(|snippet| !snippet.is_empty())
        {
            snippets.push(format!("- {}: {snippet}", tool.name));
        }
        for guideline in &tool.prompt_guidelines {
            let normalized = guideline.trim();
            if !normalized.is_empty() && seen_guidelines.insert(normalized) {
                guidelines.push(format!("- {normalized}"));
            }
        }
    }

    let mut section = String::new();
    if !snippets.is_empty() {
        section.push_str("Available tools:\n");
        section.push_str(&snippets.join("\n"));
    }
    if !guidelines.is_empty() {
        if !section.is_empty() {
            section.push_str("\n\n");
        }
        section.push_str("Guidelines:\n");
        section.push_str(&guidelines.join("\n"));
    }

    (!section.is_empty()).then_some(section)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built the way the host actually describes a registration: snake_case keys, since
    /// that is what crosses the wire, with everything the test does not care about left to
    /// its default.
    fn tool(json: serde_json::Value) -> RegisteredTool {
        serde_json::from_value(json).expect("a registered tool")
    }

    #[test]
    fn a_tool_with_a_snippet_appears() {
        let tools = vec![tool(serde_json::json!({
            "name": "deploy",
            "prompt_snippet": "ships the current branch",
        }))];
        let section = prompt_section(&tools, &["deploy".to_string()]).expect("a section");
        assert!(section.contains("Available tools:"));
        assert!(section.contains("- deploy: ships the current branch"));
    }

    #[test]
    fn a_tool_without_a_snippet_does_not_appear() {
        let tools = vec![tool(serde_json::json!({ "name": "deploy" }))];
        let section = prompt_section(&tools, &["deploy".to_string()]);
        assert!(section.is_none(), "{section:?}");
    }

    #[test]
    fn guidelines_appear_only_for_active_tools() {
        let tools = vec![
            tool(serde_json::json!({
                "name": "deploy",
                "prompt_guidelines": ["Confirm the target environment before shipping"],
            })),
            tool(serde_json::json!({
                "name": "rollback",
                "prompt_guidelines": ["Never roll back without a snapshot"],
            })),
        ];
        // Only "deploy" was actually offered to the model this run.
        let section = prompt_section(&tools, &["deploy".to_string()]).expect("a section");
        assert!(section.contains("Confirm the target environment before shipping"));
        assert!(!section.contains("Never roll back without a snapshot"));
    }

    #[test]
    fn nothing_contributing_leaves_no_stray_heading() {
        let tools = vec![tool(serde_json::json!({ "name": "deploy" }))];
        // Not even offered, so neither a missing snippet nor an absent guideline matters.
        assert!(prompt_section(&tools, &[]).is_none());
        assert!(prompt_section(&[], &["deploy".to_string()]).is_none());
    }

    #[test]
    fn a_blank_snippet_or_guideline_is_treated_as_absent() {
        let tools = vec![tool(serde_json::json!({
            "name": "deploy",
            "prompt_snippet": "   ",
            "prompt_guidelines": ["  ", ""],
        }))];
        assert!(prompt_section(&tools, &["deploy".to_string()]).is_none());
    }

    #[test]
    fn a_duplicate_guideline_across_tools_is_said_once() {
        let tools = vec![
            tool(serde_json::json!({
                "name": "deploy",
                "prompt_guidelines": ["Ask before touching production"],
            })),
            tool(serde_json::json!({
                "name": "rollback",
                "prompt_guidelines": ["Ask before touching production"],
            })),
        ];
        let section =
            prompt_section(&tools, &["deploy".to_string(), "rollback".to_string()]).expect("a section");
        assert_eq!(section.matches("Ask before touching production").count(), 1);
    }

    #[test]
    fn both_sections_are_separated_by_a_blank_line() {
        let tools = vec![tool(serde_json::json!({
            "name": "deploy",
            "prompt_snippet": "ships the current branch",
            "prompt_guidelines": ["Confirm the target environment before shipping"],
        }))];
        let section = prompt_section(&tools, &["deploy".to_string()]).expect("a section");
        assert_eq!(
            section,
            "Available tools:\n- deploy: ships the current branch\n\n\
             Guidelines:\n- Confirm the target environment before shipping"
        );
    }
}
