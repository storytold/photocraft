//! Image-dependent starting points for RAW Develop. Each panel owns its slider values.

mod effects;
mod full;
mod noise;

use photocraft_color::convert::{linear_to_srgb, srgb_to_linear};

use super::{CameraRaw, auto_tone, white_balance_gains};

#[derive(Clone, Copy)]
struct Encoding(f32);

impl Encoding {
    fn of(gamma: f32) -> Self {
        Self(if gamma.is_finite() && gamma > 0.0 { gamma.clamp(0.5, 4.0) } else { 0.0 })
    }

    fn decode(self, v: f32) -> f32 {
        if self.0 > 0.0 { v.max(0.0).powf(self.0) } else { srgb_to_linear(v) }
    }

    fn encode(self, v: f32) -> f32 {
        if self.0 > 0.0 { v.max(0.0).powf(1.0 / self.0) } else { linear_to_srgb(v) }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn curves_neutral(p: &CameraRaw) -> bool {
    [p.curve_highlights, p.curve_lights, p.curve_darks, p.curve_shadows].iter().all(|v| *v == 0.0)
        && [&p.point_curve, &p.point_curve_red, &p.point_curve_green, &p.point_curve_blue]
            .iter()
            .all(|c| c.len() < 2 || c.iter().all(|p| (p[0] - p[1]).abs() < 1e-3))
}
pub use full::suggest_full;
pub use noise::measure_shadow_noise;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Panel {
    WhiteBalance,
    Light,
    Color,
    Effects,
}

impl Panel {
    /// Light and Color can use the suggested white balance when all panels are requested.
    pub const ALL: [Self; 4] = [Self::WhiteBalance, Self::Light, Self::Color, Self::Effects];

    pub const fn id(self) -> &'static str {
        match self {
            Self::WhiteBalance => "whiteBalance",
            Self::Light => "light",
            Self::Color => "color",
            Self::Effects => "effects",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.id() == id)
    }
}

/// A bounded sampling domain. Samples outside the filter mask, transparent pixels and
/// non-finite values supply no evidence to the estimators.
pub struct Image<'a> {
    px: &'a [[f32; 4]],
    coverage: Option<&'a [f32]>,
    w: usize,
    h: usize,
    shadow_noise: f32,
    encoding: Encoding,
}

impl<'a> Image<'a> {
    pub fn new(px: &'a [[f32; 4]], w: usize, h: usize, coverage: Option<&'a [f32]>) -> Option<Self> {
        if w == 0 || h == 0 || w.checked_mul(h) != Some(px.len()) || coverage.is_some_and(|c| c.len() != px.len()) {
            return None;
        }
        Some(Self { px, coverage, w, h, shadow_noise: 0.0, encoding: Encoding::of(0.0) })
    }

    /// The source document's working transfer curve.
    pub fn with_encoding_gamma(mut self, gamma: f32) -> Self {
        self.encoding = Encoding::of(gamma);
        self
    }

    /// Use the noise level measured before reducing the image to preview size.
    pub fn with_shadow_noise(mut self, amplitude: f32) -> Self {
        if amplitude.is_finite() {
            self.shadow_noise = ((amplitude - 0.008) / 0.008).clamp(0.0, 1.0);
        }
        self
    }

    fn rgb(&self, x: usize, y: usize) -> Option<[f32; 3]> {
        let i = y.checked_mul(self.w)?.checked_add(x)?;
        let [r, g, b, a] = *self.px.get(i)?;
        let rgb = [r, g, b];
        let coverage = self.coverage.and_then(|c| c.get(i)).copied().unwrap_or(1.0);
        (x < self.w && a.is_finite() && coverage.is_finite() && a * coverage >= 0.5 && rgb.iter().all(|v| v.is_finite())).then_some(rgb)
    }

    fn samples(&self, per_side: usize) -> impl Iterator<Item = (usize, usize, [f32; 3])> + '_ {
        (0..self.h)
            .step_by(self.h.div_ceil(per_side))
            .flat_map(move |y| (0..self.w).step_by(self.w.div_ceil(per_side)).filter_map(move |x| self.rgb(x, y).map(|rgb| (x, y, rgb))))
    }
}

/// Compute the requested panel values from the source. The same values can be inspected or replayed.
pub fn suggest(image: &Image<'_>, panels: &[Panel], base: &CameraRaw) -> CameraRaw {
    let mut out = base.clone();
    if panels.is_empty() || image.samples(96).next().is_none() {
        return out;
    }
    for panel in Panel::ALL.into_iter().filter(|p| panels.contains(p)) {
        match panel {
            Panel::WhiteBalance => {
                if let Some([temperature, tint]) = white_balance(image, image.encoding) {
                    out.temperature = temperature;
                    out.tint = tint;
                }
            }
            Panel::Light => light(image, &mut out),
            Panel::Color => color(image, &mut out),
            Panel::Effects => effects::suggest(image, &mut out),
        }
    }
    out
}

fn percentile(values: &mut [f32], fraction: f32) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let index = (values.len().saturating_sub(1) as f32 * fraction).round() as usize;
    values.get(index).copied().unwrap_or(0.0)
}

