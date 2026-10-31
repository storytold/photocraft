//! The filter dialog's GPU preview (#5772), through the app's own frames on a real GPU.

use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::canvas::ViewXform;

fn render_state() -> Option<eframe::egui_wgpu::RenderState> {
    match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
        Ok(rs) => Some(rs),
        Err(error) => {
            if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping filter GPU preview test: no GPU adapter");
            None
        }
    }
}

/// Whether this adapter runs the GPU path (software adapters on CI may lack operators); a test
/// skips when it doesn't, unless `PHOTOCRAFT_REQUIRE_GPU_TESTS` asks for it.
fn gpu_path(h: &Harness<'static, PhotocraftApp>) -> bool {
    // A CPU-emulated adapter (WARP, llvmpipe on CI) runs the path but times out drawing frame
    // after frame: these are GPU tests.
    let software = h.state().perf.gpu_info.device_type == "cpu";
    let why = if software { Some("a software (CPU) adapter") } else { ineligible(h.state(), 0, &json!({"radius": 4.0})) };
    let Some(why) = why else { return true };
    // Only what the adapter can't do skips; anything else is a broken fixture.
    let capability = matches!(why, "no GPU canvas" | "the GPU compositor can't draw it" | "a software (CPU) adapter")
        || why.starts_with("the GPU canvas runs no layer operators");
    assert!(capability && std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "the GPU path is unavailable here: {why}");
    eprintln!("skipping filter GPU preview test: {why}");
    false
}

/// The app on a GPU canvas with a 2000×1500 document of hard-edged colour stripes (on a
/// transparent margin) at 100 %, and Gaussian Blur's dialog open at `radius`.
fn harness(rs: eframe::egui_wgpu::RenderState, radius: f64, background_jobs: bool) -> (Harness<'static, PhotocraftApp>, u64) {
    harness_sized(rs, radius, background_jobs, [2000, 1500])
}

/// [`harness`] with a `size` document (stripes over its first 2000×1500).
fn harness_sized(rs: eframe::egui_wgpu::RenderState, radius: f64, background_jobs: bool, size: [u32; 2]) -> (Harness<'static, PhotocraftApp>, u64) {
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).renderer(renderer).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = background_jobs;
        app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
        app.run("file.new", json!({"width": size[0], "height": size[1], "background": "transparent"})).unwrap();
        app.session
            .edit("Fixture", |doc, _| {
                let s = doc.layers[0].surface_mut().unwrap();
                for i in 0..30 {
                    let v = i as f32 / 29.0;
                    s.fill_rect(photocraft_geom::Rect::new(40 + i * 64, 40, 40 + i * 64 + 64, 1460), &[v, 1.0 - v, (i % 3) as f32 / 2.0, 1.0]);
                }
                Ok(())
            })
            .unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().run("view.actualPixels", json!({})).unwrap();
    let id = crate::filter_dialog::open(h.state_mut(), COMMAND).unwrap();
    set_radius(&mut h, id, radius);
    (h, id)
}

fn set_radius(h: &mut Harness<'static, PhotocraftApp>, id: u64, radius: f64) {
    let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).expect("the dialog");
    d.fields.insert("radius".into(), json!(radius));
    d.fields.insert("__preview".into(), json!(true));
}

/// Frames until `done` holds (at most a few seconds: the CPU preview computes on a worker).
fn settle(h: &mut Harness<'static, PhotocraftApp>, done: impl Fn(&mut PhotocraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        h.step();
        if done(h.state_mut()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the preview did not settle");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(2);
}

/// Document points along stripe edges (where a blur shows) that the view shows, away from the
/// dialog, and the canvas pixels there.
fn probes(h: &mut Harness<'static, PhotocraftApp>) -> Vec<((i32, i32), [u8; 4])> {
    probes_named(h, None)
}

/// [`probes`], saving the frame as `name` under `PHOTOCRAFT_PREVIEW_FRAMES` when that is set.
fn probes_named(h: &mut Harness<'static, PhotocraftApp>, name: Option<&str>) -> Vec<((i32, i32), [u8; 4])> {
    let image = h.render().unwrap();
    if let (Some(dir), Some(name)) = (std::env::var_os("PHOTOCRAFT_PREVIEW_FRAMES"), name) {
        let file: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        image.save(std::path::Path::new(&dir).join(format!("gpu-preview-{file}.png"))).unwrap();
    }
    let xf = ViewXform::active(h.state()).unwrap();
    let canvas = h.state().last_canvas_rect;
    // Floating windows (the dialog) and their shadows cover part of the canvas.
    let windows: Vec<egui::Rect> = h.ctx.memory(|m| m.layer_ids().filter(|l| l.order != egui::Order::Background).filter_map(|l| m.area_rect(l.id)).collect());
    let mut out = Vec::new();
    for i in 0..30 {
        for dx in [-3, -1, 0, 2] {
            for y in (100..1400).step_by(170) {
                let (x, y) = (40 + i * 64 + dx, y);
                let p = xf.to_screen(x as f32 + 0.5, y as f32 + 0.5);
                if canvas.shrink(4.0).contains(p) && !windows.iter().any(|w| w.expand(32.0).contains(p)) {
                    out.push(((x, y), image.get_pixel(p.x as u32, p.y as u32).0));
                }
            }
        }
    }
    out
}

/// The CPU preview of `radius` (the oracle; `None`: the document itself) around document point
/// `p`, through the canvas's display transform: each channel's range over the 3×3 pixels there
/// (the canvas samples between document pixels), and the least alpha (compared only where it's
/// opaque: the checker shows through elsewhere).
fn expected(app: &PhotocraftApp, radius: Option<f64>, p: (i32, i32)) -> ([(u8, u8); 3], f32) {
    expected_in(app, &oracle(app, radius), p)
}

/// The CPU preview of `radius` (`None`: the document itself), the oracle [`expected`] reads.
fn oracle(app: &PhotocraftApp, radius: Option<f64>) -> std::sync::Arc<Document> {
    let st = app.session.active().unwrap();
    match radius {
        Some(r) => std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": r}), 1).unwrap()),
        None => st.doc.clone(),
    }
}

/// [`expected`] from an oracle document made once.
fn expected_in(app: &PhotocraftApp, doc: &Document, p: (i32, i32)) -> ([(u8, u8); 3], f32) {
    expected_around(app, doc, p, 1)
}

/// [`expected_in`] over the pixels within `reach` of `p` (a minified view blends more of them).
fn expected_around(app: &PhotocraftApp, doc: &Document, p: (i32, i32), reach: i32) -> ([(u8, u8); 3], f32) {
    let buf = photocraft_compose::render(doc, photocraft_geom::Rect::new(p.0 - reach, p.1 - reach, p.0 + reach + 1, p.1 + reach + 1));
    let (display, _) = crate::canvas::canvas_display(app, doc, None);
    let img = crate::canvas::display_image(display.as_deref(), &buf);
    let mut range = [(255u8, 0u8); 3];
    for c in &img.pixels {
        for (k, v) in [c.r(), c.g(), c.b()].into_iter().enumerate() {
            range[k] = (range[k].0.min(v), range[k].1.max(v));
        }
    }
    (range, buf.px.iter().map(|p| p[3]).fold(1.0, f32::min))
}

/// Whether `px` lies within `range` (± 3 levels of display rounding).
fn within(range: &[(u8, u8); 3], px: [u8; 4]) -> bool {
    (0..3).all(|c| px[c].saturating_add(3) >= range[c].0 && px[c] <= range[c].1.saturating_add(3))
}

/// Every probe the canvas shows matches the CPU preview of `radius` (within display rounding),
/// and enough probes were checked to mean something.
fn assert_shows(h: &mut Harness<'static, PhotocraftApp>, radius: Option<f64>, what: &str) {
    let shown = probes_named(h, Some(what));
    let mut checked = 0;
    for (p, px) in &shown {
        let (want, alpha) = expected(h.state(), radius, *p);
        if alpha < 0.999 {
            continue;
        }
        assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
        checked += 1;
    }
    assert!(checked >= 40, "{what}: only {checked} probes");
}

#[test]
fn the_gpu_preview_matches_the_cpu_preview_through_slider_changes_and_pans() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "the CPU preview stands aside");
    assert_shows(&mut h, Some(4.0), "radius 4");

    // A slider change: the GPU runs the new radius, nothing on the CPU.
    let id = h.state().ui.dialogs.last().unwrap().id;
    set_radius(&mut h, id, 12.0);
    h.run_steps(3);
    assert!(on_gpu(h.state_mut(), 0));
    assert_shows(&mut h, Some(12.0), "radius 12");

    // A pan: the newly visible part catches up.
    h.state_mut().ui.views[0].center = [1700.0, 1200.0];
    h.run_steps(3);
    assert_shows(&mut h, Some(12.0), "panned");
}

