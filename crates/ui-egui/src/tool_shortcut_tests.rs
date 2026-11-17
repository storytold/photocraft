//! Photoshop's keys that aren't plain tool shortcuts: Space+F cycles the pasteboard colour, and F and ⇧F
//! the screen modes.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app
}

/// Runs one frame of `events` through the app's keyboard shortcuts.
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    ctx.begin_pass(egui::RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// A key press or release event with `modifiers` held.
fn key(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// Space+F steps the pasteboard colour forward and ⇧Space+F back, round and round; F alone is
/// still Cycle Screen Mode.
#[test]
fn space_f_cycles_the_canvas_colour() {
    use photocraft_engine::prefs::CanvasColor;
    let mut app = app();
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| frame(app, &ctx, events);
    let color = |app: &PhotocraftApp| app.session.prefs().interface.canvas_color;
    frame(&mut app, vec![key(Key::Space, true, Modifiers::NONE)]);
    frame(&mut app, vec![key(Key::F, true, Modifiers::NONE), key(Key::F, false, Modifiers::NONE)]);
    assert_eq!(color(&app), CanvasColor::Black);
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), key(Key::F, true, Modifiers::SHIFT), key(Key::F, false, Modifiers::SHIFT)]);
    frame(&mut app, vec![key(Key::F, true, Modifiers::SHIFT), key(Key::F, false, Modifiers::SHIFT)]);
    assert_eq!(color(&app), CanvasColor::Custom, "backwards past Default wraps round");
    assert_eq!(app.ui.view.screen_mode, "standard", "⇧Space+F leaves the screen mode (#15404)");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::NONE), key(Key::Space, false, Modifiers::NONE)]);
    let mode = app.ui.view.screen_mode.clone();
    frame(&mut app, vec![key(Key::F, true, Modifiers::NONE), key(Key::F, false, Modifiers::NONE)]);
    assert_eq!(color(&app), CanvasColor::Custom, "F alone leaves the canvas colour");
    assert_ne!(app.ui.view.screen_mode, mode, "and cycles the screen mode");
}

/// #15404: ⇧F cycles the screen modes backward, as Photoshop's "Cycle through screen modes
/// (backward)": Standard, Full Screen Mode, Full Screen Mode With Menu Bar, Standard. F still
/// steps forward from wherever ⇧F left off.
#[test]
fn shift_f_cycles_the_screen_modes_backward() {
    let mut app = app();
    let ctx = egui::Context::default();
    let tap = |app: &mut PhotocraftApp, modifiers: Modifiers| {
        frame(app, &ctx, vec![egui::Event::ModifiersChanged(modifiers), key(Key::F, true, modifiers), key(Key::F, false, modifiers)]);
        app.ui.view.screen_mode.clone()
    };
    assert_eq!(app.ui.view.screen_mode, "standard");
    let backward: Vec<String> = (0..3).map(|_| tap(&mut app, Modifiers::SHIFT)).collect();
    assert_eq!(backward, ["fullScreen", "fullScreenWithMenuBar", "standard"]);
    assert_eq!(tap(&mut app, Modifiers::SHIFT), "fullScreen");
    let forward: Vec<String> = (0..2).map(|_| tap(&mut app, Modifiers::NONE)).collect();
    assert_eq!(forward, ["standard", "fullScreenWithMenuBar"], "F steps forward again");
    assert_eq!(tap(&mut app, Modifiers::SHIFT), "standard", "and ⇧F back");
}
