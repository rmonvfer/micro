//! Settings named on the command line.

use crate::ConfigError;
use crate::Result;
use serde_json::Map;
use serde_json::Value;

/// A setting an assignment wrote, and the text that wrote it.
pub struct Written {
    
    pub key: String,
    /// The `key=value` text as it was given.
    pub assignment: String,
}

/// Apply every assignment to a config value, in the order they were given.
pub fn apply_all(target: &mut Value, assignments: &[String]) -> Result<Vec<Written>> {
    assignments
        .iter()
        .map(|assignment| {
            apply(target, assignment).map(|key| Written {
                key,
                assignment: assignment.clone(),
            })
        })
        .collect()
}

/// Apply one `key=value` assignment, and say which top-level setting it reached.
pub fn apply(target: &mut Value, assignment: &str) -> Result<String> {
    
    let (key, raw) = assignment
        .split_once('=')
        .ok_or_else(|| malformed(assignment, "expected key=value"))?;

    let segments = segments(key, assignment)?;
    place(target, &segments, read(raw), assignment)?;
    Ok(segments[0].clone())
}

/// The parts of a dotted key, checked for the shapes that cannot mean anything.
fn segments(key: &str, assignment: &str) -> Result<Vec<String>> {
    let key = key.trim();
    if key.is_empty() {
        return Err(malformed(assignment, "the key is empty"));
    }

    let segments: Vec<String> = key.split('.').map(str::to_string).collect();
    if segments.iter().any(String::is_empty) {
        return Err(malformed(
            assignment,
            "the key has an empty segment; write it as `one.two`",
        ));
    }
    Ok(segments)
}

/// Read a value as JSON, falling back to the text itself.
fn read(raw: &str) -> Value {
    let trimmed = raw.trim();
    match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => value,
        Err(_) => Value::String(raw.to_string()),
    }
}

/// Write `value` at `segments`, creating the objects on the way to it.
fn place(target: &mut Value, segments: &[String], value: Value, assignment: &str) -> Result<()> {
    
    if !target.is_object() {
        return Err(malformed(assignment, "the config is not a JSON object"));
    }

    let (last, leading) = segments
        .split_last()
        .expect("a key has at least one segment");

    let mut current = target;
    for segment in leading {
        let object = current
            .as_object_mut()
            .ok_or_else(|| occupied(assignment, segment))?;
        current = object
            .entry(segment.as_str())
            .or_insert_with(|| Value::Object(Map::new()));
    }

    current
        .as_object_mut()
        .ok_or_else(|| occupied(assignment, last))?
        .insert(last.clone(), value);
    Ok(())
}

fn malformed(assignment: &str, message: &str) -> ConfigError {
    ConfigError::Override {
        assignment: assignment.to_string(),
        message: message.to_string(),
    }
}

/// A key whose parent is already a plain value cannot be written without throwing that value away.
fn occupied(assignment: &str, segment: &str) -> ConfigError {
    ConfigError::Override {
        assignment: assignment.to_string(),
        message: format!("`{segment}` is already a value, so it cannot hold nested keys"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn applied(start: Value, assignments: &[&str]) -> Value {
        let mut value = start;
        let assignments: Vec<String> = assignments.iter().map(|a| a.to_string()).collect();
        apply_all(&mut value, &assignments).expect("the assignments are good");
        value
    }

    #[test]
    fn a_bare_word_is_the_string_it_looks_like() {
        assert_eq!(
            applied(json!({}), &["theme=dracula"]),
            json!({ "theme": "dracula" })
        );
    }

    #[test]
    fn json_values_keep_their_types() {
        assert_eq!(
            applied(json!({}), &["show_images=false", "image_width_cells=40"]),
            json!({ "show_images": false, "image_width_cells": 40 })
        );
    }

    #[test]
    fn a_dotted_key_reaches_a_nested_setting() {
        assert_eq!(
            applied(json!({}), &["a.b.c=1"]),
            json!({ "a": { "b": { "c": 1 } } })
        );
    }

    /// Writing one nested key leaves its siblings alone.
    #[test]
    fn a_nested_write_keeps_what_was_beside_it() {
        assert_eq!(
            applied(json!({ "a": { "keep": true } }), &["a.add=1"]),
            json!({ "a": { "keep": true, "add": 1 } })
        );
    }

    #[test]
    fn a_later_assignment_wins() {
        assert_eq!(
            applied(json!({}), &["theme=one", "theme=two"]),
            json!({ "theme": "two" })
        );
    }

    /// Only the first `=` separates, so a value may contain more of them.
    #[test]
    fn a_value_may_contain_an_equals_sign() {
        assert_eq!(applied(json!({}), &["flag=a=b"]), json!({ "flag": "a=b" }));
    }

    #[test]
    fn an_assignment_without_an_equals_sign_is_refused() {
        let mut value = json!({});
        let error = apply(&mut value, "theme").unwrap_err().to_string();
        assert!(error.contains("expected key=value"), "{error}");
    }

    #[test]
    fn an_empty_key_is_refused() {
        let mut value = json!({});
        assert!(apply(&mut value, "=dracula").is_err());
        assert!(apply(&mut value, "a..b=1").is_err());
    }

    /// A key cannot be nested under a setting that already holds a plain value, because writing it
    /// would silently discard that value.
    #[test]
    fn a_key_under_a_plain_value_is_refused() {
        let mut value = json!({ "theme": "dark" });
        let error = apply(&mut value, "theme.name=x").unwrap_err().to_string();
        assert!(error.contains("already a value"), "{error}");
        assert_eq!(value, json!({ "theme": "dark" }), "nothing was written");
    }
}
