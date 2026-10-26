//! Content-aware scaling by seam carving (Edit › Content-Aware Scale).
//!
//! S. Avidan, A. Shamir, *Seam Carving for Content-Aware Image Resizing*, SIGGRAPH 2007:
//! a seam is an 8-connected path of one pixel per row (vertical seam) found by dynamic
//! programming over a gradient-magnitude energy map. Shrinking removes the lowest-energy seams one
//! at a time; enlarging finds the `k` lowest seams on a shrinking copy and duplicates them
//! (averaging each with its right neighbour) so the same seam is not stretched repeatedly.
//! Heights are handled by transposing.
//!
//! A protect map (`0..=1`, e.g. an alpha channel or detected skin tones) adds a large energy so
//! seams avoid protected pixels. Buffers are interleaved `w × h × ch` normalised floats of any
//! colour model and depth (alpha is carried like any other channel). Deterministic on every
//! platform: an energy that isn't a number (from NaN samples, or infinite ones next to each other
//! or at the border) is the highest energy, and NaN protection protects nothing, so no NaN (whose
//! sign and payload arithmetic does not keep consistently across builds) reaches the seam search.

use photocraft_raster::{Cancelled, Interrupt};

/// Energy added per unit of protection: far above any gradient magnitude.
const PROTECT_ENERGY: f32 = 1.0e4;

/// The energy protection `p` (`0..=1`) adds. NaN protects nothing.
#[inline]
fn protection(p: f32) -> f32 {
    if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) * PROTECT_ENERGY }
}

/// Keep full horizontal detail while reducing the height used to find vertical seams. On a
/// 6000×4000 image this cuts the dynamic-programming work eight times for a width change (4000
/// rows to 500) and twelve times for a height change, without quantising the output pixels or
/// changing the requested output dimensions.
const SEAM_SEARCH_MAX_HEIGHT: usize = 512;
const PROXY_SHRINK_MIN_HEIGHT: usize = 2_048;
const PROXY_SHRINK_MIN_SEAMS: usize = 64;

/// Gradient-magnitude energy (the paper's e1, `|∂x| + |∂y|` summed over channels). Forward and
/// backward differences are both counted so one-pixel lines (whose central difference is zero)
/// carry energy too.
pub fn energy(w: usize, h: usize, ch: usize, img: &[f32]) -> Vec<f32> {
    let mut e = vec![0.0f32; w * h];
    if w == 0 || h == 0 {
        return e;
    }
    let row = |(y, out): (usize, &mut [f32])| {
        for (x, v) in out.iter_mut().enumerate() {
            *v = energy_at(img, w, w, h, ch, x, y, |xx, _| xx);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        e.par_chunks_mut(w).enumerate().for_each(row);
    }
    #[cfg(target_arch = "wasm32")]
    e.chunks_mut(w).enumerate().for_each(row);
    e
}

/// [`energy`] of pixel `x` of row `y` of a `cw`-wide, `h`-tall image whose pixel `xx` of row `yy`
/// is column `col(xx, yy)` of `img` (`w` pixels per row). Shared by the full map and the carver's
/// per-seam updates, so both give bit-identical values. An energy that isn't a number is the
/// highest energy (`+inf`).
#[allow(clippy::too_many_arguments)]
#[inline]
fn energy_at(img: &[f32], w: usize, cw: usize, h: usize, ch: usize, x: usize, y: usize, col: impl Fn(usize, usize) -> usize) -> f32 {
    let (xl, xr) = (x.saturating_sub(1), (x + 1).min(cw - 1));
    let (yu, yd) = (y.saturating_sub(1), (y + 1).min(h - 1));
    let at = |xx: usize, yy: usize| (yy * w + col(xx, yy)) * ch;
    let (centre, right, left, down, up) = (at(x, y), at(xr, y), at(xl, y), at(x, yd), at(x, yu));
    let mut s = 0.0;
    for c in 0..ch {
        let v = img[centre + c];
        s += (img[right + c] - v).abs() + (v - img[left + c]).abs();
        s += (img[down + c] - v).abs() + (v - img[up + c]).abs();
    }
    let e = s * 0.5;
    if e.is_nan() { f32::INFINITY } else { e }
}

/// The lowest-energy vertical seam: one x per row.
pub fn find_seam(w: usize, h: usize, energy: &[f32]) -> Vec<usize> {
    if w == 0 || h == 0 || energy.len() < w * h {
        return vec![0; h];
    }
    let off = vec![0; h];
    let rows = Rows { stride: w, cw: w, off: &off };
    let mut dp = Dp::default();
    dp.full(rows, energy);
    dp.seam(rows)
}

/// Where the `cw` pixels in use of each of a carver's rows are, in buffers whose rows are
/// `stride` apart: row y's are `off[y]..off[y] + cw` of its `stride` slots.
#[derive(Clone, Copy)]
struct Rows<'o> {
    stride: usize,
    cw: usize,
    off: &'o [usize],
}

impl Rows<'_> {
    fn h(&self) -> usize {
        self.off.len()
    }

    /// Row y's pixels in use, as a range of its buffers.
    #[inline]
    fn of(&self, y: usize) -> std::ops::Range<usize> {
        let start = y * self.stride + self.off[y];
        start..start + self.cw
    }
}

/// The seam search's dynamic programme over an energy map laid out as [`Rows`] (the carver's own
/// layout, so a removal shifts it like the other buffers): for every pixel, the cost of the
/// cheapest 8-connected path down to it from the top row. [`Carver`] keeps it across seams and
/// recomputes only what each removal changed ([`Dp::update`]).
#[derive(Default)]
struct Dp {
    /// Cost of the cheapest path from the top row to each pixel.
    cost: Vec<f32>,
}

/// Pixel `x` of a `cw`-wide row with energy `e`, given the path costs of the row above (`up`):
/// its path cost and where that path came from (0 = up-left, 1 = up, 2 = up-right). Ties go to
/// the pixel straight up, then up-left, then up-right. The cost doesn't depend on the tie-break
/// and the direction doesn't depend on `e`, so the seam can be traced back from the costs alone.
/// [`row_costs`] uses it at the ends of a row and repeats its selection in between, and
/// [`Dp::seam`] traces seams with it, so every way of filling the map gives the same bits (costs
/// are never NaN: see [`energy_at`] and [`protection`]).
#[inline]
fn cell(up: &[f32], cw: usize, x: usize, e: f32) -> (f32, u8) {
    let (b, d) = if x == 0 {
        if cw > 1 && up[1] < up[0] { (up[1], 2) } else { (up[0], 1) }
    } else if x + 1 == cw {
        if up[x - 1] < up[x] { (up[x - 1], 0) } else { (up[x], 1) }
    } else {
        let (b, d) = if up[x - 1] < up[x] { (up[x - 1], 0) } else { (up[x], 1) };
        if up[x + 1] < b { (up[x + 1], 2) } else { (b, d) }
    };
    (b + e, d)
}

/// Path costs of pixels `lo..lo + out.len()` of a `cw`-wide row with energies `e` (the whole
/// row's), from the path costs of the row above (`up`), into `out`. [`cell`] at the ends of the
/// row; in between, the same selection without edge tests or directions, so it vectorises.
#[inline]
fn row_costs(up: &[f32], e: &[f32], cw: usize, lo: usize, out: &mut [f32]) {
    let Some(last) = (lo + out.len()).checked_sub(1) else { return };
    let (mut a, mut b) = (lo, last);
    if a == 0 {
        out[0] = cell(up, cw, 0, e[0]).0;
        a = 1;
    }
    if b + 1 == cw && b >= a {
        out[b - lo] = cell(up, cw, b, e[b]).0;
        b -= 1;
    }
    if a > b {
        return;
    }
    // Every pixel in a..=b has all three parents: up-left, up and up-right.
    let parents = up[a - 1..b].iter().zip(&up[a..=b]).zip(&up[a + 1..=b + 1]);
    for ((n, &ev), ((&l, &u), &r)) in out[a - lo..=b - lo].iter_mut().zip(&e[a..=b]).zip(parents) {
        let m = if l < u { l } else { u };
        let m = if r < m { r } else { m };
        *n = m + ev;
    }
}

