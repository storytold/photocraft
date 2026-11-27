//! The global colour picker commands are unbound until the user assigns a shortcut.
use super::*;
use crate::shortcut_dispatch::tests::{app_harness, tap};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

const PICKERS: [(&str, &str, Key); 2] = [("color.picker.foreground", "Foreground", Key::P), ("color.picker.background", "Background", Key::O)];

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn bind(app: &mut PhotocraftApp) {
    app.run("edit.keyboardShortcuts", json!({"set": {"color.picker.foreground": "P", "color.picker.background": "O"}})).unwrap();
}

#[test]
fn picker_shortcuts_are_listed_once_in_the_single_tools_group_unbound() {
    let app = app();
    let rows = crate::shortcut_dispatch::command_rows(&app);
    for (id, role, _) in PICKERS {
        let matches: Vec<_> = rows.iter().filter(|r| r.command.id == id).collect();
        assert_eq!(matches.len(), 1);
        let row = matches[0];
        let label = format!("Color Picker ({role} Color)");
        assert_eq!(row.label, label);
        assert!(row.paths.contains(&vec!["Tools".into(), label]));
        assert!(row.defaults.is_empty());
    }
}

#[test]
fn picker_commands_open_the_correct_color_and_reuse_ok_cancel() {
    for depth in [8, 16, 32] {
        let mut app = app();
        app.run("file.new", json!({"width": 2,"height": 2,"depth":depth})).unwrap();
        for (id, role, _) in PICKERS {
            let ctx = app.ctx().clone();
            let colors = |app: &PhotocraftApp| [app.session.tools.foreground, app.session.tools.background];
            let original = colors(&app);
            for confirm in [false, true] {
                let reply = crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
                let (&dialog, target) = app.color_targets.iter().next().unwrap();
                assert_eq!(target.key(), role.to_lowercase());
                assert_eq!(reply["dialog"], dialog.value().to_string());
                let idx = usize::from(role == "Background");
                assert_eq!(testing::current(&app, dialog).unwrap().color.to_rgb_array(), original[idx][..3]);
                testing::set_color(&mut app, dialog, "#4080c0");
                if confirm {
                    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
                    assert_eq!(rgb_hex(Srgba::from_rgb_array(colors(&app)[idx][..3].try_into().unwrap())), "#4080c0");
                    assert_eq!(colors(&app)[1 - idx], original[1 - idx]);
                } else {
                    crate::dialogs::testing::cancel(&mut app, dialog).unwrap();
                    assert_eq!(colors(&app), original);
                }
            }
        }
    }
}

#[test]
fn assigned_picker_keys_dispatch_and_do_not_stack_under_a_modal() {
    for (_, role, key) in PICKERS {
        let mut app = app();
        bind(&mut app);
        let mut h = app_harness(app);
        tap(&mut h, key, Modifiers::NONE);
        let (&dialog, target) = h.state().color_targets.iter().next().expect("key opened the picker");
        assert_eq!(target.key(), role.to_lowercase());
        tap(&mut h, if key == Key::P { Key::O } else { Key::P }, Modifiers::NONE);
        assert_eq!(h.state().color_targets.len(), 1);
        assert!(h.state().color_targets.contains_key(&dialog));
    }
}

#[test]
fn assigned_picker_letters_leave_text_input_alone() {
    let mut app = app();
    bind(&mut app);
    let mut h = app_harness(app);
    crate::prefs_ui::open_shortcuts(h.state_mut(), 0);
    h.run_steps(3);
    h.get_by_label("Search:").click();
    h.run_steps(2);
    h.event(egui::Event::Text("p".into()));
    tap(&mut h, Key::P, Modifiers::NONE);
    assert!(h.state().color_targets.is_empty());
    assert!(h.query_all_by_label("Search:").any(|node| node.accesskit_node().value().as_deref() == Some("p")));
}

#[test]
fn unbound_picker_rows_are_visible_in_keyboard_shortcuts() {
    let mut h = app_harness(app());
    crate::prefs_ui::open_shortcuts(h.state_mut(), 0);
    h.run_steps(3);
    h.get_by_label("Search:").click();
    h.run_steps(2);
    h.event(egui::Event::Text("Color Picker".into()));
    h.run_steps(4);
    for (_, role, _) in PICKERS {
        let label = format!("Color Picker ({role} Color)");
        assert!(h.query_all_by_label_contains(&label).next().is_some(), "{label}");
    }
}
