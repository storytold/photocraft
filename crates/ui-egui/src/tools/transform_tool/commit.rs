use craft::behavior::EditEvent;
use craft::behavior::kurbo::Point;
use craft::draw::ui::transform::{TransformEdit, TransformRequest};

use super::begin::{fold_made, moves_kind, moving_layers, rect_to_quad_affine, take_back_made};
use super::{
    Arc, DocId, FreeTransform, JobEvent, JobId, JobOutcome, LayerContent, LayerId, MadeLayer, OpenPreview, PhotocraftApp, SessionWarp, Several, json,
    preset_for_command, swap_mode_texels,
};

/// Undo and Redo while the commit runs in the background (Free Transform, or Warp).
const STILL_APPLYING: &str = "Free Transform is still being applied";

const STILL_WARPING: &str = "Warp is still being applied";

pub fn commit(app: &mut PhotocraftApp) {
    if committing(app).is_some() {
        return;
    }
    let document = app.transform_preview.as_ref().map(|pv| pv.doc.id);
    if document.is_some_and(|id| app.session.active().is_none_or(|st| st.doc.id != id)) {
        cancel(app);
        return;
    }
    let Some(mut t) = app.ui.transform.take() else { return };
    let mut preview = app.transform_preview.take();
    // An untouched box closes without a step: a ⌥⌘T copy or a place stays.
    let result = match t.session.commit() {
        TransformRequest::Apply(result) => Some(result),
        _ => None,
    };
    if crate::frame_tool::frame_transform(&t) {
        app.frame_preview = None;
        if result.is_some() {
            let done = crate::frame_tool::transformed_frame(app, &t)
                .and_then(|transform| app.run("layer.frame.set", json!({"layer": t.layer, "transform": transform.m})));
            if let Err(error) = done {
                keep_open(app, t, preview, error);
            }
        }
        return;
    }
    if let Some(options) = &t.content_aware {
        crate::cas_transform::commit(app, &t, options);
        return;
    }
    let Some(result) = result else { return };
    let rect = [result.rect.x0, result.rect.y0, result.rect.x1, result.rect.y1];
    let quad = result.state.quad.map(|p| [p.x, p.y]);
    let interpolation = result.interpolation.id();
    let warp = result.warp.map(|w| w.warp);
    if t.selection {
        let mut p = json!({"rect": rect, "interpolation": interpolation});
        match &warp {
            Some(w) => p["warp"] = json!(w),
            None => p["quad"] = json!(quad),
        }
        if let Err(e) = app.run("select.transformSelection", p) {
            keep_open(app, t, preview, e);
        }
        return;
    }
    if let Some(target) = &t.path {
        let mut p = target.clone();
        match &warp {
            Some(w) => p["warp"] = json!(w),
            // Distort and Perspective map the path through the box (#3027).
            None => match rect_to_quad_affine(rect, quad) {
                Some(a) => p["matrix"] = json!(a.m),
                None => {
                    p["rect"] = json!(rect);
                    p["quad"] = json!(quad);
                }
            },
        }
        if let Err(e) = app.run("path.transform", p) {
            keep_open(app, t, preview, e);
        }
        return;
    }
    let made = t.made;
    let r = if let Some(w) = &warp {
        // The warp's bounds are the frame it was built over (a stored smart warp's, or Warp Text's).
        app.run("edit.transform.warp", json!({"layer": t.layer, "rect": w.bounds, "warp": w, "interpolation": interpolation}))
    } else {
        let mut p = json!({"rect": rect, "quad": quad, "interpolation": interpolation});
        if t.layers.is_empty() {
            p["layer"] = json!(t.layer);
        } else {
            p["layers"] = json!(t.layers);
        }
        if let Some(target) = &t.target {
            p["target"] = target.clone();
        }
        app.run("edit.transform", p)
    };
    // A large layer's warp runs in the background: the session stays until the job ends.
    if let Ok(v) = &r
        && let Some(job) = v.get("pending").and_then(serde_json::Value::as_bool).filter(|p| *p).and(v.get("job")).and_then(serde_json::Value::as_u64)
        && let Some(pv) = preview.as_mut()
    {
        pv.committing = Some(JobId(job));
        app.ui.transform = Some(t);
        app.transform_preview = preview;
        return;
    }
    match r {
        // Nothing changed (an untouched cylinder or Warp Text): no step to fold a copy into, so
        // it stays its own step, as for an untouched box.
        Ok(v) if v.get("changed") == Some(&json!(false)) => {}
        Ok(_) => finish(app, document, LayerId(t.layer), made, true),
        Err(e) => keep_open(app, t, preview, e),
    }
}