impl Dp {
    /// Fill the map for the pixels in use of every row of `energy`.
    fn full(&mut self, rows: Rows, energy: &[f32]) {
        let (cw, h) = (rows.cw, rows.h());
        self.cost.resize(rows.stride * h, 0.0);
        if cw == 0 || h == 0 {
            return;
        }
        let first = rows.of(0);
        self.cost[first.clone()].copy_from_slice(&energy[first]);
        for y in 1..h {
            let (above, rest) = self.cost.split_at_mut(y * rows.stride);
            let (r, base) = (rows.of(y), y * rows.stride);
            row_costs(&above[rows.of(y - 1)], &energy[r.clone()], cw, 0, &mut rest[r.start - base..r.end - base]);
        }
    }

    /// Bring the map up to date after `seam` (one x per row, counted before the removal) was
    /// removed from every row of the map and of `energy`, leaving `rows.cw` pixels per row.
    ///
    /// A pixel's path cost depends only on its energy and on the costs of its parents in the row
    /// above. Where a pixel and all its parents are on the same side of the seam in their rows,
    /// the removal moved them together; that leaves, in row y, only the pixels from one left of
    /// min(seam[y-1], seam[y]) to max(seam[y-1], seam[y]) with new parents. The energy changed
    /// only where [`Carver::update_energy`] recomputed it: from one left of the leftmost of
    /// `seam[y-1..=y+1]` to the rightmost, which covers those pixels too. Every other pixel needs
    /// recomputing only if a parent's cost changed, so the work follows the changed costs down
    /// the map and narrows wherever they settle, instead of covering the whole map. Recomputed
    /// pixels go through [`row_costs`], like a [`Dp::full`] pass's, so the map matches one over
    /// the same energy bit for bit.
    fn update(&mut self, rows: Rows, energy: &[f32], seam: &[usize]) {
        let (cw, h) = (rows.cw, rows.h());
        if cw == 0 || h == 0 || seam.len() < h {
            return;
        }
        // Pixels of the row above whose cost changed: `lo..=hi`, or none when `lo > hi`.
        let (mut changed_lo, mut changed_hi) = (usize::MAX, 0);
        let mut fresh = Vec::new();
        for y in 0..h {
            let near = &seam[y.saturating_sub(1)..(y + 2).min(h)];
            let mut lo = near.iter().min().map_or(0, |&s| s.saturating_sub(1));
            let mut hi = near.iter().max().map_or(0, |&s| s.min(cw - 1));
            if changed_lo <= changed_hi {
                lo = lo.min(changed_lo.saturating_sub(1));
                hi = hi.max((changed_hi + 1).min(cw - 1));
            }
            let (above, rest) = self.cost.split_at_mut(y * rows.stride);
            let (r, base) = (rows.of(y), y * rows.stride);
            let e = &energy[r.clone()];
            if fresh.len() < hi + 1 - lo {
                fresh.resize(hi + 1 - lo, 0.0);
            }
            let fresh = &mut fresh[..hi + 1 - lo];
            match y.checked_sub(1) {
                Some(u) => row_costs(&above[rows.of(u)], e, cw, lo, fresh),
                None => fresh.copy_from_slice(&e[lo..=hi]),
            }
            // Store only what changed, and remember where that was for the row below.
            let row = &mut rest[r.start - base + lo..r.start - base + hi + 1];
            let differs = |(a, b): (&f32, &f32)| a.to_bits() != b.to_bits();
            (changed_lo, changed_hi) = (usize::MAX, 0);
            if let Some(first) = fresh.iter().zip(row.iter()).position(differs) {
                let last = fresh.iter().zip(row.iter()).rposition(differs).unwrap_or(first);
                row[first..=last].copy_from_slice(&fresh[first..=last]);
                (changed_lo, changed_hi) = (lo + first, lo + last);
            }
        }
    }

    /// The lowest-energy seam of the map: one x per row. At the bottom, ties go to the leftmost
    /// end; going up, [`cell`] tells where each pixel's path came from.
    fn seam(&self, rows: Rows) -> Vec<usize> {
        let (cw, h) = (rows.cw, rows.h());
        let mut seam = vec![0usize; h];
        if cw == 0 || h == 0 {
            return seam;
        }
        let last = &self.cost[rows.of(h - 1)];
        let mut x = (0..cw).min_by(|a, b| last[*a].total_cmp(&last[*b])).unwrap_or(0);
        for y in (0..h).rev() {
            seam[y] = x;
            if y > 0 {
                // `cell` never points past either end of the row; saturate rather than trust it.
                x = match cell(&self.cost[rows.of(y - 1)], cw, x, 0.0).1 {
                    0 => x.saturating_sub(1),
                    2 => (x + 1).min(cw - 1),
                    _ => x,
                };
            }
        }
        seam
    }
}

/// Shrinks an image one seam at a time without moving its pixels: the carver keeps which source
/// column each remaining pixel is (`idx`) and reads pixels through it. A removal shifts, in each
/// row, whichever side of the seam is shorter in the three per-pixel buffers (in parallel over
/// rows), and nothing is recomputed in full: the gradient of a pixel changes only when a
/// neighbour does, which happens within a pixel of the seam in its own row and the rows around
/// it, and the seam search's path costs are updated from there down ([`Dp::update`]).
struct Carver<'a> {
    /// The image, `w × h × ch`, and the protection, one value per pixel: as given, never moved.
    src: &'a [f32],
    prot: Option<&'a [f32]>,
    w: usize,
    h: usize,
    ch: usize,
    /// Pixels left per row.
    cw: usize,
    /// Where each row's first remaining pixel is in the per-pixel buffers (see [`Rows`]).
    off: Vec<usize>,
    /// Source column of each remaining pixel (rows `w` apart; layer areas are at most i32::MAX
    /// wide, so `u32` holds any column).
    idx: Vec<u32>,
    /// Energy plus protection, as the seam search sees it.
    e: Vec<f32>,
    /// The seam search over `e`, kept up to date.
    dp: Dp,
}

impl<'a> Carver<'a> {
    fn new(w: usize, h: usize, ch: usize, src: &'a [f32], prot: Option<&'a [f32]>) -> Self {
        let mut e = energy(w, h, ch, src);
        if let Some(p) = prot {
            for (e, &p) in e.iter_mut().zip(p) {
                *e += protection(p);
            }
        }
        let mut idx = Vec::with_capacity(w * h);
        for _ in 0..h {
            idx.extend((0..w).map(|x| u32::try_from(x).unwrap_or(u32::MAX)));
        }
        let off = vec![0; h];
        let mut dp = Dp::default();
        dp.full(Rows { stride: w, cw: w, off: &off }, &e);
        Carver { src, prot, w, h, ch, cw: w, off, idx, e, dp }
    }

    /// Remove the lowest-energy seam. Returns, per row, the source column removed.
    fn remove_seam(&mut self) -> Vec<usize> {
        if self.cw == 0 {
            return Vec::new();
        }
        let rows = Rows { stride: self.w, cw: self.cw, off: &self.off };
        let seam = self.dp.seam(rows);
        let removed = seam.iter().enumerate().map(|(y, &x)| self.idx[rows.of(y).start + x] as usize).collect();
        self.shift(&seam);
        self.cw -= 1;
        self.update_energy(&seam);
        self.dp.update(Rows { stride: self.w, cw: self.cw, off: &self.off }, &self.e, &seam);
        removed
    }

