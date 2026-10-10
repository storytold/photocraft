//! Public JPEG 2000 contracts, with generated images and an independent native-depth oracle.

mod common;

use common::{SAMPLE_XMP, psnr, sample_exif, sample_icc, synth};
use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image as HayroImage};
use oxideav_jpeg2000::{self as backend, Container, Jpeg2000Image, PixelFormat};
use photocraft_codecs::*;

const LAYOUTS: [ChannelLayout; 4] = [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba];
const SAMPLES: [SampleType; 2] = [SampleType::U8, SampleType::U16];
const JP2_SIGNATURE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";

fn native_image(width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Image {
    let max = if sample == SampleType::U16 { u16::MAX } else { 255 };
    let mut rng = common::Rng::new(0x1047);
    let mut values: Vec<u16> = (0..width as usize * height as usize * layout.channels()).map(|_| rng.next_u64() as u16 & max).collect();
    for (v, pinned) in values.iter_mut().zip([0, max, 1, max / 2 + 1]) {
        *v = pinned;
    }
    if sample == SampleType::U16 {
        Image::from_u16(width, height, layout, &values).unwrap()
    } else {
        Image::from_u8(width, height, layout, values.into_iter().map(|v| v as u8).collect()).unwrap()
    }
}

fn native_samples(image: &Image) -> Vec<u16> {
    if image.sample_type() == SampleType::U16 { image.to_u16_samples().unwrap() } else { image.data().iter().copied().map(u16::from).collect() }
}

fn independent_samples(bytes: &[u8], original: &Image) {
    let image = HayroImage::new(bytes, &DecodeSettings::default()).unwrap();
    assert_eq!((image.width(), image.height()), original.dimensions());
    let mut context = DecoderContext::default();
    let decoded = image.decode(&mut context).unwrap();
    let channels = original.layout().channels();
    let values = native_samples(original);
    assert_eq!(decoded.components().len(), channels);
    for (channel, component) in decoded.components().iter().enumerate() {
        assert_eq!(usize::from(component.bit_depth()), original.sample_type().bytes() * 8);
        assert_eq!(component.samples().len(), original.pixel_count());
        for (pixel, &actual) in component.samples().iter().enumerate() {
            assert!(actual.is_finite());
            assert_eq!(actual.round(), f32::from(values[pixel * channels + channel]), "channel {channel}, pixel {pixel}");
        }
    }
}

// Our fixtures use only normal 32-bit box lengths. This helper inspects the emitted format,
// without depending on the same backend's interpretation of its own header.
fn box_range(bytes: &[u8], kind: &[u8; 4], start: usize) -> std::ops::Range<usize> {
    let mut at = start;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(size >= 8);
        if &bytes[at + 4..at + 8] == kind {
            return at..at + size;
        }
        at += size;
    }
    panic!("generated container lacks {kind:?}");
}

fn codestream(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(JP2_SIGNATURE) {
        let range = box_range(bytes, b"jp2c", 12);
        &bytes[range.start + 8..range.end]
    } else {
        bytes
    }
}

fn assert_coding(bytes: &[u8], reversible: bool, mct: bool) {
    let stream = codestream(bytes);
    assert!(stream.starts_with(&[0xff, 0x4f, 0xff, 0x51]));
    let cod = stream.windows(2).position(|v| v == [0xff, 0x52]).unwrap();
    // COD: Scod, progression, layers, MCT, decomposition, block size/style, transform.
    assert_eq!(stream[cod + 8], u8::from(mct));
    assert_eq!(stream[cod + 13], u8::from(reversible), "5/3 = 1; 9/7 = 0");
}

