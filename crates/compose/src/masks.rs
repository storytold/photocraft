//! A layer's effective mask (pixel mask × vector mask, each feathered) as one surface, for
//! backends that sample masks from textures (the GPU compositor) and for feathered masks on the
//! CPU. Rasterising a vector mask costs a path coverage pass and feathering a blur, so results
//! are cached per mask state; unchanged masks return the same tiles (cheap to clone, and the
//! GPU sees no change). Retention is bounded by both entry count and accounted pixel bytes;
//! [`purge_cache`] drops cached masks and prevents older builds from refilling the cache.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::Layer;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

// A U16 mask keeps substantially smaller cached tiles than F32 while adding at most
// 1/65535 quantization error. Mask values are normalized coverage, so this is finer than
// the final 8-bit display/composite precision.
const FORMAT: PixelFormat = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U16, alpha: false };

struct CacheEntry {
    surface: Surface,
    tick: u64,
    bytes: usize,
    // The key contains input tile addresses. Keep those tiles alive until eviction:
    // edits must copy them, and the allocator cannot reuse their addresses for new tiles.
    _pixel_input: Option<Surface>,
}

#[derive(Default)]
struct Cache {
    map: HashMap<u64, CacheEntry>,
    /// Builds in progress, shared by every caller that asks for the same cold mask.
    in_flight: HashMap<u64, Weak<OnceLock<Surface>>>,
    tick: u64,
    bytes: usize,
    generation: u64,
}

impl Cache {
    fn get(&mut self, key: u64) -> Option<Surface> {
        self.tick = self.tick.saturating_add(1);
        let entry = self.map.get_mut(&key)?;
        entry.tick = self.tick;
        Some(entry.surface.clone())
    }

