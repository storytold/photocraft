//! Revision Marker tool: drags the revision markers of the active document around, ⌥-drag copies
//! one, ⇧ holds the drag to one axis, and the options bar sets the selected marker's size.
//! PhotoCraft's own tool — Photoshop has nothing to move a marker with, because it has no marker
//! annotations.
//!
//! A drag is one history step: the frames are drawn at the pointer while they move
//! (`canvas::revision_overlay`) and `revision.translate` runs once on release, like the Move tool's
//! layer drag. It moves the whole picked set — or the whole group the pressed frame belongs to —
//! and re-takes the snapshot of every frame it moved, so a moved marker is red again: the box
//! frames different content now. While it moves, the frames snap to the other markers' edges and
//! centres, and the guides they snapped to are drawn, like the Move tool's smart guides.

use egui::RichText;
use photocraft_doc::Document;
use serde_json::json;

use crate::canvas::revision_overlay::{DONE, TODO};
use crate::panels::revision_panel::{layer_under_frame, next_marker_name};
use crate::state::Tool;
use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};

/// The largest frame side the options bar accepts, matching the marquee size fields.
const SIZE_MAX_PX: f32 = 300_000.0;

/// How near a moving edge or centre must come to another marker's, in screen pixels, before the
/// drag snaps to it. Screen-based, so the pull stays the same however far the view is zoomed in.
const SNAP_SCREEN_PX: f64 = 6.0;

/// Where a press on a marker started: the pointer's document position and whether ⌥ was held
/// (⌥-drag copies the marker instead of moving it).
#[derive(Clone, Debug, PartialEq)]
struct Press {
    index: usize,
    from: [f64; 2],
    alt: bool,
    /// Every marker this drag moves: the picked set when the press landed on one of several picked
    /// frames, the whole group when it landed on a grouped frame, otherwise just the pressed one.
    targets: Vec<usize>,
}

/// A smart guide the drag has snapped to: one line running across the moving frames and the
/// marker (or canvas edge) they lined up with, drawn while the drag lasts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Guide {
    /// `true`: the line sits at one x and runs down the canvas (edges lined up in a column).
    pub(crate) vertical: bool,
    /// The document coordinate the line sits on (x when vertical, y otherwise).
    pub(crate) at: f64,
    /// The document coordinates the line spans.
    pub(crate) from: f64,
    pub(crate) to: f64,
}

/// A drag in progress: every frame it moves with the position each had when the press happened, the
/// offset it would land with, and the guides it has snapped to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Preview {
    pub(crate) targets: Vec<(usize, [f64; 2], [f64; 2])>,
    pub(crate) dx: f64,
    pub(crate) dy: f64,
    pub(crate) guides: Vec<Guide>,
}

/// The tool's live state between a press and its release, plus the markers the options bar edits.
/// The picked set is a `Vec`, so the state is cloned rather than copied.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct State {
    press: Option<Press>,
    /// The marker under the pointer, highlighted on the canvas.
    pub(crate) hover: Option<usize>,
    /// The markers picked, the ones the options bar's size fields and buttons act on. Shift-click
    /// adds one frame to the set or takes it out again, so several can be aligned together.
    pub(crate) selected: Vec<usize>,
    /// A drag in progress: the frames it moves, where they would land, and the guides they snapped to.
    pub(crate) preview: Option<Preview>,
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

/// The markers a drag of the frame at `hit` moves: the picked set when more than one frame is
/// picked and the press landed on one of them, the whole group the frame belongs to (so a group
/// behaves like a layer group), otherwise the one frame.
fn drag_targets(app: &PhotocraftApp, hit: usize) -> Vec<usize> {
    if app.revision_tool.selected.contains(&hit) && app.revision_tool.selected.len() > 1 {
        return app.revision_tool.selected.clone();
    }
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return vec![hit] };
    let Some(group) = doc.revision_markers.get(hit).and_then(|m| m.group) else { return vec![hit] };
    let members: Vec<usize> =
        doc.revision_markers.iter().enumerate().filter(|(_, m)| m.group == Some(group)).map(|(i, _)| i).collect();
    if members.len() > 1 { members } else { vec![hit] }
}

