use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 90, "height": 60, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..60 {
                for x in 0..90 {
                    let v = ((x / 6 + y / 6) % 2) as f32;
                    s.write_pixel(x, y, &[v, 0.4, 1.0 - v, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

/// Sends one `ui` object to the open Filter Gallery, as `ui.menu.invoke {"ui": …}` does.
fn ui(app: &mut PhotocraftApp, v: Value) -> Result<Value, String> {
    let ctx = app.ctx().clone();

    crate::control::dialogs::workspace_ui(app, &ctx, KEY, &v)
}

/// The open gallery's stack as the workspace holds it.
fn effects_of(app: &PhotocraftApp) -> Vec<EffectLayer> {
    serde_json::from_value(describe(app).unwrap()["effects"].clone()).unwrap()
}

fn source_of(app: &PhotocraftApp) -> Arc<PcGallerySource> {
    app.gallery.as_ref().unwrap().source.clone()
}

#[test]
fn stack_editing_and_commit_match_the_engine() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let r = crate::distort_ui::menu(&mut app, &ctx, KEY, &json!({})).unwrap().unwrap();
    assert_eq!(r["gallery"]["effects"].as_array().unwrap().len(), 1);
    ui(&mut app, json!({"filter": "cutout"})).unwrap();
    ui(&mut app, json!({"params": {"numberOfLevels": 3}})).unwrap();
    ui(&mut app, json!({"add": true})).unwrap();
    ui(&mut app, json!({"filter": "texturizer"})).unwrap();
    ui(&mut app, json!({"params": {"texture": "brick", "relief": 20}})).unwrap();
    ui(&mut app, json!({"add": true})).unwrap();
    ui(&mut app, json!({"filter": "glowingEdges"})).unwrap();
    ui(&mut app, json!({"toggle": 2})).unwrap();
    ui(&mut app, json!({"move": {"from": 1, "to": 0}})).unwrap();
    let d = describe(&app).unwrap();
    let keys: Vec<&str> = d["effects"].as_array().unwrap().iter().map(|e| e["filter"].as_str().unwrap()).collect();
    assert_eq!(keys, ["texturizer", "cutout", "glowingEdges"]);
    assert_eq!(d["effects"][2]["visible"], json!(false));
    let params = source_of(&app).command_params(&effects_of(&app));
    let before = app.session.active().unwrap().history.past_len();
    ui(&mut app, json!({"commit": true})).unwrap();
    take(&mut app, &ctx);
    assert!(app.gallery.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    let (id, p) = app.session.journal.last().cloned().unwrap();
    assert_eq!(id, KEY);
    assert_eq!(p["effects"], params["effects"]);
    // Re-opening starts from the last stack.
    crate::distort_ui::menu(&mut app, &ctx, KEY, &json!({})).unwrap().unwrap();
    assert_eq!(effects_of(&app).len(), 3);
    ui(&mut app, json!({"delete": true})).unwrap();
    assert_eq!(effects_of(&app).len(), 2);
    ui(&mut app, json!({"cancel": true})).unwrap();
    take(&mut app, &ctx);
    assert!(app.gallery.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "cancel records nothing");
}

#[test]
fn smart_filter_double_click_edits_the_stored_gallery_stack() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let stack = json!({"effects": [{"filter": "chrome", "params": {"detail": 2, "smoothness": 9}, "visible": true}], "foreground": [1.0, 0.0, 0.0, 1.0], "background": [0.0, 0.0, 1.0, 1.0]});
    app.run(KEY, stack).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2.0})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    };
    let steps = app.session.active().unwrap().history.past_len();

    let sm = smart(&app);
    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    assert_eq!(app.framework.dialogs.stack().ids().count(), 1, "only the gallery opens, no generic parameter form");
    let g = app.gallery.as_ref().expect("the gallery dialog opens");
    assert_eq!((g.smart_filter, g.source.foreground, g.source.background), (Some(0), [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]));
    let effects = effects_of(&app);
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].filter, "chrome");
    assert_eq!(describe(&app).unwrap()["effects"][0]["params"]["detail"].as_f64(), Some(2.0));
    // The preview starts from the pixels below the filter (no blur, no chrome).
    let below = photocraft_engine::smart_cmds::render_below_filter(&app.session.active().unwrap().doc, &sm, 0).unwrap().unwrap();
    let source = source_of(&app);
    assert_eq!(source.src.read_region(source.canvas), below.read_region(source.canvas));

    ui(&mut app, json!({"params": {"detail": 7}})).unwrap();
    ui(&mut app, json!({"commit": true})).unwrap();
    take(&mut app, &ctx);
    assert!(app.gallery.is_none());
    let sm = smart(&app);
    assert_eq!(sm.smart_filters.len(), 2, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].command, KEY);
    assert_eq!(sm.smart_filters[0].params["effects"][0]["params"]["detail"].as_f64(), Some(7.0));
    assert_eq!(sm.smart_filters[0].params["foreground"], json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app).smart_filters[0].params["effects"][0]["params"]["detail"].as_f64(), Some(2.0));

    // Not a gallery filter / no such filter: an error, no dialog.
    assert!(open_smart_filter(&mut app, layer, 1).is_err());
    assert!(open_smart_filter(&mut app, layer, 9).is_err());
    assert!(app.gallery.is_none());
}

