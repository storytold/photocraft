//! Filter › Camera Raw Filter: a float develop pipeline over RGB pixels.
//!
//! Stages, in order (each skipped when its controls are neutral, so defaults are an identity):
//!
//! 1. **White balance** in linear light (the document's values decoded with the sRGB curve as a
//!    perceptual working encoding; encoding is undone exactly at the end), then **Exposure** and
//!    **Contrast**: tone curves measured on Photoshop's Camera Raw, hue-preserving in linear
//!    ProPhoto (see `exposure` and `contrast`).
//! 2. **Dehaze**: K. He, J. Sun, X. Tang, *Single Image Haze Removal Using Dark Channel Prior*,
//!    CVPR 2009 (dark channel, atmospheric light from its brightest 0.1 %, transmission refined
//!    with the guided filter); negative amounts add haze toward the atmospheric light.
//! 3. **Tone** on luminance with colour ratios kept: Highlights / Shadows as exposure changes
//!    weighted by an edge-preserving base layer (guided filter: K. He, J. Sun, X. Tang, *Guided
//!    Image Filtering*, ECCV 2010), so local contrast survives; Clarity (large radius, midtone
//!    weighted) and Texture (small radius) add
//!    band-pass local contrast. Whites / Blacks follow as Photoshop's Camera Raw applies them:
//!    global curves measured on Photoshop 2026, Whites up adapted to the image (see
//!    `whites_blacks`).
//! 4. **Presence**: Vibrance (weighted toward less saturated colours) and Saturation.
//! 5. **Tone Curve**: parametric regions (Highlights, Lights, Darks, Shadows with movable splits)
//!    then point curves (master and per channel), monotone cubic interpolation (F. Fritsch,
//!    R. Carlson, *Monotone Piecewise Cubic Interpolation*, SIAM J. Numer. Anal. 1980).
//! 6. **Color Mixer (HSL)**: eight hue bands (red, orange, yellow, green, aqua, blue, purple,
//!    magenta), measured tables in HSV of linear ProPhoto (see `color_mixer`).
//! 7. **Color Grading**: shadows / midtones / highlights / global wheels (hue, saturation,
//!    luminance) with Blending and Balance.
//! 8. **Detail**: sharpening on luminance (unsharp mask with Detail as halo control and Masking as
//!    an edge mask) and noise reduction (luminance by guided filter, colour by chroma blur).
//! 9. **Effects**: post-crop vignette (amount, midpoint, roundness, feather, highlights,
//!    style) and grain (amount, size, roughness; position-hashed, deterministic).

#![allow(clippy::needless_range_loop)] // index loops mirror the maths (pixels × channels)

use serde::{Deserialize, Serialize};

/// Most points a Camera Raw point curve accepts from commands and the control channel.
pub const MAX_CURVE_POINTS: usize = 16;

/// Checks a point curve from untrusted input: empty (linear) or 2..=16 finite points in 0..=255
/// with inputs increasing by at least one level. Deserialization stays lenient on purpose:
/// earlier editors saved curves this rejects, and [`curve_lut`] already sorts and de-duplicates
/// them, so stored Smart Filters keep rendering.
pub fn validate_curve(points: &[[f32; 2]]) -> Result<(), String> {
    if points.len() == 1 {
        return Err("a nonempty curve needs at least two points".into());
    }
    if points.len() > MAX_CURVE_POINTS {
        return Err(format!("at most {MAX_CURVE_POINTS} curve points"));
    }
    if !points.iter().flatten().all(|v| v.is_finite() && (0.0..=255.0).contains(v)) {
        return Err("curve coordinates must be finite and in 0..=255".into());
    }
    if points.windows(2).any(|w| w[1][0] - w[0][0] < 1.0) {
        return Err("curve inputs must increase by at least one level".into());
    }
    Ok(())
}

use crate::photo_util::{hash01, linear_to_srgb, par_rows, srgb_to_linear};

mod black_white;
mod color_mixer;
mod colour_lut;
mod contrast;
mod exposure;
pub mod geometry;
mod hsv_table;
mod local_detail;
mod local_detail_tables;
mod local_tone;
pub mod masks;
mod optics;
mod per_pixel;
mod presence;
mod white_balance;
mod whites_blacks;

pub use per_pixel::{
    CURVE_LUT, FIX_GRID, Grade, Grain, MIXER_GRID, MIXER_KINDS, PRESENCE_GRID, PRESENCE_KINDS, PerPixel, TONE_STEPS, Vignette, VignetteStyle, WhitesBlacks,
    per_pixel, white_black_points,
};
pub use white_balance::{auto_white_balance, white_balance_from_sample};
pub use whites_blacks::{RowOn, TonePoints, Tones};

/// How the pixels encode light: the sRGB curve (gamma 0) or a pure power law (ProPhoto's 1.8,
/// Adobe RGB's 2.2), and in which primaries. The tone controls work on light, so the document's
/// own curve decodes it; the measured tables and models were fitted on sRGB, so they read the
/// document's colours converted into sRGB's primaries (unclamped, so colours outside sRGB's
/// gamut survive), and luminance is weighted in the document's primaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Encoding {
    gamma: f32,
    primaries: DocPrimaries,
}

/// The primaries of the document's linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DocPrimaries {
    Srgb,
    ProPhoto,
    AdobeRgb,
    /// Any other: matrices to linear ProPhoto (D50) and back.
    Matrix(exposure::Primaries),
}

impl Encoding {
    /// sRGB's curve and primaries.
    #[cfg(test)]
    pub(crate) const SRGB: Encoding = Encoding { gamma: 0.0, primaries: DocPrimaries::Srgb };

    /// The curve of `gamma`, with the primaries that go with it: ProPhoto's gamma 1.8 is
    /// ProPhoto, Adobe RGB's 563/256 Adobe RGB, everything else sRGB's.
    pub(crate) fn of(gamma: f32) -> Self {
        let gamma = if gamma.is_finite() && gamma > 0.0 { gamma.clamp(0.5, 4.0) } else { 0.0 };
        let primaries = match gamma {
            g if (g - 1.8).abs() < 1e-3 => DocPrimaries::ProPhoto,
            g if (g - 563.0 / 256.0).abs() < 1e-3 => DocPrimaries::AdobeRgb,
            _ => DocPrimaries::Srgb,
        };
        Encoding { gamma, primaries }
    }

    /// The curve of `gamma` with the document's own primaries, `to_prophoto` (its linear RGB to
    /// linear ProPhoto); without one, or with one that is not finite or can't be inverted, the
    /// primaries [`Encoding::of`] infers.
    pub(crate) fn with_primaries(gamma: f32, to_prophoto: Option<[[f32; 3]; 3]>) -> Self {
        let enc = Self::of(gamma);
        match to_prophoto.and_then(|m| Some((m, exposure::invert(&m)?))) {
            Some(pp) => Encoding { primaries: DocPrimaries::Matrix(pp), ..enc },
            None => enc,
        }
    }

    /// The power-law gamma, or 0 for the sRGB curve.
    pub(crate) fn gamma(self) -> f32 {
        self.gamma
    }

    /// A value in this encoding as linear light.
    pub(crate) fn decode(self, v: f32) -> f32 {
        if self.gamma > 0.0 { v.max(0.0).powf(self.gamma) } else { srgb_to_linear(v) }
    }
    /// Linear light back into this encoding, the inverse of [`Encoding::decode`].
    pub(crate) fn encode(self, v: f32) -> f32 {
        if self.gamma > 0.0 { v.max(0.0).powf(1.0 / self.gamma) } else { linear_to_srgb(v) }
    }

    /// The matrices to linear ProPhoto and back, `None` for ProPhoto (no conversion).
    pub(crate) fn prophoto(self) -> Option<exposure::Primaries> {
        match self.primaries {
            DocPrimaries::Srgb => Some((exposure::SRGB_TO_PROPHOTO, exposure::PROPHOTO_TO_SRGB)),
            DocPrimaries::ProPhoto => None,
            DocPrimaries::AdobeRgb => Some((exposure::ADOBE_TO_PROPHOTO, exposure::PROPHOTO_TO_ADOBE)),
            DocPrimaries::Matrix(pp) => Some(pp),
        }
    }

    /// Whether the document's primaries are sRGB's.
    pub(crate) fn srgb_primaries(self) -> bool {
        self.primaries == DocPrimaries::Srgb
    }

    /// Linear document RGB in linear sRGB primaries (exactly `c` for sRGB documents).
    fn to_srgb_linear(self, c: [f32; 3]) -> [f32; 3] {
        match self.prophoto() {
            _ if self.srgb_primaries() => c,
            None => exposure::mat(&exposure::PROPHOTO_TO_SRGB, c),
            Some((to, _)) => exposure::mat(&exposure::PROPHOTO_TO_SRGB, exposure::mat(&to, c)),
        }
    }

    /// Linear sRGB back into the document's linear RGB, the inverse of `to_srgb_linear`.
    fn back_from_srgb_linear(self, c: [f32; 3]) -> [f32; 3] {
        match self.prophoto() {
            _ if self.srgb_primaries() => c,
            None => exposure::mat(&exposure::SRGB_TO_PROPHOTO, c),
            Some((_, from)) => exposure::mat(&from, exposure::mat(&exposure::SRGB_TO_PROPHOTO, c)),
        }
    }

    /// The relative luminance of linear document RGB `c`: Rec. 709's weights on its sRGB
    /// equivalent (exactly [`luma`] for sRGB documents).
    pub(crate) fn luminance(self, c: [f32; 3]) -> f32 {
        luma(self.to_srgb_linear(c))
    }

    /// A pixel in this encoding as sRGB-encoded values in sRGB's primaries, the domain the
    /// measured tables were fitted in; unclamped. Exactly `c` for sRGB documents.
    pub(crate) fn to_srgb_encoded(self, c: [f32; 3]) -> [f32; 3] {
        if self.srgb_primaries() {
            return if self.gamma > 0.0 { c.map(|v| linear_to_srgb(self.decode(v))) } else { c };
        }
        self.to_srgb_linear(c.map(|v| self.decode(v))).map(linear_to_srgb)
    }

    /// The inverse of [`Encoding::to_srgb_encoded`].
    pub(crate) fn back_from_srgb_encoded(self, s: [f32; 3]) -> [f32; 3] {
        if self.srgb_primaries() {
            return if self.gamma > 0.0 { s.map(|v| self.encode(srgb_to_linear(v))) } else { s };
        }
        self.back_from_srgb_linear(s.map(srgb_to_linear)).map(|v| self.encode(v))
    }
}

fn luma_linear(c: [f32; 3]) -> f32 {
    luma(c)
}

/// One colour grading wheel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Wheel {
    /// Degrees 0..360.
    pub hue: f32,
    /// 0..=100.
    pub sat: f32,
    /// −100..=100.
    pub lum: f32,
}

impl Wheel {
    fn neutral(&self) -> bool {
        self.sat == 0.0 && self.lum == 0.0
    }
}

