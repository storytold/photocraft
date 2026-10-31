//! #8762: the foreground/background pickers are bindable without taking a default key.

use egui::{Context, Event, Key, Modifiers};
use serde_json::json;

use crate::{PhotocraftApp, Services, dialogs, menus, prefs_ui, shortcut_dispatch, shortcuts};

const PICKERS: [(&str, &str, Key); 2] = [("color.picker.foreground", "foreground", Key::P), ("color.picker.background", "background", Key::O)];

/// Start without a document or platform callbacks to exercise global color shortcuts.
fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Services::default())
}

/// Assign the two picker keys through the real shortcut dialog confirmation path.
fn bind(app: &mut PhotocraftApp, ctx: &Context) {
    let id = menus::invoke(app, ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!({"color.picker.foreground": "P", "color.picker.background": "O"}));
    dialogs::confirm(app, id).unwrap();
}

/// Begin a frame with a complete key press, text event and release, or no input.
fn input(ctx: &Context, key: Option<Key>) {
    let events = key
        .map(|key| {
            vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Text(if key == Key::P { "p" } else { "o" }.into()),
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]
        })
        .unwrap_or_default();
    ctx.begin_pass(egui::RawInput { events, ..Default::default() });
}

/// Dispatch one key frame without drawing a widget that could consume the input.
fn press(app: &mut PhotocraftApp, ctx: &Context, key: Key) {
    input(ctx, Some(key));
    shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
/// Keep both unbound picker entries in the existing single Tools shortcut group.
fn picker_shortcuts_are_listed_once_in_the_single_tools_group_unbound() {
    let app = app();
    let items = prefs_ui::shortcut_items(&app);
    for (id, target, _) in PICKERS {
        let rows: Vec<_> = items.iter().filter(|item| item.0 == id).collect();
        assert_eq!(rows.len(), 1, "{id} must be assignable before it has a key");
        assert_eq!(rows[0].1, format!("Color Picker ({} Color)", if target == "foreground" { "Foreground" } else { "Background" }));
        assert_eq!(rows[0].2, ["Tools"]);
        assert_eq!(rows[0].3, None);
        assert!(shortcuts::default_shortcut(id).is_none());
        assert!(!shortcut_dispatch::bindings(&app).iter().any(|(bound, _)| bound == id));
    }
    let groups = items
        .iter()
        .enumerate()
        .filter(|(i, item)| item.2.first().map(String::as_str) == Some("Tools") && (*i == 0 || items[*i - 1].2.first() != item.2.first()))
        .count();
    assert_eq!(groups, 1, "the picker rows must join the existing Tools section");
}

#[test]
/// Open each target at every supported depth and preserve the existing OK/Cancel semantics.
fn picker_commands_open_the_correct_color_and_reuse_ok_cancel() {
    let ctx = Context::default();
    for depth in [8, 16, 32] {
        for (id, target, _) in PICKERS {
            let mut app = app();
            app.session.execute("file.new", json!({"width": 2, "height": 2, "depth": depth})).unwrap();
            let fg = [0.1234567, 0.3456789, 0.5678912, 0.75];
            let bg = [0.8765432, 0.6543219, 0.4321987, 0.25];
            app.session.tools.foreground = fg;
            app.session.tools.background = bg;
            let original = if target == "foreground" { fg } else { bg };
            let dialog = menus::invoke(&mut app, &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
            let fields = &app.ui.dialog_mut(dialog).unwrap().fields;
            assert_eq!(fields["__colorPicker"], target);
            assert_eq!(fields["color"], super::hex([original[0], original[1], original[2]]));
            app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#4080c0"));
            dialogs::cancel(&mut app, dialog).unwrap();
            assert_eq!(app.session.tools.foreground, fg, "Cancel keeps float precision and alpha");
            assert_eq!(app.session.tools.background, bg);

            let dialog = menus::invoke(&mut app, &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
            app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#4080c0"));
            dialogs::confirm(&mut app, dialog).unwrap();
            let picked = [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0];
            if target == "foreground" {
                assert_eq!(app.session.tools.foreground, picked);
                assert_eq!(app.session.tools.background, bg);
            } else {
                assert_eq!(app.session.tools.background, picked);
                assert_eq!(app.session.tools.foreground, fg);
            }
            assert!(app.ui.dialogs.is_empty());
        }
    }
}

#[test]
/// Reload assigned keys, open without a document, and obey keyboard and control modal guards.
fn assigned_picker_keys_dispatch_and_do_not_stack_under_a_modal() {
    let mut app = app();
    let ctx = Context::default();
    bind(&mut app, &ctx);
    let saved = app.session.prefs().to_json().to_string();
    app = PhotocraftApp::new(photocraft_engine::Session::new(), Services { load_prefs: Some(Box::new(move || Some(saved.clone()))), ..Default::default() });
    for (id, target, key) in PICKERS {
        assert!(shortcut_dispatch::bindings(&app).iter().any(|(bound, sc)| bound == id && sc.logical_key == key));
        press(&mut app, &ctx, key);
        assert_eq!(app.ui.dialogs.len(), 1);
        let dialog = app.ui.dialogs.last().unwrap();
        assert_eq!(dialog.fields["__colorPicker"], target);
        let dialog_id = dialog.id;
        for (_, _, other_key) in PICKERS {
            press(&mut app, &ctx, other_key);
            assert_eq!(app.ui.dialogs.len(), 1);
            assert_eq!(app.ui.dialogs.last().unwrap().id, dialog_id);
        }
        let (request, _) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"id": id}));
        let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("expected an immediate reply") };
        assert_eq!(reply["ok"], false, "the control entry must also obey the modal gate");
        dialogs::cancel(&mut app, dialog_id).unwrap();
    }
}

