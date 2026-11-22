use super::*;

/// A 4×4 opaque black and white checker on the active layer.
fn checker(s: &mut Session) {
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(10, 10, 14, 14);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = if (x + y) % 2 == 0 { 0.0 } else { 1.0 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
}

/// Scales a checker smart object up 4× with `interpolation`, then applies each of `then` (more
/// `edit.transform` parameters for the same layer); returns the rendered samples of its opaque
/// pixels and the stored interpolation.
fn scaled_samples(interpolation: &str, then: &[Value]) -> (Vec<f32>, photocraft_doc::PlacementInterpolation) {
    let mut s = session(8);
    checker(&mut s);
    let id = convert(&mut s);
    s.execute("edit.transform", json!({"layer": id, "matrix": [4, 0, 0, 4, -30, -30], "interpolation": interpolation})).unwrap();
    for p in then {
        let mut p = p.clone();
        p["layer"] = json!(id);
        s.execute("edit.transform", p).unwrap();
    }
    let sm = active_smart(&s);
    let cache = sm.cache.as_ref().unwrap();
    let b = cache.content_bounds();
    let mut samples = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let p = cache.pixel(x, y);
            if p[3] >= 0.999 {
                samples.push(p[0]);
            }
        }
    }
    (samples, sm.interpolation)
}

/// Free Transform with Nearest Neighbor keeps a smart object's pixels hard-edged; the same
/// transform with Bicubic blends them, so the check tells the two apart.
#[test]
fn smart_object_transform_honours_nearest_interpolation() {
    let hard = |samples: &[f32], what: &str| {
        assert!(samples.iter().any(|&v| v < 0.01) && samples.iter().any(|&v| v > 0.99), "{what}: both source colours survive");
        assert!(samples.iter().all(|&v| v < 0.01 || v > 0.99), "{what}: no in-between values");
    };
    let (nearest, stored) = scaled_samples("nearest", &[]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Nearest);
    hard(&nearest, "nearest");

    // A later whole-pixel move and a flip, each with the default Bicubic, resample nothing: the
    // re-render keeps Nearest.
    let (moved, stored) = scaled_samples("nearest", &[json!({"dx": 3}), json!({"matrix": [-1, 0, 0, 1, 60, 0]})]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Nearest);
    hard(&moved, "moved and flipped");

    let (bicubic, stored) = scaled_samples("bicubic", &[]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Bicubic);
    assert!(bicubic.iter().any(|&v| (0.05..0.95).contains(&v)), "bicubic blends the checker's edges");
}
