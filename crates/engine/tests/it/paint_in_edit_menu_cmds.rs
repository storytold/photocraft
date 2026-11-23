//! Tests that run paint commands, moved from the engine-core `edit_menu_cmds` tests: 15 test(s).

mod fade_visibility_tests {
    use photocraft_engine::Session;
    use photocraft_engine::edit_menu_cmds::*;
    use serde_json::{Value, json};

    use photocraft_doc::LayerId;

    /// A command and its params, given the Background and the faded layer.
    type Step = (&'static str, fn(LayerId, LayerId) -> Value);

    /// A white Background and a layer `Paint` with a black square, made active and blurred:
    /// (session, Background, Paint, Paint's edge pixel before the blur).
    fn blurred() -> (Session, LayerId, LayerId, Vec<f32>) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8, "background": "white"})).unwrap();
        let bg = s.active().unwrap().active_layer.unwrap();
        let paint = LayerId(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let sharp = pixel(&s, paint);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        assert_ne!(pixel(&s, paint), sharp, "the blur changed the edge pixel");
        assert!(s.is_enabled("edit.fade"), "Fade follows the blur");
        (s, bg, paint, sharp)
    }

    fn pixel(s: &Session, layer: LayerId) -> Vec<f32> {
        s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(16, 20)
    }

    fn visible(s: &Session, layer: LayerId) -> bool {
        s.active().unwrap().doc.layer(layer).unwrap().visible
    }

    /// Hiding the Background after the blur, by each way of hiding it, with visibility changes
    /// recorded in History or not: Fade stays available and fades the blur towards the square as it
    /// was before the blur, and the Background stays hidden.
    #[test]
    fn hiding_another_layer_keeps_fade() {
        let hides: [Step; 3] = [
            ("layer.hideLayers", |bg, _| json!({"layer": bg.0})),
            ("layer.setProps", |bg, _| json!({"layer": bg.0, "visible": false})),
            ("layer.showOnly", |_, _| json!({})),
        ];
        for undoable in [true, false] {
            for (hide, params) in hides {
                let (mut s, bg, paint, sharp) = blurred();
                s.execute("history.options", json!({"makeLayerVisibilityChangesUndoable": undoable})).unwrap();
                assert!(s.is_enabled("edit.fade"), "History Options don't touch the document");
                let steps = s.active().unwrap().history.past_len();

                s.execute(hide, params(bg, paint)).unwrap();
                assert!(!visible(&s, bg), "{hide} undoable={undoable}: the Background is hidden");
                assert_eq!(s.active().unwrap().history.past_len(), steps + usize::from(undoable), "{hide} undoable={undoable}");
                assert!(s.is_enabled("edit.fade"), "{hide} undoable={undoable}: Fade stays available");
                let preview = fade_preview(&s, &json!({"opacity": 0})).and_then(FadePreview::render).unwrap();
                assert_eq!(preview.layer(paint).unwrap().surface().unwrap().pixel(16, 20), sharp, "{hide} undoable={undoable}: the preview fades the blur");

                s.execute("edit.fade", json!({"opacity": 0})).unwrap();
                assert_eq!(pixel(&s, paint), sharp, "{hide} undoable={undoable}: 0 % fades the blur away");
                assert!(!visible(&s, bg), "{hide} undoable={undoable}: Fade leaves the Background hidden");
                assert_eq!(s.active().unwrap().history.undo_label(), Some("Fade Gaussian Blur"));
                assert!(!s.is_enabled("edit.fade"), "{hide} undoable={undoable}: Fade can't repeat");
            }
        }
    }

    /// Showing the hidden layer again keeps Fade available too, and Fade at 50 % lands halfway
    /// between the square before the blur and after it.
    #[test]
    fn hiding_and_showing_another_layer_keeps_fade() {
        let (mut s, bg, paint, sharp) = blurred();
        let blurred_px = pixel(&s, paint);
        s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
        s.execute("layer.showLayers", json!({"layer": bg.0})).unwrap();
        assert!(s.is_enabled("edit.fade"));
        s.execute("edit.fade", json!({"opacity": 50})).unwrap();
        let half = pixel(&s, paint);
        for c in 0..half.len() {
            assert!((half[c] - (sharp[c] + blurred_px[c]) / 2.0).abs() < 0.01, "channel {c}: {half:?} between {sharp:?} and {blurred_px:?}");
        }
    }

    /// Selecting another layer, or a step that is neither fadeable nor a visibility change, disables
    /// Fade as before.
    #[test]
    fn other_steps_still_disable_fade() {
        let steps: [Step; 4] = [
            ("layer.select", |bg, _| json!({"layer": bg.0})),
            ("select.all", |_, _| json!({})),
            ("view.colorSamplers.add", |_, _| json!({"x": 15, "y": 15})),
            ("layer.setProps", |_, paint| json!({"layer": paint.0, "opacity": 0.5})),
        ];
        for (step, params) in steps {
            let (mut s, bg, paint, _) = blurred();
            s.execute(step, params(bg, paint)).unwrap();
            assert!(!s.is_enabled("edit.fade"), "{step}: Fade is disabled");
            assert!(s.execute("edit.fade", json!({})).is_err(), "{step}");
            // A visibility change afterwards doesn't bring it back.
            s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
            assert!(!s.is_enabled("edit.fade"), "{step}, then a hide: Fade stays disabled");
        }
        // A hide together with another property is not a visibility change only.
        let (mut s, bg, _, _) = blurred();
        s.execute("layer.setProps", json!({"layer": bg.0, "visible": false, "name": "Back"})).unwrap();
        assert!(!s.is_enabled("edit.fade"), "a hide with a rename disables Fade");
    }

    /// Hiding the faded layer itself was not measured in the reference: Fade stays disabled, by itself
    /// and when a later step shows the layer again.
    #[test]
    fn hiding_the_faded_layer_disables_fade() {
        let (mut s, _, paint, _) = blurred();
        s.execute("layer.hideLayers", json!({})).unwrap();
        assert!(!visible(&s, paint), "Hide Layers hid the active, faded layer");
        assert!(!s.is_enabled("edit.fade"));
        s.execute("layer.showLayers", json!({"layer": paint.0})).unwrap();
        assert!(!s.is_enabled("edit.fade"), "showing it again doesn't bring Fade back");
    }
}

