//! Camera Raw's Optics panel (its manual corrections): Distortion and the lens Vignette's Amount
//! and Midpoint, Midpoint dimmed while Amount is 0; the eye button drops them from the preview.
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, p: Value) -> Result<Value, String> {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &p).ok_or("missing handler")?
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

#[test]
fn optics_sliders_drive_the_lens_settings() {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    let tr = |s| photocraft_ui_egui::i18n::tr_ctx(photocraft_ui_egui::i18n::current(), "cameraRaw", s);
    menu(h.state_mut(), &ctx, json!({"ui":{"panels":{"open":["optics"]}}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_role_and_label(Role::DisclosureTriangle, tr("Optics")).is_some());
    assert!(h.query_by_role_and_label(Role::Slider, tr("Distortion")).is_some());
    let midpoint = h.get_by_role_and_label(Role::Slider, tr("Midpoint"));
    assert!(midpoint.accesskit_node().is_disabled(), "Midpoint is dimmed while the lens Vignette is 0");
    menu(h.state_mut(), &ctx, json!({"ui":{"set":{"lensDistortion":20,"lensVignette":-30}}})).unwrap();
    h.run_steps(3);
    assert!(!h.get_by_role_and_label(Role::Slider, tr("Midpoint")).accesskit_node().is_disabled(), "Midpoint works once Amount moves");
    let p = &inspect(&h)["params"];
    assert_eq!((p["lensDistortion"].as_f64(), p["lensVignette"].as_f64()), (Some(20.0), Some(-30.0)));
    assert!(menu(h.state_mut(), &ctx, json!({"ui":{"set":{"lensDistortion":500}}})).is_err(), "out of range");
    assert_eq!(inspect(&h)["params"]["lensDistortion"], 20.0);
    // The eye button: the settings stay in the dialog, hidden from the preview and OK.
    menu(h.state_mut(), &ctx, json!({"ui":{"panels":{"hidden":["optics"]}}})).unwrap();
    assert_eq!(inspect(&h)["params"]["lensDistortion"], 20.0);
    assert_eq!(inspect(&h)["panels"]["hidden"], json!(["optics"]));
}
