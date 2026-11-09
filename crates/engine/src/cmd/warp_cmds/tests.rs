use super::*;

#[test]
fn a_warp_box_holding_the_whole_layer_gives_what_compositing_over_the_cleared_layer_did() {
    for depth in [8, 16, 32] {
        let s = session(depth);
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        let surf = l.surface().unwrap().clone();
        let rect = surf.content_bounds();
        let w = Warp::preset(WarpStyle::Flag, 35.0, rect_f(rect));
        // What the warp built before the shortcut: the warp composited over the cleared layer.
        let map = |x: f64, y: f64| w.map(x, y);
        let warped = warp_mesh_surface_with(&surf, rect, &map, Fold::IN_PLACE, Interp::Bicubic, &Interrupt::NONE).unwrap();
        let mut want = surf.clone();
        crate::pixels::try_clear_surface(&mut want, rect, None).unwrap();
        crate::transform_cmds::composite_over(&mut want, &warped);
        want.prune();
        let mut got = l.clone();
        warp_layer(None, Locks::default(), &mut got, &w, rect, Interp::Bicubic, None, &Interrupt::NONE).unwrap();
        let got = got.surface().unwrap();
        let b = want.content_bounds();
        assert_eq!(got.content_bounds(), b, "depth {depth}");
        let (g, wv) = (got.read_region(b), want.read_region(b));
        // Exact at 8 and 16 bits. At 32 the composite's `t·a / a` rounds a few values by an
        // ulp; the shortcut keeps the warp's own.
        let tol = if depth == 32 { 1e-6 } else { 0.0 };
        let worst = g.iter().zip(&wv).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst <= tol, "depth {depth}: off by {worst}");
    }
}

fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let surf = l.surface_mut().unwrap();
        for y in 10..40 {
            for x in 10..60 {
                let v = ((x * 3 + y * 5) % 11) as f32 / 10.0;
                surf.write_pixel(x, y, &[v, 1.0 - v, 0.5, 1.0]);
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

/// Warp on type sets its Warp Text (#3004): the layer stays type, one step; a custom mesh is
/// refused rather than rasterizing.
#[test]
fn warp_style_on_type_is_warp_text() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("type.create", json!({"x": 10, "y": 60, "text": "Hi"})).unwrap();
    let n = s.active().unwrap().history.past_len();
    s.execute("edit.transform.warp", json!({"style": "arc", "bend": 30})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("type") };
    assert_eq!(t.warp.as_ref().map(|w| (w.style.as_str(), w.value)), Some(("warpArc", 30.0)));
    assert_eq!(st.history.past_len(), n + 1);
    let mesh = BezierMesh::identity([0.0, 0.0, 10.0, 10.0], 1, 1).map_points(|p| [p[0] * 2.0, p[1]]);
    assert!(s.execute("edit.transform.warp", json!({"warp": Warp::custom(mesh, [0.0, 0.0, 10.0, 10.0])})).is_err());
}

/// The seven values of a cylinder over the session's painted box (x 10..60, y 10..40).
const CYL: [f64; 7] = [10.0, 40.0, 60.0, 10.0, 0.3, 0.3, 0.5];

/// `style: "cylinder"` takes its seven values (#4112); anything that isn't a horizontal
/// cylinder is refused, never a silent no-op, and the layer is left alone.
#[test]
fn cylinder_style_needs_its_values() {
    let mut s = session(8);
    let before = active_surface(&s);
    for p in [
        json!({"style": "cylinder"}),
        json!({"style": "cylinder", "values": [10, 40, 60, 10, 0.3, 0.3]}),
        json!({"style": "cylinder", "values": [60, 40, 10, 10, 0.3, 0.3, 0.5]}),
        json!({"style": "cylinder", "values": [10, 40, 60, 10, 0.3, 0.3, "x"]}),
        json!({"style": "cylinder", "values": "seven"}),
        json!({"style": "cylinder", "values": CYL, "vertical": true}),
        json!({"style": "cylinder", "values": CYL, "hDistort": 10}),
        json!({"warp": {"style": "cylinder", "bounds": [10.0, 10.0, 60.0, 40.0]}}),
        json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "vertical": true}}),
        json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "hDistort": 10.0}}),
        json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "vDistort": -5.0}}),
    ] {
        let e = s.execute("edit.transform.warp", p.clone()).unwrap_err().to_string();
        assert!(e.contains("cylinder"), "{p}: {e}");
    }
    assert_eq!(active_surface(&s), before);
    // On type it is refused like a custom mesh: Warp Text has no cylinder.
    s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi"})).unwrap();
    assert!(s.execute("edit.transform.warp", json!({"style": "cylinder", "values": CYL})).is_err());
}

