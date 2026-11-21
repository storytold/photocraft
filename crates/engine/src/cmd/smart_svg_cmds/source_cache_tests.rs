use super::*;
use crate::smart_cmds::{WARNING_SOURCE_DECODES, WARNING_SOURCE_LIMIT, WarningRenderSources, render_with_warnings_using};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::SmartFilter;
use photocraft_geom::Affine;

fn artwork(width: u32, height: u32) -> String {
    let image = photocraft_codecs::Image::from_u8(width, height, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat((width * height) as usize)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image width="8" height="16" preserveAspectRatio="none" href="data:image/png;base64,{encoded}"/><rect x="8" width="8" height="16" fill="#808080"/><image width="1" height="1" href="data:image/png;base64,A"/></svg>"##
    )
}

fn object(svg: &str) -> SmartObject {
    SmartObject::new(SmartSource::Embedded { file_name: "Memo.svg".into(), bytes: Arc::new(svg.as_bytes().to_vec()) }, Affine::IDENTITY, None)
}

fn count() -> usize {
    WARNING_SOURCE_DECODES.with(std::cell::Cell::get)
}

fn reset() {
    WARNING_SOURCE_DECODES.with(|count| count.set(0));
}

fn assert_update_warnings(result: &Value) {
    let warnings = result["warnings"].as_array().unwrap();
    let omitted = "SVG: 1 embedded image was omitted";
    assert_eq!(warnings.iter().filter(|warning| warning.as_str().unwrap().starts_with(omitted)).count(), 1);
    assert!(warnings.len() <= 2, "at most one omission and one image-layer information notice: {warnings:?}");
    assert!(warnings.iter().all(|warning| {
        let warning = warning.as_str().unwrap();
        warning.starts_with(omitted) || warning == "SVG: embedded images are pixel layers"
    }));
}

fn draw(sources: &mut WarningRenderSources<'_>, sm: &SmartObject) -> photocraft_raster::Surface {
    let (surface, warnings) = render_with_warnings_using(sm, sources).unwrap();
    assert_eq!(warnings.len(), 1);
    surface.unwrap()
}

fn shared_session(instances: usize) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":32})).unwrap();
    let original = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#;
    let placed = crate::file_cmds::place_bytes(&mut s, "Shared.svg", original.to_vec(), None, &json!({"smartObject":true})).unwrap();
    let id = LayerId(placed["layer"].as_u64().unwrap());
    let first = s.active().unwrap().doc.layer(id).unwrap().clone();
    let doc = Arc::make_mut(s.active_mut().unwrap().doc_mut());
    for index in 1..instances {
        let mut layer = first.duplicate();
        let LayerContent::Smart(sm) = &mut layer.content else { panic!("smart") };
        sm.transform = Affine::translate((index % 4) as f64, (index % 3) as f64);
        layer.opacity = 0.5;
        doc.layers.push(layer);
    }
    (s, id)
}

fn request(s: &mut Session, id: LayerId, svg: &str) -> Value {
    let handoff = s.execute(EXPORT, json!({"layer":id.0})).unwrap();
    json!({"document":handoff["document"],"layer":id.0,"contentsId":handoff["contentsId"],"fingerprint":handoff["fingerprint"],"svg":svg})
}

#[test]
fn warning_source_profile_context_stays_lazy_for_non_svg_and_missing_sources() {
    let doc = Document::new("Raster", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let sm = SmartObject::new(SmartSource::Embedded { file_name: "Raster.png".into(), bytes: Arc::new(png) }, Affine::IDENTITY, None);
    let expected = crate::smart_cmds::render(&doc, &sm).unwrap();
    let mut sources = WarningRenderSources::new(&doc);
    assert!(!sources.profile_initialized());
    reset();
    let (surface, warnings) = render_with_warnings_using(&sm, &mut sources).unwrap();
    assert_eq!(surface, expected);
    assert!(warnings.is_empty());
    assert!(!sources.profile_initialized(), "a strict non-SVG cache hit needs no SVG color context");
    let missing = SmartObject::new(SmartSource::Linked { path: String::new() }, Affine::IDENTITY, None);
    let (surface, warnings) = render_with_warnings_using(&missing, &mut sources).unwrap();
    assert!(surface.is_none());
    assert!(warnings.is_empty());
    assert!(!sources.profile_initialized());
    assert_eq!(count(), 0);
    draw(&mut sources, &object(&artwork(4, 4)));
    assert!(sources.profile_initialized());
    assert_eq!(count(), 1);
}

#[test]
fn shared_svg_update_decodes_one_source_for_equal_scale_and_keeps_warning_caches_private() {
    let (mut s, id) = shared_session(100);
    let before = s.active().unwrap().doc.clone();
    let svg = artwork(8, 8);
    let params = request(&mut s, id, &svg);
    reset();
    let result = s.execute(UPDATE, params).unwrap();
    assert_eq!(count(), 1, "each placement must reuse the update's source-space render");
    assert_update_warnings(&result);
    let doc = &s.active().unwrap().doc;
    let instances: Vec<_> = doc.layers.iter().filter(|layer| matches!(&layer.content, LayerContent::Smart(_))).collect();
    assert_eq!(instances.len(), 100);
    for layer in instances {
        let sm = smart(doc, layer.id).unwrap();
        let previous = before.layer(layer.id).unwrap();
        assert_eq!(layer.opacity, previous.opacity);
        assert_eq!(sm.transform, smart(&before, layer.id).unwrap().transform);
        assert_eq!(embedded(doc, sm).unwrap().1, svg.as_bytes());
        let x = sm.transform.m[4] as i32;
        let y = sm.transform.m[5] as i32;
        assert_eq!(sm.cache.as_ref().unwrap().rgba(x + 3, y + 3), [0., 1., 0., 1.]);
    }
    let sm = smart(doc, id).unwrap();
    assert!(crate::smart_cmds::render(doc, sm).is_err(), "strict global cache must remain unpoisoned");
    let mut stack = sm.clone();
    stack.stack_mode = Some(photocraft_doc::StackMode::Mean);
    assert!(crate::smart_cmds::render(doc, &stack).is_err(), "Stack Mode still requires complete artwork");
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.redo());
    assert_eq!(embedded(&s.active().unwrap().doc, smart(&s.active().unwrap().doc, id).unwrap()).unwrap().1, svg.as_bytes());
}

