use photocraft_color::ColorMode;
use serde_json::json;

use super::*;

/// "A" and "B" are 40×20, "C" is 30×20; "A" is active with its left 10 columns selected.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for (name, width) in [("A", 40), ("B", 40), ("C", 30)] {
        app.run("file.new", json!({"width": width, "height": 20, "name": name})).unwrap();
    }
    app.session.set_active(0);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 20})).unwrap();
    app
}

/// The id of open document `i`.
fn id(app: &PhotocraftApp, i: usize) -> u64 {
    app.session.documents()[i].doc.id.0
}

/// Opens `command`'s dialog from the menu, as Select › Save Selection… / Load Selection… do.
fn menu(app: &mut PhotocraftApp, command: &str) -> u64 {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, command, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

/// Sets `key` of dialog `d` as a click in it would, keeping the fields consistent.
fn set(app: &mut PhotocraftApp, d: u64, key: &str, v: Value) {
    let mut f = app.ui.dialog_mut(d).unwrap().fields.clone();
    f.insert(key.into(), v);
    normalize(app, &mut f);
    app.ui.dialog_mut(d).unwrap().fields = f;
}

fn fields(app: &mut PhotocraftApp, d: u64) -> Map<String, Value> {
    app.ui.dialog_mut(d).unwrap().fields.clone()
}

/// Channel `name` of document `i`, across its canvas.
fn plane(app: &PhotocraftApp, i: usize, name: &str) -> Vec<f32> {
    let d = &app.session.documents()[i].doc;
    let ch = d.channels.iter().find(|c| c.name == name).unwrap();
    ch.surface.read_region(d.bounds())
}

#[test]
fn the_document_list_holds_the_open_images_of_the_same_size() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    let f = fields(&mut app, d);
    assert!(owns(&f));
    assert_eq!(documents(&app, &f), [(id(&app, 0), "A".to_string()), (id(&app, 1), "B".to_string())], "C is another size");
    assert_eq!((f["document"].clone(), f["channel"].clone(), f["operation"].clone()), (json!(id(&app, 0)), json!("new"), json!("new")));
    assert_eq!(channels(&app, &f), [("new".to_string(), "New".to_string())]);
    app.ui.close_dialog(d);
    let d = menu(&mut app, LOAD);
    let f = fields(&mut app, d);
    assert_eq!(documents(&app, &f).len(), 2);
    assert_eq!(f["channel"], "RGB", "no alpha channel: the composite");
}

#[test]
fn new_saves_the_selection_into_a_new_multichannel_document() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!("new"));
    set(&mut app, d, "name", json!("Keep"));
    assert_eq!(channels(&app, &fields(&mut app, d)), [("new".to_string(), "New".to_string())], "a new image has only New");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.documents().len(), 4);
    assert_eq!(app.session.active_index(), Some(3));
    let new = &app.session.documents()[3].doc;
    assert_eq!((new.name.as_str(), new.mode), ("Untitled-1", ColorMode::Multichannel));
    assert_eq!(new.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
    // Ink where nothing was selected.
    let ink = plane(&app, 3, "Keep");
    assert_eq!((ink[5], ink[25]), (0.0, 1.0));
    assert!(app.session.documents()[0].doc.channels.is_empty(), "nothing saved in A");
    assert_eq!(
        app.session.journal.last().map(|(c, p)| (c.as_str(), p.clone())),
        Some((SAVE, json!({"document": "new", "channel": "new", "name": "Keep", "operation": "new"})))
    );
}

#[test]
fn another_document_lists_its_channels_and_the_operation_follows_the_channel() {
    let mut app = app();
    // B gets "Old": its right half.
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Old"})).unwrap();
    app.session.set_active(0);
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!(id(&app, 1)));
    let f = fields(&mut app, d);
    assert_eq!(channels(&app, &f), [("new".to_string(), "New".to_string()), ("Old".to_string(), "Old".to_string())]);
    set(&mut app, d, "channel", json!("Old"));
    assert_eq!(fields(&mut app, d)["operation"], "replace", "New Channel reads Replace Channel for an existing channel");
    set(&mut app, d, "operation", json!("add"));
    assert_eq!(params(&app, &fields(&mut app, d)).unwrap(), json!({"document": 1, "channel": "Old", "operation": "add"}));
    set(&mut app, d, "name", json!("Typed before"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.active_index(), Some(0), "A stays active");
    assert_eq!(app.session.documents()[1].doc.channels[0].name, "Old", "Name is for a new channel only");
    assert!(app.session.documents()[0].doc.channels.is_empty());
    let old = plane(&app, 1, "Old");
    assert_eq!((old[5], old[15], old[25]), (1.0, 0.0, 1.0), "A's selection added to B's channel");
    // Back to New: the operation is New Channel again.
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!(id(&app, 1)));
    set(&mut app, d, "channel", json!("Old"));
    set(&mut app, d, "operation", json!("intersect"));
    set(&mut app, d, "channel", json!("new"));
    assert_eq!(fields(&mut app, d)["operation"], "new");
}

