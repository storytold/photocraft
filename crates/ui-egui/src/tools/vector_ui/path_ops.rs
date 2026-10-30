use super::paths_panel::{PathEntry, PathRow, path_rows};
use super::pen::pen_commit;
use super::{Document, LayerContent, Path, PhotocraftApp, Tool, Value, json};
use serde::{Deserialize, Serialize};

/// The path that path commands act on without a name: the one selected in the Paths panel, else
/// the work path while it is shown (not after Make Selection hid it, #6079), else the selected
/// layer's shape path or vector mask; `None` when there's none.
pub fn active_path_name(app: &PhotocraftApp) -> Option<String> {
    let st = app.session.active()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let selected = app.ui.selected_path.as_deref().filter(|s| rows.iter().any(|r| key(r) == *s)).map(str::to_string);
    let work = rows.iter().find(|r| r.kind == PathRow::Work).filter(|_| work_path_shown(app)).map(key);
    selected.or(work).or_else(|| rows.iter().find(|r| r.kind == PathRow::Layer).map(key))
}

/// The commands that create a fill or adjustment layer and take a `"path"` as its vector mask.
pub fn takes_path_mask(id: &str) -> bool {
    id.starts_with("layer.newFillLayer.") || id.starts_with("layer.newAdjustmentLayer.")
}

/// The work or saved path explicitly selected in the Paths panel (not a layer's own path).
pub fn selected_document_path(app: &PhotocraftApp) -> Option<String> {
    let st = app.session.active()?;
    let sel = app.ui.selected_path.as_deref()?;
    let listed = path_rows(&st.doc, st.active_layer).iter().any(|r| match r.kind {
        PathRow::Work => sel == "work",
        PathRow::Saved => r.name == sel,
        PathRow::Layer => false,
    });
    listed.then(|| sel.to_string())
}

/// New fill and adjustment layers take the path selected in the Paths panel as their vector mask,
/// as in Photoshop (#1419): the command gets it as `"path"` unless the caller set one (`null`
/// opts out). Only an explicitly selected work or saved path counts; deselect it by clicking the
/// panel's empty area.
pub fn with_active_path(app: &PhotocraftApp, id: &str, params: Value) -> Value {
    if !takes_path_mask(id) || params.get("path").is_some() {
        return params;
    }
    let Some(sel) = selected_document_path(app) else { return params };
    match params {
        Value::Object(mut m) => {
            m.insert("path".into(), json!(sel));
            Value::Object(m)
        }
        _ => json!({ "path": sel }),
    }
}

