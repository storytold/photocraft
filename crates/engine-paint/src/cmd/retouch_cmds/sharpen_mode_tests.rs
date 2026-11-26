//! #12943: `paint.sharpen` takes the options bar's Mode (Normal, Darken, Lighten, Hue, Saturation,
//! Color, Luminosity) and combines the sharpened value with the pixel under it the way a paint
//! colour would be.
//!
//! Every check here is relative: per-channel order against the same stroke in Normal, against the
//! pixel the stroke started from, and the identity a neutral step has under the component modes. The
//! reference application's measured pixel values are deliberately not in this file: a number read
//! off another app's output needs its own decision (standards/clean-room.md, "Observing a
//! proprietary app", point 3), so the exact rows stay a follow-up on #12943.

use serde_json::json;

use super::*;

const MODES: [&str; 7] = ["normal", "darken", "lighten", "hue", "saturation", "color", "luminosity"];
/// The fixture's own two grey levels (both neutral: r == g == b).
const GREY_LOW: [i32; 3] = [96, 96, 96];
const GREY_HIGH: [i32; 3] = [160, 160, 160];
const RED: [i32; 3] = [200, 40, 40];
const BLUE: [i32; 3] = [40, 40, 200];
/// The column where the fixture's halves meet; a click with a 40 px hard tip sits on it, and these
/// four columns are inside the tip's full coverage.
const EDGE: i32 = 100;
const XS: [i32; 4] = [EDGE - 2, EDGE - 1, EDGE, EDGE + 1];
const Y: i32 = 30;

/// A 200 × 60 document of `depth` and `mode` whose left half is `left` and right half `right`.
fn two_tone(depth: u64, mode: &str, left: [i32; 3], right: [i32; 3]) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 200, "height": 60, "depth": depth, "mode": mode})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let c = if x < EDGE { left } else { right };
                data.extend(photocraft_raster::from_rgba(&fmt, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// One click on the boundary with the reference's settings: 40 px hard tip, Strength 100 %,
/// Protect Detail off; `mode` is the Mode the options bar sends (`None`: the param is absent).
fn stroke(mode: Option<&str>) -> Value {
    let mut p = json!({"points": [[EDGE, Y]], "size": 40, "hardness": 100, "strength": 100, "protectDetail": false});
    if let Some(mode) = mode {
        p["mode"] = json!(mode);
    }
    p
}

/// The 8-bit RGB of the active layer at a pixel.
fn rgb(s: &Session, x: i32, y: i32) -> [i32; 3] {
    let d = s.active().unwrap();
    let c = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as i32)
}

/// Every pixel of the active layer.
fn image(s: &Session) -> Vec<[i32; 3]> {
    (0..60).flat_map(|y| (0..200).map(move |x| (x, y))).map(|(x, y)| rgb(s, x, y)).collect()
}

/// The document after one Sharpen click in `mode`.
fn sharpened(depth: u64, colour: &str, left: [i32; 3], right: [i32; 3], mode: Option<&str>) -> Session {
    let mut s = two_tone(depth, colour, left, right);
    s.execute("paint.sharpen", stroke(mode)).unwrap_or_else(|e| panic!("{mode:?} on {depth}-bit {colour}: {e}"));
    s
}

/// Per-channel `a <= b`.
fn darker_or_equal(a: [i32; 3], b: [i32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| *x <= y)
}

/// Per-channel `a >= b`.
fn lighter_or_equal(a: [i32; 3], b: [i32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| *x >= y)
}

fn near(got: [i32; 3], want: [i32; 3], tol: i32) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol)
}

/// The reference's luminance and saturation of an 8-bit triple, as the component modes use them.
fn lum(c: [i32; 3]) -> f32 {
    0.3 * c[0] as f32 + 0.59 * c[1] as f32 + 0.11 * c[2] as f32
}

fn sat(c: [i32; 3]) -> i32 {
    *c.iter().max().unwrap() - *c.iter().min().unwrap()
}

