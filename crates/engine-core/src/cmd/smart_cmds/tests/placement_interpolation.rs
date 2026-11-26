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
    (opaque_samples(sm.cache.as_ref().unwrap()), sm.interpolation)
}

/// The first channel of each opaque pixel of `surface`.
fn opaque_samples(surface: &Surface) -> Vec<f32> {
    let b = surface.content_bounds();
    let mut samples = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let p = surface.pixel(x, y);
            if p[3] >= 0.999 {
                samples.push(p[0]);
            }
        }
    }
    samples
}

/// Only the checker's two source colours, both of them.
fn assert_hard_edged(samples: &[f32], what: &str) {
    assert!(samples.iter().any(|&v| v < 0.01) && samples.iter().any(|&v| v > 0.99), "{what}: both source colours survive");
    assert!(samples.iter().all(|&v| v < 0.01 || v > 0.99), "{what}: no in-between values");
}

/// Free Transform with Nearest Neighbor keeps a smart object's pixels hard-edged; the same
/// transform with Bicubic blends them, so the check tells the two apart.
#[test]
fn smart_object_transform_honours_nearest_interpolation() {
    let (nearest, stored) = scaled_samples("nearest", &[]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Nearest);
    assert_hard_edged(&nearest, "nearest");

    // A later whole-pixel move and a flip, each with the default Bicubic, resample nothing: the
    // re-render keeps Nearest.
    let (moved, stored) = scaled_samples("nearest", &[json!({"dx": 3}), json!({"matrix": [-1, 0, 0, 1, 60, 0]})]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Nearest);
    assert_hard_edged(&moved, "moved and flipped");

    let (bicubic, stored) = scaled_samples("bicubic", &[]);
    assert_eq!(stored, photocraft_doc::PlacementInterpolation::Bicubic);
    assert!(bicubic.iter().any(|&v| (0.05..0.95).contains(&v)), "bicubic blends the checker's edges");
}

/// A Warp Apply re-renders from the source with the interpolation it commits: a checker enlarged
/// with Bicubic and then warped with Nearest Neighbor is hard-edged at once, and an explicit
/// refresh changes nothing.
#[test]
fn nearest_warp_of_a_bicubic_enlargement_renders_from_the_source_at_once() {
    let mut s = session(8);
    checker(&mut s);
    let id = convert(&mut s);
    s.execute("edit.transform", json!({"layer": id, "matrix": [4, 0, 0, 4, -30, -30], "interpolation": "bicubic"})).unwrap();
    let enlarged = opaque_samples(active_smart(&s).cache.as_ref().unwrap());
    assert!(enlarged.iter().any(|&v| (0.05..0.95).contains(&v)), "bicubic blends the checker's edges");

    s.execute("edit.transform.warp", json!({"layer": id, "style": "arc", "bend": 30, "interpolation": "nearest"})).unwrap();
    let sm = active_smart(&s);
    assert!(sm.warp.is_some(), "the warp is stored");
    assert_eq!(sm.interpolation, photocraft_doc::PlacementInterpolation::Nearest);
    let applied = sm.cache.unwrap();
    assert_hard_edged(&opaque_samples(&applied), "warp apply");

    assert!(s.edit("refresh", |doc, _| refresh(doc, LayerId(id))).unwrap(), "the source re-renders");
    let refreshed = active_smart(&s).cache.unwrap();
    let area = applied.content_bounds();
    assert_eq!(area, refreshed.content_bounds());
    assert!(applied.read_region(area) == refreshed.read_region(area), "a refresh draws what the apply left");
}
