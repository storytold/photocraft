//! 4096² ordinary raster-layer mask versus no mask under a small brush damage rect.
//!
//! Compares compositor full-document and dirty-rect updates after identical 128² edits,
//! then reads the edited area back to verify full/dirty and reveal-all/no-mask equivalence.
//! This is a GPU compositor diagnostic, not an end-to-end UI or 3D latency benchmark.
use std::{error::Error, time::Instant};

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_gpu::{Compositor, Stats};
use photocraft_raster::Surface;

const SIDE: u32 = 4096;
const BRUSH_SIDE: u32 = 128;
const FRAMES: usize = 6;
fn brush_rect() -> Rect {
    Rect::from_xywh(1984, 1984, BRUSH_SIDE, BRUSH_SIDE)
}

fn document(masked: bool) -> Document {
    let size = Size::new(SIDE, SIDE);
    let mut doc = Document::with_background("mask stroke diagnostic", size, ColorMode::Rgb, SampleType::U8, Color::rgb(0.24, 0.28, 0.32));
    let mut top = Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]);
    top.fill_rect(doc.bounds(), &[0.46, 0.41, 0.36, 1.0]);
    let mut layer = Layer::new("Paint", LayerContent::Raster(top));
    if masked {
        layer.mask = Some(LayerMask::reveal_all());
    }
    doc.layers.push(layer);
    doc
}

fn edited(base: &Document, frame: usize) -> Result<Document, Box<dyn Error>> {
    let mut doc = base.clone();
    let Some(layer) = doc.layers.last_mut() else { return Err("missing paint layer".into()) };
    let Some(surface) = layer.surface_mut() else { return Err("paint layer is not raster".into()) };
    let color = [0.2 + frame as f32 * 0.08, 0.6, 0.35, 0.8];
    let pixels: Vec<f32> = (0..BRUSH_SIDE * BRUSH_SIDE).flat_map(|_| color).collect();
    surface.write_region(brush_rect(), &pixels);
    Ok(doc)
}

fn poll(device: &wgpu::Device) -> Result<(), Box<dyn Error>> {
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;
    Ok(())
}

fn percentile(values: &[f64], p: f64) -> f64 {
    let rank = ((values.len() as f64 * p).ceil() as usize).saturating_sub(1).min(values.len().saturating_sub(1));
    values.get(rank).copied().unwrap_or_default()
}

struct ResultSet {
    full_pixels: Vec<[f32; 4]>,
    dirty_pixels: Vec<[f32; 4]>,
}

type RenderCapture = (Vec<[f32; 4]>, Stats);

fn render_capture(
    comp: &mut Compositor,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    doc: &Document,
    render_region: Rect,
    capture_region: Rect,
) -> Result<RenderCapture, Box<dyn Error>> {
    let mut bytes_per_pixel = 8u32;
    let mut copied = false;
    let mut row = 0u32;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mask stroke diagnostic readback"),
        size: u64::from(capture_region.width()) * u64::from(capture_region.height()) * 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let stats = comp.render(device, queue, doc, render_region, |encoder, out| {
        let overlap = out.rect.intersect(&capture_region);
        if overlap.width() == 0 || overlap.height() == 0 {
            return;
        }
        bytes_per_pixel = if out.texture.format() == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        row = (capture_region.width() * bytes_per_pixel).div_ceil(256) * 256;
        let offset = u64::from((overlap.y0 - capture_region.y0) as u32) * u64::from(row)
            + u64::from((overlap.x0 - capture_region.x0) as u32) * u64::from(bytes_per_pixel);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: out.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: out.origin[0] + (overlap.x0 - out.rect.x0) as u32, y: out.origin[1] + (overlap.y0 - out.rect.y0) as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(row), rows_per_image: Some(capture_region.height()) },
            },
            wgpu::Extent3d { width: overlap.width(), height: overlap.height(), depth_or_array_layers: 1 },
        );
        copied = true;
    })?;
    if !copied {
        return Err("compositor emitted no pixels for the capture region".into());
    }
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    poll(device)?;
    receive.recv()??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let mut pixels = Vec::with_capacity(capture_region.width() as usize * capture_region.height() as usize);
    for y in 0..capture_region.height() as usize {
        for x in 0..capture_region.width() as usize {
            let offset = y * row as usize + x * bytes_per_pixel as usize;
            let px = if bytes_per_pixel == 16 {
                std::array::from_fn(|i| {
                    f32::from_le_bytes([mapped[offset + i * 4], mapped[offset + i * 4 + 1], mapped[offset + i * 4 + 2], mapped[offset + i * 4 + 3]])
                })
            } else {
                std::array::from_fn(|i| half::f16::from_le_bytes([mapped[offset + i * 2], mapped[offset + i * 2 + 1]]).to_f32())
            };
            pixels.push(px);
        }
    }
    drop(mapped);
    buffer.unmap();
    Ok((pixels, stats))
}

