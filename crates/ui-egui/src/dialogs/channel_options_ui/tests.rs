use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    app.run("select.saveSelection", json!({})).unwrap();
    app
}

fn channel(app: &PhotocraftApp) -> photocraft_doc::AlphaChannel {
    app.session.active().unwrap().doc.channels[0].clone()
}

fn values(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog is open").fields().clone()
}

fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
}

fn close_all(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    while let Some(id) = app.framework.dialogs.stack().top() {
        app.framework.dialogs.cancel_by_id(&ctx, id);
    }
}

#[test]
fn saved_selection_becomes_a_spot_channel_with_its_ink_and_solidity() {
    let mut app = app();
    let ctx = app.ctx().clone();
    assert!(channel(&app).spot.is_none());
    let id = open(&mut app, &json!(0)).unwrap();
    crate::dialogs::testing::show(&mut app);
    let f = values(&app, id);
    assert_eq!((f.text("indicates"), f.text("name")), (Some("masked"), Some(channel(&app).name.as_str())));
    set(&mut app, id, "name", json!("PMS 185"));
    set(&mut app, id, "indicates", json!("spot"));
    set(&mut app, id, "color", json!([1.0, 0.0, 0.0]));
    set(&mut app, id, "opacity", json!(40.0));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(app.framework.dialogs.is_empty());
    let ch = channel(&app);
    assert_eq!(ch.name, "PMS 185");
    let (ink, solidity) = ch.spot.expect("a spot channel");
    assert_eq!(ink.to_rgb(), [1.0, 0.0, 0.0]);
    assert!((solidity - 0.4).abs() < 1e-6);

    // Reopened, the dialog shows the spot settings; switching back makes it an alpha channel.
    let id = open(&mut app, &json!(0)).unwrap();
    crate::dialogs::testing::show(&mut app);
    let f = values(&app, id);
    assert_eq!((f.text("indicates"), f.color("color"), f.number("opacity")), (Some("spot"), Some([1.0, 0.0, 0.0]), Some(40.0)));
    set(&mut app, id, "indicates", json!("selected"));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let ch = channel(&app);
    assert!(ch.spot.is_none());
    assert_eq!(ch.indicates, ColorIndicates::SelectedAreas);
}

#[test]
fn menu_opens_the_dialog_on_the_targeted_channel_and_refuses_otherwise() {
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = empty.ctx().clone();
    assert!(crate::menus::invoke(&mut empty, &ctx, COMMAND, json!({})).is_err());

    let mut app = app();
    let ctx = app.ctx().clone();
    // The composite is targeted: nothing to edit.
    assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).is_err());
    assert!(app.framework.dialogs.is_empty());
    app.run("channel.target", json!({"channel": 0})).unwrap();
    let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert!(r["dialog"].is_string());
    assert_eq!(params::captured(&app, COMMAND)["channel"], json!(0));
    // Explicit params run the engine directly, no dialog.
    close_all(&mut app);
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"channel": 0, "indicates": "spot"})).unwrap();
    assert!(app.framework.dialogs.is_empty());
    assert!(channel(&app).spot.is_some());
}

#[test]
fn quick_mask_options_have_no_name_or_spot_and_bad_channels_open_nothing() {
    let mut app = app();
    let f = fields(&app, &json!("quickMask")).unwrap();
    assert!(!f.contains("name"));
    let keys: Vec<String> = rows(Kind::QuickMask, false).into_iter().map(|p| p.key).collect();
    assert_eq!(keys, ["indicates", "color", "opacity"]);
    assert!(fields(&app, &json!(7)).is_none());
    assert!(fields(&app, &json!("nope")).is_none());
    assert!(open(&mut app, &json!(-1)).is_none());
    assert!(app.framework.dialogs.is_empty());
}

/// Layer Mask Display Options: just the colour and opacity, from and to the
/// `layer_mask_overlay` preference; Channel Options… opens it on a targeted layer mask.
#[test]
fn layer_mask_display_options_set_the_rubylith() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let f = fields(&app, &json!("mask")).unwrap();
    assert!(!f.contains("name") && !f.contains("indicates"));
    assert_eq!((f.color("color"), f.number("opacity")), (Some([1.0, 0.0, 0.0]), Some(50.0)));
    assert_eq!(params(&f, &json!("mask")), json!({"channel": "mask", "color": "#ff0000", "opacity": 50.0}));
    let id = open(&mut app, &json!("mask")).unwrap();
    crate::dialogs::testing::show(&mut app);
    set(&mut app, id, "color", json!([0.0, 1.0, 0.0]));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(app.session.prefs().layer_mask_overlay.color, "#00ff00");
    // Channels › Channel Options… with the layer mask targeted.
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.target", json!({"channel": "mask"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert_eq!(params::captured(&app, COMMAND)["channel"], json!("mask"));
    // Or with it targeted by `\` (the rubylith), which leaves the Channels target alone.
    app.run("channel.target", json!({"channel": "composite"})).unwrap();
    close_all(&mut app);
    app.run(photocraft_engine::mask_view_cmds::OVERLAY_ID, json!({})).unwrap();
    app.sync_mask_targets();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert_eq!(params::captured(&app, COMMAND)["channel"], json!("mask"));
}

#[test]
fn new_spot_channel_dialog_creates_the_ink_from_the_selection_and_remembers_it() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    let id = open_new_spot(&mut app).unwrap();
    crate::dialogs::testing::show(&mut app);
    let f = values(&app, id);
    assert_eq!((f.text("name"), f.color("color"), f.number("solidity")), (Some("Spot Color 1"), Some([1.0, 0.0, 0.0]), Some(0.0)));
    set(&mut app, id, "name", json!("PMS 300"));
    set(&mut app, id, "color", json!([0.0, 80.0 / 255.0, 1.0]));
    set(&mut app, id, "solidity", json!(60.0));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let ch = doc.channels.last().unwrap();
    assert_eq!(ch.name, "PMS 300");
    let (ink, solidity) = ch.spot.expect("a spot channel");
    assert_eq!(ink.to_rgba8()[..3], [0x00, 0x50, 0xff]);
    assert!((solidity - 0.6).abs() < 1e-6);
    // Inked where the selection was, empty outside it.
    let (inside, outside) = (ch.surface.rgba(4, 4)[0], ch.surface.rgba(14, 14)[0]);
    assert!(inside > 0.99 && outside < 0.01, "{inside} {outside}");

    // The next one gets a fresh name and the last ink.
    let id = open_new_spot(&mut app).unwrap();
    crate::dialogs::testing::show(&mut app);
    let f = values(&app, id);
    assert_eq!(f.text("name"), Some("Spot Color 1"));
    assert_eq!(hex(f.color("color").unwrap()), "#0050ff");
}