    /// Remove `seam[y]` from each row of the per-pixel buffers.
    fn shift(&mut self, seam: &[usize]) {
        let (w, cw) = (self.w, self.cw);
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            self.e
                .par_chunks_mut(w)
                .zip(self.idx.par_chunks_mut(w))
                .zip(self.dp.cost.par_chunks_mut(w))
                .zip(self.off.par_iter_mut())
                .zip(seam.par_iter())
                .for_each(|((((e, idx), cost), off), &x)| shift_row(e, idx, cost, off, x, cw));
        }
        #[cfg(target_arch = "wasm32")]
        self.e
            .chunks_mut(w)
            .zip(self.idx.chunks_mut(w))
            .zip(self.dp.cost.chunks_mut(w))
            .zip(self.off.iter_mut())
            .zip(seam)
            .for_each(|((((e, idx), cost), off), &x)| shift_row(e, idx, cost, off, x, cw));
    }

    /// Recompute the energy of the pixels whose neighbours changed when `seam` was removed: in
    /// row y, from one left of the leftmost of the seam's x in rows y-1..=y+1 to the rightmost.
    fn update_energy(&mut self, seam: &[usize]) {
        let (w, cw, h, ch) = (self.w, self.cw, self.h, self.ch);
        if cw == 0 {
            return;
        }
        let (src, prot, off, idx) = (self.src, self.prot, &self.off, &self.idx);
        let col = |xx: usize, yy: usize| idx[yy * w + off[yy] + xx] as usize;
        let row = |(y, e): (usize, &mut [f32])| {
            let near = &seam[y.saturating_sub(1)..(y + 2).min(h)];
            let lo = near.iter().min().map_or(0, |&s| s.saturating_sub(1));
            let hi = near.iter().max().map_or(0, |&s| s.min(cw - 1));
            for x in lo..=hi {
                let mut v = energy_at(src, w, cw, h, ch, x, y, col);
                if let Some(p) = prot {
                    v += protection(p[y * w + col(x, y)]);
                }
                e[off[y] + x] = v;
            }
        };
        // A few pixels per row (seams move at most one column per row), but each waits on reads
        // from three rows of the source: rows in parallel overlap the waits.
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            self.e.par_chunks_mut(w).enumerate().for_each(row);
        }
        #[cfg(target_arch = "wasm32")]
        self.e.chunks_mut(w).enumerate().for_each(row);
    }

    /// The carved pixels, `cw × h × ch`, rows packed.
    fn into_pixels(self) -> Vec<f32> {
        let (w, cw, ch) = (self.w, self.cw, self.ch);
        let mut out = vec![0.0f32; cw * self.h * ch];
        if cw == 0 {
            return out;
        }
        let (src, off, idx) = (self.src, &self.off, &self.idx);
        let row = |(y, out): (usize, &mut [f32])| {
            let cols = &idx[y * w + off[y]..y * w + off[y] + cw];
            for (px, &x) in out.chunks_exact_mut(ch).zip(cols) {
                let s = (y * w + x as usize) * ch;
                px.copy_from_slice(&src[s..s + ch]);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            out.par_chunks_mut(cw * ch).enumerate().for_each(row);
        }
        #[cfg(target_arch = "wasm32")]
        out.chunks_mut(cw * ch).enumerate().for_each(row);
        out
    }
}

/// Remove pixel `x` of the `cw` in use, from `off` on, of one row of a carver's per-pixel
/// buffers, moving whichever side of it holds fewer pixels (the row then starts one later when
/// that is the left side).
fn shift_row(e: &mut [f32], idx: &mut [u32], cost: &mut [f32], off: &mut usize, x: usize, cw: usize) {
    let o = *off;
    if x < cw - 1 - x {
        e.copy_within(o..o + x, o + 1);
        idx.copy_within(o..o + x, o + 1);
        cost.copy_within(o..o + x, o + 1);
        *off = o + 1;
    } else {
        e.copy_within(o + x + 1..o + cw, o + x);
        idx.copy_within(o + x + 1..o + cw, o + x);
        cost.copy_within(o + x + 1..o + cw, o + x);
    }
}

fn transpose(w: usize, h: usize, ch: usize, img: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; img.len()];
    if w == 0 || h == 0 || ch == 0 {
        return out;
    }
    // Output rows (source columns) in blocks, so each source cache line serves several of them.
    const BLOCK: usize = 16;
    let block = |(b, out): (usize, &mut [f32])| {
        let x0 = b * BLOCK;
        let n = out.len() / (h * ch);
        for y in 0..h {
            for i in 0..n {
                let (s, d) = ((y * w + x0 + i) * ch, (i * h + y) * ch);
                out[d..d + ch].copy_from_slice(&img[s..s + ch]);
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(BLOCK * h * ch).enumerate().for_each(block);
    }
    #[cfg(target_arch = "wasm32")]
    out.chunks_mut(BLOCK * h * ch).enumerate().for_each(block);
    out
}

/// Each `step` rows of `src` (`h` rows of `row_len` values) averaged into one, the last group
/// possibly shorter: `h.div_ceil(step)` rows.
fn average_rows(src: &[f32], row_len: usize, h: usize, step: usize) -> Vec<f32> {
    let mut out = vec![0.0; h.div_ceil(step) * row_len];
    if row_len == 0 {
        return out;
    }
    let group = |(py, out_row): (usize, &mut [f32])| {
        let y0 = py * step;
        let y1 = (y0 + step).min(h);
        let divisor = (y1 - y0) as f32;
        for y in y0..y1 {
            for (out, &sample) in out_row.iter_mut().zip(&src[y * row_len..(y + 1) * row_len]) {
                *out += sample / divisor;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_exact_mut(row_len).enumerate().for_each(group);
    }
    #[cfg(target_arch = "wasm32")]
    out.chunks_exact_mut(row_len).enumerate().for_each(group);
    out
}

/// No pixels or channels to work with, or `img` holds fewer than `w × h × ch` values.
fn unusable(w: usize, h: usize, ch: usize, img: &[f32]) -> bool {
    w == 0 || h == 0 || ch == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(ch)).is_none_or(|n| img.len() < n)
}

/// Change the width of `img` to `new_w` by removing or duplicating vertical seams.
pub fn carve_width(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize) -> Vec<f32> {
    // Never cancelled; the fallback is unreachable.
    carve_width_with(w, h, ch, img, protect, new_w, &Interrupt::NONE).unwrap_or_else(|_| img.to_vec())
}

/// [`carve_width`] that checks `ctl` before every seam and reports progress per seam.
pub fn carve_width_with(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize, ctl: &Interrupt) -> Result<Vec<f32>, Cancelled> {
    let new_w = new_w.max(1);
    // Nothing to carve: no pixels or channels, or fewer values than the size says.
    if unusable(w, h, ch, img) {
        return Ok(img.to_vec());
    }
    // Values past the size are not part of the image.
    let img = img.get(..w * h * ch).unwrap_or(img);
    if new_w == w {
        return Ok(img.to_vec());
    }
    // A protection map that doesn't cover the image protects nothing.
    let protect = protect.filter(|p| p.len() >= w * h);
    let seams = new_w.abs_diff(w).max(1) as f32;
    if new_w < w {
        if h >= PROXY_SHRINK_MIN_HEIGHT && w - new_w >= PROXY_SHRINK_MIN_SEAMS {
            return carve_width_proxy_height(w, h, ch, img, protect, new_w, ctl);
        }
        let mut carver = Carver::new(w, h, ch, img, protect);
        while carver.cw > new_w {
            ctl.check()?;
            ctl.progress((w - carver.cw) as f32 / seams);
            carver.remove_seam();
        }
        return Ok(carver.into_pixels());
    }
    // Enlarge in rounds of at most half the current width, each duplicating the k lowest seams.
    let (mut cur, mut cw) = (img.to_vec(), w);
    let mut prot = protect.map(<[f32]>::to_vec);
    while cw < new_w {
        let k = (new_w - cw).min((cw / 2).max(1));
        // Find k seams on a shrinking copy, mapping each back to original columns.
        let mut carver = Carver::new(cw, h, ch, &cur, prot.as_deref());
        let mut dup: Vec<Vec<usize>> = vec![Vec::with_capacity(k); h];
        for _ in 0..k {
            // A 1 px wide image's only seam is its one column: take it, so it is duplicated.
            if carver.cw == 0 {
                break;
            }
            ctl.check()?;
            ctl.progress((cw + (cw - carver.cw)).saturating_sub(w) as f32 / seams);
            for (d, x) in dup.iter_mut().zip(carver.remove_seam()) {
                d.push(x);
            }
        }
        let added = dup[0].len();
        if added == 0 {
            break;
        }
        let nw = cw + added;
        let mut out = Vec::with_capacity(nw * h * ch);
        let mut nprot = prot.as_ref().map(|_| Vec::with_capacity(nw * h));
        for (y, d) in dup.iter_mut().enumerate() {
            d.sort_unstable();
            let row = &cur[y * cw * ch..(y + 1) * cw * ch];
            let mut di = 0;
            for x in 0..cw {
                out.extend_from_slice(&row[x * ch..(x + 1) * ch]);
                if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                    np.push(p[y * cw + x]);
                }
                while di < d.len() && d[di] == x {
                    let xr = (x + 1).min(cw - 1);
                    for c in 0..ch {
                        out.push((row[x * ch + c] + row[xr * ch + c]) * 0.5);
                    }
                    if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                        np.push(p[y * cw + x]);
                    }
                    di += 1;
                }
            }
        }
        cur = out;
        prot = nprot;
        cw = nw;
    }
    Ok(cur)
}

