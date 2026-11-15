//! #11999: Magic Eraser digits set its Opacity; Shift digits set the same field.

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

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

#[test]
fn digits_and_shift_digits_set_magic_eraser_opacity() {
    let mut a = app(Tool::MagicEraser);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0);
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 50.0);
    press(&mut a, 0, false, 5000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0);
    press(&mut a, 3, false, 10000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 30.0);
    press(&mut a, 4, false, 20000.0);
    press(&mut a, 5, false, 20300.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 45.0);
    let options = serde_json::to_value(&a.ui.tool_options).unwrap();
    assert_eq!(options["magic_eraser_opacity"], json!(45.0));
    press(&mut a, 5, true, 30000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 50.0);
}

#[test]
fn magic_eraser_digits_leave_tolerance_layer_and_brush_alone() {
    let mut a = app(Tool::MagicEraser);
    let tolerance = a.ui.tool_options.tolerance;
    let layer_before = layer(&a);
    let brush = a.session.tools.brush.clone();
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.tolerance, tolerance);
    assert_eq!(layer(&a), layer_before);
    assert_eq!(a.session.tools.brush, brush);
    press(&mut a, 0, false, 5000.0);
    press(&mut a, 0, false, 5100.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0, "unmeasured 00 keeps the first zero's value");
}
#[test]
fn digits_do_not_pair_across_tools() {
    let mut brush_to_magic = app(Tool::Brush);
    press(&mut brush_to_magic, 4, false, 0.0);
    brush_to_magic.ui.tool = Tool::MagicEraser;
    press(&mut brush_to_magic, 5, false, 300.0);
    assert_eq!(brush_to_magic.session.tools.brush.opacity, 0.4);
    assert_eq!(brush_to_magic.ui.tool_options.magic_eraser_opacity, 50.0);

    let mut magic_to_brush = app(Tool::MagicEraser);
    press(&mut magic_to_brush, 4, false, 0.0);
    magic_to_brush.ui.tool = Tool::Brush;
    press(&mut magic_to_brush, 5, false, 300.0);
    assert_eq!(magic_to_brush.ui.tool_options.magic_eraser_opacity, 40.0);
    assert_eq!(magic_to_brush.session.tools.brush.opacity, 0.5);
}
