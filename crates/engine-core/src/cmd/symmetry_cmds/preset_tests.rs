use super::*;

fn session(depth: u8, mode: &str) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width":81,"height":61,"depth":depth,"mode":mode,"background":"white"})).unwrap();
    s
}
#[test]
fn presets_validate_atomically_and_leave_document_paths_and_history_untouched() {
    let mut s = session(8, "rgb");
    s.execute("path.set", json!({"name":"work","path":{"subpaths":[{"knots":[[2,3],[9,15]]}]}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let revision = s.active().unwrap().revision;
    for mode in SymmetryMode::ALL {
        s.execute("paint.setSymmetry", json!({"mode":mode.id()})).unwrap();
        let p = s.active().unwrap().symmetry.as_ref().unwrap().preset().unwrap();
        assert_eq!(p.center, [40.5, 30.5]);
        assert_eq!(p.rotation, 0.0);
        assert_eq!(crate::inspect::document(s.active().unwrap())["symmetry"]["mode"], mode.id());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!((s.active().unwrap().history.past_len(), s.active().unwrap().revision), (history, revision));
    }
    s.execute("paint.setSymmetry", json!({"mode":"dual","center":[17,29],"rotation":45})).unwrap();
    let original = s.active().unwrap().symmetry.clone();
    for bad in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"mode":"radial"}),
        json!({"mode":7}),
        json!({"mode":"dual","center":[1]}),
        json!({"mode":"dual","center":[1,2,3]}),
        json!({"mode":"dual","center":null}),
        json!({"mode":"dual","center":["x",2]}),
        json!({"mode":"dual","center":[1e8,0]}),
        json!({"mode":"dual","rotation":"x"}),
        json!({"mode":"dual","rotation":null}),
    ] {
        assert!(s.execute("paint.setSymmetry", bad).is_err());
        assert_eq!(s.active().unwrap().symmetry, original);
        assert_eq!(s.active().unwrap().doc, before);
    }
    s.execute("paint.setSymmetry", json!({"mode":"dual"})).unwrap();
    assert_eq!(s.active().unwrap().symmetry.as_ref().unwrap().preset().unwrap().center, [40.5, 30.5]);
    s.execute("paint.symmetryFromPath", json!({"name":"work"})).unwrap();
    assert_eq!(s.active().unwrap().symmetry.as_ref().unwrap().path_source(), Some("work"));
    s.execute("paint.symmetryDisable", json!({})).unwrap();
    assert!(crate::inspect::document(s.active().unwrap())["symmetry"].is_null());
    s.execute("paint.setSymmetry", json!({"mode":"vertical"})).unwrap();
    s.execute("file.new", json!({"width":10,"height":20})).unwrap();
    assert!(s.active().unwrap().symmetry.is_none());
    assert!(s.set_active(0));
    assert!(s.active().unwrap().symmetry.is_some());
    let mut empty = crate::session();
    assert!(!empty.is_enabled("paint.setSymmetry"));
    assert!(empty.execute("paint.setSymmetry", json!({"mode":"vertical"})).is_err());
}
