//! Canvas gesture tests for the linked Pattern Fill drag: the dialog tool under the hosted dialog, the release
//! position, previews and the commit they match. They drive the canvas through `canvas::testing`.

use craft::draw::ui::pattern::PatternFillDialog;
use craft::ui::dialog::StackDialog;
use egui::Modifiers;
use photocraft_doc::{Document, Fill, LayerContent, LayerId, Pattern};
use serde_json::json;

use super::document_display::CanvasHost;
use super::testing::{clear_of_dialogs, ctx, frame, move_to, press, release, view};
use super::tools::ToolScope;
use super::tools::dialog::{DialogTools, dialog_tool};
use crate::PhotocraftApp;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// The phase the hosted dialog shows.
fn phase(app: &mut PhotocraftApp) -> [f64; 2] {
    app.framework.dialogs.get::<PatternFillDialog>().expect("dialog").current().phase
}

/// Asserts two phases agree within float noise.
fn assert_phase(got: [f64; 2], want: [f64; 2], why: &str) {
    assert!((got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3, "{why}: {got:?} != {want:?}");
}

/// A point `d` document units from `p`.
fn off(p: [f64; 2], d: [f64; 2]) -> [f64; 2] {
    [p[0] + d[0], p[1] + d[1]]
}

/// A document with a Pattern Fill dialog open: editing an existing linked fill, or creating one.
fn pattern_drag_app(depth: u64, create: bool) -> (PhotocraftApp, craft::ui::dialog::DialogId, Option<u64>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    let layer = if create {
        None
    } else {
        let id = app
            .run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard", "phase": [2.5, -3.25], "scale": 137.5, "angle": 45}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        Some(id)
    };
    let dialog = if create { crate::pattern_fill_ui::open_new(&mut app).unwrap() } else { crate::pattern_fill_ui::open(&mut app).unwrap() };
    frame(&mut app);
    clear_of_dialogs(&mut app, [20.0, 15.0]);
    (app, dialog, layer)
}

/// Ends the dialog with OK on the context the canvas frames run on.
fn confirm(app: &mut PhotocraftApp, dialog: craft::ui::dialog::DialogId) {
    let ctx = ctx(app);
    crate::dialogs::confirm(app, &ctx, dialog).unwrap();
}

/// Ends the dialog with Cancel on the context the canvas frames run on.
fn cancel(app: &mut PhotocraftApp, dialog: craft::ui::dialog::DialogId) {
    let ctx = ctx(app);
    crate::dialogs::cancel(app, &ctx, dialog).unwrap();
}

/// Sets the dialog's Link with Layer field as its JSON field setter does.
fn set_link(app: &mut PhotocraftApp, on: bool) {
    let ctx = ctx(app);
    app.framework.dialogs.get_mut::<PatternFillDialog>().expect("dialog").set_json_field(&ctx, "link", json!(on)).unwrap();
}

/// One press, moves and release of a drag from `start` by `delta` document units.
fn drag(app: &mut PhotocraftApp, start: [f64; 2], delta: [f64; 2]) {
    move_to(app, start, NONE);
    press(app, start, NONE);
    move_to(app, off(start, delta), NONE);
    release(app, off(start, delta), NONE);
}

#[test]
fn a_linked_pattern_fill_drag_moves_in_document_space_at_any_view() {
    let cases = [(0.25f32, 0.0f32, false), (1.0, 0.0, false), (4.0, 0.0, false), (1.0, 30.0, true)];
    for create in [false, true] {
        for depth in [8, 16, 32] {
            for (zoom, rotation, flip) in cases {
                let (mut app, dialog, layer) = pattern_drag_app(depth, create);
                // The pattern drag must be independent of all three.
                app.ui.views[0].zoom = zoom;
                app.ui.views[0].rotation = rotation;
                app.ui.view.flip_horizontal = flip;
                clear_of_dialogs(&mut app, [20.0, 15.0]);
                let before = app.session.active().unwrap().doc.clone();
                let count = app.session.active().unwrap().history.entries().len();
                let start = [20.0, 15.0];
                let delta = [7.0, -4.0];
                let expected = if create { [7.0, -4.0] } else { [9.5, -7.25] };
                let why = format!("create {create}, depth {depth}, {zoom}x, {rotation}°, flip {flip}");
                move_to(&mut app, start, NONE);
                press(&mut app, start, NONE);
                for f in [0.3, 0.7, 1.0] {
                    move_to(&mut app, off(start, [delta[0] * f, delta[1] * f]), NONE);
                    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{why}: previewing does not edit");
                }
                release(&mut app, off(start, delta), NONE);
                assert_phase(phase(&mut app), expected, &why);
                let (shown, _) = crate::pattern_fill_ui::display_doc(&mut app, 0).unwrap();
                let shown_phase = |doc: &Document, id: LayerId| {
                    let LayerContent::Fill(Fill::Pattern { phase, .. }) = &doc.layer(id).unwrap().content else { panic!() };
                    (phase.0, phase.1)
                };
                let shown_id = layer.map_or_else(|| shown.layers.last().unwrap().id, LayerId);
                let (px, py) = shown_phase(&shown, shown_id);
                assert!((px - expected[0] as f32).abs() < 1e-3 && (py - expected[1] as f32).abs() < 1e-3, "{why}: the preview shows the drag");
                if layer.is_none() {
                    assert_eq!(shown.layers.len(), before.layers.len() + 1, "the preview shows the new layer");
                }
                confirm(&mut app, dialog);
                let committed = app.session.active().unwrap().doc.clone();
                let committed_id = layer.map_or_else(|| committed.layers.last().unwrap().id, LayerId);
                if layer.is_some() {
                    assert_eq!(committed.layer(committed_id).unwrap().content, shown.layer(shown_id).unwrap().content, "OK commits the preview");
                    let LayerContent::Fill(Fill::Pattern { scale, angle, .. }) = &committed.layer(committed_id).unwrap().content else { panic!() };
                    assert!((*scale - 1.375).abs() < 1e-6 && (*angle - 45.0).abs() < 1e-6, "the drag changed only the phase");
                } else {
                    assert_eq!(committed.layers.len(), before.layers.len() + 1);
                }
                let (cx, cy) = shown_phase(&committed, committed_id);
                assert!((cx - expected[0] as f32).abs() < 1e-3 && (cy - expected[1] as f32).abs() < 1e-3);
                assert_eq!(app.session.active().unwrap().history.entries().len(), count + 1, "one History step");
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(*app.session.active().unwrap().doc, *before, "undo restores the pre-drag document");
                app.run("edit.redo", json!({})).unwrap();
                assert_eq!(*app.session.active().unwrap().doc, *committed, "redo restores the dragged document");
            }
        }
    }
}

#[test]
fn a_pattern_drag_back_or_a_press_alone_is_no_history_step_and_cancel_discards() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let start = [20.0, 15.0];
    let delta = [5.0, 3.0];
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.entries().len();
    move_to(&mut app, start, NONE);
    press(&mut app, start, NONE);
    move_to(&mut app, off(start, delta), NONE);
    assert_phase(phase(&mut app), [7.5, -0.25], "the drag moved the pattern");
    move_to(&mut app, start, NONE);
    release(&mut app, start, NONE);
    assert_phase(phase(&mut app), [2.5, -3.25], "a drag back restores the phase");
    confirm(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().history.entries().len(), count, "dragging back is no History step");
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // A press alone changes nothing.
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    frame(&mut app);
    move_to(&mut app, start, NONE);
    press(&mut app, start, NONE);
    release(&mut app, start, NONE);
    assert_phase(phase(&mut app), [2.5, -3.25], "a click moves nothing");
    confirm(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().history.entries().len(), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // Cancel after a drag discards the preview.
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    frame(&mut app);
    drag(&mut app, start, delta);
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_some());
    cancel(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().history.entries().len(), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_none());
}

