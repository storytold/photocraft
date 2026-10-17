//! Display of Multichannel documents: every spot (ink) channel printed over white paper.
//!
//! Ink density `v` (the channel value, 1 = solid) of an ink with display colour `c` and solidity
//! `s` turns the colour underneath `p` into `(1 − s·v)·p·(1 − v + v·c) + s·v·c`: transparent inks
//! (solidity 0) multiply like overprinted process inks, opaque ones cover what is below, which is
//! how Photoshop previews spot channels. An alpha-type channel of a Multichannel document
//! (selected / masked areas) still prints, as a transparent black ink, like Photoshop shows it.
//!
//! Like Photoshop, the preview adds the Spot working space's dot gain (Edit › Color Settings ›
//! Spot, [`set_spot_dot_gain`]): a 50 % dot shows as 50 % plus the gain, as it prints on press.

use photocraft_doc::Document;
use photocraft_geom::Rect;

use crate::Buffer;

/// Photoshop's default Spot working space: Dot Gain 20 %.
pub const SPOT_DOT_GAIN: f32 = 20.0;
/// The largest dot gain the setting takes: a 50 % dot at 99 % (a gain of 50 % or more would
/// print every midtone solid).
pub const MAX_SPOT_DOT_GAIN: f32 = 49.0;

static SPOT_DOT_GAIN_SETTING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x41a0_0000); // 20.0f32

