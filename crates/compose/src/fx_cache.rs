use super::composite::texture_ctx;
use super::ctx::{Ctx, PREPARE_DEPTH};
use super::extents::{effect_outline, effect_shape_with, effect_source_bounds, visible_in_stack};
use super::{BufferAlloc, InfallibleBufferAlloc, Layer, LayerContent, Rect, Surface, advanced, effects};

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
    // The same display surface can blend differently before and after rasterizing a text child.
    std::mem::discriminant(&layer.content).hash(h);
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
        LayerContent::Shape(shape) => {
            layer.surface().map_or(0, |s| surface_fp(s, pin)).hash(h);
            format!("{:?}{:?}{:?}{:?}", shape.path, shape.fill, shape.stroke, shape.live).hash(h);
            shape.use_aligned_rendering.hash(h);
            (shape.density.to_bits(), shape.feather.to_bits()).hash(h);
        }
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

struct FxCache {
    map: std::collections::HashMap<u64, FxSlot>,
    order: std::collections::VecDeque<u64>,
    bytes: usize,
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
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let freed = c.bytes;
    c.map.clear();
    c.order.clear();
    c.bytes = 0;
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
    C.get_or_init(|| std::sync::Mutex::new(FxCache { map: Default::default(), order: Default::default(), bytes: 0 }))
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
    let region = whole_effect_region(layer, cx.canvas, m);
    let clip = *cx.fx_clip.lock().unwrap_or_else(|e| e.into_inner());
    match clip {
        Some(c) if c.contains_rect(&rect) && (advanced::shapeless(layer) || cx.fx_over_budget) && effects::maps_are_local(layer) => {
            let part = region.intersect(&c.inflate(m.saturating_mul(2)));
            (part, part != region)
        }
        _ => (region, false),
    }
}

/// Where `layer`'s unclipped effect maps span: its effect source grown by `margin`, clamped to
/// `canvas` grown by the same `margin`. Returns [`Rect::EMPTY`] when the source or canvas is empty.
pub fn whole_effect_region(layer: &Layer, canvas: Rect, margin: i32) -> Rect {
    let source = effect_source_bounds(layer, canvas);
    if source.is_empty() || canvas.is_empty() {
        return Rect::EMPTY;
    }
    source.inflate(margin).intersect(&canvas.inflate(margin))
}

/// Whether the effect maps of the visible layers of `layers` (groups included), each over its
/// whole region, would take more than the effect cache budget together. A render needs all of
/// them, so when they don't fit, building one evicts the next one needed and every render builds
/// them all again, seconds even for a small rect (#8330). Counted as [`effects::FxMaps::bytes`]
/// counts the built maps ([`effects::map_count`] arrays over each region).
pub(crate) fn fx_over_budget(layers: &[Layer], canvas: Rect) -> bool {
    fx_region_bytes(layers, canvas) > effect_cache_budget() as u64
}

