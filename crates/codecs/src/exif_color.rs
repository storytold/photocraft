//! The EXIF ColorSpace tag (0xA001, in the Exif sub-IFD that IFD0's tag 0x8769 points to):
//! 1 = sRGB, 0xFFFF = uncalibrated (EXIF 2.3, §4.6.5).
//!
//! Cameras write it instead of an ICC profile. Photoshop opens a file with no embedded profile but
//! ColorSpace = sRGB as sRGB, unless Preferences › File Handling › Ignore EXIF Profile Tag is on.
//! Anything malformed, truncated or missing reads as no tag.

use crate::orientation::{Order, ifd0_entries, tiff_body};

/// IFD0's pointer to the Exif sub-IFD.
const TAG_EXIF_IFD: u16 = 0x8769;
/// The ColorSpace tag in the Exif sub-IFD.
const TAG_COLOR_SPACE: u16 = 0xA001;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
/// TIFF type 13: an IFD offset (some writers use it for sub-IFD pointers).
const TYPE_IFD: u16 = 13;
/// Bytes per classic TIFF IFD entry; EXIF is never BigTIFF.
const ENTRY: usize = 12;
/// No IFD can start inside the 8-byte TIFF header.
const HEADER: usize = 8;

/// The ColorSpace value meaning sRGB.
pub const EXIF_COLOR_SPACE_SRGB: u16 = 1;

/// The EXIF ColorSpace value of a TIFF-structured block (an EXIF payload with or without the
/// `Exif\0\0` prefix), or `None` when it has none or it is malformed.
pub fn exif_color_space(exif: &[u8]) -> Option<u16> {
    let b = tiff_body(exif);
    let (order, first, count, entry_bytes, big_tiff) = ifd0_entries(b)?;
    if big_tiff {
        return None;
    }
    let ptr = find(order, b, first, count, entry_bytes, TAG_EXIF_IFD)?;
    if !matches!(order.u16(b, ptr.checked_add(2)?)?, TYPE_LONG | TYPE_IFD) || order.u32(b, ptr.checked_add(4)?)? != 1 {
        return None;
    }
    let ifd = usize::try_from(order.u32(b, ptr.checked_add(8)?)?).ok()?;
    if ifd < HEADER {
        return None;
    }
    let n = usize::from(order.u16(b, ifd)?);
    let first = ifd.checked_add(2)?;
    // The whole table must be present.
    if first.checked_add(n.checked_mul(ENTRY)?)? > b.len() {
        return None;
    }
    let e = find(order, b, first, n, ENTRY, TAG_COLOR_SPACE)?;
    if order.u32(b, e.checked_add(4)?)? != 1 {
        return None;
    }
    match order.u16(b, e.checked_add(2)?)? {
        TYPE_SHORT => order.u16(b, e.checked_add(8)?),
        TYPE_LONG => u16::try_from(order.u32(b, e.checked_add(8)?)?).ok(),
        _ => None,
    }
}

