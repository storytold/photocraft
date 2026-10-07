//! Painting consumes ordered window events, rather than just the last position of a frame.
//! Absolute OS positions retain the trackpad's acceleration and match the native cursor.

use egui::{Event, Modifiers, PointerButton, Response};

use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::state::Tool;
use crate::{MouseMotion, PhotocraftApp};

#[derive(Default)]
pub struct BrushInput {
    pub defer_preview: bool,
    button: Option<PointerButton>,
    last: Option<egui::Pos2>,
    owner: Option<(photocraft_doc::DocId, Tool)>,
    transform: Option<ViewXform>,
    window_motion: photocraft_tablet::motion::Aligner,
    raw_frame_time: Option<f64>,
}

/// Returns true when this path owns painting; other tools retain their response-based gestures.
pub fn route(app: &mut PhotocraftApp, response: &Response, xf: &ViewXform, tool: Tool) -> bool {
    let eligible = response.hovered() || response.dragged() || response.drag_stopped() || response.is_pointer_button_down_on();
    if !tool.is_brushlike() && tool != Tool::QuickSelection {
        return false;
    }
    let (events, mods, frame_time, touch) = response.ctx.input(|i| {
        (
            i.raw.events.iter().filter(|e| app.stylus.use_pressure || !matches!(e, Event::Touch { .. })).cloned().collect::<Vec<_>>(),
            i.modifiers,
            i.time,
            i.any_touches() || i.raw.events.iter().any(|e| matches!(e, Event::Touch { .. })),
        )
    });
    let secondary = crate::paint_mouse::right_erases(app, tool);
    let pressure = app.stylus.pressure();
    let zoom = response.ctx.zoom_factor();
    let automation = app.automation_input;
    // eframe forwards Windows Raw Input, XI2 and Wayland relative motion. Unit harnesses also
    // exercise that path on macOS; the production Mac collector retains its own provenance.
    let raw = cfg!(any(target_os = "windows", target_os = "linux", test))
        && !automation
        && !touch
        && app.stylus.feed.get().is_none()
        && app.services.motion_samples.is_none();
    app.brush_input.raw_frame_time = raw.then_some(frame_time);
    if !raw {
        app.brush_input.window_motion.reset();
    }
    let motion = &mut app.services.motion_samples;
    let samples = app.brush_input.events_with_motion(&events, xf, eligible, secondary, (pressure, mods), |event| {
        if automation {
            return Vec::new();
        }
        motion.as_mut().map_or_else(Vec::new, |read| read(event, zoom))
    });
    app.brush_input.defer_preview = true;
    for (event, modifiers, erase) in samples {
        if matches!(event, ToolEvent::Up { .. }) {
            crate::canvas::feed_live_stroke(app);
        }
        if matches!(event, ToolEvent::Down { .. }) {
            // Preserve the secondary-button gesture priority from canvas_buttons.
            app.brush_resize_armed = erase && crate::brush_resize::is_right_gesture(crate::workspace_ui::sticky_mods(app, modifiers));
            app.secondary_erase = erase && !app.brush_resize_armed;
            app.brush_input.owner = app.session.active().map(|st| (st.doc.id, tool));
        }
        tool_event(app, event, modifiers);
        if matches!(event, ToolEvent::Up { .. }) {
            app.brush_input.owner = None;
        }
    }
    app.brush_input.defer_preview = false;
    crate::canvas::feed_live_stroke(app);
    true
}

