use super::filters::smart;
use super::source::{
    SourceImage, cache_get, cache_key, cache_put, convert_svg_source, source_bytes, source_surface_from_buffer, svg_source_image_in, vector_scale,
};
use super::{
    Affine, Arc, BlendMode, Document, EngineError, Homography, JobCtx, Layer, LayerContent, LayerId, PixelFormat, Rect, Result, SmartFilter, SmartObject,
    SmartSource, Stage, StageKey, Surface, Value, affine_homography, other, placement, stack_cache,
};
use std::collections::VecDeque;
use std::sync::{Mutex, Weak};

/// Layer › Smart Objects › Stack Mode: the source's layers (or, when it holds a single group, the
/// group's layers) combined per pixel with `mode`. Cached like [`super::source_image`], keyed by mode.
pub fn stack_image(file_name: &str, bytes: &[u8], fmt: PixelFormat, mode: photocraft_doc::StackMode) -> Result<SourceImage> {
    stack_image_classified(file_name, bytes, fmt, mode, None)
}

fn stack_image_classified(file_name: &str, bytes: &[u8], fmt: PixelFormat, mode: photocraft_doc::StackMode, svg: Option<bool>) -> Result<SourceImage> {
    use photocraft_algo::stack::Stat;
    let mut key = cache_key(bytes, fmt);
    // Keep the filename in the key because fallback import uses it to choose a format.
    // Use a separate domain from entries that predate plain SVG completeness checks.
    key.0 = *blake3::hash(&[&key.0[..], b"stack-source-v3:", blake3::hash(file_name.as_bytes()).as_bytes(), mode.id().as_bytes()].concat()).as_bytes();
    if let Some(img) = cache_get(&key) {
        return Ok(img);
    }
    let svg = svg.unwrap_or_else(|| photocraft_io::svg::is_svg(bytes));
    let doc = super::decode_source_classified(file_name, bytes, svg)?;
    let layers: &[Layer] = match doc.layers.as_slice() {
        [only] if only.is_group() => only.children().unwrap_or_default(),
        all => all,
    };
    let bounds = doc.bounds();
    // An SVG opens with one layer per shape, and every visible one gets its own float frame:
    // they may take 1 GiB together, the SVG import's own budget, before any is rendered.
    if svg {
        let visible = u64::try_from(layers.iter().filter(|l| l.visible).count()).unwrap_or(u64::MAX);
        let frame_bytes = doc.size.area().saturating_mul(16);
        if visible.saturating_mul(frame_bytes) > 1 << 30 {
            return Err(EngineError::Other("the SVG has too many shapes for Stack Mode at this size".into()));
        }
    }
    let frames: Vec<Vec<[f32; 4]>> = layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| {
            let mut one = l.clone();
            one.blend = BlendMode::Normal;
            one.clipped = false;
            photocraft_compose::render_layer(&one, bounds).into_px()
        })
        .collect();
    if frames.is_empty() {
        return Err(EngineError::Other("the smart object's contents have no visible layers".into()));
    }
    let stat = match mode {
        photocraft_doc::StackMode::Entropy => Stat::Entropy,
        photocraft_doc::StackMode::Kurtosis => Stat::Kurtosis,
        photocraft_doc::StackMode::Maximum => Stat::Maximum,
        photocraft_doc::StackMode::Mean => Stat::Mean,
        photocraft_doc::StackMode::Median => Stat::Median,
        photocraft_doc::StackMode::Minimum => Stat::Minimum,
        photocraft_doc::StackMode::Range => Stat::Range,
        photocraft_doc::StackMode::Skewness => Stat::Skewness,
        photocraft_doc::StackMode::StandardDeviation => Stat::StandardDeviation,
        photocraft_doc::StackMode::Summation => Stat::Summation,
        photocraft_doc::StackMode::Variance => Stat::Variance,
    };
    let buf = photocraft_compose::Buffer { rect: bounds, px: photocraft_algo::stack::combine(&frames, stat) };
    let img = SourceImage { surface: Arc::new(source_surface_from_buffer(&doc, &buf, fmt)?), bounds };
    cache_put(key, img.clone());
    Ok(img)
}

