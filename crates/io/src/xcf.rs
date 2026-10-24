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
mod coverage;
mod coverage_color;
#[cfg(test)]
mod coverage_effect_tests;
mod coverage_geometry;
mod coverage_spatial;
mod effects;
#[cfg(test)]
mod historical_precision_tests;
mod metadata;
mod metadata_packets;
#[cfg(test)]
mod metadata_tests;
mod native_alignment;
mod native_displacement;
mod native_effects;
mod native_masks;
mod native_snapshot;
#[cfg(test)]
mod native_stage_tests;
mod paths;
mod resources;
#[cfg(test)]
mod tests;
mod text;
pub(crate) mod transport;

use std::collections::HashMap;
use std::io::Read;

use photocraft_codecs::f16;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, NativeTargetRef};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

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

/// How samples are stored: bytes per sample, whether they are floats, and whether the values are
/// linear light, the assigned profile's own curve, or the perceptual sRGB transfer curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Precision {
    bytes: usize,
    float: bool,
    linear: bool,
    perceptual: bool,
}

impl Precision {
    const GAMMA8: Precision = Precision { bytes: 1, float: false, linear: false, perceptual: false };

    fn from_header(version: u32, code: u32) -> Result<Precision> {
        let p = |bytes, float, linear| Ok(Precision { bytes, float, linear, perceptual: false });
        match version {
            0..=3 => Ok(Self::GAMMA8),
            4 => match code {
                0 => p(1, false, false),
                1 => p(2, false, false),
                2 => p(4, false, true),
                3 => p(2, true, true),
                4 => p(4, true, true),
                c => Err(invalid(format!("unknown precision {c} in an XCF version 4 file"))),
            },
            5 | 6 => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                400 => p(2, true, true),
                450 => p(2, true, false),
                500 => p(4, true, true),
                550 => p(4, true, false),
                c => Err(invalid(format!("unknown precision {c} in an XCF version {version} file"))),
            },
            _ if code % 100 == 75 => {
                let mut precision = Self::from_header(version, code - 25)?;
                precision.perceptual = true;
                Ok(precision)
            }
            _ => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                500 => p(2, true, true),
                550 => p(2, true, false),
                600 => p(4, true, true),
                650 => p(4, true, false),
                700 => p(8, true, true),
                750 => p(8, true, false),
                c => Err(invalid(format!("unknown precision {c}"))),
            },
        }
    }

    /// The document depth the samples open at.
    fn depth(self) -> SampleType {
        match (self.float, self.bytes) {
            (false, 1) => SampleType::U8,
            (false, 2) => SampleType::U16,
            _ => SampleType::F32,
        }
    }

    fn reduces_precision(self) -> bool {
        matches!((self.float, self.bytes), (false, 4) | (true, 8))
    }

    /// Decode before narrowing, so invalid coverage cannot round into range. The second
    /// result records a nonzero source value rounded to native signed zero.
    fn sample(self, b: &[u8], coverage: bool) -> Result<(f32, bool)> {
        let source = match (self.float, self.bytes, b) {
            (false, 1, [v]) => f64::from(*v) / 255.0,
            (false, 2, [a, b]) => f64::from(u16::from_be_bytes([*a, *b])) / 65535.0,
            (false, 4, [a, b, c, d]) => f64::from(u32::from_be_bytes([*a, *b, *c, *d])) / f64::from(u32::MAX),
            (true, 2, [a, b]) => f64::from(f16::from_bits(u16::from_be_bytes([*a, *b])).to_f32()),
            (true, 4, [a, b, c, d]) => f64::from(f32::from_be_bytes([*a, *b, *c, *d])),
            (true, 8, [a, b, c, d, e, f, g, h]) => f64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h]),
            _ => return Err(invalid("sample size does not match the image precision")),
        };
        if !source.is_finite() {
            return Err(invalid("a floating-point sample is not finite"));
        }
        if coverage && !(0.0..=1.0).contains(&source) {
            return Err(invalid("a coverage sample is outside the range 0 to 1"));
        }
        if source.abs() > f64::from(f32::MAX) {
            return Err(invalid("a floating-point sample exceeds native F32 range"));
        }
        let native = source as f32;
        if !native.is_finite() {
            return Err(invalid("a floating-point sample exceeds native F32 range"));
        }
        Ok((native, source != 0.0 && native == 0.0))
    }
}

/// U8/U16 samples already have finite normalized color and coverage domains.
/// Keep their row conversion separate from validation/narrowing of floats and U32.
#[inline]
fn integer_samples(raw: &[u8], output: &mut [f32], sample_bytes: usize) -> Result<()> {
    match sample_bytes {
        1 if raw.len() == output.len() => {
            for (out, value) in output.iter_mut().zip(raw) {
                *out = f32::from(*value) / 255.0;
            }
        }
        2 if output.len().checked_mul(2) == Some(raw.len()) => {
            for (out, value) in output.iter_mut().zip(raw.as_chunks::<2>().0) {
                *out = f32::from(u16::from_be_bytes(*value)) / 65535.0;
            }
        }
        _ => return Err(invalid("integer row size does not match the image precision")),
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Rle,
    Zlib,
}

/// Big-endian reads with bounds checks; every failure is an error, never a panic.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
    /// XCF 11 and later store pointers as 64-bit.
    wide: bool,
}

impl<'a> Cursor<'a> {
    fn at(&self, pos: usize) -> Cursor<'a> {
        Cursor { b: self.b, pos, wide: self.wide }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("offset overflow"))?;
        let s = self.b.get(self.pos..end).ok_or_else(|| invalid(format!("the file ends early (wanted {n} bytes at {})", self.pos)))?;
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn ptr(&mut self) -> Result<usize> {
        if self.wide {
            let s = self.take(8)?;
            usize::try_from(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])).map_err(|_| invalid("pointer does not fit in memory"))
        } else {
            self.u32().map(|v| v as usize)
        }
    }

    /// A STRING: `n+1` then `n` bytes and a NUL (0 for the empty string).
    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The same bounded STRING read without allocating or decoding an unused field.
    fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return Err(invalid(format!("a string of {n} bytes exceeds the limit")));
        }
        let s = self.take(n)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s))
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One property record: its type and payload.
struct Prop<'a> {
    offset: usize,
    kind: u32,
    data: &'a [u8],
}

