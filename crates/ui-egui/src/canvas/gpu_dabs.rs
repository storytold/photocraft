//! Brush and Pencil strokes whose dabs the GPU draws in the live canvas (#17133, phase 2).
//!
//! The stroke's thread makes only the dabs (`LiveStroke::begin_gpu`, `push_dabs`); the canvas
//! stamps them into an external mask of the stroke's colour layer in the wgpu compositor, which
//! draws the live document with it, and asks for the mask's 64 px coverage tiles back as they go
//! calm, so that at the release the CPU has the coverage the commit composites
//! (`paint_thread`). The commit is the pixels `paint.stroke` writes, to within the coverage's
//! precision. A stroke the GPU can't draw (see `LiveStroke::gpu_preview`: smoothing, a selection,
//! the eraser, a deeper document…) is a CPU overlay stroke, silently: it gives the same pixels.
//!
//! `PHOTOCRAFT_GPU_DABS=0` keeps every stroke on the CPU. There is no preference for it yet.
//!
//! The external mask belongs to [`GpuUi`], which clears it when dropped: with the live stroke
//! (a cancel, an error, a switch of document, the stroke dropped out of GPU mode) or, after the
//! release, with the pending stroke once its commit has landed.

use std::collections::VecDeque;
use std::sync::Arc;

use photocraft_doc::{DocId, LayerId};
use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_engine::paint::TipGeometry;
use photocraft_gpu::{GpuDab, MaskReadback};
use serde_json::Value;

use super::PhotocraftApp;
use super::paint_thread::{CoverageSource, GpuMode, GpuWork};
use crate::gpu_canvas::GpuCanvas;

/// A dab as the stamping shader takes it.
pub(super) fn gpu_dab(g: &TipGeometry) -> GpuDab {
    GpuDab {
        rect: g.rect,
        cx: g.cx,
        cy: g.cy,
        ro: g.ro,
        core: g.core,
        shoulder: g.shoulder,
        reach2: g.reach2,
        small_reach2: g.small_reach2,
        cs: g.cs,
        sn: g.sn,
        fx: g.fx,
        fy: g.fy,
        proj: g.proj,
        small: g.small,
        flow: g.flow,
        ceil: g.ceil,
    }
}

/// Does the environment allow GPU dabs (`PHOTOCRAFT_GPU_DABS` isn't `0`)? Read once.
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PHOTOCRAFT_GPU_DABS").ok().is_none_or(|v| v.trim() != "0"))
}

/// Coverage tiles asked for, and the epoch they were asked at.
type Asked = (Vec<(i32, i32)>, u64);

/// The canvas's side of a GPU stroke: what only the UI thread can do with it.
pub(super) struct GpuUi {
    gpu: GpuCanvas,
    /// The colour layer whose mask is the GPU's.
    layer: LayerId,
    readback: MaskReadback,
    /// Coverage tiles asked for that the read-back queue had no room for yet, with the epoch they
    /// were asked at. The values they bring are at least as new as that epoch, so a late request
    /// is never wrong, only later (a tile touched again meanwhile is asked for once more).
    backlog: VecDeque<Asked>,
}

/// A GPU stroke that began: the engine's stroke, the canvas's side of it, and the paint thread's.
pub(super) struct Begun {
    pub(super) live: Box<LiveStroke>,
    pub(super) ui: GpuUi,
    pub(super) mode: GpuMode,
}

