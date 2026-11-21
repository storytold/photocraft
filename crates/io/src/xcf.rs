//! GIMP XCF (plain, gzip, bzip2 or XZ) read. Layout from the public specification
//! (https://developer.gimp.org/core/standards/xcf/, versions 0 to 26): a header, image
//! properties, then pointers to layer, channel and path structures; each drawable's pixels sit in
//! 64×64 tiles, uncompressed, RLE-coded or zlib-coded.
//!
//! Read: pixel layers at any position and size, groups (nested, pass-through), names, opacity,
//! visibility, offsets, layer masks (applied or disabled), locks, colour tags, blend modes (the
//! legacy and the 2.10 sets; those without a Photoshop equivalent open as Normal with a warning),
//! 8/16/32-bit integer and 16/32/64-bit float precisions (source transfer curves and assigned
//! colorants become an ordinary native ICC profile), the embedded ICC profile, resolution, guides, the comment, extra channels and the
//! saved selection; indexed images open as RGB. Supported text, vector and effect records become
//! ordinary native editable content with explicit conversion outcomes. Unresolved records retain
//! their usable saved pixels and inert configuration. Floating pixels open as separate native
//! raster layers; source attachments and resource locators remain inert.
//! Write: not supported (GIMP's own format; `.pcraft` keeps everything PhotoCraft holds).
//!
//! The reader is written from the specification and from files GIMP 3.2 writes; no GIMP code.

mod color;
mod conversion;
#[cfg(test)]
mod dpi_tests;
mod geometry;
mod layers;
mod samples;
mod wire;
mod work_budget;
mod xml_work;
// Parsed fields stay inert until native mapping slices land.
#[cfg(test)]
mod coverage_tests;
#[cfg(test)]
mod effect_offset_tests;
#[cfg(test)]
mod effect_retention_tests;
#[cfg(test)]
mod effect_storage_tests;
#[allow(dead_code)]
mod effects;
#[cfg(test)]
mod floating_gray_tests;
#[cfg(test)]
mod header_validation_tests;
#[cfg(test)]
mod historical_precision_tests;
#[cfg(test)]
mod import_notice_tests;
mod metadata;
#[cfg(test)]
mod metadata_admission_tests;
mod metadata_packets;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod native_conversion_tests;
#[cfg(test)]
mod optional_mask_budget_tests;
#[cfg(test)]
mod optional_mask_header_refund_tests;
#[cfg(test)]
mod path_admission_tests;
#[cfg(test)]
mod path_geometry_tests;
mod paths;
mod resources;
#[cfg(test)]
mod tests;
mod text;
#[cfg(test)]
mod tile_budget_tests;
pub(crate) mod transport;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

#[cfg(test)]
use photocraft_codecs::f16;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, NativeTargetRef};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{AllocationError, Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

#[cfg(test)]
use samples::integer_samples;
use samples::{Precision, row_samples};
use wire::{Compression, Cursor, Prop, be_f32, be_i32, be_u32, decode_tile, props};
#[cfg(test)]
use wire::{props_version, rle_decode};

type Result<T> = std::result::Result<T, IoError>;

const MAGIC: &[u8] = b"gimp xcf ";
/// Canvas side limit (Photoshop's PSB limit).
pub(crate) const MAX_SIDE: u32 = 300_000;
/// Decoded pixels across every drawable are capped, as for OpenRaster and PDN.
pub(crate) const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Layers and groups in one document.
pub(crate) const MAX_LAYERS: usize = 8192;
/// Extra channels in one document.
const MAX_CHANNELS: usize = 1024;
/// Group nesting.
const MAX_DEPTH: usize = 64;
/// Total copied layer metadata (names and item paths). A single reused record with a large
/// name is bounded by MAX_STRING, but repeated layer offsets referencing it must not multiply
/// into unbounded retained metadata.
const MAX_METADATA_BYTES: usize = 32 << 20;
/// A string field (layer names, parasite names).
const MAX_STRING: usize = 1 << 20;
/// A property payload or a parasite that is read into memory.
const MAX_PAYLOAD: usize = 64 << 20;
/// The largest file a gzipped XCF may inflate to.
const MAX_INFLATED: usize = 1 << 30;
/// Highest version this reader was written against; newer files still open (unknown properties
/// are skipped by their length), with a warning.
const KNOWN_VERSION: u32 = 26;
const TILE: u32 = 64;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Xcf(message.into())
}

/// `true` for an XCF file (by its signature).
pub(crate) fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Image-wide facts the drawables need.
struct Image<'a> {
    c: Cursor<'a>,
    version: u32,
    precision: Precision,
    /// Tile samples only; structural fields always retain XCF's BE encoding.
    sample_order: crate::HistoricalXcfSampleOrder,
    compression: Compression,
    base_type: u32,
    colormap: Vec<[u8; 3]>,
    budget: usize,
    /// Remaining allowance for copied layer metadata (names, item paths). Repeated layer
    /// offsets referencing one record with a large name would otherwise allocate an owned
    /// copy per reference, multiplying a small file into gigabytes of retained metadata.
    meta_budget: usize,
    /// Nonrefundable raw name bytes admitted to UTF-8 validation across all drawables.
    name_work_left: usize,
    vector_style_work_left: usize,
    link_work_left: usize,
    /// Nonrefundable effect decoding work shared across all drawable/coverage owners.
    effect_decode_work: effects::DecodeWork,
    /// Nonrefundable configuration and inherited attribute work across all text owners.
    text_decode_work: text::DecodeWork,
    /// Borrowed optional metadata scans share work across image, layer and report passes.
    parasite_work: wire::ParasiteWork,
    report_parasite_work: wire::ParasiteWork,
    /// Cumulative optional point-origin shaping work; independent of retained metadata.
    point_text_work_left: usize,
    warnings: Warnings,
    outcomes: conversion::Outcomes<'a>,
    /// Layers GIMP composites with "Clip to backdrop" (its default for every mode but Normal).
    clipped_to_backdrop: usize,
    initial_view: crate::initial_view::NativeInitialView,
    source_resolution: (f32, f32),
    pending_effects: Vec<PendingLayerEffects>,
    path_tattoos: HashMap<u32, Option<PathBinding>>,
    pending_vectors: Vec<(photocraft_doc::LayerId, usize, paths::VectorStyle)>,
    /// Unavailable literal-color conversion keeps source text/vector rasters intact.
    native_color_error: Option<String>,
}

/// Drawable pixels remain essential even when their optional names/effect lists do not fit.
struct Channel<'a> {
    name: String,
    properties: Vec<Prop<'a>>,
    pixels: Pixels,
    effects: Vec<usize>,
    name_limited: bool,
    effects_limited: bool,
}

/// Bounded header/configuration facts; auxiliary hierarchy pixels are never decoded.
struct CoverageMaskHeader<'a> {
    name: String,
    name_limited: bool,
    properties: Vec<Prop<'a>>,
    width: u32,
    height: u32,
    children: Vec<usize>,
}

struct PendingLayerEffects {
    layer: photocraft_doc::LayerId,
    owner: usize,
    bounds: Rect,
    pointers: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PathBinding {
    index: usize,
    geometry_complete: bool,
}

fn allocate_samples(count: usize) -> Result<Vec<f32>> {
    let mut samples = Vec::new();
    samples.try_reserve_exact(count).map_err(|error| IoError::OutOfMemory(format!("XCF drawable samples: {error}")))?;
    samples.resize(count, 0.0);
    Ok(samples)
}

/// Pixel data of a drawable: `channels` normalised samples per pixel, row-major.
struct Pixels {
    width: u32,
    height: u32,
    channels: usize,
    data: Vec<f32>,
    underflow: bool,
    coverage_clamped: bool,
    palette_fallback: bool,
}

impl<'a> Image<'a> {
    fn metadata_limit(&mut self, owner: usize, feature: &str, source: &str) -> Result<()> {
        self.outcomes.notice(
            owner,
            feature,
            ConversionReason::RetentionLimit,
            (source.into(), "Usable native pixels preserved; optional metadata not admitted beyond its retention limits".into()),
            Some("reached optional metadata retention limits; usable pixels were preserved"),
            &mut self.warnings,
        )
    }

    /// Bound validation work independently of the bytes actually retained.
    fn metadata_name(&mut self, cursor: &mut Cursor<'a>, fallback: &str) -> Result<(String, bool)> {
        let raw = cursor.string_bytes()?;
        if raw.len() > self.meta_budget || !work_budget::WorkBudget::new(&mut self.name_work_left).keep_remaining(raw.len()) {
            return Ok((fallback.into(), true));
        }
        let name = match std::str::from_utf8(raw) {
            Ok(valid) => valid.to_owned(),
            Err(_) if raw.len().saturating_mul(3) > self.meta_budget => return Ok((fallback.into(), true)),
            Err(_) => String::from_utf8_lossy(raw).into_owned(),
        };
        let Some(remaining) = self.meta_budget.checked_sub(name.capacity()) else {
            return Ok((fallback.into(), true));
        };
        self.meta_budget = remaining;
        Ok((name, false))
    }

    /// Effect pointers are optional and last in the owner header. A bounded pre-scan
    /// admits the whole list before allocation; omitted lists need not be followed.
    fn optional_effect_pointers(&mut self, cursor: &mut Cursor<'a>, ctl: &Interrupt<'_>) -> Result<(Vec<usize>, bool)> {
        let offset = cursor.pos;
        let before = self.meta_budget;
        let result = (|| -> Result<Option<Vec<usize>>> {
            let mut scan = cursor.at(cursor.pos);
            let charge = 4 * std::mem::size_of::<usize>().max(std::mem::size_of::<u64>());
            for count in 0usize..=1024 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                self.effect_decode_work.charge(if scan.wide { 8 } else { 4 })?;
                let pointer = if scan.wide { (u64::from(scan.u32()?) << 32) | u64::from(scan.u32()?) } else { u64::from(scan.u32()?) };
                if pointer == 0 {
                    return effects::read_pointers(cursor, &mut self.meta_budget, ctl).map(Some);
                }
                if count == 1024 || count.saturating_add(1).saturating_mul(charge) > self.meta_budget {
                    return Ok(None);
                }
            }
            Ok(None)
        })();
        match result {
            Ok(Some(pointers)) => Ok((pointers, false)),
            Ok(None) => Ok((Vec::new(), true)),
            Err(IoError::Cancelled) => Err(IoError::Cancelled),
            Err(IoError::XcfMetadataLimit) => {
                self.meta_budget = before;
                Ok((Vec::new(), true))
            }
            Err(error) => {
                // No pointer list survived the rejected optional parse. Refund only
                // its provisional storage, and keep independently framed pixels usable.
                self.meta_budget = before;
                self.warnings.note("Optional stored effect pointers could not be translated; usable pixels were preserved. See the conversion report");
                self.outcomes.push(ConversionEntry {
                    feature_code: "xcf.effect_list".into(),
                    source: conversion::source("effect_list", 0, offset, "", &[], None),
                    targets: Vec::new(),
                    disposition: ConversionDisposition::RetainedUnapplied,
                    reason: ConversionReason::InvalidOptionalData,
                    source_description: format!("Optional effect pointer list could not be decoded: {error}"),
                    native_description: "No effects from this malformed list were followed; independently framed native pixels preserved".into(),
                    attachments: Vec::new(),
                })?;
                Ok((Vec::new(), false))
            }
        }
    }