/// Camera Raw settings in Adobe Camera Raw's slider units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraRaw {
    // Basic
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
    // Tone curve
    pub curve_highlights: f32,
    pub curve_lights: f32,
    pub curve_darks: f32,
    pub curve_shadows: f32,
    /// Region splits (percent).
    pub curve_splits: [f32; 3],
    /// Point curves as `[input, output]` in 0..=255 (empty = linear).
    pub point_curve: Vec<[f32; 2]>,
    pub point_curve_red: Vec<[f32; 2]>,
    pub point_curve_green: Vec<[f32; 2]>,
    pub point_curve_blue: Vec<[f32; 2]>,
    // Color mixer: red, orange, yellow, green, aqua, blue, purple, magenta.
    pub hsl_hue: [f32; 8],
    pub hsl_sat: [f32; 8],
    pub hsl_lum: [f32; 8],
    /// Treatment: `"color"` or `"bw"` (Black & White, measured in `black_white`).
    pub treatment: String,
    /// The B&W mixer, −100 … 100 per band (red, orange, yellow, green, aqua, blue, purple,
    /// magenta); used when [`treatment`](Self::treatment) is `"bw"`.
    pub bw_mix: [f32; 8],
    /// Optics › Distortion, −100 … 100 (measured, see `optics`).
    pub lens_distortion: f32,
    /// Optics › Vignetting amount, −100 … 100 (measured, see `optics`).
    pub lens_vignette: f32,
    /// Optics › Vignetting midpoint, 0 … 100.
    pub lens_vignette_midpoint: f32,
    // Color grading
    pub grade_shadows: Wheel,
    pub grade_midtones: Wheel,
    pub grade_highlights: Wheel,
    pub grade_global: Wheel,
    pub grade_blending: f32,
    pub grade_balance: f32,
    // Detail
    pub sharpen_amount: f32,
    pub sharpen_radius: f32,
    pub sharpen_detail: f32,
    pub sharpen_masking: f32,
    pub noise_luminance: f32,
    pub noise_luminance_detail: f32,
    pub noise_color: f32,
    pub noise_color_detail: f32,
    // Effects
    pub grain_amount: f32,
    pub grain_size: f32,
    pub grain_roughness: f32,
    pub vignette_amount: f32,
    pub vignette_midpoint: f32,
    pub vignette_roundness: f32,
    pub vignette_feather: f32,
    pub vignette_highlights: f32,
    /// `highlightPriority | colorPriority | paintOverlay`.
    pub vignette_style: String,
    pub seed: u32,
    /// Pixel-size multiplier for previews on downsampled proxies (radii in px scale by it).
    pub pixel_scale: f32,
    /// The document's transfer curve: 0 for sRGB's, else the gamma of a power-law space (1.8
    /// for ProPhoto). Set from the document's profile; Whites and Blacks act on light.
    pub encoding_gamma: f32,
    /// The document's primaries: its linear RGB to linear ProPhoto RGB (D50), from its colour
    /// profile; `None` for the primaries `encoding_gamma` implies (ProPhoto for 1.8, Adobe RGB
    /// for 563/256, else sRGB). Set at run time by whoever knows the document (the command, the
    /// dialog), never stored, so saved settings and Smart Filters read as before.
    #[serde(skip)]
    pub document_primaries: Option<[[f32; 3]; 3]>,
    /// Upright's analysis of the full image, set at run time by a preview that develops a
    /// smaller copy, so it shows the correction the full image gets; never stored. Without one
    /// (or with one for another mode) the develop analyses the image it is given, before its
    /// first stage. See [`geometry::UprightFix`].
    #[serde(skip)]
    pub upright_fix: Option<geometry::UprightFix>,
    // Calibration (Camera Raw's Calibration panel)
    /// Shadows › Tint: −100 (green) … 100 (magenta), strongest in the darkest tones.
    pub shadow_tint: f32,
    /// Red, Green and Blue Primary › Hue: −100 … 100, the primary turned towards its neighbours.
    pub calibration_hue: [f32; 3],
    /// Red, Green and Blue Primary › Saturation: −100 (grey) … 100 (purer).
    pub calibration_sat: [f32; 3],
    // Geometry (Camera Raw's Geometry panel, see `geometry`)
    /// Upright: `off | auto | level | vertical | full | guided`.
    pub perspective_upright: String,
    /// Guided Upright's guides, `[[x0, y0], [x1, y1]]` in 0 … 1 image coordinates (at most four).
    pub upright_guides: Vec<[[f32; 2]; 2]>,
    /// Transform › Vertical, −100 … 100.
    pub perspective_vertical: f32,
    /// Transform › Horizontal, −100 … 100.
    pub perspective_horizontal: f32,
    /// Transform › Rotate, degrees −10 … 10.
    pub perspective_rotate: f32,
    /// Transform › Aspect, −100 … 100.
    pub perspective_aspect: f32,
    /// Transform › Scale, percent 50 … 150.
    pub perspective_scale: f32,
    /// Transform › Offset X, −100 … 100.
    pub perspective_x: f32,
    /// Transform › Offset Y, −100 … 100.
    pub perspective_y: f32,
    /// Constrain Crop: scale the result up so no uncovered (transparent) area shows.
    pub constrain_crop: bool,
    /// The Masks panel: local adjustments through masks (see `masks`).
    pub masks: Vec<masks::Mask>,
}

impl Default for CameraRaw {
    fn default() -> Self {
        CameraRaw {
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            curve_highlights: 0.0,
            curve_lights: 0.0,
            curve_darks: 0.0,
            curve_shadows: 0.0,
            curve_splits: [25.0, 50.0, 75.0],
            point_curve: Vec::new(),
            point_curve_red: Vec::new(),
            point_curve_green: Vec::new(),
            point_curve_blue: Vec::new(),
            hsl_hue: [0.0; 8],
            hsl_sat: [0.0; 8],
            hsl_lum: [0.0; 8],
            grade_shadows: Wheel::default(),
            grade_midtones: Wheel::default(),
            grade_highlights: Wheel::default(),
            grade_global: Wheel::default(),
            grade_blending: 50.0,
            grade_balance: 0.0,
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            noise_luminance: 0.0,
            noise_luminance_detail: 50.0,
            noise_color: 0.0,
            noise_color_detail: 50.0,
            grain_amount: 0.0,
            grain_size: 25.0,
            grain_roughness: 50.0,
            vignette_amount: 0.0,
            vignette_midpoint: 50.0,
            vignette_roundness: 0.0,
            vignette_feather: 50.0,
            vignette_highlights: 0.0,
            vignette_style: "highlightPriority".into(),
            seed: 0,
            pixel_scale: 1.0,
            encoding_gamma: 0.0,
            document_primaries: None,
            upright_fix: None,
            shadow_tint: 0.0,
            calibration_hue: [0.0; 3],
            calibration_sat: [0.0; 3],
            perspective_upright: "off".into(),
            upright_guides: Vec::new(),
            perspective_vertical: 0.0,
            perspective_horizontal: 0.0,
            perspective_rotate: 0.0,
            perspective_aspect: 0.0,
            perspective_scale: 100.0,
            perspective_x: 0.0,
            perspective_y: 0.0,
            constrain_crop: false,
            masks: Vec::new(),
            treatment: "color".into(),
            bw_mix: [0.0; 8],
            lens_distortion: 0.0,
            lens_vignette: 0.0,
            lens_vignette_midpoint: 50.0,
        }
    }
}

/// Hue band centres of the Color Mixer (degrees).
pub const HSL_BANDS: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0];

/// How far a primary turns at Hue ±100 (degrees), as the Color Mixer's hue sliders do.
const CALIBRATION_TURN: f32 = 30.0;

/// How much of each primary (red, green, blue) a hue is made of: triangular weights around
/// 0°, 120° and 240° that sum to one, so a colour between two primaries follows both.
pub fn calibration_weights(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0);
    let mut w = [0.0f32; 3];
    for (i, centre) in [0.0f32, 120.0, 240.0].iter().enumerate() {
        let d = (h - centre).abs().min(360.0 - (h - centre).abs());
        w[i] = (1.0 - d / 120.0).max(0.0);
    }
    let sum: f32 = w.iter().sum();
    if sum > 1e-6 { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0] }
}

/// The Calibration panel's primaries on one linear-light pixel: its hue turns by the weighted
/// Hue of the primaries it is made of (±100 is ±30°, positive towards the next primary) and its
/// saturation scales by their weighted Saturation (−100 greys it); the luminance is kept, so
/// neutrals are untouched.
fn calibrate(c: [f32; 3], hue: [f32; 3], sat: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = rgb_to_hsv(c.map(|x| x.max(0.0)));
    if s <= 1e-5 || v <= 1e-6 {
        return c;
    }
    let w = calibration_weights(h);
    let (mut dh, mut ds) = (0.0f32, 0.0f32);
    for i in 0..3 {
        dh += w[i] * hue[i].clamp(-100.0, 100.0);
        ds += w[i] * sat[i].clamp(-100.0, 100.0);
    }
    let nh = h + dh / 100.0 * CALIBRATION_TURN;
    let ns = (s * (1.0 + ds / 100.0)).clamp(0.0, 1.0);
    let out = hsv_to_rgb(nh, ns, v);
    let (l0, l1) = (luma(c), luma(out));
    if l1 > 1e-6 { out.map(|x| x * l0 / l1) } else { out }
}

/// Shadows › Tint on one linear-light pixel: `tint` −1 (green) … 1 (magenta), weighted by how
/// dark the pixel is; the luminance is kept.
fn shadow_tint(c: [f32; 3], tint: f32) -> [f32; 3] {
    let l = luma(c).max(0.0);
    // Linear 0.18 is middle grey: full strength below it, fading out towards white.
    let weight = (1.0 - (l / 0.18).sqrt().min(1.0)).powi(2) * 0.5 + (1.0 - l.min(1.0)).powi(4) * 0.5;
    let k = tint * 0.12 * weight * (l + 0.02);
    let o = [c[0] + k, c[1] - k, c[2] + k].map(|v| v.max(0.0));
    let l2 = luma(o);
    if l2 > 1e-6 { o.map(|v| v * l / l2) } else { o }
}

/// Camera Raw's Auto for the Light panel: Exposure, Contrast, Highlights, Shadows, Whites and
/// Blacks from the image's luminance histogram (`px` in the document's encoding). A well
/// exposed image gets near-zero settings; a dark one more exposure and lifted shadows; one
/// with a crushed or blown end gets Blacks or Whites back from the edge. The other panels are
/// left alone, as Camera Raw's Auto leaves them.
pub fn auto_tone(px: &[[f32; 4]], p: &mut CameraRaw) {
    if px.is_empty() {
        return;
    }
    let mut lum: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]]).clamp(0.0, 1.0)).collect();
    lum.sort_by(|a, b| a.total_cmp(b));
    let at = |f: f32| lum[((lum.len() - 1) as f32 * f).round() as usize];
    let (black, shadow, median, light, white) = (at(0.001), at(0.1), at(0.5), at(0.9), at(0.999));
    // Exposure: the median towards middle grey (sRGB 0.46), damped so flat scenes aren't
    // forced to it.
    let median_lin = srgb_to_linear(median).max(1e-4);
    p.exposure = (0.6 * (0.18f32 / median_lin).log2()).clamp(-2.0, 2.0);
    // Blacks and Whites bring the ends to the edges without clipping more than Camera Raw does.
    p.blacks = ((0.04 - black) * 400.0).clamp(-40.0, 40.0);
    p.whites = ((0.96 - white) * 300.0).clamp(-40.0, 40.0);
    // Shadows up when the darker tenth is crushed, highlights down when the brighter tenth is
    // near white.
    p.shadows = ((0.25 - shadow) * 200.0).clamp(0.0, 50.0);
    p.highlights = ((light - 0.85) * -300.0).clamp(-50.0, 0.0);
    // A little contrast for a flat histogram.
    let spread = (light - shadow).max(0.0);
    p.contrast = ((0.6 - spread) * 60.0).clamp(0.0, 25.0);
    // Shown as Camera Raw shows them: Exposure to a hundredth of a stop, the others whole.
    p.exposure = (p.exposure * 100.0).round() / 100.0;
    for v in [&mut p.blacks, &mut p.whites, &mut p.shadows, &mut p.highlights, &mut p.contrast] {
        *v = v.round();
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// CIE L* (0 … 100) of relative luminance `y`.
fn lstar(y: f32) -> f32 {
    let y = y.max(0.0);
    if y > 216.0 / 24389.0 { 116.0 * y.cbrt() - 16.0 } else { 24389.0 / 27.0 * y }
}

/// The relative luminance of CIE L* `l`.
fn lstar_inverse(l: f32) -> f32 {
    let l = l.max(0.0);
    if l > 8.0 { ((l + 16.0) / 116.0).powi(3) } else { l * 27.0 / 24389.0 }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Box mean of a single-channel buffer (radius `r`, edge clamped): rows in parallel, then
/// columns in parallel bands with running column sums (cache-friendly, no transposes).
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return v.to_vec();
    }
    let n = (2 * r + 1) as f64;
    let mut tmp = vec![0.0f32; w * h];
    par_rows(&mut tmp, w, 1, |y, row| {
        let src = &v[y * w..(y + 1) * w];
        let at = |k: i64| src[k.clamp(0, w as i64 - 1) as usize] as f64;
        let mut acc: f64 = (-(r as i64)..=r as i64).map(at).sum();
        for (x, o) in row.iter_mut().enumerate() {
            *o = (acc / n) as f32;
            acc += at(x as i64 + r as i64 + 1) - at(x as i64 - r as i64);
        }
    });
    const BAND: usize = 256;
    let bands = crate::photo_util::par_map(w.div_ceil(BAND), |b| {
        let (x0, x1) = (b * BAND, ((b + 1) * BAND).min(w));
        let bw = x1 - x0;
        let row = |y: i64| &tmp[y.clamp(0, h as i64 - 1) as usize * w + x0..y.clamp(0, h as i64 - 1) as usize * w + x1];
        let mut acc = vec![0.0f64; bw];
        for y in -(r as i64)..=r as i64 {
            for (a, v) in acc.iter_mut().zip(row(y)) {
                *a += *v as f64;
            }
        }
        let mut out = vec![0.0f32; bw * h];
        for y in 0..h {
            for (o, a) in out[y * bw..(y + 1) * bw].iter_mut().zip(&acc) {
                *o = (*a / n) as f32;
            }
            let (add, sub) = (row(y as i64 + r as i64 + 1), row(y as i64 - r as i64));
            for ((a, p), m) in acc.iter_mut().zip(add).zip(sub) {
                *a += (*p - *m) as f64;
            }
        }
        out
    });
    let mut out = vec![0.0f32; w * h];
    for (b, band) in bands.iter().enumerate() {
        let (x0, x1) = (b * BAND, ((b + 1) * BAND).min(w));
        let bw = x1 - x0;
        for y in 0..h {
            out[y * w + x0..y * w + x1].copy_from_slice(&band[y * bw..(y + 1) * bw]);
        }
    }
    out
}

