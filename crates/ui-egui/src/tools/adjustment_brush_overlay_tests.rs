//! #12520: the Adjustment Brush's Overlay box tints what the active adjustment layer's mask
//! reveals on the canvas, in magenta scaled by the mask value, and changes nothing in the document.

use craft::ui::canvas::{PixelRect, RasterSource};
use egui::{Color32, ColorImage, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::OverlayPrep;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 200 x 200 document whose active layer is an adjustment layer made by one Adjustment Brush
/// stroke across row 60 (a 100 px soft round), over a grey painted layer.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.ui.tool = Tool::AdjustmentBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.sync_views();
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 60.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 160.0, y: 60.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 160.0, y: 60.0 }, Modifiers::NONE);
    app.sync_views();
    assert!(matches!(active(&app).content, photocraft_doc::LayerContent::Adjustment(_)), "the stroke made an adjustment layer");
    app
}

fn active(app: &PhotocraftApp) -> &photocraft_doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

fn mask_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    active(app).mask.as_ref().expect("a mask").surface.sample_channel(x, y, 0)
}

fn prep(app: &mut PhotocraftApp) -> Option<OverlayPrep> {
    let ctx = app.ctx().clone();
    OverlayPrep::new(app, &ctx, 0, None, None)
}

/// What the canvas's second display draws over pixel (x, y); `None` while it draws nothing.
fn over(app: &mut PhotocraftApp, x: i32, y: i32) -> Option<Color32> {
    let prep = prep(app)?;
    let source = prep.source(app);
    let mut out = ColorImage::filled([1, 1], Color32::TRANSPARENT);
    source.read(PixelRect::new(x, y, x + 1, y + 1), 1, &mut out);
    Some(out.pixels[0])
}

#[test]
fn the_box_is_off_with_its_tooltip_and_ticking_it_changes_nothing_in_the_document() {
    let app = app();
    assert!(!app.ui.tool_options.adjustment_brush_overlay, "off by default");
    let before = app.session.active().map(|st| (st.revision, st.history.past_len())).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    h.run_steps(4);
    let tip = h.get_by_label("Overlay").accesskit_node().description().map(|d| d.to_string());
    assert_eq!(tip.as_deref(), Some("Show or hide the painted mask areas"));
    h.get_by_label("Overlay").click();
    h.run_steps(4);
    assert!(h.state().ui.tool_options.adjustment_brush_overlay, "ticked");
    let after = h.state().session.active().map(|st| (st.revision, st.history.past_len())).unwrap();
    assert_eq!(after, before, "no document change and no history state");
}

#[test]
fn ticked_it_tints_what_the_mask_reveals_in_magenta_scaled_by_the_mask_value() {
    let mut app = app();
    app.ui.tool_options.adjustment_brush_overlay = true;
    // The middle of the stroke, its soft edge, and a pixel the stroke never reached.
    let (full, edge, none) = ((100, 60), (100, 100), (100, 180));
    assert!(mask_at(&app, full.0, full.1) > mask_at(&app, edge.0, edge.1), "the stroke's middle reveals more than its edge");
    assert!(mask_at(&app, edge.0, edge.1) > 0.0 && mask_at(&app, none.0, none.1) == 0.0);
    let [full, edge, none] = [full, edge, none].map(|(x, y)| over(&mut app, x, y).expect("the tint draws"));
    for c in [full, edge] {
        assert!(c.r() == c.b() && c.g() == 0, "magenta: {c:?}");
    }
    assert!(full.a() > edge.a() && edge.a() > 0, "scaled by the mask value: {full:?} over {edge:?}");
    assert!(full.a() < 255, "the picture still shows through: {full:?}");
    assert_eq!(none, Color32::TRANSPARENT, "nothing where the mask hides");
}

#[test]
fn unticked_another_layer_kind_or_another_tool_shows_no_tint() {
    let mut app = app();
    assert!(prep(&mut app).is_none(), "unticked: the plain composite, nothing over it");
    app.ui.tool_options.adjustment_brush_overlay = true;
    assert!(prep(&mut app).is_some(), "ticked");

    app.ui.tool = Tool::Brush;
    assert!(prep(&mut app).is_none(), "another tool");
    app.ui.tool = Tool::AdjustmentBrush;

    let doc = &app.session.active().unwrap().doc;
    let pixel = doc.walk().into_iter().find(|(.., l)| matches!(l.content, photocraft_doc::LayerContent::Raster(_))).map(|(.., l)| l.id.0).unwrap();
    app.run("layer.select", json!({"layer": pixel})).unwrap();
    assert!(!matches!(active(&app).content, photocraft_doc::LayerContent::Adjustment(_)));
    assert!(prep(&mut app).is_none(), "a pixel layer active");
}

#[test]
fn the_tint_shows_over_a_channel_view_and_ticking_it_redraws_that_view() {
    let mut app = app();
    // Quick Mask over everything right of x = 100: a red rubylith there.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap();
    app.run("select.editInQuickMaskMode", json!({})).unwrap();
    let untinted = prep(&mut app).expect("the Quick Mask view").revision;
    let ruby = over(&mut app, 150, 60).unwrap();
    assert!(ruby.r() > 0 && ruby.b() == 0, "the rubylith alone: {ruby:?}");

    app.ui.tool_options.adjustment_brush_overlay = true;
    let tinted = prep(&mut app).unwrap();
    assert_ne!(tinted.revision, untinted);
    assert_eq!(tinted.source(&app).damage(untinted), None, "a texture of the untinted view is redrawn whole");
    let both = over(&mut app, 150, 60).unwrap();
    assert!(both.b() > 0 && both.r() > both.b(), "the tint over the rubylith: {both:?}");
    assert_eq!(over(&mut app, 150, 180).unwrap(), ruby, "only the rubylith where the mask hides");
}
