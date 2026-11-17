//! ⌥-click through the real canvas: the Brush's temporary Eyedropper (#1659) and the Clone
//! Stamp's source point and painting (#1658), the Zoom tool's Zoom Out (#3137).

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;
use craft::keymap::Mods;

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );

    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 50.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

/// Latches Alt in Window › Modifier Keys, or releases every latched key.
fn latch_alt(h: &mut Harness<'static, PhotocraftApp>, on: bool) {
    h.state_mut().ui.shell.latched = if on { Mods { alt: true, ..Mods::default() } } else { Mods::default() };
}

fn mods(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(1);
}

/// A click at document point (x, y) with `m` held from before the press until after the release.
fn click(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    mods(h, m);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    button(h, p, false, m);
    h.run_steps(1);
    mods(h, Modifiers::NONE);
}

/// A drag between document points with `m` held throughout.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2], m: Modifiers) {
    let p = screen(h, from[0], from[1]);
    mods(h, m);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        let q = screen(h, from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    button(h, screen(h, to[0], to[1]), false, m);
    h.run_steps(2);
    mods(h, Modifiers::NONE);
}

/// Two halves: red on the left, green on the right, on one background layer.
fn two_colours() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app
}

/// Three handlers compete for one ⌥ press (the brush's sampler, the clone source, the zoom step, the
/// mixer's load): which one takes it depends on the tool, and a latched ⌥ acts as a held one.
#[test]
fn alt_click_routes_by_tool_held_or_latched() {
    for latched in [false, true] {
        // The click carries ⌥ itself, or the latch supplies it.
        let m = if latched { Modifiers::NONE } else { Modifiers::ALT };
        let how = if latched { "latched" } else { "held" };

        // Brush: samples the foreground, paints nothing.
        let mut h = harness(two_colours());
        h.state_mut().ui.tool = Tool::Brush;
        h.state_mut().run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
        let rev = h.state().session.active().unwrap().revision;
        latch_alt(&mut h, latched);
        click(&mut h, 50.0, 50.0, m);
        assert_eq!(h.state().session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "Brush {how}: sampled red");
        assert_eq!(h.state().session.active().unwrap().revision, rev, "Brush {how}: nothing was painted");
        click(&mut h, 150.0, 50.0, m);
        assert_eq!(h.state().session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "Brush {how}: sampled green");

        // Clone Stamp: sets the source.
        let mut h = harness(two_colours());
        h.state_mut().ui.tool = Tool::CloneStamp;
        latch_alt(&mut h, latched);
        click(&mut h, 150.0, 50.0, m);
        assert!(h.state().session.presets.clone.active().source.is_some(), "Clone Stamp {how}: the source is set");

        // Zoom: zooms out (scrubby on and off for the latched key), and zooms in again without ⌥.
        for scrubby in [true, false] {
            if !latched && !scrubby {
                continue;
            }
            let mut h = harness(two_colours());
            h.state_mut().ui.tool = Tool::Zoom;
            h.state_mut().ui.tool_options.zoom_scrubby = scrubby;
            latch_alt(&mut h, latched);
            click(&mut h, 50.0, 50.0, m);
            assert_eq!(h.state().ui.views[0].zoom, 2.0 / 3.0, "Zoom {how} scrubby={scrubby}");
            latch_alt(&mut h, false);
            click(&mut h, 50.0, 50.0, Modifiers::NONE);
            assert_eq!(h.state().ui.views[0].zoom, 1.0, "Zoom {how} scrubby={scrubby}: without ⌥ it zooms in");
        }

        // Mixer Brush: loads the reservoir and keeps the foreground.
        let mut h = harness(two_colours());
        h.state_mut().ui.tool = Tool::MixerBrush;
        h.state_mut().run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
        let rev = h.state().session.active().unwrap().revision;
        assert!(h.state().session.tools.mixer.reservoir.is_none());
        latch_alt(&mut h, latched);
        click(&mut h, 50.0, 50.0, m);
        assert_eq!(h.state().session.tools.mixer.reservoir, Some([1.0, 0.0, 0.0, 1.0]), "Mixer {how}: loaded red");
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 1.0, 1.0], "Mixer {how}: the foreground is unchanged");
        assert_eq!(h.state().session.active().unwrap().revision, rev, "Mixer {how}: nothing was painted");
        click(&mut h, 150.0, 50.0, m);
        assert_eq!(h.state().session.tools.mixer.reservoir, Some([0.0, 1.0, 0.0, 1.0]), "Mixer {how}: loaded green");
    }
}

