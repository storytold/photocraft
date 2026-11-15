//! #11956: Mixer Brush digits set Wet, Shift sets Flow, and Alt sets Smoothing.

use serde_json::json;

use crate::PhotocraftApp;
use crate::opacity_keys::press;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

/// Send one key event through the digit handler.
fn alt_digit(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let modifiers = egui::Modifiers::ALT;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::opacity_keys::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
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
    crate::opacity_keys::press(&mut a, 4, false, 30000.0);
    assert_eq!(a.session.tools.brush.mixer.wet, 0.4, "Airbrush build-up does not swap Mixer Brush roles");
    crate::opacity_keys::press(&mut a, 6, true, 40000.0);
    assert_eq!(a.session.tools.brush.mixer.flow, 0.6);
}
#[test]
fn digits_do_not_pair_across_tools() {
    let mut brush_to_mixer = app(Tool::Brush);
    press(&mut brush_to_mixer, 4, false, 0.0);
    brush_to_mixer.ui.tool = Tool::MixerBrush;
    press(&mut brush_to_mixer, 5, false, 300.0);
    assert_eq!(brush_to_mixer.session.tools.brush.opacity, 0.4);
    assert_eq!(brush_to_mixer.session.tools.brush.mixer.wet, 0.5);

    let mut mixer_to_brush = app(Tool::MixerBrush);
    press(&mut mixer_to_brush, 4, false, 0.0);
    mixer_to_brush.ui.tool = Tool::Brush;
    press(&mut mixer_to_brush, 5, false, 300.0);
    assert_eq!(mixer_to_brush.session.tools.brush.mixer.wet, 0.4);
    assert_eq!(mixer_to_brush.session.tools.brush.opacity, 0.5);
}