#[test]
fn cylinder_warps_pixels_at_every_depth() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let r = s.execute("edit.transform.warp", json!({"style": "cylinder", "values": CYL})).unwrap();
        assert_eq!(r["changed"], true, "{depth}");
        let after = active_surface(&s);
        let b = after.content_bounds();
        // Between the silhouettes, the edges bowed down by the ellipses.
        assert!(b.x0 >= 9 && b.x1 <= 61 && b.y0 >= 9 && b.y1 > 40, "{depth}: {b:?}");
        // The same cylinder as a full warp object (its mesh rebuilt from the values) agrees.
        let mut t = session(depth);
        let w = Warp::cylinder(&CYL, [10.0, 10.0, 60.0, 40.0]).unwrap();
        let mut garbled = w.clone();
        garbled.mesh = None;
        t.execute("edit.transform.warp", json!({"warp": garbled})).unwrap();
        assert_eq!(active_surface(&t), after, "{depth}");
    }
}

/// On a smart object the cylinder is stored in source space: still a cylinder (so it goes to
/// PSD as one) when the placement only scales and moves, its mesh when it rotates.
#[test]
fn smart_object_cylinder_is_stored_in_source_space() {
    let stored = |s: &Session| {
        let st = s.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("smart") };
        sm.warp.clone().unwrap()
    };
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})).unwrap();
    let w = stored(&s);
    assert_eq!(w.style, WarpStyle::Cylinder);
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layers[1].content else { panic!("smart") };
    let inv = crate::smart_cmds::placement(sm).inverse().unwrap();
    let src = |x: f64, y: f64| inv.apply(x, y);
    let ((l, t), (r, bt)) = (src(10.0, 10.0), src(60.0, 40.0));
    for (a, b) in w.values.iter().zip([l, bt, r, t, 0.3, 0.3, 0.5]) {
        assert!((a - b).abs() < 1e-9, "{:?}", w.values);
    }
    // Twice as wide: the silhouettes double, the ellipses' depth (in half axes) halves.
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 100})).unwrap();
    let wide = [-15.0, 40.0, 85.0, 10.0, 0.3, 0.3, 0.5];
    s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": wide, "rect": [-15, 10, 85, 40]})).unwrap();
    let w = stored(&s);
    assert_eq!(w.style, WarpStyle::Cylinder);
    assert!((w.values[4] - 0.6).abs() < 1e-9 && (w.values[5] - 0.6).abs() < 1e-9, "{:?}", w.values);
    // The silhouettes land on the source's own edges (its contents span the scaled box).
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layers[1].content else { panic!("smart") };
    let inv = crate::smart_cmds::placement(sm).inverse().unwrap();
    assert!((w.values[0] - inv.apply(-15.0, 10.0).0).abs() < 1e-9 && (w.values[2] - inv.apply(85.0, 10.0).0).abs() < 1e-9, "{:?}", w.values);
    // Rotated: a custom mesh with the same shape.
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("edit.transform", json!({"rotate": 30})).unwrap();
    s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})).unwrap();
    let w = stored(&s);
    assert_eq!(w.style, WarpStyle::Custom);
    assert_eq!(w.mesh.as_ref().map(|m| (m.nx(), m.ny())), Some((13, 4)));
}

