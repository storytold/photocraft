use super::*;

/// 80×60 document, a new layer with a patterned 50×30 block at (10, 10).
fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let surf = l.surface_mut().unwrap();
        for y in 10..40 {
            for x in 10..60 {
                let v = ((x / 3 + y / 3) % 2) as f32;
                surf.write_pixel(x, y, &[v, 1.0 - v, (x as f32) / 80.0, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    s
}

fn active_surface(s: &Session) -> Surface {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

fn worst(a: &Surface, b: &Surface, r: Rect) -> f32 {
    a.read_region(r).iter().zip(b.read_region(r)).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn smart(s: &Session) -> photocraft_doc::SmartObject {
    let st = s.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not smart"),
    }
}

#[test]
fn liquify_identity_stroke_displacement_reconstruct_and_undo() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let before = active_surface(&s);
        let r = s.execute(LIQUIFY, json!({})).unwrap();
        assert_eq!(r["changed"], false);
        let r = s.execute(LIQUIFY, json!({"strokes": [{"tool": "forwardWarp", "size": 30, "pressure": 0, "points": [[30, 25], [45, 25]]}]})).unwrap();
        assert_eq!(r["changed"], false, "zero pressure is a no-op");
        let hist = s.active().unwrap().history.past_len();
        let stroke = json!({"tool": "forwardWarp", "size": 30, "points": [[30, 25], [42, 25]]});
        let r = s.execute(LIQUIFY, json!({"strokes": [stroke]})).unwrap();
        assert_eq!(r["changed"], true);
        assert!(r["maxDisplacement"].as_f64().unwrap() > 4.0);
        assert_eq!(s.active().unwrap().history.past_len(), hist + 1, "one history step");
        let after = active_surface(&s);
        // The pattern moved right: the output at (42, 25) shows what was a few px to the left.
        let d = (0..12).map(|k| worst_px(&after.pixel(42, 25), &before.pixel(42 - k, 25))).enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        assert!(d >= 3, "{depth}: displaced by {d}");
        assert_ne!(after, before);
        // Reconstruct All in the same stroke list restores the original.
        let mut s2 = session(depth);
        s2.execute(LIQUIFY, json!({"strokes": [stroke, {"tool": "reconstructAll", "amount": 100}]})).unwrap();
        assert!(worst(&active_surface(&s2), &before, Rect::new(0, 0, 80, 60)) <= 1.0 / 255.0);
        s.undo();
        assert_eq!(active_surface(&s), before);
        assert!(s.execute(LIQUIFY, json!({"strokes": [{"tool": "melt", "points": [[1, 1]]}]})).is_err());
        assert!(s.execute(LIQUIFY, json!({"strokes": [{"points": [[1]]}]})).is_err());
        assert!(s.execute(LIQUIFY, json!({"strokes": [], "meshSize": 0})).is_err());
    }
}

fn worst_px(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn liquify_respects_the_selection() {
    let mut s = session(8);
    let before = active_surface(&s);
    s.execute("select.all", json!({})).ok();
    s.edit("sel", |doc, _| {
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 35, 60), &[1.0]);
        doc.selection = Some(sel);
        Ok(())
    })
    .unwrap();
    s.execute(LIQUIFY, json!({"strokes": [{"tool": "bloat", "size": 60, "points": [[35, 25], [35, 25], [35, 25], [35, 25]]}]})).unwrap();
    let after = active_surface(&s);
    assert_eq!(after.read_region(Rect::new(36, 0, 80, 60)), before.read_region(Rect::new(36, 0, 80, 60)), "unselected pixels stay");
    assert_ne!(after.read_region(Rect::new(10, 10, 35, 40)), before.read_region(Rect::new(10, 10, 35, 40)));
}

#[test]
fn liquify_elapsed_time_is_wasm_safe() {
    let mut s = session(8);
    let r = s.execute(LIQUIFY, json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[30, 25], [42, 25]]}]})).unwrap();
    assert_eq!(r["changed"], true);
    let ms = r["ms"].as_f64().unwrap();
    assert!(ms.is_finite() && ms >= 0.0);
    #[cfg(target_arch = "wasm32")]
    assert_eq!(ms, 0.0, "the web build uses the wasm-safe stopwatch fallback");
}

#[test]
fn puppet_identity_translation_and_undo() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let before = active_surface(&s);
        assert_eq!(s.execute(PUPPET, json!({})).unwrap()["changed"], false);
        assert_eq!(s.execute(PUPPET, json!({"pins": [{"src": [20, 20], "dst": [20, 20]}]})).unwrap()["changed"], false);
        s.execute(PUPPET, json!({"pins": [{"src": [30, 25], "dst": [36, 29]}], "mode": "rigid", "interpolation": "bilinear"})).unwrap();
        let after = active_surface(&s);
        assert_eq!(after.content_bounds(), Rect::new(16, 14, 66, 44), "{depth}");
        assert!(
            worst(&after.convert(before.format()), &photocraft_algo::resample::translate_surface(&before, 6, 4), Rect::new(16, 14, 66, 44)) <= 1.0 / 255.0,
            "{depth}"
        );
        s.undo();
        assert_eq!(active_surface(&s), before);
        assert!(s.execute(PUPPET, json!({"pins": [{"src": [1, 1]}]})).is_err());
        assert!(s.execute(PUPPET, json!({"pins": [], "mode": "wobbly"})).is_err());
        assert!(s.execute(PUPPET, json!({"pins": [], "density": "lots"})).is_err());
    }
}

fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
}

/// [`session`] with a reveal-all mask that hides the stripes `stripes` (x ranges, y 10..40).
fn masked_session(depth: u64, stripes: &[(i32, i32)]) -> Session {
    let mut s = session(depth);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.edit("mask", |doc, active| {
        let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
        for &(x0, x1) in stripes {
            m.surface.fill_rect(Rect::new(x0, 10, x1, 40), &[0.0]);
        }
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn puppet_moves_a_linked_mask_with_the_pixels() {
    // #4910: Puppet Warp warped only the pixels and left a linked mask behind. A one-pin rigid
    // warp moves the block by (6, 4); the stripe the mask hides (x 20..24) must move with it.
    for depth in [8u64, 16, 32] {
        let mut s = masked_session(depth, &[(20, 24)]);
        let before = s.active().unwrap().doc.clone();
        s.execute(PUPPET, json!({"pins": [{"src": [30, 25], "dst": [36, 29]}], "mode": "rigid", "interpolation": "bilinear"})).unwrap();
        for y in [16, 30, 42] {
            assert!(mask_at(&s, 27, y) < 0.01, "{depth}: the stripe moved to x 26..30 (y {y})");
            assert!(mask_at(&s, 21, y) > 0.99, "{depth}: and left x 20..24 (y {y})");
        }
        assert!(mask_at(&s, 27, 12) > 0.99, "{depth}: and down by 4");
        // Pixels and mask still line up: the same layer pixel is hidden as before the warp.
        let was = before.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().pixel(21, 26);
        assert!(worst_px(&active_surface(&s).pixel(27, 30), &was) <= 1.0 / 255.0, "{depth}");
        s.undo();
        assert!(mask_at(&s, 21, 30) < 0.01 && mask_at(&s, 27, 30) > 0.99, "{depth}: undo puts the mask back");
    }
}

#[test]
fn puppet_leaves_an_unlinked_mask_in_place() {
    let mut s = masked_session(8, &[(20, 24)]);
    s.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
    s.execute(PUPPET, json!({"pins": [{"src": [30, 25], "dst": [36, 29]}], "mode": "rigid"})).unwrap();
    assert!(mask_at(&s, 21, 30) < 0.01 && mask_at(&s, 27, 30) > 0.99);
    assert_eq!(active_surface(&s).content_bounds(), Rect::new(16, 14, 66, 44), "the pixels still moved");
}

#[test]
fn puppet_with_a_selection_moves_only_the_selected_part_of_a_linked_mask() {
    // The selection lifts the block's left part (x 10..30): the stripe hidden there moves with
    // it, the one outside (x 40..44) stays over its pixels.
    let mut s = masked_session(8, &[(20, 24), (40, 44)]);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 60})).unwrap();
    s.execute(PUPPET, json!({"pins": [{"src": [20, 25], "dst": [26, 29]}], "mode": "rigid"})).unwrap();
    assert!(mask_at(&s, 27, 30) < 0.01 && mask_at(&s, 21, 30) > 0.99, "the selected stripe moved");
    assert!(mask_at(&s, 41, 30) < 0.01 && mask_at(&s, 45, 30) > 0.99, "the other stayed");
    assert_eq!(active_surface(&s).pixel(41, 30)[3], 1.0, "over its pixels");
}

#[test]
fn puppet_with_an_expansion_that_empties_the_mesh_keeps_the_layer() {
    // #951: a pinned warp over an empty mesh panicked in the solver; it must not erase the layer
    // either (the rasterizer keeps only pixels inside the mesh).
    let mut s = session(8);
    let before = active_surface(&s);
    s.execute(PUPPET, json!({"pins": [{"src": [30, 25], "dst": [36, 29]}], "expansion": -200})).unwrap();
    assert_eq!(active_surface(&s).read_region(Rect::new(0, 0, 80, 60)), before.read_region(Rect::new(0, 0, 80, 60)));
}

