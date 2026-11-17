use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::StackMode;
use photocraft_geom::{
    Size,
    warp::{BezierMesh, Warp},
};
use std::io::Write;

fn drawing(label: &str, omitted: bool) -> Vec<u8> {
    let image = if omitted { r#"<image width="1" height="1" href="data:image/png;base64,A"/>"# } else { "" };
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><!--{label}--><rect width="8" height="8" fill="red"/>{image}</svg>"#).into_bytes()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn document() -> Document {
    Document::new("Source work", Size::new(8, 8), ColorMode::Rgb, SampleType::U8)
}
fn object(name: &str, bytes: Arc<Vec<u8>>) -> SmartObject {
    SmartObject::new(SmartSource::Embedded { file_name: name.into(), bytes }, Affine::IDENTITY, None)
}
fn probes() -> usize {
    SVG_CLASSIFICATIONS.with(std::cell::Cell::get)
}
fn reset_probes() {
    SVG_CLASSIFICATIONS.with(|value| value.set(0));
}

#[test]
fn compressed_stack_hits_skip_format_probe_and_preserve_misnamed_sources() {
    for name in ["cached.svgz", "cached.gz", "no-extension", "wrong.dat"] {
        let encoded = gzip(&drawing(name, false));
        reset_probes();
        let first = stack_image(name, &encoded, PixelFormat::RGBA32F, StackMode::Mean).unwrap();
        assert_eq!(probes(), 1, "the cold Stack operation classifies once");
        let second = stack_image(name, &encoded, PixelFormat::RGBA32F, StackMode::Mean).unwrap();
        assert!(Arc::ptr_eq(&first.surface, &second.surface));
        assert_eq!(probes(), 1, "a strict Stack cache hit never inflates merely to choose its cache key");
    }
}

#[test]
fn compressed_render_and_bounds_classify_once_per_operation() {
    for kind in ["direct", "warp", "stack"] {
        let doc = document();
        let bytes = Arc::new(gzip(&drawing(kind, false)));
        let mut sm = object("misnamed.gz", bytes);
        if kind == "warp" {
            let bounds = [0., 0., 8., 8.];
            sm.warp = Some(Warp::custom(BezierMesh::identity(bounds, 1, 1), bounds));
        } else if kind == "stack" {
            sm.stack_mode = Some(StackMode::Mean);
        }
        reset_probes();
        let rendered = render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap();
        assert_eq!(rendered.rgba(3, 3), [1., 0., 0., 1.]);
        assert_eq!(probes(), 1, "{kind}: classification is shared with placement and decoding");
        reset_probes();
        assert_eq!(render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap(), rendered);
        assert_eq!(probes(), 0, "{kind}: the typed warm stage avoids compressed recognition entirely");
        reset_probes();
        assert_eq!(source_box(&doc, &sm), Some([0., 0., 8., 8.]));
        assert_eq!(probes(), usize::from(kind == "stack"), "bounds reuse typed placement kind; a new Stack pixel format classifies once");
        reset_probes();
        assert_eq!(source_box(&doc, &sm), Some([0., 0., 8., 8.]));
        assert_eq!(probes(), 0, "warm bounds reuse strict cache domains without recognizing compressed input again");
    }
}

#[test]
fn warning_memo_hits_skip_compressed_classification_and_keep_warnings_private() {
    let doc = document();
    let bytes = Arc::new(gzip(&drawing("warning memo probe", true)));
    let sm = object("warning-without-extension", bytes);
    let mut memo = WarningRenderSources::new(&doc);
    reset_probes();
    WARNING_SOURCE_DECODES.with(|value| value.set(0));
    let (first, warnings) = render_with_warnings_using(&sm, &mut memo).unwrap();
    assert_eq!(warnings.len(), 1);
    let (second, repeated) = render_with_warnings_using(&sm, &mut memo).unwrap();
    assert_eq!(first, second);
    assert_eq!(warnings, repeated);
    assert_eq!(probes(), 1);
    assert_eq!(WARNING_SOURCE_DECODES.with(std::cell::Cell::get), 1);
    assert!(render(&doc, &sm).is_err(), "warning-aware source pixels never populate strict caches");
}

#[test]
fn memo_prices_owned_output_and_does_not_retain_encoded_capacity() {
    let doc = document();
    let data = drawing("reserved encoded capacity", false);
    let mut reserved = Vec::with_capacity(1 << 20);
    reserved.extend_from_slice(&data);
    let bytes = Arc::new(reserved);
    let weak = Arc::downgrade(&bytes);
    let sm = object("reserved.svg", bytes);
    let tile = photocraft_raster::tile_storage_bytes(doc.bounds(), &doc.pixel_format()) as usize;
    let mut memo = WarningRenderSources::with_limits(&doc, tile + 4096, 32);
    WARNING_SOURCE_DECODES.with(|value| value.set(0));
    let first = render_with_warnings_using(&sm, &mut memo).unwrap();
    assert!(first.1.is_empty());
    assert_eq!(memo.entries.len(), 1, "encoded spare capacity is borrowed, not retained memo storage");
    assert!(memo.bytes <= tile + 4096);
    assert_eq!(render_with_warnings_using(&sm, &mut memo).unwrap(), first);
    assert_eq!(WARNING_SOURCE_DECODES.with(std::cell::Cell::get), 1);
    assert_eq!(weak.strong_count(), 1, "only the caller owns its source allocation");
    drop(sm);
    assert!(weak.upgrade().is_none(), "the memo must not pin an encoded body after its owner drops it");
}

#[test]
fn empty_memo_with_stale_accounting_recovers_without_panicking() {
    let doc = document();
    let data = Arc::new(drawing("empty accounting", false));
    let mut memo = WarningRenderSources::new(&doc);
    memo.bytes = memo.budget;
    let (image, warnings) = memo.image(&data, 1., *blake3::hash(b"empty.svg").as_bytes()).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(image.surface.rgba(3, 3), [1., 0., 0., 1.]);
    assert_eq!(memo.entries.len(), 1);
    assert!(memo.bytes <= memo.budget);
}

#[test]
fn central_recognition_keeps_real_raster_bytes_even_with_svg_name() {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    assert!(!svg_input("renamed.svg", &png));
    let doc = document();
    let sm = object("renamed.svg", Arc::new(png));
    reset_probes();
    let rendered = render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap();
    assert_eq!(rendered.rgba(0, 0), [0., 1., 0., 1.]);
    assert_eq!(probes(), 1);
    reset_probes();
    let (reported, warnings) = render_with_warnings_using(&sm, &mut WarningRenderSources::new(&doc)).unwrap();
    assert_eq!(reported.unwrap(), rendered);
    assert!(warnings.is_empty());
    assert_eq!(probes(), 1, "warning-aware non-SVG fallback reuses the same authorized snapshot and classification");
}

#[test]
fn warm_typed_stages_keep_profile_name_and_authority_boundaries() {
    let mut doc = document();
    let data = String::from_utf8(drawing("typed domains", false)).unwrap().replace("fill=\"red\"", "fill=\"#808080\"");
    let bytes = Arc::new(gzip(data.as_bytes()));
    let mut sm = object("typed-domains.svgz", bytes.clone());
    sm.smart_filters.push(SmartFilter {
        command: "image.adjustments.invert".into(),
        params: serde_json::json!({}),
        blend: BlendMode::Normal,
        opacity: 1.,
        visible: true,
    });
    let first = render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap();
    reset_probes();
    assert_eq!(render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap(), first);
    assert_eq!(probes(), 0, "filtered warm stages do not sniff compressed text");
    fn deny_invert(command: &str, _: &Value) -> Result<()> {
        if command == "image.adjustments.invert" { Err(other("test denies cached invert")) } else { Ok(()) }
    }
    {
        let _scope = crate::jobs::render_authorization_scope(Some(deny_invert));
        assert!(render_in(&doc, &sm, &JobCtx::new()).unwrap_err().to_string().contains("test denies cached invert"));
        assert_eq!(probes(), 0, "authority is checked before either cache domain");
    }
    doc.icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
    reset_probes();
    let linear = render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap();
    assert_ne!(linear.pixel(3, 3), first.pixel(3, 3), "a profile change cannot reuse sRGB stage pixels");
    assert_eq!(probes(), 1);
    reset_probes();
    assert_eq!(render_in(&doc, &sm, &JobCtx::new()).unwrap().unwrap(), linear);
    assert_eq!(probes(), 0);
    let mut renamed = sm.clone();
    renamed.source = SmartSource::Embedded { file_name: "typed-domains-without-extension".into(), bytes };
    reset_probes();
    assert_eq!(render_in(&doc, &renamed, &JobCtx::new()).unwrap().unwrap(), linear);
    assert_eq!(probes(), 1, "filename-dependent recognition uses a distinct typed domain");
}

#[test]
fn warning_memo_retains_filename_recognition_for_the_same_encoded_arc() {
    let doc = document();
    let bytes = Arc::new(gzip(&drawing("same allocation different format names", false)));
    let svg = object("shared.svgz", bytes.clone());
    let renamed = object("shared.af", bytes.clone());
    assert!(!svg_input("shared.af", &bytes));
    let cold = render_with_warnings_using(&renamed, &mut WarningRenderSources::new(&doc)).unwrap_err();
    let mut memo = WarningRenderSources::new(&doc);
    let (rendered, warnings) = render_with_warnings_using(&svg, &mut memo).unwrap();
    assert_eq!(rendered.unwrap().rgba(3, 3), [1., 0., 0., 1.]);
    assert!(warnings.is_empty());
    assert_eq!(memo.entries.len(), 1);
    reset_probes();
    let warm = render_with_warnings_using(&renamed, &mut memo).unwrap_err();
    assert_eq!(warm.to_string(), cold.to_string(), "a warm SVG instance cannot override central filename precedence");
    assert_eq!(probes(), 1, "a different recognition name must be checked even for the exact same source Arc");
    assert_eq!(memo.entries.len(), 1, "a failed alternate route does not enter the SVG memo");
}