    /// Reads a hierarchy (`hptr`): the first level's tiles into `channels` floats per pixel,
    /// `source_channels` as stored (indexed images expand to RGB here).
    fn hierarchy(&mut self, hptr: usize, source_channels: usize, coverage: bool, ctl: &Interrupt<'_>) -> Result<Pixels> {
        self.hierarchy_with_alpha(hptr, source_channels, coverage, false, ctl)
    }

    /// Decode directly into the final native channel layout. Opaque source layers
    /// synthesize alpha in this allocation, rather than retaining a second full buffer.
    fn hierarchy_with_alpha(&mut self, hptr: usize, source_channels: usize, coverage: bool, synthesize_alpha: bool, ctl: &Interrupt<'_>) -> Result<Pixels> {
        self.decode_hierarchy(hptr, source_channels, coverage, synthesize_alpha, None, ctl)
    }

    /// Decode through one XCF-sized tile buffer when the caller can write directly
    /// into native tiles. Channels and layers needing model conversion retain their samples.
    fn decode_hierarchy(
        &mut self,
        hptr: usize,
        source_channels: usize,
        coverage: bool,
        synthesize_alpha: bool,
        mut target: Option<(&mut Surface, Rect)>,
        ctl: &Interrupt<'_>,
    ) -> Result<Pixels> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if synthesize_alpha && (coverage || !matches!((self.base_type, source_channels), (0, 3) | (1 | 2, 1))) {
            return Err(invalid("opaque alpha synthesis does not match the drawable color layout"));
        }
        if !(1..=4).contains(&source_channels) {
            return Err(invalid("drawable channel count is out of range"));
        }
        let mut c = self.c.at(hptr);
        let (w, h, bpp) = (c.u32()?, c.u32()?, c.u32()? as usize);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(invalid(format!("drawable size {w}x{h} is out of range")));
        }
        let expected_bpp = source_channels.checked_mul(self.precision.bytes).ok_or_else(|| invalid("bpp overflow"))?;
        if bpp != expected_bpp {
            return Err(invalid(format!("{bpp} bytes per pixel where {expected_bpp} were expected")));
        }
        let indexed = self.base_type == 2;
        let channels = (if indexed { source_channels + 2 } else { source_channels }) + usize::from(synthesize_alpha);
        let count = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| invalid("drawable overflow"))?;
        let bytes = count.checked_mul(4).ok_or_else(|| invalid("drawable overflow"))?;
        let scratch_count = if target.is_some() { w.min(TILE) as usize * h.min(TILE) as usize * channels } else { count };
        self.budget = self.budget.checked_sub(scratch_count * 4).ok_or_else(|| invalid("the layers exceed the 1 GiB pixel limit"))?;
        if let Some((surface, rect)) = &target {
            if (rect.width(), rect.height()) != (w, h) || surface.channels() != channels {
                return Err(invalid("hierarchy dimensions or layout differ from its declared drawable"));
            }
            // Retain the logical sample charge as an admission floor. The full
            // native tile allocation must also fit while the bounded scratch is live.
            let tiles = usize::try_from(photocraft_raster::tile_storage_bytes(*rect, &surface.format())).unwrap_or(usize::MAX);
            if tiles.max(bytes) > self.budget {
                return Err(invalid("the layers exceed the 1 GiB pixel limit"));
            }
        }
        let lptr = c.ptr()?;
        let mut l = self.c.at(lptr);
        let (lw, lh) = (l.u32()?, l.u32()?);
        if (lw, lh) != (w, h) {
            return Err(invalid("the level does not match its hierarchy"));
        }
        let across = w.div_ceil(TILE) as usize;
        let down = h.div_ceil(TILE) as usize;
        let mut tiles = Vec::with_capacity(across * down);
        for _ in 0..across * down {
            let p = l.ptr()?;
            if p == 0 {
                return Err(invalid("the tile list ends before every tile"));
            }
            tiles.push(p);
        }
        if l.ptr_wire()? != 0 {
            return Err(invalid("the tile list is missing its zero terminator"));
        }
        let mut data = allocate_samples(scratch_count)?;
        let mut underflow = false;
        let mut coverage_clamped = false;
        let mut palette_fallback = false;
        let stride = w as usize * channels;
        for (i, &at) in tiles.iter().enumerate() {
            if i % 64 == 0 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let (tx, ty) = ((i % across) as u32, (i / across) as u32);
            let (tw, th) = ((w - tx * TILE).min(TILE), (h - ty * TILE).min(TILE));
            let end = tiles.get(i + 1).copied().filter(|&n| n > at).unwrap_or(self.c.b.len());
            let raw = decode_tile(&self.c, at, end, self.compression, bpp, (tw * th) as usize)?;
            let (first, row_stride) = if target.is_some() {
                (0, tw as usize * channels)
            } else {
                (ty as usize * TILE as usize * stride + tx as usize * TILE as usize * channels, stride)
            };
            if !indexed {
                let samples = tw as usize * channels;
                let row_bytes = tw as usize * bpp;
                for py in 0..th as usize {
                    let src = py * row_bytes;
                    let dst = first + py * row_stride;
                    let input = raw.get(src..src + row_bytes).ok_or_else(|| invalid("tile row is cut short"))?;
                    let output = data.get_mut(dst..dst + samples).ok_or_else(|| invalid("tile row exceeds its drawable"))?;
                    let decoded_samples = tw as usize * source_channels;
                    let decoded = output.get_mut(..decoded_samples).ok_or_else(|| invalid("native row has no source sample prefix"))?;
                    let changes = row_samples(input, decoded, self.precision, self.sample_order, source_channels, coverage).map_err(|error| {
                        let ch = error.sample % source_channels;
                        let px = error.sample / source_channels;
                        let is_coverage = coverage || (matches!(source_channels, 2 | 4) && ch + 1 == source_channels);
                        invalid(format!(
                            "{} plane {ch}, pixel ({}, {}): {}",
                            if is_coverage { "coverage" } else { "color" },
                            tx as usize * TILE as usize + px,
                            ty as usize * TILE as usize + py,
                            error.message,
                        ))
                    })?;
                    underflow |= changes.underflow;
                    coverage_clamped |= changes.coverage_clamped;
                    if synthesize_alpha {
                        // Expand backwards within this final row so each unread
                        // source sample survives overlapping native destinations.
                        for pixel in (0..tw as usize).rev() {
                            for channel in (0..source_channels).rev() {
                                let value = *output.get(pixel * source_channels + channel).ok_or_else(|| invalid("source row sample is missing"))?;
                                *output.get_mut(pixel * channels + channel).ok_or_else(|| invalid("native row sample is missing"))? = value;
                            }
                            *output.get_mut(pixel * channels + source_channels).ok_or_else(|| invalid("native alpha sample is missing"))? = 1.0;
                        }
                    }
                }
            } else {
                for py in 0..th as usize {
                    for px in 0..tw as usize {
                        let src = (py * tw as usize + px) * bpp;
                        let dst = first + py * row_stride + px * channels;
                        let end = src.checked_add(bpp).ok_or_else(|| invalid("pixel byte range overflows"))?;
                        let pixel = raw.get(src..end).ok_or_else(|| invalid("tile pixel is cut short"))?;
                        let end = dst.checked_add(channels).ok_or_else(|| invalid("pixel sample range overflows"))?;
                        let output = data.get_mut(dst..end).ok_or_else(|| invalid("tile pixel exceeds its drawable"))?;
                        let idx = usize::from(*pixel.first().ok_or_else(|| invalid("indexed pixel has no palette index"))?);
                        let [r, g, b] = self.colormap.get(idx).copied().unwrap_or_else(|| {
                            palette_fallback = true;
                            [idx as u8; 3]
                        });
                        let rgb = output.get_mut(..3).ok_or_else(|| invalid("indexed pixel has no RGB destination"))?;
                        rgb.copy_from_slice(&[f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0]);
                        if synthesize_alpha {
                            *output.get_mut(3).ok_or_else(|| invalid("indexed pixel has no native alpha destination"))? = 1.0;
                        } else if source_channels == 2 {
                            let alpha = pixel.get(1).ok_or_else(|| invalid("indexed pixel has no alpha sample"))?;
                            *output.get_mut(3).ok_or_else(|| invalid("indexed pixel has no alpha destination"))? = f32::from(*alpha) / 255.0;
                        }
                    }
                }
            }
            if let Some((surface, rect)) = &mut target {
                // The tile lies within the checked full rectangle.
                let x = rect.x0 + (tx * TILE) as i32;
                let y = rect.y0 + (ty * TILE) as i32;
                let tile = Rect::new(x, y, x + tw as i32, y + th as i32);
                let samples = data.get(..tw as usize * th as usize * channels).ok_or_else(|| invalid("native tile samples are missing"))?;
                surface.try_write_region(tile, samples).map_err(drawable_tile_error)?;
            }
        }
        if let Some((surface, _)) = target {
            surface.prune();
            let stored = photocraft_raster::stored_tiles_bytes(surface.tile_count() as u64, &surface.format());
            self.budget = self
                .budget
                .checked_sub(bytes.max(usize::try_from(stored).unwrap_or(usize::MAX)))
                .ok_or_else(|| invalid("the layers exceed the 1 GiB pixel limit"))?;
            data = Vec::new();
        }
        Ok(Pixels { width: w, height: h, channels, data, underflow, coverage_clamped, palette_fallback })
    }

    /// A channel structure (a layer mask, the selection or an extra channel) as one-channel
    /// pixels, with its properties.
    fn channel(&mut self, ptr: usize, expected: (u32, u32), layer_mask: bool, ctl: &Interrupt<'_>) -> Result<Channel<'a>> {
        let mut c = self.c.at(ptr);
        let (width, height) = (c.u32()?, c.u32()?);
        let (name, name_limited) = self.metadata_name(&mut c, "Channel")?;
        let props = props(&mut c)?;
        if (layer_mask || metadata::active(&props, 4)) && (width, height) != expected {
            let (expected_width, expected_height) = expected;
            let (channel, owner) = if layer_mask { ("layer mask", "layer") } else { ("saved selection", "canvas") };
            return Err(invalid(format!("{channel} dimensions {width}x{height} differ from the {owner}'s {expected_width}x{expected_height}")));
        }
        let hptr = c.ptr()?;
        let (effect_pointers, effects_limited) = if self.version >= 26 { self.optional_effect_pointers(&mut c, ctl)? } else { (Vec::new(), false) };
        let saved_base_type = self.base_type;
        // Channels are always one plain sample per pixel, even in indexed images.
        self.base_type = 0;
        let px = self.hierarchy(hptr, 1, true, ctl).map_err(|error| match error {
            error @ (IoError::Cancelled | IoError::OutOfMemory(_)) => error,
            error => invalid(format!("channel or mask '{name}': {error}")),
        });
        self.base_type = saved_base_type;
        let px = px?;
        if (px.width, px.height) != (width, height) {
            return Err(invalid(format!("channel or mask '{name}' hierarchy dimensions differ from the declared {width}x{height}")));
        }
        Ok(Channel { name, properties: props, pixels: px, effects: effect_pointers, name_limited, effects_limited })
    }

    /// Follow only bounded mask framing and child-effect references. The hierarchy
    /// pointer is read as inert data and is never followed or materialized.
    fn coverage_mask_header(&mut self, offset: usize, ctl: &Interrupt<'_>) -> Result<CoverageMaskHeader<'a>> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        self.effect_decode_work.charge(64)?;
        let mut c = self.c.at(offset);
        let (width, height) = (c.u32()?, c.u32()?);
        validate_canvas(width, height)?;
        // Account for the name field in the effect traversal's own work limit.
        let mut name_cursor = c.at(c.pos);
        self.effect_decode_work.charge(name_cursor.string_bytes()?.len().saturating_add(4))?;
        let (name, name_limited) = self.metadata_name(&mut c, "Mask")?;
        let properties = wire::props_with_work(&mut c, ctl, &mut |bytes| self.effect_decode_work.charge(bytes))?;
        self.effect_decode_work.charge(8)?;
        c.ptr_wire()?;
        let children =
            if self.version >= 26 { effects::read_pointers_with_work(&mut c, &mut self.meta_budget, &mut self.effect_decode_work, ctl)? } else { Vec::new() };
        // metadata_name already charged the admitted name allocation. The fallback
        // is bounded; never materialize a source-sized name before checking its allowance.
        let bytes = std::mem::size_of::<CoverageMaskHeader<'_>>().saturating_add(properties.len().saturating_mul(std::mem::size_of::<Prop<'_>>()));
        self.meta_budget = self.meta_budget.checked_sub(bytes).ok_or(IoError::XcfMetadataLimit)?;
        Ok(CoverageMaskHeader { name, name_limited, properties, width, height, children })
    }

    /// Current P2 has no coverage adapters: inventory source configuration once,
    /// leaving the supported native owner's pixels untouched.
    fn inventory_coverage_effects(&mut self, owner: usize, pointers: &[usize], ctl: &Interrupt<'_>) -> Result<()> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if pointers.is_empty() {
            return Ok(());
        }
        self.outcomes.warning(
            owner,
            "has stored coverage effects retained as inactive configuration; auxiliary mask pixels are not retained; see the conversion report",
            &mut self.warnings,
        )?;
        if !self.outcomes.recording() {
            return Ok(());
        }
        let owner_entry = self.outcomes.entry(owner)?.clone();
        let Some(remaining) = self.meta_budget.checked_sub(pointers.len().saturating_mul(6 * std::mem::size_of::<usize>())) else {
            return self.metadata_limit(owner, "xcf.coverage_effect", "Coverage-effect inventory exceeds the remaining metadata allowance");
        };
        self.meta_budget = remaining;
        let mut work: Vec<(usize, usize, usize)> = pointers.iter().rev().map(|pointer| (*pointer, 0, owner)).collect();
        let mut effects_seen = std::collections::HashSet::new();
        let mut masks_seen = std::collections::HashSet::new();
        if let Some(offset) = owner_entry.source.record_offset.and_then(|offset| usize::try_from(offset).ok()) {
            masks_seen.insert(offset);
        }
        let mut ordinal = 0u32;
        while let Some((pointer, depth, source_owner)) = work.pop() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if !self.outcomes.recording() {
                return Ok(());
            }
            let current_owner = self.outcomes.entry(source_owner)?.clone();
            if effects_seen.len() >= MAX_LAYERS {
                return self.metadata_limit(owner, "xcf.coverage_effect", "Coverage-effect inventory reached its bounded record limit");
            }
            let mut source = current_owner.source.clone();
            source.kind = format!("{}_effect", current_owner.source.kind);
            source.ordinal = ordinal;
            source.record_offset = Some(pointer as u64);
            ordinal = ordinal.checked_add(1).ok_or_else(|| invalid("coverage effect ordinal overflow"))?;
            if depth > MAX_DEPTH || !effects_seen.insert(pointer) {
                self.outcomes.push(ConversionEntry {
                    feature_code: "xcf.coverage_effect".into(),
                    source,
                    targets: current_owner.targets.clone(),
                    disposition: ConversionDisposition::RetainedUnapplied,
                    reason: ConversionReason::InvalidOptionalData,
                    source_description: "Repeated or excessively nested coverage-effect reference".into(),
                    native_description: "Native coverage preserved; cyclic source effect configuration was not followed".into(),
                    attachments: Vec::new(),
                })?;
                continue;
            }
            let Some(remaining) = self.meta_budget.checked_sub(8 * std::mem::size_of::<usize>()) else {
                return self.metadata_limit(owner, "xcf.coverage_effect", "Coverage-effect inventory exceeds the remaining metadata allowance");
            };
            self.meta_budget = remaining;
            let before_effect = self.meta_budget;
            let effect = match effects::decode_with_work(&self.c, pointer, self.version, &mut self.meta_budget, &mut self.effect_decode_work, ctl) {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(IoError::XcfMetadataLimit) => {
                    self.meta_budget = before_effect;
                    return self.metadata_limit(owner, "xcf.coverage_effect", "Coverage-effect configuration exceeds the metadata or decode-work allowance");
                }
                Err(error) => {
                    // Failed decoding retains no effect allocation; preserve only the caller's traversal charges.
                    self.meta_budget = before_effect;
                    self.outcomes.push(ConversionEntry {
                        feature_code: "xcf.coverage_effect".into(),
                        source,
                        targets: current_owner.targets.clone(),
                        disposition: ConversionDisposition::RetainedUnapplied,
                        reason: ConversionReason::InvalidOptionalData,
                        source_description: format!("Coverage-effect framing could not be decoded: {error}"),
                        native_description: "Usable native coverage preserved; malformed optional effect was not applied".into(),
                        attachments: Vec::new(),
                    })?;
                    continue;
                }
            };
            source.name = if effect.name.is_empty() { effect.operation } else { effect.name }.into();
            let mut configuration_bytes = 0usize;
            let mut string_bytes = 0usize;
            let mut invalid_arguments = Vec::new();
            for argument in &effect.arguments {
                match &argument.value {
                    effects::ArgumentValue::Config(config) => configuration_bytes = configuration_bytes.saturating_add(config.raw.len()),
                    effects::ArgumentValue::String(text) => string_bytes = string_bytes.saturating_add(text.len()),
                    effects::ArgumentValue::Invalid { kind, bytes, message } if invalid_arguments.len() < 4 => {
                        invalid_arguments.push(format!("argument type {kind}, {} bytes: {message}", bytes.len()));
                    }
                    _ => {}
                }
            }
            let reason =
                if effect.errors.is_empty() && invalid_arguments.is_empty() { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData };
            let native_description =
                "Native coverage is unchanged; conversion is not implemented. Source effect configuration remains inert and is not executed".into();
            let outcome = self.outcomes.push(ConversionEntry {
                feature_code: "xcf.coverage_effect".into(),
                source,
                targets: current_owner.targets.clone(),
                disposition: ConversionDisposition::RetainedUnapplied,
                reason,
                source_description: format!(
                    "{} version {}; visible {}, opacity {}; {} ordered arguments ({} config bytes, {} string bytes); inert icon {}; {} {}",
                    effect.operation,
                    effect.version.unwrap_or("absent"),
                    effect.visible,
                    effect.opacity,
                    effect.arguments.len(),
                    configuration_bytes,
                    string_bytes,
                    effect.icon,
                    effect.errors.join("; "),
                    invalid_arguments.join("; ")
                ),
                native_description,
                attachments: Vec::new(),
            })?;
            if !self.outcomes.recording() {
                return Ok(());
            }
            self.outcomes.property_with_reason(outcome, 45, effect.record_offset, effect.raw, reason)?;
            for property in effect.unknown_properties {
                self.outcomes.property(outcome, property.kind, property.offset, property.bytes)?;
            }
            if effect.mask_offset != 0 && masks_seen.insert(effect.mask_offset) {
                let start = effect.mask_offset;
                let effect_source = self.outcomes.entry(outcome)?.source.clone();
                let before_mask = self.meta_budget;
                let record = match self.coverage_mask_header(start, ctl) {
                    Ok(record) => record,
                    Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                    Err(IoError::XcfMetadataLimit) => {
                        self.meta_budget = before_mask;
                        self.metadata_limit(outcome, "xcf.coverage_effect_mask", "Auxiliary mask configuration exceeds the metadata or decode-work allowance")?;
                        continue;
                    }
                    Err(error) => {
                        // A failed optional header retains no child list or header allocation.
                        // Keep the outer effect/traversal debits, which precede this snapshot.
                        self.meta_budget = before_mask;
                        self.outcomes.push(ConversionEntry {
                            feature_code: "xcf.coverage_effect_mask".into(),
                            source: conversion::source("coverage_effect_mask", effect_source.ordinal as usize, start, "", &effect_source.item_path, None),
                            targets: current_owner.targets.clone(),
                            disposition: ConversionDisposition::SkippedNonessential,
                            reason: ConversionReason::InvalidOptionalData,
                            source_description: format!("Optional effect-mask header could not be decoded: {error}"),
                            native_description:
                                "Native owner coverage is preserved; auxiliary mask pixels are not retained and the source stage remains inactive".into(),
                            attachments: Vec::new(),
                        })?;
                        continue;
                    }
                };
                let mask_outcome = self.outcomes.push(ConversionEntry {
                    feature_code: "xcf.coverage_effect_mask".into(),
                    source: conversion::source("coverage_effect_mask", effect_source.ordinal as usize, start, &record.name, &effect_source.item_path, None),
                    targets: current_owner.targets.clone(),
                    disposition: ConversionDisposition::SkippedNonessential,
                    reason: ConversionReason::NativeLimitation,
                    source_description: format!("Source effect-mask coverage {}x{}; {} nested effects", record.width, record.height, record.children.len()),
                    native_description:
                        "Source auxiliary mask pixels are not retained; only mask identity and configuration remain inert with the unapplied coverage stage"
                            .into(),
                    attachments: Vec::new(),
                })?;
                if !self.outcomes.recording() {
                    return Ok(());
                }
                if record.name_limited {
                    self.metadata_limit(
                        mask_outcome,
                        "xcf.coverage_effect_mask_name",
                        "Auxiliary mask name omitted after the shared metadata allowance was exhausted",
                    )?;
                }
                for property in &record.properties {
                    self.outcomes.property(mask_outcome, property.kind, property.offset, property.data)?;
                }
                let Some(remaining) = self.meta_budget.checked_sub(record.children.len().saturating_mul(6 * std::mem::size_of::<usize>())) else {
                    return self.metadata_limit(outcome, "xcf.coverage_effect", "Nested coverage-effect inventory exceeds the remaining metadata allowance");
                };
                self.meta_budget = remaining;
                work.extend(record.children.iter().rev().map(|pointer| (*pointer, depth + 1, mask_outcome)));
            }
        }
        Ok(())
    }

    /// Records per-owner numerical loss from coverage normalization or native precision narrowing.
    fn precision_underflow(&mut self, owner: usize, pixels: &Pixels) -> Result<()> {
        if pixels.palette_fallback {
            self.outcomes.notice(
                owner,
                "xcf.indexed_palette_fallback",
                ConversionReason::InvalidOptionalData,
                (
                    "Stored palette indices have no matching color entries".into(),
                    "Missing entries interpreted as grayscale index values; stored alpha preserved".into(),
                ),
                Some("has missing palette colors; unmatched indices open as grayscale with their stored alpha"),
                &mut self.warnings,
            )?;
        }
        if pixels.coverage_clamped {
            self.outcomes.notice(
                owner,
                "xcf.coverage_clamped",
                ConversionReason::NumericalConversion,
                (
                    "Finite stored coverage outside 0 to 1".into(),
                    "Coverage clamped to the native 0 to 1 domain; signed HDR color samples remain unchanged".into(),
                ),
                Some("has finite coverage samples clamped to the native 0 to 1 range"),
                &mut self.warnings,
            )?;
        }
        if pixels.underflow {
            self.outcomes.notice(
                owner,
                "xcf.precision_underflow",
                ConversionReason::NumericalConversion,
                ("Nonzero stored samples smaller than native F32 can represent".into(), "Samples rounded to signed zero in the native surface".into()),
                Some("has tiny samples that rounded to zero at native F32 precision"),
                &mut self.warnings,
            )?;
        }
        Ok(())
    }
}

