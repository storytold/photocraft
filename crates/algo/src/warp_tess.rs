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
//!   spacing; the same test on its two columns splits it down (`v`);
//! * for a mesh that is cubic in one direction, a cell whose control net twists by more than
//!   [`TWIST_TOL`] splits along its longer side (a square cell: down the document). A full mesh's
//!   twist is the largest `P00 − P10 − P01 + P11` of the cell's net. An envelope's is how unevenly
//!   a quad of the cell's stored-degree net is stretched (the larger difference in length between
//!   two opposite sides of it): for a split across, over the net's left column of quads only (the
//!   reference splits Arc Lower's top-left corner cell, not its mirror image on the right), for a
//!   split down over its top row;
//! * a 3 × 3 envelope's wide cell splits across (never down) where the legs of the left column of
//!   its control points are more than [`TWIST_TOL`] longer than the next column's, in the cell or
//!   in the other half of the cell it was cut from;
//! * a square cell of a tall envelope (stored with more control points down than across: the
//!   presets bent sideways) whose whole net twists by more than [`TALL_SQUARE_TWIST_TOL`] splits
//!   across too;
//! * a first-level cell of a full mesh that splits across also splits down; a split down on the
//!   first level splits the other first-level cells of the same quarter of the surface down too
//!   (a full mesh's four cells are one such block);
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

use std::collections::HashMap;

/// A point in document pixels or normalised source coordinates.
type P = [f64; 2];

/// Sideways deviation (document px) of an inner control point that splits a segment.
const PERP_TOL: f64 = 1.0;
/// Mean deviation along the chord (document px) of the inner control points that splits it.
const TANG_TOL: f64 = 0.5;
/// Distance (document px) of an inner control point before its chord's start that splits a
/// segment, as the reference application's creases show (rows pulled back over themselves split
/// later there than past the chord's end).
const BEFORE_TOL: f64 = 1.35;
/// Distance (document px) of an inner control point past its chord's end that splits a segment.
const AFTER_TOL: f64 = 1.0;
/// Twist of a cell's control net (document px) past which it splits.
const TWIST_TOL: f64 = 1.0;
/// The twist past which a one-patch full mesh's cell that already splits across splits down too:
/// a corner pulled out keeps its cell at (½..¾, 0..¼) whole up to a twist of 1.16 and splits it
/// from 1.22 (a sweep of pulls), and an arc stored as a full mesh splits at 1.39.
const ACROSS_TWIST_TOL: f64 = 1.2;
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
    /// The row curve at `v` (control points across).
    fn row(&self, v: f64) -> Vec<P> {
        (0..self.nu).map(|i| blend(&(0..self.nv).map(|j| self.at(i, j)).collect::<Vec<_>>(), v)).collect()
    }
    /// The column curve at `u` (control points down).
    fn col(&self, u: f64) -> Vec<P> {
        (0..self.nv).map(|j| blend(&(0..self.nu).map(|i| self.at(i, j)).collect::<Vec<_>>(), u)).collect()
    }
    /// The net of the part `[u0, u1] × [v0, v1]`.
    fn sub(&self, c: Cell) -> Net {
        let rows: Vec<Vec<P>> = (0..self.nv).map(|j| segment(&(0..self.nu).map(|i| self.at(i, j)).collect::<Vec<_>>(), c[0], c[1])).collect();
        let cols: Vec<Vec<P>> =
            (0..self.nu).map(|i| segment(&rows.iter().map(|r| r.get(i).copied().unwrap_or([f64::NAN; 2])).collect::<Vec<_>>(), c[2], c[3])).collect();
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
    /// The largest `P00 − P10 − P01 + P11` (euclidean) of the net.
    fn twist(&self) -> f64 {
        let mut m = 0.0f64;
        for j in 0..self.nv.saturating_sub(1) {
            for i in 0..self.nu.saturating_sub(1) {
                let (a, b, c, d) = (self.at(i, j), self.at(i + 1, j), self.at(i, j + 1), self.at(i + 1, j + 1));
                let t = (a[0] - b[0] - c[0] + d[0]).hypot(a[1] - b[1] - c[1] + d[1]);
                if t.is_finite() {
                    m = m.max(t);
                }
            }
        }
        m
    }
}

