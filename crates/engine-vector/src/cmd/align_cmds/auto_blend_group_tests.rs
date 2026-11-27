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
    // Only the two layers that took part are masked: nothing else in the document gained a mask.
    let masked: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| l.mask.is_some()).map(|(_, _, l)| l.id).collect();
    assert_eq!(masked, vec![left, right], "Left and Right are the masked layers");
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
    // The merged layer holds the same pixels in both runs, not just the same shape of result.
    let (fb, gb) = {
        let (a, b) = (flat.active().unwrap(), grouped.active().unwrap());
        (
            a.doc.layer(LayerId(rf["blended"].as_u64().unwrap())).unwrap().surface().unwrap().clone(),
            b.doc.layer(LayerId(rg["blended"].as_u64().unwrap())).unwrap().surface().unwrap().clone(),
        )
    };
    assert_eq!(fb.content_bounds(), gb.content_bounds(), "the merged layer covers the same area");
    for y in (0..H).step_by(11) {
        for x in 0..W {
            assert_eq!(fb.pixel(x, y), gb.pixel(x, y), "merged pixel x {x} y {y}");
        }
    }
}

/// The group and the pixel layer inside it selected together: the branch takes part once, not
/// twice, and the merged layer still lands beside the group rather than inside it.
#[test]
fn a_group_and_its_own_layer_selected_together_blend_once() {
    let (mut s, left, right) = issue_document();
    let group = group_of(&mut s, right, "G");
    select(&mut s, &[left, group, right]);
    let before = s.active().unwrap().history.past_len();
    let states = layer_count(&s);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    assert_eq!(r["layers"], json!([left.0, right.0]), "the pair blends once");
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one history state");
    let masked: Vec<LayerId> = st.doc.walk().into_iter().filter(|(_, _, l)| l.mask.is_some()).map(|(_, _, l)| l.id).collect();
    assert_eq!(masked, vec![left, right], "one mask for each layer of the pair");
    let blended = LayerId(r["blended"].as_u64().unwrap());
    assert_eq!(st.doc.layers.index().parent(blended), None, "the merged layer sits beside the group");
    assert_eq!(st.doc.layer(group).unwrap().children().map_or(0, <[Layer]>::len), 1, "the group keeps its one layer");
    assert_eq!(layer_count(&s), states + 1, "one new layer, the merged one");
}

/// A merged layer never lands inside a group that only lent its pixels: the group is the anchor.
#[test]
fn the_merged_layer_sits_above_a_group_that_lent_its_pixels() {
    let (mut s, left, right) = issue_document();
    let group = group_of(&mut s, right, "G");
    select(&mut s, &[left, group]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let blended = LayerId(r["blended"].as_u64().unwrap());
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.index().parent(blended), None, "beside the group, at the top level");
    let root = |id: LayerId| st.doc.layers.iter().position(|l| l.id == id).expect("a top-level layer");
    assert!(root(blended) > root(group) && root(blended) > root(left), "above both entries of the selection");
}

/// A pixel layer above the group: the group still lends its layer, and the merged layer goes above
/// the top entry of the selection, at the top level.
#[test]
fn a_pixel_layer_above_the_group_stays_the_anchor() {
    let (mut s, left, right) = issue_document();
    s.execute("layer.new.layer", json!({"name": "Above"})).unwrap();
    let above = s.active().unwrap().active_layer.unwrap();
    s.edit("fill", |doc, _| {
        doc.layer_mut(above).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 400, 120), &[0.4, 0.4, 0.9, 1.0]);
        Ok(())
    })
    .unwrap();
    let group = group_of(&mut s, right, "G");
    assert_eq!(s.active().unwrap().doc.layers.index().parent(above), None, "Above sits over the group");
    select(&mut s, &[group, above]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    assert_eq!(r["layers"], json!([right.0, above.0]), "the group's layer and the layer above it ({r})");
    let st = s.active().unwrap();
    let blended = LayerId(r["blended"].as_u64().unwrap());
    assert_eq!(st.doc.layers.index().parent(blended), None, "at the top level");
    let root = |id: LayerId| st.doc.layers.iter().position(|l| l.id == id).expect("a top-level layer");
    assert_eq!(root(blended), root(above) + 1, "directly above the top entry of the selection");
    let masked: Vec<LayerId> = st.doc.walk().into_iter().filter(|(_, _, l)| l.mask.is_some()).map(|(_, _, l)| l.id).collect();
    assert_eq!(masked, vec![right, above], "both layers that took part are masked");
    assert!(st.doc.layer(left).unwrap().mask.is_none(), "an unselected layer keeps no mask");
}

