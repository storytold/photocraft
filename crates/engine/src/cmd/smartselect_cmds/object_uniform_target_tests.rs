//! Object Selection clicks on uniform, group and empty-area targets (#12863): the reference
//! application answers with partly-selected areas (coverage 254) instead of errors or a status
//! message, and a click on a multi-object layer's empty spot stays a plain nothing-found.
//!
//! The document is the one the issue measured against the reference application: 800x600 RGB/8,
//! white Background; pixel layer Shapes with a red (220,30,30) rectangle 100,100-300,250, a blue
//! (30,30,220) disc centre 500,175 radius 75 and a grey ramp at y 350-450, transparent elsewhere;
//! pixel layer Green on top with a green (30,160,30) disc centre 650,420 radius 60, transparent
//! elsewhere. Object Selection, Object Finder on, Sample All Layers off.

use super::*;

const W: i32 = 800;
const H: i32 = 600;

/// The issue's document: Background, Shapes, Green; Green (the newest layer) is active.
fn issue_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    paint_new_layer(&mut s, |x, y| {
        if (100..300).contains(&x) && (100..250).contains(&y) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        let (dx, dy) = (x as f32 + 0.5 - 500.0, y as f32 + 0.5 - 175.0);
        if dx * dx + dy * dy <= 75.0 * 75.0 {
            return [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, 1.0];
        }
        if (100..500).contains(&x) && (350..450).contains(&y) {
            let v = (100 + 5 * ((x - 100) / 40)) as f32 / 255.0;
            return [v, v, v, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    s.execute("layer.new.layer", json!({"name": "Green"})).unwrap();
    paint_new_layer(&mut s, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - 650.0, y as f32 + 0.5 - 420.0);
        if dx * dx + dy * dy <= 60.0 * 60.0 {
            return [30.0 / 255.0, 160.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    s
}

/// Writes `px` (straight RGBA) over the whole surface of the last layer, which
/// `layer.new.layer` leaves transparent.
fn paint_new_layer(s: &mut Session, px: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let n = surf.channels();
        let mut data = Vec::with_capacity((W * H) as usize * n);
        for y in 0..H {
            for x in 0..W {
                let c = px(x, y);
                if n >= 4 {
                    data.extend_from_slice(&c);
                } else {
                    data.extend_from_slice(&c[..n.min(3)]);
                }
            }
        }
        surf.write_region(Rect::new(0, 0, W, H), &data);
        Ok(())
    })
    .unwrap();
}

fn select_layer(s: &mut Session, name: &str) {
    let id = s.active().unwrap().doc.layers.iter().find(|l| l.name == name).unwrap().id;
    s.execute("layer.select", json!({"layer": id})).unwrap();
}

/// "A group active (Shapes inside it)": Group Layers wraps the Shapes layer in a plain group,
/// which is then the active layer.
fn group_shapes(s: &mut Session) {
    let id = s.active().unwrap().doc.layers.iter().find(|l| l.name == "Shapes").unwrap().id;
    s.execute("layer.groupLayers", json!({"layer": id, "name": "Group"})).unwrap();
    select_layer(s, "Group");
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(d.active_layer.unwrap()).unwrap().content.kind_name(), "Group");
}

/// The selection's 8-bit coverage at a document pixel.
fn cov8(s: &Session, x: i32, y: i32) -> u8 {
    (s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0)) * 255.0).round() as u8
}

