//! Mixer Brush: the default reservoir/pickup model, plus an opt-in premultiplied wet-blending
//! color-state model (`MixerSettings::wet_blending`).
//!
//! Model (behavioural, our own): each dab samples the canvas colour under its footprint. The pickup
//! colour `P` moves towards it by `wet`. The deposited colour mixes the reservoir `R` (weight
//! `amount × (1 − mix·wet)`) with the pickup (weight `mix·wet`); it lands at `flow` times the total
//! weight. The reservoir amount drains per dab unless `load` is 100 %, so low Load strokes dry out.
//! A dry brush (`wet = 0`) paints only reservoir colour; an empty, wet brush smears canvas colour.

use photocraft_raster::{Surface, from_rgba, from_rgba_into, to_rgba};
use serde::{Deserialize, Serialize};

use crate::Stroke;
use crate::retouch::{Footprint, Region, alpha_index, apply_dab_stroke, over_native};
use photocraft_geom::Rect;

/// Fraction of the way the pickup colour moves towards the canvas colour per dab at 100 % Wet.
pub const PICKUP_RATE: f32 = 0.25;

/// Mixer Brush options (all 0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MixerSettings {
    /// Use the independent Blending / Dilution / Persistence color-state model instead of the
    /// reservoir model below. This is opt-in so existing Mixer Brush presets keep their meaning.
    pub wet_blending: bool,
    /// Current-color contribution (0..1; mapped linearly to `1 - value`).
    pub blending: f32,
    /// Sampled-opacity correction (0..1; mapped linearly to `1 - value`).
    pub dilution: f32,
    /// Retention of the previous mixed color (0..1; candidate interpolation is `1 - value`).
    pub persistence: f32,
    pub wet: f32,
    pub load: f32,
    pub mix: f32,
    pub flow: f32,
    /// Pick up colour from all visible layers instead of the target layer only.
    pub sample_all_layers: bool,
}

impl Default for MixerSettings {
    fn default() -> Self {
        Self { wet_blending: false, blending: 0.0, dilution: 0.0, persistence: 0.0, wet: 0.5, load: 0.5, mix: 0.5, flow: 1.0, sample_all_layers: false }
    }
}

/// Paint carried between strokes (session tool state).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MixerState {
    /// Loaded paint (straight RGBA), None = clean/unloaded.
    pub reservoir: Option<[f32; 4]>,
    /// Remaining reservoir amount 0..1.
    pub amount: f32,
    /// Colour picked up from the canvas (straight RGBA), None = clean.
    pub pickup: Option<[f32; 4]>,
}

impl MixerState {
    /// Load the reservoir with a colour (full).
    pub fn load(&mut self, color: [f32; 4]) {
        self.reservoir = Some(color);
        self.amount = 1.0;
    }
    /// Clean the brush (drop the picked-up colour).
    pub fn clean(&mut self) {
        self.pickup = None;
    }
}

