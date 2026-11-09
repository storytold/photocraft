//! How the reference application cuts a one-patch warp into flat triangles: an envelope (a
//! preset's mesh, stored with fewer than four control points along a side) or a full 4 × 4 mesh.
//! Derived from coordinate probes of its warps (observation only).
//!
//! The surface is cut as a quadtree in the frame the envelope was stored in (a mesh stored with
//! more control points down than across is cut with the axes swapped), starting from 4 × 4 cells;
//! a full mesh is cut by its cubic net (even where it is flatter), starting from 2 × 2 cells:
//!
//! * a cell splits across (`u`) while one of its two rows, restricted to the cell and taken at its
//!   stored degree, has an inner control point at least [`PERP_TOL`] off the chord sideways, or its
//!   inner control points lie on average at least [`TANG_TOL`] along the chord from their even
//!   spacing; the same test on its two columns splits it down (`v`);
//! * for a mesh that is cubic in one direction, a cell whose control net twists (the largest
//!   `P00 − P10 − P01 + P11` of its stored-degree net) by more than [`TWIST_TOL`] splits along its
//!   longer side (a square cell: down the document);
//! * a first-level cell that splits across also splits down, and a down split anywhere on the
//!   first level splits every first-level cell down;
//! * for a mesh that is cubic in one direction, a cell split down for its columns' curvature splits
//!   its siblings beside it (the cells its parent was split into) down.
//!
//! Each cell becomes two triangles, split from its top-left corner, cut across at the vertices its
//! left and right neighbours put on its sides. A vertex inside a bigger cell's edge sits on that
//! edge's chord; so does a new vertex on a row whose own segment wasn't curved enough to need it,
//! unless it is inside a cell that was split, or both cells beside it were split for their
//! curvature; on the left and right sides of the surface the same holds for columns.

use std::collections::HashMap;

/// A point in document pixels or normalised source coordinates.
type P = [f64; 2];

