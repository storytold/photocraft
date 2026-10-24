//! Brush tip outlines for the painting cursor: closed rings in units of the brush radius
//! (1 = size / 2), screen-oriented (x right, y down), ready for the UI to scale by the radius.
//! The Pencil's aliased dab is whole pixels instead: [`aliased_outline`] traces them, in pixels.

use std::collections::HashMap;

use photocraft_geom::{Point, Rect};

use crate::Dab;
use crate::brush::{BrushSettings, TipShape};
use crate::render::{BrushContext, rect_around};
use crate::tile::GrayTile;

/// Segments in the outline of a computed round tip.
const ELLIPSE_SEGMENTS: usize = 64;
/// Budget for all traced ring vertices; the smallest rings are dropped first.
const MAX_POINTS: usize = 512;
/// A traced ring smaller than this many bitmap pixels squared is noise.
const MIN_AREA: f32 = 0.9;
/// "Full size" outlines everything above 1/255; the normal cursor outlines the 50 % contour.
const FULL_THRESHOLD: f32 = 1.0 / 255.0;
const HALF_THRESHOLD: f32 = 0.5;
/// The largest side, in pixels, of the box [`aliased_outline`] rasterises one dab in: round tips
/// up to 508 px fit, sampled ones up to about 359 px (they need the corner reach). Tracing it,
/// once per brush change, takes a few milliseconds in a release build, plus the sampled tip's mip
/// pyramid, which grows with its bitmap (about 12 ms for 2500 × 2500 px; each stroke with the tip
/// builds it too). Past the cap the tip's smooth outline is within a pixel of the stair-steps.
const MAX_ALIASED_SIDE: u32 = 512;
/// The most corners an aliased outline may have; a more ragged sampled tip gets none.
const MAX_ALIASED_CORNERS: usize = 1 << 14;
/// Pixel-edge directions, clockwise on screen (y down): east, south, west, north. A right turn
/// is the next one.
const DIRS: [[i64; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];

/// The tip outline as closed rings, or `None` when the tip has no usable shape. `full` outlines
/// the whole tip; otherwise the 50 % contour (the Photoshop normal brush tip cursor).
pub fn tip_outline(b: &BrushSettings, full: bool) -> Option<Vec<Vec<[f32; 2]>>> {
    if !b.size.is_finite() || b.size <= 0.0 || !b.angle.is_finite() || !b.roundness.is_finite() {
        return None;
    }
    let ro = b.roundness.clamp(0.0, 1.0).max(0.5 / (b.size / 2.0)).min(1.0);
    let (sn, cs) = b.angle.to_radians().sin_cos();
    let (fx, fy) = (if b.flip_x { -1.0 } else { 1.0 }, if b.flip_y { -1.0 } else { 1.0 });
    match &b.tip {
        TipShape::Round => {
            // The normal cursor is the 50 % contour, matching the plain-circle path (#2744):
            // the soft tip falls off as (1 - t²)⁴, not the old smoothstep midpoint.
            let f = if full { 1.0 } else { crate::half_coverage_radius(1.0, b.hardness) };
            let ring = (0..ELLIPSE_SEGMENTS)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / ELLIPSE_SEGMENTS as f32;
                    // Tip-local (y up), rotated CCW by the angle, then flipped to screen y down.
                    let (lx, ly) = (t.cos() * f, t.sin() * f * ro);
                    [lx * cs - ly * sn, -(lx * sn + ly * cs)]
                })
                .collect();
            Some(vec![ring])
        }
        TipShape::Sampled(_) | TipShape::Stored(_) => {
            let t = b.tip.bitmap()?;
            if !t.is_valid() {
                return None;
            }
            trace(t, if full { FULL_THRESHOLD } else { HALF_THRESHOLD }, (sn, cs, fx, fy, ro))
        }
    }
}

/// The whole pixels one aliased (Pencil) dab of `b` fills, for its cursor (#5262): closed rings
/// of pixel corners in pixels (x right, y down) around a dab centred at the origin. Each ring
/// keeps the filled pixels on its right, so an outer ring runs clockwise on screen and a hole
/// counter-clockwise; where two filled pixels touch only at a corner, each ring turns there to
/// stay on its own pixel. A dab centred at (x, y) fills the same pixels moved by
/// `grid_center(x, y, b.size) − grid_center(0, 0, b.size)`.
///
/// The rings come from the renderer's own rasterisation of the dab, with the brush made aliased,
/// so they are exactly what it paints: tip, size, roundness, angle, flips and Block's square.
/// What changes with the dab or where it lands (Shape Dynamics, Scattering, Texture, the Dual
/// Brush) is left out: the cursor shows the plain tip. `None` when the tip fills nothing, or is
/// too big (a round tip over 508 px) or too ragged to outline.
pub fn aliased_outline(b: &BrushSettings) -> Option<Vec<Vec<[i32; 2]>>> {
    let mut b = b.bounded_for_render();
    b.aliased = true;
    // The plain tip: Texture and the Dual Brush depend on where the dab lands, and Wet Edges and
    // Noise never change which pixels an aliased dab covers.
    b.texture.enabled = false;
    b.dual_brush.enabled = false;
    b.wet_edges = false;
    b.noise = false;
    let dab = plain_dab(&b);
    // A tip too big to trace is turned away before `BrushContext::new` builds a sampled tip's
    // whole mip pyramid, then by the context's own box in case the two ever differ.
    let fits = |r: Rect| r.width() <= MAX_ALIASED_SIDE && r.height() <= MAX_ALIASED_SIDE;
    if !fits(aliased_dab_rect(&b, &dab)) {
        return None;
    }
    let ctx = BrushContext::new(&b);
    let rect = ctx.dab_rect(&dab, false);
    if !fits(rect) {
        return None;
    }
    let mut cov = Vec::new();
    ctx.rasterize(&dab, false, rect, &mut cov);
    pixel_rings(&cov, rect)
}