fn run(masked: bool, device: &wgpu::Device, queue: &wgpu::Queue, acc_format: wgpu::TextureFormat) -> Result<ResultSet, Box<dyn Error>> {
    let base = document(masked);
    let mut full = Compositor::try_new_with_format(device, acc_format)?;
    let mut dirty = Compositor::try_new_with_format(device, acc_format)?;
    full.render(device, queue, &base, base.bounds(), |_, _| {})?;
    dirty.render(device, queue, &base, base.bounds(), |_, _| {})?;
    poll(device)?;

    let mut full_ms = Vec::with_capacity(FRAMES);
    let mut dirty_ms = Vec::with_capacity(FRAMES);
    let mut final_doc = None;
    let mut full_stats = Default::default();
    let mut dirty_stats = Default::default();
    let mut full_pixels = Vec::new();
    let mut dirty_pixels = Vec::new();
    for frame in 0..FRAMES {
        let current = edited(&base, frame)?;
        let start = Instant::now();
        if frame + 1 == FRAMES {
            (full_pixels, full_stats) = render_capture(&mut full, device, queue, &current, current.bounds(), brush_rect())?;
        } else {
            full_stats = full.render(device, queue, &current, current.bounds(), |_, _| {})?;
            poll(device)?;
        }
        full_ms.push(start.elapsed().as_secs_f64() * 1000.0);

        let start = Instant::now();
        if frame + 1 == FRAMES {
            (dirty_pixels, dirty_stats) = render_capture(&mut dirty, device, queue, &current, brush_rect(), brush_rect())?;
        } else {
            dirty_stats = dirty.render(device, queue, &current, brush_rect(), |_, _| {})?;
            poll(device)?;
        }
        dirty_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        final_doc = Some(current);
    }
    let _final_doc = final_doc.ok_or("no measured frames")?;
    full_ms.sort_by(f64::total_cmp);
    dirty_ms.sort_by(f64::total_cmp);
    println!(
        "mask={} full_p50_ms={:.3} full_p95_ms={:.3} dirty128_p50_ms={:.3} dirty128_p95_ms={:.3} full_stats(passes={},chunks={},cells={},bytes_uploaded={}) dirty_stats(passes={},chunks={},cells={},bytes_uploaded={})",
        if masked { "reveal_all" } else { "none" },
        percentile(&full_ms, 0.5),
        percentile(&full_ms, 0.95),
        percentile(&dirty_ms, 0.5),
        percentile(&dirty_ms, 0.95),
        full_stats.passes,
        full_stats.chunks,
        full_stats.cells,
        full_stats.bytes_uploaded,
        dirty_stats.passes,
        dirty_stats.chunks,
        dirty_stats.cells,
        dirty_stats.bytes_uploaded,
    );
    Ok(ResultSet { full_pixels, dirty_pixels })
}

fn max_difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> Result<f32, Box<dyn Error>> {
    if a.len() != b.len() {
        return Err(format!("pixel count differs: {} vs {}", a.len(), b.len()).into());
    }
    Ok(a.iter().zip(b).flat_map(|(x, y)| x.iter().zip(y).map(|(x, y)| (x - y).abs())).fold(0.0f32, f32::max))
}

fn main() -> Result<(), Box<dyn Error>> {
    let backend = match std::env::var("PHOTOCRAFT_MASK_BENCH_BACKEND").ok().as_deref() {
        Some("vulkan") => wgpu::Backends::VULKAN,
        Some("dx12") => wgpu::Backends::DX12,
        None => wgpu::Backends::PRIMARY,
        Some(other) => return Err(format!("unsupported PHOTOCRAFT_MASK_BENCH_BACKEND: {other}").into()),
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: backend, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let acc_format = Compositor::preferred_acc_format(&adapter);
    println!("adapter={:?} backend={backend:?} document={}x{} layers=2 brush={}x{} frames={}", adapter.get_info(), SIDE, SIDE, BRUSH_SIDE, BRUSH_SIDE, FRAMES);
    let plain = run(false, &device, &queue, acc_format)?;
    let masked = run(true, &device, &queue, acc_format)?;
    let plain_delta = max_difference(&plain.full_pixels, &plain.dirty_pixels)?;
    let mask_delta = max_difference(&masked.full_pixels, &masked.dirty_pixels)?;
    let identity_delta = max_difference(&plain.full_pixels, &masked.full_pixels)?;
    println!("edited_region_max_delta full_vs_dirty(no_mask)={plain_delta:.8} full_vs_dirty(mask)={mask_delta:.8} no_mask_vs_reveal_all={identity_delta:.8}");
    if plain_delta > 1.0 / 255.0 || mask_delta > 1.0 / 255.0 || identity_delta > 1.0 / 255.0 {
        return Err("full/dirty or reveal-all identity pixel difference exceeds 1/255".into());
    }
    Ok(())
}
