use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::Size;
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

#[test]
fn saved_svgz_refresh_keeps_native_size_pixels_and_place_reports_omissions() {
    // The fractional edge distinguishes native-size import and resampling from
    // rasterizing the vector again at the enlarged placement resolution.
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect x=".35" width="5.4" height="8" fill="red"/><image width="1" height="1" href="data:image/png;base64,A"/></svg>"#;
    let encoded = gzip(svg);
    let name = "saved-refresh.svgz";
    let mut session = crate::session();
    session.execute("file.new", serde_json::json!({"width":32,"height":32})).unwrap();
    let placed = crate::file_cmds::place_bytes(&mut session, name, encoded.clone(), None, &serde_json::json!({"smartObject":true,"scale":175})).unwrap();
    let warnings = placed["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|warning| warning.as_str().is_some_and(|warning| warning.contains("embedded image was omitted"))));
    let state = session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
    let mut doc = photocraft_format::load_from_bytes(&saved).unwrap();
    let format = doc.pixel_format();
    let mut changed = doc.layer(layer).unwrap().clone();
    let LayerContent::Smart(sm) = &mut changed.content else { panic!("saved SVGZ smart object") };
    sm.transform.m[4] += 1.;
    sm.cache = None;

    // Independent oracle for the previous route: ordinary file import at source
    // size, then the same image placement used for other raster sources.
    let source = crate::file_cmds::import(name, &encoded).unwrap();
    let bounds = source.bounds();
    let source = super::super::source::flattened_source(&source, format).unwrap();
    let expected = photocraft_algo::warp::place_source(&source, bounds, &sm.transform, None).unwrap();
    assert_eq!(source_box(&doc, sm), Some([0., 0., 8., 8.]));
    assert!(refresh_layer(&doc, &mut changed).unwrap());
    let LayerContent::Smart(sm) = &mut changed.content else { panic!("refreshed SVGZ smart object") };
    assert_eq!(sm.cache.as_ref(), Some(&expected));

    sm.smart_filters.push(SmartFilter {
        command: "image.adjustments.invert".into(),
        params: serde_json::json!({}),
        blend: BlendMode::Normal,
        opacity: 1.,
        visible: true,
    });
    let filtered = apply_smart_filters(&expected, sm, doc.bounds()).unwrap();
    *doc.layer_mut(layer).unwrap() = changed;
    assert!(refresh(&mut doc, layer).unwrap());
    let LayerContent::Smart(sm) = &doc.layer(layer).unwrap().content else { panic!("filtered SVGZ smart object") };
    assert_eq!(sm.cache.as_ref(), Some(&filtered));

    let plain = object("saved-refresh.svg", Arc::new(svg.to_vec()));
    assert!(render(&doc, &plain).is_err(), "plain SVG refresh still requires complete pixels");
    let (pixels, warnings) = render_with_warnings(&doc, &plain).unwrap();
    assert!(pixels.is_some());
    assert_eq!(warnings.len(), 1);
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
fn raster_named_svg_keeps_the_ordinary_import_path() {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let doc = document();
    let sm = object("renamed.svg", Arc::new(png));
    let rendered = render(&doc, &sm).unwrap().unwrap();
    assert_eq!(rendered.rgba(0, 0), [0., 1., 0., 1.]);
    let (reported, warnings) = render_with_warnings_using(&sm, &mut WarningRenderSources::new(&doc)).unwrap();
    assert_eq!(reported.unwrap(), rendered);
    assert!(warnings.is_empty());
}
