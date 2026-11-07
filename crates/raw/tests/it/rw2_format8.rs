//! RawFormat 8 through the public decode/develop APIs. Fixtures are generated, not photographs.

use photocraft_raw::testgen::{TiffBuilder, Val};
use photocraft_raw::{DevelopOptions, Limits, RawError, RawFormat, Rect, decode, develop_sensor};

type Tags = Vec<(u16, Val)>;

/// Store Panasonic word arrays as little-endian TIFF UNDEFINED bytes.
fn words(values: impl IntoIterator<Item = u16>) -> Val {
    Val::Undefined(values.into_iter().flat_map(u16::to_le_bytes).collect())
}

/// Encode a count-prefixed metadata list, with optional low/high word pairs.
fn list(values: &[u32], wide: bool) -> Val {
    let mut out = vec![values.len() as u16];
    for &v in values {
        out.push(v as u16);
        if wide {
            out.push((v >> 16) as u16);
        }
    }
    words(out)
}

/// Replace one fixture tag without leaving a duplicate that could hide the intended mutation.
fn replace(tags: &mut Tags, tag: u16, value: Val) {
    tags.retain(|(t, _)| *t != tag);
    tags.push((tag, value));
}

/// Encode synthetic cell differences independently of the fixture's declared sample depth.
fn encode(pixels: &[u16], width: usize, height: usize, table: &[(u16, u16); 17]) -> (Vec<u8>, u32) {
    let mut stream = Vec::new();
    let mut put = |n: u32, value: u32| stream.extend((0..n).rev().map(|i| ((value >> i) & 1) as u8));
    let at = |x, y| if x < width && y < height { i32::from(pixels[y * width + x]) } else { 0 };
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            for (px, py) in [(x, y), (x, y + 1), (x + 1, y), (x + 1, y + 1)] {
                let previous = if x >= 2 {
                    at(px - 2, py)
                } else if y >= 2 {
                    at(px, py - 2)
                } else {
                    0
                };
                let delta = at(px, py) - previous;
                let category = 32 - delta.unsigned_abs().leading_zeros();
                let (length, code) = table[category as usize];
                put(u32::from(length), u32::from(code));
                put(category, if delta < 0 { (delta + (1 << category) - 1) as u32 } else { delta as u32 });
            }
        }
    }
    let bits = stream.len() as u32;
    let bytes = stream.chunks(8).map(|chunk| chunk.iter().enumerate().fold(0, |byte, (i, bit)| byte | bit << i)).collect();
    (bytes, bits)
}

