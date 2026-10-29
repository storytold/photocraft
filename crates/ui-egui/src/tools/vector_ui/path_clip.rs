//! Edit › Copy / Cut / Paste for path components (Photoshop's Path Selection clipboard):
//! while the Path Selection tool has a component selected, the Edit menu's Copy and Cut take
//! the selected subpaths onto the engine's path clipboard and Paste lays them down centred on
//! the view — appended to the path selected in the Paths panel (as in Photoshop), or as a new
//! saved path "Pasted Path" when nothing is selected there. The Edit commands and shortcuts
//! (⌘C/X/V) keep their ids: this intercepts them in `PhotocraftApp::run_command_at_depth`
//! before the engine dispatch, so menus, the canvas context menu, agents and MCP all agree.

use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use super::paths_panel::{PathEntry, PathRow, path_rows};
use super::{PathSelection, PathTarget, PhotocraftApp, Tool};

/// The document path the selection's target addresses, for the freshness check.
fn selection_path(app: &PhotocraftApp, t: &PathTarget) -> Option<photocraft_doc::vector::Path> {
    let st = app.session.active()?;
    match t {
        PathTarget::Shape(id) => match &st.doc.layer(photocraft_doc::LayerId(*id))?.content {
            LayerContent::Shape(sh) => Some(sh.path.clone()),
            _ => None,
        },
        PathTarget::VectorMask(id) => st.doc.layer(photocraft_doc::LayerId(*id))?.vector_mask.as_ref().map(|m| m.path.clone()),
        PathTarget::Work => st.doc.work_path.clone(),
        PathTarget::Saved(name) => st.doc.paths.iter().find(|q| q.name == *name).map(|q| q.path.clone()),
    }
}

/// The live component selection: `PathSelection` on the active document with at least one
/// subpath and a knot signature that still matches its path (a structural edit since the
/// press made the stored indices stale).
pub(crate) fn live_selection(app: &PhotocraftApp) -> Option<(PathTarget, Vec<usize>)> {
    if app.ui.tool != Tool::PathSelection {
        return None;
    }
    let sel = &app.ui.path_selection;
    let t = sel.target.clone()?;
    if sel.subpaths.is_empty() || app.session.active().is_none_or(|st| st.doc.id.0 != sel.doc) {
        return None;
    }
    let path = selection_path(app, &t)?;
    let knots: Vec<usize> = path.subpaths.iter().map(|s| s.knots.len()).collect();
    (knots == sel.knots).then_some((t, sel.subpaths.clone()))
}

/// Copy/cut target params for the selected components, like `path.moveAnchors`' target.
fn target_params(t: PathTarget, subpaths: &[usize]) -> Value {
    let target = match t {
        PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
        PathTarget::Work => json!({"name": "work"}),
        PathTarget::Saved(name) => json!({"name": name}),
    };
    let mut p = json!({"subpaths": subpaths});
    if let (Some(o), Some(t)) = (p.as_object_mut(), target.as_object()) {
        o.extend(t.clone());
    }
    p
}

/// The document coordinates of the active view's centre: where Photoshop centres a pasted path.
fn view_center(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let xf = crate::canvas::ViewXform::active(app)?;
    Some([f64::from(xf.center[0]), f64::from(xf.center[1])])
}

/// The path row explicitly selected in the Paths panel — where Edit › Paste puts the copied
/// components, as in Photoshop: an existing Work Path / layer-path / saved-path row, as the
/// engine's path commands address it. `None` when nothing (or a stale row) is selected, so
/// the paste becomes a new saved "Pasted Path" instead.
fn panel_target(app: &PhotocraftApp) -> Option<Value> {
    let st = app.session.active()?;
    let sel = app.ui.selected_path.as_deref()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let row = rows.iter().find(|r| key(r) == sel)?;
    Some(match row.kind {
        PathRow::Work => json!({"name": "work"}),
        // The layer row addresses the active layer's shape path (or its vector mask).
        PathRow::Layer => json!({"name": "layer"}),
        PathRow::Saved => json!({"name": row.name}),
    })
}

/// Edit › Copy/Cut/Paste for path components; `None` = not this context, dispatch as usual.
pub(crate) fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "edit.copy" | "edit.cut" => {
            let (t, subpaths) = live_selection(app)?;
            let cmd = if id == "edit.copy" { "path.copy" } else { "path.cut" };
            let r = app.run(cmd, target_params(t, &subpaths));
            if r.is_ok() && cmd == "path.cut" {
                // The indices address the path as it was; the cut changed it.
                app.ui.path_selection = PathSelection::default();
            }
            Some(r)
        }
        "edit.paste" if app.session.path_clipboard.is_some() && params.get("target").is_none() => {
            let mut p = view_center(app).map_or_else(|| json!({}), |c| json!({"center": c}));
            // Photoshop pastes into the path selected in the Paths panel; only with nothing
            // selected does the paste become a new saved "Pasted Path" (selected afterwards).
            let target = panel_target(app);
            if let (Some(o), Some(t)) = (p.as_object_mut(), target.as_ref().and_then(Value::as_object)) {
                o.extend(t.clone());
            }
            let r = app.run("path.paste", p);
            if r.is_ok() && target.is_none() {
                // Photoshop selects the pasted path in the Paths panel.
                app.ui.selected_path = Some("Pasted Path".into());
            }
            Some(r)
        }
        _ => None,
    }
}
