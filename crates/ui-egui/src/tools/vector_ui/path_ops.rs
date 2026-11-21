use super::overlay::selected_saved_path_index;
use super::paths_panel::{PathEntry, PathRow, path_rows};
use super::pen::pen_commit;
use super::{Affine, Document, LayerContent, Path, PhotocraftApp, Tool, Value, json};
use crate::direct_select::PathRef;
use craft::behavior::kurbo::Point as EditPoint;
use craft::draw::ui::pen::{EditPath, EditSubpath, Knot as EditKnot};

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
    if app.ui.pen.as_ref().is_some_and(|p| p.draft.knots.len() >= 2) {
        // The finished path becomes the work path, or a shape layer's path or a vector mask.
        if !pen_commit(app, false) {
            return Err(app.ui.status.clone());
        }
    }
    let name = active_path_name(app)
        .ok_or_else(|| tl!("No path to make a selection from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("name".into(), json!(name));
    let r = app.run("path.toSelection", Value::Object(p))?;
    deselect_loaded_path(app, &name);
    Ok(r)
}

/// The Pen options bar's Make Shape (Path mode, #10934): a new shape layer from the path being
/// drawn, or else from the Work Path or saved path that is the target path, with the shape tools'
/// fill and stroke, as one History step. A path being drawn is not committed: the shape takes what
/// the commit would have made (with a vector mask targeted, only the drawn path, and the mask
/// stays), see [`super::pen::make_shape_from_draft`]. A shape layer's own path or vector mask
/// isn't copied. The Work Path becomes the shape's path, so it is no longer selected in the Paths
/// panel.
pub fn make_shape(app: &mut PhotocraftApp) -> Result<Value, String> {
    // #12910 measured: with the Pen in an Indexed Color document the reference application shows this
    // alert with OK and changes nothing, so return before the draft is touched or any command runs.
    // The alert names the tool: #12840 measures the Freeform Pen with different wording, and the
    // other tools that share this button were not measured and keep today's behaviour. The engine's
    // `path.toShape` is left alone, as the issue's alert is the button's.
    if app.ui.tool == Tool::Pen && indexed_document(app) {
        let message = crate::i18n::fmt(tl!(crate::tools::brush_refusal::INDEXED), &[("tool", tl!("pen tool"))]);
        crate::dialogs::alerts::alert(app, message);
        return Ok(Value::Null);
    }
    super::cancel_stale_pen(app);
    if let Some(pen) = app.ui.pen.clone().filter(|p| p.draft.knots.len() >= 2) {
        let r = super::pen::make_shape_from_draft(app, &pen)?;
        super::host::drop_draft(app);
        return Ok(r);
    }
    let name =
        shape_source_path(app).ok_or_else(|| tl!("No path to make a shape from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let r = app.run("path.toShape", json!({"name": name, "fill": super::shapes::fill_param(app), "stroke": super::shapes::stroke_param(app)}))?;
    if name == "work" && app.ui.selected_path.as_deref() == Some("work") {
        app.ui.selected_path = None;
    }
    Ok(r)
}

/// Whether the active document is Indexed Color: the colour mode the reference application refuses the
/// Pen for (#12910; the Freeform Pen's own wording is #12840). Bitmap, Duotone and Multichannel were
/// not measured and keep today's behaviour.
pub(super) fn indexed_document(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed)
}

/// The path Make Shape makes a shape from without a draft: [`active_path_name`] when it is the
/// Work Path or a saved path, never a layer's own path (that would duplicate the shape or mask).
pub fn shape_source_path(app: &PhotocraftApp) -> Option<String> {
    active_path_name(app).filter(|n| n != "layer")
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

/// Path Selection has components of the saved path targeted in the Paths panel selected: what its bare Enter and
/// Esc deselect (#13287).
pub(crate) fn saved_path_components_selected(app: &PhotocraftApp) -> bool {
    selected_saved_path_index(app).is_some_and(|(index, _)| matches!(crate::path_components::selected(app), Some((PathRef::Saved(i), ..)) if i == index))
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

/// The work or saved path explicitly selected in the Paths panel, by name, and its geometry. Only
/// such a selection overrides the active layer's path.
pub(super) fn selected_document_geometry(app: &PhotocraftApp) -> Option<(String, &Path)> {
    let st = app.session.active()?;
    match app.ui.selected_path.as_deref()? {
        "work" => st.doc.work_path.as_ref().map(|p| ("work".into(), p)),
        "layer" => None,
        name => st.doc.paths.iter().find(|p| p.name == name).map(|p| (p.name.clone(), &p.path)),
    }
}

/// The targeted vector mask of the active layer, if the Layers panel targets it.
pub(crate) fn targeted_vector_mask(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let id = targeted_vector_mask_id(app)?;
    let mask = app.session.active()?.doc.layer(photocraft_doc::LayerId(id))?.vector_mask.as_ref()?;
    Some((id, mask.path.clone()))
}

pub(super) fn targeted_vector_mask_id(app: &PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    let targeted = app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
    (targeted && !matches!(l.content, LayerContent::Shape(_))).then_some(())?;
    l.vector_mask.as_ref().map(|_| l.id.0)
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

/// Does ⌫ / Delete delete the work path? With the Pen, Freeform Pen, Curvature Pen or Path
/// Selection tool, no path being drawn and the work path the one these tools show and edit (no
/// shape layer or targeted vector mask in front of it), as in Photoshop (#5702, #12695). It was
/// drawn whenever one of these tools was active, but the key reached Edit › Clear, so it looked
/// impossible to delete. A selected anchor goes first (the overlay's own key).
pub(crate) fn delete_key_deletes_work_path(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::PathSelection)
        && app.ui.pen.is_none()
        && app.ui.transform.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path)
        && matches!(target_path(app), Some((PathRef::Work, _)))
}

/// What ⌫ / Delete edits besides an anchor (the overlay's own key): the command and its params.
/// With Direct Selection and a clicked segment (`[subpath, knot]` it leaves) and no selected
/// anchor, the segment goes on its own: its anchors stay and the subpath opens there, as in
/// Photoshop (#13203). With nothing selected, the path Direct Selection targets is deleted: a
/// saved path, the work path or a vector mask. The work path with the Pen, Freeform Pen,
/// Curvature Pen or Path Selection is [`delete_key_deletes_work_path`]. A shape layer's path
/// wasn't measured: the key keeps deleting the layer.
pub(crate) fn delete_key_edits_path(app: &PhotocraftApp) -> Option<(&'static str, Value)> {
    if delete_key_deletes_work_path(app) {
        return Some(("path.delete", json!({"name": "work"})));
    }
    let ds = &app.ui.direct_selection;
    let usable = app.ui.tool == Tool::DirectSelection
        && app.ui.pen.is_none()
        && app.ui.transform.is_none()
        && ds.editing.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path);
    usable.then_some(())?;
    let holds = crate::direct_select::still_valid(app, ds);
    if holds
        && ds.anchors.is_empty()
        && let (Some([s, k]), Some(target)) = (ds.segment, ds.target)
    {
        let (_, path) = crate::direct_select::candidates(app).into_iter().find(|(r, _)| *r == target)?;
        path.subpaths.get(s)?.knots.get(k)?;
        crate::direct_select::writable(app, &target).then_some(())?;
        let mut params = json!({"subpath": s, "knot": k});
        if let (Some(p), Some(t)) = (params.as_object_mut(), target.params(app).as_object()) {
            p.extend(t.clone());
        }
        return Some(("path.deleteSegment", params));
    }
    (!holds || (ds.anchors.is_empty() && ds.segment.is_none())).then_some(())?;
    let st = app.session.active()?;
    match target_path(app)?.0 {
        PathRef::Work => Some(("path.delete", json!({"name": "work"}))),
        PathRef::Saved(i) => Some(("path.delete", json!({"name": st.doc.paths.get(i)?.name}))),
        // As the Paths panel's trash does with a vector mask's row.
        PathRef::Layer(id) if targeted_vector_mask_id(app) == Some(id) => Some(("layer.vectorMask.delete", json!({"layer": id}))),
        _ => None,
    }
}

/// The path Path Selection edits: the selected Work or saved path, the targeted vector mask, the
/// active shape layer's path (both `PathRef::Layer`), else the work path.
pub(super) fn target_path(app: &PhotocraftApp) -> Option<(PathRef, Path)> {
    if let Some((name, p)) = selected_document_geometry(app) {
        if name == "work" {
            return Some((PathRef::Work, p.clone()));
        }
        if let Some((i, saved)) = selected_saved_path_index(app) {
            return Some((PathRef::Saved(i), saved));
        }
    }
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some((PathRef::Layer(id), p));
    }
    if let Some((id, p)) = active_shape_path(app) {
        return Some((PathRef::Layer(id), p));
    }
    if !work_path_shown(app) {
        return None;
    }
    app.session.active()?.doc.work_path.clone().map(|p| (PathRef::Work, p))
}

/// A handle this close to its anchor is retracted for the engine's `bend_segment`, but the
/// overlay's `is_straight` compares exactly, so such a handle is set equal to its anchor.
const RETRACTED: f64 = 1e-9;

/// The overlay's `EditPath` of a document path, keyed `key` (`PathRef::key`). A Type Path takes no copies of its
/// subpaths: Direct Selection's ⌥ drag on its segment moves the subpath.
pub(crate) fn edit_path(path: &Path, key: u64) -> EditPath {
    let point = |p: &photocraft_geom::Point| EditPoint::new(p.x, p.y);
    let handle = |anchor: EditPoint, h: &photocraft_geom::Point| {
        let h = point(h);
        if h.distance(anchor) < RETRACTED { anchor } else { h }
    };
    let subpaths = path
        .subpaths
        .iter()
        .map(|s| {
            let knots = s
                .knots
                .iter()
                .map(|k| {
                    let anchor = point(&k.anchor);
                    EditKnot::new(anchor, handle(anchor, &k.in_ctrl), handle(anchor, &k.out_ctrl), k.smooth)
                })
                .collect();
            EditSubpath::new(knots, s.closed)
        })
        .collect();
    let copies = !matches!(PathRef::from_key(key), Some(PathRef::TypePath(_)));
    EditPath::new(key, subpaths).copies(copies)
}

/// Edit › Free Transform Path (⌘T with Path Selection or Direct Selection): the path the
/// box transforms (the one Path Selection edits) and `path.transform`'s params that name it.
pub(crate) fn free_transform_path(app: &PhotocraftApp) -> Option<(Value, Path)> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection) {
        return None;
    }
    let (target, path) = target_path(app)?;
    path.control_bounds()?;
    Some((target.params(app), path))
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
            if near_outline(&sh.path, at, tol) && photocraft_vector::edit::hit(&sh.path, photocraft_geom::Point::new(at[0], at[1]), tol, &[]).is_some() {
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

/// Whether `at` lies within `tol` of the box round `path`'s anchors and control points, which
/// holds every curve of it (each segment stays inside its control points' hull). Outside it
/// `edit::hit` can't find an anchor or a segment, so the pick skips its curve sampling: the box's
/// side band asks on every hover frame, past every shape layer (#11473). The box is grown a hair
/// more than `tol` so rounding in the sampled distances never drops a hit.
fn near_outline(path: &Path, at: [f64; 2], tol: f64) -> bool {
    let Some((x0, y0, x1, y1)) = path.control_bounds() else { return false };
    let pad = tol * 1.001 + 1e-9 * (1.0 + at[0].abs() + at[1].abs());
    // Written so a NaN bound keeps the check (`edit::hit` decides), as an infinite one does.
    !(at[0] < x0 - pad || at[0] > x1 + pad || at[1] < y0 - pad || at[1] > y1 + pad)
}

/// How far a Path Selection drag from `start` to `end` moves its path: whole pixels. `None` for
/// a click (under half a pixel of travel), which picks a shape layer instead.
fn path_selection_offset(start: [f64; 2], end: [f64; 2]) -> Option<[f64; 2]> {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    if dx.abs() + dy.abs() < 0.5 {
        return None;
    }
    Some([dx.round(), dy.round()])
}

/// What Path Selection reports to its host: the pass's press and release, drained by the Vector
/// family's `drain`, which does the press work and the release command with the app.
#[derive(Clone, Debug)]
pub(crate) enum PathSelectionEvent {
    /// A press at the document point, with Shift held.
    Press { at: [f64; 2], shift: bool },
    /// The release: the drag from `start` to `end`, with the press-time owner `drain` set (none
    /// when no owner was set).
    Release { start: [f64; 2], end: [f64; 2], owner: Option<super::VectorOwner> },
}

/// A Path Selection drag in progress.
#[derive(Clone, Debug)]
pub(crate) struct PathSelectionDrag {
    /// The press, in document points.
    pub start: [f64; 2],
    /// The pointer's last document point.
    pub last: [f64; 2],
    /// The modifiers held at the press; the cursor keeps them for the drag.
    pub mods: craft::keymap::Mods,
    /// What the press targeted; a drag whose owner is no longer valid shows and commits nothing.
    pub owner: Option<super::VectorOwner>,
}

/// The state of the Path Selection tool (A), kept across passes: the drag in progress and the
/// events the pass reports. [`PathSelectionPass`] runs it.
#[derive(Default)]
pub(crate) struct PathSelectionTool {
    drag: Option<PathSelectionDrag>,
    events: Vec<PathSelectionEvent>,
}

impl PathSelectionTool {
    /// The drag in progress.
    pub(super) fn drag(&self) -> Option<&PathSelectionDrag> {
        self.drag.as_ref()
    }

    /// Records what the press targeted. The Vector family's `drain` calls it after a `Press` event,
    /// as capturing the owner needs the app.
    // PLAN: tools-w33's `drain` calls `owned_by(VectorOwner::capture(app))` after the Press work
    pub(crate) fn owned_by(&mut self, owner: super::VectorOwner) {
        if let Some(d) = &mut self.drag {
            d.owner = Some(owner);
        }
    }

    /// The pass's events, in order.
    pub(crate) fn take_events(&mut self) -> Vec<PathSelectionEvent> {
        std::mem::take(&mut self.events)
    }
}

/// The Path Selection tool for one pass: a press selects, a drag moves the selected path or
/// components, a release commits through [`path_selection_finish`]. Its cursor reads the app
/// through the pass's scope.
pub(crate) struct PathSelectionPass<'a> {
    pub(crate) scope: &'a crate::canvas::tools::ToolScope<'a>,
    pub(crate) tool: &'a mut PathSelectionTool,
}

