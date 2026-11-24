use photocraft_engine::Session;
use serde_json::json;

/// Make Shape preserves source geometry, defaults to legacy, and records conversion atomically.
#[test]
fn pixel_align_make_shape_supports_work_saved_and_inline_paths() {
    for source in ["work", "saved", "inline"] {
        for enabled in [None, Some(false), Some(true)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let path = json!({"subpaths":[{"closed":true,"knots":[[10,10],[50,10],[50,50]]}]});
            s.execute("path.set", json!({"name":"work","path":path})).unwrap();
            let expected = s.active().unwrap().doc.work_path.clone().unwrap();
            let mut params = json!({"fill":null,"stroke":{"width":1}});
            match source {
                "work" => params["name"] = json!("work"),
                "saved" => {
                    s.execute("path.set", json!({"name":"Saved","path":path})).unwrap();
                    params["name"] = json!("Saved");
                }
                _ => params["path"] = path,
            }
            if let Some(value) = enabled {
                params["useAlignedRendering"] = json!(value);
            }
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let info = s.execute("path.toShape", params).unwrap();
            assert_eq!(info["useAlignedRendering"], enabled.unwrap_or(false), "source={source}");
            let st = s.active().unwrap();
            let photocraft_doc::LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
            assert_eq!(shape.path, expected);
            assert_eq!(st.doc.paths, before.paths);
            assert_eq!(st.doc.work_path.is_none(), source == "work");
            assert_eq!(st.history.past_len(), history + 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.work_path, before.work_path);
            assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
            s.execute("edit.redo", json!({})).unwrap();
            let redone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(redone["useAlignedRendering"], enabled.unwrap_or(false));
            assert_eq!(redone["path"], info["path"]);
        }
    }
}

/// Invalid alignment values cannot consume a Work Path or alter document/history.
#[test]
fn pixel_align_make_shape_rejects_invalid_flag_atomically() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100})).unwrap();
    s.execute("path.set", json!({"path":{"subpaths":[{"knots":[[10,10],[40,10],[40,40]]}]}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        assert!(s.execute("path.toShape", json!({"name":"work","useAlignedRendering":invalid})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

/// Complete Stroke copies a snapshot of alignment and style, and pastes both in one undo step.
#[test]
fn complete_stroke_clipboard_preserves_pixel_alignment_and_geometry() {
    for enabled in [false, true] {
        for new_document in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let source = s
                .execute(
                    "shape.create",
                    json!({"kind":"rect","rect":[10,10,20,20],
                "useAlignedRendering":enabled,"stroke":{"width":1,"color":"#ff0000"}}),
                )
                .unwrap();
            let history = s.active().unwrap().history.past_len();
            s.execute("path.style.copyStroke", json!({})).unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), history, "copy must not add history");
            // Editing the source after copying must not change the clipboard snapshot.
            s.execute("shape.edit", json!({"useAlignedRendering":!enabled,"stroke":null})).unwrap();
            assert!(s.execute("path.style.copyStroke", json!({})).is_err());
            if new_document {
                s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            }
            let before = s
                .execute(
                    "shape.create",
                    json!({"kind":"ellipse","rect":[40,40,30,30],
                "useAlignedRendering":!enabled,"fill":"#00ff00","stroke":{"width":3,"color":"#0000ff"}}),
                )
                .unwrap();
            let history = s.active().unwrap().history.past_len();
            let after = s.execute("path.style.pasteStroke", json!({})).unwrap();
            assert_eq!(after["useAlignedRendering"], enabled, "new_document={new_document}");
            assert_eq!(after["stroke"], source["stroke"]);
            assert_eq!(after["path"], before["path"]);
            assert_eq!(after["fill"], before["fill"]);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            s.execute("edit.undo", json!({})).unwrap();
            let undone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(undone["useAlignedRendering"], !enabled);
            assert_eq!(undone["stroke"], before["stroke"]);
            assert_eq!(undone["path"], before["path"]);
            s.execute("edit.redo", json!({})).unwrap();
            let redone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(redone["useAlignedRendering"], enabled);
            assert_eq!(redone["stroke"], source["stroke"]);
            assert_eq!(redone["path"], before["path"]);
        }
    }
}

/// Appending honors an explicit flag, preserves an omitted flag, and undoes both atomically.
#[test]
fn pixel_align_append_honors_explicit_values_and_preserves_omission() {
    for initial in [false, true] {
        for explicit in [None, Some(false), Some(true)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let before = s.execute("shape.create", json!({"kind":"rect","rect":[10,10,20,20],"useAlignedRendering":initial})).unwrap();
            let mut params = json!({"kind":"rect","rect":[50,50,20,20],"addTo":before["layer"]});
            if let Some(value) = explicit {
                params["useAlignedRendering"] = json!(value);
            }
            let after = s.execute("shape.create", params).unwrap();
            assert_eq!(after["useAlignedRendering"], explicit.unwrap_or(initial));
            assert_ne!(after["path"], before["path"]);
            s.execute("edit.undo", json!({})).unwrap();
            let undone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(undone["useAlignedRendering"], initial);
            assert_eq!(undone["path"], before["path"]);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.execute("shape.info", json!({})).unwrap()["useAlignedRendering"], explicit.unwrap_or(initial));
        }
    }
}

/// Invalid flag types leave history intact, and undo/redo changes only the rendering option.
#[test]
fn pixel_align_shape_commands_preserve_path_and_undo_and_validate_bool() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100,"background":"transparent"})).unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[10,10,60,60],"stroke":{"width":1}})).unwrap();
    let legacy = s.execute("shape.info", json!({})).unwrap();
    assert_eq!(legacy["useAlignedRendering"], false);
    let path = legacy["path"].clone();
    let on = s.execute("shape.edit", json!({"useAlignedRendering":true})).unwrap();
    assert_eq!(on["useAlignedRendering"], true);
    assert_eq!(on["path"], path);
    assert!(s.execute("shape.edit", json!({"useAlignedRendering":"true"})).is_err());
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["useAlignedRendering"], true);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["useAlignedRendering"], false);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["useAlignedRendering"], true);
}

