//! #14224: an untagged document is displayed in the working space of Edit > Color Settings, not
//! always in the mode's built-in default, and a change of that space refreshes an open document.
//!
//! The checks are relative: what the canvas must show is the transform our own engine builds for
//! the profile Color Settings names ([`ColorState::working`]) to the monitor the session resolves,
//! evaluated in this file. The reference application's absolute numbers for ProPhoto working RGB
//! are its profile's tables, so they are reported rather than asserted here.
//!
//! Untagged documents only: a tagged one keeps its embedded profile, and CMYK keeps today's
//! display (the issue leaves it unmeasured).

use super::*;
use photocraft_doc::Size;
use photocraft_geom::Rect;

use crate::display_color::{DISPLAY_BPC, DISPLAY_INTENT};

/// Tolerance of the CPU canvas path against the transform evaluated here (8-bit quantisation plus
/// table evaluation), as in `display_color/tests.rs`.
const TOL: f32 = 2.0;

/// An untagged RGB document: one 4x4 block per entry of `values`, in columns.
fn untagged_rgb(values: &[[f32; 3]], depth: SampleType) -> Document {
    let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut d = Document::new("t", Size::new(values.len() as u32 * 4, 4), ColorMode::Rgb, depth);
    let mut surf = Surface::new(fmt);
    for (i, v) in values.iter().enumerate() {
        let x = i as i32 * 4;
        surf.fill_rect(Rect::new(x, 0, x + 4, 4), &[v[0], v[1], v[2], 1.0]);
    }
    d.layers.push(Layer::new("pixels", LayerContent::Raster(surf)));
    d
}

/// An RGB document tagged with `tag`.
fn tagged_rgb(tag: &Profile, v: [f32; 3]) -> Document {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgb(v[0], v[1], v[2]));
    d.icc_profile = Some(tag.to_bytes());
    d
}

/// An untagged Grayscale document.
fn untagged_gray(v: f32) -> Document {
    Document::with_background("g", Size::new(8, 8), ColorMode::Grayscale, SampleType::U8, Color::gray(v))
}

/// A two-layer untagged CMYK document, so the composite goes through the compositor.
fn untagged_cmyk(ink: [f32; 4]) -> Document {
    let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true);
    let mut d = Document::new("c", Size::new(8, 8), ColorMode::Cmyk, SampleType::U8);
    let mut surf = Surface::new(fmt);
    surf.fill_rect(Rect::new(0, 0, 8, 8), &[ink[0], ink[1], ink[2], ink[3], 1.0]);
    d.layers.push(Layer::new("ink", LayerContent::Raster(surf)));
    d.layers.push(Layer::new("empty", LayerContent::Raster(Surface::new(fmt))));
    d
}

/// What the CPU canvas shows in the middle of column `i` of `d`, in 8-bit codes.
fn shown(s: &Session, d: &Document, i: usize) -> [f32; 3] {
    let buf = photocraft_compose::flatten(d);
    let img = s.color().canvas_display(d).unwrap().to_rgba8(&buf);
    let x = (i as u32 * 4 + 2).min(d.size.width - 1);
    let y = d.size.height / 2;
    let k = (y as usize * d.size.width as usize + x as usize) * 4;
    [0, 1, 2].map(|c| img.pixels[k + c] as f32)
}

/// The transform our engine builds from `src` to the session's monitor, evaluated at `v`: what the
/// canvas has to show. Returns 8-bit codes.
fn shown_want(s: &Session, src: &Profile, v: &[f32]) -> [f32; 3] {
    let monitor = s.color().monitor();
    let t = Transform::new(src, &monitor, DISPLAY_INTENT, DISPLAY_BPC).unwrap();
    let mut out = [0.0f32; 16];
    t.eval(v, &mut out).unwrap();
    let n = t.outputs();
    [0, 1, 2].map(|c| out[if n == 1 { 0 } else { c }].clamp(0.0, 1.0) * 255.0)
}

fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

fn far(a: [f32; 3], b: [f32; 3], d: f32) -> bool {
    (0..3).any(|i| (a[i] - b[i]).abs() >= d)
}

fn raw_codes(v: &[f32]) -> [f32; 3] {
    [0, 1, 2].map(|c| v[c] * 255.0)
}

