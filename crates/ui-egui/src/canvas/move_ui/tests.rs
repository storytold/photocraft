use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

#[test]
fn gpu_channel_move_matches_native_commit_and_tracks_visibility() {
    let _gpu_test = photocraft_testkit::gpu_lock();
    let Ok(rs) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())))
    else {
        eprintln!("no GPU adapter for Move preview readback");
        return;
    };
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::with_background("Move GPU oracle", Size::new(320, 32), ColorMode::Rgb, depth, Color::WHITE);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc.clone(), None);
        app.session.tools.background = [0.35, 0.65, 0.85, 1.0];
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("opaque painted layer", |d, _| {
                let s = d.layer_mut(layer).unwrap().surface_mut().unwrap();
                let base = if depth == SampleType::F32 { [0.1, -0.2, 1.8, 1.0] } else { [0.1, 0.3, 0.5, 1.0] };
                let square = if depth == SampleType::F32 { [2.25, 0.7, -0.1, 1.0] } else { [0.9, 0.7, 0.2, 1.0] };
                s.fill_rect(Rect::new(0, 0, 320, 32), &base);
                s.fill_rect(Rect::new(250, 4, 270, 28), &square);
                Ok(())
            })
            .unwrap();
        app.session.execute("channel.target", json!({"channel":"red"})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.set_wgpu(rs.clone());
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 260.0, y: 16.0, pressure: 1.0 }, m);
        let mut cached_texture = None;
        for (dx, visible) in [(1, false), (17, true), (-3, false)] {
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 260.0 + f64::from(dx), y: 16.0, pressure: 1.0 }, m);
            for name in ["red", "green", "blue"] {
                app.session.execute("channel.setVisible", json!({"channel":name,"visible":visible || name == "red"})).unwrap();
            }
            let tex = super::channel_texture(&mut app, 0, None);
            assert!(
                tex.is_some(),
                "GPU path at {depth:?}, offset {dx}; drag={:?}, stack={:?}, source_cached={}",
                super::drag_offset(&app),
                app.session.active().unwrap().doc.layers.iter().map(|l| (l.id, l.visible, l.surface().is_some())).collect::<Vec<_>>(),
                app.move_gpu_preview.as_ref().is_some_and(|p| p.gpu.is_some())
            );
            if let Some(previous) = cached_texture {
                assert_eq!(tex, Some(previous), "eye changes and new offsets reuse the captured GPU source");
            }
            cached_texture = tex;
            if dx == 17 {
                assert!(super::channel_texture(&mut app, 0, Some(999)).is_none(), "another display uses its own CPU conversion");
                assert!(super::gpu_showing(&app, 0), "a secondary display does not hide the main GPU preview");
                assert!(super::channel_texture(&mut app, 0, None).is_some(), "main display keeps its GPU source");
            }
            assert!(super::gpu_showing(&app, 0));
            let actual = app.move_gpu_preview.as_ref().unwrap().gpu.as_ref().unwrap().readback();
            doc = (*app.session.active().unwrap().doc).clone();
            let mut moved = doc.clone();
            photocraft_engine::channel_cmds::move_color_channel(&mut moved, layer, 0, dx, 0, app.session.tools.background).unwrap();
            let view = &app.session.active().unwrap().channel_view;
            let expected = crate::channel_view::render_for_display(&moved, view, moved.bounds(), 1, crate::channel_view::Look::default(), Some(layer), None)
                .filter(|pixels| pixels.iter().all(|p| p.a() == 255))
                .unwrap_or_else(|| crate::canvas::display_image(None, &photocraft_compose::flatten(&moved)).pixels);
            let max = actual.iter().zip(&expected).flat_map(|(a, b)| a.to_array().into_iter().zip(b.to_array()).map(|(x, y)| x.abs_diff(y))).max().unwrap_or(0);
            assert!(max <= 1, "{depth:?}, offset {dx}, visible {visible}: max display difference {max}");
        }
        app.set_wgpu(rs.clone());
        assert!(app.move_gpu_preview.is_none(), "replacing the GPU drops textures from the old device");
        assert!(super::channel_texture(&mut app, 0, None).is_some(), "the current drag can recapture on the new GPU");
        crate::gpu_status::fall_back(&mut app, &photocraft_gpu::Fault::Error("Validation Error".into()));
        assert!(app.move_gpu_preview.is_none(), "CPU fallback releases the GPU drag capture");
        app.set_wgpu(rs.clone());
        assert!(super::channel_texture(&mut app, 0, None).is_some(), "retry preserves the active drag");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 257.0, y: 16.0 }, m);
        assert!(app.move_gpu_preview.is_none(), "release drops cached GPU source");
        assert_eq!(
            app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(260, 16)[1],
            doc.layer(layer).unwrap().surface().unwrap().pixel(260, 16)[1]
        );
        // A translucent top pixel reveals Background. The one-layer shader must decline
        // that stack and keep the normal CPU preview, including on a reduced source.
        app.session
            .edit("translucent top", |d, _| {
                d.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 0.5]);
                Ok(())
            })
            .unwrap();
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 260.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 261.0, y: 16.0, pressure: 1.0 }, m);
        assert!(super::channel_texture(&mut app, 0, None).is_none());
        assert!(super::display_doc(&mut app, 0).is_some(), "transparent stack keeps its CPU preview");
        app.drag = None;
        app.sync_views();
        assert!(app.move_gpu_preview.is_none(), "cancel releases the cached GPU source");
    }
}

