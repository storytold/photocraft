//! #14622 (the storage and draw half): the toolbar's four Show flags decide whether it draws its
//! "..." (Edit Toolbar) slot, the foreground/background colour chips, the Quick Mask button and the
//! Screen Mode button. They live in `ToolbarCustomization` next to `hidden` and the layout, and load
//! all-on from settings written before they existed; `edit.toolbar` reads and writes them; D, X, Q
//! and F keep working with the items hidden. The dialog's "Show:" row is the other half of the issue
//! and follows on top of #14616.

use craft::ui::shell::ScreenMode;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::prefs::{Preferences, SavedPrefsExt};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::panels::toolbar::{cells, layout_sections};
use crate::shortcut_dispatch::tests::app_harness;
use crate::shortcut_dispatch::tests::tap;
use crate::state::Tool;

/// The four Show flags, as their stored path under `toolbar` and their `edit.toolbar` param.
const FLAGS: [&str; 4] = ["showMore", "showColors", "showQuickMask", "showScreenMode"];

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// An app with a document: the toolbar is drawn as the whole app draws it, which is where the rail's
/// buttons carry their commands' names.
fn doc_app() -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app
}

/// The value of one Show flag in the app's stored preferences.
fn stored(app: &PhotocraftApp, flag: &str) -> Option<Value> {
    app.session.prefs().get(&format!("toolbar.{flag}"))
}

/// Whether the drawn app holds a widget named `text`.
fn shows(harness: &Harness<'static, Box<PhotocraftApp>>, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Whether the item one Show flag decides is drawn: the "..." slot, the colour chips, the Quick Mask
/// button and the Screen Mode button, each under the name the rail gives it.
fn shows_item(harness: &Harness<'static, Box<PhotocraftApp>>, flag: &str) -> bool {
    shows(harness, item_name(flag))
}

/// The accessible name of the item one Show flag decides.
fn item_name(flag: &str) -> &'static str {
    match flag {
        "showMore" => "Edit Toolbar",
        "showColors" => "Switch Foreground and Background Colors",
        "showQuickMask" => "Edit in Quick Mask Mode",
        _ => "Screen Mode",
    }
}

/// `edit.toolbar` params that set one Show flag.
fn flag_params(flag: &str, on: bool) -> Value {
    let mut params = serde_json::Map::new();
    params.insert(flag.to_string(), Value::Bool(on));
    Value::Object(params)
}

/// `edit.toolbar` params that set every Show flag.
fn all_flags(on: bool) -> Value {
    let mut params = serde_json::Map::new();
    for flag in FLAGS {
        params.insert(flag.to_string(), Value::Bool(on));
    }
    Value::Object(params)
}

/// A Show flag over `prefs.set`, which is how the control channel and MCP reach a preference.
fn set_flag(app: &mut PhotocraftApp, flag: &str, value: bool) -> Result<Value, String> {
    app.run("prefs.set", json!({"path": format!("toolbar.{flag}"), "value": value}))
}

#[test]
fn the_four_show_flags_are_stored_and_answer_every_read() {
    let mut app = app();
    for flag in FLAGS {
        assert_eq!(stored(&app, flag), Some(Value::Bool(true)), "{flag}: on with nothing stored");
    }
    // The command answers with the flag it was given and with the ones it did not change.
    let out = app.run("edit.toolbar", flag_params("showColors", false)).unwrap();
    assert_eq!(out["showColors"], json!(false), "edit.toolbar reads back what it wrote");
    assert_eq!(out["showMore"], json!(true), "the flags it was not given stay on");
    assert_eq!(stored(&app, "showColors"), Some(Value::Bool(false)));
    app.run("edit.toolbar", flag_params("showColors", true)).unwrap();
    assert_eq!(stored(&app, "showColors"), Some(Value::Bool(true)), "and go back on");
    // The same values over `prefs.set`.
    set_flag(&mut app, "showScreenMode", false).unwrap();
    assert_eq!(stored(&app, "showScreenMode"), Some(Value::Bool(false)));
    set_flag(&mut app, "showQuickMask", true).unwrap();
    assert_eq!(stored(&app, "showQuickMask"), Some(Value::Bool(true)), "prefs.set reads back as the same value");
    // A Show flag is a bool: anything else is refused and changes nothing.
    assert!(app.run("prefs.set", json!({"path": "toolbar.showMore", "value": "off"})).is_err(), "not a bool");
    assert_eq!(stored(&app, "showMore"), Some(Value::Bool(true)), "the refused write left it alone");
    // Stored with the other toolbar preferences, so they survive a restart (the issue leaves that
    // case to the other toolbar preferences, which are kept).
    app.run("edit.toolbar", flag_params("showQuickMask", false)).unwrap();
    let back: Preferences = serde_json::from_str(&app.session.prefs_to_json()).unwrap();
    assert_eq!(back.get("toolbar.showQuickMask"), Some(Value::Bool(false)), "the settings file carries the flag");
}

