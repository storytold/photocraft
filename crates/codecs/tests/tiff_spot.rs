use photocraft_codecs::{ChannelLayout, DecodeOptions, EncodeOptions, Image, SampleType, TiffCompression};
use std::io::Cursor;
use tiff::{decoder::Decoder, tags::Tag};

#[test]
fn spot_is_unspecified_extra_sample_and_keeps_its_polarity() {
    for alpha in [false, true] {
        for compression in [TiffCompression::None, TiffCompression::Lzw, TiffCompression::Deflate, TiffCompression::PackBits] {
            let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
            let pixels = if alpha { vec![10, 20, 30, 0, 40, 50, 60, 127, 70, 80, 90, 255] } else { vec![10, 20, 30, 40, 50, 60, 70, 80, 90] };
            let image = Image::from_raw(3, 1, layout, SampleType::U8, pixels.clone()).unwrap();
            let spot = [255, 127, 0]; // Photoshop: no ink, partial ink, full ink.
            let opts = EncodeOptions { tiff_compression: compression, ..Default::default() };
            let bytes = photocraft_codecs::encode_tiff_rgb_spot(&image, &spot, &opts).unwrap();
            let mut raw = Decoder::new(Cursor::new(&bytes)).unwrap();
            assert_eq!(raw.get_tag_u16_vec(Tag::ExtraSamples).unwrap(), if alpha { vec![2, 0] } else { vec![0] });
            assert_eq!(raw.get_tag_u32(Tag::SamplesPerPixel).unwrap(), if alpha { 5 } else { 4 });
            let decoded = photocraft_codecs::decode_with(&bytes, &DecodeOptions::default()).unwrap();
            assert_eq!(decoded.layout(), layout);
            assert_eq!(decoded.data(), pixels);
            assert_eq!(decoded.meta.tiff_extra_samples, vec![spot.to_vec()]);
        }
    }
}

#[test]
fn invalid_spot_buffers_are_errors() {
    let rgb = Image::from_raw(1, 1, ChannelLayout::Rgb, SampleType::U8, vec![1, 2, 3]).unwrap();
    assert!(photocraft_codecs::encode_tiff_rgb_spot(&rgb, &[], &Default::default()).is_err());
    assert!(photocraft_codecs::encode_tiff_rgb_spot(&rgb, &[1, 2], &Default::default()).is_err());
    let gray = Image::from_raw(1, 1, ChannelLayout::Gray, SampleType::U8, vec![1]).unwrap();
    assert!(photocraft_codecs::encode_tiff_rgb_spot(&gray, &[1], &Default::default()).is_err());
}

#[test]
fn orientation_keeps_spot_registered_with_artwork() {
    let mut img = Image::from_raw(3, 1, ChannelLayout::Rgb, SampleType::U8, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
    img.meta.tiff_extra_samples = vec![vec![255, 127, 0]];
    let flipped = img.oriented(2).unwrap();
    assert_eq!(flipped.data(), [7, 8, 9, 4, 5, 6, 1, 2, 3]);
    assert_eq!(flipped.meta.tiff_extra_samples, vec![vec![0, 127, 255]]);
}

#[path = "common/tiffgen.rs"]
mod tiffgen;

#[test]
fn independent_planar_and_chunky_spot_files_both_byte_orders() {
    use tiffgen::{Dir, LONG, SHORT, base, build};
    for little in [false, true] {
        for big in [false, true] {
            for planar in [false, true] {
                for alpha_first in [false, true] {
                    let extras = if alpha_first { [2, 0] } else { [0, 2] };
                    let mut planes = vec![vec![10u8, 40], vec![20, 50], vec![30, 60]];
                    if alpha_first {
                        planes.extend([vec![0, 127], vec![255, 0]]);
                    } else {
                        planes.extend([vec![255, 0], vec![0, 127]]);
                    }
                    let chunks = if planar { planes.clone() } else { vec![(0..2).flat_map(|i| planes.iter().map(move |p| p[i])).collect()] };
                    let dir = Dir { entries: base(2, 1, 8, 5, 2), chunks, tiled: false }
                        .tag(278, LONG, &[1])
                        .tag(284, SHORT, &[if planar { 2 } else { 1 }])
                        .tag(338, SHORT, &extras);
                    let file = build(little, big, &[dir], &[0]);
                    let img = photocraft_codecs::decode(&file.bytes).unwrap();
                    assert_eq!(img.data(), [10, 20, 30, 0, 40, 50, 60, 127]);
                    assert_eq!(img.meta.tiff_extra_samples, vec![vec![255, 0]]);
                }
            }
        }
    }
}

#[test]
fn missing_spot_plane_is_an_error_instead_of_a_silent_empty_plate() {
    use tiffgen::{Dir, LONG, SHORT, base, build};
    let dir = Dir { entries: base(2, 1, 8, 5, 2), chunks: vec![vec![10, 40], vec![20, 50], vec![30, 60], vec![255, 127], vec![255, 0]], tiled: false }
        .tag(278, LONG, &[1])
        .tag(284, SHORT, &[2])
        .tag(338, SHORT, &[2, 0]);
    let mut file = build(true, false, &[dir], &[0]);
    file.bytes.truncate(file.chunk_at[0][4] as usize + 1);
    assert!(photocraft_codecs::decode(&file.bytes).is_err());
}
