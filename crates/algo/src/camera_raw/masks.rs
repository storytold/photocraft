//! Camera Raw's Masks: local adjustments through masks built from components.
//!
//! A [`Mask`] is a list of [`Component`]s combined in order (the first adds, later ones add,
//! subtract or intersect, each optionally inverted) and a set of [`Local`] adjustments applied
//! where the mask is on, scaled by its Amount. Components are a brush (dabs with size, feather,
//! flow and density, or erasing), a linear gradient, a radial gradient, a colour range, a
//! luminance range, and the AI kinds Camera Raw offers (Subject, Sky, Background, People,
//! Objects), which here use PhotoCraft's own classical segmentation (see [`crate::segment`]):
//! they find comparable regions, not the ones Adobe's models find. Depth Range needs a depth map
//! that rendered (non-raw) pixels don't have and is refused.
//!
//! Geometry runs first, so mask coordinates are those of the developed result: 0 … 1 across its
//! width and height, as Camera Raw stores them.
//!
//! Local adjustments are applied by developing the pixels again with the mask's sliders as the
//! global ones (Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Temperature, Tint,
//! Texture, Clarity, Dehaze, Saturation, Hue) and blending that result in by the mask: the
//! local sliders follow the measured global ones.

#![allow(clippy::needless_range_loop)]

use serde::{Deserialize, Serialize};

use super::CameraRaw;

pub mod extra;
pub mod shapes;

/// Most masks the command accepts.
pub const MAX_MASKS: usize = 32;
/// Most components in one mask.
pub const MAX_COMPONENTS: usize = 32;
/// Most brush dabs in one brush component.
pub const MAX_DABS: usize = 4_000;
/// Largest brush radius, as a fraction of the long side.
pub const MAX_DAB_SIZE: f32 = 0.25;
/// Most sampled colours in a colour range.
pub const MAX_SAMPLES: usize = 5;

/// How a component combines with the components before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    #[default]
    Add,
    Subtract,
    Intersect,
}

/// One brush dab, in 0 … 1 image coordinates (`size` as a fraction of the long side).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dab {
    pub x: f32,
    pub y: f32,
    /// Radius as a fraction of the image's long side.
    pub size: f32,
    /// 0 … 100: how far in from the edge the dab starts to fade.
    pub feather: f32,
    /// 0 … 100: how much one dab adds.
    pub flow: f32,
    /// 0 … 100: the most the brush builds up to.
    pub density: f32,
    /// Erases instead of painting.
    pub erase: bool,
    /// Auto Mask: paints only over colours like the one under the dab's centre.
    pub auto: bool,
}

impl Default for Dab {
    fn default() -> Self {
        Dab { x: 0.5, y: 0.5, size: 0.05, feather: 50.0, flow: 100.0, density: 100.0, erase: false, auto: false }
    }
}

/// What a component selects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    /// Brushed dabs, in order.
    Brush {
        #[serde(default)]
        dabs: Vec<Dab>,
    },
    /// Fully on at `full`, off at `zero` (0 … 1 image coordinates), smooth in between.
    Linear { zero: [f32; 2], full: [f32; 2] },
    /// An ellipse: `center`, half axes `radius` (fractions of width and height), `angle` in
    /// degrees, `feather` 0 … 100; `inside: false` selects the outside (Camera Raw's Invert).
    Radial {
        center: [f32; 2],
        radius: [f32; 2],
        #[serde(default)]
        angle: f32,
        #[serde(default = "fifty")]
        feather: f32,
    },
    /// Colours near the sampled ones (straight RGB 0 … 1), `amount` 0 … 100 how wide.
    Color {
        samples: Vec<[f32; 3]>,
        #[serde(default = "fifty")]
        amount: f32,
    },
    /// Luminance (0 … 100) between `range`, fading over `smoothness` (0 … 100).
    Luminance {
        range: [f32; 2],
        #[serde(default = "fifty")]
        smoothness: f32,
    },
    /// Depth Range: not available on rendered pixels.
    Depth {
        #[serde(default)]
        range: [f32; 2],
    },
    Subject,
    Sky,
    Background,
    /// People found by their faces; `part` picks the whole person, the faces or the skin.
    People {
        #[serde(default)]
        part: PeoplePart,
    },
    /// An object inside `rect` (`[x0, y0, x1, y1]`, 0 … 1).
    Objects { rect: [f32; 4] },
}

