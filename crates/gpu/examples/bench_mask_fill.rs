//! CPU mask-stack refresh plus GPU render versus direct GPU Fill transforms.
//! `cargo run --release -p photocraft-gpu --example bench_mask_fill [-- --large] [--side 512]`
//! This isolates compositor work; it is not an end-to-end UI or input-latency benchmark.
use std::{error::Error, time::Instant};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, LayerMask,
    mask_paint::{EffectKind, Paint, Stack},
};
use photocraft_geom::{Rect, Size};
use photocraft_gpu::{Compositor, Stats};
use photocraft_raster::Surface;

const SOURCE_WIDTH: u32 = 1024;
const SOURCE_HEIGHT: u32 = 512;
const MEASURED_FRAMES: usize = 6;

fn render(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document) -> Result<Stats, Box<dyn Error>> {
    let stats = comp.render(device, queue, doc, doc.bounds(), |_, _| {})?;
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;
    Ok(stats)
}

fn scene(size: Size) -> Result<Document, Box<dyn Error>> {
    let gray = PixelFormat::GRAY8;
    let rgba = PixelFormat::RGBA8;
    let mut base = Surface::with_default(gray, &[0.72]);
    base.write_region(Rect::new(0, 0, 128, 128), &vec![0.92; 128 * 128]);

    let mut paint_surface = Surface::with_default(rgba, &[0.08, 0.05, 0.02, 0.0]);
    let paint_pixels: Vec<f32> = (0..128 * 128)
        .flat_map(|index| {
            let alpha = if index % 7 == 0 { 0.8 } else { 0.35 };
            [0.25, 0.4, 0.7, alpha]
        })
        .collect();
    paint_surface.write_region(Rect::new(0, 0, 128, 128), &paint_pixels);

    let mut fill_map = Surface::with_default(gray, &[0.0]);
    let pixels: Vec<f32> = (0..SOURCE_HEIGHT).flat_map(|y| (0..SOURCE_WIDTH).map(move |x| ((x * 13 + y * 29) % 1024) as f32 / 1023.0)).collect();
    fill_map.write_region(Rect::new(0, 0, SOURCE_WIDTH as i32, SOURCE_HEIGHT as i32), &pixels);

    let width = (size.width as f32 * 0.55).round() as i32;
    let height = (size.height as f32 * 0.5).round() as i32;
    let center_x = size.width as i32 / 2;
    let center_y = size.height as i32 / 2;
    let bounds = [center_x - width / 2, center_y - height / 2, center_x - width / 2 + width, center_y - height / 2 + height];
    let layers = vec![
        Paint {
            name: "Paint".into(),
            visible: true,
            opacity: 0.82,
            blend: photocraft_color::BlendMode::Normal,
            surface: paint_surface,
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        },
        Paint {
            name: "Image Fill".into(),
            visible: true,
            opacity: 0.9,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::new(rgba),
            fill_map: Some(fill_map),
            fill_map_size: [SOURCE_WIDTH, SOURCE_HEIGHT],
            fill_bounds: Some(bounds),
            kind: EffectKind::Fill { value: 0.5, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false },
        },
        Paint {
            name: "Levels".into(),
            visible: true,
            opacity: 0.88,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::new(rgba),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Levels { input_black: 0.04, gamma: 0.9, input_white: 0.96, output_black: 0.02, output_white: 0.98 },
        },
    ];
    let mut mask = LayerMask::reveal_all();
    mask.surface = base.clone();
    mask.paint = Some(Stack { base, layers, active: 0 });

    let mut layer_surface = Surface::with_default(rgba, &[0.8, 0.72, 0.64, 1.0]);
    layer_surface.fill_rect(Rect::new(0, 0, size.width as i32, size.height as i32), &[0.8, 0.72, 0.64, 1.0]);
    let mut layer = Layer::new("Masked raster", LayerContent::Raster(layer_surface));
    layer.mask = Some(mask);
    let mut doc = Document::new("mask Fill benchmark", size, ColorMode::Rgb, SampleType::U8);
    doc.layers.push(layer);
    Ok(doc)
}

/// Move the transformed Fill rectangle and rotate its image source, without flattening the stack.
fn transformed(base: &Document, frame: usize) -> Result<Document, Box<dyn Error>> {
    let mut doc = base.clone();
    let mask = doc.layers.get_mut(0).and_then(|layer| layer.mask.as_mut()).ok_or("missing layer mask")?;
    let stack = mask.paint.as_mut().ok_or("missing mask stack")?;
    let fill = stack.layers.get_mut(1).ok_or("missing Fill layer")?;
    let bounds = fill.fill_bounds.ok_or("missing Fill bounds")?;
    let dx = (frame as i32 % 3 - 1) * 64;
    let dy = (frame as i32 % 4 - 1) * 48;
    fill.fill_bounds = Some([bounds[0] + dx, bounds[1] + dy, bounds[2] + dx, bounds[3] + dy]);
    let EffectKind::Fill { rotation, .. } = &mut fill.kind else { return Err("second mask effect is not Fill".into()) };
    *rotation = (frame as f32 * 13.0).rem_euclid(180.0);
    Ok(doc)
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    let rank = ((samples.len() as f64 * percentile).ceil() as usize).saturating_sub(1).min(samples.len().saturating_sub(1));
    samples.get(rank).copied().unwrap_or_default()
}

