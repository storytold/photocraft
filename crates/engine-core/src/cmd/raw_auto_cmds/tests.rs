use super::{EngineError, ID, json};

fn fill_pixel_layer(session: &mut crate::Session) {
    session
        .edit("seed dim pixels", |doc, active| {
            let area = doc.bounds();
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surface.fill_rect(area, &[0.33, 0.28, 0.26, 1.0]);
            Ok(())
        })
        .unwrap();
}

#[test]
fn estimates_are_available_at_each_document_depth_without_editing_pixels() {
    for bits in [8, 16, 32] {
        let mut session = crate::session();
        session.execute("file.new", json!({"width": 96, "height": 96, "depth": bits})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        fill_pixel_layer(&mut session);
        let rev = session.active().unwrap().revision;
        let result = session.execute(ID, json!({"panel": "light"})).unwrap();
        assert!(result["settings"]["exposure"].as_f64().is_some_and(|v| v > 0.0));
        assert_eq!(result["settings"]["temperature"].as_f64(), Some(0.0));
        assert_eq!(session.active().unwrap().revision, rev);
    }
}

#[test]
fn full_estimate_returns_replayable_curve_settings_without_editing_the_layer() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 96, "height": 96, "depth": 16})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session
        .edit("grey ramp", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for x in 0..96 {
                let v = 0.19 + 0.30 * x as f32 / 95.0;
                surf.fill_rect(photocraft_geom::Rect::new(x, 0, x + 1, 96), &[v, v, v, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    let rev = session.active().unwrap().revision;
    let result = session.execute(ID, json!({"panel": "all"})).unwrap();
    assert_eq!(session.execute(ID, serde_json::Value::Null).unwrap(), result);
    assert_eq!(result["settings"]["pointCurve"].as_array().unwrap().len(), 5);
    assert_eq!(session.active().unwrap().revision, rev);
    session.execute("filter.cameraRaw", result["settings"].clone()).unwrap();
    assert_ne!(session.active().unwrap().revision, rev);
}

#[test]
fn rejects_unknown_panels_and_wrong_parameter_shapes() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    for params in [json!({"panel": []}), json!({"panel": "blur"}), json!({"settings": {"texture": "max"}}), json!({"other": true})] {
        assert!(matches!(session.execute(ID, params), Err(EngineError::BadParams { .. })));
    }
}

#[test]
fn estimate_can_name_a_pixel_layer_while_a_group_is_active() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    fill_pixel_layer(&mut session);
    let layer = session.active().unwrap().active_layer.unwrap();
    session.execute("layer.new.group", json!({})).unwrap();
    let group = session.active().unwrap().active_layer.unwrap();
    assert_ne!(group, layer);
    assert!(!session.is_enabled(ID));

    let result = session.execute(ID, json!({"panel": "light", "layer": layer.0})).unwrap();
    assert_eq!(result["layer"], json!(layer.0));
    assert_eq!(session.active().unwrap().active_layer, Some(group));
}
