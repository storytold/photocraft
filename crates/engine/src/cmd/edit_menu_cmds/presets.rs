use super::{CustomShape, Deserialize, Document, EngineError, LayerContent, LayerId, Path, Result, Serialize, Session, Value, bad, json, str_or};

// ------------------------------------------------------------------ Define Brush / Custom Shape

pub(super) fn define_brush_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let name = match p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => {
            let n = s.tools.presets.iter().filter(|b| b.name.starts_with("Sampled Brush")).count() + 1;
            format!("Sampled Brush {n}")
        }
    };
    s.execute("brush.defineFromSelection", json!({"name": name}))
}

/// The path Define Custom Shape uses: `"work"`, a saved path name, else the work path, the
/// active shape layer, the active layer's vector mask, or the last saved path.
pub(crate) fn current_path(doc: &Document, active: Option<LayerId>, spec: Option<&str>) -> Option<Path> {
    let ok = |p: &Path| !p.subpaths.is_empty();
    match spec {
        Some("work") => return doc.work_path.clone().filter(ok),
        Some(name) => return doc.paths.iter().find(|n| n.name == name).map(|n| n.path.clone()).filter(ok),
        None => {}
    }
    if let Some(p) = doc.work_path.clone().filter(ok) {
        return Some(p);
    }
    if let Some(l) = active.and_then(|id| doc.layer(id)) {
        if let LayerContent::Shape(sh) = &l.content
            && ok(&sh.path)
        {
            return Some(sh.path.clone());
        }
        if let Some(vm) = l.vector_mask.as_ref().filter(|v| ok(&v.path)) {
            return Some(vm.path.clone());
        }
    }
    doc.paths.last().map(|n| n.path.clone()).filter(ok)
}

pub(super) fn can_define_shape(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    current_path(&d.doc, d.active_layer, None).map(|_| ()).ok_or_else(|| "select a path or shape layer first".into())
}

pub(super) fn define_custom_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let path = current_path(&st.doc, st.active_layer, p.get("path").and_then(Value::as_str)).ok_or_else(|| bad("edit.defineCustomShape", "no such path"))?;
    let path = plain_outlines(path);
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Shape {}", s.edit_state.custom_shapes.len() + 1));
    let shapes = &mut s.edit_state.custom_shapes;
    match shapes.iter_mut().find(|c| c.name == name) {
        Some(c) => c.path = path,
        None => shapes.push(CustomShape { name: name.clone(), path }),
    }
    Ok(json!({"name": name, "count": shapes.len()}))
}

/// A custom shape is stored as plain outlines in one shape component, as in Photoshop (#14294): the path's operations
/// are applied first. Straight-edged paths are combined exactly, so a square keeps 4 anchors. A single subpath (an
/// ellipse keeps its smooth anchors), an open subpath, curves combined with operations and a path that combines to
/// nothing are stored as they are, as before.
fn plain_outlines(path: Path) -> Path {
    if path.subpaths.len() < 2 {
        return path;
    }
    outlines::combined(&path).unwrap_or(path)
}

/// The exact combination of a straight-edged path's subpath operations.
mod outlines {
    use std::collections::BTreeMap;

    use photocraft_doc::{FillRule, Knot, Path, PathOp, Subpath};

    type Pt = (f64, f64);
    type Key = (i64, i64);

    /// More edges than this keep the path as it is (the pairwise split is quadratic).
    const MAX_EDGES: usize = 2000;
    /// Parameter slack along an edge for a crossing at its end.
    const T: f64 = 1e-9;
    /// The most winding-edge evaluations the boundary classification may take (pieces x edges x the samples per piece);
    /// beyond it the path stays as it is instead of blocking the command (CodeRabbit on #16189).
    const WORK_BUDGET: usize = 20_000_000;
    /// The sampling distances beside a piece: a boundary counts only when both give the same answer, so a feature
    /// narrower than the larger one is left alone rather than misread.
    const SAMPLE_OFFSETS: [f64; 2] = [1e-4, 1e-6];

    fn sub(a: Pt, b: Pt) -> Pt {
        (a.0 - b.0, a.1 - b.1)
    }
    fn cross(a: Pt, b: Pt) -> f64 {
        a.0 * b.1 - a.1 * b.0
    }
    fn dot(a: Pt, b: Pt) -> f64 {
        a.0 * b.0 + a.1 * b.1
    }
    fn key(p: Pt) -> Key {
        ((p.0 * 1e6).round() as i64, (p.1 * 1e6).round() as i64)
    }

