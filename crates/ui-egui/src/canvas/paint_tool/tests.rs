//! The paint tool through the canvas test driver: the live stroke a drag shows, what the release
//! commits, the Mixer Brush's cancel and release rules, and the presses the tool refuses.

use crate::app::framework::BuildApp as _;
use craft::ui::canvas::RasterSource;
use craft::ui::primitives::Revision;
use egui::{Pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::Document;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::canvas::{CompositePrep, ToolEvent, canvas_display, damage_since, display_doc, tool_event};
use crate::state::Tool;

const M: egui::Modifiers = egui::Modifiers::NONE;

/// The revision of document `idx`'s canvas composite on `output` this pass, recorded as shown, as a display draws it.
fn composite_revision(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>) -> Revision {
    let prep = CompositePrep::new(app, ctx, idx, output, None).expect("an open document");
    prep.source(app).revision()
}

/// How many document pixels document 0's canvas composite redraws since it showed `seen`; `None` for all of them.
fn redrawn_since(app: &mut PhotocraftApp, ctx: &egui::Context, seen: Revision) -> Option<i64> {
    let prep = CompositePrep::new(app, ctx, 0, None, None).expect("an open document");
    prep.source(app).damage(seen).map(|r| i64::from(r.width()) * i64::from(r.height()))
}

/// The messages of the open alerts.
fn errors(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// The stroke the live paint gesture holds, as the points the engine's commit gets.
fn stroke_points(app: &PhotocraftApp) -> Vec<photocraft_paint::StrokePoint> {
    super::stroking(app).expect("a live gesture").points.iter().map(|p| photocraft_paint::StrokePoint::new(p[0], p[1], p[2] as f32)).collect()
}

/// The live trail's coverage reader.
fn trail(app: &PhotocraftApp) -> Option<&crate::tools::stroke_trail::Trail> {
    app.trail.as_ref()
}

/// A whole-document canvas at 100 % in a harness, for tests that feed several moves in one pass.
fn harness(mut app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).with_step_dt(1.0 / 60.0).build_app(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [60.0, 30.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

fn at(h: &Harness<'static, Box<PhotocraftApp>>, x: f64, y: f64) -> Pos2 {
    testing::to_screen(h.state(), [x, y])
}

fn mixer_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"size":12,"pressureSize":false})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    app
}

#[test]
fn smoothing_preview_redraws_the_tail_each_step() {
    // The tail drawn at one step must not linger once the brush moves on: a sharp turn would
    // leave a stale tail behind if it weren't restored.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width": 200, "height": 120, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0, "smoothing": {"amount": 0.6}}})).unwrap();
    app.ui.tool = Tool::Brush;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, M);
    for (x, y) in [(60.0, 20.0), (110.0, 20.0), (110.0, 70.0), (110.0, 110.0), (60.0, 110.0)] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, M);
    }
    let live = display_doc(&mut app, 0).0;
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 110.0 }, M);
    let done = app.session.documents()[0].doc.clone();
    let (a, b) = (live.layers[0].surface().unwrap(), done.layers[0].surface().unwrap());
    assert!((0..120).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y))), "no stale tails in the preview");
}

#[test]
fn retouch_drags_show_the_footprint_without_wedges() {
    // The retouching tools' trail is a mask of the footprint, with nothing outside it, however
    // small the steps. The Remove Tool's is where the stroke's dabs reach at all, which is what it
    // removes, so a soft tip's whole reach past the radius is tinted.
    for hardness in [1.0, 0.0] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        app.ui.tool = Tool::Remove;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 30, "hardness": hardness}})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
        for i in 1..80 {
            let t = f64::from(i) * 0.5;
            tool_event(&mut app, ToolEvent::Move { x: 20.0 + t, y: 30.0 + (t / 2.0).sin() * 3.0, pressure: 1.0 }, M);
        }
        // The stroke the removal gets on release, and the pixels its hole takes.
        let points = stroke_points(&app);
        let brush = photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({}));
        let (b, cov) = photocraft_paint::retouch::stroke_coverage(&photocraft_paint::Stroke { brush, points });
        let shown = trail(&app).expect("the trail shows while dragging");
        assert_eq!(shown.scale(), 1.0);
        let mut beyond = 0;
        for y in 0..60 {
            for x in 0..120 {
                let removed = b.contains(x, y) && cov[(y - b.y0) as usize * b.width() as usize + (x - b.x0) as usize] > 0.0;
                assert_eq!(shown.coverage(x as usize, y as usize), if removed { 255 } else { 0 }, "hardness {hardness}: ({x}, {y})");
                // Past the 15 px radius of the path's ends and sides (y 27..33).
                beyond += usize::from(removed && !(12..=48).contains(&y));
            }
        }
        if hardness == 0.0 {
            assert!(beyond > 100, "a soft tip's removal reaches past its radius: {beyond} px");
        }
        tool_event(&mut app, ToolEvent::Up { x: 59.5, y: 30.0 }, M);
        assert!(trail(&app).is_none(), "the trail ends with the drag");
    }
}

