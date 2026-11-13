use super::composite::texture_ctx;
use super::ctx::{Ctx, PREPARE_DEPTH};
use super::extents::{effect_outline, effect_shape, effect_source_bounds};
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
    /// What the reserved tile addresses keep allocated. The maps' arrays may be shared with other
    /// entries (a move rebases them), so they are counted per allocation in [`FxCache::arrays`].
    bytes: usize,
    /// The texture anchor the maps were built with ([`texture_ctx`]), for move reuse.
    anchor: (f64, f64),
    /// The layer the maps belong to (tests count builds and bytes per layer).
    #[cfg(test)]
    layer: u64,
}

/// Global cache of effect maps (bounded by bytes), one slot per layer state. Slots are filled
/// with `set`, never `get_or_init`: nothing may block on a build (see `cached_effect_maps`).
type FxSlot = std::sync::Arc<std::sync::OnceLock<FxEntry>>;

struct FxCache {
    map: std::collections::HashMap<u64, FxSlot>,
    /// Position-free index ([`move_key`]) → the exact key of the latest entry with it. Lookups
    /// go through `map`, so an evicted entry is simply a miss.
    moves: std::collections::HashMap<u64, u64>,
    order: std::collections::VecDeque<u64>,
    bytes: usize,
    /// Each cached arrays allocation ([`effects::FxMaps::arrays_id`]) → how many cached entries
    /// share it and its bytes, counted in `bytes` once while any of them is cached. An entry keeps
    /// its arrays alive, so no other allocation can take the address meanwhile.
    arrays: std::collections::HashMap<usize, (usize, usize)>,
    /// `bytes` by layer (tests read their own layer's share while other tests render).
    #[cfg(test)]
    layer_bytes: std::collections::HashMap<u64, usize>,
}

/// Default effect-cache budget (Preferences › Performance can change it).
pub const DEFAULT_FX_CACHE_BUDGET: usize = 768 << 20;

static FX_CACHE_BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(DEFAULT_FX_CACHE_BUDGET);

/// Set the memory budget (bytes) of the layer-effect map cache; entries over it are evicted
/// oldest-first on the next build.
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
    // Tests that read the cache across renders hold this so a parallel test can't purge between.
    #[cfg(test)]
    let _held = FX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let freed = c.bytes;
    c.map.clear();
    c.moves.clear();
    c.order.clear();
    c.arrays.clear();
    c.bytes = 0;
    #[cfg(test)]
    c.layer_bytes.clear();
    freed
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
    C.get_or_init(|| {
        std::sync::Mutex::new(FxCache {
            map: Default::default(),
            moves: Default::default(),
            order: Default::default(),
            bytes: 0,
            arrays: Default::default(),
            #[cfg(test)]
            layer_bytes: Default::default(),
        })
    })
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
    let maps = cached_effect_maps(layer, region, clipped, key, cx);
    cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert((maps, clipped)).0.clone()
}

fn build_fx_slot_maps(
    layer: &Layer,
    region: Rect,
    shape: Option<Vec<f32>>,
    generation: u64,
    key: u64,
    cx: &Ctx,
    slot: &FxSlot,
) -> std::sync::Arc<effects::FxMaps> {
    let shape: Vec<f32> = match shape {
        Some(s) => s,
        None if region.is_empty() => Vec::new(),
        None => effect_shape(layer, region, cx),
    };
    let tex = texture_ctx(layer, region, cx);
    let maps = effects::build_maps_prepared(layer, shape, region, &cx.light, &tex, cx.patterns);
    #[cfg(test)]
    count_build(layer.id.0);
    fill_slot(layer, std::sync::Arc::new(maps), tex.anchor, generation, key, slot)
}

