//! Independent native controls are the oracle, rather than a copy of the XCF cache.
use super::*;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Affine, Document, Layer, LayerContent, LayerMask, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Rect, Size};
use photocraft_io::native_xcf_tones::fixtures::{MaskedExposureOptions, masked_exposure};
use photocraft_raster::Surface;
use serde_json::json;
use std::sync::Arc;

fn session() -> crate::Session {
    let mut session = crate::session();
    session.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    session
}

fn ordinary_native_depth(depth: SampleType) -> (Document, Surface, SmartObject) {
    let document = Document::new("Native reference", Size::new(4, 2), ColorMode::Rgb, depth);
    let mut original = Surface::new(document.pixel_format());
    let mut placed = Surface::new(document.pixel_format());
    let (rgb, range) =
        if depth == SampleType::U16 { ([[8193u16, 24578, 40963], [49156, 32773, 16390]], 65535.) } else { ([[32u16, 96, 160], [192, 128, 64]], 255.) };
    for (x, rgb) in rgb.into_iter().enumerate() {
        let pixel = [(f64::from(rgb[0]) / range) as f32, (f64::from(rgb[1]) / range) as f32, (f64::from(rgb[2]) / range) as f32, 1.];
        original.write_pixel(x as i32, 0, &pixel);
        placed.write_pixel(x as i32 + 1, 0, &pixel);
    }
    let mut contents = Document::new("Original pixels", Size::new(2, 1), ColorMode::Rgb, depth);
    contents.layers.push(Layer::new("Original pixels", LayerContent::Raster(original)));
    let bytes = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "reference.pcraft".into(), bytes: Arc::new(bytes) }, Affine::translate(1., 0.), None);
    // The independently authored native controls retain scalar opacity and the
    // image-global coverage separately. Source display opacity is not coverage.
    smart.smart_filters.push(SmartFilter {
        command: "image.adjustments.exposure".into(),
        params: json!({"exposure":0.75,"offset":-f64::from(0.03f32)*0.75f64.exp2(),"gamma":1.}),
        blend: BlendMode::Normal,
        opacity: 0.65,
        visible: true,
    });
    let mut coverage = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let (values, range) =
        if depth == SampleType::U16 { ([0u16, 16385, 49153, 65535, 65535, 32769, 8195, 0], 65535.) } else { ([0u16, 64, 192, 255, 255, 128, 32, 0], 255.) };
    for (i, value) in values.into_iter().enumerate() {
        coverage.write_pixel((i % 4) as i32, (i / 4) as i32, &[(f64::from(value) / range) as f32]);
    }
    smart.filter_mask = Some(LayerMask { surface: coverage, enabled: true, linked: true, density: 1., feather: 0. });
    (document, placed, smart)
}

