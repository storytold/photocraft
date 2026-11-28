//! Color Lookup's Data Order and Table Order for `.cube` files (#14423), measured in the reference
//! application with `swap2.cube` (2x2x2, out = (b, g, r)) over gray strips and six colour strips.

use super::*;

/// The issue's strips: grays, then (200,50,50) (50,200,50) (50,50,200) (255,128,0) (30,60,90) (128,128,128).
const STRIPS: [[u8; 3]; 10] =
    [[0, 0, 0], [64, 64, 64], [192, 192, 192], [255, 255, 255], [200, 50, 50], [50, 200, 50], [50, 50, 200], [255, 128, 0], [30, 60, 90], [128, 128, 128]];

const DEPTHS: [u64; 3] = [8, 16, 32];

/// `swap2.cube`: a 2³ table in the standard layout (red index fastest) whose output is (b, g, r).
fn swap2() -> String {
    let mut cube = String::from("TITLE \"swap2\"\nLUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{b} {g} {r}\n"));
            }
        }
    }
    cube
}

/// A one-row document with one pixel per strip at `depth`.
fn strips(depth: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": STRIPS.len(), "height": 1, "depth": depth, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for c in STRIPS {
            data.extend(photocraft_raster::from_rgba(&fmt, [0, 1, 2, 3].map(|i| c.get(i).map_or(1.0, |v| f32::from(*v) / 255.0))));
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// The strips as 0..255 levels.
fn levels(s: &Session) -> Vec<[f32; 3]> {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..STRIPS.len() as i32).map(|x| surf.rgba(x, 0)).map(|p| [p[0] * 255.0, p[1] * 255.0, p[2] * 255.0]).collect()
}

fn swapped(c: [u8; 3]) -> [u8; 3] {
    [c[2], c[1], c[0]]
}

fn assert_strips(got: &[[f32; 3]], want: impl Fn([u8; 3]) -> [u8; 3], what: &str) {
    for (strip, px) in STRIPS.iter().zip(got) {
        let w = want(*strip);
        for c in 0..3 {
            assert!((px[c] - f32::from(w[c])).abs() <= 0.5, "{what}: strip {strip:?} gave {px:?}, want {w:?}");
        }
    }
}

/// The issue's truth table: the defaults apply the file as written; BGR data or RGB table order
/// alone undo the swap; both together swap again.
#[test]
fn the_four_orders_give_the_measured_swap2_results() {
    let cases: [(Option<&str>, Option<&str>, bool); 5] = [
        (None, None, true),
        (Some("rgb"), Some("bgr"), true),
        (Some("bgr"), Some("bgr"), false),
        (Some("rgb"), Some("rgb"), false),
        (Some("bgr"), Some("rgb"), true),
    ];
    for depth in DEPTHS {
        for (data, table, swaps) in cases {
            let mut s = strips(depth);
            let mut p = json!({"data": swap2(), "fileName": "swap2.cube", "dither": false});
            if let Some(o) = data {
                p["dataOrder"] = json!(o);
            }
            if let Some(o) = table {
                p["tableOrder"] = json!(o);
            }
            s.execute("image.adjustments.colorLookup", p).unwrap();
            let what = format!("{depth} bit, data {data:?}, table {table:?}");
            assert_strips(&levels(&s), if swaps { swapped } else { |c| c }, &what);
        }
    }
}

/// Table Order RGB transposes the grid (blue index fastest), so a table that is not symmetric
/// in its axes reads different nodes: out = (r, 0.25 + 0.5g, 0.125 + 0.25b) as in #9126.
#[test]
fn table_order_moves_nodes_and_data_order_reverses_each_line() {
    let f = |r: f32, g: f32, b: f32| [r, 0.25 + 0.5 * g, 0.125 + 0.25 * b];
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for i in 0..8 {
        let [r, g, b] = f((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2 & 1) as f32);
        cube.push_str(&format!("{r} {g} {b}\n"));
    }
    // Input pure red (255, 0, 0) for each order, as PR #9218 reads a PSD's stored orders.
    let cases = [("rgb", "bgr", f(1.0, 0.0, 0.0)), ("bgr", "bgr", [0.125, 0.25, 1.0]), ("rgb", "rgb", f(0.0, 0.0, 1.0)), ("bgr", "rgb", [0.375, 0.25, 0.0])];
    for (data, table, want) in cases {
        let mut one = crate::session();
        one.execute("file.new", json!({"width": 1, "height": 1, "depth": 16, "mode": "rgb", "background": "backgroundColor", "backgroundColor": [1, 0, 0]}))
            .unwrap();
        one.execute("image.adjustments.colorLookup", json!({"data": cube, "fileName": "f.cube", "dataOrder": data, "tableOrder": table, "dither": false}))
            .unwrap();
        let d = one.active().unwrap();
        let px = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
        for c in 0..3 {
            assert!((px[c] - want[c]).abs() < 1e-3, "data {data}, table {table}: {px:?}, want {want:?}");
        }
    }
}

