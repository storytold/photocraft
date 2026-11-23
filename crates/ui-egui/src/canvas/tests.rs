use super::document_display::{CanvasHost, active_view};
use super::*;
use craft::behavior::canvas::ZoomLadder;
use craft::behavior::kurbo::Size;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::primitives::Revision;

#[test]
fn display_image_matches_egui_unmultiplied_conversion() {
    // Several parallel pieces and a ragged one; translucent, out-of-range and NaN values.
    let n: usize = 3 * 16 * 1024 + 77;
    let px: Vec<[f32; 4]> = (0..n)
        .map(|i| {
            let v = match i % 9 {
                0 => f32::NAN,
                1 => -0.5,
                2 => 3.25,
                _ => (i % 1001) as f32 / 1000.0,
            };
            [v, (i % 257) as f32 / 256.0, 1.0 - (i % 113) as f32 / 112.0, (i % 6) as f32 / 5.0]
        })
        .collect();
    let buf = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, n as i32, 1), px };
    let reference = |img: photocraft_raster::Rgba8Image| egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    assert_eq!(super::display_image(None, &buf).pixels, reference(buf.to_rgba8()).pixels);
    let mut doc = photocraft_doc::Document::new("t", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    doc.icc_profile = Some(craft::cms::Builtin::AdobeRgbCompat.profile().to_bytes());
    let mut color = photocraft_engine::color_cmds::ColorState::default();
    color.settings.monitor_profile = "srgb".into();
    let d = color.canvas_display(&doc).unwrap();
    assert!(!d.is_identity());
    assert_eq!(super::display_image(Some(&d), &buf).pixels, reference(d.to_rgba8(&buf)).pixels);
}

/// #569: displays 1 (sRGB) and 4 (Display P3) side by side, and a document filled with an
/// sRGB colour.
fn app_on_two_displays() -> Box<PhotocraftApp> {
    use photocraft_engine::color_cmds::resolve_profile;
    use photocraft_engine::display_color::Display;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("edit.fill", json!({"color": "#cc8040"})).unwrap();
    let icc = |id: &str| Some(resolve_profile(id, None, None).unwrap().to_bytes());
    app.session.color.set_displays(Ok(vec![
        Display { id: 1, name: "A".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
        Display { id: 4, name: "B".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
    ]));
    app
}

#[test]
fn closed_documents_release_navigator_and_canvas_records() {
    let mut app = app_on_two_displays();
    let ctx = app.ctx().clone();
    let first = app.session.active().unwrap().doc.clone();
    composite_revision(&mut app, &ctx, 0, Some(1));
    // The same handle ownership as navigator_texture, without requiring a GPU adapter.
    let tex = ctx.load_texture("navigator-test", egui::ColorImage::filled([64, 64], egui::Color32::WHITE), TextureOptions::LINEAR);
    let nav = tex.id();
    app.navigator_textures.insert(first.id, (std::sync::Arc::downgrade(&first), 0, 0, tex));
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    let other = app.session.active().unwrap().doc.id;
    composite_revision(&mut app, &ctx, 1, Some(1));
    drop(app.session.close(0));
    app.sync_views();
    assert!(ctx.tex_manager().read().meta(nav).is_none());
    assert!(app.canvases.keys().all(|(id, _)| *id == other));
    assert!(app.raster_logs.keys().all(|(id, _)| *id == other), "the closed document's canvas records go");
    assert!(!app.raster_logs.is_empty(), "the open document's stay");
    assert!(app.navigator_textures.is_empty());
}

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

/// The GPU canvas texture of a Multichannel document is filled by the CPU compositor, so its
/// key carries the ink preview (Color Settings › Spot) like the CPU canvas key does.
#[test]
fn gpu_texture_key_follows_the_ink_preview() {
    use super::display::ink_preview_key;
    use photocraft_color::{ColorMode, SampleType};
    let size = photocraft_doc::Size::new(2, 2);
    let inks = Document::new("m", size, ColorMode::Multichannel, SampleType::U8);
    let rgb = Document::new("r", size, ColorMode::Rgb, SampleType::U8);
    assert_ne!(ink_preview_key(&inks), 0);
    assert_eq!(texture_key(None, &inks), ink_preview_key(&inks));
    assert_eq!(texture_key(None, &rgb), 0);
}

/// The navigator's update after an edit (its image with the edit's damage, grown by the
/// effect reach, rendered over it) equals a new thumbnail of the edited document.
#[test]
fn navigator_damage_updates_match_a_full_thumbnail() {
    use photocraft_compose::{render_reduced_damage, thumbnail_buffer, thumbnail_size};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 1500, "height": 900, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 200, "y": 150, "width": 300, "height": 200})).unwrap();
    app.run("edit.fill", json!({"color": "#3070a0"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 30, "size": 20})).unwrap();
    let steps = [
        ("layer.translate", json!({"dx": 37, "dy": -11})),
        ("layer.setProps", json!({"opacity": 0.4})),
        ("paint.stroke", json!({"points": [[700, 500], [900, 520]], "size": 25})),
        ("layer.setProps", json!({"blend": "Multiply"})),
        ("layer.new.layer", json!({})),
    ];
    for (id, p) in steps {
        let before = app.session.active().unwrap().doc.clone();
        app.run(id, p).unwrap();
        let st = app.session.active().unwrap();
        let d = st.last_damage.unwrap_or_else(|| panic!("{id}: a bounded damage"));
        let (w, h) = thumbnail_size(&st.doc, NAVIGATOR_SIDE);
        let mut shown = thumbnail_buffer(&before, NAVIGATOR_SIDE);
        assert_eq!((shown.rect.width(), shown.rect.height()), (w, h));
        if !d.is_empty() {
            let patch = render_reduced_damage(&st.doc, w, h, d.inflate(effect_reach(&st.doc.layers)));
            let r = patch.rect;
            for y in 0..r.height() as usize {
                let at = (r.y0 as usize + y) * w as usize + r.x0 as usize;
                shown.px[at..at + r.width() as usize].copy_from_slice(&patch.px[y * r.width() as usize..][..r.width() as usize]);
            }
        }
        assert!(shown.px == thumbnail_buffer(&st.doc, NAVIGATOR_SIDE).px, "{id}: the updated navigator differs from a new thumbnail");
    }
}

/// An app with a 64 × 64 white document, a clone source and the Clone Stamp at `size` px.
fn clone_overlay_app(size: f32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("cloneSource.set", json!({"source": [16, 16]})).unwrap();
    app.ui.tool = Tool::CloneStamp;
    app.session.tools.brush.size = size;
    app
}