/// The dab `DabBuilder::build` makes of `b` without dynamics, at full pressure and flow, centred
/// at the origin.
fn plain_dab(b: &BrushSettings) -> Dab {
    Dab {
        angle: b.angle.to_radians(),
        roundness: b.roundness.clamp(0.01, 1.0),
        flip_x: b.flip_x,
        flip_y: b.flip_y,
        ..Dab::round(Point::new(0.0, 0.0), (b.size / 2.0).max(0.5), 1.0)
    }
}

/// The box `BrushContext::dab_rect` gives `dab`, an aliased dab of `b` (bounded for rendering),
/// without building the context: it reaches the radius from the centre, or for a sampled tip √2
/// times it, the corners of its bitmap turned. Roundness, angle and the bitmap's aspect don't
/// change it.
fn aliased_dab_rect(b: &BrushSettings, dab: &Dab) -> Rect {
    let sampled = b.tip.bitmap().is_some_and(GrayTile::is_valid);
    rect_around(dab, if sampled { dab.radius * std::f32::consts::SQRT_2 } else { dab.radius })
}

/// The boundary of the pixels of `rect` that `cov` (row-major) covers, as the rings
/// [`aliased_outline`] describes: one corner per turn, so collinear edges make one side.
fn pixel_rings(cov: &[f32], rect: Rect) -> Option<Vec<Vec<[i32; 2]>>> {
    let (w, h) = (i64::from(rect.width()), i64::from(rect.height()));
    let filled =
        |x: i64, y: i64| (0..w).contains(&x) && (0..h).contains(&y) && usize::try_from(y * w + x).ok().and_then(|i| cov.get(i)).is_some_and(|c| *c > 0.0);
    // The edges leaving each corner ((w + 1) × (h + 1) of them, row-major): bit k heads DIRS[k].
    let corner = |x: i64, y: i64| usize::try_from(y * (w + 1) + x).ok();
    let mut ways = vec![0u8; corner(w, h)? + 1];
    let mut edges = 0usize;
    for y in 0..h {
        for x in 0..w {
            if !filled(x, y) {
                continue;
            }
            // Each side facing an empty pixel, run with this pixel on its right: the top heads
            // east from the top-left corner, the right side south, the bottom west, the left north.
            for (k, (nx, ny), (cx, cy)) in [(0, (x, y - 1), (x, y)), (1, (x + 1, y), (x + 1, y)), (2, (x, y + 1), (x + 1, y + 1)), (3, (x - 1, y), (x, y + 1))]
            {
                if !filled(nx, ny)
                    && let Some(c) = corner(cx, cy).and_then(|i| ways.get_mut(i))
                {
                    *c |= 1u8 << k;
                    edges += 1;
                }
            }
        }
    }
    let mut used = vec![0u8; ways.len()];
    let (mut rings, mut corners) = (Vec::new(), 0usize);
    for (sx, sy) in (0..=h).flat_map(|y| (0..=w).map(move |x| (x, y))) {
        let start = corner(sx, sy)?;
        for k0 in 0..4usize {
            if ways.get(start).is_none_or(|b| b & (1 << k0) == 0) || used.get(start).is_some_and(|b| b & (1 << k0) != 0) {
                continue;
            }
            // Follow the edges until back on this one; each step takes one, so `edges` bounds
            // the walk. A corner has one way on, or two where filled pixels touch diagonally:
            // there the right turn stays on the pixel this ring follows.
            let (mut x, mut y, mut k) = (sx, sy, k0);
            let (mut ring, mut closed) = (Vec::new(), false);
            for _ in 0..edges {
                *used.get_mut(corner(x, y)?)? |= 1 << k;
                let [dx, dy] = *DIRS.get(k)?;
                (x, y) = (x + dx, y + dy);
                let here = *ways.get(corner(x, y)?)?;
                let next = if here.count_ones() == 1 { here.trailing_zeros() as usize } else { (k + 1) % 4 };
                if here & (1 << next) == 0 {
                    return None;
                }
                if next != k {
                    ring.push([i32::try_from(i64::from(rect.x0) + x).ok()?, i32::try_from(i64::from(rect.y0) + y).ok()?]);
                }
                k = next;
                if (x, y, k) == (sx, sy, k0) {
                    closed = true;
                    break;
                }
            }
            corners += ring.len();
            if !closed || corners > MAX_ALIASED_CORNERS {
                return None;
            }
            rings.push(ring);
        }
    }
    (!rings.is_empty()).then_some(rings)
}

