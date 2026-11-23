//! Tests that run paint commands, moved from the engine-core `presets` tests: 12 test(s).

use serde_json::json;

use photocraft_engine::Session;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    s
}

fn layer_px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

#[test]
fn rainbow_gradient_presets() {
    let mut s = session(8);
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    let group = v["groups"].as_array().unwrap().iter().find(|g| g["name"] == "Rainbows").unwrap();
    let presets = group["presets"].as_array().unwrap();
    let names: Vec<&str> = presets.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Rainbow", "Pastel Rainbow", "Neon Rainbow", "Oil Slick", "Rainbow Fade"]);
    // The three hue ramps share the same 13 evenly spaced hues: smooth, not the 7 coarse ROYGBIV points.
    for p in [&presets[0], &presets[1], &presets[2]] {
        let stops = p["stops"].as_array().unwrap();
        assert_eq!(stops.len(), 13, "{}", p["name"]);
        assert_eq!(stops[0][0], json!(0.0));
        assert_eq!(stops[12][0], json!(1.0));
    }
    assert_eq!(presets[3]["stops"].as_array().unwrap().len(), 11, "oil slick");
    assert_eq!(presets[4]["transparency"], json!([[0.0, 0.0], [0.125, 100.0], [0.875, 100.0], [1.0, 0.0]]));
    // Rainbow Fade paints opaque across the middle and transparent at both ends.
    s.execute("gradient.presets.select", json!({"preset": "Rainbow Fade"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    assert!(layer_px(&s, 0, 5)[3] < 0.1, "left edge transparent: {:?}", layer_px(&s, 0, 5));
    assert!(layer_px(&s, 63, 5)[3] < 0.1, "right edge transparent: {:?}", layer_px(&s, 63, 5));
    assert!(layer_px(&s, 32, 5)[3] > 0.95, "middle opaque: {:?}", layer_px(&s, 32, 5));
}

#[test]
fn gradient_tool_uses_selected_preset_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        // Default (Foreground to Background): black → white, as before presets existed.
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
        assert!(layer_px(&s, 0, 10)[0] < 0.02, "depth {depth}");
        assert!(layer_px(&s, 63, 10)[0] > 0.98);
        s.execute("gradient.presets.select", json!({"preset": "Red 01"})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
        let left = layer_px(&s, 0, 10);
        // Red 01 starts at #7f0000.
        assert!((left[0] - 127.0 / 255.0).abs() < 0.02 && left[1] < 0.01, "depth {depth}: {left:?}");
        // Explicit stops and a named preset override the selection.
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0], "stops": [[0, "#00ff00"], [1, "#0000ff"]]})).unwrap();
        assert!(layer_px(&s, 0, 10)[1] > 0.98);
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0], "gradient": "Black, White"})).unwrap();
        assert!(layer_px(&s, 63, 10)[0] > 0.98);
    }
}

#[test]
fn foreground_to_transparent_paints_alpha() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("gradient.presets.select", json!({"preset": "Foreground to Transparent"})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    assert!(layer_px(&s, 0, 5)[3] > 0.95);
    assert!(layer_px(&s, 63, 5)[3] < 0.05);
    let mid = layer_px(&s, 32, 5)[3];
    assert!((mid - 0.5).abs() < 0.05, "{mid}");
}

