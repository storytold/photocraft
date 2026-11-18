//! Marquee behaviour through the real canvas: the Object Selection tool keeps the mode of its press, selected
//! pixels float and drop with the history steps Photoshop names, a moved selection snaps to guides and the document
//! (not to its own start), the arrows nudge the outline, and the lock refusal of the Move tool and ⌘-drags.

use crate::app::framework::BuildApp as _;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    harness_with_step_dt(tool, 0.25)
}

fn harness_with_step_dt(tool: Tool, step_dt: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 700.0)).with_step_dt(step_dt).build_app(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    // 100 %: one document pixel per point, so drags land on whole pixels.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

/// Where the canvas shows document point `(x, y)`, through the view its last pass published.
fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

fn mods(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(1);
}

/// Move the pointer to document point `(x, y)` in a few steps.
fn move_to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
}

fn press_at(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    // Past egui's click distance, so the drag starts at the press point.
    move_to(h, x + 8.0, y + 6.0);
}

fn release_at(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    move_to(h, x, y);
    button(h, screen(h, x, y), false, m);
    h.run_steps(2);
}

fn selection(h: &Harness<'static, PhotocraftApp>) -> Rect {
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

#[test]
fn object_selection_keeps_the_mode_from_the_mouse_press() {
    for dragged in [false, true] {
        for held in [false, true] {
            for at_press in [Modifiers::SHIFT, Modifiers::ALT] {
                let mut h = harness_with_step_dt(Tool::ObjectSelection, 1.0 / 60.0);
                h.state_mut().ui.extras.snap = false;
                h.state_mut().run("paint.pencil", json!({"points": [[220, 150]], "size": 60, "color": "#ff0000"})).unwrap();
                let st = h.state().session.active().unwrap();
                let doc = st.doc.clone();
                let target = st.active_layer;
                let region = photocraft_engine::smartselect_cmds::find_object(&doc, target, false, 220.0, 150.0).expect("the red disc is detectable");
                h.state_mut().session.object_finder.insert(&doc, target, std::sync::Arc::new(region));
                if at_press.alt {
                    h.state_mut().run("select.all", json!({})).unwrap();
                } else {
                    h.state_mut().run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                }
                let start_doc = if dragged { [180.0, 110.0] } else { [220.0, 150.0] };
                mods(&mut h, at_press);
                move_to(&mut h, start_doc[0], start_doc[1]);
                let start = screen(&h, start_doc[0], start_doc[1]);
                button(&mut h, start, true, at_press);
                let during = if held { at_press } else { Modifiers::NONE };
                mods(&mut h, during);
                let end_doc = if dragged { [260.0, 190.0] } else { start_doc };
                release_at(&mut h, end_doc[0], end_doc[1], during);
                let app = h.state();
                assert!(!app.ui.status_error, "{}", app.ui.status);
                let (command, params) = app.session.journal.last().expect("the gesture dispatched a command");
                assert_eq!(command, "select.object", "the canvas dispatched Object Selection");
                let expected = if at_press.alt { "subtract" } else { "add" };
                assert_eq!(params["mode"], expected, "dragged={dragged}, held={held}");
                let sel = app.session.active().unwrap().doc.selection.as_ref().expect("the old selection survives outside the object");
                assert!(sel.sample_channel(20, 20, 0) > 0.99, "the gesture must preserve selection outside the object");
                assert_eq!(sel.sample_channel(220, 150, 0) > 0.5, !at_press.alt, "the disc follows the requested mode");
                // From no selection: a New-mode click inside the selection deselects (#12853).
                h.state_mut().run("select.deselect", json!({})).unwrap();
                mods(&mut h, Modifiers::NONE);
                move_to(&mut h, 220.0, 150.0);
                let next = screen(&h, 220.0, 150.0);
                button(&mut h, next, true, Modifiers::NONE);
                release_at(&mut h, 220.0, 150.0, Modifiers::NONE);
                let (_, params) = h.state().session.journal.last().unwrap();
                assert_eq!(params["mode"], "replace", "the next unmodified click must not reuse the previous mode");
                let sel = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
                assert!(sel.sample_channel(20, 20, 0) < 0.01 && sel.sample_channel(220, 150, 0) > 0.99);
            }
        }
    }
}

/// Press at `from` and move to `to` in six ordinary pointer moves, the button still held.
fn drag_to(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2]) {
    move_to(h, from[0], from[1]);
    button(h, screen(h, from[0], from[1]), true, Modifiers::NONE);
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        move_to(h, from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
    }
}

