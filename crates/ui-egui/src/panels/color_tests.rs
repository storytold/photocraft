//! The Color tab's edits and the toolbar's colour chips, as the host runs them: a gesture is one
//! `tools.setColors` and one undo step of the recolour of what is selected, a background edit
//! recolours nothing (G3, #5568, #5690). How the field and the chips draw and pick is craft-color's.

use super::*;
use egui_kittest::kittest::Queryable;

/// The Color tab (Hue Cube) for `app` in a 300 x 200 pt group that it fills.
fn color_tab(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    // 60 fps steps, so two clicks a frame apart are a double-click.
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(300.0, 200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::color::panel(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(2);
    h
}

/// The foreground (0) or background (1) well: the pair's wells are radio buttons.
fn well<'a>(h: &'a egui_kittest::Harness<'_, Box<PhotocraftApp>>, i: usize) -> egui_kittest::Node<'a> {
    h.query_all_by_role(egui::accesskit::Role::RadioButton).nth(i).expect("a well")
}

/// The Hue Cube's field.
fn field(h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>) -> Rect {
    h.get_by_label("Saturation and brightness").rect()
}

fn click(h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>, p: egui::Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
}

fn drag(h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

/// A document with one shape layer filled `fill`, selected, the Rectangle tool in hand.
fn app_with_a_shape(fill: serde_json::Value) -> (Box<PhotocraftApp>, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = photocraft_doc::LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": fill})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = crate::state::Tool::Rectangle;
    (app, id)
}

fn shape_fill(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> Option<photocraft_doc::Fill> {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Shape(sh) => sh.fill.clone(),
        _ => panic!("not a shape layer"),
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// #5568: with a shape tool and a shape layer selected, each gesture in the field recolours the
/// fill (keeping its opacity) as its own undo step: a click, every drag, and a swatch followed by
/// a drag. A drag only reaches the field after its press frame, so it must not join the step
/// before it.
#[test]
fn field_gestures_recolour_the_selected_shape_in_separate_steps() {
    let (app, id) = app_with_a_shape(json!([0.2, 0.4, 0.6, 0.5]));
    let mut h = color_tab(app);
    let takes_foreground = |app: &PhotocraftApp| {
        let fg = app.session.tools.foreground;
        shape_fill(app, id) == Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(fg[0], fg[1], fg[2], 0.5)))
    };
    let before = steps(h.state());
    let f = field(&h);
    let at = |x: f32, y: f32| f.lerp_inside(vec2(x, y));
    // (gesture, from, to): a click where from == to.
    let gestures = [("the click", at(0.05, 0.1), at(0.05, 0.1)), ("a drag", at(0.3, 0.3), at(0.9, 0.8)), ("the next drag", at(0.7, 0.4), at(0.3, 0.9))];
    for (n, (gesture, from, to)) in gestures.into_iter().enumerate() {
        if from == to {
            click(&mut h, from)
        } else {
            drag(&mut h, from, to)
        }
        assert!(takes_foreground(h.state()), "{gesture}");
        assert_eq!(steps(h.state()), before + n + 1, "{gesture} is one step");
    }
    let fg = h.state().session.tools.foreground;
    let name = h.state_mut().run("swatches.list", json!({})).unwrap()["groups"][0]["swatches"][1]["name"].as_str().unwrap().to_string();
    crate::swatches_ui::use_swatch(h.state_mut(), &name, false);
    assert_ne!(h.state().session.tools.foreground, fg, "{name} is another colour");
    assert_eq!(steps(h.state()), before + 4, "the swatch");
    drag(&mut h, at(0.5, 0.2), at(0.8, 0.6));
    assert!(takes_foreground(h.state()));
    assert_eq!(steps(h.state()), before + 5, "a drag after a swatch is a step of its own");
}

/// A value typed into the readout's R, G or B field recolours the selected shape as one undo step
/// per field, not one per keystroke; a sum applies when Enter commits it, and a value the field
/// already holds adds no step.
#[test]
fn typing_an_rgb_value_recolours_the_selected_shape_in_one_step() {
    let (mut app, id) = app_with_a_shape(json!("#000000"));
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = color_tab(app);
    let rgb = |app: &PhotocraftApp| match shape_fill(app, id) {
        Some(photocraft_doc::Fill::Solid(c)) => [c.c[0], c.c[1], c.c[2]].map(|v| (v * 255.0).round() as u8),
        _ => panic!("a solid fill"),
    };
    let before = steps(h.state());
    // The only spin buttons are R, G and B, in that order. (field, typed, commit with Enter, steps after, fill)
    let cases = [
        (1, "128", false, 1, [0x00, 0x80, 0x00]),
        (2, "64", false, 2, [0x00, 0x80, 0x40]),
        (0, "2*64", true, 3, [0x80, 0x80, 0x40]),
        (0, "128", false, 3, [0x80, 0x80, 0x40]),
    ];
    for (field, text, enter, steps_after, want) in cases {
        h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(field).unwrap().click();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(1);
        for ch in text.chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        h.key_press(if enter { egui::Key::Enter } else { egui::Key::Tab });
        h.run_steps(2);
        assert_eq!(rgb(h.state()), want, "{text}");
        assert_eq!(steps(h.state()), before + steps_after, "{text}: one step for the typed value");
    }
}

/// A click on the background well makes the field edit the background: the foreground and the
/// selected shape keep their colours.
#[test]
fn background_edits_recolour_nothing() {
    let (mut app, id) = app_with_a_shape(json!("#000000"));
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
    let mut h = color_tab(app);
    let before = steps(h.state());
    well(&h, 1).click();
    h.run_steps(2);
    let f = field(&h);
    // The field's top-left: no saturation, full brightness.
    click(&mut h, f.lerp_inside(vec2(0.02, 0.02)));
    assert!(h.state().ui.color_panel.panel.background);
    let tools = &h.state().session.tools;
    assert_eq!(tools.foreground, [0.2, 0.4, 0.6, 1.0]);
    assert!(tools.background[..3].iter().all(|&v| v > 0.9), "{:?}", tools.background);
    assert_eq!(shape_fill(h.state(), id), Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 0.0, 1.0))));
    assert_eq!(steps(h.state()), before, "no recolour step");
}

