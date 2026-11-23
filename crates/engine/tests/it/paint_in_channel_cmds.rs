//! Tests that run paint commands, moved from the engine-core `channel_cmds` tests: 46 test(s).

mod apply_image_darker_lighter_tests {
    use photocraft_engine::{Result, Session};
    use serde_json::{Value, json};

    const W: u32 = 400;

    const H: u32 = 300;

    fn halves(s: &mut Session, left: &str, right: &str) {
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": W / 2, "height": H, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"color": left})).unwrap();
        s.execute("select.rect", json!({"x": W / 2, "y": 0, "width": W / 2, "height": H, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"color": right})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }

    /// The measured test documents: "target" (0) RGB 200/100/50 | 50/150/250 and "source" (1)
    /// 128/128/128 | 255/255/255, with "target" active.
    fn session(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        for (left, right) in [("#c86432", "#3296fa"), ("#808080", "#ffffff")] {
            s.execute("file.new", json!({"width": W, "height": H, "mode": mode, "depth": depth})).unwrap();
            halves(&mut s, left, right);
        }
        s.set_active(0);
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    fn assert_rgb(s: &Session, x: i32, want: [u8; 3], what: &str) {
        let p = px(s, x, 50);
        let ok = p[..3].iter().zip(want).all(|(v, w)| (v - f32::from(w) / 255.0).abs() < 0.003);
        assert!(ok, "{what} at ({x},50): {p:?}, want {want:?}");
    }

    fn apply(s: &mut Session, blending: &str) -> Result<Value> {
        s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": blending}))
    }

    #[test]
    fn darker_and_lighter_color_take_the_whole_pixel_by_luminosity() {
        for depth in [8, 16, 32] {
            let mut s = session("rgb", depth);
            apply(&mut s, "darkerColor").unwrap();
            assert_rgb(&s, 50, [200, 100, 50], &format!("Darker Color, {depth} bit"));
            assert_rgb(&s, 300, [50, 150, 250], &format!("Darker Color, {depth} bit"));
            s.undo();
            apply(&mut s, "lighterColor").unwrap();
            assert_rgb(&s, 50, [128, 128, 128], &format!("Lighter Color, {depth} bit"));
            assert_rgb(&s, 300, [255, 255, 255], &format!("Lighter Color, {depth} bit"));
        }
    }

    #[test]
    fn the_labels_parse_too() {
        let mut s = session("rgb", 8);
        apply(&mut s, "Darker Color").unwrap();
        assert_rgb(&s, 50, [200, 100, 50], "Darker Color");
        s.undo();
        apply(&mut s, "lighter_color").unwrap();
        assert_rgb(&s, 50, [128, 128, 128], "Lighter Color");
    }

    #[test]
    fn a_single_colour_channel_takes_darken_and_lighten() {
        let mut s = session("rgb", 8);
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        // Red 200 vs 128: Darken gives 128 (Darker Color on the whole pixel would keep 200).
        apply(&mut s, "darkerColor").unwrap();
        assert_rgb(&s, 50, [128, 100, 50], "Darker Color onto Red");
        assert_rgb(&s, 300, [50, 150, 250], "Darker Color onto Red");
        s.undo();
        s.execute("channel.target", json!({"channel": "blue"})).unwrap();
        apply(&mut s, "lighterColor").unwrap();
        assert_rgb(&s, 50, [200, 100, 128], "Lighter Color onto Blue");
        assert_rgb(&s, 300, [50, 150, 255], "Lighter Color onto Blue");
    }

    #[test]
    fn an_alpha_channel_and_a_layer_mask_take_darken_and_lighten() {
        let mut s = session("rgb", 8);
        let a = s.execute("channel.new", json!({"fill": "white"})).unwrap()["channel"].as_u64().unwrap() as usize;
        // Into an alpha channel: white onto the source's gray (Darken) is the gray.
        s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "darkerColor", "target": {"channel": a}})).unwrap();
        let alpha = |s: &Session, x| s.active().unwrap().doc.channels[a].surface.sample_channel(x, 50, 0);
        assert!((alpha(&s, 50) - 128.0 / 255.0).abs() < 0.003 && (alpha(&s, 300) - 1.0).abs() < 0.003);
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "mask"})).unwrap();
        // Into a black layer mask: Lighten takes the source's gray and white.
        s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "lighterColor"})).unwrap();
        let d = s.active().unwrap();
        let m = &d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
        assert!((m.sample_channel(50, 50, 0) - 128.0 / 255.0).abs() < 0.003, "{}", m.sample_channel(50, 50, 0));
        assert!((m.sample_channel(300, 50, 0) - 1.0).abs() < 0.003);
    }

    #[test]
    fn a_grayscale_layer_takes_darken_and_lighten() {
        let mut s = Session::new();
        for color in ["#c8c8c8", "#808080"] {
            s.execute("file.new", json!({"width": 40, "height": 20, "mode": "grayscale"})).unwrap();
            s.execute("edit.fill", json!({"color": color})).unwrap();
        }
        s.set_active(0);
        apply(&mut s, "darkerColor").unwrap();
        assert!((px(&s, 5, 5)[0] - 128.0 / 255.0).abs() < 0.003, "{:?}", px(&s, 5, 5));
        s.undo();
        apply(&mut s, "lighterColor").unwrap();
        assert!((px(&s, 5, 5)[0] - 200.0 / 255.0).abs() < 0.003, "{:?}", px(&s, 5, 5));
    }

    #[test]
    fn other_modes_and_refusals_are_unchanged() {
        let mut s = session("rgb", 8);
        // Darken still mixes the channels: min per channel.
        apply(&mut s, "darken").unwrap();
        assert_rgb(&s, 50, [128, 100, 50], "Darken");
        s.undo();
        for bad in ["hue", "luminosity", "dissolve", "darkerColour", ""] {
            assert!(apply(&mut s, bad).is_err(), "{bad} is refused");
        }
        assert_rgb(&s, 50, [200, 100, 50], "nothing changed");
        // Calculations keeps refusing them.
        for blending in ["darkerColor", "lighterColor"] {
            let e = s.execute("image.calculations", json!({"source1": {}, "source2": {}, "blending": blending})).unwrap_err();
            assert!(e.to_string().contains("unsupported blending"), "{e}");
        }
        assert!(s.active().unwrap().doc.channels.is_empty());
    }

    #[test]
    fn a_cmyk_composite_target_is_refused_and_left_alone() {
        let mut s = session("cmyk", 8);
        let before = px(&s, 50, 50);
        assert!(apply(&mut s, "darkerColor").is_err());
        assert_eq!(px(&s, 50, 50), before);
    }
}

