//! The wgpu compositor's side of a live stroke whose dabs the GPU draws (#17133): the stroke's
//! coverage lives in an external mask of its colour layer, the canvas stamps dabs into it and asks
//! for tiles of it back (`canvas::gpu_dabs`). The compositor sits in the paint callback's
//! resources, so each call here takes the renderer's lock, as `composite` does.

use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use photocraft_gpu::{Compositor, GpuDab, MaskPrecision, MaskReadback};

use super::{GpuCanvas, Resources, wgpu};

impl GpuCanvas {
    /// Run `f` with the compositor, the device and the queue; `None` when the device is unhealthy
    /// or there is no compositor (not built yet, or released). A panic in `f` (a wgpu internal
    /// error) marks the device lost, as a failed composite does, instead of taking the app down.
    fn with_compositor<R>(&self, f: impl FnOnce(&mut Compositor, &wgpu::Device, &wgpu::Queue) -> R) -> Option<R> {
        if !self.health.is_ok() {
            return None;
        }
        let mut renderer = self.rs.renderer.write();
        let comp = renderer.callback_resources.get_mut::<Resources>()?.compositor.as_mut()?;
        let (device, queue) = (&self.rs.device, &self.rs.queue);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(comp, device, queue))) {
            Ok(r) => Some(r),
            Err(_) => {
                self.health.mark(photocraft_gpu::Fault::Lost("the GPU compositor stopped (internal error)".into()));
                None
            }
        }
    }

    /// The device, for what polls read-backs off the UI thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn device(&self) -> wgpu::Device {
        self.rs.device.clone()
    }

    /// Can a stroke on `doc` have its dabs drawn by the GPU? The wgpu compositor exists and draws
    /// the document (its display texture is there and it fits the budgets), so the live document
    /// with a stroke in an external mask shows on the canvas.
    pub(crate) fn dabs_ready(&self, doc: &photocraft_doc::Document) -> bool {
        if std::env::var_os("PHOTOCRAFT_CPU_COMPOSE").is_some() || !self.health.is_ok() || !self.fits_canvas(doc) {
            return false;
        }
        let renderer = self.rs.renderer.read();
        let Some(res) = renderer.callback_resources.get::<Resources>() else { return false };
        let size = [doc.size.width, doc.size.height];
        res.compositor_failed.is_none()
            && res.docs.get(&doc.id.0).is_some_and(|d| d.size == size)
            && res.layer_fit.as_ref().is_none_or(|f| f.doc != doc.id.0 || f.fits())
            && res.compositor.as_ref().is_some_and(|c| c.supports(doc).is_ok())
    }

    /// Give `layer`'s mask to the compositor, over `canvas` (`Compositor::begin_external_mask`);
    /// the precision it holds coverage in, or `None` without a compositor.
    pub(crate) fn begin_external_mask(&self, doc: DocId, layer: LayerId, canvas: Rect) -> Option<MaskPrecision> {
        self.with_compositor(|comp, device, _| comp.begin_external_mask(device, doc, layer, canvas, None))
    }

    /// Stamp `dabs` into `layer`'s external mask and submit them, ahead of everything submitted
    /// later (a render of the live document, a read-back).
    pub(crate) fn stamp_dabs(&self, layer: LayerId, dabs: &[GpuDab]) -> Result<(), String> {
        if dabs.is_empty() {
            return Ok(());
        }
        let stamped = self.with_compositor(|comp, device, queue| -> Result<(), String> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_stamp_dabs") });
            comp.stamp_dabs(device, queue, &mut encoder, layer, dabs).map_err(|e| e.0)?;
            queue.submit([encoder.finish()]);
            Ok(())
        });
        stamped.unwrap_or_else(|| Err("the GPU compositor is not available".into()))
    }

    /// Stamp the smoothing tail `dabs` over `layer`'s external mask as a preview, after taking back
    /// the previous tail (`Compositor::stamp_tail`); returns the tiles it saved. Submitted ahead
    /// of everything submitted later, like the stamps.
    pub(crate) fn stamp_tail(&self, layer: LayerId, dabs: &[GpuDab]) -> Result<usize, String> {
        let done = self.with_compositor(|comp, device, queue| -> Result<usize, String> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_stamp_tail") });
            let saved = comp.stamp_tail(device, queue, &mut encoder, layer, dabs).map_err(|e| e.0)?;
            queue.submit([encoder.finish()]);
            Ok(saved)
        });
        done.unwrap_or_else(|| Err("the GPU compositor is not available".into()))
    }

    /// Take back `layer`'s smoothing tail (`Compositor::restore_tail`), submitted like a stamp.
    pub(crate) fn restore_tail(&self, layer: LayerId) -> Result<(), String> {
        let done = self.with_compositor(|comp, device, queue| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_restore_tail") });
            comp.restore_tail(&mut encoder, layer);
            queue.submit([encoder.finish()]);
        });
        done.ok_or_else(|| "the GPU compositor is not available".to_string())
    }

    /// Ask `readback` for coverage tiles of `layer`'s external mask as of the stamps submitted so
    /// far (`MaskReadback::request`, or `request_all` with `all`, which has no cap on what is in
    /// flight). Returns how many of `keys` it took, from the front; `None` without a compositor.
    pub(crate) fn request_mask_tiles(&self, readback: &MaskReadback, layer: LayerId, keys: &[(i32, i32)], tag: u64, all: bool) -> Option<usize> {
        self.with_compositor(|comp, device, queue| {
            if all {
                readback.request_all(comp, device, queue, layer, keys, tag);
                keys.len()
            } else {
                readback.request(comp, device, queue, layer, keys, tag)
            }
        })
    }

    /// Drop `layer`'s external mask: the compositor binds the document's mask again.
    pub(crate) fn clear_external_mask(&self, layer: LayerId) {
        // Not gated on the device's health: dropping the mask is bookkeeping and frees its textures
        // (the compositor may be gone with the device or `release`, and its masks with it).
        if let Some(comp) = self.rs.renderer.write().callback_resources.get_mut::<Resources>().and_then(|res| res.compositor.as_mut()) {
            comp.clear_external_mask(layer);
        }
    }

    /// Whether `layer` has an external mask (tests).
    #[cfg(test)]
    pub(crate) fn has_external_mask(&self, layer: LayerId) -> bool {
        let renderer = self.rs.renderer.read();
        renderer.callback_resources.get::<Resources>().and_then(|res| res.compositor.as_ref()).is_some_and(|c| c.has_external_mask(layer))
    }
}