/// The document's description as `edit.profileInfo` reports it.
fn reported(s: &mut Session) -> Value {
    s.execute("edit.profileInfo", json!({})).unwrap()["profile"].clone()
}

#[test]
fn an_untagged_rgb_document_is_displayed_in_the_working_space() {
    let mut s = crate::session();
    // The ticket's fixture: 200,30,30 and 30,60,200, profile removed.
    let values = [[200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0], [30.0 / 255.0, 60.0 / 255.0, 200.0 / 255.0]];
    let d = untagged_rgb(&values, SampleType::U8);
    // Unchanged with the default settings: sRGB working RGB means the values go to the screen.
    let first = s.color().canvas_display(&d).unwrap();
    assert_eq!(first.source.description, Builtin::Srgb.profile().description, "the default working space is sRGB");
    assert!(first.is_identity(), "sRGB on an sRGB monitor still costs nothing");
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let working = s.color().working(ColorMode::Rgb);
    assert_eq!(working.description, Builtin::ProPhotoCompat.profile().description, "the setting resolved");
    let cd = s.color().canvas_display(&d).unwrap();
    assert_eq!(cd.source.description, working.description, "an untagged document is shown in Color Settings' working RGB");
    for (i, v) in values.iter().enumerate() {
        let got = shown(&s, &d, i);
        let want = shown_want(&s, &working, v);
        assert!(close(got, want, TOL), "column {i}: canvas {got:?} vs the working space's transform {want:?}");
        assert!(far(got, raw_codes(v), 8.0), "column {i} is still shown as its raw values: {got:?}");
    }
    // The reference application's measured numbers for this fixture, tolerance 2 as the ticket
    // states them: they match what our own ProPhoto-compatible profile computes.
    assert!(close(shown(&s, &d, 0), [255.0, 0.0, 34.0], 2.0), "200,30,30: {:?}", shown(&s, &d, 0));
    assert!(close(shown(&s, &d, 1), [0.0, 82.0, 223.0], 2.0), "30,60,200: {:?}", shown(&s, &d, 1));
}

#[test]
fn changing_the_working_space_refreshes_an_open_untagged_document() {
    let mut s = crate::session();
    let v = [0.8, 0.12, 0.12];
    let d = untagged_rgb(&[v], SampleType::U8);
    let before = s.color().canvas_display(&d).unwrap();
    let signature = s.color().display_signature(&d);
    s.execute("edit.colorSettings", json!({"workingRgb": "adobe-rgb-compat"})).unwrap();
    let after = s.color().canvas_display(&d).unwrap();
    assert_eq!(after.source.description, s.color().working(ColorMode::Rgb).description);
    assert!(!Arc::ptr_eq(&before, &after), "the display cache handed back the entry made for the old working space");
    assert_ne!(before.key, after.key, "the cache entry and its signature have to move with the working space");
    assert_ne!(signature, s.color().display_signature(&d), "the canvas reloads the document without reopening it");
    assert_eq!(before.monitor.content_hash(), after.monitor.content_hash(), "only the working space moved");
    assert!(close(shown(&s, &d, 0), shown_want(&s, &after.source, &v), TOL), "{:?}", shown(&s, &d, 0));
    // Back to the default: the same entry as before, so the key follows the setting and not a counter.
    s.execute("edit.colorSettings", json!({"workingRgb": "srgb"})).unwrap();
    let back = s.color().canvas_display(&d).unwrap();
    assert!(back.is_identity());
    assert_eq!(back.key, before.key, "the same settings give the same display");
}

#[test]
fn an_untagged_document_stays_untagged_and_keeps_its_pixels() {
    let mut s = crate::session();
    let v = [200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];
    s.open_document(untagged_rgb(&[v], SampleType::U8), None);
    let before = reported(&mut s);
    assert_eq!(before["embedded"], json!(false));
    let sample = |s: &Session| s.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
    let pixels = sample(&s);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let after = reported(&mut s);
    assert_eq!(after["embedded"], json!(false), "the working space is not written into the document");
    assert_eq!(after["description"], Builtin::ProPhotoCompat.profile().description, "the document is taken in the working space");
    assert_eq!(sample(&s), pixels, "only the interpretation changes, not the stored samples");
    let d = &s.active().unwrap().doc;
    assert!(close(shown(&s, d, 0), shown_want(&s, &s.color().working(ColorMode::Rgb), &v), TOL), "{:?}", shown(&s, d, 0));
}