/// Normalize EXIF colour-space declarations for an sRGB JPEG XL codestream.
///
/// JXL export cannot embed an ICC profile and its default colour encoding is sRGB.
/// Only the existing inline ColorSpace value and an existing Adobe RGB
/// InteroperabilityIndex (R03 -> R98) are rewritten. All other EXIF bytes,
/// including GPS, camera data and offsets, remain untouched. Unknown or
/// malformed TIFF structures are returned unchanged.
pub(crate) fn srgb_exif_metadata(exif: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let body = tiff_body(exif);
    let prefix = exif.len() - body.len();
    let edits = (|| -> Option<Vec<(usize, Vec<u8>)>> {
        let (order, first, count, entry_bytes, big_tiff) = ifd0_entries(body)?;
        if big_tiff {
            return None;
        }
        let ptr = find(order, body, first, count, entry_bytes, TAG_EXIF_IFD)?;
        if !matches!(order.u16(body, ptr.checked_add(2)?)?, TYPE_LONG | TYPE_IFD)
            || order.u32(body, ptr.checked_add(4)?)? != 1
        {
            return None;
        }
        let ifd = usize::try_from(order.u32(body, ptr.checked_add(8)?)?).ok()?;
        if ifd < HEADER {
            return None;
        }
        let count = usize::from(order.u16(body, ifd)?);
        let first = ifd.checked_add(2)?;
        let end = first.checked_add(count.checked_mul(ENTRY)?)?;
        if end.checked_add(4)? > body.len() {
            return None;
        }
        let mut edits = Vec::new();
        for i in 0..count {
            let e = first.checked_add(i.checked_mul(ENTRY)?)?;
            let ty = order.u16(body, e.checked_add(2)?)?;
            if order.u32(body, e.checked_add(4)?)? != 1 {
                continue;
            }
            if order.u16(body, e)? == TAG_COLOR_SPACE {
                let at = e.checked_add(8)?;
                let replacement = match (ty, order) {
                    (TYPE_SHORT, Order::Little) => 1u16.to_le_bytes().to_vec(),
                    (TYPE_SHORT, Order::Big) => 1u16.to_be_bytes().to_vec(),
                    (TYPE_LONG, Order::Little) => 1u32.to_le_bytes().to_vec(),
                    (TYPE_LONG, Order::Big) => 1u32.to_be_bytes().to_vec(),
                    _ => continue,
                };
                if body.get(at..at.checked_add(replacement.len())?)? != replacement.as_slice() {
                    edits.push((prefix.checked_add(at)?, replacement));
                }
            }
            // ExifIFD 0xA005 points to an Interoperability IFD. R03 means
            // Adobe RGB, while R98 means sRGB. Rewrite an existing R03 only.
            if order.u16(body, e)? == 0xA005 && matches!(ty, TYPE_LONG | TYPE_IFD) {
                let sub = usize::try_from(order.u32(body, e.checked_add(8)?)?).ok()?;
                if sub < HEADER {
                    continue;
                }
                let Some(n) = order.u16(body, sub) else { continue };
                let Some(start) = sub.checked_add(2) else { continue };
                let Some(end) = start.checked_add(usize::from(n).checked_mul(ENTRY)?) else { continue };
                if end.checked_add(4)? > body.len() {
                    continue;
                }
                for j in 0..usize::from(n) {
                    let entry = start.checked_add(j.checked_mul(ENTRY)?)?;
                    if order.u16(body, entry) != Some(1)
                        || order.u16(body, entry.checked_add(2)?) != Some(2)
                    {
                        continue;
                    }
                    let length = usize::try_from(order.u32(body, entry.checked_add(4)?)?).ok()?;
                    if length < 4 {
                        continue;
                    }
                    let data_at = if length <= 4 {
                        entry.checked_add(8)?
                    } else {
                        usize::try_from(order.u32(body, entry.checked_add(8)?)?).ok()?
                    };
                    if body.get(data_at..data_at.checked_add(3)?) == Some(b"R03".as_slice()) {
                        edits.push((prefix.checked_add(data_at)?, b"R98".to_vec()));
                    }
                }
            }
        }
        Some(edits)
    })();
    let Some(edits) = edits else { return std::borrow::Cow::Borrowed(exif) };
    if edits.is_empty() {
        return std::borrow::Cow::Borrowed(exif);
    }
    let mut updated = exif.to_vec();
    for (at, bytes) in edits {
        if let Some(slot) = updated.get_mut(at..at + bytes.len()) {
            slot.copy_from_slice(&bytes);
        }
    }
    std::borrow::Cow::Owned(updated)
}

