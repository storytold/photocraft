//! Tests that run paint commands, moved from the engine-core `layer_style` tests: 9 test(s).

mod inner_shadow_ranges_tests {

    use photocraft_engine::Session;

    use serde_json::json;

    /// #14940: in-range values keep today's pixels. The measured far-distance case (Size 0,
    /// Distance 500 on a 200 x 150 px layer) paints the whole layer with the shadow colour; these
    /// levels are the ones the compositor already gave, so the range change moves no pixel.
    #[test]
    fn an_inner_shadow_reaching_past_the_layer_paints_it_as_before() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.layerStyle.innerShadow", json!({"distance": 500.0, "size": 0.0})).unwrap();
        let d = s.active().unwrap();
        let px = photocraft_compose::render(&d.doc, d.doc.bounds()).into_px();
        let level = |x: usize, y: usize| px[y * 400 + x].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as i32);
        // Inside the layer: Multiply black at the factory 75 %, so the orange stays only at a quarter.
        for (x, y) in [(200, 150), (105, 80), (295, 220)] {
            assert_eq!(level(x, y), [64, 32, 0, 255], "inside the layer at ({x}, {y})");
        }
        // Outside it, nothing: the inner shadow never leaves the layer, and the canvas stays white.
        for (x, y) in [(200, 74), (97, 72), (5, 5)] {
            assert_eq!(level(x, y), [255, 255, 255, 255], "outside the layer at ({x}, {y})");
        }
    }
}

mod outer_glow_defaults_tests {

    use photocraft_engine::Session;

    use serde_json::json;

    /// The issue's test document: 400 x 300 px at 72 ppi, RGB 8 bit, a filled (64,64,64)
    /// Background and a pixel layer with an orange (255,128,0) rectangle at x 100..299, y 75..224.
    fn test_document() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "#404040"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    /// #14944 measured: OK on the factory values over the grey test document. Top edge at x 200,
    /// y 74 outwards to 68, and the corner pixel (97, 72), as grey levels; the reference
    /// application's are 121 104 91 80 72 66 64 and 69, within 2 levels.
    #[test]
    fn the_factory_outer_glow_paints_the_measured_edge() {
        let mut s = test_document();
        s.execute("layer.layerStyle.outerGlow", json!({})).unwrap();
        let d = s.active().unwrap();
        let px = photocraft_compose::render(&d.doc, d.doc.bounds()).into_px();
        let level = |x: usize, y: usize| (px[y * 400 + x][0] * 255.0).round() as i32;
        for (y, want) in [(74, 121), (73, 104), (72, 91), (71, 80), (70, 72), (69, 66), (68, 64)] {
            let got = level(200, y);
            assert!((got - want).abs() <= 2, "top edge at (200, {y}): {got}, the reference gives {want}");
        }
        let corner = level(97, 72);
        assert!((corner - 69).abs() <= 2, "corner (97, 72): {corner}, the reference gives 69");
    }
}

mod tests {
    use photocraft_doc::Effect;
    use photocraft_engine::{EngineError, Session};
    use serde_json::Value;

