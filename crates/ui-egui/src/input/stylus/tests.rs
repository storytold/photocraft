use super::*;

/// Hold a touch contact still for 1.2 s over a click-sensing area, with the press-and-hold
/// handling the app does in `raw_input_hook`. Did the area see a secondary click?
fn held_contact_right_clicks(hover_first: bool, force: Option<f32>) -> bool {
    let ctx = egui::Context::default();
    let mut contacts = PenContacts::default();
    let mut right_clicked = false;
    let ev = |phase, force| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(3), phase, pos: egui::pos2(50.0, 50.0), force };
    // A first frame lays the area out, so the press hits it.
    let mut frames = vec![(0.0, vec![])];
    if hover_first {
        frames.push((0.05, vec![ev(egui::TouchPhase::Move, None), egui::Event::PointerMoved(egui::pos2(50.0, 50.0))]));
    }
    frames.push((
        0.1,
        vec![
            ev(egui::TouchPhase::Start, force),
            egui::Event::PointerMoved(egui::pos2(50.0, 50.0)),
            egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ],
    ));
    for t in [0.4, 0.7, 1.0, 1.3] {
        frames.push((t, vec![]));
    }
    for (time, mut events) in frames {
        // egui-winit emits the touch and its emulated pointer events together.
        let mut raw =
            egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0))), ..Default::default() };
        raw.events.append(&mut events);
        let pen = contacts.observe(&raw.events);
        pen_hold_is_no_right_click(&ctx, pen);
        let mut out = ctx.run_ui(raw, |ui| {
            let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click_and_drag());
            right_clicked |= r.secondary_clicked();
        });
        out.textures_delta.clear();
    }
    right_clicked
}

#[test]
fn a_held_pen_is_not_a_right_click_but_a_held_finger_still_is() {
    assert!(held_contact_right_clicks(false, None), "a finger's press-and-hold is egui's context menu");
    assert!(!held_contact_right_clicks(true, None), "a pen that hovered first");
    assert!(!held_contact_right_clicks(false, Some(0.4)), "a contact that reports pressure");
}

#[test]
fn pen_contacts_follow_hover_force_and_lift() {
    use egui::TouchPhase::{Cancel, End, Move, Start};
    let t = |id, phase, force| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: egui::pos2(1.0, 1.0), force };
    let mut c = PenContacts::default();
    assert!(!c.observe(&[t(1, Move, None)]), "hovering is no contact");
    assert!(c.observe(&[t(1, Start, None)]));
    assert!(c.observe(&[]), "still down");
    assert!(c.observe(&[t(1, End, None)]), "the lift frame still counts");
    assert!(!c.observe(&[]));
    // A finger, until it reports a force.
    assert!(!c.observe(&[t(9, Start, None), t(9, Move, None)]));
    assert!(c.observe(&[t(9, Move, Some(0.5))]));
    assert!(c.observe(&[t(9, Cancel, None)]));
    assert!(!c.observe(&[]));
}

fn pen(pressure: f32) -> PenSample {
    PenSample { pressure, ..Default::default() }
}

/// The runtime's reading of `pen`: tilt and rotation in radians.
fn reading(pen: PenSample) -> PointerSample {
    let mut s = PointerSample::new(pen.pressure);
    s.tilt = [pen.tilt_x.to_radians(), pen.tilt_y.to_radians()];
    s.rotation = pen.rotation.to_radians();
    s.eraser = pen.eraser;
    s
}

#[test]
fn the_pressure_curve_reshapes_pen_pressure_but_not_the_mouse() {
    let mut s = Stylus::default();
    s.set_pressure_curve(&[[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]]);
    s.record_pointer(0.0, Some(reading(pen(0.3))));
    assert!((s.pressure() - 0.6).abs() < 1e-5, "{}", s.pressure());
    s.set_pressure_curve(&[[0.0, 0.0], [1.0, 1.0]]);
    assert!((s.pressure() - 0.3).abs() < 1e-6, "linear again");
    // A mouse is full pressure whatever the curve.
    let mut m = Stylus::default();
    m.set_pressure_curve(&[[0.0, 0.0], [1.0, 0.2]]);
    m.record_pointer(0.0, None);
    assert_eq!(m.sample(), None);
    assert_eq!(m.pressure(), 1.0);
}

