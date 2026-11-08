//! #12086: with Scrubby Zoom off, a Zoom tool drag shows the rectangle's size in image pixels
//! beside the pointer, as the marquee does, and Show Transformation Values off hides it.

use super::*;
use egui::{Event, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness(show_values: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("prefs.set", json!({"values": {"tools.showTransformationValues": show_values}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = crate::state::Tool::Zoom;
    app.ui.tool_options.zoom_scrubby = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() });
    h.step();
}

/// Press, then drag 300 × 150 points; returns the press point and the zoom (points per image pixel).
fn drag(h: &mut Harness<'static, PhotocraftApp>) -> (Pos2, f32) {
    let p0 = h.state().last_canvas_rect.center() - vec2(150.0, 75.0);
    let zoom = ViewXform::active(h.state()).unwrap().zoom;
    h.event(Event::PointerMoved(p0));
    h.step();
    button(h, p0, true);
    for i in 1..=6 {
        h.event(Event::PointerMoved(p0 + vec2(50.0, 25.0) * i as f32));
        h.step();
    }
    h.step();
    (p0, zoom)
}

#[test]
fn a_zoom_rectangle_shows_its_size_in_image_pixels() {
    let mut h = harness(true);
    let (p0, zoom) = drag(&mut h);
    let (w, h_px) = ((300.0 / zoom).round(), (150.0 / zoom).round());
    assert!(h.query_by_label("W:").is_some() && h.query_by_label("H:").is_some(), "no readout while dragging");
    assert!(h.query_by_label(&format!("{w} px")).is_some(), "width {w} px not shown");
    assert!(h.query_by_label(&format!("{h_px} px")).is_some(), "height {h_px} px not shown");
    let before = h.state().current_zoom();
    button(&mut h, p0 + vec2(300.0, 150.0), false);
    h.run_steps(2);
    assert!(h.query_by_label("W:").is_none(), "readout stays after the release");
    assert!(h.state().current_zoom() > before, "the release still zooms to the rectangle");
}

#[test]
fn show_transformation_values_off_hides_the_readout() {
    let mut h = harness(false);
    drag(&mut h);
    assert!(h.query_by_label("W:").is_none());
}
