use craft::ui::dialog::DialogId;
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::new_layer_ui::testing::{fields, set};

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.run_steps(4);
    h
}

/// The open New Brush Preset dialog, if there is one (and never two).
fn dialog(app: &PhotocraftApp) -> Option<DialogId> {
    let open: Vec<DialogId> = app
        .framework
        .dialogs
        .stack()
        .ids()
        .filter(|id| app.framework.dialogs.get_by_id::<HostedParamDialog>(*id).is_some_and(|d| d.key().is_some_and(owns)))
        .collect();
    assert!(open.len() <= 1, "one dialog at a time: {open:?}");
    open.first().copied()
}

/// The dialog suggests the current brush's name and the next free number, never a name in use; OK
/// saves under the typed name with the boxes as set, and the next one suggests the next number.
#[test]
fn ok_saves_under_the_name_with_the_boxes_as_set() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().run("tools.setBrush", json!({"preset": "Chalk"})).unwrap();
    let id = open(h.state_mut(), None);
    h.run_steps(3);
    assert_eq!(fields(h.state(), id).text("name"), Some("Chalk 1"));
    let f = fields(h.state(), id);
    assert_eq!((f.bool("captureSize"), f.bool("toolSettings"), f.bool("color")), (Some(true), Some(false), Some(false)));
    // Include Color waits for Include Tool Settings.
    assert!(set(h.state_mut(), id, "color", json!(true)).is_err(), "Include Color is greyed out");
    set(h.state_mut(), id, "toolSettings", json!(true)).unwrap();
    set(h.state_mut(), id, "color", json!(true)).unwrap();
    set(h.state_mut(), id, "captureSize", json!(false)).unwrap();
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(3);
    assert!(dialog(h.state()).is_none());
    let p = h.state().session.tools.presets.iter().find(|p| p.name == "Chalk 1").expect("saved");
    assert_eq!(p.captures, photocraft_paint::PresetCaptures { size: false, tool_settings: true, color: true });
    assert!(h.state().session.tools.presets.iter().any(|p| p.name == "Chalk"), "the preset it came from is still there");
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Chalk 1"));

    // Again: the next number, and a name in use greys OK out.
    let id = open(h.state_mut(), None);
    h.run_steps(3);
    assert_eq!(fields(h.state(), id).text("name"), Some("Chalk 1 1"), "named after the current brush, now Chalk 1");
    set(h.state_mut(), id, "name", json!("chalk")).unwrap();
    h.run_steps(2);
    let before = h.state().session.tools.presets.len();
    assert!(crate::dialogs::confirm(h.state_mut(), &ctx, id).is_err(), "Chalk exists, so OK is greyed and automation can't overwrite it");
    assert_eq!(h.state().session.tools.presets.len(), before);
}

/// Include Color unticked-by-Tool-Settings with it ticked earlier: the preset doesn't take the colour.
#[test]
fn include_color_needs_include_tool_settings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let id = open(&mut app, None);
    set(&mut app, id, "name", json!("Mine")).unwrap();
    set(&mut app, id, "toolSettings", json!(true)).unwrap();
    set(&mut app, id, "color", json!(true)).unwrap();
    set(&mut app, id, "toolSettings", json!(false)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let p = app.session.tools.presets.iter().find(|p| p.name == "Mine").unwrap();
    assert_eq!(p.captures, photocraft_paint::PresetCaptures { size: true, tool_settings: false, color: false });
}
