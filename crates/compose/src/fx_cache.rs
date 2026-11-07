use super::composite::texture_ctx;
use super::ctx::{Ctx, PREPARE_DEPTH};
use super::extents::{effect_shape, effect_source_bounds};
use super::{Layer, LayerContent, Rect, Surface, advanced, effects};

// ---------------------------------------------------------------------------------------------
// Layer-effect map cache

/// Order-independent identity of a layer's pixels, masks, effects, Advanced Blending and Blend If
/// and (for groups) children.
/// Pixels are identified by their copy-on-write tile pointers. `pin` sees every tile hashed: a
/// cache entry keeps a [`std::sync::Weak`] to each, which reserves the tile's address (so no
/// other tile can reuse it and match the key) without keeping its pixels alive (#4530).
pub(crate) fn layer_identity(layer: &Layer, h: &mut std::collections::hash_map::DefaultHasher, pin: &mut dyn FnMut(&std::sync::Arc<photocraft_raster::Tile>)) {
    use std::hash::{Hash, Hasher};
    fn surface_fp(s: &Surface, pin: &mut dyn FnMut(&std::sync::Arc<photocraft_raster::Tile>)) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        s.format().hash(&mut h);
        // Missing tiles read the default pixel, so tile identity alone is not pixel identity.
        for sample in s.default_pixel() {
            sample.to_bits().hash(&mut h);
        }
        let tiles = s.tiles().fold(s.tile_count() as u64, |acc, (c, t)| {
            pin(t);
            let mut x = (std::sync::Arc::as_ptr(t) as usize as u64) ^ ((c.tx as u64) << 40) ^ ((c.ty as u32 as u64) << 8);
            x = x.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            acc.wrapping_add(x ^ (x >> 29))
        });
        tiles.hash(&mut h);
        h.finish()
    }
    layer.id.0.hash(h);
    // What the maps depend on: `fill_opacity` shades them; layer `opacity` is applied when the
    // maps are composited and `visible` only skips the lookup entirely, so neither belongs in
    // the key (an opacity drag must not rebuild shadows over the whole layer; the GPU's
    // `fx::shape_key` keeps them out for the same reason).
    layer.fill_opacity.to_bits().hash(h);
    match &layer.content {
        LayerContent::Group(g) => {
            for c in &g.children {
                // A styled group's maps come from its children's composite, so how each child
                // composites (unlike the group's own visibility and opacity) shapes them.
                (c.visible, c.opacity.to_bits(), c.clipped).hash(h);
                format!("{:?}", c.blend).hash(h);
                layer_identity(c, h, pin);
            }
            format!("{:?}{:?}", g.artboard, g.frame_stroke).hash(h);
            format!("{:?}", layer.frame).hash(h);
        }
        LayerContent::Fill(f) => {
            format!("{f:?}").hash(h);
            // The cached pixels are what shapes the maps while they match the fill: a
            // pixel-only rewrite of the cache (Image › Transform, crop) leaves `f` alone, and
            // the stale maps would keep shadowing the pre-rewrite shape.
            if let Some(c) = &layer.fill_cache
                && c.fill == *f
            {
                surface_fp(&c.surface, pin).hash(h);
            }
        }
        LayerContent::Adjustment(a) => format!("{a:?}").hash(h),
        _ => layer.surface().map_or(0, |s| surface_fp(s, pin)).hash(h),
    }
    if let Some(m) = &layer.mask {
        (surface_fp(&m.surface, pin), m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(h);
    }
    if let Some(vm) = &layer.vector_mask {
        format!("{vm:?}").hash(h);
    }
    format!("{:?}", layer.effects).hash(h);
    // Transparency Shapes Layer picks the shape (content or the whole layer); a group child's
    // Blend If and knockout change the composite a styled group's shape comes from.
    layer.advanced.hash(h);
    layer.blend_if.hash(h);
    h.write_u8(0xfe);
}

struct FxEntry {
    maps: std::sync::Arc<effects::FxMaps>,
    /// The tiles the key identifies (see [`layer_identity`]): their addresses, not their pixels.
    _pin: Vec<std::sync::Weak<photocraft_raster::Tile>>,
    bytes: usize,
}

