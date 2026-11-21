//! #12606: `paint.cloneStamp`'s `ignoreAdjustments`. With Sample = All Layers and an Invert
//! adjustment layer in the document, the copied pixels are the inverted checker when it is off (as
//! today) and the checker's own pixels when it is on. Current Layer and the unmeasured Current &
//! Below keep today's result, and a non-boolean is an error.

use super::*;

/// The checker's dark squares (60, 60, 60) and light squares (200, 200, 200), 20 px each, the
/// dark one at the origin.
const DARK: f32 = 60.0 / 255.0;
const LIGHT: f32 = 200.0 / 255.0;

/// A 200 x 100 Background with the grey checker, an Invert adjustment layer above it, and an empty
/// layer "Clone" on top, active.
fn session(depth: u64) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "depth": depth})).unwrap();
    let bg = s.active().unwrap().active_layer.unwrap();
    s.edit("checker", |doc, _| {
        let surf = doc.layer_mut(bg).unwrap().surface_mut().unwrap();
        for y in 0..100 {
            for x in 0..200 {
                let v = if (x / 20 + y / 20) % 2 == 0 { DARK } else { LIGHT };
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Clone"})).unwrap();
    let clone = s.active().unwrap().active_layer.unwrap();
    (s, clone)
}

/// One hard 10 px dab at 110,10 copying from 10,10, inside the dark square at the origin.
fn clone(s: &mut Session, extra: Value) -> Result<Value> {
    let mut p = json!({"points": [[110, 10]], "offset": [-100, 0], "size": 10, "hardness": 100});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("paint.cloneStamp", p)
}

/// The grey of layer `id` at 110,10, as 8-bit.
fn grey(s: &Session, id: LayerId) -> u8 {
    let c = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(110, 10);
    assert_eq!(c[3], 1.0, "a painted pixel");
    (c[0] * 255.0).round() as u8
}

#[test]
fn all_layers_with_the_toggle_on_copies_the_pixels_under_the_adjustment_layer() {
    for depth in [8, 16, 32] {
        let (mut s, id) = session(depth);
        clone(&mut s, json!({"sampleLayer": "all", "ignoreAdjustments": true})).unwrap();
        assert_eq!(grey(&s, id), 60, "{depth}-bit: the original checker");
    }
}

#[test]
fn all_layers_with_the_toggle_off_or_omitted_copies_the_inverted_checker_as_today() {
    for depth in [8, 16, 32] {
        for extra in
            [json!({"sampleLayer": "all"}), json!({"sampleLayer": "all", "ignoreAdjustments": false}), json!({"sampleLayer": "all", "ignoreAdjustments": null})]
        {
            let (mut s, id) = session(depth);
            clone(&mut s, extra.clone()).unwrap();
            assert_eq!(grey(&s, id), 195, "{depth}-bit {extra}: the inverted checker");
        }
    }
}

/// Current Layer samples the layer itself, and Current & Below wasn't measured: the parameter
/// changes nothing for either.
#[test]
fn current_layer_and_current_and_below_keep_todays_result() {
    for which in ["current", "currentAndBelow"] {
        let result = |ignore: bool| {
            let (mut s, id) = session(8);
            if which == "current" {
                // Clone from the checker itself onto it.
                let bg = s.active().unwrap().doc.layers[0].id;
                s.execute("layer.select", json!({"layer": bg.0, "mode": "replace"})).unwrap();
                clone(&mut s, json!({"sampleLayer": which, "ignoreAdjustments": ignore})).unwrap();
                return grey(&s, bg);
            }
            clone(&mut s, json!({"sampleLayer": which, "ignoreAdjustments": ignore})).unwrap();
            grey(&s, id)
        };
        assert_eq!(result(true), result(false), "{which}");
    }
}

/// [`session`] with the Invert clipped to the layer below it.
fn clipped_session() -> (Session, LayerId) {
    let (mut s, id) = session(8);
    let invert = s.active().unwrap().doc.layers.iter().find(|l| matches!(l.content, photocraft_doc::LayerContent::Adjustment(_))).unwrap().id;
    s.edit("clip", |doc, _| {
        doc.layer_mut(invert).unwrap().clipped = true;
        Ok(())
    })
    .unwrap();
    (s, id)
}

#[test]
fn an_adjustment_layer_clipped_to_a_layer_is_not_hidden() {
    let (mut t, tid) = clipped_session();
    clone(&mut t, json!({"sampleLayer": "all"})).unwrap();
    let today = grey(&t, tid);
    let (mut s, id) = clipped_session();
    clone(&mut s, json!({"sampleLayer": "all", "ignoreAdjustments": true})).unwrap();
    assert_eq!(grey(&s, id), today, "not measured: kept as today");
}

#[test]
fn a_non_boolean_is_an_error_and_paints_nothing() {
    for bad in [json!(1), json!("yes"), json!([true])] {
        let (mut s, id) = session(8);
        let before = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(110, 10);
        assert!(clone(&mut s, json!({"sampleLayer": "all", "ignoreAdjustments": bad})).is_err(), "{bad}");
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(110, 10), before, "{bad}: nothing painted");
    }
}

/// The live stroke the canvas draws is the commit (`fromLive`), so it samples the same way.
#[test]
fn the_live_stroke_samples_the_same_way() {
    for ignore in [true, false] {
        let (s, id) = session(8);
        let p = json!({"points": [[110, 10]], "offset": [-100, 0], "size": 10, "hardness": 100, "sampleLayer": "all", "ignoreAdjustments": ignore});
        let live = super::clone::LiveRetouch::begin(&s, "paint.cloneStamp", &p).unwrap();
        let c = live.doc.layer(id).unwrap().surface().unwrap().rgba(110, 10);
        assert_eq!((c[0] * 255.0).round() as u8, if ignore { 60 } else { 195 }, "ignore={ignore}");
    }
}
