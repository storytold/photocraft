//! #11945: Mixer Brush reset defaults belong to the UI tool, not command constructors.

use egui::{RawInput, Shape};
use photocraft_color::BlendMode;
use photocraft_engine::{BrushSettings, Session};
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

fn control(app: &mut PhotocraftApp, params: Value) {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.menu.invoke", params);
    let Reply::Done(reply) = handle(app, &ctx, &req) else { panic!("reset must finish synchronously") };
    assert_eq!(reply["ok"], true, "{reply}");
    app.sync_views();
}

fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    photocraft_ui_egui::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

fn reset(app: &mut PhotocraftApp) {
    control(app, json!({"id": "tool.reset"}));
}

fn reset_all(app: &mut PhotocraftApp) {
    control(app, json!({"id": "tool.resetAll", "params": {"confirmed": true}}));
}

fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
    (b.flip_x, b.flip_y, b.pressure_size) = (true, true, true);
    b.opacity = 0.3;
    b.mode = BlendMode::Multiply;
    b.smoothing.amount = 0.6;
    let m = &mut b.mixer;
    (m.wet, m.load, m.mix, m.flow) = (0.1, 0.2, 0.3, 0.4);
    (m.load_after_stroke, m.clean_after_stroke, m.sample_all_layers) = (false, false, true);
}

fn assert_mixer_reset(app: &mut PhotocraftApp) {
    assert_eq!(tip(app), [35.0, 0.0, 0.25, 0.0, 1.0]);
    let b = &app.session.tools.brush;
    assert_eq!((b.flip_x, b.flip_y, b.pressure_size), (false, false, false));
    assert_eq!((b.opacity, b.mode, b.smoothing.amount), (1.0, BlendMode::Normal, 0.1));
    let m = &b.mixer;
    assert_eq!((m.wet, m.load, m.mix, m.flow), (0.8, 0.75, 0.9, 1.0));
    assert_eq!((m.load_after_stroke, m.clean_after_stroke, m.sample_all_layers), (true, true, false));
}

/// Check the actual painted chip label, scoped to its public response rectangle.
fn assert_chip(app: &mut PhotocraftApp, label: &str) {
    fn has_text(shape: &Shape, rect: egui::Rect, label: &str) -> bool {
        match shape {
            Shape::Text(text) => text.galley.text() == label && rect.contains(text.pos),
            Shape::Vec(shapes) => shapes.iter().any(|shape| has_text(shape, rect, label)),
            _ => false,
        }
    }
    let ctx = app.ctx().clone();
    ctx.enable_accesskit();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 300.0)));
    let mut input = RawInput { screen_rect, ..Default::default() };
    // The app's public hook supplies the framework's current FrameInput/Env.
    <PhotocraftApp as eframe::App>::raw_input_hook(app, &ctx, &mut input);
    let mut out = ctx.run_ui(input, |ui| photocraft_ui_egui::panels::options_bar(app, ui));
    // The framework owns the chip's id. Its named ComboBox bounds keep this an
    // assertion about the painted brush-size label, even beside other controls.
    let tree = craft::ui::testing::access::tree_of(&mut out);
    let fields: Vec<_> = tree
        .keys()
        .into_iter()
        .filter(|&key| tree.role_of(key) == Some(craft::ui::contract::Role::COMBO_BOX) && tree.name_of(key) == Some("Brush Preset picker"))
        .collect();
    let [field] = fields.as_slice() else { panic!("expected one named brush picker, got {}", fields.len()) };
    let rect = tree.bounds_of(*field).expect("brush picker bounds");
    assert!(out.shapes.iter().any(|shape| has_text(&shape.shape, rect, label)), "painted chip {label}");
    out.textures_delta.clear();
}

#[test]
fn reset_tool_gives_the_mixer_its_35_px_soft_tip_and_measured_options() {
    let mut app = app();
    pick(&mut app, Tool::MixerBrush);
    scramble(&mut app);
    assert_chip(&mut app, "57");
    reset(&mut app);
    assert_mixer_reset(&mut app);
    assert_chip(&mut app, "35");
}

#[test]
fn reset_all_gives_the_mixer_its_defaults_active_or_picked_later() {
    let mut active = app();
    pick(&mut active, Tool::MixerBrush);
    scramble(&mut active);
    reset_all(&mut active);
    assert_mixer_reset(&mut active);
    assert_chip(&mut active, "35");
    let mut inactive = app();
    pick(&mut inactive, Tool::MixerBrush);
    scramble(&mut inactive);
    pick(&mut inactive, Tool::Brush);
    reset_all(&mut inactive);
    pick(&mut inactive, Tool::MixerBrush);
    assert_mixer_reset(&mut inactive);
    assert_chip(&mut inactive, "35");
}