#[test]
fn gpu_move_fills_only_the_sparse_source_bounds() {
    let _gpu_test = photocraft_testkit::gpu_lock();
    let Ok(rs) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())))
    else {
        eprintln!("no GPU adapter for sparse Move preview readback");
        return;
    };
    let mut doc = Document::new("sparse Move", Size::new(320, 32), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("sparse", doc.pixel_format());
    layer.surface_mut().unwrap().fill_rect(Rect::new(245, 4, 271, 28), &[0.8, 0.3, 0.1, 1.0]);
    let id = layer.id;
    doc.layers.push(layer);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.add_document(doc.clone(), None);
    app.sync_views();
    app.session.execute("channel.target", json!({"channel":"red"})).unwrap();
    for channel in ["red", "green", "blue"] {
        app.session.execute("channel.setVisible", json!({"channel":channel,"visible":true})).unwrap();
    }
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.set_wgpu(rs);
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 255.0, y: 16.0, pressure: 1.0 }, m);
    for dx in [10, -7, 28] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 255.0 + f64::from(dx), y: 16.0, pressure: 1.0 }, m);
        assert!(super::channel_texture(&mut app, 0, None).is_some());
        let actual = app.move_gpu_preview.as_ref().unwrap().gpu.as_ref().unwrap().readback();
        let mut moved = doc.clone();
        photocraft_engine::channel_cmds::move_color_channel(&mut moved, id, 0, dx, 0, app.session.tools.background).unwrap();
        let expected = crate::canvas::display_image(None, &photocraft_compose::flatten(&moved)).pixels;
        let max = actual.iter().zip(&expected).flat_map(|(a, b)| a.to_array().into_iter().zip(b.to_array()).map(|(x, y)| x.abs_diff(y))).max().unwrap_or(0);
        assert!(max <= 1, "sparse Move at {dx}: max display difference {max}");
    }
}

#[test]
fn dragging_a_layer_larger_than_the_canvas_redraws_what_comes_in_from_beyond_the_edge() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(-32, -32, 96, 96), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, m);
    let mut keys = Vec::new();
    for x in [42.0, 52.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        keys.push(super::display_doc(&mut app, 0).unwrap().1);
    }
    let st = app.session.active().unwrap();
    // From +10 to +20 the strip x 0..10 shows pixels that were off the canvas a frame ago.
    let r = super::damage(&app, st.doc.id, st.revision, keys[0], keys[1]).unwrap();
    assert_eq!(r, Rect::new(0, 0, 64, 64));
}

