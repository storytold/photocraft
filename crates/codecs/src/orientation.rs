//! EXIF / TIFF orientation (tag 274): reading it, applying it to the pixels,
//! and rewriting it to 1 ("top-left", upright) once the pixels are upright.
//!
//! Decoders apply the orientation by default (like Photoshop and every photo
//! viewer), so a document is always edited upright. Encoders then write
//! Orientation = 1, so a viewer never rotates the already-upright pixels a
//! second time.
//!
//! Orientation values (TIFF 6.0, EXIF 2.3): 1 = upright, 2 = mirrored, 3 =
//! rotated 180°, 4 = flipped vertically, 5 = transposed, 6 = needs a 90°
//! clockwise turn, 7 = transverse, 8 = needs a 90° counter-clockwise turn.
//! Anything malformed, truncated or out of range reads as 1: an IFD0 that
//! starts inside the 8-byte header or is cut off before its next-IFD pointer,
//! or an Orientation entry that is not exactly one SHORT or LONG (the spec says
//! SHORT, but some cameras and tools write LONG). When IFD0 holds
//! several Orientation entries the first one wins (like libexif).

use std::borrow::Cow;

use crate::error::CodecError;
use crate::image::Image;

/// The TIFF/EXIF Orientation tag.
const TAG_ORIENTATION: u16 = 274;
/// Field types accepted for the Orientation tag: SHORT (the spec) and LONG
/// (written by some cameras and tools).
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
/// Bytes per IFD entry.
const ENTRY: usize = 12;
/// The TIFF header is 8 bytes, so no IFD can start before it.
const HEADER: usize = 8;
/// BigTIFF uses a 16-byte header, 20-byte entries, and 8-byte offsets.
const BIG_ENTRY: usize = 20;
const BIG_HEADER: usize = 16;

/// Byte order of a TIFF structure.
#[derive(Clone, Copy)]
pub(crate) enum Order {
    Little,
    Big,
}

impl Order {
    pub(crate) fn u16(self, b: &[u8], at: usize) -> Option<u16> {
        let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(match self {
            Order::Little => u16::from_le_bytes(s),
            Order::Big => u16::from_be_bytes(s),
        })
    }

    pub(crate) fn u32(self, b: &[u8], at: usize) -> Option<u32> {
        let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(match self {
            Order::Little => u32::from_le_bytes(s),
            Order::Big => u32::from_be_bytes(s),
        })
    }

    pub(crate) fn u64(self, b: &[u8], at: usize) -> Option<u64> {
        let s: [u8; 8] = b.get(at..at.checked_add(8)?)?.try_into().ok()?;
        Some(match self {
            Order::Little => u64::from_le_bytes(s),
            Order::Big => u64::from_be_bytes(s),
        })
    }
}

/// The TIFF payload of an EXIF block (a JPEG-style `Exif\0\0` prefix is skipped).
pub(crate) fn tiff_body(b: &[u8]) -> &[u8] {
    b.strip_prefix(b"Exif\0\0").unwrap_or(b)
}

/// IFD0 of a TIFF structure: byte order, entry layout, and entry offsets.
///
/// The full table and next-IFD pointer must be present, and IFD0 must start
/// after the corresponding TIFF or BigTIFF header.
pub(crate) fn ifd0_entries(b: &[u8]) -> Option<(Order, usize, usize, usize, bool)> {
    let (order, big_tiff) = match b.get(0..4)? {
        [b'I', b'I', 42, 0] => (Order::Little, false),
        [b'M', b'M', 0, 42] => (Order::Big, false),
        [b'I', b'I', 43, 0] => (Order::Little, true),
        [b'M', b'M', 0, 43] => (Order::Big, true),
        _ => return None,
    };
    let (ifd, count_bytes, entry_bytes, next_ifd_bytes, header) = if big_tiff {
        if order.u16(b, 4)? != 8 || order.u16(b, 6)? != 0 {
            return None;
        }
        (usize::try_from(order.u64(b, 8)?).ok()?, 8usize, BIG_ENTRY, 8usize, BIG_HEADER)
    } else {
        (usize::try_from(order.u32(b, 4)?).ok()?, 2usize, ENTRY, 4usize, HEADER)
    };
    if ifd < header {
        return None;
    }
    let count = if big_tiff { usize::try_from(order.u64(b, ifd)?).ok()? } else { usize::from(order.u16(b, ifd)?) };
    let first = ifd.checked_add(count_bytes)?;
    let next_ifd = first.checked_add(count.checked_mul(entry_bytes)?)?;
    if next_ifd.checked_add(next_ifd_bytes)? > b.len() {
        return None;
    }
    // In bounds: every entry ends at or before `next_ifd`.
    Some((order, first, count, entry_bytes, big_tiff))
}

