//! Displace, Shear, ZigZag (inverse mapping over the reference bounds).

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::distort::{centre, remap};
use crate::image::{Edge, Image};
use crate::other::edge_of;
use crate::{Ctx, UndefinedAreas, ZigZagStyle};

/// Max displacement at 100 % scale and full map contrast, px.
const DISPLACE_PX: f32 = 128.0;

/// Bilinear read of channel `c` of a map at map-local coordinates (clamped).
fn map_sample(map: &Image, u: f32, v: f32, c: usize) -> f32 {
    let r = map.rect;
    let (w, h) = (r.width() as i32, r.height() as i32);
    let (fx, fy) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (ax, ay) = (fx - x0, fy - y0);
    let g = |x: i32, y: i32| map.get(r.x0 + x.clamp(0, w - 1), r.y0 + y.clamp(0, h - 1), c);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let top = g(x0, y0) + (g(x0 + 1, y0) - g(x0, y0)) * ax;
    let bot = g(x0, y0 + 1) + (g(x0 + 1, y0 + 1) - g(x0, y0 + 1)) * ax;
    top + (bot - top) * ay
}

/// Displace's horizontal or vertical scale in %, bounded as the filter's own ±999 % (the engine
/// clamps its parameters there too), so the reach of the read and the remap agree and stay small;
/// a non-finite scale moves nothing.
fn displace_scale(s: f32) -> f32 {
    if s.is_finite() { s.clamp(-999.0, 999.0) } else { 0.0 }
}

/// A map value's shift as a fraction of the full one. On an 8-bit map Photoshop's zero is level
/// 128, and each level is one 128th, so at 100 % a level moves one pixel (160 moves 32 px, 255
/// moves 127, 0 moves -128). Mid-grey as 127.5 with 255ths moved 160 by 32.6 px (#10444). A deeper
/// map has no level 128: its zero is its exact mid-grey, half (Photoshop's 16-bit 16384 of 32768),
/// which as level 127.5 moved 0.5 px at 100 %.
///
/// Values past the map's range (a 32-bit map's) shift no further than its ends, which bounds what
/// Displace reads ([`displace_source`]).
fn shift(m: f32, levels: bool) -> f32 {
    let m = m.clamp(0.0, 1.0);
    if levels { (m * 255.0 - 128.0) / 128.0 } else { (m - 0.5) * 2.0 }
}

