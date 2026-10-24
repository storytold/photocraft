//! A 100 % filter preview uploads only what the view shows, and pans add the rest (#5772).

use super::*;
use serde_json::json;

fn render_state() -> Option<eframe::egui_wgpu::RenderState> {
    match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
        Ok(rs) => Some(rs),
        Err(error) => {
            if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping filter preview view test: no GPU adapter");
            None
        }
    }
}

/// A `depth`-bit document of coloured stripes with Solarize's dialog open, previewed at 100 %.
fn previewing(rs: eframe::egui_wgpu::RenderState, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.background_jobs = false;
    app.run("file.new", json!({"width": 2048, "height": 1536, "depth": depth})).unwrap();
    app.session
        .edit("Fixture", |doc, _| {
            let s = doc.layers[0].surface_mut().unwrap();
            for i in 0..32 {
                let v = i as f32 / 31.0;
                s.fill_rect(DRect::new(i * 64, 0, i * 64 + 64, 1536), &[v, 1.0 - v, (i % 3) as f32 / 2.0, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 1.0;
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("a preview");
    assert_eq!((k, size), (1, [2048, 1536]), "a full-resolution preview");
    app
}

/// The preview texture matches a whole upload of the result inside `region`.
fn assert_preview_pixels(app: &mut PhotocraftApp, region: DRect) {
    let gpu = app.gpu.clone().unwrap();
    let p = app.filter_preview.as_ref().unwrap();
    let (key, result) = (p.key.doc.0 ^ (1u64 << 61), p.result.clone().unwrap());
    let reference = p.key.doc.0 ^ (1u64 << 58);
    let (display, _) = canvas_display(app, &result, None);
    gpu.upload_buffer_full(reference, &texture_buffer(display.as_deref(), &photocraft_compose::flatten(&result)), result.depth);
    let (_, size, shown) = gpu.read_texels(key).unwrap();
    let (_, _, want) = gpu.read_texels(reference).unwrap();
    assert_eq!(size, [2048, 1536]);
    for y in region.y0..region.y1 {
        for x in region.x0..region.x1 {
            let i = y as usize * size[0] as usize + x as usize;
            assert_eq!(shown[i], want[i], "preview differs at ({x},{y})");
        }
    }
}

fn area(r: DRect) -> u64 {
    u64::from(r.width()) * u64::from(r.height())
}

#[test]
fn a_full_resolution_preview_uploads_the_view_then_what_a_pan_uncovers() {
    let _guard = photocraft_testkit::gpu_lock();
    for depth in [8, 16, 32] {
        let Some(rs) = render_state() else { return };
        let mut app = previewing(rs, depth);
        let key = app.filter_preview.as_ref().unwrap().key.doc.0 ^ (1u64 << 61);
        assert!(app.filter_preview.as_ref().unwrap().uploaded.is_empty(), "{depth}-bit: nothing is uploaded before a view asks");

        // The first view: itself plus the margin, not the whole result.
        let a = DRect::new(300, 200, 900, 600);
        upload_filter_preview_view(&mut app, key, a);
        let first = app.filter_preview.as_ref().unwrap().uploaded;
        assert!(first.contains_rect(&a.inflate(FILTER_PREVIEW_MARGIN).intersect(&DRect::new(0, 0, 2048, 1536))));
        assert!(area(first) < 2048 * 1536 / 2, "{depth}-bit: {first:?} is the view's part");
        assert_eq!(app.perf.last_refresh_px, area(first));
        assert_preview_pixels(&mut app, a);

        // The same view again: nothing new.
        app.perf.last_refresh_px = 0;
        upload_filter_preview_view(&mut app, key, a);
        assert_eq!(app.perf.last_refresh_px, 0, "{depth}-bit: an unchanged view uploads nothing");

        // A jump to the far corner, away from what is there, uploads that view alone, not the
        // gap between them.
        let c = DRect::new(1700, 1300, 2048, 1536);
        upload_filter_preview_view(&mut app, key, c);
        let second = app.filter_preview.as_ref().unwrap().uploaded;
        assert!(second.contains_rect(&c) && second.intersect(&first).is_empty(), "{depth}-bit: {second:?}");
        assert_eq!(app.perf.last_refresh_px, area(second), "{depth}-bit: the new view only");
        assert_preview_pixels(&mut app, c);

        // A pan from there that overlaps uploads only what the texture didn't have.
        let b = DRect::new(1300, 900, 1900, 1300);
        upload_filter_preview_view(&mut app, key, b);
        let third = app.filter_preview.as_ref().unwrap().uploaded;
        assert!(third.contains_rect(&second) && third.contains_rect(&b));
        assert_eq!(app.perf.last_refresh_px, area(third) - area(second), "{depth}-bit: only the uncovered strips");
        assert_preview_pixels(&mut app, b);
        assert_preview_pixels(&mut app, c);

        // A new result starts again from what the view shows.
        app.filter_preview = None;
        ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("a new preview");
        assert!(app.filter_preview.as_ref().unwrap().uploaded.is_empty(), "{depth}-bit: a new result resets the upload");
    }
}

/// A proxy preview is already small and still goes up whole.
#[test]
fn a_proxy_preview_still_uploads_whole() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.background_jobs = false;
    app.run("file.new", json!({"width": 2400, "height": 2200})).unwrap();
    crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("a preview");
    assert_eq!((k, size), (2, [1200, 1100]));
    assert_eq!(app.filter_preview.as_ref().unwrap().uploaded, DRect::new(0, 0, 1200, 1100));
    // After Retry GPU (which clears the record) it goes up whole again, onto the new canvas.
    let key = app.filter_preview.as_ref().unwrap().key.doc.0 ^ (1u64 << 61);
    app.filter_preview.as_mut().unwrap().uploaded = DRect::EMPTY;
    upload_filter_preview_view(&mut app, key, DRect::EMPTY);
    assert_eq!(app.filter_preview.as_ref().unwrap().uploaded, DRect::new(0, 0, 1200, 1100), "uploaded again");
}

#[test]
fn missing_rects_cover_exactly_what_is_new() {
    let target = DRect::new(0, 0, 100, 80);
    for have in [DRect::EMPTY, DRect::new(10, 10, 50, 40), DRect::new(0, 0, 100, 40), target, DRect::new(200, 200, 300, 300)] {
        let parts = missing_rects(target, have);
        let inside = have.intersect(&target);
        assert_eq!(parts.iter().map(|r| area(*r)).sum::<u64>(), area(target) - area(inside), "{have:?}");
        for (i, a) in parts.iter().enumerate() {
            assert!(target.contains_rect(a) && a.intersect(&inside).is_empty());
            assert!(parts[i + 1..].iter().all(|b| a.intersect(b).is_empty()), "{have:?}: parts overlap");
        }
    }
    assert_eq!(snap_out(DRect::new(-3, 5, 65, 128), 64), DRect::new(-64, 0, 128, 128));
    assert_eq!(snap_out(DRect::new(i32::MIN, 0, i32::MAX, 1), 64).x1, i32::MAX);
}

/// Through the app's frames: the canvas shows the preview wherever the view goes, with nothing
/// missing (transparent) or stale (unfiltered) after a pan.
#[test]
fn the_canvas_shows_the_full_resolution_preview_before_and_after_a_pan() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).renderer(renderer).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
        app.run("file.new", json!({"width": 4000, "height": 3000})).unwrap();
        app.run("edit.fill", json!({"color": "#1a33e6"})).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().run("view.actualPixels", json!({})).unwrap();
    crate::filter_dialog::open(h.state_mut(), "filter.stylize.solarize").unwrap();
    for centre in [None, Some([3700.0, 2800.0]), Some([300.0, 2700.0])] {
        if let Some(c) = centre {
            h.state_mut().ui.views[0].center = c;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            h.step();
            let ready = h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some());
            if ready {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the preview did not arrive");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        h.run_steps(2);
        assert_eq!(h.state().filter_preview.as_ref().unwrap().key.k, 1, "100 %: a full-resolution preview");
        let image = h.render().unwrap();
        if let Some(dir) = std::env::var_os("PHOTOCRAFT_PREVIEW_FRAMES") {
            image.save(std::path::Path::new(&dir).join(format!("preview-{}.png", centre.map_or(0.0, |c| c[0]) as u32))).unwrap();
        }
        let canvas = h.state().last_canvas_rect;
        let (mut solarized, mut unfiltered) = (0, 0);
        for i in 1..12 {
            for j in 1..8 {
                let p = canvas.min + egui::vec2(canvas.width() * i as f32 / 12.0, canvas.height() * j as f32 / 8.0);
                let px = image.get_pixel(p.x as u32, p.y as u32);
                solarized += usize::from(px[0] < 80 && px[1] < 100 && px[2] < 80);
                unfiltered += usize::from(px[2] > 200 && px[0] < 60 && px[1] < 80);
            }
        }
        assert_eq!(unfiltered, 0, "{centre:?}: no unfiltered pixels on the canvas");
        assert!(solarized >= 40, "{centre:?}: the preview covers the canvas outside the dialog ({solarized} of 77)");
    }
}

/// View › Pattern Preview draws copies of the texture around the document from anywhere in it,
/// so the whole result goes up, not only the view's part.
#[test]
fn pattern_preview_uploads_the_whole_result() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = previewing(rs, 8);
    let key = app.filter_preview.as_ref().unwrap().key.doc.0 ^ (1u64 << 61);
    app.ui.view.pattern_preview = true;
    upload_filter_preview_view(&mut app, key, DRect::new(300, 200, 900, 600));
    assert_eq!(app.filter_preview.as_ref().unwrap().uploaded, DRect::new(0, 0, 2048, 1536));
    assert_preview_pixels(&mut app, DRect::new(1500, 1100, 2048, 1536));
}

/// When the GPU can't take the upload (it isn't healthy), nothing is recorded as uploaded, so a
/// later frame uploads again instead of leaving the preview blank.
#[test]
fn a_failed_upload_is_not_recorded() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = previewing(rs, 8);
    let key = app.filter_preview.as_ref().unwrap().key.doc.0 ^ (1u64 << 61);
    app.gpu_health().unwrap().mark(photocraft_gpu::Fault::Error("Validation Error".into()));
    upload_filter_preview_view(&mut app, key, DRect::new(300, 200, 900, 600));
    assert!(app.filter_preview.as_ref().unwrap().uploaded.is_empty(), "nothing is on the texture");
}