/// Find large-image seams on vertically averaged rows, then apply those paths to the original
/// pixels. The proxy has the same width, so each seam still removes exactly one source pixel per
/// row. Keeping output pixels at full resolution avoids the quality loss of resizing the result;
/// only the seam decisions use the coarser vertical signal. Small operations retain the exact
/// full-resolution algorithm above.
fn carve_width_proxy_height(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize, ctl: &Interrupt) -> Result<Vec<f32>, Cancelled> {
    let step = h.div_ceil(SEAM_SEARCH_MAX_HEIGHT);
    let proxy_h = h.div_ceil(step);
    let row_len = w * ch;
    let proxy = average_rows(img, row_len, h, step);
    let proxy_protect = protect.map(|mask| average_rows(mask, w, h, step));

    let seam_count = w - new_w;
    let mut carver = Carver::new(w, proxy_h, ch, &proxy, proxy_protect.as_deref());
    let mut removed = Vec::with_capacity(seam_count);
    while carver.cw > new_w {
        ctl.check()?;
        removed.push(carver.remove_seam());
        ctl.progress(removed.len() as f32 / seam_count as f32);
    }

    let mut removed_by_row = Vec::with_capacity(proxy_h);
    for proxy_y in 0..proxy_h {
        let mut columns: Vec<usize> = removed.iter().map(|path| path[proxy_y]).collect();
        columns.sort_unstable();
        removed_by_row.push(columns);
    }
    // Each full-resolution row removes the columns interpolated between its two proxy rows'.
    let rebuild = |columns: &mut Vec<usize>, (y, out): (usize, &mut [f32])| -> Result<(), Cancelled> {
        ctl.check()?;
        let proxy_y = (y / step).min(proxy_h - 1);
        let next_proxy_y = (proxy_y + 1).min(proxy_h - 1);
        let blend = (y % step) as f32 / step as f32;
        let lower = &removed_by_row[proxy_y];
        let upper = &removed_by_row[next_proxy_y];
        columns.clear();
        let mut min_x = 0;
        for i in 0..seam_count {
            let interpolated = lower[i] as f32 + (upper[i] as f32 - lower[i] as f32) * blend;
            let max_x = w - (seam_count - i);
            let x = (interpolated.round() as usize).clamp(min_x, max_x);
            columns.push(x);
            min_x = x.saturating_add(1);
        }
        // The columns rise strictly within the row: copy the runs between them.
        let row = &img[y * row_len..(y + 1) * row_len];
        let (mut from, mut at) = (0, 0);
        for &x in columns.iter() {
            let run = (x - from) * ch;
            out[at..at + run].copy_from_slice(&row[from * ch..x * ch]);
            (from, at) = (x + 1, at + run);
        }
        out[at..].copy_from_slice(&row[from * ch..]);
        Ok(())
    };
    let mut out = vec![0.0f32; new_w * h * ch];
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_exact_mut(new_w * ch).enumerate().try_for_each_init(|| Vec::with_capacity(seam_count), rebuild)?;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut columns = Vec::with_capacity(seam_count);
        out.chunks_exact_mut(new_w * ch).enumerate().try_for_each(|r| rebuild(&mut columns, r))?;
    }
    ctl.progress(1.0);
    Ok(out)
}

/// Content-aware resize of `img` to `new_w × new_h` (width first, then height by transposing).
pub fn carve(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize, new_h: usize) -> Vec<f32> {
    // Never cancelled; the fallback is unreachable.
    carve_with(w, h, ch, img, protect, new_w, new_h, &Interrupt::NONE).unwrap_or_else(|_| img.to_vec())
}

/// [`carve`] that checks `ctl` before every seam (a cancel takes effect within one seam) and
/// reports progress over both passes.
#[allow(clippy::too_many_arguments)]
pub fn carve_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    protect: Option<&[f32]>,
    new_w: usize,
    new_h: usize,
    ctl: &Interrupt,
) -> Result<Vec<f32>, Cancelled> {
    let (new_w, new_h) = (new_w.max(1), new_h.max(1));
    if unusable(w, h, ch, img) {
        return Ok(img.to_vec());
    }
    let img = img.get(..w * h * ch).unwrap_or(img);
    let protect = protect.filter(|p| p.len() >= w * h);
    // Split progress between the passes by their seam counts.
    let (sw, sh) = (new_w.abs_diff(w) as f32, new_h.abs_diff(h) as f32);
    let split = if sw + sh > 0.0 { sw / (sw + sh) } else { 1.0 };
    let cancel = || ctl.cancelled();
    let first = |f: f32| ctl.progress(f * split);
    let wide = carve_width_with(w, h, ch, img, protect, new_w, &Interrupt::new(&cancel, &first))?;
    if new_h == h {
        return Ok(wide);
    }
    let prot_t = protect.map(|p| transpose(new_w, h, 1, &resample_protection(w, h, p, new_w)));
    let t = transpose(new_w, h, ch, &wide);
    let second = |f: f32| ctl.progress(split + f * (1.0 - split));
    let tall = carve_width_with(h, new_w, ch, &t, prot_t.as_deref(), new_h, &Interrupt::new(&cancel, &second))?;
    Ok(transpose(new_h, new_w, ch, &tall))
}

/// The protection map `p` (`w × h`) for the height pass, after the width changed to `new_w`:
/// plainly resampled (it only steers seams). Where values out of `0..=1` resample to NaN
/// (infinities, or neighbours whose difference overflows), the map clamped to `0..=1` (NaN to 0)
/// resampled instead, so an infinite protection still protects.
fn resample_protection(w: usize, h: usize, p: &[f32], new_w: usize) -> Vec<f32> {
    let mut out = resize_bilinear(w, h, 1, p, new_w, h);
    if out.iter().any(|v| v.is_nan()) {
        let clamped: Vec<f32> = p.iter().map(|&v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }).collect();
        for (v, c) in out.iter_mut().zip(resize_bilinear(w, h, 1, &clamped, new_w, h)) {
            if v.is_nan() {
                *v = c;
            }
        }
    }
    out
}