// ---------- rendering ----------

/// Mix a filter's result with its input, without stacking the same pixel coverage twice.
pub(super) fn blend_surfaces(base: &Surface, top: &Surface, mode: BlendMode, opacity: f32) -> Surface {
    if opacity <= 0.0 {
        return base.clone();
    }
    let fmt = base.format();
    let area = base.content_bounds().union(&top.content_bounds());
    let mut out = base.clone();
    if area.is_empty() {
        return out;
    }
    let n = fmt.channels();
    let (b, t) = (base.read_region(area), top.read_region(area));
    // Alpha-preserving filters edit the same coverage. A non-Normal filter that
    // changes coverage (e.g. blur) must also retain input-only pixels and admit
    // filtered-only pixels, as Photoshop's Smart Filter blend oracles require.
    // Classify the whole result so unchanged pixels do not switch blend rules
    // independently at the edge of a blur.
    let composite_coverage = mode != BlendMode::Normal && fmt.alpha && b.chunks_exact(n).zip(t.chunks_exact(n)).any(|(bp, tp)| bp.last() != tp.last());
    let mut o = vec![0.0f32; b.len()];
    for ((bp, tp), op) in b.chunks_exact(n).zip(t.chunks_exact(n)).zip(o.chunks_exact_mut(n)) {
        let b = photocraft_raster::to_rgba(&fmt, bp);
        let t = photocraft_raster::to_rgba(&fmt, tp);
        if composite_coverage {
            let rgba = photocraft_color::blend::composite(mode, b, t, opacity);
            photocraft_raster::from_rgba_into(&fmt, rgba, op);
            continue;
        }
        let cb = [b[0], b[1], b[2]];
        let ct = [t[0], t[1], t[2]];
        let color = if fmt.sample == photocraft_color::SampleType::F32 {
            photocraft_color::blend::blend_rgb_hdr(mode, cb, ct)
        } else {
            photocraft_color::blend::blend_rgb(mode, cb, ct)
        };
        // A filter edits this surface, rather than adding a second source-over layer. Mix
        // premultiplied results so alpha-preserving filters keep coverage at every opacity,
        // while Normal-mode filters that change coverage interpolate it too.
        let a = b[3] + (t[3] - b[3]) * opacity;
        let mut rgba = [0.0, 0.0, 0.0, a];
        if a > 0.0 {
            for c in 0..3 {
                rgba[c] = (cb[c] * b[3] * (1.0 - opacity) + color[c] * t[3] * opacity) / a;
            }
        }
        photocraft_raster::from_rgba_into(&fmt, rgba, op);
    }
    out.write_region(area, &o);
    out.prune();
    out
}

/// Runs the smart filter stack on the placed (unfiltered) content. A filter the session's
/// authority denies surfaces its error (the trusted compositor path has no gate installed, so
/// this reads ambiently only when the caller established one and it refuses).
pub fn apply_smart_filters(placed: &Surface, sm: &SmartObject, canvas: Rect) -> Result<Surface> {
    if !sm.filters_enabled || !sm.smart_filters.iter().any(|f| f.visible) {
        return Ok(placed.clone());
    }
    let mut cur = placed.clone();
    // Edge pixels repeat at the canvas ∪ placed-content edge, as for layer filters.
    let extent = canvas.union(&placed.content_bounds());
    for f in sm.smart_filters.iter().filter(|f| f.visible) {
        cur = apply_smart_filter(f, sm, &cur, filter_bounds(f, placed, canvas), extent, &JobCtx::new())?;
    }
    Ok(match sm.filter_mask.as_ref().filter(|m| m.enabled) {
        Some(m) => photocraft_compose::filter_mask::mix(placed, &cur, m),
        None => cur,
    })
}

