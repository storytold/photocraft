use super::*;
use crate::{Services, menus};
use std::cell::RefCell;
use std::rc::Rc;

/// The requests the fake dialog service was given, with their replies: dialogs the user is still in.
type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app with an untitled document, whose file dialogs stay open until the test answers them, and
/// whose writer records the paths it wrote.
fn app() -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (open, written): (Open, Rc<RefCell<Vec<String>>>) = Default::default();
    let (held, w) = (open.clone(), written.clone());
    let services = Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, open, written)
}

/// The user answers the open dialog, from the dialog's own thread.
fn answer(open: &Open, answer: Option<FileDialogAnswer>) {
    let (_, reply) = open.borrow_mut().pop().expect("a dialog is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
}

#[test]
fn the_app_keeps_running_while_a_dialog_is_open() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap(), json!({"fileDialog": "save"}));
    assert!(open.borrow().is_empty(), "shown at the end of the frame, which has the window to parent it to");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested.ends_with(".psd")));
    // Frames go on while the user is in the dialog: nothing is written, commands still run, and
    // a second dialog is refused.
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
    assert!(written.borrow().is_empty());
    assert!(app.file_dialog_open());
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err(), "a file dialog is already open");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1);
    // The answer wakes the app, and the save goes on from where it asked.
    for _ in 0..4 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "idle");
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    assert!(ctx.has_requested_repaint());
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/a.psd"));
    assert!(!st.is_dirty());
    assert!(!app.file_dialog_open());
}

#[test]
fn the_answer_goes_to_the_document_it_was_asked_for() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.session.set_active(1);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let paths: Vec<_> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/pics/first.psd".to_string()), None]);
    assert_eq!(app.session.active_index(), Some(0));
    // Once that document is closed, the answer has nothing to save.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.close", json!({"document": 0})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/again.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/first.psd"]);
    assert!(app.ui.status_error && app.session.documents()[0].path.is_none());
}

#[test]
fn cancel_and_a_dialog_that_could_not_show_end_quietly() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    // A reply dropped unanswered (the dialog failed to show) is a Cancel, not a dialog left open forever.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    open.borrow_mut().clear();
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    assert!(written.borrow().is_empty());
    // Without a dialog service, asking is a Cancel straight away.
    app.services.file_dialog = None;
    assert_eq!(app.open_dialog_file().unwrap_err(), CANCELLED);
    assert!(!app.file_dialog_open());
}

#[test]
fn an_answer_of_the_wrong_kind_is_an_error_not_a_crash() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Paths(vec!["/pics/a.psd".into()])));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error && app.ui.status == UNEXPECTED);
    assert!(written.borrow().is_empty());
    app.open_dialog_file().unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.session.documents().len(), 1);
}
