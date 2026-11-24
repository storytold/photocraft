//! Tests that run paint commands, moved from the engine-core `image_cmds` tests: 13 test(s).

mod canvas_size_center_offset_tests {
    use serde_json::{Value, json};

    use photocraft_engine::Session;

    /// The issue's document: 400 x 300 px with a 50 x 50 layer at 175,125.
    fn doc() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 175, "y": 125, "width": 50, "height": 50})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    /// Runs Canvas Size on a fresh document; returns the reported offset and the layer's new origin.
    fn run_canvas_size(p: Value) -> (Value, (i32, i32)) {
        let mut s = doc();
        let r = s.execute("image.canvasSize", p).unwrap();
        let b = s.active().unwrap().doc.layers[1].surface().unwrap().content_bounds();
        (r["offset"].clone(), (b.x0, b.y0))
    }

    #[test]
    fn an_odd_change_with_the_centre_anchor_truncates_the_half_pixel() {
        for (w, h, off, at) in
            [(401, 301, [0, 0], (175, 125)), (403, 303, [1, 1], (176, 126)), (399, 299, [0, 0], (175, 125)), (397, 297, [-1, -1], (174, 124))]
        {
            let p = json!({"width": w, "height": h, "anchor": "center"});
            assert_eq!(run_canvas_size(p), (json!(off), at), "{w} x {h}");
        }
    }

    #[test]
    fn shrinking_to_one_pixel_moves_the_layers_by_minus_199_minus_149() {
        for p in [
            json!({"width": 1, "height": 1}),
            json!({"width": 0, "height": 0}),
            json!({"width": -5, "height": -5}),
            json!({"width": -400, "height": -300, "relative": true}),
        ] {
            assert_eq!(run_canvas_size(p.clone()), (json!([-199, -149]), (-24, -24)), "{p}");
        }
    }

    #[test]
    fn the_middle_edge_anchors_truncate_on_their_centred_axis() {
        for (anchor, grow, shrink) in [("top", [1, 0], [-1, 0]), ("bottom", [1, 3], [-1, -3]), ("left", [0, 1], [0, -1]), ("right", [3, 1], [-3, -1])] {
            let (off, _) = run_canvas_size(json!({"width": 403, "height": 303, "anchor": anchor}));
            assert_eq!(off, json!(grow), "{anchor} to 403 x 303");
            let (off, _) = run_canvas_size(json!({"width": 397, "height": 297, "anchor": anchor}));
            assert_eq!(off, json!(shrink), "{anchor} to 397 x 297");
        }
    }

    #[test]
    fn even_changes_and_the_corner_anchors_keep_their_offsets() {
        let (off, at) = run_canvas_size(json!({"width": 404, "height": 302, "anchor": "center"}));
        assert_eq!((off, at), (json!([2, 1]), (177, 126)));
        let (off, _) = run_canvas_size(json!({"width": 396, "height": 298, "anchor": "center"}));
        assert_eq!(off, json!([-2, -1]));
        for (anchor, even) in [("top", [2, 0]), ("bottom", [2, 2]), ("left", [0, 1]), ("right", [4, 1])] {
            let (off, _) = run_canvas_size(json!({"width": 404, "height": 302, "anchor": anchor}));
            assert_eq!(off, json!(even), "{anchor} to 404 x 302");
        }
        for (anchor, grow, shrink) in
            [("topLeft", [0, 0], [0, 0]), ("topRight", [3, 0], [-3, 0]), ("bottomLeft", [0, 3], [0, -3]), ("bottomRight", [3, 3], [-3, -3])]
        {
            let (off, _) = run_canvas_size(json!({"width": 403, "height": 303, "anchor": anchor}));
            assert_eq!(off, json!(grow), "{anchor} to 403 x 303");
            let (off, _) = run_canvas_size(json!({"width": 397, "height": 297, "anchor": anchor}));
            assert_eq!(off, json!(shrink), "{anchor} to 397 x 297");
        }
    }
}