/// Trace the contour(s) above `thr` (resampled ~2x, capped at 160 px a side) and map them with
/// the same tip transform as `render::BrushContext::rasterize`.
fn trace(t: &GrayTile, thr: f32, xf: (f32, f32, f32, f32, f32)) -> Option<Vec<Vec<[f32; 2]>>> {
    let (sn, cs, fx, fy, ro) = xf;
    let (tw, th) = (t.width as f32, t.height as f32);
    let big = tw.max(th);
    let f = if big <= 160.0 { 2.0 } else { 160.0 / big };
    let (gw, gh) = (((tw * f).ceil() as usize).max(1), ((th * f).ceil() as usize).max(1));
    let mut vals = vec![0.0f32; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let (px, py) = ((gx as f32 + 0.5) / f - 0.5, (gy as f32 + 0.5) / f - 0.5);
            vals[gy * gw + gx] = sample(t, px, py);
        }
    }
    let mut rings: Vec<Vec<[f32; 2]>> = march(&vals, gw, gh, thr).into_iter().filter(|r| area(r) >= MIN_AREA * f * f).collect();
    if rings.is_empty() {
        return None;
    }
    rings.sort_by(|a, b| area(b).total_cmp(&area(a)));
    let mut tol = 0.5;
    let mut thin: Vec<Vec<[f32; 2]>>;
    loop {
        thin = rings.iter().map(|r| simplify(r, tol)).collect();
        if thin.iter().map(Vec::len).sum::<usize>() <= MAX_POINTS || tol >= 8.0 {
            break;
        }
        tol *= 1.5;
    }
    let mut budget = MAX_POINTS;
    thin.retain(|r| {
        let keep = r.len() >= 3 && r.len() <= budget;
        if keep {
            budget -= r.len();
        }
        keep
    });
    if thin.is_empty() {
        return None;
    }
    let s = 2.0 / big;
    let map = |[gx, gy]: [f32; 2]| {
        let (px, py) = ((gx + 0.5) / f - 0.5, (gy + 0.5) / f - 0.5);
        let (u, v) = ((px - tw / 2.0) * s * fx, -(py - th / 2.0) * s * ro * fy);
        let (ux, uy) = (u * cs - v * sn, u * sn + v * cs);
        [ux, -uy]
    };
    let rings: Vec<Vec<[f32; 2]>> = thin.into_iter().map(|r| r.into_iter().map(map).collect()).collect();
    rings.iter().flatten().all(|p| p[0].is_finite() && p[1].is_finite()).then_some(rings)
}

/// Bilinear sample with zero outside (tips fade to nothing at their edges).
fn sample(t: &GrayTile, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let get = |x: i64, y: i64| if x < 0 || y < 0 || x >= t.width as i64 || y >= t.height as i64 { 0.0 } else { t.get(x as u32, y as u32) };
    let a = get(x0, y0) + (get(x0 + 1, y0) - get(x0, y0)) * tx;
    let b = get(x0, y0 + 1) + (get(x0 + 1, y0 + 1) - get(x0, y0 + 1)) * tx;
    a + (b - a) * ty
}

/// Shoelace area of a ring.
fn area(r: &[[f32; 2]]) -> f32 {
    if r.len() < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    (s * 0.5).abs()
}

