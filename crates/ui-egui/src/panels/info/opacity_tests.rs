use super::info_panel;
use crate::info_panel_options::Readout;
use crate::{PhotocraftApp, theme};
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Layer;
use serde_json::json;

/// A synthetic document with transparent, half-covered and opaque pixels; the readout is opted in.
fn opacity_app(mode: ColorMode, depth: SampleType) -> PhotocraftApp {
    let mut doc = photocraft_doc::Document::new("Opacity", photocraft_doc::Size::new(3, 1), mode, depth);
    let mut layer = Layer::raster("Pixels", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    let format = surface.format();
    for (x, alpha) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        surface.write_pixel(x as i32, 0, &photocraft_raster::from_rgba(&format, [1.0, 1.0, 1.0, alpha]));
    }
    doc.layers.push(layer);
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.hover_doc = Some([1.5, 0.5]);
    app.ui.info_panel.readouts = vec![Readout::ActualColor, Readout::Cmyk, Readout::Opacity];
    app
}

/// Check the value beside the selected opacity label, rather than matching a CMYK percentage.
fn assert_opacity(h: &Harness<'_, PhotocraftApp>, value: &str) {
    let label = h.get_by_label("Composite opacity:").rect();
    assert!(
        h.query_all_by_label(value).any(|node| {
            let rect = node.rect();
            rect.left() >= label.right() && (rect.center().y - label.center().y).abs() < 1.0
        }),
        "the composite opacity row displays {value}"
    );
}

/// Composite alpha follows pixel coverage and the visible stack, independently of selected-layer opacity.
#[test]
fn info_panel_shows_composite_opacity_at_every_depth_and_colour_model() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let app = opacity_app(mode, depth);
            let mut h = Harness::builder().with_size(vec2(260.0, 240.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
            for (x, expected) in [(0.5, "0%"), (1.5, "50%"), (2.5, "100%")] {
                h.state_mut().hover_doc = Some([x, 0.5]);
                h.run_steps(2);
                assert_opacity(&h, expected);
            }
            // Composite coverage, rather than the active layer's opacity setting alone.
            h.state_mut().run("layer.setProps", json!({"opacity": 0.5})).unwrap();
            h.run_steps(2);
            assert_opacity(&h, "50%");
            let pixels = h.state().session.active().unwrap().active_layer.unwrap();
            h.state_mut().run("layer.newFillLayer.solidColor", json!({"color": "#ffffff"})).unwrap();
            h.state_mut().run("layer.arrange.sendToBack", json!({})).unwrap();
            h.state_mut().run("layer.select", json!({"layer": pixels.0})).unwrap();
            h.run_steps(2);
            assert_opacity(&h, "100%");
            let st = h.state().session.active().unwrap();
            assert_eq!(st.active_layer, Some(pixels));
            assert_eq!(
                st.doc.layers.iter().find(|layer| layer.id == pixels).unwrap().opacity,
                0.5,
                "selected-layer opacity stays at 50% while composite opacity is 100%"
            );
        }
    }
}

