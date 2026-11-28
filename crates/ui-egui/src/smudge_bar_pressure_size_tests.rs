//! Smudge Pressure for Size (#12961): the bar toggle binds the same brush used by paint.smudge.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::BrushSettings;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const PRESSURE: &str = "Always use pressure for size";
const TOOLTIP: &str = "Always use Pressure for Size. When off, Brush Preset controls pressure";

fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h =
        Harness::builder().with_size(vec2(1600.0, 400.0)).with_context(app.ctx().clone()).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

fn toggle(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_by_label(PRESSURE).click();
    h.run_steps(3);
}

fn brush_commands(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.journal.iter().filter(|(id, _)| id == "tools.setBrush").count()
}

#[test]
fn pressure_is_off_by_default_and_follows_finger_painting_with_the_exact_tooltip() {
    let h = bar(Tool::Smudge);
    let pressure = h.get_by_label(PRESSURE);
    assert!(h.get_by_label("Finger Painting").rect().right() <= pressure.rect().left());
    assert_eq!(pressure.accesskit_node().description().as_deref(), Some(TOOLTIP));
    assert!(!h.state().session.tools.brush.pressure_size);
}

#[test]
fn the_toggle_updates_the_brush_settings_value_with_one_command_per_click() {
    let mut h = bar(Tool::Smudge);
    let n = brush_commands(&h);
    toggle(&mut h);
    assert!(h.state().session.tools.brush.pressure_size);
    assert_eq!(h.state_mut().run("brush.get", json!({})).unwrap()["pressureSize"], json!(true));
    assert_eq!(brush_commands(&h), n + 1);
    assert_eq!(h.state().session.journal.last().unwrap().1["brush"], json!({"pressureSize": true}));
    toggle(&mut h);
    assert!(!h.state().session.tools.brush.pressure_size);
    assert_eq!(brush_commands(&h), n + 2);
    assert_eq!(h.state().session.journal.last().unwrap().1["brush"], json!({"pressureSize": false}));
}

#[test]
fn reset_tool_turns_pressure_off_and_other_tools_keep_their_own_setting() {
    let mut h = bar(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"pressureSize": true}})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Smudge;
    h.run_steps(3);
    assert!(!h.state().session.tools.brush.pressure_size, "Smudge starts with its own default");
    toggle(&mut h);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert!(h.state().session.tools.brush.pressure_size, "the Brush setting is retained");
    h.state_mut().ui.tool = Tool::Smudge;
    h.run_steps(3);
    assert!(h.state().session.tools.brush.pressure_size, "Smudge remembers its value");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    assert!(!h.state().session.tools.brush.pressure_size);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert!(h.state().session.tools.brush.pressure_size, "resetting Smudge does not reset the Brush");
}

/// Read the actual shared generator's dabs for a pen tap, rather than duplicating its formula.
fn radii(brush: &BrushSettings, pressure: f32) -> Vec<f32> {
    let mut generator = photocraft_paint::dynamics::DabGenerator::new(brush, 1.0);
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    generator.push(&[photocraft_paint::StrokePoint::new(28.0, 40.0, pressure)], &mut dabs, &mut dual);
    generator.finish(&mut dabs, &mut dual);
    assert!(!dabs.is_empty());
    dabs.into_iter().map(|dab| dab.radius).collect()
}

#[test]
fn the_ui_toggle_changes_generated_dab_size_with_pressure_and_off_restores_the_baseline() {
    let mut h = bar(Tool::Smudge);
    let original = h.state().session.tools.brush.clone();
    let off_low = radii(&original, 0.25);
    let off_full = radii(&original, 1.0);
    assert_eq!(off_low, off_full);
    toggle(&mut h);
    let on = h.state().session.tools.brush.clone();
    assert_eq!(radii(&on, 1.0), off_full);
    let low = radii(&on, 0.25);
    assert!(low.iter().all(|r| (*r - original.size * 0.25 * 0.5).abs() < 1e-6));
    toggle(&mut h);
    assert_eq!(radii(&h.state().session.tools.brush, 0.25), off_low);
    assert_eq!(h.state().session.tools.brush.shape_dynamics, original.shape_dynamics, "the toggle does not rewrite preset dynamics");
}

/// A red/white edge and a real paint.smudge command, with the UI-selected brush and real point
/// pressures. All comparisons use complete resulting pixels, with no visual threshold.
fn smudged(brush: &BrushSettings, pressure: f32) -> (Vec<f32>, Vec<f32>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80, "depth": 32, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 80})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Smudge;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": brush})).unwrap();
    app.ui.tool_options.strength = 100.0;
    let read = |app: &PhotocraftApp| {
        let state = app.session.active().unwrap();
        state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().read_region(state.doc.bounds())
    };
    let before = read(&app);
    let (command, mut params) = super::dab_params(&app, Tool::Smudge).unwrap();
    assert_eq!(command, "paint.smudge");
    params["points"] = json!([[28.0, 40.0, pressure], [42.0, 40.0, pressure], [58.0, 40.0, pressure]]);
    app.run(command, params).unwrap();
    (before, read(&app))
}

#[test]
fn actual_smudge_pixels_follow_pressure_on_and_keep_the_existing_result_off() {
    let mut h = bar(Tool::Smudge);
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0, "spacing": 0.25, "smoothing": {"enabled": false}}})).unwrap();
    h.run_steps(2);
    let off = h.state().session.tools.brush.clone();
    let (before, off_low) = smudged(&off, 0.25);
    let (_, off_full) = smudged(&off, 1.0);
    assert_ne!(off_full, before, "the edge fixture must actually smudge");
    assert_eq!(off_low, off_full, "off keeps today's pressure-independent size");
    toggle(&mut h);
    let on = h.state().session.tools.brush.clone();
    let (_, on_low) = smudged(&on, 0.25);
    let (_, on_full) = smudged(&on, 1.0);
    assert_ne!(on_low, on_full, "the actual command receives pen pressure and the selected brush");
    assert_ne!(on_low, off_low);
    assert_eq!(on_full, off_full, "full pressure keeps the full-size result");
    toggle(&mut h);
    assert_eq!(smudged(&h.state().session.tools.brush, 0.25).1, off_low);
}

#[test]
fn the_smudge_change_does_not_add_pressure_controls_to_blur_or_sharpen() {
    for tool in [Tool::Blur, Tool::Sharpen] {
        assert!(bar(tool).query_by_label(PRESSURE).is_none(), "{tool:?}");
    }
}
