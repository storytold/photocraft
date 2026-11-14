//! Smart objects in PSD: the placed-layer blocks (`SoLd`, `PlLd`) and the smart filter stack
//! (`filterFX` inside `SoLd`).
//!
//! Layouts follow the Adobe PSD spec ("Placed Layer", "Placed Layer Data") and Photoshop-authored
//! files (`corpus/photoshop/smart-filters`): a `soLD` descriptor holds the linked-file id (`Idnt`,
//! the uuid of the `lnk2` item), the instance id (`placed`, which keys the `FEid` filter cache),
//! the transform quad (`Trnf`), the source size (`Sz  `), the warp, and when filters are applied a
//! `filterFX` object:
//!
//! ```text
//! filterFX (filterFXStyle): enab, validAtPosition, filterMaskEnable, filterMaskLinked,
//!   filterMaskExtendWithWhite, filterFXList = [ filterFX: Nm, blendOptions {Opct, Md},
//!   enab, hasoptions, FrgC, BckC, Fltr (the filter's own descriptor), filterID ]
//! ```
//!
//! The list is in application order (first = bottom, applied first), like
//! [`SmartObject::smart_filters`](photocraft_doc::SmartObject). Filters PhotoCraft implements map
//! to their command id and dialog parameters; any other filter becomes a
//! [`UNSUPPORTED_FILTER`] entry that keeps its descriptor verbatim (hex in `params.psd`), so it is
//! listed, can be hidden, reordered or deleted, and is written back unchanged.
//!
//! Camera Raw and Displace keep their whole item as a template next to the editable settings
//! (Displace's carries the embedded displacement map, see `displace_map.rs`); so does Perspective
//! Warp (`perspective_warp_map.rs`), whose quads become the planes of its command. Hue/Saturation
//! (`HStr`) maps to its adjustment command in `hue_sat_filter_map.rs`.

use craft_psd::Compression;
use craft_psd::Rect as PsdRect;
use craft_psd::descriptor::{Class, Descriptor, Id, ObjectArray, ReferenceItem, UnicodeString, Value, VersionedDescriptor};
use craft_psd::filter_effects::{EffectsPlane, FilterEffectsItem};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{LayerMask, SmartFilter, SmartObject};
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
use photocraft_geom::{Affine, Rect as GeomRect};
use photocraft_raster::Surface;
use serde_json::{Map, Value as J, json};

use crate::blocks::{enum_of, get_desc, num};

#[cfg(test)]
#[path = "smart_map/halftone_tests.rs"]
mod halftone_tests;

#[path = "camera_raw_map.rs"]
mod camera_raw;
#[path = "displace_map.rs"]
mod displace;
#[path = "hue_sat_filter_map.rs"]
mod hue_sat;
#[path = "perspective_warp_map.rs"]
mod perspective_warp;
#[path = "puppet_map.rs"]
mod puppet;

pub use displace::{embed_map as displace_embed_map, embedded_map as displace_embedded_map, without_map as displace_without_map};
pub use perspective_warp::{
    MAX_QUADS as MAX_PERSPECTIVE_WARP_PLANES, geometry as perspective_warp_geometry, indexed_geometry as perspective_warp_indexed_geometry,
};

mod export;
mod import;
mod placed;

use export::{blend_options, is_meta, rgbc, set, unit};
pub use export::{filter_fx, item_for_filter};
pub use import::filter_from_item;
#[cfg(test)]
use import::{GALLERY_EFFECTS, LENSES, MEZZOTINT_TYPES, adjustment_list, known_params};
use import::{code, from_hex, int, to_hex};
use placed::enumv;
pub use placed::{PlacedSpec, plld_bytes, sold_bytes, stored_size};
#[cfg(test)]
use placed::{quad_moved, warp_desc};

/// Command id of a Photoshop smart filter PhotoCraft does not implement. Its params hold the
/// filter's name, Photoshop filter id and descriptor (`psd`, hex); it renders as a pass-through.
pub const UNSUPPORTED_FILTER: &str = "psd.unsupportedFilter";

/// The smart filter stack of a placed layer, as stored in `filterFX`.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterStack {
    /// `enab`: Layer › Smart Filter › Disable Smart Filters clears it.
    pub enabled: bool,
    pub filters: Vec<SmartFilter>,
    /// `filterMaskEnable`.
    pub mask_enabled: bool,
    /// `filterMaskLinked`.
    pub mask_linked: bool,
}

impl Default for FilterStack {
    fn default() -> Self {
        FilterStack { enabled: true, filters: Vec::new(), mask_enabled: true, mask_linked: false }
    }
}