#[test]
fn puppet_refuses_pins_that_stretch_the_content_too_far() {
    // #1019: a pin dragged to (2^20, 2^20) stretched the mesh over ~17.6 million tiles, and the
    // renderer allocated bookkeeping for every one before drawing anything.
    let far = json!({"pins": [{"src": [20, 20], "dst": [20, 20]}, {"src": [50, 30], "dst": [1048576, 1048576]}]});
    let full = Rect::new(0, 0, 80, 60);
    for depth in [8u64, 16, 32] {
        for selected in [false, true] {
            let mut s = session(depth);
            if selected {
                s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
            }
            let (before, past) = (active_surface(&s), s.active().unwrap().history.past_len());
            let err = s.execute(PUPPET, far.clone()).unwrap_err();
            assert!(err.to_string().contains("too far"), "{depth} {selected}: {err}");
            assert_eq!(active_surface(&s).read_region(full), before.read_region(full), "{depth} {selected}: layer unchanged");
            assert_eq!(s.active().unwrap().history.past_len(), past, "{depth} {selected}: no history step");
        }
        // Controls: a rigid move still moves the content, and a pin 4000 px out (well inside the
        // budget) still warps.
        let mut s = session(depth);
        s.execute(PUPPET, json!({"pins": [{"src": [20, 20], "dst": [30, 25]}, {"src": [50, 30], "dst": [60, 35]}], "mode": "rigid"})).unwrap();
        assert_eq!(active_surface(&s).content_bounds(), Rect::new(20, 15, 70, 45), "{depth}");
        let mut s = session(depth);
        s.execute(PUPPET, json!({"pins": [{"src": [20, 20], "dst": [20, 20]}, {"src": [50, 30], "dst": [4050, 4030]}]})).unwrap();
        assert!(active_surface(&s).content_bounds().x1 > 1000, "{depth}");
    }
    // A smart object stores the filter; a warp too far to render leaves its pixels as they were.
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let before = active_surface(&s);
    s.execute(PUPPET_SMART, far).unwrap();
    assert_eq!(smart(&s).smart_filters.len(), 1);
    assert_eq!(active_surface(&s), before);
}

#[test]
fn puppet_two_pins_bend_the_block() {
    let mut s = session(8);
    // Hold the left end, lift the right end: the right side rises, the left stays.
    s.execute(PUPPET, json!({"pins": [{"src": [15, 25], "dst": [15, 25]}, {"src": [55, 25], "dst": [55, 10]}], "mode": "normal", "density": "more"})).unwrap();
    let after = active_surface(&s);
    assert!(after.pixel(12, 25)[3] > 0.5, "the held end stays");
    assert!(after.content_bounds().y0 < 6, "{:?}", after.content_bounds());
}

#[test]
fn puppet_mesh_follows_the_outline_so_separate_shapes_move_separately() {
    // #3105: the mesh was every grid cell touching opaque pixels, so two bars 24 px apart (more
    // than a mesh cell at Fewer Points) shared cells and a pin on one dragged the other along.
    // The mesh now follows the outline grown by the expansion: two meshes, and the unpinned one
    // stays put.
    for depth in [8u64, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 260, "height": 140, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for (y0, y1) in [(20, 60), (84, 120)] {
                for y in y0..y1 {
                    for x in 10..250 {
                        let v = ((x / 4 + y / 4) % 2) as f32;
                        surf.write_pixel(x, y, &[v, 1.0 - v, (y as f32) / 140.0, 1.0]);
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        let before = active_surface(&s);
        s.execute(PUPPET, json!({"pins": [{"src": [130, 40], "dst": [130, 30]}], "density": "fewer", "expansion": 2, "interpolation": "bilinear"})).unwrap();
        let after = active_surface(&s).convert(before.format());
        let lower = Rect::new(0, 70, 260, 140);
        let still = worst(&after, &before, lower);
        assert!(still <= 1.0 / 255.0, "{depth}: the unpinned bar stays ({still})");
        assert!(
            worst(&after, &photocraft_algo::resample::translate_surface(&before, 0, -10), Rect::new(0, 0, 260, 60)) <= 1.0 / 255.0,
            "{depth}: the pinned bar moves"
        );
    }
}

#[test]
fn perspective_identity_known_homography_and_mask() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let before = active_surface(&s);
        let q = [[10.0, 10.0], [60.0, 10.0], [60.0, 40.0], [10.0, 40.0]];
        let r = s.execute(PERSPECTIVE, json!({"planes": [{"src": q, "dst": q}]})).unwrap();
        assert_eq!(r["changed"], false);
        // Shift the whole quad by (5, 3): a pure translation of the layer.
        let d = q.map(|p| [p[0] + 5.0, p[1] + 3.0]);
        s.execute(PERSPECTIVE, json!({"planes": [{"src": q, "dst": d}], "interpolation": "bilinear"})).unwrap();
        let after = active_surface(&s);
        assert_eq!(after.content_bounds(), Rect::new(15, 13, 65, 43));
        assert!(
            worst(&after.convert(before.format()), &photocraft_algo::resample::translate_surface(&before, 5, 3), Rect::new(15, 13, 65, 43)) <= 1.0 / 255.0,
            "{depth}"
        );
        s.undo();
        assert_eq!(active_surface(&s), before);
    }
    // A keystone: corners land where asked.
    let mut s = session(8);
    let q = [[10.0, 10.0], [60.0, 10.0], [60.0, 40.0], [10.0, 40.0]];
    let d = [[20.0, 10.0], [50.0, 10.0], [70.0, 50.0], [0.0, 50.0]];
    s.execute("layer.layerMask.revealAll", json!({})).ok();
    s.execute(PERSPECTIVE, json!({"planes": [{"src": q, "dst": d}]})).unwrap();
    let after = active_surface(&s);
    let b = after.content_bounds();
    assert!(b.x0 <= 1 && b.x1 >= 69 && b.y1 >= 49 && b.y0 >= 9, "{b:?}");
    assert!(after.pixel(15, 12)[3] < 0.1, "the top narrowed");
    assert!(s.execute(PERSPECTIVE, json!({})).is_err());
    assert!(s.execute(PERSPECTIVE, json!({"planes": [{"src": [[0, 0], [0, 0], [0, 0], [0, 0]], "dst": [[0, 0], [0, 0], [0, 0], [0, 0]]}]})).is_err());
    assert!(s.execute(PERSPECTIVE, json!({"planes": [{"src": q, "dst": d}], "straighten": "sideways"})).is_err());
}

#[test]
fn perspective_with_a_selection_moves_only_the_selected_part_of_a_linked_mask() {
    // #3083: the mask hides a stripe inside the selection (x 12..16) and one outside it (x 40..44).
    let mask_at = |s: &Session, x, y| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
    };
    for selected in [true, false] {
        let mut s = session(8);
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.edit("mask", |doc, active| {
            let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(Rect::new(12, 10, 16, 40), &[0.0]);
            m.surface.fill_rect(Rect::new(40, 10, 44, 40), &[0.0]);
            Ok(())
        })
        .unwrap();
        if selected {
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 60})).unwrap();
        }
        let q = [[0.0, 0.0], [30.0, 0.0], [30.0, 60.0], [0.0, 60.0]];
        let d = q.map(|p| [p[0] + 4.0, p[1]]);
        s.execute(PERSPECTIVE, json!({"planes": [{"src": q, "dst": d}], "interpolation": "bilinear"})).unwrap();
        // The selected stripe moves with the pixels either way.
        assert!(mask_at(&s, 17, 25) < 0.01 && mask_at(&s, 13, 25) > 0.99, "selected={selected}");
        if selected {
            // The unselected stripe stays over the unselected pixels.
            assert!(mask_at(&s, 43, 25) < 0.01 && mask_at(&s, 47, 25) > 0.99);
            assert_eq!(active_surface(&s).pixel(43, 25)[3], 1.0);
        } else {
            // Without a selection the whole mask moves with the layer.
            assert!(mask_at(&s, 47, 25) < 0.01 && mask_at(&s, 43, 25) > 0.99);
        }
    }
}

