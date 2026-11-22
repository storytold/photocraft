//! Burn in a 32-bit document tones the linear values it holds, with the reference's curve
//! (#12674); 8- and 16-bit documents keep their results.

use crate::Session;
use photocraft_color::convert::linear_to_srgb;
use serde_json::{Value, json};

/// The reference's test colours (8-bit RGB): grey 32, 128 and 224, red, skin and pastel.
const PATCHES: [[u8; 3]; 6] = [[32, 32, 32], [128, 128, 128], [224, 224, 224], [200, 40, 40], [220, 170, 140], [180, 200, 230]];
const SIDE: i32 = 60;

/// An 8-bit RGB document of `PATCHES` side by side, converted to `depth` bits.
fn patches(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": SIDE * PATCHES.len() as i32, "height": SIDE, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("patches", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _ in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let c = PATCHES[(x / SIDE) as usize].map(|v| f32::from(v) / 255.0);
                data.extend(photocraft_raster::from_rgba(&fmt, [c[0], c[1], c[2], 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    if depth != 8 {
        s.execute(&format!("image.mode.bits{depth}"), json!({})).unwrap();
    }
    s
}

fn rgb(s: &Session, x: i32, y: i32) -> [f32; 3] {
    let d = s.active().unwrap();
    let p = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [p[0], p[1], p[2]]
}

/// One default Burn click (Midtones, Protect Tones, 50 %) with a hard 40 px tip on each patch's
/// centre; the stored values there.
fn burn_each(s: &mut Session, extra: Value) -> Vec<[f32; 3]> {
    (0..PATCHES.len() as i32)
        .map(|i| {
            let (x, y) = (i * SIDE + SIDE / 2, SIDE / 2);
            let mut p = json!({"points": [[x, y]], "size": 40, "hardness": 100});
            if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
                p.extend(e.clone());
            }
            s.execute("paint.burn", p).unwrap();
            rgb(s, x, y)
        })
        .collect()
}

/// A 32-bit document's linear values as 8-bit RGB, as the reference reads them.
fn as_8bit(c: [f32; 3]) -> [i32; 3] {
    c.map(|v| (linear_to_srgb(v).clamp(0.0, 1.0) * 255.0).round() as i32)
}

#[test]
fn burn_tones_linear_values_in_32_bit() {
    // Measured in the reference: one default click in a 32-bit RGB document, read as 8-bit.
    let expected = [[32, 32, 32], [122, 122, 122], [217, 217, 217], [189, 40, 40], [212, 162, 132], [168, 192, 225]];
    let mut s = patches(32);
    let got: Vec<_> = burn_each(&mut s, json!({})).into_iter().map(as_8bit).collect();
    for ((g, e), p) in got.iter().zip(expected).zip(PATCHES) {
        assert!(g.iter().zip(e).all(|(g, e)| (g - e).abs() <= 2), "32-bit Burn of {p:?}: got {g:?}, expected {e:?} (+-2); all: {got:?}");
    }
}

#[test]
fn burn_keeps_8_and_16_bit_results() {
    // Pinned from main 406ac677e1: this change only adds the 32-bit path.
    let mut s = patches(8);
    let got: Vec<_> = burn_each(&mut s, json!({})).into_iter().map(|c| c.map(|v| (v * 255.0).round() as u16)).collect();
    assert_eq!(got, [[23, 23, 23], [91, 91, 91], [218, 218, 218], [125, 25, 25], [192, 148, 122], [165, 183, 211]]);
    let mut s = patches(16);
    let got: Vec<_> = burn_each(&mut s, json!({})).into_iter().map(|c| c.map(|v| (v * 65535.0).round() as u16)).collect();
    assert_eq!(got, [[5893, 5893, 5893], [23307, 23307, 23307], [56025, 56025, 56025], [32222, 6444, 6444], [49291, 38089, 31367], [42377, 47085, 54148]]);
}

#[test]
fn burn_keeps_light_above_white_in_32_bit() {
    let mut s = patches(32);
    s.edit("bright", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let r = photocraft_geom::Rect::new(0, 0, SIDE, SIDE);
        let px = photocraft_raster::from_rgba(&surf.format(), [4.0, 1.5, 0.2, 1.0]);
        surf.write_region(r, &px.repeat((SIDE * SIDE) as usize));
        Ok(())
    })
    .unwrap();
    let c = burn_each(&mut s, json!({}))[0];
    assert_eq!([c[0], c[1]], [4.0, 1.5], "light above white must stay as it was: {c:?}");
    assert!(c[2] < 0.2, "the in-range channel is still burned: {c:?}");
}

#[test]
fn burn_keeps_tones_in_order_in_32_bit_at_full_exposure() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 256, "height": 8, "depth": 32, "mode": "rgb"})).unwrap();
    s.edit("ramp", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let data: Vec<f32> = (0..8).flat_map(|_| (0..256).flat_map(|x| photocraft_raster::from_rgba(&fmt, [x as f32 / 255.0; 4]))).collect();
        surf.write_region(photocraft_geom::Rect::new(0, 0, 256, 8), &data);
        Ok(())
    })
    .unwrap();
    s.execute("paint.burn", json!({"points": [[-20, 4], [276, 4]], "size": 40, "hardness": 100, "exposure": 100})).unwrap();
    let row: Vec<f32> = (0..256).map(|x| rgb(&s, x, 4)[0]).collect();
    assert!(row.windows(2).all(|w| w[1] >= w[0]), "{row:?}");
    assert!(row.iter().all(|v| (0.0..=1.0).contains(v)) && row[0] == 0.0 && row[255] == 1.0, "{row:?}");
}
