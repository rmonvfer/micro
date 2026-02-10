//! Deciding whether the terminal is light or dark, the way ohm decides it.
//!
//! ohm asks in two stages. The cheap stage reads `COLORFGBG`, which some terminals export
//! and which needs no round trip. The expensive stage queries the terminal itself with
//! OSC 11 and waits for a reply, which only the event loop can do. Both stages end at the
//! same place: convert whatever was learned to an RGB background and call it light when its
//! relative luminance reaches half.

/// A terminal background, once something has been learned about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalTheme {
    Dark,
    Light,
}

/// Where a verdict came from, and how much it is worth. `Fallback` is a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    pub theme: TerminalTheme,
    pub confidence: Confidence,
}

/// The verdict from the environment alone. Dark when nothing says otherwise, which is what
/// ohm falls back to.
pub fn from_env() -> Detection {
    from_colorfgbg(std::env::var("COLORFGBG").ok().as_deref())
}

/// `COLORFGBG` is `foreground;background` or `foreground;bold;background`, so the value that
/// matters is the last field that parses as a palette index.
pub fn from_colorfgbg(colorfgbg: Option<&str>) -> Detection {
    let index = colorfgbg.and_then(|value| {
        value
            .split(';')
            .rev()
            .find_map(|field| field.trim().parse::<u8>().ok())
    });

    match index {
        Some(index) => Detection {
            theme: theme_for_rgb(ansi256_to_rgb(index)),
            confidence: Confidence::High,
        },
        None => Detection {
            theme: TerminalTheme::Dark,
            confidence: Confidence::Low,
        },
    }
}

/// The verdict for a background the terminal reported, which is what an OSC 11 reply
/// carries. The event loop owns that exchange; this is the part that does not need one.
pub fn theme_for_rgb((r, g, b): (u8, u8, u8)) -> TerminalTheme {
    if relative_luminance(r, g, b) >= 0.5 {
        TerminalTheme::Light
    } else {
        TerminalTheme::Dark
    }
}

/// Relative luminance, sRGB gamma undone first, per the WCAG definition ohm uses.
fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    let linear = |channel: u8| {
        let value = channel as f64 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// A 256-color palette index as RGB.
///
/// The first sixteen are the terminal's own and vary by configuration; these are the
/// conventional values, which is the same approximation ohm makes. Above them the palette
/// is a 6×6×6 cube and then a 24-step gray ramp, both of which are exact.
pub fn ansi256_to_rgb(index: u8) -> (u8, u8, u8) {
    const BASIC: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0x80, 0x00, 0x00),
        (0x00, 0x80, 0x00),
        (0x80, 0x80, 0x00),
        (0x00, 0x00, 0x80),
        (0x80, 0x00, 0x80),
        (0x00, 0x80, 0x80),
        (0xc0, 0xc0, 0xc0),
        (0x80, 0x80, 0x80),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x00, 0x00, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];

    if index < 16 {
        return BASIC[index as usize];
    }
    if index < 232 {
        let cube = index as u16 - 16;
        let step = |n: u16| if n == 0 { 0 } else { (55 + n * 40) as u8 };
        return (step(cube / 36), step((cube % 36) / 6), step(cube % 6));
    }
    let gray = 8 + (index as u16 - 232) * 10;
    (gray as u8, gray as u8, gray as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dark_background_index_reads_as_dark() {
        // `15;0` is white on black, which is what a dark terminal exports.
        let detection = from_colorfgbg(Some("15;0"));
        assert_eq!(detection.theme, TerminalTheme::Dark);
        assert_eq!(detection.confidence, Confidence::High);
    }

    #[test]
    fn a_light_background_index_reads_as_light() {
        let detection = from_colorfgbg(Some("0;15"));
        assert_eq!(detection.theme, TerminalTheme::Light);
        assert_eq!(detection.confidence, Confidence::High);
    }

    #[test]
    fn the_bold_field_does_not_displace_the_background() {
        // Some terminals export `foreground;bold;background`; the background is still last.
        assert_eq!(
            from_colorfgbg(Some("0;default;15")).theme,
            TerminalTheme::Light
        );
        assert_eq!(
            from_colorfgbg(Some("15;default;0")).theme,
            TerminalTheme::Dark
        );
    }

    #[test]
    fn nothing_to_read_falls_back_to_dark_without_confidence() {
        for value in [None, Some(""), Some("nonsense"), Some(";;")] {
            let detection = from_colorfgbg(value);
            assert_eq!(detection.theme, TerminalTheme::Dark, "{value:?}");
            assert_eq!(detection.confidence, Confidence::Low, "{value:?}");
        }
    }

    #[test]
    fn a_reported_background_decides_by_luminance() {
        assert_eq!(theme_for_rgb((0xff, 0xff, 0xff)), TerminalTheme::Light);
        assert_eq!(theme_for_rgb((0x00, 0x00, 0x00)), TerminalTheme::Dark);
        // ohm's own dark page background.
        assert_eq!(theme_for_rgb((0x18, 0x18, 0x1e)), TerminalTheme::Dark);
        // ohm's own light page background.
        assert_eq!(theme_for_rgb((0xf8, 0xf8, 0xf8)), TerminalTheme::Light);
    }

    #[test]
    fn the_palette_matches_the_conversion_ohm_uses() {
        assert_eq!(ansi256_to_rgb(0), (0x00, 0x00, 0x00));
        assert_eq!(ansi256_to_rgb(15), (0xff, 0xff, 0xff));
        // First cube entry, then the last.
        assert_eq!(ansi256_to_rgb(16), (0, 0, 0));
        assert_eq!(ansi256_to_rgb(231), (0xff, 0xff, 0xff));
        // The gray ramp runs from 8 to 238 in steps of ten.
        assert_eq!(ansi256_to_rgb(232), (8, 8, 8));
        assert_eq!(ansi256_to_rgb(255), (238, 238, 238));
    }
}
