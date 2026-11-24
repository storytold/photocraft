//! Tests that run paint commands, moved from the engine-core `vp_cmds` tests: 4 test(s).

use photocraft_algo::vanishing::VpPlane;
use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use photocraft_engine::vp_cmds::*;

#[test]
fn planes_paste_and_clone() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({"name": "floor"})).unwrap();
    s.execute("edit.fill", json!({"color": "#404040"})).unwrap();
    let floor = s.active().unwrap().active_layer.unwrap();
    // A trapezoid "floor" receding to the top.
    let planes = json!([{"corners": [[120, 120], [280, 120], [360, 280], [40, 280]]}, {"from": 0, "edge": "top", "angle": 90}]);
    s.execute("layer.new.layer", json!({"name": "sticker"})).unwrap();
    s.edit("sticker", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 20), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let sticker = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": floor.0})).unwrap();
    let r = s.execute(VP, json!({"planes": planes, "paste": [{"plane": 0, "layer": sticker.0, "at": [0.3, 0.4], "width": 0.4}]})).unwrap();
    assert_eq!(r["planes"].as_array().unwrap().len(), 2);
    assert!(r["focal"].as_f64().unwrap() > 50.0, "{r}");
    let l = s.active().unwrap().doc.layer(floor).unwrap().surface().unwrap().clone();
    // The paste covers the plane's middle and not its far corner.
    let h = VpPlane { corners: [[120.0, 120.0], [280.0, 120.0], [360.0, 280.0], [40.0, 280.0]] }.homography().unwrap();
    let (x, y) = h.apply(0.5, 0.41);
    assert!(l.rgba(x as i32, y as i32)[0] > 0.9, "{:?}", l.rgba(x as i32, y as i32));
    assert!(l.rgba(130, 125)[0] < 0.5);
    s.undo();
    // Clone: copy the bright stripe painted near the bottom to the top of the floor.
    s.edit("stripe", |doc, _| {
        doc.layer_mut(floor).unwrap().surface_mut().unwrap().fill_rect(Rect::new(180, 250, 220, 265), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": floor.0})).unwrap();
    let r =
        s.execute(VP, json!({"planes": planes, "clone": [{"source": [200, 257], "points": [[200, 160], [205, 160]], "size": 12, "hardness": 80}]})).unwrap();
    assert!(r["dabs"].as_u64().unwrap() >= 1, "{r}");
    let l = s.active().unwrap().doc.layer(floor).unwrap().surface().unwrap();
    assert!(l.rgba(200, 160)[1] > 0.8, "{:?}", l.rgba(200, 160));
    assert!(s.execute(VP, json!({})).is_err());
    assert!(s.execute(VP, json!({"planes": [{"corners": [[0, 0], [1, 1]]}]})).is_err());
    let r = s.execute(VP, json!({"planes": planes, "newLayer": true})).unwrap();
    assert_ne!(r["layer"].as_u64().unwrap(), floor.0);
}

#[test]
fn new_layer_clone_stays_on_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
    // A red stripe where the first dab samples, so cloning it changes pixels.
    s.edit("stripe", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(140, 80, 160, 120), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
    // The second dab lies left of the canvas and samples the canvas 50 px to its right; the
    // third is far off it (its box joined with the first's once asked for petabytes).
    for points in [json!([[100, 100], [-30, 100]]), json!([[100, 100], [-30, 100], [9_000_000, 9_000_000]])] {
        let r = s.execute(VP, json!({"planes": planes, "newLayer": true, "clone": [{"source": [150, 100], "points": points, "size": 20}]})).unwrap();
        let l = s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().surface().unwrap();
        let b = l.content_bounds();
        assert!(!b.is_empty() && Rect::new(0, 0, 200, 200).contains_rect(&b), "{b:?}");
        assert!(l.rgba(100, 100)[0] > 0.9, "{:?}", l.rgba(100, 100));
        s.undo();
    }
}

/// Without a new layer the strokes paint the layer itself, and only inside the canvas, as in
/// Photoshop: a dab off it painted pixels there (and allocated tiles for every dab, up to
/// 1e7 px away).
#[test]
fn layer_clone_stays_on_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
    // The second dab lies left of the canvas and samples the canvas 50 px to its right; the
    // third is far off it.
    let points = json!([[100, 100], [-30, 100], [9_000_000, 9_000_000]]);
    let r = s.execute(VP, json!({"planes": planes, "clone": [{"source": [150, 100], "points": points, "size": 20}]})).unwrap();
    assert_eq!(r["dabs"].as_u64(), Some(1), "{r}");
    let b = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
    assert!(Rect::new(0, 0, 200, 200).contains_rect(&b), "{b:?}");
}

#[test]
fn layer_param_paints_that_layer_not_the_active_one() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("layer.new.layer", json!({"name": "floor"})).unwrap();
    s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
    s.edit("stripe", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(140, 80, 160, 120), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let floor = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "other"})).unwrap();
    let other = s.active().unwrap().active_layer.unwrap();
    let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
    let clone = json!([{"source": [150, 100], "points": [[100, 100]], "size": 20}]);
    let surf = |s: &Session, id| s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
    let r = s.execute(VP, json!({"planes": planes, "clone": clone, "layer": floor.0})).unwrap();
    assert_eq!(r["layer"].as_u64(), Some(floor.0));
    assert!(surf(&s, floor).rgba(100, 100)[0] > 0.9, "{:?}", surf(&s, floor).rgba(100, 100));
    assert!(surf(&s, other).content_bounds().is_empty(), "the active layer stays untouched");
    s.undo();
    // A new layer goes right above the named layer.
    let r = s.execute(VP, json!({"planes": planes, "clone": clone, "layer": floor.0, "newLayer": true})).unwrap();
    let d = &s.active().unwrap().doc;
    let ids: Vec<u64> = d.layers.iter().map(|l| l.id.0).collect();
    let at = |id: u64| ids.iter().position(|&i| i == id).unwrap();
    assert_eq!(at(r["layer"].as_u64().unwrap()), at(floor.0) + 1, "{ids:?}");
    s.undo();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(VP, json!({"planes": planes, "clone": clone, "layer": 999_999})).is_err());
    assert!(s.execute(VP, json!({"planes": planes, "clone": clone, "layer": 999_999, "newLayer": true})).is_err());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
}

