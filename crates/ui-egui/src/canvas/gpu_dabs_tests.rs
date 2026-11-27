//! A Brush stroke whose dabs the GPU draws (`gpu_dabs`, #17133 phase 2) through the canvas on a GPU
//! (lavapipe in CI): the press, the moves and the release run the app's own frames. Skips without a
//! GPU adapter, unless `PHOTOCRAFT_REQUIRE_GPU_TESTS` is set.

use serde_json::json;

use super::{PhotocraftApp, Tool, ToolEvent, testing, tool_event};
use photocraft_geom::Rect;

const M: egui::Modifiers = egui::Modifiers::NONE;
const W: i32 = 320;
const H: i32 = 160;

fn render_state() -> Option<eframe::egui_wgpu::RenderState> {
    match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
        Ok(rs) => Some(rs),
        Err(error) => {
            if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping the GPU dabs canvas test: no GPU adapter");
            None
        }
    }
}

/// An app on the GPU canvas with a white background under an empty layer, the Brush set to paint a
/// soft red stroke with `smoothing`.
fn app_on_gpu(rs: eframe::egui_wgpu::RenderState, smoothing: f32) -> Box<PhotocraftApp> {
    app_with(Some(rs), smoothing, json!({}))
}

/// [`app_on_gpu`] on the GPU canvas or, without `rs`, the CPU one, with `extra` settings merged
/// into the brush.
fn app_with(rs: Option<eframe::egui_wgpu::RenderState>, smoothing: f32, extra: serde_json::Value) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    if let Some(rs) = rs {
        app.set_wgpu(rs);
    }
    let mut brush = json!({"size": 22, "hardness": 0.3, "flow": 0.6, "opacity": 0.8, "pressureSize": false, "smoothing": {"amount": smoothing}});
    if let (Some(b), Some(e)) = (brush.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width": W, "height": H, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#d02060"})).unwrap();
    app.run("tools.setBrush", json!({ "brush": brush })).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    app
}

/// A wiggle across the document.
fn path() -> Vec<[f64; 2]> {
    (0..=30).map(|i| [30.0 + 8.0 * f64::from(i), 80.0 + 30.0 * (f64::from(i) / 4.0).sin()]).collect()
}

/// Press at the path's start and move along it.
fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]]) {
    tool_event(app, ToolEvent::Down { x: pts[0][0], y: pts[0][1], pressure: 1.0 }, M);
    for p in &pts[1..] {
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, M);
    }
}

fn release(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, M);
}

fn layer_pixels(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(0, 0, W, H))
}

/// The stroke's mask in the compositor: the layer it is on, while the live stroke has one.
fn mask_layer(app: &PhotocraftApp) -> photocraft_doc::LayerId {
    app.live_stroke.as_ref().and_then(|l| l.gpu.as_ref()).expect("a stroke the GPU draws").layer()
}

fn has_mask(app: &PhotocraftApp, layer: photocraft_doc::LayerId) -> bool {
    app.gpu.as_ref().expect("a GPU canvas").has_external_mask(layer)
}

/// The undone stroke's command run again from its params, on the CPU: the layer's pixels.
fn replayed(app: &mut PhotocraftApp) -> Vec<f32> {
    let params = app.session.journal.iter().rev().find(|(name, _)| name == "paint.stroke").expect("the stroke was committed").1.clone();
    app.run("edit.undo", json!({})).unwrap();
    app.run("paint.stroke", params).unwrap();
    layer_pixels(app)
}

/// The commit of a GPU stroke is the pixels `paint.stroke` writes, within the mask's precision.
fn assert_commit_matches_paint_stroke(app: &mut PhotocraftApp, what: &str) {
    let gpu = layer_pixels(app);
    assert!(gpu.iter().any(|v| *v != 0.0), "{what}: the stroke is in the layer");
    let cpu = replayed(app);
    // R32F is within a few 1e-6 of the CPU's coverage; R16F rounds each blend (and lavapipe worse
    // than a GPU), see `gpu_commit_tests`.
    let full = photocraft_gpu::Compositor::full_precision_masks(&app.gpu.as_ref().unwrap().device());
    let limit = (if full { 1.0 / 255.0 } else { 3.0 / 255.0 }) + 4.0 * f32::EPSILON;
    let worst = gpu.iter().zip(&cpu).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst <= limit, "{what}: the committed stroke differs from paint.stroke by {:.2}/255 (full precision {full})", worst * 255.0);
}

