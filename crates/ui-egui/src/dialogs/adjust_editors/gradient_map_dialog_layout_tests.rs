use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn app_with_image() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

fn harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.gradientMap").unwrap();
    h.run_steps(4);
    (h, id)
}

#[test]
fn dialog_has_two_group_boxes_ordered_options_and_right_button_column() {
    let (h, _id) = harness();
    let g1 = h.get_by_label("Gradient Used for Grayscale Mapping").rect();
    let g2 = h.get_by_label("Gradient Options").rect();
    assert!(g1.bottom() < g2.top(), "first group box sits above Gradient Options");

    assert!(h.query_by_label("Preset:").is_none(), "dialog must not have Preset: combo");
    assert!(h.query_by_label("Location:").is_none(), "dialog must not have inline Location: field");
    assert!(h.query_by_label("Add Stop").is_none(), "dialog must not have inline Add Stop button");

    let dither = h.get_by_label("Dither").rect();
    let reverse = h.get_by_label("Reverse").rect();
    let method = h.query_all_by_label("Method:").next().unwrap().rect();
    assert!(dither.bottom() < reverse.top(), "Dither before Reverse");
    assert!(reverse.bottom() < method.top(), "Reverse before Method:");

    let ok = h.get_by_label("OK").rect();
    let cancel = h.get_by_label("Cancel").rect();
    let preview = h.get_by_label("Preview").rect();
    assert!(ok.left() > method.right(), "OK is in the right column");
    assert!(ok.bottom() < cancel.top(), "OK above Cancel");
    assert!(cancel.bottom() < preview.top(), "Cancel above Preview");
}

#[test]
fn clicking_strip_opens_gradient_editor_and_ok_or_cancel_updates_dialog() {
    let (mut h, _id) = harness();
    let before_stops = h.state().ui.dialogs[0].fields["stops"].clone();
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 2, "clicking strip opens Gradient Editor on top");
    assert_eq!(h.state().ui.dialogs[1].fields.get("__label").and_then(Value::as_str), Some("Gradient Editor"));

    // Modify stops in the Gradient Editor and Cancel: parent dialog must remain unchanged.
    h.state_mut().ui.dialogs[1].fields.insert("stops".into(), json!([[0.0, "#ff0000"], [1.0, "#00ff00"]]));
    h.run_steps(2);
    h.query_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert_eq!(h.state().ui.dialogs[0].fields["stops"], before_stops);

    // Open again, modify stops, and click OK: parent dialog updates.
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 2);
    let edited = json!([[0.0, "#112233"], [1.0, "#445566"]]);
    h.state_mut().ui.dialogs[1].fields.insert("stops".into(), edited.clone());
    h.run_steps(2);
    h.query_all_by_label("OK").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert_eq!(h.state().ui.dialogs[0].fields["stops"], edited);
}

