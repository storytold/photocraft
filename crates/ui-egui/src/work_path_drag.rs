//! Transient whole/component Work Path selection; document edits happen only on release.
use crate::{PhotocraftApp, state::Tool};
use photocraft_doc::{LayerContent, LayerId, Path};
use photocraft_geom::{Affine, Point};
use serde_json::json;

#[derive(Clone, Debug, PartialEq)]
struct Selection {
    document: u64,
    revision: u64,
    layer: Option<LayerId>,
    selected_path: Option<String>,
    path: Path,
    /// None is an explicit whole-path selection, never an invalid component fallback.
    component: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
struct Gesture {
    start: [f64; 2],
    end: [f64; 2],
    hit: usize,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    selection: Option<Selection>,
    gesture: Option<Gesture>,
    // Cancelled or stale gestures consume late release instead of delegating to another target.
    owns_release: bool,
    // An ordinary outline miss consumes release without reporting cancellation.
    missed: bool,
}

fn work_target(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    if app.ui.selected_path.as_deref().is_some_and(|name| name != "work") {
        return false;
    }
    if crate::vector_ui::shape_work_path_selected(app) {
        return true;
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
fn same_target(app: &PhotocraftApp, target: &Selection) -> bool {
    app.ui.selected_path == target.selected_path
        && work_target(app)
        && target.component.is_none_or(|i| target.path.subpaths.get(i).is_some())
        && app
            .session
            .active()
            .is_some_and(|st| st.doc.id.0 == target.document && st.active_layer == target.layer && st.doc.work_path.as_ref() == Some(&target.path))
}
fn valid(app: &PhotocraftApp, target: &Selection) -> bool {
    same_target(app, target) && app.session.active().is_some_and(|st| st.revision == target.revision)
}
fn snapshot(app: &PhotocraftApp, component: Option<usize>) -> Option<Selection> {
    let st = app.session.active()?;
    let path = st.doc.work_path.clone()?;
    if component.is_some_and(|i| path.subpaths.get(i).is_none()) {
        return None;
    }
    Some(Selection { document: st.doc.id.0, revision: st.revision, layer: st.active_layer, selected_path: app.ui.selected_path.clone(), path, component })
}
fn focus_work(app: &mut PhotocraftApp) {
    app.ui.selected_path = Some("work".into());
    app.ui.shape_work_path_target = app.session.active().map(|st| (st.doc.id.0, st.active_layer.map(|id| id.0)));
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    if let Some(st) = app.session.active_mut() {
        st.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
    }
    app.sync_mask_targets();
}
/// Paths-panel whole-path selection remains explicit and can move all components together.
pub fn select_whole(app: &mut PhotocraftApp) {
    cancel(app);
    if app.session.active().is_some_and(|st| st.doc.work_path.is_some()) {
        focus_work(app);
        app.ui.work_path_drag.selection = snapshot(app, None);
    } else {
        app.ui.work_path_drag.selection = None;
    }
}
pub fn clear_selection(app: &mut PhotocraftApp) {
    cancel(app);
    app.ui.work_path_drag.selection = None;
}
pub fn selected_component(app: &PhotocraftApp) -> Option<usize> {
    if app.ui.tool != Tool::PathSelection {
        return None;
    }
    app.ui.work_path_drag.selection.as_ref().filter(|s| valid(app, s))?.component
}
fn displacement(start: [f64; 2], end: [f64; 2]) -> Option<[f64; 2]> {
    let delta = [end[0] - start[0], end[1] - start[1]];
    if !start.iter().chain(end.iter()).all(|v| v.is_finite()) || !delta.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) {
        return None;
    }
    Some(if delta[0].abs() + delta[1].abs() < 0.5 { [0.0, 0.0] } else { delta })
}
/// Outline hit selects one component; an explicit whole selection stays whole during a drag.
pub fn press(app: &mut PhotocraftApp, at: [f64; 2]) {
    app.ui.work_path_drag.gesture = None;
    app.ui.work_path_drag.owns_release = false;
    app.ui.work_path_drag.missed = false;
    if !app.ui.view.shows(app.ui.view.show.target_path) || !work_target(app) || displacement(at, at).is_none() {
        app.ui.work_path_drag.selection = None;
        return;
    }
    app.ui.work_path_drag.owns_release = true;
    let hit = app
        .session
        .active()
        .and_then(|st| st.doc.work_path.as_ref())
        .and_then(|path| photocraft_vector::edit::hit(path, Point::new(at[0], at[1]), 6.0 / f64::from(app.point_zoom().max(0.01)), &[]));
    let index = match hit {
        Some(photocraft_vector::edit::Hit::Anchor([s, _]) | photocraft_vector::edit::Hit::Segment([s, _], _)) => s,
        _ => {
            app.ui.work_path_drag.selection = None;
            app.ui.work_path_drag.missed = true;
            return;
        }
    };
    let whole = app.ui.work_path_drag.selection.as_ref().is_some_and(|s| valid(app, s) && s.component.is_none());
    focus_work(app);
    // Capture focus_work's updated target token, and preserve only a VALID whole scope.
    app.ui.work_path_drag.selection = snapshot(app, (!whole).then_some(index));
    app.ui.work_path_drag.gesture = Some(Gesture { start: at, end: at, hit: index });
}
pub fn motion(app: &mut PhotocraftApp, at: [f64; 2]) {
    let Some(d) = app.ui.work_path_drag.gesture.as_ref() else { return };
    if app.ui.tool != Tool::PathSelection || !app.ui.work_path_drag.selection.as_ref().is_some_and(|s| valid(app, s)) || displacement(d.start, at).is_none() {
        clear_selection(app);
        return;
    }
    if let Some(d) = &mut app.ui.work_path_drag.gesture {
        d.end = at;
    }
}
pub fn after_command(app: &mut PhotocraftApp, id: &str) {
    if matches!(id, "view.colorSamplers.add" | "view.colorSamplers.move" | "view.colorSamplers.delete" | "view.colorSamplers.clear")
        && app
            .ui
            .work_path_drag
            .selection
            .as_ref()
            .is_some_and(|s| same_target(app, s) && app.session.active().is_some_and(|st| s.revision.checked_add(1) == Some(st.revision)))
        && let (Some(selection), Some(st)) = (app.ui.work_path_drag.selection.as_mut(), app.session.active())
    {
        selection.revision = st.revision;
    }
    if app.ui.work_path_drag.selection.as_ref().is_some_and(|s| {
        !valid(app, s)
            || id == "layer.select"
            || id.starts_with("channel.target")
            || (app.ui.work_path_drag.gesture.is_some() && app.ui.tool != Tool::PathSelection)
    }) {
        clear_selection(app);
    }
}
pub fn preview(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let d = app.ui.work_path_drag.gesture.as_ref()?;
    let drag = app.drag.as_ref()?;
    if app.ui.tool != Tool::PathSelection
        || !app.ui.work_path_drag.selection.as_ref().is_some_and(|s| valid(app, s))
        || drag.tool != Tool::PathSelection
        || drag.start != d.start
    {
        return None;
    }
    displacement(d.start, d.end)
}
pub fn owns_drag(app: &PhotocraftApp) -> bool {
    app.ui.work_path_drag.owns_release
}
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.work_path_drag.gesture = None;
    app.ui.work_path_drag.missed = false;
    if app.drag.as_ref().is_some_and(|d| d.tool == Tool::PathSelection) {
        app.drag = None;
    }
}
pub fn reconcile(app: &mut PhotocraftApp, focused: bool) {
    if !focused {
        cancel(app);
        return;
    }
    if app.ui.work_path_drag.selection.as_ref().is_some_and(|s| !valid(app, s)) {
        clear_selection(app);
        return;
    }
    let Some(d) = app.ui.work_path_drag.gesture.as_ref() else { return };
    let lost = app.drag.as_ref().is_none_or(|drag| drag.tool != Tool::PathSelection || drag.start != d.start);
    if !focused || lost || app.ui.tool != Tool::PathSelection {
        cancel(app);
    }
}
pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> Option<Result<(), String>> {
    if !std::mem::take(&mut app.ui.work_path_drag.owns_release) {
        return None;
    }
    let missed = std::mem::take(&mut app.ui.work_path_drag.missed);
    let Some(d) = app.ui.work_path_drag.gesture.take() else {
        return Some(if missed { Ok(()) } else { Err("Work Path drag was cancelled.".into()) });
    };
    if app.ui.tool != Tool::PathSelection || d.start != start || !app.ui.work_path_drag.selection.as_ref().is_some_and(|s| valid(app, s)) {
        clear_selection(app);
        return Some(Err("Work Path selection is stale; no geometry was moved.".into()));
    }
    let Some([dx, dy]) = displacement(start, end) else { return Some(Err("Invalid Work Path displacement.".into())) };
    if dx == 0.0 && dy == 0.0 {
        if let Some(s) = &mut app.ui.work_path_drag.selection {
            s.component = Some(d.hit);
        }
        return Some(Ok(()));
    }
    let Some(selection) = app.ui.work_path_drag.selection.take() else { return Some(Err("Select a Work Path again.".into())) };
    let component = selection.component;
    let result = if let Some(index) = component {
        let anchors = photocraft_vector::edit::subpath_anchors(&selection.path, index);
        app.run("path.moveAnchors", json!({"name":"work","anchors":anchors,"move":[dx,dy]}))
    } else {
        let moved = selection.path.transform(&Affine::translate(dx, dy));
        app.run("path.set", json!({"name":"work","path":photocraft_engine::vector_cmds::path_json(&moved)}))
    };
    if result.is_ok() {
        app.ui.work_path_drag.selection = snapshot(app, component);
    }
    Some(result.map(|_| ()))
}
#[cfg(test)]
#[path = "work_path_component_tests.rs"]
mod component_tests;
#[cfg(test)]
#[path = "work_path_drag_tests.rs"]
mod tests;
