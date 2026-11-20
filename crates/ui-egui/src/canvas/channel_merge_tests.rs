//! Native channel canvas regressions retained across the shared UI migration.
use super::super::*;
use super::*;

#[test]
fn pattern_preview_keeps_its_copies_during_a_channel_transform() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("channel.target", json!({"channel":"green"})).unwrap();
    app.ui.extras.rulers = false;
    app.sync_views();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    let tex = crate::channel_transform_preview::texture(&mut app, &ctx, 0, None).unwrap();
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [32.0, 32.0];
    view.clear_pending();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(256.0, 256.0));
    let mut draw = |pattern: bool| {
        app.ui.view.pattern_preview = pattern;
        let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        let mut frames = craft::ui::testing::Frames::over(ctx.clone());
        frames.screen(input.screen_rect.unwrap().size());
        frames.record_paint();
        for _ in 0..3 {
            app.framework_before_input(&ctx);
            frames.frame_without_input(Vec::new(), |ui| {
                canvas_view(&mut app, ui, CanvasHost::main(0), rect, view.clone(), false);
            });
        }
        let mut out = frames.take_paint().unwrap();
        fn copies(shape: &egui::Shape, tex: egui::TextureId) -> usize {
            match shape {
                egui::Shape::Mesh(mesh) => usize::from(mesh.texture_id == tex),
                egui::Shape::Vec(children) => children.iter().map(|s| copies(s, tex)).sum(),
                _ => 0,
            }
        }
        let copies: usize = out.shapes.iter().map(|s| copies(&s.shape, tex)).sum();
        out.textures_delta.clear();
        copies
    };
    assert_eq!(draw(false), 1, "ordinary transform preview draws the centre tile");
    assert!(draw(true) > 1, "Pattern Preview draws its surrounding copies during a channel transform");
    crate::transform_tool::cancel(&mut app);
}

#[test]
fn colour_channel_strokes_stay_coloured_in_live_and_committed_canvas_caches() {
    for depth in [8, 16, 32] {
        for (channel, component) in [("red", 0), ("green", 1), ("blue", 2)] {
            for tool in [Tool::Brush, Tool::Pencil] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width":160,"height":80,"depth":depth})).unwrap();
                app.run("edit.fill", json!({"color":"#000000"})).unwrap();
                app.run("channel.target", json!({"channel":channel})).unwrap();
                app.run("channel.setVisible", json!({"channel":"composite","visible":true})).unwrap();
                app.run("tools.setColors", json!({"foreground":"#ffffff"})).unwrap();
                app.ui.tool = tool;
                // A first Pencil activation now installs its own tip and smoothing defaults.
                super::super::paint_mouse::sync_tool_brush(&mut app);
                app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
                let centre = 0.0;
                let ctx = egui::Context::default();
                let prep = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap();
                let source = prep.source(&app);
                let mut revision = source.revision();
                let mut cached = egui::ColorImage::filled([160, 80], Color32::TRANSPARENT);
                source.read(PixelRect::new(0, 0, 160, 80), 1, &mut cached);
                let original = app.session.active().unwrap().doc.clone();
                let layer = app.session.active().unwrap().active_layer.unwrap();
                let mut expected = [0.0, 0.0, 0.0, 1.0];
                expected[component] = 1.0;
                tool_event(&mut app, ToolEvent::Down { x: 10.0 + centre, y: 40.0 + centre, pressure: 1.0 }, egui::Modifiers::NONE);
                for x in [40.0, 80.0, 130.0] {
                    tool_event(&mut app, ToolEvent::Move { x: x + centre, y: 40.0 + centre, pressure: 1.0 }, egui::Modifiers::NONE);
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while app.live_stroke_busy() && std::time::Instant::now() < deadline {
                        crate::canvas::testing::shown(&mut app);
                        std::thread::yield_now();
                    }
                    assert!(!app.live_stroke_busy(), "the live stroke worker catches up before checking its cache");
                    let shown = display_doc(&mut app, 0).0;
                    assert_eq!(shown.layer(layer).unwrap().surface().unwrap().rgba(20, 40), expected, "{depth} {channel} {tool:?}: live data");
                    let prep = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap();
                    let source = prep.source(&app);
                    let damage = source.damage(revision).expect("live stroke gives bounded damage");
                    assert!(!damage.is_empty());
                    let mut image = egui::ColorImage::filled([damage.width() as usize, damage.height() as usize], Color32::TRANSPARENT);
                    source.read(damage, 1, &mut image);
                    for row in 0..image.size[1] {
                        let dst = (damage.y0 as usize + row) * cached.size[0] + damage.x0 as usize;
                        cached.pixels[dst..dst + image.size[0]].copy_from_slice(&image.pixels[row * image.size[0]..(row + 1) * image.size[0]]);
                    }
                    revision = source.revision();
                    let (display, _) = canvas_display(&app, &shown, None);
                    assert_eq!(
                        cached,
                        display_image(display.as_deref(), &photocraft_compose::flatten(&shown)),
                        "{depth} {channel} {tool:?}: partial uploads match full preview"
                    );
                    assert!(std::sync::Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
                }
                let shown = display_doc(&mut app, 0).0;
                tool_event(&mut app, ToolEvent::Up { x: 130.0 + centre, y: 40.0 + centre }, egui::Modifiers::NONE);
                let committed = app.session.active().unwrap().doc.clone();
                let live = shown.layer(layer).unwrap().surface().unwrap().try_to_interleaved(shown.bounds()).unwrap();
                let landed = committed.layer(layer).unwrap().surface().unwrap().try_to_interleaved(committed.bounds()).unwrap();
                let differences: Vec<_> = live.iter().zip(&landed).enumerate().filter(|(_, (a, b))| a != b).take(12).collect();
                assert!(differences.is_empty(), "{depth} {channel} {tool:?}: preview/commit first sample differences: {differences:?}");
                let prep = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap();
                let source = prep.source(&app);
                assert!(source.damage(revision).is_some_and(|damage| damage.is_empty()), "matching commit needs no corrective upload");
                assert_eq!(committed.layer(layer).unwrap().surface().unwrap().rgba(20, 40), expected);
                app.session.undo();
                assert_eq!(app.session.active().unwrap().doc.layers, original.layers);
                app.session.redo();
                assert_eq!(app.session.active().unwrap().doc.layers, committed.layers);
            }
        }
    }
}

