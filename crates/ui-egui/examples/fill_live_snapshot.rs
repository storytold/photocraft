//! Offscreen visual check of a selected mask Fill during an uncommitted LMB rotation drag.
#[cfg(not(feature = "uv-workbench"))]
fn main() {}

#[cfg(feature = "uv-workbench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::kittest::Queryable;
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services, canvas::ViewXform, state::Tool};
    use serde_json::json;
    use std::time::{Duration, Instant};

    let mut session = Session::new();
    session.execute("uv.workspace.activate", json!({"active":true}))?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    session.execute("uv.model.import", json!({"path":source}))?;
    session.execute("uv.texture.create", json!({"width":4096,"height":4096}))?;
    session.execute("edit.fill", json!({"color":"#649ecf"}))?;
    session.execute("layer.layerMask.hideAll", json!({}))?;
    session.execute("uv.maskPaint.add", json!({"name":"Live Fill","kind":"fill","value":1.0}))?;
    session.execute("uv.maskPaint.modify", json!({"index":0,"bounds":[1024,1024,3072,3072]}))?;
    session.execute("view.layerMask", json!({"mode":"gray"}))?;
    let layer_id = session.active().and_then(|state| state.active_layer).ok_or("No active mask layer")?;
    let doc_id = session.active().ok_or("No active document")?.doc.id.0;
    let original_raster = session.active().and_then(|state| state.doc.layer(layer_id)).and_then(|layer| layer.surface().cloned()).ok_or("No parent raster")?;
    let original_bounds = session
        .active()
        .and_then(|state| state.doc.layer(layer_id))
        .and_then(|layer| layer.mask.as_ref())
        .and_then(|mask| mask.paint.as_ref())
        .and_then(|stack| stack.layers.first())
        .and_then(|fill| fill.fill_bounds)
        .ok_or("Fill bounds were not initialized")?;

    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 900.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(session, Services::default());
        if let Some(render) = &cc.wgpu_render_state {
            let _ = app.set_uv_wgpu(render.clone());
        }
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        app.ui.tool = Tool::Move;
        app.ui.uv_workbench.view_3d = false;
        app.ui.uv_workbench.split_view = false;
        app.ui.dock_tabs.properties = 0;
        app
    });
    h.state_mut().sync_views();
    if let Some(index) = h.state().session.active_index()
        && let Some(view) = h.state_mut().ui.views.get_mut(index)
    {
        // Fit the 4096² image into the visible canvas and keep the Fill handles on-screen.
        view.zoom = 0.14;
        view.center = [2048.0, 2048.0];
        view.fit_pending = false;
        view.fill_pending = false;
    }
    h.run_steps(8);
    // Headless wgpu harnesses may start without an OS focus notification. Model the
    // focused window needed for the mouse gesture explicitly; later focus-loss recovery
    // is still exercised by the in-process unit test.
    h.event(Event::WindowFocused(true));
    h.step();
    if h.query_by_label("Live Fill").is_none() {
        return Err("Mask Paint effects or the uniquely named Fill row were not visible".into());
    }
    fn click(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, p: Pos2) {
        h.hover_at(p);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(2);
    }
    fn rotate_vec([x, y]: [f64; 2], degrees: f32) -> [f64; 2] {
        let angle = f64::from(degrees).to_radians();
        let (sin, cos) = angle.sin_cos();
        [cos * x - sin * y, sin * x + cos * y]
    }
    fn fill_corners(bounds: [i32; 4], rotation: f32) -> [[f64; 2]; 4] {
        let [x0, y0, x1, y1] = bounds;
        let center = [(f64::from(x0) + f64::from(x1)) * 0.5, (f64::from(y0) + f64::from(y1)) * 0.5];
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|[x, y]| {
            let offset = rotate_vec([f64::from(x) - center[0], f64::from(y) - center[1]], rotation);
            [center[0] + offset[0], center[1] + offset[1]]
        })
    }
    let row = h.get_by_label("Live Fill").rect();
    click(&mut h, row.center());
    let xf = ViewXform::active(h.state()).ok_or("No canvas transform after selecting Fill")?;
    let corners = [xf.to_screen(1024.0, 1024.0), xf.to_screen(3072.0, 1024.0), xf.to_screen(3072.0, 3072.0), xf.to_screen(1024.0, 3072.0)];
    let top = corners[0].lerp(corners[1], 0.5);
    let center = corners[0].lerp(corners[2], 0.5);
    let outward = (top - center).normalized();
    let outside = top + outward * 24.0;
    let start = if xf.rect.contains(outside) { outside } else { top - outward * 24.0 };
    let end = start + vec2(64.0, 32.0);
    if !xf.rect.contains(start) || !xf.rect.contains(end) {
        return Err(format!("Fill rotation handle is outside the canvas: {start:?} → {end:?}, {:?}", xf.rect).into());
    }

    let rotation_start_time = Instant::now();
    h.event(Event::PointerMoved(start));
    h.step();
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(end));
    h.step();
    h.step();
    let rotation_input_ms = rotation_start_time.elapsed().as_secs_f64() * 1000.0;

    if h.state().ui.transform.is_some() {
        return Err("Fill gesture opened a native parent transform".into());
    }
    let active = h.state().session.active().ok_or("Document closed during Fill gesture")?;
    let layer = active.doc.layer(layer_id).ok_or("Mask layer disappeared")?;
    let fill = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()).and_then(|stack| stack.layers.first()).ok_or("Fill effect disappeared")?;
    if layer.surface() != Some(&original_raster) {
        return Err("Parent raster changed during transient Fill manipulation".into());
    }
    if fill.fill_bounds != Some(original_bounds) {
        return Err(format!("Canonical Fill bounds changed during held rotation: expected {original_bounds:?}, found {:?}", fill.fill_bounds).into());
    }
    let rotation_before = match &fill.kind {
        photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } => *rotation,
        _ => return Err("Selected effect stopped being a Fill".into()),
    };
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    if h.state().ui.transform.is_some() {
        return Err("Releasing Fill rotation opened a native parent transform".into());
    }
    let rotated = h
        .state()
        .session
        .active()
        .and_then(|state| state.doc.layer(layer_id))
        .and_then(|layer| layer.mask.as_ref())
        .and_then(|mask| mask.paint.as_ref())
        .and_then(|stack| stack.layers.first())
        .ok_or("Fill effect disappeared after rotation")?;
    let rotation = match &rotated.kind {
        photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } if rotation.is_finite() && rotation.to_bits() != rotation_before.to_bits() => *rotation,
        _ => return Err("Held rotation did not commit a finite Fill angle".into()),
    };
    let rotated_bounds = rotated.fill_bounds.ok_or("Fill bounds disappeared after rotation")?;
    if rotated_bounds != original_bounds {
        return Err("Rotation unexpectedly changed canonical Fill bounds".into());
    }
    if h.state().session.active().and_then(|state| state.doc.layer(layer_id)).and_then(|layer| layer.surface()) != Some(&original_raster) {
        return Err("Parent raster changed during Fill rotation".into());
    }

    let resize_xf = ViewXform::active(h.state()).ok_or("No canvas transform after committing rotation")?;
    let doc_corners = fill_corners(rotated_bounds, rotation);
    let screen_corners = doc_corners.map(|[x, y]| resize_xf.to_screen(x as f32, y as f32));
    let resize_start = screen_corners[2];
    let local_growth = rotate_vec([350.0, 270.0], rotation);
    let resize_doc_end = [doc_corners[2][0] + local_growth[0], doc_corners[2][1] + local_growth[1]];
    let resize_end = resize_xf.to_screen(resize_doc_end[0] as f32, resize_doc_end[1] as f32);
    if !resize_xf.rect.contains(resize_start) || !resize_xf.rect.contains(resize_end) {
        return Err(format!("Rotated Fill resize points are outside the canvas: {resize_start:?} → {resize_end:?}, {:?}", resize_xf.rect).into());
    }
    let resize_started = Instant::now();
    h.event(Event::PointerMoved(resize_start));
    h.step();
    h.event(Event::PointerButton { pos: resize_start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(resize_end));
    h.step(); // immediate, raw-resolution resize preview frame
    let raw_ms = resize_started.elapsed().as_secs_f64() * 1000.0;

    let assert_transient_invariants = |h: &egui_kittest::Harness<'_, PhotocraftApp>| -> Result<(), Box<dyn std::error::Error>> {
        if h.state().ui.transform.is_some() {
            return Err("Fill resize opened a native parent transform".into());
        }
        let active = h.state().session.active().ok_or("Document closed during Fill resize")?;
        let layer = active.doc.layer(layer_id).ok_or("Mask layer disappeared during Fill resize")?;
        let fill =
            layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()).and_then(|stack| stack.layers.first()).ok_or("Fill effect disappeared during resize")?;
        if layer.surface() != Some(&original_raster) {
            return Err("Parent raster changed during transient Fill resize".into());
        }
        if fill.fill_bounds != Some(rotated_bounds) {
            return Err(format!("Canonical Fill bounds changed during held resize: expected {rotated_bounds:?}, found {:?}", fill.fill_bounds).into());
        }
        Ok(())
    };
    assert_transient_invariants(&h)?;
    let output = std::env::args().nth(1).unwrap_or_else(|| "fill-resize-live.png".into());
    let output_path = std::path::PathBuf::from(&output);
    let raw_path = output_path.with_file_name(format!("{}-raw.png", output_path.file_stem().and_then(|name| name.to_str()).unwrap_or("fill-resize-live")));
    h.render()?.save(&raw_path)?;

    // Let coalesced image work and its 20 Hz refresh schedule run between harness frames.
    // The sleep is outside egui's frame; this records a smoke wait, not a frame-rate benchmark.
    let wait_start = Instant::now();
    let deadline = wait_start + Duration::from_millis(500);
    let mut wait_frames = 0u32;
    while Instant::now() < deadline && wait_frames < 10 {
        std::thread::sleep(Duration::from_millis(50));
        h.step();
        wait_frames += 1;
        assert_transient_invariants(&h)?;
    }
    let waited_ms = wait_start.elapsed().as_secs_f64() * 1000.0;
    h.render()?.save(&output_path)?;
    println!("Live rotated Fill resize raw frame: {}", raw_path.display());
    println!("Live rotated Fill resize settled frame: {}", output_path.display());
    println!("Rotation input: 3 pointer events / 4 harness frames in {rotation_input_ms:.2} ms; resize raw frame in {raw_ms:.2} ms.");
    println!("Held resize waited {wait_frames} frames / {waited_ms:.0} ms (20 Hz smoke cadence, not a performance benchmark).");
    println!("Transient mask preview cap: 256 × 256 for the 4096 × 4096 texture; pointer remained held, canonical bounds and parent raster stayed unchanged.");
    println!("Document {doc_id}, layer {}: parent transform remained inactive.", layer_id.0);
    Ok(())
}