/// Approximate Gaussian (three box passes).
fn gauss(v: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.3 {
        return v.to_vec();
    }
    let mut out = v.to_vec();
    for r in crate::fxutil::gauss_box_radii(sigma) {
        out = box_mean(&out, w, h, r);
    }
    out
}

/// Guided filter of `p` guided by `i` (He et al. 2010).
fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mi = box_mean(i, w, h, r);
    let mp = box_mean(p, w, h, r);
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = i.iter().map(|a| a * a).collect();
    let mip = box_mean(&ip, w, h, r);
    let mii = box_mean(&ii, w, h, r);
    let a: Vec<f32> = (0..w * h).map(|k| (mip[k] - mi[k] * mp[k]) / (mii[k] - mi[k] * mi[k] + eps)).collect();
    let b: Vec<f32> = (0..w * h).map(|k| mp[k] - a[k] * mi[k]).collect();
    let ma = box_mean(&a, w, h, r);
    let mb = box_mean(&b, w, h, r);
    (0..w * h).map(|k| ma[k] * i[k] + mb[k]).collect()
}

/// Monotone cubic (Fritsch–Carlson) through `pts` (x ascending, 0..=1), evaluated into a LUT.
pub fn curve_lut(pts: &[[f32; 2]], n: usize) -> Vec<f32> {
    let mut p: Vec<[f32; 2]> = pts.iter().map(|q| [q[0] / 255.0, q[1] / 255.0]).collect();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
    if p.len() < 2 {
        return (0..n).map(|k| k as f32 / (n - 1) as f32).collect();
    }
    let m = p.len();
    let d: Vec<f32> = (0..m - 1).map(|k| (p[k + 1][1] - p[k][1]) / (p[k + 1][0] - p[k][0]).max(1e-6)).collect();
    let mut t = vec![0.0f32; m];
    t[0] = d[0];
    t[m - 1] = d[m - 2];
    for k in 1..m - 1 {
        t[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) / 2.0 };
    }
    for k in 0..m - 1 {
        if d[k] == 0.0 {
            t[k] = 0.0;
            t[k + 1] = 0.0;
            continue;
        }
        let (a, b) = (t[k] / d[k], t[k + 1] / d[k]);
        let s = a * a + b * b;
        if s > 9.0 {
            let tau = 3.0 / s.sqrt();
            t[k] = tau * a * d[k];
            t[k + 1] = tau * b * d[k];
        }
    }
    (0..n)
        .map(|i| {
            let x = i as f32 / (n - 1) as f32;
            if x <= p[0][0] {
                return p[0][1];
            }
            if x >= p[m - 1][0] {
                return p[m - 1][1];
            }
            let k = p.windows(2).position(|wv| x <= wv[1][0]).unwrap_or(m - 2);
            let hh = p[k + 1][0] - p[k][0];
            let s = (x - p[k][0]) / hh;
            let (h00, h10, h01, h11) = (2.0 * s * s * s - 3.0 * s * s + 1.0, s * s * s - 2.0 * s * s + s, -2.0 * s * s * s + 3.0 * s * s, s * s * s - s * s);
            (h00 * p[k][1] + h10 * hh * t[k] + h01 * p[k + 1][1] + h11 * hh * t[k + 1]).clamp(0.0, 1.0)
        })
        .collect()
}

fn lut_eval(lut: &[f32], v: f32) -> f32 {
    if !(0.0..=1.0).contains(&v) {
        // Extend linearly outside (float documents).
        return if v < 0.0 { v + lut[0] } else { lut[lut.len() - 1] + (v - 1.0) };
    }
    let x = v * (lut.len() - 1) as f32;
    let i = (x as usize).min(lut.len() - 2);
    let t = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * t
}

/// The Tone Curve panel's parametric curve as `n` samples over 0 … 1 (at least 2): regions split
/// at `curve_splits`, each slider ±100 moves its region by up to a quarter of the range,
/// smoothly; monotone. The output level of input `i / (n − 1)`, as `develop` applies it.
pub fn parametric_lut(p: &CameraRaw, n: usize) -> Vec<f32> {
    let n = n.max(2);
    let s = p.curve_splits.map(|v| v.clamp(5.0, 95.0) / 100.0);
    let centres = [s[0] / 2.0, (s[0] + s[1]) / 2.0, (s[1] + s[2]) / 2.0, (s[2] + 1.0) / 2.0];
    let amounts = [p.curve_shadows, p.curve_darks, p.curve_lights, p.curve_highlights];
    let widths = [s[0].max(0.1), (s[1] - s[0]).max(0.1), (s[2] - s[1]).max(0.1), (1.0 - s[2]).max(0.1)];
    let mut lut: Vec<f32> = (0..n)
        .map(|i| {
            let x = i as f32 / (n - 1) as f32;
            let mut y = x;
            for k in 0..4 {
                let d = (x - centres[k]) / (widths[k] * 1.2);
                let bump = (-d * d * 2.0).exp() * x * (1.0 - x) * 4.0;
                y += amounts[k] / 100.0 * 0.25 * bump;
            }
            y.clamp(0.0, 1.0)
        })
        .collect();
    // Keep it monotone.
    for i in 1..n {
        lut[i] = lut[i].max(lut[i - 1]);
    }
    lut
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 1e-9 {
        0.0
    } else if mx == c[0] {
        60.0 * (((c[1] - c[2]) / d).rem_euclid(6.0))
    } else if mx == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, if mx > 1e-9 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

impl CameraRaw {
    /// Rejects malformed point curves in new settings (see [`validate_curve`]).
    pub fn validate(&self) -> Result<(), String> {
        for (name, curve) in [
            ("pointCurve", &self.point_curve),
            ("pointCurveRed", &self.point_curve_red),
            ("pointCurveGreen", &self.point_curve_green),
            ("pointCurveBlue", &self.point_curve_blue),
        ] {
            validate_curve(curve).map_err(|e| format!("{name}: {e}"))?;
        }
        self.validate_treatment()?;
        optics::validate(self)?;
        geometry::validate(self)?;
        masks::validate(&self.masks)
    }
    /// The treatment is `"color"` or `"bw"` and the B&W mixer is finite.
    fn validate_treatment(&self) -> Result<(), String> {
        if !matches!(self.treatment.as_str(), "color" | "bw") {
            return Err(format!("treatment: expected \"color\" or \"bw\", got {:?}", self.treatment));
        }
        if self.bw_mix.iter().any(|v| !v.is_finite()) {
            return Err("bwMix: values must be finite".into());
        }
        Ok(())
    }
    /// Black & White treatment.
    pub fn is_bw(&self) -> bool {
        self.treatment == "bw"
    }
    /// [`validate`](Self::validate) for an edit of `stored` settings: a curve left as stored is
    /// accepted, so editing another control of an older Smart Filter still works.
    pub fn validate_changes(&self, stored: &CameraRaw) -> Result<(), String> {
        for (name, curve, kept) in [
            ("pointCurve", &self.point_curve, &stored.point_curve),
            ("pointCurveRed", &self.point_curve_red, &stored.point_curve_red),
            ("pointCurveGreen", &self.point_curve_green, &stored.point_curve_green),
            ("pointCurveBlue", &self.point_curve_blue, &stored.point_curve_blue),
        ] {
            if curve != kept {
                validate_curve(curve).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        // Geometry, Masks and the treatment are new: always checked (no lenient older form).
        self.validate_treatment()?;
        optics::validate(self)?;
        geometry::validate(self)?;
        masks::validate(&self.masks)
    }
    /// The Optics panel at rest.
    pub fn optics_neutral(&self) -> bool {
        optics::neutral(self)
    }
    fn wb_neutral(&self) -> bool {
        self.temperature == 0.0 && self.tint == 0.0 && self.exposure == 0.0
    }
    fn tone_neutral(&self) -> bool {
        self.contrast == 0.0 && self.local_tone_neutral() && self.whites == 0.0 && self.blacks == 0.0
    }
    /// Highlights, Shadows, Texture and Clarity (the luminance stage).
    fn local_tone_neutral(&self) -> bool {
        [self.highlights, self.shadows, self.texture, self.clarity].iter().all(|v| *v == 0.0)
    }
    fn hsl_neutral(&self) -> bool {
        self.hsl_hue.iter().chain(&self.hsl_sat).chain(&self.hsl_lum).all(|v| *v == 0.0)
    }
    fn grade_neutral(&self) -> bool {
        self.grade_shadows.neutral() && self.grade_midtones.neutral() && self.grade_highlights.neutral() && self.grade_global.neutral()
    }
    /// The Calibration panel at rest.
    fn calibration_neutral(&self) -> bool {
        self.shadow_tint == 0.0 && self.calibration_hue.iter().chain(&self.calibration_sat).all(|v| *v == 0.0)
    }
    fn curves_neutral(&self) -> bool {
        [self.curve_highlights, self.curve_lights, self.curve_darks, self.curve_shadows].iter().all(|v| *v == 0.0)
            && [&self.point_curve, &self.point_curve_red, &self.point_curve_green, &self.point_curve_blue].iter().all(|c| is_linear(c))
    }
    /// The document's encoding: its transfer curve and primaries.
    pub(crate) fn encoding(&self) -> Encoding {
        Encoding::with_primaries(self.encoding_gamma, self.document_primaries)
    }
    /// The Geometry panel at rest (no Upright, no Transform).
    pub fn geometry_neutral(&self) -> bool {
        geometry::neutral(self)
    }
    /// True when pixels move (Geometry, or Optics › Distortion): what they uncover turns
    /// transparent, alpha moves with them, and the canvas is the frame they move in.
    pub fn moves_pixels(&self) -> bool {
        !self.geometry_neutral() || self.lens_distortion != 0.0
    }
    /// True when every control is neutral (the filter is an identity).
    pub fn is_identity(&self) -> bool {
        self.wb_neutral()
            && self.calibration_neutral()
            && self.tone_neutral()
            && self.dehaze == 0.0
            && self.vibrance == 0.0
            && self.saturation == 0.0
            && self.curves_neutral()
            && self.hsl_neutral()
            && self.grade_neutral()
            && self.sharpen_amount == 0.0
            && self.noise_luminance == 0.0
            && self.noise_color == 0.0
            && self.grain_amount == 0.0
            && self.vignette_amount == 0.0
            && self.geometry_neutral()
            && !self.masks.iter().any(masks::Mask::is_active)
            && !self.is_bw()
            && self.optics_neutral()
    }
}

fn is_linear(c: &[[f32; 2]]) -> bool {
    c.len() < 2 || c.iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
}

/// Camera Raw's white-balance channel gains for Temperature / Tint (−100…100), relative to the
/// as-shot balance: warmer raises red and lowers blue (±35 % at the ends); a negative tint
/// raises green, a positive one lowers it toward magenta (±25 %). An observed approximation of
/// Adobe Camera Raw's sliders, not a colour-temperature model; shared by the filter (in linear
/// RGB) and the raw open (on the camera's white-balance multipliers) so both agree.
pub fn white_balance_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let t = temperature / 100.0;
    let tn = tint / 100.0;
    [1.0 + 0.35 * t, 1.0 - 0.25 * tn, 1.0 - 0.35 * t]
}

/// Runs the Camera Raw pipeline on straight RGBA pixels (`w × h`, display-encoded RGB).
/// `float` keeps values above 1 (32-bit documents). Pixels that don't fill `w × h` exactly are
/// left unchanged. A stage that can't get the memory it needs is skipped: for previews; a
/// result that is kept goes through [`try_develop`], which reports it.
pub fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    // The error only says a stage was skipped; the pixels are as complete as they can be.
    let _ = develop_impl(px, w, h, p, float, false);
}

/// [`develop`], failing when a stage (Texture, Clarity) can't get the memory it needs, so an
/// incomplete result isn't committed as the developed image. The pixels are then partly
/// developed and should be discarded.
pub fn try_develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) -> Result<(), String> {
    develop_impl(px, w, h, p, float, false).0
}

