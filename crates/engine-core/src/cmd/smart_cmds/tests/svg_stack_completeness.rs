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
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&incomplete_svg()).unwrap();
    let bytes = Arc::new(encoder.finish().unwrap());
    let session = session(8);
    let doc = &session.active().unwrap().doc;
    for name in ["logo.gz", "without-extension"] {
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: name.into(), bytes: bytes.clone() }, Affine::IDENTITY, None);
        assert!(decode_source(name, &bytes).is_ok());
        smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
        let rendered = render(doc, &smart).unwrap().unwrap();
        assert_eq!(rendered.rgba(1, 1), [1., 0., 0., 1.]);
        assert_eq!(source_box(doc, &smart), Some([0., 0., 2., 2.]));
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
