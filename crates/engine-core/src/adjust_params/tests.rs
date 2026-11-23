use super::*;

const KINDS: [&str; 15] = [
    "invert",
    "brightnessContrast",
    "threshold",
    "posterize",
    "exposure",
    "vibrance",
    "hueSaturation",
    "levels",
    "curves",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "gradientMap",
    "selectiveColor",
];

/// A non-default parameter set per kind (RGB channel keys for Levels/Curves).
pub(crate) fn sample(kind: &str) -> Value {
    match kind {
        "brightnessContrast" => json!({"brightness": 20, "contrast": -10, "legacy": true}),
        "threshold" => json!({"level": 100}),
        "posterize" => json!({"levels": 7}),
        "exposure" => json!({"exposure": 1.5, "offset": 0.1, "gamma": 1.4}),
        "vibrance" => json!({"vibrance": 30, "saturation": -20}),
        "hueSaturation" => {
            json!({"hue": 25, "saturation": -20, "lightness": 10, "reds": {"hue": 10, "saturation": -30, "lightness": 5, "range": [300, 340, 20, 50]}})
        }
        "levels" => json!({"inBlack": 10, "inWhite": 240, "outBlack": 5, "outWhite": 250, "gamma": 1.2, "red": {"inBlack": 3, "gamma": 0.8}}),
        "curves" => json!({"points": [[0, 0], [128, 150], [255, 255]], "blue": [[0, 20], [255, 235]]}),
        "colorBalance" => json!({"shadows": [10, -5, 0], "midtones": [0, 20, -30], "highlights": [5, 5, 5], "preserveLuminosity": false}),
        "blackWhite" => json!({"reds": 120, "blues": -40, "tint": true, "tintColor": "#d0a070"}),
        "photoFilter" => json!({"color": "#3366cc", "density": 40, "preserveLuminosity": false}),
        "channelMixer" => json!({"red": [80, 30, -10, 5], "blue": [0, 10, 90, 0]}),
        "gradientMap" => json!({"stops": [[0, "#000000"], [0.5, "#ff0000"], [1, [1, 1, 1]]], "reverse": true, "dither": true}),
        "selectiveColor" => json!({"method": "absolute", "reds": [10, 0, -20, 5]}),
        _ => json!({}),
    }
}

#[test]
fn every_kind_round_trips_through_params() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale] {
        for kind in KINDS {
            let mut p = sample(kind);
            if matches!(kind, "levels" | "curves") && default_space(mode) != ToneSpace::Rgb {
                // Off RGB the channel keys are the document's.
                p.as_object_mut().unwrap().retain(|k, _| !matches!(k.as_str(), "red" | "blue"));
            }
            let a = from_params(kind, &p, None, mode).unwrap_or_else(|e| panic!("{kind} {mode:?}: {e}"));
            assert_ne!(Some(&a), default_for(kind, mode).ok().as_ref().filter(|_| kind != "invert"), "{kind} params ignored");
            let back = from_params(kind, &to_params(&a), None, mode).unwrap();
            assert_eq!(to_params(&back), to_params(&a), "{kind} {mode:?}");
            // A partial update keeps everything it doesn't name.
            let again = from_params(kind, &json!({}), Some(&a), mode).unwrap();
            assert_eq!(again, a, "{kind} {mode:?}");
        }
    }
}

