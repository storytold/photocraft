//! Render: Clouds, Difference Clouds, Fibers, Lens Flare, Lighting Effects.

use photocraft_geom::Rect;

use crate::fxutil::{MAXC, luma, rgba, set_rgba, smoothstep, xy};
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, LensType, Light, LightKind, TextureChannel};

/// Smooth value noise in [0, 1).
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |a: i32, b: i32| hash01(a, b, 0, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bot = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bot - top) * sy
}

fn cloud_hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Four times Clouds' cloud size (the lattice cell of its first octave) for bounds of `w` × `h`:
/// the size is a quarter of the longer side, 16..512 px, so in quarter pixels it is a whole
/// number and whether a lattice fits a side is exact (see [`wrap_cells`]).
fn cloud_scale(w: u32, h: u32) -> u32 {
    w.max(h).clamp(64, 2048)
}

/// The lattice cells octave `octave` spans over `period` pixels, when that is a whole number: its
/// cells are `scale / 4 / 2^octave` px (`scale` from [`cloud_scale`]), so `period` holds exactly
/// `4 · period · 2^octave / scale` of them. The lattice then wraps there and the octave tiles.
/// `None` otherwise, however close (512 px hold 1.0005 cells of 511.75 px).
fn wrap_cells(period: Option<u32>, scale: u32, octave: u32) -> Option<i32> {
    let span = (u64::from(period?) * 4) << octave;
    let scale = u64::from(scale);
    (scale > 0 && span.is_multiple_of(scale)).then(|| i32::try_from(span / scale).ok()).flatten().filter(|n| *n >= 1)
}

/// Smooth value noise, summed over octaves (fBm), in 0..1, with cells of `scale / 4` px (see
/// [`cloud_scale`]). `period` (x, y) makes it repeat every that many pixels along an axis,
/// octave by octave where the lattice fits (see [`tile_period`]).
fn clouds_value(x: f32, y: f32, scale: u32, seed: u32, period: (Option<u32>, Option<u32>)) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 4.0 / scale as f32, 0.0);
    for o in 0..8u32 {
        let (fx, fy) = (x * freq, y * freq);
        let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let s = seed.wrapping_add(o * 7919);
        let (wx, wy) = (wrap_cells(period.0, scale, o), wrap_cells(period.1, scale, o));
        // The cell's two lattice lines on an axis, wrapped once per octave where it tiles.
        let lines = |i: i32, n: Option<i32>| match n {
            Some(n) => {
                let a = i.rem_euclid(n);
                (a, if a + 1 == n { 0 } else { a + 1 })
            }
            None => (i, i + 1),
        };
        let ((x0, x1), (y0, y1)) = (lines(ix, wx), lines(iy, wy));
        let (h00, h10) = (cloud_hash(x0, y0, s), cloud_hash(x1, y0, s));
        let (h01, h11) = (cloud_hash(x0, y1, s), cloud_hash(x1, y1, s));
        let top = h00 + (h10 - h00) * sx;
        let bot = h01 + (h11 - h01) * sx;
        sum += (top + (bot - top) * sy) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    // Stretch the bell-shaped sum so the clouds use the full foreground→background range.
    ((sum / norm - 0.5) * 2.2 + 0.5).clamp(0.0, 1.0)
}

/// The period Clouds may repeat with along an axis of `size` pixels: the size itself when it is a
/// power of two, as Photoshop's Clouds tile seamlessly at 256 × 256 but not at 300 × 200 (#8999).
/// An octave only wraps when the axis holds a whole number of its lattice cells, and the noise
/// scale is a quarter of the longer side (16..512 px), so a power-of-two side tiles whenever it is
/// the longer one and at least 16 px (512 × 300 tiles along x), but a shorter one only when that
/// quarter divides it (256 × 1024 tiles both ways, 256 × 300 along neither). Photoshop's
/// behaviour at mixed sizes is unmeasured.
fn tile_period(size: u32) -> Option<u32> {
    (size >= 2 && size.is_power_of_two()).then_some(size)
}