/// Read a drawable/path property list, using the declared framing for image-only properties.
fn props<'a>(c: &mut Cursor<'a>) -> Result<Vec<Prop<'a>>> {
    props_version(c, None)
}

/// Image properties can carry historical lengths that GIMP itself ignores. Resolve their
/// documented framing using the actual file epoch rather than probing payload bytes as headers.
fn props_version<'a>(c: &mut Cursor<'a>, version: Option<u32>) -> Result<Vec<Prop<'a>>> {
    let mut out = Vec::new();
    loop {
        let offset = c.pos;
        let kind = c.u32()?;
        let len = c.u32()? as usize;
        if kind == 0 {
            return Ok(out);
        }
        // PROP_COLORMAP's length word was wrong in some releases (n+4 for 3n+4 bytes); its real
        // size follows from the count. Unit strings changed at XCF21.
        // PROP_LINK_LAYER's length can be four bytes short (#8857); read all its fields.
        let len = match kind {
            1 => {
                let n = c.at(c.pos).u32()? as usize;
                4usize.checked_add(n.checked_mul(3).ok_or_else(|| invalid("colormap overflow"))?).ok_or_else(|| invalid("colormap overflow"))?
            }
            24 => {
                if let Some(version) = version {
                    user_unit_len(c, version)?
                } else {
                    len
                }
            }
            48 => link_layer_len(c)?,
            _ => len,
        };
        if len > MAX_PAYLOAD {
            return Err(invalid(format!("property {kind} declares {len} bytes")));
        }
        let data = c.take(len)?;
        out.push(Prop { offset, kind, data });
        if out.len() > 4096 {
            return Err(invalid("too many properties"));
        }
    }
}

/// A PROP_LINK_LAYER payload: flags, a path STRING, then the source width and height.
/// The dimensions are present for raster links too, where their values are unused.
fn link_layer_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(4)?;
    p.string_bytes()?;
    p.skip(8)?;
    Ok(p.pos - c.pos)
}

/// The real size of a PROP_USER_UNIT payload: a float, a count and three (XCF 21+) or five
/// strings. Measured without consuming.
fn user_unit_len(c: &Cursor<'_>, version: u32) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(8)?;
    for _ in 0..if version < 21 { 5 } else { 3 } {
        p.string()?;
    }
    Ok(p.pos - c.pos)
}

fn be_u32(b: &[u8]) -> Option<u32> {
    b.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be_i32(b: &[u8]) -> Option<i32> {
    be_u32(b).map(|v| v as i32)
}

fn be_f32(b: &[u8]) -> Option<f32> {
    be_u32(b).map(f32::from_bits)
}

/// The parasites of a PROP_PARASITES payload as (name, data); a later duplicate wins.
fn parasites(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // Map each name to its output slot so duplicate replacement is O(1). The parasite
    // records are file-controlled, so scanning `out` with iter_mut().find() per record is
    // O(n^2) and lets a crafted payload with many distinct names stall import. The map
    // preserves insertion order and last-value-wins behavior.
    let mut index: HashMap<String, usize> = HashMap::new();
    while c.pos < data.len() {
        let Ok(name) = c.string() else { break };
        let (Ok(_flags), Ok(len)) = (c.u32(), c.u32()) else { break };
        let Ok(payload) = c.take(len as usize) else { break };
        match index.get(&name) {
            Some(&i) => out[i].1 = payload.to_vec(),
            None => {
                index.insert(name.clone(), out.len());
                out.push((name, payload.to_vec()));
            }
        }
    }
    out
}

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel.
fn decode_tile(c: &Cursor<'_>, at: usize, end: usize, compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    let input = c.b.get(at..end.min(c.b.len())).ok_or_else(|| invalid("a tile pointer is outside the file"))?;
    match compression {
        Compression::None => input.get(..expected).map(<[u8]>::to_vec).ok_or_else(|| invalid("an uncompressed tile is cut short")),
        Compression::Zlib => {
            let mut out = Vec::with_capacity(expected);
            // Read one byte beyond the expected tile: stopping exactly at its pixels would
            // skip the decoder's trailer/checksum check and accept truncation or excess data.
            flate2::read::ZlibDecoder::new(input).take(expected as u64 + 1).read_to_end(&mut out).map_err(|e| invalid(format!("zlib tile: {e}")))?;
            if out.len() != expected {
                return Err(invalid("a zlib tile has an invalid decoded size"));
            }
            Ok(out)
        }
        Compression::Rle => rle_decode(input, bpp, pixels),
    }
}

/// RLE tile data: one run-length stream per byte position of the pixel, each of `pixels` bytes.
fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; pixels * bpp];
    let mut pos = 0usize;
    let mut plane = Vec::with_capacity(pixels);
    for b in 0..bpp {
        plane.clear();
        while plane.len() < pixels {
            let op = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
            pos += 1;
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        false,
                    )
                }
                128 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        true,
                    )
                }
                _ => (256 - usize::from(op), true),
            };
            if run == 0 {
                return Err(invalid("an RLE run has zero length"));
            }
            if plane.len() + run > pixels {
                return Err(invalid("an RLE run crosses the end of its tile"));
            }
            if verbatim {
                let data = input.get(pos..pos + run).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += run;
                plane.extend_from_slice(data);
            } else {
                let v = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += 1;
                plane.resize(plane.len() + run, v);
            }
        }
        for (i, v) in plane.iter().enumerate() {
            out[i * bpp + b] = *v;
        }
    }
    Ok(out)
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
    native_source_budget: usize,
    path_tattoos: HashMap<u32, Option<usize>>,
    pending_vectors: Vec<(photocraft_doc::LayerId, usize, paths::VectorStyle)>,
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

    /// A distinct source identity for the ordinary native mask on a layer effect.
    /// Call after the parent stage result is known, retaining raw display/unknown props.
    fn layer_effect_mask_outcome(&mut self, parent: usize, offset: usize, name: &str, properties: &[Prop<'a>], committed: bool) -> Result<usize> {
        let parent = self.outcomes.entry(parent)?.clone();
        let mut targets = parent.targets;
        for target in &mut targets {
            target.label = if committed { "Native layer-effect mask" } else { "Native parent layer; auxiliary source coverage was not installed" }.into();
        }
        let outcome = self.outcomes.push(ConversionEntry {
            feature_code: "xcf.layer_effect_mask".into(),
            source: conversion::source(
                "layer_effect_mask",
                parent.source.ordinal as usize,
                offset,
                name,
                &parent.source.item_path,
                properties.iter().find(|property| property.kind == 20).and_then(|property| be_u32(property.data)),
            ),
            targets,
            disposition: if committed { ConversionDisposition::NativeEquivalent } else { ConversionDisposition::RetainedUnapplied },
            reason: ConversionReason::NativePolicy,
            source_description: "Stored image-global layer-effect mask coverage".into(),
            native_description: if committed {
                "Stored coverage became an ordinary editable native layer/filter mask; supported nested coverage conversions are reported separately"
            } else {
                "The owning source operation was not committed; temporary auxiliary coverage was discarded and its configuration remains inert"
            }
            .into(),
            attachments: Vec::new(),
        })?;
        for property in properties {
            self.outcomes.property(outcome, property.kind, property.offset, property.data)?;
        }
        Ok(outcome)
    }

    fn apply_coverage_stack(
        &mut self,
        pointers: &[usize],
        surface: &mut Surface,
        bounds: Rect,
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
            let result = match auxiliary {
                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                Err(error) => Err(effects::MappingFailure {
                    reason: ConversionReason::InvalidOptionalData,
                    message: format!("Source auxiliary coverage could not be converted; operation remains inactive: {error}"),
                }),
                Ok(Err(failure)) => Err(failure),
                Ok(Ok(mask)) => coverage::apply(
                    surface,
                    bounds,
                    &effect,
                    mask.as_ref(),
                    self.precision.linear,
                    &mut coverage::Budgets { workspace: &mut self.native_source_budget, metadata: &mut self.meta_budget },
                    ctl,
                )?,
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
            let (name, properties, pixels, children) = self.channel(offset, ctl)?;
            // Independently saved source effect masks use image-global pixels.
            // A nonstandard layer-style placement has no established source meaning.
            if properties.iter().any(|property| property.kind == 15 && property.data.iter().any(|byte| *byte != 0)) {
                return Ok(Err("Source effect-mask channel has an unestablished placement property; pixels were not silently repositioned".into()));
            }
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, self.precision.depth(), false));
            write_pixels(&mut surface, &pixels, 0, 0)?;
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

/// Writes `px` (its channels matching `surface`'s) at `x, y` of the document.
fn write_pixels(surface: &mut Surface, px: &Pixels, x: i32, y: i32) -> Result<()> {
    let (w, h) = (px.width as i32, px.height as i32);
    let rect = Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?);
    if surface.channels() != px.channels || px.data.len() != (px.width as usize) * (px.height as usize) * px.channels {
        return Err(invalid("pixel data does not match the drawable"));
    }
    surface.write_region(rect, &px.data);
    surface.prune();
    Ok(())
}

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

