//! #16815: show the system cursor for painting tools on the blocked layer targets measured in the issue.

use craft::behavior::{Cursor, CursorIcon};
use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::dialogs::{alerts, rasterize_prompt::Rasterize};
use crate::guide_cursor_tests::{harness, hover, screen};
use crate::state::Tool;

const IN_IMAGE: f32 = 100.0;
const ON_PASTEBOARD: f32 = -40.0;
const TOOLS: [Tool; 3] = [Tool::Brush, Tool::Eraser, Tool::PaintBucket];
const BLOCKED: Option<Cursor> = Some(Cursor::System(CursorIcon::NotAllowed));

fn pixel_layer() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(Tool::Brush);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    h
}

fn cursor_at(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, x: f32, mods: Modifiers) -> Option<Cursor> {
    h.state_mut().ui.tool = tool;
    hover(h, x, 150.0, mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|response| response.cursor.clone())
}

fn image_cursor(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool) -> Option<Cursor> {
    cursor_at(h, tool, IN_IMAGE, Modifiers::NONE)
}

fn click_image(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let pos = screen(h, IN_IMAGE, 150.0);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

#[test]
fn lock_all_shows_the_blocked_cursor_on_the_image_and_pasteboard() {
    let mut h = pixel_layer();
    let normal = TOOLS.map(|tool| image_cursor(&mut h, tool));
    for (tool, cursor) in TOOLS.into_iter().zip(&normal) {
        assert!(cursor.is_some() && *cursor != BLOCKED, "{tool:?} has a normal cursor before locking");
    }
    let shift_bucket = cursor_at(&mut h, Tool::PaintBucket, ON_PASTEBOARD, Modifiers::SHIFT);
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    for tool in TOOLS {
        for x in [IN_IMAGE, ON_PASTEBOARD] {
            assert_eq!(cursor_at(&mut h, tool, x, Modifiers::NONE), BLOCKED, "{tool:?} at x={x}");
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::NotAllowed);
        }
    }
    assert_eq!(cursor_at(&mut h, Tool::PaintBucket, ON_PASTEBOARD, Modifiers::SHIFT), shift_bucket, "Shift changes the pasteboard colour");
    h.state_mut().run("layer.lockLayers", json!({"all": false})).unwrap();
    assert_eq!(TOOLS.map(|tool| image_cursor(&mut h, tool)), normal);
}

#[test]
fn a_type_layer_blocks_brush_and_bucket_and_their_clicks_still_ask_to_rasterize() {
    for tool in [Tool::Brush, Tool::PaintBucket] {
        let mut h = pixel_layer();
        h.state_mut().run("type.create", json!({"x": 40, "y": 60, "text": "Type"})).unwrap();
        let cursor = image_cursor(&mut h, tool);
        let before = h.state().session.active().unwrap().history.past_len();
        click_image(&mut h);
        assert!(alerts::prompt_tagged::<Rasterize>(h.state()).is_some(), "{tool:?}: Rasterize prompt");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), before, "{tool:?}");
        assert_eq!(cursor, BLOCKED, "{tool:?}");
    }
    let mut h = pixel_layer();
    let eraser = image_cursor(&mut h, Tool::Eraser);
    h.state_mut().run("type.create", json!({"x": 40, "y": 60, "text": "Type"})).unwrap();
    assert_eq!(image_cursor(&mut h, Tool::Eraser), eraser);
}

#[test]
fn locked_pixel_clicks_keep_their_refusal_and_history() {
    for tool in TOOLS {
        let mut h = pixel_layer();
        h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
        image_cursor(&mut h, tool);
        let before = h.state().session.active().unwrap().history.past_len();
        click_image(&mut h);
        let app = h.state();
        let dialogs = app.framework.dialogs.open_dialogs();
        let alert = dialogs.iter().any(|dialog| {
            dialog
                .fields
                .as_ref()
                .and_then(|fields| fields.get("message"))
                .and_then(|message| message.as_str())
                .is_some_and(|message| message.contains("layer is locked"))
        });
        assert!(alert || (app.ui.status_error && app.ui.status.contains("locked")), "{tool:?}: {}", app.ui.status);
        assert_eq!(app.session.active().unwrap().history.past_len(), before, "{tool:?}");
    }
}

#[test]
fn hidden_layers_partial_locks_and_groups_keep_each_tools_normal_cursor() {
    let mut h = pixel_layer();
    let normal = TOOLS.map(|tool| image_cursor(&mut h, tool));
    for lock in ["pixels", "transparency", "position"] {
        h.state_mut().run("layer.lockLayers", json!({ (lock): true })).unwrap();
        assert_eq!(TOOLS.map(|tool| image_cursor(&mut h, tool)), normal, "{lock}");
        h.state_mut().run("layer.lockLayers", json!({ (lock): false })).unwrap();
    }
    // Test hiding a fully locked layer, so the hidden and locked rules compete.
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    h.state_mut().run("layer.setProps", json!({"visible": false})).unwrap();
    assert_eq!(TOOLS.map(|tool| image_cursor(&mut h, tool)), normal, "hidden Lock All layer");

    h.state_mut().run("layer.new.group", json!({})).unwrap();
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    assert_eq!(TOOLS.map(|tool| image_cursor(&mut h, tool)), normal, "locked group itself");
}

#[test]
fn locked_layers_keep_nonpainting_cursors_and_the_stand_ins() {
    let mut h = pixel_layer();
    let tools = [Tool::RectMarquee, Tool::Move, Tool::Pencil];
    let expected = tools.map(|tool| image_cursor(&mut h, tool));
    let eyedropper = image_cursor(&mut h, Tool::Eyedropper);
    let quick_mask = image_cursor(&mut h, Tool::Brush);
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    assert_eq!(tools.map(|tool| image_cursor(&mut h, tool)), expected);
    assert_eq!(image_cursor(&mut h, Tool::MixerBrush), BLOCKED);
    assert_eq!(cursor_at(&mut h, Tool::Brush, IN_IMAGE, Modifiers::ALT), eyedropper);
    assert_eq!(cursor_at(&mut h, Tool::Brush, IN_IMAGE, Modifiers::COMMAND), expected[1]);

    h.state_mut().run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert_eq!(image_cursor(&mut h, Tool::Brush), quick_mask);
}
