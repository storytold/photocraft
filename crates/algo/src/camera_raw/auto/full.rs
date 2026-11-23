use super::{CameraRaw, Encoding, Image, Panel, luma, suggest};
use super::{curves_neutral, linear_to_srgb, srgb_to_linear};

const SIDE: usize = 192;
const HEADROOM: f32 = 0.995;
const SHADOW_FLOOR: f32 = 0.001;

/// Optimize the requested panels and, for SDR with a neutral curve, the composite tone
/// curve. The caller supplies the same develop stages it uses for the image preview.
pub fn suggest_full(
    image: &Image<'_>,
    panels: &[Panel],
    base: &CameraRaw,
    float: bool,
    render: impl Fn(&mut [[f32; 4]], usize, usize, &CameraRaw),
) -> CameraRaw {
    let seed = suggest(image, panels, base);
    // SDR headroom and a bounded point curve do not describe an HDR render.
    if float {
        return seed;
    }
    let sample = Sample::new(image);
    let enc = image.encoding;
    let source = sample.measure(&sample.pixels, enc);
    if source.count < 32 {
        return seed;
    }
    let target = Target::new(source, image.shadow_noise);
    let mut buffer = Vec::new();
    let mut evaluate = |params: &CameraRaw, strength: f32| {
        buffer.clone_from(&sample.pixels);
        render(&mut buffer, sample.width, sample.height, params);
        let rendered = sample.measure(&buffer, enc);
        let score = target.loss(rendered) + (1.0 - strength) * 0.016;
        Candidate { params: params.clone(), rendered, score, strength }
    };
    let mut best = evaluate(&seed, 1.0);
    if !panels.is_empty() {
        let without_light = panels.iter().copied().filter(|p| *p != Panel::Light).collect::<Vec<_>>();
        // A reduced RAW preview can bring highlights below white even when the corresponding
        // sensor develop still clips. Keep the initial recovery for a near-white area (at
        // least half a percent of the source) when trying less aggressive panel settings.
        let keep_highlights = panels.contains(&Panel::Light) && source.near_white >= 0.005;
        for (adjusted, weight) in [(&without_light[..], 0.60), (panels, 0.65), (panels, 0.30)] {
            let mut candidate = attenuate(&seed, base, adjusted, weight);
            if keep_highlights {
                candidate.highlights = candidate.highlights.min(seed.highlights);
            }
            if candidate != seed {
                best.keep(evaluate(&candidate, weight));
            }
        }
    }
    if panels.contains(&Panel::Light) && best.rendered.high > 0.001 {
        let mut protected = best.params.clone();
        protected.exposure = (protected.exposure - 0.20).max(-2.0);
        protected.whites = protected.whites.min(0.0);
        protected.highlights = protected.highlights.min(-10.0);
        best.keep(evaluate(&protected, best.strength));
    }
    if base.point_curve.is_empty() && curves_neutral(base) && source.span() > 0.08 {
        let start = best.params.clone();
        let observed = best.rendered;
        let strength = best.strength;
        for weight in [0.5, 1.0] {
            let mut with_curve = start.clone();
            if let Some(points) = target.curve(observed, enc, weight) {
                with_curve.point_curve = points;
                best.keep(evaluate(&with_curve, strength));
            }
        }
    }
    best.params
}

struct Candidate {
    params: CameraRaw,
    rendered: Distribution,
    score: f32,
    strength: f32,
}

impl Candidate {
    fn keep(&mut self, next: Self) {
        if next.score + 0.0005 < self.score {
            *self = next;
        }
    }
}

fn attenuate(seed: &CameraRaw, base: &CameraRaw, panels: &[Panel], weight: f32) -> CameraRaw {
    let mut p = seed.clone();
    macro_rules! blend {
        ($($field:ident),+) => {{ $(p.$field = base.$field * (1.0 - weight) + seed.$field * weight;)+ }};
    }
    for panel in panels {
        match panel {
            Panel::WhiteBalance => blend!(temperature, tint),
            Panel::Light => blend!(exposure, contrast, highlights, shadows, whites, blacks),
            Panel::Color => blend!(vibrance, saturation),
            Panel::Effects => blend!(texture, clarity, dehaze, grain_amount, vignette_amount),
        }
    }
    p
}

struct Sample {
    pixels: Vec<[f32; 4]>,
    covered: Vec<bool>,
    width: usize,
    height: usize,
}