/// Snap the moving frames' left/centre/right (and top/middle/bottom) to the other markers' and the
/// canvas's, within [`SNAP_SCREEN_PX`] at the current zoom, and return the offset to use plus the
/// lines to draw for what lined up. A locked axis (⇧) is left exactly as the pointer put it.
fn snap_frames(
    doc: &Document,
    moving: &[(usize, [f64; 2], [f64; 2])],
    dx: f64,
    dy: f64,
    zoom: f64,
    lock_x: bool,
    lock_y: bool,
) -> (f64, f64, Vec<Guide>) {
    let tol = SNAP_SCREEN_PX / zoom.max(0.01);
    let (mut left, mut top, mut right, mut bottom) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (_, pos, size) in moving {
        left = left.min(pos[0] + dx);
        top = top.min(pos[1] + dy);
        right = right.max(pos[0] + dx + size[0]);
        bottom = bottom.max(pos[1] + dy + size[1]);
    }
    // Every line the frames can land on: another marker's left edge, centre or right edge, and the
    // canvas's two sides and middle. Each carries the span the guide should cover.
    let mut xs: Vec<(f64, f64, f64)> = Vec::new();
    let mut ys: Vec<(f64, f64, f64)> = Vec::new();
    for (i, m) in doc.revision_markers.iter().enumerate() {
        if moving.iter().any(|(j, _, _)| *j == i) {
            continue;
        }
        for x in [m.pos[0], m.pos[0] + m.size[0] / 2.0, m.pos[0] + m.size[0]] {
            xs.push((x, m.pos[1].min(top), (m.pos[1] + m.size[1]).max(bottom)));
        }
        for y in [m.pos[1], m.pos[1] + m.size[1] / 2.0, m.pos[1] + m.size[1]] {
            ys.push((y, m.pos[0].min(left), (m.pos[0] + m.size[0]).max(right)));
        }
    }
    let canvas = doc.bounds();
    let (cx0, cx1) = (f64::from(canvas.x0), f64::from(canvas.x1));
    let (cy0, cy1) = (f64::from(canvas.y0), f64::from(canvas.y1));
    for x in [cx0, (cx0 + cx1) / 2.0, cx1] {
        xs.push((x, top, bottom));
    }
    for y in [cy0, (cy0 + cy1) / 2.0, cy1] {
        ys.push((y, left, right));
    }
    // The nearest line wins on each axis; ties keep the first (edges before centres, near before far).
    let mut best_x: Option<(f64, f64, f64, f64)> = None; // (distance, at, from, to)
    if !lock_x {
        for anchor in [left, (left + right) / 2.0, right] {
            for (at, from, to) in &xs {
                let d = (at - anchor).abs();
                if d <= tol && best_x.is_none_or(|(bd, _, _, _)| d < bd) {
                    best_x = Some((d, *at, *from, *to));
                }
            }
        }
    }
    let mut best_y: Option<(f64, f64, f64, f64)> = None;
    if !lock_y {
        for anchor in [top, (top + bottom) / 2.0, bottom] {
            for (at, from, to) in &ys {
                let d = (at - anchor).abs();
                if d <= tol && best_y.is_none_or(|(bd, _, _, _)| d < bd) {
                    best_y = Some((d, *at, *from, *to));
                }
            }
        }
    }
    let mut guides = Vec::new();
    let (mut ndx, mut ndy) = (dx, dy);
    if let Some((_, at, from, to)) = best_x {
        // Align the moving edge that matched, not the middle of the moving box.
        let anchor = [left, (left + right) / 2.0, right]
            .into_iter()
            .min_by(|a, b| (at - *a).abs().total_cmp(&(at - *b).abs()))
            .unwrap_or(left);
        ndx += at - anchor;
        guides.push(Guide { vertical: true, at, from, to });
    }
    if let Some((_, at, from, to)) = best_y {
        let anchor = [top, (top + bottom) / 2.0, bottom]
            .into_iter()
            .min_by(|a, b| (at - *a).abs().total_cmp(&(at - *b).abs()))
            .unwrap_or(top);
        ndy += at - anchor;
        guides.push(Guide { vertical: false, at, from, to });
    }
    (ndx, ndy, guides)
}