/// A layer as read, before the tree is built.
struct Node {
    layer: Layer,
    source_ordinal: usize,
    path: Vec<u32>,
    path_omitted: bool,
    group: bool,
    children: Vec<Node>,
    outcome: usize,
}

/// Map source layer modes to native behavior, with a note when their results differ.
fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("legacy Soft light")),
        45 => (B::SoftLight, Some("Soft light")),
        23 => (B::Overlay, None),
        6 | 32 => (B::Difference, None),
        7 | 33 => (B::LinearDodge, None),
        8 | 34 => (B::Subtract, None),
        9 | 35 => (B::Darken, None),
        10 | 36 => (B::Lighten, None),
        11 | 37 => (B::Hue, Some("HSV Hue")),
        12 | 38 => (B::Saturation, Some("HSV Saturation")),
        13 | 39 => (B::Color, Some("HSL Color")),
        14 | 40 => (B::Luminosity, Some("HSV Value")),
        15 | 41 => (B::Divide, None),
        16 | 42 => (B::ColorDodge, None),
        17 | 43 => (B::ColorBurn, None),
        18 | 44 => (B::HardLight, None),
        20 | 46 => (B::Normal, Some("Grain extract")),
        21 | 47 => (B::Normal, Some("Grain merge")),
        22 | 57 => (B::Normal, Some("Color erase")),
        24 => (B::Hue, Some("LCH Hue")),
        25 => (B::Saturation, Some("LCH Chroma")),
        26 => (B::Color, Some("LCH Color")),
        27 => (B::Luminosity, Some("LCH Lightness")),
        48 => (B::VividLight, None),
        49 => (B::PinLight, None),
        50 => (B::LinearLight, None),
        51 => (B::HardMix, None),
        52 => (B::Exclusion, None),
        53 => (B::LinearBurn, None),
        54 => (B::DarkerColor, Some("Luma darken only")),
        55 => (B::LighterColor, Some("Luma lighten only")),
        56 => (B::Luminosity, Some("Luminance")),
        58 => (B::Normal, Some("Erase")),
        59 => (B::Normal, Some("Merge")),
        60 => (B::Normal, Some("Split")),
        61 => (B::PassThrough, None),
        _ => (B::Normal, Some("an unknown mode")),
    }
}

