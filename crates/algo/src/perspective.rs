//! Edit › Perspective Warp: planes (quads) drawn over the image in Layout mode, whose corners are
//! then moved in Warp mode; the image follows a piecewise-homography map.
//!
//! Each plane maps its source quad onto its destination quad with a homography
//! `M_i = H_dst ∘ H_src⁻¹` (through the unit square, so `(u, v)` are the plane's projective
//! coordinates). Two homographies that agree on an edge's endpoints map the edge onto the same
//! segment but generally with a different spacing along it, so on its own a piecewise map tears
//! at shared edges. For every edge shared by two planes we therefore take the average of both
//! planes' images of each edge point as the target and add the difference, faded linearly
//! across the plane (weight `1 − u`, `u`, `1 − v` or `v` from that edge to the opposite one).
//! The correction vanishes at the corners (shared corners have one destination), so the map is
//! exactly continuous across shared edges and exactly the homography for a lone plane.
//! For multiple planes, corrected maps now contribute to one continuous global displacement
//! field: each plane has unit weight inside its source quad and Gaussian distance-based
//! falloff outside. This removes hard ownership switches across overlapping or separated
//! planes without blurring/compositing the raster output. The relative influence is
//! continuous, not a physical guarantee of identical tangents on different 3D surfaces.
//!
//! Corners that coincide (within half a pixel) in the source are linked: they share one
//! destination (the mean of the given ones), as Photoshop's snapped planes do. Layout
//! may explicitly weld complete near-adjacent edges before the warp is committed.

use serde::{Deserialize, Serialize};

use crate::transform::Homography;

/// One plane: source quad (Layout mode) and destination quad (Warp mode), corners clockwise from
/// top-left, document pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub src: [[f64; 2]; 4],
    pub dst: [[f64; 2]; 4],
}

impl Plane {
    pub fn identity(q: [[f64; 2]; 4]) -> Self {
        Plane { src: q, dst: q }
    }
}

const LINK_EPS: f64 = 0.5;

/// Unit square corners in plane order.
const UNIT: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

fn unit_to_quad(q: &[[f64; 2]; 4]) -> Option<Homography> {
    Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], *q)
}

/// A valid perspective plane must remain a strictly convex quadrilateral. The homography
/// solver alone can return a matrix even when the destination collapses to a point or when
/// consecutive edges cross; accepting such a map can silently deform or destroy pixels.
/// Both orientations are allowed (e.g. a deliberate mirror).
fn valid_quad(q: &[[f64; 2]; 4]) -> bool {
    let mut direction = 0.0_f64;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if !cross.is_finite() || cross.abs() <= 1.0e-9 {
            return false;
        }
        if direction != 0.0 && cross.signum() != direction {
            return false;
        }
        direction = cross.signum();
    }
    true
}

/// Union-find root with path compression.
fn uf_root(parent: &mut [usize], i: usize) -> usize {
    let mut current = i;
    while parent[current] != current {
        current = parent[current];
    }
    let root = current;
    let mut current = i;
    while parent[current] != root {
        let next = parent[current];
        parent[current] = root;
        current = next;
    }
    root
}

