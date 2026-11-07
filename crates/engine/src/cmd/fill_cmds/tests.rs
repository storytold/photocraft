use super::*;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    s
}

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    l.surface().unwrap().rgba(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

#[test]
fn history_fill_reads_only_the_selection_inside_the_canvas() {
    let mut s = session();
    s.edit("selection", |doc, _| {
        let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_pixel(-2_000_000_000, -2_000_000_000, &[1.0]);
        selection.write_pixel(4, 4, &[1.0]);
        doc.selection = Some(selection);
        Ok(())
    })
    .unwrap();
    // Reading the full sparse selection rectangle overflows the allocation size;
    // only its intersection with this small canvas is meaningful to Fill.
    s.execute(CMD, json!({"contents": "history"})).unwrap();
    assert!(s.undo());
    assert!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(4, 4, 0) > 0.0);
}

#[test]
fn every_constant_contents() {
    let mut s = session();
    s.execute("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0], "background": [0.0, 0.0, 1.0, 1.0]})).unwrap();
    for (c, want) in [
        ("foreground", [1.0, 0.0, 0.0, 1.0]),
        ("background", [0.0, 0.0, 1.0, 1.0]),
        ("black", [0.0, 0.0, 0.0, 1.0]),
        ("gray", [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]),
        ("white", [1.0; 4]),
    ] {
        s.execute(CMD, json!({"contents": c})).unwrap();
        assert!(close(px(&s, 5, 5), want), "{c}: {:?}", px(&s, 5, 5));
    }
    s.execute(CMD, json!({"contents": "color", "color": "#00ff00"})).unwrap();
    assert!(close(px(&s, 5, 5), [0.0, 1.0, 0.0, 1.0]));
    // The old form: a colour, or the foreground.
    s.execute(CMD, json!({"color": "#ffff00"})).unwrap();
    assert!(close(px(&s, 5, 5), [1.0, 1.0, 0.0, 1.0]));
}

#[test]
fn opacity_mode_and_preserve_transparency() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    s.execute(CMD, json!({"contents": "white"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    // Preserve Transparency: the empty right half stays empty.
    s.execute(CMD, json!({"contents": "black", "opacity": 50, "preserveTransparency": true})).unwrap();
    assert!(close(px(&s, 5, 5), [0.5, 0.5, 0.5, 1.0]), "{:?}", px(&s, 5, 5));
    assert_eq!(px(&s, 30, 5)[3], 0.0);
    // Multiply with white leaves the grey.
    s.execute(CMD, json!({"contents": "white", "mode": "multiply", "preserveTransparency": true})).unwrap();
    assert!(close(px(&s, 5, 5), [0.5, 0.5, 0.5, 1.0]));
    // Without it, the transparent half is filled.
    s.execute(CMD, json!({"contents": "white", "opacity": 100})).unwrap();
    assert_eq!(px(&s, 30, 5), [1.0; 4]);
    // Dissolve paints whole pixels only.
    s.execute(CMD, json!({"contents": "black", "mode": "dissolve", "opacity": 50})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let blacks = (0..40).flat_map(|x| (0..30).map(move |y| (x, y))).filter(|&(x, y)| surf.rgba(x, y)[0] < 0.01).count();
    assert!(blacks > 300 && blacks < 900, "{blacks}");
    assert!((0..40).all(|x| {
        let v = surf.rgba(x, 3)[0];
        v.is_finite() && !(0.01..=0.99).contains(&v)
    }));
}

#[test]
fn preserve_transparency_blends_source_atop_on_semi_transparent_pixels() {
    // White at 50%, then black at 50% keeping alpha: Photoshop gives (127,127,127,128), the
    // colour moved halfway, at every depth and in non-Normal modes too (#5783).
    for depth in [8, 16, 32] {
        for mode in ["normal", "multiply"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
            s.execute(CMD, json!({"contents": "white", "opacity": 50})).unwrap();
            s.execute(CMD, json!({"contents": "black", "opacity": 50, "mode": mode, "preserveTransparency": true})).unwrap();
            assert!(close(px(&s, 3, 3), [0.5, 0.5, 0.5, 0.5]), "{depth} {mode} {:?}", px(&s, 3, 3));
        }
    }
}

#[test]
fn history_pattern_and_content_aware() {
    let mut s = session();
    s.execute(CMD, json!({"contents": "white"})).unwrap();
    s.execute(CMD, json!({"contents": "black"})).unwrap();
    // History: back to state 1 (after the white fill).
    s.execute(CMD, json!({"contents": "history", "state": 1})).unwrap();
    assert_eq!(px(&s, 5, 5), [1.0; 4]);
    // The default source is the oldest state: the empty layer.
    s.execute(CMD, json!({"contents": "history"})).unwrap();
    assert_eq!(px(&s, 5, 5)[3], 0.0);
    assert!(s.execute(CMD, json!({"contents": "history", "state": 999})).is_err());
    let r = s.execute(CMD, json!({"contents": "pattern", "pattern": "Diagonal Lines"})).unwrap();
    assert!(r["pattern"].is_string());
    assert!(px(&s, 5, 5)[3] > 0.0);
    // Content-Aware fills the hole from what is around it.
    s.execute(CMD, json!({"contents": "color", "color": "#336699"})).unwrap();
    s.execute("select.rect", json!({"x": 15, "y": 10, "width": 6, "height": 6})).unwrap();
    s.execute(CMD, json!({"contents": "white"})).unwrap();
    s.execute(CMD, json!({"contents": "contentAware", "colorAdaptation": false})).unwrap();
    assert!(close(px(&s, 17, 12), [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0, 1.0]), "{:?}", px(&s, 17, 12));
}

#[test]
fn history_fill_reads_a_source_when_its_pixels_were_locked() {
    // Given a white layer locked in history state 1, then unlocked and filled red.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    s.execute("layer.lockLayers", json!({"pixels": false})).unwrap();
    s.execute(CMD, json!({"color": "#ff0000"})).unwrap();

    // When Fill reads the locked historical pixels.
    let result = s.execute(CMD, json!({"contents": "history", "state": 1}));

    // Then it restores white; the historical lock limits edits, not reads.
    assert!(result.is_ok(), "{result:?}");
    assert!(close(px(&s, 8, 8), [1.0; 4]), "{:?}", px(&s, 8, 8));
}

#[test]
fn history_fill_refuses_a_locked_destination_when_source_is_unlocked() {
    // Given an unlocked white source and a red, currently locked destination.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute(CMD, json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();

    // When Fill reads the unlocked source and attempts to write to the locked destination.
    let result = s.execute(CMD, json!({"contents": "history", "state": 0}));

    // Then the destination lock still rejects the edit and preserves red pixels.
    assert!(result.is_err());
    assert!(close(px(&s, 8, 8), [1.0, 0.0, 0.0, 1.0]), "{:?}", px(&s, 8, 8));
}

/// `cargo test -p photocraft-engine --release --lib fill_cmds::tests::timing_24mp -- --ignored --nocapture`
#[test]
#[ignore]
fn timing_24mp() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 4000, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute(CMD, json!({"contents": "white"})).unwrap();
    let time = |s: &mut Session, label: &str, p: Value| {
        let mut v = Vec::new();
        for i in 0..5 {
            let mut p = p.clone();
            if p.get("contents").is_none() {
                p["color"] = json!(if i % 2 == 0 { "#336699" } else { "#996633" });
            }
            let t = std::time::Instant::now();
            s.execute(CMD, p).unwrap();
            v.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        v.sort_by(f64::total_cmp);
        println!("{label:<48} median {:>8.1} ms  min {:>8.1} ms", v[2], v[0]);
    };
    time(&mut s, "colour, no selection", json!({}));
    time(&mut s, "colour, preserve transparency", json!({"preserveTransparency": true}));
    time(&mut s, "colour 50% multiply", json!({"opacity": 50, "mode": "multiply"}));
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 5000, "height": 3000})).unwrap();
    time(&mut s, "colour, 15 MP rectangle selection", json!({}));
    s.execute("select.deselect", json!({})).unwrap();
    // The old per-pixel f32 path, for comparison.
    let st = s.active().unwrap();
    let mut surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
    let t = std::time::Instant::now();
    crate::pixels::try_fill_surface(&mut surf, Rect::new(0, 0, 6000, 4000), [0.2, 0.4, 0.6, 1.0], None, false).unwrap();
    println!("{:<48} {:>8.1} ms", "fallible fill_surface, no selection", t.elapsed().as_secs_f64() * 1000.0);
}

#[test]
fn bad_params_are_errors() {
    let mut s = session();
    for p in [
        json!({"contents": "plaid"}),
        json!({"contents": 3}),
        json!({"mode": "sideways"}),
        json!({"mode": "passThrough"}),
        json!({"opacity": "lots"}),
        json!({"preserveTransparency": "yes"}),
        json!({"contents": "contentAware"}),
        json!({"contents": "pattern", "pattern": "no such pattern"}),
        json!({"contents": "history", "state": -1}),
        json!({"contents": "contentAware", "colorAdaptation": 1}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
    }
    // Everything selected leaves nothing to sample from.
    s.execute("select.all", json!({})).unwrap();
    assert!(s.execute(CMD, json!({"contents": "contentAware"})).is_err());
    // Out-of-range opacity is clamped.
    s.execute(CMD, json!({"contents": "white", "opacity": 1e300})).unwrap();
}

/// A session whose canvas is `size` and whose selection is the given (sparse) rectangles, as
/// a loaded `.pcraft` can declare (#963): the canvas size is not capped on load and the
/// selection tiles can lie far apart. Nothing here allocates more than a few tiles.
fn huge_selection(size: (u32, u32), rects: &[Rect]) -> Session {
    let mut s = session();
    s.execute(CMD, json!({"contents": "color", "color": "#336699"})).unwrap();
    let st = s.active_mut().unwrap();
    let doc = std::sync::Arc::make_mut(&mut st.doc);
    doc.size = photocraft_geom::Size::new(size.0, size.1);
    let mut sel = Surface::new(photocraft_color::PixelFormat::GRAY8);
    for r in rects {
        sel.fill_rect(*r, &[1.0]);
    }
    doc.selection = Some(sel);
    s
}

/// Content-Aware fill on a selection whose extent overflows the sampling-window arithmetic
/// (#963) returns the size error and leaves the document and history untouched.
#[test]
fn content_aware_huge_extent_is_an_error() {
    // Largest extent whose `ext * 3` still fits in u32.
    const EDGE: i32 = (u32::MAX / 3) as i32;
    let max = i32::MAX;
    let cases: [(&str, (u32, u32), Vec<Rect>); 7] = [
        // The issue's repro: a second selection tile 5 600 000 tiles to the right.
        ("issue repro", (2_000_000_000, 2_000_000_000), vec![Rect::new(0, 0, 8, 8), Rect::new(5_600_000 * TILE_SIZE, 0, 5_600_000 * TILE_SIZE + 8, 8)]),
        ("wide, just above", (2_000_000_000, 2_000_000_000), vec![Rect::new(0, 0, 1, 1), Rect::new(EDGE, 0, EDGE + 1, 1)]),
        ("tall, just above", (2_000_000_000, 2_000_000_000), vec![Rect::new(0, 0, 1, 1), Rect::new(0, EDGE, 1, EDGE + 1)]),
        ("wide, just below", (2_000_000_000, 2_000_000_000), vec![Rect::new(0, 0, 1, 1), Rect::new(EDGE - 1, 0, EDGE, 1)]),
        ("tall, just below", (2_000_000_000, 2_000_000_000), vec![Rect::new(0, 0, 1, 1), Rect::new(0, EDGE - 1, 1, EDGE)]),
        // The largest canvas a Rect can hold, selected corner to corner, plus a piece off the
        // canvas at the most negative coordinate.
        (
            "extreme coordinates",
            (max as u32, max as u32),
            vec![Rect::new(i32::MIN, i32::MIN, i32::MIN + 1, i32::MIN + 1), Rect::new(0, 0, 1, 1), Rect::new(max - 2, max - 2, max - 1, max - 1)],
        ),
        // A canvas wider than a Rect can hold is treated as empty, never as a panic.
        ("canvas beyond i32", (u32::MAX, u32::MAX), vec![Rect::new(0, 0, 1, 1), Rect::new(max - 2, 0, max - 1, 1)]),
    ];
    for (name, size, rects) in cases {
        let mut s = huge_selection(size, &rects);
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        let r = s.execute(CMD, json!({"contents": "contentAware", "colorAdaptation": false}));
        let err = r.expect_err(name).to_string();
        assert!(!err.contains("internal error"), "{name}: {err}");
        if name != "canvas beyond i32" {
            assert!(err.contains("too large for Content-Aware fill"), "{name}: {err}");
        }
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "{name}: the document changed");
        assert_eq!(st.history.past_len(), past, "{name}: a history step was recorded");
        assert!(close(px(&s, 5, 5), [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0, 1.0]), "{name}");
    }
    // Control: an ordinary selection still fills from its surroundings.
    let mut s = huge_selection((40, 30), &[Rect::new(15, 10, 21, 16)]);
    s.execute(CMD, json!({"contents": "white"})).unwrap();
    s.execute(CMD, json!({"contents": "contentAware", "colorAdaptation": false})).unwrap();
    assert!(close(px(&s, 17, 12), [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0, 1.0]), "{:?}", px(&s, 17, 12));
}

/// A Stroke, Fill or Clear inside a selection keeps the tiles the selection doesn't reach as
/// they were (shared with the history), instead of storing a copy of each (#4517).
#[test]
fn unselected_tiles_stay_shared() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 1024, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let surf = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
    for (cmd, params) in
        [("edit.fill", json!({"color": "#ff0000"})), ("edit.stroke", json!({"width": 4, "color": "#00ff00", "location": "inside"})), ("edit.clear", json!({}))]
    {
        // Two small selections in opposite corners (256 px tiles): the selection's bounds
        // span every tile, but only the two corner tiles have anything selected.
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 100})).unwrap();
        s.execute("select.rect", json!({"x": 900, "y": 900, "width": 100, "height": 100, "mode": "add"})).unwrap();
        let before = surf(&s);
        s.execute(cmd, params).unwrap();
        let after = surf(&s);
        let corners = [photocraft_geom::Rect::new(0, 0, 256, 256), photocraft_geom::Rect::new(768, 768, 1024, 1024)];
        let mut kept = 0;
        for (c, t) in before.tiles() {
            if corners.iter().all(|r| c.rect().intersect(r).is_empty()) {
                assert!(after.tile(*c).is_some_and(|a| std::sync::Arc::ptr_eq(a, t)), "{cmd}: unselected tile {c:?} was copied");
                kept += 1;
            }
        }
        assert_eq!(kept, 14, "{cmd}: the 14 tiles between the corners");
        for r in corners {
            assert_ne!(before.read_region(r), after.read_region(r), "{cmd} changed the selected corner {r:?}");
        }
        s.execute("select.deselect", json!({})).unwrap();
    }
}

