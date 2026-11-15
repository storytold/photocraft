use super::*;
use crate::HistoricalXcfSampleOrder as Order;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Valid canvas and image properties, deliberately ending before even the layer pointer table.
fn header(properties: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = b"gimp xcf v007\0".to_vec();
    for word in [317u32, 193, 0, 250] {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    for (kind, data) in properties {
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(data);
    }
    bytes.extend_from_slice(&[0; 8]);
    bytes
}

fn dpi(x: f32, y: f32) -> (u32, Vec<u8>) {
    (19, [x.to_be_bytes(), y.to_be_bytes()].concat())
}

/// The same independent one-block, uncompressed LZMA2 framing used by the native transport
/// tests: CRC32 block check, one index record and footer. No encoder feature is required.
fn xz(bytes: &[u8]) -> Vec<u8> {
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 128 });
            if value == 0 {
                break;
            }
        }
    }
    assert!(!bytes.is_empty());
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend(crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut block_header = vec![2, 0, 0x21, 1, 0, 0, 0, 0];
    block_header.extend(crc32fast::hash(&block_header).to_le_bytes());
    out.extend(&block_header);
    let mut data = Vec::new();
    for chunk in bytes.chunks(65536) {
        data.push(1);
        data.extend(((chunk.len() - 1) as u16).to_be_bytes());
        data.extend(chunk);
    }
    data.push(0);
    out.extend(&data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend(crc32fast::hash(bytes).to_le_bytes());
    let mut index = vec![0, 1];
    vli(block_header.len() + data.len() + 4, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend(crc32fast::hash(&index).to_le_bytes());
    out.extend(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend([0, 1]);
    out.extend(crc32fast::hash(&footer).to_le_bytes());
    out.extend(footer);
    out.extend(b"YZ");
    out
}

fn wrappers(bytes: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(bytes).unwrap();
    vec![("source.xcf.gz", gzip.finish().unwrap()), ("source.xcf.bz2", crabz2::compress(bytes, crabz2::Level::default())), ("source.xcf.xz", xz(bytes))]
}

#[test]
fn geometry_needs_no_drawable_or_sample_order_and_uses_native_resolution_policy() {
    for (properties, expected) in [
        (vec![], 72.),
        (vec![dpi(300., 150.)], 300.),
        (vec![dpi(300., 300.), dpi(144., 144.), dpi(600., f32::NAN)], 144.),
        (vec![dpi(1.0e6, 72.), (19, 300f32.to_be_bytes().to_vec())], 72.),
    ] {
        let bytes = header(&properties);
        for order in [None, Some(Order::LittleEndian), Some(Order::BigEndian)] {
            let options = crate::ImportOptions { historical_xcf_sample_order: order, ..Default::default() };
            assert!(crate::import_with_options_and_interrupt("source.xcf", &bytes, &options, &Interrupt::NONE).is_err(), "fixture cannot decode pixels");
            let progress = |_| panic!("geometry must not report drawable decode progress");
            let cancel = || false;
            let ctl = Interrupt::new(&cancel, &progress);
            assert_eq!(crate::smart_xcf_order::geometry("source.xcf", &bytes, order, &ctl).unwrap(), ((317., 193.), expected));
        }
    }
}

#[test]
fn wrapped_geometry_stops_at_properties_while_full_decode_validates_envelopes() {
    let bytes = header(&[dpi(240., 240.)]);
    for (name, wrapped) in wrappers(&bytes) {
        assert_eq!(crate::smart_xcf_order::geometry(name, &wrapped, Some(Order::LittleEndian), &Interrupt::NONE).unwrap(), ((317., 193.), 240.));
        assert_eq!(source_geometry("unknown.bin", &wrapped, &Interrupt::NONE).unwrap(), Some(((317., 193.), 240.)), "XCF signature claims the wrapper");
        let truncated = &wrapped[..wrapped.len() - 1];
        assert_eq!(
            source_geometry(name, truncated, &Interrupt::NONE).unwrap(),
            Some(((317., 193.), 240.)),
            "{name}: unread trailers are outside a prefix query"
        );
        assert!(transport::decode(name, truncated, &Interrupt::NONE).is_err(), "{name}: complete transport still validates trailers");
        let mut garbage = wrapped;
        garbage.extend_from_slice(b"not a stream");
        assert_eq!(source_geometry(name, &garbage, &Interrupt::NONE).unwrap(), Some(((317., 193.), 240.)));
        assert!(transport::decode(name, &garbage, &Interrupt::NONE).is_err(), "{name}: complete transport still rejects trailing input");
    }
}

#[test]
fn geometry_cancels_during_properties_and_wrapped_input() {
    let bytes = header(&vec![(999, vec![]); 100]);
    for (name, bytes) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
        let polls = AtomicUsize::new(0);
        let cancel = || polls.load(Ordering::Relaxed) >= 10 || polls.fetch_add(1, Ordering::Relaxed) >= 9;
        assert!(matches!(crate::smart_xcf_order::geometry(name, &bytes, None, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(polls.load(Ordering::Relaxed), 10);
    }
}

#[test]
fn geometry_preserves_canvas_and_property_limits() {
    let mut oversized = header(&[]);
    oversized[14..18].copy_from_slice(&(super::super::MAX_SIDE + 1).to_be_bytes());
    assert!(source_geometry("source.xcf", &oversized, &Interrupt::NONE).is_err());
    let too_many = header(&vec![(999, vec![]); 4098]);
    assert!(source_geometry("source.xcf", &too_many, &Interrupt::NONE).is_err());
    let mut excessive_payload = header(&[]);
    excessive_payload.truncate(excessive_payload.len() - 8);
    excessive_payload.extend_from_slice(&999u32.to_be_bytes());
    excessive_payload.extend_from_slice(&((super::super::MAX_PAYLOAD + 1) as u32).to_be_bytes());
    assert!(source_geometry("source.xcf", &excessive_payload, &Interrupt::NONE).is_err());
}

#[test]
fn wrapped_geometry_does_not_inflate_large_drawable_tails() {
    let mut bytes = header(&[dpi(360., 180.)]);
    bytes.resize(bytes.len() + 4 * 1024 * 1024, 0x7d);
    for (name, wrapped) in wrappers(&bytes) {
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 64;
        let ctl = Interrupt::cancel_only(&cancel);
        assert_eq!(source_geometry(name, &wrapped, &ctl).unwrap(), Some(((317., 193.), 360.)), "{name}");
        assert!(polls.load(Ordering::Relaxed) < 64, "geometry stops after properties, not after all drawable output");
        polls.store(0, Ordering::Relaxed);
        assert!(
            matches!(transport::decode(name, &wrapped, &ctl), Err(IoError::Cancelled)),
            "{name}: the independent full decode reaches the later cancellation threshold"
        );
    }
}

#[test]
fn geometry_stream_matches_shared_irregular_property_framing() {
    fn string(value: &[u8], data: &mut Vec<u8>) {
        data.extend((value.len() as u32).to_be_bytes());
        data.extend(value);
    }
    let mut colormap = 2u32.to_be_bytes().to_vec();
    colormap.extend([17, 31, 57, 101, 127, 193]);
    let mut link = 0u32.to_be_bytes().to_vec();
    string(b"source.xcf\0", &mut link);
    link.extend(317u32.to_be_bytes());
    link.extend(193u32.to_be_bytes());
    let mut cases: Vec<(u32, u32, u32, Vec<u8>)> = vec![(7, 1, 6, colormap), (26, 48, (link.len() - 4) as u32, link)];
    for version in [20, 21] {
        let mut unit = [1f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for _ in 0..if version < 21 { 5 } else { 3 } {
            string(b"unit\0", &mut unit);
        }
        cases.push((version, 24, 4, unit));
    }
    for (version, kind, declared, payload) in cases {
        let mut bytes = header(&[]);
        bytes[9..14].copy_from_slice(format!("v{version:03}\0").as_bytes());
        bytes.truncate(bytes.len() - 8);
        bytes.extend(kind.to_be_bytes());
        bytes.extend(declared.to_be_bytes());
        bytes.extend(payload);
        bytes.extend(19u32.to_be_bytes());
        bytes.extend(12u32.to_be_bytes());
        bytes.extend([288f32.to_be_bytes(), 144f32.to_be_bytes(), 123u32.to_be_bytes()].concat());
        bytes.extend(0u32.to_be_bytes());
        bytes.extend(999u32.to_be_bytes()); // END ignores the unused length, as wire does.
        let (mut cursor, header) = read_header(&bytes).unwrap();
        let props = wire::props_version_with_interrupt(&mut cursor, Some(header.version), &Interrupt::NONE).unwrap();
        let expected = metadata::image_resolution(&props, 72.);
        assert_eq!(expected, 288.);
        for (name, wrapped) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
            assert_eq!(
                source_geometry(name, &wrapped, &Interrupt::NONE).unwrap(),
                Some(((317., 193.), f64::from(expected))),
                "kind {kind}, version {version}, {name}"
            );
        }
    }
}

#[test]
fn geometry_stream_checks_legacy_headers_and_property_string_limits() {
    let mut legacy = header(&[dpi(144., 144.)]);
    legacy[9..14].copy_from_slice(b"file\0");
    legacy.drain(26..30); // Legacy headers have no precision word.
    for (name, bytes) in std::iter::once(("source.xcf", legacy.clone())).chain(wrappers(&legacy)) {
        assert_eq!(source_geometry(name, &bytes, &Interrupt::NONE).unwrap(), Some(((317., 193.), 144.)));
    }
    for kind in [24u32, 48] {
        let mut bytes = header(&[]);
        bytes.truncate(bytes.len() - 8);
        bytes.extend(kind.to_be_bytes());
        bytes.extend(0u32.to_be_bytes());
        bytes.extend(vec![0; if kind == 24 { 8 } else { 4 }]);
        bytes.extend(((super::super::MAX_STRING + 1) as u32).to_be_bytes());
        for (name, bytes) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
            let error = source_geometry(name, &bytes, &Interrupt::NONE).unwrap_err();
            assert!(error.to_string().contains("string"), "kind {kind}, {name}: {error}");
        }
    }
}

#[test]
fn geometry_cancels_while_discarding_large_optional_properties() {
    let bytes = header(&[(999, vec![0; 2 * 1024 * 1024]), dpi(300., 300.)]);
    for (name, bytes) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 12;
        assert!(matches!(source_geometry(name, &bytes, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)), "{name}");
    }
}

#[test]
fn non_xcf_geometry_keeps_the_existing_native_fallback() {
    let mut doc =
        photocraft_doc::Document::new("native", photocraft_geom::Size::new(23, 41), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.resolution_dpi = 180.;
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(source_geometry("source.pcraft", &native, &Interrupt::NONE).unwrap(), None);
    for order in [None, Some(Order::LittleEndian)] {
        assert_eq!(crate::smart_xcf_order::geometry("source.pcraft", &native, order, &Interrupt::NONE).unwrap(), ((23., 41.), 180.));
    }
}

#[test]
#[ignore = "24 MP F32 geometry benchmark; run explicitly in release mode"]
fn geometry_24mp_f32_header_vs_full_decode_benchmark() {
    use super::super::{
        tests::{TestImage, TestLayer},
        wire::Compression,
    };
    use std::time::Instant;
    let pixel = [-0.25f32, 1.5, 0.375, 1.];
    let encoded_pixel: Vec<_> = pixel.iter().flat_map(|sample| sample.to_be_bytes()).collect();
    let mut image = TestImage::rgb8(6000, 4000);
    image.version = 11;
    image.precision = 650;
    image.compression = Compression::None;
    image.image_props.push(dpi(300., 300.));
    image.layers.push(TestLayer::rgba("HDR signed control", 6000, 4000, encoded_pixel.repeat(24_000_000)));
    let bytes = image.write();
    drop(image);
    let start = Instant::now();
    for _ in 0..3 {
        let document = crate::smart_xcf_order::decode("benchmark.xcf", &bytes, Some(Order::BigEndian), &Interrupt::NONE).unwrap();
        assert_eq!(document.size, photocraft_geom::Size::new(6000, 4000));
        assert_eq!(document.resolution_dpi, 300.);
        assert_eq!(document.layers[0].surface().unwrap().pixel(5999, 3999), pixel);
        std::hint::black_box(&document);
    }
    let full = start.elapsed();
    let start = Instant::now();
    for _ in 0..1000 {
        let geometry = crate::smart_xcf_order::geometry("benchmark.xcf", std::hint::black_box(&bytes), Some(Order::BigEndian), &Interrupt::NONE).unwrap();
        assert_eq!(geometry, ((6000., 4000.), 300.));
        std::hint::black_box(geometry);
    }
    let header = start.elapsed();
    eprintln!(
        "24MP RGBA F32 XCF11: full_decode_3={full:?}, header_geometry_1000={header:?}; per_call_full_ns={}, per_call_header_ns={}",
        full.as_nanos() / 3,
        header.as_nanos() / 1000
    );
}
