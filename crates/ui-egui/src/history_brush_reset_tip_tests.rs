//! #12847: History Brush starts and resets with a 21 px soft round, 25 % spacing and no smoothing.

use photocraft_engine::BrushSettings;
use serde_json::json;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::Tool;

/// Build a small document with initialized views for the tool-state regressions.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Dispatch a shell command through the control channel and synchronize the resulting state.
fn invoke(app: &mut PhotocraftApp, id: &str, params: serde_json::Value) {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("ui.menu.invoke", json!({"id": id, "params": params}));
    let Outcome::Done(result) = handle(app, &ctx, &req) else { panic!("command did not finish") };
    assert_eq!(result["ok"], true, "{result}");
    app.sync_views();
}

/// Activate a tool and load its own brush before checking its settings.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Set deliberately nondefault options so stale values are visible after a reset or tool switch.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
    (b.opacity, b.flow, b.smoothing.amount) = (0.3, 0.4, 0.7);
    b.flip_x = true;
    b.flip_y = true;
    b.mode = photocraft_color::BlendMode::Multiply;
    b.pressure_size = true;
    b.pressure_opacity = true;
    b.build_up = true;
}

/// Check #12847's reset defaults through both the command response and live brush state.
fn assert_history_tip(app: &mut PhotocraftApp) {
    let b = app.run("brush.get", json!({})).unwrap();
    for (key, expected) in [("size", 21.0), ("hardness", 0.0), ("spacing", 0.25), ("angle", 0.0), ("roundness", 1.0), ("opacity", 1.0), ("flow", 1.0)] {
        assert_eq!(b[key].as_f64(), Some(expected), "{key}");
    }
    let brush = &app.session.tools.brush;
    assert_eq!(brush.smoothing.amount, 0.0);
    assert_eq!(brush.mode, photocraft_color::BlendMode::Normal);
    assert!(!brush.flip_x && !brush.flip_y);
    assert!(!brush.pressure_size && !brush.pressure_opacity && !brush.build_up);
    assert_eq!(brush.tip, photocraft_engine::paint::brush::TipShape::Round);
}

/// Reset Tool restores the History Brush defaults while preserving session colors and seed.
#[test]
fn reset_tool_restores_the_history_brush_tip_and_options() {
    let mut app = app();
    pick(&mut app, Tool::HistoryBrush);
    scramble(&mut app);
    let colors = ([0.2, 0.3, 0.4, 1.0], [0.6, 0.7, 0.8, 1.0]);
    app.session.tools.brush.color = colors.0;
    app.session.tools.brush.background = colors.1;
    app.session.tools.brush.seed = 42;
    invoke(&mut app, "tool.reset", json!({}));
    assert_history_tip(&mut app);
    let b = &app.session.tools.brush;
    assert_eq!((b.color, b.background, b.seed), (colors.0, colors.1, 42));
}

/// Render the options bar to check its 21 px chip label and absence of a Smoothing field.
#[test]
fn the_options_bar_draws_21_on_the_brush_chip_without_a_smoothing_field() {
    let mut app = app();
    pick(&mut app, Tool::HistoryBrush);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ProMedium);
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 300.0)));
        output = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| crate::panels::options_bar(&mut app, ui));
        output.textures_delta.clear();
    }
    let mut shapes: Vec<_> = output.shapes.iter().map(|s| &s.shape).collect();
    let mut text = Vec::new();
    while let Some(shape) = shapes.pop() {
        match shape {
            egui::Shape::Text(t) => text.push(t.galley.text()),
            egui::Shape::Vec(children) => shapes.extend(children),
            _ => {}
        }
    }
    assert!(text.contains(&"21"), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("Smoothing")), "{text:?}");
}

/// First use gets the History Brush defaults while the outgoing tool retains its edited brush.
#[test]
fn first_pick_uses_its_own_tip_instead_of_the_outgoing_brush() {
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::HistoryBrush);
    assert_history_tip(&mut app);
    pick(&mut app, Tool::Pencil);
    assert_eq!(app.session.tools.brush.size, 57.0);
    assert_eq!(app.session.tools.brush.smoothing.amount, 0.7);
}

/// Returning to a customized History Brush restores its settings instead of reapplying defaults.
#[test]
fn customized_history_brush_survives_tool_switches() {
    let mut app = app();
    pick(&mut app, Tool::HistoryBrush);
    scramble(&mut app);
    let custom = app.session.tools.brush.clone();
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::HistoryBrush);
    assert_eq!(app.session.tools.brush, custom);
}

/// Saved UI state retains the custom History Brush until an explicit Reset Tool request.
#[test]
fn saved_history_brush_is_preserved_until_explicitly_reset() {
    let mut app = app();
    pick(&mut app, Tool::HistoryBrush);
    scramble(&mut app);
    let custom = app.session.tools.brush.clone();
    pick(&mut app, Tool::Pencil);
    let saved = serde_json::to_value(&app.ui).unwrap();
    let mut restored = self::app();
    restored.ui = serde_json::from_value(saved).unwrap();
    pick(&mut restored, Tool::HistoryBrush);
    assert_eq!(restored.session.tools.brush, custom);
    invoke(&mut restored, "tool.reset", json!({}));
    assert_history_tip(&mut restored);
}

/// Reset All Tools restores the History Brush whether it is active or selected after the reset.
#[test]
fn reset_all_restores_the_history_brush_active_or_picked_later() {
    for active in [true, false] {
        let mut app = app();
        pick(&mut app, Tool::HistoryBrush);
        scramble(&mut app);
        if !active {
            pick(&mut app, Tool::Pencil);
        }
        invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
        pick(&mut app, Tool::HistoryBrush);
        assert_history_tip(&mut app);
    }
}

/// Guard the existing reset tip and smoothing defaults of every other brush-picker tool.
#[test]
fn other_tools_keep_their_reset_tips_and_smoothing() {
    for tool in Tool::ALL.iter().copied().filter(|t| *t != Tool::HistoryBrush && crate::paint_mouse::has_brush_picker(*t)) {
        let mut app = app();
        pick(&mut app, tool);
        scramble(&mut app);
        invoke(&mut app, "tool.reset", json!({}));
        let expected = match tool {
            Tool::Brush => (13.0, 0.0, 0.25),
            Tool::BackgroundEraser => (13.0, 1.0, 0.25),
            Tool::Healing => (19.0, 1.0, 0.25),
            Tool::Burn => (65.0, 0.0, 0.25),
            _ => {
                let b = BrushSettings::default();
                (b.size, b.hardness, b.spacing)
            }
        };
        let b = &app.session.tools.brush;
        assert_eq!((b.size, b.hardness, b.spacing), expected, "{tool:?}");
        assert_eq!(b.smoothing.amount, 0.1, "{tool:?}");
    }
}
