//! #12038: with the Ruler tool, a Ctrl+drag (⌘ on the Mac, and what the control channel's
//! `command: true` sends) is the temporary Move tool, as `docs/control-protocol.md` promises for a
//! non-selection tool and as it is from the keyboard: the drag moves the layer as one "Move"
//! history step, the ruler stays as it was, and the tool stays the Ruler. The Count and Note tools
//! do the same (the issue's last "Done when" item). The Color Sampler is left to #11731.
//!
//! Every gesture here goes through the window host (`canvas::testing`), the route a hand's input
//! and the control channel's `ui.pointer` both take.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use photocraft_engine::doc::Ruler;

/// Ctrl as a Windows or Linux press sends it, which is also what the control channel builds for
/// `ui.pointer {"command": true}`.
const CTRL: Modifiers = Modifiers { ctrl: true, command: true, shift: false, alt: false, mac_cmd: false };

/// A 400 × 300 document with a red layer covering (100,100)..(160,160), the Ruler from 60,60 to
/// 340,240, and the Ruler tool active. Snapping, smart guides and the Move tool's Auto-Select are
/// off so the drag offsets are exact.
fn ruler_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 100, 160, 160), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [60.0, 60.0], "end": [340.0, 240.0]})).unwrap();
    app.ui.tool = Tool::Ruler;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn ruler(app: &PhotocraftApp) -> Option<Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

fn bounds(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> Rect {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

/// A press at `from` and a release at `to`, `m` held through the whole gesture. The last move
/// lands where the release is, so the Ruler tool's drag ends at `to` too.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// What a Ctrl+drag with the Ruler does: the layer moves, as one "Move" step, the gesture is over;
/// the ruler and the tool stay untouched.
fn check_moves_the_layer(app: &PhotocraftApp, id: photocraft_doc::LayerId, steps: usize, before: Option<Ruler>) {
    assert_eq!(bounds(app, id), Rect::new(150, 130, 210, 190), "the layer moved +50,+30");
    assert_eq!(ruler(app), before, "the ruler stays");
    assert_eq!(history(app), steps + 1, "one history step");
    assert_eq!(label(app).as_deref(), Some("Move"), "the Move tool's history step");
    assert_eq!(app.ui.tool, Tool::Ruler, "the tool stays the Ruler");
    assert!(!testing::in_gesture(app), "the release ended the gesture");
}

#[test]
fn a_ctrl_drag_with_the_ruler_is_the_move_tool() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    drag(&mut app, [120.0, 120.0], [170.0, 150.0], CTRL);
    check_moves_the_layer(&app, id, steps, before);
}

/// The reported route: `ui.pointer` with `command: true`, as docs/control-protocol.md documents it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_control_channels_command_true_with_the_ruler_is_the_move_tool() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    let params = json!({"command": true, "events": [{"kind": "down", "x": 120.0, "y": 120.0}, {"kind": "move", "x": 145.0, "y": 135.0}, {"kind": "up", "x": 170.0, "y": 150.0}]});
    let r = testing::control(&mut app, "ui.pointer", params);
    assert_eq!(r["ok"], true, "{r}");
    check_moves_the_layer(&app, id, steps, before);
}

/// The Ruler tool's own drag, with no modifier held, is still the Ruler's: it changes the line and leaves the layer.
#[test]
fn a_drag_without_ctrl_is_still_the_rulers() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    drag(&mut app, [120.0, 120.0], [170.0, 150.0], Modifiers::NONE);
    let after = ruler(&app).expect("a ruler line");
    assert_ne!(Some(after), before, "the Ruler tool took the drag");
    assert_eq!(after.end, [170.0, 150.0], "the line ends at the release");
    assert_eq!(bounds(&app, id), Rect::new(100, 100, 160, 160), "the layer stays");
    assert_eq!(history(&app), steps, "a ruler is no history step");
}

/// A Move drag that started with Ctrl keeps the Move tool for the rest of the gesture: a release
/// without the modifier (the key let go before the button) still ends the drag as a Move.
#[test]
fn a_ctrl_drag_released_without_ctrl_still_finishes_as_the_move_tool() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Move { x: 145.0, y: 135.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Move { x: 170.0, y: 150.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Up { x: 170.0, y: 150.0 }, Modifiers::NONE);
    check_moves_the_layer(&app, id, steps, before);
}

/// The same over the control channel: Command on the first request only.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_control_channel_ctrl_drag_released_without_command_still_finishes_as_the_move_tool() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    let down = json!({"command": true, "events": [{"kind": "down", "x": 120.0, "y": 120.0}, {"kind": "move", "x": 145.0, "y": 135.0}, {"kind": "move", "x": 170.0, "y": 150.0}]});
    let up = json!({"events": [{"kind": "up", "x": 170.0, "y": 150.0}]});
    for params in [down, up] {
        let r = testing::control(&mut app, "ui.pointer", params);
        assert_eq!(r["ok"], true, "{r}");
    }
    check_moves_the_layer(&app, id, steps, before);
}

/// Count and Note: a Ctrl-drag is the temporary Move tool there too: the layer moves as one
/// "Move" step, and no marker or note is created.
#[test]
fn ctrl_with_the_count_and_note_tools_is_the_move_tool() {
    for tool in [Tool::Count, Tool::Note] {
        let mut app = ruler_app();
        app.ui.tool = tool;
        let id = app.session.active().unwrap().active_layer.unwrap();
        let (steps_before, before) = (history(&app), ruler(&app));
        drag(&mut app, [120.0, 120.0], [170.0, 150.0], CTRL);
        assert_eq!(bounds(&app, id), Rect::new(150, 130, 210, 190), "{tool:?}: the layer moved +50,+30");
        assert_eq!(history(&app), steps_before + 1, "{tool:?}: one history step");
        assert_eq!(label(&app).as_deref(), Some("Move"), "{tool:?}: the Move tool's step");
        assert_eq!(ruler(&app), before, "{tool:?}: the ruler stays");
        assert_eq!(app.ui.tool, tool, "{tool:?}: the tool stays");
        assert_eq!(app.session.active().unwrap().doc.measurement.count_total(), 0, "{tool:?}: no marker");
        assert!(app.session.active().unwrap().doc.notes.is_empty(), "{tool:?}: no note");
    }
}

/// A Ctrl press on a guide takes the guide; a release without Ctrl still finishes that guide drag
/// instead of the Ruler tool taking the event.
#[test]
fn a_guide_drag_started_with_ctrl_finishes_when_released_without_ctrl() {
    let mut app = ruler_app();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 300})).unwrap();
    let before = ruler(&app);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 120.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Move { x: 310.0, y: 120.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Move { x: 320.0, y: 120.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Up { x: 320.0, y: 120.0 }, Modifiers::NONE);
    assert!(!testing::in_gesture(&app), "the release finished the guide drag");
    assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![320.0], "the guide moved to the release");
    assert_eq!(ruler(&app), before, "the ruler stays");
    assert_eq!(app.ui.tool, Tool::Ruler, "the tool stays the Ruler");
}
