use super::*;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "transparent"})).unwrap();
    s
}

fn surface(s: &Session) -> Surface {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    surface(s).rgba(x, y)
}

fn same_pixels(a: &Surface, b: &Surface, r: Rect) -> bool {
    (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| a.pixel(x, y) == b.pixel(x, y)))
}

#[test]
fn stroke_backwards_compatible() {
    let mut s = session(80, 40);
    let r = s.execute("paint.stroke", json!({"points": [[4, 20], [60, 20]], "size": 6, "color": "#0000ff"})).unwrap();
    assert_eq!(r["damage"].as_array().unwrap().len(), 4);
    assert_eq!(rgba(&s, 30, 20), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(rgba(&s, 30, 30)[3], 0.0);
    // [x, y, pressure] with the default pressure-size brush: light pressure → thinner line.
    s.execute("paint.stroke", json!({"points": [[4, 5, 0.2], [60, 5, 0.2]], "size": 10})).unwrap();
    assert!(rgba(&s, 30, 5)[3] > 0.9 && rgba(&s, 30, 8)[3] < 0.01);
}

#[test]
fn stroke_accepts_full_points_brush_and_preset() {
    let mut s = session(200, 100);
    // 7-tuples plus a brush object with tilt-driven angle on a flat tip.
    let brush = json!({"size": 30, "roundness": 0.2, "pressureSize": false, "spacing": 0.1,
        "shapeDynamics": {"enabled": true, "angle": {"control": "penTilt"}}});
    s.execute("paint.stroke", json!({"points": [[50, 50, 1, 0, -45, 0, 0], [52, 50, 1, 0, -45, 0, 16]], "brush": brush})).unwrap();
    // Tilt up = 90°: the flat tip stands vertically.
    assert!(rgba(&s, 50, 40)[3] > 0.9 && rgba(&s, 50, 60)[3] > 0.9 && rgba(&s, 60, 50)[3] < 0.01);
    // Object points.
    s.execute("paint.stroke", json!({"points": [{"x": 150, "y": 20}, {"x": 180, "y": 20, "pressure": 1.0}], "size": 4, "brush": {"pressureSize": false}})).unwrap();
    assert!(rgba(&s, 165, 20)[3] > 0.9);
    // Presets by name.
    s.execute("paint.stroke", json!({"points": [[20, 80], [120, 80]], "preset": "Hard Round"})).unwrap();
    assert!(rgba(&s, 70, 80)[3] > 0.99);
    assert!(s.execute("paint.stroke", json!({"points": [[1, 1]], "preset": "Nope"})).is_err());
    assert!(s.execute("paint.stroke", json!({"points": [[1, 1]], "brush": {"size": "big"}})).is_err());
}

#[test]
fn replay_is_deterministic() {
    let params = json!({"points": [[10, 50], [60, 30, 0.7], [120, 60, 0.9], [180, 40]], "preset": "Spatter", "color": "#ff8800"});
    let run = || {
        let mut s = session(200, 100);
        s.execute("paint.stroke", params.clone()).unwrap();
        s.execute("tools.setBrush", json!({"preset": "Confetti"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 80], [190, 80]]})).unwrap();
        s
    };
    let (a, b) = (run(), run());
    let r = Rect::new(0, 0, 200, 100);
    assert!(same_pixels(&surface(&a), &surface(&b), r));
    // Replaying the journal into a fresh session reproduces the pixels.
    let mut c = session(200, 100);
    for (id, p) in a.journal.iter().skip(1) {
        c.execute(id, p.clone()).unwrap();
    }
    assert!(same_pixels(&surface(&a), &surface(&c), r));
    // A different explicit seed changes the scatter.
    let mut d = session(200, 100);
    let mut p2 = params.clone();
    p2["seed"] = json!(12345);
    d.execute("paint.stroke", p2).unwrap();
    let mut e = session(200, 100);
    e.execute("paint.stroke", params).unwrap();
    assert!(!same_pixels(&surface(&d), &surface(&e), r));
}

