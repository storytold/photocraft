use super::{Map, PathOptions, PhotocraftApp, Show, SnapTo, Value, ViewOptions};

/// Pixel aspect ratio (width / height of a pixel) of a preset id.
pub fn pixel_aspect_ratio(id: &str) -> f32 {
    // View › Pixel Aspect Ratio › Custom: `custom:<ratio>:<name>` (see `workspace_ui`).
    if let Some(r) = id.strip_prefix("custom:").and_then(|r| r.split(':').next()?.parse::<f32>().ok()) {
        return r;
    }
    match id {
        "d1DvNtsc" => 0.91,
        "d1DvPal" => 1.09,
        "d1DvNtscWidescreen" => 1.21,
        "hdv1080" => 1.33,
        "d1DvPalWidescreen" => 1.46,
        "dvcproHd1080" => 1.5,
        "anamorphic2To1" => 2.0,
        _ => 1.0,
    }
}

pub(super) fn show_slot<'a>(s: &'a mut Show, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "layerEdges" => &mut s.layer_edges,
        "selectionEdges" => &mut s.selection_edges,
        "targetPath" => &mut s.target_path,
        "pixelGrid" => &mut s.pixel_grid,
        "smartGuides" => &mut s.smart_guides,
        "brushPreview" => &mut s.brush_preview,
        "slices" => &mut s.slices,
        "notes" => &mut s.notes,
        "mesh" => &mut s.mesh,
        "editPins" => &mut s.edit_pins,
        "count" => &mut s.count,
        "canvasGuides" => &mut s.canvas_guides,
        "artboardGuides" => &mut s.artboard_guides,
        _ => return None,
    })
}

pub(super) fn snap_slot<'a>(s: &'a mut SnapTo, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "guides" => &mut s.guides,
        "grid" => &mut s.grid,
        "layers" => &mut s.layers,
        "slices" => &mut s.slices,
        "documentBounds" => &mut s.document_bounds,
        _ => return None,
    })
}

/// Every [`show_slot`] key.
const SHOW_KEYS: [&str; 13] = [
    "layerEdges",
    "selectionEdges",
    "targetPath",
    "pixelGrid",
    "smartGuides",
    "brushPreview",
    "slices",
    "notes",
    "mesh",
    "editPins",
    "count",
    "canvasGuides",
    "artboardGuides",
];

/// Every [`snap_slot`] key.
const SNAP_KEYS: [&str; 5] = ["guides", "grid", "layers", "slices", "documentBounds"];

/// The flags of `now` that differ from `def`, by key (`None` when none do).
fn changed_flags<T: Copy>(now: T, def: T, keys: &[&str], slot: impl for<'a> Fn(&'a mut T, &str) -> Option<&'a mut bool>) -> Option<Value> {
    let (mut now, mut def) = (now, def);
    let diff: Map<String, Value> = keys
        .iter()
        .filter_map(|k| {
            let v = *slot(&mut now, k)?;
            (Some(v) != slot(&mut def, k).copied()).then(|| ((*k).to_string(), Value::Bool(v)))
        })
        .collect();
    (!diff.is_empty()).then_some(Value::Object(diff))
}

/// The Path Options as remembered (#9713): the colour as `[r, g, b]` and the thickness, each only
/// when set (`None` when neither is).
fn changed_path_options(now: &PathOptions) -> Option<Value> {
    let mut m = Map::new();
    if let Some(rgb) = now.color {
        m.insert("color".into(), serde_json::json!(rgb));
    }
    if let Some(px) = now.thickness {
        m.insert("thickness".into(), serde_json::json!(px));
    }
    (!m.is_empty()).then_some(Value::Object(m))
}

/// A remembered Path Options colour: three bytes, `[r, g, b]`.
fn saved_color(v: &Value) -> Option<[u8; 3]> {
    let rgb = v.as_array().filter(|a| a.len() == 3)?;
    let byte = |i: usize| rgb[i].as_u64().and_then(|b| u8::try_from(b).ok());
    Some([byte(0)?, byte(1)?, byte(2)?])
}