/// The reference bounds of smart filter `f`. Render filters work within the smart object, as
/// Photoshop's do: Lens Flare's centre is a fraction of the object, and Clouds and Fibers fill it
/// and size their pattern by it. The object is its placed content's bounds (which leaves out
/// fully transparent edges of the source); the other filters use the canvas, as on a layer.
fn filter_bounds(f: &SmartFilter, placed: &Surface, canvas: Rect) -> Rect {
    match placed.content_bounds() {
        b if renders_in_object(f) && !b.is_empty() => b,
        _ => canvas,
    }
}

fn renders_in_object(f: &SmartFilter) -> bool {
    matches!(f.command.as_str(), "filter.render.clouds" | "filter.render.differenceClouds" | "filter.render.fibers" | "filter.render.lensFlare")
}

/// One visible smart filter over `cur`, blended with its blend mode and opacity. `Err` when the
/// requesting session's authority denies the filter's params: callers surface the error instead of
/// rendering (and caching) unfiltered pixels.
fn apply_smart_filter(f: &SmartFilter, sm: &SmartObject, cur: &Surface, canvas: Rect, extent: Rect, ctx: &JobCtx) -> Result<Surface> {
    // Equalize bypasses `apply_filter_to_surface_in` (the gate there), so check the command first.
    crate::jobs::authorize_render_command(&f.command, &f.params)?;
    let params = crate::distort_cmds::placed_puppet_params(&f.command, &f.params, sm);
    let rendered = if f.command == "image.adjustments.equalize" {
        ctx.check()?;
        crate::adjust_cmds::adjust_as_filter("equalize", &params, cur, sm.filter_mask.as_ref())?
    } else {
        crate::filters::apply_filter_to_surface_in(&f.command, &params, cur, canvas, None, extent, ctx)?
    };
    let out = match rendered {
        // Unknown ids (e.g. Photoshop filters we don't implement) leave the pixels alone.
        Some(out) => out,
        None => cur.clone(),
    };
    if f.blend == BlendMode::Normal && f.opacity >= 1.0 { Ok(out) } else { Ok(blend_surfaces(cur, &out, f.blend, f.opacity.clamp(0.0, 1.0))) }
}

/// The source placed in document pixels, before any smart filter (uncached test oracle).
#[cfg(test)]
pub(super) fn place(name: &str, bytes: &[u8], doc: &Document, sm: &SmartObject) -> Result<Surface> {
    place_with_svg(name, bytes, doc, sm, photocraft_io::svg::is_svg(bytes), &mut |bytes, scale| svg_source_image_in(bytes, doc, scale))
}

fn place_with_svg(
    name: &str,
    bytes: &[u8],
    doc: &Document,
    sm: &SmartObject,
    is_svg: bool,
    svg: &mut impl FnMut(&[u8], f64) -> Result<SourceImage>,
) -> Result<Surface> {
    let fmt = doc.pixel_format();
    // A vector source is rendered at the placement's scale, and the placement is divided by
    // that scale so the rendered pixels land where the source units would.
    if sm.stack_mode.is_none() && sm.warp.is_none() && is_svg {
        let k = vector_scale(sm);
        let img = svg(bytes, k)?;
        return Ok(match &sm.perspective {
            Some(p) => {
                let [h0, h1, h2, h3, h4, h5, h6, h7, h8] = *p;
                let h = Homography([h0 / k, h1 / k, h2, h3 / k, h4 / k, h5, h6 / k, h7 / k, h8]);
                crate::transform_cmds::check_budget(img.bounds, &h, "smart object", fmt)?;
                photocraft_algo::warp::place_source_projective(&img.surface, img.bounds, &h, None)?
            }
            None => {
                let [a, b, c, d, e, f] = sm.transform.m;
                let t = Affine { m: [a / k, b / k, c / k, d / k, e, f] };
                crate::transform_cmds::check_budget(img.bounds, &affine_homography(&t), "smart object", fmt)?;
                photocraft_algo::warp::place_source(&img.surface, img.bounds, &t, None)?
            }
        });
    }
    let img = match sm.stack_mode {
        Some(mode) if is_svg => convert_svg_source(stack_image_classified(name, bytes, PixelFormat::RGBA32F, mode, Some(is_svg))?, doc)?,
        None if is_svg => svg(bytes, 1.0)?,
        Some(mode) => stack_image_classified(name, bytes, fmt, mode, Some(is_svg))?,
        None => super::source::source_image_classified(name, bytes, fmt, is_svg)?,
    };
    // The placed stage is unclipped: a large enough placement (an image scaled to fill a huge
    // frame) would otherwise fill memory before any allocation could fail.
    let h = placement(sm);
    crate::transform_cmds::check_budget(img.bounds, &h, "smart object", fmt)?;
    // Through the warp (source space) and the transform in one pass; whole-pixel moves are exact.
    Ok(match &sm.perspective {
        Some(_) => photocraft_algo::warp::place_source_projective(&img.surface, img.bounds, &h, sm.warp.as_ref())?,
        None => photocraft_algo::warp::place_source(&img.surface, img.bounds, &sm.transform, sm.warp.as_ref())?,
    })
}

