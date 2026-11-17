//! One-time bounded capture of ordinary native contents for static native resources.
//! No source operation names, graph syntax, external reads or document rules live here.
use super::coverage::{self, NativeFilterDomain};
use super::native_work::Work;
use super::{IoError, Result, invalid};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::collections::HashMap;

struct Admission {
    remaining: usize,
    source: usize,
    nodes: usize,
    work: Work,
}
impl Admission {
    /// Debits the capture allowance with checked arithmetic before native preparation allocates additional
    /// data.
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(|| invalid("native snapshot exceeds its remaining capture allowance"))?;
        Ok(())
    }
}
/// Propagates an interrupted native capture as the importer's cancellation error.
fn check(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Return owner-local native pixels. Admission and the caller's budget publish
/// only after the entire capture succeeds; original encoded contents are immutable.
#[cfg(test)]
pub(super) fn capture(bytes: &[u8], target: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    capture_with_work(bytes, target, budget, &Work::default(), ctl)
}

pub(super) fn capture_with_work(bytes: &[u8], target: &Document, budget: &mut usize, work: &Work, ctl: &Interrupt<'_>) -> Result<Surface> {
    check(ctl)?;
    let mut admission =
        Admission { remaining: (*budget).min(super::MAX_PIXEL_BYTES), source: super::MAX_PIXEL_BYTES, nodes: super::MAX_LAYERS, work: work.clone() };
    let (document, surface) = render_bytes(bytes, 0, &mut admission, ctl)?;
    if document.size != target.size {
        return Err(invalid("native snapshot current contents frame differs from its validated owner"));
    }
    charge_frame(work, document.bounds(), target.pixel_format(), 3, ctl)?;
    let pixels = project_source(&surface, &document, target.pixel_format(), &mut admission.remaining, ctl)?;
    check(ctl)?;
    *budget = admission.remaining;
    Ok(pixels)
}

/// RFC 8878 section 3.1.1.1 (https://www.rfc-editor.org/rfc/rfc8878):
/// inspect at most the 18-byte frame header.
/// This is an admission claim only; the real loader still validates the frame.
struct FrameHeader {
    decoded: Option<u64>,
    window: u64,
    length: usize,
    checksum: bool,
    dictionary: u64,
}

fn frame_header(header: &[u8]) -> Result<FrameHeader> {
    if header.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Err(invalid("native snapshot object has no ordinary zstd frame header"));
    }
    let descriptor = *header.get(4).ok_or_else(|| invalid("native snapshot zstd descriptor is truncated"))?;
    if descriptor & 8 != 0 {
        return Err(invalid("native snapshot zstd reserved flag is set"));
    }
    let single = descriptor & 32 != 0;
    let mut at = 5usize;
    let window = if !single {
        let value = *header.get(at).ok_or_else(|| invalid("native snapshot zstd window is truncated"))?;
        at += 1;
        let base = 1u64.checked_shl(10 + u32::from(value >> 3)).ok_or_else(|| invalid("native snapshot zstd window overflows"))?;
        Some(base + (base / 8) * u64::from(value & 7))
    } else {
        None
    };
    let dictionary_width = match descriptor & 3 {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let end = at.checked_add(dictionary_width).ok_or_else(|| invalid("native snapshot zstd header overflows"))?;
    let mut dictionary = [0; 8];
    dictionary
        .get_mut(..dictionary_width)
        .ok_or_else(|| invalid("native dictionary width"))?
        .copy_from_slice(header.get(at..end).ok_or_else(|| invalid("native snapshot zstd dictionary id is truncated"))?);
    at = end;
    let count = match descriptor >> 6 {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let end = at.checked_add(count).ok_or_else(|| invalid("native snapshot zstd size range overflows"))?;
    let raw = header.get(at..end).ok_or_else(|| invalid("native snapshot zstd content size is truncated"))?;
    let mut bytes = [0; 8];
    bytes.get_mut(..count).ok_or_else(|| invalid("native snapshot zstd size width is invalid"))?.copy_from_slice(raw);
    let value = u64::from_le_bytes(bytes);
    let decoded = if count == 0 { None } else { Some(if count == 2 { value + 256 } else { value }) };
    Ok(FrameHeader {
        decoded,
        window: window.unwrap_or(decoded.unwrap_or(0)),
        length: end,
        checksum: descriptor & 4 != 0,
        dictionary: u64::from_le_bytes(dictionary),
    })
}

#[cfg(test)]
fn zstd_decoded_size(header: &[u8]) -> Result<Option<u64>> {
    Ok(frame_header(header)?.decoded)
}

/// RFC 8878 section 3.1.1.2: raw/RLE output lengths are explicit;
/// compressed blocks produce at most min(window,128 KiB). Scan framing only,
/// never entropy-decode. The pinned writer omits FCS, so this bounded upper
/// bound is required for ordinary small native embedded-source blobs too.
fn frame_bound(encoded: &[u8], memory: u64, work: &Work, ctl: &Interrupt<'_>) -> Result<(u64, u64)> {
    if encoded.get(..4).is_some_and(|magic| magic.first().is_some_and(|byte| byte & 0xf0 == 0x50) && magic.get(1..) == Some(&[0x2a, 0x4d, 0x18])) {
        return work.refuse_unpriced("optional native snapshot does not price skippable zstd frames", ctl);
    }
    let header = frame_header(encoded)?;
    if header.dictionary != 0 {
        return work.refuse_unpriced("optional native snapshot does not price dictionary-dependent zstd frames", ctl);
    }
    if header.window > memory || header.window > work.remaining_bytes() {
        return work.refuse_unpriced("native snapshot zstd window exceeds available capture/work admission", ctl);
    }
    let block_max = header.window.min(128 * 1024);
    let mut at = header.length;
    let mut upper = 0u64;
    loop {
        work.charge_bytes(32, ctl)?;
        let end = at.checked_add(3).ok_or(IoError::XcfNativeWorkLimit)?;
        let bytes = encoded.get(at..end).ok_or_else(|| invalid("native snapshot zstd block header is truncated"))?;
        let mut word_bytes = [0; 4];
        word_bytes.get_mut(..3).ok_or_else(|| invalid("native block header width"))?.copy_from_slice(bytes);
        let word = u32::from_le_bytes(word_bytes);
        at = end;
        let size = u64::from(word >> 3);
        if size > block_max {
            return Err(invalid("native snapshot zstd block exceeds its declared window/128 KiB limit"));
        }
        let (stored, output) = match (word >> 1) & 3 {
            0 => (size, size),
            1 => (1, size),
            2 => (size, block_max),
            _ => return Err(invalid("native snapshot zstd block uses reserved type")),
        };
        let end = at
            .checked_add(usize::try_from(stored).map_err(|_| invalid("native snapshot block length exceeds platform"))?)
            .ok_or(IoError::XcfNativeWorkLimit)?;
        encoded.get(at..end).ok_or_else(|| invalid("native snapshot zstd block is truncated"))?;
        at = end;
        upper = upper.checked_add(output).ok_or(IoError::XcfNativeWorkLimit)?;
        if word & 1 != 0 {
            break;
        }
    }
    if header.checksum {
        let end = at.checked_add(4).ok_or(IoError::XcfNativeWorkLimit)?;
        encoded.get(at..end).ok_or_else(|| invalid("native snapshot zstd checksum is truncated"))?;
        at = end;
    }
    if at != encoded.len() {
        return work.refuse_unpriced("optional native snapshot refuses unpriced concatenated/trailing zstd frames", ctl);
    }
    // Charge both the announced size and safe block upper bound. The real
    // decoder still validates FCS, entropy contents and checksum.
    Ok((upper.max(header.decoded.unwrap_or(0)), header.window))
}

#[derive(Default)]
struct ObjectWork {
    references: usize,
    tile_bytes: usize,
}

fn object_references(value: &serde_json::Value, work: &Work, ctl: &Interrupt<'_>) -> Result<HashMap<String, ObjectWork>> {
    let mut found: HashMap<String, ObjectWork> = HashMap::new();
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        work.charge_bytes(32, ctl)?;
        if let Some(hash) = value.as_str().filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
            let entry = found.entry(hash.to_owned()).or_default();
            entry.references = entry.references.checked_add(1).ok_or(IoError::XcfNativeWorkLimit)?;
        }
        match value {
            serde_json::Value::Array(values) => {
                pending.try_reserve(values.len()).map_err(|_| invalid("native snapshot reference stack allocation failed"))?;
                pending.extend(values);
            }
            serde_json::Value::Object(values) => {
                // SurfaceM carries the exact per-tile byte length enforced by
                // LoadFetch::tile. This also prices legacy unsized tile frames.
                if let (Some(format), Some(tiles)) = (values.get("format"), values.get("tiles").and_then(serde_json::Value::as_array)) {
                    if let Ok(format) = serde_json::from_value::<PixelFormat>(format.clone()) {
                        let bytes = (photocraft_geom::TILE_SIZE as usize)
                            .checked_mul(photocraft_geom::TILE_SIZE as usize)
                            .and_then(|n| n.checked_mul(format.bytes_per_pixel()))
                            .ok_or(IoError::XcfNativeWorkLimit)?;
                        for tile in tiles {
                            if let Some(hash) = tile.get("hash").and_then(serde_json::Value::as_str) {
                                let entry = found.entry(hash.to_owned()).or_default();
                                entry.tile_bytes = entry.tile_bytes.max(bytes);
                            }
                        }
                    }
                }
                pending.try_reserve(values.len()).map_err(|_| invalid("native snapshot reference stack allocation failed"))?;
                pending.extend(values.values());
            }
            _ => {}
        }
    }
    Ok(found)
}