fn assert_roundtrip(original: &Image, raw: bool) {
    let opts = EncodeOptions { jpeg2000_codestream: raw, ..Default::default() };
    assert!(fidelity_warnings_with(original, Format::Jpeg2000, &opts).is_empty());
    let bytes = encode(original, Format::Jpeg2000, &opts).unwrap();
    assert_eq!(bytes.starts_with(JP2_SIGNATURE), !raw);
    assert_eq!(detect(&bytes), Some(Format::Jpeg2000));
    assert_coding(&bytes, true, original.layout().is_rgb());
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.dimensions(), original.dimensions());
    assert_eq!(decoded.layout(), original.layout());
    assert_eq!(decoded.sample_type(), original.sample_type());
    assert_eq!(decoded.data(), original.data());
    assert!(decoded.warnings.is_empty());
    independent_samples(&bytes, original);
}

#[test]
fn lossless_all_layouts_and_native_depths_in_jp2_and_raw() {
    for layout in LAYOUTS {
        for sample in SAMPLES {
            for raw in [false, true] {
                assert_roundtrip(&native_image(37, 23, layout, sample), raw);
            }
        }
    }
}

#[test]
fn tiny_and_odd_images_keep_alpha_and_low_sixteen_bit_values() {
    for (width, height) in [(1, 1), (5, 3)] {
        for layout in LAYOUTS {
            for sample in SAMPLES {
                for raw in [false, true] {
                    assert_roundtrip(&native_image(width, height, layout, sample), raw);
                }
            }
        }
    }
}

fn backend_format(layout: ChannelLayout, sample: SampleType) -> PixelFormat {
    match (layout, sample) {
        (ChannelLayout::Gray, SampleType::U8) => PixelFormat::Gray8,
        (ChannelLayout::GrayA, SampleType::U8) => PixelFormat::Ya8,
        (ChannelLayout::Rgb, SampleType::U8) => PixelFormat::Rgb24,
        (ChannelLayout::Rgba, SampleType::U8) => PixelFormat::Rgba,
        (ChannelLayout::Gray, SampleType::U16) => PixelFormat::Gray16Le,
        (ChannelLayout::GrayA, SampleType::U16) => PixelFormat::Ya16Le,
        (ChannelLayout::Rgb, SampleType::U16) => PixelFormat::Rgb48Le,
        (ChannelLayout::Rgba, SampleType::U16) => PixelFormat::Rgba64Le,
        _ => panic!("fixture requires an integer Gray/RGB layout"),
    }
}

fn backend_image(image: &Image) -> Jpeg2000Image {
    let data =
        if image.sample_type() == SampleType::U16 { native_samples(image).into_iter().flat_map(u16::to_le_bytes).collect() } else { image.data().to_vec() };
    Jpeg2000Image::packed(image.width(), image.height(), backend_format(image.layout(), image.sample_type()), data).unwrap()
}

#[test]
fn tiled_codestreams_decode_across_all_layouts_and_depths() {
    for layout in LAYOUTS {
        for sample in SAMPLES {
            let original = native_image(37, 23, layout, sample);
            for container in [Container::J2k, Container::Jp2] {
                let opts = backend::EncodeOptions::new().with_container(container).with_tile_size((11, 9));
                let bytes = backend::encode(&backend_image(&original), &opts).unwrap();
                assert!(backend::info(&bytes).unwrap().tiles > 1);
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.layout(), layout);
                assert_eq!(decoded.sample_type(), sample);
                assert_eq!(decoded.data(), original.data());
                independent_samples(&bytes, &original);
            }
        }
    }
}