/// An 80×60 transparent document, red over (10..30)², that square selected, Rectangular Marquee.
fn painted() -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::RectMarquee;
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint", |doc, _| {
            doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    (app, layer)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: Modifiers) {
    use crate::canvas::{ToolEvent, tool_event};
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// ⌘-drag cuts the selected pixels into a floating piece (`select.float`): shown at the pointer
/// while dragging, moved again by ⌘-drags, put back by Undo, dropped by Deselect.
#[test]
fn cmd_drag_floats_the_selected_pixels() {
    use crate::canvas::{ToolEvent, tool_event};
    let (mut app, layer) = painted();
    let alpha = |d: &photocraft_doc::Document, x, y| d.layer(layer).unwrap().surface().unwrap().rgba(x, y)[3];
    let doc = |app: &PhotocraftApp| app.session.active().unwrap().doc.clone();
    let offset = |app: &PhotocraftApp| photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset);
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::COMMAND);
    tool_event(&mut app, ToolEvent::Move { x: 35.0, y: 20.0, pressure: 1.0 }, Modifiers::COMMAND);
    let (shown, _) = crate::move_ui::display_doc(&mut app, 0).expect("the piece shows at the pointer while dragging");
    assert!(alpha(&shown, 12, 20) == 0.0 && alpha(&shown, 40, 20) == 1.0);
    tool_event(&mut app, ToolEvent::Up { x: 35.0, y: 20.0 }, Modifiers::COMMAND);
    assert_eq!(offset(&app), Some((15, 0)));
    assert_eq!(alpha(&doc(&app), 12, 20), 1.0, "the document waits for the drop");
    // A ⌘-drag on the piece moves it again.
    drag(&mut app, [30.0, 20.0], [30.0, 30.0], Modifiers::COMMAND);
    assert_eq!(offset(&app), Some((15, 10)));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    // Undo puts it back.
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.undo", json!({})).unwrap();
    assert!(offset(&app).is_none() && alpha(&doc(&app), 12, 20) == 1.0);
    // Float again and deselect: dropped (one history step) where it was shown, then deselected.
    drag(&mut app, [20.0, 20.0], [35.0, 30.0], Modifiers::COMMAND);
    crate::menus::invoke(&mut app, &egui::Context::default(), "select.deselect", json!({})).unwrap();
    let d = doc(&app);
    assert!(d.selection.is_none() && offset(&app).is_none());
    assert!(alpha(&d, 12, 12) == 0.0 && alpha(&d, 26, 21) == 1.0 && alpha(&d, 44, 39) == 1.0);
    let labels: Vec<String> = app.session.active().unwrap().history.entries().into_iter().skip(steps + 1).map(|e| e.to_string()).collect();
    assert_eq!(labels.first().map(String::as_str), Some("Move"), "{labels:?}");
}

/// #11368: after the Move tool moves the selected pixels, a plain marquee drag inside the
/// selection moves only the outline, as in Photoshop: the pixels are dropped where they are (one
/// Move step), then the outline moves on its own (one Move Selection step).
#[test]
fn a_marquee_drag_after_moving_pixels_moves_only_the_outline() {
    let (mut app, layer) = painted();
    let alpha = |app: &PhotocraftApp, x, y| app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(x, y)[3];
    let sel = |app: &PhotocraftApp| app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    let floating = |app: &PhotocraftApp| photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_some();
    let steps = app.session.active().unwrap().history.past_len();
    app.ui.tool = Tool::Move;
    drag(&mut app, [20.0, 20.0], [35.0, 20.0], Modifiers::NONE);
    assert!(floating(&app), "the Move tool leaves the moved pixels floating");
    app.ui.tool = Tool::RectMarquee;
    drag(&mut app, [30.0, 20.0], [30.0, 30.0], Modifiers::NONE);
    assert!(!floating(&app), "the press dropped the piece");
    assert_eq!(sel(&app), Some(Rect::new(25, 20, 45, 40)), "the outline moved down");
    assert!(alpha(&app, 40, 15) == 1.0 && alpha(&app, 40, 35) == 0.0, "the pixels stay where the Move tool left them");
    assert_eq!(alpha(&app, 12, 20), 0.0, "the cut stays cut");
    let labels: Vec<String> = app.session.active().unwrap().history.entries().into_iter().skip(steps + 1).map(|e| e.to_string()).collect();
    assert_eq!(labels, ["Move", "Move Selection"]);
}