/// The GPU fails to draw the preview (memory, a fault): the CPU preview takes over, and no frame
/// shows the layer unfiltered where the blur changes it.
#[test]
fn a_failing_gpu_preview_falls_back_to_the_cpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state().gpu.as_ref().unwrap().fail_operators(true);
    set_radius(&mut h, id, 12.0);
    let unfiltered = {
        let st = h.state().session.active().unwrap();
        st.doc.clone()
    };
    let (raw_doc, r4_doc, r12_doc) = (oracle(h.state(), None), oracle(h.state(), Some(4.0)), oracle(h.state(), Some(12.0)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        h.step();
        // Never the unfiltered layer where the blur changes it (a stripe edge).
        let doc_now = h.state().session.active().unwrap().doc.clone();
        assert!(std::sync::Arc::ptr_eq(&doc_now, &unfiltered), "nothing applied yet");
        for (p, px) in probes(&mut h) {
            let (raw, alpha) = expected_in(h.state(), &raw_doc, p);
            let (r4, _) = expected_in(h.state(), &r4_doc, p);
            let (r12, _) = expected_in(h.state(), &r12_doc, p);
            // Where the unfiltered layer can't be mistaken for either blur, the frame shows a blur.
            let apart = |b: &[(u8, u8); 3]| (0..3).any(|c| b[c].0 > raw[c].1.saturating_add(12) || raw[c].0 > b[c].1.saturating_add(12));
            if alpha > 0.999 && apart(&r4) && apart(&r12) {
                assert!(!within(&raw, px), "a frame shows the unfiltered layer at {p:?}: {px:?}");
            }
        }
        if h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the CPU preview never took over");
    }
    assert!(!on_gpu(h.state_mut(), 0), "the session stays on the CPU");
    h.run_steps(3);
    assert_shows(&mut h, Some(12.0), "CPU fallback");
}

/// Zoomed out the preview runs on the GPU on a reduced proxy (Phase 2): the CPU preview's factor,
/// and the same pixels as the CPU's proxy preview, through slider changes.
#[test]
fn zoomed_out_previews_run_on_a_gpu_proxy() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // Big enough for a reduced proxy when zoomed out.
    let (mut h, id) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "the GPU previews zoomed out");
    assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "no CPU preview");
    let (k, _, _) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
    assert_eq!(k, 2, "the CPU preview's factor");
    for radius in [4.0, 12.0] {
        set_radius(&mut h, id, radius);
        h.run_steps(4);
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": radius}), 2).unwrap())
        };
        // The proxy texture itself against the CPU's proxy preview (the exact check).
        {
            let (_, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
            let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
            assert_eq!(tsize, size);
            let cpu = photocraft_compose::flatten(&oracle);
            let worst = cpu.px.iter().zip(&texels).enumerate().fold((0.0f32, 0usize), |w, (i, (c, g))| {
                let d = (0..3).map(|k| (c[k] * c[3] - g[k]).abs()).fold(0.0, f32::max);
                if d > w.0 { (d, i) } else { w }
            });
            let (x, y) = (worst.1 % size[0] as usize, worst.1 / size[0] as usize);
            // Texels are premultiplied, the composite straight; within a level and a half.
            assert!(
                worst.0 <= 1.5 / 255.0,
                "radius {radius}: the proxy texture is {:.2}/255 off the CPU's proxy preview at ({x},{y}): cpu {:?} gpu {:?}",
                worst.0 * 255.0,
                cpu.px[worst.1],
                texels[worst.1]
            );
        }
        let shown = probes_named(&mut h, Some(&format!("proxy radius {radius}")));
        let mut checked = 0;
        for (p, px) in &shown {
            // The probes are document points; the oracle is the k = 2 proxy, minified on screen.
            let (want, alpha) = expected_around(h.state(), &oracle, (p.0 / 2, p.1 / 2), 2);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "radius {radius}: at {p:?} the canvas shows {px:?}, the CPU proxy {want:?}");
            checked += 1;
        }
        assert!(checked >= 40, "radius {radius}: only {checked} probes");
    }
}

/// OK keeps the GPU preview until the background job lands; Cancel restores the document.
#[test]
fn ok_keeps_the_preview_until_the_job_lands_and_cancel_restores() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    let revision = h.state().session.active().unwrap().revision;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = h.state().session.active().unwrap().doc.id;
    if let Some(job) = h.state().session.job_on(doc).map(|j| j.id) {
        assert!(on_gpu(h.state_mut(), 0), "kept while the job runs");
        assert!(h.state().gpu.as_ref().unwrap().has_operators(doc));
        h.state_mut().session.wait_job(job).unwrap();
    }
    h.run_steps(3);
    assert_ne!(h.state().session.active().unwrap().revision, revision, "the filter applied");
    assert!(!on_gpu(h.state_mut(), 0));
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc), "the operator went with the dialog");
    assert_shows(&mut h, None, "after OK (the applied blur is the document now)");

    // Cancel: the document shows unfiltered again.
    let id = crate::filter_dialog::open(h.state_mut(), COMMAND).unwrap();
    set_radius(&mut h, id, 30.0);
    settle(&mut h, |app| on_gpu(app, 0));
    crate::dialogs::cancel(h.state_mut(), id).unwrap();
    h.run_steps(4);
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc));
    assert_shows(&mut h, None, "after Cancel");
}

#[test]
fn ineligible_documents_and_layers_keep_the_cpu_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    assert_eq!(ineligible(&app, 0, &json!({"radius": 4.0})), Some("no GPU canvas"));
}

/// Closing the document while its filter previews on the GPU releases the operator.
#[test]
fn closing_the_document_releases_its_operator() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    let doc = h.state().session.active().unwrap().doc.id;
    assert!(h.state().gpu.as_ref().unwrap().has_operators(doc));
    h.state_mut().ui.dialogs.clear();
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(4);
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc), "the closed document's operator is gone");
    assert!(h.state().filter_gpu_preview.is_none());
}

/// A mirrored view draws through the CPU canvas, which can't show operators: the GPU preview
/// steps aside (and leaves no operator behind), as for any view the GPU canvas doesn't draw.
#[test]
fn a_mirrored_view_leaves_the_gpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state_mut().ui.view.flip_horizontal = true;
    h.run_steps(4);
    assert!(!on_gpu(h.state_mut(), 0));
    let doc = h.state().session.active().unwrap().doc.id;
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc), "no operator left behind");
}

/// While OK's job runs, views of another document don't end the committing preview.
#[test]
fn another_documents_view_keeps_the_committing_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // The blurred document stays active; the new one is only asked about by a view.
    h.state_mut().session.set_active(0);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = h.state().session.active().unwrap().doc.id;
    if h.state().session.job_on(doc).is_some() {
        assert!(display_doc(h.state_mut(), 1).is_none(), "nothing for the other document");
        assert!(on_gpu(h.state_mut(), 0), "the committing preview stays");
        assert!(h.state().gpu.as_ref().unwrap().has_operators(doc));
    }
}

/// While one document's OK is applying, another document becoming active (and opening the same
/// dialog) leaves the committing session alone.
#[test]
fn another_active_document_leaves_the_committing_session() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    let doc = h.state().session.active().unwrap().doc.id;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    if h.state().session.job_on(doc).is_none() {
        return; // The job already landed: nothing to keep.
    }
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let other = h.state().session.active_index().unwrap();
    crate::filter_dialog::open(h.state_mut(), COMMAND).unwrap();
    assert!(display_doc(h.state_mut(), other).is_none());
    assert!(h.state().filter_gpu_preview.as_ref().is_some_and(|p| p.committing && p.s.doc == doc), "the committing session is kept");
    assert!(h.state().gpu.as_ref().unwrap().has_operators(doc));
    // The job lands while the other document stays active: the session retires there, and the
    // other document's dialog previews on the GPU.
    let job = h.state().session.job_on(doc).map(|j| j.id);
    if let Some(job) = job {
        h.state_mut().session.wait_job(job).unwrap();
    }
    settle(&mut h, |app| on_gpu(app, other));
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc), "the applied document's operator is gone");
}

