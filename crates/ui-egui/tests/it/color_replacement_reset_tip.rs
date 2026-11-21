//! #11932: Reset Tool gives the Color Replacement tool the reference application's tip, a 13 px
//! hard round at 25 % spacing, not the 20 px default at 10 %; Reset All Tools does too. Its first
//! use, the Pencil's and the Eraser's reset tips and commands without brush params keep theirs.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// A control request as the control channel sends it, then the frame's sync.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
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

/// Size, hardness, spacing, angle and roundness of the live brush, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// A tip that is neither default nor the Color Replacement tool's, so a reset has something to
/// undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
    b.pressure_size = true;
}

const COLOR_REPLACEMENT_TIP: [f64; 5] = [13.0, 1.0, 0.25, 0.0, 1.0];
const PENCIL_TIP: [f64; 5] = [1.0, 1.0, 0.25, 0.0, 1.0];
/// The Eraser's own reset tip (#11936).
const ERASER_TIP: [f64; 5] = [13.0, 1.0, 0.25, 0.0, 1.0];

#[test]
fn reset_tool_gives_color_replacement_a_13_px_hard_tip() {
    let mut app = app();
    pick(&mut app, Tool::ColorReplacement);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), COLOR_REPLACEMENT_TIP);
    assert!(!app.session.tools.brush.pressure_size, "Pressure for Size off, as Reset Tool leaves it");
    // The options-bar brush chip paints the live brush's `size.round()` as plain text (no
    // accessibility label carries it), so this is the number it reads.
    assert_eq!(app.session.tools.brush.size.round(), 13.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::ColorReplacement);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), COLOR_REPLACEMENT_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::ColorReplacement);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    pick(&mut app, Tool::ColorReplacement);
    assert_eq!(tip(&mut app), COLOR_REPLACEMENT_TIP, "picked after the reset");
}

#[test]
fn a_first_use_still_takes_the_current_tip() {
    // Not measured in the reference, so it keeps today's behaviour: the first pick takes over the
    // live brush.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::ColorReplacement);
    assert_eq!(tip(&mut app), [57.0, 0.4, 0.6, 30.0, 0.5]);
}

#[test]
fn other_tools_keep_their_reset_tips() {
    for (tool, expected) in [(Tool::Pencil, PENCIL_TIP), (Tool::Eraser, ERASER_TIP)] {
        let mut app = app();
        pick(&mut app, tool);
        scramble(&mut app);
        control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut app), expected, "{tool:?}");
    }
    // Tools without a measured tip of their own (some may get one later) don't get Color
    // Replacement's: a match arm widened by mistake would give it to them.
    for tool in [Tool::MixerBrush, Tool::Blur, Tool::Sharpen] {
        let mut app = app();
        pick(&mut app, tool);
        scramble(&mut app);
        control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_ne!(tip(&mut app), COLOR_REPLACEMENT_TIP, "{tool:?}");
    }
}

/// The layer's pixels after a short Color Replacement stroke over red, with `extra` params.
fn replaced(extra: Value) -> Vec<f32> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    let mut p = json!({"points": [[20, 50], [80, 50]]});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("paint.colorReplacement", p).unwrap();
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn commands_without_brush_params_keep_the_default_brush() {
    let default = replaced(json!({"brush": {"size": 20, "hardness": 1.0, "spacing": 0.1}}));
    assert!(replaced(json!({})) == default, "no brush params paint with the 20 px default");
    // The pixels tell Color Replacement's reset tip apart, so the comparison would catch it.
    assert!(replaced(json!({"brush": {"size": 13, "hardness": 1.0, "spacing": 0.25}})) != default);
}
