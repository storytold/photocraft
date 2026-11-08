//! A smart object under a one-way perspective (its corners moved so that only one direction
//! converges: Perspective dragged sideways or up and down, with any scale or skew), drawn as the
//! reference application draws one. Measured byte for byte at 8 bits against its renders
//! (horizontal and vertical perspectives growing and shrinking, both edges, impulse probes at
//! every phase), and every one matches:
//!
//! 1. **Bleed** as for scaling ([`crate::smart_scale`]).
//! 2. **The projective direction first:** pass 1 runs along the direction in which the map is
//!    projective (rows when the vanishing point is to a side, else columns, by transposing), pass 2
//!    along the other, where every line is a straight walk.
//! 3. **Pass 1:** each document pixel samples the source line at the exact inverse of its centre,
//!    rounded to the nearest 1/128 sample. Within half a sample of the line's ends the position is
//!    held at the end sample's centre; further out the pixel is transparent. Bicubic Automatic for
//!    the whole quad: Sharper (Keys −1, stretch 1.05) when the quad is smaller than the contents,
//!    else Smoother (Keys −0.625, stretch 1.15); 14-bit weights.
//! 4. **Pass 2:** each document column is the affine walk ([`crate::smart_affine`]) with that
//!    column's own top and length, plain Bicubic, no averaging.
//! 5. **Clip:** the corners' rounded bounding box.
//!
//! **Every other quad** (Perspective both ways, Distort, a perspective with scale, skew or
//! rotation, or turned afterwards) is drawn in two such stages, measured byte for byte on the
//! reference's impulse probes and renders (31 band probes, 4 dot probes, 5 pictures):
//!
//! 1. In coordinates centred on the contents' centre and on its image the map is
//!    `hc = [[A, t], [g, h, 1]]`, which factors exactly into a side one-way stage
//!    `[[s, 0, tx], [c', d, 0], [g1, 0, 1]]` (scale, vertical shear, horizontal perspective)
//!    followed by a down one `[[1, b2, 0], [0, 1, ty], [0, h2, 1]]` (horizontal shear, vertical
//!    perspective), or the same with x and y swapped ([`stages`]).
//! 2. Of the two orders the one whose intermediate image is smaller is used (ties: horizontal
//!    first). The intermediate is stage 1 stretched along stage 2's walk axis by stage 2's largest
//!    walk magnification (`1 / (1 + h2·y)` over stage 1's corners, scale included) and along its
//!    projective axis to the final quad's height; its left edge lands on the whole pixel below,
//!    its top exactly on the final quad's top. Stage 2 then draws the intermediate (cropped to its
//!    coverage, its colours under transparency kept, not bled again) with the same one-way rules;
//!    when stage 2 has no perspective it is the exact
//!    affine walk ([`crate::smart_affine`]), and when it would be a plain translation there is
//!    no second stage at all.
//! 3. Kernels: Automatic for stage 1 by the stretched intermediate's width against the contents'
//!    (strictly wider: Smoother, else Sharper; the reference decides this tie by its own last
//!    bits), Sharper for stage 2. The two kernels compose into the wide, two-humped weights the
//!    reference shows, and the first kernel's phase-dependent centroid, averaged by the second,
//!    into its 1/20-pixel ripples.
//!
//! The intermediate has the document's depth (8-bit documents keep an 8-bit intermediate).

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::smart_affine::{r256, round_half_up, transpose, walk, weight_table};
use crate::smart_scale::{Depth, Image, MAX_SAMPLES, ONE, bleed, keys};

/// Taps of the projective pass: `floor(u) − 3 ..= floor(u) + 3` (Smoother reaches 2.3 samples).
const TAPS: usize = 7;
/// Below this a perspective coefficient counts as zero.
const FLAT: f64 = 1e-9;

/// 14-bit weights of the [`TAPS`] taps for each of the 128 phases, for Keys `a` stretched by `st`.
fn table(a: f64, st: f64) -> Vec<[i64; TAPS]> {
    (0..128)
        .map(|ph| {
            let u = f64::from(ph) / 128.0;
            let w: Vec<f64> = (0..TAPS).map(|t| keys((u + 3.0 - t as f64) / st, a)).collect();
            let total: f64 = w.iter().sum();
            let mut q = [0i64; TAPS];
            if total.abs() < 1e-12 {
                return q;
            }
            for (qq, ww) in q.iter_mut().zip(&w) {
                *qq = (ww / total * ONE as f64).trunc() as i64;
            }
            let mut big = 0;
            for (i, v) in w.iter().enumerate() {
                if w.get(big).is_some_and(|b| v > b) {
                    big = i;
                }
            }
            let others: i64 = q.iter().enumerate().filter(|&(i, _)| i != big).map(|(_, v)| v).sum();
            if let Some(b) = q.get_mut(big) {
                *b = ONE - others;
            }
            q
        })
        .collect()
}