/// An opaque cover layer over the stripes, masked by 30 px bands (the blurred mask shows the
/// stripes through soft band edges), made active with its mask targeted.
fn masked_cover(h: &mut Harness<'static, PhotocraftApp>) -> photocraft_doc::LayerId {
    let size = h.state().session.active().unwrap().doc.size;
    let (w, ht) = (size.width as i32, size.height as i32);
    let cover = h
        .state_mut()
        .session
        .edit("Cover", |doc, _| {
            let mut l = photocraft_doc::Layer::raster("cover", doc.pixel_format());
            let s = l.surface_mut().unwrap();
            s.fill_rect(doc.bounds(), &[0.15, 0.2, 0.85, 1.0]);
            let mut mask = photocraft_doc::LayerMask::reveal_all();
            for y in (0..ht).step_by(60) {
                mask.surface.write_region(photocraft_geom::Rect::new(0, y, w, y + 30), &vec![0.0; w as usize * 30]);
            }
            l.mask = Some(mask);
            let id = l.id;
            doc.layers.push(l);
            Ok(id)
        })
        .unwrap();
    h.state_mut().session.active_mut().unwrap().active_layer = Some(cover);
    h.state_mut().ui.mask_target = true;
    cover
}

/// Zoomed out with the mask targeted, the proxy's mask runs the operator: its texture matches
/// the CPU's proxy preview of the mask filter.
#[test]
fn a_targeted_layer_mask_previews_on_the_gpu_proxy() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness_sized(rs, 4.0, true, [3000, 2400]);
    masked_cover(&mut h);
    h.run_steps(2);
    if !gpu_path(&h) {
        return;
    }
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "the GPU previews zoomed out");
    assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "no CPU preview");
    let op = h.state().filter_gpu_preview.as_ref().and_then(|p| p.proxy.as_ref()?.shown.as_ref()).map(|(_, op)| op.target);
    assert_eq!(op, Some(OperatorTarget::Mask));
    for radius in [4.0, 12.0] {
        set_radius(&mut h, id, radius);
        h.run_steps(4);
        let oracle = {
            let st = h.state().session.active().unwrap();
            crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": radius, "target": "mask"}), 2).unwrap()
        };
        let (_, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
        let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
        assert_eq!(tsize, size);
        let cpu = photocraft_compose::flatten(&oracle);
        let worst = cpu.px.iter().zip(&texels).enumerate().fold((0.0f32, 0usize), |w, (i, (c, g))| {
            let d = (0..3).map(|k| (c[k] * c[3] - g[k]).abs()).fold(0.0, f32::max);
            if d > w.0 { (d, i) } else { w }
        });
        let (x, y) = (worst.1 % size[0] as usize, worst.1 / size[0] as usize);
        assert!(
            worst.0 <= 1.5 / 255.0,
            "mask radius {radius}: the proxy texture is {:.2}/255 off the CPU's proxy preview at ({x},{y}): cpu {:?} gpu {:?}",
            worst.0 * 255.0,
            cpu.px[worst.1],
            texels[worst.1]
        );
        // And the unfiltered proxy is not within that: the check sees the mask filter.
        let plain = photocraft_compose::flatten(&crate::adjust_preview::proxy_base(&h.state().session.active().unwrap().doc, 2));
        let off = plain.px.iter().zip(&texels).map(|(c, g)| (0..3).map(|k| (c[k] * c[3] - g[k]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max);
        assert!(off > 10.0 / 255.0, "mask radius {radius}: the unfiltered proxy is only {:.2}/255 off", off * 255.0);
    }
}

/// Phase 3: with the Layers panel targeting a layer's mask, Gaussian Blur previews on the GPU as
/// an operator on the mask, and the canvas matches the CPU preview of the mask filter.
#[test]
fn a_targeted_layer_mask_previews_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    let cover = masked_cover(&mut h);
    h.run_steps(2);
    if !gpu_path(&h) {
        return;
    }
    assert_eq!(targets(h.state()), OperatorTarget::Mask);
    settle(&mut h, |app| on_gpu(app, 0));
    assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "the CPU preview stands aside");
    let op = h.state().filter_gpu_preview.as_ref().and_then(|p| p.shown.as_ref()).map(|(_, op)| op.target);
    assert_eq!(op, Some(OperatorTarget::Mask));
    for radius in [4.0, 12.0] {
        set_radius(&mut h, id, radius);
        h.run_steps(3);
        assert!(on_gpu(h.state_mut(), 0));
        let what = format!("mask radius {radius}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(
                crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": radius, "target": "mask"}), 1).unwrap(),
            )
        };
        let shown = probes_named(&mut h, Some(&what));
        let mut checked = 0;
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
        }
        assert!(checked >= 40, "{what}: only {checked} probes");
        // The unfiltered mask differs at the probes: the comparison means something.
        let plain = h.state().session.active().unwrap().doc.clone();
        let differs = shown.iter().filter(|(p, px)| !within(&expected_around(h.state(), &plain, *p, 0).0, *px)).count();
        assert!(differs >= 10, "{what}: only {differs} probes differ from the unfiltered mask");
    }
    // The layer's pixels are untouched: OK applies the filter to the mask.
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = h.state().session.active().unwrap().doc.id;
    if let Some(job) = h.state().session.job_on(doc).map(|j| j.id) {
        h.state_mut().session.wait_job(job).unwrap();
    }
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    let l = st.doc.layer(cover).unwrap();
    let px = l.surface().unwrap().pixel(1000, 15);
    assert!(px.iter().zip([0.15, 0.2, 0.85, 1.0]).all(|(a, b)| (a - b).abs() < 1.0 / 255.0), "the layer's pixels: {px:?}");
    let m = l.mask.as_ref().unwrap().surface.pixel(1000, 30)[0];
    assert!(m > 0.2 && m < 0.8, "the mask edge is blurred: {m}");
}

/// A targeted mask the GPU can't stand in for keeps the CPU preview: disabled, feathered (the
/// compositor combines it first), or shown on its own (⌥-click).
#[test]
fn masks_the_gpu_cant_take_keep_the_cpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    let p = json!({"radius": 4.0});
    let set_mask = |h: &mut Harness<'static, PhotocraftApp>, f: &dyn Fn(&mut photocraft_doc::LayerMask)| {
        h.state_mut()
            .session
            .edit("Mask", |doc, active| {
                let l = doc.layer_mut(active.unwrap()).unwrap();
                let mut m = photocraft_doc::LayerMask::reveal_all();
                f(&mut m);
                l.mask = Some(m);
                Ok(())
            })
            .unwrap();
    };
    h.state_mut().ui.mask_target = true;
    set_mask(&mut h, &|_| {});
    assert_eq!(ineligible(h.state(), 0, &p), None, "a plain mask");
    set_mask(&mut h, &|m| m.enabled = false);
    assert!(ineligible(h.state(), 0, &p).is_some_and(|w| w.contains("disabled")));
    set_mask(&mut h, &|m| m.feather = 3.0);
    assert!(ineligible(h.state(), 0, &p).is_some_and(|w| w.contains("feathered")));
    set_mask(&mut h, &|_| {});
    for id in [photocraft_engine::mask_view_cmds::ID, photocraft_engine::mask_view_cmds::OVERLAY_ID] {
        h.state_mut().run(id, json!({})).unwrap();
        assert!(h.state().session.active().unwrap().channel_view.layer_mask.is_some(), "{id}");
        assert_eq!(ineligible(h.state(), 0, &p), Some("the layer mask is viewed on its own"), "{id}");
        h.state_mut().run(id, json!({})).unwrap();
        assert_eq!(ineligible(h.state(), 0, &p), None, "{id} off");
    }
}

