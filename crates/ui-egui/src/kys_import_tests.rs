//! Photoshop `.kys` sets import into Edit › Keyboard Shortcuts (#1163): rows match our
//! commands by label, one shortcut per command, with Photoshop's menu order breaking ties.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{KysCommand, parse, plan};
use crate::PhotocraftApp;
use crate::file_dialog::FileDialogAnswer;

/// A cut-down set in Photoshop's own layout: BOM, a dynamic (filter) row, static rows with
/// their ids, a row with two shortcuts, an entity, scripts, and the tool keys.
const SAMPLE: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/texcuts=texcuts\" modified=\"0\" multi-undo=\"2\">
\t<command kind=\"dynamic\" name=\"Script_6F17BFA7-EFC8-40EA-B850-7B95ED8EA713\">
\t\t<shortcut>Control+Opt+Cmd+4</shortcut>
\t</command>
\t<command kind=\"dynamic\" name=\"Gaussian Blur...\">
\t\t<shortcut>Control+Opt+Cmd+G</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Undo\" id=\"101\">
\t\t<shortcut>Cmd+Z</shortcut>
\t\t<shortcut>F1</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Zoom In\" id=\"1004\">
\t\t<shortcut>Cmd++</shortcut>
\t\t<shortcut>Cmd+=</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Levels...\" id=\"1801\">
\t\t<shortcut>Cmd+L</shortcut>
\t</command>
\t<command kind=\"static\" name=\"All\" id=\"1017\">
\t\t<shortcut>Cmd+A</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Layer Via Copy\" id=\"2970\">
\t\t<shortcut>Cmd+J</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Black &amp; White...\" id=\"1824\">
\t\t<shortcut>Opt+Shift+Cmd+B</shortcut>
\t</command>
\t<command kind=\"static\" name=\"General...\" id=\"2311\">
\t\t<shortcut>Cmd+K</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Last Filter\" id=\"1019\">
\t\t<shortcut>Cmd+F</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Hide Photoshop\" id=\"7001\">
\t\t<shortcut>Cmd+H</shortcut>
\t</command>
\t<tool name=\"Move Tool\" type=\"1\" key=\"1819113074\">V</tool>
\t<tool name=\"Default Foreground/Background Colors\" type=\"2\"></tool>
</photoshop-keyboard-shortcuts>
";

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn parses_rows_shortcuts_entities_and_tool_keys() {
    let set = parse(SAMPLE).unwrap();
    assert_eq!(set.commands.len(), 11);
    assert_eq!(set.commands[2], KysCommand { name: "Undo".into(), shortcuts: vec!["Cmd+Z".into(), "F1".into()] });
    assert_eq!(set.commands[7].name, "Black & White...");
    assert_eq!(set.tool_keys, 1, "the empty Default Colors row carries no key");
}

#[test]
fn rejects_other_xml_and_text() {
    assert!(parse("<svg/>").unwrap_err().contains("not a Photoshop keyboard shortcuts file"));
    assert!(parse("Cmd+K\tPreferences").is_err());
    assert_eq!(parse("<photoshop-keyboard-shortcuts/>").unwrap(), super::KysSet::default());
}

#[test]
fn rows_match_our_commands_by_label_with_photoshop_tie_breaks() {
    let app = app();
    let p = plan(&app, &parse(SAMPLE).unwrap());
    // Photoshop's "..." is our "…"; the dynamic (filter) row is a filter here.
    assert_eq!(p.set.get("filter.blur.gaussianBlur").map(String::as_str), Some("Cmd+Ctrl+Alt+G"));
    // Levels… is Image › Adjustments and Layer › New Adjustment Layer: the one whose default is
    // the imported key wins. All is Select, Edit › Purge, View › Show…: Select › All has the key.
    assert_eq!(p.set.get("image.adjustments.levels").map(String::as_str), Some("Cmd+L"));
    assert!(!p.set.contains_key("layer.newAdjustmentLayer.levels"));
    assert_eq!(p.set.get("select.all").map(String::as_str), Some("Cmd+A"));
    assert!(!p.set.contains_key("edit.purge.all"));
    // Undo is Edit › Undo, not Edit › Purge › Undo; only its first shortcut counts.
    assert_eq!(p.set.get("edit.undo").map(String::as_str), Some("Cmd+Z"));
    // Zoom In lists Cmd++ first, but Cmd+= is our default and is among its keys: it stays.
    assert_eq!(p.set.get("view.zoomIn").map(String::as_str), Some("Cmd+="));
    assert_eq!(p.alternates, 2);
    // Case does not matter; an entity decodes; the menu's Settings… is the engine's General….
    assert_eq!(p.set.get("layer.new.layerViaCopy").map(String::as_str), Some("Cmd+J"));
    assert_eq!(p.set.get("image.adjustments.blackWhite").map(String::as_str), Some("Cmd+Alt+Shift+B"));
    assert_eq!(p.set.get("edit.preferences.general").map(String::as_str), Some("Cmd+K"));
    assert_eq!(p.set.get("filter.lastFilter").map(String::as_str), Some("Cmd+F"));
    // Scripts and the macOS application menu have no command here.
    assert_eq!(p.unknown, vec!["Script_6F17BFA7-EFC8-40EA-B850-7B95ED8EA713".to_string(), "Hide Photoshop".to_string()]);
    assert!(p.unreadable.is_empty());
    assert_eq!(p.set.len(), 9);
}