#[test]
fn smart_objects_store_distortions_as_smart_filters_losslessly() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let original = active_surface(&s);
        assert!(s.is_enabled(PUPPET_SMART) && s.is_enabled(PERSPECTIVE_SMART));
        s.execute(LIQUIFY, json!({"strokes": [{"tool": "twirlCw", "size": 40, "points": [[35, 25], [35, 25], [35, 25]]}]})).unwrap();
        let liq = active_surface(&s);
        assert_ne!(liq, original);
        s.execute(PUPPET_SMART, json!({"pins": [{"src": [30, 25], "dst": [34, 25]}], "mode": "rigid"})).unwrap();
        let q = [[10.0, 10.0], [60.0, 10.0], [60.0, 40.0], [10.0, 40.0]];
        s.execute(PERSPECTIVE_SMART, json!({"planes": [{"src": q, "dst": [[12.0, 10.0], [58.0, 10.0], [60.0, 40.0], [10.0, 40.0]]}]})).unwrap();
        let sm = smart(&s);
        let ids: Vec<&str> = sm.smart_filters.iter().map(|f| f.command.as_str()).collect();
        assert_eq!(ids, [LIQUIFY, PUPPET_SMART, PERSPECTIVE_SMART]);
        // Deleting the top two filters re-renders exactly the liquified state; deleting all
        // gives back the original pixels (nothing accumulated).
        s.execute("layer.smartFilter.delete", json!({})).unwrap();
        s.execute("layer.smartFilter.delete", json!({})).unwrap();
        assert_eq!(active_surface(&s), liq, "{depth}");
        s.execute("layer.smartFilter.delete", json!({})).unwrap();
        assert_eq!(active_surface(&s), original, "{depth}");
        s.undo();
        assert_eq!(active_surface(&s), liq);
    }
}

#[test]
fn enabled_states() {
    let mut s = session(8);
    assert!(s.is_enabled(LIQUIFY) && s.is_enabled(PUPPET) && s.is_enabled(PERSPECTIVE));
    assert!(!s.is_enabled(PUPPET_SMART) && !s.is_enabled(PERSPECTIVE_SMART));
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    assert!(!s.is_enabled(PUPPET) && !s.is_enabled(PERSPECTIVE));
    let mut empty = Session::new();
    assert!(!empty.is_enabled(LIQUIFY));
    assert!(empty.execute(PUPPET, json!({"pins": []})).is_err());
}

#[test]
fn background_layer_is_floated_by_geometric_warps() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8})).unwrap();
    s.execute(PUPPET, json!({"pins": [{"src": [20, 15], "dst": [25, 15]}], "mode": "rigid"})).unwrap();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(l.name, "Layer 0");
    assert!(l.surface().unwrap().format().alpha);
    assert!(l.surface().unwrap().pixel(2, 15)[3] < 0.01, "exposed area is transparent");
}

#[test]
fn liquify_with_a_far_off_stroke_point_returns() {
    // #937: the issue's request walked ~3.3e14 dab steps along a segment to (1e15, 0).
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let r = s.execute(LIQUIFY, json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[0, 0], [1e15, 0]]}]})).unwrap();
        assert_eq!(r["changed"], true, "{depth}: the start of the stroke is on the canvas");
        let mut s = session(depth);
        let before = active_surface(&s);
        s.execute(LIQUIFY, json!({"strokes": [{"tool": "bloat", "size": 30, "points": [[5000, 5000], [1e15, 1e15]]}]})).unwrap();
        assert_eq!(active_surface(&s).read_region(Rect::new(0, 0, 80, 60)), before.read_region(Rect::new(0, 0, 80, 60)), "{depth}: off-canvas only");
    }
}