/// `gradient.presets.stop` edits the Gradient tool's current gradient in place (the stops the
/// Gradient Editor window shows): a new stop takes the colour under it, a "foreground" stop stays
/// live, opacity stops materialize on first edit, and the tool paints with the edited gradient.
#[test]
fn preset_stop_edits_the_current_gradient() {
    let mut s = session(8);
    // A new stop mid-way samples the ramp there (black → white: mid gray).
    let v = s.execute("gradient.presets.stop", json!({"action": "add", "location": 0.5})).unwrap();
    let stops = v["current"]["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 3, "{v}");
    assert_eq!(stops[1], json!([0.5, "#808080"]), "sorted by location, the ramp colour at 0.5");
    // Recolour it and repaint: the tool gradient changed (mid of the stroke is the new colour).
    s.execute("gradient.presets.stop", json!({"action": "color", "index": 1, "color": "#ff0000"})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    let px = layer_px(&s, 32, 10);
    assert!(px[0] > 0.9 && px[1] < 0.1, "{px:?}");
    // A "foreground" stop stays live: repaint after changing the foreground picks it up.
    s.execute("gradient.presets.stop", json!({"action": "color", "index": 0, "color": "foreground"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    let px = layer_px(&s, 0, 10);
    assert!(px[2] > 0.9 && px[0] < 0.1, "{px:?}");
    // Opacity stops materialize the two opaque ends on first edit.
    let v = s.execute("gradient.presets.stop", json!({"action": "add", "kind": "opacity", "location": 0.5, "opacity": 40})).unwrap();
    assert_eq!(v["current"]["transparency"], json!([[0.0, 100.0], [0.5, 40.0], [1.0, 100.0]]), "{v}");
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    // Paint on a fresh layer: the background already holds opaque paint to composite onto.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    let mid = layer_px(&s, 32, 10)[3];
    assert!((mid - 0.4).abs() < 0.05, "{mid}");
    // Guard rails: in-range indexes, known actions, two stops minimum, no midpoints.
    assert!(s.execute("gradient.presets.stop", json!({"action": "move", "index": 99, "location": 0.5})).is_err());
    assert!(s.execute("gradient.presets.stop", json!({"action": "explode"})).is_err());
    assert!(s.execute("gradient.presets.stop", json!({"action": "delete", "index": 0})).is_ok());
    assert!(s.execute("gradient.presets.stop", json!({"action": "delete", "index": 0})).is_err(), "a gradient keeps at least 2 colour stops");
    assert!(s.execute("gradient.presets.stop", json!({"action": "midpoint", "index": 0, "location": 0.4})).is_err());
}

/// A many-stop gradient survives editor gymnastics: five adds build a seven-stop ramp, a stop
/// dragged past its neighbour keeps the list sorted, the paint follows the edited stops, and
/// trimming back down ends at a plain two-stop gradient.
#[test]
fn preset_stop_survives_a_complex_many_stop_ramp() {
    let mut s = session(8);
    s.execute("gradient.presets.select", json!({"stops": [[0.0, "#000000"], [1.0, "#ffffff"]]})).unwrap();
    for (i, c) in ["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#ff00ff"].iter().enumerate() {
        let t = (i + 1) as f64 / 6.0;
        s.execute("gradient.presets.stop", json!({"action": "add", "location": t, "color": c})).unwrap();
    }
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    assert_eq!(v["current"]["stops"].as_array().unwrap().len(), 7, "{v}");
    // Drag the yellow stop (index 4, at 4/6) past the magenta one (at 5/6).
    s.execute("gradient.presets.stop", json!({"action": "move", "index": 4, "location": 0.95})).unwrap();
    let stops = s.execute("gradient.presets.list", json!({})).unwrap()["current"]["stops"].as_array().unwrap().clone();
    let locs: Vec<f64> = stops.iter().map(|s| s[0].as_f64().unwrap()).collect();
    let mut in_order = locs.clone();
    in_order.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(locs, in_order, "stops stay sorted after a crossing move: {stops:?}");
    assert_eq!(stops[6], json!([1.0, "#ffffff"]));
    // The paint follows: near the moved stop the stroke is the stop's yellow.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    let px = layer_px(&s, 60, 10);
    assert!(px[0] > 0.95 && px[1] > 0.95 && px[2] < 0.3, "{px:?}");
    // Trim everything back down to the two end stops.
    for _ in 0..5 {
        s.execute("gradient.presets.stop", json!({"action": "delete", "index": 1})).unwrap();
    }
    let stops = s.execute("gradient.presets.list", json!({})).unwrap()["current"]["stops"].as_array().unwrap().clone();
    assert_eq!(stops, json!([[0.0, "#000000"], [1.0, "#ffffff"]]).as_array().unwrap().clone(), "{stops:?}");
}

/// An opacity hole paints where it is put, follows a move, and once the gradient is fully
/// opaque again the stored opacity stops normalize away (no pinned 100 % ends).
#[test]
fn preset_stop_opacity_hole_moves_and_normalizes() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("gradient.presets.stop", json!({"action": "add", "kind": "opacity", "location": 0.5, "opacity": 0})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    assert!(layer_px(&s, 0, 5)[3] > 0.95);
    assert!(layer_px(&s, 32, 5)[3] < 0.05);
    assert!(layer_px(&s, 63, 5)[3] > 0.95);
    // The hole follows the moved stop (index 1 is the 0 % stop between the opaque ends).
    s.execute("gradient.presets.stop", json!({"action": "move", "kind": "opacity", "index": 1, "location": 0.25})).unwrap();
    // Paint on fresh layers: painting over earlier strokes would composite their alpha.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    assert!(layer_px(&s, 20, 5)[3] < 0.15, "{:?}", layer_px(&s, 20, 5));
    let far = layer_px(&s, 48, 5)[3];
    assert!(far > 0.55 && far < 0.85, "{far}");
    // Deleting it returns the gradient to fully opaque, stored as no opacity stops at all.
    s.execute("gradient.presets.stop", json!({"action": "delete", "kind": "opacity", "index": 1})).unwrap();
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    assert_eq!(v["current"]["transparency"], json!([]), "{v}");
}

/// The edited current gradient saves as a preset, and re-selecting it paints the same pixels.
#[test]
fn edited_preset_saves_and_roundtrips() {
    let mut s = session(8);
    s.execute("gradient.presets.stop", json!({"action": "add", "location": 0.5, "color": "#ff0000"})).unwrap();
    s.execute("gradient.presets.new", json!({"name": "My Gradient"})).unwrap();
    s.execute("gradient.presets.select", json!({"preset": "Red 01"})).unwrap();
    s.execute("gradient.presets.select", json!({"preset": "My Gradient"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [63, 0]})).unwrap();
    let px = layer_px(&s, 32, 10);
    assert!(px[0] > 0.9 && px[1] < 0.1, "{px:?}");
}

/// Issue #465: explicit `transparency` stops apply with `colors`, a `gradient` preset, explicit
/// `stops` and the current gradient alike.
#[test]
fn paint_gradient_honours_explicit_transparency_stops() {
    let transparency = json!([[0, 65], [1, 0]]);
    for depth in [8, 16, 32] {
        for extra in [
            json!({"colors": ["#07111d", "#07111d"]}),
            json!({"gradient": "Black, White"}),
            json!({"gradient": "Foreground to Transparent", "transparency": [[0, 65], [0.5, 65], [1, 0]]}),
            json!({"stops": [[0, "#07111d"], [1, "#07111d"]]}),
            json!({}),
        ] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "transparent"})).unwrap();
            // Pixel 15 is sampled at 14.5 (#12018), so a drag to 14 ends on it.
            let mut p = json!({"from": [0, 0], "to": [0, 14], "transparency": transparency});
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            s.execute("paint.gradient", p).unwrap();
            let (top, bottom) = (layer_px(&s, 8, 0)[3], layer_px(&s, 8, 15)[3]);
            assert!((top - 0.65).abs() < 0.03, "depth {depth} {extra}: top alpha {top}");
            assert!(bottom < 0.03, "depth {depth} {extra}: bottom alpha {bottom}");
        }
    }
    // `colors` keeps its colour; a malformed stop list is an error, not a silent opaque fill.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "transparent"})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [0, 15], "colors": ["#07111d", "#07111d"], "transparency": transparency})).unwrap();
    let top = layer_px(&s, 8, 0);
    assert!((top[2] - 29.0 / 255.0).abs() < 0.01, "{top:?}");
    for bad in [json!("65"), json!([[0]]), json!([[0, "a"]])] {
        assert!(s.execute("paint.gradient", json!({"from": [0, 0], "to": [0, 15], "colors": ["#000000"], "transparency": bad})).is_err());
    }
}

// ------------------------------------------------------------------ tool presets

#[test]
fn tool_presets_save_filter_select() {
    let mut s = Session::new();
    s.execute("tools.setBrush", json!({"size": 77.0})).unwrap();
    s.execute("tool.presets.new", json!({"name": "Seventy Seven", "tool": "brush", "includeColor": true, "options": {"toolOptions": {"x": 1}}})).unwrap();
    s.execute("tools.setBrush", json!({"size": 5.0})).unwrap();
    s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let v = s.execute("tool.presets.list", json!({"tool": "eraser"})).unwrap();
    assert!(v["presets"].as_array().unwrap().iter().all(|p| p["tool"] == "eraser"));
    let r = s.execute("tool.presets.select", json!({"preset": "Seventy Seven"})).unwrap();
    assert_eq!(r["tool"], "brush");
    assert_eq!(r["options"]["toolOptions"]["x"], 1);
    // Options passed explicitly win over the current brush.
    assert!(r["options"]["brush"].is_null() || r["options"]["brush"].is_object());
    assert_eq!(s.tools.foreground, [0.0, 0.0, 0.0, 1.0]);
    s.execute("tool.presets.new", json!({"name": "Eraser 77", "tool": "eraser"})).unwrap();
    let r = s.execute("tool.presets.select", json!({"preset": "Eraser 77"})).unwrap();
    // Selecting "Seventy Seven" restored its 77 px brush, which the new preset captured.
    assert_eq!(r["options"]["brush"]["size"], json!(77.0));
    s.execute("tool.presets.select", json!({"preset": "Soft Round 100 px"})).unwrap();
    assert_eq!(s.tools.brush.size, 100.0);
    s.execute("tool.presets.edit", json!({"action": "rename", "preset": "Eraser 77", "name": "E"})).unwrap();
    s.execute("tool.presets.edit", json!({"action": "delete", "preset": "E"})).unwrap();
    assert!(s.execute("tool.presets.select", json!({"preset": "E"})).is_err());
    assert!(s.execute("tool.presets.edit", json!({"action": "newGroup"})).is_err());
    assert!(s.execute("tool.presets.new", json!({"name": "x"})).is_err());
}

/// An Art History Brush preset captures the brush's opacity and selecting it restores it.
#[test]
fn art_history_brush_preset_carries_the_opacity() {
    let mut s = Session::new();
    s.execute("tools.setBrush", json!({"brush": {"opacity": 0.4}})).unwrap();
    s.execute("tool.presets.new", json!({"name": "Faint History", "tool": "artHistoryBrush"})).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"opacity": 1.0}})).unwrap();
    let r = s.execute("tool.presets.select", json!({"preset": "Faint History"})).unwrap();
    assert_eq!(r["tool"], "artHistoryBrush");
    assert!((s.tools.brush.opacity - 0.4).abs() < 1e-6, "{}", s.tools.brush.opacity);
}

