use std::fmt;
use std::time::Duration;

use super::textures::Stats;
use super::{Compositor, Document, Rect, Unsupported};

/// Errors encountered during GPU readback.
#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error("unsupported document or feature: {0}")]
    Unsupported(String),
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    #[error("buffer allocation failed: {0}")]
    AllocationFailed(String),
    #[error("buffer mapping failed: {0}")]
    MappingFailed(String),
    #[error("rendering was cancelled")]
    Cancelled,
    #[error("readback failed: {0}")]
    Execution(String),
}

impl From<Unsupported> for ReadbackError {
    fn from(e: Unsupported) -> Self {
        ReadbackError::Unsupported(e.0)
    }
}

/// Default maximum bytes of staging buffer memory in flight during band readback (32 MiB).
pub const DEFAULT_MAX_STAGING_BYTES: usize = 32 << 20;

/// Blocking readback of a composite (native tests and tools).
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<Vec<[f32; 4]>, Unsupported> {
    render_to_vec_stats(comp, device, queue, doc, rect).map(|(v, _)| v)
}

/// [`render_to_vec`] plus the render's [`Stats`].
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec_stats(
    comp: &mut Compositor,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    doc: &Document,
    rect: Rect,
) -> Result<(Vec<[f32; 4]>, Stats), Unsupported> {
    let rect = rect.intersect(&doc.bounds());
    let mut out = vec![[0.0f32; 4]; rect.width() as usize * rect.height() as usize];
    let w = rect.width() as usize;

    render_bands(comp, device, queue, doc, rect, 0, DEFAULT_MAX_STAGING_BYTES, |band_rect, band_pixels| {
        let bw = band_rect.width() as usize;
        for by in 0..band_rect.height() as usize {
            let dst_y = (band_rect.y0 - rect.y0) as usize + by;
            let dst_row = dst_y * w + (band_rect.x0 - rect.x0) as usize;
            let src_row = by * bw;
            out[dst_row..dst_row + bw].copy_from_slice(&band_pixels[src_row..src_row + bw]);
        }
        Ok::<(), std::convert::Infallible>(())
    })
    .map_err(|e| match e {
        ReadbackError::Unsupported(s) => Unsupported(s),
        other => Unsupported(other.to_string()),
    })?;

    Ok((out, Stats::default()))
}

/// Readback of a composite in bounded horizontal bands.
///
/// Binds staging buffer memory to at most `max_staging_bytes`, mapping and transferring
/// one band of rows at a time, so arbitrarily large documents can be exported without
/// unbounded staging allocation.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_bands<E>(
    comp: &mut Compositor,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    doc: &Document,
    rect: Rect,
    band_rows: i32,
    max_staging_bytes: usize,
    mut sink: impl FnMut(Rect, Vec<[f32; 4]>) -> Result<(), E>,
) -> Result<(), ReadbackError>
where
    E: fmt::Display,
{
    let rect = rect.intersect(&doc.bounds());
    if rect.is_empty() {
        return Ok(());
    }

    let bpp = if comp.acc_format() == wgpu::TextureFormat::Rgba32Float { 16u32 } else { 8u32 };
    let row_bytes = (rect.width() * bpp).div_ceil(256) * 256;
    let max_staging = max_staging_bytes.max(256 * 1024);

    let want_rows = if band_rows > 0 { band_rows as u32 } else { (max_staging / row_bytes.max(1) as usize).max(1) as u32 };
    let step_rows = want_rows.min((max_staging / row_bytes.max(1) as usize).max(1) as u32).min(rect.height());

    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = (y as i64 + step_rows as i64).min(rect.y1 as i64) as i32;
        let band_rect = Rect::new(rect.x0, y, rect.x1, y1);

        if let Some(fault) = comp.fault() {
            return Err(ReadbackError::DeviceLost(fault.to_string()));
        }

        let mut staging: Vec<(Rect, wgpu::Buffer, u32)> = Vec::new();
        comp.render(device, queue, doc, band_rect, |_enc, out| {
            let chunk_bpp = if out.texture.format() == wgpu::TextureFormat::Rgba32Float { 16u32 } else { 8u32 };
            let chunk_row = (out.rect.width() * chunk_bpp).div_ceil(256) * 256;
            let size = (chunk_row * out.rect.height()) as u64;
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_bounded_readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            _enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(chunk_row), rows_per_image: Some(out.rect.height()) },
                },
                wgpu::Extent3d { width: out.rect.width(), height: out.rect.height(), depth_or_array_layers: 1 },
            );
            staging.push((out.rect, buf, chunk_row));
        })?;

        for (_, b, _) in &staging {
            b.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }

        match comp.health() {
            Some(h) if !h.wait(device, Some(Duration::from_secs(30))) => {
                return Err(ReadbackError::DeviceLost(h.fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string())));
            }
            Some(_) => {}
            None => {
                let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(30)) });
            }
        }

        if let Some(fault) = comp.fault() {
            return Err(ReadbackError::DeviceLost(fault.to_string()));
        }

        let band_w = band_rect.width() as usize;
        let band_h = band_rect.height() as usize;
        let mut band_pixels = vec![[0.0f32; 4]; band_w * band_h];

        for (r, b, chunk_row) in &staging {
            let data = b.slice(..).get_mapped_range().map_err(|e| ReadbackError::MappingFailed(format!("GPU readback map failed: {e:?}")))?;
            let chunk_bpp = bpp as usize;
            for cy in 0..r.height() as usize {
                for cx in 0..r.width() as usize {
                    let offset = cy * *chunk_row as usize + cx * chunk_bpp;
                    let px: [f32; 4] = if chunk_bpp == 16 {
                        std::array::from_fn(|i| {
                            f32::from_le_bytes([data[offset + i * 4], data[offset + i * 4 + 1], data[offset + i * 4 + 2], data[offset + i * 4 + 3]])
                        })
                    } else {
                        std::array::from_fn(|i| half::f16::from_le_bytes([data[offset + i * 2], data[offset + i * 2 + 1]]).to_f32())
                    };
                    let dx = (r.x0 - band_rect.x0) as usize + cx;
                    let dy = (r.y0 - band_rect.y0) as usize + cy;
                    band_pixels[dy * band_w + dx] = px;
                }
            }
            drop(data);
            b.unmap();
        }
        drop(staging);

        sink(band_rect, band_pixels).map_err(|e| ReadbackError::Execution(e.to_string()))?;
        y = y1;
    }
    Ok(())
}
