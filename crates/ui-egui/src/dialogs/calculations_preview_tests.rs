//! The Calculations dialog's live Preview (#14530): a check box on by default that shows the
//! result channel as a grey image over the whole canvas while the fields change, Alt+P and
//! automation toggling it, Cancel leaving the document untouched, and OK writing exactly what the
//! preview showed. The dialog form is opened as its menu item opens it; the canvas side is read
//! through the same entry point the canvas pass itself uses (`ensure_filter_preview`), with
//! previews computed inline (no worker thread) so each pass is deterministic.

use crate::PhotocraftApp;
use crate::canvas::display_doc;
use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};

/// A 4x3 document whose only layer is solid red, so the Red channel reads 1 and the Green channel 0:
/// every blending of the two gives a value the test can name.
fn red_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 4, "height": 3})).expect("a document");
    app.run("edit.fill", json!({"color": "#ff0000"})).expect("fill the layer solid red");
    app.sync_views();
    app
}

/// Image › Calculations as its menu item runs it, then the first pass that loads the form's values.
fn open_calculations(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, "image.calculations", json!({})).expect("the command opens its dialog");
    let want = reply["dialog"].as_str().expect("the reply names a dialog");
    let id =
        app.framework.dialogs.open_dialogs().into_iter().map(|open| open.id).find(|open| open.value().to_string() == want).expect("the named dialog is open");
    crate::dialogs::testing::show(app);
    id
}

/// The open parameter dialog: the Calculations form.
fn hosted(app: &PhotocraftApp) -> &HostedParamDialog {
    let dialog = app.framework.dialogs.get::<HostedParamDialog>().expect("a parameter dialog is open");
    assert_eq!(dialog.key(), Some("image.calculations"), "the open form is the Calculations dialog");
    dialog
}

/// Sets a row of the open form, as a click on that row would.
fn set_field(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|error| panic!("the Calculations form takes {key}: {error}"));
}

/// The document the canvas would show from the open dialog's preview, or None while it shows the
/// plain document.
fn previewed(app: &mut PhotocraftApp) -> Option<photocraft_doc::Document> {
    let ctx = app.ctx().clone();
    crate::canvas::ensure_filter_preview(app, 0, &ctx)?;
    app.filter_preview.as_ref().and_then(|preview| preview.result.clone()).map(|doc| (*doc).clone())
}

/// Every pixel of `doc` as a grey 8-bit value; a pixel with differing colour channels fails here,
/// so "in grey" is checked for every pixel of the whole document.
fn greys(doc: &photocraft_doc::Document) -> Vec<u32> {
    photocraft_compose::flatten(doc)
        .px
        .iter()
        .map(|p| {
            assert!((p[0] - p[1]).abs() < 0.01 && (p[0] - p[2]).abs() < 0.01 && p[3] > 0.99, "grey over the whole document: {p:?}");
            (p[0] * 255.0 + 0.5) as u32
        })
        .collect()
}

/// The Done-when row "the Preview check box, on by default": the framework reports the box only for
/// a dialog that previews, and it starts on, before its first pass answers anything.
#[test]
fn the_calculations_form_has_a_preview_box_on_by_default() {
    let mut app = red_app();
    open_calculations(&mut app);
    assert!(hosted(&app).is_previewing(), "the Calculations form has a Preview check box, on while it opens");
}

/// The measured reference: with Preview on, the canvas shows the result channel as a grey image
/// over the whole document. Both sources name channels of the solid red layer: Green is 0, Red is
/// 1, so Multiply gives black across every pixel.
#[test]
fn preview_shows_the_grey_result_channel_over_the_whole_document() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    set_field(&mut app, id, "blending", json!("multiply"));
    let shown = previewed(&mut app).expect("with the Preview box on the canvas shows the preview");
    assert_eq!([shown.size.width, shown.size.height], [4, 3], "the grey image covers the whole document");
    assert_eq!(greys(&shown), vec![0; 12], "Multiply of Green 0 with Red 1 is black over every pixel");
}

