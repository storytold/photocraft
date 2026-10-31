use super::batch::{batch_kernel, batch_op_words, op_words, submit_batch};
use super::consts::{FLUSH_OPS, MAP32, STRIDE};
use super::convert::{convert_tile, convert_tiles, keep_overlap, put_tiles, words};
use super::kit::{Bound, FrameRes, texture_group, uniform_group};
use super::paging::{OVER_BUDGET, cell_rect};
use super::textures::{CachedPattern, Cell, ChunkOut, ResKey, Resident, ResidentRef, Stats, Tex, TexKind};
use super::{
    Arc, Compositor, DocId, HashMap, I, Kernel, LayerOperator, OperatorTarget, Pattern, Plan, Rect, Role, Surface, TILE_SIZE, Tile, TileCoord, operator, plan,
};

impl Compositor {
    /// Bring the pages of `cell` that `plan` samples up to date and resolve its effect maps.
    #[allow(clippy::too_many_arguments)]
    fn bind_cell(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: DocId,
        plan: &Plan<'_>,
        paged: &[bool],
        cell: Cell,
        fr: &FrameRes,
        first_pass: usize,
        stats: &mut Stats,
    ) -> Bound {
        // The operators of the document being drawn (Arc clones: `sync_operator` needs `self`).
        let ops = self.frame_ops.clone();
        let cr = cell_rect(cell, self.page);
        let mut views: Vec<(Option<ResidentRef>, Option<ResidentRef>)> = Vec::with_capacity(plan.passes.len());
        let mut keys: Vec<ResKey> = Vec::new();
        let mut maps = Vec::with_capacity(plan.passes.len());
        for (i, p) in plan.passes.iter().enumerate() {
            if i < first_pass {
                views.push((None, None));
                maps.push(None);
                continue;
            }
            let (ta, ma) = fr.areas.get(i).copied().unwrap_or((Rect::EMPTY, Rect::EMPTY));
            let op =
                p.tex.as_ref().filter(|t| t.role == Role::Content).and_then(|t| ops.iter().find(|o| o.layer == t.layer && o.target == OperatorTarget::Content));
            let tex = p
                .tex
                .as_ref()
                .and_then(|t| match op {
                    Some(o) => self.sync_operator(device, queue, encoder, doc, o, t.surface.get(), cell, ta.intersect(&cr), stats),
                    None => self.sync(device, queue, encoder, doc, (t.layer, t.role, cell), t.surface.get(), ta.intersect(&cr), stats),
                })
                .map(|(k, r)| {
                    keys.push(k);
                    (keys.len() - 1, r)
                });
            let mask = p
                .mask
                .as_ref()
                .and_then(|m| match ops.iter().find(|o| o.layer == m.layer && o.target == OperatorTarget::Mask) {
                    Some(o) => self.sync_operator(device, queue, encoder, doc, o, m.surface.get(), cell, ma.intersect(&cr), stats),
                    None => self.sync(device, queue, encoder, doc, (m.layer, Role::Mask, cell), m.surface.get(), ma.intersect(&cr), stats),
                })
                .map(|(k, r)| {
                    keys.push(k);
                    (keys.len() - 1, r)
                });
            views.push((tex, mask));
            maps.push(p.map.and_then(|m| {
                let f = plan.fx.get(m.fx)?;
                let key = (f.layer.id, paged.get(m.fx).copied().unwrap_or(false).then_some(cell));
                let e = self.fx.get(&key)?;
                if m.item == plan::SHAPE_MAP {
                    return Some((e.shape.view.clone(), e.region));
                }
                let t = e.progs.get(m.item)?.maps.get(m.map)?.as_ref()?;
                Some((t.view.clone(), e.region))
            }));
        }
        Bound { views, keys, maps }
    }