/// Phase 3: while the GPU previews, the Navigator shows the filtered picture (the operator only
/// filters the canvas), close to the CPU's full-size preview reduced to the Navigator's size.
#[test]
fn the_navigator_shows_the_gpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness(rs, 30.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    let (nav, key) = navigator_doc(h.state_mut(), 0).expect("the Navigator's picture");
    // The Navigator's texture is built from it.
    let ctx = h.ctx.clone();
    crate::canvas::navigator_texture(h.state_mut(), &ctx, 0).expect("the Navigator texture");
    let (_, display_key) = crate::canvas::canvas_display(h.state(), &nav, None);
    let id = h.state().session.active().unwrap().doc.id;
    assert_eq!(h.state().navigator_textures.get(&id).map(|c| c.1), Some(key ^ display_key));
    let side = crate::canvas::NAVIGATOR_SIDE;
    let shown = photocraft_compose::thumbnail_buffer(&nav, side);
    let st = h.state().session.active().unwrap();
    let full = crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": 30.0}), 1).unwrap();
    let want = photocraft_compose::thumbnail_buffer(&full, side);
    let plain = photocraft_compose::thumbnail_buffer(&st.doc, side);
    assert_eq!((shown.rect.width(), shown.rect.height()), (want.rect.width(), want.rect.height()));
    // Per pixel, the largest premultiplied difference; then (worst, mean).
    let diff = |a: &photocraft_compose::Buffer, b: &photocraft_compose::Buffer| {
        let d: Vec<f32> = a.px.iter().zip(&b.px).map(|(x, y)| (0..4).map(|k| (x[k] * x[3] - y[k] * y[3]).abs()).fold(0.0f32, f32::max)).collect();
        (d.iter().copied().fold(0.0, f32::max), d.iter().sum::<f32>() / d.len().max(1) as f32)
    };
    // The proxy filters a reduced copy, as the CPU's zoomed-out preview does: at radius 30 the
    // full size runs box passes and the proxy (radius 10) an exact kernel, a few levels apart.
    let ((worst, mean), (_, plain_mean)) = (diff(&shown, &want), diff(&plain, &want));
    assert!(
        mean < 6.0 / 255.0 && worst < 24.0 / 255.0,
        "the Navigator is off the full-size preview: mean {:.1}/255, worst {:.1}/255",
        mean * 255.0,
        worst * 255.0
    );
    assert!(plain_mean > 4.0 * mean, "the unfiltered thumbnail is only {:.1}/255 off on average (Navigator {:.1})", plain_mean * 255.0, mean * 255.0);
    // A slider change updates it.
    let dialog = h.state().ui.dialogs.last().unwrap().id;
    set_radius(&mut h, dialog, 4.0);
    h.run_steps(3);
    let (_, key4) = navigator_doc(h.state_mut(), 0).unwrap();
    assert_ne!(key4, key);
}

/// A session started at 100 % on a document whose reduced proxy isn't faithful (a vector mask on
/// another layer) leaves the GPU when the view zooms out, rather than preview on that proxy.
#[test]
fn zooming_out_on_an_unfaithful_proxy_leaves_the_gpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness_sized(rs, 4.0, true, [3000, 2400]);
    h.state_mut()
        .session
        .edit("Masked", |doc, _| {
            use photocraft_doc::vector::{Path, Subpath, VectorMask};
            let mut l = photocraft_doc::Layer::raster("masked", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(2100, 1600, 2900, 2300), &[1.0, 1.0, 0.0, 1.0]);
            l.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(2200.0, 1700.0), (2800.0, 1700.0), (2800.0, 2200.0)])])));
            // Below the target, so the target stays active.
            doc.layers.insert(0, l);
            Ok(())
        })
        .unwrap();
    if !gpu_path(&h) {
        return;
    }
    assert!(!photocraft_compose::proxy::proxy_faithful(&h.state().session.active().unwrap().doc));
    settle(&mut h, |app| on_gpu(app, 0));
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| !on_gpu(app, 0) && app.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    let g = h.state().filter_gpu_preview.as_ref().and_then(|p| p.proxy.as_ref());
    assert!(g.is_none_or(|g| g.shown.is_none()), "no GPU proxy preview");
}

/// A canvas past the GPU texture budget draws on the CPU canvas, which can't show operators: the
/// CPU preview runs.
#[test]
fn a_canvas_past_the_texture_budget_keeps_the_cpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (h, _) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    h.state().gpu.as_ref().unwrap().set_canvas_budget(1);
    assert_eq!(ineligible(h.state(), 0, &json!({"radius": 4.0})), Some("the GPU compositor can't draw it"));
}

/// With View › Pattern Preview on, a 100 % CPU filter preview on the GPU canvas uploads the whole
/// result: the pattern's copies sample the texture beyond the view.
#[test]
fn pattern_preview_uploads_the_whole_cpu_filter_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // A reach past the GPU's limit: the CPU previews.
    let (mut h, _) = harness(rs, 1000.0, true);
    if !gpu_path(&h) {
        return;
    }
    let done = |app: &mut PhotocraftApp| app.filter_preview.as_ref().is_some_and(|p| p.result.is_some() && !p.uploaded.is_empty());
    settle(&mut h, done);
    let p = h.state().filter_preview.as_ref().unwrap();
    let whole = p.result.as_ref().unwrap().bounds();
    assert!(!p.uploaded.contains_rect(&whole), "the view alone is uploaded at first");
    h.state_mut().ui.view.pattern_preview = true;
    h.run_steps(3);
    let p = h.state().filter_preview.as_ref().unwrap();
    assert!(p.uploaded.contains_rect(&whole), "uploaded {:?} of {:?}", p.uploaded, whole);
}

/// Select and Mask at 100 % on the GPU canvas: the worker skips the whole composite (each view
/// change is a new result) and the canvas renders the part it shows, matching the preview.
#[test]
fn select_and_mask_at_100_percent_renders_only_what_the_view_shows() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    crate::dialogs::cancel(h.state_mut(), id).unwrap();
    h.state_mut().run("select.rect", json!({"x": 300, "y": 200, "width": 1200, "height": 900, "ellipse": true})).unwrap();
    let sm = crate::filter_dialog::open(h.state_mut(), crate::select_mask_view::COMMAND).unwrap();
    {
        let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == sm).unwrap();
        d.fields.insert("__preview".into(), json!(true));
        d.fields.insert("__view".into(), json!("onBlack"));
    }
    settle(&mut h, |app| app.filter_preview.as_ref().is_some_and(|p| p.result.is_some() && !p.uploaded.is_empty()));
    let p = h.state().filter_preview.as_ref().unwrap();
    assert_eq!(p.key.command, crate::select_mask_view::COMMAND);
    assert_eq!(p.key.k, 1);
    assert!(p.flat.is_none(), "no whole composite of the view");
    let result = p.result.clone().unwrap();
    assert!(!p.uploaded.contains_rect(&result.bounds()), "only the view's part went up");
    let shown = probes_named(&mut h, Some("select and mask on black"));
    let mut checked = 0;
    for (pt, px) in &shown {
        let (want, alpha) = expected_in(h.state(), &result, *pt);
        if alpha < 0.999 {
            continue;
        }
        assert!(within(&want, *px), "at {pt:?} the canvas shows {px:?}, the preview {want:?}");
        checked += 1;
    }
    assert!(checked >= 40, "only {checked} probes");
    // On Black darkens outside the selection: the probes see the view, not the plain image.
    let plain = h.state().session.active().unwrap().doc.clone();
    let differs = shown.iter().filter(|(pt, px)| !within(&expected_around(h.state(), &plain, *pt, 0).0, *px)).count();
    assert!(differs >= 10, "only {differs} probes differ from the plain image");
}

