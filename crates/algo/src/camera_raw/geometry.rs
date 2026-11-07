//! Camera Raw's Geometry panel: Upright (Off, Auto, Level, Vertical, Full, Guided), the manual
//! Transform sliders (Vertical, Horizontal, Rotate, Aspect, Scale, Offset X / Y) and Constrain
//! Crop.
//!
//! The manual Transform was measured as a black box on Photoshop 2026 (Camera Raw 18): a
//! synthetic target of dots was developed with each slider (alone and combined, eight image
//! sizes from 600 × 400 to 2000 × 2000, landscape, portrait and square) and the homography
//! fitted to the dots. No Adobe code or data was used. In coordinates centred on the image,
//! with `r` the half diagonal and `s` the short side, the result is `T · S · A · P · R`:
//!
//! * **R** (Rotate, degrees): a rotation, clockwise on screen for positive values.
//! * **P** (Vertical `v`, Horizontal `h`): a projective keystone with bottom row
//!   `c = −0.006 · (h, v) / r` and, along `ĉ`, a stretch `a(|(h, v)|)` (`I + (a − 1) ĉ ĉᵀ`)
//!   read from [`STRETCH`] (measured; 1.108 at 50, 1.681 at 100), the same for every size
//!   and aspect.
//! * **A** (Aspect `k = 1 + |aspect| / 500`): area-preserving, `√k` taller and `1/√k`
//!   narrower for positive values, the other way for negative ones.
//! * **S** (Scale): uniform, percent.
//! * **T** (Offset X / Y): a translation of `0.009 · X · r · w / s` pixels to the right and
//!   `0.009 · Y · r · h / s` up.
//!
//! On 134 measured settings this model lands within 1.1 px of Photoshop's dots (mean 0.2 px,
//! about the dots' own centroid noise). Pixels the warp leaves uncovered become transparent,
//! as in Photoshop. Upright's analysis is ours ([`upright`]): line segments, their vanishing
//! points, and a homography sending them to infinity; Photoshop's own choices differ.
//! Constrain Crop scales the result up about the centre until the image's frame covers it.

#![allow(clippy::needless_range_loop)]
// `!(x > eps)` on purpose: NaN must take the early return too.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::photo_util::par_rows;
use crate::transform::Homography;

use super::CameraRaw;

pub mod lines;
pub mod upright;

/// Camera Raw's stretch along the keystone direction for Vertical / Horizontal 0, 5, …, 100
/// (measured; the Vertical and Horizontal sliders share it, combined by their length).
pub const STRETCH: [f64; 21] = [
    1.0, 1.00099, 1.00244, 1.00523, 1.00988, 1.01733, 1.02748, 1.04102, 1.05864, 1.08134, 1.10780, 1.13991, 1.17730, 1.22035, 1.26904, 1.32364,
    1.38394, 1.45026, 1.52138, 1.59853, 1.68087,
];

/// Most Upright guides Camera Raw accepts.
pub const MAX_GUIDES: usize = 4;

/// The Upright modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upright {
    Off,
    Auto,
    Level,
    Vertical,
    Full,
    Guided,
}

impl Upright {
    /// Parses a settings value (`off | auto | level | vertical | full | guided`, any case; an
    /// empty string is Off).
    pub fn parse(s: &str) -> Option<Upright> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "" | "off" => Upright::Off,
            "auto" => Upright::Auto,
            "level" => Upright::Level,
            "vertical" => Upright::Vertical,
            "full" => Upright::Full,
            "guided" => Upright::Guided,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Upright::Off => "off",
            Upright::Auto => "auto",
            Upright::Level => "level",
            Upright::Vertical => "vertical",
            Upright::Full => "full",
            Upright::Guided => "guided",
        }
    }
}

