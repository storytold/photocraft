use super::super::source_geometry;
use super::*;
use crate::HistoricalXcfSampleOrder as Order;
use photocraft_testkit::xcf::xz;
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
    // Test both known repairs and declared framing for malformed optional records.
    let mut cases: Vec<(u32, u32, u32, u32, Vec<u8>)> = vec![(7, 2, 1, 6, colormap.clone()), (26, 0, 48, (link.len() - 4) as u32, link)];
    cases.push((26, 0, 48, 8, [0u32.to_be_bytes(), u32::MAX.to_be_bytes()].concat()));
    for base_type in [0, 1] {
        cases.push((7, base_type, 1, colormap.len() as u32, colormap.clone()));
        cases.push((7, base_type, 1, 6, colormap.clone()));
        // Unused malformed contents still use their correctly declared framing.
        cases.push((7, base_type, 1, 4, u32::MAX.to_be_bytes().to_vec()));
    }
    for version in [20, 21] {
        let mut unit = [1f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for _ in 0..if version < 21 { 5 } else { 3 } {
            string(b"unit\0", &mut unit);
        }
        cases.push((version, 0, 24, 4, unit));
    }
    for (version, base_type, kind, declared, payload) in cases {
        let mut bytes = header(&[]);
        bytes[9..14].copy_from_slice(format!("v{version:03}\0").as_bytes());
        bytes[22..26].copy_from_slice(&base_type.to_be_bytes());
        if base_type == 2 {
            bytes[26..30].copy_from_slice(&150u32.to_be_bytes()); // Indexed samples require nonlinear U8.
        }
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
        let props = wire::props_image_with_interrupt(&mut cursor, header.version, header.base_type == 2, &Interrupt::NONE).unwrap();
        let expected = metadata::image_resolution(&props, 72.);
        assert_eq!(expected, 288.);
        for (name, wrapped) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
            assert_eq!(
                source_geometry(name, &wrapped, &Interrupt::NONE).unwrap(),
                Some(((317., 193.), f64::from(expected))),
                "base {base_type}, kind {kind}, version {version}, {name}"
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
    let mut bytes = header(&[]);
    bytes.truncate(bytes.len() - 8);
    bytes.extend(24u32.to_be_bytes());
    bytes.extend(0u32.to_be_bytes());
    bytes.extend([0; 8]);
    bytes.extend(((super::super::MAX_STRING + 1) as u32).to_be_bytes());
    for (name, bytes) in std::iter::once(("source.xcf", bytes.clone())).chain(wrappers(&bytes)) {
        let error = source_geometry(name, &bytes, &Interrupt::NONE).unwrap_err();
        assert!(error.to_string().contains("string"), "{name}: {error}");
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

#[test]
fn psd_export_uses_wrapped_modern_xcf_geometry_without_a_sample_order() {
    use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Rect, Size};
    use photocraft_raster::Surface;
    let mut modern = header(&[dpi(240., 240.)]);
    modern[9..14].copy_from_slice(b"v026\0");
    for (name, bytes) in wrappers(&modern) {
        assert!(crate::import(name, &bytes).is_err(), "the test source cannot be fully decoded");
        let mut doc = Document::new("Geometry only", Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
        let mut cache = Surface::new(doc.pixel_format());
        cache.write_region(Rect::new(0, 0, 1, 1), &[1., 0., 0., 1.]);
        let source = SmartSource::Embedded { file_name: name.into(), bytes: std::sync::Arc::new(bytes) };
        doc.layers.push(Layer::new("Wrapped XCF", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, Some(cache)))));
        let exported = crate::export(&doc, "probe.psd", &Default::default()).unwrap();
        let psd = craft_psd::PsdFile::from_bytes(&exported.bytes).unwrap();
        let record = &psd.layer_info.unwrap().layers[0];
        let placed = crate::smart_map::parse_sold(&record.block(b"SoLd").unwrap().data).unwrap();
        assert_eq!(crate::smart_map::stored_size(&placed.descriptor), Some((317., 193.)), "{name}");
    }
}
