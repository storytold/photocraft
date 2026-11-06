//! Adjustment parameters (data only). Evaluation lives in `photocraft-compose`.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub input: f32,
    pub output: f32,
}

/// Whether `points` is the straight 0→0, 1→1 line (or too short to be a curve).
pub fn is_identity_curve(points: &[CurvePoint]) -> bool {
    match points {
        [a, b] => a.input == 0.0 && a.output == 0.0 && b.input == 1.0 && b.output == 1.0,
        p => p.len() < 2,
    }
}

/// A 3D LUT file format (Photoshop's `LUTFormat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LutFormat {
    #[serde(rename = "cube")]
    Cube,
    #[serde(rename = "3dl")]
    ThreeDl,
    #[serde(rename = "look")]
    Look,
}

/// A Color Lookup's LUT file as loaded: its format and bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LutSource {
    pub format: LutFormat,
    pub bytes: Arc<Vec<u8>>,
}

/// Entries in a pencil-drawn Curves map (one per 8-bit level, as in PSD `curv` maps).
pub const CURVE_MAP_LEN: usize = 256;

/// A pencil-mode curve from its map: `outputs[k]` (0..=1) is the output at input `k / 255`.
/// Missing entries repeat the last one; extra entries are ignored.
pub fn curve_map(outputs: &[f32]) -> Vec<CurvePoint> {
    let last = (CURVE_MAP_LEN - 1) as f32;
    let mut prev = 0.0;
    (0..CURVE_MAP_LEN)
        .map(|k| {
            let out = outputs.get(k).copied().filter(|v| v.is_finite()).map_or(prev, |v| v.clamp(0.0, 1.0));
            prev = out;
            CurvePoint { input: k as f32 / last, output: out }
        })
        .collect()
}