/// The Bezier curve with control points `c` at `t` (de Casteljau).
fn blend(c: &[P], t: f64) -> P {
    let mut p = c.to_vec();
    while p.len() > 1 {
        p = p.windows(2).map(|w| [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]).collect();
    }
    p.first().copied().unwrap_or([f64::NAN; 2])
}

/// Splits the curve `c` at `t`: (left part, right part).
fn split(c: &[P], t: f64) -> (Vec<P>, Vec<P>) {
    let mut p = c.to_vec();
    let (mut left, mut right) = (Vec::with_capacity(c.len()), Vec::with_capacity(c.len()));
    while let (Some(&a), Some(&b)) = (p.first(), p.last()) {
        left.push(a);
        right.push(b);
        if p.len() == 1 {
            break;
        }
        p = p.windows(2).map(|w| [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]).collect();
    }
    right.reverse();
    (left, right)
}

/// The control points of the part `[t0, t1]` of the curve `c`.
fn segment(c: &[P], t0: f64, t1: f64) -> Vec<P> {
    let mut c = c.to_vec();
    if t1 < 1.0 {
        c = split(&c, t1).0;
    }
    if t0 > 0.0 && t1 > 0.0 {
        c = split(&c, t0 / t1).1;
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
/// [`BEFORE_TOL`] or [`AFTER_TOL`], and is left out of the mean along the chord.
fn need(c: &[P]) -> f64 {
    let n = c.len().saturating_sub(1);
    let (Some(&a), Some(&b)) = (c.first(), c.last()) else { return 0.0 };
    if n < 2 {
        return 0.0;
    }
    let ch = [b[0] - a[0], b[1] - a[1]];
    let d: Vec<P> = (1..n)
        .filter_map(|k| c.get(k))
        .enumerate()
        .map(|(k, p)| {
            let s = (k + 1) as f64 / n as f64;
            [p[0] - (a[0] + ch[0] * s), p[1] - (a[1] + ch[1] * s)]
        })
        .collect();
    let len = ch[0].hypot(ch[1]);
    let v = if len > 0.0 {
        let (tx, ty) = (ch[0] / len, ch[1] / len);
        let mut far = 0.0f64;
        let (mut sum, mut inside) = (0.0f64, 0usize);
        for (k, p) in (1..n).filter_map(|k| c.get(k).map(|p| (k, p))) {
            let r = [p[0] - a[0], p[1] - a[1]];
            // Where the point lies along the chord (0 at its start, 1 at its end) and how far off it.
            let along = (r[0] * tx + r[1] * ty) / len;
            let side = (r[0] * ty - r[1] * tx).abs();
            far = far.max(side / PERP_TOL);
            if along < 0.0 {
                far = far.max(side.hypot(-along * len) / BEFORE_TOL);
            } else if along > 1.0 {
                far = far.max(side.hypot((along - 1.0) * len) / AFTER_TOL);
            } else if along >= 0.0 {
                sum += (along - k as f64 / n as f64) * len;
                inside += 1;
            }
        }
        let tang = if inside > 0 { (sum / inside as f64).abs() } else { 0.0 };
        far.max(tang / TANG_TOL)
    } else {
        d.iter().map(|x| x[0].hypot(x[1])).fold(0.0, f64::max) / PERP_TOL
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
        if across {
            (0..net.nv).map(|j| (0..net.nu).map(|i| net.at(i, j)).collect()).collect()
        } else {
            (0..net.nu).map(|i| (0..net.nv).map(|j| net.at(i, j)).collect()).collect()
        }
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
            fixed_level: 1,
            mid_row_level: 1,
            cut_sides: false,
            across_twist: ACROSS_TWIST_TOL,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
        }
    } else {
        Rules {
            start: START,
            force_down: false,
            twist_spreads: false,
            fixed_level: 2,
            mid_row_level: 1,
            cut_sides: !tall,
            across_twist: TWIST_TOL,
            twist_by_side: true,
            square_twist: tall.then_some(TALL_SQUARE_TWIST_TOL),
            column_chords: tall.then_some(TALL_COLUMN_LEVEL),
            flat_columns: (!tall).then_some(FLAT_COLUMN_TOL),
        }
    }
}

/// Tessellates one patch of a warp split into several (a quilt), whose map from the patch's own
/// normalised coordinates to the document is `at`, as the reference application cuts each patch:
/// by the full mesh's rules, but starting from the whole patch as one cell (a flat or small patch
/// stays whole), without the first level's forced split down, and with a split down for a cell's
/// twist carried to the other cells cut from the same parent. `None` when the map isn't finite at
/// the corners.
pub(crate) fn quilt_patch_mesh(at: &dyn Fn(f64, f64) -> P) -> Option<EnvelopeMesh> {
    let corners = [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)];
    if corners.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return None;
    }
    let net = cubic_net(at);
    let rules = Rules {
        start: 1,
        force_down: false,
        twist_spreads: true,
        fixed_level: 0,
        mid_row_level: 0,
        cut_sides: false,
        across_twist: TWIST_TOL,
        twist_by_side: false,
        square_twist: None,
        column_chords: None,
        flat_columns: None,
    };
    let (leaves, log) = cells(&net, true, &rules);
    Some(triangulate(&net, at, &leaves, &log, &rules))
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
    /// The twist past which a cell that already splits across splits down too: [`TWIST_TOL`],
    /// as for any cell, but [`ACROSS_TWIST_TOL`] in a one-patch full mesh.
    across_twist: f64,
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
}

