//! #14720: HDR Toning's factory defaults (Radius 7.68, Strength 0.5296) and ranges (Gamma
//! 0.01-9.99) as in the reference application; the command still clamps out-of-range params and
//! never panics on bad ones.

use super::*;

const CMD: &str = "image.adjustments.hdrToning";

/// A gray document: a ramp across, so a tone change shows at every level.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 16})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let data: Vec<f32> = (b.y0..b.y1)
            .flat_map(|_| {
                (b.x0..b.x1).flat_map(|x| {
                    let v = x as f32 / 63.0;
                    photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0])
                })
            })
            .collect();
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// The toned document's gray level at column `x`, 0..1.
fn toned(p: Value, x: i32) -> f32 {
    let mut s = session();
    s.execute(CMD, p).unwrap();
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, 8)[0]
}

#[test]
fn factory_defaults_are_the_reference_applications() {
    let d = HdrToning::default();
    let got = [d.radius, d.strength, d.gamma, d.exposure, d.detail, d.shadow, d.highlight, d.vibrance, d.saturation];
    assert_eq!(got, [7.68, 0.5296, 1.0, 0.0, 30.0, 0.0, 0.0, 0.0, 20.0]);
    // No params tone exactly as the factory defaults passed explicitly.
    let explicit =
        json!({"radius": 7.68, "strength": 0.5296, "gamma": 1, "exposure": 0, "detail": 30, "shadow": 0, "highlight": 0, "vibrance": 0, "saturation": 20});
    for x in [5, 20, 40, 60] {
        assert_eq!(toned(json!({}), x), toned(explicit.clone(), x), "column {x}");
    }
}

#[test]
fn the_params_doc_lists_the_defaults_and_ranges() {
    let params = crate::commands::find(CMD).unwrap().params;
    for entry in [
        r#""radius":1..500=7.68"#,
        r#""strength":0.1..4=0.5296"#,
        r#""gamma":0.01..9.99=1"#,
        r#""exposure":-5..5=0"#,
        r#""detail":-100..300=30"#,
        r#""shadow":-100..100=0"#,
        r#""highlight":-100..100=0"#,
        r#""vibrance":-100..100=0"#,
        r#""saturation":-100..100=20"#,
    ] {
        assert!(params.contains(entry), "{entry} in {params}");
    }
}

#[test]
fn gamma_above_2_is_accepted_and_brighter() {
    // The checked path (smart filters) takes the whole range too.
    for gamma in [0.01, 5.0, 9.99] {
        assert!(tone::checked_hdr::validate(&HdrToning { gamma, ..Default::default() }).is_ok(), "gamma {gamma}");
    }
    let (g2, g5) = (toned(json!({"gamma": 2}), 20), toned(json!({"gamma": 5}), 20));
    assert!(g5 > g2, "Gamma 5 gives {g5}, Gamma 2 {g2}");
    assert!(toned(json!({"gamma": 9.99}), 20) > g5, "Gamma 9.99 brighter than 5");
}

#[test]
fn out_of_range_params_are_clamped_without_an_error() {
    let cases = [
        (json!({"radius": 600}), json!({"radius": 500})),
        (json!({"radius": 0}), json!({"radius": 1})),
        (json!({"strength": 9}), json!({"strength": 4})),
        (json!({"gamma": 20}), json!({"gamma": 9.99})),
        (json!({"gamma": 0}), json!({"gamma": 0.01})),
        (json!({"exposure": -9}), json!({"exposure": -5})),
        (json!({"saturation": 400}), json!({"saturation": 100})),
    ];
    for (given, limit) in cases {
        for x in [10, 40] {
            assert_eq!(toned(given.clone(), x), toned(limit.clone(), x), "{given} at column {x}");
        }
    }
}

#[test]
fn bad_params_fall_back_to_the_defaults_and_never_panic() {
    let default = toned(json!({}), 30);
    for p in [json!({"radius": "wide"}), json!({"gamma": null}), json!({"strength": true}), json!({"detail": [1, 2]}), json!({"exposure": {"x": 1}})] {
        assert_eq!(toned(p.clone(), 30), default, "{p}");
    }
    // Numbers too large for f32 become infinities, which clamp to the limits.
    for p in [json!({"radius": 1e300}), json!({"gamma": -1e300}), json!({"strength": 1e300})] {
        let v = toned(p.clone(), 30);
        assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{p}: {v}");
    }
}

#[test]
fn low_gamma_on_bright_pixels_stays_finite_on_both_paths() {
    // Gamma 0.01 raises the tone to the 100th power: bright pixels overflow f32. The command and
    // the checked (smart filter) path must both give finite, equal results.
    let input: Vec<[f32; 4]> = (0..64).map(|i| [i as f32 / 63.0 * 1.5, 0.4, 0.9, 1.0]).collect();
    for (gamma, exposure, highlight) in [(0.01, 1.0, 0.0), (0.01, 0.0, -100.0), (0.01, 5.0, 100.0), (0.05, 2.0, 0.0)] {
        let p = HdrToning { gamma, exposure, highlight, ..Default::default() };
        let mut direct = input.clone();
        tone::hdr_toning(&mut direct, 8, 8, &p);
        let mut checked = input.clone();
        tone::checked_hdr::process_with(&mut checked, 8, 8, &p, &photocraft_raster::Interrupt::NONE).unwrap();
        for (a, b) in direct.iter().zip(&checked) {
            assert!(a[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "gamma {gamma}, exposure {exposure}: {a:?}");
            assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "gamma {gamma}, exposure {exposure}: {a:?} vs {b:?}");
        }
    }
    // Through the command too, on the gray ramp with exposure +1.
    let mut s = session();
    s.execute(CMD, json!({"gamma": 0.01, "exposure": 1})).unwrap();
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    for x in 0..64 {
        assert!(surf.rgba(x, 8)[..3].iter().all(|v| v.is_finite()), "column {x}");
    }
}