/// A Channel Mixer made in a CMYK document has Photoshop's four ink outputs, starting at
/// identity (#8047); RGB and other documents keep red, green and blue.
#[test]
fn channel_mixer_outputs_follow_the_document_mode() {
    let ink = default_for("channelMixer", ColorMode::Cmyk).unwrap();
    assert!(matches!(ink, Adjustment::ChannelMixer { cmyk: Some(rows), monochrome: false, .. } if rows == photocraft_doc::adjust::CMYK_MIXER_IDENTITY));
    let p = to_params(&ink);
    assert!(MIXER_INKS.iter().all(|k| p.get(*k).is_some()) && p.get("red").is_none(), "{p}");
    assert_eq!(p["black"], json!([0.0, 0.0, 0.0, 100.0, 0.0]));
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab] {
        assert!(matches!(default_for("channelMixer", mode).unwrap(), Adjustment::ChannelMixer { cmyk: None, .. }), "{mode:?}");
    }
    // CMYK keys make ink outputs in any document; a partial update keeps the rest.
    let a = from_params("channelMixer", &json!({"magenta": [0, 100, 0, 0, -20]}), None, ColorMode::Rgb).unwrap();
    let Adjustment::ChannelMixer { cmyk: Some(rows), .. } = &a else { panic!("{a:?}") };
    assert_eq!(rows[1], [0.0, 1.0, 0.0, 0.0, -0.2]);
    assert_eq!(rows[3], [0.0, 0.0, 0.0, 1.0, 0.0]);
    let b = from_params("channelMixer", &json!({"black": [10, 10, 10, 90, 0]}), Some(&a), ColorMode::Cmyk).unwrap();
    assert!(matches!(&b, Adjustment::ChannelMixer { cmyk: Some(r), .. } if r[1] == rows[1] && r[3] == [0.1, 0.1, 0.1, 0.9, 0.0]));
    assert_eq!(from_params("channelMixer", &to_params(&b), None, ColorMode::Rgb).unwrap(), b);
    // RGB keys over a CMYK base switch back to RGB outputs.
    let rgb = from_params("channelMixer", &json!({"red": [50, 50, 0, 0]}), Some(&b), ColorMode::Cmyk).unwrap();
    assert!(matches!(rgb, Adjustment::ChannelMixer { cmyk: None, matrix, .. } if matrix[0] == [0.5, 0.5, 0.0, 0.0] && matrix[1] == [0.0, 1.0, 0.0, 0.0]));
}

/// Monochrome on a CMYK Channel Mixer has no Photoshop sample yet (#8047), so turning it on is
/// an unsupported error, never an inferred mixer: in a CMYK document, with CMYK keys, over a CMYK
/// base, and over an imported monochrome CMYK mixer unless the edit turns Monochrome off. RGB
/// mixers keep Monochrome.
#[test]
fn monochrome_cmyk_channel_mixer_is_refused_as_unsupported() {
    let ink = default_for("channelMixer", ColorMode::Cmyk).unwrap();
    let imported = Adjustment::ChannelMixer {
        matrix: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        monochrome: true,
        cmyk: Some([[0.3, 0.3, 0.3, 1.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0, 0.0]]),
    };
    let cases: [(Value, Option<&Adjustment>, ColorMode); 6] = [
        (json!({"monochrome": true}), None, ColorMode::Cmyk),
        (json!({"monochrome": true, "gray": [0, 0, 0, 100, 0]}), None, ColorMode::Cmyk),
        (json!({"monochrome": true, "cyan": [100, 0, 0, 0, 0]}), None, ColorMode::Rgb),
        (json!({"monochrome": true}), Some(&ink), ColorMode::Cmyk),
        (json!({"gray": [10, 10, 10, 90, 0]}), Some(&imported), ColorMode::Cmyk),
        (to_params(&imported), None, ColorMode::Cmyk),
    ];
    for (p, base, mode) in cases {
        let err = from_params("channelMixer", &p, base, mode).expect_err(&format!("{p} over {base:?}"));
        assert!(matches!(&err, EngineError::BadParams { msg, .. } if msg == MONOCHROME_CMYK_UNSUPPORTED), "{err}");
        assert!(err.to_string().contains("unsupported"), "{err}");
    }
    // Turning an imported one's Monochrome off is a supported CMYK mixer again.
    let off = from_params("channelMixer", &json!({"monochrome": false}), Some(&imported), ColorMode::Cmyk).unwrap();
    assert!(matches!(off, Adjustment::ChannelMixer { cmyk: Some(_), monochrome: false, .. }), "{off:?}");
    // RGB keeps it, in any document.
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        let rgb = from_params("channelMixer", &json!({"monochrome": true, "red": [100, 0, 0, 0]}), None, mode).unwrap();
        assert!(matches!(rgb, Adjustment::ChannelMixer { cmyk: None, monochrome: true, .. }), "{mode:?}: {rgb:?}");
    }
    // The command refuses it and leaves the layer as it was.
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "cmyk", "background": "white"})).unwrap();
    s.execute("layer.newAdjustmentLayer.channelMixer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.layer(id).unwrap().content.clone();
    let err = s.execute("layer.setAdjustment", json!({"layer": id.0, "monochrome": true})).unwrap_err();
    assert!(err.to_string().contains("unsupported"), "{err}");
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().content, before);
}