/// A well-formed Orientation entry's value: where it sits and its field type.
#[derive(Clone, Copy)]
struct Value {
    at: usize,
    ty: u16,
}

impl Value {
    /// The stored value (a SHORT or LONG, left-justified in the 4-byte field).
    fn read(self, order: Order, b: &[u8]) -> Option<u32> {
        match self.ty {
            TYPE_SHORT => order.u16(b, self.at).map(u32::from),
            _ => order.u32(b, self.at),
        }
    }

    /// 1 in this entry's own type, width and byte order.
    fn one(self, order: Order) -> Vec<u8> {
        match (self.ty, order) {
            (TYPE_SHORT, Order::Little) => 1u16.to_le_bytes().to_vec(),
            (TYPE_SHORT, Order::Big) => 1u16.to_be_bytes().to_vec(),
            (_, Order::Little) => 1u32.to_le_bytes().to_vec(),
            (_, Order::Big) => 1u32.to_be_bytes().to_vec(),
        }
    }
}

/// The value of the entry at `e` if it is a well-formed Orientation entry:
/// exactly one SHORT or LONG.
fn orientation_value(order: Order, b: &[u8], e: usize, big_tiff: bool) -> Option<Value> {
    let count = if big_tiff { order.u64(b, e.checked_add(4)?)? } else { u64::from(order.u32(b, e.checked_add(4)?)?) };
    if order.u16(b, e)? != TAG_ORIENTATION || count != 1 {
        return None;
    }
    let ty = order.u16(b, e.checked_add(2)?)?;
    let value_offset = if big_tiff { 12 } else { 8 };
    matches!(ty, TYPE_SHORT | TYPE_LONG).then_some(Value { at: e.checked_add(value_offset)?, ty })
}

/// Locates the Orientation value in IFD0.
///
/// The **first** Orientation entry wins (like libexif and Photoshop); if that
/// one is malformed (count ≠ 1, or neither SHORT nor LONG) the tag is ignored,
/// even when a later duplicate is well formed.
fn find_entry(b: &[u8]) -> Option<(Order, Value)> {
    let (order, first, count, entry_bytes, big_tiff) = ifd0_entries(b)?;
    let e = (0..count).filter_map(|i| first.checked_add(i.checked_mul(entry_bytes)?)).find(|&e| order.u16(b, e) == Some(TAG_ORIENTATION))?;
    Some((order, orientation_value(order, b, e, big_tiff)?))
}

/// The orientation (1–8) recorded in a TIFF-structured block: an EXIF payload
/// (with or without the `Exif\0\0` prefix) or a whole TIFF file. Missing,
/// malformed or out-of-range values give 1.
pub fn exif_orientation(exif: &[u8]) -> u16 {
    let b = tiff_body(exif);
    find_entry(b).and_then(|(order, v)| v.read(order, b)).and_then(|v| u16::try_from(v).ok()).filter(|v| (1..=8).contains(v)).unwrap_or(1)
}

