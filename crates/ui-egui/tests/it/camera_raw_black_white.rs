//! Camera Raw's B&W button: the Black & White treatment, with the Color Mixer turned into the
//! B&W Mixer and Vibrance / Saturation gone from the Color panel, as in Camera Raw.
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, p: Value) -> Result<Value, String> {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &p).ok_or("missing handler")?
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn dialog() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#c04020"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_bw_button_switches_the_treatment_and_the_mixer() {
    let mut h = dialog();
    let ctx = h.ctx.clone();
    let tr = |s| photocraft_ui_egui::i18n::tr_ctx(photocraft_ui_egui::i18n::current(), "cameraRaw", s);
    menu(h.state_mut(), &ctx, json!({"ui":{"panels":{"open":["color","colorMixer"]}}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_role_and_label(Role::Slider, tr("Vibrance")).is_some());
    assert!(h.query_by_role_and_label(Role::DisclosureTriangle, tr("Color Mixer")).is_some());
    h.get_by_role_and_label(Role::Button, tr("B&W")).click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["params"]["treatment"], "bw");
    assert!(h.query_by_role_and_label(Role::Slider, tr("Vibrance")).is_none(), "no Vibrance in Black & White");
    assert!(h.query_by_role_and_label(Role::Slider, tr("Saturation")).is_none(), "no Saturation in Black & White");
    assert!(h.query_by_role_and_label(Role::DisclosureTriangle, tr("B&W Mixer")).is_some(), "the mixer is the B&W Mixer");
    for band in ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"] {
        assert!(h.query_by_role_and_label(Role::Slider, tr(band)).is_some(), "B&W Mixer: {band}");
    }
    // The mixer's settings go through the control channel too.
    menu(h.state_mut(), &ctx, json!({"ui":{"set":{"bwMix":[-40,0,0,0,0,30,0,0]}}})).unwrap();
    assert_eq!(inspect(&h)["params"]["bwMix"][0], -40.0);
    h.get_by_role_and_label(Role::Button, tr("B&W")).click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["params"]["treatment"], "color");
    assert!(h.query_by_role_and_label(Role::Slider, tr("Vibrance")).is_some());
    assert!(menu(h.state_mut(), &ctx, json!({"ui":{"set":{"treatment":"sepia"}}})).is_err(), "only color or bw");
    assert_eq!(inspect(&h)["params"]["treatment"], "color");
}