#[test]
fn a_brush_stroke_draws_on_the_gpu_and_commits_paint_strokes_pixels() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    let pts = path();
    drag(&mut app, &pts[..21]);

    // The stroke is the GPU's: its dabs are stamped into a mask the compositor holds, and the
    // canvas shows it before the release.
    let layer = mask_layer(&app);
    assert!(has_mask(&app, layer), "the compositor holds the stroke's mask");
    let id = app.session.active().unwrap().doc.id.0;
    let (_, size, texels) = app.gpu.as_ref().unwrap().read_texels(id).expect("the canvas texture");
    let at = pts[20];
    let shown = texels[at[1] as usize * size[0] as usize + at[0] as usize];
    assert!(shown[1] < 0.9, "the GPU canvas shows the stroke while it is drawn: {shown:?}");
    assert_eq!(layer_pixels(&app).iter().filter(|v| **v != 0.0).count(), 0, "the layer's pixels change at the release only");

    for p in &pts[21..] {
        tool_event(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, M);
    }
    release(&mut app, pts[30]);
    assert!(app.live_stroke.is_none() && app.pending_stroke.is_none(), "the commit landed");
    assert!(app.committed_stroke.is_some(), "the commit took the live stroke's pixels");
    assert_eq!(app.perf.gpu_dab_strokes, 1, "the stroke took the GPU path");
    assert!(!has_mask(&app, layer), "the mask is gone once the commit landed");
    assert!(!app.ui.status_error, "{}", app.ui.status);

    assert_commit_matches_paint_stroke(&mut app, "unsmoothed");
}

#[test]
fn a_stroke_the_gpu_cannot_draw_takes_the_cpu_overlay_path_and_commits_as_before() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // Wet Edges makes a pixel's value depend on more than its dab: the stroke is the CPU's.
    let mut app = app_with(Some(rs), 0.4, json!({"wetEdges": true}));
    let pts = path();
    drag(&mut app, &pts);
    let live = app.live_stroke.as_ref().expect("a live stroke");
    assert!(live.gpu.is_none(), "the CPU draws a Wet Edges stroke");
    release(&mut app, pts[30]);
    assert!(app.live_stroke.is_none() && app.pending_stroke.is_none());
    assert!(app.committed_stroke.is_some(), "the CPU overlay's pixels were committed");
    assert_eq!(app.perf.gpu_dab_strokes, 0, "no GPU stroke");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let committed = layer_pixels(&app);
    assert!(committed.iter().any(|v| *v != 0.0), "the stroke is in the layer");
    assert!(committed == replayed(&mut app), "the commit is paint.stroke's pixels");
}

#[test]
fn cancelling_a_gpu_stroke_clears_its_mask_and_commits_nothing() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    let steps = app.session.active().unwrap().history.past_len();
    let pts = path();
    drag(&mut app, &pts[..12]);
    let layer = mask_layer(&app);
    assert!(has_mask(&app, layer));
    testing::key(&mut app, egui::Key::Escape, M);
    release(&mut app, pts[11]);
    assert!(!testing::in_gesture(&app) && app.live_stroke.is_none(), "the gesture is cancelled");
    assert!(!has_mask(&app, layer), "the mask is gone");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(!app.session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing was committed");
    assert!(app.pending_stroke.is_none() && app.perf.gpu_dab_strokes == 0);
}

#[test]
fn a_gpu_stroke_whose_canvas_fell_to_the_cpu_is_rendered_by_its_command_on_release() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    let pts = path();
    drag(&mut app, &pts[..10]);
    let layer = mask_layer(&app);
    let id = app.session.active().unwrap().doc.id.0;
    // A frame that left the GPU compositor (a budget, say): the CPU canvas would show the live
    // document with an empty mask.
    crate::render_path::note(&mut app, id, crate::render_path::Path::Cpu(crate::render_path::CpuReason::Unsupported("test".into())));
    for p in &pts[10..] {
        tool_event(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, M);
    }
    release(&mut app, pts[30]);
    assert!(app.live_stroke.is_none() && app.pending_stroke.is_none());
    assert!(!has_mask(&app, layer), "the mask went with the stroke");
    assert!(app.committed_stroke.is_none(), "the command rendered the stroke itself");
    assert_eq!(app.perf.gpu_dab_strokes, 0);
    let committed = layer_pixels(&app);
    assert!(committed.iter().any(|v| *v != 0.0), "the whole stroke is in the layer");
    assert!(committed == replayed(&mut app), "the commit is paint.stroke's pixels");
}