/// Alt-click sampling respects the canvas view transform without changing document pixels.
#[test]
fn mixer_alt_click_samples_the_tip_through_zoomed_rotated_canvas_input() {
    for (zoom, rotation) in [(0.5, 0.0), (1.0, 0.0), (2.0, 30.0)] {
        let mut app = two_colours();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run(
            "tools.setBrush",
            json!({"brush":{"size":24,"roundness":0.4,"hardness":1.0,"pressureSize":false,"mixer":{"wet":0.0,"mix":0.0,"sampleAllLayers":true}}}),
        )
        .unwrap();
        let mut h = harness(app);
        h.state_mut().ui.tool = Tool::MixerBrush;
        h.state_mut().ui.views[0].zoom = zoom;
        h.state_mut().ui.views[0].rotation = rotation;
        h.run_steps(2);
        let before = h.state().session.active().unwrap().doc.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, 100.0, 50.0, Modifiers::ALT);
        let state = &h.state().session.tools.mixer;
        let load = state.spatial.as_ref().expect("Alt loads a spatial brush tip");
        assert!(load.iter().any(|c| c[0] > 0.9 && c[1] < 0.1 && c[3] > 0.9));
        assert!(load.iter().any(|c| c[1] > 0.9 && c[0] < 0.1 && c[3] > 0.9));
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
        click(&mut h, 40.0, 50.0, Modifiers::NONE);
        let st = h.state().session.active().unwrap();
        let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert!(surface.rgba(34, 50)[0] > 0.9 && surface.rgba(46, 50)[1] > 0.9, "loaded halves follow document coordinates at {zoom}x/{rotation}°");
        assert_eq!(st.history.past_len(), steps + 1);
        assert!(crate::alt_grab::alt_click_tool(Tool::MixerBrush, Modifiers::ALT));
        assert!(!crate::alt_grab::alt_click_tool(Tool::MixerBrush, Modifiers::ALT | Modifiers::CTRL));
    }
}

/// Native pointer release commits the shown mixer result before restoring a spring-loaded tool.
#[test]
fn mixer_raw_drag_commits_live_pixels_before_a_spring_tool_returns() {
    for spring in [false, true] {
        let mut app = two_colours();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#0000ff"})).unwrap();
        app.run(
            "tools.setBrush",
            json!({"brush":{"size":24,"pressureSize":false,"mixer":{"wet":1.0,"mix":0.5,"sampleAllLayers":true},"smoothing":{"amount":0.0}}}),
        )
        .unwrap();
        let mut h = harness(app);
        h.state_mut().ui.tool = Tool::MixerBrush;
        let before = h.state().session.active().unwrap().doc.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        let start = screen(&h, 40.0, 50.0);
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(1);
        button(&mut h, start, true, Modifiers::NONE);
        if spring {
            // A temporary Mixer Brush's key was released while its mouse drag is held.
            h.state_mut().ui.spring = Some(crate::spring_tools::Spring {
                previous: Tool::Zoom,
                tool: Tool::MixerBrush,
                keys: vec![],
                since: 0.0,
                up: None,
                used: true,
                released: true,
            });
        }
        let end = screen(&h, 80.0, 50.0);
        h.event(egui::Event::PointerMoved(end));
        h.run_steps(2);
        assert_eq!(h.state().ui.tool, Tool::MixerBrush);
        let shown = crate::canvas::display_doc(h.state_mut(), 0).0;
        let id = h.state().session.active().unwrap().active_layer.unwrap();
        let paint = shown.layer(id).unwrap().surface().unwrap();
        let mixed = paint.rgba(60, 50);
        assert!(mixed[0] > 0.0 && mixed[2] > 0.0 && mixed[3] > 0.0, "real red/blue mixing while held: {mixed:?}");
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
        button(&mut h, end, false, Modifiers::NONE);
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface().unwrap(), paint);
        assert_eq!(st.history.past_len(), steps + 1);
        assert_eq!(h.state().ui.tool, if spring { Tool::Zoom } else { Tool::MixerBrush });
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *before);
    }
}