/// The Move tool shows how far it has moved the layer, ΔX/ΔY in whole pixels, as Photoshop
/// and Free Transform do (#5727): once the pointer moves, until the release.
#[test]
fn a_move_drag_reads_out_the_offset_it_will_commit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(16, 16, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    assert_eq!(super::offset_readout(&app), None, "no drag");
    // Pressed on the red square (16..32): with Auto-Select (on by default) an empty spot would
    // start the layer marquee instead of a move.
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 24.0, y: 24.0, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), None, "pressed, not moved yet");
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 34.4, y: 20.8, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), Some(["10 px".to_string(), "-3 px".to_string()]));
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 34.4, y: 20.8 }, m);
    assert_eq!(super::offset_readout(&app), None, "released");
}

/// A release past the ±10,000,000 px the preview and the readout show commits the offset they
/// showed, not the raw pointer distance.
#[test]
fn a_release_commits_the_offset_the_readout_caps_at() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    super::finish(&mut app, 2.5e7, -2.5e7);
    let st = app.session.active().unwrap();
    let s = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()).unwrap();
    assert!(s.pixel(16 + 10_000_000, 16 - 10_000_000)[3] > 0.5, "moved by the capped offset");
}

/// A shape reaching past the canvas edge renders cut at the canvas, but its path goes on: a
/// drag frame must redraw where the rest of it was, or the cut edge stays behind (#3946).
#[test]
fn dragging_a_shape_past_the_canvas_edge_leaves_no_trail() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    let id = app.session.execute("shape.create", json!({"kind": "rect", "rect": [40, 16, 50, 32], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let id = photocraft_doc::LayerId(id);
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 32.0, pressure: 1.0 }, m);
    let mut frames = Vec::new();
    for x in [40.0, 10.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        frames.push(super::display_doc(&mut app, 0).unwrap());
    }
    let st = app.session.active().unwrap();
    let r = super::damage(&app, st.doc.id, st.revision, frames[0].1, frames[1].1).unwrap();
    let alpha = |d: &photocraft_doc::Document, x, y| d.layer(id).and_then(|l| l.surface()).map_or(0.0, |s| s.pixel(x, y)[3]);
    for y in 0..64 {
        for x in 0..64 {
            if alpha(&frames[0].0, x, y) != alpha(&frames[1].0, x, y) {
                assert!(r.contains(x, y), "({x}, {y}) changed outside the redrawn {r:?}");
            }
        }
    }
}

/// Dragging with an unlinked mask targeted moves the mask alone, shown live and committed;
/// with the mask linked (or the pixels targeted) the layer moves.
#[test]
fn dragging_with_an_unlinked_mask_targeted_moves_the_mask_alone() {
    for (linked, mask_target) in [(false, true), (false, false), (true, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        app.session.execute("layer.layerMask.linked", json!({"linked": linked})).unwrap();
        app.ui.mask_target = mask_target;
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 16.0, pressure: 1.0 }, m);
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let id = app.session.active().unwrap().active_layer.unwrap();
        let layer_moved = |d: &photocraft_doc::Document| d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(28, 16)[3] > 0.5);
        let alone = !linked && mask_target;
        assert_eq!(layer_moved(&shown), !alone, "preview, linked {linked} mask target {mask_target}");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 16.0 }, m);
        let d = &app.session.active().unwrap().doc;
        assert_eq!(layer_moved(d), !alone, "commit, linked {linked} mask target {mask_target}");
    }
}

/// #6068: dragging with the Red channel targeted moves the red of the active layer alone, in
/// the live preview and on release; the layer and its green and blue stay where they are.
#[test]
fn dragging_with_a_colour_channel_targeted_moves_that_channel_alone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.execute("channel.target", json!({"channel": "red"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 16.0, pressure: 1.0 }, m);
    let shown = super::display_doc(&mut app, 0).unwrap().0;
    let id = app.session.active().unwrap().active_layer.unwrap();
    let at = |d: &photocraft_doc::Document, x, y| d.layer(id).and_then(|l| l.surface()).map(|s| s.pixel(x, y)).unwrap();
    // The white square's red is now at x 28..44; its green and blue stayed at x 8..24.
    assert_eq!(at(&shown, 30, 16), vec![1.0, 0.0, 0.0, 1.0], "preview: red moved in");
    assert_eq!(at(&shown, 12, 16)[1..], [1.0, 1.0, 1.0], "preview: green, blue and the layer stay");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 16.0 }, m);
    let d = &app.session.active().unwrap().doc;
    assert_eq!(at(d, 30, 16), vec![1.0, 0.0, 0.0, 1.0], "commit: red moved in");
    assert_eq!(at(d, 12, 16)[1..], [1.0, 1.0, 1.0], "commit: green, blue and the layer stay");
}