/// #11126: in a 32-bit float document Linear Dodge (Add) keeps a sum above white, as its layers
/// composite, also with Preserve Transparency; 8- and 16-bit documents still clip at white, and a
/// sum below white is the same at every depth.
#[test]
fn a_float_linear_dodge_fill_keeps_light_above_white() {
    let fill = |depth: u32, base: f32, add: f32, preserve: bool| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 1, "height": 1, "mode": "rgb", "depth": depth, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": [base, base, base, 1.0], "mode": "normal"})).unwrap();
        s.execute("edit.fill", json!({"color": [add, add, add, 1.0], "mode": "linearDodge", "preserveTransparency": preserve})).unwrap();
        px(&s, 0, 0)
    };
    for preserve in [false, true] {
        let hdr = fill(32, 0.75, 0.5, preserve);
        assert!((hdr[0] - 1.25).abs() < 1e-6 && hdr[3] == 1.0, "32-bit, preserve {preserve}: {hdr:?}");
        for depth in [8, 16] {
            let clipped = fill(depth, 0.75, 0.5, preserve);
            assert!((clipped[0] - 1.0).abs() < 1e-3, "{depth}-bit, preserve {preserve}: {clipped:?}");
        }
        for depth in [8, 16, 32] {
            let below = fill(depth, 0.25, 0.5, preserve);
            assert!((below[0] - 0.75).abs() < 3e-3, "{depth}-bit below white, preserve {preserve}: {below:?}");
        }
    }
}

