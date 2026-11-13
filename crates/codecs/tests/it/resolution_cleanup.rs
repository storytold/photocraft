//! Stable IFD0 compaction under repeated malformed resolution tags (#8610).
//! Fixtures retain original value offsets and compare the entire metadata block.

use std::borrow::Cow;

use photocraft_codecs::resolution::exif_with_resolution;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, SampleType, decode, encode};

#[derive(Clone, Copy)]
struct Layout {
    big_tiff: bool,
    big_endian: bool,
}

impl Layout {
    /// Encode a fixture integer at the selected width and byte order.
    fn number(self, value: u64, width: usize) -> Vec<u8> {
        if self.big_endian { value.to_be_bytes()[8 - width..].to_vec() } else { value.to_le_bytes()[..width].to_vec() }
    }

    /// Build source and expected blocks separately, leaving out-of-line data at its original
    /// offsets. Cases select unrelated, malformed or rewritable entries in physical order.
    fn fixture(self, cases: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (header, count_bytes, entry_bytes, offset_bytes) = if self.big_tiff { (16, 8, 20, 8) } else { (8, 2, 12, 4) };
        let first = header + count_bytes;
        let table_end = first + cases.len() * entry_bytes;
        let mut data = b"unrelated out-of-line value\0".to_vec();
        let mut source_table = Vec::new();
        let mut kept_table = Vec::new();
        for &case in cases {
            let rational = |n, d| [self.number(n, 4), self.number(d, 4)].concat();
            let (tag, ty, count, value, replacement) = match case {
                0 => (270, 2, data.len() as u64, data.clone(), Some(data.clone())),
                1 => (282, 3, 1, self.number(300, 2), None), // wrong type
                2 => (283, 5, 1, rational(300, 0), None),    // zero denominator
                3 => (296, 4, 1, self.number(3, 4), None),   // wrong unit type
                4 => (282, 5, 1, rational(300, 1), Some(rational(240, 1))),
                5 => (283, 5, 1, rational(300, 1), Some(rational(240, 1))),
                6 => (296, 3, 1, self.number(3, 2), Some(self.number(2, 2))),
                7 => (282, 5, 2, [rational(300, 1), rational(300, 1)].concat(), None), // wrong count
                9 => (282, 5, u64::MAX, vec![0; 16], None),                            // oversized value span
                _ => (283, 1, 1, vec![99], None),                                      // unknown value type
            };
            let mut entry = [self.number(tag, 2), self.number(ty, 2), self.number(count, offset_bytes)].concat();
            let mut rewritten = entry.clone();
            if value.len() > offset_bytes {
                let at = table_end + offset_bytes + data.len();
                entry.extend(self.number(at as u64, offset_bytes));
                rewritten.extend(self.number(at as u64, offset_bytes));
                // These values occupy their original slots even if their entry is removed.
                data.extend_from_slice(replacement.as_deref().unwrap_or(&value));
            } else {
                let mut inline = value.clone();
                inline.resize(offset_bytes, 0);
                entry.extend(inline);
                let mut inline = replacement.clone().unwrap_or_else(|| value.clone());
                inline.resize(offset_bytes, 0);
                rewritten.extend(inline);
            }
            source_table.extend_from_slice(&entry);
            if replacement.is_some() {
                kept_table.extend_from_slice(&rewritten);
            }
        }
        let mut prefix = if self.big_endian { b"MM".to_vec() } else { b"II".to_vec() };
        prefix.extend(self.number(if self.big_tiff { 43 } else { 42 }, 2));
        if self.big_tiff {
            prefix.extend(self.number(8, 2));
            prefix.extend(self.number(0, 2));
        }
        prefix.extend(self.number(header as u64, offset_bytes));
        let next = self.number(0x10203040, offset_bytes);
        let mut source = prefix.clone();
        source.extend(self.number(cases.len() as u64, count_bytes));
        source.extend_from_slice(&source_table);
        source.extend_from_slice(&next);
        source.extend_from_slice(&data);
        // Replace the rewritten rational payloads in the source with their original values.
        // Locate them using source entry offsets, never offsets from the compacted table.
        for (case, entry) in cases.iter().zip(source_table.chunks_exact(entry_bytes)) {
            if !self.big_tiff && matches!(case, 4 | 5) {
                let bytes: [u8; 4] = entry[8..12].try_into().unwrap();
                let at = if self.big_endian { u32::from_be_bytes(bytes) } else { u32::from_le_bytes(bytes) } as usize;
                source[at..at + 4].copy_from_slice(&self.number(300, 4));
            }
        }
        let mut expected = prefix;
        expected.extend(self.number((kept_table.len() / entry_bytes) as u64, count_bytes));
        expected.extend_from_slice(&kept_table);
        expected.extend_from_slice(&next);
        expected.resize(table_end + offset_bytes, 0);
        expected.extend_from_slice(&data);
        (source, expected)
    }
}

/// All layout combinations share the same compaction contract, including JPEG-prefixed EXIF.
fn layouts() -> impl Iterator<Item = Layout> {
    [false, true].into_iter().flat_map(|big_tiff| [false, true].into_iter().map(move |big_endian| Layout { big_tiff, big_endian }))
}