#[test]
fn mode_takes_the_seven_names_and_refuses_anything_else() {
    for mode in MODES {
        let s = sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some(mode));
        // The options bar sends the capitalised label.
        let label = format!("{}{}", mode[..1].to_uppercase(), &mode[1..]);
        assert_eq!(image(&s), image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some(&label))), "{mode} and {label} are the same mode");
    }
    for bad in [json!("multiply"), json!("screen"), json!("dissolve"), json!("clear"), json!("bogus"), json!(""), json!(3), json!(null)] {
        let mut s = two_tone(8, "rgb", RED, BLUE);
        let (before, steps) = (image(&s), s.active().unwrap().history.past_len());
        let mut p = stroke(None);
        p["mode"] = bad.clone();
        let err = s.execute("paint.sharpen", p.clone()).unwrap_err().to_string();
        assert!(err.contains("unknown mode") && err.contains("normal|darken|lighten|hue|saturation|color|luminosity"), "{bad}: {err}");
        assert_eq!(image(&s), before, "{bad}: nothing sharpened");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{bad}: no history step");
        assert!(LiveDab::begin(&s, "paint.sharpen", &p).is_err(), "{bad}: the live stroke is refused too");
    }
}

#[test]
fn no_mode_and_normal_are_todays_sharpen() {
    let plain = image(&sharpened(8, "rgb", RED, BLUE, None));
    assert_eq!(plain, image(&sharpened(8, "rgb", RED, BLUE, Some("normal"))), "mode normal is today's stroke");
    assert_ne!(plain, image(&two_tone(8, "rgb", RED, BLUE)), "the click does sharpen");
    assert_eq!(
        image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, None)),
        image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("normal"))),
        "mode normal is today's stroke on a grey step"
    );
}

#[test]
fn the_step_overshoots_in_normal_and_the_mode_stops_the_wrong_side() {
    let normal = sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("normal"));
    // The overshoot the Mode works against: darker than the low grey on its side, brighter than the
    // high grey on its own.
    assert!(darker_or_equal(rgb(&normal, EDGE - 1, Y), [95, 95, 95]), "Normal darkens the low side of the step: {:?}", rgb(&normal, EDGE - 1, Y));
    assert!(lighter_or_equal(rgb(&normal, EDGE, Y), [161, 161, 161]), "Normal brightens the high side of the step: {:?}", rgb(&normal, EDGE, Y));
    // Darken keeps only the darker channels, so the brightened column stays as it was; Lighten is
    // the mirror image.
    let darken = rgb(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("darken")), EDGE, Y);
    assert_eq!(darken, GREY_HIGH, "Darken leaves the brightened column of the step unchanged");
    let lighten = rgb(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("lighten")), EDGE - 1, Y);
    assert_eq!(lighten, GREY_LOW, "Lighten leaves the darkened column of the step unchanged");
    // Each keeps the change on its own side of the step, exactly as Normal writes it there.
    let d = sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("darken"));
    let l = sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("lighten"));
    assert_eq!(rgb(&d, EDGE - 1, Y), rgb(&normal, EDGE - 1, Y), "Darken keeps the low side's darkening");
    assert_eq!(rgb(&l, EDGE, Y), rgb(&normal, EDGE, Y), "Lighten keeps the high side's brightening");
    // Further in, where the step is flat and Normal changes nothing, neither Mode does either.
    for x in [EDGE - 20, EDGE + 20] {
        assert_eq!(rgb(&d, x, Y), rgb(&normal, x, Y), "darken at {x}");
        assert_eq!(rgb(&l, x, Y), rgb(&normal, x, Y), "lighten at {x}");
    }
}

#[test]
fn darken_is_never_lighter_than_normal_and_lighten_never_darker() {
    for (left, right) in [(GREY_LOW, GREY_HIGH), (RED, BLUE)] {
        let before = image(&two_tone(8, "rgb", left, right));
        let normal = image(&sharpened(8, "rgb", left, right, Some("normal")));
        let darken = image(&sharpened(8, "rgb", left, right, Some("darken")));
        let lighten = image(&sharpened(8, "rgb", left, right, Some("lighten")));
        for (((d, l), n), o) in darken.iter().zip(&lighten).zip(&normal).zip(&before) {
            assert!(darker_or_equal(*d, *n) && darker_or_equal(*d, *o), "Darken never lightens a channel: {d:?} vs Normal {n:?}, original {o:?}");
            assert!(lighter_or_equal(*l, *n) && lighter_or_equal(*l, *o), "Lighten never darkens a channel: {l:?} vs Normal {n:?}, original {o:?}");
        }
        assert_ne!(darken, normal, "Darken does something on this fixture");
        assert_ne!(lighten, normal, "Lighten does something on this fixture");
        // Inside the tip's full coverage each column is the per-channel minimum / maximum of the
        // Normal pixel and the pixel the stroke started from.
        for x in XS {
            let i = usize::try_from(Y * 200 + x).unwrap();
            let (n, o) = (normal[i], before[i]);
            let want = [n[0].min(o[0]), n[1].min(o[1]), n[2].min(o[2])];
            assert!(near(darken[i], want, 2), "darken at {x}: {:?}, the minimum is {want:?}", darken[i]);
            let want = [n[0].max(o[0]), n[1].max(o[1]), n[2].max(o[2])];
            assert!(near(lighten[i], want, 2), "lighten at {x}: {:?}, the maximum is {want:?}", lighten[i]);
        }
    }
}

