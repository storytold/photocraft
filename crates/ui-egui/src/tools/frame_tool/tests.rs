use super::*;
use crate::canvas::{ToolEvent, tool_event};
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    // The frame's handles take presses from the pass that drew them: the state a test set is shown first.
    crate::canvas::testing::frame(app);
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    crate::canvas::testing::frame(app);
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, m);
}

/// Two clicks at one point inside the double-click time.
fn double_click(app: &mut PhotocraftApp, at: [f64; 2]) {
    click(app, at);
    click(app, at);
}

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

/// Adds a red raster layer under the next frame's drawn rect and makes it the active layer.
fn add_photo(app: &mut PhotocraftApp, w: i32, h: i32) -> LayerId {
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, w, h), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap()
}

/// The document area on the app's own context, for tests that need the canvas body.
fn harness(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder()
        .with_context(ctx)
        .with_step_dt(1.0 / 60.0)
        .with_size(egui::vec2(1000., 700.))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::canvas::testing::document_area(app, ui), app);

    h.run_steps(4);
    h
}

fn screen_at(h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>, x: f64, y: f64) -> Pos2 {
    crate::canvas::document_display::active_view(h.state()).unwrap().to_screen(KPoint::new(x, y))
}

fn press_drag(h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>, from: Pos2, to: Pos2) {
    let mods = egui::Modifiers::NONE;
    h.event(egui::Event::PointerMoved(from));
    h.step();
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: mods });
    h.step();
    h.event(egui::Event::PointerMoved(from.lerp(to, 0.5)));
    h.step();
    h.event(egui::Event::PointerMoved(to));
    h.step();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(3);
}

/// Dragging the Frame-target box's bottom-right corner resizes the frame as one step and creates no layer.
#[test]
fn markers_survive_release_and_handles_resize_the_frame_without_creating_another() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let id = selected(&app).unwrap().id;
    let count = app.session.active().unwrap().doc.layer_count();
    let mut h = harness(app);
    let (from, to) = (screen_at(&h, 120., 110.), screen_at(&h, 145., 130.));
    press_drag(&mut h, from, to);
    let app = h.state_mut();
    assert_eq!(app.session.active().unwrap().doc.layer_count(), count, "resizing does not create another frame");
    assert_eq!(selected(app).unwrap().id, id);
    assert_eq!(selected(app).unwrap().frame.as_ref().unwrap().transform.m, [125., 0., 0., 100., 20., 30.]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected(app).unwrap().frame.as_ref().unwrap().transform.m, [100., 0., 0., 80., 20., 30.]);
}

/// One click selects Both (or Frame on the border), a double click Content, another Both; a border drag
/// selects Frame; an image-only drag leaves the mask alone.
#[test]
fn frame_content_and_both_selection_follow_clicks_and_double_clicks() {
    let mut app = app();
    let source = add_photo(&mut app, 200, 200);
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let original = selected(&app).unwrap().frame.clone();
    click(&mut app, [65., 65.]);
    assert_eq!(app.ui.frame_tool.target, Target::Both);
    assert_eq!(app.session.active().unwrap().active_layer, Some(frame));
    // Every further click at the same point inside the double-click time counts as a double click.
    click(&mut app, [65., 65.]);
    assert_eq!(app.ui.frame_tool.target, Target::Content);
    assert_eq!(app.session.active().unwrap().active_layer, Some(source));
    click(&mut app, [65., 65.]);
    assert_eq!(app.ui.frame_tool.target, Target::Both);
    assert_eq!(app.session.active().unwrap().active_layer, Some(frame));
    drag(&mut app, [50., 30.], [65., 40.]);
    assert_eq!(app.ui.frame_tool.target, Target::Frame);
    assert_ne!(selected(&app).unwrap().frame, original);
    let source_before = app.session.active().unwrap().doc.layer(source).unwrap().clone();
    double_click(&mut app, [70., 75.]);
    let f = selected(&app).unwrap().frame.clone();
    // A new press sequence, beyond the double-click distance of the clicks above.
    drag(&mut app, [100., 100.], [115., 110.]);
    assert!(!app.ui.frame_tool.auto_transform);
    assert!(app.ui.transform.is_none());
    assert_eq!(selected(&app).unwrap().frame, f, "image-only transform leaves the mask");
    assert_ne!(app.session.active().unwrap().doc.layer(source).unwrap(), &source_before);
}

#[test]
fn free_transform_on_the_frame_thumbnail_previews_and_commits_only_the_clip() {
    let mut app = app();
    let source = add_photo(&mut app, 200, 200);
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let original = app.session.active().unwrap().doc.layer(source).unwrap().clone();
    let ctx = app.ctx().clone();
    let set_box = |app: &mut PhotocraftApp, to: KRect| {
        let t = app.ui.transform.as_mut().unwrap();
        let before = *t.session.state();
        let edit = EditEvent::set(Mixed::Same(before), BoxState::from_rect(to), Arc::from("Transform"), None);
        let _ = t.session.apply(TransformEdit::Box(edit), 0.0);
    };
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    assert!(frame_transform(app.ui.transform.as_ref().unwrap()));
    set_box(&mut app, KRect::new(40., 40., 90., 80.));
    let (shown, _) = crate::canvas::display_doc(&mut app, 0);
    assert_eq!(shown.layer(frame).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(40, 40, 90, 80));
    assert_eq!(shown.layer(source).unwrap(), &original);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
    crate::transform_tool::commit(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(40, 40, 90, 80));
    assert_eq!(app.session.active().unwrap().doc.layer(source).unwrap(), &original);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    set_box(&mut app, KRect::new(50., 50., 100., 90.));
    crate::transform_tool::cancel(&mut app);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
}

