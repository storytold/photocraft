//! Sustained full-resolution live painting, including completed GPU compositing and mip work.
//! `cargo run --release -p photocraft-ui-egui --example brush_latency_bench -- --size 20000 --brush 1000`
//! Synthetic sparse/default pixels; no imported photograph, UI event queue or monitor scanout.

use std::time::Instant;

use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_ui_egui::gpu_canvas::{GpuCanvas, wgpu_setup};
use serde_json::json;

fn option(name: &str, default: u32) -> u32 {
    let args: Vec<_> = std::env::args().collect();
    args.iter().position(|arg| arg == name).and_then(|i| args.get(i + 1)).and_then(|arg| arg.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let side = option("--size", 20_000).clamp(1024, 30_000);
    let diameter = option("--brush", 1000).clamp(1, 5000);
    let depth = option("--depth", 8);
    let spacing = f64::from(option("--spacing-percent", 5).clamp(1, 100)) / 100.0;
    let step = f64::from(option("--step", 40).clamp(1, 1000));
    let frames = option("--frames", 90).clamp(10, 1000);
    let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default()))
        .map_err(|_| "cannot create a GPU render state; run with access to a supported adapter")?;
    let canvas = GpuCanvas::new(&rs);
    println!("adapter: {:?}", rs.adapter.get_info());
    let mut session = Session::new();
    #[cfg(not(target_arch = "wasm32"))]
    let brush_gpu = std::sync::Arc::new(photocraft_gpu::brush::Rasterizer::new(rs.device.clone(), rs.queue.clone(), canvas.health().clone()));
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("PHOTOCRAFT_GPU_BRUSH").as_deref() != Ok("0") {
        session.brush_accelerator = Some(brush_gpu.clone());
    }
    session.execute("file.new", json!({"width": side, "height": side, "depth": depth, "background": "white"}))?;
    let initial = session.active().ok_or("missing document")?.doc.clone();
    let start = Instant::now();
    let cold = canvas.refresh(initial.id.0, &initial, None, None);
    if !canvas.health().wait(&rs.device, None) {
        return Err("GPU failed during initial refresh".into());
    }
    println!(
        "initial refresh {:.1} ms ({}, {:?}), display {:?}",
        start.elapsed().as_secs_f64() * 1000.0,
        cold.kind,
        cold.fallback,
        canvas.texture_info(initial.id.0)
    );
    let x0 = f64::from(side) * 0.35;
    let y0 = f64::from(side) * 0.5;
    let mut params = json!({"points": [[x0, y0, 1, 0, 0, 0, 0]], "size": diameter, "seed": 42,
        "brush": {"pressureSize": false, "spacing": spacing, "flow": 0.3, "hardness": 0.0, "smoothing": {"amount": 0.0}}, "target": "pixels"});
    let mut live = LiveStroke::begin(&session, &params)?;
    let mut points = vec![[x0, y0, 1.0, 0.0, 0.0, 0.0, 0.0]];
    let mut elapsed = Vec::new();
    let mut raster = Vec::new();
    for frame in 1..=frames {
        let point = [x0 + f64::from(frame) * step, y0 + (f64::from(frame) * 0.15).sin() * 250.0, 1.0, 0.0, 0.0, 0.0, f64::from(frame) * 1000.0 / 60.0];
        let start = Instant::now();
        let mut sample = photocraft_engine::paint::StrokePoint::new(point[0], point[1], 1.0);
        sample.time = point[6];
        let damage = live.push(&[sample])?;
        let paint_ms = start.elapsed().as_secs_f64() * 1000.0;
        let refresh = canvas.refresh(initial.id.0, &live.doc, Some(damage), None);
        if !canvas.health().wait(&rs.device, None) {
            return Err("GPU failed while painting".into());
        }
        if !refresh.kind.starts_with("gpu") && !damage.is_empty() {
            return Err(format!("CPU fallback: {:?}", refresh.fallback).into());
        }
        if frame > 5 {
            raster.push(paint_ms);
            elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        points.push(point);
    }
    raster.sort_by(f64::total_cmp);
    elapsed.sort_by(f64::total_cmp);
    let at = |v: &[f64], percent: usize| v.get(v.len().saturating_sub(1) * percent / 100).copied().unwrap_or(0.0);
    params["points"] = json!(points);
    let start = Instant::now();
    live.prepare(&mut session);
    session.execute("paint.stroke", params)?;
    println!(
        "{side}x{side} depth={depth} brush={diameter} frames={frames}: stroke update p50={:.3} p95={:.3} ms; paint+completed GPU p50={:.3} p95={:.3} p99={:.3} max={:.3} ms; commit={:.3} ms",
        at(&raster, 50),
        at(&raster, 95),
        at(&elapsed, 50),
        at(&elapsed, 95),
        at(&elapsed, 99),
        at(&elapsed, 100),
        start.elapsed().as_secs_f64() * 1000.0
    );
    #[cfg(not(target_arch = "wasm32"))]
    println!("completed GPU brush batches: {}", brush_gpu.batches());
    Ok(())
}
