//! A full-window workspace covers the canvas: the frame paints no document under it.

use super::*;
use serde_json::json;

/// The shapes a frame of `canvas_view` draws the document with (its texture, as a mesh, an
/// image or a GPU callback): some without a workspace, none with Liquify open, some again
/// once it closes. A floating document window, in a viewport of its own that the workspace's
/// area doesn't cover, keeps drawing its document meanwhile.
#[test]
fn a_covered_canvas_paints_no_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#e05030"})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0));
    let view = View { zoom: 1.0, center: [60.0, 40.0], doc_size: [120, 80], fit_pending: false, ..Default::default() };
    let document_shapes_in = |app: &mut PhotocraftApp, viewport_id: egui::ViewportId| {
        let mut raw = egui::RawInput { screen_rect: Some(rect), viewport_id, ..Default::default() };
        raw.viewports.entry(viewport_id).or_default();
        let mut output = ctx.run_ui(raw, |ui| {
            canvas_view(app, ui, 0, rect, view.clone(), viewport_id == egui::ViewportId::ROOT);
        });
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter(|s| match &s.shape {
                egui::Shape::Mesh(_) | egui::Shape::Callback(_) => true,
                egui::Shape::Rect(r) => r.brush.is_some(),
                _ => false,
            })
            .count()
    };
    let document_shapes = |app: &mut PhotocraftApp| document_shapes_in(app, egui::ViewportId::ROOT);
    assert!(document_shapes(&mut app) > 0, "the document is drawn");
    crate::liquify_ui::open(&mut app, &ctx).unwrap();
    assert!(crate::canvas::canvas_covered(&app));
    assert_eq!(document_shapes(&mut app), 0, "nothing under a full-window workspace");
    let floating = egui::ViewportId::from_hash_of("floating document window");
    assert!(document_shapes_in(&mut app, floating) > 0, "a floating document window keeps its image");
    crate::liquify_ui::cancel(&mut app);
    assert!(document_shapes(&mut app) > 0, "and again once it closes");
}