/// A click outside the box, at `at` in document units (`ApplyBy::ClickOutside`): clicking the pasteboard or
/// another layer's pixels commits; a click outside the box where only the Background has pixels does nothing and
/// the box stays open (#11100). The commit doesn't pick the layer under the click: as in the reference
/// application, that takes a second click (#8208). A click moves nothing, so there is no box to restore.
pub fn apply_click(app: &mut PhotocraftApp, at: Point) {
    let at = [at.x, at.y];
    let Some(t) = app.ui.transform.as_ref() else { return };
    if on_pasteboard(app, at) || on_other_layer(app, t, at) {
        commit(app);
    }
}

/// Commits the Frame tool's auto-opened session when one of this pass's `edits` ends a box gesture, and clears the
/// flag. It reads only the edits it is given, so a second pass of a frame commits nothing the first one did.
/// `true` when it committed.
pub fn auto_commit(app: &mut PhotocraftApp, edits: &[TransformEdit<SessionWarp>]) -> bool {
    let ends_gesture = edits.iter().any(|e| matches!(e, TransformEdit::Box(EditEvent::Commit { .. })));
    if !ends_gesture || !app.ui.frame_tool.auto_transform {
        return false;
    }
    app.ui.frame_tool.auto_transform = false;
    commit(app);
    true
}

/// Whether a document point is outside the document, on the pasteboard around the canvas.
fn on_pasteboard(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.session.active().is_some_and(|st| p[0] < 0.0 || p[1] < 0.0 || p[0] >= f64::from(st.doc.size.width) || p[1] >= f64::from(st.doc.size.height))
}

/// Whether a document point lands on pixels of a layer the box doesn't move: the frontmost hit
/// there, skipping the moved layers and fully locked ones as Auto-Select does, is a visible
/// top-level layer that is not the Background. As measured in the reference application
/// (#11100, #8208), a click there commits a layer transform and leaves the selection as it was;
/// a second click picks that layer. Not measured, so the box stays open as before: Transform
/// Selection, a path or mask target, and hidden, locked or grouped layers (also when a grouped
/// layer covers a top-level one at the click).
fn on_other_layer(app: &PhotocraftApp, t: &FreeTransform, p: [f64; 2]) -> bool {
    if t.selection || t.path.is_some() || t.target.is_some() || !p[0].is_finite() || !p[1].is_finite() {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let doc = &st.doc;
    // The layers the commit moves: `edit.transform` takes the link group along as it is then, so a
    // layer linked after the box opened counts as moved too.
    let moved = photocraft_engine::transform_cmds::box_targets(doc, &moving_layers(t));
    let moved = |id: photocraft_doc::LayerId| id.0 == t.layer || moved.contains(&id);
    // `as` saturates out-of-range floats; `layers_at` returns nothing off the canvas.
    photocraft_engine::pick_cmds::layers_at(doc, p[0].floor() as i32, p[1].floor() as i32)
        .into_iter()
        .find(|id| !moved(*id) && !doc.effective_locks(*id).all)
        .is_some_and(|id| !doc.is_background(id) && doc.layers.iter().any(|l| l.id == id && l.artboard().is_none()))
}

/// After the transform landed or not: fold the layer the session made into its step, or take it
/// back.
fn finish(app: &mut PhotocraftApp, document: Option<DocId>, layer: LayerId, made: Option<MadeLayer>, landed: bool) {
    let (Some(made), Some(document)) = (made, document) else { return };
    if landed {
        fold_made(app, document, layer, made);
    } else {
        take_back_made(app, document, layer);
    }
}

/// The job applying the transform, while it runs.
pub(crate) fn committing(app: &PhotocraftApp) -> Option<JobId> {
    app.transform_preview.as_ref().and_then(|pv| pv.committing)
}

/// A job ended (see [`crate::jobs_ui`]): when it was the commit, the session ends with it. Its
/// status and errors are reported like any job's.
pub fn on_job_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if super::begin::on_prepared(app, e) {
        return;
    }
    if committing(app) != Some(e.id) {
        return;
    }
    let after = app.after_transform_commit.take();
    let (Some(t), Some(mut pv)) = (app.ui.transform.take(), app.transform_preview.take()) else { return };
    // A refused commit (a result over the pixel budget, #3773) keeps the box open, as inline,
    // unless picking another tool or layer started it: then the box ends, as Esc would.
    if let JobOutcome::Failed(msg) = &e.outcome {
        pv.committing = None;
        keep_open(app, t, Some(pv), msg.clone());
        if switched_away(app) {
            cancel(app);
        }
        return;
    }
    let document = pv.doc.id;
    let landed = matches!(e.outcome, JobOutcome::Done(_));
    finish(app, Some(document), LayerId(t.layer), t.made, landed);
    // Not in another tab, which the click wasn't meant for.
    if let Some((id, params)) = after.filter(|_| landed && app.session.active().is_some_and(|st| st.doc.id == document))
        && let Err(e) = app.run(&id, params)
    {
        // A notice, as the commit's own status replaces the status bar's text next (jobs_ui).
        crate::notices::error(app, e);
    }
}

