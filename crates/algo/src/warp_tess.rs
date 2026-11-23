//! How the reference application cuts a one-patch warp into flat triangles: an envelope (a
//! preset's mesh, stored with fewer than four control points along a side) or a full 4 × 4 mesh.
//! Derived from coordinate probes of its warps (observation only).
//!
//! The surface is cut as a quadtree, starting from 4 × 4 cells; a full mesh is cut by its cubic
//! net (even where it is flatter), starting from 2 × 2 cells:
//!
//! * a cell splits across (`u`) while one of its two rows, restricted to the cell and taken at its
//!   stored degree, has an inner control point at least [`PERP_TOL`] off the chord sideways, or its
//!   inner control points lie on average at least [`TANG_TOL`] along the chord from their even
//!   spacing; the same test on its two columns splits it down (`v`). An inner point turned back
//!   before the chord's start or past its end counts by its distance from that end (see [`need`]);
//! * for a mesh that is cubic in one direction, a cell whose control net twists by more than
//!   [`TWIST_TOL`] splits along its longer side (a square cell: down the document). A full mesh's
//!   twist is the largest `P00 − P10 − P01 + P11` of the cell's net, for a split down of its net's
//!   top row of quads only (the reference cuts a cell twisted along its top edge, not its mirror
//!   image twisted along its bottom). An envelope's is how unevenly a quad of the cell's
//!   stored-degree net is stretched (the larger difference in length between two opposite sides
//!   of it): for a split across, over the net's left column of quads only (the reference splits
//!   Arc Lower's top-left corner cell, not its mirror image on the right), for a split down over
//!   its top row;
//! * a 3 × 3 envelope's wide cell splits across (never down) where the legs of the left column of
//!   its control points are more than [`TWIST_TOL`] longer than the next column's, in the cell or
//!   in the other half of the cell it was cut from;
//! * a square cell of a tall envelope (stored with more control points down than across: the
//!   presets bent sideways) whose whole net twists by more than [`TALL_SQUARE_TWIST_TOL`] splits
//!   across too;
//! * a full mesh measures each cell also by every row and column of the cell's own control net,
//!   with the drift of its rows' first inner point weighed in ([`FIRST_POINT_ROW_WEIGHT`]); a
//!   cell twisted past [`BOTH_WAYS_TWIST_TOL`] splits both ways, and one split down for its twist
//!   splits the cells beside it in its row down too;
//! * a full mesh's first-level cell split across twice splits down too; a split down on its first
//!   level carries to the cell beside it, and to all four where its columns ask for
//!   [`FIRST_LEVEL_SPREAD_FV`]. An envelope's split down on the first level splits the other
//!   first-level cells of the same quarter of the surface down too;
//! * for a mesh that is cubic in one direction, a square cell split down for its columns'
//!   curvature splits its sibling beside it (the other cell of its parent's row) down.
//!
//! Each cell becomes two triangles, split from its top-left corner, cut across at the vertices its
//! left and right neighbours put on its sides (never at a side on the surface's middle line,
//! u = ½: the cell stays whole there). A new vertex on a row whose own segment wasn't curved
//! enough to need it is dropped onto that segment's chord, perpendicular from its point on the
//! surface, unless it is inside a cell that was split; on the left and right sides of the surface
//! the same holds for columns. The starting grid's own points stay on the surface, with two
//! exceptions that follow the rule too: every point but the corners on the surface's top and
//! bottom edges (an edge that needs no split is drawn straight from corner to corner), and an
//! envelope's quarter points on its middle row. Inside an envelope that isn't tall, a new vertex
//! on a column goes onto its column segment's chord only where that segment is nearly straight
//! (under [`FLAT_COLUMN_TOL`] of a split). A vertex inside a bigger cell's edge is instead dropped
//! onto that edge the same way, so it keeps its place along the edge (the bigger cell's texture
//! runs on straight past it, as the reference's does).
//!
//! A tall envelope is cut in place by the same rules (not as its mirror image about the diagonal),
//! with two differences in where the vertices go: its cells stay whole beside smaller neighbours
//! (a neighbour's vertex sits on the cell's side), and the chord rule for rows holds for all its
//! columns too, from [`TALL_COLUMN_LEVEL`] halvings on.

use std::collections::{HashMap, HashSet};

/// A point in document pixels or normalised source coordinates.
type P = [f64; 2];

/// Sideways deviation (document px) of an inner control point that splits a segment.
const PERP_TOL: f64 = 1.0;
/// Mean deviation along the chord (document px) of the inner control points that splits it.
const TANG_TOL: f64 = 0.5;
/// How far (document px) a row segment's first and last inner control points drift apart along
/// its chord (one ahead of its even spacing, the other behind) before a new point on that row
/// keeps its place on the surface instead of dropping onto the chord. The reference application's
/// lumpy edges keep the point from a difference of about 0.5 px and drop it at 0.33 px or less,
/// whatever the segment's sideways bow below [`PERP_TOL`]. The same holds for a point on a side
/// tested by the row rule turned on its side, except in a quilt's patch with a seam left or right.
const UNEVEN_TOL: f64 = 0.42;
/// Distance (document px) of an inner control point before its chord's start that splits a
/// segment, as the reference application's creases show (rows pulled back over themselves split
/// later there than past the chord's end).
const BEFORE_TOL: f64 = 1.3;
/// Distance (document px) of an inner control point past its chord's end that splits a segment.
const AFTER_TOL: f64 = 1.0;
/// A segment that turns back only a little (the fold's apex lies inside it: no inner control point
/// reaches past [`SHALLOW_AFTER_REACH`] of the chord's length from its start) splits past its end
/// from this distance on; the crease probes' cells around a fold's apex split this early.
const SHALLOW_AFTER_TOL: f64 = 0.45;
/// How far along the chord (in chord lengths, from its start) an inner control point may lie for
/// [`SHALLOW_AFTER_TOL`] to apply.
const SHALLOW_AFTER_REACH: f64 = 1.7;
/// The same before the chord's start: [`BEFORE_TOL`] becomes this when no inner control point lies
/// more than [`SHALLOW_BEFORE_REACH`] chord lengths before it.
const SHALLOW_BEFORE_TOL: f64 = 0.5;
/// See [`SHALLOW_BEFORE_TOL`].
const SHALLOW_BEFORE_REACH: f64 = 0.42;
/// A segment turning back sharply past its chord's end (its inner control points within this many
/// chord lengths of each other along it, the furthest past the end) splits past its end from
/// [`SHARP_FOLD_TOL`] times its furthest reach (in chord lengths), at most [`SHARP_FOLD_MAX`], in
/// place of [`SHALLOW_AFTER_TOL`] or [`AFTER_TOL`]: the reference application's 16-bit crease
/// redraws at four scales show that limit (0.34 to 0.35 times the reach from 1.35 to 2.05 chord
/// lengths, about 0.7 px at 3.3).
const SHARP_FOLD_SPAN: f64 = 0.35;
/// See [`SHARP_FOLD_SPAN`].
const SHARP_FOLD_TOL: f64 = 0.34;
/// See [`SHARP_FOLD_SPAN`].
const SHARP_FOLD_MAX: f64 = 0.7;
/// The twist along a whole quilt patch's rows (document px) past which the patch splits across.
const ROW_TWIST_ACROSS_TOL: f64 = 1.25;
/// Twist of a cell's control net (document px) past which it splits.
const TWIST_TOL: f64 = 1.0;
/// The twist of a quilt patch's half-height strip's top row of control quads past which the
/// strip's cells split down (see [`strip_down_cuts`]).
const STRIP_TOP_TWIST_TOL: f64 = 2.0;
/// The largest twist of the other strip, per [`STRIP_WIDTH`] px of the patch's width, past which a
/// strip's cells split down as well.
const STRIP_OTHER_TWIST_TOL: f64 = 2.5;
/// See [`STRIP_OTHER_TWIST_TOL`]; patches narrower than this count as this wide.
const STRIP_WIDTH: f64 = 64.0;
/// The same for a whole patch of a quilt: the reference application leaves a patch whose twist is
/// spread evenly (a wave that grows linearly down the patch) whole up to about 1.73 and splits it
/// from about 1.85.
const WHOLE_PATCH_TWIST_TOL: f64 = 1.8;
/// The twist past which a whole patch of a quilt whose patches don't halve evenly (three rows, or
/// rows of different heights) splits down, measured on its first row of quads only: lumpy shapes
/// keep a top patch whose first row twists by 1.76 whole (at any turn of the twist), and waves keep
/// a bottom patch whose first row twists by 1.87 or 1.93 whole and split one at 2.24.
const FIRST_ROW_TWIST_TOL: f64 = 2.07;
/// The twist along its rows past which a cell as tall as its patch splits down in a quilt whose
/// patches don't halve evenly ([`TWIST_TOL`] in an even quilt): such cells stay whole at 0.96 and
/// 1.06 px (a lumpy shape at 100 % and 110 %) and split at 1.20 and 1.28 px (a wave of three by
/// three patches, a bend three patches down). It is [`TWIST_TOL`] raised by the same factor that
/// raises [`WHOLE_PATCH_TWIST_TOL`] to [`FIRST_ROW_TWIST_TOL`].
const UNEVEN_FULL_HEIGHT_TWIST_TOL: f64 = 1.15;
/// How much more an inner control row of a quilt cell's net counts than its top and bottom rows
/// when the cell is measured for a split across: cells whose largest need comes from an inner row
/// split from 0.9972 (and stay whole at 0.9900), while cells measured by their top or bottom row
/// stay whole up to 0.9999.
const INNER_ROW_WEIGHT: f64 = 1.005;
/// In a quilt's patch a cell's row asks for a split across where its inner control points drift
/// unevenly along it ([`uneven_drift`]) by this much (document px), and a column asks for a split
/// down from [`QUILT_COL_UNEVEN_TOL`]. Lumpy meshes split 3 × 2 and 2 × 3 show both: rows split
/// from 0.65 and columns stay whole at 0.674.
const QUILT_ROW_UNEVEN_TOL: f64 = 0.62;
/// See [`QUILT_ROW_UNEVEN_TOL`].
const QUILT_COL_UNEVEN_TOL: f64 = 0.72;
/// A cell of a one-patch full mesh twisted past this splits both ways (probes with one inner
/// control point moved: cells at 2.25 px split across and down, at 1.75 px only the one way).
const BOTH_WAYS_TWIST_TOL: f64 = 2.0;
/// A first-level split down of a one-patch full mesh carries to the other row only from a cell
/// whose columns ask for this much (moved control points: 1.12 stays in its row, 1.55 and up
/// carry to all four cells).
const FIRST_LEVEL_SPREAD_FV: f64 = 1.5;
/// A one-patch full mesh measures a cell's side rows also by their first inner control point's
/// drift along the chord, at this weight: the reference cuts the cell left of a control point
/// moved across (its rows' first point lags) but not the one right of it, where the second point
/// lags as much (moved-point probes and their mirror images; 0.7 to 0.75 fit both).
const FIRST_POINT_ROW_WEIGHT: f64 = 0.75;
/// [`FIRST_POINT_ROW_WEIGHT`] for a first cell (half the mesh wide) on the mesh's left edge
/// (u = 0): its rows' first inner point counts in full. With an inner control point moved 11 to
/// 14 px across, the reference application cuts these cells across where the same drift
/// elsewhere stays whole (0.97 to 1.25 fit; from 1.3 a probe moved 8 px is cut too).
const LEFT_EDGE_FIRST_POINT_WEIGHT: f64 = 1.0;
/// A fold tip: on a chord shorter than this (document px) whose inner points reach past its end
/// once and never before its start, the point past the end still counts toward the mean drift
/// along the chord, held at the end. Crease redraws keep such narrow tips whole where the inside
/// point alone would cut them (crease probes from 0.07 to 1.24 px; 1.5 and 3 fit alike).
const FOLD_TIP_CHORD: f64 = 1.5;
/// The twist of a tall envelope's square cell (its whole net) past which it splits across as well:
/// sideways Fish and Wave at ±50 match with 2.0 and lose most of their cells' agreement at 1.2.
const TALL_SQUARE_TWIST_TOL: f64 = 2.0;
/// Halvings down to which a tall envelope's column points stay on the surface: eighths stay,
/// sixteenths and finer follow the chord rule.
const TALL_COLUMN_LEVEL: u32 = 3;
/// How much of a split (see [`need`]) a stretch of an inner column of an envelope that isn't tall
/// may ask for and still count as straight: the points halfway along it sit on its chord.
const FLAT_COLUMN_TOL: f64 = 0.5;
/// Cells per side an envelope starts as.
const START: usize = 4;
/// Cells per side a full 4 × 4 mesh starts as.
const START_FULL: usize = 2;
/// Cells this narrow (normalised) are not split again: six halvings of an envelope's first cells.
const MIN_CELL: f64 = 1.0 / 64.0;
/// Comparisons allow this much rounding.
const EPS: f64 = 1e-6;
/// A point this near its chord (document px) is on it.
const ON_CHORD: f64 = 1e-9;
/// Most cells one tessellation makes.
const MAX_CELLS: usize = 1 << 16;

/// A control net: `nu` points across (u) by `nv` down (v), row-major (rows are v).
#[derive(Clone, Debug)]
struct Net {
    nu: usize,
    nv: usize,
    p: Vec<P>,
}

impl Net {
    fn at(&self, i: usize, j: usize) -> P {
        self.p.get(j * self.nu + i).copied().unwrap_or([f64::NAN, f64::NAN])
    }
    /// The net's `j`th row of control points.
    fn ctrl_row(&self, j: usize) -> Vec<P> {
        (0..self.nu).map(|i| self.at(i, j)).collect()
    }
    /// The net's `i`th column of control points.
    fn ctrl_col(&self, i: usize) -> Vec<P> {
        (0..self.nv).map(|j| self.at(i, j)).collect()
    }
    /// The row curve at `v` (control points across).
    fn row(&self, v: f64) -> Vec<P> {
        (0..self.nu).map(|i| blend(self.ctrl_col(i), v)).collect()
    }
    /// The column curve at `u` (control points down).
    fn col(&self, u: f64) -> Vec<P> {
        (0..self.nv).map(|j| blend(self.ctrl_row(j), u)).collect()
    }
    /// The net's sides from its first corner: along its first row (u) and down its first column (v).
    fn sides(&self) -> (P, P) {
        let (p00, p10, p01) = (self.at(0, 0), self.at(self.nu.saturating_sub(1), 0), self.at(0, self.nv.saturating_sub(1)));
        ([p10[0] - p00[0], p10[1] - p00[1]], [p01[0] - p00[0], p01[1] - p00[1]])
    }
    /// The twist `P00 − P10 − P01 + P11` of the control quad with its top-left point at `(i, j)`.
    fn quad_twist(&self, i: usize, j: usize) -> P {
        let (a, b, c, d) = (self.at(i, j), self.at(i + 1, j), self.at(i, j + 1), self.at(i + 1, j + 1));
        [a[0] - b[0] - c[0] + d[0], a[1] - b[1] - c[1] + d[1]]
    }
    /// The net of the part `[u0, u1] × [v0, v1]`.
    fn sub(&self, c: Cell) -> Net {
        let rows: Vec<Vec<P>> = (0..self.nv).map(|j| segment(self.ctrl_row(j), c[0], c[1])).collect();
        let cols: Vec<Vec<P>> = (0..self.nu).map(|i| segment(rows.iter().map(|r| r.get(i).copied().unwrap_or([f64::NAN; 2])).collect(), c[2], c[3])).collect();
        let mut p = Vec::with_capacity(self.p.len());
        for j in 0..self.nv {
            for col in &cols {
                p.push(col.get(j).copied().unwrap_or([f64::NAN; 2]));
            }
        }
        Net { nu: self.nu, nv: self.nv, p }
    }
    /// How unevenly the control quad with its top-left point at `(i, j)` is stretched: the larger
    /// difference in length between two opposite sides of it.
    fn side_twist(&self, i: usize, j: usize) -> f64 {
        let (a, b, c, d) = (self.at(i, j), self.at(i + 1, j), self.at(i, j + 1), self.at(i + 1, j + 1));
        let len = |p: P, q: P| (q[0] - p[0]).hypot(q[1] - p[1]);
        let (du, dv) = ((len(a, b) - len(c, d)).abs(), (len(a, c) - len(b, d)).abs());
        if du.is_finite() && dv.is_finite() { du.max(dv) } else { 0.0 }
    }
    /// The largest [`Net::side_twist`] of the net's first column of quads (the cell's left edge).
    fn twist_left(&self) -> f64 {
        (0..self.nv.saturating_sub(1)).map(|j| self.side_twist(0, j)).fold(0.0, f64::max)
    }
    /// The largest [`Net::side_twist`] of the net's first row of quads (the cell's top edge).
    fn twist_top(&self) -> f64 {
        (0..self.nu.saturating_sub(1)).map(|i| self.side_twist(i, 0)).fold(0.0, f64::max)
    }
    /// How much longer a leg of the net's first column of control points is than the leg beside
    /// it in the second column (the largest over the column; negative where the second column's
    /// legs are the longer ones).
    fn left_legs_longer(&self) -> f64 {
        let mut m = f64::NEG_INFINITY;
        for j in 0..self.nv.saturating_sub(1) {
            let (a, b, c, d) = (self.at(0, j), self.at(1, j), self.at(0, j + 1), self.at(1, j + 1));
            let t = (c[0] - a[0]).hypot(c[1] - a[1]) - (d[0] - b[0]).hypot(d[1] - b[1]);
            if t.is_finite() {
                m = m.max(t);
            }
        }
        m
    }
    /// The largest component along `d` of a quad's twist `P00 − P10 − P01 + P11` (0 for a zero `d`).
    fn twist_along(&self, d: P) -> f64 {
        let n = d[0].hypot(d[1]);
        if n.is_nan() || n <= 0.0 {
            return 0.0;
        }
        let mut m = 0.0f64;
        for j in 0..self.nv.saturating_sub(1) {
            for i in 0..self.nu.saturating_sub(1) {
                let q = self.quad_twist(i, j);
                let t = (q[0] * d[0] + q[1] * d[1]).abs() / n;
                if t.is_finite() {
                    m = m.max(t);
                }
            }
        }
        m
    }
    /// The largest quad twist with its component along the rows (`du`) counted at `1 / k`: the
    /// twist across the rows in full, the rows' own stretching less. Over the net's first `rows`
    /// rows of quads.
    fn twist_across_rows(&self, du: P, dv: P, k: f64, rows: usize) -> f64 {
        let (n, m) = (du[0].hypot(du[1]), dv[0].hypot(dv[1]));
        if n.is_nan() || m.is_nan() || n <= 0.0 || m <= 0.0 {
            return self.twist();
        }
        let (ux, uy) = (du[0] / n, du[1] / n);
        let mut m = 0.0f64;
        for j in 0..self.nv.saturating_sub(1).min(rows) {
            for i in 0..self.nu.saturating_sub(1) {
                let t = self.quad_twist(i, j);
                let (along, across) = (t[0] * ux + t[1] * uy, -t[0] * uy + t[1] * ux);
                let v = (across * across + (along / k) * (along / k)).sqrt();
                if v.is_finite() {
                    m = m.max(v);
                }
            }
        }
        m
    }
    /// The largest of the net's columns' [`need`]s, its inner control columns included.
    fn cols_need(&self) -> f64 {
        self.cols_cut_need(f64::INFINITY)
    }
    /// The largest of the net's columns' [`cut_need`]s with uneven drift tolerance `uneven`.
    fn cols_cut_need(&self, uneven: f64) -> f64 {
        (0..self.nu).map(|i| cut_need(&self.ctrl_col(i), uneven)).fold(0.0, f64::max)
    }
    /// A quilt cell's need across: the largest of its net's rows' [`cut_need`]s (uneven drift
    /// over [`QUILT_ROW_UNEVEN_TOL`]), its inner control rows included and counted
    /// [`INNER_ROW_WEIGHT`] times.
    fn quilt_rows_need(&self) -> f64 {
        self.rows_need_by(INNER_ROW_WEIGHT, QUILT_ROW_UNEVEN_TOL)
    }
    /// The largest of the net's rows' [`cut_need`]s with uneven drift tolerance `uneven`, its
    /// inner control rows counted `inner` times.
    fn rows_need_by(&self, inner: f64, uneven: f64) -> f64 {
        let last = self.nv.saturating_sub(1);
        (0..self.nv)
            .map(|j| {
                let n = cut_need(&self.ctrl_row(j), uneven);
                if j == 0 || j == last { n } else { inner * n }
            })
            .fold(0.0, f64::max)
    }
    /// The largest [`Net::quad_twist`] (euclidean) of the net's first row of quads.
    fn top_row_twist(&self) -> f64 {
        self.twist_of_rows(1)
    }
    /// The largest [`Net::quad_twist`] (euclidean) of the net.
    fn twist(&self) -> f64 {
        self.twist_of_rows(usize::MAX)
    }
    /// The largest [`Net::quad_twist`] (euclidean) of the net's first `rows` rows of quads.
    fn twist_of_rows(&self, rows: usize) -> f64 {
        let mut m = 0.0f64;
        for j in 0..self.nv.saturating_sub(1).min(rows) {
            for i in 0..self.nu.saturating_sub(1) {
                let t = self.quad_twist(i, j);
                let t = t[0].hypot(t[1]);
                if t.is_finite() {
                    m = m.max(t);
                }
            }
        }
        m
    }
}