/// What a `soLD` descriptor says about a smart object.
#[derive(Clone, Debug)]
pub struct Placed {
    pub descriptor: Descriptor,
    /// `Idnt`: uuid of the embedded or linked file (`lnk2` item).
    pub idnt: String,
    /// `placed`: id of this instance (keys the `FEid` filter cache).
    pub placed: String,
    /// Source pixels → document pixels (from `Trnf` and `Sz  `).
    pub transform: Affine,
    /// `filterFX`, if any filters were ever applied.
    pub stack: Option<FilterStack>,
}

fn text_of(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key)? {
        Value::Text(t) => Some(t.to_string_lossy()),
        _ => None,
    }
}

fn bool_or(d: &Descriptor, key: &str, default: bool) -> bool {
    match d.get(key) {
        Some(Value::Boolean(b)) => *b,
        _ => default,
    }
}

/// The `soLD` descriptor of `SoLd`/`SoLE` block data.
pub fn sold_descriptor(data: &[u8]) -> Option<Descriptor> {
    if data.get(..4)? != b"soLD" {
        return None;
    }
    crate::blocks::parse_prefix_versioned(data.get(8..)?)
}

/// Parses `SoLd`/`SoLE` block data (best effort: `None` when it is not a `soLD` structure).
pub fn parse_sold(data: &[u8]) -> Option<Placed> {
    let d = sold_descriptor(data)?;
    let (idnt, transform) = crate::blocks::parse_smart(b"SoLd", data);
    let placed = text_of(&d, "placed").unwrap_or_default();
    let mut stack = get_desc(&d, "filterFX").map(filter_stack);
    if let Some(stack) = &mut stack {
        drop_moved_puppet_meshes(stack, &d);
    }
    Some(Placed { descriptor: d, idnt, placed, transform, stack })
}

/// A Puppet Warp's stored mesh sits where the layer was when it was built (document pixels); one
/// built on other corners than the layer's own doesn't fit it, so its pins are solved instead.
fn drop_moved_puppet_meshes(stack: &mut FilterStack, d: &Descriptor) {
    let corners = |k: &str| match d.get(k) {
        Some(Value::List(v)) if v.len() == 8 => v.iter().map(|x| num(Some(x))).collect::<Option<Vec<f64>>>(),
        _ => None,
    };
    let quad = corners("nonAffineTransform").or_else(|| corners("Trnf"));
    for f in stack.filters.iter_mut().filter(|f| f.command == puppet::COMMAND) {
        let fits = match (&quad, puppet::built_on(&f.params)) {
            (Some(q), Some(b)) => q.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01),
            _ => false,
        };
        if !fits && let Some(p) = f.params.as_object_mut() {
            p.remove("mesh");
        }
    }
}

/// Records on each imported Puppet Warp mesh the placement it was solved for (`mesh.placement`,
/// [`SmartObject::placement_json`]): once the smart object is moved, transformed or warped, the
/// mesh no longer fits it and the pins are solved instead.
pub(crate) fn pin_puppet_meshes(sm: &mut SmartObject) {
    let placement = sm.placement_json();
    for f in sm.smart_filters.iter_mut().filter(|f| f.command == puppet::COMMAND) {
        if let Some(m) = f.params.get_mut("mesh").and_then(J::as_object_mut) {
            m.insert("placement".into(), placement.clone());
        }
    }
}

