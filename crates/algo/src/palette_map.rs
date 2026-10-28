//! Palette-aware colour mapping and dithering for floating-point encoded-sRGB RGBA.
//!
//! The kernel is deliberately independent of surfaces, profiles, and UI state.  Callers provide a
//! complete rectangle and a document-space origin; every pattern is therefore stable when a view
//! is panned or when a region is rendered in more than one pass.  Palette entries are never
//! rewritten or deduplicated, and the returned plane uses `u32` indices so palettes larger than
//! 256 entries remain lossless.

use std::sync::atomic::{AtomicBool, Ordering};

use photocraft_color::convert::srgb_to_lab;
use serde::{Deserialize, Serialize};

// Bound extended sRGB before RGB/Lab distance arithmetic; this remains far beyond SDR ICC gamuts.
const MAX_MAPPING_RGB: f32 = 1_000_000.0;

/// An ordered palette in encoded sRGB.  Entries may be arbitrary floating-point colours; source
/// order and duplicates are significant for deterministic ties and are preserved exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub name: String,
    pub colors: Vec<[f32; 3]>,
}

impl Palette {
    /// Validates a normalized source palette (catalog/import/command channels in 0..=1).
    /// Rendering also accepts bounded extended-sRGB colours produced by ICC transforms.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_range(0.0..=1.0)
    }

    fn validate_range(&self, range: std::ops::RangeInclusive<f32>) -> Result<(), String> {
        if self.colors.is_empty() {
            return Err("palette must contain at least one colour".to_owned());
        }
        if self.colors.len() > 4096 {
            return Err(format!("palette contains {} colours; the maximum is 4096", self.colors.len()));
        }
        for (i, color) in self.colors.iter().enumerate() {
            for (channel, &value) in color.iter().enumerate() {
                if !value.is_finite() || !range.contains(&value) {
                    return Err(format!("palette colour {i} channel {channel} must be finite and in {}..={}", range.start(), range.end()));
                }
            }
        }
        Ok(())
    }
}

/// Colour distance used by nearest-palette lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Distance {
    #[default]
    Rgb,
    Lab,
}

/// Treatment of source alpha.  Binary and Dither produce 0/1 alpha; when a transparent palette
/// index is supplied, pixels assigned alpha zero use that entry and it is excluded from opaque
/// nearest-colour lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    #[default]
    Preserve,
    Binary,
    Dither,
}

/// Palette mapping and dithering family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    #[default]
    None,
    Bayer,
    ClusteredDot,
    FloydSteinberg,
    SierraLite,
    Atkinson,
    Jarvis,
    Stucki,
    Sierra,
    Burkes,
    BlueNoise,
    WhiteNoise,
    Riemersma,
}

impl Method {
    /// Parses canonical IDs and the legacy command spellings retained by Indexed Color commands.
    pub fn from_id(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "bayer" | "pattern" => Self::Bayer,
            "clusteredDot" => Self::ClusteredDot,
            "floydSteinberg" | "diffusion" => Self::FloydSteinberg,
            "sierraLite" => Self::SierraLite,
            "atkinson" => Self::Atkinson,
            "jarvis" => Self::Jarvis,
            "stucki" => Self::Stucki,
            "sierra" => Self::Sierra,
            "burkes" => Self::Burkes,
            "blueNoise" => Self::BlueNoise,
            "whiteNoise" | "noise" => Self::WhiteNoise,
            "riemersma" => Self::Riemersma,
            _ => return None,
        })
    }

    /// The stable canonical command/UI ID.  Legacy aliases are accepted by [`from_id`] but never
    /// emitted by this method.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer => "bayer",
            Self::ClusteredDot => "clusteredDot",
            Self::FloydSteinberg => "floydSteinberg",
            Self::SierraLite => "sierraLite",
            Self::Atkinson => "atkinson",
            Self::Jarvis => "jarvis",
            Self::Stucki => "stucki",
            Self::Sierra => "sierra",
            Self::Burkes => "burkes",
            Self::BlueNoise => "blueNoise",
            Self::WhiteNoise => "whiteNoise",
            Self::Riemersma => "riemersma",
        }
    }
}

/// A threshold ingredient.  Values from built-in matrices are generated at render time; custom
/// matrices are ordinary unipolar 0..1 thresholds and are validated before rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pattern {
    pub kind: PatternKind,
    pub amount: f32,
    pub scale: u32,
    pub matrix_size: u32,
    pub phase: [i32; 2],
    pub seed: u32,
    pub enabled: bool,
    pub matrix: Vec<f32>,
}

impl Default for Pattern {
    fn default() -> Self {
        Self { kind: PatternKind::Bayer, amount: 1.0, scale: 1, matrix_size: 4, phase: [0, 0], seed: 0, enabled: true, matrix: Vec::new() }
    }
}

/// Pattern field kinds available to composable threshold ingredients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternKind {
    Bayer,
    ClusteredDot,
    BlueNoise,
    WhiteNoise,
    Custom,
}

/// Complete deterministic mapping settings.  Defaults are a nearest-colour map (`None`), RGB
/// distance, preserved alpha, unit spatial scale, serpentine scan enabled, 16 Riemersma/history
/// slots, and no optional threshold ingredients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub method: Method,
    pub amount: f32,
    pub matrix_size: u32,
    pub scale: u32,
    pub seed: u32,
    pub serpentine: bool,
    pub history: u32,
    pub decay: f32,
    pub distance: Distance,
    pub alpha: AlphaMode,
    pub patterns: Vec<Pattern>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            method: Method::None,
            amount: 1.0,
            matrix_size: 4,
            scale: 1,
            seed: 0,
            serpentine: true,
            history: 16,
            decay: 0.5,
            distance: Distance::Rgb,
            alpha: AlphaMode::Preserve,
            patterns: Vec::new(),
        }
    }
}

impl Settings {
    /// Validates all bounded settings before any threshold assets or pixel buffers are built.
    pub fn validate(&self) -> Result<(), String> {
        validate_unit(self.amount, "amount")?;
        validate_matrix_size(self.matrix_size, "matrixSize")?;
        if !(1..=64).contains(&self.scale) {
            return Err("scale must be in 1..=64".to_owned());
        }
        if !(1..=64).contains(&self.history) {
            return Err("history must be in 1..=64".to_owned());
        }
        validate_unit(self.decay, "decay")?;
        if self.patterns.len() > 16 {
            return Err("at most 16 threshold patterns are allowed".to_owned());
        }
        for (i, pattern) in self.patterns.iter().enumerate() {
            validate_unit(pattern.amount, &format!("patterns[{i}].amount"))?;
            if !(1..=64).contains(&pattern.scale) {
                return Err(format!("patterns[{i}].scale must be in 1..=64"));
            }
            validate_matrix_size(pattern.matrix_size, &format!("patterns[{i}].matrixSize"))?;
            if pattern.matrix.len() > 4096 {
                return Err(format!("patterns[{i}].matrix has more than 4096 cells"));
            }
            if matches!(pattern.kind, PatternKind::Custom) && pattern.enabled && pattern.amount > 0.0 {
                let cells = usize::try_from(pattern.matrix_size).ok().and_then(|n| n.checked_mul(n));
                if cells != Some(pattern.matrix.len()) {
                    return Err(format!("patterns[{i}].custom matrix must contain matrixSize² cells"));
                }
            }
            for (j, &value) in pattern.matrix.iter().enumerate() {
                if !value.is_finite() {
                    return Err(format!("patterns[{i}].matrix[{j}] must be finite"));
                }
                if matches!(pattern.kind, PatternKind::Custom) && !(0.0..=1.0).contains(&value) {
                    return Err(format!("patterns[{i}].custom matrix[{j}] must be in 0..=1"));
                }
            }
        }
        Ok(())
    }
}

fn validate_unit(value: f32, name: &str) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) { Err(format!("{name} must be finite and in 0..=1")) } else { Ok(()) }
}

fn validate_matrix_size(value: u32, name: &str) -> Result<(), String> {
    if matches!(value, 2 | 4 | 8 | 16) { Ok(()) } else { Err(format!("{name} must be one of 2, 4, 8, or 16")) }
}