#[test]
fn strip_arrow_opens_preset_picker_applies_immediately_and_enter_or_esc_closes() {
    let (mut h, _id) = harness();
    let before_stops = h.state().ui.dialogs[0].fields["stops"].clone();
    h.get_by_label("Gradient Presets").click();
    h.run_steps(3);
    let preset_name = h
        .state()
        .session
        .presets
        .gradients
        .iter()
        .flat_map(|g| &g.items)
        .find(|p| {
            let r = p.resolve([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);
            r.len() != 2 || r[0].1[..3] != [0.0, 0.0, 0.0] || r[1].1[..3] != [1.0, 1.0, 1.0]
        })
        .unwrap()
        .name
        .clone();
    h.get_by_label(&preset_name).click();
    h.run_steps(3);
    let after_pick = h.state().ui.dialogs[0].fields["stops"].clone();
    assert_ne!(after_pick, before_stops, "picking a preset applies at once");
    assert!(h.query_by_label(&preset_name).is_some(), "picker stays open after a pick");

    // Enter closes the picker keeping the pick.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.query_by_label(&preset_name).is_none(), "Enter closes the picker");
    assert_eq!(h.state().ui.dialogs[0].fields["stops"], after_pick);
    assert_eq!(h.state().ui.dialogs.len(), 1, "Enter closed the picker, not the dialog");

    // Reopen and Esc closes it.
    h.get_by_label("Gradient Presets").click();
    h.run_steps(3);
    assert!(h.query_by_label(&preset_name).is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label(&preset_name).is_none(), "Esc closes the picker");
    assert_eq!(h.state().ui.dialogs.len(), 1, "Esc closed the picker, not the dialog");
}

#[test]
fn reverse_draws_strip_reversed() {
    let mut app = app_with_image();
    let id = crate::adjust_dialog::open(&mut app, "image.adjustments.gradientMap").unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("stops".into(), json!([[0.0, "#ff0000"], [1.0, "#0000ff"]]));
    let render = |app: &mut PhotocraftApp, rev: bool| {
        app.ui.dialog_mut(id).unwrap().fields.insert("reverse".into(), json!(rev));
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut out = egui::FullOutput::default();
        for _ in 0..3 {
            out.textures_delta.clear();
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
            out = ctx.run_ui(raw, |ui| {
                crate::dialogs::show(app, ui.ctx());
            });
        }
        out.textures_delta.clear();
        let shapes = out.shapes;
        shapes
            .into_iter()
            .filter_map(|cs| match cs.shape {
                egui::epaint::Shape::Mesh(m) if m.vertices.len() == 4 => Some((m.vertices[0].color, m.vertices[1].color)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let normal = render(&mut app, false);
    let reversed = render(&mut app, true);
    assert!(!normal.is_empty(), "strip mesh was painted");
    assert_ne!(normal.first().unwrap().0, normal.last().unwrap().1, "normal strip is not uniform");
    assert_eq!(normal.first().unwrap().0, reversed.last().unwrap().1, "leftmost color of normal matches rightmost of reversed");
    assert_eq!(normal.last().unwrap().1, reversed.first().unwrap().0, "rightmost color of normal matches leftmost of reversed");
}

#[test]
fn alt_mnemonics_underline_and_toggle_controls() {
    let (mut h, _id) = harness();
    assert_eq!(h.state().ui.dialogs[0].fields.get("dither").and_then(Value::as_bool), Some(false));
    assert_eq!(h.state().ui.dialogs[0].fields.get("reverse").and_then(Value::as_bool), Some(false));
    assert_eq!(h.state().ui.dialogs[0].fields.get("__preview").and_then(Value::as_bool), Some(true));

    // Holding Alt turns Cancel into Reset.
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_some(), "holding Alt turns Cancel into Reset");

    // Alt+D, Alt+R, Alt+P toggle Dither, Reverse, Preview.
    h.event(egui::Event::Key { key: egui::Key::D, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields.get("dither").and_then(Value::as_bool), Some(true));

    h.event(egui::Event::Key { key: egui::Key::R, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields.get("reverse").and_then(Value::as_bool), Some(true));

    h.event(egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields.get("__preview").and_then(Value::as_bool), Some(false));

    // Alt+M focuses Method.
    h.event(egui::Event::Key { key: egui::Key::M, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(2);
    assert!(h.query_all_by_label("Method:").any(|n| n.is_focused()), "Alt+M focuses Method dropdown");
}

#[test]
fn properties_panel_keeps_inline_stop_editor() {
    let mut app = app_with_image();
    app.run("layer.newAdjustmentLayer.gradientMap", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 600.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ALL[0]);
                return;
            }
            let Some(st) = app.session.active() else { return };
            let Some(id) = st.active_layer else { return };
            let Some(layer) = st.doc.layer(id).cloned() else { return };
            let photocraft_doc::LayerContent::Adjustment(adj) = &layer.content else { return };
            layer_editor(app, ui, id, adj);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    assert!(h.query_all_by_label("Preset:").next().is_some(), "Properties panel keeps Preset:");
    assert!(h.query_all_by_label("Location:").next().is_some(), "Properties panel keeps Location:");
    assert!(h.query_by_label("Add Stop").is_some(), "Properties panel keeps Add Stop");
}