#[test]
fn a_channel_the_chosen_document_lacks_goes_back_to_its_first() {
    let mut app = app();
    app.run("select.saveSelection", json!({"name": "Mine"})).unwrap();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "channel", json!("Mine"));
    set(&mut app, d, "operation", json!("subtract"));
    set(&mut app, d, "document", json!(id(&app, 1)));
    let f = fields(&mut app, d);
    assert_eq!((f["channel"].clone(), f["operation"].clone()), (json!("new"), json!("new")));
    // A size the list doesn't hold falls back to A.
    set(&mut app, d, "document", json!(id(&app, 2)));
    assert_eq!(fields(&mut app, d)["document"], json!(id(&app, 0)));
    app.ui.close_dialog(d);
    let d = menu(&mut app, LOAD);
    assert_eq!(fields(&mut app, d)["channel"], "Mine");
    set(&mut app, d, "document", json!(id(&app, 1)));
    assert_eq!(fields(&mut app, d)["channel"], "RGB", "B has no alpha channel");
}

#[test]
fn load_selection_reads_the_chosen_documents_channels() {
    let mut app = app();
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 30, "y": 0, "width": 10, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Edge"})).unwrap();
    app.session.set_active(0);
    app.run("select.deselect", json!({})).unwrap();
    let d = menu(&mut app, LOAD);
    set(&mut app, d, "document", json!(id(&app, 1)));
    let f = fields(&mut app, d);
    assert_eq!(f["channel"], "Edge");
    let list = channels(&app, &f);
    let layer = {
        let st = &app.session.documents()[1];
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    };
    assert_eq!(list.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(), ["RGB", "Red", "Green", "Blue", "Edge", "transparency", "selection"]);
    assert_eq!(list[5].1, format!("{layer} Transparency"), "named after B's active layer");
    set(&mut app, d, "invert", json!(true));
    crate::dialogs::confirm(&mut app, d).unwrap();
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    assert_eq!((sel.sample_channel(5, 5, 0), sel.sample_channel(35, 5, 0)), (1.0, 0.0), "B's Edge, inverted, in A");
    assert!(app.session.documents()[1].doc.selection.is_some(), "B keeps its own selection");
}

#[test]
fn a_document_closed_or_switched_before_ok_is_refused() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!(id(&app, 1)));
    app.session.set_active(1);
    app.run("file.close", json!({"save": false})).unwrap();
    app.session.set_active(0);
    assert!(crate::dialogs::confirm(&mut app, d).is_err());
    assert!(app.session.documents().iter().all(|st| st.doc.channels.is_empty()), "saved nowhere");
    let d = menu(&mut app, SAVE);
    app.run("file.new", json!({"width": 40, "height": 20, "name": "D"})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_err(), "the selection is A's");
    assert!(app.session.documents().iter().all(|st| st.doc.channels.is_empty()));
}

#[test]
fn the_active_document_is_not_named_in_the_step() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    assert_eq!(params(&app, &fields(&mut app, d)).unwrap(), json!({"channel": "new", "name": "", "operation": "new"}));
}

/// The dialog draws Photoshop's fields; the first operation reads Replace Channel once an
/// existing channel is chosen.
#[test]
fn the_dialog_shows_the_document_channel_and_operations() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.run("select.saveSelection", json!({"name": "Mine"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let d = menu(h.state_mut(), SAVE);
    h.run_steps(4);
    for label in ["Document:", "Channel:", "Name:", "New Channel", "Add to Channel", "Subtract from Channel", "Intersect with Channel", "OK"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert!(h.query_by_label("Replace Channel").is_none());
    h.state_mut().ui.dialog_mut(d).unwrap().fields.insert("channel".into(), json!("Mine"));
    h.run_steps(4);
    assert!(h.query_by_label("Replace Channel").is_some());
    assert!(h.query_by_label("New Channel").is_none());
    assert_eq!(h.state_mut().ui.dialog_mut(d).unwrap().fields["operation"], "replace");
    h.state_mut().ui.close_dialog(d);
    menu(h.state_mut(), LOAD);
    h.run_steps(4);
    for label in ["Document:", "Channel:", "Invert", "New Selection", "Add to Selection", "Subtract from Selection", "Intersect with Selection"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
}
