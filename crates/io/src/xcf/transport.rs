//! Bounded transport probing. Only an inflated XCF signature claims an unrelated suffix.
use std::io::{self, BufRead, Read};

use photocraft_raster::Interrupt;

use super::{IoError, MAGIC, MAX_INFLATED, Result, invalid};

const CHUNK: usize = 32 * 1024;
const XZ_MAX_DICTIONARY_BYTES: u32 = 64 * 1024 * 1024;
// Include the BOM, 4096 UTF-16 code units, and a possible trailing surrogate.
const SVG_PROBE_BYTES: usize = 2 + 2 * 4096 + 2;

#[cfg(not(test))]
use self::make_compressed_reader as compressed_reader;
#[cfg(test)]
mod decoder_test_support;
#[cfg(test)]
use decoder_test_support::{DECODER_STARTS, compressed_reader};

pub(crate) struct DecodedImport {
    pub(crate) is_xcf: bool,
    pub(crate) bytes: Vec<u8>,
}

/// Reuse the probe's decoder for SVGZ, including misnamed SVG signatures. Other
/// inner formats remain unclaimed and use only the ordinary filename fallback.
pub(crate) fn decode_import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, allow_svg: bool) -> Result<Option<DecodedImport>> {
    decode_stream(name, bytes, ctl, MAX_INFLATED, false, allow_svg)
}

/// Recognize SVG inside a gzip envelope without inflating its remaining drawing.
/// This is a cancellable worker probe, not a complete SVG or envelope validation.
pub(crate) fn svg_prefix(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_stream(name, bytes, ctl, SVG_PROBE_BYTES, true, true)
        .map(|decoded| decoded.filter(|decoded| !decoded.is_xcf && svg_signature(&decoded.bytes)).map(|decoded| decoded.bytes))
}

mod bzip;
mod xz;

#[cfg(test)]
mod bzip_member_tests;
#[cfg(test)]
mod geometry_prefix_tests;
#[cfg(test)]
mod gzip_tests;
#[cfg(test)]
mod prefix_tests;
#[cfg(test)]
mod routing_tests;

/// Checks cancellation during headers, empty members and compressed input consumption,
/// as well as between output chunks. A long header must not bypass cooperative cancellation.
struct Input<'a, 'b> {
    remaining: &'a [u8],
    ctl: Interrupt<'b>,
}

impl BufRead for Input<'_, '_> {
    /// Exposes remaining compressed input only after checking import cancellation.
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.ctl.check().map_err(io::Error::other)?;
        Ok(self.remaining.get(..self.remaining.len().min(CHUNK)).unwrap_or_default())
    }

    /// Advances the compressed input slice safely without indexing beyond its remaining bytes.
    fn consume(&mut self, amount: usize) {
        self.remaining = self.remaining.get(amount..).unwrap_or_default();
    }
}

impl Read for Input<'_, '_> {
    /// Copies a bounded portion of compressed input through the cancellation-aware buffered reader.
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let count = output.len().min(input.len());
        let source = input.get(..count).ok_or_else(|| io::Error::other("invalid input range"))?;
        let target = output.get_mut(..count).ok_or_else(|| io::Error::other("invalid output range"))?;
        target.copy_from_slice(source);
        self.consume(count);
        Ok(count)
    }
}

/// XCF requires verified members and terminal zero padding. SVGZ ignores
/// nonmember tails, while every recognized member must pass its checksum.
/// The prefix probe selects the policy without restarting the decoder.
struct Gzip<'p, R: BufRead> {
    member: Option<flate2::bufread::GzDecoder<R>>,
    strict: &'p std::cell::Cell<bool>,
    verified_member: bool,
}

impl<'p, R: BufRead> Gzip<'p, R> {
    fn new(input: R, strict: &'p std::cell::Cell<bool>) -> Self {
        Self { member: Some(flate2::bufread::GzDecoder::new(input)), strict, verified_member: false }
    }
}

