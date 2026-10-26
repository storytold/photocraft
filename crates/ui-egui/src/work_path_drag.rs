//! Whole-Work-Path translation is transient until its single release command.
use photocraft_doc::{LayerContent, LayerId, Path};
use photocraft_geom::Affine;
use serde_json::json;

use crate::{PhotocraftApp, state::Tool};

#[derive(Clone, Debug, PartialEq)]
struct Gesture {
    document: u64,
    revision: u64,
    layer: Option<LayerId>,
    selected_path: Option<String>,
    path: Path,
    start: [f64; 2],
    end: [f64; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    gesture: Option<Gesture>,
    // A cancelled/invalidated gesture must consume its late release, never retarget it.
    owns_release: bool,
}

fn work_target(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    // Preserve upstream layer/mask priority. Named saved paths retain their legacy route;
    // adding saved-path editing is outside this Work-Path-only change.
    if app.ui.selected_path.as_deref().is_some_and(|name| name != "work") {
        return false;
    }
    if let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) {
        if matches!(layer.content, LayerContent::Shape(_)) {
            return false;
        }
        let mask = app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
        if mask && layer.vector_mask.is_some() {
            return false;
        }
    }
    st.doc.work_path.is_some()
}

fn valid(app: &PhotocraftApp, gesture: &Gesture) -> bool {
    app.ui.tool == Tool::PathSelection
        && app.ui.selected_path == gesture.selected_path
        && work_target(app)
        && app.session.active().is_some_and(|st| {
            st.doc.id.0 == gesture.document
                && st.revision == gesture.revision
                && st.active_layer == gesture.layer
                && st.doc.work_path.as_ref() == Some(&gesture.path)
        })
}

/// Same finite fractional offset for preview and commit, retaining the old click dead zone.
fn displacement(start: [f64; 2], end: [f64; 2]) -> Option<[f64; 2]> {
    let delta = [end[0] - start[0], end[1] - start[1]];
    if !start.iter().chain(end.iter()).all(|v| v.is_finite()) || !delta.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) {
        return None;
    }
    Some(if delta[0].abs() + delta[1].abs() < 0.5 { [0.0, 0.0] } else { delta })
}

/// Called after upstream's existing shape picking, without changing the selected target.
pub fn press(app: &mut PhotocraftApp, at: [f64; 2]) {
    app.ui.work_path_drag = State::default();
    if !app.ui.view.shows(app.ui.view.show.target_path) || !work_target(app) || displacement(at, at).is_none() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(path) = st.doc.work_path.clone() else { return };
    app.ui.work_path_drag = State {
        gesture: Some(Gesture {
            document: st.doc.id.0,
            revision: st.revision,
            layer: st.active_layer,
            selected_path: app.ui.selected_path.clone(),
            path,
            start: at,
            end: at,
        }),
        owns_release: true,
    };
}

/// Canvas supplies document coordinates after its existing constraints, before sample gating.
pub fn motion(app: &mut PhotocraftApp, at: [f64; 2]) {
    let Some(gesture) = app.ui.work_path_drag.gesture.as_ref() else { return };
    if !valid(app, gesture) || displacement(gesture.start, at).is_none() {
        cancel(app);
        return;
    }
    if let Some(gesture) = &mut app.ui.work_path_drag.gesture {
        gesture.end = at;
    }
}

/// Invalidate immediately after a command, even if a later command restores the target.
pub fn after_command(app: &mut PhotocraftApp, id: &str) {
    if let Some(gesture) = &app.ui.work_path_drag.gesture
        && (!valid(app, gesture) || id == "layer.select" || id.starts_with("channel.target"))
    {
        cancel(app);
    }
}

/// Renderer consumes an offset, not a cloned or serialized path.
pub fn preview(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let gesture = app.ui.work_path_drag.gesture.as_ref()?;
    let drag = app.drag.as_ref()?;
    if !valid(app, gesture) || drag.tool != Tool::PathSelection || drag.start != gesture.start {
        return None;
    }
    displacement(gesture.start, gesture.end)
}

pub fn owns_drag(app: &PhotocraftApp) -> bool {
    app.ui.work_path_drag.owns_release
}

pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.work_path_drag.gesture = None;
    if app.drag.as_ref().is_some_and(|d| d.tool == Tool::PathSelection) {
        app.drag = None;
    }
}

pub fn reconcile(app: &mut PhotocraftApp, focused: bool) {
    let Some(gesture) = app.ui.work_path_drag.gesture.as_ref() else { return };
    let lost = app.drag.as_ref().is_none_or(|d| d.tool != Tool::PathSelection || d.start != gesture.start);
    if !focused || lost || !valid(app, gesture) {
        cancel(app);
    }
}

/// None delegates non-Work-Path gestures to upstream. Owned late releases never delegate.
pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> Option<Result<(), String>> {
    if !std::mem::take(&mut app.ui.work_path_drag.owns_release) {
        return None;
    }
    let Some(gesture) = app.ui.work_path_drag.gesture.take() else { return Some(Err("Work Path drag was cancelled.".into())) };
    if start != gesture.start || !valid(app, &gesture) {
        return Some(Err("Work Path drag target changed; no path was moved.".into()));
    }
    let Some([dx, dy]) = displacement(start, end) else { return Some(Err("Invalid Work Path displacement.".into())) };
    if dx == 0.0 && dy == 0.0 {
        return Some(Ok(()));
    }
    let moved = gesture.path.transform(&Affine::translate(dx, dy));
    Some(app.run("path.set", json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&moved)})).map(|_| ()))
}

#[cfg(test)]
#[path = "work_path_drag_tests.rs"]
mod tests;
