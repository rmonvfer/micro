//! Keybinding hints: the key in `dim`, what it does in `muted`.
//!
//! One shape for every hint, so the startup screen, an overlay's footer and the activity
//! line all describe a key the same way. On macOS `alt` is written `option`, because that
//! is what is printed on the key.

use crate::theme::Theme;
use ratatui::style::Style;
use ratatui::text::Span;

/// How keys are written where a user reads them.
///
/// A chord keeps its `+`, alternatives keep their `/`, and each part is renamed on its own
/// so `alt+up` becomes `option+up` without touching the `up`.
pub fn key_text(keys: &str) -> String {
    keys.split('/')
        .map(|alternative| {
            alternative
                .split('+')
                .map(rename_part)
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `alt` is `option` on a Mac keyboard, and only there.
fn rename_part(part: &str) -> String {
    match cfg!(target_os = "macos") && part.eq_ignore_ascii_case("alt") {
        true => "option".to_string(),
        false => part.to_string(),
    }
}

/// One hint: the key, a space, then what it does.
pub fn hint(keys: &str, description: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(key_text(keys), Style::new().fg(theme.dim)),
        Span::styled(format!(" {description}"), Style::new().fg(theme.muted)),
    ]
}

/// Several hints on one line, separated the way ohm separates them.
pub fn hints(pairs: &[(&str, &str)], theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, (keys, description)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::new().fg(theme.muted)));
        }
        spans.extend(hint(keys, description, theme));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_keeps_its_shape() {
        assert_eq!(key_text("ctrl+c"), "ctrl+c");
        assert_eq!(key_text("ctrl+c/ctrl+d"), "ctrl+c/ctrl+d");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn alt_is_written_option_on_a_mac() {
        assert_eq!(key_text("alt+up"), "option+up");
        assert_eq!(key_text("alt+enter/ctrl+j"), "option+enter/ctrl+j");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn alt_is_written_alt_everywhere_else() {
        assert_eq!(key_text("alt+up"), "alt+up");
    }

    #[test]
    fn only_the_modifier_is_renamed() {
        // `alt` inside a word is a word, not a modifier.
        assert_eq!(key_text("salt"), "salt");
    }

    #[test]
    fn a_hint_is_the_key_then_what_it_does() {
        let theme = Theme::dark();
        let spans = hint("ctrl+o", "expand", &theme);
        assert_eq!(spans[0].content, key_text("ctrl+o"));
        assert_eq!(spans[1].content, " expand");
        assert_eq!(spans[0].style.fg, Some(theme.dim));
        assert_eq!(spans[1].style.fg, Some(theme.muted));
    }

    #[test]
    fn several_hints_are_separated() {
        let theme = Theme::dark();
        let spans = hints(&[("a", "one"), ("b", "two")], &theme);
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text, "a one · b two");
    }
}