/// #6290: with a fresh app's View › Snap To, a selection moved through the real canvas snaps its
/// edges to a guide and the document bounds. Layers' targets were clipped to the selection, so
/// it snapped back to its own starting edges and centre (shown as layer alignments) instead.
#[test]
fn a_moved_selection_snaps_to_guides_and_the_document_not_its_start() {
    for (to, want) in [
        // Right edge 3 px left of the guide at x = 200; 3 px down stays put (not back to y = 52).
        ([145.0, 103.0], Rect::new(100, 55, 200, 105)),
        // 5 and 7 px from the document's top-left corner (its centre was 3 px off its start).
        ([53.0, 55.0], Rect::new(0, 0, 100, 50)),
    ] {
        let mut h = harness(Tool::RectMarquee);
        h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 200})).unwrap();
        h.state_mut().run("select.rect", json!({"x": 52, "y": 52, "width": 100, "height": 50})).unwrap();
        h.run_steps(2);
        drag_to(&mut h, [100.0, 100.0], to);
        let up = screen(&h, to[0], to[1]);
        button(&mut h, up, false, Modifiers::NONE);
        h.run_steps(2);
        assert_eq!(selection(&h), want, "dragged to {to:?}");
    }
}

/// #1428: with a marquee, the arrow keys nudge the selection outline 1 px, ⇧ 10 px (Photoshop);
/// each press is one undoable step with Bundle Arrow-Key Nudges off, a run of them one step with
/// it on (#2259, the default).
#[test]
fn arrow_keys_nudge_the_selection_outline() {
    use egui::Key;
    for (tool, bundle) in [(Tool::RectMarquee, false), (Tool::EllipseMarquee, false), (Tool::RectMarquee, true), (Tool::EllipseMarquee, true)] {
        let mut h = harness(tool);
        h.state_mut().session.edit_prefs(|p| p.tools.bundle_nudges = bundle);
        press_at(&mut h, 100.0, 80.0, Modifiers::NONE);
        release_at(&mut h, 200.0, 160.0, Modifiers::NONE);
        let drawn = selection(&h);
        assert!(!drawn.is_empty(), "{tool:?} drew");
        let shifted = |dx: i32, dy: i32| Rect::new(drawn.x0 + dx, drawn.y0 + dy, drawn.x1 + dx, drawn.y1 + dy);
        let steps = h.state().session.active().unwrap().history.past_len();
        h.key_press(Key::ArrowRight);
        h.run_steps(1);
        assert_eq!(selection(&h), shifted(1, 0), "{tool:?} right 1 px");
        h.key_press(Key::ArrowUp);
        h.run_steps(1);
        assert_eq!(selection(&h), shifted(1, -1), "{tool:?} up 1 px");
        h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
        h.run_steps(1);
        assert_eq!(selection(&h), shifted(1, 9), "{tool:?} shift-down 10 px");
        h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
        h.run_steps(1);
        assert_eq!(selection(&h), shifted(-9, 9), "{tool:?} shift-left 10 px");
        let nudges = h.state().session.active().unwrap().history.past_len() - steps;
        h.state_mut().session.undo();
        if bundle {
            assert_eq!(nudges, 1, "{tool:?} four quick presses, one step");
            assert_eq!(selection(&h), drawn, "{tool:?} undo takes back the run");
        } else {
            assert_eq!(nudges, 4, "{tool:?} one step per press");
            assert_eq!(selection(&h), shifted(1, 9), "{tool:?} undo takes back one nudge");
        }
    }
}

