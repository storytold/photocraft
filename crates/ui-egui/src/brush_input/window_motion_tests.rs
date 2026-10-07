//! Replay the exact normalized window events consumed by the Windows/X11/Wayland path.
use super::*;
use egui::{pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, pressed, button: PointerButton::Primary, modifiers: Modifiers::NONE }
}

#[test]
fn raw_curve_reaches_real_paint_commands_at_all_depths_and_with_smoothing() {
    for depth in [8, 16, 32] {
        for smoothing in [0.0, 0.1] {
            for scale in [0.5, 1.0, 2.0] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width":128,"height":128,"depth":depth})).unwrap();
                app.ui.tool = Tool::Brush;
                app.session.tools.brush.smoothing.amount = smoothing;
                let mut h = Harness::builder().with_size(vec2(1000.0, 800.0)).build_eframe(|_| app);
                h.run_steps(3);
                let a = h.state().last_canvas_rect.center();
                let b = a + vec2(12.0, 0.0);
                h.input_mut().events.extend([
                    Event::PointerMoved(a),
                    button(a, true),
                    Event::MouseMoved(vec2(2.0, 2.0) * scale),
                    Event::MouseMoved(vec2(2.0, -2.0) * scale),
                    Event::PointerMoved(b),
                    button(b, false),
                ]);
                h.step();
                let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
                assert_eq!(strokes.len(), 1);
                let points = strokes[0].1["points"].as_array().unwrap();
                assert_eq!(points.len(), 3, "depth {depth}, smoothing {smoothing}, scale {scale}");
                assert_ne!(points[1][1], points[2][1], "curve survives the real painting pipeline");
            }
        }
    }
}

#[test]
fn raw_stream_keeps_endpoints_and_rejects_ambiguous_or_interrupted_intervals() {
    let xf = ViewXform { rect: egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)), zoom: 1.0, center: [50.0; 2], flip: false };
    let a = pos2(20.0, 20.0);
    let b = pos2(32.0, 20.0);
    // Replay one stream through the old OS-only route and the added relative-motion route.
    let events = [button(a, true), Event::MouseMoved(vec2(2.0, 2.0)), Event::MouseMoved(vec2(2.0, -2.0)), Event::PointerMoved(b), button(b, false)];
    let mut old = BrushInput::default();
    let baseline = old.events(&events, &xf, true, false, 1.0, Modifiers::NONE);
    let mut new = BrushInput { raw_frame_time: Some(1.0), ..Default::default() };
    let enhanced = new.events(&events, &xf, true, false, 1.0, Modifiers::NONE);
    assert_eq!((baseline.len(), enhanced.len()), (3, 4));
    assert_eq!(baseline.first(), enhanced.first());
    assert_eq!(baseline[1..], enhanced[2..], "same OS endpoint and release; only the interior curve is added");
    for barrier in [None, Some(Event::PointerGone), Some(button(a, false)), Some(Event::WindowFocused(false))] {
        let mut input = BrushInput { raw_frame_time: Some(1.0), ..Default::default() };
        input.events(&[button(a, true), Event::MouseMoved(vec2(2.0, 2.0))], &xf, true, false, 1.0, Modifiers::NONE);
        if let Some(barrier) = barrier.clone() {
            input.events(&[barrier], &xf, true, false, 1.0, Modifiers::NONE);
        }
        input.raw_frame_time = Some(1.01);
        let out = input.events(&[Event::MouseMoved(vec2(2.0, -2.0)), Event::PointerMoved(b)], &xf, true, false, 1.0, Modifiers::NONE);
        assert_eq!(
            out.len(),
            if barrier.is_none() {
                2
            } else if matches!(barrier, Some(Event::PointerGone)) {
                1
            } else {
                0
            }
        );
        if let Some((ToolEvent::Move { x, y, .. }, _, _)) = out.last() {
            assert_eq!([*x, *y], xf.to_doc(b));
        }
    }
    for raw in [vec![], vec![vec2(f32::NAN, 0.0)], vec![vec2(-2.0, 2.0), vec2(-2.0, -2.0)], vec![vec2(1.0, 0.0); 65]] {
        let mut input = BrushInput { raw_frame_time: Some(1.0), ..Default::default() };
        let mut events = vec![button(a, true)];
        events.extend(raw.into_iter().map(Event::MouseMoved));
        events.push(Event::PointerMoved(b));
        let out = input.events(&events, &xf, true, false, 1.0, Modifiers::NONE);
        assert_eq!(out.len(), 2, "only press and authoritative endpoint on fallback");
    }
    let mut input = BrushInput { raw_frame_time: Some(1.0), ..Default::default() };
    input.events(&[button(a, true), Event::MouseMoved(vec2(2.0, 2.0)), Event::MouseMoved(vec2(2.0, -2.0))], &xf, true, false, 1.0, Modifiers::NONE);
    let changed = ViewXform { zoom: 2.0, ..xf };
    assert_eq!(input.events(&[Event::PointerMoved(b)], &changed, true, false, 1.0, Modifiers::NONE).len(), 1);
}

