use super::{CustomShape, Deserialize, Document, EngineError, LayerContent, LayerId, Path, Result, Serialize, Session, Value, bad, json, str_or};
use crate::preset_store::PresetStoreExt;
use photocraft_doc::vector::PathOp;

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
    if let Some(p) = layer_path(doc, active) {
        return Some(p.clone());
    }
    doc.paths.last().map(|n| n.path.clone()).filter(ok)
}

/// The active layer's shape path, else its vector mask's path, when not empty.
fn layer_path(doc: &Document, active: Option<LayerId>) -> Option<&Path> {
    let ok = |p: &&Path| !p.subpaths.is_empty();
    let l = active.and_then(|id| doc.layer(id))?;
    let shape = match &l.content {
        LayerContent::Shape(sh) => Some(&sh.path).filter(ok),
        _ => None,
    };
    shape.or_else(|| l.vector_mask.as_ref().map(|v| &v.path).filter(ok))
}

/// Whether `p`'s area is the union of its shape components: no subpath after the first
/// subtracts, intersects or excludes (the first one always acts as "combine").
fn only_adds(p: &Path) -> bool {
    p.subpaths.iter().skip(1).all(|s| matches!(s.op, PathOp::Combine | PathOp::Join))
}

/// `targeted` stored together with the active layer's path, at their document positions, so the
/// stored shape fills the area of each. Subpath operations fold in order, so a part that
/// subtracts, intersects or excludes goes first and the part that only adds follows it as a shape
/// of its own; every subpath is closed. `None` when no order keeps both areas: both parts cut, one
/// is inverted, or their fill rules differ.
fn with_layer_path(layer: &Path, targeted: Path) -> Option<Path> {
    if layer.inverted || targeted.inverted || layer.fill_rule != targeted.fill_rule {
        return None;
    }
    let (mut out, rest) = if only_adds(&targeted) {
        (layer.clone(), targeted)
    } else if only_adds(layer) {
        (targeted, layer.clone())
    } else {
        return None;
    };
    let first = out.subpaths.len();
    out.subpaths.extend(rest.subpaths);
    if let Some(s) = out.subpaths.get_mut(first) {
        s.op = PathOp::Combine;
    }
    for s in &mut out.subpaths {
        s.closed = true;
    }
    Some(out)
}

pub(super) fn can_define_shape(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    current_path(&d.doc, d.active_layer, None).map(|_| ()).ok_or_else(|| "select a path or shape layer first".into())
}

pub(super) fn define_custom_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let spec = p.get("path").and_then(Value::as_str);
    let path = current_path(&st.doc, st.active_layer, spec).ok_or_else(|| bad("edit.defineCustomShape", "no such path"))?;
    // A targeted path is stored together with the active shape layer's path or vector mask.
    let path = match spec.and(layer_path(&st.doc, st.active_layer)) {
        Some(layer) => with_layer_path(layer, path).ok_or_else(|| {
            bad(
                "edit.defineCustomShape",
                "the targeted path and the active layer's path can't be stored as one shape: both subtract, intersect or exclude, one is inverted, or their fill rules differ",
            )
        })?,
        None => path,
    };
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
                crate::preset_store::forget_deleted_preset(s, &gone.name);
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

#[cfg(test)]
#[path = "define_custom_shape_combine_tests.rs"]
mod define_custom_shape_combine_tests;
