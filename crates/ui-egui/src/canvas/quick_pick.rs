//! ⌘⌥⌃-click on the canvas, with any tool: select the topmost visible layer with pixels under the
//! pointer (`layer.pickAt`), without switching to the Move tool. The press is the tool preflight's whole
//! (`ToolAction::Done`): the active tool never sees its drag or release, and the ⌃⌥ brush-resize gesture, which
//! would otherwise claim the press, never starts.
//!
//! Platforms: the gesture is macOS-only, because ⌘ is the only key that tells it apart from the
//! ⌃⌥ brush resize. Off macOS the window host reports Ctrl as ⌘ and never sets Ctrl with it, so there it is a
//! no-op and ⌃⌥-drag keeps resizing the brush; the Move tool's Auto-Select picks layers on every platform.
//! A failed pick is shown as a status-bar error and never panics.

use craft::behavior::kurbo::Point;
use craft::keymap::Mods;
use craft::ui::canvas::ToolPointer;
use serde_json::json;

use crate::PhotocraftApp;

/// ⌘ + ⌥ + ⌃ held together, as the window host reports the keys.
fn chord(mods: Mods) -> bool {
    mods.cmd && mods.alt && mods.ctrl
}

/// ⌘ + ⌥ + ⌃ held together. Always false off macOS, where egui never sets `mac_cmd`.
pub fn is_gesture(mods: egui::Modifiers) -> bool {
    chord(Mods { cmd: mods.mac_cmd, ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift })
}

/// Whether the press is a quick pick: the chord is held and a document is open. The tool preflight asks this to
/// take the press before any tool or stand-in does.
pub fn takes(app: &PhotocraftApp, mods: Mods) -> bool {
    chord(mods) && app.session.active().is_some()
}

/// Picks the layer under `doc` for a press with the chord held; true when the press was a pick (the preflight then
/// answers `ToolAction::Done`). A failed pick leaves the layer and shows the error in the status bar.
pub fn press(app: &mut PhotocraftApp, pointer: &ToolPointer, doc: Point) -> bool {
    if !takes(app, pointer.mods) {
        return false;
    }
    if let Err(e) = app.run("layer.pickAt", json!({"x": doc.x, "y": doc.y, "target": "layer", "mode": "replace"})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;
    use photocraft_geom::Rect;
    use serde_json::json;

    use super::*;
    use crate::canvas::testing;
    use crate::state::Tool;

    const PICK: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: true, command: true };

    /// A 64×64 document with A painted at (8..24)² and B at (40..56)²; B is active.
    fn app() -> (PhotocraftApp, photocraft_doc::LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        testing::on_a_mac(&app);
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        let mut first = None;
        for at in [8, 40] {
            app.session.execute("layer.new.layer", json!({})).unwrap();
            app.session
                .edit("paint", |doc, a| {
                    doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(at, at, at + 16, at + 16), &[1.0, 0.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            first.get_or_insert(app.session.active().unwrap().active_layer.unwrap());
        }
        (app, first.unwrap())
    }

    #[test]
    fn picks_the_layer_under_the_pointer_with_a_painting_tool() {
        let (mut app, a) = app();
        app.ui.tool = Tool::Brush;
        let before = app.session.active().unwrap().history.past_len();
        testing::press(&mut app, [12.0, 12.0], PICK);
        let picked = !testing::in_gesture(&app);
        testing::move_to(&mut app, [20.0, 20.0], PICK);
        testing::release(&mut app, [20.0, 20.0], PICK);
        let st = app.session.active().unwrap();
        assert_eq!(st.active_layer, Some(a));
        assert!(picked && !testing::in_gesture(&app) && !app.brush_resize.active(), "neither painted nor resized the brush");
        assert_eq!(st.history.past_len(), before, "no paint stroke was recorded");
    }
}
