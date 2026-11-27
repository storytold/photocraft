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
/// soft red stroke with `smoothing` (the GPU draws only an unsmoothed one).
fn app_on_gpu(rs: eframe::egui_wgpu::RenderState, smoothing: f32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    testing::still_clock(&mut app);
    app.run("file.new", json!({"width": W, "height": H, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#d02060"})).unwrap();
    app.run(
        "tools.setBrush",
        json!({"brush": {"size": 22, "hardness": 0.3, "flow": 0.6, "opacity": 0.8, "pressureSize": false, "smoothing": {"amount": smoothing}}}),
    )
    .unwrap();
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

    let gpu = layer_pixels(&app);
    assert!(gpu.iter().any(|v| *v != 0.0), "the stroke is in the layer");
    let cpu = replayed(&mut app);
    // R32F is within a few 1e-6 of the CPU's coverage; R16F rounds each blend (and lavapipe worse
    // than a GPU), see `gpu_commit_tests`.
    let full = photocraft_gpu::Compositor::full_precision_masks(&app.gpu.as_ref().unwrap().device());
    let limit = (if full { 1.0 / 255.0 } else { 3.0 / 255.0 }) + 4.0 * f32::EPSILON;
    let worst = gpu.iter().zip(&cpu).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst <= limit, "the committed stroke differs from paint.stroke by {:.2}/255 (full precision {full})", worst * 255.0);
}

#[test]
fn a_stroke_the_gpu_cannot_draw_takes_the_cpu_overlay_path_and_commits_as_before() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    // Smoothing is not a dab list a push ends with: the stroke is the CPU's.
    let mut app = app_on_gpu(rs, 0.4);
    let pts = path();
    drag(&mut app, &pts);
    let live = app.live_stroke.as_ref().expect("a live stroke");
    assert!(live.gpu.is_none(), "the CPU draws a smoothed stroke");
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