#[test]
fn spot_healing_trail_is_the_coverage_the_heal_uses() {
    // #2364: a soft brush with pen-pressure size showed as a hard footprint at full size.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
    let brush = json!({"size": 30, "hardness": 0.0, "shapeDynamics": {"enabled": true, "size": {"control": "penPressure"}}});
    app.run("tools.setBrush", json!({ "brush": brush })).unwrap();
    app.ui.tool = Tool::SpotHealing;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 0.5 }, M);
    for x in [40.0, 60.0, 80.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 0.5 }, M);
    }
    // The stroke the heal gets on release, and its coverage.
    let points = stroke_points(&app);
    let brush = photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({}));
    let (b, cov) = photocraft_paint::retouch::stroke_coverage(&photocraft_paint::Stroke { brush, points });
    let shown = trail(&app).expect("the trail shows while dragging");
    let mut soft = 0;
    for y in 0..60 {
        for x in 0..120 {
            let want = if b.contains(x, y) { cov[(y - b.y0) as usize * b.width() as usize + (x - b.x0) as usize] } else { 0.0 };
            let got = shown.coverage(x as usize, y as usize);
            assert!((f32::from(got) - want * 255.0).abs() <= 1.0, "({x}, {y}): trail {got}, heal {want}");
            soft += usize::from(got > 0 && got < 200);
        }
    }
    assert!(soft > 50, "a soft brush shows a soft edge");
    // Half pressure halves the 30 px tip: nothing past its (soft) reach.
    let reach = photocraft_paint::tip_reach(7.5, app.session.tools.brush.hardness).ceil() as usize + 2;
    assert_eq!(shown.coverage(50, 30 + reach), 0, "half pressure halves the 30 px tip");
}

/// The held drag shows true mixed paint while Alt sampling remains separate from painting.
#[test]
fn mixer_drag_shows_actual_mixed_paint_before_release_and_alt_never_paints() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":16,"mixer":{"wet":1.0,"mix":0.7,"sampleAllLayers":true},"smoothing":{"amount":0.0}}})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Move { x: 35.0, y: 30.0, pressure: 1.0 }, M);
    tool_event(&mut app, ToolEvent::Up { x: 35.0, y: 30.0 }, M);
    assert!(!testing::in_gesture(&app));
    assert!(app.session.tools.mixer.spatial.is_some());
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    app.run("brush.mixer.load", json!({})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 30.0, pressure: 0.6 }, M);
    assert!(app.live_stroke.is_some());
    let shown = display_doc(&mut app, 0).0;
    let id = app.session.active().unwrap().active_layer.unwrap();
    let pixel = shown.layer(id).unwrap().surface().unwrap().rgba(45, 30);
    assert!(pixel[0] > 0.0 && pixel[1] > 0.0 && pixel[3] > 0.0, "actual red/green mixture {pixel:?}");
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), count);
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, M);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), count + 1);
    assert_eq!(
        shown.layer(id).unwrap().surface().unwrap().read_region(before.bounds()),
        st.doc.layer(id).unwrap().surface().unwrap().read_region(before.bounds())
    );
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(45, 30)[3], 0.0);
}

