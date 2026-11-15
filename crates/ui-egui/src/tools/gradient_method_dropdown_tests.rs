//! #12004: the Gradient tool's options bar has Method (Perceptual · Linear · Classic · Smooth ·
//! Stripes, Smooth by default and after Reset Tool) after Dither, in both modes; a Classic
//! gradient drag paints with it; `paint.gradient` takes `stripes`.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;
use crate::theme::ThemeKind;

const XS: [i32; 11] = [100, 160, 220, 280, 340, 400, 460, 520, 580, 640, 700];

/// The reference's R values across a black → white drag from 100,300 to 700,300, dither off.
const TABLE: [(&str, [i32; 11]); 4] = [
    ("perceptual", [0, 1, 11, 35, 65, 98, 135, 171, 204, 233, 255]),
    ("linear", [0, 71, 108, 138, 165, 187, 207, 223, 237, 248, 255]),
    ("classic", [0, 16, 38, 65, 95, 127, 159, 189, 216, 238, 255]),
    ("smooth", [0, 2, 17, 42, 70, 99, 129, 162, 196, 228, 255]),
];

/// An 800 × 600 document (black foreground, white background) with the Gradient tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Gradient;
    app
}

fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

/// R of the active layer's pixel at `x`, 300.
fn red(app: &PhotocraftApp, x: i32) -> i32 {
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (surf.pixel(x, 300)[0] * 255.0).round() as i32
}

fn reds(app: &PhotocraftApp, xs: &[i32]) -> Vec<i32> {
    xs.iter().map(|x| red(app, *x)).collect()
}

fn close(got: &[i32], want: &[i32]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1)
}

fn x0(h: &Harness<'static, PhotocraftApp>, label: &str) -> f64 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label}")).accesskit_node().bounding_box().unwrap().x0
}

#[test]
fn method_follows_dither_in_both_modes_and_starts_at_smooth() {
    for classic in [false, true] {
        let mut a = app();
        a.ui.tool_options.gradient_classic = classic;
        assert_eq!(a.ui.tool_options.gradient_method, "smooth", "the default");
        let h = bar(a);
        assert!(x0(&h, "Dither") < x0(&h, "Method:"), "classic {classic}: after Dither");
        let shown = h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| {
            let node = n.accesskit_node();
            node.value().or(node.label()).is_some_and(|v| v.contains("Smooth"))
        });
        assert!(shown, "classic {classic}: shows Smooth");
    }
}

#[test]
fn the_list_has_the_five_methods_in_order() {
    let mut h = bar(app());
    let at = h.query_all_by_role(egui::accesskit::Role::ComboBox).min_by(|a, b| {
        let m = x0(&h, "Method:");
        (a.rect().left() as f64 - m).abs().total_cmp(&(b.rect().left() as f64 - m).abs())
    });
    let at = at.expect("the Method dropdown").rect().center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
    let ys: Vec<f32> = ["Perceptual", "Linear", "Classic", "Smooth", "Stripes"]
        .iter()
        .map(|l| {
            h.query_all_by_label(l).filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).map(|n| n.rect().center().y).fold(f32::MIN, f32::max)
        })
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "in the reference order: {ys:?}");
}

#[test]
fn reset_tool_sets_it_back_to_smooth_and_older_settings_load() {
    let mut a = app();
    a.ui.tool_options.gradient_method = "stripes".into();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut a, &ctx, "tool.reset", json!({})).unwrap();
    assert_eq!(a.ui.tool_options.gradient_method, "smooth");
    let older: crate::state::ToolOptions = serde_json::from_value(json!({"gradientDither": false})).unwrap();
    assert_eq!(older.gradient_method, "smooth", "settings saved before the field load with Smooth");
}

#[test]
fn a_classic_gradient_drag_paints_with_the_method() {
    for (method, want) in TABLE {
        let mut a = app();
        a.ui.tool_options.gradient_classic = true;
        a.ui.tool_options.gradient_dither = false;
        a.ui.tool_options.gradient_method = method.into();
        crate::canvas::tool_event(&mut a, ToolEvent::Down { x: 100.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut a, ToolEvent::Move { x: 400.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut a, ToolEvent::Move { x: 700.0, y: 300.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut a, ToolEvent::Up { x: 700.0, y: 300.0 }, egui::Modifiers::NONE);
        let got = reds(&a, &XS);
        assert!(close(&got, &want), "{method}: {got:?} vs {want:?}");
    }
}

#[test]
fn paint_gradient_takes_stripes() {
    let mut a = app();
    a.run("paint.gradient", json!({"from": [100, 300], "to": [700, 300], "method": "stripes", "dither": false})).unwrap();
    assert!(reds(&a, &[99, 100, 400, 699, 700]).iter().all(|r| *r == 0), "{:?}", reds(&a, &[99, 100, 400, 699, 700]));
    assert_eq!(red(&a, 701), 255);
    assert_eq!(red(&a, 799), 255);
    assert_eq!(photocraft_io::GradientMethod::from_name("Stripes"), Some(photocraft_io::GradientMethod::Stripes));
}

#[test]
fn an_unknown_method_keeps_straight_stops_and_never_panics() {
    let mut a = app();
    a.run("paint.gradient", json!({"from": [100, 300], "to": [700, 300], "method": "wavy", "dither": false})).unwrap();
    assert!((red(&a, 400) - 128).abs() <= 1, "straight stops: {}", red(&a, 400));
    for bad in [json!(""), json!(5), json!(null), json!(["stripes"])] {
        let _ = a.run("paint.gradient", json!({"from": [100, 300], "to": [700, 300], "method": bad}));
    }
}
