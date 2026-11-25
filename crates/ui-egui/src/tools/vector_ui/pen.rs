use craft::behavior::kurbo::Point;
use craft::draw::ui::{EditSubpath, Knot, KnotRef, VectorMode};
use photocraft_engine::vector_cmds::path_json;

use super::options::selected_shape;
use super::path_ops::{targeted_vector_mask, targeted_vector_mask_id, work_path_shown};
use super::shapes::{fill_param, stroke_param};
use super::{Document, LayerContent, Path, PenPath, PhotocraftApp, Resumed, Tool, ToolOptions, Value, json};

// ---------------------------------------------------------------------------------------------
// Pen

/// The framework's knot for a document knot.
fn edit_knot(k: &photocraft_doc::Knot) -> Knot {
    Knot::new(Point::new(k.anchor.x, k.anchor.y), Point::new(k.in_ctrl.x, k.in_ctrl.y), Point::new(k.out_ctrl.x, k.out_ctrl.y), k.smooth)
}

/// The document's knot for a framework knot.
fn doc_knot(k: &Knot) -> photocraft_doc::Knot {
    let pt = |q: Point| photocraft_geom::Point::new(q.x, q.y);
    photocraft_doc::Knot { anchor: pt(k.anchor), in_ctrl: pt(k.in_ctrl), out_ctrl: pt(k.out_ctrl), smooth: k.smooth }
}

/// The Pen path being drawn as a document path (one subpath, closed as the draft is), so ⌘/Ctrl (Direct Selection)
/// can edit it in place while drawing (#4022) and the commit sends it to the engine with `path_json`.
pub(crate) fn pen_path(pen: &PenPath) -> Path {
    let knots = pen.draft.knots.iter().map(doc_knot).collect();
    Path::new(vec![photocraft_doc::vector::Subpath { closed: pen.draft.closed, knots, op: Default::default() }])
}

pub fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    cancel_stale_pen(app);
    !app.ui.pen.as_ref().is_some_and(|p| p.commit_failed && p.owner.as_ref().is_some_and(|o| !o.valid(app)))
}

pub fn pen_commit(app: &mut PhotocraftApp, closed: bool) -> bool {
    try_pen_commit(app, closed).is_ok()
}

/// Deselect and hide the Work Path a finished path just went into. Callers check that it did: a
/// draft drawn into a vector mask leaves the Work Path selected without changing it.
pub(super) fn deselect_finished_work_path(app: &mut PhotocraftApp) {
    if app.ui.selected_path.as_deref() == Some("work") {
        super::deselect_loaded_path(app, "work");
    }
}

/// Finish the draft as [`pen_commit`] does. On failure the draft stays as it was, marked
/// `commit_failed`, and the error is shown and returned.
pub(super) fn try_pen_commit(app: &mut PhotocraftApp, closed: bool) -> Result<(), String> {
    // This is an explicit commit request; passive frame reconciliation never clears this flag.
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.commit_failed = false;
    }
    if cancel_stale_pen(app) {
        return if app.ui.pen.is_none() { Ok(()) } else { Err(app.ui.status.clone()) };
    }
    let Some(mut pen) = app.ui.pen.take() else { return Ok(()) };
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        // Its anchors are no longer the Pen's: the committed path is another path.
        app.ui.direct_selection = Default::default();
    }
    let original = pen.clone();
    let draft = super::pen_resume::draft_before_finish(&pen);
    // The draft is the one source of `closed`: the request closes it for the commit only.
    pen.draft.closed |= closed;
    let before = super::pen_resume::current_doc(app);
    if let Err(e) = commit_path(app, &pen, closed) {
        app.ui.pen = Some(PenPath { commit_failed: true, ..original });
        app.ui.status = e.clone();
        app.ui.status_error = true;
        return Err(e);
    }
    app.ui.pen_doc = None;
    super::host::release_draft(app);
    // Undo back to this finish gives the path back to the Pen (`pen_resume_on_undo`).
    super::pen_resume::remember_finish(app, draft, &before);
    Ok(())
}

/// Commit a finished Pen, Freeform Pen or Curvature Pen path, as one History step: in Shape mode a
/// new shape layer, or with a Path Operation other than New Layer a subpath of the selected shape
/// layer; in Path mode a subpath of the targeted vector mask, else of the selected path or a new
/// work path. An added subpath takes the options bar's Path Operation (#1397). `false` when the
/// document refused it, with the reason in the status bar.
///
/// The Freeform history override applies to document paths and new shape layers only.
fn commit_path_current(app: &mut PhotocraftApp, pen: &PenPath, closed: bool, freeform: bool) -> bool {
    let shape_mode = app.ui.tool_options.vector_mode == VectorMode::Shape;
    // One anchor is no path, in either mode.
    if pen.draft.knots.len() < 2 {
        return true;
    }
    let path = path_json(&pen_path(pen));
    let op = pen_path_op(&app.ui.tool_options);
    let into_shape = op.and_then(|_| selected_shape(&app.session)).map(|id| id.0);
    let r = if let Some(r) = commit_resumed_on_layer(app, pen, &path, closed) {
        r
    } else if let (Some(op), Some(layer), true) = (op, into_shape, shape_mode) {
        // The selected shape layer keeps its fill and stroke; the new component cuts, intersects…
        app.run("shape.create", json!({"kind": "path", "path": path, "addTo": layer, "op": op, "useAlignedRendering": app.ui.tool_options.align_edges}))
    } else if shape_mode {
        let fill = if closed { fill_param(app) } else { Value::Null };
        let stroke = if closed { stroke_param(app) } else { super::shapes::open_pen_stroke_param(app) };
        app.run("shape.create", json!({"kind": "path", "path": path, "fill": fill, "stroke": stroke, "useAlignedRendering": app.ui.tool_options.align_edges, "historyName": freeform.then_some("Freeform Pen")}))
    } else if let Some((name, p)) =
        pen.resumes.as_ref().and_then(|r| app.session.active().and_then(|st| with_resumed_subpath(&st.doc, r, &path)).map(|p| (r.path.clone(), p)))
    {
        // The resumed subpath, carried on, takes its old place: one step, no second subpath.
        let label = match live_finish(app, pen) {
            // #12890: a live Work Path draft's clicks are History steps of their own, so ↩ and Esc add none; the finish
            // records only what the path does not hold yet.
            Some(LiveFinish::Skip) => {
                app.ui.selected_path = Some(name);
                return true;
            }
            Some(LiveFinish::Close) => Some("Close Path"),
            Some(LiveFinish::Rest) => None,
            None => freeform.then_some("Freeform Pen"),
        };
        app.run("path.set", json!({"name": name, "path": p, "historyName": label})).inspect(|_| app.ui.selected_path = Some(name))
    } else if let Some((id, existing)) = targeted_vector_mask(app) {
        // A targeted vector mask takes the new subpath (#196), as in Photoshop.
        let mut p = path_json(&existing);
        if let (Some(subs), Some(new)) = (p.get_mut("subpaths").and_then(Value::as_array_mut), path.get("subpaths").and_then(Value::as_array)) {
            subs.extend(new.iter().cloned().map(|mut sp| {
                sp["op"] = json!(op.unwrap_or("combine"));
                sp
            }));
        }
        app.run("layer.vectorMask.edit", json!({"layer": id, "path": p}))
    } else {
        // Photoshop adds the new subpath to the path selected in the Paths panel, which is the
        // Work Path right after drawing one, so path after path builds up there (#4721). With no
        // path selected, a new Work Path replaces the old one. The drawn path ends up selected.
        // A resumed draft whose subpath changed meanwhile joins the path it came from, as a new
        // subpath, even with nothing selected in the Paths panel.
        let target = draft_target(app, pen);
        let name = target.clone().unwrap_or_else(|| "work".into());
        let mut params = json!({"name": name, "path": path, "historyName": freeform.then_some("Freeform Pen")});
        if target.is_some() {
            params["op"] = json!(op.unwrap_or("combine"));
        }
        app.run("path.set", params).inspect(|_| app.ui.selected_path = Some(name))
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
        return false;
    }
    true
}