/// The Clone Source image for the brush at (32, 32) as a function from a document pixel to its colour
/// (transparent outside the image); `None` when no image shows.
fn clone_source_image(app: &mut PhotocraftApp) -> Option<impl Fn(i32, i32) -> Color32 + use<>> {
    let source = super::strokes::clone_source(app, [32.0, 32.0])?;
    let size = source.size();
    let mut image = egui::ColorImage::filled([size.width as usize, size.height as usize], Color32::TRANSPARENT);
    source.read(PixelRect::of_size(source.size()), 1, &mut image);
    // The image's quad in document space, clockwise from the top left; the tests never rotate it.
    let [top_left, top_right, _, bottom_left] = source.corners;
    let per_unit = [f64::from(size.width) / (top_right[0] - top_left[0]), f64::from(size.height) / (bottom_left[1] - top_left[1])];
    Some(move |x: i32, y: i32| {
        let u = ((f64::from(x) + 0.5 - top_left[0]) * per_unit[0]).floor();
        let v = ((f64::from(y) + 0.5 - top_left[1]) * per_unit[1]).floor();
        if u < 0.0 || v < 0.0 || u >= f64::from(size.width) || v >= f64::from(size.height) {
            return Color32::TRANSPARENT;
        }
        image.pixels[v as usize * image.width() + u as usize]
    })
}

#[test]
fn clone_overlay_is_circular_and_respects_visibility() {
    let mut app = clone_overlay_app(20.0);
    let revision = app.session.active().unwrap().revision;
    let image = clone_source_image(&mut app).expect("the Clone Source image shows");
    assert_eq!(image(32, 32).a(), 255, "the centre shows the source");
    assert_eq!(image(32 + 8, 32).a(), 255, "inside the hard 10 px radius");
    assert_eq!(image(32 + 8, 32 + 8).a(), 0, "the brush square's corner, outside the circle, is clipped");
    assert_eq!(image(32 + 12, 32).a(), 0, "past the hard tip's rim");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.session.presets.clone.active().anchor.is_none());
    app.session.presets.clone.overlay.show = false;
    assert!(clone_source_image(&mut app).is_none(), "a hidden Clone Source shows no image");
}

/// A soft brush previews soft: the image fades towards the rim with the brush's hardness, as
/// the stroke it previews does, and reaches past the radius as a soft tip paints.
#[test]
fn clone_overlay_follows_the_brush_hardness() {
    let mut app = clone_overlay_app(20.0);
    let hard = clone_source_image(&mut app).unwrap();
    app.session.tools.brush.hardness = 0.0;
    let soft = clone_source_image(&mut app).unwrap();
    let alpha = |image: &dyn Fn(i32, i32) -> Color32, x: i32| image(x, 32).a();
    assert!(alpha(&hard, 32) == 255 && alpha(&soft, 32) > 240, "both show the source at the centre: {} {}", alpha(&hard, 32), alpha(&soft, 32));
    assert_eq!(alpha(&hard, 32 + 8), 255);
    assert!(alpha(&soft, 32 + 8) < 200, "a soft tip fades before its rim: {}", alpha(&soft, 32 + 8));
    assert!(alpha(&soft, 32 + 4) > alpha(&soft, 32 + 8), "and fades outwards");
    assert!(alpha(&hard, 32 + 12) == 0 && alpha(&soft, 32 + 13) > 0, "a soft tip reaches past the radius, a hard one doesn't");
    // The largest brush the image covers still shows at its softest.
    let mut big = clone_overlay_app(1000.0);
    big.session.tools.brush.hardness = 0.0;
    assert_eq!(clone_source_image(&mut big).unwrap()(32, 32).a(), 255, "a 1000 px soft brush previews too");
}

/// The overlay's alpha at the brush centre, 0..1.
fn clone_overlay_centre_alpha(app: &mut PhotocraftApp) -> f32 {
    f32::from(clone_source_image(app).expect("the Clone Source image shows")(32, 32).a()) / 255.0
}

/// The Clone Stamp's overlay shows at the strength a stamp prints with: its Opacity and Flow,
/// clipped or not, at every depth, and changes when they change under a still pointer (#13124).
#[test]
fn clone_overlay_opacity_and_flow_match_a_stamp_on_another_layer() {
    for depth in [8, 16, 32] {
        for clipped in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
            crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
            app.run("layer.new.layer", json!({})).unwrap();
            app.ui.tool = Tool::CloneStamp;
            app.ui.tool_options.clone_sample = "all".into();
            app.session.tools.brush.size = 20.0;
            app.session.presets.clone.overlay.clipped = clipped;
            for (opacity, flow) in [(1.0, 1.0), (0.5, 1.0), (1.0, 0.25), (0.5, 0.4)] {
                // A full-strength overlay is shown first, so the change comes after a shown one.
                clone_overlay_centre_alpha(&mut app);
                app.run("tools.setBrush", json!({"size": 20, "hardness": 1.0, "opacity": opacity, "flow": flow})).unwrap();
                let revision = app.session.active().unwrap().revision;
                let alpha = clone_overlay_centre_alpha(&mut app);
                assert_eq!(app.session.active().unwrap().revision, revision, "hovering is read-only");
                let mut params = crate::retouch_ui::clone_params(&app).unwrap();
                params["points"] = json!([[32, 32]]);
                app.run("paint.cloneStamp", params).unwrap();
                let st = app.session.active().unwrap();
                let printed = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 32)[3];
                assert!((printed - opacity * flow).abs() < 0.01, "opacity={opacity} flow={flow}: printed {printed}");
                assert!((alpha - printed).abs() < 0.01, "{depth} clipped={clipped} opacity={opacity} flow={flow}: preview {alpha}, print {printed}");
                assert!(app.session.undo());
            }
        }
    }
}

/// The overlay's own Opacity multiplies the brush's.
#[test]
fn clone_overlay_separate_opacity_setting_still_multiplies_the_brush_opacity() {
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.opacity = 0.5;
    app.session.presets.clone.overlay.opacity = 40.0;
    let alpha = clone_overlay_centre_alpha(&mut app);
    assert!((alpha - 0.2).abs() < 0.01, "50% brush opacity times 40% overlay opacity: {alpha}");
}

#[test]
fn clone_overlay_refreshes_when_the_source_layer_changes_at_the_same_coordinates() {
    let mut app = clone_overlay_app(20.0);
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32).a(), 255, "the picked white source survives the layer switch");
    crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32).a(), 0, "picking the transparent layer refreshes the cached overlay");
}

/// The Healing Brush paints at full strength, so its overlay ignores the Clone Stamp's Opacity
/// and Flow.
#[test]
fn clone_overlay_strength_changes_leave_the_healing_preview_at_full_strength() {
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.opacity = 0.5;
    app.session.tools.brush.flow = 0.4;
    assert!(clone_overlay_centre_alpha(&mut app) < 0.5, "the Clone Stamp previews at its strength");
    app.ui.tool = Tool::Healing;
    assert_eq!(clone_overlay_centre_alpha(&mut app), 1.0, "Healing Brush still previews at full strength");
}