impl<R: BufRead> Read for Gzip<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let Some(member) = &mut self.member else { return Ok(0) };
            let count = match member.read(output) {
                Ok(count) => count,
                Err(error)
                    if !self.strict.get()
                        && self.verified_member
                        && member.header().is_none()
                        && matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidInput) =>
                {
                    self.member = None;
                    return Ok(0);
                }
                Err(error) => return Err(error),
            };
            if count != 0 {
                return Ok(count);
            }
            // Single-member EOF means its CRC and size trailer were checked.
            self.verified_member = true;
            let member = self.member.take().ok_or_else(|| io::Error::other("missing gzip member"))?;
            let mut input = member.into_inner();
            let next = input.fill_buf()?;
            if next.is_empty() {
                return Ok(0);
            }
            if !self.strict.get() && !next.starts_with(&[0x1f, 0x8b]) {
                return Ok(0);
            }
            if next.first() == Some(&0) {
                loop {
                    let padding = input.fill_buf()?;
                    if padding.is_empty() {
                        return Ok(0);
                    }
                    if padding.iter().any(|byte| *byte != 0) {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "nonzero data after gzip padding"));
                    }
                    let count = padding.len();
                    input.consume(count);
                }
            }
            // Let the gzip parser validate all other tails, including partial
            // magic/header bytes, rather than mistaking them for benign padding.
            self.member = Some(flate2::bufread::GzDecoder::new(input));
        }
    }
}

/// Recognizes supported compressed XCF wrappers and applies the standard inflated-size limit.
#[cfg(test)]
pub(crate) fn decode(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_inner(name, bytes, ctl, MAX_INFLATED, false)
}

/// Inject a small inflated-byte ceiling in transport tests without allocating the normal 1 GiB limit.
#[cfg(test)]
fn decode_with_limit(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize) -> Result<Option<Vec<u8>>> {
    decode_inner(name, bytes, ctl, limit, false)
}

/// Inflates only the largest fixed XCF header (v4+: signature, version, canvas, precision).
/// This probe does not validate the stream's remainder; the actual import still does that.
pub(crate) fn header_prefix(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_inner(name, bytes, ctl, MAGIC.len() + 5 + 4 * 4, true)
}

fn decode_inner(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize, prefix_only: bool) -> Result<Option<Vec<u8>>> {
    decode_stream(name, bytes, ctl, limit, prefix_only, false).map(|value| value.map(|value| value.bytes))
}

/// Construct the same bounded decoder for prefix queries and complete imports.
fn make_compressed_reader<'a, 'bytes: 'a, 'ctl: 'a>(
    kind: crate::ImportCompression,
    input: &'a mut Input<'bytes, 'ctl>,
    strict: &'a std::cell::Cell<bool>,
) -> Box<dyn Read + 'a> {
    match kind {
        crate::ImportCompression::Gzip => Box::new(Gzip::new(input, strict)),
        crate::ImportCompression::Bzip2 => Box::new(bzip::Reader::new(input)),
        // The decoder limit includes fixed LZMA2 buffers as well as the dictionary.
        // Admit the 64 MiB dictionary used by -9/-9e without admitting larger ones.
        crate::ImportCompression::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(input, true, lzma_rust2::lzma2_get_memory_usage(XZ_MAX_DICTIONARY_BYTES))),
    }
}

/// A geometry query owns no inflated document buffer. Limit both its total
/// metadata work and cancellation latency, including discarded property payloads.
struct PrefixRead<'a, 'ctl> {
    reader: &'a mut dyn Read,
    ctl: &'a Interrupt<'ctl>,
    left: usize,
}

impl Read for PrefixRead<'_, '_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.ctl.check().map_err(io::Error::other)?;
        if output.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            return Err(io::Error::other("XCF metadata exceeds the inflated prefix allowance"));
        }
        let capacity = output.len().min(self.left).min(CHUNK);
        let target = output.get_mut(..capacity).ok_or_else(|| io::Error::other("invalid metadata read range"))?;
        let count = self.reader.read(target)?;
        self.left = self.left.saturating_sub(count);
        Ok(count)
    }
}

/// Scan only the XCF prefix the caller needs, through one cancellable decoder.
/// The caller must stop after its metadata. Unread drawables, trailers and tails
/// are deliberately not validated here; complete import keeps its strict path.
pub(super) fn with_xcf_prefix<T>(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, scan: impl FnOnce(&mut dyn Read) -> Result<T>) -> Result<Option<T>> {
    with_xcf_prefix_limit(name, bytes, ctl, MAX_INFLATED, scan)
}

