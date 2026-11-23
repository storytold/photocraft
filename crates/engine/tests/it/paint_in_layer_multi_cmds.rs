//! Tests that run paint commands, moved from the engine-core `layer_multi_cmds` tests: 12 test(s).

mod tests {
    use photocraft_doc::LayerId;
    use photocraft_engine::Session;
    use photocraft_geom::Rect;
    use serde_json::json;

    /// #7538: `selection: true` moves the selection with the layers in the same step, as the Move
    /// tool does with several layers selected; without it the selection stays.
    #[test]
    fn translate_can_carry_the_selection_in_the_same_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100, "background": "transparent"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 70, "height": 30})).unwrap();
        let sel = |s: &Session| s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds());
        let bounds = |s: &Session, id: LayerId| s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
        let steps = s.active().unwrap().history.past_len();
        s.execute("layer.translate", json!({"dx": 7, "dy": 20, "selection": true})).unwrap();
        assert_eq!((bounds(&s, a), bounds(&s, b)), (Rect::new(17, 30, 37, 50), Rect::new(57, 30, 77, 50)));
        assert_eq!(sel(&s), Some(Rect::new(12, 25, 82, 55)), "the selection came along");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
        s.undo();
        assert_eq!((sel(&s), bounds(&s, a)), (Some(Rect::new(5, 5, 75, 35)), Rect::new(10, 10, 30, 30)), "undone together");
        s.execute("layer.translate", json!({"dx": 7, "dy": 20})).unwrap();
        assert_eq!(sel(&s), Some(Rect::new(5, 5, 75, 35)), "without it the selection stays");
        let state = |s: &Session| (bounds(s, a), bounds(s, b), sel(s), s.active().unwrap().history.past_len());
        let before = state(&s);
        for dx in [1, 0] {
            assert!(s.execute("layer.translate", json!({"dx": dx, "selection": "yes"})).is_err(), "dx {dx}: a bad `selection` is an error");
            assert_eq!(state(&s), before, "dx {dx}: and changes nothing");
        }
    }
}

mod tile_damage_tests {
    use photocraft_engine::layer_multi_cmds::same_pixels;

    use photocraft_doc::{Document, LayerId};
    use photocraft_engine::Session;
    use photocraft_geom::Rect;
    use serde_json::json;

    use photocraft_engine::actions_cmds::ActionsExt;

