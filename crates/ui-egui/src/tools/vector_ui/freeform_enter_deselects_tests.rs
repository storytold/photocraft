//! #12833: ↩ right after a Freeform Pen stroke deselects the Work Path, as in Photoshop: no
//! outline is drawn, the Make buttons are greyed, and the next stroke replaces it. Esc keeps it
//! selected and the next stroke joins it. In every other case ↩ changes nothing (not measured).

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::super::overlay::overlay_paths;
use super::super::pen_draw_tests::drag;
use super::super::test_input::harness;
use super::super::{PathRow, path_rows, work_path_shown};
use crate::PhotocraftApp;
use crate::state::Tool;
use craft::draw::ui::VectorMode;

type H = Harness<'static, Box<PhotocraftApp>>;

const LINE: [[f32; 2]; 2] = [[20.0, 20.0], [120.0, 20.0]];

/// A sine wave across the canvas, starting at (30, 50).
fn sine() -> Vec<[f32; 2]> {
    (0..=12).map(|i| [30.0 + i as f32 * 12.0, 50.0 + 20.0 * (i as f32 * std::f32::consts::FRAC_PI_3).sin()]).collect()
}

/// How many paths the canvas overlay shows.
fn overlaid(h: &H) -> usize {
    let doc = h.state().session.active().unwrap().doc.id;
    overlay_paths(h.state(), doc).len()
}

/// A click on the Work Path's row in the Paths panel: it selects the Work Path and shows it again.
fn select_work_path_row(h: &mut H) {
    let app = h.state_mut();
    app.ui.hidden_work_path = None;
    app.ui.selected_path = Some("work".into());
    h.run_steps(2);
}

fn press(h: &mut H, key: Key) {
    h.key_press(key);
    h.run_steps(2);
}

/// The first anchor of each subpath of the Work Path, if there is one.
fn work_starts(h: &H) -> Vec<[f64; 2]> {
    let doc = &h.state().session.active().unwrap().doc;
    doc.work_path.iter().flat_map(|p| &p.subpaths).filter_map(|s| s.knots.first()).map(|k| [k.anchor.x.round(), k.anchor.y.round()]).collect()
}

/// Make: Selection… and Shape (Mask's label isn't the bar's alone in the full app).
fn make_enabled(h: &H) -> [bool; 2] {
    ["Selection…", "Shape"].map(|label| !h.get_by_label(label).accesskit_node().is_disabled())
}

/// What ↩ could change: the Paths panel selection, whether the Work Path is drawn, the History.
fn state(h: &H) -> (Option<String>, bool, usize, Option<String>) {
    let app = h.state();
    let history = &app.session.active().unwrap().history;
    (app.ui.selected_path.clone(), work_path_shown(app), history.past_len(), history.undo_label().map(str::to_owned))
}

fn assert_enter_changes_nothing(h: &mut H, case: &str) {
    let before = state(h);
    press(h, Key::Enter);
    assert_eq!(state(h), before, "{case}");
}

#[test]
fn enter_deselects_the_work_path_and_the_next_stroke_replaces_it() {
    let mut h = harness(Tool::FreeformPen);
    drag(&mut h, &LINE, None);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"), "the stroke is committed on release");
    let (_, _, steps, label) = state(&h);
    press(&mut h, Key::Enter);
    let st = h.state().session.active().unwrap();
    assert!(path_rows(&st.doc, st.active_layer).iter().any(|r| r.kind == PathRow::Work), "the Work Path is still in the Paths panel");
    assert_eq!(state(&h), (None, false, steps, label), "but not selected nor drawn, and the History is as it was");
    assert_eq!(overlaid(&h), 0, "no outline on the canvas");

    drag(&mut h, &sine(), None);
    assert_eq!(work_starts(&h), [[30.0, 50.0]], "the sine replaced the line");
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"), "and is selected");
}

#[test]
fn escape_keeps_the_work_path_selected_and_the_next_stroke_joins_it() {
    let mut h = harness(Tool::FreeformPen);
    drag(&mut h, &LINE, None);
    press(&mut h, Key::Escape);
    assert_eq!(state(&h).0.as_deref(), Some("work"));
    assert!(overlaid(&h) > 0, "the outline stays");
    assert_eq!(make_enabled(&h), [true; 2]);
    drag(&mut h, &sine(), None);
    assert_eq!(work_starts(&h), [[20.0, 20.0], [30.0, 50.0]], "a second subpath");
}

/// Delete right after ↩ reaches Delete Layer, as after ↩ with the Pen (#12892), instead of
/// deleting the Work Path as after Esc (#5702).
#[test]
fn delete_after_enter_deletes_the_layer_not_the_work_path() {
    let mut h = harness(Tool::FreeformPen);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    drag(&mut h, &LINE, None);
    press(&mut h, Key::Enter);
    press(&mut h, Key::Delete);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1, "only the Background is left");
    assert_eq!(work_starts(&h), [[20.0, 20.0]], "the Work Path stays");
}

#[test]
fn enter_does_nothing_with_the_work_path_selected_in_the_paths_panel() {
    let mut h = harness(Tool::FreeformPen);
    let line = json!({"subpaths": [{"closed": false, "knots": [{"anchor": [20, 20]}, {"anchor": [120, 20]}]}]});
    h.state_mut().run("path.set", json!({"name": "work", "path": line})).unwrap();
    select_work_path_row(&mut h);
    assert_eq!(state(&h).0.as_deref(), Some("work"), "the row selects the Work Path");
    assert_enter_changes_nothing(&mut h, "no stroke since it was selected");
}

