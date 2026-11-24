use super::super::{source, stack_cache};
use super::*;

fn incomplete_svg() -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgba, vec![0, 255, 0, 255]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let idat = png.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
    // Keep the complete IHDR and dimensions, but damage actual compressed pixels.
    *png.get_mut(idat + 4).unwrap() ^= 0xff;
    assert_eq!(photocraft_codecs::probe_allocation_size(&png).unwrap(), (1, 1));
    let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><!--stack-completeness-review--><rect width=\"2\" height=\"2\" fill=\"red\"/><image width=\"2\" height=\"2\" href=\"data:image/png,{data}\"/></svg>").into_bytes()
}

fn gzip(svg: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(svg).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn svgz_fallback_rejects_preview_only_affinity_without_changing_smart_contents() {
    let image = photocraft_codecs::Image::from_u8(2, 1, photocraft_codecs::ChannelLayout::Rgba, vec![17, 31, 57, 255, 0, 0, 0, 0]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    // Existing shared synthetic container API: a valid thumbnail without a
    // native object graph, so ordinary Open offers only its warned preview.
    let source = craft_affinity::synth::container(&[], Some(&png));
    for name in ["preview.afphoto", "renamed.png", "without-extension"] {
        let ordinary = photocraft_io::import(name, &source).unwrap();
        assert!(ordinary.preview_only && ordinary.source_read_only);
        assert_eq!(ordinary.document.size, Size::new(2, 1));
        assert!(crate::file_cmds::import(name, &source).unwrap_err().to_string().contains("embedded preview"));
        assert!(decode_source(name, &source).unwrap_err().to_string().contains("embedded preview"));
        let mut session = session(8);
        paint(&mut session);
        let id = LayerId(convert(&mut session));
        let before = session.active().unwrap().clone();
        let (old_name, old_bytes) = contents_of(&before.doc, id).unwrap();
        let error = replace_contents_bytes(&mut session, id, name, source.clone()).unwrap_err();
        assert!(error.to_string().contains("embedded preview"));
        let after = session.active().unwrap();
        assert!(Arc::ptr_eq(&before.doc, &after.doc));
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.saved_revision, before.saved_revision);
        assert_eq!(after.history.past_len(), before.history.past_len());
        let (name, bytes) = contents_of(&after.doc, id).unwrap();
        assert_eq!(name, old_name);
        assert!(Arc::ptr_eq(&bytes, &old_bytes));
    }
    // A complete non-SVG source still reaches its normal engine decoder.
    assert_eq!(decode_source("complete.png", &png).unwrap().size, Size::new(2, 1));
    assert_eq!(decode_source("misnamed.svg", &png).unwrap().size, Size::new(2, 1));
}

#[test]
fn non_svg_smart_contents_preserve_typed_psd_budget_refusals() {
    let source = session(8);
    let psd = photocraft_io::export(&source.active().unwrap().doc, "source.psd", &Default::default()).unwrap().bytes;
    let snapshot = photocraft_io::memory::MemorySnapshot { available_ram: Some(4 << 30), swap_free: None, commit_headroom: None };
    let hard = photocraft_io::memory::resolve_budget(snapshot, None).hard_limit;
    photocraft_io::memory::with_mock_memory(snapshot, || {
        photocraft_io::psd_budget::with_test_estimate_charge(hard + 1, || {
            // This is the real IO admission error, not a mock import result.
            assert!(matches!(photocraft_io::import("source.psd", &psd), Err(photocraft_io::IoError::PsdImportBudget { .. })));
            match decode_source("source.psd", &psd).unwrap_err() {
                EngineError::PsdHardBudget { name, required, limit, .. } => {
                    assert_eq!(name, "source.psd");
                    assert!(required > hard);
                    assert_eq!(limit, hard);
                }
                error => panic!("the engine's typed source refusal was lost: {error}"),
            }
        });
    });
}

fn smart_svg(bytes: Vec<u8>) -> SmartObject {
    SmartObject::new(SmartSource::Embedded { file_name: "Stack completeness.svg".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None)
}

fn old_stack_key(bytes: &[u8], fmt: PixelFormat, mode: photocraft_doc::StackMode) -> source::CacheKey {
    let mut key = source::cache_key(bytes, fmt);
    key.0 = *blake3::hash(&[&key.0[..], b"stack:", mode.id().as_bytes()].concat()).as_bytes();
    key
}

fn old_placed_key(bytes: &[u8], doc: &Document, smart: &SmartObject) -> StageKey {
    let key = stack_cache::placed_key(bytes, doc.pixel_format(), smart, doc.bounds());
    let mut hash = blake3::Hasher::new();
    hash.update(&key);
    hash.update(&crate::color_cmds::document_profile(doc).to_bytes());
    *hash.finalize().as_bytes()
}

#[test]
fn gzipped_svg_stack_retains_ordinary_import_for_misnamed_sources() {
    let bytes = Arc::new(gzip(&incomplete_svg()));
    let session = session(8);
    let doc = &session.active().unwrap().doc;
    for name in ["logo.gz", "without-extension"] {
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: name.into(), bytes: bytes.clone() }, Affine::IDENTITY, None);
        let ordinary = photocraft_io::import(name, &bytes).unwrap();
        assert!(ordinary.warnings.iter().any(|warning| warning.contains("embedded image was omitted")));
        assert!(decode_source(name, &bytes).is_err());
        smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
        for _ in 0..2 {
            let rendered = render(doc, &smart).unwrap().unwrap();
            assert_eq!(rendered.rgba(1, 1), [1., 0., 0., 1.]);
            assert_eq!(source_box(doc, &smart), Some([0., 0., 2., 2.]));
        }
    }
}

