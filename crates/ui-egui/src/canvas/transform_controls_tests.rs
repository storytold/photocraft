use super::transform_controls::{ControlsPress, begin_controls, frame, frame_layers};
use super::*;
use serde_json::json;

/// The corners of a rectangle as `[x0, y0, x1, y1]`.
fn corners(r: craft::behavior::kurbo::Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// The messages of the alerts the host has open (`HostedAlert`'s key is "alert", its `message` field the text).
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter(|d| d.key.as_deref() == Some("alert"))
        .filter_map(|d| Some(d.fields.as_ref()?["message"].as_str()?.to_string()))
        .collect()
}

/// Closes every alert the host has open.
fn close_alerts(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let ids: Vec<_> = app.framework.dialogs.open_dialogs().into_iter().filter(|d| d.key.as_deref() == Some("alert")).map(|d| d.id).collect();
    for id in ids {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
}

/// #7538: as in Photoshop, the box frames a loose selection until its pixels are moved, then
/// the pixels themselves, while they float and once dropped.
#[test]
fn transform_controls_frame_the_moved_pixels_not_the_marquee() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let framed = |app: &PhotocraftApp| corners(frame(app).unwrap()).map(|v| v.round() as i32);
    assert_eq!(framed(&app), [20, 20, 120, 110], "the selection, before anything moves");
    app.run("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    assert_eq!(framed(&app), [50, 45, 100, 85], "the floating pixels");
    app.run("select.drop", json!({})).unwrap();
    assert_eq!(framed(&app), [50, 45, 100, 85], "dropped");
}

/// #7538: with several layers selected the Move tool moves them whole, and the selection goes
/// with them, as in Photoshop: its ants follow the drag and it lands with them in one step.
#[test]
fn moving_several_layers_carries_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 120, "y": 120, "width": 30, "height": 30})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let steps = app.session.active().unwrap().history.past_len();
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 68.0, pressure: 1.0 }, m);
    assert_eq!(super::tools::selection::shown_offset(&app), Some((12, 8)), "the ants follow the layers");
    tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 68.0 }, m);
    let st = app.session.active().unwrap();
    assert!(photocraft_engine::float_cmds::floating(st).is_none(), "whole layers moved, nothing floats");
    assert_eq!(st.doc.selection.as_ref().map(|s| s.content_bounds()), Some(photocraft_geom::Rect::new(32, 28, 132, 118)));
    assert_eq!(st.doc.layer(a).unwrap().surface().unwrap().content_bounds(), photocraft_geom::Rect::new(52, 48, 102, 88));
    assert_eq!(st.history.past_len(), steps + 1, "one Move step");
    assert!(app.session.undo());
    let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    assert_eq!(sel, Some(photocraft_geom::Rect::new(20, 20, 120, 110)), "Undo takes the selection back too");

    // A piece floating (a marquee ⌘-drag with both selected) when the layers are dragged: the
    // press drops it first, so the ants follow the drag from where it was dropped.
    app.run("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    let outline = photocraft_engine::float_cmds::floating(app.session.active().unwrap()).unwrap().parts.outline().unwrap().content_bounds();
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
    let st = app.session.active().unwrap();
    assert!(photocraft_engine::float_cmds::floating(st).is_none(), "the press dropped the piece");
    assert_eq!(st.doc.selection.as_ref().map(|s| s.content_bounds()), Some(outline.translate(10, 5)), "at its offset");
    tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 68.0, pressure: 1.0 }, m);
    assert_eq!(super::tools::selection::shown_offset(&app), Some((12, 8)), "then the ants follow the drag");
    tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 68.0 }, m);
    let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    assert_eq!(sel, Some(outline.translate(22, 13)), "released where the ants were");
}

/// A transparent 200² document: a red square (40..90, 40..80) on its layer, a loose marquee
/// (20..120, 20..110) round it, cut and floated 10, 5 to the right and down.
fn floating_square() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.session.execute("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    app
}

/// The float's offset, if a piece floats.
fn float_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset)
}

/// #7538: a selection tool's press on a floating piece's ants drops the piece and drags its outline (#11368), a
/// ⌘ press drags the piece; one in the margin its cut left of the loose marquee draws a new selection, as the ants
/// say.
#[test]
fn a_press_in_the_margin_of_a_floating_piece_starts_a_new_selection() {
    let none = egui::Modifiers::NONE;
    let drag = |app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: egui::Modifiers| {
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
    };
    for tool in [Tool::RectMarquee, Tool::Lasso] {
        let mut app = floating_square();
        app.ui.tool = tool;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let bounds = |app: &PhotocraftApp| app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
        // The ants outline the moved pixels, at the piece's offset; a press on them drops the piece and drags that outline.
        let (dx, dy) = float_offset(&app).expect("the piece floats");
        let outline = super::tools::selection::floating_outline(&app, 0).expect("the piece's outline").content_bounds().translate(dx, dy);
        drag(&mut app, [70.0, 60.0], [80.0, 60.0], none);
        assert_eq!(float_offset(&app), None, "{tool:?}: on the square the press drops the piece (#11368)");
        assert_eq!(bounds(&app), Some(outline.translate(10, 0)), "{tool:?}: and drags its outline");

        let mut app = floating_square();
        app.ui.tool = tool;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        drag(&mut app, [70.0, 60.0], [80.0, 60.0], egui::Modifiers::COMMAND);
        assert_eq!(float_offset(&app), Some((20, 5)), "{tool:?}: ⌘ drags the square");

        let mut app = floating_square();
        app.ui.tool = tool;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        drag(&mut app, [33.0, 28.0], [10.0, 10.0], none);
        assert_eq!(float_offset(&app), None, "{tool:?}: in the old marquee's margin the piece is dropped and a new selection starts");
    }
}