/// Restore the remembered Path Options (#9713): a colour that isn't three bytes, or a thickness
/// that isn't a number in the popover's range, keeps its default.
fn restore_path_options(o: &mut PathOptions, saved: &Value) {
    if let Some(rgb) = saved.get("color").and_then(saved_color) {
        o.color = Some(rgb);
    }
    if let Some(px) = saved.get("thickness").and_then(Value::as_f64).map(|px| px as f32).filter(|px| crate::vector_ui::thickness_in_range(*px)) {
        o.thickness = Some(px);
    }
}

/// View › Extras, Show and Snap To, and the Path Options, as remembered across restarts
/// (`Preferences::view_extras`): only what differs from the defaults, so a default changed in a
/// later release still reaches users who never touched that item. `Null` when nothing differs.
fn remembered(app: &PhotocraftApp) -> Value {
    let (v, e, def) = (&app.ui.view, &app.ui.extras, crate::state::Extras::default());
    let mut m = Map::new();
    if v.extras != ViewOptions::default().extras {
        m.insert("extras".into(), Value::Bool(v.extras));
    }
    // View › Show › Grid and Guides live in `UiState::extras` beside the rulers and snapping.
    if e.grid != def.grid {
        m.insert("grid".into(), Value::Bool(e.grid));
    }
    if e.guides != def.guides {
        m.insert("guides".into(), Value::Bool(e.guides));
    }
    if e.snap != def.snap {
        m.insert("snap".into(), Value::Bool(e.snap));
    }
    if let Some(show) = changed_flags(v.show, Show::default(), &SHOW_KEYS, show_slot) {
        m.insert("show".into(), show);
    }
    if let Some(snap) = changed_flags(v.snap_to, SnapTo::default(), &SNAP_KEYS, snap_slot) {
        m.insert("snapTo".into(), snap);
    }
    if let Some(paths) = changed_path_options(&v.path_options) {
        m.insert("pathOptions".into(), paths);
    }
    if m.is_empty() { Value::Null } else { Value::Object(m) }
}

/// Remember View › Extras, Show and Snap To in the preferences once the user lets go of the
/// pointer (#3762). Cheap: a small JSON compare per frame. Run every frame, so items set by the
/// control channel or the Show Extras Options dialog are remembered too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = remembered(app);
    if app.session.prefs().view_extras != now {
        app.session.prefs.edit(|p| p.view_extras = now);
    }
}

/// Restore the remembered View › Extras, Show and Snap To items and the Path Options at launch. A
/// hand-edited file can't break them: an unknown key or a value that isn't a boolean (for the Path
/// Options, a colour or thickness that doesn't read) keeps its default.
pub fn restore(app: &mut PhotocraftApp) {
    adopt_tool_path_options(app);
    let saved = app.session.prefs().view_extras.clone();
    if !saved.is_null() && !saved.is_object() {
        log::warn!("Preferences: the remembered View › Show items are not an object; using the defaults");
        return;
    }
    let flag = |k: &str| saved.get(k).and_then(Value::as_bool);
    if let Some(on) = flag("extras") {
        app.ui.view.extras = on;
    }
    if let Some(on) = flag("grid") {
        app.ui.extras.grid = on;
    }
    if let Some(on) = flag("guides") {
        app.ui.extras.guides = on;
    }
    if let Some(on) = flag("snap") {
        app.ui.extras.snap = on;
    }
    for (k, v) in saved.get("show").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(slot), Some(on)) = (show_slot(&mut app.ui.view.show, k), v.as_bool()) {
            *slot = on;
        }
    }
    for (k, v) in saved.get("snapTo").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(slot), Some(on)) = (snap_slot(&mut app.ui.view.snap_to, k), v.as_bool()) {
            *slot = on;
        }
    }
    if let Some(paths) = saved.get("pathOptions") {
        restore_path_options(&mut app.ui.view.path_options, paths);
    }
}

/// An earlier build kept the Path Options' Color and Thickness with the Pen's tool options, which
/// `tool_memory::restore` has just loaded: they move to the shared display setting, where the
/// remembered View items, restored after this, win. The tool options keep only the Rubber Band.
fn adopt_tool_path_options(app: &mut PhotocraftApp) {
    let tool = &mut app.ui.tool_options.path_options;
    let (color, thickness) = (tool.color.take(), tool.thickness.take());
    let view = &mut app.ui.view.path_options;
    view.color = view.color.or(color);
    view.thickness = view.thickness.or(thickness.filter(|px| crate::vector_ui::thickness_in_range(*px)));
}