/// Raw whole-region bytes [`fx_over_budget`] compares with the budget. Regions above
/// [`MAX_FX_BUILD_BYTES`] stay included so oversized local maps trigger clipping before the
/// build cap runs.
pub(crate) fn fx_region_bytes(layers: &[Layer], canvas: Rect) -> u64 {
    fn bytes(layers: &[Layer], canvas: Rect, depth: u32) -> u64 {
        let mut total = 0u64;
        for l in layers.iter().filter(|l| l.visible) {
            if effects::has_effects(l) {
                let m = effects::margin(l);
                let region = whole_effect_region(l, canvas, m);
                let maps = effects::map_count(l) as u64;
                let px = region.size().area();
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

/// Worst-case clipped effect-map rectangle built for any horizontal band of height `band_rows`
/// (or the full `render` rectangle when `band_rows <= 0`) intersecting `region`.
fn worst_band_clipped_region(region: Rect, render: Rect, margin: i32, band_rows: i32) -> Rect {
    if region.is_empty() || render.is_empty() {
        return Rect::EMPTY;
    }
    let pad = margin.saturating_mul(2);
    let padded = region.intersect(&render.inflate(pad));
    if padded.is_empty() {
        return Rect::EMPTY;
    }
    if band_rows <= 0 {
        return padded;
    }
    let band_h = u64::from(u32::try_from(band_rows).unwrap_or(u32::MAX)).min(u64::from(render.height()));
    let max_h = band_h.saturating_add(u64::from(pad.max(0) as u32).saturating_mul(2));
    let clipped_h = u64::from(padded.height()).min(max_h).min(u64::from(u32::MAX)) as u32;
    Rect::from_xywh(padded.x0, padded.y0, padded.width(), clipped_h)
}

/// Estimated effect-map working memory needed to render a layer tree over a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectMemoryEstimate {
    /// Peak bytes retained in [`effects::FxMaps`] (whole maps kept across bands plus one band's
    /// clipped maps).
    pub retained_bytes: u64,
    /// Peak transient working bytes allocated while constructing effect shapes (for example, the
    /// full-region RGBA `Buffer` rendered when `transparency_shapes` is enabled).
    pub shape_build_bytes: u64,
}

impl EffectMemoryEstimate {
    /// Total peak effect working bytes (retained maps plus transient shape construction).
    pub fn peak_bytes(self) -> u64 {
        self.retained_bytes.saturating_add(self.shape_build_bytes)
    }
}

/// Maximum concurrent effect-map builds [`super::ctx::prepare_effects_with`] can run at once.
fn effect_prepare_workers() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads().max(1)
    }
}

/// Number of full-region RGBA `Buffer`s simultaneously live while `render_content_with` renders
/// `layer` (not counting nested effect-map builds, which are tracked separately).
fn content_rgba_buffers(layer: &Layer, depth_left: usize) -> u64 {
    match &layer.content {
        LayerContent::Adjustment(_) => 0,
        LayerContent::Group(g) => {
            let mut extra = u64::from(layer.frame.is_some() || g.frame_stroke.is_some());
            if depth_left > 0 {
                extra = extra.max(stack_rgba_buffers(&g.children, depth_left - 1));
            }
            1u64.saturating_add(extra)
        }
        _ => 1,
    }
}

/// Peak simultaneous full-region RGBA `Buffer`s allocated inside `composite_stack_at_with` for
/// `layers` beyond the caller's target backdrop.
fn stack_rgba_buffers(layers: &[Layer], depth_left: usize) -> u64 {
    let floor = u64::from(advanced::needs_floor(layers));
    let mut peak = 0u64;
    let mut i = 0;
    while i < layers.len() {
        let base = &layers[i];
        let mut j = i + 1;
        while j < layers.len() && layers[j].clipped && !base.clipped {
            j += 1;
        }
        if base.visible {
            let clipped = &layers[i + 1..j];
            let has_clipped = clipped.iter().any(|c| c.visible);
            let mut need = u64::from(base.excluded_channels != 0 || !base.blend_if.is_default());
            if base.advanced.knockout != photocraft_doc::Knockout::None {
                need = need.saturating_add(1);
            }
            let child_need = match &base.content {
                LayerContent::Group(g) => {
                    let pass_through = base.frame.is_none()
                        && base.blend == photocraft_color::BlendMode::PassThrough
                        && base.fill_opacity >= 1.0
                        && !effects::has_effects(base)
                        && base.blend_if.is_default();
                    let inner = if depth_left > 0 { stack_rgba_buffers(&g.children, depth_left - 1) } else { 0 };
                    if pass_through {
                        let before = u64::from(base.opacity < 1.0 || base.mask.is_some() || base.vector_mask.is_some() || has_clipped);
                        before.saturating_add(inner)
                    } else {
                        1u64.saturating_add(inner)
                    }
                }
                LayerContent::Fill(_) | LayerContent::Adjustment(_) => 1,
                _ => {
                    let plain_pixel = !effects::has_effects(base)
                        && !has_clipped
                        && base.frame.is_none()
                        && base.advanced.is_default()
                        && base.blend_if.is_default()
                        && base.excluded_channels == 0;
                    u64::from(!plain_pixel)
                }
            };
            need = need.saturating_add(child_need);
            if has_clipped {
                need = need.saturating_add(1);
            }
            peak = peak.max(need);
        }
        i = j.max(i + 1);
    }
    floor.saturating_add(peak)
}

/// Transient working bytes beyond the returned shape `Vec<f32>` (which is already counted in
/// `FxMaps`) allocated by [`effect_shape_with`] while building `src`'s effect shape over `built`.
fn effect_shape_transient_bytes(src: &Layer, built: Rect, depth_left: usize) -> u64 {
    let px = built.size().area();
    if px == 0 {
        return 0;
    }
    let f32_bytes = px.saturating_mul(std::mem::size_of::<f32>() as u64);
    let rgba_bytes = px.saturating_mul(std::mem::size_of::<[f32; 4]>() as u64);
    let mask_count = u64::from(src.mask.is_some()) + u64::from(src.vector_mask.is_some());
    let mask_bytes = f32_bytes.saturating_mul(mask_count);
    if !src.advanced.transparency_shapes {
        // `mask_vals` holds both mask arrays transiently only when pixel and vector masks combine.
        return mask_bytes.saturating_sub(f32_bytes);
    }
    if effect_outline(src).is_some() {
        let surface_phase = if src.surface().is_some() { rgba_bytes.saturating_add(f32_bytes) } else { f32_bytes };
        let outline_phase = f32_bytes.saturating_mul(3).saturating_add(mask_bytes);
        return surface_phase.max(outline_phase).saturating_sub(f32_bytes);
    }
    let rgba_buffers = content_rgba_buffers(src, depth_left);
    if rgba_buffers == 0 {
        return 0;
    }
    rgba_bytes.saturating_mul(rgba_buffers).saturating_add(mask_bytes)
}

/// Shared immutable parameters for [`estimate_layers`].
#[derive(Clone, Copy)]
struct EstimateCtx {
    canvas: Rect,
    render: Rect,
    band_rows: i32,
    over_budget: bool,
}

/// Estimates the peak retained and transient effect-shape working bytes while rendering `layers`
/// over `render` in horizontal bands of `band_rows` (or a single pass when `band_rows <= 0`),
/// descending up to [`photocraft_doc::MAX_GROUP_DEPTH`].
///
/// Visibility follows [`visible_in_stack`] (hidden layers and layers clipped to a hidden base are
/// skipped). Local maps clipped to bands reserve only their worst-case band slice; a styled
/// group's children reserve whole maps only when an active ancestor actually renders its children
/// outside the band to construct its effect shape. Returns `None` when [`MAX_FX_BUILD_BYTES`]
/// would force the renderer to omit any effect map needed for `render`, so callers that require
/// complete effects can refuse before allocating.
pub fn estimated_effect_memory(layers: &[Layer], canvas: Rect, render: Rect, band_rows: i32) -> Option<EffectMemoryEstimate> {
    if render.is_empty() {
        return Some(EffectMemoryEstimate { retained_bytes: 0, shape_build_bytes: 0 });
    }
    let ctx = EstimateCtx { canvas, render, band_rows, over_budget: fx_over_budget(layers, canvas) };
    let mut shape_builds = Vec::new();
    let (retained_bytes, max_nested_chain) = estimate_layers(layers, ctx, false, photocraft_doc::MAX_GROUP_DEPTH, &mut shape_builds)?;
    shape_builds.sort_unstable_by(|a, b| b.cmp(a));
    let parallel_peak = shape_builds.into_iter().take(effect_prepare_workers()).fold(0u64, u64::saturating_add);
    Some(EffectMemoryEstimate { retained_bytes, shape_build_bytes: parallel_peak.max(max_nested_chain) })
}

/// Recursive implementation of [`estimated_effect_memory`].
/// Returns `(retained_bytes, max_nested_shape_chain_bytes)` for `layers`.
fn estimate_layers(layers: &[Layer], ctx: EstimateCtx, in_styled_group: bool, depth_left: usize, shape_builds: &mut Vec<u64>) -> Option<(u64, u64)> {
    let mut retained = 0u64;
    let mut max_chain = 0u64;
    for l in visible_in_stack(layers) {
        let has_fx = effects::has_effects(l);
        let mut own_shape_peak = 0u64;
        let mut parent_renders_children_for_shape = false;
        let mut parent_forces_whole_children = false;
        if has_fx {
            let src = advanced::effects_source(l);
            let m = effects::margin(&src);
            let region = whole_effect_region(&src, ctx.canvas, m);
            let reaches_render = !region.intersect(&ctx.render).is_empty();
            if !region.is_empty() && (in_styled_group || reaches_render) {
                let local = effects::maps_are_local(&src);
                let clips_to_band = local && (advanced::shapeless(&src) || ctx.over_budget);
                let needs_whole = !clips_to_band || in_styled_group;
                let needs_clipped = clips_to_band && reaches_render;
                let mut pixels = 0u64;
                if needs_whole {
                    let built = effect_build_region(&src, region);
                    if built.is_empty() {
                        return None;
                    }
                    pixels = pixels.saturating_add(built.size().area());
                    let shape_bytes = effect_shape_transient_bytes(&src, built, depth_left);
                    if shape_bytes > 0 {
                        shape_builds.push(shape_bytes);
                        own_shape_peak = own_shape_peak.max(shape_bytes);
                    }
                }
                if needs_clipped {
                    let clipped = worst_band_clipped_region(region, ctx.render, m, ctx.band_rows);
                    if !clipped.is_empty() && (!needs_whole || clipped != region) {
                        let built = effect_build_region(&src, clipped);
                        if built.is_empty() {
                            return None;
                        }
                        pixels = pixels.saturating_add(built.size().area());
                        let shape_bytes = effect_shape_transient_bytes(&src, built, depth_left);
                        if shape_bytes > 0 {
                            shape_builds.push(shape_bytes);
                            own_shape_peak = own_shape_peak.max(shape_bytes);
                        }
                    }
                }
                let maps = effects::map_count(&src) as u64;
                retained = retained.saturating_add(maps.saturating_mul(pixels).saturating_mul(std::mem::size_of::<f32>() as u64));
                if src.advanced.transparency_shapes {
                    parent_renders_children_for_shape = true;
                    let clipped_stays_in_band = clips_to_band && (m == 0 || (ctx.band_rows <= 0 && ctx.render.contains_rect(&region)));
                    parent_forces_whole_children = in_styled_group || !clipped_stays_in_band;
                }
            }
        }
        let mut subtree_chain = own_shape_peak;
        if let LayerContent::Group(g) = &l.content
            && depth_left > 0
        {
            let child_in_styled_group = in_styled_group || parent_forces_whole_children;
            let (child_retained, child_chain) = estimate_layers(&g.children, ctx, child_in_styled_group, depth_left - 1, shape_builds)?;
            retained = retained.saturating_add(child_retained);
            if parent_renders_children_for_shape {
                subtree_chain = own_shape_peak.saturating_add(child_chain);
            } else {
                subtree_chain = subtree_chain.max(child_chain);
            }
        }
        max_chain = max_chain.max(subtree_chain);
    }
    Some((retained, max_chain))
}

/// Most bytes one layer's effect maps and source may take to build, whatever the cache budget.
const MAX_FX_BUILD_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 512 << 20 };

