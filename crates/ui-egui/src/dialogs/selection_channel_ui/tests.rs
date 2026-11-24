use craft::ui::dialog::{DialogId, StackDialog};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_color::ColorMode;
use serde_json::json;

use super::testing::{enabled, fields, options};
use super::*;

/// "A" and "B" are 40×20, "C" is 30×20; "A" is active with its left 10 columns selected.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (name, width) in [("A", 40), ("B", 40), ("C", 30)] {
        app.run("file.new", json!({"width": width, "height": 20, "name": name})).unwrap();
    }
    app.session.set_active(0);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 20})).unwrap();
    app
}

/// The Document list's value of open document `i`: its id.
fn id(app: &PhotocraftApp, i: usize) -> String {
    app.session.documents()[i].doc.id.0.to_string()
}

/// Opens `command`'s dialog from the menu, as Select › Save Selection… / Load Selection… do, and runs a pass.
fn menu(app: &mut PhotocraftApp, command: &str) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, command, json!({})).unwrap();
    let dialog = crate::new_layer_ui::testing::top(app);
    assert_eq!(reply["dialog"], json!(dialog.value().to_string()));
    crate::dialogs::testing::show(app);
    dialog
}

/// Sets `key` of dialog `d` as `ui.dialog.set` (or the user choosing it in the dialog) does, with no pass after it.
fn set_value(app: &mut PhotocraftApp, d: DialogId, key: &str, v: Value) {
    crate::new_layer_ui::testing::set(app, d, key, v).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// [`set_value`], then the pass that shows the dialog with the values made consistent with it.
fn set(app: &mut PhotocraftApp, d: DialogId, key: &str, v: Value) {
    set_value(app, d, key, v);
    crate::dialogs::testing::show(app);
}

/// Chooses open document `i` in dialog `d`'s Document list.
fn set_document(app: &mut PhotocraftApp, d: DialogId, i: usize) {
    let document = id(app, i);
    set(app, d, "document", json!(document));
}

/// Value `key` of dialog `d`, as text.
fn text(app: &PhotocraftApp, d: DialogId, key: &str) -> String {
    fields(app, d).text(key).unwrap_or_else(|| panic!("no {key}")).to_string()
}

/// The option ids of row `key` of dialog `d`.
fn ids(app: &PhotocraftApp, d: DialogId, key: &str) -> Vec<String> {
    options(app, d, key).into_iter().map(|(id, _)| id).collect()
}

/// OK on dialog `d`.
fn confirm(app: &mut PhotocraftApp, d: DialogId) -> Result<(), String> {
    crate::dialogs::testing::confirm(app, d)
}

/// What OK would send for dialog `d` now.
fn sent(app: &PhotocraftApp, d: DialogId) -> Value {
    let key = StackDialog::key(app.framework.dialogs.get_by_id::<HostedParamDialog>(d).unwrap()).unwrap().to_owned();
    command(app, &key, &fields(app, d)).unwrap().1
}

/// Whether dialog `d` is still open.
fn is_open(app: &PhotocraftApp, d: DialogId) -> bool {
    app.framework.dialogs.stack().ids().any(|open| open == d)
}

/// Channel `name` of document `i`, across its canvas.
fn plane(app: &PhotocraftApp, i: usize, name: &str) -> Vec<f32> {
    let d = &app.session.documents()[i].doc;
    let ch = d.channels.iter().find(|c| c.name == name).unwrap();
    ch.surface.read_region(d.bounds())
}

/// The last journaled command.
fn last_step(app: &PhotocraftApp) -> Option<(String, Value)> {
    app.session.journal.last().map(|(c, p)| (c.clone(), p.clone()))
}

/// The Document list is the open documents with the active one's pixel size; the dialogs open
/// on the active document, a new channel (Save) or its first channel (Load).
#[test]
fn the_document_list_holds_the_open_images_of_the_same_size() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    assert_eq!(ids(&app, d, "document"), [id(&app, 0), id(&app, 1), "new".to_string()], "C is another size");
    assert_eq!((text(&app, d, "document"), text(&app, d, "channel"), text(&app, d, "operation")), (id(&app, 0), "new".into(), "new".into()));
    assert_eq!(ids(&app, d, "channel"), ["new"]);
    crate::dialogs::testing::cancel(&mut app, d).unwrap();
    let d = menu(&mut app, LOAD);
    assert_eq!(ids(&app, d, "document"), [id(&app, 0), id(&app, 1)], "Load has no New");
    assert_eq!(text(&app, d, "channel"), "RGB", "no alpha channel: the composite");
}

