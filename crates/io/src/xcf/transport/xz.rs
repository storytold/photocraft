use std::io::{self, Read, Seek, SeekFrom};

use photocraft_raster::Interrupt;

use super::{IoError, Result, check_decoder_error, invalid};

struct Envelope<'a, 'ctl> {
    cursor: io::Cursor<&'a [u8]>,
    ctl: Interrupt<'ctl>,
}

impl Read for Envelope<'_, '_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.ctl.check().map_err(io::Error::other)?;
        self.cursor.read(output)
    }
}

impl Seek for Envelope<'_, '_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.ctl.check().map_err(io::Error::other)?;
        self.cursor.seek(position)
    }
}

/// The streaming decoder validates blocks and CRCs. Check its remaining footer
/// Backward Size contract with the library's bounded seek-based Index parser.
pub(super) fn validate_envelope(bytes: &[u8], inflated: usize, ctl: &Interrupt<'_>) -> Result<()> {
    if !bytes.len().is_multiple_of(4) {
        return Err(invalid("XZ stream padding must be a multiple of four bytes"));
    }
    let envelope = Envelope { cursor: io::Cursor::new(bytes), ctl: *ctl };
    let declared = lzma_rust2::xz_uncompressed_size(envelope)
        .map_err(|error| check_decoder_error(&error, ctl).err().unwrap_or_else(|| invalid(format!("XZ stream Index: {error}"))))?;
    if declared != inflated as u64 {
        return Err(invalid("XZ Index uncompressed size does not match the decoded stream"));
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
