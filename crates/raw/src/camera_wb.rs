//! Camera-supplied white-balance presets, independently of a colour profile.
//!
//! Canon ColorData8 word offsets are documented at
//! <https://exiftool.org/TagNames/Canon.html#ColorData8>. Only version 14 is
//! enabled here: its layout was checked against an EOS 2000D CR2. Other layouts
//! must be verified before enabling them; do not guess from plausible values.

use crate::tiff::Tiff;
use crate::{RawFormat, identify};

/// A fixed illuminant preset recorded by the camera. Automatic balance is an
/// image estimate, so it is deliberately separate from these fixed presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraWhiteBalance {
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
}

/// Reads camera-channel R, G, B gains (green = 1) without decoding the sensor.
/// `None` means unsupported, missing or malformed metadata; callers should use
/// their documented fallback. These are Canon presets, not Adobe presets or a
/// replacement for a camera colour matrix.
pub fn camera_white_balance(bytes: &[u8], preset: CameraWhiteBalance) -> Option<[f64; 3]> {
    if identify(bytes) != Some(RawFormat::Cr2) {
        return None;
    }
    let t = Tiff::new(bytes)?;
    let mn = crate::cr2::maker_note(&t, &t.all_ifds())?;
    let e = mn.get(0x4001)?;
    if !matches!(e.typ, 3 | 7) {
        return None;
    }
    let len = e.unit().checked_mul(usize::try_from(e.count).ok()?)?;
    let end = e.at.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    let word = |offset: usize| {
        let at = e.at.checked_add(offset.checked_mul(2)?)?;
        if at.checked_add(2)? > end {
            return None;
        }
        t.u16_at(at)
    };
    if word(0)? != 14 {
        return None;
    }
    let offset = match preset {
        CameraWhiteBalance::Daylight => 133,
        CameraWhiteBalance::Shade => 138,
        CameraWhiteBalance::Cloudy => 143,
        CameraWhiteBalance::Tungsten => 148,
        CameraWhiteBalance::Fluorescent => 153,
        CameraWhiteBalance::Flash => 163,
    };
    let r = f64::from(word(offset)?);
    let g1 = f64::from(word(offset + 1)?);
    let g2 = f64::from(word(offset + 2)?);
    let b = f64::from(word(offset + 3)?);
    let temperature = word(offset + 4)?;
    let g = (g1 + g2) * 0.5;
    if !(256.0..16384.0).contains(&g)
        || (g1 - g2).abs() > g * 0.05
        || !(0.25..8.0).contains(&(r / g))
        || !(0.25..8.0).contains(&(b / g))
        || !(2000..=15000).contains(&temperature)
    {
        return None;
    }
    Some([r / g, 1.0, b / g])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{TiffBuilder, Val};

    fn fixture(words: Vec<u16>, big_endian: bool, undefined: bool) -> Vec<u8> {
        let mut t = TiffBuilder { big_endian, ..Default::default() };
        let main = t.ifd(vec![]);
        let value = if undefined {
            Val::Undefined(words.iter().flat_map(|v| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() }).collect())
        } else {
            Val::Short(words)
        };
        let mn = t.ifd(vec![(0x4001, value)]);
        let exif = t.ifd(vec![(0x927c, Val::IfdBytes(mn))]);
        t.ifds[main] = vec![(0x8769, Val::Ifds(vec![exif]))];
        let raw = t.ifd(vec![]);
        t.cr2_raw_ifd = Some(raw);
        t.chain = vec![main];
        t.build()
    }

    fn table() -> Vec<u16> {
        let mut words = vec![0; 168];
        words[0] = 14;
        // Distinct synthetic values at each preset; unknown slots stay zero.
        for (i, offset) in [133, 143, 138, 148, 153, 163].into_iter().enumerate() {
            words[offset..offset + 5].copy_from_slice(&[1800 + i as u16 * 100, 1000, 1020, 1500 + i as u16 * 50, 5000]);
        }
        words
    }

    #[test]
    fn fixed_presets_use_their_own_camera_coefficients() {
        for big_endian in [false, true] {
            for undefined in [false, true] {
                let bytes = fixture(table(), big_endian, undefined);
                for (i, preset) in [
                    CameraWhiteBalance::Daylight,
                    CameraWhiteBalance::Cloudy,
                    CameraWhiteBalance::Shade,
                    CameraWhiteBalance::Tungsten,
                    CameraWhiteBalance::Fluorescent,
                    CameraWhiteBalance::Flash,
                ]
                .into_iter()
                .enumerate()
                {
                    let actual = camera_white_balance(&bytes, preset).expect("synthetic preset");
                    assert!((actual[0] - (1800.0 + i as f64 * 100.0) / 1010.0).abs() < 1e-12);
                    assert_eq!(actual[1], 1.0);
                    assert!((actual[2] - (1500.0 + i as f64 * 50.0) / 1010.0).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn unsupported_and_bad_metadata_fall_back_without_guessing() {
        let preset = CameraWhiteBalance::Daylight;
        assert_eq!(camera_white_balance(&[], preset), None);
        for index in [0, 133, 134, 135, 136, 137] {
            let mut words = table();
            words[index] = 0;
            assert_eq!(camera_white_balance(&fixture(words, false, false), preset), None);
        }
        let mut unknown_version = table();
        unknown_version[0] = 13;
        assert_eq!(camera_white_balance(&fixture(unknown_version, false, false), preset), None);
        // A shortened entry followed by valid-looking bytes cannot read beyond its count.
        for len in 0..138 {
            let bytes = fixture(table()[..len].to_vec(), false, false);
            assert_eq!(camera_white_balance(&bytes, preset), None, "length {len}");
        }
        let bytes = fixture(table(), false, false);
        for len in 0..bytes.len() {
            let _ = camera_white_balance(&bytes[..len], preset);
        }
    }
}
