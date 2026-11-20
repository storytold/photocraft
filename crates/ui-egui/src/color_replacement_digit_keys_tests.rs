//! #11927: Color Replacement digits set its Tolerance, while Shift/Alt remain no-ops.

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

/// Alt+5's current catalog command is ignored by Color Replacement; the app test below also sends the real chord.
fn alt_digit(app: &mut PhotocraftApp, key: egui::Key) {
    assert_eq!(key, egui::Key::Num5);
    let fired = crate::blend_keys::test_fired(craft::ui::runtime::commands::CommandRef::new("tools.smoothing.5"));
    assert!(!crate::opacity_keys::fired(app, &fired));
}

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

#[test]
fn digits_set_color_replacement_tolerance_and_quick_pairs_are_exact() {
    let mut a = app(Tool::ColorReplacement);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0);
    press(&mut a, 5, false, 0.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 50.0);
    press(&mut a, 0, false, 5000.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 100.0);
    press(&mut a, 1, false, 10000.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 10.0);
    press(&mut a, 4, false, 20000.0);
    press(&mut a, 5, false, 20300.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 45.0);
    let options = serde_json::to_value(&a.ui.tool_options).unwrap();
    assert_eq!(options["cr_tolerance"], json!(45.0));
}

#[test]
fn modifiers_and_double_zero_preserve_color_replacement_and_layer_state() {
    let mut a = app(Tool::ColorReplacement);
    let before_layer = layer(&a);
    press(&mut a, 5, true, 0.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0, "Shift+digit is unchanged");
    alt_digit(&mut a, egui::Key::Num5);
    assert_eq!(a.ui.tool_options.cr_tolerance, 30.0, "Alt+digit is unchanged");
    press(&mut a, 0, false, 5000.0);
    press(&mut a, 0, false, 5100.0);
    assert_eq!(a.ui.tool_options.cr_tolerance, 100.0, "unmeasured 00 keeps the first zero's value");
    assert_eq!(layer(&a), before_layer);
    let brush = a.session.tools.brush.clone();
    let opacity = a.ui.tool_options.fill_opacity;
    let bg_tolerance = a.ui.tool_options.bg_tolerance;
    a.ui.tool = Tool::ColorReplacement;
    press(&mut a, 3, false, 9000.0);
    assert_eq!(a.session.tools.brush, brush);
    assert_eq!(a.ui.tool_options.fill_opacity, opacity);
    assert_eq!(a.ui.tool_options.bg_tolerance, bg_tolerance);
}
#[test]
fn digit_pairs_preserve_the_existing_shared_cross_tool_sequence() {
    // The new target inherits the same pending-digit sequence as Main's existing tools.
    let mut brush_to_selected = app(Tool::Brush);
    press(&mut brush_to_selected, 4, false, 0.0);
    brush_to_selected.ui.tool = Tool::ColorReplacement;
    press(&mut brush_to_selected, 5, false, 300.0);
    assert_eq!(brush_to_selected.session.tools.brush.opacity, 0.4);
    assert_eq!(brush_to_selected.ui.tool_options.cr_tolerance, 45.0);

    // A selected-tool digit still contributes to the unchanged legacy Brush sequence.
    let mut selected_to_brush = app(Tool::ColorReplacement);
    press(&mut selected_to_brush, 4, false, 0.0);
    selected_to_brush.ui.tool = Tool::Brush;
    press(&mut selected_to_brush, 5, false, 300.0);
    assert_eq!(selected_to_brush.ui.tool_options.cr_tolerance, 40.0);
    assert_eq!(selected_to_brush.session.tools.brush.opacity, 0.45);

    // Consuming the first pair must also leave the next legacy digit unpaired.
    let mut through_selected = app(Tool::Brush);
    press(&mut through_selected, 4, false, 0.0);
    through_selected.ui.tool = Tool::ColorReplacement;
    press(&mut through_selected, 5, false, 300.0);
    through_selected.ui.tool = Tool::Brush;
    press(&mut through_selected, 6, false, 400.0);
    assert_eq!(through_selected.ui.tool_options.cr_tolerance, 45.0);
    assert_eq!(through_selected.session.tools.brush.opacity, 0.6);
}

/// Through the real current app/keymap: modifiers leave every option alone, and a plain digit reaches Tolerance.
#[test]
fn real_app_keys_set_tolerance_and_modifiers_leave_all_options_alone() {
    use egui::{Event, Key, Modifiers};
    use egui_kittest::Harness;
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut a = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        a.run("layer.new.layer", json!({})).unwrap();
        a.ui.tool = Tool::ColorReplacement;
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
    for modifiers in [Modifiers::SHIFT, Modifiers::ALT] {
        key(&mut h, modifiers);
        assert_eq!((h.state().ui.tool_options.clone(), h.state().session.tools.brush.clone(), layer(h.state())), before);
    }
    key(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool_options.cr_tolerance, 50.0);
    assert_eq!(h.state().session.tools.brush, before.1);
    assert_eq!(layer(h.state()), before.2);
}

/// Keep the existing isolation checks while Magic Eraser now has its own digit target (#11999).
#[test]
fn other_tools_keep_their_current_digit_targets() {
    for tool in [Tool::Dodge, Tool::Smudge, Tool::MagicEraser] {
        let mut a = app(tool);
        let before = (a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone());
        let mut expected = before.clone();
        if tool == Tool::MagicEraser {
            expected.2.magic_eraser_opacity = 30.0;
        }
        press(&mut a, 3, false, 0.0);
        assert_eq!((a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone()), expected);
    }
}