    /// The plain outlines of `path`: one component, a combined first subpath and joined others whose winding cuts the
    /// holes. `None` when the path has curves or open subpaths, is too large, or covers nothing.
    pub(super) fn combined(path: &Path) -> Option<Path> {
        let straight = |k: &Knot| k.in_ctrl == k.anchor && k.out_ctrl == k.anchor;
        if path.subpaths.iter().any(|sp| !sp.closed || !sp.knots.iter().all(straight)) {
            return None;
        }
        let polys: Vec<Vec<Pt>> = path.subpaths.iter().map(|sp| sp.knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()).collect();
        let edges: Vec<(Pt, Pt)> =
            polys.iter().flat_map(|p| (0..p.len()).map(move |i| (p[i], p[(i + 1) % p.len()]))).filter(|(a, b)| key(*a) != key(*b)).collect();
        if edges.is_empty() || edges.len() > MAX_EDGES || edges.iter().any(|(a, b)| ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite() && v.abs() < 1e9)) {
            return None;
        }
        // Split every edge where another crosses or touches it.
        let mut cuts: Vec<Vec<(f64, Pt)>> = edges.iter().map(|&(a, b)| vec![(0.0, a), (1.0, b)]).collect();
        for i in 0..edges.len() {
            for j in i + 1..edges.len() {
                for (t, u, p) in meets(edges[i], edges[j]) {
                    cuts[i].push((t, p));
                    cuts[j].push((u, p));
                }
            }
        }
        let mut pieces: BTreeMap<(Key, Key), (Pt, Pt)> = BTreeMap::new();
        for mut c in cuts {
            c.sort_by(|a, b| a.0.total_cmp(&b.0));
            for w in c.windows(2) {
                let (a, b) = (w[0].1, w[1].1);
                let (ka, kb) = (key(a), key(b));
                if ka != kb {
                    pieces.entry(if ka < kb { (ka, kb) } else { (kb, ka) }).or_insert((a, b));
                }
            }
        }
        // The classification evaluates every edge twice per piece and sampling distance: a path whose pieces would take
        // more than the budget is stored as it is.
        if pieces.len().saturating_mul(edges.len()).saturating_mul(2 * SAMPLE_OFFSETS.len() + 1) > WORK_BUDGET {
            return None;
        }
        // Keep the pieces with the shape on one side only, running with the shape on their left.
        let mut kept: Vec<(Pt, Pt)> = Vec::new();
        for (a, b) in pieces.into_values() {
            let d = sub(b, a);
            let len = dot(d, d).sqrt();
            let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            // Another boundary within a few sampling distances of this piece (a hole or a gap narrower than that, so
            // that both samples can land on the far side of it) makes the samples meaningless, even when they agree:
            // the path stays as it is (CodeRabbit on #16189). Edges through the midpoint itself are the piece's own.
            let crowded = edges.iter().any(|&(p, q)| {
                let d = distance_to_edge(m, p, q);
                d > 1e-9 && d < 3.0 * SAMPLE_OFFSETS[0]
            });
            if crowded {
                return None;
            }
            let sides: Vec<(bool, bool)> = SAMPLE_OFFSETS
                .iter()
                .map(|eps| {
                    let n = (d.1 / len * eps, -d.0 / len * eps);
                    (inside(path, &polys, (m.0 + n.0, m.1 + n.1)), inside(path, &polys, (m.0 - n.0, m.1 - n.1)))
                })
                .collect();
            // Samples that disagree can't classify this boundary reliably (a hole narrower than the larger offset).
            if sides.windows(2).any(|w| w[0] != w[1]) {
                return None;
            }
            match sides[0] {
                (true, false) => kept.push((a, b)),
                (false, true) => kept.push((b, a)),
                _ => {}
            }
        }
        let loops = chain(&kept)?;
        if loops.is_empty() {
            return None;
        }
        let subpaths = loops
            .into_iter()
            .enumerate()
            .map(|(i, pts)| Subpath {
                closed: true,
                knots: pts.iter().map(|&(x, y)| Knot::corner(x, y)).collect(),
                op: if i == 0 { PathOp::Combine } else { PathOp::Join },
            })
            .collect();
        Some(Path { subpaths, ..path.clone() })
    }

    /// Where edges `a` and `b` meet: (parameter on `a`, parameter on `b`, point). A crossing at an edge's end uses that
    /// end's own point, so both edges split at the same spot; collinear edges meet at each other's ends.
    fn meets(a: (Pt, Pt), b: (Pt, Pt)) -> Vec<(f64, f64, Pt)> {
        let (r, s) = (sub(a.1, a.0), sub(b.1, b.0));
        let (rr, ss) = (dot(r, r), dot(s, s));
        let qp = sub(b.0, a.0);
        let rxs = cross(r, s);
        if rxs.abs() <= 1e-12 * rr.sqrt() * ss.sqrt() {
            if cross(qp, r).abs() > 1e-7 * rr.sqrt() {
                return Vec::new();
            }
            let mut out = Vec::new();
            for (u, p) in [(0.0, b.0), (1.0, b.1)] {
                let t = dot(sub(p, a.0), r) / rr;
                if t > T && t < 1.0 - T {
                    out.push((t, u, p));
                }
            }
            for (t, p) in [(0.0, a.0), (1.0, a.1)] {
                let u = dot(sub(p, b.0), s) / ss;
                if u > T && u < 1.0 - T {
                    out.push((t, u, p));
                }
            }
            return out;
        }
        let (t, u) = (cross(qp, s) / rxs, cross(qp, r) / rxs);
        if !(-T..=1.0 + T).contains(&t) || !(-T..=1.0 + T).contains(&u) {
            return Vec::new();
        }
        let p = if t <= T {
            a.0
        } else if t >= 1.0 - T {
            a.1
        } else if u <= T {
            b.0
        } else if u >= 1.0 - T {
            b.1
        } else {
            (a.0.0 + t * r.0, a.0.1 + t * r.1)
        };
        vec![(t.clamp(0.0, 1.0), u.clamp(0.0, 1.0), p)]
    }

    /// The distance from `p` to the segment `a`-`b`.
    fn distance_to_edge(p: Pt, a: Pt, b: Pt) -> f64 {
        let d = sub(b, a);
        let len2 = dot(d, d);
        let t = if len2 > 0.0 { (dot(sub(p, a), d) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let c = (a.0 + d.0 * t, a.1 + d.1 * t);
        let e = sub(p, c);
        dot(e, e).sqrt()
    }

    /// Whether `p` is inside `path`, folding its components' operations as the renderer does (inversion aside).
    fn inside(path: &Path, polys: &[Vec<Pt>], p: Pt) -> bool {
        let mut acc = false;
        for c in path.components() {
            let w: i32 = polys[c.clone()].iter().map(|poly| winding(poly, p)).sum();
            let v = match path.fill_rule {
                FillRule::NonZero => w != 0,
                FillRule::EvenOdd => w & 1 != 0,
            };
            acc = match path.effective_op(c.start) {
                PathOp::Combine | PathOp::Join => acc | v,
                PathOp::Subtract => acc & !v,
                PathOp::Intersect => acc & v,
                PathOp::Exclude => acc ^ v,
            };
        }
        acc
    }

    fn winding(poly: &[Pt], p: Pt) -> i32 {
        let mut w = 0;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let side = cross(sub(b, a), sub(p, a));
            if a.1 <= p.1 {
                if b.1 > p.1 && side > 0.0 {
                    w += 1;
                }
            } else if b.1 <= p.1 && side < 0.0 {
                w -= 1;
            }
        }
        w
    }

    /// Joins the directed edges into closed loops, turning furthest left where several leave one point so two outlines
    /// that touch at a corner stay two; points along a straight run are dropped. `None` if an edge leads nowhere.
    fn chain(edges: &[(Pt, Pt)]) -> Option<Vec<Vec<Pt>>> {
        let mut from: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
        for (i, (a, _)) in edges.iter().enumerate() {
            from.entry(key(*a)).or_default().push(i);
        }
        let mut used = vec![false; edges.len()];
        let mut loops = Vec::new();
        for first in 0..edges.len() {
            if used[first] {
                continue;
            }
            let mut pts = vec![edges[first].0];
            let mut at = first;
            for _ in 0..=edges.len() {
                used[at] = true;
                let (a, b) = edges[at];
                let d_in = sub(b, a);
                let next = from
                    .get(&key(b))?
                    .iter()
                    .copied()
                    .filter(|&e| !used[e] || e == first)
                    .min_by(|&x, &y| turn(d_in, edges[x]).total_cmp(&turn(d_in, edges[y])))?;
                if next == first {
                    break;
                }
                pts.push(edges[next].0);
                at = next;
            }
            if key(edges[at].1) != key(edges[first].0) {
                return None;
            }
            let pts = straighten(pts);
            if pts.len() >= 3 {
                loops.push(pts);
            }
        }
        Some(loops)
    }

    /// The turn from direction `d_in` onto `e`, most negative furthest left (y down).
    fn turn(d_in: Pt, e: (Pt, Pt)) -> f64 {
        let d = sub(e.1, e.0);
        cross(d_in, d).atan2(dot(d_in, d))
    }

    /// `pts` without the points in the middle of a straight run.
    fn straighten(mut pts: Vec<Pt>) -> Vec<Pt> {
        loop {
            let n = pts.len();
            if n < 3 {
                return pts;
            }
            let Some(i) = (0..n).find(|&i| {
                let (p, q, r) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
                let (u, v) = (sub(q, p), sub(r, q));
                cross(u, v).abs() <= 1e-9 * dot(u, u).sqrt() * dot(v, v).sqrt() && dot(u, v) > 0.0
            }) else {
                return pts;
            };
            pts.remove(i);
        }
    }
}