/// A commit the engine refused (a result over the pixel budget, #3773): the box stays open as it
/// was, with the reason in the status bar, so the transform can be changed or cancelled. A copy or
/// a placed layer it made goes only when it is cancelled.
fn keep_open(app: &mut PhotocraftApp, t: FreeTransform, preview: Option<OpenPreview>, e: String) {
    app.ui.transform = Some(t);
    app.transform_preview = preview;
    app.ui.status = e;
}

/// A transform whose layer or document went away (undo, close) ends silently; picking another tool
/// or selecting another layer (Layers panel, ⌘-click, the control channel) applies it, so the box
/// never stays on a layer that is no longer the active one (#1400). Checked every frame and before
/// each pointer event, so a press with a tool chosen just before it (`ui.pointer`'s `tool`) goes to
/// that tool.
pub fn end_if_left(app: &mut PhotocraftApp) {
    let Some(t) = &app.ui.transform else { return };
    if committing(app).is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return cancel(app) };

    let layer = LayerId(t.layer);
    let made = t.made;
    let preview_doc_id = app.transform_preview.as_ref().map(|pv| pv.doc.id);
    let switched_document = preview_doc_id.is_some_and(|id| id != st.doc.id);

    let editing_placed_smart_object = switched_document
        && made == Some(MadeLayer::Place)
        && preview_doc_id
            .is_some_and(|parent_id| app.session.smart_links.iter().any(|link| link.parent == parent_id && link.child == st.doc.id && link.layer == layer));

    if editing_placed_smart_object {
        app.ui.transform = None;
        app.transform_preview = None;
    } else if switched_document || moving_layers(t).iter().any(|id| st.doc.layer(*id).is_none()) || st.doc.layer(layer).is_none() {
        cancel(app);
    } else if switched_away(app) {
        commit(app);
        // Picking another tool or layer can't leave a refused transform open: it ends, as Esc would.
        // A commit still running in the background isn't refused: cancelling it would throw the
        // transform away (and a dropped file with it, #5665). It ends the session when it lands,
        // or ends it as Esc would if it is refused then (`on_job_event`).
        if app.ui.transform.is_some() && committing(app).is_none() {
            cancel(app);
        }
    }
}

/// Another tool picked, or (outside Transform Selection) another layer selected or the selected
/// layers changed, since the box opened: the session ends ([`end_if_left`], #1400, #11101).
fn switched_away(app: &PhotocraftApp) -> bool {
    let (Some(t), Some(st)) = (&app.ui.transform, app.session.active()) else { return false };
    let opened = app.transform_preview.as_ref().and_then(|pv| pv.follow.as_ref()).and_then(|f| f.selected.as_deref());
    app.transform_preview.as_ref().is_some_and(|pv| pv.tool != app.ui.tool)
        || (!t.selection && st.active_layer.is_some_and(|active| active.0 != t.layer))
        || selected_layers_changed(st, t, opened)
}