/// ⌘⌥-drag copies the selected pixels instead of cutting them: the original stays.
#[test]
fn cmd_alt_drag_floats_a_copy() {
    let (mut app, layer) = painted();
    let alpha = |d: &photocraft_doc::Document, x, y| d.layer(layer).unwrap().surface().unwrap().rgba(x, y)[3];
    let cmd_alt = Modifiers { alt: true, ..Modifiers::COMMAND };
    drag(&mut app, [20.0, 20.0], [45.0, 20.0], cmd_alt);
    app.run("select.drop", json!({})).unwrap();
    let d = app.session.active().unwrap().doc.clone();
    assert!(alpha(&d, 12, 20) == 1.0, "the original stays");
    assert!(alpha(&d, 50, 20) == 1.0, "the copy dropped 25 px right");
}

/// The messages of the error alerts open on the dialog host.
fn error_dialogs(app: &PhotocraftApp) -> Vec<String> {
    use craft::ui::dialog::StackDialog;
    let dialogs = &app.framework.dialogs;
    dialogs
        .stack()
        .ids()
        .filter_map(|id| dialogs.get_by_id::<craft::ui::dialog::HostedAlert>(id))
        .filter_map(|alert| alert.json_fields()?.get("message")?.as_str().map(String::from))
        .collect()
}

/// Closes every open dialog.
fn close_dialogs(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let ids: Vec<_> = app.framework.dialogs.stack().ids().collect();
    for id in ids {
        app.framework.dialogs.cancel_by_id(&ctx, id);
    }
}

/// Photoshop's "Could not use the move tool because the layer is locked." when a Move-tool drag
/// (the Move tool, or ⌘ with a marquee) starts on a layer it can't move: the Background with no
/// selection, a position-locked layer; a plain click says nothing, and the drag never starts.
#[test]
fn move_on_the_background_without_a_selection_shows_photoshops_lock_message() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    let steps = app.session.active().unwrap().history.past_len();
    use crate::canvas::{ToolEvent, tool_event};
    // A click: no message.
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, Modifiers::NONE);
    assert!(error_dialogs(&app).is_empty());
    // A drag: the message once, nothing moved, nothing floating.
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 20.0 }, Modifiers::NONE);
    assert_eq!(error_dialogs(&app), [crate::move_lock::MESSAGE]);
    assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_none(), "nothing floats");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "no history step");
    close_dialogs(&mut app);
    // ⌘-drag with a marquee (the Move tool for the drag, #896): the same.
    app.ui.tool = Tool::RectMarquee;
    drag(&mut app, [20.0, 20.0], [40.0, 20.0], Modifiers::COMMAND);
    assert_eq!(error_dialogs(&app), [crate::move_lock::MESSAGE]);
    close_dialogs(&mut app);
    // A position-locked layer: the same; unlocked, it moves.
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("lock", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().locks.position = true;
            Ok(())
        })
        .unwrap();
    drag(&mut app, [20.0, 20.0], [40.0, 20.0], Modifiers::COMMAND);
    assert_eq!(error_dialogs(&app).len(), 1);
    close_dialogs(&mut app);
    app.session
        .edit("unlock", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().locks.position = false;
            Ok(())
        })
        .unwrap();
    drag(&mut app, [20.0, 20.0], [40.0, 20.0], Modifiers::COMMAND);
    assert!(error_dialogs(&app).is_empty());
}

/// With a selection the Background's pixels do move (its lock is partial: transparency and
/// position), so no message; a layer locked all over gets it, with the marquee's ⌘-drag inside the
/// selection as with the Move tool.
#[test]
fn selected_pixels_move_off_the_background_but_not_off_a_fully_locked_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::RectMarquee;
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    drag(&mut app, [20.0, 20.0], [50.0, 20.0], Modifiers::COMMAND);
    assert!(error_dialogs(&app).is_empty());
    assert_eq!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset), Some((30, 0)));
    let (mut app, layer) = painted();
    app.session
        .edit("lock", |doc, _| {
            doc.layer_mut(layer).unwrap().locks.all = true;
            Ok(())
        })
        .unwrap();
    // ⌘ with the Brush is the Move tool too (#2248).
    for (tool, m) in [(Tool::RectMarquee, Modifiers::COMMAND), (Tool::Move, Modifiers::NONE), (Tool::Brush, Modifiers::COMMAND)] {
        app.ui.tool = tool;
        drag(&mut app, [20.0, 20.0], [40.0, 20.0], m);
        assert_eq!(error_dialogs(&app), [crate::move_lock::MESSAGE], "{tool:?}");
        assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_none(), "{tool:?}");
        close_dialogs(&mut app);
    }
}