fn requested_size(args: &[String]) -> Result<Size, Box<dyn Error>> {
    let mut size = if args.iter().any(|arg| arg == "--large") {
        Size::new(6000, 4000)
    } else if args.iter().any(|arg| arg == "--quick") {
        Size::new(1024, 768)
    } else {
        Size::new(4096, 4096)
    };
    if let Some(index) = args.iter().position(|arg| arg == "--side") {
        let value = args.get(index + 1).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "--side requires a size from 128 through 8192"))?;
        let side =
            value.parse::<u32>().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "--side must be an integer from 128 through 8192"))?;
        if !(128..=8192).contains(&side) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "--side must be from 128 through 8192").into());
        }
        size = Size::new(side, side);
    }
    Ok(size)
}

fn run_mode(
    label: &str,
    base: &Document,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    acc_format: wgpu::TextureFormat,
    cpu_refresh: bool,
) -> Result<(), Box<dyn Error>> {
    let mut comp = Compositor::try_new_with_format(device, acc_format)?;
    let mut total_uploads = 0usize;
    let mut total_uploaded_bytes = 0u64;
    let cold_start = Instant::now();
    let mut current = transformed(base, 0)?;
    if cpu_refresh {
        let bounds = current.bounds();
        if let Some(mask) = current.layers.get_mut(0).and_then(|layer| layer.mask.as_mut()) {
            photocraft_compose::mask_paint::refresh(mask, Some(bounds));
        }
    }
    let cold_stats = render(&mut comp, device, queue, &current)?;
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.0;
    total_uploads += cold_stats.mask_fill_uploads;
    total_uploaded_bytes = total_uploaded_bytes.saturating_add(cold_stats.mask_fill_bytes_uploaded);

    let mut samples = Vec::with_capacity(MEASURED_FRAMES);
    let mut refresh_samples = Vec::with_capacity(MEASURED_FRAMES);
    let mut measured_uploads = 0usize;
    let mut measured_uploaded_bytes = 0u64;
    let mut last_stats = Stats::default();
    for frame in 1..(MEASURED_FRAMES + 2) {
        let start = Instant::now();
        current = transformed(base, frame)?;
        let refresh_start = Instant::now();
        if cpu_refresh {
            let bounds = current.bounds();
            if let Some(mask) = current.layers.get_mut(0).and_then(|layer| layer.mask.as_mut()) {
                photocraft_compose::mask_paint::refresh(mask, Some(bounds));
            }
        }
        let refresh_ms = if cpu_refresh { refresh_start.elapsed().as_secs_f64() * 1000.0 } else { 0.0 };
        let stats = render(&mut comp, device, queue, &current)?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        total_uploads += stats.mask_fill_uploads;
        total_uploaded_bytes = total_uploaded_bytes.saturating_add(stats.mask_fill_bytes_uploaded);
        if frame > 1 {
            samples.push(elapsed_ms);
            refresh_samples.push(refresh_ms);
            measured_uploads += stats.mask_fill_uploads;
            measured_uploaded_bytes = measured_uploaded_bytes.saturating_add(stats.mask_fill_bytes_uploaded);
        }
        last_stats = stats;
    }
    samples.sort_by(f64::total_cmp);
    refresh_samples.sort_by(f64::total_cmp);
    println!(
        "path={label} cold={cold_ms:.3} ms total_p50={:.3} ms total_p95={:.3} ms refresh_p50={:.3} ms source_uploads_total={total_uploads} source_bytes_total={total_uploaded_bytes} measured_uploads={measured_uploads} measured_bytes={measured_uploaded_bytes} cold_stats(mask_fill_uploads={}, mask_fill_bytes_uploaded={}) last_stats(cells={}, chunks={}, bytes_uploaded={}, prefix_hits={}, prefix_passes_skipped={}, passes={})",
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&refresh_samples, 0.50),
        cold_stats.mask_fill_uploads,
        cold_stats.mask_fill_bytes_uploaded,
        last_stats.cells,
        last_stats.chunks,
        last_stats.bytes_uploaded,
        last_stats.prefix_hits,
        last_stats.prefix_passes_skipped,
        last_stats.passes,
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let size = requested_size(&args)?;
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let acc_format = Compositor::preferred_acc_format(&adapter);
    println!(
        "adapter={:?} canvas={}x{} fill_source={}x{} repeat=uv transform=move(64,48)+rotate(13deg/frame) measured_frames={} after one warm-up (compositor benchmark only)",
        adapter.get_info(),
        size.width,
        size.height,
        SOURCE_WIDTH,
        SOURCE_HEIGHT,
        MEASURED_FRAMES
    );
    let base = scene(size)?;
    run_mode("cpu_refresh_plus_gpu", &base, &device, &queue, acc_format, true)?;
    run_mode("gpu_transform_only", &base, &device, &queue, acc_format, false)?;
    Ok(())
}
