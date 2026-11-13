use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Layer, Size};
use photocraft_engine::Session;
use photocraft_geom::warp::Warp;

#[test]
fn cylinder_proxy_preview_matches_native_commit() {
    let background = [0.0, 0.0, 0.0, 1.0];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::new("Cylinder proxy", Size::new(144, 108), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("gradient", doc.pixel_format());
        let id = layer.id;
        let pixels: Vec<f32> =
            (0..108).flat_map(|y| (0..144).flat_map(move |x| [0.1 + 0.8 * x as f32 / 143.0, 0.3, 0.4 + 0.2 * y as f32 / 107.0, 1.0])).collect();
        layer.surface_mut().unwrap().write_region(doc.bounds(), &pixels);
        doc.layers.push(layer);
        for target in [json!({"color": 0}), json!({"colors": [0, 2]})] {
            let mut session = Session::new();
            session.tools.background = background;
            session.add_document(doc.clone(), None);
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.run("channel.target", json!({"colors": target.get("colors").cloned().unwrap_or_else(|| json!([0]))})).unwrap();
            crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
            let t = app.ui.transform.as_mut().unwrap();
            t.interpolation = "bilinear".into();
            t.warp = Some(Warp::cylinder(&[24.0, 84.0, 120.0, 24.0, 0.6, 0.35, 0.7], t.rect).unwrap());
            let t = t.clone();
            let mut full = Session::new();
            full.tools.background = background;
            full.add_document(doc.clone(), None);
            full.execute("edit.transform.warp", json!({"layer": id.0, "target": target, "rect": t.rect, "warp": t.warp, "interpolation": t.interpolation}))
                .unwrap();
            for factor in [1, 2, 3] {
                // Exercise the same proxy construction on a small source: the size heuristic is
                // independent of Cylinder coordinates, and need not make this a megapixel test.
                let mut preview = Preview::new(&doc, Some(id), background, false).unwrap();
                preview.factor = factor;
                preview.base = Arc::new(crate::proxy::proxy_document(&doc, factor));
                preview.shown = preview.base.clone();
                preview.update(&t).unwrap();
                for component in [0, 2].into_iter().filter(|&k| k == 0 || target.get("colors").is_some()) {
                    let params = json!({"target": {"color": component}});
                    let native = photocraft_engine::channel_transform::plane(&full.active().unwrap().doc, Some(id), &params, background).unwrap().unwrap();
                    let expected = crate::proxy::downsample(&native, factor);
                    let actual = photocraft_engine::channel_transform::plane(&preview.shown, Some(id), &params, background).unwrap().unwrap();
                    let a = actual.content_bounds();
                    let e = expected.content_bounds();
                    let difference = [a.x0 - e.x0, a.y0 - e.y0, a.x1 - e.x1, a.y1 - e.y1];
                    assert!(difference.into_iter().all(|n| n.abs() <= 2), "{depth:?}/{target}/{factor}/{component}: actual {a:?}, expected {e:?}");
                    let mut samples = 0;
                    let mut error = 0.0f32;
                    for y in e.y0 + 1..e.y1 - 1 {
                        for x in e.x0 + 1..e.x1 - 1 {
                            if [(x, y), (x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].into_iter().all(|(x, y)| expected.sample_channel(x, y, 0) > 0.05) {
                                error = error.max((actual.sample_channel(x, y, 0) - expected.sample_channel(x, y, 0)).abs());
                                samples += 1;
                            }
                        }
                    }
                    assert!(samples > 100, "a nonempty Cylinder interior must be compared");
                    assert!(error < 0.06, "{depth:?}/{target}/{factor}/{component}: interior error {error}");
                }
            }
            let original = doc.layer(id).unwrap().surface().unwrap();
            let committed = full.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
            for y in 0..108 {
                for x in 0..144 {
                    assert_eq!(original.sample_channel(x, y, 1), committed.sample_channel(x, y, 1));
                    assert_eq!(original.sample_channel(x, y, 3), committed.sample_channel(x, y, 3));
                }
            }
        }
    }
}