#[cfg(test)]
thread_local! { pub(crate) static DISPLAY_NATIVE_HASH_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// BLAKE3 digests of native vector sources, remembered per source allocation so explicit
/// refreshes and warp source frames of shared contents hash a (up to 16 MiB) source once.
///
/// Entries hold a [`Weak`](std::sync::Weak), never a strong reference: the source buffer is freed
/// as soon as the document drops it, and only the small `Arc` header stays allocated until the
/// entry is pruned. That header pinning is what makes the address a sound identity: no other
/// `Arc` can be allocated there while the entry exists, and an `Arc` with a live `Weak` cannot be
/// mutated in place (`Arc::get_mut` fails, `Arc::make_mut` moves to a new allocation), so a
/// remembered digest always describes the bytes at that address. At most [`Self::CAPACITY`]
/// entries are kept, least recently used first out, and dead entries are pruned on every lookup.
pub(super) struct NativeDigests {
    pub(super) entries: Vec<(std::sync::Weak<Vec<u8>>, [u8; 32])>,
    capacity: usize,
}

impl NativeDigests {
    const CAPACITY: usize = 32;

    /// An empty cache holding at most `capacity` digests.
    pub(super) const fn new(capacity: usize) -> Self {
        Self { entries: Vec::new(), capacity }
    }

    /// The remembered digest of exactly this allocation, refreshing its recency.
    pub(super) fn get(&mut self, bytes: &Arc<Vec<u8>>) -> Option<[u8; 32]> {
        self.entries.retain(|(weak, _)| weak.strong_count() > 0);
        let index = self.entries.iter().position(|(weak, _)| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(bytes)))?;
        let entry = self.entries.remove(index);
        let digest = entry.1;
        self.entries.push(entry);
        Some(digest)
    }

    /// Remember `digest` (computed from `bytes`), evicting the least recently used entry.
    pub(super) fn insert(&mut self, bytes: &Arc<Vec<u8>>, digest: [u8; 32]) {
        if self.get(bytes).is_some() || self.capacity == 0 {
            return;
        }
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push((Arc::downgrade(bytes), digest));
    }
}

static NATIVE_DIGESTS: Mutex<NativeDigests> = Mutex::new(NativeDigests::new(NativeDigests::CAPACITY));

/// The BLAKE3 digest of a native source, hashed only the first time this allocation is seen.
/// The lock is not held while hashing, so other renders never wait on a 16 MiB hash.
fn native_digest(bytes: &Arc<Vec<u8>>) -> [u8; 32] {
    let lock = || NATIVE_DIGESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(digest) = lock().get(bytes) {
        return digest;
    }
    #[cfg(test)]
    DISPLAY_NATIVE_HASH_BYTES.with(|count| count.set(count.get().saturating_add(bytes.len())));
    let digest = *blake3::hash(bytes).as_bytes();
    lock().insert(bytes, digest);
    digest
}

