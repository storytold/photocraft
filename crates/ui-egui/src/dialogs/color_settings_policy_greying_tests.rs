//! #14245: in Color Settings, with the RGB, CMYK and Gray policies all Off, "Ask When Pasting" and "Missing
//! Profiles: Ask When Opening" are greyed with their values kept, and "Profile Mismatches: Ask When Opening" stays
//! enabled; any policy other than Off enables both again.

use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn open() -> (Box<PhotocraftApp>, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let id = crate::filter_dialog::open(&mut app, "edit.colorSettings").unwrap();
    crate::dialogs::testing::show(&mut app);
    (app, id)
}

fn set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) -> Result<(), String> {
    let ctx = app.ctx().clone();
    let r = app.framework.dialogs.set_json_field(&ctx, id, key, value);
    crate::dialogs::testing::show(app);
    r
}

fn field(app: &PhotocraftApp, id: DialogId, key: &str) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).unwrap()[key].clone()
}

/// Whether the row `key` takes input: a disabled row refuses a value.
fn takes_input(app: &mut PhotocraftApp, id: DialogId, key: &str) -> bool {
    let before = field(app, id, key);
    let flipped = json!(!before.as_bool().unwrap());
    let ok = set(app, id, key, flipped).is_ok();
    if ok {
        set(app, id, key, before).unwrap();
    }
    ok
}

fn all_off(app: &mut PhotocraftApp, id: DialogId) {
    for key in ["policyRgb", "policyCmyk", "policyGray"] {
        set(app, id, key, json!("off")).unwrap();
    }
}

#[test]
fn all_policies_off_grey_the_paste_and_missing_boxes() {
    let (mut app, id) = open();
    let (paste, missing) = (field(&app, id, "askOnPaste"), field(&app, id, "askOnMissing"));
    all_off(&mut app, id);
    assert!(!takes_input(&mut app, id, "askOnPaste"), "Ask When Pasting is greyed");
    assert!(!takes_input(&mut app, id, "askOnMissing"), "Missing Profiles is greyed");
    assert!(takes_input(&mut app, id, "askOnMismatch"), "Ask When Opening stays enabled");
    assert_eq!((field(&app, id, "askOnPaste"), field(&app, id, "askOnMissing")), (paste, missing), "their values are kept");
}

#[test]
fn any_policy_back_on_enables_them_again() {
    for (key, value) in [("policyRgb", "preserve"), ("policyCmyk", "convert"), ("policyGray", "preserve")] {
        let (mut app, id) = open();
        all_off(&mut app, id);
        set(&mut app, id, key, json!(value)).unwrap();
        assert!(takes_input(&mut app, id, "askOnPaste") && takes_input(&mut app, id, "askOnMissing"), "{key} {value}");
    }
}