/// Pass 1 (rule 3): row `j` of `img` onto the document columns `x0 .. x0 + count`, the map along
/// the row being `x ↦ (h0 x + h1 y + h2) / (h6 x + 1)` at `y = j + ½`.
fn projective(img: &Image, h: &[f64; 9], x0: i64, count: usize, depth: Depth, table: &[[i64; TAPS]]) -> Option<Image> {
    let (n, nc) = (img.w, img.n);
    let alpha = nc - 1;
    let last = n as i64 - 1;
    let nf = n as f64;
    let mut px = vec![0.0f32; count.checked_mul(img.h)?.checked_mul(nc)?];
    crate::par::chunks_mut(&mut px, count * nc, true, |j, row| {
        let Some(line) = img.px.get(j * n * nc..(j + 1) * n * nc) else { return };
        let y = j as f64 + 0.5;
        let mut acc = vec![0.0f64; nc];
        for (k, out) in row.chunks_exact_mut(nc).enumerate() {
            let xc = (x0 + k as i64) as f64 + 0.5;
            let den = h[0] - xc * h[6];
            let pos = (xc * (h[7] * y + 1.0) - h[1] * y - h[2]) / den - 0.5;
            if !(den > 0.0 && pos.is_finite() && pos >= -0.5 && pos < nf - 0.5) {
                continue; // transparent
            }
            let ph = (pos.clamp(0.0, last as f64) * 128.0 + 0.5).floor() as i64;
            let base = ph.div_euclid(128) - 3;
            let Some(q) = table.get(ph.rem_euclid(128) as usize) else { continue };
            acc.iter_mut().for_each(|v| *v = 0.0);
            for (t, &qq) in q.iter().enumerate() {
                if qq == 0 {
                    continue;
                }
                let i = base + t as i64;
                let inside = (0..=last).contains(&i);
                let at = i.clamp(0, last.max(0)) as usize * nc;
                let Some(sample) = line.get(at..at + nc) else { continue };
                for (ch, v) in sample.iter().enumerate() {
                    if ch == alpha && !inside {
                        continue;
                    }
                    if let Some(a) = acc.get_mut(ch) {
                        *a += f64::from(*v) * qq as f64;
                    }
                }
            }
            for (ch, v) in out.iter_mut().enumerate() {
                *v = depth.settle(acc.get(ch).copied().unwrap_or(0.0), ch == alpha);
            }
        }
    });
    Some(Image { w: count, h: img.h, n: nc, px })
}

/// The same map with x and y swapped (contents and document both transposed).
fn transposed(h: &[f64; 9]) -> [f64; 9] {
    [h[4], h[3], h[5], h[1], h[0], h[2], h[7], h[6], h[8]]
}

/// Bicubic Automatic's choice for a pass whose output spans `out` pixels where its input spans
/// `inp`: Smoother when the pass enlarges, else Sharper. The comparison is strict and has no
/// tolerance: the reference decides from the same floating-point extent, so a Distort that keeps
/// the width to within 1e-13 draws Smoother or Sharper by the sign of that residue (a saved one
/// redrawn from its stored corners comes out the same way).
fn automatic(out: f64, inp: f64) -> (f64, f64) {
    if out > inp { SMOOTHER } else { SHARPER }
}

/// Keys kernels (a, stretch) of Bicubic Sharper and Smoother at 1:1.
const SHARPER: (f64, f64) = (-1.0, 1.05);
const SMOOTHER: (f64, f64) = (-0.625, 1.15);

