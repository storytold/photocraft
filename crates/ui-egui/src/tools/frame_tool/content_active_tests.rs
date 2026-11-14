//! The Frame tool activates the image it took as content, without changing the command policy.
use super::*;
use crate::canvas::tool_event;

fn app_with_red() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let red = app
        .session
        .edit("Red", |doc, active| {
            let mut layer = Layer::raster("Red", doc.pixel_format());
            layer.surface_mut().unwrap().fill_rect(Rect::new(50, 50, 150, 130), &[1., 0., 0., 1.]);
            let id = layer.id;
            doc.layers.push(layer);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    (app, red)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1. }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1. }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn dragging_a_frame_activates_its_content_in_one_undo_step() {
    let (mut app, red) = app_with_red();
    let history = app.session.active().unwrap().history.entries();
    drag(&mut app, [40., 40.], [160., 140.]);
    let frame = selected(&app).unwrap().id;
    let st = app.session.active().unwrap();
    let content = st.doc.layer(frame).unwrap().children().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].id, red);
    assert!(matches!(content[0].content, LayerContent::Smart(_)));
    assert_eq!(st.active_layer, Some(red), "the image is active after the tool's drag");
    assert_eq!(st.selected_layers(), vec![red]);
    assert_eq!(app.ui.frame_tool.selected, Some(frame.0));
    assert_eq!(app.ui.frame_tool.target, Target::Content);
    let mut expected = history.clone();
    expected.push("New Frame".into());
    assert_eq!(st.history.entries(), expected, "selection adds no extra history step");
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(red));
    assert!(st.doc.layer(frame).is_none());
    assert!(matches!(st.doc.layer(red).unwrap().content, LayerContent::Raster(_)));
    assert_eq!(st.history.entries(), history);
}

#[test]
fn drawing_an_empty_frame_keeps_the_frame_active_and_undo_restores_the_prior_layer() {
    let (mut app, red) = app_with_red();
    let history = app.session.active().unwrap().history.entries();
    drag(&mut app, [400., 250.], [500., 350.]);
    let frame = selected(&app).unwrap().id;
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(frame).unwrap().children().unwrap().is_empty());
    assert_eq!(st.active_layer, Some(frame));
    assert_eq!(st.selected_layers(), vec![frame]);
    assert_eq!(app.ui.frame_tool.selected, Some(frame.0));
    assert_eq!(app.ui.frame_tool.target, Target::Frame);
    let mut expected = history.clone();
    expected.push("New Frame".into());
    assert_eq!(st.history.entries(), expected);
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(red));
    assert!(st.doc.layer(frame).is_none());
    assert_eq!(st.history.entries(), history);
}

#[test]
fn escape_commits_the_frame_with_content_active_and_later_release_changes_nothing() {
    let (mut app, red) = app_with_red();
    let before = app.session.active().unwrap().history.entries();
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 40., y: 40., pressure: 1. }, mods);
    tool_event(&mut app, ToolEvent::Move { x: 160., y: 140., pressure: 1. }, mods);
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: mods }],
        ..Default::default()
    });
    crate::shortcuts::handle(&mut app, &ctx);
    ctx.end_pass().textures_delta.clear();
    let frame = selected(&app).unwrap().id;
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(red));
    assert_eq!(st.doc.layer(frame).unwrap().children().unwrap()[0].id, red);
    assert_eq!(app.ui.frame_tool.target, Target::Content);
    assert!(app.ui.frame_tool.drag.is_none());
    let mut expected = before;
    expected.push("New Frame".into());
    assert_eq!(st.history.entries(), expected);
    let document = st.doc.clone();
    tool_event(&mut app, ToolEvent::Move { x: 500., y: 350., pressure: 1. }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 500., y: 350. }, mods);
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(red));
    assert_eq!(st.doc, document);
    assert_eq!(st.history.entries(), expected);
}

#[test]
fn explicit_new_frame_command_keeps_its_frame_selection() {
    let (mut app, red) = app_with_red();
    let result = app.run("layer.new.frame", json!({"layer":red.0,"x":40,"y":40,"width":120,"height":100})).unwrap();
    let frame = LayerId(result["layer"].as_u64().unwrap());
    assert_eq!(result["content"], json!(red.0));
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(frame), "the command outside a tool drag keeps its policy");
    assert_eq!(st.selected_layers(), vec![frame]);
    assert_eq!(st.doc.layer(frame).unwrap().children().unwrap()[0].id, red);
}