fn doc_point(pointer: &craft::ui::canvas::ToolPointer, cx: &craft::ui::canvas::ToolContext<'_>) -> Option<[f64; 2]> {
    let p = cx.space()?.view.to_doc(pointer.pos);
    Some([p.x, p.y])
}

impl craft::ui::canvas::CanvasTool for PathSelectionPass<'_> {
    fn press(&mut self, pointer: &craft::ui::canvas::ToolPointer, cx: &mut craft::ui::canvas::ToolContext<'_>) -> craft::ui::canvas::ToolAction {
        let Some(at) = doc_point(pointer, cx) else { return craft::ui::canvas::ToolAction::Ignored };
        self.tool.drag = Some(PathSelectionDrag { start: at, last: at, mods: pointer.mods, owner: None });
        self.tool.events.push(PathSelectionEvent::Press { at, shift: pointer.mods.shift });
        craft::ui::canvas::ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &craft::ui::canvas::ToolPointer, cx: &mut craft::ui::canvas::ToolContext<'_>) {
        if let (Some(d), Some(at)) = (&mut self.tool.drag, doc_point(pointer, cx)) {
            d.last = at;
        }
    }

    fn release(&mut self, pointer: &craft::ui::canvas::ToolPointer, cx: &mut craft::ui::canvas::ToolContext<'_>) {
        let Some(d) = self.tool.drag.take() else { return };
        let end = doc_point(pointer, cx).unwrap_or(d.last);
        self.tool.events.push(PathSelectionEvent::Release { start: d.start, end, owner: d.owner });
    }

    fn cancel(&mut self, _reason: craft::behavior::CancelReason, _cx: &mut craft::ui::canvas::ToolContext<'_>) {
        self.tool.drag = None;
    }

    /// #13287: with components of the saved path targeted in the Paths panel selected, a bare Enter deselects the
    /// components and also untargets the path (its row and outline go), with no History step. With no component of
    /// that path selected, or a modifier held, Enter passes on. Esc is the window's idle Esc (`shortcuts::escape_keys`),
    /// after its pixel deselect.
    fn key(&mut self, key: &craft::ui::canvas::ToolKey, cx: &mut craft::ui::canvas::ToolContext<'_>) -> bool {
        if !matches!(key, craft::ui::canvas::ToolKey::Submit) || cx.mods != craft::keymap::Mods::NONE || self.tool.drag.is_some() {
            return false;
        }
        self.scope.with_app(|app| {
            if !saved_path_components_selected(app) {
                return false;
            }
            crate::path_components::deselect_components(app);
            if let Some(name) = app.ui.selected_path.clone() {
                deselect_loaded_path(app, &name);
            }
            true
        })
    }

    fn command_form(&self) -> craft::ui::canvas::ToolPath {
        craft::ui::canvas::ToolPath::Command("the Paths panel and Edit › Transform Path")
    }

    // The moved path shows through the overlay (`show_paths`), so the tool paints nothing.
    fn paint(&self, _draw: &craft::ui::primitives::Draw<'_>, _cx: &craft::ui::canvas::ToolContext<'_>) {}

    /// `cursor_policy::path_selection_cursor`, with the press's modifiers and point while a drag runs.
    fn cursor(&self, cx: &craft::ui::canvas::ToolContext<'_>) -> craft::behavior::Cursor {
        let (mods, at) = match &self.tool.drag {
            Some(d) => (d.mods, Some(d.start)),
            None => (cx.mods, cx.pointer.as_ref().and_then(|p| doc_point(p, cx))),
        };
        self.scope.with_app(|app| crate::canvas::cursor_policy::path_selection_cursor(app, mods, at, cx.precise()))
    }
}