#[test]
fn reset_transform_keeps_existing_svgz_with_an_omitted_image() {
    let bytes = Arc::new(gzip(&incomplete_svg()));
    for name in ["existing.svgz", "existing.gz", "without-extension"] {
        let mut s = session(8);
        paint(&mut s);
        let id = LayerId(convert(&mut s));
        let transform = Affine { m: [2., 0., 0., 2., 0., 0.] };
        let smart = SmartObject::new(SmartSource::Embedded { file_name: name.into(), bytes: bytes.clone() }, transform, None);
        s.edit("restore embedded Smart Object", |doc, _| {
            doc.layer_mut(id).unwrap().content = LayerContent::Smart(smart);
            Ok(())
        })
        .unwrap();

        assert!(decode_source(name, &bytes).is_err());
        let ordinary = photocraft_io::import(name, &bytes).unwrap();
        assert!(ordinary.warnings.iter().any(|warning| warning.contains("embedded image was omitted")));
        let existing = render(&s.active().unwrap().doc, &active_smart(&s)).unwrap().unwrap();
        assert_eq!(existing.rgba(1, 1), [1., 0., 0., 1.]);

        s.execute("layer.smartObjects.resetTransform", json!({"layer": id.0})).unwrap();
        let reset = active_smart(&s);
        assert_eq!(reset.transform.m, [1., 0., 0., 1., 1., 1.]);
        assert_eq!(reset.cache.unwrap().rgba(1, 1), [1., 0., 0., 1.]);
        let (retained_name, retained_bytes) = contents_of(&s.active().unwrap().doc, id).unwrap();
        assert_eq!(retained_name, name);
        assert!(Arc::ptr_eq(&bytes, &retained_bytes));
    }
}

