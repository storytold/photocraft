//! Bounded transport probing. Only an inflated XCF signature claims an unrelated suffix.
use std::io::{self, BufRead, Read};

use photocraft_raster::Interrupt;

use super::{IoError, MAGIC, MAX_INFLATED, Result, invalid};

const CHUNK: usize = 32 * 1024;
const XZ_MEMORY_KIB: u32 = 64 * 1024;

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

/// Recognizes supported compressed XCF wrappers and applies the standard inflated-size limit.
pub(crate) fn decode(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_with_limit(name, bytes, ctl, MAX_INFLATED)
}

/// Decodes a recognized wrapper under explicit output and decoder-memory limits, propagating cancellation and
/// malformed streams.
fn decode_with_limit(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize) -> Result<Option<Vec<u8>>> {
    decode_inner(name, bytes, ctl, limit, false)
}

/// Inflates only the largest fixed XCF header (v4+: signature, version, canvas, precision).
/// This probe does not validate the stream's remainder; the actual import still does that.
pub(crate) fn header_prefix(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_inner(name, bytes, ctl, MAGIC.len() + 5 + 4 * 4, true)
}

fn decode_inner(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize, prefix_only: bool) -> Result<Option<Vec<u8>>> {
    let compression = if bytes.starts_with(&[0x1f, 0x8b]) {
        "gzip"
    } else if bytes.starts_with(b"BZh") {
        "bzip2"
    } else if bytes.starts_with(b"\xfd7zXZ\0") {
        "XZ"
    } else {
        return Ok(None);
    };
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let declared = [".xcf.gz", ".xcfgz", ".xcf.bz2", ".xcfbz2", ".xcf.xz", ".xcfxz"].iter().any(|suffix| lower.ends_with(suffix));
    let mut input = Input { remaining: bytes, ctl: *ctl };
    let mut decoder: Box<dyn Read + '_> = match compression {
        "gzip" => Box::new(flate2::bufread::MultiGzDecoder::new(&mut input)),
        "bzip2" => Box::new(crabz2::reader(&mut input)),
        _ => Box::new(lzma_rust2::XzReader::new_mem_limit(&mut input, true, XZ_MEMORY_KIB)),
    };
    let mut prefix = [0; MAGIC.len()];
    if let Err(error) = decoder.read_exact(&mut prefix) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        return if declared { Err(invalid(format!("{compression}: {error}"))) } else { Ok(None) };
    }
    if prefix != MAGIC {
        return if declared { Err(invalid(format!("{compression} content is not a plain XCF file; recursive wrapping is unsupported"))) } else { Ok(None) };
    }
    let mut output = Vec::with_capacity(CHUNK.min(limit));
    if prefix.len() > limit {
        return Err(invalid("the compressed file exceeds the inflated input limit (1 GiB in normal imports)"));
    }
    output.extend_from_slice(&prefix);
    let mut chunk = [0; CHUNK];
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if prefix_only && output.len() == limit {
            return Ok(Some(output));
        }
        let capacity = if prefix_only { limit.saturating_sub(output.len()).min(CHUNK) } else { CHUNK };
        let target = chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid decoded output capacity"))?;
        let count = decoder.read(target).map_err(|error| if ctl.cancelled() { IoError::Cancelled } else { invalid(format!("{compression}: {error}")) })?;
        if count == 0 {
            break;
        }
        if output.len().checked_add(count).is_none_or(|size| size > limit) {
            return Err(invalid("the compressed file exceeds the inflated input limit (1 GiB in normal imports)"));
        }
        output.extend_from_slice(chunk.get(..count).ok_or_else(|| invalid("invalid decoded output length"))?);
    }
    // Readers verify every member's checks and attempt another header for trailing data.
    // XZ also accepts standard stream padding. Require complete source consumption.
    drop(decoder);
    if !prefix_only && !input.remaining.is_empty() {
        return Err(invalid(format!("unconsumed data after the {compression} stream")));
    }
    Ok(Some(output))
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
        assert!(decode_with_limit("limit.xcf.gz", &wrapped, &Interrupt::NONE, 128).is_err());
        assert_eq!(decode_with_limit("limit.xcf.gz", &wrapped, &Interrupt::NONE, bytes.len()).unwrap().unwrap(), bytes);
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