#[test]
fn preview_and_thumbnails_render() {
    let mut app = app();
    open(&mut app).unwrap();
    let source = source_of(&app);
    let effects = effects_of(&app);
    let before = source.preview().revision();
    let took = paint_preview(&source.src, source.canvas, &source.command_params(&effects), source.canvas, 1, &source.preview, &Interrupt::NONE).unwrap();
    assert!(took < Duration::from_secs(60));
    assert_ne!(source.preview().revision(), before, "the landed preview moves the picture's revision");
    let thumb = source.thumb_source();
    for f in GalleryFilter::ALL {
        let key = GalleryThumb::new(f.key());
        let ThumbRender::Ready(img) = thumb.render(&key, &ThumbRequest::for_cell(80.0, 1.0)) else { panic!("{}: not rendered at once", f.key()) };
        assert_eq!(img.size, THUMB, "{}", f.key());
    }
    // Preview equals the engine's algorithm on the sampled pixels.
    let params = source.command_params(&effects);
    let layer = app.gallery.as_ref().unwrap().layer;
    let src = sample(&source.src, source.canvas, 1);
    let a = render(&src, &params);
    let b = applied(&app, layer, &params).read_region(source.canvas);
    let worst = a.read_region(source.canvas).iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
    assert!(worst < 2e-3, "preview differs from the result by {worst}");
}

#[test]
fn native_component_gallery_previews_match_commit_and_capture_the_target() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for alpha in [true, false] {
                let fixture = crate::distort_ui::component_tests::app(mode, depth);
                let mut app = if alpha { fixture } else { crate::distort_ui::component_tests::without_alpha(fixture) };
                let ctx = app.ctx().clone();
                open(&mut app).unwrap();
                ui(&mut app, json!({"filter": "cutout"})).unwrap();
                let source = source_of(&app);
                let params = source.command_params(&effects_of(&app));
                let before = source.src.as_ref().clone();
                let bounds = source.canvas;
                let layer = app.gallery.as_ref().unwrap().layer;
                let preview = preview_pixels(&before, bounds, &params, bounds, 1, &Interrupt::NONE).unwrap();
                crate::distort_ui::component_tests::protected(&before, &preview, bounds);
                let thumbnail = render(&before, &params);
                crate::distort_ui::component_tests::protected(&before, &thumbnail, bounds);
                app.run("channel.target", json!({"channel":{"color":1}})).unwrap();
                ui(&mut app, json!({"commit": true})).unwrap();
                take(&mut app, &ctx);
                let after = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
                assert_eq!(after.read_region(bounds), preview.read_region(bounds));
            }
        }
    }
}

