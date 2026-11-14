//! Bounded transport probing. Only an inflated XCF signature claims an unrelated suffix.
use std::io::{self, BufRead, Read};

use photocraft_raster::Interrupt;

use super::{IoError, MAGIC, MAX_INFLATED, Result, invalid};

const CHUNK: usize = 32 * 1024;
const XZ_MEMORY_KIB: u32 = 64 * 1024;

#[cfg(test)]
thread_local! { pub(crate) static DECODER_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(crate) struct DecodedImport {
    pub(crate) is_xcf: bool,
    pub(crate) bytes: Vec<u8>,
}

/// Reuse the probe's decoder for SVGZ, including misnamed SVG signatures. Other
/// inner formats remain unclaimed and use only the ordinary filename fallback.
pub(crate) fn decode_import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, allow_svg: bool) -> Result<Option<DecodedImport>> {
    decode_stream(name, bytes, ctl, MAX_INFLATED, false, allow_svg)
}

mod bzip;

#[cfg(test)]
mod gzip_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod routing_review_tests;

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

struct GzipPolicy {
    strict: std::cell::Cell<bool>,
    verified_output: std::cell::Cell<usize>,
    rollback: std::cell::Cell<Option<usize>>,
}

/// XCF requires verified members and terminal zero padding. SVGZ retains its
/// existing policy: ignore nonmember tails and discard a failed later member.
/// The prefix probe selects the policy without restarting the decoder.
struct Gzip<'p, R: BufRead> {
    member: Option<flate2::bufread::GzDecoder<R>>,
    policy: &'p GzipPolicy,
    total_output: usize,
    verified_member: bool,
}

impl<'p, R: BufRead> Gzip<'p, R> {
    fn new(input: R, policy: &'p GzipPolicy) -> Self {
        Self { member: Some(flate2::bufread::GzDecoder::new(input)), policy, total_output: 0, verified_member: false }
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
                Err(_) if !self.policy.strict.get() && self.verified_member => {
                    self.policy.rollback.set(Some(self.policy.verified_output.get()));
                    self.member = None;
                    return Ok(0);
                }
                Err(error) => return Err(error),
            };
            if count != 0 {
                self.total_output = self.total_output.checked_add(count).ok_or_else(|| io::Error::other("gzip output count overflow"))?;
                return Ok(count);
            }
            // Single-member EOF means its CRC and size trailer were checked.
            self.verified_member = true;
            self.policy.verified_output.set(self.total_output);
            let member = self.member.take().ok_or_else(|| io::Error::other("missing gzip member"))?;
            let mut input = member.into_inner();
            let next = input.fill_buf()?;
            if next.is_empty() {
                return Ok(0);
            }
            if !self.policy.strict.get() && !next.starts_with(&[0x1f, 0x8b]) {
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
    let declared = [".xcf.gz", ".xcfgz", ".xcf.bz2", ".xcfbz2", ".xcf.xz", ".xcfxz"].iter().any(|suffix| lower.ends_with(suffix));
    let declared_svg = lower.ends_with(".svg") || lower.ends_with(".svgz");
    let mut input = Input { remaining: bytes, ctl: *ctl };
    let gzip_policy = GzipPolicy { strict: std::cell::Cell::new(!allow_svg), verified_output: std::cell::Cell::new(0), rollback: std::cell::Cell::new(None) };
    #[cfg(test)]
    DECODER_STARTS.with(|starts| starts.set(starts.get() + 1));
    let mut decoder: Box<dyn Read + '_> = match compression {
        "gzip" => Box::new(Gzip::new(&mut input, &gzip_policy)),
        "bzip2" => Box::new(bzip::Reader::new(&mut input)),
        _ => Box::new(lzma_rust2::XzReader::new_mem_limit(&mut input, true, XZ_MEMORY_KIB)),
    };
    let mut prefix = [0; MAGIC.len()];
    let mut prefix_len = 0usize;
    while prefix_len < prefix.len() {
        let target = prefix.get_mut(prefix_len..).ok_or_else(|| invalid("invalid transport prefix capacity"))?;
        match decoder.read(target) {
            Ok(0) => break,
            Ok(count) => prefix_len += count,
            Err(error) => {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                return if declared {
                    Err(invalid(format!("{compression}: {error}")))
                } else if allow_svg {
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
        gzip_policy.strict.set(true);
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
    let mut output = Vec::with_capacity(CHUNK.min(limit));
    if prefix_len > limit {
        return Err(oversized());
    }
    output.extend_from_slice(prefix.get(..prefix_len).ok_or_else(|| invalid("invalid transport prefix length"))?);
    let mut chunk = [0; CHUNK];
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if prefix_only && output.len() == limit {
            return Ok(Some(DecodedImport { is_xcf, bytes: output }));
        }
        let capacity = if prefix_only { limit.saturating_sub(output.len()).min(CHUNK) } else { CHUNK };
        let target = chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid decoded output capacity"))?;
        let count = decoder.read(target).map_err(|error| {
            if ctl.cancelled() {
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
        if output.len().checked_add(count).is_none_or(|size| size > limit) {
            return Err(oversized());
        }
        output.extend_from_slice(chunk.get(..count).ok_or_else(|| invalid("invalid decoded output length"))?);
    }
    // Readers verify every member and consume only their admitted terminal padding
    // (zero bytes for gzip, standard stream padding for XZ). No unchecked tail remains.
    drop(decoder);
    // A cancellation while checking a later gzip member must never turn into the
    // SVGZ compatibility fallback that discards a damaged later member.
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(verified) = gzip_policy.rollback.get() {
        output.truncate(verified);
    }
    if !prefix_only && !input.remaining.is_empty() && is_xcf {
        let message = format!("unconsumed data after the {compression} stream");
        return Err(if is_xcf { invalid(message) } else { IoError::Svg(message) });
    }
    if !is_xcf && !declared_svg {
        // Sniff already-plain bytes. Decoding only a fixed prefix preserves the shared
        // sniffer's UTF-8/UTF-16/Latin-1 policy without another gzip reader or large text copy.
        let little_utf16 = output.starts_with(&[0xff, 0xfe]);
        let big_utf16 = output.starts_with(&[0xfe, 0xff]);
        // The shared sniffer reads 4096 UTF-8 bytes. Include the UTF-16 BOM plus
        // 4096 code units; a cut high surrogate belongs to the uninspected tail,
        // not to an invalid source. The complete bytes still go to the real parser.
        let limit = if little_utf16 || big_utf16 { 2 + 2 * 4096 } else { 8192 };
        let mut head = output.get(..output.len().min(limit)).ok_or_else(|| invalid("invalid SVG signature prefix"))?;
        if head.len() < output.len() && (little_utf16 || big_utf16) {
            let last = match head.get(head.len().saturating_sub(2)..) {
                Some([a, b]) if little_utf16 => u16::from_le_bytes([*a, *b]),
                Some([a, b]) => u16::from_be_bytes([*a, *b]),
                _ => 0,
            };
            if (0xd800..=0xdbff).contains(&last) {
                head = head.get(..head.len().saturating_sub(2)).ok_or_else(|| invalid("invalid UTF-16 signature prefix"))?;
            }
        }
        let svg = crate::svg::is_svg(&output) || craft_svg::svgz::decode(head).is_ok_and(|text| crate::svg::is_svg(text.as_bytes()));
        if !svg {
            return Ok(None);
        }
    }
    Ok(Some(DecodedImport { is_xcf, bytes: output }))
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
