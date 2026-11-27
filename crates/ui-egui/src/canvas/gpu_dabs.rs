//! Brush and Pencil strokes whose dabs the GPU draws in the live canvas (#17133, phase 2).
//!
//! The stroke's thread makes only the dabs (`LiveStroke::begin_gpu`, `push_dabs`); the canvas
//! stamps them into an external mask of the stroke's colour layer in the wgpu compositor, which
//! draws the live document with it, and asks for the mask's 64 px coverage tiles back as they go
//! calm, so that at the release the CPU has the coverage the commit composites
//! (`paint_thread`). The commit is the pixels `paint.stroke` writes, to within the coverage's
//! precision. A stroke the GPU can't draw (see `LiveStroke::gpu_preview`: a selection, the eraser,
//! a deeper document…) is a CPU overlay stroke, silently: it gives the same pixels.
//!
//! A smoothed stroke lags the pen, and the live document also shows its tail: what ending the
//! stroke would add. The GPU stamps the tail over the settled coverage as a preview, saving the
//! mask's tiles under it first and restoring them before the next step (`Compositor::stamp_tail`),
//! and no read-back may ever see it (`GpuUi::run`).
//!
//! `PHOTOCRAFT_GPU_DABS=0` keeps every stroke on the CPU. There is no preference for it yet.
//!
//! The external mask belongs to [`GpuUi`], which clears it when dropped: with the live stroke
//! (a cancel, an error, a switch of document, the stroke dropped out of GPU mode) or, after the
//! release, with the pending stroke once its commit has landed.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use photocraft_doc::{DocId, LayerId};
use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_engine::paint::TipGeometry;
use photocraft_gpu::{GpuDab, MaskReadback};