#[test]
fn an_unreadable_shortcut_is_reported_not_imported() {
    let app = app();
    let set = super::KysSet { commands: vec![KysCommand { name: "Undo".into(), shortcuts: vec!["Cmd+Shift+Ctrl".into()] }], tool_keys: 0 };
    let p = plan(&app, &set);
    assert!(p.set.is_empty());
    assert_eq!(p.unreadable, vec!["Undo".to_string()]);
}

/// The dialog's Import Photoshop Shortcuts… button: the chosen file fills the overrides (a key
/// equal to the default clears its override), reports what it did, and OK applies the set like
/// hand-made changes, taking a moved key from its old owner.
#[test]
fn import_button_fills_the_dialog_and_ok_applies_the_set() {
    let answers = vec![Some(FileDialogAnswer::Contents("/sets/texcuts.kys".into(), SAMPLE.as_bytes().to_vec()))];
    let services = crate::Services { file_dialog: Some(crate::file_dialog::fake(answers).0), ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), services)
    });
    let ctx = h.ctx.clone();
    // A hand-made override the file agrees with the default on: the import clears it.
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.undo": "Cmd+Shift+Z"}})).unwrap();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    // A short list keeps the button row on screen.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("filter".into(), json!("gaussian"));
    h.run_steps(2);
    h.get_by_label("Import Photoshop Shortcuts…").click();
    h.run_steps(3);
    let field = |h: &Harness<'_, PhotocraftApp>, name: &str| -> Value {
        h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
    };
    assert_eq!(h.state().ui.status, "", "the import failed");
    let overrides = field(&h, "overrides");
    assert_eq!(overrides["filter.blur.gaussianBlur"], json!("Cmd+Ctrl+Alt+G"));
    assert_eq!(overrides["edit.preferences.general"], json!("Cmd+K"));
    assert_eq!(overrides["filter.lastFilter"], json!("Cmd+F"));
    assert!(overrides.get("edit.undo").is_none(), "Cmd+Z is the default again: {overrides}");
    assert!(overrides.get("select.all").is_none(), "already the default: {overrides}");
    assert!(overrides.get("view.zoomIn").is_none(), "Cmd+= is among the keys, so the default stays: {overrides}");
    let message = field(&h, "message");
    assert_eq!(message, json!("Imported 9 shortcuts from texcuts.kys. Click OK to keep them. 2 commands are not in PhotoCraft."));
    assert_eq!(field(&h, "capture"), json!(false));
    // Nothing is applied until OK.
    assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.undo", Some("Cmd+Z")).as_deref(), Some("Cmd+Shift+Z"));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let app = h.state();
    assert_eq!(crate::shortcuts::effective_shortcut(app, "edit.undo", Some("Cmd+Z")).as_deref(), Some("Cmd+Z"));
    assert_eq!(crate::shortcuts::effective_shortcut(app, "filter.blur.gaussianBlur", None).as_deref(), Some("Cmd+Ctrl+Alt+G"));
    assert_eq!(crate::shortcuts::effective_shortcut(app, "edit.preferences.general", None).as_deref(), Some("Cmd+K"));
    // Cmd+K went to Preferences and Cmd+F to Last Filter, so Search lost its key.
    assert_eq!(crate::shortcuts::effective_shortcut(app, "edit.search", Some("Cmd+K")), None);
    assert_eq!(crate::shortcuts::effective_shortcut(app, "filter.lastFilter", Some("Cmd+Alt+F")).as_deref(), Some("Cmd+F"));
}

/// Opening or dropping a `.kys` opens the dialog with the set loaded, so it can be reviewed.
#[test]
fn an_opened_kys_file_opens_the_dialog_filled() {
    let mut app = app();
    assert!(app.ui.dialogs.is_empty());
    let r = crate::preset_files_ui::open(&mut app, "texcuts.kys", SAMPLE.as_bytes()).unwrap();
    r.unwrap();
    let d = app.ui.dialogs.iter().find(|d| d.fields.get("__prefsui").and_then(Value::as_str) == Some("shortcuts")).unwrap();
    assert_eq!(d.fields["tab"], json!(0));
    assert_eq!(d.fields["overrides"]["filter.blur.gaussianBlur"], json!("Cmd+Ctrl+Alt+G"));
    // A second file lands in the same dialog, on top of the first.
    let one = "<photoshop-keyboard-shortcuts><command kind=\"static\" name=\"Copy\"><shortcut>F3</shortcut></command></photoshop-keyboard-shortcuts>";
    crate::preset_files_ui::open(&mut app, "more.kys", one.as_bytes()).unwrap().unwrap();
    assert_eq!(app.ui.dialogs.len(), 1);
    let d = &app.ui.dialogs[0];
    assert_eq!(d.fields["overrides"]["edit.copy"], json!("F3"));
    assert_eq!(d.fields["overrides"]["filter.blur.gaussianBlur"], json!("Cmd+Ctrl+Alt+G"));
    assert_eq!(d.fields["message"], json!("Imported 1 shortcuts from more.kys. Click OK to keep them."));
    let bad = crate::preset_files_ui::open(&mut app, "notes.kys", b"hello").unwrap().unwrap_err();
    assert!(bad.contains("not a Photoshop keyboard shortcuts file"), "{bad}");
}