/// Paint a Mixer Brush stroke. `sample` is the surface colour is picked up from (the layer itself
/// when None; a flattened composite for Sample All Layers). Returns the damaged rectangle.
pub fn apply_mixer_stroke(
    target: &mut Surface,
    sample: Option<&Surface>,
    stroke: &Stroke,
    m: &MixerSettings,
    state: &mut MixerState,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> Rect {
    if m.wet_blending {
        return apply_wet_blending_stroke(target, sample, stroke, m, selection, lock_transparency);
    }
    let fmt = target.format();
    let n = fmt.channels();
    let a = alpha_index(&fmt);
    let nc = fmt.mode.color_channels();
    let native = |c: [f32; 4]| {
        let mut v = from_rgba(&fmt, c);
        if let Some(ai) = a {
            v[ai] = 1.0;
        }
        v
    };
    let res = state.reservoir.map(native);
    let mut amount = if res.is_some() { state.amount.clamp(0.0, 1.0) } else { 0.0 };
    let mut pickup: Option<Vec<f32>> = state.pickup.map(native);
    let (wet, mix, flow, load) = (m.wet.clamp(0.0, 1.0), m.mix.clamp(0.0, 1.0), m.flow.clamp(0.0, 1.0), m.load.clamp(0.0, 1.0));
    let mut st = stroke.clone();
    st.brush.opacity = 1.0;
    let mut acc = vec![0.0f32; n];
    let mut paint = vec![0.0f32; n];
    let dmg = apply_dab_stroke(target, &st, selection, lock_transparency, 0, |work, fp| {
        // Transfer › Wetness / Mix Jitter scale the settings per dab.
        let (wet, mix) = (wet * fp.dab.wet.clamp(0.0, 1.0), mix * fp.dab.mix.clamp(0.0, 1.0));
        // Sample the canvas under the dab.
        acc.iter_mut().for_each(|v| *v = 0.0);
        let (mut wsum, mut asum) = (0.0f32, 0.0f32);
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let w = fp.at(x, y);
                if w <= 0.0 {
                    continue;
                }
                let px: Vec<f32> = match sample {
                    Some(s) => {
                        let v = s.pixel(x, y);
                        if s.format() == fmt { v } else { from_rgba(&fmt, s.rgba(x, y)) }
                    }
                    None => work.px(x, y).to_vec(),
                };
                let al = a.map_or(1.0, |ai| px[ai]);
                for c in 0..nc {
                    acc[c] += px[c] * w * al;
                }
                wsum += w;
                asum += w * al;
            }
        }
        if asum > 1e-6 && wet > 0.0 {
            let ca = asum / wsum.max(1e-6);
            let canvas: Vec<f32> = (0..n).map(|c| if c < nc { acc[c] / asum } else { 1.0 }).collect();
            pickup = Some(match pickup.take() {
                None => canvas,
                Some(p) => p.iter().zip(&canvas).map(|(p, c)| p + (c - p) * PICKUP_RATE * wet * ca).collect(),
            });
        }
        let wr = if res.is_some() { amount * (1.0 - mix * wet) } else { 0.0 };
        let wp = if pickup.is_some() { mix * wet } else { 0.0 };
        let total = wr + wp;
        if total > 1e-6 {
            for c in 0..n {
                let rv = res.as_ref().map_or(0.0, |v| v[c]);
                let pv = pickup.as_ref().map_or(0.0, |v| v[c]);
                paint[c] = (rv * wr + pv * wp) / total;
            }
            let dep = total.min(1.0) * flow;
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let k = fp.at(x, y) * dep;
                    if k > 0.0 {
                        over_native(&fmt, work.px_mut(x, y), &paint, k, lock_transparency);
                    }
                }
            }
        }
        amount *= 1.0 - 0.08 * (1.0 - load);
    });
    state.amount = amount;
    state.pickup = pickup.map(|p| to_rgba(&fmt, &p));
    dmg
}