/// Sideways deviation (document px) of an inner control point that splits a segment.
const PERP_TOL: f64 = 1.0;
/// Mean deviation along the chord (document px) of the inner control points that splits it.
const TANG_TOL: f64 = 0.5;
/// Twist of a cell's control net (document px) past which it splits.
const TWIST_TOL: f64 = 1.0;
/// Cells per side an envelope starts as.
const START: usize = 4;
/// Cells per side a full 4 × 4 mesh starts as.
const START_FULL: usize = 2;
/// Cells this narrow (normalised) are not split again: six halvings of an envelope's first cells.
const MIN_CELL: f64 = 1.0 / 64.0;
/// Comparisons allow this much rounding.
const EPS: f64 = 1e-6;
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
    fn transposed(&self) -> Net {
        let mut p = Vec::with_capacity(self.p.len());
        for i in 0..self.nu {
            for j in 0..self.nv {
                p.push(self.at(i, j));
            }
        }
        Net { nu: self.nv, nv: self.nu, p }
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
        let perp = d.iter().map(|x| (x[0] * ty - x[1] * tx).abs()).fold(0.0, f64::max);
        let tang = (d.iter().map(|x| x[0] * tx + x[1] * ty).sum::<f64>() / d.len().max(1) as f64).abs();
        (perp / PERP_TOL).max(tang / TANG_TOL)
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

/// A recorded split: the cell, whether it split across, and whether its rows were curved enough
/// to split it.
struct Split {
    cell: Cell,
    across: bool,
    curved_across: bool,
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
    let stored = net_of(at, full);
    // Cut in the frame the mesh was stored in: more points down than across swaps the axes.
    let swap = stored.nv > stored.nu;
    let net = if swap { stored.transposed() } else { stored };
    let surf = |u: f64, v: f64| if swap { at(v, u) } else { at(u, v) };
    let twisty = net.nu.max(net.nv) == 4;
    let rules = if full {
        Rules { start: START_FULL, force_down: true, twist_spreads: false, fixed_level: 1, curved_makers: false }
    } else {
        Rules { start: START, force_down: true, twist_spreads: false, fixed_level: 2, curved_makers: true }
    };
    let (leaves, log) = cells(&net, twisty, swap, &rules);
    let mesh = triangulate(&net, &surf, &leaves, &log, &rules);
    Some(if swap {
        EnvelopeMesh {
            uv: mesh.uv.iter().map(|p| [p[1], p[0]]).collect(),
            pos: mesh.pos,
            quads: mesh.quads.iter().map(|&[tl, tr, bl, br]| [tl, bl, tr, br]).collect(),
        }
    } else {
        mesh
    })
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
    let rules = Rules { start: 1, force_down: false, twist_spreads: true, fixed_level: 0, curved_makers: false };
    let (leaves, log) = cells(&net, true, false, &rules);
    Some(triangulate(&net, at, &leaves, &log, &rules))
}

/// What differs between cutting a one-patch warp and a patch of a quilt.
#[derive(Clone, Copy)]
struct Rules {
    /// Cells per side to start from.
    start: usize,
    /// A first-level cell split across splits down too.
    force_down: bool,
    /// A cell split down for its twist splits the other cells cut from its parent down too.
    twist_spreads: bool,
    /// Grid lines up to this many halvings in stay where they are on the surface (a T-junction
    /// further in sits on the chord of the edge it lies on): the starting grid's.
    fixed_level: u32,
    /// A T-junction between cells that both split across for their rows' curvature stays on the
    /// surface (envelopes only).
    curved_makers: bool,
}

/// The parent cell of `c` (one halving up both ways), as a comparable key.
fn parent_of(c: &Cell) -> (i64, i64, i64, i64) {
    let (w, h) = (2.0 * (c[1] - c[0]), 2.0 * (c[3] - c[2]));
    let q = |t: f64| (t * 1e6).round() as i64;
    ((c[0] / w + EPS).floor() as i64, (c[2] / h + EPS).floor() as i64, q(w), q(h))
}

/// Splits down every cell whose parent has a cell split down for its twist.
fn spread_twist(level_cells: &[Cell], decisions: &mut [(bool, bool)], twisted: &[bool]) {
    let parents: Vec<_> = level_cells.iter().zip(twisted).filter(|(_, t)| **t).map(|(c, _)| parent_of(c)).collect();
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
fn cells(net: &Net, twisty: bool, swap: bool, rules: &Rules) -> (Vec<Cell>, Vec<Split>) {
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
        // Rows of sibling cells (by their top and bottom and their parent) with a cell split down
        // for its curvature.
        let mut curved_rows: Vec<(f64, f64, i64, f64)> = Vec::new();
        let column = |c: &Cell| (c[0] / (2.0 * (c[1] - c[0])) + EPS).floor() as i64;
        for &c in &level_cells {
            let [u0, u1, v0, v1] = c;
            let fu = need(&segment(&net.row(v0), u0, u1)).max(need(&segment(&net.row(v1), u0, u1)));
            let fv = need(&segment(&net.col(u0), v0, v1)).max(need(&segment(&net.col(u1), v0, v1)));
            let (mut across, mut down) = (fu >= 1.0 - EPS, fv >= 1.0 - EPS);
            let curved_across = across;
            let mut twist_down = false;
            if twisty && down {
                curved_rows.push((v0, v1, column(&c), u1 - u0));
            }
            if twisty {
                let tw = net.sub(c).twist();
                if tw > TWIST_TOL + EPS {
                    let (lu, lv) = (u1 - u0, v1 - v0);
                    if lu > lv + EPS || (lu > lv - EPS && swap) {
                        across = true;
                    } else {
                        twist_down = !down;
                        down = true;
                    }
                }
            }
            if rules.force_down && across && (u1 - u0 - first).abs() < EPS && (v1 - v0 - first).abs() < EPS {
                down = true;
            }
            if across || down {
                log.push(Split { cell: c, across, curved_across });
            }
            decisions.push((across, down));
            twisted.push(twist_down);
        }
        if rules.twist_spreads {
            spread_twist(&level_cells, &mut decisions, &twisted);
        }
        let first_level = level_cells.first().is_some_and(|c| (c[3] - c[2] - first).abs() < EPS);
        if first_level && decisions.iter().any(|d| d.1) {
            for d in &mut decisions {
                d.1 = true;
            }
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

/// Vertices, their places and the quads of `leaves` (stored frame).
fn triangulate(net: &Net, surf: &dyn Fn(f64, f64) -> P, leaves: &[Cell], log: &[Split], rules: &Rules) -> EnvelopeMesh {
    let fixed_level = rules.fixed_level;
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
    let inside = |line: Option<&Vec<f64>>, lo: f64, hi: f64| -> Vec<f64> {
        line.map(|l| l.iter().copied().filter(|&t| t > lo + 1e-12 && t < hi - 1e-12).collect()).unwrap_or_default()
    };
    // Places: the biggest cells first put the vertices inside their edges on their chords.
    let mut pos: HashMap<(u64, u64), P> = HashMap::new();
    let mut by_size: Vec<&Cell> = leaves.iter().collect();
    by_size.sort_by(|a, b| ((b[1] - b[0]) * (b[3] - b[2])).total_cmp(&((a[1] - a[0]) * (a[3] - a[2]))));
    let lerp = |a: P, b: P, s: f64| [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s];
    for &&[u0, u1, v0, v1] in &by_size {
        let mut corner = |u: f64, v: f64| *pos.entry(key(u, v)).or_insert_with(|| surf(u, v));
        let (c00, c10, c01, c11) = (corner(u0, v0), corner(u1, v0), corner(u0, v1), corner(u1, v1));
        for (v, a, b) in [(v0, c00, c10), (v1, c01, c11)] {
            for u in inside(on_v.get(&v.to_bits()), u0, u1) {
                pos.entry(key(u, v)).or_insert_with(|| lerp(a, b, (u - u0) / (u1 - u0)));
            }
        }
        for (u, a, b) in [(u0, c00, c01), (u1, c10, c11)] {
            for v in inside(on_u.get(&u.to_bits()), v0, v1) {
                pos.entry(key(u, v)).or_insert_with(|| lerp(a, b, (v - v0) / (v1 - v0)));
            }
        }
    }
    for p in &keys {
        pos.entry(key(p[0], p[1])).or_insert_with(|| surf(p[0], p[1]));
    }
    let place = |pos: &HashMap<(u64, u64), P>, u: f64, v: f64| pos.get(&key(u, v)).copied().unwrap_or_else(|| surf(u, v));
    // A new vertex on a row the row didn't need sits on that row segment's chord.
    let mut order = keys.clone();
    order.sort_by_key(|p| level(p[0]));
    for &[u, v] in &order {
        let lv = level(u);
        if lv <= fixed_level {
            continue;
        }
        let w = 1.0 / f64::from(1u32 << lv);
        let (a, b) = (u - w, u + w);
        if log.iter().any(|s| s.cell[0] <= a + 1e-9 && s.cell[1] >= b - 1e-9 && s.cell[2] < v - 1e-9 && s.cell[3] > v + 1e-9) {
            continue;
        }
        let makers: Vec<&Split> = log.iter().filter(|s| s.across && (s.cell[0] - a).abs() < 1e-9 && (s.cell[1] - b).abs() < 1e-9).collect();
        let above: Vec<&&Split> = makers.iter().filter(|s| (s.cell[3] - v).abs() < 1e-9).collect();
        let below: Vec<&&Split> = makers.iter().filter(|s| (s.cell[2] - v).abs() < 1e-9).collect();
        if rules.curved_makers && !above.is_empty() && !below.is_empty() && above.iter().chain(&below).all(|s| s.curved_across) {
            continue;
        }
        if need(&segment(&net.row(v), a, b)) < 1.0 - EPS {
            let m = lerp(place(&pos, a, v), place(&pos, b, v), 0.5);
            pos.insert(key(u, v), m);
        }
    }
    // The same for columns on the surface's left and right sides.
    order.sort_by_key(|p| level(p[1]));
    for &[u, v] in &order {
        let lv = level(v);
        if lv <= fixed_level || !(u.abs() < 1e-12 || (u - 1.0).abs() < 1e-12) {
            continue;
        }
        let w = 1.0 / f64::from(1u32 << lv);
        let (a, b) = (v - w, v + w);
        if need(&segment(&net.col(u), a, b)) < 1.0 - EPS {
            let m = lerp(place(&pos, u, a), place(&pos, u, b), 0.5);
            pos.insert(key(u, v), m);
        }
    }
    // Quads: each leaf cut across at the vertices on its left and right sides.
    let mut uv: Vec<P> = Vec::new();
    let mut out_pos: Vec<P> = Vec::new();
    let mut index: HashMap<(u64, u64), usize> = HashMap::new();
    let mut quads = Vec::new();
    for &[u0, u1, v0, v1] in leaves {
        let mut vs = vec![v0, v1];
        vs.extend(inside(on_u.get(&u0.to_bits()), v0, v1));
        vs.extend(inside(on_u.get(&u1.to_bits()), v0, v1));
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
        let (leaves, log) =
            cells(&net_of(&at, true), true, false, &Rules { start: START_FULL, force_down: true, twist_spreads: false, fixed_level: 1, curved_makers: false });
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
            let (leaves, _) = cells(&cubic_net(&wave), true, false, rules);
            leaves.iter().filter(|c| c[2] >= 0.5 - 1e-9).map(|c| c[3] - c[2]).fold(0.0, f64::max)
        };
        let quilt = Rules { start: 1, force_down: false, twist_spreads: true, fixed_level: 0, curved_makers: false };
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
        let chord = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0];
        assert!((mid[0] - chord[0]).abs() < 1e-9 && (mid[1] - chord[1]).abs() < 1e-9, "{mid:?} vs chord {chord:?}");
        let surf = at(0.25, 0.5);
        assert!((surf[1] - mid[1]).abs() > 0.1, "the chord is off the surface here");
    }
}