/// Warp mode shows a stored cylinder as a cylinder in document space (its handles edit the
/// seven values), and Enter commits it untouched: the smart object keeps its cylinder (else the
/// next PSD save writes no warp). The same holds for its mesh, which is what a rotated placement
/// shows and what Warp mode showed before it had a cylinder editor. A changed value stays a
/// cylinder; a moved mesh point makes it a custom warp.
#[test]
fn untouched_cylinder_committed_from_warp_mode_stays_a_cylinder() {
    for scale in [100, 150] {
        let mut s = session(8);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("edit.transform", json!({"scaleX": scale, "scaleY": 100})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let rect = warp_bounds(&s.active().unwrap().doc, s.active().unwrap().doc.layer(id).unwrap());
        let [x0, x1] = [f64::from(rect.x0), f64::from(rect.x1)];
        let values = [x0, 40.0, x1, 10.0, 0.3, 0.3, 0.5];
        s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": values, "rect": [rect.x0, 10, rect.x1, 40]})).unwrap();
        let stored = |s: &Session| {
            let st = s.active().unwrap();
            let LayerContent::Smart(sm) = &st.doc.layer(id).unwrap().content else { panic!("smart") };
            sm.warp.clone().unwrap()
        };
        let cylinder = stored(&s);
        assert_eq!(cylinder.style, WarpStyle::Cylinder, "{scale}");
        let shown = smart_warp_doc_space(s.active().unwrap().doc.layer(id).unwrap()).unwrap();
        assert_eq!(shown.style, WarpStyle::Cylinder, "{scale}: shown as a cylinder");
        for (v, sv) in values.iter().zip(&shown.values) {
            assert!((v - sv).abs() < 1e-6, "{scale}: the values come back in document space: {values:?} vs {:?}", shown.values);
        }
        let as_mesh = Warp::custom(shown.to_mesh(1, 1), shown.bounds);
        for (what, untouched) in [("cylinder", shown.clone()), ("mesh", as_mesh.clone())] {
            s.execute("select.all", json!({})).unwrap();
            let (sel, steps) = (s.active().unwrap().doc.selection.clone(), s.active().unwrap().history.past_len());
            let r = s.execute("edit.transform.warp", json!({"layer": id.0, "rect": untouched.bounds, "warp": untouched, "interpolation": "bicubic"})).unwrap();
            assert_eq!(r["changed"], false, "{scale} {what}");
            assert_eq!(stored(&s), cylinder, "{scale} {what}: untouched");
            let st = s.active().unwrap();
            assert!(st.doc.selection == sel && st.history.past_len() == steps, "{scale} {what}: no step, the selection stays");
        }
        // The same mesh over another box is another warp.
        let mut wider = as_mesh.clone();
        wider.bounds[2] += 10.0;
        s.execute("edit.transform.warp", json!({"layer": id.0, "rect": wider.bounds, "warp": wider})).unwrap();
        assert_eq!(stored(&s).style, WarpStyle::Custom, "{scale}: other box");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(stored(&s), cylinder);
        let mut moved = as_mesh.clone();
        if let Some(m) = moved.mesh.as_mut() {
            m.points[20][1] += 5.0;
        }
        s.execute("edit.transform.warp", json!({"layer": id.0, "rect": moved.bounds, "warp": moved})).unwrap();
        assert_eq!(stored(&s).style, WarpStyle::Custom, "{scale}: edited mesh");
        s.execute("edit.undo", json!({})).unwrap();
        // A value changed by a handle commits as a cylinder with that value, one step.
        let steps = s.active().unwrap().history.past_len();
        let mut deeper = shown.values.clone();
        deeper[4] = 0.45;
        let r = s.execute("edit.transform.warp", json!({"layer": id.0, "rect": shown.bounds, "warp": Warp::cylinder(&deeper, shown.bounds).unwrap()})).unwrap();
        assert_eq!(r["changed"], true, "{scale}: edited cylinder");
        let after = stored(&s);
        assert_eq!(after.style, WarpStyle::Cylinder, "{scale}: still a cylinder");
        assert!((after.values[4] - 0.45 * cylinder.values[4] / 0.3).abs() < 1e-9, "{scale}: the depth follows, in source units: {:?}", after.values);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{scale}: one step");
    }
}

/// Under a rotated placement a stored cylinder has no document-space values: Warp mode shows its
/// mesh, and committing that mesh untouched still changes nothing.
#[test]
fn a_rotated_cylinder_is_shown_as_its_mesh() {
    let mut s = session(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})).unwrap();
    s.execute("edit.transform", json!({"rotate": 30})).unwrap();
    let stored = |s: &Session| {
        let st = s.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(id).unwrap().content else { panic!("smart") };
        sm.warp.clone().unwrap()
    };
    assert_eq!(stored(&s).style, WarpStyle::Cylinder, "the placement turned, the source warp stays");
    let shown = smart_warp_doc_space(s.active().unwrap().doc.layer(id).unwrap()).unwrap();
    assert_eq!(shown.style, WarpStyle::Custom);
    assert!(shown.values.is_empty());
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("edit.transform.warp", json!({"layer": id.0, "rect": shown.bounds, "warp": shown})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn identity_and_bend_zero_are_no_ops_at_every_depth() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        let before = active_surface(&s);
        let r = s.execute("edit.transform.warp", json!({"style": "custom"})).unwrap();
        assert_eq!(r["changed"], false);
        s.execute("edit.transform.warp", json!({"style": "flag", "bend": 0})).unwrap();
        // A custom identity mesh passed explicitly (forces a resample) stays within 1/255.
        let m = BezierMesh::identity([10.0, 10.0, 60.0, 40.0], 2, 2);
        let mut w = Warp::custom(m, [10.0, 10.0, 60.0, 40.0]);
        w.mesh.as_mut().unwrap().points[5][0] += 1e-12; // not exactly identity
        s.execute("edit.transform.warp", json!({"warp": w})).unwrap();
        let after = active_surface(&s);
        let r = Rect::new(10, 10, 60, 40);
        assert_eq!(after.content_bounds(), r, "{depth}");
        let worst = before.read_region(r).iter().zip(after.read_region(r)).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst <= 1.0 / 255.0, "{depth}: {worst}");
    }
}

