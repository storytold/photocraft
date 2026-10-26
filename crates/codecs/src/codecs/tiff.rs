//! TIFF and BigTIFF: 1/2/4/8/16/32/64-bit integer and 16/32/64-bit float samples, gray / RGB /
//! CMYK / palette with optional alpha (unassociated, or associated and un-premultiplied) and
//! extra samples, WhiteIsZero, strips or tiles, planar configuration 1 or 2, ICC (34675), XMP
//! (700), DPI and a few ASCII text tags.
//!
//! **Structure.** [`super::tiff_ifd`] walks the directories with bounds checks (classic and
//! BigTIFF, either byte order, the whole IFD chain and its SubIFDs, cycles and hostile counts
//! defeated) and lists them as pages. Decoding opens the first full-resolution page, like
//! Photoshop (reduced-resolution copies and masks are skipped); further pages are reported as a
//! [`DecodeWarning::MorePages`], and [`decode_page`] opens any directory.
//!
//! **Pixels.** The `tiff` crate decompresses one strip or tile at a time (none, LZW, Deflate,
//! PackBits, with the horizontal and floating-point predictors). The decoder sees the chosen
//! directory through a byte view whose header points at it ([`Patched`]), so SubIFDs decode
//! like IFD0. Decoding is banded: the output image is allocated once, at its final size, and
//! each band of rows (one strip, or one row of tiles, across every plane) is decoded and
//! converted straight into it, in parallel on native targets. A strip that already has the
//! output's layout is decoded in place; anything else goes through one strip- or tile-sized
//! scratch buffer per worker, so no second full-size buffer exists. Image data cut off by the
//! end of the file leaves the missing chunks empty and adds a [`DecodeWarning::Truncated`].
//!
//! Photoshop's private tags are carried as opaque bytes in [`Metadata`]: 34377 (image
//! resources) and 37724 (`ImageSourceData`, the layers of a layered TIFF). They are read with
//! a bounds-checked walk of the first directory ([`photoshop_tags`]) rather than through the
//! decoder, which would materialize a multi-megabyte layer block as one `Value` per byte, and
//! written back as BYTE / UNDEFINED tags. The encoder writes files in the native byte order
//! (see [`writes_little_endian`]), which the layer data must match.

use std::borrow::Cow;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use half::f16;
use tiff::decoder::{ChunkType, Decoder};
use tiff::encoder::colortype::{self, ColorType as TiffColorType};
use tiff::encoder::{Rational, TiffEncoder, TiffKind, TiffKindBig, TiffKindStandard, TiffValue};
use tiff::tags::{PhotometricInterpretation, ResolutionUnit, SampleFormat, Tag, Type};

use super::tiff_exif::{self, ExifDirs};
use super::tiff_ifd::{self as ifd, File, Ifd};
use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits, TiffCompression};

const F: Format = Format::Tiff;
const TAG_XMP: u16 = 700;
/// Photoshop image resources (`8BIM` blocks).
const TAG_PHOTOSHOP: u16 = 34377;
/// Photoshop `ImageSourceData`: the layers of a layered TIFF.
const TAG_IMAGE_SOURCE_DATA: u16 = 37724;

/// `true` when [`encode`] writes little-endian (`II`) files: the `tiff` encoder writes the
/// running target's byte order, and every target PhotoCraft builds for is little-endian.
/// Byte-order-sensitive payloads (Photoshop layer data) must be produced to match.
pub fn writes_little_endian() -> bool {
    cfg!(target_endian = "little")
}

/// `(34377 image resources, 37724 image source data)` payloads, each absent when the tag is.
pub type PhotoshopTags<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

/// The raw payloads of the Photoshop private tags in the first directory: `(34377 image
/// resources, 37724 image source data)`. Classic and BigTIFF, either byte order; only
/// header bytes and the two payloads are touched, and nothing is allocated. Anything malformed
/// reads as "absent".
pub fn photoshop_tags(b: &[u8]) -> PhotoshopTags<'_> {
    fn walk(b: &[u8]) -> Option<PhotoshopTags<'_>> {
        let le = match b.get(0..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let u16_at = |o: u64| -> Option<u16> {
            let o = usize::try_from(o).ok()?;
            let s = b.get(o..o.checked_add(2)?)?;
            Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
        };
        let u32_at = |o: u64| -> Option<u32> {
            let o = usize::try_from(o).ok()?;
            let s: [u8; 4] = b.get(o..o.checked_add(4)?)?.try_into().ok()?;
            Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
        };
        let u64_at = |o: u64| -> Option<u64> {
            let o = usize::try_from(o).ok()?;
            let s: [u8; 8] = b.get(o..o.checked_add(8)?)?.try_into().ok()?;
            Some(if le { u64::from_le_bytes(s) } else { u64::from_be_bytes(s) })
        };
        let (big, ifd) = match u16_at(2)? {
            42 => (false, u64::from(u32_at(4)?)),
            43 => (true, u64_at(8)?),
            _ => return None,
        };
        let n = if big { u64_at(ifd)? } else { u64::from(u16_at(ifd)?) }.min(4096);
        let (entry, first, inline) = if big { (20u64, ifd.checked_add(8)?, 8u64) } else { (12, ifd.checked_add(2)?, 4) };
        let mut resources = None;
        let mut layers = None;
        for i in 0..n {
            let e = first.checked_add(i.checked_mul(entry)?)?;
            let tag = u16_at(e)?;
            if tag != TAG_PHOTOSHOP && tag != TAG_IMAGE_SOURCE_DATA {
                continue;
            }
            // BYTE, ASCII, SBYTE or UNDEFINED: one byte per element.
            if !matches!(u16_at(e + 2)?, 1 | 2 | 6 | 7) {
                continue;
            }
            let count = if big { u64_at(e + 4)? } else { u64::from(u32_at(e + 4)?) };
            let value_at = e + if big { 12 } else { 8 };
            let start = if count <= inline {
                value_at
            } else if big {
                u64_at(value_at)?
            } else {
                u64::from(u32_at(value_at)?)
            };
            let start = usize::try_from(start).ok()?;
            let data = b.get(start..start.checked_add(usize::try_from(count).ok()?)?)?;
            if tag == TAG_PHOTOSHOP {
                resources = Some(data);
            } else {
                layers = Some(data);
            }
        }
        Some((resources, layers))
    }
    walk(b).unwrap_or((None, None))
}

/// ASCII tags mapped to `Metadata::text` keys.
const TEXT_TAGS: &[(Tag, &str)] = &[
    (Tag::ImageDescription, "Description"),
    (Tag::Make, "Make"),
    (Tag::Model, "Model"),
    (Tag::Software, "Software"),
    (Tag::DateTime, "DateTime"),
    (Tag::Artist, "Artist"),
    (Tag::Copyright, "Copyright"),
];

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

fn map_err(e: tiff::TiffError) -> CodecError {
    match e {
        tiff::TiffError::LimitsExceeded => CodecError::LimitExceeded("TIFF decoder limit".into()),
        tiff::TiffError::UnsupportedError(u) => CodecError::unsupported(F, u.to_string()),
        e => err(e),
    }
}

/// TIFF/EP `PhotometricInterpretation` for colour filter array (sensor) data.
const PHOTOMETRIC_CFA: u32 = 32803;
/// DNG `PhotometricInterpretation` for demosaiced but undeveloped sensor data.
const PHOTOMETRIC_LINEAR_RAW: u32 = 34892;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_SUB_IFDS: u16 = 330;
const TAG_DNG_VERSION: u16 = 50706;

/// Recognizes TIFF-structured camera raw files (TIFF/EP, DNG, CR2 and the
/// TIFF-based NEF/ARW/PEF… layouts): a CR2 signature, a DNGVersion tag, or
/// a CFA / LinearRaw image in IFD0, its SubIFDs or the next few IFDs. Their
/// first IFD is often a reduced preview, so this runs before decoding.
/// Reads only bounds-checked header bytes; anything malformed is "not raw".
fn camera_raw(b: &[u8]) -> bool {
    let le = match b.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return false, // BigTIFF and others: no raw layouts to recognize
    };
    let u16_at = |o: usize| {
        let s = b.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let u32_at = |o: usize| {
        let s: [u8; 4] = b.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    // CR2: "CR" and major version 2 right after the TIFF header.
    if b.get(8..11) == Some(b"CR\x02") {
        return true;
    }
    // Entry value (count 1) as SHORT or LONG; anything else is ignored.
    let value = |e: usize| match u16_at(e.saturating_add(2))? {
        3 => u16_at(e.saturating_add(8)).map(u32::from),
        4 | 13 => u32_at(e.saturating_add(8)),
        _ => None,
    };
    let mut queue: Vec<u32> = u32_at(4).into_iter().collect();
    let mut visited = 0;
    while let Some(ifd) = queue.pop() {
        // A handful of IFDs is enough for every real layout; also stops loops.
        visited += 1;
        if visited > 16 {
            break;
        }
        let ifd = ifd as usize;
        let Some(n) = u16_at(ifd) else { continue };
        for i in 0..usize::from(n) {
            let e = ifd.saturating_add(2 + 12 * i);
            let (Some(tag), Some(count)) = (u16_at(e), u32_at(e.saturating_add(4))) else {
                break;
            };
            match tag {
                TAG_DNG_VERSION => return true,
                TAG_PHOTOMETRIC if matches!(value(e), Some(PHOTOMETRIC_CFA | PHOTOMETRIC_LINEAR_RAW)) => {
                    return true;
                }
                TAG_SUB_IFDS if count == 1 => queue.extend(value(e)),
                TAG_SUB_IFDS => {
                    // More than one offset: they are stored at the entry's offset.
                    let at = u32_at(e.saturating_add(8)).unwrap_or(0) as usize;
                    for k in 0..count.min(8) as usize {
                        queue.extend(u32_at(at.saturating_add(4 * k)));
                    }
                }
                _ => {}
            }
        }
        if let Some(next) = u32_at(ifd.saturating_add(2 + 12 * usize::from(n))).filter(|&o| o != 0) {
            queue.push(next);
        }
    }
    false
}

/// Decodes the page Photoshop would open: the first full-resolution directory.
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    decode_page(bytes, None, limits)
}

