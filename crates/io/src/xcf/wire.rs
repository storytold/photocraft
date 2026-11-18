//! Bounds-checked XCF framing, properties and tile compression.

#[cfg(test)]
use super::HashMap;
use super::{MAX_PAYLOAD, MAX_STRING, Read, Result, invalid};

#[cfg(test)]
mod parasite_retention_tests;
#[cfg(test)]
mod parasite_work_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compression {
    None,
    Rle,
    Zlib,
}

/// Big-endian reads with bounds checks; every failure is an error, never a panic.
pub(super) struct Cursor<'a> {
    pub(super) b: &'a [u8],
    pub(super) pos: usize,
    /// XCF 11 and later store pointers as 64-bit.
    pub(super) wide: bool,
}

impl<'a> Cursor<'a> {
    pub(super) fn at(&self, pos: usize) -> Cursor<'a> {
        Cursor { b: self.b, pos, wide: self.wide }
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("offset overflow"))?;
        let s = self.b.get(self.pos..end).ok_or_else(|| invalid(format!("the file ends early (wanted {n} bytes at {})", self.pos)))?;
        self.pos = end;
        Ok(s)
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub(super) fn ptr(&mut self) -> Result<usize> {
        if self.wide {
            let s = self.take(8)?;
            usize::try_from(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])).map_err(|_| invalid("pointer does not fit in memory"))
        } else {
            self.u32().map(|v| v as usize)
        }
    }

    /// A STRING: `n+1` then `n` bytes and a NUL (0 for the empty string).
    pub(super) fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The same bounded STRING read without allocating or decoding an unused field.
    pub(super) fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return Err(invalid(format!("a string of {n} bytes exceeds the limit")));
        }
        let s = self.take(n)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s))
    }

    pub(super) fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One property record: its type and payload.
pub(super) struct Prop<'a> {
    pub(super) offset: usize,
    pub(super) kind: u32,
    pub(super) data: &'a [u8],
}

/// Read a drawable/path property list, using the declared framing for image-only properties.
pub(super) fn props<'a>(c: &mut Cursor<'a>) -> Result<Vec<Prop<'a>>> {
    props_version(c, None)
}

/// Image properties can carry historical lengths that GIMP itself ignores. Resolve their
/// documented framing using the actual file epoch rather than probing payload bytes as headers.
pub(super) fn props_version<'a>(c: &mut Cursor<'a>, version: Option<u32>) -> Result<Vec<Prop<'a>>> {
    props_version_with_interrupt(c, version, &photocraft_raster::Interrupt::NONE)
}

/// The same bounded property framing, with cancellation between records for header-only callers.
pub(super) fn props_version_with_interrupt<'a>(c: &mut Cursor<'a>, version: Option<u32>, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Vec<Prop<'a>>> {
    props_charged(c, version, true, ctl, &mut |_| Ok(()))
}

/// Only indexed images consume a palette. Unused palette properties keep their
/// declared framing, so corrupt palette counts cannot reinterpret following headers.
pub(super) fn props_image_with_interrupt<'a>(c: &mut Cursor<'a>, version: u32, indexed: bool, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Vec<Prop<'a>>> {
    props_charged(c, Some(version), indexed, ctl, &mut |_| Ok(()))
}

/// Coverage inventory supplies a nonrefundable work debit before each framing step.
pub(super) fn props_with_work<'a>(
    c: &mut Cursor<'a>,
    ctl: &photocraft_raster::Interrupt<'_>,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<Vec<Prop<'a>>> {
    props_charged(c, None, true, ctl, charge)
}