#[test]
fn significant_precision_is_scaled_to_the_full_integer_range() {
    for bits in [1_u8, 4, 10, 12] {
        let sample = if bits <= 8 { SampleType::U8 } else { SampleType::U16 };
        let max = (1_u32 << bits) - 1;
        let target_max = if bits <= 8 { 255_u32 } else { 65535_u32 };
        for layout in LAYOUTS {
            let count = 13 * 7 * layout.channels();
            let mut values: Vec<u16> = (0..count).map(|i| ((i as u32 * 53 + 7) % (max + 1)) as u16).collect();
            values[..4].copy_from_slice(&[0, max as u16, 1, (max / 2) as u16]);
            let data: Vec<u8> = if bits <= 8 { values.iter().map(|&v| v as u8).collect() } else { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
            let image = Jpeg2000Image::packed(13, 7, backend_format(layout, sample), data).unwrap().with_bit_depth(bits).unwrap();
            let expected: Vec<u16> = values.iter().map(|&v| ((u32::from(v) * target_max + max / 2) / max) as u16).collect();
            for container in [Container::J2k, Container::Jp2] {
                let bytes = backend::encode(&image, &backend::EncodeOptions::new().with_container(container)).unwrap();
                assert_eq!(backend::info(&bytes).unwrap().bit_depth, bits);
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.layout(), layout);
                assert_eq!(decoded.sample_type(), sample);
                assert_eq!(native_samples(&decoded), expected, "{bits} bits {layout:?} {container:?}");
            }
        }
    }
}

fn metadata_image() -> Image {
    let mut image = native_image(37, 23, ChannelLayout::Rgb, SampleType::U16);
    let mut icc = sample_icc(1536);
    icc[16..20].copy_from_slice(b"RGB ");
    image.icc = Some(icc);
    image.meta = Metadata { exif: Some(sample_exif()), xmp: Some(SAMPLE_XMP.into()), dpi: Some((300.0, 150.0)), ..Default::default() };
    image
}

#[test]
fn jp2_preserves_icc_and_capture_resolution_and_warns_about_exif_xmp() {
    let image = metadata_image();
    let warnings = fidelity_warnings(&image, Format::Jpeg2000);
    assert_eq!(warnings, vec![FidelityWarning::ExifDropped, FidelityWarning::XmpDropped]);
    let bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let header = backend::jp2::parse_jp2(&bytes).unwrap().header;
    assert!(header.colr.iter().any(|c| c.icc_profile == image.icc));
    assert!(header.resolution.unwrap().capture.is_some());
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.data(), image.data());
    assert_eq!(decoded.icc, image.icc);
    let (x, y) = decoded.meta.dpi.unwrap();
    assert!((x - 300.0).abs() < 0.05 && (y - 150.0).abs() < 0.05, "DPI {x}, {y}");
    assert!(decoded.meta.exif.is_none() && decoded.meta.xmp.is_none());
}

#[test]
fn raw_export_reports_and_drops_container_metadata() {
    let image = metadata_image();
    let opts = EncodeOptions { jpeg2000_codestream: true, ..Default::default() };
    let warnings = fidelity_warnings_with(&image, Format::Jpeg2000, &opts);
    assert_eq!(warnings, vec![FidelityWarning::IccDropped, FidelityWarning::ExifDropped, FidelityWarning::XmpDropped, FidelityWarning::DpiDropped]);
    let bytes = encode(&image, Format::Jpeg2000, &opts).unwrap();
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.data(), image.data());
    assert!(decoded.icc.is_none() && decoded.meta.is_empty());
}

fn push_box(bytes: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    bytes.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(body);
}

#[test]
fn xml_and_uuid_imports_report_metadata_dropped() {
    let original = native_image(5, 3, ChannelLayout::Rgba, SampleType::U16);
    let base = encode(&original, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    for (kind, body) in [(b"xml ", SAMPLE_XMP.as_bytes()), (b"uuid", &[0_u8; 16][..])] {
        let mut bytes = base.clone();
        push_box(&mut bytes, kind, body);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.data(), original.data());
        assert!(decoded.meta.xmp.is_none());
        assert_eq!(decoded.warnings, vec![DecodeWarning::MetadataDropped { format: Format::Jpeg2000 }]);
    }
}

