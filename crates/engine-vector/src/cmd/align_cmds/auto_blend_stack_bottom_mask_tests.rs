//! #14212: Stack Images leaves the bottom layer's mask white everywhere; only the layers above
//! are masked to their sharp parts.

use super::*;

const W: i32 = 400;
const H: i32 = 300;

/// The detail both layers share: a 2 px checker, the "sharp" picture.
fn detail(x: i32, y: i32) -> f32 {
    if (x / 2 + y / 2) % 2 == 0 { 0.2 } else { 0.8 }
}

/// A layer that is sharp over x `sharp` and flat grey (the checker's mean, "blurred") elsewhere.
fn focus_layer(s: &mut Session, name: &str, sharp: std::ops::Range<i32>) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.edit(name, |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..H {
            for x in 0..W {
                let v = if sharp.contains(&x) { detail(x, y) } else { 0.5 };
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    s.active().unwrap().active_layer.unwrap()
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
}

/// The issue's focus stack: FocusL (bottom) sharp on the left half, FocusR (top) on the right.
fn focus_pair() -> (Session, LayerId, LayerId) {
    focus_pair_at(8)
}

fn focus_pair_at(depth: u32) -> (Session, LayerId, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "background": "transparent"})).unwrap();
    let l = focus_layer(&mut s, "FocusL", 0..W / 2);
    let r = focus_layer(&mut s, "FocusR", W / 2..W);
    select(&mut s, &[l, r]);
    (s, l, r)
}

fn pixels() -> impl Iterator<Item = (i32, i32)> {
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y)))
}

#[test]
fn a_stack_shows_the_bottom_layer_everywhere() {
    let (mut s, l, r) = focus_pair();
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    let ml = doc.layer(l).unwrap().mask.as_ref().expect("FocusL gets a mask");
    assert!(pixels().all(|(x, y)| ml.value(x, y) == 1.0), "FocusL's mask is white everywhere");
    let mr = doc.layer(r).unwrap().mask.as_ref().expect("FocusR gets a mask");
    assert!(mr.value(3 * W / 4, H / 2) > 0.5 && mr.value(W / 4, H / 2) < 0.5, "FocusR shows only its sharp half");
    // The sharp half of each layer shows, away from the seam.
    let img = photocraft_compose::render(doc, doc.bounds());
    let off = pixels().filter(|&(x, _)| !(W / 2 - 20..W / 2 + 20).contains(&x)).find(|&(x, y)| {
        let px = img.px[(y * W + x) as usize];
        (px[0] - detail(x, y)).abs() > 0.02 || px[3] < 0.98
    });
    assert_eq!(off, None, "the composite keeps the sharp half of each layer");
}

/// A top pixel at alpha 0.75 still wins the mask where it is sharp. Under the old mask the
/// bottom stays hidden there and the pixel stays partly transparent; revealing the bottom would
/// make it opaque and mix in the bottom's colour, so the old masks are kept.
#[test]
fn a_semi_transparent_winning_pixel_keeps_the_bottom_masked() {
    let (mut s, l, r) = focus_pair();
    let (x, y) = (3 * W / 4, H / 2);
    s.edit("soften", |doc, _| {
        let v = detail(x, y);
        doc.layer_mut(r).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 0.75]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(r).unwrap().mask.as_ref().unwrap().value(x, y) > 0.5, "the soft pixel wins the mask");
    assert!(doc.layer(l).unwrap().mask.as_ref().unwrap().value(x, y) < 0.5, "FocusL stays hidden under it");
    let img = photocraft_compose::render(doc, doc.bounds());
    let a = img.px[(y * W + x) as usize][3];
    assert!(a < 0.9, "the composite keeps the pixel partly transparent, alpha {a}");
}

/// In a 32-bit document a top pixel at alpha 0.999 is not opaque, though a mask would round that
/// weight to full. Revealing the bottom under it would make it opaque, so the old masks are kept.
#[test]
fn a_nearly_opaque_float_pixel_keeps_the_bottom_masked() {
    let (mut s, l, r) = focus_pair_at(32);
    let (x, y) = (3 * W / 4, H / 2);
    s.edit("soften", |doc, _| {
        let v = detail(x, y);
        doc.layer_mut(r).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 0.999]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(l).unwrap().mask.as_ref().unwrap().value(x, y) < 0.5, "FocusL stays hidden under it");
    let img = photocraft_compose::render(doc, doc.bounds());
    let a = img.px[(y * W + x) as usize][3];
    assert!(a < 0.9995, "the composite keeps the pixel's own alpha, {a}");
}

/// Prepares one case on the focus pair (FocusL, FocusR) and returns its Auto-Blend params.
type Setup = fn(&mut Session, &[LayerId]) -> Value;