#[test]
fn clone_overlay_repicking_on_the_original_layer_updates_preview_and_both_layers() {
    let mut app = clone_overlay_app(12.0);
    let source = app.session.active().unwrap().active_layer.unwrap();
    for (point, color) in [([16, 16], "#ff0000"), ([8, 8], "#0000ff")] {
        app.run("paint.pencil", json!({"points": [point], "size": 6, "color": color})).unwrap();
    }
    crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
    app.run("layer.new.layer", json!({})).unwrap();
    let target = app.session.active().unwrap().active_layer.unwrap();
    app.run("tools.setBrush", json!({"size": 6, "hardness": 1, "opacity": 1, "flow": 1})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32), Color32::RED);
    let stamp = |app: &mut PhotocraftApp| {
        let mut params = crate::retouch_ui::clone_params(app).unwrap();
        params["points"] = json!([[32, 32]]);
        app.run("paint.cloneStamp", params).unwrap();
    };
    stamp(&mut app);
    app.run("layer.select", json!({"layer": source.0})).unwrap();
    crate::retouch_ui::set_source(&mut app, 8.0, 8.0);
    stamp(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layer(source).unwrap().surface().unwrap().rgba(32, 32), [0.0, 0.0, 1.0, 1.0]);
    app.run("layer.select", json!({"layer": target.0})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32), Color32::BLUE);
    stamp(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layer(target).unwrap().surface().unwrap().rgba(32, 32), [0.0, 0.0, 1.0, 1.0]);
}

/// A source in another document (#10381): the overlay follows edits there and blends with this
/// document's pixels, the crosshair stays off this canvas, and an empty slot paints nothing.
#[test]
fn clone_overlay_between_documents_updates_after_source_edits_and_blends_with_the_destination() {
    let mut app = clone_overlay_app(12.0);
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("paint.pencil", json!({"points": [[16,16]], "size": 8, "color": "#ff0000"})).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("paint.pencil", json!({"points": [[32,32]], "size": 16, "color": "#000000"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"size": 8, "hardness": 1, "opacity": 1, "flow": 1})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32), Color32::RED);
    app.run("cloneSource.overlay", json!({"blend": "difference"})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32), Color32::RED, "Difference uses the black destination, not the white source document");
    let revision = app.session.active().unwrap().revision;
    assert!(app.session.set_active(0));
    app.run("paint.pencil", json!({"points": [[16,16]], "size": 8, "color": "#00ff00"})).unwrap();
    assert!(app.session.set_active(1));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(clone_source_image(&mut app).unwrap()(32, 32), Color32::GREEN, "editing another document invalidates the cached source");
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, mods);
    assert!(crate::retouch_ui::source_marker_point(&app, Some([32.0, 32.0])).is_none(), "the source crosshair belongs to the other document");
    tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, mods);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 32), [0.0, 1.0, 0.0, 1.0]);
    app.run("cloneSource.select", json!({"index": 1})).unwrap();
    assert!(crate::retouch_ui::clone_params(&app).is_none(), "an empty slot cannot reuse coordinates from another document");
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    tool_event(&mut app, ToolEvent::Down { x: 48.0, y: 48.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 48.0, y: 48.0 }, mods);
    assert_eq!(app.session.active().unwrap().doc, before, "painting with an empty slot cannot sample stale coordinates");
    assert_eq!(app.session.active().unwrap().revision, revision, "the refused gesture does not create a history edit");
}

/// Hovering another document's tile cannot replace the active document's clone offset reference:
/// the Clone Source panel's X/Y keep reading the active document's last hover point (#13124).
#[test]
fn clone_cursor_from_an_inactive_document_cannot_replace_the_active_offset_reference() {
    use craft::behavior::dock::{Arrangement, DocumentGroup};
    use craft::behavior::kurbo::Point;
    use craft::ui::canvas::ToolHost;
    for tool in [Tool::CloneStamp, Tool::Healing] {
        for aligned in [false, true] {
            let mut app = clone_overlay_app(12.0);
            app.background_jobs = false;
            app.ui.extras.rulers = false;
            app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            app.ui.tool = tool;
            app.ui.tool_options.clone_aligned = aligned;
            app.sync_views();
            let ids: Vec<_> = app.session.documents().iter().map(|st| st.doc.id).collect();
            let group = |id: u64, doc| {
                let mut g = DocumentGroup::new(id, vec![doc]);
                g.shown = Some(doc);
                g
            };
            app.ui.document_windows.docked = vec![group(0, ids[0]), group(1, ids[1])];
            app.ui.document_windows.arrangement = Some(Arrangement::TwoUpVertical);
            super::testing::frame(&mut app);
            let document = app.session.active().unwrap().doc.id;
            let slots = app.session.presets.clone.clone();
            let revision = app.session.active().unwrap().revision;
            super::testing::move_to(&mut app, [40.0, 30.0], egui::Modifiers::NONE);
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [40.0, 30.0])));
            assert_eq!(crate::preset_panels::clone_offset_readout(&app), Some([-24.0, -14.0]));
            // The pointer over the other document's tile.
            let other = ToolHost::shown_space(app.ctx(), CanvasHost::main(0).canvas_id()).expect("the other tile shows").view;
            let at = other.to_screen(Point::new(12.0, 24.0));
            assert!(other.rect.contains(at), "the point is on the other tile");
            let host = craft::ui::runtime::windows::WindowHost::of(app.ctx());
            host.inject_move(egui::ViewportId::ROOT, at, craft::keymap::Mods::NONE, 0.0);
            for _ in 0..8 {
                super::testing::frame(&mut app);
            }
            assert_eq!(app.session.active().unwrap().doc.id, document, "hovering activates nothing");
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [40.0, 30.0])), "{tool:?}/{aligned}: retain the active document's reference");
            assert_eq!(crate::preset_panels::clone_offset_readout(&app), Some([-24.0, -14.0]));
            assert_eq!(crate::preset_panels::clone_offset_edit(&app, [-20.0, -10.0]).unwrap()["anchor"], json!([40.0, 30.0]));
            super::testing::move_to(&mut app, [44.0, 34.0], egui::Modifiers::NONE);
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [44.0, 34.0])), "the active canvas still updates normally");
            assert_eq!(app.session.presets.clone, slots, "hover does not establish a source anchor");
            assert_eq!(app.session.active().unwrap().revision, revision, "hover does not paint");
        }
    }
}

/// A scaled Clone Source reads a larger source area than the image covers: a large brush's
/// image shrinks around the pointer until that area fits the sampling budget, rather than
/// vanishing.
#[test]
fn clone_overlay_of_a_scaled_source_stays_within_the_sampling_budget() {
    for (size, scale) in [(600.0, 200), (1000.0, 300), (1000.0, 50)] {
        let mut app = clone_overlay_app(size);
        app.run("cloneSource.set", json!({"width": scale, "height": scale})).unwrap();
        let source = super::strokes::clone_source(&mut app, [32.0, 32.0]).unwrap_or_else(|| panic!("{size} px at {scale} %: the image shows"));
        let side = source.size();
        let budget = super::strokes::CLONE_PREVIEW_MAX_SIDE;
        assert!(side.width <= budget && side.height <= budget, "{size} px at {scale} %: the image stays within the sampling budget: {side:?}");
        assert_eq!(clone_source_image(&mut app).unwrap()(32, 32).a(), 255, "{size} px at {scale} %: the source under the pointer");
    }
}