/// Document: New offers only a new channel, and OK puts the selection into a new Multichannel
/// document, which becomes the active one; the source gets no channel.
#[test]
fn new_saves_the_selection_into_a_new_multichannel_document() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "document", json!("new"));
    set(&mut app, d, "name", json!("Keep"));
    assert_eq!(ids(&app, d, "channel"), ["new"], "a new image has only New");
    confirm(&mut app, d).unwrap();
    assert_eq!(app.session.documents().len(), 4);
    assert_eq!(app.session.active_index(), Some(3));
    let new = &app.session.documents()[3].doc;
    assert_eq!((new.name.as_str(), new.mode), ("Untitled-1", ColorMode::Multichannel));
    assert_eq!(new.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
    // Ink where nothing was selected.
    let ink = plane(&app, 3, "Keep");
    assert_eq!((ink[5], ink[25]), (0.0, 1.0));
    assert!(app.session.documents()[0].doc.channels.is_empty(), "nothing saved in A");
    assert_eq!(last_step(&app), Some((SAVE.to_string(), json!({"document": "new", "channel": "new", "name": "Keep", "operation": "new"}))));
}

/// Another document lists its own channels; an existing channel turns New Channel into Replace
/// Channel, disables Name and allows combining; OK saves there and keeps the source active.
#[test]
fn another_document_lists_its_channels_and_the_operation_follows_the_channel() {
    let mut app = app();
    // B gets "Old": its right half.
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Old"})).unwrap();
    app.session.set_active(0);
    let d = menu(&mut app, SAVE);
    assert!(enabled(&app, d, "name"), "Name is for a new channel");
    assert_eq!(options(&app, d, "operation"), [("new".into(), true), ("add".into(), false), ("subtract".into(), false), ("intersect".into(), false)]);
    set(&mut app, d, "name", json!("Typed before"));
    set_document(&mut app, d, 1);
    assert_eq!(ids(&app, d, "channel"), ["new", "0"]);
    set(&mut app, d, "channel", json!("0"));
    assert_eq!(text(&app, d, "operation"), "replace", "New Channel reads Replace Channel for an existing channel");
    assert_eq!(options(&app, d, "operation"), [("replace".into(), true), ("add".into(), true), ("subtract".into(), true), ("intersect".into(), true)]);
    assert!(!enabled(&app, d, "name"), "no Name for an existing channel");
    set(&mut app, d, "operation", json!("add"));
    assert_eq!(sent(&app, d), json!({"document": 1, "channel": "Old", "operation": "add"}), "the typed Name is not sent");
    confirm(&mut app, d).unwrap();
    assert_eq!(app.session.active_index(), Some(0), "A stays active");
    assert_eq!(app.session.documents()[1].doc.channels[0].name, "Old", "Name is for a new channel only");
    assert!(app.session.documents()[0].doc.channels.is_empty());
    let old = plane(&app, 1, "Old");
    assert_eq!((old[5], old[15], old[25]), (1.0, 0.0, 1.0), "A's selection added to B's channel");
    // Back to New: the operation is New Channel again.
    let d = menu(&mut app, SAVE);
    set_document(&mut app, d, 1);
    set(&mut app, d, "channel", json!("0"));
    set(&mut app, d, "operation", json!("intersect"));
    set(&mut app, d, "channel", json!("new"));
    assert_eq!(text(&app, d, "operation"), "new");
}