/// `[left, top, right, bottom]` of the selection, as the issue writes bounds.
fn bounds(s: &Session) -> [i32; 4] {
    let b = s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

/// Over the whole canvas: (fully selected pixels, partly selected pixels, pixels at exactly 254).
fn select_counts(s: &Session) -> (usize, usize, usize) {
    let sel = s.active().unwrap().doc.selection.clone();
    let (mut full, mut part, mut at_254) = (0, 0, 0);
    for y in 0..H {
        for x in 0..W {
            let v = (sel.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0)) * 255.0).round() as u8;
            if v == 255 {
                full += 1;
            } else if v > 0 {
                part += 1;
                if v == 254 {
                    at_254 += 1;
                }
            }
        }
    }
    (full, part, at_254)
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// Done when #12863 line 1: Background active (uniform white), a click anywhere: the whole
/// canvas, bounds 0,0,800,600, 0 fully selected, all 480000 pixels partly selected at 254, in
/// one "Object Selection" step.
#[test]
fn background_active_click_selects_the_whole_canvas_at_254() {
    for (x, y) in [(200, 175), (650, 420), (5, 5)] {
        let mut s = issue_document();
        select_layer(&mut s, "Background");
        let before = steps(&s);
        let r = s.execute("select.object", json!({"point": [x, y]})).unwrap();
        assert_eq!(r["changed"], true, "Background click at {x},{y}");
        assert_eq!(bounds(&s), [0, 0, 800, 600], "Background click at {x},{y}");
        assert_eq!(select_counts(&s), (0, 480_000, 480_000), "Background click at {x},{y}");
        assert_eq!(steps(&s), before + 1, "one step for the click at {x},{y}");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Object Selection"));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "undone in one go at {x},{y}");
    }
}

/// Done when #12863 line 2: a group active (Shapes inside it), Sample All Layers off, a click
/// on red: the same whole-canvas result at 254, as on a uniform layer.
#[test]
fn group_active_click_on_red_selects_the_whole_canvas_at_254() {
    let mut s = issue_document();
    group_shapes(&mut s);
    let before = steps(&s);
    let r = s.execute("select.object", json!({"point": [200, 150]})).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(bounds(&s), [0, 0, 800, 600]);
    assert_eq!(select_counts(&s), (0, 480_000, 480_000));
    assert_eq!(steps(&s), before + 1, "one step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Object Selection"));
}

/// Done when #12863 line 3: Green active, a click at 200,175 (transparent on Green): the whole
/// canvas except the green disc, partly selected, in one step.
#[test]
fn green_active_click_on_transparency_selects_the_canvas_except_the_disc() {
    let mut s = issue_document();
    let before = steps(&s);
    let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
    assert_eq!(r["changed"], true, "the click no longer errors and answers with an area");
    assert_eq!(bounds(&s), [0, 0, 800, 600]);
    let (full, part, at_254) = select_counts(&s);
    assert_eq!(full, 0, "nothing fully selected");
    // The canvas minus the green disc's own pixels. The reference application measures 468698
    // with its model mask ("compare which kind of area gets selected, not exact pixel counts");
    // our fixture's disc raster is two pixels wider, so assert the rule, not the model's count.
    let disc = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let (dx, dy) = (*x as f32 + 0.5 - 650.0, *y as f32 + 0.5 - 420.0);
            dx * dx + dy * dy <= 3600.0
        })
        .count();
    assert_eq!(part, (W * H) as usize - disc, "the canvas minus the green disc, as measured");
    assert_eq!(at_254, (W * H) as usize - disc, "all of it at 254");
    assert_eq!(cov8(&s, 650, 420), 0, "the green disc itself stays unselected");
    assert_eq!(cov8(&s, 200, 175), 254, "the clicked spot is partly selected");
    assert_eq!(steps(&s), before + 1, "one step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Object Selection"));
}

/// Done when #12863 line 4: Shapes active, a click at 650,420 on the green disc (transparent on
/// Shapes): nothing selected, no step (the reference application's silent click).
#[test]
fn shapes_active_click_on_the_disc_position_changes_nothing() {
    let mut s = issue_document();
    select_layer(&mut s, "Shapes");
    let before = steps(&s);
    let r = s.execute("select.object", json!({"point": [650, 420]})).unwrap();
    assert_eq!(r["changed"], false, "a multi-object layer's empty spot finds nothing");
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(steps(&s), before, "no history step");
}