/// Several selected layers keep one frame round them all while the active one's pixels float:
/// the piece's outline frames it only when it is the only layer framed.
#[test]
fn a_floating_piece_among_several_selected_layers_keeps_the_shared_frame() {
    let mut app = floating_square();
    app.session.execute("select.drop", json!({})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert_eq!(frame_layers(&app).len(), 2, "two layers framed");
    app.session.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_some());
    let r = frame(&app).unwrap();
    assert!(r.width() > 60.0 && r.height() > 60.0, "the frame isn't shrunk to the one piece: {r:?}");
}

#[test]
fn several_selected_layers_share_one_frame_that_transforms_them_all() {
    // As in Photoshop: Show Transform Controls frames every selected layer, and its handles
    // start one Free Transform of all of them (#6501).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 40, 40], "fill": "#ff0000"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [100, 120, 50, 30], "fill": "#ff0000"})).unwrap();
    let b = app.session.active().unwrap().active_layer.unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let one = frame(&app).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    let both = frame(&app).unwrap();
    assert!(one.union(both) == both && both.width() > one.width() && both.height() > one.height(), "{both:?} frames both");
    let c = corners(both);
    assert!(c.iter().zip([20.0, 30.0, 150.0, 150.0]).all(|(v, e)| (v - e).abs() < 1e-3), "{c:?}");
    // The Background among them joins the frame, as in Photoshop; its handles refuse (#11101).
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    assert_eq!(frame_layers(&app).len(), 3);
    assert_eq!(corners(frame(&app).unwrap()), [0.0, 0.0, 200.0, 200.0]);
    app.session.execute("layer.select", json!({"layer": bg.0, "mode": "toggle"})).unwrap();
    assert_eq!(frame_layers(&app).len(), 2);
    assert_eq!(frame(&app), Some(both));
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
    let t = app.ui.transform.as_ref().expect("the shared frame's handle starts Free Transform");
    assert_eq!(corners(t.session.rect()), [20.0, 30.0, 150.0, 150.0]);
    assert_eq!(t.layers, [a.0, b.0]);
}

#[test]
fn move_transform_controls_start_free_transform_for_vector_shapes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert!(frame(&app).is_some(), "shape layers have transform bounds");
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
    let t = app.ui.transform.as_ref().expect("control press starts Free Transform");
    assert_eq!(corners(t.session.rect()), [20.0, 30.0, 100.0, 70.0]);
}

#[test]
fn move_transform_controls_follow_floating_selected_pixels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert!(crate::move_ui::float_selected(&mut app, false, 100.0, 50.0));
    assert_eq!(corners(frame(&app).unwrap()), [200.0, 150.0, 300.0, 210.0]);
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
    assert_eq!(corners(app.ui.transform.as_ref().unwrap().session.rect()), [200.0, 150.0, 300.0, 210.0]);
}

/// #5297: with a selection the Move tool's transform controls are the selection's rectangle,
/// though the layer fills only part of it, and a press on them opens that box.
#[test]
fn move_transform_controls_frame_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 60, "y": 80, "width": 100, "height": 120})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert_eq!(corners(frame(&app).unwrap()), [60.0, 80.0, 160.0, 200.0]);
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
    assert_eq!(corners(app.ui.transform.as_ref().unwrap().session.rect()), [60.0, 80.0, 160.0, 200.0]);
}

/// #2585: Free Transform on a position-locked layer is refused before the box opens, with
/// Photoshop's alert, from ⌘T and from the Move tool's transform controls alike. It used to
/// open, preview every drag and fail only at commit.
#[test]
fn free_transform_on_a_locked_layer_is_refused_up_front() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.session
        .edit("lock", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().locks.position = true;
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let messages = alerts;
    let ctx = app.ctx().clone();
    // ⌘T.
    let r = crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({}));
    assert_eq!(r, Err(crate::transform_tool::LOCKED.to_string()));
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "no box opened");
    assert_eq!(messages(&app), [crate::transform_tool::LOCKED]);
    close_alerts(&mut app, &ctx);
    // The Move tool's transform controls.
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert!(frame(&app).is_some());
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Missed);
    assert!(app.ui.transform.is_none());
    assert_eq!(messages(&app), [crate::transform_tool::LOCKED]);
}

#[test]
fn the_background_among_several_joins_the_frame_and_its_handles_refuse() {
    // Measured in the reference application (#11101): the box frames them all, the Background
    // included, and a handle drag shows its locked alert and changes nothing.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 40, 40], "fill": "#ff0000"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [100, 120, 50, 30], "fill": "#ff0000"})).unwrap();
    let b = app.session.active().unwrap().active_layer.unwrap();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert_eq!(corners(frame(&app).expect("the box frames them all")), [0.0, 0.0, 200.0, 200.0], "the Background included");
    let before = app.session.active().unwrap().doc.clone();
    let ctx = app.ctx().clone();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Refused, "the press is taken, not a move");
    assert!(app.ui.transform.is_none(), "no box opens");
    let messages = alerts(&app);
    assert_eq!(messages, [super::transform_controls::CONTROLS_LOCKED]);
    assert!(app.session.active().unwrap().doc == before, "nothing changed");
    // The Background alone: no box at all.
    app.session.execute("layer.select", json!({"layer": bg.0})).unwrap();
    assert_eq!(frame(&app), None);
    // Without the Background the handles start Free Transform, as before.
    close_alerts(&mut app, &ctx);
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    app.session.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
    assert_eq!(app.ui.transform.as_ref().unwrap().layers, [a.0, b.0]);
}
