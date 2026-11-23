//! Path alignment on the components of one path, the options bar's Path alignment popup (#13456):
//! Align moves each component so its edge or centre lines up with the components' combined bounds
//! (Align To Selection) or the canvas (Align To Canvas); Distribute spaces their edges or centres
//! evenly between the outermost two; Distribute Spacing makes the gaps between them equal. A
//! component moves as a whole, with its handles and the subpaths joined to it.

use std::ops::Range;

use photocraft_doc::Path;
use photocraft_geom::Point;
use serde_json::Value;

use super::edit_path;
use crate::vector_cmds::bad;
use crate::{EngineError, Result, Session};

/// An edge or centre line of a box, as `layer.align.*` and `layer.distribute.*` name them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

impl Edge {
    fn parse(k: &str) -> Option<Edge> {
        Some(match k {
            "leftEdges" => Edge::Left,
            "horizontalCenters" => Edge::HCenter,
            "rightEdges" => Edge::Right,
            "topEdges" => Edge::Top,
            "verticalCenters" => Edge::VCenter,
            "bottomEdges" => Edge::Bottom,
            _ => return None,
        })
    }

    fn vertical(self) -> bool {
        matches!(self, Edge::Top | Edge::VCenter | Edge::Bottom)
    }

    /// The edge's coordinate on `b` (`[x0, y0, x1, y1]`).
    fn value(self, b: &Bounds) -> f64 {
        let (lo, hi) = if self.vertical() { (b[1], b[3]) } else { (b[0], b[2]) };
        match self {
            Edge::Left | Edge::Top => lo,
            Edge::Right | Edge::Bottom => hi,
            Edge::HCenter | Edge::VCenter => (lo + hi) / 2.0,
        }
    }

    /// A move by `d` across this edge.
    fn delta(self, d: f64) -> [f64; 2] {
        if self.vertical() { [0.0, d] } else { [d, 0.0] }
    }
}

/// `[x0, y0, x1, y1]`.
type Bounds = [f64; 4];

/// The tight bounds of the subpaths in `range`: their anchors and the extremes of their curves
/// (not the handles, which lie off the outline). `None` when they have no anchor.
fn component_bounds(path: &Path, range: Range<usize>) -> Option<Bounds> {
    let mut out: Option<Bounds> = None;
    let mut add = |p: Point| {
        out = Some(match out {
            Some(b) => [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)],
            None => [p.x, p.y, p.x, p.y],
        });
    };
    for sp in path.subpaths.get(range).unwrap_or_default() {
        for k in &sp.knots {
            add(k.anchor);
        }
        for [p0, c1, c2, p1] in sp.segments() {
            for t in extremes(p0.x, c1.x, c2.x, p1.x).into_iter().chain(extremes(p0.y, c1.y, c2.y, p1.y)).flatten() {
                add(cubic(p0, c1, c2, p1, t));
            }
        }
    }
    out
}

/// The parameters in (0, 1) where one coordinate of a cubic Bézier has a turning point: the roots
/// of its derivative, `A t² + B t + C`.
fn extremes(p0: f64, c1: f64, c2: f64, p1: f64) -> [Option<f64>; 2] {
    let (a, b, c) = (c1 - p0, c2 - c1, p1 - c2);
    let (qa, qb, qc) = (a - 2.0 * b + c, 2.0 * (b - a), a);
    let inside = |t: f64| (t.is_finite() && t > 0.0 && t < 1.0).then_some(t);
    if qa.abs() < 1e-12 {
        return [if qb.abs() < 1e-12 { None } else { inside(-qc / qb) }, None];
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return [None, None];
    }
    let root = disc.sqrt();
    [inside((-qb + root) / (2.0 * qa)), inside((-qb - root) / (2.0 * qa))]
}

/// The point at `t` on the cubic Bézier `p0 c1 c2 p1`.
fn cubic(p0: Point, c1: Point, c2: Point, p1: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(w0 * p0.x + w1 * c1.x + w2 * c2.x + w3 * p1.x, w0 * p0.y + w1 * c1.y + w2 * c2.y + w3 * p1.y)
}

/// Moves the subpaths in `range` by `d`, anchors and handles alike.
fn translate(path: &mut Path, range: Range<usize>, d: [f64; 2]) {
    for sp in path.subpaths.get_mut(range).unwrap_or_default() {
        for k in &mut sp.knots {
            for p in [&mut k.anchor, &mut k.in_ctrl, &mut k.out_ctrl] {
                p.x += d[0];
                p.y += d[1];
            }
        }
    }
}

/// The subpath indices `subpaths` names, or `None` for every component. Present, it must be a
/// non-empty list of indices.
fn subpaths_param(cmd: &str, p: &Value) -> Result<Option<Vec<usize>>> {
    let Some(v) = p.get("subpaths") else { return Ok(None) };
    let list = v.as_array().filter(|a| !a.is_empty()).ok_or_else(|| bad(cmd, "`subpaths` must list subpath indices"))?;
    list.iter().map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "each subpath is an index"))).collect::<Result<_>>().map(Some)
}

