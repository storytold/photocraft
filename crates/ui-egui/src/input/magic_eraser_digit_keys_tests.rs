//! #11999: Magic Eraser digits set its Opacity; Shift digits set the same field.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

/// Send a current typed digit command at the old fixture's millisecond timestamp.
fn press(app: &mut PhotocraftApp, digit: u8, shift: bool, now_ms: f64) {
    use craft::ui::canvas::OpacityDigit as D;
    let commands = if shift {
        [D::SecondZero, D::SecondOne, D::SecondTwo, D::SecondThree, D::SecondFour, D::SecondFive, D::SecondSix, D::SecondSeven, D::SecondEight, D::SecondNine]
    } else {
        [
            D::OpacityZero,
            D::OpacityOne,
            D::OpacityTwo,
            D::OpacityThree,
            D::OpacityFour,
            D::OpacityFive,
            D::OpacitySix,
            D::OpacitySeven,
            D::OpacityEight,
            D::OpacityNine,
        ]
    };
    let mut fired = crate::blend_keys::test_fired(commands[usize::from(digit)].into());
    fired.time = now_ms / 1000.0;
    crate::opacity_keys::fired(app, &fired);
}

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

#[test]
fn digits_and_shift_digits_set_magic_eraser_opacity() {
    let mut a = app(Tool::MagicEraser);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0);
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 50.0);
    press(&mut a, 0, false, 5000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0);
    press(&mut a, 3, false, 10000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 30.0);
    press(&mut a, 4, false, 20000.0);
    press(&mut a, 5, false, 20300.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 45.0);
    let options = serde_json::to_value(&a.ui.tool_options).unwrap();
    assert_eq!(options["magic_eraser_opacity"], json!(45.0));
    press(&mut a, 5, true, 30000.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 50.0);
}

#[test]
fn magic_eraser_digits_leave_tolerance_layer_and_brush_alone() {
    let mut a = app(Tool::MagicEraser);
    let tolerance = a.ui.tool_options.tolerance;
    let layer_before = layer(&a);
    let brush = a.session.tools.brush.clone();
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.tolerance, tolerance);
    assert_eq!(layer(&a), layer_before);
    assert_eq!(a.session.tools.brush, brush);
    press(&mut a, 0, false, 5000.0);
    press(&mut a, 0, false, 5100.0);
    assert_eq!(a.ui.tool_options.magic_eraser_opacity, 100.0, "unmeasured 00 keeps the first zero's value");
}
#[test]
fn digit_pairs_preserve_the_existing_shared_cross_tool_sequence() {
    // The new target inherits the same pending-digit sequence as Main's existing tools.
    let mut brush_to_selected = app(Tool::Brush);
    press(&mut brush_to_selected, 4, false, 0.0);
    brush_to_selected.ui.tool = Tool::MagicEraser;
    press(&mut brush_to_selected, 5, false, 300.0);
    assert_eq!(brush_to_selected.session.tools.brush.opacity, 0.4);
    assert_eq!(brush_to_selected.ui.tool_options.magic_eraser_opacity, 45.0);

    // A selected-tool digit still contributes to the unchanged legacy Brush sequence.
    let mut selected_to_brush = app(Tool::MagicEraser);
    press(&mut selected_to_brush, 4, false, 0.0);
    selected_to_brush.ui.tool = Tool::Brush;
    press(&mut selected_to_brush, 5, false, 300.0);
    assert_eq!(selected_to_brush.ui.tool_options.magic_eraser_opacity, 40.0);
    assert_eq!(selected_to_brush.session.tools.brush.opacity, 0.45);

    // Consuming the first pair must also leave the next legacy digit unpaired.
    let mut through_selected = app(Tool::Brush);
    press(&mut through_selected, 4, false, 0.0);
    through_selected.ui.tool = Tool::MagicEraser;
    press(&mut through_selected, 5, false, 300.0);
    through_selected.ui.tool = Tool::Brush;
    press(&mut through_selected, 6, false, 400.0);
    assert_eq!(through_selected.ui.tool_options.magic_eraser_opacity, 45.0);
    assert_eq!(through_selected.session.tools.brush.opacity, 0.6);
}

/// The real current app routes plain and Shift digits to Magic Eraser, without changing any other option.
#[test]
fn real_app_keys_set_magic_eraser_opacity_without_other_changes() {
    use egui::{Event, Key, Modifiers};
    use egui_kittest::Harness;
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut a = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        a.run("layer.new.layer", json!({})).unwrap();
        a.ui.tool = Tool::MagicEraser;
        a
    });
    h.run_steps(4);
    let before = (h.state().ui.tool_options.clone(), h.state().session.tools.brush.clone(), layer(h.state()));
    let key = |h: &mut Harness<'_, Box<PhotocraftApp>>, modifiers| {
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::Key { key: Key::Num5, physical_key: Some(Key::Num5), pressed: true, repeat: false, modifiers });
        h.event(Event::Key { key: Key::Num5, physical_key: Some(Key::Num5), pressed: false, repeat: false, modifiers });
        h.run_steps(2);
    };
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT] {
        h.state_mut().ui.tool_options.magic_eraser_opacity = 17.0;
        let mut expected_options = before.0.clone();
        expected_options.magic_eraser_opacity = if modifiers.alt { 17.0 } else { 50.0 };
        key(&mut h, modifiers);
        assert_eq!(h.state().ui.tool_options, expected_options);
        assert_eq!(h.state().session.tools.brush, before.1);
        assert_eq!(layer(h.state()), before.2);
    }
}

/// Preserve every tool isolation check, including the Color Replacement target now on main.
#[test]
fn other_tools_keep_their_current_digit_targets() {
    for tool in [Tool::ColorReplacement, Tool::Dodge, Tool::Smudge] {
        let mut a = app(tool);
        let before = (a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone());
        let mut expected = before.clone();
        if tool == Tool::ColorReplacement {
            expected.2.cr_tolerance = 30.0;
        }
        press(&mut a, 3, false, 0.0);
        assert_eq!((a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone()), expected);
    }
}
