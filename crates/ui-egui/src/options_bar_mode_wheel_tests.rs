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

/// The centre of a dropdown showing `value`.
fn combo(h: &Harness<'_, Box<PhotocraftApp>>, value: &str) -> Option<Pos2> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some(value)).map(|n| n.rect().center())
}

/// The Paint Bucket's Mode dropdown has Behind and Clear after Dissolve, as the Brush's does; on
/// the Background both are greyed and the wheel steps past them (#12531).
#[test]
fn the_paint_bucket_mode_dropdown_has_behind_and_clear() {
    let mut h = harness(Tool::PaintBucket);
    let at = mode_dropdown(&h).center();
    // The new document's Background is active.
    wheel(&mut h, at, -1.0);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Dissolve);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().ui.tool_options.bucket_behind && !h.state().ui.tool_options.bucket_clear, "greyed on the Background");
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Darken);

    let mut h = harness(Tool::PaintBucket);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    let at = mode_dropdown(&h).center();
    wheel(&mut h, at, -1.0);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Dissolve);
    wheel(&mut h, at, -1.0);
    assert!(h.state().ui.tool_options.bucket_behind);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Normal);
    wheel(&mut h, at, -1.0);
    assert!(h.state().ui.tool_options.bucket_clear && !h.state().ui.tool_options.bucket_behind);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().ui.tool_options.bucket_clear);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Darken);
}

/// Behind chosen on a pixel layer is the bucket's stored mode, so the Mode control keeps
/// reading Behind after a switch to the Background — greyed in its dropdown menu there
/// (#12531), as the Brush's Clear and the Pattern Stamp's Behind do with theirs — until a
/// bucket click on the Background turns it off (#11897's rule, pinned for the click in
/// `paint_bucket_behind_turns_off_on_the_background`). Switching layers alone clears
/// nothing, so returning to a pixel layer finds Behind still chosen.
#[test]
fn the_paint_bucket_mode_still_reads_behind_on_the_background_until_a_click() {
    let mut h = harness(Tool::PaintBucket);
    let background = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    let pixel = h.state().session.active().unwrap().active_layer.unwrap();
    assert_ne!(pixel, background, "the new pixel layer is active");
    h.state_mut().ui.tool_options.bucket_behind = true;
    h.run_steps(4);
    assert!(combo(&h, "Behind").is_some(), "Behind reads on a pixel layer");
    h.state_mut().session.active_mut().unwrap().set_active_layer(Some(background));
    h.run_steps(4);
    assert!(h.state().ui.tool_options.bucket_behind, "switching to the Background alone does not clear Behind");
    assert!(combo(&h, "Behind").is_some(), "Mode still reads Behind on the Background");
    h.state_mut().session.active_mut().unwrap().set_active_layer(Some(pixel));
    h.run_steps(4);
    assert!(h.state().ui.tool_options.bucket_behind, "Behind survives the round trip to the Background");
    assert!(combo(&h, "Behind").is_some(), "and still reads Behind back on a pixel layer");
}