/// The pixels Displace samples for output `area` in the frame `bounds`, with the frame's undefined
/// areas filled in: the map moves a pixel by at most the full shift, so `area` grown by it (and by
/// the bilinear neighbours) is all it reads. The frame takes in a layer's pixels past the canvas,
/// and reading all of it allocated the gap to one far off-canvas pixel. Wrapping reads the
/// frame's far side, so each axis reads its runs of source pixels separately.
pub(crate) fn displace_source(surface: &Surface, area: Rect, bounds: Rect, scale: (f32, f32), undefined: UndefinedAreas) -> Image {
    let (sx, sy) = (displace_scale(scale.0), displace_scale(scale.1));
    let reach = (sx.abs().max(sy.abs()) / 100.0 * DISPLACE_PX).ceil() as i32 + 2;
    let rect = area.inflate(reach);
    let mut img = Image::new(rect, surface.channels());
    if bounds.is_empty() {
        return img;
    }
    let edge = edge_of(undefined);
    // Each of the rect's coordinates on an axis as the frame's it reads (`None`: transparent), as
    // `Image::get_edge` reads it.
    let axis = |lo: i32, hi: i32, f0: i32, f1: i32| -> Vec<Option<i32>> {
        (lo..hi)
            .map(|v| match edge {
                _ if (f0..f1).contains(&v) => Some(v),
                Edge::Transparent => None,
                Edge::Repeat => Some(v.clamp(f0, f1 - 1)),
                Edge::Wrap => Some(f0 + (v - f0).rem_euclid(f1 - f0)),
            })
            .collect()
    };
    let (xs, ys) = (axis(rect.x0, rect.x1, bounds.x0, bounds.x1), axis(rect.y0, rect.y1, bounds.y0, bounds.y1));
    // The runs of consecutive coordinates an axis reads.
    let runs = |m: &[Option<i32>]| {
        let mut v: Vec<i32> = m.iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        let mut runs: Vec<(i32, i32)> = Vec::new();
        for c in v {
            match runs.last_mut() {
                Some(r) if r.1 == c => r.1 += 1,
                _ => runs.push((c, c + 1)),
            }
        }
        runs
    };
    let (xr, yr) = (runs(&xs), runs(&ys));
    // A row's spans `(at, x run, first x, width)` of pixels read side by side from one x run.
    let mut spans: Vec<(usize, usize, i32, i32)> = Vec::new();
    for (i, sx) in xs.iter().enumerate() {
        let Some(sx) = *sx else { continue };
        let run = xr.partition_point(|r| r.1 <= sx);
        match spans.last_mut() {
            Some((at, r, x0, n)) if *r == run && *at + *n as usize == i && *x0 + *n == sx => *n += 1,
            _ => spans.push((i, run, sx, 1)),
        }
    }
    // Source rows are read a band at a time, so the copy holds little more than the result.
    const BAND: i32 = 64;
    let (w, ch) = (rect.width() as usize, img.ch);
    for &(y0, y1) in &yr {
        for b0 in (y0..y1).step_by(BAND as usize) {
            let b1 = (b0 + BAND).min(y1);
            let band: Vec<Image> = xr.iter().map(|&(x0, x1)| Image::read(surface, Rect::new(x0, b0, x1, b1))).collect();
            for (j, sy) in ys.iter().enumerate() {
                let Some(sy) = sy.filter(|sy| (b0..b1).contains(sy)) else { continue };
                for &(at, run, x0, n) in &spans {
                    let d = (j * w + at) * ch;
                    img.data[d..d + n as usize * ch].copy_from_slice(band[run].row(Rect::new(x0, sy, x0 + n, sy + 1)));
                }
            }
        }
    }
    img
}

/// Displace: the map's first channel shifts pixels horizontally and its
/// second (or first, for one-channel maps) vertically; level 128 (half, on a deeper map) = no shift.
/// The map is stretched over the bounds or tiled from their top-left. `src` is
/// [`displace_source`]'s: the bounds' undefined areas are already filled in around `out`.
pub(crate) fn displace(src: &Image, out: Rect, ctx: &Ctx, scale: (f32, f32), stretch: bool, map: Option<&Image>, map_8bit: bool) -> Vec<f32> {
    let Some(map) = map.filter(|m| !m.rect.is_empty()) else { return src.crop(out) };
    let b = ctx.bounds;
    let (mw, mh) = (map.rect.width() as f32, map.rect.height() as f32);
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    // Colour channels of the map (alpha excluded when present: 2 = gray+alpha, 4 = RGBA…).
    let vc = if map.ch >= 3 { 1 } else { 0 };
    let (sx, sy) = (displace_scale(scale.0) / 100.0 * DISPLACE_PX, displace_scale(scale.1) / 100.0 * DISPLACE_PX);
    remap(src, out, &Ctx { bounds: src.rect, ..*ctx }, Edge::Repeat, |x, y| {
        let (lx, ly) = (x - b.x0 as f32, y - b.y0 as f32);
        let (u, v) = if stretch { (lx / bw * mw, ly / bh * mh) } else { (lx.rem_euclid(mw), ly.rem_euclid(mh)) };
        let dx = shift(map_sample(map, u, v, 0), map_8bit) * sx;
        let dy = shift(map_sample(map, u, v, vc), map_8bit) * sy;
        (x + dx, y + dy)
    })
}