#[test]
fn pencil_is_aliased_and_auto_erases() {
    let mut s = session(60, 30);
    s.execute("paint.pencil", json!({"points": [[5, 10.3], [50, 10.3]], "size": 5, "color": "#000000"})).unwrap();
    let surf = surface(&s);
    for y in 0..30 {
        for x in 0..60 {
            let a = surf.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0);
        }
    }
    s.execute("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    // Starting on a foreground pixel: paints background.
    s.execute("paint.pencil", json!({"points": [[20, 10], [20, 25]], "size": 3, "autoErase": true})).unwrap();
    assert_eq!(rgba(&s, 20, 22), [1.0, 1.0, 1.0, 1.0]);
    // Starting elsewhere: paints foreground.
    s.execute("paint.pencil", json!({"points": [[40, 25], [40, 28]], "size": 3, "autoErase": true})).unwrap();
    assert_eq!(rgba(&s, 40, 26), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn mixer_brush_reservoir_and_cleaning() {
    let mut s = session(200, 30);
    s.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    s.execute("paint.mixerBrush", json!({"points": [[5, 15], [195, 15]], "size": 8, "wet": 0, "load": 0, "mix": 0, "brush": {"pressureSize": false, "spacing": 0.25}})).unwrap();
    assert!(rgba(&s, 10, 15)[3] > 0.95 && rgba(&s, 190, 15)[3] < 0.5, "low load dries out");
    assert!(s.tools.mixer.pickup.is_none(), "cleaned after stroke");
    // Wet, not cleaned: the pickup persists into the session.
    s.execute("paint.mixerBrush", json!({"points": [[5, 5], [100, 5]], "size": 8, "wet": 100, "mix": 80, "cleanAfterStroke": false})).unwrap();
    assert!(s.tools.mixer.pickup.is_some());
    // Sample All Layers path runs.
    s.execute("paint.mixerBrush", json!({"points": [[5, 25], [50, 25]], "size": 6, "sampleAllLayers": true})).unwrap();
}

#[test]
fn color_replacement_modes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 60, "height": 40, "background": "#33993f"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    let before = rgba(&s, 30, 20);
    s.execute("paint.colorReplacement", json!({"points": [[10, 20], [50, 20]], "size": 20, "mode": "color", "tolerance": 20})).unwrap();
    let c = rgba(&s, 30, 20);
    let lum = |c: [f32; 4]| photocraft_color::blend::lum([c[0], c[1], c[2]]);
    assert!(c[0] > c[1], "{c:?}");
    assert!((lum(c) - lum(before)).abs() < 0.02);
    s.execute("paint.colorReplacement", json!({"points": [[10, 5], [50, 5]], "size": 6, "mode": "luminosity", "sampling": "once", "limits": "discontiguous"})).unwrap();
    assert!((lum(rgba(&s, 30, 5)) - 0.3).abs() < 0.02);
    assert!(s.execute("paint.colorReplacement", json!({"points": [[1, 1]], "mode": "bogus"})).is_err());
}

#[test]
fn presets_save_load_delete_round_trip() {
    let mut s = session(10, 10);
    let n0 = s.execute("brush.presets.list", json!({})).unwrap()["presets"].as_array().unwrap().len();
    assert!(n0 >= 12);
    s.execute("brush.presets.save", json!({"name": "Mine", "brush": {"size": 77, "scattering": {"enabled": true, "count": 3}}})).unwrap();
    let list = s.execute("brush.presets.list", json!({"full": true})).unwrap();
    let mine = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "Mine").unwrap().clone();
    assert_eq!(mine["brush"]["size"], 77.0);
    assert_eq!(mine["brush"]["scattering"]["count"], 3);
    // Round trip: select it, read it back.
    let b = s.execute("tools.setBrush", json!({"preset": "mine"})).unwrap();
    assert_eq!(b["size"], 77.0);
    let back: BrushSettings = serde_json::from_value(s.execute("brush.get", json!({})).unwrap()).unwrap();
    assert_eq!(back, s.tools.brush);
    // Overwrite, then delete.
    s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
    assert_eq!(s.tools.presets.len(), n0 + 1);
    s.execute("brush.presets.delete", json!({"name": "Mine"})).unwrap();
    assert_eq!(s.tools.presets.len(), n0);
    assert!(s.execute("brush.presets.delete", json!({"name": "Mine"})).is_err());
    assert!(s.execute("brush.presets.save", json!({})).is_err());
}

