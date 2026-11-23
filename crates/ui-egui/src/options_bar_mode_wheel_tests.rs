//! #2061: the scroll wheel over the options bar's Mode dropdown (Brush, Paint
//! Bucket, the stamps, History Brush, Healing) steps through the blend modes, as over the Layers
//! panel's (#1747) and as in Photoshop.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The app with a new document and `tool` chosen.
pub(super) fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

/// The options bar's Mode dropdown: the highest one showing `Normal` (the Layers panel's is lower).
pub(super) fn mode_dropdown(h: &Harness<'_, Box<PhotocraftApp>>) -> Rect {
    h.query_all_by_role(Role::ComboBox).filter(|n| n.value().as_deref() == Some("Normal")).map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap()
}

/// `lines` wheel lines over `at` in one turn: negative is down.
pub(super) fn wheel(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2, lines: f32) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(6);
}

pub(super) fn brush_mode(h: &Harness<'_, Box<PhotocraftApp>>) -> BlendMode {
    h.state().session.tools.brush.mode
}

#[test]
fn the_wheel_over_the_brush_mode_dropdown_steps_the_painting_mode() {
    let mut h = harness(Tool::Brush);
    let at = mode_dropdown(&h).center();
    assert_eq!(brush_mode(&h), BlendMode::Normal);
    wheel(&mut h, at, -1.0);
    assert_eq!(brush_mode(&h), BlendMode::Dissolve);
    wheel(&mut h, at, -1.0);
    assert!(h.state().session.tools.brush.behind, "Behind follows Dissolve");
    wheel(&mut h, at, -1.0);
    assert!(h.state().ui.tool_options.brush_clear, "Clear follows Behind");
    assert!(!h.state().session.tools.brush.behind);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().ui.tool_options.brush_clear);
    assert_eq!(brush_mode(&h), BlendMode::Darken);
    wheel(&mut h, at, 4.0);
    assert_eq!(brush_mode(&h), BlendMode::Normal, "up steps back and stops at the first mode");
}