#[test]
fn enter_does_nothing_after_undo() {
    for strokes in [1, 2] {
        let mut h = harness(Tool::FreeformPen);
        for _ in 0..strokes {
            drag(&mut h, &LINE, None);
        }
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(2);
        assert_eq!(work_starts(&h).len(), strokes - 1, "Undo took the last stroke back");
        assert_enter_changes_nothing(&mut h, &format!("after {strokes} strokes and Undo"));
    }
}

/// Redo brings the last stroke back but is not a stroke: ↩ after it keeps today's behaviour.
#[test]
fn enter_does_nothing_after_undo_and_redo() {
    let mut h = harness(Tool::FreeformPen);
    drag(&mut h, &LINE, None);
    drag(&mut h, &sine(), None);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(2);
    assert_eq!(work_starts(&h).len(), 2, "Redo brought the second stroke back");
    assert_eq!(state(&h).0.as_deref(), Some("work"));
    assert_enter_changes_nothing(&mut h, "after Undo and Redo");
}

/// A stroke into a saved path leaves the Work Path as it was: ↩ with that Work Path selected
/// afterwards in the Paths panel keeps today's behaviour.
#[test]
fn enter_does_nothing_after_a_saved_path_stroke_and_selecting_the_work_path() {
    let mut h = harness(Tool::FreeformPen);
    let line = json!({"subpaths": [{"closed": false, "knots": [{"anchor": [10, 90]}, {"anchor": [60, 90]}]}]});
    h.state_mut().run("path.set", json!({"name": "work", "path": line})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Saved", "path": line})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    h.run_steps(2);
    drag(&mut h, &LINE, None);
    select_work_path_row(&mut h);
    assert_eq!(state(&h).0.as_deref(), Some("work"), "the row selects the Work Path");
    assert_enter_changes_nothing(&mut h, "a saved path stroke, then the Work Path selected");
}

#[test]
fn enter_keeps_a_saved_or_shape_layer_path_selected() {
    let mut h = harness(Tool::FreeformPen);
    let saved = json!({"subpaths": [{"closed": false, "knots": [{"anchor": [10, 90]}, {"anchor": [60, 90]}]}]});
    h.state_mut().run("path.set", json!({"name": "Saved", "path": saved})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    h.run_steps(2);
    drag(&mut h, &LINE, None);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.paths.iter().find(|p| p.name == "Saved").map(|p| p.path.subpaths.len()), Some(2), "the stroke joined the saved path");
    assert_enter_changes_nothing(&mut h, "a saved path");

    h.state_mut().ui.tool_options.vector_mode = VectorMode::Shape;
    drag(&mut h, &sine(), None);
    assert_eq!(state(&h).0.as_deref(), Some("layer"), "the stroke made a shape layer");
    assert_enter_changes_nothing(&mut h, "a shape layer's path");
}

#[test]
fn enter_after_a_stroke_into_a_vector_mask_changes_nothing() {
    let mut h = harness(Tool::FreeformPen);
    drag(&mut h, &LINE, None);
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.vectorMask.add", json!({"layer": id.0, "path": {"subpaths": [{"closed": true, "knots": [[150, 70], [190, 70], [190, 95]]}]}})).unwrap();
    app.ui.vector_mask_target = true;
    h.run_steps(2);
    drag(&mut h, &sine(), None);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layer(id).unwrap().vector_mask.as_ref().map(|m| m.path.subpaths.len()), Some(2), "the stroke went into the mask");
    assert_eq!(work_starts(&h), [[20.0, 20.0]], "not into the Work Path");
    assert_enter_changes_nothing(&mut h, "a vector mask");
}

/// ↩ deselects only once per stroke: the Work Path selected again in the Paths panel after ↩
/// keeps today's behaviour, and the next stroke joins it.
#[test]
fn enter_after_reselecting_the_work_path_in_the_paths_panel_does_nothing() {
    let mut h = harness(Tool::FreeformPen);
    drag(&mut h, &LINE, None);
    press(&mut h, Key::Enter);
    select_work_path_row(&mut h);
    assert_eq!(state(&h).0.as_deref(), Some("work"), "the row selects the Work Path again");
    assert_enter_changes_nothing(&mut h, "reselected after ↩");
    drag(&mut h, &sine(), None);
    assert_eq!(work_starts(&h), [[20.0, 20.0], [30.0, 50.0]], "the sine joins the line");
}

/// ↩ belongs to the document the stroke went into: after a stroke in A, ↩ back in B, whose last
/// step is also a Freeform Pen stroke and whose Work Path is selected again in the Paths panel,
/// changes nothing there.
#[test]
fn enter_in_another_document_after_a_stroke_changes_nothing() {
    let mut h = harness(Tool::FreeformPen);
    h.state_mut().run("file.new", json!({"width": 200, "height": 100})).unwrap();
    h.run_steps(2);
    drag(&mut h, &LINE, None);
    let switch = |h: &mut H, index| {
        assert!(h.state_mut().session.set_active(index));
        h.run_steps(2);
    };
    switch(&mut h, 0);
    drag(&mut h, &sine(), None);
    switch(&mut h, 1);
    select_work_path_row(&mut h);
    assert_eq!(state(&h).0.as_deref(), Some("work"), "the row selects B's Work Path");
    assert_enter_changes_nothing(&mut h, "a stroke in another document");
    drag(&mut h, &sine(), None);
    assert_eq!(work_starts(&h), [[20.0, 20.0], [30.0, 50.0]], "the sine joins B's line");
}
