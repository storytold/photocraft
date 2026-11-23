//! Color Settings' Settings menu in the dialog (#14229): a picked setup loads into the controls
//! (Blend Text Colors Using Gamma excepted), any change shows Custom, OK stores the name and
//! Cancel discards the pick.

use craft::ui::dialog::DialogId;

use super::*;
use crate::control::{ControlRequest, Reply};
use crate::new_layer_ui::testing::top;

/// An app whose Color Settings hold `settings`.
fn app_with(settings: ColorSettings) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.color_mut().settings = settings;
    app
}

/// Opens the dialog from the menu and runs `passes` passes of the dialog host.
fn open_dialog(app: &mut PhotocraftApp, passes: usize) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, COMMAND, json!({})).unwrap();
    let id = top(app);
    show(app, passes);
    id
}

fn show(app: &mut PhotocraftApp, passes: usize) {
    for _ in 0..passes {
        crate::dialogs::testing::show(app);
    }
}

/// The open dialog's values as JSON.
fn fields(app: &PhotocraftApp, id: DialogId) -> Map<String, Value> {
    fields_map(crate::new_layer_ui::testing::fields(app, id))
}

/// `ui.dialog.set`, as a script sends it.
fn dialog_set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    let (request, _) = ControlRequest::new("ui.dialog.set", json!({"dialog": id.value().to_string(), "field": field, "value": value}));
    let Reply::Done(reply) = crate::control::handle(app, &ctx, &request) else { panic!("ui.dialog.set should finish immediately") };
    assert_eq!(reply["ok"], true, "{field} = {value}: {reply}");
}

#[test]
fn opening_shows_the_stored_setup_and_keeps_its_values() {
    let stored = preset_setup(&ColorSettings::default(), "North America Prepress 2", "srgb").unwrap();
    let mut app = app_with(stored.clone());
    let id = open_dialog(&mut app, 3);
    let f = fields(&app, id);
    assert_eq!(f["preset"], "North America Prepress 2");
    let stored = serde_json::to_value(&stored).unwrap();
    let stored = stored.as_object().unwrap();
    for (k, v) in stored.iter().filter(|(k, _)| f.contains_key(*k)) {
        assert!(same(v, &f[k]), "{k}: {v} became {}", f[k]);
    }
}

#[test]
fn picking_a_setup_loads_it_but_keeps_the_text_gamma() {
    // Blend Text Colors Using Gamma off (1.0) before the pick.
    let mut app = app_with(ColorSettings { blend_text_gamma: 1.0, ..ColorSettings::default() });
    let id = open_dialog(&mut app, 1);
    assert_eq!(fields(&app, id)["preset"], CUSTOM_SETTINGS, "PhotoCraft's defaults match no setup");
    dialog_set(&mut app, id, "preset", json!("Europe Prepress 3"));
    show(&mut app, 3);
    let f = fields(&app, id);
    assert_eq!(f["preset"], "Europe Prepress 3", "loading the values is no edit");
    assert_eq!((&f["workingRgb"], &f["workingCmyk"], &f["workingGray"]), (&json!("adobe-rgb-compat"), &json!("coated-fogra39"), &json!("dot-gain-20-compat")));
    assert_eq!((&f["askOnMismatch"], &f["askOnPaste"], &f["askOnMissing"]), (&json!(true), &json!(true), &json!(true)));
    assert!(same(&f["spotDotGain"], &json!(15.0)), "{}", f["spotDotGain"]);
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
        let mut app = app_with(ColorSettings::default());
        let id = open_dialog(&mut app, 1);
        dialog_set(&mut app, id, "preset", json!("Japan Prepress 2"));
        show(&mut app, 2);
        assert_eq!(fields(&app, id)["preset"], "Japan Prepress 2");
        // As the control would change it.
        crate::new_layer_ui::testing::set(&mut app, id, key, value).unwrap();
        show(&mut app, 1);
        assert_eq!(fields(&app, id)["preset"], CUSTOM_SETTINGS, "{key}");
        // It stays Custom, and keeps the values.
        show(&mut app, 2);
        let f = fields(&app, id);
        assert_eq!(f["preset"], CUSTOM_SETTINGS, "{key}");
        assert_eq!(f["workingCmyk"], "coated-cmyk", "{key}");
    }
}