/// The Healing Brush shares the Clone Stamp's source and image (Photoshop's Clone Source
/// panel drives both); other tools show none.
#[test]
fn clone_overlay_shows_for_the_healing_brush_too() {
    for (tool, shown) in [(Tool::Healing, true), (Tool::CloneStamp, true), (Tool::Brush, false), (Tool::SpotHealing, false)] {
        let mut app = clone_overlay_app(20.0);
        app.ui.tool = tool;
        let image = clone_source_image(&mut app);
        assert_eq!(image.is_some(), shown, "{tool:?}");
        if let Some(image) = image {
            assert_eq!(image(32, 32).a(), 255, "{tool:?}: the source shows under the brush");
        }
    }
}

/// Pattern Preview changes the Healing Brush source overlay at a stationary cursor.
#[test]
fn clone_healing_overlay_wraps_with_the_live_texture_at_source_edges() {
    let mut app = clone_overlay_app(12.0);
    app.ui.tool = Tool::Healing;
    app.run("cloneSource.set", json!({"source": [0,0]})).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    assert_eq!(clone_source_image(&mut app).unwrap()(30, 32).a(), 0);
    app.ui.view.pattern_preview = true;
    assert_eq!(crate::retouch_ui::clone_params(&app).unwrap()["wrap"], true);
    assert_eq!(clone_source_image(&mut app).unwrap()(30, 32).a(), 255, "the repeated source is visible before painting");
    let live = photocraft_engine::retouch_cmds::LiveRetouch::begin(
        &app.session,
        "paint.healingBrush",
        &json!({"points": [[32,32]], "size": 12, "hardness": 100, "wrap": true}),
    )
    .unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(live.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 32), [1.0; 4]);
    app.ui.view.pattern_preview = false;
    assert_eq!(clone_source_image(&mut app).unwrap()(30, 32).a(), 0);
}

/// Unsupported cross-document targets never reuse a cached pixel-layer source overlay.
#[test]
fn clone_overlay_suppresses_cross_document_masks_and_channels() {
    for tool in [Tool::CloneStamp, Tool::Healing] {
        for mask in [false, true] {
            let mut app = clone_overlay_app(12.0);
            app.ui.tool = tool;
            app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            if mask {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
            } else {
                app.run("channel.new", json!({"fill": "white"})).unwrap();
            }
            app.run("channel.target", json!({"channel": "composite"})).unwrap();
            app.ui.mask_target = false;
            assert!(clone_source_image(&mut app).is_some());
            let revision = app.session.active().unwrap().revision;
            if mask {
                app.ui.mask_target = true;
            } else {
                app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Alpha(0);
            }
            assert_eq!(app.session.active().unwrap().revision, revision, "only the target changed");
            assert!(clone_source_image(&mut app).is_none(), "{tool:?}, mask={mask}: the gesture has no supported source");
            app.ui.mask_target = false;
            app.run("channel.target", json!({"channel": "composite"})).unwrap();
            assert!(clone_source_image(&mut app).is_some(), "returning to pixels restores the source overlay");
        }
    }
}

#[test]
fn clone_overlay_blend_mode_mixes_with_the_image_under_the_brush() {
    let mut app = clone_overlay_app(20.0);
    // The image's pixel at the brush centre for `mode`.
    let mut centre = |mode: &str| {
        app.run("cloneSource.overlay", json!({"blend": mode})).unwrap();
        clone_source_image(&mut app).expect("the Clone Source image shows")(32, 32)
    };
    // A white source over the white image: Normal shows white, Difference black.
    assert_eq!(centre("normal"), Color32::WHITE);
    assert_eq!(centre("difference"), Color32::BLACK, "Difference turns black where source and image match");
    assert_eq!(centre("lighten"), Color32::WHITE);
}

/// A shaped tip clips the Clone Source image to its outline, not to the brush circle.
#[test]
fn clone_overlay_is_clipped_to_a_shaped_tip() {
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.roundness = 0.25;
    let image = clone_source_image(&mut app).unwrap();
    assert_eq!(image(32, 32).a(), 255, "the tip's centre shows the source");
    assert_eq!(image(32, 24).a(), 0, "inside the circle but outside the flat tip: clipped");
    assert!(image(25, 32).a() > 240, "the tip's long axis stays: {}", image(25, 32).a());
}

/// #4298: an adjustment layer has no pixels, so after its icon thumbnail is clicked (the
/// Layers panel no longer targets the mask) Fill, the fill keys and the Paint Bucket still
/// edit its mask, as in Photoshop, instead of failing with "not a pixel layer".
#[test]
fn fills_on_an_adjustment_layer_edit_its_mask_when_the_icon_is_targeted() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    // Click the Curves icon thumbnail: the panel targets the layer, not its mask.
    app.ui.mask_target = false;
    app.sync_mask_targets();
    assert!(!app.ui.mask_target, "the panel's target is the icon");
    let mask_at = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(10, 10, 0)
    };
    assert_eq!(paint_target(&app), json!("mask"));
    assert!(crate::menus::is_enabled(&app, "edit.fill"), "Edit > Fill (⇧F5) is enabled");
    assert!(crate::menus::is_enabled(&app, "edit.fillForeground"));
    app.ui.tool = Tool::PaintBucket;
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
    assert_eq!(mask_at(&app), 0.0, "the Paint Bucket filled the mask black");
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    assert_eq!(mask_at(&app), 1.0, "Edit > Fill filled the mask white");
    assert!(!app.ui.mask_target, "filling doesn't move the panel's target");
    // A pixel layer with an untargeted mask still paints its pixels.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = false;
    assert_eq!(paint_target(&app), json!("pixels"));
    assert!(!app.layer_mask_targeted());
}

/// #9329 PR C: Paste Into is three revisions; the canvas redraws what the whole command
/// changed (the pasted area), not the whole document as when only one revision counted.
#[test]
fn paste_into_refreshes_only_the_pasted_area() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 100, "height": 100})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("select.rect", json!({"x": 500, "y": 400, "width": 90, "height": 60})).unwrap();
    let seen = composite_revision(&mut app, &ctx, 0, None);
    let rev = app.session.active().unwrap().revision;
    app.run("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    assert!(app.session.active().unwrap().revision >= rev + 2, "several revisions");
    let px = redrawn_since(&mut app, &ctx, seen);
    assert!(px.is_some_and(|px| px > 0 && px < 800 * 600 / 4), "redraws {px:?} px of {}", 800 * 600);
}