#[test]
fn a_grey_step_is_unchanged_under_hue_saturation_and_color() {
    let before = image(&two_tone(8, "rgb", GREY_LOW, GREY_HIGH));
    for mode in ["hue", "saturation", "color"] {
        assert_eq!(image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some(mode))), before, "{mode} leaves a neutral step as it is");
    }
    assert_ne!(image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("normal"))), before, "Normal does change the step");
    assert_ne!(image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("luminosity"))), before, "Luminosity changes it, as the reference does");
}

#[test]
fn luminosity_on_a_grey_step_is_normal() {
    let normal = image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("normal")));
    assert_eq!(image(&sharpened(8, "rgb", GREY_LOW, GREY_HIGH, Some("luminosity"))), normal, "a neutral source has no hue or saturation to keep");
}

#[test]
fn the_component_modes_take_that_component_from_the_sharpened_value() {
    let before = two_tone(8, "rgb", RED, BLUE);
    let normal = sharpened(8, "rgb", RED, BLUE, Some("normal"));
    for x in XS {
        let (o, n) = (rgb(&before, x, Y), rgb(&normal, x, Y));
        // Hue, Saturation and Color keep the pixel's own luminosity and take that component of the
        // sharpened value; Hue also keeps the pixel's saturation.
        for mode in ["hue", "saturation", "color"] {
            let got = rgb(&sharpened(8, "rgb", RED, BLUE, Some(mode)), x, Y);
            assert!((lum(got) - lum(o)).abs() <= 3.0, "{mode} at {x}: the luminosity of {got:?} is not the pixel's {o:?}");
        }
        let got = rgb(&sharpened(8, "rgb", RED, BLUE, Some("hue")), x, Y);
        assert!((sat(got) - sat(o)).abs() <= 2, "hue at {x}: the saturation of {got:?} is not the pixel's {o:?}");
        // Luminosity is the mirror image: the pixel's hue and saturation, the sharpened value's
        // luminosity - which is where Normal moves the pixel to.
        let got = rgb(&sharpened(8, "rgb", RED, BLUE, Some("luminosity")), x, Y);
        assert!((sat(got) - sat(o)).abs() <= 2, "luminosity at {x}: the saturation of {got:?} is not the pixel's {o:?}");
        assert!((lum(got) - lum(n)).abs() <= 3.0, "luminosity at {x}: {got:?} does not reach the luminosity Normal gives {n:?}");
    }
    // The component modes do change pixels the way Normal does not: the edge is sharpened differently.
    assert_ne!(image(&sharpened(8, "rgb", RED, BLUE, Some("color"))), image(&normal), "Color is not Normal");
    assert_ne!(image(&sharpened(8, "rgb", RED, BLUE, Some("hue"))), image(&normal), "Hue is not Normal");
}

#[test]
fn unmeasured_colour_modes_and_depths_keep_todays_sharpen() {
    for (depth, colour) in [(16, "rgb"), (32, "rgb"), (8, "gray"), (8, "cmyk")] {
        let plain = image(&sharpened(depth, colour, RED, BLUE, None));
        assert_ne!(plain, image(&two_tone(depth, colour, RED, BLUE)), "{depth}-bit {colour}: sharpened");
        for mode in ["darken", "hue", "luminosity"] {
            assert_eq!(image(&sharpened(depth, colour, RED, BLUE, Some(mode))), plain, "{depth}-bit {colour} {mode}: today's sharpening");
        }
    }
}