fn charge_product(work: &Work, bytes: u64, count: usize, ctl: &Interrupt<'_>) -> Result<()> {
    let Some(total) = bytes.checked_mul(count as u64).and_then(|n| usize::try_from(n).ok()) else {
        return work.refuse_unpriced("native snapshot admission size exceeds platform work bounds", ctl);
    };
    work.charge_bytes(total, ctl)
}

#[cfg(test)]
thread_local! { static NATIVE_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Price all potentially fetched references BEFORE the measured loader runs.
/// Failed reads/hash checks spend this shared ledger, while storage is private.
/// Callers need not charge ZIP inspection separately. Passed memory limits are
/// preserved or made stricter; successful measurement is never a work debit.
pub(super) fn load_with_work(bytes: &[u8], options: &photocraft_format::LoadOptions, work: &Work, ctl: &Interrupt<'_>) -> Result<(Document, u64)> {
    let (document, measured, _) = load_priced(bytes, options, work, ctl)?;
    Ok((document, measured))
}

fn load_priced(bytes: &[u8], options: &photocraft_format::LoadOptions, work: &Work, ctl: &Interrupt<'_>) -> Result<(Document, u64, usize)> {
    charge_product(work, bytes.len() as u64, 4, ctl)?;
    let zip = photocraft_format::zip::ZipReader::new(bytes).map_err(|e| invalid(format!("native snapshot archive: {e}")))?;
    let declared = zip
        .declared_size("manifest.json")
        .map_err(|e| invalid(e.to_string()))?
        .ok_or_else(|| invalid("uncached native snapshot contents require an embedded native document"))?;
    let manifest_bytes = usize::try_from(declared).map_err(|_| invalid("native snapshot manifest exceeds platform bounds"))?;
    if manifest_bytes > options.max_manifest_bytes || declared.saturating_mul(64) > options.max_total_bytes {
        return Err(invalid("native snapshot manifest exceeds capture allowance"));
    }
    // Account for the prefix scan, JSON representation, the loader's second
    // parse and manifest traversal before the first read can fail its checksum.
    charge_product(work, declared, 64, ctl)?;
    let manifest = zip.read_by_name("manifest.json", manifest_bytes).map_err(|e| invalid(format!("native snapshot manifest admission: {e}")))?;
    check(ctl)?;
    let json: serde_json::Value = serde_json::from_slice(&manifest).map_err(|e| invalid(format!("native snapshot reference manifest: {e}")))?;
    let references = object_references(&json, work, ctl)?;
    let mut decoded = 0u64;
    let mut largest = 0u64;
    let mut largest_window = 0u64;
    let mut largest_encoded = 0u64;
    for entry in &zip.entries {
        work.charge_bytes(64, ctl)?;
        let tile = entry.name.starts_with("tiles/");
        let Some(hash) = entry.name.strip_prefix(if tile { "tiles/" } else { "blobs/" }).and_then(|name| name.strip_suffix(".zst")) else { continue };
        let Some(reference) = references.get(hash).filter(|r| r.references != 0) else { continue };
        let outer = zip.declared_size(&entry.name).map_err(|e| invalid(e.to_string()))?.ok_or_else(|| invalid("native snapshot entry has no declared size"))?;
        let compressed =
            zip.compressed_size(&entry.name).map_err(|e| invalid(e.to_string()))?.ok_or_else(|| invalid("native snapshot entry has no compressed size"))?;
        charge_product(work, outer, reference.references.checked_mul(2).ok_or(IoError::XcfNativeWorkLimit)?, ctl)?;
        charge_product(work, compressed, reference.references.checked_mul(2).ok_or(IoError::XcfNativeWorkLimit)?, ctl)?;
        let head = zip
            .read_head(&entry.name, 18)
            .map_err(|e| invalid(format!("native snapshot object header: {e}")))?
            .ok_or_else(|| invalid("native snapshot object header is missing"))?;
        let info = frame_header(&head);
        if let Ok(info) = &info {
            if info.window > options.max_total_bytes || info.window > work.remaining_bytes() {
                return work.refuse_unpriced("native snapshot zstd window exceeds available capture/work admission", ctl);
            }
        }
        let encoded_limit = usize::try_from(options.max_total_bytes.min(super::MAX_PIXEL_BYTES as u64)).unwrap_or(usize::MAX);
        if outer > encoded_limit as u64 {
            return work.refuse_unpriced("native snapshot encoded object exceeds bounded inspection storage", ctl);
        }
        let encoded = zip.read_by_name(&entry.name, encoded_limit).map_err(|e| invalid(format!("native snapshot encoded object inspection: {e}")))?;
        let (upper, window) = frame_bound(&encoded, options.max_total_bytes, work, ctl)?;
        let size = upper.max(if tile { reference.tile_bytes as u64 } else { 0 });
        let Some(priced_size) = size.checked_add(1) else {
            return work.refuse_unpriced("native snapshot decoded size cannot be priced", ctl);
        };
        charge_product(work, priced_size, reference.references, ctl)?;
        charge_product(work, window, reference.references, ctl)?;
        largest_window = largest_window.max(window);
        largest_encoded = largest_encoded.max(outer);
        decoded = decoded.checked_add(size.checked_mul(reference.references as u64).ok_or(IoError::XcfNativeWorkLimit)?).ok_or(IoError::XcfNativeWorkLimit)?;
        largest = largest.max(size);
    }
    let metadata = manifest_bytes.checked_mul(64).ok_or(IoError::XcfNativeWorkLimit)?;
    let mut bounded = *options;
    bounded.max_total_bytes = bounded
        .max_total_bytes
        .min(decoded)
        .min(options.max_total_bytes.saturating_sub(metadata as u64).saturating_sub(largest_window).saturating_sub(largest_encoded));
    bounded.max_blob_bytes = bounded.max_blob_bytes.min(usize::try_from(largest).unwrap_or(usize::MAX));
    check(ctl)?;
    #[cfg(test)]
    NATIVE_LOADS.with(|count| count.set(count.get().saturating_add(1)));
    let (document, measured) =
        photocraft_format::load_from_bytes_measured(bytes, &bounded).map_err(|e| invalid(format!("native snapshot embedded contents: {e}")))?;
    check(ctl)?;
    Ok((document, measured, manifest_bytes))
}

/// Prepay a bounded frame operation using the same padded tile/scratch admission
/// as native capture, rather than reserving the whole per-document byte ceiling.
fn charge_frame(work: &Work, bounds: Rect, format: PixelFormat, passes: usize, ctl: &Interrupt<'_>) -> Result<()> {
    let mut remaining = usize::MAX;
    coverage::reserve_native_frame(bounds, format, &mut remaining)?;
    charge_product(work, (usize::MAX - remaining) as u64, passes, ctl)
}

/// Recognizes admitted native archives without resolving external source locators or exceeding inspection
/// memory.
fn native_kind(bytes: &[u8], budget: usize, work: &Work, ctl: &Interrupt<'_>) -> Result<bool> {
    check(ctl)?;
    if bytes.get(..2) != Some(b"PK") {
        return Ok(false);
    }
    let index = bytes.len().checked_mul(4).ok_or_else(|| invalid("native snapshot archive inspection overflows"))?;
    if index > budget {
        return Err(invalid("native snapshot archive inspection exceeds remaining capture allowance"));
    }
    work.charge_bytes(index, ctl)?;
    // ZipReader metadata allocation is admitted before signature inspection too.
    let native = photocraft_format::is_pcraft(bytes);
    check(ctl)?;
    Ok(native)
}

/// Loads and prepares bounded embedded native contents recursively, then renders their ordinary native
/// pixels.
fn render_bytes(bytes: &[u8], depth: usize, admission: &mut Admission, ctl: &Interrupt<'_>) -> Result<(Document, Surface)> {
    check(ctl)?;
    if depth > 64 {
        return Err(invalid("native snapshot exceeds 64 embedded native documents"));
    }
    admission.source = admission.source.checked_sub(bytes.len()).ok_or_else(|| invalid("native snapshot repeated embedded source bytes exceed limit"))?;
    // The native measured loader counts decoded tiles/blobs, not manifest
    // object allocations. Admit ZIP indexes and bounded JSON representation
    // separately before each load, including fan-out copies of earlier inputs.
    admission.charge(bytes.len().checked_mul(4).ok_or_else(|| invalid("native snapshot archive admission overflows"))?)?;
    let manifest_limit = super::MAX_METADATA_BYTES.min(admission.remaining / 64);
    check(ctl)?;
    let options = photocraft_format::LoadOptions {
        max_manifest_bytes: manifest_limit,
        max_blob_bytes: admission.remaining,
        max_total_bytes: admission.remaining as u64,
        max_canvas_side: super::MAX_SIDE,
        max_canvas_pixels: (admission.remaining / 32) as u64,
        preserve_ids: false,
    };
    let (mut document, measured, manifest_bytes) = load_priced(bytes, &options, &admission.work, ctl)?;
    admission.charge(manifest_bytes.checked_mul(64).ok_or_else(|| invalid("native snapshot metadata admission overflows"))?)?;
    admission.charge(usize::try_from(measured).map_err(|_| invalid("native snapshot decoded size exceeds platform limits"))?)?;
    check(ctl)?;
    // Layer data is prepared privately while the actual native document's frame,
    // profile, resources and typography remain available as immutable context.
    let mut layers = std::mem::take(&mut document.layers);
    for layer in &mut layers {
        prepare_layer(layer, &document, depth, 0, admission, ctl)?;
    }
    document.layers = layers;
    let format = document.pixel_format();
    charge_frame(&admission.work, document.bounds(), format, super::MAX_LAYERS.saturating_sub(admission.nodes).saturating_add(1), ctl)?;
    let surface = coverage::render_prepared_document(&document, format, &mut admission.remaining, ctl)?;
    finite(&surface, document.bounds(), ctl)?;
    Ok((document, surface))
}
/// Prepares native text, shape, group and Smart contents while enforcing capture depth and node limits.
fn prepare_layer(layer: &mut Layer, document: &Document, depth: usize, groups: usize, admission: &mut Admission, ctl: &Interrupt<'_>) -> Result<()> {
    check(ctl)?;
    if groups > super::MAX_DEPTH {
        return Err(invalid("native snapshot exceeds native group depth"));
    }
    if !layer.visible {
        return Ok(());
    }
    admission.work.charge_bytes(256, ctl)?;
    charge_frame(&admission.work, document.bounds(), document.pixel_format(), layer.effects.items.len().saturating_add(1), ctl)?;
    match &mut layer.content {
        LayerContent::Group(group) => {
            admission.nodes = admission.nodes.checked_sub(1).ok_or_else(|| invalid("native snapshot layer count exceeds limit"))?;
            for child in &mut group.children {
                prepare_layer(child, document, depth, groups + 1, admission, ctl)?;
            }
            admit_styles(layer, document, &mut admission.remaining)?;
        }
        LayerContent::Smart(smart) => {
            admission.nodes = admission.nodes.checked_sub(1).ok_or_else(|| invalid("native snapshot layer count exceeds limit"))?;
            let native_bytes = match &smart.source {
                SmartSource::Embedded { bytes, .. } if native_kind(bytes, admission.remaining, &admission.work, ctl)? => Some(bytes.clone()),
                _ if smart.cache.is_some() => None,
                SmartSource::Linked { .. } => {
                    return Err(invalid("uncached linked native contents require explicit resource resolution; snapshot performs no external read"));
                }
                _ => return Err(invalid("uncached snapshot contents require an embedded native document; nonnative source decoding is not authorized")),
            };
            if let Some(bytes) = native_bytes {
                // Actual native model wins over a historical/stale cache.
                let (source, pixels) = render_bytes(&bytes, depth + 1, admission, ctl)?;
                charge_frame(&admission.work, source.bounds(), document.pixel_format(), 3, ctl)?;
                let pixels = project_source(&pixels, &source, document.pixel_format(), &mut admission.remaining, ctl)?;
                charge_frame(&admission.work, pixels.content_bounds().union(&document.bounds()), document.pixel_format(), 3, ctl)?;
                let placed = place(&pixels, source.bounds(), smart, &mut admission.remaining, ctl)?;
                charge_frame(
                    &admission.work,
                    placed.content_bounds().union(&document.bounds()),
                    document.pixel_format(),
                    smart.smart_filters.len().saturating_add(2),
                    ctl,
                )?;
                smart.cache = Some(replay_smart(&placed, smart, document, &mut admission.remaining, ctl)?);
            }
            admit_styles(layer, document, &mut admission.remaining)?;
        }
        _ => coverage::prepare_direct_layer(layer, document, &mut admission.remaining, groups, &mut admission.nodes, ctl)?,
    }
    check(ctl)
}
/// Admits native style workspace before rendering effects on the captured layer.
fn admit_styles(layer: &Layer, document: &Document, budget: &mut usize) -> Result<()> {
    if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) || !layer.fill_opacity.is_finite() || !(0.0..=1.0).contains(&layer.fill_opacity) {
        return Err(invalid("native snapshot layer opacity is invalid"));
    }
    let margin = photocraft_compose::effects::margin(layer).max(0);
    let b = document.bounds();
    let area = Rect::new(
        b.x0.checked_sub(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.y0.checked_sub(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.x1.checked_add(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.y1.checked_add(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
    );
    for _ in &layer.effects.items {
        coverage::reserve_native_frame(area, document.pixel_format(), budget)?;
    }
    Ok(())
}
/// Applies ordinary native Smart placement to prepared pixels under the capture workspace allowance.
fn place(surface: &Surface, bounds: Rect, smart: &SmartObject, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    if smart.stack_mode.is_some() {
        return Err(invalid("native snapshot stack statistics need bounded native frame preparation; no source substitution is allowed"));
    }
    if smart.warp.as_ref().is_some_and(|warp| !warp.is_identity()) {
        return Err(invalid("native snapshot mesh warp needs bounded native mesh preparation; no source substitution is allowed"));
    }
    let [a, b, c, d, e, f] = smart.transform.m;
    if smart.perspective.is_none()
        && [a, b, c, d, e, f].iter().all(|v| v.is_finite())
        && (a - 1.).abs() < 1e-9
        && b.abs() < 1e-9
        && c.abs() < 1e-9
        && (d - 1.).abs() < 1e-9
        && (e - e.round()).abs() < 1e-9
        && (f - f.round()).abs() < 1e-9
    {
        if e.abs() > f64::from(i32::MAX) || f.abs() > f64::from(i32::MAX) {
            return Err(invalid("native snapshot integer placement exceeds coordinates"));
        }
        let (dx, dy) = (e.round() as i32, f.round() as i32);
        for (value, shift) in [(bounds.x0, dx), (bounds.x1, dx), (bounds.y0, dy), (bounds.y1, dy)] {
            value.checked_add(shift).ok_or_else(|| invalid("native snapshot translated frame overflows"))?;
        }
        coverage::reserve_native_frame(bounds.translate(dx, dy), surface.format(), budget)?;
        check(ctl)?;
        return Ok(photocraft_algo::resample::translate_surface(surface, dx, dy));
    }
    let matrix = photocraft_algo::transform::Homography(smart.perspective.unwrap_or([a, c, e, b, d, f, 0., 0., 1.]));
    coverage::native_projective_warp_full(surface, bounds, &matrix, budget, ctl)
}
/// Replays only admitted native Smart Filter parameters through existing native algorithms, preserving native
/// envelope rules.
fn replay_smart(placed: &Surface, smart: &SmartObject, document: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    if !smart.filters_enabled || !smart.smart_filters.iter().any(|f| f.visible) {
        return Ok(placed.clone());
    }
    if smart.smart_filters.len() > 64 {
        return Err(invalid("native snapshot Smart filter stack exceeds limit"));
    }
    let canvas = document.bounds();
    let object = placed.content_bounds();
    let extent = canvas.union(&object);
    let mut current = placed.clone();
    for filter in smart.smart_filters.iter().filter(|f| f.visible) {
        check(ctl)?;
        let reference = if matches!(
            filter.command.as_str(),
            "filter.render.clouds" | "filter.render.differenceClouds" | "filter.render.fibers" | "filter.render.lensFlare"
        ) && !object.is_empty()
        {
            object
        } else {
            canvas
        };
        let output = coverage::raw_filter(&current, filter, NativeFilterDomain { reference_bounds: reference, extent, document }, budget, ctl)?;
        current = if filter.blend == BlendMode::Normal && filter.opacity >= 1. {
            output
        } else {
            let mix_area = current.content_bounds().union(&output.content_bounds());
            coverage::reserve_native_frame(mix_area, current.format(), budget)?;
            photocraft_compose::smart_mix::mix(&current, &output, mix_area, filter.blend, filter.opacity, ctl).map_err(|e| {
                if e == photocraft_compose::smart_mix::Error::Cancelled { IoError::Cancelled } else { invalid(format!("native snapshot Smart strength: {e}")) }
            })?
        };
    }
    if let Some(mask) = smart.filter_mask.as_ref().filter(|m| m.enabled) {
        current = filter_mask(placed, &current, mask, budget, ctl)?;
    }
    Ok(current)
}
/// Mixes the complete native filter result with its base using ordinary mask density and coverage semantics.
fn filter_mask(base: &Surface, filtered: &Surface, mask: &LayerMask, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    let area = base.content_bounds().union(&filtered.content_bounds());
    let fmt = base.format();
    coverage::reserve_native_frame(area, fmt, budget)?;
    // Local base/filter/result staging each owns a padded native tile, even
    // for a one-pixel mask. Logical frame scratch alone does not price these.
    if !area.is_empty() {
        let scratch = (photocraft_geom::TILE_SIZE as usize)
            .checked_mul(photocraft_geom::TILE_SIZE as usize)
            .and_then(|n| n.checked_mul(fmt.bytes_per_pixel()))
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| invalid("native filter-mask staging allocation overflows"))?;
        *budget = budget.checked_sub(scratch).ok_or_else(|| invalid("native filter-mask padded staging exceeds capture allowance"))?;
    }
    let mut output = filtered.clone();
    // Native filter-mask mixing is pixel-local. Invoke the existing policy on
    // bounded tiles, retaining its exact color/depth/alpha arithmetic and clamp.
    for tile in area.tiles() {
        check(ctl)?;
        let r = tile.rect().intersect(&area);
        let mut b = Surface::new(fmt);
        let mut f = Surface::new(fmt);
        b.try_write_region(r, &base.try_read_region(r).map_err(|e| invalid(format!("native mask base read: {e}")))?)
            .map_err(|e| invalid(format!("native mask base staging: {e}")))?;
        f.try_write_region(r, &filtered.try_read_region(r).map_err(|e| invalid(format!("native mask output read: {e}")))?)
            .map_err(|e| invalid(format!("native mask output staging: {e}")))?;
        let mixed = photocraft_compose::filter_mask::mix(&b, &f, mask);
        check(ctl)?;
        output
            .try_write_region(r, &mixed.try_read_region(r).map_err(|e| invalid(format!("native mask result read: {e}")))?)
            .map_err(|e| invalid(format!("native mask result write: {e}")))?;
    }
    Ok(output)
}
/// Rejects nonfinite prepared samples before they can become stored native channels or masks.
fn finite(surface: &Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<()> {
    for y in bounds.y0..bounds.y1 {
        check(ctl)?;
        let row = surface.try_read_region(Rect::new(bounds.x0, y, bounds.x1, y + 1)).map_err(|e| invalid(format!("native snapshot sample read: {e}")))?;
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid("native snapshot produced nonfinite pixels"));
        }
    }
    Ok(())
}

/// Mirror ordinary Smart source projection, including its established default
/// Gray working profile and integer-to-F32 transfer policy. No new parent-ICC rule.
fn project_source(surface: &Surface, source: &Document, target: PixelFormat, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    check(ctl)?;
    let bounds = source.bounds();
    let gray = target.mode == ColorMode::Grayscale && source.pixel_format().mode != ColorMode::Grayscale;
    let linear = target.sample == SampleType::F32 && source.depth != SampleType::F32;
    let mut current = surface.clone();
    if gray {
        use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
        let space = match source.mode {
            ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => ColorSpace::Gray,
            ColorMode::Cmyk => ColorSpace::Cmyk,
            ColorMode::Lab => ColorSpace::Lab,
            _ => ColorSpace::Rgb,
        };
        let profile = source.icc_profile.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == space);
        let fallback = craft_cms::builtin::default_for(space).unwrap_or(Builtin::Srgb.profile());
        let transform = Transform::new(profile.as_ref().unwrap_or(fallback), Builtin::DotGain20Compat.profile(), Intent::RelativeColorimetric, true)
            .map_err(|e| invalid(format!("native Smart Gray projection: {e}")))?;
        let fmt = PixelFormat::new(ColorMode::Grayscale, source.depth, true);
        coverage::reserve_native_frame(bounds, fmt, budget)?;
        current = cms_gray(&current, fmt, &transform, bounds, ctl)?;
    }
    if linear {
        let old = current.format();
        coverage::reserve_native_frame(bounds, PixelFormat { sample: SampleType::F32, ..old }, budget)?;
        current = current.try_convert(PixelFormat { sample: SampleType::F32, ..old }).map_err(|e| invalid(format!("native Smart depth projection: {e}")))?;
        if matches!(current.format().mode, ColorMode::Rgb | ColorMode::Grayscale) {
            let transfer = photocraft_compose::adjust::Transfer::for_document(current.format().mode, source.depth);
            let fmt = current.format();
            let colors = fmt.mode.color_channels();
            for y in bounds.y0..bounds.y1 {
                check(ctl)?;
                let r = Rect::new(bounds.x0, y, bounds.x1, y + 1);
                let mut row = current.try_read_region(r).map_err(|e| invalid(format!("native linear projection read: {e}")))?;
                for pixel in row.chunks_exact_mut(fmt.channels()) {
                    for value in pixel.iter_mut().take(colors) {
                        *value = transfer.decode(*value);
                    }
                }
                current.try_write_region(r, &row).map_err(|e| invalid(format!("native linear projection write: {e}")))?;
            }
        }
    }
    if current.format() != target {
        coverage::reserve_native_frame(bounds, target, budget)?;
        current = current.try_convert(target).map_err(|e| invalid(format!("native Smart model projection: {e}")))?;
    }
    finite(&current, bounds, ctl)?;
    Ok(current)
}
/// Projects prepared pixels into the target Gray format through the supplied native CMS transform.
fn cms_gray(source: &Surface, target: PixelFormat, transform: &craft_cms::Transform, bounds: Rect, ctl: &Interrupt<'_>) -> Result<Surface> {
    let mut output = Surface::new(target);
    let from = source.format();
    for y in bounds.y0..bounds.y1 {
        check(ctl)?;
        let r = Rect::new(bounds.x0, y, bounds.x1, y + 1);
        let values = source.try_read_region(r).map_err(|e| invalid(format!("native CMS projection read: {e}")))?;
        let count = (bounds.width() as usize).checked_mul(target.channels()).ok_or_else(|| invalid("native CMS projection row overflows"))?;
        let mut converted = Vec::new();
        converted.try_reserve_exact(count).map_err(|_| invalid("native CMS projection row unavailable"))?;
        converted.resize(count, 0.);
        match from.sample {
            SampleType::F32 => transform
                .convert_f32(&values, from.channels(), &mut converted, target.channels(), true)
                .map_err(|error| invalid(format!("native CMS projection: {error}")))?,
            SampleType::U8 => {
                for (pixel, out) in values.chunks_exact(from.channels()).zip(converted.chunks_exact_mut(target.channels())) {
                    let mut src = [0u8; 5];
                    for (sample, value) in src.iter_mut().zip(pixel) {
                        *sample = (value.clamp(0., 1.) * 255. + 0.5) as u8;
                    }
                    let mut dst = [0u8; 2];
                    let input = src.get(..from.channels()).ok_or_else(|| invalid("native CMS source channel count exceeds supported color layout"))?;
                    transform
                        .convert_u8(input, from.channels(), &mut dst, target.channels(), true)
                        .map_err(|error| invalid(format!("native CMS projection: {error}")))?;
                    for (sample, value) in out.iter_mut().zip(dst) {
                        *sample = f32::from(value) / 255.;
                    }
                }
            }
            SampleType::U16 => {
                for (pixel, out) in values.chunks_exact(from.channels()).zip(converted.chunks_exact_mut(target.channels())) {
                    let mut src = [0u16; 5];
                    for (sample, value) in src.iter_mut().zip(pixel) {
                        *sample = (value.clamp(0., 1.) * 65535. + 0.5) as u16;
                    }
                    let mut dst = [0u16; 2];
                    let input = src.get(..from.channels()).ok_or_else(|| invalid("native CMS source channel count exceeds supported color layout"))?;
                    transform
                        .convert_u16(input, from.channels(), &mut dst, target.channels(), true)
                        .map_err(|error| invalid(format!("native CMS projection: {error}")))?;
                    for (sample, value) in out.iter_mut().zip(dst) {
                        *sample = f32::from(value) / 65535.;
                    }
                }
            }
        }
        output.try_write_region(r, &converted).map_err(|e| invalid(format!("native CMS projection write: {e}")))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::{Affine, Fill, Path, ShapeLayer, SmartFilter, Subpath};
    use photocraft_geom::Size;
    use std::sync::Arc;
    /// Builds a small native raster document at the requested mode and depth for snapshot comparisons.
    fn raster(mode: ColorMode, depth: SampleType) -> Document {
        let mut doc = Document::new("Native snapshot input", Size::new(3, 2), mode, depth);
        let mut layer = Layer::raster("Native actual pixels", doc.pixel_format());
        let values = if mode == ColorMode::Rgb { vec![0.1, 0.3, 0.6, 0.5] } else { vec![0.3, 0.5] };
        layer.surface_mut().unwrap().fill_rect(doc.bounds(), &values);
        doc.layers.push(layer);
        doc
    }
    /// Serializes a native fixture document so capture tests exercise the real embedded-data boundary.
    fn bytes(doc: &Document) -> Vec<u8> {
        photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()
    }
    /// Compares complete native surface regions with bounded diagnostics instead of relying on cache
    /// identity.
    fn assert_pixels(a: &Surface, b: &Surface, bounds: Rect) {
        assert_eq!(a.format(), b.format());
        assert_eq!(a.read_region(bounds), b.read_region(bounds));
    }
    #[test]
    /// Compares captured Smart strength and mask behavior with independent native output, including alpha
    /// preservation.
    fn snapshot_replays_native_smart_strength_and_filter_mask_without_doubling_alpha() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let source = raster(mode, depth);
                let mut doc = source.clone();
                doc.layers.clear();
                let filter = SmartFilter {
                    command: "image.adjustments.invert".into(),
                    params: serde_json::json!({}),
                    visible: true,
                    opacity: 0.4,
                    blend: BlendMode::Multiply,
                };
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.5]);
                let mut smart =
                    SmartObject::new(SmartSource::Embedded { file_name: "actual.pcraft".into(), bytes: Arc::new(bytes(&source)) }, Affine::IDENTITY, None);
                smart.smart_filters = vec![filter.clone()];
                smart.filter_mask = Some(mask.clone());
                let mut stale = Surface::new(source.pixel_format());
                let stale_values = if mode == ColorMode::Rgb { vec![0.9, 0.9, 0.9, 1.] } else { vec![0.9, 1.] };
                stale.fill_rect(source.bounds(), &stale_values);
                smart.cache = Some(stale); // current embedded data must override this historical cache

                doc.layers.push(Layer::new("Actual native editable Smart", LayerContent::Smart(smart)));
                let mut allowance = super::super::MAX_PIXEL_BYTES;
                let got = capture(&bytes(&doc), &doc, &mut allowance, &Interrupt::NONE).unwrap();
                let original = source.layers[0].surface().unwrap();
                let mut ordinary = photocraft_compose::render(&source, source.bounds());
                photocraft_compose::adjust::apply_in(
                    &photocraft_doc::Adjustment::Invert,
                    &mut ordinary,
                    photocraft_compose::adjust::Transfer::for_document(mode, depth),
                    if depth == SampleType::F32 { Some(depth) } else { None },
                    mode,
                );
                let mut inverted = Surface::new(doc.pixel_format());
                let samples = ordinary.px.iter().flat_map(|pixel| photocraft_raster::from_rgba(&doc.pixel_format(), *pixel)).collect::<Vec<_>>();
                inverted.write_region(doc.bounds(), &samples);
                let mixed = photocraft_compose::smart_mix::mix(original, &inverted, doc.bounds(), filter.blend, filter.opacity, &Interrupt::NONE).unwrap();
                let masked = photocraft_compose::filter_mask::mix(original, &mixed, &mask);
                assert_pixels(&got, &masked, doc.bounds());
                assert!(
                    (got.sample_channel(0, 0, got.channels() - 1) - original.sample_channel(0, 0, original.channels() - 1)).abs() < 0.005,
                    "native filter strength must preserve coverage"
                );
            }
        }
    }
    #[test]
    /// Checks capture uses native projection, depth and profile rules while preserving prepared geometry.
    fn snapshot_uses_native_projection_depth_profile_and_retains_prepared_geometry() {
        let source = raster(ColorMode::Rgb, SampleType::U8);
        let mut doc = source.clone();
        doc.depth = SampleType::F32;
        doc.layers.clear();
        doc.layers.push(Layer::new(
            "Native depth projection",
            LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "integer.pcraft".into(), bytes: Arc::new(bytes(&source)) },
                Affine::IDENTITY,
                None,
            )),
        ));
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&doc), &doc, &mut allowance, &Interrupt::NONE).unwrap();
        let transfer = photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, SampleType::U8);
        assert!((got.sample_channel(0, 0, 0) - transfer.decode(source.layers[0].surface().unwrap().sample_channel(0, 0, 0))).abs() < 1e-6);
        assert!((got.sample_channel(0, 0, 3) - source.layers[0].surface().unwrap().sample_channel(0, 0, 3)).abs() < 1e-6);
        let mut gray = source.clone();
        gray.mode = ColorMode::Grayscale;
        gray.icc_profile = Some(craft_cms::Builtin::SGray.profile().to_bytes());
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&source), &gray, &mut allowance, &Interrupt::NONE).unwrap();
        let transform = craft_cms::Transform::new(
            craft_cms::Builtin::Srgb.profile(),
            craft_cms::Builtin::DotGain20Compat.profile(),
            craft_cms::Intent::RelativeColorimetric,
            true,
        )
        .unwrap();
        let pixel = source.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
        let input = pixel.iter().map(|v| (v * 255. + 0.5) as u8).collect::<Vec<_>>();
        let mut output = [0u8; 2];
        transform.convert_u8(&input, 4, &mut output, 2, true).unwrap();
        assert!(
            (got.sample_channel(0, 0, 0) - f32::from(output[0]) / 255.).abs() < 1e-6,
            "ordinary Smart Gray projection uses native default working profile, not a new assigned-parent rule"
        );
        let mut shape = Document::new("Editable native vector", Size::new(12, 12), ColorMode::Rgb, SampleType::U16);
        shape.layers.push(Layer::new(
            "Native vector",
            LayerContent::Shape(ShapeLayer {
                path: Path::new(vec![Subpath::polygon(&[(1., 1.), (10., 1.), (10., 10.), (1., 10.)])]),
                fill: Some(Fill::Solid(Color::rgb(0.2, 0.5, 0.8))),
                ..Default::default()
            }),
        ));
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&shape), &shape, &mut allowance, &Interrupt::NONE).unwrap();
        assert!(got.sample_channel(5, 5, 3) > 0.99);
        assert_eq!(shape.layers[0].name, "Native vector");
    }
    #[test]
    /// Verifies capture never follows external locators and failed admission leaves the caller's budget
    /// unchanged.
    fn snapshot_never_resolves_external_locators_and_failure_budget_is_atomic() {
        let source = raster(ColorMode::Rgb, SampleType::F32);
        let original = bytes(&source);
        let mut linked = source.clone();
        linked.layers = vec![Layer::new(
            "No external permission",
            LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: "C:/not-authorized/native-snapshot.pcraft".into() }, Affine::IDENTITY, None)),
        )];
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let before = allowance;
        assert!(capture(&bytes(&linked), &linked, &mut allowance, &Interrupt::NONE).is_err());
        assert_eq!(allowance, before);
        let LayerContent::Smart(smart) = &mut linked.layers[0].content else {
            panic!("Smart");
        };
        smart.cache = source.layers[0].surface().cloned();
        let captured = capture(&bytes(&linked), &linked, &mut allowance, &Interrupt::NONE).unwrap();
        assert_pixels(&captured, source.layers[0].surface().unwrap(), source.bounds());
        let mut zero = 0;
        assert!(capture(&original, &source, &mut zero, &Interrupt::NONE).is_err());
        assert_eq!(zero, 0);
        let mut allowance = before;
        let cancel = || true;
        assert!(matches!(capture(&original, &source, &mut allowance, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(allowance, before);
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 5;
        assert!(matches!(capture(&original, &source, &mut allowance, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(allowance, before);
        assert_eq!(bytes(&source), original);
    }
    fn header(single: bool, flag: u8, dictionary: u8, size: u64) -> Vec<u8> {
        let mut bytes = vec![0x28, 0xb5, 0x2f, 0xfd, (flag << 6) | (u8::from(single) << 5) | dictionary];
        if !single {
            bytes.push(0);
        }
        bytes.extend(vec![
            0;
            match dictionary {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 4,
            }
        ]);
        let count = match flag {
            0 => usize::from(single),
            1 => 2,
            2 => 4,
            _ => 8,
        };
        bytes.extend_from_slice(&size.to_le_bytes()[..count]);
        bytes
    }

    #[test]
    fn zstd_header_pricing_handles_all_size_dictionary_and_segment_flags() {
        for single in [false, true] {
            for dictionary in 0..4 {
                for flag in 0..4 {
                    let bytes = header(single, flag, dictionary, 19);
                    let expected = match (flag, single) {
                        (0, false) => None,
                        (1, _) => Some(275),
                        _ => Some(19),
                    };
                    assert_eq!(zstd_decoded_size(&bytes).unwrap(), expected);
                    assert!(bytes.len() <= 18);
                    for end in 0..bytes.len() {
                        assert!(zstd_decoded_size(&bytes[..end]).is_err(), "single={single} flag={flag} dictionary={dictionary} end={end}");
                    }
                }
            }
        }
        assert_eq!(zstd_decoded_size(&header(true, 0, 0, 0)).unwrap(), Some(0));
        assert_eq!(zstd_decoded_size(&header(false, 3, 3, u64::MAX)).unwrap(), Some(u64::MAX));
        let mut invalid = header(true, 0, 0, 0);
        invalid[4] |= 8;
        assert!(zstd_decoded_size(&invalid).is_err());
        invalid[4] = 32 | 16; // RFC's unused bit is not interpreted by a decoder.
        assert_eq!(zstd_decoded_size(&invalid).unwrap(), Some(0));
    }

    fn block(mut header: Vec<u8>, kind: u32, size: u32, payload: &[u8]) -> Vec<u8> {
        let word = (size << 3) | (kind << 1) | 1;
        header.extend_from_slice(&word.to_le_bytes()[..3]);
        header.extend_from_slice(payload);
        header
    }

    #[test]
    fn zstd_block_pricing_bounds_unsized_canonical_frames_and_rejects_unpriced_forms() {
        let raw = block(header(false, 0, 0, 0), 0, 3, &[1, 2, 3]);
        assert_eq!(frame_bound(&raw, 4096, &Work::default(), &Interrupt::NONE).unwrap(), (3, 1024));
        let rle = block(header(false, 0, 0, 0), 1, 900, &[7]);
        assert_eq!(frame_bound(&rle, 4096, &Work::default(), &Interrupt::NONE).unwrap().0, 900);
        let compressed = block(header(false, 0, 0, 0), 2, 2, &[1, 2]);
        assert_eq!(frame_bound(&compressed, 4096, &Work::default(), &Interrupt::NONE).unwrap().0, 1024);
        // Two-byte FCS includes the specified +256 in the single-segment window.
        let sized = block(header(true, 1, 0, 19), 0, 275, &vec![0; 275]);
        assert_eq!(frame_bound(&sized, 4096, &Work::default(), &Interrupt::NONE).unwrap(), (275, 275));
        let empty = block(header(true, 0, 0, 0), 0, 0, &[]);
        assert_eq!(frame_bound(&empty, 4096, &Work::default(), &Interrupt::NONE).unwrap(), (0, 0));
        for end in 0..raw.len() {
            assert!(frame_bound(&raw[..end], 4096, &Work::default(), &Interrupt::NONE).is_err());
        }
        let mut checked = raw.clone();
        checked[4] |= 4;
        assert!(frame_bound(&checked, 4096, &Work::default(), &Interrupt::NONE).is_err());
        checked.extend([0; 4]);
        assert_eq!(frame_bound(&checked, 4096, &Work::default(), &Interrupt::NONE).unwrap().0, 3);
        assert!(frame_bound(&block(header(false, 0, 0, 0), 3, 0, &[]), 4096, &Work::default(), &Interrupt::NONE).is_err());
        assert!(frame_bound(&block(header(false, 0, 0, 0), 0, 1025, &vec![0; 1025]), 4096, &Work::default(), &Interrupt::NONE).is_err());
        let mut concatenated = raw.clone();
        concatenated.extend_from_slice(&raw);
        let mut dictionary = raw.clone();
        dictionary[4] |= 1;
        dictionary.insert(6, 1);
        let mut huge_window = raw.clone();
        huge_window[5] = 255;
        for refused in [concatenated, dictionary, huge_window, vec![0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0]] {
            let work = Work::with_units(4096);
            assert!(matches!(frame_bound(&refused, 4096, &work, &Interrupt::NONE), Err(IoError::XcfNativeWorkUnpriced(_))));
            assert_eq!(work.remaining_units(), 0);
        }
        let work = Work::with_units(4096);
        let cancel = || true;
        assert!(matches!(frame_bound(&raw, 4096, &work, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(work.remaining_units(), 4096);
        let mut two_blocks = raw.clone();
        let start = header(false, 0, 0, 0).len();
        two_blocks[start] &= !1;
        two_blocks.extend_from_slice(&[1, 0, 0]); // last empty raw block
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) != 0;
        assert!(matches!(frame_bound(&two_blocks, 4096, &work, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(work.remaining_units(), 4096 - 8, "completed block inspection is not refunded on cancellation");
    }

    #[test]
    fn snapshot_shared_work_exact_boundary_keeps_tiny_canonical_rgb_gray_admitted() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let doc = raster(mode, depth);
                let encoded = bytes(&doc);
                let work = Work::default();
                let before = work.remaining_units();
                let mut allowance = super::super::MAX_PIXEL_BYTES;
                let got = capture_with_work(&encoded, &doc, &mut allowance, &work, &Interrupt::NONE).unwrap();
                assert_pixels(&got, doc.layers[0].surface().unwrap(), doc.bounds());
                let spent = before - work.remaining_units();
                assert!(spent > 0 && spent < (64 << 20) / 4, "tiny capture must not reserve a whole document ceiling");
                let exact = Work::with_units(spent);
                let mut memory = super::super::MAX_PIXEL_BYTES;
                capture_with_work(&encoded, &doc, &mut memory, &exact, &Interrupt::NONE).unwrap();
                assert_eq!(exact.remaining_units(), 0);
                let short = Work::with_units(spent - 1);
                let mut memory = super::super::MAX_PIXEL_BYTES;
                assert!(capture_with_work(&encoded, &doc, &mut memory, &short, &Interrupt::NONE).is_err());
                assert_eq!(memory, super::super::MAX_PIXEL_BYTES);
                assert_eq!(short.remaining_units(), 0);
            }
        }
    }

    #[test]
    fn late_native_hash_failure_spends_shared_work_without_publishing_storage() {
        use photocraft_format::zip::{ZipReader, ZipWriter};
        let doc = raster(ColorMode::Rgb, SampleType::U8);
        let original = bytes(&doc);
        let mut changed = doc.clone();
        changed.layers[0].surface_mut().unwrap().fill_rect(doc.bounds(), &[0.8, 0.7, 0.6, 0.5]);
        let changed_bytes = bytes(&changed);
        let other = ZipReader::new(&changed_bytes).unwrap();
        let replacement = other.entries.iter().find(|entry| entry.name.starts_with("tiles/")).unwrap();
        let replacement = other.read(replacement, super::super::MAX_PIXEL_BYTES).unwrap();
        let zip = ZipReader::new(&original).unwrap();
        let mut writer = ZipWriter::new();
        let mut replaced = false;
        for entry in &zip.entries {
            let content = if entry.name.starts_with("tiles/") && !replaced {
                replaced = true;
                replacement.clone()
            } else {
                zip.read(entry, super::super::MAX_PIXEL_BYTES).unwrap()
            };
            writer.add(&entry.name, &content).unwrap();
        }
        assert!(replaced);
        let damaged = writer.finish().unwrap();
        let work = Work::default();
        let fork = work.clone();
        let before = work.remaining_units();
        NATIVE_LOADS.with(|count| count.set(0));
        for ledger in [&work, &fork] {
            let previous = work.remaining_units();
            let mut memory = super::super::MAX_PIXEL_BYTES;
            assert!(capture_with_work(&damaged, &doc, &mut memory, ledger, &Interrupt::NONE).is_err());
            assert_eq!(memory, super::super::MAX_PIXEL_BYTES);
            assert!(work.remaining_units() < previous);
        }
        NATIVE_LOADS.with(|count| assert_eq!(count.get(), 2, "both failures reached the real loader after admission"));
        let spent = (before - work.remaining_units()) / 2;
        let limited = Work::with_units(spent - 1);
        NATIVE_LOADS.with(|count| count.set(0));
        let mut memory = super::super::MAX_PIXEL_BYTES;
        assert!(capture_with_work(&damaged, &doc, &mut memory, &limited, &Interrupt::NONE).is_err());
        NATIVE_LOADS.with(|count| assert_eq!(count.get(), 0, "work refusal precedes another loader attempt"));
        assert_eq!(memory, super::super::MAX_PIXEL_BYTES);
    }

    #[test]
    fn repeated_tile_references_are_priced_before_loader_and_cancel_never_publishes() {
        let one = raster(ColorMode::Rgb, SampleType::F32);
        let mut two = one.clone();
        two.layers.push(two.layers[0].clone());
        let options = photocraft_format::LoadOptions::default();
        let a = Work::default();
        let b = Work::default();
        load_with_work(&bytes(&one), &options, &a, &Interrupt::NONE).unwrap();
        load_with_work(&bytes(&two), &options, &b, &Interrupt::NONE).unwrap();
        assert!(b.remaining_units() < a.remaining_units());
        let work = Work::with_units(100);
        let cancel = || true;
        NATIVE_LOADS.with(|count| count.set(0));
        assert!(matches!(load_with_work(&bytes(&one), &options, &work, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        NATIVE_LOADS.with(|count| assert_eq!(count.get(), 0));
        assert_eq!(work.remaining_units(), 100);
    }

    #[test]
    /// Checks filter-mask mixing across tiles against native density, HDR and default-coverage behavior.
    fn tiled_filter_mask_matches_native_density_hdr_and_tile_boundary_policy() {
        let bounds = Rect::new(255, 0, 258, 1);
        for alpha in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
            let mut base = Surface::new(format);
            let mut filtered = Surface::new(format);
            let before = if alpha { vec![3., -2., 1., 0.4] } else { vec![3., -2., 1.] };
            let after = if alpha { vec![1.5, 2., -1., 0.7] } else { vec![1.5, 2., -1.] };
            base.fill_rect(bounds, &before);
            filtered.fill_rect(bounds, &after);
            let mask = LayerMask { surface: Surface::with_default(PixelFormat::GRAY8, &[0.2]), density: 0.35, feather: 3., ..LayerMask::reveal_all() };
            let expected = photocraft_compose::filter_mask::mix(&base, &filtered, &mask);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let got = filter_mask(&base, &filtered, &mask, &mut budget, &Interrupt::NONE).unwrap();
            assert_pixels(&got, &expected, bounds);
            // The uniform mask's density gives 0.72 effective coverage. HDR colors
            // stay signed and above one with or without alpha; alpha-bearing colors
            // use the independently weighted premultiplied numerator and coverage.
            let expected_native: &[f32] = if alpha { &[1.092 / 0.616, 0.784 / 0.616, -0.392 / 0.616, 0.616] } else { &[1.92, 0.88, -0.44] };
            for x in 255..258 {
                for (channel, expected) in expected_native.iter().enumerate() {
                    assert!((got.sample_channel(x, 0, channel) - expected).abs() < 1e-5, "alpha={alpha} x={x} channel={channel}");
                }
            }
            let mut tiny = 0;
            assert!(filter_mask(&base, &filtered, &mask, &mut tiny, &Interrupt::NONE).is_err());
            assert_eq!(tiny, 0);
        }
    }
}
