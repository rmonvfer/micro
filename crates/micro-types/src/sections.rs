//! A part of the system prompt that changed while a conversation ran, said in the conversation
//! instead of in the system prompt, so the prefix a provider cached stays as it was.

const OPEN: &str = "<system_prompt_update section=\"";
const CLOSE: &str = "</system_prompt_update>";

/// What a section update says: which section, and what it reads now, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionUpdate {
    pub name: String,
    /// `None` when the section no longer applies.
    pub body: Option<String>,
}

fn now_reads(name: &str) -> String {
    format!("The `{name}` section of the system prompt now reads:")
}

fn no_longer_applies(name: &str) -> String {
    format!("The `{name}` section of the system prompt no longer applies.")
}

/// The text of a message saying that the section `name` now reads `body`, or no longer applies.
pub fn section_update(name: &str, body: Option<&str>) -> String {
    match body {
        Some(body) => format!("{OPEN}{name}\">\n{}\n\n{body}\n{CLOSE}", now_reads(name)),
        None => format!("{OPEN}{name}\">\n{}\n{CLOSE}", no_longer_applies(name)),
    }
}

/// The section update a message's text is, if it is one.
pub fn read_section_update(text: &str) -> Option<SectionUpdate> {
    let rest = text.trim().strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    let (name, rest) = rest.split_once("\">\n")?;
    let rest = rest.trim_end_matches('\n');
    if rest == no_longer_applies(name) {
        return Some(SectionUpdate {
            name: name.to_string(),
            body: None,
        });
    }
    let body = rest.strip_prefix(&now_reads(name))?.strip_prefix("\n\n")?;
    Some(SectionUpdate {
        name: name.to_string(),
        body: Some(body.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_reads_back_as_written() {
        for body in [Some("- mcp__docs: Search the docs\n- mcp__gh"), None] {
            let text = section_update("mcp_servers", body);
            assert_eq!(
                read_section_update(&text),
                Some(SectionUpdate {
                    name: "mcp_servers".to_string(),
                    body: body.map(str::to_string),
                })
            );
        }
        assert_eq!(read_section_update("just a question"), None);
    }
}