#[test]
fn switching_document_during_a_gpu_stroke_clears_its_mask() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    let pts = path();
    drag(&mut app, &pts[..12]);
    let layer = mask_layer(&app);
    assert!(has_mask(&app, layer));
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    testing::frame(&mut app);
    testing::frame(&mut app);
    assert!(app.live_stroke.is_none(), "the stroke ended with its document");
    assert!(!has_mask(&app, layer), "the mask is gone");
    assert!(app.pending_stroke.is_none() && app.perf.gpu_dab_strokes == 0);
}

#[test]
fn a_gpu_stroke_the_gpu_fails_to_stamp_is_dropped_and_rendered_by_its_command_on_release() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    let pts = path();
    drag(&mut app, &pts[..10]);
    let layer = mask_layer(&app);
    // The compositor loses the stroke's mask (as if it had been rebuilt): the next stamp is refused.
    app.gpu.as_ref().unwrap().clear_external_mask(layer);
    assert!(!has_mask(&app, layer));
    tool_event(&mut app, ToolEvent::Move { x: pts[10][0], y: pts[10][1], pressure: 1.0 }, M);
    assert!(app.live_stroke.is_none(), "the failed step drops the live stroke");
    for p in &pts[11..] {
        tool_event(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, M);
    }
    release(&mut app, pts[30]);
    assert!(app.pending_stroke.is_none() && !has_mask(&app, layer));
    assert!(app.committed_stroke.is_none(), "the command rendered the stroke itself");
    assert_eq!(app.perf.gpu_dab_strokes, 0);
    assert!(layer_pixels(&app).iter().any(|v| *v != 0.0), "the whole stroke is in the layer");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// The canvas's pixels of the active document as premultiplied RGBA.
fn canvas_texels(app: &PhotocraftApp) -> Vec<[f32; 4]> {
    let id = app.session.active().unwrap().doc.id.0;
    app.gpu.as_ref().unwrap().read_texels(id).expect("the canvas texture").2
}

/// A smoothed stroke (the default 10 % and a stronger one) takes the GPU path, shows the same live
/// document as the CPU overlay, tail included, and commits what `paint.stroke` writes.
#[test]
fn a_smoothed_brush_stroke_takes_the_gpu_path_and_looks_and_commits_like_the_cpu_overlay() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    for amount in [0.1, 0.6] {
        let what = format!("smoothing {amount}");
        let mut app = app_on_gpu(rs.clone(), amount);
        let mut cpu_app = app_with(None, amount, json!({}));
        let pts = path();
        // Mid-stroke, the pen well ahead of the smoothed brush.
        drag(&mut app, &pts[..21]);
        drag(&mut cpu_app, &pts[..21]);
        let live = app.live_stroke.as_ref().unwrap_or_else(|| panic!("{what}: a live stroke"));
        let ui = live.gpu.as_ref().unwrap_or_else(|| panic!("{what}: the GPU draws the smoothed stroke"));
        let (saved, stamped) = ui.tail_stats();
        assert!(stamped > 0 && saved > 0, "{what}: the stroke's tail was stamped ({stamped} tails, {saved} tiles saved)");
        assert!(cpu_app.live_stroke.as_ref().is_some_and(|l| l.gpu.is_none()), "{what}: the reference is the CPU overlay");

        let want = photocraft_compose::flatten(cpu_app.live_stroke.as_ref().unwrap().doc());
        let shown = canvas_texels(&app);
        assert_eq!(shown.len(), want.px.len(), "{what}: the canvas texture is the document's size");
        let (mut worst, mut painted) = (0.0f32, 0usize);
        for (i, w) in want.px.iter().enumerate() {
            let s = shown[i];
            painted += usize::from(w[1] < 0.9);
            worst = worst.max((0..3).map(|k| (s[k] - w[k] * w[3]).abs()).fold(0.0, f32::max));
        }
        assert!(painted > 200, "{what}: the stroke is on screen ({painted} pixels)");
        assert!(worst <= 3.0 / 255.0, "{what}: the live document differs from the CPU overlay's by {:.2}/255", worst * 255.0);

        for p in &pts[21..] {
            tool_event(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, M);
        }
        release(&mut app, pts[30]);
        assert!(app.live_stroke.is_none() && app.pending_stroke.is_none(), "{what}: the commit landed");
        assert!(app.committed_stroke.is_some(), "{what}: the commit took the live stroke's pixels");
        assert_eq!(app.perf.gpu_dab_strokes, 1, "{what}: the stroke took the GPU path");
        assert!(!app.ui.status_error, "{what}: {}", app.ui.status);
        assert_commit_matches_paint_stroke(&mut app, &what);
    }
}

