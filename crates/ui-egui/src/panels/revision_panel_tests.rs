//! The panel's Delete and Clear All buttons: they used to keep drawing rows at the index the list
//! had before they ran, so pressing them read past the end of the marker list and took PhotoCraft
//! down with it.

use super::revision_panel::panel;
use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

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
