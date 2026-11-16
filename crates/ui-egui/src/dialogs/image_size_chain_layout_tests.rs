use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};

use super::*;

fn open_image_size() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    (h, id)
}

#[test]
fn chain_button_is_left_of_fields_and_hidden_when_resample_off() {
    let (mut h, id) = open_image_size();
    let w_rect = h.get_by_role_and_label(Role::SpinButton, "Width").rect();
    let chain = h.get_by_label("Constrain proportions");
    assert!(chain.rect().right() < w_rect.left(), "chain should sit left of Width field");

    chain.click();
    h.run_steps(2);
    let constrain_val = |h: &Harness<'_, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get("__constrain").cloned();
    assert_eq!(constrain_val(&h), Some(json!(false)));

    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("resample".into(), json!("none"));
    h.run_steps(2);
    assert!(h.query_by_label("Constrain proportions").is_none(), "Resample off should not show a toggle button");

    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("resample".into(), json!("bicubic"));
    h.run_steps(2);
    assert!(h.query_by_label("Constrain proportions").is_some(), "Resample on should restore the toggle button");
    assert_eq!(constrain_val(&h), Some(json!(false)));
}
