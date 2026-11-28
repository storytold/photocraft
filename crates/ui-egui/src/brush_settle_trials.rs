//! The options bar's Brush Preset field runs only for what it shows (the presets, the brush's size, hardness, angle,
//! roundness and tip): a brush write that leaves those alone replays it, and the paint members, which show the
//! opacity, run.

use craft::ui::runtime::measure::{MemoRun, last_pass_memo_runs};
use egui::{Event, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::chrome_settle_trials::{ShellSlot, slot_runs};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// How the memoized view whose props type is named `props` took part in the last pass. A view inside the options bar is
/// not decided when the shell's options slot replays: it replays with it.
fn run_of(h: &H, props: &str) -> Option<MemoRun> {
    let runs = last_pass_memo_runs(&h.ctx);
    let own = runs.iter().find(|(name, _, _)| name.contains(props)).map(|(_, _, run)| *run);
    own.or_else(|| (slot_runs(&runs, ShellSlot::Options) == (0, 1)).then_some(MemoRun::Replayed))
}

#[test]
fn a_brush_write_that_leaves_the_shown_values_alone_replays_the_brush_preset_field() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default());
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    let at = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(at));
    h.run_steps(4);
    assert_eq!(run_of(&h, "brush_picker::FieldProps"), Some(MemoRun::Replayed), "the field replays at rest");
    for (step, opacity) in [0.5, 0.6, 0.7].into_iter().enumerate() {
        h.state_mut().run("tools.setBrush", json!({"brush": {"opacity": opacity}})).unwrap();
        h.run_steps(1);
        assert_eq!(run_of(&h, "brush_picker::FieldProps"), Some(MemoRun::Replayed), "opacity step {step}: the field shows no opacity");
        assert!(run_of(&h, "paint::PaintProps").is_some_and(|run| run != MemoRun::Replayed), "opacity step {step}: the paint members show it");
    }
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 42}})).unwrap();
    h.run_steps(1);
    assert!(run_of(&h, "brush_picker::FieldProps").is_some_and(|run| run != MemoRun::Replayed), "a size write runs the field");
}
