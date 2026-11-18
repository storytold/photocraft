//! Photoshop's tool keys that aren't plain shortcuts: Space+F cycles the pasteboard colour.

use craft::ui::shell::ScreenMode;
use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::shortcut_dispatch::tests::{app_harness, tap};

/// Space+F steps the pasteboard colour forward and ⇧Space+F back, round and round; F alone is
/// still Cycle Screen Mode.
#[test]
fn space_f_cycles_the_canvas_colour() {
    use photocraft_engine::prefs::CanvasColor;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    let mut h = app_harness(app);
    let color = |h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>| h.state().session.prefs().interface.canvas_color;
    let space = |h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>, pressed| {
        h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    space(&mut h, true);
    tap(&mut h, Key::F, Modifiers::NONE);
    assert_eq!(color(&h), CanvasColor::Black);
    tap(&mut h, Key::F, Modifiers::SHIFT);
    tap(&mut h, Key::F, Modifiers::SHIFT);
    assert_eq!(color(&h), CanvasColor::Custom, "backwards past Default wraps round");
    space(&mut h, false);
    assert_eq!(h.state().ui.view.screen_mode, ScreenMode::Standard);
    tap(&mut h, Key::F, Modifiers::NONE);
    assert_eq!(color(&h), CanvasColor::Custom, "F alone leaves the canvas colour");
    assert_ne!(h.state().ui.view.screen_mode, ScreenMode::Standard, "and cycles the screen mode");
}