// ------------------------------------------------------------------ presets

/// Libraries the Preset Manager edits (patterns export as `.pat` with `pattern.export`).
const PRESET_KINDS: [&str; 3] = ["brushes", "customShapes", "patterns"];

fn preset_names(s: &Session, kind: &str) -> Option<Vec<String>> {
    Some(match kind {
        "brushes" => s.tools.presets.iter().map(|b| b.name.clone()).collect(),
        "customShapes" => s.edit_state.custom_shapes.iter().map(|c| c.name.clone()).collect(),
        "patterns" => s.patterns.items.iter().map(|p| p.name.clone()).collect(),
        _ => return None,
    })
}

fn preset_index(s: &Session, kind: &str, p: &Value) -> Result<usize> {
    let names = preset_names(s, kind).ok_or_else(|| bad("edit.presets.presetManager", format!("unknown preset kind `{kind}` ({})", PRESET_KINDS.join("|"))))?;
    if let Some(i) = p.get("index").and_then(Value::as_u64) {
        return if (i as usize) < names.len() { Ok(i as usize) } else { Err(bad("edit.presets.presetManager", format!("no preset {i}"))) };
    }
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad("edit.presets.presetManager", "give `index` or `name`"))?;
    names.iter().position(|n| n == name).ok_or_else(|| bad("edit.presets.presetManager", format!("no preset named `{name}`")))
}

