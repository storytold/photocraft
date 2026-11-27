//! Tests that run paint commands, moved from the engine-core `file_cmds` tests: 9 test(s).

mod layers_to_files_prefix_tests {
    use photocraft_engine::Session;
    use serde_json::json;

    use photocraft_engine::file_cmds::{export_layers_report_with, export_layers_with};

    fn session(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"name": "project.v2.psd", "width": 4, "height": 3, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Type"})).unwrap();
        s.execute("edit.fill", json!({"color": "#406080"})).unwrap();
        s
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn empty_prefix_writes_underscore_names_at_each_depth() {
        for depth in [8, 16, 32] {
            let dir = tempfile::tempdir().unwrap();
            let mut s = session(depth);
            let result = s.execute("file.export.layersToFiles", json!({"dir": dir.path(), "prefix": ""})).unwrap();
            let files = result["files"].as_array().unwrap();
            assert_eq!(files.len(), 2);
            for (file, expected) in files.iter().zip(["_0000_Type.png", "_0001_Background.png"]) {
                let path = std::path::Path::new(file.as_str().unwrap());
                assert_eq!(path.file_name().unwrap(), expected, "depth {depth}");
                assert_eq!(photocraft_io::import(expected, &std::fs::read(path).unwrap()).unwrap().document.size, s.active().unwrap().doc.size);
            }
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn automation_shared_export_path_preserves_the_empty_prefix() {
        let s = session(8);
        let params = json!({"prefix": ""});
        let mut written = Vec::new();
        let report = export_layers_report_with(&s, &params, |name, bytes| {
            written.push(name.to_string());
            assert_eq!(photocraft_io::import(name, bytes).unwrap().document.size, s.active().unwrap().doc.size);
            Ok(())
        })
        .unwrap();
        assert_eq!(report.files, ["_0000_Type.png", "_0001_Background.png"]);
        assert_eq!(written, report.files);
        assert!(report.skipped.is_empty());
        assert_eq!(export_layers_with(&s, &params, |_, _| Ok(())).unwrap(), report.files);
    }

    #[test]
    fn omitted_and_nonempty_prefixes_keep_their_defaults_and_sanitizing() {
        let s = session(8);
        for (params, prefix) in [
            (json!({}), "project.v2"),
            (json!({"prefix": "card-1"}), "card-1"),
            (json!({"prefix": "..."}), "layer"),
            (json!({"prefix": "   "}), "layer"),
            (json!({"prefix": "/?*"}), "___"),
            (json!({"prefix": "a/b"}), "a_b"),
        ] {
            let files = export_layers_with(&s, &params, |_, _| Ok(())).unwrap();
            assert_eq!(files, [format!("{prefix}_0000_Type.png"), format!("{prefix}_0001_Background.png")], "{params}");
        }
    }

    #[test]
    fn empty_prefix_keeps_layer_name_fallbacks_and_sanitizing() {
        for (name, sanitized) in [("", "layer"), ("...", "layer"), ("   ", "layer"), ("/?*", "___")] {
            let mut s = session(8);
            s.execute("layer.setProps", json!({"name": name})).unwrap();
            let files = export_layers_with(&s, &json!({"prefix": ""}), |_, _| Ok(())).unwrap();
            assert_eq!(files, [format!("_0000_{sanitized}.png"), "_0001_Background.png".into()], "layer name {name:?}");
        }
    }
}

mod tests {
    use photocraft_color::{ColorMode, PixelFormat};
    use photocraft_doc::{Document, LayerContent};
    use photocraft_engine::Session;
    use photocraft_geom::Rect;
    use serde_json::json;

    fn tmp(name: &str) -> String {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("photocraft-file-cmds-{}-{name}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    fn session(w: u32, h: u32, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
        s
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    fn composite(s: &Session, x: i32, y: i32) -> [f32; 4] {
        let b = photocraft_compose::render(doc(s), Rect::new(x, y, x + 1, y + 1));
        b.px[0]
    }

    #[test]
    fn flatten_all_layer_effects_and_masks() {
        for depth in [8, 16, 32] {
            let mut s = session(40, 40, depth);
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            s.edit("fx", |doc, _| {
                let l = doc.layer_mut(id).unwrap();
                l.effects.enabled = true;
                l.effects.items.push(photocraft_doc::Effect::ColorOverlay {
                    common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                    color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
                });
                Ok(())
            })
            .unwrap();
            let before = composite(&s, 15, 15);
            assert!(before[1] > 0.99, "overlay renders green");
            assert_eq!(s.execute("file.scripts.flattenAllLayerEffects", json!({})).unwrap()["flattened"], 1);
            let l = doc(&s).layer(id).unwrap();
            assert!(l.effects.items.is_empty() && matches!(l.content, LayerContent::Raster(_)));
            let after = composite(&s, 15, 15);
            assert!((after[1] - before[1]).abs() < 0.01 && after[0] < 0.01, "{after:?}");
            // Masks: hide the left half, flatten, mask gone, pixels gone.
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 40})).unwrap();
            s.execute("select.inverse", json!({})).unwrap();
            s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
            assert!(composite(&s, 15, 15)[1] > 0.99 && composite(&s, 15, 15)[0] > 0.99, "white shows through the mask");
            let r = s.execute("file.scripts.flattenAllMasks", json!({})).unwrap();
            assert_eq!(r["flattened"], 1);
            let l = doc(&s).layer(id).unwrap();
            assert!(l.mask.is_none());
            assert!(l.surface().unwrap().content_bounds().x0 >= 20);
            assert!(composite(&s, 25, 15)[1] > 0.99 && composite(&s, 25, 15)[0] < 0.01);
            // Each script is a single history step.
            assert!(s.undo());
            assert!(doc(&s).layer(id).unwrap().mask.is_some());
        }
    }

    /// Flattens all layer effects on a red square carrying `fx` (in a document prepared by `setup`)
    /// and asserts the composite did not change: the bake uses the document's own context (#5719).
    fn assert_flatten_effects_is_a_no_op(depth: u32, setup: impl FnOnce(&mut Document), fx: photocraft_doc::Effect) {
        let mut s = session(40, 40, depth);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 14, "y": 14, "width": 12, "height": 12})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("fx", |doc, _| {
            setup(doc);
            let l = doc.layer_mut(id).unwrap();
            l.effects.enabled = true;
            l.effects.items.push(fx);
            Ok(())
        })
        .unwrap();
        let before = photocraft_compose::flatten(doc(&s));
        assert_eq!(s.execute("file.scripts.flattenAllLayerEffects", json!({})).unwrap()["flattened"], 1);
        let after = photocraft_compose::flatten(doc(&s));
        for (i, (a, b)) in after.px.iter().zip(&before.px).enumerate() {
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 0.02, "depth {depth}: pixel ({}, {}) changed: {b:?} -> {a:?}", i % 40, i / 40);
            }
        }
    }

    /// A Drop Shadow that uses Global Light is baked at the document's angle, not the default 120°.
    #[test]
    fn flatten_all_layer_effects_uses_the_documents_global_light() {
        for depth in [8, 16, 32] {
            let shadow = photocraft_doc::Effect::DropShadow(photocraft_doc::effects::Shadow {
                common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                color: photocraft_color::Color::rgb(0.0, 0.0, 1.0),
                angle: 120.0,
                use_global_light: true,
                distance: 10.0,
                spread: 0.0,
                size: 0.0,
                contour: photocraft_doc::effects::Contour::Linear,
                anti_alias: false,
                noise: 0.0,
                knocks_out: true,
            });
            assert_flatten_effects_is_a_no_op(depth, |d| d.global_light.angle = 0.0, shadow);
        }
    }

    /// A Pattern Overlay of a document pattern survives the bake instead of vanishing.
    #[test]
    fn flatten_all_layer_effects_keeps_a_pattern_overlay() {
        for depth in [8, 16, 32] {
            let mut tile = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Rgb, photocraft_color::SampleType::U8, true));
            tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
            let pat = photocraft_doc::Pattern::new("green", tile, 2, 2);
            let overlay = photocraft_doc::Effect::PatternOverlay {
                common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                name: pat.name.clone(),
                id: pat.id.clone(),
                scale: 1.0,
                angle: 0.0,
                link: true,
                phase: (0.0, 0.0),
            };
            assert_flatten_effects_is_a_no_op(depth, |d| d.patterns.push(pat), overlay);
        }
    }

    #[test]
    fn layers_to_files_can_trim_transparent_margins_without_changing_the_document() {
        let dir = tmp("layers-trim");
        for depth in [8, 16, 32] {
            let mut s = session(12, 11, depth);
            s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
            s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();

            let before = doc(&s).clone();
            let regular = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("full-{depth}"), "format": "png"})).unwrap();
            let full = std::fs::read(regular["files"][0].as_str().unwrap()).unwrap();
            let full = photocraft_io::import("full.png", &full).unwrap().document;
            assert_eq!((full.size.width, full.size.height), (12, 11), "default is unchanged");

            let trimmed =
                s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("trim-{depth}"), "format": "png", "trim": true})).unwrap();
            let image = std::fs::read(trimmed["files"][0].as_str().unwrap()).unwrap();
            let cropped = photocraft_io::import("trim.png", &image).unwrap().document;
            assert_eq!((cropped.size.width, cropped.size.height), (4, 5), "depth {depth}");
            let px = photocraft_compose::render(&cropped, Rect::new(0, 0, 1, 1)).px[0];
            assert!(px[0] > 0.95 && px[1] < 0.05 && px[2] < 0.05, "depth {depth}: {px:?}");
            assert_eq!(*doc(&s), before, "export must not crop the original document");
        }
    }

    #[test]
    fn streamed_layer_export_respects_trim() {
        let mut s = session(12, 11, 8);
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let mut encoded = Vec::new();
        let names = photocraft_engine::file_cmds::export_layers_with(&s, &json!({"trim": true, "format": "png"}), |name, bytes| {
            encoded.push((name.to_string(), bytes.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(names.len(), encoded.len());
        let cropped = photocraft_io::import(&encoded[0].0, &encoded[0].1).unwrap().document;
        assert_eq!((cropped.size.width, cropped.size.height), (4, 5));
    }
}