/// Mid-drag on a painted layer: is the layer shown at the pointer?
fn live(outline: bool) -> bool {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.edit_prefs(|p| p.interface.show_bounding_box_when_dragging_layer = outline);
    app.ui.tool = Tool::Move;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    let shown = super::display_doc(&mut app, 0).is_some();
    assert_eq!(shown, super::showing(&app));
    shown
}

#[test]
fn bounding_box_preference_turns_the_live_drag_off() {
    assert!(live(false), "by default the layer follows the pointer");
    assert!(!live(true), "with the preference on, only the outline and arrow move");
}

/// A `depth`-bit transparent document whose layer holds a red square (8..24)² and a blue one
/// (40..56)×(8..24), the red one selected; the Move tool with Auto-Select on.
fn selected_pixels(depth: u32) -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent", "depth": depth})).unwrap();
    app.sync_views();
    for (x, color) in [(40, "#0000ff"), (8, "#ff0000")] {
        app.session.execute("select.rect", json!({"x": x, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": color})).unwrap();
    }
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let id = app.session.active().unwrap().active_layer.unwrap();
    (app, id)
}

fn alpha(app: &PhotocraftApp, id: photocraft_doc::LayerId, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// ↩ through the app's key handling.
fn enter(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let input = egui::RawInput { events: vec![key], ..Default::default() };
    ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx())).textures_delta.clear();
}

/// #578: with a selection the Move tool drags the selected pixels (from anywhere, without an
/// Auto-Select pick), not the layer; ↩ drops them as one "Move" step that one undo takes back.
#[test]
fn with_a_selection_the_move_tool_moves_only_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let steps = app.session.active().unwrap().history.past_len();
        drag(&mut app, [50.0, 40.0], [60.0, 45.0], egui::Modifiers::NONE);
        assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit: floating");
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let shown_alpha = |x, y| shown.layer(id).unwrap().surface().unwrap().rgba(x, y)[3];
        assert!(shown_alpha(10, 10) == 0.0 && shown_alpha(30, 25) == 1.0, "{depth}-bit: shown moved");
        enter(&mut app);
        assert_eq!(offset(&app), None, "{depth}-bit: ↩ dropped it");
        assert_eq!(alpha(&app, id, 10, 10), 0.0, "{depth}-bit: the hole is transparent");
        assert_eq!(alpha(&app, id, 30, 25), 1.0, "{depth}-bit: the red square moved");
        assert_eq!(alpha(&app, id, 48, 16), 1.0, "{depth}-bit: the unselected blue one stayed");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(18, 13, 34, 29), "{depth}-bit");
        assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some("Move")), "{depth}-bit");
        assert!(app.session.undo());
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 25) == 0.0, "{depth}-bit: undone");
    }
}

/// ⌥ copies the selected pixels (no layer copy), ⇧ locks the drag to multiples of 45°.
#[test]
fn alt_copies_and_shift_constrains_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let layers = app.session.active().unwrap().doc.layers.len();
        drag(&mut app, [16.0, 16.0], [30.0, 28.0], egui::Modifiers::ALT | egui::Modifiers::SHIFT);
        assert_eq!(offset(&app), Some((13, 13)), "{depth}-bit: 45°");
        app.run("select.drop", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "{depth}-bit: no layer copy");
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 35, 35) == 1.0, "{depth}-bit: original and copy");
        assert!(app.session.undo());
        assert_eq!(alpha(&app, id, 35, 35), 0.0, "{depth}-bit: undone");
    }
}

