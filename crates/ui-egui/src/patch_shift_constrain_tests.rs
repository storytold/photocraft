//! #12129: ⇧ pressed during a Patch drag (after the press) keeps the drag horizontal or vertical.
//! Only 0 and 90 degrees snap; ⇧ at the press still adds to the selection.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 500 x 400 document with 100..200 square selected, the Patch tool active.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 500, "height": 400, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 100, "mode": "replace"})).unwrap();
    app.ui.tool = Tool::Patch;
    app.ui.tool_options.patch_content_aware = false;
    app
}

fn mv(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}

/// Press at 140,140 without ⇧, move past the first 60 px, then to `to` with `m` held. Returns the press as
/// captured at the Down, the last point and whether ⇧ is held there.
fn drag_to(app: &mut PhotocraftApp, to: [f64; 2], m: Modifiers) -> (crate::retouch_ui::PatchPress, [f64; 2], bool) {
    let press = crate::retouch_ui::PatchPress::new(Tool::Patch, [140.0, 140.0], craft::keymap::Mods::default());
    tool_event(app, ToolEvent::Down { x: 140.0, y: 140.0, pressure: 1.0 }, Modifiers::NONE);
    mv(app, 200.0, 145.0, Modifiers::NONE);
    mv(app, to[0], to[1], m);
    (press, to, m.shift)
}

fn patch_offsets(app: &PhotocraftApp) -> Vec<serde_json::Value> {
    app.session.journal.iter().filter(|(id, _)| id == "paint.patch").map(|(_, p)| p["offset"].clone()).collect()
}

/// The offset the outline shows for the press `drag_to` made, ending at `last`.
fn outline_offset(app: &mut PhotocraftApp, (press, last, shift): (crate::retouch_ui::PatchPress, [f64; 2], bool)) -> [i32; 2] {
    let end = crate::retouch_ui::patch_end(&press, last, shift);
    crate::retouch_ui::patch_offset(app, press.start, end)
}

#[test]
fn shift_during_the_drag_snaps_the_offset_to_the_dominant_axis() {
    let mut app = app();
    drag_to(&mut app, [380.0, 170.0], Modifiers::SHIFT);
    assert_eq!(crate::patch_preview::drag_offset(&mut app), Some([240, 0]));

    let mut app = self::app();
    drag_to(&mut app, [380.0, 170.0], Modifiers::NONE);
    assert_eq!(crate::patch_preview::drag_offset(&mut app), Some([240, 30]), "without ⇧ the offset is free");

    // The dominant axis can be vertical too.
    let mut app = self::app();
    drag_to(&mut app, [150.0, 300.0], Modifiers::SHIFT);
    assert_eq!(crate::patch_preview::drag_offset(&mut app), Some([0, 160]));
}

#[test]
fn the_outline_the_preview_and_the_applied_patch_use_the_snapped_offset() {
    let mut app = app();
    let drag = drag_to(&mut app, [380.0, 170.0], Modifiers::SHIFT);
    assert_eq!(outline_offset(&mut app, drag), [240, 0], "outline");
    assert_eq!(crate::patch_preview::drag_offset(&mut app), Some([240, 0]), "preview");
    tool_event(&mut app, ToolEvent::Up { x: 380.0, y: 170.0 }, Modifiers::SHIFT);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(patch_offsets(&app), vec![json!([240, 0])], "applied patch");

    let mut app = self::app();
    let drag = drag_to(&mut app, [380.0, 170.0], Modifiers::NONE);
    assert_eq!(outline_offset(&mut app, drag), [240, 30]);
    tool_event(&mut app, ToolEvent::Up { x: 380.0, y: 170.0 }, Modifiers::NONE);
    assert_eq!(patch_offsets(&app), vec![json!([240, 30])]);
}

#[test]
fn shift_at_mouse_down_still_adds_to_the_selection() {
    let mut app = app();
    let m = Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 140.0, y: 140.0, pressure: 1.0 }, m);
    for [x, y] in [[300.0, 140.0], [340.0, 140.0], [340.0, 180.0], [300.0, 180.0]] {
        mv(&mut app, x, y, m);
    }
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 180.0 }, m);
    assert!(patch_offsets(&app).is_empty(), "no patch was applied");
    let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
    assert!(sel.sample_channel(150, 150, 0) > 0.0, "the old selection is kept");
    assert!(sel.sample_channel(320, 160, 0) > 0.0, "the lasso is added");
}

#[test]
fn only_horizontal_and_vertical_snap_never_forty_five_degrees() {
    // A perfect diagonal still goes to one axis.
    let mut app = app();
    drag_to(&mut app, [240.0, 240.0], Modifiers::SHIFT);
    let off = crate::patch_preview::drag_offset(&mut app).unwrap();
    assert!(off[0] == 0 || off[1] == 0, "{off:?} is not on an axis");
    // A 160 x 120 drag is not pulled to 45 degrees (140 x 140).
    let mut app = self::app();
    drag_to(&mut app, [300.0, 260.0], Modifiers::SHIFT);
    assert_eq!(crate::patch_preview::drag_offset(&mut app), Some([160, 0]));
}