/// The parent cell of `c` (one halving up both ways), as a comparable key.
fn parent_of(c: &Cell) -> (i64, i64, i64, i64) {
    let (w, h) = (2.0 * (c[1] - c[0]), 2.0 * (c[3] - c[2]));
    let q = |t: f64| (t * 1e6).round() as i64;
    ((c[0] / w + EPS).floor() as i64, (c[2] / h + EPS).floor() as i64, q(w), q(h))
}

/// Splits down every cell whose parent has a cell marked in `from`.
fn spread_down(level_cells: &[Cell], decisions: &mut [(bool, bool)], from: &[bool]) {
    let parents: Vec<_> = level_cells.iter().zip(from).filter(|(_, t)| **t).map(|(c, _)| parent_of(c)).collect();
    if parents.is_empty() {
        return;
    }
    for (c, d) in level_cells.iter().zip(decisions) {
        if parents.contains(&parent_of(c)) {
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
        // Rows of sibling cells (by their top and bottom and their parent) with a square cell
        // split down for its curvature.
        let mut curved_rows: Vec<(f64, f64, i64, f64)> = Vec::new();
        let column = |c: &Cell| (c[0] / (2.0 * (c[1] - c[0])) + EPS).floor() as i64;
        for &c in &level_cells {
            let [u0, u1, v0, v1] = c;
            let fu = need(&segment(&net.row(v0), u0, u1)).max(need(&segment(&net.row(v1), u0, u1)));
            let fv = need(&segment(&net.col(u0), v0, v1)).max(need(&segment(&net.col(u1), v0, v1)));
            let (mut across, mut down) = (fu >= 1.0 - EPS, fv >= 1.0 - EPS);
            let mut twist_down = false;
            let (lu, lv) = (u1 - u0, v1 - v0);
            let square = (lu - lv).abs() < EPS;
            if twisty && down && square {
                curved_rows.push((v0, v1, column(&c), lu));
            }
            let goes_across = lu > lv + EPS;
            if twisty {
                let sub = net.sub(c);
                let tw = match (rules.twist_by_side, goes_across) {
                    (true, true) => sub.twist_left(),
                    (true, false) => sub.twist_top(),
                    (false, _) => sub.twist(),
                };
                if tw > TWIST_TOL + EPS && (!across || tw > rules.across_twist + EPS) {
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
                if tw > TWIST_TOL + EPS && (!across || tw > rules.across_twist + EPS) {
                    across = true;
                }
            }
            if let Some(tol) = rules.square_twist
                && twisty
                && square
                && net.sub(c).twist() > tol + EPS
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
        }
        if rules.twist_spreads {
            spread_down(&level_cells, &mut decisions, &twisted);
        }
        let first_level = level_cells.first().is_some_and(|c| (c[3] - c[2] - first).abs() < EPS);
        if first_level {
            let down: Vec<bool> = decisions.iter().map(|d| d.1).collect();
            spread_down(&level_cells, &mut decisions, &down);
        }
        for (c, d) in level_cells.iter().zip(&mut decisions) {
            if curved_rows.iter().any(|&(a, b, k, cw)| (a - c[2]).abs() < EPS && (b - c[3]).abs() < EPS && (cw - (c[1] - c[0])).abs() < EPS && k == column(c)) {
                d.1 = true;
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

/// The values of `line` strictly between `lo` and `hi`.
fn inside(line: Option<&Vec<f64>>, lo: f64, hi: f64) -> Vec<f64> {
    line.map(|l| l.iter().copied().filter(|&t| t > lo + 1e-12 && t < hi - 1e-12).collect()).unwrap_or_default()
}

fn lerp(a: P, b: P, s: f64) -> P {
    [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s]
}

/// The point of the segment `a`-`b` nearest `p`: a point dropped onto a bigger cell's edge.
fn onto(a: P, b: P, p: P) -> P {
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
    // A new vertex on a row the row didn't need is dropped onto that row segment's chord.
    let mut order = keys.to_vec();
    order.sort_by_key(|p| level(p[0]));
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
        if log.iter().any(|s| s.cell[0] <= a + 1e-9 && s.cell[1] >= b - 1e-9 && s.cell[2] < v - 1e-9 && s.cell[3] > v + 1e-9) {
            continue;
        }
        if need(&segment(&net.row(v), a, b)) < 1.0 - EPS {
            let m = onto_chord(place(&own, a, v), place(&own, b, v), surf(u, v));
            own.insert(key(u, v), m);
        }
    }
    // The same for columns: on the surface's left and right sides, or (a tall envelope) everywhere,
    // by the row rule turned on its side. Inside other envelopes, only where the column's own
    // stretch is nearly straight.
    order.sort_by_key(|p| level(p[1]));
    for &[u, v] in &order {
        let lv = level(v);
        let w = 1.0 / f64::from(1u32 << lv);
        let (a, b) = (v - w, v + w);
        let on_side = u.abs() < 1e-12 || (u - 1.0).abs() < 1e-12;
        let across = |s: &Split| s.cell[2] <= a + 1e-9 && s.cell[3] >= b - 1e-9 && s.cell[0] < u - 1e-9 && s.cell[1] > u + 1e-9;
        if let (None, Some(tol), false) = (rules.column_chords, rules.flat_columns, on_side) {
            if lv > fixed_level && !log.iter().any(across) && need(&segment(&net.col(u), a, b)) < tol {
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
        if need(&segment(&net.col(u), a, b)) < 1.0 - EPS {
            let m = onto_chord(place(&own, u, a), place(&own, u, b), surf(u, v));
            own.insert(key(u, v), m);
        }
    }
    // A point inside a bigger cell's edge is where the surface puts it, as far as its own cells see
    // it (the chord rules above are for points that are corners on both sides).
    for &[u0, u1, v0, v1] in leaves {
        for v in [v0, v1] {
            for u in inside(on_v.get(&v.to_bits()), u0, u1) {
                own.insert(key(u, v), surf(u, v));
            }
        }
        for u in [u0, u1] {
            for v in inside(on_u.get(&u.to_bits()), v0, v1) {
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
    for &&[u0, u1, v0, v1] in &by_size {
        let mut corner = |u: f64, v: f64| *pos.entry(key(u, v)).or_insert_with(|| place(&own, u, v));
        let (c00, c10, c01, c11) = (corner(u0, v0), corner(u1, v0), corner(u0, v1), corner(u1, v1));
        for (v, a, b) in [(v0, c00, c10), (v1, c01, c11)] {
            for u in inside(on_v.get(&v.to_bits()), u0, u1) {
                let o = place(&own, u, v);
                pos.entry(key(u, v)).or_insert_with(|| onto(a, b, o));
            }
        }
        for (u, a, b) in [(u0, c00, c01), (u1, c10, c11)] {
            for v in inside(on_u.get(&u.to_bits()), v0, v1) {
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
                for v in inside(on_u.get(&u.to_bits()), v0, v1) {
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
            // A point of this cell's side that is no vertex: on the side's chord, its own.
            let s = (v - v0) / (v1 - v0);
            uv.push([u, v]);
            out_pos.push(if u == u0 { lerp(c00, c01, s) } else { lerp(c10, c11, s) });
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
        let b = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
        let at = |u: f64, v: f64| {
            let (bu, bv) = (b(u), b(v));
            let mut o = [0.0; 2];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    o[0] += wu * wv * p[j * 4 + i][0];
                    o[1] += wu * wv * p[j * 4 + i][1];
                }
            }
            o
        };
        let (leaves, log) = cells(
            &net_of(&at, true),
            true,
            &Rules {
                start: START_FULL,
                force_down: true,
                twist_spreads: false,
                fixed_level: 1,
                mid_row_level: 1,
                cut_sides: false,
                across_twist: ACROSS_TWIST_TOL,
                twist_by_side: false,
                square_twist: None,
                column_chords: None,
                flat_columns: None,
            },
        );
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
        let b = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
        let at = |u: f64, v: f64| {
            let (bu, bv) = (b(u), b(v));
            let mut o = [0.0; 2];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    o[0] += wu * wv * p[j * 4 + i][0];
                    o[1] += wu * wv * p[j * 4 + i][1];
                }
            }
            o
        };
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

    /// A control point outside its chord's span (a row turning back on itself in a crease) counts
    /// by its distance from that end, not in the mean along the chord.
    #[test]
    fn need_measures_a_point_past_the_chord_from_that_end() {
        // 1 px before the start, the other point in its even place: whole (the mean along the
        // chord would be 1 px and split it); 1.4 px before splits.
        assert!(need(&[[0.0, 0.0], [-1.0, 0.0], [2.0, 0.0], [3.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [-1.4, 0.0], [2.0, 0.0], [3.0, 0.0]]) >= 1.0);
        // Past the end the tolerance is 1 px.
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [3.9, 0.0], [3.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [1.0, 0.0], [4.1, 0.0], [3.0, 0.0]]) >= 1.0);
        // The distance is from the end itself, sideways included: 1.2 px before and 0.7 px off.
        assert!(need(&[[0.0, 0.0], [-1.2, 0.0], [2.0, 0.0], [3.0, 0.0]]) < 1.0);
        assert!(need(&[[0.0, 0.0], [-1.2, 0.7], [2.0, 0.0], [3.0, 0.0]]) >= 1.0);
        // The points left inside still split by their own mean: half a px along.
        assert!(need(&[[0.0, 0.0], [-1.0, 0.0], [2.5, 0.0], [3.0, 0.0]]) >= 1.0);
        // Both points outside and close to the ends: nothing to split; non-finite points neither.
        assert!(need(&[[0.0, 0.0], [-0.5, 0.0], [3.5, 0.0], [3.0, 0.0]]) < 1.0);
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
                let q = blend(&cubic.col(u), v);
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
        let m = quilt_patch_mesh(&at).expect("finite");
        assert_eq!(m.quads.len(), 1);
        assert_eq!(m.uv.len(), 4);
        assert!(quilt_patch_mesh(&|_, _| [f64::NAN, 0.0]).is_none());
    }

    /// A quilt patch whose rows bend but whose columns are straight splits across only; a one-patch
    /// full mesh splits its first cells down as well.
    #[test]
    fn a_quilt_patch_splits_only_where_it_bends() {
        let at = |u: f64, v: f64| [128.0 * u, 128.0 * v + 20.0 * (std::f64::consts::PI * u).sin()];
        let m = quilt_patch_mesh(&at).expect("finite");
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
        let b = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
        let wave = |u: f64, v: f64| {
            let (bu, bv) = (b(u), b(v));
            let mut o = [0.0; 2];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    o[0] += wu * wv * p[j * 4 + i][0];
                    o[1] += wu * wv * p[j * 4 + i][1];
                }
            }
            o
        };
        let tallest = |rules: &Rules| {
            let (leaves, _) = cells(&cubic_net(&wave), true, rules);
            leaves.iter().filter(|c| c[2] >= 0.5 - 1e-9).map(|c| c[3] - c[2]).fold(0.0, f64::max)
        };
        let quilt = Rules {
            start: 1,
            force_down: false,
            twist_spreads: true,
            fixed_level: 0,
            mid_row_level: 0,
            cut_sides: false,
            across_twist: TWIST_TOL,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
        };
        assert!(tallest(&quilt) <= 0.25 + 1e-9, "bottom cells {}", tallest(&quilt));
        let alone = Rules { twist_spreads: false, ..quilt };
        assert!(tallest(&alone) >= 0.5 - 1e-9, "without the rule the bottom cells stay half the patch");
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
        let b = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
        let at = |u: f64, v: f64| {
            let (bu, bv) = (b(u), b(v));
            let mut o = [0.0; 2];
            for (j, wv) in bv.iter().enumerate() {
                for (i, wu) in bu.iter().enumerate() {
                    o[0] += wu * wv * p[j * 4 + i][0];
                    o[1] += wu * wv * p[j * 4 + i][1];
                }
            }
            o
        };
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

    /// In a full mesh, a cell that splits across for its rows splits down for its twist only past
    /// [`ACROSS_TWIST_TOL`]: the reference keeps the cell at (½..¾, 0..¼) of a corner pulled out
    /// 48 px a quarter tall (twist 1.06), and splits it pulled 55 px (1.22).
    #[test]
    fn a_full_mesh_cell_split_across_ignores_its_twist() {
        let at = corner_out();
        let net = net_of(&at, true);
        let full = Rules {
            start: START_FULL,
            force_down: true,
            twist_spreads: false,
            fixed_level: 1,
            mid_row_level: 1,
            cut_sides: false,
            across_twist: ACROSS_TWIST_TOL,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
        };
        let (leaves, _) = cells(&net, true, &full);
        let tall = |ls: &[Cell]| ls.iter().any(|c| c[0] >= 0.5 - 1e-9 && c[1] <= 0.75 + 1e-9 && c[2].abs() < 1e-9 && (c[3] - 0.25).abs() < 1e-9);
        assert!(tall(&leaves), "a quarter-tall cell stays at the top, right of the middle");
        let (old, _) = cells(&net, true, &Rules { across_twist: TWIST_TOL, ..full });
        assert!(!tall(&old), "with the twist it would split down");
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
        let full = Rules {
            start: START_FULL,
            force_down: true,
            twist_spreads: false,
            fixed_level: 1,
            mid_row_level: 1,
            cut_sides: false,
            across_twist: ACROSS_TWIST_TOL,
            twist_by_side: false,
            square_twist: None,
            column_chords: None,
            flat_columns: None,
        };
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
        let (low, _) = cells(&net, twisty_net(&net), &Rules { square_twist: Some(ACROSS_TWIST_TOL), ..rules });
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
            if !quarter || level(uv[1]) < 2 || need(&segment(&net.row(uv[1]), a, c)) >= 1.0 - EPS {
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
            let straight = need(&segment(&net.col(0.25), v - 0.125, v + 0.125));
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
            let asks = need(&segment(&net.row(0.5), a, b));
            assert!(asks > 0.8 && asks < 0.9, "the middle row asks for {asks} of a split");
            assert!(off(u, 0.5) > 0.6, "({u}, ½) is off the surface: {}", off(u, 0.5));
            assert!(off_line(vertex(&m, a, 0.5), vertex(&m, b, 0.5), vertex(&m, u, 0.5)) < 1e-9, "({u}, ½) is on the chord");
            assert!(off(a, 0.5) < 1e-9 && off(b, 0.5) < 1e-9, "the chord's ends stay on the surface");
            assert!(need(&segment(&net.row(0.25), a, b)) < 0.5);
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
        let asks = need(&segment(&net.row(0.0), 0.0, 1.0));
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
        assert!(need(&segment(&net.row(0.0), 0.0, 1.0)) < 1.0 - EPS);
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