/// Hardening: the GPU fails while previewing a targeted mask; no frame shows the mask unfiltered
/// where the blur changes it, and the CPU preview takes over.
#[test]
fn a_failing_mask_preview_falls_back_to_the_cpu_preview() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness(rs, 4.0, true);
    masked_cover(&mut h);
    h.run_steps(2);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state().gpu.as_ref().unwrap().fail_operators(true);
    set_radius(&mut h, id, 12.0);
    let mask_oracle = |app: &PhotocraftApp, r: f64| {
        let st = app.session.active().unwrap();
        std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &json!({"radius": r, "target": "mask"}), 1).unwrap())
    };
    let (raw_doc, r4_doc, r12_doc) = (oracle(h.state(), None), mask_oracle(h.state(), 4.0), mask_oracle(h.state(), 12.0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        h.step();
        for (p, px) in probes(&mut h) {
            let (raw, alpha) = expected_around(h.state(), &raw_doc, p, 0);
            let (r4, _) = expected_in(h.state(), &r4_doc, p);
            let (r12, _) = expected_in(h.state(), &r12_doc, p);
            let apart = |b: &[(u8, u8); 3]| (0..3).any(|c| b[c].0 > raw[c].1.saturating_add(12) || raw[c].0 > b[c].1.saturating_add(12));
            if alpha > 0.999 && apart(&r4) && apart(&r12) {
                assert!(!within(&raw, px), "a frame shows the mask unfiltered at {p:?}: {px:?}");
            }
        }
        if h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the CPU preview never took over");
    }
    assert!(!on_gpu(h.state_mut(), 0), "the session stays on the CPU");
}

/// Hardening: the GPU fails zoomed out. Until the CPU's proxy preview is ready the canvas holds
/// the GPU proxy's last picture (never the document unfiltered), and so does the Navigator; then
/// the CPU preview shows.
#[test]
fn a_failing_zoomed_out_preview_holds_its_last_picture() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, id) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    let (nav_before, _) = navigator_doc(h.state_mut(), 0).expect("the Navigator's picture");
    h.state().gpu.as_ref().unwrap().fail_operators(true);
    set_radius(&mut h, id, 12.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut held_frames = 0;
    loop {
        h.step();
        let ready = h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some());
        if ready {
            break;
        }
        let failed = h.state().filter_gpu_preview.as_ref().is_some_and(|p| p.failed);
        if failed {
            // Held: the canvas draws the GPU proxy's texture, the Navigator its last picture.
            assert!(held_proxy(h.state_mut(), 0).is_some(), "an unfiltered frame between the failure and the CPU preview");
            let (nav, _) = navigator_doc(h.state_mut(), 0).expect("the Navigator holds its picture");
            assert!(std::sync::Arc::ptr_eq(&nav, &nav_before));
            held_frames += 1;
        }
        assert!(std::time::Instant::now() < deadline, "the CPU preview never took over");
    }
    assert!(h.state().filter_gpu_preview.as_ref().is_some_and(|p| p.failed), "the GPU failed");
    h.run_steps(3);
    assert!(!on_gpu(h.state_mut(), 0));
    assert!(gpu_proxy(h.state_mut(), 0).is_none() && held_proxy(h.state_mut(), 0).is_none(), "the CPU preview shows now");
    assert!(navigator_doc(h.state_mut(), 0).is_none(), "the Navigator shows the CPU preview");
    assert!(held_frames >= 1, "no frame came between the failure and the CPU preview: nothing was checked");
}

/// Opens Unsharp Mask's dialog (closing the harness's Gaussian Blur) with `params`, Preview on.
fn open_unsharp(h: &mut Harness<'static, PhotocraftApp>, blur_dialog: u64, params: serde_json::Value) -> u64 {
    crate::dialogs::cancel(h.state_mut(), blur_dialog).unwrap();
    let id = crate::filter_dialog::open(h.state_mut(), UNSHARP).unwrap();
    let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    for (k, v) in params.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    d.fields.insert("__preview".into(), json!(true));
    id
}

/// Phase 4: Unsharp Mask previews on the GPU at 100 %, matching the CPU preview, and it isn't
/// the unsharpened image.
#[test]
fn unsharp_mask_previews_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, blur) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    let id = open_unsharp(&mut h, blur, json!({"amount": 200.0, "radius": 3.0, "threshold": 0.0}));
    settle(&mut h, |app| on_gpu(app, 0));
    assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "the CPU preview stands aside");
    for params in [json!({"amount": 200.0, "radius": 3.0, "threshold": 0.0}), json!({"amount": 120.0, "radius": 8.0, "threshold": 4.0})] {
        let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
        for (k, v) in params.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
        h.run_steps(3);
        assert!(on_gpu(h.state_mut(), 0));
        let what = format!("unsharp {params}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, UNSHARP, &params, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the unsharpened image");
    }
}

/// Zoomed out, Unsharp Mask's proxy texture matches the CPU's proxy preview exactly (as for the
/// blur), its radius scaled by the factor and its amount and threshold as they are.
#[test]
fn unsharp_mask_previews_on_the_gpu_proxy() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, blur) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    let params = json!({"amount": 250.0, "radius": 6.0, "threshold": 0.0});
    open_unsharp(&mut h, blur, params.clone());
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "the GPU previews zoomed out");
    let (k, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
    assert_eq!(k, 2);
    let oracle = {
        let st = h.state().session.active().unwrap();
        crate::filter_dialog::preview_document(&st.doc, st.active_layer, UNSHARP, &params, 2).unwrap()
    };
    let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
    assert_eq!(tsize, size);
    let premul = |c: &[f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3]];
    let off = |cpu: &photocraft_compose::Buffer| {
        cpu.px.iter().zip(&texels).map(|(c, g)| premul(c).iter().zip(g).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max)
    };
    let worst = off(&photocraft_compose::flatten(&oracle));
    assert!(worst <= 1.5 / 255.0, "the proxy texture is {:.2}/255 off the CPU's proxy preview", worst * 255.0);
    let plain = off(&photocraft_compose::flatten(&crate::adjust_preview::proxy_base(&h.state().session.active().unwrap().doc, 2)));
    assert!(plain > 10.0 / 255.0, "the unsharpened proxy is only {:.2}/255 off", plain * 255.0);
}

/// Phase 4: High Pass and Box Blur preview on the GPU at 100 %, matching the CPU preview.
#[test]
fn high_pass_and_box_blur_preview_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, mut open) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    for (command, params) in [(HIGH_PASS, json!({"radius": 6.0})), (BOX_BLUR, json!({"radius": 3.0})), (BOX_BLUR, json!({"radius": 12.0}))] {
        crate::dialogs::cancel(h.state_mut(), open).unwrap();
        open = crate::filter_dialog::open(h.state_mut(), command).unwrap();
        {
            let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == open).unwrap();
            for (k, v) in params.as_object().unwrap() {
                d.fields.insert(k.clone(), v.clone());
            }
            d.fields.insert("__preview".into(), json!(true));
        }
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{command}: the CPU preview stands aside");
        let what = format!("{command} {params}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, command, &params, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the unfiltered image");
    }
}

/// Phase 4: Minimum (Preserve Squareness), Maximum (Preserve Roundness) and Custom preview on
/// the GPU at 100 %, matching the CPU preview; a disc past the GPU's limit keeps the CPU's.
#[test]
fn minimum_maximum_and_custom_preview_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, mut open) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    let blur_more = json!([0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 2, 2, 2, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0, 0]);
    let cases = [
        (MINIMUM, json!({"radius": 5.0, "preserve": "squareness"}), true),
        (MAXIMUM, json!({"radius": 9.0, "preserve": "roundness"}), true),
        (CUSTOM, json!({"kernel": blur_more, "scale": 14, "offset": 0}), true),
        (MAXIMUM, json!({"radius": 40.0, "preserve": "roundness"}), true),
        (MAXIMUM, json!({"radius": 150.0, "preserve": "roundness"}), false),
    ];
    for (command, params, gpu) in cases {
        crate::dialogs::cancel(h.state_mut(), open).unwrap();
        open = crate::filter_dialog::open(h.state_mut(), command).unwrap();
        {
            let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == open).unwrap();
            for (k, v) in params.as_object().unwrap() {
                d.fields.insert(k.clone(), v.clone());
            }
            d.fields.insert("__preview".into(), json!(true));
        }
        let what = format!("{command} {params}");
        if !gpu {
            settle(&mut h, |app| app.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
            assert!(!on_gpu(h.state_mut(), 0), "{what}: a disc past the GPU's limit previews on the CPU");
            continue;
        }
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{what}: the CPU preview stands aside");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, command, &params, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the unfiltered image");
    }
}

