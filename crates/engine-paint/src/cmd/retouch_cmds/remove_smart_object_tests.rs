//! The Remove tool on a Smart Object layer (#12555): its fill goes to a new normal layer named
//! "Remove tool edits" directly above the smart object, which keeps its own pixels, with Create
//! new layer off as well as on. The removal is one history step, and one undo takes the fill and
//! the new layer back together. A locked Smart Object, a layer mask target and a pixel layer keep
//! today's behaviour.

use super::*;

/// A grey layer with a dark square at 40..52 × 24..36, converted to a Smart Object over the
/// Background.
fn smart_session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 96, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_square(&mut s);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s
}

/// Fill the active pixel layer with grey and a dark square at 40..52 × 24..36.
fn paint_square(s: &mut Session) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if (40..52).contains(&x) && (24..36).contains(&y) { 0.05 } else { 0.5 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

/// One stroke across the square, with the options bar's Create new layer as `new_layer`.
fn remove_stroke(s: &mut Session, new_layer: bool) -> Value {
    s.execute("paint.remove", json!({"points": STROKE, "size": 16, "newLayer": new_layer})).unwrap()
}

const STROKE: [[i32; 2]; 2] = [[38, 30], [54, 30]];

fn order(s: &Session) -> Vec<LayerId> {
    s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).collect()
}

fn name(s: &Session, id: LayerId) -> String {
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

fn kind(s: &Session, id: LayerId) -> &'static str {
    s.active().unwrap().doc.layer(id).unwrap().content.kind_name()
}