/// Build a tiled RW2 container with a supplied code table and optional hostile metadata edits.
fn fixture_with(pixels: &[u16], columns: &[usize], rows: &[usize], depth: u16, table: &[(u16, u16); 17], tweak: impl FnOnce(&mut Tags)) -> Vec<u8> {
    let (width, height): (usize, usize) = (columns.iter().sum(), rows.iter().sum());
    let mut payload = Vec::new();
    let (mut offsets, mut origins, mut lengths, mut widths, mut heights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut top = 0;
    for &h in rows {
        let mut left = 0;
        for &w in columns {
            let part: Vec<_> = (top..top + h).flat_map(|y| pixels[y * width + left..y * width + left + w].iter().copied()).collect();
            let (encoded, bits) = encode(&part, w, h, table);
            offsets.push(payload.len() as u32);
            origins.push(left as u32 | (top as u32) << 16);
            lengths.push(bits);
            widths.push(w as u32);
            heights.push(h as u32);
            payload.extend(encoded);
            // Cameras may include unused bytes/rows after the useful tile data.
            payload.extend([0; 8]);
            left += w;
        }
        top += h;
    }
    let mut builder = TiffBuilder::default();
    let blob = builder.blob(payload);
    let mut tags = vec![
        (2, Val::Short(vec![width as u16])),
        (3, Val::Short(vec![height as u16])),
        (4, Val::Short(vec![0])),
        (5, Val::Short(vec![0])),
        (6, Val::Short(vec![height as u16])),
        (7, Val::Short(vec![width as u16])),
        (9, Val::Short(vec![1])),
        (10, Val::Short(vec![depth])),
        (0x2d, Val::Short(vec![8])),
        (0x1c, Val::Short(vec![128])),
        (0x1d, Val::Short(vec![128])),
        (0x1e, Val::Short(vec![128])),
        (0x24, Val::Short(vec![512])),
        (0x25, Val::Short(vec![256])),
        (0x26, Val::Short(vec![384])),
        (0x40, words(std::iter::once(17).chain(table.iter().flat_map(|&(length, code)| [length, code])))),
        (0x43, Val::Short(vec![rows.len() as u16])),
        (0x44, list(&offsets, true)),
        (0x45, list(&origins, true)),
        (0x46, list(&lengths, true)),
        (0x47, list(&widths, false)),
        (0x48, list(&heights, false)),
        (0x118, Val::Blobs(vec![blob])),
    ];
    tweak(&mut tags);
    let ifd = builder.ifd(tags);
    builder.chain = vec![ifd];
    let first = builder.build();
    let count = u16::from_le_bytes(first[8..10].try_into().unwrap()) as usize;
    let raw_offset = (0..count)
        .find_map(|i| {
            let at = 10 + i * 12;
            (u16::from_le_bytes(first[at..at + 2].try_into().unwrap()) == 0x118).then(|| u32::from_le_bytes(first[at + 8..at + 12].try_into().unwrap()))
        })
        .unwrap();
    replace(&mut builder.ifds[ifd], 0x44, list(&offsets.iter().map(|v| raw_offset + v).collect::<Vec<_>>(), true));
    let mut file = builder.build();
    file[2] = b'U';
    file
}

/// Use disjoint five-bit prefixes so synthetic streams can exercise every difference category.
fn table() -> [(u16, u16); 17] {
    std::array::from_fn(|i| (5, i as u16))
}

/// Build a fixture with the synthetic prefix table and the requested tile geometry.
fn fixture(pixels: &[u16], columns: &[usize], rows: &[usize], depth: u16, tweak: impl FnOnce(&mut Tags)) -> Vec<u8> {
    fixture_with(pixels, columns, rows, depth, &table(), tweak)
}

/// All supported depths and strip/tile layouts must preserve sensor samples and metadata.
#[test]
fn strips_and_tiles_decode_exactly_at_all_supported_depths() {
    let (w, h) = (48, 30);
    for depth in [12, 14, 16] {
        let mut seed = 17u32;
        let pixels: Vec<_> = (0..w * h)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> (32 - depth)) as u16
            })
            .collect();
        for (columns, rows) in [(vec![48], vec![30]), (vec![20, 28], vec![30]), (vec![12; 4], vec![16, 14])] {
            let bytes = fixture(&pixels, &columns, &rows, depth, |_| {});
            let sensor = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(sensor.format, RawFormat::Rw2);
            assert_eq!((sensor.width, sensor.height, sensor.samples), (w, h, 1));
            assert_eq!(sensor.data, pixels, "{depth}-bit {columns:?} × {rows:?}");
            assert_eq!(sensor.black.values, vec![128.0; 4], "format 8 has no format-4 black offset");
            assert_eq!(sensor.white, [((1u32 << depth) - 1) as f32; 3]);
            assert_eq!(sensor.camera_wb, Some([2.0, 1.0, 1.5]));
        }
    }
}

/// Every sensor site must reject values above the declared 12- or 14-bit range.
#[test]
fn rejects_samples_above_declared_depth() {
    for (depth, value) in [(12, 4096), (12, 5000), (14, 16384), (14, 20000)] {
        for site in 0..16 {
            let mut pixels = [0; 16];
            pixels[site] = value;
            for (columns, rows) in [(vec![4], vec![4]), (vec![2, 2], vec![2, 2])] {
                let bytes = fixture(&pixels, &columns, &rows, depth, |_| {});
                assert!(
                    matches!(decode(&bytes, &Limits::default()), Err(RawError::Malformed(_))),
                    "accepted {value} at site {site} of a {depth}-bit {columns:?} × {rows:?} image"
                );
            }
        }
    }
}

/// Zero and the exact declared maximum remain legal, including the full 16-bit range.
#[test]
fn declared_depth_boundaries_decode_exactly() {
    for depth in [12, 14, 16] {
        let max = ((1u32 << depth) - 1) as u16;
        let pixels = [0, max, 1, max, max, 0, max, 1];
        for columns in [vec![4], vec![2, 2]] {
            let bytes = fixture(&pixels, &columns, &[2], depth, |_| {});
            assert_eq!(decode(&bytes, &Limits::default()).unwrap().data, pixels, "{depth}-bit {columns:?}");
        }
    }
}

/// A hand-coded cell checks the green-site order without relying on the fixture encoder.
#[test]
fn known_answer_cell_has_column_major_sites() {
    // Hand-written 28 bits with five-bit category codes: differences 1, 2, 3, 4.
    // Independent of the fixture encoder: the greens must not trade positions.
    let mut bytes = fixture(&[1, 3, 2, 4], &[2], &[2], 12, |_| {});
    let at = bytes.len() - 12; // four stream bytes followed by eight unused bytes
    bytes[at..at + 4].copy_from_slice(&[48, 10, 141, 3]);
    let sensor = decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(sensor.data, [1, 3, 2, 4]);
}

