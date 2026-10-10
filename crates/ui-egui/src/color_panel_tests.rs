//! Color panel modes (#294): value round trips through every mode, the cms conversions, the
//! panel menu, persistence, and the panel drawn and driven in each mode.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::theme::ThemeKind;

fn sp() -> Spaces {
    Spaces::defaults().unwrap()
}

fn close(a: &[f32], b: &[f32], tol: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn lab_matches_known_values_and_round_trips() {
    let sp = sp();
    let white = sp.rgb_to_lab([1.0; 3]);
    assert!(close(&white, &[100.0, 0.0, 0.0], 0.6), "{white:?}");
    let black = sp.rgb_to_lab([0.0; 3]);
    assert!(close(&black, &[0.0, 0.0, 0.0], 0.6), "{black:?}");
    // sRGB red is about L 54, a 81, b 70 under D50 (Bradford-adapted).
    let red = sp.rgb_to_lab([1.0, 0.0, 0.0]);
    assert!((red[0] - 54.3).abs() < 2.0 && (red[1] - 80.8).abs() < 3.0 && (red[2] - 69.9).abs() < 3.0, "{red:?}");
    // In-gamut Lab values come back as typed.
    for lab in [[50.0, 20.0, -30.0], [75.0, -10.0, 15.0], [30.0, 5.0, 5.0], [90.0, 0.0, 0.0]] {
        let rgb = from_components(ColorPanelMode::Lab, &lab, &sp);
        let back = to_components(ColorPanelMode::Lab, rgb, &sp);
        assert!(close(&back, &lab, 0.6), "{lab:?} -> {rgb:?} -> {back:?}");
    }
}

#[test]
fn every_mode_round_trips_its_values() {
    let sp = sp();
    // Muted colours sit inside the coated CMYK gamut too.
    let colours = [[0.2, 0.4, 0.6], [0.7, 0.55, 0.3], [0.5, 0.5, 0.5], [0.85, 0.8, 0.75], [0.3, 0.45, 0.35]];
    for mode in ColorPanelMode::ALL {
        for rgb in colours {
            let comps = to_components(mode, rgb, &sp);
            assert_eq!(comps.len(), mode.components().len(), "{mode:?}");
            for (v, c) in comps.iter().zip(mode.components()) {
                assert!(v.is_finite() && *v >= c.min - 1e-3 && *v <= c.max + 1e-3, "{mode:?} {}: {v}", c.label);
            }
            let back = from_components(mode, &comps, &sp);
            let tol = match mode {
                ColorPanelMode::Web => 0.11,                               // snaps to the 51-step cube
                ColorPanelMode::Grayscale => 1.0,                          // grey loses the hue
                ColorPanelMode::Cmyk | ColorPanelMode::Lab => 2.5 / 255.0, // through the cms tables
                _ => 1e-4,
            };
            if mode == ColorPanelMode::Grayscale {
                // Grey through grey is stable: K → RGB → K.
                let k = comps[0];
                let again = to_components(mode, back, &sp)[0];
                assert!((again - k).abs() < 0.5, "K {k} -> {again}");
                assert!((back[0] - back[1]).abs() < 1e-3 && (back[1] - back[2]).abs() < 1e-3, "grey stays neutral: {back:?}");
            } else {
                assert!(close(&back, &rgb, tol), "{mode:?}: {rgb:?} -> {comps:?} -> {back:?}");
            }
        }
    }
}

#[test]
fn slider_values_typed_in_each_mode_come_back() {
    let sp = sp();
    let cases: [(ColorPanelMode, &[f32]); 5] = [
        (ColorPanelMode::Rgb, &[12.0, 200.0, 99.0]),
        (ColorPanelMode::Hsb, &[210.0, 40.0, 70.0]),
        (ColorPanelMode::Web, &[51.0, 204.0, 153.0]),
        (ColorPanelMode::Grayscale, &[35.0]),
        (ColorPanelMode::Cmyk, &[30.0, 20.0, 40.0, 0.0]),
    ];
    for (mode, typed) in cases {
        let rgb = from_components(mode, typed, &sp);
        let back = to_components(mode, rgb, &sp);
        let tol = if mode == ColorPanelMode::Cmyk { 3.0 } else { 0.6 };
        assert!(close(&back, typed, tol), "{mode:?}: {typed:?} -> {back:?}");
    }
}

#[test]
fn hostile_components_never_panic_and_stay_in_range() {
    let sp = sp();
    for mode in ColorPanelMode::ALL {
        for c in [vec![], vec![f32::NAN; 4], vec![f32::INFINITY, -1e9, 1e9, f32::NEG_INFINITY], vec![1.0; 9]] {
            let rgb = from_components(mode, &c, &sp);
            assert!(rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{mode:?} {c:?} -> {rgb:?}");
        }
        let comps = to_components(mode, [f32::NAN, 2.0, -1.0], &sp);
        assert!(comps.iter().all(|v| v.is_finite()), "{mode:?}");
    }
    assert_eq!(web_safe(f32::NAN), 0.0);
    assert_eq!(web_safe(130.0), 153.0);
    assert_eq!(hsb_to_rgb([f32::NAN, 50.0, 50.0]).len(), 3);
}

#[test]
fn cmyk_follows_the_working_space_not_a_formula() {
    // A naive 1-max(rgb) formula gives K = 0 for pure red; a real press profile puts ink in
    // M and Y and keeps C low, and white paper has no ink at all.
    let sp = sp();
    let paper = sp.rgb_to_cmyk([1.0; 3]);
    assert!(paper.iter().all(|v| *v < 1.0), "{paper:?}");
    let black = sp.rgb_to_cmyk([0.0; 3]);
    assert!(black[3] > 60.0, "rich black uses K: {black:?}");
    // Saturated sRGB blue is outside the coated gamut; a muted blue is inside.
    assert!(sp.printable([0.0, 0.0, 1.0]).iter().zip([0.0, 0.0, 1.0]).any(|(a, b)| (a - b).abs() > 0.02));
    assert!(sp.out_of_gamut([0.0, 0.0, 1.0]) && sp.out_of_gamut([0.0, 1.0, 0.0]));
    let muted = [0.4, 0.5, 0.6];
    assert!(close(&sp.printable(muted), &muted, 2.5 / 255.0));
    // Black, white and greys print (the press's black is a little lighter, but close).
    for c in [[0.0; 3], [1.0; 3], [0.5; 3], muted] {
        assert!(!sp.out_of_gamut(c), "{c:?}");
    }
}

#[test]
fn modes_serialise_by_key_and_old_state_loads() {
    for m in ColorPanelMode::ALL {
        assert_eq!(ColorPanelMode::from_key(m.key()), Some(m));
        assert_eq!(serde_json::to_value(m).unwrap(), json!(m.key()));
    }
    assert_eq!(ColorPanelMode::from_key("nope"), None);
    let s: ColorPanelState = serde_json::from_value(json!({"mode": "bogus"})).unwrap_or_default();
    assert_eq!(s.mode, ColorPanelMode::HueCube);
    let mut ui = serde_json::to_value(crate::state::UiState::default()).unwrap();
    ui.as_object_mut().unwrap().remove("color_panel");
    let ui: crate::state::UiState = serde_json::from_value(ui).unwrap();
    assert_eq!(ui.color_panel, ColorPanelState::default());
}

#[test]
fn the_mode_is_remembered_in_the_preferences() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    set_mode(&mut app, ColorPanelMode::Lab);
    let prefs = app.session.prefs_to_json();
    let mut s2 = photocraft_engine::Session::new();
    s2.load_prefs_json(&prefs).unwrap();
    // The app restores it when it starts.
    let mut app2 = PhotocraftApp::new(s2, crate::Services::default());
    assert_eq!(app2.ui.color_panel.mode, ColorPanelMode::Lab);
    app2.ui.color_panel.mode = ColorPanelMode::HueCube;
    restore(&mut app2);
    assert_eq!(app2.ui.color_panel.mode, ColorPanelMode::Lab);
    // A corrupt value keeps the default.
    app2.session.prefs.edit(|p| {
        p.dialogs.insert(PREF_KEY.into(), json!({"mode": 7}));
    });
    app2.ui.color_panel.mode = ColorPanelMode::HueCube;
    restore(&mut app2);
    assert_eq!(app2.ui.color_panel.mode, ColorPanelMode::HueCube);
}

fn harness(mode: ColorPanelMode) -> Harness<'static, PhotocraftApp> {
    harness_sized(mode, vec2(300.0, 220.0))
}