/// Several moves of one pass and the release point all reach the commit, in the live path and in
/// the fallback after a failed preview. The paint tool feeds the live stroke once per pass.
#[test]
fn mixer_release_includes_a_new_endpoint_and_deferred_frame_samples() {
    for failed in [false, true] {
        let mut app = mixer_app();
        // Smoothing off: this is about which points reach the commit, and the Mixer Brush's
        // first-use 10 % lag (measured, #11888) would hold the brush short of the release point.
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"smoothing": {"amount": 0.0}}})).unwrap();
        let mut h = harness(app);
        let before = h.state().session.active().unwrap().doc.clone();
        let p = |h: &Harness<'static, Box<PhotocraftApp>>, x: f64| at(h, x, 30.0);
        let start = p(&h, 20.0);
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: M });
        h.run_steps(1);
        if failed {
            h.state_mut().live_stroke.as_mut().expect("a live preview").preview_failed = true;
        }
        // Three moves in one pass, then the release at a new endpoint in the next pass.
        for x in [35.0, 50.0, 65.0] {
            let q = p(&h, x);
            h.input_mut().events.push(egui::Event::PointerMoved(q));
        }
        h.run_steps(1);
        let end = p(&h, 95.0);
        h.input_mut().events.push(egui::Event::PointerMoved(end));
        h.input_mut().events.push(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: M });
        h.run_steps(3);
        let app = h.state_mut();
        let doc = app.session.active().unwrap().doc.clone();
        let id = app.session.active().unwrap().active_layer.unwrap();
        assert!(doc.layer(id).unwrap().surface().unwrap().rgba(92, 30)[3] > 0.0, "failed {failed}: the release point is painted");
        let params = app.session.journal.iter().rev().find(|(name, _)| name == "paint.mixerBrush").unwrap().1.clone();
        // The live commit journals the engine's points (objects); the fallback journals the stroke's own ([x, y, ...]).
        let xs: Vec<f64> = params["points"].as_array().unwrap().iter().map(|q| q["x"].as_f64().or_else(|| q[0].as_f64()).expect("a point has an x")).collect();
        for x in [35.0, 50.0, 65.0, 95.0] {
            assert!(xs.iter().any(|v| (v - x).abs() < 1.0), "failed {failed}: the point at {x} is in the commit: {xs:?}");
        }
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("paint.mixerBrush", params).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *doc, "failed {failed}");
    }
}

/// A tool change and Escape (the host cancels the gesture) drop the Mixer Brush's preview, in the
/// live path and after a failed preview, with no edit.
#[test]
fn mixer_cancel_without_an_edit() {
    for mode in 0..4 {
        let mut app = mixer_app();
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, M);
        assert!(app.live_stroke.is_some());
        if mode >= 2 {
            app.live_stroke.as_mut().unwrap().preview_failed = true;
        }
        if mode % 2 == 0 {
            testing::key(&mut app, egui::Key::Escape, M);
        } else {
            app.ui.tool = Tool::Brush;
            testing::frame(&mut app);
        }
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, M);
        assert!(!testing::in_gesture(&app) && app.live_stroke.is_none(), "mode {mode}");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "mode {mode}");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "mode {mode}");
        assert!(!app.session.journal.iter().any(|(id, _)| id == "paint.mixerBrush"), "mode {mode}");
    }
}

/// Control-driven strokes remain live in a background window, matching the ordinary Brush.
#[test]
fn mixer_background_window_keeps_and_commits_control_strokes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":60,"background":"transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":12,"pressureSize":false}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let mut h = harness(app);
    let ctx = h.ctx.clone();
    let pointer = |h: &mut Harness<'static, Box<PhotocraftApp>>, events: serde_json::Value| {
        let (request, _reply) = crate::control::ControlRequest::new("ui.pointer", json!({"tool":"mixerBrush","events": events}));
        let _ = crate::control::handle(h.state_mut(), &ctx, &request);
        // Every pass of the unfocused window runs the queued input.
        for _ in 0..4 {
            h.input_mut().focused = false;
            h.run_steps(1);
        }
    };
    pointer(&mut h, json!([{"kind":"down","x":20,"y":30},{"kind":"move","x":60,"y":30}]));
    assert!(testing::in_gesture(h.state_mut()), "the unfocused window keeps the stroke");
    pointer(&mut h, json!([{"kind":"up","x":95,"y":30}]));
    let app = h.state();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert!(app.session.journal.iter().any(|(id, _)| id == "paint.mixerBrush"));
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, 30)[3] > 0.0);
}