#[test]
fn float_export_quantizes_to_sixteen_bits_with_honest_warnings() {
    let values = [-0.25, 0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 0.125];
    for sample in [SampleType::F16, SampleType::F32] {
        let image = Image::from_normalized(2, 1, ChannelLayout::Rgba, sample, &values).unwrap();
        let warnings = fidelity_warnings(&image, Format::Jpeg2000);
        assert!(warnings.contains(&FidelityWarning::DepthReduced { from: sample, to: SampleType::U16 }));
        assert!(warnings.contains(&FidelityWarning::RangeClipped));
        for raw in [false, true] {
            let opts = EncodeOptions { jpeg2000_codestream: raw, ..Default::default() };
            let decoded = decode(&encode(&image, Format::Jpeg2000, &opts).unwrap()).unwrap();
            assert_eq!(decoded.sample_type(), SampleType::U16);
            assert_eq!(decoded.data(), image.convert(ChannelLayout::Rgba, SampleType::U16).data());
        }
    }
}

#[test]
fn cmyk_conversion_discards_the_incompatible_icc_profile() {
    for layout in [ChannelLayout::Cmyk, ChannelLayout::CmykA] {
        let image = native_image(5, 3, layout, SampleType::U16).with_icc(Some(sample_icc(512)));
        let target = if layout.has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
        let warnings = fidelity_warnings(&image, Format::Jpeg2000);
        assert!(warnings.contains(&FidelityWarning::CmykConverted { to: target }));
        assert!(warnings.contains(&FidelityWarning::IccDropped));
        let bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.layout(), target);
        assert_eq!(decoded.sample_type(), SampleType::U16);
        assert_eq!(decoded.data(), image.convert(target, SampleType::U16).data());
        assert!(decoded.icc.is_none());
    }
}

#[test]
fn quality_eighty_uses_nine_seven_and_reaches_a_measured_psnr_floor() {
    for sample in SAMPLES {
        for layout in LAYOUTS {
            for noise in [0.0, 1.0] {
                let image = synth(37, 23, layout, sample, 0x1047, noise);
                for raw in [false, true] {
                    let opts = EncodeOptions { jpeg2000_quality: Some(80), jpeg2000_codestream: raw, ..Default::default() };
                    assert_eq!(fidelity_warnings_with(&image, Format::Jpeg2000, &opts), vec![FidelityWarning::LossyCompression]);
                    let bytes = encode(&image, Format::Jpeg2000, &opts).unwrap();
                    assert_coding(&bytes, false, matches!(layout, ChannelLayout::Rgb | ChannelLayout::Rgba));
                    let decoded = decode(&bytes).unwrap();
                    assert_eq!(decoded.sample_type(), sample);
                    assert_eq!(decoded.layout(), image.layout());
                    let measured = psnr(&image, &decoded);
                    assert!(measured >= 35.0, "{layout:?} {sample:?}, noise {noise}, raw {raw}: PSNR {measured:.3} dB");
                }
            }
        }
    }
}

#[test]
fn quality_endpoints_produce_valid_native_depth_streams_with_rate_allocation() {
    for layout in LAYOUTS {
        for sample in SAMPLES {
            let image = native_image(37, 23, layout, sample);
            let mut previous = None;
            for q in [1, 100] {
                let opts = EncodeOptions { jpeg2000_quality: Some(q), jpeg2000_codestream: true, ..Default::default() };
                let bytes = encode(&image, Format::Jpeg2000, &opts).unwrap();
                assert_coding(&bytes, false, matches!(layout, ChannelLayout::Rgb | ChannelLayout::Rgba));
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.dimensions(), image.dimensions());
                assert_eq!(decoded.layout(), layout);
                assert_eq!(decoded.sample_type(), sample);
                let measured = psnr(&image, &decoded);
                if q == 1 {
                    assert!(bytes.len() <= 512 + image.data().len() / 10_000);
                } else {
                    assert!(bytes.len() <= 512 + image.data().len());
                    let (length, low_psnr) = previous.unwrap();
                    assert!(bytes.len() >= length);
                    assert!(measured >= low_psnr);
                }
                previous = Some((bytes.len(), measured));
            }
            let tiny = native_image(1, 1, layout, sample);
            let opts = EncodeOptions { jpeg2000_quality: Some(1), ..Default::default() };
            let decoded = decode(&encode(&tiny, Format::Jpeg2000, &opts).unwrap()).unwrap();
            assert_eq!(decoded.dimensions(), (1, 1));
            assert_eq!(decoded.layout(), layout);
            assert_eq!(decoded.sample_type(), sample);
        }
    }
}

