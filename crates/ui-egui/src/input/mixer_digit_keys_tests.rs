//! #11956: Mixer Brush digits set Wet, Shift sets Flow, and Alt sets Smoothing.

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

/// Send the current catalog command; the real-app test below also sends the actual chord.
fn alt_digit(app: &mut PhotocraftApp, key: egui::Key) {
    let digit = match key {
        egui::Key::Num0 => 0,
        egui::Key::Num3 => 3,
        egui::Key::Num4 => 4,
        egui::Key::Num5 => 5,
        _ => panic!("the fixture sends only these smoothing digits"),
    };
    let command = craft::ui::runtime::commands::CommandRef::new(format!("tools.smoothing.{digit}"));
    let fired = crate::blend_keys::test_fired(command);
    assert!(crate::opacity_keys::fired(app, &fired));
}

#[test]
fn plain_digits_set_wet_and_quick_pairs_are_exact() {
    let mut a = app(Tool::MixerBrush);
    press(&mut a, 4, false, 0.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.4);
    press(&mut a, 4, false, 5000.0);
    press(&mut a, 5, false, 5300.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.45);
    press(&mut a, 0, false, 10000.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 1.0);
}

#[test]
fn shift_sets_flow_alt_sets_smoothing_and_airbrush_does_not_swap_roles() {
    let mut a = app(Tool::MixerBrush);
    let load_mix = (a.session.tools.brush.mixer.load, a.session.tools.brush.mixer.mix);
    let opacity = a.session.tools.brush.opacity;
    press(&mut a, 3, true, 0.0);
    assert_eq!(a.session.tools.brush.mixer.flow, 0.3);
    press(&mut a, 0, true, 5000.0);
    assert_eq!(a.session.tools.brush.mixer.flow, 1.0);
    press(&mut a, 3, false, 10000.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.3);
    // Alt path is exercised through the same event handler as a real key event.
    alt_digit(&mut a, egui::Key::Num3);
    assert!((a.session.tools.brush.smoothing.amount - 0.3).abs() < 1e-6);
    alt_digit(&mut a, egui::Key::Num0);
    assert_eq!(a.session.tools.brush.smoothing.amount, 1.0, "Alt+0 = 100% smoothing");
    assert_eq!((a.session.tools.brush.mixer.load, a.session.tools.brush.mixer.mix), load_mix);
    assert_eq!(a.session.tools.brush.opacity, opacity);
    a.session.tools.brush.build_up = true;
    press(&mut a, 4, false, 30000.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.4, "Airbrush build-up does not swap Mixer Brush roles");
    press(&mut a, 6, true, 40000.0);
    assert_eq!(a.session.tools.brush.mixer.flow, 0.6);
}
#[test]
fn digit_pairs_preserve_the_existing_shared_cross_tool_sequence() {
    // The new target inherits the same pending-digit sequence as Main's existing tools.
    let mut brush_to_selected = app(Tool::Brush);
    press(&mut brush_to_selected, 4, false, 0.0);
    brush_to_selected.ui.tool = Tool::MixerBrush;
    press(&mut brush_to_selected, 5, false, 300.0);
    assert_eq!(brush_to_selected.session.tools.brush.opacity, 0.4);
    assert_eq!(brush_to_selected.session.tools.brush.mixer.wet, 0.45);

    // A selected-tool digit still contributes to the unchanged legacy Brush sequence.
    let mut selected_to_brush = app(Tool::MixerBrush);
    press(&mut selected_to_brush, 4, false, 0.0);
    selected_to_brush.ui.tool = Tool::Brush;
    press(&mut selected_to_brush, 5, false, 300.0);
    assert_eq!(selected_to_brush.session.tools.brush.mixer.wet, 0.4);
    assert_eq!(selected_to_brush.session.tools.brush.opacity, 0.45);

    // Consuming the first pair must also leave the next legacy digit unpaired.
    let mut through_selected = app(Tool::Brush);
    press(&mut through_selected, 4, false, 0.0);
    through_selected.ui.tool = Tool::MixerBrush;
    press(&mut through_selected, 5, false, 300.0);
    through_selected.ui.tool = Tool::Brush;
    press(&mut through_selected, 6, false, 400.0);
    assert_eq!(through_selected.session.tools.brush.mixer.wet, 0.45);
    assert_eq!(through_selected.session.tools.brush.opacity, 0.6);
}

