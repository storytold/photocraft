//! Shape-only live translation. Never mutate the session until the owned release.
use crate::{PhotocraftApp, state::Tool};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Affine;
use serde_json::json;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct State {
    gesture: Option<Gesture>,
    owns_release: bool,
    sequence: u64,
    trace: Option<Trace>,
}
struct Gesture {
    original: Arc<Document>,
    revision: u64,
    layer: LayerId,
    selected: Vec<LayerId>,
    selected_path: Option<String>,
    channel: photocraft_engine::channel_cmds::ChannelTarget,
    tool: Tool,
    start: [f64; 2],
    end: [f64; 2],
    shown_end: Option<[f64; 2]>,
    shown: Option<Arc<Document>>,
}
fn target(app: &PhotocraftApp) -> Option<LayerId> {
    if app.ui.transform.is_some() || app.layer_mask_targeted() || app.ui.vector_mask_target {
        return None;
    }
    let st = app.session.active()?;
    match app.ui.tool {
        Tool::PathSelection => crate::vector_ui::shape_drag_target(app),
        Tool::Move => {
            let d = app.drag.as_ref()?;
            if d.tool != Tool::Move
                || d.modifiers.alt
                || crate::move_mods::duplicated(app)
                || app.session.prefs().interface.show_bounding_box_when_dragging_layer
                || st.selected_layers().len() != 1
            {
                return None;
            }
            let id = st.active_layer?;
            let layer = st.doc.layer(id)?;
            // Mask/floating/multi-layer operations retain their existing Move semantics.
            if layer.mask.is_some() || layer.vector_mask.is_some() || photocraft_engine::float_cmds::floating(st).is_some() {
                return None;
            }
            matches!(layer.content, LayerContent::Shape(_)).then_some(id)
        }
        _ => None,
    }
}
fn movable(doc: &Document, id: LayerId) -> bool {
    let locks = doc.effective_locks(id);
    doc.path_of(id).is_some_and(|path| (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible)))
        && !locks.all
        && !locks.position
        && !locks.pixels
        && doc.layer(id).is_some_and(|l| l.visible && matches!(l.content, LayerContent::Shape(_)))
}
fn valid(app: &PhotocraftApp, g: &Gesture) -> bool {
    app.ui.tool == g.tool
        && app.ui.selected_path == g.selected_path
        && app.ui.transform.is_none()
        && !app.layer_mask_targeted()
        && !app.ui.vector_mask_target
        && (g.tool != Tool::PathSelection || crate::vector_ui::shape_drag_target(app) == Some(g.layer))
        && app.session.active().is_some_and(|st| {
            st.doc.id == g.original.id
                && st.revision == g.revision
                && st.channel_view.target == g.channel
                && st.active_layer == Some(g.layer)
                && st.selected_layers() == g.selected
                && movable(&st.doc, g.layer)
        })
}
fn delta(start: [f64; 2], end: [f64; 2]) -> Option<[f64; 2]> {
    let d = [end[0] - start[0], end[1] - start[1]];
    (start.iter().chain(end.iter()).all(|v| v.is_finite()) && d.iter().all(|v| v.is_finite() && v.abs() <= 1e7)).then_some(d)
}
/// A new press supersedes even a cancelled/refused gesture's late-release tombstone.
pub(crate) fn reset(app: &mut PhotocraftApp) {
    app.shape_drag.gesture = None;
    app.shape_drag.owns_release = false;
}
pub(crate) fn begin(app: &mut PhotocraftApp) {
    app.shape_drag.gesture = None;
    app.shape_drag.owns_release = false;
    #[cfg(not(target_arch = "wasm32"))]
    if app.shape_drag.trace.is_none() && std::env::var_os("PHOTOCRAFT_SHAPE_DRAG_TRACE").is_some_and(|v| v == "1") {
        app.shape_drag.trace = Some(Trace { remaining: 256, last_submit: None });
    }

    let Some(id) = target(app) else {
        trace_route(app, "legacy");
        return;
    };
    trace_route(app, "shape-target");
    // Even a refused shape gesture owns its release: never fall through to legacy movement.
    app.shape_drag.owns_release = true;

    let Some(st) = app.session.active() else { return };
    let Some(d) = app.drag.as_ref() else { return };
    if d.tool != app.ui.tool || delta(d.start, d.start).is_none() || !movable(&st.doc, id) {
        return;
    }
    app.shape_drag.gesture = Some(Gesture {
        original: st.doc.clone(),
        revision: st.revision,
        layer: id,
        selected: st.selected_layers(),
        selected_path: app.ui.selected_path.clone(),
        channel: st.channel_view.target,
        tool: app.ui.tool,
        start: d.start,
        end: d.start,
        shown_end: None,
        shown: None,
    });
}
pub(crate) fn owns(app: &PhotocraftApp) -> bool {
    app.shape_drag.owns_release
}
pub(crate) fn cancel(app: &mut PhotocraftApp) {
    if let Some(g) = app.shape_drag.gesture.take()
        && app.drag.as_ref().is_some_and(|d| d.tool == g.tool && d.start == g.start)
    {
        app.drag = None;
    }
}
pub(crate) fn reconcile(app: &mut PhotocraftApp, focused: bool) {
    if app.shape_drag.gesture.as_ref().is_some_and(|g| !focused || !valid(app, g) || app.drag.as_ref().is_none_or(|d| d.tool != g.tool || d.start != g.start)) {
        cancel(app);
    }
}
pub(crate) fn motion(app: &mut PhotocraftApp, at: [f64; 2]) {
    // The existing Alt-copy workflow intentionally creates a duplicate. Hand it back intact.
    if app.ui.tool == Tool::Move && app.drag.as_ref().is_some_and(|d| d.modifiers.alt || crate::move_mods::duplicated(app)) {
        app.shape_drag.gesture = None;
        app.shape_drag.owns_release = false;
        return;
    }
    let Some(g) = app.shape_drag.gesture.as_ref() else { return };
    if !valid(app, g) || delta(g.start, at).is_none() {
        cancel(app);
        return;
    }
    if let Some(g) = app.shape_drag.gesture.as_mut() {
        g.end = at;
    }
}
pub(crate) fn active(app: &PhotocraftApp) -> bool {
    app.shape_drag.gesture.as_ref().is_some_and(|g| valid(app, g) && app.drag.as_ref().is_some_and(|d| d.tool == g.tool && d.start == g.start))
}
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) || !active(app) {
        return None;
    }
    let prepare_clock = trace_clock(app);
    let g = app.shape_drag.gesture.as_ref()?;
    if !valid(app, g) {
        return None;
    }
    let d = delta(g.start, g.end)?;
    if d == [0.0, 0.0] {
        if let Some(g) = app.shape_drag.gesture.as_mut() {
            g.shown = None;
            g.shown_end = None;
        }
        return None;
    }
    if g.shown_end != Some(g.end) {
        // Document/surfaces are COW; translate/rasterize only the one shape, once per shown offset.
        let mut doc = (*g.original).clone();
        if let LayerContent::Shape(sh) = &mut doc.layer_mut(g.layer)?.content {
            photocraft_engine::vector_cmds::transform_shape(sh, &Affine::translate(d[0], d[1]));
            photocraft_engine::vector_cmds::refresh_shape(&g.original, sh);
        } else {
            return None;
        }
        let sequence = app.shape_drag.sequence.checked_add(1)?;
        app.shape_drag.sequence = sequence;
        let g = app.shape_drag.gesture.as_mut()?;
        g.shown_end = Some(g.end);
        g.shown = Some(Arc::new(doc));
        trace_event(app, "prepare", None, None, prepare_clock);
    }
    Some((app.shape_drag.gesture.as_ref()?.shown.clone()?, (1u64 << 47).checked_add(app.shape_drag.sequence)?))
}
pub(crate) fn layer_edges(app: &PhotocraftApp, idx: usize, id: LayerId) -> Option<photocraft_geom::Rect> {
    if app.session.active_index() != Some(idx) || !active(app) {
        return None;
    }
    let g = app.shape_drag.gesture.as_ref().filter(|g| valid(app, g) && g.layer == id)?;
    g.shown.as_ref()?.layer(id)?.surface().map(photocraft_compose::bounds::content_bounds)
}
/// The final constrained/snapped document position is shared with motion; no rounding.
pub(crate) fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> bool {
    if !std::mem::take(&mut app.shape_drag.owns_release) {
        return false;
    }
    let Some(g) = app.shape_drag.gesture.take() else {
        return true;
    };
    // app.drag has been taken by canvas release; validate immutable ownership, not pointer state.
    if !valid(app, &g) || g.start != start {
        return true;
    }
    let Some([dx, dy]) = delta(start, end) else {
        return true;
    };
    if dx == 0.0 && dy == 0.0 {
        return true;
    }
    if let Err(error) = app.run("shape.edit", json!({"layer":g.layer.0,"move":[dx,dy]})) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    true
}
#[cfg(test)]
#[path = "shape_drag_tests.rs"]
mod tests;