/// Panorama, and the Stack Images cases the issue did not measure, keep complementary masks:
/// the bottom layer stays hidden where the layer above takes over.
#[test]
fn other_runs_keep_the_bottom_layer_masked() {
    let cases: [(&str, Setup); 7] = [
        ("panorama", |s, ids| {
            // A panorama pair: FocusL over x 0..240, FocusR over x 160..400.
            let (l, r) = (ids[0], ids[1]);
            s.edit("trim", |doc, _| {
                doc.layer_mut(l).unwrap().surface_mut().unwrap().fill_rect(Rect::new(240, 0, W, H), &[0.0; 4]);
                doc.layer_mut(r).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 160, H), &[0.0; 4]);
                Ok(())
            })
            .unwrap();
            json!({"method": "panorama", "seamlessTones": false})
        }),
        ("stack with Seamless Tones", |_, _| json!({"method": "stack", "seamlessTones": true})),
        ("stack of three layers", |s, ids| {
            let flat = focus_layer(s, "Flat", 0..0);
            select(s, &[ids[0], ids[1], flat]);
            json!({"method": "stack", "seamlessTones": false})
        }),
        ("stack under a clipped top", |s, ids| {
            // A clipped top shows only where the bottom does, so revealing the bottom shows more of it.
            let top = ids[1];
            s.edit("clip", |doc, _| {
                doc.layer_mut(top).unwrap().clipped = true;
                Ok(())
            })
            .unwrap();
            json!({"method": "stack", "seamlessTones": false})
        }),
        ("stack with the top in a half-opaque group", |s, ids| {
            // The group thins the top's coverage, so the bottom shows through even where the
            // top is opaque and sharp; revealing all of the bottom would show more of it.
            s.execute("layer.select", json!({"layer": ids[1].0})).unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
            let group = s.active().unwrap().active_layer.unwrap();
            s.edit("fade", |doc, _| {
                doc.layer_mut(group).unwrap().opacity = 0.5;
                Ok(())
            })
            .unwrap();
            select(s, ids);
            json!({"method": "stack", "seamlessTones": false})
        }),
        ("stack under a top that Blend If hides", |s, ids| {
            // This Layer's white point at 0 hides every top pixel, so the complementary masks
            // leave its winning half empty; revealing the bottom would fill it.
            let top = ids[1];
            s.edit("blend if", |doc, _| {
                let hide = photocraft_doc::BlendRange { black: [0, 0], white: [0, 0] };
                doc.layer_mut(top).unwrap().blend_if.set(0, [hide, photocraft_doc::BlendRange::FULL]);
                Ok(())
            })
            .unwrap();
            json!({"method": "stack", "seamlessTones": false})
        }),
        ("stack over an existing mask", |s, ids| {
            let top = ids[1];
            s.edit("mask", |doc, _| {
                doc.layer_mut(top).unwrap().mask = Some(LayerMask::reveal_all());
                Ok(())
            })
            .unwrap();
            json!({"method": "stack", "seamlessTones": false})
        }),
    ];
    for (name, setup) in cases {
        let (mut s, l, r) = focus_pair();
        let params = setup(&mut s, &[l, r]);
        s.execute("edit.autoBlendLayers", params).unwrap();
        let ml = s.active().unwrap().doc.layer(l).unwrap().mask.clone().expect("FocusL gets a mask");
        assert!(pixels().any(|(x, y)| ml.value(x, y) < 0.5), "{name}: FocusL is hidden where another layer takes over");
    }
}

/// The middle of the canvas, where the patch pairs keep their pixels.
fn patch() -> Rect {
    Rect::new(100, 50, 300, 250)
}

/// Keeps `id`'s pixels inside `patch()` only; everywhere else the surface holds `default`.
fn keep_to_patch(s: &mut Session, id: LayerId, default: [f32; 4]) {
    s.edit("patch", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let mut kept = Surface::with_default(surf.format(), &default[..surf.format().channels()]);
        kept.write_region(patch(), &surf.read_region(patch()));
        kept.prune();
        *surf = kept;
        Ok(())
    })
    .unwrap();
}

/// A bottom layer whose default pixel is opaque white (as an imported layer can have) shows that
/// white past its changed pixels. The old mask hides it outside the blended area, so revealing
/// the bottom would paint the rest of the canvas white; the old masks are kept.
#[test]
fn an_opaque_default_pixel_keeps_the_bottom_masked() {
    let (mut s, l, r) = focus_pair();
    keep_to_patch(&mut s, l, [1.0; 4]);
    keep_to_patch(&mut s, r, [0.0; 4]);
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    let outside = (10, 10);
    assert!(doc.layer(l).unwrap().mask.as_ref().unwrap().value(outside.0, outside.1) < 0.5, "FocusL stays hidden past the patch");
    let img = photocraft_compose::render(doc, doc.bounds());
    let a = img.px[(outside.1 * W + outside.0) as usize][3];
    assert!(a < 0.02, "the canvas past the patch stays empty, alpha {a}");
}

/// A bottom layer with a deep knockout and Transparency Shapes Layer off knocks out wherever its
/// mask shows, not only under its pixels. Revealing it would clear the layer beneath past the
/// patch, where the old mask kept the knockout away; the old masks are kept.
#[test]
fn a_shapeless_knockout_keeps_the_bottom_masked() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Under"})).unwrap();
    s.edit("red", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, W, H), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let l = focus_layer(&mut s, "FocusL", 0..W / 2);
    let r = focus_layer(&mut s, "FocusR", W / 2..W);
    keep_to_patch(&mut s, l, [0.0; 4]);
    keep_to_patch(&mut s, r, [0.0; 4]);
    s.edit("knockout", |doc, _| {
        let advanced = &mut doc.layer_mut(l).unwrap().advanced;
        advanced.knockout = photocraft_doc::Knockout::Deep;
        advanced.transparency_shapes = false;
        Ok(())
    })
    .unwrap();
    select(&mut s, &[l, r]);
    s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    let outside = (10, 10);
    assert!(doc.layer(l).unwrap().mask.as_ref().unwrap().value(outside.0, outside.1) < 0.5, "FocusL stays hidden past the patch");
    let img = photocraft_compose::render(doc, doc.bounds());
    let px = img.px[(outside.1 * W + outside.0) as usize];
    assert!(px[3] > 0.98 && px[0] > 0.98, "the red layer beneath shows past the patch: {px:?}");
}