#[test]
fn one_click_with_protect_detail_on_keeps_the_same_ordering() {
    // Protect Detail with a Mode is not measured: the two Modes still only keep the one dab's
    // sharpened value on the side it lies on.
    let click = |mode: Option<&str>| {
        let mut s = two_tone(8, "rgb", RED, BLUE);
        let mut p = json!({"points": [[EDGE, Y]], "size": 40, "hardness": 100, "strength": 100, "protectDetail": true});
        if let Some(mode) = mode {
            p["mode"] = json!(mode);
        }
        s.execute("paint.sharpen", p).unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        (image(&two_tone(8, "rgb", RED, BLUE)), image(&s))
    };
    let (before, normal) = click(None);
    assert_ne!(normal, before, "the click sharpens with Protect Detail on");
    let (_, darken) = click(Some("darken"));
    let (_, lighten) = click(Some("lighten"));
    for ((d, l), (n, o)) in darken.iter().zip(&lighten).zip(normal.iter().zip(&before)) {
        assert!(darker_or_equal(*d, *n) && darker_or_equal(*d, *o), "Darken at Protect Detail on: {d:?} vs Normal {n:?}, original {o:?}");
        assert!(lighter_or_equal(*l, *n) && lighter_or_equal(*l, *o), "Lighten at Protect Detail on: {l:?} vs Normal {n:?}, original {o:?}");
    }
    assert_ne!(darken, normal, "Darken does something");
    assert_ne!(lighten, normal, "Lighten does something");
    assert_eq!(click(Some("normal")).1, normal, "normal is the tool's own stroke");
}

#[test]
fn a_drag_stays_on_its_own_side_of_the_pixel_it_started_from() {
    // A drag is not measured either. Each dab blends with what the previous ones left, so the
    // bound along a drag is against the pixel before the stroke, not against the same drag in
    // Normal: Darken never leaves a channel above where it started, Lighten never below.
    let drag = |mode: Option<&str>| {
        let mut s = two_tone(8, "rgb", RED, BLUE);
        let mut p = json!({"points": [[80, Y], [120, Y]], "size": 40, "hardness": 100, "strength": 100, "protectDetail": false});
        if let Some(mode) = mode {
            p["mode"] = json!(mode);
        }
        s.execute("paint.sharpen", p).unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        image(&s)
    };
    let before = image(&two_tone(8, "rgb", RED, BLUE));
    let normal = drag(None);
    assert_ne!(normal, before, "the drag sharpens");
    let (darken, lighten) = (drag(Some("darken")), drag(Some("lighten")));
    for ((d, l), o) in darken.iter().zip(&lighten).zip(&before) {
        assert!(darker_or_equal(*d, *o), "Darken over a drag lightens a channel: {d:?} from {o:?}");
        assert!(lighter_or_equal(*l, *o), "Lighten over a drag darkens a channel: {l:?} from {o:?}");
    }
    assert_ne!(darken, before, "Darken sharpens where the value is darker");
    assert_ne!(lighten, before, "Lighten sharpens where the value is lighter");
    assert_eq!(drag(Some("normal")), normal, "normal is the tool's own drag");
}

#[test]
fn the_live_stroke_previews_the_mode() {
    // The canvas draws the stroke with the tool it commits, parsed from the same params.
    let s = two_tone(8, "rgb", RED, BLUE);
    let parsed = |p: &Value| DabTool::parse(&s, "paint.sharpen", p, &parse_brush(&s, p, "paint.sharpen").unwrap().0).unwrap().0;
    for mode in ["darken", "hue"] {
        let p = stroke(Some(mode));
        assert_ne!(parsed(&p), parsed(&stroke(None)), "{mode} is part of the tool");
        assert!(LiveDab::begin(&s, "paint.sharpen", &p).is_ok(), "{mode}: live");
    }
    assert_eq!(parsed(&stroke(Some("normal"))), parsed(&stroke(None)));
}

#[test]
fn blur_and_sponge_keep_their_own_params() {
    // Neighbour cases: #12642 owns the Blur tool's Mode, and Sponge already has its own `mode`.
    for cmd in ["paint.blur", "paint.sponge"] {
        let mut plain = two_tone(8, "rgb", RED, BLUE);
        plain.execute(cmd, stroke(None)).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        let mut with = two_tone(8, "rgb", RED, BLUE);
        let got = with.execute(cmd, stroke(Some("darken")));
        if cmd == "paint.blur" {
            assert_eq!(image(&with), image(&plain), "{cmd} keeps ignoring a mode until #12642 lands");
        } else {
            assert!(got.is_err(), "{cmd} keeps refusing a Painting Mode: {got:?}");
        }
    }
}