/// The document path a Path-mode draft joins when committed (not into a vector mask): the path it
/// resumed, else the path selected in the Paths panel. `None`: it becomes a new Work Path.
fn draft_target(app: &PhotocraftApp, pen: &PenPath) -> Option<String> {
    let exists = |n: &String| app.session.active().is_some_and(|st| path_exists(&st.doc, n));
    let resumed_from =
        pen.resumes.as_ref().filter(|r| r.layer.is_none() && app.session.active().is_some_and(|st| st.doc.id.0 == r.doc)).map(|r| r.path.clone());
    resumed_from.filter(exists).or_else(|| app.ui.selected_path.clone().filter(exists))
}

/// What Make Shape turns a Path-mode draft into, without committing it: the path the commit would
/// leave (the draft joined to the path it goes into, with the Path Operation, or alone as a new
/// Work Path), and whether that is the Work Path, which the shape then takes. With a vector mask
/// targeted, the draft alone: the mask stays as it was. `None` with no document.
pub(super) fn draft_shape_path(app: &PhotocraftApp, pen: &PenPath) -> Option<(Value, bool)> {
    let drawn = path_json(&pen_path(pen));
    let doc = &app.session.active()?.doc;
    if pen.resumes.as_ref().is_some_and(|r| r.layer.is_some()) || targeted_vector_mask(app).is_some() {
        return Some((drawn, false));
    }
    if let Some(r) = pen.resumes.as_ref().filter(|r| r.doc == doc.id.0)
        && let Some(p) = with_resumed_subpath(doc, r, &drawn)
    {
        return Some((p, r.path == "work"));
    }
    // With nothing to join, the commit's new Work Path would replace the old one.
    let Some(name) = draft_target(app, pen) else { return Some((drawn, true)) };
    let existing = doc_path(doc, &name)?;
    let mut p = path_json(existing);
    let op = pen_path_op(&app.ui.tool_options).unwrap_or("combine");
    if let (Some(subs), Some(new)) = (p.get_mut("subpaths").and_then(Value::as_array_mut), drawn.get("subpaths").and_then(Value::as_array)) {
        subs.extend(new.iter().cloned().map(|mut sp| {
            sp["op"] = json!(op);
            sp
        }));
    }
    Some((p, name == "work"))
}

/// Does `doc` have the path the Paths panel calls `name`: "work", or a saved path's name?
fn path_exists(doc: &Document, name: &str) -> bool {
    doc_path(doc, name).is_some()
}

/// The path the Paths panel calls `name`: "work", or a saved path's name.
fn doc_path<'a>(doc: &'a Document, name: &str) -> Option<&'a Path> {
    if name == "work" { doc.work_path.as_ref() } else { doc.paths.iter().find(|p| p.name == name).map(|p| &p.path) }
}

/// The path a Pen path carries on (#6593): a targeted vector mask, else in Shape mode the
/// selected shape layer's, else the path a new Pen path would join (the one selected in the Paths
/// panel, else the work path). Its Paths-panel name ("layer" for a layer's), layer and path.
fn resume_target(app: &PhotocraftApp) -> Option<(String, Option<u64>, &Path)> {
    let st = app.session.active()?;
    if let Some(id) = targeted_vector_mask_id(app) {
        let mask = st.doc.layer(photocraft_doc::LayerId(id))?.vector_mask.as_ref()?;
        return Some(("layer".to_string(), Some(id), &mask.path));
    }
    if app.ui.tool_options.vector_mode == VectorMode::Shape {
        let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
        let LayerContent::Shape(sh) = &l.content else { return None };
        return Some(("layer".to_string(), Some(l.id.0), &sh.path));
    }
    let name = app.ui.selected_path.clone().filter(|n| path_exists(&st.doc, n)).unwrap_or_else(|| "work".into());
    // A Work Path hidden by Make Selection isn't carried on: a new path replaces it (#6079).
    if name == "work" && !work_path_shown(app) {
        return None;
    }
    let p = doc_path(&st.doc, &name)?;
    Some((name, None, p))
}

/// The path list's entry for the path [`resume_target`] names.
fn target_ref(app: &PhotocraftApp, name: &str, layer: Option<u64>) -> Option<crate::direct_select::PathRef> {
    let st = app.session.active()?;
    Some(match layer {
        Some(id) => crate::direct_select::PathRef::Layer(id),
        None if name == "work" => crate::direct_select::PathRef::Work,
        None => crate::direct_select::PathRef::Saved(st.doc.paths.iter().position(|p| p.name == name)?),
    })
}

/// Subpath `index` of the path a Pen path carries on, as a Pen path to carry on drawing from its
/// last anchor, or from its first when `reversed`: then its knots run backwards, handles swapped.
/// The carried knots are the draft's `kept` run.
fn resumed(doc: u64, name: String, layer: Option<u64>, index: usize, sub: &photocraft_doc::vector::Subpath, reversed: bool) -> PenPath {
    let mut knots: Vec<Knot> = sub.knots.iter().map(edit_knot).collect();
    if reversed {
        knots.reverse();
        for k in &mut knots {
            std::mem::swap(&mut k.in_ctrl, &mut k.out_ctrl);
        }
    }
    let resumes = Resumed { doc, path: name, layer, index, was: sub.clone() };
    PenPath { kept: knots.len(), draft: EditSubpath::new(knots, false), resumes: Some(resumes), ..Default::default() }
}

