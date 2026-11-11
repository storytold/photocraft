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
mod dpi_review_tests;
mod layers;
mod samples;
mod wire;
// Parsed fields stay inert until native mapping slices land.
#[allow(dead_code)]
mod effects;
#[cfg(test)]
mod floating_gray_review_tests;
#[cfg(test)]
mod historical_precision_tests;
mod metadata;
mod metadata_packets;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod optional_mask_budget_tests;
#[cfg(test)]
mod path_admission_tests;
mod paths;
mod resources;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
mod text;
#[cfg(test)]
mod tile_budget_tests;
pub(crate) mod transport;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use photocraft_codecs::f16;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, NativeTargetRef};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

use samples::{Precision, integer_samples};
#[cfg(test)]
use wire::rle_decode;
use wire::{Compression, Cursor, Prop, be_f32, be_i32, be_u32, decode_tile, parasites, props, props_version};

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
    warnings: Warnings,
    outcomes: conversion::Outcomes<'a>,
    /// Layers GIMP composites with "Clip to backdrop" (its default for every mode but Normal).
    clipped_to_backdrop: usize,
    initial_view: crate::initial_view::NativeInitialView,
    resolution_dpi: f32,
    pending_effects: Vec<PendingLayerEffects>,
    path_tattoos: HashMap<u32, Option<usize>>,
    pending_vectors: Vec<(photocraft_doc::LayerId, usize, paths::VectorStyle)>,
    /// Unavailable literal-color conversion keeps source text/vector rasters intact.
    native_color_error: Option<String>,
}

/// Bounded source mask facts retained for reporting after native coverage conversion.
struct CoverageMaskRecord<'a> {
    name: String,
    properties: Vec<Prop<'a>>,
    width: u32,
    height: u32,
    children: Vec<usize>,
}
#[derive(Default)]
struct CoverageTranslation<'a> {
    applied: HashMap<usize, std::result::Result<String, effects::MappingFailure>>,
    masks: HashMap<usize, CoverageMaskRecord<'a>>,
    active_effects: std::collections::HashSet<usize>,
    active_masks: std::collections::HashSet<usize>,
    visited: usize,
}

struct PendingLayerEffects {
    layer: photocraft_doc::LayerId,
    owner: usize,
    bounds: Rect,
    pointers: Vec<usize>,
}

/// Pixel data of a drawable: `channels` normalised samples per pixel, row-major.
struct Pixels {
    width: u32,
    height: u32,
    channels: usize,
    data: Vec<f32>,
    underflow: bool,
}