#[test]
/// Let a focused text widget receive the assigned letters without opening a picker.
fn assigned_picker_letters_leave_text_input_alone() {
    let mut app = app();
    let ctx = Context::default();
    bind(&mut app, &ctx);
    let mut text = String::new();
    for key in [None, Some(Key::P), Some(Key::O)] {
        input(&ctx, key);
        egui::Area::new(egui::Id::unique("picker-shortcut-typing")).show(&ctx, |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        });
        shortcuts::handle(&mut app, &ctx);
        assert!(app.ui.dialogs.is_empty(), "typing must not open a Color Picker");
        assert!(shortcut_dispatch::take_log(&ctx).is_empty());
        ctx.end_pass().textures_delta.clear();
    }
    assert_eq!(text, "po");
}

#[test]
/// Find both rows in the rendered dialog and optionally capture the native picker windows.
fn unbound_picker_rows_are_visible_in_keyboard_shortcuts() {
    use egui_kittest::kittest::Queryable;

    // The optional native capture uses the same renderer as the offscreen snapshot example.
    let directory = std::env::var_os("PHOTOCRAFT_PICKER_SHORTCUT_SNAPSHOTS").map(std::path::PathBuf::from);
    let _gpu = directory.as_ref().map(|_| photocraft_testkit::gpu_lock());
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64);
    let builder = if directory.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app()
    });
    let ctx = h.ctx.clone();
    let dialog = menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("filter".into(), json!("Color Picker"));
    h.run_steps(4);
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
        h.render().unwrap().save(directory.join("keyboard-shortcuts.png")).unwrap();
    }
    h.get_by_label_contains("Color Picker (Foreground Color)");
    h.get_by_label_contains("Color Picker (Background Color)");
    if let Some(directory) = &directory {
        dialogs::cancel(h.state_mut(), dialog).unwrap();
        for (id, target, _) in PICKERS {
            let dialog = menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
            h.run_steps(4);
            h.render().unwrap().save(directory.join(format!("{target}-picker.png"))).unwrap();
            dialogs::cancel(h.state_mut(), dialog).unwrap();
        }
    }
}