fn harness_sized(mode: ColorPanelMode, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.color_panel.mode = mode;
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            panel(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.state_mut().ui.theme = ThemeKind::ProMedium;
    h.run_steps(3);
    h
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn fg(h: &Harness<'static, PhotocraftApp>) -> [f32; 3] {
    let c = h.state().session.tools.foreground;
    [c[0], c[1], c[2]]
}

#[test]
fn every_mode_draws_its_controls() {
    for mode in ColorPanelMode::ALL {
        let h = harness(mode);
        let r = last_rects(&h.ctx);
        assert_eq!(r.mode, Some(mode));
        assert!(r.foreground.is_some() && r.background.is_some(), "{mode:?}: chips");
        if mode.is_field() {
            assert!(r.field.is_some_and(|f| f.width() > 40.0 && f.height() > 40.0), "{mode:?}: {r:?}");
            assert!(r.strip.is_some());
        } else {
            assert_eq!(r.ramps.len(), mode.components().len(), "{mode:?}");
            assert!(r.spectrum.is_some());
        }
        // Drawing changes nothing.
        assert!(close(&fg(&h), &[0.2, 0.4, 0.6], 1e-6), "{mode:?}");
    }
}

#[test]
fn clicking_a_ramp_sets_that_component() {
    // RGB: the right end of the R ramp is R 255; G and B stay.
    let mut h = harness(ColorPanelMode::Rgb);
    let r = last_rects(&h.ctx).ramps[0];
    click(&mut h, Pos2::new(r.right() - 0.5, r.center().y));
    let c = fg(&h);
    assert!((c[0] - 1.0).abs() < 0.01 && (c[1] - 0.4).abs() < 0.01 && (c[2] - 0.6).abs() < 0.01, "{c:?}");
    // CMYK: K at the left end (0 %) is what the colour already had (no K for this colour), and
    // clicking the middle of the K ramp darkens the colour.
    let mut h = harness(ColorPanelMode::Cmyk);
    let k = last_rects(&h.ctx).ramps[3];
    let before = fg(&h);
    click(&mut h, Pos2::new(k.center().x, k.center().y));
    let after = fg(&h);
    assert!(after.iter().sum::<f32>() < before.iter().sum::<f32>() - 0.2, "{before:?} -> {after:?}");
    // Lab: the left end of L is the darkest colour with these a and b.
    let mut h = harness(ColorPanelMode::Lab);
    let l = last_rects(&h.ctx).ramps[0];
    click(&mut h, Pos2::new(l.left() + 0.5, l.center().y));
    let lab = sp().rgb_to_lab(fg(&h));
    assert!(lab[0] < 12.0, "{lab:?} from {:?}", fg(&h));
}

#[test]
fn the_cubes_and_the_wheel_pick_colours() {
    // Hue Cube: the top-right corner of the field is full saturation and brightness.
    let mut h = harness(ColorPanelMode::HueCube);
    let f = last_rects(&h.ctx).field.unwrap();
    click(&mut h, Pos2::new(f.right() - 0.5, f.top() + 0.5));
    let hsb = rgb_to_hsb(fg(&h));
    assert!(hsb[1] > 97.0 && hsb[2] > 97.0 && (hsb[0] - 210.0).abs() < 2.0, "{hsb:?}");
    // Brightness Cube: the left edge is red hue, the top full saturation.
    let mut h = harness(ColorPanelMode::BrightnessCube);
    let f = last_rects(&h.ctx).field.unwrap();
    click(&mut h, Pos2::new(f.left() + 0.5, f.top() + 0.5));
    let hsb = rgb_to_hsb(fg(&h));
    assert!((hsb[0] < 3.0 || hsb[0] > 357.0) && hsb[1] > 97.0, "{hsb:?}");
    // Color Wheel: the ring's rightmost point is red; the square's bottom is black.
    let mut h = harness(ColorPanelMode::ColorWheel);
    let r = last_rects(&h.ctx);
    let wheel = r.strip.unwrap();
    click(&mut h, Pos2::new(wheel.right() - 3.0, wheel.center().y));
    let hsb = rgb_to_hsb(fg(&h));
    assert!(hsb[0] < 4.0 || hsb[0] > 356.0, "ring right is red: {hsb:?}");
    let sq = r.field.unwrap();
    click(&mut h, Pos2::new(sq.center().x, sq.bottom() - 0.5));
    assert!(fg(&h).iter().all(|v| *v < 0.02), "{:?}", fg(&h));
}

#[test]
fn the_background_chip_switches_what_the_panel_edits() {
    let mut h = harness(ColorPanelMode::Rgb);
    let bg_before = h.state().session.tools.background;
    let r = last_rects(&h.ctx);
    let bg = r.background.unwrap();
    click(&mut h, bg.right_bottom() - vec2(2.0, 2.0));
    assert!(h.state().ui.color_panel.background);
    let ramp = last_rects(&h.ctx).ramps[1];
    click(&mut h, Pos2::new(ramp.left() + 0.5, ramp.center().y));
    assert!(close(&fg(&h), &[0.2, 0.4, 0.6], 1e-6), "foreground untouched");
    let b = h.state().session.tools.background;
    assert!(b[1] < 0.01 && (b[0] - bg_before[0]).abs() < 0.01, "{b:?}");
    click(&mut h, r.foreground.unwrap().left_top() + vec2(2.0, 2.0));
    assert!(!h.state().ui.color_panel.background);
}

#[test]
fn the_spectrum_picks_white_and_black() {
    let mut h = harness(ColorPanelMode::Hsb);
    let s = last_rects(&h.ctx).spectrum.unwrap();
    click(&mut h, Pos2::new(s.right() - 2.0, s.top() + 2.0));
    assert!(fg(&h).iter().all(|v| *v > 0.99));
    click(&mut h, Pos2::new(s.right() - 2.0, s.bottom() - 2.0));
    assert!(fg(&h).iter().all(|v| *v < 0.01));
}

#[test]
fn hsb_keeps_the_hue_of_a_grey() {
    // Dragging saturation to zero must not reset the hue the user chose.
    let mut h = harness(ColorPanelMode::Hsb);
    let ramps = last_rects(&h.ctx).ramps;
    click(&mut h, Pos2::new(ramps[1].left() + 0.5, ramps[1].center().y));
    assert!(rgb_to_hsb(fg(&h))[1] < 0.5, "the colour is grey now");
    let c = components(h.state(), &h.ctx, &sp());
    assert!((c[0] - 210.0).abs() < 1.0, "hue kept at {c:?}");
    // Raising saturation again brings back the same hue.
    let r = last_rects(&h.ctx).ramps[1];
    click(&mut h, Pos2::new(r.right() - 0.5, r.center().y));
    assert!((rgb_to_hsb(fg(&h))[0] - 210.0).abs() < 1.5, "{:?}", rgb_to_hsb(fg(&h)));
}

#[test]
fn the_controls_fit_short_groups_and_tiny_ones_scroll() {
    for mode in ColorPanelMode::ALL {
        // A compact dock group: the controls shrink to fit, nothing is cut off.
        let h = harness_sized(mode, vec2(290.0, 100.0));
        let r = last_rects(&h.ctx);
        let bottom = r.ramps.iter().map(|x| x.bottom()).chain(r.field.map(|f| f.bottom())).chain(r.spectrum.map(|f| f.bottom())).fold(0.0f32, f32::max);
        assert!(bottom <= 100.0, "{mode:?} ends at {bottom}: {r:?}");
        if !mode.is_field() {
            assert_eq!(r.ramps.len(), mode.components().len(), "{mode:?}: every slider shows");
        }
        // Tiny and huge panels don't panic.
        for size in [vec2(40.0, 20.0), vec2(2000.0, 1500.0)] {
            let h = harness_sized(mode, size);
            assert_eq!(last_rects(&h.ctx).mode, Some(mode));
        }
    }
}

#[test]
fn the_panel_menu_switches_modes() {
    // The menu draws every mode (inside a real frame), and picking one switches and remembers it.
    let mut h = Harness::builder().with_size(vec2(240.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            panel_menu(app, ui);
        },
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()),
    );
    h.run_steps(2);
    // The default last: with nothing saved yet, picking it has nothing to write.
    for m in ColorPanelMode::ALL.into_iter().rev() {
        h.get_by_label(tl!(m.label())).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.color_panel.mode, m);
        assert_eq!(h.state().session.prefs().dialogs.get(PREF_KEY), Some(&json!({"mode": m.key()})));
    }
}