/// Reads a `filterFX` object.
pub fn filter_stack(fx: &Descriptor) -> FilterStack {
    let filters = match fx.get("filterFXList") {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Descriptor(d) => Some(filter_from_item(d)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    FilterStack {
        enabled: bool_or(fx, "enab", true),
        filters,
        mask_enabled: bool_or(fx, "filterMaskEnable", true),
        mask_linked: bool_or(fx, "filterMaskLinked", false),
    }
}

// ---------- the filter mask (`FEid`) ----------

/// Most pixel bytes the smart-filter caches may keep decoded across one PSD import or export: the
/// unfiltered `FEid` sources, the filter masks and the smart objects mixed through them. These are
/// not in the PSD import budget, which they could exceed many times over from a small file (one
/// cache can serve any number of layers).
pub const MAX_SMART_FILTER_CACHE_BYTES: u64 = 1 << 30;

/// What is left of [`MAX_SMART_FILTER_CACHE_BYTES`] (or a lower cap) in one import or export:
/// every smart-filter cache it keeps is charged here before it is decoded or made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterCacheBudget {
    used: u64,
    cap: u64,
}

impl Default for FilterCacheBudget {
    fn default() -> Self {
        #[cfg(test)]
        if let Some(cap) = TEST_CACHE_CAP.with(std::cell::Cell::get) {
            return Self::new(cap);
        }
        Self::new(MAX_SMART_FILTER_CACHE_BYTES)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_CACHE_CAP: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with every [`FilterCacheBudget`] made on this thread capped at `cap` (an import's or
/// an export's), so a test needs no gigabyte of caches to reach the cap.
#[cfg(test)]
pub(crate) fn with_test_cache_cap<R>(cap: u64, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<u64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CACHE_CAP.with(|c| c.set(self.0));
        }
    }
    let _reset = Reset(TEST_CACHE_CAP.with(|c| c.replace(Some(cap))));
    f()
}

impl FilterCacheBudget {
    pub fn new(cap: u64) -> Self {
        Self { used: 0, cap }
    }

    /// Bytes charged so far.
    pub fn used(&self) -> u64 {
        self.used
    }

    /// Whether `bytes` more fit under the cap.
    pub fn fits(&self, bytes: u64) -> bool {
        self.used.checked_add(bytes).is_some_and(|total| total <= self.cap)
    }

    /// Charges `bytes` when they fit (`true`); charges nothing otherwise.
    pub fn try_charge(&mut self, bytes: u64) -> bool {
        let fits = self.fits(bytes);
        if fits {
            self.used = self.used.saturating_add(bytes);
        }
        fits
    }

    /// Gives back `bytes` charged earlier, once what they paid for is freed: the budget bounds
    /// what is held at once, not what passes through.
    pub fn release(&mut self, bytes: u64) {
        self.used = self.used.saturating_sub(bytes);
    }
}

/// What [`mask_from_item`] charged for the `mask` it made.
pub fn mask_charge(mask: &LayerMask) -> u64 {
    photocraft_raster::stored_tiles_bytes(mask.surface.tile_count() as u64, &mask.surface.format())
}

/// What [`unfiltered_from_item`] charged for the pixels it made of `item` in `fmt`.
pub fn unfiltered_charge(item: &FilterEffectsItem, fmt: PixelFormat) -> u64 {
    photocraft_raster::tile_storage_bytes(psd_rect(&item.rect), &fmt)
}

/// Why a smart-filter cache was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterCacheError {
    /// Damaged or unsupported data; the cache is ignored.
    Unreadable(String),
    /// More pixels than are left of the [`FilterCacheBudget`]; the cache is ignored.
    TooLarge,
    /// Its pixels could not be allocated: the open fails.
    Allocation(photocraft_raster::AllocationError),
}

impl std::fmt::Display for FilterCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => f.write_str(why),
            Self::TooLarge => f.write_str("the smart filter caches are too large to keep in memory"),
            Self::Allocation(e) => write!(f, "the smart filter cache could not be allocated: {e}"),
        }
    }
}

fn psd_rect(r: &PsdRect) -> GeomRect {
    GeomRect::new(r.left, r.top, r.right, r.bottom)
}

/// The filter mask stored in an `FEid` item, in document coordinates and white beyond its bounds
/// (`filterMaskExtendWithWhite`), as a grayscale mask of `sample` depth. `Ok(None)` when the
/// item has no mask or it is all white (no mask); `Err` when it can't be decoded, or its tiles
/// don't fit what is left of `budget` (checked before anything is decoded; the tiles kept are
/// charged).
pub fn mask_from_item(
    item: &FilterEffectsItem,
    sample: SampleType,
    stack: &FilterStack,
    budget: &mut FilterCacheBudget,
) -> Result<Option<LayerMask>, FilterCacheError> {
    let Some((rect, _)) = &item.mask else { return Ok(None) };
    if item.depth != u32::from(crate::pixels::psd_depth(sample)) {
        return Err(FilterCacheError::Unreadable(format!("the smart filter mask is {}-bit in a {:?} document", item.depth, sample)));
    }
    let (w, h) = rect.size().map_err(|e| FilterCacheError::Unreadable(e.to_string()))?;
    let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let area = psd_rect(rect);
    if !budget.fits(photocraft_raster::tile_storage_bytes(area, &fmt)) {
        return Err(FilterCacheError::TooLarge);
    }
    let unreadable = || FilterCacheError::Unreadable("the smart filter mask could not be decoded".into());
    let (_, mut plane) = item.decoded_mask().ok_or_else(unreadable)?;
    if w.checked_mul(h).and_then(|n| n.checked_mul(sample.bytes())) != Some(plane.len()) {
        return Err(unreadable());
    }
    // One channel: its big-endian samples become native in place (float bits kept as they are).
    #[cfg(target_endian = "little")]
    for s in plane.chunks_exact_mut(sample.bytes()) {
        s.reverse();
    }
    let mut surface = Surface::with_default(fmt, &[1.0]);
    surface.try_write_interleaved(area, &plane).map_err(FilterCacheError::Allocation)?;
    drop(plane);
    surface.prune();
    if surface.content_bounds().is_empty() {
        return Ok(None);
    }
    budget.try_charge(photocraft_raster::stored_tiles_bytes(surface.tile_count() as u64, &fmt));
    Ok(Some(LayerMask { surface, enabled: stack.mask_enabled, linked: stack.mask_linked, density: 1.0, feather: 0.0 }))
}