/// Catmull–Rom interpolation through sorted `(t, value)` points (clamped ends).
fn curve_at(pts: &[[f32; 2]], t: f32) -> f32 {
    match pts.len() {
        0 => 0.0,
        1 => pts[0][1],
        _ => {
            if t <= pts[0][0] {
                return pts[0][1];
            }
            let last = pts.len() - 1;
            if t >= pts[last][0] {
                return pts[last][1];
            }
            let i = pts.windows(2).position(|w| t >= w[0][0] && t <= w[1][0]).unwrap_or(0);
            let p1 = pts[i];
            let p2 = pts[i + 1];
            let p0 = if i > 0 { pts[i - 1] } else { p1 };
            let p3 = if i + 2 <= last { pts[i + 2] } else { p2 };
            let span = (p2[0] - p1[0]).max(1e-6);
            let s = (t - p1[0]) / span;
            // Tangents scaled to the span (non-uniform Catmull–Rom).
            let m1 = if i > 0 { (p2[1] - p0[1]) / (p2[0] - p0[0]).max(1e-6) * span } else { p2[1] - p1[1] };
            let m2 = if i + 2 <= last { (p3[1] - p1[1]) / (p3[0] - p1[0]).max(1e-6) * span } else { p2[1] - p1[1] };
            let (s2, s3) = (s * s, s * s * s);
            (2.0 * s3 - 3.0 * s2 + 1.0) * p1[1] + (s3 - 2.0 * s2 + s) * m1 + (-2.0 * s3 + 3.0 * s2) * p2[1] + (s3 - s2) * m2
        }
    }
}

/// Shear: each row shifts horizontally by the curve through `points`
/// (`[t, offset]`, t top→bottom, offset in half-widths).
pub(crate) fn shear(src: &Image, out: Rect, ctx: &Ctx, points: &[[f32; 2]], undefined: UndefinedAreas) -> Vec<f32> {
    let b = ctx.bounds;
    let mut pts: Vec<[f32; 2]> = points.iter().map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(-1.0, 1.0)]).collect();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    // Per-row offsets are shared by every pixel of the row.
    let rows: Vec<f32> = (out.y0..out.y1).map(|y| curve_at(&pts, (y as f32 + 0.5 - b.y0 as f32) / bh) * bw / 2.0).collect();
    remap(src, out, ctx, edge_of(undefined), |x, y| {
        let i = ((y - 0.5) as i32 - out.y0).clamp(0, rows.len() as i32 - 1) as usize;
        (x - rows[i], y)
    })
}

/// ZigZag: radial ripples inside the bounds' inscribed circle — angular
/// (around centre), radial (out from centre) or both (pond ripples).
pub(crate) fn zigzag(src: &Image, out: Rect, ctx: &Ctx, amount: f32, ridges: f32, style: ZigZagStyle) -> Vec<f32> {
    let (cx, cy, rr) = centre(ctx.bounds);
    let a = (amount / 100.0).clamp(-1.0, 1.0);
    let k = ridges.clamp(0.0, 20.0).max(0.5);
    remap(src, out, ctx, crate::image::Edge::Repeat, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt();
        if r >= rr || r <= 0.0 || a == 0.0 {
            return (x, y);
        }
        let rn = r / rr;
        let wave = (std::f32::consts::TAU * k * rn).sin() * (1.0 - rn);
        let (radial, angular) = match style {
            ZigZagStyle::OutFromCenter => (a * wave * rr / (4.0 * k), 0.0),
            ZigZagStyle::AroundCenter => (0.0, a * wave * std::f32::consts::PI / (2.0 * k)),
            ZigZagStyle::PondRipples => (a * wave * rr / (8.0 * k), a * wave * std::f32::consts::PI / (4.0 * k)),
        };
        let th = dy.atan2(dx) + angular;
        let rs = (r + radial).max(0.0);
        (cx + rs * th.cos(), cy + rs * th.sin())
    })
}

#[cfg(test)]
mod tests {
    use super::curve_at;

    #[test]
    fn curve_passes_through_points_and_clamps() {
        let pts = [[0.0, 0.0], [0.5, 0.4], [1.0, -0.2]];
        assert!((curve_at(&pts, 0.5) - 0.4).abs() < 1e-6);
        assert!((curve_at(&pts, 1.0) + 0.2).abs() < 1e-6);
        assert_eq!(curve_at(&pts, -1.0), 0.0);
        assert_eq!(curve_at(&[[0.0, 0.3], [1.0, 0.3]], 0.42), 0.3);
    }
}