fn bytes(h: &Harness<'static, PhotocraftApp>) -> [u8; 3] {
    crate::panels::srgb_bytes(h.state().session.tools.foreground)
}

/// Click the readout's hex field (the last text field: Web Color Sliders have one per channel
/// too), select its text and type `text` one key per frame.
fn type_hex(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::TextInput).last().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
}

/// Type `text` into the `nth` spin button (number field) and leave it with Tab.
fn type_number(h: &mut Harness<'static, PhotocraftApp>, nth: usize, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(nth).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

/// Every mode keeps the editable hex readout, with or without the leading `#`.
#[test]
fn the_hex_readout_takes_a_typed_hex_in_every_mode() {
    for mode in ColorPanelMode::ALL {
        let mut h = harness(mode);
        type_hex(&mut h, "003300");
        assert_eq!(bytes(&h), [0x00, 0x33, 0x00], "{mode:?}");
        type_hex(&mut h, "#ff8000");
        assert_eq!(bytes(&h), [0xff, 0x80, 0x00], "{mode:?}");
    }
}

/// An incomplete entry never changes the colour and the field snaps back when focus leaves.
#[test]
fn the_hex_readout_ignores_partial_input() {
    let mut h = harness(ColorPanelMode::HueCube);
    let before = h.state().session.tools.foreground;
    type_hex(&mut h, "12zz");
    assert_eq!(h.state().session.tools.foreground, before, "invalid input changes nothing");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.get_by_role(egui::accesskit::Role::TextInput).value().as_deref(), Some("336699"), "snaps back to the colour");
}