#[cfg(test)]
thread_local! { pub(super) static PROPERTY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn props_charged<'a>(
    c: &mut Cursor<'a>,
    version: Option<u32>,
    palette_from_count: bool,
    ctl: &photocraft_raster::Interrupt<'_>,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<Vec<Prop<'a>>> {
    let mut out = Vec::new();
    loop {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        charge(8)?;
        #[cfg(test)]
        PROPERTY_READS.with(|count| count.set(count.get().saturating_add(1)));
        let offset = c.pos;
        let Some((kind, declared)) = property_header(c, out.len())? else { return Ok(out) };
        let mut framing = c.at(c.pos);
        let remaining = property_payload(&mut framing, kind, declared, version, palette_from_count)?;
        let len = (framing.pos - c.pos).checked_add(remaining).ok_or_else(|| invalid("property size overflow"))?;
        charge(len)?;
        let data = c.take(len)?;
        out.try_reserve(1).map_err(|_| invalid("not enough memory for XCF properties"))?;
        out.push(Prop { offset, kind, data });
    }
}

/// Framing shared by the in-memory decoder and the streaming geometry probe.
pub(super) trait PropertyInput {
    fn word(&mut self) -> Result<u32>;
    fn discard(&mut self, length: usize) -> Result<()>;
}

impl PropertyInput for Cursor<'_> {
    fn word(&mut self) -> Result<u32> {
        self.u32()
    }

    fn discard(&mut self, length: usize) -> Result<()> {
        self.skip(length)
    }
}

pub(super) fn property_header(input: &mut impl PropertyInput, count: usize) -> Result<Option<(u32, usize)>> {
    let kind = input.word()?;
    let declared = input.word()? as usize;
    if kind == 0 {
        return Ok(None);
    }
    if count >= 4096 {
        return Err(invalid("too many properties"));
    }
    Ok(Some((kind, declared)))
}

/// Consume only the bytes needed to determine framing. Return the payload bytes
/// still unread, so the geometry probe can inspect DPI without retaining other properties.
pub(super) fn property_payload(input: &mut impl PropertyInput, kind: u32, declared: usize, version: Option<u32>, indexed: bool) -> Result<usize> {
    let (length, prefix) = match (kind, version) {
        // Old XCFs can declare n+4 for 3n+4 bytes. Non-indexed images do not consume this palette.
        (1, _) if indexed => {
            let colors = input.word()? as usize;
            let length = colors.checked_mul(3).and_then(|n| n.checked_add(4)).ok_or_else(|| invalid("colormap overflow"))?;
            (length, 4)
        }
        // Both variants fit MAX_PAYLOAD because each individual string is bounded.
        (24, Some(version)) => {
            input.discard(8)?;
            for _ in 0..if version < 21 { 5 } else { 3 } {
                discard_property_string(input)?;
            }
            return Ok(0);
        }
        // PROP_LINK_LAYER can declare four bytes too few (#8857).
        (48, _) => {
            input.discard(4)?;
            discard_property_string(input)?;
            input.discard(8)?;
            return Ok(0);
        }
        _ => (declared, 0),
    };
    if length > MAX_PAYLOAD {
        return Err(invalid(format!("property {kind} declares {length} bytes")));
    }
    Ok(length - prefix)
}

fn discard_property_string(input: &mut impl PropertyInput) -> Result<()> {
    let length = input.word()? as usize;
    if length > MAX_STRING {
        return Err(invalid(format!("a string of {length} bytes exceeds the limit")));
    }
    input.discard(length)
}

pub(super) fn be_u32(b: &[u8]) -> Option<u32> {
    b.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

pub(super) fn be_i32(b: &[u8]) -> Option<i32> {
    be_u32(b).map(|v| v as i32)
}

pub(super) fn be_f32(b: &[u8]) -> Option<f32> {
    be_u32(b).map(f32::from_bits)
}

/// Parse optional records without copying names or unused payloads. A malformed tail leaves
/// its valid prefix available; cancellation is always an import failure, never optional data.
#[cfg(test)]
pub(super) fn scan_parasites<'a>(data: &'a [u8], ctl: &photocraft_raster::Interrupt<'_>, mut visit: impl FnMut(&'a [u8], &'a [u8])) -> Result<bool> {
    let status = scan_parasites_with_work(data, &mut ParasiteWork::default(), ctl, |record| {
        visit(record.name, record.payload);
        Ok(true)
    })?;
    Ok(status.malformed)
}

/// File-wide scan work is independent of storage refunds and of individual owner offsets.
pub(super) struct ParasiteWork {
    remaining: usize,
}

