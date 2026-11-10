use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;

/// A wgpu render state, or `None` (the test skips) without a GPU adapter, unless
/// `PHOTOCRAFT_REQUIRE_GPU_TESTS` asks for one.
fn render_state() -> Option<eframe::egui_wgpu::RenderState> {
    match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
        Ok(rs) => Some(rs),
        Err(error) => {
            if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping redraw thread test: no GPU adapter");
            None
        }
    }
}

/// A block drawn on another thread through `composite_lent` lands in the canvas texture as the UI
/// thread would draw it, from the document it is given, and nothing outside the block changes.
/// Without a texture to draw into it fails instead, and either way the compositor is back for
/// the UI thread afterwards.
#[test]
fn a_block_drawn_off_the_ui_thread_matches_the_ui_thread_and_only_that_block_changes() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.run("file.new", json!({"width": 1024, "height": 768, "background": "#336699"})).unwrap();
    let gpu = app.gpu.clone().unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let key = before.id.0;
    let (display, _) = crate::canvas::canvas_display(&app, &before, None);
    let encode_srgb = display.as_deref().is_some_and(|d| d.encode_srgb);

    // Nothing drawn yet: there is no texture to draw a block into.
    let (g, d) = (gpu.clone(), before.clone());
    let none = std::thread::spawn(move || g.composite_lent(|| Some((d, encode_srgb)), &[Rect::new(0, 0, 64, 64)])).join().unwrap();
    assert!(none.is_err(), "no canvas texture yet");

    let r = gpu.refresh_with(key, &before, None, display.as_deref(), false);
    if !r.kind.starts_with("gpu") {
        eprintln!("skipping redraw thread test: no GPU compositor ({}, {:?})", r.kind, r.fallback);
        return;
    }
    // A layer filled over everything; only one block of it is drawn, on another thread.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#cc8844"})).unwrap();
    let after = app.session.active().unwrap().doc.clone();
    let block = Rect::new(0, 0, 512, 256);
    let (g, d) = (gpu.clone(), after.clone());
    let drawn = std::thread::spawn(move || g.composite_lent(|| Some((d, encode_srgb)), &[block])).join().unwrap();
    assert!(drawn.is_ok(), "{drawn:?}");

    // Each version composited into a reference texture of its own.
    let (old_ref, new_ref) = (key ^ (1u64 << 60), key ^ (1u64 << 61));
    gpu.upload_composite(old_ref, &before, display.as_deref());
    gpu.upload_composite(new_ref, &after, display.as_deref());
    let (_, size, shown) = gpu.read_texels(key).expect("canvas texture");
    let (_, _, old) = gpu.read_texels(old_ref).expect("reference texture");
    let (_, _, new) = gpu.read_texels(new_ref).expect("reference texture");
    for y in 0..size[1] as i32 {
        for x in 0..size[0] as i32 {
            let i = y as usize * size[0] as usize + x as usize;
            let (want, which) = if block.contains(x, y) { (new[i], "the block, as the new document") } else { (old[i], "outside the block, as before") };
            for c in 0..4 {
                assert!((shown[i][c] - want[c]).abs() <= 2.0 / 255.0, "({x},{y}) channel {c}: {which}");
            }
        }
    }
    // The compositor went back: the UI thread draws on the GPU as before.
    let r = gpu.refresh_with(key, &after, None, display.as_deref(), false);
    assert!(r.kind.starts_with("gpu"), "the compositor is back for the UI thread: {} {:?}", r.kind, r.fallback);
}