#[test]
fn params_reach_the_model() {
    basic_params_reach_the_model();
    color_space_params_reach_the_model();
}

fn basic_params_reach_the_model() {
    let a = from_params("colorBalance", &json!({"midtones": [50, 0, -200]}), None, ColorMode::Rgb).unwrap();
    assert_eq!(a, Adjustment::ColorBalance { shadows: [0.0; 3], midtones: [50.0, 0.0, -100.0], highlights: [0.0; 3], preserve_luminosity: true });
    let a = from_params("channelMixer", &json!({"monochrome": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::ChannelMixer { matrix, monochrome: true, .. } if matrix[0] == [0.4, 0.4, 0.2, 0.0]));
    let a = from_params("photoFilter", &json!({"filter": "cooling80", "density": 60}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::PhotoFilter { color, density, .. } if color[2] == 1.0 && (density - 0.6).abs() < 1e-6));
    let a = from_params("blackWhite", &json!({"reds": 120, "tint": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::BlackWhite { weights, tint: Some(_) } if weights[0] == 120.0 && weights[1] == 60.0));
    let a = from_params("hueSaturation", &json!({"blues": {"hue": 40}}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::HueSaturation { ranges, .. } if ranges[4].hue == 40.0 && ranges[0].is_neutral()));
    // Colorize without a saturation starts at 25%; an explicit 0 and an already colorized base are kept.
    let a = from_params("hueSaturation", &json!({"colorize": true, "hue": 30}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 25.0, .. }), "{a:?}");
    let a = from_params("hueSaturation", &json!({"colorize": true, "saturation": 0}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 0.0, .. }), "{a:?}");
    let a = from_params("hueSaturation", &json!({"hue": 120}), Some(&a), ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 0.0, hue: 120.0, .. }), "{a:?}");
    let a = from_params("gradientMap", &json!({"stops": [[1, "#ffffff"], [0, "#000000"]], "dither": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::GradientMap { stops, dither: true, .. } if stops[0].0 == 0.0));
}

fn color_space_params_reach_the_model() {
    // CMYK curves: ink channels; Lab: lightness without a composite.
    let a = from_params("curves", &json!({"black": [[0, 0], [255, 200]]}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::Curves { space: ToneSpace::Cmyk, black, .. } if black.len() == 2));
    let a = from_params("curves", &json!({"points": [[0, 40], [255, 255]]}), None, ColorMode::Lab).unwrap();
    assert!(matches!(&a, Adjustment::Curves { space: ToneSpace::Lab, per_channel, .. } if per_channel[0][0].output > 0.1));
    let a = from_params("levels", &json!({"inBlack": 20}), None, ColorMode::Cmyk).unwrap();
    assert!(matches!(&a, Adjustment::Levels { space: ToneSpace::Cmyk, master, .. } if (master.in_black - 20.0 / 255.0).abs() < 1e-6));
}

