//! Actual tool-event producer -> shared live history -> textures::damage_since -> CPU update.
//! No renderer, texture upload, GPU/page admission or native-window assertion.
use photocraft_compose::{Buffer, effects_frame, render, render_reduced_damage};
use photocraft_geom::Rect;
use serde_json::json;

use crate::{
    PhotocraftApp,
    canvas::{ToolEvent, tool_event},
    state::Tool,
};

/// Build matching hard-brush fixtures with and without a frame-relative gradient overlay.
fn app(overlay: bool) -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width":1024,"height":64,"mode":"rgb","depth":32,"background":"white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.tools.brush = photocraft_engine::paint::BrushSettings {
        size: 16.0,
        hardness: 1.0,
        spacing: 0.25,
        opacity: 1.0,
        flow: 1.0,
        pressure_size: false,
        ..Default::default()
    };
    s.tools.brush.smoothing.enabled = false;
    s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("shared live damage fixture", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(64, 8, 768, 56), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    if overlay {
        s.execute("layer.layerStyle.gradientOverlay", json!({"from":"#000000","to":"#ffffff","angle":0,"scale":100,"opacity":100,"blend":"Normal"})).unwrap();
    }
    let mut app = PhotocraftApp::new(s, Default::default());
    app.ui.tool = Tool::Brush;
    app.sync_views();
    app
}

/// Compare every channel of the partially updated display with the full current CPU render.
fn same(a: &Buffer, b: &Buffer) {
    assert_eq!(a.rect, b.rect);
    let worst = a.px.iter().zip(&b.px).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
    assert!(worst < 2e-5, "actual shared-history CPU update differs from full live render: {worst}");
}

/// Apply only the reported CPU damage, leaving the rest of the displayed frame intact.
fn patch(shown: &mut Buffer, doc: &photocraft_doc::Document, damage: Rect) {
    let patch = render_reduced_damage(doc, doc.size.width, doc.size.height, damage);
    for y in patch.rect.y0..patch.rect.y1 {
        for x in patch.rect.x0..patch.rect.x1 {
            shown.px[(y * doc.size.width as i32 + x) as usize] = patch.get(x, y);
        }
    }
}

