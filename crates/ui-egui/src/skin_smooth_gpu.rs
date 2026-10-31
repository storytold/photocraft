//! Native Skin Smooth accelerator, shared by background jobs and preview sessions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::wgpu;
use photocraft_gpu::{DeviceHealth, SkinSmooth, SkinSmoothParams};
use photocraft_plugins::acceleration::{Accelerator, Band};
use photocraft_plugins::{Error, Result};
use photocraft_raster::Interrupt;
use serde_json::Value;

pub(crate) fn create(rs: &eframe::egui_wgpu::RenderState, health: DeviceHealth) -> Option<Arc<dyn Accelerator>> {
    // A software adapter consumes CPU rather than accelerating the filter.
    if rs.adapter.get_info().device_type == wgpu::DeviceType::Cpu || rs.device.limits().max_storage_buffers_per_shader_stage < 5 {
        return None;
    }
    Some(Arc::new(Backend {
        device: rs.device.clone(),
        queue: rs.queue.clone(),
        health,
        pass: Mutex::new(None),
        failed: AtomicBool::new(false),
        reported: AtomicBool::new(false),
    }))
}

struct Backend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    health: DeviceHealth,
    pass: Mutex<Option<SkinSmooth>>,
    failed: AtomicBool,
    reported: AtomicBool,
}

fn settings(p: &Value) -> Option<SkinSmoothParams> {
    let n = |key| p.get(key).and_then(Value::as_f64).map(|v| v as f32);
    Some(SkinSmoothParams {
        fine: n("fine")?,
        medium: n("medium")?,
        large: n("large")?,
        detail: n("detail")?,
        brightness: p.get("brightness")?.as_f64()?,
        warmth: n("warmth")?,
        redness: n("redness")?,
        tolerance: n("tolerance")?,
        skin_only: p.get("skinOnly")?.as_bool()?,
        positive: p.get("positiveColors").map_or(Some(Vec::new()), |v| serde_json::from_value(v.clone()).ok())?,
        negative: p.get("negativeColors").map_or(Some(Vec::new()), |v| serde_json::from_value(v.clone()).ok())?,
    })
}

// Same verified-edge trimming as the WASM algorithm; off-canvas texture is retained.
fn bounds(px: &[[f32; 4]], w: usize, h: usize, p: &Value) -> (usize, usize, usize, usize) {
    let Some(g) = p.get("_image") else { return (0, 0, w, h) };
    let (Some(x), Some(y), Some(cw), Some(ch), Some(ov)) =
        (g["x"].as_i64(), g["y"].as_i64(), g["canvasWidth"].as_i64(), g["canvasHeight"].as_i64(), g["overlap"].as_u64())
    else {
        return (0, 0, w, h);
    };
    let ov = (ov as usize).min(w / 2).min(h / 2);
    let same_col = |a, b| (0..h).all(|r| px.get(r * w + a) == px.get(r * w + b));
    let same_row = |a, b| px.get(a * w..(a + 1) * w) == px.get(b * w..(b + 1) * w);
    let (l, t) = (x.saturating_neg().clamp(0, ov as i64) as usize, y.saturating_neg().clamp(0, ov as i64) as usize);
    let (r, b) = (cw.saturating_sub(x).clamp((w - ov) as i64, w as i64) as usize, ch.saturating_sub(y).clamp((h - ov) as i64, h as i64) as usize);
    (
        if (0..l).all(|i| same_col(i, l)) { l } else { 0 },
        if (0..t).all(|i| same_row(i, t)) { t } else { 0 },
        if (r..w).all(|i| same_col(i, r.saturating_sub(1))) { r } else { w },
        if (b..h).all(|i| same_row(i, b.saturating_sub(1))) { b } else { h },
    )
}

impl Accelerator for Backend {
    fn algorithm(&self) -> &str {
        "skinsmooth.v2"
    }

    fn filter(&self, band: Band<'_>, ctl: &Interrupt<'_>) -> Result<bool> {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        if self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
            return Ok(false);
        }
        let (w, h) = (band.size[0] as usize, band.size[1] as usize);
        let alpha = band.format & 256 != 0;
        if band.format >> 16 & 255 != 3
            || ![8, 16, 32].contains(&(band.format & 255))
            || band.channels != if alpha { 4 } else { 3 }
            || w == 0
            || h == 0
            || w.checked_mul(h).is_none_or(|n| n > 1_048_576 || n.checked_mul(band.channels) != Some(band.pixels.len()))
            || band.pixels.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Ok(false);
        }
        let Some(p) = settings(band.params) else { return Ok(false) };
        let expanded: Vec<[f32; 4]>;
        let rgba = if alpha {
            band.pixels.as_chunks::<4>().0
        } else {
            expanded = band.pixels.chunks_exact(3).map(|p| [p[0], p[1], p[2], 1.0]).collect();
            expanded.as_slice()
        };
        let (left, top, right, bottom) = bounds(rgba, w, h, band.params);
        if right <= left || bottom <= top {
            return Ok(false);
        }
        let packed: Vec<[f32; 4]>;
        let pixels = if left == 0 && right == w {
            rgba.get(top * w..bottom * w).unwrap_or(&[])
        } else {
            packed = (top..bottom).flat_map(|y| rgba.get(y * w + left..y * w + right).unwrap_or(&[]).iter().copied()).collect();
            packed.as_slice()
        };
        let mut lock = loop {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            if self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
                return Ok(false);
            }
            match self.pass.try_lock() {
                Ok(lock) => break lock,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        };
        // Another invocation may have retired the pass while this one waited for its lock.
        if self.failed.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if lock.is_none() {
                *lock = Some(SkinSmooth::new(&self.device)?);
            }
            lock.as_mut().ok_or_else(|| photocraft_gpu::Unsupported("Skin Smooth pass unavailable".into()))?.filter(
                &self.device,
                &self.queue,
                &self.health,
                pixels,
                [(right - left) as u32, (bottom - top) as u32],
                p,
                ctl,
            )
        }))
        .unwrap_or_else(|_| Err(photocraft_gpu::Unsupported("Skin Smooth GPU runtime failed".into())));
        if result.is_err() {
            // Submitted GPU work cannot be cancelled. Retire its resources instead of reusing
            // a possibly still-mapped buffer; wgpu retains in-flight resources until completion.
            self.failed.store(true, Ordering::Relaxed);
            *lock = None;
        }
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let out = match result {
            Ok(out) => out,
            Err(error) => {
                log::warn!("{error}; Skin Smooth uses WASM");
                return Ok(false);
            }
        };
        if !self.reported.swap(true, Ordering::Relaxed) {
            log::info!("Skin Smooth GPU compute active");
        }
        let width = right - left;
        for (y, row) in out.chunks_exact(width).enumerate() {
            if alpha {
                let start = ((y + top) * w + left) * 4;
                if let Some(dst) = band.pixels.get_mut(start..start + width * 4) {
                    dst.copy_from_slice(row.as_flattened());
                }
                continue;
            }
            for (x, p) in row.iter().enumerate() {
                let i = ((y + top) * w + x + left) * band.channels;
                if let Some(dst) = band.pixels.get_mut(i..i + 3) {
                    dst.copy_from_slice(&p[..3]);
                }
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