/// The Move tool's Auto-Select pick is its own revision that changes no pixels: the drag
/// that follows still redraws only where the layer was and is.
#[test]
fn auto_select_move_is_still_bounded() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    for (x, color) in [(100, "#ff0000"), (500, "#0000ff")] {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": x, "y": 100, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let active = app.session.active().unwrap().active_layer;
    let seen = composite_revision(&mut app, &ctx, 0, None);
    let rev = app.session.active().unwrap().revision;
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, m);
    assert_ne!(app.session.active().unwrap().active_layer, active, "Auto-Select picked the red layer");
    assert_eq!(app.session.active().unwrap().revision, rev + 1, "the pick is one revision");
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 130.0, pressure: 1.0 }, m);
    let px = redrawn_since(&mut app, &ctx, seen);
    assert!(px.is_some_and(|px| px > 0 && px < 800 * 600 / 4), "redraws {px:?} px of {}", 800 * 600);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 130.0 }, m);
}

#[test]
fn a_secondary_canvas_does_not_overwrite_the_main_canvas_scale() {
    // #1943 review: a document window can sit on a display with another scale; only the
    // primary canvas records `app.ppp`, so the main mapping doesn't change under it.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.sync_views();
    let view = app.ui.views[0].clone();
    let frame = |app: &mut PhotocraftApp, ppp: f32, host: CanvasHost, primary: bool| {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, host, Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), view.clone(), primary);
        });
        out.textures_delta.clear();
    };
    frame(&mut app, 2.0, CanvasHost::main(0), true);
    assert_eq!(app.ppp, 2.0);
    // The secondary canvas is a document window's, a canvas of its own beside the main one.
    frame(&mut app, 1.0, CanvasHost { idx: 0, window: Some(1) }, false);
    assert_eq!(app.ppp, 2.0, "a secondary window's scale doesn't replace the main canvas's");
    // The main canvas mapping still uses its own scale: 100 % is 1 doc px per physical px.
    let shown = active_view(&app).unwrap();
    let step = shown.to_screen(craft::behavior::kurbo::Point::new(1.0, 0.0)) - shown.to_screen(craft::behavior::kurbo::Point::new(0.0, 0.0));
    assert!((step.x - 0.5).abs() < 1e-3, "{step:?}");
}

#[test]
fn a_secondary_canvas_hand_pans_while_a_transform_is_open() {
    // #2302 review: `app.ui.transform` is global, but only the primary canvas draws and drives
    // the box. A secondary window's Hand must still pan while the main window transforms.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.ui.tool = Tool::Hand;
    app.sync_views();
    crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
    let box_key = |app: &PhotocraftApp| app.transform_preview.as_ref().map(|p| crate::transform_tool::preview_key(p.session));
    let box_before = box_key(&app);
    assert!(app.ui.transform.is_some() && box_before.is_some(), "the box is open");
    let mut view0 = app.ui.views[0].clone();
    view0.zoom = 1.0;
    view0.center = [200.0, 150.0];
    view0.clear_pending();
    let center0 = view0.center;
    // The same document in a document window of its own, whose canvas is not the main window's.
    let window = CanvasHost { idx: 0, window: Some(1) };
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> View {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut seen = view0.clone();
        let mut out = ctx.run_ui(input, |ui| {
            (seen, _) = canvas_view(app, ui, window, rect, view0.clone(), false);
        });
        out.textures_delta.clear();
        seen
    };
    let start = Pos2::new(200.0, 150.0);
    let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![egui::Event::PointerMoved(start)]);
    frame(&mut app, vec![press(start, true)]);
    for dx in [10.0, 20.0, 40.0] {
        frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(dx, 0.0))]);
    }
    let dragged = frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(60.0, 0.0))]);
    frame(&mut app, vec![press(start + vec2(60.0, 0.0), false)]);
    assert_ne!(dragged.center, center0, "a secondary window's Hand pans while transforming");
    assert!(app.ui.transform.is_some(), "the box is still open");
    assert_eq!(box_key(&app), box_before, "the drag didn't touch the box");
}

/// The canvas can paint a batch as a nested list, which keeps its place in egui's final paint order.
fn flattened_paint_shapes(output: &egui::FullOutput) -> Vec<&egui::Shape> {
    fn open<'a>(shape: &'a egui::Shape, into: &mut Vec<&'a egui::Shape>) {
        match shape {
            egui::Shape::Vec(list) => list.iter().for_each(|shape| open(shape, into)),
            shape => into.push(shape),
        }
    }
    let mut shapes = Vec::new();
    for clipped in &output.shapes {
        open(&clipped.shape, &mut shapes);
    }
    shapes
}

#[test]
fn guides_draw_over_the_transform_preview_and_under_its_box() {
    // #5086: a guide crossing a layer under Free Transform was hidden behind the preview of
    // the moving pixels. The preview is image content, drawn before the grid and guides like
    // the document is; the box and handles still go over them, as in Photoshop. The document
    // picture and the controls are on different paint layers, so the order is read from the
    // pass's shapes, which egui hands over merged in paint order. The canvas's picture slot holds the document and the
    // preview as one list shape at its place, so the order is read from the shapes with every list opened in place.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 150})).unwrap();
    app.ui.extras.guides = true;
    app.ui.extras.rulers = false;
    app.sync_views();
    crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [200.0, 150.0];
    view.clear_pending();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        canvas_view(&mut app, ui, CanvasHost::main(0), rect, view.clone(), true);
    });
    out.textures_delta.clear();
    // At 100 % centred on the document, screen and document coordinates coincide.
    let layer = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 160.0));
    let near = |r: Rect| (r.min - layer.min).length() < 1.0 && (r.max - layer.max).length() < 1.0;
    let shapes = flattened_paint_shapes(&out);
    let position = |pred: &dyn Fn(&egui::Shape) -> bool| shapes.iter().position(|s| pred(s));
    // The moving pixels: the mesh that spans the layer's box.
    let preview = position(&|s| matches!(s, egui::Shape::Mesh(m) if near(m.calc_bounds())));
    // The guide: a vertical line at x = 150, which the framework's rule puts on a pixel centre (within a pixel).
    let guide = position(&|s| matches!(s, egui::Shape::LineSegment { points, .. } if points.iter().all(|p| (p.x - 150.0).abs() <= 0.5)));
    // The box: the closed four-point outline round the layer.
    let frame = position(&|s| matches!(s, egui::Shape::Path(p) if p.closed && p.points.len() == 4 && near(Rect::from_points(&p.points))));
    let (Some(preview), Some(guide), Some(frame)) = (preview, guide, frame) else {
        panic!("all painted: preview {preview:?}, guide {guide:?}, box {frame:?}");
    };
    assert!(preview < guide, "the guide ({guide}) draws over the transform preview ({preview})");
    assert!(guide < frame, "the box ({frame}) draws over the guide ({guide})");
}

