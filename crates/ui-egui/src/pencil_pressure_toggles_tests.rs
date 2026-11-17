//! #11962: the Pencil's options bar has the two pressure toggles, editing the Pencil's own brush;
//! the Brush keeps its own. Every theme shows the same toggles.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::AppTheme;

const OPACITY: &str = "Pressure controls opacity";
const SIZE: &str = "Pressure controls size";

fn bar(theme: AppTheme) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.ui.tool = Tool::Pencil;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

#[test]
fn the_toggles_edit_the_pencils_own_flags_and_brush_get_sees_them() {
    let mut h = bar(AppTheme::pro_medium());
    let flags = |h: &Harness<'static, PhotocraftApp>| (h.state().session.tools.brush.pressure_opacity, h.state().session.tools.brush.pressure_size);
    assert_eq!(flags(&h), (false, false), "both off on a fresh Pencil");
    h.get_by_label(OPACITY).click();
    h.run_steps(2);
    h.get_by_label(SIZE).click();
    h.run_steps(2);
    assert_eq!(flags(&h), (true, true));
    let got = h.state_mut().run("brush.get", json!({})).unwrap();
    assert_eq!(got["pressureOpacity"], json!(true), "brush.get reads the Pencil's flag");
    assert_eq!(got["pressureSize"], json!(true));
}

#[test]
fn the_brush_keeps_its_own_pressure_flags() {
    let mut h = bar(AppTheme::pro_medium());
    let flags = |h: &Harness<'static, PhotocraftApp>| (h.state().session.tools.brush.pressure_opacity, h.state().session.tools.brush.pressure_size);
    let toggle = |h: &mut Harness<'static, PhotocraftApp>, label: &str| {
        h.get_by_label(label).click();
        h.run_steps(2);
    };

    // Give the Brush its own brush first, so the Pencil's later toggles cannot be "inherited" on
    // the first switch and hide a leak.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    h.state_mut().session.tools.brush.pressure_opacity = false;
    h.state_mut().session.tools.brush.pressure_size = false;
    h.run_steps(2);

    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    toggle(&mut h, OPACITY);
    toggle(&mut h, SIZE);
    assert_eq!(flags(&h), (true, true), "the Pencil's flags are on");

    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (false, false), "the Pencil's toggles did not change the Brush");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Pencil's flags came back");

    // The mirror: the Brush on, the Pencil off, so a reset to one value cannot pass.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    h.state_mut().session.tools.brush.pressure_opacity = true;
    h.state_mut().session.tools.brush.pressure_size = true;
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    toggle(&mut h, OPACITY);
    toggle(&mut h, SIZE);
    assert_eq!(flags(&h), (false, false), "the Pencil is off");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Brush stayed on");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (false, false), "the Pencil stayed off");

    // A mixed setting: opacity on, size off, across a switch and back.
    toggle(&mut h, OPACITY);
    assert_eq!(flags(&h), (true, false), "mixed: opacity on, size off");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Brush still has its own pair");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, false), "the mixed pair came back unchanged");
    // And the bar draws that pair, not just stores it.
    // The toggle button, not the tooltip of the same name the pointer resting on the opacity toggle shows. An off
    // toggle button reports no toggled state.
    let on = |h: &Harness<'static, PhotocraftApp>, label: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    };
    assert!(on(&h, OPACITY), "the opacity toggle reads as on");
    assert!(!on(&h, SIZE), "the size toggle reads as off");
}

/// Painted width at column `x` on the active layer, through the stroke's row band.
fn width(app: &PhotocraftApp, x: i32, y: i32) -> usize {
    let st = app.session.active().expect("a document");
    let layer = st.active_layer.expect("an active layer");
    let s = st.doc.layer(layer).and_then(|l| l.surface()).expect("a raster layer");
    (y - 20..=y + 20).filter(|yy| s.rgba(x, *yy)[3] > 0.5).count()
}

#[test]
fn pressure_for_size_narrows_a_low_pressure_pencil_stroke() {
    let mut h = bar(AppTheme::pro_medium());
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 13.0, "pressureSize": true, "smoothing": {"amount": 0.0}}})).unwrap();
    app.run("paint.pencil", json!({"points": [[20.0, 50.0, 0.2], [30.0, 50.0, 0.2], [60.0, 50.0, 1.0]]})).unwrap();
    let (low, high) = (width(app, 22, 50), width(app, 58, 50));
    assert!(low < high, "pressure for size on: low-pressure {low} px, high-pressure {high} px");
    assert!((2..=4).contains(&low), "pressure for size on: the 0.2 stroke is about 3 px, got {low}");

    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"pressureSize": false}})).unwrap();
    app.run("paint.pencil", json!({"points": [[20.0, 50.0, 0.2], [30.0, 50.0, 0.2], [60.0, 50.0, 1.0]]})).unwrap();
    let (low, high) = (width(app, 22, 50), width(app, 58, 50));
    assert_eq!((low, high), (13, 13), "pressure for size off: every dab is 13 px");
}