mod apply_image_lock_transparency_tests {
    use photocraft_doc::Layer;
    use photocraft_engine::Session;
    use serde_json::{Value, json};

    /// A 40 x 20 document whose layer "Top" is empty except a half-transparent green 10 x 10 square
    /// at (10,5), and a second document "source" filled with 50% gray.
    fn session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 5, "width": 10, "height": 10, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00", "opacity": 50})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "source"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
        s.set_active(0);
        s
    }

    fn layer(s: &Session) -> &Layer {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap()
    }

    fn pixels(s: &Session) -> Vec<f32> {
        let d = s.active().unwrap();
        layer(s).surface().unwrap().read_region(d.doc.bounds())
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        layer(s).surface().unwrap().pixel(x, y)
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// The layer's pixels after Apply Image with `extra` params, unlocked and with Preserve transparency.
    fn preserved(depth: u32, extra: &Value) -> Vec<f32> {
        let mut s = session(depth);
        let mut p = json!({"source": {"document": 1}, "blending": "normal", "preserveTransparency": true});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("image.applyImage", p).unwrap();
        pixels(&s)
    }

    #[test]
    fn transparency_lock_acts_as_preserve_transparency() {
        for depth in [8, 16, 32] {
            for extra in [json!({}), json!({"blending": "multiply"}), json!({"opacity": 50})] {
                let want = preserved(depth, &extra);
                let mut s = session(depth);
                s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
                let mut p = json!({"source": {"document": 1}, "blending": "normal"});
                for (k, v) in extra.as_object().unwrap() {
                    p[k] = v.clone();
                }
                s.execute("image.applyImage", p).unwrap();
                assert!(close(&pixels(&s), &want), "{depth} bit {extra}: the lock must keep the layer's transparency");
                assert_eq!(px(&s, 30, 2), vec![0.0; 4], "{depth} bit {extra}: a transparent pixel stays transparent");
                assert!((px(&s, 15, 10)[3] - px(&session(depth), 15, 10)[3]).abs() < 1e-5, "{depth} bit {extra}: the square keeps its alpha");
            }
        }
    }

    #[test]
    fn transparency_lock_inside_a_selection_acts_as_preserve_transparency() {
        let sel = |s: &mut Session| s.execute("select.rect", json!({"x": 0, "y": 0, "width": 15, "height": 20, "mode": "replace"})).unwrap();
        let mut want = session(8);
        sel(&mut want);
        want.execute("image.applyImage", json!({"source": {"document": 1}, "preserveTransparency": true})).unwrap();
        let mut s = session(8);
        s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
        sel(&mut s);
        s.execute("image.applyImage", json!({"source": {"document": 1}})).unwrap();
        assert!(close(&pixels(&s), &pixels(&want)));
        assert_eq!(px(&s, 2, 2), vec![0.0; 4]);
    }

    #[test]
    fn lock_stays_on_and_one_history_step_is_added() {
        let mut s = session(8);
        s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("image.applyImage", json!({"source": {"document": 1}})).unwrap();
        assert!(layer(&s).locks.transparency, "the lock stays on");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one Apply Image step");
        s.undo();
        assert!(close(&pixels(&s), &pixels(&session(8))), "undo restores the layer");
    }

    #[test]
    fn unlocked_layer_keeps_source_over() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "normal"})).unwrap();
            let g = 128.0 / 255.0;
            let p = px(&s, 30, 2);
            assert!(p[..3].iter().all(|v| (v - g).abs() < 0.003) && (p[3] - 1.0).abs() < 1e-5, "{depth} bit: {p:?}");
        }
    }
}

mod apply_image_locked_layer_tests {
    use photocraft_doc::Layer;
    use photocraft_engine::Session;
    use serde_json::{Value, json};

