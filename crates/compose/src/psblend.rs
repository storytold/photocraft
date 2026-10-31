//! Photoshop-exact blend functions where they differ from the generic
//! reference in `photocraft_color::blend`, plus the compositing formula.
//!
//! Delegated directly to `craft-blitter` for zero-allocation SIMD and bit-exact oracle execution.

use photocraft_color::blend::BlendMode;

// Re-export constants and mathematical kernels from craft_blitter
pub use craft_blitter::{EDGE, TEXT_GAMMA, color_burn, color_dodge, hard_mix_ps, vivid_light_generic, vivid_light_ps};

/// Maps `photocraft_color::BlendMode` to `craft_blitter::BlendMode`.
///
/// Exhaustive const match compiling to a zero-cost identity/register translation.
#[inline(always)]
pub const fn to_blitter_blend_mode(mode: BlendMode) -> craft_blitter::BlendMode {
    match mode {
        BlendMode::PassThrough => craft_blitter::BlendMode::PassThrough,
        BlendMode::Normal => craft_blitter::BlendMode::Normal,
        BlendMode::Dissolve => craft_blitter::BlendMode::Dissolve,
        BlendMode::Darken => craft_blitter::BlendMode::Darken,
        BlendMode::Multiply => craft_blitter::BlendMode::Multiply,
        BlendMode::ColorBurn => craft_blitter::BlendMode::ColorBurn,
        BlendMode::LinearBurn => craft_blitter::BlendMode::LinearBurn,
        BlendMode::DarkerColor => craft_blitter::BlendMode::DarkerColor,
        BlendMode::Lighten => craft_blitter::BlendMode::Lighten,
        BlendMode::Screen => craft_blitter::BlendMode::Screen,
        BlendMode::ColorDodge => craft_blitter::BlendMode::ColorDodge,
        BlendMode::LinearDodge => craft_blitter::BlendMode::LinearDodge,
        BlendMode::LighterColor => craft_blitter::BlendMode::LighterColor,
        BlendMode::Overlay => craft_blitter::BlendMode::Overlay,
        BlendMode::SoftLight => craft_blitter::BlendMode::SoftLight,
        BlendMode::HardLight => craft_blitter::BlendMode::HardLight,
        BlendMode::VividLight => craft_blitter::BlendMode::VividLight,
        BlendMode::LinearLight => craft_blitter::BlendMode::LinearLight,
        BlendMode::PinLight => craft_blitter::BlendMode::PinLight,
        BlendMode::HardMix => craft_blitter::BlendMode::HardMix,
        BlendMode::Difference => craft_blitter::BlendMode::Difference,
        BlendMode::Exclusion => craft_blitter::BlendMode::Exclusion,
        BlendMode::Subtract => craft_blitter::BlendMode::Subtract,
        BlendMode::Divide => craft_blitter::BlendMode::Divide,
        BlendMode::Hue => craft_blitter::BlendMode::Hue,
        BlendMode::Saturation => craft_blitter::BlendMode::Saturation,
        BlendMode::Color => craft_blitter::BlendMode::Color,
        BlendMode::Luminosity => craft_blitter::BlendMode::Luminosity,
        BlendMode::Reflect => craft_blitter::BlendMode::Reflect,
        BlendMode::Glow => craft_blitter::BlendMode::Glow,
        BlendMode::Negation => craft_blitter::BlendMode::Negation,
        BlendMode::Xor => craft_blitter::BlendMode::Xor,
        BlendMode::PaintNetColorBurn => craft_blitter::BlendMode::PaintNetColorBurn,
        BlendMode::PaintNetColorDodge => craft_blitter::BlendMode::PaintNetColorDodge,
    }
}

thread_local! {
    /// Set while rendering a Lab document: Normal blending mixes backdrop and source in CIELAB
    pub static LAB_MIX: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while rendering a 32-bit float document: Linear Dodge (Add) and Divide don't clip at 1
    pub static HDR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn with_lab_mix<R>(lab: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            LAB_MIX.with(|l| l.set(self.0));
        }
    }
    let _restore = Restore(LAB_MIX.with(|l| l.replace(lab)));
    f()
}

pub fn with_hdr<R>(hdr: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            HDR.with(|h| h.set(self.0));
        }
    }
    let _restore = Restore(HDR.with(|h| h.replace(hdr)));
    f()
}