/// Placement forwards either explicit alignment value through the custom-shape preset command.
#[test]
fn custom_shape_presets_forward_pixel_alignment_without_changing_preset_geometry() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100})).unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[10,10,60,60]})).unwrap();
    let preset = s.execute("shape.presets.new", json!({"name":"Pixel Alignment Regression"})).unwrap();
    let mut placed_path = None;
    for (index, value) in [true, false].into_iter().enumerate() {
        s.execute("shape.presets.place", json!({"preset":preset["name"],"rect":[10,10,60,60],"useAlignedRendering":value,"stroke":{"width":1}})).unwrap();
        assert_eq!(s.execute("shape.info", json!({})).unwrap()["useAlignedRendering"], value);
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, format!("{} {}", preset["name"].as_str().unwrap(), index + 1));
        let photocraft_doc::LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
        if let Some(expected) = &placed_path {
            assert_eq!(&shape.path, expected, "alignment must not alter placed preset geometry");
        } else {
            placed_path = Some(shape.path.clone());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The rendering rule, against the reference application's measured pixels: a black shape on a
// white 64x64 canvas, the red channel 0 = covered, 255 = empty. Our antialiasing is plain area
// coverage, so a half-covered pixel reads 128 where the reference reads 120-135: those cases check
// the structure (partly covered) rather than the exact byte.
// ---------------------------------------------------------------------------------------------

/// Every red value (0..255) of the flattened document, row by row.
fn reds(s: &Session) -> Vec<u8> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().map(|p| (p[0].clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

/// The red value at (x, y).
fn red(s: &Session, x: usize, y: usize) -> u8 {
    reds(s)[y * 64 + x]
}

/// The red values of row `y` at `cols`.
fn row(s: &Session, y: usize, cols: &[usize]) -> Vec<u8> {
    let px = reds(s);
    cols.iter().map(|&x| px[y * 64 + x]).collect()
}

/// A white 64x64 document with one black shape made by `shape.create` with `params`, Align Edges as given.
fn shape_doc(mut params: serde_json::Value, aligned: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":64,"background":"white"})).unwrap();
    if params.get("fill").is_none() {
        params["fill"] = json!("#000000");
    }
    params["useAlignedRendering"] = json!(aligned);
    s.execute("shape.create", params).unwrap();
    s
}

/// `[x0, y0, w, h]` filled black.
fn rect_doc(rect: [f64; 4], aligned: bool) -> Session {
    shape_doc(json!({"kind":"rect","rect":rect}), aligned)
}

/// Reference: rectangle 10.5..30.5, fill only. On: column 10 empty, column 30 covered (the shape
/// moved to 11..31). Off: both soft (reference 135 / 120).
#[test]
fn align_edges_moves_a_half_pixel_rectangle_to_whole_pixels() {
    let on = rect_doc([10.5, 10.5, 20.0, 20.0], true);
    assert_eq!(row(&on, 20, &[10, 11, 30, 31]), [255, 0, 0, 255]);
    assert_eq!(red(&on, 20, 10), 255, "the top edge moved down too");
    assert_eq!(red(&on, 20, 11), 0);
    let off = rect_doc([10.5, 10.5, 20.0, 20.0], false);
    for v in row(&off, 20, &[10, 30]) {
        assert!((100..=160).contains(&v), "soft edge without Align Edges: {v}");
    }
}

/// Reference: the left edge rounds to the nearest pixel (a half up) and the width rounds too.
/// 10.25..30.75 covers 10..31; 10.4..30.6 covers 10..30; 10.75..30.25 and 10.6..30.4 cover 11..31.
#[test]
fn align_edges_rounds_the_left_edge_and_the_width() {
    for (x0, x1, first, last) in [(10.25, 30.75, 10, 30), (10.4, 30.6, 10, 29), (10.75, 30.25, 11, 30), (10.6, 30.4, 11, 30)] {
        let s = rect_doc([x0, 10.0, x1 - x0, 20.0], true);
        let got = row(&s, 20, &[first - 1, first, last, last + 1]);
        assert_eq!(got, [255, 0, 0, 255], "{x0}..{x1} covers {first}..={last}");
    }
}

/// Reference: ellipse 10.5..40.5 moves to 11..41 (columns 10/11/40/41 = 255/6/6/255 on the middle
/// row) and is moved, not bent: the curve pixels (15,15) and (36,36), mirror images about the new
/// centre (26,26), read the same (82/82). The curve keeps its antialiasing.
#[test]
fn align_edges_moves_an_ellipse_without_bending_its_curve() {
    let on = shape_doc(json!({"kind":"ellipse","rect":[10.5,10.5,30.0,30.0]}), true);
    let mid = row(&on, 25, &[10, 11, 40, 41]);
    assert_eq!((mid[0], mid[3]), (255, 255), "{mid:?}");
    assert!(mid[1] < 40 && mid[2] < 40, "almost covered at the extremes: {mid:?}");
    let (a, b) = (red(&on, 15, 15), red(&on, 36, 36));
    assert!(a.abs_diff(b) <= 2, "symmetric about the moved centre: {a} vs {b}");
    assert!((60..=110).contains(&a), "the curve stays antialiased (reference 82): {a}");
    // Without Align Edges the same two pixels are not mirror images (centre 25.5).
    let off = shape_doc(json!({"kind":"ellipse","rect":[10.5,10.5,30.0,30.0]}), false);
    assert!(red(&off, 15, 15).abs_diff(red(&off, 36, 36)) > 20);
}

/// Reference: ellipse 10.75..40.25 covers 11..41, 10.4..40.6 covers 10..40, and an ellipse on whole
/// pixels is unchanged.
#[test]
fn align_edges_ellipse_bounds_round_like_rectangles() {
    for (x0, x1, first, last) in [(10.75, 40.25, 11usize, 40usize), (10.4, 40.6, 10, 39)] {
        let s = shape_doc(json!({"kind":"ellipse","rect":[x0, x0, x1 - x0, x1 - x0]}), true);
        let y = (first + last).div_ceil(2);
        let got = row(&s, y, &[first - 1, first, last, last + 1]);
        assert_eq!((got[0], got[3]), (255, 255), "{x0}..{x1}: {got:?}");
        assert!(got[1] < 60 && got[2] < 60, "{x0}..{x1}: {got:?}");
    }
    let on = shape_doc(json!({"kind":"ellipse","rect":[10.0,10.0,30.0,30.0]}), true);
    let off = shape_doc(json!({"kind":"ellipse","rect":[10.0,10.0,30.0,30.0]}), false);
    assert_eq!(reds(&on), reds(&off));
}

/// Reference: a 1 px centred stroke on rectangle 10..30 (no fill) paints columns 10 and 30 (its
/// outer edge 9.5..30.5 moves to 10..31); with a fill, column 30 is covered.
#[test]
fn align_edges_one_pixel_stroke_lands_on_whole_columns() {
    for fill in [false, true] {
        let mut p = json!({"kind":"rect","rect":[10.0,10.0,20.0,20.0],"stroke":{"width":1,"color":"#000000"}});
        if !fill {
            p["fill"] = serde_json::Value::Null;
        }
        let s = shape_doc(p, true);
        let got = row(&s, 20, &[9, 10, 11, 29, 30, 31]);
        let inside = if fill { 0 } else { 255 };
        assert_eq!(got, [255, 0, inside, inside, 0, 255], "fill={fill}");
    }
}

/// Reference: 1 px stroke on rectangle 10.25..30.25 paints columns 10 and 30; 2 px paints 9-10 and
/// 29-30; 3 px on rectangle 10..30 paints 9-11 and 29-31.
#[test]
fn align_edges_strokes_snap_by_their_outer_edge() {
    let stroked =
        |x: f64, width: u32| shape_doc(json!({"kind":"rect","rect":[x, 10.0, 20.0, 20.0],"fill":null,"stroke":{"width":width,"color":"#000000"}}), true);
    let s = stroked(10.25, 1);
    assert_eq!(row(&s, 20, &[9, 10, 11, 29, 30, 31]), [255, 0, 255, 255, 0, 255]);
    let s = stroked(10.25, 2);
    assert_eq!(row(&s, 20, &[8, 9, 10, 11, 28, 29, 30, 31]), [255, 0, 0, 255, 255, 0, 0, 255]);
    let s = stroked(10.0, 3);
    assert_eq!(row(&s, 20, &[8, 9, 10, 11, 12, 28, 29, 30, 31, 32]), [255, 0, 0, 0, 255, 255, 0, 0, 0, 255]);
}

/// Reference: fill + 1 px stroke at 10.5 is already on whole pixels (outer edge 10..31): column 10
/// covered, the same on and off.
#[test]
fn align_edges_leaves_an_already_aligned_shape_alone() {
    let p = json!({"kind":"rect","rect":[10.5,10.5,20.0,20.0],"stroke":{"width":1,"color":"#000000"}});
    let on = shape_doc(p.clone(), true);
    assert_eq!(red(&on, 10, 20), 0);
    assert_eq!(reds(&on), reds(&shape_doc(p, false)));
}

/// Reference: a rectangle rotated 30 degrees moves with its bounding box (row 20 columns 7/8 went
/// 252 -> 130 and 44 -> 0); its diagonal edges keep their antialiasing. Here a 20 px square about
/// (30, 30) rotated 30 degrees spans 16.34..43.66; aligned it spans 16..43.
#[test]
fn align_edges_moves_a_rotated_rectangle_by_its_bounding_box() {
    let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let corner = |dx: f64, dy: f64| [30.0 + dx * c - dy * s, 30.0 + dx * s + dy * c];
    let knots = [corner(-10.0, -10.0), corner(10.0, -10.0), corner(10.0, 10.0), corner(-10.0, 10.0)];
    let p = json!({"kind":"path","path":{"subpaths":[{"closed":true,"knots":knots}]}});
    let columns = |sess: &Session| {
        let px = reds(sess);
        (0..64).map(|x| (0..64).map(|y| px[y * 64 + x]).min().unwrap()).collect::<Vec<_>>()
    };
    let on = columns(&shape_doc(p.clone(), true));
    assert_eq!((on[15], on[43]), (255, 255), "nothing outside 16..43");
    assert!(on[16] < 255 && on[42] < 255);
    let off = columns(&shape_doc(p.clone(), false));
    assert!(off[43] < 255, "unaligned it reaches into column 43");
    let all = reds(&shape_doc(p, true));
    assert!(all.iter().filter(|v| **v > 0 && **v < 255).count() > 40, "diagonal edges stay antialiased");
}

/// Reference: Pen triangle (10.5,10.5) (30.5,10.5) (10.5,30.5), row 15: column 25 covered, column
/// 26 half (127). The whole triangle moved by half a pixel on both axes.
#[test]
fn align_edges_moves_a_pen_triangle() {
    let p = json!({"kind":"path","path":{"subpaths":[{"closed":true,"knots":[[10.5,10.5],[30.5,10.5],[10.5,30.5]]}]}});
    let s = shape_doc(p, true);
    assert_eq!(red(&s, 25, 15), 0);
    assert!(red(&s, 26, 15).abs_diff(127) <= 8, "{}", red(&s, 26, 15));
}

/// Reference: two rectangles in one layer: only the outer bounds snap, the inner edges stay soft
/// (225/225). Here 10.5..21.25 and 25.75..40.5 move by +0.5 to 11..21.75 and 26.25..41.
#[test]
fn align_edges_does_not_snap_inner_edges_of_several_subpaths() {
    let rect = |x0: f64, x1: f64| json!({"closed":true,"knots":[[x0,10.0],[x1,10.0],[x1,30.0],[x0,30.0]]});
    let p = json!({"kind":"path","path":{"subpaths":[rect(10.5, 21.25), rect(25.75, 40.5)]}});
    let s = shape_doc(p, true);
    assert_eq!(row(&s, 20, &[10, 11, 40, 41]), [255, 0, 0, 255], "outer edges on whole pixels");
    for x in [21, 26] {
        let v = red(&s, x, 20);
        assert!(v > 0 && v < 255, "inner edge at column {x} stays soft: {v}");
    }
}

/// Predicted (the reference's Line tool could not be driven by script): a 1 px line at y = 20 paints
/// the band 19.5..20.5, which Align Edges moves to row 20.
#[test]
fn align_edges_line_lands_on_one_row() {
    let s = shape_doc(json!({"kind":"line","from":[10.0,20.0],"to":[50.0,20.0],"weight":1}), true);
    let col: Vec<u8> = (18..23).map(|y| red(&s, 30, y)).collect();
    assert_eq!(col, [255, 255, 0, 255, 255]);
}

/// Turning Align Edges on changes only the rendering: the stored path stays (10.5 stays 10.5), and
/// turning it off again renders the original soft pixels.
#[test]
fn align_edges_is_render_only_and_off_restores_the_soft_pixels() {
    let mut s = rect_doc([10.5, 10.5, 20.0, 20.0], false);
    let soft = reds(&s);
    let path = s.execute("shape.info", json!({})).unwrap()["path"].clone();
    let on = s.execute("shape.edit", json!({"useAlignedRendering":true})).unwrap();
    assert_eq!(on["path"], path, "the anchors stay where they were");
    assert_ne!(reds(&s), soft);
    s.execute("shape.edit", json!({"useAlignedRendering":false})).unwrap();
    assert_eq!(reds(&s), soft);
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(reds(&s), soft, "undo brings the aligned pixels back");
}
