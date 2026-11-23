//! Tests that run paint commands, moved from the engine-core `symmetry_cmds` tests: 7 test(s).

mod preset_tests {
    use photocraft_engine::Session;
    use photocraft_paint::{StrokePoint, symmetry::SymmetryMode};
    use serde_json::json;

    use photocraft_engine::brush_cmds::LiveStroke;

    use photocraft_doc::{Layer, LayerMask};

    fn session(depth: u8, mode: &str) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":81,"height":61,"depth":depth,"mode":mode,"background":"white"})).unwrap();
        s
    }

    fn pixel(s: &Session, x: i32, y: i32) -> [f32; 4] {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
    }

    #[test]
    fn every_preset_live_preview_matches_committed_pixels_and_undo_at_every_depth() {
        for depth in [8, 16, 32] {
            for mode in ["rgb", "gray"] {
                for preset in SymmetryMode::ALL {
                    for command in ["paint.stroke", "paint.pencil"] {
                        let mut s = session(depth, mode);
                        s.execute("paint.setSymmetry", json!({"mode":preset.id(),"center":[40,30]})).unwrap();
                        let before = s.active().unwrap().doc.clone();
                        let params = json!({"points":[[20,10],[24,14]],"size":5,"hardness":1.0,"color":"#000000","opacity":0.5,"smoothing":0.0,"seed":7});
                        let live = LiveStroke::begin_with(&s, command, &params).unwrap();
                        let past = s.active().unwrap().history.past_len();
                        s.execute(command, params).unwrap();
                        let st = s.active().unwrap();
                        assert_eq!(st.history.past_len(), past + 1);
                        let id = st.active_layer.unwrap();
                        let actual = st.doc.layer(id).unwrap().surface().unwrap();
                        let preview = live.doc.layer(id).unwrap().surface().unwrap();
                        let original = StrokePoint::new(20.0, 10.0, 1.0);
                        let p = st.symmetry.as_ref().unwrap().preset().unwrap();
                        let positions = std::iter::once(original).chain(p.reflected_passes(&[original]).into_iter().map(|p| p[0]));
                        for p in positions {
                            assert!(actual.rgba(p.x.round() as i32, p.y.round() as i32)[0] < 0.9, "{depth} {mode} {preset:?}");
                        }
                        for y in 0..61 {
                            for x in 0..81 {
                                let a = actual.rgba(x, y);
                                let b = preview.rgba(x, y);
                                assert!((0..4).all(|i| (a[i] - b[i]).abs() < 0.02));
                            }
                        }
                        let after = st.doc.clone();
                        assert!(s.undo());
                        assert_eq!(s.active().unwrap().doc, before);
                        assert!(s.redo());
                        assert_eq!(s.active().unwrap().doc, after);
                    }
                }
            }
        }
    }

    #[test]
    fn dual_centre_overlap_has_one_opacity_ceiling_and_eraser_uses_the_same_copies() {
        for depth in [8, 16, 32] {
            let mut plain = session(depth, "rgb");
            let mut symmetric = session(depth, "rgb");
            symmetric.execute("paint.setSymmetry", json!({"mode":"dual","center":[40,30]})).unwrap();
            let params = json!({"points":[[40,30]],"size":9,"hardness":1.0,"opacity":0.5,"color":"#000000","smoothing":0.0});
            plain.execute("paint.stroke", params.clone()).unwrap();
            symmetric.execute("paint.stroke", params).unwrap();
            let a = pixel(&plain, 40, 30);
            let b = pixel(&symmetric, 40, 30);
            assert!((0..4).all(|i| (a[i] - b[i]).abs() < 0.02));
            symmetric.execute("layer.new.layer", json!({})).unwrap();
            symmetric.execute("edit.fill", json!({"color":"#000000"})).unwrap();
            let params = json!({"points":[[20,10],[25,15]],"size":7,"hardness":1.0,"erase":true,"smoothing":0.0});
            let mut live = LiveStroke::begin(&symmetric, &json!({"points":[[20,10]],"size":7,"hardness":1.0,"erase":true,"smoothing":0.0})).unwrap();
            live.push(&[StrokePoint::new(25.0, 15.0, 1.0)]).unwrap();
            symmetric.execute("paint.stroke", params).unwrap();
            let st = symmetric.active().unwrap();
            let id = st.active_layer.unwrap();
            for (x, y) in [(20, 10), (60, 10), (20, 50), (60, 50)] {
                let a = pixel(&symmetric, x, y);
                let b = live.doc.layer(id).unwrap().surface().unwrap().rgba(x, y);
                assert!(a[3] < 0.2);
                assert!((0..4).all(|i| (a[i] - b[i]).abs() < 0.02));
            }
        }
    }

    #[test]
    fn selection_transparency_lock_and_mask_targets_apply_to_all_passes() {
        let mut s = session(8, "rgb");
        s.execute("paint.setSymmetry", json!({"mode":"dual","center":[40,30]})).unwrap();
        s.execute("select.rect", json!({"x":0,"y":0,"width":40,"height":30})).unwrap();
        s.execute("paint.stroke", json!({"points":[[20,10]],"size":5,"hardness":1.0,"color":"#000000"})).unwrap();
        assert!(pixel(&s, 20, 10)[0] < 0.2);
        assert!(pixel(&s, 60, 10)[0] > 0.9);
        s.execute("select.deselect", json!({})).unwrap();
        s.edit("Mask fixture", |doc, active| {
            let mut layer = Layer::raster("Mask", doc.pixel_format());
            layer.mask = Some(LayerMask::reveal_all());
            layer.locks.transparency = true;
            *active = Some(layer.id);
            doc.layers.push(layer);
            Ok(())
        })
        .unwrap();
        s.execute("paint.stroke", json!({"points":[[20,10]],"size":5,"color":"#000000","target":"mask"})).unwrap();
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        for (x, y) in [(20, 10), (60, 10), (20, 50), (60, 50)] {
            assert!(l.mask.as_ref().unwrap().surface.rgba(x, y)[0] < 0.8);
        }
        assert!(l.surface().unwrap().content_bounds().is_empty());
        s.execute("paint.stroke", json!({"points":[[20,10]],"size":5,"color":"#ff0000"})).unwrap();
        assert!(s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().content_bounds().is_empty());
    }

    #[test]
    fn streaming_across_dual_axes_keeps_one_opacity_and_matches_commit() {
        let points = [[40.0, 30.0], [40.0, 20.0], [35.0, 20.0], [45.0, 40.0]];
        for depth in [8, 16, 32] {
            let mut s = session(depth, "rgb");
            s.execute("paint.setSymmetry", json!({"mode":"dual","center":[40,30]})).unwrap();
            let params = json!({"points":[points[0]],"size":9,"hardness":1.0,"opacity":0.5,"color":"#000000","smoothing":0.0});
            let mut live = LiveStroke::begin(&s, &params).unwrap();
            for [x, y] in points.into_iter().skip(1) {
                live.push(&[StrokePoint::new(x, y, 1.0)]).unwrap();
            }
            let mut params = params;
            params["points"] = json!(points);
            s.execute("paint.stroke", params).unwrap();
            let st = s.active().unwrap();
            let id = st.active_layer.unwrap();
            for y in 0..61 {
                for x in 0..81 {
                    let a = pixel(&s, x, y);
                    let b = live.doc.layer(id).unwrap().surface().unwrap().rgba(x, y);
                    assert!(a[0] >= 0.49, "opacity applied more than once at {x},{y}");
                    assert!((0..4).all(|i| (a[i] - b[i]).abs() < 0.02), "preview differs at {x},{y}");
                }
            }
        }
    }
}