/// The Bezier curve with control points `p` at `t` (de Casteljau, in place).
fn blend(mut p: Vec<P>, t: f64) -> P {
    for n in (1..p.len()).rev() {
        for k in 0..n {
            if let (Some(&a), Some(&b)) = (p.get(k), p.get(k + 1)) {
                p[k] = lerp(a, b, t);
            }
        }
    }
    p.first().copied().unwrap_or([f64::NAN; 2])
}

/// Replaces the control points `p` of a curve with those of its part before `t` (de Casteljau in
/// place: each point becomes the first of its level).
fn keep_before(p: &mut [P], t: f64) {
    for r in 1..p.len() {
        for k in (r..p.len()).rev() {
            if let (Some(&a), Some(&b)) = (p.get(k - 1), p.get(k)) {
                p[k] = lerp(a, b, t);
            }
        }
    }
}

/// Replaces the control points `p` of a curve with those of its part after `t` (each point
/// becomes the last of its level).
fn keep_after(p: &mut [P], t: f64) {
    for r in 1..p.len() {
        for k in 0..p.len() - r {
            if let (Some(&a), Some(&b)) = (p.get(k), p.get(k + 1)) {
                p[k] = lerp(a, b, t);
            }
        }
    }
}

/// The control points of the part `[t0, t1]` of the curve `c` (worked out in place).
fn segment(mut c: Vec<P>, t0: f64, t1: f64) -> Vec<P> {
    if t1 < 1.0 {
        keep_before(&mut c, t1);
    }
    if t0 > 0.0 && t1 > 0.0 {
        keep_after(&mut c, t0 / t1);
    }
    c
}

/// How much a curve segment asks to be split: the larger of its inner control points' largest
/// sideways deviation from the chord over [`PERP_TOL`] and their mean deviation along it over
/// [`TANG_TOL`] (each against its even spacing on the chord); `>= 1` splits. A straight segment
/// (two points) never splits.
///
/// An inner control point that lies before the chord's start or past its end (the curve turns back
/// on itself there, as a row does in a crease) counts by its distance from that end instead, over
/// [`BEFORE_TOL`] or [`AFTER_TOL`] ([`SHALLOW_BEFORE_TOL`] or [`SHALLOW_AFTER_TOL`] where the
/// segment turns back only a little), and is left out of the mean along the chord.
fn need(c: &[P]) -> f64 {
    need_weighing_first(c, 0.0)
}

/// How unevenly a curve segment's inner control points drift along its chord: the first one's
/// drift from its even spacing minus the last one's (document px, unsigned). Drifts the same way
/// cancel, so this measures a segment that speeds up and slows down along a nearly straight
/// chord. Zero for a segment with fewer than two inner points.
fn uneven_drift(c: &[P]) -> f64 {
    let n = c.len().saturating_sub(1);
    let (Some(&a), Some(&b)) = (c.first(), c.last()) else { return 0.0 };
    let ch = [b[0] - a[0], b[1] - a[1]];
    let len = ch[0].hypot(ch[1]);
    if n < 3 || len <= 0.0 {
        return 0.0;
    }
    let drift = |k: usize| c.get(k).map_or(0.0, |p| ((p[0] - a[0]) * ch[0] + (p[1] - a[1]) * ch[1]) / len - len * k as f64 / n as f64);
    let d = (drift(1) - drift(n - 1)).abs();
    if d.is_finite() { d } else { 0.0 }
}

/// [`need`], or the segment's [`uneven_drift`] over `uneven` (px) where that is larger; an
/// infinite `uneven` leaves [`need`] alone.
fn cut_need(c: &[P], uneven: f64) -> f64 {
    let n = need(c);
    if uneven.is_finite() && uneven > 0.0 { n.max(uneven_drift(c) / uneven) } else { n }
}

/// [`need`], with the first inner control point's own drift along the chord, counted
/// `first_weight` times, standing for the mean drift where it is larger (the rows of a one-patch
/// full mesh, see [`FIRST_POINT_ROW_WEIGHT`]).
fn need_weighing_first(c: &[P], first_weight: f64) -> f64 {
    let n = c.len().saturating_sub(1);
    let (Some(&a), Some(&b)) = (c.first(), c.last()) else { return 0.0 };
    if n < 2 {
        return 0.0;
    }
    let inner = c.get(1..n).unwrap_or_default();
    let ch = [b[0] - a[0], b[1] - a[1]];
    let len = ch[0].hypot(ch[1]);
    let v = if len > 0.0 {
        let (tx, ty) = (ch[0] / len, ch[1] / len);
        // Where each inner control point lies along the chord (0 at its start, 1 at its end) and
        // how far off it.
        let at: Vec<(f64, f64)> = inner
            .iter()
            .map(|p| {
                let r = [p[0] - a[0], p[1] - a[1]];
                ((r[0] * tx + r[1] * ty) / len, (r[0] * ty - r[1] * tx).abs())
            })
            .collect();
        // How far they reach before the chord's start and past its end.
        let first = at.iter().map(|q| q.0).fold(f64::INFINITY, f64::min);
        let last = at.iter().map(|q| q.0).fold(f64::NEG_INFINITY, f64::max);
        let fold_tip = len < FOLD_TIP_CHORD && first >= 0.0 && at.iter().filter(|q| q.0 > 1.0).count() == 1;
        let before_tol = if first > -SHALLOW_BEFORE_REACH { SHALLOW_BEFORE_TOL } else { BEFORE_TOL };
        let after_tol = if last >= 1.0 && last - first < SHARP_FOLD_SPAN {
            (SHARP_FOLD_TOL * last).min(SHARP_FOLD_MAX)
        } else if last < SHALLOW_AFTER_REACH {
            SHALLOW_AFTER_TOL
        } else {
            AFTER_TOL
        };
        let (mut far, mut sum, mut inside, mut first_drift) = (0.0f64, 0.0f64, 0usize, 0.0f64);
        for (k, &(along, side)) in (1..).zip(&at) {
            let even = f64::from(k) / n as f64;
            far = far.max(side / PERP_TOL);
            if along < 0.0 {
                far = far.max(side.hypot(-along * len) / before_tol);
            } else if along > 1.0 {
                far = far.max(side.hypot((along - 1.0) * len) / after_tol);
                if fold_tip {
                    sum += (1.0 - even) * len;
                    inside += 1;
                }
            } else if along >= 0.0 {
                let drift = (along - even) * len;
                if k == 1 {
                    first_drift = drift.abs();
                }
                sum += drift;
                inside += 1;
            }
        }
        let tang = if inside > 0 { (sum / inside as f64).abs() } else { 0.0 };
        let tang = tang.max(first_weight * first_drift);
        far.max(tang / TANG_TOL)
    } else {
        // A closed chord: how far the inner points lie from its ends.
        inner
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let s = (k + 1) as f64 / n as f64;
                (p[0] - (a[0] + ch[0] * s)).hypot(p[1] - (a[1] + ch[1] * s))
            })
            .fold(0.0, f64::max)
            / PERP_TOL
    };
    if v.is_finite() { v } else { 0.0 }
}

/// The 4 × 4 cubic control net of the map `at` over the unit square, from its values at thirds:
/// exact for a cubic patch (a warp, moved, scaled, rotated or skewed).
fn cubic_net(at: &dyn Fn(f64, f64) -> P) -> Net {
    // A cubic through values at 0, 1/3, 2/3, 1 has inner control points
    // (−5 S0 + 18 S1 − 9 S2 + 2 S3)/6 and (2 S0 − 9 S1 + 18 S2 − 5 S3)/6.
    let fit = |s: [P; 4]| -> [P; 4] {
        let c = |k: usize, w: [f64; 4]| (0..4).map(|i| s[i][k] * w[i]).sum::<f64>() / 6.0;
        let (w1, w2) = ([-5.0, 18.0, -9.0, 2.0], [2.0, -9.0, 18.0, -5.0]);
        [s[0], [c(0, w1), c(1, w1)], [c(0, w2), c(1, w2)], s[3]]
    };
    let t = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
    let rows: Vec<[P; 4]> = t.iter().map(|&v| fit(t.map(|u| at(u, v)))).collect();
    let mut p = vec![[0.0; 2]; 16];
    for i in 0..4 {
        let col = fit([0, 1, 2, 3].map(|j| rows.get(j).map_or([f64::NAN; 2], |r| r[i])));
        for (j, q) in col.iter().enumerate() {
            if let Some(slot) = p.get_mut(j * 4 + i) {
                *slot = *q;
            }
        }
    }
    Net { nu: 4, nv: 4, p }
}

/// The net at the degree it was stored at: a direction whose curves are all exactly quadratic or
/// straight (an elevated envelope) is reduced to three or two points.
fn stored_net(cubic: &Net) -> Net {
    let scale = cubic.p.iter().fold(1.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
    let tol = 1e-7 * scale;
    let small = |p: P| p[0].abs().max(p[1].abs()) <= tol;
    let lines = |net: &Net, across: bool| -> Vec<Vec<P>> {
        if across { (0..net.nv).map(|j| net.ctrl_row(j)).collect() } else { (0..net.nu).map(|i| net.ctrl_col(i)).collect() }
    };
    let degree = |ls: &[Vec<P>]| -> usize {
        let d2 = |c: &[P], k: usize| match (c.get(k), c.get(k + 1), c.get(k + 2)) {
            (Some(a), Some(b), Some(c)) => [a[0] - 2.0 * b[0] + c[0], a[1] - 2.0 * b[1] + c[1]],
            _ => [0.0; 2],
        };
        let d3 = |c: &[P]| match (c.first(), c.get(1), c.get(2), c.get(3)) {
            (Some(a), Some(b), Some(c), Some(d)) => [a[0] - 3.0 * b[0] + 3.0 * c[0] - d[0], a[1] - 3.0 * b[1] + 3.0 * c[1] - d[1]],
            _ => [0.0; 2],
        };
        if ls.iter().all(|c| small(d2(c, 0)) && small(d2(c, 1))) {
            1
        } else if ls.iter().all(|c| small(d3(c))) {
            2
        } else {
            3
        }
    };
    let reduce = |c: &[P], deg: usize| -> Vec<P> {
        let (a, b, d) = (c.first().copied().unwrap_or([f64::NAN; 2]), c.get(1).copied().unwrap_or([f64::NAN; 2]), c.last().copied().unwrap_or([f64::NAN; 2]));
        match deg {
            1 => vec![a, d],
            2 => vec![a, [(3.0 * b[0] - a[0]) / 2.0, (3.0 * b[1] - a[1]) / 2.0], d],
            _ => c.to_vec(),
        }
    };
    let du = degree(&lines(cubic, true));
    let rows: Vec<Vec<P>> = lines(cubic, true).iter().map(|r| reduce(r, du)).collect();
    let nu = du + 1;
    let across = Net { nu, nv: 4, p: rows.concat() };
    let dv = degree(&lines(&across, false));
    let cols: Vec<Vec<P>> = lines(&across, false).iter().map(|c| reduce(c, dv)).collect();
    let nv = dv + 1;
    let mut p = vec![[0.0; 2]; nu * nv];
    for (i, col) in cols.iter().enumerate() {
        for (j, q) in col.iter().enumerate() {
            if let Some(slot) = p.get_mut(j * nu + i) {
                *slot = *q;
            }
        }
    }
    Net { nu, nv, p }
}

/// A cell `[u0, u1, v0, v1]` (normalised).
type Cell = [f64; 4];

/// A recorded split: the cell and whether it split across and down (the directions are what the
/// tests read; placing the points needs only the cell).
struct Split {
    cell: Cell,
    #[cfg_attr(not(test), allow(dead_code))]
    across: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    down: bool,
}

/// The tessellation: vertices (normalised source position, document position) and quads of vertex
/// indices `[top-left, top-right, bottom-left, bottom-right]` in the document's orientation, each
/// drawn as two triangles split along top-left to bottom-right.
pub(crate) struct EnvelopeMesh {
    pub uv: Vec<P>,
    pub pos: Vec<P>,
    pub quads: Vec<[usize; 4]>,
}

fn key(u: f64, v: f64) -> (u64, u64) {
    (u.to_bits(), v.to_bits())
}

/// How many halvings `t` (a dyadic fraction) is from the whole: 0 for 0 and 1, 1 for ½, …
fn level(t: f64) -> u32 {
    (0..12)
        .find(|&k| {
            let s = t * f64::from(1u32 << k);
            (s - s.round()).abs() < 1e-9
        })
        .unwrap_or(12)
}

/// Tessellates the one-patch warp whose map from normalised source coordinates to the document is
/// `at`: an envelope, or (`full`) a full 4 × 4 mesh. `None` when the map isn't finite at the
/// corners.
pub(crate) fn envelope_mesh(at: &dyn Fn(f64, f64) -> P, full: bool) -> Option<EnvelopeMesh> {
    let corners = [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)];
    if corners.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return None;
    }
    let net = net_of(at, full);
    let rules = one_patch_rules(&net, full);
    let (leaves, log) = cells(&net, twisty_net(&net), &rules);
    Some(triangulate(&net, at, &leaves, &log, &rules))
}

/// Whether the net is cubic in a direction: its cells are tested for twist.
fn twisty_net(net: &Net) -> bool {
    net.nu.max(net.nv) == 4
}

/// The rules a one-patch warp with the control net `net` is cut by.
fn one_patch_rules(net: &Net, full: bool) -> Rules {
    // A preset bent sideways: stored with more points down than across.
    let tall = net.nv > net.nu;
    if full {
        Rules {
            start: START_FULL,
            force_down: true,
            twist_spreads: false,
            twist_spreads_in_row: true,
            fixed_level: 1,
            mid_row_level: 1,
            cut_sides: false,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
            surface_sides: [false; 2],
            uneven_quilt: false,
            side_seam: false,
        }
    } else {
        Rules {
            start: START,
            force_down: false,
            twist_spreads: false,
            twist_spreads_in_row: false,
            fixed_level: 2,
            mid_row_level: 1,
            cut_sides: !tall,
            twist_by_side: true,
            square_twist: tall.then_some(TALL_SQUARE_TWIST_TOL),
            column_chords: tall.then_some(TALL_COLUMN_LEVEL),
            flat_columns: (!tall).then_some(FLAT_COLUMN_TOL),
            surface_sides: [false; 2],
            uneven_quilt: false,
            side_seam: false,
        }
    }
}

/// Which sides of a quilt's patch it shares with another patch.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Seams {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
    /// The quilt's patches don't halve evenly: there aren't a power of two of equal patches each way.
    pub uneven: bool,
}

/// Tessellates one patch of a warp split into several (a quilt), whose map from the patch's own
/// normalised coordinates to the document is `at`, as the reference application cuts each patch:
/// by the full mesh's rules, but starting from the whole patch as one cell (a flat or small patch
/// stays whole), without the first level's forced split down, and with a split down for a cell's
/// twist carried to the other cells cut from the same parent. `None` when the map isn't finite at
/// the corners.
pub(crate) fn quilt_patch_mesh(at: &dyn Fn(f64, f64) -> P, seams: Seams) -> Option<EnvelopeMesh> {
    let corners = [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)];
    if corners.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return None;
    }
    let net = cubic_net(at);
    let rules = patch_rules(seams);
    let (leaves, log) = cells(&net, true, &rules);
    Some(triangulate(&net, at, &leaves, &log, &rules))
}

/// The rules a quilt's patch with seams `seams` is cut by (see [`quilt_patch_mesh`]).
fn patch_rules(seams: Seams) -> Rules {
    Rules {
        start: 1,
        force_down: false,
        twist_spreads: true,
        twist_spreads_in_row: false,
        fixed_level: 0,
        mid_row_level: 0,
        cut_sides: false,
        twist_by_side: false,
        square_twist: None,
        column_chords: None,
        flat_columns: None,
        surface_sides: if seams.top || seams.bottom { [false; 2] } else { [seams.left, seams.right] },
        uneven_quilt: seams.uneven,
        side_seam: seams.left || seams.right,
    }
}

/// What differs between cutting a one-patch warp and a patch of a quilt.
#[derive(Clone, Copy)]
struct Rules {
    /// Cells per side to start from.
    start: usize,
    /// A first-level cell split across twice splits down too: the half of it that splits across
    /// again (a one-patch full mesh).
    force_down: bool,
    /// A cell split down for its twist splits the other cells cut from its parent down too.
    twist_spreads: bool,
    /// A cell split down for its twist splits the cells beside it in its row, cut from the same
    /// parent, down too (a one-patch full mesh).
    twist_spreads_in_row: bool,
    /// Grid lines up to this many halvings in stay where they are on the surface (a T-junction
    /// further in sits on the chord of the edge it lies on): the starting grid's.
    fixed_level: u32,
    /// The same on the surface's middle row (v = ½), for its points along the row: an envelope's
    /// quarter points there follow the chord rule though they are the starting grid's. (On the
    /// top and bottom edges every point but the corners follows it.)
    mid_row_level: u32,
    /// A leaf is cut across at the points of smaller neighbours on its left and right sides
    /// (envelopes that aren't tall); otherwise it stays whole and those points sit on its side's
    /// chord.
    cut_sides: bool,
    /// A twist split is measured on one side of the cell's control net only, as how unevenly its
    /// quads are stretched (the reference's envelopes): a split across on its left column of
    /// quads (Arc Lower's mirror-image corner cells split on the left, not on the right), a split
    /// down on its top row. Otherwise as the largest `P00 − P10 − P01 + P11` of the whole net.
    twist_by_side: bool,
    /// A square cell whose whole net twists past this splits across as well (tall envelopes).
    square_twist: Option<f64>,
    /// The chord rule for rows holds for every column too, for points more than this many
    /// halvings in (tall envelopes); otherwise only on the surface's left and right sides.
    column_chords: Option<u32>,
    /// Otherwise, a point further in than the fixed lines on an inner column whose stretch of the
    /// column asks for less than this much of a split sits on that stretch's chord (envelopes
    /// that aren't tall).
    flat_columns: Option<f64>,
    /// The left and right sides whose points stay where the surface puts them rather than on the
    /// side's chord: a quilt patch's inner seams when the quilt has a single row of patches.
    surface_sides: [bool; 2],
    /// The patch belongs to a quilt that doesn't halve evenly: a whole patch splits down for the
    /// twist across the rows of its first row of quads only, past [`FIRST_ROW_TWIST_TOL`], and a
    /// cell as tall as its patch for the twist along its rows past
    /// [`UNEVEN_FULL_HEIGHT_TWIST_TOL`].
    uneven_quilt: bool,
    /// The patch belongs to a quilt and has a seam on its left or right: its side points drop onto
    /// the side's chord however unevenly the side drifts (see [`UNEVEN_TOL`]).
    side_seam: bool,
}

impl Rules {
    /// A one-patch full mesh (the one cut forced down): measured by its cells' whole control nets,
    /// with the first inner point of their rows weighed, twisted cells split both ways, and its
    /// first-level splits down spread by column need.
    fn full_mesh(&self) -> bool {
        self.force_down && !self.twist_spreads
    }
}

/// The parent cell of `c` (one halving up both ways), as a comparable key.
fn parent_of(c: &Cell) -> (i64, i64, i64, i64) {
    let (w, h) = (2.0 * (c[1] - c[0]), 2.0 * (c[3] - c[2]));
    let q = |t: f64| (t * 1e6).round() as i64;
    ((c[0] / w + EPS).floor() as i64, (c[2] / h + EPS).floor() as i64, q(w), q(h))
}