/// Changing the Document starts from New (Save) or the document's first channel (Load), and a
/// document of another size is not offered.
#[test]
fn a_channel_the_chosen_document_lacks_goes_back_to_its_first() {
    let mut app = app();
    app.run("select.saveSelection", json!({"name": "Mine"})).unwrap();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "channel", json!("0"));
    set(&mut app, d, "operation", json!("subtract"));
    set_document(&mut app, d, 1);
    assert_eq!((text(&app, d, "channel"), text(&app, d, "operation")), ("new".into(), "new".into()));
    // C has another size: it is no choice.
    let c = id(&app, 2);
    assert!(crate::new_layer_ui::testing::set(&mut app, d, "document", json!(c)).is_err());
    assert_eq!(text(&app, d, "document"), id(&app, 1));
    crate::dialogs::testing::cancel(&mut app, d).unwrap();
    let d = menu(&mut app, LOAD);
    assert_eq!(text(&app, d, "channel"), "0", "A's first alpha channel");
    set_document(&mut app, d, 1);
    assert_eq!(text(&app, d, "channel"), "RGB", "B has no alpha channel");
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
    assert_eq!(text(&app, d, "channel"), "0", "B's first alpha channel, Edge");
    assert_eq!(ids(&app, d, "channel"), ["RGB", "Red", "Green", "Blue", "0", "transparency", "selection"]);
    let layer = {
        let st = &app.session.documents()[1];
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    };
    let labels = channels(&fields(&app, d), false);
    assert_eq!(labels[4].1, "Edge");
    assert_eq!(labels[5].1, format!("{layer} Transparency"), "named after B's active layer");
    set(&mut app, d, "invert", json!(true));
    confirm(&mut app, d).unwrap();
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    assert_eq!((sel.sample_channel(5, 5, 0), sel.sample_channel(35, 5, 0)), (1.0, 0.0), "B's Edge, inverted, in A");
    assert!(app.session.documents()[1].doc.selection.is_some(), "B keeps its own selection");
}