/// Plain bilinear resize (the non-content-aware part of the Amount blend).
pub fn resize_bilinear(w: usize, h: usize, ch: usize, img: &[f32], nw: usize, nh: usize) -> Vec<f32> {
    let Some(len) = nw.checked_mul(nh).and_then(|n| n.checked_mul(ch)) else {
        return Vec::new();
    };
    let mut out = vec![0.0f32; len];
    if len == 0 || unusable(w, h, ch, img) {
        return out;
    }
    let row = |(y, out): (usize, &mut [f32])| {
        let fy = ((y as f32 + 0.5) * h as f32 / nh as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..nw {
            let fx = ((x as f32 + 0.5) * w as f32 / nw as f32 - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..ch {
                let p = |xx: usize, yy: usize| img[(yy * w + xx) * ch + c];
                let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
                let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
                out[x * ch + c] = top + (bot - top) * ty;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(nw * ch).enumerate().for_each(row);
    }
    #[cfg(target_arch = "wasm32")]
    out.chunks_mut(nw * ch).enumerate().for_each(row);
    out
}

/// Skin-tone protection map (1 = skin) for RGB(A) buffers: the explicit RGB rule of
/// J. Kovač, P. Peer, F. Solina, *Human Skin Colour Clustering for Face Detection*, EUROCON 2003.
pub fn skin_mask(w: usize, h: usize, ch: usize, img: &[f32]) -> Vec<f32> {
    let mut m = vec![0.0f32; w * h];
    if ch < 3 {
        return m;
    }
    for (i, v) in m.iter_mut().enumerate() {
        let p = &img[i * ch..i * ch + 3];
        let (r, g, b) = (p[0] * 255.0, p[1] * 255.0, p[2] * 255.0);
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        if r > 95.0 && g > 40.0 && b > 20.0 && mx - mn > 15.0 && (r - g).abs() > 15.0 && r > g && r > b {
            *v = 1.0;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The straightforward implementation the per-seam carver replaced (full energy map, fresh
    /// seam search and a new buffer per seam), kept as the oracle for `carver_matches_reference`.
    mod reference {
        use super::super::{protection, resample_protection, transpose};

        pub(super) fn energy(w: usize, h: usize, ch: usize, img: &[f32]) -> Vec<f32> {
            let mut e = vec![0.0f32; w * h];
            for y in 0..h {
                for x in 0..w {
                    let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
                    let (yu, yd) = (y.saturating_sub(1), (y + 1).min(h - 1));
                    let at = |xx: usize, yy: usize, c: usize| img[(yy * w + xx) * ch + c];
                    let mut s = 0.0;
                    for c in 0..ch {
                        let v = at(x, y, c);
                        s += (at(xr, y, c) - v).abs() + (v - at(xl, y, c)).abs();
                        s += (at(x, yd, c) - v).abs() + (v - at(x, yu, c)).abs();
                    }
                    let v = s * 0.5;
                    e[y * w + x] = if v.is_nan() { f32::INFINITY } else { v };
                }
            }
            e
        }
        pub(super) fn find_seam(w: usize, h: usize, energy: &[f32]) -> Vec<usize> {
            let mut cost = energy[..w].to_vec();
            let mut from = vec![0u8; w * h]; // 0 = up-left, 1 = up, 2 = up-right
            let mut next = vec![0.0f32; w];
            for y in 1..h {
                for x in 0..w {
                    let mut best = (cost[x], 1u8);
                    if x > 0 && cost[x - 1] < best.0 {
                        best = (cost[x - 1], 0);
                    }
                    if x + 1 < w && cost[x + 1] < best.0 {
                        best = (cost[x + 1], 2);
                    }
                    next[x] = best.0 + energy[y * w + x];
                    from[y * w + x] = best.1;
                }
                std::mem::swap(&mut cost, &mut next);
            }
            let mut x = (0..w).min_by(|a, b| cost[*a].total_cmp(&cost[*b])).unwrap_or(0);
            let mut seam = vec![0usize; h];
            for y in (0..h).rev() {
                seam[y] = x;
                if y > 0 {
                    x = match from[y * w + x] {
                        0 => x - 1,
                        2 => x + 1,
                        _ => x,
                    };
                }
            }
            seam
        }
        fn remove_seam(w: usize, h: usize, ch: usize, img: &[f32], seam: &[usize]) -> Vec<f32> {
            let mut out = Vec::with_capacity((w - 1) * h * ch);
            for (y, &sx) in seam.iter().enumerate().take(h) {
                let row = &img[y * w * ch..(y + 1) * w * ch];
                out.extend_from_slice(&row[..sx * ch]);
                out.extend_from_slice(&row[(sx + 1) * ch..]);
            }
            out
        }
        pub(super) fn carve_width(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize) -> Vec<f32> {
            let new_w = new_w.max(1);
            if new_w == w || w == 0 || h == 0 {
                return img.to_vec();
            }
            let protect_e = |e: &mut [f32], p: Option<&Vec<f32>>| {
                if let Some(p) = p {
                    for (e, &p) in e.iter_mut().zip(p) {
                        *e += protection(p);
                    }
                }
            };
            if new_w < w {
                let (mut cur, mut cw) = (img.to_vec(), w);
                let mut prot = protect.map(<[f32]>::to_vec);
                while cw > new_w {
                    let mut e = energy(cw, h, ch, &cur);
                    protect_e(&mut e, prot.as_ref());
                    let seam = find_seam(cw, h, &e);
                    cur = remove_seam(cw, h, ch, &cur, &seam);
                    prot = prot.map(|p| remove_seam(cw, h, 1, &p, &seam));
                    cw -= 1;
                }
                return cur;
            }
            // Enlarge in rounds of at most half the current width, each duplicating the k lowest seams.
            let (mut cur, mut cw) = (img.to_vec(), w);
            let mut prot = protect.map(<[f32]>::to_vec);
            while cw < new_w {
                let k = (new_w - cw).min((cw / 2).max(1));
                // Find k seams on a shrinking copy, mapping each back to original columns.
                let mut idx: Vec<Vec<usize>> = (0..h).map(|_| (0..cw).collect()).collect();
                let (mut tmp, mut tw) = (cur.clone(), cw);
                let mut tprot = prot.clone();
                let mut dup: Vec<Vec<usize>> = vec![Vec::with_capacity(k); h];
                for _ in 0..k {
                    // A 1 px wide image's only seam is its one column: take it, so it is duplicated.
                    if tw == 0 {
                        break;
                    }
                    let mut e = energy(tw, h, ch, &tmp);
                    protect_e(&mut e, tprot.as_ref());
                    let seam = find_seam(tw, h, &e);
                    for (y, &sx) in seam.iter().enumerate() {
                        dup[y].push(idx[y].remove(sx));
                    }
                    tmp = remove_seam(tw, h, ch, &tmp, &seam);
                    tprot = tprot.map(|p| remove_seam(tw, h, 1, &p, &seam));
                    tw -= 1;
                }
                let added = dup[0].len();
                if added == 0 {
                    break;
                }
                let nw = cw + added;
                let mut out = Vec::with_capacity(nw * h * ch);
                let mut nprot = prot.as_ref().map(|_| Vec::with_capacity(nw * h));
                for (y, d) in dup.iter_mut().enumerate() {
                    d.sort_unstable();
                    let row = &cur[y * cw * ch..(y + 1) * cw * ch];
                    let mut di = 0;
                    for x in 0..cw {
                        out.extend_from_slice(&row[x * ch..(x + 1) * ch]);
                        if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                            np.push(p[y * cw + x]);
                        }
                        while di < d.len() && d[di] == x {
                            let xr = (x + 1).min(cw - 1);
                            for c in 0..ch {
                                out.push((row[x * ch + c] + row[xr * ch + c]) * 0.5);
                            }
                            if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                                np.push(p[y * cw + x]);
                            }
                            di += 1;
                        }
                    }
                }
                cur = out;
                prot = nprot;
                cw = nw;
            }
            cur
        }

        pub(super) fn carve(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize, new_h: usize) -> Vec<f32> {
            let (new_w, new_h) = (new_w.max(1), new_h.max(1));
            let wide = carve_width(w, h, ch, img, protect, new_w);
            if new_h == h {
                return wide;
            }
            let prot_t = protect.map(|p| transpose(new_w, h, 1, &resample_protection(w, h, p, new_w)));
            let t = transpose(new_w, h, ch, &wide);
            let tall = carve_width(h, new_w, ch, &t, prot_t.as_deref(), new_h);
            transpose(new_h, new_w, ch, &tall)
        }
    }

    /// The carver gives bit-identical output to the reference implementation it replaced, over
    /// random shapes (including 1 px wide and tall), channel counts, tie-heavy quantised noise,
    /// protection, shrinking, enlarging past half the width and both axes at once.
    #[test]
    fn carver_matches_reference() {
        let mut seed = 1u32;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize % n
        };
        for case in 0..400u32 {
            let (w, h, ch) = (1 + next(24), 1 + next(18), [1, 3, 4][next(3)]);
            let levels = [2, 5, 255, 65_536][next(4)];
            let img = noise(w * h * ch, levels, 11 + case);
            let protect = (next(3) == 0).then(|| noise(w * h, 3, 101 + case));
            let (nw, nh) = (1 + next(2 * w + 2), 1 + next(2 * h + 2));
            let got = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
            let want = reference::carve(w, h, ch, &img, protect.as_deref(), nw, nh);
            let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&got), bits(&want), "case {case}: {w}x{h}x{ch} -> {nw}x{nh}, levels {levels}, protect {}", protect.is_some());
        }
        // The seam search alone, on raw energy maps full of ties.
        for case in 0..200u32 {
            let (w, h) = (1 + next(40), 1 + next(30));
            let e = noise(w * h, [2, 3, 1000][next(3)], 7 + case);
            assert_eq!(find_seam(w, h, &e), reference::find_seam(w, h, &e), "seam case {case}: {w}x{h}");
            let img = noise(w * h * 3, 7, 3 + case);
            let bits = |v: Vec<f32>| v.into_iter().map(f32::to_bits).collect::<Vec<_>>();
            assert_eq!(bits(energy(w, h, 3, &img)), bits(reference::energy(w, h, 3, &img)), "energy case {case}");
        }
    }

    /// After every removal, the carver's incrementally updated seam search matches a full pass
    /// over its current energy bit for bit (path costs and the seam traced from them), on images
    /// where seams run through the edges, through flat areas full of ties and through protection.
    /// Its energy matches a fresh map of the pixels left, and so do the pixels.
    #[test]
    fn incremental_dp_matches_full_dp() {
        let mut ramp = vec![0.0f32; 120 * 90];
        for (i, v) in ramp.iter_mut().enumerate() {
            let (x, y) = ((i % 120) as f32, (i / 120) as f32);
            *v = ((x * 0.07).sin() * (y * 0.05).cos() * 0.5 + 0.5).clamp(0.0, 1.0);
        }
        // NaN and infinite samples and NaN protection: infinite energies, never NaN costs.
        let mut odd = noise(40 * 30 * 3, 255, 11);
        odd.iter_mut().step_by(13).for_each(|v| *v = f32::NAN);
        odd.iter_mut().skip(5).step_by(17).for_each(|v| *v = f32::INFINITY);
        let mut odd_prot = noise(40 * 30, 3, 12);
        odd_prot.iter_mut().step_by(7).for_each(|v| *v = -f32::NAN);
        // (w, h, channels, image, protection, seams to remove)
        type Case = (usize, usize, usize, Vec<f32>, Option<Vec<f32>>, usize);
        let cases: Vec<Case> = vec![
            (120, 90, 1, ramp, None, 40),
            (97, 64, 3, noise(97 * 64 * 3, 2, 5), None, 90),
            (80, 70, 4, noise(80 * 70 * 4, 1000, 6), Some(noise(80 * 70, 3, 7)), 60),
            (64, 50, 3, bar(64, 50), None, 63),
            (33, 1, 1, noise(33, 5, 8), None, 32),
            (2, 40, 3, noise(2 * 40 * 3, 9, 9), None, 2),
            (50, 40, 1, vec![0.5; 50 * 40], Some(noise(50 * 40, 2, 10)), 49),
            (40, 30, 3, odd, Some(odd_prot), 25),
        ];
        let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        for (case, (w, h, ch, img, prot, seams)) in cases.into_iter().enumerate() {
            let mut c = Carver::new(w, h, ch, &img, prot.as_deref());
            for step in 0..=seams {
                let rows = Rows { stride: c.w, cw: c.cw, off: &c.off };
                let mut fresh = Dp::default();
                fresh.full(rows, &c.e);
                for y in 0..h {
                    assert_eq!(bits(&c.dp.cost[rows.of(y)]), bits(&fresh.cost[rows.of(y)]), "case {case} step {step} row {y}: costs");
                }
                assert_eq!(c.dp.seam(rows), fresh.seam(rows), "case {case} step {step}: seam");
                // The pixels left, packed, and their energy (plus protection) from scratch.
                let left: Vec<f32> = (0..h)
                    .flat_map(|y| c.idx[rows.of(y)].iter().flat_map(move |&x| (0..ch).map(move |k| (y * w + x as usize) * ch + k)))
                    .map(|i| img[i])
                    .collect();
                let mut want = energy(c.cw, h, ch, &left);
                if let Some(p) = &prot {
                    let kept = (0..h).flat_map(|y| c.idx[rows.of(y)].iter().map(move |&x| y * w + x as usize));
                    for (e, i) in want.iter_mut().zip(kept) {
                        *e += protection(p[i]);
                    }
                }
                let have: Vec<f32> = (0..h).flat_map(|y| c.e[rows.of(y)].to_vec()).collect();
                assert_eq!(bits(&have), bits(&want), "case {case} step {step}: energy");
                if step == seams {
                    assert_eq!(bits(&c.into_pixels()), bits(&left), "case {case}: pixels");
                    break;
                }
                c.remove_seam();
            }
        }
    }

    /// Hostile but well-formed shapes return early instead of panicking: no channels, empty
    /// sizes (also through the height pass), and images or protection maps shorter than their
    /// size says (a short map protects nothing).
    #[test]
    fn degenerate_inputs_do_not_panic() {
        assert!(carve(3, 3, 0, &[], None, 2, 2).is_empty());
        assert!(carve(3, 3, 0, &[], None, 5, 5).is_empty());
        assert!(transpose(3, 3, 0, &[]).is_empty());
        assert!(resize_bilinear(3, 3, 0, &[], 2, 2).is_empty());
        assert!(resize_bilinear(3, 3, 1, &[0.5; 9], 0, 2).is_empty());
        assert!(resize_bilinear(3, 3, 1, &[0.5; 9], 2, 0).is_empty());
        assert_eq!(resize_bilinear(3, 3, 1, &[0.5; 4], 2, 2), vec![0.0; 4]);
        assert!(carve(0, 5, 3, &[], None, 3, 7).is_empty());
        assert!(carve(0, 5, 3, &[], None, 3, 2).is_empty());
        let img = noise(10 * 5, 7, 3);
        assert_eq!(carve(10, 5, 1, &img, Some(&[0.0; 3]), 4, 5), carve(10, 5, 1, &img, None, 4, 5));
        assert_eq!(carve(10, 5, 1, &img, Some(&[0.0; 3]), 4, 3), carve(10, 5, 1, &img, None, 4, 3));
        assert_eq!(carve(10, 5, 3, &[0.5; 20], None, 4, 5), vec![0.5; 20]);
        assert_eq!(carve_width(usize::MAX, 2, 4, &[0.5; 8], None, 1), vec![0.5; 8]);
        // Values past the size are ignored, through both passes.
        let long = noise(20, 7, 4);
        assert_eq!(carve(5, 3, 1, &long, None, 3, 3), carve(5, 3, 1, &long[..15], None, 3, 3));
        assert_eq!(carve(5, 3, 1, &long, None, 5, 2), carve(5, 3, 1, &long[..15], None, 5, 2));
    }

    /// Infinite samples, and finite ones whose differences overflow, give energies that aren't
    /// numbers; those are the highest energy (pinned deliberately: the code before NaN-free costs
    /// fed NaN to the seam search and chose differently).
    #[test]
    fn infinite_samples_carve_deterministically() {
        let inf = f32::INFINITY;
        assert_eq!(carve(2, 1, 1, &[inf, 0.5], None, 1, 1), [0.5]);
        assert_eq!(carve(2, 1, 1, &[f32::MAX, f32::MAX], None, 6, 1), [f32::MAX, inf, inf, inf, inf, f32::MAX]);
        let mut img = noise(30 * 20 * 3, 255, 41);
        img.iter_mut().step_by(11).for_each(|v| *v = inf);
        img.iter_mut().skip(4).step_by(23).for_each(|v| *v = -inf);
        let out = carve(30, 20, 3, &img, None, 21, 14);
        assert_eq!(out.len(), 21 * 14 * 3);
        assert_eq!(out, carve(30, 20, 3, &img, None, 21, 14));
    }

    /// Infinite protection protects like full protection in the height pass too, where the map
    /// is resampled (and an infinity alone would resample to NaN).
    #[test]
    fn infinite_protection_protects_in_both_passes() {
        let (w, h) = (6, 6);
        let img: Vec<f32> = (0..w * h).map(|i| (i * 7 % 5) as f32 / 4.0).collect();
        let row = |v: f32| (0..w * h).map(|i| if i / w == 2 { v } else { 0.0 }).collect::<Vec<f32>>();
        let (full, inf) = (row(1.0), row(f32::INFINITY));
        for (nw, nh) in [(6, 3), (4, 3), (3, 6)] {
            assert_eq!(carve(w, h, 1, &img, Some(&inf), nw, nh), carve(w, h, 1, &img, Some(&full), nw, nh), "{nw}x{nh}");
        }
        let kept = carve(w, h, 1, &img, Some(&inf), 6, 3);
        assert!(kept.chunks(6).any(|r| r == &img[12..18]), "the protected row survives");
    }

    /// A cancel that arrives after the seam search, during the tall-image rebuild, still stops
    /// the carve.
    #[test]
    fn proxy_rebuild_observes_cancellation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (w, h, ch, new_w) = (128, PROXY_SHRINK_MIN_HEIGHT, 3, 64);
        let img = noise(w * h * ch, 7, 5);
        let calls = AtomicUsize::new(0);
        // One check per seam passes; the first one after the search cancels.
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= w - new_w;
        assert_eq!(carve_width_with(w, h, ch, &img, None, new_w, &Interrupt::cancel_only(&cancel)), Err(Cancelled));
        assert!(calls.load(Ordering::Relaxed) > w - new_w, "the rebuild checked for cancellation");
    }

    /// NaN samples and NaN protection steer seams the same whatever their sign and payload
    /// (which arithmetic does not preserve consistently across builds): the same pixels are kept.
    #[test]
    fn nan_payloads_do_not_steer_seams() {
        let (w, h, ch) = (24, 18, 3);
        let (base, prot_base) = (noise(w * h * ch, 255, 21), noise(w * h, 3, 22));
        let carve_with_nans = |pixel: f32, prot: f32| {
            let (mut img, mut p) = (base.clone(), prot_base.clone());
            img.iter_mut().step_by(37).for_each(|v| *v = pixel);
            p.iter_mut().step_by(29).for_each(|v| *v = prot);
            carve(w, h, ch, &img, Some(&p), 15, 11)
        };
        let canonical = |v: Vec<f32>| v.into_iter().map(|f| if f.is_nan() { u32::MAX } else { f.to_bits() }).collect::<Vec<_>>();
        let nans = [f32::NAN, -f32::NAN, f32::from_bits(0x7fc0_1234), f32::from_bits(0xffc0_0001)];
        let want = canonical(carve_with_nans(nans[0], nans[0]));
        for &a in &nans {
            for &b in &nans {
                assert_eq!(canonical(carve_with_nans(a, b)), want, "pixel NaN {:#x}, protection NaN {:#x}", a.to_bits(), b.to_bits());
            }
        }
    }

    #[test]
    fn cancelled_carve_stops() {
        let (w, h) = (40, 20);
        let img = vec![0.5f32; w * h * 3];
        let yes = || true;
        assert_eq!(carve_with(w, h, 3, &img, None, 30, 15, &Interrupt::cancel_only(&yes)), Err(Cancelled));
        assert_eq!(carve_with(w, h, 3, &img, None, 30, 15, &Interrupt::NONE).unwrap(), carve(w, h, 3, &img, None, 30, 15));
    }

    /// A flat grey image with a salient vertical bar at x = 10..14 (red/blue columns).
    fn bar(w: usize, h: usize) -> Vec<f32> {
        let mut v = vec![0.5f32; w * h * 3];
        for y in 0..h {
            for x in 10..14 {
                let c = if x % 2 == 0 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
                v[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
            }
        }
        v
    }

    /// Columns that are pure red or pure blue top to bottom.
    fn red_columns(w: usize, h: usize, img: &[f32]) -> usize {
        let pure = |p: &[f32]| p[1] < 0.05 && ((p[0] > 0.95 && p[2] < 0.05) || (p[2] > 0.95 && p[0] < 0.05));
        (0..w).filter(|x| (0..h).all(|y| pure(&img[(y * w + x) * 3..(y * w + x) * 3 + 3]))).count()
    }

    #[test]
    fn seam_follows_low_energy() {
        let (w, h) = (5, 4);
        let mut e = vec![9.0f32; w * h];
        for y in 0..h {
            e[y * w + 2] = 0.0;
        }
        assert_eq!(find_seam(w, h, &e), vec![2; 4]);
    }

    #[test]
    fn shrinking_keeps_the_salient_bar() {
        let (w, h) = (40, 12);
        let img = bar(w, h);
        let out = carve(w, h, 3, &img, None, 24, h);
        assert_eq!(out.len(), 24 * h * 3);
        assert_eq!(red_columns(24, h, &out), 4, "the bar survives intact");
        // Plain resampling squashes it instead.
        let plain = resize_bilinear(w, h, 3, &img, 24, h);
        assert!(red_columns(24, h, &plain) < 4);
    }

    #[test]
    fn tall_proxy_shrink_preserves_full_resolution_salient_and_protected_pixels() {
        let (w, h, ch, new_w) = (512, PROXY_SHRINK_MIN_HEIGHT, 3, 384);
        let mut img = bar(w, h);
        let mut protect = vec![0.0; w * h];
        for y in 0..h {
            let i = (y * w + 256) * ch;
            img[i..i + ch].copy_from_slice(&[0.03, 0.87, 0.42]);
            protect[y * w + 256] = 1.0;
        }

        let out = carve_width_with(w, h, ch, &img, Some(&protect), new_w, &Interrupt::NONE).unwrap();

        assert_eq!(out.len(), new_w * h * ch);
        assert_eq!(red_columns(new_w, h, &out), 4, "the full-resolution bar survives");
        for y in 0..h {
            let row = &out[y * new_w * ch..(y + 1) * new_w * ch];
            assert!(row.chunks_exact(ch).any(|px| px == [0.03, 0.87, 0.42]), "protected mark survives row {y}");
        }
    }

    #[test]
    fn tall_proxy_shrink_observes_cancellation_before_modifying_source() {
        let (w, h, ch) = (128, PROXY_SHRINK_MIN_HEIGHT, 3);
        let img = vec![0.5; w * h * ch];
        let cancelled = || true;
        assert_eq!(carve_width_with(w, h, ch, &img, None, 64, &Interrupt::cancel_only(&cancelled)), Err(Cancelled));
    }

    #[test]
    fn enlarging_duplicates_flat_areas() {
        let (w, h) = (30, 8);
        let img = bar(w, h);
        let out = carve(w, h, 3, &img, None, 50, h);
        assert_eq!(out.len(), 50 * h * 3);
        assert_eq!(red_columns(50, h, &out), 4);
    }

    #[test]
    fn height_and_protection() {
        let (w, h) = (20, 30);
        // Horizontal bar: carve height.
        let mut img = vec![0.2f32; w * h];
        for x in 0..w {
            img[12 * w + x] = 1.0;
        }
        let out = carve(w, h, 1, &img, None, w, 18);
        assert_eq!(out.len(), w * 18);
        assert_eq!((0..18).filter(|y| (0..w).all(|x| out[y * w + x] > 0.9)).count(), 1);
        // Protecting a flat region keeps it: the seams go elsewhere.
        let flat = vec![0.5f32; 16 * 4];
        let mut protect = vec![0.0f32; 16 * 4];
        for y in 0..4 {
            protect[y * 16 + 3] = 1.0;
        }
        let mut marked = flat.clone();
        for y in 0..4 {
            marked[y * 16 + 3] = 0.51;
        }
        let out = carve(16, 4, 1, &marked, Some(&protect), 8, 4);
        assert!((0..4).all(|y| out[y * 8..y * 8 + 8].iter().any(|v| (*v - 0.51).abs() < 1e-6)));
    }

    /// Noise quantised to `levels` values, so the energy map is full of exact ties.
    fn noise(len: usize, levels: u32, seed: u32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s % levels) as f32 / (levels - 1) as f32
            })
            .collect()
    }

    fn fnv(v: &[f32]) -> u64 {
        v.iter().flat_map(|f| f.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    }

    /// The exact output bits of a spread of carves (shrink, enlarge past half the width, both
    /// axes, protection, 1/3/4 channels, tie-heavy and smooth images), pinned so that speed work
    /// can't change which seams are chosen or how pixels are blended.
    #[test]
    fn output_bits_are_pinned() {
        let cases: [(usize, usize, usize, u32, bool, usize, usize); 8] = [
            (61, 47, 3, 2, false, 40, 47),
            (61, 47, 3, 255, false, 61, 30),
            (61, 47, 4, 4, true, 35, 29),
            (33, 20, 1, 3, false, 80, 20),
            (33, 20, 3, 1000, true, 50, 45),
            (48, 36, 4, 7, false, 70, 18),
            (5, 3, 1, 2, false, 1, 1),
            (40, 40, 3, 2, true, 39, 41),
        ];
        let got: Vec<u64> = cases
            .iter()
            .enumerate()
            .map(|(i, &(w, h, ch, levels, prot, nw, nh))| {
                let img = noise(w * h * ch, levels, 7 + i as u32);
                let protect = prot.then(|| noise(w * h, 3, 99 + i as u32));
                let out = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
                assert_eq!(out.len(), nw * nh * ch);
                fnv(&out)
            })
            .collect();
        assert_eq!(got, PINNED);
    }

    const PINNED: [u64; 8] = [
        0x24ea_9353_02c2_7c58,
        0x5a6e_8554_a0cb_ebb1,
        0x8b18_d63f_396e_21c9,
        0x1db2_23ad_3e6a_a7c5,
        0x721d_6cc5_34e0_3fdf,
        0x961a_ea6d_e512_ec7d,
        0x4d25_767f_9dce_13f5,
        0x2157_9f86_27d6_b835,
    ];

    /// [`output_bits_are_pinned`] for the reduced-height seam search of tall images (rows at
    /// least [`PROXY_SHRINK_MIN_HEIGHT`], at least [`PROXY_SHRINK_MIN_SEAMS`] seams): widths and,
    /// through the transposed pass, heights, with and without protection, on tie-heavy and
    /// fine-grained noise.
    #[test]
    fn tall_proxy_output_bits_are_pinned() {
        let cases: [(usize, usize, usize, u32, bool, usize, usize); 5] = [
            (96, PROXY_SHRINK_MIN_HEIGHT, 3, 2, false, 30, PROXY_SHRINK_MIN_HEIGHT),
            (130, 2_100, 4, 255, true, 64, 2_100),
            (80, 2_600, 1, 65_536, false, 16, 2_600),
            (PROXY_SHRINK_MIN_HEIGHT, 90, 3, 5, false, PROXY_SHRINK_MIN_HEIGHT, 20),
            (2_050, 100, 4, 1000, true, 2_050, 36),
        ];
        let got: Vec<u64> = cases
            .iter()
            .enumerate()
            .map(|(i, &(w, h, ch, levels, prot, nw, nh))| {
                let img = noise(w * h * ch, levels, 31 + i as u32);
                let protect = prot.then(|| noise(w * h, 3, 131 + i as u32));
                let out = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
                assert_eq!(out.len(), nw * nh * ch);
                fnv(&out)
            })
            .collect();
        assert_eq!(got, TALL_PINNED);
    }

    const TALL_PINNED: [u64; 5] = [0x5657_83ac_cb26_56a5, 0x81a6_b5f7_e299_fdfb, 0xcdf2_9df9_9b83_def5, 0xa713_544a_4dcc_d8d8, 0x3557_de9f_ffb0_881e];

    #[test]
    fn one_pixel_lines_enlarge_by_repeating() {
        // Width 1 has no seam to spare: its one column is repeated. Height 1 likewise (through
        // the transposed pass), alone or with the other axis.
        let col = [0.1f32, 0.2, 0.3, 0.4, 0.5];
        assert_eq!(carve(1, 5, 1, &col, None, 4, 5), col.iter().flat_map(|v| [*v; 4]).collect::<Vec<_>>());
        assert_eq!(carve(5, 1, 1, &col, None, 5, 3), col.repeat(3));
        assert_eq!(carve(1, 1, 3, &[0.7, 0.8, 0.9], None, 3, 2), [0.7, 0.8, 0.9].repeat(6));
        for (w, h, nw, nh) in [(1, 5, 6, 10), (1, 17, 6, 34), (16, 1, 21, 2), (7, 1, 3, 8), (2, 1, 1, 8)] {
            let img = vec![0.5f32; w * h * 4];
            assert_eq!(carve(w, h, 4, &img, Some(&vec![1.0; w * h]), nw, nh).len(), nw * nh * 4, "{w}x{h} -> {nw}x{nh}");
        }
    }

    #[test]
    fn skin_rule() {
        let px = [0.86f32, 0.62, 0.5, 0.2, 0.4, 0.8];
        assert_eq!(skin_mask(2, 1, 3, &px), vec![1.0, 0.0]);
    }
}
