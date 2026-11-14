//! #11963: the Pencil's options bar has the Smoothing Options gear right after its Smoothing
//! field, with Pulled String Mode, Stroke Catch-up, Catch-up on Stroke End and Adjust for Zoom on
//! the Pencil's own brush, greyed at 0 % Smoothing.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

const GEAR: &str = "Set additional smoothing options";
const OPTIONS: [&str; 4] = ["Pulled String Mode", "Stroke Catch-up", "Catch-up on Stroke End", "Adjust for Zoom"];

fn bar(theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

fn x0(h: &Harness<'static, PhotocraftApp>, label: &str) -> f64 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label}")).accesskit_node().bounding_box().expect(label).x0
}

fn open_gear(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_label(GEAR).click();
    h.run_steps(3);
}

fn smoothing(app: &mut PhotocraftApp) -> serde_json::Value {
    app.run("brush.get", json!({})).unwrap()["smoothing"].clone()
}

#[test]
fn the_gear_sits_after_smoothing_and_before_auto_erase() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let h = bar(theme);
        let gear = x0(&h, GEAR);
        // After the Smoothing label and its field, before Auto Erase.
        let smoothing = h.query_all_by_label("Smoothing").map(|n| n.accesskit_node().bounding_box().unwrap().x1).fold(0.0, f64::max);
        assert!(smoothing <= gear && gear < x0(&h, "Auto Erase"), "{theme:?}: Smoothing ({smoothing}) < gear ({gear}) < Auto Erase");
    }
}

#[test]
fn it_opens_the_four_options_in_order_with_the_reset_values() {
    let mut h = bar(ThemeKind::ProMedium);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    open_gear(&mut h);
    assert!(h.query_by_label("Smoothing Options").is_some(), "the popup's title");
    let ys: Vec<f64> = OPTIONS.iter().map(|o| h.get_by_label(o).accesskit_node().bounding_box().unwrap().y0).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "in the reference order: {ys:?}");
    let s = h.state().session.tools.brush.smoothing.clone();
    assert_eq!((s.amount, s.pulled_string, s.catch_up, s.catch_up_on_end, s.adjust_for_zoom), (0.1, false, true, false, true), "after Reset Tool");
    for o in OPTIONS {
        assert!(!h.get_by_label(o).accesskit_node().is_disabled(), "{o} enabled at 10 %");
    }
}

#[test]
fn the_options_are_the_pencils_own() {
    let mut h = bar(ThemeKind::ProMedium);
    open_gear(&mut h);
    let before = h.state().session.tools.brush.smoothing.pulled_string;
    h.get_by_label("Pulled String Mode").click();
    h.run_steps(2);
    assert_eq!(smoothing(h.state_mut())["pulledString"], json!(!before), "brush.get reads the Pencil's");
    // The Brush keeps its own; back on the Pencil its value is there again.
    h.state_mut().ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(h.state_mut());
    assert_eq!(h.state().session.tools.brush.smoothing.pulled_string, before, "the Brush's unchanged");
    h.state_mut().ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(h.state_mut());
    assert_eq!(h.state().session.tools.brush.smoothing.pulled_string, !before, "the Pencil's back");
}

#[test]
fn at_zero_smoothing_the_four_are_greyed() {
    let mut h = bar(ThemeKind::ProMedium);
    h.state_mut().session.tools.brush.smoothing.amount = 0.0;
    h.run_steps(2);
    open_gear(&mut h);
    for o in OPTIONS {
        assert!(h.get_by_label(o).accesskit_node().is_disabled(), "{o} greyed at 0 %");
    }
}

#[test]
fn the_non_pro_layout_has_the_gear_too() {
    let mut h = bar(ThemeKind::Classic);
    open_gear(&mut h);
    for o in OPTIONS {
        assert!(h.query_by_label(o).is_some(), "{o}");
    }
}