fn validate_canvas(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(invalid(format!("canvas {w}x{h} must be 1 to {MAX_SIDE} pixels a side")));
    }
    if (w as usize).saturating_mul(h as usize).saturating_mul(16) > MAX_PIXEL_BYTES {
        return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
    }
    Ok(())
}

fn sample_order_request(version: u32, precision: Precision) -> Result<Option<crate::HistoricalXcfSampleOrderRequest>> {
    if (7..=11).contains(&version) && precision.bytes > 1 {
        let bits_per_sample = precision.bytes.checked_mul(8).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| invalid("precision bit count overflows"))?;
        Ok(Some(crate::HistoricalXcfSampleOrderRequest { version, bits_per_sample }))
    } else {
        Ok(None)
    }
}

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

    // Image properties.
    let image_props = props_version(&mut c, Some(version))?;
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
        resolution_dpi: image_props.iter().find(|p| p.kind == 19).and_then(|p| be_f32(p.data)).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(72.0),
        pending_effects: Vec::new(),
        native_source_budget: MAX_PIXEL_BYTES,
        path_tattoos: HashMap::new(),
        pending_vectors: Vec::new(),
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
    let total = layer_ptrs.len().max(1) as f32;

    // Layers, topmost first in the file.
    let mut nodes: Vec<Node> = Vec::new();
    let mut linear_compositing = false;
    for (i, &ptr) in layer_ptrs.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(0.05 + 0.85 * i as f32 / total);
        if let Some((node, linear)) = image.layer(i, ptr, fmt, mask_fmt, ctl)? {
            linear_compositing |= linear;
            nodes.push(node);
        }
    }
    let mut roots: Vec<Node> = Vec::new();
    for (i, mut node) in nodes.into_iter().enumerate() {
        // Default path: the item's position at the top level, as a file without groups has it.
        if node.path.is_empty() {
            node.path = vec![i as u32];
        }
        if node.path.len() > MAX_DEPTH {
            image.warnings.layer(&node.layer.name, "is nested deeper than the reader allows and was moved to the top level");
            image.outcomes.detail(
                node.outcome,
                "xcf.item_path",
                ConversionReason::NativeLimitation,
                "Source hierarchy exceeds 64 components".into(),
                "Layer moved to the native top level; source identity retains the ordinal and bounded path prefix".into(),
            )?;
            node.path.truncate(1);
        }
        place(&mut roots, node, &mut image.warnings, &mut image.outcomes)?;
    }
    let layers = finish(roots);

    // Channels: the selection and the extras.
    let mut channels = Vec::new();
    let mut selection = None;
    for (ordinal, &ptr) in channel_ptrs.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (name, cprops, px, coverage_effects) = image.channel(ptr, ctl)?;
        let mut surface = Surface::new(mask_fmt);
        write_pixels(&mut surface, &px, 0, 0)?;
        let is_selection = cprops.iter().any(|p| p.kind == 4);
        let native_selection = is_selection && selection.is_none();
        let channel_outcome = image.outcomes.push(ConversionEntry {
            feature_code: if is_selection { "xcf.selection" } else { "xcf.channel" }.into(),
            source: conversion::source(
                if is_selection { "selection" } else { "channel" },
                ordinal,
                ptr,
                &name,
                &[],
                cprops.iter().find(|p| p.kind == 20).and_then(|p| be_u32(p.data)),
            ),
            targets: vec![NativeTargetRef {
                channel_index: if native_selection { None } else { Some(channels.len() as u32) },
                label: if native_selection { "Document selection" } else { "Alpha channel" }.into(),
                ..Default::default()
            }],
            disposition: if precision.reduces_precision() { ConversionDisposition::NativeApproximation } else { ConversionDisposition::NativeEquivalent },
            reason: ConversionReason::NativePolicy,
            source_description: "Stored channel coverage samples".into(),
            native_description: "Native coverage surface".into(),
            attachments: Vec::new(),
        })?;
        image.precision_underflow(channel_outcome, &px)?;
        image.translate_coverage_effects(channel_outcome, &coverage_effects, &mut surface, Rect::new(0, 0, px.width as i32, px.height as i32), ctl)?;
        let mut channel = AlphaChannel::new(name, surface);
        let handled = metadata::channel_overlay(&mut channel, &cprops);
        for property in &cprops {
            if !matches!(property.kind, 4 | 20) && !handled.contains(&property.kind) {
                let reason = match property.kind {
                    3 | 8 => ConversionReason::NativePolicy,
                    6 | 14 | 16 | 33 | 38 => ConversionReason::InvalidOptionalData,
                    9 | 28 | 32 | 34 | 42 => ConversionReason::NativeLimitation,
                    _ => ConversionReason::MissingMapping,
                };
                image.outcomes.property_with_reason(channel_outcome, property.kind, property.offset, property.data, reason)?;
            }
        }
        if native_selection {
            selection = Some(channel.surface);
        } else {
            // Preserve ambiguous additional selection records as ordinary saved alpha channels.
            if is_selection {
                image.outcomes.detail(
                    channel_outcome,
                    "xcf.duplicate_selection",
                    ConversionReason::InvalidOptionalData,
                    "More than one source channel is marked as the active selection".into(),
                    "First selection kept; additional coverage retained as an ordinary alpha channel".into(),
                )?;
            }
            if metadata::flag(&cprops, 8) {
                image.initial_view.visible_alpha_channels.push(channels.len());
            }
            if metadata::active(&cprops, 3) {
                if matches!(image.initial_view.target, crate::initial_view::NativeImportTarget::AlphaChannel(_)) {
                    image.outcomes.detail(
                        channel_outcome,
                        "xcf.active_channels",
                        ConversionReason::NativeLimitation,
                        "Multiple source channels selected".into(),
                        "Native initial view targets one alpha channel; original markers remain inert".into(),
                    )?;
                } else {
                    image.initial_view.target = crate::initial_view::NativeImportTarget::AlphaChannel(channels.len());
                }
            }
            channels.push(channel);
        }
    }

    let mut document = Document::new(name, Size::new(w, h), mode, depth);
    document.layers = layers;
    document.channels = channels;
    document.selection = selection;
    metadata::image(&mut document, &image_props, version, &mut image.initial_view, &mut image.outcomes, image_outcome, &mut image.meta_budget)?;
    let assigned_profile = icc.is_some();
    document.icc_profile = Some(color::profile(mode, precision, icc)?);
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
    image.initial_view.validate(&document)?;
    let initial_view = (image.initial_view != Default::default()).then_some(image.initial_view);
    document.metadata.import_conversion = Some(image.outcomes.finish()?);
    ctl.progress(1.0);
    Ok(ImportResult { initial_view, document, warnings: image.warnings.finish(), source_read_only: true, preview_only: false })
}