/// Decodes page `page` of [`ifd::tiff_info`]'s list (`None`: the default page). The image
/// carries the page's metadata, a [`DecodeWarning::MorePages`] when the file holds more
/// full-resolution pages, and a [`DecodeWarning::Truncated`] when image data is cut off.
pub(crate) fn decode_page(bytes: &[u8], page: Option<usize>, limits: &Limits) -> Result<Image, CodecError> {
    if camera_raw(bytes) {
        return Err(CodecError::unsupported(F, "camera raw files (such as CR2, NEF, ARW or DNG) are not flat images; decode them with photocraft-raw"));
    }
    let file = File::parse(bytes).ok_or_else(|| err("not a TIFF or BigTIFF header"))?;
    let info = ifd::info_of(&file)?;
    let index = match page {
        Some(i) => i,
        None => info.default_page().ok_or_else(|| err("no image directory"))?,
    };
    let p = info
        .pages
        .get(index)
        .ok_or_else(|| CodecError::InvalidImage(format!("TIFF page {index} does not exist (the file has {} directories)", info.pages.len())))?;
    let dir = file.ifd(p.ifd_offset).ok_or_else(|| err("image directory cut off"))?;
    let mut img = decode_ifd(&file, &dir, limits)?;
    let extra_samples = std::mem::take(&mut img.meta.extra_samples);
    img.meta = metadata(&file, &dir, &mut img.icc);
    img.meta.extra_samples = extra_samples;
    let pages = info.page_count();
    if pages > 1 {
        img.warnings.push(DecodeWarning::MorePages { total: info.complete.then_some(u32::try_from(pages).unwrap_or(u32::MAX)) });
    }
    Ok(img)
}

/// The Orientation (1–8) of page `page` (`None`: the default page); 1 when absent, malformed
/// or out of range. Classic TIFF and BigTIFF.
pub(crate) fn orientation(bytes: &[u8], page: Option<usize>) -> u16 {
    let Some(file) = File::parse(bytes) else { return 1 };
    let Ok(info) = ifd::info_of(&file) else { return 1 };
    page.or_else(|| info.default_page()).and_then(|i| info.pages.get(i)).and_then(|p| file.ifd(p.ifd_offset)).map_or(1, |d| ifd::orientation_of(&file, &d))
}

/// The page's metadata; the ICC profile goes to `icc`.
fn metadata(f: &File<'_>, dir: &Ifd, icc: &mut Option<Vec<u8>>) -> Metadata {
    *icc = f.tag_bytes(dir, Tag::IccProfile.to_u16()).filter(|b| !b.is_empty()).map(<[u8]>::to_vec);
    let text = |tag: u16| {
        let b = f.tag_bytes(dir, tag)?;
        let s = std::str::from_utf8(b).ok()?.trim_end_matches('\0');
        (!s.is_empty()).then(|| s.to_owned())
    };
    let mut meta = Metadata { xmp: text(TAG_XMP), ..Default::default() };
    let unit = f.tag_uint(dir, Tag::ResolutionUnit.to_u16()).unwrap_or(2);
    if let (Some(x), Some(y)) = (f.tag_f64(dir, Tag::XResolution.to_u16()), f.tag_f64(dir, Tag::YResolution.to_u16()))
        && x > 0.0
        && y > 0.0
        && x.is_finite()
        && y.is_finite()
    {
        meta.dpi = match unit {
            2 => Some((x as f32, y as f32)),
            3 => Some(((x * 2.54) as f32, (y * 2.54) as f32)),
            _ => None,
        };
    }
    for (tag, key) in TEXT_TAGS {
        if let Some(s) = text(tag.to_u16()) {
            meta.text.push(((*key).to_owned(), s));
        }
    }
    // Photoshop keeps a layered TIFF's layers and resources in IFD0, where `photoshop_tags`
    // reads them; any other page reads its own directory's copies.
    let (resources, layers) =
        if dir.at == f.first_ifd { photoshop_tags(f.b) } else { (f.tag_bytes(dir, TAG_PHOTOSHOP), f.tag_bytes(dir, TAG_IMAGE_SOURCE_DATA)) };
    meta.photoshop_resources = resources.map(<[u8]>::to_vec);
    meta.photoshop_layers = layers.map(<[u8]>::to_vec);
    // EXIF: the Exif and GPS directories IFD0 points to, with IFD0's descriptive tags, as an
    // EXIF block (the other pages are reduced copies or layers of the same image).
    if dir.at == f.first_ifd
        && let Some(e) = tiff_exif::read(f.b).filter(ExifDirs::has_directories)
    {
        meta.exif = Some(tiff_exif::block(&e));
    }
    meta
}

// ---------------------------------------------------------------------------
// Banded pixel decoding
// ---------------------------------------------------------------------------

/// The file seen with a few byte ranges replaced: the header's first-IFD offset (so the `tiff`
/// decoder opens the chosen directory as its IFD0) and, for WhiteIsZero and palette images,
/// the PhotometricInterpretation value (so the decoder hands over the stored samples and this
/// module applies the interpretation itself). Reads past the end return no bytes.
#[derive(Clone, Copy)]
struct Patched<'a> {
    b: &'a [u8],
    patches: &'a [(u64, Vec<u8>)],
    pos: u64,
}

impl Read for Patched<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(src) = usize::try_from(self.pos).ok().and_then(|p| self.b.get(p..)) else { return Ok(0) };
        let n = out.len().min(src.len());
        let (Some(o), Some(s)) = (out.get_mut(..n), src.get(..n)) else { return Ok(0) };
        o.copy_from_slice(s);
        let end = self.pos.saturating_add(n as u64);
        for (at, bytes) in self.patches {
            let lo = (*at).max(self.pos);
            let hi = at.saturating_add(bytes.len() as u64).min(end);
            for i in lo..hi {
                let (Ok(oi), Ok(pi)) = (usize::try_from(i - self.pos), usize::try_from(i - at)) else { continue };
                if let (Some(d), Some(&v)) = (o.get_mut(oi), bytes.get(pi)) {
                    *d = v;
                }
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Patched<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => (self.b.len() as u64).checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let new = new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        self.pos = new;
        Ok(new)
    }
}

/// How a stored sample is laid out in the `tiff` crate's chunk buffer (native endian).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Src {
    /// 1, 2 or 4-bit unsigned, packed MSB first, rows padded to a byte.
    Bits(u8),
    U8,
    U16,
    U32,
    U64,
    F16,
    F32,
    F64,
}

impl Src {
    fn bytes(self) -> usize {
        match self {
            Src::Bits(_) | Src::U8 => 1,
            Src::U16 | Src::F16 => 2,
            Src::U32 | Src::F32 => 4,
            Src::U64 | Src::F64 => 8,
        }
    }

    /// The output sample type: integers above 16 bits keep their top 16, doubles become f32.
    fn out(self) -> SampleType {
        match self {
            Src::Bits(_) | Src::U8 => SampleType::U8,
            Src::U16 | Src::U32 | Src::U64 => SampleType::U16,
            Src::F16 => SampleType::F16,
            Src::F32 | Src::F64 => SampleType::F32,
        }
    }

    /// Stored bytes and output bytes are the same: a plain copy.
    fn is_identity(self) -> bool {
        matches!(self, Src::U8 | Src::U16 | Src::F16 | Src::F32)
    }
}

/// Everything needed to decode and place the chunks of one directory.
struct Layout {
    w: usize,
    src: Src,
    out: SampleType,
    out_layout: ChannelLayout,
    /// Samples per pixel in a decoded chunk (1 for planar data).
    chunk_spp: usize,
    planar: bool,
    /// Planes decoded for planar data: one per output channel (extra planes are skipped).
    planes: usize,
    /// Tile width (a strip is `w` wide).
    cw: usize,
    across: usize,
    chunks_per_plane: usize,
    /// WhiteIsZero: invert the gray channel.
    invert: bool,
    /// The decoded sample (chunky) or file plane (planar) of the alpha channel (when there is one).
    alpha_sample: usize,
    /// Associated (premultiplied) alpha to turn straight.
    premultiplied: bool,
    /// Palette images: the colour map (16-bit RGB per index).
    palette: Option<Vec<[u16; 3]>>,
    /// Upper bound for one chunk's scratch buffer.
    scratch_cap: usize,
    /// File sample indices that are neither colour nor the chosen alpha (spot / extra alpha).
    extra_at: Vec<usize>,
}

impl Layout {
    fn out_channels(&self) -> usize {
        self.out_layout.channels()
    }
    /// The decoded sample (chunky) or file plane (planar) of output channel `k`.
    fn source_sample(&self, k: usize) -> usize {
        if self.out_layout.has_alpha() && k + 1 == self.out_channels() { self.alpha_sample } else { k }
    }
    fn out_row(&self) -> usize {
        self.w * self.out_channels() * self.out.bytes()
    }
    fn extra_row(&self) -> usize {
        self.w.saturating_mul(self.extra_at.len()).saturating_mul(self.out.bytes())
    }
    /// A chunky strip already in the output's layout: decoded in place.
    fn direct(&self) -> bool {
        self.copyable() && self.across == 1
    }
    /// Chunk rows are output rows: copied without conversion.
    fn copyable(&self) -> bool {
        !self.planar && self.palette.is_none() && self.src.is_identity() && self.chunk_spp == self.out_channels()
    }
}

/// A chunk read that hit the end of the file (the data is missing, not malformed).
fn is_eof(e: &tiff::TiffError) -> bool {
    matches!(e, tiff::TiffError::IoError(io) if io.kind() == io::ErrorKind::UnexpectedEof)
}

