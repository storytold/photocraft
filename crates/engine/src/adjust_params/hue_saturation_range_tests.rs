use super::*;

#[test]
fn master_and_range_out_of_range_values_are_refused_without_changing_doc() {
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": [0.8, 0.3, 0.2]})).unwrap();
    let doc_before = s.active().unwrap().doc.clone();
    let history_before = s.active().unwrap().history.past_len();

    for params in [
        json!({"hue": 181}),
        json!({"hue": -181}),
        json!({"hue": 210}),
        json!({"saturation": 101}),
        json!({"saturation": -101}),
        json!({"lightness": 101}),
        json!({"lightness": -101}),
        json!({"reds": {"hue": 181}}),
        json!({"reds": {"hue": -181}}),
        json!({"yellows": {"saturation": 101}}),
        json!({"greens": {"lightness": -101}}),
        json!({"reds": {"range": [315, 345, 15, 400]}}),
    ] {
        let err = s.execute("image.adjustments.hueSaturation", params.clone()).unwrap_err().to_string();
        assert!(err.contains("invalid settings"), "expected invalid settings error for {params}: {err}");
        assert_eq!(s.active().unwrap().history.past_len(), history_before, "history changed on {params}");
        assert_eq!(s.active().unwrap().doc.layers[0].content, doc_before.layers[0].content, "doc changed on {params}");
    }
}

#[test]
fn colorize_values_clamp_and_in_range_values_succeed() {
    let a = from_params("hueSaturation", &json!({"colorize": true, "hue": 400, "saturation": 150}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { hue: 360.0, saturation: 100.0, colorize: true, .. }));

    let b = from_params("hueSaturation", &json!({"colorize": true, "hue": -145}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(b, Adjustment::HueSaturation { hue: 0.0, colorize: true, .. }));

    for params in [
        json!({"hue": -180, "saturation": -100, "lightness": -100}),
        json!({"hue": 180, "saturation": 100, "lightness": 100}),
        json!({"reds": {"hue": -180, "saturation": 100, "lightness": -100, "range": [315, 345, 15, 45]}}),
    ] {
        assert!(from_params("hueSaturation", &params, None, ColorMode::Rgb).is_ok(), "{params}");
    }

    assert!(from_params("hueSaturation", &json!({"hue": 180.000001}), None, ColorMode::Rgb).is_err());
    assert!(from_params("hueSaturation", &json!({"reds": {"range": [315, 345, 15, 399.999999]}}), None, ColorMode::Rgb).is_ok());

    let wrapped = from_params("hueSaturation", &json!({"reds": {"range": [100, 300, 340, 40]}}), None, ColorMode::Rgb).unwrap();
    let round_tripped = from_params("hueSaturation", &to_params(&wrapped), None, ColorMode::Rgb).unwrap();
    assert_eq!(wrapped, round_tripped);
}