#[test]
fn arc_warps_pixels_and_undoes() {
    let mut s = session(8);
    let before = active_surface(&s);
    s.execute("edit.transform.warp", json!({"style": "arc", "bend": 50})).unwrap();
    let after = active_surface(&s);
    // As in Photoshop: the bottom edge keeps its corners, the top rises and spreads.
    let c = after.content_bounds();
    assert!(c.y1 <= 41 && c.y0 < 8 && c.x0 < 8 && c.x1 > 62, "{c:?}");
    s.undo();
    assert_eq!(active_surface(&s), before);
    assert!(s.execute("edit.transform.warp", json!({"style": "spiral"})).is_err());
    assert!(s.execute("edit.transform.warp", json!({})).is_err());
    assert!(s.execute("edit.transform.warp", json!({"mesh": {"us": [0, 1], "vs": [0, 1], "points": [[0, 0]]}})).is_err());
}

#[test]
fn custom_mesh_moves_a_control_point() {
    let mut s = session(16);
    let mut m = BezierMesh::identity([10.0, 10.0, 60.0, 40.0], 1, 1);
    m.points[15] = [70.0, 50.0]; // bottom-right corner
    s.execute("edit.transform.warp", json!({"mesh": m})).unwrap();
    let b = active_surface(&s).content_bounds();
    assert!(b.x1 >= 69 && b.y1 >= 49, "{b:?}");
}

#[test]
fn split_cuts_existing_sections_instead_of_replacing_the_grid() {
    let b = [0.0, 0.0, 90.0, 60.0];
    let plain = Warp::custom(BezierMesh::identity(b, 1, 1), b);
    let cut = split_warp(&plain, Some([b[0] + 0.25 * 90.0, 30.0]), Split::Vertical).unwrap();
    let m = cut.mesh.unwrap();
    for u in [0.0, 0.25, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
        assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "default guides stay, cut at 0.25: {:?}", m.us);
    }
    assert_eq!(m.vs, vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
    let grid = BezierMesh::identity(b, 3, 3);
    let cut = split_warp(&Warp::custom(grid.clone(), b), Some([45.0, 10.0]), Split::Vertical).unwrap();
    let m = cut.mesh.unwrap();
    assert_eq!(m.vs, grid.vs);
    assert_eq!(m.us.len(), grid.us.len() + 1);
    for u in [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0] {
        assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.us);
    }
    let (nx0, nx1) = (grid.nx(), m.nx());
    for j in 0..grid.ny() {
        for i in 0..3 {
            assert_eq!(m.points[j * nx1 + i], grid.points[j * nx0 + i]);
        }
    }
}