#[test]
fn quality_outside_one_through_one_hundred_is_an_encode_error() {
    let image = native_image(1, 1, ChannelLayout::Gray, SampleType::U8);
    for quality in [0, 101] {
        let opts = EncodeOptions { jpeg2000_quality: Some(quality), ..Default::default() };
        assert!(matches!(encode(&image, Format::Jpeg2000, &opts), Err(CodecError::Encode { format: Format::Jpeg2000, .. })));
    }
}

fn bounded_options() -> DecodeOptions {
    DecodeOptions { limits: Limits { max_width: 64, max_height: 64, max_pixels: 4096, max_alloc: 16 << 20 }, ..Default::default() }
}

#[test]
fn width_height_pixel_and_working_byte_budgets_are_enforced() {
    let image = native_image(37, 23, ChannelLayout::Rgba, SampleType::U16);
    for raw in [false, true] {
        let bytes = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: raw, ..Default::default() }).unwrap();
        assert!(decode_with(&bytes, &bounded_options()).is_ok());
        for limits in [
            Limits { max_width: 36, ..bounded_options().limits },
            Limits { max_height: 22, ..bounded_options().limits },
            Limits { max_pixels: 37 * 23 - 1, ..bounded_options().limits },
            Limits { max_alloc: image.data().len() as u64 - 1, ..bounded_options().limits },
        ] {
            let opts = DecodeOptions { limits, ..Default::default() };
            assert!(matches!(decode_with(&bytes, &opts), Err(CodecError::LimitExceeded(_))), "{limits:?}");
        }
    }
}

#[test]
fn huge_siz_dimensions_are_rejected_before_missing_image_packets() {
    let image = native_image(1, 1, ChannelLayout::Gray, SampleType::U8);
    let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: true, ..Default::default() }).unwrap();
    assert_eq!(&bytes[..4], &[0xff, 0x4f, 0xff, 0x51]);
    for offset in [8, 12, 24, 28] {
        bytes[offset..offset + 4].copy_from_slice(&1_000_000_u32.to_be_bytes());
    }
    // Leave SIZ only: the dimension guard must run before requiring coding or image packets.
    let siz_end = 4 + usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
    bytes.truncate(siz_end);
    let side = bounded_options();
    assert!(matches!(decode_as_with(Format::Jpeg2000, &bytes, &side), Err(CodecError::LimitExceeded(_))));
    let pixels = DecodeOptions { limits: Limits { max_width: u32::MAX, max_height: u32::MAX, ..side.limits }, ..Default::default() };
    assert!(matches!(decode_as_with(Format::Jpeg2000, &bytes, &pixels), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn truncation_and_bounded_bit_flips_never_escape_as_panics() {
    let image = native_image(37, 23, ChannelLayout::Rgba, SampleType::U16);
    for raw in [false, true] {
        let bytes = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: raw, ..Default::default() }).unwrap();
        for length in [0, 1, 2, 3, 4, 10, bytes.len() / 2, bytes.len() - 1] {
            let result = std::panic::catch_unwind(|| decode_as_with(Format::Jpeg2000, &bytes[..length], &bounded_options()));
            assert!(result.is_ok(), "panic at truncation {length}/{}", bytes.len());
            assert!(result.unwrap().is_err(), "accepted truncation {length}/{}", bytes.len());
        }
        for position in (0..bytes.len()).step_by((bytes.len() / 32).max(1)) {
            let mut corrupted = bytes.clone();
            corrupted[position] ^= 0x55;
            let result = std::panic::catch_unwind(|| decode_as_with(Format::Jpeg2000, &corrupted, &bounded_options()));
            assert!(result.is_ok(), "panic at bit flip {position}/{}", bytes.len());
            // Some changes describe another valid stream; success is allowed within limits.
            if let Ok(image) = result.unwrap() {
                assert!(image.width() <= 64 && image.height() <= 64 && image.pixel_count() <= 4096);
            }
        }
    }
}