/// The path a Path Selection drag in progress shows moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MovedPath {
    Work,
    /// The saved path selected in the Paths panel.
    Saved,
}

/// The work path or selected saved path of a Path Selection drag in progress, moved where its
/// release puts it ([`path_selection_finish`]: the same offset, only the selected components when
/// some but not all are), so the outline follows the pointer instead of jumping on release.
/// `None` without a live drag on `doc`, for a click, and for a shape layer's or vector mask's
/// path, whose pixels move only on release.
pub(crate) fn path_selection_preview(app: &PhotocraftApp, doc: &Document) -> Option<(MovedPath, Path)> {
    let d = app.canvas_tools.vector.path_selection.drag().filter(|d| d.owner.as_ref().is_some_and(|o| o.valid(app)))?;
    if app.session.active().is_none_or(|st| st.doc.id != doc.id) {
        return None;
    }
    let [dx, dy] = path_selection_offset(d.start, d.last).filter(|o| o.iter().all(|v| v.is_finite()))?;
    if let Some((r, path)) = crate::path_components::moved_selected(app, [dx, dy]) {
        return match r {
            crate::direct_select::PathRef::Work => Some((MovedPath::Work, path)),
            crate::direct_select::PathRef::Saved(_) => Some((MovedPath::Saved, path)),
            _ => None,
        };
    }
    let (target, path) = target_path(app)?;
    let moved = match target {
        PathRef::Work => MovedPath::Work,
        PathRef::Saved(_) => MovedPath::Saved,
        _ => return None,
    };
    Some((moved, path.transform(&Affine::translate(dx, dy))))
}

