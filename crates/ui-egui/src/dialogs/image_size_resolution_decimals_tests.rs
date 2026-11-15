use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};

use super::*;

fn open_image_size(w: u32, h_px: u32) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": w, "height": h_px, "resolution": 72})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    (h, id)
}

#[test]
fn resolution_shows_up_to_three_decimals_and_keeps_whole_values() {
    let (mut h, id) = open_image_size(400, 300);
    assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Resolution").value().as_deref(), Some("72"));

    for (res, want) in [(28.346, "28.346"), (36.002835, "36.003")] {
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("resolution".into(), json!(res));
        h.run_steps(2);
        assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Resolution").value().as_deref(), Some(want));
    }
}

#[test]
fn resample_off_width_28_22_cm_shows_height_21_17_and_resolution_36_003() {
    let (mut h, id) = open_image_size(400, 300);
    {
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("resample".into(), json!("none"));
        d.fields.insert("__unit".into(), json!("cm"));
    }
    h.run_steps(3);
    assert!(h.get_by_role_and_label(Role::SpinButton, "Width").is_focused());
    h.event(egui::Event::Text("28.22".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Height").value().as_deref(), Some("21.17"));
    assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Resolution").value().as_deref(), Some("36.003"));
}
