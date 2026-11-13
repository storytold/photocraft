//! Luminosity masks: Lights, Darks and Midtones at several degrees, and colour masks that can be
//! narrowed by a tonal degree.
//!
//! A luminosity mask is a grayscale mask read from the pixels' luminance `L` (Rec. 601, 0..1).
//! The degrees are the classic "stacked" masks, each one the previous multiplied by itself again,
//! so a higher degree keeps only the more extreme tones:
//!
//! * Lights *n* = `L^n`
//! * Darks *n* = `(1 − L)^n`
//! * Midtones *n* = `(4·L·(1 − L))^n`, the product of Lights and Darks scaled so mid gray is 1
//!
//! Colour masks reuse [`crate::selection::hue_range`] (the Color Range families) and may be
//! multiplied by a tonal mask, which is how "reds, lights only" is built.
//!
//! Everything works on straight RGBA `f32` samples already read from a surface, so it is
//! depth-agnostic. Fully transparent pixels give 0. Out-of-range or non-finite samples are
//! clamped, never propagated.

use crate::selection::hue_range;

/// The highest degree offered (Lights 6 keeps only near-white pixels).
pub const MAX_DEGREE: u32 = 6;

/// Which end of the tonal scale a mask follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Lights,
    Darks,
    Midtones,
}

impl Tone {
    /// Parses `lights`, `darks` or `midtones` (case-insensitive).
    pub fn parse(s: &str) -> Option<Tone> {
        match s.to_ascii_lowercase().as_str() {
            "lights" => Some(Tone::Lights),
            "darks" => Some(Tone::Darks),
            "midtones" => Some(Tone::Midtones),
            _ => None,
        }
    }
}

/// Rec. 601 luminance of a straight RGB triple, clamped to 0..1; non-finite gives 0.
#[inline]
fn luminance([r, g, b]: [f32; 3]) -> f32 {
    let l = 0.299 * r + 0.587 * g + 0.114 * b;
    if l.is_finite() { l.clamp(0.0, 1.0) } else { 0.0 }
}

/// One tonal value for a luminance `l` (0..1) at `degree` (clamped to 1..=[`MAX_DEGREE`]).
#[inline]
fn tone_value(l: f32, tone: Tone, degree: u32) -> f32 {
    let n = degree.clamp(1, MAX_DEGREE) as i32;
    let base = match tone {
        Tone::Lights => l,
        Tone::Darks => 1.0 - l,
        Tone::Midtones => 4.0 * l * (1.0 - l),
    };
    base.clamp(0.0, 1.0).powi(n)
}

/// A Lights, Darks or Midtones mask of `degree` for every pixel, 0..1.
pub fn tone_mask(px: &[[f32; 4]], tone: Tone, degree: u32) -> Vec<f32> {
    px.iter().map(|&[r, g, b, a]| if a <= 0.0 { 0.0 } else { tone_value(luminance([r, g, b]), tone, degree) }).collect()
}

/// A colour-family mask (`center` hue 0, 60, 120, 180, 240 or 300°, as Color Range's Reds to
/// Magentas), multiplied by a tonal mask when `tone` is given as `(tone, degree)`.
pub fn colour_mask(px: &[[f32; 4]], center: f32, tone: Option<(Tone, u32)>) -> Vec<f32> {
    let mut mask = hue_range(px, center);
    if let Some((t, degree)) = tone {
        for (m, &[r, g, b, a]) in mask.iter_mut().zip(px) {
            let k = if a <= 0.0 { 0.0 } else { tone_value(luminance([r, g, b]), t, degree) };
            *m = (*m * k).clamp(0.0, 1.0);
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(v: f32) -> [f32; 4] {
        [v, v, v, 1.0]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn lights_follow_luminance_and_stack_by_power() {
        let px = [gray(0.0), gray(0.5), gray(1.0)];
        let l1 = tone_mask(&px, Tone::Lights, 1);
        assert!(close(l1[0], 0.0) && close(l1[1], 0.5) && close(l1[2], 1.0));
        let l3 = tone_mask(&px, Tone::Lights, 3);
        assert!(close(l3[1], 0.125), "0.5 cubed, got {}", l3[1]);
    }

    #[test]
    fn darks_are_the_mirror_of_lights() {
        let px = [gray(0.0), gray(0.25), gray(1.0)];
        let d2 = tone_mask(&px, Tone::Darks, 2);
        assert!(close(d2[0], 1.0) && close(d2[1], 0.5625) && close(d2[2], 0.0));
    }

    #[test]
    fn midtones_peak_at_mid_gray_and_narrow_with_degree() {
        let px = [gray(0.0), gray(0.25), gray(0.5), gray(1.0)];
        let m1 = tone_mask(&px, Tone::Midtones, 1);
        assert!(close(m1[0], 0.0) && close(m1[2], 1.0) && close(m1[3], 0.0));
        assert!(close(m1[1], 0.75), "4 * 0.25 * 0.75, got {}", m1[1]);
        let m2 = tone_mask(&px, Tone::Midtones, 2);
        assert!(m2[1] < m1[1], "a higher degree is tighter");
        assert!(close(m2[2], 1.0));
    }

    #[test]
    fn degree_is_clamped_not_rejected() {
        let px = [gray(0.5)];
        assert!(close(tone_mask(&px, Tone::Lights, 0)[0], 0.5), "0 behaves as degree 1");
        let top = tone_mask(&px, Tone::Lights, 999)[0];
        assert!(close(top, 0.5f32.powi(MAX_DEGREE as i32)));
    }

    #[test]
    fn transparent_and_hostile_samples_give_zero_or_stay_in_range() {
        let px = [[1.0, 1.0, 1.0, 0.0], [f32::NAN, 0.5, 0.5, 1.0], [f32::INFINITY, 1.0, 1.0, 1.0], [9.0, 9.0, 9.0, 1.0]];
        for tone in [Tone::Lights, Tone::Darks, Tone::Midtones] {
            let m = tone_mask(&px, tone, 2);
            assert_eq!(m.len(), px.len());
            assert!(close(m[0], 0.0), "transparent is never selected");
            assert!(m.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn empty_input_gives_empty_mask() {
        assert!(tone_mask(&[], Tone::Lights, 1).is_empty());
        assert!(colour_mask(&[], 0.0, Some((Tone::Darks, 2))).is_empty());
    }

    #[test]
    fn colour_mask_is_narrowed_by_its_tone() {
        // A bright red and a dark red: both red, only the bright one is "Lights".
        let px = [[1.0, 0.0, 0.0, 1.0], [0.3, 0.0, 0.0, 1.0]];
        let plain = colour_mask(&px, 0.0, None);
        assert!(plain[0] > 0.0 && plain[1] > 0.0);
        let lights = colour_mask(&px, 0.0, Some((Tone::Lights, 2)));
        assert!(lights[0] > lights[1], "the brighter red weighs more once toned");
        assert!(lights[1] < plain[1], "toning only ever lowers the mask");
    }

    #[test]
    fn colour_mask_ignores_neutral_pixels() {
        let m = colour_mask(&[gray(0.5)], 0.0, Some((Tone::Midtones, 1)));
        assert!(close(m[0], 0.0), "gray is no colour family");
    }

    #[test]
    fn parse_names() {
        assert_eq!(Tone::parse("Lights"), Some(Tone::Lights));
        assert_eq!(Tone::parse("DARKS"), Some(Tone::Darks));
        assert_eq!(Tone::parse("midtones"), Some(Tone::Midtones));
        assert_eq!(Tone::parse("shadows"), None);
    }
}