#[test]
fn mixer_reset_preserves_the_brush_and_eraser_own_resets() {
    let mut app = app();
    pick(&mut app, Tool::MixerBrush);
    reset(&mut app);
    for (tool, expected) in [(Tool::Brush, [13.0, 0.0, 0.25, 0.0, 1.0]), (Tool::Eraser, [13.0, 1.0, 0.25, 0.0, 1.0])] {
        pick(&mut app, tool);
        scramble(&mut app);
        reset(&mut app);
        assert_eq!(tip(&mut app), expected, "{tool:?} Reset Tool");
    }
    pick(&mut app, Tool::MixerBrush);
    reset_all(&mut app);
    for (tool, expected) in [(Tool::Brush, [13.0, 0.0, 0.25, 0.0, 1.0]), (Tool::Eraser, [13.0, 1.0, 0.25, 0.0, 1.0])] {
        pick(&mut app, tool);
        assert_eq!(tip(&mut app), expected, "{tool:?} Reset All");
    }
}

#[test]
fn switching_back_keeps_the_mixers_edited_brush() {
    let mut app = app();
    pick(&mut app, Tool::MixerBrush);
    reset(&mut app);
    scramble(&mut app);
    let edited = app.session.tools.brush.clone();
    pick(&mut app, Tool::Brush);
    let brush = app.session.tools.brush.clone();
    pick(&mut app, Tool::MixerBrush);
    assert_eq!(app.session.tools.brush, edited);
    pick(&mut app, Tool::Brush);
    assert_eq!(app.session.tools.brush, brush);
}

#[test]
fn mixer_reset_keeps_session_fields_and_clears_the_current_preset() {
    let mut app = app();
    pick(&mut app, Tool::MixerBrush);
    let b = &mut app.session.tools.brush;
    b.color = [0.2, 0.4, 0.6, 1.0];
    b.background = [0.8, 0.6, 0.4, 1.0];
    b.erase = true;
    b.seed = 1234;
    let fields = (b.color, b.background, b.erase, b.seed);
    app.session.tools.current_preset = Some("mixer-before-reset".into());
    reset(&mut app);
    assert_mixer_reset(&mut app);
    let b = &app.session.tools.brush;
    assert_eq!((b.color, b.background, b.erase, b.seed), fields);
    assert_eq!(app.session.tools.current_preset, None);
}

#[test]
fn constructors_and_implicit_mixer_commands_keep_global_defaults() {
    let defaults = BrushSettings::default();
    assert_eq!((defaults.size, defaults.hardness, defaults.spacing), (20.0, 1.0, 0.1));
    assert_eq!(defaults.smoothing.amount, 0.0);
    let m = defaults.mixer;
    assert_eq!((m.wet, m.load, m.mix, m.flow), (0.5, 0.5, 0.5, 1.0));
    assert_eq!((m.load_after_stroke, m.clean_after_stroke, m.sample_all_layers), (true, true, false));
    // A session starts at 10% smoothing (ToolState::default), unlike the raw brush constructor.
    let session_defaults = BrushSettings { smoothing: photocraft_paint::Smoothing { amount: 0.1, ..Default::default() }, ..BrushSettings::default() };
    assert_eq!(Session::new().tools.brush, session_defaults);
    let mut implicit = app();
    let mut explicit = app();
    for app in [&mut implicit, &mut explicit] {
        assert_eq!(app.session.tools.brush, session_defaults);
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#8090a0"})).unwrap();
    }
    let before = {
        let st = implicit.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(30, 24)
    };
    implicit.run("paint.mixerBrush", json!({"points": [[20, 24], [40, 24]], "seed": 17})).unwrap();
    explicit.run("paint.mixerBrush", json!({"points": [[20, 24], [40, 24]], "seed": 17, "wet": 50, "load": 50, "mix": 50, "flow": 100})).unwrap();
    let a = implicit.session.active().unwrap();
    let b = explicit.session.active().unwrap();
    let a = a.doc.layer(a.active_layer.unwrap()).unwrap().surface().unwrap();
    let b = b.doc.layer(b.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_ne!(a.pixel(30, 24), before, "the default-settings control actually paints");
    for y in 0..48 {
        for x in 0..64 {
            assert_eq!(a.pixel(x, y), b.pixel(x, y), "implicit mixer defaults at {x},{y}");
        }
    }
    assert_eq!(implicit.session.tools.brush, session_defaults);
}

#[test]
fn first_use_without_reset_keeps_existing_inheritance() {
    // A compatibility control, not a claim that the reference's first use was measured.
    let mut app = app();
    pick(&mut app, Tool::Blur);
    scramble(&mut app);
    let old = app.session.tools.brush.clone();
    pick(&mut app, Tool::MixerBrush);
    let expected = BrushSettings { mode: BlendMode::Normal, smoothing: photocraft_paint::Smoothing { amount: 0.1, ..Default::default() }, ..old };
    assert_eq!(app.session.tools.brush, expected);
}
