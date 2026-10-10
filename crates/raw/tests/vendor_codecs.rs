//! Vendor raw codecs, round-tripped through the in-test synthetic encoders:
//! Sony compressed ARW (cRAW), Panasonic RW2 (RawFormat 5) and uncompressed
//! Olympus ORF (with its maker-note preview).

use photocraft_raw::testgen::{craw_block, mosaic, orf, orf_padded12, rw2, scene, sony_craw};
use photocraft_raw::*;

const CURVE: [u16; 4] = [8000, 10400, 12900, 14100];

/// The cRAW tone curve as the decoder documents it (independent re-statement
/// for the tests): step 1, 2, 4, 8, 16 per 1/8 code between the knots, ÷ 4.
fn tone(code: u16) -> u16 {
    let pts = [0u32, 8000, 10400, 12900, 14100, 16384];
    let x = u32::from(code) * 8;
    let y: u32 = pts.windows(2).enumerate().filter(|(_, w)| x > w[0]).map(|(i, w)| (x.min(w[1]) - w[0]) << i).sum();
    (y / 4).min(16383) as u16
}

/// 11-bit codes over a smooth scene: every 16-pixel run spans < 128 codes, so
/// cRAW is exact.
fn smooth_codes(w: usize, h: usize) -> Vec<u16> {
    mosaic(&scene(w, h), w, [0, 1, 1, 2], 256, 1900)
}

#[test]
fn craw_round_trip_is_exact_on_smooth_data() {
    let (w, h) = (640, 8);
    let codes = smooth_codes(w, h);
    let b = sony_craw(w, h, &codes, CURVE);
    assert_eq!(identify(&b), Some(RawFormat::Arw));
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!((s.width, s.height), (w, h));
    let want: Vec<u16> = codes.iter().map(|&c| tone(c)).collect();
    assert_eq!(s.data, want);
    assert_eq!(s.black.values, vec![512.0; 4]);
    assert_eq!(s.white, [16383.0; 3]);
    assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
    let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width as usize, d.height as usize), (w, h));
}

#[test]
fn craw_wide_ranges_lose_only_low_bits() {
    // Pseudo-random codes: wide ranges force steps of 2..16.
    let (w, h) = (64, 8);
    let mut s = 12345u32;
    let codes: Vec<u16> = (0..w * h)
        .map(|_| {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            ((s >> 16) % 2000) as u16
        })
        .collect();
    let b = sony_craw(w, h, &codes, CURVE);
    let d = decode(&b, &Limits::default()).unwrap();
    // Re-derive each pixel's step from its block and check the code error.
    for y in 0..h {
        for g in 0..w / 32 {
            for parity in 0..2 {
                let run: Vec<u16> = (0..16).map(|i| codes[y * w + g * 32 + 2 * i + parity]).collect();
                let range = run.iter().max().unwrap() - run.iter().min().unwrap();
                let step = (0..5).map(|k| 1u16 << k).find(|s| 128 * s > range).unwrap();
                for (i, &c) in run.iter().enumerate() {
                    let got = d.data[y * w + g * 32 + 2 * i + parity];
                    let lo = tone(c.saturating_sub(step - 1));
                    assert!(got <= tone(c) && got >= lo, "code {c} step {step}: got {got}, want {lo}..={}", tone(c));
                }
            }
        }
    }
}

#[test]
fn craw_block_layout() {
    // Max 300 at 3, min 100 at 9, others 100 + 2k: range 200 → step 2.
    let mut codes = [0u16; 16];
    for (i, c) in codes.iter_mut().enumerate() {
        *c = 100 + 2 * i as u16;
    }
    codes[3] = 300;
    codes[9] = 100;
    codes[0] = 101; // odd: rounds down to 100 at step 2
    let b = u128::from_le_bytes(craw_block(&codes));
    assert_eq!(b & 0x7FF, 300);
    assert_eq!((b >> 11) & 0x7FF, 100);
    assert_eq!((b >> 22) & 0xF, 3);
    assert_eq!((b >> 26) & 0xF, 9);
    assert_eq!((b >> 30) & 0x7F, 0); // pixel 0: (101 - 100) >> 1
}