/// The three distortions on the active smart object, each with a visible change.
fn smart_distortions() -> [(&'static str, Value); 3] {
    let q = [[10.0, 10.0], [60.0, 10.0], [60.0, 40.0], [10.0, 40.0]];
    [
        (LIQUIFY, json!({"strokes": [{"tool": "twirlCw", "size": 40, "points": [[35, 25], [35, 25], [35, 25]]}]})),
        (PUPPET_SMART, json!({"pins": [{"src": [30, 25], "dst": [34, 25]}], "mode": "rigid"})),
        (PERSPECTIVE_SMART, json!({"planes": [{"src": q, "dst": [[12.0, 10.0], [58.0, 10.0], [60.0, 40.0], [10.0, 40.0]]}]})),
    ]
}

#[test]
fn locked_smart_objects_refuse_distortions() {
    // As filters do: a pixel or full lock (the layer's own or its group's) refuses the smart
    // filter, leaving the layer and the history as they were.
    for lock in [json!({"all": true}), json!({"pixels": true})] {
        for on_group in [false, true] {
            for (cmd, p) in smart_distortions() {
                let mut s = session(8);
                s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
                let layer = s.active().unwrap().active_layer.unwrap();
                if on_group {
                    s.execute("layer.new.groupFromLayers", json!({})).unwrap();
                }
                s.execute("layer.lockLayers", lock.clone()).unwrap();
                s.execute("layer.select", json!({"layer": layer.0})).unwrap();
                let locks = s.active().unwrap().doc.effective_locks(layer);
                assert!(locks.all || locks.pixels, "the lock reaches the smart object ({lock}, group: {on_group})");
                let steps = s.active().unwrap().history.past_len();
                let before = s.active().unwrap().doc.layer(layer).unwrap().clone();
                let r = s.execute(cmd, p);
                assert!(r.is_err(), "{cmd} edited a smart object locked with {lock} (group: {on_group}): {r:?}");
                let st = s.active().unwrap();
                let LayerContent::Smart(sm) = &st.doc.layer(layer).unwrap().content else { panic!("not smart") };
                assert!(sm.smart_filters.is_empty(), "{cmd} {lock} group: {on_group}");
                assert_eq!(st.doc.layer(layer).unwrap().surface(), before.surface(), "{cmd} {lock} group: {on_group}");
                assert_eq!(st.history.past_len(), steps, "{cmd} {lock} group: {on_group}: no history step");
            }
        }
    }
}

#[test]
fn unlocked_or_position_locked_smart_objects_take_distortions() {
    // Filters don't check the position lock, so neither do the smart-filter distortions.
    for lock in [None, Some(json!({"position": true})), Some(json!({"transparency": true}))] {
        for (cmd, p) in smart_distortions() {
            let mut s = session(8);
            s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            if let Some(lock) = &lock {
                s.execute("layer.lockLayers", lock.clone()).unwrap();
            }
            let steps = s.active().unwrap().history.past_len();
            s.execute(cmd, p).unwrap();
            assert_eq!(smart(&s).smart_filters.len(), 1, "{cmd} {lock:?}");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{cmd} {lock:?}");
        }
    }
}

/// #10457: with a layer mask (or the Quick Mask) targeted, Photoshop applies Liquify, Filter
/// Gallery and Camera Raw Filter to the mask and greys out Lens Correction, Adaptive Wide Angle and
/// Vanishing Point. They all edited the layer's pixels.
#[test]
fn dialog_filters_follow_a_targeted_mask_as_photoshops_do() {
    let mask_of = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.read_region(st.doc.bounds())
    };
    let masked = || {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        // Reveal Selection uses the selection up, as in Photoshop.
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("channel.target", json!({"channel": "mask"})).unwrap();
        s
    };
    let stroke = json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
    let edits = [
        (LIQUIFY, stroke.clone()),
        ("filter.filterGallery", json!({"effects": [{"filter": "spatter", "params": {}}]})),
        (crate::lens_cmds::RAW, json!({"exposure": -1.5})),
    ];
    for (id, params) in edits {
        let mut s = masked();
        let (pixels, mask) = (active_surface(&s), mask_of(&s));
        s.execute(id, params).unwrap();
        assert_eq!(active_surface(&s), pixels, "{id}: the layer's pixels are untouched");
        assert_ne!(mask_of(&s), mask, "{id}: the mask is filtered");
    }
    for id in [crate::lens_cmds::LENS, crate::lens_cmds::WIDE, crate::vp_cmds::VP] {
        let mut s = masked();
        assert!(s.execute(id, json!({})).is_err(), "{id} is disabled on a targeted mask");
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        assert!(s.is_enabled(id), "{id} is enabled on the layer again");
    }
    // Quick Mask: Liquify deforms it, not the layer.
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let pixels = active_surface(&s);
    let quick = |s: &Session| s.active().unwrap().doc.quick_mask.as_ref().unwrap().surface.read_region(s.active().unwrap().doc.bounds());
    let before = quick(&s);
    s.execute(LIQUIFY, stroke).unwrap();
    assert_eq!(active_surface(&s), pixels, "Quick Mask: the layer's pixels are untouched");
    assert_ne!(quick(&s), before, "Quick Mask: deformed");
}