#[test]
fn bad_params_are_errors_not_panics() {
    for (kind, p) in [
        ("curves", json!({"points": "abc"})),
        ("curves", json!({"points": [[0, 0]]})),
        ("curves", json!({"red": [[0, 0], [1]]})),
        ("curves", json!({"points": [[5, 5], [5, 9]]})),
        ("curves", json!({"pencil": true, "points": [1, 2, 3]})),
        ("curves", json!({"pencil": "yes"})),
        ("curves", json!({"points": (0..256).map(|k| if k == 9 { json!("x") } else { json!(k) }).collect::<Vec<_>>()})),
        ("levels", json!({"red": 3})),
        ("levels", json!({"inBlack": "x"})),
        ("hueSaturation", json!({"reds": {"range": [1, 2]}})),
        ("hueSaturation", json!({"colorize": 1})),
        ("colorBalance", json!({"shadows": [1, 2]})),
        ("blackWhite", json!({"tintColor": "#zz0000"})),
        ("photoFilter", json!({"filter": "nope"})),
        ("photoFilter", json!({"color": [1]})),
        ("channelMixer", json!({"red": [1, 2, 3]})),
        ("channelMixer", json!({"cyan": [1, 2, 3, 4]})),
        ("channelMixer", json!({"black": [1, 2, 3, 4, "x"]})),
        ("gradientMap", json!({"stops": [[0, "#000"]]})),
        ("gradientMap", json!({"stops": 7})),
        ("selectiveColor", json!({"method": 5})),
        ("selectiveColor", json!({"reds": "not-an-array"})),
        ("selectiveColor", json!({"colors": "reds", "cyan": "50"})),
        ("colorLookup", json!({"lut": 42})),
        ("colorLookup", json!({"interpolation": 5})),
        ("colorLookup", json!({"dither": "yes"})),
        ("nope", json!({})),
        ("levels", json!([1, 2])),
    ] {
        assert!(from_params(kind, &p, None, ColorMode::Rgb).is_err(), "{kind} {p}");
    }
    // Out-of-range numbers clamp.
    let a = from_params("hueSaturation", &json!({"hue": 1e9, "saturation": -1e9}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { hue: 180.0, saturation: -100.0, .. }));
    let a = from_params("levels", &json!({"inBlack": 250, "inWhite": 10}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::Levels { master, .. } if master.in_white > master.in_black));
}

#[test]
fn curve_points_outside_0_255_are_refused_as_the_message_says() {
    // Photoshop refuses them too ("the settings in the file are invalid"); they used to be
    // moved onto the edge silently.
    for pts in [json!([[0, 0], [300, 255]]), json!([[0, -1], [255, 255]]), json!([[0, 0]])] {
        let e = from_params("curves", &json!({ "points": pts }), None, ColorMode::Rgb).unwrap_err().to_string();
        assert!(e.contains("[input, output] pairs in 0..255") && e.contains("`curves adjustment`"), "{e}");
    }
    assert!(from_params("curves", &json!({"points": [[0, 0], [128.5, 140], [255, 255]]}), None, ColorMode::Rgb).is_ok());
}

fn curves_parts(a: &Adjustment) -> (&[CurvePoint], &[Vec<CurvePoint>; 3]) {
    match a {
        Adjustment::Curves { master, per_channel, .. } => (master, per_channel),
        _ => panic!("expected curves"),
    }
}

fn tone_output(adjustment: &Adjustment, sample: [f32; 3]) -> [f32; 3] {
    let luts = photocraft_compose::adjust::tone_luts(adjustment);
    std::array::from_fn(|channel| {
        let row = &luts[channel];
        let position = sample[channel].clamp(0.0, 1.0) * (row.len().saturating_sub(1)) as f32;
        let lower = position.floor() as usize;
        let upper = lower.saturating_add(1).min(row.len().saturating_sub(1));
        let fraction = position - lower as f32;
        row.get(lower).copied().unwrap_or(0.0) * (1.0 - fraction) + row.get(upper).copied().unwrap_or(0.0) * fraction
    })
}

