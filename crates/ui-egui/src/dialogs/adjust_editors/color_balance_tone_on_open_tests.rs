use super::*;

fn selected(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn new_color_balance_dialog_starts_at_midtones_after_any_close() {
    const COMMAND: &str = "image.adjustments.colorBalance";
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    // The Properties editor uses a different memory root and keeps its choice.
    let panel_tone = crate::adjust_editors::layer_mem(h.state().session.active().unwrap().active_layer.unwrap()).with("cb-tone");
    h.ctx.data_mut(|data| data.insert_temp(panel_tone, 2usize));

    for (label, close) in [("Shadows", Some("OK")), ("Highlights", Some("Cancel")), ("Shadows", None)] {
        let id = open(h.state_mut(), COMMAND).unwrap();
        h.run_steps(3);
        assert!(selected(&h, "Midtones"));
        for key in ["shadows", "midtones", "highlights"] {
            assert_eq!(field(&h, id, key), json!([0.0, 0.0, 0.0]));
        }
        click_radio(&mut h, label);
        assert!(selected(&h, label));

        if let Some(button) = close {
            h.get_by_label(button).click();
            h.run_steps(3);
        } else {
            input(
                &mut h,
                egui::Modifiers::NONE,
                &[egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            );
        }
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.ctx.data(|data| data.get_temp::<usize>(panel_tone)), Some(2));
    }
    open(h.state_mut(), COMMAND).unwrap();
    h.run_steps(3);
    assert!(selected(&h, "Midtones"));
}
