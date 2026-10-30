//! The Info panel's readouts: which rows the panel menu's choices show, for the pointer and the
//! Color Sampler points, with the extra channels read all at once.

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;
use crate::info_panel_options::{InfoPanelState, Readout};

/// A white 80×60 sRGB document with a red fill in its left half, a spot channel "Ink Red" solid
/// over the top-left quarter and an alpha channel "Alpha 1" selected over the same quarter.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    app.run("channel.newSpot", json!({"name": "Ink Red", "color": "#ff0000"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "selection"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(340.0, 600.0)).build_ui_state(|ui, app: &mut PhotocraftApp| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

/// How many labels read exactly `text`.
fn count(h: &Harness<'static, PhotocraftApp>, text: &str) -> usize {
    h.query_all_by_label(text).count()
}

/// The defaults are Photoshop's (Actual Color, CMYK): an RGB document reads RGB and CMYK rows.
#[test]
fn default_readouts_are_actual_color_and_cmyk() {
    let mut app = app();
    app.hover_doc = Some([10.0, 10.0]);
    let h = harness(app);
    for key in ["R:", "G:", "B:", "C:", "M:", "Y:", "K:"] {
        assert!(count(&h, key) >= 1, "{key} row");
    }
    assert!(h.query_by_label("Ink Red:").is_none(), "spot channels are off by default");
    assert_eq!(h.query_all_by_label("255").count(), 1, "R 255");
    assert_eq!(h.query_all_by_label("0").count(), 2, "G and B 0");
}

/// Grayscale and the channel readouts show K % and every extra channel's % at the pointer, as
/// Photoshop reads them (red = K 70 %, solid spot ink = 100 %, selected alpha = 0 %); off the
/// canvas the rows stay, with dashes, so the panel does not jump.
#[test]
fn grayscale_and_channel_readouts_read_the_pointer() {
    let mut app = app();
    app.ui.info_panel = InfoPanelState { readouts: vec![Readout::Grayscale, Readout::SpotChannels, Readout::AlphaChannels, Readout::Opacity] };
    app.hover_doc = Some([10.0, 10.0]);
    let mut h = harness(app);
    assert!(h.query_by_label("R:").is_none(), "RGB is off");
    assert!(h.query_by_label("K:").is_some());
    assert!(h.query_by_label("Ink Red:").is_some());
    assert!(h.query_by_label("Alpha 1:").is_some());
    assert!(h.query_by_label("Composite opacity:").is_some());
    assert_eq!(count(&h, "70%"), 1, "K 70 % for red");
    assert_eq!(count(&h, "100%"), 2, "spot ink 100 % and opacity 100 %");
    assert_eq!(count(&h, "0%"), 1, "alpha selected = 0 %");
    // The bottom-right quarter: white, no ink, masked alpha.
    h.state_mut().hover_doc = Some([70.0, 50.0]);
    h.run_steps(2);
    assert_eq!(count(&h, "0%"), 2, "K 0 %, ink 0 %");
    assert_eq!(count(&h, "100%"), 2, "alpha masked 100 %, opacity 100 %");
    // Off the canvas: the same rows, dashed.
    h.state_mut().hover_doc = None;
    h.run_steps(2);
    assert!(h.query_by_label("Ink Red:").is_some(), "the row stays");
    assert!(count(&h, "—") >= 4, "dashes");
}

/// Actual Color follows the document's mode: K for a grayscale document, every channel of a
/// Multichannel one.
#[test]
fn actual_color_follows_the_document_mode() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 20, "mode": "grayscale", "background": "white"})).unwrap();
    app.ui.info_panel = InfoPanelState { readouts: vec![Readout::ActualColor] };
    app.hover_doc = Some([5.0, 5.0]);
    let h = harness(app);
    assert!(h.query_by_label("K:").is_some() && h.query_by_label("R:").is_none(), "gray reads K");
    assert!(h.query_by_label("0%").is_some(), "white = K 0 %");
}

