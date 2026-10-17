use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.run_steps(4);
    h
}

/// The open New Brush Preset dialog, if there is one (and never two).
fn dialog(h: &Harness<'_, PhotocraftApp>) -> Option<u64> {
    let open: Vec<u64> = h.state().ui.dialogs.iter().filter(|d| owns(&d.fields)).map(|d| d.id).collect();
    assert!(open.len() <= 1, "one dialog at a time: {open:?}");
    open.first().copied()
}

fn field<'a>(h: &'a Harness<'_, PhotocraftApp>, id: u64, key: &str) -> &'a Value {
    &h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[key]
}

fn settings_menu(h: &mut Harness<'_, PhotocraftApp>, tab: usize) {
    h.state_mut().ui.panels.brush_settings = true;
    h.state_mut().ui.brush_tab = tab;
    h.run_steps(3);
    // A larger tab can clamp the floating panel to the screen after a pointer frame.
    // Wait for its actual target to settle; clicking a stale rectangle can hit the tab body.
    let mut settled = false;
    for _ in 0..8 {
        let menu = h.get_by_role_and_label(egui::accesskit::Role::Window, "Brush Settings").get_by_label("Panel menu").rect();
        h.hover_at(menu.center());
        h.run_steps(2);
        let next = h.get_by_role_and_label(egui::accesskit::Role::Window, "Brush Settings").get_by_label("Panel menu").rect();
        if menu == next {
            settled = true;
            break;
        }
    }
    assert!(settled, "the menu target must settle before clicking");
    h.get_by_role_and_label(egui::accesskit::Role::Window, "Brush Settings").get_by_label("Panel menu").click();
    h.run_steps(3);
    h.get_by_label("New Brush Preset…").click();
    h.run_steps(3);
}

/// Every way of making a preset opens the dialog, and Cancel saves nothing.
#[test]
fn every_new_preset_button_opens_the_dialog_and_cancel_saves_nothing() {
    let mut h = harness();
    let before = h.state().session.tools.presets.clone();
    let brush = h.state().session.tools.brush.clone();
    let mut opened = 0;
    let mut cancel = |h: &mut Harness<'_, PhotocraftApp>| {
        dialog(h).expect("the dialog opened");
        h.run_steps(2);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(dialog(h).is_none(), "Cancel closes it");
        assert_eq!(h.state().session.tools.presets, before, "and saves nothing");
        assert_eq!(h.state().session.tools.brush, brush, "nor touches the brush");
        opened += 1;
    };
    // The picker's [+] and its gear menu.
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.run_steps(3);
    h.get_by_label("New Brush Preset").click();
    h.run_steps(3);
    cancel(&mut h);
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.run_steps(3);
    h.get_by_label("Brush Preset Options").click();
    h.run_steps(2);
    h.get_by_label("New Brush Preset…").click();
    h.run_steps(3);
    cancel(&mut h);
    // Brush Settings' [+], then the Brushes tab's.
    h.state_mut().ui.brush_picker = None;
    h.state_mut().ui.panels.brush_settings = true;
    h.state_mut().ui.brush_tab = 0;
    h.run_steps(3);
    h.get_by_label("Create new brush from these settings").click();
    h.run_steps(3);
    cancel(&mut h);
    h.state_mut().ui.brush_tab = 1;
    h.run_steps(3);
    h.get_by_label("Create new brush from the current settings").click();
    h.run_steps(3);
    cancel(&mut h);
    // The Brush Settings panel menu must use the same deferred creation flow.
    settings_menu(&mut h, 0);
    cancel(&mut h);
    assert_eq!(opened, 5);
}