/// Offset of the first entry tagged `tag` in the IFD whose `count` entries start at `first`.
fn find(order: Order, b: &[u8], first: usize, count: usize, entry_bytes: usize, tag: u16) -> Option<usize> {
    (0..count).filter_map(|i| first.checked_add(i.checked_mul(entry_bytes)?)).find(|&e| order.u16(b, e) == Some(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little- or big-endian EXIF block: IFD0 holding only the Exif pointer, then an Exif IFD
    /// holding `entries` (tag, type, count, value).
    fn exif(big_endian: bool, entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let u16b = |v: u16| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let u32b = |v: u32| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut b = if big_endian { b"MM\0\x2a".to_vec() } else { b"II\x2a\0".to_vec() };
        b.extend(u32b(8));
        // IFD0: one entry, then the next-IFD pointer. The Exif IFD starts right after (8 + 2 + 12 + 4).
        b.extend(u16b(1));
        b.extend(u16b(TAG_EXIF_IFD));
        b.extend(u16b(TYPE_LONG));
        b.extend(u32b(1));
        b.extend(u32b(26));
        b.extend(u32b(0));
        b.extend(u16b(entries.len() as u16));
        for &(tag, ty, count, value) in entries {
            b.extend(u16b(tag));
            b.extend(u16b(ty));
            b.extend(u32b(count));
            // SHORT values are left-justified in the 4-byte field.
            if ty == TYPE_SHORT {
                b.extend(u16b(value as u16));
                b.extend([0, 0]);
            } else {
                b.extend(u32b(value));
            }
        }
        b.extend(u32b(0));
        b
    }

    #[test]
    fn reads_the_color_space_in_either_byte_order() {
        for big in [false, true] {
            let srgb = exif(big, &[(0x9000, 7, 4, 0x3032_3330), (TAG_COLOR_SPACE, TYPE_SHORT, 1, 1)]);
            assert_eq!(exif_color_space(&srgb), Some(EXIF_COLOR_SPACE_SRGB), "big endian: {big}");
            let mut prefixed = b"Exif\0\0".to_vec();
            prefixed.extend(&srgb);
            assert_eq!(exif_color_space(&prefixed), Some(1));
            assert_eq!(exif_color_space(&exif(big, &[(TAG_COLOR_SPACE, TYPE_SHORT, 1, 0xFFFF)])), Some(0xFFFF));
            assert_eq!(exif_color_space(&exif(big, &[(TAG_COLOR_SPACE, TYPE_LONG, 1, 1)])), Some(1), "LONG is accepted");
        }
    }

    #[test]
    fn srgb_export_corrects_exif_colour_space_but_preserves_other_tags() {
        for big_endian in [false, true] {
            let input = exif(
                big_endian,
                &[
                    (TAG_COLOR_SPACE, TYPE_SHORT, 1, 0xffff),
                    (0x9000, 7, 4, 0x3032_3330), // ExifVersion: untouched
                ],
            );
            let output = srgb_exif_metadata(&input);
            assert_eq!(exif_color_space(&output), Some(1));
            assert_eq!(output.len(), input.len());
            // Only the two-byte ColorSpace value may change.
            let differences: Vec<_> = input.iter().zip(output.iter()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
            assert!(!differences.is_empty());
            assert!(differences.iter().all(|&i| (38..40).contains(&i)));
            assert!(matches!(srgb_exif_metadata(&output), std::borrow::Cow::Borrowed(_)));

            let mut prefixed = b"Exif\0\0".to_vec();
            prefixed.extend_from_slice(&input);
            let corrected = srgb_exif_metadata(&prefixed);
            assert!(corrected.starts_with(b"Exif\0\0"));
            assert_eq!(exif_color_space(&corrected), Some(1));
        }

        assert!(matches!(srgb_exif_metadata(b"not an EXIF payload"), std::borrow::Cow::Borrowed(_)));
        assert!(matches!(srgb_exif_metadata(&exif(false, &[(0x9000, 7, 4, 0)])), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn missing_or_malformed_reads_as_no_tag() {
        assert_eq!(exif_color_space(&exif(false, &[(0x9000, 7, 4, 0)])), None, "no ColorSpace entry");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, TYPE_SHORT, 2, 1)])), None, "count ≠ 1");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, 2, 1, 1)])), None, "ASCII type");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, TYPE_LONG, 1, 0x1_0000)])), None, "LONG over u16");
        let good = exif(false, &[(TAG_COLOR_SPACE, TYPE_SHORT, 1, 1)]);
        // Every truncation is rejected without panicking.
        for len in 0..good.len() - 4 {
            assert_eq!(exif_color_space(&good[..len]), None, "truncated to {len}");
        }
        // An Exif pointer past the end, into the header, or of the wrong type.
        for (at, value) in [(18usize, 0xFFFF_FFF0u32), (18, 4), (18, u32::MAX)] {
            let mut b = good.clone();
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(exif_color_space(&b), None, "pointer {value:#x}");
        }
        let mut b = good.clone();
        b[12..14].copy_from_slice(&TYPE_SHORT.to_le_bytes());
        assert_eq!(exif_color_space(&b), None, "pointer type SHORT");
        // An Exif IFD claiming more entries than the block holds.
        let mut b = good;
        b[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(exif_color_space(&b), None);
        assert_eq!(exif_color_space(b"not exif"), None);
    }
}