/// [`develop`], also returning the pixels as the masks first see them (before
/// [`masks::apply`]), so the dialog can show a mask's coverage exactly as it applied rather than
/// recomputed from the result, which already holds the mask's own adjustment. `None` when `p`
/// has no masks or the pixels don't fill `w × h`.
pub fn develop_keeping_unmasked(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) -> Option<Vec<[f32; 4]>> {
    develop_impl(px, w, h, p, float, !p.masks.is_empty()).1
}

/// The develop: the first stage that ran out of memory, and the pixels before the masks when
/// `keep_unmasked`.
fn develop_impl(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool, keep_unmasked: bool) -> (Result<(), String>, Option<Vec<[f32; 4]>>) {
    if px.is_empty() || px.len() != w.saturating_mul(h) {
        return (Ok(()), None);
    }
    if p.is_identity() {
        return (Ok(()), keep_unmasked.then(|| px.to_vec()));
    }
    // Upright analyses the image as it comes in, so the tone stages never change its
    // correction and a preview handed the full image's analysis matches it.
    let resolved;
    let p = match geometry::pending_upright(px, w, h, p) {
        Some(fix) => {
            resolved = CameraRaw { upright_fix: Some(fix), ..p.clone() };
            &resolved
        }
        None => p,
    };
    let mut skipped: Option<String> = None;
    let ps = p.pixel_scale.clamp(0.01, 1.0);
    // 1. White balance (per-channel curves, see `white_balance`), then Exposure.
    white_balance::apply(px, w, p.temperature, p.tint, p.encoding());
    if !optics::vignette_neutral(p) {
        // Lens Vignetting is a local Exposure: the slider's stops plus each pixel's own.
        optics::exposure_with_vignette(px, w, h, p, p.encoding());
    } else if p.exposure != 0.0 {
        exposure::apply(px, w, &exposure::row(p.exposure), p.encoding());
    }
    if p.contrast != 0.0 {
        let enc = p.encoding();
        let tones = tone_points(px, w, h, enc, |q| q);
        exposure::apply(px, w, &contrast::adaptive_row(p.contrast, Some(tones)), enc);
    }
    // 1b. Calibration: the primaries turned and purer or greyer, and the shadows tinted, in
    // linear light (the camera's primaries are what Camera Raw's panel calibrates).
    if !p.calibration_neutral() {
        let primaries = p.calibration_hue.iter().chain(&p.calibration_sat).any(|v| *v != 0.0);
        let tint = p.shadow_tint.clamp(-100.0, 100.0) / 100.0;
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                let c = [srgb_to_linear(q[0]), srgb_to_linear(q[1]), srgb_to_linear(q[2])];
                let mut o = if primaries { calibrate(c, p.calibration_hue, p.calibration_sat) } else { c };
                if tint != 0.0 {
                    o = shadow_tint(o, tint);
                }
                for k in 0..3 {
                    q[k] = linear_to_srgb(o[k].max(0.0));
                }
            }
        });
    }
    // 2. Dehaze: removing haze (dark-channel model); adding it, a measured blend (`add_haze`).
    if p.dehaze > 0.0 {
        dehaze(px, w, h, p.dehaze / 100.0);
    } else if p.dehaze < 0.0 {
        add_haze(px, w, h, p.dehaze, p.encoding());
    }
    // 3. Tone. Highlights and Shadows: a local Laplacian filter (see `local_tone`).
    if p.highlights != 0.0 || p.shadows != 0.0 {
        local_tone::apply(px, w, h, p.shadows, p.highlights, p.encoding());
    }
    // Texture and Clarity: fitted multi-scale models of local detail (see `local_detail`).
    // Without memory a stage is skipped and the rest still runs (a preview stays useful); the
    // first failure is returned at the end.
    if p.texture != 0.0
        && let Err(e) = local_detail::apply(&local_detail::TEXTURE, px, w, h, p.texture, p.encoding())
    {
        skipped.get_or_insert(e);
    }
    if p.clarity != 0.0
        && let Err(e) = local_detail::apply(&local_detail::CLARITY, px, w, h, p.clarity, p.encoding())
    {
        skipped.get_or_insert(e);
    }
    // 3b. Whites and Blacks: global curves, Whites up adapted to the image's tones.
    if p.whites != 0.0 || p.blacks != 0.0 {
        let enc = p.encoding();
        let points = (p.whites > 0.0).then(|| tone_points(px, w, h, enc, |q| q));
        let (wr, br) = (whites_blacks::whites_row(p.whites, points), whites_blacks::blacks_row(p.blacks));
        let fix = (p.whites < 0.0).then(|| colour_lut::correction(colour_lut::Slider::Whites, p.whites)).flatten();
        whites_blacks::apply(px, w, wr.as_ref(), br.as_ref(), fix.as_ref(), enc);
    }
    // 4–7: per-pixel colour stages.
    let need_color = p.vibrance != 0.0 || p.saturation != 0.0 || !p.curves_neutral() || !p.hsl_neutral() || !p.grade_neutral() || p.is_bw();
    if need_color {
        let par = (!(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0)).then(|| parametric_lut(p, 1024));
        let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, 1024));
        let (master, cr, cg, cb) = (pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue));
        // Black & White replaces the colour stages: no Vibrance, Saturation or Color Mixer.
        let mono = p.is_bw().then(|| black_white::mono(&p.bw_mix)).flatten();
        let mixer = if p.is_bw() { None } else { color_mixer::mixer(&p.hsl_hue, &p.hsl_sat, &p.hsl_lum) };
        let grade = !p.grade_neutral();
        let Grade { tints, lums, blend, balance: bal } = Grade::new(p);
        let enc = p.encoding();
        // Vibrance and Saturation: measured tables (see `presence`).
        let presence = if p.is_bw() { None } else { presence::presence(p.vibrance, p.saturation) };
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                let mut c = [q[0], q[1], q[2]];
                if let Some(t) = &presence {
                    c = t.apply_encoded(c, enc);
                }
                if let Some(lut) = &par {
                    c = c.map(|v| lut_eval(lut, v));
                }
                if let Some(lut) = &master {
                    c = c.map(|v| lut_eval(lut, v));
                }
                for (ch, lut) in [&cr, &cg, &cb].iter().enumerate() {
                    if let Some(lut) = lut {
                        c[ch] = lut_eval(lut, c[ch]);
                    }
                }
                if let Some(m) = &mixer {
                    c = m.apply_encoded(c, enc);
                }
                if let Some(m) = &mono {
                    c = m.apply_encoded(c, enc);
                }
                if grade {
                    let l = luma(c).clamp(0.0, 1.0);
                    let sh = 1.0 - smoothstep(0.0, 0.5 + bal + blend, l);
                    let hi = smoothstep(0.5 + bal - blend, 1.0, l);
                    let mid = (1.0 - sh - hi).max(0.0);
                    let masks = [sh, mid, hi, 1.0];
                    for k in 0..4 {
                        for ch in 0..3 {
                            c[ch] += masks[k] * tints[k][ch];
                        }
                        if lums[k] != 0.0 {
                            let f = 2f32.powf(masks[k] * lums[k] / 100.0 * 0.6);
                            c = c.map(|x| x * f);
                        }
                    }
                }
                q[0] = c[0];
                q[1] = c[1];
                q[2] = c[2];
            }
        });
    }
    // 8. Detail.
    if p.noise_luminance > 0.0 || p.noise_color > 0.0 || p.sharpen_amount > 0.0 {
        detail(px, w, h, p, ps);
    }
    // 8a. Optics: Distortion (pixels it uncovers become transparent; Vignetting ran with Exposure).
    if !p.optics_neutral() {
        optics::apply(px, w, h, p, float);
    }
    // 8b. Geometry: Upright and Transform (pixels the warp uncovers become transparent).
    if !p.geometry_neutral() {
        geometry::apply(px, w, h, p, float);
    }
    // 8c. Masks: local adjustments, in the coordinates of the result.
    let unmasked = keep_unmasked.then(|| px.to_vec());
    if p.masks.iter().any(masks::Mask::is_active) {
        masks::apply(px, w, h, p, float);
    }
    // 9. Effects.
    if p.vignette_amount != 0.0 {
        vignette(px, w, h, p);
    }
    if p.grain_amount > 0.0 {
        let Grain { cell, roughness: rough, amount: amt, seed, scale: pscale } = Grain::new(p);
        par_rows(px, w, 1, |y, row| {
            for (x, q) in row.iter_mut().enumerate() {
                // Sample in full-resolution coordinates so previews match the result.
                let (fx, fy) = (x as f32 * pscale / cell, y as f32 * pscale / cell);
                let n = value_noise(fx, fy, seed) * (1.0 - rough) + (value_noise(fx * 2.3, fy * 2.3, seed + 7) - 0.0) * rough;
                let l = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
                let g = (n - 0.5) * 2.0 * amt * (0.4 + 2.4 * l * (1.0 - l));
                for c in 0..3 {
                    q[c] += g;
                }
            }
        });
    }
    if !float {
        for q in px.iter_mut() {
            for c in 0..3 {
                q[c] = q[c].clamp(0.0, 1.0);
            }
        }
    }
    (skipped.map_or(Ok(()), Err), unmasked)
}

