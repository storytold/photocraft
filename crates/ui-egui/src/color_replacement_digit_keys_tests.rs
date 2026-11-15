//! #11927: Color Replacement digits set its Tolerance, while Shift/Alt remain no-ops.

use serde_json::json;

use crate::PhotocraftApp;
use crate::opacity_keys::press;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

/// Send one key event through the digit handler.
fn alt_digit(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let modifiers = egui::Modifiers::ALT;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::opacity_keys::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

#[test]
fn digits_set_color_replacement_tolerance_and_quick_pairs_are_exact() {
    let mut a = app(Tool::ColorReplacement);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0);
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 50.0);
    press(&mut a, 0, false, 5000.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 100.0);
    press(&mut a, 1, false, 10000.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 10.0);
    press(&mut a, 4, false, 20000.0);
    press(&mut a, 5, false, 20300.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 45.0);
    let options = serde_json::to_value(&a.ui.tool_options).unwrap();
    assert_eq!(options["cr_tolerance"], json!(45.0));
}

#[test]
fn modifiers_and_double_zero_preserve_color_replacement_and_layer_state() {
    let mut a = app(Tool::ColorReplacement);
    let before_layer = layer(&a);
    press(&mut a, 5, true, 0.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0, "Shift+digit is unchanged");
    alt_digit(&mut a, egui::Key::Num5);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0, "Alt+digit is unchanged");
    press(&mut a, 0, false, 5000.0);
    press(&mut a, 0, false, 5100.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 100.0, "unmeasured 00 keeps the first zero's value");
    assert_eq!(layer(&a), before_layer);
    let brush = a.session.tools.brush.clone();
    let opacity = a.ui.tool_options.fill_opacity;
    let bg_tolerance = a.ui.tool_options.bg_tolerance;
    a.ui.tool = Tool::ColorReplacement;
    press(&mut a, 3, false, 9000.0);
    assert_eq!(a.session.tools.brush, brush);
    assert_eq!(a.ui.tool_options.fill_opacity, opacity);
    assert_eq!(a.ui.tool_options.bg_tolerance, bg_tolerance);
}
