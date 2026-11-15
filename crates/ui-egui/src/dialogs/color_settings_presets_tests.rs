//! Color Settings' Settings menu in the dialog (#14229): a picked setup loads into the controls
//! (Blend Text Colors Using Gamma excepted), any change shows Custom, OK stores the name and
//! Cancel discards the pick.

use super::*;
use crate::PhotocraftApp;

/// The dialog's fields as the opener fills them: the spec's controls with `settings`' values.
fn fields(settings: &ColorSettings) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("__command".into(), json!(COMMAND));
    if let Value::Object(m) = serde_json::to_value(settings).unwrap() {
        f.extend(m);
    }
    f.insert("reset".into(), json!(false));
    f
}

fn draw(f: &mut Map<String, Value>) {
    egui::Context::default().run_ui(Default::default(), |ui| super::super::body(ui, f)).textures_delta.clear();
}

fn draw_frames(ctx: &egui::Context, f: &mut Map<String, Value>, frames: usize) {
    for _ in 0..frames {
        ctx.run_ui(Default::default(), |ui| super::super::body(ui, f)).textures_delta.clear();
    }
}

#[test]
fn opening_shows_the_stored_setup_and_keeps_its_values() {
    let stored = preset_setup(&ColorSettings::default(), "North America Prepress 2", "srgb").unwrap();
    let mut f = fields(&stored);
    let before = f.clone();
    let ctx = egui::Context::default();
    draw_frames(&ctx, &mut f, 3);
    assert_eq!(f["preset"], "North America Prepress 2");
    for (k, v) in &before {
        assert!(same(v, &f[k]), "{k}: {v} became {}", f[k]);
    }
}

#[test]
fn picking_a_setup_loads_it_but_keeps_the_text_gamma() {
    // Blend Text Colors Using Gamma off (1.0) before the pick.
    let mut f = fields(&ColorSettings { blend_text_gamma: 1.0, ..ColorSettings::default() });
    let ctx = egui::Context::default();
    draw_frames(&ctx, &mut f, 1);
    assert_eq!(f["preset"], CUSTOM_SETTINGS, "PhotoCraft's defaults match no setup");
    // As the menu or `ui.dialog.set` picks it.
    f.insert("preset".into(), json!("Europe Prepress 3"));
    draw_frames(&ctx, &mut f, 3);
    assert_eq!(f["preset"], "Europe Prepress 3", "loading the values is no edit");
    assert_eq!((&f["workingRgb"], &f["workingCmyk"], &f["workingGray"]), (&json!("adobe-rgb-compat"), &json!("coated-fogra39"), &json!("dot-gain-20-compat")));
    assert_eq!((&f["askOnMismatch"], &f["askOnPaste"], &f["askOnMissing"]), (&json!(true), &json!(true), &json!(true)));
    assert!(same(&f["spotDotGain"], &json!(15.0)));
    assert!(same(&f["blendTextGamma"], &json!(1.0)), "kept: {}", f["blendTextGamma"]);
}

#[test]
fn changing_any_control_shows_custom() {
    for (key, value) in [
        ("workingRgb", json!("display-p3")),
        ("policyGray", json!("off")),
        ("askOnPaste", json!(false)),
        ("intent", json!("saturation")),
        ("bpc", json!(false)),
        ("spotDotGain", json!(30.0)),
        ("blendTextGamma", json!(1.8)),
        ("reset", json!(true)),
    ] {
        let mut f = fields(&ColorSettings::default());
        let ctx = egui::Context::default();
        draw_frames(&ctx, &mut f, 1);
        f.insert("preset".into(), json!("Japan Prepress 2"));
        draw_frames(&ctx, &mut f, 2);
        assert_eq!(f["preset"], "Japan Prepress 2");
        f.insert(key.into(), value);
        draw_frames(&ctx, &mut f, 1);
        assert_eq!(f["preset"], CUSTOM_SETTINGS, "{key}");
        // It stays Custom, and keeps the values.
        draw_frames(&ctx, &mut f, 2);
        assert_eq!(f["preset"], CUSTOM_SETTINGS, "{key}");
        assert_eq!(f["workingCmyk"], "coated-cmyk", "{key}");
    }
}