/// Global cache of effect maps (bounded by bytes), one slot per layer state. Slots are filled
/// with `set`, never `get_or_init`: nothing may block on a build (see `cached_effect_maps`).
type FxSlot = std::sync::Arc<std::sync::OnceLock<FxEntry>>;

#[derive(Default)]
struct FxCache {
    map: std::collections::HashMap<u64, FxSlot>,
    order: Recency,
    bytes: usize,
}

impl FxCache {
    /// The slot of `key`, made the most recently used: the cached one, or a new empty one.
    fn slot(&mut self, key: u64) -> FxSlot {
        let s = self.map.entry(key).or_default().clone();
        self.order.touch(key);
        s
    }

    /// Counts `bytes` for `slot`, just filled by a build, while it is still the slot of `key`
    /// (eviction may have dropped it meanwhile).
    fn admit(&mut self, key: u64, slot: &FxSlot, bytes: usize) {
        if self.map.get(&key).is_some_and(|s| std::sync::Arc::ptr_eq(s, slot)) {
            self.bytes += bytes;
        }
    }

    /// Drops the least recently used entries until the cache is within `budget`, never `keep`
    /// (the one a render is using now), so a single entry over the budget stays.
    fn evict_over(&mut self, budget: usize, keep: u64) {
        while self.bytes > budget && self.order.len() > 1 {
            let Some(old) = self.order.oldest_except(keep) else { break };
            self.order.remove(old);
            if let Some(s) = self.map.remove(&old) {
                self.bytes = self.bytes.saturating_sub(s.get().map_or(0, |e| e.bytes));
            }
        }
    }

    /// Drops every entry. Returns the bytes released.
    fn purge(&mut self) -> usize {
        self.map.clear();
        self.order.clear();
        std::mem::take(&mut self.bytes)
    }
}

/// Default effect-cache budget (Preferences › Performance can change it).
pub const DEFAULT_FX_CACHE_BUDGET: usize = 768 << 20;

static FX_CACHE_BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(DEFAULT_FX_CACHE_BUDGET);

/// Set the memory budget (bytes) of the layer-effect map cache; entries over it are evicted
/// least recently used first on the next build.
pub fn set_effect_cache_budget(bytes: usize) {
    FX_CACHE_BUDGET.store(bytes.max(1 << 20), std::sync::atomic::Ordering::Relaxed);
}

/// The current effect-cache budget in bytes.
pub fn effect_cache_budget() -> usize {
    FX_CACHE_BUDGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bytes currently held by the effect-map cache.
pub fn effect_cache_bytes() -> usize {
    fx_cache().lock().unwrap_or_else(|e| e.into_inner()).bytes
}

/// Drop every cached effect map (Edit › Purge › All). Returns the bytes released.
pub fn purge_effect_cache() -> usize {
    fx_cache().lock().unwrap_or_else(|e| e.into_inner()).purge()
}

/// Discard derived pixels and their source pins once after each new scratch read failure.
/// This lets closing the affected document release its failed tiles instead of keeping them
/// in a cache. Caches never admit results built from unreadable tiles, so healthy documents'
/// caches are not purged again on every call while one document stays unreadable.
pub fn discard_unreadable_pixel_caches() {
    static SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let generation = photocraft_raster::spill::read_error_generation();
    if SEEN.swap(generation, std::sync::atomic::Ordering::AcqRel) != generation && photocraft_raster::spill::any_failed() {
        purge_effect_cache();
        super::bounds::purge();
        super::masks::purge();
        super::shape_split::purge();
    }
}

fn fx_cache() -> &'static std::sync::Mutex<FxCache> {
    static C: std::sync::OnceLock<std::sync::Mutex<FxCache>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(FxCache::default()))
}