/// Side of the coverage tiles a stroke is read back in.
const TILE: i32 = 64;
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
    /// The 64 px tiles under the smoothing tail now shown over the mask (empty: no tail).
    tail_tiles: HashSet<(i32, i32)>,
    /// Tests: tail tiles saved so far, and tails stamped.
    #[cfg(test)]
    tail_saved: u64,
    #[cfg(test)]
    tails_stamped: u64,
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
    let (mut live, first) = LiveStroke::begin_gpu(&app.session, cmd, params).ok()?;
    let preview = live.gpu_preview().ok()?;
    gpu.begin_external_mask(preview.doc.id, preview.layer, preview.doc.bounds())?;
    let readback = MaskReadback::new();
    let mut ui = GpuUi {
        gpu: gpu.clone(),
        layer: preview.layer,
        readback: readback.clone(),
        backlog: VecDeque::new(),
        tail_tiles: HashSet::new(),
        #[cfg(test)]
        tail_saved: 0,
        #[cfg(test)]
        tails_stamped: 0,
    };
    // The canvas draws the document before the stroke's first step: the dab must be in by then.
    ui.stamp(&first.iter().map(gpu_dab).collect::<Vec<_>>()).ok()?;
    // A lone first dab is the stroke's tail until the pen moves.
    ui.show_tail(&live.tail_dabs().0.iter().map(gpu_dab).collect::<Vec<_>>()).ok()?;
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
    /// Do `work`, in order: stamp the dabs, queue the tile requests and hand them to the read-back
    /// as far as it has room (all of them with `all`, at the release), show the tail. `Err` with
    /// the reason when the GPU can't do it: the stroke then can't be shown or committed from its
    /// dabs.
    ///
    /// No read-back may ever see a smoothing tail: the tail is a preview, the commit is made from
    /// the settled coverage and the dabs of the release. Three rules keep it so.
    /// - A `Dabs` takes the shown tail back before it stamps (even an empty one, a request may
    ///   follow), so settled dabs never land on a tail and the mask is the settled coverage again.
    /// - Each step is `Dabs`, `Read`, `Tail`, and a `Read` is asked for as it comes, so its copies
    ///   are queued after the settled stamps and before the new tail's.
    /// - A request that waits in the backlog (the read-back was full) is asked for later, when a
    ///   tail may be shown: `ask` holds back the tiles under it until a pass that finds them clear.
    ///
    /// A `Tail` that a later `Dabs` or `Tail` of the same batch replaces is never seen by anything
    /// (the next one takes it back before anything is asked), so it is not stamped.
    pub(super) fn run(&mut self, work: Vec<GpuWork>, all: bool) -> Result<(), String> {
        let last_stamp = work.iter().rposition(|w| matches!(w, GpuWork::Dabs(_) | GpuWork::Tail(_)));
        for (i, item) in work.into_iter().enumerate() {
            match item {
                GpuWork::Dabs(dabs) => {
                    self.take_tail_back()?;
                    self.stamp(&dabs)?;
                }
                GpuWork::Read(keys, epoch) => {
                    if !keys.is_empty() {
                        self.backlog.push_back((keys, epoch));
                    }
                    self.ask(all)?;
                }
                GpuWork::Tail(dabs) if Some(i) == last_stamp => self.show_tail(&dabs)?,
                GpuWork::Tail(_) => {}
            }
        }
        self.ask(all)
    }

    fn stamp(&self, dabs: &[GpuDab]) -> Result<(), String> {
        self.gpu.stamp_dabs(self.layer, dabs)
    }

    /// Take back the tail shown over the mask, if any.
    fn take_tail_back(&mut self) -> Result<(), String> {
        if self.tail_tiles.is_empty() {
            return Ok(());
        }
        self.tail_tiles.clear();
        self.gpu.restore_tail(self.layer)
    }

    /// Show `dabs` as the tail, in place of the previous one.
    fn show_tail(&mut self, dabs: &[GpuDab]) -> Result<(), String> {
        if dabs.is_empty() {
            return self.take_tail_back();
        }
        self.tail_tiles.clear();
        for d in dabs {
            for ty in d.rect.y0.div_euclid(TILE)..=(d.rect.y1 - 1).div_euclid(TILE) {
                for tx in d.rect.x0.div_euclid(TILE)..=(d.rect.x1 - 1).div_euclid(TILE) {
                    self.tail_tiles.insert((tx, ty));
                }
            }
        }
        match self.gpu.stamp_tail(self.layer, dabs) {
            Ok(saved) => {
                self.note_tail(saved);
                Ok(())
            }
            Err(e) => {
                // Unknown which tiles are under a half-made tail: the stroke stops here.
                self.tail_tiles.clear();
                Err(e)
            }
        }
    }

    /// Hand the queued tile requests to the read-back, the oldest first, except the tiles under
    /// the tail now shown, which stay queued for a later pass (see [`run`](Self::run)). Each is
    /// asked for after the stamps submitted so far, which include those of every push it was
    /// queued behind.
    fn ask(&mut self, all: bool) -> Result<(), String> {
        let mut kept = VecDeque::new();
        let mut full = false;
        while let Some((keys, epoch)) = self.backlog.pop_front() {
            if full {
                kept.push_back((keys, epoch));
                continue;
            }
            let (held, mut now): (Vec<_>, Vec<_>) = keys.into_iter().partition(|k| self.tail_tiles.contains(k));
            if !held.is_empty() {
                kept.push_back((held, epoch));
            }
            if now.is_empty() {
                continue;
            }
            let Some(taken) = self.gpu.request_mask_tiles(&self.readback, self.layer, &now, epoch, all) else {
                return Err("the GPU compositor is not available".into());
            };
            now.drain(..taken);
            if !now.is_empty() {
                // The queue is full: the rest goes with the next pass.
                kept.push_back((now, epoch));
                full = true;
            }
        }
        self.backlog = kept;
        Ok(())
    }

    #[cfg(test)]
    fn note_tail(&mut self, saved: usize) {
        self.tail_saved += saved as u64;
        self.tails_stamped += 1;
    }

    #[cfg(not(test))]
    fn note_tail(&mut self, _saved: usize) {}

    /// Tail tiles saved and tails stamped so far (tests).
    #[cfg(test)]
    pub(super) fn tail_stats(&self) -> (u64, u64) {
        (self.tail_saved, self.tails_stamped)
    }

    /// Whether a tail is shown over the mask (tests).
    #[cfg(test)]
    pub(super) fn tail_shown(&self) -> bool {
        !self.tail_tiles.is_empty()
    }

    /// A `GpuUi` over `layer`'s external mask, which the caller has begun (tests).
    #[cfg(test)]
    pub(super) fn for_test(gpu: GpuCanvas, layer: LayerId) -> Self {
        Self { gpu, layer, readback: MaskReadback::new(), backlog: VecDeque::new(), tail_tiles: HashSet::new(), tail_saved: 0, tails_stamped: 0 }
    }

    /// Requests queued that no read-back was asked for yet (tests).
    #[cfg(test)]
    pub(super) fn backlog_len(&self) -> usize {
        self.backlog.len()
    }

    /// Read-backs requested and not yet polled (tests).
    #[cfg(test)]
    pub(super) fn in_flight(&self) -> usize {
        self.readback.in_flight()
    }

    /// Wait for the read-backs asked for so far and return their tiles, by key (tests).
    #[cfg(test)]
    pub(super) fn collect(&self) -> Vec<super::paint_thread::Coverage> {
        let mut out = self.readback.poll(&self.gpu.device(), true);
        out.sort_by_key(|c| c.0);
        out
    }

    /// The tiles `keys` as the mask holds them now, whatever the tail: bypasses the rule that no
    /// read-back sees it, to show what such a read-back would have got (tests).
    #[cfg(test)]
    pub(super) fn read_unchecked(&self, keys: &[(i32, i32)]) -> Vec<super::paint_thread::Coverage> {
        let _ = self.gpu.request_mask_tiles(&self.readback, self.layer, keys, 0, true);
        self.collect()
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
