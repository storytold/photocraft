//! Tests that run paint commands, moved from the engine-core `new_layer` tests: 1 test(s).

use photocraft_doc::BlendMode;

use photocraft_geom::Rect;

use serde_json::json;

use photocraft_engine::new_layer::neutral_color;

use photocraft_engine::Session;

/// A document with a busy Background: every pixel a different colour.
fn busy(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "mode": mode, "depth": depth})).unwrap();
    s.edit("busy", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        for y in 0..32 {
            for x in 0..32 {
                let rgba = [x as f32 / 31.0, y as f32 / 31.0, ((x * 7 + y * 3) % 32) as f32 / 31.0, 1.0];
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &photocraft_raster::from_rgba(&fmt, rgba));
            }
        }
        Ok(())
    })
    .unwrap();
    s
}

fn max_diff(a: &photocraft_compose::Buffer, b: &photocraft_compose::Buffer) -> f32 {
    a.px.iter().zip(&b.px).flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs())).fold(0.0, f32::max)
}

/// Fill with neutral color leaves the composite unchanged in every mode that offers it, at
/// every depth; the wrong neutral (gray for Multiply) does change it.
#[test]
fn neutral_fills_leave_the_image_unchanged() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("gray", 16)] {
        let s0 = busy(mode, depth);
        let doc0 = s0.active().unwrap().doc.clone();
        let shown = photocraft_compose::render(&doc0, doc0.bounds());
        for m in BlendMode::LAYER_MODES.into_iter().filter(|m| neutral_color(*m).is_some()) {
            let mut s = busy(mode, depth);
            s.execute("layer.new.layer", json!({"mode": m.label(), "fillNeutral": true})).unwrap();
            let doc = &s.active().unwrap().doc;
            let top = doc.layers.last().unwrap();
            assert_eq!(top.blend, m);
            assert!(top.surface().unwrap().tile_count() > 0, "{m:?} filled");
            let d = max_diff(&photocraft_compose::render(doc, doc.bounds()), &shown);
            // 8-bit stores 50% gray as 128 (0.502), which can move a pixel by one level.
            let tolerance = if depth == 8 { 1.0 / 255.0 + 1e-6 } else { 1e-3 };
            assert!(d <= tolerance, "{mode}/{depth}: {m:?} changed the image by {d}");
        }
        let mut s = busy(mode, depth);
        s.execute("layer.new.layer", json!({"mode": "Multiply"})).unwrap();
        s.execute("edit.fill", json!({"contents": "gray"})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert!(max_diff(&photocraft_compose::render(doc, doc.bounds()), &shown) > 0.1, "{mode}/{depth}: the control changes it");
    }
    for m in [BlendMode::Normal, BlendMode::Dissolve, BlendMode::HardMix, BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity] {
        assert!(neutral_color(m).is_none(), "{m:?}");
    }
}