/// Press: pick the marker under the pointer (or clear the pick on empty canvas) and remember where
/// the drag started. Nothing moves until the pointer actually travels.
pub(crate) fn press(app: &mut PhotocraftApp, p: [f64; 2], alt: bool, shift: bool) {
    let hit = marker_at(app, p);
    let targets = hit.map(|index| drag_targets(app, index)).unwrap_or_default();
    let state = &mut app.revision_tool;
    state.press = hit.map(|index| Press { index, from: p, alt, targets });
    match hit {
        // Shift-click adds a frame to the picked set, or takes it out again. A plain click on one of
        // several picked frames keeps the set, so they can still be aligned; a click elsewhere picks
        // just the frame under the pointer, or nothing at all on empty canvas.
        Some(index) if shift => {
            if state.selected.contains(&index) {
                state.selected.retain(|i| *i != index);
            } else {
                state.selected.push(index);
                state.selected.sort_unstable();
            }
        }
        Some(index) if !state.selected.contains(&index) => state.selected = vec![index],
        Some(_) => {}
        None if !shift => state.selected.clear(),
        None => {}
    }
    state.hover = hit;
    state.preview = None;
}

/// Pointer moved: follow it with ghost frames of every marker the drag moves. ⇧ keeps the drag on
/// one axis — the one the pointer has travelled further along — so frames slide straight along the
/// row they frame instead of drifting off it. The frames snap to the neighbours' edges and centres
/// (smart guides), and the lines they lined up with are drawn until the drag ends.
pub(crate) fn move_to(app: &mut PhotocraftApp, p: [f64; 2], shift: bool, zoom: f32) {
    app.revision_tool.hover = marker_at(app, p);
    let Some(press) = app.revision_tool.press.clone() else { return };
    let (mut dx, mut dy) = (p[0] - press.from[0], p[1] - press.from[1]);
    // A click's jitter is not a drag: the frames move once the pointer is more than a pixel away.
    if dx.abs() < 1.0 && dy.abs() < 1.0 {
        return;
    }
    let mut lock_x = false;
    let mut lock_y = false;
    if shift {
        if dx.abs() >= dy.abs() {
            dy = 0.0;
            lock_y = true;
        } else {
            dx = 0.0;
            lock_x = true;
        }
    }
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return };
    let moving: Vec<(usize, [f64; 2], [f64; 2])> = press
        .targets
        .iter()
        .filter_map(|i| doc.revision_markers.get(*i).map(|m| (*i, m.pos, m.size)))
        .collect();
    if moving.is_empty() {
        return;
    }
    let (sdx, sdy, guides) = snap_frames(doc, &moving, dx, dy, f64::from(zoom), lock_x, lock_y);
    app.revision_tool.preview =
        Some(Preview { targets: moving, dx: sdx.round(), dy: sdy.round(), guides });
}

