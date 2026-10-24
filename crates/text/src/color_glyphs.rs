//! Colour glyphs: COLR v0 layers and v1 paint graphs, coloured from the font's CPAL palette.
//! Emoji and other colour fonts draw in their own colours (#7711); the text colour fills only
//! what the font leaves to it (palette entry 0xFFFF), and its opacity applies to the whole glyph.
//!
//! skrifa walks the paint graph and calls a [`ColorPainter`]; [`Painter`] paints into a window of
//! the text raster in premultiplied sRGB, [`OutlineBounds`] only collects where it would paint.

use photocraft_color::blend::{BlendMode, blend_rgb};
use skrifa::GlyphId;
use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend, Transform};
use skrifa::raw::types::BoundingBox;

use crate::raster::{Coverage, LineSink, Xform, rect_to};

/// Palette entry standing for the text colour.
const FOREGROUND: u16 = 0xFFFF;
/// Layers a glyph may stack (each is a window-sized buffer); deeper ones paint straight into the
/// layer below. skrifa itself stops at 64 nested paints.
const MAX_LAYERS: usize = 8;

fn xform(t: &Transform) -> Xform {
    Xform([t.xx as f64, t.yx as f64, t.xy as f64, t.yy as f64, t.dx as f64, t.dy as f64])
}

/// Collects the outlines a colour glyph paints within (its clip glyphs and clip boxes) into a
/// [`LineSink`], for the ink bounds. `outline` draws a glyph (font units, through the given paint
/// transform) into the sink; `to_sink` maps font units to the sink for clip boxes.
pub(crate) struct OutlineBounds<'a, S: LineSink, F: FnMut(GlyphId, &Xform, &mut S)> {
    pub sink: &'a mut S,
    pub outline: F,
    pub to_sink: Xform,
    xforms: Vec<Xform>,
}

impl<'a, S: LineSink, F: FnMut(GlyphId, &Xform, &mut S)> OutlineBounds<'a, S, F> {
    pub fn new(sink: &'a mut S, outline: F, to_sink: Xform) -> Self {
        Self { sink, outline, to_sink, xforms: Vec::new() }
    }
    fn current(&self) -> Xform {
        self.xforms.last().copied().unwrap_or(Xform::IDENTITY)
    }
}

impl<S: LineSink, F: FnMut(GlyphId, &Xform, &mut S)> ColorPainter for OutlineBounds<'_, S, F> {
    fn push_transform(&mut self, t: Transform) {
        self.xforms.push(self.current().mul(&xform(&t)));
    }
    fn pop_transform(&mut self) {
        self.xforms.pop();
    }
    fn push_clip_glyph(&mut self, id: GlyphId) {
        let m = self.current();
        (self.outline)(id, &m, self.sink);
    }
    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        let m = self.to_sink.mul(&self.current());
        rect_to(self.sink, &m, b.x_min as f64, b.y_min as f64, b.x_max as f64, b.y_max as f64);
    }
    fn pop_clip(&mut self) {}
    fn fill(&mut self, _: Brush<'_>) {}
    fn push_layer(&mut self, _: CompositeMode) {}
}

/// A gradient's colour line: premultiplied stops in offset order.
struct ColorLine {
    stops: Vec<(f32, [f32; 4])>,
    extend: Extend,
}

impl ColorLine {
    fn at(&self, t: f32) -> [f32; 4] {
        let (Some(first), Some(last)) = (self.stops.first(), self.stops.last()) else { return [0.0; 4] };
        // Repeat and Reflect run over the stops' own span, which COLR doesn't pin to 0..1; Pad is
        // the first/last stop beyond it.
        let (t0, span) = (first.0, last.0 - first.0);
        let t = match self.extend {
            Extend::Repeat if span > 0.0 => t0 + (t - t0).rem_euclid(span),
            Extend::Reflect if span > 0.0 => {
                let m = (t - t0).rem_euclid(2.0 * span);
                t0 + if m > span { 2.0 * span - m } else { m }
            }
            _ => t,
        };
        if t.is_nan() || t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        for pair in self.stops.windows(2) {
            let [(o0, c0), (o1, c1)] = [pair[0], pair[1]];
            if t <= o1 {
                let k = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
                return std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * k);
            }
        }
        last.1
    }
}

/// What a fill paints, resolved to premultiplied colours.
enum Paint {
    Solid([f32; 4]),
    Linear { p0: (f32, f32), d: (f32, f32), len2: f32, line: ColorLine },
    Radial { c0: (f32, f32), r0: f32, cd: (f32, f32), dr: f32, line: ColorLine },
    Sweep { c: (f32, f32), start: f32, end: f32, line: ColorLine },
}

