//! Tests that run paint commands, moved from the engine-core `pattern_cmds` tests: 8 test(s).

use photocraft_doc::{Pattern, Rect};
use photocraft_engine::Session;
use serde_json::json;

use photocraft_engine::pattern_cmds::*;

fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    s
}

fn pixel(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let b = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(x, y, x + 1, y + 1));
    b.px[0]
}

#[test]
fn define_rename_delete_and_pat_round_trip() {
    let mut s = session(8);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 3, "width": 5, "height": 4})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": "Red"})).unwrap();
    let id = r["pattern"].as_str().unwrap().to_string();
    let pat = resolve(&s, &id).unwrap();
    assert_eq!((pat.width, pat.height), (5, 4));
    assert!(!pat.surface.format().alpha, "opaque pattern drops alpha");
    // Same content → same id (replayable); not duplicated.
    let n = s.patterns.items.len();
    s.execute("edit.definePattern", json!({"name": "Red"})).unwrap();
    assert_eq!(s.patterns.items.len(), n);
    s.execute("pattern.rename", json!({"pattern": id, "name": "Crimson"})).unwrap();
    assert_eq!(resolve(&s, "Crimson").unwrap().id, id);
    let dir = std::env::temp_dir().join(format!("pc-pat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lib.pat");
    let path = path.to_str().unwrap();
    s.execute("pattern.export", json!({"path": path})).unwrap();
    s.execute("pattern.delete", json!({"pattern": "Crimson"})).unwrap();
    assert!(resolve(&s, "Crimson").is_none());
    assert!(s.execute("pattern.delete", json!({"pattern": "Crimson"})).is_err());
    let v = s.execute("pattern.import", json!({"path": path})).unwrap();
    assert!(v["imported"].as_array().unwrap().iter().any(|x| x == id.as_str()));
    assert_eq!(resolve(&s, &id).unwrap().surface.read_region(Rect::new(0, 0, 5, 4)), pat.surface.read_region(Rect::new(0, 0, 5, 4)));
    // The same file as bytes (a dropped file, the web build).
    let bytes = std::fs::read(path).unwrap();
    s.execute("pattern.delete", json!({"pattern": "Crimson"})).unwrap();
    let v = s.execute("pattern.import", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(v["count"], v["imported"].as_array().unwrap().len());
    assert_eq!(resolve(&s, "Crimson").unwrap().id, id);
    std::fs::remove_dir_all(dir).ok();
    assert!(s.execute("pattern.import", json!({"path": "/nonexistent.pat"})).is_err());
    for bad in [json!({}), json!({"data": "not base64!"}), json!({"data": "aGVsbG8="})] {
        assert!(s.execute("pattern.import", bad.clone()).is_err(), "{bad}");
    }
}

/// Review of #8902: what Define Pattern accepts, the compositor paints. Areas over
/// 4096 x 4096, past the compositor's whole-copy cap, define and fill at 8 and 16 bits (held
/// at the pattern's own depth); a depth that has no smaller exact form is refused up front
/// with its limit, instead of defining a pattern that paints nothing.
#[test]
fn defined_patterns_over_4096_squared_paint_or_are_refused() {
    for depth in [8, 16] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4100, "height": 4100, "depth": depth})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        s.execute("edit.definePattern", json!({"name": "Big"})).unwrap();
        assert!(s.patterns.items.iter().any(|p| p.name == "Big" && p.is_paintable()), "depth {depth}");
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": "Big"})).unwrap();
        let [r, g, b, a] = pixel(&s, 5, 5);
        assert!(a == 1.0 && (r - 0.2).abs() < 0.01 && (g - 0.4).abs() < 0.01 && (b - 0.6).abs() < 0.01, "depth {depth}: {:?}", [r, g, b, a]);
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4100, "height": 4100, "depth": 32})).unwrap();
    let e = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
    assert!(e.contains("32-bit") && e.contains("16777216"), "{e}");
    assert!(s.patterns.items.iter().all(|p| p.name != "Pattern"), "nothing was defined");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 5600, "depth": 16})).unwrap();
    let e = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
    assert!(e.contains("16-bit") && e.contains("33554432"), "{e}");
}

