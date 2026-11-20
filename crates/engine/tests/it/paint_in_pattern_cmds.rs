//! Tests that run painting commands, moved from the engine-core `pattern_cmds` tests: 10 test(s).

use photocraft_color::PixelFormat;
use photocraft_doc::{Pattern, Rect};
use photocraft_engine::Session;
use photocraft_engine::preset_store::PresetStoreExt;
use photocraft_raster::Surface;
use serde_json::json;

use photocraft_engine::pattern_cmds::*;

use photocraft_color::SampleType;

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

/// Review of the 8000 px cap: compositing a band of rows at a time gives the pixels and the
/// opacity of one pass, at every depth, whatever the band height.
#[test]
fn banded_compositing_matches_a_single_pass() {
    for depth in [8u64, 16, 32] {
        for transparent in [false, true] {
            let mut s = Session::new();
            let background = if transparent { "transparent" } else { "white" };
            s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": background})).unwrap();
            s.execute("select.rect", json!({"x": 3, "y": 4, "width": 20, "height": 13})).unwrap();
            s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
            let doc = &s.active().unwrap().doc;
            let area = Rect::new(2, 1, 38, 29);
            let all = Rect::new(0, 0, 36, 28);
            let (whole, whole_opaque) = composite_surface_in_bands(doc, area, 28);
            for band_rows in [1, 5, 27] {
                let (banded, opaque) = composite_surface_in_bands(doc, area, band_rows);
                assert_eq!(opaque, whole_opaque, "depth {depth}, transparent {transparent}, bands of {band_rows}");
                assert_eq!(banded.read_region(all), whole.read_region(all), "depth {depth}, transparent {transparent}, bands of {band_rows}");
            }
            assert_eq!(whole_opaque, !transparent);
        }
    }
}

/// Review of the banded composite: opacity is the stored samples', so an alpha an 8-bit
/// document stores as 255 is opaque and a float document's 0.999 is not.
#[test]
fn alpha_that_stores_as_opaque_counts_as_opaque() {
    for (depth, opaque) in [(8u64, true), (32, false)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.999})).unwrap();
        let (_, all_opaque) = composite_surface_in_bands(&s.active().unwrap().doc, Rect::new(0, 0, 8, 8), 3);
        assert_eq!(all_opaque, opaque, "depth {depth}");
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

/// A pattern at the area limit is paintable and renders; production permits 8000×8000 at 8 bits.
#[test]
fn a_pattern_at_the_area_limit_is_defined_and_paints() {
    let mut s = Session::new();
    assert_eq!(MAX_DEFINE_EDGE, 8000);
    assert!(photocraft_doc::pattern::max_paint_pixels(SampleType::U8) >= 8000 * 8000);
    assert_eq!(photocraft_doc::pattern::working_bytes_for(8000 * 8000, SampleType::U8), 8000 * 8000 * 4);
    let (side, max_px) = (80, 80 * 80);
    s.execute("file.new", json!({"width": side, "height": side, "depth": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    assert!(define_with_max_pixels(&mut s, &json!({"name": "Too large"}), max_px - 1).is_err());
    define_with_max_pixels(&mut s, &json!({"name": "Largest"}), max_px).unwrap();
    let p = s.patterns.items.iter().find(|p| p.name == "Largest").unwrap();
    assert_eq!((p.width, p.height), (side, side));
    // A small pattern uses the expanded float representation, unlike the largest 8-bit one.
    assert!(p.is_paintable() && p.working_bytes() == max_px * 16);
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8})).unwrap();
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Largest"})).unwrap();
    let [r, g, b, a] = pixel(&s, 5, 5);
    assert!(a == 1.0 && (r - 0.2).abs() < 0.01 && (g - 0.4).abs() < 0.01 && (b - 0.6).abs() < 0.01, "{:?}", [r, g, b, a]);
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