    use photocraft_geom::TILE_SIZE;

    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h})).unwrap();
        s
    }

    fn doc(s: &Session) -> std::sync::Arc<Document> {
        s.active().unwrap().doc.clone()
    }

    fn damage(s: &Session) -> Option<Rect> {
        s.active().unwrap().last_damage
    }

    /// The composite of `after` equals `before`'s outside `damage`.
    fn unchanged_outside(before: &Document, after: &Document, damage: Rect) -> bool {
        let (x, y) = (photocraft_compose::render(before, before.bounds()), photocraft_compose::render(after, after.bounds()));
        let w = before.bounds().width() as i32;
        x.px.iter().zip(&y.px).enumerate().all(|(i, (p, q))| p == q || damage.contains(i as i32 % w, i as i32 / w))
    }

    /// The tiles `r` overlaps, as one rect.
    fn tiles_of(r: Rect) -> Rect {
        let down = |v: i32| v.div_euclid(TILE_SIZE) * TILE_SIZE;
        Rect::new(down(r.x0), down(r.y0), down(r.x1 - 1) + TILE_SIZE, down(r.y1 - 1) + TILE_SIZE)
    }

    /// A new pixel layer with `rects` filled opaque, through one plain edit.
    fn layer_with(s: &mut Session, rects: &[(Rect, [f32; 4])]) -> LayerId {
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.edit("paint", |doc, _| {
            let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            for (r, px) in rects {
                surf.fill_rect(*r, px);
            }
            Ok(())
        })
        .unwrap();
        id
    }

    fn full_layer(s: &mut Session) -> LayerId {
        let canvas = doc(s).bounds();
        layer_with(s, &[(canvas, [0.4, 0.5, 0.6, 1.0])])
    }

    #[test]
    fn a_fill_on_a_selection_damages_only_its_tiles() {
        let mut s = session(1024, 768);
        full_layer(&mut s);
        let sel = Rect::new(300, 300, 440, 410);
        s.execute("select.rect", json!({"x": sel.x0, "y": sel.y0, "width": sel.width(), "height": sel.height()})).unwrap();
        let before = doc(&s);
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let d = damage(&s).expect("a bounded fill");
        assert!(!d.is_empty() && tiles_of(sel).contains_rect(&d), "{d:?}");
        assert!(d.size().area() < doc(&s).bounds().size().area() / 4, "{d:?}");
        assert!(unchanged_outside(&before, &doc(&s), d));
    }

    #[test]
    fn undo_of_a_brush_dab_on_a_full_layer_redraws_only_its_tiles() {
        let mut s = session(1024, 768);
        full_layer(&mut s);
        let four_tiles = 4 * (TILE_SIZE as u64).pow(2);
        let before = doc(&s);
        s.execute("paint.stroke", json!({"points": [[500, 400]], "size": 60, "color": "#ff0000"})).unwrap();
        let painted = doc(&s);
        let d = damage(&s).expect("the dab");
        assert!(!d.is_empty() && d.size().area() <= four_tiles, "dab {d:?}");
        assert!(unchanged_outside(&before, &painted, d));
        assert!(s.undo());
        let d = damage(&s).expect("undo is bounded");
        assert!(!d.is_empty() && d.size().area() <= four_tiles, "undo {d:?}");
        assert!(unchanged_outside(&painted, &doc(&s), d));
        assert!(s.redo());
        let d = damage(&s).expect("redo is bounded");
        assert!(!d.is_empty() && d.size().area() <= four_tiles, "redo {d:?}");
        assert!(unchanged_outside(&before, &doc(&s), d));
    }

    /// The default drop shadow, `distance` px away and `size` px soft.
    fn drop_shadow(distance: f32, size: f32) -> photocraft_doc::Effect {
        match photocraft_doc::Effect::default_drop_shadow() {
            photocraft_doc::Effect::DropShadow(sh) => photocraft_doc::Effect::DropShadow(photocraft_doc::Shadow { distance, size, ..sh }),
            e => e,
        }
    }

    /// A styled layer inside a styled group: the group's shadow is cast by the child's shadow too, so
    /// a dab's change reaches as far as both shadows together, not just the farther one.
    #[test]
    fn a_dab_inside_a_group_with_a_drop_shadow_damages_the_shadow() {
        let mut s = session(1024, 768);
        let child = layer_with(&mut s, &[(Rect::new(200, 400, 300, 500), [1.0, 0.0, 0.0, 1.0])]);
        s.execute("layer.select", json!({"layer": child.0})).unwrap();
        let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.edit("fx", |doc, _| {
            for id in [child, g] {
                doc.layer_mut(id).unwrap().effects.items.push(drop_shadow(40.0, 2.0));
            }
            Ok(())
        })
        .unwrap();
        assert!(photocraft_compose::effects::margin(doc(&s).layer(g).unwrap()) > 40);
        // A brush dab (a hinted edit) in empty space left of a tile's right edge. Each shadow falls
        // about 20 px right and 35 px down, so the group's shadow of the child's shadow lands about
        // 40 px right and 70 px down, in the next tile and past either shadow's reach alone.
        s.execute("layer.select", json!({"layer": child.0})).unwrap();
        let before = doc(&s);
        s.execute("paint.stroke", json!({"points": [[TILE_SIZE - 30, 300]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": child.0})).unwrap();
        assert_eq!(s.damage_misses, 0);
        let d = damage(&s).expect("a bounded dab");
        assert!(d.x0 < TILE_SIZE && d.x1 > TILE_SIZE, "{d:?}");
        assert!(unchanged_outside(&before, &doc(&s), d), "the group's shadow changed outside {d:?}");
    }

    /// A layer the compositor doesn't draw adds nothing to a dab elsewhere: neither a hidden layer's
    /// effects nor those of layers inside a hidden group grow the hint.
    #[test]
    fn a_brush_hint_ignores_the_effects_of_hidden_layers() {
        let mut s = session(1024, 768);
        let plain = full_layer(&mut s);
        let hidden = layer_with(&mut s, &[(Rect::new(800, 600, 900, 700), [1.0, 0.0, 0.0, 1.0])]);
        let child = layer_with(&mut s, &[(Rect::new(50, 600, 150, 700), [0.0, 1.0, 0.0, 1.0])]);
        s.execute("layer.select", json!({"layer": child.0})).unwrap();
        let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.edit("fx", |doc, _| {
            for id in [hidden, child] {
                let l = doc.layer_mut(id).unwrap();
                l.effects.items.push(drop_shadow(40.0, 40.0));
            }
            doc.layer_mut(hidden).unwrap().visible = false;
            doc.layer_mut(g).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        assert!(photocraft_compose::effects::margin(doc(&s).layer(hidden).unwrap()) > 40);
        // A dab in the middle of a tile, far from every tile edge.
        s.execute("layer.select", json!({"layer": plain.0})).unwrap();
        let before = doc(&s);
        s.execute("paint.stroke", json!({"points": [[384, 384]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": plain.0})).unwrap();
        assert_eq!(s.damage_misses, 0);
        let d = damage(&s).expect("a bounded dab");
        assert!(d.contains(384, 384) && d.width() <= 20 && d.height() <= 20, "grown by a hidden layer: {d:?}");
        assert!(unchanged_outside(&before, &doc(&s), d));
    }

    /// A brush hint is the bare dab; on a feathered mask the dab changes a soft ring around it, and
    /// the hint must not cut that off.
    #[test]
    fn a_brush_hint_on_a_feathered_mask_keeps_the_ring() {
        let mut s = session(1024, 768);
        let id = full_layer(&mut s);
        s.edit("mask", |doc, _| {
            doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask { feather: 10.0, ..photocraft_doc::LayerMask::reveal_all() });
            Ok(())
        })
        .unwrap();
        let before = doc(&s);
        s.execute("paint.stroke", json!({"points": [[500, 400]], "size": 10, "hardness": 1.0, "color": "#000000", "target": "mask", "layer": id.0})).unwrap();
        assert_eq!(s.damage_misses, 0);
        let d = damage(&s).expect("a bounded dab");
        assert!(unchanged_outside(&before, &doc(&s), d), "the feathered mask changed outside {d:?}");
    }

    /// The same for a layer's effects: the dab's drop shadow lands outside the dab.
    #[test]
    fn a_brush_hint_on_a_layer_with_a_drop_shadow_keeps_the_shadow() {
        let mut s = session(1024, 768);
        let id = layer_with(&mut s, &[(Rect::new(600, 400, 700, 500), [1.0, 0.0, 0.0, 1.0])]);
        s.edit("fx", |doc, _| {
            doc.layer_mut(id).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
            Ok(())
        })
        .unwrap();
        let before = doc(&s);
        s.execute("paint.stroke", json!({"points": [[300, 300]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": id.0})).unwrap();
        assert_eq!(s.damage_misses, 0);
        let d = damage(&s).expect("a bounded dab");
        assert!(unchanged_outside(&before, &doc(&s), d), "the shadow changed outside {d:?}");
    }

    #[test]
    fn painting_command_hints_cover_their_changed_tiles() {
        let mut s = session(600, 400);
        let mut rects = Vec::new();
        for by in 0..20 {
            for bx in 0..30 {
                let v = ((bx * 7 + by * 13) % 17) as f32 / 17.0;
                rects.push((Rect::from_xywh(bx * 20, by * 20, 20, 20), [v, 1.0 - v, 0.5 * v + 0.2, 1.0]));
            }
        }
        rects.push((Rect::new(430, 280, 470, 320), [0.1, 0.1, 0.1, 1.0]));
        for y in 292..309 {
            for x in 442..459 {
                if (x - 450) * (x - 450) + (y - 300) * (y - 300) <= 64 {
                    rects.push((Rect::from_xywh(x, y, 1, 1), [0.9, 0.05, 0.05, 1.0]));
                }
            }
        }
        layer_with(&mut s, &rects);
        let ty = s.execute("type.create", json!({"x": 40, "y": 380, "text": "Hi", "size": 20})).unwrap()["layer"].as_u64().unwrap();
        let mut steps: Vec<(&str, serde_json::Value, Option<Rect>)> = vec![
            ("paint.stroke", json!({"points": [[100, 100], [300, 120]], "size": 20, "color": "#2040ff"}), None),
            ("paint.backgroundEraser", json!({"points": [[120, 300], [200, 320]], "size": 20}), None),
            ("paint.cloneStamp", json!({"points": [[400, 100]], "offset": [-100, 0], "size": 30, "hardness": 100}), None),
            ("paint.healingBrush", json!({"points": [[420, 200]], "source": [300, 200], "size": 30, "hardness": 100}), None),
            ("paint.redEye", json!({"x": 450, "y": 300}), None),
            ("paint.patch", json!({"offset": [60, 0]}), Some(Rect::new(60, 250, 100, 290))),
            ("paint.remove", json!({"points": [[500, 100], [540, 140]], "size": 24}), None),
            ("paint.contentAwareMove", json!({"offset": [100, -50], "structure": 3, "color": 4}), Some(Rect::new(300, 300, 340, 340))),
            (
                "paint.patternStamp",
                json!({"points": [[200, 200]], "size": 16, "hardness": 100, "pattern": "Checkerboard", "scale": 100, "aligned": true}),
                None,
            ),
        ];
        steps.push(("type.setStyle", json!({"layer": ty, "size": 40}), None));
        for (cmd, p, sel) in steps {
            if let Some(r) = sel {
                s.execute("select.rect", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()})).unwrap();
            } else if doc(&s).selection.is_some() {
                s.execute("select.deselect", json!({})).unwrap();
            }
            if cmd != "type.setStyle" {
                let layer = doc(&s).layers.iter().find(|l| l.surface().is_some() && !doc(&s).is_background(l.id)).unwrap().id;
                s.select_layer(layer).unwrap();
            }
            let before = doc(&s);
            s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
            assert_eq!(s.damage_misses, 0, "{cmd}: its hint missed a changed tile");
            assert!(!same_pixels(&before, &doc(&s)), "{cmd} changed nothing");
            let d = damage(&s).unwrap_or_else(|| doc(&s).bounds());
            assert!(unchanged_outside(&before, &doc(&s), d), "{cmd}: pixels changed outside {d:?}");
        }
    }

    // PR C (#9329): damage spans a command, not a revision.

    /// The damage since the command began: measured from the revision before it, bounded, and
    /// covering every pixel the command changed.
    fn command_damage(s: &mut Session, cmd: &str, p: serde_json::Value) -> (std::sync::Arc<Document>, Rect) {
        let (before, rev) = (doc(s), s.active().unwrap().revision);
        s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        let st = s.active().unwrap();
        assert!(st.revision - rev >= 2, "{cmd}: several revisions ({} → {})", rev, st.revision);
        assert_eq!(st.damage_from(), rev, "{cmd}: measured from before the command");
        let d = st.last_damage.unwrap_or_else(|| panic!("{cmd}: bounded damage"));
        assert!(unchanged_outside(&before, &st.doc, d), "{cmd}: pixels changed outside {d:?}");
        (before, d)
    }

    #[test]
    fn stroke_path_damage_spans_every_subpath() {
        let mut s = session(1024, 768);
        layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.5, 0.5, 0.5, 1.0])]);
        let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20], [80, 80]]}, {"knots": [[800, 600], [900, 600], [900, 700]]}]});
        s.execute("path.set", json!({ "path": path })).unwrap();
        // A tool with its own `paint.*` command runs once per subpath, one revision each.
        let (_, d) = command_damage(&mut s, "path.stroke", json!({"tool": "burn", "size": 16}));
        assert!(d.contains(50, 20) && d.contains(850, 600), "both subpaths: {d:?}");
        assert_eq!(s.whole_redraws, 0);
    }

    /// Pattern Stamp copies its pattern into the document on first use: a pattern no layer
    /// refers to draws nothing, so a multi-subpath Stroke Path stays bounded.
    #[test]
    fn a_new_unreferenced_pattern_keeps_stroke_path_bounded() {
        let mut s = session(1024, 768);
        layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.5, 0.5, 0.5, 1.0])]);
        let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20], [80, 80]]}, {"knots": [[800, 600], [900, 600], [900, 700]]}]});
        s.execute("path.set", json!({ "path": path })).unwrap();
        let n = doc(&s).patterns.len();
        let p = json!({"tool": "patternStamp", "size": 16, "hardness": 100, "pattern": "Checkerboard", "scale": 100, "aligned": true});
        let (_, d) = command_damage(&mut s, "path.stroke", p);
        assert!(doc(&s).patterns.len() > n, "the stamp added its pattern");
        assert!(d.contains(50, 20) && d.contains(850, 600), "both subpaths: {d:?}");
        assert_eq!(s.whole_redraws, 0);
    }

    #[test]
    fn select_then_edit_in_one_command_is_bounded() {
        let mut s = session(1024, 768);
        let id = layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0])]);
        use photocraft_engine::actions_cmds::ActionStep;
        let steps = vec![
            ActionStep::new("layer.select", json!({"layer": id.0})),
            ActionStep::new("select.rect", json!({"x": 300, "y": 300, "width": 50, "height": 40})),
            ActionStep::new("edit.fill", json!({"color": "#ff0000"})),
            ActionStep::new("select.deselect", json!({})),
        ];
        s.actions_mut().list.push(photocraft_engine::actions_cmds::Action::new("A", steps)).unwrap();
        let (_, d) = command_damage(&mut s, "actions.play", json!({"action": "A"}));
        assert!(d.contains(300, 300) && d.contains(349, 339), "{d:?}");
        assert!(u64::from(d.width()) * u64::from(d.height()) * 4 < 1024 * 768, "the filled tiles, not the canvas: {d:?}");
        assert_eq!(s.whole_redraws, 0);
    }

    #[test]
    fn no_command_redraws_the_whole_document_for_a_small_edit() {
        let mut s = session(1024, 768);
        let base = layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0])]);
        let small = layer_with(&mut s, &[(Rect::new(100, 100, 160, 160), [0.9, 0.1, 0.1, 1.0])]);
        let other = layer_with(&mut s, &[(Rect::new(130, 130, 190, 190), [0.1, 0.9, 0.1, 1.0])]);
        let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20]]}, {"knots": [[800, 600], [900, 600]]}]});
        let steps: Vec<(&str, serde_json::Value)> = vec![
            ("layer.select", json!({"layer": base.0})),
            ("select.rect", json!({"x": 300, "y": 300, "width": 50, "height": 40})),
            ("edit.fill", json!({"color": "#ff0000"})),
            ("filter.blur.gaussianBlur", json!({"radius": 2})),
            ("edit.copy", json!({})),
            ("select.rect", json!({"x": 600, "y": 400, "width": 80, "height": 60})),
            ("edit.pasteSpecial.pasteInto", json!({})),
            ("edit.undo", json!({})),
            ("edit.redo", json!({})),
            ("layer.select", json!({"layer": base.0})),
            ("paint.stroke", json!({"points": [[100, 400], [200, 420]], "size": 20, "color": "#2040ff"})),
            ("edit.undo", json!({})),
            ("edit.redo", json!({})),
            ("path.set", json!({ "path": path })),
            ("path.stroke", json!({"tool": "pencil", "size": 4, "color": "#0000ff", "opacity": 100})),
            ("path.stroke", json!({"tool": "burn", "size": 16})),
            ("layer.select", json!({"layer": small.0})),
            ("layer.setProps", json!({"layer": small.0, "opacity": 0.5})),
            ("layer.setProps", json!({"layer": small.0, "visible": false})),
            ("layer.setProps", json!({"layer": small.0, "visible": true})),
            ("layer.translate", json!({"layer": small.0, "dx": 20, "dy": 10})),
            // Show Only hides the full-canvas base: a large change, so not counted either way.
            ("layer.showOnly", json!({"layer": small.0})),
            ("layer.showOnly", json!({"layer": small.0})),
            ("layer.arrange.bringForward", json!({})),
            ("layer.arrange.sendBackward", json!({})),
            ("layer.setProps", json!({"layer": other.0, "clipped": true})),
            ("layer.new.layer", json!({})),
        ];
        for (cmd, p) in steps {
            s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
            assert_eq!(s.whole_redraws, 0, "{cmd}: redrew the whole document for a small edit");
        }
    }
}
