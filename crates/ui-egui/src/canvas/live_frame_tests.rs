//! Live painting tools show this frame's pointer input in this frame (no extra frame of delay): the tools run before
//! the canvas shows. The tools map through the canvas's zoom ease.

use egui::{PointerButton, Pos2, Rect, vec2};
use serde_json::json;

use super::document_display::{CanvasHost, shown_camera};
use super::{canvas_display, canvas_view, document_area};
use crate::PhotocraftApp;
use crate::state::{Tool, View};

#[test]
fn live_brush_texture_includes_this_frames_press_and_moves() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 128, "height": 64, "background": "white"})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool = tool;
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 640.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(3);
        // One pass a frame: a second pass would show the input whatever the order of tools and canvas.
        h.ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
        let p = h.state().last_canvas_rect.center();
        h.input_mut().events.push(egui::Event::PointerMoved(p));
        h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        let shown = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            let app = h.state_mut();
            let live = app.live_stroke.as_ref().expect("painting starts on press");
            let key = live.display_key();
            let doc = live.doc().clone();
            let (_, display) = canvas_display(app, &doc, None);
            let shown = app.raster_logs.values().next().and_then(|log| log.composite_seen()).map(|seen| seen.1);
            assert_eq!(shown, Some(key ^ display), "{tool:?}: the canvas picture includes this frame's input");
        };
        shown(&mut h);
        h.input_mut().events.push(egui::Event::PointerMoved(p + vec2(25.0, 0.0)));
        h.run_steps(1);
        shown(&mut h);
        let end = p + vec2(25.0, 0.0);
        h.input_mut().events.push(egui::Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        assert!(h.state().live_stroke.is_none());
        // The Brush keeps the shown pixels as its committed stroke; the Mixer Brush commits the
        // pixels its live preview rendered (`gesture.rs`).
        if tool == Tool::Brush {
            assert!(h.state().committed_stroke.is_some(), "release reuses the shown pixels");
        }
        let label = h.state().session.active().and_then(|st| st.history.undo_label()).map(str::to_owned);
        assert_eq!(label.as_deref(), Some(if tool == Tool::MixerBrush { "Mixer Brush" } else { "Brush Tool" }));
        assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    }
}

#[test]
fn live_tools_paint_the_interpolated_camera_during_zoom_animation() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": true}})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        let v = &mut app.ui.views[0];
        v.clear_pending();
        v.zoom = 1.0;
        v.center = [32.0, 32.0];
        v.doc_size = [64, 64];
        let ctx = app.ctx().clone();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
        let pass = |app: &mut PhotocraftApp, view: View, time: f64| {
            let mut shown = view.clone();
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), time: Some(time), ..Default::default() }, |ui| {
                (shown, _) = canvas_view(app, ui, CanvasHost::main(0), rect, view.clone(), true);
            });
            out.textures_delta.clear();
            shown
        };
        let first = app.ui.views[0].clone();
        let mut view = pass(&mut app, first, 0.0);
        // A discrete zoom to 200 % eases from the zoom it was asked from: the tools map through the camera in between.
        view.request_zoom_animation(true);
        view.zoom = 2.0;
        let view = pass(&mut app, view, 0.1);
        let view = pass(&mut app, view, 0.15);
        let ppp = ctx.pixels_per_point();
        let shown = shown_camera(&ctx, CanvasHost::main(0), &view, ppp).0;
        assert!(shown > 1.0 && shown < 2.0, "{tool:?}: the canvas's ease is between the endpoints, {shown}");
        assert_eq!(view.zoom, 2.0, "{tool:?}: the requested zoom stays the target");
    }
}