/// A quilt patch cut both ways splits its four first-level cells down by row strip, as the
/// reference's split warps show (left and right cells of a row always agree there, whatever their
/// own twist): the strip across the whole patch at that height splits when its top row of
/// control quads twists past [`STRIP_TOP_TWIST_TOL`], when its columns at the patch's sides and
/// middle or any column of its control net ask for a split, or when the other strip's largest
/// twist, per [`STRIP_WIDTH`] px of the patch's width, passes [`STRIP_OTHER_TWIST_TOL`]. Their
/// own twist then spreads no further.
fn strip_down_cuts(net: &Net, level_cells: &[Cell], decisions: &mut [(bool, bool)], twisted: &mut [bool]) {
    let halves = level_cells.len() == 4 && level_cells.iter().all(|c| (c[1] - c[0] - 0.5).abs() < EPS && (c[3] - c[2] - 0.5).abs() < EPS);
    if !halves {
        return;
    }
    let strip = |v0: f64| {
        let s = net.sub([0.0, 1.0, v0, v0 + 0.5]);
        let sides = [0.0, 0.5, 1.0].iter().map(|&u| cut_need(&segment(net.col(u), v0, v0 + 0.5), QUILT_COL_UNEVEN_TOL)).fold(0.0, f64::max);
        let fv = sides.max(s.cols_cut_need(QUILT_COL_UNEVEN_TOL));
        (s.top_row_twist(), s.twist(), fv)
    };
    let (top, bottom) = (strip(0.0), strip(0.5));
    let (p, q) = (net.at(0, 0), net.at(net.nu.saturating_sub(1), 0));
    let per_width = STRIP_WIDTH / (q[0] - p[0]).hypot(q[1] - p[1]).max(STRIP_WIDTH);
    let cut = |own: (f64, f64, f64), other: (f64, f64, f64)| {
        own.0 > STRIP_TOP_TWIST_TOL + EPS || own.2 >= 1.0 - EPS || other.1 * per_width > STRIP_OTHER_TWIST_TOL + EPS
    };
    let (cut_top, cut_bottom) = (cut(top, bottom), cut(bottom, top));
    for ((c, d), t) in level_cells.iter().zip(decisions.iter_mut()).zip(twisted.iter_mut()) {
        d.1 = if c[2] < 0.25 { cut_top } else { cut_bottom };
        *t = false;
    }
}

/// Splits down every cell whose parent has a cell marked in `from`.
fn spread_down(level_cells: &[Cell], decisions: &mut [(bool, bool)], from: &[bool]) {
    let parents: HashSet<_> = level_cells.iter().zip(from).filter(|(_, t)| **t).map(|(c, _)| parent_of(c)).collect();
    if parents.is_empty() {
        return;
    }
    for (c, d) in level_cells.iter().zip(decisions) {
        if parents.contains(&parent_of(c)) {
            d.1 = true;
        }
    }
}

/// Splits down every cell beside a cell marked in `from`: cut from the same parent, in the same row.
fn spread_down_in_row(level_cells: &[Cell], decisions: &mut [(bool, bool)], from: &[bool]) {
    // Cell bounds are exact halvings of the whole, so a row's top is one value.
    let rows: HashSet<_> = level_cells.iter().zip(from).filter(|(_, t)| **t).map(|(c, _)| (parent_of(c), c[2].to_bits())).collect();
    if rows.is_empty() {
        return;
    }
    for (c, d) in level_cells.iter().zip(decisions) {
        if rows.contains(&(parent_of(c), c[2].to_bits())) {
            d.1 = true;
        }
    }
}

/// How much a cell's rows and columns ask to be split (across, down), `sub` giving the cell's own
/// control net when a rule needs it.
fn cell_needs<'a>(net: &Net, c: Cell, rules: &Rules, sub: impl Fn() -> &'a Net) -> (f64, f64) {
    let [u0, u1, v0, v1] = c;
    let full_one = rules.full_mesh();
    // A quilt's patch measures its cells' rows by all the rows of their control nets.
    let fu = if rules.twist_spreads {
        sub().quilt_rows_need()
    } else {
        let w = match (full_one, u0 < EPS && u1 - u0 > 0.5 - EPS) {
            (true, true) => LEFT_EDGE_FIRST_POINT_WEIGHT,
            (true, false) => FIRST_POINT_ROW_WEIGHT,
            (false, _) => 0.0,
        };
        need_weighing_first(&segment(net.row(v0), u0, u1), w).max(need_weighing_first(&segment(net.row(v1), u0, u1), w))
    };
    // A whole quilt patch measures its columns by every column of its control net.
    // A quilt's patch also counts its columns' uneven drift.
    let tol = if rules.twist_spreads { QUILT_COL_UNEVEN_TOL } else { f64::INFINITY };
    let fv = if rules.twist_spreads && u1 - u0 > 1.0 - EPS && v1 - v0 > 1.0 - EPS {
        net.cols_cut_need(tol)
    } else {
        cut_need(&segment(net.col(u0), v0, v1), tol).max(cut_need(&segment(net.col(u1), v0, v1), tol))
    };
    // A one-patch full mesh also measures its cells by every row and column of their control nets.
    if full_one { (fu.max(sub().rows_need_by(1.0, f64::INFINITY)), fv.max(sub().cols_need())) } else { (fu, fv) }
}

/// A one-patch full mesh's cells twisted past [`BOTH_WAYS_TWIST_TOL`] (`measured`, column need
/// and twist per cell) split both ways.
fn split_both_ways_past_twist(decisions: &mut [(bool, bool)], measured: &[(f64, f64)]) {
    for (d, &(_, tw)) in decisions.iter_mut().zip(measured) {
        if tw > BOTH_WAYS_TWIST_TOL + EPS {
            *d = (true, true);
        }
    }
}

/// A one-patch full mesh's first-level split down: it carries to the cell beside it in its row, and
/// to every first cell where the cell's columns (`measured`, column need and twist per cell) ask
/// for [`FIRST_LEVEL_SPREAD_FV`].
fn first_level_spread_full(level_cells: &[Cell], decisions: &mut [(bool, bool)], measured: &[(f64, f64)]) {
    let down: Vec<(Cell, f64)> = level_cells.iter().zip(decisions.iter()).zip(measured).filter(|((_, d), _)| d.1).map(|((c, _), m)| (*c, m.0)).collect();
    for (c, d) in level_cells.iter().zip(decisions) {
        if down.iter().any(|(m, fv)| (m[2] - c[2]).abs() < EPS || *fv > FIRST_LEVEL_SPREAD_FV + EPS) {
            d.1 = true;
        }
    }
}

/// The control net the reference application cuts by: an envelope's at the degree it was stored at
/// (recovered from the map), a full mesh's cubic both ways, even where it is flatter.
fn net_of(at: &dyn Fn(f64, f64) -> P, full: bool) -> Net {
    let cubic = cubic_net(at);
    if full { cubic } else { stored_net(&cubic) }
}

/// A cell's twist as `rules` measure it, with the tolerance it must pass to split:
/// `(twist, tolerance)`.
fn twist_test(sub: &Net, rules: &Rules, whole: bool, full_height: bool, goes_across: bool) -> (f64, f64) {
    let (du, dv) = sub.sides();
    let first_row = rules.uneven_quilt && rules.twist_spreads && whole && !goes_across;
    let tw = match (rules.twist_by_side, goes_across) {
        (true, true) => sub.twist_left(),
        (true, false) => sub.twist_top(),
        // A quilt's patch: a whole patch splits down for the twist across its rows (the rows' own
        // stretching counts half), in an uneven quilt across its first row of quads only; a
        // smaller cell for the larger of its twist's components along its v side (at 0.9) and
        // along its u side, or for the latter alone where it is as tall as its patch (the patch
        // kept whole down).
        (false, _) if first_row => sub.twist_across_rows(du, dv, 2.0, 1),
        (false, false) if rules.twist_spreads && full_height => sub.twist_along(du),
        (false, _) if rules.twist_spreads && whole && !goes_across => sub.twist_across_rows(du, dv, 2.0, usize::MAX),
        (false, _) if rules.twist_spreads && !whole => (0.9 * sub.twist_along(dv)).max(sub.twist_along(du)),
        // A one-patch full mesh's cell splits down for the twist of its top edge (its net's first
        // row of quads), split across or not. The reference application is not mirror-symmetric
        // here: a scaled wave's top side cells are cut down, their bottom mirrors (as twisted in
        // all, less along their top edges) not; with an inner control point moved 18 px across,
        // the quarter cell whose top edge twists 1.125 px is cut into eighths down, its mirror
        // twisted as much along its bottom edge stays whole, and at 14 px (0.875) neither is.
        (false, false) if rules.full_mesh() => sub.twist_top(),
        (false, _) => sub.twist(),
    };
    // A whole quilt patch splits for its twist only past WHOLE_PATCH_TWIST_TOL (FIRST_ROW_TWIST_TOL
    // for its first row), a full-height cell of an uneven quilt past UNEVEN_FULL_HEIGHT_TWIST_TOL.
    if first_row {
        (tw, FIRST_ROW_TWIST_TOL)
    } else if rules.twist_spreads && rules.uneven_quilt && full_height && !goes_across {
        (tw, UNEVEN_FULL_HEIGHT_TWIST_TOL)
    } else if rules.twist_spreads && whole {
        (tw, WHOLE_PATCH_TWIST_TOL)
    } else {
        (tw, TWIST_TOL)
    }
}

/// The leaf cells and the record of splits, in the stored frame.
fn cells(net: &Net, twisty: bool, rules: &Rules) -> (Vec<Cell>, Vec<Split>) {
    let quadratic = rules.twist_by_side && net.nu == 3 && net.nv == 3;
    let start = rules.start.max(1);
    let first = 1.0 / start as f64;
    let mut level_cells: Vec<Cell> = (0..start * start)
        .map(|k| {
            let (i, j) = ((k / start) as f64, (k % start) as f64);
            [i * first, (i + 1.0) * first, j * first, (j + 1.0) * first]
        })
        .collect();
    let (mut leaves, mut log) = (Vec::new(), Vec::new());
    while !level_cells.is_empty() {
        let mut decisions = Vec::with_capacity(level_cells.len());
        // Cells split down for their twist alone.
        let mut twisted = Vec::with_capacity(level_cells.len());
        // Rows of sibling cells (by their top and bottom, their width and their parent's column)
        // with a square cell split down for its curvature. Cell bounds are exact halvings of the
        // whole, so each is one value.
        let mut curved_rows: HashSet<(u64, u64, i64, u64)> = HashSet::new();
        let column = |c: &Cell| (c[0] / (2.0 * (c[1] - c[0])) + EPS).floor() as i64;
        // A one-patch full mesh: each cell's column need and twist, for the rules after the cells' own decisions.
        let full_one = rules.full_mesh();
        let mut measured: Vec<(f64, f64)> = Vec::with_capacity(level_cells.len());
        for &c in &level_cells {
            let [u0, u1, v0, v1] = c;
            // The cell's own control net, built once when a rule needs it.
            let sub_net = std::cell::OnceCell::new();
            let sub = || sub_net.get_or_init(|| net.sub(c));
            let (fu, fv) = cell_needs(net, c, rules, sub);
            let (mut across, mut down) = (fu >= 1.0 - EPS, fv >= 1.0 - EPS);
            let mut twist_down = false;
            let (lu, lv) = (u1 - u0, v1 - v0);
            let square = (lu - lv).abs() < EPS;
            if twisty && down && square {
                curved_rows.insert((v0.to_bits(), v1.to_bits(), column(&c), lu.to_bits()));
            }
            let goes_across = lu > lv + EPS;
            if twisty {
                let whole = lu > 1.0 - EPS && lv > 1.0 - EPS;
                let full_height = !whole && lv > 1.0 - EPS;
                let (tw, tol) = twist_test(sub(), rules, whole, full_height, goes_across);
                if tw > tol + EPS {
                    if goes_across {
                        across = true;
                    } else {
                        twist_down = !down;
                        down = true;
                    }
                }
            } else if quadratic && goes_across {
                // A 3 × 3 envelope's twist splits a wide cell across only: where the legs of the
                // left column of its control points are longer than the next column's, in the
                // cell or in the other half of the cell it was cut from (Squeeze and Inflate below
                // 0 split their left cells; above 0 the legs beside are the longer ones).
                let h = 2.0 * lv;
                let top = (v0 / h + EPS).floor() * h;
                let tw = net.sub([u0, u1, top, top + lv]).left_legs_longer().max(net.sub([u0, u1, top + lv, top + h]).left_legs_longer());
                if tw > TWIST_TOL + EPS {
                    across = true;
                }
            }
            // A whole quilt patch whose twist runs along its rows (the rows stretch unevenly) past
            // ROW_TWIST_ACROSS_TOL splits across, as twist across its rows splits it down.
            if rules.twist_spreads && twisty && lu > 1.0 - EPS && lv > 1.0 - EPS {
                let sub = sub();
                if sub.twist_along(sub.sides().0) > ROW_TWIST_ACROSS_TOL + EPS {
                    across = true;
                }
            }
            if let Some(tol) = rules.square_twist
                && twisty
                && square
                && sub().twist() > tol + EPS
            {
                across = true;
            }
            // Forced down at the second split across: a cell half as wide as a first cell and as tall as one.
            if rules.force_down && across && (u1 - u0 - first / 2.0).abs() < EPS && (v1 - v0 - first).abs() < EPS {
                down = true;
            }
            if across || down {
                log.push(Split { cell: c, across, down });
            }
            decisions.push((across, down));
            twisted.push(twist_down);
            if full_one {
                measured.push((fv, if twisty { sub().twist() } else { 0.0 }));
            }
        }
        if rules.twist_spreads {
            strip_down_cuts(net, &level_cells, &mut decisions, &mut twisted);
            spread_down(&level_cells, &mut decisions, &twisted);
        }
        if rules.twist_spreads_in_row {
            spread_down_in_row(&level_cells, &mut decisions, &twisted);
        }
        if full_one {
            split_both_ways_past_twist(&mut decisions, &measured);
        }
        let first_level = level_cells.first().is_some_and(|c| (c[3] - c[2] - first).abs() < EPS);
        if first_level && full_one {
            first_level_spread_full(&level_cells, &mut decisions, &measured);
        } else if first_level {
            let down: Vec<bool> = decisions.iter().map(|d| d.1).collect();
            spread_down(&level_cells, &mut decisions, &down);
        }
        if !curved_rows.is_empty() {
            for (c, d) in level_cells.iter().zip(&mut decisions) {
                if curved_rows.contains(&(c[2].to_bits(), c[3].to_bits(), column(c), (c[1] - c[0]).to_bits())) {
                    d.1 = true;
                }
            }
        }
        let mut next = Vec::new();
        let room = leaves.len() + level_cells.len() * 4 < MAX_CELLS;
        for (&c, &(across, down)) in level_cells.iter().zip(&decisions) {
            let [u0, u1, v0, v1] = c;
            let across = room && across && u1 - u0 > MIN_CELL + EPS;
            let down = room && down && v1 - v0 > MIN_CELL + EPS;
            let (um, vm) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            match (across, down) {
                (true, true) => next.extend([[u0, um, v0, vm], [um, u1, v0, vm], [u0, um, vm, v1], [um, u1, vm, v1]]),
                (true, false) => next.extend([[u0, um, v0, v1], [um, u1, v0, v1]]),
                (false, true) => next.extend([[u0, u1, v0, vm], [u0, u1, vm, v1]]),
                (false, false) => leaves.push(c),
            }
        }
        level_cells = next;
    }
    (leaves, log)
}

/// The corners along each row (keyed by v's bits) or each column (by u's bits), sorted.
type Lines = HashMap<u64, Vec<f64>>;

/// The values of the sorted `line` strictly between `lo` and `hi`.
fn inside(line: Option<&Vec<f64>>, lo: f64, hi: f64) -> &[f64] {
    let Some(l) = line else { return &[] };
    let (from, to) = (l.partition_point(|&t| t <= lo + 1e-12), l.partition_point(|&t| t < hi - 1e-12));
    l.get(from..to.max(from)).unwrap_or_default()
}

fn lerp(a: P, b: P, s: f64) -> P {
    [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s]
}

/// The point of the segment `a`-`b` nearest `p`: a point dropped onto a bigger cell's edge.
pub(crate) fn onto(a: P, b: P, p: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let s = if l2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    lerp(a, b, s)
}

/// A point `p` of the surface dropped onto the chord `a`-`b` of its row or column. A point that
/// is on the chord already stays exactly where the surface puts it: on a straight row the drop
/// would only move it by the arithmetic's rounding, and a vertex a rounding step off its place
/// shifts the pixels along an edge that runs through pixel centres (a sheared mesh).
fn onto_chord(a: P, b: P, p: P) -> P {
    let m = onto(a, b, p);
    if (m[0] - p[0]).hypot(m[1] - p[1]) < ON_CHORD { p } else { m }
}

/// Where each corner's own cells put it, before any is dropped onto a bigger cell's edge: on the
/// surface, or on its row's chord (`lines`: the corners along each row by v, each column by u).
fn own_points(
    net: &Net,
    surf: &dyn Fn(f64, f64) -> P,
    keys: &[P],
    leaves: &[Cell],
    log: &[Split],
    rules: &Rules,
    lines: (&Lines, &Lines),
) -> HashMap<(u64, u64), P> {
    let (on_v, on_u) = lines;
    let fixed_level = rules.fixed_level;
    let place = |pos: &HashMap<(u64, u64), P>, u: f64, v: f64| pos.get(&key(u, v)).copied().unwrap_or_else(|| surf(u, v));
    let mut own: HashMap<(u64, u64), P> = HashMap::new();
    for p in keys {
        own.insert(key(p[0], p[1]), surf(p[0], p[1]));
    }
    // A new vertex on a row the row didn't need is dropped onto that row segment's chord, unless
    // the segment's inner points drift unevenly along it.
    let mut order = keys.to_vec();
    order.sort_by_cached_key(|p| level(p[0]));
    for &[u, v] in &order {
        let lv = level(u);
        // The surface's top and bottom edges keep only their corners, its middle row its own
        // level; the other rows the starting grid's points.
        let fixed = match level(v) {
            0 => 0,
            1 => rules.mid_row_level,
            _ => fixed_level,
        };
        if lv <= fixed {
            continue;
        }
        let w = 1.0 / f64::from(1u32 << lv);
        let (a, b) = (u - w, u + w);
        // A point inside a cell that was cut through its row keeps its place on the surface; in a
        // quilt's patch, the whole patch's own cut keeps only the patch's halfway point there.
        let whole = |s: &Split| s.cell[1] - s.cell[0] > 1.0 - 1e-9 && s.cell[3] - s.cell[2] > 1.0 - 1e-9;
        if log.iter().any(|s| {
            !(rules.twist_spreads && lv >= 2 && whole(s)) && s.cell[0] <= a + 1e-9 && s.cell[1] >= b - 1e-9 && s.cell[2] < v - 1e-9 && s.cell[3] > v + 1e-9
        }) {
            continue;
        }
        let seg = segment(net.row(v), a, b);
        if need(&seg) < 1.0 - EPS && uneven_drift(&seg) < UNEVEN_TOL {
            let m = onto_chord(place(&own, a, v), place(&own, b, v), surf(u, v));
            own.insert(key(u, v), m);
        }
    }
    // The same for columns: on the surface's left and right sides, or (a tall envelope) everywhere,
    // by the row rule turned on its side. Inside other envelopes, only where the column's own
    // stretch is nearly straight.
    order.sort_by_cached_key(|p| level(p[1]));
    for &[u, v] in &order {
        let lv = level(v);
        let w = 1.0 / f64::from(1u32 << lv);
        let (a, b) = (v - w, v + w);
        let on_side = (u.abs() < 1e-12 && !rules.surface_sides[0]) || ((u - 1.0).abs() < 1e-12 && !rules.surface_sides[1]);
        let across = |s: &Split| s.cell[2] <= a + 1e-9 && s.cell[3] >= b - 1e-9 && s.cell[0] < u - 1e-9 && s.cell[1] > u + 1e-9;
        if let (None, Some(tol), false) = (rules.column_chords, rules.flat_columns, on_side) {
            if lv > fixed_level && !log.iter().any(across) && need(&segment(net.col(u), a, b)) < tol {
                let m = onto_chord(place(&own, u, a), place(&own, u, b), surf(u, v));
                own.insert(key(u, v), m);
            }
            continue;
        }
        if let Some(fixed) = rules.column_chords {
            if lv <= fixed {
                continue;
            }
            if log.iter().any(across) {
                continue;
            }
        } else if lv <= fixed_level || !on_side {
            continue;
        }
        let seg = segment(net.col(u), a, b);
        if need(&seg) < 1.0 - EPS && (rules.side_seam || uneven_drift(&seg) < UNEVEN_TOL) {
            let m = onto_chord(place(&own, u, a), place(&own, u, b), surf(u, v));
            own.insert(key(u, v), m);
        }
    }
    // A point inside a bigger cell's edge is where the surface puts it, as far as its own cells see
    // it (the chord rules above are for points that are corners on both sides).
    for &[u0, u1, v0, v1] in leaves {
        for v in [v0, v1] {
            for &u in inside(on_v.get(&v.to_bits()), u0, u1) {
                own.insert(key(u, v), surf(u, v));
            }
        }
        for u in [u0, u1] {
            for &v in inside(on_u.get(&u.to_bits()), v0, v1) {
                own.insert(key(u, v), surf(u, v));
            }
        }
    }
    own
}