#[test]
fn craw_unusual_variants_are_unsupported() {
    let h = 4;
    let codes32 = vec![300u16; 32 * h];
    let ok = sony_craw(32, h, &codes32, CURVE);
    assert!(decode(&ok, &Limits::default()).is_ok());
    // A non-increasing tone curve is not guessed at.
    let bad_curve = sony_craw(32, h, &codes32, [9000, 8000, 12000, 13000]);
    assert!(matches!(decode(&bad_curve, &Limits::default()), Err(RawError::Unsupported(_))));
    // Truncated strip.
    let cut = &ok[..ok.len() - 20];
    assert!(decode(cut, &Limits::default()).is_err());
}

/// Renames a tag of the raw SubIFD (tag 330 of IFD0), hiding it from the
/// decoder: the first cRAW bodies write no BlackLevel tag at all.
fn hide_raw_sub_ifd_tag(b: &mut [u8], tag: u16) {
    let ifd = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes([b[ifd], b[ifd + 1]]) as usize;
    for i in 0..n {
        let e = ifd + 2 + 12 * i;
        if u16::from_le_bytes([b[e], b[e + 1]]) == 330 {
            let sub = u32::from_le_bytes(b[e + 8..e + 12].try_into().unwrap()) as usize;
            let sn = u16::from_le_bytes([b[sub], b[sub + 1]]) as usize;
            for j in 0..sn {
                let se = sub + 2 + 12 * j;
                if u16::from_le_bytes([b[se], b[se + 1]]) == tag {
                    b[se..se + 2].copy_from_slice(&0xFFFEu16.to_le_bytes());
                    return;
                }
            }
        }
    }
    panic!("tag {tag:#x} not found in the raw SubIFD");
}

#[test]
fn craw_without_a_black_level_tag_reads_black_from_the_tone_curve() {
    // An ILCE-7 cRAW carries no BlackLevel tag (0x7310): the tone curve maps
    // the black code (256) to 512, so the decoded data has a 512 black level.
    let (w, h) = (64, 4);
    let codes = vec![1024u16; w * h];
    let mut b = sony_craw(w, h, &codes, CURVE);
    hide_raw_sub_ifd_tag(&mut b, 0x7310);
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.black.values, vec![512.0]);
    assert!(!s.warnings.iter().any(|w| w.contains("black level")), "{:?}", s.warnings);
    // With the tag present the level is the same: 512 either way.
    let b = sony_craw(w, h, &codes, CURVE);
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.black.values, vec![512.0; 4]);
}

#[test]
fn rw2_round_trip_12_and_14_bit() {
    for (bits, w, h) in [(12u32, 120usize, 300usize), (14, 117, 260)] {
        let max = (1u16 << bits) - 1;
        let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 128, max);
        let b = rw2(w, h, &data, bits);
        assert!(b.len() > 3 * 0x4000, "spans several pages");
        assert_eq!(identify(&b), Some(RawFormat::Rw2));
        let s = decode(&b, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height), (w, h));
        assert_eq!(s.data, data, "{bits}-bit");
        assert_eq!(s.cfa.as_ref().unwrap().phase(0, 0), [0, 1, 1, 2]);
        assert_eq!(s.black.values, vec![128.0, 129.0, 129.0, 130.0]);
        assert_eq!(s.white, [f32::from(max); 3]);
        assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
        assert_eq!(s.crop, Rect::new(2, 2, w - 4, h - 4));
        let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
        assert_eq!((d.width as usize, d.height as usize), (w - 4, h - 4));
    }
}

