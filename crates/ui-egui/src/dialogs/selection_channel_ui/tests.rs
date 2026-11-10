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

/// Chooses open document `i` in dialog `d`'s Document list.
fn set_document(app: &mut PhotocraftApp, d: u64, i: usize) {
    let document = id(app, i);
    set(app, d, "document", json!(document));
}

/// The fields of dialog `d` as they stand.
fn fields(app: &mut PhotocraftApp, d: u64) -> Map<String, Value> {
    app.ui.dialog_mut(d).unwrap().fields.clone()
}

/// Channel `name` of document `i`, across its canvas.
fn plane(app: &PhotocraftApp, i: usize, name: &str) -> Vec<f32> {
    let d = &app.session.documents()[i].doc;
    let ch = d.channels.iter().find(|c| c.name == name).unwrap();
    ch.surface.read_region(d.bounds())
}

/// The Document list is the open documents with the active one's pixel size; the dialogs open
/// on the active document, a new channel (Save) or its first channel (Load).
#[test]
fn the_document_list_holds_the_open_images_of_the_same_size() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    let f = fields(&mut app, d);
    assert!(owns(&f));
    assert_eq!(documents(&app, &f), [(id(&app, 0), "A".to_string()), (id(&app, 1), "B".to_string())], "C is another size");
    assert_eq!((f["document"].clone(), f["channel"].clone(), f["operation"].clone()), (json!(id(&app, 0)), json!("new"), json!("new")));
    assert_eq!(channels(&app, &f), [(json!("new"), "New".to_string())]);
    app.ui.close_dialog(d);
    let d = menu(&mut app, LOAD);
    let f = fields(&mut app, d);
    assert_eq!(documents(&app, &f).len(), 2);
    assert_eq!(f["channel"], "RGB", "no alpha channel: the composite");
}

/// Document: New offers only a new channel, and OK puts the selection into a new Multichannel
/// document, which becomes the active one; the source gets no channel.
#[test]
fn new_saves_the_selection_into_a_new_multichannel_document() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!("new"));
    set(&mut app, d, "name", json!("Keep"));
    let f = fields(&mut app, d);
    assert_eq!(channels(&app, &f), [(json!("new"), "New".to_string())], "a new image has only New");
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

