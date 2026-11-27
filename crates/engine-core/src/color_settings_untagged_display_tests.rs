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
    // The monitor side of the comparison: with no platform profile the canvas is shown on sRGB.
    assert_eq!(s.color().monitor().description, Builtin::Srgb.profile().description, "the test monitor");
    let cd = s.color().canvas_display(&d).unwrap();
    assert_eq!(cd.source.description, working.description, "an untagged document is shown in Color Settings' working RGB");
    for (i, v) in values.iter().enumerate() {
        let got = shown(&s, &d, i);
        let want = shown_want(&s, &working, v);
        assert!(close(got, want, TOL), "column {i}: canvas {got:?} vs the working space's transform {want:?}");
        assert!(far(got, raw_codes(v), 8.0), "column {i} is still shown as its raw values: {got:?}");
    }
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
fn a_layer_copied_into_an_untagged_document_lands_in_its_space() {
    let mut s = crate::session();
    // A: untagged, opened under the default settings so nothing is assigned to it on the way in.
    s.open_document(untagged_rgb(&[[0.5, 0.5, 0.5]], SampleType::U8), None);
    // B: the same shape tagged sRGB, with a saturated block.
    let v = [0.9f32, 0.2, 0.2];
    s.open_document(tagged_rgb(Builtin::Srgb.profile(), v), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let block = s.active().unwrap().active_layer.unwrap();
    let r = s.execute("layer.copyToDocument", json!({"document": 0, "source": 1, "layers": [block.0]})).unwrap();
    let id = photocraft_doc::LayerId(r["layers"][0].as_u64().unwrap());
    let a = &s.documents()[0].doc;
    assert!(a.icc_profile.is_none(), "the destination is still untagged");
    let px = a.layer(id).unwrap().surface().unwrap().pixel(1, 1);
    // It has to keep its look, so it is re-encoded into the space A is shown in.
    let mut out = [0.0f32; 16];
    let working = s.color().working(ColorMode::Rgb);
    Transform::new(Builtin::Srgb.profile(), &working, s.color().settings.intent(), s.color().settings.bpc).unwrap().eval(&v, &mut out).unwrap();
    let want = [out[0], out[1], out[2]];
    assert!((0..3).all(|c| (px[c] - want[c]).abs() <= 2.0 / 255.0), "the pasted block landed as {px:?}, not in the space A is shown in ({want:?})");
    assert!((0..3).any(|c| (px[c] - v[c]).abs() > 8.0 / 255.0), "the fixture must tell a raw copy apart from a re-encoded one: {px:?} vs {v:?}");
}

#[test]
fn the_gamut_count_is_judged_in_the_space_the_canvas_marks_in() {
    let mut s = crate::session();
    // Six colours that straddle the coated-CMYK gamut: read as sRGB some of them are inside it,
    // read as ProPhoto the same samples are far outside.
    let colours = [[0.95f32, 0.05, 0.05], [0.8, 0.3, 0.3], [0.6, 0.1, 0.1], [0.3, 0.85, 0.3], [0.1, 0.4, 0.9], [0.45, 0.45, 0.95]];
    s.open_document(untagged_rgb(&colours, SampleType::U8), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    s.execute("view.proofSetup", json!({"profile": "coated-cmyk", "intent": "perceptual"})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let setup = s.color().proof(doc.id).setup.clone();
    let buf = photocraft_compose::flatten(&doc);
    let count_with = |src: &Profile| {
        let check = GamutCheck::new(src, &setup.profile, 4.0).unwrap();
        let mut mask = vec![0u8; buf.px.len()];
        let count = check.mask_rgba(&buf.px, &mut mask).unwrap();
        (mask, count)
    };
    // The canvas marks its out-of-gamut pixels through the display source; that is the count the
    // Gamut Warning command and Color Range > Out of Gamut have to agree with.
    let want = count_with(&s.color().composite_profile(&doc));
    let built_in = count_with(&composite_profile(&doc));
    assert_ne!(want.1, built_in.1, "the fixture must tell the two spaces apart: {want:?} vs {built_in:?}");
    let warning = s.execute("view.gamutWarning", json!({"on": true, "threshold": 4.0})).unwrap();
    assert_eq!(warning["outOfGamut"].as_u64().unwrap() as usize, want.1, "Gamut Warning judged another space than the canvas");
    // Color Range › Out of Gamut has to mark exactly those pixels.
    s.execute("select.colorRange", json!({"select": "outOfGamut"})).unwrap();
    let with = s.active().unwrap().doc.clone();
    let selection = with.selection.clone().expect("the selection was set");
    let width = with.size.width as usize;
    let marked = |surface: &photocraft_raster::Surface| -> Vec<usize> {
        (0..with.size.width as usize / 4).map(|i| (surface.pixel(i as i32 * 4 + 2, 2)[0] > 0.5) as usize).collect()
    };
    let marks = marked(&selection);
    let columns: Vec<usize> = (0..width / 4).map(|i| usize::from(want.0[2 * width + i * 4 + 2] == 255)).collect();
    assert_eq!(marks, columns, "Color Range > Out of Gamut marks other pixels than the warning counts");
}

#[test]
fn converting_an_untagged_document_to_working_leaves_its_samples() {
    let mut s = crate::session();
    let v = [0.9f32, 0.2, 0.2];
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    // The answer to the "missing profile" prompt for a document that is not open yet.
    let mut d = untagged_rgb(&[v], SampleType::U8);
    s.color().resolve_open(&mut d, "convert").unwrap();
    let tagged = profile_from_bytes(d.icc_profile.as_ref().expect("tagged by the answer")).unwrap();
    assert_eq!(tagged.description, s.color().working(ColorMode::Rgb).description, "tagged with the working space");
    let px = d.layers[0].surface().unwrap().pixel(1, 1);
    assert!((0..3).all(|c| (px[c] - v[c]).abs() <= 2.0 / 255.0), "the answer read the document in the built-in default and moved its pixels: {px:?} vs {v:?}");
    // The command path agrees: an untagged document answers the same way once it is open.
    s.open_document(untagged_rgb(&[v], SampleType::U8), None);
    let r = s.execute("color.profileMismatch", json!({"action": "convertToWorking"})).unwrap();
    assert_eq!(r["action"], json!("converted"));
    let px = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
    assert!((0..3).all(|c| (px[c] - v[c]).abs() <= 2.0 / 255.0), "the open document's answer moved its pixels: {px:?}");
    let info = reported(&mut s);
    assert_eq!(info["embedded"], json!(true));
    assert_eq!(info["description"], s.color().working(ColorMode::Rgb).description);
}

#[test]
fn an_untagged_indexed_document_follows_the_mode_default_until_it_becomes_rgb() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ff0000"})).unwrap();
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "colors": 8})).unwrap();
    // Indexed is not one of the two modes the item measures: it keeps the mode's built-in default.
    let indexed = working_profile(ColorMode::Indexed);
    assert_eq!(reported(&mut s)["description"], indexed.description, "an Indexed document is not switched");
    // Image > Mode > RGB without a profile keeps the stored samples; the document that results is
    // untagged RGB, so the reply and Profile Info have to name the same space.
    let r = s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(r["profile"], s.color().working(ColorMode::Rgb).description, "the reply names the space the result is taken in");
    assert_eq!(reported(&mut s)["description"], s.color().working(ColorMode::Rgb).description, "and Profile Info agrees");
}