#[test]
fn a_tagged_document_keeps_its_embedded_profile() {
    let mut s = crate::session();
    let v = [0.4, 0.6, 0.2];
    for tag in [Builtin::Srgb, Builtin::AdobeRgbCompat, Builtin::DisplayP3] {
        let d = tagged_rgb(tag.profile(), v);
        let before = s.color().canvas_display(&d).unwrap();
        s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
        let cd = s.color().canvas_display(&d).unwrap();
        assert_eq!(cd.source.content_hash(), tag.profile().content_hash(), "{} was replaced by the working space", tag.description());
        assert_eq!(cd.key, before.key, "a tagged document's display does not move with the setting");
        assert!(close(shown(&s, &d, 0), shown_want(&s, tag.profile(), &v), TOL), "{}: {:?}", tag.description(), shown(&s, &d, 0));
        s.execute("edit.colorSettings", json!({"workingRgb": "srgb"})).unwrap();
    }
    // The command read-back of a tagged document is its own profile.
    s.open_document(tagged_rgb(Builtin::AdobeRgbCompat.profile(), v), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let info = reported(&mut s);
    assert_eq!(info["embedded"], json!(true));
    assert_eq!(info["description"], Builtin::AdobeRgbCompat.profile().description);
}

#[test]
fn an_untagged_grayscale_document_follows_the_working_gray() {
    let mut s = crate::session();
    let d = untagged_gray(0.3);
    let first = s.color().canvas_display(&d).unwrap();
    // The default working Gray is the built-in default, so nothing moves without a setting.
    assert_eq!(first.source.content_hash(), working_profile(ColorMode::Grayscale).gray_as_rgb().unwrap().content_hash());
    s.execute("edit.colorSettings", json!({"workingGray": "gray-gamma-2.2"})).unwrap();
    let working = s.color().working(ColorMode::Grayscale);
    assert_eq!(working.description, Builtin::GrayGamma22.profile().description, "the setting resolved");
    let cd = s.color().canvas_display(&d).unwrap();
    assert_eq!(cd.source.content_hash(), working.gray_as_rgb().unwrap().content_hash(), "an untagged gray document is shown in the working Gray");
    assert_ne!(cd.key, first.key, "and its display cache entry re-keys");
    assert!(close(shown(&s, &d, 0), shown_want(&s, &cd.source, &[0.3, 0.3, 0.3]), TOL), "{:?}", shown(&s, &d, 0));
}

#[test]
fn an_untagged_cmyk_document_keeps_todays_display() {
    let mut s = crate::session();
    let ink = [0.1, 0.6, 0.7, 0.05];
    let d = untagged_cmyk(ink);
    let before = shown(&s, &d, 0);
    let entry = s.color().canvas_display(&d).unwrap();
    s.execute("edit.colorSettings", json!({"workingCmyk": "coated-fogra39"})).unwrap();
    let cd = s.color().canvas_display(&d).unwrap();
    assert_eq!(cd.source.description, Builtin::Srgb.profile().description, "CMYK is not measured and keeps its sRGB composite");
    assert_eq!(cd.key, entry.key, "the working CMYK does not re-key an untagged CMYK document");
    assert!(close(shown(&s, &d, 0), before, 0.6), "its inks are still read through the built-in coated profile: {before:?} -> {:?}", shown(&s, &d, 0));
    // And the profile it is taken in stays the mode's built-in default even though the setting
    // now names another one: only RGB and Grayscale follow Color Settings here.
    assert_ne!(s.color().working(ColorMode::Cmyk).content_hash(), working_profile(ColorMode::Cmyk).content_hash(), "the setting really moved");
    assert_eq!(
        s.color().document_profile(&d).content_hash(),
        working_profile(ColorMode::Cmyk).content_hash(),
        "an untagged CMYK document keeps the mode default"
    );
}

#[test]
fn the_same_rule_holds_at_every_depth() {
    let mut s = crate::session();
    let v = [0.8, 0.12, 0.12];
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let working = s.color().working(ColorMode::Rgb);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = untagged_rgb(&[v], depth);
        let cd = s.color().canvas_display(&d).unwrap();
        assert_eq!(cd.source.description, working.description, "{depth:?}");
        let got = shown(&s, &d, 0);
        let want = shown_want(&s, &working, &v);
        let tol = if depth == SampleType::U8 { TOL } else { 1.0 };
        assert!(close(got, want, tol), "{depth:?}: {got:?} vs {want:?}");
    }
}

