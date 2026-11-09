//! Bevelled outer corners for Edit › Stroke (#10775). Photoshop strokes a selection's outline
//! with bevel joins: past a rectangle's corner, the two outer edges of the band are joined by a
//! 45° cut through the points `r` px out along each side, not by the quarter circle that a
//! Euclidean grow ([`super::expand`]) leaves. Curves keep their round grow: only a corner of the
//! selection's local bounding box, with nothing else selected within the stroke's reach beyond
//! it, is cut, and a staircase approximating a curve always has selected pixels there.

use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

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
/// nothing. One parallel pass over the rows finds the corners; however many there are, their
/// checks read at most about eight canvases' worth plus a few hundred pixels each (see
/// [`bevel_with`]); a cut touches only its quadrant's `r` × `r` box.
pub fn bevel_corners(grown: &mut [f32], m: &[f32], w: usize, h: usize, r: usize) {
    bevel_with(grown, m, w, h, r, Check::Auto);
}

/// How [`bevel_corners`] tells whether a quadrant is clear: `Auto` scans, then switches to the
/// summed-area table once the scans have read [`SCAN_BUDGET`] canvases' worth; the others force
/// one way (tests compare them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Check {
    Auto,
    #[cfg(test)]
    Scan,
    #[cfg(test)]
    Table,
    /// `Auto` with another budget, in canvases.
    #[cfg(test)]
    Budget(usize),
}

/// Pixels a quadrant scan may read without counting towards the switch to the table.
const SHORT_SCAN: usize = 256;

/// Canvases' worth of long scans before the rest of the checks switch to the table: the scans
/// run on every core and the table is built on one (about 40 ms at 24 MP), so it pays off only
/// once the scans have read several canvases.
const SCAN_BUDGET: usize = 8;

/// [`bevel_corners`], telling whether the table answered any check.
fn bevel_with(grown: &mut [f32], m: &[f32], w: usize, h: usize, r: usize, check: Check) -> bool {
    // Below 2 px the round corner and the cut keep the same pixels.
    if r < 2 || w == 0 || h == 0 || w.checked_mul(h) != Some(m.len()) || grown.len() != m.len() {
        return false;
    }
    // From w + h on, every in-canvas pixel is within reach and the cut (dx + dy > r + 1)
    // lies off the canvas, so a larger reach changes nothing.
    let r = r.min(w.saturating_add(h));
    // A scan reads a quadrant's reach (up to about 6r² pixels) only until it meets a selected
    // pixel, so a selection that goes on past its corners costs little: on 24 MP of Magic Wand
    // noise a 250 px reach is about as fast as 2 px. A selection could still make many corners
    // read far (#10828 review), so once the scans have read a few canvases' worth the rest are
    // answered from a summed-area table built then (one pass, a u32 per pixel): a few lookups
    // a corner, the same answer. Its wrapping u32 counts are exact while the canvas has fewer
    // than 2³² pixels; past that the scans go on.
    let fits = u32::try_from(m.len()).is_ok();
    let (always, never, budget) = match check {
        Check::Auto => (false, !fits, SCAN_BUDGET),
        #[cfg(test)]
        Check::Scan => (false, true, 0),
        #[cfg(test)]
        Check::Table => (fits, !fits, 0),
        #[cfg(test)]
        Check::Budget(budget) => (false, !fits, budget),
    };
    let (read, table) = (AtomicUsize::new(0), OnceLock::new());
    let found = corners(m, w, h, |c, d| {
        if !never && (always || read.load(Ordering::Relaxed) > m.len().saturating_mul(budget)) {
            // Built on one thread: a parallel build could run another row's check on this
            // thread while it initialises the table.
            return table.get_or_init(|| Counts::new(m, w, h)).quadrant_is_clear(c, d, r);
        }
        let (clear, n) = quadrant_is_clear(m, (w, h), c, d, r);
        // A short scan costs about what the table's lookups do; leaving it out of the count
        // keeps the threads off the shared counter on speckles.
        if n > SHORT_SCAN {
            read.fetch_add(n, Ordering::Relaxed);
        }
        clear
    });
    let r = r as isize;
    for (x, y, sx, sy) in found {
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
    table.get().is_some()
}

/// The corners of `m` (see [`bevel_corners`]) whose quadrant `clear` says is clear, found in
/// one parallel pass over the rows.
fn corners(m: &[f32], w: usize, h: usize, clear: impl Fn((usize, usize), (isize, isize)) -> bool + Sync + Send) -> Vec<(usize, usize, isize, isize)> {
    let selected = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return false;
        }
        m.get(y as usize * w + x as usize).is_some_and(|v| *v >= 0.5)
    };
    par_map(h, |y| {
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
                    if !selected(xi, yi + sy) && !selected(xi + sx, yi + sy) && clear((end, y), (sx, sy)) {
                        found.push((end, y, sx, sy));
                    }
                }
            }
            x = b + 1;
        }
        found
    })
    .into_iter()
    .flatten()
    .collect()
}

/// The columns `i` (steps out from the corner along x) of the row `j` steps out that are
/// within reach of the quadrant's `r` × `r` box (`i`, `j` in 1..=r): less than `r + 1` from
/// it, the farthest a selected pixel can be and still grow into it, and only past the corner
/// when the row is level with or behind it (the pixels behind it on both axes are its own).
/// Rows `1 - r ..= 2r` have columns; the span is `near..=far`.
fn span(j: isize, r: isize) -> (isize, isize) {
    let reach = (i128::from(r as i64) + 1).pow(2);
    let gy = i128::from((1 - j).max(j - r).max(0) as i64);
    let gx = (reach - 1 - gy * gy).max(0).isqrt() as isize;
    let near = if j <= 0 { 1 } else { 1 - gx };
    (near, r + gx)
}