/// Install owner-local native data, preserving each source operation as a historical event.
fn import_native_effects<'a>(document: &mut Document, image: &mut Image<'a>, ctl: &Interrupt<'_>) -> Result<()> {
    let mut ordinal = 0usize;
    for pending in std::mem::take(&mut image.pending_effects) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let original = document.layer(pending.layer).cloned().ok_or_else(|| invalid("native effect owner is missing"))?;
        let mut builder = None;
        let mut applied = false;
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
            let context = effects::MappingContext {
                mode: document.mode,
                linear: image.precision.linear,
                owner_bounds: Rect::from_xywh(0, 0, pending.bounds.width(), pending.bounds.height()),
                native_filter_bounds: Rect::from_xywh(0, 0, pending.bounds.width(), pending.bounds.height()),
                target_profile: document.icc_profile.clone(),
                resolution_dpi: document.resolution_dpi,
                remaining_generation_bytes: image.budget.min(image.native_source_budget),
                remaining_metadata_bytes: image.meta_budget,
                interrupt: *ctl,
            };
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
            let mapped = effects::map(&effect, &context);
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut mask_report = None;
            let result = match mapped {
                Err(failure) => Err(failure),
                Ok(mapping) => {
                    // Generated native parameters (notably native LUT text) share the
                    // existing metadata ceiling; a tiny repeated source cannot grow it freely.
                    let params = native_parameter_bytes(&mapping)?;
                    image.meta_budget =
                        image.meta_budget.checked_sub(params).ok_or_else(|| invalid("native effect parameters exceed the shared metadata allowance"))?;
                    if builder.is_none() {
                        match native_effects::NativeOwner::new(document, &original, pending.bounds, &mut image.native_source_budget, ctl) {
                            Ok(owner) => builder = Some(owner),
                            Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                            Err(error) => notes.push(format!("Native effect contents could not be prepared: {error}")),
                        }
                    }
                    if let Some(owner) = &mut builder {
                        let mask = (|| -> Result<Option<LayerMask>> {
                            if effect.mask_offset == 0 {
                                Ok(None)
                            } else {
                                let (name, properties, pixels, mask_effects) = image.channel(effect.mask_offset, ctl)?;
                                let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, document.depth, false));
                                write_pixels(&mut surface, &pixels, 0, 0)?;
                                let state = image.convert_coverage_effects(
                                    &mask_effects,
                                    &mut surface,
                                    Rect::new(0, 0, pixels.width as i32, pixels.height as i32),
                                    Some(effect.mask_offset),
                                    ctl,
                                )?;
                                mask_report = Some((name, properties, mask_effects, state));
                                let dx = pending.bounds.x0.checked_neg().ok_or_else(|| invalid("effect mask origin overflow"))?;
                                let dy = pending.bounds.y0.checked_neg().ok_or_else(|| invalid("effect mask origin overflow"))?;
                                surface = photocraft_algo::resample::translate_surface(&surface, dx, dy);
                                Ok(Some(LayerMask { surface, ..LayerMask::reveal_all() }))
                            }
                        })();
                        // GIMP's Replace filter mode becomes the normal native filter mix.
                        let (blend, approximation) = if effect.mode == 62 { (BlendMode::Normal, None) } else { blend_mode(effect.mode as u32) };
                        if let Some(name) = approximation {
                            notes.push(format!("Source stage blend {name} uses the native {blend:?} approximation"));
                        }
                        match mask {
                            Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                            Err(error) => Err(effects::MappingFailure {
                                reason: ConversionReason::InvalidOptionalData,
                                message: format!("Optional effect mask could not be translated; stage not applied: {error}"),
                            }),
                            Ok(mask) => match owner.push(
                                mapping,
                                native_effects::NativeEnvelope { opacity: effect.opacity, blend, visible: effect.visible, mask },
                                &mut image.native_source_budget,
                                ctl,
                            ) {
                                Ok(stage_notes) => {
                                    applied = true;
                                    notes.extend(stage_notes);
                                    Ok(())
                                }
                                Err(IoError::Cancelled) => return Err(IoError::Cancelled),
                                Err(IoError::Unsupported(message)) => Err(effects::MappingFailure::resource(message)),
                                Err(error) => Err(effects::MappingFailure::from(format!("Native structure for this effect is unfinished: {error}"))),
                            },
                        }
                    } else {
                        Err(effects::MappingFailure::from(notes.join("; ")))
                    }
                }
            };
            if let Some((name, properties, pointers, state)) = mask_report {
                let committed = result.is_ok();
                let owner = image.layer_effect_mask_outcome(pending.owner, effect.mask_offset, &name, &properties, committed)?;
                image.report_converted_coverage(owner, &pointers, &state, committed, ctl)?;
            }
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
                    "{} ({}) version {}; visible {}, opacity {}",
                    effect.name,
                    effect.operation,
                    effect.version.unwrap_or("absent"),
                    effect.visible,
                    effect.opacity
                ),
                native_description: native,
                attachments: Vec::new(),
            })?;
            image.outcomes.property_with_reason(outcome, 45, effect.record_offset, effect.raw, reason)?;
            for property in effect.unknown_properties {
                image.outcomes.property(outcome, property.kind, property.offset, property.bytes)?;
            }
        }
        if applied && let Some(builder) = builder {
            let layer = document.layer_mut(pending.layer).ok_or_else(|| invalid("native effect owner disappeared"))?;
            layer.content = builder.content();
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

/// Count generated native filter data without allocating a second JSON copy.
/// Composite branches are ordinary native filters and share this same ceiling.
fn native_parameter_bytes(mapping: &effects::NativeMapping) -> Result<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|n| *n <= MAX_METADATA_BYTES)
                .ok_or_else(|| std::io::Error::other("native effect parameters exceed the metadata allowance"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let mut pending = vec![mapping];
    let mut visited = 0usize;
    while let Some(mapping) = pending.pop() {
        visited = visited.saturating_add(1);
        if visited > 65 {
            return Err(invalid("native effect construction exceeds its flat stage allowance"));
        }
        for filter in &mapping.filters {
            serde_json::to_writer(&mut count, &filter.params).map_err(|e| invalid(e.to_string()))?;
        }
        match &mapping.structure {
            Some(effects::NativeStructure::Composite { branches }) => {
                for branch in branches {
                    for filter in &branch.filters {
                        serde_json::to_writer(&mut count, &filter.params).map_err(|e| invalid(e.to_string()))?;
                    }
                }
            }
            Some(effects::NativeStructure::Stages { steps, output }) => {
                effects::validate_stages(steps, *output).map_err(|e| invalid(e.message))?;
                for step in steps {
                    match step {
                        effects::NativeStage::Apply { mapping, .. } => pending.push(mapping),
                        effects::NativeStage::Displace { metadata_limit, .. } => {
                            count.0 = count
                                .0
                                .checked_add(*metadata_limit)
                                .filter(|n| *n <= MAX_METADATA_BYTES)
                                .ok_or_else(|| invalid("native dynamic maps exceed shared parameter allowance"))?;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(count.0)
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
    let mut remaining = MAX_METADATA_BYTES;
    let mut records = Vec::new();
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
            records.push((ordinal, property.offset, path, None));
            ordinal += 1;
        }
    }
    for &offset in pointers {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut cursor = image.c.at(offset);
        let name = cursor.string()?;
        let length = cursor.u32()? as usize;
        remaining = remaining.checked_sub(length).ok_or_else(|| invalid("saved paths exceed the 32 MiB geometry allowance"))?;
        cursor.take(length)?;
        let raw = image.c.b.get(offset..cursor.pos).ok_or_else(|| invalid("saved path range"))?;
        if records.len() >= MAX_CHANNELS {
            return Err(invalid("more than 1,024 saved paths"));
        }
        match paths::standalone(&image.c, offset) {
            Ok(path) => records.push((ordinal, offset, path, Some(raw))),
            Err(error) => unavailable_path(image, ordinal, offset, &name, &error, Some(raw))?,
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

impl<'a> Image<'a> {
    /// Reads a layer structure, and whether GIMP composites it in linear light.
    fn layer(&mut self, ordinal: usize, ptr: usize, fmt: PixelFormat, mask_fmt: PixelFormat, ctl: &Interrupt<'_>) -> Result<Option<(Node, bool)>> {
        let mut c = self.c.at(ptr);
        let (lw, lh, kind) = (c.u32()?, c.u32()?, c.u32()?);
        let name = c.string()?;
        // Charge the owned name against the document-wide metadata budget: repeated layer
        // offsets referencing one large name must not retain unbounded copies.
        self.meta_budget = self.meta_budget.checked_sub(name.len()).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
        let lprops = props(&mut c)?;
        let hptr = c.ptr()?;
        let mptr = c.ptr()?;
        let effect_pointers = if self.version >= 20 { effects::read_pointers(&mut c, &mut self.meta_budget, ctl)? } else { Vec::new() };
        let (type_rgb, alpha) = match kind {
            0 => (true, false),
            1 => (true, true),
            2 | 4 => (false, false),
            3 | 5 => (false, true),
            k => return Err(invalid(format!("unknown layer type {k} on '{name}'"))),
        };

        let mut layer_mode = 0u32;
        let mut opacity = 1.0f32;
        // PROP_FLOAT_OPACITY wins over PROP_OPACITY whatever their order in the file.
        let mut float_opacity = None;
        let mut visible = true;
        let (mut x, mut y) = (0i32, 0i32);
        let mut apply_mask = true;
        let mut group = false;
        let mut expanded = true;
        let mut path = Vec::new();
        let mut text = false;
        let mut vector = false;
        let mut link = false;
        let mut floating = false;
        let mut legacy_linked = false;
        let mut clipped = false;
        let mut composite_mode = 0i32;
        let mut composite_space = 0i32;
        let mut blend_space = 0i32;
        let mut locks = photocraft_doc::Locks::default();
        let mut tag = LabelColor::None;
        let mut tattoo = None;
        for p in &lprops {
            let v = be_u32(p.data).unwrap_or(0);
            match p.kind {
                5 => floating = true,
                6 => opacity = (v.min(255) as f32) / 255.0,
                33 => float_opacity = be_f32(p.data).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).or(float_opacity),
                7 => layer_mode = v,
                8 => visible = v != 0,
                9 => legacy_linked = v != 0,
                10 => locks.transparency = v != 0,
                11 => apply_mask = v != 0,
                15 => {
                    x = be_i32(p.data).unwrap_or(0);
                    y = p.data.get(4..).and_then(be_i32).unwrap_or(0);
                }
                26 => text = true,
                20 => tattoo = (v != 0).then_some(v),
                28 => locks.pixels = v != 0,
                29 => group = true,
                30 => {
                    path = p.data.as_chunks::<4>().0.iter().map(|s| u32::from_be_bytes(*s)).collect();
                    self.meta_budget =
                        self.meta_budget.checked_sub(path.len().saturating_mul(4)).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
                }
                31 => expanded = v & 1 != 0,
                32 => locks.position = v != 0,
                34 => tag = label(v),
                35 => composite_mode = be_i32(p.data).unwrap_or(0),
                36 => composite_space = be_i32(p.data).unwrap_or(0),
                37 => blend_space = be_i32(p.data).unwrap_or(0),
                47 => vector = true,
                48 => link = true,
                21 => {
                    for (pname, _) in parasites(p.data) {
                        match pname.as_str() {
                            "gimphoto-clipped" => clipped = true,
                            "gimp-text-layer" | "plug_in_gdyntext/data" => text = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let gray_floating = floating && matches!(kind, 2 | 3) && self.base_type != 1;
        if !gray_floating && type_rgb != (self.base_type != 1) && self.base_type != 2 {
            return Err(invalid(format!("layer '{name}' does not match the image's colour model")));
        }

        let (blend, note) = blend_mode(layer_mode);
        if composite_mode.unsigned_abs() == 2 {
            self.clipped_to_backdrop += 1;
        }

        let mut layer = if group {
            let mut g = Layer::group(name.clone(), Vec::new());
            if let LayerContent::Group(gr) = &mut g.content {
                gr.expanded = expanded;
            }
            g.blend = if blend == BlendMode::PassThrough { BlendMode::PassThrough } else { blend };
            g
        } else {
            let mut l = Layer::raster(name.clone(), fmt);
            l.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            l
        };
        layer.opacity = float_opacity.unwrap_or(opacity);
        layer.visible = visible;
        layer.locks = locks;
        layer.label = tag;
        layer.clipped = clipped;
        // Historical XCF stores one boolean movement set, not arbitrary independent groups.
        layer.link_group = legacy_linked.then_some(1);

        let outcome = self.outcomes.push(ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: conversion::source("layer", ordinal, ptr, &name, &path, tattoo),
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), ..Default::default() }],
            disposition: if text || vector || link || self.precision.reduces_precision() {
                ConversionDisposition::NativeApproximation
            } else {
                ConversionDisposition::NativeEquivalent
            },
            reason: ConversionReason::NativePolicy,
            source_description: "Layer structure, offsets, opacity, visibility, locks and stored pixel samples".into(),
            native_description: if group { "Native group and its properties" } else { "Native raster layer and its properties" }.into(),
            attachments: Vec::new(),
        })?;
        if floating {
            let attachment = lprops.iter().rev().find(|p| p.kind == 5).map(|p| resources::attachment(p.data, self.c.wide));
            let (reason, source) = match attachment {
                Some(Ok(pointer)) => (ConversionReason::NativePolicy, format!("Floating pixels; source attachment record {pointer}")),
                _ => (ConversionReason::InvalidOptionalData, "Floating pixels with an invalid optional attachment record".into()),
            };
            self.outcomes.notice(
                outcome,
                "xcf.floating_selection",
                reason,
                (
                    source,
                    if gray_floating {
                        "Separate positioned native raster layer; gray coverage repeated into RGB, attachment retained inert; original channel/mask unchanged"
                    } else {
                        "Separate positioned native raster layer; attachment retained inert, without merging or anchoring"
                    }
                    .into(),
                ),
                Some("is a floating selection and opens as a separate editable layer"),
                &mut self.warnings,
            )?;
        }
        if legacy_linked {
            self.outcomes.detail(
                outcome,
                "xcf.legacy_linked_movement",
                ConversionReason::NativePolicy,
                "Member of the source's single legacy linked movement set".into(),
                "Native link_group 1; ordinary PhotoCraft movement, linking and unlinking apply".into(),
            )?;
        }
        if link {
            let metadata = lprops.iter().rev().find(|p| p.kind == 48).map(|p| resources::link(p.data));
            let (reason, source) = match metadata {
                Some(Ok(metadata)) => (
                    if hptr == 0 { ConversionReason::ResourceUnavailable } else { ConversionReason::NativePolicy },
                    format!(
                        "Link resource {}x{}, flags {}, nonempty locator {} bytes retained inert; {}",
                        metadata.width,
                        metadata.height,
                        metadata.flags,
                        metadata.locator.len(),
                        if hptr == 0 { "no stored pixel hierarchy" } else { "stored pixel hierarchy available" },
                    ),
                ),
                _ => (ConversionReason::InvalidOptionalData, "Invalid optional link-resource configuration retained inert".into()),
            };
            self.outcomes.notice(
                outcome,
                "xcf.link_layer",
                reason,
                (
                    source,
                    if hptr == 0 {
                        "Native raster layer has no stored pixels; source resource is unavailable until explicitly supplied by the user"
                    } else {
                        "Stored pixels preserved as an editable native raster; no locator resolution, automatic refresh or filesystem watch"
                    }
                    .into(),
                ),
                Some(if hptr == 0 {
                    "is a link layer without stored pixels; its source must be supplied explicitly"
                } else {
                    "is a link layer and opens as stored editable pixels"
                }),
                &mut self.warnings,
            )?;
            if let Some(property) = lprops.iter().rev().find(|p| p.kind == 49) {
                let (reason, source) = match resources::transform(property.data) {
                    Ok(transform) => (
                        ConversionReason::NativePolicy,
                        format!(
                            "Source placement offset {:?}, interpolation {}, projective matrix {:?}",
                            transform.offset, transform.interpolation, transform.matrix
                        ),
                    ),
                    Err(_) => (ConversionReason::InvalidOptionalData, "Invalid optional link placement retained inert".into()),
                };
                self.outcomes.detail(outcome, "xcf.link_transform", reason, source,
                    "Stored drawable pixels already contain the source placement; native layer offsets are preserved and the source transform is not applied a second time".into())?;
            }
        }
        // These describe the native translation, not a promise of GIMP formula equivalence.
        if layer_mode != 0 && layer_mode != 28 && !(group && layer_mode == 61) {
            let human = note.map(|note| {
                format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".into() } else { format!("Photoshop's {blend:?}") }
                )
            });
            self.outcomes.notice(
                outcome,
                "xcf.blend_mode",
                if blend == BlendMode::Normal { ConversionReason::MissingMapping } else { ConversionReason::NativePolicy },
                (
                    format!("GIMP mode {layer_mode}{}", note.map(|s| format!(" ({s})")).unwrap_or_default()),
                    format!("Native {:?}; PhotoCraft blend formulas and clamping apply", layer.blend),
                ),
                human.as_deref(),
                &mut self.warnings,
            )?;
        }
        if composite_mode.unsigned_abs() > 1 {
            let human = match composite_mode.unsigned_abs() {
                3 => Some("composites with Clip to layer, which is not reproduced"),
                4 => Some("composites with Intersection, which is not reproduced"),
                _ => None,
            };
            self.outcomes.notice(
                outcome,
                "xcf.composite_mode",
                ConversionReason::NativePolicy,
                (
                    format!(
                        "GIMP coverage mode {composite_mode} ({})",
                        match composite_mode.unsigned_abs() {
                            2 => "Clip to backdrop",
                            3 => "Clip to layer",
                            4 => "Intersection",
                            _ => "unknown",
                        }
                    ),
                    "Native source-over coverage; source coverage does not enable a native clipping mask".into(),
                ),
                human,
                &mut self.warnings,
            )?;
        }
        let linear = composite_space.unsigned_abs() == 1 || blend_space.unsigned_abs() == 1;
        if linear || composite_space.unsigned_abs() > 2 || blend_space.unsigned_abs() > 2 {
            self.outcomes.detail(
                outcome,
                "xcf.compositing_space",
                ConversionReason::NativePolicy,
                format!("GIMP blend space {blend_space}, composite space {composite_space}"),
                "Native document encoding and PhotoCraft compositing rules".into(),
            )?;
        }

        if !group && hptr != 0 {
            let source_base_type = if floating {
                match kind {
                    0 | 1 => 0,
                    2 | 3 => 1,
                    _ => 2,
                }
            } else {
                self.base_type
            };
            let source_channels = match (source_base_type, alpha) {
                (2, false) => 1,
                (2, true) => 2,
                (1, false) => 1,
                (1, true) => 2,
                (_, false) => 3,
                (_, true) => 4,
            };
            let saved_base_type = self.base_type;
            self.base_type = source_base_type;
            let decoded = self.hierarchy(hptr, source_channels, gray_floating, ctl);
            self.base_type = saved_base_type;
            let mut px = decoded.map_err(|error| match error {
                IoError::Cancelled => IoError::Cancelled,
                error => invalid(format!("layer '{name}': {error}")),
            })?;
            if (px.width, px.height) != (lw, lh) {
                return Err(invalid(format!("layer '{name}' pixels are {}x{} where {lw}x{lh} were declared", px.width, px.height)));
            }
            if !alpha {
                // An opaque layer: add the alpha channel the document's pixel format carries.
                let n = px.channels;
                let mut data = Vec::with_capacity(px.data.len() / n * (n + 1));
                for p in px.data.chunks_exact(n) {
                    data.extend_from_slice(p);
                    data.push(1.0);
                }
                px = Pixels { width: px.width, height: px.height, channels: n + 1, data, underflow: px.underflow };
            }
            if gray_floating {
                px = resources::gray_to_rgb(px, &mut self.budget, ctl)?;
            }
            let mut surface = Surface::new(fmt);
            write_pixels(&mut surface, &px, x, y)?;
            self.precision_underflow(outcome, &px)?;
            layer.content = LayerContent::Raster(surface);
            if !alpha {
                layer.locks.transparency = true;
            }
        }
        if vector
            && !group
            && let Some(property) = lprops.iter().rev().find(|p| p.kind == 47)
        {
            match paths::vector_style(property.data) {
                Ok(style) => self.pending_vectors.push((layer.id, outcome, style)),
                Err(error) => self.outcomes.notice(
                    outcome,
                    "xcf.vector_layer",
                    if matches!(error, IoError::Unsupported(_)) { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData },
                    (
                        format!("Source vector style could not be translated: {error}"),
                        "Stored pixels preserved as a native raster; source style remains inert".into(),
                    ),
                    Some("has unsupported or invalid vector paint and opens as pixels"),
                    &mut self.warnings,
                )?,
            }
        }
        if text && !group {
            let modified = lprops.iter().find(|p| p.kind == 26).and_then(|p| be_u32(p.data)).is_some_and(|flags| flags & 2 != 0);
            let config = lprops
                .iter()
                .filter(|p| p.kind == 21)
                .flat_map(|p| parasites(p.data))
                .filter(|(name, _)| matches!(name.as_str(), "gimp-text-layer" | "plug_in_gdyntext/data"))
                .min_by_key(|(name, _)| name != "gimp-text-layer");
            if modified {
                self.outcomes.notice(
                    outcome,
                    "xcf.text",
                    ConversionReason::NativePolicy,
                    (
                        "Source text pixels were modified independently of the saved text configuration".into(),
                        "Native raster preserves modified pixels; original text configuration remains inert".into(),
                    ),
                    Some("has modified text pixels and opens as a native raster layer"),
                    &mut self.warnings,
                )?;
            } else if let Some((config_name, config)) = config {
                let decoded = if config_name == "plug_in_gdyntext/data" {
                    text::decode_gdyntext(&config, self.resolution_dpi, (x, y))
                } else {
                    text::decode(&config, self.resolution_dpi, (x, y))
                };
                match decoded {
                    Ok(decoded) => {
                        layer.content = LayerContent::Text(decoded.layer);
                        self.outcomes.notice(
                            outcome,
                            "xcf.text",
                            ConversionReason::NativePolicy,
                            (
                                "Editable source text, typography and placement".into(),
                                "Native editable text; PhotoCraft font matching, shaping and rasterization apply".into(),
                            ),
                            Some("opens as editable native text with PhotoCraft typography"),
                            &mut self.warnings,
                        )?;
                        for note in decoded.native_policy {
                            self.outcomes.detail(
                                outcome,
                                "xcf.text.formatting",
                                ConversionReason::NativePolicy,
                                note,
                                "Native text placement and editable affine data apply".into(),
                            )?;
                        }
                        for note in decoded.source_only {
                            self.outcomes.detail(
                                outcome,
                                "xcf.text.formatting",
                                ConversionReason::MissingMapping,
                                note,
                                "Supported native typography is editable; additional source formatting remains inert".into(),
                            )?;
                        }
                    }
                    Err(error) => self.outcomes.notice(
                        outcome,
                        "xcf.text",
                        if matches!(error, IoError::Unsupported(_)) { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData },
                        (
                            format!("Source text configuration could not be translated: {error}"),
                            "Stored pixels preserved as a native raster; source configuration remains inert".into(),
                        ),
                        Some("has unsupported or invalid text configuration and opens as pixels"),
                        &mut self.warnings,
                    )?,
                }
            } else {
                self.outcomes.notice(
                    outcome,
                    "xcf.text",
                    ConversionReason::InvalidOptionalData,
                    ("Source text marker has no saved text configuration".into(), "Stored pixels preserved as a native raster".into()),
                    Some("has no saved text configuration and opens as pixels"),
                    &mut self.warnings,
                )?;
            }
        }
        for property in &lprops {
            if property.kind == 21 {
                self.outcomes.parasites(outcome, property.offset, property.data, &["gimphoto-clipped"])?;
            } else if !matches!(property.kind, 6 | 7 | 8 | 9 | 10 | 11 | 15 | 20 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37) {
                let reason = match property.kind {
                    5 => {
                        if resources::attachment(property.data, self.c.wide).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    48 => {
                        if resources::link(property.data).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    49 => {
                        if resources::transform(property.data).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    2 | 12 | 13 => ConversionReason::NativePolicy,
                    42 => ConversionReason::NativeLimitation,
                    _ => ConversionReason::MissingMapping,
                };
                self.outcomes.property_with_reason(outcome, property.kind, property.offset, property.data, reason)?;
            }
        }
        if mptr != 0 {
            let (mname, mprops, px, coverage_effects) = self.channel(mptr, ctl)?;
            let mut surface = Surface::new(mask_fmt);
            write_pixels(&mut surface, &px, x, y)?;
            let mask_outcome = self.outcomes.push(ConversionEntry {
                feature_code: "xcf.layer_mask".into(),
                source: conversion::source("layer_mask", ordinal, mptr, &mname, &path, mprops.iter().find(|p| p.kind == 20).and_then(|p| be_u32(p.data))),
                targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: "Layer mask".into(), ..Default::default() }],
                disposition: if self.precision.reduces_precision() {
                    ConversionDisposition::NativeApproximation
                } else {
                    ConversionDisposition::NativeEquivalent
                },
                reason: ConversionReason::NativePolicy,
                source_description: "Stored mask coverage and enabled state".into(),
                native_description: "Native linked layer mask".into(),
                attachments: Vec::new(),
            })?;
            self.precision_underflow(mask_outcome, &px)?;
            self.translate_coverage_effects(
                mask_outcome,
                &coverage_effects,
                &mut surface,
                Rect::new(x, y, x.saturating_add(px.width as i32), y.saturating_add(px.height as i32)),
                ctl,
            )?;
            layer.mask = Some(LayerMask { surface, enabled: apply_mask, linked: true, density: 1.0, feather: 0.0 });
            for property in &mprops {
                let reason = match property.kind {
                    6 | 8 | 9 | 14 | 16 | 20 | 28 | 32 | 33 | 34 | 38 | 42 => ConversionReason::NativeLimitation,
                    _ => ConversionReason::MissingMapping,
                };
                self.outcomes.property_with_reason(mask_outcome, property.kind, property.offset, property.data, reason)?;
            }
        }
        if metadata::active(&lprops, 2) {
            self.initial_view.selected_layers.push(layer.id);
            if self.initial_view.active_layer.is_none() {
                self.initial_view.active_layer = Some(layer.id);
                self.initial_view.show_layer_mask = layer.mask.is_some() && metadata::flag(&lprops, 13);
                if layer.mask.is_some() && metadata::flag(&lprops, 12) {
                    self.initial_view.target = crate::initial_view::NativeImportTarget::LayerMask;
                }
            }
        }
        if !effect_pointers.is_empty() {
            let bounds = x.checked_add(lw as i32).zip(y.checked_add(lh as i32)).map_or(Rect::EMPTY, |(right, bottom)| Rect::new(x, y, right, bottom));
            self.pending_effects.push(PendingLayerEffects { layer: layer.id, owner: outcome, bounds, pointers: effect_pointers });
        }
        Ok(Some((Node { layer, path, group, children: Vec::new(), outcome }, linear)))
    }
}
