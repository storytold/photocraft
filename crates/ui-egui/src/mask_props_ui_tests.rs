use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn harness(adjustment: bool, theme: crate::theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let command = if adjustment { "layer.newAdjustmentLayer.curves" } else { "layer.new.layer" };
    session.execute(command, json!({})).unwrap();
    if session.active().unwrap().doc.layer(session.active().unwrap().active_layer.unwrap()).unwrap().mask.is_none() {
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    }
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.ui.mask_target = true;
    app.ui.panels.properties = true;
    app.last_canvas_rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0));
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                if Tokens::get(ui.ctx()).pro {
                    properties_body(app, ui);
                } else {
                    properties_window(app, ui.ctx());
                }
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

fn mask(h: &Harness<'_, PhotocraftApp>) -> photocraft_doc::LayerMask {
    let s = h.state().session.active().unwrap();
    s.doc.layer(s.active_layer.unwrap()).unwrap().mask.clone().unwrap()
}

#[test]
fn pixel_mask_properties_follow_target_for_raster_and_adjustment_in_both_themes() {
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        for adjustment in [false, true] {
            let mut h = harness(adjustment, theme);
            assert!(h.query_all_by_label("Density").next().is_some(), "{theme:?}, adjustment={adjustment}");
            assert!(h.query_all_by_label("Feather").next().is_some());
            h.state_mut().ui.mask_target = false;
            h.run_steps(4);
            assert!(h.query_all_by_label("Density").next().is_none());
            if adjustment {
                assert!(h.query_all_by_label("Reset to defaults").next().is_some(), "Curves controls restored");
            }
            h.state_mut().ui.mask_target = true;
            h.state_mut().ui.vector_mask_target = true;
            h.run_steps(4);
            assert!(h.query_all_by_label("Density").next().is_none(), "a vector target must not edit the pixel mask");
        }
    }
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, field: usize, fractions: &[f32]) {
    let r = h.query_all_by_role(egui::accesskit::Role::Slider).nth(field).unwrap().rect();
    let pos = |f: f32| pos2(r.left() + 7.0 + (r.width() - 14.0) * f, r.center().y);
    let start = pos(fractions[0]);
    h.hover_at(start);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for &f in &fractions[1..] {
        h.hover_at(pos(f));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: pos(*fractions.last().unwrap()),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
}

#[test]
fn mask_slider_drags_preview_live_and_undo_as_separate_gestures() {
    let mut h = harness(true, crate::theme::ThemeKind::Pro);
    let before = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, 0, &[1.0, 0.8, 0.6, 0.5]);
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1);
    drag(&mut h, 0, &[0.5, 0.4, 0.25]);
    assert!((mask(&h).density - 0.25).abs() < 0.01);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(mask(&h).density, 1.0);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    h.run_steps(3);
    drag(&mut h, 1, &[0.0, 0.01, 0.02]);
    assert!((mask(&h).feather - 20.0).abs() < 0.1);
}

#[test]
fn uncommitted_mask_number_stays_with_its_layer() {
    let mut h = harness(false, crate::theme::ThemeKind::Studio);
    let first = h.state().session.active().unwrap().active_layer.unwrap();
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(1);
    for ch in "25*2".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.state_mut().session.execute("layer.new.layer", json!({})).unwrap();
    h.state_mut().session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.run_steps(4);
    assert_eq!(mask(&h).density, 1.0, "the newly selected mask must not receive the old field's expression");
    let doc = &h.state().session.active().unwrap().doc;
    assert_ne!(doc.layer(first).unwrap().mask.as_ref().unwrap().density, 0.5, "uncommitted expression is not applied on selection change");
}
