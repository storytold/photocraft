//! Real compute coverage must remain ordinary native pixels after committing a live gesture.
use photocraft_engine::{Session, brush_cmds::LiveStroke, paint::StrokePoint};
use photocraft_gpu::{DeviceHealth, brush::Rasterizer};
use serde_json::json;
use std::sync::Arc;

#[test]
fn computed_live_stroke_commits_native_pixels_and_keeps_history() {
    let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(photocraft_ui_egui::gpu_canvas::wgpu_setup(), Default::default())) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut backend = Rasterizer::new(rs.device.clone(), rs.queue.clone(), DeviceHealth::watch(&rs.device));
    backend.min_work = 0;
    backend.prewarm();
    let backend = Arc::new(backend);
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.brush_accelerator = Some(backend.clone());
        session.execute("file.new", json!({"width": 1000, "height": 700, "depth": depth, "background": "transparent"})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let layer = session.active().unwrap().active_layer.unwrap();
        let points = [[200.25, 300.75, 1.0], [450.75, 320.125, 1.0], [700.5, 400.25, 1.0]];
        let mut p = json!({"points": [points[0]], "size": 300, "seed": 42, "color": "#3070b0", "target": "pixels",
            "brush": {"hardness": 0, "pressureSize": false, "spacing": 0.02, "smoothing": {"amount": 0}}});
        let count = backend.batches();
        let mut live = LiveStroke::begin(&session, &p).unwrap();
        for point in &points[1..] {
            live.push(&[StrokePoint::new(point[0], point[1], 1.0)]).unwrap();
        }
        assert!(backend.batches() > count);
        let shown = live.doc.clone();
        p["points"] = json!(points);
        live.prepare(&mut session);
        session.execute("paint.stroke", p).unwrap();
        let committed = session.active().unwrap().doc.clone();
        let pixels = committed.layer(layer).unwrap().surface().unwrap();
        let saved = photocraft_format::save_to_bytes(&committed, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(restored.layer(layer).unwrap().surface(), Some(pixels), "native save must preserve computed pixels");
        for (key, tile) in shown.layer(layer).unwrap().surface().unwrap().tiles() {
            assert!(Arc::ptr_eq(tile, pixels.tile(*key).unwrap()), "adopt computed pixels without replay");
        }
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().surface(), before.layer(layer).unwrap().surface());
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().surface(), Some(pixels));
    }
}
