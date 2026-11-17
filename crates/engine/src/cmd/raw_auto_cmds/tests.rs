use super::{EngineError, ID, Session, json};

#[test]
fn estimates_are_available_at_each_document_depth_without_editing_pixels() {
    for bits in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 96, "height": 96, "depth": bits})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        session.execute("edit.fill", json!({"color": "#534743"})).unwrap();
        let rev = session.active().unwrap().revision;
        let result = session.execute(ID, json!({"panel": "light"})).unwrap();
        assert!(result["settings"]["exposure"].as_f64().is_some_and(|v| v > 0.0));
        assert_eq!(result["settings"]["temperature"].as_f64(), Some(0.0));
        assert_eq!(session.active().unwrap().revision, rev);
    }
}

#[test]
fn rejects_unknown_panels_and_wrong_parameter_shapes() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    for params in [json!({"panel": []}), json!({"panel": "blur"}), json!({"settings": {"texture": "max"}}), json!({"other": true})] {
        assert!(matches!(session.execute(ID, params), Err(EngineError::BadParams { .. })));
    }
}