impl<'a> Image<'a> {
    /// Reads a hierarchy (`hptr`): the first level's tiles into `channels` floats per pixel,
    /// `source_channels` as stored (indexed images expand to RGB here).
    fn hierarchy(&mut self, hptr: usize, source_channels: usize, coverage: bool, ctl: &Interrupt<'_>) -> Result<Pixels> {
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
        let channels = if indexed { source_channels + 2 } else { source_channels };
        let count = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| invalid("drawable overflow"))?;
        let bytes = count.checked_mul(4).ok_or_else(|| invalid("drawable overflow"))?;
        self.budget = self.budget.checked_sub(bytes).ok_or_else(|| invalid("the layers exceed the 1 GiB pixel limit"))?;
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
        let mut data = vec![0.0f32; count];
        let mut underflow = false;
        let stride = w as usize * channels;
        for (i, &at) in tiles.iter().enumerate() {
            if i % 64 == 0 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let (tx, ty) = ((i % across) as u32, (i / across) as u32);
            let (tw, th) = ((w - tx * TILE).min(TILE), (h - ty * TILE).min(TILE));
            let end = tiles.get(i + 1).copied().filter(|&n| n > at).unwrap_or(self.c.b.len());
            let mut raw = decode_tile(&self.c, at, end, self.compression, bpp, (tw * th) as usize)?;
            if self.sample_order == crate::HistoricalXcfSampleOrder::LittleEndian && self.precision.bytes > 1 {
                if !raw.len().is_multiple_of(self.precision.bytes) {
                    return Err(invalid("historical tile does not contain complete samples"));
                }
                for (sample, bytes) in raw.chunks_exact_mut(self.precision.bytes).enumerate() {
                    if sample.is_multiple_of(2048) {
                        ctl.check().map_err(|_| IoError::Cancelled)?;
                    }
                    bytes.reverse();
                }
            }
            if !indexed && !self.precision.float && matches!(self.precision.bytes, 1 | 2) {
                let row_samples = tw as usize * channels;
                let row_bytes = tw as usize * bpp;
                for py in 0..th as usize {
                    let src = py * row_bytes;
                    let dst = (ty as usize * TILE as usize + py) * stride + tx as usize * TILE as usize * channels;
                    let input = raw.get(src..src + row_bytes).ok_or_else(|| invalid("integer tile row is cut short"))?;
                    let output = data.get_mut(dst..dst + row_samples).ok_or_else(|| invalid("integer tile row exceeds its drawable"))?;
                    integer_samples(input, output, self.precision.bytes)?;
                }
                continue;
            }
            for py in 0..th as usize {
                for px in 0..tw as usize {
                    let src = (py * tw as usize + px) * bpp;
                    let dst = (ty as usize * TILE as usize + py) * stride + (tx as usize * TILE as usize + px) * channels;
                    let pixel = &raw[src..src + bpp];
                    if indexed {
                        let idx = usize::from(pixel[0]);
                        let [r, g, b] = self.colormap.get(idx).copied().ok_or_else(|| invalid(format!("palette index {idx} has no color entry")))?;
                        data[dst] = f32::from(r) / 255.0;
                        data[dst + 1] = f32::from(g) / 255.0;
                        data[dst + 2] = f32::from(b) / 255.0;
                        if source_channels == 2 {
                            data[dst + 3] = f32::from(pixel[1]) / 255.0;
                        }
                    } else {
                        for ch in 0..channels {
                            let is_coverage = coverage || (matches!(source_channels, 2 | 4) && ch + 1 == source_channels);
                            let (value, rounded_zero) =
                                self.precision.sample(&pixel[ch * self.precision.bytes..(ch + 1) * self.precision.bytes], is_coverage).map_err(|error| {
                                    invalid(format!(
                                        "{} plane {ch}, pixel ({}, {}): {error}",
                                        if is_coverage { "coverage" } else { "color" },
                                        tx as usize * TILE as usize + px,
                                        ty as usize * TILE as usize + py,
                                    ))
                                })?;
                            data[dst + ch] = value;
                            underflow |= rounded_zero;
                        }
                    }
                }
            }
        }
        Ok(Pixels { width: w, height: h, channels, data, underflow })
    }

    /// A channel structure (a layer mask, the selection or an extra channel) as one-channel
    /// pixels, with its properties.
    fn channel(&mut self, ptr: usize, ctl: &Interrupt<'_>) -> Result<(String, Vec<Prop<'a>>, Pixels, Vec<usize>)> {
        let mut c = self.c.at(ptr);
        let (width, height) = (c.u32()?, c.u32()?);
        let name = c.string()?;
        self.meta_budget = self.meta_budget.checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?;
        let props = props(&mut c)?;
        let hptr = c.ptr()?;
        let effect_pointers = if self.version >= 26 { effects::read_pointers(&mut c, &mut self.meta_budget, ctl)? } else { Vec::new() };
        let saved = (self.base_type, self.precision);
        // Channels are always one plain sample per pixel, even in indexed images.
        self.base_type = 0;
        let px = self.hierarchy(hptr, 1, true, ctl).map_err(|error| match error {
            IoError::Cancelled => IoError::Cancelled,
            error => invalid(format!("channel or mask '{name}': {error}")),
        });
        (self.base_type, self.precision) = saved;
        let px = px?;
        if (px.width, px.height) != (width, height) {
            return Err(invalid(format!("channel or mask '{name}' hierarchy dimensions differ from the declared {width}x{height}")));
        }
        Ok((name, props, px, effect_pointers))
    }

    /// Source operations are mapped to existing native Gray operations at import.
    /// Auxiliary mask stacks are converted first; source references never become
    /// native runtime instructions and recursive source cycles are not followed.
    fn translate_coverage_effects(&mut self, owner: usize, pointers: &[usize], surface: &mut Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<()> {
        let offset = self.outcomes.entry(owner)?.source.record_offset.and_then(|v| usize::try_from(v).ok());
        let state = self.convert_coverage_effects(pointers, surface, bounds, offset, ctl)?;
        self.report_converted_coverage(owner, pointers, &state, true, ctl)
    }

    /// Convert into a temporary native surface without claiming it was installed.
    /// Layer-effect owners publish this bounded state only after their stage commits.
    fn convert_coverage_effects(
        &mut self,
        pointers: &[usize],
        surface: &mut Surface,
        bounds: Rect,
        source_offset: Option<usize>,
        ctl: &Interrupt<'_>,
    ) -> Result<CoverageTranslation<'a>> {
        let mut state = CoverageTranslation::default();
        if let Some(offset) = source_offset {
            state.active_masks.insert(offset);
        }
        self.apply_coverage_stack(pointers, surface, bounds, 0, &mut state, ctl)?;
        Ok(state)
    }

    /// Publishes coverage conversion outcomes only after their native result is committed or explicitly
    /// discarded.
    fn report_converted_coverage(
        &mut self,
        owner: usize,
        pointers: &[usize],
        state: &CoverageTranslation<'a>,
        committed: bool,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        self.report_coverage_effects_with(owner, pointers, &state.applied, &state.masks, committed, ctl)
    }

    /// Applies stored coverage stages in order, preserving usable pixels and reporting optional stages that
    /// cannot be mapped.
    fn apply_coverage_stack(
        &mut self,
        pointers: &[usize],
        surface: &mut Surface,
        _bounds: Rect,
        depth: usize,
        state: &mut CoverageTranslation<'a>,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(invalid("native coverage source nesting exceeds its limit"));
        }
        let mut omitted = false;
        for &pointer in pointers.iter().rev() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            state.visited = state.visited.checked_add(1).ok_or_else(|| invalid("coverage traversal count overflows"))?;
            if state.visited > MAX_LAYERS {
                return Err(invalid("coverage traversal exceeds 8,192 source effects"));
            }
            if !state.active_effects.insert(pointer) {
                state.applied.insert(
                    pointer,
                    Err(effects::MappingFailure {
                        reason: ConversionReason::InvalidOptionalData,
                        message: "Cyclic source coverage operation was not followed".into(),
                    }),
                );
                omitted = true;
                continue;
            }
            let effect = match effects::decode(&self.c, pointer, self.version, &mut self.meta_budget, ctl) {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(_) => {
                    state.active_effects.remove(&pointer);
                    omitted = true;
                    continue;
                }
            };
            if !effect.visible || effect.opacity == 0. {
                state.active_effects.remove(&pointer);
                continue;
            }
            let auxiliary = if effect.mask_offset == 0 {
                Ok(Ok(None))
            } else {
                self.coverage_auxiliary(effect.mask_offset, depth + 1, state, ctl).map(|value| value.map(Some))
            };
            let result: std::result::Result<(Surface, String), effects::MappingFailure> = match auxiliary {
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => Err(effects::MappingFailure {
                    reason: ConversionReason::InvalidOptionalData,
                    message: format!("Source auxiliary coverage could not be converted; operation remains inactive: {error}"),
                }),
                Ok(Err(failure)) => Err(failure),
                Ok(Ok(_mask)) => {
                    Err(effects::MappingFailure::from("Native coverage conversion is not implemented; original coverage and source records are preserved"))
                }
            };
            match result {
                Ok((converted, mut notes)) => {
                    if omitted {
                        notes.push_str("; An earlier source coverage stage was omitted; this stage receives preserved native coverage instead of the complete source pipeline");
                    }
                    *surface = converted;
                    state.applied.insert(pointer, Ok(notes));
                }
                Err(message) => {
                    omitted = true;
                    state.applied.insert(pointer, Err(message));
                }
            }
            state.active_effects.remove(&pointer);
        }
        Ok(())
    }

    /// Resolves bounded auxiliary coverage references while detecting cycles and retaining unavailable
    /// optional masks.
    fn coverage_auxiliary(
        &mut self,
        offset: usize,
        depth: usize,
        state: &mut CoverageTranslation<'a>,
        ctl: &Interrupt<'_>,
    ) -> Result<std::result::Result<LayerMask, effects::MappingFailure>> {
        if depth > MAX_DEPTH || !state.active_masks.insert(offset) {
            return Err(invalid("cyclic or excessively nested source auxiliary coverage"));
        }
        let result = (|| -> Result<std::result::Result<LayerMask, effects::MappingFailure>> {
            let pixel_budget = self.budget;
            let (name, properties, pixels, children) = match self.channel(offset, ctl) {
                Ok(channel) => channel,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
                    // A failed optional decode retains no pixels. Its speculative
                    // sample charge must not starve later independently valid layers.
                    self.budget = pixel_budget;
                    return Err(error);
                }
            };
            // Independently saved source effect masks use image-global pixels.
            // A nonstandard layer-style placement has no established source meaning.
            if properties.iter().any(|property| property.kind == 15 && property.data.iter().any(|byte| *byte != 0)) {
                return Ok(Err("Source effect-mask channel has an unestablished placement property; pixels were not silently repositioned".into()));
            }
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, self.precision.depth(), false));
            write_pixels(&mut surface, &pixels, 0, 0, &mut self.budget)?;
            let bounds = Rect::new(0, 0, pixels.width as i32, pixels.height as i32);
            self.apply_coverage_stack(&children, &mut surface, bounds, depth, state, ctl)?;
            self.meta_budget = self
                .meta_budget
                .checked_sub(std::mem::size_of::<CoverageMaskRecord<'a>>() + 4 * std::mem::size_of::<usize>())
                .ok_or_else(|| invalid("native auxiliary coverage metadata exceeds its allowance"))?;
            state.masks.insert(offset, CoverageMaskRecord { name, properties, width: pixels.width, height: pixels.height, children });
            Ok(Ok(LayerMask { surface, ..LayerMask::reveal_all() }))
        })();
        state.active_masks.remove(&offset);
        result
    }

    /// Decodes and inventories nested coverage effects with owner identity, limits and deferred application
    /// outcomes.
    fn report_coverage_effects_with(
        &mut self,
        owner: usize,
        pointers: &[usize],
        applied: &HashMap<usize, std::result::Result<String, effects::MappingFailure>>,
        decoded_masks: &HashMap<usize, CoverageMaskRecord<'a>>,
        committed: bool,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        if pointers.is_empty() {
            return Ok(());
        }
        let owner_entry = self.outcomes.entry(owner)?.clone();
        self.meta_budget = self
            .meta_budget
            .checked_sub(pointers.len().saturating_mul(6 * std::mem::size_of::<usize>()))
            .ok_or_else(|| invalid("coverage effect traversal exceeds metadata allowance"))?;
        let mut work: Vec<(usize, usize, usize)> = pointers.iter().rev().map(|pointer| (*pointer, 0, owner)).collect();
        let mut effects_seen = std::collections::HashSet::new();
        let mut masks_seen = std::collections::HashSet::new();
        if let Some(offset) = owner_entry.source.record_offset.and_then(|offset| usize::try_from(offset).ok()) {
            masks_seen.insert(offset);
        }
        let mut ordinal = 0u32;
        while let Some((pointer, depth, source_owner)) = work.pop() {
            let current_owner = self.outcomes.entry(source_owner)?.clone();
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if effects_seen.len() >= MAX_LAYERS {
                return Err(invalid("coverage effects exceed 8,192 records"));
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
            self.meta_budget = self
                .meta_budget
                .checked_sub(8 * std::mem::size_of::<usize>())
                .ok_or_else(|| invalid("coverage effect traversal exceeds metadata allowance"))?;
            let effect = match effects::decode(&self.c, pointer, self.version, &mut self.meta_budget, ctl) {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
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
            let owner_committed = committed && (depth == 0 || current_owner.disposition == ConversionDisposition::NativeApproximation);
            let translated = owner_committed && applied.get(&pointer).is_some_and(|result| result.is_ok());
            let reason = if applied.get(&pointer).is_some_and(|value| value.is_ok()) {
                ConversionReason::NativePolicy
            } else {
                applied.get(&pointer).and_then(|result| result.as_ref().err()).map(|failure| failure.reason).unwrap_or(reason)
            };
            let native_description = if translated {
                format!(
                    "Native Gray operations applied once to editable coverage samples under PhotoCraft depth, blending and opacity rules. Source parameter editability is lost; original configuration remains inert in the report. Later stages use the native result. {}",
                    applied.get(&pointer).and_then(|value| value.as_ref().ok()).map(String::as_str).unwrap_or("")
                )
            } else if !owner_committed && applied.get(&pointer).is_some_and(|value| value.is_ok()) {
                "Native auxiliary coverage was not installed because its owning source operation was not committed; temporary conversion results were discarded and the original configuration remains inert".into()
            } else if let Some(Err(message)) = applied.get(&pointer) {
                format!("Native coverage preserved; {}. Source configuration remains inert; later stages may receive different coverage.", message.message)
            } else {
                "Native coverage samples remain editable. Stored stage remains inactive; translation into native coverage operations is unfinished, not intrinsically impossible.".into()
            };
            let outcome = self.outcomes.push(ConversionEntry {
                feature_code: "xcf.coverage_effect".into(),
                source,
                targets: current_owner.targets.clone(),
                disposition: if translated { ConversionDisposition::NativeApproximation } else { ConversionDisposition::RetainedUnapplied },
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
            self.outcomes.property_with_reason(outcome, 45, effect.record_offset, effect.raw, reason)?;
            for property in effect.unknown_properties {
                self.outcomes.property(outcome, property.kind, property.offset, property.bytes)?;
            }
            if effect.mask_offset != 0 && masks_seen.insert(effect.mask_offset) {
                let start = effect.mask_offset;
                let fallback;
                let record = if let Some(record) = decoded_masks.get(&start) {
                    record
                } else {
                    fallback = match self.channel(start, ctl) {
                        Ok((name, properties, pixels, children)) => {
                            CoverageMaskRecord { name, properties, width: pixels.width, height: pixels.height, children }
                        }
                        Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                        Err(error) => {
                            self.outcomes.push(ConversionEntry {
                                feature_code: "xcf.coverage_effect_mask".into(),
                                source: conversion::source("coverage_effect_mask", ordinal as usize, start, "", &current_owner.source.item_path, None),
                                targets: current_owner.targets.clone(),
                                disposition: ConversionDisposition::RetainedUnapplied,
                                reason: ConversionReason::InvalidOptionalData,
                                source_description: format!("Optional effect-mask coverage could not be decoded: {error}"),
                                native_description: "Native owner coverage is preserved; the unapplied source stage and its unreadable mask remain inactive"
                                    .into(),
                                attachments: Vec::new(),
                            })?;
                            continue;
                        }
                    };
                    &fallback
                };
                let mask_applied = translated && decoded_masks.contains_key(&start);
                let mask_outcome = self.outcomes.push(ConversionEntry {
                    feature_code: "xcf.coverage_effect_mask".into(),
                    source: conversion::source("coverage_effect_mask", ordinal as usize, start, &record.name, &owner_entry.source.item_path, None),
                    targets: current_owner.targets.clone(), disposition: if mask_applied { ConversionDisposition::NativeApproximation } else { ConversionDisposition::RetainedUnapplied }, reason: if mask_applied { ConversionReason::NativePolicy } else { ConversionReason::MissingMapping },
                    source_description: format!("Source effect-mask coverage {}x{}; {} nested effects", record.width, record.height, record.children.len()),
                    native_description: if mask_applied { "Source image-global auxiliary pixels and supported nested operations became ordinary native layer-mask coverage; source operation parameters remain historical" } else if !owner_committed { "Temporary native auxiliary coverage was discarded because the owning source operation was not committed; original configuration remains inert" } else { "Mask identity and configuration retained inert with its unapplied coverage stage; native coverage conversion remains unfinished" }.into(), attachments: Vec::new(),
                })?;
                for property in &record.properties {
                    self.outcomes.property(mask_outcome, property.kind, property.offset, property.data)?;
                }
                self.meta_budget = self
                    .meta_budget
                    .checked_sub(record.children.len().saturating_mul(6 * std::mem::size_of::<usize>()))
                    .ok_or_else(|| invalid("coverage effect work exceeds metadata allowance"))?;
                work.extend(record.children.iter().rev().map(|pointer| (*pointer, depth + 1, mask_outcome)));
            }
        }
        self.warnings.layer(&owner_entry.source.name, "has stored coverage effects; native coverage conversion may lose parameter editability or leave unsupported stages inactive; see the conversion report");
        Ok(())
    }

    /// Records source samples lost below native precision without misclassifying their owner as fully
    /// preserved.
    fn precision_underflow(&mut self, owner: usize, pixels: &Pixels) -> Result<()> {
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
    path: Vec<u32>,
    group: bool,
    children: Vec<Node>,
    outcome: usize,
}

/// GIMP's layer mode as Photoshop's, with a note when the two differ.
fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("GIMP's legacy Soft light")),
        45 => (B::SoftLight, Some("GIMP's Soft light")),
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
    let w = i32::try_from(px.width).map_err(|_| invalid("drawable width overflow"))?;
    let h = i32::try_from(px.height).map_err(|_| invalid("drawable height overflow"))?;
    if w == 0 || h == 0 {
        return Err(invalid("drawable dimensions must be positive"));
    }
    let rect = Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?);
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
    surface.try_write_region(rect, &px.data).map_err(|error| invalid(format!("drawable tile allocation failed: {error}")))?;
    surface.prune();
    let added = surface.tile_count().saturating_sub(before) as u64;
    *budget = budget.saturating_sub(usize::try_from(photocraft_raster::stored_tiles_bytes(added, &surface.format())).unwrap_or(usize::MAX));
    Ok(())
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

/// Inspects an XCF header for unresolved historical sample order without decoding its document pixels.
pub(crate) fn historical_sample_order_request(bytes: &[u8]) -> Result<Option<crate::HistoricalXcfSampleOrderRequest>> {
    if !is_xcf(bytes) {
        return Err(invalid("not an XCF file"));
    }
    let mut c = Cursor { b: bytes, pos: MAGIC.len(), wide: false };
    let version = read_version(&mut c)?;
    let (width, height, base_type) = (c.u32()?, c.u32()?, c.u32()?);
    validate_canvas(width, height)?;
    if base_type > 2 {
        return Err(invalid(format!("unknown image type {base_type}")));
    }
    let precision = if version >= 4 { Precision::from_header(version, c.u32()?)? } else { Precision::GAMMA8 };
    if base_type == 2 && precision != Precision::GAMMA8 {
        return Err(invalid("indexed pixels require 8-bit nonlinear integer precision"));
    }
    sample_order_request(version, precision)
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
}

/// Decode only drawable prerequisites here; native metadata conversion consumes the retained properties later.
fn read_image_metadata<'a>(c: &mut Cursor<'a>, version: u32) -> Result<ImageMetadata<'a>> {
    // Image properties.
    let image_props = props_version(c, Some(version))?;
    let mut compression = Compression::None;
    let mut colormap = Vec::new();
    let mut icc = None;
    for p in &image_props {
        match p.kind {
            1 => {
                let n = be_u32(p.data).ok_or_else(|| invalid("the palette is missing its color count"))? as usize;
                let size = n.checked_mul(3).and_then(|bytes| bytes.checked_add(4)).ok_or_else(|| invalid("palette size overflow"))?;
                if n > 256 || p.data.len() != size {
                    return Err(invalid("the palette must contain its declared count of at most 256 RGB colors"));
                }
                colormap = p.data.get(4..).unwrap_or(&[]).as_chunks::<3>().0.to_vec();
            }
            17 => {
                compression = match p.data.first() {
                    Some(0) => Compression::None,
                    Some(1) => Compression::Rle,
                    Some(2) => Compression::Zlib,
                    Some(k) => return Err(invalid(format!("unknown compression {k}"))),
                    None => return Err(invalid("empty compression property")),
                }
            }
            21 => {
                for (name, data) in parasites(p.data) {
                    match name.as_str() {
                        "icc-profile" if !data.is_empty() => icc = Some(data),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ImageMetadata { properties: image_props, compression, colormap, icc })
}

/// Imports XCF with an explicit historical sample-order decision, preserving native data and durable
/// conversion outcomes.
pub(crate) fn import_with_sample_order(
    name: &str,
    bytes: &[u8],
    selected_order: Option<crate::HistoricalXcfSampleOrder>,
    ctl: &Interrupt<'_>,
) -> Result<ImportResult> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !is_xcf(bytes) {
        return Err(invalid("not an XCF file"));
    }
    let mut c = Cursor { b: bytes, pos: MAGIC.len(), wide: false };
    let version = read_version(&mut c)?;
    c.wide = version >= 11;
    let mut warnings = Warnings::default();
    if version > KNOWN_VERSION {
        warnings.note(&format!("XCF version {version} is newer than this reader (version {KNOWN_VERSION}); unknown parts were skipped"));
    }
    let (w, h, base_type) = (c.u32()?, c.u32()?, c.u32()?);
    validate_canvas(w, h)?;
    let precision = if version >= 4 { Precision::from_header(version, c.u32()?)? } else { Precision::GAMMA8 };
    if base_type > 2 {
        return Err(invalid(format!("unknown image type {base_type}")));
    }
    if base_type == 2 && precision != Precision::GAMMA8 {
        return Err(invalid("indexed pixels require 8-bit nonlinear integer precision"));
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

    let ImageMetadata { properties: image_props, compression, colormap, icc } = read_image_metadata(&mut c, version)?;
    let assigned_profile = icc.is_some();
    let native_profile = color::profile(mode, precision, icc)?;
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
    if version >= 18 {
        loop {
            let pointer = c.ptr()?;
            if pointer == 0 {
                break;
            }
            if path_ptrs.len() >= MAX_CHANNELS {
                return Err(invalid("more than 1,024 saved paths"));
            }
            path_ptrs.push(pointer);
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
        warnings,
        outcomes: conversion::Outcomes::new(version)?,
        clipped_to_backdrop: 0,
        initial_view: Default::default(),
        resolution_dpi: metadata::image_resolution(&image_props, 72.0),
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
            format!("Explicit {sample_order} sample byte order selected before decoding; structural fields remain big-endian. Native samples retain their declared precision and source transfer curve"),
        )?;
    }
    for property in &image_props {
        if property.kind == 21 {
            image.outcomes.parasites(image_outcome, property.offset, property.data, &["icc-profile"])?;
        } else if !matches!(property.kind, 1 | 17) {
            let reason = match property.kind {
                18 | 19 => ConversionReason::NativePolicy,
                20 | 22 | 24 | 27 | 39 => ConversionReason::NativeLimitation,
                _ => ConversionReason::MissingMapping,
            };
            image.outcomes.property_with_reason(image_outcome, property.kind, property.offset, property.data, reason)?;
        }
    }
    let (layers, linear_compositing) = image.import_layers(&layer_ptrs, fmt, mask_fmt, ctl)?;
    let (channels, selection) = image.import_channels(&channel_ptrs, mask_fmt, ctl)?;

    let mut document = Document::new(name, Size::new(w, h), mode, depth);
    document.layers = layers;
    document.channels = channels;
    document.selection = selection;
    metadata::image(&mut document, &image_props, version, &mut image.initial_view, &mut image.outcomes, image_outcome, &mut image.meta_budget)?;
    document.icc_profile = Some(native_profile);
    image.outcomes.push(ConversionEntry {
        feature_code: "xcf.color_encoding".into(),
        source: conversion::source("image", 0, 0, "", &[], None),
        targets: vec![NativeTargetRef { label: "Document color profile".into(), ..Default::default() }],
        disposition: ConversionDisposition::NativeEquivalent,
        reason: ConversionReason::NativePolicy,
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
        native_description: "Native ICC profile preserves the source colorants and sample transfer curve; samples retain native precision and range".into(),
        attachments: Vec::new(),
    })?;
    import_paths(&mut document, &mut image, &image_props, &path_ptrs, image_outcome, ctl)?;
    import_vectors(&mut document, &mut image, ctl)?;
    text::convert_colors(&mut document)?;
    import_native_effects(&mut document, &mut image, ctl)?;
    add_document_warnings(&mut image, &document, precision, linear_compositing);
    image.initial_view.validate(&document)?;
    let initial_view = (image.initial_view != Default::default()).then_some(image.initial_view);
    document.metadata.import_conversion = Some(Arc::new(image.outcomes.finish()?));
    ctl.progress(1.0);
    Ok(ImportResult { initial_view, document, warnings: image.warnings.finish(), source_read_only: true, preview_only: false })
}

/// Report native compositing differences without changing imported content or its durable conversion report.
fn add_document_warnings(image: &mut Image<'_>, document: &Document, precision: Precision, linear_compositing: bool) {
    // GIMP clips a layer in any mode but Normal to the layers beneath it, so it shows nothing where
    // they are transparent; PhotoCraft (like Photoshop) shows it there. Only tell when there is
    // transparency to show through: a bottom layer that is not opaque.
    if image.clipped_to_backdrop > 0 && document.layers.first().is_some_and(|l| !l.locks.transparency || l.is_group()) {
        image.warnings.note(&format!(
            "{} layer(s) use a mode GIMP clips to the layers beneath; where those are transparent GIMP shows nothing and PhotoCraft shows the layer",
            image.clipped_to_backdrop
        ));
    }
    // GIMP blends and composites in linear light by default (since 2.10); PhotoCraft, like
    // Photoshop, in the document's own encoding, so semi-transparent edges and most blend modes
    // render a little differently. Layers set to a perceptual space in GIMP match.
    if !precision.linear && linear_compositing {
        image.warnings.note("GIMP composites this image in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
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
        let mut omitted = false;
        // GIMP's serialized list is top-first; its last stored filter is the first
        // operation on the drawable. Independently saved stack evidence verifies this.
        for pointer in pending.pointers.iter().rev().copied() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut source = image.outcomes.entry(pending.owner)?.source.clone();
            source.kind = "layer_effect".into();
            source.ordinal = ordinal as u32;
            source.record_offset = Some(pointer as u64);
            ordinal = ordinal.checked_add(1).ok_or_else(|| invalid("effect ordinal overflow"))?;
            let effect = match effects::decode(&image.c, pointer, image.version, &mut image.meta_budget, ctl) {
                Ok(effect) => effect,
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => {
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
                    omitted = true;
                    continue;
                }
            };
            source.name = effect.name.into();
            let mut notes = Vec::new();
            if !effect.icon.is_empty() {
                notes.push("Source icon identity remains inert presentation metadata".into());
            }
            for argument in &effect.arguments {
                match &argument.value {
                    effects::ArgumentValue::String(value) => notes.push(format!(
                        "Stored string argument {} ({} bytes) remains bounded source configuration; source strings grant no external resource access",
                        argument.name,
                        value.len()
                    )),
                    effects::ArgumentValue::Config(config) => {
                        notes.push(format!("Stored configuration {} ({} bytes) was decoded only as bounded data", argument.name, config.raw.len()))
                    }
                    effects::ArgumentValue::Unknown { kind, bytes } => {
                        notes.push(format!("Unknown argument {} type {kind} ({} bytes) is retained inert", argument.name, bytes.len()))
                    }
                    effects::ArgumentValue::Invalid { kind, bytes, message } => {
                        notes.push(format!("Invalid argument {} type {kind} ({} bytes): {message}", argument.name, bytes.len()))
                    }
                    _ => {}
                }
            }
            let result: std::result::Result<(), effects::MappingFailure> =
                Err(effects::MappingFailure::from("Native effect mapping is not implemented; stored configuration remains inert"));
            if effect.blend_space != 0 || effect.composite_space != 0 || effect.composite_mode != 0 || effect.clip || effect.region != 0 {
                notes.push(format!("Native document blending/coverage applies; stored blend space {}, composite space {}, composite mode {}, clip {}, region {} are historical", effect.blend_space, effect.composite_space, effect.composite_mode, effect.clip, effect.region));
            }
            if omitted {
                notes.push(
                    "An earlier source stage was omitted; subsequent native stages receive the preserved native result instead of the original source pipeline"
                        .into(),
                );
            }
            let (disposition, reason, native) = match result {
                Ok(()) => (
                    ConversionDisposition::NativeApproximation,
                    ConversionReason::NativePolicy,
                    format!("Editable owner-local native smart contents and native filter/style rules. {}", notes.join("; ")),
                ),
                Err(failure) => {
                    omitted = true;
                    (
                        ConversionDisposition::RetainedUnapplied,
                        failure.reason,
                        format!("Effect not applied; usable separate native content preserved. {} {}", failure.message, notes.join("; ")),
                    )
                }
            };
            let outcome = image.outcomes.push(ConversionEntry {
                feature_code: "xcf.layer_effect".into(),
                source,
                targets: vec![NativeTargetRef {
                    layer_id: Some(pending.layer),
                    label: "Native owner and embedded filter contents".into(),
                    ..Default::default()
                }],
                disposition,
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
            image.outcomes.property_with_reason(outcome, 45, effect.record_offset, effect.raw, reason)?;
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
                    native_description:
                        "Referenced effect mask data is not retained in this decoder slice; stored effect configuration remains inert and is not executed"
                            .into(),
                    attachments: Vec::new(),
                })?;
                image.warnings.layer(&original.name, "has a deferred effect mask whose referenced data is not retained; see the conversion report");
            }
            for property in effect.unknown_properties {
                image.outcomes.property(outcome, property.kind, property.offset, property.bytes)?;
            }
        }
        image.warnings.layer(
            &original.name,
            if omitted {
                "contains source effects that were retained without application; see the conversion report"
            } else {
                "uses editable native filter contents with PhotoCraft compositing rules; see the conversion report"
            },
        );
    }
    Ok(())
}

/// Translate saved cubic geometry into ordinary native Paths panel entries.
fn import_paths<'a>(
    document: &mut Document,
    image: &mut Image<'a>,
    properties: &[Prop<'a>],
    pointers: &[usize],
    _owner: usize,
    ctl: &Interrupt<'_>,
) -> Result<()> {
    import_paths_with_allowance(document, image, properties, pointers, ctl, MAX_METADATA_BYTES)
}

/// Share one allowance across framed source payloads and each owned native record.
/// Repeated pointers must pay for every retained copy, not merely their shared input bytes.
fn import_paths_with_allowance<'a>(
    document: &mut Document,
    image: &mut Image<'a>,
    properties: &[Prop<'a>],
    pointers: &[usize],
    ctl: &Interrupt<'_>,
    mut remaining: usize,
) -> Result<()> {
    let mut records: Vec<SavedPathRecord<'a>> = Vec::new();
    let mut ordinal = 0usize;
    for property in properties.iter().filter(|p| matches!(p.kind, 23 | 25)) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        remaining = remaining.checked_sub(property.data.len()).ok_or_else(|| invalid("saved paths exceed the 32 MiB geometry allowance"))?;
        let paths = match if property.kind == 23 { paths::legacy(property.data) } else { paths::vectors(property.data) } {
            Ok(paths) => paths,
            Err(error) => {
                unavailable_path(image, ordinal, property.offset, "", &error, None)?;
                ordinal += 1;
                continue;
            }
        };
        if paths.len() > MAX_CHANNELS.saturating_sub(records.len()) {
            return Err(invalid("more than 1,024 saved paths"));
        }
        for path in paths {
            admit_path(&mut remaining, &path)?;
            records.push((ordinal, property.offset, path, None));
            ordinal += 1;
        }
    }
    for &offset in pointers {
        ctl.check().map_err(|_| IoError::Cancelled)?;
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
                unavailable_path(image, ordinal, offset, "", &error, None)?;
                ordinal += 1;
                continue;
            }
        };
        remaining = remaining.checked_sub(length).ok_or_else(|| invalid("saved paths exceed the 32 MiB geometry allowance"))?;
        let name = std::str::from_utf8(name).unwrap_or("Invalid UTF-8 path name");
        if records.len() >= MAX_CHANNELS {
            return Err(invalid("more than 1,024 saved paths"));
        }
        match paths::standalone(&image.c, offset) {
            Ok(path) => {
                admit_path(&mut remaining, &path)?;
                records.push((ordinal, offset, path, Some(raw)));
            }
            Err(error) => unavailable_path(image, ordinal, offset, name, &error, Some(raw))?,
        }
        ordinal += 1;
    }
    if records.len() > MAX_CHANNELS {
        return Err(invalid("more than 1,024 saved paths"));
    }
    for (ordinal, offset, path, raw) in records {
        let index = document.paths.len();
        let name = path.named.name.clone();
        if path.selected {
            image.initial_view.selected_path = Some(index);
        }
        if let Some(tattoo) = path.tattoo {
            image.path_tattoos.entry(tattoo).and_modify(|value| *value = None).or_insert(Some(index));
        }
        let outcome = image.outcomes.push(ConversionEntry {
            feature_code: "xcf.path".into(),
            source: conversion::source("path", ordinal, offset, &name, &[], path.tattoo),
            targets: vec![NativeTargetRef { path_index: Some(index as u32), label: name, ..Default::default() }],
            disposition: if path.source_only { ConversionDisposition::NativeApproximation } else { ConversionDisposition::NativeEquivalent },
            reason: ConversionReason::NativePolicy,
            source_description: "Saved cubic anchors, handles and open/closed subpaths".into(),
            native_description: if path.source_only {
                "Native editable cubic path; source auxiliary axes and non-geometric flags are not editing rules"
            } else {
                "Native editable cubic path with document-coordinate geometry"
            }
            .into(),
            attachments: Vec::new(),
        })?;
        if let Some(raw) = raw {
            image.outcomes.property(outcome, 25, offset, raw)?;
        }
        document.paths.push(path.named);
    }
    // Raw source path properties are already covered by the image's bounded inert retention.
    Ok(())
}