/// Arrows move the Frame target's transform, the Content target's child, the Both target's layer; Shift is ten.
#[test]
fn frame_keyboard_nudges_change_only_the_selected_thumbnail() {
    let mut app = app();
    add_photo(&mut app, 200, 200);
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let child = selected(&app).unwrap().children().unwrap()[0].clone();
    // A unit test cannot build a `ToolContext`, so the test calls the function `FramePass::key` runs.
    let shift = Mods { shift: true, ..Mods::default() };
    assert!(nudge(&mut app, 1., 0., Mods::default(), 0.0));
    assert!(nudge(&mut app, 0., 1., shift, 5.0));
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(21, 40, 121, 120));
    assert_eq!(app.session.active().unwrap().doc.layer(child.id).unwrap(), &child);
    select(&mut app, frame, Target::Content);
    let f = selected(&app).unwrap().frame.clone();
    assert!(nudge(&mut app, 1., 0., Mods::default(), 10.0));
    assert_eq!(selected(&app).unwrap().frame, f);
    assert_ne!(app.session.active().unwrap().doc.layer(child.id).unwrap(), &child);
    assert!(!nudge(&mut app, 1., 0., Mods { alt: true, ..Mods::default() }, 11.0), "Alt arrows are not the tool's");
}

/// A frame under a hidden group is not found, and the canvas body builds no box for it.
#[test]
fn hidden_frame_ancestors_hide_placeholders_and_handles() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let id = selected(&app).unwrap().id;
    let mut h = harness(app);
    assert!(h.query_all_by_label("Frame").next().is_some(), "a visible selected frame shows its box");
    h.state_mut()
        .session
        .edit("Hide group", |doc, _| {
            let frame = doc.remove(id).unwrap();
            let mut parent = Layer::group("Hidden", vec![frame]);
            parent.visible = false;
            doc.layers.push(parent);
            Ok(())
        })
        .unwrap();
    h.run_steps(3);
    assert_eq!(frame_at(h.state(), [50., 50.]), None);
    assert!(h.query_all_by_label("Frame").next().is_none(), "no box for a hidden frame");
}

/// Esc during a frame's content drag ends its automatic transform: a Free Transform started
/// afterwards waits for Enter instead of committing on its first pointer release.
#[test]
fn cancelling_a_frame_content_drag_clears_its_auto_commit() {
    let mut app = app();
    add_photo(&mut app, 200, 200);
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    tool_event(&mut app, ToolEvent::Down { x: 70., y: 75., pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(app.ui.frame_tool.auto_transform, "a content drag transforms automatically");
    crate::transform_tool::cancel(&mut app);
    assert!(app.ui.transform.is_none());
    assert!(!app.ui.frame_tool.auto_transform);
}

#[test]
fn filled_frame_closing_border_selects_frame() {
    let mut app = app();
    add_photo(&mut app, 200, 200);
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    tool_event(&mut app, ToolEvent::Down { x: 20., y: 50., pressure: 1. }, egui::Modifiers::NONE);
    assert_eq!(app.ui.frame_tool.target, Target::Frame);
    // The tool's reference is `app.canvas_tools`, which the pass hands back after the event.
    assert!(app.canvas_tools.frame.busy());
}

#[test]
fn overhanging_content_handles_start_transform_and_cancel() {
    let mut app = app();
    add_photo(&mut app, 200, 100);
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    let st = app.session.active().unwrap();
    let frame = selected(&app).unwrap();
    let child = frame.children().unwrap().first().unwrap();
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, child);
    assert!(b.x0 < 20, "image must overhang the frame: {b:?}");
    let entries = st.history.entries();
    for (x, y) in [(f64::from(b.x0), f64::from(b.y0)), (f64::from(b.x0), f64::from(b.y0 + b.y1) / 2.)] {
        crate::canvas::testing::frame(&mut app);
        tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1. }, egui::Modifiers::NONE);
        // The transform box begins its edit when the drag starts, not at the press.
        tool_event(&mut app, ToolEvent::Move { x: x - 15., y: y - 15., pressure: 1. }, egui::Modifiers::NONE);
        assert!(app.ui.transform.is_some());
        assert!(app.ui.frame_tool.auto_transform);
        crate::transform_tool::cancel(&mut app);
        assert!(app.ui.transform.is_none());
        tool_event(&mut app, ToolEvent::Up { x: x - 15., y: y - 15. }, egui::Modifiers::NONE);
    }
    assert_eq!(app.session.active().unwrap().history.entries(), entries);
}
