//! #12492: a warp committed on a smart object is one History step named "Free Transform", as in
//! the reference application; on a pixel layer, a selection on the Background and a type layer
//! the step stays "Warp".

use serde_json::{Value, json};

use crate::Session;

/// An 80 × 60 document with a painted pixel layer, the active one.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// Runs `cmd` and returns the name of the one History step it added.
fn step(s: &mut Session, cmd: &str, p: Value) -> String {
    let before = history(s);
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    let after = history(s);
    assert_eq!(after.len(), before.len() + 1, "{cmd}: one History step: {after:?}");
    after.last().cloned().unwrap_or_default()
}

const CYL: [f64; 7] = [10.0, 40.0, 60.0, 10.0, 0.3, 0.3, 0.5];

#[test]
fn a_warp_on_a_smart_object_is_free_transform() {
    let warps = [
        ("edit.transform.warp", json!({"style": "arc", "bend": 30})),
        ("layer.smartObjects.warp", json!({"style": "arc", "bend": 30})),
        ("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})),
    ];
    for (cmd, p) in warps {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        assert_eq!(step(&mut s, cmd, p.clone()), "Free Transform", "{cmd} {p}");
        assert!(s.undo(), "{cmd} {p}");
        assert!(*s.active().unwrap().doc == *doc, "{cmd} {p}: one undo restores the layer");
    }
}

#[test]
fn a_warp_on_pixels_a_selection_or_type_stays_warp() {
    let mut s = session();
    assert_eq!(step(&mut s, "edit.transform.warp", json!({"style": "arc", "bend": 30})), "Warp", "pixel layer");

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    assert_eq!(step(&mut s, "edit.transform.warp", json!({"style": "arc", "bend": 30})), "Warp", "Background with a selection");

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    s.execute("type.create", json!({"x": 20, "y": 60, "text": "Warp", "size": 30})).unwrap();
    assert_eq!(step(&mut s, "edit.transform.warp", json!({"style": "arc", "bend": 30})), "Warp", "type layer");
}