/// The Smart Object's own pixels: its rendered cache.
fn pixels(s: &Session, id: LayerId) -> Surface {
    s.active().unwrap().doc.layer(id).unwrap().surface().expect("the smart object holds rendered pixels").clone()
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn last_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

/// The removal's new layer, checked with Create new layer as `new_layer`: the smart object keeps
/// its kind and its pixels, the new layer sits directly above it and carries the fill.
fn check_edits_layer(new_layer: bool) {
    let mut s = smart_session();
    let smart = s.active().unwrap().active_layer.unwrap();
    let below = order(&s);
    let before = pixels(&s, smart);
    let r = remove_stroke(&mut s, new_layer);
    let id = LayerId(r["layer"].as_u64().unwrap_or_else(|| panic!("no new layer: {r}")));
    assert_eq!(name(&s, id), "Remove tool edits", "newLayer {new_layer}");
    assert_eq!(order(&s), [below, vec![id]].concat(), "the edits layer sits directly above the smart object");
    assert_eq!(kind(&s, id), "Pixel", "a normal pixel layer");
    assert_eq!(kind(&s, smart), "Smart Object", "the smart object stays a smart object");
    assert_eq!(pixels(&s, smart), before, "the smart object's pixels are untouched, dark square and all");
    assert!((pixels(&s, smart).rgba(46, 30)[0] - 0.05).abs() < 0.02, "the square is still there underneath: {:?}", pixels(&s, smart).rgba(46, 30));
    let fill = pixels(&s, id);
    assert!((fill.rgba(46, 30)[0] - 0.5).abs() < 0.03, "the new layer carries the fill sampled from the smart object: {:?}", fill.rgba(46, 30));
    assert_eq!(fill.rgba(46, 30)[3], 1.0, "the fill is opaque where the hole was");
    assert_eq!(s.active().unwrap().active_layer, Some(id), "the new layer is the active one");
}

#[test]
fn a_remove_stroke_on_a_smart_object_writes_to_remove_tool_edits_with_create_new_layer_off() {
    check_edits_layer(false);
}

#[test]
fn a_remove_stroke_on_a_smart_object_writes_to_remove_tool_edits_with_create_new_layer_on() {
    check_edits_layer(true);
}

#[test]
fn the_removal_is_one_step_and_one_undo_takes_the_fill_and_the_layer_back() {
    for new_layer in [false, true] {
        let mut s = smart_session();
        let smart = s.active().unwrap().active_layer.unwrap();
        let below = order(&s);
        let before = pixels(&s, smart);
        let steps = past(&s);
        let r = remove_stroke(&mut s, new_layer);
        let id = LayerId(r["layer"].as_u64().unwrap_or_else(|| panic!("no new layer: {r}")));
        assert_eq!(past(&s), steps + 1, "newLayer {new_layer}: one history state");
        assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"), "newLayer {new_layer}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(past(&s), steps, "newLayer {new_layer}: one undo takes the step back");
        assert!(s.active().unwrap().doc.layer(id).is_none(), "newLayer {new_layer}: the fill left with its layer");
        assert_eq!(order(&s), below, "newLayer {new_layer}: only the smart object is left where it was");
        assert_eq!(kind(&s, smart), "Smart Object", "newLayer {new_layer}");
        assert_eq!(pixels(&s, smart), before, "newLayer {new_layer}: undo gives the smart object its pixels back untouched");
        assert_eq!(s.active().unwrap().active_layer, Some(smart), "newLayer {new_layer}: the smart object is active again");
    }
}

/// A second removal of the same smart object: *not measured* in the reference, so the new-layer
/// path keeps doing what it does today, one more "Remove tool edits N" layer above it.
#[test]
fn a_second_stroke_on_the_smart_object_adds_another_edits_layer() {
    let mut s = smart_session();
    let bg = order(&s)[0];
    let smart = s.active().unwrap().active_layer.unwrap();
    let first = LayerId(remove_stroke(&mut s, false)["layer"].as_u64().unwrap());
    // The edits layer became the active one, so the next stroke targets the smart object again.
    s.execute("layer.select", json!({"layer": smart.0})).unwrap();
    let r = remove_stroke(&mut s, false);
    let second = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(name(&s, second), "Remove tool edits 1", "the taken name is numbered, as with Create new layer");
    assert_eq!(order(&s), vec![bg, smart, second, first], "the new layer sits directly above the smart object");
    assert_eq!(kind(&s, smart), "Smart Object", "the smart object is still untouched");
}

/// A locked Smart Object keeps today's refusal: no edits layer and no stroke.
#[test]
fn a_locked_smart_object_still_refuses_the_stroke() {
    let mut s = smart_session();
    let stack = order(&s);
    let (bg, smart) = (stack[0], stack[1]);
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    let steps = past(&s);
    let e = s.execute("paint.remove", json!({"points": STROKE, "size": 16})).unwrap_err().to_string();
    assert!(e.contains("locked"), "{e}");
    assert_eq!(past(&s), steps, "nothing was added to the history");
    assert_eq!(order(&s), vec![bg, smart], "no layer was added");
    assert_eq!(kind(&s, smart), "Smart Object");
}

/// A stroke on the smart object's layer mask keeps today's path: it fills the mask in place and
/// adds no layer.
#[test]
fn a_stroke_on_the_layer_mask_of_a_smart_object_adds_no_layer() {
    let mut s = smart_session();
    let smart = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let below = order(&s);
    let r = s.execute("paint.remove", json!({"points": STROKE, "size": 16, "target": "mask"})).unwrap();
    assert!(r.get("layer").is_none(), "a mask has no above: {r}");
    assert_eq!(order(&s), below, "no new layer");
    assert_eq!(kind(&s, smart), "Smart Object");
}

/// A pixel layer with Create new layer off keeps today's in-place fill and adds no layer.
#[test]
fn a_stroke_on_a_pixel_layer_without_create_new_layer_still_fills_in_place() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 96, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    paint_square(&mut s);
    let layer = s.active().unwrap().active_layer.unwrap();
    let before = order(&s);
    let r = remove_stroke(&mut s, false);
    assert!(r.get("layer").is_none(), "{r}");
    assert_eq!(order(&s), before, "no new layer");
    assert_eq!(kind(&s, layer), "Pixel");
    assert!((s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(46, 30)[0] - 0.5).abs() < 0.03, "filled in place");
}