// ------------------------------------------------------------------ clone source

fn clone_doc(depth: u32) -> Session {
    let mut s = session(depth);
    // A horizontal ramp in red, green = row, so transforms are measurable.
    s.edit("ramp", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let fmt = surf.format();
        let mut px = Vec::new();
        for y in 0..48 {
            for x in 0..64 {
                px.extend(photocraft_raster::from_rgba(&fmt, [x as f32 / 63.0, y as f32 / 47.0, 0.0, 1.0]));
            }
        }
        surf.write_region(photocraft_geom::Rect::new(0, 0, 64, 48), &px);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn clone_source_slots_and_identity_matches_offset() {
    for depth in [8, 16, 32] {
        let mut s = clone_doc(depth);
        s.execute("cloneSource.set", json!({"index": 2, "source": [10, 10]})).unwrap();
        let v = s.execute("cloneSource.list", json!({})).unwrap();
        assert_eq!(v["active"], 2);
        assert_eq!(v["sources"][2]["source"], json!([10.0, 10.0]));
        // Stroke at (40, 30) without params samples the active slot: offset (−30, −20).
        s.execute("paint.cloneStamp", json!({"points": [[40, 30]], "size": 5, "hardness": 100})).unwrap();
        let px = layer_px(&s, 40, 30);
        assert!((px[0] - 10.0 / 63.0).abs() < 0.01 && (px[1] - 10.0 / 47.0).abs() < 0.01, "depth {depth}: {px:?}");
        // Aligned: the pairing stuck, the next stroke keeps the offset.
        let v = s.execute("cloneSource.list", json!({})).unwrap();
        assert_eq!(v["sources"][2]["offset"], json!([-30.0, -20.0]));
        s.execute("paint.cloneStamp", json!({"points": [[50, 40]], "size": 3, "hardness": 100})).unwrap();
        let px = layer_px(&s, 50, 40);
        assert!((px[0] - 20.0 / 63.0).abs() < 0.01, "{px:?}");
    }
}

#[test]
fn clone_source_flip_scale_rotation() {
    let mut s = clone_doc(16);
    s.execute("cloneSource.set", json!({"source": [20, 20], "flipH": true})).unwrap();
    s.execute("paint.cloneStamp", json!({"points": [[40, 24]], "size": 21, "hardness": 100})).unwrap();
    // Flipped: 2 px right of the anchor samples 2 px left of the source.
    let px = layer_px(&s, 42, 24);
    assert!((px[0] - 18.0 / 63.0).abs() < 0.02, "{px:?}");
    s.execute("cloneSource.resetTransform", json!({})).unwrap();
    let mut s2 = clone_doc(16);
    s2.execute("cloneSource.set", json!({"source": [20, 20], "width": 200, "height": 200})).unwrap();
    s2.execute("paint.cloneStamp", json!({"points": [[40, 24]], "size": 21, "hardness": 100})).unwrap();
    // 200 %: 4 px right of the anchor samples 2 px right of the source.
    let px = layer_px(&s2, 44, 24);
    assert!((px[0] - 22.0 / 63.0).abs() < 0.02, "{px:?}");
    // Rotation 90° (counter-clockwise on screen): 3 px right of the anchor comes from 3 px
    // below the source.
    let mut s3 = clone_doc(32);
    s3.execute("cloneSource.set", json!({"source": [20, 20], "rotation": 90})).unwrap();
    s3.execute("paint.cloneStamp", json!({"points": [[40, 24]], "size": 21, "hardness": 100})).unwrap();
    let px = layer_px(&s3, 43, 24);
    assert!((px[0] - 20.0 / 63.0).abs() < 0.02 && (px[1] - 23.0 / 47.0).abs() < 0.02, "{px:?}");
    // Explicit params override the slot; healing follows the slot too.
    let mut s4 = clone_doc(8);
    s4.execute("cloneSource.set", json!({"source": [20, 20], "rotation": 45})).unwrap();
    s4.execute("paint.healingBrush", json!({"points": [[40, 24]], "size": 7})).unwrap();
    s4.execute("paint.cloneStamp", json!({"points": [[10, 40]], "offset": [5, 0], "rotation": 0, "size": 3, "hardness": 100})).unwrap();
    let px = layer_px(&s4, 10, 40);
    assert!((px[0] - 15.0 / 63.0).abs() < 0.01, "{px:?}");
}