#[test]
fn set_brush_merges_fields() {
    let mut s = session(10, 10);
    s.execute("tools.setBrush", json!({"size": 55, "shapeDynamics": {"enabled": true, "size": {"jitter": 0.5}}})).unwrap();
    s.execute("tools.setBrush", json!({"shapeDynamics": {"angle": {"jitter": 0.25}}})).unwrap();
    let b = &s.tools.brush;
    assert_eq!(b.size, 55.0);
    assert!(b.shape_dynamics.enabled && b.shape_dynamics.size.jitter == 0.5 && b.shape_dynamics.angle.jitter == 0.25);
    s.execute("tools.setBrush", json!({"reset": true})).unwrap();
    assert_eq!(s.tools.brush, BrushSettings::default());
    // Protect texture carries the pattern across textured presets.
    s.execute("tools.setBrush", json!({"preset": "Chalk", "protectTexture": true})).unwrap();
    let chalk_pattern = s.tools.brush.texture.pattern.clone();
    s.execute("tools.setBrush", json!({"preset": "Canvas Texture"})).unwrap();
    assert_eq!(s.tools.brush.texture.pattern, chalk_pattern);
}

#[test]
fn define_brush_from_selection() {
    let mut s = session(40, 40);
    assert!(s.execute("brush.defineFromSelection", json!({"name": "Blob"})).is_err(), "needs a selection");
    // A black 6×4 block inside a larger selection.
    s.edit("setup", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 12, 16, 16), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    let r = s.execute("brush.defineFromSelection", json!({"name": "Blob"})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(6), Some(4)));
    assert_eq!(s.tools.brush.size, 6.0);
    assert!(matches!(&s.tools.brush.tip, TipShape::Sampled(t) if t.width == 6 && t.height == 4 && t.get(0, 0) == 1.0));
    assert!(s.tools.presets.iter().any(|p| p.name == "Blob"));
    // Paint with it.
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[30, 32]], "color": "#ff0000"})).unwrap();
    assert!(rgba(&s, 30, 32)[3] > 0.9);
}

#[test]
fn works_on_cmyk_and_16_bit() {
    for (mode, depth) in [("cmyk", 8), ("rgb", 16), ("grayscale", 32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 30, "mode": mode, "depth": depth})).unwrap();
        s.execute("paint.stroke", json!({"points": [[5, 15], [55, 15]], "preset": "Chalk", "color": "#000000"})).unwrap();
        s.execute("paint.pencil", json!({"points": [[5, 5], [55, 5]], "size": 2, "color": "#000000"})).unwrap();
        assert!(rgba(&s, 30, 5)[0] < 0.2, "{mode}/{depth}: {:?}", rgba(&s, 30, 5));
    }
}


#[test]
fn brush_blend_mode_multiply() {
    // Painting with a blend mode (options-bar "Mode"): blue × yellow = black (Multiply).
    let fill = |s: &mut Session| {
        s.edit("bg", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 40), &[1.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    };
    let mut s = session(40, 40);
    fill(&mut s);
    s.execute("paint.stroke", json!({"points": [[5, 20], [35, 20]], "size": 12, "color": "#0000ff", "mode": "multiply", "brush": {"hardness": 1.0}})).unwrap();
    let c = rgba(&s, 20, 20);
    assert!(c[0] < 0.1 && c[1] < 0.1 && c[2] < 0.1, "multiply blue×yellow ≈ black: {c:?}");
    // Normal mode paints opaque blue at the same spot.
    let mut s2 = session(40, 40);
    fill(&mut s2);
    s2.execute("paint.stroke", json!({"points": [[5, 20], [35, 20]], "size": 12, "color": "#0000ff", "brush": {"hardness": 1.0}})).unwrap();
    let n = rgba(&s2, 20, 20);
    assert!(n[2] > 0.9 && n[0] < 0.1, "normal paints blue: {n:?}");
}

#[test]
fn eraser_on_background_layer_paints_the_background_colour() {
    // Issue #15: erasing the Background layer (locked transparency) must paint the background
    // colour, not silently do nothing (opacity can't be removed there).
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20, "background": "white"})).unwrap();
    s.execute("tools.setColors", json!({"background": "#ff0000"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 10], [18, 10]], "size": 8, "erase": true, "brush": {"hardness": 1.0}})).unwrap();
    let st = s.active().unwrap();
    let c = st.doc.layers[0].surface().unwrap().rgba(10, 10);
    assert!(c[0] > 0.9 && c[1] < 0.1 && c[2] < 0.1 && c[3] > 0.99, "erased to opaque background red: {c:?}");
    // A corner the stroke didn't touch stays white.
    let w = st.doc.layers[0].surface().unwrap().rgba(10, 2);
    assert!(w[0] > 0.9 && w[1] > 0.9 && w[2] > 0.9, "untouched stays white: {w:?}");
}
