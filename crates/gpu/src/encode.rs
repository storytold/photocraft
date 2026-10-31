use std::sync::Arc;

use super::batch::{batch_runs, run_starts, submit_batch, tile_grid};
use super::consts::{FLUSH_BYTES, FLUSH_OPS, FLUSH_WORK, PATTERN_IDLE_FRAMES};
use super::convert::lut_texture;
use super::kit::FrameRes;
use super::paging::{OVER_BUDGET, cell_rect, first_error, grid_rect_count, grid_rects, lru_victims, scoped_work, uniform_buffer_len};
use super::textures::{CachedMaskFill, ChunkOut, FxKey, ResKey, Resident, Stats, Tex};
use super::{Compositor, DocId, Document, LayerId, Plan, Rect, Surface, TILE_SIZE, Unsupported, plan, timing};

type PatternCacheLruItem = (u64, Option<String>, Option<(DocId, LayerId, u16)>, u64);

impl Compositor {
    /// Composite `region` of `doc`. Each finished chunk is passed to `sink` while its encoder is
    /// still open; the chunk texture is reused afterwards, so the sink must record any copies
    /// or passes that read it into the given encoder. Submits the work before returning; a
    /// refresh of a huge document also submits in between (the sink may be handed a new
    /// encoder), so its uploads and evicted pages never pile up in memory.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        doc: &Document,
        region: Rect,
        mut sink: impl FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>),
    ) -> Result<Stats, Unsupported> {
        let result = scoped_work(device, self.health.clone(), || {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_compose") });
            if self.timer.as_ref().is_some_and(timing::GpuTimer::busy) {
                // The readback map completes on a poll. The app polls every frame, but a
                // non-blocking poll here keeps the measurements coming whatever the caller does.
                match &self.health {
                    Some(h) => h.poll(device),
                    None => {
                        let _ = device.poll(wgpu::PollType::Poll);
                    }
                }
            }
            // Whatever came back since the last render travels out with this one's stats, so
            // nothing has to reach back into the compositor for it afterwards.
            let landed = self.timer.as_mut().and_then(timing::GpuTimer::take);
            if let Some(t) = &mut self.timer {
                let r = region.intersect(&doc.bounds());
                t.start(&mut encoder, r.width() as u64 * r.height() as u64);
            }
            let mut stats = self.encode_inner(device, queue, &mut encoder, doc, region, &mut sink, true);
            if let Ok(s) = &mut stats {
                s.gpu_time = landed;
            }
            if let Some(t) = &mut self.timer {
                // A refresh that failed part way measures nothing: its spans describe work the
                // caller is about to redo on the CPU.
                match stats {
                    Ok(_) => t.finish(&mut encoder),
                    Err(_) => {
                        t.cancel();
                        // The refresh is lost, but a measurement that came back with it is not:
                        // hand it to the next render rather than drop it on the fallback path.
                        if let Some(landed) = landed {
                            t.restore(landed);
                        }
                    }
                }
            }
            let stats = stats?;
            queue.submit([encoder.finish()]);
            if let Some(t) = &mut self.timer {
                t.submitted();
            }
            Ok(stats)
        });
        if result.is_err() {
            self.discard_failed_work();
        }
        result
    }

    /// Like [`Self::render`] but records into `encoder` without submitting.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &Document,
        region: Rect,
        sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>),
    ) -> Result<Stats, Unsupported> {
        let result = scoped_work(device, self.health.clone(), || self.encode_inner(device, queue, encoder, doc, region, sink, false));
        if result.is_err() {
            self.discard_failed_work();
        }
        result
    }

    fn discard_failed_work(&mut self) {
        // Cache metadata advances while recording. A failed encoder may never execute its
        // texture uploads or effect maps, so none of those cached pixels can be trusted.
        self.prefix.clear();
        self.residents.clear();
        self.fx.clear();
        self.patterns.clear();
        self.mask_fills.clear();
        self.temps.clear();
        self.pool.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &Document,
        region: Rect,
        sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>),
        flush: bool,
    ) -> Result<Stats, Unsupported> {
        if let Some(f) = self.fault() {
            return Err(Unsupported(f.to_string()));
        }
        // CMYK layers convert through the document's own CMYK profile (uploads and plan colours).
        let space = photocraft_compose::cmyk_space(doc);
        self.cmyk = space.as_ref().map_or(0, |s| s.id);
        photocraft_color::convert::with_cmyk_space(space.as_ref(), || self.encode_scoped(device, queue, encoder, doc, region, sink, flush))
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_scoped(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &Document,
        region: Rect,
        sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>),
        flush: bool,
    ) -> Result<Stats, Unsupported> {
        let canvas = doc.bounds();
        let region = region.intersect(&canvas);
        let plan = plan(doc)?;
        self.check_fx(doc, &plan)?;
        self.pattern_mode = doc.pixel_format().mode;
        self.pattern_doc = doc.id;
        let mut stats = Stats { passes: plan.passes.len(), slots: plan.slots, ..Default::default() };
        if region.is_empty() {
            return Ok(stats);
        }
        self.configure_mask_halo(plan.mask_halo)?;
        let page = self.page;
        let mut cells = grid_rects(region, page);
        // Check every actual page cell before uploads, submissions or sink output. A later
        // cell can need more chunk records than the first; a failure must leave the device healthy.
        let buffer_limit = device.limits().max_buffer_size;
        for part in &cells {
            uniform_buffer_len(grid_rect_count(*part, self.chunk)?, plan.passes.len(), buffer_limit)?;
        }
        if self.frame == 0 {
            // The first submission after device creation is dropped on some drivers (RADV),
            // which would render the first frame black; land it on an empty buffer instead.
            queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
        }
        self.frame += 1;
        self.stamp += 1;
        self.staged = 0;
        self.recorded = 0;
        self.layer_alloc = None;
        self.admit_freed = 0;

        // Only render owns submission. A caller may discard encode's command buffer, so it
        // cannot establish (or consume) a checkpoint whose GPU copy might never execute.
        // Runs of plain layers drawn as one pass each (`fs_layerbatch`). The grouping is the same
        // for every render of this plan, so a layer's bits don't depend on the cache: renders
        // capture and resume only where a run starts.
        let runs = batch_runs(&plan);
        let checkpoint = if flush && plan.mask_halo == 0 {
            let starts = run_starts(&runs);
            let resumable: Vec<_> = plan.checkpoints.iter().filter(|c| starts.get(c.pass_end).copied().unwrap_or(false)).copied().collect();
            self.prefix.prepare(doc, &resumable)
        } else {
            None
        };

        // Effect maps of whole regions first (they may render group shapes through sub-plans);
        // regions larger than the limit are built per cell below.
        let paged: Vec<bool> = plan.fx.iter().map(|f| self.fx_paged(f.region)).collect();
        let aprons: Vec<i32> = plan.fx.iter().zip(&paged).map(|(f, &pg)| if pg { Self::fx_apron(doc, f) } else { 0 }).collect();
        for (f, &pg) in plan.fx.iter().zip(&paged) {
            if !pg {
                self.sync_fx(device, queue, encoder, doc, f, (f.layer.id, None), f.region, &mut stats)?;
            }
        }
        let fr = self.frame_res(device, queue, canvas, &plan, &mut stats);

        // Page cells; every other frame in reverse, so a repeated full refresh of a document
        // larger than the budget starts with the pages still resident.
        if self.frame.is_multiple_of(2) {
            cells.reverse();
        }
        if let Some(f) = self.focus {
            let p = page as i32;
            let focused = |r: &Rect| !cell_rect((r.x0.div_euclid(p), r.y0.div_euclid(p)), page).intersect(&f).is_empty();
            cells.sort_by_key(focused);
        }
        let flush_bytes = FLUSH_BYTES.min(self.memory_budget() / 8).max(16 << 20);
        stats.cells = cells.len();
        // The previous intermediate submit (see `submit_batch`).
        let mut last_submit: Option<wgpu::SubmissionIndex> = None;
        // Work recorded and bytes evicted since the last submit.
        let (mut work, mut freed) = (0u64, 0u64);
        for part in cells {
            self.stamp += 1;
            let cell = (part.x0.div_euclid(page as i32), part.y0.div_euclid(page as i32));
            let cr = cell_rect(cell, page);
            for (i, f) in plan.fx.iter().enumerate() {
                let pf = cr.intersect(&f.region);
                if !paged.get(i).copied().unwrap_or(false) || pf.is_empty() {
                    continue;
                }
                let window = pf.inflate(aprons.get(i).copied().unwrap_or(0)).intersect(&f.region);
                self.sync_fx(device, queue, encoder, doc, f, (f.layer.id, Some(cell)), window, &mut stats)?;
            }
            let chunks = grid_rects(part, self.chunk);
            stats.chunks += chunks.len();
            let halo = plan.mask_halo as i32;
            let padded_pixels = chunks.iter().fold(0u64, |total, chunk| {
                let work = chunk.inflate(halo);
                total.saturating_add(u64::from(work.width()) * u64::from(work.height()))
            });
            work = work.saturating_add(padded_pixels.saturating_mul(plan.passes.len() as u64));
            let mut batch = flush.then_some(&mut last_submit);
            self.run_cell(device, queue, encoder, doc.id, &plan, &runs, &paged, cell, &fr, &chunks, checkpoint, &mut stats, sink, &mut batch);
            // The device was lost in a submit inside the cell: issue no more work.
            if let Some(f) = self.fault() {
                return Err(Unsupported(f.to_string()));
            }
            // Out of memory although every page this cell doesn't use was dropped: the work recorded
            // since the last submit can still hold those pages. Submit it, wait until the GPU is done
            // with it so their memory is released, and run the cell once more. `encode` can't
            // submit, so there the refresh goes to the CPU straight away.
            if flush && self.layer_alloc.as_deref() == Some(OVER_BUDGET) {
                self.layer_alloc = None;
                submit_batch(device, queue, encoder, self.health.as_ref(), self.timer.as_mut(), &mut last_submit);
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let idle = match self.health.as_ref() {
                        Some(h) => h.wait_idle(device, last_submit.clone()),
                        None => device.poll(wgpu::PollType::Wait { submission_index: last_submit.clone(), timeout: None }).is_ok(),
                    };
                    // The GPU has run every timestamp submitted so far, so a retry that fails too
                    // can drop this measurement without switching timing off for good.
                    if idle && let Some(t) = self.timer.as_mut() {
                        t.drained();
                    }
                }
                self.staged = 0;
                self.recorded = 0;
                work = 0;
                freed = 0;
                self.admit_freed = 0;
                stats.flushes += 1;
                if let Some(f) = self.fault() {
                    return Err(Unsupported(f.to_string()));
                }
                let mut batch = Some(&mut last_submit);
                self.run_cell(device, queue, encoder, doc.id, &plan, &runs, &paged, cell, &fr, &chunks, checkpoint, &mut stats, sink, &mut batch);
                if let Some(f) = self.fault() {
                    return Err(Unsupported(f.to_string()));
                }
            }
            // A page didn't fit even after evicting: this refresh goes to the CPU, the next one
            // tries the GPU again (#6563).
            if let Some(e) = self.layer_alloc.take() {
                return Err(Unsupported(e));
            }
            let (evicted, bytes) = self.evict();
            stats.evicted += evicted;
            freed = freed.saturating_add(bytes.saturating_add(std::mem::take(&mut self.admit_freed)));
            if flush && (self.staged >= flush_bytes || work >= FLUSH_WORK || freed >= flush_bytes || self.recorded >= FLUSH_OPS) {
                submit_batch(device, queue, encoder, self.health.as_ref(), self.timer.as_mut(), &mut last_submit);
                self.staged = 0;
                self.recorded = 0;
                work = 0;
                freed = 0;
                stats.flushes += 1;
                // The device was lost mid-refresh: issue no more work.
                if let Some(f) = self.fault() {
                    return Err(Unsupported(f.to_string()));
                }
            }
        }

        // Evict this document's textures whose layers are gone (hidden layers stay resident so
        // toggling visibility costs no upload).
        let frame = self.frame;
        let live: std::collections::HashSet<LayerId> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        self.residents.retain(|(id, _, _), r| r.doc != doc.id || r.last_used == frame || live.contains(id));
        self.mask_fills.retain(|(id, layer, _), p| *id != doc.id || p.last_used == frame || live.contains(layer));
        self.fx.retain(|(id, _), e| e.doc != doc.id || e.last_used == frame || live.contains(id));
        stats.evicted += self.evict().0;
        self.temps.retain(|(_, used)| frame.saturating_sub(*used) < 240);
        self.evict_patterns();
        Ok(stats)
    }

    /// Keep padded Blur work areas inside the device's texture limit. Source residents are
    /// tile-aligned, so their margin rounds up to whole source tiles; output work uses the exact
    /// Gaussian support. A smaller page leaves room for that source margin on small adapters.
    fn configure_mask_halo(&mut self, halo: u32) -> Result<(), Unsupported> {
        let tile = TILE_SIZE as u32;
        let source_halo = halo.div_ceil(tile).saturating_mul(tile);
        let source_side = self.max_dim.saturating_sub(source_halo.saturating_mul(2));
        let work_side = self.max_dim.saturating_sub(halo.saturating_mul(2));
        if source_side < tile || work_side < tile {
            return Err(Unsupported(format!("Mask Blur halo {halo}px needs a padded source/work texture larger than the GPU limit {}px", self.max_dim)));
        }
        let requested_page = self.max_dim.min(super::consts::PAGE);
        let page_cap = requested_page.min(source_side);
        let page = 1u32 << (31 - page_cap.leading_zeros());
        let chunk = super::consts::CHUNK.min(page).min(work_side);
        if page != self.page || chunk != self.chunk {
            self.page = page;
            self.chunk = chunk;
            self.prefix.set_step(chunk);
            self.prefix.clear();
            self.pool.clear();
            self.residents.clear();
            self.fx.clear();
            self.temps.clear();
        }
        Ok(())
    }

    /// Drop the pattern textures no frame has painted with for [`PATTERN_IDLE_FRAMES`] frames,
    /// then the least recently painted ones over the pattern budget. Never one this frame painted
    /// with: it would only be uploaded again on the next frame.
    fn evict_patterns(&mut self) {
        let frame = self.frame;
        self.patterns.retain(|_, p| frame.saturating_sub(p.last_used) < PATTERN_IDLE_FRAMES);
        self.mask_fills.retain(|_, p| frame.saturating_sub(p.last_used) < PATTERN_IDLE_FRAMES);
        let mut total = self.pattern_cache_bytes().saturating_add(self.mask_fill_cache_bytes());
        if total > self.pattern_budget {
            let mut lru: Vec<PatternCacheLruItem> = self
                .patterns
                .iter()
                .filter(|(_, p)| p.last_used != frame)
                .map(|(k, p)| (p.last_used, Some(k.clone()), None, p.bytes))
                .chain(self.mask_fills.iter().filter(|(_, p)| p.last_used != frame).map(|(k, p)| (p.last_used, None, Some(*k), p.bytes)))
                .collect();
            lru.sort_unstable_by_key(|item| item.0);
            for (_, pattern, fill, bytes) in lru {
                if total <= self.pattern_budget {
                    break;
                }
                if let Some(key) = pattern {
                    self.patterns.remove(&key);
                }
                if let Some(key) = fill {
                    self.mask_fills.remove(&key);
                }
                total = total.saturating_sub(bytes);
            }
        }
    }

    /// Keep resident pages within the resident budget and the effect cache within
    /// its budget, least recently used first. Never drops what the current cell uses, nor
    /// whole-region effect maps drawn this frame. Returns how many textures were dropped and
    /// their bytes.
    fn evict(&mut self) -> (usize, u64) {
        let (mut n, mut freed) = self.evict_residents(self.resident_budget, 0, None);
        let mut total = self.fx_cache_bytes();
        let fx_budget = usize::try_from(self.fx_budget).unwrap_or(usize::MAX);
        if total > fx_budget {
            let (frame, stamp) = (self.frame, self.stamp);
            let mut lru: Vec<((u64, u64), FxKey)> = self
                .fx
                .iter()
                .filter(|(k, e)| e.last_used != frame || (k.1.is_some() && e.stamp != stamp))
                .map(|(k, e)| ((e.last_used, e.stamp), *k))
                .collect();
            lru.sort_unstable_by_key(|c| c.0);
            for (_, k) in lru {
                if total <= fx_budget {
                    break;
                }
                if let Some(e) = self.fx.remove(&k) {
                    total = total.saturating_sub(e.bytes());
                    n += 1;
                    freed += e.bytes() as u64;
                }
            }
        }
        (n, freed)
    }

    /// Drop least recently used resident pages until `incoming` more bytes fit in `budget`.
    /// Never drops what the current cell uses, nor `keep`. Returns how many pages were dropped
    /// and their bytes.
    fn evict_residents(&mut self, budget: u64, incoming: u64, keep: Option<ResKey>) -> (usize, u64) {
        let stamp = self.stamp;
        let pages: Vec<(u64, ResKey, u64)> =
            self.residents.iter().filter(|(k, r)| r.stamp != stamp && Some(**k) != keep).map(|(k, r)| (r.stamp, *k, r.bytes())).collect();
        let (mut n, mut freed) = (0, 0u64);
        for k in lru_victims(pages, self.resident_bytes(), incoming, budget) {
            if let Some(r) = self.residents.remove(&k) {
                n += 1;
                freed += r.bytes();
            }
        }
        (n, freed)
    }

    /// Create the page texture for `key`, `bytes` large. Least recently used pages make room in
    /// the budget first; when the device is still out of memory, every page the current cell
    /// doesn't use is dropped and creation is tried once more (#6563). `None` (with
    /// `layer_alloc` set) when it still fails: `render` submits the recorded work, which can still
    /// hold the dropped pages, and runs the cell once more; after that the CPU draws the refresh.
    pub(crate) fn create_layer_texture(
        &mut self,
        device: &wgpu::Device,
        desc: &wgpu::TextureDescriptor<'_>,
        key: ResKey,
        bytes: u64,
        stats: &mut Stats,
    ) -> Option<wgpu::Texture> {
        let replaced = self.residents.get(&key).map_or(0, Resident::bytes);
        let (n, dropped) = self.evict_residents(self.resident_budget, bytes.saturating_sub(replaced), Some(key));
        stats.evicted += n;
        self.admit_freed = self.admit_freed.saturating_add(dropped);
        // WebGPU reports errors asynchronously: its failures reach the health callbacks.
        if cfg!(target_arch = "wasm32") {
            return Some(device.create_texture(desc));
        }
        for retry in [false, true] {
            let (texture, mut error) = first_error(device, || device.create_texture(desc));
            if error.is_none() && self.fail_allocs > 0 {
                self.fail_allocs -= 1;
                error = Some(wgpu::Error::OutOfMemory { source: "out of memory (test hook)".into() });
            }
            match error {
                None => return Some(texture),
                Some(wgpu::Error::OutOfMemory { .. }) if !retry => {
                    let (n, dropped) = self.evict_residents(0, 0, Some(key));
                    stats.evicted += n;
                    self.admit_freed = self.admit_freed.saturating_add(dropped);
                }
                Some(wgpu::Error::OutOfMemory { .. }) => {
                    self.layer_alloc = Some(OVER_BUDGET.into());
                    return None;
                }
                Some(e) => {
                    self.layer_alloc = Some(format!("GPU layer texture failed: {e}"));
                    return None;
                }
            }
        }
        self.layer_alloc = Some(OVER_BUDGET.into());
        None
    }

    /// Per-frame resources of `plan`: the tile area of each pass's surfaces, its LUT and pattern.
    fn frame_res(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, canvas: Rect, plan: &Plan<'_>, stats: &mut Stats) -> FrameRes {
        let grid = tile_grid(canvas);
        let area = |s: &Surface| s.tile_bounds().intersect(&grid);
        let areas = plan
            .passes
            .iter()
            .map(|p| {
                (
                    p.tex.as_ref().map_or(Rect::EMPTY, |t| area(t.surface.get())),
                    p.mask
                        .as_ref()
                        .and_then(|m| match &m.source {
                            plan::MaskSource::Surface(surface) => Some(area(surface.get())),
                            plan::MaskSource::Slot(_) => None,
                        })
                        .unwrap_or(Rect::EMPTY),
                )
            })
            .collect();
        let luts = plan.passes.iter().map(|p| p.lut.as_ref().map(|rows| lut_texture(device, queue, rows))).collect();
        let patterns = plan.passes.iter().map(|p| p.pattern.map(|pat| self.pattern_view(device, queue, pat))).collect();
        let mask_fills = plan.passes.iter().map(|p| p.mask_fill.as_ref().map(|fill| self.mask_fill_view(device, queue, fill, stats))).collect();
        FrameRes { areas, luts, patterns, mask_fills }
    }

    fn mask_fill_view(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fill: &plan::MaskFillUse<'_>, stats: &mut Stats) -> wgpu::TextureView {
        let surface = fill.surface;
        let fingerprint = surface.tiles().fold((u64::from(fill.size[0]) << 32) | u64::from(fill.size[1]), |acc, (coord, tile)| {
            acc.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (Arc::as_ptr(tile) as usize as u64) ^ ((coord.tx as u64) << 20) ^ coord.ty as u64
        });
        let default_bits = surface.default_pixel().iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        let key = (self.pattern_doc, fill.layer, fill.index);
        let frame = self.frame;
        if let Some(cached) = self.mask_fills.get_mut(&key)
            && cached.fingerprint == fingerprint
            && cached.format == surface.format()
            && cached.default_bits == default_bits
        {
            cached.last_used = frame;
            return cached.texture.view.clone();
        }
        let (width, height) = (fill.size[0], fill.size[1]);
        let row_width = width.div_ceil(64) * 64;
        let texture = Tex::new(
            device,
            "pc_mask_fill_source",
            row_width,
            height,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let channels = surface.channels().max(1);
        for y in (0..height).step_by(64) {
            let rows = (height - y).min(64);
            let pixels = surface.read_region(Rect::new(0, y as i32, width as i32, (y + rows) as i32));
            let mut data = vec![0_u8; row_width as usize * rows as usize * 4];
            for (source, destination) in pixels.chunks_exact(width as usize * channels).zip(data.chunks_exact_mut(row_width as usize * 4)) {
                for (pixel, bytes) in source.chunks_exact(channels).zip(destination.as_chunks_mut::<4>().0) {
                    *bytes = pixel.first().copied().unwrap_or(0.0).to_le_bytes();
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row_width * 4), rows_per_image: Some(rows) },
                wgpu::Extent3d { width: row_width, height: rows, depth_or_array_layers: 1 },
            );
        }
        let bytes = u64::from(row_width) * u64::from(height) * 4;
        stats.mask_fill_uploads = stats.mask_fill_uploads.saturating_add(1);
        stats.mask_fill_bytes_uploaded = stats.mask_fill_bytes_uploaded.saturating_add(bytes);
        let view = texture.view.clone();
        let pin = surface.tiles().map(|(_, tile)| Arc::downgrade(tile)).collect();
        self.mask_fills.insert(key, CachedMaskFill { fingerprint, _pin: pin, format: surface.format(), default_bits, texture, last_used: frame, bytes });
        view
    }
}
