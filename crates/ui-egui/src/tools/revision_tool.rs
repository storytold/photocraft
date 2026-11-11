//! Revision Marker tool: drags the revision markers of the active document around, ⌥-drag copies
//! one, and the options bar sets the selected marker's size. PhotoCraft's own tool — Photoshop has
//! nothing to move a marker with, because it has no marker annotations.
//!
//! A drag is one history step: the frame is drawn at the pointer while it moves
//! (`canvas::revision_overlay`) and `revision.set` runs once on release, like the Move tool's layer
//! drag. `revision.set` re-takes the marker's snapshot when its frame moves, so a moved marker is
//! red again: the box frames different content now.

use egui::RichText;
use serde_json::json;

use crate::canvas::revision_overlay::{DONE, TODO};
use crate::panels::revision_panel::{layer_under_frame, next_marker_name};
use crate::state::Tool;
use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};

/// The largest frame side the options bar accepts, matching the marquee size fields.
const SIZE_MAX_PX: f32 = 300_000.0;

/// Where a press on a marker started: the pointer's document position and whether ⌥ was held
/// (⌥-drag copies the marker instead of moving it).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    index: usize,
    from: [f64; 2],
    alt: bool,
}

/// The tool's live state between a press and its release, plus the marker the options bar edits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct State {
    press: Option<Press>,
    /// The marker under the pointer, highlighted on the canvas.
    pub(crate) hover: Option<usize>,
    /// The marker last picked, the one the options bar's size fields and buttons act on.
    pub(crate) selected: Option<usize>,
    /// A drag in progress: the marker, and the frame it would have at the pointer's current place.
    pub(crate) preview: Option<(usize, [f64; 2])>,
}

/// The marker whose frame contains `p`, the topmost one (the last marker added is drawn last, so it
/// wins where two frames overlap).
fn marker_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<usize> {
    let doc = &app.session.active()?.doc;
    doc.revision_markers
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| p[0] >= m.pos[0] && p[1] >= m.pos[1] && p[0] <= m.pos[0] + m.size[0] && p[1] <= m.pos[1] + m.size[1])
        .map(|(i, _)| i)
}

/// Press: pick the marker under the pointer (or clear the pick on empty canvas) and remember where
/// the drag started. Nothing moves until the pointer actually travels.
pub(crate) fn press(app: &mut PhotocraftApp, p: [f64; 2], alt: bool) {
    let hit = marker_at(app, p);
    let state = &mut app.revision_tool;
    state.press = hit.map(|index| Press { index, from: p, alt });
    state.selected = hit;
    state.hover = hit;
    state.preview = None;
}

/// Pointer moved: follow it with a ghost frame of the marker being dragged.
pub(crate) fn move_to(app: &mut PhotocraftApp, p: [f64; 2]) {
    app.revision_tool.hover = marker_at(app, p);
    let Some(press) = app.revision_tool.press else { return };
    let (dx, dy) = (p[0] - press.from[0], p[1] - press.from[1]);
    // A click's jitter is not a drag: the frame moves once the pointer is more than a pixel away.
    if dx.abs() < 1.0 && dy.abs() < 1.0 {
        return;
    }
    let Some(orig) = app.session.active().and_then(|st| st.doc.revision_markers.get(press.index).map(|m| m.pos)) else {
        return;
    };
    let pos = [(orig[0] + dx).round(), (orig[1] + dy).round()];
    app.revision_tool.preview = Some((press.index, pos));
}

/// Release: move the marker (`revision.set`, one history step) or copy it (⌥-drag, `revision.add`
/// with the next free number). A press and release with no travel between them only leaves the
/// marker selected.
pub(crate) fn release(app: &mut PhotocraftApp, _p: [f64; 2]) {
    let press = app.revision_tool.press.take();
    let preview = app.revision_tool.preview.take();
    let (Some(press), Some((index, pos))) = (press, preview) else { return };
    let Some((size, note)) = app
        .session
        .active()
        .and_then(|st| st.doc.revision_markers.get(index).map(|m| (m.size, m.note.clone())))
    else {
        return;
    };
    if press.alt {
        // A copy frames the same amount of content at its new place and binds the layer under it.
        // Its number comes from the same counter Add from Selection uses, so two markers never
        // share one.
        let name = next_marker_name(app);
        let mut params = json!({"x": pos[0], "y": pos[1], "width": size[0], "height": size[1], "name": name, "note": note});
        if let Some(id) = layer_under_frame(app, pos, size) {
            params["layerId"] = json!(id);
        }
        if app.run("revision.add", params).is_ok() {
            app.revision_tool.selected = app.session.active().map(|st| st.doc.revision_markers.len().saturating_sub(1));
        }
    } else if app.run("revision.set", json!({"index": index, "x": pos[0], "y": pos[1]})).is_ok() {
        app.revision_tool.selected = Some(index);
    }
}

