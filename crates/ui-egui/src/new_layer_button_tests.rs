//! The New Layer shortcut: ⇧⌘N asks first, ⌥⇧⌘N makes the layer at once (#4794).

use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;

fn panel() -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(4);
    h
}

fn layers(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().doc.layers.len()
}

fn dialog_open(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.state().framework.dialogs.get::<HostedParamDialog>().is_some_and(|d| d.key().is_some_and(crate::new_layer_ui::owns))
}

#[test]
fn shortcut_asks_first_and_alt_shortcut_makes_a_layer_at_once() {
    let mut h = panel();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::N);
    h.run_steps(4);
    assert_eq!(layers(&h), 1, "⇧⌘N asks first");
    assert!(dialog_open(&h), "the New Layer dialog");

    let mut h = panel();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::ALT, egui::Key::N);
    h.run_steps(4);
    assert_eq!(layers(&h), 2, "⌥⇧⌘N makes the layer");
    assert!(!dialog_open(&h));
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::ALT, egui::Key::N);
    h.run_steps(4);
    assert_eq!(layers(&h), 3, "once per press");
}