#[test]
fn colour_range_judges_sampled_colours_in_the_space_the_canvas_shows() {
    let mut s = crate::session();
    // A box spanned by two sampled colours and a ramp of columns across it. Membership is decided in
    // Lab, through the profile the composite is read in, so once the working space moves the box has
    // to move with the picture: with the ramp below one column drops out going from sRGB to
    // ProPhoto, and none of this changes at the default setting.
    let columns: Vec<[f32; 3]> = (0..8).map(|i| [0.55 + 0.03 * i as f32, 0.45 - 0.03 * i as f32, 0.45 - 0.02 * i as f32]).collect();
    s.open_document(untagged_rgb(&columns, SampleType::U8), None);
    let marked = |s: &mut Session| -> Vec<usize> {
        s.execute("select.colorRange", json!({"select": "sampledColors", "colors": ["#cc4d4d", "#996666"], "fuzziness": 40})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let selection = doc.selection.clone().expect("a selection was set");
        (0..columns.len()).filter(|i| selection.pixel(*i as i32 * 4 + 2, 2)[0] > 0.5).collect()
    };
    let as_built_in = marked(&mut s);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let as_working = marked(&mut s);
    assert_ne!(as_built_in, as_working, "the sampled colour box marked the same columns whatever the working space: {as_built_in:?} vs {as_working:?}");
}

#[test]
fn replace_color_judges_its_lab_bounds_in_the_space_the_canvas_shows() {
    // A Lab cube drawn around one column of the gamut fixture through the profile this session reads
    // the layer in. The same device numbers sit far away when they are read in the mode's built-in
    // default, so the cube has to catch that column here and miss it in a session at the default.
    let columns = [[0.95f32, 0.05, 0.05], [0.8f32, 0.3, 0.3], [0.6f32, 0.1, 0.1], [0.3, 0.85, 0.3], [0.1, 0.4, 0.9], [0.45, 0.45, 0.95]];
    let column = 1usize;
    let lab_in = |profile: &Profile, v: [f32; 3]| -> [f32; 3] {
        let t = craft_cms::cached(profile, Builtin::LabD50.profile(), craft_cms::TransformOptions::default()).unwrap();
        let mut out = [0.0f32; 16];
        t.eval(&v, &mut out).unwrap();
        [out[0] * 100.0, out[1] * 255.0 - 128.0, out[2] * 255.0 - 128.0]
    };
    let mut s = crate::session();
    s.open_document(untagged_rgb(&columns, SampleType::U8), None);
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let centre = lab_in(&s.color().document_profile(&doc), columns[column]);
    let elsewhere = lab_in(&document_profile(&doc), columns[column]);
    assert!((0..3).any(|c| (centre[c] - elsewhere[c]).abs() > 10.0), "the fixture must tell the two readings apart: {centre:?} vs {elsewhere:?}");
    let cube = json!({
        "labMinimum": [centre[0] - 4.0, centre[1] - 4.0, centre[2] - 4.0],
        "labMaximum": [centre[0] + 4.0, centre[1] + 4.0, centre[2] + 4.0],
        "fuzziness": 0,
        "saturation": -60,
    });
    let shifted = |s: &Session| -> bool {
        // The layer makes a Lab round trip whether it is inside the cube or not, which moves single
        // codes here and there; only the desaturation is a real change.
        let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
        let got = raw_codes(&surf.pixel(column as i32 * 4 + 2, 2));
        let stored = raw_codes(&columns[column]);
        (0..3).any(|c| (got[c] - stored[c]).abs() > 8.0)
    };
    s.execute("image.adjustments.replaceColor", cube.clone()).unwrap();
    assert!(shifted(&s), "the cube drawn in this session's own space missed the column it was drawn around: {centre:?}");
    let mut plain = crate::session();
    plain.open_document(untagged_rgb(&columns, SampleType::U8), None);
    plain.execute("image.adjustments.replaceColor", cube).unwrap();
    assert!(!shifted(&plain), "the same cube also caught the column read in the built-in default: {centre:?} vs {elsewhere:?}");
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