/// Options bar of the Revision Marker tool: the picked marker's size (the "set its size in the
/// properties" part of the request), Copy, Bind/Unbind and the red/green state. Returns true when
/// drawn, so the options bar stops there.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.tool != Tool::RevisionMarker {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else {
        return true;
    };
    let Some(i) = app.revision_tool.selected.filter(|i| *i < doc.revision_markers.len()) else {
        ui.label(
            RichText::new(tl!("Click a marker to move it; ⌥-drag copies it. Its size appears here once one is picked."))
                .size(11.0)
                .color(t.text_dim),
        );
        return true;
    };
    let (name, pos, size, done, bound, note) = {
        let m = &doc.revision_markers[i];
        (m.name.clone(), m.pos, m.size, m.done, m.layer_id.is_some(), m.note.clone())
    };
    ui.label(RichText::new(name).size(11.5).color(if done { DONE } else { TODO }));
    widgets::vline(ui, 22.0);
    ui.label(RichText::new(tl!("W")).size(11.0).color(t.text_dim));
    let mut w = size[0] as f32;
    let wfield = widgets::value_field(ui, &mut w, 1.0..=SIZE_MAX_PX, "", 62.0);
    ui.label(RichText::new(tl!("H")).size(11.0).color(t.text_dim));
    let mut h = size[1] as f32;
    let hfield = widgets::value_field(ui, &mut h, 1.0..=SIZE_MAX_PX, "", 62.0);
    if (wfield.changed() || hfield.changed()) && w > 0.0 && h > 0.0 {
        let _ = app.run("revision.set", json!({"index": i, "width": f64::from(w), "height": f64::from(h)}));
    }
    widgets::vline(ui, 22.0);
    if widgets::secondary_button(ui, tl!("Copy"), 0.0).clicked() {
        // A copy lands a little down and to the right of the original, where dragging one there
        // would put it.
        let copy_pos = [pos[0] + 16.0, pos[1] + 16.0];
        let name = next_marker_name(app);
        let mut params = json!({"x": copy_pos[0], "y": copy_pos[1], "width": size[0], "height": size[1], "name": name});
        if !note.is_empty() {
            params["note"] = json!(note);
        }
        if let Some(id) = layer_under_frame(app, copy_pos, size) {
            params["layerId"] = json!(id);
        }
        if app.run("revision.add", params).is_ok() {
            app.revision_tool.selected = app.session.active().map(|st| st.doc.revision_markers.len().saturating_sub(1));
        }
    }
    let label = if done { tl!("Mark as Red") } else { tl!("Mark as Green") };
    if widgets::secondary_button(ui, label, 0.0).clicked() {
        let _ = app.run("revision.set", json!({"index": i, "done": !done}));
    }
    if bound {
        if widgets::secondary_button(ui, tl!("Unbind"), 0.0).clicked() {
            let _ = app.run("revision.set", json!({"index": i, "layerId": null}));
        }
    } else if widgets::secondary_button(ui, tl!("Bind Layer"), 0.0).clicked()
        && let Some(id) = layer_under_frame(app, pos, size)
    {
        let _ = app.run("revision.set", json!({"index": i, "layerId": id}));
    }
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A 400 × 300 document with one marker framing (100, 80) – (180, 120), the tool picked.
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("revision.add", json!({"x": 100.0, "y": 80.0, "width": 80.0, "height": 40.0, "name": "Marker 1"})).unwrap();
        app.ui.tool = Tool::RevisionMarker;
        app
    }

    /// The markers' positions and names, in document order.
    fn markers(app: &PhotocraftApp) -> Vec<([f64; 2], String)> {
        app.session.active().map(|st| st.doc.revision_markers.iter().map(|m| (m.pos, m.name.clone())).collect()).unwrap_or_default()
    }

    /// Dragging a marker moves it by the distance the pointer travelled, in one history step that
    /// re-snapshots the frame it now covers.
    #[test]
    fn dragging_a_marker_moves_it_one_step() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], false);
        assert_eq!(app.revision_tool.selected, Some(0), "the press picks the marker under the pointer");
        move_to(&mut app, [180.0, 120.0]);
        assert_eq!(app.revision_tool.preview, Some((0, [140.0, 100.0])), "the ghost frame follows the pointer");
        release(&mut app, [180.0, 120.0]);
        assert_eq!(markers(&app), [([140.0, 100.0], "Marker 1".to_string())], "moved by the pointer's travel");
        assert_eq!(app.revision_tool.preview, None, "the ghost frame is gone");
        let steps = app.session.active().unwrap().history.entries();
        assert_eq!(steps.last().map(String::as_str), Some("Resize Revision Marker"), "one step for the move");
    }

    /// ⌥-drag copies the marker where it lands, under the next free number, and leaves the original
    /// where it was.
    #[test]
    fn alt_dragging_a_marker_copies_it() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], true);
        move_to(&mut app, [240.0, 200.0]);
        release(&mut app, [240.0, 200.0]);
        let markers = markers(&app);
        assert_eq!(markers.len(), 2, "a copy is added");
        assert_eq!(markers[0], ([100.0, 80.0], "Marker 1".to_string()), "the original stays");
        assert_eq!(markers[1], ([200.0, 180.0], "Marker 2".to_string()), "the copy lands where the drag put it");
        assert_eq!(app.revision_tool.selected, Some(1), "the copy is picked, ready to be sized");
    }

    /// A press and release with no travel only picks the marker: nothing moves, nothing is added.
    #[test]
    fn a_click_picks_a_marker_without_moving_it() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], false);
        move_to(&mut app, [140.0, 100.0]);
        release(&mut app, [140.0, 100.0]);
        assert_eq!(markers(&app), [([100.0, 80.0], "Marker 1".to_string())], "unchanged");
        assert_eq!(app.revision_tool.selected, Some(0), "but picked");
        // A press on empty canvas clears the pick, so the options bar stops editing a marker.
        press(&mut app, [20.0, 20.0], false);
        assert_eq!(app.revision_tool.selected, None);
    }
}