/// Choosing another document starts from that document's first channel (above); a channel set
/// after the document, as automation sets fields one by one, is kept.
#[test]
fn a_channel_chosen_together_with_its_document_is_kept() {
    let mut app = app();
    let d = menu(&mut app, LOAD);
    assert_eq!(text(&app, d, "channel"), "RGB");
    set_document(&mut app, d, 1);
    set(&mut app, d, "channel", json!("Blue"));
    confirm(&mut app, d).unwrap();
    assert_eq!(last_step(&app), Some((LOAD.to_string(), json!({"document": 1, "channel": "Blue", "invert": false, "operation": "new"}))));
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
    for (i, want) in [json!(0), json!("Twin"), json!(2), json!(3), json!("Plain")].into_iter().enumerate() {
        let d = menu(&mut app, SAVE);
        set(&mut app, d, "channel", json!(i.to_string()));
        assert_eq!(text(&app, d, "operation"), "replace", "channel {i}");
        assert_eq!(sent(&app, d), json!({"channel": want, "operation": "replace"}), "channel {i}");
        confirm(&mut app, d).unwrap();
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
    set(&mut app, d, "channel", json!("3"));
    assert_eq!(sent(&app, d), json!({"channel": 3, "invert": false, "operation": "new"}));
    confirm(&mut app, d).unwrap();
    let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap();
    assert_eq!((sel.sample_channel(2, 5, 0), sel.sample_channel(8, 5, 0), sel.sample_channel(25, 5, 0)), (1.0, 0.0, 0.0), "the channel named Red");
}

/// The channels of A after the dialog saved A's right half into some of them: whether each holds it.
fn replaced(app: &PhotocraftApp) -> Vec<(String, bool)> {
    let doc = &app.session.documents()[0].doc;
    doc.channels.iter().map(|c| (c.name.clone(), c.surface.read_region(doc.bounds())[25] == 1.0)).collect()
}

/// A command that deletes or adds channels while the dialog is open doesn't move the choice to
/// the channel now at its place: the chosen channel follows its name, and when its name doesn't
/// tell it apart Save Selection goes back to New and Load Selection to the first channel. The
/// dialog shows the followed channel from its next pass, and OK follows it with no pass between.
#[test]
fn a_channel_deleted_or_added_under_the_dialog_does_not_move_the_choice() {
    let mut app = app();
    for name in ["X", "Y", "Z"] {
        app.run("select.saveSelection", json!({"name": name})).unwrap();
    }
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20, "mode": "replace"})).unwrap();
    // Y is chosen; X is deleted, so Z is the second channel now.
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "channel", json!("1"));
    app.run("channel.delete", json!({"channel": "X"})).unwrap();
    crate::dialogs::testing::show(&mut app);
    assert_eq!((text(&app, d, "channel"), ids(&app, d, "channel")), ("0".into(), vec!["new".into(), "0".into(), "1".into()]), "Y, now first, stays chosen");
    confirm(&mut app, d).unwrap();
    assert_eq!(replaced(&app), [("Y".to_string(), true), ("Z".to_string(), false)]);
    // Z is chosen and a channel is added after it: Z keeps its place and is still the one.
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "channel", json!("1"));
    app.run("select.saveSelection", json!({"name": "W"})).unwrap();
    confirm(&mut app, d).unwrap();
    assert_eq!(replaced(&app).iter().filter(|(_, r)| *r).map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Y", "Z", "W"]);
    // The second of two "Twin" channels is chosen and the first is deleted, so "Tail" is at its
    // place: the name no longer tells which was chosen, so the selection goes into a new channel
    // and neither the other Twin nor Tail changes.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 20, "mode": "replace"})).unwrap();
    for name in ["Twin", "Twin", "Tail"] {
        app.run("select.saveSelection", json!({"name": name})).unwrap();
    }
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20, "mode": "replace"})).unwrap();
    let d = menu(&mut app, SAVE);
    set(&mut app, d, "channel", json!("4"));
    app.run("channel.delete", json!({"channel": 3})).unwrap();
    confirm(&mut app, d).unwrap();
    let after: Vec<bool> = replaced(&app).into_iter().skip(3).map(|(_, r)| r).collect();
    assert_eq!(after, [false, false, true], "Twin and Tail as they were, the selection in a new channel");
    // Load Selection follows Z too after Y is deleted, rather than loading W.
    app.run("select.deselect", json!({})).unwrap();
    let d = menu(&mut app, LOAD);
    set(&mut app, d, "channel", json!("1"));
    app.run("channel.delete", json!({"channel": "Y"})).unwrap();
    confirm(&mut app, d).unwrap();
    assert_eq!(last_step(&app), Some((LOAD.to_string(), json!({"channel": "Z", "invert": false, "operation": "new"}))));
}

/// OK after the chosen document closed, or after another document became active, is refused with
/// the reason, keeps the dialog open and saves nothing.
#[test]
fn a_document_closed_or_switched_before_ok_is_refused() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    set_document(&mut app, d, 1);
    app.session.set_active(1);
    app.run("file.close", json!({"save": false})).unwrap();
    app.session.set_active(0);
    assert_eq!(confirm(&mut app, d), Err(tl!("The document was closed. Choose Save Selection again.").to_string()));
    assert!(is_open(&app, d), "the dialog stays open with the reason");
    assert!(app.session.documents().iter().all(|st| st.doc.channels.is_empty()), "saved nowhere");
    crate::dialogs::testing::cancel(&mut app, d).unwrap();
    let d = menu(&mut app, SAVE);
    app.run("file.new", json!({"width": 40, "height": 20, "name": "D"})).unwrap();
    assert_eq!(confirm(&mut app, d), Err(tl!("The active document changed. Choose the command again.").to_string()), "the selection is A's");
    assert!(app.session.documents().iter().all(|st| st.doc.channels.is_empty()));
}

/// A save into the active document sends no `document`, so the recorded step is as before.
#[test]
fn the_active_document_is_not_named_in_the_step() {
    let mut app = app();
    let d = menu(&mut app, SAVE);
    assert_eq!(sent(&app, d), json!({"channel": "new", "name": "", "operation": "new"}));
    confirm(&mut app, d).unwrap();
    assert_eq!(last_step(&app), Some((SAVE.to_string(), json!({"channel": "new", "name": "", "operation": "new"}))));
}