/// Fills `slot` with `maps` and returns the maps the slot holds: a concurrent fill that finished
/// first wins, so every tile shares one map. `generation` is the scratch read-error generation
/// from before the layer's pixels were read: maps whose shape came from a scratch placeholder
/// are returned for this frame but never cached.
fn fill_slot(
    layer: &Layer,
    maps: std::sync::Arc<effects::FxMaps>,
    anchor: (f64, f64),
    generation: u64,
    key: u64,
    slot: &FxSlot,
) -> std::sync::Arc<effects::FxMaps> {
    let mut pin = Vec::new();
    let mut readable = true;
    layer_identity(layer, &mut std::collections::hash_map::DefaultHasher::new(), &mut |t| {
        readable &= t.scratch_health().is_ok();
        pin.push(std::sync::Arc::downgrade(t));
    });
    if !readable || photocraft_raster::spill::check_since(generation).is_err() {
        // Built from a scratch placeholder: use it for this frame, never cache it.
        return maps;
    }
    let bytes = pin.len().saturating_mul(std::mem::size_of::<photocraft_raster::Tile>() + 2 * std::mem::size_of::<usize>());
    #[cfg(test)]
    let entry = FxEntry { maps: maps.clone(), _pin: pin, bytes, anchor, layer: layer.id.0 };
    #[cfg(not(test))]
    let entry = FxEntry { maps: maps.clone(), _pin: pin, bytes, anchor };
    // Filled and counted under the cache lock, so eviction uncounts exactly what was counted:
    // only a fill that finds its slot still cached is counted (eviction may have dropped it).
    let lost = {
        let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        match slot.set(entry) {
            Ok(()) => {
                if c.map.get(&key).is_some_and(|s| std::sync::Arc::ptr_eq(s, slot))
                    && let Some(e) = slot.get()
                {
                    count_entry(&mut c, e);
                }
                None
            }
            Err(e) => Some(e),
        }
    };
    // A losing build's maps are freed outside the lock.
    drop(lost);
    slot.get().map_or(maps, |e| e.maps.clone())
}

/// Adds a cached entry to the byte count: its pins, and its arrays unless another cached entry
/// already counts them.
fn count_entry(c: &mut FxCache, e: &FxEntry) {
    let shared = c.arrays.entry(e.maps.arrays_id()).or_insert((0, e.maps.bytes()));
    shared.0 += 1;
    let added = e.bytes.saturating_add(if shared.0 == 1 { shared.1 } else { 0 });
    c.bytes = c.bytes.saturating_add(added);
    #[cfg(test)]
    {
        *c.layer_bytes.entry(e.layer).or_default() += added;
    }
}

/// The position-free key under which a whole-pixel move may reuse `layer`'s maps over `region`
/// (built elsewhere, at the same size), or `None` where a rebase couldn't be proven exact. It is
/// the GPU's move rule (`fx::shape_key` with an equal-size region): everything the maps are
/// built from except the shape, which [`moved_maps`] compares bit for bit, and the region's
/// origin. Unlike the GPU's key it holds the effects, since these maps are per effect.
///
/// Left out, rebuilt as before: maps that aren't [`effects::maps_are_local`] (their arithmetic
/// depends on where the region starts), a region clipped to the render rectangle (it follows the
/// render, not the layer), groups (their shape is their children's composite), fill layers (laid
/// out over their absolute frame, as on the GPU) and a bevel texture not linked with the layer
/// (it tiles from the canvas origin).
fn move_key(layer: &Layer, region: Rect, clipped: bool, cx: &Ctx) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    if clipped || region.is_empty() || !effects::maps_are_local(layer) {
        return None;
    }
    if matches!(layer.content, LayerContent::Group(_) | LayerContent::Fill(_) | LayerContent::Adjustment(_)) {
        return None;
    }
    if bevel_textures(layer).any(|t| !t.link) {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write_u8(0x4d);
    layer.id.0.hash(&mut h);
    layer.fill_opacity.to_bits().hash(&mut h);
    std::mem::discriminant(&layer.content).hash(&mut h);
    effect_outline(layer).is_some().hash(&mut h);
    // The effects without their reference point: where a linked texture anchors is checked
    // against the move itself (`anchor_follows`).
    format!("{:?}", layer.effects.items).hash(&mut h);
    layer.effects.enabled.hash(&mut h);
    layer.advanced.hash(&mut h);
    layer.blend_if.hash(&mut h);
    (cx.light.angle.to_bits(), cx.light.altitude.to_bits()).hash(&mut h);
    (region.width(), region.height()).hash(&mut h);
    Some(h.finish())
}

/// The enabled bevels' textures: the only map input that reads the texture anchor.
fn bevel_textures(layer: &Layer) -> impl Iterator<Item = &photocraft_doc::BevelTexture> {
    layer.effects.items.iter().filter(|e| e.enabled()).filter_map(|e| match e {
        photocraft_doc::Effect::BevelEmboss(b) => b.texture.as_ref(),
        _ => None,
    })
}

