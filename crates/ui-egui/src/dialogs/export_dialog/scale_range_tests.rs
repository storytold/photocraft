//! #13738: Export As's Scale is an editable list (10..500 % presets, or any value with decimals)
//! and Scale, Width and Height stop at 1..7500 % of the source. A clamped or rounded size shows a
//! warning.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Color, SampleType, Size};

#[test]
fn the_scale_range_is_1_to_7500_percent_with_decimals() {
    let size = |scale: f64| resize_from_scale(200.0, 150.0, scale).map(|s| (s.width, s.height));
    assert_eq!(size(33.3), Ok((67.0, 50.0)));
    assert_eq!(size(250.0), Ok((500.0, 375.0)));
    assert_eq!(size(50.0), Ok((100.0, 75.0)));
    assert_eq!(size(1.0), Ok((2.0, 2.0)), "each side rounded on its own: 1.5 rounds to 2");
    assert_eq!(size(7500.0), Ok((15000.0, 11250.0)));
    assert!(size(7500.5).is_err() && size(0.0).is_err() && size(f64::NAN).is_err());
    assert!(valid_setting("scale", &json!(7500)) && valid_setting("scale", &json!(33.3)));
    assert!(!valid_setting("scale", &json!(7501)) && !valid_setting("scale", &json!(0.5)));
    assert_eq!(max_side_px(200.0), 15000.0);
    assert_eq!(max_side_px(100_000.0), MAX_PX, "the pixel limit still applies on top");
}

#[test]
fn a_recorded_scale_past_7500_is_an_error_and_older_remembered_sizes_load() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let err = replay(&mut app, &json!({"path": "/x.png", "format": "png", "scale": 7600})).unwrap_err();
    assert!(err.contains("scale"), "{err}");
    for (scale, want) in [(37.0, (74.0, 56.0)), (1000.0, (2000.0, 1500.0))] {
        app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"scale": scale})));
        let id = open(&mut app).unwrap();
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!((n(f, "width", 0.0), n(f, "height", 0.0)), want, "a remembered {scale} %");
    }
}

fn harness() -> Harness<'static, PhotocraftApp> {
    harness_sized(200, 150)
}

fn harness_sized(w: u32, h: u32) -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(Document::with_background("Sizes", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
        app
    })
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

/// The dialog's spin button `label`: the one in Scale's column (the app's own options bar can
/// have a field of the same name).
fn type_into(h: &mut Harness<'_, PhotocraftApp>, label: &str, text: &str) {
    let x = h.get_by_role_and_label(Role::SpinButton, "Scale").rect().min.x;
    let field = h.query_all_by_role_and_label(Role::SpinButton, label).find(|n| (n.rect().min.x - x).abs() < 40.0).expect("the dialog's field");
    field.click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
}

/// Like [`type_into`], but the whole text arrives as one event, as a paste does.
fn paste_into(h: &mut Harness<'_, PhotocraftApp>, label: &str, text: &str) {
    let x = h.get_by_role_and_label(Role::SpinButton, "Scale").rect().min.x;
    let field = h.query_all_by_role_and_label(Role::SpinButton, label).find(|n| (n.rect().min.x - x).abs() < 40.0).expect("the dialog's field");
    field.click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(text.to_string()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
}

/// (width, height, scale, warning shown)
fn state(h: &Harness<'_, PhotocraftApp>) -> (f64, f64, f64, bool) {
    let f = &h.state().ui.dialogs.last().expect("Export As is open").fields;
    (n(f, "width", 0.0), n(f, "height", 0.0), (n(f, "scale", 0.0) * 1000.0).round() / 1000.0, h.query_by_label("⚠").is_some())
}

#[test]
fn typed_values_round_clamp_and_warn_as_measured() {
    let mut h = harness();
    open_dialog(&mut h);
    assert_eq!(state(&h), (200.0, 150.0, 100.0, false));
    type_into(&mut h, "Scale", "33.3");
    assert_eq!(state(&h), (67.0, 50.0, 33.3, true), "rounded, with the warning");
    type_into(&mut h, "Scale", "250");
    assert_eq!(state(&h), (500.0, 375.0, 250.0, false), "exact: no warning");
    type_into(&mut h, "Scale", "0");
    assert_eq!(state(&h), (2.0, 2.0, 1.0, true), "0 is 1 %");
    type_into(&mut h, "Scale", "50");
    assert_eq!(state(&h), (100.0, 75.0, 50.0, false));
    type_into(&mut h, "Width", "99999");
    assert_eq!(state(&h), (15000.0, 11250.0, 7500.0, true), "clamped to 7500 %");
}

#[test]
fn the_scale_list_offers_the_twelve_presets_in_order() {
    let mut h = harness();
    open_dialog(&mut h);
    h.get_by_label("▾").click();
    h.run_steps(4);
    let presets = ["10%", "25%", "33%", "50%", "75%", "100%", "125%", "150%", "200%", "300%", "400%", "500%"];
    let ys: Vec<f32> = presets.iter().map(|p| h.get_by_role_and_label(Role::Button, p).rect().min.y).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "in the measured order: {ys:?}");
    h.get_by_role_and_label(Role::Button, "25%").click();
    h.run_steps(4);
    assert_eq!(state(&h), (50.0, 38.0, 25.0, true), "37.5 px rounds to 38");
}

/// CodeRabbit on #15032: a scale whose size passes the pixel limit clamps, with the warning, instead
/// of being dropped (5000 % of 10000 px is 500000 px; the limit is 300000).
#[test]
fn a_scale_past_the_pixel_limit_clamps_with_the_warning() {
    assert_eq!(max_scale(10_000.0, 10_000.0), 3000.0);
    assert_eq!(max_scale(200.0, 150.0), 7500.0);
    let mut h = harness_sized(10_000, 10_000);
    open_dialog(&mut h);
    type_into(&mut h, "Scale", "5000");
    assert_eq!(state(&h), (300_000.0, 300_000.0, 3000.0, true));
}

