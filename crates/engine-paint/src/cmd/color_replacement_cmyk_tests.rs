//! #11918: in a CMYK document the Color mode of Color Replacement blends on the document's own ink
//! channels, the non-separable rule of ISO 32000-1 11.3.5.3 (C, M and Y complemented, blended like
//! RGB with SetLum / ClipColor, complemented back, K from the backdrop), and the foreground becomes
//! the document's inks through the document's CMYK profile. Hue, Saturation and Luminosity in CMYK,
//! and CMYK at 16 bits, were not measured and keep the RGB blend of [#11913]'s other items.

use photocraft_color::BlendMode;
use photocraft_color::blend::{blend_rgb, lum};
use photocraft_color::convert::{CmykSpace, rgb_to_cmyk, with_cmyk_space};
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

/// The issue's test document, its stripes and blocks side by side: 320 x 120, the red, green, blue
/// and pink stripe at y 0..60 and block 0 below at x 0..80, y 60..120. Converted to CMYK/8 through
/// `profile` (`None`: the working CMYK space), with pure blue as the foreground.
fn issue_document(profile: Option<&str>) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 320, "height": 120, "depth": 8, "background": "transparent"})).unwrap();
    fill(&mut s, 0, 0, 80, 60, [220.0, 30.0, 30.0]);
    fill(&mut s, 80, 0, 80, 60, [30.0, 180.0, 60.0]);
    fill(&mut s, 160, 0, 80, 60, [40.0, 60.0, 220.0]);
    fill(&mut s, 240, 0, 80, 60, [240.0, 120.0, 120.0]);
    fill(&mut s, 0, 60, 80, 60, [200.0, 40.0, 40.0]);
    match profile {
        Some(p) => s.execute("image.mode.cmyk", json!({"profile": p})).unwrap(),
        None => s.execute("image.mode.cmyk", json!({})).unwrap(),
    };
    s.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
    s
}

/// The four rows of the issue's table: where to click and the inks the reference application
/// stored there (the reference's CMYK working space is Coated FOGRA39).
const REFERENCE_ROWS: [(i32, i32, [f32; 4]); 4] = [
    (40, 30, [0.89, 0.71, 0.0, 0.01]), // the red stripe
    (120, 30, [0.42, 0.34, 0.0, 0.0]), // the green stripe
    (280, 30, [0.55, 0.44, 0.0, 0.0]), // the pink stripe
    (40, 90, [0.91, 0.72, 0.0, 0.04]), // block 0
];

/// The backdrop inks the reference measured for those four pixels, in the same order.
const REFERENCE_BACKDROPS: [[f32; 4]; 4] =
    [[0.047_1, 0.968_6, 0.933_3, 0.007_8], [0.749, 0.0, 0.949, 0.0], [0.0, 0.651, 0.427_5, 0.0], [0.145_1, 0.949, 0.878_4, 0.043_1]];

/// Pure blue as the reference's foreground, in the inks it picked.
const REFERENCE_FG: [f32; 4] = [0.933_3, 0.746, 0.0, 0.0];