#[test]
fn a_conversion_starts_from_the_working_space() {
    let mut s = crate::session();
    let v = [200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];
    let filled = [v[0], v[1], v[2], 1.0];
    s.open_document(untagged_rgb(&[v], SampleType::U8), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let working = s.color().working(ColorMode::Rgb);
    let srgb = Builtin::Srgb.profile().clone();
    // What Convert to Profile has to write: the working space's own transform of the stored
    // samples, clamped the way the sample write clamps them (this colour is outside sRGB).
    let clamp3 = |o: &[f32; 16]| [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0)];
    let mut out = [0.0f32; 16];
    Transform::new(&working, &srgb, Intent::RelativeColorimetric, false).unwrap().eval(&v, &mut out).unwrap();
    let want = clamp3(&out);
    let mut built_in = [0.0f32; 16];
    let default = working_profile(ColorMode::Rgb);
    Transform::new(&default, &srgb, Intent::RelativeColorimetric, false).unwrap().eval(&v, &mut built_in).unwrap();
    assert!(
        want.iter().zip(clamp3(&built_in).iter()).any(|(a, b)| (a - b).abs() > 8.0 / 255.0),
        "the fixture must tell the working space apart from the built-in default: {want:?} vs {built_in:?}"
    );
    for dither in [false, true] {
        let r = s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false, "dither": dither})).unwrap();
        assert_eq!(r["converted"], json!(true), "dither {dither}");
        let px = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
        assert!((0..3).all(|c| (px[c] - want[c]).abs() <= 2.0 / 255.0), "dither {dither}: {px:?} vs the working space's own conversion {want:?}");
        assert_eq!(reported(&mut s)["description"], srgb.description, "tagged with the destination");
        // Back to the untagged start for the second round.
        s.execute("edit.assignProfile", json!({"profile": "none"})).unwrap();
        s.edit("reset", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &filled);
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn a_mode_change_starts_from_the_working_space() {
    let mut s = crate::session();
    let v = [0.8, 0.12, 0.12];
    s.open_document(untagged_rgb(&[v], SampleType::U8), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat", "workingGray": "gray-gamma-2.2"})).unwrap();
    let src = s.color().working(ColorMode::Rgb);
    let dst = s.color().working(ColorMode::Grayscale);
    let t = Transform::new(&src, &dst, Intent::RelativeColorimetric, false).unwrap();
    let from_built_in = {
        let built_in = working_profile(ColorMode::Rgb);
        Transform::new(&built_in, &dst, Intent::RelativeColorimetric, false).unwrap()
    };
    let mut out = [0.0f32; 16];
    let mut other = [0.0f32; 16];
    t.eval(&v, &mut out).unwrap();
    from_built_in.eval(&v, &mut other).unwrap();
    assert!((out[0] - other[0]).abs() > 1.0 / 255.0, "the fixture must tell the two sources apart: {out:?} vs {other:?}");
    s.execute("image.mode.grayscale", json!({"intent": "relative", "bpc": false})).unwrap();
    let px = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
    assert!((px[0] - out[0]).abs() <= 2.0 / 255.0, "the mode change read the built-in default, not the working RGB: {px:?} vs {out:?}");
}

#[test]
fn the_profile_commands_answer_the_same_for_a_tagged_document() {
    let mut replies = Vec::new();
    for working in ["srgb", "prophoto-compat"] {
        let mut s = crate::session();
        s.open_document(tagged_rgb(Builtin::AdobeRgbCompat.profile(), [0.4, 0.6, 0.2]), None);
        s.execute("edit.colorSettings", json!({"workingRgb": working})).unwrap();
        replies.push(json!({
            "info": reported(&mut s),
            "assign": s.execute("edit.assignProfile", json!({"profile": "display-p3"})).unwrap(),
            "info_after_assign": reported(&mut s),
            "convert": s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false})).unwrap(),
            "mode": s.execute("image.mode.grayscale", json!({"intent": "relative", "bpc": false})).unwrap(),
            "info_after_mode": reported(&mut s),
        }));
    }
    assert_eq!(replies[0], replies[1], "a tagged document's replies must not depend on the working space");
}
