//! Brush strokes through the real canvas: the moves before egui recognises a drag reach the
//! stroke, and stroke points carry the times the engine's time-based features need.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Brush;
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 50.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
    };
    xf.to_screen(x, y)
}

fn button(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, down: bool) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

/// The points of the committed `paint.stroke`.
fn committed_points(h: &Harness<'static, PhotocraftApp>) -> Vec<Vec<f64>> {
    let (_, p) = h.state().session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").cloned().expect("a committed stroke");
    p["points"].as_array().expect("points").iter().map(|v| v.as_array().expect("point").iter().filter_map(Value::as_f64).collect()).collect()
}

#[test]
fn moves_before_the_drag_threshold_reach_the_stroke() {
    let mut h = harness();
    move_to(&mut h, 20.0, 50.0);
    button(&mut h, 20.0, 50.0, true);
    // Small steps, each well inside egui's click distance from the press.
    for x in [21.0, 22.0, 23.0] {
        move_to(&mut h, x, 50.0);
    }
    for x in [40.0, 60.0] {
        move_to(&mut h, x, 50.0);
    }
    button(&mut h, 60.0, 50.0, false);
    h.run_steps(2);
    let xs: Vec<f64> = committed_points(&h).iter().map(|p| p[0]).collect();
    for x in [21.0, 22.0, 23.0] {
        assert!(xs.iter().any(|v| (v - x).abs() < 0.01), "the move to {x} is part of the stroke: {xs:?}");
    }
}

#[test]
fn stroke_points_carry_increasing_times_from_zero() {
    let mut h = harness();
    move_to(&mut h, 20.0, 50.0);
    button(&mut h, 20.0, 50.0, true);
    for x in [30.0, 40.0, 50.0, 60.0] {
        move_to(&mut h, x, 50.0);
    }
    button(&mut h, 60.0, 50.0, false);
    h.run_steps(2);
    let pts = committed_points(&h);
    assert!(pts.len() >= 4, "{pts:?}");
    let times: Vec<f64> = pts.iter().map(|p| *p.get(6).expect("timeMs")).collect();
    assert_eq!(times.first().copied(), Some(0.0), "{times:?}");
    assert!(times.windows(2).all(|w| w[1] >= w[0]), "times never go back: {times:?}");
    assert!(times.last().copied().unwrap_or(0.0) > 0.0, "time advances over the frames: {times:?}");
}

/// Points of the drag in progress after holding the pointer still for a while.
fn held_points(h: &mut Harness<'static, PhotocraftApp>) -> (usize, usize) {
    move_to(h, 20.0, 50.0);
    button(h, 20.0, 50.0, true);
    move_to(h, 30.0, 50.0);
    let before = h.state().drag.as_ref().map_or(0, |d| d.points.len());
    h.run_steps(12);
    let after = h.state().drag.as_ref().map_or(0, |d| d.points.len());
    button(h, 30.0, 50.0, false);
    (before, after)
}

#[test]
fn a_held_airbrush_keeps_the_stroke_clock_running() {
    let mut h = harness();
    h.state_mut().run("tools.setBrush", json!({"buildUp": true, "smoothing": {"amount": 0.0}})).unwrap();
    let (before, after) = held_points(&mut h);
    assert!(after > before, "a held airbrush adds timed repeats of its point ({before} → {after})");
}

#[test]
fn a_held_plain_brush_adds_nothing() {
    let mut h = harness();
    h.state_mut().run("tools.setBrush", json!({"buildUp": false, "smoothing": {"amount": 0.0}})).unwrap();
    let (before, after) = held_points(&mut h);
    assert_eq!(after, before, "nothing changes with time, so nothing is repeated");
}

#[test]
fn spread_ms_spaces_a_frames_samples_after_the_last_point() {
    use crate::canvas::spread_ms;
    assert_eq!((0..4).map(|k| spread_ms(100.0, 116.0, k, 4)).collect::<Vec<_>>(), vec![104.0, 108.0, 112.0, 116.0]);
    assert_eq!(spread_ms(116.0, 116.0, 0, 3), 116.0, "a press frame's moves share its time");
    assert_eq!(spread_ms(120.0, 116.0, 0, 1), 116.0, "a clock that went back never goes below now");
    assert_eq!(spread_ms(f64::NAN, 116.0, 0, 1), 116.0);
}

#[test]
fn moves_in_one_frame_take_the_pressures_the_pen_reported_between_them() {
    use crate::stylus::PenSample;
    let mut h = harness();
    let pen = |p: f32| Some(PenSample { pressure: p, ..Default::default() });
    h.state().stylus.feed.set(pen(0.2));
    move_to(&mut h, 20.0, 50.0);
    button(&mut h, 20.0, 50.0, true);
    // One frame: three moves while the pen reported 0.4, 0.6 and 0.8. They go straight into the
    // next frame's raw input, as moves piling up during a slow frame do (kittest's `event`
    // delivers queued pointer moves one per frame).
    for (x, p) in [(30.0, 0.4), (40.0, 0.6), (50.0, 0.8)] {
        h.state().stylus.feed.set(pen(p));
        let q = screen(&h, x, 50.0);
        h.input_mut().events.push(egui::Event::PointerMoved(q));
    }
    h.run_steps(1);
    button(&mut h, 50.0, 50.0, false);
    h.run_steps(2);
    let pts = committed_points(&h);
    let tail: Vec<f64> = pts.iter().filter(|p| p[0] >= 29.0).map(|p| (p[2] * 100.0).round() / 100.0).collect();
    assert_eq!(tail.get(..3), Some(&[0.4, 0.6, 0.8][..]), "each move keeps its own pressure: {pts:?}");
}

#[test]
fn the_pressure_curve_shapes_the_committed_stroke() {
    use crate::stylus::PenSample;
    let mut h = harness();
    h.state_mut().run("prefs.set", json!({"path": "tools.pressureCurve", "value": [[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]]})).unwrap();
    h.state().stylus.feed.set(Some(PenSample { pressure: 0.3, ..Default::default() }));
    move_to(&mut h, 20.0, 50.0);
    button(&mut h, 20.0, 50.0, true);
    for x in [40.0, 60.0] {
        h.state().stylus.feed.set(Some(PenSample { pressure: 0.3, ..Default::default() }));
        move_to(&mut h, x, 50.0);
    }
    button(&mut h, 60.0, 50.0, false);
    h.run_steps(2);
    let pts = committed_points(&h);
    assert!(pts.iter().all(|p| (p[2] - 0.6).abs() < 1e-3), "pen 0.3 through the curve is 0.6: {pts:?}");
}