/// Clouds between the foreground and background colours; Difference Clouds subtracts them from
/// the pixels (alpha kept). The cloud size is a quarter of the bounds' longer side, and the
/// pattern is anchored at the bounds' corner (so it moves with a smart object).
pub(crate) fn clouds(src: &Image, out: Rect, ctx: &Ctx, seed: u32, difference: bool, colours: ([f32; 4], [f32; 4])) -> Vec<f32> {
    let n = src.ch;
    let (fg, bg) = colours;
    let scale = cloud_scale(ctx.bounds.width(), ctx.bounds.height());
    let period = (tile_period(ctx.bounds.width()), tile_period(ctx.bounds.height()));
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (x, y) = (x - ctx.bounds.x0, y - ctx.bounds.y0);
        let t = clouds_value(x as f32, y as f32, scale, seed, period);
        let c: [f32; 3] = std::array::from_fn(|k| fg[k] + (bg[k] - fg[k]) * t);
        if difference {
            let p = rgba(ctx, px);
            set_rgba(ctx, px, [(p[0] - c[0]).abs(), (p[1] - c[1]).abs(), (p[2] - c[2]).abs(), p[3]]);
        } else {
            set_rgba(ctx, px, [c[0], c[1], c[2], 1.0]);
        }
    }
    res
}

/// Fibers: vertical fibrous streaks between the foreground and background
/// colours. Variance sets how quickly the colour varies across fibres (and
/// how short they are); strength stretches each fibre vertically.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fibers(src: &Image, out: Rect, ctx: &Ctx, variance: f32, strength: f32, seed: u32, colours: ([f32; 4], [f32; 4])) -> Vec<f32> {
    let n = src.ch;
    let var = variance.clamp(1.0, 64.0);
    let st = strength.clamp(1.0, 64.0);
    let (fg, bg) = colours;
    // Horizontal frequency rises with variance; vertical frequency falls with strength.
    let fx = 0.08 + var / 64.0 * 0.6;
    let fy = fx / (2.0 + st * 1.5) * (0.5 + var / 32.0);
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        // Anchored at the bounds' corner, so the fibres move with a smart object.
        let (x, y) = xy(out, i);
        let (xf, yf) = ((x - ctx.bounds.x0) as f32, (y - ctx.bounds.y0) as f32);
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut norm = 0.0;
        let (mut ax, mut ay) = (fx, fy);
        for o in 0..4u32 {
            sum += value_noise(xf * ax, yf * ay, seed.wrapping_add(o * 977)) * amp;
            norm += amp;
            amp *= 0.55;
            ax *= 2.0;
            ay *= 2.0;
        }
        // Fine per-column grain gives the hairline texture.
        let grain = value_noise(xf * 0.9, yf * fy * 0.25, seed ^ 0x5bd1) - 0.5;
        let t = ((sum / norm - 0.5) * (1.6 + var / 40.0) + 0.5 + grain * 0.25).clamp(0.0, 1.0);
        let c = [fg[0] + (bg[0] - fg[0]) * t, fg[1] + (bg[1] - fg[1]) * t, fg[2] + (bg[2] - fg[2]) * t, 1.0];
        set_rgba(ctx, px, c);
    }
    res
}

/// One lens-flare ghost: position along the flare→centre axis, radius and tint.
struct Ghost {
    t: f32,
    r: f32,
    tint: [f32; 3],
    k: f32,
}