/// `.3dl` and `.look` files ignore both orders: Lustre tables have their own layout.
#[test]
fn other_formats_ignore_the_orders() {
    let mut lut = String::from("0 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                // Out = (b, g, r), blue fastest as `.3dl` files are.
                lut.push_str(&format!("{} {} {}\n", b * 1023, g * 1023, r * 1023));
            }
        }
    }
    let mut plain = strips(8);
    plain.execute("image.adjustments.colorLookup", json!({"data": lut, "fileName": "swap2.3dl", "dither": false})).unwrap();
    let mut ordered = strips(8);
    ordered
        .execute("image.adjustments.colorLookup", json!({"data": lut, "fileName": "swap2.3dl", "dataOrder": "bgr", "tableOrder": "rgb", "dither": false}))
        .unwrap();
    assert_eq!(levels(&plain), levels(&ordered));
    assert_ne!(levels(&plain), levels(&strips(8)), "the .3dl applied");
}

/// Not measured for 1D `.cube` files (`LUT_1D_SIZE`): they keep today's result whatever the orders.
#[test]
fn a_1d_cube_ignores_the_orders() {
    let cube = "LUT_1D_SIZE 2\n0 0 0\n0.5 1 0.25\n";
    let mut plain = strips(8);
    plain.execute("image.adjustments.colorLookup", json!({"data": cube, "fileName": "ramp.cube", "dither": false})).unwrap();
    let mut ordered = strips(8);
    ordered
        .execute("image.adjustments.colorLookup", json!({"data": cube, "fileName": "ramp.cube", "dataOrder": "bgr", "tableOrder": "rgb", "dither": false}))
        .unwrap();
    assert_eq!(levels(&plain), levels(&ordered));
}

/// A smart filter records the orders with its params, so a re-run (here: an edit of the smart
/// object's contents being re-rendered through the filter) gives the same pixels.
#[test]
fn a_smart_filter_keeps_both_orders() {
    for depth in DEPTHS {
        let mut s = strips(depth);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let p = json!({"data": swap2(), "fileName": "swap2.cube", "dataOrder": "bgr", "tableOrder": "bgr", "dither": false});
        s.execute("image.adjustments.colorLookup", p).unwrap();
        let d = s.active().unwrap();
        let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
        let sf = sm.smart_filters.last().unwrap();
        assert_eq!(sf.params["dataOrder"], "bgr");
        assert_eq!(sf.params["tableOrder"], "bgr");
        // The filter re-runs from its recorded params: BGR data + BGR table undoes the file's swap.
        let flat = photocraft_compose::flatten(&d.doc).into_px();
        let got: Vec<[f32; 3]> = flat.iter().take(STRIPS.len()).map(|p| [p[0] * 255.0, p[1] * 255.0, p[2] * 255.0]).collect();
        assert_strips(&got, |c| c, &format!("smart filter, {depth} bit"));
    }
}

/// Wrong values are refused with the key named, and nothing is applied.
#[test]
fn bad_order_values_are_errors_not_panics() {
    for (key, value) in [
        ("dataOrder", json!("grb")),
        ("dataOrder", json!("")),
        ("dataOrder", json!(1)),
        ("dataOrder", json!(true)),
        ("tableOrder", json!("RGBA")),
        ("tableOrder", json!(0.5)),
        ("tableOrder", json!(["rgb"])),
        ("tableOrder", json!({"order": "rgb"})),
    ] {
        let mut s = strips(8);
        let before = levels(&s);
        let mut p = json!({"data": swap2(), "fileName": "swap2.cube"});
        p[key] = value.clone();
        let e = s.execute("image.adjustments.colorLookup", p).unwrap_err();
        assert!(e.to_string().contains(key), "{key}={value}: {e}");
        assert_eq!(levels(&s), before, "{key}={value} changed pixels");
        // Without a LUT the value is still checked.
        assert!(s.execute("image.adjustments.colorLookup", json!({key: value})).is_err(), "{key}={value} without a LUT");
    }
}
