//! The `system` theme: micro's colors derived from the terminal's own.
//!
//! Every token belongs to a color family, its hue, and has contrast rules: a contrast level it must
//! reach on the background and on the panels it is drawn on. Hue and saturation come from the
//! terminal's palette color for the family's ANSI slot, or from the family's own hue when the
//! terminal reports no palette. Lightness comes from the rules alone. Colors are built in OKHSL,
//! whose saturation is relative to the sRGB gamut, and fade toward gray near black and white. A
//! palette color never gains OKLCH chroma when it moves to another lightness, so pastel palettes
//! stay pastel.
//!
//! A contrast level is a target-lightness curve: the OKLab lightness a token needs, given the
//! lightness of the surface below it. On dark backgrounds the curves aim for nearly fixed
//! lightness; on light backgrounds the required difference grows as the background darkens.

use super::oklab;
use super::oklab::Rgb;
use super::TerminalTheme;
use std::collections::HashMap;

/// What the terminal reported about its own colors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalColors {
    /// The default foreground, from OSC 10.
    pub foreground: Option<Rgb>,
    /// The default background, from OSC 11.
    pub background: Option<Rgb>,
    /// ANSI colors 0-15, from OSC 4, when the terminal reported all of them.
    pub palette: Option<[Rgb; 16]>,
}

/// A family's OKHSL hue and saturation: `max` at mid lightness, falling toward `min` at black and
/// white.
#[derive(Debug, Clone, Copy)]
struct Family {
    hue: f64,
    min: f64,
    max: f64,
    /// The ANSI palette slot the family takes its hue and saturation from.
    slot: usize,
}

const NEUTRAL: Family = family(231.49, 0.02, 0.08, 8);
const BLUE: Family = family(231.49, 0.1, 0.68, 4);
const GREEN: Family = family(158.68, 0.1, 0.76, 2);
const RED: Family = family(20.0, 0.1, 0.92, 1);
const YELLOW: Family = family(82.36, 0.5, 1.0, 3);
const ORANGE: Family = family(52.0, 0.12, 0.85, 3);
const VIOLET: Family = family(295.0, 0.2, 0.6, 5);
const CALAMINE: Family = family(202.43, 0.1, 0.74, 6);
const THINKING_SLATE: Family = family(231.49, 0.08, 0.2, 4);
const THINKING_BLUE: Family = family(231.49, 0.2, 0.45, 4);
const THINKING_PERIWINKLE: Family = family(263.25, 0.3, 0.6, 6);
const THINKING_VIOLET: Family = family(295.0, 0.4, 0.75, 5);
const THINKING_MAGENTA: Family = family(337.5, 0.5, 0.85, 13);
const THINKING_RED: Family = family(20.0, 0.95, 1.0, 1);

const fn family(hue: f64, min: f64, max: f64, slot: usize) -> Family {
    Family {
        hue,
        min,
        max,
        slot,
    }
}

/// The family each token takes its hue from.
fn family_of(token: &str) -> Family {
    match token {
        "selectedBg" | "userMessageBg" | "border" | "mdLink" | "syntaxKeyword" => BLUE,
        "customMessageBg" | "accent" | "borderAccent" | "customMessageLabel" | "mdCode"
        | "mdListBullet" | "syntaxType" => VIOLET,
        "toolSuccessBg" | "success" | "mdCodeBlock" | "toolDiffAdded" | "bashMode"
        | "syntaxNumber" => GREEN,
        "toolErrorBg" | "error" | "toolDiffRemoved" => RED,
        "warning" | "mdHeading" | "syntaxFunction" => YELLOW,
        "syntaxString" => ORANGE,
        "syntaxVariable" => CALAMINE,
        "thinkingMinimal" => THINKING_SLATE,
        "thinkingLow" => THINKING_BLUE,
        "thinkingMedium" => THINKING_PERIWINKLE,
        "thinkingHigh" => THINKING_VIOLET,
        "thinkingXhigh" => THINKING_MAGENTA,
        "thinkingMax" => THINKING_RED,
        _ => NEUTRAL,
    }
}

