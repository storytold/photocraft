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

/// What Define Custom Shape refuses to store, as in Photoshop (#14295).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeRefusal {
    /// A path with no area (a single anchor, a straight line): an alert says so ([`EMPTY_PATH`]).
    Empty,
    /// A path whose every subpath subtracts: nothing happens, with no alert.
    SubtractOnly,
}

/// The alert of a Define Custom Shape refused for a path with no area.
pub const EMPTY_PATH: &str = "Could not define the custom shape because the current path is empty.";

/// Why `path` can't be a custom shape, if it can't.
fn refusal(path: &Path) -> Option<ShapeRefusal> {
    if !path.subpaths.is_empty() && path.subpaths.iter().all(|sp| sp.op == photocraft_doc::PathOp::Subtract) {
        return Some(ShapeRefusal::SubtractOnly);
    }
    // Not measured, so they keep today's behaviour and are stored: a closed path of two anchors, and a path whose every
    // subpath intersects (CodeRabbit on #16238).
    if path.subpaths.iter().any(|sp| sp.closed && sp.knots.len() == 2)
        || (!path.subpaths.is_empty() && path.subpaths.iter().all(|sp| sp.op == photocraft_doc::PathOp::Intersect))
    {
        return None;
    }
    // Every flattened point on one line (or one point): nothing is enclosed. Not the signed area, which cancels for a
    // self-crossing outline like a bow tie that does cover pixels (CodeRabbit on #16238).
    let lines = photocraft_vector::flatten_path(path, 0.05);
    let mut pts: Vec<(f64, f64)> = lines.iter().flat_map(|l| l.pts.iter().copied()).collect();
    // A curve flattened to one piece keeps only its end points, so the handles that shape each segment count too: a path
    // whose anchors are on a line but whose handles bulge encloses area (CodeRabbit on #16238).
    for sp in &path.subpaths {
        let n = sp.knots.len();
        let segments = if sp.closed { n } else { n.saturating_sub(1) };
        for i in 0..segments {
            let (from, to) = (&sp.knots[i], &sp.knots[(i + 1) % n]);
            pts.extend([(from.out_ctrl.x, from.out_ctrl.y), (to.in_ctrl.x, to.in_ctrl.y)]);
        }
    }
    let Some(&p0) = pts.first() else { return Some(ShapeRefusal::Empty) };
    if pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Some(ShapeRefusal::Empty);
    }
    let far = pts.iter().copied().max_by(|a, b| ((a.0 - p0.0).hypot(a.1 - p0.1)).total_cmp(&((b.0 - p0.0).hypot(b.1 - p0.1)))).unwrap_or(p0);
    let len = (far.0 - p0.0).hypot(far.1 - p0.1);
    let off_line =
        if len <= 1e-9 { 0.0 } else { pts.iter().map(|p| ((p.0 - p0.0) * (far.1 - p0.1) - (p.1 - p0.1) * (far.0 - p0.0)).abs() / len).fold(0.0, f64::max) };
    // Collinear to floating-point noise only: a few dozen units in the last place of the largest coordinate, which
    // grows with the coordinates, not with the length of the line: a thin rectangle has area and is stored, however narrow and however long.
    let magnitude = pts.iter().fold(0.0_f64, |m, p| m.max(p.0.abs()).max(p.1.abs()));
    (off_line <= 64.0 * f64::EPSILON * (1.0 + magnitude)).then_some(ShapeRefusal::Empty)
}

/// What Define Custom Shape would refuse for the path it would use on the active document (the menu
/// checks this before opening its dialog). `None` when it would store a shape, or has no path.
pub fn define_custom_shape_refusal(s: &Session) -> Option<ShapeRefusal> {
    let d = s.active()?;
    refusal(&current_path(&d.doc, d.active_layer, None)?)
}

pub(super) fn define_custom_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(r) = current_path(&st.doc, st.active_layer, p.get("path").and_then(Value::as_str)).as_ref().and_then(refusal) {
        return Err(match r {
            ShapeRefusal::Empty => EngineError::Other(EMPTY_PATH.into()),
            ShapeRefusal::SubtractOnly => bad("edit.defineCustomShape", "the path only subtracts: there is no shape to define"),
        });
    }
    let path = current_path(&st.doc, st.active_layer, p.get("path").and_then(Value::as_str)).ok_or_else(|| bad("edit.defineCustomShape", "no such path"))?;
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