/// #11126: a layer mask or an alpha channel of a 32-bit document holds coverage, so a Linear Dodge
/// fill there still stops at 1, while the layer's colour doesn't.
#[test]
fn a_float_linear_dodge_fill_on_a_mask_or_channel_stays_coverage() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1, "mode": "rgb", "depth": 32, "background": "transparent"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.new", json!({"name": "Alpha 1"})).unwrap();
    let gray = |v: f32| json!([v, v, v, 1.0]);
    for target in [json!("mask"), json!({"channel": 0})] {
        s.execute("edit.fill", json!({"color": gray(0.75), "mode": "normal", "target": target})).unwrap();
        s.execute("edit.fill", json!({"color": gray(0.5), "mode": "linearDodge", "target": target})).unwrap();
    }
    let st = s.active().unwrap();
    let mask = st.doc.layers.last().unwrap().mask.as_ref().unwrap().surface.read_region(Rect::new(0, 0, 1, 1))[0];
    let channel = st.doc.channels[0].surface.read_region(Rect::new(0, 0, 1, 1))[0];
    assert!(mask <= 1.0 && (mask - 1.0).abs() < 1e-3, "the mask stops at full coverage: {mask}");
    assert!(channel <= 1.0 && (channel - 1.0).abs() < 1e-3, "the alpha channel stops at full coverage: {channel}");
}