#[test]
fn a_pattern_drag_ends_on_a_nested_dialog_or_unlink_and_pan_wins() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let start = [20.0, 15.0];
    let step = |d: f64| off(start, [d, 0.0]);
    move_to(&mut app, start, NONE);
    press(&mut app, start, NONE);
    move_to(&mut app, step(5.0), NONE);
    assert_phase(phase(&mut app), [7.5, -3.25], "the drag began");
    release(&mut app, step(5.0), NONE);
    // A new press moves from the phase shown.
    drag(&mut app, start, [2.0, 0.0]);
    assert_phase(phase(&mut app), [9.5, -3.25], "a new press starts from the shown phase");
    // A dialog above takes the pointer; closing it under the same press does not restart the drag.
    move_to(&mut app, start, NONE);
    press(&mut app, start, NONE);
    move_to(&mut app, step(3.0), NONE);
    // Another kind of dialog: a second Pattern Fill dialog would replace the one under the drag.
    let nested = crate::dialogs::solid_fill_ui::open_new(&mut app).unwrap();
    frame(&mut app);
    move_to(&mut app, step(8.0), NONE);
    cancel(&mut app, nested);
    frame(&mut app);
    move_to(&mut app, step(12.0), NONE);
    release(&mut app, step(12.0), NONE);
    assert_phase(phase(&mut app), [12.5, -3.25], "a nested dialog stops the drag; closing it does not restart it");
    // Unlinking under the press ends the gesture; relinking does not restart it.
    move_to(&mut app, start, NONE);
    press(&mut app, start, NONE);
    move_to(&mut app, step(4.0), NONE);
    set_link(&mut app, false);
    move_to(&mut app, step(8.0), NONE);
    set_link(&mut app, true);
    move_to(&mut app, step(12.0), NONE);
    release(&mut app, step(12.0), NONE);
    assert_phase(phase(&mut app), [16.5, -3.25], "unlinking stops the drag; relinking does not restart it");
    // With the Hand tool a linked drag moves the pattern, not the view.
    app.ui.tool = Tool::Hand;
    let center = view(&app).unwrap().center;
    let before = phase(&mut app);
    drag(&mut app, start, [5.0, 0.0]);
    assert_phase(phase(&mut app), [before[0] + 5.0, before[1]], "the Hand tool moves the pattern");
    assert_eq!(view(&app).unwrap().center, center, "the Hand tool does not pan");
    // Space pans through the host's momentary Hand; the pattern stays.
    let before = phase(&mut app);
    let window = craft::ui::runtime::windows::WindowHost::of(&ctx(&app));
    window.inject_key(egui::ViewportId::ROOT, craft::keymap::Key::Space, craft::keymap::Mods::default(), true);
    frame(&mut app);
    drag(&mut app, start, [6.0, 0.0]);
    window.end_injection();
    assert_phase(phase(&mut app), before, "Space pans instead");
    assert_ne!(view(&app).unwrap().center, center, "the view moved");
    cancel(&mut app, dialog);
}