/// Another document lists its own channels; an existing channel turns New Channel into Replace
/// Channel and allows combining; OK saves there, ignores a typed Name and keeps the source active.
#[test]
fn another_document_lists_its_channels_and_the_operation_follows_the_channel() {
    let mut app = app();
    // B gets "Old": its right half.
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Old"})).unwrap();
    app.session.set_active(0);
    let d = menu(&mut app, SAVE);
    set_document(&mut app, d, 1);
    let f = fields(&mut app, d);
    assert_eq!(channels(&app, &f), [(json!("new"), "New".to_string()), (json!(0), "Old".to_string())]);
    set(&mut app, d, "channel", json!(0));
    assert_eq!(fields(&mut app, d)["operation"], "replace", "New Channel reads Replace Channel for an existing channel");
    set(&mut app, d, "operation", json!("add"));
    let f = fields(&mut app, d);
    assert_eq!(params(&app, &f).unwrap(), json!({"document": 1, "channel": "Old", "operation": "add"}));
    set(&mut app, d, "name", json!("Typed before"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.active_index(), Some(0), "A stays active");
    assert_eq!(app.session.documents()[1].doc.channels[0].name, "Old", "Name is for a new channel only");
    assert!(app.session.documents()[0].doc.channels.is_empty());
    let old = plane(&app, 1, "Old");
    assert_eq!((old[5], old[15], old[25]), (1.0, 0.0, 1.0), "A's selection added to B's channel");
    // Back to New: the operation is New Channel again.
    let d = menu(&mut app, SAVE);
    set_document(&mut app, d, 1);
    set(&mut app, d, "channel", json!("Old"));
    set(&mut app, d, "operation", json!("intersect"));
    set(&mut app, d, "channel", json!("new"));
    assert_eq!(fields(&mut app, d)["operation"], "new");
}

/// Changing the Document starts from New (Save) or the document's first channel (Load), and a
/// document of another size is not taken.
#[test]
fn a_channel_the_chosen_document_lacks_goes_back_to_its_first() {
    let mut app = app();
    app.run("select.saveSelection", json!({"name": "Mine"})).unwrap();
    let d = menu(&mut app, SAVE);
    // A channel given by its name is that channel of the list.
    set(&mut app, d, "channel", json!("Mine"));
    assert_eq!(fields(&mut app, d)["channel"], json!(0));
    set(&mut app, d, "operation", json!("subtract"));
    set_document(&mut app, d, 1);
    let f = fields(&mut app, d);
    assert_eq!((f["channel"].clone(), f["operation"].clone()), (json!("new"), json!("new")));
    // A size the list doesn't hold falls back to A.
    set_document(&mut app, d, 2);
    assert_eq!(fields(&mut app, d)["document"], json!(id(&app, 0)));
    app.ui.close_dialog(d);
    let d = menu(&mut app, LOAD);
    assert_eq!(fields(&mut app, d)["channel"], json!(0), "A's first alpha channel");
    set_document(&mut app, d, 1);
    assert_eq!(fields(&mut app, d)["channel"], "RGB", "B has no alpha channel");
}

/// Load Selection lists the chosen document's channels and its active layer, and loads the
/// channel, inverted when asked, into the document the dialog was opened on.
#[test]
fn load_selection_reads_the_chosen_documents_channels() {
    let mut app = app();
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 30, "y": 0, "width": 10, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Edge"})).unwrap();
    app.session.set_active(0);
    app.run("select.deselect", json!({})).unwrap();
    let d = menu(&mut app, LOAD);
    set_document(&mut app, d, 1);
    let f = fields(&mut app, d);
    assert_eq!(f["channel"], json!(0), "B's first alpha channel, Edge");
    let list = channels(&app, &f);
    let layer = {
        let st = &app.session.documents()[1];
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    };
    let values: Vec<Value> = list.iter().map(|(v, _)| v.clone()).collect();
    assert_eq!(values, [json!("RGB"), json!("Red"), json!("Green"), json!("Blue"), json!(0), json!("transparency"), json!("selection")]);
    assert_eq!(list[4].1, "Edge");
    assert_eq!(list[5].1, format!("{layer} Transparency"), "named after B's active layer");
    set(&mut app, d, "invert", json!(true));
    crate::dialogs::confirm(&mut app, d).unwrap();
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    assert_eq!((sel.sample_channel(5, 5, 0), sel.sample_channel(35, 5, 0)), (1.0, 0.0), "B's Edge, inverted, in A");
    assert!(app.session.documents()[1].doc.selection.is_some(), "B keeps its own selection");
}

/// Choosing another document starts from that document's first channel (above); a channel set
/// together with the document, as automation sets fields, is kept.
#[test]
fn a_channel_chosen_together_with_its_document_is_kept() {
    let mut app = app();
    let d = menu(&mut app, LOAD);
    let b = id(&app, 1);
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!(f["channel"], "RGB");
    f.insert("document".into(), json!(b));
    f.insert("channel".into(), json!("Blue"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(
        app.session.journal.last().map(|(c, p)| (c.as_str(), p.clone())),
        Some((LOAD, json!({"document": 1, "channel": "Blue", "invert": false, "operation": "new"})))
    );
}

/// Channels the commands would read as something else by name go by their index: one named
/// like a built-in source ("Mask", "Red"), and the second of two with one name. A channel with
/// a name of its own still goes by that name, as recorded steps name it.
#[test]
fn channels_with_reserved_or_repeated_names_are_the_ones_chosen() {
    let mut app = app();
    for name in ["Mask", "Twin", "Twin", "Red", "Plain"] {
        app.run("select.saveSelection", json!({"name": name})).unwrap();
    }
    // A's selection is its left 10 columns; every channel holds it. Save the right half into each.
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20, "mode": "replace"})).unwrap();
    for (i, sent) in [json!(0), json!("Twin"), json!(2), json!(3), json!("Plain")].into_iter().enumerate() {
        let d = menu(&mut app, SAVE);
        set(&mut app, d, "channel", json!(i));
        let f = fields(&mut app, d);
        assert_eq!(f["operation"], "replace", "channel {i}");
        assert_eq!(params(&app, &f).unwrap(), json!({"channel": sent, "operation": "replace"}), "channel {i}");
        crate::dialogs::confirm(&mut app, d).unwrap();
        // Exactly the first i + 1 channels hold the right half now.
        let doc = &app.session.documents()[0].doc;
        let replaced: Vec<bool> = doc.channels.iter().map(|c| c.surface.read_region(doc.bounds())[25] == 1.0).collect();
        assert_eq!(replaced, (0..5).map(|k| k <= i).collect::<Vec<_>>(), "after saving into channel {i}");
    }
    // Load Selection reads the channel chosen, not the layer mask or the red channel.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 20, "mode": "replace"})).unwrap();
    app.run("select.saveSelection", json!({"channel": 3, "operation": "replace"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let d = menu(&mut app, LOAD);
    set(&mut app, d, "channel", json!(3));
    let f = fields(&mut app, d);
    assert_eq!(params(&app, &f).unwrap(), json!({"channel": 3, "invert": false, "operation": "new"}));
    crate::dialogs::confirm(&mut app, d).unwrap();
    let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap();
    assert_eq!((sel.sample_channel(2, 5, 0), sel.sample_channel(8, 5, 0), sel.sample_channel(25, 5, 0)), (1.0, 0.0, 0.0), "the channel named Red");
}

/// OK after the chosen document closed, or after another document became active, is an error
/// and saves nothing.
#[test]
fn a_document_closed_or_switched_before_ok_is_refused() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set_document(&mut app, d, 1);
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

/// A save into the active document sends no `document`, so the recorded step is as before.
#[test]
fn the_active_document_is_not_named_in_the_step() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    let f = fields(&mut app, d);
    assert_eq!(params(&app, &f).unwrap(), json!({"channel": "new", "name": "", "operation": "new"}));
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
