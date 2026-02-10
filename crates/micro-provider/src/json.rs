//! Small readers for the loosely-typed JSON that streaming APIs send.

use serde_json::Map;
use serde_json::Value;

pub(crate) fn read_str(value: Option<&Value>, key: &str) -> String {
    value
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn read_u32(value: &Value, key: &str) -> u32 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0) as u32
}

/// Tool arguments arrive as streamed JSON text. An empty body means "no arguments";
/// anything unparseable is surfaced as an empty object so the call still round-trips.
pub(crate) fn parse_arguments(json: &str) -> Value {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Value::Object(Map::new());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::Object(Map::new()))
}