/// Draws `img` through `h` (h6 ≠ 0, h7 = 0, h8 = 1): the result and its top-left corner. `kernel`
/// for the projective pass, or Automatic by the quad's extent along it.
fn draw(img: &Image, h: &[f64; 9], depth: Depth, kernel: Option<(f64, f64)>) -> Option<(Image, i64, i64)> {
    let (wf, hf) = (img.w as f64, img.h as f64);
    let map = |x: f64, y: f64| {
        let w = h[6] * x + h[7] * y + 1.0;
        ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w, w)
    };
    let corners = [map(0.0, 0.0), map(wf, 0.0), map(wf, hf), map(0.0, hf)];
    if corners.iter().any(|&(x, y, w)| !(w > 0.0 && x.is_finite() && y.is_finite())) {
        return None;
    }
    let lo = |f: fn(&(f64, f64, f64)) -> f64| corners.iter().map(f).fold(f64::INFINITY, f64::min);
    let hi = |f: fn(&(f64, f64, f64)) -> f64| corners.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
    let (x0, x1) = (round_half_up(lo(|c| c.0)), round_half_up(hi(|c| c.0)));
    let (y0, y1) = (round_half_up(lo(|c| c.1)), round_half_up(hi(|c| c.1)));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (nx, ny) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if nx.max(img.w).checked_mul(ny.max(img.h))?.checked_mul(img.n)? > MAX_SAMPLES {
        return None;
    }
    // Rule 3's Automatic: the quad's extent along the projective direction against the contents'.
    let (a, st) = kernel.unwrap_or_else(|| automatic(hi(|c| c.0) - lo(|c| c.0), wf));
    let inter = projective(img, h, x0, nx, depth, &table(a, st))?;
    // Pass 2: column X samples source x = p y + q; along it y ↦ (A y + B) / Dd (h7 = 0).
    let mut dist0 = Vec::with_capacity(nx);
    let mut lengths = Vec::with_capacity(nx);
    for i in 0..nx {
        let xc = (x0 + i as i64) as f64 + 0.5;
        let den = h[0] - xc * h[6];
        let (p, q) = (-h[1] / den, (xc - h[2]) / den);
        let (aa, bb, dd) = (h[3] * p + h[4], h[3] * q + h[5], h[6] * q + 1.0);
        if !(den > 0.0 && dd > 0.0 && aa > 0.0) {
            return None;
        }
        dist0.push(y0 * 256 - r256(bb / dd));
        lengths.push(r256(hf * aa / dd));
    }
    let out = transpose(&walk(&transpose(&inter), &dist0, ny, &lengths, depth, &weight_table())?);
    Some((out, x0, y0))
}

/// The pixels of `src` inside `bounds` (its contents' canvas; `src` has alpha) drawn through the
/// homography `h` (row-major 3×3, contents → document, `x, y` measured from `bounds`' corner).
/// `None` when the map isn't a one-way perspective (affine, or converging both ways), folds over,
/// or the size passes [`MAX_SAMPLES`].
pub fn render(src: &Surface, bounds: Rect, h: [f64; 9]) -> Option<Surface> {
    render_with(src, bounds, h, None, true)
}

/// [`render`] with the projective pass's kernel fixed (`None`: Automatic) and the bleed optional
/// (a cascade's intermediate keeps its computed colours), for the cascade's stages. A map that
/// is neither a one-way perspective nor a general quad is `None`.
fn render_with(src: &Surface, bounds: Rect, h: [f64; 9], kernel: Option<(f64, f64)>, bleed_contents: bool) -> Option<Surface> {
    let fmt = src.format();
    let nc = src.channels();
    if !fmt.alpha || nc == 0 || bounds.is_empty() || h.iter().any(|v| !v.is_finite()) || h[8].abs() < FLAT {
        return None;
    }
    let h = h.map(|v| v / h[8]);
    // One-way: converging along rows only with each document column a straight walk (h7 = h1 =
    // 0), or the same along columns (h6 = h3 = 0).
    let flat = |v: f64| v.abs() <= FLAT;
    let side = !flat(h[6]) && flat(h[7]) && flat(h[1] / h[0].abs().max(FLAT));
    let down = !flat(h[7]) && flat(h[6]) && flat(h[3] / h[4].abs().max(FLAT));
    if !(side || down) {
        // Any other perspective: two one-way stages (an affine map is for `smart_affine`).
        return if flat(h[6]) && flat(h[7]) || kernel.is_some() { None } else { cascade(src, bounds, &h) };
    }
    let depth = Depth::of(fmt.sample);
    let unit = depth.levels.unwrap_or(1.0);
    let (w, hgt) = (bounds.width() as usize, bounds.height() as usize);
    w.checked_mul(hgt)?.checked_mul(nc)?.le(&MAX_SAMPLES).then_some(())?;
    let mut px = src.read_region(bounds);
    if depth.levels.is_some() {
        crate::par::chunks_mut(&mut px, 1 << 16, true, |_, part| {
            for v in part {
                *v = (v.clamp(0.0, 1.0) * unit).round();
            }
        });
    }
    let mut img = Image { w, h: hgt, n: nc, px };
    // The bleed's tie rule is not symmetric, so a down perspective bleeds in its own (transposed)
    // frame, as the reference does.
    let (out, x0, y0) = if side {
        if bleed_contents {
            bleed(&mut img);
        }
        draw(&img, &h, depth, kernel)?
    } else {
        let mut t = transpose(&img);
        if bleed_contents {
            bleed(&mut t);
        }
        let (out, y0, x0) = draw(&t, &transposed(&h), depth, kernel)?;
        (transpose(&out), x0, y0)
    };
    let mut surface = Surface::new(fmt);
    let mut data = out.px;
    crate::par::chunks_mut(&mut data, 1 << 16, true, |_, part| {
        for v in part {
            *v /= unit;
        }
    });
    let rect = Rect::new(i32::try_from(x0).ok()?, i32::try_from(y0).ok()?, i32::try_from(x0 + out.w as i64).ok()?, i32::try_from(y0 + out.h as i64).ok()?);
    surface.write_region_par(rect, &data);
    Some(surface)
}

