//! #15055: New Guide and Edit Selected Guides open on Position with its text selected, as
//! Photoshop does, so typing replaces the readout and Enter makes the guide.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;

/// A 4 × 2 inch document at 100 ppi with pixel rulers, drawn by the dialog layer only.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "pixels"})).unwrap();
    app.sync_views();
    let h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.horizontal.clone(), g.vertical.clone())
}

#[test]
fn new_guide_opens_on_position_selected() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let d = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(3);
    assert!(h.get_by_label("Position").is_focused(), "Position has the keyboard on open");
    h.event(egui::Event::Text("120".into()));
    h.run_steps(2);
    assert_eq!(h.state_mut().ui.dialog_mut(d).unwrap().fields["position"], "120", "typing replaced the selected \"0 px\"");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Enter is OK");
    assert_eq!(guides(h.state()), (vec![120.0], vec![]));
}

#[test]
fn edit_selected_guides_opens_on_position_selected() {
    let mut h = harness();
    h.state_mut().run(COMMAND, json!({"orientation": "vertical", "position": 12.5})).unwrap();
    crate::guide_select::select(h.state_mut(), true, 0);
    let d = open_edit(h.state_mut()).unwrap();
    h.run_steps(3);
    assert!(h.get_by_label("Position").is_focused(), "Position has the keyboard on open");
    h.event(egui::Event::Text("1 in".into()));
    h.run_steps(2);
    assert_eq!(h.state_mut().ui.dialog_mut(d).unwrap().fields["position"], "1 in");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(guides(h.state()), (vec![], vec![100.0]));
}