/// Groups of linked corners: `(plane, corner)` pairs that coincide in the source.
///
/// The groups are the transitive connected components of the relation "within `LINK_EPS` on
/// both axes in the original source space", so they depend only on the geometry, never on
/// plane order (a greedy first-representative grouping would). Groups are listed by their
/// smallest `(plane, corner)` member and members are in ascending order.
pub fn linked_corners(planes: &[Plane]) -> Vec<Vec<(usize, usize)>> {
    let n = planes.len() * 4;
    let pos = |i: usize| planes[i / 4].src[i % 4];
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = (pos(i), pos(j));
            if (a[0] - b[0]).abs() <= LINK_EPS && (a[1] - b[1]).abs() <= LINK_EPS {
                let (ri, rj) = (uf_root(&mut parent, i), uf_root(&mut parent, j));
                if ri != rj {
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut slot = vec![usize::MAX; n];
    let mut groups: Vec<Vec<(usize, usize)>> = Vec::new();
    for i in 0..n {
        let r = uf_root(&mut parent, i);
        if slot[r] == usize::MAX {
            slot[r] = groups.len();
            groups.push(Vec::new());
        }
        groups[slot[r]].push((i / 4, i % 4));
    }
    groups
}

/// Snaps linked corners to one source and one destination position (their means). The groups
/// are computed once from the original positions (see [`linked_corners`]) and the means are
/// taken over the original values, so the result does not depend on plane order.
pub fn unify(planes: &mut [Plane]) {
    let groups = linked_corners(planes);
    unify_groups(planes, &groups);
}

fn unify_groups(planes: &mut [Plane], groups: &[Vec<(usize, usize)>]) {
    for g in groups {
        if g.len() < 2 {
            continue;
        }
        let n = g.len() as f64;
        let s = g.iter().fold([0.0, 0.0], |a, &(p, c)| [a[0] + planes[p].src[c][0] / n, a[1] + planes[p].src[c][1] / n]);
        let d = g.iter().fold([0.0, 0.0], |a, &(p, c)| [a[0] + planes[p].dst[c][0] / n, a[1] + planes[p].dst[c][1] / n]);
        for &(p, c) in g {
            planes[p].src[c] = s;
            planes[p].dst[c] = d;
        }
    }
}

/// Match a complete edge on opposite sides of the same source boundary. Reject edges
/// belonging to overlapping planes facing the same way: they are not adjacent surfaces.
/// The two corner indices on `b` correspond to a[k] and a[k+1], respectively.
fn matched_adjacent_edge(a: &Plane, k: usize, b: &Plane, m: usize, tolerance: f64) -> Option<(usize, usize)> {
    let (ak, an, bm, bn) = (k, (k + 1) % 4, m, (m + 1) % 4);
    let (p, q) = (a.src[ak], a.src[an]);
    let ca = a.src.iter().fold([0.0; 2], |sum, p| [sum[0] + p[0] * 0.25, sum[1] + p[1] * 0.25]);
    let cb = b.src.iter().fold([0.0; 2], |sum, p| [sum[0] + p[0] * 0.25, sum[1] + p[1] * 0.25]);
    let side = |c: [f64; 2]| (q[0] - p[0]) * (c[1] - p[1]) - (q[1] - p[1]) * (c[0] - p[0]);
    if side(ca) * side(cb) >= 0.0 {
        return None;
    }
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= tolerance;
    if near(p, b.src[bm]) && near(q, b.src[bn]) {
        Some((bm, bn))
    } else if near(p, b.src[bn]) && near(q, b.src[bm]) {
        Some((bn, bm))
    } else {
        None
    }
}

/// Number of true shared edges (as opposed to two unrelated overlapping planes).
pub fn linked_edge_count(planes: &[Plane]) -> usize {
    let mut count = 0;
    for (i, a) in planes.iter().enumerate() {
        for b in planes.iter().skip(i + 1) {
            for k in 0..4 {
                for m in 0..4 {
                    if matched_adjacent_edge(a, k, b, m, LINK_EPS).is_some() {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Explicitly weld complete near-matching edges, while preserving all unrelated geometry.
/// Both source and destination corners are linked by group means (including 3+ planes
/// meeting at one point). No partially matched edges are snapped.
///
/// Returns the number of matched edge pairs. Invalid resulting quads abort without altering
/// the input slice. Tolerances are source document pixels, not screen/zoom pixels.
pub fn weld_adjacent_edges(planes: &mut [Plane], tolerance: f64) -> Result<usize, &'static str> {
    if !tolerance.is_finite() || !(0.0..=32.0).contains(&tolerance) {
        return Err("linkEdges tolerance must be finite and within 0..32 document pixels");
    }
    if planes.is_empty() {
        return Ok(0);
    }
    let n = planes.len() * 4;
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        let mut current = i;
        while parent[current] != current {
            current = parent[current];
        }
        let root = current;
        let mut current = i;
        while parent[current] != root {
            let next = parent[current];
            parent[current] = root;
            current = next;
        }
        root
    }
    let mut links = 0;
    for i in 0..planes.len() {
        for j in (i + 1)..planes.len() {
            for k in 0..4 {
                for m in 0..4 {
                    let Some((bk, bn)) = matched_adjacent_edge(&planes[i], k, &planes[j], m, tolerance) else {
                        continue;
                    };
                    for (ia, ib) in [(4 * i + k, 4 * j + bk), (4 * i + (k + 1) % 4, 4 * j + bn)] {
                        let (a, b) = (root(&mut parent, ia), root(&mut parent, ib));
                        if a != b {
                            parent[b] = a;
                        }
                    }
                    links += 1;
                }
            }
        }
    }
    if links == 0 {
        return Ok(0);
    }
    let original = planes.to_vec();
    let mut groups = vec![Vec::new(); n];
    for idx in 0..n {
        let r = root(&mut parent, idx);
        groups[r].push(idx);
    }
    for group in groups.iter().filter(|group| group.len() > 1) {
        let mut s = [0.0; 2];
        let mut d = [0.0; 2];
        let inv = 1.0 / group.len() as f64;
        for &index in group {
            let corner = index % 4;
            let plane = index / 4;
            for axis in 0..2 {
                s[axis] += original[plane].src[corner][axis] * inv;
                d[axis] += original[plane].dst[corner][axis] * inv;
            }
        }
        for &index in group {
            planes[index / 4].src[index % 4] = s;
            planes[index / 4].dst[index % 4] = d;
        }
    }
    if PerspectiveMap::new(planes).is_none() {
        planes.clone_from_slice(&original);
        return Err("linkEdges would create an invalid plane; no changes applied");
    }
    Ok(links)
}

/// Edge `k` of a plane runs from corner `k` to corner `k + 1`.
fn edge_uv(k: usize, u: f64, v: f64) -> ([f64; 2], f64) {
    // (point on the edge in plane coordinates, weight of that edge's correction)
    match k {
        0 => ([u, 0.0], 1.0 - v),
        1 => ([1.0, v], u),
        2 => ([u, 1.0], v),
        _ => ([0.0, v], 1.0 - u),
    }
}

/// Unit coordinate on edge k at its own 0..1 edge parameter. The edge on a
/// nearly-adjacent plane must be sampled on that plane's own boundary, not by
/// extrapolating its homography at an off-boundary point.
fn edge_point_uv(k: usize, t: f64) -> [f64; 2] {
    match k {
        0 => [t, 0.0],
        1 => [1.0, t],
        2 => [1.0 - t, 1.0],
        _ => [0.0, 1.0 - t],
    }
}

/// Projective spacing of edge `k` of a source quad: `r = w1 / w0`, the ratio of the homogeneous
/// weights at the edge's end corners. Along the edge the physical (linear) fraction `s` and the
/// plane's edge parameter `t` are related by `s = t·r / (1 − t + t·r)`, so a physical source
/// position can be converted between two planes without evaluating either one off its own edge.
/// Falls back to 1 (affine spacing) for anything non-finite.
fn edge_ratio(src: &Homography, q: &[[f64; 2]; 4], k: usize) -> f64 {
    let (a, b) = (q[k], q[(k + 1) % 4]);
    let mid = edge_point_uv(k, 0.5);
    let (x, y) = src.apply(mid[0], mid[1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if !(l2 > 0.0) {
        return 1.0;
    }
    let s = ((x - a[0]) * dx + (y - a[1]) * dy) / l2;
    let r = s / (1.0 - s);
    if s.is_finite() && s > 0.0 && s < 1.0 && r.is_finite() && r > 0.0 { r } else { 1.0 }
}

struct PlaneMap {
    src_inv: Homography,
    dst: Homography,
    quad: [[f64; 2]; 4],
    /// Per edge: every (other plane, corresponding edge, reversed edge order) sharing it.
    shared: [Vec<(usize, usize, bool)>; 4],
    /// Projective edge spacing of the source quad, see [`edge_ratio`].
    edge_ratio: [f64; 4],
    /// The plain local homography `dst ∘ src⁻¹` in source pixels (no edge correction); only its
    /// Jacobian at a source-boundary point is used, to extend the map outside the quad.
    local: Homography,
}

impl PlaneMap {
    /// Boundary Jacobian, exact for affines and moderate projective derivatives. Large
    /// projective derivatives retain their direction with a continuously bounded norm.
    fn jacobian(&self, x: f64, y: f64) -> Option<[f64; 4]> {
        const MAX_JACOBIAN_NORM: f64 = 1.0e3;
        let m = &self.local.0;
        if m[6] == 0.0 && m[7] == 0.0 {
            let j = [m[0] / m[8], m[1] / m[8], m[3] / m[8], m[4] / m[8]];
            return j.iter().all(|v| v.is_finite()).then_some(j);
        }
        let w = m[6] * x + m[7] * y + m[8];
        let (nx, ny) = (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]);
        // J = numerator / w². Normalize the numerator first so a derivative overflowing
        // near a horizon still has a finite direction, including the limit at w = 0.
        let numerator = [m[0] * w - nx * m[6], m[1] * w - nx * m[7], m[3] * w - ny * m[6], m[4] * w - ny * m[7]];
        if !w.is_finite() || numerator.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let peak = numerator.iter().fold(0.0_f64, |a, v| a.max(v.abs()));
        if peak == 0.0 {
            return (w != 0.0).then_some([0.0; 4]);
        }
        let direction = numerator.map(|v| v / peak);
        let length = direction.iter().fold(0.0_f64, |a, v| a.hypot(*v));
        let norm = (peak / w.abs() / w.abs()) * length;
        let safe = MAX_JACOBIAN_NORM * 0.5;
        if norm <= safe {
            return Some(numerator.map(|v| v / w / w));
        }
        // Value and first derivative agree with the exact norm at `safe`; the norm
        // approaches MAX without ever switching to an unrelated identity gradient.
        let bounded = safe + safe * (1.0 - (-(norm - safe) / safe).exp());
        Some(direction.map(|v| (v / length) * bounded))
    }
}

/// A source corner whose destination handle must be hit exactly: `delta` is the miss of the
/// plain blend there, added back with a compact bump (see [`PerspectiveMap::map`]).
struct Anchor {
    src: [f64; 2],
    delta: [f64; 2],
    radius: f64,
}

/// A prepared perspective warp (forward map, source → destination).
pub struct PerspectiveMap {
    planes: Vec<PlaneMap>,
    /// One anchor per distinct (source, destination) corner pair; empty for a lone plane.
    anchors: Vec<Anchor>,
}

/// Straighten modes (Photoshop's Warp-mode buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Straighten {
    Horizontal,
    Vertical,
    Auto,
}

impl Straighten {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "h" | "horizontal" | "level" => Some(Straighten::Horizontal),
            "v" | "vertical" => Some(Straighten::Vertical),
            "auto" | "both" => Some(Straighten::Auto),
            _ => None,
        }
    }
}

/// Makes the destination edges that are near-horizontal exactly horizontal and/or near-vertical
/// exactly vertical (within 45°), moving linked corners together.
pub fn straighten(planes: &mut [Plane], mode: Straighten) {
    unify(planes);
    let groups = linked_corners(planes);
    let group_of = |p: usize, c: usize| groups.iter().position(|g| g.contains(&(p, c))).unwrap_or(0);
    let mut pos: Vec<[f64; 2]> = groups.iter().map(|g| planes[g[0].0].dst[g[0].1]).collect();
    let edges: Vec<(usize, usize)> =
        (0..planes.len()).flat_map(|p| (0..4).map(move |k| (p, k))).map(|(p, k)| (group_of(p, k), group_of(p, (k + 1) % 4))).collect();
    for (a, b) in edges {
        let (pa, pb) = (pos[a], pos[b]);
        let (dx, dy) = ((pb[0] - pa[0]).abs(), (pb[1] - pa[1]).abs());
        if dy <= dx && matches!(mode, Straighten::Horizontal | Straighten::Auto) {
            let y = (pa[1] + pb[1]) / 2.0;
            pos[a][1] = y;
            pos[b][1] = y;
        } else if dx < dy && matches!(mode, Straighten::Vertical | Straighten::Auto) {
            let x = (pa[0] + pb[0]) / 2.0;
            pos[a][0] = x;
            pos[b][0] = x;
        }
    }
    for (g, p) in groups.iter().zip(pos) {
        for &(pi, ci) in g {
            planes[pi].dst[ci] = p;
        }
    }
}

impl PerspectiveMap {
    /// Prepares the map. `None` for empty/degenerate geometry, incompatible coincident
    /// handles, or anchor constraints that cannot be represented with finite arithmetic.
    pub fn new(planes: &[Plane]) -> Option<Self> {
        if planes.is_empty() {
            return None;
        }
        let mut planes = planes.to_vec();
        let groups = linked_corners(&planes);
        let mut component = vec![[0; 4]; planes.len()];
        for (id, group) in groups.iter().enumerate() {
            for &(p, c) in group {
                component[p][c] = id;
            }
        }
        unify_groups(&mut planes, &groups);
        if planes.iter().any(|p| !valid_quad(&p.src) || !valid_quad(&p.dst)) {
            return None;
        }
        let mut maps = Vec::with_capacity(planes.len());
        for (i, p) in planes.iter().enumerate() {
            let src = unit_to_quad(&p.src)?;
            let edge_ratios: [f64; 4] = std::array::from_fn(|k| edge_ratio(&src, &p.src, k));
            let src_inv = src.inverse()?;
            let dst = unit_to_quad(&p.dst)?;
            let mut shared: [Vec<(usize, usize, bool)>; 4] = Default::default();
            for (k, s) in shared.iter_mut().enumerate() {
                let (a, b) = (component[i][k], component[i][(k + 1) % 4]);
                *s = planes
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .filter_map(|(j, o)| {
                        (0..4).find_map(|m| {
                            let (c, d) = (component[j][m], component[j][(m + 1) % 4]);
                            if (a == c && b == d) || (a == d && b == c) {
                                // Component equality welds the endpoints exactly. The side
                                // check excludes coincident edges of overlapping surfaces.
                                return matched_adjacent_edge(p, k, o, m, 0.0).map(|(bk, _)| (j, m, bk != m));
                            }
                            None
                        })
                    })
                    .collect();
            }
            let local = dst.mul(&src_inv);
            maps.push(PlaneMap { src_inv, dst, quad: p.src, shared, edge_ratio: edge_ratios, local });
        }
        let mut map = PerspectiveMap { planes: maps, anchors: Vec::new() };
        if planes.len() > 1 {
            // Distinct original components can have coincident means. One source cannot
            // satisfy two different destinations; reject that conflict instead of silently
            // averaging constraints. Compatible coincident constraints need just one bump.
            let mut anchors: Vec<([f64; 2], [f64; 2])> = Vec::new();
            for p in &planes {
                for k in 0..4 {
                    if let Some((_, target)) = anchors.iter().find(|(src, _)| *src == p.src[k]) {
                        if *target != p.dst[k] {
                            return None;
                        }
                    } else {
                        anchors.push((p.src[k], p.dst[k]));
                    }
                }
            }
            for &(src, target) in &anchors {
                // Disjoint open supports: no other anchor contributes at this handle,
                // and the compact C¹ bump is zero with zero slope at its support boundary.
                let radius = anchors
                    .iter()
                    .filter(|(other, _)| *other != src)
                    .fold(LINK_EPS, |radius, (other, _)| radius.min(0.5 * (src[0] - other[0]).hypot(src[1] - other[1])));
                let base = map.blend(src[0], src[1])?;
                let delta = [target[0] - base.0, target[1] - base.1];
                if radius <= 0.0 || delta.iter().any(|v| !v.is_finite()) {
                    return None;
                }
                map.anchors.push(Anchor { src, delta, radius });
            }
        }
        Some(map)
    }

    /// Value of plane `i`'s local map at a point `p` OUTSIDE its source quad. The projective map is
    /// never evaluated beyond the quad (its horizon may cross other planes): the corrected map is
    /// taken at the nearest boundary point `b` and continued with the first-order term
    /// `J(b)·(p − b)`, `J` being the Jacobian of the local homography in source pixels. A shared
    /// affine map therefore continues exactly. Projective derivatives are smoothly bounded;
    /// only undefined arithmetic falls back to the constant displacement at `b`.
    fn extend_outside(&self, i: usize, p: [f64; 2], boundary: [f64; 2]) -> Option<(f64, f64)> {
        let pm = &self.planes[i];
        let (bu, bv) = pm.src_inv.apply(boundary[0], boundary[1]);
        let (bx, by) = self.map_plane(i, bu, bv)?;
        let (dx, dy) = (p[0] - boundary[0], p[1] - boundary[1]);
        let (ex, ey) = match pm.jacobian(boundary[0], boundary[1]) {
            Some(j) => (j[0] * dx + j[1] * dy, j[2] * dx + j[3] * dy),
            None => (dx, dy),
        };
        let out = (bx + ex, by + ey);
        (out.0.is_finite() && out.1.is_finite()).then_some(out)
    }

    /// Finite fallback when the blend's weights underflow/overflow. Evaluate only on
    /// source boundaries, even when the input is far beyond a projective horizon.
    fn nearest_extension(&self, x: f64, y: f64) -> (f64, f64) {
        let mut best = (f64::INFINITY, (x, y));
        for (i, pm) in self.planes.iter().enumerate() {
            let (boundary, distance) = nearest_on_quad(&pm.quad, [x, y]);
            if distance >= best.0 {
                continue;
            }
            if let Some(out) = self.extend_outside(i, [x, y], boundary) {
                best = (distance, out);
            }
        }
        best.1
    }

    /// Maps through one local projective plane, including the pre-existing shared-edge
    /// correction. Keeping this separate lets the global field blend the same corrected
    /// geometry rather than jumping between incompatible local maps.
    fn map_plane(&self, i: usize, u: f64, v: f64) -> Option<(f64, f64)> {
        let pm = &self.planes[i];
        let (mut ox, mut oy) = pm.dst.apply(u, v);
        if !ox.is_finite() || !oy.is_finite() {
            return None;
        }
        let (cu, cv) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        for (k, neighbors) in pm.shared.iter().enumerate() {
            if neighbors.is_empty() {
                continue;
            }
            let (e, w) = edge_uv(k, cu, cv);
            if w <= 0.0 {
                continue;
            }
            let mine = pm.dst.apply(e[0], e[1]);
            if !mine.0.is_finite() || !mine.1.is_finite() {
                return None;
            }
            // Match the SAME PHYSICAL source-space position on every neighbour's own
            // edge: this plane's edge parameter `t` is converted to the linear fraction
            // `s` along the (shared) source segment, then to each neighbour's parameter.
            // Equal `t` would pair different source points when the projective spacings
            // differ, and the neighbour is only ever sampled on its own boundary
            // (never extrapolated through its horizon).
            let t = match k {
                0 => cu,
                1 => cv,
                2 => 1.0 - cu,
                _ => 1.0 - cv,
            };
            let r = pm.edge_ratio[k];
            let s = (t * r / (1.0 - t + t * r)).clamp(0.0, 1.0);
            // Mean over this plane and ALL neighbours of the edge: order independent
            // for any number of planes sharing it (one neighbour gives the old 0.5).
            let (mut dx, mut dy) = (0.0, 0.0);
            for &(j, other_edge, reverse) in neighbors {
                let other = &self.planes[j];
                let s2 = if reverse { 1.0 - s } else { s };
                let t2 = (s2 / (s2 + (1.0 - s2) * other.edge_ratio[other_edge])).clamp(0.0, 1.0);
                let oe = edge_point_uv(other_edge, t2);
                let theirs = other.dst.apply(oe[0], oe[1]);
                if !t2.is_finite() || !theirs.0.is_finite() || !theirs.1.is_finite() {
                    return None;
                }
                dx += theirs.0 - mine.0;
                dy += theirs.1 - mine.1;
            }
            let scale = w / (neighbors.len() as f64 + 1.0);
            ox += scale * dx;
            oy += scale * dy;
        }
        if ox.is_finite() && oy.is_finite() { Some((ox, oy)) } else { None }
    }

    /// Continuous global forward map for multi-plane work. The previous "first containing
    /// plane, or nearest plane" rule introduced a hard jump wherever overlapping/adjacent
    /// quads had different local projective maps. Every plane now contributes smoothly:
    /// full weight inside its quad and a Gaussian falloff with distance outside.
    ///
    /// Subtracting the largest log-weight before accumulation is equivalent to a normalized
    /// weighted mean, without overflowing or allocating per mapped image vertex.
    /// A single plane keeps its original exact homography path.
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        if self.planes.len() == 1 {
            let (u, v) = self.planes[0].src_inv.apply(x, y);
            return self.map_plane(0, u, v).unwrap_or((x, y));
        }
        let Some(out) = self.blend(x, y) else {
            return self.nearest_extension(x, y);
        };
        // Anchor constraint: the unified destination handle must be hit exactly, for any
        // plane order. It is applied as a smooth DISPLACEMENT CORRECTION, never by
        // replacing the position with the static corner: at each source corner `c` near
        // `(x, y)` the blend's miss `δ = target − blend(c)` (precomputed per distinct linked
        // group in `new`) is added with the compact C¹ bump `φ = (1 − d²/R²)²` (φ = 1 and
        // φ' = 0 at `c`). The blend's own Jacobian is kept, so a legitimate affine/identity
        // region is not flattened (δ = 0 there), and the result is exactly `target` at `c`.
        // Each support stops before any other anchor, even if original component means
        // drift close together. This keeps all handle constraints exact without a flat core.
        let (mut sum_x, mut sum_y) = (0.0, 0.0);
        for a in &self.anchors {
            let distance = (a.src[0] - x).hypot(a.src[1] - y);
            if distance >= a.radius {
                continue;
            }
            let fraction = 1.0 - (distance / a.radius).powi(2);
            let phi = fraction * fraction;
            sum_x += phi * a.delta[0];
            sum_y += phi * a.delta[1];
        }
        let corrected = (out.0 + sum_x, out.1 + sum_y);
        if corrected.0.is_finite() && corrected.1.is_finite() {
            return corrected;
        }
        out
    }

    /// The smooth multi-plane blend without the corner-anchor correction.
    fn blend(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        const BLEND_RADIUS: f64 = 12.0; // source document pixels; local enough to preserve interiors
        let (mut best_log, mut weight_sum, mut sum_x, mut sum_y) = (f64::NEG_INFINITY, 0.0, 0.0, 0.0);
        for (i, pm) in self.planes.iter().enumerate() {
            let (u, v) = pm.src_inv.apply(x, y);
            let inside = u.is_finite() && v.is_finite() && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v);
            let (projected, distance) = if inside {
                (self.map_plane(i, u, v), 0.0)
            } else {
                // Never extrapolate a projective map beyond its source quad:
                // its horizon may run directly through another valid plane.
                // Extend the finite boundary value to first order instead.
                let (boundary, distance) = nearest_on_quad(&pm.quad, [x, y]);
                (self.extend_outside(i, [x, y], boundary), distance)
            };
            let Some((ox, oy)) = projected else { continue };
            // Anchor weights grow towards each source corner, so the blended
            // map converges to every explicitly chosen destination handle.
            // Exact corners take the same limit without an infinite weight.
            let corner_dist2 = pm.quad.iter().map(|p| (p[0] - x).powi(2) + (p[1] - y).powi(2)).fold(f64::INFINITY, f64::min);
            // Clamp the singular boost: an exact handle is enforced below using
            // its unified destination, never whichever plane happens to be first.
            let finite_corner_dist2 = corner_dist2.max(1.0e-18);
            let corner_boost = if corner_dist2 < BLEND_RADIUS * BLEND_RADIUS { (BLEND_RADIUS * BLEND_RADIUS / finite_corner_dist2).ln() } else { 0.0 };
            let log_weight = -0.5 * (distance / BLEND_RADIUS).powi(2) + corner_boost;
            if !log_weight.is_finite() {
                continue;
            }
            if log_weight > best_log {
                let scale = (best_log - log_weight).exp();
                weight_sum = weight_sum * scale + 1.0;
                sum_x = sum_x * scale + ox;
                sum_y = sum_y * scale + oy;
                best_log = log_weight;
            } else {
                let weight = (log_weight - best_log).exp();
                weight_sum += weight;
                sum_x += weight * ox;
                sum_y += weight * oy;
            }
        }
        if weight_sum > 0.0 {
            let out = (sum_x / weight_sum, sum_y / weight_sum);
            if out.0.is_finite() && out.1.is_finite() {
                return Some(out);
            }
        }
        None
    }

    /// Whether every plane maps onto itself (identity).
    pub fn is_identity(planes: &[Plane]) -> bool {
        planes.iter().all(|p| p.src.iter().zip(&p.dst).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9))
    }
}

/// Continuous nearest boundary point of a convex source quad. Outside the
/// quad this gives a bounded place to extend its displacement, avoiding poles
/// in a project's homography extrapolated into a neighboring plane.
fn nearest_on_quad(q: &[[f64; 2]; 4], p: [f64; 2]) -> ([f64; 2], f64) {
    let mut best = (q[0], f64::INFINITY);
    for k in 0..4 {
        let (a, b) = (q[k], q[(k + 1) % 4]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [a[0] + t * dx, a[1] + t * dy];
        let distance = (q[0] - p[0]).hypot(q[1] - p[1]);
        if distance < best.1 {
            best = (q, distance);
        }
    }
    best
}

/// Corners of the unit square in plane order (for callers drawing plane grids).
pub fn unit_corners() -> [[f64; 2]; 4] {
    UNIT
}

/// A point of plane `p` at plane coordinates `(u, v)`, in source space.
pub fn plane_point(p: &Plane, u: f64, v: f64, dst: bool) -> [f64; 2] {
    let q = if dst { &p.dst } else { &p.src };
    unit_to_quad(q)
        .map(|h| {
            let (x, y) = h.apply(u, v);
            [x, y]
        })
        .unwrap_or(q[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64), tol: f64) -> bool {
        (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol
    }

    #[test]
    fn reject_collapsed_and_crossing_quads_before_warping() {
        let square = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let collapsed = [[4.0, 4.0]; 4];
        let crossing = [[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]];
        assert!(PerspectiveMap::new(&[Plane { src: square, dst: collapsed }]).is_none());
        assert!(PerspectiveMap::new(&[Plane { src: crossing, dst: square }]).is_none());
        assert!(PerspectiveMap::new(&[Plane { src: square, dst: crossing }]).is_none());
        let mirrored = [[100.0, 0.0], [0.0, 0.0], [0.0, 100.0], [100.0, 100.0]];
        assert!(PerspectiveMap::new(&[Plane { src: square, dst: mirrored }]).is_some());
    }

    #[test]
    fn identity_and_a_known_homography() {
        let q = [[10.0, 10.0], [90.0, 12.0], [85.0, 70.0], [12.0, 75.0]];
        let m = PerspectiveMap::new(&[Plane::identity(q)]).unwrap();
        for p in [(20.0, 20.0), (50.0, 40.0), (0.0, 0.0), (120.0, 90.0)] {
            assert!(close(m.map(p.0, p.1), p, 1e-9), "{p:?} → {:?}", m.map(p.0, p.1));
        }
        // Square → trapezoid: corners land exactly, the centre lands where the homography says.
        let src = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let dst = [[20.0, 0.0], [80.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let m = PerspectiveMap::new(&[Plane { src, dst }]).unwrap();
        for (s, d) in src.iter().zip(&dst) {
            assert!(close(m.map(s[0], s[1]), (d[0], d[1]), 1e-9));
        }
        let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], dst).unwrap();
        assert!(close(m.map(50.0, 50.0), h.apply(50.0, 50.0), 1e-9));
        // The diagonals' crossing stays the crossing: (50, 50) maps to the trapezoid's diagonal intersection.
        let c = m.map(50.0, 50.0);
        assert!((c.0 - 50.0).abs() < 1e-9 && c.1 < 50.0, "{c:?}");
    }

    #[test]
    fn overlapping_planes_do_not_create_a_mapping_step() {
        // On the current first-containing-plane implementation, the overlap changes
        // ownership at x=70 and instantly changes the vertical displacement by 16px.
        // Multi-plane warping must instead form one continuous field.
        let a = Plane { src: [[0.0, 0.0], [70.0, 0.0], [70.0, 100.0], [0.0, 100.0]], dst: [[0.0, 0.0], [70.0, 0.0], [70.0, 100.0], [0.0, 100.0]] };
        let b = Plane { src: [[60.0, 0.0], [130.0, 0.0], [130.0, 100.0], [60.0, 100.0]], dst: [[60.0, 16.0], [130.0, 16.0], [130.0, 116.0], [60.0, 116.0]] };
        let m = PerspectiveMap::new(&[a, b]).unwrap();
        let left = m.map(70.0 - 0.001, 50.0);
        let right = m.map(70.0 + 0.001, 50.0);
        assert!((left.0 - right.0).hypot(left.1 - right.1) < 0.2, "hard seam at overlap ownership boundary: {left:?} -> {right:?}");
        let far_left = m.map(1.0, 50.0);
        assert!((far_left.1 - 50.0).abs() < 0.1, "far plane must not perturb unaffected interior: {far_left:?}");
    }

    /// A second plane may contain the extrapolated horizon of the first one.
    /// Its valid interior must never receive an unbounded projection.
    #[test]
    fn extrapolated_projective_horizon_cannot_corrupt_neighbor_plane() {
        let a = Plane {
            src: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            dst: [[0.0, 0.0], [100.0 / 11.0, 0.0], [100.0 / 11.0, 100.0 / 11.0], [0.0, 100.0]],
        };
        let b = Plane::identity([[-20.0, 0.0], [-1.0, 0.0], [-1.0, 100.0], [-20.0, 100.0]]);
        let map = PerspectiveMap::new(&[a, b]).unwrap();
        let left = map.map(-10.001, 50.0);
        let right = map.map(-9.999, 50.0);
        assert!(left.0.is_finite() && left.1.is_finite() && right.0.is_finite() && right.1.is_finite());
        assert!((left.0 - right.0).hypot(left.1 - right.1) < 2.0, "horizon caused a jump: {left:?} -> {right:?}");
        for p in [left, right] {
            assert!((p.0 + 10.0).hypot(p.1 - 50.0) < 25.0, "a distant plane corrupted the local identity: {p:?}");
        }
    }

    /// The global blend must honor all explicitly specified destination handles,
    /// not just avoid visible seams between them.
    #[test]
    fn multi_plane_blend_interpolates_unified_destination_corners() {
        let a = Plane::identity([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        let b = Plane { src: [[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]], dst: [[10.0, 0.0], [20.0, 10.0], [20.0, 20.0], [10.0, 10.0]] };
        let map = PerspectiveMap::new(&[a, b.clone()]).unwrap();
        for (s, dst) in b.src.iter().zip(b.dst.iter()) {
            assert!(
                close(map.map(s[0], s[1]), (dst[0], dst[1]), 1e-8),
                "destination handle at {s:?} was diluted; expected {dst:?}, got {:?}",
                map.map(s[0], s[1])
            );
        }
    }

    /// Almost-shared source edges must not evaluate the other plane's
    /// projective map beyond its own quad and through its horizon.
    #[test]
    fn shared_edge_correction_cannot_extrapolate_neighbor_horizon() {
        let a = Plane::identity([[0.0, 0.0], [10.0, 0.0], [10.0, 100.0], [0.0, 100.0]]);
        let b = Plane {
            src: [[10.25, 0.0], [20.25, 0.0], [20.25, 100.0], [10.25, 100.0]],
            dst: [[10.0, 0.0], [10.0 + 10.0 / 40.99, 0.0], [10.0 + 10.0 / 40.99, 100.0 / 40.99], [10.0, 100.0]],
        };
        let map = PerspectiveMap::new(&[a, b]).unwrap();
        let result = map.map(10.0, 50.0);
        assert!(result.0.is_finite() && result.1.is_finite());
        assert!((result.0 - 10.0).hypot(result.1 - 50.0) < 100.0, "shared-edge extrapolation amplified a distant horizon: {result:?}");
    }

    /// Three planes can share a source corner without sharing every edge.
    /// Every order and near-corner sample must honor the unified destination.
    #[test]
    fn near_shared_corners_are_order_independent_and_continuous() {
        let scale = |p: [f64; 2]| [16.0 * p[0], 16.0 * p[1]];
        let a_src = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let b_src = [[100.0, 0.0], [200.0, 0.0], [200.0, -100.0], [100.0, -100.0]];
        let a = Plane { src: a_src, dst: a_src.map(scale) };
        let b = Plane { src: b_src, dst: b_src.map(scale) };
        let c = Plane {
            src: [[100.25, 0.0], [150.25, 0.0], [150.25, 100.0], [100.25, 100.0]],
            dst: [[1600.0, 0.0], [2400.0, 0.0], [2400.0, 1600.0], [1600.0, 1600.0]],
        };
        let mut old_coordinate_result = None;
        for (idx, order) in [[a.clone(), b.clone(), c.clone()], [b, a, c]].into_iter().enumerate() {
            // Linking nearby SOURCE corners moves the handle to their unified
            // source coordinate. The original (100,0) is no longer the handle:
            // demanding its exact destination would require flattening a
            // finite disk, destroying identity and affine derivatives.
            let mut normalized = order.clone();
            unify(&mut normalized);
            let handle = normalized
                .iter()
                .flat_map(|plane| plane.src.iter())
                .find(|p| (p[0] - 100.0).abs() < 0.5 && p[1].abs() < 1e-9)
                .copied()
                .expect("linked corner retained");
            let map = PerspectiveMap::new(&order).unwrap();
            let exact = map.map(handle[0], handle[1]);
            let nearby = map.map(handle[0] + 0.5e-9, handle[1]);
            let far = map.map(handle[0] + 2.0e-9, handle[1]);
            for sample in [exact, nearby, far] {
                assert!((sample.0 - 1600.0).abs() < 0.1 && sample.1.abs() < 0.1, "order {idx}: unified handle missed its destination: {sample:?}");
            }
            assert!((exact.0 - far.0).hypot(exact.1 - far.1) < 0.1, "order {idx}: corner snap introduced a discontinuity: {exact:?} versus {far:?}");
            let old_coordinate = map.map(100.0, 0.0);
            if let Some(previous) = old_coordinate_result {
                assert!(close(old_coordinate, previous, 0.1), "original non-handle coordinate depends on plane order: {previous:?} versus {old_coordinate:?}");
            }
            old_coordinate_result = Some(old_coordinate);
        }
    }

    /// Two planes with different projective spacing along a shared edge: identity and a
    /// common translation must stay exact (no pairing of unequal source points).
    #[test]
    fn shared_edge_identity_and_translation_are_exact_with_unequal_projective_spacing() {
        let a_src = [[0.0, 0.0], [100.0, 0.0], [100.0, 1000.0], [0.0, 1000.0]];
        let b_src = [[100.0, 0.0], [200.0, 0.0], [300.0, 1000.0], [100.0, 1000.0]];
        for shift in [[0.0, 0.0], [7.0, -3.0]] {
            let mv = |q: [[f64; 2]; 4]| q.map(|p| [p[0] + shift[0], p[1] + shift[1]]);
            let planes = [Plane { src: a_src, dst: mv(a_src) }, Plane { src: b_src, dst: mv(b_src) }];
            for order in [[0, 1], [1, 0]] {
                let map = PerspectiveMap::new(&[planes[order[0]].clone(), planes[order[1]].clone()]).unwrap();
                for p in [(50.0, 500.0), (99.0, 250.0), (80.0, 900.0), (150.0, 500.0), (120.0, 100.0), (101.0, 800.0)] {
                    let got = map.map(p.0, p.1);
                    assert!(close(got, (p.0 + shift[0], p.1 + shift[1]), 1e-6), "shift {shift:?} order {order:?} {p:?} → {got:?}");
                }
            }
        }
    }

    /// Three planes on one exact shared edge, each with its own projective spacing:
    /// every order must give the same map, and an identity configuration stays identity.
    #[test]
    fn three_planes_on_one_edge_are_order_invariant() {
        let a = Plane::identity([[0.0, 0.0], [100.0, 0.0], [100.0, 1000.0], [0.0, 1000.0]]);
        let b = Plane::identity([[100.0, 0.0], [200.0, 0.0], [300.0, 1000.0], [100.0, 1000.0]]);
        let c = Plane::identity([[100.0, 0.0], [160.0, 0.0], [130.0, 1000.0], [100.0, 1000.0]]);
        let planes = [a, b, c];
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        for p in [(50.0, 500.0), (99.0, 250.0), (110.0, 900.0), (101.0, 300.0)] {
            for order in orders {
                let map = PerspectiveMap::new(&order.map(|i| planes[i].clone())).unwrap();
                let got = map.map(p.0, p.1);
                assert!(close(got, p, 1e-6), "identity broken at {p:?} order {order:?}: {got:?}");
            }
        }
        // Non-identity: moving only plane `a`'s outer corners; order must not matter.
        let mut moved = planes.clone();
        moved[0].dst = [[0.0, 20.0], [100.0, 0.0], [100.0, 1000.0], [0.0, 980.0]];
        let reference = PerspectiveMap::new(&moved).unwrap();
        for order in orders {
            let map = PerspectiveMap::new(&order.map(|i| moved[i].clone())).unwrap();
            for p in [(50.0, 500.0), (99.0, 250.0), (110.0, 900.0), (101.0, 300.0)] {
                let (r, g) = (reference.map(p.0, p.1), map.map(p.0, p.1));
                assert!(close(r, g, 1e-6), "order {order:?} {p:?}: {r:?} vs {g:?}");
            }
        }
    }

    /// A legitimate affine 16x map near a corner must not be flattened onto the corner.
    #[test]
    fn corner_anchor_does_not_flatten_affine_or_identity_maps() {
        let sq = |o: f64| [[o, 0.0], [o + 100.0, 0.0], [o + 100.0, 100.0], [o, 100.0]];
        let scale = |q: [[f64; 2]; 4]| q.map(|p| [16.0 * p[0], 16.0 * p[1]]);
        let planes = [Plane { src: sq(0.0), dst: scale(sq(0.0)) }, Plane { src: sq(1000.0), dst: scale(sq(1000.0)) }];
        let map = PerspectiveMap::new(&planes).unwrap();
        for p in [(0.2, 0.1), (0.05, 0.4), (0.0, 0.0), (0.3, 0.3)] {
            assert!(close(map.map(p.0, p.1), (16.0 * p.0, 16.0 * p.1), 1e-9), "{p:?} → {:?}", map.map(p.0, p.1));
        }
        let ident = [Plane::identity(sq(0.0)), Plane::identity(sq(1000.0))];
        let map = PerspectiveMap::new(&ident).unwrap();
        for p in [(0.2, 0.1), (1000.3, 0.2), (1100.0, 100.0)] {
            assert!(close(map.map(p.0, p.1), p, 1e-9), "{p:?} → {:?}", map.map(p.0, p.1));
        }
    }

    #[test]
    fn weld_adjacent_edges_joins_source_and_destination_atomically() {
        let a = Plane::identity([[0.0, 0.0], [50.0, 0.0], [50.0, 100.0], [0.0, 100.0]]);
        let b = Plane::identity([[54.0, 0.0], [104.0, 0.0], [104.0, 100.0], [54.0, 100.0]]);
        let mut ps = vec![a.clone(), b.clone()];
        assert_eq!(linked_edge_count(&ps), 0);
        assert_eq!(weld_adjacent_edges(&mut ps, 8.0), Ok(1));
        assert_eq!(ps[0].src[1], [52.0, 0.0]);
        assert_eq!(ps[0].src[2], [52.0, 100.0]);
        assert_eq!(ps[0].src[1], ps[1].src[0]);
        assert_eq!(ps[0].src[2], ps[1].src[3]);
        assert_eq!(ps[0].dst[1], ps[1].dst[0]);
        assert_eq!(ps[0].dst[2], ps[1].dst[3]);
        assert_eq!(linked_edge_count(&ps), 1);
        assert!(PerspectiveMap::new(&ps).is_some());

        let before = ps.clone();
        assert!(weld_adjacent_edges(&mut ps, f64::NAN).is_err());
        assert!(weld_adjacent_edges(&mut ps, 100.0).is_err());
        assert_eq!(ps, before, "invalid tolerance must not mutate any plane");

        let mut unrelated = vec![a, Plane::identity([[80.0, 0.0], [130.0, 0.0], [130.0, 100.0], [80.0, 100.0]])];
        let original = unrelated.clone();
        assert_eq!(weld_adjacent_edges(&mut unrelated, 8.0), Ok(0));
        assert_eq!(unrelated, original);
        // Two overlapping quads with matching top/bottom edges do not face each
        // other across those edges, so they are not falsely declared adjacent.
        let mut overlapped = vec![
            Plane::identity([[0.0, 0.0], [50.0, 0.0], [50.0, 100.0], [0.0, 100.0]]),
            Plane::identity([[4.0, 0.0], [54.0, 0.0], [54.0, 100.0], [4.0, 100.0]]),
        ];
        let original = overlapped.clone();
        assert_eq!(weld_adjacent_edges(&mut overlapped, 8.0), Ok(0));
        assert_eq!(overlapped, original);
    }

    #[test]
    fn shared_edges_stay_continuous() {
        // Two planes sharing the edge x = 50 (with slightly mismatched corners that get linked).
        let a = Plane { src: [[0.0, 0.0], [50.0, 0.0], [50.0, 100.0], [0.0, 100.0]], dst: [[0.0, 10.0], [50.0, 0.0], [55.0, 100.0], [0.0, 90.0]] };
        let b = Plane { src: [[50.2, 0.0], [100.0, 0.0], [100.0, 100.0], [50.0, 100.2]], dst: [[50.0, 0.0], [110.0, 20.0], [100.0, 80.0], [55.0, 100.0]] };
        let m = PerspectiveMap::new(&[a, b]).unwrap();
        // After linking, the shared edge runs from (50.1, 0) to (50, 100.1).
        for t in [0.05, 0.25, 0.5, 0.77, 0.95] {
            let (x, y) = (50.1 - 0.1 * t, 100.1 * t);
            let l = m.map(x - 1e-6, y);
            let r = m.map(x + 1e-6, y);
            assert!(close(l, r, 1e-4), "t={t}: {l:?} vs {r:?}");
        }
        assert_eq!(linked_corners(&[Plane::identity([[0.0; 2]; 4])]).len(), 1);
    }

    #[test]
    fn straighten_levels_and_plumbs_edges() {
        let mut ps =
            vec![Plane { src: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]], dst: [[0.0, 3.0], [100.0, -2.0], [96.0, 100.0], [4.0, 104.0]] }];
        straighten(&mut ps, Straighten::Vertical);
        assert_eq!(ps[0].dst[1][0], ps[0].dst[2][0]);
        assert_eq!(ps[0].dst[0][0], ps[0].dst[3][0]);
        assert_ne!(ps[0].dst[0][1], ps[0].dst[1][1]);
        straighten(&mut ps, Straighten::Auto);
        assert_eq!(ps[0].dst[0][1], ps[0].dst[1][1]);
        assert_eq!(ps[0].dst[2][1], ps[0].dst[3][1]);
    }

    const ORDERS3: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];

    /// F1: the old norm cutoff changed J to identity over a 2e-6 input interval,
    /// producing a 15.307px jump. The output increment must instead shrink with the input.
    #[test]
    fn projective_extension_is_continuous_across_the_old_jacobian_cutoff() {
        let sq = |x: f64| [[x, 0.0], [x + 100.0, 0.0], [x + 100.0, 100.0], [x, 100.0]];
        let project = |p: [f64; 2]| [p[0] / (1.0 - 0.0099 * p[1]), p[1] / (1.0 - 0.0099 * p[1])];
        let planes = [0.0, -102.0].map(|x| Plane { src: sq(x), dst: sq(x).map(project) });
        for order in [[0, 1], [1, 0]] {
            let map = PerspectiveMap::new(&order.map(|i| planes[i].clone())).unwrap();
            let mut previous = f64::INFINITY;
            for epsilon in [1e-6, 1e-7, 1e-8] {
                let a = map.map(-1.0, 97.815082496586 - epsilon);
                let b = map.map(-1.0, 97.815082496586 + epsilon);
                let jump = (a.0 - b.0).hypot(a.1 - b.1);
                assert!(jump <= 4000.0 * epsilon, "order {order:?}, epsilon {epsilon}: {a:?} -> {b:?}");
                assert!(jump < previous * 0.2, "increment did not shrink: {previous} -> {jump}");
                previous = jump;
            }
            // Away from regularization, preserve the actual projective derivative.
            let w = 1.0 - 0.0099 * 50.0;
            let j = map.planes[order.iter().position(|&i| i == 0).unwrap()].jacobian(0.0, 50.0).unwrap();
            for (got, expected) in j.into_iter().zip([1.0 / w, 0.0, 0.0, 1.0 / (w * w)]) {
                assert!((got - expected).abs() < 1e-10, "{got} != {expected}");
            }
        }
    }

    #[test]
    fn bounded_jacobian_retains_its_direction_when_the_derivative_overflows() {
        let mut map = PerspectiveMap::new(&[Plane::identity(UNIT)]).unwrap();
        // Invertible H(x,y) = (x/y, 1/y). J itself overflows at |y| = 1e-160,
        // but its bounded direction has the same finite limit on both sides of y = 0.
        map.planes[0].local = Homography([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
        let at_horizon = map.planes[0].jacobian(1.0, 0.0).unwrap();
        for y in [-1e-160, 1e-160] {
            let j = map.planes[0].jacobian(1.0, y).unwrap();
            assert!(j.iter().all(|v| v.is_finite()));
            assert!(j.iter().fold(0.0_f64, |a, v| a.hypot(*v)) <= 1000.0 + 1e-10);
            assert!(j[1] < 0.0 && j[3] < 0.0);
            for (got, expected) in j.into_iter().zip(at_horizon) {
                assert!((got - expected).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn common_affine_extension_is_not_subject_to_the_projective_norm_limit() {
        let sq = |x: f64| [[x, 0.0], [x + 100.0, 0.0], [x + 100.0, 100.0], [x, 100.0]];
        for scale in [16.0, 2000.0] {
            let affine = |p: [f64; 2]| [scale * p[0] + 0.25 * scale * p[1] + 7.0, -scale * p[1] - 3.0];
            let map = PerspectiveMap::new(&[0.0, 100.0].map(|x| Plane { src: sq(x), dst: sq(x).map(affine) })).unwrap();
            for p in [[99.0, 50.0], [101.0, 50.0], [-1.0, 20.0], [201.0, 100.0]] {
                let expected = affine(p);
                assert!(close(map.map(p[0], p[1]), (expected[0], expected[1]), 1e-7));
            }
        }
    }

    /// F2: component means may move within LINK_EPS of an unrelated component.
    /// Neither that proximity nor same-side coincident edges may create adjacency.
    #[test]
    fn close_component_means_preserve_identity_affines_and_all_handles() {
        let sq = |x: f64, y: f64| [[x, y], [x + 100.0, y], [x + 100.0, y + 100.0], [x, y + 100.0]];
        let sources = [sq(0.0, 0.0), sq(0.5, 0.5), sq(-0.01, 0.51)];
        for (scale, shift) in [(1.0, [0.0, 0.0]), (1.0, [7.0, -3.0]), (16.0, [0.0, 0.0])] {
            let affine = |p: [f64; 2]| (scale * p[0] + shift[0], scale * p[1] + shift[1]);
            let planes = sources.map(|src| Plane {
                src,
                dst: src.map(|p| {
                    let (x, y) = affine(p);
                    [x, y]
                }),
            });
            for order in ORDERS3 {
                let map = PerspectiveMap::new(&order.map(|i| planes[i].clone())).unwrap();
                assert!(map.planes.iter().all(|p| p.shared.iter().all(Vec::is_empty)));
                for p in [[1.0, 1.0], [0.25, 0.25], [-0.01, 0.51], [100.25, 100.25], [99.99, 100.51]] {
                    assert!(close(map.map(p[0], p[1]), affine(p), 1e-9), "scale {scale}, order {order:?}, at {p:?}");
                }
                let h = 1e-5;
                let (xp, xm) = (map.map(0.25 + h, 0.25), map.map(0.25 - h, 0.25));
                let (yp, ym) = (map.map(0.25, 0.25 + h), map.map(0.25, 0.25 - h));
                assert!(close(((xp.0 - xm.0) / (2.0 * h), (xp.1 - xm.1) / (2.0 * h)), (scale, 0.0), 1e-7));
                assert!(close(((yp.0 - ym.0) / (2.0 * h), (yp.1 - ym.1) / (2.0 * h)), (0.0, scale), 1e-7));
            }
        }
        // Different destinations make dilution of a neighboring handle observable.
        let mut planes = sources.map(Plane::identity);
        planes[2].dst = sources[2].map(|p| [p[0], p[1] + 10.0]);
        for order in ORDERS3 {
            let ordered = order.map(|i| planes[i].clone());
            let map = PerspectiveMap::new(&ordered).unwrap();
            let mut normalized = ordered;
            unify(&mut normalized);
            for plane in normalized {
                for (src, dst) in plane.src.into_iter().zip(plane.dst) {
                    assert!(close(map.map(src[0], src[1]), (dst[0], dst[1]), 1e-9), "order {order:?}, handle {src:?}");
                    for dx in [-1e-8, 1e-8] {
                        assert!(close(map.map(src[0] + dx, src[1]), (dst[0], dst[1]), 1e-6), "discontinuous handle {src:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn original_components_still_link_tolerated_neighbor_edges() {
        let a = Plane::identity([[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]);
        let b = Plane::identity([[100.25, 0.25], [200.25, 0.25], [200.25, 100.25], [100.25, 100.25]]);
        for planes in [[a.clone(), b.clone()], [b, a]] {
            let map = PerspectiveMap::new(&planes).unwrap();
            assert_eq!(map.planes.iter().flat_map(|p| &p.shared).map(Vec::len).sum::<usize>(), 2);
            for p in [(100.125, 0.125), (100.125, 50.0), (99.0, 50.0), (101.0, 50.0)] {
                assert!(close(map.map(p.0, p.1), p, 1e-9));
            }
        }
    }

    // A transitive ring of offsets surrounds a separate center component. Their means
    // can coincide (or be arbitrarily close) although none of the original corners link.
    fn ring_and_center_planes(offset: f64, shift: f64) -> Vec<Plane> {
        let sq = |x: f64, y: f64| [[x, y], [x + 100.0, y], [x + 100.0, y + 100.0], [x, y + 100.0]];
        let mut planes = vec![Plane::identity(sq(0.0, 0.0))];
        for ix in -4_i32..=4 {
            for iy in -4_i32..=4 {
                if ix.abs().max(iy.abs()) == 4 {
                    let src = sq(f64::from(ix) * 0.25 + offset, f64::from(iy) * 0.25);
                    planes.push(Plane { src, dst: src.map(|p| [p[0], p[1] + shift]) });
                }
            }
        }
        planes
    }

    #[test]
    fn coincident_component_means_reject_incompatible_handles() {
        assert!(PerspectiveMap::new(&ring_and_center_planes(0.0, 10.0)).is_none());
        let map = PerspectiveMap::new(&ring_and_center_planes(0.0, 0.0)).unwrap();
        assert!(close(map.map(1.0, 1.0), (1.0, 1.0), 1e-9));
    }

    #[test]
    fn close_anchors_with_nonzero_residuals_interpolate_without_a_flat_core() {
        let planes = ring_and_center_planes(1e-10, 10.0);
        let map = PerspectiveMap::new(&planes).unwrap();
        assert!(map.anchors.iter().any(|a| a.delta[1].abs() > 0.1));
        let mut normalized = planes;
        unify(&mut normalized);
        for plane in normalized {
            for (src, dst) in plane.src.into_iter().zip(plane.dst) {
                assert!(close(map.map(src[0], src[1]), (dst[0], dst[1]), 1e-9), "handle {src:?} -> {dst:?}");
            }
        }
        let center = &map.anchors[0];
        // Moving well inside the support must change the value; there is no snapped disk.
        let nearby = map.map(center.radius * 0.1, 0.0);
        assert!(nearby.1.abs() > 1e-3, "anchor introduced a flat core: {nearby:?}");
        // The correction and its first derivative vanish where the support stops.
        let h = center.radius * 1e-4;
        let residual = |x: f64| {
            let (out, base) = (map.map(x, 0.0), map.blend(x, 0.0).unwrap());
            (out.0 - base.0, out.1 - base.1)
        };
        let a = residual(center.radius - h);
        let b = residual(center.radius - 0.5 * h);
        assert!(a.0.hypot(a.1) > 0.0 && b.0.hypot(b.1) < 0.3 * a.0.hypot(a.1));
        assert!(close(residual(center.radius), (0.0, 0.0), 1e-12));
    }

    /// F3: squared blend distances overflow at x=1e156. The fallback must continue
    /// from the source boundary, not return the far side of H's horizon at x=100.
    #[test]
    fn extreme_coordinates_use_boundary_extension_when_blending_fails() {
        let plane = Plane { src: [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]], dst: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 50.0]] };
        let map = PerspectiveMap::new(&[plane.clone(), plane]).unwrap();
        let x = 1e156;
        assert!(map.blend(x, 25.0).is_none());
        let out = map.map(x, 25.0);
        assert!(out.0.is_finite() && out.1.is_finite());
        // dX/dx at the right boundary is 4; the old horizon-crossing result was -100.
        assert!((out.0 / x - 4.0).abs() < 1e-12, "unsafe fallback: {out:?}");
    }

    /// P1: squares at x offsets 0, 0.49, 0.51 are one transitive cluster per corner (0 ~ 0.49 ~ 0.51
    /// although 0 and 0.51 are > LINK_EPS apart) for every plane order. Greedy first-representative
    /// grouping gave two clusters for [A, B, C] and one for [B, A, C].
    #[test]
    fn transitive_corner_links_are_order_invariant() {
        let sq = |o: f64| [[o, 0.0], [o + 100.0, 0.0], [o + 100.0, 100.0], [o, 100.0]];
        let planes: Vec<Plane> = [0.0, 0.49, 0.51].iter().map(|&o| Plane { src: sq(o), dst: sq(o).map(|p| [p[0], p[1] + 10.0]) }).collect();
        let mean = (0.0 + 0.49 + 0.51) / 3.0;
        for order in ORDERS3 {
            let ordered: Vec<Plane> = order.iter().map(|&i| planes[i].clone()).collect();
            let groups = linked_corners(&ordered);
            assert_eq!(groups.len(), 4, "order {order:?}: {groups:?}");
            assert!(groups.iter().all(|g| g.len() == 3), "order {order:?}: {groups:?}");
            let mut unified = ordered.clone();
            unify(&mut unified);
            assert_eq!(unified[0].src, unified[1].src);
            assert_eq!(unified[1].src, unified[2].src);
            assert_eq!(unified[0].dst, unified[2].dst);
            assert!((unified[0].src[0][0] - mean).abs() < 1e-12 && (unified[0].dst[0][1] - 10.0).abs() < 1e-12);
            // dst = src + (0, 10): every point, including the unified handle, is translated exactly.
            let map = PerspectiveMap::new(&ordered).unwrap();
            for p in [(mean, 0.0), (mean + 100.0, 100.0), (50.0, 50.0), (0.1, 5.0), (100.2, 99.0), (mean + 0.05, 0.02)] {
                let got = map.map(p.0, p.1);
                assert!(close(got, (p.0, p.1 + 10.0), 1e-6), "order {order:?} {p:?} → {got:?}");
            }
        }
    }

    /// P1/P3: a true 3-way link with different destinations takes the mean handle for every
    /// order, and every order gives the same map.
    #[test]
    fn three_way_corner_link_hits_mean_handle_in_every_order() {
        let sq = |o: f64| [[o, 0.0], [o + 100.0, 0.0], [o + 100.0, 100.0], [o, 100.0]];
        let planes: Vec<Plane> =
            [0.0, 0.49, 0.51].iter().enumerate().map(|(i, &o)| Plane { src: sq(o), dst: sq(o).map(|p| [p[0], p[1] + 10.0 + 2.0 * i as f64]) }).collect();
        let mean = 1.0 / 3.0;
        let reference = PerspectiveMap::new(&planes).unwrap();
        for order in ORDERS3 {
            let map = PerspectiveMap::new(&order.map(|i| planes[i].clone())).unwrap();
            let handle = map.map(mean, 0.0);
            assert!(close(handle, (mean, 12.0), 1e-9), "order {order:?}: handle {handle:?}");
            for p in [(50.0, 50.0), (0.2, 3.0), (mean + 0.1, 0.1), (100.0, 100.0)] {
                let (r, g) = (reference.map(p.0, p.1), map.map(p.0, p.1));
                assert!(close(r, g, 1e-6), "order {order:?} {p:?}: {r:?} vs {g:?}");
            }
        }
    }

    /// P3: near a joined corner the affine Jacobian (16 I, det 256) is neither flattened nor singular.
    #[test]
    fn joined_corner_keeps_a_nonsingular_affine_jacobian() {
        let sq = |o: f64| [[o, 0.0], [o + 100.0, 0.0], [o + 100.0, 100.0], [o, 100.0]];
        let planes: Vec<Plane> = [0.0, 0.49, 0.51].iter().map(|&o| Plane { src: sq(o), dst: sq(o).map(|p| [16.0 * p[0], 16.0 * p[1]]) }).collect();
        let h = 1.0e-3;
        for order in ORDERS3 {
            let map = PerspectiveMap::new(&order.map(|i| planes[i].clone())).unwrap();
            for c in [(0.4, 0.3), (0.5, 0.2), (0.6, 0.45)] {
                let (xp, xm) = (map.map(c.0 + h, c.1), map.map(c.0 - h, c.1));
                let (yp, ym) = (map.map(c.0, c.1 + h), map.map(c.0, c.1 - h));
                let (a, b) = ((xp.0 - xm.0) / (2.0 * h), (yp.0 - ym.0) / (2.0 * h));
                let (c2, d) = ((xp.1 - xm.1) / (2.0 * h), (yp.1 - ym.1) / (2.0 * h));
                let det = a * d - b * c2;
                assert!((det - 256.0).abs() < 1e-3, "order {order:?} at {c:?}: det {det}");
                assert!(close(map.map(c.0, c.1), (16.0 * c.0, 16.0 * c.1), 1e-8), "order {order:?} at {c:?}");
            }
        }
    }

    /// P2: two touching planes sharing the affine map 16·x: both inside and outside points
    /// (including the corners) get exactly 16·x. Constant boundary displacement gave 1591.49
    /// instead of 1584 at (99, 50).
    #[test]
    fn shared_affine_map_continues_exactly_outside_each_plane() {
        let sq = |x0: f64| [[x0, 0.0], [x0 + 100.0, 0.0], [x0 + 100.0, 100.0], [x0, 100.0]];
        let scale = |q: [[f64; 2]; 4]| q.map(|p| [16.0 * p[0], 16.0 * p[1]]);
        let planes = [Plane { src: sq(0.0), dst: scale(sq(0.0)) }, Plane { src: sq(100.0), dst: scale(sq(100.0)) }];
        for order in [[0, 1], [1, 0]] {
            let map = PerspectiveMap::new(&[planes[order[0]].clone(), planes[order[1]].clone()]).unwrap();
            let points = [
                (99.0, 50.0),
                (101.0, 50.0),
                (99.9, 1.0),
                (100.1, 99.0),
                (100.0, 50.0),
                (0.0, 0.0),
                (100.0, 0.0),
                (100.0, 100.0),
                (200.0, 0.0),
                (200.0, 100.0),
                (50.0, 50.0),
                (150.0, 50.0),
            ];
            for p in points {
                let got = map.map(p.0, p.1);
                assert!(close(got, (16.0 * p.0, 16.0 * p.1), 1e-6), "order {order:?} {p:?} → {got:?}");
            }
        }
    }

    /// P2: differently projective neighbours stay finite, bounded and seam-free across the shared
    /// edge (each plane is continued linearly from its own boundary, never through its horizon).
    #[test]
    fn projective_neighbours_are_finite_and_continuous_across_the_shared_edge() {
        let a = Plane { src: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]], dst: [[0.0, 0.0], [100.0, 10.0], [100.0, 90.0], [0.0, 100.0]] };
        let b = Plane { src: [[100.0, 0.0], [200.0, 0.0], [200.0, 100.0], [100.0, 100.0]], dst: [[100.0, 10.0], [200.0, 0.0], [200.0, 100.0], [100.0, 90.0]] };
        for order in [[0, 1], [1, 0]] {
            let map = PerspectiveMap::new(&order.map(|i| [a.clone(), b.clone()][i].clone())).unwrap();
            for y in [10.0, 50.0, 90.0] {
                let (l, r) = (map.map(100.0 - 1e-3, y), map.map(100.0 + 1e-3, y));
                assert!(l.0.is_finite() && l.1.is_finite() && r.0.is_finite() && r.1.is_finite());
                assert!((l.0 - r.0).hypot(l.1 - r.1) < 0.1, "seam at y={y}: {l:?} -> {r:?}");
                for p in [l, r] {
                    assert!((p.0 - 100.0).abs() < 5.0 && (p.1 - y).abs() < 20.0, "y={y}: wild value {p:?}");
                }
            }
        }
    }
}