#[test]
fn svg_contents_replacement_rejects_omissions_without_changing_target() {
    let svg = incomplete_svg();
    let wrapped = gzip(&svg);
    let mut padded_svg = vec![b' '; 4096];
    padded_svg.extend_from_slice(&svg);
    let wrapped_names =
        ["contents.svgz", "contents.gz", "contents.psd", "contents.afphoto", "contents.afdesign", "contents.afpub", "contents.AfPhOtO", "without-extension"];
    let cases = wrapped_names.into_iter().map(|name| (name, wrapped.as_slice())).chain(std::iter::once(("contents.svg", padded_svg.as_slice())));
    for (name, bytes) in cases {
        let opened = photocraft_io::import(name, bytes).unwrap();
        assert!(opened.warnings.iter().any(|warning| warning.contains("embedded image was omitted")), "{name}");
        assert_eq!(photocraft_compose::flatten(&opened.document).get(1, 1), [1., 0., 0., 1.]);
        let mut s = session(8);
        paint(&mut s);
        let id = LayerId(convert(&mut s));
        s.active_mut().unwrap().source_read_only = true;
        let before = s.active().unwrap().clone();
        let pixels = flat(&s);
        let (old_name, old_bytes) = contents_of(&before.doc, id).unwrap();
        let error = replace_contents_bytes(&mut s, id, name, bytes.to_vec()).unwrap_err().to_string();
        assert!(error.contains("incomplete smart object contents"), "{name}: {error}");
        let after = s.active().unwrap();
        assert!(Arc::ptr_eq(&before.doc, &after.doc), "{name}");
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.saved_revision, before.saved_revision);
        assert_eq!(after.history.past_len(), before.history.past_len());
        assert_eq!(after.path, before.path);
        assert_eq!(after.source_read_only, before.source_read_only);
        assert_eq!(flat(&s), pixels);
        let (retained_name, retained_bytes) = contents_of(&after.doc, id).unwrap();
        assert_eq!(retained_name, old_name);
        assert!(Arc::ptr_eq(&retained_bytes, &old_bytes));
    }
}

#[test]
fn complete_svgz_replacement_keeps_encoded_bytes_and_accepts_supported_utf16() {
    let xml = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>"#;
    let bytes = gzip(xml.as_bytes());
    for source_name in
        ["complete.svgz", "complete.gz", "complete.psd", "complete.afphoto", "complete.afdesign", "complete.afpub", "complete.AfPhOtO", "without-extension"]
    {
        let mut s = session(8);
        paint(&mut s);
        let id = LayerId(convert(&mut s));
        let before = s.active().unwrap().clone();
        replace_contents_bytes(&mut s, id, source_name, bytes.clone()).unwrap();
        let after = s.active().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.history.past_len(), before.history.past_len() + 1);
        let (name, retained) = contents_of(&after.doc, id).unwrap();
        assert_eq!(name, source_name);
        assert_eq!(retained.as_slice(), bytes);
        let decoded = decode_source(&name, &retained).unwrap();
        assert_eq!(decoded.size, Size::new(2, 2));
        assert_eq!(photocraft_compose::flatten(&decoded).get(1, 1), [1., 0., 0., 1.]);
    }
    for name in ["explicit.af", "explicit.AF", ".af"] {
        assert!(decode_source(name, &bytes).is_err(), "{name}: the explicit Affinity route must retain priority");
    }
    // BOM-marked UTF-16 is now an intentional ordinary and strict SVG input.
    // Replacement must keep its encoded source, not silently rewrite it to UTF-8.
    for little in [true, false] {
        let mut encoded = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
        for unit in xml.encode_utf16() {
            encoded.extend_from_slice(&if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        let malformed = gzip(&[encoded.as_slice(), &[0]].concat()); // Odd final UTF-16 byte.
        let encoded = gzip(&encoded);
        assert_eq!(photocraft_io::import("supported.svgz", &encoded).unwrap().document.size, Size::new(2, 2));
        assert_eq!(decode_source("supported.svgz", &encoded).unwrap().size, Size::new(2, 2));
        let mut s = session(8);
        paint(&mut s);
        let id = LayerId(convert(&mut s));
        replace_contents_bytes(&mut s, id, "supported.svgz", encoded.clone()).unwrap();
        assert_eq!(contents_of(&s.active().unwrap().doc, id).unwrap().1.as_slice(), encoded);
        assert!(photocraft_io::import("malformed.svgz", &malformed).is_err());
        assert!(decode_source("malformed.svgz", &malformed).is_err());
    }
}

#[test]
fn svg_stack_rejects_incomplete_pixels_repeatedly_but_ordinary_placement_keeps_warnings() {
    let session = session(8);
    let doc = &session.active().unwrap().doc;
    let bytes = incomplete_svg();
    let mut smart = smart_svg(bytes.clone());
    let (ordinary, warnings) = render_with_warnings(doc, &smart).unwrap();
    assert_eq!(ordinary.unwrap().rgba(1, 1), [1., 0., 0., 1.]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("embedded image was omitted"));
    smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
    for _ in 0..2 {
        assert!(stack_image("Stack completeness.svg", &bytes, PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).is_err());
        assert!(render(doc, &smart).is_err());
        assert!(render_with_warnings(doc, &smart).is_err());
        assert_eq!(source_box(doc, &smart), None);
        assert!(smart.cache.is_none());
    }
}

#[test]
fn svg_stack_does_not_reuse_incomplete_legacy_source_or_filter_stages() {
    let _cache = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = session(8);
    let doc = &session.active().unwrap().doc;
    let bytes = incomplete_svg();
    let mut smart = smart_svg(bytes.clone());
    smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
    let mut poisoned = Surface::new(PixelFormat::RGBA32F);
    poisoned.fill_rect(Rect::new(0, 0, 2, 2), &[0., 1., 0., 1.]);
    let key = old_stack_key(&bytes, PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean);
    source::cache_put(key, source::SourceImage { surface: Arc::new(poisoned), bounds: Rect::new(0, 0, 2, 2) });
    assert!(stack_image("Stack completeness.svg", &bytes, PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).is_err());

    let mut poisoned = Surface::new(doc.pixel_format());
    poisoned.fill_rect(Rect::new(0, 0, 2, 2), &[0., 1., 0., 1.]);
    let stage = Stage { surface: poisoned, extent: doc.bounds() };
    let placed = old_placed_key(&bytes, doc, &smart);
    stack_cache::put(placed, &stage);
    assert!(render(doc, &smart).is_err(), "old placed pixels must not bypass strict decoding");
    smart.smart_filters.push(SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": 1}),
        blend: BlendMode::Normal,
        opacity: 1.,
        visible: true,
    });
    let filtered = stack_cache::filter_key(&placed, &smart.smart_filters[0], None);
    stack_cache::put(filtered, &stage);
    assert!(render(doc, &smart).is_err(), "old deepest filter stages must not bypass strict decoding");
    assert!(render_with_warnings(doc, &smart).is_err());
}