/// The [`CURVE_MAP_LEN`] outputs (0..=1) of `points` read as a pencil-mode curve: straight
/// segments between the points, constant outside the first/last (identity without two points).
pub fn curve_map_outputs(points: &[CurvePoint]) -> Vec<f32> {
    let mut pts: Vec<(f32, f32)> = points.iter().filter(|p| p.input.is_finite() && p.output.is_finite()).map(|p| (p.input, p.output)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let last = (CURVE_MAP_LEN - 1) as f32;
    (0..CURVE_MAP_LEN)
        .map(|k| {
            let x = k as f32 / last;
            let y = match (pts.first(), pts.last()) {
                (Some(a), Some(b)) if pts.len() >= 2 => {
                    if x <= a.0 {
                        a.1
                    } else if x >= b.0 {
                        b.1
                    } else {
                        let i = pts.partition_point(|p| p.0 < x);
                        match (pts.get(i.wrapping_sub(1)), pts.get(i)) {
                            (Some(p0), Some(p1)) if p1.0 > p0.0 => p0.1 + (p1.1 - p0.1) * (x - p0.0) / (p1.0 - p0.0),
                            (_, Some(p1)) => p1.1,
                            _ => b.1,
                        }
                    }
                }
                _ => x,
            };
            y.clamp(0.0, 1.0)
        })
        .collect()
}

/// Number of values in the table of a `size`³ Color Lookup (`size³ × 3`). `None` for a size
/// below 2 or one whose table length overflows: a stored document's size is untrusted.
pub fn lut3d_len(size: u32) -> Option<usize> {
    if size < 2 {
        return None;
    }
    usize::try_from(size).ok()?.checked_pow(3)?.checked_mul(3)
}

/// The default Color Lookup input domain's lower end (`.cube` `DOMAIN_MIN 0 0 0`).
pub const LUT_DOMAIN_MIN: [f32; 3] = [0.0; 3];
/// The default Color Lookup input domain's upper end (`.cube` `DOMAIN_MAX 1 1 1`).
pub const LUT_DOMAIN_MAX: [f32; 3] = [1.0; 3];

fn default_lut_domain_min() -> [f32; 3] {
    LUT_DOMAIN_MIN
}

fn default_lut_domain_max() -> [f32; 3] {
    LUT_DOMAIN_MAX
}

fn is_default_lut_domain_min(v: &[f32; 3]) -> bool {
    *v == LUT_DOMAIN_MIN
}

fn is_default_lut_domain_max(v: &[f32; 3]) -> bool {
    *v == LUT_DOMAIN_MAX
}

/// How every Color Lookup evaluator (CPU compositor, GPU kernel) maps an input colour onto the
/// table: per channel `(min, 1 / (max - min))`, so a channel value `c` samples the table at
/// `(c - min) · inv`, then clamps to 0..1. A stored domain is untrusted: a channel whose span
/// isn't a finite positive number (or whose reciprocal overflows) falls back to the plain 0..1
/// domain rather than dividing by zero. The default domain gives `([0; 3], [1; 3])` exactly, so
/// default-domain lookups sample exactly as before domains were carried through.
pub fn lut_domain_scale(min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = lut_domain_effective(min, max);
    (lo, std::array::from_fn(|k| 1.0 / (hi[k] - lo[k])))
}

/// The domain the evaluators actually apply (see [`lut_domain_scale`]): each channel as stored
/// when its span is a finite positive number with a finite reciprocal, otherwise plain 0..1.
/// What a save writes, so a file always re-reads as what was rendered.
pub fn lut_domain_effective(min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let mut lo = LUT_DOMAIN_MIN;
    let mut hi = LUT_DOMAIN_MAX;
    for k in 0..3 {
        let span = max[k] - min[k];
        if min[k].is_finite() && max[k].is_finite() && span.is_finite() && span > 0.0 && (1.0 / span).is_finite() {
            lo[k] = min[k];
            hi[k] = max[k];
        }
    }
    (lo, hi)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelsChannel {
    pub in_black: f32,
    pub in_white: f32,
    pub gamma: f32,
    pub out_black: f32,
    pub out_white: f32,
}

impl Default for LevelsChannel {
    fn default() -> Self {
        Self { in_black: 0.0, in_white: 1.0, gamma: 1.0, out_black: 0.0, out_white: 1.0 }
    }
}

/// Which document channels the per-channel records of Levels and Curves address. Compositing
/// happens in display RGB; `Cmyk` and `Lab` records are applied to the pixel converted into that
/// space (the same conversions the document's surfaces use), so ink and Lab channel edits behave
/// like Photoshop's in CMYK and Lab documents. Grayscale documents use `Rgb` with the master record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneSpace {
    /// `per_channel` = red, green, blue.
    #[default]
    Rgb,
    /// `per_channel` = cyan, magenta, yellow plus `black`; values are channel brightness
    /// (1 - ink), and the master record applies to all four inks.
    Cmyk,
    /// `per_channel` = lightness, a, b (stored 0..=1 like Lab surfaces); there is no composite
    /// record in Lab, so the master is ignored.
    Lab,
}

/// One of Hue/Saturation's six colour ranges (reds, yellows, greens, cyans, blues, magentas).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HueRange {
    /// Hue shift in degrees (-180..=180).
    pub hue: f32,
    /// Saturation change in percent (-100..=100).
    pub saturation: f32,
    /// Lightness change in percent (-100..=100).
    pub lightness: f32,
    /// Hue degrees where the range starts fading in, is fully on, starts fading out and ends
    /// (Photoshop's four range sliders). May wrap past 0/360.
    pub bounds: [f32; 4],
}

impl HueRange {
    /// The neutral range `i` (0 = reds … 5 = magentas) with Photoshop's default extent: fully on
    /// for 30° around its centre, fading over 30° each side.
    pub fn neutral(i: usize) -> Self {
        let c = 60.0 * i as f32;
        Self { hue: 0.0, saturation: 0.0, lightness: 0.0, bounds: [c - 45.0, c - 15.0, c + 15.0, c + 45.0] }
    }

    pub fn defaults() -> [HueRange; 6] {
        std::array::from_fn(Self::neutral)
    }