/// #10457 (CodeRabbit on #10469): Lens Correction, Adaptive Wide Angle and Vanishing Point refuse
/// a mask named in their params as they do a mask targeted in the Channels panel, so a caller's
/// `"target":"mask"` neither filters the mask nor slips past to the layer.
#[test]
fn lens_filters_and_vanishing_point_refuse_an_explicit_mask_target() {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    let mask_of = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.read_region(st.doc.bounds())
    };
    let (pixels, mask) = (active_surface(&s), mask_of(&s));
    let calls = [
        (crate::lens_cmds::LENS, json!({"target": "mask", "distortion": 10})),
        (crate::lens_cmds::WIDE, json!({"target": "mask", "model": "perspective", "scale": 90})),
        (crate::vp_cmds::VP, json!({"target": "mask"})),
    ];
    for (id, params) in calls {
        assert!(s.is_enabled(id), "{id} is enabled on the layer");
        assert!(!s.is_enabled_with(id, &params), "{id} is disabled on a mask named in its params");
        let err = s.execute(id, params).unwrap_err();
        assert!(matches!(err, EngineError::Disabled(..)), "{id}: {err}");
        assert_eq!(active_surface(&s), pixels, "{id}: the layer's pixels are untouched");
        assert_eq!(mask_of(&s), mask, "{id}: the mask is untouched");
    }
}

/// #10457: a set of colour channels is still the layer's pixels, so the lens filters and Vanishing
/// Point stay enabled there; only a mask or channel greys them out.
#[test]
fn lens_filters_and_vanishing_point_stay_enabled_on_a_colour_set() {
    let mut s = session(8);
    s.execute("channel.target", json!({"colors": [0, 2]})).unwrap();
    for id in [crate::lens_cmds::LENS, crate::lens_cmds::WIDE, crate::vp_cmds::VP] {
        assert!(s.is_enabled(id), "{id} is enabled on a colour set: {:?}", s.disabled_reason(id));
    }
}

/// A levels adjustment layer whose mask reveals a 30×20 block, in a document of `mode`.
fn adjustment_with_mask(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "mode": mode})).unwrap();
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s
}

/// An adjustment layer's mask (the active layer's), and the Quick Mask, over the canvas.
fn adjustment_mask(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.read_region(st.doc.bounds())
}

/// Checks that dialog filter `id` with `params` filters an adjustment layer's mask, whether it is
/// targeted in the Channels panel or named in the params, and the Quick Mask over one.
fn filters_an_adjustment_layers_mask(id: &str, params: &Value) {
    let mut s = adjustment_with_mask("rgb");
    assert!(!s.is_enabled(id), "{id}: no pixels to filter on the layer itself");
    let mut explicit = params.clone();
    explicit["target"] = json!("mask");
    assert!(s.is_enabled_with(id, &explicit), "{id}: enabled on a mask named in its params");
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    assert!(s.is_enabled(id), "{id}: enabled on the targeted mask");
    let mask = adjustment_mask(&s);
    s.execute(id, params.clone()).unwrap();
    assert_ne!(adjustment_mask(&s), mask, "{id}: the mask is filtered");
    // The Quick Mask, with the adjustment layer active.
    let mut s = adjustment_with_mask("rgb");
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    assert!(s.is_enabled(id), "{id}: enabled in Quick Mask on an adjustment layer");
    let quick = |s: &Session| s.active().unwrap().doc.quick_mask.as_ref().unwrap().surface.read_region(s.active().unwrap().doc.bounds());
    let before = quick(&s);
    s.execute(id, params.clone()).unwrap();
    assert_ne!(quick(&s), before, "{id}: the Quick Mask is filtered");
}

/// #10457 (CodeRabbit on #10469): an adjustment layer's targeted mask takes Liquify as it takes the
/// other filters (#780), though the layer has no pixels of its own.
#[test]
fn liquify_deforms_an_adjustment_layers_targeted_mask() {
    let stroke = json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
    filters_an_adjustment_layers_mask(LIQUIFY, &stroke);
}

/// #10457 (CodeRabbit on #10469): an adjustment layer's targeted mask takes the Camera Raw Filter as
/// it takes the other filters (#780). It still needs an RGB or Grayscale document.
#[test]
fn camera_raw_filters_an_adjustment_layers_targeted_mask() {
    filters_an_adjustment_layers_mask(crate::lens_cmds::RAW, &json!({"exposure": -1.5}));
    let mut s = adjustment_with_mask("cmyk");
    let raw = crate::lens_cmds::RAW;
    assert!(!s.is_enabled_with(raw, &json!({"target": "mask"})), "Camera Raw: not in CMYK, even on a mask");
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    assert!(!s.is_enabled(raw), "Camera Raw: not in CMYK, even on a targeted mask");
}