/// Sets Color Settings › Spot: the dot gain, in percent at a 50 % dot, that the ink preview adds
/// to every spot channel. It is an application setting, not stored in documents; values are
/// clamped to 0–[`MAX_SPOT_DOT_GAIN`].
///
/// The value is process-wide: a test that changes it races every concurrently running test that
/// renders a Multichannel document, so such tests belong in their own test binary (see
/// `crates/compose/tests/spot_dot_gain.rs`).
pub fn set_spot_dot_gain(percent: f32) {
    let g = if percent.is_finite() { percent.clamp(0.0, MAX_SPOT_DOT_GAIN) } else { SPOT_DOT_GAIN };
    SPOT_DOT_GAIN_SETTING.store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The current Spot dot gain in percent (see [`set_spot_dot_gain`]).
pub fn spot_dot_gain() -> f32 {
    f32::from_bits(SPOT_DOT_GAIN_SETTING.load(std::sync::atomic::Ordering::Relaxed))
}

/// The ink coverage that prints for channel value `v` (0..1) with a dot gain of `percent` at a
/// 50 % dot: `1 − (1 − v)^γ`, with γ chosen so a 50 % dot prints at 50 % + `percent`. Solids and
/// paper stay as they are. This is the curve family of the built-in "Dot Gain 20 %-compatible"
/// gray (a gamma fitted on Photoshop's Dot Gain 20 % profile; 20 % here gives γ ≈ 1.737).
pub fn dot_gain(v: f32, percent: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let g = if percent.is_finite() { percent.clamp(0.0, MAX_SPOT_DOT_GAIN) / 100.0 } else { 0.0 };
    if g <= 0.0 {
        return v;
    }
    let gamma = (0.5 - g).ln() / 0.5f32.ln();
    1.0 - (1.0 - v).powf(gamma)
}

/// The printed inks of `doc` over `rect` (opaque).
pub fn inks(doc: &Document, rect: Rect) -> Buffer {
    let mut buf = Buffer::filled(rect, [1.0, 1.0, 1.0, 1.0]);
    let mut vals = Vec::new();
    let gain = spot_dot_gain();
    for ch in &doc.channels {
        let (ink, solidity) = ch.spot.unwrap_or((photocraft_color::Color::BLACK, 0.0));
        if ch.surface.content_bounds().intersect(&rect).is_empty() {
            continue;
        }
        let c = ink.to_rgb();
        let s = solidity.clamp(0.0, 1.0);
        let n = ch.surface.channels();
        ch.surface.read_region_into(rect, &mut vals);
        for (p, v) in buf.px.iter_mut().zip(vals.chunks_exact(n)) {
            let v = v[0].clamp(0.0, 1.0);
            if v <= 0.0 {
                continue;
            }
            let v = dot_gain(v, gain);
            let keep = 1.0 - s * v;
            for j in 0..3 {
                p[j] = keep * p[j] * (1.0 - v + v * c[j]) + s * v * c[j];
            }
        }
    }
    buf
}

/// The starting backdrop of a document render: the inks for Multichannel, else transparent.
pub(crate) fn backdrop(doc: &Document, rect: Rect) -> Buffer {
    if doc.mode == photocraft_color::ColorMode::Multichannel { inks(doc, rect) } else { Buffer::transparent(rect) }
}

#[cfg(test)]
mod tests {
    use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Document, Size};
    use photocraft_raster::Surface;

    use super::*;

    #[test]
    fn inks_multiply_over_paper() {
        let mut doc = Document::new("m", Size::new(4, 1), ColorMode::Multichannel, SampleType::U8);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mut a = Surface::new(fmt);
        a.write_region(Rect::new(0, 0, 4, 1), &[0.0, 1.0, 1.0, 0.0]);
        let mut b = Surface::new(fmt);
        b.write_region(Rect::new(0, 0, 4, 1), &[0.0, 0.0, 1.0, 1.0]);
        doc.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 1.0, 1.0), 0.0)), ..AlphaChannel::new("Cyan", a) });
        doc.channels.push(AlphaChannel { spot: Some((Color::rgb(1.0, 0.0, 1.0), 0.0)), ..AlphaChannel::new("Magenta", b) });
        let r = crate::flatten(&doc);
        assert_eq!(r.px[0], [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(r.px[1], [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(r.px[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(r.px[3], [1.0, 0.0, 1.0, 1.0]);
        // An alpha-type channel prints as a black ink.
        let mut k = Surface::new(fmt);
        k.write_region(Rect::new(0, 0, 4, 1), &[0.5, 0.0, 0.0, 0.0]);
        doc.channels.push(AlphaChannel::new("Alpha", k));
        let r = crate::flatten(&doc);
        // 0.5 is stored as 128/255 in an 8-bit channel; the preview adds the Spot dot gain.
        let paper = 1.0 - dot_gain(128.0 / 255.0, spot_dot_gain());
        assert!((r.px[0][0] - paper).abs() <= 1e-4 && (r.px[0][1] - paper).abs() <= 1e-4, "{:?}", r.px[0]);
        assert_eq!(r.px[1], [0.0, 1.0, 1.0, 1.0]);
        // Other modes have no inks.
        doc.mode = ColorMode::Rgb;
        assert_eq!(crate::flatten(&doc).px[1][3], 0.0);
    }

    #[test]
    fn dot_gain_lifts_a_50_percent_dot_by_the_gain_and_keeps_paper_and_solids() {
        for gain in [10.0, 15.0, 20.0, 25.0, 30.0, 38.0, MAX_SPOT_DOT_GAIN] {
            assert!((dot_gain(0.5, gain) - (0.5 + gain / 100.0)).abs() < 1e-5, "{gain}");
            assert_eq!(dot_gain(0.0, gain), 0.0);
            assert!((dot_gain(1.0, gain) - 1.0).abs() < 1e-6);
            let mut last = 0.0;
            for i in 1..=100 {
                let v = dot_gain(i as f32 / 100.0, gain);
                assert!(v >= last && v >= i as f32 / 100.0, "{gain}: rises and never lightens");
                last = v;
            }
        }
        assert_eq!(dot_gain(0.3, 0.0), 0.3, "no gain is linear");
        assert_eq!(dot_gain(0.3, f32::NAN), 0.3);
        // The same curve family as the built-in Dot Gain 20 %-compatible gray (gamma 1.732).
        assert!((dot_gain(0.5, 20.0) - (1.0 - 0.5f32.powf(1.732))).abs() < 2e-3);
    }
}