/// Vertices, their places and the quads of `leaves` (stored frame).
fn triangulate(net: &Net, surf: &dyn Fn(f64, f64) -> P, leaves: &[Cell], log: &[Split], rules: &Rules) -> EnvelopeMesh {
    // Every corner, and the corners along each horizontal (by v) and vertical (by u) line.
    let mut keys: Vec<P> = Vec::new();
    let mut seen: HashMap<(u64, u64), ()> = HashMap::new();
    for &[u0, u1, v0, v1] in leaves {
        for p in [[u0, v0], [u1, v0], [u0, v1], [u1, v1]] {
            if seen.insert(key(p[0], p[1]), ()).is_none() {
                keys.push(p);
            }
        }
    }
    keys.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let mut on_v: HashMap<u64, Vec<f64>> = HashMap::new();
    let mut on_u: HashMap<u64, Vec<f64>> = HashMap::new();
    for p in &keys {
        on_v.entry(p[1].to_bits()).or_default().push(p[0]);
        on_u.entry(p[0].to_bits()).or_default().push(p[1]);
    }
    for l in on_v.values_mut().chain(on_u.values_mut()) {
        l.sort_by(f64::total_cmp);
    }
    let own = own_points(net, surf, &keys, leaves, log, rules, (&on_v, &on_u));
    let place = |pos: &HashMap<(u64, u64), P>, u: f64, v: f64| pos.get(&key(u, v)).copied().unwrap_or_else(|| surf(u, v));
    // Then the biggest cells first drop the points inside their edges onto them (perpendicular), so
    // a point keeps its place along the edge as its own cells see it: the cells meet without a
    // gap, and the bigger cell's texture runs on along its straight edge.
    let mut pos: HashMap<(u64, u64), P> = HashMap::new();
    let mut by_size: Vec<&Cell> = leaves.iter().collect();
    by_size.sort_by(|a, b| ((b[1] - b[0]) * (b[3] - b[2])).total_cmp(&((a[1] - a[0]) * (a[3] - a[2]))));
    // Lines the T-point rule keeps on the surface: rows by the row rule's levels, columns by the
    // fixed level.
    let fixed_row = |v: f64| {
        level(v)
            <= match level(v) {
                0 => 0,
                1 => rules.mid_row_level,
                _ => rules.fixed_level,
            }
    };
    let fixed_col = |u: f64| level(u) <= rules.fixed_level;
    let area = |c: &Cell| (c[1] - c[0]) * (c[3] - c[2]);
    // Every leaf's top and bottom edges by their row, its sides by their column, and the areas of
    // the leaves meeting at each corner.
    let mut rows_at: HashMap<u64, Vec<(f64, f64, f64)>> = HashMap::new();
    let mut cols_at: HashMap<u64, Vec<(f64, f64, f64)>> = HashMap::new();
    let mut corner_areas: HashMap<(u64, u64), Vec<f64>> = HashMap::new();
    for c in leaves {
        let [u0, u1, v0, v1] = *c;
        for v in [v0, v1] {
            rows_at.entry(v.to_bits()).or_default().push((u0, u1, area(c)));
        }
        for u in [u0, u1] {
            cols_at.entry(u.to_bits()).or_default().push((v0, v1, area(c)));
        }
        for p in [[u0, v0], [u1, v0], [u0, v1], [u1, v1]] {
            corner_areas.entry(key(p[0], p[1])).or_default().push(area(c));
        }
    }
    // A corner lying inside the edge of a leaf at least as big (not on a line kept on the surface)
    // is that edge's to place: two equal leaves meeting at a T don't race for it.
    let edge_point = |u: f64, v: f64, at_least: f64| {
        let inside_of = |t: f64, (a, b, s): &(f64, f64, f64)| *s >= at_least - 1e-12 && t > a + 1e-12 && t < b - 1e-12;
        (!fixed_row(v) && rows_at.get(&v.to_bits()).is_some_and(|l| l.iter().any(|e| inside_of(u, e))))
            || (!fixed_col(u) && cols_at.get(&u.to_bits()).is_some_and(|l| l.iter().any(|e| inside_of(v, e))))
    };
    let corner_of_equal = |u: f64, v: f64, me: f64| corner_areas.get(&key(u, v)).is_some_and(|l| l.iter().any(|s| (s - me).abs() < 1e-12));
    for c in &by_size {
        let [u0, u1, v0, v1] = **c;
        let me = area(c);
        let mut corner = |u: f64, v: f64| {
            if let Some(p) = pos.get(&key(u, v)) {
                return *p;
            }
            if edge_point(u, v, me) {
                return place(&own, u, v);
            }
            *pos.entry(key(u, v)).or_insert_with(|| place(&own, u, v))
        };
        let (c00, c10, c01, c11) = (corner(u0, v0), corner(u1, v0), corner(u0, v1), corner(u1, v1));
        // On a line kept on the surface, a corner of a leaf as big as this one keeps its own place.
        for (v, a, b) in [(v0, c00, c10), (v1, c01, c11)] {
            for &u in inside(on_v.get(&v.to_bits()), u0, u1) {
                if fixed_row(v) && corner_of_equal(u, v, me) {
                    continue;
                }
                let o = place(&own, u, v);
                pos.entry(key(u, v)).or_insert_with(|| onto(a, b, o));
            }
        }
        for (u, a, b) in [(u0, c00, c01), (u1, c10, c11)] {
            for &v in inside(on_u.get(&u.to_bits()), v0, v1) {
                if fixed_col(u) && corner_of_equal(u, v, me) {
                    continue;
                }
                let o = place(&own, u, v);
                pos.entry(key(u, v)).or_insert_with(|| onto(a, b, o));
            }
        }
    }
    for p in &keys {
        let o = place(&own, p[0], p[1]);
        pos.entry(key(p[0], p[1])).or_insert(o);
    }
    if !rules.cut_sides {
        // A leaf stays whole: a point of a smaller neighbour on its left or right side is dropped
        // onto that side's chord (biggest leaves first, so a side's own ends are where they belong).
        let mut big: Vec<&Cell> = leaves.iter().collect();
        big.sort_by(|a, b| (b[3] - b[2]).total_cmp(&(a[3] - a[2])));
        for &&[u0, u1, v0, v1] in &big {
            for u in [u0, u1] {
                let (a, b) = (place(&pos, u, v0), place(&pos, u, v1));
                for &v in inside(on_u.get(&u.to_bits()), v0, v1) {
                    let o = place(&own, u, v);
                    pos.insert(key(u, v), onto(a, b, o));
                }
            }
        }
    }
    // Quads: each leaf cut across at the vertices on its left and right sides, where it is.
    let mut uv: Vec<P> = Vec::new();
    let mut out_pos: Vec<P> = Vec::new();
    let mut index: HashMap<(u64, u64), usize> = HashMap::new();
    let mut quads = Vec::new();
    for &[u0, u1, v0, v1] in leaves {
        let mut vs = vec![v0, v1];
        if rules.cut_sides {
            // Not at a side on the surface's middle line (u = ½): the two halves stay whole there.
            for u in [u0, u1] {
                if (u - 0.5).abs() > 1e-12 {
                    vs.extend(inside(on_u.get(&u.to_bits()), v0, v1));
                }
            }
        }
        vs.sort_by(f64::total_cmp);
        vs.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let (c00, c10, c01, c11) = (place(&pos, u0, v0), place(&pos, u1, v0), place(&pos, u0, v1), place(&pos, u1, v1));
        let mut vertex = |u: f64, v: f64, uv: &mut Vec<P>, out_pos: &mut Vec<P>| -> usize {
            if let Some(&p) = pos.get(&key(u, v)) {
                return *index.entry(key(u, v)).or_insert_with(|| {
                    uv.push([u, v]);
                    out_pos.push(p);
                    uv.len() - 1
                });
            }
            // A point of this cell's side that is no vertex: dropped onto the side's chord from its own place.
            uv.push([u, v]);
            let o = place(&own, u, v);
            out_pos.push(if u == u0 { onto(c00, c01, o) } else { onto(c10, c11, o) });
            uv.len() - 1
        };
        for w in vs.windows(2) {
            let (va, vb) = (w[0], w[1]);
            let tl = vertex(u0, va, &mut uv, &mut out_pos);
            let tr = vertex(u1, va, &mut uv, &mut out_pos);
            let bl = vertex(u0, vb, &mut uv, &mut out_pos);
            let br = vertex(u1, vb, &mut uv, &mut out_pos);
            quads.push([tl, tr, bl, br]);
        }
    }
    EnvelopeMesh { uv, pos: out_pos, quads }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bicubic Bezier patch with the 4 × 4 control points `p` (row-major).
    fn bicubic(p: [P; 16]) -> impl Fn(f64, f64) -> P {
        let b = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
        move |u: f64, v: f64| {
            let (bu, bv) = (b(u), b(v));
            let mut o = [0.0; 2];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    o[0] += wu * wv * p[j * 4 + i][0];
                    o[1] += wu * wv * p[j * 4 + i][1];
                }
            }
            o
        }
    }

    /// A bilinear (flat) map: the four first-level cells per side stay whole.
    #[test]
    fn a_flat_map_stays_at_sixteen_cells() {
        let at = |u: f64, v: f64| [10.0 + 100.0 * u, 20.0 + 50.0 * v];
        let m = envelope_mesh(&at, false).expect("finite");
        assert_eq!(m.quads.len(), 16);
        assert_eq!(m.uv.len(), 25);
    }

    /// A full mesh keeps its cubic net where its rows are only quadratic (an arch drawn by hand), so
    /// the twist test still applies to it; the same map as an envelope is stored quadratic.
    #[test]
    fn a_full_mesh_is_cut_at_cubic_degree() {
        let at = |u: f64, v: f64| [128.0 * u, 128.0 * v - 30.0 * 4.0 * u * (1.0 - u) * (1.0 - 2.0 * v)];
        let full = net_of(&at, true);
        assert_eq!((full.nu, full.nv), (4, 4));
        let env = net_of(&at, false);
        assert_eq!((env.nu, env.nv), (3, 2));
    }

    /// A 128 px square with its top-left corner pulled 48 px out diagonally: the reference keeps
    /// the far end of the top row (a quarter by a quarter) whole, though cells of that row under
    /// another parent split down for their curvature: the row rule stays among siblings.
    #[test]
    fn the_row_rule_stays_among_siblings() {
        let mut p = [[0.0f64; 2]; 16];
        for (k, q) in p.iter_mut().enumerate() {
            *q = [128.0 * (k % 4) as f64 / 3.0, 128.0 * (k / 4) as f64 / 3.0];
        }
        p[0] = [-48.0, -48.0];
        let at = bicubic(p);
        let net = net_of(&at, true);
        let (leaves, log) = cells(&net, true, &one_patch_rules(&net, true));
        assert!(log.iter().any(|s| s.cell == [0.0, 0.25, 0.0, 0.25]), "the corner cell splits");
        assert!(leaves.contains(&[0.75, 1.0, 0.0, 0.25]), "{leaves:?}");
    }

    /// A full mesh starts as 2 × 2 cells: a flat one stays at four.
    #[test]
    fn a_flat_full_mesh_stays_at_four_cells() {
        let at = |u: f64, v: f64| [10.0 + 100.0 * u, 20.0 + 50.0 * v];
        let m = envelope_mesh(&at, true).expect("finite");
        assert_eq!(m.quads.len(), 4);
    }

    /// The reference application's cells for a 128 px square with one handle (the top-left
    /// corner's, along the top) dragged 40 px down, read from its coordinate probe: the far,
    /// nearly flat quarter stays as two cells half wide and a quarter tall. A down split for
    /// curvature carries along its row; a twist elsewhere on the level does not.
    #[test]
    fn a_full_mesh_splits_rows_not_levels() {
        let mut p = [[0.0f64; 2]; 16];
        for (k, q) in p.iter_mut().enumerate() {
            *q = [128.0 * (k % 4) as f64 / 3.0, 128.0 * (k / 4) as f64 / 3.0];
        }
        p[1][1] = 40.0;
        let at = bicubic(p);
        let m = envelope_mesh(&at, true).expect("finite");
        let mut far: Vec<[f64; 4]> = m
            .quads
            .iter()
            .map(|&[tl, _, _, br]| [m.uv[tl][0], m.uv[br][0], m.uv[tl][1], m.uv[br][1]])
            .filter(|c| c[0] >= 0.5 - 1e-9 && c[2] >= 0.5 - 1e-9)
            .collect();
        far.sort_by(|a, b| a[2].total_cmp(&b[2]));
        assert_eq!(far, vec![[0.5, 1.0, 0.5, 0.75], [0.5, 1.0, 0.75, 1.0]]);
    }

    #[test]
    fn a_quadratic_direction_is_measured_at_its_own_degree() {
        // An arch: rows quadratic, columns straight.
        let at = |u: f64, v: f64| [100.0 * u, 40.0 * v + 30.0 * 4.0 * u * (1.0 - u)];
        let n = stored_net(&cubic_net(&at));
        assert_eq!((n.nu, n.nv), (3, 2));
    }

    /// At a fold tip under 1.5 px (crease redraws), the point past the end is held at the end
    /// and counts toward the mean: the inside point's 0.59 px drift alone would cut the chord.
    #[test]
    fn a_fold_tip_counts_its_point_past_the_end_toward_the_drift() {
        let l = 0.997;
        assert!(need(&[[0.0, 0.0], [0.925 * l, 0.0], [1.232 * l, 0.0], [l, 0.0]]) < 1.0);
        // Not past FOLD_TIP_CHORD: held at the end of a 3 px chord, the second point of this
        // shallow turn would count a full px of drift and cut what the reference keeps whole.
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [3.4, 0.0], [3.0, 0.0]]) < 1.0);
    }

    #[test]
    fn need_tests_sideways_and_along() {
        // Inner points of a straight cubic evenly spaced: nothing to split.
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]]) < 1e-12);
        // One px sideways splits.
        assert!(need(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 0.0]]) >= 1.0);
        // Both inner points half a px along the chord split; a quarter doesn't.
        assert!(need(&[[0.0, 0.0], [1.5, 0.0], [2.5, 0.0], [3.0, 0.0]]) >= 1.0);
        assert!(need(&[[0.0, 0.0], [1.25, 0.0], [2.25, 0.0], [3.0, 0.0]]) < 1.0);
    }

    /// A quilt cell's row with inner points drifting 0.65 px apart asks for a split across, though
    /// its plain need is small; a column needs [`QUILT_COL_UNEVEN_TOL`]; outside quilts nothing
    /// changes.
    #[test]
    fn quilt_cut_need_counts_uneven_drift() {
        let seg = [[0.0, 0.0], [10.325, 0.0], [19.675, 0.0], [30.0, 0.0]];
        assert!(need(&seg) < 1.0);
        assert!((uneven_drift(&seg) - 0.65).abs() < 1e-9);
        assert!(cut_need(&seg, QUILT_ROW_UNEVEN_TOL) >= 1.0);
        assert!(cut_need(&seg, QUILT_COL_UNEVEN_TOL) < 1.0);
        assert_eq!(cut_need(&seg, f64::INFINITY), need(&seg));
    }

    /// Inner points drifting apart along the chord count as uneven; drifting together, they don't.
    #[test]
    fn uneven_drift_takes_the_first_drift_against_the_last() {
        let row = |d1: f64, d2: f64| [[0.0, 0.0], [1.0 + d1, 0.0], [2.0 + d2, 0.0], [3.0, 0.0]];
        assert!((uneven_drift(&row(0.3, -0.2)) - 0.5).abs() < 1e-12);
        assert!(uneven_drift(&row(-0.5, -0.45)) < 0.1);
        // Opposite drifts too small to split by the mean still keep the point on the surface.
        assert!(need(&row(0.3, -0.2)) < 1.0 && uneven_drift(&row(0.3, -0.2)) >= UNEVEN_TOL);
        assert!(uneven_drift(&[[0.0, 0.0], [1.0, 0.4], [2.0, 0.0]]) == 0.0);
        assert!(uneven_drift(&[[0.0, 0.0], [0.5, 0.0], [0.5, 0.0], [0.0, 0.0]]) == 0.0);
    }

    /// A control point outside its chord's span (a row turning back on itself in a crease) counts
    /// by its distance from that end, not in the mean along the chord (except at a fold tip, see
    /// [`FOLD_TIP_CHORD`]).
    #[test]
    fn need_measures_a_point_past_the_chord_from_that_end() {
        let t = 4.0 / 3.0;
        // A long turn back (more than SHALLOW_BEFORE_REACH chord lengths before the start): 1 px
        // before a 2 px chord stays whole (the mean along the chord would split it); 1.4 px splits.
        assert!(need(&[[0.0, 0.0], [-1.0, 0.0], [t, 0.0], [2.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [-1.4, 0.0], [t, 0.0], [2.0, 0.0]]) >= 1.0);
        // Past the end the tolerance is 1 px (reaching past SHALLOW_AFTER_REACH chord lengths).
        assert!(need(&[[0.0, 0.0], [1.0 / 3.0, 0.0], [1.9, 0.0], [1.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [1.0 / 3.0, 0.0], [2.1, 0.0], [1.0, 0.0]]) >= 1.0);
        // The distance is from the end itself, sideways included: 1.2 px before and 0.7 px off.
        assert!(need(&[[0.0, 0.0], [-1.2, 0.0], [t, 0.0], [2.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [-1.2, 0.7], [t, 0.0], [2.0, 0.0]]) >= 1.0);
        // The points left inside still split by their own mean: more than half a px along.
        assert!(need(&[[0.0, 0.0], [-1.0, 0.0], [1.9, 0.0], [2.0, 0.0]]) >= 1.0);
        // A shallow turn back (a fold's apex inside the segment) splits sooner: from half a px
        // before the start, from 0.45 px past the end.
        assert!(need(&[[0.0, 0.0], [-0.45, 0.0], [2.0, 0.0], [3.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [-0.55, 0.0], [2.0, 0.0], [3.0, 0.0]]) >= 1.0);
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [3.4, 0.0], [3.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [3.5, 0.0], [3.0, 0.0]]) >= 1.0);
        // Turning back sharply (both inner points past the end, close together): past the end from
        // 0.34 times the reach (at most 0.7 px); spread apart, from 1 px.
        let sharp = |l: f64| need(&[[0.0, 0.0], [1.743 * l, 0.0], [1.976 * l, 0.0], [l, 0.0]]);
        assert!(sharp(0.6) < 1.0 && sharp(0.75) >= 1.0, "{} {}", sharp(0.6), sharp(0.75));
        assert!(need(&[[0.0, 0.0], [1.476 * 0.75, 0.0], [1.976 * 0.75, 0.0], [0.75, 0.0]]) < 1.0);
        // Reaching 3.3 chord lengths: at most 0.7 px; reaching 1.55 (short of SHALLOW_AFTER_REACH): 0.53 px.
        let long = |l: f64| need(&[[0.0, 0.0], [3.183 * l, 0.0], [3.312 * l, 0.0], [l, 0.0]]);
        assert!(long(0.29) < 1.0 && long(0.32) >= 1.0, "{} {}", long(0.29), long(0.32));
        let short = |l: f64| need(&[[0.0, 0.0], [1.288 * l, 0.0], [1.553 * l, 0.0], [l, 0.0]]);
        assert!(short(0.93) < 1.0 && short(0.99) >= 1.0, "{} {}", short(0.93), short(0.99));
        // Both points outside and close to the ends: nothing to split; non-finite points neither.
        assert!(need(&[[0.0, 0.0], [-0.4, 0.0], [3.4, 0.0], [3.0, 0.0]]) < 1.0);
        assert_eq!(need(&[[0.0, 0.0], [f64::NAN, 0.0], [2.0, f64::INFINITY], [3.0, 0.0]]), 0.0);
    }

    /// The preset envelopes are polynomial patches at the degrees the reference stores them at, so
    /// the fitted net reproduces the map and reduces to those degrees.
    #[test]
    fn preset_envelopes_fit_exactly_at_their_stored_degrees() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        for (style, want) in [
            (WarpStyle::Arc, (4, 2)),
            (WarpStyle::Arch, (4, 2)),
            (WarpStyle::Bulge, (4, 2)),
            (WarpStyle::Flag, (4, 2)),
            (WarpStyle::Fish, (4, 2)),
            (WarpStyle::Rise, (4, 2)),
            (WarpStyle::ArcLower, (4, 2)),
            (WarpStyle::Inflate, (3, 3)),
            (WarpStyle::Squeeze, (3, 3)),
            (WarpStyle::Wave, (4, 3)),
        ] {
            let w = Warp::preset(style, 50.0, b);
            let at = |u: f64, v: f64| {
                let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
                [x, y]
            };
            let cubic = cubic_net(&at);
            for k in 0..=10 {
                let (u, v) = (k as f64 / 10.0, (k * 7 % 11) as f64 / 10.0);
                let q = blend(cubic.col(u), v);
                let p = at(u, v);
                assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{style:?} at ({u}, {v}): {p:?} vs {q:?}");
            }
            let n = stored_net(&cubic);
            assert_eq!((n.nu, n.nv), want, "{style:?}");
        }
    }

    #[test]
    fn a_non_finite_map_is_refused() {
        assert!(envelope_mesh(&|_, _| [f64::NAN, 0.0], false).is_none());
        assert!(envelope_mesh(&|_, _| [f64::NAN, 0.0], true).is_none());
    }

    /// A flat patch of a quilt stays one cell: the reference leaves flat or small patches whole
    /// (a one-patch full mesh starts at four).
    #[test]
    fn a_flat_quilt_patch_stays_whole() {
        let at = |u: f64, v: f64| [10.0 + 100.0 * u, 20.0 + 50.0 * v];
        let m = quilt_patch_mesh(&at, Seams::default()).expect("finite");
        assert_eq!(m.quads.len(), 1);
        assert_eq!(m.uv.len(), 4);
        assert!(quilt_patch_mesh(&|_, _| [f64::NAN, 0.0], Seams::default()).is_none());
    }

    /// A quilt patch whose rows bend but whose columns are straight splits across only; a one-patch
    /// full mesh splits its first cells down as well.
    #[test]
    fn a_quilt_patch_splits_only_where_it_bends() {
        let at = |u: f64, v: f64| [128.0 * u, 128.0 * v + 20.0 * (std::f64::consts::PI * u).sin()];
        let m = quilt_patch_mesh(&at, Seams::default()).expect("finite");
        assert!(m.quads.len() > 1);
        for q in &m.quads {
            let (top, bottom) = (m.uv[q[0]][1], m.uv[q[2]][1]);
            assert!(top.abs() < 1e-12 && (bottom - 1.0).abs() < 1e-12, "a cell was split down: {top}..{bottom}");
        }
        let one = envelope_mesh(&at, true).expect("finite");
        assert!(one.quads.iter().any(|q| one.uv[q[2]][1] - one.uv[q[0]][1] < 1.0 - 1e-9));
    }

    /// The top-left patch of a 2 × 2 quilt over a wave (a 128 px box, control points moved down by
    /// 12 sin 2πu · sin πv px): its top cells split down for their twist, and the reference splits
    /// the bottom cells of the same parent down with them, though they aren't twisted enough.
    #[test]
    fn a_twist_split_down_carries_to_its_siblings() {
        let pi = std::f64::consts::PI;
        let mut p = [[0.0f64; 2]; 16];
        for (k, q) in p.iter_mut().enumerate() {
            let (x, y) = (0.5 * (k % 4) as f64 / 3.0, 0.5 * (k / 4) as f64 / 3.0);
            *q = [128.0 * x, 128.0 * y + 12.0 * (2.0 * pi * x).sin() * (pi * y).sin()];
        }
        let wave = bicubic(p);
        let tallest = |rules: &Rules| {
            let (leaves, _) = cells(&cubic_net(&wave), true, rules);
            leaves.iter().filter(|c| c[2] >= 0.5 - 1e-9).map(|c| c[3] - c[2]).fold(0.0, f64::max)
        };
        let quilt = Rules {
            start: 1,
            force_down: false,
            twist_spreads: true,
            twist_spreads_in_row: false,
            fixed_level: 0,
            mid_row_level: 0,
            cut_sides: false,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
            surface_sides: [false; 2],
            uneven_quilt: false,
            side_seam: false,
        };
        assert!(tallest(&quilt) <= 0.25 + 1e-9, "bottom cells {}", tallest(&quilt));
        let alone = Rules { twist_spreads: false, ..quilt };
        assert!(tallest(&alone) >= 0.5 - 1e-9, "without the rule the bottom cells stay half the patch");
    }

    /// Two leaves of the same size meeting at a T (an eighth by a sixteenth beside a sixteenth by
    /// an eighth) don't race for the point they share, so a mesh symmetric top to bottom keeps its
    /// vertices symmetric (the reference's smooth pinch does; placing by leaf order didn't).
    #[test]
    fn equal_leaves_place_their_shared_point_alike() {
        for amp in [30.0, 60.0, 90.0, 120.0, 150.0] {
            for fall in [2.0, 4.0, 8.0, 16.0] {
                let pinch = |u: f64, v: f64| {
                    let (dx, dy) = (u - 0.5, v - 0.5);
                    let k = amp * (-(dx * dx + dy * dy) * fall).exp();
                    [128.0 * u - k * dx, 128.0 * v - k * dy]
                };
                let m = envelope_mesh(&pinch, true).expect("finite");
                for (k, p) in m.uv.iter().enumerate() {
                    let Some(j) = m.uv.iter().position(|q| (q[0] - p[0]).abs() < 1e-12 && (q[1] - (1.0 - p[1])).abs() < 1e-12) else { continue };
                    let (a, b) = (m.pos[k], m.pos[j]);
                    assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] + b[1] - 128.0).abs() < 1e-9, "amp {amp} fall {fall} {p:?}: {a:?} vs {b:?}");
                }
            }
        }
    }

    fn quilt_rules() -> Rules {
        Rules {
            start: 1,
            force_down: false,
            twist_spreads: true,
            twist_spreads_in_row: false,
            fixed_level: 0,
            mid_row_level: 0,
            cut_sides: false,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
            surface_sides: [false; 2],
            uneven_quilt: false,
            side_seam: false,
        }
    }

    /// The parts of a quad's twist `P00 − P10 − P01 + P11`: along the rows (the rows stretch
    /// unevenly) and across them.
    #[test]
    fn a_twist_splits_into_its_parts_along_and_across_the_rows() {
        let quad = |d: P| Net { nu: 2, nv: 2, p: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], d] };
        let (du, dv) = ([10.0, 0.0], [0.0, 10.0]);
        // Twist (2, 0): along the rows only, counted at half across the rows.
        let along = quad([12.0, 10.0]);
        assert!((along.twist_along(du) - 2.0).abs() < 1e-12 && along.twist_along(dv).abs() < 1e-12);
        assert!((along.twist_across_rows(du, dv, 2.0, usize::MAX) - 1.0).abs() < 1e-12);
        // Twist (0, 1.5): across the rows, in full.
        let across = quad([10.0, 11.5]);
        assert!((across.twist_across_rows(du, dv, 2.0, usize::MAX) - 1.5).abs() < 1e-12);
        assert!((across.twist_along(dv) - 1.5).abs() < 1e-12);
        // A degenerate side: the whole twist, or nothing along it.
        assert!((along.twist_across_rows([0.0, 0.0], dv, 2.0, usize::MAX) - 2.0).abs() < 1e-12);
        assert_eq!(along.twist_along([0.0, 0.0]), 0.0);
    }

    /// A whole quilt patch splits down for the twist across its rows; rows that only stretch
    /// unevenly count half. Bent patches the reference redrew stay whole at 1.2 to 1.9 px of twist
    /// along their rows; waved ones (twist across the rows, spread evenly) stay whole up to about
    /// 1.73 px and split from about 1.85 px.
    #[test]
    fn a_quilt_patch_bent_along_its_rows_does_not_split_down() {
        let quilt = quilt_rules();
        let splits_down = |at: &dyn Fn(f64, f64) -> P| {
            let (_, log) = cells(&cubic_net(at), true, &quilt);
            log.iter().any(|s| s.down && s.cell[3] - s.cell[2] > 1.0 - 1e-9)
        };
        // Straight rows and columns, twist 16/9 px per quad of the cubic net.
        let bend = |u: f64, v: f64| [64.0 * u + 16.0 * u * v, 64.0 * v];
        let wave = |u: f64, v: f64| [64.0 * u, 64.0 * v + 20.7 * u * v];
        let slight_wave = |u: f64, v: f64| [64.0 * u, 64.0 * v + 13.5 * u * v];
        assert!(!splits_down(&bend), "twist along the rows split the patch down");
        assert!(splits_down(&wave), "2.3 px of twist across the rows left the patch whole");
        assert!(!splits_down(&slight_wave), "1.5 px of twist across the rows split the patch down");
        // Twist along the rows past ROW_TWIST_ACROSS_TOL splits the whole patch across instead.
        let splits_across = |at: &dyn Fn(f64, f64) -> P| {
            let (_, log) = cells(&cubic_net(at), true, &quilt);
            log.iter().any(|s| s.across && s.cell[1] - s.cell[0] > 1.0 - 1e-9 && s.cell[3] - s.cell[2] > 1.0 - 1e-9)
        };
        assert!(splits_across(&bend), "16/9 px of twist along the rows left the patch whole across");
        let slight = |u: f64, v: f64| [64.0 * u + 9.9 * u * v, 64.0 * v];
        assert!(!splits_across(&slight), "1.1 px of twist along the rows split the patch across");
        let (_, log) = cells(&cubic_net(&bend), true, &Rules { twist_spreads: false, ..quilt });
        assert!(log.iter().any(|s| s.down), "a one-patch mesh splits for any twist");
    }

    /// A wave ±50 net scaled to 0.9 (the reference application's redraw of it): the cell at
    /// u 0..⅛, v ⅛..¼ splits down for its top edge's twist, and so does its row sibling; its mirror
    /// at the bottom, twisted as much down its column but less along its top edge, stays whole
    /// down.
    #[test]
    fn a_scaled_wave_cuts_its_top_side_cells_down_and_not_its_bottom_ones() {
        let rows = [[83.2, 83.2, 83.2, 83.2], [134.4, 226.56, 42.24, 134.4], [185.6, 277.76, 93.44, 185.6], [236.8, 236.8, 236.8, 236.8]];
        let p = (0..16).map(|k| [96.0 + 128.0 * (k % 4) as f64 / 3.0, rows[k / 4][k % 4]]).collect();
        let net = Net { nu: 4, nv: 4, p };
        let (leaves, _) = cells(&net, true, &one_patch_rules(&net, true));
        let leaf = |c: Cell| leaves.iter().any(|l| (0..4).all(|k| (l[k] - c[k]).abs() < EPS));
        assert!(leaf([0.0, 0.0625, 0.125, 0.1875]) && leaf([0.125, 0.1875, 0.125, 0.1875]), "top: cut down");
        assert!(leaf([0.0, 0.0625, 0.75, 0.875]), "bottom: whole down");
    }

    /// A wave ±50 net scaled to 0.7: the corner cell at the top (u 0..⅛, v 0..⅛, twist 1.12 on its
    /// top edge) splits down for its twist, its mirror at the bottom (as twisted in all, 0.84 on
    /// its top edge) stays whole down.
    #[test]
    fn a_full_mesh_cell_splits_down_for_the_twist_of_its_top_edge() {
        let rows = [[83.2, 83.2, 83.2, 83.2], [134.4, 206.08, 62.72, 134.4], [185.6, 257.28, 113.92, 185.6], [236.8, 236.8, 236.8, 236.8]];
        let p = (0..16).map(|k| [96.0 + 128.0 * (k % 4) as f64 / 3.0, rows[k / 4][k % 4]]).collect();
        let net = Net { nu: 4, nv: 4, p };
        let (leaves, _) = cells(&net, true, &one_patch_rules(&net, true));
        let leaf = |c: Cell| leaves.iter().any(|l| (0..4).all(|k| (l[k] - c[k]).abs() < EPS));
        assert!(leaf([0.0, 0.125, 0.0, 0.0625]), "top: cut down");
        assert!(leaf([0.0, 0.125, 0.875, 1.0]), "bottom: whole down");
    }

    /// A full mesh's cell that splits across splits down too where its top edge twists past
    /// TWIST_TOL, though its columns are straight.
    #[test]
    fn a_full_mesh_cell_split_across_splits_down_for_its_top_edge_alone() {
        let at = |u: f64, v: f64| [128.0 * u, 128.0 * v + 20.0 * u * u * v];
        let net = cubic_net(&at);
        let (_, log) = cells(&net, true, &one_patch_rules(&net, true));
        let cell = [0.5, 1.0, 0.5, 1.0];
        let split = log.iter().find(|s| s.cell.iter().zip(&cell).all(|(a, b)| (a - b).abs() < 1e-9));
        assert!(split.is_some_and(|s| s.across && s.down), "the cell didn't split both ways: {:?}", split.map(|s| (s.across, s.down)));
    }

    /// A Fish saved as a full mesh (the reference application's redraws of re-saved presets): four
    /// cells in its top and bottom rows whose neighbours in the row split down for their twist
    /// split down too; nothing else changes, above or below them.
    #[test]
    fn a_full_mesh_twist_split_down_spreads_along_its_row() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = Warp::preset(WarpStyle::Fish, 50.0, b);
        let at = |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        };
        let net = cubic_net(&at);
        let rules = one_patch_rules(&net, true);
        let splits = |r: &Rules| {
            let (_, log) = cells(&net, true, r);
            log.iter().map(|s| (s.cell.map(|x| (x * 64.0).round() as i64), s.across, s.down)).collect::<std::collections::BTreeSet<_>>()
        };
        let (alone, spread) = (splits(&Rules { twist_spreads_in_row: false, ..rules }), splits(&rules));
        let cut = [[8, 12, 0, 8], [8, 12, 56, 64], [52, 56, 0, 8], [52, 56, 56, 64]];
        assert_eq!(alone.difference(&spread).cloned().collect::<Vec<_>>(), cut.map(|c| (c, true, false)), "cells that stopped splitting across only");
        // Split down too, their halves split across only.
        let halves = [[8, 12, 0, 4], [8, 12, 60, 64], [52, 56, 0, 4], [52, 56, 60, 64]];
        assert_eq!(spread.difference(&alone).cloned().collect::<Vec<_>>(), halves.map(|c| (c, true, false)), "new splits");
    }

    /// A wave's bottom patch: its control points (v from 2/3 to 1 of a 128 px mesh of three rows),
    /// each moved down by 5 sin(2 pi (u + 1/8)) sin(pi v).
    fn wave_bottom_patch() -> Net {
        wave_bottom_patch_by(5.0)
    }

    /// [`wave_bottom_patch`] with its control points moved down by `amp` sin(2 pi (u + 1/8)) sin(pi v).
    fn wave_bottom_patch_by(amp: f64) -> Net {
        let pi = std::f64::consts::PI;
        let p = (0..4)
            .flat_map(|j| {
                (0..4).map(move |i| {
                    let (u, v) = (i as f64 / 3.0, 2.0 / 3.0 + j as f64 / 9.0);
                    [128.0 * u, 128.0 * v + amp * (2.0 * pi * (u + 0.125)).sin() * (pi * v).sin()]
                })
            })
            .collect();
        Net { nu: 4, nv: 4, p }
    }

    /// A whole patch of a quilt that doesn't halve evenly splits down for the twist of its first row
    /// of quads only: a wave's bottom patch (three rows) twists most along its bottom, past the
    /// whole-net tolerance, and stays whole.
    #[test]
    fn an_uneven_quilts_whole_patch_splits_down_by_its_first_row() {
        let net = wave_bottom_patch();
        let (p00, p10, p01) = (net.at(0, 0), net.at(net.nu - 1, 0), net.at(0, net.nv - 1));
        let (du, dv) = ([p10[0] - p00[0], p10[1] - p00[1]], [p01[0] - p00[0], p01[1] - p00[1]]);
        let (all, first) = (net.twist_across_rows(du, dv, 2.0, usize::MAX), net.twist_across_rows(du, dv, 2.0, 1));
        assert!(all > WHOLE_PATCH_TWIST_TOL && first < FIRST_ROW_TWIST_TOL, "{all} {first}");
        let whole_down = |seams: Seams| {
            let (_, log) = cells(&net, true, &patch_rules(seams));
            log.iter().find(|s| s.cell == [0.0, 1.0, 0.0, 1.0]).map(|s| s.down)
        };
        let top = Seams { top: true, ..Seams::default() };
        assert_eq!(whole_down(top), Some(true), "an even quilt's patch splits by its whole net");
        assert_eq!(whole_down(Seams { uneven: true, ..top }), Some(false), "an uneven quilt's patch stays whole");
    }

    /// A quilt cell as tall as its patch (the patch kept whole down) splits down for its twist
    /// along its rows alone: the wave's bottom patch splits across into halves, and the right half,
    /// twisted across its rows past the tolerance, stays whole down, so nothing splits down.
    #[test]
    fn a_full_height_quilt_cell_splits_down_by_its_row_twist_alone() {
        let net = wave_bottom_patch();
        let half = net.sub([0.5, 1.0, 0.0, 1.0]);
        let (p00, p10, p01) = (half.at(0, 0), half.at(half.nu - 1, 0), half.at(0, half.nv - 1));
        let (du, dv) = ([p10[0] - p00[0], p10[1] - p00[1]], [p01[0] - p00[0], p01[1] - p00[1]]);
        assert!(0.9 * half.twist_along(dv) > TWIST_TOL && half.twist_along(du) < TWIST_TOL, "{} {}", half.twist_along(dv), half.twist_along(du));
        let (leaves, log) = cells(&net, true, &patch_rules(Seams { top: true, uneven: true, ..Seams::default() }));
        let splits: Vec<(Cell, bool, bool)> = log.iter().map(|s| (s.cell, s.across, s.down)).collect();
        assert!(splits.iter().all(|s| !s.2), "{splits:?}");
        assert!(splits.contains(&([0.5, 1.0, 0.0, 1.0], true, false)), "{splits:?}");
        assert!(leaves.iter().all(|c| c[2] == 0.0 && c[3] == 1.0), "{leaves:?}");
    }

    /// A cell as tall as its patch splits down for its twist along its rows past [`TWIST_TOL`] in a
    /// quilt that halves evenly, but only past [`UNEVEN_FULL_HEIGHT_TWIST_TOL`] in one that doesn't:
    /// a patch sheared further along its rows the further down it is splits across into halves
    /// that twist 1.08 px along their rows, and those split down in the even quilt only.
    #[test]
    fn an_uneven_quilts_full_height_cell_splits_down_later() {
        let at = |u: f64, v: f64| [96.0 + 128.0 * u + 19.44 * u * v, 96.0 + 40.0 * v];
        let net = cubic_net(&at);
        let half = net.sub([0.5, 1.0, 0.0, 1.0]);
        let (p00, p10) = (half.at(0, 0), half.at(half.nu - 1, 0));
        let tw = half.twist_along([p10[0] - p00[0], p10[1] - p00[1]]);
        assert!(tw > TWIST_TOL && tw < UNEVEN_FULL_HEIGHT_TWIST_TOL, "{tw}");
        for uneven in [false, true] {
            let (_, log) = cells(&net, true, &patch_rules(Seams { top: true, uneven, ..Seams::default() }));
            let splits: Vec<(Cell, bool, bool)> = log.iter().map(|s| (s.cell, s.across, s.down)).collect();
            assert!(splits.contains(&([0.0, 1.0, 0.0, 1.0], true, false)), "the patch splits across only: {splits:?}");
            for c in [[0.0, 0.5, 0.0, 1.0], [0.5, 1.0, 0.0, 1.0]] {
                let down = splits.iter().any(|s| s.0 == c && s.2);
                assert_eq!(down, !uneven, "uneven {uneven}: {splits:?}");
            }
        }
    }

    /// An inner control row counts a little more than the top and bottom rows: a row that bends
    /// 0.997 px off its chord asks for a split across inside the net, not at its top.
    #[test]
    fn a_quilt_cells_inner_control_rows_count_a_little_more() {
        let net = |j_bent: usize| Net {
            nu: 4,
            nv: 4,
            p: (0..4)
                .flat_map(|j| (0..4).map(move |i| [10.0 * i as f64, 10.0 * j as f64 + if j == j_bent && (i == 1 || i == 2) { 0.997 } else { 0.0 }]))
                .collect(),
        };
        assert!(net(0).quilt_rows_need() < 1.0, "a top row at 0.997 needs no split");
        assert!(net(1).quilt_rows_need() > 1.0, "an inner row at 0.997 does");
    }

    /// In a quilt with a single row of patches, a point on a patch's inner seam stays where the
    /// surface puts it; elsewhere a nearly straight side's points sit on its chord.
    #[test]
    fn a_single_row_quilts_seam_points_stay_on_the_surface() {
        let pi = std::f64::consts::PI;
        let at = |u: f64, v: f64| [64.0 * u + 0.6 * (pi * v).sin(), 64.0 * v + 30.0 * u * v + 10.0 * (2.0 * pi * u).sin() * (pi * v).sin()];
        let seam = Seams { left: true, ..Seams::default() };
        let (plain, kept) = (quilt_patch_mesh(&at, Seams::default()).expect("finite"), quilt_patch_mesh(&at, seam).expect("finite"));
        let side = |m: &EnvelopeMesh| {
            m.uv.iter().zip(&m.pos).filter(|(q, _)| q[0].abs() < 1e-12 && q[1] > 1e-9 && q[1] < 1.0 - 1e-9).map(|(q, p)| (q[1], *p)).collect::<Vec<_>>()
        };
        let (a, b) = (side(&plain), side(&kept));
        assert!(!a.is_empty() && a.len() == b.len(), "{} {}", a.len(), b.len());
        assert!(
            b.iter().all(|(v, p)| {
                let s = at(0.0, *v);
                (p[0] - s[0]).hypot(p[1] - s[1]) < 1e-9
            }),
            "a seam point left the surface"
        );
        assert!(
            a.iter().any(|(v, p)| {
                let s = at(0.0, *v);
                (p[0] - s[0]).hypot(p[1] - s[1]) > 1e-3
            }),
            "no side point moved onto its chord: {a:?}"
        );
        let both = Seams { left: true, top: true, ..Seams::default() };
        let c = side(&quilt_patch_mesh(&at, both).expect("finite"));
        assert_eq!(a, c, "a seam with a patch above or below kept its points on the surface");
    }

    /// A quilt patch cut both ways decides its first-level cells' down cuts by row strip, as the
    /// reference redrew split waves (control points moved by a wave): the top-left patch of a 2 x 2 wave splits its top strip (the
    /// warp's top edge twists) but not the one below; the bottom patch of a 1 x 2 wave of
    /// amplitude 12 splits its top strip for the twist of the strip below, at amplitude 11 not.
    #[test]
    fn a_quilt_patch_splits_its_first_cells_down_by_row_strip() {
        let pi = std::f64::consts::PI;
        let quarters: Vec<Cell> = vec![[0.0, 0.5, 0.0, 0.5], [0.5, 1.0, 0.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 1.0, 0.5, 1.0]];
        // Control points placed where `at` puts their grid positions (as the probe files were made).
        let decide = |at: &dyn Fn(f64, f64) -> P| {
            let p = (0..16).map(|k| at(f64::from(k % 4) / 3.0, f64::from(k / 4) / 3.0)).collect();
            let mut d = vec![(true, false); 4];
            let mut t = vec![true; 4];
            strip_down_cuts(&Net { nu: 4, nv: 4, p }, &quarters, &mut d, &mut t);
            assert!(t.iter().all(|t| !t), "a strip's twist spread further");
            assert!(d[0].1 == d[1].1 && d[2].1 == d[3].1, "a strip's left and right cells disagree: {d:?}");
            (d[0].1, d[2].1)
        };
        let top_left = |u: f64, v: f64| [64.0 * u, 64.0 * v + 10.0 * (pi * u).sin() * (0.5 * pi * v).sin()];
        assert_eq!(decide(&top_left), (true, false), "the 2 x 2 wave's top-left patch");
        let bottom = |a: f64| move |u: f64, v: f64| [128.0 * u, 64.0 * v + a * (2.0 * pi * u).sin() * (0.5 * pi * (1.0 + v)).sin()];
        assert_eq!(decide(&bottom(12.0)), (true, true), "the 1 x 2 wave's bottom patch, amplitude 12");
        assert_eq!(decide(&bottom(11.0)), (false, true), "the 1 x 2 wave's bottom patch, amplitude 11");
    }

    /// A strip splits down when any column of its control net asks for it, though the columns at
    /// the patch's sides and middle are straight: its inner control columns bow out and back.
    #[test]
    fn a_quilt_patch_strip_measures_its_inner_control_columns() {
        let (bow, rows) = ([0.0, 1.0, -1.0, 0.0], [0.0, 0.0, 1.0, 1.0]);
        let p = (0..16).map(|k: usize| [128.0 * (k % 4) as f64 / 3.0 + 8.0 * bow[k % 4] * rows[k / 4], 64.0 * (k / 4) as f64 / 3.0]).collect();
        let net = Net { nu: 4, nv: 4, p };
        let straight = [0.0, 0.5, 1.0].iter().map(|&u| need(&segment(net.col(u), 0.0, 0.5))).fold(0.0, f64::max);
        assert!(straight < 1e-9, "a side or middle column bends: {straight}");
        let quarters: Vec<Cell> = vec![[0.0, 0.5, 0.0, 0.5], [0.5, 1.0, 0.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 1.0, 0.5, 1.0]];
        let mut d = vec![(true, false); 4];
        strip_down_cuts(&net, &quarters, &mut d, &mut [false; 4]);
        assert!(d[0].1 && d[1].1, "the top strip stayed whole: {d:?}");
    }

    /// In a quilt's patch the whole patch's own cut keeps only its halfway point on the middle row
    /// where the surface puts it: the quarter points of a nearly straight middle row sit on the
    /// row's chord, as on any other row. (The reference's 16-bit coordinate renders of rippled
    /// split warps put them there, within 0.01 px.)
    #[test]
    fn a_quilt_patch_middle_row_drops_its_quarter_points_onto_the_chord() {
        // Twisted across the rows (so the whole patch splits down) and waved along them, the wave
        // strong at the top and bottom (so the halves split across) and slight on the middle row.
        let pi = std::f64::consts::PI;
        let at = |u: f64, v: f64| {
            let wave = 8.0 * (2.0 * pi * u).sin() * (0.05 + 4.0 * (v - 0.5) * (v - 0.5));
            [64.0 * u, 64.0 * v + 16.0 * u * v + wave]
        };
        let m = quilt_patch_mesh(&at, Seams::default()).expect("finite");
        let pos_of = |u: f64, v: f64| m.uv.iter().position(|p| (p[0] - u).abs() < 1e-12 && (p[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let (Some(q), Some(h)) = (pos_of(0.25, 0.5), pos_of(0.5, 0.5)) else {
            panic!("the middle row wasn't cut at its quarters: {:?}", m.uv);
        };
        let s = at(0.25, 0.5);
        assert!((q[0] - s[0]).hypot(q[1] - s[1]) > 0.05, "the quarter point stayed on the surface: {q:?}");
        let (a, b) = (at(0.0, 0.5), at(0.5, 0.5));
        let cross = ((b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])).abs() / (b[0] - a[0]).hypot(b[1] - a[1]);
        assert!(cross < 1e-9, "the quarter point is off the chord by {cross}");
        assert_eq!(h, at(0.5, 0.5), "the halfway point moved");
    }

    /// A quilt patch measures its cells' rows by all the rows of their control nets: a bulge whose
    /// top and bottom rows are straight still splits across.
    #[test]
    fn a_quilt_patch_measures_its_inner_control_rows() {
        let quilt = quilt_rules();
        let bulge = |u: f64, v: f64| [128.0 * u, 128.0 * v + 96.0 * u * (1.0 - u) * v * (1.0 - v)];
        let across = |rules: &Rules| {
            let (_, log) = cells(&cubic_net(&bulge), true, rules);
            log.iter().any(|s| s.across && s.cell[1] - s.cell[0] > 1.0 - 1e-9 && s.cell[3] - s.cell[2] > 1.0 - 1e-9)
        };
        assert!(across(&quilt), "the inner rows' bend didn't split the patch across");
        assert!(!across(&Rules { twist_spreads: false, ..quilt }), "straight outer rows split a one-patch mesh across");
    }

    /// And its columns by every column of its control net: a bulge sideways whose left and right
    /// columns are straight and whose twist stays under WHOLE_PATCH_TWIST_TOL still splits down.
    #[test]
    fn a_quilt_patch_measures_its_inner_control_columns() {
        let bulge = |u: f64, v: f64| [128.0 * u + 24.0 * u * (1.0 - u) * v * (1.0 - v), 128.0 * v];
        let net = cubic_net(&bulge);
        let sides = need(&net.col(0.0)).max(need(&net.col(1.0)));
        let du = [net.at(3, 0)[0] - net.at(0, 0)[0], net.at(3, 0)[1] - net.at(0, 0)[1]];
        let dv = [net.at(0, 3)[0] - net.at(0, 0)[0], net.at(0, 3)[1] - net.at(0, 0)[1]];
        assert!(
            sides < 1e-9 && net.twist_across_rows(du, dv, 2.0, usize::MAX) < WHOLE_PATCH_TWIST_TOL,
            "{sides} {}",
            net.twist_across_rows(du, dv, 2.0, usize::MAX)
        );
        let (_, log) = cells(&net, true, &quilt_rules());
        assert!(
            log.iter().any(|s| s.down && s.cell[1] - s.cell[0] > 1.0 - 1e-9 && s.cell[3] - s.cell[2] > 1.0 - 1e-9),
            "the inner columns' bend didn't split the patch down"
        );
    }

    /// A full mesh starts as 2 × 2 cells, so only its halfway lines stay on the surface: a point at
    /// a quarter the row didn't need sits on the row's chord, also between two cells that split
    /// across for their rows' curvature (that exception is an envelope's). Measured on a ripple
    /// (control points moved by (6 sin 3πv, 5 sin 2π(u + v)) px over a 128 px box): the reference
    /// puts (¼, ½) 0.15 px off the surface, on the chord from (0, ½) to (½, ½).
    #[test]
    fn a_full_mesh_puts_quarter_points_on_the_row_chord() {
        let pi = std::f64::consts::PI;
        let mut p = [[0.0f64; 2]; 16];
        for (k, q) in p.iter_mut().enumerate() {
            let (u, v) = ((k % 4) as f64 / 3.0, (k / 4) as f64 / 3.0);
            *q = [128.0 * u + 6.0 * (3.0 * pi * v).sin(), 128.0 * v + 5.0 * (2.0 * pi * (u + v)).sin()];
        }
        let at = bicubic(p);
        let m = envelope_mesh(&at, true).expect("finite");
        let find = |u: f64, v: f64| m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let (a, c, mid) = (find(0.0, 0.5).expect("left"), find(0.5, 0.5).expect("middle"), find(0.25, 0.5).expect("quarter"));
        let chord = onto(a, c, at(0.25, 0.5));
        assert!((mid[0] - chord[0]).abs() < 1e-9 && (mid[1] - chord[1]).abs() < 1e-9, "{mid:?} vs chord {chord:?}");
        let surf = at(0.25, 0.5);
        assert!((surf[1] - mid[1]).abs() > 0.1, "the chord is off the surface here");
    }

    /// A point inside a bigger cell's edge is dropped onto that edge from where its own cells put
    /// it, keeping its place along the edge: Shell Lower 50 (a full mesh) at (⅛, ¼), under the
    /// whole cell u 0..¼. The reference's point there lies on the chord, slid 0.285 px along it.
    #[test]
    fn a_point_on_a_bigger_cells_edge_keeps_its_place_along_it() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = Warp::preset(WarpStyle::ShellLower, 50.0, b);
        let at = |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        };
        let m = envelope_mesh(&at, true).expect("finite");
        let find = |u: f64, v: f64| m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let (a, c, p) = (find(0.0, 0.25).expect("left end"), find(0.25, 0.25).expect("right end"), find(0.125, 0.25).expect("middle"));
        let d = [c[0] - a[0], c[1] - a[1]];
        let len = d[0].hypot(d[1]);
        let along = |p: P| ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len;
        let off = ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]) / len;
        assert!(off.abs() < 1e-9, "{p:?} is off the edge by {off}");
        let s = at(0.125, 0.25);
        assert!((along(p) - along(s)).abs() < 1e-9, "at {} along, its surface point at {}", along(p), along(s));
        assert!(along(p) - len / 2.0 > 0.2, "slid only {} from the edge's middle", along(p) - len / 2.0);
    }

    /// A point inside a bigger cell's edge starts from the surface even where its row would put it
    /// on a chord: Shell Upper −50 at (⅛, ½), where the reference's point slides 0.356 px along.
    #[test]
    fn a_point_on_a_bigger_cells_edge_starts_from_the_surface() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = Warp::preset(WarpStyle::ShellUpper, -50.0, b);
        let at = |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        };
        let m = envelope_mesh(&at, true).expect("finite");
        let find = |u: f64, v: f64| m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let (a, c, p) = (find(0.0, 0.5).expect("left end"), find(0.25, 0.5).expect("right end"), find(0.125, 0.5).expect("middle"));
        let d = [c[0] - a[0], c[1] - a[1]];
        let len = d[0].hypot(d[1]);
        let along = |p: P| ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len;
        let off = ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]) / len;
        assert!(off.abs() < 1e-9, "{p:?} is off the edge by {off}");
        assert!((along(p) - along(at(0.125, 0.5))).abs() < 1e-9, "at {} along", along(p));
        assert!(along(p) - len / 2.0 > 0.3, "slid only {} from the edge's middle", along(p) - len / 2.0);
    }

    /// A row point its row didn't need is dropped onto the row's chord from its surface point, so
    /// it keeps its place along the chord: Arc Lower 50 at (⅛, ¼), which the reference slides
    /// 0.125 px along the chord (no seam: both sides' cells have it there).
    #[test]
    fn a_row_point_keeps_its_place_along_the_chord() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = Warp::preset(WarpStyle::ArcLower, 50.0, b);
        let at = |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        };
        let m = envelope_mesh(&at, false).expect("finite");
        let find = |u: f64, v: f64| m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let (a, c, p) = (find(0.0, 0.25).expect("left end"), find(0.25, 0.25).expect("right end"), find(0.125, 0.25).expect("middle"));
        let d = [c[0] - a[0], c[1] - a[1]];
        let len = d[0].hypot(d[1]);
        let along = |p: P| ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len;
        let off = ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]) / len;
        assert!(off.abs() < 1e-9, "{p:?} is off the chord by {off}");
        assert!((along(p) - along(at(0.125, 0.25))).abs() < 1e-9, "at {} along", along(p));
        assert!((along(p) - len / 2.0).abs() > 0.08, "slid only {} from the chord's middle", along(p) - len / 2.0);
    }

    /// An envelope's twist split across counts the cell's left column of quads only: Arc Lower 50's
    /// top corner cells are mirror images, but the reference splits the left one only, whose left
    /// quads are stretched unevenly by 1.11 px (the right one's by 0.55).
    #[test]
    fn an_envelope_twist_split_across_counts_the_left_column() {
        use photocraft_geom::warp::WarpStyle;
        let at = in_perspective(WarpStyle::ArcLower, 50.0, 0.0, 0.0);
        let net = net_of(&at, false);
        assert_eq!((net.nu, net.nv), (4, 2));
        let rules = one_patch_rules(&net, false);
        let (_, log) = cells(&net, true, &rules);
        let split = |log: &[Split], c: Cell| log.iter().any(|s| s.cell == c && s.across);
        let (left, right) = ([0.0, 0.25, 0.0, 0.125], [0.75, 1.0, 0.0, 0.125]);
        assert!(net.sub(left).twist_left() > TWIST_TOL + EPS && net.sub(right).twist_left() < TWIST_TOL);
        assert!(split(&log, left), "the top-left corner cell splits across");
        assert!(!split(&log, right), "its mirror image on the right stays whole");
        let (_, sym) = cells(&net, true, &Rules { twist_by_side: false, ..rules });
        assert!(split(&sym, right), "the whole net's twist would split it");
    }

    /// An envelope's twist is how unevenly a control quad is stretched: the larger difference in
    /// length between two opposite sides of it, counted on the cell's left column of quads for a
    /// split across and on its top row for a split down. A parallelogram has none, however far
    /// it leans.
    #[test]
    fn an_envelopes_twist_compares_opposite_sides() {
        let quad = |d: P| Net { nu: 2, nv: 2, p: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], d] };
        // Top 10, bottom √185, left 10, right √205.
        let n = quad([13.0, 14.0]);
        assert!((n.side_twist(0, 0) - (205f64.sqrt() - 10.0)).abs() < 1e-12, "{}", n.side_twist(0, 0));
        assert!((n.twist() - 5.0).abs() < 1e-12, "the whole net's measure is another: {}", n.twist());
        let leaning = Net { nu: 2, nv: 2, p: vec![[0.0, 0.0], [10.0, 0.0], [7.0, 10.0], [17.0, 10.0]] };
        assert_eq!(leaning.side_twist(0, 0), 0.0);
        assert_eq!(quad([f64::NAN, 14.0]).side_twist(0, 0), 0.0, "a point that isn't finite counts as no twist");
        // A 3 × 3 grid of 10 px quads with one corner point pulled out.
        let grid =
            |k: usize, to: P| Net { nu: 3, nv: 3, p: (0..9).map(|i| if i == k { to } else { [10.0 * (i % 3) as f64, 10.0 * (i / 3) as f64] }).collect() };
        let bottom_right = grid(8, [25.0, 26.0]);
        assert!(bottom_right.twist() > 1.0);
        assert_eq!((bottom_right.twist_left(), bottom_right.twist_top()), (0.0, 0.0), "neither the left column nor the top row has that quad");
        let bottom_left = grid(6, [-3.0, 24.0]);
        assert!(bottom_left.twist_left() > 1.0 && bottom_left.twist_top() == 0.0);
        let top_right = grid(2, [24.0, -3.0]);
        assert!(top_right.twist_top() > 1.0 && top_right.twist_left() == 0.0);
    }

    /// A 3 × 3 envelope's wide cell splits across where the legs of its left column of control
    /// points are longer than the second column's by more than 1 px, in the cell itself or in the
    /// other half of the cell it was cut from. Inflate −46.2 splits its four left cells between
    /// v ¼ and ¾ as the reference does: the outer two measure 1.0002, the inner two 0.94 and split
    /// with them. Their mirror images on the right stay whole, and so does everything at −46 and
    /// at +48 (there the legs beside are the longer ones). Lengths don't change when the envelope
    /// is turned upside down, so the same cells split then.
    #[test]
    fn a_quadratic_envelope_splits_its_left_cells_for_longer_left_legs() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let inflate = |bend: f64, flip: bool| {
            let w = Warp::preset(WarpStyle::Inflate, bend, b);
            move |u: f64, v: f64| {
                let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * if flip { 1.0 - v } else { v });
                [x, if flip { 320.0 - y } else { y }]
            }
        };
        let cut = |bend: f64, flip: bool| {
            let at = inflate(bend, flip);
            let net = net_of(&at, false);
            assert_eq!((net.nu, net.nv), (3, 3));
            let (leaves, log) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
            let across: Vec<Cell> = log.iter().filter(|s| s.across && s.cell[1] - s.cell[0] > s.cell[3] - s.cell[2] + EPS).map(|s| s.cell).collect();
            (net, leaves.len(), across)
        };
        let left: Vec<Cell> = [0.25, 0.375, 0.5, 0.625].iter().map(|&v| [0.0, 0.25, v, v + 0.125]).collect();
        for flip in [false, true] {
            let (net, leaves, across) = cut(-46.2, flip);
            assert_eq!(across, left, "the four left cells split across, and no cell on the right");
            assert_eq!(leaves, 52);
            let legs = |v: f64| net.sub([0.0, 0.25, v, v + 0.125]).left_legs_longer();
            assert!(legs(0.25) > TWIST_TOL + EPS && legs(0.625) > TWIST_TOL + EPS, "the outer cells' own legs: {} {}", legs(0.25), legs(0.625));
            assert!(legs(0.375) < TWIST_TOL && legs(0.5) < TWIST_TOL, "the inner cells split for their other halves: {} {}", legs(0.375), legs(0.5));
        }
        let (_, leaves, across) = cut(-46.0, false);
        assert!(across.is_empty() && leaves == 48, "shorter legs split nothing: {across:?}");
        let (_, _, across) = cut(-48.0, false);
        assert_eq!(across, left);
        let (_, leaves, across) = cut(48.0, false);
        assert!(across.is_empty() && leaves == 48, "bent the other way nothing splits: {across:?}");
    }

    /// The nine control points the reference stores for Inflate −50 over the 128 px square at
    /// (96, 96) in `d` % of horizontal perspective.
    fn inflate_in_perspective(d: f64) -> [P; 9] {
        let (e, m) = (0.64 * d, 0.32 * d);
        [
            [96.0, 96.0 + e],
            [160.0, 128.0],
            [224.0, 96.0 - e],
            [128.0 - m, 160.0],
            [160.0, 160.0],
            [192.0 - m, 160.0],
            [96.0, 224.0 - e],
            [160.0, 192.0],
            [224.0, 224.0 + e],
        ]
    }

    /// The surface of nine control points (three rows of three).
    fn nine_points(c: [P; 9]) -> impl Fn(f64, f64) -> P {
        let w = |t: f64| [(1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t];
        move |u: f64, v: f64| {
            let (wu, wv) = (w(u), w(v));
            let mut o = [0.0; 2];
            for (row, b) in c.chunks(3).zip(wv) {
                for (q, a) in row.iter().zip(wu) {
                    o[0] += a * b * q[0];
                    o[1] += a * b * q[1];
                }
            }
            o
        }
    }

    /// Perspective that shortens an envelope's left side shortens its left legs with it: Inflate
    /// −50 (1.09 px longer when plain) still splits its left cells in 2 % perspective, and stays
    /// whole there in 8 %, as in the reference. Where exactly it changes over this measure gets
    /// slightly wrong: between 4.2 % and 4.6 %, and the reference between 4 % and 4.2 %.
    #[test]
    fn a_quadratic_envelopes_left_legs_shorten_in_perspective() {
        let cut = |d: f64| {
            let at = nine_points(inflate_in_perspective(d));
            let net = net_of(&at, false);
            assert_eq!((net.nu, net.nv), (3, 3));
            let (leaves, log) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
            let legs = net.sub([0.0, 0.25, 0.25, 0.375]).left_legs_longer();
            (leaves.len(), log.iter().any(|s| s.cell == [0.0, 0.25, 0.25, 0.375] && s.across), legs)
        };
        let (leaves, split, legs) = cut(0.0);
        assert!(leaves == 52 && split && (legs - 1.0873).abs() < 1e-3, "plain: {leaves} cells, legs {legs}");
        let (leaves, split, legs) = cut(2.0);
        assert!(leaves == 52 && split && legs > TWIST_TOL + EPS, "2 %: {leaves} cells, legs {legs}");
        let (leaves, split, legs) = cut(8.0);
        assert!(leaves == 48 && !split && legs < TWIST_TOL, "8 %: {leaves} cells, legs {legs}");
        assert!(cut(4.2).1 && !cut(4.6).1, "the measure's own change-over");
    }

    /// The 128 px square with its top-left corner pulled out to (48, 48), as a full mesh.
    fn corner_out() -> impl Fn(f64, f64) -> P {
        corner_out_by(48.0)
    }

    /// The 128 px square at (96, 96) with its top-left corner pulled out diagonally by `d` px.
    fn corner_out_by(d: f64) -> impl Fn(f64, f64) -> P {
        let mut p = [[0.0f64; 2]; 16];
        for (k, q) in p.iter_mut().enumerate() {
            *q = [96.0 + 128.0 * (k % 4) as f64 / 3.0, 96.0 + 128.0 * (k / 4) as f64 / 3.0];
        }
        p[0] = [96.0 - d, 96.0 - d];
        bicubic(p)
    }

    /// In a full mesh, a cell that splits across for its rows splits down for its twist only where
    /// its top edge twists past TWIST_TOL: the reference keeps the cell at (½..¾, 0..¼) of a corner
    /// pulled out 48 px a quarter tall, and splits it pulled 55 px.
    #[test]
    fn a_full_mesh_cell_split_across_ignores_its_twist() {
        let at = corner_out();
        let net = net_of(&at, true);
        let full = one_patch_rules(&net, true);
        let (leaves, _) = cells(&net, true, &full);
        let tall = |ls: &[Cell]| ls.iter().any(|c| c[0] >= 0.5 - 1e-9 && c[1] <= 0.75 + 1e-9 && c[2].abs() < 1e-9 && (c[3] - 0.25).abs() < 1e-9);
        assert!(tall(&leaves), "a quarter-tall cell stays at the top, right of the middle");
        // Pulled 55 px instead of 48, the same cell's twist is 1.22: past the limit, it splits down.
        let far = corner_out_by(55.0);
        let (leaves, _) = cells(&net_of(&far, true), true, &full);
        assert!(!tall(&leaves), "a twist of 1.22 splits the cell down");
    }

    /// A full mesh's leaf stays whole next to smaller cells: its quads are its leaves, and a point
    /// of a smaller neighbour on its side sits on that side's chord, so no crack opens.
    #[test]
    fn a_full_mesh_leaf_stays_whole_beside_smaller_cells() {
        let at = corner_out();
        let m = envelope_mesh(&at, true).expect("finite");
        let net = net_of(&at, true);
        let full = one_patch_rules(&net, true);
        let (leaves, _) = cells(&net, true, &full);
        assert_eq!(m.quads.len(), leaves.len(), "no leaf is cut across");
        // Some point lies inside a leaf's side; it is on the chord between that side's ends.
        let find = |u: f64, v: f64| m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).map(|k| m.pos[k]);
        let mut checked = 0;
        for c in &leaves {
            for u in [c[0], c[1]] {
                for (k, q) in m.uv.iter().enumerate() {
                    if (q[0] - u).abs() < 1e-12 && q[1] > c[2] + 1e-9 && q[1] < c[3] - 1e-9 {
                        let (a, b) = (find(u, c[2]).expect("top"), find(u, c[3]).expect("bottom"));
                        let (d, e) = ([b[0] - a[0], b[1] - a[1]], [m.pos[k][0] - a[0], m.pos[k][1] - a[1]]);
                        let off = (e[0] * d[1] - e[1] * d[0]) / d[0].hypot(d[1]);
                        let s = (e[0] * d[0] + e[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]);
                        assert!(off.abs() < 1e-9 && (0.0..=1.0).contains(&s), "{q:?} off its side's chord ({off}, {s})");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "the case has points on bigger leaves' sides");
    }
    /// A preset bent sideways (stored with more control points down than across) over the 128 px
    /// square at (96, 96).
    fn sideways(style: photocraft_geom::warp::WarpStyle, bend: f64) -> impl Fn(f64, f64) -> P {
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = photocraft_geom::warp::Warp { vertical: true, ..photocraft_geom::warp::Warp::preset(style, bend, b) };
        move |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        }
    }

    /// A tall envelope is cut in place: sideways Arc 50 is 8 cells across by 16 down in the
    /// reference, as its columns' curvature asks. Cut as its mirror image about the diagonal (by
    /// the rules for rows, then turned back) it came out in sixteenths both ways.
    #[test]
    fn a_tall_envelope_is_cut_in_place() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::Arc, 50.0);
        let net = net_of(&at, false);
        assert_eq!((net.nu, net.nv), (2, 4));
        let (leaves, _) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
        assert_eq!(leaves.len(), 128);
        assert!(leaves.iter().all(|c| (c[1] - c[0] - 0.125).abs() < EPS && (c[3] - c[2] - 0.0625).abs() < EPS), "every cell is an eighth by a sixteenth");
        let m = envelope_mesh(&at, false).expect("finite");
        for (uv, p) in m.uv.iter().zip(&m.pos) {
            let s = at(uv[0], uv[1]);
            assert!((p[0] - s[0]).hypot(p[1] - s[1]) < 1.0, "the vertex at {uv:?} belongs to its own source point");
        }
    }

    /// An envelope's twist split down counts the cell's top row of quads only: sideways Bulge 50's
    /// bottom cells (an eighth by an eighth) stay whole in the reference, though their whole nets
    /// twist enough to split.
    #[test]
    fn an_envelope_twist_split_down_counts_the_top_row() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::Bulge, 50.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        let (leaves, _) = cells(&net, twisty_net(&net), &rules);
        let c = [0.25, 0.375, 0.875, 1.0];
        assert!(leaves.contains(&c), "the bottom cell stays whole");
        let sub = net.sub(c);
        assert!(sub.twist() > TWIST_TOL + EPS && sub.twist_top() < TWIST_TOL, "whole {} top {}", sub.twist(), sub.twist_top());
        let (whole, _) = cells(&net, twisty_net(&net), &Rules { twist_by_side: false, ..rules });
        assert!(whole.contains(&[0.25, 0.375, 0.875, 0.9375]), "the whole net's twist would split it down");
    }

    /// A cell split down for its columns' curvature takes its sibling along only when it is
    /// square: sideways Fish 50's cell at u 0..⅛, v ⅛..3⁄16 splits down, and the reference keeps
    /// the cell beside it whole.
    #[test]
    fn only_a_square_cell_takes_its_sibling_down() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::Fish, 50.0);
        let net = net_of(&at, false);
        let (leaves, log) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
        assert!(log.iter().any(|s| s.cell == [0.0, 0.125, 0.125, 0.1875] && s.down), "the outer cell splits down for its curvature");
        assert!(leaves.contains(&[0.125, 0.25, 0.125, 0.1875]), "its sibling stays whole");
    }

    /// A tall envelope's square cell splits across as well when its whole net twists by more than
    /// 2.0: sideways Arc Lower 50's first corner cell does (twist left and top alone don't ask for
    /// it), sideways Fish 50's cells an eighth wide don't, though they twist by more than 1.2.
    #[test]
    fn a_tall_envelopes_square_cell_splits_across_for_its_twist() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::ArcLower, 50.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        assert_eq!(rules.square_twist, Some(TALL_SQUARE_TWIST_TOL));
        let (leaves, _) = cells(&net, twisty_net(&net), &rules);
        assert!(leaves.contains(&[0.0, 0.125, 0.125, 0.25]), "the corner cell is split across");
        let (without, _) = cells(&net, twisty_net(&net), &Rules { square_twist: None, ..rules });
        assert!(without.contains(&[0.0, 0.25, 0.125, 0.25]), "without the rule it is split down only");

        let at = sideways(WarpStyle::Fish, 50.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        let (leaves, _) = cells(&net, twisty_net(&net), &rules);
        assert!(leaves.contains(&[0.0, 0.125, 0.1875, 0.25]), "a twist under 2.0 leaves the cell an eighth wide");
        let (low, _) = cells(&net, twisty_net(&net), &Rules { square_twist: Some(1.2), ..rules });
        assert!(low.contains(&[0.0, 0.0625, 0.1875, 0.25]), "a tolerance of 1.2 would split it");
        // A wide envelope has no such rule.
        let wide = net_of(&|u: f64, v: f64| at(v, u), false);
        assert_eq!(one_patch_rules(&wide, false).square_twist, None);
    }

    /// In a tall envelope the chord rule holds for every column: sideways Arc 50's middle column
    /// is straight enough between eighths, so its sixteenth points sit on the chords between the
    /// eighth points (0.7 px off the surface), and the eighth points stay on the surface.
    #[test]
    fn a_tall_envelopes_column_points_follow_the_chord_rule() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::Arc, 50.0);
        let m = envelope_mesh(&at, false).expect("finite");
        let pos = |u: f64, v: f64| {
            m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).and_then(|i| m.pos.get(i).copied()).expect("a vertex")
        };
        let (a, p, b) = (pos(0.5, 0.25), pos(0.5, 0.3125), pos(0.5, 0.375));
        let s = at(0.5, 0.3125);
        assert!((p[0] - s[0]).hypot(p[1] - s[1]) > 0.5, "off the surface: {p:?} vs {s:?}");
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        assert!(cross.abs() / (b[0] - a[0]).hypot(b[1] - a[1]) < 1e-9, "on the chord between the eighth points");
        for v in [0.25, 0.375] {
            let (p, s) = (pos(0.5, v), at(0.5, v));
            assert!((p[0] - s[0]).hypot(p[1] - s[1]) < 1e-9, "the eighth point at {v} stays on the surface");
        }
    }

    /// A tall envelope's cells stay whole beside smaller neighbours (sideways Fish 50: one quad
    /// per cell); a wide envelope's are cut across at their neighbours' points.
    #[test]
    fn a_tall_envelopes_cells_stay_whole_beside_smaller_ones() {
        use photocraft_geom::warp::WarpStyle;
        let at = sideways(WarpStyle::Fish, 50.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        let (leaves, log) = cells(&net, twisty_net(&net), &rules);
        let m = envelope_mesh(&at, false).expect("finite");
        assert_eq!(m.quads.len(), leaves.len());
        let cut = triangulate(&net, &at, &leaves, &log, &Rules { cut_sides: true, ..rules });
        assert!(cut.quads.len() > leaves.len(), "the same cells cut at their sides make more quads");
    }

    /// A wide preset pulled into perspective (horizontal distortion, percent), which its stored
    /// control points carry.
    fn distorted(style: photocraft_geom::warp::WarpStyle, bend: f64, distort: f64) -> impl Fn(f64, f64) -> P {
        in_perspective(style, bend, distort, 0.0)
    }

    /// A wide preset over the 128 px square at (96, 96), pulled into perspective both ways
    /// (horizontal and vertical distortion, percent).
    fn in_perspective(style: photocraft_geom::warp::WarpStyle, bend: f64, h: f64, v: f64) -> impl Fn(f64, f64) -> P {
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = photocraft_geom::warp::Warp { h_distort: h, v_distort: v, ..photocraft_geom::warp::Warp::preset(style, bend, b) };
        move |u: f64, t: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * t);
            [x, y]
        }
    }

    fn vertex(m: &EnvelopeMesh, u: f64, v: f64) -> P {
        m.uv.iter().position(|q| (q[0] - u).abs() < 1e-12 && (q[1] - v).abs() < 1e-12).and_then(|i| m.pos.get(i).copied()).expect("a vertex")
    }

    fn off_line(a: P, b: P, p: P) -> f64 {
        ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / (b[0] - a[0]).hypot(b[1] - a[1])
    }

    /// An envelope's quarter points on its top and bottom edges follow the rows' chord rule: Arc
    /// Lower 60 in 20 % perspective has a gently S-shaped top edge, and the reference puts (¼, 0)
    /// and (¾, 0) on the chords from the corners to the middle, 0.4 px off the surface.
    #[test]
    fn an_envelopes_edge_quarter_points_follow_the_chord_rule() {
        use photocraft_geom::warp::WarpStyle;
        let at = distorted(WarpStyle::ArcLower, 60.0, 20.0);
        let m = envelope_mesh(&at, false).expect("finite");
        for (a, u, b) in [(0.0, 0.25, 0.5), (0.5, 0.75, 1.0)] {
            let (p, s) = (vertex(&m, u, 0.0), at(u, 0.0));
            assert!((p[0] - s[0]).hypot(p[1] - s[1]) > 0.3, "({u}, 0) is off the surface: {p:?} vs {s:?}");
            assert!(off_line(vertex(&m, a, 0.0), vertex(&m, b, 0.0), p) < 1e-9, "({u}, 0) is on the chord");
            for end in [a, b] {
                let (p, s) = (vertex(&m, end, 0.0), at(end, 0.0));
                assert!((p[0] - s[0]).hypot(p[1] - s[1]) < 1e-9, "({end}, 0) stays on the surface");
            }
        }
    }

    /// On the rows at ¼ and ¾ an envelope's quarter points stay where they are, however straight
    /// the rows: Arc Lower 20's row at ¼ asks for no split between the side and the middle, and
    /// the reference's quarter points there are on the surface all the same (on the chords they
    /// would be up to 0.5 px off).
    #[test]
    fn an_envelopes_inner_quarter_points_stay_on_the_surface() {
        use photocraft_geom::warp::{Warp, WarpStyle};
        let b = [96.0, 96.0, 224.0, 224.0];
        let w = Warp::preset(WarpStyle::ArcLower, 20.0, b);
        let at = move |u: f64, v: f64| {
            let (x, y) = w.map(b[0] + 128.0 * u, b[1] + 128.0 * v);
            [x, y]
        };
        let net = net_of(&at, false);
        let m = envelope_mesh(&at, false).expect("finite");
        let mut seen = 0;
        for (uv, p) in m.uv.iter().zip(&m.pos) {
            let quarter = (uv[0] - 0.25).abs() < 1e-12 || (uv[0] - 0.75).abs() < 1e-12;
            let (a, c) = if uv[0] < 0.5 { (0.0, 0.5) } else { (0.5, 1.0) };
            if !quarter || level(uv[1]) < 2 || need(&segment(net.row(uv[1]), a, c)) >= 1.0 - EPS {
                continue;
            }
            let s = at(uv[0], uv[1]);
            assert!((p[0] - s[0]).hypot(p[1] - s[1]) < 1e-9, "the quarter point at {uv:?} stays on the surface");
            let chord = onto(at(a, uv[1]), at(c, uv[1]), s);
            if (chord[0] - s[0]).hypot(chord[1] - s[1]) > 0.1 {
                seen += 1;
            }
        }
        assert!(seen > 0, "some of those rows' chords are well off the surface");
    }

    /// Inside an envelope that isn't tall, a column's new points go onto its chord only where the
    /// column is nearly straight. Inflate −50 in 12 % perspective, by the nine control points the
    /// reference stores for it: the middle column bows by a tenth of a pixel between quarter
    /// points, and its eighth points sit on the chords between them (0.04 px off the surface); the
    /// column at ¼ curves more, though less than a split's worth, and its eighth points stay on
    /// the surface.
    #[test]
    fn an_envelopes_nearly_straight_column_takes_the_chord_rule() {
        // inflate_in_perspective(12.0), as stored.
        let c: [P; 9] =
            [[96.0, 103.68], [160.0, 128.0], [224.0, 88.32], [124.16, 160.0], [160.0, 160.0], [188.16, 160.0], [96.0, 216.32], [160.0, 192.0], [224.0, 231.68]];
        let at = nine_points(c);
        let net = net_of(&at, false);
        assert_eq!((net.nu, net.nv), (3, 3));
        let m = envelope_mesh(&at, false).expect("finite");
        for v in [0.125, 0.375, 0.625, 0.875] {
            let (p, s) = (vertex(&m, 0.5, v), at(0.5, v));
            let off = (p[0] - s[0]).hypot(p[1] - s[1]);
            assert!(off > 0.02 && off < 0.2, "(½, {v}) is just off the surface: {off}");
            assert!(off_line(vertex(&m, 0.5, v - 0.125), vertex(&m, 0.5, v + 0.125), p) < 1e-9, "(½, {v}) is on the chord between the quarter points");
            let straight = need(&segment(net.col(0.25), v - 0.125, v + 0.125));
            assert!(straight > FLAT_COLUMN_TOL && straight < 1.0, "the column at ¼ asks for {straight} of a split");
            let (p, s) = (vertex(&m, 0.25, v), at(0.25, v));
            assert!((p[0] - s[0]).hypot(p[1] - s[1]) < 1e-9, "(¼, {v}) stays on the surface");
        }
    }
    /// An envelope's middle row follows the rows' chord rule like its top and bottom edges: Arc
    /// Lower 10's asks for 0.84 of a split between the side and the middle, and the reference puts
    /// its quarter points on those chords, 0.63 px off the surface. The row at ¼ asks for less and
    /// keeps its quarter points on the surface.
    #[test]
    fn an_envelopes_middle_row_quarter_points_follow_the_chord_rule() {
        use photocraft_geom::warp::WarpStyle;
        let at = in_perspective(WarpStyle::ArcLower, 10.0, 0.0, 0.0);
        let net = net_of(&at, false);
        let m = envelope_mesh(&at, false).expect("finite");
        let off = |u: f64, v: f64| {
            let (p, s) = (vertex(&m, u, v), at(u, v));
            (p[0] - s[0]).hypot(p[1] - s[1])
        };
        for (a, u, b) in [(0.0, 0.25, 0.5), (0.5, 0.75, 1.0)] {
            let asks = need(&segment(net.row(0.5), a, b));
            assert!(asks > 0.8 && asks < 0.9, "the middle row asks for {asks} of a split");
            assert!(off(u, 0.5) > 0.6, "({u}, ½) is off the surface: {}", off(u, 0.5));
            assert!(off_line(vertex(&m, a, 0.5), vertex(&m, b, 0.5), vertex(&m, u, 0.5)) < 1e-9, "({u}, ½) is on the chord");
            assert!(off(a, 0.5) < 1e-9 && off(b, 0.5) < 1e-9, "the chord's ends stay on the surface");
            assert!(need(&segment(net.row(0.25), a, b)) < 0.5);
            assert!(off(u, 0.25) < 1e-9, "({u}, ¼) stays on the surface");
        }
    }

    /// An envelope's first cells split down only where their columns ask for it: Arch 40 is cut
    /// into eighths across and stays in quarters down in the reference (32 cells). A full mesh's
    /// first cell (a half of the mesh each way) splits down too where it splits across twice: split
    /// across once it stays whole down (Arch 12, quarters by halves), and the halves of it that split
    /// across again split down with that (Arch 40, eighths by quarters).
    #[test]
    fn an_envelopes_first_cells_split_down_only_for_their_columns() {
        use photocraft_geom::warp::WarpStyle;
        let at = in_perspective(WarpStyle::Arch, 40.0, 0.0, 0.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        assert!(!rules.force_down);
        let (leaves, _) = cells(&net, twisty_net(&net), &rules);
        assert_eq!(leaves.len(), 32);
        assert!(leaves.iter().all(|c| (c[1] - c[0] - 0.125).abs() < EPS && (c[3] - c[2] - 0.25).abs() < EPS), "every cell is an eighth by a quarter");
        let sized = |cells: &[Cell], w: f64, h: f64| cells.iter().all(|c| (c[1] - c[0] - w).abs() < EPS && (c[3] - c[2] - h).abs() < EPS);
        let full = net_of(&at, true);
        let rules = one_patch_rules(&full, true);
        assert!(rules.force_down);
        let (twice, _) = cells(&full, twisty_net(&full), &rules);
        assert!(twice.len() == 32 && sized(&twice, 0.125, 0.25), "split across twice: eighths by quarters, {} cells", twice.len());
        let (unforced, _) = cells(&full, twisty_net(&full), &Rules { force_down: false, ..rules });
        assert!(unforced.len() == 16 && sized(&unforced, 0.125, 0.5), "left alone they would stay halves down, {} cells", unforced.len());
        let gentle = net_of(&in_perspective(WarpStyle::Arch, 12.0, 0.0, 0.0), true);
        let (once, _) = cells(&gentle, twisty_net(&gentle), &one_patch_rules(&gentle, true));
        assert!(once.len() == 8 && sized(&once, 0.25, 0.5), "split across once: quarters by halves, {} cells", once.len());
    }

    /// A full mesh over 128 px with its inner control point (i, j) moved by `d`.
    fn moved_point_net(i: usize, j: usize, d: P) -> Net {
        let mut p: Vec<P> = (0..16).map(|k| [96.0 + 128.0 * (k % 4) as f64 / 3.0, 96.0 + 128.0 * (k / 4) as f64 / 3.0]).collect();
        if let Some(q) = p.get_mut(j * 4 + i) {
            *q = [q[0] + d[0], q[1] + d[1]];
        }
        Net { nu: 4, nv: 4, p }
    }

    /// A one-patch full mesh's first split down carries along its row only, unless the cell's
    /// columns ask for 1.5 or more: with its top-left inner point moved 8 px across, the top cells
    /// split down and the bottom right cell stays a half tall (columns 1.12); moved 8 px down
    /// (columns 1.88), all four first cells split down.
    #[test]
    fn a_full_meshs_first_split_down_carries_along_its_row() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        assert!(rules.full_mesh());
        let halved = |leaves: &[Cell]| leaves.iter().filter(|c| c[0] > 0.5 - EPS && c[2] > 0.5 - EPS).all(|c| (c[3] - c[2] - 0.5).abs() < EPS);
        let (across, _) = cells(&moved_point_net(1, 1, [8.0, 0.0]), true, &rules);
        assert!(halved(&across), "moved across: the bottom right cell stays whole down");
        assert!(across.iter().any(|c| c[0] > 0.5 - EPS && c[2] < EPS && c[3] - c[2] < 0.5 - EPS), "the top right cell splits down with the top left");
        let (down, _) = cells(&moved_point_net(1, 1, [0.0, 8.0]), true, &rules);
        assert!(!halved(&down), "moved down: the split carries to the bottom right cell too");
    }

    /// A one-patch full mesh measures a cell by every column of its control net, not only by the
    /// surface's columns at its sides: with the top-left inner point moved 11 px down, the quarter
    /// cell at u ¼..½, v 0..¼ has sides asking 0.94 but an inner control column asking 1.05, and
    /// the reference cuts it (and the cell beside it) into eighths down; moved 8 px (0.76) it stays
    /// a quarter.
    #[test]
    fn a_full_mesh_cell_is_measured_by_its_whole_control_net() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        let eighths = |leaves: &[Cell]| leaves.iter().any(|c| c[0] > 0.25 - EPS && c[1] < 0.5 + EPS && c[3] < 0.25 + EPS && c[3] - c[2] < 0.25 - EPS);
        let (deep, _) = cells(&moved_point_net(1, 1, [0.0, 11.0]), true, &rules);
        assert!(eighths(&deep), "11 px: cut into eighths down");
        let (shallow, _) = cells(&moved_point_net(1, 1, [0.0, 8.0]), true, &rules);
        assert!(!eighths(&shallow), "8 px: stays a quarter");
    }

    /// A one-patch full mesh weighs the first inner point of a cell's side rows: with the top-right
    /// inner point moved 14 px right, the top-left first cell's rows lag at their first inner
    /// point and it splits across; with the top-left point moved 14 px right, the top-right cell's
    /// rows lag as much at their second point and it stays whole across.
    #[test]
    fn a_full_mesh_weighs_the_first_point_of_its_rows() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        let narrow = |leaves: &[Cell], u: f64| leaves.iter().any(|c| (c[0] - u).abs() < EPS && c[3] < 0.5 + EPS && c[1] - c[0] < 0.5 - EPS);
        let (left, _) = cells(&moved_point_net(2, 1, [14.0, 0.0]), true, &rules);
        assert!(narrow(&left, 0.0), "the cell left of the moved point splits across");
        let (right, _) = cells(&moved_point_net(1, 1, [14.0, 0.0]), true, &rules);
        assert!(!narrow(&right, 0.5), "the cell right of it stays whole across");
    }

    /// A one-patch full mesh's first cell on the mesh's left edge weighs its rows' first inner point
    /// in full, not at 0.75: with an inner point moved 11 px across, the top left quarter is cut
    /// across, whether the point is in the top or the bottom row; the mirror cells on the right
    /// edge stay whole.
    #[test]
    fn a_full_mesh_cell_on_the_left_edge_weighs_its_first_point_more() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        let leaf = |leaves: &[Cell], c: Cell| leaves.iter().any(|l| (0..4).all(|k| (l[k] - c[k]).abs() < EPS));
        let (left, _) = cells(&moved_point_net(2, 1, [11.0, 0.0]), true, &rules);
        assert!(leaf(&left, [0.0, 0.25, 0.0, 0.25]), "left edge: cut across");
        let (lower, _) = cells(&moved_point_net(2, 2, [11.0, 0.0]), true, &rules);
        assert!(leaf(&lower, [0.0, 0.25, 0.0, 0.25]), "point in the bottom row: cut across too");
        let (right, _) = cells(&moved_point_net(1, 1, [-11.0, 0.0]), true, &rules);
        assert!(leaf(&right, [0.5, 1.0, 0.0, 0.25]), "right edge: whole across");
    }

    /// A one-patch full mesh's cell split across splits down too where its top edge twists past
    /// 1 px: with an inner control point of the top row moved 18 px across, the top right quarter
    /// cell (top edge 1.125) is cut into eighths down; moved 14 px (0.875) it stays a quarter tall.
    /// The mirror cell, twisted as much along its bottom edge, stays whole down. The cut counts as
    /// one for twist: the cell beside it in its row (u ½..¾) is cut down too.
    #[test]
    fn a_full_mesh_cell_split_across_splits_down_past_its_top_edge_twist() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        let leaf = |leaves: &[Cell], c: Cell| leaves.iter().any(|l| (0..4).all(|k| (l[k] - c[k]).abs() < EPS));
        let (far, _) = cells(&moved_point_net(2, 1, [18.0, 0.0]), true, &rules);
        assert!(leaf(&far, [0.75, 0.875, 0.0, 0.125]), "top edge 1.125: cut down");
        assert!(leaf(&far, [0.5, 0.75, 0.0, 0.125]), "and its sibling in the row");
        let (near, _) = cells(&moved_point_net(2, 1, [14.0, 0.0]), true, &rules);
        assert!(leaf(&near, [0.75, 0.875, 0.0, 0.25]), "top edge 0.875: whole down");
        let (mirror, _) = cells(&moved_point_net(1, 2, [18.0, 0.0]), true, &rules);
        assert!(leaf(&mirror, [0.0, 0.125, 0.75, 1.0]), "bottom edge 1.125: whole down");
    }

    /// A one-patch full mesh's cell twisted past 2 px splits both ways: with the top-left inner
    /// point moved 18 px across, the top right cell (twist 2.25, its rows asking 0.84) splits across
    /// as well as down, so the lower half of it is cut across too; moved 14 px (twist 1.75) it
    /// splits down only and that half stays whole across.
    #[test]
    fn a_full_mesh_cell_twisted_past_two_splits_both_ways() {
        let rules = one_patch_rules(&moved_point_net(1, 1, [0.0, 0.0]), true);
        let lower_half_whole = |leaves: &[Cell]| {
            leaves.iter().any(|c| (c[0] - 0.5).abs() < EPS && (c[1] - 1.0).abs() < EPS && (c[2] - 0.25).abs() < EPS && (c[3] - 0.5).abs() < EPS)
        };
        let (far, _) = cells(&moved_point_net(1, 1, [18.0, 0.0]), true, &rules);
        assert!(!lower_half_whole(&far), "twist 2.25: split across too");
        let (near, _) = cells(&moved_point_net(1, 1, [14.0, 0.0]), true, &rules);
        assert!(lower_half_whole(&near), "twist 1.75: down only");
    }

    /// A first cell's split down carries to the other cells of its quarter of the envelope and no
    /// further. Arc Lower 60 in −12 % perspective: the cell at u ¼..½, v 0..¼ asks for nothing
    /// down itself and is split down with the corner cell beside it. Arc Upper 40 in −12 % by 8 %:
    /// first cells of both right quarters ask for the split and none on the left does, and the
    /// reference keeps the left half a quarter tall beside a right half in eighths.
    #[test]
    fn a_first_cells_split_down_stays_in_its_quarter() {
        use photocraft_geom::warp::WarpStyle;
        let tall = |c: &Cell| c[3] - c[2];
        let first_down = |log: &[Split], u: f64, v: f64| {
            log.iter().any(|s| s.down && (s.cell[0] - u).abs() < EPS && (s.cell[2] - v).abs() < EPS && (tall(&s.cell) - 0.25).abs() < EPS)
        };
        let at = in_perspective(WarpStyle::ArcLower, 60.0, -12.0, 0.0);
        let net = net_of(&at, false);
        let (leaves, log) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
        assert!(first_down(&log, 0.0, 0.0) && !first_down(&log, 0.25, 0.0));
        assert!(
            leaves.iter().filter(|c| c[0] > 0.25 - EPS && c[1] < 0.5 + EPS && c[3] < 0.25 + EPS).all(|c| (tall(c) - 0.125).abs() < EPS),
            "split down with its neighbour"
        );

        let at = in_perspective(WarpStyle::ArcUpper, 40.0, -12.0, 8.0);
        let net = net_of(&at, false);
        let (leaves, log) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
        let asks = |quarter: [f64; 2]| [0.0, 0.25].iter().any(|du| [0.0, 0.25].iter().any(|dv| first_down(&log, quarter[0] + du, quarter[1] + dv)));
        assert!(asks([0.5, 0.0]) && asks([0.5, 0.5]), "both right quarters have a cell that asks");
        assert!(!asks([0.0, 0.0]) && !asks([0.0, 0.5]), "no left cell does");
        assert!(leaves.iter().filter(|c| c[1] < 0.5 + EPS).all(|c| (tall(c) - 0.25).abs() < EPS), "the left half stays a quarter tall");
        assert!(leaves.iter().filter(|c| c[0] > 0.5 - EPS).all(|c| (tall(c) - 0.125).abs() < EPS), "the right half is in eighths");
    }

    /// An envelope's cell isn't cut at a smaller neighbour's point on a side that lies on the
    /// surface's middle line (u = ½): the two halves meet there whole. Arc Upper 40 in −12 % by
    /// 8 % perspective has cells a quarter tall left of the line and an eighth tall right of it,
    /// and the reference draws one quad per cell, the points between on the bigger cells' sides.
    /// Off the middle line a cell is cut at such points (Wave 30 in 8 % by 8 %).
    #[test]
    fn an_envelopes_cells_stay_whole_along_the_middle_line() {
        use photocraft_geom::warp::WarpStyle;
        let at = in_perspective(WarpStyle::ArcUpper, 40.0, -12.0, 8.0);
        let net = net_of(&at, false);
        let rules = one_patch_rules(&net, false);
        assert!(rules.cut_sides);
        let (leaves, _) = cells(&net, twisty_net(&net), &rules);
        let m = envelope_mesh(&at, false).expect("finite");
        assert!(leaves.contains(&[0.25, 0.5, 0.5, 0.75]) && leaves.contains(&[0.25, 0.5, 0.75, 1.0]), "left of the line, a quarter tall");
        assert!(leaves.iter().filter(|c| (c[0] - 0.5).abs() < EPS).all(|c| (c[3] - c[2] - 0.125).abs() < EPS), "right of it, an eighth");
        assert_eq!(m.quads.len(), leaves.len(), "one quad per cell");
        for v in [0.625, 0.875] {
            assert!(off_line(vertex(&m, 0.5, v - 0.125), vertex(&m, 0.5, v + 0.125), vertex(&m, 0.5, v)) < 1e-9, "(½, {v}) is on the bigger cell's side");
        }

        let at = in_perspective(WarpStyle::Wave, 30.0, 8.0, 8.0);
        let net = net_of(&at, false);
        let (leaves, _) = cells(&net, twisty_net(&net), &one_patch_rules(&net, false));
        let m = envelope_mesh(&at, false).expect("finite");
        assert!(m.quads.len() > leaves.len(), "{} quads for {} cells", m.quads.len(), leaves.len());
    }

    /// An envelope's top and bottom edges keep only their corners where the surface puts them:
    /// every other point follows the rows' chord rule, the midpoint too. Arc Lower 60 in −12 %
    /// perspective has a top edge that asks for 0.89 of a split from corner to corner, and the
    /// reference draws it straight, its quarter point 0.26 px off the surface. (In 20 % the edge
    /// asks for the split and its midpoint stays on the surface, see
    /// `an_envelopes_edge_quarter_points_follow_the_chord_rule`.) A full mesh's edges are drawn
    /// the same way: Fisheye 40 in 8 % by 8 % perspective.
    #[test]
    fn an_edge_that_needs_no_split_is_drawn_corner_to_corner() {
        use photocraft_geom::warp::WarpStyle;
        let at = distorted(WarpStyle::ArcLower, 60.0, -12.0);
        let net = net_of(&at, false);
        let asks = need(&segment(net.row(0.0), 0.0, 1.0));
        assert!(asks > 0.85 && asks < 1.0 - EPS, "the top edge asks for {asks} of a split");
        let m = envelope_mesh(&at, false).expect("finite");
        let (a, b) = (vertex(&m, 0.0, 0.0), vertex(&m, 1.0, 0.0));
        for u in [0.25, 0.5, 0.75] {
            assert!(off_line(a, b, vertex(&m, u, 0.0)) < 1e-9, "({u}, 0) is on the chord between the corners");
        }
        let (p, s) = (vertex(&m, 0.25, 0.0), at(0.25, 0.0));
        assert!((p[0] - s[0]).hypot(p[1] - s[1]) > 0.25, "(¼, 0) is off the surface: {p:?} vs {s:?}");
        for u in [0.0, 1.0] {
            assert_eq!(vertex(&m, u, 0.0), at(u, 0.0), "the corner at {u} stays");
        }

        let at = in_perspective(WarpStyle::Fisheye, 40.0, 8.0, 8.0);
        let net = net_of(&at, true);
        assert!(need(&segment(net.row(0.0), 0.0, 1.0)) < 1.0 - EPS);
        let m = envelope_mesh(&at, true).expect("finite");
        let (p, s) = (vertex(&m, 0.5, 0.0), at(0.5, 0.0));
        assert!(off_line(vertex(&m, 0.0, 0.0), vertex(&m, 1.0, 0.0), p) < 1e-9, "a full mesh's (½, 0) is on the chord between the corners");
        assert!((p[0] - s[0]).hypot(p[1] - s[1]) > 0.02, "and off the surface: {p:?} vs {s:?}");
    }

    /// A point that is on its chord already stays exactly where the surface puts it. A sheared
    /// flat mesh's rows and columns are straight, so every point of it is on its chord, and each
    /// vertex is the surface's to the last bit; dropped onto the chord it would come out a
    /// rounding step away, which is enough to move pixels along an edge through pixel centres.
    #[test]
    fn a_point_on_its_chord_stays_where_the_surface_puts_it() {
        for (sx, sy) in [(12.8, 6.4), (41.0, 13.7), (33.3, 7.7)] {
            let at = move |u: f64, v: f64| [96.3 + 128.0 * u + sx * v, 96.7 + 128.0 * v + sy * u];
            let m = envelope_mesh(&at, true).expect("finite");
            assert_eq!(m.uv.len(), 9);
            for (uv, p) in m.uv.iter().zip(&m.pos) {
                assert_eq!(*p, at(uv[0], uv[1]), "the vertex at {uv:?} of the mesh sheared by ({sx}, {sy})");
            }
            // The midpoints of the mesh's four sides are the points the chord rule is asked about.
            let sides = [([0.0, 0.0], [1.0, 0.0]), ([0.0, 1.0], [1.0, 1.0]), ([0.0, 0.0], [0.0, 1.0]), ([1.0, 0.0], [1.0, 1.0])];
            let moved = sides.iter().filter(|(a, b)| {
                let p = at((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
                onto(at(a[0], a[1]), at(b[0], b[1]), p) != p
            });
            assert!(moved.count() > 0, "the plain drop moves a point of the mesh sheared by ({sx}, {sy})");
        }
        // A point really off its chord is dropped onto it.
        assert_eq!(onto_chord([0.0, 0.0], [10.0, 0.0], [5.0, 1e-6]), [5.0, 0.0]);
        assert_eq!(onto_chord([0.0, 0.0], [10.0, 0.0], [5.0, 1e-12]), [5.0, 1e-12]);
    }
}