mod tests {
    /// Serialises the tests of this binary that purge shared caches (core's lock cannot reach across test binaries).
    static PURGE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use photocraft_doc::Layer;
    use photocraft_doc::vector::Path;
    use photocraft_engine::Session;
    use photocraft_engine::edit_menu_cmds::*;
    use serde_json::json;

    use photocraft_doc::vector::Subpath;

    fn session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "background": "white"})).unwrap();
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    fn px_rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
        active(s).surface().unwrap().rgba(x, y)
    }

    fn active(s: &Session) -> &Layer {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap()
    }

    /// #8230: the Fade dialog's preview is the document Fade would leave, at every depth and in a
    /// blend mode, with no history step and nothing changed until OK; with nothing to fade or a bad
    /// mode it is an error, not a panic.
    #[test]
    fn fade_preview_matches_fade_without_a_step() {
        for depth in [8, 16, 32] {
            assert!(fade_preview(&session(depth), &json!({"opacity": 50})).is_err(), "nothing to fade yet");
            for p in [json!({"opacity": 25}), json!({"opacity": 60, "mode": "difference"}), json!({})] {
                let mut s = session(depth);
                s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
                let (steps, revision) = (s.active().unwrap().history.past_len(), s.active().unwrap().revision);
                let preview = fade_preview(&s, &p).and_then(FadePreview::render).unwrap();
                assert_eq!((s.active().unwrap().history.past_len(), s.active().unwrap().revision), (steps, revision), "depth {depth} {p}: no step");
                assert_eq!(px(&s, 5, 5)[0], 0.0, "depth {depth} {p}: the layer is untouched");
                let id = s.active().unwrap().active_layer.unwrap();
                let shown = preview.layer(id).unwrap().surface().unwrap().pixel(5, 5);
                s.execute("edit.fade", p.clone()).unwrap();
                assert_eq!(shown, px(&s, 5, 5), "depth {depth} {p}: the preview is what OK applies");
            }
            let mut s = session(depth);
            s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
            assert!(fade_preview(&s, &json!({"mode": "nonsense"})).is_err());
            s.execute("layer.new.layer", json!({})).unwrap();
            assert!(fade_preview(&s, &json!({})).is_err(), "not right after the step");
        }
    }

    #[test]
    fn fade_blends_last_step_at_all_depths() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            assert!(!s.is_enabled("edit.fade"), "nothing to fade yet");
            s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
            assert!(s.is_enabled("edit.fade"));
            let undo_steps = s.active().unwrap().history.past_len();
            s.execute("edit.fade", json!({"opacity": 25})).unwrap();
            let v = px(&s, 5, 5);
            assert!((v[0] - 0.75).abs() < 0.01, "depth {depth}: {v:?}");
            // One history step, labelled after the faded command, and Fade can't repeat.
            assert_eq!(s.active().unwrap().history.past_len(), undo_steps + 1);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Fade Fill"));
            assert!(!s.is_enabled("edit.fade"));
            s.undo();
            assert_eq!(px(&s, 5, 5)[0], 0.0);
        }
    }

    #[test]
    fn fade_with_blend_mode_and_invalidation() {
        let mut s = session(8);
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        // Difference of grey and inverted grey at 100%: |0.502 − 0.498| ≈ 0.
        s.execute("edit.fade", json!({"opacity": 100, "mode": "difference"})).unwrap();
        assert!(px(&s, 1, 1)[0] < 0.02);
        // Any other edit in between disables Fade.
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        assert!(!s.is_enabled("edit.fade"));
        assert!(s.execute("edit.fade", json!({})).is_err());
        // Queries don't.
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("document.inspect", json!({})).unwrap();
        assert!(s.is_enabled("edit.fade"));
        assert!(s.execute("edit.fade", json!({"mode": "nonsense"})).is_err());
    }

    /// A command told to edit the layer (`"target": "pixels"`) does so with an alpha channel targeted,
    /// and Fade then fades the layer, not the channel.
    #[test]
    fn fade_follows_the_target_the_command_ran_on() {
        let mut s = session(8);
        s.execute("channel.new", json!({"fill": "black"})).unwrap();
        s.execute("channel.target", json!({"channel": 0})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000", "target": "pixels"})).unwrap();
        assert_eq!(px(&s, 5, 5)[0], 0.0, "the layer was filled");
        assert!(matches!(s.edit_state.fade.as_ref().map(|f| f.target), Some(FadeTarget::Pixels(_))));
        s.execute("edit.fade", json!({"opacity": 25})).unwrap();
        assert!((px(&s, 5, 5)[0] - 0.75).abs() < 0.01, "the layer faded");
        assert_eq!(s.active().unwrap().doc.channels[0].surface.pixel(5, 5)[0], 0.0, "the channel is left alone");
    }

    /// Edit › Stroke and the Magic Eraser follow the Channels panel's target but still edit the layer,
    /// so with an alpha channel targeted Fade fades the layer they changed, not the untouched channel.
    #[test]
    fn fade_after_a_routed_command_that_edited_the_layer_fades_the_layer() {
        for depth in [8, 16, 32] {
            for command in ["edit.stroke", "paint.magicEraser"] {
                let mut s = session(depth);
                s.execute("layer.new.layer", json!({})).unwrap();
                s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
                s.execute("channel.new", json!({"fill": "black"})).unwrap();
                s.execute("channel.target", json!({"channel": 0})).unwrap();
                let (x, y) = if command == "edit.stroke" {
                    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
                    s.execute(command, json!({"width": 2, "color": "#000000", "location": "inside"})).unwrap();
                    assert_eq!(px(&s, 10, 15)[0], 0.0, "{depth} {command}: the layer was stroked");
                    (10, 15)
                } else {
                    s.execute(command, json!({"x": 5, "y": 5, "antiAlias": false})).unwrap();
                    assert_eq!(px(&s, 5, 5)[3], 0.0, "{depth} {command}: the layer was erased");
                    (5, 5)
                };
                assert!(matches!(s.edit_state.fade.as_ref().map(|f| f.target), Some(FadeTarget::Pixels(_))), "{depth} {command}");
                s.execute("edit.fade", json!({"opacity": 50})).unwrap();
                let k = if command == "edit.stroke" { 0 } else { 3 };
                assert!((px(&s, x, y)[k] - 0.5).abs() < 0.01, "{depth} {command}: the layer faded {:?}", px(&s, x, y));
                assert_eq!(s.active().unwrap().doc.channels[0].surface.pixel(x, y)[0], 0.0, "{depth} {command}: the channel is left alone");
            }
        }
    }

    #[test]
    fn purge_commands() {
        let _purge_test_guard = PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut s = session(8);
        assert!(!s.is_enabled("edit.purge.undo"));
        assert!(s.is_enabled("edit.purge.videoCache"));
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let r = s.execute("edit.purge.undo", json!({})).unwrap();
        assert_eq!(r["purged"][0], "undo");
        assert!(r["bytes"].as_u64().unwrap() > 0);
        // One step left to undo (the black fill is gone from history).
        assert!(s.undo());
        assert!(!s.undo());
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        assert!(s.is_enabled("edit.purge.clipboard"));
        s.execute("edit.purge.clipboard", json!({})).unwrap();
        assert!(s.clipboard.is_none());
        assert!(!s.is_enabled("edit.purge.clipboard"));
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("edit.purge.histories", json!({})).unwrap();
        assert!(s.documents().iter().all(|d| !d.history.can_undo()));
        assert!(!s.is_enabled("edit.purge.histories"));
        assert_eq!(s.execute("edit.purge.videoCache", json!({})).unwrap()["bytes"], 0);
        let _ = s.execute("edit.purge.all", json!({}));
    }

    /// On the Background, which can't hold transparency, the area a selection's Content-Aware Scale
    /// vacates takes the background colour, as Delete leaves it (Photoshop); the layer stays the
    /// Background. On a normal layer it is left transparent.
    #[test]
    fn content_aware_scale_on_the_background_vacates_to_the_background_colour() {
        for depth in [8, 16] {
            let mut s = session(depth);
            s.tools.background = [0.0, 0.0, 1.0, 1.0];
            let id = s.active().unwrap().active_layer.unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
            let r = s.execute("edit.contentAwareScale", json!({"width": 10})).unwrap();
            assert_eq!(r["bounds"], json!([10, 10, 10, 10]), "{depth}-bit");
            let st = s.active().unwrap();
            assert!(st.doc.is_background(id), "{depth}-bit: still the Background");
            let px = |x, y| st.doc.layer(id).unwrap().surface().unwrap().rgba(x, y);
            for (x, y) in [(20, 10), (25, 15), (29, 19)] {
                assert_eq!(px(x, y), [0.0, 0.0, 1.0, 1.0], "{depth}-bit: ({x}, {y}) is vacated");
            }
            assert_eq!(px(12, 15), [1.0, 1.0, 1.0, 1.0], "{depth}-bit: the scaled pixels");
            assert_eq!(px(5, 5), [1.0, 1.0, 1.0, 1.0], "{depth}-bit: outside the selection");
            // A normal layer's vacated area is transparent.
            let mut s = session(depth);
            s.tools.background = [0.0, 0.0, 1.0, 1.0];
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("edit.contentAwareScale", json!({"width": 10})).unwrap();
            assert_eq!(px_rgba(&s, 25, 15)[3], 0.0, "{depth}-bit: a layer's hole is transparent");
            assert_eq!(px_rgba(&s, 12, 15), [1.0, 0.0, 0.0, 1.0], "{depth}-bit");
        }
    }

    fn square_path(x: f64, y: f64, w: f64) -> Path {
        Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + w), (x, y + w)])])
    }

    #[test]
    fn define_custom_shape_and_preset_manager() {
        let mut s = session(8);
        assert!(!s.is_enabled("edit.defineCustomShape"));
        s.edit("path", |doc, _| {
            doc.work_path = Some(square_path(4.0, 4.0, 10.0));
            Ok(())
        })
        .unwrap();
        s.execute("edit.defineCustomShape", json!({"name": "Box"})).unwrap();
        s.execute("edit.defineCustomShape", json!({})).unwrap();
        assert_eq!(custom_shapes(&s).len(), 2);
        assert_eq!(custom_shapes(&s)[1].name, "Shape 2");
        let list = s.execute("edit.presets.presetManager", json!({})).unwrap();
        assert_eq!(list["customShapes"], json!(["Box", "Shape 2"]));
        assert!(list["brushes"].as_array().unwrap().len() > 3);
        s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "customShapes", "name": "Shape 2", "newName": "Star"})).unwrap();
        s.execute("edit.presets.presetManager", json!({"action": "move", "kind": "customShapes", "index": 1, "to": 0})).unwrap();
        assert_eq!(custom_shapes(&s)[0].name, "Star");
        s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "customShapes", "name": "Box"})).unwrap();
        assert_eq!(custom_shapes(&s).len(), 1);
        assert!(s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "gradients", "index": 0})).is_err());
        // The pattern library (Edit › Define Pattern) is managed here too.
        let n = list["patterns"].as_array().unwrap().len();
        assert!(n > 0);
        s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "patterns", "index": 0, "newName": "First"})).unwrap();
        s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "patterns", "name": "First"})).unwrap();
        assert_eq!(s.patterns.items.len(), n - 1);
        // Export → import into a fresh session.
        s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
        let out = s.execute("edit.presets.exportImportPresets", json!({"action": "export"})).unwrap();
        assert_eq!(out["brushes"], 1, "built-ins are not exported by default");
        let mut t = Session::new();
        let r = t.execute("edit.presets.exportImportPresets", json!({"action": "import", "data": out["data"].to_string()})).unwrap();
        assert_eq!((r["brushes"].as_u64(), r["customShapes"].as_u64()), (Some(1), Some(1)));
        assert!(t.tools.presets.iter().any(|b| b.name == "Mine"));
        assert_eq!(custom_shapes(&t)[0].path, custom_shapes(&s)[0].path);
        assert!(t.execute("edit.presets.exportImportPresets", json!({"action": "import", "data": {"format": "other"}})).is_err());
    }

    #[test]
    fn preset_manager_brush_rename_follows_current_preset() {
        let mut s = Session::new();
        s.execute("brush.presets.setCurrent", json!({"name": "Chalk"})).unwrap();
        s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Chalk", "newName": "Chalky"})).unwrap();
        assert_eq!(s.tools.current_preset.as_deref(), Some("Chalky"));
        s.execute("brush.presets.update", json!({})).unwrap();
    }

    #[test]
    fn preset_manager_brush_delete_clears_current_preset() {
        let mut s = Session::new();
        s.execute("brush.presets.setCurrent", json!({"name": "Chalk"})).unwrap();
        s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "brushes", "name": "Chalk"})).unwrap();
        assert!(!s.tools.presets.iter().any(|b| b.name == "Chalk"));
        assert_eq!(s.tools.current_preset, None);
    }

    #[test]
    fn define_brush_preset_wraps_define_from_selection() {
        let mut s = session(8);
        assert!(s.is_enabled("edit.defineBrushPreset"));
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        s.execute("edit.defineBrushPreset", json!({})).unwrap();
        assert!(s.tools.presets.iter().any(|b| b.name == "Sampled Brush 1"));
    }
}