/// A tip above the live preview's 1024 px paints on release, as main and Photoshop paint it
/// up to 5000 px; a stroke that can't paint at all is still refused on press.
#[test]
fn mixer_drag_above_the_preview_size_paints_on_release() {
    for locked in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        if locked {
            app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
        }
        app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":2000,"hardness":1.0,"pressureSize":false,"mixer":{"wet":0.0,"mix":0.0}}})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        let steps = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, M);
        assert!(app.live_stroke.is_none(), "no live preview above 1024 px");
        assert_eq!(testing::in_gesture(&app), !locked, "a valid 2000 px stroke is kept for release");
        assert_eq!(app.ui.status_error, locked, "{}", app.ui.status);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, M);
        tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 30.0 }, M);
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + usize::from(!locked));
        let pixel = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, 30);
        assert_eq!(pixel[0] > 0.9 && pixel[3] > 0.9, !locked, "{pixel:?}");
    }
}

#[test]
fn brush_drag_shows_the_real_stroke_and_commits_it() {
    // The drag shows the real soft stroke, not a hard flat stand-in.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    let alpha = |d: &Document, x, y| d.layers[0].surface().unwrap().rgba(x, y)[3];
    let rev = app.session.documents()[0].revision;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, M);
    for x in [40.0, 70.0, 100.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, M);
    }
    let (shown, key) = display_doc(&mut app, 0);
    assert_ne!(key, 0);
    // A partial value two thirds of the way out: the soft tip, not a hard flat stand-in.
    assert!(alpha(&shown, 40, 30) > 0.5 && (0.01..0.95).contains(&alpha(&shown, 40, 36)), "soft stroke while drawing");
    assert_eq!(alpha(&app.session.documents()[0].doc, 40, 30), 0.0, "not committed yet");
    // The canvas redraws only what the stroke touched.
    let dk = canvas_display(&app, &app.session.documents()[0].doc, None).1;
    let d = damage_since(&app, 0, (rev, dk), (rev, key), dk, (rev, None)).unwrap();
    assert!(d.contains(40, 30) && !d.contains(40, 2) && d.height() < 60, "{d:?}");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, M);
    let doc = app.session.documents()[0].doc.clone();
    assert!(app.live_stroke.is_none() && display_doc(&mut app, 0).1 == 0);
    assert_eq!((alpha(&doc, 40, 30), alpha(&doc, 40, 38)), (alpha(&shown, 40, 30), alpha(&shown, 40, 38)), "commit matches the preview");
}

#[test]
fn live_stroke_uses_pen_tilt_and_redraws_partially() {
    // Pen tilt drives the size; the preview must use the tilt the commit gets, and each step
    // redraws only the pixels it touched.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 4097, "height": 90, "background": "transparent"})).unwrap();
    // Smoothing off: the default 10 % lag (measured, #11888) on wall-clock times would leave the
    // preview's tail anywhere short of the release, so the comparison below couldn't hold.
    let tilt =
        json!({"size": 30, "hardness": 1.0, "spacing": 0.05, "smoothing": {"amount": 0.0}, "shapeDynamics": {"enabled": true, "size": {"control": "penTilt"}}});
    app.run("tools.setBrush", json!({ "brush": tilt })).unwrap();
    app.ui.tool = Tool::Brush;
    let mut seen = composite_revision(&mut app, &ctx, 0, None);
    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: 1.0, tilt_x: 60.0, tilt_y: 0.0, rotation: 0.0, eraser: false }));
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 45.0, pressure: 1.0 }, M);
    for x in [300.0, 600.0, 900.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 45.0, pressure: 1.0 }, M);
        let px = redrawn_since(&mut app, &ctx, seen);
        assert!(px.is_some_and(|px| px < 4097 * 90 / 4), "step to {x} redraws {px:?} px");
        seen = composite_revision(&mut app, &ctx, 0, None);
    }
    let shown = display_doc(&mut app, 0).0;
    tool_event(&mut app, ToolEvent::Up { x: 900.0, y: 45.0 }, M);
    assert!(redrawn_since(&mut app, &ctx, seen).is_some(), "the commit redraws only the stroke");
    let doc = app.session.documents()[0].doc.clone();
    let (a, b) = (doc.layers[0].surface().unwrap(), shown.layers[0].surface().unwrap());
    // 60° tilt shrinks the dab well below 30 px: the preview shows that size, as committed.
    assert!(a.rgba(500, 45)[3] > 0.5 && a.rgba(500, 45 + 12)[3] == 0.0);
    assert!((0..90).all(|y| (0..700).all(|x| a.rgba(x, y) == b.rgba(x, y))), "commit matches the preview");
}

