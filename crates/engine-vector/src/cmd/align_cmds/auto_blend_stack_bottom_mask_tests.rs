//! #14212: Stack Images leaves the bottom layer's mask white everywhere; only the layers above
//! are masked to their sharp parts. #16725: whatever the two layers hold.

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

/// Prepares one case on the focus pair (FocusL, FocusR) and returns its Auto-Blend params.
type Setup = fn(&mut Session, &[LayerId]) -> Value;

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

/// Makes FocusR's pixel in the middle of its sharp half partly transparent.
fn soften_top(s: &mut Session, top: LayerId, alpha: f32) {
    let (x, y) = (3 * W / 4, H / 2);
    s.edit("soften", |doc, _| {
        let v = detail(x, y);
        doc.layer_mut(top).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, alpha]);
        Ok(())
    })
    .unwrap();
}

/// The measured run's params: Stack Images, Seamless Tones off.
fn stack() -> Value {
    json!({"method": "stack", "seamlessTones": false})
}

/// A two-layer stack with Seamless Tones off and no masks before reveals the bottom layer
/// everywhere, as measured, even where the layers would make the composite differ from the
/// complementary masks' (#16725).
#[test]
fn every_two_layer_stack_shows_the_bottom_layer_everywhere() {
    let cases: [(&str, u32, Setup); 8] = [
        ("a semi-transparent winning top pixel", 8, |s, ids| {
            soften_top(s, ids[1], 0.75);
            stack()
        }),
        ("a nearly opaque float top pixel", 32, |s, ids| {
            soften_top(s, ids[1], 0.999);
            stack()
        }),
        ("a half-opaque Multiply top", 8, |s, ids| {
            let top = ids[1];
            s.edit("style", |doc, _| {
                let l = doc.layer_mut(top).unwrap();
                l.blend = photocraft_color::BlendMode::Multiply;
                l.opacity = 0.5;
                Ok(())
            })
            .unwrap();
            stack()
        }),
        ("a clipped top", 8, |s, ids| {
            let top = ids[1];
            s.edit("clip", |doc, _| {
                doc.layer_mut(top).unwrap().clipped = true;
                Ok(())
            })
            .unwrap();
            stack()
        }),
        ("the top in a half-opaque group", 8, |s, ids| {
            s.execute("layer.select", json!({"layer": ids[1].0})).unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
            let group = s.active().unwrap().active_layer.unwrap();
            s.edit("fade", |doc, _| {
                doc.layer_mut(group).unwrap().opacity = 0.5;
                Ok(())
            })
            .unwrap();
            select(s, ids);
            stack()
        }),
        ("a top that Blend If hides", 8, |s, ids| {
            let top = ids[1];
            s.edit("blend if", |doc, _| {
                let hide = photocraft_doc::BlendRange { black: [0, 0], white: [0, 0] };
                doc.layer_mut(top).unwrap().blend_if.set(0, [hide, photocraft_doc::BlendRange::FULL]);
                Ok(())
            })
            .unwrap();
            stack()
        }),
        ("a bottom whose opaque default pixel lies past the blended area", 8, |s, ids| {
            keep_to_patch(s, ids[0], [1.0; 4]);
            keep_to_patch(s, ids[1], [0.0; 4]);
            stack()
        }),
        ("a bottom with a shapeless deep knockout", 8, |s, ids| {
            keep_to_patch(s, ids[0], [0.0; 4]);
            keep_to_patch(s, ids[1], [0.0; 4]);
            let bottom = ids[0];
            s.edit("knockout", |doc, _| {
                let advanced = &mut doc.layer_mut(bottom).unwrap().advanced;
                advanced.knockout = photocraft_doc::Knockout::Deep;
                advanced.transparency_shapes = false;
                Ok(())
            })
            .unwrap();
            stack()
        }),
    ];
    for (name, depth, setup) in cases {
        let (mut s, l, r) = focus_pair_at(depth);
        let params = setup(&mut s, &[l, r]);
        s.execute("edit.autoBlendLayers", params).unwrap();
        let doc = &s.active().unwrap().doc;
        let ml = doc.layer(l).unwrap().mask.as_ref().expect("FocusL gets a mask");
        assert!(pixels().all(|(x, y)| ml.value(x, y) == 1.0), "{name}: FocusL's mask is white everywhere");
        let mr = doc.layer(r).unwrap().mask.as_ref().expect("FocusR gets a mask");
        assert!(pixels().any(|(x, y)| mr.value(x, y) < 0.5), "{name}: FocusR is still masked to its sharp parts");
    }
}

/// Panorama, and the Stack Images runs outside the measured case, keep complementary masks:
/// the bottom layer stays hidden where the layer above takes over.
#[test]
fn other_runs_keep_the_bottom_layer_masked() {
    let cases: [(&str, Setup); 4] = [
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
            stack()
        }),
        ("stack over an existing mask", |s, ids| {
            let top = ids[1];
            s.edit("mask", |doc, _| {
                doc.layer_mut(top).unwrap().mask = Some(LayerMask::reveal_all());
                Ok(())
            })
            .unwrap();
            stack()
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