/// Lens Flare: an additive (screen) glow, rays, halo ring and ghost discs
/// along the line through the image centre. Lens types vary the elements.
pub(crate) fn lens_flare(src: &Image, out: Rect, ctx: &Ctx, brightness: f32, centre: (f32, f32), lens: LensType) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let diag = bw.hypot(bh).max(1.0);
    let (cx, cy) = (b.x0 as f32 + centre.0 * bw, b.y0 as f32 + centre.1 * bh);
    let (mx, my) = (b.x0 as f32 + bw / 2.0, b.y0 as f32 + bh / 2.0);
    let bright = brightness.clamp(10.0, 300.0) / 100.0;
    let s = diag / 1000.0;
    let (core, glow, ring_r, ring_w, rays, streak) = match lens {
        LensType::Zoom => (14.0, 90.0, 0.0, 0.0, 0.5, 0.0),
        LensType::Prime35 => (12.0, 70.0, 90.0, 6.0, 0.3, 0.0),
        LensType::Prime105 => (22.0, 130.0, 0.0, 0.0, 0.8, 0.0),
        LensType::MoviePrime => (10.0, 60.0, 0.0, 0.0, 0.2, 1.0),
    };
    let ghosts: Vec<Ghost> = match lens {
        LensType::Zoom => vec![
            Ghost { t: 0.35, r: 10.0, tint: [0.6, 0.8, 1.0], k: 0.18 },
            Ghost { t: 0.6, r: 26.0, tint: [0.5, 1.0, 0.6], k: 0.10 },
            Ghost { t: 0.85, r: 6.0, tint: [1.0, 0.7, 0.4], k: 0.25 },
            Ghost { t: 1.25, r: 40.0, tint: [0.7, 0.5, 1.0], k: 0.08 },
            Ghost { t: 1.5, r: 16.0, tint: [1.0, 0.6, 0.5], k: 0.15 },
            Ghost { t: 1.9, r: 70.0, tint: [0.5, 0.7, 1.0], k: 0.06 },
        ],
        LensType::Prime35 => vec![Ghost { t: 0.7, r: 18.0, tint: [0.6, 1.0, 0.7], k: 0.12 }, Ghost { t: 1.4, r: 30.0, tint: [1.0, 0.7, 0.5], k: 0.09 }],
        LensType::Prime105 => vec![Ghost { t: 0.5, r: 14.0, tint: [1.0, 0.8, 0.5], k: 0.12 }, Ghost { t: 1.6, r: 50.0, tint: [0.6, 0.7, 1.0], k: 0.07 }],
        LensType::MoviePrime => vec![
            Ghost { t: 0.4, r: 8.0, tint: [0.5, 0.7, 1.0], k: 0.2 },
            Ghost { t: 1.0, r: 20.0, tint: [0.4, 0.6, 1.0], k: 0.12 },
            Ghost { t: 1.6, r: 12.0, tint: [0.5, 0.8, 1.0], k: 0.16 },
        ],
    };
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (dx, dy) = (fx - cx, fy - cy);
        let d = (dx * dx + dy * dy).sqrt();
        let mut f = [0.0f32; 3];
        // Hot core and wide glow (warm white).
        let g = (-(d / (core * s)).powi(2)).exp() + 0.35 * (-d / (glow * s)).exp();
        for (k, w) in [1.0, 0.95, 0.85].iter().enumerate() {
            f[k] += g * w;
        }
        // Rays: angular modulation decaying with distance.
        if rays > 0.0 {
            let th = dy.atan2(dx);
            let m = ((th * 12.0).sin() * 0.5 + 0.5).powi(8) + ((th * 7.0 + 1.3).sin() * 0.5 + 0.5).powi(12) * 0.6;
            let rr = rays * m * (-d / (glow * 1.6 * s)).exp();
            for v in f.iter_mut() {
                *v += rr * 0.6;
            }
        }
        // Anamorphic horizontal streak (movie prime).
        if streak > 0.0 {
            let st = (-(dy.abs() / (2.5 * s))).exp() * (-(dx.abs() / (450.0 * s))).exp();
            f[0] += st * 0.5;
            f[1] += st * 0.7;
            f[2] += st;
        }
        // Halo ring with a slight chromatic spread.
        if ring_r > 0.0 {
            for (k, off) in [1.04f32, 1.0, 0.96].iter().enumerate() {
                let rd = (d - ring_r * s * off) / (ring_w * s);
                f[k] += 0.18 * (-rd * rd).exp();
            }
        }
        // Ghosts along the axis through the centre.
        for gh in &ghosts {
            let (gx, gy) = (cx + (mx - cx) * gh.t * 2.0, cy + (my - cy) * gh.t * 2.0);
            let gd = ((fx - gx).powi(2) + (fy - gy).powi(2)).sqrt() / (gh.r * s);
            if gd < 1.2 {
                // Soft-edged disc, slightly brighter at the rim.
                let e = smoothstep(1.0, 0.85, gd) * (0.6 + 0.4 * gd);
                for (fk, t) in f.iter_mut().zip(gh.tint) {
                    *fk += e * gh.k * t;
                }
            }
        }
        let mut c = rgba(ctx, px);
        for k in 0..3 {
            let a = (f[k] * bright).max(0.0);
            // Screen (c + a·(1 − c)) keeps display-referred values below white; a 32-bit
            // value already above white is left as is rather than pulled down towards it.
            c[k] += a.min(1.0) * (1.0 - c[k]).max(0.0);
        }
        set_rgba(ctx, px, c);
    }
    res
}

