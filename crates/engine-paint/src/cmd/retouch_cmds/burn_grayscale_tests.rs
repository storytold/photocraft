//! Burn in an 8-bit Grayscale document and on a layer mask uses the reference's single-channel
//! curve (#12675); RGB documents and the cases the reference wasn't measured in keep the RGB
//! curve. The helpers are shared with `dodge_grayscale_tests`.

use super::LiveDab;
use crate::Session;
use photocraft_algo::retouch::{ToneRange, dodge_burn, single_channel_midtones};
use photocraft_raster::Surface;
use serde_json::{Value, json};

/// The greys the reference was measured on.
pub(super) const GREYS: [u8; 6] = [32, 105, 128, 180, 197, 224];
const SIDE: i32 = 60;

/// Fill `surf` with the `GREYS` bands, side by side.
fn bands(surf: &mut Surface, b: photocraft_geom::Rect) {
    let fmt = surf.format();
    let mut data = Vec::new();
    for _ in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let v = f32::from(GREYS[(x / SIDE) as usize]) / 255.0;
            data.extend(photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
        }
    }
    surf.write_region(b, &data);
}

/// A `mode` (`"gray"` or `"rgb"`) document of `depth` bits whose layer holds the `GREYS` bands.
pub(super) fn doc(mode: &str, depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": SIDE * GREYS.len() as i32, "height": SIDE, "depth": depth, "mode": mode, "background": "white"})).unwrap();
    s.edit("bands", |doc, active| {
        let b = doc.bounds();
        bands(doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap(), b);
        Ok(())
    })
    .unwrap();
    s
}

/// The reference's mask setup in a `depth`-bit RGB document: a white pixel layer over a black one,
/// with a layer mask holding the `GREYS` bands.
pub(super) fn masked(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute(
        "file.new",
        json!({"width": SIDE * GREYS.len() as i32, "height": SIDE, "depth": depth, "background": "backgroundColor", "backgroundColor": [0.0, 0.0, 0.0]}),
    )
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.edit("bands", |doc, active| {
        let b = doc.bounds();
        let layer = doc.layer_mut(active.unwrap()).unwrap();
        let surf = layer.surface_mut().unwrap();
        let white = photocraft_raster::from_rgba(&surf.format(), [1.0; 4]);
        surf.write_region(b, &white.repeat((b.width() * b.height()) as usize));
        bands(&mut layer.mask.as_mut().unwrap().surface, b);
        Ok(())
    })
    .unwrap();
    s
}

/// The stored value (0..255) the target of `extra` holds at `(x, y)`.
fn read(s: &Session, extra: &Value, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    let surf = match extra.get("target") {
        Some(Value::String(t)) if t == "mask" => &d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface,
        Some(Value::String(t)) if t == "quickMask" => &d.doc.quick_mask.as_ref().unwrap().surface,
        Some(Value::Object(_)) => &d.doc.channels[0].surface,
        _ => d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap(),
    };
    surf.rgba(x, y)[0] * 255.0
}

/// One `cmd` click (40 px, hardness 100, plus `extra`) on each band's centre; the values there.
pub(super) fn click_each(s: &mut Session, cmd: &str, extra: &Value) -> Vec<f32> {
    (0..GREYS.len() as i32)
        .map(|i| {
            let (x, y) = (i * SIDE + SIDE / 2, SIDE / 2);
            let mut p = json!({"points": [[x, y]], "size": 40, "hardness": 100});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            s.execute(cmd, p).unwrap();
            read(s, extra, x, y)
        })
        .collect()
}

pub(super) fn assert_near(got: &[f32], want: &[f32], tol: f32, what: &str) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol), "{what}: got {got:?}, expected {want:?} (±{tol})");
}

/// Today's RGB curve for the `GREYS`, as 0..255.
pub(super) fn rgb_curve(e: f32, range: ToneRange, burn: bool, protect: bool) -> Vec<f32> {
    GREYS.iter().map(|&g| dodge_burn([f32::from(g) / 255.0; 3], e, range, burn, protect)[0] * 255.0).collect()
}

/// The cases the reference wasn't measured in keep the RGB curve: 16- and 32-bit Grayscale, a
/// mask in a 16- or 32-bit document, the other ranges and exposures, Protect Tones off, an alpha
/// channel and the Quick Mask.
pub(super) fn keeps_rgb_curve_where_not_measured(cmd: &str, burn: bool) {
    let mid = |e, protect| rgb_curve(e, ToneRange::Midtones, burn, protect);
    for depth in [16, 32] {
        let tol = if depth == 16 { 0.05 } else { 0.01 };
        assert_near(&click_each(&mut doc("gray", depth), cmd, &json!({})), &mid(0.5, true), tol, &format!("{cmd} gray {depth}-bit"));
        assert_near(&click_each(&mut masked(depth), cmd, &json!({"target": "mask"})), &mid(0.5, true), tol, &format!("{cmd} mask {depth}-bit"));
    }
    let cases = [
        (json!({"range": "shadows"}), rgb_curve(0.5, ToneRange::Shadows, burn, true)),
        (json!({"range": "highlights"}), rgb_curve(0.5, ToneRange::Highlights, burn, true)),
        (json!({"exposure": 100}), mid(1.0, true)),
        (json!({"exposure": 49}), mid(0.49, true)),
        (json!({"protectTones": false}), mid(0.5, false)),
    ];
    for (extra, want) in cases {
        // A rounding step: the 8-bit document stores whole levels.
        let want: Vec<f32> = want.iter().map(|v| v.round()).collect();
        assert_near(&click_each(&mut doc("gray", 8), cmd, &extra), &want, 0.51, &format!("{cmd} gray 8-bit {extra}"));
    }
    let want: Vec<f32> = mid(0.5, true).iter().map(|v| v.round()).collect();
    let mut s = doc("gray", 8);
    s.execute("channel.new", json!({})).unwrap();
    s.edit("bands", |doc, _| {
        let b = doc.bounds();
        bands(&mut doc.channels[0].surface, b);
        Ok(())
    })
    .unwrap();
    assert_near(&click_each(&mut s, cmd, &json!({"target": {"channel": 0}})), &want, 0.51, &format!("{cmd} alpha channel"));
    let mut s = doc("gray", 8);
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    s.edit("bands", |doc, _| {
        let b = doc.bounds();
        bands(&mut doc.quick_mask.as_mut().unwrap().surface, b);
        Ok(())
    })
    .unwrap();
    assert_near(&click_each(&mut s, cmd, &json!({"target": "quickMask"})), &want, 0.51, &format!("{cmd} Quick Mask"));
}

