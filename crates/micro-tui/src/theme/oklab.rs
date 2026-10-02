//! Oklab, OKLCH, and OKHSL, to and from sRGB.
//!
//! Oklab and OKHSL are Björn Ottosson's color spaces; OKHSL's saturation is relative to the sRGB
//! gamut at each hue and lightness. This follows his reference implementation
//! (https://bottosson.github.io/posts/colorpicker/), Copyright (c) 2021 Björn Ottosson, used under
//! the MIT license: Permission is hereby granted, free of charge, to any person obtaining a copy of
//! this software and associated documentation files (the "Software"), to deal in the Software
//! without restriction, including without limitation the rights to use, copy, modify, merge,
//! publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to
//! whom the Software is furnished to do so, subject to the following conditions: The above
//! copyright notice and this permission notice shall be included in all copies or substantial
//! portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
//! EXPRESS OR IMPLIED.

/// An sRGB color, one byte per channel.
pub type Rgb = (u8, u8, u8);

type Vector = [f64; 3];
type Matrix = [Vector; 3];

const LINEAR_SRGB_TO_LMS: Matrix = [
    [0.4122214694707629, 0.5363325372617349, 0.0514459932675022],
    [0.2119034958178251, 0.6806995506452344, 0.1073969535369405],
    [0.0883024591900564, 0.2817188391361215, 0.6299787016738222],
];
const LMS_TO_LAB: Matrix = [
    [0.210454268309314, 0.793617774702305, -0.0040720430116193],
    [1.9779985324311684, -2.42859224204858, 0.450593709617411],
    [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
];
const LAB_TO_LMS: Matrix = [
    [1.0, 0.3963377773761749, 0.2158037573099136],
    [1.0, -0.1055613458156586, -0.0638541728258133],
    [1.0, -0.0894841775298119, -1.2914855480194092],
];
const LMS_TO_LINEAR_SRGB: Matrix = [
    [4.076741636075958, -3.307711539258063, 0.2309699031821043],
    [-1.2684379732850315, 2.609757349287688, -0.341319376002657],
    [-0.0041960761386756, -0.7034186179359362, 1.7076146940746117],
];

/// Per sRGB channel: the (a, b) half-plane where that channel clips first, and the polynomial
/// approximating the maximum saturation there.
const SATURATION_FIT: [([f64; 2], [f64; 5]); 3] = [
    (
        [-1.8817031, -0.80936501],
        [1.19086277, 1.76576728, 0.59662641, 0.75515197, 0.56771245],
    ),
    (
        [1.8144408, -1.19445267],
        [0.73956515, -0.45954404, 0.08285427, 0.12541073, -0.14503204],
    ),
    (
        [0.13110758, 1.81333971],
        [1.35733652, -0.00915799, -1.1513021, -0.50559606, 0.00692167],
    ),
];

const K1: f64 = 0.206;
const K2: f64 = 0.03;
const K3: f64 = (1.0 + K1) / (1.0 + K2);

fn multiply(matrix: &Matrix, [x, y, z]: Vector) -> Vector {
    matrix.map(|row| row[0] * x + row[1] * y + row[2] * z)
}

/// Oklab lightness to OKHSL lightness.
pub fn oklab_to_okhsl_lightness(x: f64) -> f64 {
    0.5 * (K3 * x - K1 + ((K3 * x - K1).powi(2) + 4.0 * K2 * K3 * x).sqrt())
}

/// OKHSL lightness to Oklab lightness.
fn okhsl_to_oklab_lightness(x: f64) -> f64 {
    (x * x + K1 * x) / (K3 * (x + K2))
}

fn linear_to_srgb(value: f64) -> f64 {
    match value > 0.0031308 {
        true => 1.055 * value.powf(1.0 / 2.4) - 0.055,
        false => 12.92 * value,
    }
}

fn srgb_to_linear(value: f64) -> f64 {
    match value <= 0.04045 {
        true => value / 12.92,
        false => ((value + 0.055) / 1.055).powf(2.4),
    }
}

/// Oklab to linear sRGB, which may lie outside the gamut.
fn oklab_to_linear_srgb(lab: Vector) -> Vector {
    multiply(
        &LMS_TO_LINEAR_SRGB,
        multiply(&LAB_TO_LMS, lab).map(|value| value.powi(3)),
    )
}

/// An sRGB color in Oklab.
pub fn rgb_to_oklab((r, g, b): Rgb) -> Vector {
    let linear = [r, g, b].map(|channel| srgb_to_linear(channel as f64 / 255.0));
    multiply(
        &LMS_TO_LAB,
        multiply(&LINEAR_SRGB_TO_LMS, linear).map(f64::cbrt),
    )
}

/// Linear sRGB to bytes, clipping whatever lies outside the gamut.
fn linear_to_rgb(linear: Vector) -> Rgb {
    let [r, g, b] =
        linear.map(|value| (linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8);
    (r, g, b)
}

/// Rate of change of each cube-root LMS component along a chroma direction.
fn lms_slopes(a: f64, b: f64) -> Vector {
    LAB_TO_LMS.map(|row| row[1] * a + row[2] * b)
}

/// The largest saturation (C/L) inside sRGB for the hue (a, b).
fn max_saturation(a: f64, b: f64) -> f64 {
    let channel = SATURATION_FIT
        .iter()
        .position(|([x, y], _)| x * a + y * b > 1.0)
        .unwrap_or(2);
    let [k0, k1, k2, k3, k4] = SATURATION_FIT[channel].1;
    let weights = LMS_TO_LINEAR_SRGB[channel];
    let saturation = k0 + k1 * a + k2 * b + k3 * a * a + k4 * a * b;

    let slopes = lms_slopes(a, b);
    let base = slopes.map(|k| 1.0 + saturation * k);
    let dot = |values: Vector| {
        (0..3)
            .map(|index| weights[index] * values[index])
            .sum::<f64>()
    };
    let f = dot(base.map(|value| value.powi(3)));
    let f1 = dot([0, 1, 2].map(|index| 3.0 * slopes[index] * base[index].powi(2)));
    let f2 = dot([0, 1, 2].map(|index| 6.0 * slopes[index].powi(2) * base[index]));
    saturation - (f * f1) / (f1 * f1 - 0.5 * f * f2)
}

/// Oklab lightness and chroma of the most saturated sRGB color of the hue (a, b).
fn cusp(a: f64, b: f64) -> (f64, f64) {
    let saturation = max_saturation(a, b);
    let [r, g, blue] = oklab_to_linear_srgb([1.0, saturation * a, saturation * b]);
    let lightness = (1.0 / r.max(g).max(blue)).cbrt();
    (lightness, lightness * saturation)
}

/// The chroma where the line of constant `lightness` leaves the sRGB gamut.
fn max_chroma(a: f64, b: f64, lightness: f64, (cusp_l, cusp_c): (f64, f64)) -> f64 {
    if lightness <= cusp_l {
        return cusp_c * lightness / cusp_l;
    }
    let t = cusp_c * (lightness - 1.0) / (cusp_l - 1.0);
    let slopes = lms_slopes(a, b);
    let lms = slopes.map(|k| lightness + t * k);
    let cubes = lms.map(|value| value.powi(3));
    let first = [0, 1, 2].map(|index| 3.0 * slopes[index] * lms[index].powi(2));
    let second = [0, 1, 2].map(|index| 6.0 * slopes[index].powi(2) * lms[index]);
    let dot =
        |row: Vector, values: Vector| row[0] * values[0] + row[1] * values[1] + row[2] * values[2];
    let step = LMS_TO_LINEAR_SRGB
        .iter()
        .map(|row| {
            let f = dot(*row, cubes) - 1.0;
            let f1 = dot(*row, first);
            let f2 = dot(*row, second);
            let u = f1 / (f1 * f1 - 0.5 * f * f2);
            match u >= 0.0 {
                true => -f * u,
                false => f64::MAX,
            }
        })
        .fold(f64::MAX, f64::min);
    t + step
}

/// OKHSL's chroma reference points at lightness `l` and hue (a, b): zero, mid, and maximum.
fn chroma_stops(l: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let peak = cusp(a, b);
    let c_max = max_chroma(a, b, l, peak);
    let k = c_max / (l * (peak.1 / peak.0)).min((1.0 - l) * (peak.1 / (1.0 - peak.0)));
    let mid_s = 0.11516993
        + 1.0
            / (7.4477897
                + 4.1590124 * b
                + a * (-2.19557347
                    + 1.75198401 * b
                    + a * (-2.13704948 - 10.02301043 * b
                        + a * (-4.24894561 + 5.38770819 * b + 4.69891013 * a))));
    let mid_t = 0.11239642
        + 1.0
            / (1.6132032 - 0.68124379 * b
                + a * (0.40370612
                    + 0.90148123 * b
                    + a * (-0.27087943
                        + 0.6122399 * b
                        + a * (0.00299215 - 0.45399568 * b - 0.14661872 * a))));
    let c_mid = 0.9
        * k
        * (1.0 / (1.0 / (l * mid_s).powi(4) + 1.0 / ((1.0 - l) * mid_t).powi(4)))
            .sqrt()
            .sqrt();
    let c0 = (1.0 / (1.0 / (l * 0.4).powi(2) + 1.0 / ((1.0 - l) * 0.8).powi(2))).sqrt();
    (c0, c_mid, c_max)
}

/// An OKHSL color in sRGB: hue in degrees, saturation and lightness from 0 to 1.
pub fn okhsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> Rgb {
    let l = okhsl_to_oklab_lightness(lightness.clamp(0.0, 1.0));
    let saturation = saturation.clamp(0.0, 1.0);
    let mut lab = [l, 0.0, 0.0];
    if l > 0.0 && l < 1.0 && saturation > 0.0 {
        let angle = hue.rem_euclid(360.0).to_radians();
        let (a, b) = (angle.cos(), angle.sin());
        let (c0, c_mid, c_max) = chroma_stops(l, a, b);
        let chroma = match saturation < 0.8 {
            true => {
                let t = 1.25 * saturation;
                let k1 = 0.8 * c0;
                t * k1 / (1.0 - (1.0 - k1 / c_mid) * t)
            }
            false => {
                let t = 5.0 * (saturation - 0.8);
                let k1 = 0.2 * c_mid.powi(2) * 1.25f64.powi(2) / c0;
                c_mid + t * k1 / (1.0 - (1.0 - k1 / (c_max - c_mid)) * t)
            }
        };
        lab = [l, chroma * a, chroma * b];
    }
    linear_to_rgb(oklab_to_linear_srgb(lab))
}

/// An sRGB color in OKHSL: hue in degrees (zero for grays), saturation and lightness from 0 to 1.
pub fn rgb_to_okhsl(rgb: Rgb) -> (f64, f64, f64) {
    let [l, a, b] = rgb_to_oklab(rgb);
    let chroma = a.hypot(b);
    let lightness = oklab_to_okhsl_lightness(l);
    if chroma < 1e-9 || lightness <= 0.0 || lightness >= 1.0 {
        return (0.0, 0.0, lightness);
    }
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    let (c0, c_mid, c_max) = chroma_stops(l, a / chroma, b / chroma);
    let saturation = match chroma < c_mid {
        true => {
            let k1 = 0.8 * c0;
            0.8 * (chroma / (k1 + (1.0 - k1 / c_mid) * chroma))
        }
        false => {
            let k1 = 0.2 * c_mid.powi(2) * 1.25f64.powi(2) / c0;
            let offset = chroma - c_mid;
            0.8 + 0.2 * (offset / (k1 + (1.0 - k1 / (c_max - c_mid)) * offset))
        }
    };
    (hue, saturation.clamp(0.0, 1.0), lightness)
}

/// An sRGB color in OKLCH: lightness from 0 to 1, chroma, and hue in degrees.
pub fn rgb_to_oklch(rgb: Rgb) -> (f64, f64, f64) {
    let [l, a, b] = rgb_to_oklab(rgb);
    (l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0))
}

/// An OKLCH color in sRGB, keeping its hue and lightness and giving up chroma until it fits the
/// gamut.
pub fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> Rgb {
    let angle = hue.rem_euclid(360.0).to_radians();
    let (cos, sin) = (angle.cos(), angle.sin());
    let at = |chroma: f64| oklab_to_linear_srgb([lightness, chroma * cos, chroma * sin]);
    let fits = |linear: &Vector| {
        linear
            .iter()
            .all(|channel| (-1e-7..=1.0 + 1e-7).contains(channel))
    };

    let direct = at(chroma);
    if fits(&direct) {
        return linear_to_rgb(direct);
    }
    let mut linear = at(0.0);
    let (mut low, mut high) = (0.0, chroma);
    for _ in 0..20 {
        let middle = (low + high) / 2.0;
        let candidate = at(middle);
        if fits(&candidate) {
            low = middle;
            linear = candidate;
        } else {
            high = middle;
        }
    }
    linear_to_rgb(linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(found: Rgb, wanted: Rgb) -> bool {
        let near = |x: u8, y: u8| (x as i16 - y as i16).abs() <= 1;
        near(found.0, wanted.0) && near(found.1, wanted.1) && near(found.2, wanted.2)
    }

    #[test]
    fn white_and_black_sit_at_the_ends_of_lightness() {
        assert!((rgb_to_oklab((255, 255, 255))[0] - 1.0).abs() < 1e-3);
        assert!(rgb_to_oklab((0, 0, 0))[0].abs() < 1e-9);
    }

    #[test]
    fn okhsl_round_trips_through_rgb() {
        for rgb in [
            (244, 184, 228),
            (30, 102, 245),
            (64, 160, 43),
            (128, 128, 128),
        ] {
            let (h, s, l) = rgb_to_okhsl(rgb);
            assert!(close(okhsl_to_rgb(h, s, l), rgb), "{rgb:?}");
        }
    }

    #[test]
    fn oklch_round_trips_through_rgb() {
        for rgb in [(244, 184, 228), (210, 15, 57), (220, 138, 120)] {
            let (l, c, h) = rgb_to_oklch(rgb);
            assert!(close(oklch_to_rgb(l, c, h), rgb), "{rgb:?}");
        }
    }

    #[test]
    fn a_color_outside_the_gamut_keeps_its_lightness_and_loses_chroma() {
        assert_eq!(oklch_to_rgb(1.0, 0.3, 150.0), (255, 255, 255));
        let (l, _, _) = rgb_to_oklch(oklch_to_rgb(0.6, 0.5, 30.0));
        assert!((l - 0.6).abs() < 0.02);
    }

    #[test]
    fn full_saturation_reaches_the_edge_of_the_gamut() {
        let red = okhsl_to_rgb(29.23, 1.0, 0.568);
        assert!(red.0 > 240 && red.1 < 20 && red.2 < 20, "{red:?}");
    }

    #[test]
    fn lightness_scales_convert_both_ways() {
        for x in [0.1, 0.5, 0.9] {
            assert!((okhsl_to_oklab_lightness(oklab_to_okhsl_lightness(x)) - x).abs() < 1e-9);
        }
    }
}