/// The manual Transform sliders in Camera Raw's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// −100 … 100.
    pub vertical: f64,
    /// −100 … 100.
    pub horizontal: f64,
    /// Degrees, −10 … 10.
    pub rotate: f64,
    /// −100 … 100.
    pub aspect: f64,
    /// Percent, 50 … 150.
    pub scale: f64,
    /// −100 … 100.
    pub x: f64,
    /// −100 … 100.
    pub y: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { vertical: 0.0, horizontal: 0.0, rotate: 0.0, aspect: 0.0, scale: 100.0, x: 0.0, y: 0.0 }
    }
}

fn finite_or(v: f32, d: f64, lo: f64, hi: f64) -> f64 {
    if v.is_finite() { f64::from(v).clamp(lo, hi) } else { d }
}

impl Transform {
    /// The sliders of `p`, clamped to Camera Raw's ranges (non-finite values are neutral).
    pub fn of(p: &CameraRaw) -> Transform {
        Transform {
            vertical: finite_or(p.perspective_vertical, 0.0, -100.0, 100.0),
            horizontal: finite_or(p.perspective_horizontal, 0.0, -100.0, 100.0),
            rotate: finite_or(p.perspective_rotate, 0.0, -10.0, 10.0),
            aspect: finite_or(p.perspective_aspect, 0.0, -100.0, 100.0),
            scale: finite_or(p.perspective_scale, 100.0, 50.0, 150.0),
            x: finite_or(p.perspective_x, 0.0, -100.0, 100.0),
            y: finite_or(p.perspective_y, 0.0, -100.0, 100.0),
        }
    }
    pub fn is_identity(&self) -> bool {
        *self == Transform::default()
    }
}

/// `a · b` for row-major 3 × 3 matrices.
pub(crate) fn mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    std::array::from_fn(|i| {
        let (r, c) = (i / 3, i % 3);
        (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum()
    })
}

const IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The measured stretch at keystone strength `t` (0 … 100+, by the sliders' length).
pub fn stretch(t: f64) -> f64 {
    let t = if t.is_finite() { t.abs() } else { 0.0 };
    let f = t / 5.0;
    let i = (f.floor() as usize).min(STRETCH.len() - 2);
    let k = (f - i as f64).min(1.5);
    let (a, b) = (STRETCH[i], STRETCH[i + 1]);
    a + (b - a) * k
}

/// The manual Transform as a homography in coordinates centred on a `w × h` image (pixels,
/// y down), mapping source to result.
pub fn manual(t: &Transform, w: f64, h: f64) -> [f64; 9] {
    let r = (w.hypot(h) / 2.0).max(1e-9);
    let short = w.min(h).max(1e-9);
    let c = [-0.006 * t.horizontal / r, -0.006 * t.vertical / r];
    let len = t.vertical.hypot(t.horizontal);
    let mut p = IDENTITY;
    if len > 0.0 {
        let a = stretch(len);
        let n = c[0].hypot(c[1]);
        let u = [c[0] / n, c[1] / n];
        p[0] = 1.0 + (a - 1.0) * u[0] * u[0];
        p[1] = (a - 1.0) * u[0] * u[1];
        p[3] = (a - 1.0) * u[1] * u[0];
        p[4] = 1.0 + (a - 1.0) * u[1] * u[1];
        p[6] = c[0];
        p[7] = c[1];
    }
    let th = t.rotate.to_radians();
    let rot = [th.cos(), -th.sin(), 0.0, th.sin(), th.cos(), 0.0, 0.0, 0.0, 1.0];
    let k = 1.0 + t.aspect.abs() / 500.0;
    let (ax, ay) = if t.aspect >= 0.0 { (k.powf(-0.5), k.sqrt()) } else { (k.sqrt(), k.powf(-0.5)) };
    let asp = [ax, 0.0, 0.0, 0.0, ay, 0.0, 0.0, 0.0, 1.0];
    let s = t.scale / 100.0;
    let sc = [s, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 1.0];
    let tr = [1.0, 0.0, 0.009 * t.x * r * w / short, 0.0, 1.0, -0.009 * t.y * r * h / short, 0.0, 0.0, 1.0];
    normalized(mul(&tr, &mul(&sc, &mul(&asp, &mul(&p, &rot)))))
}

fn normalized(m: [f64; 9]) -> [f64; 9] {
    if m[8].abs() > 1e-12 { m.map(|v| v / m[8]) } else { m }
}

fn project(m: &[f64; 9], x: f64, y: f64) -> Option<[f64; 2]> {
    let wv = m[6] * x + m[7] * y + m[8];
    (wv > 1e-9).then(|| [(m[0] * x + m[1] * y + m[2]) / wv, (m[3] * x + m[4] * y + m[5]) / wv])
}

/// The largest `k ≤ 1` for which the centred `k·w × k·h` rectangle lies inside the image of the
/// source frame under `m` (centred coordinates), or `None` when the frame folds over.
pub fn fill_factor(m: &[f64; 9], w: f64, h: f64) -> Option<f64> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]];
    let mut q = [[0.0; 2]; 4];
    for (i, c) in corners.iter().enumerate() {
        q[i] = project(m, c[0], c[1])?;
    }
    // Orientation of the quad (it keeps the frame's, clockwise in y-down screen space).
    let area: f64 = (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum();
    if area.abs() < 1e-9 {
        return None;
    }
    let inside = |p: [f64; 2]| {
        (0..4).all(|i| {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            cross * area.signum() >= -1e-9
        })
    };
    let fits = |k: f64| corners.iter().all(|c| inside([c[0] * k, c[1] * k]));
    if !fits(1e-6) {
        return None;
    }
    if fits(1.0) {
        return Some(1.0);
    }
    let (mut lo, mut hi) = (1e-6, 1.0);
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        if fits(mid) { lo = mid } else { hi = mid }
    }
    Some(lo)
}