#[test]
fn complete_svg_stack_keeps_mean_pixels_across_source_stage_and_warning_render_caches() {
    let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><!--complete-stack-review--><rect width="2" height="2" fill="red"/><rect width="2" height="2" fill="blue"/></svg>"#;
    let session = session(32);
    let mut doc = (*session.active().unwrap().doc).clone();
    doc.icc_profile = Some(craft_cms::Builtin::Srgb.profile().to_bytes());
    let mut smart = smart_svg(bytes.to_vec());
    smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
    // The red/blue Mean is 0.5 in the SVG's U8 encoding. Existing source
    // conversion rounds it to 128/255 before widening and decoding the sRGB
    // transfer for F32. The explicit sRGB destination keeps the rendered result
    // within ICC roundoff of that value (the matrix path is not bit-exact).
    let expected = photocraft_color::convert::srgb_to_linear(128.0 / 255.0);
    let check_mean = |pixel: [f32; 4]| {
        assert!((pixel[0] - expected).abs() < 3e-5, "red Mean: {pixel:?}");
        assert!(pixel[1].abs() < 1e-6, "green Mean: {pixel:?}");
        assert!((pixel[2] - expected).abs() < 3e-5, "blue Mean: {pixel:?}");
        assert_eq!(pixel[3], 1.0);
    };
    for _ in 0..2 {
        let source = stack_image("Complete.svg", bytes, PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).unwrap();
        check_mean(source.surface.rgba(1, 1));
        let strict = render(&doc, &smart).unwrap().unwrap();
        check_mean(strict.rgba(1, 1));
        let (reported, warnings) = render_with_warnings(&doc, &smart).unwrap();
        assert_eq!(reported.unwrap(), strict);
        assert!(warnings.is_empty());
        assert_eq!(source_box(&doc, &smart), Some([0., 0., 2., 2.]));
    }
}

