//! Consume IFD0's EXIF orientation once, before decoded pixels enter a document.
//! Thumbnail/sub-IFD orientations describe other pixels and must not be followed.

use crate::{CodecError, Image, Limits};

struct Orientation {
    value: u16,
    offset: usize,
    little: bool,
}

fn entry(bytes: &[u8]) -> Option<Orientation> {
    // Some WebP producers include the JPEG prefix in their EXIF chunk.
    let prefix = if bytes.starts_with(b"Exif\0\0") { 6 } else { 0 };
    let tiff = bytes.get(prefix..)?;
    let little = match tiff.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let short = |b: &[u8]| -> Option<u16> {
        let b = b.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let long = |b: &[u8]| -> Option<u32> {
        let b = b.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(long(tiff.get(4..8)?)?).ok()?;
    if ifd < 8 {
        return None;
    }
    let start = ifd.checked_add(2)?;
    let count = usize::from(short(tiff.get(ifd..start)?)?);
    let end = start.checked_add(count.checked_mul(12)?)?;
    // Refuse truncated tables, even if an orientation happened to precede the truncation.
    tiff.get(end..end.checked_add(4)?)?;
    let mut found = None;
    for (i, field) in tiff.get(start..end)?.chunks_exact(12).enumerate() {
        if short(field.get(..2)?)? != 0x0112 {
            continue;
        }
        // TIFF Orientation is a single, inline SHORT. Ambiguous duplicates are malformed.
        if found.is_some() || short(field.get(2..4)?)? != 3 || long(field.get(4..8)?)? != 1 {
            return None;
        }
        let value = short(field.get(8..10)?)?;
        if !(1..=8).contains(&value) {
            return None;
        }
        let offset = prefix.checked_add(start)?.checked_add(i.checked_mul(12)?)?.checked_add(8)?;
        found = Some(Orientation { value, offset, little });
    }
    found
}

pub(crate) fn normalize(mut img: Image, limits: &Limits) -> Result<Image, CodecError> {
    let Some(orientation) = img.meta.exif.as_deref().and_then(entry).filter(|o| o.value != 1) else {
        return Ok(img);
    };
    let (width, height) = img.dimensions();
    let swap = orientation.value >= 5;
    let (out_width, out_height) = if swap { (height, width) } else { (width, height) };
    limits.check(out_width, out_height, img.layout(), img.sample_type())?;

    // Permute whole pixels: no colour conversion, rounding, or assumptions about depth/alpha.
    let pixel_bytes = img.layout().channels() * img.sample_type().bytes();
    let mut data = Vec::new();
    data.try_reserve_exact(img.data().len()).map_err(|_| CodecError::LimitExceeded("not enough memory to orient image".into()))?;
    for y in 0..out_height {
        for x in 0..out_width {
            let (sx, sy) = match orientation.value {
                2 => (width - 1 - x, y),
                3 => (width - 1 - x, height - 1 - y),
                4 => (x, height - 1 - y),
                5 => (y, x),
                6 => (y, height - 1 - x),
                7 => (width - 1 - y, height - 1 - x),
                8 => (width - 1 - y, x),
                _ => return Err(CodecError::InvalidImage("invalid EXIF orientation".into())),
            };
            let offset = (sy as usize).checked_mul(width as usize).and_then(|n| n.checked_add(sx as usize)).and_then(|n| n.checked_mul(pixel_bytes));
            let pixel = offset.and_then(|n| n.checked_add(pixel_bytes).and_then(|end| img.data().get(n..end)));
            let pixel = pixel.ok_or_else(|| CodecError::InvalidImage("EXIF orientation exceeds pixel buffer".into()))?;
            data.extend_from_slice(pixel);
        }
    }

    let value = if orientation.little { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
    let tag = img
        .meta
        .exif
        .as_mut()
        .and_then(|b| orientation.offset.checked_add(2).and_then(|end| b.get_mut(orientation.offset..end)))
        .ok_or_else(|| CodecError::InvalidImage("EXIF orientation tag disappeared".into()))?;
    tag.copy_from_slice(&value);
    if swap {
        img.meta.dpi = img.meta.dpi.map(|(x, y)| (y, x));
    }
    Ok(Image::from_raw(out_width, out_height, img.layout(), img.sample_type(), data)?.with_icc(img.icc).with_meta(img.meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, SampleType};

    fn exif(value: u8) -> Vec<u8> {
        vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, value, 0, 0, 0, 0, 0, 0, 0]
    }

    #[test]
    fn every_depth_and_layout_preserves_sample_bits_and_icc() {
        let orders =
            [[2, 1, 0, 5, 4, 3], [5, 4, 3, 2, 1, 0], [3, 4, 5, 0, 1, 2], [0, 3, 1, 4, 2, 5], [3, 0, 4, 1, 5, 2], [5, 2, 4, 1, 3, 0], [2, 5, 1, 4, 0, 3]];
        for layout in ChannelLayout::ALL {
            for sample in SampleType::ALL {
                let bpp = layout.channels() * sample.bytes();
                let pixels: Vec<u8> = (0..6 * bpp).map(|n| n as u8).collect();
                for (i, order) in orders.iter().enumerate() {
                    let mut img = Image::from_raw(3, 2, layout, sample, pixels.clone()).unwrap();
                    img.icc = Some(vec![1, 2, 3, 4]);
                    img.meta.exif = Some(exif(i as u8 + 2));
                    let got = normalize(img, &Limits::default()).unwrap();
                    assert_eq!(got.layout(), layout);
                    assert_eq!(got.sample_type(), sample);
                    assert_eq!(got.icc, Some(vec![1, 2, 3, 4]));
                    let expected: Vec<u8> = order.iter().flat_map(|&p| pixels[p * bpp..(p + 1) * bpp].iter().copied()).collect();
                    assert_eq!(got.data(), expected);
                    assert_eq!(got.meta.exif, Some(exif(1)));
                }
            }
        }
    }

    #[test]
    fn missing_identity_and_invalid_metadata_do_not_copy_pixels() {
        for meta in [None, Some(exif(1)), Some(exif(9)), Some(vec![0; 32])] {
            let mut img = Image::from_u8(3, 2, ChannelLayout::Gray, vec![1, 2, 3, 4, 5, 6]).unwrap();
            img.meta.exif = meta.clone();
            let ptr = img.data().as_ptr();
            let got = normalize(img, &Limits::default()).unwrap();
            assert_eq!(got.data().as_ptr(), ptr);
            assert_eq!(got.meta.exif, meta);
        }
    }

    #[test]
    fn prefixed_exif_works_and_thumbnail_ifd_is_not_followed() {
        let mut bytes = b"Exif\0\0".to_vec();
        bytes.extend(exif(6));
        // A cyclic next-IFD pointer cannot cause recursion or change the main orientation.
        bytes[28..32].copy_from_slice(&8u32.to_le_bytes());
        let mut img = Image::from_u8(3, 2, ChannelLayout::Gray, vec![1, 2, 3, 4, 5, 6]).unwrap();
        img.meta.exif = Some(bytes.clone());
        let got = normalize(img, &Limits::default()).unwrap();
        assert_eq!(got.data(), [4, 1, 5, 2, 6, 3]);
        bytes[24] = 1;
        assert_eq!(got.meta.exif, Some(bytes));
    }

    #[test]
    fn duplicate_orientation_is_ambiguous_and_ignored() {
        let mut bytes = exif(6);
        bytes[8] = 2;
        let duplicate = bytes[10..22].to_vec();
        bytes.splice(22..22, duplicate);
        assert!(entry(&bytes).is_none());
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_exif_never_panics(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
            let _ = entry(&bytes);
        }
    }
}