/// The components to act on (those holding `subpaths`, else all of them), front to back as in the
/// path, each with its bounds; components without an anchor have none and are left out.
fn operands(path: &Path, subpaths: Option<&[usize]>) -> std::result::Result<Vec<(Range<usize>, Bounds)>, String> {
    let components = path.components();
    let chosen: Vec<Range<usize>> = match subpaths {
        None => components,
        Some(list) => {
            let mut heads = Vec::with_capacity(list.len());
            for i in list {
                let c = components.iter().find(|c| c.contains(i)).ok_or_else(|| format!("no subpath {i} (the path has {})", path.subpaths.len()))?;
                if !heads.contains(c) {
                    heads.push(c.clone());
                }
            }
            heads.sort_by_key(|c| c.start);
            heads
        }
    };
    Ok(chosen.into_iter().filter_map(|c| Some((c.clone(), component_bounds(path, c)?))).collect())
}

/// `path.alignComponents`: lines the components up on one edge or centre, one History step.
pub(super) fn align_components(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.alignComponents";
    let edge =
        p.get("edge").and_then(Value::as_str).and_then(Edge::parse).ok_or_else(|| {
            bad(CMD, "`edge` must be \"leftEdges\", \"horizontalCenters\", \"rightEdges\", \"topEdges\", \"verticalCenters\" or \"bottomEdges\"")
        })?;
    let canvas = match p.get("to") {
        None => false,
        Some(v) => match v.as_str() {
            Some("selection") => false,
            Some("canvas") => true,
            _ => return Err(bad(CMD, "`to` must be \"selection\" or \"canvas\"")),
        },
    };
    let subpaths = subpaths_param(CMD, p)?;
    let page = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let page = [f64::from(page.x0), f64::from(page.y0), f64::from(page.x1), f64::from(page.y1)];
    edit_path(s, CMD, p, "Align Path Components", |path| {
        let items = operands(path, subpaths.as_deref())?;
        let target = if canvas {
            if items.is_empty() {
                return Err("there is no component to align".into());
            }
            page
        } else {
            if items.len() < 2 {
                return Err("aligning to the selection needs two or more components".into());
            }
            items.iter().fold(items[0].1, |a, (_, b)| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
        };
        let to = edge.value(&target);
        for (range, b) in items {
            translate(path, range, edge.delta(to - edge.value(&b)));
        }
        Ok(())
    })
}

/// `path.distributeComponents`: spaces three or more components evenly, one History step. The
/// outermost two stay where they are.
pub(super) fn distribute_components(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.distributeComponents";
    let kind = p.get("edge").and_then(Value::as_str).unwrap_or_default().to_owned();
    let edge = Edge::parse(&kind);
    if edge.is_none() && kind != "horizontally" && kind != "vertically" {
        return Err(bad(
            CMD,
            "`edge` must be \"leftEdges\", \"horizontalCenters\", \"rightEdges\", \"topEdges\", \"verticalCenters\", \"bottomEdges\", \"horizontally\" or \"vertically\"",
        ));
    }
    let subpaths = subpaths_param(CMD, p)?;
    edit_path(s, CMD, p, "Distribute Path Components", |path| {
        let mut items = operands(path, subpaths.as_deref())?;
        if items.len() < 3 {
            return Err("distributing needs three or more components".into());
        }
        let n = items.len();
        let mut moves = Vec::with_capacity(n);
        match edge {
            Some(edge) => {
                // Evenly spaced edges or centres between the first and the last.
                items.sort_by(|a, b| edge.value(&a.1).total_cmp(&edge.value(&b.1)));
                let (first, last) = (edge.value(&items[0].1), edge.value(&items[n - 1].1));
                for (i, (range, b)) in items.into_iter().enumerate() {
                    let want = first + (last - first) * i as f64 / (n - 1) as f64;
                    moves.push((range, edge.delta(want - edge.value(&b))));
                }
            }
            None => {
                // Equal gaps between neighbours, as `layer.distribute.horizontally|vertically`.
                let v = kind == "vertically";
                let lo = |b: &Bounds| if v { b[1] } else { b[0] };
                let hi = |b: &Bounds| if v { b[3] } else { b[2] };
                items.sort_by(|a, b| (lo(&a.1) + hi(&a.1)).total_cmp(&(lo(&b.1) + hi(&b.1))).then(lo(&a.1).total_cmp(&lo(&b.1))));
                let span = hi(&items[n - 1].1) - lo(&items[0].1);
                let total: f64 = items.iter().map(|(_, b)| hi(b) - lo(b)).sum();
                let gap = (span - total) / (n - 1) as f64;
                let mut cursor = lo(&items[0].1);
                for (range, b) in items {
                    let d = cursor - lo(&b);
                    moves.push((range, if v { [0.0, d] } else { [d, 0.0] }));
                    cursor += hi(&b) - lo(&b) + gap;
                }
            }
        }
        for (range, d) in moves {
            translate(path, range, d);
        }
        Ok(())
    })
}