/// Review of #8902: a pattern too large to paint is named by every command that would use it
/// and by pattern.list, instead of rendering as transparency with no message.
#[test]
fn a_pattern_that_would_paint_nothing_is_reported() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8})).unwrap();
    // A 32-bit 5000 x 5000 pattern (declared only: 400 MB as a working copy, past the cap).
    let surface = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA32F);
    s.patterns.items.push(Pattern { id: "huge".into(), name: "Huge".into(), width: 5000, height: 5000, surface });
    let list = s.execute("pattern.list", json!({})).unwrap();
    let huge = list.as_array().unwrap().iter().find(|p| p["name"] == "Huge").unwrap().clone();
    assert_eq!(huge["paintable"], json!(false));
    assert!(huge["paintProblem"].as_str().unwrap().contains("32 bits"), "{huge}");
    for (id, params) in [
        ("layer.newFillLayer.pattern", json!({"pattern": "Huge"})),
        ("layer.layerStyle.patternOverlay", json!({"pattern": "Huge"})),
        ("edit.fill", json!({"contents": "pattern", "pattern": "Huge"})),
        ("brush.texturePattern", json!({"pattern": "Huge"})),
    ] {
        let e = s.execute(id, params).expect_err(id).to_string();
        assert!(e.contains("Huge") && e.contains("would paint nothing"), "{id}: {e}");
    }
    let e = s.execute("layer.newFillLayer.pattern", json!({"pattern": "Huge"})).unwrap_err().to_string();
    assert!(e.contains("would paint nothing") && e.contains("5000 × 5000"), "{e}");
    assert!(list.as_array().unwrap().iter().any(|p| p["name"] != "Huge" && p["paintable"] == json!(true)), "built-in patterns paint");
}

#[test]
fn define_pattern_accepts_edges_over_4000_up_to_8000() {
    for (w, ok) in [(4100, true), (8000, true), (8001, false)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": 2, "depth": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        let r = s.execute("edit.definePattern", json!({}));
        assert_eq!(r.is_ok(), ok, "width {w}: {r:?}");
    }
}

#[test]
fn fill_with_pattern_respects_selection() {
    for depth in [8u64, 16, 32] {
        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"contents": "pattern", "pattern": "Diagonal Lines"})).unwrap();
        let st = s.active().unwrap();
        let surf = st.doc.layers[0].surface().unwrap();
        assert!(surf.pixel(0, 0)[0] < 0.3, "{depth}: line pixel");
        assert!(surf.pixel(4, 0)[0] > 0.9);
        assert!(surf.pixel(20, 20)[0] > 0.99, "outside the selection untouched");
    }
}

#[test]
fn pattern_overlay_composes_with_an_outside_stroke() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "Diagonal Lines"})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "position": "outside", "color": "#00ff00", "add": true})).unwrap();
    let out = pixel(&s, 8, 15);
    assert!(out[1] > 0.9 && out[0] < 0.1, "stroke outside: {out:?}");
}

#[test]
fn pattern_overlay_renders_and_brush_texture_uses_library() {
    let mut s = session(16);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "Checkerboard", "opacity": 100})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.patterns.len(), 1);
    let p = pixel(&s, 1, 1);
    assert!((p[0] - 0.8).abs() < 0.01 && (p[2] - 0.8).abs() < 0.01, "{p:?}");
    assert!(s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "missing"})).is_err());
    s.execute("layer.layerStyle.stroke", json!({"size": 4, "color": "#ffffff", "add": true})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().effects.items.len(), 2);
    s.execute("brush.texturePattern", json!({"pattern": "Dots", "scale": 50})).unwrap();
    assert!(s.tools.brush.texture.enabled);
    assert!(matches!(s.tools.brush.texture.pattern, photocraft_paint::Pattern::Tile(_)));
}

#[test]
fn pattern_fill_and_bucket_reject_non_finite_angle_and_phase() {
    // 1e39 overflowed f32 to infinity: a NaN render.
    let mut s = session(8);
    let n = s.active().unwrap().doc.layers.len();
    for cmd in ["edit.fill", "paint.bucket"] {
        for bad in [json!({"angle": 1e39}), json!({"angle": "x"}), json!({"phase": [1e39, 0]}), json!({"phase": [0, -1e300]})] {
            let mut p = json!({"pattern": "Checkerboard", "contents": "pattern", "x": 1, "y": 1});
            p.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains("`angle`") || e.contains("`phase`"), "{cmd} {bad}: {e}");
        }
    }
    assert_eq!(s.active().unwrap().doc.layers.len(), n);
}