/// Apply the documented premultiplied color-state recurrence as an opt-in Mixer Brush model.
/// The state is stroke-local and each generated dab samples the already-updated working surface.
/// Final pixel deposition remains PhotoCraft's normal source-over operation.
// Keep sample → candidate → persistence → deposition together: that order is the behavioral spec.
#[allow(clippy::too_many_lines)]
fn apply_wet_blending_stroke(
    target: &mut Surface,
    sample: Option<&Surface>,
    stroke: &Stroke,
    settings: &MixerSettings,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> Rect {
    let fmt = target.format();
    let mut wet = WetBlending::new(fmt, settings);
    let mut st = stroke.clone();
    st.brush.opacity = 1.0;
    apply_dab_stroke(target, &st, selection, lock_transparency, 0, |work, fp| wet.dab(work, fp, sample, lock_transparency))
}

/// Stroke-local Wet Blending color state. `dab` is public so live previews and the committed
/// whole-stroke renderer use the same sample, recurrence, and deposition path.
#[derive(Clone)]
pub struct WetBlending {
    fmt: photocraft_color::PixelFormat,
    nc: usize,
    alpha: Option<usize>,
    blend0: f32,
    dilution: f32,
    persistence: f32,
    flow: f32,
    carried: Option<Vec<f32>>,
    sampled: Vec<f32>,
    straight_sample: Vec<f32>,
    next: Vec<f32>,
    paint: Vec<f32>,
    converted_sample: Vec<f32>,
}

impl WetBlending {
    pub fn new(fmt: photocraft_color::PixelFormat, settings: &MixerSettings) -> Self {
        let nc = fmt.mode.color_channels();
        Self {
            fmt,
            nc,
            alpha: alpha_index(&fmt),
            blend0: 1.0 - settings.blending.clamp(0.0, 1.0),
            dilution: 1.0 - settings.dilution.clamp(0.0, 1.0),
            persistence: 1.0 - settings.persistence.clamp(0.0, 1.0),
            flow: settings.flow.clamp(0.0, 1.0),
            carried: None,
            sampled: vec![0.0; nc],
            straight_sample: vec![0.0; nc],
            next: vec![0.0; nc + 1],
            paint: vec![0.0; fmt.channels()],
            converted_sample: vec![0.0; fmt.channels()],
        }
    }

    pub fn dab(&mut self, work: &mut Region, fp: &Footprint, sample: Option<&Surface>, lock_transparency: bool) {
        let mut source_native = from_rgba(&self.fmt, fp.dab.color);
        if let Some(a) = self.alpha
            && let Some(ch) = source_native.get_mut(a)
        {
            *ch = fp.dab.color[3].clamp(0.0, 1.0);
        }
        let r = fp.rect.intersect(&work.rect);
        if r.is_empty() {
            return;
        }
        let sample_data = sample.map(|s| s.read_region(r));
        let sample_channels = sample.map_or(0, Surface::channels);
        let (mut weight_sum, mut alpha_sum) = (0.0f32, 0.0f32);
        self.sampled.fill(0.0);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let w = fp.at(x, y);
                if w <= 0.0 {
                    continue;
                }
                let px: &[f32] = if let (Some(s), Some(data)) = (sample, sample_data.as_ref()) {
                    let i = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * sample_channels;
                    let Some(channels) = data.get(i..i + sample_channels) else { continue };
                    if s.format() == self.fmt {
                        channels
                    } else {
                        let rgba = to_rgba(&s.format(), channels);
                        from_rgba_into(&self.fmt, rgba, &mut self.converted_sample);
                        &self.converted_sample
                    }
                } else {
                    work.px(x, y)
                };
                let a = self.alpha.and_then(|i| px.get(i).copied()).unwrap_or(1.0).clamp(0.0, 1.0);
                for (sum, value) in self.sampled.iter_mut().zip(px).take(self.nc) {
                    *sum += value * a * w;
                }
                weight_sum += w;
                alpha_sum += a * w;
            }
        }
        if weight_sum <= f32::EPSILON {
            return;
        }
        let sample_alpha = (alpha_sum / weight_sum).clamp(0.0, 1.0);
        self.straight_sample.fill(0.0);
        if sample_alpha >= 1.0 / 128.0 && alpha_sum > f32::EPSILON {
            for (straight, sum) in self.straight_sample.iter_mut().zip(&self.sampled) {
                *straight = sum / alpha_sum;
            }
        } else {
            self.straight_sample.copy_from_slice(&source_native[..self.nc]);
        }

        // Dab transfer controls apply to the current-color contribution per update. The recovered
        // pressure response and tool-specific overrides are intentionally not inferred here.
        let blend = self.blend0 * fp.dab.mix.clamp(0.0, 1.0);
        let mixed_alpha = sample_alpha + (1.0 - sample_alpha) * self.dilution;
        let source_alpha = self.alpha.and_then(|i| source_native.get(i).copied()).unwrap_or(1.0).clamp(0.0, 1.0) * fp.dab.opacity.clamp(0.0, 1.0);
        self.next.fill(0.0);
        for (c, dst) in self.next.iter_mut().take(self.nc).enumerate() {
            let sampled_premul = self.sampled[c] / weight_sum;
            let diluted = sampled_premul + (self.straight_sample[c] - sampled_premul) * self.dilution;
            let source_premul = mixed_alpha * source_alpha * source_native.get(c).copied().unwrap_or(0.0);
            *dst = diluted + (source_premul - diluted) * blend;
        }
        self.next[self.nc] = mixed_alpha + (mixed_alpha * source_alpha - mixed_alpha) * blend;
        if let Some(previous) = self.carried.as_ref() {
            for (dst, prev) in self.next.iter_mut().zip(previous) {
                *dst = *prev + (*dst - *prev) * self.persistence;
            }
            if let Some(state) = self.carried.as_mut() {
                state.copy_from_slice(&self.next);
            }
        } else {
            self.carried = Some(self.next.clone());
        }
        let Some(carried_color) = self.carried.as_ref() else { return };
        let carried_alpha = carried_color[self.nc].clamp(0.0, 1.0);
        self.paint.fill(0.0);
        if carried_alpha > f32::EPSILON {
            for (dst, src) in self.paint.iter_mut().zip(carried_color).take(self.nc) {
                *dst = src / carried_alpha;
            }
        } else {
            self.paint[..self.nc].copy_from_slice(&self.straight_sample);
        }
        if let Some(a) = self.alpha {
            self.paint[a] = carried_alpha;
        }
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y) * self.flow;
                if k > 0.0 {
                    over_native(&self.fmt, work.px_mut(x, y), &self.paint, k, lock_transparency);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_color::PixelFormat;

    fn stroke(y: f64) -> Stroke {
        Stroke {
            brush: BrushSettings { size: 8.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: vec![StrokePoint::new(4.0, y, 1.0), StrokePoint::new(196.0, y, 1.0)],
        }
    }

    #[test]
    fn low_load_runs_dry_and_full_load_does_not() {
        for (load, expect_dry) in [(0.0, true), (1.0, false)] {
            let mut s = Surface::new(PixelFormat::RGBA8);
            let mut st = MixerState::default();
            st.load([1.0, 0.0, 0.0, 1.0]);
            apply_mixer_stroke(&mut s, None, &stroke(10.0), &MixerSettings { wet: 0.0, load, mix: 0.0, flow: 1.0, ..Default::default() }, &mut st, None, false);
            let (start, end) = (s.rgba(10, 10)[3], s.rgba(190, 10)[3]);
            assert!(start > 0.95, "{start}");
            if expect_dry {
                assert!(end < start * 0.6, "load {load}: {start} → {end}");
                assert!(st.amount < 0.2);
            } else {
                assert!(end > 0.95, "{end}");
                assert!((st.amount - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn wet_brush_picks_up_and_smears_canvas_colour() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        // Blue canvas on the left half, white on the right.
        s.fill_rect(Rect::new(0, 0, 100, 20), &[0.0, 0.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(100, 0, 200, 20), &[1.0, 1.0, 1.0, 1.0]);
        let mut st = MixerState::default();
        st.load([1.0, 0.0, 0.0, 1.0]);
        apply_mixer_stroke(
            &mut s,
            None,
            &stroke(10.0),
            &MixerSettings { wet: 1.0, load: 0.3, mix: 0.9, flow: 1.0, ..Default::default() },
            &mut st,
            None,
            false,
        );
        // Blue is dragged into the white half.
        let c = s.rgba(115, 10);
        assert!(c[2] > c[1] + 0.05 && c[1] < 0.95, "{c:?}");
        // The pickup well now holds a canvas-contaminated colour.
        let p = st.pickup.unwrap();
        assert!(p[2] > 0.2, "{p:?}");
        st.clean();
        assert!(st.pickup.is_none());
    }

    #[test]
    fn dry_empty_brush_paints_nothing() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let mut st = MixerState::default();
        let dmg = apply_mixer_stroke(&mut s, None, &stroke(10.0), &MixerSettings { wet: 0.0, ..Default::default() }, &mut st, None, false);
        assert!(!dmg.is_empty());
        assert_eq!(s.rgba(50, 10)[3], 0.0);
    }

    #[test]
    fn transfer_wetness_control_scales_pickup_per_dab() {
        // Wetness on Pen Pressure: no pressure means a dry brush (no pickup), full pressure picks up.
        let run = |pressure: f32| {
            let mut s = Surface::new(PixelFormat::RGBA8);
            s.fill_rect(Rect::new(0, 0, 200, 20), &[0.0, 0.0, 1.0, 1.0]);
            let mut st = MixerState::default();
            st.load([1.0, 0.0, 0.0, 1.0]);
            let mut k = stroke(10.0);
            k.brush.transfer = crate::Transfer { enabled: true, wetness: crate::Dynamic::controlled(crate::Control::PenPressure), ..Default::default() };
            for p in &mut k.points {
                p.pressure = pressure;
            }
            apply_mixer_stroke(&mut s, None, &k, &MixerSettings { wet: 1.0, load: 1.0, mix: 1.0, flow: 1.0, ..Default::default() }, &mut st, None, false);
            st.pickup
        };
        assert!(run(0.0).is_none());
        assert!(run(1.0).is_some_and(|p| p[2] > 0.2));
    }

    #[test]
    fn wet_blending_slider_endpoints_separate_source_color_and_opacity() {
        let make_stroke = || {
            let mut s = stroke(10.0);
            s.brush.color = [1.0, 0.0, 0.0, 1.0];
            s.points.truncate(1);
            s
        };
        let settings = |dilution| MixerSettings { wet_blending: true, blending: 0.0, dilution, flow: 1.0, ..Default::default() };

        let mut undiluted = Surface::new(PixelFormat::RGBA8);
        let mut state = MixerState::default();
        apply_mixer_stroke(&mut undiluted, None, &make_stroke(), &settings(0.0), &mut state, None, false);
        assert!(undiluted.rgba(4, 10)[3] > 0.95, "zero Dilution should retain opaque paint: {:?}", undiluted.rgba(4, 10));

        let mut diluted = Surface::new(PixelFormat::RGBA8);
        apply_mixer_stroke(&mut diluted, None, &make_stroke(), &settings(1.0), &mut state, None, false);
        assert!(diluted.rgba(4, 10)[3] < 0.05, "full Dilution should follow transparent sample alpha: {:?}", diluted.rgba(4, 10));
    }

    #[test]
    fn wet_blending_keeps_straight_source_color_when_paint_alpha_changes() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let tolerance = match fmt {
                PixelFormat::RGBA8 => 1.0 / 255.0 + 1e-4,
                PixelFormat::RGBA16 => 1.0 / 65535.0 + 1e-4,
                _ => 1e-5,
            };
            for (paint_alpha, pressure, expected_alpha) in [(0.25, 1.0, 0.25), (1.0, 0.5, 0.5), (0.25, 0.5, 0.125)] {
                let mut target = Surface::new(fmt);
                let mut k = stroke(10.0);
                k.brush.color = [0.2, 0.2, 0.2, paint_alpha];
                k.points.truncate(1);
                k.points[0].pressure = pressure;
                if pressure < 1.0 {
                    k.brush.transfer =
                        crate::Transfer { enabled: true, opacity: crate::Dynamic::controlled(crate::Control::PenPressure), ..Default::default() };
                }
                let settings = MixerSettings { wet_blending: true, blending: 0.0, dilution: 0.0, persistence: 0.0, flow: 1.0, ..Default::default() };
                apply_mixer_stroke(&mut target, None, &k, &settings, &mut MixerState::default(), None, false);
                let pixel = target.rgba(4, 10);
                assert!((pixel[0] - 0.2).abs() <= tolerance, "{fmt:?}, alpha={paint_alpha}, pressure={pressure}: {pixel:?}");
                assert!((pixel[1] - 0.2).abs() <= tolerance, "{fmt:?}, alpha={paint_alpha}, pressure={pressure}: {pixel:?}");
                assert!((pixel[2] - 0.2).abs() <= tolerance, "{fmt:?}, alpha={paint_alpha}, pressure={pressure}: {pixel:?}");
                assert!((pixel[3] - expected_alpha).abs() <= tolerance, "{fmt:?}, alpha={paint_alpha}, pressure={pressure}: {pixel:?}");
            }
        }
    }

    #[test]
    fn wet_blending_blending_endpoint_selects_source_or_sample_color() {
        for (blending, expect_red) in [(0.0, true), (1.0, false)] {
            let mut s = Surface::new(PixelFormat::RGBA8);
            s.fill_rect(Rect::new(0, 0, 200, 20), &[0.0, 0.0, 1.0, 1.0]);
            let mut k = stroke(10.0);
            k.brush.color = [1.0, 0.0, 0.0, 1.0];
            let mut state = MixerState::default();
            let m = MixerSettings { wet_blending: true, blending, dilution: 0.0, flow: 1.0, ..Default::default() };
            apply_mixer_stroke(&mut s, None, &k, &m, &mut state, None, false);
            let pixel = s.rgba(90, 10);
            assert_eq!(pixel[0] > pixel[2], expect_red, "Blending {blending}: {pixel:?}");
        }
    }

    #[test]
    fn wet_blending_persistence_retains_the_first_mixture_across_dabs() {
        let mut sample = Surface::new(PixelFormat::RGBA8);
        sample.fill_rect(Rect::new(0, 0, 100, 20), &[0.0, 0.0, 1.0, 1.0]);
        sample.fill_rect(Rect::new(100, 0, 200, 20), &[1.0, 1.0, 1.0, 1.0]);
        let run = |persistence| {
            let mut target = Surface::new(PixelFormat::RGBA8);
            let mut state = MixerState::default();
            let settings = MixerSettings { wet_blending: true, blending: 1.0, dilution: 0.0, persistence, flow: 1.0, ..Default::default() };
            apply_mixer_stroke(&mut target, Some(&sample), &stroke(10.0), &settings, &mut state, None, false);
            target.rgba(160, 10)
        };
        let reset_each_dab = run(0.0);
        let retain_first = run(1.0);
        assert!(reset_each_dab[..3].iter().all(|v| *v > 0.9), "no persistence follows the white sample: {reset_each_dab:?}");
        assert!(retain_first[2] > 0.9 && retain_first[0] < 0.1, "full persistence retains the first blue mixture: {retain_first:?}");
    }
}