pub(crate) struct LightingSpec<'a> {
    pub lights: &'a [Light],
    pub gloss: f32,
    pub metallic: f32,
    pub exposure: f32,
    pub ambience: f32,
    pub texture: TextureChannel,
    pub height: f32,
    pub white_is_high: bool,
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / m, v[1] / m, v[2] / m]
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Lighting Effects: Blinn–Phong shading of the image as a surface (bumped by
/// a texture channel) under spot, point and infinite lights.
pub(crate) fn lighting(src: &Image, out: Rect, ctx: &Ctx, spec: &LightingSpec) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let short = bw.min(bh).max(1.0);
    let hscale = spec.height.clamp(0.0, 100.0) / 100.0 * 0.06 * short;
    let shininess = 2f32.powf(1.0 + (spec.gloss.clamp(-100.0, 100.0) + 100.0) / 200.0 * 7.0);
    let spec_k = (spec.gloss.clamp(-100.0, 100.0) + 100.0) / 200.0 * 0.7;
    let metal = (spec.metallic.clamp(-100.0, 100.0) + 100.0) / 200.0;
    let expo = 2f32.powf(spec.exposure.clamp(-100.0, 100.0) / 50.0);
    let ambient = spec.ambience.clamp(-100.0, 100.0) / 100.0 * 0.5;
    let mut tmp = [0.0f32; MAXC];
    let tex = |x: i32, y: i32, tmp: &mut [f32; MAXC]| -> f32 {
        let xx = x.clamp(b.x0, b.x1 - 1);
        let yy = y.clamp(b.y0, b.y1 - 1);
        for (c, t) in tmp.iter_mut().enumerate().take(n) {
            *t = src.get(xx, yy, c);
        }
        let v = match spec.texture {
            TextureChannel::None => 0.0,
            TextureChannel::Alpha => {
                if ctx.alpha {
                    tmp[n - 1]
                } else {
                    1.0
                }
            }
            TextureChannel::Luminance => luma(ctx, &tmp[..n]),
            TextureChannel::Red | TextureChannel::Green | TextureChannel::Blue => {
                let k = match spec.texture {
                    TextureChannel::Red => 0,
                    TextureChannel::Green => 1,
                    _ => 2,
                };
                if ctx.mode == photocraft_color::ColorMode::Rgb { tmp[k] } else { rgba(ctx, &tmp[..n])[k] }
            }
        };
        if spec.white_is_high { v } else { 1.0 - v }
    };
    let lights: Vec<Light> = spec.lights.to_vec();
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let nrm = if spec.texture == TextureChannel::None || hscale <= 0.0 {
            [0.0, 0.0, 1.0]
        } else {
            let hx = (tex(x + 1, y, &mut tmp) - tex(x - 1, y, &mut tmp)) * 0.5 * hscale;
            let hy = (tex(x, y + 1, &mut tmp) - tex(x, y - 1, &mut tmp)) * 0.5 * hscale;
            norm3([-hx, -hy, 1.0])
        };
        let mut c = rgba(ctx, px);
        let base = [c[0], c[1], c[2]];
        let mut lit = [ambient; 3];
        let mut spc = [0.0f32; 3];
        for l in &lights {
            let lpos = [b.x0 as f32 + l.x * bw, b.y0 as f32 + l.y * bh, l.z * short];
            let (ldir, atten) = match l.kind {
                LightKind::Infinite => {
                    let (s, co) = l.angle.to_radians().sin_cos();
                    let e = l.elevation.clamp(0.0, 90.0).to_radians();
                    (norm3([co * e.cos(), -s * e.cos(), e.sin()]), 1.0)
                }
                LightKind::Point => {
                    let v = [lpos[0] - fx, lpos[1] - fy, lpos[2]];
                    let dist = (v[0] * v[0] + v[1] * v[1]).sqrt();
                    let reach = (l.radius * short).max(1.0);
                    (norm3(v), (1.0 - dist / reach).clamp(0.0, 1.0).powi(2))
                }
                LightKind::Spot => {
                    let v = [lpos[0] - fx, lpos[1] - fy, lpos[2]];
                    let ld = norm3(v);
                    let tgt = [b.x0 as f32 + l.target_x * bw, b.y0 as f32 + l.target_y * bh, 0.0];
                    let axis = norm3([tgt[0] - lpos[0], tgt[1] - lpos[1], tgt[2] - lpos[2]]);
                    let cosang = -dot3(ld, axis);
                    let outer = l.cone.clamp(1.0, 89.0).to_radians().cos();
                    let inner = (l.cone.clamp(1.0, 89.0) * (l.hotspot.clamp(0.0, 100.0) / 100.0)).to_radians().cos();
                    (ld, smoothstep(outer, inner.max(outer + 1e-4), cosang))
                }
            };
            if atten <= 0.0 {
                continue;
            }
            let inten = l.intensity.clamp(-100.0, 100.0) / 50.0 * atten;
            let ndl = dot3(nrm, ldir).max(0.0);
            let hv = norm3([ldir[0], ldir[1], ldir[2] + 1.0]);
            let s = dot3(nrm, hv).max(0.0).powf(shininess) * spec_k;
            for k in 0..3 {
                lit[k] += ndl * inten * l.color[k];
                // Metallic highlights take the surface colour; plastic ones the light's.
                let sc = l.color[k] * (1.0 - metal) + base[k] * metal;
                spc[k] += s * inten.max(0.0) * sc;
            }
        }
        for k in 0..3 {
            c[k] = ((base[k] * lit[k] + spc[k]) * expo).max(0.0);
        }
        set_rgba(ctx, px, c);
    }
    res
}

