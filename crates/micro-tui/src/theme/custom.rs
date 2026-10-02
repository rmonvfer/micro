//! Reading a theme written by the user.

use super::oklab;
use super::TerminalTheme;
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

/// The name reserved for the theme built from the terminal's own colors.
pub const SYSTEM: &str = "system";

/// A theme file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub name: String,
    /// The background the theme was made for, when the file says.
    pub appearance: Option<TerminalTheme>,
    pub colors: Resolved,
}

/// Parses a theme file, resolving var references and checking that every token the built-in themes
/// carry is present.
pub fn parse(contents: &str, required: &[&str]) -> Result<Parsed, String> {
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
    if name == SYSTEM {
        return Err(format!(
            "theme name {SYSTEM:?} is reserved for the terminal's own colors"
        ));
    }

    let appearance = match document.get("appearance") {
        None | Some(Value::Null) => None,
        Some(Value::String(word)) if word == "dark" => Some(TerminalTheme::Dark),
        Some(Value::String(word)) if word == "light" => Some(TerminalTheme::Light),
        Some(other) => {
            return Err(format!(
                "appearance must be \"dark\" or \"light\", found {other}"
            ))
        }
    };

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

    Ok(Parsed {
        name: name.to_string(),
        appearance,
        colors: resolved,
    })
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
    if let Some(color) = parse_function(text) {
        return color;
    }

    if !seen.insert(text.to_string()) {
        return Err(format!("circular variable reference: {text}"));
    }
    let next = vars
        .and_then(|vars| vars.get(text))
        .ok_or_else(|| format!("unknown variable: {text}"))?;
    resolve(next, vars, seen)
}

/// A three- or six-digit hex color, without its `#`.
pub(crate) fn parse_hex(hex: &str) -> Result<Color, String> {
    if !matches!(hex.len(), 3 | 6) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("#{hex} is not a three- or six-digit hex color"));
    }
    let digits: String = match hex.len() {
        3 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
        _ => hex.to_string(),
    };
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).unwrap_or_default();
    Ok(Color::Rgb(channel(0), channel(2), channel(4)))
}

