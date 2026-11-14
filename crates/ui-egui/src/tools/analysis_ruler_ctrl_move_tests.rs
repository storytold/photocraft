//! #12038: with the Ruler tool, a Ctrl+drag (âŒ˜ on the Mac, and what the control channel's
//! `command: true` sends) is the temporary Move tool, as `docs/control-protocol.md` already
//! promises for a non-selection tool and as it already is from the keyboard: the drag moves the
//! layer as one "Move" history step, the ruler stays as it was, and the tool stays the Ruler.
//! The Count and Note tools do what the keyboard does for them (the issue's last "Done when" item):
//! a Ctrl-drag is the Move tool there too. The Color Sampler is left to #11731.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use photocraft_engine::doc::Ruler;

/// Ctrl as a Windows or Linux press sends it, which is also what the control channel builds for
/// `ui.pointer {"command": true}` and `{"ctrl": true}` (`control::params::modifiers_from`).
const CTRL: Modifiers = Modifiers { ctrl: true, command: true, shift: false, alt: false, mac_cmd: false };

/// A 400 Ã— 300 document with a red layer covering (100,100)..(160,160), the Ruler from 60,60 to
/// 340,240, and the Ruler tool active. Snapping, smart guides and the Move tool's Auto-Select are
/// off so the drag offsets are exact.
fn ruler_app() -> PhotocraftApp {
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

/// What a Ctrl+drag with the Ruler does: the layer moves, as one "Move" step; the ruler, the tool
/// and the Ruler tool's own drag state stay untouched.
fn check_moves_the_layer(app: &mut PhotocraftApp, id: photocraft_doc::LayerId, steps: usize, before: Option<Ruler>) {
    assert_eq!(bounds(app, id), Rect::new(150, 130, 210, 190), "the layer moved +50,+30");
    assert_eq!(ruler(app), before, "the ruler stays");
    assert_eq!(history(app), steps + 1, "one history step");
    assert_eq!(label(app).as_deref(), Some("Move"), "the Move tool's history step");
    assert_eq!(app.ui.tool, Tool::Ruler, "the tool stays the Ruler");
    assert!(app.ui.analysis.drag.is_none(), "the Ruler tool never started its own drag");
}

#[test]
fn a_ctrl_drag_with_the_ruler_is_the_move_tool() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    drag(&mut app, [120.0, 120.0], [170.0, 150.0], CTRL);
    check_moves_the_layer(&mut app, id, steps, before);
}

/// The reported route: `ui.pointer` with `command: true`, as docs/control-protocol.md documents it.
#[test]
fn the_control_channels_command_true_with_the_ruler_is_the_move_tool() {
    use crate::control::{ControlRequest, Outcome, handle};
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    let ctx = egui::Context::default();
    let params = json!({"command": true, "events": [{"kind": "down", "x": 120.0, "y": 120.0}, {"kind": "move", "x": 145.0, "y": 135.0}, {"kind": "up", "x": 170.0, "y": 150.0}]});
    let (request, _rx) = ControlRequest::new("ui.pointer", params);
    let Outcome::Done(_) = handle(&mut app, &ctx, &request) else { panic!("ui.pointer should reply immediately") };
    check_moves_the_layer(&mut app, id, steps, before);
}

/// The keyboard route, which already worked: the canvas sets `tool_override` for the held âŒ˜ before
/// the event reaches the tools.
#[test]
fn a_held_ctrl_key_with_the_ruler_is_the_move_tool_too() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    app.tool_override = Some(Tool::Move);
    drag(&mut app, [120.0, 120.0], [170.0, 150.0], Modifiers::NONE);
    app.tool_override = None;
    check_moves_the_layer(&mut app, id, steps, before);
}

/// The Ruler tool's own drag, with no modifier held, still draws the ruler and leaves the layer.
#[test]
fn a_drag_without_ctrl_still_draws_the_ruler() {
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let steps = history(&app);
    drag(&mut app, [120.0, 120.0], [170.0, 150.0], Modifiers::NONE);
    assert_eq!(ruler(&app), Some(Ruler { start: [120.0, 120.0], end: [170.0, 150.0], protractor: None }));
    assert_eq!(bounds(&app, id), Rect::new(100, 100, 160, 160), "the layer stays");
    assert_eq!(history(&app), steps, "a ruler is no history step");
}

/// A Move drag that started with Ctrl keeps the Move tool for the rest of the gesture: a release
/// without the modifier (the key let go before the button) still ends the drag as a Move.
#[test]
fn a_ctrl_drag_released_without_ctrl_still_finishes_as_the_move_tool() {
    use crate::control::{ControlRequest, Outcome, handle};
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Move { x: 145.0, y: 135.0, pressure: 1.0 }, CTRL);
    tool_event(&mut app, ToolEvent::Up { x: 170.0, y: 150.0 }, Modifiers::NONE);
    assert!(app.drag.is_none(), "the Move drag ended on the release");
    check_moves_the_layer(&mut app, id, steps, before);

    // The same gesture over the control channel: Ctrl on the first events only.
    let mut app = ruler_app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let (steps, before) = (history(&app), ruler(&app));
    let ctx = egui::Context::default();
    let down = json!({"command": true, "events": [{"kind": "down", "x": 120.0, "y": 120.0}, {"kind": "move", "x": 145.0, "y": 135.0}]});
    let up = json!({"events": [{"kind": "up", "x": 170.0, "y": 150.0}]});
    for params in [down, up] {
        let (request, _rx) = ControlRequest::new("ui.pointer", params);
        let Outcome::Done(_) = handle(&mut app, &ctx, &request) else { panic!("ui.pointer should reply immediately") };
    }
    assert!(app.drag.is_none(), "the Move drag ended on the release");
    check_moves_the_layer(&mut app, id, steps, before);
}

/// Count and Note: with Ctrl on the event the control route does what the keyboard route does for
/// them, which is the temporary Move tool: the layer moves as one "Move" step, and no marker or
/// note is created.
#[test]
fn ctrl_with_the_count_and_note_tools_is_the_move_tool_as_from_the_keyboard() {
    for tool in [Tool::Count, Tool::Note] {
        // The keyboard route, which already worked.
        let mut keyboard = ruler_app();
        keyboard.ui.tool = tool;
        let id = keyboard.session.active().unwrap().active_layer.unwrap();
        keyboard.tool_override = Some(Tool::Move);
        drag(&mut keyboard, [120.0, 120.0], [170.0, 150.0], Modifiers::NONE);
        keyboard.tool_override = None;
        assert_eq!(bounds(&keyboard, id), Rect::new(150, 130, 210, 190), "{tool:?}: the keyboard moves the layer");

        // The control route, now the same.
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