/// `exif` with its Orientation rewritten to 1, borrowed when there is nothing
/// to change (no well-formed tag, already 1, or unparseable).
///
/// Only the value bytes of each well-formed Orientation entry change, written
/// as 1 in the entry's existing type and width (SHORT or LONG); every other
/// byte (other tags, their offset-based values, sub-IFDs) is kept as is.
/// Well-formed duplicates are all set to 1, so a reader that lets a later
/// duplicate win cannot rotate the upright pixels either.
pub fn upright_exif(exif: &[u8]) -> Cow<'_, [u8]> {
    let body = tiff_body(exif);
    let prefix = exif.len() - body.len();
    let Some((order, first, count, entry_bytes, big_tiff)) = ifd0_entries(body) else {
        return Cow::Borrowed(exif);
    };
    let values: Vec<Value> =
        (0..count).filter_map(|i| first.checked_add(i.checked_mul(entry_bytes)?)).filter_map(|e| orientation_value(order, body, e, big_tiff)).collect();
    if values.iter().all(|v| v.read(order, body) == Some(1)) {
        return Cow::Borrowed(exif);
    }
    let mut out = exif.to_vec();
    for v in values {
        let one = v.one(order);
        let slot = prefix.checked_add(v.at).and_then(|at| out.get_mut(at..at.checked_add(one.len())?));
        if let Some(slot) = slot {
            slot.copy_from_slice(&one);
        }
    }
    Cow::Owned(out)
}

/// `xmp` with numeric TIFF Orientation properties (attribute or simple element form)
/// set to 1. Namespace aliases are resolved; unrelated bytes are kept verbatim.
/// Malformed packets, DTDs and packets exceeding the XML parsing budgets are
/// borrowed unchanged rather than risking a partial or ambiguous rewrite. The
/// conservative markup budget is 256 `<` bytes, including comments and CDATA.
pub fn upright_xmp(xmp: &str) -> Cow<'_, str> {
    const TIFF_NS: &str = "http://ns.adobe.com/tiff/1.0/";
    const MAX_XML_NODES: u32 = 256;
    const MAX_XML_ATTRIBUTES: usize = 4096;
    const MAX_XML_NAMESPACES: usize = 256;
    const MAX_XML_TEXT_WORK: usize = 16 * 1024 * 1024;
    if !xmp.contains("Orientation") {
        return Cow::Borrowed(xmp);
    }
    // Adjacent text/CDATA is repeatedly concatenated into one node by roxmltree
    // 0.20, so nodes_limit alone does not bound that quadratic copying. Each CDATA
    // can split text twice; packet bytes times that segment bound conservatively
    // bounds cumulative copies, and also caps the input length. DTDs stay off, so
    // general entities cannot introduce more segments or expand the source text.
    let text_segments = xmp.match_indices("<![CDATA[").count().saturating_mul(2).saturating_add(1);
    if xmp.len().saturating_mul(text_segments) > MAX_XML_TEXT_WORK {
        return Cow::Borrowed(xmp);
    }
    // roxmltree reserves nodes/attributes from these marker counts before parsing.
    // The tokenizer is recursive, so a small total markup budget also bounds its
    // stack depth, even for unclosed input. Attribute duplicate checks and inherited
    // namespace lists also need bounds. These marker counts deliberately
    // overestimate structure inside comments/values; DTDs stay off.
    if xmp.bytes().filter(|b| *b == b'<').take(MAX_XML_NODES as usize + 1).count() > MAX_XML_NODES as usize
        || xmp.bytes().filter(|b| *b == b'=').take(MAX_XML_ATTRIBUTES + 1).count() > MAX_XML_ATTRIBUTES
        || xmp.match_indices("xmlns").take(MAX_XML_NAMESPACES + 1).count() > MAX_XML_NAMESPACES
    {
        return Cow::Borrowed(xmp);
    }
    let opts = roxmltree::ParsingOptions { nodes_limit: MAX_XML_NODES, ..Default::default() };
    let Ok(doc) = roxmltree::Document::parse_with_options(xmp, opts) else { return Cow::Borrowed(xmp) };
    let mut edits = Vec::new();
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        for attr in node.attributes().filter(|a| a.namespace() == Some(TIFF_NS) && a.name() == "Orientation") {
            // Use the full attribute span: range_value() truncates long names or
            // unusually large whitespace runs around '=' in roxmltree 0.20.
            let range = attr.range();
            let Some(raw) = xmp.get(range.clone()) else { continue };
            let Some(quote) = raw.find(['\'', '"']) else { continue };
            let Some(start) = range.start.checked_add(quote + 1) else { continue };
            let Some(end) = range.end.checked_sub(1) else { continue };
            if let Some(edit) = xmp_orientation_edit(xmp, start..end, attr.value()) {
                edits.push(edit);
            }
        }
        if !node.has_tag_name((TIFF_NS, "Orientation")) {
            continue;
        }
        let mut children = node.children();
        let Some(text) = children.next().filter(roxmltree::Node::is_text) else { continue };
        if children.next().is_some() {
            continue;
        }
        let mut range = text.range();
        // Adjacent text / CDATA sections are coalesced, but the parser's range
        // only covers the first section. Do not rewrite just part of a value.
        if !xmp.get(range.end..node.range().end).is_some_and(|s| s.starts_with("</")) {
            continue;
        }
        if xmp.get(range.clone()).is_some_and(|s| s.starts_with("<![CDATA[")) {
            range.start = range.start.saturating_add(9);
            range.end = range.end.saturating_sub(3);
        }
        if let Some(edit) = xmp_orientation_edit(xmp, range, text.text().unwrap_or_default()) {
            edits.push(edit);
        }
    }
    if edits.is_empty() {
        return Cow::Borrowed(xmp);
    }
    edits.sort_unstable_by_key(|e| e.start);
    let mut out = String::with_capacity(xmp.len());
    let mut at = 0;
    for edit in edits {
        let Some(before) = xmp.get(at..edit.start) else { return Cow::Borrowed(xmp) };
        out.push_str(before);
        out.push('1');
        at = edit.end;
    }
    let Some(tail) = xmp.get(at..) else { return Cow::Borrowed(xmp) };
    out.push_str(tail);
    Cow::Owned(out)
}