/// A tile of dab geometry at `x`, `y`, 11 px in radius, for the stamping calls.
fn dab_at(x: f64, y: f64) -> photocraft_gpu::GpuDab {
    let ctx = photocraft_paint::BrushContext::new(&photocraft_paint::BrushSettings { size: 22.0, hardness: 0.3, ..Default::default() });
    let d = photocraft_paint::Dab::round(photocraft_geom::Point { x, y }, 11.0, 0.6);
    super::gpu_dabs::gpu_dab(&ctx.tip_geometry(&d).expect("a plain tip"))
}

/// No read-back ever contains the smoothing tail, whichever way a request gets to the read-back:
/// asked for right after the settled stamps (as a step's own), held in the backlog while a tail is
/// shown, or asked for after the next step's dabs. (Were a read-back recorded after a tail's
/// stamps, its tiles would hold the tail's coverage and differ from the settled ones: the
/// unchecked read shows that it would.) Moving the `Read` after the `Tail` in a step, or asking
/// for the backlog without holding back the tail's tiles, fails the equalities below.
#[test]
fn a_read_back_never_sees_the_smoothing_tail() {
    use super::gpu_dabs::GpuUi;
    use super::paint_thread::GpuWork;
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = app_on_gpu(rs, 0.0);
    testing::frame(&mut app);
    let (doc, layer) = {
        let st = app.session.active().unwrap();
        (st.doc.clone(), st.active_layer.unwrap())
    };
    let gpu = app.gpu.clone().expect("a GPU canvas");
    gpu.begin_external_mask(doc.id, layer, doc.bounds()).expect("a compositor");
    let mut ui = GpuUi::for_test(gpu, layer);
    // Calm tiles (0, 1) and (1, 1), settled; a tail over the same tiles, and past them.
    let keys = vec![(0, 1), (1, 1)];
    let settled = vec![dab_at(60.0, 80.0), dab_at(62.0, 84.0)];
    let tail = vec![dab_at(66.0, 80.0), dab_at(70.0, 82.0), dab_at(90.0, 100.0)];
    let more = vec![dab_at(58.0, 90.0)];

    // A step as the thread makes it: dabs, the request, the tail. The request sees the settled
    // coverage only.
    ui.run(vec![GpuWork::Dabs(settled.clone()), GpuWork::Read(keys.clone(), 1), GpuWork::Tail(tail.clone())], false).unwrap();
    assert!(ui.tail_shown());
    let first = ui.collect();
    assert_eq!(first.len(), keys.len());
    // What a read-back that came after the tail's stamps would have got.
    let with_tail = ui.read_unchecked(&keys);
    assert!(with_tail.iter().zip(&first).any(|(a, b)| a.1 != b.1), "the tail changes the tiles under it");

    // The same request, handed over while the tail is shown and nothing else is: its tiles are
    // under the tail, so it waits in the backlog.
    ui.run(vec![GpuWork::Read(keys.clone(), 2)], false).unwrap();
    assert_eq!((ui.backlog_len(), ui.in_flight()), (1, 0), "tiles under the tail are not asked for while it is shown");
    // The next step's dabs take the tail back first; then the request goes, and sees no tail.
    ui.run(vec![GpuWork::Dabs(vec![]), GpuWork::Tail(vec![])], false).unwrap();
    assert!(!ui.tail_shown());
    assert_eq!(ui.backlog_len(), 0);
    let held = ui.collect();
    assert_eq!(held.iter().map(|c| &c.1).collect::<Vec<_>>(), first.iter().map(|c| &c.1).collect::<Vec<_>>(), "the held request read the settled coverage");

    // Two steps in one batch (the canvas was behind), the first's tail replaced by the second's;
    // the second step's dabs land on the settled coverage, not on the first tail.
    ui.run(
        vec![GpuWork::Dabs(vec![]), GpuWork::Tail(tail.clone()), GpuWork::Dabs(more.clone()), GpuWork::Read(keys.clone(), 3), GpuWork::Tail(tail.clone())],
        false,
    )
    .unwrap();
    assert!(ui.tail_shown());
    let second = ui.collect();
    let unchecked = ui.read_unchecked(&keys);
    assert!(unchecked.iter().zip(&second).any(|(a, b)| a.1 != b.1), "the tail is shown over the second read's tiles");
    // The settled coverage once the tail is taken back: what the second read must have been.
    ui.run(vec![GpuWork::Dabs(vec![])], false).unwrap();
    let settled_now = ui.read_unchecked(&keys);
    assert_eq!(second.iter().map(|c| &c.1).collect::<Vec<_>>(), settled_now.iter().map(|c| &c.1).collect::<Vec<_>>(), "the read saw no tail");
    assert!(second.iter().zip(&first).any(|(a, b)| a.1 != b.1), "the second step's dabs are in it");
}