/// ⌘↩ / Ctrl+Enter (#306): load a path as a selection, like Photoshop with a Pen or Path
/// Selection tool or a path selected in the Paths panel. A path being drawn with the Pen is
/// finished first and loads; otherwise [`active_path_name`]. `params` may set feather, mode…
pub fn path_to_selection(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) {
        // The finished path becomes the work path, or a shape layer's path or a vector mask.
        let on_layer = app.ui.tool_options.vector_mode == "shape" || targeted_vector_mask(app).is_some();
        pen_commit(app, false);
        app.ui.selected_path = Some(if on_layer { "layer" } else { "work" }.into());
    }
    let name = active_path_name(app)
        .ok_or_else(|| tl!("No path to make a selection from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("name".into(), json!(name));
    let r = app.run("path.toSelection", Value::Object(p))?;
    deselect_loaded_path(app, &name);
    Ok(r)
}

/// After the Pen or Path Selection tool made a selection from the path `name` (⌘↩, the canvas
/// menu's Make Selection…), Photoshop deselects it in the Paths panel and stops drawing it; the
/// path itself stays (#6079). The Work Path is hidden too when a saved path was loaded, so no
/// path is the target path afterwards. A shape layer's path or vector mask (`"layer"`) stays the active
/// path, so layer targeting is unchanged.
pub(crate) fn deselect_loaded_path(app: &mut PhotocraftApp, name: &str) {
    if name == "layer" {
        return;
    }
    app.ui.selected_path = None;
    // Whichever path was loaded, nothing is the target path afterwards: a shown Work Path would
    // otherwise become the active path (a second ⌘↩ or the canvas menu's Delete Path acting on it).
    app.ui.hidden_work_path = app.session.active().and_then(|st| st.doc.work_path.clone().map(|p| (st.doc.id.0, p)));
}

/// Is the active document's Work Path drawn and edited on the canvas? Not after
/// [`deselect_loaded_path`] hid it, until the Work Path is selected again in the Paths panel (which
/// ends the hide; a saved path or a layer's path doesn't show it) or it changes (a new one drawn, Make Work Path,
/// `path.set`…) (#6079).
pub(crate) fn work_path_shown(app: &PhotocraftApp) -> bool {
    app.session.active().is_none_or(|st| work_path_shown_in(app, &st.doc))
}

/// [`work_path_shown`] for `doc`, which the canvas may draw while another document is already
/// active (the frame a click into another arranged tile activates it). The Paths panel's
/// selection belongs to the active document only.
pub(crate) fn work_path_shown_in(app: &PhotocraftApp, doc: &Document) -> bool {
    let active = app.session.active().is_some_and(|st| st.doc.id == doc.id);
    if active && app.ui.selected_path.as_deref() == Some("work") {
        return true;
    }
    let Some((id, hidden)) = &app.ui.hidden_work_path else { return true };
    doc.id.0 != *id || doc.work_path.as_ref() != Some(hidden)
}

// ---------------------------------------------------------------------------------------------
// Path Selection

/// What Path Selection edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "layer")]
pub enum PathTarget {
    Shape(u64),
    /// The active layer's vector mask, when its Layers thumbnail is targeted (#196).
    VectorMask(u64),
    Work,
    /// The work or saved path selected in the Paths panel (the content is the path's name).
    Saved(String),
}

impl Default for PathTarget {
    fn default() -> Self {
        PathTarget::Shape(0)
    }
}

/// The targeted vector mask of the active layer, if the Layers panel targets it.
pub(crate) fn targeted_vector_mask(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    let targeted = app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
    (targeted && !matches!(l.content, LayerContent::Shape(_))).then_some(())?;
    l.vector_mask.as_ref().map(|m| (l.id.0, m.path.clone()))
}

/// The active type layer's path in document space, if it is type on a path (#9114).
pub(crate) fn active_type_path(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Text(t) => photocraft_engine::type_extra_cmds::type_path(t).map(|p| (l.id.0, p)),
        _ => None,
    }
}

/// The active shape layer's path, if the active layer is a shape layer.
pub(crate) fn active_shape_path(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some((l.id.0, sh.path.clone())),
        _ => None,
    }
}

/// Does ⌫ / Delete delete the work path? With the Pen, Freeform Pen or Path Selection tool, no
/// path being drawn and the work path the one these tools show and edit (no shape layer or
/// targeted vector mask in front of it), as in Photoshop (#5702). It was drawn whenever one of
/// these tools was active, but the key reached Edit › Clear, so it looked impossible to delete.
/// Direct Selection and the Curvature Pen delete selected anchors in Photoshop, not the path.
pub(crate) fn delete_key_deletes_work_path(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool, Tool::Pen | Tool::FreeformPen | Tool::PathSelection)
        && app.ui.pen.is_none()
        && app.ui.transform.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path)
        && matches!(target_path(app), Some((PathTarget::Work, _)))
}

/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, else
/// the work path.
pub(super) fn target_path(app: &PhotocraftApp) -> Option<(PathTarget, Path)> {
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some((PathTarget::VectorMask(id), p));
    }
    if let Some((id, p)) = active_shape_path(app) {
        return Some((PathTarget::Shape(id), p));
    }
    if !work_path_shown(app) {
        return None;
    }
    app.session.active()?.doc.work_path.clone().map(|p| (PathTarget::Work, p))
}