    /// Run `plan` (drawn as `runs`, its `batch_runs`) over the `chunks` of one page cell
    /// (document rects, at most a chunk square), handing each finished chunk to `sink`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_cell(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: DocId,
        plan: &Plan<'_>,
        runs: &[usize],
        paged: &[bool],
        cell: Cell,
        fr: &FrameRes,
        chunks: &[Rect],
        checkpoint: Option<plan::Checkpoint>,
        stats: &mut Stats,
        sink: &mut dyn FnMut(&mut wgpu::CommandEncoder, ChunkOut<'_>),
        // The previous intermediate submit, when this refresh may submit in between.
        batch: &mut Option<&mut Option<wgpu::SubmissionIndex>>,
    ) {
        if chunks.is_empty() {
            return;
        }
        // Chunks may hold different checkpoints: bind from the lowest one any of them resumes at.
        let first_pass = checkpoint
            .and_then(|_| chunks.iter().map(|c| self.prefix.restore_point(*c).map(|r| r.pass_end)).try_fold(usize::MAX, |m, p| Some(m.min(p?))))
            .unwrap_or(0);
        let bound = self.bind_cell(device, queue, encoder, doc, plan, paged, cell, fr, first_pass, stats);
        // A page couldn't be created: draw nothing with the pages missing (the caller ends the refresh).
        if self.layer_alloc.is_some() {
            return;
        }

        // Chunk pool.
        while self.pool.len() < plan.slots as usize {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pc_compose_chunk"),
                size: wgpu::Extent3d { width: self.chunk, height: self.chunk, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.acc_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            self.pool.push((t, v));
        }

        // Uniforms: chunk records, then one record per pass. encode_scoped preflight checked
        // the length, device capacity and dynamic offsets before any GPU work for this frame.
        let op_base = chunks.len() as u64 * STRIDE;
        let mut data = vec![0u8; (op_base + plan.passes.len() as u64 * STRIDE) as usize];
        for (i, c) in chunks.iter().enumerate() {
            let w = words(&[I(c.x0), I(c.y0), I(c.width() as i32), I(c.height() as i32)]);
            data[i * STRIDE as usize..][..w.len()].copy_from_slice(&w);
        }
        for (i, p) in plan.passes.iter().enumerate() {
            if i < first_pass {
                continue;
            }
            let (tex, mask) = bound.views[i];
            let map = bound.maps[i].as_ref().map(|(_, r)| *r);
            let run = runs.get(i).copied().unwrap_or(1);
            let w = if run > 1 {
                let extras: Vec<_> = (i + 1..i + run).filter_map(|j| Some((plan.passes.get(j)?, bound.views.get(j).and_then(|v| v.0).map(|t| t.1)))).collect();
                batch_op_words(p, tex.map(|t| t.1), mask.map(|m| m.1), &extras)
            } else {
                op_words(p, tex.map(|t| t.1), mask.map(|m| m.1), map)
            };
            let off = (op_base + i as u64 * STRIDE) as usize;
            data[off..][..w.len()].copy_from_slice(&w);
        }
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_compose_uniforms"),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, &data);
        let bg0 = uniform_group(device, &self.kit.bgl0, &ubuf);

        // Per-pass texture bind groups (chunk-independent: pool slots are reused per chunk).
        let dummy = &self.kit.dummy;
        let resident_views: Vec<&wgpu::TextureView> = bound.keys.iter().map(|k| self.residents.get(k).map_or(dummy, |r| &r.view)).collect();
        let mut bg1 = Vec::with_capacity(plan.passes.len());
        for (i, p) in plan.passes.iter().enumerate() {
            if i < first_pass || p.kernel.entry().is_none() {
                bg1.push(None);
                continue;
            }
            let slot = |s: Option<u32>| s.map_or(dummy, |s| &self.pool[s as usize].1);
            let (tex, mask) = &bound.views[i];
            let tv = tex.map_or(slot(p.d), |(k, _)| resident_views[k]);
            let mv = mask.map_or(dummy, |(k, _)| resident_views[k]);
            let lv = fr.luts.get(i).and_then(Option::as_ref).map_or(dummy, |(_, v)| v);
            let map = bound.maps[i].as_ref().map_or(dummy, |(v, _)| v);
            let pat = fr.patterns.get(i).and_then(Option::as_ref).unwrap_or(dummy);
            let run = runs.get(i).copied().unwrap_or(1);
            if run > 1 {
                // The extra layers' pixels in tex_b, tex_c and map_tex (`batch_layer`).
                let extra = |e: usize| bound.views.get(i + e).and_then(|v| v.0).map_or(dummy, |(k, _)| resident_views[k]);
                bg1.push(Some(texture_group(device, &self.kit.bgl1, &[slot(p.a), extra(1), tv, mv, lv, extra(2), extra(3), pat])));
                continue;
            }
            bg1.push(Some(texture_group(device, &self.kit.bgl1, &[slot(p.a), slot(p.b), tv, mv, lv, slot(p.c), map, pat])));
        }

        for (ci, c) in chunks.iter().enumerate() {
            let mut resume = 0;
            if let Some(cp) = checkpoint.and_then(|_| self.prefix.restore_point(*c))
                && let Some((dst, _)) = self.pool.get(cp.slot as usize)
                && self.prefix.copy_out(encoder, *c, dst)
            {
                resume = cp.pass_end;
                stats.prefix_hits = stats.prefix_hits.saturating_add(1);
                stats.prefix_passes_skipped = stats.prefix_passes_skipped.saturating_add(resume);
            }
            // Passes drawn inside the previous `fs_layerbatch` pass.
            let mut batched_to = 0;
            for (i, p) in plan.passes.iter().enumerate() {
                if i < resume || i < batched_to {
                    continue;
                }
                // Chunk textures, uniforms and bind groups outlive the submit, and the queue
                // keeps the order, so a chunk may continue in the next command buffer.
                if self.recorded >= FLUSH_OPS
                    && let Some(last) = batch.as_deref_mut()
                {
                    submit_batch(device, queue, encoder, self.health.as_ref(), self.timer.as_mut(), last);
                    self.staged = 0;
                    self.recorded = 0;
                    stats.flushes += 1;
                    if self.fault().is_some() {
                        return;
                    }
                }
                // Also after resuming from a lower checkpoint: the chunk moves up to this one.
                if resume < i
                    && let Some(cp) = checkpoint.filter(|cp| cp.pass_end == i)
                    && let Some((src, _)) = self.pool.get(cp.slot as usize)
                {
                    self.prefix.capture(device, encoder, *c, src, cp);
                }
                let scissor = match p.clip {
                    Some(clip) => {
                        let r = clip.intersect(c);
                        if r.is_empty() {
                            continue;
                        }
                        Rect::new(r.x0 - c.x0, r.y0 - c.y0, r.x1 - c.x0, r.y1 - c.y0)
                    }
                    None => Rect::new(0, 0, c.width() as i32, c.height() as i32),
                };
                match p.kernel {
                    Kernel::CopyRect | Kernel::CopyFull => {
                        // The planner always gives copies a source.
                        let Some(a) = p.a else { continue };
                        let src = &self.pool[a as usize].0;
                        let dst = &self.pool[p.dst as usize].0;
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: src,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: scissor.x0 as u32, y: scissor.y0 as u32, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: dst,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: scissor.x0 as u32, y: scissor.y0 as u32, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d { width: scissor.width(), height: scissor.height(), depth_or_array_layers: 1 },
                        );
                        self.recorded += 1;
                        continue;
                    }
                    _ => {}
                }
                self.recorded += 1;
                // A run draws as one pass into the slot its last layer writes.
                let run = runs.get(i).copied().unwrap_or(1);
                let (kernel, dst) = match plan.passes.get(i + run - 1) {
                    Some(last) if run > 1 => {
                        batched_to = i + run;
                        stats.layers_batched = stats.layers_batched.saturating_add(run - 1);
                        let kernel = plan.passes.get(i..i + run).map_or(Kernel::LayerBatch, batch_kernel);
                        if kernel == Kernel::LayerBatchNormal {
                            stats.normal_batches = stats.normal_batches.saturating_add(1);
                        }
                        (kernel, last.dst)
                    }
                    _ => (p.kernel, p.dst),
                };
                let target = &self.pool[dst as usize].1;
                let clear = p.kernel == Kernel::Clear;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pc_compose_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if clear {
                    continue;
                }
                pass.set_pipeline(&self.kit.pipelines[&(kernel, self.acc_format)]);
                pass.set_scissor_rect(scissor.x0 as u32, scissor.y0 as u32, scissor.width(), scissor.height());
                pass.set_bind_group(0, &bg0, &[(ci as u64 * STRIDE) as u32, (op_base + i as u64 * STRIDE) as u32]);
                pass.set_bind_group(1, bg1[i].as_ref(), &[]);
                pass.draw(0..3, 0..1);
            }
            if resume < plan.passes.len()
                && let Some(cp) = checkpoint.filter(|cp| cp.pass_end == plan.passes.len())
                && let Some((src, _)) = self.pool.get(cp.slot as usize)
            {
                self.prefix.capture(device, encoder, *c, src, cp);
            }
            let (t, v) = &self.pool[plan.root as usize];
            sink(encoder, ChunkOut { rect: *c, texture: t, view: v });
        }
    }