mod tests {
    use photocraft_doc::{Document, Size};
    use photocraft_engine::Session;
    use photocraft_geom::Rect;
    use serde_json::json;

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    /// #6019: Image Size on an Indexed Color document makes no colours the colour table doesn't have.
    #[test]
    fn image_size_resamples_indexed_color_with_nearest_neighbor() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 1, "y": 0, "width": 1, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("image.mode.indexedColor", json!({})).unwrap();
        for resample in ["bicubic", "automatic", "bilinear", "lanczos", "preserveDetails"] {
            s.execute("image.imageSize", json!({"width": 8, "resample": resample})).unwrap();
            let st = s.active().unwrap();
            let surf = st.doc.layers[0].surface().unwrap();
            for x in 0..8 {
                let px = surf.pixel(x, 0);
                assert!(px[..3] == [1.0, 0.0, 0.0] || px[..3] == [0.0, 0.0, 1.0], "{resample}: no blended colour at x {x}: {px:?}");
            }
            s.execute("history.undo", json!({})).unwrap();
        }
    }

    /// #1114: nearest neighbor reduction keeps source values without averaging.
    #[test]
    fn nearest_neighbor_reduction_keeps_source_values() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                if (x + y) % 2 == 0 {
                    s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
                    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
                }
            }
        }
        s.execute("select.deselect", json!({})).unwrap();
        let st = s.active().unwrap();
        let before = st.doc.layers[0].surface().unwrap().rgba(0, 0);
        assert!(before[0] < 0.01, "setup: (0,0) should be black, got {before:?}");
        s.execute("image.imageSize", json!({"width": 4, "height": 4, "resample": "nearest"})).unwrap();
        let st = s.active().unwrap();
        let surf = st.doc.layers[0].surface().unwrap();
        for (x, y) in [(0, 0), (1, 1), (2, 3)] {
            let v = surf.rgba(x, y)[0];
            assert!(v.is_finite() && !(0.01..=0.99).contains(&v), "nearest produced an averaged value {v} at ({x},{y})");
        }
    }

    #[test]
    fn crop_extension_fills_the_background_with_the_background_colour() {
        let mut shrink = Session::new();
        shrink.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        shrink.execute("image.crop", json!({"x": 2, "y": 0, "width": 12, "height": 16})).unwrap();
        let layer = &doc(&shrink).layers[0];
        assert_eq!(layer.name, "Background", "a shrink-only crop keeps the Background");
        assert!(layer.locks.transparency && layer.locks.position);

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        s.tools.background = [0.0, 1.0, 0.0, 1.0];
        s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(layer.name, "Background", "an extending crop keeps the Background (#3081)");
        assert!(layer.locks.transparency && layer.locks.position);
        assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 1.0, 0.0, 1.0], "the added border takes the background colour");
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0], "the original pixels stay");

        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 0.0, 1.0, 1.0], "Fill paints the extended border");
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [0.0, 0.0, 1.0, 1.0]);

        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let layer = &doc(&s).layers[0];
        assert_eq!(doc(&s).size, Size::new(16, 16));
        assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(0, 0, 16, 16), "undo removes the filled border");

        // A normal layer gets transparent pixels in the added area.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
        assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(1, 8), [0.0; 4]);
    }

    /// A document (`mode`, `depth`) with a white Background and a layer holding a red pixel at
    /// (12, 8) and a blue one at (30, 3).
    fn marked(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": mode, "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        for (x, y, c) in [(12, 8, "#ff0000"), (30, 3, "#0000ff")] {
            s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn layer_rgba(s: &Session, layer: usize, x: i32, y: i32) -> [f32; 4] {
        doc(s).layers[layer].surface().unwrap().rgba(x, y)
    }

    /// #1792: a frame turned 90° clockwise crops what Image Rotation › Arbitrary 90° CCW and a
    /// plain crop give, in one undo step that restores the document, at 8, 16 and 32 bits.
    #[test]
    fn crop_at_90_degrees_is_rotate_then_crop_in_one_step() {
        for depth in [8, 16, 32] {
            let mut s = marked("rgb", depth);
            let before = doc(&s).clone();
            let past = s.active().unwrap().history.past_len();
            // The 10 × 20 frame at (10, 0), turned 90° about (15, 10), covers x 5..25, y 5..15.
            let r = s.execute("image.crop", json!({"x": 10, "y": 0, "width": 10, "height": 20, "angle": 90})).unwrap();
            assert_eq!(r["angle"], json!(90.0), "{depth}");
            assert_eq!(doc(&s).size, Size::new(10, 20), "{depth}");
            assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{depth}: one undo step");
            // (12, 8) is 2.5 left of and 1.5 above the centre: in the upright frame, 1.5 left of
            // and 2.5 below its centre (5, 10).
            let red = layer_rgba(&s, 1, 3, 12);
            assert!(red[0] > 0.9 && red[1] < 0.1 && red[3] > 0.9, "{depth}: {red:?}");
            let mut reference = marked("rgb", depth);
            reference.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "ccw"})).unwrap();
            reference.execute("image.crop", json!({"x": 5, "y": 15, "width": 10, "height": 20})).unwrap();
            for l in 0..2 {
                for y in 0..20 {
                    for x in 0..10 {
                        let (a, b) = (layer_rgba(&s, l, x, y), layer_rgba(&reference, l, x, y));
                        // A fully transparent pixel's colour is invisible, and the turned crop
                        // doesn't turn the pixels it deletes, which the warp spreads colour from.
                        let n = if b[3] > 1e-3 { 4 } else { 1 };
                        assert!(a.iter().rev().zip(b.iter().rev()).take(n).all(|(p, q)| (p - q).abs() < 1e-3), "{depth} layer {l} ({x}, {y}): {a:?} vs {b:?}");
                    }
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            let d = doc(&s);
            assert_eq!(d.size, before.size, "{depth}");
            assert_eq!(d.depth, before.depth);
            for y in 0..20 {
                for x in 0..40 {
                    assert_eq!(layer_rgba(&s, 1, x, y), before.layers[1].surface().unwrap().rgba(x, y), "{depth} undo at ({x}, {y})");
                }
            }
        }
    }

    /// A slightly turned frame keeps its own size, the pixel under its centre and the Background's
    /// colour; the turn's sign matters (a counter-clockwise frame is a negative angle).
    #[test]
    fn crop_at_a_small_angle_keeps_the_frame_size_in_every_mode() {
        for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 16), ("cmyk", 8), ("lab", 16)] {
            for angle in [5.0, -7.5, 365.0] {
                let mut s = marked(mode, depth);
                s.execute("image.crop", json!({"x": 2, "y": 1, "width": 21, "height": 15, "angle": angle})).unwrap();
                let d = doc(&s);
                assert_eq!(d.size, Size::new(21, 15), "{mode} {depth} {angle}");
                assert_eq!(d.depth.bytes() * 8, depth as usize, "{mode} {depth}");
                assert_eq!(format!("{:?}", d.mode).to_lowercase(), mode, "{mode} {depth}");
                assert_eq!(d.layers.len(), 2);
                // The frame's centre (12.5, 8.5) is the red pixel (the layer's only paint nearby);
                // it stays at the centre, its colour kept.
                let want = layer_rgba(&marked(mode, depth), 1, 12, 8);
                let red = layer_rgba(&s, 1, 10, 7);
                assert!(red[3] > 0.5 && (0..3).all(|c| (red[c] - want[c]).abs() < 0.2), "{mode} {depth} {angle}: {red:?} vs {want:?}");
                // The Background stays opaque white inside the canvas it came from.
                let bg = layer_rgba(&s, 0, 10, 7);
                assert!(bg.iter().all(|v| (v - 1.0).abs() < 0.02), "{mode} {depth} {angle}: {bg:?}");
            }
        }
    }

    /// Angle 0 (or a whole turn) is the plain crop: no resampling, the same document.
    #[test]
    fn crop_at_angle_zero_is_the_plain_crop() {
        let mut plain = marked("rgb", 16);
        plain.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10})).unwrap();
        for angle in [json!(0), json!(0.0), json!(360), json!(-720), json!(null)] {
            let mut s = marked("rgb", 16);
            let r = s.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10, "angle": angle})).unwrap();
            assert!(r.get("angle").is_none(), "{angle}: {r}");
            assert_eq!(doc(&s).size, Size::new(30, 10));
            for l in 0..2 {
                for y in 0..10 {
                    for x in 0..30 {
                        assert_eq!(layer_rgba(&s, l, x, y), layer_rgba(&plain, l, x, y), "{angle} layer {l} ({x}, {y})");
                    }
                }
            }
        }
    }

    /// Bad angles fail cleanly, before anything changes.
    #[test]
    fn crop_angle_rejects_bad_values_without_changing_the_document() {
        let bad = [
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "NaN"}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "12"}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": [1]}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": true}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": 1e308}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": -3601}),
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": f64::MAX}),
            // An angle needs the frame: a selection crop has none.
            json!({"angle": 10}),
            json!({"x": 0, "y": 0, "width": 0, "height": 10, "angle": 10}),
            json!({"x": 0, "y": 0, "width": 300_001, "height": 10, "angle": 10}),
            json!({"x": i32::MAX - 5, "y": 0, "width": 10, "height": 10, "angle": 10}),
        ];
        for p in bad {
            let mut s = marked("rgb", 8);
            s.execute("select.all", json!({})).unwrap();
            let before = doc(&s).clone();
            let past = s.active().unwrap().history.past_len();
            assert!(s.execute("image.crop", p.clone()).is_err(), "{p}");
            assert_eq!(doc(&s).size, before.size, "{p}");
            assert_eq!(s.active().unwrap().history.past_len(), past, "{p}");
        }
        // Far-off but representable frames turn and crop without panicking.
        for (x, y) in [(-1_000_000, 0), (0, 1_000_000), (i32::MIN / 4, i32::MAX / 4)] {
            let mut s = marked("rgb", 8);
            s.execute("image.crop", json!({"x": x, "y": y, "width": 4, "height": 3, "angle": 33.3})).unwrap();
            assert_eq!(doc(&s).size, Size::new(4, 3));
        }
    }

    /// #7722: a W x H crop of an Indexed Color document resamples like Image Size (#6019), with
    /// Nearest Neighbor, so no pixel gets a blend that isn't in the colour table.
    #[test]
    fn crop_target_resamples_indexed_color_with_nearest_neighbor() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 1, "y": 0, "width": 1, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("image.mode.indexedColor", json!({})).unwrap();
        for resample in [None, Some("bicubic"), Some("automatic"), Some("bilinear"), Some("lanczos")] {
            let mut p = json!({"x": 0, "y": 0, "width": 2, "height": 1, "targetWidth": 8, "targetHeight": 1});
            if let Some(r) = resample {
                p["resample"] = json!(r);
            }
            s.execute("image.crop", p).unwrap();
            let surf = doc(&s).layers[0].surface().unwrap();
            for x in 0..8 {
                let px = surf.pixel(x, 0);
                assert!(px[..3] == [1.0, 0.0, 0.0] || px[..3] == [0.0, 0.0, 1.0], "{resample:?}: no blended colour at x {x}: {px:?}");
            }
            s.execute("history.undo", json!({})).unwrap();
        }
    }
}