/// An `oklch(L C H)` or `okhsl(H S L)` color, when `text` is written as one. Lightness and
/// saturation may be fractions or percentages, and a hue may carry `deg`.
fn parse_function(text: &str) -> Option<Result<Color, String>> {
    let lower = text.trim().to_ascii_lowercase();
    let (kind, inner) = lower
        .strip_prefix("oklch(")
        .map(|rest| ("oklch", rest))
        .or_else(|| lower.strip_prefix("okhsl(").map(|rest| ("okhsl", rest)))?;
    let Some(inner) = inner.strip_suffix(')') else {
        return Some(Err(format!("{text} is missing its closing parenthesis")));
    };
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() != 3 {
        return Some(Err(format!("{text} needs three values")));
    }
    let number = |part: &str| part.parse::<f64>().ok().filter(|value| value.is_finite());
    let fraction = |part: &str| match part.strip_suffix('%') {
        Some(percent) => number(percent).map(|value| value / 100.0),
        None => number(part),
    };
    let degrees = |part: &str| number(part.strip_suffix("deg").unwrap_or(part));
    let invalid = || Err(format!("{text} is not a valid {kind} color"));

    let (r, g, b) = match kind {
        "oklch" => {
            let (Some(lightness), Some(chroma), Some(hue)) =
                (fraction(parts[0]), number(parts[1]), degrees(parts[2]))
            else {
                return Some(invalid());
            };
            if !(0.0..=1.0).contains(&lightness) || chroma < 0.0 {
                return Some(invalid());
            }
            oklab::oklch_to_rgb(lightness, chroma, hue)
        }
        _ => {
            let (Some(hue), Some(saturation), Some(lightness)) =
                (degrees(parts[0]), fraction(parts[1]), fraction(parts[2]))
            else {
                return Some(invalid());
            };
            if !(0.0..=1.0).contains(&saturation) || !(0.0..=1.0).contains(&lightness) {
                return Some(invalid());
            }
            oklab::okhsl_to_rgb(hue, saturation, lightness)
        }
    };
    Some(Ok(Color::Rgb(r, g, b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: &[&str] = &["accent", "text"];

    #[test]
    fn a_hex_color_becomes_an_rgb_color() {
        assert_eq!(parse_hex("8abeb7").unwrap(), Color::Rgb(0x8a, 0xbe, 0xb7));
        assert_eq!(parse_hex("FFFFFF").unwrap(), Color::Rgb(255, 255, 255));
        assert_eq!(parse_hex("0af").unwrap(), Color::Rgb(0x00, 0xaa, 0xff));
        assert!(parse_hex("ffff").is_err());
        assert!(parse_hex("gggggg").is_err());
    }

    #[test]
    fn a_theme_resolves_its_variables() {
        let name = parse(
            r##"{
                "name": "mine",
                "vars": { "brand": "#8abeb7", "alias": "brand" },
                "colors": { "accent": "alias", "text": "#d4d4d4" }
            }"##,
            REQUIRED,
        )
        .unwrap();
        let (name, resolved) = (name.name, name.colors);

        assert_eq!(name, "mine");
        assert_eq!(resolved[0], ("accent".into(), Color::Rgb(0x8a, 0xbe, 0xb7)));
        assert_eq!(resolved[1], ("text".into(), Color::Rgb(0xd4, 0xd4, 0xd4)));
    }

    #[test]
    fn the_other_two_color_forms_are_understood() {
        let resolved = parse(
            r##"{ "name": "mine", "colors": { "accent": 214, "text": "" } }"##,
            REQUIRED,
        )
        .unwrap()
        .colors;

        assert_eq!(resolved[0].1, Color::Indexed(214));

        assert_eq!(resolved[1].1, Color::Reset);
    }

    #[test]
    fn perceptual_colors_are_converted_to_rgb() {
        let parsed = parse(
            r##"{
                "name": "mine",
                "colors": { "accent": "oklch(62.8% 0.2577 29.23deg)", "text": "okhsl(0 0% 100%)" }
            }"##,
            REQUIRED,
        )
        .unwrap();
        let Color::Rgb(r, g, b) = parsed.colors[0].1 else {
            panic!("an rgb color");
        };
        assert!(r > 240 && g < 20 && b < 20, "{:?}", (r, g, b));
        assert_eq!(parsed.colors[1].1, Color::Rgb(255, 255, 255));

        assert_eq!(
            parse_function("OKHSL(120 0.5 0.5)").map(|color| color.is_ok()),
            Some(true)
        );
        assert!(parse_function("oklch(1.5 0.1 20)").unwrap().is_err());
        assert!(parse_function("okhsl(10 50%)").unwrap().is_err());
        assert!(parse_function("brand").is_none());
    }

    #[test]
    fn a_theme_may_say_what_background_it_was_made_for() {
        let light = parse(
            r##"{ "name": "mine", "appearance": "light", "colors": { "accent": 1, "text": 2 } }"##,
            REQUIRED,
        )
        .unwrap();
        assert_eq!(light.appearance, Some(TerminalTheme::Light));

        let unsaid = parse(
            r##"{ "name": "mine", "colors": { "accent": 1, "text": 2 } }"##,
            REQUIRED,
        )
        .unwrap();
        assert_eq!(unsaid.appearance, None);

        assert!(parse(
            r##"{ "name": "mine", "appearance": "dim", "colors": { "accent": 1, "text": 2 } }"##,
            REQUIRED,
        )
        .is_err());
    }

    #[test]
    fn the_system_name_is_reserved() {
        let error = parse(
            r##"{ "name": "system", "colors": { "accent": 1, "text": 2 } }"##,
            REQUIRED,
        )
        .unwrap_err();
        assert!(error.contains("reserved"), "{error}");
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
