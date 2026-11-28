//! #12560: Reset Tool gives the Spot Healing Brush the reference application's tip, a 19 px hard
//! round at 25 % spacing (the Healing Brush's), not the 20 px default at 10 %; Reset All Tools and a
//! first use do too, and the tool then keeps its own tip. The Healing Brush, the Clone Stamp and
//! the Brush keep theirs.

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

fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// A tip that is neither default nor the Spot Healing Brush's, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
}

const SPOT_HEALING_TIP: [f64; 5] = [19.0, 1.0, 0.25, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_spot_healing_brush_a_19_px_hard_tip() {
    let mut app = app();
    pick(&mut app, Tool::SpotHealing);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP);
    // The options-bar brush chip paints the live brush's `size.round()` as plain text (no
    // accessibility label carries it), so this is the number it reads.
    assert_eq!(app.session.tools.brush.size.round(), 19.0);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::SpotHealing);
    scramble(&mut app);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::SpotHealing);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
    pick(&mut app, Tool::SpotHealing);
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP, "picked after the reset");
}

#[test]
fn a_first_use_starts_with_the_tip_and_then_keeps_its_own() {
    // No stored tool brushes, as on a fresh install: the Spot Healing Brush's first pick, after
    // another tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::SpotHealing);
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP);
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::SpotHealing);
    assert_eq!(tip(&mut app)[0], 42.0, "kept across tool switches");
}

#[test]
fn a_first_use_keeps_the_current_brushs_other_settings() {
    // Only the tip was measured. Settings that shape the heal stay the current brush's, as before:
    // Pressure for Size and a Dual Brush, which keeps its ratio to the tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    let b = &mut app.session.tools.brush;
    b.pressure_size = true;
    (b.dual_brush.enabled, b.dual_brush.size) = (true, 114.0);
    b.spacing_enabled = false;
    pick(&mut app, Tool::SpotHealing);
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP);
    let b = &app.session.tools.brush;
    assert!(b.pressure_size, "Pressure for Size kept");
    assert!((b.dual_brush.size - 38.0).abs() < 1e-3, "dual brush 114 px at 57 px scales to 38 px at 19 px: {}", b.dual_brush.size);
    // The measured 25 % spacing only takes effect with Spacing on.
    assert!(b.spacing_enabled);
}

#[test]
fn a_first_use_after_a_sampled_tip_starts_round() {
    use photocraft_engine::paint::{GrayTile, TipShape};
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    app.session.tools.brush.tip = TipShape::Sampled(GrayTile::from_fn(8, 6, |x, y| ((x + y) % 2) as f32));
    pick(&mut app, Tool::SpotHealing);
    assert!(matches!(app.session.tools.brush.tip, TipShape::Round), "a round tip, not the sampled one");
    assert_eq!(tip(&mut app), SPOT_HEALING_TIP);
}

#[test]
fn the_healing_brush_clone_stamp_and_brush_keep_their_reset_tips() {
    let cases = [(Tool::Healing, [19.0, 1.0, 0.25, 0.0, 1.0]), (Tool::CloneStamp, [21.0, 0.0, 0.25, 0.0, 1.0]), (Tool::Brush, [13.0, 0.0, 0.25, 0.0, 1.0])];
    for (tool, expected) in cases {
        let mut app = app();
        pick(&mut app, tool);
        scramble(&mut app);
        control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
        assert_eq!(tip(&mut app), expected, "{tool:?}");
    }
}