/// Scales `m` up about the centre so the source frame covers the whole result (Constrain Crop).
pub fn constrain(m: &[f64; 9], w: f64, h: f64) -> [f64; 9] {
    match fill_factor(m, w, h) {
        Some(k) if k > 1e-3 && k < 1.0 => {
            let z = 1.0 / k;
            mul(&[z, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 1.0], m)
        }
        _ => *m,
    }
}

/// Moves `m`'s result so the bounding box of the warped frame is centred, as Photoshop's Upright
/// does (measured: a Vertical keystone shifts the picture by the box's offset).
pub fn recentre(m: &[f64; 9], w: f64, h: f64) -> [f64; 9] {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for c in [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]] {
        let Some(q) = project(m, c[0], c[1]) else { return *m };
        for k in 0..2 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let (tx, ty) = (-(lo[0] + hi[0]) / 2.0, -(lo[1] + hi[1]) / 2.0);
    if !(tx.is_finite() && ty.is_finite()) {
        return *m;
    }
    mul(&[1.0, 0.0, tx, 0.0, 1.0, ty, 0.0, 0.0, 1.0], m)
}

/// The scale that makes the warped frame's box exactly as wide as the frame, or as tall,
/// whichever is the larger scale (the looser side fits, the other may overflow).
pub fn fit_loose(m: &[f64; 9], w: f64, h: f64) -> Option<f64> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (mut ex, mut ey) = (0.0f64, 0.0f64);
    for c in [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]] {
        let q = project(m, c[0], c[1])?;
        ex = ex.max(q[0].abs());
        ey = ey.max(q[1].abs());
    }
    let k = (hw / ex.max(1e-9)).max(hh / ey.max(1e-9));
    (k.is_finite() && k > 1e-3).then_some(k)
}

/// Scales `m` down about the centre (never up) until the whole source frame shows.
pub fn fit(m: &[f64; 9], w: f64, h: f64) -> [f64; 9] {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut k: f64 = 1.0;
    for c in [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]] {
        let Some(q) = project(m, c[0], c[1]) else { return *m };
        if q[0].abs() > 1e-9 {
            k = k.min(hw / q[0].abs());
        }
        if q[1].abs() > 1e-9 {
            k = k.min(hh / q[1].abs());
        }
    }
    if k.is_finite() && k > 1e-3 && k < 1.0 { mul(&[k, 0.0, 0.0, 0.0, k, 0.0, 0.0, 0.0, 1.0], m) } else { *m }
}

