//! #13724: a JPG from Export As is progressive (SOF2) at every quality, keeps today's chroma
//! subsampling and opens again with the same pixels.

use super::*;
use photocraft_doc::{Color, ColorMode, SampleType, Size};

/// The marker of the main JPEG frame (SOF0 = 0xc0 baseline, SOF2 = 0xc2 progressive) and its
/// components' sampling factors. Whole segments are skipped, so an APP thumbnail's frame is not
/// taken for the main one.
fn frame(bytes: &[u8]) -> (u8, [(u8, u8); 3]) {
    assert_eq!(bytes.get(..2), Some([0xff, 0xd8].as_slice()), "JPEG SOI");
    let mut offset = 2;
    loop {
        assert_eq!(bytes.get(offset), Some(&0xff), "JPEG marker prefix");
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset).expect("JPEG marker");
        offset += 1;
        match marker {
            0x01 | 0xd0..=0xd8 => continue,
            0x00 | 0xd9 | 0xda => panic!("JPEG has no frame before scan/end"),
            _ => {}
        }
        let length = bytes.get(offset..offset + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))).expect("JPEG segment length");
        let segment = bytes.get(offset + 2..offset + length).expect("complete JPEG segment");
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            assert_eq!(segment.get(5), Some(&3), "three components");
            return (marker, std::array::from_fn(|i| (segment[6 + 3 * i + 1] >> 4, segment[6 + 3 * i + 1] & 0x0f)));
        }
        offset += length;
    }
}

#[test]
fn export_as_writes_progressive_jpgs_that_reopen_with_the_same_pixels() {
    let colour = Color::rgb(0.2, 0.5, 0.8);
    let doc = Document::with_background("progressive", Size::new(24, 16), ColorMode::Rgb, SampleType::U8, colour);
    let want = [51u8, 128, 204];
    for quality in [1, 30, 50, 51, 85, 100] {
        let fields = json!({"format": "jpg", "quality": quality, "scale": 100}).as_object().unwrap().clone();
        let out = export_document(&doc, &fields, None, &photocraft_engine::color_cmds::document_profile(&doc)).unwrap();
        let encoded = photocraft_io::export(&out, "progressive.jpg", &settings(&fields).io_options()).unwrap();
        let (marker, sampling) = frame(&encoded.bytes);
        assert_eq!(marker, 0xc2, "quality {quality}: progressive (SOF2)");
        let expected = if quality <= 50 { [(2, 2), (1, 1), (1, 1)] } else { [(1, 1); 3] };
        assert_eq!(sampling, expected, "quality {quality}: chroma subsampling as before");
        let decoded = photocraft_codecs::decode(&encoded.bytes).unwrap().convert(photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8);
        assert_eq!(decoded.dimensions(), (24, 16), "quality {quality}");
        // At the default quality and above, a flat colour comes back within 2 per channel.
        if quality >= 85 {
            for pixel in decoded.data().as_chunks::<3>().0 {
                for (got, want) in pixel.iter().zip(want) {
                    assert!(got.abs_diff(want) <= 2, "quality {quality}: {pixel:?} against {want:?}");
                }
            }
        }
    }
}