    /// The canonical form of four range sliders: increasing, within one turn, centre in
    /// -30..330 (so the default reds are -45, -15, 15, 45 whichever way they were written).
    pub fn canonical_bounds(b: [f32; 4]) -> [f32; 4] {
        if b.iter().any(|v| !v.is_finite()) {
            return [-45.0, -15.0, 15.0, 45.0];
        }
        let mut out = [b[0]; 4];
        for k in 1..4 {
            out[k] = (out[k - 1] + (b[k] - out[k - 1]).rem_euclid(360.0)).min(out[0] + 359.0);
        }
        let centre = (out[1] + out[2]) / 2.0;
        let shift = (centre + 30.0).rem_euclid(360.0) - 30.0 - centre;
        out.map(|v| v + shift)
    }

    pub fn is_neutral(&self) -> bool {
        self.hue == 0.0 && self.saturation == 0.0 && self.lightness == 0.0
    }

    /// How much a pixel of hue `deg` (degrees) belongs to this range, 0..=1.
    pub fn weight(&self, deg: f32) -> f32 {
        let [a, b, c, d] = self.bounds;
        // Measure from the range's start so wrap-around ranges (reds) work.
        let rel = |x: f32| if x.is_finite() { (x - a).rem_euclid(360.0) } else { 0.0 };
        let b = rel(b);
        let c = rel(c).max(b);
        let d = rel(d).max(c);
        let h = rel(deg);
        if h <= b {
            if b <= 0.0 { 1.0 } else { h / b }
        } else if h <= c {
            1.0
        } else if h <= d {
            if d <= c { 1.0 } else { (d - h) / (d - c) }
        } else {
            0.0
        }
    }
}

fn default_hue_ranges() -> [HueRange; 6] {
    HueRange::defaults()
}

/// Photoshop adjustment layers (Layer → New Adjustment Layer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Adjustment {
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
        legacy: bool,
    },
    /// R, G, B (or the channels `space` names) first, then the master (composite) record.
    /// In CMYK this also applies to `black`; in Lab the master is ignored.
    Levels {
        master: LevelsChannel,
        per_channel: [LevelsChannel; 3],
        #[serde(default)]
        space: ToneSpace,
        /// The black ink channel when `space` is [`ToneSpace::Cmyk`].
        #[serde(default)]
        black: LevelsChannel,
    },
    /// R, G, B curves (or the channels `space` names) first, then the master (composite) curve.
    /// In CMYK this also applies to `black`; in Lab the master is ignored.
    Curves {
        master: Vec<CurvePoint>,
        per_channel: [Vec<CurvePoint>; 3],
        #[serde(default)]
        space: ToneSpace,
        /// The black ink curve when `space` is [`ToneSpace::Cmyk`] (empty = identity).
        #[serde(default)]
        black: Vec<CurvePoint>,
        /// Drawn with the pencil (Photoshop's "Draw to modify the curve", PSD `curv` map form):
        /// every curve is a [`CURVE_MAP_LEN`]-entry map (see [`curve_map`]) evaluated
        /// piecewise-linearly instead of a spline through its points.
        #[serde(default)]
        pencil: bool,
    },
    Exposure {
        exposure: f32,
        offset: f32,
        gamma: f32,
    },
    Vibrance {
        vibrance: f32,
        saturation: f32,
    },
    /// Master hue / saturation / lightness (or the Colorize colour) plus six hue ranges.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
        colorize: bool,
        #[serde(default = "default_hue_ranges")]
        ranges: [HueRange; 6],
    },
    ColorBalance {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        preserve_luminosity: bool,
    },
    BlackWhite {
        weights: [f32; 6],
        tint: Option<[f32; 3]>,
    },
    PhotoFilter {
        color: [f32; 3],
        density: f32,
        preserve_luminosity: bool,
    },
    ChannelMixer {
        matrix: [[f32; 4]; 3],
        monochrome: bool,
    },
    /// A 3D LUT: `size`³ RGB triplets, red varying fastest (`((b·size + g)·size + r)·3`), in 0..=1.
    /// `lut` is None for a lookup Photoshop stores as an ICC profile (identity here).
    ColorLookup {
        name: String,
        lut: Option<Arc<Vec<f32>>>,
        size: u32,
        /// Tetrahedral instead of trilinear interpolation.
        #[serde(default)]
        tetrahedral: bool,
        /// Ordered dither of ±½ an 8-bit step to hide banding (Photoshop's "Dither").
        #[serde(default)]
        dither: bool,
        /// The LUT file `lut` was read from, embedded unchanged on PSD save as Photoshop does.
        /// None for a table made in PhotoCraft (a built-in look); a save also ignores it when
        /// it no longer parses to `lut`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<LutSource>,
        /// The table's input domain (`.cube` `DOMAIN_MIN`/`DOMAIN_MAX`, `LUT_3D_INPUT_RANGE`):
        /// a channel value `c` samples the table at `(c - min) / (max - min)`, clamped to 0..1
        /// (see [`lut_domain_scale`]). `[0; 3]`..`[1; 3]` (the default, and what documents
        /// written before domains were carried through load as) is plain 0..1 sampling.
        #[serde(default = "default_lut_domain_min", skip_serializing_if = "is_default_lut_domain_min")]
        domain_min: [f32; 3],
        #[serde(default = "default_lut_domain_max", skip_serializing_if = "is_default_lut_domain_max")]
        domain_max: [f32; 3],
    },
    Invert,
    Posterize {
        levels: u32,
    },
    Threshold {
        level: f32,
    },
    GradientMap {
        stops: Vec<(f32, [f32; 3])>,
        reverse: bool,
        /// Ordered dither of ±½ an 8-bit step to hide banding.
        #[serde(default)]
        dither: bool,
    },
    /// Per range (reds, yellows, greens, cyans, blues, magentas, whites, neutrals, blacks) the
    /// cyan, magenta, yellow and black change in percent (-100..=100), as in Photoshop.
    SelectiveColor {
        relative: bool,
        adjustments: [[f32; 4]; 9],
    },
    /// A PSD adjustment we can't evaluate yet; preserved raw for round-trip, shared between
    /// document snapshots (#4535).
    Unsupported {
        psd_key: String,
        raw: std::sync::Arc<[u8]>,
    },
}

