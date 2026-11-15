//! #14208: in Edit › Keyboard Shortcuts, tools of one toolbar group may share a letter, as their
//! defaults do (L for the three lassos), and ⇧+letter cycles the tools sharing it. A letter held
//! by a tool of another group still moves, with the warning.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

fn dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("filter".into(), json!("lasso"));
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

/// Select `command` in the dialog and record a plain `k` as its shortcut.
fn assign(h: &mut Harness<'_, PhotocraftApp>, id: u64, command: &str, k: Key) {
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("selected".into(), json!(command));
    f.insert("capture".into(), json!(true));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

/// A tap of `key` with `modifiers`, outside any dialog.
fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    ctx.begin_pass(egui::RawInput {
        events: vec![
            Event::ModifiersChanged(modifiers),
            Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
            Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
        ],
        ..Default::default()
    });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn default_shift_l_cycles_the_lassos() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.ui.tool = Tool::Brush;
    press(&mut app, &ctx, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Lasso);
    for expected in [Tool::PolygonLasso, Tool::MagneticLasso, Tool::Lasso] {
        press(&mut app, &ctx, Key::L, Modifiers::SHIFT);
        assert_eq!(app.ui.tool, expected);
    }
}

#[test]
fn the_lassos_can_share_a_new_letter_in_the_dialog() {
    let (mut h, id) = dialog();
    for tool in ["lasso", "polygonLasso", "magneticLasso"] {
        assign(&mut h, id, &format!("tools.select.{tool}"), Key::Q);
        let message = field(&h, id, "message");
        assert!(!message.as_str().unwrap().contains("Lasso"), "{tool}: {message}");
    }
    let overrides = field(&h, id, "overrides");
    for tool in ["lasso", "polygonLasso", "magneticLasso"] {
        assert_eq!(overrides[format!("tools.select.{tool}")], json!("Q"), "{overrides}");
    }
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let prefs = &h.state().session.prefs().shortcuts;
    for tool in ["lasso", "polygonLasso", "magneticLasso"] {
        assert_eq!(prefs.get(&format!("tools.select.{tool}")).map(String::as_str), Some("Q"), "{tool} keeps Q after OK");
    }
}

#[test]
fn a_letter_held_by_another_group_still_moves() {
    let (mut h, id) = dialog();
    assign(&mut h, id, "tools.select.lasso", Key::Q);
    assign(&mut h, id, "tools.select.brush", Key::Q);
    let message = field(&h, id, "message");
    assert!(message.as_str().unwrap().contains("already in use"), "{message}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let prefs = &h.state().session.prefs().shortcuts;
    assert_eq!(prefs.get("tools.select.lasso").map(String::as_str), Some(""));
    assert_eq!(prefs.get("tools.select.brush").map(String::as_str), Some("Q"));
}

#[test]
fn a_shared_user_letter_selects_and_shift_cycles_the_group() {
    let mut app = app();
    let ctx = egui::Context::default();
    let set = json!({"tools.select.lasso": "Q", "tools.select.polygonLasso": "Q", "tools.select.magneticLasso": "Q"});
    app.run("edit.keyboardShortcuts", json!({"set": set, "allowUnknown": true})).unwrap();
    app.ui.tool = Tool::Brush;
    press(&mut app, &ctx, Key::Q, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Lasso);
    for expected in [Tool::PolygonLasso, Tool::MagneticLasso, Tool::Lasso] {
        press(&mut app, &ctx, Key::Q, Modifiers::SHIFT);
        assert_eq!(app.ui.tool, expected, "Shift+Q");
    }
    // Plain Q keeps the group's tool with the Shift preference on, and cycles with it off.
    press(&mut app, &ctx, Key::Q, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Lasso);
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    press(&mut app, &ctx, Key::Q, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::PolygonLasso);
    // Leaving the group and coming back returns to the group's last tool.
    press(&mut app, &ctx, Key::B, Modifiers::NONE);
    press(&mut app, &ctx, Key::Q, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::PolygonLasso);
}

#[test]
fn a_single_user_letter_still_selects_its_one_tool() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("edit.keyboardShortcuts", json!({"set": {"tools.select.polygonLasso": "Q"}, "allowUnknown": true})).unwrap();
    app.ui.tool = Tool::Brush;
    press(&mut app, &ctx, Key::Q, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::PolygonLasso);
    press(&mut app, &ctx, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Lasso);
    press(&mut app, &ctx, Key::L, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::MagneticLasso, "Shift+L skips the remapped Polygonal Lasso");
}
