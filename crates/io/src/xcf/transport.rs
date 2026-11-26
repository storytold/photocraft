//! App format selection over the shared bounded wrapper reader.
use std::borrow::Cow;

use craft_xcf::transport::{Limits, PrefixProbe, Wrapper};
use photocraft_raster::Interrupt;

use super::{Result, invalid, map_error};

#[cfg(test)]
mod tests;

pub(crate) enum Decoded<'a> {
    Xcf(Cow<'a, [u8]>),
    Svg,
    Unrelated,
}

/// A recognized XCF continues its existing decoder through full envelope checks.
/// Unrelated compressed input contributes only a bounded prefix; its own importer
/// remains responsible for complete decoding and its format-specific tail policy.
pub(crate) fn decode<'a>(name: &str, bytes: &'a [u8], ctl: &Interrupt<'_>, forced_xcf: bool, allow_svg: bool) -> Result<Decoded<'a>> {
    let wrapper = Wrapper::sniff(bytes);
    let hint = Wrapper::from_name(name).or(if forced_xcf { wrapper } else { None });
    let keep_going = || ctl.check().is_ok();
    let limits = Limits { probe_bytes: if allow_svg { 2 + 2 * 4096 + 2 } else { Limits::default().probe_bytes }, ..Limits::default() };
    match craft_xcf::transport::probe_prefix(bytes, hint, limits, &keep_going).map_err(map_error)? {
        PrefixProbe::Xcf(source) => {
            let decoded = source.read().map_err(map_error)?;
            Ok(Decoded::Xcf(match decoded {
                Cow::Borrowed(_) => Cow::Borrowed(bytes),
                Cow::Owned(bytes) => Cow::Owned(bytes),
            }))
        }
        PrefixProbe::Unrelated { .. } if forced_xcf => Err(invalid("the selected XCF decoder did not recognize a plain XCF source")),
        PrefixProbe::Unrelated { bytes: prefix, .. } if allow_svg && wrapper == Some(Wrapper::Gzip) && svg_signature(&prefix) => Ok(Decoded::Svg),
        PrefixProbe::Unrelated { .. } => Ok(Decoded::Unrelated),
    }
}

/// Match shared SVG text encodings on bounded evidence. A final cut surrogate is
/// unread evidence, not malformed full input; the real SVG importer checks it all.
fn svg_signature(bytes: &[u8]) -> bool {
    let little = bytes.starts_with(&[0xff, 0xfe]);
    let big = bytes.starts_with(&[0xfe, 0xff]);
    let limit = if little || big { 2 + 2 * 4096 } else { 8192 };
    let mut head = bytes.get(..bytes.len().min(limit)).unwrap_or_default();
    if head.len() < bytes.len() && (little || big) {
        let last = match head.get(head.len().saturating_sub(2)..) {
            Some([a, b]) if little => u16::from_le_bytes([*a, *b]),
            Some([a, b]) => u16::from_be_bytes([*a, *b]),
            _ => 0,
        };
        if (0xd800..=0xdbff).contains(&last) {
            head = head.get(..head.len().saturating_sub(2)).unwrap_or_default();
        }
    }
    crate::svg::is_svg(bytes) || craft_svg::svgz::decode(head).is_ok_and(|text| crate::svg::is_svg(text.as_bytes()))
}