#[test]
fn settings_written_before_the_show_flags_load_with_all_four_on() {
    // A settings file from the layout change (#14459): the flat hidden list, no Show fields.
    let prefs: Preferences = serde_json::from_value(json!({"toolbar": {"hidden": ["Sponge", "Move"]}})).unwrap();
    for flag in FLAGS {
        assert_eq!(prefs.get(&format!("toolbar.{flag}")), Some(Value::Bool(true)), "{flag}: an older file loads all-on");
    }
    // No toolbar section at all.
    let prefs: Preferences = serde_json::from_value(json!({})).unwrap();
    for flag in FLAGS {
        assert_eq!(prefs.get(&format!("toolbar.{flag}")), Some(Value::Bool(true)), "{flag}");
    }
    // A file that names some of them keeps those and loads the rest on.
    let prefs: Preferences = serde_json::from_value(json!({"toolbar": {"showColors": false, "showScreenMode": false}})).unwrap();
    assert_eq!(prefs.get("toolbar.showColors"), Some(Value::Bool(false)));
    assert_eq!(prefs.get("toolbar.showScreenMode"), Some(Value::Bool(false)));
    for flag in ["showMore", "showQuickMask"] {
        assert_eq!(prefs.get(&format!("toolbar.{flag}")), Some(Value::Bool(true)), "{flag}");
    }
}

#[test]
fn each_show_flag_hides_its_own_item_and_leaves_the_other_three() {
    for off in FLAGS {
        let mut app = doc_app();
        app.run("edit.toolbar", flag_params(off, false)).unwrap();
        let harness = app_harness(app);
        for flag in FLAGS {
            assert_eq!(shows_item(&harness, flag), flag != off, "{off} off: only its own item goes away");
        }
    }
}

#[test]
fn with_every_show_flag_off_the_toolbar_ends_with_its_last_tool_slot() {
    let all_on = doc_app();
    let cells_on = cells(&all_on);
    let harness = app_harness(all_on);
    for flag in FLAGS {
        assert!(shows_item(&harness, flag), "{flag} on: {name}", name = item_name(flag));
    }

    let mut off = doc_app();
    off.run("edit.toolbar", all_flags(false)).unwrap();
    assert_eq!(cells(&off), cells_on - 1, "the hidden \"...\" slot gives up its grid cell too");
    let harness = app_harness(off);
    for flag in FLAGS {
        assert!(!shows_item(&harness, flag), "{flag} off: its item is not drawn");
    }
    // The measured end of the toolbar: the last tool slot, its tools as they were.
    assert!(shows(&harness, "Hand Tool") && shows(&harness, "Zoom Tool"), "the toolbar still ends with its last tool slot");
    let slots: Vec<Vec<Tool>> = layout_sections(&harness.state().session.prefs().toolbar).into_iter().flatten().map(|(_, tools)| tools).collect();
    assert_eq!(slots.last().and_then(|group| group.last()), Some(&Tool::Zoom), "Zoom is the toolbar's last slot");
}