/// A Stack Images run on a group pair adds the hidden Depth layer beside the group, and scales it
/// for the two layers that took part, not for three.
#[test]
fn a_group_pair_still_gets_a_depth_map_scaled_for_two_layers() {
    let (mut s, left, right) = issue_document();
    let group = group_of(&mut s, right, "G");
    select(&mut s, &[left, group]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false, "depthMap": true})).unwrap();
    assert_eq!(r["layers"], json!([left.0, right.0]), "two layers take part ({r})");
    let st = s.active().unwrap();
    let depth = LayerId(r["depth"].as_u64().expect("a depth map"));
    let d = st.doc.layer(depth).unwrap();
    assert!(!d.visible && d.name.starts_with("Depth"), "{} visible {}", d.name, d.visible);
    assert_eq!(st.doc.layers.index().parent(depth), None, "beside the group, not inside it");
    // White = the top layer: the scale is 1 / (2 - 1), so the layer above shows as 1.0.
    let surf = d.surface().unwrap();
    assert!(surf.pixel(390, 150)[0] > 0.9, "the top layer is white: {:?}", surf.pixel(390, 150));
    assert!(surf.pixel(10, 150)[0] < 0.1, "the bottom layer is black: {:?}", surf.pixel(10, 150));
}

/// Builds one unmeasured case on the issue's document and returns the layers to select.
type Build = fn(&mut Session, LayerId, LayerId) -> Vec<LayerId>;

/// Cases the issue did not measure keep today's behaviour: the group contributes nothing, so
/// Auto-Blend stays unavailable and changes neither the document nor the history.
#[test]
fn other_group_selections_keep_todays_refusal() {
    let builds: [(&str, Build); 11] = [
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
        ("a group and its own pixel layer alone", |s, _, right| {
            let g = group_of(s, right, "G");
            vec![g, right]
        }),
        ("an artboard holding one pixel layer and a pixel layer", |s, left, right| {
            let g = group_of(s, right, "G");
            s.execute("layer.new.artboardFromGroup", json!({"layer": g.0})).unwrap();
            vec![left, g]
        }),
        ("a frame holding its layer and a pixel layer", |s, left, right| {
            select(s, &[right]);
            let r = s.execute("layer.new.frameFromLayers", json!({})).unwrap();
            vec![left, LayerId(r["layer"].as_u64().unwrap())]
        }),
        ("a hidden group and a pixel layer", |s, left, right| {
            let g = group_of(s, right, "G");
            s.execute("layer.hideLayers", json!({"layer": g.0})).unwrap();
            vec![left, g]
        }),
        ("a masked group and a pixel layer", |s, left, right| {
            let g = group_of(s, right, "G");
            s.edit("mask", |doc, _| {
                doc.layer_mut(g).unwrap().mask = Some(LayerMask::reveal_all());
                Ok(())
            })
            .unwrap();
            vec![left, g]
        }),
    ];
    for (name, build) in builds {
        let (mut s, left, right) = issue_document();
        let ids = build(&mut s, left, right);
        select(&mut s, &ids);
        let before = s.active().unwrap().history.past_len();
        let states = layer_count(&s);
        let masked =
            |s: &Session| -> Vec<LayerId> { s.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| l.mask.is_some()).map(|(_, _, l)| l.id).collect() };
        let masks = masked(&s);
        assert!(!s.is_enabled("edit.autoBlendLayers"), "{name}: unavailable");
        assert!(s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).is_err(), "{name}: refused");
        let st = s.active().unwrap();
        assert_eq!(st.history.past_len(), before, "{name}: no history state");
        assert_eq!(masked(&s), masks, "{name}: no mask added");
        assert_eq!(layer_count(&s), states, "{name}: no layer added");
        assert!(!s.is_enabled("edit.autoAlignLayers"), "{name}: Auto-Align stays as it was");
    }
}

/// A group that fades or blends its own pixels is a case the issue did not measure and is the one
/// the change does not keep at today's refusal: it still lends its layer, and the masks stay
/// complementary because the group's own coverage is what it was before the run. Asked on #14206.
#[test]
fn a_faded_or_blended_group_still_lends_its_pixels() {
    for (name, blend, opacity) in
        [("Opacity 50", photocraft_color::BlendMode::PassThrough, 0.5f32), ("Multiply at full opacity", photocraft_color::BlendMode::Multiply, 1.0)]
    {
        let (mut s, left, right) = issue_document();
        let group = group_of(&mut s, right, "G");
        s.edit("group look", |doc, _| {
            let g = doc.layer_mut(group).unwrap();
            g.blend = blend;
            g.opacity = opacity;
            Ok(())
        })
        .unwrap();
        select(&mut s, &[left, group]);
        assert!(s.is_enabled("edit.autoBlendLayers"), "{name}: available");
        let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama", "seamlessTones": false})).unwrap();
        assert_eq!(r["layers"], json!([left.0, right.0]), "{name}: the pair takes part ({r})");
        let st = s.active().unwrap();
        let (ml, mr) = (st.doc.layer(left).unwrap().mask.clone().unwrap(), st.doc.layer(right).unwrap().mask.clone().unwrap());
        for x in (0..W).step_by(3) {
            let (a, b) = (ml.value(x, H / 2), mr.value(x, H / 2));
            assert!((a + b - 1.0).abs() < 0.02, "{name}: x {x} sums {a} + {b} to one layer");
        }
        assert!(st.doc.layer(group).unwrap().mask.is_none(), "{name}: the group keeps no mask");
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
