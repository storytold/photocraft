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
    let decodes = source_cache_statistics().svg_decodes;
    let first = render_with_warnings_using(&sm, &mut memo).unwrap();
    assert!(first.1.is_empty());
    assert_eq!(memo.entries.len(), 1, "encoded spare capacity is borrowed, not retained memo storage");
    assert!(memo.bytes <= tile + 4096);
    assert_eq!(render_with_warnings_using(&sm, &mut memo).unwrap(), first);
    assert_eq!(source_cache_statistics().svg_decodes - decodes, 1);
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

/// The bounded source memo of [`WarningRenderSources`] (one SVG update's source-space renders).
mod warning_sources {
    use super::*;
    use base64::Engine as _;
    use photocraft_color::PixelFormat;
    use serde_json::json;

    fn artwork(width: u32, height: u32) -> String {
        let image =
            photocraft_codecs::Image::from_u8(width, height, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat((width * height) as usize)).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image width="8" height="16" preserveAspectRatio="none" href="data:image/png;base64,{encoded}"/><rect x="8" width="8" height="16" fill="#808080"/><image width="1" height="1" href="data:image/png;base64,A"/></svg>"##
        )
    }

    fn memo_object(svg: &str) -> SmartObject {
        object("Memo.svg", Arc::new(svg.as_bytes().to_vec()))
    }

    fn decodes() -> usize {
        source_cache_statistics().svg_decodes
    }

    fn draw(sources: &mut WarningRenderSources<'_>, sm: &SmartObject) -> Surface {
        let (surface, warnings) = render_with_warnings_using(sm, sources).unwrap();
        assert_eq!(warnings.len(), 1);
        surface.unwrap()
    }

    #[test]
    fn warning_source_profile_context_stays_lazy_for_non_svg_and_missing_sources() {
        let doc = Document::new("Raster", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let sm = object("Raster.png", Arc::new(png));
        let expected = render(&doc, &sm).unwrap();
        let mut sources = WarningRenderSources::new(&doc);
        assert!(!sources.profile_initialized());
        let start = decodes();
        let (surface, warnings) = render_with_warnings_using(&sm, &mut sources).unwrap();
        assert_eq!(surface, expected);
        assert!(warnings.is_empty());
        assert!(!sources.profile_initialized(), "a strict non-SVG cache hit needs no SVG color context");
        let missing = SmartObject::new(SmartSource::Linked { path: String::new() }, Affine::IDENTITY, None);
        let (surface, warnings) = render_with_warnings_using(&missing, &mut sources).unwrap();
        assert!(surface.is_none());
        assert!(warnings.is_empty());
        assert!(!sources.profile_initialized());
        assert_eq!(decodes() - start, 0);
        draw(&mut sources, &memo_object(&artwork(4, 4)));
        assert!(sources.profile_initialized());
        assert_eq!(decodes() - start, 1);
    }

    #[test]
    fn local_svg_sources_separate_scale_and_profile_but_keep_instance_filters_and_placement() {
        // Reads the command registry before any session exists.
        crate::install().unwrap();
        let mut doc = Document::new("Profile", Size::new(64, 64), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = Some(craft_cms::Builtin::Srgb.profile().to_bytes());
        let mut linear = doc.clone();
        linear.icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let first = memo_object(&artwork(4, 4));
        let mut moved = first.clone();
        moved.transform = Affine::translate(20., 0.);
        moved.smart_filters.push(SmartFilter {
            command: "image.adjustments.invert".into(),
            params: json!({}),
            blend: BlendMode::Normal,
            opacity: 0.5,
            visible: true,
        });
        let mut sources = WarningRenderSources::new(&doc);
        let start = decodes();
        let a = draw(&mut sources, &first);
        let b = draw(&mut sources, &moved);
        assert_eq!(decodes() - start, 1);
        assert_eq!(a.rgba(3, 3), [0., 1., 0., 1.]);
        assert_eq!(b.rgba(3, 3), [0.; 4]);
        assert!(b.rgba(23, 3).iter().take(3).all(|value| (*value - 0.5).abs() < 0.01));
        let mut scaled = first.clone();
        scaled.transform = Affine { m: [2., 0., 0., 2., 0., 0.] };
        draw(&mut sources, &scaled);
        assert_eq!(decodes() - start, 2, "different raster scales require different source renders");
        let corrected = draw(&mut WarningRenderSources::new(&linear), &first);
        assert_eq!(decodes() - start, 3, "a different document interpretation cannot reuse the earlier source");
        assert!((a.rgba(12, 3)[0] - 0.502).abs() < 0.01);
        assert!((corrected.rgba(12, 3)[0] - 0.216).abs() < 0.01);
        assert_eq!(draw(&mut sources, &first), a);
        assert_eq!(decodes() - start, 3);
    }

    #[test]
    fn local_svg_source_memo_evicts_and_oversize_entries_render_without_retention() {
        let mut doc = Document::new("Budget", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        doc.icc_profile = Some(craft_cms::Builtin::Srgb.profile().to_bytes());
        let first = memo_object(&artwork(4, 4));
        let mut scaled = first.clone();
        scaled.transform = Affine { m: [2., 0., 0., 2., 0., 0.] };
        let tile = photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 32, 32), &PixelFormat::RGBA8) as usize;
        for (budget, limit) in [(tile + 4096, 32), (usize::MAX, 1)] {
            let mut sources = WarningRenderSources::with_limits(&doc, budget, limit);
            let start = decodes();
            let expected = draw(&mut sources, &first);
            draw(&mut sources, &scaled);
            assert_eq!(draw(&mut sources, &first), expected);
            assert_eq!(decodes() - start, 3, "byte and entry limits both evict the old scale");
        }
        let huge_source = artwork(4, 4).replace("</svg>", &format!("<!--{}--></svg>", " ".repeat(tile + 8192)));
        let encoded = memo_object(&huge_source);
        let mut sources = WarningRenderSources::with_limits(&doc, tile + 4096, 32);
        let start = decodes();
        let expected = draw(&mut sources, &encoded);
        assert_eq!(draw(&mut sources, &encoded), expected);
        assert_eq!(decodes() - start, 1, "the immutable document owns encoded bytes; only rendered output belongs to the memo budget");
        let mut output_too_large = WarningRenderSources::with_limits(&doc, tile - 1, 32);
        let start = decodes();
        let expected = draw(&mut output_too_large, &first);
        assert_eq!(draw(&mut output_too_large, &first), expected);
        assert_eq!(decodes() - start, 2, "oversize rendered output remains an uncached success");
        let mut disabled = WarningRenderSources::with_limits(&doc, 0, 0);
        assert_eq!(draw(&mut disabled, &first), draw(&mut WarningRenderSources::new(&doc), &first));
    }
}