fn fill(s: &mut Session, x: i64, y: i64, w: i64, h: i64, rgb: [f32; 3]) {
    let level = |v: f32| v.clamp(0.0, 255.0).round() as u8;
    let color = format!("#{:02x}{:02x}{:02x}", level(rgb[0]), level(rgb[1]), level(rgb[2]));
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

/// Fill a rectangle with the ink values `inks` as they stand in the document (#4196: a colour
/// picked as CMYK fills a CMYK document with exactly those inks). `stand_in` is the sRGB colour the
/// pick belongs to, so different picks don't overwrite each other.
fn fill_inks(s: &mut Session, x: i64, y: i64, w: i64, h: i64, inks: [f32; 4], stand_in: &str) {
    s.execute("tools.setColors", json!({"foreground": stand_in, "foregroundCmyk": inks})).unwrap();
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    s.execute("edit.fill", json!({"color": stand_in})).unwrap();
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

/// The ink-channel rule of ISO 32000-1 11.3.5.3 for Color, on ink values that already mean the
/// document's channels: complement C, M and Y, blend them like RGB (SetLum / ClipColor), complement
/// back, K from the backdrop.
fn ink_rule_on(backdrop: [f32; 4], fg: [f32; 4]) -> [f32; 4] {
    let cb = [1.0 - backdrop[0], 1.0 - backdrop[1], 1.0 - backdrop[2]];
    let cs = [1.0 - fg[0], 1.0 - fg[1], 1.0 - fg[2]];
    let r = blend_rgb(BlendMode::Color, cb, cs);
    [1.0 - r[0], 1.0 - r[1], 1.0 - r[2], backdrop[3]]
}

/// The same rule where the `fg_rgb` foreground still has to become the document's inks: that
/// conversion goes through the profile `space` is active, the backdrop keeps its channel values.
fn ink_rule(space: Option<&std::sync::Arc<CmykSpace>>, backdrop: [f32; 4], fg_rgb: [f32; 3]) -> [f32; 4] {
    with_cmyk_space(space, || ink_rule_on(backdrop, rgb_to_cmyk(fg_rgb)))
}

/// The luminosity the rule of 11.3.5.3 keeps: that of the C, M and Y channels complemented.
fn ink_lum(v: [f32; 4]) -> f32 {
    lum([1.0 - v[0], 1.0 - v[1], 1.0 - v[2]])
}

fn assert_close_inks(what: &str, got: &[f32; 4], want: &[f32; 4]) {
    for i in 0..4 {
        assert!((got[i] - want[i]).abs() <= 0.5 / 255.0 + 1e-4, "{what}: ink {i} is {:?}, want {want:?}", (got[i] * 255.0).round(),);
    }
}

/// Compare the inks a click stored with the issue's table: the item's slack is 3 percentage points
/// per ink.
fn assert_inks_within(what: &str, got: &[f32; 4], want: [f32; 4]) {
    for i in 0..4 {
        assert!((got[i] - want[i]).abs() <= 0.03, "{what}: ink {i} is {:.2} %, the table asks for {:.2} %", got[i] * 100.0, want[i] * 100.0);
    }
}

/// Done-when 1, in the CMYK space the reference numbers were measured in: the four clicks of the
/// issue's document, converted to CMYK/8 with Coated FOGRA39 as its working space, land on the
/// table within 3 percentage points per ink.
#[test]
fn the_four_clicks_land_on_the_issue_s_ink_table_in_the_space_it_was_measured_in() {
    let mut s = issue_document(Some("coated-fogra39"));
    assert!(CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref()).is_some(), "the document now carries Coated FOGRA39");
    for (x, y, want) in REFERENCE_ROWS {
        click(&mut s, x, y, "color");
        let got = inks(&s, x, y);
        assert_inks_within(&format!("click at ({x}, {y})"), &got, want);
    }
}

/// The same four clicks in the working CMYK space (the built-in coated one `image.mode.cmyk` uses
/// unless the item asks for another profile). The rule runs there too and keeps the pixel's
/// lightness; the inks it stores are not the table's, because that space renders the pure blue
/// foreground as C96.9 M100 Y0 K0, and a Color blend takes its hue from the foreground. Measured
/// with this change, one click per row.
#[test]
fn the_four_clicks_in_the_working_colour_space_store_that_space_s_inks() {
    let mut s = issue_document(None);
    let want: [[f32; 4]; 4] =
        [[0.749_019_6, 0.772_549, 0.0, 0.0], [0.345_098, 0.356_862_7, 0.0, 0.0], [0.411_764_7, 0.427_451, 0.0, 0.0], [0.768_627_5, 0.792_156_9, 0.0, 0.0]];
    for (i, ((x, y, _), want)) in REFERENCE_ROWS.into_iter().zip(want).enumerate() {
        click(&mut s, x, y, "color");
        assert_inks_pinned(&format!("working-space click {i} at ({x}, {y})"), &inks(&s, x, y), want, 0.5 / 255.0);
    }
}

/// The lightness the issue is about: a Color click keeps the lightness of the pixel it lands on
/// instead of darkening it to the foreground's. Measured in the default working CMYK space, where
/// the inks differ from the table's because that space renders pure blue differently.
#[test]
fn the_color_click_keeps_the_pixel_s_lightness() {
    let mut s = issue_document(None);
    for (x, y, _) in REFERENCE_ROWS {
        let before = inks(&s, x, y);
        click(&mut s, x, y, "color");
        let after = inks(&s, x, y);
        let fg_lum = ink_lum(with_cmyk_space(None, || rgb_to_cmyk([0.0, 0.0, 1.0])));
        assert!(
            (fg_lum - ink_lum(before)).abs() > 0.08,
            "the fixture does not separate the two lightnesses: backdrop {:.3}, foreground {:.3}",
            ink_lum(before),
            fg_lum
        );
        assert!(
            (ink_lum(after) - ink_lum(before)).abs() < 0.01,
            "click at ({x}, {y}): the pixel's lightness moved from {:.3} to {:.3}, the foreground's is {:.3}",
            ink_lum(before),
            ink_lum(after),
            fg_lum
        );
    }
}

/// Done-when 2, the fixture that depends on no profile: the backdrop holds the issue's ink values
/// and the foreground was picked as C93.33 M74.60 Y0 K0, so the four clicks measure the rule in the
/// ink channels alone.
#[test]
fn the_color_blend_follows_the_ink_rule_without_a_profile() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 320, "height": 120, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    for (i, ((x, y, _), backdrop)) in REFERENCE_ROWS.iter().zip(REFERENCE_BACKDROPS).enumerate() {
        let stand_in = format!("#{:06x}", 0x10_00_00 + 0x20_00 * i);
        fill_inks(&mut s, (x - 30) as i64, (y - 30) as i64, 60, 60, backdrop, &stand_in);
    }
    s.execute("tools.setColors", json!({"foreground": "#0000ff", "foregroundCmyk": REFERENCE_FG})).unwrap();
    assert!(
        CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref()).is_none(),
        "the fixture carries no CMYK profile of its own: the four clicks test the rule, not a profile"
    );
    for (i, (x, y, want)) in REFERENCE_ROWS.into_iter().enumerate() {
        // The backdrop really holds the table's inks: a 60 x 60 field, and the click lands inside it.
        assert_close_inks(&format!("the {i}th backdrop"), &inks(&s, x, y), &REFERENCE_BACKDROPS[i]);
        click(&mut s, x, y, "color");
        assert_inks_within(&format!("click at ({x}, {y})"), &inks(&s, x, y), want);
        // And the stored inks are what the rule of 11.3.5.3 computes from those ink values.
        let want_rule = ink_rule_on(REFERENCE_BACKDROPS[i], REFERENCE_FG);
        assert_close_inks(&format!("the rule for {i}"), &inks(&s, x, y), &want_rule);
    }
}

