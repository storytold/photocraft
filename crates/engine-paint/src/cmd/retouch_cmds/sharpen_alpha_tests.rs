//! #12940: with Protect Detail off, the Sharpen tool sharpens a layer's transparency by the same
//! rule as its colour, and keeps the colour. The reference test layer is (200,40,40) at alpha 64
//! for x 300..399 and alpha 191 for x 400..499, y 300..349, clicked once at 400,320 with a 40 px
//! hard tip at Strength 100 %.
//!
//! The reference's exact alpha (40 at x 399, 215 at x 400, x 398 and 401 unchanged) comes from the
//! 8-neighbour kernel of #12939; until that lands, alpha follows today's colour kernel.

use serde_json::json;

use super::*;

const RED: [f32; 3] = [200.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0];

/// An 800 × 600 RGB document of `depth` whose active layer is `f(x, y)` (straight RGBA, 0..1).
fn doc(depth: u64, transparent: bool, f: impl Fn(i32, i32) -> [f32; 4]) -> Session {
    let mut s = crate::session();
    let mut p = json!({"width": 800, "height": 600, "depth": depth, "mode": "rgb"});
    if transparent {
        p["background"] = json!("transparent");
    }
    s.execute("file.new", p).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// The reference layer: red at alpha 64 | 191, transparent elsewhere.
fn steps(depth: u64) -> Session {
    doc(depth, true, |x, y| {
        let a = match (x, y) {
            (300..=399, 300..=349) => 64.0,
            (400..=499, 300..=349) => 191.0,
            _ => 0.0,
        };
        [RED[0], RED[1], RED[2], a / 255.0]
    })
}

fn click(s: &mut Session, at: [i32; 2], extra: Value) {
    let mut p = json!({"points": [at], "size": 40, "hardness": 100, "strength": 100, "protectDetail": false});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("paint.sharpen", p).unwrap();
}

/// The active layer's pixel as 8-bit RGBA.
fn px(s: &Session, x: i32, y: i32) -> [i32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y).map(|v| (v * 255.0).round() as i32)
}

fn row(s: &Session, xs: std::ops::Range<i32>, y: i32) -> Vec<[i32; 4]> {
    xs.map(|x| px(s, x, y)).collect()
}

#[test]
fn a_click_sharpens_the_alpha_boundary_and_keeps_the_colour() {
    let mut s = steps(8);
    click(&mut s, [400, 320], json!({}));
    let got = row(&s, 397..403, 320);
    for p in &got {
        assert!((p[0] - 200).abs() <= 1 && (p[1] - 40).abs() <= 1 && (p[2] - 40).abs() <= 1, "colour must stay 200,40,40: {got:?}");
    }
    let alpha: Vec<i32> = got.iter().map(|p| p[3]).collect();
    // The low side goes down and the high side up, as with colour (the reference: 64 · 64 · 40 ·
    // 215 · 191 · 191 with #12939's kernel). Pinned from today's sigma-1 kernel.
    assert!(alpha[2] < 64 && alpha[3] > 191, "alpha boundary not sharpened: {alpha:?}");
    assert_eq!(alpha, PINNED_ALPHA, "alpha x 397..402");
}

/// Alpha at x 397..402 after the reference click, with today's kernel.
const PINNED_ALPHA: [i32; 6] = [63, 57, 26, 229, 198, 192];

#[test]
fn opaque_next_to_transparent_stays_as_it_is() {
    // A red square on an otherwise empty layer; a click on its left edge.
    let mut s = doc(8, true, |x, y| {
        let a = if (100..200).contains(&x) && (100..200).contains(&y) { 1.0 } else { 0.0 };
        [RED[0], RED[1], RED[2], a]
    });
    let before = row(&s, 80..120, 150);
    click(&mut s, [100, 150], json!({}));
    let after = row(&s, 80..120, 150);
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(b[3], a[3], "alpha changed: {before:?} -> {after:?}");
        if b[3] == 255 {
            assert_eq!(b, a, "an opaque pixel changed: {before:?} -> {after:?}");
        }
    }
}

#[test]
fn an_opaque_layer_keeps_todays_result() {
    // The epic's grey 96 | 160 step on the Background: pinned from main 038d035110.
    let mut s = doc(8, false, |x, _| {
        let g = if x < 400 { 96.0 } else { 160.0 } / 255.0;
        [g, g, g, 1.0]
    });
    click(&mut s, [400, 320], json!({}));
    let grey: Vec<i32> = row(&s, 396..404, 320).iter().map(|p| p[0]).collect();
    assert_eq!(grey, [96, 96, 92, 77, 179, 164, 160, 160]);
}

#[test]
fn lock_transparent_pixels_keeps_the_alpha() {
    let mut s = steps(8);
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    let before = row(&s, 390..410, 320);
    click(&mut s, [400, 320], json!({}));
    let after = row(&s, 390..410, 320);
    assert_eq!(before.iter().map(|p| p[3]).collect::<Vec<_>>(), after.iter().map(|p| p[3]).collect::<Vec<_>>());
}

#[test]
fn protect_detail_keeps_the_alpha() {
    // Protect Detail on across a semi-transparent boundary was not measured: alpha stays.
    let mut s = steps(8);
    let before = row(&s, 390..410, 320);
    click(&mut s, [400, 320], json!({"protectDetail": true}));
    let after = row(&s, 390..410, 320);
    assert_eq!(before.iter().map(|p| p[3]).collect::<Vec<_>>(), after.iter().map(|p| p[3]).collect::<Vec<_>>());
}

#[test]
fn sixteen_and_32_bit_keep_the_alpha() {
    // Alpha at 16 and 32 bit was not measured: it stays as it was.
    for depth in [16, 32] {
        let mut s = steps(depth);
        let before = row(&s, 390..410, 320);
        click(&mut s, [400, 320], json!({}));
        let after = row(&s, 390..410, 320);
        assert_eq!(before.iter().map(|p| p[3]).collect::<Vec<_>>(), after.iter().map(|p| p[3]).collect::<Vec<_>>(), "{depth}-bit");
    }
}

#[test]
fn blur_is_unchanged() {
    // Blur already blurred alpha; this change is Sharpen's alone.
    let mut s = steps(16);
    s.execute("paint.blur", json!({"points": [[400, 320]], "size": 40, "hardness": 100, "strength": 100})).unwrap();
    let alpha: Vec<i32> = row(&s, 397..403, 320).iter().map(|p| p[3]).collect();
    assert!(alpha[2] > 64 && alpha[3] < 191, "16-bit Blur still blurs alpha: {alpha:?}");
}
