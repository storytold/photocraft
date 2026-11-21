//! Smart-source geometry needs only canvas framing and image properties, never drawables.

use std::io::Read;

use super::{Interrupt, IoError, Result, invalid, is_xcf, metadata, read_header, read_version, transport, wire};

type Geometry = ((f64, f64), f64);

/// Unclaimed input stays available to the ordinary non-XCF geometry reader.
/// The usual prefix scan stops at image-property END. Competing palette boundaries
/// can require the complete envelope and drawable table framing.
pub(super) fn source_geometry(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<Geometry>> {
    match transport::with_xcf_prefix(name, bytes, ctl, |reader| scan(reader, ctl))? {
        None => Ok(None),
        Some(Some(geometry)) => Ok(Some(geometry)),
        Some(None) if is_xcf(bytes) => scan_authoritative(bytes, ctl).map(Some),
        Some(None) => {
            let source = transport::decode_import(name, bytes, ctl, false)?.ok_or_else(|| invalid("could not read the XCF palette boundary"))?;
            scan_authoritative(&source.bytes, ctl).map(Some)
        }
    }
}

/// Resolve competing property boundaries using the same image framing as full import.
fn scan_authoritative(bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Geometry> {
    let (mut cursor, header) = read_header(bytes)?;
    let properties = wire::props_image_with_interrupt(&mut cursor, header.version, header.base_type == 2, ctl)?;
    Ok(((f64::from(header.width), f64::from(header.height)), f64::from(metadata::image_resolution(&properties, 72.))))
}

fn scan(reader: &mut dyn Read, ctl: &Interrupt<'_>) -> Result<Option<Geometry>> {
    // Use the shared version/header validators, including legacy 26-byte headers.
    let mut bytes = [0; 30];
    read(reader, bytes.get_mut(..14).ok_or_else(|| invalid("invalid geometry header range"))?)?;
    let mut cursor = wire::Cursor { b: &bytes, pos: super::MAGIC.len(), wide: false };
    let version = read_version(&mut cursor)?;
    let length = if version >= 4 { 30 } else { 26 };
    read(reader, bytes.get_mut(14..length).ok_or_else(|| invalid("invalid geometry header range"))?)?;
    let (_, header) = read_header(bytes.get(..length).ok_or_else(|| invalid("invalid geometry header length"))?)?;
    let mut input = Stream::new(reader);
    let mut dpi = 72.;
    let mut count = 0;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some((kind, declared)) = wire::property_header(&mut input, count)? else {
            return Ok(Some(((f64::from(header.width), f64::from(header.height)), f64::from(dpi))));
        };
        count += 1;
        let remaining = wire::property_payload(&mut input, kind, declared, Some(header.version), header.base_type == 2, ctl)?;
        if input.unresolved_palette {
            return Ok(None);
        }
        if kind == 19 && remaining >= 8 {
            let mut pair = [0; 8];
            input.read_exact(&mut pair)?;
            // Last valid pair wins, and extra bytes do not affect it.
            dpi = metadata::image_resolution(&[wire::Prop { offset: 0, kind, data: &pair }], dpi);
            wire::PropertyInput::discard(&mut input, remaining - pair.len())?;
        } else {
            wire::PropertyInput::discard(&mut input, remaining)?;
        }
    }
}

struct Stream<'a> {
    reader: &'a mut dyn Read,
    buffered: Vec<u8>,
    consumed: usize,
    unresolved_palette: bool,
}

impl<'a> Stream<'a> {
    fn new(reader: &'a mut dyn Read) -> Self {
        Self { reader, buffered: Vec::new(), consumed: 0, unresolved_palette: false }
    }

    fn read_exact(&mut self, output: &mut [u8]) -> Result<()> {
        let pending = &self.buffered[self.consumed..];
        let count = pending.len().min(output.len());
        let (prefix, tail) = output.split_at_mut(count);
        prefix.copy_from_slice(&pending[..count]);
        self.consumed += count;
        if self.consumed == self.buffered.len() {
            self.buffered.clear();
            self.consumed = 0;
        }
        read(self.reader, tail)
    }
}

impl wire::PropertyInput for Stream<'_> {
    fn unresolved_palette_boundary(&mut self) {
        self.unresolved_palette = true;
    }

    fn word(&mut self) -> Result<u32> {
        let mut bytes = [0; 4];
        self.read_exact(&mut bytes)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn discard(&mut self, mut length: usize) -> Result<()> {
        let mut scratch = [0; 32 * 1024];
        while length != 0 {
            let count = length.min(scratch.len());
            self.read_exact(&mut scratch[..count])?;
            length -= count;
        }
        Ok(())
    }

    fn peek(&mut self, offset: usize, length: usize) -> Result<Option<&[u8]>> {
        let end = offset.checked_add(length).filter(|end| *end <= super::MAX_STRING + 16).ok_or_else(|| invalid("property lookahead exceeds its bound"))?;
        if self.consumed != 0 {
            self.buffered.drain(..self.consumed);
            self.consumed = 0;
        }
        self.buffered.try_reserve(end.saturating_sub(self.buffered.len())).map_err(|_| invalid("not enough memory for property lookahead"))?;
        let mut scratch = [0; 4096];
        while self.buffered.len() < end {
            let count = (end - self.buffered.len()).min(scratch.len());
            match self.reader.read(&mut scratch[..count]) {
                Ok(0) => return Ok(None),
                Ok(count) => self.buffered.extend_from_slice(&scratch[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(invalid(format!("XCF geometry prefix: {error}"))),
            }
        }
        Ok(self.buffered.get(offset..end))
    }
}

fn read(reader: &mut dyn Read, output: &mut [u8]) -> Result<()> {
    reader.read_exact(output).map_err(|error| invalid(format!("XCF geometry prefix: {error}")))
}

#[cfg(test)]
mod tests;