fn mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    m
}

fn inverse(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let inv = [e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d];
    Some(inv.map(|v| v / det))
}

fn translation(x: f64, y: f64) -> [f64; 9] {
    [1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0, 1.0]
}

/// The map's image of the box `w × h`: the four corners, or `None` past the horizon.
fn corners(m: &[f64; 9], w: f64, h: f64) -> Option<[(f64, f64); 4]> {
    let mut out = [(0.0, 0.0); 4];
    for (o, (x, y)) in out.iter_mut().zip([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]) {
        let d = m[6] * x + m[7] * y + m[8];
        if !d.is_finite() || d <= 0.0 {
            return None;
        }
        *o = ((m[0] * x + m[1] * y + m[2]) / d, (m[3] * x + m[4] * y + m[5]) / d);
    }
    out.iter().all(|c| c.0.is_finite() && c.1.is_finite()).then_some(out)
}

/// `[x0, y0, x1, y1]` around the corners.
fn extent(c: &[(f64, f64); 4]) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in c {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    [x0, y0, x1, y1]
}

/// The smallest rectangle holding every pixel of `s` with any coverage.
fn alpha_bounds(s: &Surface) -> Option<Rect> {
    let r = s.content_bounds();
    if r.is_empty() {
        return None;
    }
    let nc = s.channels();
    let px = s.read_region(r);
    let w = r.width() as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, p) in px.chunks_exact(nc).enumerate() {
        if p.get(nc - 1).is_some_and(|a| *a > 0.0) {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > 0).then(|| Rect::new(r.x0 + x0 as i32, r.y0 + y0 as i32, r.x0 + x1 as i32, r.y0 + y1 as i32))
}

/// The two one-way stages of a general quad in coordinates centred on the contents' centre and
/// its image: `hc = stage2 · stage1` with `stage1 = [[s, 0, tx], [c', d, 0], [g1, 0, 1]]` (a side
/// perspective with any scale and vertical shear) and `stage2 = [[1, b2, 0], [0, 1, ty], [0, h2, 1]]`
/// (a down perspective with a horizontal shear). Returns `(stage1, stage2, g1, h2)`.
fn stages(hc: &[f64; 9]) -> Option<([f64; 9], [f64; 9], f64, f64)> {
    let [a, b, tx, c, d, ty, g, h, _] = *hc;
    if d.abs() <= FLAT {
        return None;
    }
    let (h2, b2) = (h / d, b / d);
    let den = 1.0 - h2 * ty;
    if den.abs() <= FLAT {
        return None;
    }
    let g1 = (g - h * c / d) / den;
    let cp = c - ty * g1;
    let s = a - b2 * cp;
    let st1 = [s, 0.0, tx, cp, d, 0.0, g1, 0.0, 1.0];
    let st2 = [1.0, b2, 0.0, 0.0, 1.0, ty, 0.0, h2, 1.0];
    Some((st1, st2, g1, h2))
}

/// One orientation's cascade plan: stage 1 in document coordinates, its stretch, the final
/// box, the stage-1 quad's box and the two coefficients.
struct Plan {
    m1: [f64; 9],
    stage2_shear: f64,
    h2: f64,
    sx: f64,
    sy: f64,
    fb: [f64; 4],
    b: [f64; 4],
    area: f64,
}

fn plan(h: &[f64; 9], w: f64, hgt: f64) -> Option<Plan> {
    let flat = |v: f64| v.abs() <= FLAT;
    let (cx, cy) = (w / 2.0, hgt / 2.0);
    let dc = h[6] * cx + h[7] * cy + h[8];
    if !dc.is_finite() || dc <= FLAT {
        return None;
    }
    let (tx, ty) = ((h[0] * cx + h[1] * cy + h[2]) / dc, (h[3] * cx + h[4] * cy + h[5]) / dc);
    let hc = mul(&mul(&translation(-tx, -ty), h), &translation(cx, cy));
    if hc[8].abs() < FLAT {
        return None;
    }
    let hc = hc.map(|v| v / hc[8]);
    let (st1, st2, _g1, h2) = stages(&hc)?;
    let m1 = mul(&mul(&translation(tx, ty), &st1), &translation(-cx, -cy));
    let q1 = corners(&m1, w, hgt)?;
    let fb = extent(&q1);
    let b = extent(&corners(h, w, hgt)?);
    let (sx, sy) = if flat(h2) {
        (1.0, 1.0)
    } else {
        let sx = q1.iter().map(|c| (1.0 / (1.0 + h2 * (c.1 - ty))).abs()).fold(f64::NEG_INFINITY, f64::max);
        (sx, (b[3] - b[1]) / (fb[3] - fb[1]))
    };
    if !(sx.is_finite() && sy.is_finite() && sx > 0.0 && sy > 0.0) {
        return None;
    }
    let area = sx * (fb[2] - fb[0]) * sy * (fb[3] - fb[1]);
    Some(Plan { m1, stage2_shear: st2[1], h2, sx, sy, fb, b, area })
}