fn with_xcf_prefix_limit<T>(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize, scan: impl FnOnce(&mut dyn Read) -> Result<T>) -> Result<Option<T>> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if super::is_xcf(bytes) {
        let mut reader = bytes;
        let result = scan(&mut PrefixRead { reader: &mut reader, ctl, left: limit });
        ctl.check().map_err(|_| IoError::Cancelled)?;
        return result.map(Some);
    }
    let Some(kind) = crate::import_compression(bytes) else { return Ok(None) };
    let mut input = Input { remaining: bytes, ctl: *ctl };
    let strict = std::cell::Cell::new(true);
    let mut decoder = compressed_reader(kind, &mut input, &strict);
    let mut magic = [0; MAGIC.len()];
    let result = decoder.read_exact(&mut magic);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Err(error) = result {
        return if crate::is_xcf_input_name(name) { Err(invalid(format!("compressed XCF prefix: {error}"))) } else { Ok(None) };
    }
    if magic != MAGIC {
        return if crate::is_xcf_input_name(name) {
            Err(invalid("compressed content is not a plain XCF file; recursive wrapping is unsupported"))
        } else {
            Ok(None)
        };
    }
    // Replay only the already-read magic, not another decoder. The scanner then
    // sees the same framing as an uncompressed XCF and pays for every byte once.
    let mut reader = magic.as_slice().chain(decoder);
    let result = scan(&mut PrefixRead { reader: &mut reader, ctl, left: limit });
    ctl.check().map_err(|_| IoError::Cancelled)?;
    result.map(Some)
}

fn decode_stream(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize, prefix_only: bool, allow_svg: bool) -> Result<Option<DecodedImport>> {
    let Some(kind) = crate::import_compression(bytes) else { return Ok(None) };
    let compression = match kind {
        crate::ImportCompression::Gzip => "gzip",
        crate::ImportCompression::Bzip2 => "bzip2",
        crate::ImportCompression::Xz => "XZ",
    };
    // SVGZ is a gzip envelope; other compressed formats may claim only actual XCF bytes.
    let allow_svg = allow_svg && kind == crate::ImportCompression::Gzip;
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let declared = crate::is_xcf_input_name(name) && !lower.ends_with(".xcf");
    let declared_svg = lower.ends_with(".svg") || lower.ends_with(".svgz");
    let mut input = Input { remaining: bytes, ctl: *ctl };
    let strict = std::cell::Cell::new(!allow_svg);
    let mut decoder = compressed_reader(kind, &mut input, &strict);
    let mut prefix = [0; MAGIC.len()];
    let mut prefix_len = 0usize;
    while prefix_len < prefix.len() {
        let target = prefix.get_mut(prefix_len..).ok_or_else(|| invalid("invalid transport prefix capacity"))?;
        match decoder.read(target) {
            Ok(0) => break,
            Ok(count) => prefix_len += count,
            Err(error) => {
                check_decoder_error(&error, ctl)?;
                return if declared {
                    Err(invalid(format!("{compression}: {error}")))
                } else if allow_svg && (declared_svg || svg_signature(prefix.get(..prefix_len).unwrap_or_default())) {
                    Err(IoError::Svg(format!("{compression}: {error}")))
                } else {
                    Ok(None)
                };
            }
        }
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let is_xcf = prefix_len == prefix.len() && prefix == MAGIC;
    if is_xcf {
        strict.set(true);
    }
    if !is_xcf && (declared || !allow_svg) {
        return if declared { Err(invalid(format!("{compression} content is not a plain XCF file; recursive wrapping is unsupported"))) } else { Ok(None) };
    }
    // The SVG parser normally unwraps once before parsing plain XML. Its reused input
    // must not trigger that unwrap again and admit a recursively compressed document.
    if !is_xcf && crate::import_compression(prefix.get(..prefix_len).ok_or_else(|| invalid("invalid transport prefix length"))?).is_some() {
        return Err(IoError::Svg("compressed SVG contains another compressed envelope; recursive wrapping is unsupported".into()));
    }
    let limit = if is_xcf { limit } else { limit.min(usize::try_from(crate::svg::MAX_INFLATED_BYTES).unwrap_or(usize::MAX)) };
    let oversized = || {
        if is_xcf {
            invalid("the compressed file exceeds the inflated input limit (1 GiB in normal imports)")
        } else {
            IoError::Svg(format!("compressed SVG inflates past {limit} bytes"))
        }
    };
    let mut output = Vec::new();
    let prefix = prefix.get(..prefix_len).ok_or_else(|| invalid("invalid transport prefix length"))?;
    append_output(&mut output, prefix, limit, oversized)?;
    if !is_xcf && !declared_svg && !probe_svg(decoder.as_mut(), &mut output, ctl, limit)? {
        return Ok(None);
    }
    let mut chunk = [0; CHUNK];
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if prefix_only && output.len() == limit {
            return Ok(Some(DecodedImport { is_xcf, bytes: output }));
        }
        let capacity = if prefix_only { limit.saturating_sub(output.len()).min(CHUNK) } else { CHUNK };
        let target = chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid decoded output capacity"))?;
        let count = decoder.read(target).map_err(|error| {
            if check_decoder_error(&error, ctl).is_err() {
                IoError::Cancelled
            } else if is_xcf {
                invalid(format!("{compression}: {error}"))
            } else {
                IoError::Svg(format!("{compression}: {error}"))
            }
        })?;
        if count == 0 {
            break;
        }
        let bytes = chunk.get(..count).ok_or_else(|| invalid("invalid decoded output length"))?;
        append_output(&mut output, bytes, limit, oversized)?;
    }
    // Readers verify every member and consume admitted terminal padding. The pinned
    // XZ reader may accept unaligned zero padding at EOF. XZ streams and inter-stream
    // padding are each four-byte aligned, so the full envelope must be aligned too.
    drop(decoder);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !prefix_only && is_xcf {
        if !input.remaining.is_empty() {
            return Err(invalid(format!("unconsumed data after the {compression} stream")));
        }
        if kind == crate::ImportCompression::Xz {
            xz::validate_envelope(bytes, output.len(), ctl)?;
        }
    }
    if !is_xcf && !declared_svg && !svg_signature(&output) {
        return Ok(None);
    }
    Ok(Some(DecodedImport { is_xcf, bytes: output }))
}