#[test]
fn local_svg_sources_separate_scale_and_profile_but_keep_instance_filters_and_placement() {
    let mut doc = Document::new("Profile", Size::new(64, 64), ColorMode::Rgb, SampleType::F32);
    doc.icc_profile = Some(craft_cms::Builtin::Srgb.profile().to_bytes());
    let mut linear = doc.clone();
    linear.icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let first = object(&artwork(4, 4));
    let mut moved = first.clone();
    moved.transform = Affine::translate(20., 0.);
    moved.smart_filters.push(SmartFilter {
        command: "image.adjustments.invert".into(),
        params: json!({}),
        blend: photocraft_color::BlendMode::Normal,
        opacity: 0.5,
        visible: true,
    });
    let mut sources = WarningRenderSources::new(&doc);
    reset();
    let a = draw(&mut sources, &first);
    let b = draw(&mut sources, &moved);
    assert_eq!(count(), 1);
    assert_eq!(a.rgba(3, 3), [0., 1., 0., 1.]);
    assert_eq!(b.rgba(3, 3), [0.; 4]);
    assert!(b.rgba(23, 3).iter().take(3).all(|value| (*value - 0.5).abs() < 0.01));
    let mut scaled = first.clone();
    scaled.transform = Affine { m: [2., 0., 0., 2., 0., 0.] };
    draw(&mut sources, &scaled);
    assert_eq!(count(), 2, "different raster scales require different source renders");
    let corrected = draw(&mut WarningRenderSources::new(&linear), &first);
    assert_eq!(count(), 3, "a different document interpretation cannot reuse the earlier source");
    assert!((a.rgba(12, 3)[0] - 0.502).abs() < 0.01);
    assert!((corrected.rgba(12, 3)[0] - 0.216).abs() < 0.01);
    assert_eq!(draw(&mut sources, &first), a);
    assert_eq!(count(), 3);
}

#[test]
fn local_svg_source_memo_evicts_and_oversize_entries_render_without_retention() {
    let mut doc = Document::new("Budget", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    doc.icc_profile = Some(craft_cms::Builtin::Srgb.profile().to_bytes());
    let first = object(&artwork(4, 4));
    let mut scaled = first.clone();
    scaled.transform = Affine { m: [2., 0., 0., 2., 0., 0.] };
    let tile = photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 32, 32), &PixelFormat::RGBA8) as usize;
    for (budget, limit) in [(tile + 4096, 32), (usize::MAX, 1)] {
        let mut sources = WarningRenderSources::with_limits(&doc, budget, limit);
        reset();
        let expected = draw(&mut sources, &first);
        draw(&mut sources, &scaled);
        assert_eq!(draw(&mut sources, &first), expected);
        assert_eq!(count(), 3, "byte and entry limits both evict the old scale");
    }
    let huge_source = artwork(4, 4).replace("</svg>", &format!("<!--{}--></svg>", " ".repeat(tile + 8192)));
    let encoded = object(&huge_source);
    let mut sources = WarningRenderSources::with_limits(&doc, tile + 4096, 32);
    reset();
    let expected = draw(&mut sources, &encoded);
    assert_eq!(draw(&mut sources, &encoded), expected);
    assert_eq!(count(), 1, "the immutable document owns encoded bytes; only rendered output belongs to the memo budget");
    let mut output_too_large = WarningRenderSources::with_limits(&doc, tile - 1, 32);
    reset();
    let expected = draw(&mut output_too_large, &first);
    assert_eq!(draw(&mut output_too_large, &first), expected);
    assert_eq!(count(), 2, "oversize rendered output remains an uncached success");
    let mut disabled = WarningRenderSources::with_limits(&doc, 0, 0);
    assert_eq!(draw(&mut disabled, &first), draw(&mut WarningRenderSources::new(&doc), &first));
}