/// Decodes the pixels of one directory.
fn decode_ifd(f: &File<'_>, dir: &Ifd, limits: &Limits) -> Result<Image, CodecError> {
    // The values the `tiff` crate reads must exist: a count past the end of the file is
    // neither allocated nor read.
    if let Some(e) = dir.entries.iter().find(|e| ifd::STRUCTURAL_TAGS.contains(&e.tag) && f.data(e).is_none()) {
        return Err(err(format!("tag {} points past the end of the file", e.tag)));
    }
    let need = |tag: u16, name: &str| f.tag_uint(dir, tag).ok_or_else(|| err(format!("missing or unreadable {name}")));
    let w = u32::try_from(need(ifd::TAG_WIDTH, "ImageWidth")?).map_err(|_| err("ImageWidth out of range"))?;
    let h = u32::try_from(need(ifd::TAG_HEIGHT, "ImageLength")?).map_err(|_| err("ImageLength out of range"))?;
    if w == 0 || h == 0 {
        return Err(CodecError::InvalidImage(format!("zero-sized image {w}x{h}")));
    }
    let spp = f.tag_uint(dir, ifd::TAG_SAMPLES).unwrap_or(1);
    if spp == 0 || spp > 64 {
        return Err(CodecError::unsupported(F, format!("{spp} samples per pixel")));
    }
    let bits_all = f.tag_uints(dir, ifd::TAG_BITS, spp).unwrap_or_else(|| vec![1]);
    let bits = bits_all.first().copied().unwrap_or(1);
    if bits_all.iter().any(|&b| b != bits) {
        return Err(CodecError::unsupported(F, format!("mixed bits per sample {bits_all:?}")));
    }
    let format = f.tag_uint(dir, ifd::TAG_SAMPLE_FORMAT).unwrap_or(1);
    let photometric_entry = dir.get(ifd::TAG_PHOTOMETRIC).copied();
    let photometric = f.tag_uint(dir, ifd::TAG_PHOTOMETRIC).ok_or_else(|| CodecError::unsupported(F, "missing PhotometricInterpretation"))?;
    let planar = f.tag_uint(dir, ifd::TAG_PLANAR) == Some(2);
    let extra = f.tag_uints(dir, ifd::TAG_EXTRA_SAMPLES, 64).unwrap_or_default();
    let tiled = dir.get(ifd::TAG_TILE_OFFSETS).is_some();
    let (cw, ch) = if tiled {
        let tw = need(ifd::TAG_TILE_WIDTH, "TileWidth")?;
        let tl = need(ifd::TAG_TILE_LENGTH, "TileLength")?;
        (tw, tl)
    } else {
        (u64::from(w), f.tag_uint(dir, ifd::TAG_ROWS_PER_STRIP).unwrap_or(u64::from(h)).min(u64::from(h)))
    };
    if cw == 0 || ch == 0 {
        return Err(err("zero tile or strip size"));
    }
    // The chunk tables must match the geometry. Checked here, in 64 bits, before the `tiff`
    // crate does the same arithmetic in narrower types.
    let planes_in_file = if planar { spp } else { 1 };
    let expected = u64::from(w)
        .div_ceil(cw)
        .checked_mul(u64::from(h).div_ceil(ch))
        .and_then(|n| n.checked_mul(planes_in_file))
        .filter(|&n| n <= u64::from(u32::MAX))
        .ok_or_else(|| err("too many strips or tiles"))?;
    let (offsets_tag, counts_tag) = if tiled { (324, 325) } else { (273, 279) };
    let counts = (dir.get(offsets_tag).map(|e| e.count), dir.get(counts_tag).map(|e| e.count));
    if counts != (Some(expected), Some(expected)) {
        return Err(err(format!("{expected} strips or tiles expected, the tables list {counts:?}")));
    }
    let src = match (format, bits) {
        (1, 1 | 2 | 4) => Src::Bits(bits as u8),
        (1, 8) => Src::U8,
        (1, 16) => Src::U16,
        (1, 32) => Src::U32,
        (1, 64) => Src::U64,
        (3, 16) => Src::F16,
        (3, 32) => Src::F32,
        (3, 64) => Src::F64,
        (2, _) => return Err(CodecError::unsupported(F, "signed integer samples")),
        _ => return Err(CodecError::unsupported(F, format!("{bits}-bit samples (sample format {format})"))),
    };

    // WhiteIsZero and palette images are handed to the `tiff` crate as BlackIsZero: it then
    // returns the stored samples, and the interpretation is applied here (for every depth and
    // with alpha, which the crate's own inversion does not cover).
    let invert = photometric == 0;
    let palette = if photometric == 3 {
        if spp != 1 || !matches!(src, Src::Bits(_) | Src::U8 | Src::U16) {
            return Err(CodecError::unsupported(F, format!("palette image with {spp} samples of {bits} bits")));
        }
        let n = 1u64 << bits;
        let map = dir.get(ifd::TAG_COLOR_MAP).filter(|e| e.ty == 3 && e.count == 3 * n).ok_or_else(|| err("palette image without a valid ColorMap"))?;
        let at = |i: u64| f.uint(map, i).and_then(|v| u16::try_from(v).ok());
        let table: Option<Vec<[u16; 3]>> = (0..n).map(|i| Some([at(i)?, at(n + i)?, at(2 * n + i)?])).collect();
        Some(table.ok_or_else(|| err("palette ColorMap cut off"))?)
    } else {
        None
    };
    // CIE L*a*b*: CIELab (8; a* and b* signed) and ICCLab (9; a* and b* offset) are handed to the
    // `tiff` crate as RGB, then labelled Lab with a* and b* in PhotoCraft's encoding
    // (`convert_ab`).
    let lab = matches!(photometric, 8 | 9);
    if lab && (spp < 3 || !matches!(src, Src::U8 | Src::U16)) {
        return Err(CodecError::unsupported(F, format!("Lab image with {spp} samples of {bits} bits")));
    }
    // TIFF 6.0 lets any extra sample be the alpha (#1349). For RGB and CMYK the `tiff` crate
    // returns the alpha only when it is the first extra sample: otherwise it is asked for every
    // sample (as multiband gray) and the colour and alpha samples are picked here. An alpha
    // position past the samples the file has is ignored.
    let color: usize = match photometric {
        2 | 8 | 9 => 3,
        5 => 4,
        _ => 1,
    };
    let alpha_at = extra.iter().position(|&e| e == 1 || e == 2).filter(|&a| (color as u64).saturating_add(a as u64) < spp);
    // Samples the colour model does not keep. RGB, CMYK and Lab only hand back every sample when
    // asked (photometric patched to black-is-zero below); gray already comes back as multiband.
    let unused = !extra.is_empty() && (spp as usize) > color.saturating_add(usize::from(alpha_at.is_some()));
    let all_samples = palette.is_none() && matches!(photometric, 2 | 5 | 8 | 9) && (alpha_at.is_some_and(|a| a > 0) || (!planar && unused));
    let mut patches: Vec<(u64, Vec<u8>)> = Vec::new();
    let encode_u = |v: u64, len: usize| -> Vec<u8> {
        let b = if f.le { v.to_le_bytes() } else { v.to_be_bytes() };
        if f.le { b.get(..len).map(<[u8]>::to_vec).unwrap_or_default() } else { b.get(8 - len..).map(<[u8]>::to_vec).unwrap_or_default() }
    };
    if f.big {
        patches.push((8, encode_u(dir.at, 8)));
    } else {
        let at = u32::try_from(dir.at).map_err(|_| err("directory offset out of range"))?;
        patches.push((4, encode_u(u64::from(at), 4)));
    }
    // The directory is seen as the last of its chain: the chain was walked already, and a
    // cycle through it must not fail this page.
    patches.push((dir.next_at, vec![0; if f.big { 8 } else { 4 }]));
    if invert || palette.is_some() || all_samples || lab {
        let e = photometric_entry.ok_or_else(|| err("missing PhotometricInterpretation"))?;
        let len = match e.ty {
            3 if e.count == 1 => 2,
            4 if e.count == 1 => 4,
            _ => return Err(err("malformed PhotometricInterpretation")),
        };
        patches.push((e.value_at, encode_u(if lab && !all_samples { 2 } else { 1 }, len)));
    }

    let tl = {
        let mut l = tiff::decoder::Limits::default();
        l.decoding_buffer_size = limits.alloc_usize();
        // No compressed chunk can be larger than the file.
        l.intermediate_buffer_size = f.b.len().max(1);
        l
    };
    let open = || Decoder::new(Patched { b: f.b, patches: &patches, pos: 0 }).map(|d| d.with_limits(tl.clone())).map_err(map_err);
    let mut dec = open()?;
    if dec.dimensions().map_err(map_err)? != (w, h) || (dec.get_chunk_type() == ChunkType::Tile) != tiled {
        return Err(err("inconsistent image directory"));
    }
    let ct = dec.colortype().map_err(map_err)?;
    let gray_alpha = alpha_at.is_some() || (extra.is_empty() && spp == 2);
    let (out_layout, chunk_spp) = match ct {
        _ if palette.is_some() => (ChannelLayout::Rgb, 1),
        tiff::ColorType::Gray(_) => (ChannelLayout::Gray, 1),
        tiff::ColorType::Multiband { num_samples, .. } if all_samples => {
            // Lab reads as RGB here and is relabelled below.
            let layout = match photometric {
                2 | 8 | 9 => {
                    if alpha_at.is_some() {
                        ChannelLayout::Rgba
                    } else {
                        ChannelLayout::Rgb
                    }
                }
                5 => {
                    if alpha_at.is_some() {
                        ChannelLayout::CmykA
                    } else {
                        ChannelLayout::Cmyk
                    }
                }
                _ => {
                    if gray_alpha {
                        ChannelLayout::GrayA
                    } else {
                        ChannelLayout::Gray
                    }
                }
            };
            (layout, usize::from(num_samples))
        }
        tiff::ColorType::Multiband { num_samples, .. } if photometric <= 1 => {
            (if gray_alpha { ChannelLayout::GrayA } else { ChannelLayout::Gray }, usize::from(num_samples))
        }
        tiff::ColorType::RGB(_) => (ChannelLayout::Rgb, 3),
        tiff::ColorType::RGBA(_) => (ChannelLayout::Rgba, 4),
        tiff::ColorType::CMYK(_) => (ChannelLayout::Cmyk, 4),
        tiff::ColorType::CMYKA(_) => (ChannelLayout::CmykA, 5),
        other => return Err(CodecError::unsupported(F, format!("colour type {other:?}"))),
    };
    let out_layout = match out_layout {
        ChannelLayout::Rgb if lab => ChannelLayout::Lab,
        ChannelLayout::Rgba if lab => ChannelLayout::LabA,
        _ if lab => return Err(err("Lab image that does not read as three colour samples")),
        l => l,
    };
    let chunk_spp = if planar { 1 } else { chunk_spp };
    // The sample (chunky) or plane (planar) that holds the alpha: the colour samples come first.
    let alpha_sample = out_layout.color_channels() + alpha_at.unwrap_or(0);
    if out_layout.has_alpha() && (alpha_sample as u64 >= spp || (!planar && alpha_sample >= chunk_spp)) {
        return Err(err("the alpha sample is past the samples of a pixel"));
    }
    if matches!(src, Src::Bits(_)) && (out_layout.channels() > 1 || spp != 1) && palette.is_none() {
        return Err(CodecError::unsupported(F, format!("{bits}-bit samples with {spp} samples per pixel")));
    }
    if (!planar && palette.is_none() && chunk_spp < out_layout.channels()) || (planar && out_layout.channels() as u64 > spp) {
        return Err(err("fewer samples than the colour type needs"));
    }
    let out = match (&palette, src) {
        (Some(_), Src::U16) => SampleType::U16,
        (Some(_), _) => SampleType::U8,
        _ => src.out(),
    };
    limits.check(w, h, out_layout, out)?;
    // Decompression-bomb guard: no compression method expands data by more than a few thousand
    // times (Deflate at most ~1032:1, LZW ~2730:1, PackBits 128:1), so a file far too small for
    // its declared size is refused before the image is allocated. Uncompressed data may be up
    // to three quarters missing (that decodes with a `Truncated` warning).
    let stored = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(spp).saturating_mul(bits).div_ceil(8);
    let ratio: u64 = match f.tag_uint(dir, ifd::TAG_COMPRESSION).unwrap_or(1) {
        1 => 4,
        32773 => 130,
        _ => 4096,
    };
    if stored > (f.b.len() as u64).saturating_mul(ratio).saturating_add(1 << 20) {
        return Err(err(format!("a {} byte file cannot hold a {w}x{h} image", f.b.len())));
    }

    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (usize::try_from(cw).map_err(|_| err("tile too wide"))?, usize::try_from(ch).map_err(|_| err("tile too tall"))?);
    let across = wu.div_ceil(cw);
    let down = hu.div_ceil(ch);
    let chunks_per_plane = across.checked_mul(down).ok_or_else(|| err("too many tiles"))?;
    let planes_in_file = if planar { spp as usize } else { 1 };
    let total = if tiled { dec.tile_count() } else { dec.strip_count() }.map_err(map_err)?;
    if Some(total as usize) != chunks_per_plane.checked_mul(planes_in_file) {
        return Err(err("strip or tile count does not match the image size"));
    }
    // One chunk's decoded bytes, clipped to the image: the scratch buffer may hold a little
    // more (the `tiff` crate reads whole tiles in some planar cases) but never an amount that
    // only a hostile tile size could ask for.
    let chunk_bytes = cw
        .min(wu)
        .checked_mul(ch.min(hu))
        .and_then(|px| px.checked_mul(chunk_spp))
        .and_then(|s| s.checked_mul(src.bytes()))
        .ok_or_else(|| CodecError::LimitExceeded("tile size".into()))?;
    let scratch_cap = chunk_bytes.saturating_mul(2).max(64 << 20).min(limits.alloc_usize());
    let (extra_at, extras_over_limit) = extra_sample_indices(spp as usize, color, alpha_at, out_layout.has_alpha());
    let mut lay = Layout {
        w: wu,
        src,
        out,
        out_layout,
        chunk_spp,
        planar,
        planes: if planar { out_layout.channels() } else { 1 },
        cw,
        across,
        chunks_per_plane,
        invert: invert && palette.is_none(),
        alpha_sample,
        premultiplied: out_layout.has_alpha() && alpha_at.and_then(|a| extra.get(a)) == Some(&1),
        palette,
        scratch_cap,
        extra_at,
    };
    // Extra samples share the allocation budget with the image: their interleaved buffer and the
    // planes split from it are held next to the image, so both count against `max_alloc`. When
    // they don't fit, the image still opens (it passed its own check above) without them.
    let mut extras_dropped = 0;
    if !lay.extra_at.is_empty() {
        let per_pixel = out_layout.channels().saturating_add(lay.extra_at.len().saturating_mul(2)).saturating_mul(out.bytes());
        if limits.check_bytes(w, h, per_pixel as u64).is_err() {
            extras_dropped = u32::try_from(lay.extra_at.len()).unwrap_or(u32::MAX);
            lay.extra_at.clear();
        }
    }

    // The output, allocated once and fallibly.
    let out_row = lay.out_row();
    let len = out_row.checked_mul(hu).ok_or_else(|| CodecError::LimitExceeded("image size".into()))?;
    let mut data = zeroed(len)?;
    let band_bytes = out_row.checked_mul(ch.min(hu)).ok_or_else(|| CodecError::LimitExceeded("band size".into()))?;

    let extra_row = lay.extra_row();
    let extra_len = extra_row.checked_mul(hu).ok_or_else(|| CodecError::LimitExceeded("extra samples".into()))?;
    let mut extra_data = if lay.extra_at.is_empty() { Vec::new() } else { zeroed(extra_len)? };
    let extra_band = extra_row.saturating_mul(ch.min(hu));
    let tally = decode_bands(&lay, &mut data, band_bytes, extra_data.as_mut_slice(), extra_band, &mut dec, &open)?;
    if tally.decoded == 0 && tally.missing > 0 {
        return Err(err("the image data is missing (cut off or past the end of the file)"));
    }
    if lab {
        convert_ab(&mut data, out_layout.channels(), out, photometric == 8, false);
    }
    let mut img = Image::from_raw(w, h, out_layout, out, data)?;
    if !lay.extra_at.is_empty() {
        // TIFF 6.0 defines ExtraSamples 0, 1 and 2 only; a reserved value reads as unspecified,
        // so a save never writes it back.
        let kind_of = |si: usize| match si.checked_sub(color).and_then(|k| extra.get(k)).copied() {
            Some(v @ 0..=2) => v as u16,
            _ => 0,
        };
        img.meta.extra_samples = split_extra_samples(&extra_data, wu, hu, &lay.extra_at, out, kind_of)?;
    }
    if extras_over_limit > 0 {
        let count = u32::try_from(extras_over_limit).unwrap_or(u32::MAX);
        img.warnings.push(DecodeWarning::ExtraSamplesOverLimit { format: F, count });
    }
    if extras_dropped > 0 {
        img.warnings.push(DecodeWarning::ExtraSamplesDropped { format: F, count: extras_dropped });
    }
    if tally.missing > 0 {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

/// The a* and b* samples (1 and 2 of every pixel) of a Lab image between the file's encoding and
/// PhotoCraft's (8 bits: `a + 128`; 16 bits: `(a + 128) · 257`). In the file, CIELab holds them
/// signed (8 bits: `a`; 16 bits: `a · 256`) and ICCLab offset (8 bits: `a + 128`; 16 bits:
/// `(a + 128) · 256`). `to_file` picks the direction; samples are native-endian.
fn convert_ab(data: &mut [u8], channels: usize, sample: SampleType, signed: bool, to_file: bool) {
    match sample {
        SampleType::U8 if signed => data.chunks_exact_mut(channels).for_each(|px| {
            px[1] ^= 0x80;
            px[2] ^= 0x80;
        }),
        SampleType::U16 => data.chunks_exact_mut(channels * 2).for_each(|px| {
            for c in [1, 2] {
                let v = u16::from_ne_bytes([px[2 * c], px[2 * c + 1]]);
                // The file's value as `a · 256` (signed) and back.
                let file_a256 = |v: u16| if signed { f64::from(v as i16) } else { f64::from(v) - 32768.0 };
                let out = if to_file {
                    let a256 = (f64::from(v) - 32896.0) * 256.0 / 257.0;
                    let a256 = a256.round().clamp(-32768.0, 32767.0);
                    if signed { (a256 as i16) as u16 } else { (a256 + 32768.0) as u16 }
                } else {
                    (file_a256(v) * 257.0 / 256.0 + 32896.0).round().clamp(0.0, 65535.0) as u16
                };
                px[2 * c..2 * c + 2].copy_from_slice(&out.to_ne_bytes());
            }
        }),
        _ => {}
    }
}

/// File sample indices that are not a colour channel and not the one alpha sample, capped at
/// [`crate::MAX_EXTRA_SAMPLES`], and how many past the cap were left out.
fn extra_sample_indices(spp: usize, color: usize, alpha_at: Option<usize>, has_alpha: bool) -> (Vec<usize>, usize) {
    let mut used = vec![false; spp];
    for i in 0..color.min(spp) {
        if let Some(slot) = used.get_mut(i) {
            *slot = true;
        }
    }
    if has_alpha {
        let a = alpha_at.map(|k| color.saturating_add(k)).unwrap_or(color);
        if let Some(slot) = used.get_mut(a) {
            *slot = true;
        }
    }
    let mut all: Vec<usize> = used.into_iter().enumerate().filter(|(_, on)| !*on).map(|(i, _)| i).collect();
    let over = all.len().saturating_sub(crate::MAX_EXTRA_SAMPLES);
    all.truncate(crate::MAX_EXTRA_SAMPLES);
    (all, over)
}

/// Interleaved extra samples (`extra_at.len()` per pixel) → one plane per sample. Each plane is
/// allocated fallibly (the caller has checked their total against the limits).
fn split_extra_samples(
    buf: &[u8],
    w: usize,
    h: usize,
    at: &[usize],
    sample: SampleType,
    mut kind_of: impl FnMut(usize) -> u16,
) -> Result<Vec<crate::ExtraSample>, CodecError> {
    let b = sample.bytes();
    let n = w.saturating_mul(h);
    let spp = at.len();
    let mut out = Vec::with_capacity(spp);
    for (ei, &si) in at.iter().enumerate() {
        let mut data = zeroed(n.saturating_mul(b))?;
        for i in 0..n {
            let src = (i.saturating_mul(spp).saturating_add(ei)).saturating_mul(b);
            let dst = i.saturating_mul(b);
            if let (Some(s), Some(d)) = (buf.get(src..src.saturating_add(b)), data.get_mut(dst..dst.saturating_add(b))) {
                d.copy_from_slice(s);
            }
        }
        out.push(crate::ExtraSample { kind: kind_of(si), data });
    }
    Ok(out)
}

/// A zero-filled buffer, allocated fallibly. On native targets the zeroing (and the first
/// touch of every page, which dominates on a fresh multi-hundred-megabyte buffer) is spread over
/// all cores.
fn zeroed(len: usize) -> Result<Vec<u8>, CodecError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {len} bytes for the image")))?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        // The capacity is already there: this only writes the zeros.
        v.par_extend(rayon::iter::repeat_n(0u8, len));
    }
    #[cfg(target_arch = "wasm32")]
    v.resize(len, 0);
    Ok(v)
}

