//! Smart-source geometry needs only canvas framing and image properties, never drawables.

use super::{Interrupt, IoError, Result, is_xcf, metadata, read_header, transport, wire};

/// Unclaimed input stays available to the ordinary non-XCF geometry reader. Compressed XCF
/// envelopes retain the transport's complete checksum, input-size and cancellation validation;
/// after unwrapping, no layer pointers, profiles, hierarchies or pixels are decoded.
pub(super) fn source_geometry(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<Option<((f64, f64), f64)>> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let inflated = if is_xcf(bytes) { None } else { transport::decode(name, bytes, ctl)? };
    let bytes = inflated.as_deref().unwrap_or(bytes);
    if !is_xcf(bytes) {
        return Ok(None);
    }
    let (mut cursor, header) = read_header(bytes)?;
    let properties = wire::props_version_with_interrupt(&mut cursor, Some(header.version), ctl)?;
    let dpi = metadata::image_resolution(&properties, 72.);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(Some(((f64::from(header.width), f64::from(header.height)), f64::from(dpi))))
}

#[cfg(test)]
mod tests;
