//! Ranking tools by how well they match a query, for `searchTools()`.

use micro_types::ToolNamespace;
use serde_json::Value;
use std::collections::HashMap;

/// How many tools a search answers with when the script does not say.
pub const DEFAULT_LIMIT: usize = 8;

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on",
    "or", "that", "the", "this", "to", "with",
];

/// The text a tool is searched by.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub name: String,
    pub text: String,
}

/// The search text of a tool: its name, the name with `_` as spaces, its description, the
/// descriptions and property names of its parameters, and its namespace.
pub fn document(
    name: &str,
    description: &str,
    parameters: &Value,
    namespace: Option<&ToolNamespace>,
) -> Document {
    let mut parts = vec![
        name.to_string(),
        name.replace('_', " "),
        description.to_string(),
    ];
    schema_text(parameters, &mut parts);
    if let Some(namespace) = namespace {
        parts.push(namespace.name.clone());
        parts.extend(namespace.description.clone());
        parts.extend(namespace.instructions.clone());
    }
    Document {
        name: name.to_string(),
        text: parts
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn schema_text(schema: &Value, parts: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(description) = object.get("description").and_then(Value::as_str) {
        parts.push(description.to_string());
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            parts.push(name.clone());
            schema_text(property, parts);
        }
    }
    if let Some(items) = object.get("items") {
        schema_text(items, parts);
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        for variant in object.get(key).and_then(Value::as_array).into_iter().flatten() {
            schema_text(variant, parts);
        }
    }
}

/// A naive singular form, so `issues` matches `issue` and `searches` matches `search`.
fn stem(term: &str) -> String {
    let length = term.chars().count();
    if length > 4 && term.ends_with("ies") {
        return format!("{}y", &term[..term.len() - 3]);
    }
    if length > 4 && ["ches", "shes", "sses", "xes", "zes"].iter().any(|end| term.ends_with(end)) {
        return term[..term.len() - 2].to_string();
    }
    if length > 3 && term.ends_with('s') && !term.ends_with("ss") {
        return term[..term.len() - 1].to_string();
    }
    term.to_string()
}

/// Lowercase terms, split at camelCase boundaries and at anything that is not a letter or digit,
/// without stop words.
pub fn tokenize(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut spaced = String::with_capacity(text.len() + 8);
    for (index, &character) in characters.iter().enumerate() {
        let previous = index.checked_sub(1).map(|at| characters[at]);
        let next = characters.get(index + 1).copied();
        let lower_to_upper = character.is_ascii_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_ascii_lowercase() || previous.is_ascii_digit()
            });
        let acronym_end = character.is_ascii_uppercase()
            && previous.is_some_and(|previous| previous.is_ascii_uppercase())
            && next.is_some_and(|next| next.is_ascii_lowercase());
        if lower_to_upper || acronym_end {
            spaced.push(' ');
        }
        spaced.push(character);
    }
    spaced
        .to_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty() && !STOP_WORDS.contains(term))
        .map(stem)
        .collect()
}

/// Rank `documents` for `query` with Okapi BM25 (k1 1.2, b 0.75), best first, keeping at most
/// `limit`. Documents that match no term are left out; ties keep document order.
pub fn rank(query: &str, documents: &[Document], limit: usize) -> Vec<String> {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let mut terms: Vec<String> = Vec::new();
    for term in tokenize(query) {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() || documents.is_empty() || limit == 0 {
        return Vec::new();
    }
    let counts: Vec<HashMap<String, usize>> = documents
        .iter()
        .map(|document| {
            let mut counts = HashMap::new();
            for term in tokenize(&document.text) {
                *counts.entry(term).or_insert(0) += 1;
            }
            counts
        })
        .collect();
    let lengths: Vec<f64> = counts
        .iter()
        .map(|counts| counts.values().sum::<usize>() as f64)
        .collect();
    let average = match lengths.iter().sum::<f64>() / documents.len() as f64 {
        average if average > 0.0 => average,
        _ => 1.0,
    };
    let total = documents.len() as f64;
    let idf: Vec<f64> = terms
        .iter()
        .map(|term| {
            let frequency = counts.iter().filter(|counts| counts.contains_key(term)).count() as f64;
            (1.0 + (total - frequency + 0.5) / (frequency + 0.5)).ln()
        })
        .collect();

    let mut scored: Vec<(usize, f64)> = Vec::new();
    for (index, document_counts) in counts.iter().enumerate() {
        let mut score = 0.0;
        for (term, idf) in terms.iter().zip(&idf) {
            let Some(&count) = document_counts.get(term) else {
                continue;
            };
            let count = count as f64;
            let norm = K1 * (1.0 - B + B * lengths[index] / average);
            score += idf * (count * (K1 + 1.0)) / (count + norm);
        }
        if score > 0.0 {
            scored.push((index, score));
        }
    }
    scored.sort_by(|left, right| right.1.total_cmp(&left.1));
    scored
        .into_iter()
        .take(limit)
        .map(|(index, _)| documents[index].name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terms_split_at_case_and_punctuation_and_lose_their_plural() {
        assert_eq!(
            tokenize("listPullRequests for the HTTPServer"),
            vec!["list", "pull", "request", "http", "server"]
        );
        assert_eq!(tokenize("issues searches boxes"), vec!["issue", "search", "box"]);
    }

    #[test]
    fn the_closest_tool_ranks_first() {
        let documents = vec![
            document(
                "mcp__github__create_issue",
                "Open an issue on a repository",
                &json!({}),
                None,
            ),
            document("mcp__github__list_pulls", "List pull requests", &json!({}), None),
            document(
                "mcp__notes__append",
                "Add a line to today's note",
                &json!({ "properties": { "line": { "description": "The issue to note" } } }),
                None,
            ),
        ];
        let ranked = rank("create issues", &documents, 8);
        assert_eq!(ranked[0], "mcp__github__create_issue");
        assert!(ranked.contains(&"mcp__notes__append".to_string()));
        assert!(!ranked.contains(&"mcp__github__list_pulls".to_string()));
        assert_eq!(rank("create issues", &documents, 1).len(), 1);
        assert!(rank("the", &documents, 8).is_empty());
    }

    #[test]
    fn a_namespace_makes_its_tools_findable() {
        let namespace = ToolNamespace {
            name: "mcp__tracker".into(),
            description: Some("Bug tracker".into()),
            instructions: None,
        };
        let documents = vec![document("mcp__tracker__get", "Get one", &json!({}), Some(&namespace))];
        assert_eq!(rank("bug", &documents, 8), vec!["mcp__tracker__get"]);
    }
}
