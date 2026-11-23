//! #11918: in a CMYK document Color Replacement converts its pixels to the RGB blend space and
//! back through the document's own CMYK profile, the way the Brush stroke, the compositor and the
//! composite exports already do. Before, only the built-in coated CMYK was ever used, so a
//! document that carries a CMYK profile of its own was recoloured against the wrong device colour.

use photocraft_color::BlendMode;
use photocraft_color::blend::blend_rgb;
use photocraft_color::convert::{CmykSpace, cmyk_to_rgb, rgb_to_cmyk, with_cmyk_space};
use serde_json::json;

use super::*;

/// A 160 x 120 document, red above and green below, converted to CMYK by Image > Mode > CMYK
/// Colour (the issue's route), with pure blue as the foreground.
fn cmyk_session(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 160, "height": 120, "depth": depth, "background": "transparent"})).unwrap();
    fill(&mut s, 0, 0, 160, 60, [220.0, 30.0, 30.0]);
    fill(&mut s, 0, 60, 160, 60, [30.0, 180.0, 60.0]);
    s.execute("image.mode.cmyk", json!({})).unwrap();
    s.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
    s
}

fn fill(s: &mut Session, x: i64, y: i64, w: i64, h: i64, rgb: [f32; 3]) {
    let level = |v: f32| v.clamp(0.0, 255.0).round() as u8;
    let color = format!("#{:02x}{:02x}{:02x}", level(rgb[0]), level(rgb[1]), level(rgb[2]));
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

/// Give the document a CMYK profile of its own (what Convert to Profile or an imported file does).
fn tag_cmyk_profile(s: &mut Session, profile: std::sync::Arc<Vec<u8>>) {
    std::sync::Arc::make_mut(s.active_mut().unwrap().doc_mut()).icc_profile = Some(profile);
}

/// An untagged document: the built-in coated CMYK conversions apply.
fn clear_profile(s: &mut Session) {
    std::sync::Arc::make_mut(s.active_mut().unwrap().doc_mut()).icc_profile = None;
}

/// The inks the document stores at one pixel (0..1 per channel), from a session or a live preview.
fn inks_of(doc: &photocraft_doc::Document, layer: photocraft_doc::LayerId, x: i32, y: i32) -> [f32; 4] {
    let surf = doc.layer(layer).unwrap().surface().unwrap();
    let px = surf.pixel(x, y);
    std::array::from_fn(|i| px.get(i).copied().unwrap_or(0.0))
}

fn inks(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    inks_of(&st.doc, st.active_layer.unwrap(), x, y)
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// A 40 px hard round tip, Mode Color, default options: one click.
fn click(s: &mut Session, x: i32, y: i32, mode: &str) {
    s.execute("paint.colorReplacement", json!({"points": [[x, y]], "size": 40, "hardness": 100, "mode": mode})).unwrap();
}

/// The inks the Color blend of the blue foreground onto `backdrop` (as stored in the document)
/// asks for, converted through the CMYK profile `space` in both directions.
fn blend_inks(space: Option<&std::sync::Arc<CmykSpace>>, backdrop: [f32; 4]) -> [f32; 4] {
    with_cmyk_space(space, || {
        let c = cmyk_to_rgb(backdrop);
        rgb_to_cmyk(blend_rgb(BlendMode::Color, [c[0], c[1], c[2]], [0.0, 0.0, 1.0]))
    })
}

fn assert_close_inks(what: &str, got: &[f32; 4], want: &[f32; 4]) {
    for i in 0..4 {
        assert!((got[i] - want[i]).abs() <= 0.5 / 255.0 + 1e-4, "{what}: ink {i} is {:?}, want {want:?}", (got[i] * 255.0).round(),);
    }
}

/// The document's own CMYK profile changes what its stored inks mean, so the Color blend of the
/// same pixels asks for different inks: the command has to convert through that profile.
#[test]
fn color_blend_in_cmyk_converts_through_the_documents_cmyk_profile() {
    let mut s = cmyk_session(8);
    tag_cmyk_profile(&mut s, craft_cms::Builtin::CoatedFogra39.profile().to_bytes());
    let space = CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref());
    assert!(space.is_some(), "FOGRA39 is a CMYK profile, so it is a space of its own");
    let before = inks(&s, 80, 30);
    let green_before = inks(&s, 10, 110);
    let want = blend_inks(space.as_ref(), before);
    // The built-in coated conversions ask for other inks: without this difference the test would
    // not show which profile the command used.
    let built_in = blend_inks(None, before);
    let apart = (0..4).map(|i| (want[i] - built_in[i]).abs()).fold(0.0f32, f32::max);
    assert!(apart > 0.03, "the document profile and the built-in CMYK give the same inks {want:?}: the test proves nothing");
    click(&mut s, 80, 30, "color");
    assert_close_inks("the clicked pixel", &inks(&s, 80, 30), &want);
    // Outside the tip nothing moves at all.
    assert_eq!(inks(&s, 10, 110), green_before, "a pixel the tip never reached keeps its inks");
}

/// The live preview runs its dabs outside the command call, so it must carry the document's CMYK
/// profile into them: the dabs it shows store the same inks the commit does.
#[test]
fn a_live_cmyk_stroke_previews_the_documents_cmyk_profile() {
    let mut s = cmyk_session(8);
    tag_cmyk_profile(&mut s, craft_cms::Builtin::CoatedFogra39.profile().to_bytes());
    assert!(CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref()).is_some(), "FOGRA39 is a CMYK profile of its own");
    let coated = blend_inks(None, inks(&s, 80, 30));
    // The preview: the first point builds the stroke, and the second one arrives after that call
    // returned, when nothing on this thread names the document's profile any more.
    let mut live = live_color_replacement(&s, &json!({"points": [[80, 30]], "size": 40, "hardness": 100, "mode": "color"})).unwrap();
    live.push(&[StrokePoint::new(95.0, 30.0, 1.0)]).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.clone();
    // The commit of the same stroke stores what the preview showed.
    s.execute("paint.colorReplacement", json!({"points": [[80, 30], [95, 30]], "size": 40, "hardness": 100, "mode": "color"})).unwrap();
    for (x, y) in [(80, 30), (95, 30)] {
        assert_close_inks(&format!("the preview at {x}, {y}"), &inks_of(&shown, id, x, y), &inks(&s, x, y));
    }
    // And those inks come from the document's profile rather than the built-in coated CMYK. This
    // pixel lies under two dabs, so the value an unfixed stroke stores is the coated inks blended
    // once per covering dab - near `coated`, and far from what the profile asks for: the test is a
    // proximity gate, not a pin of an exact unfixed value.
    let apart = (0..4).map(|i| (inks_of(&shown, id, 95, 30)[i] - coated[i]).abs()).fold(0.0f32, f32::max);
    assert!(apart > 0.05, "the preview stored the built-in coated inks {coated:?}: the document's profile was not used");
}