/// The unfiltered placed pixels of an `FEid` item (the inverse of [`feid_item`]), in `fmt`: colour
/// channels from slot 0, transparency from the last slot (opaque without it). `Ok(None)` when the
/// item has no colour planes, its depth isn't `fmt`'s, or a plane doesn't decode;
/// [`FilterCacheError::TooLarge`] when its tiles don't fit what is left of `budget` (checked
/// before anything is decoded; charged when it is made).
pub fn unfiltered_from_item(item: &FilterEffectsItem, fmt: PixelFormat, budget: &mut FilterCacheBudget) -> Result<Option<Surface>, FilterCacheError> {
    let Some(depth) = u16::try_from(item.depth).ok().filter(|_| u32::from(crate::pixels::psd_depth(fmt.sample)) == item.depth && fmt.alpha) else {
        return Ok(None);
    };
    let Ok((w, h)) = item.rect.size() else { return Ok(None) };
    let Some(n) = w.checked_mul(h) else { return Ok(None) };
    let area = psd_rect(&item.rect);
    // The surface keeps whole tiles over the item's rect, not its logical pixels.
    let cost = photocraft_raster::tile_storage_bytes(area, &fmt);
    if !budget.fits(cost) {
        return Err(FilterCacheError::TooLarge);
    }
    let cc = fmt.mode.color_channels();
    let mut planes = Vec::with_capacity(cc + 1);
    for slot in (0..cc).chain(std::iter::once(item.slots.len().saturating_sub(1))) {
        planes.push(match item.slots.get(slot).and_then(Option::as_ref) {
            Some(p) => match p.decode(w, h, depth) {
                Ok(p) => Some(p),
                Err(_) => return Ok(None),
            },
            None if slot < cc => return Ok(None),
            None => None,
        });
    }
    let refs: Vec<Option<&[u8]>> = planes.iter().map(|p| p.as_deref()).collect();
    let mut fill = vec![crate::pixels::zero_sample(fmt.sample); cc];
    // Without a transparency plane the pixels are opaque (a native-endian full sample).
    fill.push(match fmt.sample {
        SampleType::U8 => vec![u8::MAX],
        SampleType::U16 => u16::MAX.to_ne_bytes().to_vec(),
        SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
    });
    let mut invert = vec![fmt.mode == ColorMode::Cmyk; cc];
    invert.push(false);
    let mut bytes = match crate::pixels::try_interleave(&refs, &fill, n, fmt.sample, &invert) {
        Ok(bytes) => bytes,
        Err(craft_psd::PsdError::OutOfMemory(_)) => {
            return Err(FilterCacheError::Allocation(photocraft_raster::AllocationError::NotEnoughMemory { bytes: n.saturating_mul(fmt.bytes_per_pixel()) }));
        }
        Err(_) => return Ok(None),
    };
    drop(refs);
    drop(planes);
    if fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16 {
        crate::pixels::lab16_chroma(&mut bytes, cc + 1, true);
    }
    let mut surface = Surface::new(fmt);
    surface.try_write_interleaved(area, &bytes).map_err(FilterCacheError::Allocation)?;
    photocraft_raster::scratch::give(bytes);
    surface.prune();
    budget.try_charge(cost);
    Ok(Some(surface))
}

