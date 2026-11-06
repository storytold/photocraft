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

    // The Crop tool's default mode defers the image until its camera is known; under a
    // workspace there is still nothing to draw.
    app.ui.tool = crate::Tool::Crop;
    assert!(crate::crop_mode::moves_image(&app));
    assert!(document_shapes(&mut app) > 0, "the Crop tool draws the document");
    crate::liquify_ui::open(&mut app, &ctx).unwrap();
    assert_eq!(document_shapes(&mut app), 0, "nothing under a workspace with the Crop tool either");
    crate::liquify_ui::cancel(&mut app);
    app.ui.tool = crate::Tool::Move;

    // The other full-window workspaces cover the canvas the same way.
    app.run("select.all", json!({})).unwrap();
    app.sync_views();
    type Open = fn(&mut PhotocraftApp, &egui::Context) -> Result<(), String>;
    type Cancel = fn(&mut PhotocraftApp);
    let workspaces: [(&str, Open, Cancel); 4] = [
        ("Select and Mask", |app, _| crate::select_mask_ui::open(app), crate::select_mask_ui::cancel),
        ("Content-Aware Fill", |app, _| crate::caf_ui::open(app), crate::caf_ui::cancel),
        ("Filter Gallery", |app, _| crate::gallery_ui::open(app), |app| app.distort.gallery = None),
        ("Adaptive Wide Angle", crate::wide_angle_ui::open, |app| app.wide_angle = None),
    ];
    for (name, open, cancel) in workspaces {
        open(&mut app, &ctx).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(crate::canvas::canvas_covered(&app), "{name} covers the canvas");
        assert_eq!(document_shapes(&mut app), 0, "nothing under {name}");
        assert!(document_shapes_in(&mut app, floating) > 0, "a floating document window keeps its image under {name}");
        cancel(&mut app);
        assert!(!crate::canvas::canvas_covered(&app), "{name} closed");
        assert!(document_shapes(&mut app) > 0, "the document is drawn again after {name}");
    }
}

/// The grid is under a full-window workspace too: its lines aren't drawn while one is open.
#[test]
fn a_covered_canvas_draws_no_grid() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0));
    let view = View { zoom: 1.0, center: [60.0, 40.0], doc_size: [120, 80], fit_pending: false, ..Default::default() };
    let line_shapes = |app: &mut PhotocraftApp| {
        let viewport_id = egui::ViewportId::ROOT;
        let mut raw = egui::RawInput { screen_rect: Some(rect), viewport_id, ..Default::default() };
        raw.viewports.entry(viewport_id).or_default();
        let mut output = ctx.run_ui(raw, |ui| {
            canvas_view(app, ui, 0, rect, view.clone(), true);
        });
        output.textures_delta.clear();
        output.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. } | egui::Shape::Path(_))).count()
    };
    app.ui.view.extras = true;
    app.ui.extras.grid = false;
    let without = line_shapes(&mut app);
    app.ui.extras.grid = true;
    assert!(line_shapes(&mut app) > without, "the grid adds lines");
    crate::liquify_ui::open(&mut app, &ctx).unwrap();
    let covered = line_shapes(&mut app);
    app.ui.extras.grid = false;
    assert_eq!(covered, line_shapes(&mut app), "no grid lines under a full-window workspace");
    crate::liquify_ui::cancel(&mut app);
    app.ui.extras.grid = true;
    assert!(line_shapes(&mut app) > without, "the grid is back once it closes");
}