/// Chunks decoded and chunks whose data ended early.
#[derive(Default, Clone, Copy)]
struct Tally {
    decoded: usize,
    missing: usize,
}

impl std::ops::Add for Tally {
    type Output = Tally;
    fn add(self, o: Tally) -> Tally {
        Tally { decoded: self.decoded + o.decoded, missing: self.missing + o.missing }
    }
}

/// Decodes every band into `data`, in parallel on native targets (one decoder and scratch
/// buffer per worker). `extra` receives samples that are not part of the colour image, one
/// band of rows at a time, when it is non-empty.
#[cfg(not(target_arch = "wasm32"))]
fn decode_bands<'a, O>(
    lay: &Layout,
    data: &mut [u8],
    band_bytes: usize,
    extra: &mut [u8],
    extra_band: usize,
    _first: &mut Decoder<Patched<'a>>,
    open: &O,
) -> Result<Tally, CodecError>
where
    O: Fn() -> Result<Decoder<Patched<'a>>, CodecError> + Sync,
{
    use rayon::prelude::*;
    if extra.is_empty() {
        return data
            .par_chunks_mut(band_bytes.max(1))
            .enumerate()
            .map_init(
                || open().map(|d| (d, Vec::new())).map_err(|e| e.to_string()),
                |worker, (b, band)| {
                    let (dec, scratch) = worker.as_mut().map_err(|e| err(e.clone()))?;
                    decode_band(lay, dec, scratch, b, band, None)
                },
            )
            .try_reduce(Tally::default, |a, b| Ok(a + b));
    }
    data.par_chunks_mut(band_bytes.max(1))
        .zip(extra.par_chunks_mut(extra_band.max(1)))
        .enumerate()
        .map_init(
            || open().map(|d| (d, Vec::new())).map_err(|e| e.to_string()),
            |worker, (b, (band, extra_band))| {
                let (dec, scratch) = worker.as_mut().map_err(|e| err(e.clone()))?;
                decode_band(lay, dec, scratch, b, band, Some(extra_band))
            },
        )
        .try_reduce(Tally::default, |a, b| Ok(a + b))
}

