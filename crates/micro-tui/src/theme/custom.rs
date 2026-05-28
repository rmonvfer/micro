//! Reading a theme written by the user.

use ratatui::style::Color;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

/// What the directory of user themes is called.
pub const THEMES_DIR: &str = "themes";

/// Where a user's own themes live: the `themes` directory of micro's configuration directory, since
/// a theme is something they wrote.
pub fn themes_dir() -> Option<PathBuf> {
    micro_dirs::config_dir().map(|dir| dir.join(THEMES_DIR))
}

/// The path a named user theme would live at.
pub fn path_for(name: &str) -> Option<PathBuf> {
    
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return None;
    }
    themes_dir().map(|dir| dir.join(format!("{name}.json")))
}

/// Every token a theme file resolved to, by its schema name.
pub type Resolved = Vec<(String, Color)>;

/// Parses a theme file, resolving var references and checking that every token the built-in themes
/// carry is present.
pub fn parse(contents: &str, required: &[&str]) -> Result<(String, Resolved), String> {
    let document: Value =
        serde_json::from_str(contents).map_err(|error| format!("not valid JSON: {error}"))?;

    let name = document
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "a theme needs a name".to_string())?;
    if name.contains('/') {
        return Err(format!(
            "theme name {name:?} contains '/', which is reserved for the light/dark form"
        ));
    }

    let vars = document.get("vars").and_then(Value::as_object);
    let colors = document
        .get("colors")
        .and_then(Value::as_object)
        .ok_or_else(|| "a theme needs a colors block".to_string())?;

    let mut resolved = Vec::with_capacity(required.len());
    for token in required {
        let value = colors
            .get(*token)
            .ok_or_else(|| format!("missing color: {token}"))?;
        let color = resolve(value, vars, &mut HashSet::new())
            .map_err(|error| format!("{token}: {error}"))?;
        resolved.push(((*token).to_string(), color));
    }

    Ok((name.to_string(), resolved))
}

/// Follows a value to the color it names.
fn resolve(
    value: &Value,
    vars: Option<&serde_json::Map<String, Value>>,
    seen: &mut HashSet<String>,
) -> Result<Color, String> {
    if let Some(index) = value.as_u64() {
        return u8::try_from(index)
            .map(Color::Indexed)
            .map_err(|_| format!("color index {index} is outside 0-255"));
    }

    let Some(text) = value.as_str() else {
        return Err(format!("expected a color, found {value}"));
    };
    
    if text.is_empty() {
        return Ok(Color::Reset);
    }
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }

    if !seen.insert(text.to_string()) {
        return Err(format!("circular variable reference: {text}"));
    }
    let next = vars
        .and_then(|vars| vars.get(text))
        .ok_or_else(|| format!("unknown variable: {text}"))?;
    resolve(next, vars, seen)
}

pub(crate) fn parse_hex(hex: &str) -> Result<Color, String> {
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("#{hex} is not a six-digit hex color"));
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or_default();
    Ok(Color::Rgb(channel(0), channel(2), channel(4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: &[&str] = &["accent", "text"];

    #[test]
    fn a_hex_color_becomes_an_rgb_color() {
        assert_eq!(parse_hex("8abeb7").unwrap(), Color::Rgb(0x8a, 0xbe, 0xb7));
        assert_eq!(parse_hex("FFFFFF").unwrap(), Color::Rgb(255, 255, 255));
        assert!(parse_hex("fff").is_err());
        assert!(parse_hex("gggggg").is_err());
    }

    #[test]
    fn a_theme_resolves_its_variables() {
        let (name, resolved) = parse(
            r##"{
                "name": "mine",
                "vars": { "brand": "#8abeb7", "alias": "brand" },
                "colors": { "accent": "alias", "text": "#d4d4d4" }
            }"##,
            REQUIRED,
        )
        .unwrap();

        assert_eq!(name, "mine");
        assert_eq!(resolved[0], ("accent".into(), Color::Rgb(0x8a, 0xbe, 0xb7)));
        assert_eq!(resolved[1], ("text".into(), Color::Rgb(0xd4, 0xd4, 0xd4)));
    }

    #[test]
    fn the_other_two_color_forms_are_understood() {
        let (_, resolved) = parse(
            r##"{ "name": "mine", "colors": { "accent": 214, "text": "" } }"##,
            REQUIRED,
        )
        .unwrap();

        assert_eq!(resolved[0].1, Color::Indexed(214));
        
        assert_eq!(resolved[1].1, Color::Reset);
    }

    #[test]
    fn a_circular_variable_is_reported_rather_than_followed() {
        let error = parse(
            r##"{
                "name": "mine",
                "vars": { "a": "b", "b": "a" },
                "colors": { "accent": "a", "text": "#000000" }
            }"##,
            REQUIRED,
        )
        .unwrap_err();
        assert!(error.contains("circular"), "{error}");
    }

    #[test]
    fn a_missing_token_names_itself() {
        let error = parse(
            r##"{ "name": "mine", "colors": { "accent": "#000000" } }"##,
            REQUIRED,
        )
        .unwrap_err();
        assert!(error.contains("missing color: text"), "{error}");
    }

    #[test]
    fn an_unknown_variable_names_itself() {
        let error = parse(
            r##"{ "name": "mine", "colors": { "accent": "nope", "text": "#000000" } }"##,
            REQUIRED,
        )
        .unwrap_err();
        assert!(error.contains("unknown variable: nope"), "{error}");
    }

    #[test]
    fn a_name_with_a_slash_is_refused() {
        let error = parse(
            r##"{ "name": "a/b", "colors": { "accent": "#000000", "text": "#000000" } }"##,
            REQUIRED,
        )
        .unwrap_err();
        assert!(error.contains("reserved"), "{error}");
    }

    #[test]
    fn a_theme_name_cannot_climb_out_of_the_themes_directory() {
        assert!(path_for("../../etc/passwd").is_none());
        assert!(path_for("a/b").is_none());
        assert!(path_for("").is_none());
    }

    #[test]
    fn malformed_json_is_reported_rather_than_panicking() {
        assert!(parse("{ not json", REQUIRED).is_err());
        assert!(parse(r##"{ "colors": {} }"##, REQUIRED).is_err());
        assert!(parse(r##"{ "name": "mine" }"##, REQUIRED).is_err());
    }
}