fn paste_sticker(transform: Value) -> Surface {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({"name": "sticker"})).unwrap();
    let sticker = s.active().unwrap().active_layer.unwrap();
    s.edit("sticker", |doc, _| {
        let surf = doc.layer_mut(sticker).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(10, 5, 30, 25), &[1.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(30, 5, 50, 25), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let mut paste = json!({"layer": sticker.0, "at": [0.25, 0.25], "width": 0.4});
    paste.as_object_mut().unwrap().extend(transform.as_object().unwrap().clone());
    let result = s.execute(VP, json!({"planes": [{"corners": [[0, 0], [200, 0], [200, 100], [0, 100]]}], "paste": [paste], "newLayer": true})).unwrap();
    assert_eq!(result["pasted"], 1);
    let id = LayerId(result["layer"].as_u64().unwrap());
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone()
}

#[test]
fn paste_flip_h_swaps_the_stickers_left_and_right() {
    let out = paste_sticker(json!({"flipH": true}));
    assert!(out.rgba(70, 45)[2] > 0.9);
    assert!(out.rgba(110, 45)[0] > 0.9);
}

#[test]
fn paste_angle_turns_the_sticker_in_the_plane_metric() {
    let out = paste_sticker(json!({"angle": 90}));
    assert!(out.rgba(90, 15)[0] > 0.9);
    assert!(out.rgba(90, 75)[2] > 0.9);
    assert_eq!(out.rgba(60, 45)[3], 0.0);
}

/// A green field with a red stripe at x 140..160: cloning from (150, 100) paints red.
fn striped(s: &mut Session) -> LayerId {
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
    s.edit("stripe", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(140, 0, 160, 200), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.active().unwrap().active_layer.unwrap()
}

#[test]
fn the_document_keeps_the_planes() {
    let mut s = Session::new();
    striped(&mut s);
    let planes = json!([{"corners": [[20, 60], [180, 60], [190, 190], [10, 190]]}, {"from": 0, "edge": "top", "angle": 90, "depth": 0.5}]);
    let steps = s.active().unwrap().history.past_len();
    // OK with only planes is one step that keeps them; the same planes again change nothing.
    s.execute(VP, json!({"planes": planes})).unwrap();
    s.execute(VP, json!({"planes": planes})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let kept = s.active().unwrap().doc.vanishing_planes.clone();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].corners, [[20.0, 60.0], [180.0, 60.0], [190.0, 190.0], [10.0, 190.0]]);
    assert!(kept[0].torn.is_none());
    let torn = kept[1].torn.as_ref().unwrap();
    assert_eq!((torn.from, torn.edge.as_str(), torn.angle, torn.depth), (0, "top", 90.0, 0.5));
    s.undo();
    assert!(s.active().unwrap().doc.vanishing_planes.is_empty());
    // A paint run keeps them too.
    s.execute(VP, json!({"planes": planes, "clone": [{"source": [150, 150], "points": [[60, 150]], "size": 10}]})).unwrap();
    assert_eq!(s.active().unwrap().doc.vanishing_planes, kept);
}