/// Renders a complete row-major rectangle, returning one wide index per input pixel and replacing
/// each mapped RGB triplet with the exact selected palette entry.
pub fn render(
    px: &mut [[f32; 4]],
    width: usize,
    palette: &Palette,
    settings: &Settings,
    origin: [i32; 2],
    transparent: Option<usize>,
) -> Result<Vec<u32>, String> {
    let never_cancelled = || false;
    render_with(px, width, palette, settings, origin, transparent, &never_cancelled)
}

/// Maps pixels in place without allocating a returned index plane.
pub fn render_pixels(
    px: &mut [[f32; 4]],
    width: usize,
    palette: &Palette,
    settings: &Settings,
    origin: [i32; 2],
    transparent: Option<usize>,
) -> Result<(), String> {
    let never_cancelled = || false;
    render_pixels_with(px, width, palette, settings, origin, transparent, &never_cancelled)
}

/// [`render_pixels`] with cooperative cancellation.  The input may be partially written when
/// cancellation is reported; callers should discard that operation as a whole.
pub fn render_pixels_with(
    px: &mut [[f32; 4]],
    width: usize,
    palette: &Palette,
    settings: &Settings,
    origin: [i32; 2],
    transparent: Option<usize>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    render_internal(RenderTarget { pixels: px, indices: None }, width, palette, settings, origin, transparent, cancelled)
}

/// [`render`] with a cooperative cancellation callback.  Cancellation returns an error and may
/// leave a partially written buffer; callers should discard that buffer as one operation.
pub fn render_with(
    px: &mut [[f32; 4]],
    width: usize,
    palette: &Palette,
    settings: &Settings,
    origin: [i32; 2],
    transparent: Option<usize>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<u32>, String> {
    let mut indices = Vec::new();
    indices.try_reserve_exact(px.len()).map_err(|_| "could not allocate palette index plane")?;
    indices.resize(px.len(), 0u32);
    render_internal(RenderTarget { pixels: px, indices: Some(indices.as_mut_slice()) }, width, palette, settings, origin, transparent, cancelled)?;
    Ok(indices)
}

struct RenderTarget<'a> {
    pixels: &'a mut [[f32; 4]],
    indices: Option<&'a mut [u32]>,
}

/// Immutable state shared by pixel mapping, row workers, and sequential traversals.
struct MapContext<'a> {
    palette: &'a Palette,
    settings: &'a Settings,
    lookup: Lookup,
    plan: ThresholdPlan,
    transparent: Option<usize>,
    width: usize,
    height: usize,
    origin: [i32; 2],
}

fn render_internal(
    target: RenderTarget<'_>,
    width: usize,
    palette: &Palette,
    settings: &Settings,
    origin: [i32; 2],
    transparent: Option<usize>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    let RenderTarget { pixels: px, indices } = target;
    palette.validate_range(-MAX_MAPPING_RGB..=MAX_MAPPING_RGB)?;
    settings.validate()?;
    if width == 0 {
        return if px.is_empty() { Ok(()) } else { Err("width must be non-zero for a non-empty pixel buffer".to_owned()) };
    }
    if !px.len().is_multiple_of(width) {
        return Err(format!("pixel buffer length {} is not divisible by width {width}", px.len()));
    }
    if let Some(index) = transparent
        && index >= palette.colors.len()
    {
        return Err(format!("transparent palette index {index} is out of range"));
    }
    if let Some(indexes) = indices.as_ref()
        && indexes.len() != px.len()
    {
        return Err("internal index plane length does not match pixels".to_owned());
    }
    if cancelled() {
        return Err("palette mapping cancelled".to_owned());
    }
    if px.is_empty() {
        return Ok(());
    }
    let h = px.len() / width;
    let _height_i64 = i64::try_from(h).map_err(|_| "height exceeds supported coordinate range".to_owned())?;
    let _width_i64 = i64::try_from(width).map_err(|_| "width exceeds supported coordinate range".to_owned())?;
    let lookup = Lookup::new(palette, settings.distance, transparent)?;
    let plan = ThresholdPlan::new(settings, cancelled)?;
    let context = MapContext { palette, settings, lookup, plan, transparent, width, height: h, origin };
    let stateful = matches!(
        settings.method,
        Method::FloydSteinberg | Method::SierraLite | Method::Atkinson | Method::Jarvis | Method::Stucki | Method::Sierra | Method::Burkes | Method::Riemersma
    );
    if settings.method == Method::Riemersma {
        render_riemersma(px, &context, indices, cancelled)
    } else if stateful {
        render_diffusion(px, &context, indices, cancelled)
    } else {
        render_local(px, &context, indices, cancelled)
    }
}

struct ThresholdPlan {
    method_blue: Option<BlueNoise>,
    patterns: Vec<PatternField>,
    has_ingredients: bool,
    calibration: Option<Calibration>,
}

struct PatternField {
    kind: PatternKind,
    amount: f32,
    scale: u32,
    matrix_size: u32,
    phase: [i32; 2],
    seed: u32,
    matrix: Vec<f32>,
    blue: Option<BlueNoise>,
}

impl ThresholdPlan {
    fn new(settings: &Settings, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Self, String> {
        let method_active = settings.amount > 0.0 && matches!(settings.method, Method::Bayer | Method::ClusteredDot | Method::BlueNoise | Method::WhiteNoise);
        let method_blue =
            if settings.method == Method::BlueNoise && settings.amount > 0.0 { Some(BlueNoise::generate(settings.seed, cancelled)?) } else { None };
        let mut patterns = Vec::with_capacity(settings.patterns.len());
        for pattern in settings.patterns.iter().filter(|p| p.enabled && p.amount > 0.0) {
            let blue = if pattern.kind == PatternKind::BlueNoise { Some(BlueNoise::generate(pattern.seed, cancelled)?) } else { None };
            patterns.push(PatternField {
                kind: pattern.kind,
                amount: pattern.amount,
                scale: pattern.scale,
                matrix_size: pattern.matrix_size,
                phase: pattern.phase,
                seed: pattern.seed,
                matrix: pattern.matrix.clone(),
                blue,
            });
        }
        let mut plan = Self { method_blue, has_ingredients: method_active || !patterns.is_empty(), calibration: None, patterns };
        // A single rank field is already calibrated by construction for all built-ins.  A mixture
        // is not: averaging two ranks changes its CDF and can move a nominal 50% tone.  Calibrate
        // mixtures against a fixed document-independent sample set once per render plan.
        let active_count = (if method_active { 1 } else { 0 }) + plan.patterns.len();
        if active_count >= 2 {
            plan.calibration = Some(Calibration::build(&plan, settings, cancelled)?);
        }
        Ok(plan)
    }

    fn raw_value(&self, settings: &Settings, x: i64, y: i64) -> (f32, f32) {
        let mut weighted = 0.0f32;
        let mut total = 0.0f32;
        if settings.amount > 0.0 {
            let raw = match settings.method {
                Method::Bayer => bayer_value(x, y, settings.matrix_size, settings.scale, [0, 0]),
                Method::ClusteredDot => clustered_value(x, y, settings.matrix_size, settings.scale, [0, 0]),
                Method::BlueNoise => self.method_blue.as_ref().map_or(0.0, |b| b.value(x, y, settings.scale, [0, 0])),
                Method::WhiteNoise => white_value(x, y, settings.scale, [0, 0], settings.seed),
                _ => 0.0,
            };
            if matches!(settings.method, Method::Bayer | Method::ClusteredDot | Method::BlueNoise | Method::WhiteNoise) {
                weighted += raw * settings.amount;
                total += settings.amount;
            }
        }
        for pattern in &self.patterns {
            weighted += pattern.value(x, y) * pattern.amount;
            total += pattern.amount;
        }
        (weighted, total)
    }

    /// Returns a centred, CDF-calibrated threshold.  Strength is applied after calibration so
    /// adding ingredients cannot silently bias the result or make their average exceed ±0.5.
    #[inline]
    fn value(&self, settings: &Settings, x: i64, y: i64) -> f32 {
        let (weighted, total) = self.raw_value(settings, x, y);
        if total <= 0.0 {
            return 0.0;
        }
        let normalized = weighted / total;
        let rank = self.calibration.as_ref().map_or(normalized, |calibration| calibration.rank(normalized));
        rank * total.min(1.0)
    }
}

struct Calibration {
    samples: Vec<f32>,
}

impl Calibration {
    const SAMPLE_COUNT: usize = 8192;