/// Link segments end-to-end into closed rings (quantised endpoint keys absorb float noise).
fn stitch(segs: Vec<([f32; 2], [f32; 2])>) -> Vec<Vec<[f32; 2]>> {
    let key = |p: [f32; 2]| ((p[0] * 4096.0).round() as i64, (p[1] * 4096.0).round() as i64);
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segs.iter().enumerate() {
        ends.entry(key(*a)).or_default().push(i);
        ends.entry(key(*b)).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut rings = Vec::new();
    for s in 0..segs.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut ring = vec![segs[s].0, segs[s].1];
        while let Some(tail) = ring.last().copied() {
            let Some(&next) = ends.get(&key(tail)).and_then(|c| c.iter().find(|&&i| !used[i])) else { break };
            used[next] = true;
            let (a, b) = segs[next];
            let p = if key(a) == key(tail) { b } else { a };
            if key(p) == key(ring[0]) {
                break;
            }
            ring.push(p);
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings
}

/// Marching squares over `v` (w by h samples at lattice nodes), outside treated as 0 so every
/// contour closes; returns rings in node coordinates.
fn march(v: &[f32], w: usize, h: usize, thr: f32) -> Vec<Vec<[f32; 2]>> {
    let val = |x: i64, y: i64| -> f32 { if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 { 0.0 } else { v[y as usize * w + x as usize] } };
    let lerp = |v0: f32, v1: f32, a: (f32, f32), b: (f32, f32)| -> [f32; 2] {
        let t = if (v1 - v0).abs() < 1e-9 { 0.5 } else { ((thr - v0) / (v1 - v0)).clamp(0.0, 1.0) };
        [a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t]
    };
    let mut segs: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for y in -1..h as i64 {
        for x in -1..w as i64 {
            let (va, vb, vc, vd) = (val(x, y), val(x + 1, y), val(x + 1, y + 1), val(x, y + 1));
            let idx = u8::from(va >= thr) | (u8::from(vb >= thr) << 1) | (u8::from(vc >= thr) << 2) | (u8::from(vd >= thr) << 3);
            if idx == 0 || idx == 15 {
                continue;
            }
            let (fx, fy) = (x as f32, y as f32);
            let top = lerp(va, vb, (fx, fy), (fx + 1.0, fy));
            let right = lerp(vb, vc, (fx + 1.0, fy), (fx + 1.0, fy + 1.0));
            let bottom = lerp(vd, vc, (fx, fy + 1.0), (fx + 1.0, fy + 1.0));
            let left = lerp(va, vd, (fx, fy), (fx, fy + 1.0));
            let mid = (va + vb + vc + vd) * 0.25 >= thr;
            match idx {
                1 | 14 => segs.push((top, left)),
                2 | 13 => segs.push((top, right)),
                3 | 12 => segs.push((right, left)),
                4 | 11 => segs.push((right, bottom)),
                6 | 9 => segs.push((top, bottom)),
                7 | 8 => segs.push((left, bottom)),
                5 | 10 => {
                    let opposite = (idx == 5) != mid;
                    if opposite {
                        segs.push((top, right));
                        segs.push((left, bottom));
                    } else {
                        segs.push((top, left));
                        segs.push((right, bottom));
                    }
                }
                _ => {}
            }
        }
    }
    stitch(segs)
}
/// Douglas-Peucker on a closed ring (split at the point farthest from the first).
fn simplify(ring: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    if ring.len() <= 4 {
        return ring.to_vec();
    }
    let (mut far, mut best) = (0, 0.0);
    for (i, p) in ring.iter().enumerate().skip(1) {
        let d = (p[0] - ring[0][0]).powi(2) + (p[1] - ring[0][1]).powi(2);
        if d > best {
            best = d;
            far = i;
        }
    }
    let mut open: Vec<[f32; 2]> = ring[far..].to_vec();
    open.push(ring[0]);
    let mut out = Vec::new();
    dp(&ring[..=far], tol, &mut out);
    out.pop();
    dp(&open, tol, &mut out);
    out.pop();
    out
}

/// Iterative Douglas-Peucker: keep the points of `pts` farther than `tol` from the chord.
fn dp(pts: &[[f32; 2]], tol: f32, out: &mut Vec<[f32; 2]>) {
    let n = pts.len();
    if n == 0 {
        return;
    }
    if n < 3 {
        out.extend_from_slice(pts);
        return;
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let (mut dmax, mut k) = (0.0f32, i);
        for m in i + 1..j {
            let d = seg_dist2(pts[m], pts[i], pts[j]);
            if d > dmax {
                dmax = d;
                k = m;
            }
        }
        if dmax > tol * tol {
            keep[k] = true;
            stack.push((i, k));
            stack.push((k, j));
        }
    }
    for (i, p) in pts.iter().enumerate() {
        if keep[i] {
            out.push(*p);
        }
    }
}

/// Squared distance from `p` to the segment `a`-`b`.
fn seg_dist2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let t = (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / (abx * abx + aby * aby).max(1e-12)).clamp(0.0, 1.0);
    (p[0] - a[0] - abx * t).powi(2) + (p[1] - a[1] - aby * t).powi(2)
}

#[cfg(test)]
mod tests {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    use super::*;

    fn max_radius(rings: &[Vec<[f32; 2]>]) -> f32 {
        rings.iter().flatten().fold(0.0f32, |m, p| m.max(p[0].hypot(p[1])))
    }

    /// The normal cursor of a rotated or flattened round tip is the same 50 % contour as the
    /// plain-circle path (#2744), and the full cursor reaches the whole radius.
    #[test]
    fn round_outlines_sit_on_the_half_coverage_contour() {
        for hardness in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let b = BrushSettings { size: 40.0, angle: 33.0, roundness: 0.4, hardness, ..Default::default() };
            let normal = tip_outline(&b, false).expect("a rotated round tip traces");
            let full = tip_outline(&b, true).expect("a rotated round tip traces");
            assert!((max_radius(&normal) - crate::half_coverage_radius(1.0, hardness)).abs() < 1e-4, "hardness {hardness}");
            assert!((max_radius(&full) - 1.0).abs() < 1e-4, "hardness {hardness}");
        }
    }

    /// A sampled tip traces its 50 % alpha contour, inside the tip's square; a blank tip has none.
    #[test]
    fn sampled_outlines_stay_inside_the_tip_and_blank_tips_have_none() {
        let disc = GrayTile::from_fn(32, 32, |x, y| {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            (1.0 - (dx * dx + dy * dy).sqrt() / 16.0).clamp(0.0, 1.0)
        });
        let b = BrushSettings { size: 20.0, tip: TipShape::Sampled(disc), ..Default::default() };
        let rings = tip_outline(&b, false).expect("a sampled disc traces");
        assert!(rings.iter().all(|r| r.len() >= 3));
        assert!(rings.iter().flatten().all(|p| p[0].is_finite() && p[1].is_finite() && p[0].hypot(p[1]) <= 1.001));
        let blank = GrayTile::from_fn(8, 8, |_, _| 0.0);
        let b = BrushSettings { size: 20.0, tip: TipShape::Sampled(blank), ..Default::default() };
        assert!(tip_outline(&b, false).is_none());
    }

    /// Hostile brush data yields no outline instead of NaNs or a panic.
    #[test]
    fn hostile_settings_get_no_outline() {
        for size in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            let b = BrushSettings { size, angle: 33.0, roundness: 0.5, ..Default::default() };
            assert!(tip_outline(&b, false).is_none(), "size {size}");
        }
        for angle in [f32::NAN, f32::INFINITY] {
            let b = BrushSettings { size: 40.0, angle, roundness: 0.5, ..Default::default() };
            assert!(tip_outline(&b, false).is_none(), "angle {angle}");
        }
    }

    /// Directed unit pixel edges `(x, y, k)`: from corner (x, y) one pixel along `DIRS[k]`, with
    /// the filled pixels on their right.
    type Edges = std::collections::BTreeSet<(i64, i64, usize)>;

    /// The unit edges of `rings` moved by `d` whole pixels. Every side runs along an axis and
    /// every corner turns.
    fn ring_edges(rings: &[Vec<[i32; 2]>], d: [i64; 2]) -> Edges {
        let mut edges = Edges::new();
        for r in rings {
            assert!(r.len() >= 4, "{r:?}");
            let side = |i: usize| {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                let (dx, dy) = (i64::from(b[0] - a[0]), i64::from(b[1] - a[1]));
                let k = DIRS.iter().position(|v| *v == [dx.signum(), dy.signum()]).unwrap_or_else(|| panic!("{a:?} to {b:?} is not along an axis"));
                (k, dx.abs() + dy.abs())
            };
            for i in 0..r.len() {
                let (k, n) = side(i);
                assert_ne!(k, side((i + 1) % r.len()).0, "a corner of {r:?} doesn't turn");
                for j in 0..n {
                    edges.insert((i64::from(r[i][0]) + d[0] + DIRS[k][0] * j, i64::from(r[i][1]) + d[1] + DIRS[k][1] * j, k));
                }
            }
        }
        edges
    }

    /// The unit edges around the pixels one aliased stroke of `b` with the single point `at`
    /// paints into a `fmt` layer, scanned `reach` pixels around it, and how many pixels that is.
    /// Each pixel is painted fully or not at all.
    fn painted(b: &BrushSettings, at: [f64; 2], fmt: PixelFormat, reach: i64) -> (Edges, i64) {
        let b = BrushSettings { aliased: true, hardness: 1.0, flow: 1.0, opacity: 1.0, pressure_size: false, color: [0.0, 0.0, 0.0, 1.0], ..b.clone() };
        let mut s = photocraft_raster::Surface::new(fmt);
        crate::render::render_stroke(&mut s, &b, &[crate::StrokePoint::new(at[0], at[1], 1.0)], None, false, 1.0);
        let filled = |x: i64, y: i64| {
            let a = s.rgba(x as i32, y as i32)[3];
            assert!(a == 0.0 || a == 1.0, "{fmt:?}: partial alpha {a} at ({x}, {y})");
            a == 1.0
        };
        let (cx, cy) = (at[0].floor() as i64, at[1].floor() as i64);
        let (mut edges, mut count) = (Edges::new(), 0);
        for y in cy - reach..=cy + reach {
            for x in cx - reach..=cx + reach {
                if !filled(x, y) {
                    continue;
                }
                count += 1;
                for (k, (nx, ny), (ex, ey)) in
                    [(0, (x, y - 1), (x, y)), (1, (x + 1, y), (x + 1, y)), (2, (x, y + 1), (x + 1, y + 1)), (3, (x - 1, y), (x, y + 1))]
                {
                    if !filled(nx, ny) {
                        edges.insert((ex, ey, k));
                    }
                }
            }
        }
        (edges, count)
    }

    /// The pixels `rings` hold: the area inside the outer rings less the holes.
    fn pixels(rings: &[Vec<[i32; 2]>]) -> i64 {
        let twice: i64 = rings
            .iter()
            .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
            .map(|(a, b)| i64::from(a[0]) * i64::from(b[1]) - i64::from(b[0]) * i64::from(a[1]))
            .sum();
        twice / 2
    }

    /// Where the outline of `b` lands for a dab at `at`, moved by whole pixels as the cursor moves
    /// it, and how many pixels it holds.
    fn placed(b: &BrushSettings, at: [f64; 2]) -> Option<(Edges, i64)> {
        let rings = aliased_outline(b)?;
        let (cx, cy) = crate::grid_center(at[0], at[1], b.size);
        let (ox, oy) = crate::grid_center(0.0, 0.0, b.size);
        let d = [(cx - ox) as i64, (cy - oy) as i64];
        assert_eq!([f64::from(cx - ox), f64::from(cy - oy)], [d[0] as f64, d[1] as f64], "the move is whole pixels");
        Some((ring_edges(&rings, d), pixels(&rings)))
    }

    /// A sampled ring, so the outline has a hole.
    fn ring_tip() -> GrayTile {
        GrayTile::from_fn(24, 24, |x, y| {
            let d = (x as f32 - 11.5).hypot(y as f32 - 11.5) / 12.0;
            if (0.35..0.85).contains(&d) { 1.0 } else { 0.0 }
        })
    }

    /// The outline is exactly what one real dab paints (#5262): from 4 px a round Pencil dab is a
    /// stair-stepped disc, not its square, and roundness, angle, flips, sampled tips and the
    /// Block square shape it. Checked at sub-pixel positions either side of a pixel and of the
    /// origin, on exact pixel edges and centres, at 8, 16 and 32 bits.
    #[test]
    fn aliased_outlines_are_the_pixels_one_dab_paints() {
        let mut brushes: Vec<BrushSettings> = (1..=12).map(|n| BrushSettings { size: n as f32, ..Default::default() }).collect();
        // The radius is half the unrounded size: 2.6 px is a plus, 4.4 px a whole square.
        brushes.extend([0.3, 2.6, 4.4, 7.5].map(|size| BrushSettings { size, ..Default::default() }));
        brushes.extend([
            BrushSettings { size: 9.0, roundness: 0.5, ..Default::default() },
            BrushSettings { size: 9.0, roundness: 0.5, angle: 30.0, flip_x: true, ..Default::default() },
            BrushSettings { size: 8.0, roundness: 0.4, angle: -70.0, flip_y: true, ..Default::default() },
            // So thin and turned 45°: a diagonal of pixels touching only at their corners.
            BrushSettings { size: 9.0, roundness: 0.01, angle: 45.0, ..Default::default() },
            BrushSettings { size: 12.0, tip: TipShape::Sampled(ring_tip()), ..Default::default() },
            BrushSettings { size: 5.0, square: true, ..Default::default() },
            BrushSettings { size: 4.0, square: true, angle: 30.0, roundness: 0.5, ..Default::default() },
        ]);
        let formats = [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F];
        for b in &brushes {
            let what = format!("{} px, roundness {}, angle {}, sampled {}, square {}", b.size, b.roundness, b.angle, b.tip != TipShape::Round, b.square);
            for at in [[20.3, 15.6], [20.8, 15.2], [-7.5, 3.0], [0.0, -0.5], [13.5, 13.49]] {
                let (want, _) = placed(b, at).unwrap_or_else(|| panic!("{what}: no outline"));
                for fmt in formats {
                    assert_eq!(want, painted(b, at, fmt, 16).0, "{what} at {at:?}, {fmt:?}");
                }
            }
        }
        let round = |size: f32| pixels(&aliased_outline(&BrushSettings { size, ..Default::default() }).unwrap_or_default());
        assert_eq!([3.0, 4.0, 5.0, 9.0].map(round), [9, 12, 21, 69]);
        let sampled = aliased_outline(&brushes[brushes.len() - 3]).unwrap_or_default();
        assert!(sampled.len() >= 2, "the ring's hole is traced: {sampled:?}");
        let thin = aliased_outline(&brushes[brushes.len() - 4]).unwrap_or_default();
        assert!(thin.len() > 1 && thin.iter().all(|r| r.len() == 4), "each corner-touching pixel is its own ring: {thin:?}");
    }

    /// Sizes the renderer can't use paint one pixel, so that is the outline; a tip too big to
    /// trace in a frame, or one that paints nothing, has none, and nothing panics.
    #[test]
    fn hostile_or_huge_aliased_tips_get_one_pixel_or_no_outline() {
        for size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -4.0, 0.0] {
            let rings = aliased_outline(&BrushSettings { size, ..Default::default() });
            assert_eq!(rings.as_deref().map(pixels), Some(1), "size {size}");
        }
        assert!(aliased_outline(&BrushSettings { size: 500.0, ..Default::default() }).is_some());
        assert!(aliased_outline(&BrushSettings { size: 5000.0, ..Default::default() }).is_none());
        assert!(aliased_outline(&BrushSettings { size: 9.0, angle: f32::NAN, ..Default::default() }).is_none());
        let blank = GrayTile::from_fn(8, 8, |_, _| 0.0);
        assert!(aliased_outline(&BrushSettings { size: 9.0, tip: TipShape::Sampled(blank), ..Default::default() }).is_none());
        for v in [f32::NAN, f32::INFINITY, -1.0, 1e9] {
            let _ = aliased_outline(&BrushSettings { size: 9.0, roundness: v, angle: v, ..Default::default() });
        }
    }

    /// How far around a dab of `size` to scan for its pixels: past its corners.
    fn reach(size: f32) -> i64 {
        (size.clamp(0.5, 200.0) * 0.75).ceil() as i64 + 3
    }

    /// `None` when the outline of `b` placed at `at` is exactly what one dab there paints into a
    /// `fmt` layer, unit edge for unit edge and pixel for pixel, else what differs. No outline is
    /// right only for a dab that paints nothing.
    fn mismatch(b: &BrushSettings, at: [f64; 2], fmt: PixelFormat) -> Option<String> {
        let what = || {
            let (flips, sampled) = ((b.flip_x, b.flip_y), b.tip != TipShape::Round);
            format!("size {} roundness {} angle {} flips {flips:?} square {} sampled {sampled} at {at:?} {fmt:?}", b.size, b.roundness, b.angle, b.square)
        };
        let (got, count) = painted(b, at, fmt, reach(b.size));
        let Some((want, area)) = placed(b, at) else {
            return (count > 0).then(|| format!("{}: no outline, but the dab paints {count} px", what()));
        };
        if want == got && area == count {
            return None;
        }
        let shared = want.intersection(&got).count();
        Some(format!("{}: the outline holds {area} px ({} edges), the dab paints {count} px ({} edges, {shared} shared)", what(), want.len(), got.len()))
    }

    /// An L-shaped tip, wider than tall: not symmetric about either axis or a diagonal.
    fn l_tip() -> GrayTile {
        GrayTile::from_fn(20, 14, |x, y| if x < 6 || y >= 9 { 1.0 } else { 0.0 })
    }

    /// A soft wedge: its 50 % contour is a slanted edge, so the aliased cut lands between samples.
    fn wedge_tip() -> GrayTile {
        GrayTile::from_fn(17, 23, |x, y| (x as f32 / 16.0 - y as f32 / 22.0 + 0.5).clamp(0.0, 1.0))
    }

    /// Seeded noise: speckles, holes and pixels touching only at their corners.
    fn noise_tip(seed: u64) -> GrayTile {
        GrayTile::from_fn(16, 16, |x, y| {
            let h = ((u64::from(x) * 73_856_093) ^ (u64::from(y) * 19_349_663) ^ seed.wrapping_mul(83_492_791)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            if (h >> 40) % 100 < 55 { 1.0 } else { 0.0 }
        })
    }

    /// A small deterministic generator (SplitMix64) for the fuzz below.
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// In [0, 1).
        fn unit(&mut self) -> f32 {
            (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    /// The footprint doesn't depend on the layer's format: every colour mode, at 8, 16 and 32 bits.
    #[test]
    fn the_outline_is_the_footprint_in_every_colour_mode_and_depth() {
        let brushes = [
            BrushSettings { size: 1.0, ..Default::default() },
            BrushSettings { size: 4.0, ..Default::default() },
            BrushSettings { size: 9.0, roundness: 0.5, angle: 30.0, ..Default::default() },
            BrushSettings { size: 14.0, tip: TipShape::Sampled(l_tip()), angle: 70.0, flip_x: true, ..Default::default() },
            BrushSettings { size: 6.0, square: true, ..Default::default() },
        ];
        let mut fails = Vec::new();
        let modes = [
            ColorMode::Bitmap,
            ColorMode::Grayscale,
            ColorMode::Indexed,
            ColorMode::Rgb,
            ColorMode::Cmyk,
            ColorMode::Lab,
            ColorMode::Multichannel,
            ColorMode::Duotone,
        ];
        for mode in modes {
            for sample in SampleType::ALL {
                let fmt = PixelFormat { mode, sample, alpha: true };
                for b in &brushes {
                    for at in [[20.3, 15.6], [-7.5, 3.0], [13.5, 13.49]] {
                        fails.extend(mismatch(b, at, fmt));
                    }
                }
            }
        }
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    /// Tips with no symmetry, at quarter turns and in between, squashed, flipped either way: the
    /// outline turns and mirrors exactly as the dab does.
    #[test]
    fn asymmetric_sampled_tips_turn_squash_and_flip_like_the_dab() {
        let mut fails = Vec::new();
        for tip in [l_tip(), wedge_tip()] {
            for size in [7.0, 16.0, 33.0] {
                for angle in [0.0, 17.0, 90.0, 135.0, 180.0, 270.0, -60.0] {
                    for roundness in [1.0, 0.6, 0.25] {
                        for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
                            let b = BrushSettings { size, angle, roundness, flip_x, flip_y, tip: TipShape::Sampled(tip.clone()), ..Default::default() };
                            for at in [[20.3, 15.6], [20.8, 15.2], [-3.5, -0.5]] {
                                fails.extend(mismatch(&b, at, PixelFormat::RGBA8));
                            }
                        }
                    }
                }
            }
        }
        assert!(fails.is_empty(), "{} mismatches:\n{}", fails.len(), fails.join("\n"));
    }

    /// A seeded fuzz over size, roundness, angle, flips, the Block square and four tips (round, an
    /// L, a ring and noise), at positions on and between pixel edges and centres.
    #[test]
    fn fuzzed_brushes_outline_exactly_what_they_paint() {
        let mut rng = Rng(0x5262);
        let mut fails = Vec::new();
        for case in 0..600 {
            let tip = match rng.below(6) {
                0 => TipShape::Sampled(l_tip()),
                1 => TipShape::Sampled(ring_tip()),
                2 => TipShape::Sampled(noise_tip(case)),
                _ => TipShape::Round,
            };
            let size = match rng.below(4) {
                0 => (1 + rng.below(12)) as f32,
                1 => 0.3 + rng.unit() * 5.0,
                2 => (2 * (1 + rng.below(20))) as f32 + [0.0, 0.5, 0.49, 0.51][rng.below(4) as usize],
                _ => 1.0 + rng.unit() * 59.0,
            };
            let roundness = if rng.below(3) == 0 { 1.0 } else { 0.01 + rng.unit() * 0.99 };
            let angle = if rng.below(4) == 0 { [0.0, 90.0, -90.0, 45.0, 180.0, 360.0][rng.below(6) as usize] } else { rng.unit() * 800.0 - 400.0 };
            let (flip_x, flip_y, square) = (rng.below(2) == 0, rng.below(2) == 0, rng.below(10) == 0);
            let b = BrushSettings { size, roundness, angle, flip_x, flip_y, square, tip, ..Default::default() };
            let frac = [0.0, 0.5, 0.25, 0.75, 0.999, 0.001, f64::from(rng.unit())];
            let mut coord = || rng.below(80) as f64 - 40.0 + frac[rng.below(frac.len() as u64) as usize];
            let at = [coord(), coord()];
            if let Some(m) = mismatch(&b, at, PixelFormat::RGBA8) {
                fails.push(format!("case {case}: {m}"));
            }
        }
        assert!(fails.is_empty(), "{} of 600 cases:\n{}", fails.len(), fails.join("\n"));
    }

    /// The limit: round and Block tips up to 508 px get an outline, bigger ones none, and at
    /// 508 px it is still exactly the footprint.
    #[test]
    fn round_tips_get_an_outline_up_to_508_px() {
        for square in [false, true] {
            let brush = |size: f32| BrushSettings { size, square, ..Default::default() };
            let rings = aliased_outline(&brush(508.0)).unwrap_or_else(|| panic!("square {square}: no outline at 508 px"));
            assert!(aliased_outline(&brush(509.0)).is_none(), "square {square}");
            let corners: usize = rings.iter().map(Vec::len).sum();
            assert!(corners <= MAX_ALIASED_CORNERS, "{corners}");
            // A disc of radius 254 holds about π·254² pixels; the Block square exactly 508².
            let area = pixels(&rings);
            if square {
                assert_eq!(area, 508 * 508);
            } else {
                assert!((area as f64 - std::f64::consts::PI * 254.0 * 254.0).abs() < 4.0 * 508.0, "{area}");
            }
            let at = [3.3, -2.6];
            let placed = placed(&brush(508.0), at).unwrap_or_else(|| panic!("square {square}: no outline at {at:?}"));
            assert_eq!(placed, painted(&brush(508.0), at, PixelFormat::RGBA8, 260), "square {square}");
        }
    }

    /// The cap is checked before `BrushContext::new` builds a sampled tip's mipmaps, on the box
    /// the context's `dab_rect` would give: for round, sampled, unusable and stored tips, at any
    /// roundness, angle and flip, for the Block square and for sizes the renderer bounds. So the
    /// early check turns away exactly the tips over the cap (round over 508 px, sampled over
    /// about 359), and no others.
    #[test]
    fn the_cap_is_checked_before_the_mipmaps_are_built() {
        let stored = |full: Option<GrayTile>| {
            TipShape::Stored(crate::StoredTile {
                key: "l".into(),
                width: 20,
                height: 14,
                preview: std::sync::Arc::new(l_tip()),
                full: full.map(std::sync::Arc::new),
            })
        };
        let tips = [
            ("round", TipShape::Round),
            ("L", TipShape::Sampled(l_tip())),
            ("3×40", TipShape::Sampled(GrayTile::from_fn(3, 40, |_, _| 1.0))),
            ("unusable", TipShape::Sampled(GrayTile { width: 4, height: 4, data: vec![u16::MAX; 3].into() })),
            ("stored", stored(None)),
            ("stored, loaded", stored(Some(ring_tip()))),
        ];
        let sizes: Vec<f32> = (1..=40)
            .map(|s| s as f32 / 2.0)
            .chain((1430..=1450).map(|s| s as f32 / 4.0))
            .chain((2028..=2040).map(|s| s as f32 / 4.0))
            .chain([1000.0, 5000.0, 1e9, f32::NAN, f32::INFINITY, -4.0, 0.0])
            .collect();
        let mut fails = Vec::new();
        for (name, tip) in &tips {
            for &size in &sizes {
                for (roundness, angle, flip_x, square) in [(1.0, 0.0, false, false), (0.2, 30.0, true, false), (0.5, -110.0, false, true)] {
                    let b = BrushSettings { size, roundness, angle, flip_x, square, aliased: true, tip: tip.clone(), ..Default::default() };
                    let b = b.bounded_for_render();
                    let dab = plain_dab(&b);
                    let (early, exact) = (aliased_dab_rect(&b, &dab), BrushContext::new(&b).dab_rect(&dab, false));
                    if early != exact {
                        fails.push(format!("{name} tip at {size} px, roundness {roundness}, angle {angle}: {early:?}, the context's {exact:?}"));
                    }
                }
            }
        }
        assert!(fails.is_empty(), "{} boxes differ:\n{}", fails.len(), fails.join("\n"));
        let fits = |tip: &TipShape, size: f32| {
            let b = BrushSettings { size, aliased: true, tip: tip.clone(), ..Default::default() };
            let r = aliased_dab_rect(&b, &plain_dab(&b));
            r.width().max(r.height()) <= MAX_ALIASED_SIDE
        };
        for (name, tip, last) in [("round", TipShape::Round, 508.0), ("sampled", TipShape::Sampled(l_tip()), 359.0)] {
            let b = |size: f32| BrushSettings { size, tip: tip.clone(), ..Default::default() };
            assert!(fits(&tip, last) && aliased_outline(&b(last)).is_some(), "{name} tip at {last} px");
            assert!(!fits(&tip, last + 1.0) && aliased_outline(&b(last + 1.0)).is_none(), "{name} tip at {} px", last + 1.0);
        }
    }

    /// Hostile but finite tip numbers: no panic, and any outline is still exactly what a dab
    /// paints. So is the round tip that sampled tips with unusable bitmaps paint instead.
    #[test]
    fn hostile_finite_tip_numbers_still_outline_the_footprint() {
        let mut fails = Vec::new();
        for size in [f32::MIN_POSITIVE, 1e-40, 0.499_999, 0.5, 0.500_001, 1.499_999, 1.5, 2.500_001, 13.0] {
            for roundness in [0.0, -0.0, 1e-40, f32::MIN_POSITIVE, 0.01, 0.009, 1.0, 1.000_001, f32::MAX, -5.0] {
                for angle in [0.0, -0.0, 1e-30, 1e30, -1e30, 360.0, 720.5, f32::MAX, -f32::MAX] {
                    fails.extend(mismatch(&BrushSettings { size, roundness, angle, ..Default::default() }, [5.3, -2.7], PixelFormat::RGBA8));
                }
            }
        }
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let _ = aliased_outline(&BrushSettings { size: 9.0, angle: v, ..Default::default() });
            let _ = aliased_outline(&BrushSettings { size: 9.0, roundness: v, ..Default::default() });
        }
        // An empty bitmap, and one whose data doesn't fit its size.
        for tip in [GrayTile { width: 0, height: 0, data: Vec::new().into() }, GrayTile { width: 4, height: 4, data: vec![u16::MAX; 3].into() }] {
            fails.extend(mismatch(&BrushSettings { size: 9.0, tip: TipShape::Sampled(tip), ..Default::default() }, [5.3, -2.7], PixelFormat::RGBA8));
        }
        assert!(fails.is_empty(), "{} mismatches:\n{}", fails.len(), fails.join("\n"));
    }
}