/// A 240×160 layer with gradients, edges and fine detail, so every gallery filter has
/// something to draw.
fn photo_app() -> PhotocraftApp {
    let (w, h) = (240, 160);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": w, "height": h, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                    let disc = (((x - 150) * (x - 150) + (y - 70) * (y - 70)) < 45 * 45) as i32 as f32;
                    let stripes = ((x + y) / 11 % 2) as f32;
                    s.write_pixel(x, y, &[u * 0.8 + disc * 0.2, 0.25 + 0.5 * stripes * (1.0 - disc), v * (1.0 - disc) + disc * 0.9, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

/// What OK applies: `filter.filterGallery` with `params` on a copy of the document.
fn applied(app: &PhotocraftApp, layer: LayerId, params: &Value) -> Surface {
    let mut s2 = photocraft_engine::Session::new();
    s2.add_document(app.session.active().unwrap().doc.as_ref().clone(), None);
    s2.select_layer(layer).unwrap();
    s2.execute(KEY, params.clone()).unwrap();
    s2.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().clone()
}

/// Mean of `k`×`k` blocks (opaque layers), written independently of [`reduce`].
fn box_mean(surf: &Surface, r: Rect, k: i32) -> Vec<f32> {
    let mut out = Vec::new();
    let mut px = [0.0f32; 4];
    for by in (r.y0..r.y1).step_by(k as usize) {
        for bx in (r.x0..r.x1).step_by(k as usize) {
            let mut sum = [0.0f32; 4];
            let mut n = 0.0;
            for y in by..(by + k).min(r.y1) {
                for x in bx..(bx + k).min(r.x1) {
                    surf.read_pixel(x, y, &mut px);
                    sum.iter_mut().zip(px).for_each(|(s, v)| *s += v);
                    n += 1.0;
                }
            }
            out.extend(sum.map(|s| s / n));
        }
    }
    out
}

/// Mean absolute RGB difference.
fn mean_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let (sum, n) = a.chunks(4).zip(b.chunks(4)).fold((0.0, 0.0), |(s, n), (p, q)| (s + (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>(), n + 3.0));
    sum / n
}

#[test]
fn zoomed_out_preview_matches_the_applied_result_scaled_down() {
    // #2502: below 100 % the preview filtered every k-th pixel with the same params, so
    // strokes, grain and textures showed several times too large.
    let mut app = photo_app();
    open(&mut app).unwrap();
    let source = source_of(&app);
    let layer = app.gallery.as_ref().unwrap().layer;
    for f in ["graphicPen", "charcoal", "reticulation", "stainedGlass", "spatter", "chalkCharcoal"] {
        ui(&mut app, json!({"filter": f})).unwrap();
        let params = source.command_params(&effects_of(&app));
        let reference = box_mean(&applied(&app, layer, &params), source.canvas, 4);
        // A render request at 25 % shows the whole layer, averaged 4×4.
        let k = preview_factor(0.25);
        assert_eq!(k, 4, "{f}: at 25 % the preview averages 4×4");
        let before = source.preview().revision();
        paint_preview(&source.src, source.canvas, &params, source.canvas, k, &source.preview, &Interrupt::NONE).unwrap();
        assert_ne!(source.preview().revision(), before, "{f}: the render landed");
        let vis = source.canvas;
        let shown = preview_pixels(&source.src, source.canvas, &params, vis, k, &Interrupt::NONE).unwrap();
        let (w, h) = (reduced(vis.width(), k) as i32, reduced(vis.height(), k) as i32);
        let diff = mean_diff(&shown.read_region(Rect::new(0, 0, w, h)), &reference);
        // The old preview (every 4th pixel, filtered at that size) for comparison.
        let old = mean_diff(&render(&sample(&source.src, vis, k), &params).read_region(Rect::new(0, 0, w, h)), &reference);
        assert!(diff < 1e-3, "{f}: preview differs from the applied result by {diff}");
        assert!(old > 20.0 * diff.max(1e-3), "{f}: the old preview differed by only {old}");
    }
}

#[test]
fn a_region_renders_the_same_pixels_as_the_full_result() {
    // Zoomed in, only the visible part renders: noise, cells and textures are seeded by
    // document position, and the filters read their reach around the region.
    let mut app = photo_app();
    open(&mut app).unwrap();
    let source = source_of(&app);
    let layer = app.gallery.as_ref().unwrap().layer;
    let region = Rect::new(37, 21, 151, 117);
    for f in GalleryFilter::ALL {
        ui(&mut app, json!({"filter": f.key()})).unwrap();
        let params = source.command_params(&effects_of(&app));
        let full = applied(&app, layer, &params).read_region(region);
        let part = preview_pixels(&source.src, source.canvas, &params, region, 1, &Interrupt::NONE).unwrap();
        let part = part.read_region(Rect::new(0, 0, region.width() as i32, region.height() as i32));
        let worst = full.iter().zip(&part).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-3, "{}: region differs from the full result by {worst}", f.key());
    }
}

#[test]
fn reduce_averages_blocks_weighted_by_alpha() {
    let mut s = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true));
    s.write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
    s.write_pixel(1, 0, &[0.0, 1.0, 0.0, 0.0]); // transparent: its colour doesn't count
    s.write_pixel(2, 0, &[0.2, 0.4, 0.6, 1.0]); // a partial block at the edge
    let r = reduce(&s, Rect::new(0, 0, 3, 2), 2).read_region(Rect::new(0, 0, 2, 1));
    assert_eq!(r, [1.0, 0.0, 0.0, 0.25, 0.2, 0.4, 0.6, 0.5]);
    assert_eq!(reduce(&s, Rect::EMPTY, 2).content_bounds(), Rect::EMPTY);
}