/// Applies [`MAX_FX_BUILD_BYTES`] to one layer's build region, returning [`Rect::EMPTY`] when
/// building the maps and shape array would exceed the cap.
fn effect_build_region(layer: &Layer, region: Rect) -> Rect {
    // Non-local effects keep their whole source region, which a few distant pixels stretch over
    // the canvas: past the cap the effects are left out instead of allocating it.
    let region_px = region.size().area();
    if (effects::map_count(layer) as u64 + 1).saturating_mul(region_px).saturating_mul(4) > MAX_FX_BUILD_BYTES { Rect::EMPTY } else { region }
}

/// The layer's effect maps over its region (layer bounds grown by the effect reach, within the
/// canvas grown likewise; see [`effect_region`]), built once per layer state and shared by every
/// tile. `rect` is what the caller composites the layer over (see [`effect_region`]).
pub(crate) fn effect_maps(layer: &Layer, rect: Rect, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    match effect_maps_with(layer, rect, cx, &InfallibleBufferAlloc) {
        Ok(maps) => maps,
        Err(never) => match never {},
    }
}

pub(crate) fn effect_maps_with<A: BufferAlloc>(layer: &Layer, rect: Rect, cx: &Ctx, alloc: &A) -> Result<std::sync::Arc<effects::FxMaps>, A::Error> {
    use std::hash::{Hash, Hasher};
    let (region, clipped) = effect_region(layer, rect, cx);
    let region = effect_build_region(layer, region);
    static NO_CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *NO_CACHE.get_or_init(|| std::env::var_os("PHOTOCRAFT_FX_NOCACHE").is_some()) {
        let shape = if region.is_empty() { Vec::new() } else { effect_shape_with(layer, region, cx, alloc)? };
        return Ok(std::sync::Arc::new(effects::build_maps_prepared(layer, shape, region, &cx.light, &texture_ctx(layer, region, cx), cx.patterns)));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cx.layer_keys.identity(layer, cx.doc).hash(&mut h);
    // Converting a layer can reuse its exact tiles and ID but change how its stroke reads alpha.
    std::mem::discriminant(&layer.content).hash(&mut h);
    (region.x0, region.y0, region.x1, region.y1).hash(&mut h);
    (cx.light.angle.to_bits(), cx.light.altitude.to_bits()).hash(&mut h);
    let key = h.finish();
    // Prepared for this render (see `prepare_effects`): no global lookup, no build.
    if let Some((m, _)) = cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(m.clone());
    }
    let maps = cached_effect_maps_with(layer, region, key, cx, alloc)?;
    Ok(cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert((maps, clipped)).0.clone())
}