impl Paint {
    /// The colour at a point in paint space (font units); `None` where a radial gradient is
    /// undefined (the point lies outside every circle of the cone).
    fn at(&self, u: f32, v: f32) -> Option<[f32; 4]> {
        match self {
            Paint::Solid(c) => Some(*c),
            Paint::Linear { p0, d, len2, line } => Some(line.at(((u - p0.0) * d.0 + (v - p0.1) * d.1) / len2)),
            Paint::Radial { c0, r0, cd, dr, line } => {
                // The largest t with |p - c(t)| = r(t) and r(t) >= 0 (two-point conical gradient).
                let (px, py) = (u - c0.0, v - c0.1);
                let a = cd.0 * cd.0 + cd.1 * cd.1 - dr * dr;
                let b = px * cd.0 + py * cd.1 + r0 * dr;
                let c = px * px + py * py - r0 * r0;
                let ok = |t: f32| t.is_finite() && r0 + t * dr >= 0.0;
                if a.abs() < 1e-6 {
                    let t = c / (2.0 * b);
                    return ok(t).then(|| line.at(t));
                }
                let disc = b * b - a * c;
                if disc < 0.0 {
                    return None;
                }
                let s = disc.sqrt();
                let (t1, t2) = ((b + s) / a, (b - s) / a);
                let (hi, lo) = if t1 >= t2 { (t1, t2) } else { (t2, t1) };
                [hi, lo].into_iter().find(|t| ok(*t)).map(|t| line.at(t))
            }
            Paint::Sweep { c, start, end, line } => {
                // skrifa hands the angles clockwise (360° minus the font's counter-clockwise ones).
                let ccw = (v - c.1).atan2(u - c.0).to_degrees();
                let cw = (360.0 - ccw).rem_euclid(360.0);
                let span = end - start;
                let t = if span.abs() > 1e-6 {
                    (cw - start) / span
                } else if cw >= *start {
                    1.0
                } else {
                    0.0
                };
                Some(line.at(t))
            }
        }
    }
}

/// Premultiplied Porter-Duff or blend-mode composite of `s` over `b`.
fn composite(mode: CompositeMode, b: [f32; 4], s: [f32; 4]) -> [f32; 4] {
    let (ab, as_) = (b[3], s[3]);
    let blend = match mode {
        CompositeMode::Screen => Some(BlendMode::Screen),
        CompositeMode::Overlay => Some(BlendMode::Overlay),
        CompositeMode::Darken => Some(BlendMode::Darken),
        CompositeMode::Lighten => Some(BlendMode::Lighten),
        CompositeMode::ColorDodge => Some(BlendMode::ColorDodge),
        CompositeMode::ColorBurn => Some(BlendMode::ColorBurn),
        CompositeMode::HardLight => Some(BlendMode::HardLight),
        CompositeMode::SoftLight => Some(BlendMode::SoftLight),
        CompositeMode::Difference => Some(BlendMode::Difference),
        CompositeMode::Exclusion => Some(BlendMode::Exclusion),
        CompositeMode::Multiply => Some(BlendMode::Multiply),
        CompositeMode::HslHue => Some(BlendMode::Hue),
        CompositeMode::HslSaturation => Some(BlendMode::Saturation),
        CompositeMode::HslColor => Some(BlendMode::Color),
        CompositeMode::HslLuminosity => Some(BlendMode::Luminosity),
        _ => None,
    };
    if let Some(m) = blend {
        if ab <= 0.0 || as_ <= 0.0 {
            return std::array::from_fn(|i| s[i] + b[i] * (1.0 - as_));
        }
        let mixed = blend_rgb(m, [b[0] / ab, b[1] / ab, b[2] / ab], [s[0] / as_, s[1] / as_, s[2] / as_]);
        let mut out = [0.0; 4];
        for i in 0..3 {
            out[i] = s[i] * (1.0 - ab) + b[i] * (1.0 - as_) + as_ * ab * mixed[i];
        }
        out[3] = as_ + ab - as_ * ab;
        return out;
    }
    let (fs, fb) = match mode {
        CompositeMode::Clear => (0.0, 0.0),
        CompositeMode::Src => (1.0, 0.0),
        CompositeMode::Dest => (0.0, 1.0),
        CompositeMode::DestOver => (1.0 - ab, 1.0),
        CompositeMode::SrcIn => (ab, 0.0),
        CompositeMode::DestIn => (0.0, as_),
        CompositeMode::SrcOut => (1.0 - ab, 0.0),
        CompositeMode::DestOut => (0.0, 1.0 - as_),
        CompositeMode::SrcAtop => (ab, 1.0 - as_),
        CompositeMode::DestAtop => (1.0 - ab, as_),
        CompositeMode::Xor => (1.0 - ab, 1.0 - as_),
        CompositeMode::Plus => (1.0, 1.0),
        _ => (1.0, 1.0 - as_),
    };
    std::array::from_fn(|i| (s[i] * fs + b[i] * fb).min(1.0))
}