/// Where `layer`'s effect maps are built (see [`effect_maps`]) and whether that is clipped to the
/// rectangle being rendered rather than the whole region.
///
/// Transparency Shapes Layer off makes the region the whole canvas whatever the layer holds: on a
/// 1920 × 103675 webtoon page each such layer's maps took about 2.4 GB, and a render held them for
/// every layer at once (60 GB with 25 stroked type layers, #1909). When its effects are local
/// ([`effects::maps_are_local`]), such a layer's maps are built over the render rectangle padded
/// by twice the effect reach instead, which gives the same pixels there. So are any layer's local
/// maps when the render's whole-region maps wouldn't fit in the effect cache ([`fx_over_budget`]).
/// Only when `rect`, what
/// the caller composites the layer over, lies within the render rectangle: a group's effect shape
/// (or anything else rendering children over a wider area) gets the whole region.
pub(crate) fn effect_region(layer: &Layer, rect: Rect, cx: &Ctx) -> (Rect, bool) {
    let m = effects::margin(layer);
    let region = effect_source_bounds(layer, cx.canvas).inflate(m).intersect(&cx.canvas.inflate(m));
    let clip = *cx.fx_clip.lock().unwrap_or_else(|e| e.into_inner());
    match clip {
        Some(c) if c.contains_rect(&rect) && (advanced::shapeless(layer) || cx.fx_over_budget) && effects::maps_are_local(layer) => {
            let part = region.intersect(&c.inflate(m.saturating_mul(2)));
            (part, part != region)
        }
        _ => (region, false),
    }
}

/// Whether the effect maps of the visible layers of `layers` (groups included), each over its
/// whole region, would take more than the effect cache budget together. A render needs all of
/// them, so when they don't fit, building one evicts the next one needed and every render builds
/// them all again, seconds even for a small rect (#8330). Counted as [`effects::FxMaps::bytes`]
/// counts the built maps ([`effects::map_count`] arrays over each region).
pub(crate) fn fx_over_budget(layers: &[Layer], canvas: Rect) -> bool {
    fx_region_bytes(layers, canvas) > effect_cache_budget() as u64
}

/// The bytes [`fx_over_budget`] compares with the budget.
pub(crate) fn fx_region_bytes(layers: &[Layer], canvas: Rect) -> u64 {
    fn bytes(layers: &[Layer], canvas: Rect, depth: u32) -> u64 {
        let mut total = 0u64;
        for l in layers.iter().filter(|l| l.visible) {
            if effects::has_effects(l) {
                let m = effects::margin(l);
                let region = effect_source_bounds(l, canvas).inflate(m).intersect(&canvas.inflate(m));
                let maps = effects::map_count(l) as u64;
                let px = u64::from(region.width()) * u64::from(region.height());
                total = total.saturating_add(maps.saturating_mul(px).saturating_mul(4));
            }
            if let LayerContent::Group(g) = &l.content
                && depth < PREPARE_DEPTH
            {
                total = total.saturating_add(bytes(&g.children, canvas, depth + 1));
            }
        }
        total
    }
    bytes(layers, canvas, 0)
}

/// Most bytes one layer's effect maps and source may take to build, whatever the cache budget.
const MAX_FX_BUILD_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 512 << 20 };

/// The layer's effect maps over its region (layer bounds grown by the effect reach, within the
/// canvas grown likewise; see [`effect_region`]), built once per layer state and shared by every
/// tile. `rect` is what the caller composites the layer over (see [`effect_region`]).
pub(crate) fn effect_maps(layer: &Layer, rect: Rect, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    use std::hash::{Hash, Hasher};
    let (mut region, clipped) = effect_region(layer, rect, cx);
    // Non-local effects keep their whole source region, which a few distant pixels stretch over
    // the canvas: past the cap the effects are left out instead of allocating it.
    let region_px = u64::from(region.width()) * u64::from(region.height());
    if (effects::map_count(layer) as u64 + 1).saturating_mul(region_px).saturating_mul(4) > MAX_FX_BUILD_BYTES {
        region = Rect::EMPTY;
    }
    static NO_CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *NO_CACHE.get_or_init(|| std::env::var_os("PHOTOCRAFT_FX_NOCACHE").is_some()) {
        let shape = if region.is_empty() { Vec::new() } else { effect_shape(layer, region, cx) };
        return std::sync::Arc::new(effects::build_maps_prepared(layer, shape, region, &cx.light, &texture_ctx(layer, region, cx), cx.patterns));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cx.layer_keys.identity(layer, cx.doc).hash(&mut h);
    (region.x0, region.y0, region.x1, region.y1).hash(&mut h);
    (cx.light.angle.to_bits(), cx.light.altitude.to_bits()).hash(&mut h);
    let key = h.finish();
    // Prepared for this render (see `prepare_effects`): no global lookup, no build.
    if let Some((m, _)) = cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return m.clone();
    }
    let maps = cached_effect_maps(layer, region, key, cx);
    cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert((maps, clipped)).0.clone()
}