fn build_fx_slot_maps_with<A: BufferAlloc>(
    layer: &Layer,
    region: Rect,
    key: u64,
    cx: &Ctx,
    slot: &FxSlot,
    alloc: &A,
) -> Result<std::sync::Arc<effects::FxMaps>, A::Error> {
    let generation = photocraft_raster::spill::read_error_generation();
    let shape: Vec<f32> = if region.is_empty() { Vec::new() } else { effect_shape_with(layer, region, cx, alloc)? };
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
        return Ok(maps);
    }
    if slot.set(FxEntry { maps: maps.clone(), _pin: pin, bytes }).is_ok() {
        // Counted exactly once, by the build that filled the slot, and only while the
        // slot is still cached (eviction may have dropped it meanwhile).
        let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        if c.map.get(&key).is_some_and(|s| std::sync::Arc::ptr_eq(s, slot)) {
            c.bytes += bytes;
        }
    }
    // A concurrent build that finished first wins, so every tile shares one map.
    Ok(slot.get().map_or(maps, |e| e.maps.clone()))
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
        if let Some(s) = c.map.remove(&old) {
            c.bytes = c.bytes.saturating_sub(s.get().map_or(0, |e| e.bytes));
        }
    }
    // The current entry alone over the budget: its callers hold the maps, the cache must not.
    if c.bytes > budget
        && let Some(s) = c.map.remove(&key)
    {
        c.order.retain(|k| *k != key);
        c.bytes = c.bytes.saturating_sub(s.get().map_or(0, |e| e.bytes));
    }
}