fn selection_segments(app: &PhotocraftApp) -> Vec<craft::imaging::trace::Segment> {
    use craft::imaging::trace::{MaskSource, outline};
    let doc = &app.session.active().unwrap().doc;
    let selection = doc.selection.as_ref().unwrap();
    let offset = crate::move_ui::wrapped_selection_offset(app, 0);
    let mask = crate::outline::SurfaceMask::new(selection, selection.generation(), [doc.size.width, doc.size.height]).wrapped(offset);
    let mut segments = outline(&mask, mask.content_bounds(), 1);
    if offset.is_none() {
        let (dx, dy) = super::super::tools::selection::shown_offset(app).unwrap_or((0, 0));
        for (a, b) in &mut segments {
            for p in [a, b] {
                p[0] += dx;
                p[1] += dy;
            }
        }
    }
    segments
}

fn paint_selection_pass(app: &mut PhotocraftApp, ctx: &egui::Context) -> (Rect, bool) {
    use craft::ui::canvas::Canvas;
    use craft::ui::contract::{Bind, ShowContainer as _};
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(128.0, 128.0));
    let view = craft::behavior::CanvasView::new(rect, craft::behavior::kurbo::Point::new(16.0, 8.0), 1.0);
    let mut retraced = false;
    let mut frames = craft::ui::testing::Frames::over(ctx.clone());
    app.framework_before_input(ctx);
    frames.frame_without_input(Vec::new(), |ui| {
        Canvas::new("native ants fixture").view(Bind::given(view)).document(super::super::document_display::view_target(app, 0)).show_at(
            ui,
            rect,
            |ui, body| {
                retraced = super::super::document_display::show_selection(app, ui, body.rect, 0, "native ants").unwrap().retraced;
            },
        );
    });
    let bounds = selection_segments(app)
        .iter()
        .fold(Rect::NOTHING, |r, (a, b)| r.union(Rect::from_two_pos(pos2(a[0] as f32, a[1] as f32), pos2(b[0] as f32, b[1] as f32))));
    (bounds, retraced)
}
fn paint_move_selection_ants(app: &mut PhotocraftApp, ctx: &egui::Context) -> Rect {
    paint_selection_pass(app, ctx).0
}