/// #10457 (CodeRabbit on #10469): an alpha channel or the Quick Mask named in the params takes Liquify
/// and the Camera Raw Filter on an adjustment layer with the composite targeted, as it does when
/// targeted in the Channels panel; a pixel target there still needs pixels.
#[test]
fn dialog_filters_take_a_channel_named_in_their_params_on_an_adjustment_layer() {
    let stroke = json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
    for (id, params) in [(LIQUIFY, stroke), (crate::lens_cmds::RAW, json!({"exposure": -1.5}))] {
        let mut s = adjustment_with_mask("rgb");
        s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
        s.execute("select.saveSelection", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        assert!(!s.is_enabled(id), "{id}: no pixels to filter on the layer itself");
        let mut explicit = params.clone();
        explicit["target"] = json!({"channel": 0});
        assert!(s.is_enabled_with(id, &explicit), "{id}: enabled on an alpha channel named in its params");
        let alpha = |s: &Session| s.active().unwrap().doc.channels[0].surface.read_region(s.active().unwrap().doc.bounds());
        let before = alpha(&s);
        s.execute(id, explicit).unwrap();
        assert_ne!(alpha(&s), before, "{id}: the alpha channel is filtered");
        let mut missing = params.clone();
        missing["target"] = json!({"channel": 5});
        assert!(!s.is_enabled_with(id, &missing), "{id}: no such alpha channel");
    }
    let mut s = adjustment_with_mask("cmyk");
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    assert!(!s.is_enabled_with(crate::lens_cmds::RAW, &json!({"target": {"channel": 0}})), "Camera Raw: not in CMYK, even on a channel");
}

/// #10457 (CodeRabbit on #10469): Liquify naming its layer (`"layer"`) runs wherever it reads as
/// enabled: on an adjustment layer with its mask or an alpha channel targeted in the Channels
/// panel, the command judges the target the panel fills in, as `is_enabled_with` does.
#[test]
fn liquify_naming_its_layer_runs_wherever_it_is_enabled() {
    for target in [json!("mask"), json!(0)] {
        let mut s = adjustment_with_mask("rgb");
        s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
        s.execute("select.saveSelection", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": target})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let p = json!({"layer": layer.0, "strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
        assert!(s.is_enabled_with(LIQUIFY, &p), "{target}: enabled");
        let r = s.execute(LIQUIFY, p);
        assert!(r.is_ok(), "{target}: enabled, so it runs: {r:?}");
    }
}

/// #10457 (CodeRabbit on #10469): Camera Raw naming its layer (`"layer"`) with a mask targeted is
/// admitted by that layer, as it runs on that layer's mask: it runs exactly when the named layer
/// has a mask, whatever the active layer has, and is otherwise refused up front.
#[test]
fn camera_raw_naming_its_layer_is_admitted_by_that_layers_mask() {
    let raw = crate::lens_cmds::RAW;
    let mut s = session(8);
    let plain = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    let masked = s.active().unwrap().active_layer.unwrap();
    for (active, named) in [(masked, plain), (plain, masked)] {
        s.execute("layer.select", json!({"layer": active.0})).unwrap();
        let ran = s.execute(raw, json!({"layer": named.0, "target": "mask", "exposure": -1.5}));
        assert_eq!(ran.is_ok(), named == masked, "active {active:?}, named {named:?}: runs iff the named layer has a mask: {ran:?}");
        if let Err(e) = ran {
            assert!(matches!(e, EngineError::Disabled(..)), "refused at admission: {e}");
        }
    }
}

/// The session's layer with a mask that reveals a 30×20 block at (20, 15).
fn session_with_mask() -> Session {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s
}

fn active_mask(s: &Session) -> Surface {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.clone()
}

/// FredApps on #10469: Photoshop's Puppet Warp "works on image layers, layer masks, and vector
/// masks" (Adobe's help), so with the layer mask named in its params it warps the mask, which
/// stays a one-channel mask, and leaves the layer's pixels. A layer with no mask refuses the call
/// up front, with no history step.
#[test]
fn puppet_warp_warps_a_targeted_mask_and_refuses_a_missing_one() {
    let p = json!({"target": "mask", "pins": [{"src": [20, 20], "dst": [20, 20]}, {"src": [50, 30], "dst": [56, 34]}, {"src": [30, 35], "dst": [30, 35]}]});
    let mut s = session_with_mask();
    let (pixels, mask) = (active_surface(&s), active_mask(&s));
    s.execute(PUPPET, p.clone()).unwrap();
    let warped = active_mask(&s);
    assert_eq!(active_surface(&s), pixels, "the layer's pixels are untouched");
    assert_eq!(warped.format(), mask.format(), "the mask stays a one-channel mask");
    let canvas = s.active().unwrap().doc.bounds();
    assert_ne!(warped.read_region(canvas), mask.read_region(canvas), "the mask is warped");
    // The pins pull the revealed block's right side down: the strip it vacates along the block's
    // top edge takes the mask's default (black for Reveal Selection), not the white it held.
    let default = mask.default_pixel()[0];
    let vacated = Rect::new(40, 15, 50, 16);
    assert!(mask.read_region(vacated).iter().all(|&v| v != default), "the strip held content: {:?}", mask.read_region(vacated));
    assert!(warped.read_region(vacated).iter().all(|&v| v == default), "vacated: {:?}", warped.read_region(vacated));
    // No mask: refused before it runs, and nothing is recorded.
    let mut s = session(8);
    let (pixels, steps) = (active_surface(&s), s.active().unwrap().history.entries().len());
    assert!(!s.is_enabled_with(PUPPET, &p), "disabled on a layer without a mask");
    let err = s.execute(PUPPET, p).unwrap_err();
    assert!(matches!(&err, EngineError::Disabled(_, why) if why.contains("no layer mask")), "{err}");
    assert_eq!(s.active().unwrap().history.entries().len(), steps, "no history step");
    assert_eq!(active_surface(&s), pixels, "the layer's pixels are untouched");
}

/// FredApps on #10469: Adobe's help doesn't document Perspective Warp on a layer mask, so it keeps
/// what it did before #10469 with a `"target"`: it warps the layer, and its linked mask with it,
/// exactly as without one.
#[test]
fn perspective_warp_warps_the_layer_whatever_the_target() {
    let quad = json!({"planes": [{"src": [[10, 10], [60, 10], [60, 40], [10, 40]], "dst": [[14, 10], [60, 12], [58, 40], [10, 38]]}]});
    let mut targeted = quad.clone();
    targeted["target"] = json!("mask");
    let run = |p: &Value| {
        let mut s = session_with_mask();
        s.execute(PERSPECTIVE, p.clone()).unwrap();
        (active_surface(&s), active_mask(&s))
    };
    let pixels = active_surface(&session_with_mask());
    let (plain, mask) = run(&quad);
    assert_ne!(plain, pixels, "the layer is warped");
    assert!(run(&targeted) == (plain, mask), "a mask target changes nothing");
}

/// FredApps on #10469: with an alpha channel targeted in the Channels panel over a pixel layer, the
/// Camera Raw Filter filters the channel and leaves the layer's pixels.
#[test]
fn camera_raw_filters_an_alpha_channel_targeted_in_the_channels_panel() {
    let raw = crate::lens_cmds::RAW;
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 20, "y": 15, "width": 30, "height": 20})).unwrap();
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": 0})).unwrap();
    let alpha = |s: &Session| s.active().unwrap().doc.channels[0].surface.read_region(s.active().unwrap().doc.bounds());
    let (pixels, before) = (active_surface(&s), alpha(&s));
    assert!(s.is_enabled(raw), "enabled on the targeted channel: {:?}", s.disabled_reason(raw));
    s.execute(raw, json!({"exposure": -1.5})).unwrap();
    assert_eq!(active_surface(&s), pixels, "the layer's pixels are untouched");
    assert_ne!(alpha(&s), before, "the alpha channel is filtered");
}