    fn build(plan: &ThresholdPlan, settings: &Settings, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Self, String> {
        let mut samples = Vec::with_capacity(Self::SAMPLE_COUNT);
        for i in 0..Self::SAMPLE_COUNT {
            if i.is_multiple_of(256) && cancelled() {
                return Err("palette mapping cancelled".to_owned());
            }
            // Fixed pseudo-random document coordinates make calibration independent of viewport
            // origin, image extent, and tile partitioning while exercising long pattern periods.
            let x = (hash64(i as i64, 0, 0x3A51_9E21) & 0x7fff) as i64 - 16_384;
            let y = (hash64(i as i64, 1, 0xC6EF_3720) & 0x7fff) as i64 - 16_384;
            let (weighted, total) = plan.raw_value(settings, x, y);
            if total > 0.0 {
                samples.push(weighted / total);
            }
        }
        samples.sort_by(f32::total_cmp);
        Ok(Self { samples })
    }

    #[inline]
    fn rank(&self, value: f32) -> f32 {
        if self.samples.is_empty() {
            return value;
        }
        let value = value.clamp(-0.5, 0.5);
        let mut lower = 0usize;
        let mut upper = self.samples.len();
        while lower < upper {
            let middle = (lower + upper) / 2;
            if self.samples[middle] < value {
                lower = middle + 1;
            } else {
                upper = middle;
            }
        }
        let first_ge = lower;
        upper = self.samples.len();
        while lower < upper {
            let middle = (lower + upper) / 2;
            if self.samples[middle] <= value {
                lower = middle + 1;
            } else {
                upper = middle;
            }
        }
        let first_gt = lower;
        ((first_ge + first_gt) as f32 * 0.5 + 0.5) / self.samples.len() as f32 - 0.5
    }
}

impl PatternField {
    #[inline]
    fn value(&self, x: i64, y: i64) -> f32 {
        match self.kind {
            PatternKind::Bayer => bayer_value(x, y, self.matrix_size, self.scale, self.phase),
            PatternKind::ClusteredDot => clustered_value(x, y, self.matrix_size, self.scale, self.phase),
            PatternKind::BlueNoise => self.blue.as_ref().map_or(0.0, |b| b.value(x, y, self.scale, self.phase)),
            PatternKind::WhiteNoise => white_value(x, y, self.scale, self.phase, self.seed),
            PatternKind::Custom => custom_value(&self.matrix, self.matrix_size, x, y, self.scale, self.phase),
        }
    }
}

fn scaled_position(x: i64, y: i64, scale: u32, phase: [i32; 2]) -> (i64, i64) {
    let s = i64::from(scale.max(1));
    ((x + i64::from(phase[0])).div_euclid(s), (y + i64::from(phase[1])).div_euclid(s))
}

#[inline]
fn bayer_value(x: i64, y: i64, size: u32, scale: u32, phase: [i32; 2]) -> f32 {
    let (x, y) = scaled_position(x, y, scale, phase);
    let n = i64::from(size);
    let xx = x.rem_euclid(n) as u32;
    let yy = y.rem_euclid(n) as u32;
    let rank = bayer_rank(xx, yy, size);
    (rank as f32 + 0.5) / (size * size) as f32 - 0.5
}

#[inline]
fn bayer_rank(x: u32, y: u32, size: u32) -> u32 {
    let mut rank = 0u32;
    let mut bit = 1u32;
    while bit < size {
        let q = ((x & bit != 0) as usize) | (((y & bit != 0) as usize) << 1);
        let mapped = [0u32, 2, 3, 1][q];
        rank = rank * 4 + mapped;
        bit *= 2;
    }
    rank
}

#[inline]
fn clustered_value(x: i64, y: i64, size: u32, scale: u32, phase: [i32; 2]) -> f32 {
    let (x, y) = scaled_position(x, y, scale, phase);
    let n = i64::from(size);
    let xx = x.rem_euclid(n);
    let yy = y.rem_euclid(n);
    let rx = (2 * xx + 1 - n).abs();
    let ry = (2 * yy + 1 - n).abs();
    let ring = rx.max(ry);
    let (base, pos) = if ring == 0 {
        (0i64, 0i64)
    } else {
        let left = (n - 1 - ring) / 2;
        let right = left + ring;
        let top = left;
        let bottom = right;
        let position = if yy == top {
            xx - left
        } else if xx == right {
            (ring + 1) + (yy - top - 1)
        } else if yy == bottom {
            (ring + 1) + ring + (right - 1 - xx)
        } else {
            (ring + 1) + ring + ring + (bottom - 1 - yy)
        };
        ((ring - 1) * (ring - 1), position)
    };
    let rank = base + pos;
    (rank as f32 + 0.5) / (size * size) as f32 - 0.5
}

#[inline]
fn custom_value(matrix: &[f32], size: u32, x: i64, y: i64, scale: u32, phase: [i32; 2]) -> f32 {
    let (x, y) = scaled_position(x, y, scale, phase);
    let n = i64::from(size);
    let xx = x.rem_euclid(n) as usize;
    let yy = y.rem_euclid(n) as usize;
    let index = yy * size as usize + xx;
    matrix[index] - 0.5
}

#[inline]
fn white_value(x: i64, y: i64, scale: u32, phase: [i32; 2], seed: u32) -> f32 {
    let (x, y) = scaled_position(x, y, scale, phase);
    hash01(x, y, seed) - 0.5
}

#[inline]
fn hash64(x: i64, y: i64, seed: u32) -> u64 {
    let mut z =
        (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ u64::from(seed).wrapping_mul(0x94D0_49BB_1331_11EB);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn hash01(x: i64, y: i64, seed: u32) -> f32 {
    ((hash64(x, y, seed) >> 40) as f32) / 16_777_215.0
}

#[path = "blue_noise_data.rs"]
mod blue_noise_data;

/// A deterministic 64×64 void-and-cluster rank tile. Generated offline from a relaxed random
/// prototype, not a recursive lattice; see the `generate_blue_noise` example and Ulichney's papers.
/// Normalization happens at compile time. Seeds select toroidal shifts and rotations/reflections.
struct BlueNoise {
    ranks: &'static [f32; 4096],
    seed: u32,
}

impl BlueNoise {
    const SIDE: usize = 64;
    const CELLS: usize = Self::SIDE * Self::SIDE;

    const RANKS: [f32; Self::CELLS] = {
        let mut ranks = [0.0; Self::CELLS];
        let mut index = 0;
        while index < Self::CELLS {
            ranks[index] = (blue_noise_data::RANKS[index] as f32 + 0.5) / Self::CELLS as f32 - 0.5;
            index += 1;
        }
        ranks
    };

    fn generate(seed: u32, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Self, String> {
        if cancelled() {
            return Err("palette mapping cancelled".to_owned());
        }
        Ok(Self { ranks: &Self::RANKS, seed })
    }

    #[inline]
    fn value(&self, x: i64, y: i64, scale: u32, phase: [i32; 2]) -> f32 {
        let (x, y) = scaled_position(x, y, scale, phase);
        let sx = i64::from(self.seed & 63);
        let sy = i64::from((self.seed >> 6) & 63);
        let mode = (self.seed >> 12) & 7;
        let (x, y) = match mode {
            0 => (x, y),
            1 => (-y, x),
            2 => (-x, -y),
            3 => (y, -x),
            4 => (-x, y),
            5 => (x, -y),
            6 => (y, x),
            _ => (-y, -x),
        };
        let xx = (x + sx).rem_euclid(Self::SIDE as i64) as usize;
        let yy = (y + sy).rem_euclid(Self::SIDE as i64) as usize;
        self.ranks[yy * Self::SIDE + xx]
    }
}

#[derive(Clone, Copy)]
struct Entry {
    point: [f32; 3],
    index: usize,
    axis: usize,
}

#[derive(Clone, Copy)]
struct Candidate {
    distance: f32,
    index: usize,
    point: [f32; 3],
}

fn empty_candidate() -> Candidate {
    Candidate { distance: f32::INFINITY, index: usize::MAX, point: [0.0; 3] }
}

struct Lookup {
    entries: Vec<Entry>,
    tree: bool,
    distance: Distance,
}

impl Lookup {
    fn new(palette: &Palette, distance: Distance, transparent: Option<usize>) -> Result<Self, String> {
        let excluded = if transparent.is_some() { 1 } else { 0 };
        let mut entries = Vec::with_capacity(palette.colors.len().saturating_sub(excluded));
        for (index, &color) in palette.colors.iter().enumerate() {
            if Some(index) == transparent {
                continue;
            }
            let point = match distance {
                Distance::Rgb => color,
                Distance::Lab => srgb_to_lab(color),
            };
            entries.push(Entry { point, index, axis: 0 });
        }
        if entries.is_empty() {
            return Err("palette has no opaque entries after excluding transparent index".to_owned());
        }
        let tree = entries.len() > 64;
        if tree {
            partition(&mut entries);
        }
        Ok(Self { entries, tree, distance })
    }

    #[inline]
    fn query_point(&self, color: [f32; 3]) -> [f32; 3] {
        match self.distance {
            Distance::Rgb => color,
            Distance::Lab => srgb_to_lab(color),
        }
    }

    fn find_nearest(&self, point: [f32; 3]) -> Candidate {
        let mut best = empty_candidate();
        if self.tree && point.iter().all(|v| v.is_finite()) {
            search_one(&self.entries, point, &mut best);
        } else {
            for &entry in &self.entries {
                let candidate = Candidate { distance: metric_distance(entry.point, point), index: entry.index, point: entry.point };
                if candidate_better(candidate, best) {
                    best = candidate;
                }
            }
        }
        best
    }
    fn find_two(&self, point: [f32; 3]) -> [Candidate; 2] {
        let mut best = [empty_candidate(), empty_candidate()];
        if self.tree && point.iter().all(|v| v.is_finite()) {
            search(&self.entries, point, &mut best);
        } else {
            for &entry in &self.entries {
                consider(entry, point, &mut best);
            }
        }
        best
    }

    #[inline]
    fn choose(&self, color: [f32; 3], threshold: f32) -> usize {
        let color = color.map(safe_rgb);
        let query = self.query_point(color);
        if threshold.abs() <= 1e-7 {
            let best = self.find_nearest(query);
            return if best.index == usize::MAX { 0 } else { best.index };
        }
        let best = self.find_two(query);
        if best[0].index == usize::MAX {
            return 0;
        }
        // A source member is already exactly represented.  In particular, a threshold must not
        // turn an exact palette colour into a needless neighbour.
        if best[1].index == usize::MAX || best[0].distance <= 1e-12 {
            return best[0].index;
        }
        // Threshold selection is performed in the same metric as nearest lookup (RGB or Lab).
        // Project the query onto the segment joining the two candidates, then shift the 0.5
        // boundary by the centred threshold.  This remains meaningful for arbitrary palettes,
        // unlike comparing Lab distances against an RGB-space colour span.
        let a = best[0].point;
        let b = best[1].point;
        let segment = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let denominator = metric_distance(segment, [0.0; 3]);
        if denominator <= 1e-12 {
            return best[0].index;
        }
        let projection = ((query[0] - a[0]) * segment[0] + (query[1] - a[1]) * segment[1] + (query[2] - a[2]) * segment[2]) / denominator;
        if projection + threshold.clamp(-0.5, 0.5) >= 0.5 { best[1].index } else { best[0].index }
    }
}

#[inline]
fn metric_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[inline]
fn candidate_better(a: Candidate, b: Candidate) -> bool {
    a.distance < b.distance || (a.distance == b.distance && a.index < b.index)
}

#[inline]
fn coincident(a: [f32; 3], b: [f32; 3]) -> bool {
    metric_distance(a, b) <= 1e-12
}

fn consider(entry: Entry, point: [f32; 3], best: &mut [Candidate; 2]) {
    let candidate = Candidate { distance: metric_distance(entry.point, point), index: entry.index, point: entry.point };
    if best[0].index == usize::MAX {
        best[0] = candidate;
        return;
    }
    if coincident(candidate.point, best[0].point) {
        if candidate.index < best[0].index {
            best[0] = candidate;
        }
        if best[1].index != usize::MAX && coincident(best[1].point, best[0].point) {
            best[1] = empty_candidate();
        }
        return;
    }
    if candidate_better(candidate, best[0]) {
        let previous = best[0];
        let previous_second = best[1];
        best[0] = candidate;
        best[1] = empty_candidate();
        for option in [previous, previous_second] {
            if option.index != usize::MAX && !coincident(option.point, best[0].point) && (best[1].index == usize::MAX || candidate_better(option, best[1])) {
                best[1] = option;
            }
        }
    } else if best[1].index == usize::MAX || candidate_better(candidate, best[1]) {
        best[1] = candidate;
    }
}

fn partition(entries: &mut [Entry]) {
    if entries.len() <= 8 {
        return;
    }
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for entry in entries.iter() {
        for axis in 0..3 {
            low[axis] = low[axis].min(entry.point[axis]);
            high[axis] = high[axis].max(entry.point[axis]);
        }
    }
    let axis = (0..3).max_by(|&a, &b| (high[a] - low[a]).total_cmp(&(high[b] - low[b]))).unwrap_or(0);
    let mid = entries.len() / 2;
    let (left, pivot, right) = entries.select_nth_unstable_by(mid, |a, b| a.point[axis].total_cmp(&b.point[axis]).then(a.index.cmp(&b.index)));
    pivot.axis = axis;
    partition(left);
    partition(right);
}

fn search_one(entries: &[Entry], point: [f32; 3], best: &mut Candidate) {
    if entries.len() <= 8 {
        for &entry in entries {
            let candidate = Candidate { distance: metric_distance(entry.point, point), index: entry.index, point: entry.point };
            if candidate_better(candidate, *best) {
                *best = candidate;
            }
        }
        return;
    }
    let (left, rest) = entries.split_at(entries.len() / 2);
    let Some((pivot, right)) = rest.split_first() else { return };
    let candidate = Candidate { distance: metric_distance(pivot.point, point), index: pivot.index, point: pivot.point };
    if candidate_better(candidate, *best) {
        *best = candidate;
    }
    let delta = point[pivot.axis] - pivot.point[pivot.axis];
    let (near, far) = if delta <= 0.0 { (left, right) } else { (right, left) };
    search_one(near, point, best);
    if best.index == usize::MAX || delta * delta <= best.distance {
        search_one(far, point, best);
    }
}

fn search(entries: &[Entry], point: [f32; 3], best: &mut [Candidate; 2]) {
    if entries.len() <= 8 {
        for &entry in entries {
            consider(entry, point, best);
        }
        return;
    }
    let (left, rest) = entries.split_at(entries.len() / 2);
    let Some((pivot, right)) = rest.split_first() else { return };
    consider(*pivot, point, best);
    let delta = point[pivot.axis] - pivot.point[pivot.axis];
    let (near, far) = if delta <= 0.0 { (left, right) } else { (right, left) };
    search(near, point, best);
    if best[1].index == usize::MAX || delta * delta <= best[1].distance {
        search(far, point, best);
    }
}

#[derive(Clone, Copy)]
struct Tap {
    dx: i32,
    dy: usize,
    weight: f32,
}

const FS_TAPS: [Tap; 4] = [
    Tap { dx: 1, dy: 0, weight: 7.0 / 16.0 },
    Tap { dx: -1, dy: 1, weight: 3.0 / 16.0 },
    Tap { dx: 0, dy: 1, weight: 5.0 / 16.0 },
    Tap { dx: 1, dy: 1, weight: 1.0 / 16.0 },
];
const SIERRA_LITE_TAPS: [Tap; 3] = [Tap { dx: 1, dy: 0, weight: 0.5 }, Tap { dx: -1, dy: 1, weight: 0.25 }, Tap { dx: 0, dy: 1, weight: 0.25 }];
const ATKINSON_TAPS: [Tap; 6] = [
    Tap { dx: 1, dy: 0, weight: 1.0 / 8.0 },
    Tap { dx: 2, dy: 0, weight: 1.0 / 8.0 },
    Tap { dx: -1, dy: 1, weight: 1.0 / 8.0 },
    Tap { dx: 0, dy: 1, weight: 1.0 / 8.0 },
    Tap { dx: 1, dy: 1, weight: 1.0 / 8.0 },
    Tap { dx: 0, dy: 2, weight: 1.0 / 8.0 },
];
const JARVIS_TAPS: [Tap; 12] = [
    Tap { dx: 1, dy: 0, weight: 7.0 / 48.0 },
    Tap { dx: 2, dy: 0, weight: 5.0 / 48.0 },
    Tap { dx: -2, dy: 1, weight: 3.0 / 48.0 },
    Tap { dx: -1, dy: 1, weight: 5.0 / 48.0 },
    Tap { dx: 0, dy: 1, weight: 7.0 / 48.0 },
    Tap { dx: 1, dy: 1, weight: 5.0 / 48.0 },
    Tap { dx: 2, dy: 1, weight: 3.0 / 48.0 },
    Tap { dx: -2, dy: 2, weight: 1.0 / 48.0 },
    Tap { dx: -1, dy: 2, weight: 3.0 / 48.0 },
    Tap { dx: 0, dy: 2, weight: 5.0 / 48.0 },
    Tap { dx: 1, dy: 2, weight: 3.0 / 48.0 },
    Tap { dx: 2, dy: 2, weight: 1.0 / 48.0 },
];
const STUCKI_TAPS: [Tap; 12] = [
    Tap { dx: 1, dy: 0, weight: 8.0 / 42.0 },
    Tap { dx: 2, dy: 0, weight: 4.0 / 42.0 },
    Tap { dx: -2, dy: 1, weight: 2.0 / 42.0 },
    Tap { dx: -1, dy: 1, weight: 4.0 / 42.0 },
    Tap { dx: 0, dy: 1, weight: 8.0 / 42.0 },
    Tap { dx: 1, dy: 1, weight: 4.0 / 42.0 },
    Tap { dx: 2, dy: 1, weight: 2.0 / 42.0 },
    Tap { dx: -2, dy: 2, weight: 1.0 / 42.0 },
    Tap { dx: -1, dy: 2, weight: 2.0 / 42.0 },
    Tap { dx: 0, dy: 2, weight: 4.0 / 42.0 },
    Tap { dx: 1, dy: 2, weight: 2.0 / 42.0 },
    Tap { dx: 2, dy: 2, weight: 1.0 / 42.0 },
];
const SIERRA_TAPS: [Tap; 10] = [
    Tap { dx: 1, dy: 0, weight: 5.0 / 32.0 },
    Tap { dx: 2, dy: 0, weight: 3.0 / 32.0 },
    Tap { dx: -2, dy: 1, weight: 2.0 / 32.0 },
    Tap { dx: -1, dy: 1, weight: 4.0 / 32.0 },
    Tap { dx: 0, dy: 1, weight: 5.0 / 32.0 },
    Tap { dx: 1, dy: 1, weight: 4.0 / 32.0 },
    Tap { dx: 2, dy: 1, weight: 2.0 / 32.0 },
    Tap { dx: -1, dy: 2, weight: 2.0 / 32.0 },
    Tap { dx: 0, dy: 2, weight: 3.0 / 32.0 },
    Tap { dx: 1, dy: 2, weight: 2.0 / 32.0 },
];
const BURKES_TAPS: [Tap; 7] = [
    Tap { dx: 1, dy: 0, weight: 8.0 / 32.0 },
    Tap { dx: 2, dy: 0, weight: 4.0 / 32.0 },
    Tap { dx: -2, dy: 1, weight: 2.0 / 32.0 },
    Tap { dx: -1, dy: 1, weight: 4.0 / 32.0 },
    Tap { dx: 0, dy: 1, weight: 8.0 / 32.0 },
    Tap { dx: 1, dy: 1, weight: 4.0 / 32.0 },
    Tap { dx: 2, dy: 1, weight: 2.0 / 32.0 },
];

fn diffusion_taps(method: Method) -> &'static [Tap] {
    match method {
        Method::FloydSteinberg => &FS_TAPS,
        Method::SierraLite => &SIERRA_LITE_TAPS,
        Method::Atkinson => &ATKINSON_TAPS,
        Method::Jarvis => &JARVIS_TAPS,
        Method::Stucki => &STUCKI_TAPS,
        Method::Sierra => &SIERRA_TAPS,
        Method::Burkes => &BURKES_TAPS,
        _ => &FS_TAPS,
    }
}

#[derive(Clone, Copy)]
struct Mapped {
    index: u32,
    rgb: [f32; 3],
    alpha: f32,
    colour: [f32; 3],
    diffuse: bool,
}

#[inline]
fn safe_sample(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else if value.is_sign_positive() {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn safe_rgb(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value.clamp(-MAX_MAPPING_RGB, MAX_MAPPING_RGB) }
}

#[inline]
fn alpha_opaque(mode: AlphaMode, alpha: f32, threshold: f32) -> bool {
    match mode {
        AlphaMode::Preserve => true,
        AlphaMode::Binary => alpha >= 0.5,
        AlphaMode::Dither => alpha + threshold >= 0.5,
    }
}

fn map_pixel(source: [f32; 4], error: [f32; 3], color_threshold: f32, alpha_threshold: f32, context: &MapContext<'_>) -> Mapped {
    let palette = context.palette;
    let settings = context.settings;
    let lookup = &context.lookup;
    let transparent = context.transparent;
    let alpha = safe_sample(source[3]);
    let opaque = alpha_opaque(settings.alpha, alpha, alpha_threshold);
    let colour = [safe_rgb(source[0]) + error[0], safe_rgb(source[1]) + error[1], safe_rgb(source[2]) + error[2]];
    if !opaque {
        if let Some(index) = transparent {
            let rgb = palette.colors[index];
            return Mapped { index: index as u32, rgb, alpha: 0.0, colour, diffuse: false };
        }
        let index = lookup.choose(colour, color_threshold);
        return Mapped { index: index as u32, rgb: palette.colors[index], alpha: 0.0, colour, diffuse: alpha > 0.0 || settings.alpha == AlphaMode::Dither };
    }
    let index = lookup.choose(colour, color_threshold);
    let rgb = palette.colors[index];
    let out_alpha = match settings.alpha {
        AlphaMode::Preserve => alpha,
        AlphaMode::Binary | AlphaMode::Dither => 1.0,
    };
    let diffuse = !(alpha == 0.0 && transparent.is_none() && matches!(settings.alpha, AlphaMode::Preserve | AlphaMode::Binary));
    Mapped { index: index as u32, rgb, alpha: out_alpha, colour, diffuse }
}

fn render_local(px: &mut [[f32; 4]], context: &MapContext<'_>, indices: Option<&mut [u32]>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    let width = context.width;
    let stopped = AtomicBool::new(false);
    if let Some(indexes) = indices {
        crate::photo_util::par_rows2(indexes, px, width, |row, irow, prow| {
            map_local_row(row, prow, Some(irow), context, &stopped, cancelled);
        });
    } else {
        crate::photo_util::par_rows(px, width, 1, |row, prow| {
            map_local_row(row, prow, None, context, &stopped, cancelled);
        });
    }
    if stopped.load(Ordering::Relaxed) || cancelled() { Err("palette mapping cancelled".to_owned()) } else { Ok(()) }
}

fn map_local_row(
    row: usize,
    pixels: &mut [[f32; 4]],
    mut indexes: Option<&mut [u32]>,
    context: &MapContext<'_>,
    stopped: &AtomicBool,
    cancelled: &(dyn Fn() -> bool + Sync),
) {
    let settings = context.settings;
    let plan = &context.plan;
    let origin = context.origin;
    if stopped.load(Ordering::Relaxed) || cancelled() {
        stopped.store(true, Ordering::Relaxed);
        return;
    }
    let gy = i64::from(origin[1]) + row as i64;
    for (column, pixel) in pixels.iter_mut().enumerate() {
        if column.is_multiple_of(256) && (stopped.load(Ordering::Relaxed) || cancelled()) {
            stopped.store(true, Ordering::Relaxed);
            return;
        }
        let gx = i64::from(origin[0]) + column as i64;
        let color_threshold = plan.value(settings, gx, gy);
        let alpha_threshold = if settings.alpha == AlphaMode::Dither && !plan.has_ingredients { bayer_value(gx, gy, 4, 1, [0, 0]) } else { color_threshold };
        let mapped = map_pixel(*pixel, [0.0; 3], color_threshold, alpha_threshold, context);
        if let Some(indexes) = indexes.as_deref_mut() {
            indexes[column] = mapped.index;
        }
        *pixel = [mapped.rgb[0], mapped.rgb[1], mapped.rgb[2], mapped.alpha];
    }
}

fn render_diffusion(
    px: &mut [[f32; 4]],
    context: &MapContext<'_>,
    mut indices: Option<&mut [u32]>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    let width = context.width;
    let height = context.height;
    let width_i64 = width as i64; // Validated by render_internal.
    let settings = context.settings;
    let plan = &context.plan;
    let origin = context.origin;
    let taps = diffusion_taps(settings.method);
    let rows = taps.iter().map(|tap| tap.dy).max().unwrap_or(0).saturating_add(1).min(height);
    let cells = width.checked_mul(rows).ok_or("diffusion row buffer size overflow")?;
    let mut errors = Vec::new();
    errors.try_reserve_exact(cells).map_err(|_| "could not allocate diffusion row buffer")?;
    errors.resize(cells, [0.0f32; 3]);
    for y in 0..height {
        if cancelled() {
            return Err("palette mapping cancelled".to_owned());
        }
        let reverse = settings.serpentine && y % 2 == 1;
        for step in 0..width {
            if step.is_multiple_of(256) && cancelled() {
                return Err("palette mapping cancelled".to_owned());
            }
            let x = if reverse { width - 1 - step } else { step };
            let i = y * width + x;
            let current = (y % rows) * width + x;
            let gx = i64::from(origin[0]) + x as i64;
            let gy = i64::from(origin[1]) + y as i64;
            let color_threshold = plan.value(settings, gx, gy);
            let alpha_threshold =
                if settings.alpha == AlphaMode::Dither && !plan.has_ingredients { bayer_value(gx, gy, 4, 1, [0, 0]) } else { color_threshold };
            let mapped = map_pixel(px[i], errors[current], color_threshold, alpha_threshold, context);
            if let Some(indexes) = indices.as_deref_mut() {
                indexes[i] = mapped.index;
            }
            px[i] = [mapped.rgb[0], mapped.rgb[1], mapped.rgb[2], mapped.alpha];
            errors[current] = [0.0; 3];
            if mapped.diffuse {
                let dx_sign = if reverse { -1i64 } else { 1i64 };
                let gain = settings.amount;
                for tap in taps {
                    let Some(target_y) = y.checked_add(tap.dy) else { continue };
                    if target_y >= height {
                        continue;
                    }
                    let tx = x as i64 + dx_sign * i64::from(tap.dx);
                    if tx < 0 || tx >= width_i64 {
                        continue;
                    }
                    let target = (target_y % rows) * width + tx as usize;
                    let weight = tap.weight * gain;
                    for (error, (&colour, &rgb)) in errors[target].iter_mut().zip(mapped.colour.iter().zip(&mapped.rgb)) {
                        *error += (colour - rgb) * weight;
                    }
                }
            }
        }
    }
    Ok(())
}

fn render_riemersma(
    px: &mut [[f32; 4]],
    context: &MapContext<'_>,
    mut indices: Option<&mut [u32]>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    let width = context.width;
    let height = context.height;
    let settings = context.settings;
    let plan = &context.plan;
    let origin = context.origin;
    let mut queue = vec![[0.0f32; 3]; settings.history as usize];
    let mut weights = vec![0.0f32; queue.len()];
    let retention = 1.0 - settings.decay;
    let mut sum = 0.0;
    for (i, weight) in weights.iter_mut().enumerate() {
        *weight = retention.powi(i as i32);
        sum += *weight;
    }
    if sum > 0.0 {
        for weight in &mut weights {
            *weight /= sum;
        }
    }
    let mut head = 0usize;
    let mut visited = 0usize;
    let mut stopped = false;
    let completed = visit_hilbert_rect(width, height, |x, y| {
        if visited.is_multiple_of(256) && cancelled() {
            stopped = true;
            return false;
        }
        visited += 1;
        let i = y * width + x;
        let gx = i64::from(origin[0]) + x as i64;
        let gy = i64::from(origin[1]) + y as i64;
        let color_threshold = plan.value(settings, gx, gy);
        let alpha_threshold = if settings.alpha == AlphaMode::Dither && !plan.has_ingredients { bayer_value(gx, gy, 4, 1, [0, 0]) } else { color_threshold };
        let mut feedback = [0.0f32; 3];
        for (offset, &weight) in weights.iter().enumerate() {
            let error = queue[(head + offset) % queue.len()];
            for channel in 0..3 {
                feedback[channel] += error[channel] * weight;
            }
        }
        let mapped = map_pixel(px[i], feedback.map(|v| v * settings.amount), color_threshold, alpha_threshold, context);
        if let Some(indexes) = indices.as_deref_mut() {
            indexes[i] = mapped.index;
        }
        px[i] = [mapped.rgb[0], mapped.rgb[1], mapped.rgb[2], mapped.alpha];
        let error = if mapped.diffuse { std::array::from_fn(|channel| mapped.colour[channel] - mapped.rgb[channel]) } else { [0.0; 3] };
        head = (head + queue.len() - 1) % queue.len();
        queue[head] = error;
        true
    });
    if !completed || stopped || cancelled() { Err("palette mapping cancelled".to_owned()) } else { Ok(()) }
}

/// Visits every cell of a rectangle in a clipped Hilbert traversal.  The recursion prunes padded
/// square nodes before descending, so a very wide or very tall image does not walk a huge virtual
/// square.  A row-major fallback handles dimensions whose next power of two cannot be represented.
fn visit_hilbert_rect(width: usize, height: usize, mut visit: impl FnMut(usize, usize) -> bool) -> bool {
    if width == 0 || height == 0 {
        return true;
    }
    let max_side = width.max(height);
    let mut side = 1usize;
    while side < max_side {
        let Some(next) = side.checked_mul(2) else {
            return visit_rectangle(width, height, &mut visit);
        };
        side = next;
    }
    let Ok(side_i64) = i64::try_from(side) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let Ok(width_i64) = i64::try_from(width) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let Ok(height_i64) = i64::try_from(height) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let Some(scale_i64) = side_i64.checked_mul(2) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let Some(width_scaled) = width_i64.checked_mul(2) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let Some(height_scaled) = height_i64.checked_mul(2) else {
        return visit_rectangle(width, height, &mut visit);
    };
    let order = side.trailing_zeros() as usize;
    visit_hilbert_node(order, [0, 0], [[scale_i64, 0], [0, scale_i64]], [width_scaled, height_scaled], &mut visit)
}

fn visit_rectangle(width: usize, height: usize, visit: &mut impl FnMut(usize, usize) -> bool) -> bool {
    for y in 0..height {
        for x in 0..width {
            if !visit(x, y) {
                return false;
            }
        }
    }
    true
}

fn visit_hilbert_node(order: usize, origin: [i64; 2], axes: [[i64; 2]; 2], extent: [i64; 2], visit: &mut impl FnMut(usize, usize) -> bool) -> bool {
    let [x0, y0] = origin;
    let [[xi, xj], [yi, yj]] = axes;
    let [width, height] = extent;
    let corners_x = [x0, x0.saturating_add(xi), x0.saturating_add(yi), x0.saturating_add(xi).saturating_add(yi)];
    let corners_y = [y0, y0.saturating_add(xj), y0.saturating_add(yj), y0.saturating_add(xj).saturating_add(yj)];
    let min_x = corners_x.iter().copied().min().unwrap_or(0);
    let max_x = corners_x.iter().copied().max().unwrap_or(0);
    let min_y = corners_y.iter().copied().min().unwrap_or(0);
    let max_y = corners_y.iter().copied().max().unwrap_or(0);
    if max_x < 0 || max_y < 0 || min_x >= width || min_y >= height {
        return true;
    }
    if order == 0 {
        let x = x0 + (xi + yi).div_euclid(2);
        let y = y0 + (xj + yj).div_euclid(2);
        if x >= 0 && y >= 0 && x < width && y < height {
            let Ok(x) = usize::try_from(x.div_euclid(2)) else { return true };
            let Ok(y) = usize::try_from(y.div_euclid(2)) else { return true };
            return visit(x, y);
        }
        return true;
    }
    let next = order - 1;
    if !visit_hilbert_node(next, [x0, y0], [[yi / 2, yj / 2], [xi / 2, xj / 2]], extent, visit) {
        return false;
    }
    if !visit_hilbert_node(next, [x0 + xi / 2, y0 + xj / 2], [[xi / 2, xj / 2], [yi / 2, yj / 2]], extent, visit) {
        return false;
    }
    if !visit_hilbert_node(next, [x0 + xi / 2 + yi / 2, y0 + xj / 2 + yj / 2], [[xi / 2, xj / 2], [yi / 2, yj / 2]], extent, visit) {
        return false;
    }
    visit_hilbert_node(next, [x0 + xi / 2 + yi, y0 + xj / 2 + yj], [[-yi / 2, -yj / 2], [-xi / 2, -xj / 2]], extent, visit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black_white() -> Palette {
        Palette { name: "bw".to_owned(), colors: vec![[0.0; 3], [1.0; 3]] }
    }

    #[test]
    fn floyd_steinberg_uses_the_standard_forward_stencil() {
        let mut pixels = vec![[0.4, 0.4, 0.4, 1.0]; 16];
        let settings = Settings { method: Method::FloydSteinberg, serpentine: false, ..Settings::default() };
        let indices = render(&mut pixels, 4, &black_white(), &settings, [0, 0], None).unwrap();
        assert_eq!(indices, vec![0, 1, 0, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0, 1]);
    }

    #[test]
    fn validates_float_palettes_and_preserves_member_entries_above_u8_index() {
        let colors: Vec<[f32; 3]> = (0..=4095).map(|i| [i as f32 / 4095.0, 0.25, 0.75]).collect();
        let palette = Palette { name: "wide".to_owned(), colors };
        let mut px = vec![[palette.colors[3000][0], 0.25, 0.75, 1.0]];
        let result = render(&mut px, 1, &palette, &Settings::default(), [17, -3], None);
        assert_eq!(result, Ok(vec![3000]));
        assert_eq!(px[0], [palette.colors[3000][0], 0.25, 0.75, 1.0]);
    }

    #[test]
    fn extended_srgb_members_remain_distinct_in_rgb_and_lab_with_dithering() {
        let palette = Palette { name: "ICC comparison colours".into(), colors: vec![[-0.2, 0.4, 1.2], [-0.1, 0.4, 1.1], [1.2, -0.1, 0.2], [0.0; 3]] };
        assert!(palette.validate().is_err(), "serialized source palettes remain normalized");
        let source: Vec<_> = palette.colors.iter().cycle().take(8).map(|rgb| [rgb[0], rgb[1], rgb[2], 1.0]).collect();
        for distance in [Distance::Rgb, Distance::Lab] {
            for method in [Method::None, Method::Bayer, Method::FloydSteinberg, Method::Riemersma] {
                let mut pixels = source.clone();
                let settings = Settings { method, distance, ..Settings::default() };
                let indices = render(&mut pixels, 4, &palette, &settings, [0, 0], None).unwrap();
                assert_eq!(indices, [0, 1, 2, 3, 0, 1, 2, 3], "{distance:?} {method:?}");
                assert_eq!(pixels, source, "{distance:?} {method:?}");
            }
        }
    }

    #[test]
    fn ids_include_legacy_command_spellings() {
        assert_eq!(Method::from_id("diffusion"), Some(Method::FloydSteinberg));
        assert_eq!(Method::from_id("pattern"), Some(Method::Bayer));
        assert_eq!(Method::from_id("noise"), Some(Method::WhiteNoise));
    }

    #[test]
    fn bayer_and_white_noise_are_origin_stable() {
        let palette = black_white();
        let mut a = vec![[0.5, 0.5, 0.5, 1.0]; 24];
        let mut b = a.clone();
        let settings = Settings { method: Method::Bayer, amount: 1.0, matrix_size: 4, ..Settings::default() };
        let ia = render(&mut a, 6, &palette, &settings, [100, -20], None);
        let ib = render(&mut b, 6, &palette, &settings, [100, -20], None);
        assert_eq!(ia, ib);
        assert_eq!(a, b);
        let mut c = vec![[0.5, 0.5, 0.5, 1.0]; 24];
        let ic = render(&mut c, 6, &palette, &settings, [101, -20], None);
        assert_ne!(ia, ic);
    }

    #[test]
    fn bayer_half_tone_keeps_two_by_two_checker_cells() {
        let palette = black_white();
        let settings = Settings { method: Method::Bayer, matrix_size: 4, ..Settings::default() };
        let mut px = vec![[0.5, 0.5, 0.5, 1.0]; 64 * 64];
        let result = render(&mut px, 64, &palette, &settings, [0, 0], None);
        let Some(indices) = result.ok() else { return };
        for y in (0..64).step_by(2) {
            for x in (0..64).step_by(2) {
                let mut white = 0usize;
                for dy in 0..2 {
                    for dx in 0..2 {
                        white += usize::from(indices[(y + dy) * 64 + x + dx] == 1);
                    }
                }
                assert_eq!(white, 2);
            }
        }
    }

    #[test]
    fn diffusion_conserves_a_gray_ramp_without_changing_palette_members() {
        let palette = black_white();
        let mut px: Vec<[f32; 4]> = (0..256).map(|i| [i as f32 / 255.0, i as f32 / 255.0, i as f32 / 255.0, 1.0]).collect();
        let before = px.clone();
        let settings = Settings { method: Method::FloydSteinberg, amount: 1.0, ..Settings::default() };
        let result = render(&mut px, 256, &palette, &settings, [0, 0], None);
        assert!(result.is_ok());
        let white = result.ok().map_or(0, |indices| indices.into_iter().filter(|&i| i == 1).count());
        assert!((white as isize - 128).abs() <= 3);
        for (source, output) in before.iter().zip(&px) {
            if source[0] == 0.0 || source[0] == 1.0 {
                assert_eq!(output[0], source[0]);
            }
        }
    }

    #[test]
    fn all_stateful_methods_cover_a_non_square_region() {
        let palette = black_white();
        for method in [Method::SierraLite, Method::Atkinson, Method::Jarvis, Method::Stucki, Method::Sierra, Method::Burkes, Method::Riemersma] {
            let mut px = vec![[0.37, 0.37, 0.37, 1.0]; 35];
            let settings = Settings { method, history: 4, ..Settings::default() };
            let result = render(&mut px, 7, &palette, &settings, [-9, 11], None);
            assert_eq!(result.as_ref().map(Vec::len), Ok(35));
            assert!(px.iter().all(|p| p[0] == 0.0 || p[0] == 1.0));
        }
    }

    #[test]
    fn alpha_dither_uses_reserved_transparency_and_preserves_rgb_filter_without_one() {
        let palette = Palette { name: "rgb".to_owned(), colors: vec![[0.0; 3], [1.0; 3], [0.3, 0.4, 0.5]] };
        let settings = Settings { alpha: AlphaMode::Binary, ..Settings::default() };
        let mut with_reserved = vec![[0.8, 0.8, 0.8, 0.2], [0.8, 0.8, 0.8, 0.8]];
        let reserved = render(&mut with_reserved, 2, &palette, &settings, [0, 0], Some(0));
        assert_eq!(reserved, Ok(vec![0, 1]));
        assert_eq!(with_reserved[0][3], 0.0);
        let mut without_reserved = vec![[0.8, 0.8, 0.8, 0.2]];
        let no_reserved = render(&mut without_reserved, 1, &palette, &settings, [0, 0], None);
        assert_eq!(no_reserved, Ok(vec![1]));
        assert_eq!(without_reserved[0][3], 0.0);
    }

    #[test]
    fn fully_transparent_preserve_pixel_does_not_seed_diffusion_without_reservation() {
        let palette = black_white();
        let settings = Settings { method: Method::FloydSteinberg, alpha: AlphaMode::Preserve, ..Settings::default() };
        let mut px = vec![[0.5, 0.5, 0.5, 0.0], [0.4, 0.4, 0.4, 1.0]];
        let result = render(&mut px, 2, &palette, &settings, [0, 0], None);
        assert_eq!(result, Ok(vec![0, 0]));
    }

    #[test]
    fn custom_patterns_compose_and_disabled_patterns_do_not_change_output() {
        let palette = black_white();
        let custom = Pattern { kind: PatternKind::Custom, matrix_size: 2, matrix: vec![0.0, 1.0, 1.0, 0.0], ..Pattern::default() };
        let mut a = vec![[0.5, 0.5, 0.5, 1.0]; 16];
        let mut b = a.clone();
        let one = Settings { patterns: vec![custom.clone()], ..Settings::default() };
        let two = Settings { patterns: vec![Pattern { enabled: false, ..custom }], ..Settings::default() };
        let ia = render(&mut a, 4, &palette, &one, [0, 0], None);

        let ib = render(&mut b, 4, &palette, &two, [0, 0], None);
        assert!(ia.is_ok() && ib.is_ok());
        assert_ne!(ia, ib);
        let mut c = vec![[0.5, 0.5, 0.5, 1.0]; 16];
        let none = render(&mut c, 4, &palette, &Settings::default(), [0, 0], None);
        assert_eq!(ib, none);
    }

    #[test]
    fn custom_pattern_rejects_centered_or_out_of_range_cells() {
        let pattern = Pattern { kind: PatternKind::Custom, matrix_size: 2, matrix: vec![-0.1, 0.5, 0.5, 1.0], ..Pattern::default() };
        let settings = Settings { patterns: vec![pattern], ..Settings::default() };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn cdf_calibrated_mixtures_cover_gray_tones_without_brightness_bias() {
        let palette = black_white();
        for kind in [PatternKind::WhiteNoise, PatternKind::BlueNoise] {
            for tone in [0.1f32, 0.25, 0.5, 0.75, 0.9] {
                let settings = Settings {
                    method: Method::Bayer,
                    amount: 0.5,
                    patterns: vec![Pattern { kind, amount: 0.5, seed: 19, ..Pattern::default() }],
                    ..Settings::default()
                };
                let mut px = vec![[tone, tone, tone, 1.0]; 64 * 64];
                let result = render(&mut px, 64, &palette, &settings, [-100, 37], None);
                assert!(result.is_ok());
                let coverage = result.ok().map_or(0.0, |indices| indices.into_iter().filter(|&index| index == 1).count() as f32 / (64.0 * 64.0));
                assert!((coverage - tone).abs() < 0.06, "kind {kind:?} tone {tone} coverage {coverage}");
            }
        }
    }

    #[test]
    fn duplicate_palette_entries_do_not_consume_the_second_threshold_candidate() {
        let palette = Palette { name: "duplicates".to_owned(), colors: vec![[0.0; 3], [0.0; 3], [1.0; 3]] };
        for distance in [Distance::Rgb, Distance::Lab] {
            let settings = Settings { method: Method::Bayer, distance, ..Settings::default() };
            let mut px = vec![[0.5, 0.5, 0.5, 1.0]; 16 * 16];
            let result = render(&mut px, 16, &palette, &settings, [0, 0], None);
            assert!(result.is_ok());
            if let Ok(indices) = result {
                assert!(indices.iter().all(|&index| index != 1));
                assert!(indices.contains(&2));
            }
        }
    }

    #[test]
    fn alpha_dither_has_expected_coverage_and_does_not_enable_rgb_dither() {
        let palette = black_white();
        let settings = Settings { alpha: AlphaMode::Dither, ..Settings::default() };
        for tone in [0.1f32, 0.25, 0.5, 0.75, 0.9] {
            let mut px = vec![[0.5, 0.5, 0.5, tone]; 16 * 16];
            let result = render(&mut px, 16, &palette, &settings, [0, 0], None);
            assert!(result.is_ok());
            let coverage = px.iter().filter(|p| p[3] >= 0.5).count() as f32 / 256.0;
            assert!((coverage - tone).abs() < 0.1, "tone {tone} coverage {coverage}");
            assert!(result.ok().is_some_and(|indices| indices.iter().all(|&index| index == 0)));
        }
    }

    #[test]
    fn render_pixels_avoids_return_plane_but_matches_render_output() {
        let palette = black_white();
        let settings = Settings { method: Method::Bayer, ..Settings::default() };
        let mut pixels = vec![[0.37, 0.37, 0.37, 1.0]; 24];
        let mut expected = pixels.clone();
        let indexes = render(&mut expected, 6, &palette, &settings, [3, -4], None);
        let result = render_pixels(&mut pixels, 6, &palette, &settings, [3, -4], None);
        assert!(result.is_ok());
        assert_eq!(pixels, expected);
        assert!(indexes.is_ok());
    }

    #[test]
    fn hilbert_rectangle_visits_each_cell_once_without_padding_duplicates() {
        for (width, height) in [(1usize, 1usize), (2, 2), (3, 5), (17, 3), (31, 19)] {
            let mut seen = vec![false; width * height];
            let mut count = 0usize;
            let completed = visit_hilbert_rect(width, height, |x, y| {
                let cell = y * width + x;
                if seen[cell] {
                    return false;
                }
                seen[cell] = true;
                count += 1;
                true
            });
            assert!(completed);
            assert_eq!(count, width * height);
            assert!(seen.into_iter().all(|cell| cell));
        }
    }

    fn low_frequency_power(samples: &[f32], coverage: f32) -> f32 {
        let side = BlueNoise::SIDE;
        let cutoff = coverage - 0.5;
        let harmonics = [(1i32, 0i32), (0, 1), (1, 1), (2, 0), (0, 2)];
        let mut power = 0.0f32;
        for &(kx, ky) in &harmonics {
            let mut real = 0.0f32;
            let mut imag = 0.0f32;
            for y in 0..side {
                for x in 0..side {
                    if samples[y * side + x] >= cutoff {
                        continue;
                    }
                    let angle = std::f32::consts::TAU * (kx as f32 * x as f32 + ky as f32 * y as f32) / side as f32;
                    real += angle.cos();
                    imag += angle.sin();
                }
            }
            power += real * real + imag * imag;
        }
        power
    }

    #[test]
    fn blue_noise_suppresses_low_frequency_harmonics_at_multiple_coverages() {
        let never_cancelled = || false;
        let blue = BlueNoise::generate(29, &never_cancelled);
        assert!(blue.is_ok());
        let Some(blue) = blue.ok() else { return };
        let white: Vec<f32> =
            (0..BlueNoise::CELLS).map(|cell| white_value((cell % BlueNoise::SIDE) as i64, (cell / BlueNoise::SIDE) as i64, 1, [0, 0], 29)).collect();
        for coverage in [0.1f32, 0.5, 0.9] {
            let blue_power = low_frequency_power(blue.ranks.as_slice(), coverage);
            let white_power = low_frequency_power(&white, coverage);
            assert!(blue_power < white_power, "coverage {coverage} blue {blue_power} white {white_power}");
            // Suppressing low harmonics alone also accepts a perfectly regular checkerboard.
            // A dispersed blue-noise screen must not lock to that lattice at any coverage.
            let mut checker = 0.0f32;
            for y in 0..BlueNoise::SIDE {
                for x in 0..BlueNoise::SIDE {
                    if blue.ranks[y * BlueNoise::SIDE + x] < coverage - 0.5 {
                        checker += if (x + y).is_multiple_of(2) { 1.0 } else { -1.0 };
                    }
                }
            }
            let minority = coverage.min(1.0 - coverage) * BlueNoise::CELLS as f32;
            assert!(checker.abs() < minority * 0.25, "coverage {coverage} checkerboard amplitude {checker}");
        }
    }

    #[test]
    fn cancellation_is_reported_before_mutating_pixels() {
        let palette = black_white();
        let mut px = vec![[0.2, 0.2, 0.2, 1.0]; 16];
        let before = px.clone();
        let cancel = || true;
        let result = render_with(&mut px, 4, &palette, &Settings::default(), [0, 0], None, &cancel);
        assert!(result.is_err());
        assert_eq!(px, before);
    }

    #[test]
    fn settings_round_trip_through_serde_keeps_custom_matrix() {
        let settings = Settings {
            method: Method::BlueNoise,
            amount: 0.7,
            patterns: vec![Pattern { kind: PatternKind::Custom, matrix_size: 4, matrix: vec![0.25; 16], phase: [-3, 8], ..Pattern::default() }],
            ..Settings::default()
        };
        let encoded = serde_json::to_string(&settings);
        assert!(encoded.is_ok());
        let decoded: Option<Settings> = encoded.ok().and_then(|json| serde_json::from_str(&json).ok());
        assert!(decoded.is_some());
        if let Some(value) = decoded {
            assert_eq!(value, settings);
        }
    }
}