/// Checks Geometry settings from untrusted input: a known Upright mode, at most
/// [`MAX_GUIDES`] guides with finite points in 0 … 1, finite sliders.
pub fn validate(p: &CameraRaw) -> Result<(), String> {
    if Upright::parse(&p.perspective_upright).is_none() {
        return Err("perspectiveUpright must be off, auto, level, vertical, full or guided".into());
    }
    if p.upright_guides.len() > MAX_GUIDES {
        return Err(format!("at most {MAX_GUIDES} upright guides"));
    }
    if !p.upright_guides.iter().flatten().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
        return Err("upright guide points must be finite and in 0..=1".into());
    }
    let sliders = [p.perspective_vertical, p.perspective_horizontal, p.perspective_rotate, p.perspective_aspect, p.perspective_scale, p.perspective_x, p.perspective_y];
    if !sliders.iter().all(|v| v.is_finite()) {
        return Err("Transform sliders must be finite".into());
    }
    Ok(())
}

/// The settings' Geometry is neutral (no Upright, no Transform, no Constrain Crop effect).
pub fn neutral(p: &CameraRaw) -> bool {
    matches!(Upright::parse(&p.perspective_upright), None | Some(Upright::Off)) && Transform::of(p).is_identity()
}

/// The whole Geometry homography (centred source → result) for `p` on these pixels.
pub fn homography(px: &[[f32; 4]], w: usize, h: usize, p: &CameraRaw) -> [f64; 9] {
    let (fw, fh) = (w as f64, h as f64);
    let mode = Upright::parse(&p.perspective_upright).unwrap_or(Upright::Off);
    let up = match mode {
        Upright::Off => None,
        Upright::Guided => upright::guided(&p.upright_guides, fw, fh),
        m => upright::analyse(px, w, h, m),
    };
    // Upright's zoom, measured on Photoshop (a facade tilted 13 ways): Level fills the frame when
    // that takes at most 1.10, else stays at 100 %; the other modes fit the picture's box along
    // its looser side (width for a turned picture, height for a keystone) and fill the frame
    // instead when that is at most 10 % more. The manual sliders go on top.
    let up = up
        .map(|u| recentre(&u, fw, fh))
        .map(|u| {
            let fill = fill_factor(&u, fw, fh).filter(|k| *k > 1e-3).map(|k| 1.0 / k);
            if mode == Upright::Level {
                return match fill {
                    Some(z) if z <= upright::MAX_FILL => constrain(&u, fw, fh),
                    _ => u,
                };
            }
            let loose = fit_loose(&u, fw, fh);
            match (fill, loose) {
                (Some(z), Some(k)) if z / k <= upright::MAX_FILL => constrain(&u, fw, fh),
                (_, Some(k)) => mul(&[k, 0.0, 0.0, 0.0, k, 0.0, 0.0, 0.0, 1.0], &u),
                (Some(z), None) if z <= upright::MAX_FILL => constrain(&u, fw, fh),
                _ => u,
            }
        })
        .unwrap_or(IDENTITY);
    let m = mul(&manual(&Transform::of(p), fw, fh), &up);
    let m = if p.constrain_crop { constrain(&m, fw, fh) } else { m };
    normalized(m)
}