    use serde_json::json;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    }

    fn effects(s: &Session) -> Vec<Effect> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items.clone()
    }

    /// #1543: a Satin or Bevel & Emboss size or softness far past Photoshop's range rendered a
    /// blur kernel of 2^62 taps ("capacity overflow"), or ran for minutes at 1e6. The blur width
    /// is capped at the effect reach, so the document still renders, quickly.
    #[test]
    fn oversized_satin_and_bevel_sizes_stay_saveable_and_render() {
        for (cmd, p, accepted) in [
            ("satin", json!({"size": 1e308}), false),
            ("satin", json!({"size": 1e30}), true),
            ("satin", json!({"size": 1_000_000}), true),
            ("bevelEmboss", json!({"size": 1e308}), false),
            ("bevelEmboss", json!({"soften": 1e308}), false),
            ("bevelEmboss", json!({"size": 1e30, "soften": 1e30}), true),
            ("bevelEmboss", json!({"size": 1_000_000, "soften": 1_000_000}), true),
        ] {
            let mut s = session();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            let result = s.execute(&format!("layer.layerStyle.{cmd}"), p.clone());
            if accepted {
                result.unwrap();
            } else {
                assert!(matches!(result, Err(EngineError::BadParams { .. })));
            }
            photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
            let start = std::time::Instant::now();
            s.execute("document.pixel", json!({"x": 0, "y": 0})).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
            assert!(start.elapsed().as_secs() < 20, "{cmd} {p}: {:?}", start.elapsed());
        }
    }

    fn pixel(s: &mut Session) -> Vec<f32> {
        serde_json::from_value(s.execute("document.pixel", json!({"x": 3, "y": 3})).unwrap()).unwrap()
    }

    fn list_enabled(s: &Session) -> bool {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().effects.enabled
    }

    /// #3648: the Layers panel's "Effects" eye (`layer.setEffectsVisible`) must survive a Layer
    /// Style dialog OK: the dialog's whole-list replace re-creates the items but leaves the
    /// list's master switch alone.
    #[test]
    fn replacing_the_list_keeps_a_disabled_effect_list_disabled() {
        let mut s = session();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
        let shown = pixel(&mut s);
        s.execute("layer.setEffectsVisible", json!({"visible": false})).unwrap();
        let hidden = pixel(&mut s);
        assert_ne!(hidden, shown, "the eye really hides the style");
        let r = s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "colorOverlay", "params": {"color": "#00ff00"}}]})).unwrap();
        assert_eq!(r["effects"], 1, "the list itself is updated");
        assert!(!list_enabled(&s), "the whole-list switch survives the edit");
        assert!(effects(&s)[0].enabled(), "the effect's own state is updated");
        assert_eq!(pixel(&mut s), hidden, "a disabled list still draws nothing");
        s.execute("layer.setEffectsVisible", json!({"visible": true})).unwrap();
        assert_ne!(pixel(&mut s), hidden, "re-showing the list draws the edited effect");
    }

    /// #3648: adding or replacing one effect (`layer.layerStyle.<kind>`) also leaves the master
    /// switch as the user left it.
    #[test]
    fn set_effect_keeps_a_disabled_effect_list_disabled() {
        let mut s = session();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
        s.execute("layer.setEffectsVisible", json!({"visible": false})).unwrap();
        let hidden = pixel(&mut s);
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#00ff00"})).unwrap();
        assert!(!list_enabled(&s), "the whole-list switch survives the edit");
        assert!(effects(&s)[0].enabled(), "the effect's own state is updated");
        assert_eq!(pixel(&mut s), hidden, "a disabled list still draws nothing");
    }

    /// A layer that never had effects keeps the default: its first effect shows.
    #[test]
    fn a_first_effect_shows_on_a_layer_that_never_had_effects() {
        let mut s = session();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        let plain = pixel(&mut s);
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#00ff00"})).unwrap();
        assert!(list_enabled(&s), "a fresh effects list defaults to enabled");
        assert_ne!(pixel(&mut s), plain, "the first effect renders");
    }

    #[test]
    fn double_stroke_bands_render_and_survive_an_edit() {
        // The manga/typesetting case (#157): a 7 px white stroke under a 3 px black one.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 48, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000", "position": "outside"})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "position": "outside", "add": true})).unwrap();
        let px = |s: &mut Session, x: i32, y: i32| -> [f32; 4] {
            let mut v: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap()).unwrap();
            v.resize(4, 1.0);
            [v[0], v[1], v[2], v[3]]
        };
        // Bands from the square edge outward: 0..3 px black (the first stroke renders on top),
        // 3..7 px white, then the white background.
        assert!(px(&mut s, 24, 24)[0] < 0.1, "the square is black");
        assert!(px(&mut s, 24, 14)[0] < 0.1, "the first stroke covers the wider one");
        assert!(px(&mut s, 24, 11)[0] > 0.9, "the second stroke shows outside it");
        assert!(px(&mut s, 24, 5)[0] > 0.9, "background beyond both");
        // A dialog round trip (replace with the same snapshots) keeps both bands.
        let before = px(&mut s, 24, 14);
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let items = st.doc.layer(id).unwrap().effects.items.clone();
        let entries: Vec<Value> = items.iter().map(|fx| json!({"kind": "stroke", "fx": serde_json::to_value(fx).unwrap(), "params": {}})).collect();
        s.execute("layer.layerStyle.replace", json!({"layer": id.0, "effects": entries})).unwrap();
        assert_eq!(px(&mut s, 24, 14), before, "both strokes render identically after the round trip");
    }
}

