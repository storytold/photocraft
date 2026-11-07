//! Develop's per-pixel stages resolved for one image: every constant and table they use, so
//! another implementation of them (the GPU preview, `photocraft_gpu::CameraRawDevelop`) runs the
//! same maths. [`develop`](super::develop) builds its own constants with the same functions.

use super::whites_blacks::{RowOn, TonePoints};
use super::{CameraRaw, Encoding, Wheel, contrast, curve_lut, exposure, hsv_to_rgb, is_linear, luma, parametric_lut, whites_blacks};

/// Entries of the tone curve tables (parametric and point curves).
pub const CURVE_LUT: usize = 1024;

/// Samples of the Exposure and Contrast curve rows: grey levels 0, 1/64, …, 1.
pub const TONE_STEPS: usize = exposure::STEPS;

/// Nodes per channel of the measured colour correction grid ([`WhitesBlacks::whites_fix`]).
pub const FIX_GRID: usize = super::colour_lut::N;
/// Vibrance's and Saturation's tables' nodes: hues, saturations, values (see
/// [`PerPixel::presence`]).
pub const PRESENCE_GRID: [usize; 3] = [super::presence::GRID.hues, super::presence::GRID.sats, super::presence::GRID.vals];
/// Tables in [`PerPixel::presence`].
pub const PRESENCE_KINDS: usize = super::presence::KINDS;
/// The Color Mixer table's nodes: hues, saturations, values (see [`PerPixel::hsl`]).
pub const MIXER_GRID: [usize; 3] = [super::color_mixer::HUES, super::color_mixer::SATS, super::color_mixer::VALS];
/// Tables in [`PerPixel::hsl`].
pub const MIXER_KINDS: usize = super::color_mixer::KINDS;

/// Whites and Blacks as two curve rows over sRGB-encoded grey levels 0, 1/64, …, 1, with how
/// each acts (each channel with the hue kept, or the luminance).
#[derive(Clone, Debug, PartialEq)]
pub struct WhitesBlacks {
    /// The Whites curve, when Whites is set.
    pub whites: Option<([f32; TONE_STEPS], RowOn)>,
    /// The Blacks curve, applied after Whites.
    pub blacks: Option<([f32; TONE_STEPS], RowOn)>,
    /// Whites down's measured colour correction, added right after the Whites curve:
    /// sRGB-encoded RGB deltas on a [`FIX_GRID`]³ grid (node `(r · n + g) · n + b`), looked up
    /// trilinearly at the pixel before the Whites curve; a grey result takes the deltas' mean,
    /// and a result whose input was within 0 … 1 is kept to 0 … 1.
    pub whites_fix: Option<Vec<[f32; 3]>>,
}

/// Color Grading's wheels resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Grade {
    /// Colour offsets of the shadows, midtones, highlights and global wheels.
    pub tints: [[f32; 3]; 4],
    /// Luminance of the same wheels (slider units).
    pub lums: [f32; 4],
    pub blend: f32,
    pub balance: f32,
}

impl Grade {
    pub(crate) fn new(p: &CameraRaw) -> Self {
        let tint = |wh: &Wheel| -> [f32; 3] {
            let c = hsv_to_rgb(wh.hue, 1.0, 1.0);
            let l = luma(c);
            [c[0] - l, c[1] - l, c[2] - l].map(|v| v * wh.sat / 100.0 * 0.3)
        };
        Grade {
            tints: [tint(&p.grade_shadows), tint(&p.grade_midtones), tint(&p.grade_highlights), tint(&p.grade_global)],
            lums: [p.grade_shadows.lum, p.grade_midtones.lum, p.grade_highlights.lum, p.grade_global.lum],
            blend: 0.15 + p.grade_blending.clamp(0.0, 100.0) / 100.0 * 0.35,
            balance: p.grade_balance.clamp(-100.0, 100.0) / 100.0 * 0.25,
        }
    }
}

/// The vignette's `vignette_style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VignetteStyle {
    HighlightPriority,
    ColorPriority,
    PaintOverlay,
}

/// The post-crop vignette's shape on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Vignette {
    /// −1 … 1.
    pub amount: f32,
    /// 0 … 1.
    pub highlights: f32,
    pub style: VignetteStyle,
    /// Frame centre (pixels).
    pub centre: [f32; 2],
    /// Half axes of the shape (pixels).
    pub axes: [f32; 2],
    /// Exponent of the superellipse.
    pub power: f32,
    /// The mask's smoothstep edges in normalised radius.
    pub edges: [f32; 2],
}