/// Decodes every band into `data`, one after the other.
#[cfg(target_arch = "wasm32")]
fn decode_bands<'a, O>(
    lay: &Layout,
    data: &mut [u8],
    band_bytes: usize,
    extra: &mut [u8],
    extra_band: usize,
    first: &mut Decoder<Patched<'a>>,
    _open: &O,
) -> Result<Tally, CodecError>
where
    O: Fn() -> Result<Decoder<Patched<'a>>, CodecError> + Sync,
{
    let mut scratch = Vec::new();
    let mut tally = Tally::default();
    if extra.is_empty() {
        for (b, band) in data.chunks_mut(band_bytes.max(1)).enumerate() {
            tally = tally + decode_band(lay, first, &mut scratch, b, band, None)?;
        }
        return Ok(tally);
    }
    for (b, (band, extra_band)) in data.chunks_mut(band_bytes.max(1)).zip(extra.chunks_mut(extra_band.max(1))).enumerate() {
        tally = tally + decode_band(lay, first, &mut scratch, b, band, Some(extra_band))?;
    }
    Ok(tally)
}

/// Decodes band `b` (one strip, or one row of tiles, in every plane) into its output rows.
fn decode_band(
    lay: &Layout,
    dec: &mut Decoder<Patched<'_>>,
    scratch: &mut Vec<u8>,
    b: usize,
    band: &mut [u8],
    mut extra: Option<&mut [u8]>,
) -> Result<Tally, CodecError> {
    let out_row = lay.out_row();
    let rows = band.len() / out_row.max(1);
    let px = lay.out_channels() * lay.out.bytes();
    let mut tally = Tally::default();
    for plane in 0..lay.planes {
        let file_plane = lay.source_sample(plane);
        for col in 0..lay.across {
            let chunk = file_plane
                .checked_mul(lay.chunks_per_plane)
                .and_then(|c| c.checked_add(b.checked_mul(lay.across)?))
                .and_then(|c| c.checked_add(col))
                .and_then(|c| u32::try_from(c).ok())
                .ok_or_else(|| err("chunk index out of range"))?;
            let pref = dec.image_chunk_buffer_layout(chunk).map_err(map_err)?;
            let stride = pref.row_stride.map(|s| s.get()).ok_or_else(|| err("empty chunk row"))?;
            // In place: the strip's decoded bytes are exactly its output rows.
            if lay.direct() && pref.len == band.len() && stride == out_row {
                match dec.read_chunk_bytes(chunk, band) {
                    Ok(()) => tally.decoded += 1,
                    Err(e) if is_eof(&e) => tally.missing += 1,
                    Err(e) => return Err(map_err(e)),
                }
                continue;
            }
            if pref.len > lay.scratch_cap {
                return Err(CodecError::LimitExceeded(format!("a {} byte strip or tile", pref.len)));
            }
            if scratch.len() < pref.len {
                scratch.resize(pref.len, 0);
            }
            let buf = scratch.get_mut(..pref.len).ok_or_else(|| err("scratch buffer"))?;
            match dec.read_chunk_bytes(chunk, buf) {
                Ok(()) => tally.decoded += 1,
                Err(e) if is_eof(&e) => {
                    tally.missing += 1;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            }
            let x0 = col * lay.cw;
            let n = lay.cw.min(lay.w.saturating_sub(x0));
            for (ri, (src_row, dst_row)) in buf.chunks_exact(stride).zip(band.chunks_exact_mut(out_row)).take(rows).enumerate() {
                let Some(dst) = dst_row.get_mut(x0 * px..(x0 + n) * px) else { continue };
                convert_row(lay, src_row, dst, n, plane);
                if !lay.planar && plane == 0 && !lay.extra_at.is_empty() {
                    let erow = lay.extra_row();
                    if let Some(extra) = extra.as_deref_mut()
                        && let Some(er) = extra.get_mut(ri.saturating_mul(erow)..ri.saturating_mul(erow).saturating_add(erow))
                    {
                        deposit_extras(lay, src_row, er, n, x0);
                    }
                }
            }
        }
    }
    if lay.planar && !lay.extra_at.is_empty() {
        read_planar_extras(lay, dec, scratch, b, rows, extra, &mut tally)?;
    }
    finish_band(lay, band);
    Ok(tally)
}

/// Copies the non-colour samples of a chunky row into the extra-sample band.
fn deposit_extras(lay: &Layout, src: &[u8], dst: &mut [u8], n: usize, x0: usize) {
    let ech = lay.extra_at.len();
    if ech == 0 {
        return;
    }
    let px = ech * lay.out.bytes();
    let Some(dst) = dst.get_mut(x0.saturating_mul(px)..(x0.saturating_add(n)).saturating_mul(px)) else { return };
    for (ei, &si) in lay.extra_at.iter().enumerate() {
        transfer(lay.src, src, lay.chunk_spp, si, dst, lay.out, ech, ei, n);
    }
}

/// Planar files store each extra sample as its own plane. Read those planes into `extra`.
fn read_planar_extras(
    lay: &Layout,
    dec: &mut Decoder<Patched<'_>>,
    scratch: &mut Vec<u8>,
    b: usize,
    rows: usize,
    extra: Option<&mut [u8]>,
    tally: &mut Tally,
) -> Result<(), CodecError> {
    let Some(extra) = extra else { return Ok(()) };
    let ech = lay.extra_at.len();
    let erow = lay.extra_row();
    if ech == 0 || erow == 0 {
        return Ok(());
    }
    for (ei, &file_plane) in lay.extra_at.iter().enumerate() {
        for col in 0..lay.across {
            let chunk = file_plane
                .checked_mul(lay.chunks_per_plane)
                .and_then(|c| c.checked_add(b.checked_mul(lay.across)?))
                .and_then(|c| c.checked_add(col))
                .and_then(|c| u32::try_from(c).ok())
                .ok_or_else(|| err("chunk index out of range"))?;
            let pref = dec.image_chunk_buffer_layout(chunk).map_err(map_err)?;
            let stride = pref.row_stride.map(|s| s.get()).ok_or_else(|| err("empty chunk row"))?;
            if pref.len > lay.scratch_cap {
                return Err(CodecError::LimitExceeded(format!("a {} byte strip or tile", pref.len)));
            }
            if scratch.len() < pref.len {
                scratch.resize(pref.len, 0);
            }
            let buf = scratch.get_mut(..pref.len).ok_or_else(|| err("scratch buffer"))?;
            match dec.read_chunk_bytes(chunk, buf) {
                Ok(()) => tally.decoded += 1,
                Err(e) if is_eof(&e) => {
                    tally.missing += 1;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            }
            let x0 = col * lay.cw;
            let n = lay.cw.min(lay.w.saturating_sub(x0));
            for (ri, src_row) in buf.chunks_exact(stride).take(rows).enumerate() {
                let start = ri.saturating_mul(erow);
                let Some(er) = extra.get_mut(start..start.saturating_add(erow)) else { continue };
                let px = ech * lay.out.bytes();
                let Some(dst) = er.get_mut(x0.saturating_mul(px)..(x0.saturating_add(n)).saturating_mul(px)) else { continue };
                transfer(lay.src, src_row, 1, 0, dst, lay.out, ech, ei, n);
            }
        }
    }
    Ok(())
}

/// Converts `n` pixels of one decoded chunk row into output pixels (`plane` for planar data).
fn convert_row(lay: &Layout, src: &[u8], dst: &mut [u8], n: usize, plane: usize) {
    let och = lay.out_channels();
    if let Some(map) = &lay.palette {
        let ob = lay.out.bytes();
        for (i, d) in dst.chunks_exact_mut(och * ob).take(n).enumerate() {
            let index = sample_index(lay.src, src, i);
            let Some(rgb) = map.get(index) else { continue };
            for (c, v) in d.chunks_exact_mut(ob).zip(rgb) {
                if ob == 1 {
                    c.copy_from_slice(&[(v >> 8) as u8]);
                } else {
                    c.copy_from_slice(&v.to_ne_bytes());
                }
            }
        }
        return;
    }
    if lay.copyable() {
        if let Some(s) = src.get(..dst.len()) {
            dst.copy_from_slice(s);
        }
        return;
    }
    if lay.planar {
        transfer(lay.src, src, 1, 0, dst, lay.out, och, plane, n);
    } else {
        for k in 0..och {
            transfer(lay.src, src, lay.chunk_spp, lay.source_sample(k), dst, lay.out, och, k, n);
        }
    }
}

/// The value of pixel `i` in a row of one-sample indices (palette and sub-byte gray).
fn sample_index(src: Src, row: &[u8], i: usize) -> usize {
    match src {
        Src::Bits(bits) => {
            let bits = usize::from(bits);
            let bit = i * bits;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            usize::from((byte >> (8 - bits - bit % 8)) & ((1u8 << bits) - 1))
        }
        Src::U8 => row.get(i).copied().map_or(0, usize::from),
        Src::U16 => row.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0, |s| usize::from(u16::from_ne_bytes(s))),
        _ => 0,
    }
}

/// Copies sample `si` of each of `n` decoded pixels (`sspp` samples each) to output channel
/// `di` (`dch` channels), converting the sample type.
#[allow(clippy::too_many_arguments)]
fn transfer(src: Src, row: &[u8], sspp: usize, si: usize, dst: &mut [u8], out: SampleType, dch: usize, di: usize, n: usize) {
    let ob = out.bytes();
    if let Src::Bits(bits) = src {
        let max = (1u32 << bits) - 1;
        for (i, d) in dst.chunks_exact_mut(dch * ob).take(n).enumerate() {
            if let Some(d) = d.get_mut(di) {
                *d = (sample_index(src, row, i) as u32 * 255 / max) as u8;
            }
        }
        return;
    }
    let sb = src.bytes();
    if src.is_identity() {
        // Same sample type: a strided copy.
        if sb == 1 {
            for (d, s) in dst.iter_mut().skip(di).step_by(dch).zip(row.iter().skip(si).step_by(sspp)).take(n) {
                *d = *s;
            }
        } else {
            for (d, s) in dst.chunks_exact_mut(ob).skip(di).step_by(dch).zip(row.chunks_exact(sb).skip(si).step_by(sspp)).take(n) {
                d.copy_from_slice(s);
            }
        }
        return;
    }
    for (s, d) in row.chunks_exact(sspp * sb).zip(dst.chunks_exact_mut(dch * ob)).take(n) {
        let (Some(s), Some(d)) = (s.get(si * sb..(si + 1) * sb), d.get_mut(di * ob..(di + 1) * ob)) else { continue };
        match src {
            Src::U32 => {
                let v = s.try_into().map_or(0, u32::from_ne_bytes);
                d.copy_from_slice(&((v >> 16) as u16).to_ne_bytes());
            }
            Src::U64 => {
                let v = s.try_into().map_or(0, u64::from_ne_bytes);
                d.copy_from_slice(&((v >> 48) as u16).to_ne_bytes());
            }
            Src::F64 => {
                let v = s.try_into().map_or(0.0, f64::from_ne_bytes);
                d.copy_from_slice(&(v as f32).to_ne_bytes());
            }
            _ => d.copy_from_slice(s),
        }
    }
}

/// Applies what the stored samples leave to the reader: WhiteIsZero inversion of the gray
/// channel, and turning associated (premultiplied) alpha into straight alpha.
fn finish_band(lay: &Layout, band: &mut [u8]) {
    if !lay.invert && !lay.premultiplied {
        return;
    }
    let ch = lay.out_channels();
    let ob = lay.out.bytes();
    let color = lay.out_layout.color_channels();
    for p in band.chunks_exact_mut(ch * ob) {
        match lay.out {
            SampleType::U8 => {
                if lay.invert
                    && let Some(g) = p.first_mut()
                {
                    *g = 255 - *g;
                }
                if lay.premultiplied {
                    let a = u32::from(p.get(ch - 1).copied().unwrap_or(255));
                    for c in p.iter_mut().take(color) {
                        *c = (u32::from(*c) * 255 + a / 2).checked_div(a).unwrap_or(0).min(255) as u8;
                    }
                }
            }
            SampleType::U16 => {
                let get = |p: &[u8], i: usize| p.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0, u16::from_ne_bytes);
                let a = u32::from(get(p, ch - 1));
                for i in 0..color {
                    let mut v = u32::from(get(p, i));
                    if lay.invert && i == 0 {
                        v = 65535 - v;
                    }
                    if lay.premultiplied {
                        v = (v * 65535 + a / 2).checked_div(a).unwrap_or(0).min(65535);
                    }
                    if let Some(s) = p.get_mut(i * 2..i * 2 + 2) {
                        s.copy_from_slice(&(v as u16).to_ne_bytes());
                    }
                }
            }
            SampleType::F16 | SampleType::F32 => {
                let get = |p: &[u8], i: usize| -> f32 {
                    if ob == 2 {
                        p.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0.0, |s| f16::from_bits(u16::from_ne_bytes(s)).to_f32())
                    } else {
                        p.get(i * 4..i * 4 + 4).and_then(|s| s.try_into().ok()).map_or(0.0, f32::from_ne_bytes)
                    }
                };
                let a = get(p, ch - 1);
                for i in 0..color {
                    let mut v = get(p, i);
                    if lay.invert && i == 0 {
                        v = 1.0 - v;
                    }
                    if lay.premultiplied && a > 0.0 {
                        v /= a;
                    }
                    if ob == 2 {
                        if let Some(s) = p.get_mut(i * 2..i * 2 + 2) {
                            s.copy_from_slice(&f16::from_f32(v).to_bits().to_ne_bytes());
                        }
                    } else if let Some(s) = p.get_mut(i * 4..i * 4 + 4) {
                        s.copy_from_slice(&v.to_ne_bytes());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Bytes written with TIFF type UNDEFINED (7), as required for ICC.
struct Undefined<'a>(&'a [u8]);

impl TiffValue for Undefined<'_> {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::UNDEFINED;
    fn count(&self) -> usize {
        self.0.len()
    }
    fn data(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(self.0)
    }
}

macro_rules! custom_colortype {
    ($name:ident, $inner:ty, $photo:expr, $bits:expr, $fmt:expr, int) => {
        struct $name;
        impl TiffColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $fmt;
            fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
                let n = Self::SAMPLE_FORMAT.len();
                let n = n.min(row.len());
                result.extend_from_slice(&row[..n]);
                result.extend(row.iter().zip(&row[n..]).map(|(p, c)| c.wrapping_sub(*p)));
            }
        }
    };
    ($name:ident, $inner:ty, $photo:expr, $bits:expr, $fmt:expr, float) => {
        struct $name;
        impl TiffColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $fmt;
            fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
                result.extend_from_slice(row);
            }
        }
    };
}

