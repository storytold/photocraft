//! #14419: Image › Adjustments › Color Lookup interpolates tetrahedrally and dithers unless the call says
//! otherwise, as the reference application does. Explicit values still apply, and the adjustment layer keeps
//! trilinear and no dither.

use super::*;

const IMAGE: &str = "image.adjustments.colorLookup";
const LAYER: &str = "layer.newAdjustmentLayer.colorLookup";

/// The width of each colour strip, and the document's height: 40 x 30 = 1200 pixels a strip, as measured.
const STRIP: i32 = 40;
const HEIGHT: i32 = 30;

/// The issue's `rg2.cube`: two points per axis, red out 1 only at the corners where red and green are both 1,
/// green and blue identity. Tetrahedral interpolation gives red = min(r, g) in the r >= g >= b tetrahedron,
/// trilinear gives red = r * g.
fn rg2() -> String {
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{} {g} {b}\n", r & g));
            }
        }
    }
    cube
}

/// A table that scales every channel by 0.3 (the same under either interpolation).
fn scale_by_three_tenths() -> String {
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{} {} {}\n", 0.3 * r as f32, 0.3 * g as f32, 0.3 * b as f32));
            }
        }
    }
    cube
}

/// An 8-bit RGB document with one strip of each of `colors`, left to right.
fn strips(colors: &[[u8; 3]]) -> Session {
    let mut s = crate::session();
    let width = STRIP * colors.len() as i32;
    s.execute("file.new", json!({"width": width, "height": HEIGHT, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _ in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let c = colors[(x / STRIP) as usize];
                data.extend(photocraft_raster::from_rgba(&fmt, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// Every pixel of strip `i` of the active layer, in 8-bit levels.
fn strip(s: &Session, i: usize) -> Vec<[u8; 3]> {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    let x0 = STRIP * i as i32;
    let mut out = Vec::new();
    for y in 0..HEIGHT {
        for x in x0..x0 + STRIP {
            let px = surf.rgba(x, y);
            out.push(std::array::from_fn(|c| (px[c] * 255.0).round() as u8));
        }
    }
    out
}

/// The measured colours through `rg2.cube`, and what the reference application gives for them.
const RG2_CASES: [([u8; 3], [u8; 3]); 6] = [
    ([16, 16, 16], [16, 16, 16]),
    ([128, 128, 128], [128, 128, 128]),
    ([240, 240, 240], [240, 240, 240]),
    ([200, 50, 50], [50, 50, 50]),
    ([255, 128, 0], [128, 128, 0]),
    ([30, 60, 90], [30, 60, 90]),
];

#[test]
fn without_interpolation_or_dither_the_adjustment_is_tetrahedral() {
    let colors = RG2_CASES.map(|(before, _)| before);
    let mut s = strips(&colors);
    s.execute(IMAGE, json!({"data": rg2(), "fileName": "rg2.cube"})).unwrap();
    for (i, (before, want)) in RG2_CASES.iter().enumerate() {
        // The results are whole levels, so dither leaves every pixel on them.
        assert!(strip(&s, i).iter().all(|px| px == want), "{before:?} should become {want:?}, got {:?}", strip(&s, i)[0]);
    }
}

#[test]
fn explicit_trilinear_and_no_dither_still_apply() {
    // Trilinear: red = r * g, so the grays darken in red (16 -> 1, 128 -> 64, 240 -> 226).
    let cases = [([16, 16, 16], [1, 16, 16]), ([128, 128, 128], [64, 128, 128]), ([240, 240, 240], [226, 240, 240])];
    // The command takes the choice as `interpolation` or as the `tetrahedral` flag.
    for params in [json!({"interpolation": "trilinear", "dither": false}), json!({"tetrahedral": false, "dither": false})] {
        let mut s = strips(&cases.map(|(before, _)| before));
        let mut p = params.clone();
        p["data"] = json!(rg2());
        p["fileName"] = json!("rg2.cube");
        s.execute(IMAGE, p).unwrap();
        for (i, (before, want)) in cases.iter().enumerate() {
            assert!(strip(&s, i).iter().all(|px| px == want), "{params}: {before:?} should become {want:?}, got {:?}", strip(&s, i)[0]);
        }
    }
}

#[test]
fn by_default_a_fraction_is_dithered_over_its_two_levels() {
    // 112 * 0.3 = 33.6: dither puts some pixels on 33 and the rest on 34; without it all round to 34.
    let levels = |params: Value| {
        let mut s = strips(&[[112, 112, 112]]);
        let mut p = params;
        p["data"] = json!(scale_by_three_tenths());
        p["fileName"] = json!("scale.cube");
        s.execute(IMAGE, p).unwrap();
        let reds: Vec<u8> = strip(&s, 0).iter().map(|px| px[0]).collect();
        (reds.iter().filter(|&&r| r == 33).count(), reds.iter().filter(|&&r| r == 34).count(), reds.len())
    };
    let (low, high, n) = levels(json!({}));
    assert!(low > 0 && high > 0 && low + high == n, "dithered by default: {low} at 33, {high} at 34 of {n}");
    let (low, high, n) = levels(json!({"dither": false}));
    assert_eq!((low, high), (0, n), "an explicit `dither: false` rounds every pixel");
}

#[test]
fn the_adjustment_layer_keeps_trilinear_and_no_dither() {
    let mut s = strips(&[[128, 128, 128]]);
    let id = photocraft_doc::LayerId(s.execute(LAYER, json!({"data": rg2(), "fileName": "rg2.cube"})).unwrap()["layer"].as_u64().unwrap());
    let d = s.active().unwrap();
    match &d.doc.layer(id).unwrap().content {
        LayerContent::Adjustment(Adjustment::ColorLookup { tetrahedral, dither, .. }) => {
            assert!(!tetrahedral && !dither, "tetrahedral {tetrahedral}, dither {dither}");
        }
        other => panic!("{other:?}"),
    }
}

/// The params line is what agents read and what the dialog opens with, so Dither opens checked there.
#[test]
fn each_commands_params_name_its_own_defaults() {
    let _ = crate::session();
    let image = crate::registry::find(IMAGE).unwrap().params;
    assert!(image.contains(r#""interpolation":"trilinear|tetrahedral"="tetrahedral""#) && image.contains(r#""dither":bool=true"#), "{image}");
    let layer = crate::registry::find(LAYER).unwrap().params;
    assert!(layer.contains(r#""interpolation":"trilinear|tetrahedral"="trilinear""#) && layer.contains(r#""dither":bool=false"#), "{layer}");
}

/// A Smart Filter's editor opens on `lookup_filter_params`, so params the filter would refuse must reach it as
/// recorded: an unknown `interpolation` stays an error even when the legacy flag sits beside it.
#[test]
fn a_smart_filters_unknown_interpolation_is_not_turned_into_a_setting() {
    for flag in [false, true] {
        let recorded = json!({"interpolation": "unsupported", "tetrahedral": flag});
        assert!(lookup_from_params(&recorded, None).is_err(), "{recorded}: the filter refuses it");
        assert_eq!(Value::Object(lookup_filter_params(recorded.as_object().unwrap())), recorded, "{recorded}: comes back as recorded");
    }
}
