//! Shape Properties edits reach the document as one history step per gesture.

use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::shape_properties;
use crate::PhotocraftApp;

fn properties_harness(params: serde_json::Value, depth: u32) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
    let mut params = params;
    params["fill"] = json!("#d92070");
    params["stroke"] = json!({"width": 5, "color": "#21bb66"});
    app.run("shape.create", params).unwrap();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(640.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            if let Some(id) = app.session.active().and_then(|st| st.active_layer) {
                shape_properties(app, ui, id);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro_medium());
    h.run_steps(4);
    h
}

fn selected_shape(h: &Harness<'_, Box<PhotocraftApp>>) -> photocraft_doc::ShapeLayer {
    let st = h.state().session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Shape(shape) => shape.clone(),
        _ => panic!("expected the selected vector shape"),
    }
}

fn steps(h: &Harness<'_, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// Types `text` into the field named `name` and commits it with Enter.
fn type_into(h: &mut Harness<'_, Box<PhotocraftApp>>, name: &str, text: &str) {
    h.get_by_label(name).click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label(name).type_text(text);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

/// Drags the caption `name` by `dx` points: a scrub gesture from press to release.
fn scrub(h: &mut Harness<'_, Box<PhotocraftApp>>, name: &str, dx: f32) {
    let start: Pos2 = h.get_by_role_and_label(egui::accesskit::Role::Label, name).rect().center();
    h.event(egui::Event::PointerMoved(start));
    h.event(egui::Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for step in 1..=3 {
        h.event(egui::Event::PointerMoved(start + vec2(dx * step as f32 / 3.0, 0.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: start + vec2(dx, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn selected_shape_presets_edit_live_preserve_paint_and_undo_at_all_depths() {
    for depth in [8, 16, 32] {
        for params in [
            json!({"kind": "rect", "rect": [12, 12, 45, 30]}),
            json!({"kind": "polygon", "sides": 3, "rect": [12, 12, 45, 30]}),
            json!({"kind": "ellipse", "rect": [12, 12, 45, 30]}),
        ] {
            let mut h = properties_harness(params, depth);
            let original = selected_shape(&h);
            let before = steps(&h);
            type_into(&mut h, "Stroke width", "9");
            let edited = selected_shape(&h);
            assert_eq!(edited.stroke.as_ref().unwrap().width, 9.0, "depth {depth}: the width edit lands");
            assert_eq!(edited.stroke.as_ref().unwrap().paint, original.stroke.as_ref().unwrap().paint, "depth {depth}: the stroke paint stays");
            assert_eq!(edited.fill, original.fill, "depth {depth}: the fill stays");
            assert_eq!(edited.live, original.live, "depth {depth}: the live shape stays");
            assert_eq!(steps(&h), before + 1, "depth {depth}: one edit is one history step");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.run_steps(3);
            assert_eq!(selected_shape(&h), original, "depth {depth}: undo restores the shape");
        }
    }
}

#[test]
fn selected_shape_numeric_drags_are_live_and_undo_as_separate_gestures() {
    let mut h = properties_harness(json!({"kind": "rect", "rect": [12, 12, 45, 30]}), 16);
    let before = steps(&h);
    let width = |h: &Harness<'_, Box<PhotocraftApp>>| match selected_shape(h).live {
        Some(photocraft_doc::vector::LiveShape::Rect { rect, .. }) => rect[2],
        other => panic!("expected a rectangle, got {other:?}"),
    };
    let start = width(&h);
    scrub(&mut h, "Width", 24.0);
    let first = width(&h);
    assert_ne!(first, start, "the scrub changes the width");
    assert_eq!(steps(&h), before + 1, "the whole scrub is one step");
    scrub(&mut h, "Width", 24.0);
    let second = width(&h);
    assert_ne!(second, first, "the second scrub changes it again");
    assert_eq!(steps(&h), before + 2, "a second scrub is a second step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(width(&h), first, "undo steps back one gesture");
}