#[test]
fn a_new_stroke_never_starts_from_an_old_strokes_pressure() {
    let mut s = Stylus::default();
    s.record_pointer(0.0, Some(reading(PenSample { tilt_x: 40.0, ..pen(0.9) })));
    s.begin_stroke();
    s.record_pointer(10.0, Some(reading(pen(0.1))));
    // The next stroke's press: its own reading, never the old peak or the old tilt.
    s.record_pointer(500.0, Some(reading(pen(0.1))));
    s.begin_stroke();
    assert!((s.pressure() - 0.1).abs() < 1e-6, "{}", s.pressure());
    assert!((s.tap_pressure() - 0.1).abs() < 1e-6, "the tap peak restarts: {}", s.tap_pressure());
    assert_eq!(s.stroke.len(), 1, "the old stroke's poses are gone");
    assert_eq!(s.stroke[0], [0.0, 0.0, 0.0]);
}

#[test]
fn mouse_is_full_pressure() {
    let mut s = Stylus::default();
    s.record_pointer(0.0, None);
    assert_eq!(s.pressure(), 1.0);
    assert_eq!(s.sample(), None);
}

#[test]
fn feed_wins_and_is_sanitized() {
    // A tablet backend's out-of-range values are clamped before any brush sees them.
    let wild = PenSample { pressure: 7.0, tilt_x: 120.0, tilt_y: f32::NAN, rotation: -90.0, eraser: false }.sanitized();
    assert_eq!(wild, PenSample { pressure: 1.0, tilt_x: 90.0, tilt_y: 0.0, rotation: 270.0, eraser: false });
    let feed = StylusFeed::default();
    feed.set(Some(PenSample { pressure: 0.6, tilt_x: 120.0, tilt_y: f32::NAN, rotation: -90.0, eraser: false }));
    assert_eq!(feed.get(), Some(PenSample { pressure: 0.6, tilt_x: 90.0, tilt_y: 0.0, rotation: 270.0, eraser: false }));
}

#[test]
fn pen_samples_reach_the_brush_stroke() {
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.ui.tool = crate::state::Tool::Brush;
    let m = egui::Modifiers::NONE;
    // The pen's readings go into the feed; each event carries the pressure of the reading it goes with.
    app.stylus.feed.set(Some(PenSample { pressure: 0.2, tilt_x: 40.0, tilt_y: 0.0, rotation: 30.0, eraser: false }));
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 0.2 }, m);
    app.stylus.feed.set(Some(PenSample { pressure: 0.9, tilt_x: 10.0, tilt_y: -5.0, rotation: 60.0, eraser: false }));
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 0.9 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    let (id, p) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").cloned().unwrap();
    assert_eq!(id, "paint.stroke");
    // x, y, pressure, tilt and rotation in degrees (through the runtime's radians), then the point's time.
    let near = |point: &serde_json::Value, want: [f64; 6]| {
        let got: Vec<f64> = point.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        assert!(got.len() == 7 && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-4), "{got:?} is not {want:?}");
    };
    near(&p["points"][0], [10.0, 40.0, f64::from(0.2f32), 40.0, 0.0, 30.0]);
    near(&p["points"][1], [100.0, 40.0, f64::from(0.9f32), 10.0, -5.0, 60.0]);
    // A mouse stroke stays at full pressure with no tilt.
    app.stylus.feed.set(None);
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 60.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 60.0 }, m);
    let (_, p) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").cloned().unwrap();
    assert_eq!(p["points"][0].as_array().unwrap()[..3], [json!(10.0), json!(60.0), json!(1.0)]);
    assert!(p["points"][0].as_array().unwrap().iter().skip(3).take(3).all(|v| v.as_f64() == Some(0.0)), "no tilt or rotation");
}

#[test]
fn stroke_points_carry_tilt_only_when_present() {
    let mut s = Stylus::default();
    let pts = [[0.0, 0.0, 0.5], [10.0, 0.0, 0.7], [20.0, 0.0, 0.7]];
    s.record_pointer(0.0, None);
    s.begin_stroke();
    s.record_point();
    assert_eq!(s.stroke_points(&pts)[1], vec![10.0, 0.0, 0.7]);
    s.record_pointer(10.0, Some(reading(PenSample { pressure: 0.7, tilt_x: 30.0, tilt_y: 10.0, rotation: 45.0, eraser: false })));
    s.record_point();
    let out = s.stroke_points(&pts);
    assert_eq!(out[0], vec![0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0]);
    // Degrees survive the radians of the runtime's reading.
    let tail = &out[2][3..6];
    assert!([30.0, 10.0, 45.0].iter().zip(tail).all(|(w, g)| (w - g).abs() < 1e-3), "{tail:?}");
}