    fn insert(&mut self, key: u64, surface: Surface, pixel_input: Option<Surface>, bytes: usize, budget: usize, generation: u64) {
        // A large result can still be used by its caller without keeping it globally. A purge
        // also invalidates admission by computations that were already running.
        if bytes > budget || generation != self.generation {
            return;
        }
        if let Some(previous) = self.map.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        while self.map.len() >= CAPACITY || self.bytes > budget.saturating_sub(bytes) {
            let Some(old) = self.map.iter().min_by_key(|(_, entry)| entry.tick).map(|(&key, _)| key) else { break };
            if let Some(entry) = self.map.remove(&old) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.tick = self.tick.saturating_add(1);
        self.map.insert(key, CacheEntry { surface, tick: self.tick, bytes, _pixel_input: pixel_input });
    }

    fn clear(&mut self) -> usize {
        let bytes = self.bytes;
        self.map.clear();
        self.bytes = 0;
        self.tick = 0;
        self.generation = self.generation.wrapping_add(1);
        bytes
    }
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

pub(crate) fn purge() {
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    c.clear();
    c.in_flight.clear();
}

/// Entries kept (least recently used dropped first).
const CAPACITY: usize = 64;

/// Accounted pixel-byte limit of the combined-mask cache. Five 50 MP U8 input/U16 result pairs
/// fit, while the hard cap still bounds retention for larger documents or more active masks.
pub const CACHE_BUDGET: usize = 768 << 20;

enum CachedMask {
    Hit(Surface),
    /// A build slot, and the cache generation it may fill.
    Pending(Arc<OnceLock<Surface>>, u64),
}

fn lookup(c: &Mutex<Cache>, k: u64) -> CachedMask {
    let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(surface) = c.get(k) {
        return CachedMask::Hit(surface);
    }
    let generation = c.generation;
    // The callers own each slot, so completed builds do not retain an extra
    // surface after their last caller returns. Sweep dead weak slots on misses.
    c.in_flight.retain(|_, slot| slot.strong_count() > 0);
    if let Some(slot) = c.in_flight.get(&k).and_then(Weak::upgrade) {
        return CachedMask::Pending(slot, generation);
    }
    let slot = Arc::new(OnceLock::new());
    c.in_flight.insert(k, Arc::downgrade(&slot));
    CachedMask::Pending(slot, generation)
}

/// The slot's surface, built once by whichever caller gets there first. `build` returns the
/// mask and the pixel-mask input its key was made from (kept alive by the cache entry).
fn finish_mask(c: &Mutex<Cache>, k: u64, slot: &Arc<OnceLock<Surface>>, generation: u64, build: impl FnOnce() -> (Surface, Option<Surface>)) -> Surface {
    slot.get_or_init(|| {
        // Never hold the global cache mutex while rasterizing or feathering.
        let read_generation = photocraft_raster::spill::read_error_generation();
        let (surface, pixel_input) = build();
        if photocraft_raster::spill::check_since(read_generation).is_err() || pixel_input.as_ref().is_some_and(|p| p.scratch_health().is_err()) {
            return surface;
        }
        let bytes = entry_bytes(&surface, pixel_input.as_ref());
        let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
        if c.in_flight.get(&k).and_then(Weak::upgrade).is_some_and(|active| Arc::ptr_eq(&active, slot)) {
            c.insert(k, surface.clone(), pixel_input, bytes, CACHE_BUDGET, generation);
        }
        surface
    })
    .clone()
}

fn cached_mask(c: &Mutex<Cache>, k: u64, build: impl FnOnce() -> (Surface, Option<Surface>)) -> Surface {
    match lookup(c, k) {
        CachedMask::Hit(surface) => surface,
        CachedMask::Pending(slot, generation) => finish_mask(c, k, &slot, generation, build),
    }
}

fn surface_bytes(surface: &Surface) -> usize {
    surface.tiles().fold(surface.default_bytes().len(), |bytes, (_, tile)| bytes.saturating_add(tile.len()))
}

fn entry_bytes(surface: &Surface, pixel_input: Option<&Surface>) -> usize {
    // Charge the whole source as well as the result, including off-canvas tiles. Shared tiles
    // are deliberately charged per entry rather than relying on their current owners.
    surface_bytes(surface).saturating_add(pixel_input.map_or(0, surface_bytes))
}

/// Pixel bytes accounted for by the combined-mask cache. This conservatively includes source
/// tiles and counts tiles shared across entries more than once; it is not process memory use.
pub fn cache_bytes() -> usize {
    cache().lock().unwrap_or_else(|e| e.into_inner()).bytes
}

/// Drop every cached combined mask (Edit › Purge › All), returning its accounted pixel bytes.
/// Callers may still hold returned surfaces, so this is not a measure of freed process memory.
pub fn purge_cache() -> usize {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear()
}

fn key(layer: &Layer, canvas: Rect) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    layer.id.0.hash(&mut h);
    (canvas.x0, canvas.y0, canvas.x1, canvas.y1).hash(&mut h);
    format!("{:?}", layer.vector_mask).hash(&mut h);
    if let Some(m) = &layer.mask {
        (m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(&mut h);
        format!("{:?}", m.surface.default_pixel()).hash(&mut h);
        for (c, t) in m.surface.tiles() {
            (c.tx, c.ty, Arc::as_ptr(t) as usize).hash(&mut h);
        }
    }
    h.finish()
}

pub use photocraft_vector::feather::feather_sigma;
use photocraft_vector::feather::gaussian;

/// Whether `layer` has an enabled mask with a feather (rendered through [`combined_mask`]).
pub fn has_feather(layer: &Layer) -> bool {
    layer.mask.as_ref().is_some_and(|m| m.enabled && feather_sigma(m.feather) > 0.0)
        || layer.vector_mask.as_ref().is_some_and(|v| v.enabled && feather_sigma(v.feather) > 0.0)
}

/// The layer's mask values (pixel mask with density × vector mask, each blurred by its feather,
/// exactly as the CPU compositor applies them) as a single-channel f32 surface over `canvas`, or
/// `None` when the layer has no enabled vector mask and no feathered pixel mask (use the pixel
/// mask directly). Values outside the computed area are the surface's default.
pub fn combined_mask(layer: &Layer, canvas: Rect) -> Option<Surface> {
    if !layer.vector_mask.as_ref().is_some_and(|v| v.enabled) && !has_feather(layer) {
        return None;
    }
    Some(cached_mask(cache(), key(layer, canvas), || (build_mask(layer, canvas), layer.mask.as_ref().map(|m| m.surface.clone()))))
}

/// The dense working set a mask build may allocate: coverage, pixel-mask values and blur scratch,
/// one `f32` plane each over the block being built.
const MAX_MASK_WORKING_BYTES: u64 = 512 << 20;

fn build_mask(layer: &Layer, canvas: Rect) -> Surface {
    build_mask_within(layer, canvas, MAX_MASK_WORKING_BYTES)
}

fn build_mask_within(layer: &Layer, canvas: Rect, budget: u64) -> Surface {
    let vm = layer.vector_mask.as_ref().filter(|v| v.enabled);
    let pixel = layer.mask.as_ref().filter(|m| m.enabled);
    let (sv, sp) = (vm.map_or(0.0, |v| feather_sigma(v.feather)), pixel.map_or(0.0, |m| feather_sigma(m.feather)));
    let compiled = vm.map(photocraft_vector::CompiledVectorMask::new);
    // Far outside the path the vector mask is constant.
    let far = Rect::from_xywh(canvas.x0 - 1_000_000, canvas.y0 - 1_000_000, 1, 1);
    let v_out = compiled.as_ref().map_or(1.0, |vm| vm.render_sequential(far)[0]);
    let p_def = pixel.map_or(1.0, |m| {
        let d = m.surface.default_pixel().first().copied().unwrap_or(1.0);
        1.0 - m.density * (1.0 - d)
    });
    // Where the product can differ from `p_def × v_out`: the path's bounds, plus the pixel
    // mask's painted tiles when the vector mask lets them through outside the path, each grown
    // by its feather.
    let mut area = vm.map_or(Rect::EMPTY, |vm| photocraft_vector::feather::path_area(&vm.path, sv));
    if let Some(m) = pixel
        && v_out > 0.0
    {
        let b = photocraft_vector::feather::grow(m.surface.content_bounds(), sp);
        area = if area.is_empty() {
            b
        } else if b.is_empty() {
            area
        } else {
            area.union(&b)
        };
    }
    let area = area.intersect(&canvas);
    let mut s = Surface::with_default(FORMAT, &[p_def * v_out]);
    if area.is_empty() {
        return s;
    }
    // Bound the transient dense working set: building f32 planes over a whole 50 MP mask would
    // take gigabytes. Over the budget, build square blocks, each blurred with the feather's
    // reach of mask around it, so every block gets the values one dense build would (#17060).
    let samples = budget / (std::mem::size_of::<f32>() as u64 * 3);
    let (w, h) = (u64::from(area.width()), u64::from(area.height()));
    let (step, reach) = if w.saturating_mul(h) <= samples {
        (w.max(h), 0)
    } else {
        let reach = photocraft_vector::feather::reach(sv).max(photocraft_vector::feather::reach(sp)) as u64;
        // Never below 64 px, so a tiny budget still makes progress; the real budget leaves
        // blocks of about 690 px even at the 1000 px feather limit.
        (samples.isqrt().saturating_sub(reach.saturating_mul(2)).max(64), reach)
    };
    let (step, reach) = (i32::try_from(step).unwrap_or(i32::MAX), i32::try_from(reach).unwrap_or(i32::MAX));
    let mut y0 = area.y0;
    while y0 < area.y1 {
        let y1 = y0.saturating_add(step).min(area.y1);
        let mut x0 = area.x0;
        while x0 < area.x1 {
            let x1 = x0.saturating_add(step).min(area.x1);
            let block = Rect::new(x0, y0, x1, y1);
            let context = Rect::new(x0.saturating_sub(reach), y0.saturating_sub(reach), x1.saturating_add(reach), y1.saturating_add(reach)).intersect(&area);
            let v = mask_values(compiled.as_ref(), pixel, context, sv, sp);
            if context == block {
                s.write_region(block, &v);
            } else {
                let (cw, bw) = (context.width() as usize, block.width() as usize);
                let left = (block.x0 - context.x0) as usize;
                let inner: Vec<f32> = (block.y0..block.y1)
                    .flat_map(|y| {
                        let start = (y - context.y0) as usize * cw + left;
                        v.get(start..start + bw).unwrap_or_default().iter().copied()
                    })
                    .collect();
                s.write_region(block, &inner);
            }
            x0 = x1;
        }
        y0 = y1;
    }
    s
}

/// The combined mask values over `r`: vector-mask coverage and pixel-mask values, each blurred by
/// its feather (continued past `r`'s edges with its edge values), multiplied.
fn mask_values(compiled: Option<&photocraft_vector::CompiledVectorMask>, pixel: Option<&photocraft_doc::LayerMask>, r: Rect, sv: f32, sp: f32) -> Vec<f32> {
    let (w, h) = (r.width() as usize, r.height() as usize);
    // A slot initializer must not enter Rayon: another tile requesting the
    // same mask could be stolen onto its stack and wait for its own build.
    let mut v = match compiled {
        Some(vm) => vm.render_sequential(r),
        None => vec![1.0; w * h],
    };
    gaussian(&mut v, w, h, sv);
    if let Some(m) = pixel {
        let mut pm = Vec::new();
        m.values_into(r, &mut pm);
        gaussian(&mut pm, w, h, sp);
        for (a, b) in v.iter_mut().zip(pm) {
            *a *= b;
        }
    }
    v
}

#[cfg(test)]
mod tests;
