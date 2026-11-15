//! CodeRabbit on #15018: the Transparency checkbox's stored value reaches a drag only while the
//! checkbox is shown (Classic gradient, no gradient fill layer selected), and the checkbox follows
//! the effective mode.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;
use crate::theme::ThemeKind;

/// An 800 × 600 document with the Gradient tool, a white foreground and Foreground to Transparent.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ffffff"})).unwrap();
    app.run("gradient.presets.select", json!({"preset": "Foreground to Transparent"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Gradient;
    app
}

/// Drag across the alpha channel (a target the live gradient never takes), then the channel's value at x.
fn channel_after_drag(classic: bool, transparency: bool, x: i32) -> f32 {
    let mut a = app();
    a.run("channel.new", json!({"name": "Alpha"})).unwrap();
    a.run("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    a.ui.tool_options.gradient_classic = classic;
    a.ui.tool_options.gradient_transparency = transparency;
    a.ui.tool_options.gradient_dither = false;
    crate::canvas::tool_event(&mut a, ToolEvent::Down { x: 100.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(&mut a, ToolEvent::Move { x: 400.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(&mut a, ToolEvent::Move { x: 700.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(&mut a, ToolEvent::Up { x: 700.0, y: 300.0 }, egui::Modifiers::NONE);
    let st = a.session.active().unwrap();
    st.doc.channels[0].surface.pixel(x, 300)[0]
}

#[test]
fn a_stored_off_value_is_ignored_outside_classic_mode() {
    // Live mode chosen, the alpha channel as target: the drag takes the classic path, but the
    // checkbox is hidden, so the gradient fades as it does by default.
    let v = channel_after_drag(false, false, 175);
    assert!((v - 0.874).abs() < 0.03, "fades: {v}");
}

#[test]
fn classic_mode_still_honours_the_checkbox() {
    assert!(channel_after_drag(true, false, 175) > 0.98, "off: opaque");
    assert!((channel_after_drag(true, true, 175) - 0.874).abs() < 0.03, "on: fades");
}

fn shows_transparency(app: PhotocraftApp) -> bool {
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h.query_all_by_label("Transparency").next().is_some()
}

#[test]
fn the_checkbox_follows_the_effective_mode() {
    let mut a = app();
    a.ui.tool_options.gradient_classic = true;
    assert!(shows_transparency(a), "Classic on a pixel layer");
    let mut a = app();
    a.ui.tool_options.gradient_classic = false;
    assert!(!shows_transparency(a), "Live mode");
    // A gradient fill layer is edited live whatever the stored choice: the bar reads Gradient.
    let mut a = app();
    a.run("gradient.fill.create", json!({"from": [0, 0], "to": [100, 0]})).unwrap();
    a.ui.tool_options.gradient_classic = true;
    assert!(!shows_transparency(a), "a fill layer is live, so no Transparency");
}
