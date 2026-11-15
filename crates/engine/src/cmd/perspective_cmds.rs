//! The perspective planes kept with the document ([`photocraft_doc::PerspectivePlanes`]): written
//! by `filter.vanishingPoint`, read back here and cleared here.

use serde_json::{Value, json};

use photocraft_doc::PerspectivePlanes;

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const CLEAR: &str = "perspective.clearPlanes";
pub const PLANES: &str = "perspective.planes";
pub const SET: &str = "perspective.setPlanes";
/// Most planes the document keeps (as Vanishing Point's own cap).
pub const MAX_PLANES: usize = 64;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_planes(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.doc.perspective.is_empty()) { Ok(()) } else { Err("the document has no perspective planes".into()) }
}

fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    let n = s.active().ok_or(EngineError::NoDocument)?.doc.perspective.planes.len();
    s.edit("Clear Perspective Planes", |doc, _| {
        doc.perspective = Default::default();
        Ok(())
    })?;
    Ok(json!({"cleared": n}))
}

fn planes(s: &mut Session, _p: &Value) -> Result<Value> {
    let d = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let pp = &d.perspective;
    // Planes saved for another canvas size are stale: reported, not returned as usable.
    let stale = !pp.is_empty() && !pp.valid_for(d.size.width, d.size.height);
    let planes: &[[[f64; 2]; 4]] = if stale { &[] } else { &pp.planes };
    Ok(json!({"planes": planes, "focal": if stale { None } else { pp.focal }, "stale": stale}))
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: SET.into(), msg: msg.into() }
}