/// The saved path selected in the Paths panel, with its name: the target path for the path
/// tools, so the pasted path and its siblings are editable on the canvas like any other path.
pub(super) fn saved_target(app: &PhotocraftApp) -> Option<(String, Path)> {
    let name = selected_document_path(app)?;
    if name == "work" {
        return None;
    }
    let st = app.session.active()?;
    let p = st.doc.paths.iter().find(|q| q.name == name)?.path.clone();
    Some((name, p))
}

/// Edit › Free Transform Path (⌘T with Path Selection or Direct Selection): the path the
/// box transforms (the one Path Selection edits) and `path.transform`'s params that name it.
pub(crate) fn free_transform_path(app: &PhotocraftApp) -> Option<(Value, Path)> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection) {
        return None;
    }
    // The Paths panel's selection is the target path (as `active_path_name`), then the
    // targeted vector mask, the active shape's path or the work path.
    let (target, path) = saved_target(app).map(|(n, p)| (PathTarget::Saved(n), p)).or_else(|| target_path(app))?;
    path.control_bounds()?;
    let params = match target {
        PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
        PathTarget::Work => json!({"name": "work"}),
        PathTarget::Saved(name) => json!({"name": name}),
    };
    Some((params, path))
}

fn point_in_subpath(sp: &photocraft_doc::vector::Subpath, pt: [f64; 2]) -> bool {
    if !sp.closed || sp.knots.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = sp.knots.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let xi = sp.knots[i].anchor.x;
        let yi = sp.knots[i].anchor.y;
        let xj = sp.knots[j].anchor.x;
        let yj = sp.knots[j].anchor.y;
        let intersect = ((yi > pt[1]) != (yj > pt[1])) && (pt[0] < (xj - xi) * (pt[1] - yi) / (yj - yi) + xi);
        if intersect {
            inside = !inside;
        }
    }
    inside
}

/// Path Selection's (black arrow) component selection: which subpaths of which path are
/// selected, so a drag moves just those instead of the whole path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathSelection {
    /// The document the selection belongs to.
    pub doc: u64,
    /// The path the selected subpaths belong to (`None` = nothing selected).
    pub target: Option<PathTarget>,
    /// Selected subpath indices into the target path.
    pub subpaths: Vec<usize>,
    /// Knot count per subpath when the selection was made: a structural edit (anchors or
    /// subpaths added/removed) invalidates the selection instead of moving stale indices.
    pub knots: Vec<usize>,
}

/// The subpaths `app.ui.path_selection` currently holds on `(doc, target, path)`: empty when
/// the selection belongs to another document or path, or its knot signature no longer matches
/// (a structural edit made the stored indices stale, so they are ignored, not moved).
pub(crate) fn selected_subpaths(app: &PhotocraftApp, doc: &Document, target: PathTarget, path: &Path) -> Vec<usize> {
    let sel = &app.ui.path_selection;
    if sel.target.as_ref() != Some(&target) || sel.doc != doc.id.0 || sel.subpaths.is_empty() {
        return Vec::new();
    }
    if sel.knots != path.subpaths.iter().map(|s| s.knots.len()).collect::<Vec<_>>() {
        return Vec::new();
    }
    sel.subpaths.iter().copied().filter(|&i| i < path.subpaths.len()).collect()
}

/// The shape component of `path` under document point `at`: the range of its subpaths, picked
/// topmost-first (the last component folded in, as compositing draws it). Components are
/// [`Path::components`] — a `Join` subpath is its predecessor's hole or continuation, not an
/// independently filled, selectable shape; the interior test renders the component's exact
/// coverage, so a joined hole is not part of its component. The black arrow selects the
/// whole component: a drag moves every subpath of the range together.
fn pick_subpath_at(path: &Path, at: [f64; 2], tol: f64) -> Option<std::ops::Range<usize>> {
    let p = photocraft_geom::Point::new(at[0], at[1]);
    path.components().into_iter().rev().find(|c| {
        let single = Path { subpaths: path.subpaths[c.clone()].to_vec(), fill_rule: path.fill_rule, inverted: false };
        if photocraft_vector::edit::hit(&single, p, tol, &[]).is_some() {
            return true;
        }
        let area = photocraft_doc::Rect::from_xywh(at[0].floor() as i32, at[1].floor() as i32, 1, 1);
        photocraft_vector::fill_rasterizer(&single, photocraft_vector::DEFAULT_TOLERANCE).render(area).first().is_some_and(|&v| v > 0.0)
    })
}