/// `B(Cb, Cs)` with Photoshop's variants.
#[inline]
pub fn blend_rgb(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    let bm = to_blitter_blend_mode(mode);
    if HDR.with(|h| h.get()) { craft_blitter::blend_rgb_hdr(bm, cb, cs) } else { craft_blitter::blend_rgb(bm, cb, cs) }
}

/// Photoshop's "special eight": the modes for which a layer's Fill isn't just less coverage.
#[inline]
pub fn fill_is_special(mode: BlendMode) -> bool {
    craft_blitter::fill_is_special(to_blitter_blend_mode(mode))
}

/// [`blend_rgb`] for a layer at `fill` (0..1).
#[inline]
pub fn blend_rgb_fill(mode: BlendMode, cb: [f32; 3], cs: [f32; 3], fill: f32) -> [f32; 3] {
    craft_blitter::blend_rgb_fill(to_blitter_blend_mode(mode), cb, cs, fill, HDR.with(|h| h.get()))
}

/// [`composite_gamma`] for a layer with separate `opacity` and `fill`.
#[inline]
pub fn composite_fill(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, fill: f32, gamma: f32) -> [f32; 4] {
    if fill >= 1.0 || !fill_is_special(mode) {
        return composite_gamma(mode, backdrop, source, opacity * fill.clamp(0.0, 1.0), gamma);
    }
    craft_blitter::composite_fill_f32(to_blitter_blend_mode(mode), backdrop, source, opacity, fill, gamma, HDR.with(|h| h.get()))
}

/// Straight-alpha source over straight-alpha backdrop (W3C/PDF general formula).
#[inline]
pub fn composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let ao = as_ + ab * (1.0 - as_);
    if mode == BlendMode::Normal && LAB_MIX.with(|l| l.get()) && ab > 0.0 && ao > 0.0 {
        let lb = photocraft_color::convert::srgb_to_lab([backdrop[0], backdrop[1], backdrop[2]]);
        let ls = photocraft_color::convert::srgb_to_lab([source[0], source[1], source[2]]);
        let kb = ab * (1.0 - as_) / ao;
        let k = as_ / ao;
        let m: [f32; 3] = std::array::from_fn(|i| lb[i] * kb + ls[i] * k);
        let r = photocraft_color::convert::lab_to_srgb(m);
        return [r[0], r[1], r[2], ao];
    }
    craft_blitter::float::composite_f32(to_blitter_blend_mode(mode), backdrop, source, opacity, HDR.with(|h| h.get()))
}

/// Normal mixing outside Lab.
#[inline]
#[allow(dead_code)]
pub(crate) fn composite_normal_rgb(backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    craft_blitter::float::composite_normal_f32(backdrop, source, opacity)
}

/// [`composite_normal_rgb`] over a row in vectorized chunks.
#[inline]
#[allow(dead_code)]
pub(crate) fn composite_normal_rgb_span(dst: &mut [[f32; 4]], src: &[[f32; 4]], opacity: f32) {
    craft_blitter::simd::composite_normal_rgb_span(dst, src, opacity);
}

static TEXT_GAMMA_SETTING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3fb9_999a); // 1.45f32

