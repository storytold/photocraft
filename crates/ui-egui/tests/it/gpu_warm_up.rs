//! The GPU compositor is built in the background, so the first document doesn't wait for its
//! pipelines to compile: on DX12, FXC compiles them one at a time, and building them on the UI
//! thread froze the app for seconds the first time a file was opened (#6419, #8733). The canvas
//! draws with the CPU compositor until the build is done (a transient state, not a fault), and
//! the GPU takes over then. Skips when there is no GPU adapter.

use std::time::{Duration, Instant};

use eframe::egui_wgpu::RenderState;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_ui_egui::gpu_canvas::{self, GpuCanvas};

use photocraft_testkit::gpu_lock;

/// A headless GPU canvas on a device created like the app's.
fn canvas() -> Option<(GpuCanvas, RenderState)> {
    let rs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        egui_kittest::wgpu::create_render_state(gpu_canvas::wgpu_setup(), Default::default())
    }))
    .ok();
    let Some(rs) = rs else {
        eprintln!("skipping: no GPU adapter");
        return None;
    };
    Some((GpuCanvas::new(&rs), rs))
}

/// A one-layer 300×200 RGB document.
fn doc() -> Document {
    Document::with_background("t", Size::new(300, 200), ColorMode::Rgb, SampleType::U8, Color::rgba(0.2, 0.5, 0.8, 1.0))
}

#[test]
fn the_compositor_is_built_in_the_background_and_the_gpu_takes_over_afterwards() {
    let _gpu = gpu_lock();
    let Some((g, _rs)) = canvas() else { return };
    let ctx = egui::Context::default();
    g.warm_up(&ctx);
    // The build is running: this refresh draws with the CPU compositor and says it is warming
    // up, without calling it a fault (no fallback reason).
    let d = doc();
    let r = g.refresh(d.id.0, &d, None, None);
    assert!(r.warming || r.kind.starts_with("gpu"), "the refresh while warming: {:?} {:?}", r.kind, r.fallback);
    if r.warming {
        assert!(r.fallback.is_none(), "warming up is not a fallback: {:?}", r.fallback);
    }
    // The build finishes on its own and the app is told (the generation changes).
    let deadline = Instant::now() + Duration::from_secs(180);
    while g.warm_generation() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(g.warm_generation(), 0, "the background build finished");
    // Now the GPU composites, with no waiting and no fallback.
    let d2 = doc();
    let r2 = g.refresh(d2.id.0, &d2, None, None);
    assert!(r2.kind.starts_with("gpu"), "the GPU composites once the build is done: {:?} {:?}", r2.kind, r2.fallback);
    assert!(!r2.warming, "the compositor is ready");
}