/// Whether every linked bevel texture tiles from an origin that moved by exactly `d`, so each
/// map pixel samples the texture at the same tile coordinates as before. The texture places
/// pixel `x` at `x + 0.5 − (anchor + phase)` (`Placement::anchored`). Both `x` and the origin
/// moved by `d` without rounding, so each difference rounds from the same real number to the
/// same float: that holds only when `old + d` is exact and equals `new`, which the error term of
/// a two-sum checks.
fn anchor_follows(layer: &Layer, old: (f64, f64), new: (f64, f64), d: (i32, i32)) -> bool {
    fn exact_sum(a: f64, b: f64) -> Option<f64> {
        let s = a + b;
        let bb = s - a;
        let err = (a - (s - bb)) + (b - bb);
        (s.is_finite() && err == 0.0).then_some(s)
    }
    bevel_textures(layer).all(|t| {
        let axis = |old: f64, new: f64, phase: f32, d: i32| {
            let (o_old, o_new) = (old + f64::from(phase), new + f64::from(phase));
            exact_sum(o_old, f64::from(d)).is_some_and(|s| s.to_bits() == o_new.to_bits())
        };
        t.link && axis(old.0, new.0, t.phase.0, d.0) && axis(old.1, new.1, t.phase.1, d.1)
    })
}

/// Maps for `layer` over `region` reused from the latest entry with the same [`move_key`]: when
/// that entry covers an equal-size region and the layer's shape over `region` equals the one it
/// was built from bit for bit, every map is the same array at a new origin. Returns the rebased
/// maps with their anchor, or the shape computed for the check (a miss builds from it).
#[allow(clippy::type_complexity)]
fn moved_maps(layer: &Layer, region: Rect, mk: u64, cx: &Ctx) -> Result<(std::sync::Arc<effects::FxMaps>, (f64, f64)), Option<Vec<f32>>> {
    let found = {
        let c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        c.moves.get(&mk).and_then(|k| c.map.get(k)).and_then(|s| s.get()).map(|e| (e.maps.clone(), e.anchor))
    };
    let Some((old, old_anchor)) = found else { return Err(None) };
    let from = old.rect();
    if from.width() != region.width() || from.height() != region.height() {
        return Err(None);
    }
    let d = (region.x0.saturating_sub(from.x0), region.y0.saturating_sub(from.y0));
    let anchor = texture_ctx(layer, region, cx).anchor;
    if !anchor_follows(layer, old_anchor, anchor, d) {
        return Err(None);
    }
    let shape = effect_shape(layer, region, cx);
    let same = shape.len() == old.shape().len() && shape.iter().zip(old.shape()).all(|(a, b)| a.to_bits() == b.to_bits());
    if !same {
        return Err(Some(shape));
    }
    Ok((std::sync::Arc::new(old.rebased((region.x0, region.y0))), anchor))
}

fn evict_fx_cache_over_budget(key: u64) {
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let budget = effect_cache_budget();
    while c.bytes > budget && c.order.len() > 1 {
        let Some(old) = c.order.pop_front() else { break };
        if old == key {
            c.order.push_back(old);
            continue;
        }
        remove_entry(&mut c, old);
    }
    // The current entry alone over the budget: its callers hold the maps, the cache must not.
    if c.bytes > budget && c.map.contains_key(&key) {
        c.order.retain(|k| *k != key);
        remove_entry(&mut c, key);
    }
}

/// Drops the entry under `key` (already out of `order`) from the cache and its bytes from the
/// count: its pins, and its arrays once no other cached entry shares them (a moved entry keeps
/// them alive).
fn remove_entry(c: &mut FxCache, key: u64) {
    if let Some(s) = c.map.remove(&key)
        && let Some(e) = s.get()
    {
        let id = e.maps.arrays_id();
        let mut freed = e.bytes;
        if let Some(shared) = c.arrays.get_mut(&id) {
            shared.0 = shared.0.saturating_sub(1);
            if shared.0 == 0 {
                freed = freed.saturating_add(shared.1);
                c.arrays.remove(&id);
            }
        }
        c.bytes = c.bytes.saturating_sub(freed);
        #[cfg(test)]
        if let Some(b) = c.layer_bytes.get_mut(&e.layer) {
            *b = b.saturating_sub(freed);
        }
    }
    c.moves.retain(|_, k| *k != key);
}