#[test]
fn wrapped_channel_selection_ants_match_commit_with_pixel_preview_on_or_off() {
    for bounding_box in [false, true] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":32,"height":16})).unwrap();
        app.run("select.rect", json!({"x":28,"y":4,"width":4,"height":8})).unwrap();
        app.run("channel.target", json!({"channel":"red"})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.view.pattern_preview = true;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.session.edit_prefs(|p| p.interface.show_bounding_box_when_dragging_layer = bounding_box);
        app.sync_views();
        tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let preview = paint_move_selection_ants(&mut app, &ctx);
        assert!(preview.is_positive());
        let cached = selection_segments(&app);
        assert!(!cached.is_empty());
        assert!(cached.iter().flat_map(|(a, b)| [a, b]).all(|p| (2..=6).contains(&p[0])), "ants wrap to the opposite edge");
        assert_eq!(paint_move_selection_ants(&mut app, &ctx), preview);
        assert!(!paint_selection_pass(&mut app, &ctx).1, "idle frames reuse the shared projected outline cache");
        app.ui.view.pattern_preview = false;
        assert_ne!(paint_move_selection_ants(&mut app, &ctx), preview);
        app.ui.view.pattern_preview = true;
        assert_eq!(paint_move_selection_ants(&mut app, &ctx), preview);
        for dx in [10, 2, -3, 6] {
            tool_event(&mut app, ToolEvent::Move { x: 30.0 + f64::from(dx), y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
            paint_move_selection_ants(&mut app, &ctx);
            let source = app.session.active().unwrap().doc.selection.as_ref().unwrap();
            let moved = photocraft_engine::layer_multi_cmds::wrapped_selection(source, dx, 0, app.session.active().unwrap().doc.bounds()).unwrap();
            let expected = {
                use craft::imaging::trace::MaskSource as _;
                let mask = crate::outline::SurfaceMask::new(&moved, 0, [32, 16]);
                craft::imaging::trace::outline(&mask, mask.content_bounds(), 1)
            };
            assert_eq!(selection_segments(&app), expected, "the projected bounds follow offset {dx}");
        }
        tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 8.0 }, egui::Modifiers::NONE);
        assert_eq!(paint_move_selection_ants(&mut app, &ctx), preview, "preview and commit apply the offset once");
    }
}

#[test]
fn implicit_native_move_ants_match_bounded_coverage_before_during_and_after_drag() {
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;
    for pattern in [false, true] {
        for hole in [false, true] {
            let mut outlines = Vec::new();
            for implicit in [false, true] {
                let ctx = egui::Context::default();
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width":32,"height":16})).unwrap();
                app.session
                    .edit("equivalent coverage", |doc, _| {
                        let mut mask = if implicit { Surface::with_default(PixelFormat::GRAY8, &[1.0]) } else { Surface::new(PixelFormat::GRAY8) };
                        if !implicit {
                            mask.fill_rect(doc.bounds(), &[1.0]);
                        }
                        if hole {
                            mask.fill_rect(photocraft_geom::Rect::new(8, 4, 12, 8), &[0.0]);
                        }
                        doc.selection = Some(mask);
                        Ok(())
                    })
                    .unwrap();
                app.run("channel.target", json!({"channel":"red"})).unwrap();
                app.ui.tool = Tool::Move;
                app.ui.view.pattern_preview = pattern;
                app.ui.tool_options.move_auto_select = false;
                app.ui.extras.snap = false;
                app.ui.view.show.smart_guides = false;
                app.sync_views();
                let before = paint_move_selection_ants(&mut app, &ctx);
                let before_segments = selection_segments(&app);
                tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
                tool_event(&mut app, ToolEvent::Move { x: 22.0, y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
                let preview = paint_move_selection_ants(&mut app, &ctx);
                let preview_segments = selection_segments(&app);
                tool_event(&mut app, ToolEvent::Up { x: 22.0, y: 8.0 }, egui::Modifiers::NONE);
                assert_eq!(paint_move_selection_ants(&mut app, &ctx), preview, "{pattern}/{hole}/{implicit}: preview and commit agree");
                let committed_segments = selection_segments(&app);
                outlines.push((before, preview, before_segments, preview_segments, committed_segments));
            }
            assert_eq!(outlines[0], outlines[1], "{pattern}/{hole}: implicit and explicit coverage have the same boundary");
        }
    }
}

#[test]
fn implicit_selection_ants_cache_tracks_canvas_size_without_mask_edits() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":32,"height":16})).unwrap();
    app.session
        .edit("implicit selection", |doc, _| {
            doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]));
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let before = paint_move_selection_ants(&mut app, &ctx);
    assert!(before.is_positive());
    let original = app.session.active().unwrap().doc.selection.clone();
    app.session
        .edit("grow canvas", |doc, _| {
            doc.size.width += 4;
            Ok(())
        })
        .unwrap();
    assert_eq!(app.session.active().unwrap().doc.selection, original, "selection identity is unchanged");
    let grown = paint_move_selection_ants(&mut app, &ctx);
    assert_eq!(grown.max.x, before.max.x + 4.0, "a canvas-sized boundary must invalidate on resize");
    assert_eq!(grown.min, before.min);
}