/// Catmull-Rom weights for offset `t` in 0..1.
fn cubic(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

/// Premultiplied RGBA at `(x, y)` (pixel-centre coordinates: pixel `i` spans `i … i+1`),
/// bicubic, transparent outside the image.
fn sample(src: &[[f32; 4]], w: usize, h: usize, x: f64, y: f64) -> [f32; 4] {
    let (fx, fy) = (x - 0.5, y - 0.5);
    if !(fx > -2.0 && fy > -2.0 && fx < w as f64 + 1.0 && fy < h as f64 + 1.0) {
        return [0.0; 4];
    }
    let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
    let (wx, wy) = (cubic((fx - ix as f64) as f32), cubic((fy - iy as f64) as f32));
    let mut o = [0.0f32; 4];
    for j in 0..4 {
        let yy = iy - 1 + j as i64;
        if yy < 0 || yy >= h as i64 {
            continue;
        }
        let row = yy as usize * w;
        for i in 0..4 {
            let xx = ix - 1 + i as i64;
            if xx < 0 || xx >= w as i64 {
                continue;
            }
            let Some(q) = src.get(row + xx as usize) else { continue };
            let k = wx[i] * wy[j];
            let a = q[3];
            o[0] += k * q[0] * a;
            o[1] += k * q[1] * a;
            o[2] += k * q[2] * a;
            o[3] += k * a;
        }
    }
    o
}

/// Warps straight RGBA pixels in place by `m` (centred source → result coordinates). Uncovered
/// pixels become transparent; a shrinking warp averages several samples so it does not alias.
pub fn warp(px: &mut [[f32; 4]], w: usize, h: usize, m: &[f64; 9], float: bool) {
    if px.len() < w.saturating_mul(h) || w == 0 || h == 0 {
        return;
    }
    let Some(inv) = Homography(*m).inverse() else { return };
    let inv = inv.0;
    let src = px.to_vec();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let back = |x: f64, y: f64| -> Option<[f64; 2]> {
        let (u, v) = (x - cx, y - cy);
        let wv = inv[6] * u + inv[7] * v + inv[8];
        (wv > 1e-9).then(|| [(inv[0] * u + inv[1] * v + inv[2]) / wv + cx, (inv[3] * u + inv[4] * v + inv[5]) / wv + cy])
    };
    par_rows(&mut px[..w * h], w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let (ox, oy) = (x as f64 + 0.5, y as f64 + 0.5);
            let Some(s) = back(ox, oy) else {
                *q = [0.0; 4];
                continue;
            };
            // Footprint of one result pixel in the source: more samples when it shrinks.
            let foot = match (back(ox + 1.0, oy), back(ox, oy + 1.0)) {
                (Some(a), Some(b)) => ((a[0] - s[0]).hypot(a[1] - s[1])).max((b[0] - s[0]).hypot(b[1] - s[1])),
                _ => 1.0,
            };
            let n = if foot.is_finite() { (foot.ceil() as usize).clamp(1, 4) } else { 1 };
            let mut acc = [0.0f32; 4];
            for j in 0..n {
                for i in 0..n {
                    let (dx, dy) = ((i as f64 + 0.5) / n as f64 - 0.5, (j as f64 + 0.5) / n as f64 - 0.5);
                    let Some(t) = back(ox + dx, oy + dy) else { continue };
                    let v = sample(&src, w, h, t[0], t[1]);
                    for c in 0..4 {
                        acc[c] += v[c];
                    }
                }
            }
            let inv_n = 1.0 / (n * n) as f32;
            let a = (acc[3] * inv_n).clamp(0.0, 1.0);
            if a <= 1e-6 {
                *q = [0.0; 4];
                continue;
            }
            for c in 0..3 {
                let v = acc[c] * inv_n / a;
                q[c] = if float { v } else { v.clamp(0.0, 1.0) };
            }
            q[3] = a;
        }
    });
}

/// Camera Raw's Geometry on straight RGBA pixels (the develop pipeline's step before Effects).
pub fn apply(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    if neutral(p) || px.is_empty() {
        return;
    }
    let m = homography(px, w, h, p);
    if m.iter().zip(IDENTITY.iter()).all(|(a, b)| (a - b).abs() < 1e-12) {
        return;
    }
    warp(px, w, h, &m, float);
}

#[cfg(test)]
mod tests;
