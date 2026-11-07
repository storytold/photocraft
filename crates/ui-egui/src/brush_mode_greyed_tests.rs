//! #11815: the Brush and Pencil Mode list greys what the active layer and document can't paint
//! with, as in the reference application: Behind and Clear on the Background, 14 entries in
//! 32-bit RGB, all but Threshold and Dissolve in Indexed, all but Threshold, Dissolve, Darken and
//! Lighten in Bitmap; there the first entry reads Threshold. Clicks, the wheel and ⇧−/⇧+ skip the
//! greyed entries, and `ui.inspect` reports them.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_mode_menu::{Item, PaintModes, choices, enabled};
use crate::state::Tool;

fn app(new: serde_json::Value, layer: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", new).unwrap();
    if layer {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    app.ui.tool = Tool::Brush;
    app
}

/// The labels of the greyed options-bar choices, in list order.
fn greyed(app: &PhotocraftApp) -> Vec<&'static str> {
    let modes = PaintModes::of(app);
    choices().into_iter().filter(|(c, _)| modes.disabled.contains(c)).map(|(_, l)| l).collect()
}

#[test]
fn behind_and_clear_are_greyed_on_the_background_only() {
    let mut a = app(json!({"width": 40, "height": 30}), false);
    assert_eq!(greyed(&a), ["Behind", "Clear"]);
    a.run("layer.new.layer", json!({})).unwrap();
    assert!(greyed(&a).is_empty(), "a normal layer can take every mode");
}

#[test]
fn rgb_32_greys_the_fourteen_measured_entries() {
    let a = app(json!({"width": 40, "height": 30, "depth": 32}), true);
    assert_eq!(
        greyed(&a),
        [
            "Behind",
            "Clear",
            "Color Burn",
            "Linear Burn",
            "Screen",
            "Color Dodge",
            "Overlay",
            "Soft Light",
            "Hard Light",
            "Vivid Light",
            "Linear Light",
            "Pin Light",
            "Hard Mix",
            "Exclusion"
        ]
    );
}

#[test]
fn other_documents_keep_every_entry() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("cmyk", 8), ("lab", 8), ("grayscale", 8), ("grayscale", 16)] {
        let a = app(json!({"width": 40, "height": 30, "mode": mode, "depth": depth}), true);
        assert!(greyed(&a).is_empty(), "{mode} {depth}: {:?}", greyed(&a));
        assert!(!PaintModes::of(&a).threshold);
    }
}

#[test]
fn indexed_and_bitmap_keep_threshold_and_a_few_modes() {
    let enabled_labels = |a: &PhotocraftApp| {
        let modes = PaintModes::of(a);
        choices().into_iter().filter(|(c, _)| !modes.disabled.contains(c)).map(|(_, l)| l).collect::<Vec<_>>()
    };
    let mut a = app(json!({"width": 40, "height": 30}), false);
    a.run("image.mode.indexedColor", json!({})).unwrap();
    assert!(PaintModes::of(&a).threshold);
    assert_eq!(enabled_labels(&a), ["Normal", "Dissolve"], "Normal is the Threshold row");

    let mut a = app(json!({"width": 40, "height": 30}), false);
    a.run("image.mode.grayscale", json!({})).unwrap();
    a.run("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert!(PaintModes::of(&a).threshold);
    assert_eq!(enabled_labels(&a), ["Normal", "Dissolve", "Darken", "Lighten"]);
}

#[test]
fn the_canvas_menu_and_inspect_share_the_rule() {
    let a = app(json!({"width": 40, "height": 30, "depth": 32}), true);
    assert!(!enabled(&a, Item::Blend(BlendMode::Screen)) && enabled(&a, Item::Blend(BlendMode::Multiply)));
    // Other tools' lists are unchanged.
    let mut g = app(json!({"width": 40, "height": 30, "depth": 32}), true);
    g.ui.tool = Tool::Gradient;
    assert!(enabled(&g, Item::Blend(BlendMode::Screen)));
}

#[test]
fn shift_plus_and_minus_skip_greyed_modes() {
    let mut a = app(json!({"width": 40, "height": 30}), false);
    a.run("tools.setBrush", json!({"brush": {"mode": "Dissolve"}})).unwrap();
    crate::blend_keys::press(&mut a, 1);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Darken, "Clear is skipped on the Background");
    assert!(!a.ui.tool_options.brush_clear);
    crate::blend_keys::press(&mut a, -1);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Dissolve);
}

fn bar(a: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), a);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

#[test]
fn the_open_list_greys_rows_and_ignores_clicks_on_them() {
    let mut h = bar(app(json!({"width": 40, "height": 30}), false));
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run_steps(3);
    let row = |h: &Harness<'static, PhotocraftApp>, l: &str| {
        h.query_all_by_label(l).find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).map(|n| n.accesskit_node().is_disabled())
    };
    assert_eq!(row(&h, "Clear"), Some(true));
    assert_eq!(row(&h, "Behind"), Some(true));
    assert_eq!(row(&h, "Multiply"), Some(false));
    h.query_all_by_label("Clear").find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.brush_clear, "a greyed row can't be picked");
}

#[test]
fn the_wheel_over_the_dropdown_skips_greyed_rows() {
    let mut a = app(json!({"width": 40, "height": 30}), false);
    a.run("tools.setBrush", json!({"brush": {"mode": "Dissolve"}})).unwrap();
    let mut h = bar(a);
    let at = h.get_by_role(egui::accesskit::Role::ComboBox).rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -1.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Darken);
    assert!(!h.state().ui.tool_options.brush_clear && !h.state().session.tools.brush.behind);
}

#[test]
fn indexed_names_the_first_entry_threshold() {
    let mut a = app(json!({"width": 40, "height": 30}), false);
    a.run("image.mode.indexedColor", json!({})).unwrap();
    let h = bar(a);
    let combo = h.get_by_role(egui::accesskit::Role::ComboBox);
    let shown = combo.accesskit_node().value().map(|v| v.to_string()).unwrap_or_default();
    assert!(shown.contains("Threshold") || h.query_by_label("Threshold").is_some(), "{shown}");
}