#[test]
fn the_monitor_profile_is_no_change() {
    let mut app = app_with(ColorSettings::default());
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("North America Web/Internet"));
    show(&mut app, 2);
    dialog_set(&mut app, id, "monitorProfile", json!("display-p3"));
    show(&mut app, 2);
    assert_eq!(fields(&app, id)["preset"], "North America Web/Internet");
}

#[test]
fn monitor_color_takes_the_monitor_profile_the_dialog_shows() {
    let mut app = app_with(ColorSettings { monitor_profile: "display-p3".into(), ..ColorSettings::default() });
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("Monitor Color"));
    show(&mut app, 2);
    let f = fields(&app, id);
    assert_eq!(f["preset"], "Monitor Color");
    assert_eq!(f["workingRgb"], app.session.color().monitor_rgb_for("display-p3"));
    assert_eq!((&f["policyRgb"], &f["policyCmyk"], &f["policyGray"]), (&json!("off"), &json!("off"), &json!("off")));
}

#[test]
fn the_greyed_label_and_unknown_names_are_no_entry() {
    for bad in [json!("Other"), json!("Nowhere")] {
        let mut app = app_with(ColorSettings::default());
        let id = open_dialog(&mut app, 1);
        dialog_set(&mut app, id, "preset", bad.clone());
        show(&mut app, 1);
        assert_eq!(fields(&app, id)["preset"], CUSTOM_SETTINGS, "{bad}");
    }
}

#[test]
fn ok_stores_the_setup_and_cancel_discards_it() {
    let mut app = app_with(ColorSettings::default());
    // Cancel: nothing changes.
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("North America General Purpose 2"));
    show(&mut app, 2);
    crate::dialogs::testing::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.color().settings, ColorSettings::default());
    // OK: the setup's name and values.
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("North America General Purpose 2"));
    show(&mut app, 2);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let c = &app.session.color().settings;
    assert_eq!(c.preset, "North America General Purpose 2");
    assert_eq!((c.ask_on_mismatch, c.ask_on_paste, c.ask_on_missing), (false, false, false));
    // Opened again, it shows that setup; an edit then OK stores Custom.
    let id = open_dialog(&mut app, 1);
    assert_eq!(fields(&app, id)["preset"], "North America General Purpose 2");
    dialog_set(&mut app, id, "dither", json!(false));
    assert_eq!(fields(&app, id)["preset"], CUSTOM_SETTINGS);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.color().settings.preset, CUSTOM_SETTINGS);
    assert!(!app.session.color().settings.dither);
}

#[test]
fn a_setup_set_by_script_loads_before_the_next_draw() {
    let expected = preset_setup(&ColorSettings::default(), "Europe Prepress 3", "srgb").unwrap();
    let mut app = app_with(ColorSettings::default());
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("Europe Prepress 3"));
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let c = &app.session.color().settings;
    assert_eq!(c.preset, "Europe Prepress 3");
    assert_eq!((&c.working_rgb, &c.working_cmyk), (&expected.working_rgb, &expected.working_cmyk));
    assert_eq!((c.ask_on_mismatch, c.ask_on_paste, c.ask_on_missing), (true, true, true));
}

#[test]
fn a_control_set_by_script_shows_custom_before_the_next_draw() {
    let name = "North America General Purpose 2";
    let stored = preset_setup(&ColorSettings::default(), name, "srgb").unwrap();
    // The engine keeps a named setup's name over the first two (gamma is no part of a setup, and
    // the setup loads over a reset), so only the dialog can tell they changed. Setting the value a
    // control already shows is no change.
    for (key, value, expected) in
        [("blendTextGamma", json!(1.8), CUSTOM_SETTINGS), ("reset", json!(true), CUSTOM_SETTINGS), ("blendTextGamma", json!(1.45), name)]
    {
        // A fresh app each time: the dialog reopens with the last confirmed values.
        let mut app = app_with(stored.clone());
        let id = open_dialog(&mut app, 1);
        dialog_set(&mut app, id, key, value.clone());
        assert_eq!(fields(&app, id)["preset"], expected, "{key} {value}");
        crate::dialogs::testing::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.color().settings.preset, expected, "{key} {value}");
    }
}