/// #7538: a loose marquee round the red square shrinks to it as the drag starts: the canvas
/// outlines the moved pixels while they float, and the dropped selection is theirs. Alt-drag
/// (a copy) too.
#[test]
fn dragging_selected_pixels_shrinks_the_ants_to_them() {
    for depth in [8, 16, 32] {
        for m in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
            let (mut app, _) = selected_pixels(depth);
            app.session.execute("select.rect", json!({"x": 2, "y": 2, "width": 30, "height": 30})).unwrap();
            drag(&mut app, [16.0, 16.0], [26.0, 21.0], m);
            assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit, {m:?}: floating");
            let ants = crate::canvas::floating_outline(&app, 0).map(|s| s.content_bounds());
            assert_eq!(ants, Some(Rect::new(8, 8, 24, 24)), "{depth}-bit, {m:?}: the ants hug the red square");
            assert!(crate::canvas::floating_outline(&app, 1).is_none(), "only the active document's");
            app.run("select.drop", json!({})).unwrap();
            let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
            assert_eq!(sel, Some(Rect::new(18, 13, 34, 29)), "{depth}-bit, {m:?}: dropped");
            assert!(crate::canvas::floating_outline(&app, 0).is_none(), "nothing floats");
        }
    }
}

/// ⌘-drag with a painting tool drags the selected pixels as the Move tool does, also when ⌘
/// comes with the event (control channel, MCP) rather than as the held-key override.
#[test]
fn command_drag_with_a_painting_tool_moves_only_the_selected_pixels() {
    let (mut app, id) = selected_pixels(8);
    app.ui.tool = Tool::Brush;
    app.ui.tool_options.move_auto_select = false;
    let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((10, 5)), "the selected pixels float");
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((20, 10)), "a second drag moves the same floating piece");
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 40, 30) == 1.0, "the red square moved");
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the unselected blue one stayed");
}

/// A click moves nothing and leaves nothing floating (Undo isn't spent putting it back).
#[test]
fn a_click_with_a_selection_moves_nothing() {
    let (mut app, id) = selected_pixels(8);
    let steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, [16.0, 16.0], [16.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 48, 16) == 1.0);
}

#[test]
fn arrow_keys_nudge_the_selected_pixels() {
    let (mut app, id) = selected_pixels(16);
    crate::move_mods::nudge(&mut app, 10.0, 0.0, false);
    crate::move_mods::nudge(&mut app, 0.0, 1.0, false);
    assert_eq!(offset(&app), Some((10, 1)));
    // ⌥ leaves the piece where it is and nudges a copy of it.
    crate::move_mods::nudge(&mut app, 1.0, 0.0, true);
    assert_eq!(offset(&app), Some((1, 0)));
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 18, 20) == 1.0 && alpha(&app, id, 34, 20) == 1.0);
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the layer itself didn't move");
}

/// Without floatable pixels (a position-locked layer) the Move tool behaves as without a
/// selection: it moves the layer, which the lock refuses.
#[test]
fn a_layer_that_cannot_float_moves_whole_as_before() {
    let (mut app, id) = selected_pixels(8);
    app.session.execute("layer.lockLayers", json!({"position": true})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 16) == 0.0, "nothing moved");
    // Without a selection the layer moves whole.
    app.session.execute("layer.lockLayers", json!({"position": false})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 58, 16) == 1.0, "the whole layer moved");
}

#[test]
fn floating_selection_does_not_snap_to_its_own_starting_outline_or_preview_at_zero() {
    let (mut app, _id) = selected_pixels(8);
    app.ui.extras.snap = true;
    app.ui.view.snap_to.document_bounds = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "no preview composite at offset (0, 0) before the pointer moves");
    // A small 2 px move within the snap radius must move immediately instead of snapping back to (0, 0).
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 18.0, y: 17.0, pressure: 1.0 }, m);
    assert_eq!(super::floating_offset(&app), Some((2, 1)), "small drag moves without snapping to own selection bounds");
    assert!(super::display_doc(&mut app, 0).is_some(), "preview is shown once offset is non-zero");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 18.0, y: 17.0 }, m);
    assert_eq!(offset(&app), Some((2, 1)));
}