fn build_fx_slot_maps(layer: &Layer, region: Rect, key: u64, cx: &Ctx, slot: &FxSlot) -> std::sync::Arc<effects::FxMaps> {
    let generation = photocraft_raster::spill::read_error_generation();
    let shape: Vec<f32> = if region.is_empty() { Vec::new() } else { effect_shape(layer, region, cx) };
    let maps = effects::build_maps_prepared(layer, shape, region, &cx.light, &texture_ctx(layer, region, cx), cx.patterns);
    let mut pin = Vec::new();
    let mut readable = true;
    layer_identity(layer, &mut std::collections::hash_map::DefaultHasher::new(), &mut |t| {
        readable &= t.scratch_health().is_ok();
        pin.push(std::sync::Arc::downgrade(t));
    });
    // The maps, plus what each reserved tile address keeps allocated.
    let bytes = maps.bytes().saturating_add(pin.len().saturating_mul(std::mem::size_of::<photocraft_raster::Tile>() + 2 * std::mem::size_of::<usize>()));
    let maps = std::sync::Arc::new(maps);
    if !readable || photocraft_raster::spill::check_since(generation).is_err() {
        // Built from a scratch placeholder: use it for this frame, never cache it.
        return maps;
    }
    if slot.set(FxEntry { maps: maps.clone(), _pin: pin, bytes }).is_ok() {
        // Counted exactly once, by the build that filled the slot.
        fx_cache().lock().unwrap_or_else(|e| e.into_inner()).admit(key, slot, bytes);
    }
    // A concurrent build that finished first wins, so every tile shares one map.
    slot.get().map_or(maps, |e| e.maps.clone())
}

fn evict_fx_cache_over_budget(key: u64) {
    fx_cache().lock().unwrap_or_else(|e| e.into_inner()).evict_over(effect_cache_budget(), key);
}

/// The layer's effect maps from the global cache, built on a miss.
///
/// Never waits for another thread's build (#276): this can run on a Rayon worker inside a tile,
/// and the build itself runs Rayon work. A worker waiting on that work may steal another tile of
/// the same layer; had it blocked on a once-init held further up its own stack, it would deadlock.
/// Instead a miss builds the maps here and the first finished build is kept. Renders build every
/// layer's maps before their parallel tiles (`prepare_effects`), so tiles rarely miss.
fn cached_effect_maps(layer: &Layer, region: Rect, key: u64, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    let slot = fx_cache().lock().unwrap_or_else(|e| e.into_inner()).slot(key);
    let maps = match slot.get() {
        Some(e) => e.maps.clone(),
        None => build_fx_slot_maps(layer, region, key, cx, &slot),
    };
    // Evict the least recently used entries over budget (never the one just used).
    evict_fx_cache_over_budget(key);
    maps
}

/// When each cached key was last used, for least-recently-used eviction: a hit only bumps a
/// counter (renders hit the cache for every layer with effects), and eviction, which runs over
/// budget only, looks for the oldest.
#[derive(Default)]
struct Recency {
    tick: u64,
    last: std::collections::HashMap<u64, u64>,
}

impl Recency {
    /// Marks `key` the most recently used.
    fn touch(&mut self, key: u64) {
        self.tick += 1;
        self.last.insert(key, self.tick);
    }

    fn remove(&mut self, key: u64) {
        self.last.remove(&key);
    }

    fn clear(&mut self) {
        self.last.clear();
    }

    fn len(&self) -> usize {
        self.last.len()
    }

    /// The least recently used key other than `keep` (the one a render is using now).
    fn oldest_except(&self, keep: u64) -> Option<u64> {
        self.last.iter().filter(|&(&k, _)| k != keep).min_by_key(|&(_, &t)| t).map(|(&k, _)| k)
    }
}

