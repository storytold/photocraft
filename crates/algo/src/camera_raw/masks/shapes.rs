//! Rendering one mask component to per-pixel coverage (0 … 1).

use photocraft_color::convert::srgb_to_lab;
use photocraft_geom::Rect;

use super::{Dab, PeoplePart, Shape};
use crate::photo_util::par_rows;
use crate::segment::{ImageSampler, RgbImage, sky, subject};

/// Smooth step from 0 at `e0` to 1 at `e1` (either order).
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 - e0).abs() < 1e-9 {
        return if x >= e1 { 1.0 } else { 0.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Coverage of `shape` over a `w × h` image of developed straight-RGBA pixels.
pub fn render(shape: &Shape, px: &[[f32; 4]], w: usize, h: usize) -> Vec<f32> {
    let n = w.saturating_mul(h);
    let mut out = vec![0.0f32; n];
    if n == 0 || px.len() < n {
        return out;
    }
    let (fw, fh) = (w as f32, h as f32);
    match shape {
        Shape::Brush { dabs } => brush(&mut out, w, h, dabs, px),
        Shape::Linear { zero, full } => {
            let (z, f) = ([zero[0] * fw, zero[1] * fh], [full[0] * fw, full[1] * fh]);
            let d = [f[0] - z[0], f[1] - z[1]];
            let len2 = d[0] * d[0] + d[1] * d[1];
            par_rows(&mut out, w, 1, |y, row| {
                for (x, v) in row.iter_mut().enumerate() {
                    let (px_, py) = (x as f32 + 0.5 - z[0], y as f32 + 0.5 - z[1]);
                    *v = if len2 > 1e-12 { smooth(0.0, 1.0, (px_ * d[0] + py * d[1]) / len2) } else { 0.0 };
                }
            });
        }
        Shape::Radial { center, radius, angle, feather } => {
            let (cx, cy) = (center[0] * fw, center[1] * fh);
            let (rx, ry) = ((radius[0].abs() * fw).max(1e-3), (radius[1].abs() * fh).max(1e-3));
            let (s, c) = angle.to_radians().sin_cos();
            // Measured on Photoshop: the feather reaches past the ellipse, and its middle moves in
            // (Feather 50: half strength at 0.85 of the radius over 0.38 … 1.32; Feather 100:
            // close to linear from the centre to 1.25 radii).
            let f = feather.clamp(0.0, 100.0) / 100.0;
            let (mid, half) = (1.0 - 0.4 * f.powf(1.4), 0.65 * f.sqrt());
            let (inner, outer) = (mid - half, mid + half);
            par_rows(&mut out, w, 1, |y, row| {
                for (x, v) in row.iter_mut().enumerate() {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    let (u, t) = ((dx * c + dy * s) / rx, (-dx * s + dy * c) / ry);
                    let d = (u * u + t * t).sqrt();
                    *v = 1.0 - smooth(inner, outer, d);
                }
            });
        }
        Shape::Color { samples, amount } => {
            let labs: Vec<[f32; 3]> = samples.iter().take(super::MAX_SAMPLES).map(|s| srgb_to_lab(s.map(|v| v.clamp(0.0, 1.0)))).collect();
            // Tolerance in ΔE76: 5 at Amount 0, 65 at 100.
            let tol = 5.0 + amount.clamp(0.0, 100.0) * 0.6;
            par_rows(&mut out, w, 1, |y, row| {
                for (x, v) in row.iter_mut().enumerate() {
                    let q = px[y * w + x];
                    let l = srgb_to_lab([q[0], q[1], q[2]].map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }));
                    let best = labs.iter().map(|s| ((l[0] - s[0]).powi(2) + (l[1] - s[1]).powi(2) + (l[2] - s[2]).powi(2)).sqrt()).fold(f32::MAX, f32::min);
                    *v = 1.0 - smooth(tol * 0.5, tol, best);
                }
            });
        }
        Shape::Luminance { range, smoothness } => {
            let (lo, hi) = (range[0].min(range[1]).clamp(0.0, 100.0), range[0].max(range[1]).clamp(0.0, 100.0));
            let fade = smoothness.clamp(0.0, 100.0) * 0.25 + 1e-3;
            par_rows(&mut out, w, 1, |y, row| {
                for (x, v) in row.iter_mut().enumerate() {
                    let q = px[y * w + x];
                    let l = srgb_to_lab([q[0], q[1], q[2]].map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }))[0];
                    *v = smooth(lo - fade, lo, l) * (1.0 - smooth(hi, hi + fade, l));
                }
            });
        }
        Shape::Depth { .. } => {}
        Shape::Subject | Shape::Background => {
            let cov = ai(px, w, h, None, false);
            for (o, c) in out.iter_mut().zip(cov) {
                *o = if matches!(shape, Shape::Background) { 1.0 - c } else { c };
            }
        }
        Shape::People { part } => out = people(px, w, h, *part),
        Shape::Sky => out = ai(px, w, h, None, true),
        Shape::Objects { rect } => {
            let r = [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])].map(|v| v.clamp(0.0, 1.0));
            let area = Rect::new((r[0] * fw) as i32, (r[1] * fh) as i32, (r[2] * fw).ceil() as i32, (r[3] * fh).ceil() as i32);
            if !area.is_empty() {
                out = ai(px, w, h, Some(area), false);
            }
        }
    }
    out
}