fn label(tag: u32) -> LabelColor {
    match tag {
        1 => LabelColor::Blue,
        2 => LabelColor::Green,
        3 => LabelColor::Yellow,
        4 => LabelColor::Orange,
        6 => LabelColor::Red,
        7 => LabelColor::Violet,
        8 => LabelColor::Gray,
        _ => LabelColor::None,
    }
}

/// Writes `px` (its channels matching `surface`'s) at `x, y` of the document, charging the
/// tiles that creates to `budget`: the drawable was admitted by its logical samples, and a
/// narrow drawable (or one referenced by many layer pointers) stores far more in whole tiles.
fn write_pixels(surface: &mut Surface, px: &Pixels, x: i32, y: i32, budget: &mut usize) -> Result<()> {
    let rect = drawable_rect(px.width, px.height, x, y)?;
    let expected = (px.width as usize)
        .checked_mul(px.height as usize)
        .and_then(|pixels| pixels.checked_mul(px.channels))
        .ok_or_else(|| invalid("drawable sample count overflow"))?;
    if surface.channels() != px.channels || px.data.len() != expected {
        return Err(invalid("pixel data does not match the drawable"));
    }
    // Checked before the write (it allocates the whole rect's tiles); only the tiles kept after
    // `prune()` are charged.
    let tiles = usize::try_from(photocraft_raster::tile_storage_bytes(rect, &surface.format())).unwrap_or(usize::MAX);
    if tiles > *budget {
        return Err(invalid("the layers exceed the 1 GiB pixel limit"));
    }
    let before = surface.tile_count();
    surface.try_write_region(rect, &px.data).map_err(drawable_tile_error)?;
    surface.prune();
    let added = surface.tile_count().saturating_sub(before) as u64;
    *budget = budget.saturating_sub(usize::try_from(photocraft_raster::stored_tiles_bytes(added, &surface.format())).unwrap_or(usize::MAX));
    Ok(())
}

fn drawable_rect(width: u32, height: u32, x: i32, y: i32) -> Result<Rect> {
    let w = i32::try_from(width).map_err(|_| invalid("drawable width overflow"))?;
    let h = i32::try_from(height).map_err(|_| invalid("drawable height overflow"))?;
    if w == 0 || h == 0 {
        return Err(invalid("drawable dimensions must be positive"));
    }
    Ok(Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?))
}

fn drawable_tile_error(error: AllocationError) -> IoError {
    let message = format!("drawable tile write failed: {error}");
    match error {
        AllocationError::NotEnoughMemory { .. } => IoError::OutOfMemory(message),
        _ => invalid(message),
    }
}

/// Parses legacy or numeric XCF version tags and rejects malformed header framing.
fn read_version(c: &mut Cursor<'_>) -> Result<u32> {
    let tag = c.take(5)?;
    match tag {
        b"file\0" => Ok(0),
        [b'v', a, b, d, 0] if a.is_ascii_digit() && b.is_ascii_digit() && d.is_ascii_digit() => {
            Ok(u32::from(a - b'0') * 100 + u32::from(b - b'0') * 10 + u32::from(d - b'0'))
        }
        _ => Err(invalid("unknown XCF version tag")),
    }
}

/// Checks nonzero dimensions and the shared canvas limits before document or pixel allocation.
fn validate_canvas(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(invalid(format!("canvas {w}x{h} must be 1 to {MAX_SIDE} pixels a side")));
    }
    if (w as usize).saturating_mul(h as usize).saturating_mul(16) > MAX_PIXEL_BYTES {
        return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
    }
    Ok(())
}

/// Returns an explicit byte-order choice request for historically ambiguous high-depth storage.
fn sample_order_request(version: u32, precision: Precision) -> Result<Option<crate::HistoricalXcfSampleOrderRequest>> {
    if (7..=11).contains(&version) && precision.bytes > 1 {
        let bits_per_sample = precision.bytes.checked_mul(8).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| invalid("precision bit count overflows"))?;
        Ok(Some(crate::HistoricalXcfSampleOrderRequest { version, bits_per_sample }))
    } else {
        Ok(None)
    }
}

/// Shared validated framing for both bounded probing and complete import.
struct Header {
    version: u32,
    width: u32,
    height: u32,
    base_type: u32,
    precision: Precision,
}

fn read_header(bytes: &[u8]) -> Result<(Cursor<'_>, Header)> {
    if !is_xcf(bytes) {
        return Err(invalid("not an XCF file"));
    }
    let mut c = Cursor { b: bytes, pos: MAGIC.len(), wide: false };
    let version = read_version(&mut c)?;
    c.wide = version >= 11;
    let (width, height, base_type) = (c.u32()?, c.u32()?, c.u32()?);
    validate_canvas(width, height)?;
    if base_type > 2 {
        return Err(invalid(format!("unknown image type {base_type}")));
    }
    let precision = if version >= 4 { Precision::from_header(version, c.u32()?)? } else { Precision::GAMMA8 };
    if base_type == 2 && precision != Precision::GAMMA8 {
        return Err(invalid("indexed pixels require 8-bit nonlinear integer precision"));
    }
    Ok((c, Header { version, width, height, base_type, precision }))
}