impl Vignette {
    pub(crate) fn new(w: usize, h: usize, p: &CameraRaw) -> Self {
        let mid = p.vignette_midpoint.clamp(0.0, 100.0) / 100.0;
        let feather = p.vignette_feather.clamp(0.0, 100.0) / 100.0;
        let round = p.vignette_roundness.clamp(-100.0, 100.0) / 100.0;
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        // Roundness: 0 follows the frame's aspect, +1 a circle, −1 a rounded rectangle.
        let (ax, ay) = if round >= 0.0 {
            let r = cx.min(cy) + (cx.max(cy) - cx.min(cy)) * (1.0 - round);
            (if cx >= cy { r } else { cx }, if cy > cx { r } else { cy })
        } else {
            (cx, cy)
        };
        let inner = 1.0 - mid * 0.8;
        Vignette {
            amount: p.vignette_amount / 100.0,
            highlights: p.vignette_highlights.clamp(0.0, 100.0) / 100.0,
            style: match p.vignette_style.as_str() {
                "paintOverlay" => VignetteStyle::PaintOverlay,
                "colorPriority" => VignetteStyle::ColorPriority,
                _ => VignetteStyle::HighlightPriority,
            },
            centre: [cx, cy],
            axes: [ax, ay],
            power: 2.0 + (-round).max(0.0) * 6.0,
            edges: [inner - feather * 0.5 * inner, inner + feather * 0.5 * (1.4 - inner) + 0.02],
        }
    }
}

/// Grain: value noise sampled in full-resolution coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Grain {
    /// Noise cell size (full-resolution pixels).
    pub cell: f32,
    /// Weight of the finer octave.
    pub roughness: f32,
    pub amount: f32,
    pub seed: u64,
    /// Full-resolution pixels per pixel (`1 / pixel_scale`).
    pub scale: f32,
}

impl Grain {
    pub(crate) fn new(p: &CameraRaw) -> Self {
        Grain {
            cell: 1.0 + p.grain_size / 25.0,
            roughness: p.grain_roughness / 100.0,
            amount: p.grain_amount / 100.0 * 0.12,
            seed: u64::from(p.seed),
            scale: 1.0 / p.pixel_scale.clamp(0.01, 1.0),
        }
    }
}

/// Develop's per-pixel stages for one image and settings, in order. `None` stages are off.
#[derive(Clone, Debug, PartialEq)]
pub struct PerPixel {
    /// 1. White balance gains in linear light (sRGB-decoded values). Always `None` now: Temperature
    ///    and Tint are per-channel curves, developed on the CPU.
    pub gains: Option<[f32; 3]>,
    /// Exposure (after white balance): a curve over sRGB-encoded grey levels 0, 1/64, …, 1
    /// (input and output), hue-preserving in linear ProPhoto.
    pub exposure: Option<[f32; TONE_STEPS]>,
    /// Contrast (after Exposure): a curve like Exposure's.
    pub contrast: Option<[f32; TONE_STEPS]>,
    /// The document's linear RGB to linear ProPhoto (identity for ProPhoto documents).
    pub to_prophoto: [[f32; 3]; 3],
    /// Linear ProPhoto back to the document's linear RGB.
    pub from_prophoto: [[f32; 3]; 3],
    /// 3b. Whites and Blacks, in the document's encoding.
    pub whites_blacks: Option<WhitesBlacks>,
    /// The document's transfer curve: 0 for sRGB's, else a power-law gamma.
    pub encoding: f32,
    /// 4. Vibrance and Saturation: two measured tables one after the other (Saturation, then
    ///    Vibrance), on a [`PRESENCE_GRID`] grid, applied as [`PerPixel::hsl`]'s are.
    pub presence: Option<Vec<[f32; 3]>>,
    /// 5. The parametric tone curve, [`CURVE_LUT`] entries over 0 … 1.
    pub parametric: Option<Vec<f32>>,
    /// 5. Point curves: master, red, green, blue.
    pub point: [Option<Vec<f32>>; 4],
    /// 6. The Color Mixer's measured change: three tables one after the other (Luminance,
    ///    Saturation, Hue, in the order they apply), each per node of a [`MIXER_GRID`] grid over
    ///    hue (every 5°, wrapping round) × saturation × value in HSV of linear ProPhoto RGB (node
    ///    `(h · sats + s) · vals + v`): the hue shift in degrees, the saturation change and the
    ///    log-value change. Each is looked up trilinearly at the input's hue and the saturation
    ///    and value the previous table left; greys stay, and a colour within 0 … 1 stays within.
    pub hsl: Option<Vec<[f32; 3]>>,
    /// 7. Color Grading.
    pub grade: Option<Grade>,
    /// 9. Vignette.
    pub vignette: Option<Vignette>,
    /// 9. Grain.
    pub grain: Option<Grain>,
    /// Clip RGB to 0 … 1 at the end (all but 32-bit documents).
    pub clamp: bool,
}