/// FredApps on #10469: once the composite is targeted again after Liquifying the layer mask,
/// Liquify deforms the layer's pixels and leaves the mask as it was.
#[test]
fn liquify_deforms_the_pixels_again_once_the_composite_is_targeted() {
    let stroke = json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
    let mut s = session_with_mask();
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    let mask = active_mask(&s);
    s.execute(LIQUIFY, stroke.clone()).unwrap();
    assert_ne!(active_mask(&s), mask, "the targeted mask is deformed");
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let (pixels, mask) = (active_surface(&s), active_mask(&s));
    s.execute(LIQUIFY, stroke).unwrap();
    assert_ne!(active_surface(&s), pixels, "the layer's pixels are deformed");
    assert_eq!(active_mask(&s), mask, "the mask is untouched");
}

/// CodeRabbit on #10469: with a smart object's filter mask named in the params, Liquify and the
/// Camera Raw Filter act on the smart object as the menu does (they're left out of
/// `follows_filter_mask`: in Photoshop they add a smart filter, they don't filter the filter mask),
/// so the filter mask stays as it was and a new smart filter records the edit.
#[test]
fn liquify_and_camera_raw_add_a_smart_filter_when_the_filter_mask_is_named() {
    let stroke = json!({"strokes": [{"tool": "forwardWarp", "size": 30, "points": [[22, 25], [34, 25]]}]});
    for (id, params) in [(LIQUIFY, stroke), (crate::lens_cmds::RAW, json!({"exposure": -1.5}))] {
        let mut s = session(8);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        // A black stroke down x = 30 gives the smart object a filter mask that isn't plain white.
        s.execute("paint.stroke", json!({"points": [[30, 5], [30, 55]], "size": 8, "hardness": 1, "color": "#000000", "target": "filterMask"})).unwrap();
        let canvas = s.active().unwrap().doc.bounds();
        let filter_mask = |s: &Session| smart(s).filter_mask.expect("a filter mask").surface.read_region(canvas);
        let before = filter_mask(&s);
        let mut named = params.clone();
        named["target"] = json!("filterMask");
        s.execute(id, named).unwrap();
        assert!(filter_mask(&s) == before, "{id}: the filter mask is untouched");
        let commands: Vec<String> = smart(&s).smart_filters.iter().map(|f| f.command.clone()).collect();
        assert_eq!(commands, ["filter.blur.gaussianBlur", id], "{id}: added as a smart filter, as from the menu");
    }
}