#[test]
fn marching_ants_draw_over_a_guide_on_the_selection_edge() {
    use craft::behavior::kurbo::Point;

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 100})).unwrap();
    app.ui.tool = Tool::RectMarquee;
    app.ui.extras.rulers = false;
    app.ui.extras.grid = false;
    app.ui.extras.guides = true;
    app.ui.view.extras = true;
    app.ui.view.show.canvas_guides = true;
    app.ui.view.show.selection_edges = true;
    app.sync_views();

    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [200.0, 150.0];
    view.clear_pending();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let draw = |app: &mut PhotocraftApp| {
        let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, CanvasHost::main(0), rect, view.clone(), true);
        });
        output.textures_delta.clear();
        output
    };

    let output = draw(&mut app);
    let shown = active_view(&app).expect("the canvas view");
    let edge = Rect::from_two_pos(shown.to_screen(Point::new(100.0, 100.0)), shown.to_screen(Point::new(100.0, 160.0)));
    let pixel = output.pixels_per_point.recip();
    let is_guide = |shape: &egui::Shape| {
        let egui::Shape::LineSegment { points, .. } = shape else { return false };
        points.iter().all(|p| (p.x - edge.left()).abs() <= pixel)
            && points[0].y.min(points[1].y) < edge.top() - pixel
            && points[0].y.max(points[1].y) > edge.bottom() + pixel
    };
    let is_ants = |shape: &egui::Shape| {
        let bounds = match shape {
            egui::Shape::Mesh(mesh) => mesh.calc_bounds(),
            egui::Shape::Rect(rect) => rect.rect,
            egui::Shape::LineSegment { points, .. } => Rect::from_two_pos(points[0], points[1]),
            _ => return false,
        };
        // A strip or dash on the selection edge, allowing for the framework's device-pixel snap.
        bounds.height() > 0.0
            && bounds.width() <= 2.0 * pixel
            && (bounds.center().x - edge.left()).abs() <= pixel
            && bounds.top() >= edge.top() - pixel
            && bounds.bottom() <= edge.bottom() + 2.0 * pixel
    };
    let shapes = flattened_paint_shapes(&output);
    let guide = shapes.iter().position(|shape| is_guide(shape)).expect("the guide spanning the selection");
    let ants = shapes.iter().position(|shape| is_ants(shape)).expect("the ants along the left edge");
    assert!(guide < ants, "the guide ({guide}) paints before the ants ({ants})");

    app.ui.view.show.selection_edges = false;
    let output = draw(&mut app);
    let shapes = flattened_paint_shapes(&output);
    assert!(shapes.iter().any(|shape| is_guide(shape)), "hiding selection edges leaves the guide");
    assert!(!shapes.iter().any(|shape| is_ants(shape)), "the edge paint belongs to the ants");
}

#[test]
fn layer_style_dialog_previews_live_and_cancel_restores() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let fx = |d: &Document| d.layers.iter().map(|l| l.effects.items.len()).sum::<usize>();
    let id = crate::layer_style::open(&mut app, Some("colorOverlay")).expect("the dialog opens");
    let (shown, key) = display_doc(&mut app, 0);
    assert_eq!((fx(&shown), fx(&app.session.documents()[0].doc)), (1, 0), "previewed, not committed");
    // Adding a stroke through the dialog's instance list re-renders the canvas.
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<craft::imaging::ui::layer_style::HostedLayerStyle>(id) {
        let mut params = craft::imaging::layer_style::StyleParams::default();
        params.set("size", craft::imaging::layer_style::StyleValue::Number(3.0));
        dialog.draft_mut().add_instance(craft::imaging::layer_style::EffectKind::Stroke, &params);
    }
    let (shown, key2) = display_doc(&mut app, 0);
    assert_eq!(fx(&shown), 2);
    assert_ne!(key, key2, "an edit re-renders the canvas");
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    let (shown, key) = display_doc(&mut app, 0);
    assert_eq!((fx(&shown), key), (0, 0));
    assert!(app.style_preview.is_none());
}

/// Layer Style opened while Free Transform is open applies the box first, as Photoshop does,
/// so the dialog previews on the transformed layer (#4821): a dropped file (placed in the box,
/// untouched) and a box opened by hand and moved.
#[test]
fn layer_style_opened_during_free_transform_applies_the_box_and_previews() {
    use craft::behavior::geometry::BoxState;
    use craft::behavior::{EditEvent, Mixed};
    use craft::draw::ui::transform::TransformEdit;
    for placed in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.sync_views();
        let ctx = app.ctx().clone();
        let dx = if placed {
            crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
            0
        } else {
            crate::transform_tool::begin(&mut app, &ctx).unwrap();
            let moved = BoxState::from_rect(craft::behavior::kurbo::Rect::new(28.0, 8.0, 44.0, 24.0));
            let session = &mut app.ui.transform.as_mut().unwrap().session;
            let before = Mixed::Same(*session.state());
            session.apply(TransformEdit::Box(EditEvent::set(before, moved, "Move".into(), None)), 0.0);
            20
        };
        crate::layer_style::open(&mut app, Some("dropShadow")).expect("the dialog opens");
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "placed {placed}: the box is applied");
        let (shown, _) = display_doc(&mut app, 0);
        let rect = photocraft_geom::Rect::new(0, 0, 64, 64);
        let with = photocraft_compose::render(&shown, rect);
        let without = photocraft_compose::render(&app.session.documents()[0].doc, rect);
        assert!(
            with.get(16 + dx, 16)[0] < 0.1 && with.get(16 + dx, 16)[2] > 0.9,
            "placed {placed}: the blue block where the box left it, not the white under it"
        );
        // The shadow darkens the white around the block, which the document doesn't have yet.
        let block = |x: i32, y: i32| (8 + dx..24 + dx).contains(&x) && (8..24).contains(&y);
        let shadow =
            (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| !block(x, y) && without.get(x, y)[0] - with.get(x, y)[0] > 0.05).count();
        assert!(shadow > 20, "placed {placed}: the Drop Shadow is previewed ({shadow} px)");
    }
}

#[test]
fn damage_grows_by_nested_effect_reach() {
    use photocraft_doc::{Effect, Layer};
    let fmt = photocraft_color::PixelFormat::RGBA8;
    assert_eq!(effect_reach(&[Layer::raster("plain", fmt)]), 0);
    let mut inner = Layer::raster("inner", fmt);
    inner.effects.items.push(Effect::default_drop_shadow());
    let m = photocraft_compose::effects::margin(&inner);
    let mut group = Layer::group("g", vec![inner.clone()]);
    group.effects.items.push(Effect::default_drop_shadow());
    assert_eq!(effect_reach(std::slice::from_ref(&inner)), m);
    // A child's edit moves the group's shape, whose effects reach further.
    assert_eq!(effect_reach(&[group.clone()]), 2 * m);
    group.visible = false;
    assert_eq!(effect_reach(&[group]), 0);
}