#[test]
fn the_monitor_profile_is_no_change() {
    let mut f = fields(&ColorSettings::default());
    let ctx = egui::Context::default();
    draw_frames(&ctx, &mut f, 1);
    f.insert("preset".into(), json!("North America Web/Internet"));
    draw_frames(&ctx, &mut f, 2);
    f.insert("monitorProfile".into(), json!("display-p3"));
    draw_frames(&ctx, &mut f, 2);
    assert_eq!(f["preset"], "North America Web/Internet");
}

#[test]
fn monitor_color_takes_the_monitor_profile_in_use() {
    let mut f = fields(&ColorSettings::default());
    f.insert(MONITOR_RGB.into(), json!("display-p3"));
    let ctx = egui::Context::default();
    draw_frames(&ctx, &mut f, 1);
    f.insert("preset".into(), json!("Monitor Color"));
    draw_frames(&ctx, &mut f, 2);
    assert_eq!(f["preset"], "Monitor Color");
    assert_eq!(f["workingRgb"], "display-p3");
    assert_eq!((&f["policyRgb"], &f["policyCmyk"], &f["policyGray"]), (&json!("off"), &json!("off"), &json!("off")));
}

#[test]
fn the_greyed_label_and_unknown_names_are_no_entry() {
    for bad in [json!("Other"), json!("Nowhere"), json!(7)] {
        let mut f = fields(&ColorSettings::default());
        f.insert("preset".into(), bad.clone());
        draw(&mut f);
        assert_eq!(f["preset"], CUSTOM_SETTINGS, "{bad}");
    }
}

/// The real dialog: opened from the menu, set through the control channel, then OK or Cancel.
fn open_dialog(app: &mut PhotocraftApp, ctx: &egui::Context) -> u64 {
    crate::menus::invoke(app, ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn show(app: &mut PhotocraftApp, ctx: &egui::Context, frames: usize) {
    for _ in 0..frames {
        ctx.run_ui(Default::default(), |ui| crate::dialogs::show(app, ui.ctx())).textures_delta.clear();
    }
}

fn dialog_set(app: &mut PhotocraftApp, ctx: &egui::Context, id: u64, field: &str, value: Value) {
    let (request, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    let crate::control::Outcome::Done(reply) = crate::control::handle(app, ctx, &request) else { panic!("ui.dialog.set should finish immediately") };
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn ok_stores_the_setup_and_cancel_discards_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    // Cancel: nothing changes.
    let id = open_dialog(&mut app, &ctx);
    show(&mut app, &ctx, 1);
    dialog_set(&mut app, &ctx, id, "preset", json!("North America General Purpose 2"));
    show(&mut app, &ctx, 2);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.color.settings, ColorSettings::default());
    // OK: the setup's name and values.
    let id = open_dialog(&mut app, &ctx);
    show(&mut app, &ctx, 1);
    dialog_set(&mut app, &ctx, id, "preset", json!("North America General Purpose 2"));
    show(&mut app, &ctx, 2);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let c = &app.session.color.settings;
    assert_eq!(c.preset, "North America General Purpose 2");
    assert_eq!((c.ask_on_mismatch, c.ask_on_paste, c.ask_on_missing), (false, false, false));
    // Opened again, it shows that setup; an edit then OK stores Custom.
    let id = open_dialog(&mut app, &ctx);
    show(&mut app, &ctx, 1);
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["preset"], "North America General Purpose 2");
    dialog_set(&mut app, &ctx, id, "dither", json!(false));
    show(&mut app, &ctx, 1);
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["preset"], CUSTOM_SETTINGS);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.color.settings.preset, CUSTOM_SETTINGS);
    assert!(!app.session.color.settings.dither);
}