/// The measured reference updates the preview as any field changes; here Blending, Opacity and
/// Invert Source 2 each redraw the grey channel in the same pass.
#[test]
fn every_field_change_redraws_the_grey_preview_live() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    set_field(&mut app, id, "blending", json!("multiply"));
    let multiply = greys(&previewed(&mut app).expect("the opening preview"));
    assert!(multiply.iter().all(|v| *v < 2), "Multiply of 0 with 1 is black: {multiply:?}");

    set_field(&mut app, id, "blending", json!("screen"));
    let screen = greys(&previewed(&mut app).expect("the preview follows the Blending row"));
    assert!(screen.iter().all(|v| *v > 253), "Screen of 0 with 1 is white: {screen:?}");

    set_field(&mut app, id, "opacity", json!(50.0));
    let half = greys(&previewed(&mut app).expect("the preview follows the Opacity row"));
    assert!(half.iter().all(|v| (125..=131).contains(v)), "half opacity stands between the two: {half:?}");

    set_field(&mut app, id, "source2Invert", json!(true));
    let inverted = greys(&previewed(&mut app).expect("the preview follows the Invert Source 2 row"));
    assert!(inverted.iter().all(|v| *v > 253), "inverting Green gives Screen of 1 with 1: {inverted:?}");
}

/// Alt+P toggles the box (the reference application's mnemonic); with the box off the canvas shows
/// the normal image again, and Alt+P turns the preview back on.
#[test]
fn alt_p_toggles_the_preview_and_the_canvas_shows_the_document_without_it() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    assert!(previewed(&mut app).is_some(), "the preview shows while the box is on");

    crate::canvas::testing::key(&mut app, egui::Key::P, egui::Modifiers::ALT);
    assert!(!hosted(&app).is_previewing(), "Alt+P turns the Preview box off");
    assert!(previewed(&mut app).is_none(), "with the box off the preview is gone");
    let (shown, key) = display_doc(&mut app, 0);
    assert_eq!(key, 0, "the canvas draws the document, not a preview");
    assert!(std::sync::Arc::ptr_eq(&shown, &app.session.documents()[0].doc), "the normal image is the document itself");

    crate::canvas::testing::key(&mut app, egui::Key::P, egui::Modifiers::ALT);
    assert!(hosted(&app).is_previewing(), "Alt+P turns the Preview box back on");
    assert!(previewed(&mut app).is_some(), "and the grey channel is on the canvas again");
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some(), "the dialog stayed open across both toggles");
}

/// The automation channel writes the box like any other row, so a script can turn the preview on
/// and off without a keyboard (`ui.dialog.set` with the `preview` field).
#[test]
fn automation_sets_the_preview_box() {
    let mut app = red_app();
    open_calculations(&mut app);
    assert!(hosted(&app).is_previewing(), "on while it opens");
    crate::canvas::testing::control(&mut app, "ui.dialog.set", json!({"dialog": "top", "field": "preview", "value": false}));
    assert!(!hosted(&app).is_previewing(), "the preview row reaches the box");
    assert!(previewed(&mut app).is_none(), "and the canvas shows the plain document");
    crate::canvas::testing::control(&mut app, "ui.dialog.set", json!({"dialog": "top", "field": "preview", "value": true}));
    assert!(previewed(&mut app).is_some(), "a script can turn it back on");
}

/// Neighbour: the other channel-workflow forms the same opener builds keep today's behaviour -
/// Apply Image and Save Selection get no Preview box, and Alt+P does nothing to them.
#[test]
fn the_other_channel_forms_keep_no_preview_box() {
    for command in ["image.applyImage", "select.saveSelection"] {
        let mut app = red_app();
        let ctx = app.ctx().clone();
        let reply = crate::menus::invoke(&mut app, &ctx, command, json!({})).expect("the command opens its dialog");
        let want = reply["dialog"].as_str().expect("the reply names a dialog");
        let id = app
            .framework
            .dialogs
            .open_dialogs()
            .into_iter()
            .map(|open| open.id)
            .find(|open| open.value().to_string() == want)
            .expect("the named dialog is open");
        crate::dialogs::testing::show(&mut app);
        let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the form is open");
        assert_eq!(dialog.key(), Some(command));
        assert!(!dialog.is_previewing(), "{command} has no Preview box");
        crate::canvas::testing::key(&mut app, egui::Key::P, egui::Modifiers::ALT);
        assert!(!app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("still open").is_previewing(), "Alt+P changes nothing in {command}");
        assert_eq!(app.session.active().expect("a document").doc.channels.len(), 0, "{command}: Alt+P wrote nothing");
    }
}