#[cfg(test)]
mod clouds_tests {
    use super::{cloud_scale, clouds_value, tile_period, wrap_cells};

    /// The mean jump between neighbouring values across the wrap seam and inside the pattern.
    fn jumps(w: u32, h: u32) -> (f32, f32) {
        let scale = cloud_scale(w, h);
        let period = (tile_period(w), tile_period(h));
        let v = |x: u32, y: u32| clouds_value(x as f32, y as f32, scale, 7, period);
        let seam = (0..h).map(|y| (v(w - 1, y) - v(0, y)).abs()).sum::<f32>() / h as f32;
        let inside = (0..h).map(|y| (v(w / 2, y) - v(w / 2 + 1, y)).abs()).sum::<f32>() / h as f32;
        (seam, inside)
    }

    /// [`jumps`] down the columns: across the seam from the last row to the first, and inside.
    fn row_jumps(w: u32, h: u32) -> (f32, f32) {
        let scale = cloud_scale(w, h);
        let period = (tile_period(w), tile_period(h));
        let v = |x: u32, y: u32| clouds_value(x as f32, y as f32, scale, 7, period);
        let seam = (0..w).map(|x| (v(x, h - 1) - v(x, 0)).abs()).sum::<f32>() / w as f32;
        let inside = (0..w).map(|x| (v(x, h / 2) - v(x, h / 2 + 1)).abs()).sum::<f32>() / w as f32;
        (seam, inside)
    }