/// #11126 for every tool that paints with a blend mode: in a 32-bit document, 0.5 in Linear Dodge
/// over 0.75 on a layer keeps 1.25 (Paint Bucket, Gradient, Brush, Edit › Stroke, Fade), and the
/// same on a layer mask stops at 1 (masks hold coverage).
#[test]
fn every_float_paint_tool_keeps_light_above_white_on_colour_only() {
    let base = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "mode": "rgb", "depth": 32, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": [0.75, 0.75, 0.75, 1.0], "mode": "normal"})).unwrap();
        s
    };
    let half = "#808080";
    let tools: [(&str, serde_json::Value); 5] = [
        ("paint.bucket", json!({"x": 20, "y": 20, "color": half, "mode": "linearDodge", "tolerance": 0})),
        ("paint.gradient", json!({"from": [0, 20], "to": [40, 20], "colors": [half, half], "mode": "linearDodge", "dither": false})),
        ("paint.stroke", json!({"points": [[20, 20], [21, 20]], "color": half, "size": 12, "hardness": 100, "mode": "linearDodge"})),
        ("edit.stroke", json!({"width": 4, "color": half, "location": "inside", "mode": "linearDodge"})),
        ("edit.fade", json!({"opacity": 100, "mode": "linearDodge"})),
    ];
    for (cmd, params) in tools {
        let mut s = base();
        if cmd == "edit.stroke" {
            s.execute("select.all", json!({})).unwrap();
        }
        if cmd == "edit.fade" {
            // Fade blends the last edit (a 0.5 fill) over what it replaced (0.75).
            s.execute("edit.fill", json!({"color": [0.5, 0.5, 0.5, 1.0], "mode": "normal"})).unwrap();
        }
        s.execute(cmd, params.clone()).unwrap();
        let (x, y) = if cmd == "edit.stroke" { (1, 1) } else { (20, 20) };
        let v = px(&s, x, y)[0];
        assert!(v > 1.2, "{cmd}: light above white is kept on the layer: {v}");
    }
    // On a layer mask: coverage, so it stops at 1.
    for (cmd, params) in [
        ("paint.bucket", json!({"x": 20, "y": 20, "color": half, "mode": "linearDodge", "tolerance": 0, "target": "mask"})),
        ("paint.stroke", json!({"points": [[20, 20], [21, 20]], "color": half, "size": 12, "hardness": 100, "mode": "linearDodge", "target": "mask"})),
    ] {
        let mut s = base();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": [0.75, 0.75, 0.75, 1.0], "mode": "normal", "target": "mask"})).unwrap();
        s.execute(cmd, params).unwrap();
        let st = s.active().unwrap();
        let m = st.doc.layers.last().unwrap().mask.as_ref().unwrap().surface.read_region(Rect::new(20, 20, 21, 21))[0];
        assert!(m <= 1.0 && m > 0.99, "{cmd} on a mask stops at full coverage: {m}");
    }
}