#[cfg(test)]
mod tests {
    use super::{FxCache, FxEntry, Recency};
    use crate::{Layer, LayerContent, Rect, Surface, effects};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    /// Each test builds its own cache: the global one is shared by every test running at once.
    /// `fill` goes through the same steps as a miss in `cached_effect_maps`: take the slot, fill
    /// it as `build_fx_slot_maps` does, count it, evict over `budget`.
    fn fill(c: &mut FxCache, key: u64, bytes: usize, budget: usize) {
        let slot = c.slot(key);
        let layer = Layer::new("fx", LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true))));
        let tex = effects::TextureCtx { rect: Rect::EMPTY, patterns: &[], anchor: (0.0, 0.0) };
        let maps = std::sync::Arc::new(effects::build_maps(&layer, Vec::new(), Rect::EMPTY, &photocraft_doc::GlobalLight::default(), &tex));
        assert!(slot.set(FxEntry { maps, _pin: Vec::new(), bytes }).is_ok());
        c.admit(key, &slot, bytes);
        c.evict_over(budget, key);
    }

    fn hit(c: &mut FxCache, key: u64, budget: usize) {
        assert!(c.slot(key).get().is_some(), "{key} is cached");
        c.evict_over(budget, key);
    }

    fn keys(c: &FxCache) -> Vec<u64> {
        let mut k: Vec<u64> = c.map.keys().copied().collect();
        k.sort_unstable();
        k
    }

    #[test]
    fn a_hit_keeps_an_entry_from_eviction() {
        let mut c = FxCache::default();
        for k in [1, 2, 3] {
            fill(&mut c, k, 10, 30);
        }
        hit(&mut c, 1, 30);
        fill(&mut c, 4, 10, 30);
        assert_eq!(keys(&c), [1, 3, 4], "2, not 1 (just used), is the least recently used");
        assert_eq!(c.bytes, 30);
        assert_eq!(c.order.len(), 3);
    }

    #[test]
    fn eviction_drops_as_many_entries_as_it_takes_and_counts_their_bytes() {
        let mut c = FxCache::default();
        for k in [1, 2, 3, 4] {
            fill(&mut c, k, 10, 100);
        }
        fill(&mut c, 5, 25, 40);
        assert_eq!(keys(&c), [4, 5], "1, 2 and 3 go in one call");
        assert_eq!((c.bytes, c.order.len()), (35, 2));
    }

    #[test]
    fn the_entry_in_use_is_never_evicted_even_when_oldest_or_alone() {
        let mut c = FxCache::default();
        fill(&mut c, 1, 50, 10);
        assert_eq!((keys(&c), c.bytes), (vec![1], 50), "a single entry over the budget stays");
        fill(&mut c, 2, 5, 100);
        // 1 is the oldest, but it is the one in use: 2 goes, and 1 stays over the budget.
        hit(&mut c, 1, 10);
        assert_eq!((keys(&c), c.bytes, c.order.len()), (vec![1], 50, 1));
    }

    #[test]
    fn purging_clears_the_recency_with_the_entries() {
        let mut c = FxCache::default();
        for k in [1, 2] {
            fill(&mut c, k, 10, 100);
        }
        assert_eq!(c.purge(), 20);
        assert_eq!((c.map.len(), c.order.len(), c.bytes), (0, 0, 0));
        // Nothing left behind: a later fill over the budget has nothing older to evict.
        fill(&mut c, 3, 10, 5);
        assert_eq!((keys(&c), c.order.len()), (vec![3], 1));
    }

    #[test]
    fn a_slot_dropped_before_its_build_finished_is_not_counted() {
        let mut c = FxCache::default();
        let slot = c.slot(1);
        c.purge();
        c.admit(1, &slot, 10);
        assert_eq!(c.bytes, 0);
    }

    /// Eviction takes the oldest first: maps used by every render (layers a render server never
    /// changes) must outlive maps of layers that changed and will not be asked for again.
    #[test]
    fn a_cache_hit_makes_the_entry_the_newest() {
        let mut r = Recency::default();
        for k in [1, 2, 3] {
            r.touch(k);
        }
        assert_eq!(r.oldest_except(3), Some(1));
        r.touch(1);
        assert_eq!(r.oldest_except(3), Some(2), "1 was just used");
        r.remove(2);
        assert_eq!(r.oldest_except(1), Some(3));
        assert_eq!(r.oldest_except(3), Some(1));
        r.touch(1);
        assert_eq!(r.len(), 2, "a hit doesn't add a second entry");
        r.clear();
        assert_eq!(r.oldest_except(0), None);
    }
}