/// Every readout mode renders rows for an RGB sample, and the Color Sampler points list the same
/// readouts, channel readouts one channel per line.
#[test]
fn every_readout_and_the_samplers() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 70, "y": 50})).unwrap();
    app.ui.info_panel = InfoPanelState { readouts: Readout::ALL.to_vec() };
    app.hover_doc = Some([10.0, 10.0]);
    let h = harness(app);
    for key in ["R:", "G:", "B:", "C:", "M:", "Y:", "K:", "H:", "S:", "L:", "a:", "b:", "CMYK:", "Spot:", "Composite opacity:", "Ink Red:", "Alpha 1:"] {
        assert!(count(&h, key) >= 1, "{key} row");
    }
    assert!(count(&h, "ff") >= 1, "web hex");
    assert!(count(&h, "0°") >= 1, "hue");
    // Point 1 (red, inked) and point 2 (white, no ink).
    assert_eq!(h.query_all_by_label_contains("R:255 G:0 B:0").count(), 2, "Actual Color and RGB lines of point 1");
    assert_eq!(h.query_all_by_label_contains("R:255 G:255 B:255").count(), 2, "point 2");
    assert_eq!(h.query_all_by_label("K:70%").count(), 1, "point 1 grayscale");
    assert_eq!(h.query_all_by_label("K:0%").count(), 1, "point 2 grayscale");
    assert_eq!(h.query_all_by_label("Ink Red:100%").count(), 1);
    assert_eq!(h.query_all_by_label("Ink Red:0%").count(), 1);
    assert_eq!(h.query_all_by_label("Alpha 1:0%").count(), 1);
    assert_eq!(h.query_all_by_label("Alpha 1:100%").count(), 1);
    assert_eq!(h.query_all_by_label_contains("R:ff G:00 B:00").count(), 1, "web");
    assert_eq!(h.query_all_by_label_contains("H:0° S:100% B:100%").count(), 1, "hsb");
    assert_eq!(h.query_all_by_label_contains("H:0° S:100% L:50%").count(), 1, "hsl");
    assert_eq!(h.query_all_by_label_contains("Composite opacity:100%").count(), 2, "both points opaque");
    assert_eq!(h.query_all_by_label("Spot:100%").count(), 1, "point 1 total spot ink");
    assert_eq!(h.query_all_by_label("Spot:0%").count(), 1, "point 2 total spot ink");
}

/// No readout at all leaves only position and size.
#[test]
fn no_readouts_leaves_position_and_size() {
    let mut app = app();
    app.ui.info_panel = InfoPanelState { readouts: vec![] };
    app.hover_doc = Some([10.0, 10.0]);
    let h = harness(app);
    assert!(h.query_by_label("R:").is_none() && h.query_by_label("K:").is_none());
    assert!(h.query_by_label("X:").is_some() && h.query_by_label("W:").is_some());
}

/// Channel readouts take the full panel width: a long spot name is cut with an ellipsis and its
/// value stays inside the panel, clear of the name, while the colour readouts keep two columns.
#[test]
fn long_channel_names_never_run_under_their_value() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let long = "PANTONE 2995 C a very long spot channel name indeed";
    app.run("channel.newSpot", json!({"name": long, "color": "#00a0ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.info_panel = InfoPanelState { readouts: vec![Readout::Rgb, Readout::Cmyk, Readout::SpotChannels] };
    app.hover_doc = Some([10.0, 10.0]);
    let width = 260.0;
    let mut h = Harness::builder().with_size(vec2(width, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    let panel = h.ctx.input(|i| i.viewport_rect());
    let value = h.get_by_label("100%").rect();
    assert!(value.max.x <= panel.max.x + 0.5, "the ink value is inside the panel: {value:?} in {panel:?}");
    let name = h.get_by_label_contains("PANTONE 2995").rect();
    assert!(name.max.x <= value.min.x + 0.5, "the name ends before the value: {name:?} vs {value:?}");
    assert!(name.min.x < panel.min.x + 20.0, "the name starts at the panel's left edge, not in a column: {name:?}");
    assert!(value.min.y > h.get_by_label("R:").rect().min.y, "the channel row sits below the column rows");
    let c = h.get_by_label("C:").rect();
    assert!(c.min.x > panel.center().x - 10.0, "CMYK is still in the second column: {c:?}");
}