/// The canvas range `lo..=hi` that `a`, `b` steps from `at` in direction `s` cover, clipped to
/// `0..len`; `None` when it lies off the canvas.
fn clip(at: isize, s: isize, (a, b): (isize, isize), len: usize) -> Option<(usize, usize)> {
    let (a, b) = (at + s * a, at + s * b);
    let (lo, hi) = (a.min(b).max(0), a.max(b).min(len as isize - 1));
    (lo <= hi).then_some((lo as usize, hi as usize))
}

/// Whether nothing but the pixels behind corner `(x, y)` (on or behind it on both axes) is
/// selected within reach of its quadrant ([`span`]), by reading the pixels, and how many it
/// read. Rows nearest the corner are read first, so a selection that goes on beyond the corner
/// is found quickly.
fn quadrant_is_clear(m: &[f32], (w, h): (usize, usize), (x, y): (usize, usize), (sx, sy): (isize, isize), r: usize) -> (bool, usize) {
    let r = r as isize;
    let (x, y) = (x as isize, y as isize);
    // j = 0, 1, -1, 2, -2, … out to 1 - r and 2r: the rows less than r + 1 from the box.
    let order = (0..=2 * r).flat_map(|k| [k, -k].into_iter().take(if k == 0 || k >= r { 1 } else { 2 }));
    let mut read = 0;
    for j in order {
        let py = y + sy * j;
        if py < 0 || py as usize >= h {
            continue;
        }
        let Some((lo, hi)) = clip(x, sx, span(j, r), w) else { continue };
        let row = py as usize * w;
        read += hi - lo + 1;
        if m.get(row + lo..=row + hi).is_some_and(|px| px.iter().any(|v| *v >= 0.5)) {
            return (false, read);
        }
    }
    (true, read)
}

/// A summed-area table of the selected pixels (`m >= 0.5`): `s[y * (w + 1) + x]` counts those
/// above and left of `(x, y)`, so any rectangle's count is four lookups.
struct Counts {
    s: Vec<u32>,
    w: usize,
    h: usize,
}

impl Counts {
    fn new(m: &[f32], w: usize, h: usize) -> Self {
        let stride = w + 1;
        let mut s = vec![0u32; stride * (h + 1)];
        // Each entry is the one above plus the row's count so far.
        let mut above = vec![0u32; stride];
        for (row, src) in s.chunks_exact_mut(stride).skip(1).zip(m.chunks_exact(w.max(1))) {
            let mut acc = 0u32;
            for ((out, a), v) in row.iter_mut().skip(1).zip(above.iter_mut().skip(1)).zip(src) {
                acc = acc.wrapping_add(u32::from(*v >= 0.5));
                *a = a.wrapping_add(acc);
                *out = *a;
            }
        }
        Counts { s, w, h }
    }

    /// Selected pixels in columns `x0..=x1`, rows `y0..=y1` (in the canvas).
    fn rect(&self, (x0, x1): (usize, usize), (y0, y1): (usize, usize)) -> u32 {
        let at = |x: usize, y: usize| self.s.get(y * (self.w + 1) + x).copied().unwrap_or(0);
        at(x1 + 1, y1 + 1).wrapping_sub(at(x0, y1 + 1)).wrapping_sub(at(x1 + 1, y0)).wrapping_add(at(x0, y0))
    }

    /// Selected pixels at steps `i0..=i1` out along x and `j0..=j1` along y from `(x, y)`.
    fn steps(&self, (x, y): (isize, isize), (sx, sy): (isize, isize), i: (isize, isize), j: (isize, isize)) -> u32 {
        match (clip(x, sx, i, self.w), clip(y, sy, j, self.h)) {
            (Some(xs), Some(ys)) => self.rect(xs, ys),
            _ => 0,
        }
    }

    /// [`quadrant_is_clear`] from the table: the region within reach is a rounded box, so the
    /// cross through it (two rectangles) settles most corners as not clear, the two rectangles
    /// bounding it settle most of the rest as clear, and only a selection in its rounded ends
    /// needs the per-row spans.
    fn quadrant_is_clear(&self, (x, y): (usize, usize), d: (isize, isize), r: usize) -> bool {
        let (r, at) = (r as isize, (x as isize, y as isize));
        // Rows 1..=r reach columns 1 - r ..= 2r, and every row reaches columns 1..=r.
        if self.steps(at, d, (1 - r, 2 * r), (1, r)) != 0 || self.steps(at, d, (1, r), (1 - r, 2 * r)) != 0 {
            return false;
        }
        // All of it lies in columns 1..=2r of rows 1 - r ..= 2r and columns 1 - r ..= 0 of rows
        // 1..=2r.
        if self.steps(at, d, (1, 2 * r), (1 - r, 2 * r)) == 0 && self.steps(at, d, (1 - r, 0), (1, 2 * r)) == 0 {
            return true;
        }
        (1 - r..=2 * r).all(|j| self.steps(at, d, span(j, r), (j, j)) == 0)
    }
}

#[cfg(test)]
mod tests;
