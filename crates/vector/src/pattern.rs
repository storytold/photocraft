//! Shared pattern pixels and placement for vector shapes and compositing.

use photocraft_doc::{ColorMode, Pattern, PixelFormat, Rect, SampleType};
use photocraft_raster::Surface;

/// Shapes prepare at most two paints; larger patterns sample their existing COW surface.
const PREPARED_TILE_BYTES: usize = 64 << 20;

fn paint_in_mode(mut rgba: [f32; 4], source_mode: ColorMode, mode: ColorMode) -> [f32; 4] {
    let paints_in = matches!(mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone | ColorMode::Cmyk);
    if paints_in && source_mode != mode {
        let fmt = PixelFormat::new(mode, SampleType::F32, true);
        let mut values = [0.0f32; 5];
        let n = photocraft_raster::from_rgba_into(&fmt, rgba, &mut values);
        rgba = photocraft_raster::to_rgba(&fmt, values.get(..n).unwrap_or_default());
    }
    rgba
}

/// A pattern's pixels (straight-alpha RGBA, row-major) as they look painted into a document of
/// colour mode `mode`. A pattern keeps its own colours (an RGB pattern stays RGB, in the
/// library and in the document), but in a Grayscale or CMYK document it paints that mode's
/// colours: what Edit › Fill writes into the document's pixels, and what Photoshop shows (the
/// pattern in colour in its picker, filled in grey). Empty for a pattern that is not
/// [paintable](Pattern::is_paintable).
pub fn pattern_rgba(p: &Pattern, mode: ColorMode) -> Vec<[f32; 4]> {
    let Some(n) = p.is_paintable().then(|| usize::try_from(p.pixel_count()).ok()).flatten() else {
        return Vec::new();
    };
    let mut px = vec![[0.0f32; 4]; n];
    p.surface.read_rgba_into(p.rect(), &mut px);
    for q in &mut px {
        *q = paint_in_mode(*q, p.surface.format().mode, mode);
    }
    px
}

/// A pattern converted once to straight-alpha RGBA for sampling.
#[derive(Clone, Debug)]
pub struct Tile {
    w: usize,
    h: usize,
    /// Premultiplied RGBA.
    px: Vec<[f32; 4]>,
    /// Oversized tiles retain their COW source instead of allocating a second full buffer.
    source: Option<(Surface, ColorMode)>,
}

impl Tile {
    pub fn new(p: &Pattern) -> Option<Tile> {
        Tile::new_in(p, ColorMode::Rgb)
    }

    /// The tile of `p` painted into a document of colour mode `mode` (see [`pattern_rgba`]).
    pub fn new_in(p: &Pattern, mode: ColorMode) -> Option<Tile> {
        Self::new_in_with_budget(p, mode, PREPARED_TILE_BYTES)
    }

    fn new_in_with_budget(p: &Pattern, mode: ColorMode, budget: usize) -> Option<Tile> {
        if !p.is_paintable() {
            return None;
        }
        let (w, h) = (p.width as usize, p.height as usize);
        let bytes = w.checked_mul(h)?.checked_mul(std::mem::size_of::<[f32; 4]>())?;
        if bytes > budget {
            return Some(Tile { w, h, px: Vec::new(), source: Some((p.surface.clone(), mode)) });
        }
        let mut px = pattern_rgba(p, mode);
        for q in &mut px {
            for c in 0..3 {
                q[c] *= q[3];
            }
        }
        Some(Tile { w, h, px, source: None })
    }

    #[inline]
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let xi = x.rem_euclid(self.w as i64) as usize;
        let yi = y.rem_euclid(self.h as i64) as usize;
        if let Some((surface, mode)) = &self.source {
            let mut rgba = paint_in_mode(surface.rgba(xi as i32, yi as i32), surface.format().mode, *mode);
            let alpha = rgba[3];
            for c in rgba.iter_mut().take(3) {
                *c *= alpha;
            }
            rgba
        } else {
            self.px.get(yi * self.w + xi).copied().unwrap_or([0.0; 4])
        }
    }

    /// Bilinear, wrapping sample at continuous tile coordinates (pixel centres at `i + 0.5`).
    /// Returns straight alpha.
    pub fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let (u, v) = (u.rem_euclid(self.w as f64) - 0.5, v.rem_euclid(self.h as f64) - 0.5);
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = ((u - x0) as f32, (v - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut acc = [0.0f32; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            if wy == 0.0 {
                continue;
            }
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                if wx == 0.0 {
                    continue;
                }
                let p = self.at(x0 + dx, y0 + dy);
                let k = wx * wy;
                for c in 0..4 {
                    acc[c] += p[c] * k;
                }
            }
        }
        if acc[3] <= 0.0 {
            return [0.0; 4];
        }
        [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3].min(1.0)]
    }
}

/// Where a pattern sits: document point → tile coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    origin: (f64, f64),
    /// Inverse rotation (cos, sin) and inverse scale.
    cs: (f64, f64),
    inv_scale: f64,
}

impl Placement {
    /// `frame`: the layer's frame (used when `link`); `phase` in pixels; `scale` as a fraction;
    /// `angle` in degrees counter-clockwise.
    pub fn new(frame: Rect, link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let anchor = if frame.is_empty() { (0.0, 0.0) } else { (f64::from(frame.x0), f64::from(frame.y0)) };
        Self::anchored(anchor, link, phase, scale, angle)
    }

    /// Like [`Placement::new`], with the linked origin given as a point. Layer effects anchor
    /// linked patterns at the layer's effects reference point (PSD `fxrp`): Photoshop's
    /// pattern overlays and pattern strokes match pixel-for-pixel tiled from there.
    pub fn anchored(anchor: (f64, f64), link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let base = if link { anchor } else { (0.0, 0.0) };
        let origin = (base.0 + f64::from(phase.0), base.1 + f64::from(phase.1));
        let s = f64::from(scale);
        let inv_scale = if s.is_finite() && s > 0.0 { 1.0 / s } else { 1.0 };
        let a = f64::from(angle).to_radians();
        Placement { origin, cs: (a.cos(), a.sin()), inv_scale }
    }

    /// (origin, inverse rotation (cos, sin), inverse scale), for GPU shaders.
    pub fn parts(&self) -> ((f64, f64), (f64, f64), f64) {
        (self.origin, self.cs, self.inv_scale)
    }

    /// Tile coordinates of the document point `(x, y)`.
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        // Undo a counter-clockwise rotation (y points down on screen).
        let (c, s) = self.cs;
        let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
        (rx * self.inv_scale, ry * self.inv_scale)
    }
}