/// An `FEid` item for a smart object with filters: the unfiltered placed pixels Photoshop
/// filters from (colour channels from slot 0, transparency in the last slot; in the document's
/// pixel format, CMYK inverted as in layer channels) and the filter mask over `bounds` (white
/// when `mask` is `None`). Photoshop needs the planes: re-rendering the filters of an item
/// without them crashes it. `None` when there are no unfiltered pixels.
pub fn feid_item(placed: &str, unfiltered: &Surface, mask: Option<&LayerMask>, bounds: GeomRect, doc_fmt: PixelFormat) -> Option<FilterEffectsItem> {
    let sample = doc_fmt.sample;
    let px = if unfiltered.format() == doc_fmt { unfiltered.clone() } else { unfiltered.convert(doc_fmt) };
    let content = px.content_bounds();
    if content.is_empty() {
        return None;
    }
    // Over the whole area the filters may draw into: Photoshop clips the filtered result to these
    // planes, so the content bounds alone would cut blurs off at the source's edge.
    let r = content.union(&bounds);
    let cc = doc_fmt.mode.color_channels();
    let depth = crate::pixels::psd_depth(sample);
    let compression = if sample == SampleType::F32 { Compression::ZipPrediction } else { Compression::Rle };
    let encode = |plane: &[u8], w: usize, h: usize| {
        EffectsPlane::encode(compression, plane, w, h, depth)
            .or_else(|_| EffectsPlane::encode(Compression::Raw, plane, w, h, depth))
            .unwrap_or(EffectsPlane { compression: Compression::Raw, data: plane.to_vec() })
    };
    let mut invert = vec![doc_fmt.mode == ColorMode::Cmyk; cc];
    invert.push(false);
    // Fully transparent pixels carry white, as in Photoshop's own cache: filters that look at
    // colour regardless of alpha (Shadows/Highlights) read it.
    let mut vals = px.read_region(r);
    let white = photocraft_raster::from_rgba(&doc_fmt, [1.0, 1.0, 1.0, 1.0]);
    for p in vals.chunks_exact_mut(cc + 1) {
        if p[cc] <= 0.0 {
            p[..cc].copy_from_slice(&white[..cc]);
        }
    }
    let mut filled = Surface::new(doc_fmt);
    filled.write_region(r, &vals);
    drop(vals);
    let mut bytes = filled.to_interleaved(r);
    if doc_fmt.mode == ColorMode::Lab && sample == SampleType::U16 {
        // 16-bit Lab a*/b* in Photoshop's scale, as in layer channels.
        crate::pixels::lab16_chroma(&mut bytes, cc + 1, false);
    }
    let planes = crate::pixels::deinterleave(&bytes, cc + 1, sample, &invert);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut slots = vec![None; 26];
    for (i, p) in planes.iter().enumerate() {
        let slot = if i == cc { 25 } else { i };
        if let Some(s) = slots.get_mut(slot) {
            *s = Some(encode(p, w, h));
        }
    }
    let mut item = filter_mask_item(placed, mask, bounds, sample);
    item.rect = PsdRect { top: r.y0, left: r.x0, bottom: r.y1, right: r.x1 };
    item.slots = slots;
    Some(item)
}

fn filter_mask_item(placed: &str, mask: Option<&LayerMask>, bounds: GeomRect, sample: SampleType) -> FilterEffectsItem {
    let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let area = match mask {
        Some(m) => m.surface.content_bounds().union(&bounds),
        None => bounds,
    };
    let area = if area.is_empty() { GeomRect::new(0, 0, 1, 1) } else { area };
    let rect = PsdRect { top: area.y0, left: area.x0, bottom: area.y1, right: area.x1 };
    let (w, h) = (area.width() as usize, area.height() as usize);
    let plane = match mask {
        Some(m) => {
            let s = if m.surface.format() == fmt { m.surface.clone() } else { m.surface.convert(fmt) };
            crate::pixels::deinterleave(&s.to_interleaved(area), 1, sample, &[false]).into_iter().next().unwrap_or_default()
        }
        None => {
            let mut white = Vec::with_capacity(w * h * sample.bytes());
            for _ in 0..w * h {
                crate::pixels::encode_be(1.0, sample, &mut white);
            }
            white
        }
    };
    let depth = crate::pixels::psd_depth(sample);
    let compression = if sample == SampleType::F32 { Compression::ZipPrediction } else { Compression::Rle };
    let plane = EffectsPlane::encode(compression, &plane, w, h, depth)
        .or_else(|_| EffectsPlane::encode(Compression::Raw, &plane, w, h, depth))
        .unwrap_or(EffectsPlane { compression: Compression::Raw, data: plane });
    FilterEffectsItem { id: placed.to_string(), version: 1, rect, depth: u32::from(depth), max_channels: 24, slots: vec![None; 26], mask: Some((rect, plane)) }
}

/// A deterministic uuid-shaped id (8-4-4-4-12 hex digits) derived from `seed`.
pub fn uuid_from(seed: &[u8]) -> String {
    let h = blake3::hash(seed);
    let x = to_hex(&h.as_bytes()[..16]);
    format!("{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32])
}

#[cfg(test)]
mod tests;