fn point(v: &Value) -> Option<[f64; 2]> {
    let a = v.as_array().filter(|a| a.len() == 2)?;
    let p = [a.first()?.as_f64()?, a.get(1)?.as_f64()?];
    p.iter().all(|x| x.is_finite() && x.abs() < 1e7).then_some(p)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let list = p.get("planes").and_then(Value::as_array).ok_or_else(|| bad("pass `planes`: [[[x,y]×4], …]"))?;
    if list.len() > MAX_PLANES {
        return Err(bad(format!("at most {MAX_PLANES} planes")));
    }
    let mut planes = Vec::with_capacity(list.len());
    for (i, pl) in list.iter().enumerate() {
        let a = pl.as_array().filter(|a| a.len() == 4).ok_or_else(|| bad(format!("plane {i}: four [x,y] corners")))?;
        let mut c = [[0.0; 2]; 4];
        for (slot, v) in c.iter_mut().zip(a) {
            *slot = point(v).ok_or_else(|| bad(format!("plane {i}: every corner is a finite [x,y] under 1e7")))?;
        }
        planes.push(c);
    }
    let d = &s.active().ok_or(EngineError::NoDocument)?.doc;
    // Absent keeps the saved focal length (the dialog never edits it); null clears it.
    let focal = match p.get("focal") {
        None => d.perspective.focal,
        Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().filter(|f| f.is_finite() && *f > 0.0).ok_or_else(|| bad("`focal` must be a number above 0, or null"))?),
    };
    // Without planes there is no canvas size to record: the candidate keeps the saved one, so only
    // the planes and the focal length decide whether anything changes.
    let size = if planes.is_empty() { d.perspective.size } else { Some([d.size.width, d.size.height]) };
    let new = PerspectivePlanes { planes, focal, size };
    if new == d.perspective {
        return Ok(json!({"changed": false}));
    }
    s.edit("Set Perspective Planes", |doc, _| {
        doc.perspective = new;
        Ok(())
    })?;
    Ok(json!({"changed": true}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CLEAR,
            label: "Clear Perspective Planes",
            menu: &[],
            shortcut: None,
            params: r##"{} → {"cleared": n}: forgets the planes saved with the document"##,
            enabled: has_planes,
            run: clear,
            journal: true,
        },
        CommandSpec {
            id: SET,
            label: "Set Perspective Planes",
            menu: &[],
            shortcut: None,
            params: r##"{"planes":[[[x,y]×4]] (at most 64, clockwise from the top-left),"focal":px>0|null (absent keeps the saved one)} → {"changed": bool}"##,
            enabled: has_doc,
            run: set,
            journal: true,
        },
        CommandSpec {
            id: PLANES,
            label: "Perspective Planes",
            menu: &[],
            shortcut: None,
            params: r##"{} → {"planes": [[[x,y]×4]], "focal": px|null, "stale": bool (saved for another canvas size: planes is then empty)}"##,
            enabled: has_doc,
            run: planes,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vp_cmds::VP;

    const FLOOR: [[f64; 2]; 4] = [[120.0, 120.0], [280.0, 120.0], [360.0, 280.0], [40.0, 280.0]];

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    }

    fn vp(s: &mut Session, extra: Value) -> Value {
        let mut p = json!({"planes": [{"corners": FLOOR}, {"from": 0, "edge": "top", "angle": 90}]});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute(VP, p).unwrap()
    }

    #[test]
    fn fail_gracefully_without_a_document_or_with_wrong_params() {
        let mut s = Session::new();
        assert!(s.execute(CLEAR, json!({})).is_err());
        assert!(s.execute(PLANES, json!({})).is_err());
        let mut s = session();
        // Nothing to clear yet.
        assert!(s.execute(CLEAR, json!({})).is_err());
        assert!(s.execute(CLEAR, json!("x")).is_err());
        // Params are ignored by the query: whatever their type, it must not panic.
        let _ = s.execute(PLANES, json!([1, 2]));
        assert_eq!(s.execute(PLANES, json!({})).unwrap(), json!({"planes": [], "focal": null, "stale": false}));
        // A wrong-typed `keepPlanes` is ignored (the default stores) rather than panicking.
        vp(&mut s, json!({"keepPlanes": "no"}));
        assert_eq!(s.active().unwrap().doc.perspective.planes.len(), 2);
    }

    #[test]
    fn vanishing_point_stores_planes_in_one_undoable_step() {
        let mut s = session();
        let steps = s.active().unwrap().history.past_len();
        vp(&mut s, json!({"focalLength": 500}));
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        let pp = s.active().unwrap().doc.perspective.clone();
        assert_eq!(pp.planes.len(), 2, "the torn-off plane is kept too");
        assert_eq!(pp.planes[0], FLOOR);
        assert_eq!(pp.focal, Some(500.0));
        let q = s.execute(PLANES, json!({})).unwrap();
        assert_eq!(q["planes"].as_array().unwrap().len(), 2);
        assert_eq!(q["focal"], json!(500.0));
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "the query records no step");
        s.undo();
        assert!(s.active().unwrap().doc.perspective.is_empty());
        s.redo();
        assert_eq!(s.active().unwrap().doc.perspective, pp);
    }

    #[test]
    fn no_focal_length_stores_none_and_keep_planes_false_stores_nothing() {
        let mut s = session();
        vp(&mut s, json!({}));
        assert_eq!(s.active().unwrap().doc.perspective.focal, None);
        s.undo();
        vp(&mut s, json!({"keepPlanes": false}));
        assert!(s.active().unwrap().doc.perspective.is_empty());
    }

    #[test]
    fn set_planes_validates_skips_equal_values_and_undoes() {
        let mut s = session();
        assert!(Session::new().execute(SET, json!({"planes": []})).is_err());
        let long: Vec<Value> = vec![json!(FLOOR); MAX_PLANES + 1];
        for bad in [
            json!({}),
            json!({"planes": "x"}),
            json!({"planes": [[[0, 0], [1, 0], [1, 1]]]}),
            json!({"planes": [[[0, 0], [1, 0], [1, 1], ["a", 1]]]}),
            json!({"planes": [[[0, 0], [1, 0], [1, 1], [1e9, 1]]]}),
            json!({"planes": [FLOOR], "focal": "wide"}),
            json!({"planes": [FLOOR], "focal": -3}),
            json!({"planes": long}),
        ] {
            assert!(s.execute(SET, bad.clone()).is_err(), "{bad}");
        }
        assert!(s.active().unwrap().doc.perspective.is_empty());
        let steps = s.active().unwrap().history.past_len();
        assert_eq!(s.execute(SET, json!({"planes": []})).unwrap()["changed"], false);
        assert_eq!(s.execute(SET, json!({"planes": [FLOOR], "focal": 300})).unwrap()["changed"], true);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        let pp = s.active().unwrap().doc.perspective.clone();
        assert_eq!((pp.planes.len(), pp.focal, pp.size), (1, Some(300.0), Some([400, 300])));
        assert_eq!(s.execute(SET, json!({"planes": [FLOOR], "focal": 300})).unwrap()["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        s.undo();
        assert!(s.active().unwrap().doc.perspective.is_empty());
    }

    #[test]
    fn a_focal_length_with_no_planes_is_set_and_cleared() {
        let mut s = session();
        let steps = s.active().unwrap().history.past_len();
        assert_eq!(s.execute(SET, json!({"planes": [], "focal": 500})).unwrap()["changed"], true);
        assert_eq!(s.active().unwrap().doc.perspective.focal, Some(500.0));
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["focal"], json!(500.0));
        // The same again, and no planes with the focal kept, change nothing.
        assert_eq!(s.execute(SET, json!({"planes": [], "focal": 500})).unwrap()["changed"], false);
        assert_eq!(s.execute(SET, json!({"planes": []})).unwrap()["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        // null clears the retained focal length.
        assert_eq!(s.execute(SET, json!({"planes": [], "focal": null})).unwrap()["changed"], true);
        assert_eq!(s.active().unwrap().doc.perspective.focal, None);
        assert_eq!(s.execute(SET, json!({"planes": [], "focal": null})).unwrap()["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 2);
        s.undo();
        assert_eq!(s.active().unwrap().doc.perspective.focal, Some(500.0));
    }

    fn saved(s: &Session) -> PerspectivePlanes {
        s.active().unwrap().doc.perspective.clone()
    }

    /// `planes` with every corner moved by `(dx, dy)`.
    fn shifted(planes: &[[[f64; 2]; 4]], dx: f64, dy: f64) -> Vec<[[f64; 2]; 4]> {
        planes.iter().map(|pl| pl.map(|c| [c[0] + dx, c[1] + dy])).collect()
    }

    #[test]
    fn a_size_change_with_no_map_drops_the_planes_and_undo_restores_them() {
        let mut s = session();
        vp(&mut s, json!({}));
        let original = saved(&s);
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["stale"], false);
        // Nothing maps this resize, so the planes cannot follow it.
        s.edit("resize", |doc, _| {
            doc.size = photocraft_geom::Size { width: 500, height: 300 };
            Ok(())
        })
        .unwrap();
        assert_eq!(saved(&s), PerspectivePlanes::default());
        assert!(!saved(&s).valid_for(500, 300));
        s.undo();
        assert_eq!(saved(&s), original);
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["stale"], false);
    }

    #[test]
    fn a_crop_moves_the_planes_with_the_picture() {
        let mut s = session();
        vp(&mut s, json!({}));
        let original = saved(&s);
        assert_eq!(original.planes.len(), 2);
        s.execute("image.crop", json!({"x": 50, "y": 40, "width": 300, "height": 200})).unwrap();
        let after = saved(&s);
        assert_eq!(after.planes, shifted(&original.planes, -50.0, -40.0));
        assert_eq!(after.size, Some([300, 200]));
        assert!(after.valid_for(300, 200));
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["stale"], false);
        // One undo step brings the original planes back, with their canvas.
        s.undo();
        assert_eq!(saved(&s), original);
        assert!(saved(&s).valid_for(400, 300));
    }

    #[test]
    fn a_crop_then_a_canvas_change_back_to_the_old_size_does_not_revive_the_old_corners() {
        let mut s = session();
        vp(&mut s, json!({}));
        let original = saved(&s);
        s.execute("image.crop", json!({"x": 50, "y": 40, "width": 300, "height": 200})).unwrap();
        s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "topLeft"})).unwrap();
        let after = saved(&s);
        assert_eq!(after.size, Some([400, 300]));
        assert_ne!(after.planes, original.planes, "the planes stay on the pixels they were drawn on");
        assert_eq!(after.planes, shifted(&original.planes, -50.0, -40.0));
        // A centred Canvas Size moves them by the margin it adds.
        s.execute("image.canvasSize", json!({"width": 500, "height": 300, "anchor": "center"})).unwrap();
        assert_eq!(saved(&s).planes, shifted(&original.planes, -50.0 + 50.0, -40.0));
        assert!(saved(&s).valid_for(500, 300));
    }

    #[test]
    fn image_size_scales_the_planes_and_the_focal_length_and_a_flip_keeps_them_clockwise() {
        let mut s = session();
        s.execute(SET, json!({"planes": [FLOOR], "focal": 500})).unwrap();
        s.execute("image.imageSize", json!({"width": 200, "height": 150})).unwrap();
        let half = saved(&s);
        assert_eq!(half.planes[0], FLOOR.map(|c| [c[0] / 2.0, c[1] / 2.0]));
        assert_eq!(half.size, Some([200, 150]));
        assert!((half.focal.unwrap() - 250.0).abs() < 1e-9, "{:?}", half.focal);
        // Mirrored across the canvas: x -> 200 - x, and the order reverses to stay clockwise.
        s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
        let flipped = saved(&s);
        let c = half.planes[0];
        let mirror = |p: [f64; 2]| [200.0 - p[0], p[1]];
        assert_eq!(flipped.planes[0], [mirror(c[0]), mirror(c[3]), mirror(c[2]), mirror(c[1])]);
        assert!(flipped.valid_for(200, 150));
        assert_eq!(flipped.focal, half.focal);
    }

    #[test]
    fn a_focal_length_with_no_planes_follows_a_resize_too() {
        let mut s = session();
        s.execute(SET, json!({"planes": [], "focal": 500})).unwrap();
        s.execute("image.imageSize", json!({"width": 200, "height": 150})).unwrap();
        assert!((saved(&s).focal.unwrap() - 250.0).abs() < 1e-9);
        assert!(saved(&s).planes.is_empty());
    }

    #[test]
    fn clear_planes_is_one_undo_step() {
        let mut s = session();
        vp(&mut s, json!({}));
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute(CLEAR, json!({})).unwrap();
        assert_eq!(r["cleared"], 2);
        assert!(s.active().unwrap().doc.perspective.is_empty());
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        s.undo();
        assert_eq!(s.active().unwrap().doc.perspective.planes.len(), 2);
    }
}