/// Inspects only the shared fixed header, without decoding document pixels.
pub(crate) fn historical_sample_order_request(bytes: &[u8]) -> Result<Option<crate::HistoricalXcfSampleOrderRequest>> {
    let (_, header) = read_header(bytes)?;
    sample_order_request(header.version, header.precision)
}

/// Imports unwrapped XCF; ambiguous historical samples require an explicit choice.
#[cfg(test)]
pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    import_with_sample_order(name, bytes, None, ctl)
}

/// Validated decoding facts plus the original version-aware properties retained by native metadata conversion.
struct ImageMetadata<'a> {
    properties: Vec<Prop<'a>>,
    compression: Compression,
    colormap: Vec<[u8; 3]>,
    icc: Option<Vec<u8>>,
    parasite_work: wire::ParasiteWork,
    parasites_limited: bool,
}

/// Indexed palette validity is essential; unused palette bytes remain optional source data.
fn palette_count(data: &[u8], version: u32) -> Result<usize> {
    let n = be_u32(data).ok_or_else(|| invalid("the palette is missing its color count"))? as usize;
    let components = if version == 0 { 1 } else { 3 };
    if n > 256 || data.len() != 4 + n * components {
        return Err(invalid("the palette does not contain its declared count of at most 256 colors"));
    }
    Ok(n)
}

fn palette_colors(data: &[u8], version: u32) -> Result<Vec<[u8; 3]>> {
    let n = palette_count(data, version)?;
    if version == 0 {
        // Early writers did not serialize usable RGB entries.
        Ok((0..n).map(|value| [value as u8; 3]).collect())
    } else {
        Ok(data.get(4..).unwrap_or(&[]).as_chunks::<3>().0.to_vec())
    }
}

/// Decode only drawable prerequisites here; native metadata conversion consumes the retained properties later.
#[cfg(test)]
fn read_image_metadata<'a>(c: &mut Cursor<'a>, version: u32, ctl: &Interrupt<'_>) -> Result<ImageMetadata<'a>> {
    read_image_metadata_with_work(c, version, true, wire::ParasiteWork::default(), ctl)
}

/// An incomplete last-wins scan cannot certify a prefix profile as the final assignment.
fn read_image_metadata_with_work<'a>(
    c: &mut Cursor<'a>,
    version: u32,
    indexed: bool,
    mut parasite_work: wire::ParasiteWork,
    ctl: &Interrupt<'_>,
) -> Result<ImageMetadata<'a>> {
    // Image properties.
    let image_props = wire::props_image_with_interrupt(c, version, indexed, ctl)?;
    let mut compression = Compression::None;
    let mut colormap = Vec::new();
    let mut selected_icc = None;
    let mut parasites_limited = false;
    for p in &image_props {
        match p.kind {
            1 if indexed => colormap = palette_colors(p.data, version)?,
            17 => {
                compression = match p.data {
                    [0] => Compression::None,
                    [1] => Compression::Rle,
                    [2] => Compression::Zlib,
                    [k] => return Err(invalid(format!("unknown compression {k}"))),
                    _ => return Err(invalid("compression property must contain exactly one byte")),
                }
            }
            21 => {
                let mut last = None;
                let scan = wire::scan_parasites_with_work(p.data, &mut parasite_work, ctl, |record| {
                    if record.name == b"icc-profile" {
                        last = Some(record.payload);
                    }
                    Ok(true)
                })?;
                parasites_limited |= scan.limited;
                if let Some(data) = last {
                    selected_icc = Some(data);
                }
            }
            _ => {}
        }
    }
    // The property allowance is also the fixed prerequisite-profile copy bound. Scan every
    // record first so only the final assignment is copied, never unused parasites/profiles.
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let icc = selected_icc.filter(|_| !parasites_limited).map(<[u8]>::to_vec);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(ImageMetadata { properties: image_props, compression, colormap, icc, parasite_work, parasites_limited })
}

pub(crate) fn source_geometry(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<((f64, f64), f64)>> {
    geometry::source_geometry(name, bytes, ctl)
}

/// Imports XCF with an explicit historical sample-order decision, preserving native data and durable
/// conversion outcomes.
pub(crate) fn import_with_sample_order(
    name: &str,
    bytes: &[u8],
    selected_order: Option<crate::HistoricalXcfSampleOrder>,
    ctl: &Interrupt<'_>,
) -> Result<ImportResult> {
    import_with_sample_order_and_parasite_work(name, bytes, selected_order, wire::ParasiteWork::default(), wire::ParasiteWork::default(), ctl)
}

/// Keep the same scan allowance from prerequisite color selection through all owner/report scans.
fn import_with_sample_order_and_parasite_work(
    name: &str,
    bytes: &[u8],
    selected_order: Option<crate::HistoricalXcfSampleOrder>,
    parasite_work: wire::ParasiteWork,
    report_parasite_work: wire::ParasiteWork,
    ctl: &Interrupt<'_>,
) -> Result<ImportResult> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let (mut c, Header { version, width: w, height: h, base_type, precision }) = read_header(bytes)?;
    let mut warnings = Warnings::default();
    if version > KNOWN_VERSION {
        warnings.note(&format!("XCF version {version} is newer than this reader (version {KNOWN_VERSION}); unknown parts were skipped"));
    }
    // Development writers did not define high-depth sample byte order in XCF7–11.
    // Neither source pixels nor producer names establish the original host order.
    let request = sample_order_request(version, precision)?;
    let historical_sample_order_ambiguous = request.is_some();
    let sample_order = match request {
        Some(request) => selected_order.ok_or(IoError::HistoricalXcfSampleOrderRequired(request))?,
        None => crate::HistoricalXcfSampleOrder::BigEndian,
    };
    if historical_sample_order_ambiguous {
        warnings.note(&format!(
            "Historical high-depth XCF version {version} has unspecified sample byte order; the explicit {sample_order} choice was used. Verify this choice against the original source"
        ));
    }
    let mode = match base_type {
        0 => ColorMode::Rgb,
        1 => ColorMode::Grayscale,
        2 => ColorMode::Rgb,
        t => return Err(invalid(format!("unknown image type {t}"))),
    };
    let depth = precision.depth();
    if base_type == 2 {
        warnings.note("the indexed image opens as RGB");
    }
    if !precision.float && precision.bytes == 4 {
        warnings.note("32-bit integer samples open as native F32 with reduced integer precision");
    } else if precision.float && precision.bytes == 8 {
        warnings.note("64-bit floating-point samples open as native F32 with reduced precision");
    }

    let ImageMetadata { properties: image_props, compression, colormap, icc, parasite_work, parasites_limited } =
        read_image_metadata_with_work(&mut c, version, base_type == 2, parasite_work, ctl)?;
    let assigned_profile = icc.is_some();
    let color::ProfileSelection { bytes: native_profile, fallback_reason, tag_document } = select_image_profile(mode, precision, icc, parasites_limited)?;
    if let Some(reason) = &fallback_reason {
        warnings.note(&format!(
            "The assigned ICC profile could not be used: {reason}. Original samples use the implicit {} profile for their encoding; assigned colorants could not be preserved",
            if mode == ColorMode::Grayscale { "gray" } else { "RGB" }
        ));
    }
    let native_color_error = text::color_transform(&native_profile).err().map(|error| error.to_string());

    let mut layer_ptrs = Vec::new();
    loop {
        let p = c.ptr()?;
        if p == 0 {
            break;
        }
        layer_ptrs.push(p);
        if layer_ptrs.len() > MAX_LAYERS {
            return Err(invalid(format!("more than {MAX_LAYERS} layers")));
        }
    }
    let mut channel_ptrs = Vec::new();
    loop {
        let p = c.ptr()?;
        if p == 0 {
            break;
        }
        channel_ptrs.push(p);
        if channel_ptrs.len() > MAX_CHANNELS {
            return Err(invalid(format!("more than {MAX_CHANNELS} channels")));
        }
    }
    let mut path_ptrs = Vec::new();
    let mut omitted_paths: Option<(usize, usize)> = None;
    if version >= 18 {
        loop {
            let pointer = c.ptr_wire()?;
            if pointer == 0 {
                break;
            }
            // An unaddressable offset is invalid optional path data, like any offset outside the file.
            let pointer = usize::try_from(pointer).unwrap_or(usize::MAX);
            if path_ptrs.len() == MAX_CHANNELS {
                let omitted = omitted_paths.get_or_insert((pointer, 0));
                omitted.1 = omitted.1.saturating_add(1);
                if omitted.1.is_multiple_of(1024) {
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                }
            } else {
                path_ptrs.push(pointer);
            }
        }
    }

    let mut image = Image {
        c: c.at(0),
        version,
        precision,
        sample_order,
        compression,
        base_type,
        colormap,
        budget: MAX_PIXEL_BYTES,
        meta_budget: MAX_METADATA_BYTES,
        name_work_left: MAX_METADATA_BYTES,
        vector_style_work_left: MAX_METADATA_BYTES,
        link_work_left: MAX_METADATA_BYTES,
        effect_decode_work: Default::default(),
        text_decode_work: Default::default(),
        parasite_work,
        report_parasite_work,
        point_text_work_left: crate::xcf::text::POINT_LAYOUT_WORK_UNITS,
        warnings,
        outcomes: conversion::Outcomes::new(version)?,
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        source_resolution: metadata::effective_resolution(&image_props).unwrap_or((72.0, 72.0)),
        pending_effects: Vec::new(),
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
        native_color_error,
    };
    let fmt = PixelFormat::new(mode, depth, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let image_outcome = image.outcomes.push(ConversionEntry {
        feature_code: "xcf.canvas".into(),
        source: conversion::source("image", 0, 0, "", &[], None),
        targets: vec![NativeTargetRef { label: "Document canvas".into(), ..Default::default() }],
        disposition: ConversionDisposition::NativeEquivalent,
        reason: ConversionReason::NativePolicy,
        source_description: format!("Canvas {w} × {h}"),
        native_description: "Native document canvas dimensions".into(),
        attachments: Vec::new(),
    })?;
    if parasites_limited {
        image.metadata_limit(
            image_outcome,
            "xcf.parasite_work",
            "Image parasite scan work exceeded its file allowance; incomplete selections were not applied",
        )?;
    }
    if base_type == 2 {
        image.outcomes.detail(
            image_outcome,
            "xcf.indexed_color",
            ConversionReason::NativePolicy,
            "Indexed source pixels and palette".into(),
            "Expanded native RGB pixels; palette-index editing is not preserved".into(),
        )?;
    }
    if precision.reduces_precision() {
        image.outcomes.detail(
            image_outcome,
            "xcf.precision",
            ConversionReason::NumericalConversion,
            format!("{}-bit {} samples", precision.bytes * 8, if precision.float { "floating-point" } else { "integer" }),
            format!("Native {depth:?} samples; round-to-nearest, ties-to-even conversion reduces source precision"),
        )?;
    }
    if historical_sample_order_ambiguous {
        image.outcomes.detail(
            image_outcome,
            "xcf.historical_sample_order",
            ConversionReason::NativePolicy,
            format!("Development XCF version {version} with {}-bit samples has no reliable sample byte-order marker", precision.bytes * 8),
            format!("Explicit {sample_order} sample byte order selected before decoding. Structural fields remain big-endian."),
        )?;
    }
    let mut document = Document::new(name, Size::new(w, h), mode, depth);
    metadata::image_with_work(
        &mut document,
        &image_props,
        version,
        &mut image.initial_view,
        &mut image.outcomes,
        image_outcome,
        &mut image.meta_budget,
        &mut image.warnings,
        &mut image.parasite_work,
        ctl,
    )?;
    for property in &image_props {
        if property.kind == 21 {
            let handled: &[&str] = if fallback_reason.is_some() { &[] } else { &["icc-profile"] };
            if image.outcomes.parasites_with_work(image_outcome, property.offset, property.data, handled, &mut image.report_parasite_work, ctl)? {
                image.warnings.note("Image parasite scan work exceeded its file allowance; optional source records were omitted; see the conversion report");
            }
        } else if property.kind != 17 && (property.kind != 1 || base_type != 2) {
            let reason = match property.kind {
                1 if palette_count(property.data, version).is_ok() => ConversionReason::NativePolicy,
                1 => ConversionReason::InvalidOptionalData,
                18 | 19 => ConversionReason::NativePolicy,
                // GIMP's nonzero item identifier and standard dialog units are
                // editing bookkeeping, not unapplied native image content.
                20 if property.data.len() == 4 && be_u32(property.data).is_some_and(|value| value != 0) => ConversionReason::NativePolicy,
                20 => ConversionReason::InvalidOptionalData,
                22 => metadata::unit_reason(property.data),
                24 | 27 | 39 => ConversionReason::NativeLimitation,
                _ => ConversionReason::MissingMapping,
            };
            image.outcomes.property_with_reason(image_outcome, property.kind, property.offset, property.data, reason)?;
        }
    }
    let (layers, linear_compositing) = image.import_layers(&layer_ptrs, fmt, mask_fmt, ctl)?;
    let (channels, selection) = image.import_channels(&channel_ptrs, (w, h), mask_fmt, ctl)?;

    document.layers = layers.into();
    document.channels = channels;
    document.selection = selection;
    document.icc_profile = tag_document.then(|| native_profile.clone());
    let mut color_outcome = color_encoding_outcome(assigned_profile, precision, fallback_reason, tag_document);
    if parasites_limited {
        color_outcome.source_description = "Image parasite scan was incomplete; the final source ICC assignment is unknown".into();
        color_outcome.reason = ConversionReason::RetentionLimit;
    }
    image.outcomes.push(color_outcome)?;
    import_paths(&mut document, &mut image, &image_props, &path_ptrs, omitted_paths, ctl)?;
    import_vectors(&mut document, &mut image, ctl)?;
    text::convert_colors(&mut document, &native_profile)?;
    import_native_effects(&mut document, &mut image, ctl)?;
    add_document_warnings(&mut image, &document, precision, linear_compositing);
    let initial_view = (image.initial_view != Default::default()).then_some(image.initial_view);
    if !image.outcomes.recording() {
        image.warnings.note("conversion details exceeded report limits; usable native content was imported, but the conversion report is incomplete");
    }
    document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish()?));
    ctl.progress(1.0);
    Ok(ImportResult { initial_view, document, warnings: image.warnings.finish(), source_read_only: true, preview_only: false })
}

