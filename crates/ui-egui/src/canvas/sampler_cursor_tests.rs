//! #11744: the Color Sampler's cursors, as in Photoshop. Off a sampler the pipette with the
//! sampler target (the crosshair for Precise Other Cursors), over one the arrow with the target,
//! and with ⌥ over one the arrow with scissors. "Over" is the tool's own hit test.

use super::cursors::{SamplerCursor, sampler_cursor, sampler_cursor_kind};
use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;
use serde_json::json;

/// A 200 × 100 document with the Color Sampler tool and one point on pixel 50,40.
fn app() -> (PhotocraftApp, [f64; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 40})).unwrap();
    let at = app.session.active().unwrap().color_samplers[0];
    (app, at)
}

fn precise(app: &mut PhotocraftApp) {
    app.run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
}

#[test]
fn off_a_sampler_it_is_the_pipette_also_with_shift_or_alt() {
    let (app, _) = app();
    // Shift is not an input of the cursor: only ⌥ is.
    for alt in [false, true] {
        assert_eq!(sampler_cursor_kind(&app, [150.0, 80.0], alt), SamplerCursor::Pipette, "alt {alt}");
    }
}

#[test]
fn over_a_sampler_it_is_the_arrow_and_with_alt_the_scissors() {
    let (app, at) = app();
    assert_eq!(sampler_cursor_kind(&app, at, false), SamplerCursor::Move);
    assert_eq!(sampler_cursor_kind(&app, at, true), SamplerCursor::Delete);
    // Releasing ⌥ without moving the pointer goes back to the arrow with the target.
    assert_eq!(sampler_cursor_kind(&app, at, false), SamplerCursor::Move);
}

#[test]
fn precise_other_cursors_keep_the_crosshair_off_a_sampler_only() {
    let (mut app, at) = app();
    precise(&mut app);
    assert_eq!(sampler_cursor_kind(&app, [150.0, 80.0], false), SamplerCursor::Crosshair);
    assert_eq!(sampler_cursor_kind(&app, [150.0, 80.0], true), SamplerCursor::Crosshair);
    assert_eq!(sampler_cursor_kind(&app, at, false), SamplerCursor::Move);
    assert_eq!(sampler_cursor_kind(&app, at, true), SamplerCursor::Delete);
}

#[test]
fn caps_lock_keeps_todays_cursor_everywhere() {
    // Not measured: ⇪ toggles the precise crosshair as before, over a sampler too.
    let (mut app, at) = app();
    app.caps_lock = true;
    assert_eq!(sampler_cursor_kind(&app, at, false), SamplerCursor::Crosshair);
    assert_eq!(sampler_cursor_kind(&app, [150.0, 80.0], false), SamplerCursor::Crosshair);
    precise(&mut app);
    assert_eq!(sampler_cursor_kind(&app, at, false), SamplerCursor::Pipette, "⇪ turns Precise back to the pipette");
}

#[test]
fn the_cursor_changes_where_a_drag_starts_moving_the_sampler() {
    // Walk away from the point: wherever the cursor is the arrow a drag moves the point, and
    // wherever it is the pipette the press grabs nothing.
    for dx in [0.0, 2.0, 4.0, 6.0, 6.9, 7.1, 8.0, 12.0] {
        let (mut app, at) = app();
        let press = [at[0] + dx, at[1]];
        let kind = sampler_cursor_kind(&app, press, false);
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: press[0], y: press[1], pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 80.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 80.0 }, egui::Modifiers::NONE);
        let moved = app.session.active().unwrap().color_samplers[0] != at;
        assert_eq!(kind == SamplerCursor::Move, moved, "{dx} px off: cursor {kind:?}, moved {moved}");
        assert!(matches!(kind, SamplerCursor::Move | SamplerCursor::Pipette), "{dx}: {kind:?}");
    }
}

#[test]
fn each_kind_sets_its_os_cursor() {
    // The pipette is drawn (OS cursor hidden, tip on the pixel); the arrows keep the OS arrow,
    // whose hotspot is its tip, and draw a badge beside it.
    let ctx = egui::Context::default();
    let p = egui::pos2(100.0, 60.0);
    let mut got = Vec::new();
    let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
        for kind in [SamplerCursor::Pipette, SamplerCursor::Crosshair, SamplerCursor::Move, SamplerCursor::Delete] {
            got.push((kind, sampler_cursor(ui.ctx(), kind, p)));
        }
    });
    o.textures_delta.clear();
    assert_eq!(
        got,
        vec![
            (SamplerCursor::Pipette, egui::CursorIcon::None),
            (SamplerCursor::Crosshair, egui::CursorIcon::Crosshair),
            (SamplerCursor::Move, egui::CursorIcon::Default),
            (SamplerCursor::Delete, egui::CursorIcon::Default),
        ]
    );
}