/// The optional readout uses Eyedropper averaging and never shows a stale off-canvas sample.
#[test]
fn info_panel_opacity_uses_sample_size_and_clears_outside_the_canvas() {
    let app = opacity_app(ColorMode::Rgb, SampleType::U8);
    let mut h = Harness::builder().with_size(vec2(260.0, 240.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
    h.state_mut().hover_doc = Some([0.5, 0.5]);
    h.run_steps(2);
    assert_opacity(&h, "0%");
    h.state_mut().ui.tool_options.eyedropper_size = 5;
    h.run_steps(2);
    assert_opacity(&h, "50%");
    for pos in [None, Some([-0.5, 0.5]), Some([3.0, 0.5]), Some([0.5, 1.0])] {
        h.state_mut().hover_doc = pos;
        h.run_steps(2);
        assert_opacity(&h, "—");
    }
    h.state_mut().hover_doc = Some([2.5, 0.5]);
    h.state_mut().ui.tool_options.eyedropper_size = 1;
    h.run_steps(2);
    assert_opacity(&h, "100%");
}

/// An empty session has no sampled opacity, even when the option is enabled.
#[test]
fn info_panel_without_a_document_has_no_opacity_readout() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.info_panel.readouts = vec![Readout::ActualColor, Readout::Cmyk, Readout::Opacity];
    let h = Harness::builder().build_ui_state(|ui, app| info_panel(app, ui), app);
    assert!(h.query_by_label("No document").is_some());
    assert!(h.query_by_label("Composite opacity:").is_none());
}

/// Documents with the same revision and pointer coordinates must not share a cached sample.
#[test]
fn info_panel_opacity_cache_follows_the_active_document() {
    let app = opacity_app(ColorMode::Rgb, SampleType::U8);
    let mut h = Harness::builder().with_size(vec2(260.0, 240.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
    h.run_steps(2);
    assert_opacity(&h, "50%");
    let mut opaque = h.state().session.active().unwrap().doc.as_ref().clone();
    let surface = opaque.layers.first_mut().unwrap().surface_mut().unwrap();
    let format = surface.format();
    surface.write_pixel(1, 0, &photocraft_raster::from_rgba(&format, [1.0, 1.0, 1.0, 1.0]));
    h.state_mut().session.add_document(opaque, None);
    h.run_steps(2);
    assert_opacity(&h, "100%");
    assert!(h.state_mut().session.set_active(0));
    h.run_steps(2);
    assert_opacity(&h, "50%");
}

/// Closing and reopening a saved document can reuse its persisted ID and initial revision.
#[test]
fn info_panel_opacity_cache_follows_reopened_document_instances() {
    let app = opacity_app(ColorMode::Rgb, SampleType::U8);
    let mut h = Harness::builder().with_size(vec2(260.0, 240.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
    h.run_steps(2);
    assert_opacity(&h, "50%");
    let st = h.state().session.active().unwrap();
    let (id, revision) = (st.doc.id, st.revision);
    let mut reopened = st.doc.as_ref().clone();
    let surface = reopened.layers.first_mut().unwrap().surface_mut().unwrap();
    let format = surface.format();
    surface.write_pixel(1, 0, &photocraft_raster::from_rgba(&format, [1.0, 1.0, 1.0, 1.0]));
    h.state_mut().session.close(0).unwrap();
    h.state_mut().session.add_document(reopened, None);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.id, id, "reopening retains the saved document ID");
    assert_eq!(st.revision, revision, "the new session starts at the same revision");
    // No frame was rendered between closing and reopening: the stale sample must not survive.
    h.run_steps(2);
    assert_opacity(&h, "100%");
}

/// Both visibility states fit the narrow and wide panel in every theme; native renders are opt-in.
#[test]
fn info_panel_opacity_layout_in_every_theme() {
    let directory = std::env::var_os("PHOTOCRAFT_INFO_SNAPSHOTS").map(std::path::PathBuf::from);
    if let Some(dir) = &directory {
        std::fs::create_dir_all(dir).unwrap();
    }
    let _gpu = directory.as_ref().map(|_| photocraft_testkit::gpu_lock());
    for theme in theme::ThemeKind::ALL {
        for width in [220.0, 340.0] {
            let app = opacity_app(ColorMode::Rgb, SampleType::U8);
            let builder = Harness::builder().with_size(vec2(width, 280.0)).with_pixels_per_point(2.0);
            let builder = if directory.is_some() { builder.wgpu() } else { builder };
            let mut h = builder.build_ui_state(|ui, app| info_panel(app, ui), app);
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(3);
            assert_opacity(&h, "50%");
            let op = h.get_by_label("Composite opacity:").rect();
            assert!(op.left() >= 0.0 && op.right() < width, "{theme:?} at {width}: {op:?}");
            assert!(op.bottom() <= h.get_by_label("X:").rect().top(), "the selectable readout ends before the position rows: {theme:?} at {width}: {op:?}");
            let value = h.query_all_by_label("50%").find(|node| (node.rect().center().y - op.center().y).abs() < 1.0).unwrap().rect();
            assert!(value.right() <= width, "{theme:?} at {width}: {value:?}");
            if let Some(dir) = &directory {
                h.render().unwrap().save(dir.join(format!("info-opacity-{}-{width}-enabled.png", theme.id()))).unwrap();
            }
            // Longest percentage and the reviewer's example: a 50% selected layer over an opaque background.
            h.state_mut().run("layer.setProps", json!({"opacity": 0.5})).unwrap();
            let pixels = h.state().session.active().unwrap().active_layer.unwrap();
            h.state_mut().run("layer.newFillLayer.solidColor", json!({"color": "#ffffff"})).unwrap();
            h.state_mut().run("layer.arrange.sendToBack", json!({})).unwrap();
            h.state_mut().run("layer.select", json!({"layer": pixels.0})).unwrap();
            h.run_steps(2);
            assert_opacity(&h, "100%");
            let value = h.query_all_by_label("100%").find(|node| (node.rect().center().y - op.center().y).abs() < 1.0).unwrap().rect();
            assert!(value.right() <= width, "{theme:?} at {width}: {value:?}");
            if let Some(dir) = &directory {
                h.render().unwrap().save(dir.join(format!("info-opacity-{}-{width}-opaque-background.png", theme.id()))).unwrap();
            }
            h.state_mut().ui.info_panel.readouts = vec![Readout::ActualColor, Readout::Cmyk];
            h.run_steps(2);
            assert!(h.query_by_label("Composite opacity:").is_none());
            if let Some(dir) = &directory {
                h.render().unwrap().save(dir.join(format!("info-opacity-{}-{width}-disabled.png", theme.id()))).unwrap();
            }
        }
    }
}

/// The opacity block is absent until explicitly enabled, and toggling it leaves the layer alone.
#[test]
fn info_panel_composite_opacity_is_optional() {
    let mut app = opacity_app(ColorMode::Rgb, SampleType::U8);
    app.ui.info_panel.readouts = vec![Readout::ActualColor, Readout::Cmyk];
    let mut h = Harness::builder().with_size(vec2(220.0, 240.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
    h.run_steps(2);
    assert!(h.query_by_label("Composite opacity:").is_none());
    for on in [true, false, true] {
        h.state_mut().ui.info_panel.readouts =
            if on { vec![Readout::ActualColor, Readout::Cmyk, Readout::Opacity] } else { vec![Readout::ActualColor, Readout::Cmyk] };
        h.run_steps(2);
        assert_eq!(h.query_by_label("Composite opacity:").is_some(), on);
        if on {
            assert_opacity(&h, "50%");
        }
        assert_eq!(h.state().session.active().unwrap().doc.layers[0].opacity, 1.0);
    }
}