/// The dialogs in a harness that runs the dialog host each frame, as the app does.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1000.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h
}

/// The dialog draws Photoshop's fields; the first operation reads Replace Channel once an
/// existing channel is chosen.
#[test]
fn the_dialog_shows_the_document_channel_and_operations() {
    let mut app = app();
    app.run("select.saveSelection", json!({"name": "Mine"})).unwrap();
    let mut h = harness(app);
    let d = menu(h.state_mut(), SAVE);
    h.run_steps(4);
    for label in ["Document", "Channel", "Name", "New Channel", "Add to Channel", "Subtract from Channel", "Intersect with Channel", "OK"] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label} is shown");
    }
    assert!(h.query_all_by_label("Replace Channel").next().is_none());
    set_value(h.state_mut(), d, "channel", json!("0"));
    h.run_steps(4);
    assert!(h.query_all_by_label("Replace Channel").next().is_some());
    assert!(h.query_all_by_label("New Channel").next().is_none());
    assert_eq!(text(h.state(), d, "operation"), "replace");
    crate::dialogs::testing::cancel(h.state_mut(), d).unwrap();
    menu(h.state_mut(), LOAD);
    h.run_steps(4);
    for label in ["Document", "Channel", "Invert", "New Selection", "Add to Selection", "Subtract from Selection", "Intersect with Selection"] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label} is shown");
    }
}

/// Holds Alt, which turns Cancel into Reset, and presses the Reset button.
fn press_reset(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    let reset = h.get_by_label("Reset").rect();
    let click = |pressed| egui::Event::PointerButton { pos: reset.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
    for event in [egui::Event::PointerMoved(reset.center()), click(true), click(false)] {
        h.event(event);
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

/// Alt+Cancel (Reset) puts both dialogs back as they opened, Document included, and leaves them
/// open; OK then runs with the open-time choices.
#[test]
fn alt_reset_restores_the_fields_the_dialog_opened_with() {
    let mut app = app();
    app.session.set_active(1);
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.saveSelection", json!({"name": "Old"})).unwrap();
    app.session.set_active(0);
    let mut h = harness(app);
    let d = menu(h.state_mut(), SAVE);
    h.run_steps(2);
    let opened = fields(h.state(), d);
    let b = id(h.state(), 1);
    set_value(h.state_mut(), d, "document", json!(b));
    h.run_steps(2);
    set_value(h.state_mut(), d, "channel", json!("0"));
    set_value(h.state_mut(), d, "operation", json!("subtract"));
    h.run_steps(2);
    assert_ne!(text(h.state(), d, "document"), opened.text("document").unwrap());
    press_reset(&mut h);
    assert!(is_open(h.state(), d), "Reset leaves the dialog open");
    let f = fields(h.state(), d);
    for k in ["document", "channel", "name", "operation", SEEN] {
        assert_eq!(f.get(k), opened.get(k), "{k}");
    }
    confirm(h.state_mut(), d).unwrap();
    assert_eq!(h.state().session.documents()[0].doc.channels.len(), 1, "saved into a new channel of A");

    h.state_mut().run("select.saveSelection", json!({"name": "Second"})).unwrap();
    let d = menu(h.state_mut(), LOAD);
    h.run_steps(2);
    let opened = fields(h.state(), d);
    assert_eq!(opened.text("channel"), Some("0"));
    set_value(h.state_mut(), d, "channel", json!("1"));
    set_value(h.state_mut(), d, "invert", json!(true));
    set_value(h.state_mut(), d, "operation", json!("intersect"));
    h.run_steps(2);
    press_reset(&mut h);
    assert!(is_open(h.state(), d), "Reset leaves the dialog open");
    let f = fields(h.state(), d);
    for k in ["document", "channel", "invert", "operation", SEEN] {
        assert_eq!(f.get(k), opened.get(k), "{k}");
    }
}