/// Adds a 300×200 map document (red and green ramps with steps) after the harness's document,
/// keeping the harness's document active; returns its index.
fn add_map_document(h: &mut Harness<'static, PhotocraftApp>) -> usize {
    let mut map =
        photocraft_doc::Document::new("map", photocraft_geom::Size::new(300, 200), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut l = photocraft_doc::Layer::raster("map", map.pixel_format());
    let s = l.surface_mut().unwrap();
    let data: Vec<f32> = (0..300 * 200)
        .flat_map(|i| {
            let (x, y) = ((i % 300) as f32, (i / 300) as f32);
            [(x / 299.0 * 0.9 + 0.05 + if (i % 300) % 40 < 6 { 0.3 } else { 0.0 }).min(1.0), 1.0 - y / 199.0 * 0.8 - 0.1, 0.5, 1.0]
        })
        .collect();
    s.write_region(photocraft_geom::Rect::new(0, 0, 300, 200), &data);
    map.layers.push(l);
    let at = h.state_mut().session.add_document(map, None);
    h.state_mut().session.set_active(0);
    h.run_steps(2);
    at
}

/// Phase 4, multi-input: Displace with an open document as its map previews on the GPU at 100 %
/// and zoomed out, matching the CPU preview; with no map (or a map file) it keeps the CPU's.
#[test]
fn displace_with_an_open_map_document_previews_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, blur) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    let map = add_map_document(&mut h);
    crate::dialogs::cancel(h.state_mut(), blur).unwrap();
    let id = crate::filter_dialog::open(h.state_mut(), DISPLACE).unwrap();
    let set = |h: &mut Harness<'static, PhotocraftApp>, params: &Value| {
        let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
        for (k, v) in params.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
        d.fields.insert("__preview".into(), json!(true));
    };
    for params in [
        json!({"horizontal": 10.0, "vertical": 10.0, "fit": "stretch", "undefinedAreas": "wrap", "mapDocument": map}),
        json!({"horizontal": -30.0, "vertical": 20.0, "fit": "tile", "undefinedAreas": "repeat", "mapDocument": map}),
    ] {
        set(&mut h, &params);
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{params}: the CPU preview stands aside");
        let what = format!("displace {params}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            let docs: Vec<_> = h.state().session.documents().iter().map(|d| d.doc.clone()).collect();
            std::sync::Arc::new(
                crate::filter_dialog::preview_document_for_target(
                    &st.doc,
                    st.active_layer,
                    DISPLACE,
                    &params,
                    1,
                    None,
                    photocraft_engine::channel_cmds::ChannelTarget::Composite,
                    &docs,
                    &Default::default(),
                )
                .unwrap(),
            )
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the undisplaced image");
    }
    // The map is made once for the session, not per settings change.
    assert_eq!(h.state().filter_gpu_preview.as_ref().map(|p| p.maps.len()), Some(1));
    // The Navigator shows the displacement too: its run gets the open documents, so the map
    // document resolves (it showed the unfiltered document).
    let (nav, _) = navigator_doc(h.state_mut(), 0).expect("the Navigator's picture");
    let k = nav.size.width.max(1);
    let k = h.state().session.active().unwrap().doc.size.width.div_ceil(k);
    let plain = crate::proxy::proxy_document(&h.state().session.active().unwrap().doc, k);
    let last = json!({"horizontal": -30.0, "vertical": 20.0, "fit": "tile", "undefinedAreas": "repeat", "mapDocument": map});
    let want = {
        let st = h.state().session.active().unwrap();
        let docs: Vec<_> = h.state().session.documents().iter().map(|d| d.doc.clone()).collect();
        crate::filter_dialog::preview_document_for_target(
            &st.doc,
            st.active_layer,
            DISPLACE,
            &last,
            k,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &docs,
            &Default::default(),
        )
        .unwrap()
    };
    let (shown, plain, want) = (photocraft_compose::flatten(&nav), photocraft_compose::flatten(&plain), photocraft_compose::flatten(&want));
    assert_eq!(shown.px, want.px, "the Navigator: the CPU preview at its factor");
    assert_ne!(shown.px, plain.px, "displaced, not the plain document");
    // Zoomed out: the proxy, with its own reduced map.
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "zoomed out on the GPU");
    let maps = h.state().filter_gpu_preview.as_ref().unwrap().maps.iter().map(|(k, _)| k.3).collect::<Vec<_>>();
    assert!(maps.contains(&2), "the proxy's map: {maps:?}");
    // The proxy texture against the CPU's proxy preview (its map reduced the same way).
    let last = json!({"horizontal": -30.0, "vertical": 20.0, "fit": "tile", "undefinedAreas": "repeat", "mapDocument": map});
    let (k, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
    assert_eq!(k, 2);
    let oracle = {
        let st = h.state().session.active().unwrap();
        let docs: Vec<_> = h.state().session.documents().iter().map(|d| d.doc.clone()).collect();
        crate::filter_dialog::preview_document_for_target(
            &st.doc,
            st.active_layer,
            DISPLACE,
            &last,
            2,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &docs,
            &Default::default(),
        )
        .unwrap()
    };
    let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
    assert_eq!(tsize, size);
    let cpu = photocraft_compose::flatten(&oracle);
    let worst = cpu.px.iter().zip(&texels).map(|(c, g)| (0..3).map(|i| (c[i] * c[3] - g[i]).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max);
    assert!(worst <= 1.5 / 255.0, "the displaced proxy is {:.2}/255 off the CPU's proxy preview", worst * 255.0);
    // No map document: the CPU's (which has none to show either).
    h.state_mut().ui.views[0].zoom = 1.0;
    set(&mut h, &json!({"mapDocument": -1}));
    h.run_steps(4);
    assert!(!on_gpu(h.state_mut(), 0), "no map, no GPU preview");
    // Remembered, so the session isn't measured and built again every frame…
    let refused = h.state().filter_gpu_refused;
    assert!(refused.is_some(), "the refused settings are kept");
    h.run_steps(2);
    assert_eq!(h.state().filter_gpu_refused, refused);
    // …and new settings try again.
    set(&mut h, &json!({"mapDocument": map}));
    settle(&mut h, |app| on_gpu(app, 0));
    assert!(h.state().filter_gpu_refused.is_none());
}

/// Only the main canvas gives the dialog's preview window its GPU view: an Arrange tile or a
/// floating window (perhaps on another display) must not replace it.
#[test]
fn only_the_main_canvas_gives_the_preview_window_its_view() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.set_wgpu(rs);
    app.run("file.new", json!({"width": 640, "height": 480})).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let ctx = egui::Context::default();
    let view = app.ui.views[0].clone();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    for primary in [false, true] {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            crate::canvas::canvas_view(&mut app, ui, 0, rect, view.clone(), primary);
        });
        output.textures_delta.clear();
        assert!(app.perf.gpu, "drawn on the GPU canvas");
        assert_eq!(app.canvas_gpu.contains_key(&doc), primary, "primary {primary}");
    }
}

/// Phase 4: the distort filters preview on the GPU at 100 %, matching the CPU preview (a few of
/// them through the app: the mappings themselves are covered in `photocraft-gpu`'s parity tests).
#[test]
fn distort_filters_preview_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, mut open) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    let cases = [
        ("filter.distort.twirl", json!({"angle": 120.0})),
        (
            "filter.distort.wave",
            json!({"generators": 3, "wavelengthMin": 30, "wavelengthMax": 200, "amplitudeMin": 10, "amplitudeMax": 40, "type": "sine", "undefinedAreas": "wrap", "seed": 3}),
        ),
        ("filter.distort.polarCoordinates", json!({"mode": "rectangularToPolar"})),
        ("filter.distort.shear", json!({"points": [[0.0, 0.0], [0.5, 0.2], [1.0, -0.1]], "undefinedAreas": "repeat"})),
    ];
    for (command, params) in cases {
        crate::dialogs::cancel(h.state_mut(), open).unwrap();
        open = crate::filter_dialog::open(h.state_mut(), command).unwrap();
        {
            let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == open).unwrap();
            for (k, v) in params.as_object().unwrap() {
                d.fields.insert(k.clone(), v.clone());
            }
            d.fields.insert("__preview".into(), json!(true));
        }
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{command}: the CPU preview stands aside");
        let what = format!("{command} {params}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, command, &params, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the undistorted image");
    }
}

