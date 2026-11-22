use super::*;
use crate::{Services, state::Tool};
use photocraft_doc::LayerContent;
use photocraft_engine::Session;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn type_layer(app: &mut PhotocraftApp, text: &str) -> u64 {
    app.run("type.create", json!({"text": text, "x": 10, "y": 30})).unwrap()["layer"].as_u64().unwrap()
}

fn text(app: &PhotocraftApp, layer: u64) -> &str {
    let LayerContent::Text(text) = &app.session.active().unwrap().doc.layer(LayerId(layer)).unwrap().content else { panic!("expected a type layer") };
    &text.text
}

#[test]
fn repeated_search_and_replace_show_the_match_and_restore_the_opening_selection() {
    let mut app = app();
    let layer = type_layer(&mut app, "café cat cat");
    let raster = LayerId(app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let before = app.session.active().unwrap().history.past_len();
    open(&mut app).unwrap();
    assert!(command(&mut app, "find").is_err());
    set_field(&mut app, "find", json!("cat")).unwrap();
    set_field(&mut app, "replace", json!("dog")).unwrap();
    assert!(command(&mut app, "change").is_err());

    assert_eq!(command(&mut app, "find").unwrap()["found"]["layer"], layer);
    let edit = app.ui.text_edit.as_ref().unwrap();
    assert_eq!((edit.anchor, edit.caret), (5, 8), "the canvas selects character offsets");
    assert_eq!(app.ui.tool, Tool::Type);
    assert_eq!(app.session.active().unwrap().history.past_len(), before);
    assert_eq!(inspect(&app).unwrap()["hasMatch"], true);
    let reply = command(&mut app, "changeFind").unwrap();
    assert_eq!(reply["found"]["start"], 10);
    assert_eq!(text(&app, layer), "café dog cat");
    assert_eq!(command(&mut app, "change").unwrap()["changed"]["layer"], layer);
    assert_eq!(inspect(&app).unwrap()["hasMatch"], false);
    assert_eq!(text(&app, layer), "café dog dog");

    set_field(&mut app, "forward", json!(false)).unwrap();
    set_field(&mut app, "ignoreAccents", json!(true)).unwrap();
    command(&mut app, "done").unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(raster));
    assert!(app.ui.text_edit.is_none());
    assert_eq!(text(&app, layer), "café dog dog");

    open(&mut app).unwrap();
    let fields = inspect(&app).unwrap()["fields"].clone();
    assert_eq!(fields["find"], "cat");
    assert_eq!(fields["replace"], "dog");
    assert_eq!(fields["forward"], false);
    assert_eq!(fields["ignoreAccents"], true);
    assert_eq!(inspect(&app).unwrap()["hasMatch"], false);
}

#[test]
fn query_and_external_edits_invalidate_the_previous_match() {
    let mut app = app();
    let layer = type_layer(&mut app, "Cat cat");
    open(&mut app).unwrap();
    set_field(&mut app, "find", json!("cat")).unwrap();
    command(&mut app, "find").unwrap();
    set_field(&mut app, "caseSensitive", json!(true)).unwrap();
    assert!(app.ui.text_edit.is_none());
    assert!(command(&mut app, "change").is_err());
    assert_eq!(command(&mut app, "find").unwrap()["found"]["start"], 4);
    app.run("type.edit", json!({"layer": layer, "text": "different cat"})).unwrap();
    assert!(command(&mut app, "changeFind").is_err());
    assert_eq!(text(&app, layer), "different cat");
}

#[test]
fn window_cursor_is_independent_of_headless_searches_and_bound_to_its_document() {
    let mut app = app();
    let layer = type_layer(&mut app, "cat cat");
    assert_eq!(app.run(COMMAND, json!({"find": "cat", "action": "find"})).unwrap()["found"]["start"], 0);
    open(&mut app).unwrap();
    set_field(&mut app, "find", json!("cat")).unwrap();
    assert_eq!(command(&mut app, "find").unwrap()["found"]["start"], 0);
    assert_eq!(app.run(COMMAND, json!({"find": "cat", "action": "find"})).unwrap()["found"]["start"], 4);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let other = app.session.active().unwrap().doc.id;
    assert!(command(&mut app, "changeAll").is_err());
    assert!(inspect(&app).is_none());
    assert_eq!(app.session.active().unwrap().doc.id, other);
    assert!(app.session.documents()[0].doc.layer(LayerId(layer)).is_some());
}

#[test]
fn modeless_window_focus_enter_tab_and_escape() {
    use egui::{Event, Key, vec2};

    let mut app = app();
    type_layer(&mut app, "cat dog");
    let mut h = egui_kittest::Harness::builder().with_size(vec2(960.0, 660.0)).build_ui_state(
        |ui, app| {
            escape(app, ui.ctx());
            super::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    open(h.state_mut()).unwrap();
    set_field(h.state_mut(), "find", json!("selected on open")).unwrap();
    set_field(h.state_mut(), "replace", json!("old")).unwrap();
    h.run_steps(5);
    h.event(Event::Text("cat".into()));
    h.run_steps(2);
    assert_eq!(inspect(h.state()).unwrap()["fields"]["find"], "cat");
    h.key_press(Key::Tab);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.event(Event::Text("before find".into()));
    h.run_steps(2);
    assert_eq!(inspect(h.state()).unwrap()["fields"]["replace"], "before find");
    open(h.state_mut()).unwrap();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(inspect(h.state()).unwrap()["hasMatch"], true);
    h.event(Event::Text("after find".into()));
    h.run_steps(2);
    assert_eq!(inspect(h.state()).unwrap()["fields"]["replace"], "after find");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(inspect(h.state()).is_none());
    assert!(h.state().ui.text_edit.is_none());
}

#[test]
fn dialog_replacement_clears_an_outside_match_at_an_old_offset() {
    let mut app = app();
    let layer = type_layer(&mut app, "a a a");
    for start in [0, 2] {
        assert_eq!(app.run(COMMAND, json!({"find": "a", "action": "find"})).unwrap()["found"]["start"], start);
    }
    let outside = app.session.edit_state.find_cursor.clone();
    open(&mut app).unwrap();
    set_field(&mut app, "find", json!("a")).unwrap();
    set_field(&mut app, "replace", json!("aaa")).unwrap();
    command(&mut app, "find").unwrap();
    assert_eq!(app.session.edit_state.find_cursor, outside);
    command(&mut app, "change").unwrap();

    assert!(app.session.edit_state.find_cursor.is_none());
    let reply = app.run(COMMAND, json!({"find": "a", "replace": "Z", "action": "change"})).unwrap();
    assert!(reply["changed"].is_null());
    assert_eq!(text(&app, layer), "aaa a a");
}