pub(super) fn preset_manager(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.presets.presetManager";
    let action = str_or(p, "action", "list");
    if action == "list" {
        let kinds: Vec<&str> = match p.get("kind").and_then(Value::as_str) {
            Some(k) => vec![k],
            None => PRESET_KINDS.to_vec(),
        };
        let mut out = serde_json::Map::new();
        for k in kinds {
            out.insert(k.to_string(), json!(preset_names(s, k).ok_or_else(|| bad(cmd, format!("unknown preset kind `{k}`")))?));
        }
        return Ok(Value::Object(out));
    }
    let kind = p.get("kind").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `kind`"))?.to_string();
    let i = preset_index(s, &kind, p)?;
    match action {
        "rename" => {
            let new =
                p.get("newName").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "missing `newName`"))?.to_string();
            match kind.as_str() {
                // Same rules as Brush Settings: no duplicate names (case-insensitive), the current
                // preset follows, a renamed built-in is the user's preset now (and persists as one).
                "brushes" => crate::brush_preset_cmds::rename_at(s, i, &new, cmd)?,
                "patterns" => s.patterns.items[i].name = new,
                _ => s.edit_state.custom_shapes[i].name = new,
            }
        }
        "delete" => match kind.as_str() {
            "brushes" => {
                let gone = std::sync::Arc::make_mut(&mut *s.tools.presets).remove(i);
                crate::brush_cmds::forget_deleted_preset(s, &gone.name);
            }
            "patterns" => drop(s.patterns.items.remove(i)),
            _ => drop(s.edit_state.custom_shapes.remove(i)),
        },
        "move" => {
            let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `to`"))? as usize;
            match kind.as_str() {
                "brushes" => crate::move_item(std::sync::Arc::make_mut(&mut *s.tools.presets), i, to),
                "patterns" => crate::move_item(&mut s.patterns.items, i, to),
                _ => crate::move_item(&mut s.edit_state.custom_shapes, i, to),
            };
        }
        other => return Err(bad(cmd, format!("unknown action `{other}` (list|rename|delete|move)"))),
    }
    if kind == "brushes" {
        s.brush_presets_changed();
    }
    Ok(json!({kind.clone(): preset_names(s, &kind)}))
}

/// Exported preset file (`.pcpresets`, JSON).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PresetFile {
    pub format: String,
    pub version: u32,
    pub brushes: Vec<photocraft_paint::BrushPreset>,
    pub custom_shapes: Vec<CustomShape>,
    pub tool_presets: Vec<crate::presets::tools::ToolPreset>,
}

pub const PRESET_FORMAT: &str = "photocraft-presets";