/// Complete ink in its original document when a gesture loses its canvas or tool.
pub fn interrupt(app: &mut PhotocraftApp) {
    let owner = app.brush_input.owner.take();
    app.brush_input.button = None;
    app.brush_input.last = None;
    let transform = app.brush_input.transform.take();
    app.brush_input.defer_preview = false;
    app.brush_input.window_motion.reset();
    app.brush_input.raw_frame_time = None;
    app.brush_resize = None;
    let Some((doc, tool)) = owner else { return };
    let previous = app.session.active().map(|st| st.doc.id);
    let owner_index = app.session.documents().iter().position(|st| st.doc.id == doc);
    if let Some(index) = owner_index {
        app.session.set_active(index);
        let previous_zoom = app.ui.views.get(index).map(|view| view.zoom);
        if let Some(view) = app.ui.views.get_mut(index)
            && let Some(transform) = transform
        {
            view.zoom = transform.zoom;
        }
        crate::canvas::finish_brush_capture(app, tool);
        if let Some(view) = app.ui.views.get_mut(index)
            && let Some(zoom) = previous_zoom
        {
            view.zoom = zoom;
        }
        if let Some(index) = previous.and_then(|id| app.session.documents().iter().position(|st| st.doc.id == id)) {
            app.session.set_active(index);
        }
    } else {
        app.drag = None;
        app.live_stroke = None;
    }
}

pub fn sync_capture(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((doc, tool)) = app.brush_input.owner else { return };
    // Route the current batch first when it contains focus loss, retaining preceding motion.
    let unfocused = ctx.input(|i| !i.focused && !i.raw.events.iter().any(|event| matches!(event, Event::WindowFocused(false))));
    if app.session.active().is_none_or(|st| st.doc.id != doc) || app.ui.tool != tool || !app.ui.dialogs.is_empty() || unfocused {
        interrupt(app);
    }
}

pub fn sync_effective_tool(app: &mut PhotocraftApp, tool: Tool) {
    if app.brush_input.owner.is_some_and(|(_, owner)| owner != tool) {
        interrupt(app);
    }
}

impl BrushInput {
    #[cfg(test)]
    fn events(
        &mut self,
        events: &[Event],
        xf: &ViewXform,
        eligible: bool,
        secondary: bool,
        pressure: f32,
        mods: Modifiers,
    ) -> Vec<(ToolEvent, Modifiers, bool)> {
        self.events_with_motion(events, xf, eligible, secondary, (pressure, mods), |_| Vec::new())
    }

