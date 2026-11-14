use super::{ChanRef, gray_plane};
use crate::Session;
use craft_cms::Builtin;
use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

const POINTS: [(i32, i32); 4] = [(50, 50), (300, 50), (50, 200), (225, 150)];
const GRAY: [u8; 4] = [129, 144, 129, 176];

fn fixture() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "target", "width": 400, "height": 300, "background": "white"})).unwrap();
    s.edit("fixture", |doc, active| {
        let format = doc.pixel_format();
        let background = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        background.fill_rect(Rect::new(0, 0, 200, 300), &[200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 1.0]);
        background.fill_rect(Rect::new(200, 0, 400, 300), &[50.0 / 255.0, 150.0 / 255.0, 250.0 / 255.0, 1.0]);
        let mut top = Layer::raster("Top", format);
        top.surface_mut().unwrap().fill_rect(Rect::new(150, 100, 250, 200), &[0.0, 1.0, 0.0, 128.0 / 255.0]);
        doc.layers.push(top);
        let mut masked = Layer::raster("Masked", format);
        masked.surface_mut().unwrap().fill_rect(doc.bounds(), &[1.0, 0.0, 0.0, 1.0]);
        let mut mask = LayerMask::hide_all();
        mask.surface.fill_rect(Rect::new(0, 0, 200, 300), &[1.0]);
        masked.mask = Some(mask);
        *active = Some(masked.id);
        doc.layers.push(masked);
        Ok(())
    })
    .unwrap();
    s
}

fn samples(surface: &Surface) -> [f32; 4] {
    POINTS.map(|(x, y)| surface.sample_channel(x, y, 0))
}

fn assert_values(actual: [f32; 4], expected: [u8; 4]) {
    for (value, reference) in actual.into_iter().zip(expected) {
        assert!((value * 255.0 - f32::from(reference)).abs() <= 1.01, "{actual:?}: expected {expected:?}");
    }
}

fn result_samples(s: &Session, result: &Value, kind: &str) -> [f32; 4] {
    let doc = &s.active().unwrap().doc;
    let surface = match kind {
        "newChannel" => &doc.channels[result["channel"].as_u64().unwrap() as usize].surface,
        "selection" => doc.selection.as_ref().unwrap(),
        "newDocument" => doc.layers[0].surface().unwrap(),
        _ => panic!("unknown result kind"),
    };
    samples(surface)
}

#[test]
fn measured_merged_gray_and_rgb_alias_match_in_every_result() {
    for kind in ["newChannel", "newDocument", "selection"] {
        for channel in ["composite", "RGB"] {
            let mut s = fixture();
            let before = s.active().unwrap().doc.clone();
            let result = s
                .execute(
                    "image.calculations",
                    json!({"source1": {"layer": "merged", "channel": channel},
                        "source2": {"layer": "Background", "channel": "green"}, "blending": "normal", "result": kind}),
                )
                .unwrap();
            assert_values(result_samples(&s, &result, kind), GRAY);
            if kind == "newDocument" {
                assert_eq!(s.documents()[0].doc, before, "source document is untouched");
            } else {
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, before, "one undo restores the source");
            }
        }
    }
}

#[test]
fn second_source_uses_the_same_gray_conversion() {
    let mut s = fixture();
    let result = s
        .execute(
            "image.calculations",
            json!({"source1": {"layer": "Masked", "channel": "green"},
                "source2": {"layer": "merged", "channel": "RGB"}, "blending": "normal", "opacity": 0}),
        )
        .unwrap();
    assert_values(result_samples(&s, &result, "newChannel"), GRAY);
}

#[test]
fn calculations_gray_mask_matches_in_every_result() {
    for kind in ["newChannel", "newDocument", "selection"] {
        let mut s = fixture();
        let result = s
            .execute(
                "image.calculations",
                json!({"source1": {"layer": "Background", "channel": "transparency"},
                    "source2": {"layer": "Masked", "channel": "green"}, "blending": "normal", "result": kind,
                    "mask": {"layer": "merged", "channel": "RGB"}}),
            )
            .unwrap();
        assert_values(result_samples(&s, &result, kind), GRAY);
    }
}

#[test]
fn apply_image_mask_uses_the_same_gray_conversion() {
    let mut s = fixture();
    let source_before = s.active().unwrap().doc.clone();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "#000000"})).unwrap();
    let target_before = s.active().unwrap().doc.clone();
    s.execute(
        "image.applyImage",
        json!({"source": {"document": 0, "layer": "Background", "channel": "transparency"},
            "blending": "normal", "mask": {"document": 0, "layer": "merged", "channel": "RGB"}}),
    )
    .unwrap();
    let target = s.active().unwrap();
    assert_values(samples(target.doc.layer(target.active_layer.unwrap()).unwrap().surface().unwrap()), GRAY);
    assert_eq!(s.documents()[0].doc, source_before);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, target_before);
}

#[test]
fn individual_rgb_channels_keep_their_values() {
    let mut s = fixture();
    let result = s
        .execute(
            "image.calculations",
            json!({"source1": {"layer": "merged", "channel": "red"},
                "source2": {"layer": "Background", "channel": "green"}, "blending": "normal"}),
        )
        .unwrap();
    assert_values(result_samples(&s, &result, "newChannel"), [255, 50, 255, 25]);
}

#[test]
fn grayscale_sources_keep_their_native_values_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
        s.edit("fixture", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 300), &[0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let result = s.execute("image.calculations", json!({"source1Channel": "Gray", "source2Channel": "Gray", "blending": "normal"})).unwrap();
        assert_values(result_samples(&s, &result, "newChannel"), [0, 255, 0, 255]);
    }
}

#[test]
fn unmeasured_rgb_depths_and_profiles_keep_legacy_gray_values() {
    for (depth, profile) in [(16, None), (32, None), (8, Some(Builtin::DisplayP3)), (8, Some(Builtin::AdobeRgbCompat))] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2, "height": 1, "depth": depth, "background": "#ff0000"})).unwrap();
        if let Some(profile) = profile {
            s.edit("profile", |doc, _| {
                doc.icc_profile = Some(profile.profile().to_bytes());
                Ok(())
            })
            .unwrap();
        }
        let doc: &Document = &s.active().unwrap().doc;
        let plane = gray_plane(doc, None, None, ChanRef::Composite).unwrap();
        assert!(plane.iter().all(|v| (v - 0.299).abs() < 1e-5), "depth {depth}, profile {profile:?}: {plane:?}");
    }
}