/// A CMYK/16 document that carries a profile of its own goes through the same change: it stores the
/// inks its own profile asks for, not the built-in coated ones. The golden values below were
/// measured with the change; `upstream/main` stored the untagged 16-bit values quoted there for the
/// same clicks, whatever profile the document carried.
#[test]
fn a_tagged_cmyk_16_bit_document_blends_through_its_own_profile_too() {
    for (mode, want, coated) in [
        ("color", [0.742_946_5, 0.606_225_67, 0.0, 0.0], [0.959_304_2, 1.0, 0.0, 0.0]),
        ("luminosity", [0.686_488_15, 0.992_675_66, 0.943_694_23, 0.484_611_27], [0.268_467_22, 0.990_020_6, 0.909_620_8, 0.364_675_37]),
    ] {
        let mut s = cmyk_session(16);
        tag_cmyk_profile(&mut s, craft_cms::Builtin::CoatedFogra39.profile().to_bytes());
        click(&mut s, 80, 30, mode);
        assert_inks_pinned(&format!("16-bit tagged {mode}"), &inks(&s, 80, 30), want, 1.0 / 65_535.0);
        let apart = (0..4).map(|i| (inks(&s, 80, 30)[i] - coated[i]).abs()).fold(0.0f32, f32::max);
        assert!(apart > 0.05, "16-bit {mode}: the tagged document stored the built-in coated inks {coated:?}");
    }
}