#[test]
fn tab_reorder_keeps_floating_documents_in_place() {
    let visible = [0, 2];
    assert_eq!(move_target_after_filter(&visible, 0, 1, 3), 2);
    assert_eq!(move_target_after_filter(&visible, 2, 0, 3), 0);
}

/// The `document` param naming the open document at index `i`: floating windows are keyed by document id.
fn doc_param(app: &PhotocraftApp, i: usize) -> serde_json::Value {
    serde_json::to_value(app.session.documents()[i].doc.id).unwrap()
}

/// Window › Arrange › Float in Window for the open document at index `i`.
fn float_doc(app: &mut PhotocraftApp, ctx: &egui::Context, i: usize) -> Result<serde_json::Value, String> {
    let document = doc_param(app, i);
    crate::menus::invoke(app, ctx, "window.arrange.floatInWindow", json!({"document": document}))
}

/// The id of the first floating document window.
fn first_window(app: &PhotocraftApp) -> u64 {
    app.ui.document_windows.floating[0].0.id
}

#[test]
fn torn_off_document_is_not_drawn_again_in_the_main_window() {
    use egui_kittest::kittest::{NodeT, Queryable};
    for kind in [crate::theme::AppTheme::pro(), crate::theme::AppTheme::studio()] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60, "name": "floating"})).unwrap();
        app.run("file.new", json!({"width": 80, "height": 60, "name": "docked"})).unwrap();
        app.sync_views();
        let ctx = app.ctx().clone();
        float_doc(&mut app, &ctx, 0).unwrap();
        assert!(crate::view_cmds::document_is_floating(&app, app.session.documents()[0].doc.id));
        assert_eq!(app.session.active_index(), Some(0), "{kind:?}: the floated page remains active");
        assert_eq!(main_canvas_document(&app), Some(1), "{kind:?}: the main canvas picks a docked page instead");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).with_context(app.ctx().clone()).build_ui_state(
            |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind.clone());
        h.state_mut().1 = true;
        h.run_steps(3);
        // The main tab strip's tabs, by the label each one exposes.
        let tabs: Vec<String> = h.query_all_by_role(egui::accesskit::Role::Tab).filter_map(|n| n.accesskit_node().label()).collect();
        assert_eq!(tabs.len(), 1, "{kind:?}: only the docked document stays on the main tab strip: {tabs:?}");
        assert!(tabs[0].contains("docked"), "{kind:?}: the tab is the docked document's: {tabs:?}");
    }
}

/// The zoom typed into a zoom field: the field's percent, kept inside the zoom range of the document.
fn type_zoom(view: &mut View, percent: f32) {
    view.zoom = ZoomLadder::clamp(f64::from(percent) / 100.0, Size::new(f64::from(view.doc_size[0]), f64::from(view.doc_size[1]))) as f32;
    view.clear_pending();
}

/// #11276: Float All in Windows leaves a document that already floats in its one window; a
/// second floating window would share the document's view and overwrite its centre.
#[test]
fn float_all_keeps_one_window_per_floating_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "one"})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300, "name": "two"})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    let first = float_doc(&mut app, &ctx, 0).unwrap()["window"].as_u64();
    for _ in 0..2 {
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
        let window_of = |app: &PhotocraftApp, doc: usize| {
            let id = app.session.documents()[doc].doc.id;
            let holding: Vec<u64> = app.ui.document_windows.floating.iter().filter(|(g, _)| g.documents.contains(&id)).map(|(g, _)| g.id).collect();
            assert_eq!(holding.len(), 1, "document {doc}: one floating window");
            holding[0]
        };
        assert_eq!(Some(window_of(&app, 0)), first, "the floating document keeps its window");
        window_of(&app, 1);
    }
}

/// #11276: the zoom a floating document took (its window draws the document's own view) stays
/// when the windows are consolidated back into tabs.
#[test]
fn consolidating_keeps_the_zoom_set_while_floating() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "floating"})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    float_doc(&mut app, &ctx, 0).unwrap();
    type_zoom(&mut app.ui.views[0], 200.0);
    crate::menus::invoke(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert!(app.ui.document_windows.floating.is_empty());
    assert!((app.ui.views[0].zoom - 2.0).abs() < 1e-3, "the tab shows the floating zoom: {}", app.ui.views[0].zoom);
}

/// #11276: a New Window for Document (not floating) keeps a view of its own beside the main
/// canvas: zooming either leaves the other as it was.
#[test]
fn a_new_window_for_document_keeps_its_own_view() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "doc"})).unwrap();
    app.sync_views();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "window.arrange.newWindowForDocument", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                extra_windows(app, ui);
            }
        },
        (app, false),
    );
    h.state_mut().1 = true;
    h.run_steps(4);
    assert!(!h.state().0.ui.document_windows.floating.is_empty(), "the extra window is open");
    let window = first_window(&h.state().0);
    let window_zoom = |h: &egui_kittest::Harness<'_, (Box<PhotocraftApp>, bool)>| h.state().0.ui.window_views[&window].zoom;
    let before = window_zoom(&h);
    type_zoom(&mut h.state_mut().0.ui.views[0], 300.0);
    h.run_steps(3);
    assert!((h.state().0.ui.views[0].zoom - 3.0).abs() < 1e-3, "the main canvas keeps its zoom");
    assert!((window_zoom(&h) - before).abs() < 1e-3, "the extra window keeps its own: {}", window_zoom(&h));
    let main = h.state().0.ui.views[0].zoom;
    type_zoom(h.state_mut().0.ui.window_views.get_mut(&window).unwrap(), 50.0);
    h.run_steps(3);
    assert!((window_zoom(&h) - 0.5).abs() < 1e-3, "the extra window keeps its zoom");
    assert!((h.state().0.ui.views[0].zoom - main).abs() < 1e-3, "the main canvas is not written over");
}

/// #11276: in a tiled arrangement the active document's cell zooms from the Navigator and the
/// status bar too.
#[test]
fn a_tiled_document_follows_navigator_and_status_bar_zoom() {
    for layout in ["twoUpVertical", "tileAllVertically", "fourUp"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["one", "two", "three"] {
            app.run("file.new", json!({"width": 400, "height": 300, "name": name})).unwrap();
        }
        app.sync_views();
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &app_ctx, &format!("window.arrange.{layout}"), json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
            |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        h.state_mut().1 = true;
        h.run_steps(4);
        let active = h.state().0.session.active_index().unwrap();
        for pct in [200.0, 50.0] {
            type_zoom(&mut h.state_mut().0.ui.views[active], pct);
            h.run_steps(3);
            let zoom = h.state().0.ui.views[active].zoom;
            assert!((zoom - pct / 100.0).abs() < 1e-3, "{layout} {pct}%: {zoom}");
        }
    }
}