mod satin_defaults_tests {
    use photocraft_engine::Session;

    use serde_json::json;

    /// The issue's test document: 400 x 300 px at 72 ppi, RGB 8 bit, a white Background and a pixel
    /// layer with an orange (255,128,0) rectangle at x 100..299, y 75..224.
    fn test_document() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "#ffffff"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    /// The flattened test document, as one row-major pixel buffer.
    fn flattened(s: &Session) -> Vec<[f32; 4]> {
        let d = s.active().unwrap();
        photocraft_compose::render(&d.doc, d.doc.bounds()).into_px()
    }

    /// One 8-bit level of the flattened test document: `channel` 0 is R, 1 G and 2 B.
    fn level(px: &[[f32; 4]], channel: usize, x: usize, y: usize) -> i32 {
        (px[y * 400 + x][channel] * 255.0).round() as i32
    }

    /// Compare an RGB triple with the reference application's, within 1 level.
    fn assert_rgb(px: &[[f32; 4]], x: usize, y: usize, want: [i32; 3]) {
        let got = [level(px, 0, x, y), level(px, 1, x, y), level(px, 2, x, y)];
        assert!(
            got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1),
            "({x}, {y}) = {}.{}.{}. the reference gives {}.{}.{}",
            got[0],
            got[1],
            got[2],
            want[0],
            want[1],
            want[2]
        );
    }

    /// #14977 measured: OK on the factory values over the white test document. The G channel down
    /// the centre column x 200 (every 5 px from y 75), the darkest point at y 149 and 150, the top
    /// edge and the corners, within 1 level of the reference application's.
    #[test]
    fn the_factory_satin_gives_the_measured_profile() {
        let mut s = test_document();
        s.execute("layer.layerStyle.satin", json!({})).unwrap();
        let px = flattened(&s);
        let column: [i32; 30] =
            [119, 118, 118, 116, 115, 112, 109, 105, 101, 96, 91, 85, 80, 74, 69, 65, 70, 76, 81, 86, 92, 97, 102, 106, 110, 113, 115, 117, 118, 119];
        for (i, want) in column.iter().copied().enumerate() {
            let y = 75 + 5 * i;
            let got = level(&px, 1, 200, y);
            assert!((got - want).abs() <= 1, "centre column at (200, {y}): G {got}, the reference gives {want}");
        }
        for y in [149, 150] {
            assert_eq!(level(&px, 1, 200, y), 65, "the darkest point at y {y}");
        }
        let top_edge: [i32; 20] = [92, 98, 104, 108, 112, 115, 117, 118, 119, 119, 119, 119, 119, 118, 117, 115, 112, 108, 103, 98];
        for (i, want) in top_edge.iter().copied().enumerate() {
            let x = 100 + 10 * i;
            let got = level(&px, 1, x, 75);
            assert!((got - want).abs() <= 1, "top edge at ({x}, 75): G {got}, the reference gives {want}");
        }
        for y in [75, 76, 77, 78, 79] {
            assert_rgb(&px, 200, y, [237, 119, 0]);
        }
        assert_rgb(&px, 100, 75, [183, 92, 0]);
        assert_rgb(&px, 100, 150, [128, 64, 0]);
        assert_rgb(&px, 200, 150, [129, 65, 0]);
    }

    /// #14977: the values themselves, passed as params, give the same picture the factory defaults
    /// now must. Guard test, passes before and after: it pins the profile to the effect's own params.
    #[test]
    fn the_measured_values_as_params_give_the_same_profile() {
        let mut s = test_document();
        s.execute("layer.layerStyle.satin", json!({"angle": 90, "distance": 50, "size": 80})).unwrap();
        let px = flattened(&s);
        let column: [i32; 30] =
            [119, 118, 118, 116, 115, 112, 109, 105, 101, 96, 91, 85, 80, 74, 69, 65, 70, 76, 81, 86, 92, 97, 102, 106, 110, 113, 115, 117, 118, 119];
        for (i, want) in column.iter().copied().enumerate() {
            let y = 75 + 5 * i;
            let got = level(&px, 1, 200, y);
            assert!((got - want).abs() <= 1, "centre column at (200, {y}): G {got}, the reference gives {want}");
        }
        assert_rgb(&px, 100, 75, [183, 92, 0]);
        assert_rgb(&px, 200, 150, [129, 65, 0]);
    }
}
