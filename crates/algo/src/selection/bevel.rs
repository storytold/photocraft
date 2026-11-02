//! Bevelled outer corners for Edit › Stroke (#10775). Photoshop strokes a selection's outline
//! with bevel joins: past a rectangle's corner, the two outer edges of the band are joined by a
//! 45° cut through the points `r` px out along each side, not by the quarter circle that a
//! Euclidean grow ([`super::expand`]) leaves. Curves keep their round grow: only a corner of the
//! selection's local bounding box, with nothing else selected within the stroke's reach beyond
//! it, is cut, and a staircase approximating a curve always has selected pixels there.

use crate::photo_util::par_map;

/// Cuts the outer corners of `grown`, the selection `m` grown by `r` px ([`super::expand`]), at
/// 45°, for a hard selection (seeds are `m >= 0.5`, as in the grow).
///
/// A corner is a selected pixel `c` whose two side neighbours and diagonal neighbour towards
/// one quadrant are unselected, with no other selected pixel within reach of that quadrant
/// (only pixels behind `c` on both axes). There a pixel `dx`, `dy` (both ≥ 1) px out from `c`
/// keeps its coverage when `dx + dy ≤ r + 1` (its centre is on or inside the cut through the
/// pixels `r` px out along each edge) and is cleared otherwise. Straight edges, concave
/// corners, curves and the selection itself are untouched. Inconsistent dimensions change
/// nothing. One parallel pass over the rows finds the corners; a cut touches only its
/// quadrant's `r` × `r` box.
pub fn bevel_corners(grown: &mut [f32], m: &[f32], w: usize, h: usize, r: usize) {
    // Below 2 px the round corner and the cut keep the same pixels.
    if r < 2 || w == 0 || h == 0 || w.checked_mul(h) != Some(m.len()) || grown.len() != m.len() {
        return;
    }
    // From w + h on, every in-canvas pixel is within reach and the cut (dx + dy > r + 1)
    // lies off the canvas, so a larger reach changes nothing.
    let r = r.min(w.saturating_add(h));
    let selected = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return false;
        }
        m.get(y as usize * w + x as usize).is_some_and(|v| *v >= 0.5)
    };
    let corners: Vec<Vec<(usize, usize, isize, isize)>> = par_map(h, |y| {
        let Some(row) = m.get(y * w..(y + 1) * w) else { return Vec::new() };
        let mut found = Vec::new();
        // Only the two ends of a run of selected pixels have an unselected side neighbour.
        let mut x = 0;
        while let Some(start) = row.get(x..).and_then(|rest| rest.iter().position(|v| *v >= 0.5)) {
            let a = x + start;
            let b = row.get(a..).and_then(|rest| rest.iter().position(|v| *v < 0.5)).map_or(w, |len| a + len) - 1;
            for (end, sx) in [(a, -1), (b, 1)] {
                let (xi, yi) = (end as isize, y as isize);
                for sy in [-1, 1] {
                    if !selected(xi, yi + sy) && !selected(xi + sx, yi + sy) && quadrant_is_clear(m, (w, h), (end, y), (sx, sy), r) {
                        found.push((end, y, sx, sy));
                    }
                }
            }
            x = b + 1;
        }
        found
    });
    let r = r as isize;
    for (x, y, sx, sy) in corners.into_iter().flatten() {
        for dy in 1..=r {
            let py = y as isize + sy * dy;
            if py < 0 || py as usize >= h {
                break;
            }
            // dx + dy > r + 1: past the cut.
            for dx in (r + 2 - dy).max(1)..=r {
                let px = x as isize + sx * dx;
                if px < 0 || px as usize >= w {
                    break;
                }
                if let Some(v) = grown.get_mut(py as usize * w + px as usize) {
                    *v = 0.0;
                }
            }
        }
    }
}

/// Whether nothing but the pixels behind corner `(x, y)` (on or behind it on both axes) is
/// selected within reach of its quadrant: less than `r + 1` from the quadrant's `r` × `r` box,
/// the farthest a selected pixel can be and still grow into it. Rows nearest the corner are
/// read first, so a selection that goes on beyond the corner is found quickly.
fn quadrant_is_clear(m: &[f32], (w, h): (usize, usize), (x, y): (usize, usize), (sx, sy): (isize, isize), r: usize) -> bool {
    let r = r as isize;
    let (x, y) = (x as isize, y as isize);
    let reach = (i128::from(r as i64) + 1).pow(2);
    // The x span of the row `j` steps out, clipped to the canvas: the columns `i` less than
    // r + 1 from the box (`i`, `j` in 1..=r), and only past the corner when the row is level
    // with or behind it (the pixels behind it on both axes are its own).
    let span = |j: isize| -> Option<(usize, usize)> {
        let gy = i128::from((1 - j).max(j - r).max(0) as i64);
        let gx = (reach - 1 - gy * gy).max(0).isqrt() as isize;
        let near = if j <= 0 { 1 } else { 1 - gx };
        let (a, b) = (x + sx * near, x + sx * (r + gx));
        let (lo, hi) = (a.min(b).max(0), a.max(b).min(w as isize - 1));
        (lo <= hi).then_some((lo as usize, hi as usize))
    };
    // j = 0, 1, -1, 2, -2, … out to 1 - r and 2r: the rows less than r + 1 from the box.
    let order = (0..=2 * r).flat_map(|k| [k, -k].into_iter().take(if k == 0 || k >= r { 1 } else { 2 }));
    for j in order {
        let py = y + sy * j;
        if py < 0 || py as usize >= h {
            continue;
        }
        let Some((lo, hi)) = span(j) else { continue };
        let row = py as usize * w;
        if m.get(row + lo..=row + hi).is_some_and(|px| px.iter().any(|v| *v >= 0.5)) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;