mod tests {
    use photocraft_engine::Session;
    use photocraft_paint::StrokePoint;
    use serde_json::json;

    #[test]
    fn symmetry_paints_mirrored_pixels_and_disable_stops_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
        s.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
        let params = json!({"points": [[20, 20], [30, 30]], "size": 5, "hardness": 1.0, "color": "#ff0000", "smoothing": 0.0});
        let mut live = photocraft_engine::brush_cmds::LiveStroke::begin(
            &s,
            &json!({"points": [[20, 20]], "size": 5, "hardness": 1.0, "color": "#ff0000", "smoothing": 0.0}),
        )
        .unwrap();
        live.push(&[StrokePoint::new(30.0, 30.0, 1.0)]).unwrap();
        s.execute("paint.stroke", params).unwrap();
        let committed = &s.active().unwrap().doc;
        let id = s.active().unwrap().active_layer.unwrap();
        let surface = committed.layer(id).unwrap().surface().unwrap();
        assert!(surface.rgba(20, 20)[1] < 0.2);
        assert!(surface.rgba(60, 20)[1] < 0.2);
        let preview = live.doc.layer(id).unwrap().surface().unwrap();
        for (x, y) in [(20, 20), (60, 20), (30, 30), (50, 30), (40, 20)] {
            let expected = surface.rgba(x, y);
            let actual = preview.rgba(x, y);
            assert!((0..4).all(|i| (expected[i] - actual[i]).abs() < 0.02), "preview/commit at ({x},{y}): {actual:?} vs {expected:?}");
        }
        s.execute("paint.symmetryDisable", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[15, 35]], "size": 5, "hardness": 1.0, "color": "#0000ff"})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(65, 35)[0] > 0.8, "white remains without a mirrored blue stroke");
    }

    #[test]
    fn stroke_on_axis_is_not_painted_twice_at_half_opacity() {
        let mut plain = Session::new();
        plain.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        let mut symmetric = Session::new();
        symmetric.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        symmetric.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
        symmetric.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
        let params = json!({"points": [[40, 15], [40, 40]], "size": 8, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0});
        let mut preview = photocraft_engine::brush_cmds::LiveStroke::begin(
            &symmetric,
            &json!({"points": [[40, 15]], "size": 8, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0}),
        )
        .unwrap();
        preview.push(&[StrokePoint::new(40.0, 40.0, 1.0)]).unwrap();
        plain.execute("paint.stroke", params.clone()).unwrap();
        symmetric.execute("paint.stroke", params).unwrap();
        let pixels = |session: &Session| {
            let st = session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(40, 28)
        };
        let expected = pixels(&plain);
        let actual = pixels(&symmetric);
        let preview_pixel = {
            let st = symmetric.active().unwrap();
            preview.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(40, 28)
        };
        assert!(expected[0] > 0.2 && expected[0] < 0.8, "half opacity should produce a gray stroke: {expected:?}");
        assert!((0..4).all(|i| (actual[i] - expected[i]).abs() < 0.02), "symmetry must not darken on-axis paint: {actual:?} vs {expected:?}");
        assert!((0..4).all(|i| (preview_pixel[i] - actual[i]).abs() < 0.02), "preview must match commit: {preview_pixel:?} vs {actual:?}");
    }

    #[test]
    fn mixed_axis_and_off_axis_stroke_unions_coverage_once() {
        let mut plain = Session::new();
        plain.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        let mut symmetric = Session::new();
        symmetric.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        symmetric.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 0], [40, 60]]}]}})).unwrap();
        symmetric.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
        let params = json!({"points": [[40, 10], [40, 25], [30, 40]], "size": 7, "hardness": 1.0, "opacity": 0.5, "color": "#000000", "smoothing": 0.0});
        let preview = photocraft_engine::brush_cmds::LiveStroke::begin(&symmetric, &params).unwrap();
        plain.execute("paint.stroke", params.clone()).unwrap();
        symmetric.execute("paint.stroke", params).unwrap();
        let pixel = |session: &Session, x, y| {
            let st = session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
        };
        let axis_plain = pixel(&plain, 40, 18);
        let axis_symmetric = pixel(&symmetric, 40, 18);
        assert!(
            (0..4).all(|i| (axis_plain[i] - axis_symmetric[i]).abs() < 0.02),
            "shared axis must have one opacity ceiling: {axis_plain:?} vs {axis_symmetric:?}"
        );
        assert!(pixel(&symmetric, 50, 40)[0] < 0.9, "the off-axis part must be mirrored");
        let st = symmetric.active().unwrap();
        let preview_surface = preview.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        for (x, y) in [(40, 18), (30, 40), (50, 40)] {
            let expected = pixel(&symmetric, x, y);
            let actual = preview_surface.rgba(x, y);
            assert!((0..4).all(|i| (expected[i] - actual[i]).abs() < 0.02), "preview/commit at ({x},{y}): {actual:?} vs {expected:?}");
        }
    }
}