/// #11126: a 32-bit fill blends as a layer of the same mode composites, also for the modes the
/// compositor has its own variants of.
#[test]
fn a_float_fill_blends_as_its_layers_composite() {
    for (mode, name) in [
        (BlendMode::ColorDodge, "colorDodge"),
        (BlendMode::ColorBurn, "colorBurn"),
        (BlendMode::VividLight, "vividLight"),
        (BlendMode::LinearDodge, "linearDodge"),
        (BlendMode::Color, "color"),
    ] {
        // The second pair tells the compositor's Color Dodge from the generic one: a backdrop of
        // 1e-6 stays black in the compositor's, and doesn't in the generic one.
        for (b, c) in [([0.6, 0.3, 1.4, 1.0], [0.5, 0.7, 0.2, 1.0]), ([1e-6, 0.3, 1.4, 1.0], [1.0, 0.7, 0.2, 1.0])] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 1, "height": 1, "mode": "rgb", "depth": 32, "background": "transparent"})).unwrap();
            s.execute("edit.fill", json!({"color": b, "mode": "normal"})).unwrap();
            s.execute("edit.fill", json!({"color": c, "mode": name})).unwrap();
            let got = px(&s, 0, 0);
            let want = photocraft_compose::psblend::with_hdr(true, || photocraft_compose::psblend::composite(mode, b, c, 1.0));
            assert!(close(got, want), "{mode:?} over {b:?}: fill {got:?}, layer {want:?}");
        }
    }
}