    /// The spec's test document "target" (400 x 300): layer "Top" is empty except a 100 x 100 square
    /// at (150,100) of 0/255/0 at alpha 128. Document "source" is 128/128/128 on the left half and
    /// white on the right half.
    fn session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "name": "target"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s.execute("select.rect", json!({"x": 150, "y": 100, "width": 100, "height": 100, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00", "opacity": 128.0 / 255.0 * 100.0})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "name": "source"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.set_active(0);
        s
    }

    fn apply() -> Value {
        json!({"source": {"document": 1}, "blending": "normal", "opacity": 100})
    }

    fn layer(s: &Session) -> &Layer {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap()
    }

    fn pixels(s: &Session) -> Vec<f32> {
        let d = s.active().unwrap();
        layer(s).surface().unwrap().read_region(d.doc.bounds())
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        layer(s).surface().unwrap().pixel(x, y)
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    fn lock(s: &mut Session, locks: &Value) {
        s.execute("layer.setProps", json!({"locks": locks})).unwrap();
    }

    const LOCKS: [&str; 2] = [r#"{"all": true}"#, r#"{"pixels": true}"#];

    fn locks() -> impl Iterator<Item = Value> {
        LOCKS.iter().map(|l| serde_json::from_str(l).unwrap())
    }

    #[test]
    fn a_locked_layer_takes_the_image_in_one_step_and_keeps_its_transparency() {
        for depth in [8, 16, 32] {
            for locks in locks() {
                let mut s = session(depth);
                lock(&mut s, &locks);
                let before = px(&s, 175, 125);
                let steps = s.active().unwrap().history.past_len();
                s.execute("image.applyImage", apply()).unwrap();
                let d = s.active().unwrap();
                assert_eq!(d.history.past_len(), steps + 1, "{depth} bit {locks}: one history step");
                assert_eq!(d.history.undo_label(), Some("Apply Image"), "{depth} bit {locks}");
                assert!(!close(&px(&s, 175, 125), &before), "{depth} bit {locks}: (175,125) changes");
                assert_eq!(px(&s, 50, 50), vec![0.0; 4], "{depth} bit {locks}: (50,50) stays transparent");
                assert!((px(&s, 175, 125)[3] - before[3]).abs() < 1e-5, "{depth} bit {locks}: the square keeps its alpha");
            }
        }
    }

    #[test]
    fn a_locked_layer_gets_the_preserve_transparency_result() {
        for depth in [8, 16, 32] {
            for extra in [json!({}), json!({"blending": "multiply"}), json!({"opacity": 50})] {
                let with = |p: Value| {
                    let mut p = p;
                    for (k, v) in extra.as_object().unwrap() {
                        p[k] = v.clone();
                    }
                    p
                };
                let mut want = session(depth);
                let mut p = with(apply());
                p["preserveTransparency"] = json!(true);
                want.execute("image.applyImage", p).unwrap();
                for locks in locks() {
                    let mut s = session(depth);
                    lock(&mut s, &locks);
                    s.execute("image.applyImage", with(apply())).unwrap();
                    assert!(close(&pixels(&s), &pixels(&want)), "{depth} bit {locks} {extra}");
                }
            }
        }
    }

    #[test]
    fn the_locks_stay_and_other_commands_keep_refusing() {
        for locks in locks() {
            let mut s = session(8);
            lock(&mut s, &locks);
            let set = layer(&s).locks;
            s.execute("image.applyImage", apply()).unwrap();
            assert_eq!(layer(&s).locks, set, "{locks}: the lock flags are unchanged");
            let after = pixels(&s);
            let steps = s.active().unwrap().history.past_len();
            let fill = s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap_err().to_string();
            assert!(fill.contains("is locked"), "{locks}: Fill refuses: {fill}");
            let stroke = s.execute("paint.stroke", json!({"points": [[160, 110], [240, 190]], "size": 20})).unwrap_err().to_string();
            assert!(stroke.contains("is locked"), "{locks}: a brush stroke refuses: {stroke}");
            assert!(close(&pixels(&s), &after), "{locks}: nothing else painted");
            assert_eq!(s.active().unwrap().history.past_len(), steps, "{locks}: no more steps");
            s.undo();
            assert!(close(&pixels(&s), &pixels(&session(8))), "{locks}: undo restores the layer");
        }
    }

    #[test]
    fn a_lock_from_a_group_still_refuses() {
        // Not measured in the reference application: keeps today's refusal, also when the layer is
        // locked itself as well.
        for own in [None, Some(json!({"pixels": true})), Some(json!({"all": true}))] {
            let mut s = session(8);
            let top = s.active().unwrap().active_layer.unwrap();
            if let Some(own) = &own {
                lock(&mut s, own);
            }
            s.execute("layer.new.groupFromLayers", json!({})).unwrap();
            lock(&mut s, &json!({"all": true}));
            s.set_active_layer(Some(top));
            let before = pixels(&s);
            let err = s.execute("image.applyImage", apply()).unwrap_err().to_string();
            assert!(err.contains("is locked"), "{own:?}: {err}");
            assert!(close(&pixels(&s), &before), "{own:?}");
        }
    }

    #[test]
    fn an_unlocked_layer_keeps_source_over() {
        let mut s = session(8);
        s.execute("image.applyImage", apply()).unwrap();
        let p = px(&s, 50, 50);
        let g = 128.0 / 255.0;
        assert!(p[..3].iter().all(|v| (v - g).abs() < 0.003) && (p[3] - 1.0).abs() < 1e-5, "{p:?}");
    }
}

mod apply_image_preserve_tests {

    use photocraft_engine::Session;

    use serde_json::{Value, json};

    /// Target "target" (document 0): white Background, layer "Top" transparent except a 0/255/0
    /// square at alpha 128 over (10,4)-(19,13). Source "source" (document 1): 128/128/128.
    fn session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "target"})).unwrap();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "name": "source"})).unwrap();
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s.set_active(0);
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 4, "width": 10, "height": 10, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00", "opacity": 50})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    fn apply(s: &mut Session, extra: Value) {
        let mut p = json!({"source": {"document": 1, "layer": "merged", "channel": "composite"}});
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        s.execute("image.applyImage", p).unwrap();
    }

    /// Checks RGBA in 0..=255 levels, one level of slack (the fill's 50% alpha is 127.5).
    fn assert_rgba(got: &[f32], want: [f32; 4], what: &str) {
        let got: Vec<f32> = got.iter().map(|v| v * 255.0).collect();
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.01), "{what}: {got:?}, want {want:?}");
    }

    const GRAY: f32 = 128.0;

    const HALF: f32 = 127.5;

    #[test]
    fn normal_with_preserve_transparency_takes_the_source_and_keeps_alpha() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            apply(&mut s, json!({"blending": "normal", "preserveTransparency": true}));
            assert_rgba(&px(&s, 15, 8), [GRAY, GRAY, GRAY, HALF], &format!("depth {depth}: green pixel"));
            assert_rgba(&px(&s, 2, 2), [0.0; 4], &format!("depth {depth}: transparent pixel"));
        }
    }

    #[test]
    fn multiply_with_preserve_transparency_blends_fully_and_keeps_alpha() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            apply(&mut s, json!({"blending": "multiply", "preserveTransparency": true}));
            assert_rgba(&px(&s, 15, 8), [0.0, GRAY, 0.0, HALF], &format!("depth {depth}: green pixel"));
            assert_rgba(&px(&s, 2, 2), [0.0; 4], &format!("depth {depth}: transparent pixel"));
        }
    }

    #[test]
    fn opacity_and_selection_still_scale_the_preserved_blend() {
        let mut s = session(8);
        // 50% Normal: the colour moves half way to the source, the alpha stays.
        apply(&mut s, json!({"blending": "normal", "opacity": 50, "preserveTransparency": true}));
        assert_rgba(&px(&s, 15, 8), [64.0, 191.5, 64.0, HALF], "opacity 50");
        s.undo();
        // Outside the selection nothing changes.
        s.execute("select.rect", json!({"x": 10, "y": 4, "width": 5, "height": 10, "mode": "replace"})).unwrap();
        apply(&mut s, json!({"blending": "normal", "preserveTransparency": true}));
        assert_rgba(&px(&s, 12, 8), [GRAY, GRAY, GRAY, HALF], "inside the selection");
        assert_rgba(&px(&s, 17, 8), [0.0, 255.0, 0.0, HALF], "outside the selection");
    }

    #[test]
    fn a_mask_still_scales_the_preserved_blend() {
        let mut s = session(8);
        // A 50% gray alpha channel as the mask halves the effect, like 50% opacity.
        s.execute("channel.new", json!({"fill": "white"})).unwrap();
        s.execute("channel.target", json!({"channel": 0})).unwrap();
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        apply(&mut s, json!({"blending": "normal", "preserveTransparency": true, "mask": {"document": 0, "channel": 0}}));
        assert_rgba(&px(&s, 15, 8), [64.0, 191.5, 64.0, HALF], "mask at 50%");
    }

    #[test]
    fn without_preserve_transparency_the_results_stay() {
        let mut s = session(8);
        apply(&mut s, json!({"blending": "normal"}));
        assert_rgba(&px(&s, 15, 8), [GRAY, GRAY, GRAY, 255.0], "normal, green pixel");
        assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, 255.0], "normal, transparent pixel");
        s.undo();
        apply(&mut s, json!({"blending": "multiply"}));
        assert_rgba(&px(&s, 15, 8), [64.0, GRAY, 64.0, 255.0], "multiply, green pixel");
        assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, 255.0], "multiply, transparent pixel");
        s.undo();
        apply(&mut s, json!({"blending": "normal", "opacity": 50}));
        assert_rgba(&px(&s, 15, 8), [85.0, 170.0, 85.0, 191.5], "normal 50%, green pixel");
        assert_rgba(&px(&s, 2, 2), [GRAY, GRAY, GRAY, HALF], "normal 50%, transparent pixel");
    }
}