type SavedPathRecord<'a> = (usize, usize, paths::DecodedPath, Option<&'a [u8]>);

/// Account for owned strings, native geometry capacities and the retained record itself
/// before pushing it. Wire payload lengths omit names and undercount native knot storage.
fn admit_path(remaining: &mut usize, path: &paths::DecodedPath) -> Result<()> {
    let overflow = || invalid("saved path retained geometry size overflow");
    let mut bytes = std::mem::size_of::<SavedPathRecord<'_>>()
        .checked_add(path.named.name.capacity())
        .and_then(|bytes| bytes.checked_add(path.named.psd_raw.as_ref().map_or(0, |raw| raw.capacity())))
        .ok_or_else(overflow)?;
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
        let index = image.path_tattoos.get(&vector.path_tattoo).copied().flatten();
        let Some(path) = index.and_then(|index| document.paths.get(index)) else {
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
        let knots = path.path.subpaths.iter().try_fold(0usize, |n, s| n.checked_add(s.knots.len())).ok_or_else(|| invalid("vector geometry count overflow"))?;
        let bytes = knots
            .checked_mul(std::mem::size_of::<photocraft_doc::vector::Knot>())
            .and_then(|n| n.checked_add(path.path.subpaths.len().saturating_mul(std::mem::size_of::<photocraft_doc::vector::Subpath>())))
            .ok_or_else(|| invalid("vector geometry size overflow"))?;
        image.meta_budget = image.meta_budget.checked_sub(bytes).ok_or_else(|| invalid("vector layer copies exceed the geometry metadata allowance"))?;
        vector.shape.path = path.path.clone();
        let layer = document.layer_mut(id).ok_or_else(|| invalid("missing native vector target"))?;
        layer.content = LayerContent::Shape(vector.shape);
        image.outcomes.notice(
            outcome,
            "xcf.vector_layer",
            ConversionReason::NativePolicy,
            ("Saved vector geometry and paint".into(), "Native editable shape; PhotoCraft fill/stroke, geometry editing and rasterization apply".into()),
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

/// Puts `node` where its item path says, under the groups already placed; a path that leads
/// nowhere (at any depth) puts it at the top level, so later siblings keep their indices.
fn place(roots: &mut Vec<Node>, node: Node, warnings: &mut Warnings, outcomes: &mut conversion::Outcomes<'_>) -> Result<()> {
    let depth = node.path.len().saturating_sub(1);
    let mut probe: &Vec<Node> = roots;
    let reachable = node.path.iter().take(depth).all(|&index| match probe.get(index as usize) {
        Some(n) if n.group => {
            probe = &n.children;
            true
        }
        _ => false,
    });
    if !reachable {
        warnings.layer(&node.layer.name, "has an item path that leads to no group; opened at the top level");
        outcomes.detail(
            node.outcome,
            "xcf.item_path",
            ConversionReason::InvalidOptionalData,
            "Source item path leads to no group".into(),
            "Layer opened at the native top level".into(),
        )?;
        let mut node = node;
        node.path = vec![];
        roots.push(node);
        return Ok(());
    }
    let mut level = roots;
    for &index in node.path.iter().take(depth) {
        level = &mut level[index as usize].children;
    }
    level.push(node);
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
