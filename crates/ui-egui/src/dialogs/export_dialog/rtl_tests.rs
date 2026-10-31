//! The Export As body mirrors in a right-to-left UI: the settings pane moves to the reading
//! start (the right) and every row inside it reads from the right, the editor at the left.

use super::*;
use egui_kittest::kittest::Queryable;

/// The rectangles the export body lays out: the settings pane's heading, the TGA row's label
/// and its dropdown, and the estimate label under the preview.
struct Layout {
    settings: egui::Rect,
    resolution: egui::Rect,
    depth: egui::Rect,
    estimate: egui::Rect,
}

/// The export body of a 320 × 200 document, drawn in `dir`.
fn render(dir: crate::direction::Dir) -> Layout {
    let _dir = crate::direction::direction_scope(dir);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 320, "height": 200})).unwrap();
    let mut fields = Map::new();
    fields.insert("format".into(), json!("tga"));
    fields.insert("scale".into(), json!(100));
    fields.insert("sizing".into(), json!("scale"));
    fields.insert("width".into(), json!(320));
    fields.insert("height".into(), json!(200));
    fields.insert("transparency".into(), json!(true));
    fields.insert("metadata".into(), json!("none"));
    fields.insert("tgaBits".into(), json!("auto"));
    fields.insert("profile".into(), json!("document"));
    fields.insert("embedProfile".into(), json!(true));
    fields.insert("resample".into(), json!(DEFAULT_RESAMPLE));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, state| {
            // The named font families land in the second pass; drawing before that paints text
            // with an empty family and panics.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            body(state, ui, &mut fields.clone());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    Layout {
        settings: h.get_by_label("File Settings").rect(),
        resolution: h.get_by_label("Resolution").rect(),
        depth: h.get_by_label("Depth").rect(),
        estimate: h.get_by_label_contains("≈").rect(),
    }
}

/// LTR keeps the settings pane first and every row's label before its editor; RTL swaps both.
#[test]
fn the_export_body_mirrors_in_a_right_to_left_ui() {
    let ltr = render(crate::direction::Dir::Ltr);
    assert!(ltr.resolution.right() < ltr.depth.left(), "LTR: the label reads before the dropdown");
    assert!(ltr.settings.left() < ltr.estimate.left(), "LTR: the settings pane comes first");
    // LTR: the settings pane is the reading-start pane; its heading sits at the row's left edge.
    assert!(ltr.settings.left() <= ltr.resolution.left().min(ltr.estimate.left()), "LTR: the settings pane is at the reading start");

    let rtl = render(crate::direction::Dir::Rtl);
    assert!(rtl.depth.right() < rtl.resolution.left(), "RTL: the label reads after the dropdown");
    assert!(rtl.estimate.left() < rtl.settings.left(), "RTL: the settings pane comes last");
    // RTL: the settings pane is still the reading-start pane, now at the row's right edge.
    assert!(rtl.settings.right() >= rtl.resolution.right().max(rtl.estimate.right()), "RTL: the settings pane hugs the reading edge");
}