/// ⌥-click in one open document, then paint in another: the stroke clones from the first, as in
/// Photoshop, rather than from the same point of the document being painted.
#[test]
fn alt_click_in_one_document_clones_into_another() {
    let mut app = two_colours();
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("document.activate", json!({"document": 0})).unwrap();
    app.sync_views();
    let mut h = harness(app);
    h.state_mut().ui.tool = Tool::CloneStamp;
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0}})).unwrap();
    let rev = h.state().session.active().unwrap().revision;
    click(&mut h, 150.0, 50.0, Modifiers::ALT);
    assert_eq!(h.state().session.active().unwrap().revision, rev, "⌥-click only sets the source");
    h.state_mut().run("document.activate", json!({"document": 1})).unwrap();
    h.state_mut().sync_views();
    let view = h.state().ui.views[0].clone();
    h.state_mut().ui.views[1] = view;
    h.run_steps(2);
    let source = h.state().session.documents()[0].doc.clone();
    drag(&mut h, [30.0, 50.0], [70.0, 50.0], Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(1));
    let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(50, 50);
    assert!(px[1] > 0.9 && px[0] < 0.1 && px[2] < 0.1, "green cloned from the other document over white: {px:?}");
    assert_eq!(*app.session.documents()[0].doc, *source, "the source document is only read");
}

#[test]
fn clone_stamp_paints_an_empty_layer_from_the_layers_below() {
    // The reported setup: a new empty layer on top, Sample: All Layers (or Current & Below).
    for sample in ["all", "currentAndBelow"] {
        for aligned in [true, false] {
            let mut app = two_colours();
            app.run("layer.new.layer", json!({})).unwrap();
            app.sync_views();
            let mut h = harness(app);
            h.state_mut().ui.tool = Tool::CloneStamp;
            h.state_mut().ui.tool_options.clone_sample = sample.into();
            h.state_mut().ui.tool_options.clone_aligned = aligned;
            h.state_mut().run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0}})).unwrap();
            click(&mut h, 150.0, 50.0, Modifiers::ALT);
            drag(&mut h, [30.0, 50.0], [70.0, 50.0], Modifiers::NONE);
            drag(&mut h, [30.0, 20.0], [70.0, 20.0], Modifiers::NONE);
            let st = h.state().session.active().unwrap();
            let top = st.doc.layers.last().unwrap().surface().unwrap();
            for y in [50, 20] {
                let px = top.rgba(50, y);
                assert!(px[1] > 0.9 && px[0] < 0.1 && px[3] > 0.9, "{sample} aligned={aligned} y={y}: {px:?}");
            }
        }
    }
}

/// What X11 and Wayland deliver when the window manager grabs an Alt+left press (Cinnamon's
/// window move): the pointer leaves and the keyboard focus goes (winit clears the modifiers),
/// and both come back on release. The click itself never arrives.
fn grabbed_alt_click(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, refocus: bool) {
    let p = screen(h, x, y);
    mods(h, Modifiers::ALT);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    h.event(egui::Event::PointerGone);
    h.event(egui::Event::WindowFocused(false));
    h.input_mut().focused = false;
    mods(h, Modifiers::NONE);
    h.run_steps(1);
    if refocus {
        h.event(egui::Event::WindowFocused(true));
        h.input_mut().focused = true;
        mods(h, Modifiers::ALT);
    }
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    mods(h, Modifiers::NONE);
    h.input_mut().focused = true;
}

#[test]
fn an_alt_click_the_window_manager_took_says_so() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::CloneStamp, Tool::Healing, Tool::Zoom] {
        let mut h = harness(two_colours());
        h.state_mut().ui.tool = tool;
        grabbed_alt_click(&mut h, 50.0, 50.0, true);
        assert_eq!(h.state().ui.status, crate::alt_grab::message(), "{tool:?}");
        assert!(h.state().ui.status_error);
    }
}

#[test]
fn leaving_the_window_without_a_grab_says_nothing() {
    // Alt+Tab to another window: the pointer comes back to an unfocused window.
    let mut h = harness(two_colours());
    h.state_mut().ui.tool = Tool::Brush;
    grabbed_alt_click(&mut h, 50.0, 50.0, false);
    assert_ne!(h.state().ui.status, crate::alt_grab::message());
    // No ⌥, or a tool whose ⌥-click does nothing special.
    for (tool, m) in [(Tool::Brush, Modifiers::NONE), (Tool::Eraser, Modifiers::ALT)] {
        let mut h = harness(two_colours());
        h.state_mut().ui.tool = tool;
        let p = screen(&h, 50.0, 50.0);
        mods(&mut h, m);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(2);
        h.event(egui::Event::PointerGone);
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(2);
        assert_ne!(h.state().ui.status, crate::alt_grab::message(), "{tool:?} {m:?}");
    }
    // A real ⌥-click arrives: it samples, and no message follows.
    let mut h = harness(two_colours());
    h.state_mut().ui.tool = Tool::Brush;
    click(&mut h, 50.0, 50.0, Modifiers::ALT);
    assert_eq!(h.state().session.tools.foreground, [1.0, 0.0, 0.0, 1.0]);
    assert_ne!(h.state().ui.status, crate::alt_grab::message());
}
