//! The CPU canvas's frame time on the offscreen software adapter (a manual benchmark).
use super::*;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The CPU-canvas fixture on the offscreen software adapter, or `None` when this machine has no
/// graphics adapter. Only creating the renderer may be skipped: a failing fixture command or a
/// panic in the first frames must fail the test, not pass it as "no adapter".
fn harness(size: [u32; 2], background: &str) -> Option<Harness> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // app.gpu remains None: compose on the CPU, use the harness's software window adapter.
    app.run("file.new", json!({"width": size[0], "height": size[1], "background": background})).unwrap();
    app.run("select.rect", json!({"x": size[0] / 2, "y": size[1] / 2, "width": size[0] / 10, "height": size[1] / 10})).unwrap();
    app.run("edit.fill", json!({"color": "#2050d0"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    // `wgpu()` creates the render state and panics when no adapter is found.
    let builder = std::panic::catch_unwind(|| {
        egui_kittest::Harness::<Box<PhotocraftApp>>::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu()
    })
    .ok()?;
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        if let Some(rs) = &cc.wgpu_render_state {
            app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        }
        app
    });
    h.run_steps(12);
    Some(h)
}

#[test]
#[ignore = "manual release benchmark: 24 MP CPU canvas on the offscreen software adapter"]
fn cpu_canvas_background_benchmark() {
    let _gpu = photocraft_testkit::gpu_lock();
    use std::time::Instant;
    let mut results = Vec::new();
    for background in ["white", "transparent"] {
        let mut h = harness([6000, 4000], background).expect("an offscreen graphics adapter");
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = 1.0;
        v.center = [3000.0, 2000.0];
        v.clear_pending();
        for _ in 0..3 {
            h.step();
            h.render().unwrap();
        }
        let mut frames = Vec::new();
        for n in 0..24 {
            h.state_mut().ui.views[0].center[0] = 3000.0 + if n % 2 == 0 { -3.0 } else { 3.0 };
            let t = Instant::now();
            h.step();
            h.render().unwrap();
            frames.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        frames.sort_by(f64::total_cmp);
        results.push(json!({
            "background": background, "document_pixels": 24_000_000, "viewport": [1440, 900],
            "adapter": h.state().perf.gpu_info, "samples": frames.len(),
            "p50_ms": frames[11], "p95_ms": frames[22], "frames_ms": frames,
            "includes": "UI frame, the canvas's raster display and synchronous readback; no document recomposite",
        }));
    }
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}