/// The open ends of the path a press would carry on, for `PenTool::ends`: the path's key, the end's knot and its point.
/// The last knot comes first, as it wins a tie.
pub(super) fn ends(app: &PhotocraftApp) -> Vec<(u64, KnotRef, Point)> {
    let Some((name, layer, path)) = resume_target(app) else { return Vec::new() };
    let Some(key) = target_ref(app, &name, layer).map(|r| r.key()) else { return Vec::new() };
    let mut ends = Vec::new();
    for (i, sub) in path.subpaths.iter().enumerate().filter(|(_, s)| !s.closed) {
        let at = |k: usize| sub.knots.get(k).map(|knot| (key, KnotRef::new(i, k), Point::new(knot.anchor.x, knot.anchor.y)));
        let n = sub.knots.len();
        ends.extend(n.checked_sub(1).and_then(at));
        if n > 1 {
            ends.extend(at(0));
        }
    }
    // Of ends on one point the pen carries on the first: the one Direct Selection has selected (#10726), else the last
    // knot of the earlier subpath.
    if let Some((target, subpath, knot)) = crate::direct_select::selected_open_end(app)
        && target_ref(app, &name, layer) == Some(target)
    {
        ends.sort_by_key(|(_, at, _)| (at.subpath, at.knot) != (subpath, knot));
    }
    ends
}

/// The open end Direct Selection has selected, for `PenTool::carry_on`: with it selected the Curvature Pen's next press
/// carries that subpath on from there (#10726). `None` unless the end is in the path a Pen path carries on
/// ([`resume_target`]): one in another path starts a new one.
pub(super) fn carry_on(app: &PhotocraftApp) -> Option<(u64, KnotRef, EditSubpath)> {
    let (target, subpath, knot) = crate::direct_select::selected_open_end(app)?;
    let (name, layer, _) = resume_target(app)?;
    let at = target_ref(app, &name, layer).filter(|r| *r == target)?;
    let resumed = resume_end(app, subpath, knot)?;
    Some((at.key(), KnotRef::new(subpath, knot), resumed.draft))
}

/// The open subpath `subpath` of the path a Pen path carries on ([`resume_target`]), as a Pen path to carry on drawing
/// from its end `knot`: the first or last anchor (#6593). `None` for another knot, or a closed subpath.
fn resume_end(app: &PhotocraftApp, subpath: usize, knot: usize) -> Option<PenPath> {
    let (name, layer, path) = resume_target(app)?;
    let sub = path.subpaths.get(subpath).filter(|s| !s.closed)?;
    let reversed = if knot + 1 == sub.knots.len() {
        false
    } else if knot == 0 {
        true
    } else {
        return None;
    };
    Some(resumed(app.session.active()?.doc.id.0, name, layer, subpath, sub, reversed))
}

/// What `PenEvent::Resumed(key, knot)` makes the draft: the end `knot` of the path `key`, which must still be the path
/// a Pen path carries on.
pub(super) fn resume_key(app: &PhotocraftApp, key: u64, knot: KnotRef) -> Option<PenPath> {
    let (name, layer, _) = resume_target(app)?;
    if target_ref(app, &name, layer)?.key() != key {
        return None;
    }
    resume_end(app, knot.subpath, knot.knot)
}

/// The resumed path with its subpath carried on: the one subpath of `drawn` takes the old one's
/// anchors and closing, which keeps its other settings (its path operation). `None` when the
/// subpath is no longer the one drawing resumed (another document, or the path changed).
fn with_resumed_subpath(doc: &Document, r: &Resumed, drawn: &Value) -> Option<Value> {
    let path = resumed_path(doc, r)?;
    if path.subpaths.get(r.index) != Some(&r.was) {
        return None;
    }
    let mut p = path_json(path);
    let slot = p.get_mut("subpaths")?.as_array_mut()?.get_mut(r.index)?.as_object_mut()?;
    let new = drawn.get("subpaths")?.get(0)?;
    for key in ["closed", "knots"] {
        slot.insert(key.into(), new.get(key)?.clone());
    }
    Some(p)
}

/// The path a resumed subpath belongs to, while `doc` is the document drawing resumed in: a shape
/// layer's or a vector mask's, or a document path.
fn resumed_path<'a>(doc: &'a Document, r: &Resumed) -> Option<&'a Path> {
    if doc.id.0 != r.doc {
        return None;
    }
    let Some(id) = r.layer else { return doc_path(doc, &r.path) };
    let l = doc.layer(photocraft_doc::LayerId(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some(&sh.path),
        _ => l.vector_mask.as_ref().map(|m| &m.path),
    }
}

/// Commit a resumed Pen path (#6593) into a shape layer or vector mask: the carried-on subpath
/// takes its old place, one step. Closing the open path a shape layer was made of gives it the
/// options bar's fill and stroke, as drawing it closed would have. `None` when the draft resumes
/// a document path, or its subpath changed meanwhile.
fn commit_resumed_on_layer(app: &mut PhotocraftApp, pen: &PenPath, path: &Value, closed: bool) -> Option<Result<Value, String>> {
    let r = pen.resumes.as_ref()?;
    let id = r.layer?;
    let st = app.session.active()?;
    let Some(p) = with_resumed_subpath(&st.doc, r, path) else {
        // A live draft never makes a second layer: its layer's subpath changed under it.
        return pen.live_layer.then(|| Err("Pen draft target is stale; no path was changed".to_string()));
    };
    let layer = st.doc.layer(photocraft_doc::LayerId(id))?;
    let LayerContent::Shape(sh) = &layer.content else {
        return Some(app.run("layer.vectorMask.edit", json!({"layer": id, "path": p})));
    };
    // Finishing a live draft open leaves its layer as it is: no step when the anchors are in it (#12909).
    if pen.live_layer && !closed && live_subpath(&st.doc, pen).zip(pen_path(pen).subpaths.first()).is_some_and(|(a, b)| same_subpath(a, b)) {
        return Some(Ok(Value::Null));
    }
    let mut params = json!({"layer": id, "path": p});
    // Closing a shape made of open paths gives it the options bar's fill and stroke, as drawing it closed would have.
    // A Pen shape has its fill from the first anchor (#12909), so besides one with no fill this counts one whose fill
    // is still the options bar's; a fill set by hand stays.
    let pen_paint = sh.fill.is_none() || (app.ui.tool_options.shape_fill && sh.fill.as_ref() == Some(&super::shapes::tool_paint(app, false)));
    if closed && (pen.live_layer || (pen_paint && sh.path.subpaths.iter().all(|s| !s.closed))) {
        params["fill"] = fill_param(app);
        params["stroke"] = stroke_param(app);
    }
    if app.ui.tool_options.vector_mode == craft::draw::ui::shape::VectorMode::Shape {
        params["useAlignedRendering"] = json!(app.ui.tool_options.align_edges);
    }
    Some(app.run("shape.edit", params))
}

// ---------------------------------------------------------------------------------------------
// The live shape layer (#12909): in Shape mode with New Layer the Pen's first anchor makes the shape layer, and the
// layer takes each anchor as it is placed, so its fill shows while the path is still open.