/// #11126: a 32-bit gradient keeps stop colours above white on a layer's colour, and Stroke Path's
/// brush keeps a Linear Dodge above white as the Brush does.
#[test]
fn float_gradient_stops_and_stroke_path_keep_light_above_white() {
    let base = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "mode": "rgb", "depth": 32, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": [0.5, 0.5, 0.5, 1.0], "mode": "normal"})).unwrap();
        s
    };
    let mut s = base();
    let bright = [2.0, 2.0, 2.0, 1.0];
    s.execute("paint.gradient", json!({"from": [0, 20], "to": [40, 20], "colors": [bright, bright], "mode": "linearDodge"})).unwrap();
    let v = px(&s, 20, 20)[0];
    assert!((v - 2.5).abs() < 0.01, "a stop of 2 over 0.5 in Linear Dodge: {v}");
    let mut s = base();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[10, 10], [30, 10], [30, 30], [10, 30]]}]}})).unwrap();
    // Stroke Path paints with the Brush tool's settings, its blend mode included.
    s.tools.brush.mode = BlendMode::LinearDodge;
    s.execute("path.stroke", json!({"tool": "pencil", "size": 4, "color": [0.75, 0.75, 0.75, 1.0], "opacity": 100})).unwrap();
    let v = px(&s, 20, 10)[0];
    assert!((v - 1.25).abs() < 0.01, "Stroke Path in Linear Dodge keeps light above white: {v}");
}
