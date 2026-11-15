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
    Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(Document::with_background("Sizes", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
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