/// Begin a GPU stroke of `cmd` with `params`, if the stroke and the canvas allow it (`None`: the
/// CPU overlay stroke begins instead). The first dabs are stamped when this returns.
pub(super) fn begin(app: &PhotocraftApp, cmd: &str, params: &Value) -> Option<Begun> {
    // The web build can't block on a read-back.
    if cfg!(target_arch = "wasm32") || !enabled() {
        return None;
    }
    let gpu = app.gpu.as_ref()?;
    let st = app.session.active()?;
    // The canvas draws this document on the GPU compositor now, and its layer pages fit.
    if !app.render_path.gpu_draws(st.doc.id.0) || !gpu.dabs_ready(&st.doc) {
        return None;
    }
    let (live, first) = LiveStroke::begin_gpu(&app.session, cmd, params).ok()?;
    let preview = live.gpu_preview().ok()?;
    gpu.begin_external_mask(preview.doc.id, preview.layer, preview.doc.bounds())?;
    let readback = MaskReadback::new();
    let ui = GpuUi { gpu: gpu.clone(), layer: preview.layer, readback: readback.clone(), backlog: VecDeque::new() };
    // The canvas draws the document before the stroke's first step: the dab must be in by then.
    ui.stamp(&first.iter().map(gpu_dab).collect::<Vec<_>>()).ok()?;
    let source = coverage_source(readback, gpu)?;
    Some(Begun { live: Box::new(live), ui, mode: GpuMode { doc: preview.doc, source } })
}

/// Reads the mask's tiles back and feeds them to the stroke on its thread.
#[cfg(not(target_arch = "wasm32"))]
fn coverage_source(readback: MaskReadback, gpu: &GpuCanvas) -> Option<Arc<dyn CoverageSource>> {
    struct DeviceReadback {
        readback: MaskReadback,
        device: eframe::wgpu::Device,
    }
    impl CoverageSource for DeviceReadback {
        fn poll(&self, block: bool) -> Vec<super::paint_thread::Coverage> {
            self.readback.poll(&self.device, block)
        }

        fn in_flight(&self) -> usize {
            self.readback.in_flight()
        }
    }
    Some(Arc::new(DeviceReadback { readback, device: gpu.device() }))
}

#[cfg(target_arch = "wasm32")]
fn coverage_source(_readback: MaskReadback, _gpu: &GpuCanvas) -> Option<Arc<dyn CoverageSource>> {
    None
}

/// Is the canvas still drawing document `doc` with the GPU compositor? Only it shows a GPU
/// stroke: the CPU canvas draws the live document with an empty mask.
pub(super) fn drawn_on_gpu(gpu: Option<&GpuCanvas>, path: &crate::render_path::RenderPath, doc: DocId) -> bool {
    gpu.is_some_and(|g| g.health().is_ok()) && path.gpu_draws(doc.0)
}

impl GpuUi {
    /// Do `work`, in order: stamp the dabs, and queue the tile requests, which then go to the
    /// read-back as far as it has room (all of them with `all`, at the release). `Err` with the
    /// reason when the GPU can't do it: the stroke then can't be shown or committed from its dabs.
    pub(super) fn run(&mut self, work: Vec<GpuWork>, all: bool) -> Result<(), String> {
        for item in work {
            match item {
                GpuWork::Dabs(dabs) => self.stamp(&dabs)?,
                GpuWork::Read(keys, epoch) => {
                    if !keys.is_empty() {
                        self.backlog.push_back((keys, epoch));
                    }
                }
            }
        }
        self.ask(all)
    }

    fn stamp(&self, dabs: &[GpuDab]) -> Result<(), String> {
        self.gpu.stamp_dabs(self.layer, dabs)
    }

    /// Hand the queued tile requests to the read-back, the oldest first. Each is asked for after
    /// the stamps submitted so far, which include those of every push it was queued behind.
    fn ask(&mut self, all: bool) -> Result<(), String> {
        while let Some((keys, epoch)) = self.backlog.front_mut() {
            let Some(taken) = self.gpu.request_mask_tiles(&self.readback, self.layer, keys, *epoch, all) else {
                return Err("the GPU compositor is not available".into());
            };
            keys.drain(..taken);
            if keys.is_empty() {
                self.backlog.pop_front();
            } else {
                // The queue is full: the rest goes with the next pass.
                break;
            }
        }
        Ok(())
    }

    /// The colour layer whose mask the GPU holds (tests).
    #[cfg(test)]
    pub(super) fn layer(&self) -> LayerId {
        self.layer
    }
}

impl Drop for GpuUi {
    fn drop(&mut self) {
        self.gpu.clear_external_mask(self.layer);
    }
}