/// A layer's box no longer matches the Layers panel's selection it opened with (`opened`): a layer
/// was added to the selection or taken out of it, a group's child included. Photoshop applies the
/// box first, to the layers it moves, and doesn't open it again (#11101). The selection itself is
/// compared, not the layers the box moves: those also hold the layers linked to it (#11217) and
/// leave out the children of a selected group. Boxes over a selection, a mask or channel, a path,
/// a placed file or Content-Aware Scale don't follow the selection.
fn selected_layers_changed(st: &photocraft_engine::DocState, t: &FreeTransform, opened: Option<&[LayerId]>) -> bool {
    if t.selection || t.target.is_some() || t.path.is_some() || t.made == Some(MadeLayer::Place) || t.content_aware.is_some() {
        return false;
    }
    let Some(opened) = opened else { return false };
    let mut now = st.selected_layers().to_vec();
    now.sort_unstable();
    now != opened
}

/// Edits made while the box is open show in the preview at once, not only after the commit
/// (#1384): the moving layer's opacity, fill and visibility tint the moving pixels, and the
/// document under them (other layers, and the part of the layer a selection leaves behind) is
/// rebuilt from the current one. The moving pixels keep the session's. Called before the canvas
/// reads the preview, so it never caches a revision with the old preview.
pub fn follow_edits(app: &mut PhotocraftApp) {
    let (Some(t), Some(pv)) = (&app.ui.transform, app.transform_preview.as_mut()) else { return };
    let Some(follow) = pv.follow.as_mut() else { return };
    let Some(st) = app.session.active().filter(|st| st.doc.id == pv.doc.id && st.revision != follow.revision) else { return };
    let ids = moving_layers(t);
    let mut pd = (*st.doc).clone();
    for id in &ids {
        let (Some(_), Some(moving)) = (st.doc.layer(*id), pv.doc.layer(*id)) else { return };
        if let Some(l) = pd.layer_mut(*id) {
            l.content = moving.content.clone();
            l.visible = l.visible && !follow.hidden.contains(id);
        }
    }
    if let (Some(a), Some(top)) = (pv.above.as_mut(), ids.last()) {
        a.rebuild(&st.doc, *top);
        a.hide_in(&mut pd.layers);
    }
    // Several layers' opacity, fill, visibility and styles are in their composite: composite them
    // again when an edit changed how they look.
    if let Some(sv) = pv.several.as_mut() {
        let now = Several::shown(&st.doc, &sv.ids, &sv.lifted);
        if now.layers != sv.moving.layers {
            sv.moving = now;
            pv.textures = sv.render();
        }
    }
    if t.layers.is_empty()
        && let Some(live) = st.doc.layer(LayerId(t.layer))
    {
        pv.opacity = if live.visible { live.opacity * live.fill_opacity } else { 0.0 };
    }
    follow.revision = st.revision;
    pv.doc = Arc::new(pd);
}

pub fn cancel(app: &mut PhotocraftApp) {
    if app.ui.transform.as_ref().is_some_and(crate::frame_tool::frame_transform) {
        app.frame_preview = None;
    }
    // A frame drag's session (frame or content) ends here; a later one must not commit on release.
    app.ui.frame_tool.auto_transform = false;
    // Esc while the commit runs cancels the job; the session ends when it does.
    if let Some(job) = committing(app) {
        crate::jobs_ui::cancel(app, job);
        return;
    }
    let transform = app.ui.transform.take().map(|mut t| {
        let _ = t.session.discard();
        t
    });
    let preview = app.transform_preview.take();
    if let (Some(t), Some(pv)) = (transform, preview)
        && t.made.is_some()
    {
        take_back_made(app, pv.doc.id, LayerId(t.layer));
    }
}

/// Save for Web (Legacy): refused under an open box, as it would export without the preview (#6737).
pub(crate) const SAVE_FOR_WEB: &str = "file.export.saveForWebLegacy";

/// Layer › Copy CSS: refused under an open box too, as it would copy the bounds from before it.
const COPY_CSS: &str = photocraft_engine::layer_css_cmds::ID;

