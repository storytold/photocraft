//! #12648: the Blur tool's options bar ends with the Pressure for Size toggle, after Sample All Layers, with the
//! reference application's tooltip, off by default and after Reset Tool. It edits the Blur tool's own brush.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

const PRESSURE_SIZE: &str = "Always use pressure for size";

fn bar(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn switch(h: &mut H, tool: Tool) {
    h.state_mut().ui.tool = tool;
    h.run_steps(3);
}

fn pressure_size(h: &H) -> bool {
    h.state().session.tools.brush.pressure_size
}

#[test]
fn the_blur_bar_ends_with_pressure_for_size_after_sample_all_layers() {
    let h = bar(Tool::Blur);
    let toggle = h.get_by_label(PRESSURE_SIZE);
    assert_eq!(
        toggle.accesskit_node().description().as_deref(),
        Some("Always use Pressure for Size. When off, Brush Preset controls pressure"),
        "the reference's tooltip"
    );
    assert!(toggle.rect().left() > h.get_by_label("Sample All Layers").rect().right(), "after Sample All Layers");
    assert!(!pressure_size(&h), "off by default");
}

#[test]
fn it_turns_on_the_blur_brush_only_and_reset_tool_turns_it_off() {
    let mut h = bar(Tool::Brush);
    switch(&mut h, Tool::Blur);
    h.get_by_label(PRESSURE_SIZE).click();
    h.run_steps(2);
    assert!(pressure_size(&h), "on");
    switch(&mut h, Tool::Brush);
    assert!(!pressure_size(&h), "the Brush keeps its own setting");
    switch(&mut h, Tool::Blur);
    assert!(pressure_size(&h), "the Blur tool keeps its own");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    assert!(!pressure_size(&h), "off after Reset Tool");
}

/// Entering Blur for the first time from a Brush that has Pressure for Size on starts it off (CodeRabbit on #16117).
#[test]
fn blur_starts_off_when_entered_from_a_pressure_enabled_brush() {
    let mut h = bar(Tool::Brush);
    h.state_mut().session.tools.brush.pressure_size = true;
    switch(&mut h, Tool::Blur);
    assert!(!pressure_size(&h), "Blur's first use is off");
    switch(&mut h, Tool::Brush);
    assert!(pressure_size(&h), "the Brush keeps its own");
}