/// The topmost visible shape layer under `at` (document coordinates):
/// hits the shape path or anchor within `tol`, or any non-transparent pixel of the shape's raster surface,
/// or point inside the shape's filled geometry.
pub fn pick_shape_at(doc: &Document, at: [f64; 2], tol: f64) -> Option<photocraft_doc::LayerId> {
    let (x, y) = (at[0].floor() as i32, at[1].floor() as i32);
    for (path, _, l) in doc.walk().iter().rev() {
        if !l.visible || !(1..=path.len()).all(|n| doc.layer_at(&path[..n]).is_some_and(|a| a.visible)) {
            continue;
        }
        let board = path.first().and_then(|i| doc.layers.get(*i)).and_then(|t| t.artboard());
        if board.is_some_and(|a| !a.rect.contains(x, y)) {
            continue;
        }
        if let LayerContent::Shape(sh) = &l.content {
            if photocraft_vector::edit::hit(&sh.path, photocraft_geom::Point::new(at[0], at[1]), tol, &[]).is_some() {
                return Some(l.id);
            }
            if sh.fill.is_some() {
                if let Some(surf) = l.surface() {
                    let mut px = [[0.0f32; 4]; 1];
                    surf.read_rgba_into(photocraft_doc::Rect::from_xywh(x, y, 1, 1), &mut px);
                    if px[0][3] > 0.0 {
                        return Some(l.id);
                    }
                }
                if let Some(photocraft_doc::LiveShape::Rect { rect, .. }) = &sh.live {
                    let x0 = rect[0].min(rect[0] + rect[2]);
                    let x1 = rect[0].max(rect[0] + rect[2]);
                    let y0 = rect[1].min(rect[1] + rect[3]);
                    let y1 = rect[1].max(rect[1] + rect[3]);
                    if at[0] >= x0 && at[0] <= x1 && at[1] >= y0 && at[1] <= y1 {
                        return Some(l.id);
                    }
                }
                if let Some(photocraft_doc::LiveShape::Ellipse { rect }) = &sh.live {
                    let rx = rect[2].abs() / 2.0;
                    let ry = rect[3].abs() / 2.0;
                    if rx > 1e-6 && ry > 1e-6 {
                        let cx = rect[0] + rect[2] / 2.0;
                        let cy = rect[1] + rect[3] / 2.0;
                        let dx = (at[0] - cx) / rx;
                        let dy = (at[1] - cy) / ry;
                        if dx * dx + dy * dy <= 1.0 {
                            return Some(l.id);
                        }
                    }
                }
                if sh.path.subpaths.iter().any(|sp| point_in_subpath(sp, at)) {
                    return Some(l.id);
                }
            }
        }
    }
    None
}

