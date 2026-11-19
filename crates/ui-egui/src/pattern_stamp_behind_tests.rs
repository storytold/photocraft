//! #12879: the Pattern Stamp's Mode list has Behind right after Dissolve (and no Clear); greyed on
//! the Background. Picking it makes the stamp paint only where the layer is transparent; picking
//! another mode leaves it. The Clone Stamp's and the History Brush's lists are not changed here.

use craft::keymap::Mods;
use egui::{Context, Event, Key, Modifiers, RawInput};
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use super::{Item, choose, current, enabled, engine_call, items};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A 64 × 64 document with the Pattern Stamp; with `layer`, a layer above the Background whose
/// left half is red and whose right half is transparent.
fn app(layer: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    if layer {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
        app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    app.ui.tool = Tool::PatternStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.sync_views();
    app
}

/// Shift+Alt+`key` through the shortcut handler.
fn key(app: &mut PhotocraftApp, ctx: &Context, key: Key) {
    let modifiers = Modifiers::SHIFT | Modifiers::ALT;
    let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }];
    ctx.begin_pass(RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// The last journaled command.
fn last_call(app: &PhotocraftApp) -> (String, Value) {
    app.session.journal.last().cloned().unwrap()
}

/// The active layer's pixel as straight RGBA.
fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// One Pattern Stamp stroke across both halves of the layer, as a drag commits it.
fn stamp(app: &mut PhotocraftApp) {
    assert!(crate::retouch_ui::finish_stroke(app, Tool::PatternStamp, &[[12.0, 32.0, 1.0], [52.0, 32.0, 1.0]], Mods::default(), false));
}

/// Behind belongs to the Pattern Stamp alone (the Clone Stamp #12602 and the History Brush #12849
/// are separate items), and the Background has no transparency to paint behind: the row is greyed,
/// choosing it and its key do nothing.
#[test]
fn behind_is_offered_only_to_the_pattern_stamp_and_greyed_on_the_background() {
    // (tool, on a pixel layer, listed, engine call, enabled where it is offered)
    let table = [
        (Tool::PatternStamp, true, true, true, Some(true)),
        (Tool::PatternStamp, false, true, true, Some(false)),
        (Tool::CloneStamp, true, false, false, None),
        (Tool::HistoryBrush, true, false, false, None),
        (Tool::Healing, true, false, false, None),
        (Tool::Gradient, true, false, false, None),
        (Tool::PaintBucket, true, false, false, None),
    ];
    for (tool, layer, listed, called, on) in table {
        let mut app = app(layer);
        app.ui.tool = tool;
        crate::paint_mouse::sync_tool_brush(&mut app);
        assert_eq!(items(tool).contains(&Item::Behind), listed, "{tool:?} listed");
        assert_eq!(engine_call(&app, tool, Item::Behind).is_some(), called, "{tool:?} engine call");
        let Some(on) = on else { continue };
        assert_eq!(enabled(&app, Item::Behind), on, "{tool:?} on a layer: {layer}");
        if !on {
            let ctx = app.ctx().clone();
            let n = app.session.journal.len();
            choose(&mut app, Item::Behind);
            key(&mut app, &ctx, Key::Q);
            assert_eq!(app.session.journal.len(), n, "a greyed row runs nothing");
            assert!(!app.session.tools.brush.behind);
            assert!(enabled(&app, Item::Blend(BlendMode::Multiply)));
        }
    }
}

/// Behind lives in the Pattern Stamp's own brush, through `tools.setBrush`.
#[test]
fn choosing_behind_sets_it_on_the_tools_brush_and_another_mode_leaves_it() {
    let mut app = app(true);
    assert!(enabled(&app, Item::Behind));
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Blend(BlendMode::Normal)));
    choose(&mut app, Item::Behind);
    assert!(app.session.tools.brush.behind);
    assert_eq!(last_call(&app), ("tools.setBrush".into(), json!({"brush": {"mode": "Normal", "behind": true}})));
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Behind), "the checked row");
    assert_eq!(crate::retouch_ui::pattern_stamp_params(&app, &[[1.0, 1.0, 1.0]])["mode"], "Behind");
    // Another mode leaves Behind, in the same one command.
    let n = app.session.journal.len();
    choose(&mut app, Item::Blend(BlendMode::Multiply));
    assert_eq!(app.session.journal.len(), n + 1);
    assert_eq!(last_call(&app), ("tools.setBrush".into(), json!({"brush": {"mode": "Multiply", "behind": false}})));
    assert!(!app.session.tools.brush.behind);
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Blend(BlendMode::Multiply)));
    assert_eq!(crate::retouch_ui::pattern_stamp_params(&app, &[[1.0, 1.0, 1.0]])["mode"], "Multiply");
    // Behind is the Pattern Stamp's own: the Brush keeps its mode.
    choose(&mut app, Item::Behind);
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert!(!app.session.tools.brush.behind, "the Brush's own brush");
    app.ui.tool = Tool::PatternStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert!(app.session.tools.brush.behind, "kept with the Pattern Stamp");
}

/// A stroke committed from the tool, Aligned on and off.
#[test]
fn a_behind_stroke_paints_only_the_transparent_half() {
    for aligned in [true, false] {
        let mut app = app(true);
        app.ui.tool_options.pattern_stamp_aligned = aligned;
        app.run("tools.setBrush", json!({"brush": {"size": 16.0, "hardness": 1.0}})).unwrap();
        let red = pixel(&app, 16, 32);
        assert_eq!(pixel(&app, 48, 32)[3], 0.0);
        choose(&mut app, Item::Behind);
        let steps = app.session.active().unwrap().history.past_len();
        stamp(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "aligned {aligned}: one History step");
        assert!((0..32).all(|x| pixel(&app, x, 32) == red), "aligned {aligned}: the existing pixels are unchanged");
        assert!(pixel(&app, 48, 32)[3] > 0.99, "aligned {aligned}: the pattern fills the transparency");
        // In Normal the same stroke paints over the red.
        choose(&mut app, Item::Blend(BlendMode::Normal));
        stamp(&mut app);
        assert_ne!(pixel(&app, 16, 32), red, "aligned {aligned}: Normal paints over");
    }
}