#[test]
fn use_pressure_off_ignores_pen_and_touch() {
    let mut s = Stylus { use_pressure: false, ..Default::default() };
    s.record_pointer(0.0, Some(reading(PenSample { pressure: 0.2, tilt_x: 40.0, ..Default::default() })));
    assert_eq!((s.sample(), s.pressure()), (None, 1.0));
    s.begin_stroke();
    assert_eq!(s.stroke_points(&[[1.0, 2.0, 1.0]]), vec![vec![1.0, 2.0, 1.0]], "no tilt either");
}

#[test]
fn eraser_end_switches_tools_once_and_never_mid_drag() {
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", serde_json::json!({"width": 60, "height": 60})).unwrap();
    app.ui.tool = Tool::Brush;
    let pen = |eraser| Some(PenSample { pressure: 0.5, eraser, ..Default::default() });
    // A mouse never switches.
    assert!(!Stylus::sync_eraser_tool(&mut app));
    app.stylus.feed.set(pen(true));
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
    assert!(!Stylus::sync_eraser_tool(&mut app), "reported once");
    // Flipping back during a drag waits for the drag to end.
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 0.5 }, egui::Modifiers::NONE);
    app.stylus.feed.set(pen(false));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
    // The canvas pass after the release takes the flip. The driver's release ends the pen's reading with the press, so
    // the pen, still in range at its tip, is read again for that pass.
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, egui::Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Eraser, "the release itself changes no tool");
    app.stylus.feed.set(pen(false));
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Brush, "the tip restores the previous tool");
    assert!(!Stylus::sync_eraser_tool(&mut app), "reported once");
    // The eraser end while the Eraser is already chosen remembers nothing to restore.
    app.ui.tool = Tool::Eraser;
    app.stylus.feed.set(pen(true));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    app.stylus.feed.set(pen(false));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
}

/// Windows reports the pen's end through `Services::pen_end`, not the feed (#2285): flipping the
/// pen switches tools the same way, and a feed sample (automation, another backend) wins.
#[test]
fn the_platform_pen_end_switches_to_the_eraser_and_back() {
    use crate::state::Tool;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", serde_json::json!({"width": 60, "height": 60})).unwrap();
    app.ui.tool = Tool::Brush;
    // 0 = no pen, 1 = tip, 2 = eraser end.
    let end = Arc::new(AtomicU8::new(0));
    let read = end.clone();
    app.services.pen_end = Some(Box::new(move || match read.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }));
    assert!(!Stylus::sync_eraser_tool(&mut app), "no pen in range");
    end.store(1, Ordering::Relaxed);
    assert!(!Stylus::sync_eraser_tool(&mut app), "the tip keeps the tool");
    end.store(2, Ordering::Relaxed);
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
    assert!(!Stylus::sync_eraser_tool(&mut app), "reported once");
    // Out of range and back with the eraser end: still the Eraser, nothing to switch.
    end.store(0, Ordering::Relaxed);
    assert!(!Stylus::sync_eraser_tool(&mut app));
    end.store(2, Ordering::Relaxed);
    assert!(!Stylus::sync_eraser_tool(&mut app));
    end.store(1, Ordering::Relaxed);
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Brush, "the tip restores the previous tool");
    // A feed sample is read before the platform's end.
    end.store(2, Ordering::Relaxed);
    app.stylus.feed.set(Some(PenSample { pressure: 0.5, eraser: false, ..Default::default() }));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Brush);
}

/// The pen's eraser end is a tool switch like any other: Add New Artboard's placing mode ends
/// (#11386), and coming back to the Artboard tool on the tip leaves it off.
#[test]
fn the_eraser_end_ends_the_artboard_placing_mode() {
    use crate::state::Tool;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", serde_json::json!({"width": 60, "height": 60})).unwrap();
    app.ui.tool = Tool::Artboard;
    app.ui.artboard_tool.adding = true;
    let pen = |eraser| Some(PenSample { pressure: 0.5, eraser, ..Default::default() });
    app.stylus.feed.set(pen(true));
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert!(!app.ui.artboard_tool.adding, "the eraser end ends the mode");
    app.stylus.feed.set(pen(false));
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Artboard);
    assert!(!app.ui.artboard_tool.adding);
}