fn fifty() -> f32 {
    50.0
}

fn hundred() -> f32 {
    100.0
}

fn yes() -> bool {
    true
}

/// Which part of the people a People component selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeoplePart {
    #[default]
    Person,
    Face,
    Skin,
}

/// One component of a mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    #[serde(default)]
    pub op: Op,
    #[serde(default)]
    pub invert: bool,
    #[serde(flatten)]
    pub shape: Shape,
}

/// A mask's local adjustments, in Camera Raw's local slider units.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Local {
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
    /// −100 … 100 (Camera Raw's local Hue turns colours up to ±180°... here ±30°, as the Color
    /// Mixer's hue sliders).
    pub hue: f32,
    pub saturation: f32,
    pub sharpness: f32,
    pub noise: f32,
    /// Color: the swatch's hue (degrees) and saturation (0 … 100).
    pub color_hue: f32,
    pub color_sat: f32,
    /// Moiré reduction, 0 … 100.
    pub moire: f32,
    /// Defringe, 0 … 100.
    pub defringe: f32,
}

impl Local {
    pub fn is_neutral(&self) -> bool {
        [
            self.temperature,
            self.tint,
            self.exposure,
            self.contrast,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
            self.texture,
            self.clarity,
            self.dehaze,
            self.hue,
            self.saturation,
            self.sharpness,
            self.noise,
            self.color_sat,
            self.moire,
            self.defringe,
        ]
        .iter()
        .all(|v| *v == 0.0 || !v.is_finite())
    }
}

/// One mask of the Masks panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mask {
    pub name: String,
    /// The eye button: a hidden mask applies nothing.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Inverts the whole mask.
    pub invert: bool,
    /// Amount, percent 0 … 200.
    #[serde(default = "hundred")]
    pub amount: f32,
    pub components: Vec<Component>,
    pub adjust: Local,
}

impl Default for Mask {
    fn default() -> Self {
        Mask { name: String::new(), visible: true, invert: false, amount: 100.0, components: Vec::new(), adjust: Local::default() }
    }
}

impl Mask {
    /// Whether the mask changes anything.
    pub fn is_active(&self) -> bool {
        self.visible && !self.components.is_empty() && !self.adjust.is_neutral() && self.amount.is_finite() && self.amount > 0.0
    }
}

/// Checks masks from untrusted input: counts, finite coordinates and Depth Range.
pub fn validate(masks: &[Mask]) -> Result<(), String> {
    if masks.len() > MAX_MASKS {
        return Err(format!("at most {MAX_MASKS} masks"));
    }
    for (i, m) in masks.iter().enumerate() {
        if m.components.len() > MAX_COMPONENTS {
            return Err(format!("mask {i}: at most {MAX_COMPONENTS} components"));
        }
        for c in &m.components {
            match &c.shape {
                Shape::Brush { dabs } => {
                    if dabs.len() > MAX_DABS {
                        return Err(format!("mask {i}: at most {MAX_DABS} brush dabs"));
                    }
                    if dabs.iter().any(|d| ![d.x, d.y, d.size, d.feather, d.flow, d.density].iter().all(|v| v.is_finite())) {
                        return Err(format!("mask {i}: brush dabs must be finite"));
                    }
                }
                Shape::Linear { zero, full } => {
                    if !zero.iter().chain(full).all(|v| v.is_finite()) {
                        return Err(format!("mask {i}: gradient points must be finite"));
                    }
                }
                Shape::Radial { center, radius, angle, feather } => {
                    if !center.iter().chain(radius).chain([angle, feather]).all(|v| v.is_finite()) {
                        return Err(format!("mask {i}: radial gradient values must be finite"));
                    }
                }
                Shape::Color { samples, amount } => {
                    if samples.is_empty() || samples.len() > MAX_SAMPLES {
                        return Err(format!("mask {i}: a colour range takes 1 to {MAX_SAMPLES} samples"));
                    }
                    if !samples.iter().flatten().chain([amount]).all(|v| v.is_finite()) {
                        return Err(format!("mask {i}: colour range values must be finite"));
                    }
                }
                Shape::Luminance { range, smoothness } => {
                    if !range.iter().chain([smoothness]).all(|v| v.is_finite()) {
                        return Err(format!("mask {i}: luminance range values must be finite"));
                    }
                }
                Shape::Depth { .. } => return Err(format!("mask {i}: Depth Range needs a depth map, which only raw captures with depth data have")),
                Shape::Objects { rect } => {
                    if !rect.iter().all(|v| v.is_finite()) {
                        return Err(format!("mask {i}: object rectangle must be finite"));
                    }
                }
                Shape::Subject | Shape::Sky | Shape::Background | Shape::People { .. } => {}
            }
        }
    }
    Ok(())
}