#[test]
fn settings_menu_cancel_saves_nothing_in_both_tabs_and_every_theme() {
    for kind in crate::theme::ThemeKind::ALL {
        for tab in [0, 1] {
            let mut h = harness();
            h.state_mut().run("prefs.set", json!({"path":"interface.theme", "value":kind.id()})).unwrap();
            h.state_mut().run("tools.setBrush", json!({"preset": "Chalk"})).unwrap();
            h.run_steps(3);
            assert_eq!(h.state().ui.theme, kind);
            assert_eq!(Tokens::get(&h.ctx), Tokens::for_kind(kind));
            let presets = h.state().session.tools.presets.clone();
            let brush = h.state().session.tools.brush.clone();
            let current = h.state().session.tools.current_preset.clone();
            let journal = h.state().session.journal.len();
            settings_menu(&mut h, tab);
            let id = dialog(&h).expect("the menu opens the shared dialog");
            assert_eq!(field(&h, id, "name"), "Chalk 1", "{kind:?}, tab {tab}");
            assert_eq!(field(&h, id, "captureSize"), true);
            assert_eq!(field(&h, id, "toolSettings"), false);
            assert_eq!(field(&h, id, "color"), false);
            assert!(h.get_by_label("Include Color").accesskit_node().is_disabled());
            assert!(!egui::Popup::is_any_open(&h.ctx), "the menu closed before the modal");
            assert_eq!(h.state().session.tools.presets, presets, "opening the dialog does not save");
            assert!(h.state().ui.brushes_panel.renaming.is_none() && h.state().ui.brush_picker_list.renaming.is_none());
            if tab == 0 {
                h.get_by_label("Cancel").click();
            } else {
                h.key_press(egui::Key::Escape);
            }
            h.run_steps(3);
            assert!(dialog(&h).is_none());
            assert_eq!(h.state().session.tools.presets, presets);
            assert_eq!(h.state().session.tools.brush, brush);
            assert_eq!(h.state().session.tools.current_preset, current);
            assert_eq!(h.state().session.journal.len(), journal);
        }
    }
}

#[test]
fn settings_menu_confirm_uses_the_name_and_capture_choices_once() {
    let mut h = harness();
    h.state_mut().run("tools.setBrush", json!({"preset": "Chalk", "size": 73, "opacity": 0.41, "flow": 0.63, "mode": "Multiply"})).unwrap();
    let source = h.state().session.tools.presets.iter().find(|p| p.name == "Chalk").unwrap().clone();
    let brush = h.state().session.tools.brush.clone();
    let count = h.state().session.tools.presets.len();
    let journal = h.state().session.journal.len();
    settings_menu(&mut h, 0);
    dialog(&h).expect("the menu opens the shared dialog");
    h.event(egui::Event::Text("Menu capture".into()));
    h.run_steps(2);
    h.get_by_label("Capture Brush Size in Preset").click();
    h.get_by_label("Include Tool Settings").click();
    h.run_steps(2);
    h.get_by_label("Include Color").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.tools.presets.len(), count + 1);
    let saved = h.state().session.tools.presets.iter().find(|p| p.name == "Menu capture").expect("the typed name was used");
    assert_eq!(saved.captures, photocraft_paint::PresetCaptures { size: false, tool_settings: true, color: true });
    assert_eq!(saved.brush, brush);
    assert_eq!(h.state().session.tools.brush, brush);
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Menu capture"));
    assert_eq!(h.state().session.tools.presets.iter().find(|p| p.name == "Chalk"), Some(&source));
    assert_eq!(h.state().session.journal.len(), journal + 1);
    assert_eq!(
        h.state().session.journal[journal],
        ("brush.presets.save".into(), json!({"name":"Menu capture", "captureSize":false, "toolSettings":true, "color":true}))
    );

    // The choices affect picking too: size stays with the tool; captured options return.
    h.state_mut().run("tools.setBrush", json!({"size":21, "opacity":0.9, "flow":0.8, "mode":"Normal"})).unwrap();
    h.state_mut().run("tools.setBrush", json!({"preset":"Menu capture"})).unwrap();
    let picked = &h.state().session.tools.brush;
    assert_eq!(picked.size, 21.0);
    assert_eq!((picked.opacity, picked.flow, picked.mode), (brush.opacity, brush.flow, brush.mode));
}