/// `s` with x and y swapped (contents and document), and the box it occupies.
fn transposed_surface(s: &Surface, r: Rect) -> Option<(Surface, Rect)> {
    let nc = s.channels();
    let (w, h) = (r.width() as usize, r.height() as usize);
    w.checked_mul(h)?.checked_mul(nc)?.le(&MAX_SAMPLES).then_some(())?;
    let img = Image { w, h, n: nc, px: s.read_region(r) };
    let t = transpose(&img);
    let tr = Rect::new(r.y0, r.x0, r.y1, r.x1);
    let mut out = Surface::new(s.format());
    out.write_region_par(tr, &t.px);
    Some((out, tr))
}

/// A general quad (see the module docs): two one-way stages through a stretched intermediate.
/// `h` is normalised and measured from `bounds`' corner.
fn cascade(src: &Surface, bounds: Rect, h: &[f64; 9]) -> Option<Surface> {
    let (w, hgt) = (f64::from(bounds.width()), f64::from(bounds.height()));
    let normal = plan(h, w, hgt);
    let mirrored = plan(&transposed(h), hgt, w);
    let (p, turned) = match (normal, mirrored) {
        (Some(n), Some(m)) => {
            if m.area < n.area * (1.0 - 1e-9) { (m, true) } else { (n, false) }
        }
        (Some(n), None) => (n, false),
        (None, Some(m)) => (m, true),
        (None, None) => return None,
    };
    if turned {
        let (ts, tb) = transposed_surface(src, bounds)?;
        let out = cascade_normal(&ts, tb, &transposed(h), &p)?;
        let r = out.content_bounds();
        if r.is_empty() {
            return Some(Surface::new(out.format()));
        }
        return transposed_surface(&out, r).map(|(s, _)| s);
    }
    cascade_normal(src, bounds, h, &p)
}

/// `s` without the rows at or below `end` (document rows; `end` is exclusive).
fn clip_rows_below(s: Surface, end: f64) -> Option<Surface> {
    let r = alpha_bounds(&s)?;
    if !end.is_finite() {
        return Some(s);
    }
    let end = end.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    if end >= r.y1 {
        return Some(s);
    }
    let keep = Rect::new(r.x0, r.y0, r.x1, end.max(r.y0));
    let mut out = Surface::new(s.format());
    if !keep.is_empty() {
        out.write_region_par(keep, &s.read_region(keep));
    }
    Some(out)
}