impl CameraRaw {
    /// Whether developing needs a stage that reads neighbouring pixels: Dehaze, Shadows and
    /// Highlights, Texture, Clarity, sharpening or noise reduction.
    pub fn spatial(&self) -> bool {
        self.dehaze != 0.0
            || [self.highlights, self.shadows, self.texture, self.clarity].iter().any(|v| *v != 0.0)
            || self.sharpen_amount > 0.0
            || self.noise_luminance > 0.0
            || self.noise_color > 0.0
    }
}

/// The image's tones that Contrast and Whites up adapt to ([`TonePoints`]), for `px` (`w × h`,
/// display-encoded) developed per pixel with `p`: what [`develop`](super::develop) measures
/// after white balance and Exposure, and after Contrast too. `None` when neither Contrast nor a
/// raised Whites needs them, or `p` needs a spatial stage.
pub fn white_black_points(px: &[[f32; 4]], w: usize, h: usize, p: &CameraRaw) -> Option<TonePoints> {
    if (p.whites <= 0.0 && p.contrast == 0.0) || p.spatial() || px.len() != w.saturating_mul(h) {
        return None;
    }
    let enc = Encoding::of(p.encoding_gamma);
    let wb = super::white_balance::curves(p.temperature, p.tint);
    let m = exposure::primaries(enc);
    let ex = (p.exposure != 0.0).then(|| exposure::row(p.exposure));
    let before = |q: [f32; 4]| {
        let q = wb.as_ref().map_or(q, |c| super::white_balance::pixel(q, c, enc));
        ex.as_ref().map_or(q, |row| exposure::pixel(q, row, enc, m.as_ref()))
    };
    let tones_c = if p.contrast != 0.0 { super::tone_points(px, w, h, enc, before) } else { [0.0; 4] };
    let con = (p.contrast != 0.0).then(|| contrast::adaptive_row(p.contrast, Some(tones_c)));
    let tones_w = if p.whites > 0.0 {
        super::tone_points(px, w, h, enc, |q| {
            let q = before(q);
            con.as_ref().map_or(q, |row| exposure::pixel(q, row, enc, m.as_ref()))
        })
    } else {
        [0.0; 4]
    };
    Some([tones_c, tones_w])
}

/// Develop's stages for settings without spatial stages, on a `w × h` image; `points` are the
/// image's [`white_black_points`]. `None` when `p` needs a spatial stage or Calibration (these
/// stages don't run it), Temperature or Tint, or Contrast or a raised Whites has no `points`. The points themselves
/// leave Calibration out, so a caller may cache them across Calibration changes.
pub fn per_pixel(p: &CameraRaw, w: usize, h: usize, float: bool, points: Option<TonePoints>) -> Option<PerPixel> {
    if p.spatial() || !p.calibration_neutral() {
        return None;
    }
    let identity = p.is_identity();
    if (p.whites > 0.0 || p.contrast != 0.0) && points.is_none() {
        return None;
    }
    // Temperature and Tint are per-channel curves with measured colour corrections (see
    // `white_balance`), which these stages don't run.
    if p.temperature != 0.0 || p.tint != 0.0 {
        return None;
    }
    let (whites, blacks) = (whites_blacks::whites_row(p.whites, points.map(|t| t[1])), whites_blacks::blacks_row(p.blacks));
    let whites_fix = (p.whites < 0.0).then(|| super::colour_lut::correction(super::colour_lut::Slider::Whites, p.whites)).flatten().map(|c| c.grid().to_vec());
    let whites_blacks = (whites.is_some() || blacks.is_some()).then_some(WhitesBlacks { whites, blacks, whites_fix });
    const ID: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (to_prophoto, from_prophoto) = exposure::primaries(Encoding::of(p.encoding_gamma)).unwrap_or((ID, ID));
    let parametric_on = !(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0);
    let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, CURVE_LUT));
    Some(PerPixel {
        gains: None,
        exposure: (p.exposure != 0.0).then(|| exposure::row(p.exposure)),
        contrast: (p.contrast != 0.0).then(|| contrast::adaptive_row(p.contrast, points.map(|t| t[0]))),
        to_prophoto,
        from_prophoto,
        whites_blacks,
        encoding: Encoding::of(p.encoding_gamma).0,
        presence: super::presence::presence(p.vibrance, p.saturation).map(|t| t.nodes().to_vec()),
        parametric: parametric_on.then(|| parametric_lut(p, CURVE_LUT)),
        point: [pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue)],
        hsl: super::color_mixer::mixer(&p.hsl_hue, &p.hsl_sat, &p.hsl_lum).map(|m| m.nodes().to_vec()),
        grade: (!p.grade_neutral()).then(|| Grade::new(p)),
        vignette: (p.vignette_amount != 0.0).then(|| Vignette::new(w, h, p)),
        grain: (p.grain_amount > 0.0).then(|| Grain::new(p)),
        clamp: !float && !identity,
    })
}