/// People: each face PhotoCraft's detector finds stands for a person whose body is looked for
/// (Select Subject) in a box around and below the face. `Face` is the faces themselves (soft
/// ellipses), `Skin` the skin-coloured part of the people (a YCbCr skin range). Camera Raw uses
/// Adobe's models; this finds comparable regions, not the same ones.
fn people(px: &[[f32; 4]], w: usize, h: usize, part: PeoplePart) -> Vec<f32> {
    let n = w * h;
    let mut out = vec![0.0f32; n];
    let faces = crate::faces::detect(&px[..n], w, h, 0.6).unwrap_or_default();
    let (fw, fh) = (w as f32, h as f32);
    for f in &faces {
        let [x, y, bw, bh] = f.rect;
        if !(bw > 1.0 && bh > 1.0) {
            continue;
        }
        match part {
            PeoplePart::Face => {
                let (cx, cy, rx, ry) = (x + bw / 2.0, y + bh / 2.0, bw * 0.6, bh * 0.75);
                for yy in ((cy - ry).max(0.0) as usize)..((cy + ry).min(fh) as usize) {
                    for xx in ((cx - rx).max(0.0) as usize)..((cx + rx).min(fw) as usize) {
                        let d = ((xx as f32 + 0.5 - cx) / rx).hypot((yy as f32 + 0.5 - cy) / ry);
                        let v = 1.0 - smooth(0.8, 1.0, d);
                        if let Some(o) = out.get_mut(yy * w + xx) {
                            *o = o.max(v);
                        }
                    }
                }
            }
            PeoplePart::Person | PeoplePart::Skin => {
                let (x0, x1) = ((x - 1.6 * bw).max(0.0), (x + 2.6 * bw).min(fw));
                let (y0, y1) = ((y - 0.6 * bh).max(0.0), fh.min(y + 9.0 * bh));
                let area = Rect::new(x0 as i32, y0 as i32, x1.ceil() as i32, y1.ceil() as i32);
                if area.is_empty() {
                    continue;
                }
                let body = ai(px, w, h, Some(area), false);
                for (o, b) in out.iter_mut().zip(body) {
                    *o = o.max(b);
                }
            }
        }
    }
    if part == PeoplePart::Skin {
        for (o, q) in out.iter_mut().zip(px) {
            if *o <= 0.0 {
                continue;
            }
            let [r, g, b] = [q[0], q[1], q[2]].map(|c| if c.is_finite() { c.clamp(0.0, 1.0) * 255.0 } else { 0.0 });
            let cb = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
            let cr = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
            // The classic skin box (Cb 77–127, Cr 133–173), softened at its edges.
            let k = smooth(72.0, 82.0, cb) * (1.0 - smooth(122.0, 132.0, cb)) * smooth(128.0, 138.0, cr) * (1.0 - smooth(168.0, 178.0, cr));
            *o *= k;
        }
    }
    out
}

/// Classical Select Subject / Select Sky over the image (or the subject inside `within`).
fn ai(px: &[[f32; 4]], w: usize, h: usize, within: Option<Rect>, sky_mask: bool) -> Vec<f32> {
    let n = w * h;
    let img = RgbImage { w, h, px: px[..n].iter().map(|q| [q[0], q[1], q[2]].map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 })).collect() };
    let sampler = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = within.unwrap_or_else(|| Rect::new(0, 0, w as i32, h as i32));
    let region = if sky_mask { sky::select_sky(&sampler, canvas, sky::SkyParams::default()) } else { subject::select_subject(&sampler, canvas) };
    let mut out = vec![0.0f32; n];
    if let Some(r) = region {
        for y in r.bbox.y0.max(0)..r.bbox.y1.min(h as i32) {
            for x in r.bbox.x0.max(0)..r.bbox.x1.min(w as i32) {
                if let Some(o) = out.get_mut(y as usize * w + x as usize) {
                    *o = r.at(x, y);
                }
            }
        }
    }
    out
}

/// Side of the grid brushes are painted on (then interpolated up): masks are soft, and this
/// bounds the work for any image size.
const BRUSH_GRID: usize = 1024;