/// Compare stored inks with a pinned golden value, to the quantum of the depth. The tests say which
/// run each value came from: a run of `upstream/main`, or a run of this change.
fn assert_inks_pinned(what: &str, got: &[f32; 4], want: [f32; 4], quantum: f32) {
    for i in 0..4 {
        assert!((got[i] - want[i]).abs() < quantum, "{what}: ink {i} is {}, want {}", got[i], want[i]);
    }
}

/// Neighbour, an RGB document: the Color blend and the untouched pixels are exactly what they were
/// (#11918 changes no RGB path). Guard test, passes before and after.
#[test]
fn color_blend_on_an_rgb_document_keeps_todays_pixels() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 160, "height": 120, "background": "#dc1e1e"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
    click(&mut s, 80, 30, "color");
    assert_inks_pinned("the clicked pixel", &rgba(&s, 80, 30), [0.258_823_54, 0.258_823_54, 1.0, 1.0], 0.5 / 255.0);
    assert_inks_pinned("a pixel outside the tip", &rgba(&s, 8, 8), [0.862_745_1, 0.117_647_06, 0.117_647_06, 1.0], 0.5 / 255.0);
}

/// Neighbour, a CMYK document that carries the built-in working profile (the common case): the
/// blend rule of every mode and the inks it stores are unchanged. Guard test, passes both ways.
#[test]
fn the_cmyk_blend_modes_keep_todays_inks_in_the_working_colour_space() {
    for (mode, want) in [
        ("color", [0.960_784_3, 1.0, 0.0, 0.0]),
        ("hue", [0.960_784_3, 1.0, 0.0, 0.0]),
        ("saturation", [0.0, 0.952_941_2, 0.996_078_43, 0.0]),
        ("luminosity", [0.266_666_68, 0.988_235_3, 0.909_803_9, 0.364_705_9]),
    ] {
        let mut s = cmyk_session(8);
        click(&mut s, 80, 30, mode);
        assert_inks_pinned(&format!("{mode} mode"), &inks(&s, 80, 30), want, 0.5 / 255.0);
    }
}

/// Neighbour, an untagged CMYK document: no profile of its own, so the built-in coated conversions
/// apply exactly as before. Guard test, passes both ways.
#[test]
fn an_untagged_cmyk_document_keeps_todays_inks() {
    for (mode, want) in [("color", [0.960_784_3, 1.0, 0.0, 0.0]), ("luminosity", [0.266_666_68, 0.988_235_3, 0.909_803_9, 0.364_705_9])] {
        let mut s = cmyk_session(8);
        clear_profile(&mut s);
        assert!(CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref()).is_none(), "untagged: the built-in CMYK applies");
        click(&mut s, 80, 30, mode);
        assert_inks_pinned(&format!("untagged {mode}"), &inks(&s, 80, 30), want, 0.5 / 255.0);
    }
}

/// Neighbour, CMYK/16 (not measured in the issue): the same blend rule and the same inks, up to the
/// finer quantum. Guard test, passes both ways.
#[test]
fn a_cmyk_16_bit_document_keeps_todays_inks() {
    for (mode, want) in [("color", [0.959_304_2, 1.0, 0.0, 0.0]), ("luminosity", [0.268_467_22, 0.990_020_6, 0.909_620_8, 0.364_675_37])] {
        let mut s = cmyk_session(16);
        click(&mut s, 80, 30, mode);
        assert_inks_pinned(&format!("16-bit {mode}"), &inks(&s, 80, 30), want, 1.0 / 65_535.0);
    }
}
