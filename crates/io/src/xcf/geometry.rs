//! Smart-source geometry needs only canvas framing and image properties, never drawables.

use std::io::Read;

use super::{Interrupt, IoError, MAX_PAYLOAD, MAX_STRING, Result, invalid, metadata, read_header, read_version, transport, wire};

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
    let mut dpi = 72.;
    let mut count = 0usize;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let kind = word(reader)?;
        let declared = word(reader)? as usize;
        if kind == 0 {
            return Ok(((f64::from(header.width), f64::from(header.height)), f64::from(dpi)));
        }
        count = count.saturating_add(1);
        if count > 4096 {
            return Err(invalid("too many properties"));
        }
        match kind {
            // Match wire::props_version_with_interrupt's historical framing:
            // colormaps, user units and links do not trust the length word.
            1 => {
                let colors = word(reader)? as usize;
                let length = colors.checked_mul(3).and_then(|n| n.checked_add(4)).ok_or_else(|| invalid("colormap overflow"))?;
                payload_limit(kind, length)?;
                skip(reader, length.saturating_sub(4))?;
            }
            24 => user_unit(reader, header.version)?,
            48 => {
                skip(reader, 4)?;
                string(reader)?;
                skip(reader, 8)?;
            }
            _ => {
                payload_limit(kind, declared)?;
                if kind == 19 && declared >= 8 {
                    let mut pair = [0; 8];
                    read(reader, &mut pair)?;
                    // Reuse the importer policy: last valid pair wins, invalid
                    // pairs leave the preceding DPI intact, trailing bytes ignored.
                    dpi = metadata::image_resolution(&[wire::Prop { offset: 0, kind, data: &pair }], dpi);
                    skip(reader, declared.saturating_sub(pair.len()))?;
                } else {
                    skip(reader, declared)?;
                }
            }
        }
    }
}

fn payload_limit(kind: u32, length: usize) -> Result<()> {
    if length > MAX_PAYLOAD {
        return Err(invalid(format!("property {kind} declares {length} bytes")));
    }
    Ok(())
}

fn user_unit(reader: &mut dyn Read, version: u32) -> Result<()> {
    skip(reader, 8)?;
    for _ in 0..if version < 21 { 5 } else { 3 } {
        string(reader)?;
    }
    // The maximum five bounded strings and fixed fields fit MAX_PAYLOAD.
    Ok(())
}

fn string(reader: &mut dyn Read) -> Result<()> {
    let length = word(reader)? as usize;
    if length > MAX_STRING {
        return Err(invalid(format!("a string of {length} bytes exceeds the limit")));
    }
    skip(reader, length)
}

fn word(reader: &mut dyn Read) -> Result<u32> {
    let mut bytes = [0; 4];
    read(reader, &mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read(reader: &mut dyn Read, output: &mut [u8]) -> Result<()> {
    reader.read_exact(output).map_err(|error| invalid(format!("XCF geometry prefix: {error}")))
}

fn skip(reader: &mut dyn Read, mut length: usize) -> Result<()> {
    let mut scratch = [0; 32 * 1024];
    while length != 0 {
        let count = length.min(scratch.len());
        read(reader, scratch.get_mut(..count).ok_or_else(|| invalid("invalid geometry discard range"))?)?;
        length = length.saturating_sub(count);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