/// #11276: a document floating in its own window zooms from the zoom typed into the status bar
/// and into the Navigator, as a tabbed one does; the window doesn't keep or write back an older
/// copy of the view. #16415: so does the zoom typed into the floating window's own status bar.
/// Only the typed zoom fields are driven here, not the Navigator's slider or zoom buttons.
#[test]
fn a_floating_document_follows_navigator_and_status_bar_zoom() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300, "name": "floating"})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300, "name": "docked"})).unwrap();
        app.sync_views();
        app
    });
    h.run_steps(6);
    // Floated once the window has its size, as from the menu: the floating window takes its size from the window's
    // content area, which egui doesn't know before the first frame.
    let ctx = h.ctx.clone();
    float_doc(h.state_mut(), &ctx, 0).unwrap();
    h.run_steps(6);
    // The panel is shown, not toggled: the menu entry closes a Navigator the workspace already shows.
    crate::panel_docking::show_panel(h.state_mut(), "navigator", Some(true)).unwrap();
    // A Navigator in a column of icons shows in a flyout, which the click into the status bar closes: expand its column
    // so the panel stays while both fields are typed into.
    let navigator = "navigator".to_owned();
    if h.state().ui.docking.layout().opens_in_flyout(&navigator) {
        crate::panel_docking::apply(h.state_mut(), vec![craft::behavior::dock::Action::set_iconic(navigator.clone(), false)]).unwrap();
        h.state_mut().ui.flyout = None;
        crate::panel_docking::show_panel(h.state_mut(), "navigator", Some(true)).unwrap();
    }
    h.run_steps(6);
    assert!(crate::panel_docking::is_front(h.state(), "navigator"), "the Navigator shows as a panel");
    assert_eq!(h.state().session.active_index(), Some(0), "the floating document is active");
    // The status bar's zoom field (bottom left) and the Navigator's.
    let fields = |h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>| {
        // The fields are the nodes named Zoom that show the percentage as their value (a field's group and the Navigator's slider carry the name too).
        let mut f: Vec<egui::Rect> = h
            .query_all_by_label("Zoom")
            .filter(|n| {
                let a = n.accesskit_node();
                // A node without bounds is not on screen (nothing to click).
                a.role() != egui::accesskit::Role::Slider && a.value().is_some() && a.bounding_box().is_some()
            })
            .map(|n| n.rect())
            .collect();
        f.sort_by(|a, b| a.left().total_cmp(&b.left()));
        f
    };
    // The main status bar's field lies lowest, the Navigator's furthest right, the floating window's status bar's is the
    // third.
    let pick = |f: &[egui::Rect], which: &str| -> egui::Rect {
        let lowest = f.iter().copied().max_by(|a, b| a.bottom().total_cmp(&b.bottom())).unwrap_or(egui::Rect::NOTHING);
        let right = f.iter().copied().max_by(|a, b| a.left().total_cmp(&b.left())).unwrap_or(egui::Rect::NOTHING);
        match which {
            "status" => lowest,
            "navigator" => right,
            _ => f.iter().copied().find(|r| *r != lowest && *r != right).unwrap_or(egui::Rect::NOTHING),
        }
    };
    assert_eq!(fields(&h).len(), 3, "the status bar's, the Navigator's and the floating window's zoom fields");
    for (field, typed, want) in [("status", "200", 2.0), ("navigator", "50", 0.5), ("window", "300", 3.0), ("navigator", "100", 1.0)] {
        let at = pick(&fields(&h), field).center();
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(typed.into()));
        h.key_press(egui::Key::Enter);
        // The harness steps 1/4 s a frame, so 10 frames after Enter are 2.5 s: well past the 150 ms zoom ease, with
        // frames after it in which the window could write its view back (60 frames, 15 s, before #15652).
        h.run_steps(10);
        assert!(h.state().ui.window_views.is_empty(), "the window keeps no copy of the view");
        assert!(
            (h.state().ui.views[0].zoom - want).abs() < 1e-3,
            "field {field}, typed {typed}: not written back over by the window: {}",
            h.state().ui.views[0].zoom
        );
    }
}

#[test]
fn free_transform_redraws_only_the_moving_layer_when_the_box_opens_and_closes() {
    // Opening the box swapped the canvas to a preview document with no damage for it, so a
    // gigapixel canvas recomposited in full (over a second) on every ⌘T, and again on Cancel.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(20, 20, 60, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let rev = app.session.documents()[0].revision;
    let app_ctx = app.ctx().clone();
    crate::transform_tool::begin(&mut app, &app_ctx).expect("the box opens");
    let (_, key) = display_doc(&mut app, 0);
    assert_eq!(key, crate::transform_tool::preview_key(app.transform_preview.as_ref().unwrap().session));
    let dk = canvas_display(&app, &app.session.documents()[0].doc, None).1;
    let opened = damage_since(&app, 0, (rev, dk), (rev, key), dk, (rev, None)).expect("opening the box has a damage area");
    assert!(opened.contains(20, 20) && opened.contains(59, 49), "{opened:?}");
    assert!(opened.width() < 400 && opened.height() < 200, "not the whole canvas: {opened:?}");
    let canceled = damage_since(&app, 0, (rev, key ^ dk), (rev, 0), dk, (rev, None)).expect("canceling has one too");
    assert_eq!(canceled, opened);
    // A commit (one revision later) adds what it changed; anything else redraws everything.
    let moved = photocraft_geom::Rect::new(100, 20, 140, 50);
    let committed = damage_since(&app, 0, (rev, key ^ dk), (rev + 1, 0), dk, (rev, Some(moved))).expect("committing too");
    assert!(committed.contains(20, 20) && committed.contains(139, 49), "{committed:?}");
    assert!(damage_since(&app, 0, (rev, key ^ dk), (rev + 2, 0), dk, (rev + 1, Some(moved))).is_none(), "a later revision isn't ours");
}

mod gpu_tests {
    use super::*;

    #[test]
    fn gpu_display_luts_are_per_display() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut app = app_on_two_displays();
        app.set_wgpu(rs);
        let doc = app.session.documents()[0].doc.clone();
        let key = doc.id.0;
        // sRGB on the sRGB display needs no LUT; on the P3 display it does.
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
        let gpu = app.gpu.clone().unwrap();
        let (s1, s4) = (gpu.display_lut_signature(key, 1).unwrap(), gpu.display_lut_signature(key, 4).unwrap());
        assert_ne!(s1.0, s4.0);
        assert!(gpu.has_display_lut(key, 4) && !gpu.has_display_lut(key, 1));
        // Drawing one window leaves the other's LUT alone.
        for _ in 0..3 {
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
        }
        assert_eq!((gpu.display_lut_signature(key, 1), gpu.display_lut_signature(key, 4)), (Some(s1), Some(s4)));
        assert!(gpu.has_display_lut(key, 4));
        // The shared texture's key is the same for both displays.
        assert_eq!(texture_key(canvas_display(&app, &doc, Some(1)).0.as_deref(), &doc), texture_key(canvas_display(&app, &doc, Some(4)).0.as_deref(), &doc));
    }
}
