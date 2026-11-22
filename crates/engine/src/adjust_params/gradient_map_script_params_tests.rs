use super::tests::{colourful, pixels};
use super::*;

const EDIT: &str = "image.adjustments.gradientMap";
const CREATE: &str = "layer.newAdjustmentLayer.gradientMap";

#[test]
fn gradient_map_single_stop_colors_only_the_selection_and_undoes_in_one_step() {
    for depth in [8, 16, 32] {
        for params in [
            json!({"stops": [[-1, "#00ff00"]]}),
            json!({"source": {"stops": [{"location": 2, "color": [0, 1, 0]}], "midpoints": [], "smoothness": 1, "method": "perceptual"}, "reverse": true}),
        ] {
            let mut s = colourful("rgb", depth);
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
            let before = pixels(&s);
            let steps = s.active().unwrap().history.past_len();
            s.execute(EDIT, params).unwrap();
            let after = pixels(&s);
            for i in [0, 2] {
                assert_eq!(&after[i][..3], &[0.0, 1.0, 0.0], "{depth}-bit selected pixel {i}");
                assert_eq!(after[i][3], before[i][3]);
            }
            for i in [1, 3] {
                assert_eq!(after[i], before[i], "{depth}-bit unselected pixel {i}");
            }
            let history = &s.active().unwrap().history;
            assert_eq!(history.past_len(), steps + 1);
            assert_eq!(history.undo_label(), Some("Gradient Map"));
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(pixels(&s), before);
        }
    }
}

#[test]
fn gradient_map_script_unknown_method_uses_classic() {
    let mut params = json!({"source": {"stops": [[-2, "#102030"], [2, "#ffcc33"]], "midpoints": [0.3], "smoothness": 0.8, "method": "classic"}});
    let classic = from_params("gradientMap", &params, None, ColorMode::Rgb).unwrap();
    params["source"].as_object_mut().unwrap().remove("method");
    assert_eq!(from_params("gradientMap", &params, None, ColorMode::Rgb).unwrap(), classic);
    for method in [json!("bogus"), json!(""), json!(42), Value::Null] {
        params["source"]["method"] = method;
        assert_eq!(from_params("gradientMap", &params, None, ColorMode::Rgb).unwrap(), classic);
    }
    params["source"]["method"] = json!("LINEAR");
    let linear = from_params("gradientMap", &params, None, ColorMode::Rgb).unwrap();
    assert_ne!(linear, classic);
    assert_eq!(to_params(&linear)["source"]["method"], "linear");
}

#[test]
fn gradient_map_script_single_stop_is_editable_and_survives_a_psd_round_trip() {
    for params in [json!({"stops": [[0.4, "#00ff00"]]}), json!({"source": {"stops": [[0.4, "#00ff00"]], "method": "bogus"}})] {
        let mut s = colourful("rgb", 8);
        s.execute(CREATE, params.clone()).unwrap();
        s.execute("layer.setAdjustment", json!({"reverse": true})).unwrap();
        let state = s.active().unwrap();
        let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Adjustment(adjustment) = &layer.content else { panic!("expected adjustment layer") };
        let actual = to_params(adjustment);
        assert_eq!(actual["stops"], json!([[0.0, "#00ff00"], [1.0, "#00ff00"]]));
        assert_eq!(actual["reverse"], true);
        if params.get("source").is_some() {
            assert_eq!(actual["source"]["method"], "classic");
            assert_eq!(actual["source"]["stops"], actual["stops"]);
            assert_eq!(actual["source"]["midpoints"], json!([0.5]));
        }
        let psd = photocraft_io::document_to_psd(&state.doc).unwrap();
        let (reopened, _) = photocraft_io::psd_to_document(&psd);
        let reopened = reopened
            .layers
            .iter()
            .find_map(|layer| match &layer.content {
                photocraft_doc::LayerContent::Adjustment(adjustment) => Some(to_params(adjustment)),
                _ => None,
            })
            .unwrap();
        assert_eq!(reopened["stops"], actual["stops"]);
        assert_eq!(reopened["reverse"], true);
    }
}

#[test]
fn gradient_map_script_rejects_invalid_stop_counts_and_values_without_an_edit() {
    let mut s = colourful("rgb", 8);
    let before = pixels(&s);
    let steps = s.active().unwrap().history.past_len();
    for stops in [json!("not a list"), json!([]), json!(vec![json!([0, "#00ff00"]); 65]), json!([[0, "bad color"]]), json!([["invalid location", "#00ff00"]])] {
        for command in [EDIT, CREATE] {
            for params in [json!({"stops": stops}), json!({"source": {"stops": stops}})] {
                assert!(matches!(s.execute(command, params.clone()), Err(EngineError::BadParams { .. })), "{command}: {params}");
            }
        }
    }
    assert!(matches!(s.execute(EDIT, json!({"source": {"stops": [[0, "#00ff00"]], "midpoints": [0.5]}})), Err(EngineError::BadParams { .. })));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert_eq!(pixels(&s), before);
}

#[test]
fn gradient_map_script_accepts_the_existing_maximum_stop_count() {
    let stops = (0..64).map(|i| json!([f64::from(i) / 63.0, "#00ff00"])).collect::<Vec<_>>();
    for params in [json!({"stops": stops}), json!({"source": {"stops": stops}})] {
        let actual = from_params("gradientMap", &params, None, ColorMode::Rgb).unwrap();
        assert_eq!(to_params(&actual)["stops"].as_array().unwrap().len(), 64);
    }
}