pub fn set_text_gamma(gamma: f32) {
    let g = if gamma.is_finite() { gamma.clamp(1.0, 2.2) } else { TEXT_GAMMA };
    TEXT_GAMMA_SETTING.store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn text_gamma() -> f32 {
    f32::from_bits(TEXT_GAMMA_SETTING.load(std::sync::atomic::Ordering::Relaxed))
}

#[inline]
pub fn text_encode(v: f32, gamma: f32) -> f32 {
    craft_blitter::blend::text_encode(v, gamma)
}

#[inline]
pub fn text_decode(v: f32, gamma: f32) -> f32 {
    craft_blitter::blend::text_decode(v, gamma)
}

#[inline]
pub fn composite_gamma(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32, gamma: f32) -> [f32; 4] {
    if gamma == 1.0 {
        return composite(mode, backdrop, source, opacity);
    }
    craft_blitter::float::composite_gamma_f32(to_blitter_blend_mode(mode), backdrop, source, opacity, gamma, HDR.with(|h| h.get()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn text_gamma_lightens_dark_edges() {
        // 69 % blue (0, 51, 153) over grey 129: Photoshop shows (57, 80, 146).
        let r = composite_gamma(BlendMode::Normal, [129.0 / 255.0, 129.0 / 255.0, 129.0 / 255.0, 1.0], [0.0, 0.2, 0.6, 0.686], 1.0, TEXT_GAMMA);
        assert!((r[0] * 255.0 - 57.0).abs() < 1.0 && (r[1] * 255.0 - 80.0).abs() < 1.0 && (r[2] * 255.0 - 146.0).abs() < 1.0, "{r:?}");
        // Near-opaque edges stay dark: 94 % coverage shows (13, 57, 152) (a v^1.45 mix gives 19 red).
        let r = composite_gamma(BlendMode::Normal, [129.0 / 255.0, 129.0 / 255.0, 129.0 / 255.0, 1.0], [0.0, 0.2, 0.6, 0.937], 1.0, TEXT_GAMMA);
        assert!((r[0] * 255.0 - 13.0).abs() < 1.0 && (r[1] * 255.0 - 57.0).abs() < 1.0 && (r[2] * 255.0 - 152.0).abs() < 1.0, "{r:?}");
        assert_eq!(
            composite_gamma(BlendMode::Multiply, [0.2, 0.4, 0.6, 0.7], [0.5, 0.1, 0.9, 0.5], 0.8, 1.0),
            composite(BlendMode::Multiply, [0.2, 0.4, 0.6, 0.7], [0.5, 0.1, 0.9, 0.5], 0.8)
        );
    }

    use super::*;
    use photocraft_color::blend as generic;

    /// Photoshop 27.11, measured: Background #c89664, a layer #64b4dc at Fill 50% (128/255),
    /// Opacity 100%, in each of the special eight. Values are the composite at 8 bits.
    #[test]
    fn special_eight_fill_matches_photoshop() {
        let measured: [(BlendMode, [u8; 3]); 8] = [
            (BlendMode::ColorBurn, [176, 132, 88]),
            (BlendMode::LinearBurn, [122, 112, 82]),
            (BlendMode::ColorDodge, [249, 232, 176]),
            (BlendMode::LinearDodge, [250, 240, 210]),
            (BlendMode::VividLight, [193, 188, 157]),
            (BlendMode::LinearLight, [171, 202, 192]),
            (BlendMode::HardMix, [245, 225, 165]),
            (BlendMode::Difference, [150, 60, 10]),
        ];
        let px = |c: [u8; 3]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0];
        let (b, t) = (px([200, 150, 100]), px([100, 180, 220]));
        for (mode, want) in measured {
            let got = composite_fill(mode, b, t, 1.0, 128.0 / 255.0, 1.0);
            for i in 0..3 {
                assert!((got[i] * 255.0 - f32::from(want[i])).abs() <= 1.5, "{mode:?} channel {i}: {} vs Photoshop {}", got[i] * 255.0, want[i]);
            }
            // At 100% Fill it is the plain blend; at 0% the backdrop.
            assert_eq!(composite_fill(mode, b, t, 1.0, 1.0, 1.0), composite(mode, b, t, 1.0), "{mode:?}");
            let none = composite_fill(mode, b, t, 1.0, 0.0, 1.0);
            assert!((0..3).all(|i| (none[i] - b[i]).abs() < 1e-5), "{mode:?} at Fill 0: {none:?}");
        }
        // A second measurement: Background #3c78b4, layer #d25a1e at Fill 25% (64/255) and 75%
        // (191/255). Hard Mix at 75% is 3 levels off in red: dividing by 1 − f = 0.25 magnifies
        // Photoshop's 8-bit rounding.
        let confirm: [(BlendMode, [u8; 3], [u8; 3]); 8] = [
            (BlendMode::ColorBurn, [51, 94, 159], [30, 0, 33]),
            (BlendMode::LinearBurn, [49, 79, 124], [26, 0, 11]),
            (BlendMode::ColorDodge, [76, 132, 186], [156, 163, 197]),
            (BlendMode::LinearDodge, [113, 143, 188], [217, 187, 202]),
            (BlendMode::VividLight, [71, 109, 162], [117, 81, 80]),
            (BlendMode::LinearLight, [100, 100, 130], [183, 63, 33]),
            (BlendMode::HardMix, [65, 105, 166], [102, 0, 47]),
            (BlendMode::Difference, [7, 97, 172], [97, 53, 158]),
        ];
        let (b2, t2) = (px([60, 120, 180]), px([210, 90, 30]));
        for (mode, q, tq) in confirm {
            for (fill, want) in [(64.0 / 255.0, q), (191.0 / 255.0, tq)] {
                let got = composite_fill(mode, b2, t2, 1.0, fill, 1.0);
                let tol = if mode == BlendMode::HardMix { 3.5 } else { 1.5 };
                for i in 0..3 {
                    assert!(
                        (got[i] * 255.0 - f32::from(want[i])).abs() <= tol,
                        "{mode:?} fill {fill} channel {i}: {} vs Photoshop {}",
                        got[i] * 255.0,
                        want[i]
                    );
                }
            }
        }
        // Other modes keep Fill as coverage, like Opacity.
        assert_eq!(composite_fill(BlendMode::Multiply, b, t, 0.8, 0.5, 1.0), composite(BlendMode::Multiply, b, t, 0.4));
        // Over transparency Fill is still coverage for the special eight.
        let r = composite_fill(BlendMode::LinearDodge, [0.0; 4], t, 1.0, 0.5, 1.0);
        assert!((r[3] - 0.5).abs() < 1e-6 && (r[0] - t[0]).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn vivid_light_source_extremes_win() {
        assert_eq!(vivid_light_ps(0.0, 1.0), 1.0);
        assert_eq!(vivid_light_ps(1.0, 0.0), 0.0);
        assert!((vivid_light_ps(0.5, 0.75) - 1.0).abs() < 1e-6);
        assert!((vivid_light_ps(0.5, 0.25) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn hard_mix_edges_match_photoshop() {
        assert_eq!(hard_mix_ps(1.0, 0.0), 1.0);
        assert_eq!(hard_mix_ps(0.0, 1.0), 0.0);
        assert_eq!(hard_mix_ps(0.6, 0.6), 1.0);
        assert_eq!(hard_mix_ps(0.3, 0.3), 0.0);
    }

    #[test]
    fn burn_and_dodge_treat_near_extremes_as_exact() {
        assert_eq!(color_burn(1.0 - 1e-6, 0.0), 1.0);
        assert_eq!(color_dodge(1e-6, 1.0), 0.0);
        assert!((color_burn(0.5, 0.5) - 0.0).abs() < 1e-6);
        assert!((color_dodge(0.25, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hdr_nests_and_restores() {
        let bright = |m| blend_rgb(m, [0.5, 0.5, 0.5], [1.0, 0.0, 0.0])[0];
        with_hdr(true, || {
            // A 32-bit document keeps Color's red above 1.
            assert!(bright(BlendMode::Color) > 1.1);
            // Another tile's (or document's) scope run inside this one doesn't reset it.
            with_hdr(false, || assert!(bright(BlendMode::Color) <= 1.0));
            assert!(HDR.get());
        });
        assert!(!HDR.get());
        assert!(bright(BlendMode::Color) <= 1.0);
    }

    #[test]
    fn other_modes_delegate() {
        for m in BlendMode::LAYER_MODES {
            if matches!(m, BlendMode::VividLight | BlendMode::HardMix | BlendMode::ColorBurn | BlendMode::ColorDodge) {
                continue;
            }
            let (cb, cs) = ([0.2, 0.5, 0.9], [0.7, 0.1, 0.4]);
            assert_eq!(blend_rgb(m, cb, cs), generic::blend_rgb(m, cb, cs));
            let (a, b) =
                (composite(m, [0.2, 0.5, 0.9, 0.7], [0.7, 0.1, 0.4, 0.6], 0.8), generic::composite(m, [0.2, 0.5, 0.9, 0.7], [0.7, 0.1, 0.4, 0.6], 0.8));
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 1e-6, "{m:?} {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn generic_vivid_light_and_hard_mix_composites_match_photoshop() {
        let cases = [
            (BlendMode::HardMix, 0.0, 1.0),
            (BlendMode::HardMix, 1.0, 0.0),
            (BlendMode::HardMix, 0.6, 0.4),
            (BlendMode::HardMix, 0.6, 0.3),
            (BlendMode::VividLight, 1.0, 0.0),
            (BlendMode::VividLight, 0.0, 1.0),
            (BlendMode::VividLight, 0.75, 0.25),
            (BlendMode::VividLight, 0.25, 0.75),
        ];

        for (mode, cb, cs) in cases {
            let backdrop = [cb, cb, cb, 1.0];
            let source = [cs, cs, cs, 1.0];
            let generic = generic::composite(mode, backdrop, source, 1.0);
            let photoshop = composite(mode, backdrop, source, 1.0);
            for channel in 0..4 {
                assert!((generic[channel] - photoshop[channel]).abs() < 1e-6, "{mode:?} Cb={cb} Cs={cs}: {generic:?} != {photoshop:?}");
            }
        }
    }
}