    /// The page of `cell` that layer operator `op` outputs over `region` (the cell's part of the
    /// layer's pixels and what the operator changes), brought up to date: the window of the
    /// layer's `surface` it reads is uploaded when its tiles changed, and the passes run when the
    /// window or the program did. Bound in place of the layer's own page. `None` (with
    /// `layer_alloc` set, so the refresh goes to the CPU rather than show the layer unfiltered)
    /// when a texture can't be had.
    #[allow(clippy::too_many_arguments)]
    fn sync_operator(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: DocId,
        op: &LayerOperator,
        surface: &Surface,
        cell: Cell,
        region: Rect,
        stats: &mut Stats,
    ) -> Option<(ResKey, [i32; 4])> {
        if region.is_empty() || self.layer_alloc.is_some() {
            return None;
        }
        let operator::Program::Separable(blur) = op.program.program();
        let (wk, ok, mk) = ((op.layer, Role::OperatorSource, cell), (op.layer, Role::Operator, cell), (op.layer, Role::OperatorMask, cell));
        // The cell's source window stays the same while the view moves (uploaded once) and while
        // a slider drag's reach still fits it; when it outgrows it, the new one has headroom, so
        // a drag doesn't upload every window again at each step. The blur and its output cover
        // only the part this refresh draws.
        let needed = region.inflate(blur.reach());
        let window = match self.residents.get(&wk).map(|r| r.region) {
            Some(held) if held.contains_rect(&needed) => held,
            _ => region.inflate(operator::apron(blur.reach().saturating_add(blur.reach() / 4))),
        };
        let region = region.intersect(&self.frame_region);
        if region.is_empty() {
            return None;
        }
        let sub = region.inflate(operator::apron(blur.reach())).intersect(&window);
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST;
        let desc = |label, r: Rect| wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: r.width().max(1), height: r.height().max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage,
            view_formats: &[],
        };
        let bytes = |r: Rect| u64::from(r.width()) * u64::from(r.height()) * 16;
        // The source window: what the passes read. Its tiles fingerprint it (the edge repeats from
        // inside the extent, so those are the tiles read).
        let read = window.intersect(&blur.extent);
        let read = if read.is_empty() { window } else { read };
        let tiles: Vec<(TileCoord, Arc<Tile>)> = read.tiles().filter_map(|c| surface.tile(c).map(|t| (c, t.clone()))).collect();
        let source_print = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            // The depth picks the source texels' format (RGBA8 or floats).
            std::hash::Hash::hash(&(window, blur.extent, blur.depth as u8), &mut h);
            for (c, t) in &tiles {
                std::hash::Hash::hash(&(c.tx, c.ty, Arc::as_ptr(t) as usize), &mut h);
            }
            std::hash::Hash::hash(&surface.default_pixel().iter().map(|v| v.to_bits()).collect::<Vec<_>>(), &mut h);
            std::hash::Hasher::finish(&h)
        };
        let fresh = |s: &Self, k: &ResKey, r: Rect, print: u64| s.residents.get(k).is_some_and(|x| x.region == r) && s.op_made.get(k) == Some(&print);
        // A page reused in this call is marked used now, so making the next one can't evict it.
        let touch = |s: &mut Self, k: &ResKey| {
            let (frame, stamp) = (s.frame, s.stamp);
            if let Some(r) = s.residents.get_mut(k) {
                r.last_used = frame;
                r.stamp = stamp;
            }
        };
        let mut changed = false;
        if fresh(self, &wk, window, source_print) {
            touch(self, &wk);
        } else {
            // 8-bit sources as 8-bit texels: exact, and a quarter of the upload.
            let eight = operator::source_is_8bit(blur.depth);
            let (format, kind, bpp) =
                if eight { (wgpu::TextureFormat::Rgba8Unorm, TexKind::Rgba8, 4) } else { (wgpu::TextureFormat::Rgba32Float, TexKind::Rgba32F, 16) };
            let source_desc = wgpu::TextureDescriptor {
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                ..desc("pc_operator_source", window)
            };
            let size = u64::from(window.width()) * u64::from(window.height()) * bpp;
            let texture = self.create_layer_texture(device, &source_desc, wk, size, stats)?;
            let texels = operator::read_window(surface, window, blur.extent);
            operator::write_bytes(queue, &texture, window.width(), window.height(), bpp as u32, &operator::source_bytes(blur.depth, &texels));
            self.staged += size;
            stats.bytes_uploaded += size as usize;
            let mut resident = self.operator_resident(texture, window, surface, doc);
            resident.kind = kind;
            // The fingerprint hashes the tiles' addresses: holding them keeps another tile from
            // taking one of those addresses while the page is kept.
            resident.tiles = tiles.iter().cloned().collect();
            self.residents.insert(wk, resident);
            self.op_made.insert(wk, source_print);
            changed = true;
        }
        // The selection over the output, once per selection.
        // The whole output: an inverted selection's default pixel is selected.
        let selection = blur.selection.as_ref().map(|s| (s, region));
        if let Some((sel, sr)) = &selection {
            let print = {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&(Arc::as_ptr(sel) as usize, *sr), &mut h);
                std::hash::Hasher::finish(&h)
            };
            if fresh(self, &mk, *sr, print) {
                touch(self, &mk);
            } else if !sr.is_empty() {
                let texture = self.create_layer_texture(device, &desc("pc_operator_selection", *sr), mk, bytes(*sr), stats)?;
                let k: Vec<[f32; 4]> = operator::read_selection(sel, *sr).into_iter().map(|v| [v, 0.0, 0.0, 0.0]).collect();
                operator::write_texels(queue, &texture, sr.width(), sr.height(), &k);
                self.staged += bytes(*sr);
                let resident = self.operator_resident(texture, *sr, surface, doc);
                self.residents.insert(mk, resident);
                self.op_made.insert(mk, print);
                self.op_pins.insert(mk, Arc::clone(sel));
                changed = true;
            }
        }
        // The output: redrawn when the program, the window or the selection changed.
        let print = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&(op.program.key(), source_print, region, self.op_made.get(&mk).copied()), &mut h);
            std::hash::Hasher::finish(&h)
        };
        if changed || !fresh(self, &ok, region, print) {
            if !self.operators_supported(device) {
                self.layer_alloc = Some("GPU layer operators are unavailable on this device".into());
                return None;
            }
            // The output page of an unchanged region is written again in place.
            let out = match self.residents.get(&ok).filter(|r| r.region == region && r.kind == TexKind::Rgba32F) {
                Some(r) => r.texture.clone(),
                None => self.create_layer_texture(device, &desc("pc_operator_output", region), ok, bytes(region), stats)?,
            };
            let size = [sub.width(), sub.height()];
            let count = operator::scratch_count(&blur.blur);
            if self.op_scratch.as_ref().is_none_or(|(s, t)| s[0] < size[0] || s[1] < size[1] || t.len() < count) {
                let grown = self.op_scratch.as_ref().map_or(size, |(s, _)| [s[0].max(size[0]), s[1].max(size[1])]);
                // Within the budget and its fallback, as layer pages are: the old set goes first,
                // room is made for the whole new set, and each texture counts once made.
                self.op_scratch = None;
                let each = bytes(Rect::from_xywh(0, 0, grown[0], grown[1]));
                let (n, dropped) = self.evict_residents(self.resident_budget, each.saturating_mul(count as u64), Some(ok));
                stats.evicted += n;
                self.admit_freed = self.admit_freed.saturating_add(dropped);
                let mut made = Vec::with_capacity(count);
                for i in 0..count {
                    let label = format!("pc_operator_scratch{i}");
                    let desc = operator::float_desc(&label, grown[0], grown[1]);
                    let Some(texture) = self.create_budgeted(device, &desc, each, Some(ok), stats) else {
                        self.scratch_making = 0;
                        return None;
                    };
                    self.scratch_making = self.scratch_making.saturating_add(each);
                    made.push(texture);
                }
                self.scratch_making = 0;
                self.op_scratch = Some((grown, made));
            }
            let Some(source) = self.residents.get(&wk).map(|r| r.texture.clone()) else {
                // Evicted making room for the output: the refresh goes to the CPU.
                self.layer_alloc = Some(OVER_BUDGET.into());
                return None;
            };
            let sel = selection.as_ref().and_then(|(_, sr)| {
                let r = self.residents.get(&mk)?;
                Some((sr.translate(-region.x0, -region.y0), r.texture.create_view(&Default::default())))
            });
            let (Some(Some(kit)), Some((_, scratch))) = (self.op_kit.as_ref(), self.op_scratch.as_ref()) else { return None };
            let origin = [(sub.x0 - window.x0) as u32, (sub.y0 - window.y0) as u32];
            let blurred = operator::record_blur(device, encoder, kit, &blur.blur, &source, origin, size, scratch)?;
            let fmt = surface.format();
            operator::record_finish(
                device,
                encoder,
                kit,
                &source.create_view(&Default::default()),
                &blurred.create_view(&Default::default()),
                &out.create_view(&Default::default()),
                operator::FinishParams {
                    size: [region.width(), region.height()],
                    offset: [(region.x0 - window.x0) as u32, (region.y0 - window.y0) as u32],
                    blur_offset: [(region.x0 - sub.x0) as u32, (region.y0 - sub.y0) as u32],
                    area: blur.area.intersect(&blur.extent).intersect(&region).translate(-region.x0, -region.y0),
                    selection: sel,
                    keep_alpha: blur.keep_alpha,
                    levels: operator::levels(blur.depth),
                    has_alpha: fmt.alpha,
                    combine: blur.combine,
                },
            );
            self.recorded += 1;
            let resident = self.operator_resident(out, region, surface, doc);
            self.residents.insert(ok, resident);
            self.op_made.insert(ok, print);
            stats.operator_cells += 1;
        }
        for k in [wk, ok, mk] {
            if let Some(r) = self.residents.get_mut(&k) {
                r.last_used = self.frame;
                r.stamp = self.stamp;
            }
        }
        let r = self.residents.get(&ok)?;
        Some((ok, [r.region.x0, r.region.y0, r.region.width() as i32, r.region.height() as i32]))
    }

    /// A resident holding an operator's float texture over `region`.
    fn operator_resident(&self, texture: wgpu::Texture, region: Rect, surface: &Surface, doc: DocId) -> Resident {
        let view = texture.create_view(&Default::default());
        Resident {
            texture,
            view,
            region,
            kind: TexKind::Rgba32F,
            format: surface.format(),
            tiles: HashMap::new(),
            default_nonzero: false,
            default_bits: Vec::new(),
            doc,
            last_used: self.frame,
            stamp: self.stamp,
            cmyk: 0,
            // Written by the encoder's passes: later writes must go through it too.
            ordered: true,
        }
    }

    /// Upload changed tiles of `surface` within `region` (its tile area inside one page cell);
    /// returns the resident key and its region (x, y, w, h).
    #[allow(clippy::too_many_arguments)]
    fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: DocId,
        key: ResKey,
        surface: &Surface,
        region: Rect,
        stats: &mut Stats,
    ) -> Option<(ResKey, [i32; 4])> {
        // An earlier page of this cell didn't fit: the refresh ends after the cell anyway.
        if region.is_empty() || self.layer_alloc.is_some() {
            return None;
        }
        let format = surface.format();
        let kind = TexKind::for_surface(key.1, format);
        let cmyk = if format.mode == photocraft_color::ColorMode::Cmyk { self.cmyk } else { 0 };
        let default_pixel = surface.default_pixel();
        let default_bits: Vec<u32> = default_pixel.iter().map(|v| v.to_bits()).collect();
        let stale = self
            .residents
            .get(&key)
            .is_none_or(|r| r.region != region || r.kind != kind || r.format != format || r.cmyk != cmyk || r.default_bits != default_bits);
        if stale {
            let desc = wgpu::TextureDescriptor {
                label: Some("pc_compose_layer"),
                size: wgpu::Extent3d { width: region.width(), height: region.height(), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: kind.format(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            };
            let bytes = region.width() as u64 * region.height() as u64 * kind.bytes_per_pixel() as u64;
            let texture = self.create_layer_texture(device, &desc, key, bytes, stats)?;
            let view = texture.create_view(&Default::default());
            let default_nonzero = default_pixel.iter().any(|v| *v != 0.0);
            let mut r = Resident {
                texture,
                view,
                region,
                kind,
                format,
                tiles: HashMap::new(),
                default_nonzero,
                default_bits: default_bits.clone(),
                doc,
                last_used: 0,
                stamp: 0,
                cmyk,
                ordered: false,
            };
            // Only the region moved (painting past a layer's tiles grows it): keep the texels the
            // old texture already holds instead of converting and uploading every tile again.
            let kept = self.residents.remove(&key).map_or(Rect::EMPTY, |old| keep_overlap(device, queue, &old, &mut r));
            // A new layer whose tiles another resident already holds (a duplicate shares its
            // source's tiles) starts as a GPU copy of that texture. Duplicating a 20 MP photo
            // converted and uploaded every tile again.
            let shares =
                |o: &Resident| region.tiles().filter(|c| surface.tile(*c).is_some_and(|t| o.tiles.get(c).is_some_and(|ot| Arc::ptr_eq(ot, t)))).count();
            let donor = self
                .residents
                .iter()
                .filter(|(k, o)| k.1 == key.1 && o.region == region && o.kind == kind && o.format == format && o.cmyk == cmyk && o.default_bits == default_bits)
                .map(|(k, o)| (shares(o), *k))
                .filter(|(n, _)| *n > 0)
                .max_by_key(|(n, _)| *n);
            if kept.is_empty()
                && let Some((n, o)) = donor.and_then(|(n, k)| Some((n, self.residents.get_mut(&k)?)))
            {
                // Recorded in the frame's encoder, after the writes and copies that brought the
                // donor up to date ([`keep_overlap`]'s own submission would run before those).
                o.ordered = true;
                encoder.copy_texture_to_texture(
                    o.texture.as_image_copy(),
                    r.texture.as_image_copy(),
                    wgpu::Extent3d { width: region.width(), height: region.height(), depth_or_array_layers: 1 },
                );
                r.tiles = o.tiles.clone();
                r.ordered = true;
                stats.tiles_shared += n;
            } else {
                // A new texture starts zeroed: write the tiles that are present, and the default
                // pixel where they are missing if it isn't zero. Coordinates sharing a tile (a
                // solid fill, the default) take one upload and GPU copies (#1774). Kept tiles are
                // brought up to date below, like those of a resident that didn't move.
                let mut present = Vec::new();
                let mut missing = Vec::new();
                for c in region.tiles().filter(|c| !kept.contains_rect(&c.rect())) {
                    match surface.tile(c) {
                        Some(t) => present.push((c, t.clone())),
                        None if default_nonzero => missing.push(c),
                        None => {}
                    }
                }
                if !missing.is_empty() {
                    let bytes = convert_tile(surface, None, kind, TileCoord::new(0, 0));
                    self.staged += put_tiles(device, queue, encoder, &r, &missing, &bytes);
                }
                let staged = &mut self.staged;
                convert_tiles(surface, present, kind, |t, cs, bytes| {
                    *staged += put_tiles(device, queue, encoder, &r, cs, bytes);
                    for &c in cs {
                        stats.tiles_uploaded += 1;
                        stats.bytes_uploaded += bytes.len();
                        r.tiles.insert(c, t.clone());
                    }
                });
            }
            // Tiles the donor held that this surface doesn't, or holds differently, are fixed up
            // by the pass over the region below.
            self.residents.insert(key, r);
        }
        // Present: inserted above when it was missing or stale.
        let r = self.residents.get_mut(&key)?;
        r.last_used = self.frame;
        r.stamp = self.stamp;
        r.doc = doc;
        let mut changed = Vec::new();
        let mut gone = Vec::new();
        for c in region.tiles() {
            match surface.tile(c) {
                Some(t) => {
                    if !r.tiles.get(&c).is_some_and(|old| Arc::ptr_eq(old, t)) {
                        changed.push((c, t.clone()));
                    }
                }
                None => {
                    if r.tiles.remove(&c).is_some() {
                        gone.push(c);
                    }
                }
            }
        }
        if !gone.is_empty() {
            // A tile that went away reads as the default pixel again.
            let bytes = if r.default_nonzero {
                convert_tile(surface, None, kind, TileCoord::new(0, 0))
            } else {
                vec![0u8; (TILE_SIZE * TILE_SIZE) as usize * kind.bytes_per_pixel()]
            };
            self.staged += put_tiles(device, queue, encoder, r, &gone, &bytes);
        }
        let staged = &mut self.staged;
        convert_tiles(surface, changed, kind, |t, cs, bytes| {
            *staged += put_tiles(device, queue, encoder, r, cs, bytes);
            for &c in cs {
                stats.tiles_uploaded += 1;
                stats.bytes_uploaded += bytes.len();
                r.tiles.insert(c, t.clone());
            }
        });
        Some((key, [region.x0, region.y0, region.width() as i32, region.height() as i32]))
    }

    /// Premultiplied RGBA32F texture of a pattern as it paints in the document's mode (cached by
    /// pixel identity and mode).
    pub(crate) fn pattern_view(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pat: &Pattern) -> wgpu::TextureView {
        let fp = pat.surface.tiles().fold((pat.width as u64) << 32 | pat.height as u64, |acc, (c, t)| {
            acc.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (Arc::as_ptr(t) as usize as u64) ^ ((c.tx as u64) << 20) ^ c.ty as u64
        });
        let mode = self.pattern_mode;
        let key = format!("{}\u{0}{}\u{0}{mode:?}", pat.id, pat.name);
        let format = pat.surface.format();
        // Unstored tiles read the default pixel, interpreted in the source format.
        let default_bits: Vec<u32> = pat.surface.default_pixel().iter().map(|v| v.to_bits()).collect();
        let (doc, last_used) = (self.pattern_doc, self.frame);
        if let Some(cached) = self.patterns.get_mut(&key)
            && cached.fingerprint == fp
            && cached.format == format
            && cached.default_bits == default_bits
        {
            cached.doc = doc;
            cached.last_used = last_used;
            return cached.texture.view.clone();
        }
        let (w, h) = (pat.width, pat.height);
        let px = photocraft_compose::pattern::pattern_rgba(pat, mode);
        let bytes: Vec<u8> = px.iter().flat_map(|q| [q[0] * q[3], q[1] * q[3], q[2] * q[3], q[3]]).flat_map(f32::to_le_bytes).collect();
        let t = Tex::new(device, "pc_pattern", w, h, wgpu::TextureFormat::Rgba32Float, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let v = t.view.clone();
        let bytes = u64::from(w.max(1)) * u64::from(h.max(1)) * 16;
        self.patterns.insert(key, CachedPattern { fingerprint: fp, format, default_bits, texture: t, doc, last_used, bytes });
        v
    }

    /// An R32F temporary at least `region`-sized (the smallest that fits) not in `taken`.
    /// Stages only read what they wrote, inside the region, so a larger one is as good; page
    /// windows of different sizes share them.
    pub(crate) fn temp(&mut self, device: &wgpu::Device, region: Rect, taken: &[wgpu::TextureView]) -> wgpu::TextureView {
        let (w, h) = (region.width(), region.height());
        let frame = self.frame;
        let fit = self
            .temps
            .iter_mut()
            .filter(|(t, _)| {
                let s = t.texture.size();
                s.width >= w && s.height >= h && !taken.contains(&t.view)
            })
            .min_by_key(|(t, _)| t.texture.size().width as u64 * t.texture.size().height as u64);
        if let Some((t, used)) = fit {
            *used = frame;
            return t.view.clone();
        }
        let t = Tex::map(device, "pc_fx_temp", region, MAP32);
        let v = t.view.clone();
        self.temps.push((t, frame));
        v
    }
}
