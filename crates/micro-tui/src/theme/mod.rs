//! Colors for the whole interface, as ohm defines them.
//!
//! Every token in ohm's theme schema is here under the name ohm gives it, so a renderer
//! asks for the token that paints a thing — `md_heading`, `tool_error_bg`, `syntax_string`
//! — rather than picking a color. The values come from ohm's `dark.json` and `light.json`
//! unchanged; see [`palette`] for the transcription.
//!
//! Three fields have no ohm token behind them, because ohm's set names no editor or status
//! surface and no user label: [`Theme::surface`], [`Theme::status`], and [`Theme::user`].
//! Each is documented where it is declared with what it borrows and why.

mod custom;
mod detect;
mod palette;

pub use custom::themes_dir;
pub use detect::ansi256_to_rgb;
pub use detect::theme_for_rgb;
pub use detect::Confidence;
pub use detect::Detection;
pub use detect::TerminalTheme;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;

/// Declares the token set once, so the struct fields, the wire names, and the lookup can
/// never drift apart.
macro_rules! tokens {
    ($($(#[$note:meta])* $field:ident => $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Theme {
            /// The theme's own name, as its file gives it.
            pub name: &'static str,
            $($(#[$note])* pub $field: Color,)*

            /// Fill behind the input editor. ohm names no editor surface, so this takes the
            /// `cardBg` its HTML export uses for the same raised-panel role.
            pub surface: Color,
            /// Fill behind the status bar. ohm names no status surface, so this takes the
            /// `pageBg` its HTML export uses for the ground everything else sits on.
            pub status: Color,
            /// The user's own label. ohm tints the whole message with `user_message_bg`
            /// instead of coloring a label, so this takes `border`, ohm's blue.
            pub user: Color,
            /// Background for a highlighted note in ohm's HTML export. No terminal region
            /// uses it yet; it is carried so the set is complete.
            pub info_bg: Color,
        }

        impl Theme {
            /// Every token name, in ohm's schema order.
            pub const TOKEN_NAMES: &'static [&'static str] = &[$($name),*];

            /// One token by its ohm name, for a caller driven by data rather than by field.
            pub fn token(&self, name: &str) -> Option<Color> {
                match name {
                    $($name => Some(self.$field),)*
                    _ => None,
                }
            }

            fn build(
                name: &'static str,
                lookup: impl Fn(&str) -> Color,
                export: impl Fn(&str) -> Color,
            ) -> Self {
                Theme {
                    name,
                    $($field: lookup($name),)*
                    surface: export("cardBg"),
                    status: export("pageBg"),
                    user: lookup("border"),
                    info_bg: export("infoBg"),
                }
            }
        }
    };
}

tokens! {
    /// Logo, selected items, cursor.
    accent => "accent",
    border => "border",
    border_accent => "borderAccent",
    border_muted => "borderMuted",
    success => "success",
    error => "error",
    warning => "warning",
    muted => "muted",
    dim => "dim",
    /// Primary body text.
    text => "text",
    thinking_text => "thinkingText",

    selected_bg => "selectedBg",
    user_message_bg => "userMessageBg",
    user_message_text => "userMessageText",
    custom_message_bg => "customMessageBg",
    custom_message_text => "customMessageText",
    custom_message_label => "customMessageLabel",
    tool_pending_bg => "toolPendingBg",
    tool_success_bg => "toolSuccessBg",
    tool_error_bg => "toolErrorBg",
    tool_title => "toolTitle",
    tool_output => "toolOutput",

    md_heading => "mdHeading",
    md_link => "mdLink",
    md_link_url => "mdLinkUrl",
    md_code => "mdCode",
    md_code_block => "mdCodeBlock",
    md_code_block_border => "mdCodeBlockBorder",
    md_quote => "mdQuote",
    md_quote_border => "mdQuoteBorder",
    md_hr => "mdHr",
    md_list_bullet => "mdListBullet",

    tool_diff_added => "toolDiffAdded",
    tool_diff_removed => "toolDiffRemoved",
    tool_diff_context => "toolDiffContext",

    syntax_comment => "syntaxComment",
    syntax_keyword => "syntaxKeyword",
    syntax_function => "syntaxFunction",
    syntax_variable => "syntaxVariable",
    syntax_string => "syntaxString",
    syntax_number => "syntaxNumber",
    syntax_type => "syntaxType",
    syntax_operator => "syntaxOperator",
    syntax_punctuation => "syntaxPunctuation",

    thinking_off => "thinkingOff",
    thinking_minimal => "thinkingMinimal",
    thinking_low => "thinkingLow",
    thinking_medium => "thinkingMedium",
    thinking_high => "thinkingHigh",
    thinking_xhigh => "thinkingXhigh",
    thinking_max => "thinkingMax",

    bash_mode => "bashMode",
}

impl Theme {
    pub fn dark() -> Self {
        Theme::build(
            "dark",
            |token| resolve(token, palette::DARK_COLORS, palette::DARK_VARS),
            |token| literal(token, palette::DARK_EXPORT),
        )
    }

    pub fn light() -> Self {
        Theme::build(
            "light",
            |token| resolve(token, palette::LIGHT_COLORS, palette::LIGHT_VARS),
            |token| literal(token, palette::LIGHT_EXPORT),
        )
    }

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Theme::dark()),
            "light" => Some(Theme::light()),
            _ => None,
        }
    }

    /// The palette to open with.
    ///
    /// `MICRO_THEME` names it. A name with a single slash is ohm's automatic form,
    /// `light-theme/dark-theme`, which picks by what the terminal looks like. With nothing
    /// set, the terminal decides on its own, falling back to dark.
    pub fn from_env() -> Self {
        Theme::resolve_setting(
            std::env::var("MICRO_THEME").ok().as_deref(),
            detect::from_env(),
        )
    }

    /// The palette a setting asks for, given what is known about the terminal. Separated
    /// from the environment so it can be tested without touching one.
    pub fn resolve_setting(setting: Option<&str>, detected: Detection) -> Self {
        let wanted = match setting.map(str::trim).filter(|value| !value.is_empty()) {
            Some(setting) => match auto_pair(setting) {
                Some((light, dark)) => match detected.theme {
                    TerminalTheme::Light => light.to_string(),
                    TerminalTheme::Dark => dark.to_string(),
                },
                None => setting.to_string(),
            },
            None => match detected.theme {
                TerminalTheme::Light => "light".to_string(),
                TerminalTheme::Dark => "dark".to_string(),
            },
        };

        Theme::named(&wanted)
            .or_else(|| Theme::from_user_file(&wanted).ok())
            // A theme that cannot be found or read is not worth failing to start over.
            .unwrap_or_else(Theme::dark)
    }

    /// A theme the user wrote, read from the themes directory.
    ///
    /// The result borrows the built-in dark theme's identity for its name, since a `Theme`
    /// holds a `&'static str`; a caller that needs the user's own name reads it from
    /// [`Theme::user_theme_name`].
    pub fn from_user_file(name: &str) -> Result<Self, String> {
        let path = custom::path_for(name).ok_or_else(|| format!("invalid theme name: {name}"))?;
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Theme::from_json(&contents)
    }

    /// A theme parsed from ohm's JSON shape.
    pub fn from_json(contents: &str) -> Result<Self, String> {
        let (_, resolved) = custom::parse(contents, Theme::TOKEN_NAMES)?;
        let lookup = |token: &str| {
            resolved
                .iter()
                .find(|(name, _)| name == token)
                .map(|(_, color)| *color)
                .unwrap_or(Color::Reset)
        };
        // A user theme carries no export block, so the surfaces keep the built-in ground.
        let fallback = Theme::dark();
        Ok(Theme {
            surface: fallback.surface,
            status: fallback.status,
            info_bg: fallback.info_bg,
            ..Theme::build("custom", lookup, |_| Color::Reset)
        })
    }

    /// The name of the theme a file declares, without loading it as a palette.
    pub fn user_theme_name(contents: &str) -> Result<String, String> {
        custom::parse(contents, Theme::TOKEN_NAMES).map(|(name, _)| name)
    }

    pub fn body(&self) -> Style {
        Style::new().fg(self.text)
    }

    pub fn dimmed(&self) -> Style {
        Style::new().fg(self.dim)
    }

    pub fn secondary(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn thinking(&self) -> Style {
        Style::new()
            .fg(self.thinking_text)
            .add_modifier(Modifier::ITALIC)
    }

    pub fn heading(&self) -> Style {
        Style::new().fg(self.text).add_modifier(Modifier::BOLD)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

/// ohm's automatic form: exactly one slash, splitting the light name from the dark one.
fn auto_pair(setting: &str) -> Option<(&str, &str)> {
    let (light, dark) = setting.split_once('/')?;
    if dark.contains('/') {
        return None;
    }
    let (light, dark) = (light.trim(), dark.trim());
    if light.is_empty() || dark.is_empty() {
        return None;
    }
    Some((light, dark))
}

/// A token from a built-in table, following the var it names.
fn resolve(token: &str, colors: &[(&str, &str)], vars: &[(&str, &str)]) -> Color {
    let value = colors
        .iter()
        .find(|(name, _)| *name == token)
        .map(|(_, value)| *value)
        .unwrap_or_else(|| panic!("built-in theme is missing {token}"));
    resolve_value(value, vars)
}

fn resolve_value(value: &str, vars: &[(&str, &str)]) -> Color {
    match value.strip_prefix('#') {
        Some(hex) => custom::parse_hex(hex)
            .unwrap_or_else(|error| panic!("built-in theme has a bad color: {error}")),
        None => {
            let next = vars
                .iter()
                .find(|(name, _)| *name == value)
                .map(|(_, value)| *value)
                .unwrap_or_else(|| panic!("built-in theme references unknown var {value}"));
            resolve_value(next, vars)
        }
    }
}

fn literal(token: &str, table: &[(&str, &str)]) -> Color {
    let value = table
        .iter()
        .find(|(name, _)| *name == token)
        .map(|(_, value)| *value)
        .unwrap_or_else(|| panic!("built-in theme is missing {token}"));
    resolve_value(value, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> Color {
        custom::parse_hex(hex.trim_start_matches('#')).unwrap()
    }

    /// Every token both themes must carry, taken from the `required` list in ohm's
    /// `theme-schema.json` plus `thinkingMax`, which the schema leaves optional but both
    /// built-in themes define.
    const SCHEMA_TOKENS: &[&str] = palette::TOKENS;

    #[test]
    fn every_schema_token_is_declared() {
        for token in SCHEMA_TOKENS {
            assert!(
                Theme::TOKEN_NAMES.contains(token),
                "{token} is in ohm's schema but not in the theme"
            );
        }
        assert_eq!(Theme::TOKEN_NAMES.len(), SCHEMA_TOKENS.len());
    }

    #[test]
    fn no_token_is_declared_twice() {
        let mut seen: Vec<&str> = Theme::TOKEN_NAMES.to_vec();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "a token name is repeated");
    }

    #[test]
    fn both_themes_define_every_token() {
        for token in SCHEMA_TOKENS {
            for (theme, table) in [
                ("dark", palette::DARK_COLORS),
                ("light", palette::LIGHT_COLORS),
            ] {
                assert!(
                    table.iter().any(|(name, _)| name == token),
                    "{theme} is missing {token}"
                );
            }
        }
    }

    #[test]
    fn neither_theme_carries_a_token_the_other_lacks() {
        fn names<'a>(table: &[(&'a str, &str)]) -> Vec<&'a str> {
            let mut names: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
            names.sort_unstable();
            names
        }
        assert_eq!(names(palette::DARK_COLORS), names(palette::LIGHT_COLORS));
        assert_eq!(names(palette::DARK_EXPORT), names(palette::LIGHT_EXPORT));

        let mut expected = palette::EXPORT_TOKENS.to_vec();
        expected.sort_unstable();
        assert_eq!(names(palette::DARK_EXPORT), expected);
    }

    #[test]
    fn every_token_resolves_to_a_literal_color() {
        for theme in [Theme::dark(), Theme::light()] {
            for token in Theme::TOKEN_NAMES {
                let color = theme.token(token).expect("every token is reachable");
                assert!(
                    matches!(color, Color::Rgb(..)),
                    "{}/{token} did not resolve to a color: {color:?}",
                    theme.name
                );
            }
        }
    }

    /// The values, spelled out against ohm's JSON. A typo in a hex digit is the failure
    /// this whole exercise is exposed to, so every token is checked rather than sampled.
    #[test]
    fn the_dark_values_match_ohm() {
        let theme = Theme::dark();
        for (token, expected) in [
            ("accent", "#8abeb7"),
            ("border", "#5f87ff"),
            ("borderAccent", "#00d7ff"),
            ("borderMuted", "#505050"),
            ("success", "#b5bd68"),
            ("error", "#cc6666"),
            ("warning", "#ffff00"),
            ("muted", "#808080"),
            ("dim", "#666666"),
            ("text", "#d4d4d4"),
            ("thinkingText", "#808080"),
            ("selectedBg", "#3a3a4a"),
            ("userMessageBg", "#343541"),
            ("userMessageText", "#d4d4d4"),
            ("customMessageBg", "#2d2838"),
            ("customMessageText", "#d4d4d4"),
            ("customMessageLabel", "#9575cd"),
            ("toolPendingBg", "#282832"),
            ("toolSuccessBg", "#283228"),
            ("toolErrorBg", "#3c2828"),
            ("toolTitle", "#d4d4d4"),
            ("toolOutput", "#808080"),
            ("mdHeading", "#f0c674"),
            ("mdLink", "#81a2be"),
            ("mdLinkUrl", "#666666"),
            ("mdCode", "#8abeb7"),
            ("mdCodeBlock", "#b5bd68"),
            ("mdCodeBlockBorder", "#808080"),
            ("mdQuote", "#808080"),
            ("mdQuoteBorder", "#808080"),
            ("mdHr", "#808080"),
            ("mdListBullet", "#8abeb7"),
            ("toolDiffAdded", "#b5bd68"),
            ("toolDiffRemoved", "#cc6666"),
            ("toolDiffContext", "#808080"),
            ("syntaxComment", "#6A9955"),
            ("syntaxKeyword", "#569CD6"),
            ("syntaxFunction", "#DCDCAA"),
            ("syntaxVariable", "#9CDCFE"),
            ("syntaxString", "#CE9178"),
            ("syntaxNumber", "#B5CEA8"),
            ("syntaxType", "#4EC9B0"),
            ("syntaxOperator", "#D4D4D4"),
            ("syntaxPunctuation", "#D4D4D4"),
            ("thinkingOff", "#505050"),
            ("thinkingMinimal", "#6e6e6e"),
            ("thinkingLow", "#5f87af"),
            ("thinkingMedium", "#81a2be"),
            ("thinkingHigh", "#b294bb"),
            ("thinkingXhigh", "#d183e8"),
            ("thinkingMax", "#ff5fff"),
            ("bashMode", "#b5bd68"),
        ] {
            assert_eq!(theme.token(token), Some(rgb(expected)), "dark/{token}");
        }
    }

    #[test]
    fn the_light_values_match_ohm() {
        let theme = Theme::light();
        for (token, expected) in [
            ("accent", "#5a8080"),
            ("border", "#547da7"),
            ("borderAccent", "#5a8080"),
            ("borderMuted", "#b0b0b0"),
            ("success", "#588458"),
            ("error", "#aa5555"),
            ("warning", "#9a7326"),
            ("muted", "#6c6c6c"),
            ("dim", "#767676"),
            ("text", "#1f2328"),
            ("thinkingText", "#6c6c6c"),
            ("selectedBg", "#d0d0e0"),
            ("userMessageBg", "#e8e8e8"),
            ("userMessageText", "#1f2328"),
            ("customMessageBg", "#ede7f6"),
            ("customMessageText", "#1f2328"),
            ("customMessageLabel", "#7e57c2"),
            ("toolPendingBg", "#e8e8f0"),
            ("toolSuccessBg", "#e8f0e8"),
            ("toolErrorBg", "#f0e8e8"),
            ("toolTitle", "#1f2328"),
            ("toolOutput", "#6c6c6c"),
            ("mdHeading", "#9a7326"),
            ("mdLink", "#547da7"),
            ("mdLinkUrl", "#767676"),
            ("mdCode", "#5a8080"),
            ("mdCodeBlock", "#588458"),
            ("mdCodeBlockBorder", "#6c6c6c"),
            ("mdQuote", "#6c6c6c"),
            ("mdQuoteBorder", "#6c6c6c"),
            ("mdHr", "#6c6c6c"),
            ("mdListBullet", "#588458"),
            ("toolDiffAdded", "#588458"),
            ("toolDiffRemoved", "#aa5555"),
            ("toolDiffContext", "#6c6c6c"),
            ("syntaxComment", "#008000"),
            ("syntaxKeyword", "#0000FF"),
            ("syntaxFunction", "#795E26"),
            ("syntaxVariable", "#001080"),
            ("syntaxString", "#A31515"),
            ("syntaxNumber", "#098658"),
            ("syntaxType", "#267F99"),
            ("syntaxOperator", "#000000"),
            ("syntaxPunctuation", "#000000"),
            ("thinkingOff", "#b0b0b0"),
            ("thinkingMinimal", "#767676"),
            ("thinkingLow", "#547da7"),
            ("thinkingMedium", "#5a8080"),
            ("thinkingHigh", "#875f87"),
            ("thinkingXhigh", "#8b008b"),
            ("thinkingMax", "#af005f"),
            ("bashMode", "#588458"),
        ] {
            assert_eq!(theme.token(token), Some(rgb(expected)), "light/{token}");
        }
    }

    #[test]
    fn the_export_surfaces_match_ohm() {
        let dark = Theme::dark();
        assert_eq!(dark.status, rgb("#18181e"));
        assert_eq!(dark.surface, rgb("#1e1e24"));
        assert_eq!(dark.info_bg, rgb("#3c3728"));

        let light = Theme::light();
        assert_eq!(light.status, rgb("#f8f8f8"));
        assert_eq!(light.surface, rgb("#ffffff"));
        assert_eq!(light.info_bg, rgb("#fffae6"));
    }

    #[test]
    fn the_user_label_borrows_the_border_color() {
        assert_eq!(Theme::dark().user, Theme::dark().border);
        assert_eq!(Theme::light().user, Theme::light().border);
    }

    #[test]
    fn a_name_selects_a_built_in_theme() {
        assert_eq!(Theme::named("dark").unwrap().name, "dark");
        assert_eq!(Theme::named("light").unwrap().name, "light");
        assert!(Theme::named("nocturne").is_none());
    }

    #[test]
    fn the_setting_names_the_theme() {
        let dark = Detection {
            theme: TerminalTheme::Dark,
            confidence: Confidence::High,
        };
        assert_eq!(Theme::resolve_setting(Some("light"), dark).name, "light");
        assert_eq!(Theme::resolve_setting(Some("dark"), dark).name, "dark");
        // An unreadable name falls back rather than refusing to start.
        assert_eq!(Theme::resolve_setting(Some("nocturne"), dark).name, "dark");
    }

    #[test]
    fn without_a_setting_the_terminal_decides() {
        let light = Detection {
            theme: TerminalTheme::Light,
            confidence: Confidence::High,
        };
        let dark = Detection {
            theme: TerminalTheme::Dark,
            confidence: Confidence::Low,
        };
        assert_eq!(Theme::resolve_setting(None, light).name, "light");
        assert_eq!(Theme::resolve_setting(None, dark).name, "dark");
        assert_eq!(Theme::resolve_setting(Some("  "), light).name, "light");
    }

    #[test]
    fn the_automatic_form_picks_a_side() {
        let light = Detection {
            theme: TerminalTheme::Light,
            confidence: Confidence::High,
        };
        let dark = Detection {
            theme: TerminalTheme::Dark,
            confidence: Confidence::High,
        };
        assert_eq!(
            Theme::resolve_setting(Some("light/dark"), light).name,
            "light"
        );
        assert_eq!(
            Theme::resolve_setting(Some("light/dark"), dark).name,
            "dark"
        );
        // Spacing around the names is ignored, as ohm ignores it.
        assert_eq!(
            Theme::resolve_setting(Some(" light / dark "), dark).name,
            "dark"
        );
    }

    #[test]
    fn a_malformed_automatic_form_is_treated_as_a_plain_name() {
        assert_eq!(auto_pair("light/dark"), Some(("light", "dark")));
        assert_eq!(auto_pair("a/b/c"), None);
        assert_eq!(auto_pair("/dark"), None);
        assert_eq!(auto_pair("light/"), None);
        assert_eq!(auto_pair("dark"), None);
    }

    #[test]
    fn a_user_theme_is_read_in_ohms_shape() {
        let mut json =
            String::from(r##"{ "name": "mine", "vars": { "brand": "#123456" }, "colors": {"##);
        for (index, token) in Theme::TOKEN_NAMES.iter().enumerate() {
            if index > 0 {
                json.push(',');
            }
            json.push_str(&format!(r##""{token}": "brand""##));
        }
        json.push_str("} }");

        let theme = Theme::from_json(&json).expect("a complete theme loads");
        for token in Theme::TOKEN_NAMES {
            assert_eq!(theme.token(token), Some(rgb("#123456")), "{token}");
        }
        assert_eq!(Theme::user_theme_name(&json).unwrap(), "mine");
        // A user theme carries no export block, so the surfaces keep the built-in ground.
        assert_eq!(theme.surface, Theme::dark().surface);
    }

    #[test]
    fn an_incomplete_user_theme_is_rejected_by_name() {
        let error = Theme::from_json(r##"{ "name": "mine", "colors": { "accent": "#000000" } }"##)
            .unwrap_err();
        assert!(error.starts_with("missing color: "), "{error}");
    }

    #[test]
    fn the_styles_the_renderers_ask_for_still_resolve() {
        let theme = Theme::dark();
        assert_eq!(theme.body().fg, Some(theme.text));
        assert_eq!(theme.dimmed().fg, Some(theme.dim));
        assert_eq!(theme.secondary().fg, Some(theme.muted));
        assert_eq!(theme.heading().fg, Some(theme.text));
        assert_eq!(theme.thinking().fg, Some(theme.thinking_text));
        assert!(theme.thinking().add_modifier.contains(Modifier::ITALIC));
        assert!(theme.heading().add_modifier.contains(Modifier::BOLD));
    }
}