/// Is `pen` a fresh Pen draft that makes its shape layer now: one anchor, drawn with the Pen (not the Curvature Pen),
/// in Shape mode with New Layer, not carrying a path on?
fn starts_live_shape(app: &PhotocraftApp, pen: &PenPath) -> bool {
    pen.draft.knots.len() == 1
        && pen.resumes.is_none()
        && !pen.live_layer
        && !pen.curvature
        && !pen.commit_failed
        && app.ui.tool == Tool::Pen
        && app.ui.tool_options.vector_mode == VectorMode::Shape
        && pen_path_op(&app.ui.tool_options).is_none()
}

/// Run `id` on the live draft's layer with the draft set aside: after every command `PhotocraftApp::run` reconciles
/// the Pen draft with the document (`sync_views`), and would cancel one whose document just changed as stale. The
/// draft comes back as it was; callers rebind it.
fn run_beside_draft(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    let pen = app.ui.pen.take();
    let r = app.run(id, params);
    app.ui.pen = pen;
    r
}

/// The draft's first anchor makes its shape layer, with the options bar's fill and the open path's stroke. The draft
/// then carries that layer's subpath (`resumes`, `live_layer`). A refused layer leaves the draft as before: the shape
/// is made when the path is finished.
pub(super) fn start_live_shape(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| starts_live_shape(app, p)) else { return };
    let params = json!({"kind": "path", "path": path_json(&pen_path(pen)), "fill": fill_param(app), "stroke": super::shapes::open_pen_stroke_param(app)});
    let made = match run_beside_draft(app, "shape.create", params) {
        Ok(made) => made,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            return;
        }
    };
    let Some(mut pen) = app.ui.pen.take() else { return };
    let made = made.get("layer").and_then(Value::as_u64).and_then(|id| {
        let st = app.session.active()?;
        let LayerContent::Shape(sh) = &st.doc.layer(photocraft_doc::LayerId(id))?.content else { return None };
        Some((st.doc.id.0, id, sh.path.subpaths.first()?.clone()))
    });
    if let Some((doc, id, was)) = made {
        pen.resumes = Some(Resumed { doc, path: "layer".into(), layer: Some(id), index: 0, was });
        pen.live_layer = true;
        pen.kept = 0;
    }
    super::pen_resume::rebind_owner(app, &mut pen);
    if let (Some(owner), Some(id)) = (pen.owner.as_mut(), pen.resumes.as_ref().and_then(|r| r.layer)) {
        // Finishing the path leaves the new layer active, not the one drawing began above.
        owner.layer = Some(photocraft_doc::LayerId(id));
    }
    app.ui.pen = Some(pen);
}

/// Do two subpaths have the same anchors and closing?
fn same_subpath(a: &photocraft_doc::vector::Subpath, b: &photocraft_doc::vector::Subpath) -> bool {
    a.closed == b.closed && a.knots == b.knots
}

/// The subpath of its layer a live draft carries, in `doc`.
fn live_subpath<'a>(doc: &'a Document, pen: &PenPath) -> Option<&'a photocraft_doc::vector::Subpath> {
    let r = pen.resumes.as_ref()?;
    resumed_path(doc, r)?.subpaths.get(r.index)
}

/// The live draft has taken in the document the last command made: the subpath it carries and its owner are that
/// state's.
fn follow_live_layer(app: &mut PhotocraftApp) {
    let Some(mut pen) = app.ui.pen.take() else { return };
    let was = app.session.active().and_then(|st| live_subpath(&st.doc, &pen).cloned());
    if let (Some(r), Some(was)) = (pen.resumes.as_mut(), was) {
        r.was = was;
    }
    super::pen_resume::rebind_owner(app, &mut pen);
    app.ui.pen = Some(pen);
}

