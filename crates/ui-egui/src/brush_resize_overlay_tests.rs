//! The brush-resize size preview through the real canvas: Alt + right-drag shows the tip falloff in the canvas body.

use egui::epaint::Shape;
use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 50.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

fn flatten<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
    if let Shape::Vec(v) = shape {
        v.iter().for_each(|s| flatten(s, out));
    } else {
        out.push(shape);
    }
}

/// The untextured meshes with a transparent outer ring (the falloff) round `press` painted this frame, each with the clip rect it
/// was painted under.
fn falloff_meshes(h: &Harness<'static, Box<PhotocraftApp>>, press: egui::Pos2) -> Vec<(egui::Rect, egui::Rect)> {
    let mut found = Vec::new();
    for clipped in &h.output().shapes {
        let mut all = Vec::new();
        flatten(&clipped.shape, &mut all);
        for s in all {
            if let Shape::Mesh(m) = s
                && m.texture_id == egui::TextureId::default()
                && m.vertices.iter().any(|v| v.color == egui::Color32::TRANSPARENT)
                && m.calc_bounds().contains(press)
                && m.calc_bounds().width() < 400.0
            {
                found.push((m.calc_bounds(), clipped.clip_rect));
            }
        }
    }
    found
}

/// Alt + right-drag resizes the brush and shows the falloff preview inside the canvas body, not clipped to a
/// component rect of its own, with the diameter readout beside the pointer.
#[test]
fn the_resize_drag_shows_the_size_preview_in_the_canvas_body() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Brush;
    let size = h.state().session.tools.brush.size;
    let from = crate::canvas::testing::to_screen(h.state(), [100.0, 50.0]);
    let alt = Modifiers::ALT;
    h.event(egui::Event::ModifiersChanged(alt));
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Secondary, pressed: true, modifiers: alt });
    h.run_steps(1);
    for i in 1..=6 {
        h.event(egui::Event::PointerMoved(from + vec2(i as f32 * 8.0, 0.0)));
        h.run_steps(1);
    }
    assert_ne!(h.state().session.tools.brush.size, size, "the drag changed the brush size");

    // The canvas body fills the harness window; a component rect of its own is a few dozen points.
    let mut text = Vec::new();
    for c in &h.output().shapes {
        let mut all = Vec::new();
        flatten(&c.shape, &mut all);
        text.extend(all.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }));
    }
    assert!(text.iter().any(|t| t == "Diameter:") && text.iter().any(|t| t.ends_with(" px")), "the diameter readout shows beside the pointer: {text:?}");
    let body = h.ctx.content_rect();
    let meshes = falloff_meshes(&h, from);
    assert!(!meshes.is_empty(), "a falloff mesh paints during the resize");
    for (bounds, clip) in meshes {
        assert!(clip.contains_rect(bounds.intersect(body)) && clip.width() >= body.width() * 0.5, "the preview is clipped to {clip:?}, not the body {body:?}");
    }
}