    /// #8999: like Photoshop's, Clouds on a 256 × 256 image tile: the last column runs on into
    /// the first as smoothly as neighbouring columns do (Filter › Other › Offset shows no seam).
    /// At 300 × 200 Photoshop's don't, and neither do these.
    #[test]
    fn clouds_tile_at_power_of_two_sizes() {
        let (seam, inside) = jumps(256, 256);
        assert!(seam < inside * 3.0 + 0.01, "seam {seam} vs {inside}");
        let (seam, inside) = row_jumps(256, 256);
        assert!(seam < inside * 3.0 + 0.01, "bottom row into the top: seam {seam} vs {inside}");
        let (seam, inside) = jumps(300, 200);
        assert!(seam > inside * 3.0, "300 × 200 doesn't tile: seam {seam} vs {inside}");
        assert_eq!(tile_period(256), Some(256));
        assert_eq!(tile_period(300), None);
        assert_eq!(tile_period(1), None);
    }

    /// Tiling only changes power-of-two sizes: at 300 × 200 the values are upstream's before
    /// #8999 (computed with its `clouds_value`, seed 7).
    #[test]
    fn clouds_at_other_sizes_are_unchanged() {
        let (w, h) = (300u32, 200u32);
        let scale = cloud_scale(w, h);
        let period = (tile_period(w), tile_period(h));
        let before = [
            (0u32, 0u32, 0.366_656_63f32),
            (1, 0, 0.350_685_9),
            (37, 11, 0.350_264_6),
            (150, 100, 0.535_888_6),
            (299, 0, 0.168_244_15),
            (0, 199, 0.850_466_7),
            (299, 199, 0.756_825_2),
            (212, 157, 0.899_339_4),
        ];
        for (x, y, want) in before {
            assert_eq!(clouds_value(x as f32, y as f32, scale, 7, period), want, "({x}, {y})");
        }
    }

    /// 512 × 2047: a power-of-two width whose lattice doesn't fit it (the cloud size is
    /// 2047 / 4 = 511.75 px, so 512 px hold 1.0005 cells), so no octave wraps and the values are
    /// upstream's before #8999 (computed with its `clouds_value`, seed 7).
    #[test]
    fn clouds_whose_lattice_nearly_fits_are_unchanged() {
        let (w, h) = (512u32, 2047u32);
        let scale = cloud_scale(w, h);
        let period = (tile_period(w), tile_period(h));
        assert_eq!(period.0, Some(512));
        assert_eq!((0..8).find_map(|o| wrap_cells(period.0, scale, o)), None, "no octave fits");
        assert_eq!(wrap_cells(Some(256), cloud_scale(256, 1024), 0), Some(1));
        let before = [
            (0u32, 0u32, 0.366_656_63f32),
            (1, 0, 0.365_521_55),
            (37, 11, 0.285_922),
            (0, 1000, 0.307_037_9),
            (511, 1000, 0.814_692_74),
            (256, 1023, 0.612_607_2),
            (511, 2046, 0.960_405),
        ];
        for (x, y, want) in before {
            assert_eq!(clouds_value(x as f32, y as f32, scale, 7, period), want, "({x}, {y})");
        }
    }
}