fn layer(app: &PhotocraftApp) -> (f32, f32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    (l.opacity, l.fill_opacity)
}

/// Main's Shift pair rule and individual Alt tens values remain the shared rules for Mixer Brush.
#[test]
fn shift_pairs_and_smoothing_keep_the_current_shared_rules() {
    let mut a = app(Tool::MixerBrush);
    press(&mut a, 4, true, 0.0);
    press(&mut a, 5, true, 300.0);
    assert_eq!(a.session.tools.brush.mixer.flow, 0.45);
    press(&mut a, 4, false, 5000.0);
    alt_digit(&mut a, egui::Key::Num3);
    press(&mut a, 5, false, 5300.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.5, "Smoothing ends the pending pair");
    alt_digit(&mut a, egui::Key::Num4);
    assert_eq!(a.session.tools.brush.smoothing.amount, 0.4);
    alt_digit(&mut a, egui::Key::Num5);
    assert_eq!(a.session.tools.brush.smoothing.amount, 0.5, "Alt digits never pair");
    assert_eq!(a.session.tools.brush.mixer.flow, 0.45);
}

/// Exercise current full-App focus/shortcut dispatch and preserve every unrelated setting.
#[test]
fn real_app_keys_set_only_mixer_wet_flow_and_smoothing_with_either_airbrush_mode() {
    use egui::{Event, Key, Modifiers};
    use egui_kittest::Harness;
    for build_up in [false, true] {
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
            a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
            a.run("layer.new.layer", json!({})).unwrap();
            a.ui.tool = Tool::MixerBrush;
            a
        });
        h.run_steps(4);
        h.state_mut().session.tools.brush.build_up = build_up;
        let options = h.state().ui.tool_options.clone();
        let before_layer = layer(h.state());
        let mut expected = (*h.state().session.tools.brush).clone();
        for (key, modifiers, wet, flow, smoothing) in [
            (Key::Num4, Modifiers::NONE, Some(0.4), None, None),
            (Key::Num6, Modifiers::SHIFT, None, Some(0.6), None),
            (Key::Num3, Modifiers::ALT, None, None, Some(0.3)),
        ] {
            h.event(Event::ModifiersChanged(modifiers));
            h.event(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
            h.event(Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers });
            h.run_steps(2);
            if let Some(wet) = wet {
                expected.mixer.wet = wet;
            }
            if let Some(flow) = flow {
                expected.mixer.flow = flow;
            }
            if let Some(smoothing) = smoothing {
                expected.smoothing.enabled = true;
                expected.smoothing.amount = smoothing;
            }
            assert_eq!(*h.state().session.tools.brush, expected);
            assert_eq!(h.state().ui.tool_options, options);
            assert_eq!(layer(h.state()), before_layer);
        }
    }
}

/// Preserve all isolation checks, including the Color Replacement and Magic Eraser targets now on main.
#[test]
fn other_tools_keep_their_current_digit_targets() {
    for tool in [Tool::ColorReplacement, Tool::MagicEraser, Tool::Dodge, Tool::Smudge] {
        let mut a = app(tool);
        let before = (a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone());
        let mut expected = before.clone();
        if tool == Tool::ColorReplacement {
            expected.2.cr_tolerance = 30.0;
        } else if tool == Tool::MagicEraser {
            expected.2.magic_eraser_opacity = 30.0;
        }
        press(&mut a, 3, false, 0.0);
        assert_eq!((a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.clone()), expected);
    }
}