/// Explicit refreshes and warp frames of one shared native source verify its preview hash once,
/// not once per call; a replaced source is verified again and a forged preview still fails.
#[test]
fn native_preview_digest_is_reused_per_source_allocation_and_rechecked_after_replacement() {
    const RED: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="red"/></svg>"#;
    const BLUE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="blue"/></svg>"#;
    let mut s = crate::session();
    s.execute("file.new", serde_json::json!({"width":32,"height":24,"depth":8})).unwrap();
    let placed = crate::file_cmds::place_bytes(&mut s, "Artwork.svg", RED.to_vec(), None, &serde_json::json!({"smartObject":true})).unwrap();
    let id = LayerId(placed["layer"].as_u64().unwrap());
    let original = b"%!PS-Adobe-3.0 EPSF-3.0\noriginal";
    let mut changed = vec![b' '; 1024 * 1024];
    changed[..original.len()].copy_from_slice(original);
    let doc = s.active().unwrap().doc.clone();
    let LayerContent::Smart(sm) = &doc.layer(id).unwrap().content else { panic!("smart") };
    let mut sm = sm.clone();
    // What saving an EPS with its SVG preview stores: the source and the preview verified for it.
    sm.vector_preview = Some(photocraft_doc::VectorPreview { source_hash: *blake3::hash(&changed).as_bytes(), svg: Arc::new(BLUE.to_vec()) });
    // A fresh allocation of the same bytes: nothing about it can be remembered yet.
    sm.source = SmartSource::Embedded { file_name: "Logo.eps".into(), bytes: Arc::new(changed.clone()) };
    let hashed = || source_cache_statistics().native_hash_bytes;
    let start = hashed();
    let frame = source_box(&doc, &sm).unwrap();
    for _ in 0..3 {
        assert_eq!(source_box(&doc, &sm), Some(frame));
        assert_eq!(render(&doc, &sm).unwrap().unwrap().pixel(15, 11), vec![0.0, 0.0, 1.0, 1.0], "the verified preview is drawn");
    }
    assert_eq!(hashed() - start, changed.len(), "one verification for one source allocation");
    // Different bytes in a new allocation never inherit the verified digest.
    let mut other = changed.clone();
    other[original.len()] = b'%';
    sm.source = SmartSource::Embedded { file_name: "Logo.eps".into(), bytes: Arc::new(other) };
    let before = hashed();
    assert!(render(&doc, &sm).is_err(), "a stale preview must not draw a different source");
    assert_eq!(hashed() - before, changed.len());
}