/// Moves the selected subpaths by `(dx, dy)` through `path.moveAnchors`: one history step per
/// drag; the command's target params address a layer's shape path, a targeted vector mask and
/// the work path alike.
fn move_selected(app: &mut PhotocraftApp, target: PathTarget, path: &Path, subpaths: &[usize], dx: f64, dy: f64) {
    let anchors: Vec<[usize; 2]> =
        subpaths.iter().filter_map(|&s| path.subpaths.get(s).map(|sp| (s, sp))).flat_map(|(s, sp)| (0..sp.knots.len()).map(move |k| [s, k])).collect();
    if anchors.is_empty() {
        return;
    }
    let mut params = json!({"anchors": anchors, "move": [dx, dy]});
    let target = match target {
        PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
        PathTarget::Work => json!({"name": "work"}),
        PathTarget::Saved(name) => json!({"name": name}),
    };
    if let (Some(p), Some(t)) = (params.as_object_mut(), target.as_object()) {
        p.extend(t.clone());
    }
    if let Err(e) = app.run("path.moveAnchors", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The black arrow's press-release: selects the shape layer or path component under `start`
/// (activating the layer, as in Photoshop), records the component selection, and moves the
/// selected components by the press-to-release offset. A press on empty canvas clears the
/// component selection; a drag starting there is a no-op (Photoshop marquee-selects, a
/// follow-up).
pub fn path_selection_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let clear = |app: &mut PhotocraftApp| app.ui.path_selection = PathSelection::default();

    // The press's target: a shape layer under the pointer (selected on the press, as in
    // Photoshop), else the path selected in the Paths panel (a saved path — the pasted path is
    // one), else the targeted vector mask, the active shape's path or the work path.
    let hit: Option<(PathTarget, Path, Option<std::ops::Range<usize>>)> = app
        .session
        .active()
        .and_then(|st| {
            let id = pick_shape_at(&st.doc, start, tol)?;
            let l = st.doc.layer(id)?;
            let LayerContent::Shape(sh) = &l.content else { return None };
            Some((PathTarget::Shape(id.0), sh.path.clone(), pick_subpath_at(&sh.path, start, tol)))
        })
        .or_else(|| {
            saved_target(app).map(|(name, p)| {
                let sub = pick_subpath_at(&p, start, tol);
                (PathTarget::Saved(name), p, sub)
            })
        })
        .or_else(|| {
            target_path(app).map(|(t, p)| {
                let sub = pick_subpath_at(&p, start, tol);
                (t, p, sub)
            })
        });
    let Some((target, path, component)) = hit else {
        clear(app);
        return;
    };

    if let PathTarget::Shape(id) = &target
        && app.session.active().and_then(|st| st.active_layer).map(|l| l.0) != Some(*id)
    {
        let _ = app.run("layer.select", json!({"layer": id}));
    }

    // The press landed on the path's layer but no component: keep/drop the layer selection and
    // clear the component selection (a drag from empty canvas is a no-op; Photoshop
    // marquee-selects here, a follow-up).
    let Some(component) = component else {
        clear(app);
        return;
    };

    // Selection bookkeeping: a press on the already-selected component of the same,
    // structurally unchanged path keeps the selection (a drag starting on it moves all of
    // it); any other press makes the hit the new selection.
    let picked: Vec<usize> = component.collect();
    let knots: Vec<usize> = path.subpaths.iter().map(|s| s.knots.len()).collect();
    let sel = &mut app.ui.path_selection;
    let keep =
        sel.target.as_ref() == Some(&target) && sel.doc == app.session.active().map_or(0, |st| st.doc.id.0) && sel.knots == knots && sel.subpaths == picked;
    if !keep {
        *sel = PathSelection { doc: app.session.active().map_or(0, |st| st.doc.id.0), target: Some(target.clone()), subpaths: picked, knots };
    }

    if dx.abs() + dy.abs() >= 0.5 {
        let subpaths = app.ui.path_selection.subpaths.clone();
        move_selected(app, target, &path, &subpaths, dx, dy);
    }
}

/// Layer › Combine Shapes narrows to the black arrow's selected component: with exactly one
/// component of a shape layer selected (Path Selection tool), Unite / Subtract / Intersect /
/// Exclude set that component's path operation alone, as Photoshop's options-bar
/// path-operation buttons do; the other components keep theirs. `None` = not this context,
/// dispatch as usual (every component but the first, or a merge of the selected layers).
pub(crate) fn combine_shapes_intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(
        id,
        "layer.combineShapes.unite"
            | "layer.combineShapes.subtractFrontShape"
            | "layer.combineShapes.intersectShapeAreas"
            | "layer.combineShapes.excludeOverlappingShapes"
    ) {
        return None;
    }
    let (target, subpaths) = super::path_clip::live_selection(app)?;
    let PathTarget::Shape(layer) = target else { return None };
    let [sub] = subpaths.as_slice() else { return None };
    // Explicit params address the command elsewhere; respect them.
    if params.get("subpath").is_some() || params.get("layer").is_some_and(|v| v.as_u64() != Some(layer)) {
        return None;
    }
    Some(app.run(id, json!({"layer": layer, "subpath": sub})))
}