/// The numeric part of an XML value, keeping its original surrounding whitespace.
fn xmp_orientation_edit(xmp: &str, range: std::ops::Range<usize>, decoded: &str) -> Option<std::ops::Range<usize>> {
    let value = decoded.trim_ascii();
    if value.is_empty() || value == "1" || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let raw = xmp.get(range.clone())?;
    let start = range.start.checked_add(raw.len() - raw.trim_ascii_start().len())?;
    let end = range.start.checked_add(raw.trim_ascii_end().len())?;
    (start < end).then_some(start..end)
}

/// Source pixel index for output pixel `(0, y2)` and the step per output column, for
/// orientation `o` of a `w`×`h` source (values 2–8; see the module docs).
fn walk(o: u16, w: isize, h: isize, y2: isize) -> (isize, isize) {
    match o {
        2 => (y2 * w + w - 1, -1),
        3 => ((h - 1 - y2) * w + w - 1, -1),
        4 => ((h - 1 - y2) * w, 1),
        5 => (y2, w),
        6 => ((h - 1) * w + y2, -w),
        7 => ((h - 1) * w + (w - 1 - y2), -w),
        8 => (w - 1 - y2, w),
        _ => (y2 * w, 1),
    }
}

/// Output columns per block: a block's source pixels (for a 90° turn, `BLOCK`
/// rows × the band height) stay in cache while it is written.
const BLOCK: usize = 64;
/// Output rows per parallel band.
const BAND: usize = 64;

/// Writes the oriented pixels of one band of output rows, `N` bytes per pixel.
fn band<const N: usize>(src: &[[u8; N]], dst: &mut [[u8; N]], o: u16, (w, h): (usize, usize), ow: usize, y0: usize) {
    let (wi, hi) = (w as isize, h as isize);
    let rows = dst.len() / ow.max(1);
    for x0 in (0..ow).step_by(BLOCK) {
        let x1 = (x0 + BLOCK).min(ow);
        for r in 0..rows {
            let (base, step) = walk(o, wi, hi, (y0 + r) as isize);
            let Some(out) = dst.get_mut(r * ow + x0..r * ow + x1) else { return };
            let mut i = base + x0 as isize * step;
            for px in out {
                if let Some(s) = usize::try_from(i).ok().and_then(|i| src.get(i)) {
                    *px = *s;
                }
                i += step;
            }
        }
    }
}

