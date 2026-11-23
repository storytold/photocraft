//! Ordinary native commands are independently authored output/cache oracles.
use super::*;
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Affine, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_io::native_xcf_tones::fixtures::point::{WireValue as W, fixture};
use photocraft_raster::Surface;
use serde_json::json;
use std::sync::Arc;
fn session() -> crate::Session {
    let mut s = crate::session();
    s.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    s
}
fn filter(command: &str, params: Value) -> SmartFilter {
    SmartFilter { command: command.into(), params, blend: BlendMode::Normal, opacity: 1., visible: true }
}
fn original(depth: SampleType) -> Surface {
    let mut out = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, depth, true));
    let values = match depth {
        SampleType::U8 => [32u8, 96, 160, 128, 192, 128, 64, 255].into_iter().map(|v| f32::from(v) / 255.).collect::<Vec<_>>(),
        SampleType::U16 => [8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535].into_iter().map(|v| (f64::from(v) / 65535.) as f32).collect(),
        SampleType::F32 => vec![-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.],
    };
    out.write_region(Rect::from_xywh(-1, 1, 2, 1), &values);
    out
}
#[test]
fn scalar_color_native_controls_match_caches_and_rerender_after_reopen_at_each_depth() {
    let cases = [
        ("gegl:invert-linear", vec![], vec![filter("image.adjustments.invert", json!({}))]),
        ("gegl:invert-gamma", vec![], vec![filter("image.adjustments.invert", json!({}))]),
        ("gimp:posterize", vec![("levels", W::Int(8))], vec![filter("image.adjustments.posterize", json!({"levels":8.}))]),
        (
            "gegl:vibrance",
            vec![("vibrance", W::Float(25.)), ("saturation", W::Float(3.))],
            vec![
                filter("image.adjustments.vibrance", json!({"vibrance":25.,"saturation":100.})),
                filter("image.adjustments.vibrance", json!({"vibrance":0.,"saturation":50.})),
            ],
        ),
        (
            "gegl:saturation",
            vec![("scale", W::Float(2.)), ("colorspace", W::Enum(0))],
            vec![filter("image.adjustments.hueSaturation", json!({"hue":0.,"saturation":50.,"lightness":0.,"colorize":false}))],
        ),
        (
            "gegl:channel-mixer",
            vec![("rr-gain", W::Float(0.5)), ("rg-gain", W::Float(0.5))],
            vec![filter("image.adjustments.channelMixer", json!({"red":[50.,50.,0.,0.],"green":[0.,100.,0.,0.],"blue":[0.,0.,100.,0.],"monochrome":false}))],
        ),
        (
            "gegl:mono-mixer",
            vec![("red", W::Float(0.5)), ("green", W::Float(0.25)), ("blue", W::Float(0.25))],
            vec![filter("image.adjustments.channelMixer", json!({"gray":[50.,25.,25.,0.],"monochrome":true}))],
        ),
        (
            "gegl:sepia",
            vec![("scale", W::Float(0.5)), ("srgb", W::Bool(false))],
            vec![filter("image.adjustments.photoFilter", json!({"color":[172./255.,122./255.,51./255.],"density":50.,"preserveLuminosity":true}))],
        ),
    ];
    let area = Rect::from_xywh(-1, 1, 2, 1);
    let canvas = Rect::from_xywh(0, 0, 3, 3);
    for (op, args, expected_filters) in cases {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bytes = fixture(op, &args, depth, None);
            let before = original(depth);
            let mut controls =
                SmartObject::new(SmartSource::Embedded { file_name: "unused.pcraft".into(), bytes: Arc::new(Vec::new()) }, Affine::translate(-1., 1.), None);
            controls.smart_filters = expected_filters.clone();
            let expected = crate::smart_cmds::apply_smart_filters(&before, &controls, canvas).unwrap();
            assert_ne!(expected.to_interleaved(area), before.to_interleaved(area), "oracle must exercise {op} {depth:?}");
            let mut s = session();
            super::super::open_bytes_as(&mut s, "point.xcf", &bytes, None, None).unwrap();
            let document = &s.active().unwrap().doc;
            let LayerContent::Smart(smart) = &document.layers[0].content else { panic!("{op} {depth:?} missing") };
            assert_eq!(smart.smart_filters, expected_filters);
            assert_eq!(smart.cache.as_ref().unwrap().to_interleaved(area), expected.to_interleaved(area), "{op} {depth:?}");
            assert_eq!(crate::smart_cmds::render(document, smart).unwrap().unwrap().to_interleaved(area), expected.to_interleaved(area));
            for x in -1..1 {
                assert_eq!(smart.cache.as_ref().unwrap().rgba(x, 1)[3], before.rgba(x, 1)[3]);
            }
            let native = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native controls lost") };
            assert_eq!(again.smart_filters, expected_filters);
            assert_eq!(again.source, smart.source);
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
            assert_eq!(crate::smart_cmds::render(&reopened, again).unwrap().unwrap().to_interleaved(area), expected.to_interleaved(area));
        }
    }
}
fn deny_color(command: &str, _: &Value) -> Result<()> {
    if command == "image.adjustments.vibrance" { Err(EngineError::Other("color denied".into())) } else { Ok(()) }
}
#[test]
fn denied_color_mapping_never_generates_native_stages() {
    let bytes = fixture("gegl:vibrance", &[("vibrance", W::Float(25.))], SampleType::U8, None);
    let mut s = session();
    s.authorize = Some(deny_color);
    let before = photocraft_io::native_xcf_tones::preparation_count();
    assert!(super::super::open_bytes_as(&mut s, "denied.xcf", &bytes, None, None).unwrap_err().to_string().contains("not authorized"));
    assert!(s.documents().is_empty());
    assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
}