/// The live stroke shows the same single-channel result its commit lays down, on a short drag
/// across the grey 128 band (the stroke's last dab shows on release, so the start is compared).
pub(super) fn live_matches_commit(cmd: &str) {
    let mut s = doc("gray", 8);
    s.tools.brush.smoothing.enabled = false;
    let (x, y) = (2 * SIDE + 15, SIDE / 2);
    let all = [[x, y], [x + 10, y], [x + 20, y]];
    let opts = |p: &[[i32; 2]]| json!({"points": p, "size": 20, "hardness": 100});
    let mut live = LiveDab::begin(&s, cmd, &opts(&all[..1])).unwrap();
    live.push(&all[1..].iter().map(|q| photocraft_paint::StrokePoint::new(f64::from(q[0]), f64::from(q[1]), 1.0)).collect::<Vec<_>>()).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.layer(id).unwrap().surface().unwrap().rgba(x, y)[0] * 255.0;
    s.execute(cmd, opts(&all)).unwrap();
    let committed = read(&s, &json!({}), x, y);
    assert_ne!(committed, 128.0, "{cmd}: the stroke toned the band");
    assert_eq!(shown, committed, "{cmd}: live against commit");
}

/// The single-channel curve keeps black and white, keeps tones in order and passes values outside
/// 0..1 through.
pub(super) fn curve_is_monotone_and_bounded(burn: bool) {
    let mut last = 0.0f32;
    for i in 0..=10_000 {
        let v = i as f32 / 10_000.0;
        let o = single_channel_midtones(v, burn);
        assert!(o.is_finite() && o >= last - 1e-6 && (0.0..=1.0).contains(&o), "burn={burn}: {v} → {o} after {last}");
        assert!(if burn { o <= v } else { o >= v }, "burn={burn}: {v} → {o}");
        last = o;
    }
    assert_eq!(single_channel_midtones(0.0, burn), 0.0);
    assert_eq!(single_channel_midtones(1.0, burn), 1.0);
    for v in [-0.5, 1.5, 4.0, f32::NAN] {
        let o = single_channel_midtones(v, burn);
        assert!(o == v || (v.is_nan() && o.is_nan()), "burn={burn}: {v} → {o}");
    }
}

/// Measured in the reference: one default Burn click in an 8-bit Grayscale document.
const REFERENCE: [f32; 6] = [25.0, 92.0, 115.0, 170.0, 188.0, 219.0];

#[test]
fn burn_in_8_bit_grayscale_matches_the_reference() {
    let got = click_each(&mut doc("gray", 8), "paint.burn", &json!({}));
    assert_near(&got, &REFERENCE, 2.0, "Burn, 8-bit Grayscale");
}

#[test]
fn burn_on_a_layer_mask_matches_grayscale() {
    let mut s = masked(8);
    let before = s.active().unwrap().history.past_len();
    let got = click_each(&mut s, "paint.burn", &json!({"target": "mask"}));
    assert_near(&got, &REFERENCE, 2.0, "Burn, layer mask");
    let h = &s.active().unwrap().history;
    assert_eq!((h.past_len(), h.undo_label()), (before + GREYS.len(), Some("Burn Tool")));
}

#[test]
fn burn_keeps_rgb_results() {
    // Pinned from main d5582062a0: neutral greys and a colour in an 8-bit RGB document.
    let got = click_each(&mut doc("rgb", 8), "paint.burn", &json!({}));
    assert_near(&got, &[23.0, 69.0, 91.0, 156.0, 180.0, 218.0], 0.0, "Burn, RGB greys");
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 60, "height": 60, "background": "backgroundColor", "backgroundColor": [0.7843137, 0.15686275, 0.15686275]})).unwrap();
    s.execute("paint.burn", json!({"points": [[30, 30]], "size": 40, "hardness": 100})).unwrap();
    let d = s.active().unwrap();
    let px = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 30);
    assert_eq!([px[0], px[1], px[2]].map(|v| (v * 255.0).round()), [125.0, 25.0, 25.0]);
}

#[test]
fn burn_keeps_the_rgb_curve_where_the_reference_was_not_measured() {
    keeps_rgb_curve_where_not_measured("paint.burn", true);
}

#[test]
fn live_burn_in_grayscale_matches_the_commit() {
    live_matches_commit("paint.burn");
}

#[test]
fn single_channel_burn_curve_is_monotone_and_bounded() {
    curve_is_monotone_and_bounded(true);
}