/// Preserve the existing implicit-profile fallback when final ICC selection is unknown.
fn select_image_profile(mode: ColorMode, precision: Precision, icc: Option<Vec<u8>>, parasites_limited: bool) -> Result<color::ProfileSelection> {
    let mut selection = color::select_profile(mode, precision, icc)?;
    if parasites_limited {
        selection.fallback_reason = Some("Image parasite scan work was exhausted before the final ICC assignment could be established".into());
        selection.tag_document = true;
    }
    Ok(selection)
}

/// Describe the effective color interpretation without claiming unusable assigned colorants.
fn color_encoding_outcome(assigned_profile: bool, precision: Precision, fallback_reason: Option<String>, tag_document: bool) -> ConversionEntry {
    ConversionEntry {
        feature_code: "xcf.color_encoding".into(),
        source: conversion::source("image", 0, 0, "", &[], None),
        targets: vec![NativeTargetRef { label: "Document color profile".into(), ..Default::default() }],
        disposition: if fallback_reason.is_some() { ConversionDisposition::NativeApproximation } else { ConversionDisposition::NativeEquivalent },
        reason: if fallback_reason.is_some() { ConversionReason::NativeLimitation } else { ConversionReason::NativePolicy },
        source_description: format!(
            "{} profile; {} sample encoding",
            if assigned_profile { "Assigned" } else { "Implicit sRGB/sGray" },
            if precision.linear {
                "linear"
            } else if precision.perceptual {
                "sRGB transfer curve"
            } else {
                "profile transfer curve"
            }
        ),
        native_description: match fallback_reason {
            Some(reason) => format!(
                "The assigned ICC profile could not be used: {reason}. An encoding-correct implicit profile interprets the unchanged samples; assigned colorants could not be preserved. Optional source profile bytes remain inert when retention limits permit"
            ),
            None if !tag_document => "Unassigned nonlinear samples remain untagged for ordinary native color policy; implicit sRGB/sGray meaning is used only for internal literal conversion".into(),
            None => "Native ICC profile preserves the source colorants and sample transfer curve".into(),
        },
        attachments: Vec::new(),
    }
}

/// Report native compositing differences without changing imported content or its durable conversion report.
fn add_document_warnings(image: &mut Image<'_>, document: &Document, precision: Precision, linear_compositing: bool) {
    // The reference editor clips non-Normal layers to underlying coverage, showing nothing
    // where underlying layers are transparent; PhotoCraft shows them. A locked layer may
    // still have transparent pixels or cover only part of the canvas.
    if image.clipped_to_backdrop > 0 {
        image.warnings.note(&format!(
            "{} layer(s) use a mode clipped to underlying coverage in the reference editor; PhotoCraft shows the layer where underlying layers are transparent",
            image.clipped_to_backdrop
        ));
    }
    // The reference editor defaults to linear-light blending/compositing; PhotoCraft uses
    // the document's own encoding, so semi-transparent edges and blend modes differ.
    // Source layers set to a perceptual space match the native behavior.
    if !precision.linear && linear_compositing {
        image
            .warnings
            .note("The source image composites in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
    }
    if document.layers.is_empty() {
        image.warnings.note("the XCF file has no layers");
    }
}

/// Install owner-local native data, preserving each source operation as a historical event.
fn import_native_effects<'a>(document: &mut Document, image: &mut Image<'a>, ctl: &Interrupt<'_>) -> Result<()> {
    let mut ordinal = 0usize;
    for pending in std::mem::take(&mut image.pending_effects) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let original = document.layer(pending.layer).ok_or_else(|| invalid("native effect owner is missing"))?;
        // GIMP's serialized list is top-first; its last stored filter is the first
        // operation on the drawable. Independently saved stack evidence verifies this.
        for pointer in pending.pointers.iter().rev().copied() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut source = image.outcomes.entry(pending.owner)?.source.clone();
            source.kind = "layer_effect".into();
            source.ordinal = ordinal as u32;
            source.record_offset = Some(pointer as u64);
            ordinal = ordinal.checked_add(1).ok_or_else(|| invalid("effect ordinal overflow"))?;
            let before_effect = image.meta_budget;
            let effect = match effects::decode_with_work(&image.c, pointer, image.version, &mut image.meta_budget, &mut image.effect_decode_work, ctl) {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(IoError::XcfMetadataLimit) => {
                    image.meta_budget = before_effect;
                    image.metadata_limit(pending.owner, "xcf.layer_effect", "Layer-effect configuration exceeds the metadata or decode-work allowance")?;
                    break;
                }
                Err(error) => {
                    // Failed decoding retains no effect allocation; preserve only the caller's traversal charges.
                    image.meta_budget = before_effect;
                    image.outcomes.push(ConversionEntry {
                        feature_code: "xcf.layer_effect".into(),
                        source,
                        targets: vec![NativeTargetRef { layer_id: Some(pending.layer), ..Default::default() }],
                        disposition: ConversionDisposition::SkippedNonessential,
                        reason: ConversionReason::InvalidOptionalData,
                        source_description: format!("Stored effect framing could not be decoded: {error}"),
                        native_description: "Usable native owner content preserved; malformed optional effect was not applied".into(),
                        attachments: Vec::new(),
                    })?;
                    continue;
                }
            };
            source.name = effect.name.into();
            let mut notes: Vec<_> = effect.errors.iter().take(4).map(|error| error[..error.floor_char_boundary(128)].to_owned()).collect();
            let mut malformed = !effect.errors.is_empty();
            if !effect.icon.is_empty() {
                notes.push("Source icon identity was not applied".into());
            }
            for argument in &effect.arguments {
                malformed |= matches!(argument.value, effects::ArgumentValue::Invalid { .. });
                if notes.len() >= 8 {
                    continue;
                }
                let name = &argument.name[..argument.name.floor_char_boundary(128)];
                match &argument.value {
                    effects::ArgumentValue::String(value) => notes.push(format!(
                        "Stored string argument {} ({} bytes) was not applied. Source strings grant no external resource access",
                        name,
                        value.len()
                    )),
                    effects::ArgumentValue::Config(config) => {
                        notes.push(format!("Stored configuration {name} ({} bytes) was decoded only as bounded data", config.raw.len()))
                    }
                    effects::ArgumentValue::Unknown { kind, bytes } => {
                        notes.push(format!("Unknown argument {name} type {kind} ({} bytes) was not applied", bytes.len()))
                    }
                    effects::ArgumentValue::Invalid { kind, bytes, message } => {
                        let message = &message[..message.floor_char_boundary(128)];
                        notes.push(format!("Invalid argument {name} type {kind} ({} bytes): {message}", bytes.len()))
                    }
                    _ => {}
                }
            }
            if effect.blend_space != 0 || effect.composite_space != 0 || effect.composite_mode != 0 || effect.clip || effect.region != 0 {
                notes.push(format!("Native document blending/coverage applies; stored blend space {}, composite space {}, composite mode {}, clip {}, region {} are historical", effect.blend_space, effect.composite_space, effect.composite_mode, effect.clip, effect.region));
            }
            let reason = if malformed { ConversionReason::InvalidOptionalData } else { ConversionReason::MissingMapping };
            let explanation = if malformed { "Effect contains invalid optional fields or arguments." } else { "This effect has no native mapping." };
            let native = format!("{explanation} Usable native owner content preserved. The effect was left unapplied. {}", notes.join("; "));
            let outcome = image.outcomes.push(ConversionEntry {
                feature_code: "xcf.layer_effect".into(),
                source,
                targets: vec![NativeTargetRef { layer_id: Some(pending.layer), label: "Native owner".into(), ..Default::default() }],
                disposition: ConversionDisposition::SkippedNonessential,
                reason,
                source_description: format!(
                    "{} ({}) version {}; visible {}, opacity {}, owner bounds {:?}",
                    effect.name,
                    effect.operation,
                    effect.version.unwrap_or("absent"),
                    effect.visible,
                    effect.opacity,
                    pending.bounds
                ),
                native_description: native,
                attachments: Vec::new(),
            })?;
            image.outcomes.primary_property(outcome, 45, effect.record_offset, effect.raw, reason)?;
            if effect.mask_offset != 0 {
                // This slice retains the envelope, not the referenced mask channel/tiles.
                // A later mask adapter must replace this loss disclosure when it retains
                // or installs that data; the envelope pointer alone is not preservation.
                let mut mask_source = image.outcomes.entry(outcome)?.source.clone();
                mask_source.kind = "effect-mask".into();
                mask_source.record_offset = Some(effect.mask_offset as u64);
                image.outcomes.push(ConversionEntry {
                    feature_code: "xcf.layer_effect.mask".into(),
                    source: mask_source,
                    targets: vec![NativeTargetRef { layer_id: Some(pending.layer), label: "Native raster owner".into(), ..Default::default() }],
                    disposition: ConversionDisposition::SkippedNonessential,
                    reason: ConversionReason::NativeLimitation,
                    source_description: "Stored effect references a separate mask channel and tile data".into(),
                    native_description: "Referenced effect mask data is not retained. The effect was left unapplied".into(),
                    attachments: Vec::new(),
                })?;
                image.warnings.layer(&original.name, "has a deferred effect mask whose referenced data is not retained; see the conversion report");
            }
            for property in effect.unknown_properties {
                image.outcomes.property(outcome, property.kind, property.offset, property.bytes)?;
            }
        }
        image.warnings.layer(&original.name, "contains source effects that were left unapplied. See the conversion report");
    }
    Ok(())
}

