//! Smart Guides › measuring distances, as in Photoshop (#4946): with Smart Guides on (View › Show
//! › Smart Guides, Extras on) and the Move tool, holding ⌘ (Ctrl off the Mac) with no button down
//! and hovering another layer shows the distances between the selected layers' bounds and that
//! layer's, as lines labelled with the value in the ruler unit. Over empty canvas (or the
//! Background, or the selection itself) the distances run to the canvas edges instead. While an
//! ⌥-drag duplicates the layers, the distance between the original and the copy shows.
//!
//! It's an overlay only: nothing changes in the document and there is no command or History
//! state. The hovered layer is the one a click there would Auto-Select
//! (`pick_cmds::move_auto_select_target`), cached per document revision, selection and pointer pixel,
//! so a pointer resting on the canvas costs nothing per frame. The snapped lines of a move are the
//! tool host's; this overlay draws only the distances and their dashed extension lines.

use craft::behavior::Axis;
use craft::behavior::kurbo::Rect;
use craft::draw::ui::measure::box_distances;
use craft::ui::canvas_controls::SmartGuides;
use craft::ui::contract::Show as _;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::snap::union;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The last answer: the boxes measured with the pointer at `at` on that revision and selection.
pub(crate) struct Cache {
    doc: DocId,
    revision: u64,
    at: [i32; 2],
    group: bool,
    selected: Vec<LayerId>,
    boxes: Option<([f64; 4], [f64; 4])>,
}

/// The selected layers' box and the box to measure it to with the pointer at `at` (document px):
/// the layer a click there would Auto-Select, or the canvas where that is no other layer.
fn boxes(app: &mut PhotocraftApp, at: [f64; 2]) -> Option<([f64; 4], [f64; 4])> {
    if !at[0].is_finite() || !at[1].is_finite() {
        return None;
    }
    let at = [at[0].floor() as i32, at[1].floor() as i32];
    let group = app.ui.tool_options.move_target == "group";
    let st = app.session.active()?;
    let selected = st.selected_layers();
    let (doc, revision) = (st.doc.id, st.revision);
    if let Some(c) = app.move_distance.as_ref().filter(|c| c.doc == doc && c.revision == revision && c.at == at && c.group == group && c.selected == selected) {
        return c.boxes;
    }
    let d = &st.doc;
    let a = union(selected.iter().filter(|id| !d.is_background(**id)).filter_map(|id| crate::snap_ui::tree_rect(d, *id)));
    let canvas = [0.0, 0.0, d.size.width as f64, d.size.height as f64];
    let hovered = photocraft_engine::pick_cmds::move_auto_select_target(d, at[0], at[1], group).filter(|id| {
        // The selection itself (or a layer in a selected group) isn't another layer.
        !d.is_background(*id) && !selected.iter().any(|s| *s == *id || d.path_of(*id).zip(d.path_of(*s)).is_some_and(|(p, q)| p.starts_with(&q)))
    });
    let boxes = a.map(|a| (a, hovered.and_then(|id| crate::snap_ui::tree_rect(d, id)).unwrap_or(canvas)));
    app.move_distance = Some(Cache { doc, revision, at, group, selected, boxes });
    boxes
}

fn rect([x0, y0, x1, y1]: [f64; 4]) -> Rect {
    Rect::new(x0, y0, x1, y1)
}

/// What to measure now: the ⌥-drag's original and copy while it duplicates, else, with ⌘ held
/// (`command`) and no button down (`pressed`), the selection and what's under the pointer `at`.
/// `None` when Smart Guides are off or another tool is current.
pub(crate) fn current(app: &mut PhotocraftApp, at: Option<[f64; 2]>, command: bool, pressed: bool) -> Option<(Rect, Rect)> {
    if app.ui.tool != Tool::Move || !app.ui.view.shows(app.ui.view.show.smart_guides) || app.ui.transform.is_some() {
        return None;
    }
    if let Some(pair) = duplicate_drag(app) {
        return Some(pair);
    }
    // A button down covers a canvas drag and a guide drag alike.
    if !command || pressed {
        return None;
    }
    let (a, b) = boxes(app, at?)?;
    Some((rect(a), rect(b)))
}

/// An ⌥-drag that has duplicated the layers: the original's box and the copy's where it is now.
fn duplicate_drag(app: &PhotocraftApp) -> Option<(Rect, Rect)> {
    if !crate::move_mods::duplicated(app) {
        return None;
    }
    let (r, d) = app.canvas_tools.mover.dragged_box()?;
    Some((rect(r), rect([r[0] + d[0], r[1] + d[1], r[2] + d[0], r[3] + d[1]])))
}

