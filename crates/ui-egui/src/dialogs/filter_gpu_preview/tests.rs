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
    let capability = matches!(why, "no GPU canvas" | "the GPU compositor can't draw it" | "a software (CPU) adapter");
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
    // Floating windows (the dialog) cover part of the canvas.
    let windows: Vec<egui::Rect> = h.ctx.memory(|m| m.layer_ids().filter(|l| l.order != egui::Order::Background).filter_map(|l| m.area_rect(l.id)).collect());
    let mut out = Vec::new();
    for i in 0..30 {
        for dx in [-3, -1, 0, 2] {
            for y in (100..1400).step_by(170) {
                let (x, y) = (40 + i * 64 + dx, y);
                let p = xf.to_screen(x as f32 + 0.5, y as f32 + 0.5);
                if canvas.shrink(4.0).contains(p) && !windows.iter().any(|w| w.expand(4.0).contains(p)) {
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
    let buf = photocraft_compose::render(doc, photocraft_geom::Rect::new(p.0 - 1, p.1 - 1, p.0 + 2, p.1 + 2));
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

/// Zoomed out the preview is the CPU's reduced proxy (a later phase moves it to the GPU).
#[test]
fn zoomed_out_previews_stay_on_the_cpu() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // Big enough for a reduced proxy when zoomed out.
    let (mut h, _) = harness_sized(rs, 4.0, true, [3000, 2400]);
    if !gpu_path(&h) {
        return;
    }
    settle(&mut h, |app| on_gpu(app, 0));
    h.state_mut().ui.views[0].zoom = 0.25;
    settle(&mut h, |app| app.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    assert!(!on_gpu(h.state_mut(), 0));
    assert_eq!(h.state().filter_preview.as_ref().unwrap().key.k, 2, "the CPU proxy");
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