impl Sample {
    fn new(image: &Image<'_>) -> Self {
        let step = image.w.max(image.h).div_ceil(SIDE);
        let width = image.w.div_ceil(step);
        let height = image.h.div_ceil(step);
        let mut pixels = Vec::with_capacity(width * height);
        let mut covered = Vec::with_capacity(width * height);
        for y in (0..image.h).step_by(step) {
            for x in (0..image.w).step_by(step) {
                let x = x.saturating_add(step / 2).min(image.w - 1);
                let y = y.saturating_add(step / 2).min(image.h - 1);
                let q = image.px[y * image.w + x];
                pixels.push(q.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
                covered.push(image.rgb(x, y).is_some());
            }
        }
        Self { pixels, covered, width, height }
    }

    fn measure(&self, rendered: &[[f32; 4]], enc: Encoding) -> Distribution {
        let mut light = Vec::with_capacity(rendered.len());
        let (mut high, mut low, mut near_white) = (0, 0, 0);
        for ((after, before), &covered) in rendered.iter().zip(&self.pixels).zip(&self.covered) {
            if !covered {
                continue;
            }
            let [r, g, b, _] = *after;
            let rgb = [r, g, b];
            near_white += usize::from(rgb.iter().any(|v| *v >= HEADROOM));
            high += usize::from(rgb.iter().zip(before).any(|(a, b)| *a >= HEADROOM && *b < HEADROOM));
            low += usize::from(rgb.iter().zip(before).any(|(a, b)| *a <= SHADOW_FLOOR && *b > SHADOW_FLOOR));
            light.push(linear_to_srgb(luma(rgb.map(|v| enc.decode(v.clamp(0.0, 1.0))))));
        }
        let count = light.len();
        light.sort_unstable_by(f32::total_cmp);
        let at = |fraction: f32| {
            let index = (count.saturating_sub(1) as f32 * fraction).round() as usize;
            light.get(index).copied().unwrap_or(0.0)
        };
        Distribution {
            low: low as f32 / count.max(1) as f32,
            high: high as f32 / count.max(1) as f32,
            near_white: near_white as f32 / count.max(1) as f32,
            shadow: at(0.10),
            middle: at(0.50),
            bright: at(0.90),
            count,
        }
    }
}

#[derive(Clone, Copy)]
struct Distribution {
    low: f32,
    high: f32,
    near_white: f32,
    shadow: f32,
    middle: f32,
    bright: f32,
    count: usize,
}

impl Distribution {
    fn span(self) -> f32 {
        self.bright - self.shadow
    }
}

struct Target {
    middle: f32,
    span: f32,
}

impl Target {
    fn new(source: Distribution, noise: f32) -> Self {
        // Keep low- and high-key intent. Reserve the strongest moves toward middle grey
        // for ordinary scenes with midtones and room on both sides of the histogram.
        let shift = ((0.46 - source.middle) * 0.48).clamp(-0.08, 0.09) * (1.0 - 0.65 * noise);
        let span = source.span();
        Self { middle: source.middle + shift, span: span + ((0.54 - span) * 0.32).clamp(0.0, 0.065) }
    }

    fn loss(&self, d: Distribution) -> f32 {
        12.0 * d.high + 8.0 * d.low + (d.middle - self.middle).abs() + 0.45 * (d.span() - self.span).abs()
    }

    fn curve(&self, d: Distribution, enc: Encoding, weight: f32) -> Option<Vec<[f32; 2]>> {
        let to_working = |v: f32| enc.encode(srgb_to_linear(v.clamp(0.0, 1.0)));
        let shift = (to_working(self.middle) - to_working(d.middle)).clamp(-0.045, 0.045);
        let spread = ((self.span - d.span()) * 0.45).clamp(-0.035, 0.035);
        let offsets = [shift * 0.65 - spread, shift, shift * 0.45 + spread].map(|v| (v * 255.0 * weight).round());
        if offsets == [0.0; 3] {
            return None;
        }
        // Anchored endpoints limit the adjustment to the inner tones. The bounded
        // offsets leave the points ordered in both input and output.
        Some(vec![[0.0, 0.0], [48.0, 48.0 + offsets[0]], [128.0, 128.0 + offsets[1]], [208.0, 208.0 + offsets[2]], [255.0, 255.0]])
    }
}
