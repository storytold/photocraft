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
    let new = PerspectivePlanes { planes, focal, size: Some([d.size.width, d.size.height]) };
    if new == d.perspective || (new.planes.is_empty() && d.perspective.is_empty()) {
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
    fn planes_go_stale_when_the_canvas_changes() {
        let mut s = session();
        vp(&mut s, json!({}));
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["stale"], false);
        s.execute("image.canvasSize", json!({"width": 500, "height": 300, "anchor": "topLeft"})).unwrap();
        let q = s.execute(PLANES, json!({})).unwrap();
        assert_eq!(q["stale"], true);
        assert_eq!(q["planes"], json!([]));
        assert!(!s.active().unwrap().doc.perspective.valid_for(500, 300));
        s.undo();
        assert_eq!(s.execute(PLANES, json!({})).unwrap()["stale"], false);
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