/// Offset of the geometry actually prepared for the canvas, for session-based handles.
pub(crate) fn shown_offset(app: &PhotocraftApp, layer: LayerId) -> [f64; 2] {
    app.shape_drag
        .gesture
        .as_ref()
        .filter(|g| valid(app, g) && g.layer == layer)
        .and_then(|g| g.shown_end.and_then(|end| delta(g.start, end)))
        .unwrap_or([0.0, 0.0])
}

struct Trace {
    remaining: u16,
    last_submit: Option<f64>,
}
pub(crate) fn trace_clock(app: &PhotocraftApp) -> Option<f64> {
    app.shape_drag.trace.as_ref().filter(|t| t.remaining > 0).map(|_| crate::gpu_canvas::now_ms())
}
pub(crate) fn trace_pointer(app: &mut PhotocraftApp, raw: crate::canvas::ToolEvent, filtered: crate::canvas::ToolEvent) {
    if trace_clock(app).is_none() {
        return;
    }
    let point = |event| match event {
        crate::canvas::ToolEvent::Down { x, y, .. } | crate::canvas::ToolEvent::Move { x, y, .. } | crate::canvas::ToolEvent::Up { x, y } => [x, y],
    };
    trace_event(app, "pointer", Some(point(raw)), Some(point(filtered)), None);
}
pub(crate) fn trace_submit(app: &mut PhotocraftApp, start: Option<f64>) {
    trace_event(app, "submit", None, None, start);
}
fn trace_event(app: &mut PhotocraftApp, phase: &str, raw: Option<[f64; 2]>, filtered: Option<[f64; 2]>, start: Option<f64>) {
    if app.shape_drag.trace.as_ref().is_none_or(|t| t.remaining == 0) {
        return;
    }
    let Some(g) = app.shape_drag.gesture.as_ref() else {
        return;
    };
    let displacement = delta(g.start, filtered.unwrap_or(g.end));
    let prepared = g.shown_end.and_then(|end| delta(g.start, end));
    let bounds = g
        .shown
        .as_ref()
        .and_then(|doc| doc.layer(g.layer))
        .and_then(|l| if let LayerContent::Shape(sh) = &l.content { sh.path.control_bounds() } else { None });
    let caches = app
        .canvases
        .iter()
        .filter(|((doc, _), _)| *doc == g.original.id)
        .map(|((_, out), c)| (*out, c.on_gpu, c.preview_key, c.tex_preview_key))
        .collect::<Vec<_>>();
    let now = crate::gpu_canvas::now_ms();
    let elapsed = start.map(|s| now - s);
    let Some(trace) = app.shape_drag.trace.as_mut() else {
        return;
    };
    let gap = if phase == "submit" {
        let old = trace.last_submit;
        trace.last_submit = Some(now);
        old.map(|s| now - s)
    } else {
        None
    };
    trace.remaining = trace.remaining.saturating_sub(1);
    log::info!(target:"photocraft.shape_drag","phase={phase} raw={raw:?} filtered={filtered:?} delta={displacement:?} prepared={prepared:?} bounds={bounds:?} elapsed_ms={elapsed:?} submit_gap_ms={gap:?} cache(output,gpu,key,texture_key)={caches:?}");
}