/// Shows the measurements over the canvas `rect`, in the Smart Guides colour. `pointer` is the
/// screen hover position.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, pointer: Option<egui::Pos2>) {
    let command = craft::ui::runtime::pointer::frame(ui.ctx()).mods.cmd;
    let pressed = crate::canvas::tools::in_gesture(app);
    let at = pointer.and_then(|p| crate::canvas::document_display::doc_point_at(app, p)).map(|p| [p.x, p.y]);
    let Some((a, b)) = current(app, at, command, pressed) else { return };
    let Some(st) = app.session.active() else { return };
    let (dpi, size) = (f64::from(st.doc.resolution_dpi), st.doc.size);
    let ur = &app.session.prefs().units_and_rulers;
    let (lines, measures) = box_distances(a, b, |len, axis| {
        let extent = (match axis {
            Axis::X => size.width,
            _ => size.height,
        }) as f64;
        format!("{} {}", ur.format(len, dpi, extent), ur.rulers.suffix())
    });
    let guides = SmartGuides::new(&lines, &measures);
    let guides = match crate::snap_ui::smart_guide_color(app) {
        Some(color) => guides.color(color),
        None => guides,
    };
    guides.show_at(ui, canvas);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The `snap_ui` test document: a target box at (300..350, 200..260) and the selected mover
    /// at (40..90, 40..80) on a 400×300 canvas.
    fn app_with_boxes() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({"name": "target"})).unwrap();
        app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.view.extras = true;
        app.ui.view.show.smart_guides = true;
        app
    }

    #[test]
    fn command_hover_over_another_layer_measures_to_it() {
        let mut app = app_with_boxes();
        let revision = app.session.active().unwrap().revision;
        let mover = Rect::new(40.0, 40.0, 90.0, 80.0);
        let canvas = Rect::new(0.0, 0.0, 400.0, 300.0);
        let (a, b) = current(&mut app, Some([320.5, 230.0]), true, false).unwrap();
        assert_eq!((a, b), (mover, Rect::new(300.0, 200.0, 350.0, 260.0)));
        // Over empty canvas (the Background): to the canvas.
        assert_eq!(current(&mut app, Some([200.0, 150.0]), true, false), Some((mover, canvas)));
        // Over the selected layer itself: also the canvas.
        assert_eq!(current(&mut app, Some([60.0, 60.0]), true, false), Some((mover, canvas)));
        // Only an overlay: the document didn't change.
        assert_eq!(app.session.active().unwrap().revision, revision);
        let target = app.session.active().unwrap().doc.layers.iter().find(|l| l.name == "target").unwrap().id;
        app.run("layer.setProps", json!({"layer": target.0, "locks": {"position": true}})).unwrap();
        assert_eq!(current(&mut app, Some([320.5, 230.0]), true, false), Some((mover, canvas)), "a position-locked target measures to the canvas");
    }

    #[test]
    fn nothing_without_command_smart_guides_or_the_move_tool() {
        let mut app = app_with_boxes();
        let at = Some([320.5, 230.0]);
        assert!(current(&mut app, at, true, false).is_some());
        // ⌘ not held, or a button down (⌘-click auto-selects, unchanged).
        assert_eq!(current(&mut app, at, false, false), None);
        assert_eq!(current(&mut app, at, true, true), None);
        assert_eq!(current(&mut app, None, true, false), None);
        // Smart Guides off, or Extras off.
        app.ui.view.show.smart_guides = false;
        assert_eq!(current(&mut app, at, true, false), None);
        app.ui.view.show.smart_guides = true;
        app.ui.view.extras = false;
        assert_eq!(current(&mut app, at, true, false), None);
        app.ui.view.extras = true;
        // Another tool.
        app.ui.tool = Tool::Brush;
        assert_eq!(current(&mut app, at, true, false), None);
        // Nothing to measure from: the Background is selected (the Move tool doesn't move it).
        app.ui.tool = Tool::Move;
        let bg = app.session.active().unwrap().doc.walk().first().unwrap().2.id;
        app.run("layer.select", json!({"layer": bg.0})).unwrap();
        assert_eq!(current(&mut app, at, true, false), None);
    }

    #[test]
    fn alt_drag_duplicate_measures_from_the_original_to_the_copy() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = app_with_boxes();
        app.sync_views();
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, alt);
        tool_event(&mut app, ToolEvent::Move { x: 160.0, y: 60.0, pressure: 1.0 }, alt);
        // The copy sits 100 px right of the original; a button is down and ⌘ isn't held.
        let original = Rect::new(40.0, 40.0, 90.0, 80.0);
        let copy = Rect::new(140.0, 40.0, 190.0, 80.0);
        assert_eq!(current(&mut app, None, false, true), Some((original, copy)));
        tool_event(&mut app, ToolEvent::Up { x: 160.0, y: 60.0 }, alt);
        assert_eq!(current(&mut app, None, false, false), None);
    }
}