#[test]
fn wayland_batched_absolute_positions_do_not_compress_a_raw_curve() {
    let xf = ViewXform { rect: egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)), zoom: 1.0, center: [50.0; 2], flip: false };
    let a = pos2(20.0, 20.0);
    let b = pos2(32.0, 20.0);
    let c = pos2(44.0, 20.0);
    let mut input = BrushInput { raw_frame_time: Some(1.0), ..Default::default() };
    let out = input.events(
        &[
            button(a, true),
            Event::MouseMoved(vec2(2.0, 2.0)),
            Event::MouseMoved(vec2(2.0, -2.0)),
            Event::PointerMoved(b),
            Event::PointerMoved(c),
            button(c, false),
        ],
        &xf,
        true,
        false,
        1.0,
        Modifiers::NONE,
    );
    assert_eq!(out.len(), 4, "press, both OS endpoints, release; no misattributed curve");
}

#[test]
fn touch_native_collector_and_automation_do_not_mix_with_raw_mouse_motion() {
    for mode in ["touch", "active touch", "unpressured touch", "invalid force touch", "pen", "native", "automation"] {
        let services = if mode == "native" {
            crate::Services {
                motion_samples: Some(Box::new(|event, _| match event {
                    crate::MouseMotion::Move { from, to } => vec![from.lerp(to, 0.5) - vec2(0.0, 6.0)],
                    _ => Vec::new(),
                })),
                ..Default::default()
            }
        } else {
            crate::Services::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width":128,"height":128})).unwrap();
        app.ui.tool = Tool::Brush;
        app.stylus.use_pressure = false;
        if mode == "pen" {
            app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: 0.5, ..Default::default() }));
        }
        let mut h = Harness::builder().with_size(vec2(1000.0, 800.0)).build_eframe(|_| app);
        h.run_steps(3);
        h.state_mut().automation_input = mode == "automation";
        let a = h.state().last_canvas_rect.center();
        let b = a + vec2(12.0, 0.0);
        if mode.ends_with("touch") && mode != "touch" {
            let force = match mode {
                "unpressured touch" => None,
                "invalid force touch" => Some(f32::NAN),
                _ => Some(0.5),
            };
            h.input_mut().events.push(Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(0), phase: egui::TouchPhase::Start, pos: a, force });
            h.step();
        }
        h.input_mut().events.extend([
            Event::PointerMoved(a),
            button(a, true),
            Event::MouseMoved(vec2(2.0, 2.0)),
            Event::MouseMoved(vec2(2.0, -2.0)),
            Event::PointerMoved(b),
            button(b, false),
        ]);
        if mode == "touch" {
            h.input_mut().events.push(Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(0),
                phase: egui::TouchPhase::Move,
                pos: b,
                force: Some(0.5),
            });
        }
        h.step();
        let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
        let points = strokes[0].1["points"].as_array().unwrap();
        assert_eq!(points.len(), if mode == "native" { 3 } else { 2 }, "{mode}");
        if mode == "native" {
            assert!(points[1][1].as_f64().unwrap() < points[2][1].as_f64().unwrap(), "native samples take precedence");
        }
    }
}