pub fn path_selection_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let Some([dx, dy]) = path_selection_offset(start, end) else {
        let Some(st) = app.session.active() else { return };
        if let Some(id) = pick_shape_at(&st.doc, start, tol) {
            let _ = app.run("layer.select", json!({"layer": id.0}));
        }
        return;
    };
    if target_path(app).is_none() {
        let Some(st) = app.session.active() else { return };
        if let Some(id) = pick_shape_at(&st.doc, start, tol) {
            let _ = app.run("layer.select", json!({"layer": id.0}));
        }
    }
    // Selected components move on their own (`path_components`); with none, the whole path.
    if let Some(r) = crate::path_components::move_selected(app, [dx, dy]) {
        if let Err(e) = r {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    let Some((target, path)) = target_path(app) else { return };
    let moved_json = || photocraft_engine::vector_cmds::path_json(&path.transform(&Affine::translate(dx, dy)));
    let _ = match target {
        // The targeted vector mask's pixels move through its own command; a shape layer's through the shape's.
        PathRef::Layer(id) if targeted_vector_mask(app).is_some_and(|(mask, _)| mask == id) => {
            app.run("layer.vectorMask.edit", json!({"layer": id, "path": moved_json()}))
        }
        PathRef::Layer(id) => app.run("shape.edit", json!({"layer": id, "move": [dx, dy]})),
        PathRef::Work | PathRef::Saved(_) => {
            let mut params = target.params(app);
            params["path"] = moved_json();
            app.run("path.set", params)
        }
        PathRef::TypePath(_) | PathRef::Pen => return,
    };
}