#[test]
fn shared_svg_update_render_failure_and_cancelled_authorization_preserve_document_history() {
    for cancel in [false, true] {
        let (mut s, id) = shared_session(2);
        let doc = Arc::make_mut(s.active_mut().unwrap().doc_mut());
        let LayerContent::Smart(sm) = &mut doc.layers.last_mut().unwrap().content else { panic!("smart") };
        sm.smart_filters.push(SmartFilter {
            command: if cancel { "image.adjustments.invert" } else { "unsupported.persisted.filter" }.into(),
            params: json!({}),
            blend: photocraft_color::BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
        if cancel {
            s.authorize = Some(|command, _| if command == "image.adjustments.invert" { Err(EngineError::Cancelled) } else { Ok(()) });
        }
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        let revision = s.active().unwrap().revision;
        let params = request(&mut s, id, &artwork(4, 4));
        reset();
        let error = s.execute(UPDATE, params).unwrap_err();
        if cancel {
            assert!(matches!(error, EngineError::Cancelled), "expected the authority cancellation, got {error}");
        } else {
            assert!(error.to_string().contains("unsupported visible filters"), "{error}");
        }
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(count(), if cancel { 0 } else { 1 });
    }
}

#[test]
fn shared_svg_update_keeps_the_existing_placed_pixel_budget_atomic() {
    let (mut s, id) = shared_session(2);
    let LayerContent::Smart(sm) = &mut Arc::make_mut(s.active_mut().unwrap().doc_mut()).layers.last_mut().unwrap().content else { panic!("smart") };
    sm.transform = Affine { m: [1000., 0., 0., 1000., 0., 0.] };
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    let revision = s.active().unwrap().revision;
    let params = request(&mut s, id, &artwork(4, 4));
    reset();
    assert!(s.execute(UPDATE, params).is_err());
    assert_eq!(count(), 0, "placed-frame admission precedes source rendering");
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.active().unwrap().revision, revision);
}

#[test]
#[ignore = "24MP embedded PNG / 100 shared instances; run once in release for performance qualification"]
fn shared_svg_update_24mp_source_timing() {
    struct MemoLimit(Option<usize>);
    impl MemoLimit {
        fn set(limit: usize) -> Self {
            Self(WARNING_SOURCE_LIMIT.with(|current| current.replace(Some(limit))))
        }
    }
    impl Drop for MemoLimit {
        fn drop(&mut self) {
            WARNING_SOURCE_LIMIT.with(|current| current.set(self.0));
        }
    }
    let previous_limit = WARNING_SOURCE_LIMIT.with(std::cell::Cell::get);
    // Encode once. Each pass builds the same initial document and handoff outside
    // the clock; UPDATE includes its normal import and all instance rendering.
    let svg = artwork(6000, 4000);
    let mut expected = None;
    let mut timings = Vec::new();
    for (label, limit, decodes) in [("optimized-before", 32, 1), ("memo-off", 0, 100), ("optimized-after", 32, 1)] {
        let (mut s, id) = shared_session(100);
        let params = request(&mut s, id, &svg);
        let _restore = MemoLimit::set(limit);
        reset();
        let start = std::time::Instant::now();
        let result = s.execute(UPDATE, params).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(count(), decodes, "{label}");
        assert_update_warnings(&result);
        let doc = &s.active().unwrap().doc;
        let mut pixels = blake3::Hasher::new();
        let mut sources = blake3::Hasher::new();
        let mut samples = Vec::new();
        for layer in doc.layers.iter().filter(|layer| matches!(&layer.content, LayerContent::Smart(_))) {
            let sm = smart(doc, layer.id).unwrap();
            let source = embedded(doc, sm).unwrap().1;
            assert_eq!(source, svg.as_bytes());
            sources.update(blake3::hash(source).as_bytes());
            let cache = sm.cache.as_ref().unwrap();
            for value in cache.read_region(doc.bounds()) {
                pixels.update(&value.to_bits().to_le_bytes());
            }
            let (x, y) = (sm.transform.m[4] as i32, sm.transform.m[5] as i32);
            let sample = cache.rgba(x + 3, y + 3);
            assert_eq!(sample, [0., 1., 0., 1.]);
            samples.push(sample);
        }
        assert_eq!(samples.len(), 100);
        let signature = (*pixels.finalize().as_bytes(), *sources.finalize().as_bytes(), samples, result["warnings"].clone());
        if let Some(expected) = &expected {
            assert_eq!(&signature, expected, "pixels, samples, encoded sources and warnings must match: {label}");
        } else {
            expected = Some(signature);
        }
        eprintln!("24MP PNG / 16x16 SVG / 100 shared instances: {label}: {elapsed:.3} ms, {decodes} source renders");
        timings.push(elapsed);
    }
    eprintln!("optimized median of two: {:.3} ms; memo-off: {:.3} ms", (timings[0] + timings[2]) / 2., timings[1]);
    assert_eq!(WARNING_SOURCE_LIMIT.with(std::cell::Cell::get), previous_limit, "test override restored");
}