/// Measured "as today": Green active, a click on the green disc: the disc itself.
#[test]
fn green_active_click_on_the_disc_still_selects_only_the_disc() {
    let mut s = issue_document();
    let r = s.execute("select.object", json!({"point": [650, 420]})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov8(&s, 650, 420) >= 128, "the disc is selected");
    assert!(cov8(&s, 200, 175) < 128 && cov8(&s, 500, 175) < 128, "nothing else joins it");
    assert_eq!(bounds(&s), [589, 359, 711, 481], "the disc, as the issue notes today");
}

/// Neighbour: the working red-rectangle click on Shapes is unchanged.
#[test]
fn shapes_active_click_on_red_still_selects_the_red_rectangle() {
    let mut s = issue_document();
    select_layer(&mut s, "Shapes");
    let r = s.execute("select.object", json!({"point": [200, 150]})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov8(&s, 200, 150) >= 128, "the red rectangle stays found");
    assert!(cov8(&s, 500, 175) < 128 && cov8(&s, 650, 420) < 128, "and stays on its own object");
    assert_eq!(bounds(&s), [99, 99, 301, 251]);
}

/// Neighbour, unmeasured so pinned to today: Sample All Layers on keeps answering from the
/// composite.
#[test]
fn sample_all_layers_on_green_active_still_answers_from_the_composite() {
    let mut s = issue_document();
    let r = s.execute("select.object", json!({"point": [200, 175], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["changed"], true, "the composite's red rectangle, as today");
    assert!(cov8(&s, 200, 150) >= 128, "the clicked object is the composite's red");
    assert!(cov8(&s, 650, 420) < 128 && cov8(&s, 500, 175) < 128, "and stays one object");
}

/// Neighbour, unmeasured so pinned to today: a group with Sample All Layers on still reads the
/// composite.
#[test]
fn group_with_sample_all_layers_on_still_reads_the_composite() {
    let mut s = issue_document();
    group_shapes(&mut s);
    let r = s.execute("select.object", json!({"point": [200, 150], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov8(&s, 200, 150) >= 128, "red 1 of the grouped Shapes");
    assert!(cov8(&s, 650, 420) < 128 && cov8(&s, 500, 175) < 128, "one composite object, as today");
}

/// Neighbour, not measured so pinned to today: an adjustment layer active keeps answering
/// from the composite (no area answer - Smart Object and type layers take the same fallback).
#[test]
fn an_adjustment_layer_active_still_answers_from_the_composite() {
    let mut s = issue_document();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let r = s.execute("select.object", json!({"point": [200, 150]})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov8(&s, 200, 150) >= 128, "the composite's rectangle (here inverted), as today");
    assert!(cov8(&s, 500, 175) < 128 && cov8(&s, 650, 420) < 128, "and stays one object");
}

/// Neighbour, pinned to today: the rectangle path keeps the #5246 error over an empty area.
#[test]
fn a_rectangle_over_green_transparency_still_errors() {
    let mut s = issue_document();
    let e = s.execute("select.object", json!({"rect": [100, 100, 150, 150]})).unwrap_err().to_string();
    assert!(e.contains("no pixels in that area") && e.contains("sampleAllLayers"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none());
}

/// Neighbour, unmeasured so pinned to today: a point outside the canvas keeps the #5246 answer.
#[test]
fn a_point_outside_the_canvas_still_errors_on_a_pixel_layer() {
    let mut s = issue_document();
    select_layer(&mut s, "Shapes");
    let e = s.execute("select.object", json!({"point": [820, 300]})).unwrap_err().to_string();
    assert!(e.contains("no pixels in that area"), "{e}");
}

/// An 800x600 document whose Background is painted flat `color`, Background active.
fn flat_background_document(color: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// #15472 review F1: flatness must not depend on the working step. 800 % 3 = 2, so a scaled
/// scan's edge boxes average different pixel counts and their float means sit an ulp apart for
/// most 8-bit colours; a uniform RED Background is as flat as white and answers the whole canvas.
#[test]
fn uniform_red_background_click_selects_the_whole_canvas_at_254() {
    let mut s = flat_background_document("#dc1e1e");
    let before = steps(&s);
    let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
    assert_eq!(r["changed"], true, "a uniform red fill is as flat as white");
    assert_eq!(bounds(&s), [0, 0, 800, 600]);
    assert_eq!(select_counts(&s), (0, 480_000, 480_000));
    assert_eq!(steps(&s), before + 1, "one step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Object Selection"));
}

/// #15472 review F1 guard: a layer that is not flat - here one deviating pixel - keeps the
/// plain nothing-found answer.
#[test]
fn one_deviating_pixel_breaks_the_uniform_answer() {
    let mut s = flat_background_document("#dc1e1e");
    s.execute("paint.pencil", json!({"points": [[600, 400]], "size": 2, "color": "#000000"})).unwrap();
    let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
    assert_eq!(r["changed"], false, "one dark pixel makes the layer not flat");
    assert!(s.active().unwrap().doc.selection.is_none());
}

/// #15472 review F2: Hard Edge off (#12855) softens found objects, not the area answers:
/// soften_edge's taps sum to 1.0 and its ramp maps every 254 back to fully selected 255,
/// contradicting the measured partly-selected answer. The area answer keeps 254.
#[test]
fn hard_edge_off_keeps_area_answers_partly_selected() {
    let mut s = issue_document();
    select_layer(&mut s, "Background");
    let r = s.execute("select.object", json!({"point": [5, 5], "hardEdge": false})).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(select_counts(&s), (0, 480_000, 480_000), "still 254 everywhere, none at 255");
}

/// #15472 review F3, guard test passing before and after this change: the area answer reads
/// the layer's raw pixels, exactly like the object lookup under it - a hide-all layer mask
/// hides on the canvas but does not gate the answer.
#[test]
fn a_hide_all_layer_mask_does_not_gate_the_area_answer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Flat"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
    assert_eq!(r["changed"], true, "the raw surface decides, mask ignored, as the object lookup does");
    assert_eq!(select_counts(&s), (0, 480_000, 480_000));
}

/// #15472 review round 2 (smartselect_cmds.rs:489): flatness is judged over the whole canvas,
/// not over the layer's content box. A solid rectangle on a transparent layer is one opaque
/// object: a click well outside it answers the canvas with the rectangle cut out (the green
/// disc's rule), not the uncut whole canvas at 254.
#[test]
fn a_solid_rectangle_on_a_transparent_layer_is_cut_from_the_area_answer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Card"})).unwrap();
    paint_new_layer(&mut s, |x, y| {
        if (100..300).contains(&x) && (100..250).contains(&y) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    let r = s.execute("select.object", json!({"point": [650, 420]})).unwrap();
    assert_eq!(r["changed"], true, "one opaque object, a click on the layer's transparency");
    assert_eq!(bounds(&s), [0, 0, 800, 600]);
    assert_eq!(cov8(&s, 200, 150), 0, "the rectangle stays unselected, not 254");
    assert_eq!(cov8(&s, 650, 420), 254, "the clicked spot is partly selected");
    assert_eq!(select_counts(&s), (0, 480_000 - 200 * 150, 480_000 - 200 * 150), "the canvas minus the rectangle, all at 254");
}

/// #15472 review round 2 (smartselect_cmds.rs:399): the area answer's bounds follow the cut
/// selection. A full-height stripe along the canvas edge zeroes whole mask columns, so the
/// answer trims to the selected pixels' extent - trimmed()'s rule for found objects. An
/// interior cut (the disc test above) still keeps the whole canvas.
#[test]
fn a_full_height_cut_trims_the_area_answer_bounds_to_the_selected_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Stripe"})).unwrap();
    paint_new_layer(&mut s, |x, _y| {
        if x < 100 {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    let r = s.execute("select.object", json!({"point": [400, 300]})).unwrap();
    assert_eq!(r["changed"], true, "one opaque object, a click on its transparency");
    assert_eq!(bounds(&s), [100, 0, 800, 600], "the selection reaches x 100, not the cut-away edge");
    assert_eq!(r["bounds"], json!([100, 0, 700, 600]), "the reported bounds match the selection");
    assert_eq!(select_counts(&s), (0, 480_000 - 100 * 600, 480_000 - 100 * 600), "the canvas minus the stripe, all at 254");
}

/// #15472 review round 2 (smartselect_cmds.rs:483): a Smart Object's click keeps its answer
/// from before #12863. Layer::surface hands the area path the rendered cache, and the cache is
/// flat over its own crop - so without a filter a click on the document's empty canvas beside
/// a smart object answers the whole canvas. The pre-#12863 answer there is #5246's error.
#[test]
fn a_smart_object_target_takes_no_area_answer_and_keeps_its_no_pixels_error() {
    let mut s = flat_background_document("#ffffff");
    s.execute("layer.new.layer", json!({"name": "Card"})).unwrap();
    paint_new_layer(&mut s, |x, y| {
        if (100..300).contains(&x) && (100..250).contains(&y) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let d = s.active().unwrap();
    assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().surface().is_some(), "the converted smart object carries its rendered cache");
    let e = s.execute("select.object", json!({"point": [650, 420]})).unwrap_err().to_string();
    assert!(e.contains("no pixels in that area") && e.contains("sampleAllLayers"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none(), "and selects nothing");
}

/// Writes `px` (straight RGBA) over `rect`, which may reach past the canvas edge: a layer surface
/// keeps its off-canvas tiles (crates/engine/src/cmd/layer_menu_cmds.rs:184, #3043).
fn paint_rect(s: &mut Session, rect: Rect, px: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let n = surf.channels();
        let mut data = Vec::with_capacity((rect.width() * rect.height()) as usize * n);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let c = px(x, y);
                if n >= 4 {
                    data.extend_from_slice(&c);
                } else {
                    data.extend_from_slice(&c[..n.min(3)]);
                }
            }
        }
        surf.write_region(rect, &data);
        Ok(())
    })
    .unwrap();
}

/// #15472 review round 3 (smartselect_cmds.rs:554): #12863 lists the layer mask and a channel
/// "as the target" among the cases that keep today's behaviour. With the layer mask the edit
/// target, a flat raster's click keeps its pre-#12863 answer: nothing found, no selection, no
/// step. Quick Selection applies the same gate to its #12919 transparency drop (:279).
#[test]
fn a_targeted_layer_mask_takes_no_area_answer() {
    let mut s = flat_background_document("#dc1e1e");
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    for channel in ["mask", "red"] {
        s.execute("channel.target", json!({"channel": channel})).unwrap();
        let d = s.active().unwrap();
        assert!(d.channel_view.target != crate::channel_cmds::ChannelTarget::Composite, "{channel} is the edit target");
        let before = steps(&s);
        let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
        assert_eq!(r["changed"], false, "a {channel} target keeps today's nothing-found answer");
        assert!(s.active().unwrap().doc.selection.is_none(), "and leaves no selection, {channel}");
        assert_eq!(steps(&s), before, "no history step, {channel}");
    }
    // Back on the composite, the same click keeps the measured area answer.
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let r = s.execute("select.object", json!({"point": [200, 175]})).unwrap();
    assert_eq!(r["changed"], true, "a non-mask target keeps the whole-canvas answer");
    assert_eq!(select_counts(&s), (0, 480_000, 480_000), "the composite target answers 254 everywhere");
}

/// #15472 review round 3 (smartselect_cmds.rs:695): the opaque-part gate read the working scan,
/// which averages a cell. A 400 x 300 card scans at step 2, so a 1 px transparent hole inside it
/// sits in a cell of mean alpha 0.75 - the click was judged to be on the opaque part and answered
/// nothing-found. The clicked pixel is transparent, and the layer holds exactly one opaque object:
/// the documented answer is the canvas with the card's own pixels cut out, the hole at 254.
#[test]
fn a_one_pixel_hole_in_a_card_is_a_transparent_spot_not_the_opaque_part() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Card"})).unwrap();
    paint_new_layer(&mut s, |x, y| {
        if (100..500).contains(&x) && (100..400).contains(&y) && (x, y) != (301, 251) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    let r = s.execute("select.object", json!({"point": [301, 251]})).unwrap();
    assert_eq!(r["changed"], true, "the clicked pixel is transparent on the card");
    assert_eq!(bounds(&s), [0, 0, 800, 600], "the canvas, as the disc's rule");
    assert_eq!(cov8(&s, 301, 251), 254, "the hole joins the area answer");
    assert_eq!(cov8(&s, 300, 251), 0, "the card's own pixels stay unselected");
    assert_eq!(select_counts(&s), (0, 480_000 - (400 * 300 - 1), 480_000 - (400 * 300 - 1)), "the canvas minus the card's pixels, all at 254");
}

/// #15472 review round 3 (smartselect_cmds.rs:462): the predicate marks the points where the click
/// runs #5246's check, and that check tests the layer's own pixels. A layer whose pixels hang past
/// the canvas edge has pixels at an out-of-canvas point, so no message applies there: the click
/// keeps the silent nothing-found it had before #12863, and the predicate says the same.
#[test]
fn an_out_of_canvas_point_over_pixels_past_the_edge_promises_no_error() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Overhang"})).unwrap();
    paint_rect(&mut s, Rect::new(W - 10, 100, W + 90, 120), |_, _| [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]);
    let (doc, layer) = {
        let d = s.active().unwrap();
        (std::sync::Arc::clone(&d.doc), d.active_layer)
    };
    let surface = doc.layer(layer.unwrap()).unwrap().surface().unwrap();
    assert!(surface.content_bounds().x1 > W, "the layer's pixels reach past the canvas edge: {}", surface.content_bounds().x1);
    assert!(!point_reports_sampled_pixels_error(&doc, layer, false, Rect::from_xywh(W + 20, 110, 1, 1)), "no #5246 message where the layer has pixels");
    assert!(point_reports_sampled_pixels_error(&doc, layer, false, Rect::from_xywh(W + 200, 110, 1, 1)), "and the message where it has none");
    let r = s.execute("select.object", json!({"point": [W + 20, 110]})).unwrap();
    assert_eq!(r["changed"], false, "the click over the off-canvas pixels stays the silent nothing-found");
    assert!(s.active().unwrap().doc.selection.is_none(), "and selects nothing");
}

/// #15472 review round 2 (smartselect_cmds.rs:483): same for a type layer whose rendered cache
/// is flat - its click keeps #5246's clear error, never an area answer.
#[test]
fn a_type_target_takes_no_area_answer_and_keeps_its_no_pixels_error() {
    let mut s = flat_background_document("#ffffff");
    s.edit("type", |doc, active| {
        let mut t = photocraft_doc::TextLayer { text: "Hi".into(), ..Default::default() };
        let fmt = doc.pixel_format();
        let mut cache = photocraft_raster::Surface::new(fmt);
        cache.fill_rect(Rect::new(400, 300, 460, 340), &photocraft_raster::from_rgba(&fmt, [0.4, 0.2, 0.6, 1.0]));
        t.cache = Some(cache);
        let id = doc.insert_above(None, Layer::new("Type", LayerContent::Text(t)));
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    let e = s.execute("select.object", json!({"point": [100, 100]})).unwrap_err().to_string();
    assert!(e.contains("no pixels in that area") && e.contains("sampleAllLayers"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none(), "and selects nothing");
}