#[test]
fn a_control_set_before_the_first_pass_shows_custom() {
    let name = "North America General Purpose 2";
    let stored = preset_setup(&ColorSettings::default(), name, "srgb").unwrap();
    // Before the first pass the dialog has no baseline yet: a changed control is still an edit,
    // and setting the value it already shows is none.
    for (value, expected) in [(json!(1.8), CUSTOM_SETTINGS), (json!(1.45), name)] {
        let mut app = app_with(stored.clone());
        let id = open_dialog(&mut app, 0);
        dialog_set(&mut app, id, "blendTextGamma", value.clone());
        assert_eq!(fields(&app, id)["preset"], expected, "{value}");
        show(&mut app, 2);
        assert_eq!(fields(&app, id)["preset"], expected, "{value} after a draw");
        crate::dialogs::testing::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.color().settings.preset, expected, "{value}");
    }
}

#[test]
fn reset_set_before_the_first_pass_shows_custom() {
    let name = "North America General Purpose 2";
    let stored = preset_setup(&ColorSettings::default(), name, "srgb").unwrap();
    for (value, expected) in [(json!(true), CUSTOM_SETTINGS), (json!(false), name)] {
        let mut app = app_with(stored.clone());
        let id = open_dialog(&mut app, 0);
        dialog_set(&mut app, id, "reset", value.clone());
        assert_eq!(fields(&app, id)["preset"], expected, "reset {value}");
    }
}

#[test]
fn monitor_color_follows_a_monitor_profile_set_before_the_first_pass() {
    let stored = preset_setup(&ColorSettings { monitor_profile: "srgb".into(), ..ColorSettings::default() }, "Monitor Color", "srgb").unwrap();
    let mut app = app_with(stored);
    let id = open_dialog(&mut app, 0);
    dialog_set(&mut app, id, "monitorProfile", json!("display-p3"));
    let f = fields(&app, id);
    assert_eq!((&f["preset"], &f["workingRgb"]), (&json!("Monitor Color"), &json!("display-p3")), "loaded at once");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let c = &app.session.color().settings;
    assert_eq!((c.preset.as_str(), c.working_rgb.as_str()), ("Monitor Color", "display-p3"));
}

#[test]
fn a_setup_set_before_the_first_pass_loads_too() {
    let expected = preset_setup(&ColorSettings::default(), "Europe Prepress 3", "srgb").unwrap();
    let mut app = app_with(ColorSettings::default());
    let id = open_dialog(&mut app, 0);
    dialog_set(&mut app, id, "preset", json!("Europe Prepress 3"));
    let f = fields(&app, id);
    assert_eq!((&f["workingRgb"], &f["workingCmyk"]), (&json!(expected.working_rgb), &json!(expected.working_cmyk)), "loaded at once");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let c = &app.session.color().settings;
    assert_eq!(c.preset, "Europe Prepress 3");
    assert_eq!((&c.working_rgb, &c.working_cmyk), (&expected.working_rgb, &expected.working_cmyk));
    assert_eq!((c.ask_on_mismatch, c.ask_on_paste, c.ask_on_missing), (true, true, true));
}

#[test]
fn monitor_color_follows_a_monitor_profile_change() {
    let mut app = app_with(ColorSettings { monitor_profile: "srgb".into(), ..ColorSettings::default() });
    let id = open_dialog(&mut app, 1);
    dialog_set(&mut app, id, "preset", json!("Monitor Color"));
    show(&mut app, 2);
    assert_eq!(fields(&app, id)["workingRgb"], "srgb");
    dialog_set(&mut app, id, "monitorProfile", json!("display-p3"));
    let f = fields(&app, id);
    assert_eq!(f["preset"], "Monitor Color");
    assert_eq!(f["workingRgb"], "display-p3", "the working RGB follows the monitor profile");
    show(&mut app, 2);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let c = &app.session.color().settings;
    assert_eq!((c.preset.as_str(), c.working_rgb.as_str(), c.monitor_profile.as_str()), ("Monitor Color", "display-p3", "display-p3"));
}