/// Sets the value of a SHORT entry of IFD0 in a little-endian TIFF-like file.
fn patch_ifd0_short(b: &mut [u8], tag: u16, v: u16) {
    let ifd = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes([b[ifd], b[ifd + 1]]) as usize;
    for i in 0..n {
        let e = ifd + 2 + 12 * i;
        if u16::from_le_bytes([b[e], b[e + 1]]) == tag {
            b[e + 8..e + 10].copy_from_slice(&v.to_le_bytes());
            return;
        }
    }
    panic!("tag {tag:#x} not found");
}

#[test]
fn rw2_compressed_formats_fall_back() {
    let data = vec![200u16; 20 * 4];
    let mut b = rw2(20, 4, &data, 12);
    assert!(decode(&b, &Limits::default()).is_ok());
    patch_ifd0_short(&mut b, 0x002D, 4);
    match decode(&b, &Limits::default()) {
        Err(RawError::Unsupported(m)) => assert!(m.contains("raw format 4"), "{m}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
    // Width not made of whole blocks.
    let mut b = rw2(20, 4, &data, 12);
    patch_ifd0_short(&mut b, 0x0002, 19);
    assert!(matches!(decode(&b, &Limits::default()), Err(RawError::Unsupported(_))));
    // Sizes beyond the limits are refused before allocating.
    let mut b = rw2(20, 4, &data, 12);
    patch_ifd0_short(&mut b, 0x0003, 60000);
    let tight = Limits { max_pixels: 1 << 16, ..Limits::default() };
    assert!(matches!(decode(&b, &tight), Err(RawError::LimitExceeded(_))));
}

#[test]
fn orf_uncompressed_round_trip() {
    let (w, h) = (40, 24);
    let data = mosaic(&scene(w, h), w, [1, 0, 2, 1], 64, 4095);
    let b = orf(w, h, &data);
    assert_eq!(identify(&b), Some(RawFormat::Orf));
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.data, data, "left-justified samples are shifted back");
    assert_eq!(s.cfa.as_ref().unwrap().phase(0, 0), [1, 0, 2, 1]);
    assert_eq!(s.black.values, vec![64.0; 4]);
    assert_eq!(s.white, [4095.0; 3]);
    assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
    assert_eq!(s.crop, Rect::new(2, 2, w - 4, h - 4));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert!(develop_sensor(&s, &DevelopOptions::default()).is_ok());
    // The maker-note preview is found too (used when the data is compressed).
    let p = embedded_preview(&b).unwrap();
    assert_eq!((p.width, p.height), (16, 8));
}

/// A file cut short inside IFD0 keeps its width, height and compression entries but loses the
/// strip byte counts; their empty sum used to read as "fewer than 16 bits per sample" and the
/// damaged file was reported as the undecoded compressed variant. Damage is `Malformed`,
/// whichever byte it lands on.
#[test]
fn orf_truncated_in_ifd0_is_malformed_not_the_compressed_variant() {
    let (w, h) = (40, 24);
    let data = mosaic(&scene(w, h), w, [1, 0, 2, 1], 64, 4095);
    let b = orf(w, h, &data);
    assert!(decode(&b, &Limits::default()).is_ok());
    let ifd = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes([b[ifd], b[ifd + 1]]) as usize;
    let entries: Vec<u16> = (0..n).map(|i| u16::from_le_bytes([b[ifd + 2 + 12 * i], b[ifd + 3 + 12 * i]])).collect();
    let at = |tag: u16| ifd + 2 + 12 * entries.iter().position(|&t| t == tag).unwrap();
    // Cut right before the StripByteCounts entry: width, height and compression survive.
    let cut = at(279);
    assert!(cut > at(259), "compression precedes the byte counts");
    assert!(matches!(decode(&b[..cut], &Limits::default()), Err(RawError::Malformed(_))), "{:?}", decode(&b[..cut], &Limits::default()).err());
    // No prefix of the file reads as the compressed variant: the compression declaration is
    // intact or gone, never "compressed".
    for cut in 0..b.len() {
        if let Err(RawError::Unsupported(m)) = decode(&b[..cut], &Limits::default()) {
            assert!(!m.contains("compressed"), "prefix {cut}: {m}");
        }
    }
}

#[test]
fn orf_packed_or_compressed_falls_back() {
    let (w, h) = (40, 24);
    let mut b = orf(w, h, &vec![1000u16; w * h]);
    // Halve the declared strip size: no longer 16 bits per sample.
    let ifd = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes([b[ifd], b[ifd + 1]]) as usize;
    for i in 0..n {
        let e = ifd + 2 + 12 * i;
        if u16::from_le_bytes([b[e], b[e + 1]]) == 279 {
            b[e + 8..e + 12].copy_from_slice(&((w * h) as u32).to_le_bytes());
        }
    }
    assert!(matches!(decode(&b, &Limits::default()), Err(RawError::Unsupported(_))));
}

/// Offset of an IFD0 entry in the synthetic little-endian ORFs.
fn orf_entry(bytes: &[u8], tag: u16) -> usize {
    let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
    (0..count).map(|i| ifd + 2 + i * 12).find(|&at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) == tag).unwrap()
}