#[test]
fn a_pattern_drag_previews_without_editing_masks_selection_or_the_tool() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": 16})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 20, "height": 10})).unwrap();
    let id = app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap();
    // A mask on the non-active Background must not change.
    let background = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    app.run("layer.layerMask.fromTransparency", json!({})).unwrap();
    app.run("layer.select", json!({"layer": id})).unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 8})).unwrap();
    app.ui.tool = Tool::Brush;
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    frame(&mut app);
    clear_of_dialogs(&mut app, [20.0, 15.0]);
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.entries().len();
    let revision = app.session.active().unwrap().revision;
    let active_layer = app.session.active().unwrap().active_layer;
    let fill_mask = before.layer(LayerId(id)).unwrap().mask.clone();
    let background_mask = before.layer(background).unwrap().mask.clone();
    let selection = before.selection.clone();
    drag(&mut app, [20.0, 15.0], [4.0, 6.0]);
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "previewing does not edit the document");
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.entries().len(), count);
    assert_eq!(st.doc.layer(LayerId(id)).unwrap().mask, fill_mask);
    assert_eq!(st.doc.layer(background).unwrap().mask, background_mask);
    assert_eq!(st.doc.selection, selection);
    assert_eq!(st.active_layer, active_layer);
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_some());
    confirm(&mut app, dialog);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries().len(), count + 1, "OK is one History step");
    assert_eq!(st.doc.layer(LayerId(id)).unwrap().mask, fill_mask);
    assert_eq!(st.doc.layer(background).unwrap().mask, background_mask);
    assert_eq!(st.doc.selection, selection);
    assert_eq!(st.active_layer, active_layer);
    assert_eq!(app.ui.tool, Tool::Brush);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().mask, fill_mask);
}

#[test]
fn the_dialog_tool_serves_only_the_active_documents_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard", "phase": [2.5, -3.25]})).unwrap();
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    // A second document in its own cell, the fill's document active.
    app.run("file.new", json!({"width": 30, "height": 20})).unwrap();
    assert!(app.session.set_active(0));
    frame(&mut app);
    let active = app.session.active_index().unwrap();
    let scope = ToolScope::for_test(&mut app);
    let mut state = DialogTools::default();
    assert!(dialog_tool(&scope, &mut state, CanvasHost::main(active)).is_some(), "the active document's canvas runs the dialog tool");
    let mut state = DialogTools::default();
    assert!(dialog_tool(&scope, &mut state, CanvasHost::main(1 - active)).is_none(), "another document's canvas does not");
    cancel(&mut app, dialog);
}

#[test]
fn a_drag_moves_the_visible_pattern_not_only_its_phase() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 2, "depth": 8})).unwrap();
    // Our own 8 × 2 tile: a solid red 2 × 2 left edge, transparent elsewhere.
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[1.0, 0.0, 0.0, 1.0]);
    let tile = Pattern::new("Shift tile", surface, 8, 2);
    let pattern = tile.id.clone();
    app.session.patterns.items.push(tile);
    app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": pattern, "phase": [0.0, 0.0], "scale": 100, "angle": 0, "link": true})).unwrap();
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    frame(&mut app);
    clear_of_dialogs(&mut app, [4.0, 1.0]);
    let band = |doc: &Document| {
        let buf = photocraft_compose::render(doc, photocraft_geom::Rect::new(0, 0, 8, 1));
        (0..8).filter(|x| buf.px[*x as usize][0] > 0.5 && buf.px[*x as usize][1] < 0.5).collect::<Vec<i32>>()
    };
    let before = band(&app.session.active().unwrap().doc);
    assert_eq!(before, vec![0, 1], "the tile's red half starts at the origin");
    drag(&mut app, [4.0, 1.0], [2.0, 0.0]);
    let (shown, _) = crate::pattern_fill_ui::display_doc(&mut app, 0).unwrap();
    let moved = band(&shown);
    assert_eq!(moved, vec![2, 3], "the preview paints the red half two document pixels right");
    confirm(&mut app, dialog);
    let committed = band(&app.session.active().unwrap().doc);
    assert_eq!(committed, moved, "OK commits the pixels the preview showed");
    assert_ne!(committed, before, "the drag is visible, not only a phase field");
}