#[test]
fn the_show_flags_leave_the_layout_and_the_hidden_list_as_they_were() {
    let mut placed = app();
    placed.run("edit.toolbar", json!({"toolbar": [["Move"], ["Lasso", "EllipseMarquee"]], "hidden": ["Sponge"]})).unwrap();
    let mut hidden_off = app();
    hidden_off
        .run("edit.toolbar", json!({"toolbar": [["Move"], ["Lasso", "EllipseMarquee"]], "hidden": ["Sponge"], "showMore": false, "showColors": false}))
        .unwrap();
    assert_eq!(layout_sections(&hidden_off.session.prefs().toolbar), layout_sections(&placed.session.prefs().toolbar), "hiding the items moves no tool");
    let layout = hidden_off.session.prefs().toolbar.layout();
    assert!(layout.extras.iter().any(|group| group == &vec!["Sponge".to_string()]), "the hidden tool is still an extra");
    // A layout write through `edit.toolbar`, which is how the drawer saves an arrangement, keeps the
    // Show flags the user set.
    let mut app = app();
    app.run("edit.toolbar", all_flags(false)).unwrap();
    app.run("edit.toolbar", json!({"toolbar": [["Move"], ["Zoom"]]})).unwrap();
    for flag in FLAGS {
        assert_eq!(stored(&app, flag), Some(Value::Bool(false)), "{flag}: the arrangement write kept it off");
    }
    let layout = app.session.prefs().toolbar.layout();
    assert_eq!(layout.toolbar, vec![vec!["Move".to_string()], vec!["Zoom".to_string()]], "and wrote the layout as before");
}

#[test]
fn reset_turns_every_show_flag_back_on() {
    let mut app = app();
    for flag in FLAGS {
        app.run("edit.toolbar", flag_params(flag, false)).unwrap();
        assert_eq!(stored(&app, flag), Some(Value::Bool(false)), "{flag} off first");
    }
    let out = app.run("edit.toolbar", json!({"reset": true})).unwrap();
    for flag in FLAGS {
        assert_eq!(out[flag], json!(true), "{flag}: reset turns it on");
        assert_eq!(stored(&app, flag), Some(Value::Bool(true)), "{flag}: and stores it on");
    }
}

#[test]
fn d_x_q_and_f_still_work_with_the_items_hidden() {
    let mut app = doc_app();
    app.run("edit.toolbar", all_flags(false)).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    let mut harness = app_harness(app);
    for flag in FLAGS {
        assert!(!shows_item(&harness, flag), "{flag} off: the app draws no such item");
    }
    // D: Default Colors, the keys of the chips that are not drawn.
    tap(&mut harness, Key::D, Modifiers::NONE);
    assert_eq!(harness.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "D sets the foreground");
    assert_eq!(harness.state().session.tools.background, [1.0, 1.0, 1.0, 1.0], "D sets the background");
    // X: Switch Colors.
    tap(&mut harness, Key::X, Modifiers::NONE);
    assert_eq!(harness.state().session.tools.foreground, [1.0, 1.0, 1.0, 1.0], "X switches the foreground up");
    assert_eq!(harness.state().session.tools.background, [0.0, 0.0, 0.0, 1.0], "X switches the background down");
    // Q: Edit in Quick Mask Mode, the key of the button that is not drawn.
    assert!(harness.state().session.active().is_some_and(|session| session.doc.quick_mask.is_none()), "Quick Mask off to start");
    tap(&mut harness, Key::Q, Modifiers::NONE);
    assert!(harness.state().session.active().is_some_and(|session| session.doc.quick_mask.is_some()), "Q turns Quick Mask on");
    tap(&mut harness, Key::Q, Modifiers::NONE);
    assert!(harness.state().session.active().is_some_and(|session| session.doc.quick_mask.is_none()), "Q again turns it off");
    // F: Cycle Screen Mode, the key of the button that is not drawn.
    assert_eq!(harness.state().ui.view.screen_mode, ScreenMode::Standard, "the standard mode to start");
    tap(&mut harness, Key::F, Modifiers::NONE);
    assert_ne!(harness.state().ui.view.screen_mode, ScreenMode::Standard, "F cycles the screen mode");
}
