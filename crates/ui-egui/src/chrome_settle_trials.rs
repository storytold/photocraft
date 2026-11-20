//! The window's chrome settles (#15493): a frame whose only input is the pointer moving over the canvas runs no dock
//! group header, panel tab strip or grip and no title bar; each repaints its last run. Before, every frame ran them.

use craft::ui::runtime::measure::last_pass_runs;
use egui::{Event, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;

/// The chrome components the last pass ran, by type name.
fn chrome_runs(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<(&'static str, u32)> {
    const CHROME: [&str; 4] =
        ["docking::group::DockGroup<", "docking::group::GroupGrip<", "TabStrip<'_, craft_ui_panels::docking::PanelKey>", "title_bar::bar::TitleBar<"];
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| CHROME.iter().any(|name| ty.contains(name))).collect()
}

#[test]
fn a_hover_over_the_canvas_runs_no_dock_chrome_and_no_title_bar() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default())
    });
    h.run_steps(8);
    let at = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(at));
    h.run_steps(4);
    assert!(!last_pass_runs(&h.ctx).is_empty(), "the frame runs components");
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        assert_eq!(chrome_runs(&h), [], "hover step {step}: the dock chrome and the title bar repaint their last run");
    }
}