/// Neighbour: another document's canvas never draws the Calculations preview - command dialogs
/// belong to the active document, as with every command preview.
#[test]
fn another_documents_canvas_shows_no_calculations_preview() {
    let mut app = red_app();
    app.run("file.new", json!({"width": 6, "height": 6})).expect("a second document");
    app.sync_views();
    let id = open_calculations(&mut app);
    assert!(hosted(&app).is_previewing(), "the new document's form previews");
    let ctx = app.ctx().clone();
    assert!(crate::canvas::ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the inactive document's canvas shows nothing");
    assert!(crate::canvas::ensure_filter_preview(&mut app, 1, &ctx).is_some(), "the active document's canvas previews");
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some(), "the dialog stayed open");
}

/// The measured reference shows the grey result channel; what the other Result choices preview
/// "beyond" it is not measured, and nothing more is shown: choosing New Document or Selection
/// keeps the same grey channel picture.
#[test]
fn every_result_choice_shows_the_same_grey_channel() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    set_field(&mut app, id, "blending", json!("multiply"));
    for result in ["newDocument", "selection"] {
        set_field(&mut app, id, "result", json!(result));
        let shown = previewed(&mut app).unwrap_or_else(|| panic!("{result}: the canvas still previews the grey channel"));
        assert_eq!(greys(&shown), vec![0; 12], "{result}: the same grey channel picture");
    }
}

/// The Done-when row "Cancel leaves the document, its channels and its history unchanged": even
/// with a preview computed and fields edited, nothing was written.
#[test]
fn cancel_leaves_the_document_its_channels_and_its_history_unchanged() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    set_field(&mut app, id, "blending", json!("screen"));
    let shown = greys(&previewed(&mut app).expect("the preview shows while the dialog is open"));
    assert!(shown.iter().all(|v| *v > 253), "a real preview was computed before Cancel");

    let (channels, history, revision) = {
        let st = app.session.active().expect("a document");
        (st.doc.channels.len(), st.history.entries(), st.revision)
    };
    crate::dialogs::testing::cancel(&mut app, id).expect("Cancel answers the open dialog");
    let st = app.session.active().expect("the document is still open");
    assert_eq!(st.doc.channels.len(), channels, "no channel was added by the preview");
    assert_eq!(st.history.entries(), history, "the preview left no history step");
    assert_eq!(st.revision, revision, "the document itself did not change");
    assert!(previewed(&mut app).is_none(), "the closed dialog shows nothing on the canvas");
}

/// The Done-when row "OK gives the same values as the preview showed and adds exactly one
/// Calculations history step".
#[test]
fn ok_writes_one_calculations_step_with_the_values_the_preview_showed() {
    let mut app = red_app();
    let id = open_calculations(&mut app);
    set_field(&mut app, id, "source1Channel", json!("Red"));
    set_field(&mut app, id, "source2Channel", json!("Green"));
    set_field(&mut app, id, "blending", json!("screen"));
    set_field(&mut app, id, "opacity", json!(50.0));
    let preview = greys(&previewed(&mut app).expect("the preview stands before OK"));
    assert!(preview.iter().all(|v| (125..=131).contains(v)), "the preview holds half-Screen: {preview:?}");

    let history = app.session.active().expect("a document").history.entries();
    crate::dialogs::testing::confirm(&mut app, id).expect("OK runs the command");
    let st = app.session.active().expect("the document is open");
    assert_eq!(st.history.entries().len(), history.len() + 1, "exactly one history step");
    assert_eq!(st.history.entries().last().map(String::as_str), Some("Calculations"), "the step is named Calculations");
    assert_eq!(st.doc.channels.len(), 1, "OK adds the result channel the preview showed");
    let got = st.doc.channels[0].surface.read_region(st.doc.bounds());
    for (i, grey) in preview.iter().enumerate() {
        let want = *grey as f32 / 255.0;
        assert!((got[i] - want).abs() <= 1.5 / 255.0, "OK gives the value the preview showed at {i}: {} vs {want}", got[i]);
    }
    assert_eq!(st.doc.channels[0].name, "Alpha 1", "the next free channel name");
    assert!(previewed(&mut app).is_none(), "the preview is gone once OK has run");
}