use PhotometricInterpretation as PI;
custom_colortype!(GrayA8, u8, PI::BlackIsZero, &[8, 8], &[SampleFormat::Uint; 2], int);
custom_colortype!(GrayA16, u16, PI::BlackIsZero, &[16, 16], &[SampleFormat::Uint; 2], int);
custom_colortype!(GrayA32F, f32, PI::BlackIsZero, &[32, 32], &[SampleFormat::IEEEFP; 2], float);
custom_colortype!(CmykA16, u16, PI::CMYK, &[16; 5], &[SampleFormat::Uint; 5], int);
custom_colortype!(CmykA32F, f32, PI::CMYK, &[32; 5], &[SampleFormat::IEEEFP; 5], float);
// CIELab as Photoshop writes it: a* and b* signed, the sample format left unsigned.
custom_colortype!(Lab8, u8, PI::CIELab, &[8; 3], &[SampleFormat::Uint; 3], int);
custom_colortype!(Lab16, u16, PI::CIELab, &[16; 3], &[SampleFormat::Uint; 3], int);
custom_colortype!(LabA8, u8, PI::CIELab, &[8; 4], &[SampleFormat::Uint; 4], int);
custom_colortype!(LabA16, u16, PI::CIELab, &[16; 4], &[SampleFormat::Uint; 4], int);

// Colour types with 1..=MAX_EXTRA_SAMPLES trailing extra samples (spot channels, extra alpha).
// One generic type per (sample, photometric, bit depth, total sample count) instead of a
// hand-written type for every layout. `N` is the total sample count, not the extra count.
const fn extra_photo(v: u16) -> PhotometricInterpretation {
    match v {
        1 => PhotometricInterpretation::BlackIsZero,
        2 => PhotometricInterpretation::RGB,
        5 => PhotometricInterpretation::CMYK,
        8 => PhotometricInterpretation::CIELab,
        _ => PhotometricInterpretation::RGB,
    }
}

trait ExtraKind: Copy + TiffValue + 'static {
    const FORMAT: SampleFormat;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize);
}

impl ExtraKind for u8 {
    const FORMAT: SampleFormat = SampleFormat::Uint;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize) {
        let n = samples.min(row.len());
        out.extend_from_slice(&row[..n]);
        out.extend(row.iter().zip(&row[n..]).map(|(prev, current)| current.wrapping_sub(*prev)));
    }
}

impl ExtraKind for u16 {
    const FORMAT: SampleFormat = SampleFormat::Uint;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize) {
        let n = samples.min(row.len());
        out.extend_from_slice(&row[..n]);
        out.extend(row.iter().zip(&row[n..]).map(|(prev, current)| current.wrapping_sub(*prev)));
    }
}

impl ExtraKind for f32 {
    const FORMAT: SampleFormat = SampleFormat::IEEEFP;
    fn predict(row: &[Self], out: &mut Vec<Self>, _samples: usize) {
        out.extend_from_slice(row);
    }
}

struct Extras<T, const PHOTO: u16, const BITS: u16, const N: usize>(std::marker::PhantomData<T>);

impl<T, const PHOTO: u16, const BITS: u16, const N: usize> TiffColorType for Extras<T, PHOTO, BITS, N>
where
    T: ExtraKind,
{
    type Inner = T;
    const TIFF_VALUE: PhotometricInterpretation = extra_photo(PHOTO);
    const BITS_PER_SAMPLE: &'static [u16] = &[BITS; N];
    const SAMPLE_FORMAT: &'static [SampleFormat] = &[T::FORMAT; N];
    fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
        T::predict(row, result, N);
    }
}

macro_rules! write_extras {
    ($n:expr, $enc:expr, $w:expr, $h:expr, $data:expr, $tags:expr, $sample:ty, $photo:literal, $bits:literal, $base:literal) => {
        match $n {
            1 => write_one::<Extras<$sample, $photo, $bits, { $base + 1 }>, K>($enc, $w, $h, $data, $tags),
            2 => write_one::<Extras<$sample, $photo, $bits, { $base + 2 }>, K>($enc, $w, $h, $data, $tags),
            3 => write_one::<Extras<$sample, $photo, $bits, { $base + 3 }>, K>($enc, $w, $h, $data, $tags),
            4 => write_one::<Extras<$sample, $photo, $bits, { $base + 4 }>, K>($enc, $w, $h, $data, $tags),
            5 => write_one::<Extras<$sample, $photo, $bits, { $base + 5 }>, K>($enc, $w, $h, $data, $tags),
            6 => write_one::<Extras<$sample, $photo, $bits, { $base + 6 }>, K>($enc, $w, $h, $data, $tags),
            7 => write_one::<Extras<$sample, $photo, $bits, { $base + 7 }>, K>($enc, $w, $h, $data, $tags),
            8 => write_one::<Extras<$sample, $photo, $bits, { $base + 8 }>, K>($enc, $w, $h, $data, $tags),
            9 => write_one::<Extras<$sample, $photo, $bits, { $base + 9 }>, K>($enc, $w, $h, $data, $tags),
            10 => write_one::<Extras<$sample, $photo, $bits, { $base + 10 }>, K>($enc, $w, $h, $data, $tags),
            11 => write_one::<Extras<$sample, $photo, $bits, { $base + 11 }>, K>($enc, $w, $h, $data, $tags),
            12 => write_one::<Extras<$sample, $photo, $bits, { $base + 12 }>, K>($enc, $w, $h, $data, $tags),
            13 => write_one::<Extras<$sample, $photo, $bits, { $base + 13 }>, K>($enc, $w, $h, $data, $tags),
            14 => write_one::<Extras<$sample, $photo, $bits, { $base + 14 }>, K>($enc, $w, $h, $data, $tags),
            15 => write_one::<Extras<$sample, $photo, $bits, { $base + 15 }>, K>($enc, $w, $h, $data, $tags),
            16 => write_one::<Extras<$sample, $photo, $bits, { $base + 16 }>, K>($enc, $w, $h, $data, $tags),
            other => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported extra samples ({other})")).into()),
        }
    };
}