/// The layer's effect maps from the global cache, built on a miss.
///
/// Never waits for another thread's build (#276): this can run on a Rayon worker inside a tile,
/// and the build itself runs Rayon work. A worker waiting on that work may steal another tile of
/// the same layer; had it blocked on a once-init held further up its own stack, it would deadlock.
/// Instead a miss builds the maps here and the first finished build is kept. Renders build every
/// layer's maps before their parallel tiles (`prepare_effects`), so tiles rarely miss.
fn cached_effect_maps(layer: &Layer, region: Rect, clipped: bool, key: u64, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    let slot = {
        let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = c.map.get(&key) {
            s.clone()
        } else {
            let s: FxSlot = Default::default();
            c.map.insert(key, s.clone());
            c.order.push_back(key);
            s
        }
    };
    let maps = match slot.get() {
        Some(e) => e.maps.clone(),
        None => {
            let generation = photocraft_raster::spill::read_error_generation();
            let mk = move_key(layer, region, clipped, cx);
            let maps = match mk.map(|mk| moved_maps(layer, region, mk, cx)) {
                // The arrays are shared with the entry moved from and counted once for both.
                Some(Ok((maps, anchor))) => fill_slot(layer, maps, anchor, generation, key, &slot),
                Some(Err(shape)) => build_fx_slot_maps(layer, region, shape, generation, key, cx, &slot),
                None => build_fx_slot_maps(layer, region, None, generation, key, cx, &slot),
            };
            // Only a filled slot still in the cache can be moved from (one built from unreadable
            // tiles stays empty, and one evicted meanwhile would leave a stale index entry).
            if let Some(mk) = mk
                && slot.get().is_some()
            {
                let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
                if c.map.get(&key).is_some_and(|s| std::sync::Arc::ptr_eq(s, &slot)) {
                    c.moves.insert(mk, key);
                }
            }
            maps
        }
    };
    // Evict the oldest entries over budget (never the one just used).
    evict_fx_cache_over_budget(key);
    maps
}

/// Held by tests that read the effect cache across renders; [`purge_effect_cache`] takes it, so
/// a test purging in parallel waits instead of emptying the cache between their renders.
#[cfg(test)]
pub(crate) static FX_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn build_counts() -> &'static std::sync::Mutex<std::collections::HashMap<u64, usize>> {
    static B: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u64, usize>>> = std::sync::OnceLock::new();
    B.get_or_init(Default::default)
}

#[cfg(test)]
fn count_build(layer: u64) {
    *build_counts().lock().unwrap_or_else(|e| e.into_inner()).entry(layer).or_default() += 1;
}

/// How many times the maps of the layer with id `layer` were built (not reused).
#[cfg(test)]
pub(crate) fn fx_builds(layer: u64) -> usize {
    build_counts().lock().unwrap_or_else(|e| e.into_inner()).get(&layer).copied().unwrap_or(0)
}

/// The cached entries of the layer with id `layer`: their maps and their own bytes (pins).
#[cfg(test)]
pub(crate) fn fx_entries(layer: u64) -> Vec<(std::sync::Arc<effects::FxMaps>, usize)> {
    let c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    c.map.values().filter_map(|s| s.get()).filter(|e| e.layer == layer).map(|e| (e.maps.clone(), e.bytes)).collect()
}

/// The part of [`effect_cache_bytes`] held by the layer with id `layer`.
#[cfg(test)]
pub(crate) fn fx_layer_bytes(layer: u64) -> usize {
    fx_cache().lock().unwrap_or_else(|e| e.into_inner()).layer_bytes.get(&layer).copied().unwrap_or(0)
}

/// Evicts the cached entries of the layer with id `layer` whose maps match `which`, as the
/// budget would.
#[cfg(test)]
pub(crate) fn fx_evict(layer: u64, which: impl Fn(&effects::FxMaps) -> bool) {
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let keys: Vec<u64> = c.map.iter().filter(|(_, s)| s.get().is_some_and(|e| e.layer == layer && which(&e.maps))).map(|(k, _)| *k).collect();
    for k in keys {
        remove_entry(&mut c, k);
        c.order.retain(|o| *o != k);
    }
}
