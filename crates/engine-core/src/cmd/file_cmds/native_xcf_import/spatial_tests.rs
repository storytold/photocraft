use super::*;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Affine, LayerContent, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_io::native_xcf_tones::fixtures::point::{WireValue as W, fixture};
use photocraft_raster::Surface;
use serde_json::json;
use std::sync::Arc;
fn filter(id: &str, p: Value) -> SmartFilter {
    SmartFilter { command: id.into(), params: p, blend: BlendMode::Normal, opacity: 1., visible: true }
}
#[test]
fn spatial_cache_and_reopen_match_actual_native_filter_commands_at_each_depth() {
    let cases = [
        (
            "gegl:gaussian-blur",
            vec![("std-dev-x", W::Float(2.)), ("std-dev-y", W::Float(1.))],
            filter("filter.blur.gaussianBlur", json!({"radius":2.5f64.sqrt()})),
        ),
        ("gegl:box-blur", vec![("radius", W::Int(2))], filter("filter.blur.boxBlur", json!({"radius":2.}))),
        (
            "gegl:motion-blur-linear",
            vec![("length", W::Float(3.)), ("angle", W::Float(45.))],
            filter("filter.blur.motionBlur", json!({"distance":3.,"angle":-45.})),
        ),
        (
            "gegl:unsharp-mask",
            vec![("std-dev", W::Float(2.)), ("scale", W::Float(1.5)), ("threshold", W::Float(0.25))],
            filter("filter.sharpen.unsharpMask", json!({"radius":2.,"amount":150.,"threshold":63.75})),
        ),
        ("gegl:high-pass", vec![("std-dev", W::Float(2.))], filter("filter.other.highPass", json!({"radius":2.}))),
    ];
    let canvas = Rect::from_xywh(0, 0, 3, 3);
    let domain = Rect::from_xywh(-1, 0, 4, 3);
    for (op, args, native_filter) in cases {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut original = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let samples = match depth {
                SampleType::U8 => [32u8, 96, 160, 128, 192, 128, 64, 255].into_iter().map(|v| f32::from(v) / 255.).collect::<Vec<_>>(),
                SampleType::U16 => [8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535].into_iter().map(|v| (f64::from(v) / 65535.) as f32).collect(),
                SampleType::F32 => vec![-0.25, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.],
            };
            original.write_region(Rect::from_xywh(-1, 1, 2, 1), &samples);
            let mut controls =
                SmartObject::new(SmartSource::Embedded { file_name: "unused.pcraft".into(), bytes: Arc::new(Vec::new()) }, Affine::translate(-1., 1.), None);
            controls.smart_filters = vec![native_filter.clone()];
            let expected = crate::smart_cmds::apply_smart_filters(&original, &controls, canvas).unwrap();
            assert_ne!(expected.to_interleaved(domain), original.to_interleaved(domain), "{op} oracle must alter pixels");
            let bytes = fixture(op, &args, depth, None);
            let mut s = crate::session();
            s.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
            super::super::open_bytes_as(&mut s, "spatial.xcf", &bytes, None, None).unwrap();
            let document = &s.active().unwrap().doc;
            let LayerContent::Smart(smart) = &document.layers[0].content else { panic!("{op} {depth:?} not native") };
            assert_eq!(smart.smart_filters, vec![native_filter.clone()]);
            assert_eq!(smart.cache.as_ref().unwrap().to_interleaved(domain), expected.to_interleaved(domain), "{op} {depth:?}");
            assert_eq!(crate::smart_cmds::render(document, smart).unwrap().unwrap().to_interleaved(domain), expected.to_interleaved(domain));
            let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Smart(again) = &reopened.layers[0].content else { panic!("native filter lost") };
            assert_eq!(again.smart_filters, smart.smart_filters);
            assert_eq!(again.source, smart.source);
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
            assert_eq!(crate::smart_cmds::render(&reopened, again).unwrap().unwrap().to_interleaved(domain), expected.to_interleaved(domain));
        }
    }
}