/// An empty buffer with room for `len` elements, allocated fallibly: a save too large for memory
/// is an error, not an abort.
fn try_with_capacity<T>(len: usize) -> io::Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, format!("cannot allocate {len} samples for the extra samples")))?;
    Ok(v)
}

fn bytes_as_u16(b: &[u8]) -> io::Result<Vec<u16>> {
    let chunks = b.as_chunks::<2>().0;
    let mut v = try_with_capacity(chunks.len())?;
    v.extend(chunks.iter().map(|c| u16::from_ne_bytes(*c)));
    Ok(v)
}

fn bytes_as_f32(b: &[u8]) -> io::Result<Vec<f32>> {
    let chunks = b.as_chunks::<4>().0;
    let mut v = try_with_capacity(chunks.len())?;
    v.extend(chunks.iter().map(|c| f32::from_ne_bytes(*c)));
    Ok(v)
}

/// Interleaves colour (and alpha) samples with [`Metadata::extra_samples`].
fn interleave_extras(img: &Image) -> Result<Vec<u8>, String> {
    let (w, h) = img.dimensions();
    let n = (w as usize).checked_mul(h as usize).ok_or_else(|| "image size overflows".to_string())?;
    let sb = img.sample_type().bytes();
    let ch = img.layout().channels();
    let extras = &img.meta.extra_samples;
    let total = ch.checked_add(extras.len()).ok_or_else(|| "too many samples".to_string())?;
    let px = total.checked_mul(sb).ok_or_else(|| "sample size overflows".to_string())?;
    let len = n.checked_mul(px).ok_or_else(|| "image size overflows".to_string())?;
    let mut out = try_with_capacity::<u8>(len).map_err(|e| e.to_string())?;
    out.resize(len, 0);
    let base = img.data();
    let base_px = ch.saturating_mul(sb);
    for i in 0..n {
        let src = i.saturating_mul(base_px);
        let dst = i.saturating_mul(px);
        let Some(s) = base.get(src..src.saturating_add(base_px)) else {
            return Err("image buffer is shorter than its dimensions".into());
        };
        let Some(d) = out.get_mut(dst..dst.saturating_add(base_px)) else {
            return Err("extra-sample buffer is short".into());
        };
        d.copy_from_slice(s);
        for (k, e) in extras.iter().enumerate() {
            let es = i.saturating_mul(sb);
            let ed = dst.saturating_add(base_px).saturating_add(k.saturating_mul(sb));
            let Some(s) = e.data.get(es..es.saturating_add(sb)) else {
                return Err(format!("extra sample {k} is shorter than the image"));
            };
            let Some(d) = out.get_mut(ed..ed.saturating_add(sb)) else {
                return Err("extra-sample buffer is short".into());
            };
            d.copy_from_slice(s);
        }
    }
    Ok(out)
}

fn write_with_extras<K: TiffKind>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, img: &Image, tags: &TagSet<'_>) -> tiff::TiffResult<()> {
    use ChannelLayout as L;
    use SampleType as S;
    let (w, h) = img.dimensions();
    let n = img.meta.extra_samples.len();
    if img.layout().is_lab() {
        let file = lab_file(img);
        let bytes = interleave_extras(&file).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        return match (file.layout(), file.sample_type()) {
            (L::Lab, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 8, 8, 3),
            (L::Lab, _) => {
                let v = bytes_as_u16(&bytes)?;
                write_extras!(n, enc, w, h, &v, tags, u16, 8, 16, 3)
            }
            (_, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 8, 8, 4),
            _ => {
                let v = bytes_as_u16(&bytes)?;
                write_extras!(n, enc, w, h, &v, tags, u16, 8, 16, 4)
            }
        };
    }
    let bytes = interleave_extras(img).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    match (img.layout(), img.sample_type()) {
        (L::Gray, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 1, 8, 1),
        (L::Gray, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 1, 16, 1)
        }
        (L::Gray, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 1, 32, 1)
        }
        (L::GrayA, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 1, 8, 2),
        (L::GrayA, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 1, 16, 2)
        }
        (L::GrayA, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 1, 32, 2)
        }
        (L::Rgb, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 2, 8, 3),
        (L::Rgb, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 2, 16, 3)
        }
        (L::Rgb, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 2, 32, 3)
        }
        (L::Rgba, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 2, 8, 4),
        (L::Rgba, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 2, 16, 4)
        }
        (L::Rgba, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 2, 32, 4)
        }
        (L::Cmyk, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 5, 8, 4),
        (L::Cmyk, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 5, 16, 4)
        }
        (L::Cmyk, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 5, 32, 4)
        }
        (L::CmykA, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 5, 8, 5),
        (L::CmykA, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 5, 16, 5)
        }
        (L::CmykA, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 5, 32, 5)
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported extra samples ({n})")).into()),
    }
}

struct TagSet<'a> {
    icc: Option<&'a [u8]>,
    /// The EXIF block's directories and IFD0 tags, written as the TIFF's own.
    exif: Option<ExifDirs>,
    xmp: Option<&'a [u8]>,
    dpi: Option<(f32, f32)>,
    text: Vec<(Tag, &'a str)>,
    /// ExtraSamples tag, including the unassociated-alpha marker when the image has alpha.
    extra_samples: &'a [u16],
    photoshop_resources: Option<&'a [u8]>,
    photoshop_layers: Option<&'a [u8]>,
}

/// Writes one image directory. (The `tiff` encoder only compresses through `write_data`, which
/// takes every sample at once.)
fn write_one<C, K>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, w: u32, h: u32, data: &[C::Inner], tags: &TagSet<'_>) -> tiff::TiffResult<()>
where
    C: TiffColorType,
    [C::Inner]: TiffValue,
    K: TiffKind,
{
    // The Exif and GPS directories go before the image, outside its chain; IFD0 points to them.
    let mut sub = |e: &ExifDirs, fields: &[tiff_exif::Field]| (!fields.is_empty()).then(|| tiff_exif::write_directory(enc, fields, e.le)).transpose();
    let exif_at = match &tags.exif {
        Some(e) => sub(e, &e.exif)?,
        None => None,
    };
    let gps_at = match &tags.exif {
        Some(e) => sub(e, &e.gps)?,
        None => None,
    };
    let mut im = enc.new_image::<C>(w, h)?;
    if let Some((x, y)) = tags.dpi {
        im.resolution_unit(ResolutionUnit::Inch);
        im.x_resolution(Rational { n: (x * 1000.0).round() as u32, d: 1000 });
        im.y_resolution(Rational { n: (y * 1000.0).round() as u32, d: 1000 });
    }
    let d = im.encoder();
    if !tags.extra_samples.is_empty() {
        d.write_tag(Tag::ExtraSamples, tags.extra_samples)?;
    }
    if let Some(icc) = tags.icc {
        d.write_tag(Tag::IccProfile, Undefined(icc))?;
    }
    if let Some(xmp) = tags.xmp {
        d.write_tag(Tag::Unknown(TAG_XMP), xmp)?;
    }
    for (tag, s) in &tags.text {
        d.write_tag(*tag, *s)?;
    }
    if let Some(e) = &tags.exif {
        // The EXIF block's descriptive IFD0 tags, unless the text metadata already wrote them.
        let ifd0: Vec<tiff_exif::Field> = e.ifd0.iter().filter(|f| !tags.text.iter().any(|(t, _)| t.to_u16() == f.tag)).cloned().collect();
        tiff_exif::write_fields(d, &ifd0, e.le)?;
    }
    if let Some(at) = exif_at {
        d.write_tag(Tag::Unknown(tiff_exif::TAG_EXIF_IFD), at)?;
    }
    if let Some(at) = gps_at {
        d.write_tag(Tag::Unknown(tiff_exif::TAG_GPS_IFD), at)?;
    }
    if let Some(r) = tags.photoshop_resources {
        d.write_tag(Tag::Unknown(TAG_PHOTOSHOP), r)?;
    }
    if let Some(l) = tags.photoshop_layers {
        d.write_tag(Tag::Unknown(TAG_IMAGE_SOURCE_DATA), Undefined(l))?;
    }
    im.write_data(data)
}

/// A Lab image as CIELab stores it: a* and b* signed, 8 or 16 bits (CIELab has no float form:
/// 32-bit Lab is written with 16 bits). Extra samples follow the new sample type.
fn lab_file(img: &Image) -> Image {
    let sample = if img.sample_type() == SampleType::U8 { SampleType::U8 } else { SampleType::U16 };
    // `convert` brings the extra samples to the new sample type with the image.
    let mut file = img.convert(img.layout(), sample);
    let channels = file.layout().channels();
    convert_ab(file.data_mut(), channels, sample, true, true);
    file
}

