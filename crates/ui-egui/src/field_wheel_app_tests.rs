//! #5473 in the app: a focused value field takes the wheel over the canvas, which neither pans
//! nor zooms with it.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    h
}

/// The value field showing `value`, unit included: the options bar's Smoothing is the brush's only one at 10 %.
fn field<'a>(h: &'a Harness<'_, Box<PhotocraftApp>>, value: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.value().as_deref() == Some(value)).unwrap()
}

fn wheel(h: &mut Harness<'_, Box<PhotocraftApp>>, at: egui::Pos2, lines: f32, modifiers: egui::Modifiers) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers,
    });
    h.run_steps(6);
}

/// (zoom, centre) of the document's view.
fn view(h: &Harness<'_, Box<PhotocraftApp>>) -> (f32, [f32; 2]) {
    let v = &h.state().ui.views[0];
    (v.zoom, v.center)
}

#[test]
fn the_wheel_over_a_field_steps_it_without_a_click() {
    let mut h = harness();
    let before = view(&h);
    let at = field(&h, "10 %").rect().center();
    wheel(&mut h, at, 1.0, egui::Modifiers::NONE);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("11 %")), "Smoothing stepped from 10 to 11");
    wheel(&mut h, at, 1.0, egui::Modifiers::SHIFT);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("21 %")), "Shift: ten a notch");
    assert!(!field(&h, "21 %").is_focused(), "hovering doesn't put the cursor in the field");
    assert_eq!(view(&h), before, "the canvas didn't move");
}

/// Frames a wheel event gets to play out (0.25 s each, the harness's default step). egui's wheel smoothing reaches
/// 90 % in 0.1 s and takes the rest under one point at once, so the smoothed tail of a notch (about 50 points) ends
/// within three frames even at a 0.1 s step; four leave one to spare: the view after
/// them is the one the old forty left.
const TAIL_STEPS: usize = 4;

/// How far a wheel of `delta` lines with `modifiers` over the canvas moves the view, a Smoothing
/// field focused or not, once egui's smoothed scroll has run out. The field isn't stepped.
fn canvas_pan(focus: bool, delta: egui::Vec2, modifiers: egui::Modifiers) -> (f32, [f32; 2]) {
    let mut h = harness();
    if focus {
        field(&h, "10 %").click();
        h.run();
    }
    let canvas = egui::pos2(640.0, 500.0);
    h.hover_at(canvas);
    h.run_steps(1);
    let before = view(&h);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers,
    });
    h.run_steps(TAIL_STEPS);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("10 %")), "focus {focus}: the field wasn't stepped");
    let after = view(&h);
    (after.0 / before.0, [after.1[0] - before.1[0], after.1[1] - before.1[1]])
}

/// The wheel a focused field leaves to the view moves it as far as with no field focused: its
/// smoothed tail isn't cut off (⌘/Ctrl + wheel, and a plain sideways trackpad swipe).
#[test]
fn the_view_keeps_its_whole_wheel_while_a_field_is_focused() {
    for (delta, m) in [(vec2(0.0, -1.0), egui::Modifiers::COMMAND), (vec2(0.0, -1.0), egui::Modifiers::CTRL), (vec2(-1.0, 0.0), egui::Modifiers::NONE)] {
        let free = canvas_pan(false, delta, m);
        assert!(free != (1.0, [0.0, 0.0]), "{delta:?} {m:?}: the wheel moves the view");
        let focused = canvas_pan(true, delta, m);
        let close = (focused.0 - free.0).abs() < 1e-3 && (0..2).all(|k| (focused.1[k] - free.1[k]).abs() < 0.5);
        assert!(close, "{delta:?} {m:?}: {focused:?} with a field focused, {free:?} without");
    }
}