/// Original uncompressed RGB DICOM control. PS3.10 section 7.1 permits implementation-defined
/// preamble bytes; the Explicit VR Little Endian data set contains only synthetic image data.
fn dicom_with_gzip_like_preamble() -> Vec<u8> {
    fn element(bytes: &mut Vec<u8>, group: u16, tag: u16, vr: &[u8; 2], value: &[u8]) {
        bytes.extend_from_slice(&group.to_le_bytes());
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(vr);
        if vr == b"OB" {
            bytes.extend_from_slice(&[0, 0]);
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        } else {
            bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
        }
        bytes.extend_from_slice(value);
    }
    let mut bytes = vec![0; 128];
    bytes[..3].copy_from_slice(&[0x1f, 0x8b, 0]);
    bytes.extend_from_slice(b"DICM");
    let mut meta = Vec::new();
    element(&mut meta, 2, 1, b"OB", &[0, 1]);
    element(&mut meta, 2, 2, b"UI", b"1.2.840.10008.5.1.4.1.1.7\0");
    element(&mut meta, 2, 3, b"UI", b"2.25.1");
    element(&mut meta, 2, 0x10, b"UI", b"1.2.840.10008.1.2.1\0");
    element(&mut meta, 2, 0x12, b"UI", b"2.25.2");
    element(&mut bytes, 2, 0, b"UL", &(meta.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&meta);
    element(&mut bytes, 8, 0x16, b"UI", b"1.2.840.10008.5.1.4.1.1.7\0");
    element(&mut bytes, 8, 0x18, b"UI", b"2.25.1");
    element(&mut bytes, 0x28, 2, b"US", &3_u16.to_le_bytes());
    element(&mut bytes, 0x28, 4, b"CS", b"RGB ");
    for (tag, value) in [(6, 0_u16), (0x10, 1), (0x11, 2), (0x100, 8), (0x101, 8), (0x102, 7), (0x103, 0)] {
        element(&mut bytes, 0x28, tag, b"US", &value.to_le_bytes());
    }
    element(&mut bytes, 0x7fe0, 0x10, b"OB", &[255, 0, 0, 0, 255, 0]);
    bytes
}

#[test]
fn speculative_svgz_failure_preserves_non_svg_dicom_admission() {
    use std::io::Read as _;
    let bytes = dicom_with_gzip_like_preamble();
    assert!(bytes.starts_with(&[0x1f, 0x8b]));
    assert!(flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut Vec::new()).is_err());
    for name in ["synthetic.dcm", "renamed.bin", "without-extension"] {
        assert!(!photocraft_io::is_svg_input(name, &bytes));
        assert!(photocraft_io::try_import_smart_object_svg(name, &bytes, 24_000_000).unwrap().is_none());
        let ordinary = photocraft_io::import(name, &bytes).unwrap();
        let decoded = decode_source(name, &bytes).unwrap();
        for doc in [&ordinary.document, &decoded] {
            assert_eq!(doc.size, Size::new(2, 1));
            let pixels = photocraft_compose::flatten(doc);
            assert_eq!(pixels.get(0, 0), [1., 0., 0., 1.]);
            assert_eq!(pixels.get(1, 0), [0., 1., 0., 1.]);
        }
    }
    // Also preserve the fallback when the preamble is a valid gzip member whose decoded
    // payload is not text. The actual DICOM image and pixel metadata are unchanged.
    let mut nontext = bytes.clone();
    nontext[..128].fill(0);
    let preamble = gzip(&[0xff]);
    nontext[..preamble.len()].copy_from_slice(&preamble);
    assert!(photocraft_io::try_import_smart_object_svg("nontext.dcm", &nontext, 24_000_000).unwrap().is_none());
    let decoded = decode_source("nontext.dcm", &nontext).unwrap();
    assert_eq!(photocraft_compose::flatten(&decoded).get(0, 0), [1., 0., 0., 1.]);
    assert_eq!(photocraft_compose::flatten(&decoded).get(1, 0), [0., 1., 0., 1.]);
    let valid = gzip(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"/>"#);
    for damaged in [valid[..valid.len() - 8].to_vec(), vec![0x1f, 0x8b, 0]] {
        for name in ["broken.svgz", "broken.svg"] {
            assert!(photocraft_io::is_svg_input(name, &damaged));
            assert!(photocraft_io::try_import_smart_object_svg(name, &damaged, 24_000_000).is_err());
            assert!(decode_source(name, &damaged).is_err());
        }
    }
}