/// Every removal mask exercises first/middle/last entries, adjacent runs, all and none removed.
/// Mixed valid duplicates must still be rewritten and unrelated payload offsets must not move.
#[test]
fn compaction_preserves_complete_blocks_for_removal_patterns() {
    for layout in layouts() {
        for mask in 0..256 {
            let cases: Vec<u8> = (0..8).map(|i| if mask & (1 << i) != 0 { 1 + (i % 3) as u8 } else { [0, 4, 5, 6][i % 4] }).collect();
            let (source, expected) = layout.fixture(&cases);
            for prefix in [b"".as_slice(), b"Exif\0\0".as_slice()] {
                let source = [prefix, &source].concat();
                let expected = [prefix, &expected].concat();
                assert_eq!(exif_with_resolution(&source, (240.0, 240.0)).as_ref(), expected, "mask {mask}");
            }
        }
        let (source, expected) = layout.fixture(&[7, 4, 1, 4, 8, 5, 2, 5, 3, 6, 9, 0]);
        assert_eq!(exif_with_resolution(&source, (240.0, 240.0)).as_ref(), expected);
    }
}

/// Invalid output resolution, empty/missing metadata and already-correct first duplicates keep
/// the borrowed fast path; truncated or invalid table layouts cannot be partially compacted.
#[test]
fn cleanup_preserves_noop_and_invalid_layout_behavior() {
    for layout in layouts() {
        let (source, _) = layout.fixture(&[1, 4, 2, 5, 3, 6, 0]);
        for ppi in [(0.0, 240.0), (-1.0, 240.0), (f32::NAN, 240.0), (240.0, f32::INFINITY), (1_000_001.0, 240.0)] {
            assert!(matches!(exif_with_resolution(&source, ppi), Cow::Borrowed(_)));
        }
        let table_end = if layout.big_tiff { 24 + 7 * 20 + 8 } else { 10 + 7 * 12 + 4 };
        for n in 0..table_end {
            assert!(matches!(exif_with_resolution(&source[..n], (240.0, 240.0)), Cow::Borrowed(_)), "length {n}");
        }
        let mut invalid = source;
        invalid[0] = 0;
        assert!(matches!(exif_with_resolution(&invalid, (240.0, 240.0)), Cow::Borrowed(_)));
        for cases in [&[][..], &[0][..], &[4, 5, 4, 1][..]] {
            let (source, _) = layout.fixture(cases);
            assert!(matches!(exif_with_resolution(&source, (300.0, 300.0)), Cow::Borrowed(_)));
        }
    }
    assert!(matches!(exif_with_resolution(&[], (240.0, 240.0)), Cow::Borrowed(_)));
}

/// A small PNG can retain many malformed eXIf entries on decode. Export must clean them while
/// preserving pixels at both PNG depths, unrelated metadata, and the metadata-disabled option.
#[test]
fn png_decode_export_cleans_large_repeated_resolution_metadata() {
    let layout = Layout { big_tiff: true, big_endian: false };
    let mut cases = vec![1; 40_000];
    cases.insert(0, 0);
    cases.insert(20_000, 0);
    cases.push(0);
    let (source, mut expected) = layout.fixture(&cases);
    // Export intentionally clears the source thumbnail pointer before compaction.
    expected[24 + 3 * 20..24 + 3 * 20 + 8].fill(0);
    for depth in [png::BitDepth::Eight, png::BitDepth::Sixteen] {
        let pixels = match depth {
            png::BitDepth::Eight => vec![255, 0, 10, 255, 0, 200, 30, 128, 0, 0, 255, 255, 77, 88, 99, 0],
            _ => (0..16u16).flat_map(|v| (v * 4097).to_be_bytes()).collect(),
        };
        let mut info = png::Info::with_size(2, 2);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = depth;
        info.exif_metadata = Some(source.clone().into());
        info.uncompressed_latin1_text.push(png::text_metadata::TEXtChunk::new("Comment", "keep this text"));
        info.utf8_text.push(png::text_metadata::ITXtChunk::new("XML:com.adobe.xmp", "<x:xmpmeta>unrelated</x:xmpmeta>"));
        let mut input = Vec::new();
        png::Encoder::with_info(&mut input, info).unwrap().write_header().unwrap().write_image_data(&pixels).unwrap();
        let mut image = decode(&input).unwrap();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.layout(), ChannelLayout::Rgba);
        assert_eq!(image.sample_type(), if depth == png::BitDepth::Eight { SampleType::U8 } else { SampleType::U16 });
        let native_pixels: Vec<u8> = if depth == png::BitDepth::Eight {
            pixels.clone()
        } else {
            pixels.as_chunks::<2>().0.iter().flat_map(|p| u16::from_be_bytes(*p).to_ne_bytes()).collect()
        };
        assert_eq!(image.data(), native_pixels);
        assert_eq!(image.meta.exif.as_deref(), Some(source.as_slice()));
        image.meta.dpi = Some((240.0, 240.0));
        let output = encode(&image, Format::Png, &EncodeOptions::default()).unwrap();
        let restored = decode(&output).unwrap();
        assert_eq!(restored.data(), image.data());
        assert_eq!(restored.dimensions(), image.dimensions());
        assert_eq!(restored.layout(), image.layout());
        assert_eq!(restored.sample_type(), image.sample_type());
        assert_eq!(restored.meta.exif.as_deref(), Some(expected.as_slice()));
        assert_eq!(restored.meta.text, image.meta.text);
        assert_eq!(restored.meta.xmp, image.meta.xmp);
        assert_eq!(restored.meta.dpi, Some((240.0, 240.0)));
        let output = encode(&image, Format::Png, &EncodeOptions { embed_metadata: false, ..Default::default() }).unwrap();
        let stripped = decode(&output).unwrap();
        assert_eq!(stripped.data(), image.data());
        assert!(stripped.meta.exif.is_none() && stripped.meta.xmp.is_none() && stripped.meta.text.is_empty());
    }
}