#[test]
fn settings_menu_name_focus_enter_and_reopening_use_fresh_dialog_state() {
    let mut h = harness();
    h.state_mut().run("tools.setBrush", json!({"preset":"Chalk"})).unwrap();
    settings_menu(&mut h, 0);
    dialog(&h).expect("the menu opens the shared dialog");
    h.event(egui::Event::Text("Menu keyboard".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Menu keyboard"));
    settings_menu(&mut h, 0);
    let id = dialog(&h).expect("the menu can open again");
    assert_eq!(field(&h, id, "name"), "Menu keyboard 1");
    h.event(egui::Event::Text("chalk".into()));
    h.run_steps(2);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "a source preset cannot be overwritten");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(dialog(&h).is_some(), "Enter cannot accept a duplicate name");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    let id = {
        settings_menu(&mut h, 1);
        dialog(&h).unwrap()
    };
    assert_eq!(field(&h, id, "name"), "Menu keyboard 1");
    assert_eq!(field(&h, id, "captureSize"), true);
    assert_eq!(field(&h, id, "toolSettings"), false);
}

/// The dialog suggests the current brush's name and the next free number, never a name in use; OK
/// saves under the typed name with the boxes as set, and the next one suggests the next number.
#[test]
fn ok_saves_under_the_name_with_the_boxes_as_set() {
    let mut h = harness();
    h.state_mut().run("tools.setBrush", json!({"preset": "Chalk"})).unwrap();
    let id = open(h.state_mut(), None);
    h.run_steps(3);
    assert_eq!(field(&h, id, "name"), "Chalk 1");
    assert_eq!((field(&h, id, "captureSize"), field(&h, id, "toolSettings"), field(&h, id, "color")), (&json!(true), &json!(false), &json!(false)));
    // Include Color waits for Include Tool Settings.
    assert!(h.get_by_label("Include Color").accesskit_node().is_disabled(), "Include Color is greyed out");
    h.get_by_label("Include Tool Settings").click();
    h.run_steps(2);
    assert!(!h.get_by_label("Include Color").accesskit_node().is_disabled());
    h.get_by_label("Include Color").click();
    h.get_by_label("Capture Brush Size in Preset").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    let p = h.state().session.tools.presets.iter().find(|p| p.name == "Chalk 1").expect("saved");
    assert_eq!(p.captures, photocraft_paint::PresetCaptures { size: false, tool_settings: true, color: true });
    assert!(h.state().session.tools.presets.iter().any(|p| p.name == "Chalk"), "the preset it came from is still there");
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Chalk 1"));

    // Again: the next number, and a name in use greys OK out.
    let id = open(h.state_mut(), None);
    h.run_steps(3);
    assert_eq!(field(&h, id, "name"), "Chalk 1 1", "named after the current brush, now Chalk 1");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("chalk"));
    h.run_steps(2);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "Chalk exists");
    assert!(h.query_by_label("A brush preset with this name already exists").is_some());
    assert!(crate::dialogs::confirm(h.state_mut(), id).is_err(), "automation can't overwrite it either");
}

/// Include Color unchecked with it ticked earlier: the preset doesn't take the colour.
#[test]
fn include_color_needs_include_tool_settings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let id = open(&mut app, None);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("name".into(), json!("Mine"));
    f.insert("color".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let p = app.session.tools.presets.iter().find(|p| p.name == "Mine").unwrap();
    assert_eq!(p.captures, photocraft_paint::PresetCaptures { size: true, tool_settings: false, color: false });
}

