use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::theme::Srgba;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// The colour picker `id` shows, as `"#rrggbb"`.
fn dialog_color(app: &PhotocraftApp, id: DialogId) -> String {
    rgb_hex(testing::current(app, id).expect("the picker is open").color)
}

/// A harness running the app's dialog pass on the app's own context, so a key or a click reaches the hosted picker.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(760.0, 480.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app)
}

/// The one place a picker's result reaches the app is `take` → `commit`: OK and Enter set only the colour the picker
/// was opened for, Cancel and Escape change neither.
#[test]
fn side_buttons_and_keyboard_confirm_or_cancel_without_changing_the_other_color() {
    for target in ["foreground", "background"] {
        for action in ["OK", "Cancel", "Enter", "Escape"] {
            let mut h = harness(PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()));
            let original_fg = h.state().session.tools.foreground;
            let original_bg = h.state().session.tools.background;
            let id = open(h.state_mut(), target);
            testing::set_color(h.state_mut(), id, "#336699");
            h.run_steps(4);
            match action {
                "Enter" => h.key_press(egui::Key::Enter),
                "Escape" => h.key_press(egui::Key::Escape),
                _ => h.get_by_label(action).click(),
            }
            h.run_steps(3);
            assert!(h.state().framework.dialogs.get_by_id::<ColorPickerDialog>(id).is_none(), "{target}: {action} closes the picker");
            let confirm = matches!(action, "OK" | "Enter");
            let tools = &h.state().session.tools;
            if confirm && target == "foreground" {
                assert_eq!(hex([tools.foreground[0], tools.foreground[1], tools.foreground[2]]), "#336699", "{target}: {action}");
            } else {
                assert_eq!(tools.foreground, original_fg, "{target}: {action}");
            }
            if confirm && target == "background" {
                assert_eq!(hex([tools.background[0], tools.background[1], tools.background[2]]), "#336699", "{target}: {action}");
            } else {
                assert_eq!(tools.background, original_bg, "{target}: {action}");
            }
        }
    }
}

/// Hex parsing refuses text whose bytes split a character instead of panicking.
#[test]
fn parse_hex_rejects_multibyte_text() {
    // Six bytes, three characters: the byte range 0..2 splits "é", which used to panic.
    assert_eq!(parse_hex("aé€"), None);
    assert_eq!(parse_hex("#ff880"), None);
}

/// `from_str_radix` accepts a leading `+`, which a hex colour must refuse.
#[test]
fn parse_hex_rejects_signs() {
    assert_eq!(parse_hex("#+f+f+f"), None);
    assert_eq!(parse_hex("+1+2+3"), None);
    assert!(parse_hex("#0f0f0f").is_some());
}

/// #4196: CMYK values typed into the picker survive OK, so the colour fills a CMYK document with those inks and
/// reopening the picker shows them again.
#[test]
fn cmyk_values_survive_ok_and_a_reopen() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let cyan = [1.0, 0.0, 0.0, 0.0];
    let id = open(&mut app, "foreground");
    let rgb = Srgba::from_rgb_array(photocraft_color::convert::cmyk_to_rgb(cyan).map(|c| c.clamp(0.0, 1.0)));
    // The picker's CMYK fields are percent.
    app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id).expect("the picker is open").set_color(PickedColor::cmyk(rgb, cyan.map(|v| v * 100.0)));
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let fg = app.session.tools.foreground;
    assert_eq!(app.session.tools.picked_cmyk([fg[0], fg[1], fg[2]]), Some(cyan));
    let id = open(&mut app, "foreground");
    let again = testing::current(&app, id).expect("the reopened picker shows a colour");
    assert_eq!(inks_of(&again), Some(cyan), "the reopened picker is seeded with the inks");
}

/// A frame of the canvas and the dialogs, as the app runs them, so queued pointer input is delivered.
fn frames(app: &mut PhotocraftApp) {
    for _ in 0..3 {
        crate::canvas::testing::frame(app);
    }
}

/// A press on the canvas at document point `(x, y). The dialog opens centred over the canvas and takes the presses
/// that land on it, so the view first shows a point the press can reach.
fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) {
    if x.abs() < 1e6 && y.abs() < 1e6 {
        crate::canvas::testing::clear_of_dialogs(app, [x, y]);
    }
    crate::canvas::testing::click(app, [x, y], egui::Modifiers::NONE);
}

/// The top picker takes the colour under a canvas press, and nothing off the image, over transparency or at a point
/// that is not finite; `ui.pointer` samples instead of driving the Brush; a dialog opened over the picker takes the
/// input.
#[test]
fn the_top_picker_samples_the_image() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 10, 20], "fill": "#00ff00"})).unwrap();
    let id = open(&mut app, "foreground");
    assert_eq!(top(&app), Some(id));
    sample_at(&mut app, 25.5, 5.5);
    assert_eq!(dialog_color(&app, id), "#00ff00");
    let opening = app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).and_then(|d| d.opening()).expect("the picker remembers its opening colour");
    assert_eq!(rgb_hex(opening.color), "#000000", "the current colour stays");
    // Off the image or over transparency nothing changes.
    for (x, y) in [(35.0, 5.0), (-3.0, 5.0), (5.0, 20.0), (1e12, -1e12), (f64::NAN, f64::INFINITY)] {
        sample_at(&mut app, x, y);
        assert_eq!(dialog_color(&app, id), "#00ff00", "({x}, {y})");
    }

    // Agents: `ui.pointer` samples into the picker instead of driving the tool. The events queue and are delivered on
    // the next pass, so frames run before the colour is read.
    app.ui.tool = crate::state::Tool::Brush;
    let rev = app.session.active().unwrap().revision;
    let ctx = app.ctx().clone();
    crate::canvas::testing::clear_of_dialogs(&mut app, [5.0, 5.0]);
    let events = json!([{"kind": "down", "x": 5, "y": 5}, {"kind": "up", "x": 5, "y": 5}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(&mut app, &ctx, &req);
    frames(&mut app);
    assert_eq!(dialog_color(&app, id), "#ff0000");
    assert_eq!(app.session.active().unwrap().revision, rev, "the Brush must not paint");
    assert_eq!(app.session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "only OK sets the foreground");

    // A dialog opened over the picker takes the input.
    let over = crate::dialogs::open_command_dialog(&mut app, "edit.fill", "Fill").expect("Fill opens over the picker");
    assert_eq!(top(&app), None);
    sample_at(&mut app, 25.0, 5.0);
    assert_eq!(dialog_color(&app, id), "#ff0000");
    crate::dialogs::testing::cancel(&mut app, over).unwrap();
    assert_eq!(top(&app), Some(id), "the picker is on top again");
    sample_at(&mut app, 25.0, 5.0);
    assert_eq!(dialog_color(&app, id), "#00ff00");
}