/// The readout's R, G and B fields are editable too (in the field modes they are the only
/// number fields).
#[test]
fn the_readout_rgb_fields_take_values() {
    let mut h = harness(ColorPanelMode::HueCube);
    h.state_mut().session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    h.run_steps(2);
    type_number(&mut h, 1, "128");
    assert_eq!(bytes(&h), [0x00, 0x80, 0x00]);
}

/// Black has no hue or saturation of its own: typing H, then S, then B must not reset the first
/// two to 0 (which ended on white).
#[test]
fn hsb_fields_take_values_on_black() {
    let mut h = harness(ColorPanelMode::Hsb);
    h.state_mut().session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    h.run_steps(2);
    for (field, typed) in [(0, "120"), (1, "100"), (2, "100")] {
        type_number(&mut h, field, typed);
    }
    assert_eq!(bytes(&h), [0, 255, 0]);
}

/// A single click only picks the chip; a double-click opens the Color Picker on it. Where the
/// chips overlap the foreground is on top.
#[test]
fn double_clicking_a_chip_opens_its_color_picker() {
    let mut h = harness(ColorPanelMode::HueCube);
    let r = last_rects(&h.ctx);
    let (fg, bg) = (r.foreground.unwrap(), r.background.unwrap());
    for (p, target) in [(bg.right_bottom() - vec2(2.0, 2.0), "background"), (bg.left_top() + vec2(2.0, 2.0), "foreground")] {
        h.state_mut().ui.dialogs.clear();
        // Past the last double-click, so this one isn't counted as a triple-click.
        h.run_steps(40);
        click(&mut h, p);
        assert!(h.state().ui.dialogs.is_empty(), "a single click only picks the chip");
        assert_eq!(h.state().ui.color_panel.background, target == "background");
        click(&mut h, p);
        let [d] = h.state().ui.dialogs.as_slice() else { panic!("one Color Picker") };
        assert_eq!(d.fields.get("__colorPicker").and_then(serde_json::Value::as_str), Some(target));
    }
    assert!(fg.contains(bg.left_top() + vec2(2.0, 2.0)));
}