/// The live draft's anchors go into its shape layer, one History step, when they differ: after a press ends or a key
/// changes the draft (`host::drain`). A refused edit is reported; the anchors stay in the draft for the next anchor or
/// the finish to try again.
pub(super) fn sync_live_layer_from_draft(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| p.live_layer && !p.commit_failed && !p.draft.knots.is_empty()) else { return };
    let Some((r, id)) = pen.resumes.as_ref().and_then(|r| Some((r, r.layer?))) else { return };
    let drawn = pen_path(pen);
    let Some(st) = app.session.active() else { return };
    if live_subpath(&st.doc, pen).zip(drawn.subpaths.first()).is_some_and(|(a, b)| same_subpath(a, b)) {
        return;
    }
    let Some(path) = with_resumed_subpath(&st.doc, r, &path_json(&drawn)) else { return };
    match run_beside_draft(app, "shape.edit", json!({"layer": id, "path": path})) {
        Ok(_) => follow_live_layer(app),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Does the live draft have anchors its document path has not taken (a refused edit), while that path is still as the
/// draft last left it? Undo then takes those back one by one, as anchors of the draft alone (`host::remove_last`).
pub(super) fn live_anchors_pending(app: &PhotocraftApp) -> bool {
    let (Some(pen), Some(st)) = (app.ui.pen.as_ref().filter(|p| p.live_layer || p.live_path), app.session.active()) else { return false };
    let drawn = pen_path(pen);
    live_subpath(&st.doc, pen)
        .is_some_and(|sub| pen.resumes.as_ref().is_some_and(|r| *sub == r.was) && !drawn.subpaths.first().is_some_and(|d| same_subpath(sub, d)))
}

/// Edit › Undo while a live shape layer or Work Path is being drawn, with no anchor pending: undo its last History
/// step, which is its last anchor (or the layer or Work Path itself with the first); the draft follows the document.
pub(super) fn undo_live_step(app: &mut PhotocraftApp) -> bool {
    if let Err(e) = run_beside_draft(app, "edit.undo", json!({})) {
        app.ui.status = e;
        app.ui.status_error = true;
        return true;
    }
    rebuild_live_draft(app);
    true
}

/// A live draft whose document moved on (Undo, Redo, the History panel…) follows its layer or path: the draft is rebuilt
/// from the layer's or path's open subpath, or dropped when it is gone, closed or empty. When that subpath is as the
/// draft last left it (an edit elsewhere in the document), the draft keeps its anchors, including any it refused, and
/// only follows the new document state. Returns the rebuilt draft, which the Pen must draw from now on.
pub(super) fn sync_live_draft_from_layer(app: &mut PhotocraftApp) -> Option<EditSubpath> {
    let pen = app.ui.pen.as_ref().filter(|p| (p.live_layer || p.live_path) && !p.commit_failed)?;
    let (owner, r) = (pen.owner.as_ref()?, pen.resumes.as_ref()?);
    let st = app.session.active()?;
    if st.doc.id != owner.document || owner.valid(app) {
        return None;
    }
    if live_subpath(&st.doc, pen).is_some_and(|sub| !sub.closed && *sub == r.was) {
        follow_live_layer(app);
        return None;
    }
    if pen.live_layer {
        return rebuild_live_draft(app);
    }
    // A live Work Path draft follows its path when the document moved to a state the Pen's own clicks made: back to an
    // earlier one (Undo, Step Backward, the History panel: a state is waiting to be redone) or forward to one of its
    // clicks (Redo, Step Forward, the History panel at the tip). What names them is not the step's label: `path.set`
    // takes a Pen step name from ANY caller (`crates/engine-core/src/cmd/vector_cmds/paths.rs:86-97` only checks it
    // against the list), so a control-channel, scripted or panel write can put the Work Path under "New Anchor Point"
    // and the label would claim the Pen made it. The proof is the History state itself - the draft records the id of
    // every state its own live writes minted (`PenPath::live_steps`, the same id `pen_resume` pins a finish by), and
    // History mints and never reuses those ids, so no parameter can name one. A command from elsewhere left a newer
    // state instead: like any other draft that carried a path on (#6593) it is stale, and `cancel_stale_pen` drops it
    // with the reason rather than quietly going on with the newer path's anchors.
    let pen_stepped_its_own_path = app.session.active().is_some_and(|st| {
        let h = &st.history;
        h.can_redo() || h.row_id(h.past_len()).is_some_and(|id| pen.live_steps.contains(&id))
    });
    if pen_stepped_its_own_path { rebuild_live_draft(app) } else { None }
}

/// Rebuilds a live draft from its layer's or path's open subpath and hands it to the Pen, or drops it when the subpath
/// is gone, closed or empty.
fn rebuild_live_draft(app: &mut PhotocraftApp) -> Option<EditSubpath> {
    let pen = app.ui.pen.as_ref().filter(|p| p.live_layer || p.live_path)?;
    let (owner, r, live_layer, live_path, kept, live_steps) =
        (pen.owner.clone(), pen.resumes.clone()?, pen.live_layer, pen.live_path, pen.kept, pen.live_steps.clone());
    let rebuilt = app.session.active().and_then(|st| {
        let sub = resumed_path(&st.doc, &r)?.subpaths.get(r.index).filter(|s| !s.closed && !s.knots.is_empty())?;
        let mut draft = resumed(r.doc, r.path.clone(), r.layer, r.index, sub, false);
        draft.live_layer = live_layer;
        draft.live_path = live_path;
        // The states the Pen's own clicks minted belong to the rebuilt draft too: Redo to the tip and the step after
        // it have to stay recognizable as the Pen's own work (#16686 review).
        draft.live_steps = live_steps;
        // A live shape layer's draft owns every anchor of its layer (#12909); a live Work Path draft keeps which of its
        // anchors the path it picked up brought, so Undo of the pick-up itself still knows it (#12890).
        if live_layer {
            draft.kept = 0;
        } else {
            draft.kept = kept;
        }
        draft.owner = owner;
        Some(draft)
    });
    let Some(draft) = rebuilt else {
        super::host::drop_draft(app);
        return None;
    };
    let path = draft.draft.clone();
    app.ui.pen = Some(draft);
    follow_live_layer(app);
    super::host::replace_draft(app, &path);
    Some(path)
}

// ---------------------------------------------------------------------------------------------
// The live Work Path (#12890): in Path mode the Pen's first anchor makes the Work Path, and the path takes every
// further click as a History step of its own, so one Undo takes back one click. The step names are the reference
// application's (its measured rows); the draft keeps carrying that path's subpath, as a live shape layer's does.

/// Is `pen` a fresh Pen draft that draws straight into the Work Path: one anchor, drawn with the Pen (not the Curvature
/// Pen) in Path mode with a Path Operation that joins, nothing carried on, no vector mask targeted, and the Work Path
/// as the destination its finish would pick? A selected saved path and the other operations were not measured and keep
/// today's draft-until-the-finish route.
fn starts_live_path(app: &PhotocraftApp, pen: &PenPath) -> bool {
    pen.draft.knots.len() == 1
        && pen.resumes.is_none()
        && !pen.live_layer
        && !pen.live_path
        && !pen.curvature
        && !pen.commit_failed
        && app.ui.tool == Tool::Pen
        && app.ui.tool_options.vector_mode == VectorMode::Path
        && pen_path_op(&app.ui.tool_options) == Some("combine")
        && targeted_vector_mask(app).is_none()
        && draft_target(app, pen).as_deref().is_none_or(|n| n == "work")
}

/// The live write of this draft just made a History state: record the id History minted for it
/// (`History::row_id` of the current row), which is what `sync_live_draft_from_layer` accepts as the Pen's own
/// click. History mints these ids and never reuses them, and no command parameter names one, so a `path.set` from
/// the control channel or a script cannot leave this mark even when it carries a Pen step name (#16686 review).
fn record_live_step(app: &mut PhotocraftApp) {
    let (Some(st), Some(pen)) = (app.session.active(), app.ui.pen.as_mut()) else { return };
    if let Some(id) = st.history.row_id(st.history.past_len()) {
        pen.live_steps.push(id);
    }
}

/// The first anchor makes the Work Path with that one anchor in it; the draft then carries the path's new subpath
/// (`resumes`, `live_path`), so each further click is a step of its own. With the Work Path selected in the Paths panel
/// the click joins it, which the reference names "New Path Component"; with nothing selected it replaces it: "New Work
/// Path". A refused write leaves the draft as it was: the anchors go in at the next click or at the finish.
pub(super) fn start_live_path(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| starts_live_path(app, p)) else { return };
    let append = draft_target(app, pen).is_some();
    // The subpath the write puts in place: joining a path adds it at the end of what is there, so the count the path
    // had is its index. A replacing write leaves the drawn path the document's only one (`paths.rs`: `path.set` without
    // `op`), however many subpaths the replaced path had, so its subpath is the one at index zero.
    let index = if append { app.session.active().map_or(0, |st| st.doc.work_path.as_ref().map_or(0, |p| p.subpaths.len())) } else { 0 };
    let mut params = json!({"name": "work", "path": path_json(&pen_path(pen)), "historyName": if append { "New Path Component" } else { "New Work Path" }});
    if append {
        params["op"] = json!("combine");
    }
    if let Err(e) = run_beside_draft(app, "path.set", params) {
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    record_live_step(app);
    let Some(mut pen) = app.ui.pen.take() else { return };
    let made: Option<(u64, photocraft_doc::vector::Subpath)> =
        app.session.active().and_then(|st| Some((st.doc.id.0, st.doc.work_path.as_ref()?.subpaths.get(index)?.clone())));
    if let Some((doc, was)) = made {
        pen.resumes = Some(Resumed { doc, path: "work".into(), layer: None, index, was });
        pen.live_path = true;
        pen.kept = 0;
    }
    super::pen_resume::rebind_owner(app, &mut pen);
    app.ui.pen = Some(pen);
}

/// Is `pen` a draft carried on from the end point of an open subpath of the Work Path, drawn live like a fresh one
/// (#12890)? The press that picked it up is the step "Pick-up Path". The Curvature Pen and Freeform Pen, a saved path,
/// a vector mask and the other Path Operations were not measured and keep today's route. A Work Path that a finish hid
/// (#12892) is not drawn, so nothing can be carried on from it: `resume_target` refuses it the same way, and minting
/// from it here would put a live path back beside a hidden one.
fn carries_on_live_path(app: &PhotocraftApp, pen: &PenPath) -> bool {
    !pen.curvature
        && !pen.live_layer
        && !pen.live_path
        && !pen.commit_failed
        && app.ui.tool == Tool::Pen
        && app.ui.tool_options.vector_mode == VectorMode::Path
        && pen_path_op(&app.ui.tool_options) == Some("combine")
        && targeted_vector_mask(app).is_none()
        && work_path_shown(app)
        && pen.resumes.as_ref().is_some_and(|r| r.layer.is_none() && r.path == "work")
}

/// The press on an open Work Path's end point: the path goes on being drawn live from there, and the press is its own
/// step "Pick-up Path". The subpath takes the draft's order, as the finish of a carried-on path always did: picked up
/// at its first anchor, the draft runs it backwards (#6593), and every further click compares with that order.
pub(super) fn pick_up_live_path(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| carries_on_live_path(app, p)) else { return };
    let Some(r) = pen.resumes.as_ref() else { return };
    let drawn = path_json(&pen_path(pen));
    let (name, full) = (r.path.clone(), app.session.active().and_then(|st| with_resumed_subpath(&st.doc, r, &drawn)));
    let Some(full) = full else { return };
    match run_beside_draft(app, "path.set", json!({"name": name, "path": full, "historyName": "Pick-up Path"})) {
        // Only a write that went through makes the draft live: a refused one keeps today's route, with the anchors in
        // the draft and the finish taking them, and leaves no live path behind (#12890).
        Ok(_) => {
            record_live_step(app);
            if let Some(pen) = app.ui.pen.as_mut() {
                pen.live_path = true;
            }
            follow_live_layer(app);
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The History name of one click's change to the live Work Path (#12890), from what the path holds and what the draft
/// has now: its close, a new anchor at its end, a retracted handle (the press on the last smooth anchor, which the
/// reference names "Pick-up Path") or an anchor the pointer dragged. `None` for a change the issue did not measure —
/// a handle dragged alone, an anchor removed or one inserted between others: that stays in the draft until the finish
/// takes it with the command's own name.
fn live_step_name(path: &photocraft_doc::vector::Subpath, drawing: &photocraft_doc::vector::Subpath) -> Option<&'static str> {
    if drawing.closed && !path.closed {
        return Some("Close Path");
    }
    if drawing.knots.len() > path.knots.len() && drawing.knots[..path.knots.len()] == path.knots[..] {
        return Some("New Anchor Point");
    }
    if drawing.knots.len() != path.knots.len() || drawing.closed != path.closed {
        return None;
    }
    if drawing.knots.iter().zip(&path.knots).any(|(a, b)| a.anchor != b.anchor) {
        return Some("Drag Anchor Point");
    }
    // ⌥-click on the last smooth anchor: its outgoing handle is gone, and that press is "Pick-up Path" too.
    let retracted = |k: &photocraft_doc::Knot| !k.smooth && k.out_ctrl == k.anchor;
    (path.knots.last().is_some_and(|k| !retracted(k)) && drawing.knots.last().is_some_and(retracted)).then_some("Pick-up Path")
}

/// The live Work Path takes the draft's anchors as one step, when a click changed them and the change has a measured
/// name (`host::drain`). A refused edit is reported; the anchors stay in the draft for the next click or the finish.
pub(super) fn sync_live_path_from_draft(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref().filter(|p| p.live_path && !p.commit_failed && !p.draft.knots.is_empty()) else { return };
    let Some(r) = pen.resumes.as_ref().filter(|r| r.layer.is_none()) else { return };
    let drawn = pen_path(pen);
    let Some(st) = app.session.active() else { return };
    let Some(path) = live_subpath(&st.doc, pen).cloned() else { return };
    let Some(drawing) = drawn.subpaths.first() else { return };
    if same_subpath(&path, drawing) {
        return;
    }
    let Some(label) = live_step_name(&path, drawing) else { return };
    let Some(full) = with_resumed_subpath(&st.doc, r, &path_json(&drawn)) else { return };
    match run_beside_draft(app, "path.set", json!({"name": r.path.clone(), "path": full, "historyName": label})) {
        Ok(_) => {
            record_live_step(app);
            follow_live_layer(app);
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// What the finish of a live Work Path draft still owes the document (#12890): its clicks were steps of their own, so
/// ↩ and Esc add none. `Close` is the closing of a path whose anchors are in it, and `Rest` the anchors a write
/// refused or did not name, which go in under the command's own label.
enum LiveFinish {
    Skip,
    Close,
    Rest,
}

/// [`LiveFinish`] for `pen`'s draft, or `None` while it is not a live Work Path draft.
fn live_finish(app: &PhotocraftApp, pen: &PenPath) -> Option<LiveFinish> {
    if !pen.live_path {
        return None;
    }
    let st = app.session.active()?;
    let path = live_subpath(&st.doc, pen)?;
    let drawing = pen_path(pen).subpaths.first()?.clone();
    if same_subpath(path, &drawing) {
        return Some(LiveFinish::Skip);
    }
    Some(if path.knots == drawing.knots { LiveFinish::Close } else { LiveFinish::Rest })
}

/// The Pen's Path Operations (#1397), in Photoshop's menu order: the `pen_path_op` value, its
/// label and its icon.
pub(crate) const PATH_OPS: [(&str, &str, &str); 5] = [
    ("newLayer", "New Layer", "path-op-new-layer"),
    ("combine", "Combine Shapes", "path-op-combine"),
    ("subtract", "Subtract Front Shape", "path-op-subtract"),
    ("intersect", "Intersect Shape Areas", "path-op-intersect"),
    ("exclude", "Exclude Overlapping Shapes", "path-op-exclude"),
];

/// The operation (`path.set` / `shape.create` `op`) a new Pen path is added with; None for a new
/// shape layer (New Layer, in Shape mode). Path mode has no New Layer: there it combines.
pub(crate) fn pen_path_op(o: &ToolOptions) -> Option<&'static str> {
    let op = PATH_OPS.iter().map(|(v, ..)| *v).find(|v| *v == o.pen_path_op).unwrap_or("newLayer");
    match op {
        "newLayer" if o.vector_mode == VectorMode::Shape => None,
        "newLayer" => Some("combine"),
        op => Some(op),
    }
}

/// The operation (`shape.create` `op`) a Shape-tool drag is added with (#13235); None for a new
/// shape layer (New Layer, in Shape mode). Path mode has no New Layer: there it is Exclude
/// Overlapping Shapes, as the reference application's default.
pub(crate) fn shape_path_op(o: &ToolOptions) -> Option<&'static str> {
    let op = PATH_OPS.iter().map(|(v, ..)| *v).find(|v| *v == o.shape_path_op).unwrap_or("newLayer");
    match op {
        "newLayer" if o.shape_mode == VectorMode::Shape => None,
        "newLayer" => Some("exclude"),
        op => Some(op),
    }
}

/// Whether the active tool draws shape layers (so New Layer applies): the Shape tools in Shape
/// mode, the Pens in Shape mode; Path Selection and Direct Selection have no shape layer to make.
pub(super) fn tool_in_shape_mode(app: &PhotocraftApp) -> bool {
    let (tool, o) = (app.ui.tool, &app.ui.tool_options);
    if matches!(tool, Tool::PathSelection | Tool::DirectSelection) {
        false
    } else if super::shapes::is_shape_tool(tool) {
        o.shape_mode == VectorMode::Shape
    } else {
        o.vector_mode == VectorMode::Shape
    }
}

/// The active tool's resolved operation (see `pen_path_op` and `shape_path_op`).
pub(super) fn tool_path_op(app: &PhotocraftApp) -> Option<&'static str> {
    if super::shapes::is_shape_tool(app.ui.tool) { shape_path_op(&app.ui.tool_options) } else { pen_path_op(&app.ui.tool_options) }
}

/// The Path Operations setting the active tool keeps: the Shape tools' own, else the Pens'.
pub(crate) fn path_op_field(app: &mut PhotocraftApp) -> &mut String {
    if super::shapes::is_shape_tool(app.ui.tool) { &mut app.ui.tool_options.shape_path_op } else { &mut app.ui.tool_options.pen_path_op }
}

/// What the Path Operations menu's Merge Shape Components merges, as a command, its params and
/// the number of shape components there: with Path Selection or Direct Selection the path they
/// select in (with Direct Selection the one with selected anchors first); in Shape mode the
/// selected shape layer; in Path mode the path the Pen draws into (a targeted vector mask, the
/// selected path, else the Work Path).
pub(super) fn merge_target(app: &PhotocraftApp) -> Option<(&'static str, Value, usize)> {
    if matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection) {
        let anchored = crate::path_components::selected(app).filter(|_| app.ui.tool == Tool::DirectSelection).map(|(r, p, _)| (r, p));
        let (r, p) = anchored.or_else(|| crate::direct_select::active_paths(app).into_iter().next())?;
        return Some(("path.mergeShapeComponents", r.params(app), p.components().len()));
    }
    let st = app.session.active()?;
    if tool_in_shape_mode(app) {
        let id = st.active_layer?;
        let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
        return Some(("layer.combineShapes.mergeShapeComponents", json!({"layer": id.0}), sh.path.components().len()));
    }
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some(("path.mergeShapeComponents", json!({"name": "layer", "layer": id}), p.components().len()));
    }
    let name = app.ui.selected_path.clone().filter(|n| path_exists(&st.doc, n)).unwrap_or_else(|| "work".into());
    let path = if name == "work" { st.doc.work_path.as_ref() } else { st.doc.paths.iter().find(|p| p.name == name).map(|p| &p.path) }?;
    Some(("path.mergeShapeComponents", json!({"name": name}), path.components().len()))
}