/// Phase 4: Lens Blur previews on the GPU at 100 %, matching the CPU preview, with each depth
/// source: none (with Specular Highlights), the layer's transparency, its layer mask and an
/// alpha channel (the second inputs, read as the engine reads them), and noise.
#[test]
fn lens_blur_previews_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // Big enough for a reduced proxy when zoomed out.
    let (mut h, mut open) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    h.state_mut()
        .session
        .edit("Depth", |doc, _| {
            let (w, ht) = (doc.bounds().width(), doc.bounds().height());
            let across: Vec<f32> = (0..w * ht).map(|i| (i % w) as f32 / w as f32).collect();
            let down: Vec<f32> = (0..w * ht).map(|i| (i / w) as f32 / ht as f32).collect();
            let mut mask = photocraft_doc::LayerMask::reveal_all();
            mask.surface.write_region(doc.bounds(), &across);
            // Disabled, so it doesn't hide the layer: Lens Blur reads its depth all the same.
            mask.enabled = false;
            doc.layers[0].mask = Some(mask);
            let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
            s.write_region(doc.bounds(), &down);
            doc.channels.push(photocraft_doc::AlphaChannel::new("Depth", s));
            Ok(())
        })
        .unwrap();
    let cases = [
        json!({"radius": 25, "shape": "hexagon", "brightness": 60, "threshold": 120, "depthMap": "none"}),
        json!({"radius": 20, "shape": "pentagon", "bladeCurvature": 40, "depthMap": "transparency", "focalDistance": 0}),
        json!({"radius": 30, "shape": "octagon", "rotation": 30, "depthMap": "layerMask", "focalDistance": 0}),
        json!({"radius": 15, "depthMap": "channel", "depthChannel": 0, "focalDistance": 60, "invert": true, "noise": 30, "distribution": "gaussian", "seed": 4}),
    ];
    for params in cases.clone() {
        crate::dialogs::cancel(h.state_mut(), open).unwrap();
        open = crate::filter_dialog::open(h.state_mut(), LENS_BLUR).unwrap();
        {
            let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == open).unwrap();
            for (k, v) in params.as_object().unwrap() {
                d.fields.insert(k.clone(), v.clone());
            }
            d.fields.insert("__preview".into(), json!(true));
        }
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{params}: the CPU preview stands aside");
        let what = format!("Lens Blur {params}");
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, LENS_BLUR, &params, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the unblurred image");
    }
    // Zoomed out: the proxy, its depth channel reduced the same way, against the CPU's proxy
    // preview (the last case's settings).
    let last = cases.last().unwrap().clone();
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "zoomed out on the GPU");
    let depths = h.state().filter_gpu_preview.as_ref().unwrap().depths.iter().map(|(k, _)| k.0).collect::<Vec<_>>();
    assert!(depths.contains(&2), "the proxy's depth map: {depths:?}");
    let (k, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
    assert_eq!(k, 2);
    let oracle = {
        let st = h.state().session.active().unwrap();
        crate::filter_dialog::preview_document(&st.doc, st.active_layer, LENS_BLUR, &last, 2).unwrap()
    };
    let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
    assert_eq!(tsize, size);
    let cpu = photocraft_compose::flatten(&oracle);
    let worst = cpu.px.iter().zip(&texels).map(|(c, g)| (0..3).map(|i| (c[i] * c[3] - g[i]).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max);
    assert!(worst <= 1.5 / 255.0, "the lens-blurred proxy is {:.2}/255 off the CPU's proxy preview", worst * 255.0);
}

/// Phase 4: Apply Image as a layer operator (built as the preview builds it, `blur_for`) against
/// the engine's command, both composited (GPU against CPU): blend modes, Add and Subtract with
/// scale and offset, opacity, a mask channel, a feathered selection, Preserve Transparency, an
/// inverted and a single-channel source, a source layer; 8/16/32-bit and Grayscale.
#[test]
fn apply_image_operator_matches_the_command() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Document, Layer};
    use photocraft_geom::{Rect, Size};
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (device, queue) = (&rs.device, &rs.queue);
    let Ok(mut comp) = photocraft_gpu::Compositor::try_new_with_format(device, eframe::wgpu::TextureFormat::Rgba32Float) else {
        assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no compositor");
        return;
    };
    if !comp.operators_supported(device) {
        assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no operators");
        return;
    }
    let rnd = |seed: u32, i: u32| {
        let mut h = seed.wrapping_mul(0x9e37_79b9) ^ i.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        h = h.wrapping_mul(0xc2b2_ae35);
        h ^= h >> 16;
        (h & 0xffff) as f32 / 65535.0
    };
    // A noise layer over `rect`, its alpha at least `min_alpha`.
    let noise = |name: &str, fmt: PixelFormat, rect: Rect, seed: u32, min_alpha: f32| {
        let mut l = Layer::raster(name, fmt);
        let ch = fmt.channels();
        let n = rect.width() as usize * rect.height() as usize;
        let data: Vec<f32> = (0..n * ch)
            .map(|i| {
                let v = rnd(seed, i as u32);
                if fmt.alpha && i % ch == ch - 1 { min_alpha + (1.0 - min_alpha) * v } else { v }
            })
            .collect();
        l.surface_mut().unwrap().write_region(rect, &data);
        l
    };
    let doc_of = |mode: ColorMode, depth: SampleType| {
        let fmt = PixelFormat::new(mode, depth, true);
        let mut d = Document::new("apply", Size::new(240, 160), mode, depth);
        d.layers.push(noise("bg", fmt, Rect::new(0, 0, 240, 160), 1, 1.0));
        let target = noise("target", fmt, Rect::new(30, 20, 210, 140), 2, 0.0);
        let id = target.id;
        d.layers.push(target);
        d.layers.push(noise("other", fmt, Rect::new(0, 0, 240, 160), 3, 0.5));
        let mut m = photocraft_raster::Surface::new(PixelFormat::GRAY8);
        m.write_region(d.bounds(), &(0..240 * 160).map(|i| (i % 240) as f32 / 239.0).collect::<Vec<_>>());
        d.channels.push(AlphaChannel::new("Ramp", m));
        (d, id)
    };
    let feathered = || {
        let mut s = photocraft_raster::Surface::new(PixelFormat::GRAY8);
        let data: Vec<f32> = (0..240 * 160)
            .map(|i| {
                let (x, y) = ((i % 240) as f32, (i / 240) as f32);
                (1.5 - (((x - 150.0) / 60.0).powi(2) + ((y - 80.0) / 50.0).powi(2)).sqrt()).clamp(0.0, 1.0)
            })
            .collect();
        s.write_region(Rect::new(0, 0, 240, 160), &data);
        s
    };
    let cases = [
        json!({"blending": "multiply"}),
        json!({"blending": "screen", "opacity": 60}),
        json!({"blending": "overlay", "sourceInvert": true}),
        json!({"blending": "softLight", "maskChannel": "Ramp"}),
        json!({"blending": "colorDodge", "sourceLayer": "other"}),
        json!({"blending": "colorBurn", "preserveTransparency": true}),
        json!({"blending": "difference", "sourceChannel": "Green", "opacity": 75}),
        json!({"blending": "add", "scale": 1.5, "offset": 20}),
        json!({"blending": "subtract", "scale": 1.2, "offset": -30, "maskChannel": "Ramp", "maskInvert": true}),
        json!({"blending": "linearBurn", "sourceChannel": "Ramp"}),
        json!({"blending": "hardLight", "sourceLayer": "other", "sourceChannel": "transparency", "preserveTransparency": true, "opacity": 50}),
    ];
    let mut failures = Vec::new();
    for (mode, depth) in
        [(ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Rgb, SampleType::F32), (ColorMode::Grayscale, SampleType::U8)]
    {
        for params in &cases {
            // Grayscale has no Green.
            if mode == ColorMode::Grayscale && params.get("sourceChannel") == Some(&json!("Green")) {
                continue;
            }
            for selection in [false, true] {
                let (mut doc, id) = doc_of(mode, depth);
                if selection {
                    doc.selection = Some(feathered());
                }
                let what = format!("{mode:?} {depth:?} {params}{}", if selection { ", selection" } else { "" });
                let doc = std::sync::Arc::new(doc);
                let mut s = photocraft_engine::Session::new();
                s.add_document((*doc).clone(), None);
                s.select_layer(id).unwrap();
                if let Err(e) = s.execute(APPLY_IMAGE, params.clone()) {
                    failures.push(format!("{what}: the command failed: {e}"));
                    continue;
                }
                let rect = doc.bounds();
                let cpu = photocraft_compose::render(&s.active().unwrap().doc, rect);
                let measured = Measured::of(&doc, id, APPLY_IMAGE, OperatorTarget::Content).unwrap();
                let apply = apply_inputs(&doc, id, params).and_then(|i| apply_params(params, i)).map(std::sync::Arc::new);
                let Some((blur, _)) = blur_for(&doc, id, params, None, measured, Inputs { apply, ..Default::default() }) else {
                    failures.push(format!("{what}: no operator"));
                    continue;
                };
                let op = LayerOperator { layer: id, target: OperatorTarget::Content, program: std::sync::Arc::new(blur) };
                comp.set_operators(doc.id, vec![op]);
                let out = match photocraft_gpu::render_to_vec_stats(&mut comp, device, queue, &doc, rect) {
                    Ok((out, _)) => out,
                    Err(e) => {
                        failures.push(format!("{what}: {e}"));
                        continue;
                    }
                };
                comp.clear_operators(doc.id);
                let tol = if depth == SampleType::U8 { 1.5 / 255.0 } else { 1.0 / 255.0 };
                let worst = cpu
                    .px
                    .iter()
                    .zip(&out)
                    .enumerate()
                    .map(|(i, (c, g))| ((0..4).map(|k| (c[k] * c[3] - g[k] * g[3]).abs()).fold(0.0f32, f32::max), i))
                    .fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a });
                if worst.0 > tol {
                    let w = rect.width() as usize;
                    failures.push(format!(
                        "{what}: {:.2}/255 off at ({}, {}): cpu {:?} gpu {:?}",
                        worst.0 * 255.0,
                        worst.1 % w,
                        worst.1 / w,
                        cpu.px[worst.1],
                        out[worst.1]
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Phase 4: Image › Apply Image previews on the GPU, at 100 % and zoomed out, matching the CPU
/// preview; its source and weight are made once per source and mask, not per settings change.
#[test]
fn apply_image_previews_on_the_gpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, blur) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    crate::dialogs::cancel(h.state_mut(), blur).unwrap();
    let ctx = h.ctx.clone();
    let opened = crate::view_cmds::invoke(h.state_mut(), &ctx, APPLY_IMAGE, &json!({})).expect("ours").unwrap();
    let id = opened["dialog"].as_u64().unwrap();
    let set = |h: &mut Harness<'static, PhotocraftApp>, params: &Value| {
        let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
        for (k, v) in params.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
    };
    // One input build at a time: a source change while a build runs waits for it (each holds the
    // canvas's planes), rather than starting a second one beside it.
    on_gpu(h.state_mut(), 0);
    let pending = |h: &Harness<'static, PhotocraftApp>| h.state().filter_gpu_preview.as_ref().and_then(|p| p.apply_pending.as_ref().map(|(k, _)| k.clone()));
    let first = pending(&h).expect("the first build runs off the UI thread");
    set(&mut h, &json!({"sourceInvert": true}));
    on_gpu(h.state_mut(), 0);
    assert_eq!(pending(&h), Some(first), "the second waits for the first");
    set(&mut h, &json!({"sourceInvert": false}));
    let cases = [
        json!({"blending": "difference", "opacity": 70.0}),
        json!({"blending": "screen", "sourceInvert": true}),
        json!({"blending": "add", "scale": 1.4, "offset": 10}),
    ];
    let mut sources = Vec::new();
    for params in &cases {
        set(&mut h, params);
        settle(&mut h, |app| on_gpu(app, 0));
        assert!(h.state().filter_preview.as_ref().is_none_or(|p| p.result.is_none()), "{params}: the CPU preview stands aside");
        let what = format!("Apply Image {params}");
        let fields = crate::filter_dialog::params_of(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
        let oracle = {
            let st = h.state().session.active().unwrap();
            std::sync::Arc::new(crate::filter_dialog::preview_document(&st.doc, st.active_layer, APPLY_IMAGE, &fields, 1).unwrap())
        };
        let plain = h.state().session.active().unwrap().doc.clone();
        let shown = probes_named(&mut h, Some(&what));
        let (mut checked, mut differs) = (0, 0);
        for (p, px) in &shown {
            let (want, alpha) = expected_in(h.state(), &oracle, *p);
            if alpha < 0.999 {
                continue;
            }
            assert!(within(&want, *px), "{what}: at {p:?} the canvas shows {px:?}, the CPU {want:?}");
            checked += 1;
            differs += usize::from(!within(&expected_around(h.state(), &plain, *p, 0).0, *px));
        }
        assert!(checked >= 40 && differs >= 10, "{what}: {checked} probes checked, {differs} differ from the image");
        let p = h.state().filter_gpu_preview.as_ref().unwrap();
        sources.push(p.applies.iter().map(|(_, (source, _))| std::sync::Arc::as_ptr(source) as usize).collect::<Vec<_>>());
    }
    // The source is made once per source: the inverted one replaced the first, and Add (still
    // inverted: the fields stay) kept it.
    assert!(sources.iter().all(|s| s.len() == 1), "one source at a time: {sources:?}");
    assert!(sources[0] != sources[1] && sources[1] == sources[2], "{sources:?}");
    // Zoomed out: the proxy, against the CPU's proxy preview.
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_gpu_preview.as_ref().is_some_and(|p| p.proxy.as_ref().is_some_and(|g| g.drawn.is_some())));
    assert!(on_gpu(h.state_mut(), 0), "zoomed out on the GPU");
    let (k, key, size) = gpu_proxy(h.state_mut(), 0).expect("the proxy");
    assert_eq!(k, 2);
    let fields = crate::filter_dialog::params_of(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    let oracle = {
        let st = h.state().session.active().unwrap();
        crate::filter_dialog::preview_document(&st.doc, st.active_layer, APPLY_IMAGE, &fields, 2).unwrap()
    };
    let (_, tsize, texels) = h.state().gpu.as_ref().unwrap().read_texels(key).expect("the proxy texture");
    assert_eq!(tsize, size);
    let cpu = photocraft_compose::flatten(&oracle);
    let worst = cpu.px.iter().zip(&texels).map(|(c, g)| (0..3).map(|i| (c[i] * c[3] - g[i]).abs()).fold(0.0f32, f32::max)).fold(0.0, f32::max);
    assert!(worst <= 1.5 / 255.0, "the applied proxy is {:.2}/255 off the CPU's proxy preview", worst * 255.0);
}

/// The canvas budget shrinking mid-session (the canvas then draws on the CPU, which can't show the
/// operator): the session fails over to the CPU preview instead of hiding it, with its operator
/// cleared (nothing stale for the canvas to draw once it fits again), and is back on the GPU when
/// the budget grows.
#[test]
fn a_canvas_that_stops_fitting_mid_session_falls_back_to_the_cpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let (mut h, _) = harness(rs, 4.0, true);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    let doc = h.state().session.active().unwrap().doc.id;
    h.state().gpu.as_ref().unwrap().set_canvas_budget(1);
    assert!(!on_gpu(h.state_mut(), 0), "the CPU preview takes over");
    assert!(h.state().filter_gpu_preview.as_ref().is_some_and(|p| p.failed));
    assert!(!h.state().gpu.as_ref().unwrap().has_operators(doc), "its operator is cleared");
    h.state().gpu.as_ref().unwrap().set_canvas_budget(u64::MAX);
    assert!(on_gpu(h.state_mut(), 0), "back on the GPU once the canvas fits");
    assert!(h.state().gpu.as_ref().unwrap().has_operators(doc));
}