/// The image's 1st, 50th and 99th percentiles and the mean of the log2 luminance, on cells of
/// a copy at most 384 pixels across (each the log of its pixels' mean
/// luminance, so a preview proxy and the full image agree), interpolated between ranks. `map` is
/// applied to each pixel first. Fully transparent pixels hold no colour and are left out.
fn tone_points(px: &[[f32; 4]], w: usize, h: usize, enc: Encoding, map: impl Fn([f32; 4]) -> [f32; 4] + Sync) -> contrast::Tones {
    const FLOOR: f32 = 1.0 / 65536.0;
    let f = w.max(h).div_ceil(384).max(1);
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    // Each band of `f` rows sums into its own row of cells, in the same order as one pass would.
    let bands = crate::photo_util::par_map(sh, |sy| {
        let mut sum = vec![(0.0f64, 0u32); sw];
        for y in sy * f..((sy + 1) * f).min(h) {
            for x in 0..w {
                let Some(&q) = px.get(y * w + x).filter(|q| q[3] > 0.0) else { continue };
                let q = map(q);
                let s = &mut sum[x / f];
                s.0 += f64::from(enc.luminance([0, 1, 2].map(|c| enc.decode(q[c]))).max(0.0));
                s.1 += 1;
            }
        }
        sum
    });
    let mut cells: Vec<f32> =
        bands.iter().flatten().filter(|s| s.1 > 0).map(|s| ((s.0 / f64::from(s.1)) as f32 + FLOOR).log2()).filter(|v| v.is_finite()).collect();
    if cells.is_empty() {
        return [-16.0, -1.0, 0.0, -1.0];
    }
    let mean = (cells.iter().map(|v| f64::from(*v)).sum::<f64>() / cells.len() as f64) as f32;
    cells.sort_by(f32::total_cmp);
    let pct = |q: f32| {
        let x = (cells.len() - 1) as f32 * q;
        let i = (x as usize).min(cells.len() - 1);
        let j = (i + 1).min(cells.len() - 1);
        cells[i] + (cells[j] - cells[i]) * (x - i as f32)
    };
    [pct(0.01), pct(0.5), pct(0.99), mean]
}

fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |dx: i64, dy: i64| hash01(xi as i64 + dx, yi as i64 + dy, seed);
    let top = h(0, 0) + (h(1, 0) - h(0, 0)) * sx;
    let bot = h(0, 1) + (h(1, 1) - h(0, 1)) * sx;
    top + (bot - top) * sy
}

/// Dehaze below 0 as haze added, fitted by observation on Photoshop 2026's Camera Raw Filter
/// (no Adobe code or data): in linear light each pixel blends toward a haze colour `A` by
/// `w = clamp(w0 + w1·y + w2·y², 0, 1)` of its luminance `y`, where `A` is the mean colour of the
/// 0.1 % of a softened copy (≤ 256 px, Gaussian of 4 px) whose darkest channel is highest — the
/// picture's brightest neutral region. `[w0, w1, w2]` were fitted by least squares on 16
/// photographs at −50 and −100 and scale toward 0 above −50. Mean ΔE76 on each photograph left
/// out of the fit: −100 6.9, −50 4.3 (the dark-channel model run backwards: 11.2, 5.4).
fn add_haze(px: &mut [[f32; 4]], w: usize, h: usize, v: f32, enc: Encoding) {
    const AT_50: [f32; 3] = [0.020, 0.976, -0.432];
    const AT_100: [f32; 3] = [0.104, 2.145, -1.470];
    if !v.is_finite() || v >= 0.0 || px.len() != w.saturating_mul(h) || px.is_empty() {
        return;
    }
    let a = (-v).min(100.0);
    let k: [f32; 3] = if a <= 50.0 { AT_50.map(|x| x * a / 50.0) } else { std::array::from_fn(|i| AT_50[i] + (AT_100[i] - AT_50[i]) * (a - 50.0) / 50.0) };
    // The haze colour from a ≤ 256 px linear copy of the visible pixels (a fully transparent
    // one holds no colour, and one that is not a number would poison the estimate).
    let s = w.max(h).div_ceil(256).max(1);
    let (sw, sh) = (w.div_ceil(s), h.div_ceil(s));
    let mut small = vec![0.0f32; sw * sh * 3];
    let mut count = vec![0.0f32; sw * sh];
    for (i, q) in px.iter().enumerate().filter(|(_, q)| q[3] > 0.0 && q[..3].iter().all(|v| v.is_finite())) {
        let j = (i / w / s) * sw + (i % w) / s;
        for c in 0..3 {
            if let Some(d) = small.get_mut(j * 3 + c) {
                *d += enc.decode(q[c].clamp(0.0, 1.0));
            }
        }
        if let Some(n) = count.get_mut(j) {
            *n += 1.0;
        }
    }
    for (j, n) in count.iter_mut().enumerate() {
        for c in 0..3 {
            small[j * 3 + c] /= n.max(1.0);
        }
        // From here on, whether the cell holds any colour: blurred alongside the colour so
        // empty cells (transparent padding) don't dilute it.
        *n = if *n > 0.0 { 1.0 } else { 0.0 };
    }
    crate::fxutil::gauss_blur_n(&mut small, sw, sh, 3, 4.0);
    crate::fxutil::gauss_blur_n(&mut count, sw, sh, 1, 4.0);
    let mut dark: Vec<(f32, usize)> = (0..sw * sh)
        .filter(|j| count[*j] > 1e-3)
        .map(|j| {
            for c in 0..3 {
                small[j * 3 + c] /= count[j];
            }
            (small[j * 3].min(small[j * 3 + 1]).min(small[j * 3 + 2]), j)
        })
        .collect();
    if dark.is_empty() {
        return;
    }
    dark.sort_by(|x, y| y.0.total_cmp(&x.0));
    let top = (dark.len() / 1000).max(1);
    let mut air = [0.0f32; 3];
    for (_, j) in dark.iter().take(top) {
        for c in 0..3 {
            air[c] += small[j * 3 + c] / top as f32;
        }
    }
    par_rows(px, w, 1, |_, row| {
        for q in row.iter_mut() {
            let lin = [0, 1, 2].map(|c| enc.decode(q[c]));
            let y = luma(lin).clamp(0.0, 1.0);
            let wt = (k[0] + k[1] * y + k[2] * y * y).clamp(0.0, 1.0);
            for c in 0..3 {
                q[c] = enc.encode(lin[c] * (1.0 - wt) + air[c] * wt);
            }
        }
    });
}

fn dehaze(px: &mut [[f32; 4]], w: usize, h: usize, amount: f32) {
    // Transmission on a ≤ 768 px proxy, upsampled.
    let k = w.max(h).div_ceil(768).max(1);
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let mut small = vec![[0.0f32; 3]; sw * sh];
    let mut cnt = vec![0f32; sw * sh];
    for y in 0..h {
        for x in 0..w {
            let i = (y / k) * sw + x / k;
            for c in 0..3 {
                small[i][c] += px[y * w + x][c].clamp(0.0, 1.0);
            }
            cnt[i] += 1.0;
        }
    }
    for (s, c) in small.iter_mut().zip(&cnt) {
        *s = s.map(|v| v / c.max(1.0));
    }
    let r = (sw.max(sh) / 60).max(2);
    // The minimum (or, with `f32::max`, maximum) over each (2r + 1)² window.
    let rank_filter = |v: &[f32], start: f32, pick: fn(f32, f32) -> f32| -> Vec<f32> {
        let mut a = vec![0.0f32; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let (x0, x1) = (x.saturating_sub(r), (x + r).min(sw - 1));
                a[y * sw + x] = (x0..=x1).map(|xx| v[y * sw + xx]).fold(start, pick);
            }
        }
        let mut b = vec![0.0f32; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let (y0, y1) = (y.saturating_sub(r), (y + r).min(sh - 1));
                b[y * sw + x] = (y0..=y1).map(|yy| a[yy * sw + x]).fold(start, pick);
            }
        }
        b
    };
    let min_filter = |v: &[f32]| rank_filter(v, f32::MAX, f32::min);
    let dark = min_filter(&small.iter().map(|c| c[0].min(c[1]).min(c[2])).collect::<Vec<_>>());
    // Atmospheric light: mean colour of the brightest 0.1 % of the dark channel.
    let mut idx: Vec<usize> = (0..sw * sh).collect();
    idx.sort_by(|&a, &b| dark[b].total_cmp(&dark[a]));
    let top = (sw * sh / 1000).max(1);
    let mut air = [0.0f32; 3];
    for &i in &idx[..top] {
        for c in 0..3 {
            air[c] += small[i][c] / top as f32;
        }
    }
    let air = air.map(|v| v.max(0.2));
    let norm_dark = min_filter(&small.iter().map(|c| (c[0] / air[0]).min(c[1] / air[1]).min(c[2] / air[2])).collect::<Vec<_>>());
    // Opened (a maximum filter of the same radius after the minimum): the minimum grows every
    // dark object by `r`, so sky beside a dark building took its dark channel, counted as
    // haze-free and kept a bright halo (#9273). Opening shrinks dark regions back to their
    // outline and is still a lower bound of the dark channel over the window. It raises the dark
    // channel on average, so the 0.95 weight is scaled to keep the slider's overall strength.
    let opened = rank_filter(&norm_dark, f32::MIN, f32::max);
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let weight = 0.95 * mean(&norm_dark) / mean(&opened).max(1e-6);
    let t_raw: Vec<f32> = opened.iter().map(|d| 1.0 - weight * d).collect();
    let guide: Vec<f32> = small.iter().map(|c| luma(*c)).collect();
    let t = guided(&guide, &t_raw, sw, sh, r * 2, 1e-3);
    let ks = k as f32;
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let tv = crate::photo_util::bilinear(&t, sw, sh, 1, 0, (x as f32 + 0.5) / ks - 0.5, (y as f32 + 0.5) / ks - 0.5).clamp(0.1, 1.0);
            for c in 0..3 {
                let i = q[c];
                let j = (i - air[c]) / tv + air[c];
                q[c] = if amount >= 0.0 { i + (j - i) * amount } else { i + (air[c] - i) * (-amount) * 0.5 * (1.0 - tv * 0.5) };
            }
        }
    });
}

fn detail(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, ps: f32) {
    // The controls' documented 0..=100 range: raw values drive the filter radii below.
    let [nl, nld, nc, ncd] = [p.noise_luminance, p.noise_luminance_detail, p.noise_color, p.noise_color_detail].map(|v| v.clamp(0.0, 100.0));
    // Work in Y + chroma (BT.709 luma, B−Y, R−Y).
    let mut y: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]])).collect();
    let mut cb: Vec<f32> = px.iter().zip(&y).map(|(q, l)| q[2] - l).collect();
    let mut cr: Vec<f32> = px.iter().zip(&y).map(|(q, l)| q[0] - l).collect();
    if nl > 0.0 {
        let n = nl / 100.0;
        let r = ((1.0 + 3.0 * n) * ps).round().max(1.0) as usize;
        let smooth = guided(&y, &y, w, h, r, (n * 0.06).powi(2) + 1e-6);
        let keep = nld / 100.0 * 0.5;
        for i in 0..w * h {
            y[i] = smooth[i] + (y[i] - smooth[i]) * keep;
        }
    }
    if nc > 0.0 {
        let sigma = (nc / 100.0 * 6.0 * ps).max(0.3);
        let (bcb, bcr) = (gauss(&cb, w, h, sigma), gauss(&cr, w, h, sigma));
        let keep = ncd / 100.0 * 0.3;
        for i in 0..w * h {
            cb[i] = bcb[i] + (cb[i] - bcb[i]) * keep;
            cr[i] = bcr[i] + (cr[i] - bcr[i]) * keep;
        }
    }
    if p.sharpen_amount > 0.0 {
        let sigma = (p.sharpen_radius.clamp(0.5, 3.0) * ps).max(0.3);
        let blur = gauss(&y, w, h, sigma);
        // Masking: an edge mask from the gradient of a smoothed luminance.
        let mask: Option<Vec<f32>> = (p.sharpen_masking > 0.0).then(|| {
            let s = gauss(&y, w, h, 1.0 * ps.max(0.5));
            let t = p.sharpen_masking / 100.0 * 0.08;
            crate::photo_util::par_map(w * h, |i| {
                let (x, yy) = (i % w, i / w);
                let gx = s[yy * w + (x + 1).min(w - 1)] - s[yy * w + x.saturating_sub(1)];
                let gy = s[(yy + 1).min(h - 1) * w + x] - s[yy.saturating_sub(1) * w + x];
                smoothstep(t * 0.5, t + 1e-4, gx.hypot(gy))
            })
        });
        let amt = p.sharpen_amount / 100.0 * 1.2;
        let det = p.sharpen_detail / 100.0;
        for i in 0..w * h {
            let hp = y[i] - blur[i];
            // Low Detail damps large (halo-producing) differences.
            let limit = 0.03 + det * 0.3;
            let hp = hp.clamp(-limit, limit) * (0.5 + 0.5 * det);
            let m = mask.as_ref().map_or(1.0, |m| m[i]);
            y[i] += amt * hp * m;
        }
    }
    for i in 0..w * h {
        let (l, b, r) = (y[i], cb[i], cr[i]);
        let (rr, bb) = (r + l, b + l);
        let g = (l - 0.2126 * rr - 0.0722 * bb) / 0.7152;
        px[i][0] = rr;
        px[i][1] = g;
        px[i][2] = bb;
    }
}