// ---------------------------------------------------------------------------------------------
// Freeform Pen

/// [`freeform_finish_owned`]'s fit and commit of a Freeform Pen trace to the destination captured when the trace began.
/// A trace of another document commits nothing; a failed commit keeps the fitted path as a draft. Releasing within the
/// reach of the start point closes the path; a click draws nothing. The points at indices `straight` were ⌥-clicked: the
/// segment into each one stays straight.
pub(crate) fn freeform_finish_owned(app: &mut PhotocraftApp, points: &[[f64; 3]], straight: &[usize], owner: PenOwner) {
    freeform_finish_continued(app, points, straight, owner, None);
}

/// Fits a trace started at the final anchor of a selected, open Work Path subpath.
pub(super) fn freeform_finish_continued(app: &mut PhotocraftApp, points: &[[f64; 3]], straight: &[usize], owner: PenOwner, continuation: Option<usize>) {
    if !owner.valid(app) {
        return;
    }
    let trace: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    let (Some(first), Some(last)) = (trace.first(), trace.last()) else { return };
    let zoom = app.point_zoom().max(0.01) as f64;
    // Photoshop closes a release within 3 screen px of the start on each axis, a square (#12830).
    let reach = 3.0 / zoom;
    // The trace must still be longer than the former 4 × 6 screen px to close: unchanged.
    let min_length = 24.0 / zoom;
    let length: f64 = trace.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
    let closed = near_start(first, last, reach) && length > min_length;
    let fit = f64::from(app.ui.tool_options.curve_fit);
    let (mut pen, closed) = if straight.is_empty() {
        let Some(sub) = photocraft_vector::trace::fit_trace(&trace, fit, closed) else { return };
        let knots = sub.knots.iter().map(edit_knot).collect();
        (PenPath { draft: EditSubpath::new(knots, sub.closed), ..Default::default() }, sub.closed)
    } else {
        fit_with_straight_segments(&trace, straight, fit, closed)
    };
    if let Some(subpath) = continuation
        && !closed
        && let Some((_, _, path)) = resume_target(app)
        && let Some(sub) = path.subpaths.get(subpath)
        && let Some(first) = sub.knots.first()
        && !near_start(&[first.anchor.x, first.anchor.y], last, reach)
        && let Some(mut joined) = resume_end(app, subpath, sub.knots.len() - 1)
        && let Some(join) = joined.draft.knots.last_mut()
    {
        // The recorded shared anchor is a corner; its incoming handle stays as it was.
        join.out_ctrl = join.anchor;
        join.smooth = false;
        joined.draft.knots.extend(pen.draft.knots.into_iter().skip(1));
        pen = joined;
    }
    pen.owner = Some(owner);
    if let Err(e) = commit_path_named(app, &pen, closed, true) {
        app.ui.pen = Some(PenPath { commit_failed: true, freeform_origin: true, ..pen });
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Is the Freeform Pen's `end` within `reach` of its `start` on both axes?
fn near_start(start: &[f64; 2], end: &[f64; 2], reach: f64) -> bool {
    // The screen-to-document conversion runs in f32, so a release exactly `reach` off can land a hair past it.
    let reach = reach + 1e-3;
    (start[0] - end[0]).abs() <= reach && (start[1] - end[1]).abs() <= reach
}

/// A Freeform Pen trace with ⌥-clicked straight segments (into the points at `straight`): each
/// freehand run between them is fitted on its own, and the runs are joined by straight lines
/// between corner anchors. `close`: the release was near the start, so the last anchor becomes
/// the first one. The fitted last anchor is not tested again: fitting can move it past the reach.
fn fit_with_straight_segments(trace: &[[f64; 2]], straight: &[usize], fit: f64, close: bool) -> (PenPath, bool) {
    let mut cuts: Vec<usize> = straight.iter().copied().filter(|&i| i > 0 && i < trace.len()).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.push(trace.len());
    let mut knots: Vec<Knot> = Vec::new();
    let mut start = 0;
    for end in cuts {
        let run = trace.get(start..end).unwrap_or_default();
        let first = knots.len();
        match photocraft_vector::trace::fit_trace(run, fit, false) {
            Some(sub) => knots.extend(sub.knots.iter().map(edit_knot)),
            // A single click (or a run that never moved): one corner anchor.
            None => knots.extend(run.first().map(|p| Knot::corner(Point::new(p[0], p[1])))),
        }
        // Where a straight segment meets the run: a corner with no handle on that side.
        if first > 0
            && let Some(k) = knots.get_mut(first)
        {
            k.in_ctrl = k.anchor;
            k.smooth = false;
        }
        if end < trace.len()
            && let Some(k) = knots.last_mut()
        {
            k.out_ctrl = k.anchor;
            k.smooth = false;
        }
        start = end;
    }
    let closed = close && knots.len() > 3;
    if closed
        && let Some(last) = knots.pop()
        && let Some(k) = knots.first_mut()
    {
        k.in_ctrl = Point::new(last.in_ctrl.x - last.anchor.x + k.anchor.x, last.in_ctrl.y - last.anchor.y + k.anchor.y);
        k.smooth = false;
    }
    (PenPath { draft: EditSubpath::new(knots, closed), ..Default::default() }, closed)
}

#[cfg(test)]
use super::capture_pen_owner;
use super::{PenOwner, cancel_stale_pen};

/// Commits a draft with the normal history labels used by Pen and Curvature Pen.
fn commit_path(app: &mut PhotocraftApp, pen: &PenPath, closed: bool) -> Result<(), String> {
    commit_path_named(app, pen, closed, false)
}

/// Commits a draft with its captured owner and returns the refusal message on failure.
/// `freeform` changes history labels only for document paths and newly created shape layers.
fn commit_path_named(app: &mut PhotocraftApp, pen: &PenPath, closed: bool, freeform: bool) -> Result<(), String> {
    let Some(owner) = pen.owner.clone().filter(|o| o.valid(app)) else { return Err("Pen draft target is stale; no path was changed".into()) };
    let result = as_owner(app, &owner, |app| {
        let result = commit_path_current(app, pen, closed, freeform);
        if result && matches!(owner.target, super::PenTarget::Shape { .. } | super::PenTarget::VectorMask(_)) {
            app.ui.selected_path = Some("layer".into());
        }
        result
    });
    if result { Ok(()) } else { Err(app.ui.status.clone()) }
}

/// Run `f` with the options, colours, active layer, Paths panel selection and vector mask target
/// the draft's `owner` captured when drawing began, then put the current ones back (the Paths
/// panel selection stays as `f` left it).
fn as_owner<R>(app: &mut PhotocraftApp, owner: &PenOwner, f: impl FnOnce(&mut PhotocraftApp) -> R) -> R {
    let options = std::mem::replace(&mut app.ui.tool_options, owner.options.clone());
    let vector_mask = std::mem::replace(&mut app.ui.vector_mask_target, owner.vector_mask);
    app.ui.selected_path = match &owner.target {
        super::PenTarget::Path { name, append } => append.then(|| name.clone()),
        _ => None,
    };
    app.session.set_active_layer(owner.layer);
    let foreground = std::mem::replace(&mut app.session.tools.foreground, owner.foreground);
    let background = std::mem::replace(&mut app.session.tools.background, owner.background);
    let channel = app.session.active().map(|st| st.channel_view.target);
    if let Some(st) = app.session.active_mut() {
        st.channel_view.target = if matches!(owner.target, super::PenTarget::VectorMask(_)) {
            photocraft_engine::channel_cmds::ChannelTarget::VectorMask
        } else {
            photocraft_engine::channel_cmds::ChannelTarget::Composite
        };
    }
    let result = f(app);
    if let (Some(st), Some(channel)) = (app.session.active_mut(), channel) {
        st.channel_view.target = channel;
    }
    app.session.tools.foreground = foreground;
    app.session.tools.background = background;
    app.ui.tool_options = options;
    app.ui.vector_mask_target = vector_mask;
    result
}

/// Make Shape on a Pen draft (#10934), as one History step: a draft begun in Shape mode is
/// committed (it is a shape already, and a resumed subpath keeps its layer's other subpaths);
/// otherwise a new shape layer, with the shape tools' fill and stroke, of what the commit would
/// have made, worked out as the commit does, with the settings drawing began with.
pub(super) fn make_shape_from_draft(app: &mut PhotocraftApp, pen: &PenPath) -> Result<Value, String> {
    let owner = pen.owner.clone();
    if owner.as_ref().is_some_and(|o| !o.valid(app)) {
        return Err("Pen draft target is stale; no path was changed".into());
    }
    if owner.as_ref().is_some_and(|o| matches!(o.target, super::PenTarget::Shape { .. })) {
        return try_pen_commit(app, false).map(|()| Value::Null);
    }
    let selected = app.ui.selected_path.clone();
    let params = |app: &mut PhotocraftApp| {
        let (path, consume) = draft_shape_path(app, pen).ok_or_else(|| tl!("No document").to_string())?;
        let params = json!({"path": path, "consumeWorkPath": consume, "useAlignedRendering": app.ui.tool_options.align_edges, "fill": fill_param(app), "stroke": stroke_param(app)});
        Ok::<_, String>((params, consume))
    };
    let made = match &owner {
        Some(o) => as_owner(app, o, params),
        None => params(app),
    };
    app.ui.selected_path = selected;
    let (params, consume) = made?;
    let r = app.run("path.toShape", params);
    if r.is_ok() && consume && app.ui.selected_path.as_deref() == Some("work") {
        app.ui.selected_path = None;
    }
    r
}

#[cfg(test)]
#[path = "freeform_history_name_tests.rs"]
mod freeform_history_name_tests;

#[cfg(test)]
#[path = "freeform_close_tolerance_tests.rs"]
mod freeform_close_tolerance_tests;

#[cfg(test)]
#[path = "pen_shape_first_click_tests.rs"]
mod pen_shape_first_click_tests;

#[cfg(test)]
#[path = "pen_history_steps_tests.rs"]
mod pen_history_steps_tests;