/// The layer's effect maps from the global cache, built on a miss.
///
/// Never waits for another thread's build (#276): this can run on a Rayon worker inside a tile,
/// and the build itself runs Rayon work. A worker waiting on that work may steal another tile of
/// the same layer; had it blocked on a once-init held further up its own stack, it would deadlock.
/// Instead a miss builds the maps here and the first finished build is kept. Renders build every
/// layer's maps before their parallel tiles (`prepare_effects`), so tiles rarely miss.
fn cached_effect_maps_with<A: BufferAlloc>(layer: &Layer, region: Rect, key: u64, cx: &Ctx, alloc: &A) -> Result<std::sync::Arc<effects::FxMaps>, A::Error> {
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
        None => match build_fx_slot_maps_with(layer, region, key, cx, &slot, alloc) {
            Ok(maps) => maps,
            Err(error) => {
                // Allocation failures never publish a partial cache entry. If another renderer
                // finished the same slot while this one failed, use that complete result.
                let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = slot.get() {
                    entry.maps.clone()
                } else {
                    if c.map.get(&key).is_some_and(|current| std::sync::Arc::ptr_eq(current, &slot)) {
                        c.map.remove(&key);
                        c.order.retain(|candidate| *candidate != key);
                    }
                    return Err(error);
                }
            }
        },
    };
    // Evict the oldest entries over budget (never the one just used).
    evict_fx_cache_over_budget(key);
    Ok(maps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{BlendMode, Color, PixelFormat};
    use photocraft_doc::{Effect, FxCommon};

    fn styled_layer(blend: BlendMode) -> Layer {
        let mut layer = Layer::raster("styled", PixelFormat::RGBA8);
        layer.advanced.transparency_shapes = false;
        layer.effects.enabled = true;
        layer.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(blend, 1.0), color: Color::BLACK });
        layer
    }

    #[test]
    fn whole_effect_region_is_empty_for_empty_source_or_canvas() {
        let mut empty = Layer::raster("empty", PixelFormat::RGBA8);
        empty.effects.enabled = true;
        empty.effects.items.push(Effect::default_drop_shadow());
        assert_eq!(whole_effect_region(&empty, Rect::new(0, 0, 128, 128), effects::margin(&empty)), Rect::EMPTY);
        let shapeless = styled_layer(BlendMode::Normal);
        assert_eq!(whole_effect_region(&shapeless, Rect::EMPTY, effects::margin(&shapeless)), Rect::EMPTY);
    }

    #[test]
    fn effect_map_admission_skips_clipped_subtrees_with_hidden_bases() {
        let canvas = Rect::new(0, 0, 32, 24);
        let styled = styled_layer(BlendMode::Normal);
        let expected = estimated_effect_memory(std::slice::from_ref(&styled), canvas, canvas, 0).unwrap();
        assert!(expected.retained_bytes > 0);
        assert_eq!(expected.shape_build_bytes, 0, "shapeless layers without dual masks need no extra RGBA shape buffer");
        let mut hidden = Layer::raster("hidden base", PixelFormat::RGBA8);
        hidden.visible = false;
        let mut clipped_group = Layer::group("clipped", vec![styled.clone()]);
        clipped_group.clipped = true;
        let mut layers = vec![hidden, clipped_group];
        assert_eq!(estimated_effect_memory(&layers, canvas, canvas, 0), Some(EffectMemoryEstimate { retained_bytes: 0, shape_build_bytes: 0 }));
        assert_eq!(fx_region_bytes(&layers, canvas), expected.retained_bytes, "raw cache sizing preserves its clipping decision");
        layers.push(styled);
        assert_eq!(estimated_effect_memory(&layers, canvas, canvas, 0), Some(expected), "a new base restores visibility");
    }

    #[test]
    fn group_effect_source_bounds_ignore_children_clipped_to_hidden_bases() {
        let canvas = Rect::new(0, 0, 128, 128);
        let mut visible = Layer::raster("visible", PixelFormat::RGBA8);
        visible.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 20, 20), &[1.0, 0.0, 0.0, 1.0]);
        let mut hidden = Layer::raster("hidden base", PixelFormat::RGBA8);
        hidden.visible = false;
        hidden.surface_mut().unwrap().fill_rect(Rect::new(50, 50, 60, 60), &[0.0, 1.0, 0.0, 1.0]);
        let mut clipped = Layer::raster("clipped far", PixelFormat::RGBA8);
        clipped.clipped = true;
        clipped.surface_mut().unwrap().fill_rect(Rect::new(100, 100, 120, 120), &[0.0, 0.0, 1.0, 1.0]);
        let mut group = Layer::group("styled", vec![visible, hidden, clipped]);
        group.effects.enabled = true;
        group.effects.items.push(Effect::default_drop_shadow());

        assert_eq!(super::super::extents::layer_bounds(&group, canvas), Rect::new(10, 10, 20, 20));
        assert_eq!(effect_source_bounds(&group, canvas), Rect::new(10, 10, 20, 20));
        let margin = effects::margin(&group);
        assert_eq!(whole_effect_region(&group, canvas, margin), Rect::new(10, 10, 20, 20).inflate(margin).intersect(&canvas.inflate(margin)));
    }

    #[test]
    fn effect_map_admission_applies_the_build_limit_per_region_and_band() {
        let canvas = Rect::new(0, 0, 300_000, 300_000);
        let render = Rect::new(100, 100, 116, 116);
        let local = styled_layer(BlendMode::Normal);
        let layers = std::slice::from_ref(&local);
        let whole = whole_effect_region(&local, canvas, effects::margin(&local));
        assert_eq!(effect_build_region(&local, whole), Rect::EMPTY);
        assert!(fx_region_bytes(layers, canvas) > MAX_FX_BUILD_BYTES, "raw sizing must still trigger clipping");
        assert_eq!(estimated_effect_memory(layers, canvas, canvas, 0), None);
        // Banded rendering clips local maps to one band plus twice the margin, fitting within the cap.
        let band_rows = 256;
        let band_clipped = worst_band_clipped_region(whole, canvas, effects::margin(&local), band_rows);
        assert_eq!(
            estimated_effect_memory(layers, canvas, canvas, band_rows),
            Some(EffectMemoryEstimate { retained_bytes: band_clipped.size().area() * 4, shape_build_bytes: 0 })
        );
        let clipped = whole.intersect(&render.inflate(2 * effects::margin(&local)));
        assert_eq!(
            estimated_effect_memory(layers, canvas, render, 0),
            Some(EffectMemoryEstimate { retained_bytes: clipped.size().area() * 4, shape_build_bytes: 0 })
        );

        // Non-local effects always build their whole region, even when banded or rendered over a small rect.
        let non_local = styled_layer(BlendMode::Dissolve);
        assert!(!effects::maps_are_local(&non_local));
        assert_eq!(estimated_effect_memory(std::slice::from_ref(&non_local), canvas, render, band_rows), None);
    }

    #[test]
    fn effect_map_admission_gates_whole_child_maps_on_parent_shape_rendering() {
        let canvas = Rect::new(0, 0, 300_000, 300_000);
        let local = styled_layer(BlendMode::Normal);
        // Outside the render rectangle, a top-level layer builds no maps and does not trigger refusal.
        let disjoint = Rect::new(400_000, 400_000, 400_016, 400_016);
        assert_eq!(
            estimated_effect_memory(std::slice::from_ref(&local), canvas, disjoint, 256),
            Some(EffectMemoryEstimate { retained_bytes: 0, shape_build_bytes: 0 })
        );

        let render = Rect::new(100, 100, 116, 116);
        // A styled group with `transparency_shapes = true` and a non-zero margin renders its children
        // past the band (`render.inflate(2 * margin)`) to build its shape, requesting the child's whole map.
        let mut shaped_parent = Layer::group("shaped styled parent", vec![local.clone()]);
        shaped_parent.effects.enabled = true;
        shaped_parent.effects.items.push(Effect::default_drop_shadow());
        assert_eq!(estimated_effect_memory(std::slice::from_ref(&shaped_parent), canvas, render, 256), None);

        // With `transparency_shapes = false`, the parent builds its shape from masks alone and never
        // composites its children outside the band, so a local-effect child stays clipped to the band.
        let mut shapeless_parent = Layer::group("shapeless styled parent", vec![local.clone()]);
        shapeless_parent.advanced.transparency_shapes = false;
        shapeless_parent.effects.enabled = true;
        shapeless_parent.effects.items.push(Effect::default_drop_shadow());
        let estimate = estimated_effect_memory(std::slice::from_ref(&shapeless_parent), canvas, render, 256).unwrap();
        assert!(estimate.retained_bytes > 0);
        assert_eq!(estimate.shape_build_bytes, 0);

        // A styled group whose own effect region does not intersect `render` never builds its shape either.
        assert_eq!(
            estimated_effect_memory(std::slice::from_ref(&shaped_parent), canvas, disjoint, 256),
            Some(EffectMemoryEstimate { retained_bytes: 0, shape_build_bytes: 0 })
        );
    }

    #[test]
    fn effect_memory_includes_transient_rgba_shape_buffers() {
        let canvas = Rect::new(0, 0, 128, 128);
        let mut layer = Layer::raster("shaped", PixelFormat::RGBA8);
        layer.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 20, 20), &[1.0, 0.0, 0.0, 1.0]);
        layer.effects.enabled = true;
        layer.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color: Color::BLACK });
        let estimate = estimated_effect_memory(std::slice::from_ref(&layer), canvas, canvas, 0).unwrap();
        let region_px = whole_effect_region(&layer, canvas, effects::margin(&layer)).size().area();
        assert_eq!(estimate.retained_bytes, region_px * 4);
        assert_eq!(estimate.shape_build_bytes, region_px * 16);
    }

    #[test]
    fn effect_map_build_limit_accepts_its_exact_boundary() {
        let layer = styled_layer(BlendMode::Normal);
        let pixels = MAX_FX_BUILD_BYTES / ((effects::map_count(&layer) as u64 + 1) * 4);
        let at_limit = Rect::new(0, 0, pixels as i32, 1);
        assert_eq!(effect_build_region(&layer, at_limit), at_limit);
        assert_eq!(effect_build_region(&layer, Rect::new(0, 0, pixels as i32 + 1, 1)), Rect::EMPTY);
    }
}