#[test]
fn split_commands_edit_meshes_exactly() {
    let mut s = session(8);
    let w = Warp::preset(WarpStyle::Bulge, 30.0, [10.0, 10.0, 60.0, 40.0]);
    let r = s.execute("edit.transform.splitWarpCrosswise", json!({"warp": w, "at": [30.0, 20.0]})).unwrap();
    let split: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    let m = split.mesh.as_ref().unwrap();
    assert_eq!((m.nx(), m.ny()), (7, 7));
    let r = s.execute("edit.transform.splitWarpVertically", json!({"warp": split})).unwrap();
    let v: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    assert_eq!(v.mesh.as_ref().unwrap().us.len(), 4);
    let r = s.execute("edit.transform.removeWarpSplit", json!({"warp": v, "at": [30.0, 20.0]})).unwrap();
    let back: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    assert!(back.mesh.as_ref().unwrap().us.len() + back.mesh.as_ref().unwrap().vs.len() < 7);
    // Splitting never changes the shape.
    for (x, y) in [(15.0, 15.0), (40.0, 33.0)] {
        let (a, b) = (w.map(x, y), split.map(x, y));
        assert!((a.0 - b.0).abs() < 0.6 && (a.1 - b.1).abs() < 0.6, "{a:?} {b:?}");
    }
}

#[test]
fn smart_object_warp_is_lossless() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let original = active_surface(&s);
        s.execute("layer.smartObjects.warp", json!({"style": "wave", "bend": 60})).unwrap();
        let warped = active_surface(&s);
        assert_ne!(warped, original);
        let st = s.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
        assert!(sm.warp.is_some());
        // Removing the warp re-renders the source exactly (no accumulated resampling).
        s.execute("layer.smartObjects.warp", json!({"style": "none"})).unwrap();
        assert_eq!(active_surface(&s), original, "{depth}");
        // Warping twice equals warping once with the second warp.
        s.execute("layer.smartObjects.warp", json!({"style": "arc", "bend": 20})).unwrap();
        s.execute("layer.smartObjects.warp", json!({"style": "wave", "bend": 60})).unwrap();
        assert_eq!(active_surface(&s), warped, "{depth}");
        // Split on the stored warp keeps the appearance (within resampling of the mesh fit).
        s.execute("edit.transform.splitWarpCrosswise", json!({})).unwrap();
        let st = s.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
        assert_eq!(sm.warp.as_ref().unwrap().style, WarpStyle::Custom);
    }
}

#[test]
fn crosswise_split_of_an_identity_custom_warp_is_four_by_four() {
    let mut s = session(8);
    let r = s.execute("edit.transform.splitWarpCrosswise", json!({"style": "custom", "rect": [0.0, 0.0, 90.0, 60.0]})).unwrap();
    let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    let m = out.mesh.unwrap();
    assert_eq!((m.us.len(), m.vs.len()), (5, 5), "thirds, then the centre cut: {:?}", (m.us.clone(), m.vs.clone()));
    for u in [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0] {
        assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.us);
        assert!(m.vs.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.vs);
    }
}

#[test]
fn extreme_rect_is_bad_params_not_a_giant_warp() {
    let mut s = session(8);
    for cmd in ["edit.transform.warp", "layer.smartObjects.warp"] {
        if cmd == "layer.smartObjects.warp" {
            s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        }
        let e = s.execute(cmd, json!({"style": "arc", "bend": 50, "rect": [-1e30, -1e30, 1e30, 1e30]})).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("rect too large")), "{cmd}: {e}");
    }
    s.execute("layer.smartObjects.warp", json!({"style": "arc", "bend": 50, "rect": [10, 10, 60, 40]})).unwrap();
}