/// Palette slots for tokens that would otherwise share a hue with a similar token.
fn slot_of(token: &str) -> usize {
    match token {
        "syntaxString" => 2,
        "syntaxNumber" => 5,
        other => family_of(other).slot,
    }
}

/// A target-lightness curve: a polynomial in the surface's OKLab lightness giving the OKLab
/// lightness a token needs on it, and the range of surface lightness where it can be reached.
#[derive(Debug, Clone, Copy)]
struct Curve {
    coefficients: [f64; 6],
    reachable: (f64, f64),
}

const fn curve(coefficients: [f64; 6], low: f64, high: f64) -> Curve {
    Curve {
        coefficients,
        reachable: (low, high),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Panel,
    Thinking0,
    Thinking1,
    Thinking2,
    Thinking3,
    Thinking4,
    Thinking5,
    Thinking6,
    Subtle,
    Readable,
    Emphasis,
    TextOnPanel,
    Text,
}

/// The dark and light curves of a level.
fn curves(level: Level) -> (Curve, Curve) {
    match level {
        Level::Panel => (
            curve(
                [0.29131, -0.39746, 2.33185, -0.85524, -1.2076, 0.86276],
                0.0,
                0.979,
            ),
            curve(
                [-3.74073, 27.94549, -78.44258, 112.6798, -79.60015, 22.11277],
                0.348,
                1.0,
            ),
        ),
        Level::Thinking0 => (
            curve(
                [0.52988, -0.05809, -0.30924, 4.63567, -6.52933, 2.89108],
                0.0,
                0.873,
            ),
            curve(
                [
                    -28.27749, 182.85284, -469.62416, 603.15916, -384.59976, 97.35147,
                ],
                0.51,
                1.0,
            ),
        ),
        Level::Thinking1 => (
            curve(
                [0.55278, -0.03667, -0.45659, 4.95347, -6.90265, 3.0706],
                0.0,
                0.858,
            ),
            curve(
                [
                    -37.10484, 235.86282, -596.62344, 754.3633, -474.00763, 118.3551,
                ],
                0.535,
                1.0,
            ),
        ),
        Level::Thinking2 => (
            curve(
                [0.57486, -0.01765, -0.58987, 5.25227, -7.27175, 3.25532],
                0.0,
                0.842,
            ),
            curve(
                [
                    -59.89653, 377.05024, -945.07843, 1182.03145, -734.96375, 181.68658,
                ],
                0.556,
                1.0,
            ),
        ),
        Level::Thinking3 => (
            curve(
                [0.59621, -0.00062, -0.71148, 5.53588, -7.6392, 3.44606],
                0.0,
                0.827,
            ),
            curve(
                [
                    -72.07122,
                    445.84082,
                    -1099.57352,
                    1353.88793,
                    -829.53392,
                    202.26164,
                ],
                0.58,
                1.0,
            ),
        ),
        Level::Thinking4 => (
            curve(
                [0.61691, 0.01462, -0.82288, 5.80651, -8.00641, 3.64333],
                0.0,
                0.811,
            ),
            curve(
                [
                    -110.14338,
                    674.21488,
                    -1645.75941,
                    2004.32367,
                    -1215.15899,
                    293.3183,
                ],
                0.6,
                1.0,
            ),
        ),
        Level::Thinking5 => (
            curve(
                [0.63702, 0.02826, -0.92498, 6.06465, -8.37246, 3.84651],
                0.0,
                0.795,
            ),
            curve(
                [
                    -175.47701,
                    1063.54495,
                    -2570.70594,
                    3098.80776,
                    -1860.15527,
                    444.76392,
                ],
                0.62,
                1.0,
            ),
        ),
        Level::Thinking6 => (
            curve(
                [0.65658, 0.04044, -1.01835, 6.30989, -8.73529, 4.05439],
                0.0,
                0.779,
            ),
            curve(
                [
                    -183.81712,
                    1094.70055,
                    -2602.68539,
                    3088.71276,
                    -1826.91131,
                    430.75931,
                ],
                0.643,
                1.0,
            ),
        ),
        Level::Subtle => (
            curve(
                [0.56762, -0.02475, -0.5383, 5.12628, -7.10931, 3.17324],
                0.0,
                0.848,
            ),
            curve(
                [
                    -232.85459,
                    1376.54473,
                    -3249.11801,
                    3827.91186,
                    -2248.29472,
                    526.55751,
                ],
                0.657,
                1.0,
            ),
        ),
        Level::Readable => (
            curve(
                [0.66937, 0.04704, -1.06871, 6.43941, -8.9332, 4.17229],
                0.0,
                0.77,
            ),
            curve(
                [
                    -1554.52576,
                    8733.56817,
                    -19604.93507,
                    21977.72696,
                    -12300.99599,
                    2749.81288,
                ],
                0.751,
                1.0,
            ),
        ),
        Level::Emphasis => (
            curve(
                [0.7303, 0.07695, -1.31626, 7.1681, -10.14436, 4.92846],
                0.0,
                0.712,
            ),
            curve(
                [
                    -4948.31942,
                    26870.91986,
                    -58334.48399,
                    63280.17197,
                    -34298.01053,
                    7430.30146,
                ],
                0.811,
                1.0,
            ),
        ),
        Level::TextOnPanel => (
            curve(
                [0.86713, 0.05232, -0.89428, 4.79014, -5.5432, 1.75023],
                0.0,
                0.542,
            ),
            curve(
                [
                    -8570.89457,
                    43954.60805,
                    -90084.00702,
                    92220.6791,
                    -47152.15802,
                    9632.27113,
                ],
                0.867,
                1.0,
            ),
        ),
        Level::Text => (
            curve(
                [0.89242, 0.02311, -0.44862, 2.34417, -0.06084, -2.63844],
                0.0,
                0.5,
            ),
            curve(
                [
                    -2004.67048,
                    6664.47299,
                    -6060.70202,
                    -1792.61209,
                    5133.82359,
                    -1939.85583,
                ],
                0.894,
                1.0,
            ),
        ),
    }
}

/// What a token is drawn on: the terminal's background, or another token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Surface {
    Background,
    Token(&'static str),
}

struct Rule {
    token: &'static str,
    on: Vec<Surface>,
    level: Level,
}

const PANELS: [&str; 6] = [
    "userMessageBg",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
    "selectedBg",
    "customMessageBg",
];
const TOOL_PANELS: [&str; 3] = ["toolPendingBg", "toolSuccessBg", "toolErrorBg"];
const MESSAGE_PANELS: [&str; 2] = ["userMessageBg", "customMessageBg"];
const THINKING: [(&str, Level); 7] = [
    ("thinkingOff", Level::Thinking0),
    ("thinkingMinimal", Level::Thinking1),
    ("thinkingLow", Level::Thinking2),
    ("thinkingMedium", Level::Thinking3),
    ("thinkingHigh", Level::Thinking4),
    ("thinkingXhigh", Level::Thinking5),
    ("thinkingMax", Level::Thinking6),
];
/// Text-level tokens that take the terminal's foreground.
const FOREGROUND_TOKENS: [&str; 3] = ["text", "userMessageText", "toolTitle"];
/// WCAG 2 contrast ratio body text must reach on the surfaces it is drawn on.
const TEXT_MINIMUM_WCAG_CONTRAST: f64 = 4.5;

fn surfaces(background: bool, panels: &[&[&'static str]]) -> Vec<Surface> {
    let mut on = Vec::new();
    if background {
        on.push(Surface::Background);
    }
    on.extend(
        panels
            .iter()
            .flat_map(|group| group.iter())
            .map(|token| Surface::Token(token)),
    );
    on
}

fn rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut each = |tokens: &[&'static str], on: Vec<Surface>, level: Level| {
        for token in tokens {
            rules.push(Rule {
                token,
                on: on.clone(),
                level,
            });
        }
    };
    each(&PANELS, surfaces(true, &[]), Level::Panel);
    each(&["text"], surfaces(true, &[]), Level::Text);
    each(
        &["text"],
        surfaces(false, &[&["selectedBg"]]),
        Level::TextOnPanel,
    );
    each(
        &["userMessageText"],
        surfaces(false, &[&["userMessageBg"]]),
        Level::TextOnPanel,
    );
    each(
        &["toolTitle"],
        surfaces(false, &[&TOOL_PANELS]),
        Level::TextOnPanel,
    );
    each(
        &["accent", "success", "error", "warning"],
        surfaces(true, &[&["selectedBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["muted"],
        surfaces(true, &[&["selectedBg", "customMessageBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["dim"],
        surfaces(true, &[&["selectedBg", "customMessageBg"], &TOOL_PANELS]),
        Level::Subtle,
    );
    each(&["thinkingText"], surfaces(true, &[]), Level::Readable);
    each(
        &["customMessageText"],
        surfaces(false, &[&["customMessageBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["customMessageLabel"],
        surfaces(true, &[&["customMessageBg", "selectedBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["toolOutput"],
        surfaces(true, &[&TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &[
            "mdHeading",
            "mdLink",
            "mdLinkUrl",
            "mdCode",
            "mdQuote",
            "mdCodeBlockBorder",
            "mdListBullet",
        ],
        surfaces(true, &[&MESSAGE_PANELS]),
        Level::Readable,
    );
    each(
        &["mdCodeBlock"],
        surfaces(true, &[&MESSAGE_PANELS, &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
        surfaces(true, &[&TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &[
            "syntaxComment",
            "syntaxKeyword",
            "syntaxFunction",
            "syntaxVariable",
            "syntaxString",
            "syntaxNumber",
            "syntaxType",
            "syntaxOperator",
            "syntaxPunctuation",
        ],
        surfaces(true, &[&MESSAGE_PANELS, &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["bashMode", "border", "borderAccent"],
        surfaces(true, &[]),
        Level::Readable,
    );
    each(&["borderMuted"], surfaces(true, &[]), Level::Subtle);
    each(
        &["mdQuoteBorder", "mdHr"],
        surfaces(true, &[&MESSAGE_PANELS, &TOOL_PANELS]),
        Level::Readable,
    );
    for (token, level) in THINKING {
        each(&[token], surfaces(true, &[]), level);
    }
    rules
}

/// Every token, each surface before the tokens drawn on it.
fn solve_order(rules: &[Rule]) -> Vec<&'static str> {
    fn visit(token: &'static str, rules: &[Rule], order: &mut Vec<&'static str>) {
        if order.contains(&token) {
            return;
        }
        for rule in rules.iter().filter(|rule| rule.token == token) {
            for surface in &rule.on {
                if let Surface::Token(below) = surface {
                    visit(below, rules, order);
                }
            }
        }
        order.push(token);
    }
    let mut order = Vec::new();
    for rule in rules {
        visit(rule.token, rules, &mut order);
    }
    order
}

/// OKLab lightness of an sRGB color.
fn lightness(color: Rgb) -> f64 {
    oklab::rgb_to_oklab(color)[0]
}

/// WCAG 2 relative luminance.
fn luminance((r, g, b): Rgb) -> f64 {
    let linear = |channel: u8| {
        let value = channel as f64 / 255.0;
        match value <= 0.04045 {
            true => value / 12.92,
            false => ((value + 0.055) / 1.055).powf(2.4),
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG 2 contrast ratio, 1 to 21.
fn contrast(first: Rgb, second: Rgb) -> f64 {
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

const WHITE: Rgb = (255, 255, 255);
const BLACK: Rgb = (0, 0, 0);

/// Whether a terminal is dark or light, from its reported colors: the direction of its own
/// foreground when text can be readable that way, otherwise dark when white text has more contrast
/// on the background than black text.
pub fn appearance(background: Rgb, foreground: Option<Rgb>) -> TerminalTheme {
    let white = contrast(WHITE, background);
    let black = contrast(BLACK, background);
    if let Some(foreground) = foreground {
        let (front, back) = (lightness(foreground), lightness(background));
        if (front - back).abs() > 0.05 {
            let (appearance, best) = match front > back {
                true => (TerminalTheme::Dark, white),
                false => (TerminalTheme::Light, black),
            };
            if best >= TEXT_MINIMUM_WCAG_CONTRAST {
                return appearance;
            }
        }
    }
    match white >= black {
        true => TerminalTheme::Dark,
        false => TerminalTheme::Light,
    }
}

/// Saturation weight at a lightness: a Gaussian centered on 0.5, zero at black and white.
fn bell_weight(lightness: f64) -> f64 {
    let gaussian = |x: f64| (-(x - 0.5).powi(2) / (2.0 * 0.25f64.powi(2))).exp();
    (gaussian(lightness) - gaussian(0.0)) / (1.0 - gaussian(0.0))
}

/// A family's saturation relative to its maximum: 1 at mid lightness, `min / max` at the ends.
fn saturation_curve(family: Family, lightness: f64) -> f64 {
    let floor = match family.max > 0.0 {
        true => family.min / family.max,
        false => 1.0,
    };
    floor + (1.0 - floor) * bell_weight(lightness)
}

/// A terminal color's OKHSL channels and its OKLCH chroma.
#[derive(Debug, Clone, Copy)]
struct Source {
    hue: f64,
    saturation: f64,
    lightness: f64,
    chroma: f64,
}

fn source_of(color: Rgb) -> Source {
    let (hue, saturation, lightness) = oklab::rgb_to_okhsl(color);
    Source {
        hue,
        saturation,
        lightness,
        chroma: oklab::rgb_to_oklch(color).1,
    }
}

/// A source color's hue at another OKHSL lightness. Its saturation applies at its own lightness and
/// falls off toward black and white along the family's curve; its chroma is capped at the source's
/// with the same falloff, so a pastel never turns vivid.
fn anchored(source: Source, family: Family, lightness: f64) -> Rgb {
    let anchor = saturation_curve(family, source.lightness);
    let falloff = match anchor > 0.0 {
        true => (saturation_curve(family, lightness) / anchor).min(1.0),
        false => 1.0,
    };
    let color = oklab::okhsl_to_rgb(source.hue, source.saturation * falloff, lightness);
    let cap = source.chroma * falloff;
    let (l, c, _) = oklab::rgb_to_oklch(color);
    match c <= cap {
        true => color,
        false => oklab::oklch_to_rgb(l, cap, source.hue),
    }
}

/// Move a text color toward white or black until it reaches the WCAG minimum on every surface.
fn with_text_contrast(color: Rgb, surfaces: &[Rgb], lighter: bool) -> Rgb {
    let meets = |candidate: Rgb| {
        surfaces
            .iter()
            .all(|surface| contrast(candidate, *surface) >= TEXT_MINIMUM_WCAG_CONTRAST)
    };
    if meets(color) {
        return color;
    }
    let (hue, saturation, start) = oklab::rgb_to_okhsl(color);
    let at = |l: f64| oklab::okhsl_to_rgb(hue, saturation, l);
    let extreme = if lighter { 1.0 } else { 0.0 };
    if !meets(at(extreme)) {
        return at(extreme);
    }
    let (mut low, mut high) = (start, extreme);
    for _ in 0..20 {
        let middle = (low + high) / 2.0;
        if meets(at(middle)) {
            high = middle;
        } else {
            low = middle;
        }
    }
    at(high)
}

/// The colors the system theme gives every token, by schema name, plus the surfaces micro draws
/// that the schema does not name. `None` for a token means the terminal's own default color.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub appearance: TerminalTheme,
    pub tokens: HashMap<&'static str, Option<Rgb>>,
    /// Behind the input and the open lists.
    pub surface: Rgb,
    /// Behind the status bar.
    pub status: Rgb,
    /// Behind a highlighted note.
    pub info: Rgb,
}

/// The system theme for the colors a terminal reported, or nothing when it did not report its
/// background, which every lightness is placed against.
pub fn generate(colors: &TerminalColors) -> Option<Generated> {
    let background = colors.background?;
    let foreground = colors.foreground;
    let palette = colors.palette.map(|palette| palette.map(source_of));

    let appearance = appearance(background, foreground);
    let lighter = appearance == TerminalTheme::Dark;
    let extreme = if lighter { 1.0 } else { 0.0 };
    let background_l = lightness(background);
    let readable_floor = match appearance {
        TerminalTheme::Dark => Level::Readable,
        TerminalTheme::Light => Level::Subtle,
    };

    let level_target = |level: Level, surface_l: f64| -> Option<f64> {
        let (dark, light) = curves(level);
        let curve = match appearance {
            TerminalTheme::Dark => dark,
            TerminalTheme::Light => light,
        };
        if surface_l < curve.reachable.0 || surface_l > curve.reachable.1 {
            return None;
        }
        Some(
            curve
                .coefficients
                .iter()
                .enumerate()
                .map(|(power, coefficient)| coefficient * surface_l.powi(power as i32))
                .sum(),
        )
    };

    let paint_family = |family: Family, slot: usize, oklab_l: f64| -> Rgb {
        let l = oklab::oklab_to_okhsl_lightness(oklab_l);
        match &palette {
            None => oklab::okhsl_to_rgb(
                family.hue,
                family.min + (family.max - family.min) * bell_weight(l),
                l,
            ),
            Some(palette) => anchored(palette[slot], family, l),
        }
    };
    let paint = |token: &str, oklab_l: f64| paint_family(family_of(token), slot_of(token), oklab_l);

    // The lightness a level needs on a surface, relaxed by `t`: from 0 to 1, levels stronger than
    // the readable floor move toward it; from 1 to 2, every level moves toward the surface itself.
    let target = |level: Level, surface_l: f64, t: f64| -> Option<f64> {
        let reached = level_target(level, surface_l);
        if reached.is_none() && t == 0.0 {
            return None;
        }
        let distance = reached.unwrap_or(extreme) - surface_l;
        let floor = level_target(readable_floor, surface_l).unwrap_or(extreme) - surface_l;
        let compressed = match distance.abs() > floor.abs() {
            true => distance - (distance - floor) * t.min(1.0),
            false => distance,
        };
        Some(surface_l + compressed * (1.0 - (t - 1.0).max(0.0)))
    };

    // A panel stays light or dark enough that white or black text still reaches the body text
    // minimum on it, which matters only for backgrounds near mid gray.
    let extreme_text = if lighter { WHITE } else { BLACK };
    let readable = |color: Rgb| contrast(extreme_text, color) >= TEXT_MINIMUM_WCAG_CONTRAST;
    let limit_panel = |token: &str, l: f64| -> Rgb {
        let color = paint(token, l);
        if readable(color) {
            return color;
        }
        let (mut low, mut high) = (background_l, l);
        for _ in 0..20 {
            let middle = (low + high) / 2.0;
            if readable(paint(token, middle)) {
                low = middle;
            } else {
                high = middle;
            }
        }
        paint(token, low)
    };

    let rules = rules();
    let order = solve_order(&rules);
    let color_of = |solved: &HashMap<Surface, Rgb>, surface: &Surface| {
        solved.get(surface).copied().unwrap_or(background)
    };

    let solve = |t: f64| -> Option<HashMap<Surface, Rgb>> {
        let mut solved = HashMap::from([(Surface::Background, background)]);
        for token in &order {
            let mut targets = Vec::new();
            for rule in rules.iter().filter(|rule| rule.token == *token) {
                for surface in &rule.on {
                    let value = target(rule.level, lightness(color_of(&solved, surface)), t)?;
                    if !(0.0..=1.0).contains(&value) {
                        return None;
                    }
                    targets.push(value);
                }
            }
            let l = match lighter {
                true => targets.iter().copied().fold(f64::MIN, f64::max),
                false => targets.iter().copied().fold(f64::MAX, f64::min),
            };
            let color = match PANELS.contains(token) {
                true => limit_panel(token, l),
                false => paint(token, l),
            };
            solved.insert(Surface::Token(token), color);
        }
        Some(solved)
    };

    let (solved, relaxation) = match solve(0.0) {
        Some(solved) => (solved, 0.0),
        None => {
            // Mid-gray backgrounds cannot fit every level: relax as little as possible. Full
            // relaxation always fits.
            let (mut low, mut high) = (0.0, 2.0);
            let mut best = solve(high).unwrap_or_default();
            for _ in 0..20 {
                let middle = (low + high) / 2.0;
                match solve(middle) {
                    Some(attempt) => {
                        high = middle;
                        best = attempt;
                    }
                    None => low = middle,
                }
            }
            (best, high)
        }
    };
    let surfaces_of = |token: &str| -> Vec<Rgb> {
        rules
            .iter()
            .filter(|rule| rule.token == token)
            .flat_map(|rule| rule.on.iter().map(|surface| color_of(&solved, surface)))
            .collect()
    };

    let mut tokens: HashMap<&'static str, Option<Rgb>> = order
        .iter()
        .map(|token| (*token, solved.get(&Surface::Token(token)).copied()))
        .collect();

    for token in FOREGROUND_TOKENS {
        let on = surfaces_of(token);
        let mut text = solved.get(&Surface::Token(token)).copied();
        if let Some(foreground) = foreground {
            // Body text uses the terminal's own foreground where it is clearly stronger than muted
            // text; otherwise the foreground's hue at just enough lightness.
            let targets: Option<Vec<f64>> = on
                .iter()
                .map(|surface| {
                    target(Level::Emphasis, lightness(*surface), relaxation)
                        .filter(|value| (0.0..=1.0).contains(value))
                })
                .collect();
            if let Some(targets) = targets {
                let needed = match lighter {
                    true => targets.iter().copied().fold(f64::MIN, f64::max),
                    false => targets.iter().copied().fold(f64::MAX, f64::min),
                };
                let foreground_l = lightness(foreground);
                let strong_enough = match lighter {
                    true => foreground_l >= needed,
                    false => foreground_l <= needed,
                };
                if strong_enough {
                    tokens.insert(token, None);
                    continue;
                }
                text = Some(anchored(
                    source_of(foreground),
                    NEUTRAL,
                    oklab::oklab_to_okhsl_lightness(needed),
                ));
            }
        }
        // Body text keeps at least 4.5:1 on the surfaces it is drawn on, even on relaxed mid-gray
        // backgrounds.
        if let Some(text) = text {
            tokens.insert(token, Some(with_text_contrast(text, &on, lighter)));
        }
    }

    let panel_l = target(Level::Panel, background_l, relaxation)
        .unwrap_or(background_l)
        .clamp(0.0, 1.0);
    Some(Generated {
        appearance,
        surface: solved
            .get(&Surface::Token("toolPendingBg"))
            .copied()
            .unwrap_or(background),
        status: background,
        info: paint_family(YELLOW, YELLOW.slot, panel_l),
        tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catppuccin Frappe: a dark background, a light foreground, and a pastel palette.
    fn frappe() -> TerminalColors {
        TerminalColors {
            foreground: Some((0xc6, 0xd0, 0xf5)),
            background: Some((0x30, 0x34, 0x46)),
            palette: Some([
                (0x51, 0x57, 0x6d),
                (0xe7, 0x82, 0x84),
                (0xa6, 0xd1, 0x89),
                (0xe5, 0xc8, 0x90),
                (0x8c, 0xaa, 0xee),
                (0xf4, 0xb8, 0xe4),
                (0x81, 0xc8, 0xbe),
                (0xb5, 0xbf, 0xe2),
                (0x62, 0x68, 0x80),
                (0xe7, 0x82, 0x84),
                (0xa6, 0xd1, 0x89),
                (0xe5, 0xc8, 0x90),
                (0x8c, 0xaa, 0xee),
                (0xf4, 0xb8, 0xe4),
                (0x81, 0xc8, 0xbe),
                (0xa5, 0xad, 0xce),
            ]),
        }
    }

    fn color(generated: &Generated, token: &str) -> Rgb {
        generated.tokens[token].expect("a concrete color")
    }

    #[test]
    fn nothing_is_generated_without_a_background() {
        assert!(generate(&TerminalColors::default()).is_none());
    }

    #[test]
    fn every_token_is_given_a_color() {
        let generated = generate(&frappe()).unwrap();
        for token in crate::theme::Theme::TOKEN_NAMES {
            assert!(generated.tokens.contains_key(token), "{token}");
        }
    }

    #[test]
    fn the_appearance_follows_the_background() {
        assert_eq!(appearance((0x30, 0x34, 0x46), None), TerminalTheme::Dark);
        assert_eq!(appearance((0xef, 0xf1, 0xf5), None), TerminalTheme::Light);
        assert_eq!(
            appearance((0x80, 0x80, 0x80), Some((0, 0, 0))),
            TerminalTheme::Light,
            "black text on mid gray reads as a light terminal"
        );
    }

    #[test]
    fn a_readable_foreground_is_left_to_the_terminal() {
        let generated = generate(&frappe()).unwrap();
        assert_eq!(generated.tokens["text"], None);
        assert_eq!(generated.appearance, TerminalTheme::Dark);
    }

    #[test]
    fn readable_tokens_stand_out_from_the_background() {
        for colors in [
            frappe(),
            TerminalColors {
                foreground: Some((0x4c, 0x4f, 0x69)),
                background: Some((0xef, 0xf1, 0xf5)),
                palette: None,
            },
        ] {
            let generated = generate(&colors).unwrap();
            let background = colors.background.unwrap();
            for token in ["accent", "error", "success", "warning", "muted", "mdLink"] {
                let ratio = contrast(color(&generated, token), background);
                assert!(ratio >= 3.0, "{token} has only {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn body_text_reaches_the_minimum_contrast_on_its_panels() {
        let mut colors = frappe();
        colors.foreground = Some((0x70, 0x70, 0x70));
        let generated = generate(&colors).unwrap();
        let text = color(&generated, "userMessageText");
        let panel = color(&generated, "userMessageBg");
        assert!(contrast(text, panel) >= TEXT_MINIMUM_WCAG_CONTRAST);
    }

    #[test]
    fn a_pastel_palette_keeps_its_chroma() {
        let generated = generate(&frappe()).unwrap();
        let pink_chroma = oklab::rgb_to_oklch((0xf4, 0xb8, 0xe4)).1;
        let accent_chroma = oklab::rgb_to_oklch(color(&generated, "accent")).1;
        assert!(
            accent_chroma <= pink_chroma + 0.005,
            "the accent took chroma {accent_chroma} from a pastel of {pink_chroma}"
        );
    }

    #[test]
    fn a_hue_comes_from_the_palette_slot_of_its_family() {
        let mut colors = frappe();
        let palette = colors.palette.as_mut().unwrap();
        palette[1] = (0x00, 0x80, 0xff);
        let generated = generate(&colors).unwrap();
        let (_, _, hue) = oklab::rgb_to_oklch(color(&generated, "error"));
        let (_, _, wanted) = oklab::rgb_to_oklch((0x00, 0x80, 0xff));
        assert!(
            (hue - wanted).abs() < 5.0,
            "error hue {hue}, palette red {wanted}"
        );
    }

    #[test]
    fn a_mid_gray_background_still_gets_a_theme() {
        let generated = generate(&TerminalColors {
            foreground: None,
            background: Some((0x77, 0x77, 0x77)),
            palette: None,
        })
        .unwrap();
        assert!(generated.tokens["accent"].is_some());
    }
}