/// The mask's coverage (0 … 1) per pixel of a `w × h` image `px` (the developed pixels the
/// range and AI components look at).
pub fn coverage(m: &Mask, px: &[[f32; 4]], w: usize, h: usize) -> Vec<f32> {
    let n = w.saturating_mul(h);
    let mut acc: Option<Vec<f32>> = None;
    for c in &m.components {
        let mut v = shapes::render(&c.shape, px, w, h);
        if c.invert {
            for x in v.iter_mut() {
                *x = 1.0 - *x;
            }
        }
        acc = Some(match (acc, c.op) {
            (None, Op::Subtract) => vec![0.0; n],
            (None, _) => v,
            (Some(mut a), Op::Add) => {
                for (x, y) in a.iter_mut().zip(&v) {
                    *x = *x + *y - *x * *y;
                }
                a
            }
            (Some(mut a), Op::Subtract) => {
                for (x, y) in a.iter_mut().zip(&v) {
                    *x *= 1.0 - *y;
                }
                a
            }
            (Some(mut a), Op::Intersect) => {
                for (x, y) in a.iter_mut().zip(&v) {
                    *x *= *y;
                }
                a
            }
        });
    }
    let mut out = acc.unwrap_or_else(|| vec![0.0; n]);
    if m.invert {
        for x in out.iter_mut() {
            *x = 1.0 - *x;
        }
    }
    for x in out.iter_mut() {
        *x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
    }
    out
}

/// How much stronger (or weaker) a local slider is than the global one, below and above zero:
/// the global value that matched each local one best in Photoshop 2026 (100 % masks, ±50 and
/// ±1 EV, two photographs; Whites − and Blacks + were still stronger at twice).
const LOCAL_SCALE: [(&str, f32, f32); 9] = [
    ("contrast", 0.65, 0.75),
    ("highlights", 1.25, 1.25),
    ("shadows", 1.25, 1.25),
    ("whites", 2.0, 1.5),
    ("blacks", 1.5, 2.0),
    ("clarity", 0.75, 1.25),
    ("dehaze", 1.25, 1.25),
    ("saturation", 1.0, 0.4),
    ("texture", 1.0, 1.0),
];

fn scaled(name: &str, v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let (neg, pos) = LOCAL_SCALE.iter().find(|(n, ..)| *n == name).map_or((1.0, 1.0), |(_, a, b)| (*a, *b));
    (v * if v < 0.0 { neg } else { pos }).clamp(-100.0, 100.0)
}

/// The global settings that stand for a mask's local sliders.
pub fn as_global(l: &Local, base: &CameraRaw) -> CameraRaw {
    let f = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
    let mut hue = [0.0; 8];
    let h = f(l.hue, -100.0, 100.0);
    hue.iter_mut().for_each(|x| *x = h);
    CameraRaw {
        temperature: f(l.temperature, -100.0, 100.0),
        tint: f(l.tint, -100.0, 100.0),
        exposure: f(l.exposure, -4.0, 4.0),
        contrast: scaled("contrast", l.contrast),
        highlights: scaled("highlights", l.highlights),
        shadows: scaled("shadows", l.shadows),
        whites: scaled("whites", l.whites),
        blacks: scaled("blacks", l.blacks),
        texture: scaled("texture", l.texture),
        clarity: scaled("clarity", l.clarity),
        dehaze: scaled("dehaze", l.dehaze),
        saturation: scaled("saturation", l.saturation),
        hsl_hue: hue,
        sharpen_amount: f(l.sharpness, 0.0, 100.0) * 1.5,
        noise_luminance: f(-l.sharpness.min(0.0) + l.noise.max(0.0), 0.0, 100.0),
        pixel_scale: base.pixel_scale,
        encoding_gamma: base.encoding_gamma,
        seed: base.seed,
        ..CameraRaw::default()
    }
}