#[test]
fn warp_grid_fits_the_surface_and_rejects_bad_sizes() {
    let mut s = session(8);
    assert!(s.execute("edit.transform.warpGrid", json!({})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": 0})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": -3})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": 65})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": 1.5})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": "custom"})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"size": "nope"})).is_err());
    assert!(s.execute("edit.transform.warpGrid", json!({"grid": [3, 3]})).is_err());
    let b = [10.0, 10.0, 60.0, 40.0];
    let mut bent = BezierMesh::identity(b, 1, 1);
    bent.points[0][0] += 12.0;
    let r = s.execute("edit.transform.warpGrid", json!({"warp": Warp::custom(bent.clone(), b), "size": 4})).unwrap();
    let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    let m = out.mesh.unwrap();
    assert_eq!(out.style, WarpStyle::Custom);
    assert_eq!(m.us, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    assert_eq!(m.vs, m.us);
    assert!((m.eval(0.0, 0.0)[0] - bent.eval(0.0, 0.0)[0]).abs() < 1e-3);
    assert!((m.eval(1.0, 1.0)[0] - b[2]).abs() < 1e-3);
    let r = s.execute("edit.transform.warpGrid", json!({"warp": Warp::custom(m, b), "size": "default"})).unwrap();
    let back: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    assert_eq!(back.mesh.unwrap().us.len(), 2);
}

/// A selection limits the warp of a linked mask too, as it does the pixels (#4175; Free
/// Transform and Perspective Warp did this already, #3083): the moved part of the mask lines
/// up with the moved pixels and the unselected part stays put.
#[test]
fn linked_mask_with_a_selection_warps_only_the_selected_region() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
        // Hide the left quarter (selected) and the right quarter (not selected).
        let m = l.mask.as_mut().unwrap();
        m.surface.fill_rect(Rect::new(10, 10, 15, 20), &[0.0]);
        m.surface.fill_rect(Rect::new(25, 10, 30, 20), &[0.0]);
        m.linked = true;
        Ok(())
    })
    .unwrap();
    let mask_at = |s: &Session, x, y| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
    };
    let alpha_at = |s: &Session, x, y| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
    };
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    // A mesh that translates the selected box 50 px down.
    let b = [10.0, 10.0, 20.0, 20.0];
    let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| [p[0], p[1] + 50.0]);
    s.execute("edit.transform.warp", json!({"rect": [10, 10, 20, 20], "mesh": mesh})).unwrap();
    // Moved pixels and their mask line up.
    assert!(alpha_at(&s, 12, 65) > 0.99);
    assert!(mask_at(&s, 12, 65) < 0.01, "selected hidden quarter moved");
    assert!(mask_at(&s, 17, 65) > 0.99, "selected visible quarter moved");
    // The vacated area reads as revealed, with transparent pixels.
    assert!(alpha_at(&s, 12, 15) < 0.01);
    assert!(mask_at(&s, 12, 15) > 0.99);
    // The unselected part of the mask stays, aligned with the unselected pixels.
    assert!(alpha_at(&s, 27, 15) > 0.99);
    assert!(mask_at(&s, 27, 15) < 0.01, "unselected mask stays put");
    assert!(mask_at(&s, 27, 65) > 0.99, "and isn't copied along");
    for (x, y) in [(11, 61), (13, 68), (16, 61), (18, 68)] {
        let want = if x < 15 { 0.0 } else { 1.0 };
        assert!((mask_at(&s, x, y) - want).abs() < 0.01, "({x},{y}): {}", mask_at(&s, x, y));
    }
}