#[test]
fn curves_black_and_white_eyedroppers_move_the_input_references() {
    let base = json!({
        "points": [[0, 8], [120, 150], [255, 245]],
        "red": [[0, 0], [100, 130], [255, 255]],
        "green": [[0, 0], [110, 125], [255, 255]],
        "blue": [[0, 0], [90, 120], [255, 255]]
    });
    let black = curves_eyedropper(&base, CurvesEyedropper::Black, [0.2, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&black);
    for (curve, sample) in channels.iter().zip([0.2, 0.3, 0.4]) {
        assert!((curve[0].input - sample).abs() < 1e-6 && curve[0].output == 0.0, "{curve:?}");
        assert_eq!(curve.len(), 3, "existing edit points are preserved");
    }
    assert_eq!(master[0].output, 0.0, "the composite cannot lift the sampled black");
    assert!(tone_output(&black, [0.2, 0.3, 0.4]).iter().all(|value| *value < 1e-3));

    let white = curves_eyedropper(&base, CurvesEyedropper::White, [0.6, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&white);
    for (curve, sample) in channels.iter().zip([0.6, 0.7, 0.8]) {
        let end = curve.last().unwrap();
        assert!((end.input - sample).abs() < 1e-6 && end.output == 1.0, "{curve:?}");
        assert_eq!(curve.len(), 3, "existing edit points are preserved");
    }
    assert_eq!(master.last().map(|p| p.output), Some(1.0), "the composite cannot lower the sampled white");
    assert!(tone_output(&white, [0.6, 0.7, 0.8]).iter().all(|value| *value > 1.0 - 1e-3));

    let edge_black = curves_eyedropper(&base, CurvesEyedropper::Black, [1.0, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let (_, edge_channels) = curves_parts(&edge_black);
    assert_eq!(edge_channels[0].len(), 3, "edge samples preserve other curve points");
    assert_eq!(edge_channels[0].last().map(|p| p.output), Some(0.0));
    let edge_white = curves_eyedropper(&base, CurvesEyedropper::White, [0.0, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let (_, edge_channels) = curves_parts(&edge_white);
    assert_eq!(edge_channels[0].len(), 3, "edge samples preserve other curve points");
    assert_eq!(edge_channels[0].first().map(|p| p.output), Some(1.0));
}

#[test]
fn black_and_white_eyedroppers_round_trip_near_serialization_boundaries() {
    let cases = [
        (CurvesEyedropper::White, 0.0, 1.0),
        (CurvesEyedropper::White, 0.001, 1.0),
        (CurvesEyedropper::White, 0.494 / 255.0, 1.0),
        // 0.496 level rounds to 0.50; the sample is just below that representable point.
        (CurvesEyedropper::White, 0.496 / 255.0, 0.992),
        (CurvesEyedropper::White, 0.2, 1.0),
        (CurvesEyedropper::Black, 1.0, 0.0),
        (CurvesEyedropper::Black, 0.999, 0.0),
        (CurvesEyedropper::Black, 1.0 - 0.494 / 255.0, 0.0),
        // 254.504 levels rounds to 254.50, leaving a representable 0.50-level gap.
        (CurvesEyedropper::Black, 1.0 - 0.496 / 255.0, 0.008),
        (CurvesEyedropper::Black, 0.8, 0.0),
    ];
    for (picker, sample, expected_output) in cases {
        let resolved = curves_eyedropper(&json!({}), picker, [sample; 3], ColorMode::Rgb).unwrap();
        let params = to_params(&resolved);
        let parsed = from_params("curves", &params, None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert!(curve.len() >= 2, "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.len() <= MAX_CURVE_POINTS, "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[0].input < pair[1].input), "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{picker:?} sample={sample}: {curve:?}");
        }
        let output = tone_output(&parsed, [sample; 3]);
        assert!(output.iter().all(|value| (*value - expected_output).abs() < 1e-3), "{picker:?} sample={sample} output={output:?} expected={expected_output}");
    }
}

#[test]
fn endpoint_eyedroppers_fall_back_when_serialization_collapses_curve_points() {
    let base = json!({
        "points": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "red": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "green": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "blue": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]]
    });
    let cases = [(CurvesEyedropper::Black, 0.995, 0.0, true), (CurvesEyedropper::White, 0.005, 1.0, false)];
    for (picker, sample, target, black) in cases {
        let resolved = curves_eyedropper(&base, picker, [sample; 3], ColorMode::Rgb).unwrap();
        let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert_eq!(curve.len(), 5, "collapsed points use the deterministic fallback: {curve:?}");
            for (point, input) in curve.iter().zip([0.0, 64.0, 128.0, 192.0, 255.0]) {
                assert!((point.input * 255.0 - input).abs() < 1e-3, "existing point position was retained: {curve:?}");
            }
        }
        for curve in channels {
            let plateau: Vec<_> = if black { curve.iter().rev().take(2).collect() } else { curve.iter().take(2).collect() };
            assert!(plateau.into_iter().all(|point| (point.output - target).abs() < 1e-6), "{curve:?}");
        }
        let output = tone_output(&parsed, [sample; 3]);
        assert!(output.iter().all(|value| (*value - target).abs() < 0.01), "{picker:?}: {output:?}");
    }
}

#[test]
fn black_and_white_round_trips_preserve_representable_curve_edits() {
    let base = json!({
        "points": [[0, 8], [120, 150], [255, 245]],
        "red": [[0, 0], [100, 130], [255, 255]],
        "green": [[0, 0], [110, 125], [255, 255]],
        "blue": [[0, 0], [90, 120], [255, 255]]
    });
    let black = curves_eyedropper(&base, CurvesEyedropper::Black, [0.2, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let black = from_params("curves", &to_params(&black), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&black);
    assert_eq!(channels[0].len(), 3);
    assert!((channels[0][1].input * 255.0 - 131.0).abs() < 1e-3);
    assert!((channels[0][1].output * 255.0 - 130.0).abs() < 1e-3);

    let white = curves_eyedropper(&base, CurvesEyedropper::White, [0.6, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let white = from_params("curves", &to_params(&white), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&white);
    assert_eq!(channels[0].len(), 3);
    assert!((channels[0][1].input * 255.0 - 60.0).abs() < 1e-3);
    assert!((channels[0][1].output * 255.0 - 130.0).abs() < 1e-3);
}

fn assert_gray_point_preserves_luma_and_existing_curve_edits(sample: [f32; 3]) {
    let base = json!({
        "points": [[0, 0], [128, 96], [255, 255]],
        "red": [[0, 0], [80, 90], [255, 255]],
        "green": [[0, 0], [90, 100], [255, 255]],
        "blue": [[0, 0], [70, 80], [255, 255]]
    });
    let original = from_params("curves", &base, None, ColorMode::Rgb).unwrap();
    let expected_luma = rgb_luma(sample);
    let resolved = curves_eyedropper(&base, CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
    let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&parsed);
    let (original_master, original_channels) = curves_parts(&original);
    assert_eq!(master, original_master, "the existing master curve is retained");
    for (curve, old_curve) in channels.iter().zip(original_channels) {
        let old_edit = old_curve.get(1).expect("the test curve has an interior edit");
        assert!(
            curve.iter().any(|point| { (point.input - old_edit.input).abs() < 1e-6 && (point.output - old_edit.output).abs() < 1e-6 }),
            "the existing channel edit is retained: {curve:?}"
        );
    }
    let output = tone_output(&parsed, sample);
    assert!((output[0] - output[1]).abs() < 0.002 && (output[1] - output[2]).abs() < 0.002, "sample must become neutral: {output:?}");
    assert!((rgb_luma(output) - expected_luma).abs() < 0.003, "luma changed from {expected_luma} to {} ({output:?})", rgb_luma(output));
}

#[test]
fn curves_neutral_gray_preserves_luma_for_a_light_cast_sample() {
    assert_gray_point_preserves_luma_and_existing_curve_edits([0.88, 0.76, 0.63]);
}

#[test]
fn curves_neutral_gray_preserves_luma_for_a_dark_cast_sample() {
    assert_gray_point_preserves_luma_and_existing_curve_edits([0.17, 0.105, 0.075]);
}

#[test]
fn curves_neutral_gray_handles_near_black_and_near_white_samples() {
    for sample in [[0.004, 0.006, 0.008], [0.992, 0.996, 0.999], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
        let resolved = curves_eyedropper(&json!({}), CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
        let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert!(curve.len() >= 2, "{sample:?}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[0].input < pair[1].input), "{sample:?}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{sample:?}: {curve:?}");
        }
        let output = tone_output(&parsed, sample);
        assert!((output[0] - output[1]).abs() < 0.002 && (output[1] - output[2]).abs() < 0.002, "{sample:?}: {output:?}");
        assert!((rgb_luma(output) - rgb_luma(sample)).abs() < 0.003, "{sample:?}: {output:?}");
    }
}

#[test]
fn gray_point_fallback_keeps_tightly_spaced_points_round_trip_safe() {
    let base = json!({
        "red": [[0, 0], [100, 100], [100.51, 100], [255, 255]],
        "green": [[0, 0], [100, 100], [100.51, 100], [255, 255]],
        "blue": [[0, 0], [100, 100], [100.51, 100], [255, 255]]
    });
    let sample = [100.25 / 255.0, 100.1 / 255.0, 100.4 / 255.0];
    let target = rgb_luma(sample);
    let resolved = curves_eyedropper(&base, CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
    let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&parsed);
    for (curve, input) in channels.iter().zip(sample) {
        assert_eq!(curve.len(), 4, "the fallback keeps every serialized point: {curve:?}");
        assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{curve:?}");
        let sample_pair = curve.windows(2).find(|pair| pair[0].input <= input && input <= pair[1].input).unwrap();
        assert!((sample_pair[0].output - sample_pair[1].output).abs() < 1e-6, "fallback forms a plateau around the sample: {curve:?}");
        assert!((sample_pair[0].output - target).abs() < 0.001, "fallback uses the luma target: {curve:?}");
    }
}

#[test]
fn curves_eyedropper_support_is_explicit_and_bad_samples_fail() {
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [0.4; 3], ColorMode::Grayscale).is_ok());
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::White, [0.4; 3], ColorMode::Grayscale).is_ok());
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::NeutralGray, [0.4; 3], ColorMode::Grayscale).is_err());
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [0.4; 3], mode).is_err());
    }
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [f32::NAN, 0.0, 0.0], ColorMode::Rgb).is_err());
}