#[test]
fn masked_exposure_open_cache_matches_ordinary_native_controls_and_reopens_exactly() {
    for depth in [SampleType::U8, SampleType::U16] {
        let fixture = masked_exposure(MaskedExposureOptions { u16_owner: depth == SampleType::U16, ..Default::default() });
        let (reference, placed, controls) = ordinary_native_depth(depth);
        let canvas = Rect::from_xywh(0, 0, 4, 2);
        let expected = crate::smart_cmds::apply_smart_filters(&placed, &controls, canvas).unwrap();
        assert_ne!(expected.to_interleaved(canvas), placed.to_interleaved(canvas), "the oracle must exercise exposure, strength and partial coverage");
        let mut opacity_only = controls.clone();
        opacity_only.filter_mask = None;
        assert_ne!(expected.to_interleaved(canvas), crate::smart_cmds::apply_smart_filters(&placed, &opacity_only, canvas).unwrap().to_interleaved(canvas));
        let mut full_strength = controls.clone();
        full_strength.smart_filters[0].opacity = 1.;
        assert_ne!(expected.to_interleaved(canvas), crate::smart_cmds::apply_smart_filters(&placed, &full_strength, canvas).unwrap().to_interleaved(canvas));
        let mut session = session();
        super::super::open_bytes_as(&mut session, "masked.xcf", &fixture.bytes, None, None).unwrap();
        let document = &session.active().unwrap().doc;
        let layer = &document.layers[0];
        let LayerContent::Smart(smart) = &layer.content else { panic!("authorized proven mask must install") };
        assert_eq!(smart.smart_filters, controls.smart_filters);
        assert_eq!(smart.filter_mask, controls.filter_mask);
        assert_eq!(smart.transform, controls.transform);
        assert_eq!(layer.opacity, 0.75);
        assert_eq!(smart.cache.as_ref().unwrap().to_interleaved(canvas), expected.to_interleaved(canvas));
        assert_eq!(crate::smart_cmds::render(document, smart).unwrap().unwrap().to_interleaved(canvas), expected.to_interleaved(canvas));
        let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native controls lost") };
        assert_eq!(again.smart_filters, controls.smart_filters);
        assert_eq!(again.filter_mask, controls.filter_mask);
        assert_eq!(again.source, smart.source);
        assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        assert_eq!(crate::smart_cmds::render(&reopened, again).unwrap().unwrap().to_interleaved(canvas), expected.to_interleaved(canvas));
        assert_eq!(reference.pixel_format(), document.pixel_format());
        let direct = photocraft_io::import("masked.xcf", &fixture.bytes).unwrap();
        let LayerContent::Raster(original) = &direct.document.layers[0].content else { panic!("direct IO must remain inert") };
        assert_eq!(original.to_interleaved(canvas), placed.to_interleaved(canvas));
    }
}

#[test]
fn masked_exposure_unknown_policy_and_invalid_mask_preserve_original_pixels() {
    let canvas = Rect::from_xywh(0, 0, 4, 2);
    for options in [
        MaskedExposureOptions { unknown_policy: true, ..Default::default() },
        MaskedExposureOptions { invalid_mask: true, ..Default::default() },
        MaskedExposureOptions { unknown_policy: true, u16_owner: true, ..Default::default() },
        MaskedExposureOptions { invalid_mask: true, u16_owner: true, ..Default::default() },
    ] {
        let (_, original, _) = ordinary_native_depth(if options.u16_owner { SampleType::U16 } else { SampleType::U8 });
        let fixture = masked_exposure(options);
        let mut session = session();
        super::super::open_bytes_as(&mut session, "unapplied.xcf", &fixture.bytes, None, None).unwrap();
        let document = &session.active().unwrap().doc;
        let LayerContent::Raster(actual) = &document.layers[0].content else { panic!("unproven source must remain raster") };
        assert_eq!(actual.to_interleaved(canvas), original.to_interleaved(canvas));
        let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        let LayerContent::Raster(again) = &reopened.layers[0].content else { panic!("fallback lost") };
        assert_eq!(again.to_interleaved(canvas), original.to_interleaved(canvas));
    }
}

fn deny(_: &str, _: &Value) -> Result<()> {
    Err(EngineError::Other("native request denied".into()))
}
fn cancel(_: &str, _: &Value) -> Result<()> {
    Err(EngineError::Cancelled)
}
#[test]
fn masked_exposure_authority_refusal_precedes_mask_decode_and_preparation() {
    // This damaged mask would otherwise become a nonfatal raster fallback.
    // A denied/cancelled request must instead preserve its typed fatal result.
    let fixture = masked_exposure(MaskedExposureOptions { invalid_mask: true, ..Default::default() });
    for (gate, cancelled) in [(deny as Gate, false), (cancel as Gate, true)] {
        let mut session = session();
        session.authorize = Some(gate);
        let before = photocraft_io::native_xcf_tones::preparation_count();
        let error = super::super::open_bytes_as(&mut session, "denied.xcf", &fixture.bytes, None, None).unwrap_err();
        if cancelled {
            assert_eq!(error, EngineError::Cancelled);
        } else {
            assert!(error.to_string().contains("not authorized"), "{error}");
        }
        assert!(session.documents().is_empty());
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
    }
}