mod tests {
    use photocraft_color::{Color, ColorMode};
    use photocraft_doc::{ColorIndicates, Document, Layer};
    use photocraft_engine::Session;
    use serde_json::json;

    fn session_depth(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth})).unwrap();
        s
    }

    fn session() -> Session {
        session_depth(8)
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    fn chan(s: &Session, i: usize, x: i32, y: i32) -> f32 {
        doc(s).channels[i].surface.sample_channel(x, y, 0)
    }

    fn sel_at(s: &Session, x: i32, y: i32) -> f32 {
        doc(s).selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    fn rect(s: &mut Session, x: i32, y: i32, w: u32, h: u32, mode: &str) {
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h, "mode": mode})).unwrap();
    }

    #[test]
    fn load_transparency_mask_and_composite() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "background": "transparent"})).unwrap();
        rect(&mut s, 0, 0, 5, 10, "replace");
        s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.loadSelection", json!({"channel": "transparency"})).unwrap();
        assert_eq!((sel_at(&s, 2, 2), sel_at(&s, 10, 2)), (1.0, 0.0));
        // Composite luminosity: white = selected; transparent over nothing = black.
        s.execute("select.loadSelection", json!({"channel": "composite"})).unwrap();
        assert!(sel_at(&s, 2, 2) > 0.99);
        s.execute("select.loadSelection", json!({"channel": "red"})).unwrap();
        assert!(sel_at(&s, 2, 2) > 0.99);
        // Layer mask
        assert!(s.execute("select.loadSelection", json!({"channel": "mask"})).is_err());
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("select.loadSelection", json!({"channel": "mask", "invert": true})).unwrap();
        assert_eq!(sel_at(&s, 10, 5), 1.0);
    }

    /// A white CMYK document (no ink anywhere) whose left half carries some cyan ink.
    fn cmyk_session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": "cmyk", "depth": depth, "background": "white"})).unwrap();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute("edit.fill", json!({"color": "#00ffff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    #[test]
    fn cmyk_channels_duplicate_and_load_as_tone_not_ink() {
        // #5787: a CMYK channel is its tone, as the Channels panel draws it: white where there is no ink.
        // (The merged image goes through RGB, so inked pixels may drift a few levels.)
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let ink = px(&s, 5, 5)[0];
            assert!(ink > 0.5 && px(&s, 30, 5)[0] == 0.0, "depth {depth}: cyan ink on the left only, {ink}");
            s.execute("channel.duplicate", json!({"channel": "cyan"})).unwrap();
            assert_eq!(doc(&s).channels[0].name, "Cyan copy");
            assert!((chan(&s, 0, 30, 5) - 1.0).abs() < 1e-3, "depth {depth}: no ink duplicates as white");
            assert!((chan(&s, 0, 5, 5) - (1.0 - ink)).abs() < 0.02, "depth {depth}: ink duplicates dark");
            s.execute("select.loadSelection", json!({"channel": "cyan"})).unwrap();
            assert!((sel_at(&s, 30, 5) - 1.0).abs() < 1e-3, "depth {depth}: no ink is selected");
            assert!((sel_at(&s, 5, 5) - (1.0 - ink)).abs() < 0.02, "depth {depth}");
            // Calculations read the same tone.
            let r = s.execute("image.calculations", json!({"source1": {"channel": "cyan"}, "source2": {"channel": "cyan"}, "blending": "normal"})).unwrap();
            let i = r["channel"].as_u64().unwrap() as usize;
            assert!((chan(&s, i, 30, 5) - 1.0).abs() < 1e-3, "depth {depth}");
        }
    }

    #[test]
    fn quick_mask_and_a_layer_mask_can_be_the_only_visible_channel() {
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        s.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
        assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 0);
        assert!(s.active().unwrap().doc.quick_mask.is_some());
        s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
        // Nothing else to show: the colour channels come back.
        s.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
        assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 3);

        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        for name in ["red", "green", "blue"] {
            s.execute("channel.setVisible", json!({"channel": name, "visible": false})).unwrap();
        }
        assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 0, "a transparency mask can be shown alone");
    }

    #[test]
    fn painting_filtering_and_adjusting_into_an_alpha_channel() {
        for depth in [8, 16, 32] {
            let mut s = session_depth(depth);
            s.execute("channel.new", json!({})).unwrap();
            let before = px(&s, 10, 10);
            // Targeted channel: the stroke goes into the channel, not the layer.
            s.execute("paint.stroke", json!({"points": [[10, 10]], "size": 8, "hardness": 1.0, "color": "#ffffff"})).unwrap();
            assert!(chan(&s, 0, 10, 10) > 0.99, "depth {depth}");
            assert_eq!(chan(&s, 0, 30, 10), 0.0);
            assert_eq!(px(&s, 10, 10), before);
            // Erasing paints the background colour (white) on channels; use black foreground fill.
            s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
            assert!((chan(&s, 0, 30, 10) - 0.5).abs() < 0.01);
            // Adjustments apply to the channel (also where it was untouched).
            s.execute("image.adjustments.invert", json!({})).unwrap();
            assert!((chan(&s, 0, 30, 10) - 0.5).abs() < 0.01);
            assert_eq!(px(&s, 10, 10), before, "layer untouched by the adjustment");
            // Filters too, limited by the selection.
            s.execute("channel.new", json!({})).unwrap();
            s.execute("paint.stroke", json!({"points": [[20, 10]], "size": 6, "hardness": 1.0, "color": "#ffffff"})).unwrap();
            rect(&mut s, 0, 0, 20, 20, "replace");
            s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
            assert!(chan(&s, 1, 16, 10) > 0.0, "blurred inside the selection");
            assert_eq!(chan(&s, 1, 26, 10), 0.0, "outside the selection untouched");
            // Explicit targets work without targeting the channel.
            s.execute("channel.target", json!({"channel": "composite"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.execute("paint.bucket", json!({"x": 1, "y": 1, "color": "#ffffff", "target": {"channel": 1}, "tolerance": 0})).unwrap();
            assert_eq!(chan(&s, 1, 1, 1), 1.0);
            assert!(s.execute("paint.stroke", json!({"points": [[1, 1]], "target": {"channel": 9}})).is_err());
            // Undo restores the channel.
            s.undo();
            assert_eq!(chan(&s, 1, 1, 1), 0.0);
        }
    }

    #[test]
    fn single_colour_channel_target_limits_edits() {
        let mut s = session();
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![0.0, 1.0, 1.0, 1.0]);
        s.execute("channel.target", json!({"channel": "blue"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[20, 10]], "size": 6, "hardness": 1.0, "color": "#000000"})).unwrap();
        assert_eq!(px(&s, 20, 10), vec![0.0, 1.0, 0.0, 1.0]);
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn quick_mask_enter_paint_exit() {
        let mut s = session();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        let d = doc(&s);
        let q = d.quick_mask.as_ref().unwrap();
        assert!(d.selection.is_none());
        assert_eq!((q.surface.sample_channel(5, 5, 0), q.surface.sample_channel(30, 5, 0)), (1.0, 0.0));
        assert_eq!((q.opacity, q.indicates), (0.5, ColorIndicates::MaskedAreas));
        // Painting black masks; white unmasks. The layer is untouched.
        let before = px(&s, 5, 5);
        s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4, "hardness": 1.0, "color": "#000000"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[30, 10]], "size": 4, "hardness": 1.0, "color": "#ffffff"})).unwrap();
        assert_eq!(px(&s, 5, 5), before);
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        assert!(doc(&s).quick_mask.is_none());
        assert_eq!(sel_at(&s, 5, 5), 0.0, "painted black: masked");
        assert_eq!(sel_at(&s, 12, 12), 1.0);
        assert_eq!(sel_at(&s, 30, 10), 1.0, "painted white: selected");
        assert_eq!(sel_at(&s, 35, 2), 0.0);
        // History: enter and exit are steps; undo goes back into Quick Mask with the paint.
        let labels: Vec<String> = s.active().unwrap().history.entries().iter().map(|e| e.to_string()).collect();
        assert!(labels.iter().any(|l| l.contains("Edit in Quick Mask")) && labels.iter().any(|l| l.contains("Exit Quick Mask")), "{labels:?}");
        s.undo();
        assert!(doc(&s).quick_mask.is_some());
        assert!(doc(&s).selection.is_none());
        s.redo();
        assert!(doc(&s).quick_mask.is_none());
        assert_eq!(sel_at(&s, 30, 10), 1.0);
    }

    #[test]
    fn quick_mask_without_selection_and_options() {
        let mut s = session();
        s.execute("channel.options", json!({"channel": "quickMask", "color": "#0000ff", "opacity": 30})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
        let q = doc(&s).quick_mask.clone().unwrap();
        assert_eq!(q.color, Color::rgb(0.0, 0.0, 1.0));
        assert!((q.opacity - 0.3).abs() < 1e-6);
        // Leaving an untouched Quick Mask leaves no selection.
        s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
        assert!(doc(&s).selection.is_none());
        // A filter in Quick Mask edits the mask.
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10]], "size": 12, "hardness": 1.0, "color": "#000000"})).unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        let v = doc(&s).quick_mask.as_ref().unwrap().surface.sample_channel(16, 10, 0);
        assert!(v > 0.0 && v < 1.0, "feathered edge {v}");
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        assert!(sel_at(&s, 10, 10) < 0.05 && sel_at(&s, 30, 10) > 0.99, "{} {}", sel_at(&s, 10, 10), sel_at(&s, 30, 10));
    }

    /// Overlay coverage of the live Quick Mask at (x, y): how much colour the canvas shows there.
    fn qm_overlay(s: &Session, x: i32, y: i32) -> f32 {
        let q = doc(s).quick_mask.as_ref().unwrap();
        q.overlay_coverage(q.surface.sample_channel(x, y, 0))
    }

    #[test]
    fn quick_mask_without_selection_starts_empty_in_both_modes() {
        // #2743 case 1: nothing selected → nothing selected in the mask, whichever way the colour
        // reads; painting white adds to the selection that comes out.
        for depth in [8, 16, 32] {
            // Selected Areas set before entering.
            let mut s = session_depth(depth);
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": "selected"})).unwrap();
            s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            assert_eq!((qm_overlay(&s, 5, 5), qm_overlay(&s, 35, 15)), (0.0, 0.0), "depth {depth}: no overlay");
            s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
            assert!(doc(&s).selection.is_none(), "depth {depth}: untouched → no selection");
            s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4, "hardness": 1.0, "color": "#ffffff"})).unwrap();
            assert!(qm_overlay(&s, 5, 5) > 0.99 && qm_overlay(&s, 30, 10) == 0.0, "depth {depth}");
            s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
            assert!(sel_at(&s, 5, 5) > 0.99 && sel_at(&s, 30, 10) == 0.0, "depth {depth}: painted area is the selection");

            // Selected Areas switched on while in Quick Mask (the order in the report).
            let mut s = session_depth(depth);
            s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            assert_eq!(qm_overlay(&s, 5, 5), 0.0, "depth {depth}: Masked Areas, nothing masked yet");
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": "selected"})).unwrap();
            assert_eq!((qm_overlay(&s, 5, 5), qm_overlay(&s, 35, 15)), (0.0, 0.0), "depth {depth}: still no overlay");
            // …and back again: still the untouched, nothing-selected mask.
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": "masked"})).unwrap();
            assert_eq!(qm_overlay(&s, 5, 5), 0.0, "depth {depth}");
            s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
            assert!(doc(&s).selection.is_none(), "depth {depth}");
        }
    }

    #[test]
    fn apply_image_hand_computed() {
        let mut s = session();
        // Target: 50% gray. Source: a second document, red #ff0000 / 50% gray.
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff4000"})).unwrap();
        s.set_active(0);
        let g = 128.0 / 255.0;
        s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "multiply"})).unwrap();
        let p = px(&s, 3, 3);
        assert!((p[0] - g).abs() < 0.003 && (p[1] - g * 64.0 / 255.0).abs() < 0.003 && p[2].abs() < 0.003, "{p:?}");
        s.undo();
        // Single source channel applied to every target channel, screen at 50% opacity, inverted.
        s.execute("image.applyImage", json!({"source": {"document": 1, "channel": "green", "invert": true}, "blending": "screen", "opacity": 50})).unwrap();
        let top = 1.0 - 64.0 / 255.0;
        let screen = 1.0 - (1.0 - g) * (1.0 - top);
        let want = g + (screen - g) * 0.5;
        let p = px(&s, 3, 3);
        assert!(p[..3].iter().all(|v| (v - want).abs() < 0.003), "{p:?} vs {want}");
        s.undo();
        // Add with scale and offset; masked by an alpha channel; limited to the selection.
        s.execute("channel.new", json!({"fill": "white"})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute(
            "image.applyImage",
            json!({"source": {"document": 1, "channel": "red"}, "blending": "add", "scale": 2, "offset": -10, "mask": {"channel": 0}}),
        )
        .unwrap();
        let want = ((g + 1.0) / 2.0 - 10.0 / 255.0).clamp(0.0, 1.0);
        assert!((px(&s, 3, 3)[1] - want).abs() < 0.003);
        assert!((px(&s, 30, 3)[1] - g).abs() < 0.003, "outside the selection");
        // Size mismatch is refused.
        s.execute("file.new", json!({"width": 5, "height": 5})).unwrap();
        s.set_active(0);
        assert!(s.execute("image.applyImage", json!({"source": {"document": 2}})).is_err());
        assert!(s.execute("image.applyImage", json!({"blending": "hue"})).is_err());
    }

    #[test]
    fn apply_image_subtract_layer_from_itself_is_mid_grey() {
        // Frequency separation: Subtract, Scale 2, Offset 128, in the flat form the dialog sends.
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#c08040"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "High"})).unwrap();
        s.execute("edit.fill", json!({"color": "#406080"})).unwrap();
        s.execute("image.applyImage", json!({"sourceLayer": "High", "blending": "subtract", "scale": 2.0, "offset": 128})).unwrap();
        let p = px(&s, 3, 3);
        assert!(p[..3].iter().all(|v| (v - 128.0 / 255.0).abs() < 0.003), "{p:?}");
    }

    #[test]
    fn apply_image_onto_transparent_layer() {
        for depth in [8, 16, 32] {
            let mut s = session_depth(depth);
            s.execute("edit.fill", json!({"color": "#c08040"})).unwrap();
            let (r, g, b) = (192.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0);
            // An empty layer has no pixels to blend with: it takes the source, whatever the mode.
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("image.applyImage", json!({"blending": "multiply"})).unwrap();
            let p = px(&s, 3, 3);
            assert!((p[0] - r).abs() < 0.003 && (p[1] - g).abs() < 0.003 && (p[2] - b).abs() < 0.003 && p[3] > 0.999, "depth {depth}: {p:?}");
            s.undo();
            // At 50% opacity the empty layer becomes the source at 50% alpha.
            s.execute("image.applyImage", json!({"blending": "multiply", "opacity": 50})).unwrap();
            let p = px(&s, 3, 3);
            assert!((p[0] - r).abs() < 0.003 && (p[3] - 0.5).abs() < 0.003, "depth {depth}: {p:?}");
            s.undo();
            // Preserve Transparency leaves an empty layer empty.
            s.execute("image.applyImage", json!({"blending": "multiply", "preserveTransparency": true})).unwrap();
            assert!(px(&s, 3, 3)[3] < 0.001, "depth {depth}");
        }
    }

    #[test]
    fn apply_image_into_alpha_channel() {
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
        s.execute("channel.new", json!({"fill": "white"})).unwrap();
        // Targeted channel: Normal blend of the inverted composite → black.
        s.execute("image.applyImage", json!({"source": {"channel": "composite", "invert": true}, "blending": "normal"})).unwrap();
        assert!(chan(&s, 0, 3, 3) < 0.01);
    }

    #[test]
    fn cmyk_apply_image_of_a_colour_channel_keeps_ink_and_tone_apart() {
        // #5787: the Cyan channel is read as tone and written back as ink, so applying it onto its own
        // CMYK layer leaves cyan as it was, and into an alpha channel it lands as tone. (The merged
        // image goes through RGB, so a few levels of drift are allowed.)
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let before = (px(&s, 5, 5), px(&s, 30, 5));
            s.execute("image.applyImage", json!({"source": {"channel": "cyan"}, "blending": "normal"})).unwrap();
            let after = px(&s, 5, 5);
            assert!((after[0] - before.0[0]).abs() < 0.02, "depth {depth}: cyan unchanged, {after:?} vs {:?}", before.0);
            assert!(px(&s, 30, 5).iter().zip(&before.1).all(|(a, b)| (a - b).abs() < 1e-3), "depth {depth}: no ink stays no ink");
            s.undo();
            s.execute("image.applyImage", json!({"blending": "normal"})).unwrap();
            assert!(px(&s, 5, 5).iter().zip(&before.0).all(|(a, b)| (a - b).abs() < 0.02), "depth {depth}: CMYK onto itself is a no-op");
            s.execute("channel.new", json!({"fill": "black"})).unwrap();
            s.execute("image.applyImage", json!({"source": {"channel": "cyan"}, "blending": "normal"})).unwrap();
            assert!((chan(&s, 0, 30, 5) - 1.0).abs() < 1e-3, "depth {depth}: tone into an alpha channel");
        }
    }

    #[test]
    fn cmyk_apply_image_blends_as_tone() {
        // Photoshop blends a CMYK layer as tone: Multiply (the default) with a white source changes
        // nothing, and Multiply with Cyan multiplies every channel's tone by Cyan's, so it adds ink.
        // A spot channel reads as tone too: its solid ink applies as full ink and isn't selected.
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let (left, right) = (px(&s, 5, 5), px(&s, 30, 5));
            let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
            s.execute("channel.new", json!({"fill": "white"})).unwrap();
            s.execute("channel.target", json!({"channel": "composite"})).unwrap();
            s.execute("image.applyImage", json!({"source": {"channel": 0}})).unwrap();
            assert!(near(&px(&s, 5, 5), &left), "depth {depth}: white multiplies to no change, {:?} vs {left:?}", px(&s, 5, 5));
            s.undo();
            s.execute("image.applyImage", json!({"source": {"channel": "cyan"}, "blending": "multiply"})).unwrap();
            let want: Vec<f32> = left[..4].iter().map(|&ink| 1.0 - (1.0 - ink) * (1.0 - left[0])).chain([1.0]).collect();
            assert!(near(&px(&s, 5, 5), &want), "depth {depth}: {:?} vs {want:?}", px(&s, 5, 5));
            assert!(near(&px(&s, 30, 5), &right), "depth {depth}: no ink stays no ink");
            s.undo();
            rect(&mut s, 0, 0, 20, 20, "replace");
            let r = s.execute("channel.newSpot", json!({"color": "#ff00ff", "solidity": 100})).unwrap();
            let spot = r["channel"].as_u64().unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.execute("channel.target", json!({"channel": "composite"})).unwrap();
            s.execute("image.applyImage", json!({"source": {"channel": spot}, "blending": "normal"})).unwrap();
            assert!(near(&px(&s, 5, 5), &[1.0; 5]) && near(&px(&s, 30, 5), &right), "depth {depth}: spot ink applies as ink, {:?}", px(&s, 5, 5));
            s.execute("select.loadSelection", json!({"channel": spot})).unwrap();
            assert!(sel_at(&s, 5, 5) < 1e-3 && (sel_at(&s, 30, 5) - 1.0).abs() < 1e-3, "depth {depth}: the spot's unprinted area loads");
        }
    }

    #[test]
    fn apply_image_onto_a_spot_channel_blends_as_tone() {
        let near = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
        // #5787: a spot channel stores ink but reads and takes tone, so the spot applied onto itself
        // changes nothing (Normal or Multiply), and Cyan applied onto it gives it Cyan's ink.
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let cyan = px(&s, 5, 5)[0];
            rect(&mut s, 0, 0, 10, 20, "replace");
            let spot = s.execute("channel.newSpot", json!({"color": "#ff00ff", "solidity": 100})).unwrap()["channel"].as_u64().unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            let ink = |s: &Session| [5, 15, 30].map(|x| chan(s, spot as usize, x, 5));
            let before = ink(&s);
            assert!(near(&before, &[1.0, 0.0, 0.0]), "depth {depth}: {before:?}");
            for blending in ["normal", "multiply"] {
                s.execute("image.applyImage", json!({"source": {"channel": spot}, "blending": blending, "target": {"channel": spot}})).unwrap();
                assert!(near(&ink(&s), &before), "depth {depth}: {blending} onto itself, {:?}", ink(&s));
            }
            s.execute("image.applyImage", json!({"source": {"channel": "cyan"}, "blending": "normal", "target": {"channel": spot}})).unwrap();
            assert!(near(&ink(&s), &[cyan, cyan, 0.0]), "depth {depth}: Cyan's ink, {:?}", ink(&s));
        }
    }

    #[test]
    fn apply_image_from_an_rgb_document_onto_cmyk_reads_it_in_its_own_mode() {
        let near = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
        // #5787: an RGB composite is tone in RGB terms: white multiplied (the default) onto a CMYK
        // layer changes nothing.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": "cmyk", "background": "white"})).unwrap();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute("edit.fill", json!({"color": "#00ffff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let before = (px(&s, 5, 5), px(&s, 30, 5));
        s.execute("image.applyImage", json!({"source": {"document": 0}})).unwrap();
        assert!(near(&px(&s, 5, 5), &before.0) && near(&px(&s, 30, 5), &before.1), "{:?} vs {before:?}", (px(&s, 5, 5), px(&s, 30, 5)));
    }

    #[test]
    fn apply_image_of_a_cmyk_composite_onto_a_spot_channel_blends_as_tone() {
        // #5787: the CMYK composite stores ink, so its white has none: applied in Normal onto an empty
        // spot channel, the spot stays empty on the white and takes ink on the Cyan.
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let spot = s.execute("channel.newSpot", json!({"color": "#ff00ff", "solidity": 100})).unwrap()["channel"].as_u64().unwrap() as usize;
            s.execute("image.applyImage", json!({"blending": "normal", "target": {"channel": spot}})).unwrap();
            let (cyan, white) = (chan(&s, spot, 5, 5), chan(&s, spot, 30, 5));
            assert!(white < 0.01 && cyan > 0.1 && cyan < 0.9, "depth {depth}: cyan {cyan}, white {white}");
        }
    }

    #[test]
    fn apply_image_into_targeted_layer_mask_leaves_the_pixels() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "mask"})).unwrap();
        // The green channel of the red composite is black: it lands in the mask, not the layer.
        s.execute("image.applyImage", json!({"source": {"channel": "green"}, "blending": "normal"})).unwrap();
        let d = s.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        let m = &l.mask.as_ref().unwrap().surface;
        assert!(m.sample_channel(3, 3, 0) < 0.01, "mask {}", m.sample_channel(3, 3, 0));
        // Outside the canvas the mask keeps revealing.
        assert!(m.sample_channel(-5, -5, 0) > 0.99, "mask outside {}", m.sample_channel(-5, -5, 0));
        let p = px(&s, 3, 3);
        assert!(p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01 && p[3] > 0.99, "layer pixels {p:?}");
    }

    #[test]
    fn split_and_merge_channels() {
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
        s.execute("channel.new", json!({"fill": "white", "name": "A"})).unwrap();
        let r = s.execute("channel.split", json!({})).unwrap();
        assert_eq!(r["documents"].as_array().unwrap().len(), 4);
        assert_eq!(s.documents().len(), 4, "original closed");
        let names: Vec<_> = s.documents().iter().map(|d| d.doc.name.clone()).collect();
        assert_eq!(names, ["Untitled_R", "Untitled_G", "Untitled_B", "Untitled_A"]);
        let v = |s: &Session, i: usize| s.documents()[i].doc.layers[0].surface().unwrap().pixel(1, 1)[0];
        assert_eq!(v(&s, 0), 1.0);
        assert!((v(&s, 1) - 128.0 / 255.0).abs() < 0.003);
        assert_eq!(v(&s, 2), 0.0);
        // Merge the three colour docs back (order: active first, then the rest).
        s.set_active(0);
        let r = s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 1, 2]})).unwrap();
        assert_eq!(s.documents().len(), 2);
        assert_eq!(r["document"], 1);
        let p = px(&s, 1, 1);
        assert!((p[0] - 1.0).abs() < 0.003 && (p[1] - 128.0 / 255.0).abs() < 0.003 && p[2].abs() < 0.003, "{p:?}");
        assert!(s.execute("channel.merge", json!({"mode": "cmyk"})).is_err());
    }

    /// #9236: CMYK channels split into greyscale shown as the Channels panel shows them (white
    /// paper, dark ink), and merge back into the same inks.
    #[test]
    fn split_and_merge_cmyk_keep_ink_dark() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "mode": "cmyk", "background": "white"})).unwrap();
        // Rich black (#8359): its inks don't survive a trip through RGB, so the old path fails here.
        let rgb = photocraft_color::convert::cmyk_to_rgb([1.0, 1.0, 1.0, 0.0]).map(|c| c.clamp(0.0, 1.0));
        s.execute("tools.setColors", json!({"foreground": rgb, "foregroundCmyk": [1.0, 1.0, 1.0, 0.0]})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let (ink, paper) = (px(&s, 1, 1), px(&s, 30, 1));
        assert!(ink[..3].iter().all(|v| *v > 0.99) && paper[..4].iter().all(|v| *v < 0.01), "{ink:?} {paper:?}");
        s.execute("channel.split", json!({})).unwrap();
        assert_eq!(s.documents().len(), 4);
        for (i, ink) in ink[..4].iter().enumerate() {
            let at = |x| s.documents()[i].doc.layers[0].surface().unwrap().pixel(x, 1)[0];
            assert!((at(1) - (1.0 - ink)).abs() < 0.003, "channel {i}: {} for ink {ink}", at(1));
            assert!((at(30) - 1.0).abs() < 0.003, "channel {i}: paper is white, got {}", at(30));
        }
        s.execute("channel.merge", json!({"mode": "cmyk", "documents": [0, 1, 2, 3]})).unwrap();
        let back = px(&s, 1, 1);
        assert!(back[..4].iter().zip(&ink[..4]).all(|(a, b)| (a - b).abs() < 0.003), "{back:?} vs {ink:?}");
    }

    #[test]
    fn cmyk_split_and_merge_use_tone() {
        // #5787: Split Channels of a white CMYK image gives white grayscale documents, and merging
        // them back gives the same CMYK pixels (within the few levels the merged image's trip
        // through RGB costs).
        for depth in [8, 16] {
            let mut s = cmyk_session(depth);
            let (left, right) = (px(&s, 5, 5), px(&s, 30, 5));
            s.execute("channel.split", json!({})).unwrap();
            assert_eq!(s.documents().len(), 4);
            let v = |s: &Session, i: usize, x: i32| s.documents()[i].doc.layers[0].surface().unwrap().pixel(x, 5)[0];
            for i in 0..4 {
                assert!((v(&s, i, 30) - 1.0).abs() < 1e-3, "depth {depth}: channel {i} of white is white");
            }
            assert!((v(&s, 0, 5) - (1.0 - left[0])).abs() < 0.02, "depth {depth}: cyan ink splits dark");
            s.set_active(0);
            s.execute("channel.merge", json!({"mode": "cmyk", "documents": [0, 1, 2, 3]})).unwrap();
            assert_eq!(doc(&s).mode, ColorMode::Cmyk);
            let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02);
            assert!(near(&px(&s, 5, 5), &left), "depth {depth}: round trip {:?} vs {left:?}", px(&s, 5, 5));
            assert!(near(&px(&s, 30, 5), &right), "depth {depth}: white stays white {:?}", px(&s, 30, 5));
        }
    }

    #[test]
    fn merge_rejects_a_repeated_source_and_keeps_unrelated_documents() {
        // #934: `documents: [0, 0, 1]` passed validation, then the close loop ran `close(1)`,
        // `close(0)`, `close(0)` and the last call removed the unrelated RGB document at index 2.
        let mut s = session();
        s.execute("channel.split", json!({})).unwrap();
        assert_eq!(s.documents().len(), 3);
        s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        let keep = s.documents()[3].doc.id;
        let before: Vec<_> = s.documents().iter().map(|d| d.doc.id).collect();
        let err = s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 0, 1]})).unwrap_err();
        assert!(err.to_string().contains("more than once"), "{err}");
        let after: Vec<_> = s.documents().iter().map(|d| d.doc.id).collect();
        assert_eq!(after, before, "a rejected merge closes nothing");
        assert!(s.documents().iter().any(|d| d.doc.id == keep));
        assert!(s.set_active(3) && s.undo(), "the unrelated document keeps its history");
        // Distinct indices still merge and close exactly the named sources.
        s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 1, 2]})).unwrap();
        assert_eq!(s.documents().len(), 2);
        assert!(s.documents().iter().any(|d| d.doc.id == keep));
    }

    #[test]
    fn clear_with_a_colour_channel_targeted_clears_only_that_channel() {
        let near = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        // The Background, then a normal layer (Delete there can't make one channel transparent).
        for duplicate in [false, true] {
            let mut s = session_depth(16);
            rect(&mut s, 0, 0, 20, 20, "replace");
            s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            if duplicate {
                s.execute("layer.duplicate", json!({})).unwrap();
            }
            let (left, right) = (px(&s, 5, 5), px(&s, 30, 5));
            s.tools.background = [0.8, 0.1, 0.9, 1.0];
            s.execute("channel.target", json!({"channel": "red"})).unwrap();
            rect(&mut s, 2, 2, 10, 10, "replace");
            s.execute("edit.clear", json!({})).unwrap();
            // Red takes the background colour's red; green, blue and transparency stay.
            assert!(near(&px(&s, 5, 5), &[0.8, left[1], left[2], 1.0]), "duplicate {duplicate}: {:?}", px(&s, 5, 5));
            assert!(near(&px(&s, 15, 5), &left), "outside the selection");
            assert!(near(&px(&s, 30, 5), &right));
            s.undo();
            assert!(near(&px(&s, 5, 5), &left), "one undoable step");
        }
    }

    #[test]
    fn command_alt_2_loads_the_images_luminosity() {
        let mut s = session();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        assert_eq!(photocraft_engine::commands::find("select.loadLuminosity").and_then(|c| c.shortcut), Some("Cmd+Alt+2"));
        s.execute("select.loadLuminosity", json!({})).unwrap();
        assert!(sel_at(&s, 5, 5) < 0.01, "black: unselected");
        assert!(sel_at(&s, 30, 5) > 0.99, "white: selected");
    }

    /// Split Channels closes the original, so as in the reference application it splits only an
    /// image with a single layer: two layers (even with one hidden) or a lone group are refused and
    /// the document stays open with its layers; a flattened image or a lone normal layer still
    /// splits (a played `SplC` step goes through the same check).
    #[test]
    fn split_refuses_an_image_with_more_than_one_layer() {
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        assert!(!s.is_enabled("channel.split"));
        let err = s.execute("channel.split", json!({})).unwrap_err();
        assert!(err.to_string().contains("flatten"), "{err}");
        assert_eq!(s.documents().len(), 1, "the layered document stays open");
        assert_eq!(doc(&s).layers.len(), 2, "with its layers");
        std::sync::Arc::make_mut(s.active_mut().unwrap().doc_mut()).layers[1].visible = false;
        assert!(s.execute("channel.split", json!({})).is_err(), "a hidden second layer still counts");
        assert_eq!((s.documents().len(), doc(&s).layers.len()), (1, 2));
        s.execute("layer.flattenImage", json!({})).unwrap();
        assert!(s.is_enabled("channel.split"));
        assert_eq!(s.execute("channel.split", json!({})).unwrap()["documents"].as_array().unwrap().len(), 3);
        // A lone normal layer (Layer 0) splits too.
        let mut s = session();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        assert!(!doc(&s).is_background(doc(&s).layers[0].id));
        assert!(s.is_enabled("channel.split"));
        assert_eq!(s.execute("channel.split", json!({})).unwrap()["documents"].as_array().unwrap().len(), 3);
        // A lone group is a layer the split would lose.
        let mut s = session();
        std::sync::Arc::make_mut(s.active_mut().unwrap().doc_mut()).layers = vec![Layer::group("Group 1", vec![])].into();
        assert!(!s.is_enabled("channel.split"));
        assert!(s.execute("channel.split", json!({})).is_err());
        assert_eq!(s.documents().len(), 1);
    }
}