/// Paints the dabs in order: each adds `flow` of the way to `density` under its soft disc, or
/// takes `flow` away when erasing.
fn brush(out: &mut [f32], w: usize, h: usize, dabs: &[Dab], px: &[[f32; 4]]) {
    let f = (w.max(h) as f32 / BRUSH_GRID as f32).max(1.0);
    let (gw, gh) = (((w as f32 / f).ceil() as usize).max(1), ((h as f32 / f).ceil() as usize).max(1));
    let mut grid = vec![0.0f32; gw * gh];
    // Auto Mask compares colours on the brush grid (nearest pixel of the image).
    let colours: Option<Vec<[f32; 3]>> = dabs.iter().any(|d| d.auto).then(|| {
        (0..gw * gh)
            .map(|i| {
                let (x, y) = ((((i % gw) as f32 + 0.5) * f) as usize, (((i / gw) as f32 + 0.5) * f) as usize);
                let q = px.get(y.min(h.saturating_sub(1)) * w + x.min(w.saturating_sub(1))).copied().unwrap_or([0.0; 4]);
                [q[0], q[1], q[2]].map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 })
            })
            .collect()
    });
    paint(&mut grid, gw, gh, dabs, colours.as_deref());
    if gw == w && gh == h {
        out.copy_from_slice(&grid[..w * h]);
        return;
    }
    par_rows(out, w, 1, |y, row| {
        let gy = ((y as f32 + 0.5) / f - 0.5).clamp(0.0, (gh - 1) as f32);
        let (y0, ty) = (gy.floor() as usize, gy.fract());
        let y1 = (y0 + 1).min(gh - 1);
        for (x, v) in row.iter_mut().enumerate() {
            let gx = ((x as f32 + 0.5) / f - 0.5).clamp(0.0, (gw - 1) as f32);
            let (x0, tx) = (gx.floor() as usize, gx.fract());
            let x1 = (x0 + 1).min(gw - 1);
            let g = |xx: usize, yy: usize| grid.get(yy * gw + xx).copied().unwrap_or(0.0);
            let top = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
            let bot = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
            *v = top + (bot - top) * ty;
        }
    });
}

/// Measured on Photoshop (Camera Raw brush masks through XMP): a dab is solid out to
/// `(1 − feather)·radius`, then falls off as a Gaussian of width `0.38·feather·radius`; one dab
/// adds `flow^2.75` of the way to its density (Flow 50 % leaves 0.15).
fn paint(out: &mut [f32], w: usize, h: usize, dabs: &[Dab], colours: Option<&[[f32; 3]]>) {
    let long = w.max(h) as f32;
    for d in dabs.iter().take(super::MAX_DABS) {
        let r = (d.size.abs().min(super::MAX_DAB_SIZE) * long).max(0.5);
        let f = d.feather.clamp(0.0, 100.0) / 100.0;
        let core = r * (1.0 - f);
        let sigma = (0.38 * f * r).max(1e-3);
        let flow = (d.flow.clamp(0.0, 100.0) / 100.0).powf(2.75);
        let density = d.density.clamp(0.0, 100.0) / 100.0;
        let (cx, cy) = (d.x * w as f32, d.y * h as f32);
        if !(cx.is_finite() && cy.is_finite() && r.is_finite()) {
            continue;
        }
        let x0 = ((cx - r).floor().max(0.0) as usize).min(w);
        let x1 = ((cx + r).ceil().max(0.0) as usize).min(w);
        let y0 = ((cy - r).floor().max(0.0) as usize).min(h);
        let y1 = ((cy + r).ceil().max(0.0) as usize).min(h);
        let centre = colours.filter(|_| d.auto).and_then(|c| c.get((cy as usize).min(h.saturating_sub(1)) * w + (cx as usize).min(w.saturating_sub(1)))).copied();
        for y in y0..y1 {
            for x in x0..x1 {
                let dist = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                if dist > r {
                    continue;
                }
                let mut profile = if dist <= core { 1.0 } else { (-((dist - core) / sigma).powi(2)).exp() };
                if let (Some(c0), Some(c)) = (centre, colours.and_then(|c| c.get(y * w + x))) {
                    // Similar colours only: off by an RGB distance of about 0.15.
                    let dd = (c[0] - c0[0]).powi(2) + (c[1] - c0[1]).powi(2) + (c[2] - c0[2]).powi(2);
                    profile *= (-dd / (0.12 * 0.12)).exp();
                }
                let a = flow * profile;
                if a <= 1e-4 {
                    continue;
                }
                let Some(v) = out.get_mut(y * w + x) else { continue };
                if d.erase {
                    *v *= 1.0 - a;
                } else if *v < density {
                    *v = (*v + a * (density - *v)).min(density);
                }
            }
        }
    }
}