impl Default for ParasiteWork {
    fn default() -> Self {
        // Room for ordinary prerequisite, native-metadata, flag and text-selection
        // passes over the shared metadata allowance, while aliases remain bounded.
        Self { remaining: super::MAX_METADATA_BYTES.saturating_mul(4) }
    }
}

impl ParasiteWork {
    fn charge(&mut self, bytes: usize) -> bool {
        super::work_budget::WorkBudget::new(&mut self.remaining).exhaust_on_reject(bytes)
    }

    #[cfg(test)]
    pub(super) fn with_limit(remaining: usize) -> Self {
        Self { remaining }
    }

    #[cfg(test)]
    pub(super) fn remaining(&self) -> usize {
        self.remaining
    }
}

pub(super) struct ParasiteRecord<'a> {
    pub(super) name: &'a [u8],
    pub(super) payload: &'a [u8],
    pub(super) raw: &'a [u8],
    pub(super) start: usize,
}

#[derive(Debug)]
pub(super) struct ParasiteScan {
    pub(super) malformed: bool,
    pub(super) limited: bool,
    pub(super) stopped_at: usize,
}

#[cfg(test)]
thread_local! { pub(super) static PARASITE_RECORD_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Debit framing, names and payloads before parsing or visiting. A false callback
/// stops an unnecessary report scan; selectors return true to preserve final-value order.
pub(super) fn scan_parasites_with_work<'a>(
    data: &'a [u8],
    work: &mut ParasiteWork,
    ctl: &photocraft_raster::Interrupt<'_>,
    mut visit: impl FnMut(ParasiteRecord<'a>) -> Result<bool>,
) -> Result<ParasiteScan> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    while c.pos < data.len() {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        let start = c.pos;
        if !work.charge(24) {
            return Ok(ParasiteScan { malformed: false, limited: true, stopped_at: start });
        }
        #[cfg(test)]
        PARASITE_RECORD_READS.with(|count| count.set(count.get().saturating_add(1)));
        let record = (|| {
            let length = c.u32()? as usize;
            if length > MAX_STRING {
                return Err(invalid("parasite name exceeds the string limit"));
            }
            if length > data.len().saturating_sub(c.pos) {
                return Err(invalid("truncated parasite name"));
            }
            if !work.charge(length) {
                return Ok(None);
            }
            let name = c.take(length)?;
            let name = name.strip_suffix(&[0]).unwrap_or(name);
            c.u32()?; // Inert flags never grant authority.
            let length = c.u32()? as usize;
            if length > data.len().saturating_sub(c.pos) {
                return Err(invalid("truncated parasite payload"));
            }
            if !work.charge(length) {
                return Ok(None);
            }
            Ok::<_, super::IoError>(Some((name, c.take(length)?)))
        })();
        match record {
            Ok(Some((name, payload))) => {
                let raw = data.get(start..c.pos).ok_or_else(|| invalid("parasite record bounds"))?;
                if !visit(ParasiteRecord { name, payload, raw, start })? {
                    return Ok(ParasiteScan { malformed: false, limited: false, stopped_at: c.pos });
                }
            }
            Ok(None) => return Ok(ParasiteScan { malformed: false, limited: true, stopped_at: start }),
            Err(_) => return Ok(ParasiteScan { malformed: true, limited: false, stopped_at: start }),
        }
    }
    // Even an empty/spent scan must still respect cancellation.
    ctl.check().map_err(|_| super::IoError::Cancelled)?;
    Ok(ParasiteScan { malformed: false, limited: false, stopped_at: c.pos })
}

#[cfg(test)]
pub(super) const MAX_PARASITES: usize = 4096;
#[cfg(test)]
const PARASITE_SCRATCH_BYTES: usize = 8 << 20;

#[cfg(test)]
pub(super) struct Parasites {
    pub(super) values: Vec<(String, Vec<u8>)>,
    pub(super) limited: bool,
    pub(super) malformed: bool,
}

