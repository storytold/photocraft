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
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.ctl.check().map_err(io::Error::other)?;
        Ok(self.remaining.get(..self.remaining.len().min(CHUNK)).unwrap_or_default())
    }

    fn consume(&mut self, amount: usize) {
        self.remaining = self.remaining.get(amount..).unwrap_or_default();
    }
}

impl Read for Input<'_, '_> {
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

pub(crate) fn decode(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>> {
    decode_with_limit(name, bytes, ctl, MAX_INFLATED)
}

fn decode_with_limit(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, limit: usize) -> Result<Option<Vec<u8>>> {
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
        let count = decoder.read(&mut chunk).map_err(|error| if ctl.cancelled() { IoError::Cancelled } else { invalid(format!("{compression}: {error}")) })?;
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
    if !input.remaining.is_empty() {
        return Err(invalid(format!("unconsumed data after the {compression} stream")));
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn inflated_limit_is_checked_before_appending_output() {
        let bytes = [MAGIC, &[0; 1024]].concat();
        let wrapped = gzip(&bytes);
        assert!(decode_with_limit("limit.xcf.gz", &wrapped, &Interrupt::NONE, 128).is_err());
        assert_eq!(decode_with_limit("limit.xcf.gz", &wrapped, &Interrupt::NONE, bytes.len()).unwrap().unwrap(), bytes);
    }

    #[test]
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
}