fn light(image: &Image<'_>, out: &mut CameraRaw) {
    let enc = image.encoding;
    let gains = white_balance_gains(out.temperature, out.tint);
    let norm = luma(gains);
    let mut luminance = Vec::new();
    let mut peaks = Vec::new();
    // auto_tone uses sRGB-like luminance for its midgrey target. Use the working
    // transfer curve for the linear-light conversion and reserve headroom per channel.
    for (_, _, rgb) in image.samples(96) {
        let linear = [0, 1, 2].map(|c| enc.decode(rgb[c].clamp(0.0, 1.0)) * gains[c] / norm);
        let v = linear_to_srgb(luma(linear).max(0.0));
        luminance.push([v, v, v, 1.0]);
        peaks.push(linear[0].max(linear[1]).max(linear[2]));
    }
    auto_tone(&luminance, out);
    // The median alone can brighten a dark scene dramatically even if its bright
    // areas already touch white. Leave the global exposure down and lift shadows
    // instead. A high percentile keeps individual hot pixels from setting the limit.
    let peak = percentile(&mut peaks, 0.995);
    let room = (enc.decode(0.95) / peak.max(1e-4)).log2().max(0.0);
    if out.exposure > room {
        let extra = out.exposure - room;
        out.exposure = (room * 100.0).floor() / 100.0;
        out.whites = out.whites.min(0.0);
        out.highlights = out.highlights.min(-(extra * 28.0).round().min(24.0));
    }
    // Brightening also exposes the source noise that the downsampled histogram hides.
    if out.exposure > 0.0 && image.shadow_noise > 0.0 {
        out.exposure = (out.exposure * (1.0 - 0.55 * image.shadow_noise) * 100.0).floor() / 100.0;
    }
    out.shadows = (out.shadows * (1.0 - 0.50 * image.shadow_noise)).round();
}

fn white_balance(image: &Image<'_>, enc: Encoding) -> Option<[f32; 2]> {
    // Estimate two chromatic ratios in separate regions. A brightly colored object or
    // a clipped highlight should not determine the balance for the entire frame.
    let mut regions = [[0.0f32; 4]; 16];
    let mut count = 0;
    for (x, y, rgb) in image.samples(96) {
        count += 1;
        let [r, g, b] = rgb;
        let hi = r.max(g).max(b);
        let lo = r.min(g).min(b);
        if lo < 0.16 || !(0.32..0.93).contains(&hi) || hi - lo > 0.26 * hi {
            continue;
        }
        // Prefer brighter, less chromatic anchors. Counting all candidates equally
        // treats a large patch of gently colored foliage as an illuminant.
        let chroma = (hi - lo) / hi;
        let weight = (1.0 - chroma / 0.30).powi(3) * hi.powi(2);
        let [r, g, b] = rgb.map(|v| enc.decode(v));
        let index = (y / image.h.div_ceil(4)) * 4 + x / image.w.div_ceil(4);
        if let Some(region) = regions.get_mut(index) {
            region[0] += weight * (r / g).ln();
            region[1] += weight * (b / g).ln();
            region[2] += weight;
            region[3] += 1.0;
        }
    }
    let supported: f32 = regions.iter().map(|r| r[3]).sum();
    if supported < (count as f32 * 0.02).max(12.0) {
        return None;
    }
    let strongest = regions.iter().map(|r| r[2]).fold(0.0, f32::max);
    let usable = |r: &&[f32; 4]| r[3] >= 3.0 && r[2] > (strongest * 0.07).max(0.10);
    let mut rs: Vec<_> = regions.iter().filter(usable).map(|r| r[0] / r[2]).collect();
    let mut bs: Vec<_> = regions.iter().filter(usable).map(|r| r[1] / r[2]).collect();
    if rs.len() < 4 {
        return None;
    }
    let (r, b) = (percentile(&mut rs, 0.5), percentile(&mut bs, 0.5));
    let disagreement = (percentile(&mut rs, 0.8) - percentile(&mut rs, 0.2)).max(percentile(&mut bs, 0.8) - percentile(&mut bs, 0.2));
    let confidence = (1.0 - disagreement / 0.35).clamp(0.0, 1.0);
    if confidence < 0.25 {
        return None;
    }
    let (r, b) = (r.exp(), b.exp());
    let [red_gain, green_gain, _] = white_balance_gains(100.0, 100.0);
    let temperature = (b - r) / (r + b) / (red_gain - 1.0);
    let tint = (1.0 - 2.0 * r * b / (r + b)) / (1.0 - green_gain);
    let correction = [temperature, tint].map(|v| (v * 65.0 * confidence).clamp(-35.0, 35.0).round());
    // Preserve the as-shot adjustment when the inferred cast is very small.
    (correction.iter().any(|v| v.abs() >= 5.0)).then_some(correction)
}

fn color(image: &Image<'_>, p: &mut CameraRaw) {
    let enc = image.encoding;
    let gains = white_balance_gains(p.temperature, p.tint);
    let exposure = p.exposure.exp2() / luma(gains);
    let mut chroma = Vec::new();
    let mut visible = 0;
    for (_, _, rgb) in image.samples(96) {
        let rgb = [0, 1, 2].map(|c| enc.encode(enc.decode(rgb[c].clamp(0.0, 1.0)) * gains[c] * exposure));
        let hi = rgb[0].max(rgb[1]).max(rgb[2]);
        let lo = rgb[0].min(rgb[1]).min(rgb[2]);
        if !(0.15..0.98).contains(&hi) {
            continue;
        }
        visible += 1;
        // Relative saturation in very dark/grayscale areas would drive an unwanted boost.
        let s = (hi - lo) / hi;
        if s > 0.07 {
            chroma.push(s);
        }
    }
    if chroma.len() < 16 || chroma.len() * 8 < visible {
        return;
    }
    let median = percentile(&mut chroma, 0.50);
    let upper = percentile(&mut chroma, 0.90);
    let headroom = (1.0 - upper / 0.85).max(0.0);
    p.vibrance = ((0.50 - median).max(0.0) * 68.0 * headroom).clamp(0.0, 20.0).round();
    p.saturation = (-(upper - 0.86).max(0.0) * 50.0).clamp(-7.0, 0.0).round();
}

#[cfg(test)]
mod tests;