impl Adjustment {
    pub fn label(&self) -> &str {
        match self {
            Adjustment::BrightnessContrast { .. } => "Brightness/Contrast",
            Adjustment::Levels { .. } => "Levels",
            Adjustment::Curves { .. } => "Curves",
            Adjustment::Exposure { .. } => "Exposure",
            Adjustment::Vibrance { .. } => "Vibrance",
            Adjustment::HueSaturation { .. } => "Hue/Saturation",
            Adjustment::ColorBalance { .. } => "Color Balance",
            Adjustment::BlackWhite { .. } => "Black & White",
            Adjustment::PhotoFilter { .. } => "Photo Filter",
            Adjustment::ChannelMixer { .. } => "Channel Mixer",
            Adjustment::ColorLookup { .. } => "Color Lookup",
            Adjustment::Invert => "Invert",
            Adjustment::Posterize { .. } => "Posterize",
            Adjustment::Threshold { .. } => "Threshold",
            Adjustment::GradientMap { .. } => "Gradient Map",
            Adjustment::SelectiveColor { .. } => "Selective Color",
            Adjustment::Unsupported { .. } => "Adjustment",
        }
    }

    pub fn default_hue_saturation() -> Self {
        Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges: HueRange::defaults() }
    }

    pub fn identity_curves() -> Self {
        let line = || vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }];
        Adjustment::Curves { master: line(), per_channel: [line(), line(), line()], space: ToneSpace::Rgb, black: Vec::new(), pencil: false }
    }

    pub fn identity_levels() -> Self {
        Adjustment::Levels { master: LevelsChannel::default(), per_channel: Default::default(), space: ToneSpace::Rgb, black: LevelsChannel::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_range_weights() {
        let reds = HueRange::neutral(0);
        assert_eq!(reds.weight(0.0), 1.0);
        assert_eq!(reds.weight(350.0), 1.0);
        assert!((reds.weight(30.0) - 0.5).abs() < 1e-5);
        assert!((reds.weight(330.0) - 0.5).abs() < 1e-5);
        assert_eq!(reds.weight(60.0), 0.0);
        assert_eq!(reds.weight(180.0), 0.0);
        let blues = HueRange::neutral(4);
        assert_eq!(blues.weight(240.0), 1.0);
        assert_eq!(blues.weight(120.0), 0.0);
        // Degenerate bounds never divide by zero.
        let odd = HueRange { bounds: [10.0, 10.0, 10.0, 10.0], ..HueRange::neutral(0) };
        assert!(odd.weight(10.0).is_finite() && odd.weight(200.0).is_finite());
        let nan = HueRange { bounds: [f32::NAN; 4], ..HueRange::neutral(0) };
        assert!(nan.weight(10.0).is_finite());
    }

    #[test]
    fn old_files_get_neutral_new_fields() {
        let a: Adjustment = serde_json::from_str(r#"{"HueSaturation":{"hue":1.0,"saturation":2.0,"lightness":3.0,"colorize":false}}"#).unwrap();
        assert!(matches!(a, Adjustment::HueSaturation { ranges, .. } if ranges == HueRange::defaults()));
        let c: Adjustment = serde_json::from_str(r#"{"Curves":{"master":[],"per_channel":[[],[],[]]}}"#).unwrap();
        assert!(matches!(c, Adjustment::Curves { space: ToneSpace::Rgb, pencil: false, .. }));
        let g: Adjustment = serde_json::from_str(r#"{"GradientMap":{"stops":[],"reverse":true}}"#).unwrap();
        assert!(matches!(g, Adjustment::GradientMap { dither: false, .. }));
        let l: Adjustment = serde_json::from_str(r#"{"ColorLookup":{"name":"x","lut":null,"size":0}}"#).unwrap();
        assert!(matches!(l, Adjustment::ColorLookup { domain_min, domain_max, .. } if domain_min == LUT_DOMAIN_MIN && domain_max == LUT_DOMAIN_MAX));
    }

    #[test]
    fn color_lookup_domain_round_trips_and_default_is_not_written() {
        let mk = |domain_min, domain_max| Adjustment::ColorLookup {
            name: "x".into(),
            lut: None,
            size: 0,
            tetrahedral: false,
            dither: false,
            source: None,
            domain_min,
            domain_max,
        };
        let custom = mk([0.25, 0.1, 0.0], [0.75, 0.9, 2.0]);
        let back: Adjustment = serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        assert_eq!(back, custom);
        let json = serde_json::to_string(&mk(LUT_DOMAIN_MIN, LUT_DOMAIN_MAX)).unwrap();
        assert!(!json.contains("domain"), "{json}");
    }

    #[test]
    fn lut_domain_scale_normalises_and_guards_degenerate_domains() {
        assert_eq!(lut_domain_scale(LUT_DOMAIN_MIN, LUT_DOMAIN_MAX), ([0.0; 3], [1.0; 3]));
        assert_eq!(lut_domain_scale([0.25; 3], [0.75; 3]), ([0.25; 3], [2.0; 3]));
        // Empty, NaN and infinite spans fall back per channel to 0..1.
        assert_eq!(lut_domain_scale([0.5, f32::NAN, 0.0], [0.5, 1.0, f32::INFINITY]), ([0.0; 3], [1.0; 3]));
        // Inverted spans and spans whose reciprocal overflows do too.
        assert_eq!(lut_domain_scale([0.9, 0.0, 0.0], [0.1, f32::MIN_POSITIVE / 16.0, 1.0]), ([0.0; 3], [1.0; 3]));
        // Channels are independent.
        assert_eq!(lut_domain_scale([-1.0, 0.0, 0.0], [1.0, 1.0, 1.0]), ([-1.0, 0.0, 0.0], [0.5, 1.0, 1.0]));
        // What a save writes is the domain the evaluators apply.
        assert_eq!(lut_domain_effective([0.25, f32::NAN, 0.9], [0.75, 1.0, 0.1]), ([0.25, 0.0, 0.0], [0.75, 1.0, 1.0]));
    }

    #[test]
    fn lut3d_len_rejects_tiny_and_overflowing_sizes() {
        assert_eq!(lut3d_len(0), None);
        assert_eq!(lut3d_len(1), None);
        assert_eq!(lut3d_len(2), Some(24));
        assert_eq!(lut3d_len(33), Some(33 * 33 * 33 * 3));
        assert_eq!(lut3d_len(1 << 22), None, "(2²²)³ overflows a 64-bit length");
        assert_eq!(lut3d_len(u32::MAX), None);
    }
}