#[test]
fn hue_range_sliders_stay_ordered() {
    let a = from_params("hueSaturation", &json!({"greens": {"range": [100, 90, 200, 150]}}), None, ColorMode::Rgb).unwrap();
    let Adjustment::HueSaturation { ranges, .. } = a else { panic!() };
    let b = ranges[2].bounds;
    assert!(b[0] <= b[1] && b[1] <= b[2] && b[2] <= b[3] && b[3] - b[0] < 360.0, "{b:?}");
}

/// #8351: a Gradient Map's Photoshop gradient goes through the params: kept (with the stops
/// already baked) while unchanged, baked again when edited, dropped by plain stops or null,
/// and refused when malformed.
#[test]
fn gradient_map_source_params() {
    let src = json!({"stops": [[0, "#102030"], [1, "#ffcc33"]], "midpoints": [0.3], "smoothness": 0.8, "method": "perceptual"});
    let a = from_params("gradientMap", &json!({"source": src}), None, ColorMode::Rgb).unwrap();
    let Adjustment::GradientMap { stops, source: Some(kept), .. } = &a else { panic!("{a:?}") };
    assert!(stops.len() > 2 && kept.stops.len() == 2 && kept.method == "perceptual");
    // The params round trip, and an unchanged source keeps the stops as they were.
    let p = to_params(&a);
    assert_eq!(p["source"]["midpoints"], json!([0.30000001192092896]));
    let mut same = a.clone();
    if let Adjustment::GradientMap { stops, .. } = &mut same {
        stops[1].1 = [0.5; 3];
    }
    let kept_stops = from_params("gradientMap", &json!({"source": to_params(&same)["source"], "reverse": true}), Some(&same), ColorMode::Rgb).unwrap();
    assert!(matches!(&kept_stops, Adjustment::GradientMap { stops, reverse: true, .. } if stops[1].1 == [0.5; 3]), "not baked again");
    // An edited source is baked again.
    let mut edited = p["source"].clone();
    edited["stops"][1][1] = json!("#00ff00");
    let b = from_params("gradientMap", &json!({"source": edited}), Some(&a), ColorMode::Rgb).unwrap();
    let Adjustment::GradientMap { stops: sb, source: Some(kb), .. } = &b else { panic!() };
    assert_eq!(kb.stops[1].1, [0.0, 1.0, 0.0]);
    assert_ne!(sb, stops);
    // Plain stops or null drop it; untouched, it stays.
    for p in [json!({"stops": [[0, "#000000"], [1, "#ffffff"]]}), json!({"source": null})] {
        assert!(matches!(from_params("gradientMap", &p, Some(&a), ColorMode::Rgb).unwrap(), Adjustment::GradientMap { source: None, .. }), "{p}");
    }
    assert!(matches!(from_params("gradientMap", &json!({"dither": true}), Some(&a), ColorMode::Rgb).unwrap(), Adjustment::GradientMap { source: Some(_), .. }));
    // Malformed sources are refused.
    for bad in [
        json!({"stops": [[0, "#000000"]]}),
        json!({"stops": [[0, "#000000"], [1, "#ffffff"]], "midpoints": [0.5, 0.5]}),
        json!({"stops": [[0, "#000000"], [1, "#ffffff"]], "midpoints": [2]}),
        json!({"stops": [[0, "#000000"], [1, "#ffffff"]], "smoothness": 7}),
        json!({"stops": [[0, "#000000"], [1, "#ffffff"]], "method": "wavy"}),
        json!(3),
    ] {
        assert!(from_params("gradientMap", &json!({"source": bad}), None, ColorMode::Rgb).is_err(), "{bad}");
    }
}