/// CodeRabbit on #15032: a remembered exact pixel size whose scale shows rounded stays pixels on an
/// idle frame (30 x 40 with a linked width of 20 is 66.667 %, but exactly 20 px).
#[test]
fn an_idle_frame_does_not_turn_a_pixel_size_into_a_scale() {
    let mut h = harness_sized(30, 40);
    open_dialog(&mut h);
    let f = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
    f.insert("sizing".into(), json!("pixels"));
    f.insert("width".into(), json!(20.0));
    f.insert("height".into(), json!(27.0));
    f.insert("scale".into(), json!(200.0 / 3.0));
    h.run_steps(8);
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!(s(f, "sizing"), "pixels");
    assert_eq!((n(f, "width", 0.0), n(f, "height", 0.0)), (20.0, 27.0));
    assert!((n(f, "scale", 0.0) - 200.0 / 3.0).abs() < 1e-9, "the stored scale is untouched: {}", n(f, "scale", 0.0));
}

/// CodeRabbit on #15032: Width and Height both stop at 1 % of their source side, so a linked 1 px on a
/// 2000 x 1000 source is 20 x 10 at 1 %, with the warning.
#[test]
fn width_and_height_stop_at_one_percent_of_the_source() {
    assert_eq!((min_side_px(2000.0), min_side_px(1000.0), min_side_px(150.0), min_side_px(201.0), min_side_px(1.0)), (20.0, 10.0, 2.0, 2.0, 1.0));
    let mut h = harness_sized(2000, 1000);
    open_dialog(&mut h);
    type_into(&mut h, "Width", "1");
    assert_eq!(state(&h), (20.0, 10.0, 1.0, true));
    type_into(&mut h, "Scale", "50");
    type_into(&mut h, "Height", "1");
    assert_eq!(state(&h), (20.0, 10.0, 1.0, true));
}

/// CodeRabbit on #15032: a preset past the pixel limit applies clamped and says so.
#[test]
fn a_preset_past_the_limit_clamps_with_the_warning() {
    let mut h = harness_sized(100_000, 100_000);
    open_dialog(&mut h);
    h.get_by_label("▾").click();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "500%").click();
    h.run_steps(4);
    assert_eq!(state(&h), (300_000.0, 300_000.0, 300.0, true));
    // Exactly reaching the limit with a preset is still a clamp: 100000 x 1 at 400 %.
    let mut h = harness_sized(100_000, 1);
    open_dialog(&mut h);
    h.get_by_label("▾").click();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "400%").click();
    h.run_steps(4);
    assert_eq!(state(&h), (300_000.0, 3.0, 300.0, true));
}

/// CodeRabbit on #15032: the 1 % that Scale itself gives is not pushed up by the side bounds.
#[test]
fn a_valid_scale_result_is_not_resized_again_by_the_side_bounds() {
    let mut h = harness_sized(201, 151);
    open_dialog(&mut h);
    type_into(&mut h, "Scale", "1");
    h.run_steps(6);
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!((n(f, "width", 0.0), n(f, "height", 0.0), s(f, "sizing")), (2.0, 2.0, "scale".to_string()));
}

/// CodeRabbit on #15032: with Constrain proportions the edited side is also bounded by the linked
/// side's limit: 4000 px wide on a 100 x 10000 source would be 400000 px tall.
#[test]
fn a_linked_side_limits_the_edited_one() {
    assert_eq!(side_range(100.0, 10_000.0, true), (1.0, 3000.0));
    assert_eq!(side_range(100.0, 10_000.0, false), (1.0, 7500.0));
    let mut h = harness_sized(100, 10_000);
    open_dialog(&mut h);
    type_into(&mut h, "Width", "4000");
    assert_eq!(state(&h), (3000.0, 300_000.0, 3000.0, true));
}

/// CodeRabbit on #15032: the linked upper bound applies to edits only. 7498 % of a 1 x 4001 source is a
/// valid 75 x 299995, one pixel wider than the linked bound allows a typed width; it stays as it is.
#[test]
fn a_valid_scale_result_just_above_the_linked_bound_is_left_alone() {
    assert_eq!(side_range(1.0, 4001.0, true).1, 74.0);
    let mut h = harness_sized(1, 4001);
    open_dialog(&mut h);
    type_into(&mut h, "Scale", "7498");
    h.run_steps(6);
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!((n(f, "width", 0.0), n(f, "height", 0.0), s(f, "sizing")), (75.0, 299_995.0, "scale".to_string()));
    // Typing a width past the bound still clamps, with the warning.
    type_into(&mut h, "Width", "200");
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    assert!(n(f, "width", 0.0) <= 75.0 && n(f, "height", 0.0) <= 300_000.0, "{:?}", (n(f, "width", 0.0), n(f, "height", 0.0)));
}

/// CodeRabbit on #15032: a width pasted in one go above the linked bound is clamped to that bound (74 px,
/// not the stored 75) with the warning, not rejected: on 1 x 4001 at 7498 %, 200 gives 74 x 296074.
#[test]
fn a_pasted_width_above_the_linked_bound_is_clamped_to_it() {
    let mut h = harness_sized(1, 4001);
    open_dialog(&mut h);
    type_into(&mut h, "Scale", "7498");
    assert_eq!(state(&h).0, 75.0);
    paste_into(&mut h, "Width", "200");
    let (w, hh, _, warn) = state(&h);
    assert_eq!((w, hh, warn), (74.0, 296_074.0, true));
}