/// Preview data is untrusted after reload. Keep it on the bounded SVG path and
/// never reuse it after a source replacement, even if a command retained the field.
pub(super) fn display_source(sm: &SmartObject, bytes: &Arc<Vec<u8>>) -> Arc<Vec<u8>> {
    sm.vector_preview
        .as_ref()
        .filter(|preview| {
            if bytes.len() > 16 * 1024 * 1024 || !photocraft_io::vector_preview::is_native_source(bytes) {
                return false;
            }
            preview.source_hash == native_digest(bytes)
                && !preview.svg.is_empty()
                && preview.svg.len() <= 16 * 1024 * 1024
                && photocraft_io::svg::is_svg(&preview.svg)
        })
        .map_or_else(|| bytes.clone(), |preview| preview.svg.clone())
}

/// The box of a smart object's source in source pixels (what its warp's `us`/`vs` span), or `None`
/// when the source can't be read.
///
/// An SVG (or a native source's SVG preview) without Stack Mode is rendered by the bounded
/// whole-drawing path, so its box only needs the canvas bounds: no editable layer caches are
/// built, and a drawing whose layer caches exceed the budget still gets a warp frame. Stack Mode
/// keeps the strict layered import it renders with.
pub(crate) fn source_box(doc: &Document, sm: &SmartObject) -> Option<[f64; 4]> {
    let (name, bytes) = source_bytes(&doc.metadata, &sm.source)?;
    let bytes = display_source(sm, &bytes);
    let fmt = doc.pixel_format();
    let r = match sm.stack_mode {
        None if photocraft_io::svg::is_svg(&bytes) => photocraft_io::svg::bounded_canvas_bounds(&bytes, 24_000_000).ok()?,
        Some(mode) => stack_image(&name, &bytes, fmt, mode).ok()?.bounds,
        None => super::source::source_image(&name, &bytes, fmt).ok()?.bounds,
    };
    Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)])
}

/// Renders a smart object from its source. `Ok(None)` when the source is unavailable (a missing
/// linked file, or a PSD placed layer without embedded data): callers keep the existing cache.
pub fn render(doc: &Document, sm: &SmartObject) -> Result<Option<Surface>> {
    render_in(doc, sm, &JobCtx::new())
}

/// Command-only rendering that retains usable SVG artwork and reports omitted embedded images.
/// Its local source cache never enters the strict process-wide source/filter caches.
pub(crate) fn render_with_warnings(doc: &Document, sm: &SmartObject) -> Result<(Option<Surface>, Vec<String>)> {
    render_with_warnings_using(sm, &mut WarningRenderSources::new(doc))
}

struct WarningSource {
    source: Weak<Vec<u8>>,
    name: [u8; 32],
    scale: u64,
    image: SourceImage,
    warnings: Vec<String>,
    bytes: usize,
}

/// One command's bounded source-space renders. Placement, warp, filters and masks
/// are instance-specific and must never be cached here. Weak identity pins only the
/// allocation header; the borrowed document owns encoded bytes throughout the command.
pub(crate) struct WarningRenderSources<'a> {
    document: &'a Document,
    srgb: Option<bool>,
    entries: VecDeque<WarningSource>,
    budget: usize,
    limit: usize,
    bytes: usize,
}

