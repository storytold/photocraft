//! #11810: Reset Tool gives the Brush tool the reference application's tip, a 13 px soft round
//! (hardness 0 %, spacing 25 %): a click with the reset brush paints the soft dab. The tip values
//! themselves are checked in `reset_tip_tests.rs`.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// A control request as the control channel sends it, then the frame's sync.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = app.ctx().clone();
    let (req, rx) = ControlRequest::new(method, params);
    handle(app, &ctx, &req);
    app.sync_views();
    rx.try_recv().unwrap_or(Value::Null)
}

/// Pick `tool`, and swap its brush in as a frame does.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    photocraft_ui_egui::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// The reference's click with the reset Brush at 100,100 paints layer bounds 89,89 to 110,110:
/// the soft tip reaches past its nominal diameter. Ours reaches 89,89 to 112,112: the outermost
/// pixels are the stroke's 8-bit dither at 1/255 alpha, which the paint engine owns (not the reset
/// tip), so this allows 2 px. The default 20 px hard tip reaches 88,89 to 113,112.
#[test]
fn a_click_with_the_reset_brush_paints_the_soft_dab() {
    let mut app = app();
    pick(&mut app, Tool::Brush);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("paint.stroke", json!({"points": [[100.5, 100.5, 1.0]]})).unwrap();
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let b = surf.content_bounds();
    for (got, want) in [(b.x0, 89), (b.y0, 89), (b.x1, 110), (b.y1, 110)] {
        assert!((got - want).abs() <= 2, "bounds {b:?}, want 89,89 to 110,110");
    }
    let alpha = |x| (surf.pixel(x, 100)[3] * 255.0).round();
    assert_eq!(alpha(100), 255.0, "full at the centre");
    assert!((50.0..200.0).contains(&alpha(104)), "half-way out the tip is partly covered: {}", alpha(104));
}
