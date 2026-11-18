//! Smart-source geometry needs only canvas framing and image properties, never drawables.

use std::io::Read;

use super::{Interrupt, IoError, Result, invalid, metadata, read_header, read_version, transport, wire};

/// Unclaimed input stays available to the ordinary non-XCF geometry reader. This
/// prefix query stops at image-property END: it does not validate unread drawable
/// bytes, compression trailers or tails. Complete import still validates them.
pub(super) fn source_geometry(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<((f64, f64), f64)>> {
    transport::with_xcf_prefix(name, bytes, ctl, |reader| scan(reader, ctl))
}

fn scan(reader: &mut dyn Read, ctl: &Interrupt<'_>) -> Result<((f64, f64), f64)> {
    // Use the shared version/header validators, including legacy 26-byte headers.
    let mut bytes = [0; 30];
    read(reader, bytes.get_mut(..14).ok_or_else(|| invalid("invalid geometry header range"))?)?;
    let mut cursor = wire::Cursor { b: &bytes, pos: super::MAGIC.len(), wide: false };
    let version = read_version(&mut cursor)?;
    let length = if version >= 4 { 30 } else { 26 };
    read(reader, bytes.get_mut(14..length).ok_or_else(|| invalid("invalid geometry header range"))?)?;
    let (_, header) = read_header(bytes.get(..length).ok_or_else(|| invalid("invalid geometry header length"))?)?;
    let mut input = Stream(reader);
    let mut dpi = 72.;
    let mut count = 0;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some((kind, declared)) = wire::property_header(&mut input, count)? else {
            return Ok(((f64::from(header.width), f64::from(header.height)), f64::from(dpi)));
        };
        count += 1;
        let remaining = wire::property_payload(&mut input, kind, declared, Some(header.version), header.base_type == 2)?;
        if kind == 19 && remaining >= 8 {
            let mut pair = [0; 8];
            read(input.0, &mut pair)?;
            // Last valid pair wins, and extra bytes do not affect it.
            dpi = metadata::image_resolution(&[wire::Prop { offset: 0, kind, data: &pair }], dpi);
            wire::PropertyInput::discard(&mut input, remaining - pair.len())?;
        } else {
            wire::PropertyInput::discard(&mut input, remaining)?;
        }
    }
}

struct Stream<'a>(&'a mut dyn Read);

impl wire::PropertyInput for Stream<'_> {
    fn word(&mut self) -> Result<u32> {
        let mut bytes = [0; 4];
        read(self.0, &mut bytes)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn discard(&mut self, mut length: usize) -> Result<()> {
        let mut scratch = [0; 32 * 1024];
        while length != 0 {
            let count = length.min(scratch.len());
            read(self.0, scratch.get_mut(..count).ok_or_else(|| invalid("invalid geometry discard range"))?)?;
            length -= count;
        }
        Ok(())
    }
}

fn read(reader: &mut dyn Read, output: &mut [u8]) -> Result<()> {
    reader.read_exact(output).map_err(|error| invalid(format!("XCF geometry prefix: {error}")))
}

#[cfg(test)]
mod tests;