/// Translate saved cubic geometry into ordinary native Paths panel entries.
fn import_paths<'a>(
    document: &mut Document,
    image: &mut Image<'a>,
    properties: &[Prop<'a>],
    pointers: &[usize],
    omitted_standalone: Option<(usize, usize)>,
    ctl: &Interrupt<'_>,
) -> Result<()> {
    import_paths_with_work(document, image, properties, pointers, omitted_standalone, &mut paths::DecodeWork::default(), ctl)
}

/// Preserve saved pixels when optional path source work cannot be admitted. Successful
/// and rejected records share this nonrefundable file allowance, independently of storage.
fn import_paths_with_work<'a>(
    document: &mut Document,
    image: &mut Image<'a>,
    properties: &[Prop<'a>],
    pointers: &[usize],
    omitted_standalone: Option<(usize, usize)>,
    work: &mut paths::DecodeWork,
    ctl: &Interrupt<'_>,
) -> Result<()> {
    let mut records: Vec<SavedPathRecord<'a>> = Vec::new();
    let mut ordinal = 0usize;
    let mut limit = None;
    let mut incomplete_identity = false;
    'admission: {
        for property in properties.iter().filter(|p| matches!(p.kind, 23 | 25)) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            // Source slices remain borrowed and have a separate inert-report allowance.
            // Source bytes debit nonrefundable decoding work; retained native storage
            // has its own allowance and never refunds failed parsing work.
            if records.len() >= MAX_CHANNELS || property.data.len() > image.meta_budget {
                limit = Some((ordinal, property.offset, "saved-path count or remaining document metadata allowance"));
                break 'admission;
            }
            let (paths, source_count) = match if property.kind == 23 {
                paths::legacy_with_work(property.data, work, ctl)
            } else {
                paths::vectors_with_work(property.data, work, ctl).map(|paths| {
                    paths.map(|paths| {
                        let count = paths.len();
                        (paths, count)
                    })
                })
            } {
                Ok(Some(batch)) => batch,
                Ok(None) => {
                    limit = Some((ordinal, property.offset, "saved-path decoding work allowance"));
                    break 'admission;
                }
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
                    incomplete_identity = true;
                    unavailable_path(image, ordinal, property.offset, "", &error, None)?;
                    ordinal += 1;
                    continue;
                }
            };
            for path in paths {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                let source_ordinal = ordinal + path.source_index;
                if records.len() >= MAX_CHANNELS || admit_path(&mut image.meta_budget, &path).is_err() {
                    limit = Some((source_ordinal, property.offset, "saved-path count or remaining document metadata allowance"));
                    break 'admission;
                }
                records.push((source_ordinal, property.offset, path, None));
            }
            ordinal += source_count;
        }
        for &offset in pointers {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if records.len() >= MAX_CHANNELS || image.meta_budget == 0 {
                limit = Some((ordinal, offset, "saved-path count or remaining document metadata allowance"));
                break 'admission;
            }
            // Outer framing is optional too: a length extending past EOF cannot
            // invalidate independently framed drawable pixels, nor justify a raw attachment.
            let frame = (|| -> Result<_> {
                let mut cursor = image.c.at(offset);
                let name = cursor.string_bytes()?;
                let length = cursor.u32()? as usize;
                cursor.take(length)?;
                let raw = image.c.b.get(offset..cursor.pos).ok_or_else(|| invalid("saved path range"))?;
                Ok((name, length, raw))
            })();
            let (name, length, raw) = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    incomplete_identity = true;
                    unavailable_path(image, ordinal, offset, "", &error, None)?;
                    ordinal += 1;
                    continue;
                }
            };
            if length > image.meta_budget
                || std::mem::size_of::<photocraft_doc::vector::NamedPath>().checked_add(name.len()).is_none_or(|bytes| bytes > image.meta_budget)
            {
                limit = Some((ordinal, offset, "remaining document metadata allowance"));
                break 'admission;
            }
            let name = std::str::from_utf8(name).unwrap_or("Invalid UTF-8 path name");
            match paths::standalone_with_work(&image.c, offset, work, ctl) {
                Ok(Some(path)) => {
                    if admit_path(&mut image.meta_budget, &path).is_err() {
                        limit = Some((ordinal, offset, "remaining document metadata allowance"));
                        break 'admission;
                    }
                    records.push((ordinal, offset, path, Some(raw)));
                }
                Ok(None) => {
                    limit = Some((ordinal, offset, "saved-path decoding work allowance"));
                    break 'admission;
                }
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
                    incomplete_identity = true;
                    unavailable_path(image, ordinal, offset, name, &error, Some(raw))?;
                }
            }
            ordinal += 1;
        }
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let incomplete_identity = incomplete_identity || limit.is_some() || omitted_standalone.is_some();
    let disclosure = match (limit, omitted_standalone) {
        (Some((ordinal, offset, reason)), Some((_, count))) => {
            Some((ordinal, offset, format!("{reason}; standalone pointer retention limit; {count} later records omitted")))
        }
        (Some((ordinal, offset, reason)), None) => Some((ordinal, offset, reason.to_string())),
        (None, Some((offset, count))) => Some((ordinal, offset, format!("standalone pointer retention limit; {count} later records omitted"))),
        (None, None) => None,
    };
    if let Some((ordinal, offset, reason)) = disclosure {
        limited_paths(image, ordinal, offset, &reason)?;
    }
    if incomplete_identity {
        // Undecoded records could select another path or duplicate an admitted tattoo.
        // Keep their saved pixels rather than guessing a vector binding or initial target.
        image.initial_view.selected_path = None;
        image.path_tattoos.values_mut().for_each(|index| *index = None);
    }
    let mut disclosed_multiple_selections = false;
    for (ordinal, offset, path, raw) in records {
        let index = document.paths.len();
        let name = path.named.name.clone();
        let multiple_selections = path.selected && !incomplete_identity && image.initial_view.selected_path.replace(index).is_some();
        if let Some(tattoo) = path.tattoo {
            image
                .path_tattoos
                .entry(tattoo)
                .and_modify(|value| *value = None)
                .or_insert((!incomplete_identity).then_some(PathBinding { index, geometry_complete: path.geometry_complete }));
        }
        let outcome = image.outcomes.push(ConversionEntry {
            feature_code: "xcf.path".into(),
            source: conversion::source("path", ordinal, offset, &name, &[], path.tattoo),
            targets: vec![NativeTargetRef { path_index: Some(index as u32), label: name, ..Default::default() }],
            disposition: if path.source_only { ConversionDisposition::NativeApproximation } else { ConversionDisposition::NativeEquivalent },
            reason: if path.geometry_complete { ConversionReason::NativePolicy } else { ConversionReason::MissingMapping },
            source_description: if path.geometry_complete {
                "Saved cubic anchors, handles and open/closed subpaths"
            } else {
                "Saved path includes unsupported strokes; only part of its geometry could be translated"
            }
            .into(),
            native_description: if !path.geometry_complete {
                "Native path contains only supported cubic geometry; this incomplete path cannot replace a vector layer's saved pixels"
            } else if path.source_only {
                "Native editable cubic path; source auxiliary axes and non-geometric flags are not editing rules"
            } else {
                "Native editable cubic path with document-coordinate geometry"
            }
            .into(),
            attachments: Vec::new(),
        })?;
        if path.invalid_selection_marker {
            image.outcomes.notice(
                outcome,
                "xcf.path_selection",
                ConversionReason::InvalidOptionalData,
                ("Saved path selection marker has a nonempty payload".into(), "Path geometry preserved; malformed marker ignored".into()),
                Some("has an invalid saved path selection marker"),
                &mut image.warnings,
            )?;
        }
        if multiple_selections && !disclosed_multiple_selections {
            disclosed_multiple_selections = true;
            image.outcomes.notice(
                outcome,
                "xcf.path_selection",
                ConversionReason::NativeLimitation,
                ("Multiple source paths were selected".into(), "PhotoCraft restores the last valid selection and keeps all usable paths".into()),
                Some("is among multiple selected paths; PhotoCraft restores only the last valid selection"),
                &mut image.warnings,
            )?;
        }
        if let Some(raw) = raw {
            image.outcomes.property(outcome, 25, offset, raw)?;
        }
        document.paths.push(path.named);
    }
    // Raw source path properties are already covered by the image's bounded inert retention.
    Ok(())
}