/// Brush drag along y = 40 with one canvas frame per pointer move; returns the document the
/// canvas showed at the last move, the committed one, and whether the release refreshed only
/// the stroke's rectangle.
fn drag_frames(smoothing: f32, xs: &[f64]) -> (std::sync::Arc<Document>, std::sync::Arc<Document>, bool, Box<PhotocraftApp>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5}})).unwrap();
    // What the options bar's Smoothing field writes.
    app.session.tools.brush.smoothing.amount = smoothing;
    // Catch-up on Stroke End starts off (#11887); these tests draw the catch-up tail.
    app.session.tools.brush.smoothing.catch_up_on_end = true;
    app.ui.tool = Tool::Brush;
    let mut seen = composite_revision(&mut app, &ctx, 0, None);
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, M);
    for &x in xs {
        tool_event(&mut app, ToolEvent::Move { x, y: 40.0 + (x / 7.0).sin() * 8.0, pressure: 1.0 }, M);
        seen = composite_revision(&mut app, &ctx, 0, None);
    }
    let live = display_doc(&mut app, 0).0;
    let last = *xs.last().unwrap();
    tool_event(&mut app, ToolEvent::Up { x: last, y: 40.0 + (last / 7.0).sin() * 8.0 }, M);
    let (next, key) = display_doc(&mut app, 0);
    assert_eq!(key, 0, "the frame after release shows the committed document");
    let partial = redrawn_since(&mut app, &ctx, seen).is_some();
    (live, next, partial, app)
}

fn same_pixels(a: &Document, b: &Document) -> bool {
    let (a, b) = (a.layers[0].surface().unwrap(), b.layers[0].surface().unwrap());
    (0..80).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y)))
}

#[test]
fn release_shows_nothing_new_without_smoothing() {
    // #73: at 0 % the stroke follows the pointer; the last frame drawn while dragging is
    // exactly the committed stroke, so release changes nothing on screen.
    let (live, done, partial, app) = drag_frames(0.0, &[40.0, 70.0, 100.0, 130.0, 160.0]);
    assert!(same_pixels(&live, &done), "preview at the last move = committed stroke");
    assert!(partial, "the handover refreshes only the stroke");
    let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    assert!(p.get("smoothing").is_none(), "the commit uses the session brush's smoothing, not a hard-coded one");
}

#[test]
fn smoothed_stroke_shows_its_catch_up_tail_while_drawing() {
    // #73: with smoothing the brush lags behind the pointer and catches up at the end; the
    // preview draws that tail live, so no frame after release is missing the end.
    let xs = [30.0, 50.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0];
    let (live, done, partial, _) = drag_frames(0.5, &xs);
    assert!(same_pixels(&live, &done), "preview at the last move = committed stroke, tail included");
    assert!(partial);
    let end = |d: &Document| (0..200).rev().find(|&x| (0..80).any(|y| d.layers[0].surface().unwrap().rgba(x, y)[3] > 0.0)).unwrap();
    assert!(end(&live) >= 170, "the end reaches the pointer while drawing ({})", end(&live));
    // The options-bar value reaches the stroke: 50 % smooths the wiggle, 0 % doesn't.
    let (_, rough, _, _) = drag_frames(0.0, &xs);
    assert!(!same_pixels(&rough, &done), "smoothing changes the stroke");
    // And the commit is what `paint.stroke` gives with that smoothing.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5, "smoothing": {"amount": 0.5, "catchUpOnEnd": true}}})).unwrap();
    let mut pts = vec![json!([10.0, 40.0, 1.0])];
    pts.extend(xs.iter().map(|&x| json!([x, 40.0 + (x / 7.0).sin() * 8.0, 1.0])));
    // (Round tips without dynamics draw the same for any seed.)
    s.execute("paint.stroke", json!({"points": pts, "seed": 0})).unwrap();
    // The drag reaches the tool through the pointer, as f32 screen points, so its points carry f32 rounding that
    // these f64 points don't: the two strokes may differ by one 8-bit level where a dab edge falls.
    assert!(within_a_level(&s.active().unwrap().doc, &done));
}