/// Bound the distinct-name index before reserving it, then copy only final winning values.
/// Scanning continues after the cap so later duplicates of retained slots still win. Borrowed
/// names/payloads avoid owning one copy per record, even for a flood of replacements.
#[cfg(test)]
pub(super) fn collect_parasites(data: &[u8], allowance: usize, max_records: usize, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Parasites> {
    let mut remaining = allowance;
    let mut records: Vec<(&[u8], &[u8])> = Vec::new();
    let mut index: HashMap<&[u8], usize> = HashMap::new();
    let mut limited = false;
    // Cover Vec slots, hash buckets/control bytes and allocator slack before either reserve.
    const SLOT_BYTES: usize = 192;
    let malformed = scan_parasites(data, ctl, |name, payload| {
        if let Some(&slot) = index.get(name) {
            if let Some(record) = records.get_mut(slot) {
                *record = (name, payload);
            }
            return;
        }
        if records.len() >= max_records.min(MAX_PARASITES) || remaining < SLOT_BYTES || records.try_reserve_exact(1).is_err() || index.try_reserve(1).is_err() {
            limited = true;
            return;
        }
        remaining -= SLOT_BYTES;
        index.insert(name, records.len());
        records.push((name, payload));
    })?;
    let mut values = Vec::new();
    for (name, payload) in records {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        // Lossy decoding can expand each source byte to a three-byte replacement character.
        let bytes = name.len().saturating_mul(3).saturating_add(payload.len());
        if bytes > remaining || values.try_reserve_exact(1).is_err() {
            limited = true;
            continue;
        }
        remaining -= bytes;
        values.push((String::from_utf8_lossy(name).into_owned(), payload.to_vec()));
    }
    Ok(Parasites { values, limited, malformed })
}

/// Test compatibility for bounded final-value collection; production selectors borrow records.
#[cfg(test)]
pub(super) fn parasites(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    collect_parasites(data, PARASITE_SCRATCH_BYTES, MAX_PARASITES, &photocraft_raster::Interrupt::NONE).map(|records| records.values).unwrap_or_default()
}

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel.
pub(super) fn decode_tile(c: &Cursor<'_>, at: usize, end: usize, compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
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
pub(super) fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    // XCF tiles contain at most 64x64 pixels, four channels and eight bytes
    // per sample. This helper also receives direct calls, outside hierarchy's
    // validated dimensions, so reject hostile sizes before allocating.
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("RLE tile size overflow"))?;
    if pixels == 0 || pixels > (super::TILE as usize) * (super::TILE as usize) || !(1..=32).contains(&bpp) {
        return Err(invalid("RLE tile dimensions or bytes per pixel are out of range"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(expected).map_err(|_| invalid("RLE tile allocation failed"))?;
    out.resize(expected, 0);
    let mut cursor = Cursor { b: input, pos: 0, wide: false };
    for b in 0..bpp {
        let mut written = 0usize;
        while written < pixels {
            let op = cursor.take(1)?.first().copied().ok_or_else(|| invalid("an RLE tile is cut short"))?;
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 | 128 => {
                    let length = cursor.take(2)?;
                    let high = length.first().copied().ok_or_else(|| invalid("an RLE tile is cut short"))?;
                    let low = length.get(1).copied().ok_or_else(|| invalid("an RLE tile is cut short"))?;
                    (usize::from(u16::from_be_bytes([high, low])), op == 128)
                }
                _ => (256 - usize::from(op), true),
            };
            if run == 0 {
                return Err(invalid("an RLE run has zero length"));
            }
            let end = written.checked_add(run).filter(|end| *end <= pixels).ok_or_else(|| invalid("an RLE run crosses the end of its tile"))?;
            let bytes = cursor.take(if verbatim { run } else { 1 })?;
            for i in written..end {
                let value = bytes.get(if verbatim { i - written } else { 0 }).copied().ok_or_else(|| invalid("an RLE tile is cut short"))?;
                let index = i.checked_mul(bpp).and_then(|index| index.checked_add(b)).ok_or_else(|| invalid("RLE destination offset overflow"))?;
                *out.get_mut(index).ok_or_else(|| invalid("RLE destination is outside its tile"))? = value;
            }
            written = end;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod rle_tests;