/// Paints one colour glyph into a `w × h` window, premultiplied sRGB. `outline` adds a glyph's
/// coverage (font units through the given paint transform) to a window-sized [`Coverage`];
/// `to_window` maps font units into the window (gradients and clip boxes).
pub(crate) struct Painter<'a, F: FnMut(GlyphId, &Xform, &mut Coverage)> {
    w: usize,
    h: usize,
    outline: F,
    to_window: Xform,
    palette: &'a [skrifa::color::Color],
    /// The text colour, straight sRGB and alpha.
    foreground: [f32; 4],
    /// Anti-aliasing off: coverage is 0 or 1.
    hard: bool,
    xforms: Vec<Xform>,
    clips: Vec<Vec<f32>>,
    /// Whether COLR clip boxes clip. Off for bent type: the outlines are bent but a clip box
    /// would stay a straight rectangle and cut them off. Gradients under a bend are laid out
    /// unbent either way.
    clip_boxes: bool,
    layers: Vec<Vec<[f32; 4]>>,
    /// `push_layer` calls past [`MAX_LAYERS`], painted into the layer below.
    flat: usize,
}

impl<'a, F: FnMut(GlyphId, &Xform, &mut Coverage)> Painter<'a, F> {
    pub fn new(w: usize, h: usize, outline: F, to_window: Xform, palette: &'a [skrifa::color::Color], foreground: [f32; 4], hard: bool) -> Self {
        let layers = vec![vec![[0.0; 4]; w * h]];
        Self { w, h, outline, to_window, palette, foreground, hard, xforms: Vec::new(), clips: Vec::new(), clip_boxes: true, layers, flat: 0 }
    }

    /// Leaves COLR clip boxes out (bent type); see [`Painter::clip_boxes`].
    pub fn with_clip_boxes(mut self, on: bool) -> Self {
        self.clip_boxes = on;
        self
    }

    /// The painted glyph, premultiplied sRGB, row-major.
    pub fn finish(mut self) -> Vec<[f32; 4]> {
        self.layers.truncate(1);
        self.layers.pop().unwrap_or_default()
    }

    fn current(&self) -> Xform {
        self.xforms.last().copied().unwrap_or(Xform::IDENTITY)
    }

    fn push_coverage(&mut self, cov: Coverage) {
        let mut c = cov.finish();
        if self.hard {
            c.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 });
        }
        if let Some(outer) = self.clips.last() {
            c.iter_mut().zip(outer).for_each(|(v, o)| *v *= o);
        }
        self.clips.push(c);
    }

    /// A palette entry (or the text colour), premultiplied, times `alpha`.
    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let [r, g, b, a] = if index == FOREGROUND {
            self.foreground
        } else {
            match self.palette.get(usize::from(index)) {
                Some(c) => [c.red, c.green, c.blue, c.alpha].map(|v| f32::from(v) / 255.0),
                None => return [0.0; 4],
            }
        };
        let a = (a * alpha).clamp(0.0, 1.0);
        [r * a, g * a, b * a, a]
    }

    fn line(&self, stops: &[ColorStop], extend: Extend) -> ColorLine {
        let mut stops: Vec<(f32, [f32; 4])> = stops.iter().filter(|s| s.offset.is_finite()).map(|s| (s.offset, self.color(s.palette_index, s.alpha))).collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        ColorLine { stops, extend }
    }

    fn paint(&self, brush: Brush<'_>) -> Option<Paint> {
        let pt = |p: skrifa::raw::types::Point<f32>| (p.x, p.y);
        Some(match brush {
            Brush::Solid { palette_index, alpha } => Paint::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                let d = (p1.x - p0.x, p1.y - p0.y);
                let len2 = d.0 * d.0 + d.1 * d.1;
                if !len2.is_normal() || color_stops.is_empty() {
                    return None;
                }
                Paint::Linear { p0: pt(p0), d, len2, line: self.line(color_stops, extend) }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                if color_stops.is_empty() {
                    return None;
                }
                Paint::Radial { c0: pt(c0), r0, cd: (c1.x - c0.x, c1.y - c0.y), dr: r1 - r0, line: self.line(color_stops, extend) }
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                if color_stops.is_empty() {
                    return None;
                }
                Paint::Sweep { c: pt(c0), start: start_angle, end: end_angle, line: self.line(color_stops, extend) }
            }
        })
    }
}