/// Real-camera prefix shadowing and partial 2×2 cells must retain exact samples.
#[test]
fn camera_prefix_table_and_partial_cells() {
    // Camera-supplied 12-bit table: the final two codes are shadowed by the
    // shorter category-14 prefix. Keep them legal, as in actual S9 RW2 headers.
    let table = [
        (5, 30),
        (5, 28),
        (4, 12),
        (3, 4),
        (3, 2),
        (2, 0),
        (3, 3),
        (3, 5),
        (4, 13),
        (5, 29),
        (6, 62),
        (7, 126),
        (7, 127),
        (8, 254),
        (8, 255),
        (12, 4094),
        (12, 4095),
    ];
    for (w, h) in [(1, 1), (3, 5), (32, 24)] {
        let pixels: Vec<_> = (0..w * h).map(|i| ((i * 913 + 7) % 4096) as u16).collect();
        let bytes = fixture_with(&pixels, &[w], &[h], 12, &table, |_| {});
        assert_eq!(decode(&bytes, &Limits::default()).unwrap().data, pixels);
    }
}

/// Border cropping must produce the requested dimensions without reducing development to 8 bits.
#[test]
fn crop_and_development_keep_full_resolution() {
    let pixels: Vec<_> = (0..32 * 24).map(|i| 400 + (i % 1700) as u16).collect();
    let bytes = fixture(&pixels, &[16, 16], &[24], 12, |tags| {
        for (tag, value) in [(4, 2), (5, 2), (6, 22), (7, 30)] {
            replace(tags, tag, Val::Short(vec![value]));
        }
    });
    let sensor = decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(sensor.crop, Rect::new(2, 2, 28, 20));
    let rgb = develop_sensor(&sensor, &DevelopOptions::default()).unwrap();
    assert_eq!((rgb.width, rgb.height), (28, 20));
    assert_eq!(rgb.rgb.len(), 28 * 20 * 3);
    assert!(rgb.rgb.iter().any(|v| v % 257 != 0), "development must retain more than eight bits");
}

/// Corrupt tile metadata and streams must fail instead of returning incomplete sensor data.
#[test]
fn malformed_layouts_and_streams_are_rejected() {
    let pixels = vec![400; 32 * 24];
    let case = |tag, value| decode(&fixture(&pixels, &[16, 16], &[12, 12], 12, |tags| replace(tags, tag, value)), &Limits::default());
    for (tag, value) in [
        (0x45, list(&[0, 0, 12 << 16, (12 << 16) | 16], true)),      // overlap
        (0x47, list(&[14, 16, 16, 16], false)),                      // hole
        (0x48, list(&[12, 12, 12, 32], false)),                      // outside sensor
        (0x46, list(&[1; 4], true)),                                 // truncated within an existing byte
        (0x46, list(&[u32::MAX; 4], true)),                          // outside file
        (0x45, words([65])),                                         // bounded before allocating
        (0x47, list(&[16], false)),                                  // mismatched counts
        (0x40, words([17, 5, 0])),                                   // truncated code table
        (0x40, words(std::iter::once(17).chain([5, 0].repeat(17)))), // duplicate codes
    ] {
        assert!(matches!(case(tag, value), Err(RawError::Malformed(_))), "tag {tag:#x}");
    }
    for (tag, value) in [(0x3b, 4095), (0x43, 1), (10, 32)] {
        assert!(matches!(case(tag, Val::Short(vec![value])), Err(RawError::Unsupported(_))));
    }
}

/// Peak buffer accounting and byte-level truncation must fail before an unsafe read or allocation.
#[test]
fn allocations_and_truncation_stay_bounded() {
    let pixels = vec![400; 16 * 12];
    let bytes = fixture(&pixels, &[8, 8], &[12], 12, |_| {});
    let limits = Limits { max_alloc: (pixels.len() * 4 - 1) as u64, ..Limits::default() };
    assert!(matches!(decode(&bytes, &limits), Err(RawError::LimitExceeded(_))));
    for length in 0..bytes.len() - 8 {
        assert!(decode(&bytes[..length], &Limits::default()).is_err(), "accepted {length} bytes");
    }
}

/// Bounded deterministic mutations must either fail gracefully or return a complete sensor plane.
#[test]
fn mutated_format8_inputs_never_panic() {
    let pixels: Vec<_> = (0..16 * 12).map(|i| (128 + i * 7) as u16).collect();
    let original = fixture(&pixels, &[8, 8], &[6, 6], 12, |_| {});
    let limits = Limits { max_width: 256, max_height: 256, max_pixels: 65536, max_alloc: 1 << 20 };
    let mut seed = 731u32;
    for _ in 0..1000 {
        let mut bytes = original.clone();
        for _ in 0..4 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let at = seed as usize % bytes.len();
            bytes[at] ^= (seed >> 16) as u8;
        }
        if let Ok(sensor) = decode(&bytes, &limits) {
            assert_eq!(sensor.data.len(), sensor.width * sensor.height);
        }
    }
}