/// Black has no hue of its own, so each chip remembers the hue last picked for it.
#[test]
fn each_chip_keeps_its_own_hue_on_black() {
    let mut h = harness(ColorPanelMode::HueCube);
    h.state_mut().session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    h.state_mut().session.tools.background = [0.0, 0.0, 0.0, 1.0];
    h.run_steps(2);
    let r = last_rects(&h.ctx);
    let (strip, field) = (r.strip.unwrap(), r.field.unwrap());
    // The hue strip runs from red at the top through blue (a third down) and green (two thirds).
    let green = Pos2::new(strip.center().x, strip.top() + strip.height() * 2.0 / 3.0);
    let blue = Pos2::new(strip.center().x, strip.top() + strip.height() / 3.0);
    click(&mut h, green);
    click(&mut h, r.background.unwrap().right_bottom() - vec2(2.0, 2.0));
    click(&mut h, blue);
    click(&mut h, r.foreground.unwrap().left_top() + vec2(2.0, 2.0));
    // The field's top-right: full saturation and brightness.
    click(&mut h, Pos2::new(field.right() - 0.5, field.top() + 0.5));
    let [r, g, b] = bytes(&h);
    assert!(g > 200 && r < 64 && b < 64, "the foreground's green, not the background's blue: {:?}", [r, g, b]);
}