/// The document's own CMYK profile changes the inks the foreground becomes, so a Color click in a
/// document tagged with a profile of its own stores different inks than the same click in one that
/// carries the working space.
#[test]
fn color_blend_in_cmyk_converts_through_the_documents_cmyk_profile() {
    let mut s = cmyk_session(8);
    tag_cmyk_profile(&mut s, craft_cms::Builtin::CoatedFogra39.profile().to_bytes());
    let space = CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref());
    assert!(space.is_some(), "FOGRA39 is a CMYK profile, so it is a space of its own");
    let before = inks(&s, 80, 30);
    let green_before = inks(&s, 10, 110);
    let want = ink_rule(space.as_ref(), before, [0.0, 0.0, 1.0]);
    // The built-in coated conversions ask for other inks: without this difference the test would
    // not show which profile the command used.
    let built_in = ink_rule(None, before, [0.0, 0.0, 1.0]);
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
    let coated = ink_rule(None, inks(&s, 80, 30), [0.0, 0.0, 1.0]);
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
/// inks its own profile asks for, not the built-in coated ones. CMYK/16 is *not measured* in the
/// issue, so the blend itself stays the RGB one; the golden values below were measured with the
/// profile plumbing; `upstream/main` stored the untagged 16-bit values quoted there for the same
/// clicks, whatever profile the document carried.
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

/// The same Color click in the working CMYK space (the built-in coated one, which is what
/// `image.mode.cmyk` uses unless the item asks for another profile): the ink-channel rule runs
/// there too, and stores these inks. They are NOT the table's: that space renders pure blue as
/// C96.9 M100 Y0 K0, and a Color blend takes its hue from the foreground, so the C and M of the
/// result follow that ratio rather than the table's 89 : 71. Measured with this change.
#[test]
fn the_color_click_in_the_working_colour_space_blends_on_the_ink_channels() {
    let mut s = cmyk_session(8);
    click(&mut s, 80, 30, "color");
    assert_inks_pinned("working-space color", &inks(&s, 80, 30), [0.749_019_6, 0.772_549, 0.0, 0.0], 0.5 / 255.0);
}

/// Neighbour, the three blend modes the issue leaves unmeasured, in a CMYK/8 document on the working
/// colour space: they keep the RGB blend and the inks it stores. Guard test, passes both ways.
#[test]
fn the_unmeasured_cmyk_blend_modes_keep_todays_inks_in_the_working_colour_space() {
    for (mode, want) in [
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
    let mut s = cmyk_session(8);
    clear_profile(&mut s);
    assert!(CmykSpace::for_profile(s.active().unwrap().doc.icc_profile.as_ref()).is_none(), "untagged: the built-in CMYK applies");
    click(&mut s, 80, 30, "luminosity");
    assert_inks_pinned("untagged luminosity", &inks(&s, 80, 30), [0.266_666_68, 0.988_235_3, 0.909_803_9, 0.364_705_9], 0.5 / 255.0);
}

/// Neighbour, CMYK/16 (not measured in the issue): the RGB blend and the same inks, up to the finer
/// quantum. Guard test, passes both ways.
#[test]
fn a_cmyk_16_bit_document_keeps_todays_inks() {
    for (mode, want) in [("color", [0.959_304_2, 1.0, 0.0, 0.0]), ("luminosity", [0.268_467_22, 0.990_020_6, 0.909_620_8, 0.364_675_37])] {
        let mut s = cmyk_session(16);
        click(&mut s, 80, 30, mode);
        assert_inks_pinned(&format!("16-bit {mode}"), &inks(&s, 80, 30), want, 1.0 / 65_535.0);
    }
}