/// Drive real Down/Move events through shared damage history, excluding unknown/full fallback.
/// Cover initial and pushed relayout plus a no-gradient control using CPU rendering.
#[test]
fn actual_shared_live_damage_covers_initial_and_pushed_gradient_relayout() {
    for (overlay, initial_only) in [(true, true), (true, false), (false, false)] {
        let mut app = app(overlay);
        let revision = app.session.active().unwrap().revision;
        let source = app.session.active().unwrap().doc.clone();
        let mut shown = render(&source, source.bounds());
        let start = if initial_only { 896.0 } else { 744.0 };
        tool_event(&mut app, ToolEvent::Down { x: start, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let first = app.live_stroke.as_ref().expect("real Down starts a live stroke");
        assert_eq!(first.damage.len(), 1);
        let seen = if initial_only {
            0
        } else {
            shown = render(first.doc(), first.doc().bounds());
            first.display_key()
        };
        if !initial_only {
            tool_event(&mut app, ToolEvent::Move { x: 896.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            assert_eq!(app.live_stroke.as_ref().unwrap().damage.len(), 2, "real Move feeds one history step");
        }
        let live = app.live_stroke.as_ref().unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        let doc = live.doc();
        assert_eq!(effects_frame(doc.layer(id).unwrap(), doc.bounds()), Rect::new(64, 8, 906, 56));
        // Real cache keys fold in the display key; damage_since consumes that exact convention.
        let display_key = 0x13ab;
        let damage =
            crate::canvas::textures::damage_since(&app, 0, (revision, seen ^ display_key), (revision, live.display_key()), display_key, (revision, None))
                .expect("known live damage, not an unknown/full-refresh fallback");
        if overlay {
            assert!(damage.contains_rect(&Rect::new(256, 24, 272, 40)));
            let cold = render(doc, doc.bounds());
            assert!((shown.get(256, 32)[0] - 3.0 / 11.0).abs() < 2e-5);
            assert!((cold.get(256, 32)[0] - 96.0 / 421.0).abs() < 2e-5);
        } else {
            assert!(Rect::new(733, 21, 908, 44).contains_rect(&damage), "no-gradient consumer stays local");
        }
        patch(&mut shown, doc, damage);
        same(&shown, &render(doc, doc.bounds()));
    }
}
/// Reproduce frame growth from a real Alt-reversed Eraser Retouch with a history-only Background.
fn history_app(overlay: bool) -> PhotocraftApp {
    let mut app = app(overlay);
    let background = app.session.active().unwrap().doc.layers[0].id;
    // State 0 is file.new: its only layer is the white Background. Keep it as the source while
    // hiding it in the current document so the Retouch must restore white into the active raster.
    app.session.execute("history.setSource", json!({"state":0})).unwrap();
    app.session.execute("layer.setProps", json!({"layer":background.0,"visible":false})).unwrap();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.session.execute("tools.setBrush", json!({"brush":{"size":1,"hardness":1.0,"opacity":1.0,"flow":1.0,"smoothing":{"amount":0.0}}})).unwrap();
    app.ui.tool_options.eraser_mode = "pencil".into();
    assert!(!app.ui.tool_options.erase_to_history);
    app.sync_views();
    app
}

/// Alt at the real press routes Eraser pencil mode through Retouch and exposes its damage steps.
#[test]
fn actual_history_retouch_frame_growth_covers_gradient_damage() {
    for (overlay, pushed) in [(false, false), (true, false), (true, true)] {
        let mut app = history_app(overlay);
        let active = app.session.active().unwrap();
        let revision = active.revision;
        let source = active.doc.clone();
        let id = active.active_layer.unwrap();
        assert_eq!(effects_frame(source.layer(id).unwrap(), source.bounds()), Rect::new(64, 8, 768, 56));
        let mut shown = render(&source, source.bounds());
        assert_eq!(source.layer(id).unwrap().surface().unwrap().rgba(896, 32), [0.0; 4]);
        let start = if pushed { 744.5 } else { 896.5 };
        tool_event(&mut app, ToolEvent::Down { x: start, y: 32.5, pressure: 1.0 }, egui::Modifiers::ALT);

        let seen = {
            let first = app.live_stroke.as_ref().expect("real Alt Down starts a live Retouch");
            assert!(first.paint.peek(|stroke| matches!(stroke, super::EngineStroke::Retouch(_))).expect("real Alt Down retains its engine stroke"));
            assert_eq!(first.damage.len(), 1, "the real Down publishes one damage step");
            if pushed {
                assert_eq!(
                    effects_frame(first.doc().layer(id).unwrap(), source.bounds()).x1,
                    768,
                    "the first Retouch remains inside the existing frame"
                );
                assert_eq!(
                    first.doc().layer(id).unwrap().surface().unwrap().rgba(896, 32),
                    [0.0; 4],
                    "the later target is still transparent before Move"
                );
                shown = render(first.doc(), first.doc().bounds());
                first.display_key()
            } else {
                0
            }
        };

        if pushed {
            tool_event(&mut app, ToolEvent::Move { x: 896.5, y: 32.5, pressure: 1.0 }, egui::Modifiers::NONE);
            assert_eq!(app.live_stroke.as_ref().unwrap().damage.len(), 2);
        }

        let live = app.live_stroke.as_ref().expect("live Retouch remains active after the damage step");
        assert!(live.paint.peek(|stroke| matches!(stroke, super::EngineStroke::Retouch(_))).expect("live Retouch retains its engine stroke"));
        let doc = live.doc();
        let pixel = doc.layer(id).unwrap().surface().unwrap().rgba(896, 32);
        assert_eq!(pixel, [1.0; 4], "history paints the source Background into the raster");
        assert_eq!(effects_frame(doc.layer(id).unwrap(), doc.bounds()), Rect::new(64, 8, 897, 56));
        let cold = render(doc, doc.bounds());
        if overlay {
            assert!((shown.get(256, 32)[0] - 3.0 / 11.0).abs() < 2e-5);
            assert!((cold.get(256, 32)[0] - 192.0 / 833.0).abs() < 2e-5);
        } else {
            assert_eq!(cold.get(896, 32), [1.0; 4]);
            assert!((shown.get(256, 32)[0] - 0.5).abs() < 2e-5, "no-overlay control retains the gray raster");
        }

        // Use the adjacent live-damage fixture's cache-key convention for initial and pushed steps.
        let display_key = 0x13ab;
        let damage =
            crate::canvas::textures::damage_since(&app, 0, (revision, seen ^ display_key), (revision, live.display_key()), display_key, (revision, None))
                .expect("known Retouch damage, not an unknown/full-refresh fallback");
        assert!(damage.contains_rect(&Rect::new(896, 32, 897, 33)), "damage includes the restored raster pixel");
        assert_eq!(shown.get(256, 32)[3], 1.0);

        if overlay {
            if pushed {
                assert!(!live.paint.peek(|stroke| stroke.bounds()).expect("live Retouch retains its raw bounds").contains_rect(&Rect::new(256, 32, 257, 33)), "raw live stroke bounds exclude the distant gradient pixel");
            }
            assert!(damage.contains_rect(&Rect::new(256, 32, 257, 33)), "overlay damage includes the distant gradient pixel");
        } else {
            assert!(Rect::new(880, 16, 913, 49).contains_rect(&damage), "no-overlay Retouch damage stays local: {damage:?}");
        }
        patch(&mut shown, doc, damage);
        same(&shown, &cold);
    }
}