/// While characters are highlighted in a type layer being edited, the Switch Colors and
/// Default Colors buttons recolour them with the new foreground (#5690); without a selection
/// they only change the tool colours, as before.
#[test]
fn swap_and_default_colors_recolour_selected_type() {
    use photocraft_doc::LayerId;
    let rgb = |app: &PhotocraftApp, id: LayerId, ci: usize| {
        let doc = &app.session.active().unwrap().doc;
        let photocraft_doc::LayerContent::Text(t) = &doc.layer(id).unwrap().content else { panic!("not a type layer") };
        let b = t.text.char_indices().nth(ci).map_or(t.text.len(), |(b, _)| b);
        let mut at = 0;
        for r in t.char_runs() {
            if b < at + r.len {
                return r.style.color.to_rgba8();
            }
            at += r.len;
        }
        [0; 4]
    };
    for selected in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let made = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 50, "y": 150, "color": "#000000"})).unwrap();
        let id = LayerId(made["layer"].as_u64().unwrap());
        app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
        let anchor = if selected { 6 } else { 11 };
        app.ui.text_edit = Some(crate::state::TextEdit { layer: id.0, document: None, caret: 11, anchor, session: "s".into(), created: false, mask: false });
        let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(80.0, 120.0)).build_ui_state(
            |ui, app: &mut Box<PhotocraftApp>| {
                let w = ui.available_width();
                color_chips(app, ui, w)
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
        h.run_steps(2);
        h.get_by_label("Switch Foreground and Background Colors (X)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(h.state().session.tools.background, [1.0, 0.0, 0.0, 1.0]);
        let blue = if selected { [0, 0, 255, 255] } else { [0, 0, 0, 255] };
        assert_eq!(rgb(h.state(), id, 0), [0, 0, 0, 255], "unselected characters keep their colour");
        assert_eq!((rgb(h.state(), id, 6), rgb(h.state(), id, 10)), (blue, blue), "selected: {selected}");
        // Default Colors: the selection takes black, the new foreground.
        h.state_mut().run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
        h.get_by_label("Default Foreground and Background Colors (D)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgb(h.state(), id, 6), [0, 0, 0, 255], "selected: {selected}");
    }
}

/// A click on the well the panel edits opens the Color Picker on that well; a click on the other
/// well only makes it the edited one.
#[test]
fn a_click_on_the_edited_well_opens_its_color_picker() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = color_tab(app);
    use crate::color_picker_ui::PickTarget;
    for (i, well_name, foreground) in [(1, "Background color", false), (0, "Foreground color", true)] {
        well(&h, i).click();
        h.run_steps(2);
        assert!(!h.state().dialog_open(), "{well_name}: the first click only makes it the edited well");
        well(&h, i).click();
        h.run_steps(2);
        assert_eq!(h.state().color_targets.len(), 1, "{well_name}: one Color Picker");
        let (&id, target) = h.state().color_targets.iter().next().unwrap();
        let expected = if foreground { matches!(target, PickTarget::Foreground) } else { matches!(target, PickTarget::Background) };
        assert!(expected, "{well_name}: the picker edits the clicked well");
        crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
        h.run_steps(2);
        assert!(!h.state().dialog_open(), "{well_name}: cancel closes the picker");
    }
}