/// Whether `a` and `b` differ by at most one 8-bit level in every channel of every pixel.
fn within_a_level(a: &Document, b: &Document) -> bool {
    let (a, b) = (a.layers[0].surface().unwrap(), b.layers[0].surface().unwrap());
    (0..80).all(|y| (0..200).all(|x| a.rgba(x, y).iter().zip(b.rgba(x, y)).all(|(p, q)| (p - q).abs() <= 1.0 / 255.0 + 1e-6)))
}

/// #10074: painting a group looked as if it worked (the stroke's trail followed the pointer) and
/// vanished on release with only a status-bar note. Photoshop refuses at the press with a
/// message; so does PhotoCraft, and nothing is painted or drawn.
#[test]
fn painting_a_group_is_refused_at_the_press_with_a_message() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().active_layer.unwrap();
    assert!(app.session.active().unwrap().doc.layer(group).unwrap().is_group());
    app.ui.tool_options.gradient_classic = true;
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::CloneStamp, Tool::PaintBucket, Tool::MagicEraser, Tool::RedEye, Tool::Gradient] {
        let seen = errors(&app).len();
        app.ui.tool = tool;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, M);
        // Brush and Classic gradient use the measured alerts (#11898, #12017).
        let want = match tool {
            Tool::Brush => "Could not use the brush tool because the pixels in a group cannot be edited without merging the group.".to_string(),
            Tool::Gradient => "Could not use the gradient tool because the pixels in a group cannot be edited without merging the group.".to_string(),
            _ => format!("Could not use the {} because the target layer is a group.", tool.label()),
        };
        assert_eq!(errors(&app)[seen..], [want], "{tool:?}");
        assert!(!testing::in_gesture(&app) && app.live_stroke.is_none(), "{tool:?} starts no stroke");
        // The alert is modal: the next tool's press reaches the canvas once it is answered.
        let ctx = testing::ctx(&app);
        for id in app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).collect::<Vec<_>>() {
            let _ = crate::dialogs::cancel(&mut app, &ctx, id);
        }
        testing::frame(&mut app);
        tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 10.0, pressure: 1.0 }, M);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, M);
        assert!(trail(&app).is_none(), "{tool:?} draws no trail");
    }
    // Tools that don't paint the group's pixels are unaffected.
    let seen = errors(&app).len();
    app.ui.tool = Tool::RectMarquee;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, M);
    assert_eq!(errors(&app).len(), seen);
    tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 15.0, pressure: 1.0 }, M);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 15.0 }, M);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "a marquee still selects with a group active");
    app.run("select.deselect", json!({})).unwrap();
    // A group's mask can be painted: targeting it paints, without the message.
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.sync_mask_targets();
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"));
    let mask_at = |app: &PhotocraftApp, x: i32| {
        let st = app.session.active().unwrap();
        st.doc.layer(group).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, 10, 0)
    };
    let before = mask_at(&app, 15);
    // Black on a reveal-all mask hides: a stroke that paints shows.
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.ui.tool = Tool::Brush;
    let seen = errors(&app).len();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, M);
    assert_eq!(errors(&app).len(), seen, "the mask is paintable");
    assert!(testing::in_gesture(&app), "the stroke starts on the mask");
    tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 10.0, pressure: 1.0 }, M);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, M);
    assert_ne!(mask_at(&app, 15), before, "the stroke painted the group's mask");
}