/// Release: move the marker (`revision.set`, one history step) or copy it (⌥-drag, `revision.add`
/// with the next free number). A press and release with no travel between them only leaves the
/// marker selected.
pub(crate) fn release(app: &mut PhotocraftApp, _p: [f64; 2]) {
    let press = app.revision_tool.press.take();
    let preview = app.revision_tool.preview.take();
    let (Some(press), Some(preview)) = (press, preview) else { return };
    if preview.targets.is_empty() {
        return;
    }
    let index = press.index;
    if press.alt {
        // ⌥-drag copies the frame under the pointer at its new place and binds the layer under it;
        // the rest of a picked set stays where it is. Its number comes from the same counter Add
        // from Selection uses, so two markers never share one.
        let Some((_, pos, size)) = preview.targets.iter().find(|(i, _, _)| *i == index).copied() else { return };
        let at = [pos[0] + preview.dx, pos[1] + preview.dy];
        let note = app
            .session
            .active()
            .and_then(|st| st.doc.revision_markers.get(index).map(|m| m.note.clone()))
            .unwrap_or_default();
        let name = next_marker_name(app);
        let mut params = json!({"x": at[0], "y": at[1], "width": size[0], "height": size[1], "name": name, "note": note});
        if let Some(id) = layer_under_frame(app, at, size) {
            params["layerId"] = json!(id);
        }
        if app.run("revision.add", params).is_ok() {
            app.revision_tool.selected =
                vec![app.session.active().map_or(0, |st| st.doc.revision_markers.len().saturating_sub(1))];
        }
        return;
    }
    let indices: Vec<usize> = preview.targets.iter().map(|(i, _, _)| *i).collect();
    if app.run("revision.translate", json!({"indices": indices, "dx": preview.dx, "dy": preview.dy})).is_ok() {
        // The moved frames stay picked, whole set included: frames picked to be aligned stay
        // picked after one of them is nudged.
        if !app.revision_tool.selected.contains(&index) {
            app.revision_tool.selected = vec![index];
        }
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
    if app.session.active().is_none() {
        return true;
    }
    // How solid the markers draw over the artwork (Preferences ▸ Workspace ▸ Revision marker
    // opacity, stored there so it holds across documents). It sits first on the bar, so it can be
    // set before any marker is placed.
    let mut opacity = app.session.prefs().workspace.revision_opacity() * 100.0;
    ui.label(RichText::new(tl!("Opacity")).size(11.0).color(t.text_dim));
    if widgets::value_field_named(ui, tl!("Opacity"), &mut opacity, 5.0..=100.0, "%", 62.0).changed() {
        let v = (opacity / 100.0).clamp(0.05, 1.0);
        app.session.prefs.edit(|p| p.workspace.revision_marker_opacity = v);
    }
    widgets::vline(ui, 22.0);
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else {
        return true;
    };
    let picked: Vec<usize> =
        app.revision_tool.selected.iter().copied().filter(|i| *i < doc.revision_markers.len()).collect();
    if picked.is_empty() {
        ui.label(
            RichText::new(tl!("Click a marker to move it; Alt-drag copies it; Shift-click picks several to align. Its size appears here once one is picked."))
                .size(11.0)
                .color(t.text_dim),
        );
        return true;
    }
    if picked.len() > 1 {
        // Several frames picked in one go: line their edges or their centres up, in one undo step.
        ui.label(RichText::new(format!("{} x {}", picked.len(), tl!("Markers"))).size(11.5).color(t.text));
        widgets::vline(ui, 22.0);
        for (label, mode) in [
            (tl!("Left"), "left"),
            (tl!("Right"), "right"),
            (tl!("Top"), "top"),
            (tl!("Bottom"), "bottom"),
            (tl!("Center H"), "centerX"),
            (tl!("Center V"), "centerY"),
        ] {
            if widgets::secondary_button(ui, label, 0.0).clicked() {
                let _ = app.run("revision.align", json!({"indices": picked, "mode": mode}));
            }
        }
        widgets::vline(ui, 22.0);
        if widgets::secondary_button(ui, tl!("Group"), 0.0)
            .on_hover_text(tl!("Make these frames one group: they are picked and moved together"))
            .clicked()
        {
            let _ = app.run("revision.group", json!({ "indices": picked }));
        }
        // Distribute evens the gaps along one axis; it takes a third frame, so the two buttons only
        // appear once the user has picked three or more.
        if picked.len() > 2 {
            widgets::vline(ui, 22.0);
            for (label, mode) in [(tl!("Distribute Horizontally"), "distributeX"), (tl!("Distribute Vertically"), "distributeY")] {
                if widgets::secondary_button(ui, label, 0.0).clicked() {
                    let _ = app.run("revision.align", json!({"indices": picked, "mode": mode}));
                }
            }
        }
        return true;
    }
    let i = picked[0];
    let (name, pos, size, done, bound, note, group) = {
        let m = &doc.revision_markers[i];
        (m.name.clone(), m.pos, m.size, m.done, m.layer_id.is_some(), m.note.clone(), m.group)
    };
    ui.label(RichText::new(name).size(11.5).color(if done { DONE } else { TODO }));
    // The group this frame moves with, and the way out of it.
    if let Some(id) = group {
        let group_name = doc.revision_groups.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default();
        if !group_name.is_empty() {
            ui.label(RichText::new(format!("▸ {group_name}")).size(11.0).color(t.text_dim));
        }
        if widgets::secondary_button(ui, tl!("Ungroup"), 0.0).clicked() {
            let _ = app.run("revision.ungroup", json!({ "group": id }));
        }
    }
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
            app.revision_tool.selected =
                vec![app.session.active().map_or(0, |st| st.doc.revision_markers.len().saturating_sub(1))];
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
        press(&mut app, [140.0, 100.0], false, false);
        assert_eq!(app.revision_tool.selected, vec![0], "the press picks the marker under the pointer");
        move_to(&mut app, [180.0, 120.0], false, 1.0);
        let preview = app.revision_tool.preview.clone().unwrap();
        assert_eq!((preview.dx, preview.dy), (40.0, 20.0), "the ghost frame follows the pointer");
        release(&mut app, [180.0, 120.0]);
        assert_eq!(markers(&app), [([140.0, 100.0], "Marker 1".to_string())], "moved by the pointer's travel");
        assert_eq!(app.revision_tool.preview, None, "the ghost frame is gone");
        let steps = app.session.active().unwrap().history.entries();
        assert_eq!(steps.last().map(String::as_str), Some("Move Revision Marker"), "one step for the move");
    }

    /// ⌥-drag copies the marker where it lands, under the next free number, and leaves the original
    /// where it was.
    #[test]
    fn alt_dragging_a_marker_copies_it() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], true, false);
        move_to(&mut app, [240.0, 200.0], false, 1.0);
        release(&mut app, [240.0, 200.0]);
        let markers = markers(&app);
        assert_eq!(markers.len(), 2, "a copy is added");
        assert_eq!(markers[0], ([100.0, 80.0], "Marker 1".to_string()), "the original stays");
        assert_eq!(markers[1], ([200.0, 180.0], "Marker 2".to_string()), "the copy lands where the drag put it");
        assert_eq!(app.revision_tool.selected, vec![1], "the copy is picked, ready to be sized");
    }

    /// A press and release with no travel only picks the marker: nothing moves, nothing is added.
    #[test]
    fn a_click_picks_a_marker_without_moving_it() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], false, false);
        move_to(&mut app, [140.0, 100.0], false, 1.0);
        release(&mut app, [140.0, 100.0]);
        assert_eq!(markers(&app), [([100.0, 80.0], "Marker 1".to_string())], "unchanged");
        assert_eq!(app.revision_tool.selected, vec![0], "but picked");
        // A press on empty canvas clears the pick, so the options bar stops editing a marker.
        press(&mut app, [20.0, 20.0], false, false);
        assert!(app.revision_tool.selected.is_empty(), "a click on empty canvas clears the pick");
    }

    /// ⇧ keeps the drag on one axis: horizontally while the pointer travels further sideways than
    /// up or down, vertically when it is the other way round, so the marker stays on its row.
    #[test]
    fn shift_dragging_constrains_a_marker_to_one_axis() {
        let mut app = app();
        press(&mut app, [140.0, 100.0], false, false);
        // Further sideways than up: the vertical part of the travel is dropped.
        move_to(&mut app, [220.0, 112.0], true, 1.0);
        let preview = app.revision_tool.preview.clone().unwrap();
        assert_eq!((preview.dx, preview.dy), (80.0, 0.0), "moved 80 px sideways, 0 px down");
        // Further up than sideways: the horizontal part is dropped instead.
        move_to(&mut app, [148.0, 40.0], true, 1.0);
        let preview = app.revision_tool.preview.clone().unwrap();
        assert_eq!((preview.dx, preview.dy), (0.0, -60.0), "moved 60 px up, 0 px sideways");
        release(&mut app, [148.0, 40.0]);
        assert_eq!(markers(&app), [([100.0, 20.0], "Marker 1".to_string())], "the marker keeps its column");
    }

    /// Dragging one frame of a picked set moves every picked frame by the same offset, in one undo
    /// step: the "batch move" the panel's grouping relies on.
    #[test]
    fn dragging_one_picked_frame_moves_the_picked_set() {
        let mut app = app();
        app.run("revision.add", json!({"x": 200.0, "y": 180.0, "width": 80.0, "height": 40.0, "name": "Marker 2"}))
            .unwrap();
        press(&mut app, [140.0, 100.0], false, false);
        press(&mut app, [220.0, 190.0], false, true);
        assert_eq!(app.revision_tool.selected, vec![0, 1], "both frames picked");
        // A press on one of the picked frames keeps the set and drags the whole of it.
        press(&mut app, [140.0, 100.0], false, false);
        let preview = app.revision_tool.preview.clone();
        assert!(preview.is_none(), "nothing moves until the pointer travels");
        move_to(&mut app, [160.0, 110.0], false, 1.0);
        let preview = app.revision_tool.preview.clone().unwrap();
        assert_eq!(preview.targets.len(), 2, "both frames are dragged");
        release(&mut app, [160.0, 110.0]);
        assert_eq!(
            markers(&app),
            [([120.0, 90.0], "Marker 1".to_string()), ([220.0, 190.0], "Marker 2".to_string())],
            "both frames took the same offset"
        );
        let steps = app.session.active().unwrap().history.entries();
        assert_eq!(steps.last().map(String::as_str), Some("Move Revision Markers"), "one step for the set");
    }

    /// A frame dragged to within a few pixels of another frame's left edge snaps onto it, and the
    /// guide it snapped to is recorded so the canvas can draw the line.
    #[test]
    fn a_drag_snaps_to_a_neighbours_edge() {
        let mut app = app();
        app.run("revision.add", json!({"x": 220.0, "y": 200.0, "width": 80.0, "height": 40.0, "name": "Marker 2"}))
            .unwrap();
        // The second frame's left edge sits at x 220: a drag that lands the first frame's left edge at
        // 218 is pulled the last 2 px, and a vertical guide is drawn on x 220.
        press(&mut app, [140.0, 100.0], false, false);
        move_to(&mut app, [258.0, 130.0], false, 1.0);
        let preview = app.revision_tool.preview.clone().unwrap();
        let snapped = preview.targets[0].1[0] + preview.dx;
        assert_eq!(snapped, 220.0, "the left edge landed exactly on the neighbour's");
        assert!(
            preview.guides.iter().any(|g| g.vertical && g.at == 220.0),
            "the guide it snapped to is drawn: {:?}",
            preview.guides
        );
    }

    /// Shift-click adds frames to the picked set and takes them out again; a plain click on empty
    /// canvas clears it, so the align row stops acting on frames that are no longer picked.
    #[test]
    fn shift_click_picks_several_markers() {
        let mut app = app();
        app.run("revision.add", json!({"x": 200.0, "y": 180.0, "width": 80.0, "height": 40.0, "name": "Marker 2"}))
            .unwrap();
        press(&mut app, [140.0, 100.0], false, false);
        assert_eq!(app.revision_tool.selected, vec![0], "a plain click picks one frame");
        press(&mut app, [220.0, 190.0], false, true);
        assert_eq!(app.revision_tool.selected, vec![0, 1], "shift-click adds the second frame");
        press(&mut app, [220.0, 190.0], false, true);
        assert_eq!(app.revision_tool.selected, vec![0], "shift-clicking it again takes it out");
        press(&mut app, [20.0, 20.0], false, false);
        assert!(app.revision_tool.selected.is_empty(), "a click on empty canvas clears the set");
    }
}
