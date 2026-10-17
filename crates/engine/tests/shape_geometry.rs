use photocraft_engine::Session;
use serde_json::json;

#[test]
fn star_indents_and_arrowhead_concavity_edit_and_undo_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":96,"height":64,"depth":depth,"background":"transparent"})).unwrap();
        for (params, edit, probe, expected) in [
            (json!({"kind":"star","rect":[10,10,50,40],"sides":5,"starRatio":0.4}), json!({"smoothIndents":true}), "/live/smooth_indents", json!(true)),
            (
                json!({"kind":"line","from":[10,30],"to":[70,30],"weight":3,"arrowheads":{"start":true,"end":true,"width":12,"length":10}}),
                json!({"arrowheads":{"start":true,"end":true,"width":12,"length":10,"concavity":35}}),
                "/live/arrowheads/concavity",
                json!(35.0),
            ),
        ] {
            s.execute("shape.create", params).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let info = s.execute("shape.edit", edit).unwrap();
            assert_eq!(info.pointer(probe), Some(&expected));
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            let after = s.active().unwrap().doc.clone();
            assert_ne!(before, after);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, after);
            s.execute("shape.edit", json!({"transform":[2,0,0,1,0,0]})).unwrap();
            assert_eq!(s.execute("shape.info", json!({})).unwrap().pointer(probe), Some(&expected));
        }
    }
}

#[test]
fn invalid_geometry_options_fail_without_document_or_history_changes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":48})).unwrap();
    s.execute("shape.create", json!({"kind":"star","rect":[5,5,30,30]})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for params in [json!({"smoothIndents":"true"}), json!({"smoothIndents":null}), json!({"smoothIndents":1})] {
        assert!(s.execute("shape.edit", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    for concavity in [json!(-50.1), json!(50.1), json!(1e100), json!("0"), json!(null)] {
        let params = json!({"kind":"line","from":[0,20],"to":[60,20],"arrowheads":{"end":true,"width":10,"length":20,"concavity":concavity}});
        assert!(s.execute("shape.create", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