fn cascade_normal(src: &Surface, bounds: Rect, h: &[f64; 9], p: &Plan) -> Option<Surface> {
    let flat = |v: f64| v.abs() <= FLAT;
    let w = f64::from(bounds.width());
    if flat(p.stage2_shear) && flat(p.h2) {
        // Stage 2 would be a plain translation: the one stage draws the whole map.
        return render_with(src, bounds, *h, None, true);
    }
    // The intermediate: stage 1 stretched along stage 2's walk axis by stage 2's largest walk
    // magnification and along its projective axis to the final box's height; its left edge on the
    // whole pixel below, its top exactly on the final box's top.
    let dc = h[6] * w / 2.0 + h[7] * f64::from(bounds.height()) / 2.0 + h[8];
    let tx = (h[0] * w / 2.0 + h[1] * f64::from(bounds.height()) / 2.0 + h[2]) / dc;
    let left = tx + p.sx * (p.fb[0] - tx);
    let s = [p.sx, 0.0, tx - p.sx * tx + left.floor() - left, 0.0, p.sy, if flat(p.h2) { 0.0 } else { p.b[1] - p.sy * p.fb[1] }, 0.0, 0.0, 1.0];
    let m1s = mul(&s, &p.m1);
    let k1 = automatic(p.sx * (p.fb[2] - p.fb[0]), w);
    let inter = if flat(m1s[6]) && flat(m1s[7]) {
        crate::smart_affine::render(src, bounds, [m1s[0], m1s[3], m1s[1], m1s[4], m1s[2], m1s[5]])?
    } else {
        render_with(src, bounds, m1s, Some(k1), true)?
    };
    // The intermediate lives in a buffer of ceil(height) rows from the whole pixel above the
    // stretched quad's top (floor), while its rows are drawn from the nearest pixel edge (round):
    // when the top's fraction passes a half and the bottom's does not exceed it, the bottom row of
    // the drawn range falls outside the buffer and is lost. The reference loses it too (its
    // distort-everything references have no bottom row; a one-way draw keeps its last row).
    let (top, bottom) = (p.b[1], p.b[3]);
    let buffer_end = top.floor() + (bottom - top).ceil();
    let inter = clip_rows_below(inter, buffer_end)?;
    let ib = alpha_bounds(&inter)?;
    let m2 = mul(&mul(h, &inverse(&m1s)?), &translation(f64::from(ib.x0), f64::from(ib.y0)));
    if m2[8].abs() <= FLAT {
        return None;
    }
    let m2 = m2.map(|v| v / m2[8]);
    if flat(m2[6]) && flat(m2[7]) {
        // Stage 2 is a horizontal shear (and the translation): the exact affine walk.
        return crate::smart_affine::render_with(&inter, ib, [m2[0], m2[3], m2[1], m2[4], m2[2], m2[5]], false);
    }
    render_with(&inter, ib, m2, Some(SHARPER), false)
}