/// Applies every active mask of `p` to developed straight-RGBA pixels.
pub fn apply(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    let n = w.saturating_mul(h);
    if px.len() < n || n == 0 {
        return;
    }
    for m in p.masks.iter().take(MAX_MASKS).filter(|m| m.is_active()) {
        let cov = coverage(m, px, w, h);
        let local = as_global(&m.adjust, p);
        let a = &m.adjust;
        let extras = [a.color_sat, a.moire, a.defringe].iter().any(|v| v.is_finite() && *v > 0.0);
        if local.is_identity() && !extras {
            continue;
        }
        // Develop only the mask's box (plus a margin for the spatial stages' reach).
        let Some((x0, y0, x1, y1)) = bounds(&cov, w, h) else { continue };
        let pad = (w.max(h) / 16).max(8);
        let (x0, y0, x1, y1) = (x0.saturating_sub(pad), y0.saturating_sub(pad), (x1 + pad).min(w), (y1 + pad).min(h));
        let bw = x1 - x0;
        let mut dev: Vec<[f32; 4]> = Vec::with_capacity(bw * (y1 - y0));
        for y in y0..y1 {
            dev.extend_from_slice(&px[y * w + x0..y * w + x1]);
        }
        let bh = y1 - y0;
        super::develop(&mut dev, bw, bh, &local, float);
        if extras {
            extra::tint(&mut dev, bw, a.color_hue, a.color_sat);
            extra::moire(&mut dev, bw, bh, a.moire);
            extra::defringe(&mut dev, bw, bh, a.defringe);
        }
        let k = (m.amount / 100.0).clamp(0.0, 2.0);
        let enc = super::Encoding::of(p.encoding_gamma);
        for y in y0..y1 {
            for x in x0..x1 {
                let (i, j) = (y * w + x, (y - y0) * bw + (x - x0));
                // Photoshop weighs a local adjustment by coverage^1.2 and mixes it in exposure
                // terms (measured with local Exposure through linear and radial masks).
                let t = cov[i].powf(WEIGHT_POWER) * k;
                if t <= 0.0 {
                    continue;
                }
                let (Some(q), Some(d)) = (px.get(i).copied(), dev.get(j)) else { continue };
                let mut o = q;
                for ch in 0..3 {
                    let v = mix(q[ch], d[ch], t, enc);
                    o[ch] = if float { v } else { v.clamp(0.0, 1.0) };
                }
                px[i] = o;
            }
        }
    }
}

/// How a mask's coverage weighs its adjustment (measured: half coverage gives 0.43 of the
/// exposure change).
pub const WEIGHT_POWER: f32 = 1.2;

/// `a` moved `t` of the way to `b` in exposure terms (log of linear light); near black, where
/// logs run away, in encoded values.
fn mix(a: f32, b: f32, t: f32, enc: super::Encoding) -> f32 {
    let (la, lb) = (enc.decode(a), enc.decode(b));
    const FLOOR: f32 = 1e-4;
    if la > FLOOR && lb > FLOOR && la.is_finite() && lb.is_finite() {
        enc.encode((la.ln() + (lb.ln() - la.ln()) * t).exp())
    } else {
        a + (b - a) * t
    }
}

/// The box `[x0, x1) × [y0, y1)` where `cov` is on, or `None` when it is off everywhere.
fn bounds(cov: &[f32], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        let row = cov.get(y * w..(y + 1) * w)?;
        if let (Some(a), Some(b)) = (row.iter().position(|v| *v > 0.0), row.iter().rposition(|v| *v > 0.0)) {
            x0 = x0.min(a);
            x1 = x1.max(b + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

#[cfg(test)]
mod tests;