mod canvas_size_gray_tests {
    use serde_json::{Value, json};

    use photocraft_engine::Session;

    /// A 400 x 300 document at `depth` with a white Background and an empty layer "Top" above it.
    fn doc(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s
    }

    /// The Background's stored colour at pixel (x, y).
    fn background_value(s: &Session, x: i32, y: i32) -> [f32; 4] {
        s.active().unwrap().doc.background().unwrap().surface().unwrap().rgba(x, y)
    }

    const GROW: fn(&str) -> Value = |color| json!({"width": 500, "height": 400, "anchor": "topLeft", "extensionColor": color});

    /// A 32-bit document holds linear light: its Gray is the value an 8-bit grey 128 becomes when the
    /// document is converted to 32 bits (0.2159), not 128/255.
    #[test]
    fn gray_in_a_32_bit_document_is_linear_grey_128() {
        let mut converted = Session::new();
        converted.execute("file.new", json!({"width": 4, "height": 4, "depth": 8, "background": "white"})).unwrap();
        converted.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        converted.execute("image.mode.bits32", json!({})).unwrap();
        let want = background_value(&converted, 1, 1);
        assert!((want[0] - 0.2159).abs() < 0.002, "converted grey 128: {want:?}");
        let mut s = doc(32);
        s.execute("image.canvasSize", GROW("gray")).unwrap();
        let got = background_value(&s, 450, 350);
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-4, "channel {i}: {got:?} vs {want:?}");
        }
        assert_eq!(got[3], 1.0);
        assert_eq!(background_value(&s, 10, 10)[..3], [1.0, 1.0, 1.0], "the old Background keeps its pixels");
    }
}