#[test]
fn signed_subsampled_and_mixed_precision_components_are_explicitly_unsupported() {
    let image = native_image(5, 3, ChannelLayout::Rgb, SampleType::U8);
    let base = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: true, ..Default::default() }).unwrap();
    assert_eq!(&base[2..4], &[0xff, 0x51]);
    for (offset, value) in [(42, 0x87), (43, 2), (42, 6), (42, 16)] {
        let mut bytes = base.clone();
        bytes[offset] = value;
        assert!(
            matches!(decode_as_with(Format::Jpeg2000, &bytes, &bounded_options()), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })),
            "SIZ offset {offset} = {value}"
        );
    }
}

#[test]
fn more_than_four_components_are_explicitly_unsupported() {
    let image = native_image(5, 3, ChannelLayout::Rgba, SampleType::U8);
    let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: true, ..Default::default() }).unwrap();
    let size = u16::from_be_bytes(bytes[4..6].try_into().unwrap());
    let siz_end = 4 + usize::from(size);
    bytes[4..6].copy_from_slice(&(size + 3).to_be_bytes());
    bytes[40..42].copy_from_slice(&5_u16.to_be_bytes());
    bytes.splice(siz_end..siz_end, [7, 1, 1]);
    assert!(matches!(decode_as_with(Format::Jpeg2000, &bytes, &bounded_options()), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })));
}

#[test]
fn only_jp2_compatible_jpx_is_accepted() {
    let image = native_image(5, 3, ChannelLayout::Gray, SampleType::U8);
    let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let ftyp = box_range(&bytes, b"ftyp", 12);
    bytes[ftyp.start + 8..ftyp.start + 12].copy_from_slice(b"jpx ");
    assert_eq!(decode(&bytes).unwrap().data(), image.data());
    bytes[ftyp.start + 16..ftyp.end].as_chunks_mut::<4>().0.iter_mut().for_each(|brand| brand.copy_from_slice(b"jpx "));
    assert!(matches!(decode(&bytes), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })));
}

#[test]
fn palette_boxes_are_rejected_before_backend_table_allocation() {
    let image = native_image(5, 3, ChannelLayout::Gray, SampleType::U8);
    let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let header = box_range(&bytes, b"jp2h", 12);
    let mut palette = Vec::new();
    push_box(&mut palette, b"pclr", &[0xff, 0xff, 4, 15, 15, 15, 15]);
    let new_size = (header.len() + palette.len()) as u32;
    bytes[header.start..header.start + 4].copy_from_slice(&new_size.to_be_bytes());
    bytes.splice(header.end..header.end, palette);
    assert!(matches!(decode_with(&bytes, &bounded_options()), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })));
}

fn append_header_box(bytes: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    let header = box_range(bytes, b"jp2h", 12);
    let mut child = Vec::new();
    push_box(&mut child, kind, body);
    bytes[header.start..header.start + 4].copy_from_slice(&((header.len() + child.len()) as u32).to_be_bytes());
    bytes.splice(header.end..header.end, child);
}

fn replace_header_box(bytes: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    let header = box_range(bytes, b"jp2h", 12);
    let old = box_range(bytes, kind, header.start + 8);
    let mut child = Vec::new();
    push_box(&mut child, kind, body);
    bytes[header.start..header.start + 4].copy_from_slice(&((header.len() - old.len() + child.len()) as u32).to_be_bytes());
    bytes.splice(old, child);
}

fn assert_header_rejected(bytes: &[u8], what: &str) {
    let result = decode_with(bytes, &bounded_options());
    assert!(
        matches!(result, Err(CodecError::Malformed { format: Format::Jpeg2000, .. }) | Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })),
        "{what}: {result:?}"
    );
}