impl<F: FnMut(GlyphId, &Xform, &mut Coverage)> ColorPainter for Painter<'_, F> {
    fn push_transform(&mut self, t: Transform) {
        self.xforms.push(self.current().mul(&xform(&t)));
    }
    fn pop_transform(&mut self) {
        self.xforms.pop();
    }
    fn push_clip_glyph(&mut self, id: GlyphId) {
        let mut cov = Coverage::new(self.w, self.h);
        let m = self.current();
        (self.outline)(id, &m, &mut cov);
        self.push_coverage(cov);
    }
    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        if !self.clip_boxes {
            // Keep push/pop balanced: the clip stays what it was.
            let same = self.clips.last().cloned().unwrap_or_else(|| vec![1.0; self.w * self.h]);
            self.clips.push(same);
            return;
        }
        let mut cov = Coverage::new(self.w, self.h);
        rect_to(&mut cov, &self.to_window.mul(&self.current()), b.x_min as f64, b.y_min as f64, b.x_max as f64, b.y_max as f64);
        self.push_coverage(cov);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn fill(&mut self, brush: Brush<'_>) {
        let Some(paint) = self.paint(brush) else { return };
        let to_paint = self.to_window.mul(&self.current()).invert();
        let w = self.w;
        let clip = self.clips.last();
        let Some(layer) = self.layers.last_mut() else { return };
        for (i, px) in layer.iter_mut().enumerate() {
            let k = clip.map_or(1.0, |c| c.get(i).copied().unwrap_or(0.0));
            if k <= 0.0 {
                continue;
            }
            let src = match (&paint, &to_paint) {
                (Paint::Solid(c), _) => Some(*c),
                (_, Some(inv)) => {
                    let (u, v) = inv.apply((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                    paint.at(u as f32, v as f32)
                }
                (_, None) => None,
            };
            let Some(s) = src else { continue };
            let keep = 1.0 - s[3] * k;
            for j in 0..4 {
                px[j] = s[j] * k + px[j] * keep;
            }
        }
    }
    fn push_layer(&mut self, _: CompositeMode) {
        if self.layers.len() >= MAX_LAYERS || self.flat > 0 {
            self.flat += 1;
        } else {
            self.layers.push(vec![[0.0; 4]; self.w * self.h]);
        }
    }
    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if self.flat > 0 {
            self.flat -= 1;
            return;
        }
        if self.layers.len() < 2 {
            return;
        }
        let Some(src) = self.layers.pop() else { return };
        let Some(dst) = self.layers.last_mut() else { return };
        for (b, s) in dst.iter_mut().zip(&src) {
            *b = composite(mode, *b, *s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn line(extend: Extend) -> ColorLine {
        ColorLine { stops: vec![(0.0, RED), (1.0, BLUE)], extend }
    }

    #[test]
    fn colour_line_extends_pad_repeat_and_reflect() {
        let mid = |c: [f32; 4]| (c[0] - 0.5).abs() < 1e-6 && (c[2] - 0.5).abs() < 1e-6;
        assert!(mid(line(Extend::Pad).at(0.5)));
        assert_eq!(line(Extend::Pad).at(-3.0), RED);
        assert_eq!(line(Extend::Pad).at(7.0), BLUE);
        assert_eq!(line(Extend::Pad).at(f32::NAN), RED);
        assert!(mid(line(Extend::Repeat).at(1.5)));
        assert!((line(Extend::Repeat).at(1.25)[0] - 0.75).abs() < 1e-6);
        assert!((line(Extend::Reflect).at(1.25)[0] - 0.25).abs() < 1e-6);
        assert_eq!(ColorLine { stops: Vec::new(), extend: Extend::Pad }.at(0.5), [0.0; 4]);
    }

    #[test]
    fn colour_line_extends_over_its_stops_not_over_0_to_1() {
        // COLR repeats and reflects over [first stop, last stop], and stops may lie outside 0..1.
        let inner = |extend| ColorLine { stops: vec![(0.25, RED), (0.75, BLUE)], extend };
        let red_share = |c: [f32; 4]| c[0];
        // Period 0.5: 0.875 repeats 0.375 (a quarter of the way to blue).
        assert!((red_share(inner(Extend::Repeat).at(0.875)) - 0.75).abs() < 1e-5);
        // Reflect: 0.875 mirrors 0.625 (three quarters of the way to blue).
        assert!((red_share(inner(Extend::Reflect).at(0.875)) - 0.25).abs() < 1e-5);
        // Pad reaches stops outside 0..1.
        let wide = ColorLine { stops: vec![(-0.5, RED), (1.5, BLUE)], extend: Extend::Pad };
        assert!((red_share(wide.at(0.0)) - 0.75).abs() < 1e-5);
        assert_eq!(wide.at(1.5), BLUE);
    }

    #[test]
    fn clip_boxes_can_be_left_out_for_bent_type() {
        // Under a bending warp the outlines are bent but a clip box would stay a straight
        // rectangle and cut the bent glyph off, so bent type leaves clip boxes out.
        let palette = [skrifa::color::Color { red: 255, green: 0, blue: 0, alpha: 255 }];
        let paint = |clip_boxes: bool| {
            let full = |_: GlyphId, _: &Xform, c: &mut Coverage| rect_to(c, &Xform::IDENTITY, 0.0, 0.0, 8.0, 8.0);
            let mut p = Painter::new(8, 8, full, Xform::IDENTITY, &palette, [0.0, 0.0, 0.0, 1.0], false).with_clip_boxes(clip_boxes);
            p.push_clip_box(BoundingBox { x_min: 0.0, y_min: 0.0, x_max: 2.0, y_max: 2.0 });
            p.push_clip_glyph(GlyphId::new(1));
            p.fill(Brush::Solid { palette_index: 0, alpha: 1.0 });
            p.pop_clip();
            p.pop_clip();
            p.finish()
        };
        let (clipped, bent) = (paint(true), paint(false));
        // Pixel (5, 5) lies outside the 2x2 clip box.
        assert_eq!(clipped.get(5 * 8 + 5).map(|c| c[3]), Some(0.0));
        assert_eq!(bent.get(5 * 8 + 5).map(|c| c[3]), Some(1.0));
        assert_eq!(bent.first().map(|c| c[3]), Some(1.0));
    }

    #[test]
    fn radial_gradient_is_the_two_point_cone() {
        // Concentric circles, radius 0 to 10: t is the distance over 10, red at the centre.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 0.0, cd: (0.0, 0.0), dr: 10.0, line: line(Extend::Pad) };
        assert_eq!(p.at(0.0, 0.0), Some(RED));
        let c = p.at(3.0, 4.0).unwrap_or_default();
        assert!((c[0] - 0.5).abs() < 1e-5, "{c:?}");
        assert_eq!(p.at(30.0, 0.0), Some(BLUE));
        // Two equal circles side by side: no cone covers a point far above them.
        let p = Paint::Radial { c0: (0.0, 0.0), r0: 1.0, cd: (4.0, 0.0), dr: 0.0, line: line(Extend::Pad) };
        assert_eq!(p.at(2.0, 50.0), None);
        assert!(p.at(2.0, 0.5).is_some());
    }

    #[test]
    fn sweep_gradient_turns_clockwise_from_the_start_angle() {
        // skrifa's angles: clockwise, i.e. 360° minus the font's counter-clockwise ones.
        let p = Paint::Sweep { c: (0.0, 0.0), start: 0.0, end: 360.0, line: line(Extend::Pad) };
        // Font space is y up: (0, -1) is 90° counter-clockwise below, 90° clockwise.
        let below = p.at(0.0, -1.0).unwrap_or_default();
        assert!((below[2] - 0.25).abs() < 1e-4, "{below:?}");
        let above = p.at(0.0, 1.0).unwrap_or_default();
        assert!((above[2] - 0.75).abs() < 1e-4, "{above:?}");
    }

    #[test]
    fn layers_composite_with_porter_duff_and_blend_modes() {
        let half_red = [0.5, 0.0, 0.0, 0.5];
        assert_eq!(composite(CompositeMode::SrcOver, BLUE, half_red), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(composite(CompositeMode::DestOver, BLUE, half_red), BLUE);
        assert_eq!(composite(CompositeMode::SrcIn, [0.0; 4], RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::DestOut, BLUE, RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::Clear, BLUE, RED), [0.0; 4]);
        assert_eq!(composite(CompositeMode::Plus, RED, BLUE), [1.0, 0.0, 1.0, 1.0]);
        // Multiply of red and blue on opaque pixels is black.
        assert_eq!(composite(CompositeMode::Multiply, BLUE, RED), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(composite(CompositeMode::Screen, BLUE, RED), [1.0, 0.0, 1.0, 1.0]);
    }
}