/// Drawing never rewrites the colour: a round trip through HSB, CMYK or Lab isn't exact for every
/// colour (#D8452E isn't), and a rewrite every frame would recolour selected type the moment it
/// was selected. An edit does recolour it.
#[test]
fn idle_modes_leave_selected_type_alone_and_edits_recolour_it() {
    for mode in ColorPanelMode::ALL {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("tools.setColors", json!({"foreground": "#d8452e"})).unwrap();
        let before = app.session.tools.foreground;
        app.ui.tool = crate::state::Tool::Type;
        app.ui.text_edit =
            Some(crate::state::TextEdit { layer: id, caret: 0, anchor: 5, session: "s".into(), created: false, dragging: false, resize: None, preedit: None });
        app.ui.color_panel.mode = mode;
        let mut h = Harness::builder().with_size(vec2(300.0, 220.0)).build_ui_state(|ui, app: &mut PhotocraftApp| panel(app, ui), app);
        h.run_steps(4);
        assert_eq!(h.state().session.tools.foreground, before, "{mode:?}");
        let runs = |h: &Harness<'static, PhotocraftApp>| {
            let st = h.state().session.active().unwrap();
            let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
            t.char_runs().len()
        };
        assert_eq!(runs(&h), 1, "{mode:?}: still one white run");
        if mode == ColorPanelMode::Rgb {
            let ramp = last_rects(&h.ctx).ramps[0];
            click(&mut h, Pos2::new(ramp.left() + 0.5, ramp.center().y));
            assert_eq!(runs(&h), 2, "the edit recolours the selected characters");
        }
    }
}

/// A mode set while the panel is hidden (the control channel) is remembered too, once the
/// pointer is up.
#[test]
fn a_mode_set_by_automation_is_remembered() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let rev = app.session.prefs.rev();
    app.ui.color_panel.mode = ColorPanelMode::HueCube;
    persist(&mut app, &ctx);
    assert_eq!(app.session.prefs.rev(), rev, "the default mode isn't written on a first launch");
    app.ui.color_panel.mode = ColorPanelMode::Web;
    persist(&mut app, &ctx);
    assert_eq!(app.session.prefs().dialogs.get(PREF_KEY), Some(&json!({"mode": "web"})));
    // Back to the default: saved, so it isn't lost to the stored Web.
    app.ui.color_panel.mode = ColorPanelMode::HueCube;
    persist(&mut app, &ctx);
    assert_eq!(app.session.prefs().dialogs.get(PREF_KEY), Some(&json!({"mode": "hueCube"})));
}
