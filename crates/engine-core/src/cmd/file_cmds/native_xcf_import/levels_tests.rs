use super::*;
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Affine, Document, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Rect, Size};
use photocraft_io::native_xcf_tones::fixtures::levels::fixture;
use photocraft_raster::Surface;
use serde_json::json;
use std::sync::Arc;
fn session() -> crate::Session {
    let mut session = crate::session();
    session.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    session
}
fn reference(depth: SampleType) -> (Document, Surface, SmartObject) {
    let document = Document::new("Native Levels reference", Size::new(3, 3), ColorMode::Rgb, depth);
    let mut placed = Surface::new(document.pixel_format());
    let samples: Vec<f32> = match depth {
        SampleType::U8 => [32u8, 96, 160, 128, 192, 128, 64, 255].into_iter().map(|v| f32::from(v) / 255.).collect(),
        SampleType::U16 => [8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535].into_iter().map(|v| (f64::from(v) / 65535.) as f32).collect(),
        SampleType::F32 => vec![-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.],
    };
    placed.write_region(Rect::from_xywh(-1, 1, 2, 1), &samples);
    let mut smart =
        SmartObject::new(SmartSource::Embedded { file_name: "unused.pcraft".into(), bytes: Arc::new(Vec::new()) }, Affine::translate(-1., 1.), None);
    smart.smart_filters.push(SmartFilter {
        command: "image.adjustments.levels".into(),
        params: json!({"inBlack":31.875,"inWhite":223.125,"gamma":1.,"outBlack":63.75,"outWhite":191.25}),
        blend: BlendMode::Normal,
        opacity: 1.,
        visible: true,
    });
    (document, placed, smart)
}

#[test]
fn levels_authorized_open_matches_independent_native_controls_and_rerenders_after_reopen() {
    let depth = SampleType::F32;
    let bytes = fixture(depth, None);
    let (reference, original, controls) = reference(depth);
    let canvas = Rect::from_xywh(0, 0, 3, 3);
    let owner = Rect::from_xywh(-1, 1, 2, 1);
    let expected = crate::smart_cmds::apply_smart_filters(&original, &controls, canvas).unwrap();
    assert_ne!(expected.to_interleaved(owner), original.to_interleaved(owner));
    let mut session = session();
    super::super::open_bytes_as(&mut session, "levels.xcf", &bytes, None, None).unwrap();
    let document = &session.active().unwrap().doc;
    let LayerContent::Smart(smart) = &document.layers[0].content else { panic!("Levels not installed") };
    assert_eq!(smart.smart_filters, controls.smart_filters);
    assert_eq!(smart.transform, controls.transform);
    assert_eq!(smart.cache.as_ref().unwrap().to_interleaved(owner), expected.to_interleaved(owner), "{depth:?}");
    assert_eq!(crate::smart_cmds::render(document, smart).unwrap().unwrap().to_interleaved(owner), expected.to_interleaved(owner));
    for x in -1..1 {
        assert_eq!(smart.cache.as_ref().unwrap().rgba(x, 1)[3], original.rgba(x, 1)[3]);
    }
    let native = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("Levels controls lost") };
    assert_eq!(again.smart_filters, controls.smart_filters);
    assert_eq!(again.source, smart.source);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    assert_eq!(crate::smart_cmds::render(&reopened, again).unwrap().unwrap().to_interleaved(owner), expected.to_interleaved(owner));
    assert_eq!(reference.depth, document.depth);
    let direct = photocraft_io::import("direct.xcf", &bytes).unwrap();
    assert!(matches!(direct.document.layers[0].content, LayerContent::Raster(_)));
}
fn deny_levels(command: &str, _: &Value) -> Result<()> {
    if command == "image.adjustments.levels" { Err(EngineError::Other("Levels denied".into())) } else { Ok(()) }
}
fn cancel_levels(command: &str, _: &Value) -> Result<()> {
    if command == "image.adjustments.levels" { Err(EngineError::Cancelled) } else { Ok(()) }
}
#[test]
fn levels_authority_denial_and_cancellation_precede_any_native_preparation() {
    let bytes = fixture(SampleType::U8, Some("mask"));
    for (gate, cancelled) in [(deny_levels as Gate, false), (cancel_levels as Gate, true)] {
        let mut session = session();
        session.authorize = Some(gate);
        let before = photocraft_io::native_xcf_tones::preparation_count();
        let error = super::super::open_bytes_as(&mut session, "denied-levels.xcf", &bytes, None, None).unwrap_err();
        if cancelled {
            assert_eq!(error, EngineError::Cancelled);
        } else {
            assert!(error.to_string().contains("not authorized"), "{error}");
        }
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
        assert!(session.documents().is_empty());
    }
}