/// Esc stops the warp of a selected linked mask too, not only of the pixels. A fill layer has
/// no pixels to warp, so the mask is the only part that can see the cancel.
#[test]
fn a_cancelled_warp_stops_in_a_selected_linked_mask() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.edit("link", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().linked = true;
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let st = s.active().unwrap();
    let (sel, mut l) = (st.doc.selection.clone().unwrap(), st.doc.layer(st.active_layer.unwrap()).unwrap().clone());
    let rect = Rect::new(10, 10, 20, 20);
    let w = Warp::preset(WarpStyle::Flag, 35.0, rect_f(rect));
    let ctl = Interrupt::cancel_only(&|| true);
    let r = warp_layer(Some(&sel), Locks::default(), &mut l, &w, rect, Interp::Bicubic, None, &ctl);
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
}

/// #8927: Warp mode on a distorted smart object edits its warp in source space: the start is no
/// warp over the contents (which fill that box unplaced) with the Distort as the placement.
/// Committed with `contents`, the mesh is stored exactly, also with a point pulled far outside its
/// box (the render reaches it), and with a moved placement in the same step. Untouched is no step;
/// bad input is refused.
#[test]
fn a_smart_objects_warp_is_edited_and_stored_in_source_space() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 16, "width": 32, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let quad = [[8.0, 16.0], [56.0, 4.0], [40.0, 32.0], [8.0, 32.0]];
    s.execute("edit.transform", json!({"rect": [8, 16, 40, 32], "quad": quad})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let layer = st.doc.layer(id).unwrap();
    let (w, place) = smart_warp_source(&st.doc, layer).unwrap();
    assert!(w.is_identity());
    let b = w.bounds;
    for (corner, want) in [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].iter().zip(quad) {
        let (x, y) = place.apply(corner[0], corner[1]);
        assert!((x - want[0]).abs() < 1e-6 && (y - want[1]).abs() < 1e-6, "{corner:?} → ({x}, {y}), not {want:?}");
    }
    let c = smart_unplaced_contents(&st.doc, layer).unwrap().0.content_bounds();
    assert_eq!([f64::from(c.x0), f64::from(c.y0), f64::from(c.x1), f64::from(c.y1)], b, "the contents fill the box");
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": w})).unwrap();
    assert_eq!(r["changed"], false, "untouched");
    // Bad input: no step.
    for bad in [
        json!({"layer": id.0, "contents": true}),
        json!({"layer": id.0, "contents": true, "warp": w, "placement": [1, 0, 0]}),
        json!({"layer": id.0, "contents": true, "warp": w, "placement": [0, 0, 0, 0, 0, 0, 0, 0, 0]}),
        json!({"layer": id.0, "contents": true, "warp": w, "placement": "identity"}),
    ] {
        assert!(s.execute("edit.transform.warp", bad.clone()).is_err(), "accepted {bad}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    // A grid point pulled 40 px out to the right of the contents' box, in source space.
    let before = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
    let mut pulled = w.clone();
    if let Some(m) = pulled.mesh.as_mut() {
        m.points[7][0] += 40.0;
    }
    s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": pulled})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(id).unwrap().content else { panic!("smart") };
    assert_eq!(sm.warp.as_ref(), Some(&pulled), "stored as it was edited");
    assert_eq!(crate::smart_cmds::placement(sm).0, place.0, "the Distort is kept");
    let after = st.doc.layer(id).unwrap().surface().unwrap().content_bounds();
    assert!(after.x1 > before.x1 + 10, "the pulled part renders past the box: {after:?}, was {before:?}");
    assert_eq!(st.history.past_len(), steps + 1);
    // A moved placement in the same step.
    let shifted = Homography([1.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).mul(&place);
    s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": pulled, "placement": shifted.0})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(id).unwrap().content else { panic!("smart") };
    let got = crate::smart_cmds::placement(sm);
    let (x, y) = got.apply(b[0], b[1]);
    assert!((x - 13.0).abs() < 1e-6 && (y - 16.0).abs() < 1e-6, "moved 5 px: ({x}, {y})");
    assert_eq!(st.history.past_len(), steps + 2);
    // A pixel layer has no contents to warp.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.execute("edit.transform.warp", json!({"contents": true, "warp": w})).is_err());
}

/// The contents warp moves a linked layer mask with the contents, refuses a position-locked
/// smart object, and refuses a placement that would make an absurd layer, all without a step.
#[test]
fn a_contents_warp_takes_its_linked_mask_along_and_respects_locks_and_size() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 30})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    let _ = s.execute("select.deselect", json!({}));
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let (w, place) = smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap();
    let mask_bounds = |s: &Session| {
        let m = s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.clone();
        let px = m.read_region(Rect::new(0, 0, 128, 128));
        let xs: Vec<usize> = (0..128 * 128).filter(|k| px[*k] > 0.5).map(|k| k % 128).collect();
        (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
    };
    let before = mask_bounds(&s);
    let shifted = Homography([1.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).mul(&place);
    let steps = s.active().unwrap().history.past_len();
    // Absurd: refused before allocating.
    let huge = Homography([1e5, 0.0, 0.0, 0.0, 1e5, 0.0, 0.0, 0.0, 1.0]).mul(&place);
    assert!(s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": w, "placement": huge.0})).is_err());
    // Locked: refused.
    s.edit("lock", |doc, _| {
        doc.layer_mut(id).unwrap().locks.position = true;
        Ok(())
    })
    .unwrap();
    let locked_steps = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": w, "placement": shifted.0})).is_err());
    assert_eq!(s.active().unwrap().history.past_len(), locked_steps);
    s.undo();
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    // The linked mask moves with the contents.
    s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": w, "placement": shifted.0})).unwrap();
    let after = mask_bounds(&s);
    assert_eq!((after.0 as i64 - before.0 as i64, after.1 as i64 - before.1 as i64), (10, 10), "the mask follows: {before:?} → {after:?}");
}

/// The contents warp moves the selection outline with the contents, as a warp of the layer does.
#[test]
fn a_contents_warp_moves_the_selection_with_the_contents() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 30})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let (w, place) = smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap();
    let before = st.doc.selection.as_ref().unwrap().content_bounds();
    let shifted = Homography([1.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).mul(&place);
    s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": w, "placement": shifted.0})).unwrap();
    let after = s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    assert!((after.x0 - before.x0 - 10).abs() <= 1 && (after.x1 - before.x1 - 10).abs() <= 1, "{before:?} → {after:?}");
}

/// A contents warp whose mesh reaches far outside its box is refused before anything is
/// rendered, with or without a placement, and what lies outside a warp moves with its edge
/// rather than collapsing onto it.
#[test]
fn a_contents_warp_prices_the_meshs_reach_and_moves_what_is_outside() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let (w, place) = smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap();
    let steps = st.history.past_len();
    let mut far = w.to_mesh(1, 1);
    far.points[5] = [1e7, -1e7];
    let far = Warp::custom(far, w.bounds);
    for placement in [None, Some(place.0)] {
        let err = s.execute("edit.transform.warp", json!({"layer": id.0, "contents": true, "warp": far, "placement": placement}));
        assert!(err.is_err(), "{err:?}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    // A warp that moves the contents' right edge 10 px right takes a point 5 px past that edge
    // along, 5 px past where the edge went.
    let b = w.bounds;
    let mut shift = w.to_mesh(1, 1);
    for (k, p) in shift.points.iter_mut().enumerate() {
        if k % 4 == 3 {
            p[0] += 10.0;
        }
    }
    let shifted = Warp::custom(shift, b);
    let (ex, ey) = place.apply(b[2] + 5.0, (b[1] + b[3]) / 2.0);
    let forward = contents_move(place, None, place, Some(shifted.clone())).unwrap();
    let (fx, fy) = forward(ex, ey);
    let (gx, gy) = place.apply(b[2] + 15.0, (b[1] + b[3]) / 2.0);
    assert!((fx - gx).abs() < 1e-6 && (fy - gy).abs() < 1e-6, "({fx}, {fy}) vs ({gx}, {gy})");
    // And back: the old warp's outside is found from its edge too.
    let back = contents_move(place, Some(shifted), place, None).unwrap();
    let (bx, by) = back(gx, gy);
    assert!((bx - ex).abs() < 1e-3 && (by - ey).abs() < 1e-3, "({bx}, {by}) vs ({ex}, {ey})");
}
