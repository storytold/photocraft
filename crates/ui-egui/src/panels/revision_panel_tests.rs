//! The panel's Delete and Clear All buttons: they used to keep drawing rows at the index the list
//! had before they ran, so pressing them read past the end of the marker list and took PhotoCraft
//! down with it.

use super::revision_panel::{layer_under_frame, next_marker_name, panel};
use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// A 420 × 520 harness on a document with `markers` named markers, the panel already drawn.
fn harness(markers: usize) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    for i in 0..markers {
        let name = format!("Marker {}", i + 1);
        let x = 10.0 + 60.0 * i as f64;
        app.run("revision.add", json!({"x": x, "y": 20.0, "width": 50.0, "height": 40.0, "name": name})).unwrap();
    }
    // Every row's name field is a bare TextEdit with no accessible name; the check is about
    // screen-reader labels, not about the row list this test drives.
    let mut h = Harness::builder()
        .with_size(vec2(420.0, 520.0))
        .with_accessibility_check(false)
        .build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            // The panel's ✓ / ● marks come from the app's own font stack, which a bare harness
            // does not install (and the harness panics on a missing glyph).
            if !ready {
                crate::theme::install_fonts(ui.ctx());
                ready = true;
                return;
            }
            panel(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The markers' names, in document order.
fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().map(|st| st.doc.revision_markers.iter().map(|m| m.name.clone()).collect()).unwrap_or_default()
}

/// Three rows, the first one's ✕ pressed: the two below it slide up into their new rows.
#[test]
fn deleting_the_first_of_three_markers_keeps_the_other_two() {
    let mut h = harness(3);
    h.get_all_by_label("Delete").next().expect("one Delete per row").click();
    h.run_steps(4);
    assert_eq!(names(h.state()), ["Marker 2", "Marker 3"], "the first marker is gone, the others stay");
}

/// Clear All empties the list while its rows are on screen.
#[test]
fn clearing_all_markers_leaves_an_empty_list() {
    let mut h = harness(3);
    h.get_by_label("Clear All").click();
    h.run_steps(4);
    assert!(names(h.state()).is_empty());
}

/// Deleting the last row is the other edge of the same loop.
#[test]
fn deleting_the_last_marker_keeps_the_rows_above_it() {
    let mut h = harness(2);
    h.get_all_by_label("Delete").last().expect("one Delete per row").click();
    h.run_steps(4);
    assert_eq!(names(h.state()), ["Marker 1"]);
}

/// The row's own green button: the marker turns green by hand, and keeps the snapshot the next
/// check compares against (Reset, not this button, re-takes it).
#[test]
fn the_row_green_button_marks_the_marker_without_dropping_its_snapshot() {
    let mut h = harness(1);
    let snapshot = h.state().session.active().unwrap().doc.revision_markers[0].snapshot.clone();
    assert!(!snapshot.is_empty(), "the marker starts with a snapshot of its frame");
    h.get_by_label("Mark as Green").click();
    h.run_steps(4);
    let m = &h.state().session.active().unwrap().doc.revision_markers[0];
    assert!(m.done, "the marker is green");
    assert_eq!(m.snapshot, snapshot, "and keeps the snapshot it had");
    // Pressing it again turns the marker back to red.
    h.get_by_label("Mark as Red").click();
    h.run_steps(4);
    assert!(!h.state().session.active().unwrap().doc.revision_markers[0].done, "back to red");
}

/// The next marker's number comes from the highest one already used, not the length of the list:
/// deleting a marker in the middle used to make the next one repeat a number that is still there.
#[test]
fn a_new_marker_number_is_never_one_already_used() {
    let mut h = harness(3);
    h.get_all_by_label("Delete").nth(1).expect("one Delete per row").click();
    h.run_steps(4);
    assert_eq!(names(h.state()), ["Marker 1", "Marker 3"]);
    assert_eq!(next_marker_name(h.state()), "Marker 4", "the highest number in use decides, not the row count");
}

/// The frame's centre alone does not name a layer: a marker frame only covers the background, which
/// markers never bind, so binding reports "no layer" instead of binding the background.
#[test]
fn a_frame_over_the_background_binds_no_layer() {
    let h = harness(1);
    assert_eq!(layer_under_frame(h.state(), [0.0, 0.0], [800.0, 600.0]), None, "the background is not a layer to bind");
}