#[cfg(test)]
thread_local! {
    pub(crate) static WARNING_SOURCE_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static WARNING_SOURCE_LIMIT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

impl<'a> WarningRenderSources<'a> {
    pub(crate) fn new(document: &'a Document) -> Self {
        #[cfg(test)]
        let limit = WARNING_SOURCE_LIMIT.with(|limit| limit.get().unwrap_or(32));
        #[cfg(not(test))]
        let limit = 32;
        Self { document, srgb: None, entries: VecDeque::new(), budget: if cfg!(target_arch = "wasm32") { 128 << 20 } else { 384 << 20 }, limit, bytes: 0 }
    }

    #[cfg(test)]
    pub(crate) fn with_limits(document: &'a Document, budget: usize, limit: usize) -> Self {
        Self { budget, limit, ..Self::new(document) }
    }

    #[cfg(test)]
    pub(crate) fn profile_initialized(&self) -> bool {
        self.srgb.is_some()
    }

    fn image(&mut self, bytes: &Arc<Vec<u8>>, scale: f64, name: [u8; 32]) -> Result<(SourceImage, Vec<String>)> {
        let doc = self.document;
        // Non-SVG and unavailable sources keep their old fast path without
        // materializing ICC bytes. This immutable document needs the context once.
        let key = scale.to_bits();
        if let Some(index) =
            self.entries.iter().position(|entry| entry.scale == key && entry.name == name && std::ptr::eq(entry.source.as_ptr(), Arc::as_ptr(bytes)))
            && let Some(entry) = self.entries.remove(index)
        {
            let result = (entry.image.clone(), entry.warnings.clone());
            self.entries.push_back(entry);
            return Ok(result);
        }
        // Only the exact sRGB fast path needs this lazy, immutable document context.
        // Pixel format/profile do not belong in each entry's key: the document is borrowed.
        let srgb = *self.srgb.get_or_insert_with(|| {
            doc.mode == photocraft_color::ColorMode::Rgb
                && crate::color_cmds::document_profile(doc).to_bytes().as_ref() == craft_cms::Builtin::Srgb.profile().to_bytes().as_ref()
        });
        #[cfg(test)]
        WARNING_SOURCE_DECODES.with(|count| count.set(count.get().saturating_add(1)));
        let (buf, warnings) = photocraft_io::svg::rasterize_bounded_with_warnings(bytes, scale as f32, 24_000_000)
            .map_err(|e| other(format!("can't render the SVG smart object: {e}")))?;
        // Charge owned native tiles, entry and warning storage, not borrowed encoded bytes.
        // This is a soft memo budget: oversize entries retain the existing uncached path.
        let cost = usize::try_from(photocraft_raster::tile_storage_bytes(buf.rect, &doc.pixel_format()))
            .unwrap_or(usize::MAX)
            .saturating_add(std::mem::size_of::<WarningSource>())
            .saturating_add(warnings.capacity().saturating_mul(std::mem::size_of::<String>()))
            .saturating_add(warnings.iter().fold(0usize, |total, warning| total.saturating_add(warning.capacity())));
        let admitted = self.limit != 0 && cost <= self.budget;
        if admitted {
            while self.entries.len() >= self.limit || self.bytes > self.budget - cost {
                let Some(entry) = self.entries.pop_front() else {
                    // Recover conservatively if accounting ever outlives an empty queue.
                    self.bytes = 0;
                    break;
                };
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        // Preserve the normal renderer's exact built-in sRGB fast path.
        // A full memo is optional. Let the placement proceed uncached if the
        // browser cannot afford retaining these tiles along with prior renders.
        let admitted = admitted && crate::memory_gate::ensure("retain the SVG source for this update", cost as u64).is_ok();
        let image = if srgb {
            SourceImage { surface: Arc::new(super::source::buffer_to_surface(&buf, doc.pixel_format())), bounds: buf.rect }
        } else {
            let source = SourceImage { surface: Arc::new(super::source::buffer_to_surface(&buf, PixelFormat::RGBA32F)), bounds: buf.rect };
            convert_svg_source(source, doc)?
        };
        if admitted && self.entries.try_reserve(1).is_ok() {
            self.bytes += cost; // cost was admitted against budget - bytes above.
            self.entries.push_back(WarningSource {
                source: Arc::downgrade(bytes),
                name,
                scale: key,
                image: image.clone(),
                warnings: warnings.clone(),
                bytes: cost,
            });
        }
        Ok((image, warnings))
    }
}

/// Reuse only warning-aware source pixels during one shared-content update.
pub(crate) fn render_with_warnings_using(sm: &SmartObject, sources: &mut WarningRenderSources<'_>) -> Result<(Option<Surface>, Vec<String>)> {
    let doc = sources.document;
    if sm.vector_preview.is_some() || sm.stack_mode.is_some() {
        return render(doc, sm).map(|surface| (surface, Vec::new()));
    }
    let Some((name, bytes)) = source_bytes(&doc.metadata, &sm.source) else { return Ok((None, Vec::new())) };
    if !photocraft_io::svg::is_svg(&bytes) {
        // Keep the same source snapshot for fallback, including authorized linked reads.
        return render_in_with_source(doc, sm, &JobCtx::new(), Some((name, bytes))).map(|surface| (surface, Vec::new()));
    }
    let name_key = *blake3::hash(name.as_bytes()).as_bytes();
    if !unapplied_filters(sm).is_empty() {
        return Err(other("the smart object has unsupported visible filters"));
    }
    let mut warnings = Vec::new();
    let placed = place_with_svg(&name, &bytes, doc, sm, true, &mut |_, scale| {
        let (image, omitted) = sources.image(&bytes, scale, name_key)?;
        warnings.extend(omitted);
        Ok(image)
    })?;
    Ok((Some(apply_smart_filters(&placed, sm, doc.bounds())?), warnings))
}

/// [`render`], reporting progress per smart filter to `ctx` and stopping between filters once it
/// is cancelled. Gives the same pixels as [`apply_smart_filters`] on the placed source, but
/// starts from the deepest stage of the stack already rendered (see [`stack_cache`]).
pub(super) fn render_in(doc: &Document, sm: &SmartObject, ctx: &JobCtx) -> Result<Option<Surface>> {
    render_in_with_source(doc, sm, ctx, None)
}

fn render_in_with_source(doc: &Document, sm: &SmartObject, ctx: &JobCtx, snapshot: Option<(String, Arc<Vec<u8>>)>) -> Result<Option<Surface>> {
    // A trusted UI render may have populated these stages. Check authority even on a hit.
    for filter in sm.smart_filters.iter().filter(|f| sm.filters_enabled && f.visible) {
        crate::jobs::authorize_render_command(&filter.command, &filter.params)?;
    }
    let Some((name, original)) = snapshot.or_else(|| source_bytes(&doc.metadata, &sm.source)) else { return Ok(None) };
    let bytes = display_source(sm, &original);
    let (fmt, canvas) = (doc.pixel_format(), doc.bounds());
    // Plain SVG uses the original strict vector path. Gzip sources use ordinary
    // file import and native-size pixels, as existing saved objects expect.
    let is_svg = photocraft_io::svg::is_svg(&bytes);
    let mut h = blake3::Hasher::new();
    h.update(b"smart-placement-v3:");
    h.update(&stack_cache::placed_key(&bytes, fmt, sm, canvas));
    h.update(blake3::hash(name.as_bytes()).as_bytes());
    if is_svg {
        h.update(&crate::color_cmds::document_profile(doc).to_bytes());
    }
    let placed_key: StageKey = *h.finalize().as_bytes();
    let placed = || -> Result<Stage> {
        if let Some(stage) = stack_cache::get(&placed_key) {
            return Ok(stage);
        }
        let surface = place_with_svg(&name, &bytes, doc, sm, is_svg, &mut |bytes, scale| svg_source_image_in(bytes, doc, scale))?;
        let stage = Stage { extent: canvas.union(&surface.content_bounds()), surface };
        stack_cache::put(placed_key, &stage);
        Ok(stage)
    };
    let filters: Vec<&SmartFilter> = sm.smart_filters.iter().filter(|f| sm.filters_enabled && f.visible).collect();
    if filters.is_empty() {
        return Ok(Some(placed()?.surface));
    }
    let keys: Vec<StageKey> = filters
        .iter()
        .scan(placed_key, |key, f| {
            *key = stack_cache::filter_key(key, f, sm.filter_mask.as_ref());
            Some(*key)
        })
        .collect();
    let deepest = keys.iter().enumerate().rev().find_map(|(i, key)| stack_cache::get(key).map(|stage| (i + 1, stage)));
    let (done, mut cur) = match deepest {
        Some(hit) => hit,
        None => (0, placed()?),
    };
    let n = filters.len() as f32;
    // The object's bounds, found once: the placed stage may have been evicted from the cache, and
    // placing it again for every render filter would decode the source each time.
    let mut object_bounds: Option<Rect> = None;
    for (i, (f, key)) in filters.iter().zip(&keys).enumerate().skip(done) {
        ctx.check()?;
        ctx.progress(i as f32 / n, "");
        let bounds = match object_bounds {
            _ if !renders_in_object(f) => canvas,
            Some(b) => b,
            None => *object_bounds.insert(filter_bounds(f, &placed()?.surface, canvas)),
        };
        cur = Stage { surface: apply_smart_filter(f, sm, &cur.surface, bounds, cur.extent, ctx)?, extent: cur.extent };
        stack_cache::put(*key, &cur);
    }
    ctx.progress(1.0, "");
    Ok(Some(match sm.filter_mask.as_ref().filter(|m| m.enabled) {
        Some(m) => photocraft_compose::filter_mask::mix(&placed()?.surface, &cur.surface, m),
        None => cur.surface,
    }))
}

/// Bytes held by rendered smart filter stages.
pub fn stack_cache_bytes() -> usize {
    stack_cache::bytes()
}

/// Drop every rendered smart filter stage (Edit › Purge › All). Returns the bytes released.
pub fn purge_stack_cache() -> usize {
    stack_cache::purge()
}

/// The smart object's pixels below smart filter `index` (the input that filter edits): the placed
/// source and the filters under it, without the filter mask. `Ok(None)` when the source is
/// unavailable.
pub fn render_below_filter(doc: &Document, sm: &SmartObject, index: usize) -> Result<Option<Surface>> {
    let mut below = sm.clone();
    below.smart_filters.truncate(index);
    below.filter_mask = None;
    render(doc, &below)
}

/// Re-renders smart layer `l` (which lives in `doc`) in place. Returns false if its source is
/// unavailable or a visible filter can't be applied (the cache is then left alone).
pub fn refresh_layer(doc: &Document, l: &mut Layer) -> Result<bool> {
    if matches!(&l.content, LayerContent::Smart(sm) if !unapplied_filters(sm).is_empty()) {
        return Ok(false);
    }
    refresh_layer_in(doc, l, &JobCtx::new())
}

pub(super) fn unapplied_filters(sm: &SmartObject) -> Vec<String> {
    sm.smart_filters
        .iter()
        .filter(|f| sm.filters_enabled && f.visible && crate::commands::find(&f.command).is_none())
        .map(|f| f.params.get("name").and_then(Value::as_str).unwrap_or(&f.command).to_string())
        .collect()
}

fn refresh_layer_in(doc: &Document, l: &mut Layer, ctx: &JobCtx) -> Result<bool> {
    let LayerContent::Smart(sm) = &mut l.content else { return Ok(false) };
    match render_in(doc, sm, ctx)? {
        Some(px) => {
            sm.cache = Some(px);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Re-renders the smart object `id` of `doc`.
pub fn refresh(doc: &mut Document, id: LayerId) -> Result<bool> {
    refresh_in(doc, id, &JobCtx::new())
}

fn refresh_in(doc: &mut Document, id: LayerId, ctx: &JobCtx) -> Result<bool> {
    let mut l = doc.layer(id).cloned().ok_or(EngineError::NoLayer(id))?;
    if !matches!(l.content, LayerContent::Smart(_)) {
        return Err(other("the layer is not a smart object"));
    }
    let ok = refresh_layer_in(doc, &mut l, ctx)?;
    if ok && let Some(dst) = doc.layer_mut(id) {
        dst.content = l.content;
    }
    Ok(ok)
}

pub(super) fn refresh_or_fail(doc: &mut Document, id: LayerId) -> Result<()> {
    refresh_or_fail_in(doc, id, &JobCtx::new())
}

pub(super) fn refresh_or_fail_in(doc: &mut Document, id: LayerId, ctx: &JobCtx) -> Result<()> {
    if refresh_in(doc, id, ctx)? {
        return Ok(());
    }
    if let SmartSource::Linked { path } = &smart(doc, id)?.source
        && let Err(why @ crate::doc_path::NotFollowed::Refused(_)) = crate::doc_path::resolve(path)
    {
        return Err(crate::doc_path::refusal("the linked file", path, why));
    }
    Err(other("the smart object's contents are unavailable (missing linked file?), so it can't be re-rendered"))
}

#[cfg(test)]
#[path = "tests/source_work.rs"]
mod source_work_tests;