#[test]
fn background_preview_renders_off_the_ui_thread() {
    let mut app = photo_app();
    open(&mut app).unwrap();
    ui(&mut app, json!({"filter": "graphicPen"})).unwrap();
    let source = source_of(&app);
    let before = source.preview().revision();
    let params = source.command_params(&effects_of(&app));
    let ui_thread = std::thread::current().id();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || {
        let s = &worker_source;
        (std::thread::current().id(), paint_preview(&s.src, s.canvas, &params, s.canvas, 1, &s.preview, &Interrupt::NONE))
    });
    let (thread, done) = worker.join().unwrap();
    assert_ne!(thread, ui_thread);
    done.unwrap();
    assert_ne!(source.preview().revision(), before, "the landing bumps the preview raster's revision");
}

/// Preview cost on 24 MP: `cargo test -p photocraft-ui-egui --lib bench_gallery_preview -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_gallery_preview() {
    let (w, h) = (6000, 4000);
    let canvas = Rect::new(0, 0, w, h);
    let mut s = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true));
    s.fill_rect(canvas, &[0.5, 0.4, 0.3, 1.0]);
    for y in (0..h).step_by(9) {
        s.fill_rect(Rect::new(0, y, w, y + 3), &[0.9, 0.8, 0.2, 1.0]);
    }
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    std::hint::black_box(reduce(&s, canvas, 8));
    println!("{:>14}: {:>7.0} ms", "reduce 1/8", ms(t));
    for f in ["graphicPen", "charcoal", "reticulation", "stainedGlass", "spatter", "roughPastels"] {
        let p = json!({"effects": [{"filter": f}]});
        let t = std::time::Instant::now();
        std::hint::black_box(render(&sample(&s, canvas, 8), &p));
        let old = ms(t);
        let t = std::time::Instant::now();
        std::hint::black_box(preview_pixels(&s, canvas, &p, canvas, 8, &Interrupt::NONE));
        let fit = ms(t);
        let t = std::time::Instant::now();
        std::hint::black_box(preview_pixels(&s, canvas, &p, Rect::new(2400, 1600, 3500, 2400), 1, &Interrupt::NONE));
        let screen = ms(t);
        println!("{f:>14}: old fit {old:>5.0} ms, new fit {fit:>6.0} ms, new 100 % (1100x800) {screen:>5.0} ms");
    }
}

/// The Glow Color swatch asks the app for a colour; the dialog's floats stay exact through OK and
/// Cancel on the opening colour and change only on a new pick. The command records the colour linear.
#[test]
fn glow_color_keeps_exact_floats_until_a_new_pick_and_stores_linear() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    open(&mut app).unwrap();
    ui(&mut app, json!({"filter": "neonGlow"})).unwrap();
    ui(&mut app, json!({"params": {"glowColor": [0.123456, 0.234567, 0.345678]}})).unwrap();
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder().with_context(ctx.clone()).with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            app.framework.dialogs.run(ui);
            take(app, ui.ctx());
        },
        app,
    );
    let glow = |app: &PhotocraftApp| -> Vec<f64> {
        describe(app).unwrap()["effects"][0]["params"]["glowColor"].as_array().unwrap().iter().filter_map(Value::as_f64).collect()
    };
    let exact = [0.123456, 0.234567, 0.345678];
    let near = |got: &[f64], want: &[f64], eps: f64| got.iter().zip(want).all(|(a, b)| (a - b).abs() < eps);
    h.run_steps(3);
    // OK on the opening colour keeps the floats.
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Glow Color").click();
    h.run_steps(3);
    assert_eq!(h.state().framework.dialogs.stack().ids().count(), 2, "the swatch opens the colour picker as a child");
    let picker = |app: &PhotocraftApp| app.framework.dialogs.stack().top().expect("the colour picker is on top");
    let id = picker(h.state());
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert!(near(&glow(h.state()), &exact, 1e-6), "{:?}", glow(h.state()));
    // Cancel keeps them too.
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Glow Color").click();
    h.run_steps(3);
    let id = picker(h.state());
    crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert!(near(&glow(h.state()), &exact, 1e-6), "{:?}", glow(h.state()));
    // A new pick stores the linear colour.
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Glow Color").click();
    h.run_steps(3);
    let id = picker(h.state());
    crate::color_picker_ui::testing::set_color(h.state_mut(), id, "#8040ff");
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let recorded = |app: &PhotocraftApp| -> Vec<f64> {
        let g = app.gallery.as_ref().expect("the gallery is open");
        let hosted = app.framework.dialogs.get_by_id::<Hosted>(g.id).expect("the gallery dialog");
        g.source.command_params(&hosted.workspace().session().effects)["effects"][0]["params"]["glowColor"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_f64)
            .collect()
    };
    assert!(near(&recorded(h.state())[..3], &[0.21586, 0.05127, 1.0], 1e-3), "{:?}", recorded(h.state()));
}