fn trace_route(app: &mut PhotocraftApp, route: &str) {
    let Some(trace) = app.shape_drag.trace.as_mut().filter(|t| t.remaining > 0) else {
        return;
    };
    trace.remaining = trace.remaining.saturating_sub(1);
    log::info!(target:"photocraft.shape_drag","route={route} tool={:?} selected_count={} mask_target={} vector_mask_target={} bounding_box_only={}",app.ui.tool,app.session.active().map_or(0,|st|st.selected_layers().len()),app.ui.mask_target,app.ui.vector_mask_target,app.session.prefs().interface.show_bounding_box_when_dragging_layer);
}

/// Only Shape previews change paint order. Legacy raster/multi-layer Move must keep its
/// image and Layer Edges on the same original pre-input frame.
pub(crate) fn defer_canvas(app: &PhotocraftApp, press: Option<[f64; 2]>, mods: egui::Modifiers) -> bool {
    if active(app) {
        return true;
    }
    if app.drag.is_some() || app.ui.transform.is_some() || app.layer_mask_targeted() || app.ui.vector_mask_target {
        return false;
    }
    let Some(st) = app.session.active() else {
        return false;
    };
    let Some(at) = press.filter(|p| p.iter().all(|v| v.is_finite() && v.abs() < f64::from(i32::MAX))) else {
        return false;
    };
    let id = match app.ui.tool {
        Tool::PathSelection => crate::vector_ui::pick_shape_at(&st.doc, at, 6.0 / f64::from(app.point_zoom().max(0.01))).or(st.active_layer),
        Tool::Move => {
            if mods.alt || app.session.prefs().interface.show_bounding_box_when_dragging_layer || photocraft_engine::float_cmds::floating(st).is_some() {
                return false;
            }
            let selected = st.selected_layers();
            let id = if app.ui.tool_options.move_auto_select != mods.command {
                let picked = photocraft_engine::pick_cmds::auto_select_target(
                    &st.doc,
                    at[0].floor() as i32,
                    at[1].floor() as i32,
                    app.ui.tool_options.move_target == "group",
                );
                let Some(picked) = picked else {
                    return false;
                };
                // Match layer.pickAt's multi-selection retention/add rules.
                if (selected.contains(&picked) && selected.len() > 1) || (mods.shift && !selected.is_empty() && !selected.contains(&picked)) {
                    return false;
                }
                Some(picked)
            } else {
                if selected.len() != 1 {
                    return false;
                }
                st.active_layer
            };
            if id.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some() || l.vector_mask.is_some()) {
                return false;
            }
            id
        }
        _ => None,
    };
    id.is_some_and(|id| movable(&st.doc, id))
}
