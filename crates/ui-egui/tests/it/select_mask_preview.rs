//! Preview quality changes display resolution without choosing a different selection.

use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_ui_egui::{PhotocraftApp, select_mask_ui};
use serde_json::{Value, json};

/// A small red object on a wide blue canvas, large enough to use a reduced preview.
fn app(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 3600, "height": 600, "depth": depth})).unwrap();
    app.session
        .edit("fixture", |doc, _| {
            let s = doc.layers[0].surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 3600, 600), &[0.0, 0.0, 1.0, 1.0]);
            s.fill_rect(Rect::new(1575, 150, 2025, 450), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
    select_mask_ui::open(&mut app).unwrap();
    select_mask_ui::control(&mut app, &json!({"clearSelection": true})).unwrap();
    app
}

/// Wait for the requested preview and count its selected pixels.
fn preview(app: &mut PhotocraftApp, settings: Value) -> u64 {
    select_mask_ui::control(app, &settings).unwrap();
    select_mask_ui::control(app, &json!({"waitPreview": true})).unwrap()["selectedProxyPixels"].as_u64().unwrap()
}

/// Count the committed mask at the same pixel positions as the preview.
fn selected(mask: &Surface, k: usize) -> u64 {
    let mut count = 0;
    for y in (0..600).step_by(k) {
        for x in (0..3600).step_by(k) {
            count += u64::from(mask.sample_channel(x, y, 0) >= 0.5);
        }
    }
    count
}

/// With refinement disabled, both preview sizes show the selection that OK commits.
#[test]
fn quick_selection_agrees_with_ok_at_both_preview_qualities() {
    for depth in [8, 16, 32] {
        let mut app = app(depth);
        let low = preview(&mut app, json!({"size": 30, "stroke": {"points": [[1800.0, 300.0]]}}));
        let high = preview(&mut app, json!({"highQualityPreview": true}));
        assert_eq!(preview(&mut app, json!({"highQualityPreview": false})), low, "switching back reuses the same selection");
        select_mask_ui::commit(&mut app).unwrap();
        let mask = app.session.active().unwrap().doc.selection.as_ref().unwrap();
        assert_eq!(high, selected(mask, 1), "{depth}-bit full-size preview agrees with OK");
        assert_eq!(low, selected(mask, 3), "{depth}-bit reduced preview samples the same selection as OK");
    }
}

/// Undo and redo discard or extend the cached selection after changing preview quality.
#[test]
fn brush_edits_and_undo_keep_the_cached_selection_in_document_coordinates() {
    let mut app = app(8);
    let first = preview(&mut app, json!({"tool": "brush", "size": 18, "stroke": {"points": [[1800.0, 300.0]]}}));
    preview(&mut app, json!({"highQualityPreview": true}));
    let both = preview(&mut app, json!({"highQualityPreview": false, "stroke": {"points": [[2400.0, 300.0]]}}));
    assert!(both > first);
    assert_eq!(preview(&mut app, json!({"undo": true})), first);
    assert_eq!(preview(&mut app, json!({"redo": true})), both);
    let inverted = preview(&mut app, json!({"invert": true}));
    assert_eq!(inverted, 1200 * 200 - both);
    select_mask_ui::commit(&mut app).unwrap();
    assert_eq!(inverted, selected(app.session.active().unwrap().doc.selection.as_ref().unwrap(), 3));
}

/// A targeted mask supplies the starting pixels even when the document also has a selection.
#[test]
fn reduced_preview_edits_the_targeted_layer_mask() {
    let mut app = app(8);
    select_mask_ui::cancel(&mut app);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 90, "y": 30, "width": 60, "height": 60})).unwrap();
    app.run("layer.layerMask.revealSelection", json!({})).unwrap();
    app.run("select.rect", json!({"x": 2700, "y": 30, "width": 3, "height": 3})).unwrap();
    app.ui.mask_target = true;
    select_mask_ui::open(&mut app).unwrap();
    assert_eq!(preview(&mut app, json!({})), 400, "the mask, rather than the separate selection");
    let low = preview(&mut app, json!({"tool": "brush", "size": 18, "stroke": {"points": [[1800.0, 300.0]]}}));
    let high = preview(&mut app, json!({"highQualityPreview": true}));
    select_mask_ui::commit(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(low, selected(mask, 3));
    assert_eq!(high, selected(mask, 1));
    assert!(st.doc.selection.is_none(), "output to the layer mask consumes the document selection");
}