#[test]
fn oversized_file_type_compatibility_is_budgeted_before_parsing_other_headers() {
    let image = native_image(1, 1, ChannelLayout::Gray, SampleType::U8);
    let base = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let old = box_range(&base, b"ftyp", 12);
    let mut body = b"jp2 \0\0\0\0jp2 ".to_vec();
    while body.len() < 32 << 10 {
        body.extend_from_slice(b"test");
    }
    let mut replacement = Vec::new();
    push_box(&mut replacement, b"ftyp", &body);
    let mut bytes = base.clone();
    bytes.splice(old.clone(), replacement.clone());
    let options = DecodeOptions { limits: Limits { max_alloc: 16 << 10, ..bounded_options().limits }, ..Default::default() };
    assert!(matches!(decode_with(&bytes, &options), Err(CodecError::LimitExceeded(_))));
    // Even with the mandatory image header absent, the oversized table must hit the budget
    // first. This proves the test is not passing because of later sample/scratch accounting.
    let mut header_only = base[..old.start].to_vec();
    header_only.extend_from_slice(&replacement);
    assert!(matches!(decode_as_with(Format::Jpeg2000, &header_only, &options), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn duplicate_mandatory_container_and_channel_boxes_are_rejected() {
    let image = native_image(5, 3, ChannelLayout::Rgba, SampleType::U8);
    let base = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    for kind in [b"ftyp", b"jp2h"] {
        let range = box_range(&base, kind, 12);
        let duplicate = base[range.clone()].to_vec();
        let mut bytes = base.clone();
        bytes.splice(range.end..range.end, duplicate);
        assert_header_rejected(&bytes, &format!("duplicate {kind:?}"));
    }
    let header = box_range(&base, b"jp2h", 12);
    for kind in [b"ihdr", b"cdef"] {
        let range = box_range(&base, kind, header.start + 8);
        let mut bytes = base.clone();
        append_header_box(&mut bytes, kind, &base[range.start + 8..range.end]);
        assert_header_rejected(&bytes, &format!("duplicate {kind:?}"));
    }
    let mut bytes = base;
    let ihdr = box_range(&bytes, b"ihdr", header.start + 8);
    bytes[ihdr.start + 18] = 0xff; // BPC is carried by the per-component box.
    append_header_box(&mut bytes, b"bpcc", &[7; 4]);
    append_header_box(&mut bytes, b"bpcc", &[7; 4]);
    assert_header_rejected(&bytes, "duplicate bpcc");
}

fn profile_container(layout: ChannelLayout, signature: &[u8; 4]) -> (Image, Vec<u8>, Vec<u8>) {
    let image = native_image(5, 3, layout, SampleType::U16);
    let mut profile = sample_icc(512);
    profile[16..20].copy_from_slice(signature);
    let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let mut colr = vec![2, 0, 0]; // Restricted ICC method, precedence and approximation.
    colr.extend_from_slice(&profile);
    replace_header_box(&mut bytes, b"colr", &colr);
    (image, profile, bytes)
}

#[test]
fn recognized_incompatible_icc_device_spaces_are_explicitly_unsupported() {
    for signature in [b"CMYK", b"Lab ", b"XYZ "] {
        for layout in [ChannelLayout::Gray, ChannelLayout::Rgb] {
            let (_, _, bytes) = profile_container(layout, signature);
            assert!(
                matches!(decode_with(&bytes, &bounded_options()), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })),
                "{layout:?} with {signature:?} profile"
            );
        }
    }
    for (layout, signature) in [(ChannelLayout::Gray, b"RGB "), (ChannelLayout::Rgb, b"GRAY")] {
        let (_, _, bytes) = profile_container(layout, signature);
        assert!(
            matches!(decode_with(&bytes, &bounded_options()), Err(CodecError::Unsupported { format: Format::Jpeg2000, .. })),
            "{layout:?} with {signature:?} profile"
        );
    }
}

#[test]
fn compatible_and_unrecognized_synthetic_icc_profiles_remain_opaque_and_preserved() {
    for (layout, signature) in [(ChannelLayout::Gray, b"GRAY"), (ChannelLayout::Rgb, b"RGB "), (ChannelLayout::Gray, b"test"), (ChannelLayout::Rgb, b"test")] {
        let (image, profile, bytes) = profile_container(layout, signature);
        let decoded = decode_with(&bytes, &bounded_options()).unwrap();
        assert_eq!(decoded.data(), image.data());
        assert_eq!(decoded.icc.as_deref(), Some(profile.as_slice()));
    }
}

fn channel_definitions(entries: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut body = (entries.len() as u16).to_be_bytes().to_vec();
    for &(channel, kind, association) in entries {
        for value in [channel, kind, association] {
            body.extend_from_slice(&value.to_be_bytes());
        }
    }
    body
}

#[test]
fn channel_definitions_cannot_silently_change_the_number_of_color_channels() {
    let image = native_image(5, 3, ChannelLayout::Rgb, SampleType::U8);
    let mut rgb = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    append_header_box(&mut rgb, b"cdef", &channel_definitions(&[(0, 0, 1), (1, 0, 2), (2, 1, 0)]));
    assert_header_rejected(&rgb, "three components with only two colors and alpha");

    let image = native_image(5, 3, ChannelLayout::Rgba, SampleType::U8);
    let mut rgba = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    replace_header_box(&mut rgba, b"cdef", &channel_definitions(&[(0, 0, 1), (1, 0, 2), (2, 0, 3), (3, 0, 4)]));
    assert_header_rejected(&rgba, "four color components with no alpha");
}

#[test]
fn channel_definition_entry_order_does_not_change_valid_gray_or_rgb_alpha() {
    for layout in [ChannelLayout::GrayA, ChannelLayout::Rgba] {
        let image = native_image(5, 3, layout, SampleType::U16);
        let mut bytes = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
        let mut entries: Vec<_> = (0..layout.color_channels()).map(|c| (c as u16, 0, (c + 1) as u16)).collect();
        entries.push(((layout.channels() - 1) as u16, 1, 0));
        entries.reverse();
        replace_header_box(&mut bytes, b"cdef", &channel_definitions(&entries));
        let decoded = decode_with(&bytes, &bounded_options()).unwrap();
        assert_eq!(decoded.layout(), layout);
        assert_eq!(decoded.data(), image.data());
    }
}

#[test]
fn container_metadata_and_codestream_work_share_the_allocation_budget() {
    let image = native_image(32, 24, ChannelLayout::Gray, SampleType::U8);
    let options = DecodeOptions { limits: Limits { max_alloc: 100 << 10, ..bounded_options().limits }, ..Default::default() };
    let raw = encode(&image, Format::Jpeg2000, &EncodeOptions { jpeg2000_codestream: true, ..Default::default() }).unwrap();
    let plain_jp2 = encode(&image, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    assert_eq!(decode_with(&raw, &options).unwrap().data(), image.data());
    assert_eq!(decode_with(&plain_jp2, &options).unwrap().data(), image.data());

    let mut profile = sample_icc(10 << 10);
    profile[16..20].copy_from_slice(b"GRAY");
    let rich = image.with_icc(Some(profile));
    let jp2 = encode(&rich, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    assert_eq!(codestream(&jp2), raw.as_slice());
    let decoded = decode(&jp2).unwrap();
    assert_eq!(decoded.data(), rich.data());
    assert_eq!(decoded.icc, rich.icc);

    // A container without its final codestream reaches the missing-stream error, proving
    // the metadata by itself fits. The identical pixel work also fits above; their sum does not.
    let stream = box_range(&jp2, b"jp2c", 12);
    assert!(matches!(decode_as_with(Format::Jpeg2000, &jp2[..stream.start], &options), Err(CodecError::Malformed { format: Format::Jpeg2000, .. })));
    assert!(matches!(decode_with(&jp2, &options), Err(CodecError::LimitExceeded(_))));
}