#[test]
fn failed_new_brush_confirmation_keeps_the_form_and_accepts_a_corrected_retry() {
    for theme in crate::theme::ThemeKind::ALL {
        for enter in [false, true] {
            let mut h = harness();
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.state_mut().run("tools.setBrush", json!({"size":17.25,"noise":true})).unwrap();
            let id = open(h.state_mut(), None);
            {
                let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
                f.insert("name".into(), json!("hard round"));
                f.insert("captureSize".into(), json!(false));
                f.insert("toolSettings".into(), json!(true));
                f.insert("color".into(), json!(true));
            }
            h.run_steps(3);
            let fields = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
            let presets = h.state().session.tools.presets.clone();
            let brush = h.state().session.tools.brush.clone();
            let current = h.state().session.tools.current_preset.clone();
            let journal = h.state().session.journal.clone();
            assert!(crate::dialogs::confirm(h.state_mut(), id).is_err());
            assert_eq!(dialog(&h), Some(id), "{theme:?}, Enter={enter}: rejected confirmation must retain the original dialog");
            assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields, fields);
            {
                let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
                f.insert("name".into(), json!("Retry preset"));
                f.insert("brush".into(), json!({"size":photocraft_paint::MAX_BRUSH_SIZE + 1.0}));
            }
            h.run_steps(3);
            if enter {
                h.key_press(egui::Key::Enter);
            } else {
                h.get_by_label("OK").click();
            }
            h.run_steps(3);
            assert!(h.state().ui.status.contains("at most 5000"));
            assert_eq!(dialog(&h), Some(id), "invalid brush settings must also keep the form");
            assert_eq!(field(&h, id, "name"), "Retry preset");
            assert_eq!(field(&h, id, "brush")["size"], json!(photocraft_paint::MAX_BRUSH_SIZE + 1.0));
            assert_eq!(h.state().session.tools.presets, presets);
            assert_eq!(h.state().session.tools.brush, brush);
            assert_eq!(h.state().session.tools.current_preset, current);
            assert_eq!(h.state().session.journal, journal);
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("brush".into(), json!({"size":17.25}));
            h.run_steps(2);
            if enter {
                h.key_press(egui::Key::Enter);
            } else {
                h.get_by_label("OK").click();
            }
            h.run_steps(3);
            assert!(dialog(&h).is_none());
            assert_eq!(h.state().session.tools.presets.len(), presets.len() + 1);
            assert_eq!(&h.state().session.tools.presets[..presets.len()], presets);
            let saved = h.state().session.tools.presets.last().unwrap();
            assert_eq!(saved.name, "Retry preset");
            assert_eq!(saved.brush.size, 17.25);
            assert_eq!(saved.captures, photocraft_paint::PresetCaptures { size: false, tool_settings: true, color: true });
            assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Retry preset"));
            assert_eq!(
                &h.state().session.journal[journal.len()..],
                &[(
                    "brush.presets.save".to_string(),
                    json!({"name":"Retry preset","captureSize":false,"toolSettings":true,"color":true,"brush":{"size":17.25}})
                )]
            );
        }
    }
}

/// The dialog opens on the name, all of it selected: typing replaces it, and Enter is OK.
#[test]
fn typing_replaces_the_suggested_name_and_enter_is_ok() {
    let mut h = harness();
    let id = open(h.state_mut(), None);
    h.run_steps(4);
    h.event(egui::Event::Text("Inky".into()));
    h.run_steps(2);
    assert_eq!(field(&h, id, "name"), "Inky");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(dialog(&h).is_none(), "Enter is OK");
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Inky"));
}

/// A tool without a picked preset gets a usable fresh name, even with differently cased names.
#[test]
fn fresh_preset_names_after_a_tool_switch_save_without_touching_existing_presets() {
    for kind in crate::theme::ThemeKind::ALL {
        for menu in [false, true] {
            let mut h = harness();
            h.state_mut().run("prefs.set", json!({"path":"interface.theme", "value":kind.id()})).unwrap();
            for (name, size) in [("brush 1", 31), ("BRUSH 2", 52)] {
                h.state_mut().run("tools.setBrush", json!({"size":size})).unwrap();
                h.state_mut().run("brush.presets.save", json!({"name":name})).unwrap();
            }
            // A real tool switch forgets the outgoing preset's identity.
            h.state_mut().ui.tool = crate::state::Tool::Eraser;
            h.run_steps(3);
            assert_eq!(h.state().session.tools.current_preset, None);
            assert_eq!(Tokens::get(&h.ctx), Tokens::for_kind(kind));
            let presets = h.state().session.tools.presets.clone();
            let journal = h.state().session.journal.len();
            if menu {
                settings_menu(&mut h, 0);
            } else {
                h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
                h.run_steps(3);
                h.get_by_label("New Brush Preset").click();
                h.run_steps(3);
            }
            let id = dialog(&h).expect("the creation dialog opened");
            assert_eq!(field(&h, id, "name"), "Brush 3", "{kind:?}, menu={menu}");
            assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
            assert_eq!(h.state().session.tools.presets, presets, "opening does not change a preset");
            if menu {
                h.get_by_label("OK").click();
            } else {
                h.key_press(egui::Key::Enter);
            }
            h.run_steps(3);
            assert!(dialog(&h).is_none());
            assert_eq!(h.state().session.tools.presets.len(), presets.len() + 1);
            assert_eq!(h.state().session.tools.presets.get(..presets.len()), Some(presets.as_slice()));
            assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Brush 3"));
            assert_eq!(h.state().session.journal.len(), journal + 1);
            assert_eq!(h.state().session.journal.last().unwrap().0, "brush.presets.save");
        }
    }
}
