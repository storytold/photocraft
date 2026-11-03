use super::*;
use photocraft_geom::Rect;

fn cutout(depth: u32, mode: &str) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":24,"depth":depth,"mode":mode})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Subject"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#cc4422"})).unwrap();
    s.execute("select.rect", json!({"x":8,"y":6,"width":16,"height":12})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":2})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    (s, id)
}

#[test]
fn fills_preserve_subject_selection_depth_and_undo_in_one_step() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            for kind in ["solid", "gradient"] {
                let (mut s, id) = cutout(depth, mode);
                let before = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                let r = s.execute(COMMAND, json!({"kind":kind,"color":"#2266aa","to":"#eeeeee"})).unwrap();
                let st = s.active().unwrap();
                assert_eq!(st.doc.layer(id), before.layer(id));
                assert_eq!(st.doc.selection, before.selection);
                assert_eq!(st.active_layer, Some(id));
                assert_eq!(st.history.past_len(), history + 1);
                let fill_id = LayerId(r["layer"].as_u64().unwrap());
                let path = st.doc.path_of(fill_id).unwrap();
                let target_path = st.doc.path_of(id).unwrap();
                assert_eq!(path.last().unwrap() + 1, *target_path.last().unwrap());
                assert!(st.doc.layer(fill_id).unwrap().mask.is_none());
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc.layers, before.layers);
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.layer(id), before.layer(id));
            }
        }
    }
}

#[test]
fn solid_background_is_visible_outside_the_subject_and_does_not_cover_it() {
    let (mut s, id) = cutout(8, "rgb");
    // The blank initial Background would obscure our new fill, so remove it.
    let first = s.active().unwrap().doc.layers[0].id;
    s.edit("remove blank", |doc, _| {
        doc.remove(first);
        Ok(())
    })
    .unwrap();
    s.execute(COMMAND, json!({"color":"#2266aa"})).unwrap();
    let image = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(0, 0, 32, 24));
    let background = image.px[0];
    let foreground = image.px[12 * 32 + 16];
    for (actual, expected) in background.into_iter().zip([34. / 255., 102. / 255., 170. / 255., 1.]) {
        assert!((actual - expected).abs() < 0.02);
    }
    assert!(foreground[0] > foreground[2]);
    assert_eq!(s.active().unwrap().active_layer, Some(id));
}

#[test]
fn image_background_moves_existing_layer_and_restores_its_order_on_undo() {
    let (mut s, id) = cutout(16, "rgb");
    let r = s.execute("layer.new.layer", json!({"name":"Image background"})).unwrap();
    let source = LayerId(r["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"contents":"color","color":"#4477aa"})).unwrap();
    s.select_layer(id).unwrap();
    let before = s.active().unwrap().doc.clone();
    let r = s.execute(COMMAND, json!({"kind":"image","sourceLayer":source.0})).unwrap();
    assert_eq!(r["layer"], source.0);
    assert_eq!(s.active().unwrap().doc.layer(source), before.layer(source));
    assert_eq!(s.active().unwrap().doc.layer(id), before.layer(id));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}

#[test]
fn invalid_input_and_missing_mask_leave_document_and_history_unchanged() {
    let (mut s, id) = cutout(8, "rgb");
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for p in [
        Value::Null,
        json!([]),
        json!({"kind":1}),
        json!({"kind":"unknown"}),
        json!({"layer":-1}),
        json!({"color":"#nope"}),
        json!({"document":0}),
        json!({"kind":"gradient","angle":"bad"}),
        json!({"kind":"image"}),
        json!({"kind":"image","sourceLayer":id.0}),
        json!({"kind":"image","sourceLayer":u64::MAX}),
    ] {
        assert!(s.execute(COMMAND, p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc.layers, before.layers);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    let mut empty = Session::new();
    assert!(empty.execute(COMMAND, json!({})).is_err());
    empty.execute("file.new", json!({"width":8,"height":8})).unwrap();
    assert!(!empty.is_enabled(COMMAND));
    assert!(empty.execute(COMMAND, json!({})).is_err());
}

#[test]
fn saved_cutouts_keep_their_mask_and_editable_background_fill() {
    for depth in [8, 16, 32] {
        let (mut s, subject) = cutout(depth, "rgb");
        let layer = s.execute(COMMAND, json!({"kind":"gradient","color":"#224466","to":"#eeeeee"})).unwrap()["layer"].as_u64().unwrap();
        let before = &s.active().unwrap().doc;
        let (bytes, _) = photocraft_format::PcraftWriter::new().save_zip(before, &Default::default()).unwrap();
        let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(loaded.layer(subject), before.layer(subject));
        assert_eq!(loaded.layer(LayerId(layer)), before.layer(LayerId(layer)));
        assert_eq!(loaded.depth, before.depth);
    }
}

#[test]
fn moving_an_image_clipping_base_or_locked_layer_is_rejected() {
    let (mut s, subject) = cutout(8, "rgb");
    let source = LayerId(s.execute("layer.new.layer", json!({"name":"Image"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.new.layer", json!({"name":"Clipped"})).unwrap();
    s.execute("layer.createClippingMask", json!({})).unwrap();
    s.select_layer(subject).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(COMMAND, json!({"kind":"image","sourceLayer":source.0})).is_err());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    s.execute("layer.releaseClippingMask", json!({"layer":before.layers.last().unwrap().id.0})).unwrap();
    s.edit("lock source", |doc, _| {
        doc.layer_mut(source).unwrap().locks.position = true;
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(COMMAND, json!({"kind":"image","sourceLayer":source.0})).is_err());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}