/// Orients `src` (`N` bytes per pixel) into a new buffer, a band of rows per task.
fn orient_n<const N: usize>(src: &[u8], o: u16, (w, h): (usize, usize), ow: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = alloc_zeroed(src.len())?;
    let (s, _) = src.as_chunks::<N>();
    let (d, _) = out.as_chunks_mut::<N>();
    let per = (BAND * ow).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        d.par_chunks_mut(per).enumerate().for_each(|(b, chunk)| band(s, chunk, o, (w, h), ow, b * BAND));
    }
    #[cfg(target_arch = "wasm32")]
    for (b, chunk) in d.chunks_mut(per).enumerate() {
        band(s, chunk, o, (w, h), ow, b * BAND);
    }
    Ok(out)
}

/// A zeroed buffer of `n` bytes, or an error (not an abort) when memory runs out.
fn alloc_zeroed(n: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {n} bytes to orient the image")))?;
    out.resize(n, 0);
    Ok(out)
}

/// Pixel-format-agnostic orientation: any byte size per pixel.
fn orient_bytes(src: &[u8], bpp: usize, o: u16, (w, h): (usize, usize), ow: usize) -> Result<Vec<u8>, CodecError> {
    match bpp {
        1 => orient_n::<1>(src, o, (w, h), ow),
        2 => orient_n::<2>(src, o, (w, h), ow),
        3 => orient_n::<3>(src, o, (w, h), ow),
        4 => orient_n::<4>(src, o, (w, h), ow),
        5 => orient_n::<5>(src, o, (w, h), ow),
        6 => orient_n::<6>(src, o, (w, h), ow),
        8 => orient_n::<8>(src, o, (w, h), ow),
        10 => orient_n::<10>(src, o, (w, h), ow),
        12 => orient_n::<12>(src, o, (w, h), ow),
        16 => orient_n::<16>(src, o, (w, h), ow),
        _ => orient_n::<20>(src, o, (w, h), ow),
    }
}

impl Image {
    /// The image turned upright for orientation `o` (1–8; anything else is
    /// treated as 1 and returns the image unchanged). Width and height swap
    /// for 5–8, as does the DPI, and the EXIF and XMP orientation are
    /// rewritten to 1 so the result is not rotated again on display or export.
    pub fn oriented(self, o: u16) -> Result<Image, CodecError> {
        if !(2..=8).contains(&o) {
            return Ok(self);
        }
        let (w, h) = (self.width() as usize, self.height() as usize);
        let swap = o >= 5;
        let (ow, oh) = if swap { (h, w) } else { (w, h) };
        let (layout, sample) = (self.layout(), self.sample_type());
        let bpp = layout.channels() * sample.bytes();
        // Every layout/sample pair is 1–20 bytes per pixel and has a fixed-size path.
        if !matches!(bpp, 1..=6 | 8 | 10 | 12 | 16 | 20) || w == 0 || h == 0 {
            return Err(CodecError::InvalidImage(format!("cannot orient {layout:?} {sample:?} {w}x{h}")));
        }
        let data = orient_bytes(self.data(), bpp, o, (w, h), ow)?;
        let mut meta = self.meta.clone();
        if let Some(e) = &mut meta.exif
            && let Cow::Owned(fixed) = upright_exif(e)
        {
            *e = fixed;
        }
        if let Some(x) = &mut meta.xmp
            && let Cow::Owned(fixed) = upright_xmp(x)
        {
            *x = fixed;
        }
        if swap {
            meta.dpi = meta.dpi.map(|(x, y)| (y, x));
        }
        let mut out = Image::from_raw(ow as u32, oh as u32, layout, sample, data)?.with_icc(self.icc).with_meta(meta);
        out.warnings = self.warnings;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impossible_allocation_is_an_error_not_an_abort() {
        assert!(matches!(alloc_zeroed(usize::MAX), Err(CodecError::LimitExceeded(_))));
        assert!(matches!(orient_n::<1>(&[], 6, (0, 0), 0), Ok(v) if v.is_empty()));
        assert_eq!(alloc_zeroed(3).unwrap(), vec![0, 0, 0]);
    }
}