/// `h` (row-major, in the contents' own coordinates) measured instead from the point `(x, y)`, the
/// corner of the box [`render`] takes.
pub fn from_corner(h: [f64; 9], x: f64, y: f64) -> [f64; 9] {
    let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = h;
    [h0, h1, h0 * x + h1 * y + h2, h3, h4, h3 * x + h4 * y + h5, h6, h7, h6 * x + h7 * y + h8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projective_weights_sum_to_one() {
        for (a, st) in [(-1.0, 1.05), (-0.625, 1.15)] {
            for q in table(a, st) {
                assert_eq!(q.iter().sum::<i64>(), ONE);
            }
        }
    }

    #[test]
    fn affine_and_two_way_maps_are_left_to_other_paths() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 4, 4), &[1.0; 64]);
        let r = Rect::new(0, 0, 4, 4);
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_none());
        // Converging both ways about a point other than the contents' centre (an affine part): the cascade.
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.01, 1.0]).is_some());
        // The same about the centre: the two-stage path.
        let about_centre = mul(&mul(&translation(10.0, 10.0), &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.005, 1.0]), &translation(-2.0, -2.0));
        assert!(render(&s, r, about_centre).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.01, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, f64::NAN, 0.0, 1.0]).is_none());
        // A one-way perspective turned afterwards: two stages, the second an affine shear.
        assert!(render(&s, r, [0.94, -0.34, 0.0, 0.34, 0.94, 0.0, 0.01, 0.0, 1.0]).is_some());
        assert!(render(&s, r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0]).is_none());
    }

    /// A 128 px grey field (96) with a 224 dot every 16 px from (8, 8): the reference's impulse
    /// probe, whose renders give the expected values below.
    fn dots() -> Surface {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut data = vec![0.0f32; 128 * 128 * 4];
        for (i, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 128, i / 128);
            let v = if x % 16 == 8 && y % 16 == 8 { 224.0 } else { 96.0 };
            px.copy_from_slice(&[v / 255.0, v / 255.0, v / 255.0, 1.0]);
        }
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 128, 128), &data);
        s
    }

    fn grey_patch(s: &Surface, x: i32, y: i32) -> [[u8; 3]; 3] {
        let px = s.read_region(Rect::new(x - 1, y - 1, x + 2, y + 2));
        let mut out = [[0u8; 3]; 3];
        for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
            assert!(p[3] > 0.999, "opaque at ({x}, {y})");
            out[i / 3][i % 3] = (p[0] * 255.0).round() as u8;
        }
        out
    }

    /// The reference's render of the dots moved by (30, 10) under Perspective 0.35 % / 0.2 %:
    /// the vertical stage goes first here (its intermediate is the smaller), although the
    /// horizontal coefficient is larger.
    #[test]
    fn automatic_has_no_tolerance_at_the_tie() {
        // The reference decides from the same floating-point extent: a Distort whose stored corners
        // keep the width to within 1e-12 draws Smoother, one that keeps it exactly draws Sharper.
        assert_eq!(automatic(128.0 * (1.0 + 9e-13), 128.0), SMOOTHER);
        assert_eq!(automatic(128.0, 128.0), SHARPER);
        assert_eq!(automatic(128.0 * (1.0 - 9e-13), 128.0), SHARPER);
    }

    /// distort-everything's map: the stretched quad spans y 102.556..249.555, so its intermediate's
    /// buffer (floor(top) + ceil(height) = 249) ends a row before the drawn range (round: 250) and the
    /// final quad's bottom row 249 stays empty, as in the reference, although the quad's lowest corner
    /// (249.555) lies below that row's centre.
    #[test]
    fn a_cascade_loses_the_bottom_row_its_intermediate_buffer_cannot_hold() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let h = [0.5001371796382342, 0.024111162934328828, 101.48460242572091, -0.2210246357031403, 1.0279091153627808, 102.55647359375378, -0.003193430656934307, 0.0018248175182481743, 1.0];
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 128, 128), &vec![1.0; 128 * 128 * 4]);
        let out = render(&s, Rect::new(0, 0, 128, 128), h).expect("a general quad");
        let row = |y: i32| out.read_region(Rect::new(180, y, 230, y + 1)).as_chunks::<4>().0.iter().map(|p| p[3]).fold(0.0f32, f32::max);
        assert_eq!(row(249), 0.0, "row 249 must be empty");
        assert!(row(248) > 0.9, "row 248 holds the quad's tip");
        assert_eq!(alpha_bounds(&out), Some(Rect::new(85, 103, 280, 249)));
    }

    #[test]
    fn a_moved_two_way_perspective_matches_the_reference_byte_for_byte() {
        let h = [2.4074074074099214, 0.49382716049342734, 107.53086419753102, 0.864197530867415, 2.037037037036821, 76.6666666666538, 0.005401234567917046, 0.0030864197530817293, 1.0];
        let out = render(&dots(), Rect::new(0, 0, 128, 128), h).expect("drawn");
        assert_eq!(grey_patch(&out, 184, 126), [[95, 93, 87], [89, 129, 143], [91, 137, 165]]);
        assert_eq!(grey_patch(&out, 196, 177), [[97, 92, 93], [94, 146, 139], [95, 121, 110]]);
    }

    /// The reference's render of the dots in a one-way perspective turned by 20 degrees: stage 1
    /// draws Smoother (its stretched intermediate is wider than the contents) and stage 2 is a
    /// plain horizontal shear through the affine walk.
    #[test]
    fn a_turned_perspective_matches_the_reference_byte_for_byte() {
        let h = [3.015369202285501, -0.34202014332565317, 124.07807401908897, 2.0570322987729495, 0.9396926207829912, 64.76132928581427, 0.009428879310300618, 0.0, 1.0];
        let out = render(&dots(), Rect::new(0, 0, 128, 128), h).expect("drawn");
        assert_eq!(grey_patch(&out, 186, 132), [[97, 96, 96], [103, 94, 89], [91, 189, 104]]);
        assert_eq!(grey_patch(&out, 188, 167), [[96, 95, 96], [106, 140, 120], [96, 91, 90]]);
    }

    /// A down perspective is the side perspective of the transposed contents, bleed included (the
    /// bleed's tie rule is not symmetric, so it has to run in the transposed frame).
    #[test]
    fn a_down_perspective_is_the_transposed_side_perspective() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let mut data = vec![0.0f32; 24 * 20 * 4];
        for (i, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 24, i / 24);
            // A slanted transparent corner over a two-tone field.
            let (c, a) = if x + y < 10 { (0.0, 0.0) } else { (if (x / 3 + y / 5) % 2 == 0 { 0.2 } else { 0.9 }, 1.0) };
            px.copy_from_slice(&[c, c * 0.5, 1.0 - c, a]);
        }
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        s.write_region(Rect::new(0, 0, 24, 20), &data);
        let (ts, tr) = transposed_surface(&s, Rect::new(0, 0, 24, 20)).expect("transposed");
        let down = [1.0, 0.0, 30.0, 0.0, 1.3, 40.0, 0.0, 0.004, 1.0];
        let a = render(&s, Rect::new(0, 0, 24, 20), down).expect("down");
        let b = render(&ts, tr, transposed(&down)).expect("side");
        let (bt, _) = transposed_surface(&b, b.content_bounds()).expect("back");
        let r = a.content_bounds();
        assert_eq!(r, bt.content_bounds());
        assert_eq!(a.read_region(r), bt.read_region(r));
    }

    /// Renders the cases listed in `$PERSP_CASES` (`name source.rgba h0 .. h8` per line, 128 px
    /// 8-bit sources; or `name source.rgba W H auto|sharper|smoother h0 .. h8` for another size
    /// and a fixed projective kernel) into `$PERSP_OUT/<name>.rust.rgba` (320 px, straight 8-bit)
    /// for comparison with the reference's renders.
    #[test]
    #[ignore]
    fn dump_reference_cases() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let (Ok(cases), Ok(out)) = (std::env::var("PERSP_CASES"), std::env::var("PERSP_OUT")) else { return };
        for line in std::fs::read_to_string(cases).expect("cases").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let bytes = std::fs::read(f[1]).expect("source");
            let (w, hgt, kern, hs) = if f.len() == 11 {
                (128, 128, None, &f[2..11])
            } else {
                let k = match f[4] {
                    "sharper" => Some(SHARPER),
                    "smoother" => Some(SMOOTHER),
                    _ => None,
                };
                (f[2].parse().expect("w"), f[3].parse().expect("h"), k, &f[5..14])
            };
            let h: Vec<f64> = hs.iter().map(|v| v.parse().expect("number")).collect();
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
            let data: Vec<f32> = bytes.iter().map(|&b| f32::from(b) / 255.0).collect();
            s.write_region(Rect::new(0, 0, w, hgt), &data);
            let h: [f64; 9] = h.try_into().expect("nine");
            let r = render_with(&s, Rect::new(0, 0, w, hgt), h, kern, true);
            let px = r.map(|r| r.read_region(Rect::new(0, 0, 320, 320))).unwrap_or_else(|| vec![0.0; 320 * 320 * 4]);
            let b: Vec<u8> = px.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect();
            std::fs::write(format!("{out}/{}.rust.rgba", f[0]), b).expect("write");
        }
    }

    /// Bleeds the straight 8-bit RGBA image `$BLEED_IN` (`W H` in `$BLEED_SIZE`) into `$BLEED_OUT`.
    #[test]
    #[ignore]
    fn dump_bleed() {
        let (Ok(inp), Ok(size), Ok(out)) = (std::env::var("BLEED_IN"), std::env::var("BLEED_SIZE"), std::env::var("BLEED_OUT")) else { return };
        let dims: Vec<usize> = size.split_whitespace().map(|v| v.parse().expect("size")).collect();
        let bytes = std::fs::read(inp).expect("input");
        let mut img = Image { w: dims[0], h: dims[1], n: 4, px: bytes.iter().map(|&b| f32::from(b)).collect() };
        bleed(&mut img);
        let b: Vec<u8> = img.px.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
        std::fs::write(out, b).expect("write");
    }

    /// General-quad experiments: each line of `$GEN_CASES` is `name source.rgba depth mode ...`
    /// (128 px 8-bit RGBA source, rendered at `depth` 8 or 16): `direct h0..h8` (the pixel-layer
    /// sampler on the source) or `scaled nw nh ox oy g0..g8` (the smart-object scaling to
    /// `nw x nh` at `(ox, oy)`, then the pixel-layer sampler through `g`). Writes
    /// `$GEN_OUT/<name>.f32` (320 x 320 RGBA, 0..1).
    #[test]
    #[ignore]
    fn dump_general_cases() {
        use crate::transform::{Homography, Interp, warp_surface};
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let (Ok(cases), Ok(out)) = (std::env::var("GEN_CASES"), std::env::var("GEN_OUT")) else { return };
        for line in std::fs::read_to_string(cases).expect("cases").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let bytes = std::fs::read(f[1]).expect("source");
            let sample = if f[2] == "16" { SampleType::U16 } else { SampleType::U8 };
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            let data: Vec<f32> = bytes.iter().map(|&b| f32::from(b) / 255.0).collect();
            s.write_region(Rect::new(0, 0, 128, 128), &data);
            let nums: Vec<f64> = f[4..].iter().map(|v| v.parse().expect("number")).collect();
            let r = match f[3] {
                "direct" => {
                    let h: [f64; 9] = nums[..9].try_into().expect("nine");
                    warp_surface(&s, Rect::new(0, 0, 128, 128), &Homography(h), Interp::Bicubic).expect("warp")
                }
                _ => {
                    let (nw, nh, ox, oy) = (nums[0] as u32, nums[1] as u32, nums[2] as i32, nums[3] as i32);
                    let inter = crate::smart_scale::scale(&s, Rect::new(0, 0, 128, 128), nw, nh, (ox, oy)).expect("scale");
                    let g: [f64; 9] = nums[4..13].try_into().expect("nine");
                    let ib = Rect::new(ox, oy, ox + nw as i32, oy + nh as i32);
                    warp_surface(&inter, ib, &Homography(g), Interp::Bicubic).expect("warp")
                }
            };
            let px = r.read_region(Rect::new(0, 0, 320, 320));
            let b: Vec<u8> = px.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(format!("{out}/{}.f32", f[0]), b).expect("write");
        }
    }
}
