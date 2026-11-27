//! #14206: Auto-Blend Layers takes the pixels of a selected group into the blend, next to a
//! selected pixel layer, instead of dropping the group and refusing.

use super::*;

const W: i32 = 400;
const H: i32 = 300;

/// The issue's scene - six coloured blocks over a noise floor - multiplied by `gain`, written into
/// `id` over x `x0..x1` of the 400 x 300 canvas.
fn paint_scene(s: &mut Session, id: LayerId, x0: i32, x1: i32, gain: f32) {
    s.edit("scene", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let blocks: [(Rect, [f32; 3]); 6] = [
            (Rect::new(30, 40, 90, 110), [0.9, 0.1, 0.1]),
            (Rect::new(120, 60, 190, 140), [0.1, 0.8, 0.2]),
            (Rect::new(200, 170, 260, 250), [0.1, 0.2, 0.9]),
            (Rect::new(250, 30, 320, 90), [0.9, 0.8, 0.1]),
            (Rect::new(300, 150, 370, 230), [0.8, 0.2, 0.8]),
            (Rect::new(60, 190, 140, 260), [0.1, 0.8, 0.8]),
        ];
        let mut seed = 7u64;
        for y in 0..H {
            for x in x0..x1 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let noise = ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.06;
                let c = blocks.iter().find(|(r, _)| r.contains(x, y)).map_or([0.5; 3], |(_, c)| *c);
                let px = c.map(|v| ((v + noise) * gain).clamp(0.0, 1.0));
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[px[0], px[1], px[2], 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
}

/// The issue's test document: Left (the scene at x 0..240) and Right (the scene at x 160..400,
/// darkened), overlapping at x 160..240, no Background.
fn issue_document() -> (Session, LayerId, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Left"})).unwrap();
    let left = s.active().unwrap().active_layer.unwrap();
    paint_scene(&mut s, left, 0, 240, 1.0);
    s.execute("layer.new.layer", json!({"name": "Right"})).unwrap();
    let right = s.active().unwrap().active_layer.unwrap();
    paint_scene(&mut s, right, 160, 400, 0.8);
    (s, left, right)
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
}

/// Wraps `child` in a new group and returns the group.
fn group_of(s: &mut Session, child: LayerId, name: &str) -> LayerId {
    select(s, &[child]);
    let r = s.execute("layer.groupLayers", json!({"name": name})).unwrap();
    LayerId(r["layer"].as_u64().expect("layer.groupLayers names the new group"))
}

/// How many layers the document holds, nested ones included.
fn layer_count(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

/// The issue's run: group `G` (holding Right) and the pixel layer Left selected. Auto-Blend is
/// available, runs, and masks Left and Right into the two sides of the seam.
#[test]
fn a_group_and_a_pixel_layer_blend_together() {
    let (mut s, left, right) = issue_document();
    let group = group_of(&mut s, right, "G");
    select(&mut s, &[left, group]);
    assert!(s.is_enabled("edit.autoBlendLayers"), "a group and a pixel layer selected");
    let before = s.active().unwrap().history.past_len();
    let states = layer_count(&s);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one history state");
    assert_eq!(st.history.undo_label(), Some("Auto-Blend Layers"));
    let doc = &st.doc;
    // The pixel layer and the group's pixel layer each take part and carry their side of the seam.
    let ml = doc.layer(left).unwrap().mask.as_ref().expect("Left gets a mask");
    let mr = doc.layer(right).unwrap().mask.as_ref().expect("Right gets a mask");
    assert!(ml.value(10, H / 2) > 0.9 && mr.value(10, H / 2) < 0.1, "Left keeps its own side");
    assert!(mr.value(390, H / 2) > 0.9 && ml.value(390, H / 2) < 0.1, "the group's layer keeps its side");
    // Complementary along the seam: no column of the overlap is covered twice or left uncovered.
    for x in 0..W {
        let (a, b) = (ml.value(x, H / 2), mr.value(x, H / 2));
        assert!((a + b - 1.0).abs() < 0.02, "x {x}: Left {a} + the group {b} is not one layer");
    }
    // The group itself is untouched: no mask of its own, still the one layer it held, and that
    // layer's pixels where they were.
    let g = doc.layer(group).unwrap();
    assert!(g.mask.is_none(), "the group carries no mask");
    assert_eq!(g.children().map_or(0, <[Layer]>::len), 1, "the group still holds one layer");
    assert_eq!(doc.layer(right).unwrap().surface().unwrap().content_bounds(), Rect::new(160, 0, 400, 300), "Right's pixels stay put");
    // One merged layer, above the group rather than inside it.
    assert_eq!(layer_count(&s), states + 1, "one new layer, the merged one ({r})");
    let blended = LayerId(r["blended"].as_u64().expect("Seamless Tones is on by default"));
    assert!(doc.layer(blended).unwrap().name.starts_with("Blended"), "{blended:?}");
    assert_eq!(doc.layers.index().parent(blended), None, "the merged layer sits beside the group");
    s.undo();
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(left).unwrap().mask.is_none() && doc.layer(right).unwrap().mask.is_none(), "undo removes both masks");
    assert_eq!(layer_count(&s), states, "undo drops the merged layer");
}

/// The group's run and the plain run of the same two pixel layers give the same seam: the group
/// only decides which layers take part, nothing else.
#[test]
fn a_group_selection_blends_like_the_same_two_pixel_layers() {
    let (mut flat, left, right) = issue_document();
    select(&mut flat, &[left, right]);
    let flat_states = layer_count(&flat);
    let rf = flat.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let (mut grouped, g_left, g_right) = issue_document();
    let group = group_of(&mut grouped, g_right, "G");
    select(&mut grouped, &[g_left, group]);
    let grouped_states = layer_count(&grouped);
    let rg = grouped.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let (ml, mr) = {
        let st = flat.active().unwrap();
        (st.doc.layer(left).unwrap().mask.clone().unwrap(), st.doc.layer(right).unwrap().mask.clone().unwrap())
    };
    let (gl, gr) = {
        let st = grouped.active().unwrap();
        (st.doc.layer(g_left).unwrap().mask.clone().unwrap(), st.doc.layer(g_right).unwrap().mask.clone().unwrap())
    };
    for y in (0..H).step_by(7) {
        for x in 0..W {
            assert_eq!((ml.value(x, y), mr.value(x, y)), (gl.value(x, y), gr.value(x, y)), "x {x} y {y}");
        }
    }
    assert_eq!(rf["blended"].is_null(), rg["blended"].is_null(), "the same merged layer");
    assert_eq!(layer_count(&flat), flat_states + 1, "one new layer in the plain run");
    assert_eq!(layer_count(&grouped), grouped_states + 1, "one new layer in the group run");
}

/// Cases the issue did not measure keep today's behaviour: the group contributes nothing, so
/// Auto-Blend stays unavailable and changes neither the document nor the history.
#[test]
fn other_group_selections_keep_todays_refusal() {
    let builds: [(&str, fn(&mut Session, LayerId, LayerId) -> Vec<LayerId>); 6] = [
        ("a group alone", |s, _, right| vec![group_of(s, right, "G")]),
        ("an empty group and a pixel layer", |s, left, _| {
            let r = s.execute("layer.new.group", json!({"name": "Empty"})).unwrap();
            vec![left, LayerId(r["layer"].as_u64().unwrap())]
        }),
        ("a group of two pixel layers and a pixel layer", |s, left, right| {
            s.execute("layer.new.layer", json!({"name": "Third"})).unwrap();
            let third = s.active().unwrap().active_layer.unwrap();
            select(s, &[right, third]);
            let r = s.execute("layer.groupLayers", json!({"name": "G"})).unwrap();
            vec![left, LayerId(r["layer"].as_u64().unwrap())]
        }),
        ("an outer group and a pixel layer", |s, left, right| {
            let inner = group_of(s, right, "Inner");
            let r = s.execute("layer.groupLayers", json!({"layer": inner.0, "name": "Outer"})).unwrap();
            vec![left, LayerId(r["layer"].as_u64().unwrap())]
        }),
        ("an inner group and a pixel layer", |s, left, right| {
            let inner = group_of(s, right, "Inner");
            s.execute("layer.groupLayers", json!({"layer": inner.0, "name": "Outer"})).unwrap();
            vec![left, inner]
        }),
        ("two groups and a pixel layer", |s, left, right| {
            let g1 = group_of(s, right, "One");
            s.execute("layer.new.layer", json!({"name": "Third"})).unwrap();
            let third = s.active().unwrap().active_layer.unwrap();
            let g2 = group_of(s, third, "Two");
            vec![left, g1, g2]
        }),
    ];
    for (name, build) in builds {
        let (mut s, left, right) = issue_document();
        let ids = build(&mut s, left, right);
        select(&mut s, &ids);
        let before = s.active().unwrap().history.past_len();
        let states = layer_count(&s);
        assert!(!s.is_enabled("edit.autoBlendLayers"), "{name}: unavailable");
        assert!(s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).is_err(), "{name}: refused");
        let st = s.active().unwrap();
        assert_eq!(st.history.past_len(), before, "{name}: no history state");
        assert!(st.doc.walk().into_iter().all(|(_, _, l)| l.mask.is_none()), "{name}: no mask added");
        assert_eq!(layer_count(&s), states, "{name}: no layer added");
        assert!(!s.is_enabled("edit.autoAlignLayers"), "{name}: Auto-Align stays as it was");
    }
}

/// Auto-Align's selection rule is unchanged: a group and a pixel layer is still too few pixel
/// layers, while two pixel layers are enough.
#[test]
fn auto_align_still_counts_only_pixel_layers() {
    let (mut s, left, right) = issue_document();
    let group = group_of(&mut s, right, "G");
    select(&mut s, &[left, group]);
    assert!(!s.is_enabled("edit.autoAlignLayers"), "a group is not a pixel layer here");
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err(), "and still refused");
    select(&mut s, &[left, right]);
    assert!(s.is_enabled("edit.autoAlignLayers"), "two pixel layers still pass");
}