fn vignette(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw) {
    let Vignette { amount: amt, highlights: hl, style, centre: [cx, cy], axes: [ax, ay], power: pw, edges: [edge0, edge1] } = Vignette::new(w, h, p);
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let (dx, dy) = (((x as f32 + 0.5 - cx) / ax).abs(), ((y as f32 + 0.5 - cy) / ay).abs());
            let r = (dx.powf(pw) + dy.powf(pw)).powf(1.0 / pw);
            let m = smoothstep(edge0, edge1, r);
            if m <= 0.0 {
                continue;
            }
            let l = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
            match style {
                VignetteStyle::PaintOverlay => {
                    let target = if amt < 0.0 { 0.0 } else { 1.0 };
                    let k = m * amt.abs();
                    for c in 0..3 {
                        q[c] += (target - q[c]) * k;
                    }
                }
                _ => {
                    let mut k = 2f32.powf(amt * 2.0 * m);
                    if amt < 0.0 && hl > 0.0 {
                        // Highlights: let bright areas punch through a darkening vignette.
                        let protect = smoothstep(0.6, 1.0, l) * hl;
                        k += (1.0 - k) * protect;
                    }
                    if style == VignetteStyle::ColorPriority {
                        let nl = l * k;
                        for c in 0..3 {
                            q[c] += nl - l;
                        }
                    } else {
                        for c in 0..3 {
                            q[c] *= k;
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #9273: Dehaze clears the haze evenly over a sky, right up to a dark building, instead of
    /// leaving a bright halo of uncorrected haze around it. Synthetic scene: a clear sky and a
    /// dark tower, hazed with `I = J·t + A·(1 − t)`.
    #[test]
    fn adding_haze_flattens_toward_the_brightest_neutral_and_is_safe() {
        let (w, h) = (200usize, 120usize);
        let base: Vec<[f32; 4]> = (0..w * h).map(|i| if (i % w) < 100 { [0.05, 0.08, 0.1, 1.0] } else { [0.9, 0.88, 0.85, 1.0] }).collect();
        let mut px = base.clone();
        add_haze(&mut px, w, h, -100.0, Encoding::of(0.0));
        assert!(px[0][0] > base[0][0] + 0.02, "{:?}", px[0]);
        assert!(px.iter().all(|q| q[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))));
        let mut half = base.clone();
        add_haze(&mut half, w, h, -50.0, Encoding::of(1.8));
        assert!(half[0][0] > base[0][0] && half[0][0] < px[0][0] + 0.2);
        for v in [0.0, 10.0, f32::NAN] {
            let mut same = base.clone();
            add_haze(&mut same, w, h, v, Encoding::of(0.0));
            assert_eq!(same, base);
        }
        add_haze(&mut [], 0, 0, -50.0, Encoding::of(0.0));
        let mut odd = vec![[f32::NAN, 2.0, -1.0, 1.0]; 4];
        add_haze(&mut odd, 2, 2, -80.0, Encoding::of(0.0));
    }

    /// The colour hidden under fully transparent pixels takes no part in the haze colour or in
    /// the tones Contrast and Whites adapt to.
    #[test]
    fn transparent_pixels_do_not_count() {
        let (w, h) = (64usize, 64usize);
        let visible = |i: usize| i % w < 32;
        let scene = |hidden: [f32; 4]| -> Vec<[f32; 4]> {
            (0..w * h).map(|i| if visible(i) { [0.2 + 0.6 * ((i / w) as f32 / h as f32), 0.3, 0.25, 1.0] } else { hidden }).collect()
        };
        let (white, black) = (scene([1.0, 1.0, 1.0, 0.0]), scene([0.0, 0.0, 0.0, 0.0]));
        assert_eq!(tone_points(&white, w, h, Encoding::of(0.0), |q| q), tone_points(&black, w, h, Encoding::of(0.0), |q| q));
        let (mut a, mut b) = (white.clone(), black.clone());
        add_haze(&mut a, w, h, -100.0, Encoding::of(0.0));
        add_haze(&mut b, w, h, -100.0, Encoding::of(0.0));
        for i in (0..w * h).filter(|i| visible(*i)) {
            assert_eq!(a[i], b[i], "pixel {i}");
        }
        // Everything transparent: the fallback tones.
        let clear = vec![[0.5, 0.5, 0.5, 0.0]; w * h];
        assert_eq!(tone_points(&clear, w, h, Encoding::of(0.0), |q| q), [-16.0, -1.0, 0.0, -1.0]);
    }

    /// A pixel that is not a number takes no part in the haze colour, so negative Dehaze
    /// can't turn the rest of a float image into NaN.
    #[test]
    fn a_nan_pixel_stays_out_of_the_haze_colour() {
        let (w, h) = (64usize, 64usize);
        let base: Vec<[f32; 4]> = (0..w * h).map(|i| [0.2 + 0.6 * ((i % w) as f32 / w as f32), 0.4, 0.3, 1.0]).collect();
        let (mut with_nan, mut hidden) = (base.clone(), base.clone());
        with_nan[100] = [0.5, f32::NAN, 0.5, 1.0];
        hidden[100] = [0.5, 0.5, 0.5, 0.0];
        add_haze(&mut with_nan, w, h, -100.0, Encoding::of(0.0));
        add_haze(&mut hidden, w, h, -100.0, Encoding::of(0.0));
        for i in (0..w * h).filter(|i| *i != 100) {
            assert!(with_nan[i][..3].iter().all(|v| v.is_finite()), "pixel {i}: {:?}", with_nan[i]);
            assert_eq!(with_nan[i], hidden[i], "pixel {i}");
        }
    }

    /// Transparent padding around a small object doesn't dilute the haze colour: the object
    /// comes out as it does when it fills the canvas.
    #[test]
    fn transparent_padding_does_not_darken_the_haze_colour() {
        let (w, h) = (512usize, 512usize);
        let object = [0.9, 0.85, 0.8, 1.0];
        // In the middle: at a canvas edge the blur repeats the edge cells and hides the padding.
        let inside = |i: usize| (240..272).contains(&(i % w)) && (240..272).contains(&(i / w));
        let mut padded: Vec<[f32; 4]> = (0..w * h).map(|i| if inside(i) { object } else { [0.0; 4] }).collect();
        let mut full = vec![object; w * h];
        add_haze(&mut padded, w, h, -100.0, Encoding::of(0.0));
        add_haze(&mut full, w, h, -100.0, Encoding::of(0.0));
        for i in (0..w * h).filter(|i| inside(*i)) {
            for c in 0..3 {
                assert!((padded[i][c] - full[i][c]).abs() < 1e-4, "pixel {i}: {:?} vs {:?}", padded[i], full[i]);
            }
        }
    }

    /// Fewer pixels than the dimensions claim: no panic, the pixels unchanged.
    #[test]
    fn a_short_buffer_is_left_alone() {
        let px = vec![[0.5, 0.4, 0.3, 1.0]; 3];
        assert!(tone_points(&px, 2, 2, Encoding::of(0.0), |q| q).iter().all(|v| v.is_finite()));
        let mut short = px.clone();
        develop(&mut short, 2, 2, &CameraRaw { contrast: 50.0, whites: 40.0, ..Default::default() }, false);
        assert_eq!(short, px);
    }

    #[test]
    fn dehaze_leaves_no_halo_beside_dark_objects() {
        let (w, h) = (600usize, 400usize);
        let tower = |x: usize, y: usize| (250..350).contains(&x) && y >= 120;
        let (t, air) = (0.55f32, [0.9f32, 0.9, 0.9]);
        let mut px: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let j = if tower(x, y) { [0.06, 0.06, 0.07] } else { [0.35 + 0.1 * y as f32 / h as f32, 0.5, 0.8] };
                let c: [f32; 3] = std::array::from_fn(|k| j[k] * t + air[k] * (1.0 - t));
                [c[0], c[1], c[2], 1.0]
            })
            .collect();
        dehaze(&mut px, w, h, 1.0);
        let sky = |xs: std::ops::Range<usize>| {
            let v: Vec<f32> =
                (150..390).flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| luma([px[y * w + x][0], px[y * w + x][1], px[y * w + x][2]])).collect();
            v.iter().sum::<f32>() / v.len() as f32
        };
        // Sky 1–8 px from the tower against open sky: the haze left beside it, in 8-bit levels.
        let halo = (sky(242..250) - sky(20..80)) * 255.0;
        // 6.4 levels before the dark channel was opened, under 1 after.
        assert!(halo < 3.0, "a {halo:.1}-level halo beside the tower");
        // The open sky is still cleared: darker than the hazy input.
        let hazy = luma(std::array::from_fn(|k| [0.4, 0.5, 0.8][k] * t + air[k] * (1.0 - t)));
        assert!(sky(20..80) < hazy - 0.02, "{} vs hazy {hazy}", sky(20..80));
    }

    #[test]
    fn new_curves_are_validated_but_saved_curves_still_load_and_render() {
        for field in ["pointCurve", "pointCurveRed", "pointCurveGreen", "pointCurveBlue"] {
            for points in [
                serde_json::json!([[128, 128]]),
                serde_json::json!([[255, 255], [0, 0]]),
                serde_json::json!([[0, 0], [0, 255]]),
                serde_json::json!([[0, -1], [255, 255]]),
                serde_json::json!([[0, 0], [256, 255]]),
                serde_json::json!([[0, 0], [1e30, 255]]),
                serde_json::json!((0..17).map(|i| [i * 15, i * 15]).collect::<Vec<_>>()),
            ] {
                let p: CameraRaw = serde_json::from_value(serde_json::json!({field: points})).unwrap();
                assert!(p.validate().is_err(), "{field}: {points}");
            }
            for points in
                [serde_json::json!([]), serde_json::json!([[0, 0], [255, 255]]), serde_json::json!((0..16).map(|i| [i * 17, i * 17]).collect::<Vec<_>>())]
            {
                let p: CameraRaw = serde_json::from_value(serde_json::json!({field: points})).unwrap();
                assert!(p.validate().is_ok(), "{field}: {points}");
            }
        }
        assert_eq!(serde_json::from_str::<CameraRaw>("{}").unwrap(), CameraRaw::default());
        // An older editor could stack two points on one input and exceed 16 points; such a
        // saved Smart Filter must still develop instead of silently disappearing.
        let mut legacy: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 12.0, i as f32 * 12.0]).collect();
        legacy.insert(5, [60.0, 200.0]);
        let p: CameraRaw = serde_json::from_value(serde_json::json!({"pointCurve": legacy})).unwrap();
        assert!(p.validate().is_err());
        let edited = CameraRaw { exposure: 1.0, ..p.clone() };
        assert!(edited.validate_changes(&p).is_ok(), "an unchanged stored curve stays editable");
        let broken = CameraRaw { point_curve: vec![[10.0, 10.0]], ..p.clone() };
        assert!(broken.validate_changes(&p).is_err(), "a changed curve is validated");
        let mut px = img(4, 4);
        develop(&mut px, 4, 4, &p, false);
        assert!(px.iter().flatten().all(|v| v.is_finite()));
    }

    fn img(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [0.2 + 0.6 * x, 0.3 + 0.4 * y, 0.5 - 0.3 * x * y, 1.0]
            })
            .collect()
    }

    fn mean(px: &[[f32; 4]], c: usize) -> f32 {
        px.iter().map(|q| q[c]).sum::<f32>() / px.len() as f32
    }

    #[test]
    fn black_and_white_greys_the_image_and_bad_treatments_are_rejected() {
        let p = CameraRaw { treatment: "bw".into(), ..Default::default() };
        assert!(p.is_bw() && !p.is_identity() && p.validate().is_ok());
        let mut a = img(24, 16);
        develop(&mut a, 24, 16, &p, false);
        assert!(a.iter().all(|q| q[0] == q[1] && q[1] == q[2] && (0.0..=1.0).contains(&q[0])));
        // B&W ignores Vibrance, Saturation and the Color Mixer, as Camera Raw does.
        let mut b = img(24, 16);
        develop(&mut b, 24, 16, &CameraRaw { saturation: 80.0, vibrance: -40.0, hsl_lum: [60.0; 8], ..p.clone() }, false);
        assert_eq!(a, b);
        // The GPU stages leave B&W to the CPU develop.
        assert!(per_pixel(&p, 24, 16, false, None).is_none());
        for bad in [
            CameraRaw { treatment: "sepia".into(), ..Default::default() },
            CameraRaw { treatment: "BW".into(), ..Default::default() },
            CameraRaw { bw_mix: [f32::NAN; 8], ..Default::default() },
        ] {
            assert!(bad.validate().is_err() && bad.validate_changes(&CameraRaw::default()).is_err(), "{:?}", bad.treatment);
        }
        // Older settings without the fields load as colour.
        let old: CameraRaw = serde_json::from_str(r#"{"exposure": 0.5}"#).expect("json");
        assert!(!old.is_bw() && old.bw_mix == [0.0; 8]);
    }

    #[test]
    fn defaults_are_identity() {
        let mut a = img(40, 30);
        let b = a.clone();
        develop(&mut a, 40, 30, &CameraRaw::default(), false);
        assert_eq!(a, b);
    }

    #[test]
    fn vibrance_follows_photoshop() {
        // Photoshop 2026, Vibrance +100: (0.55, 0.48, 0.44) → (0.592, 0.475, 0.400) and
        // (0.90, 0.15, 0.10) → (0.937, 0, 0).
        let mut px = vec![[0.55, 0.48, 0.44, 1.0], [0.9, 0.15, 0.1, 1.0]];
        develop(&mut px, 2, 1, &CameraRaw { vibrance: 100.0, ..Default::default() }, false);
        // The strong red sits on the gamut's edge, where the tables interpolate least well.
        let want = [([0.592, 0.475, 0.400], 0.03), ([0.937, 0.0, 0.0], 0.08)];
        for (q, (w, tol)) in px.iter().zip(want) {
            assert!(q.iter().zip(w).all(|(a, b)| (a - b).abs() < tol), "{px:?}");
        }
    }

    #[test]
    fn basic_panel_moves_the_right_way() {
        let (w, h) = (64, 48);
        let base = img(w, h);
        let run = |p: CameraRaw| {
            let mut a = base.clone();
            develop(&mut a, w, h, &p, false);
            a
        };
        let warm = run(CameraRaw { temperature: 50.0, ..Default::default() });
        assert!(mean(&warm, 0) > mean(&base, 0) && mean(&warm, 2) < mean(&base, 2));
        let bright = run(CameraRaw { exposure: 1.0, ..Default::default() });
        assert!(mean(&bright, 1) > mean(&base, 1) + 0.1);
        let shadows = run(CameraRaw { shadows: 100.0, ..Default::default() });
        let dark_px = |a: &[[f32; 4]]| a[0][1];
        assert!(dark_px(&shadows) > dark_px(&base));
        let hl = run(CameraRaw { highlights: -100.0, ..Default::default() });
        assert!(hl[w * h - 1][0] < base[w * h - 1][0]);
        let con = run(CameraRaw { contrast: 100.0, ..Default::default() });
        // More contrast: darker darks, brighter brights.
        assert!(con[0][1] < base[0][1] && con[w * h - 1][0] > base[w * h - 1][0]);
        let desat = run(CameraRaw { saturation: -100.0, ..Default::default() });
        assert!(desat.iter().all(|q| (q[0] - q[1]).abs() < 1e-4 && (q[1] - q[2]).abs() < 1e-4));
        let vib = run(CameraRaw { vibrance: 100.0, ..Default::default() });
        let spread = |q: [f32; 4]| q[0].max(q[1]).max(q[2]) - q[0].min(q[1]).min(q[2]);
        assert!(spread(vib[w * h / 2]) > spread(base[w * h / 2]));
        for p in [
            CameraRaw { clarity: 80.0, ..Default::default() },
            CameraRaw { texture: 80.0, ..Default::default() },
            CameraRaw { dehaze: 60.0, ..Default::default() },
            CameraRaw { dehaze: -60.0, ..Default::default() },
            CameraRaw { whites: 50.0, blacks: -50.0, ..Default::default() },
        ] {
            let out = run(p.clone());
            assert!(out.iter().all(|q| q[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))), "{p:?}");
            assert_ne!(out, base);
        }
    }

    #[test]
    fn curves_hsl_grading_detail_effects() {
        // Monotone cubic LUT through the points.
        let lut = curve_lut(&[[0.0, 0.0], [64.0, 40.0], [192.0, 220.0], [255.0, 255.0]], 256);
        assert!((lut[64] - 40.0 / 255.0).abs() < 0.01 && (lut[192] - 220.0 / 255.0).abs() < 0.01);
        assert!(lut.windows(2).all(|p| p[1] >= p[0]));
        let (w, h) = (48, 32);
        let reds: Vec<[f32; 4]> = vec![[0.8, 0.2, 0.2, 1.0]; w * h];
        let mut a = reds.clone();
        let mut hs = [0.0; 8];
        hs[0] = -100.0;
        develop(&mut a, w, h, &CameraRaw { hsl_sat: hs, ..Default::default() }, false);
        // Photoshop: (0.81, 0.19, 0.19) → (0.59, 0.42, 0.42); Reds alone never grey a red fully.
        assert!(a[0][0] - a[0][1] < 0.25 && a[0][0] - a[0][1] > 0.08 && a[0][0] < 0.65, "reds desaturated: {:?}", a[0]);
        let blues: Vec<[f32; 4]> = vec![[0.2, 0.2, 0.8, 1.0]; w * h];
        let mut b = blues.clone();
        develop(&mut b, w, h, &CameraRaw { hsl_sat: hs, ..Default::default() }, false);
        assert!((b[0][2] - 0.8).abs() < 2e-3 && (b[0][0] - 0.2).abs() < 2e-3 && (b[0][1] - 0.2).abs() < 2e-3, "blues untouched: {:?}", b[0]);
        // Grading: blue shadows tint dark pixels, leave bright ones nearly alone.
        let mut g: Vec<[f32; 4]> = vec![[0.1, 0.1, 0.1, 1.0], [0.95, 0.95, 0.95, 1.0]];
        develop(&mut g, 2, 1, &CameraRaw { grade_shadows: Wheel { hue: 220.0, sat: 80.0, lum: 0.0 }, ..Default::default() }, false);
        assert!(g[0][2] > g[0][0] + 0.05 && (g[1][2] - g[1][0]).abs() < 0.02, "{g:?}");
        // Point curve + parametric.
        let mut c = img(w, h);
        develop(&mut c, w, h, &CameraRaw { point_curve: vec![[0.0, 30.0], [255.0, 255.0]], curve_shadows: 50.0, ..Default::default() }, false);
        assert!(c.iter().all(|q| q[1] >= 30.0 / 255.0 - 1e-4));
        // Noise reduction reduces variance; sharpening increases it; grain is deterministic.
        let noisy: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let n = hash01(i as i64, 0, 3) * 0.2;
                [0.4 + n, 0.4 + n, 0.4 + n, 1.0]
            })
            .collect();
        let var = |a: &[[f32; 4]]| {
            let m = mean(a, 1);
            a.iter().map(|q| (q[1] - m).powi(2)).sum::<f32>() / a.len() as f32
        };
        let mut nr = noisy.clone();
        develop(&mut nr, w, h, &CameraRaw { noise_luminance: 80.0, ..Default::default() }, false);
        assert!(var(&nr) < var(&noisy) * 0.5);
        // Out-of-range noise settings are held to their 0..=100 range, not turned into huge radii.
        let mut wild = noisy.clone();
        let hostile =
            CameraRaw { noise_luminance: 1.0e30, noise_color: 1.0e30, noise_luminance_detail: 1.0e30, noise_color_detail: 1.0e30, ..Default::default() };
        develop(&mut wild, w, h, &hostile, false);
        assert!(wild.iter().all(|q| q[0].is_finite()));
        let mut sh = noisy.clone();
        develop(&mut sh, w, h, &CameraRaw { sharpen_amount: 100.0, ..Default::default() }, false);
        assert!(var(&sh) > var(&noisy));
        let mut g1 = img(w, h);
        let mut g2 = img(w, h);
        let gp = CameraRaw { grain_amount: 50.0, seed: 4, ..Default::default() };
        develop(&mut g1, w, h, &gp, false);
        develop(&mut g2, w, h, &gp, false);
        assert_eq!(g1, g2);
        assert_ne!(g1, img(w, h));
        // Post-crop vignette darkens corners, leaves the centre.
        let flat = vec![[0.5f32, 0.5, 0.5, 1.0]; w * h];
        let mut v = flat.clone();
        develop(&mut v, w, h, &CameraRaw { vignette_amount: -80.0, ..Default::default() }, false);
        assert!(v[0][0] < 0.35 && (v[(h / 2) * w + w / 2][0] - 0.5).abs() < 0.01, "{:?}", v[0]);
    }

    #[test]
    fn calibration_keeps_the_develop_off_the_per_pixel_gpu_stages() {
        // The per-pixel stages (the Camera Raw dialog's GPU preview) don't run Calibration, so
        // settings with it must fall back to the CPU develop instead of previewing without it.
        for p in [
            CameraRaw { shadow_tint: 40.0, ..Default::default() },
            CameraRaw { calibration_hue: [0.0, 0.0, 30.0], whites: 20.0, ..Default::default() },
            CameraRaw { calibration_sat: [-50.0, 0.0, 0.0], exposure: 0.5, ..Default::default() },
        ] {
            assert!(!p.spatial());
            assert!(per_pixel(&p, 2, 2, false, Some([[-8.0, -2.0, 0.0, -3.0]; 2])).is_none(), "{p:?}");
        }
        assert!(per_pixel(&CameraRaw { exposure: 0.5, ..Default::default() }, 2, 2, false, None).is_some());
    }

    #[test]
    fn calibration_turns_primaries_keeps_neutrals_and_tints_shadows() {
        let w = calibration_weights(0.0);
        assert!((w[0] - 1.0).abs() < 1e-6, "pure red is all red primary: {w:?}");
        let w = calibration_weights(60.0);
        assert!((w[0] - 0.5).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6, "yellow is half red, half green: {w:?}");
        let w = calibration_weights(300.0);
        assert!((w[2] - 0.5).abs() < 1e-6 && (w[0] - 0.5).abs() < 1e-6, "magenta wraps round: {w:?}");
        let (none, zero) = ([0.0f32; 3], [0.0f32; 3]);
        let grey = calibrate([0.3, 0.3, 0.3], [100.0, -100.0, 50.0], [100.0, -100.0, 100.0]);
        assert!(grey.iter().all(|v| (v - 0.3).abs() < 1e-5), "greys are untouched: {grey:?}");
        // Red turned towards green: it gains green, keeps its luminance.
        let red = calibrate([0.5, 0.0, 0.0], [100.0, 0.0, 0.0], zero);
        assert!(red[1] > 0.05 && (red[1] / red[0] - 0.5).abs() < 0.02 && red[2] < 1e-6, "red turns 30° towards green (orange): {red:?}");
        assert!((luma(red) - luma([0.5, 0.0, 0.0])).abs() < 1e-4, "luminance kept: {red:?}");
        // Blue at saturation −100 is grey; green at −50 is paler.
        let blue = calibrate([0.0, 0.0, 0.8], none, [0.0, 0.0, -100.0]);
        assert!((blue[0] - blue[1]).abs() < 1e-5 && (blue[1] - blue[2]).abs() < 1e-5, "blue at −100 saturation is grey: {blue:?}");
        let green = calibrate([0.2, 0.8, 0.2], none, [0.0, -50.0, 0.0]);
        assert!(green[0] > 0.2 && green[1] < 0.8, "green at −50 is paler: {green:?}");
        // A primary's sliders leave the other primaries alone.
        let unchanged = calibrate([0.0, 0.0, 0.8], [100.0, 100.0, 0.0], [100.0, 100.0, 0.0]);
        assert!(unchanged.iter().zip([0.0, 0.0, 0.8]).all(|(a, b)| (a - b).abs() < 1e-5), "{unchanged:?}");
        // Shadows tint: magenta lifts red and blue over green in a dark pixel, keeps luminance,
        // and leaves white alone.
        let dark = shadow_tint([0.02, 0.02, 0.02], 1.0);
        assert!(dark[0] > dark[1] && dark[2] > dark[1], "{dark:?}");
        assert!((luma(dark) - 0.02).abs() < 1e-4, "luminance kept: {dark:?}");
        let green = shadow_tint([0.02, 0.02, 0.02], -1.0);
        assert!(green[1] > green[0], "{green:?}");
        let white = shadow_tint([1.0, 1.0, 1.0], 1.0);
        assert!(white.iter().all(|v| (v - 1.0).abs() < 1e-3), "white is not tinted: {white:?}");
        // Through develop: neutral settings are an identity, a tint changes dark pixels only.
        let mut p = CameraRaw::default();
        assert!(p.is_identity());
        p.shadow_tint = 100.0;
        assert!(!p.is_identity());
        let mut px = vec![[0.1, 0.1, 0.1, 1.0], [0.95, 0.95, 0.95, 1.0]];
        develop(&mut px, 2, 1, &p, false);
        assert!(px[0][0] > px[0][1], "dark grey turns magenta: {:?}", px[0]);
        assert!((px[1][0] - px[1][1]).abs() < 0.01, "near-white stays neutral: {:?}", px[1]);
        p = CameraRaw::default();
        p.calibration_hue[2] = 100.0;
        let mut px = vec![[0.0, 0.0, 0.8, 1.0]];
        develop(&mut px, 1, 1, &p, false);
        assert!(px[0][0] > 0.05, "blue turned towards red: {:?}", px[0]);
    }

    #[test]
    fn auto_tone_brightens_dark_and_darkens_bright_images() {
        let mut p = CameraRaw::default();
        auto_tone(&[], &mut p);
        assert_eq!(p, CameraRaw::default(), "no pixels: nothing changes");
        let dark: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.05 + (i % 64) as f32 / 64.0 * 0.15;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&dark, &mut p);
        assert!(p.exposure > 0.5, "a dark image gets exposure: {}", p.exposure);
        assert!(p.shadows > 0.0, "and lifted shadows: {}", p.shadows);
        let mut p = CameraRaw::default();
        let bright: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.8 + (i % 64) as f32 / 64.0 * 0.19;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&bright, &mut p);
        assert!(p.exposure < -0.5, "a bright image loses exposure: {}", p.exposure);
        assert!(p.highlights < 0.0, "and its highlights come down: {}", p.highlights);
        let mut p = CameraRaw::default();
        let good: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.04 + (i % 64) as f32 / 63.0 * 0.92;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&good, &mut p);
        assert!(p.exposure.abs() < 0.3 && p.blacks.abs() < 10.0 && p.whites.abs() < 10.0, "a full-range image is left nearly alone: {p:?}");
        assert!(p.saturation == 0.0 && p.hsl_neutral(), "other panels untouched");
    }

    #[test]
    fn float_documents_keep_overrange() {
        let mut a = vec![[2.0f32, 1.5, 0.5, 1.0]; 16];
        develop(&mut a, 4, 4, &CameraRaw { exposure: 0.5, ..Default::default() }, true);
        // Exposure keeps the excess over white (no clipping) and brightens the rest.
        assert!(a[0][0] > 1.9 && a[0][1] > 1.4 && a[0][2] > 0.5, "{:?}", a[0]);
    }

    /// A Gaussian narrower than 0.3 px returns its input unchanged.
    #[test]
    fn a_gaussian_below_0_3_px_leaves_the_image_alone() {
        let (w, h) = (64usize, 48usize);
        let v: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 101) as f32 / 100.0).collect();
        assert_eq!(gauss(&v, w, h, 0.29), v);
        assert_ne!(gauss(&v, w, h, 1.0), v);
    }

    /// Display P3 (D65) linear RGB to linear ProPhoto (D50): P3's RGB → XYZ, Bradford D65 → D50,
    /// then XYZ → ROMM RGB (published matrices).
    fn p3_to_prophoto() -> [[f32; 3]; 3] {
        let p3_xyz = [[0.486_570_9, 0.265_667_7, 0.198_217_3], [0.228_974_6, 0.691_738_5, 0.079_286_9], [0.0, 0.045_113_4, 1.043_944_4]];
        let d65_d50 = [[1.047_811_2, 0.022_886_6, -0.050_127_0], [0.029_542_4, 0.990_484_4, -0.017_049_1], [-0.009_234_5, 0.015_043_6, 0.752_131_6]];
        let xyz_romm = [[1.345_943_3, -0.255_607_5, -0.051_111_8], [-0.544_598_9, 1.508_167_3, 0.020_535_1], [0.0, 0.0, 1.211_812_8]];
        let mul =
            |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] { std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum())) };
        mul(xyz_romm, mul(d65_d50, p3_xyz))
    }

    /// A Display P3 document has sRGB's curve, so its curve alone made it read as sRGB: its own
    /// primaries now give its luminance and its conversion into sRGB, and an out-of-gamut colour
    /// survives the round trip.
    #[test]
    fn display_p3_is_not_read_as_srgb() {
        let p3 = Encoding::with_primaries(0.0, Some(p3_to_prophoto()));
        assert!(!p3.srgb_primaries());
        assert_ne!(p3.prophoto(), Encoding::of(0.0).prophoto());
        // P3's red is brighter than sRGB's, its green darker; white is white in both.
        assert!(p3.luminance([1.0, 0.0, 0.0]) > luma([1.0, 0.0, 0.0]) + 0.01);
        assert!(p3.luminance([0.0, 1.0, 0.0]) < luma([0.0, 1.0, 0.0]) - 0.01);
        assert!((p3.luminance([1.0; 3]) - 1.0).abs() < 2e-3);
        // P3's pure red lies outside sRGB: past 1 in red, below 0 in green and blue, and back.
        let s = p3.to_srgb_encoded([1.0, 0.0, 0.0]);
        assert!(s[0] > 1.0 && s[1] < 0.0 && s[2] < 0.0, "{s:?}");
        let back = p3.back_from_srgb_encoded(s);
        assert!(back.iter().zip([1.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-4), "{back:?}");
        // sRGB documents convert as before, exactly.
        let srgb = CameraRaw::default().encoding();
        assert_eq!(srgb.to_srgb_encoded([0.3, 1.2, -0.1]), [0.3, 1.2, -0.1]);
        assert_eq!(srgb.luminance([0.2, 0.5, 0.9]).to_bits(), luma([0.2, 0.5, 0.9]).to_bits());
        // A matrix that can't be inverted falls back to the primaries the curve implies.
        assert_eq!(Encoding::with_primaries(1.8, Some([[0.0; 3]; 3])), Encoding::of(1.8));
    }

    /// The same picture in ProPhoto, or in Display P3, develops as it does in sRGB: the
    /// sRGB-measured tables and models read its colours in sRGB's primaries, and Contrast
    /// weighs its luminance in its own.
    #[test]
    fn other_primaries_develop_like_the_same_picture_in_srgb() {
        let (w, h) = (96, 64);
        let srgb: Vec<[f32; 4]> = img(w, h)
            .into_iter()
            .enumerate()
            .map(|(i, q)| {
                // Fine detail for Texture and Clarity, and colour that Contrast keeps within
                // sRGB's gamut (outside it the sRGB document clips where the others don't).
                let t = (((i % w) / 3 + (i / w) / 3) % 2) as f32 * 0.12;
                let c = [q[0] + t * 0.5, (q[1] - t * 0.8).max(0.05), q[2] + t];
                let m = (c[0] + c[1] + c[2]) / 3.0;
                let d = |k: usize| m + (c[k] - m) * 0.6;
                [d(0), d(1), d(2), 1.0]
            })
            .collect();
        for p in [
            CameraRaw { contrast: 45.0, ..Default::default() },
            CameraRaw { whites: -60.0, ..Default::default() },
            CameraRaw { texture: 70.0, ..Default::default() },
            CameraRaw { clarity: 40.0, ..Default::default() },
            CameraRaw { contrast: 45.0, whites: -60.0, texture: 70.0, clarity: 40.0, ..Default::default() },
        ] {
            let mut want = srgb.clone();
            // As 32-bit documents: the final clip to 0 … 1 would cut each document's gamut apart.
            develop(&mut want, w, h, &p, true);
            let to_lin = |q: &[f32; 4]| [0, 1, 2].map(|c| srgb_to_linear(q[c]));
            for (gamma, primaries) in [(1.8, None), (0.0, Some(p3_to_prophoto()))] {
                let doc = CameraRaw { encoding_gamma: gamma, document_primaries: primaries, ..p.clone() };
                let enc = doc.encoding();
                let (to, from) = enc.prophoto().unwrap_or((exposure::IDENTITY, exposure::IDENTITY));
                // sRGB linear → ProPhoto → the document's linear RGB, and encoded.
                let mut px: Vec<[f32; 4]> = srgb
                    .iter()
                    .map(|q| {
                        let d = exposure::mat(&from, exposure::mat(&exposure::SRGB_TO_PROPHOTO, to_lin(q)));
                        [enc.encode(d[0]), enc.encode(d[1]), enc.encode(d[2]), 1.0]
                    })
                    .collect();
                develop(&mut px, w, h, &doc, true);
                let mut worst = 0.0f32;
                for (q, r) in px.iter().zip(&want) {
                    let back = exposure::mat(&exposure::PROPHOTO_TO_SRGB, exposure::mat(&to, [0, 1, 2].map(|c| enc.decode(q[c]))));
                    for c in 0..3 {
                        worst = worst.max((linear_to_srgb(back[c]) - r[c]).abs());
                    }
                }
                assert!(worst < 3e-3, "{p:?} gamma {gamma}: off by {worst}");
            }
        }
    }
}