/// Writes the image with the colour type matching its layout and sample type.
fn write_image<K: TiffKind>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, img: &Image, tags: &TagSet<'_>) -> tiff::TiffResult<()> {
    use ChannelLayout as L;
    use SampleType as S;
    if !img.meta.extra_samples.is_empty() {
        return write_with_extras::<K>(enc, img, tags);
    }
    let (w, h) = img.dimensions();
    let u16s = || img.to_u16_samples().unwrap_or_default();
    let f32s = || img.to_f32_samples().unwrap_or_default();
    let d8 = img.data();
    match (img.layout(), img.sample_type()) {
        (L::Gray, S::U8) => write_one::<colortype::Gray8, K>(enc, w, h, d8, tags),
        (L::Gray, S::U16) => write_one::<colortype::Gray16, K>(enc, w, h, &u16s(), tags),
        (L::Gray, S::F32) => write_one::<colortype::Gray32Float, K>(enc, w, h, &f32s(), tags),
        (L::GrayA, S::U8) => write_one::<GrayA8, K>(enc, w, h, d8, tags),
        (L::GrayA, S::U16) => write_one::<GrayA16, K>(enc, w, h, &u16s(), tags),
        (L::GrayA, S::F32) => write_one::<GrayA32F, K>(enc, w, h, &f32s(), tags),
        (L::Rgb, S::U8) => write_one::<colortype::RGB8, K>(enc, w, h, d8, tags),
        (L::Rgb, S::U16) => write_one::<colortype::RGB16, K>(enc, w, h, &u16s(), tags),
        (L::Rgb, S::F32) => write_one::<colortype::RGB32Float, K>(enc, w, h, &f32s(), tags),
        (L::Rgba, S::U8) => write_one::<colortype::RGBA8, K>(enc, w, h, d8, tags),
        (L::Rgba, S::U16) => write_one::<colortype::RGBA16, K>(enc, w, h, &u16s(), tags),
        (L::Rgba, S::F32) => write_one::<colortype::RGBA32Float, K>(enc, w, h, &f32s(), tags),
        (L::Cmyk, S::U8) => write_one::<colortype::CMYK8, K>(enc, w, h, d8, tags),
        (L::Cmyk, S::U16) => write_one::<colortype::CMYK16, K>(enc, w, h, &u16s(), tags),
        (L::Cmyk, S::F32) => write_one::<colortype::CMYK32Float, K>(enc, w, h, &f32s(), tags),
        (L::CmykA, S::U8) => write_one::<colortype::CMYKA8, K>(enc, w, h, d8, tags),
        (L::CmykA, S::U16) => write_one::<CmykA16, K>(enc, w, h, &u16s(), tags),
        (L::CmykA, S::F32) => write_one::<CmykA32F, K>(enc, w, h, &f32s(), tags),
        (L::Lab | L::LabA, _) => {
            let file = lab_file(img);
            let u16s = || file.to_u16_samples().unwrap_or_default();
            match (file.layout(), file.sample_type()) {
                (L::Lab, S::U8) => write_one::<Lab8, K>(enc, w, h, file.data(), tags),
                (L::Lab, _) => write_one::<Lab16, K>(enc, w, h, &u16s(), tags),
                (_, S::U8) => write_one::<LabA8, K>(enc, w, h, file.data(), tags),
                _ => write_one::<LabA16, K>(enc, w, h, &u16s(), tags),
            }
        }
        (l, s) => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported {l:?} {s:?}")).into()),
    }
}

/// Classic TIFF offsets are 32-bit: a file that may grow past this is written as BigTIFF.
const CLASSIC_LIMIT: u64 = u32::MAX as u64;

/// An upper estimate of the file size: the uncompressed samples plus every tag payload plus
/// room for the directory and strip tables.
fn estimated_size(img: &Image, tags: &TagSet<'_>) -> u64 {
    let payloads = [tags.icc, tags.xmp, tags.photoshop_resources, tags.photoshop_layers].iter().flatten().map(|p| p.len() as u64).sum::<u64>();
    let text = tags.text.iter().map(|(_, s)| s.len() as u64).sum::<u64>()
        + tags.exif.iter().flat_map(|e| e.ifd0.iter().chain(&e.exif).chain(&e.gps)).map(|f| f.value.len() as u64 + 12).sum::<u64>();
    // Strip tables: one offset and one count per strip of about 1 MB.
    let tables = (img.data().len() as u64 / (1 << 20) + 1) * 16;
    let extras: u64 = img.meta.extra_samples.iter().map(|e| e.data.len() as u64).sum();
    (img.data().len() as u64).saturating_add(extras).saturating_add(payloads).saturating_add(text).saturating_add(tables).saturating_add(1 << 20)
}

/// The byte length of one extra-sample plane: one sample per pixel of `img`, in its sample type.
/// `None` when the pixel count overflows a `usize` (no such image buffer can exist).
fn extra_plane_len(img: &Image) -> Option<usize> {
    let (w, h) = img.dimensions();
    (w as usize).checked_mul(h as usize)?.checked_mul(img.sample_type().bytes())
}

/// Every extra plane must hold exactly one sample per pixel of `img`, in its sample type.
///
/// The writer copies one sample per pixel out of each plane, so a plane of any other length
/// would be silently truncated (long) or read past its end (short). A sample-type conversion
/// reinterprets the bytes in the new width instead, so an unchecked plane would also change
/// meaning rather than fail. Refuse the image instead of writing either.
fn check_extra_planes(img: &Image) -> Result<(), CodecError> {
    let (w, h) = img.dimensions();
    let sample = img.sample_type();
    let plane_len = extra_plane_len(img).ok_or_else(|| CodecError::encode(F, format!("the extra-sample size of a {w}x{h} image overflows")))?;
    for (k, e) in img.meta.extra_samples.iter().enumerate() {
        if e.data.len() != plane_len {
            return Err(CodecError::encode(
                F,
                format!("extra sample {k} is {} bytes; a {w}x{h} {sample:?} image needs exactly {plane_len} (one sample per pixel)", e.data.len()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    // Check the source planes before `converted`, which brings them to the plan's sample type
    // with the image but deliberately leaves a wrong-sized plane alone: past that point a bad
    // plane would be reinterpreted in the new width rather than rejected.
    check_extra_planes(src)?;
    // `converted` brings the extra samples to the plan's sample type with the image.
    encode_ready(&src.converted(plan.layout, plan.sample), opts)
}

fn encode_ready(img: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let compression = match opts.tiff_compression {
        TiffCompression::None => tiff::encoder::Compression::Uncompressed,
        TiffCompression::Lzw => tiff::encoder::Compression::Lzw,
        TiffCompression::Deflate => tiff::encoder::Compression::Deflate(tiff::encoder::DeflateLevel::Balanced),
        TiffCompression::PackBits => tiff::encoder::Compression::Packbits,
    };
    let predictor = if !img.sample_type().is_float() && matches!(opts.tiff_compression, TiffCompression::Lzw | TiffCompression::Deflate) {
        tiff::encoder::Predictor::Horizontal
    } else {
        tiff::encoder::Predictor::None
    };
    let text: Vec<(Tag, &str)> = if opts.embed_metadata {
        img.meta
            .text
            .iter()
            .filter_map(|(k, v)| TEXT_TAGS.iter().find(|(_, key)| key.eq_ignore_ascii_case(k)).map(|(t, _)| (*t, v.as_str())))
            .filter(|(_, v)| v.is_ascii() && !v.contains('\0'))
            .collect()
    } else {
        Vec::new()
    };
    // TIFF writes no Orientation tag (= 1): keep the XMP from contradicting it.
    let xmp = if opts.embed_metadata { img.meta.xmp.as_deref().map(|x| crate::resolution::export_xmp(x, img.meta.dpi)) } else { None };
    if img.meta.extra_samples.len() > crate::MAX_EXTRA_SAMPLES {
        return Err(CodecError::encode(F, format!("TIFF stores at most {} extra samples", crate::MAX_EXTRA_SAMPLES)));
    }
    // Validate again in the sample type the planes now hold: `Image::convert` takes a plane's
    // bytes before it rebuilds them, so one it could not rebuild would arrive here empty. The Lab
    // path's later re-type through `lab_file` converts a plane only when its length already
    // matches the image, which this guarantees.
    check_extra_planes(img)?;
    let mut kinds = Vec::with_capacity(img.meta.extra_samples.len().saturating_add(1));
    if img.layout().has_alpha() {
        kinds.push(2);
    }
    // TIFF 6.0 reserves ExtraSamples values above 2: those are written as 0 (unspecified).
    for e in &img.meta.extra_samples {
        kinds.push(if e.kind <= 2 { e.kind } else { 0 });
    }
    let tags = TagSet {
        icc: if opts.embed_icc { img.icc.as_deref() } else { None },
        exif: if opts.embed_metadata { img.meta.exif.as_deref().and_then(|e| tiff_exif::read(crate::orientation::tiff_body(e))) } else { None },
        xmp: xmp.as_deref().map(str::as_bytes),
        dpi: if opts.embed_metadata { img.meta.dpi.filter(|d| d.0 > 0.0 && d.1 > 0.0) } else { None },
        text,
        extra_samples: &kinds,
        // Document content, not metadata: written whenever present.
        photoshop_resources: img.meta.photoshop_resources.as_deref().filter(|r| !r.is_empty()),
        photoshop_layers: img.meta.photoshop_layers.as_deref().filter(|l| !l.is_empty()),
    };

    // BigTIFF when asked, or when the file could pass the 4 GiB a classic TIFF can address. A
    // classic write that still runs out of 32-bit offsets (compression grew the data) is redone
    // as BigTIFF.
    let write = |big: bool| -> tiff::TiffResult<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        if big {
            let mut enc = TiffEncoder::new_big(&mut cursor)?.with_compression(compression).with_predictor(predictor);
            write_image::<TiffKindBig>(&mut enc, img, &tags)?;
        } else {
            let mut enc = TiffEncoder::new(&mut cursor)?.with_compression(compression).with_predictor(predictor);
            write_image::<TiffKindStandard>(&mut enc, img, &tags)?;
        }
        Ok(cursor.into_inner())
    };
    let big = opts.tiff_bigtiff || estimated_size(img, &tags) > CLASSIC_LIMIT;
    match write(big) {
        Ok(v) => Ok(v),
        Err(tiff::TiffError::IntSizeError) if !big => write(true).map_err(|e| CodecError::encode(F, e)),
        Err(e) => Err(CodecError::encode(F, e)),
    }
}

#[cfg(test)]
mod bits_len {
    use super::Extras;
    use tiff::encoder::colortype::ColorType as TiffColorType;

    #[test]
    fn const_generic_sample_tables_have_one_entry_per_sample() {
        // A repeat expression in an associated const (`&[BITS; N]`) has, on some compilers,
        // promoted to a slice shorter than `N`. The encoder sizes every strip from that length.
        assert_eq!(<Extras<u8, 2, 8, 4> as TiffColorType>::BITS_PER_SAMPLE.len(), 4, "{:?}", <Extras<u8, 2, 8, 4> as TiffColorType>::BITS_PER_SAMPLE);
        assert_eq!(<Extras<u8, 2, 8, 5> as TiffColorType>::BITS_PER_SAMPLE.len(), 5);
        assert_eq!(<Extras<u8, 2, 8, 19> as TiffColorType>::BITS_PER_SAMPLE.len(), 19);
        assert_eq!(<Extras<u16, 5, 16, 6> as TiffColorType>::SAMPLE_FORMAT.len(), 6);
    }
}

#[cfg(test)]
mod extra_sample_buffers {
    use super::try_with_capacity;

    /// The extra-sample writer's buffers are allocated fallibly: a size no allocation can serve
    /// is an out-of-memory error the save reports, not an abort.
    #[test]
    fn an_impossible_buffer_is_an_error() {
        let err = try_with_capacity::<u8>(usize::MAX).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::OutOfMemory);
        assert!(try_with_capacity::<u16>(usize::MAX / 2).is_err());
        assert!(try_with_capacity::<f32>(64).is_ok_and(|v| v.is_empty() && v.capacity() >= 64));
    }
}