fn orf_inline_u32(bytes: &[u8], tag: u16) -> u32 {
    let at = orf_entry(bytes, tag) + 8;
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn orf_set_u32(bytes: &mut [u8], tag: u16, value: u32) {
    let at = orf_entry(bytes, tag) + 8;
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn orf_padded12_round_trip_preserves_samples_and_metadata() {
    let (w, h) = (30, 8);
    // Include both extrema and varying nibbles at every sample position, block and row boundary.
    let data: Vec<u16> = (0..w * h)
        .map(|i| match i % 17 {
            0 => 0,
            1 => 4095,
            _ => ((i * 0x123) & 0xfff) as u16,
        })
        .collect();
    let bytes = orf_padded12(w, h, &data);
    let raw = orf_inline_u32(&bytes, 273) as usize;
    assert_eq!(&bytes[raw..raw + 3], &[0x00, 0xf0, 0xff], "first pair is 0 and 4095");
    assert_eq!(bytes[raw + 15], 0, "padding is not a sample");
    let sensor = decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(sensor.data, data);
    assert_eq!((sensor.width, sensor.height, sensor.samples), (w, h, 1));
    assert_eq!(sensor.format, RawFormat::Orf);
    assert_eq!(sensor.cfa.as_ref().unwrap().phase(0, 0), [1, 0, 2, 1]);
    assert_eq!(sensor.black.values, vec![64.0; 4]);
    assert_eq!(sensor.white, [4095.0; 3]);
    assert_eq!(sensor.camera_wb, Some([2.0, 1.0, 1.5]));
    assert_eq!(sensor.crop, Rect::new(2, 2, w - 4, h - 4));
    assert!(sensor.warnings.is_empty(), "{:?}", sensor.warnings);
    assert!(develop_sensor(&sensor, &DevelopOptions::default()).is_ok());
    let preview = embedded_preview(&bytes).unwrap();
    assert_eq!((preview.width, preview.height), (16, 8));
    // The same storage layout is accepted with a 16-bit sample declaration.
    let mut bits16 = bytes;
    patch_ifd0_short(&mut bits16, 258, 16);
    assert_eq!(decode(&bits16, &Limits::default()).unwrap().data, data);
}

#[test]
fn orf_padded12_requires_the_exact_storage_layout() {
    let bytes = orf_padded12(20, 8, &vec![1234; 160]);
    let mut exact12 = bytes.clone();
    orf_set_u32(&mut exact12, 279, 160 * 12 / 8);
    assert!(matches!(decode(&exact12, &Limits::default()), Err(RawError::Unsupported(_))));
    // Keep the total pixel count and strip size, but make each row end inside a block.
    let mut partial_row = bytes.clone();
    orf_set_u32(&mut partial_row, 256, 16);
    orf_set_u32(&mut partial_row, 257, 10);
    assert!(matches!(decode(&partial_row, &Limits::default()), Err(RawError::Unsupported(_))));
    let mut compressed = bytes.clone();
    patch_ifd0_short(&mut compressed, 259, 7);
    assert!(matches!(decode(&compressed, &Limits::default()), Err(RawError::Unsupported(_))));
    for (tag, value) in [(277, 2), (258, 8)] {
        let mut wrong_samples = bytes.clone();
        patch_ifd0_short(&mut wrong_samples, tag, value);
        assert!(matches!(decode(&wrong_samples, &Limits::default()), Err(RawError::Unsupported(_))));
    }
    for format in [2, 3] {
        let mut signed_or_float = bytes.clone();
        // Replace the optional PhotometricInterpretation entry with SampleFormat.
        let entry = orf_entry(&signed_or_float, 262);
        signed_or_float[entry..entry + 2].copy_from_slice(&339u16.to_le_bytes());
        patch_ifd0_short(&mut signed_or_float, 339, format);
        assert!(matches!(decode(&signed_or_float, &Limits::default()), Err(RawError::Unsupported(_))));
    }
    // A valid big-endian ORF is not one of the measured little-endian padded layouts.
    use photocraft_raw::testgen::{TiffBuilder, Val};
    let mut tiff = TiffBuilder { big_endian: true, ..Default::default() };
    let raw = orf_inline_u32(&bytes, 273) as usize;
    let strip = tiff.blob(bytes[raw..raw + 256].to_vec());
    let ifd = tiff.ifd(vec![
        (256, Val::Long(vec![20])),
        (257, Val::Long(vec![8])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![1])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![8])),
        (279, Val::Long(vec![256])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![1, 0, 2, 1])),
    ]);
    tiff.chain = vec![ifd];
    let mut big_endian = tiff.build();
    big_endian[2..4].copy_from_slice(b"OR");
    assert_eq!(identify(&big_endian), Some(RawFormat::Orf));
    assert!(matches!(decode(&big_endian, &Limits::default()), Err(RawError::Unsupported(_))));
    // Two otherwise contiguous equal strips are not the observed single-strip layout.
    let mut two_strips = bytes;
    let start = orf_inline_u32(&two_strips, 273);
    for (tag, values) in [(273, [start, start + 128]), (279, [128, 128])] {
        let entry = orf_entry(&two_strips, tag);
        let array_at = two_strips.len() as u32;
        two_strips.extend(values.into_iter().flat_map(u32::to_le_bytes));
        two_strips[entry + 4..entry + 8].copy_from_slice(&2u32.to_le_bytes());
        orf_set_u32(&mut two_strips, tag, array_at);
    }
    orf_set_u32(&mut two_strips, 278, 4);
    assert!(matches!(decode(&two_strips, &Limits::default()), Err(RawError::Unsupported(_))));
}

#[test]
fn orf_padded12_rejects_damaged_payload_and_obeys_limits() {
    let bytes = orf_padded12(20, 8, &vec![1234; 160]);
    let raw = orf_inline_u32(&bytes, 273) as usize;
    let len = orf_inline_u32(&bytes, 279) as usize;
    let mut bad_pad = bytes.clone();
    bad_pad[raw + 15] = 1;
    assert!(matches!(decode(&bad_pad, &Limits::default()), Err(RawError::Malformed(_))));
    assert!(matches!(decode(&bytes[..raw + len - 1], &Limits::default()), Err(RawError::Malformed(_))));
    for limits in [Limits { max_pixels: 159, ..Limits::default() }, Limits { max_alloc: 319, ..Limits::default() }] {
        assert!(matches!(decode(&bytes, &limits), Err(RawError::LimitExceeded(_))));
    }
}