/// While transforming, Undo and Redo apply to the box: `Some(enabled)` for those commands (and
/// Toggle Last State, which would change the document under the box), `None` for the rest.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if (matches!(id, "edit.freeTransform" | "edit.freeTransformCopy") || id.starts_with("edit.transform."))
        && crate::vector_ui::free_transform_path(app).is_none()
        && crate::canvas::multiple_color_target(app)
    {
        return Some(false);
    }
    let t = app.ui.transform.as_ref()?;
    if committing(app).is_some() {
        return box_command(id).then_some(false);
    }
    match id {
        // Content-Aware Scale's box only scales: no switching it to another transform or Warp.
        "edit.freeTransform" | "edit.freeTransformCopy" if t.content_aware.is_some() => Some(false),
        i if i.starts_with("edit.transform.") && t.content_aware.is_some() => Some(false),
        "edit.undo" => Some(t.session.can_undo()),
        "edit.redo" => Some(t.session.can_redo()),
        // No Toggle Last State or history jump under the box, and one box at a time.
        "edit.toggleLastState" | "history.selectState" | "edit.contentAwareScale" | "edit.puppetWarp" => Some(false),
        // Save for Web exports the committed document and would ignore the pending box (#6737).
        // Copy CSS, likewise, would copy the layers' bounds from before the box.
        SAVE_FOR_WEB | COPY_CSS => Some(false),
        // Rotate and Flip turn the box (not while warping, where the mesh is the box).
        _ if preset_for_command(id).is_some() => Some(t.session.warp().is_none()),
        // One box over several layers doesn't warp (#6501).
        "edit.transform.warp"
        | "layer.smartObjects.warp"
        | "edit.transform.splitWarpCrosswise"
        | "edit.transform.splitWarpHorizontally"
        | "edit.transform.splitWarpVertically"
        | "edit.transform.removeWarpSplit"
            if !t.layers.is_empty() =>
        {
            Some(false)
        }
        // Editable type can't be distorted (#2630, as #2701 rules for the menus outside a box).
        "edit.transform.distort" | "edit.transform.perspective" if moves_type(app, t) => Some(false),
        _ => None,
    }
}

/// Does the box move an editable type layer (not its selection outline, a lone mask or a path)?
pub(super) fn moves_type(app: &PhotocraftApp, t: &FreeTransform) -> bool {
    !t.selection && t.target.is_none() && t.path.is_none() && moves_kind(app, t, |c| matches!(c, LayerContent::Text(_)))
}

/// While transforming, the box owns the history for every caller (menus, shortcuts, the control
/// channel, MCP): Undo and Redo step through it, and Toggle Last State and History state jumps,
/// which would change the document under the box, are refused. Edit › Transform's Rotate and
/// Flip turn the box itself, as in Photoshop, instead of the layer under it; committing stays
/// with ↩ and the options bar.
/// The commands the open box answers itself ([`is_enabled`], [`intercept`]).
fn box_command(id: &str) -> bool {
    matches!(id, "edit.undo" | "edit.redo" | "edit.toggleLastState" | "edit.contentAwareScale" | "edit.puppetWarp" | SAVE_FOR_WEB | COPY_CSS)
        || preset_for_command(id).is_some()
}

/// `None` for other commands or when not transforming.
pub fn intercept(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    let warping = app.ui.transform.as_ref()?.session.warp().is_some();
    if committing(app).is_some() {
        return box_command(id).then(|| Err(if warping { STILL_WARPING } else { STILL_APPLYING }.into()));
    }
    match id {
        "edit.undo" => Some(Ok(step(app, false))),
        "edit.redo" => Some(Ok(step(app, true))),
        "edit.toggleLastState" | "history.selectState" | "edit.contentAwareScale" | "edit.puppetWarp" | SAVE_FOR_WEB | COPY_CSS => {
            Some(Err("Commit or cancel the transform first".into()))
        }
        _ => {
            let preset = preset_for_command(id)?;
            if warping {
                return Some(Err("Commit or cancel the transform first".into()));
            }
            let now = app.nudge_bundle.now;
            if let Some(t) = app.ui.transform.as_mut() {
                t.session.apply(TransformEdit::Preset(preset), now);
            }
            Some(Ok(json!({"transform": app.ui.transform})))
        }
    }
}

/// Undo (or redo) one step of the open session; nothing mid-drag or with no step to take.
fn step(app: &mut PhotocraftApp, redo: bool) -> serde_json::Value {
    let Some(t) = app.ui.transform.as_mut() else { return serde_json::Value::Null };
    let had_warp = t.session.warp().is_some();
    let moved = if redo { t.session.redo() } else { t.session.undo() };
    if !moved {
        return serde_json::Value::Null;
    }
    // Across the Warp boundary, a stored warp's edit shows other texels (begin.rs): the step
    // brings its own box, so only the texels swap.
    if had_warp != t.session.warp().is_some()
        && let Some(pv) = app.transform_preview.as_mut()
    {
        swap_mode_texels(pv);
    }
    json!({"transform": app.ui.transform})
}