type SavedPathRecord<'a> = (usize, usize, paths::DecodedPath, Option<&'a [u8]>);

/// Account for owned strings, native geometry capacities and persistent path storage
/// before pushing it. Wire payload lengths omit names and undercount native knot storage.
fn admit_path(remaining: &mut usize, path: &paths::DecodedPath) -> Result<()> {
    let overflow = || invalid("saved path retained geometry size overflow");
    let mut bytes = std::mem::size_of::<photocraft_doc::vector::NamedPath>()
        .checked_add(path.named.name.capacity())
        .and_then(|bytes| bytes.checked_add(path.named.psd_raw.as_ref().map_or(0, |raw| raw.capacity())))
        .ok_or_else(overflow)?;
    if path.tattoo.is_some() {
        bytes = bytes.checked_add(4 * std::mem::size_of::<usize>()).ok_or_else(overflow)?;
    }
    bytes = bytes
        .checked_add(path.named.path.subpaths.capacity().checked_mul(std::mem::size_of::<photocraft_doc::vector::Subpath>()).ok_or_else(overflow)?)
        .ok_or_else(overflow)?;
    for subpath in &path.named.path.subpaths {
        bytes = bytes
            .checked_add(subpath.knots.capacity().checked_mul(std::mem::size_of::<photocraft_doc::vector::Knot>()).ok_or_else(overflow)?)
            .ok_or_else(overflow)?;
    }
    *remaining = remaining.checked_sub(bytes).ok_or_else(|| invalid("saved paths exceed the 32 MiB geometry and metadata allowance"))?;
    Ok(())
}

/// A path admission ceiling omits optional geometry, never already decoded pixels.
fn limited_paths(image: &mut Image<'_>, ordinal: usize, offset: usize, reason: &str) -> Result<()> {
    image.outcomes.push(ConversionEntry {
        feature_code: "xcf.path".into(),
        source: conversion::source("path", ordinal, offset, "", &[], None),
        targets: Vec::new(),
        disposition: ConversionDisposition::SkippedNonessential,
        reason: ConversionReason::RetentionLimit,
        source_description: format!("Optional saved paths exceeded the {reason}"),
        native_description: "Earlier native paths and usable saved pixels were preserved. Later saved-path geometry was not decoded or retained; separately retained source properties remain inert. Vector bindings and initial path selection were not guessed.".into(),
        attachments: Vec::new(),
    })?;
    image.warnings.note("Optional saved paths exceeded an admission limit; earlier paths and usable pixels were preserved. See the conversion report");
    Ok(())
}

/// Optional path syntax cannot make separately framed source pixels unavailable.
/// Standalone record bytes need their own inert attachment; image properties
/// already have one in the normal image-property retention pass.
fn unavailable_path<'a>(image: &mut Image<'a>, ordinal: usize, offset: usize, name: &str, error: &IoError, raw: Option<&'a [u8]>) -> Result<()> {
    let outcome = image.outcomes.push(ConversionEntry {
        feature_code: "xcf.path".into(),
        source: conversion::source("path", ordinal, offset, name, &[], None),
        targets: Vec::new(),
        disposition: ConversionDisposition::RetainedUnapplied,
        reason: ConversionReason::InvalidOptionalData,
        source_description: format!("Optional saved path could not be translated: {error}"),
        native_description: "Usable native image content preserved; bounded saved-path configuration remains inert when the retention allowance permits".into(),
        attachments: Vec::new(),
    })?;
    if let Some(raw) = raw {
        image.outcomes.property_with_reason(outcome, 25, offset, raw, ConversionReason::InvalidOptionalData)?;
    }
    image.warnings.note("Optional saved paths could not be translated; usable pixels were preserved. See the conversion report");
    Ok(())
}

/// Converts saved source paths to bounded editable native paths while retaining malformed optional records
/// honestly.
fn import_vectors(document: &mut Document, image: &mut Image<'_>, ctl: &Interrupt<'_>) -> Result<()> {
    for (id, outcome, mut vector) in std::mem::take(&mut image.pending_vectors) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if vector.modified {
            image.outcomes.notice(
                outcome,
                "xcf.vector_layer",
                ConversionReason::NativePolicy,
                (
                    "Source vector pixels were modified independently of path/style".into(),
                    "Native raster preserves modified pixels; saved path and paint configuration remain inert".into(),
                ),
                Some("has modified vector pixels and opens as a native raster"),
                &mut image.warnings,
            )?;
            continue;
        }
        if let Some(error) = &image.native_color_error {
            image.outcomes.notice(
                outcome,
                "xcf.vector_layer",
                ConversionReason::NativeLimitation,
                (
                    format!("Native vector colors cannot be converted: {error}"),
                    "Stored pixels preserved as a native raster; source path and paint configuration remain inert".into(),
                ),
                Some("has unavailable native color conversion and opens as pixels"),
                &mut image.warnings,
            )?;
            continue;
        }
        let binding = image.path_tattoos.get(&vector.path_tattoo).copied().flatten();
        let Some((binding, path)) = binding.and_then(|binding| document.paths.get(binding.index).map(|path| (binding, path))) else {
            image.outcomes.notice(
                outcome,
                "xcf.vector_layer",
                ConversionReason::InvalidOptionalData,
                (
                    format!("Source vector path tattoo {} is missing or ambiguous", vector.path_tattoo),
                    "Stored pixels preserved as a native raster; no path identity guessed".into(),
                ),
                Some("references a missing or ambiguous path and opens as pixels"),
                &mut image.warnings,
            )?;
            continue;
        };
        if !binding.geometry_complete {
            image.outcomes.notice(
                outcome,
                "xcf.vector_layer",
                ConversionReason::MissingMapping,
                (
                    format!("Source path tattoo {} includes unsupported stroke geometry", vector.path_tattoo),
                    "Stored pixels preserved as a native raster; incomplete editable path remains available without replacing the layer".into(),
                ),
                Some("has incomplete vector geometry and opens as pixels"),
                &mut image.warnings,
            )?;
            continue;
        }
        let knots = path.path.subpaths.iter().try_fold(0usize, |n, s| n.checked_add(s.knots.len())).ok_or_else(|| invalid("vector geometry count overflow"))?;
        let bytes = knots
            .checked_mul(std::mem::size_of::<photocraft_doc::vector::Knot>())
            .and_then(|n| n.checked_add(path.path.subpaths.len().saturating_mul(std::mem::size_of::<photocraft_doc::vector::Subpath>())))
            .ok_or_else(|| invalid("vector geometry size overflow"))?;
        let Some(remaining) = image.meta_budget.checked_sub(bytes) else {
            image.outcomes.notice(
                outcome,
                "xcf.vector_layer",
                ConversionReason::RetentionLimit,
                (
                    "Editable vector geometry exceeds the remaining document metadata allowance".into(),
                    "Stored pixels preserved as a native raster; saved path and paint configuration remain inert".into(),
                ),
                Some("exceeds the editable geometry allowance and opens as pixels"),
                &mut image.warnings,
            )?;
            continue;
        };
        image.meta_budget = remaining;
        vector.shape.path = path.path.clone();
        let layer = document.layer_mut(id).ok_or_else(|| invalid("missing native vector target"))?;
        // Keep the already-budgeted appearance until the editable shape is changed.
        let previous = std::mem::replace(&mut layer.content, LayerContent::Shape(vector.shape));
        if let LayerContent::Shape(shape) = &mut layer.content {
            shape.cache = match previous {
                LayerContent::Raster(pixels) => Some(pixels),
                LayerContent::Text(text) => text.cache,
                _ => None,
            };
        }
        image.outcomes.notice(
            outcome,
            "xcf.vector_layer",
            ConversionReason::NativePolicy,
            ("Saved vector geometry and paint".into(), "Native editable shape with saved appearance. PhotoCraft rasterizes subsequent edits".into()),
            Some("opens as an editable native shape"),
            &mut image.warnings,
        )?;
        for detail in vector.source_only {
            image.outcomes.detail(
                outcome,
                "xcf.vector_layer.paint",
                ConversionReason::MissingMapping,
                detail,
                "Supported native shape paint is editable; additional source paint configuration remains inert".into(),
            )?;
        }
    }
    Ok(())
}

/// Keep recovered nodes outside the source index space until placement ends.
fn place(roots: &mut Vec<Node>, recovered: &mut Vec<Node>, node: Node, warnings: &mut Warnings, outcomes: &mut conversion::Outcomes<'_>) -> Result<()> {
    let depth = node.path.len().saturating_sub(1);
    let destination = node
        .path
        .iter()
        .take(depth)
        .try_fold(&mut *roots, |level, &index| level.get_mut(index as usize).filter(|node| node.group).map(|group| &mut group.children));
    if let Some(level) = destination {
        level.push(node);
    } else {
        warnings.layer(&node.layer.name, "has an item path that leads to no group; opened at the top level");
        outcomes.detail(
            node.outcome,
            "xcf.item_path",
            ConversionReason::InvalidOptionalData,
            "Source item path leads to no group".into(),
            "Layer opened at the native top level".into(),
        )?;
        recovered.push(node);
    }
    Ok(())
}

/// Nodes top-first into layers bottom-first, groups recursively.
fn finish(nodes: Vec<Node>) -> Vec<Layer> {
    let mut out = Vec::with_capacity(nodes.len());
    for mut node in nodes.into_iter().rev() {
        if node.group {
            let children = finish(std::mem::take(&mut node.children));
            if let Some(c) = node.layer.children_mut() {
                *c = children;
            }
        }
        out.push(node.layer);
    }
    out
}