/// #14414: the reference application mixes with whole percentages. Scripted values round to the
/// nearest whole percent before clamping; the dialog already sends whole percents.
#[test]
fn channel_mixer_percentages_are_whole_numbers() {
    let a = from_params(
        "channelMixer",
        &json!({"red": [0.4, 0.5, 50.6, -0.5], "green": [33.3, 33.3, 33.3, 0.0], "blue": [-300.0, 300.0, 0.0, 0.0]}),
        None,
        ColorMode::Rgb,
    )
    .unwrap();
    let Adjustment::ChannelMixer { matrix, .. } = a else { panic!("wrong kind") };
    assert_eq!(matrix[0], [0.0, 0.01, 0.51, -0.01], "0.4 to 0, 0.5 to 1, 50.6 to 51, halves away from zero");
    assert_eq!(matrix[1], [0.33, 0.33, 0.33, 0.0], "33.3 stores as 33");
    assert_eq!(matrix[2], [-2.0, 2.0, 0.0, 0.0], "clamps stay -200 and 200");
    assert_eq!(to_params(&a)["red"], json!([0.0, 1.0, 51.0, -1.0]), "the stored step reads back whole");
}

/// #14417: tone values are whole percentages; scripted fractions round like the dialog's own
/// slider pass (away from zero on halves, which no measured case pins to).
#[test]
fn color_balance_tones_round_to_whole_numbers() {
    let a = from_params("colorBalance", &json!({"midtones": [12.7, -0.4, 100.4], "highlights": [-100.4, 0.0, 0.0]}), None, ColorMode::Rgb).unwrap();
    let Adjustment::ColorBalance { midtones, highlights, .. } = a else { panic!("wrong kind") };
    assert_eq!(midtones, [13.0, 0.0, 100.0], "12.7 to 13, -0.4 to 0, 100.4 rounds into the clamp");
    assert_eq!(highlights, [-100.0, 0.0, 0.0], "-100.4 to -100");
}
