use super::*;
use crate::smart_cmds::source_cache_statistics;
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

fn decodes() -> usize {
    source_cache_statistics().svg_decodes
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

fn shared_session(instances: usize) -> (Session, LayerId) {
    let mut s = crate::session();
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
fn shared_svg_update_decodes_one_source_for_equal_scale_and_keeps_warning_caches_private() {
    let (mut s, id) = shared_session(100);
    let before = s.active().unwrap().doc.clone();
    let svg = artwork(8, 8);
    let params = request(&mut s, id, &svg);
    let start = decodes();
    let result = s.execute(UPDATE, params).unwrap();
    assert_eq!(decodes() - start, 1, "each placement must reuse the update's source-space render");
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
        let start = decodes();
        let error = s.execute(UPDATE, params).unwrap_err();
        if cancel {
            assert!(matches!(error, EngineError::Cancelled), "expected the authority cancellation, got {error}");
        } else {
            assert!(error.to_string().contains("unsupported visible filters"), "{error}");
        }
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(decodes() - start, if cancel { 0 } else { 1 });
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
    let start = decodes();
    assert!(s.execute(UPDATE, params).is_err());
    assert_eq!(decodes() - start, 0, "placed-frame admission precedes source rendering");
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.active().unwrap().revision, revision);
}