    fn events_with_motion(
        &mut self,
        events: &[Event],
        xf: &ViewXform,
        eligible: bool,
        secondary: bool,
        (mut pressure, mut mods): (f32, Modifiers),
        mut motion: impl FnMut(MouseMotion) -> Vec<egui::Pos2>,
    ) -> Vec<(ToolEvent, Modifiers, bool)> {
        let mut out = Vec::new();
        for (index, event) in events.iter().enumerate() {
            if matches!(event, Event::PointerButton { .. } | Event::WindowFocused(false) | Event::PointerGone) {
                self.window_motion.reset();
            }
            // Native records every press; synchronize ignored presses too, before hit testing.
            if let Event::PointerButton { pos, button, pressed: true, .. } = *event
                && matches!(button, PointerButton::Primary | PointerButton::Secondary)
            {
                motion(MouseMotion::Press { position: pos, button });
            }
            match *event {
                Event::PointerButton { pos, button, pressed: true, modifiers }
                    if self.button.is_none()
                        && eligible
                        && xf.rect.contains(pos)
                        && (button == PointerButton::Primary || button == PointerButton::Secondary && (modifiers.alt || secondary && !modifiers.command)) =>
                {
                    self.button = Some(button);
                    self.last = Some(pos);
                    self.transform = Some(*xf);
                    if let Some(time) = self.raw_frame_time {
                        self.window_motion.frame_endpoint([f64::from(pos.x), f64::from(pos.y)], time);
                    }
                    mods = modifiers;
                    let [x, y] = xf.to_doc(pos);
                    out.push((ToolEvent::Down { x, y, pressure }, mods, button == PointerButton::Secondary));
                }
                Event::PointerMoved(pos) if self.button.is_some() => {
                    let same_transform = self.transform == Some(*xf);
                    self.transform = Some(*xf);
                    let points = if let Some(time) = self.raw_frame_time {
                        // Wayland may deliver a raw batch before several absolute positions.
                        // With no timestamps, that batch cannot belong to the first endpoint.
                        let batched_positions = events[index + 1..]
                            .iter()
                            .find(|event| {
                                matches!(
                                    event,
                                    Event::MouseMoved(_)
                                        | Event::PointerMoved(_)
                                        | Event::PointerButton { .. }
                                        | Event::PointerGone
                                        | Event::WindowFocused(false)
                                )
                            })
                            .is_some_and(|event| matches!(event, Event::PointerMoved(_)));
                        if !same_transform || batched_positions {
                            self.window_motion.reset();
                        }
                        self.window_motion
                            .frame_endpoint([f64::from(pos.x), f64::from(pos.y)], time)
                            .into_iter()
                            .map(|p| egui::pos2(p[0] as f32, p[1] as f32))
                            .collect()
                    } else {
                        self.last.map_or_else(Vec::new, |from| motion(MouseMotion::Move { from, to: pos }))
                    };
                    for point in points.into_iter().take(64).filter(|p| same_transform && p.x.is_finite() && p.y.is_finite()) {
                        let [x, y] = xf.to_doc(point);
                        out.push((ToolEvent::Move { x, y, pressure }, mods, false));
                    }
                    self.last = Some(pos);
                    let [x, y] = xf.to_doc(pos);
                    out.push((ToolEvent::Move { x, y, pressure }, mods, false));
                }
                Event::PointerButton { pos, button, pressed: false, modifiers } if self.button == Some(button) => {
                    self.button = None;
                    let [x, y] = xf.to_doc(pos);
                    out.push((ToolEvent::Up { x, y }, modifiers, false));
                }
                Event::WindowFocused(false) if self.button.take().is_some() => {
                    if let Some(pos) = self.last {
                        let [x, y] = xf.to_doc(pos);
                        out.push((ToolEvent::Up { x, y }, mods, false));
                    }
                }
                Event::MouseMoved(delta) if self.button.is_some() && self.raw_frame_time.is_some() => {
                    // Device IDs are not forwarded by eframe. Never claim per-device correlation.
                    self.window_motion.push(0, [f64::from(delta.x), f64::from(delta.y)]);
                }
                Event::Touch { force: Some(force), .. } if force.is_finite() => pressure = force.clamp(0.0, 1.0),
                Event::Key { modifiers, .. } | Event::ModifiersChanged(modifiers) => mods = modifiers,
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, pos2};

    use super::*;

    #[test]
    fn ignored_gestures_cannot_contaminate_the_next_paint_stroke() {
        use egui_kittest::Harness;
        use photocraft_tablet::motion::Feed;
        use serde_json::json;
        let feed = Feed::default();
        let native = feed.clone();
        let services = crate::Services {
            motion_samples: Some(Box::new(move |event, _| {
                let point = |p: egui::Pos2| [f64::from(p.x), f64::from(p.y)];
                match event {
                    MouseMotion::Press { position, button } => {
                        native.begin(point(position), button as u8);
                        Vec::new()
                    }
                    MouseMotion::Move { from, to } => native.take(point(from), point(to)).into_iter().map(|p| pos2(p[0] as f32, p[1] as f32)).collect(),
                    MouseMotion::EndFrame => {
                        native.clear();
                        Vec::new()
                    }
                }
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width":128,"height":128})).unwrap();
        app.ui.tool = Tool::Brush;
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
        h.run_steps(3);
        let a = h.state().last_canvas_rect.center();
        let b = a + egui::vec2(20.0, 0.0);
        let p = |p: egui::Pos2| [f64::from(p.x), f64::from(p.y)];
        feed.press(p(a), 1);
        feed.publish(p(a), p(b), vec![p(a + egui::vec2(10.0, -10.0))]);
        feed.press(p(a), 0);
        feed.publish(p(a), p(b), vec![p(a + egui::vec2(10.0, 10.0))]);
        h.input_mut().events.extend([
            Event::PointerMoved(a),
            button(a, true, PointerButton::Secondary),
            Event::PointerMoved(b),
            button(b, false, PointerButton::Secondary),
            Event::PointerMoved(a),
            button(a, true, PointerButton::Primary),
            Event::PointerMoved(b),
            button(b, false, PointerButton::Primary),
        ]);
        h.step();
        let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
        assert_eq!(strokes.len(), 1);
        let points = strokes[0].1["points"].as_array().unwrap();
        assert_eq!(points.len(), 3);
        assert!(points[1][1].as_f64().unwrap() > points[0][1].as_f64().unwrap(), "only the current curve reaches painting");
        // An ignored primary press while another tool owns the canvas must expire this frame.
        h.state_mut().ui.tool = Tool::Move;
        feed.press(p(a), 0);
        feed.publish(p(a), p(b), vec![p(a + egui::vec2(10.0, -10.0))]);
        h.input_mut().events.extend([button(a, true, PointerButton::Primary), Event::PointerMoved(b), button(b, false, PointerButton::Primary)]);
        h.step();
        assert!(feed.take(p(a), p(b)).is_empty());
    }

    #[test]
    fn captured_resize_completes_without_painting() {
        use egui_kittest::Harness;
        use serde_json::json;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":128,"height":128})).unwrap();
        app.ui.tool = Tool::Brush;
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
        h.run_steps(3);
        let p = h.state().last_canvas_rect.center();
        let mods = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
        h.input_mut().events.extend([Event::PointerMoved(p), Event::PointerButton { pos: p, pressed: true, button: PointerButton::Primary, modifiers: mods }]);
        h.step();
        assert!(h.state().brush_resize.is_some());
        assert!(h.state().drag.is_none());
        h.input_mut().events.push(Event::PointerMoved(p + egui::vec2(20.0, 0.0)));
        h.step();
        h.input_mut().events.push(Event::PointerButton { pos: p + egui::vec2(20.0, 0.0), pressed: false, button: PointerButton::Primary, modifiers: mods });
        h.step();
        assert!(h.state().brush_resize.is_none());
        assert!(h.state().brush_input.button.is_none());
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    }

    #[test]
    fn interruption_commits_ink_to_owner_and_restores_current_document() {
        use serde_json::json;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":100,"height":100})).unwrap();
        let owner = app.session.active().unwrap().doc.id;
        app.ui.tool = Tool::Brush;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        app.brush_input.owner = Some((owner, Tool::Brush));
        app.brush_input.button = Some(PointerButton::Primary);
        app.brush_input.transform = Some(xf());
        app.session.execute("file.new", json!({"width":100,"height":100})).unwrap();
        let current = app.session.active().unwrap().doc.id;
        app.ui.tool = Tool::Hand;
        interrupt(&mut app);
        assert_eq!(app.session.active().unwrap().doc.id, current);
        assert_eq!(app.ui.tool, Tool::Hand);
        assert!(app.drag.is_none());
        assert!(app.brush_input.button.is_none());
        assert_eq!(app.session.journal.iter().filter(|(id, _)| id == "paint.stroke").count(), 1);
    }

    #[test]
    fn capture_without_paint_drag_survives_sync() {
        use serde_json::json;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":100,"height":100})).unwrap();
        app.ui.tool = Tool::Brush;
        app.brush_input.owner = Some((app.session.active().unwrap().doc.id, Tool::Brush));
        app.brush_input.button = Some(PointerButton::Primary);
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput { focused: true, ..Default::default() });
        sync_capture(&mut app, &ctx);
        ctx.end_pass().textures_delta.clear();
        assert!(app.brush_input.button.is_some());
        sync_effective_tool(&mut app, Tool::Hand);
        assert!(app.brush_input.button.is_none());
    }

    #[test]
    fn view_change_consumes_but_rejects_raw_interval() {
        let mut input = BrushInput::default();
        let mut transform = xf();
        let p = pos2(20.0, 20.0);
        input.events(&[button(p, true, PointerButton::Primary)], &transform, true, false, 1.0, Modifiers::NONE);
        transform.zoom = 3.0;
        let mut consumed = false;
        let out = input.events_with_motion(&[Event::PointerMoved(pos2(30.0, 30.0))], &transform, true, false, (1.0, Modifiers::NONE), |_| {
            consumed = true;
            vec![pos2(25.0, 50.0)]
        });
        assert!(consumed);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn production_motion_reaches_strokes_with_smoothing_and_every_depth() {
        use egui_kittest::Harness;
        use serde_json::json;
        for depth in [8, 16, 32] {
            for smoothing in [0.0, 0.1] {
                for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
                    let services = crate::Services {
                        motion_samples: Some(Box::new(|event, _| match event {
                            MouseMotion::Move { from, to } => vec![from.lerp(to, 0.5) + egui::vec2(0.0, 10.0)],
                            _ => Vec::new(),
                        })),
                        ..Default::default()
                    };
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
                    app.run("file.new", json!({"width":128,"height":128,"depth":depth,"background":"white"})).unwrap();
                    app.ui.tool = tool;
                    app.session.tools.brush.smoothing.amount = smoothing;
                    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
                    h.run_steps(3);
                    let p = h.state().last_canvas_rect.center();
                    let end = p + egui::vec2(20.0, 0.0);
                    h.input_mut().events.extend([
                        Event::PointerMoved(p),
                        button(p, true, PointerButton::Primary),
                        Event::PointerMoved(end),
                        button(end, false, PointerButton::Primary),
                    ]);
                    h.step();
                    let command = if tool == Tool::Pencil { "paint.pencil" } else { "paint.stroke" };
                    let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == command).collect();
                    assert_eq!(strokes.len(), 1, "{tool:?}, depth {depth}, smoothing {smoothing}");
                    let points = strokes[0].1["points"].as_array().unwrap();
                    assert_eq!(points.len(), 3);
                    assert_ne!(points[1][1], points[2][1], "interior curve survives input routing");
                }
            }
        }
    }

    fn xf() -> ViewXform {
        ViewXform { rect: Rect::from_min_max(Pos2::ZERO, pos2(100.0, 100.0)), zoom: 2.0, center: [50.0; 2], flip: true }
    }

    fn button(pos: egui::Pos2, pressed: bool, button: PointerButton) -> Event {
        Event::PointerButton { pos, pressed, button, modifiers: Modifiers::NONE }
    }

    #[test]
    fn preserves_fast_curve_and_press_release_in_one_frame() {
        let p = pos2(20.0, 20.0);
        let events = [
            button(p, true, PointerButton::Primary),
            Event::PointerMoved(pos2(40.0, 80.0)),
            Event::PointerMoved(pos2(80.0, 20.0)),
            button(p, false, PointerButton::Primary),
        ];
        let mut input = BrushInput::default();
        let out = input.events(&events, &xf(), true, false, 0.4, Modifiers::NONE);
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].0, ToolEvent::Move { x: 55.0, y: 65.0, pressure: 0.4 });
        assert!(input.button.is_none());
    }

    #[test]
    fn only_captures_canvas_presses_and_matching_releases() {
        let p = pos2(20.0, 20.0);
        let events = [button(p, true, PointerButton::Primary), Event::PointerMoved(p)];
        let mut input = BrushInput::default();
        assert!(input.events(&events, &xf(), false, false, 1.0, Modifiers::NONE).is_empty());
        let right = [button(p, true, PointerButton::Secondary)];
        assert!(input.events(&right, &xf(), true, false, 1.0, Modifiers::NONE).is_empty());
        assert_eq!(input.events(&right, &xf(), true, true, 1.0, Modifiers::NONE).len(), 1);
        // Captured strokes continue beyond the canvas; the other button cannot finish them.
        let outside = [Event::PointerMoved(pos2(200.0, 200.0)), button(p, false, PointerButton::Primary)];
        assert_eq!(input.events(&outside, &xf(), false, true, 1.0, Modifiers::NONE).len(), 1);
        assert!(input.button.is_some());
        input.events(&[button(p, false, PointerButton::Secondary)], &xf(), false, true, 1.0, Modifiers::NONE);
        assert!(input.button.is_none());
    }

    #[test]
    fn focus_loss_finishes_capture() {
        let p = pos2(20.0, 20.0);
        let mut input = BrushInput::default();
        let events = [button(p, true, PointerButton::Primary), Event::PointerMoved(p), Event::WindowFocused(false)];
        let out = input.events(&events, &xf(), true, false, 1.0, Modifiers::NONE);
        assert!(matches!(out.last().unwrap().0, ToolEvent::Up { .. }));
        assert!(input.button.is_none());
    }

    #[test]
    fn touch_force_stays_in_event_order() {
        let p = pos2(20.0, 20.0);
        let touch = |force| Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(0), phase: egui::TouchPhase::Move, pos: p, force: Some(force) };
        let events = [button(p, true, PointerButton::Primary), touch(0.2), Event::PointerMoved(p), touch(0.8), Event::PointerMoved(p)];
        let out = BrushInput::default().events(&events, &xf(), true, false, 1.0, Modifiers::NONE);
        assert!(matches!(out[1].0, ToolEvent::Move { pressure: 0.2, .. }));
        assert!(matches!(out[2].0, ToolEvent::Move { pressure: 0.8, .. }));
    }

    #[test]
    fn disabled_tablet_pressure_stays_disabled_with_batched_input() {
        use egui_kittest::Harness;
        use serde_json::json;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 128, "height": 128})).unwrap();
        app.run("prefs.set", json!({"path":"tools.useTabletPressure", "value":false})).unwrap();
        app.ui.tool = Tool::Brush;
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
        h.run_steps(3);
        let p = h.state().last_canvas_rect.center();
        h.input_mut().events.extend([
            Event::PointerMoved(p),
            button(p, true, PointerButton::Primary),
            Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(0), phase: egui::TouchPhase::Move, pos: p, force: Some(0.2) },
            Event::PointerMoved(p + egui::vec2(20.0, 0.0)),
            button(p + egui::vec2(20.0, 0.0), false, PointerButton::Primary),
        ]);
        h.step();
        let points = &h.state().session.journal.iter().find(|(id, _)| id == "paint.stroke").unwrap().1["points"];
        assert!(points.as_array().unwrap().iter().all(|p| p[2] == 1.0));
    }

    #[test]
    fn real_canvas_preserves_batched_curve_and_one_undo_step() {
        use egui_kittest::Harness;
        use serde_json::json;
        for _ in 0..1 {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 256, "height": 256, "background": "transparent"})).unwrap();
            app.ui.tool = Tool::Brush;
            app.session.tools.brush.size = 6.0;
            app.session.tools.brush.smoothing.amount = 0.0;
            let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
            h.run_steps(3);
            let c = h.state().last_canvas_rect.center();
            let a = c - egui::vec2(60.0, 0.0);
            h.event(Event::PointerMoved(a));
            h.run_steps(1);
            h.event(button(a, true, PointerButton::Primary));
            h.run_steps(1);
            for p in [c + egui::vec2(-30.0, 50.0), c + egui::vec2(30.0, -50.0), c + egui::vec2(60.0, 0.0)] {
                h.input_mut().events.push(Event::PointerMoved(p));
            }
            h.run_steps(1);
            h.event(button(c + egui::vec2(60.0, 0.0), false, PointerButton::Primary));
            h.run_steps(1);
            let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
            assert_eq!(strokes.len(), 1, "one undo step");
            assert_eq!(strokes[0].1["points"].as_array().unwrap().len(), 4);
        }
    }
}

#[cfg(test)]
#[path = "brush_input/window_motion_tests.rs"]
mod window_motion_tests;