#[cfg(test)]
thread_local! {
    static OUTPUT_RESERVATION_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}

/// Reserve before copying. Grow geometrically within the transport limit.
fn append_output(output: &mut Vec<u8>, bytes: &[u8], limit: usize, oversized: impl FnOnce() -> IoError) -> Result<()> {
    let Some(end) = output.len().checked_add(bytes.len()).filter(|size| *size <= limit) else { return Err(oversized()) };
    if end > output.capacity() {
        let capacity = end.max(CHUNK).max(output.capacity().saturating_mul(2)).min(limit);
        let exhausted = || IoError::OutOfMemory(format!("unable to reserve {capacity} bytes for inflated image data"));
        #[cfg(test)]
        if OUTPUT_RESERVATION_LIMIT.with(|cap| capacity > cap.get()) {
            return Err(exhausted());
        }
        output.try_reserve_exact(capacity - output.len()).map_err(|_| exhausted())?;
    }
    output.extend_from_slice(bytes);
    Ok(())
}

/// Unknown gzip bodies need only a bounded signature before filename fallback.
/// A claimed SVG keeps this decoder, including its existing member/tail policy.
fn probe_svg(decoder: &mut dyn Read, output: &mut Vec<u8>, ctl: &Interrupt<'_>, limit: usize) -> Result<bool> {
    let probe_limit = SVG_PROBE_BYTES.min(limit);
    let mut chunk = [0; SVG_PROBE_BYTES];
    while output.len() < probe_limit {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let capacity = probe_limit.saturating_sub(output.len());
        let target = chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid SVG probe capacity"))?;
        let count = match decoder.read(target) {
            Ok(count) => count,
            Err(error) => {
                check_decoder_error(&error, ctl)?;
                return if svg_signature(output) { Err(IoError::Svg(format!("gzip: {error}"))) } else { Ok(false) };
            }
        };
        if count == 0 {
            break;
        }
        let bytes = chunk.get(..count).ok_or_else(|| invalid("invalid SVG probe length"))?;
        append_output(output, bytes, limit, || IoError::Svg(format!("compressed SVG inflates past {limit} bytes")))?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(svg_signature(output))
}

fn check_decoder_error(error: &io::Error, ctl: &Interrupt<'_>) -> Result<()> {
    if error.get_ref().is_some_and(|cause| cause.is::<photocraft_raster::Cancelled>()) {
        return Err(IoError::Cancelled);
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Recognizes plain XML using the shared bounded UTF-8/UTF-16/Latin-1 policy.
fn svg_signature(output: &[u8]) -> bool {
    let little_utf16 = output.starts_with(&[0xff, 0xfe]);
    let big_utf16 = output.starts_with(&[0xfe, 0xff]);
    // A cut high surrogate belongs to the uninspected tail, not to an invalid
    // source. The complete bytes still go to the real SVG parser after claiming.
    let limit = if little_utf16 || big_utf16 { 2 + 2 * 4096 } else { 8192 };
    let mut head = output.get(..output.len().min(limit)).unwrap_or_default();
    if head.len() < output.len() && (little_utf16 || big_utf16) {
        let last = match head.get(head.len().saturating_sub(2)..) {
            Some([a, b]) if little_utf16 => u16::from_le_bytes([*a, *b]),
            Some([a, b]) => u16::from_be_bytes([*a, *b]),
            _ => 0,
        };
        if (0xd800..=0xdbff).contains(&last) {
            head = head.get(..head.len().saturating_sub(2)).unwrap_or_default();
        }
    }
    crate::svg::is_svg(output) || craft_svg::svgz::decode(head).is_ok_and(|text| crate::svg::is_svg(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Creates a small independent gzip wrapper used to exercise transport limits and header scanning.
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    /// Checks inflated output is rejected before an oversized append can exceed the admitted buffer.
    fn inflated_limit_is_checked_before_appending_output() {
        let bytes = [MAGIC, &[0; 1024]].concat();
        let wrapped = gzip(&bytes);
        assert!(decode_inner("limit.xcf.gz", &wrapped, &Interrupt::NONE, 128, false).is_err());
        assert_eq!(decode_inner("limit.xcf.gz", &wrapped, &Interrupt::NONE, bytes.len(), false).unwrap().unwrap(), bytes);
    }

    #[test]
    fn inflated_output_reservation_failure_is_typed() {
        let wrapped = gzip(&[MAGIC, &[0; CHUNK]].concat());
        let result = OUTPUT_RESERVATION_LIMIT.with(|cap| {
            let previous = cap.replace(CHUNK);
            let result = decode("pressure.xcf.gz", &wrapped, &Interrupt::NONE);
            cap.set(previous);
            result
        });
        assert!(matches!(result, Err(IoError::OutOfMemory(_))));
    }

    #[test]
    /// Verifies cancellation is honored even when gzip processing scans headers or empty members rather than
    /// producing pixels.
    fn cancellation_is_checked_while_scanning_empty_members_and_long_headers() {
        let checks = AtomicUsize::new(0);
        let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= 10;
        let ctl = Interrupt::cancel_only(&cancelled);
        let members = [gzip(&[]).repeat(100), gzip(MAGIC)].concat();
        assert!(matches!(decode("members.xcf.gz", &members, &ctl), Err(IoError::Cancelled)));

        let mut header = gzip(MAGIC);
        header[3] |= 8; // FNAME: a long zero-terminated header field, without changing the body.
        header.splice(10..10, [vec![b'x'; CHUNK * 20], vec![0]].concat());
        checks.store(0, Ordering::Relaxed);
        assert!(matches!(decode("header.xcf.gz", &header, &ctl), Err(IoError::Cancelled)));
    }
    /// A probe must stop at the header even when a large, compressible payload follows.
    #[test]
    fn header_probe_stops_before_large_payload_and_still_cancels_long_headers() {
        let bytes = [MAGIC, &[0; 21], &vec![0; 16 * 1024 * 1024]].concat();
        let wrapped = gzip(&bytes);
        let checks = AtomicUsize::new(0);
        let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= 100;
        let ctl = Interrupt::cancel_only(&cancelled);
        assert_eq!(header_prefix("large.xcf.gz", &wrapped, &ctl).unwrap().unwrap(), bytes[..30]);
        checks.store(0, Ordering::Relaxed);
        assert!(matches!(decode("large.xcf.gz", &wrapped, &ctl), Err(IoError::Cancelled)));

        let mut header = gzip(&bytes[..30]);
        header[3] |= 8;
        header.splice(10..10, [vec![b'x'; CHUNK * 200], vec![0]].concat());
        checks.store(0, Ordering::Relaxed);
        assert!(matches!(header_prefix("header.xcf.gz", &header, &ctl), Err(IoError::Cancelled)));
        checks.store(0, Ordering::Relaxed);
        let members = [gzip(&[]).repeat(1000), gzip(&bytes[..30])].concat();
        assert!(matches!(header_prefix("members.xcf.gz", &members, &ctl), Err(IoError::Cancelled)));
    }
}
