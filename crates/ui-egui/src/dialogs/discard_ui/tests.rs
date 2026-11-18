use super::*;
use craft::host::{Files, Picked, ScriptedFiles};
use craft::ui::dialog::DialogId;
// The five format test modules below reach `Key` through this glob.
use egui::Key;
use std::sync::Arc;

fn app_with_docs(n: usize) -> PhotocraftApp {
    app_with_docs_over(n, None)
}

/// `n` new documents in an app whose host serves `files`: the host takes them when the app is made, so a
/// later `services.files` assignment would not reach the pickers.
fn app_with_docs_over(n: usize, files: Option<Arc<dyn Files>>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files, ..Default::default() });
    for _ in 0..n {
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    }
    app.sync_views();
    app
}

fn make_dirty(app: &mut PhotocraftApp, doc: usize) {
    app.session.set_active(doc);
    app.run("layer.new.layer", json!({})).unwrap();
}

fn doc_id(app: &PhotocraftApp, i: usize) -> DocId {
    app.session.documents()[i].doc.id
}

#[test]
fn a_save_document_script_that_edits_the_document_keeps_the_prompt_up() {
    use photocraft_engine::automate_cmds::ScriptBinding;
    let mut app = app_with_docs(1);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/kept.psd".into());
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
    app.session.edit_prefs(|prefs| {
        prefs.script_events.enabled = true;
        prefs.script_events.bindings = vec![ScriptBinding {
            event: "saveDocument".into(),
            script: None,
            steps: Some(json!([["layer.new.layer", {"name": "From the script"}]])),
            name: "Binding".into(),
        }];
    });
    let doc = doc_id(&app, 0);
    assert!(!save(&mut app, &ctx, doc), "the script's layer was never written");
    assert!(app.session.documents()[0].is_dirty());
    // Without the script the same save releases the prompt.
    app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    assert!(save(&mut app, &ctx, doc));
}

#[test]
fn clean_documents_close_without_asking() {
    let mut app = app_with_docs(2);
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn closing_a_dirty_document_waits_for_an_answer() {
    let mut app = app_with_docs(2);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 1);
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 1})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "nothing closes before the user answers");
    assert!(app.discard.is_some());
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn only_documents_that_would_be_discarded_are_asked_about() {
    let mut app = app_with_docs(3);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    let ids = [doc_id(&app, 0), doc_id(&app, 2)];
    assert_eq!(discarded(&app, "file.closeOthers", &json!({"document": 0})), (Some(ids[0]), vec![ids[1]]));
    assert_eq!(discarded(&app, "file.closeAll", &json!({})).1, ids);
    assert_eq!(discarded(&app, EXIT, &json!({})).1, ids);
    assert!(discarded(&app, "view.zoomIn", &json!({})).1.is_empty());
}

#[test]
fn close_all_asks_per_dirty_document_then_runs() {
    let mut app = app_with_docs(3);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    crate::menus::invoke(&mut app, &ctx, "file.closeAll", json!({})).unwrap();
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 2, "the answered document closes; still waiting on the second");
    advance(&mut app, &ctx);
    assert!(app.session.documents().is_empty());
}

/// A document that became dirty while the prompt was up is asked about before the final close (#4954).
#[test]
fn a_document_dirtied_during_the_prompt_is_asked_about_before_the_final_close() {
    for command in ["file.closeAll", EXIT] {
        let mut app = app_with_docs(2);
        let ctx = app.ctx().clone();
        make_dirty(&mut app, 0);
        let late = doc_id(&app, 1);
        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        make_dirty(&mut app, 1);
        advance(&mut app, &ctx);
        assert_eq!(app.discard.as_ref().map(|p| p.docs.clone()), Some(vec![late]), "{command}: asks about the newly dirty document");
        assert!(!app.allow_close, "{command}");
        assert_eq!(app.session.documents().len(), 1, "{command}: it is not discarded unasked");
        advance(&mut app, &ctx);
        assert!(app.discard.is_none(), "{command}");
    }
    // Close Others asks about the others too, but still not about the document it keeps.
    let mut app = app_with_docs(3);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 0);
    let (late, kept) = (doc_id(&app, 1), doc_id(&app, 2));
    crate::menus::invoke(&mut app, &ctx, "file.closeOthers", json!({"document": 2})).unwrap();
    make_dirty(&mut app, 1);
    make_dirty(&mut app, 2);
    advance(&mut app, &ctx);
    assert_eq!(app.discard.as_ref().map(|p| p.docs.clone()), Some(vec![late]), "file.closeOthers: asks about the newly dirty other document");
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().iter().map(|d| d.doc.id).collect::<Vec<_>>(), [kept]);
}

#[test]
fn close_others_closes_each_answered_document_and_keeps_its_target() {
    let mut app = app_with_docs(3);
    let ctx = app.ctx().clone();
    (0..3).for_each(|i| make_dirty(&mut app, i));
    let kept = doc_id(&app, 1);
    crate::menus::invoke(&mut app, &ctx, "file.closeOthers", json!({"document": 1})).unwrap();
    assert_eq!(app.session.active_index(), Some(0), "the first one asked about is shown");
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().map(|d| d.doc.id), Some(doc_id(&app, 1)), "the next one asked about is shown");
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().iter().map(|d| d.doc.id).collect::<Vec<_>>(), [kept]);
}

#[test]
fn the_parked_close_still_hits_its_document_after_tabs_move() {
    let mut app = app_with_docs(3);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 2);
    let doomed = doc_id(&app, 2);
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 2})).unwrap();
    // Another document closes while the prompt is up, shifting the tab to index 1.
    app.run("file.close", json!({"document": 0})).unwrap();
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert!(index_of(&app, doomed).is_none());
}

#[test]
fn quitting_replaces_a_pending_close_but_other_requests_are_refused() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    assert!(intercept(&mut app, "file.close", &json!({"document": 0})));
    assert!(intercept(&mut app, "file.closeAll", &Value::Null));
    assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some(("file.close", 1)));
    assert!(app.ui.status_error);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some((EXIT, 2)));
    // Pressing the window's X again must not restart the questions.
    app.discard.as_mut().unwrap().docs.remove(0);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert_eq!(app.discard.as_ref().unwrap().docs.len(), 1);
}

#[test]
fn exit_is_held_back_until_confirmed() {
    let mut app = app_with_docs(1);
    let ctx = app.ctx().clone();
    make_dirty(&mut app, 0);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert!(!app.allow_close);
    advance(&mut app, &ctx);
    assert!(app.allow_close);
}

type Prompted = egui_kittest::Harness<'static, PhotocraftApp>;

/// The unsaved-changes prompt for `command` over `app`'s dirty documents, as `os` draws it (with
/// the file dialogs answered at the end of each frame, as the app does).
fn prompt_for(app: PhotocraftApp, command: &str, os: craft::host::target::Os) -> Prompted {
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            // As `ui_frame` starts: a pass carrying injected input acts for the automation client.
            app.automation_input = craft::ui::runtime::windows::WindowHost::of(ui.ctx()).injected_pass();
            crate::dialogs::run(app, ui);
            app.poll_file_dialog();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    craft::ui::testing::windows::set_os(&h.ctx, os);
    assert!(intercept(h.state_mut(), command, &Value::Null));
    // The first pass opens the question, the second draws it, the third shows it to the access tree.
    h.run_steps(3);
    h
}

/// The unsaved-changes prompt for Close All over two dirty documents, as `os` draws it.
fn prompt_on(os: craft::host::target::Os) -> Prompted {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    prompt_for(app, "file.closeAll", os)
}

fn docs_left(h: &Prompted) -> Option<usize> {
    h.state().discard.as_ref().map(|p| p.docs.len())
}

/// Whether the question is on the dialog host.
fn question_open(h: &Prompted) -> bool {
    h.state().framework.dialogs.is_open::<UnsavedChangesDialog>()
}

/// The open question's host id.
fn question_id(h: &Prompted) -> DialogId {
    h.state().framework.dialogs.open_dialogs().last().expect("the question is open").id
}

/// The open question's fields, as `ui.inspect` lists them.
fn question_fields(h: &Prompted) -> Value {
    h.state().framework.dialogs.open_dialogs().last().and_then(|d| d.fields.clone()).unwrap_or_default()
}

/// Answers the open question as its button would (`choice`: `save`, `dontSave` or `cancel`, with
/// the Apply to All box ticked when `all`), then runs the passes that take the answer.
fn answer(h: &mut Prompted, choice: &str, all: bool) {
    let (ctx, id) = (h.ctx.clone(), question_id(h));
    let host = &mut h.state_mut().framework.dialogs;
    host.set_json_field(&ctx, id, "choice", json!(choice)).unwrap();
    if all {
        host.set_json_field(&ctx, id, "applyToAll", json!(true)).unwrap();
    }
    crate::dialogs::confirm(h.state_mut(), &ctx, id).expect("the question takes the answer");
    h.run_steps(2);
}

/// A control request on `h`'s app, answered at once.
fn control(h: &mut Prompted, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(method, params);
    match crate::control::handle(h.state_mut(), &ctx, &req) {
        crate::control::Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// The id `ui.inspect` lists the open question under.
fn listed_id(h: &mut Prompted) -> Value {
    let listed = control(h, "ui.inspect", json!({}))["result"]["dialogs"].clone();
    listed.as_array().and_then(|dialogs| dialogs.last()).map(|d| d["id"].clone()).expect("ui.inspect lists the question")
}

/// "Apply to All" (a checkbox, as in Photoshop) gives the answer to every document still to ask about.
#[test]
fn apply_to_all_discards_the_rest_without_asking_again() {
    let mut h = prompt_on(craft::host::target::Os::Windows);
    answer(&mut h, "dontSave", true);
    h.run_steps(8);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty(), "No was applied to both unsaved documents");
}

#[test]
fn apply_to_all_is_only_offered_while_several_documents_are_left() {
    let mut one_dirty = app_with_docs(2);
    make_dirty(&mut one_dirty, 0);
    let h = prompt_for(one_dirty, "file.closeAll", craft::host::target::Os::Windows);
    assert!(question_fields(&h).get("applyToAll").is_none(), "only one document is unsaved");
    let h = prompt_on(craft::host::target::Os::Windows);
    assert!(question_fields(&h).get("applyToAll").is_some());
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let h = prompt_for(app, "file.revert", craft::host::target::Os::Windows);
    assert!(question_fields(&h).get("applyToAll").is_none(), "a revert never offers it");
}

/// A save that was cancelled or failed brings its document back: asking again beats retrying forever.
#[test]
fn a_document_that_comes_back_after_apply_to_all_is_asked_about_again() {
    let mut h = prompt_on(craft::host::target::Os::Windows);
    let first = doc_id(h.state(), 0);
    if let Some(p) = h.state_mut().discard.as_mut() {
        (p.auto, p.auto_answered) = (Some(Answer::Save), Some(first));
    }
    h.run_steps(3);
    let prompt = h.state().discard.as_ref().unwrap();
    assert!(prompt.auto.is_none());
    assert_eq!(docs_left(&h), Some(2), "both documents are still waiting for an answer");
    assert!(question_open(&h), "the question is up again");
}

/// Apply to All answers for the documents the prompt opened with, not for ones dirtied since.
#[test]
fn a_document_dirtied_after_apply_to_all_is_asked_about() {
    let mut app = app_with_docs(3);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let mut h = prompt_for(app, "file.closeAll", craft::host::target::Os::Windows);
    let late = doc_id(h.state(), 2);
    make_dirty(h.state_mut(), 2);
    if let Some(p) = h.state_mut().discard.as_mut() {
        p.auto = Some(Answer::Discard);
    }
    h.run_steps(8);
    let prompt = h.state().discard.as_ref().unwrap();
    assert_eq!(prompt.docs, vec![late], "the two unsaved documents were discarded, the late one is asked about");
    assert!(prompt.auto.is_none());
    assert_eq!(h.state().session.documents().len(), 1);
    assert!(question_open(&h), "the late document gets its question");
}

/// A pass applies only the result stored for it: a second pass in the same frame finds none, so an
/// answer closes one document, not two.
#[test]
fn a_second_pass_in_one_frame_applies_an_answer_once() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            crate::dialogs::run(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    assert!(intercept(h.state_mut(), "file.closeAll", &Value::Null));
    h.run_steps(2);
    answer(&mut h, "dontSave", false);
    assert_eq!(docs_left(&h), Some(1), "the answer was applied once");
    assert_eq!(h.state().session.documents().len(), 1);
}

#[test]
fn control_clients_can_answer_for_all() {
    let mut app = app_with_docs(3);
    for i in 0..3 {
        make_dirty(&mut app, i);
    }
    let mut h = prompt_for(app, "file.closeAll", craft::host::target::Os::Windows);
    let id = listed_id(&mut h);
    let reply = control(&mut h, "ui.dialog.confirm", json!({"dialog": id, "choice": "dontSaveAll"}));
    assert_eq!(reply["ok"], true, "{reply}");
    h.run_steps(8);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}

/// A client answering one document isn't overruled by an Apply to All whose save just failed.
#[test]
fn a_control_answer_clears_a_stale_apply_to_all() {
    let mut app = app_with_docs(3);
    for i in 0..3 {
        make_dirty(&mut app, i);
    }
    let mut h = prompt_for(app, "file.closeAll", craft::host::target::Os::Windows);
    let (first, last) = (doc_id(h.state(), 0), doc_id(h.state(), 2));
    if let Some(p) = h.state_mut().discard.as_mut() {
        (p.auto, p.auto_answered) = (Some(Answer::Discard), Some(first));
    }
    let id = listed_id(&mut h);
    let reply = control(&mut h, "ui.dialog.confirm", json!({"dialog": id, "choice": "dontSave"}));
    assert_eq!(reply["ok"], true, "{reply}");
    h.run_steps(8);
    let prompt = h.state().discard.as_ref().unwrap();
    assert_eq!(prompt.docs.len(), 2, "only the answered document was closed");
    assert!(prompt.docs.contains(&last) && prompt.auto.is_none());
}

#[test]
fn the_all_answers_need_more_than_one_document_left() {
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    let mut h = prompt_for(app, "file.close", craft::host::target::Os::Windows);
    let id = listed_id(&mut h);
    let reply = control(&mut h, "ui.dialog.confirm", json!({"dialog": id, "choice": "saveAll"}));
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(h.state().discard.is_some(), "a refused answer leaves the prompt up");
    assert!(question_open(&h));
}

/// Whether the open question is about the document `id`.
fn asks_about(h: &Prompted, id: DocId) -> bool {
    question_open(h) && h.state().discard.as_ref().and_then(|p| p.asked) == Some(id)
}

/// No on one of several documents closes it and asks about the next one (#2547).
#[test]
fn answering_no_closes_that_document_and_asks_about_the_next() {
    for command in [EXIT, "file.closeAll"] {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        let second = doc_id(&app, 1);
        let first = doc_id(&app, 0);
        assert_eq!(app.session.active_index(), Some(1), "the last new document is active");
        let mut h = prompt_for(app, command, craft::host::target::Os::Windows);
        assert!(asks_about(&h, first), "{command}");
        assert_eq!(h.state().session.active().map(|d| d.doc.id), Some(first), "{command}: the first tab asked about is shown");
        answer(&mut h, "dontSave", false);
        assert_eq!(h.state().session.documents().len(), 1, "{command}: No closed the first document");
        assert_eq!(doc_id(h.state(), 0), second);
        assert_eq!(h.state().session.active_index(), Some(0), "{command}: the tab asked about is shown");
        assert!(asks_about(&h, second), "{command}: then about the second");
        assert!(!h.state().allow_close);
        answer(&mut h, "dontSave", false);
        assert!(h.state().discard.is_none());
        if command == EXIT {
            assert!(h.state().allow_close, "answering the last document lets the app quit");
        } else {
            assert!(h.state().session.documents().is_empty());
        }
    }
}

#[test]
fn cancel_after_no_keeps_the_unanswered_documents_and_the_app_open() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let second = doc_id(&app, 1);
    let mut h = prompt_for(app, EXIT, craft::host::target::Os::Windows);
    answer(&mut h, "dontSave", false);
    answer(&mut h, "cancel", false);
    assert!(h.state().discard.is_none());
    assert!(!question_open(&h), "Cancel closes the question");
    assert!(!h.state().allow_close, "Cancel aborts the quit");
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(doc_id(h.state(), 0), second);
    assert!(h.state().session.documents()[0].is_dirty(), "the unanswered document keeps its changes");
}

/// Quitting over a pending close replaces the question: the old one closes and the new one asks in
/// the quit wording, so one answer never reaches the wrong prompt.
#[test]
fn quitting_over_a_pending_close_replaces_the_open_question() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let mut h = prompt_for(app, "file.close", craft::host::target::Os::Windows);
    assert!(question_open(&h));
    assert!(intercept(h.state_mut(), EXIT, &Value::Null));
    h.run_steps(3);
    assert!(question_open(&h));
    assert_eq!(h.state().discard.as_ref().map(|p| p.id.as_str()), Some(EXIT));
    assert_eq!(docs_left(&h), Some(2));
    assert!(question_fields(&h).get("applyToAll").is_some(), "the new question counts both documents");
}

/// Beneath the prompt the shortcuts hold their keys, so ⌘D with a selection on the document
/// answers Don't Save rather than running Select › Deselect (or refusing it in the status bar).
#[test]
fn command_d_answers_the_prompt_rather_than_deselecting() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    app.run("select.all", json!({})).unwrap();
    let selected = app.session.active().map(|st| st.doc.id).unwrap();
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            crate::shortcuts::handle(app, ui.ctx());
            crate::dialogs::run(app, ui);
            app.poll_file_dialog();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    craft::ui::testing::windows::set_os(&h.ctx, craft::host::target::Os::Mac);
    assert!(intercept(h.state_mut(), "file.close", &json!({"document": 0})));
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::D);
    h.run_steps(2);
    assert!(h.state().discard.is_none(), "⌘D answered the prompt");
    assert_eq!(h.state().session.documents().len(), 1, "the asked-about document closed unsaved");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let kept = h.state().session.documents().iter().find(|d| d.doc.id == selected);
    assert!(kept.is_some_and(|d| d.doc.selection.is_some()), "the selection on the other document is still there");
}

#[test]
fn saving_from_the_prompt_waits_for_the_save_dialog_then_closes() {
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let files = Arc::new(ScriptedFiles::held());
    let mut app = app_with_docs_over(1, Some(files.clone()));
    make_dirty(&mut app, 0);
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    let mut h = prompt_for(app, "file.close", craft::host::target::Os::Windows);
    // Cancelling the save dialog brings the question back.
    answer(&mut h, "save", false);
    h.run_steps(3);
    assert!(h.state().file_dialog_open());
    assert!(!question_open(&h), "the question waits behind the dialog");
    files.answer(Picked::Cancelled);
    h.run_steps(3);
    assert_eq!(docs_left(&h), Some(1));
    assert!(question_open(&h), "the question comes back");
    // Saving closes the document.
    answer(&mut h, "save", false);
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 1, "still open while the dialog is");
    files.answer(Picked::Paths(vec!["/pics/kept.psd".into()]));
    h.run_steps(3);
    assert_eq!(*written.borrow(), ["/pics/kept.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}

#[test]
fn saving_from_the_prompt_waits_for_a_background_save_then_closes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let (gate, written) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::<String>::new())));
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/big.psd".into());
    app.background_jobs = true;
    let (g, w) = (gate.clone(), written.clone());
    app.services.save_file = Some(Arc::new(move |_, path, _, ctx| {
        while !g.load(Ordering::Relaxed) {
            ctx.check().map_err(|e| e.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        w.lock().unwrap().push(path.to_string());
        Ok(Vec::new())
    }));
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            crate::jobs_ui::tick(app, ui.ctx());
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    craft::ui::testing::windows::set_os(&h.ctx, craft::host::target::Os::Windows);
    assert!(intercept(h.state_mut(), "file.close", &Value::Null));
    h.run_steps(2);
    answer(&mut h, "save", false);
    h.run_steps(3);
    assert!(h.state().saving());
    assert!(!question_open(&h), "the question waits for the save");
    assert_eq!(h.state().session.documents().len(), 1, "still open while saving");
    gate.store(true, Ordering::Relaxed);
    let t = std::time::Instant::now();
    while h.state().saving() && t.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    assert_eq!(*written.lock().unwrap(), ["/pics/big.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty(), "closed once saved");
}

/// #6414: on the web a save stays dirty (the download isn't confirmed), and answering Save
/// still closes the tab after one write.
#[test]
fn saving_from_the_prompt_closes_after_an_unconfirmed_write() {
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/kept.psd".into());
    app.services.write_unconfirmed = true;
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    let mut h = prompt_for(app, "file.close", craft::host::target::Os::Windows);
    answer(&mut h, "save", false);
    h.run_steps(3);
    assert_eq!(*written.borrow(), ["/pics/kept.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty(), "closed after one write");
}

/// Synthetic input pressing the prompt's Save key (Y) reaches the hosted question and is judged
/// as `file.save`: a policy that refuses it writes nothing, keeps the prompt and the document,
/// and reports the refusal in the status bar.
#[test]
fn a_synthetic_save_press_on_the_hosted_prompt_is_refused_and_reported() {
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/kept.psd".into());
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    app.services.automation_command =
        Some(Box::new(|id: &str, _: &Value| if id == "file.save" { Err(format!("automation command `{id}` is disabled")) } else { Ok(()) }));
    let mut h = prompt_for(app, "file.close", craft::host::target::Os::Windows);
    // Y as synthetic input: the window host injects it, and the passes carrying it act for the
    // automation client.
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.key", serde_json::json!({"key": "Y"}));
    assert!(matches!(crate::control::handle(h.state_mut(), &ctx, &req), crate::control::Reply::AfterInput));
    h.run_steps(4);
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert_eq!(docs_left(&h), Some(1), "the prompt stays up");
    assert!(question_open(&h), "the question comes back");
    assert_eq!(h.state().session.documents().len(), 1, "the document stays open");
    assert!(h.state().ui.status.contains("file.save"), "status: {:?}", h.state().ui.status);
    assert!(h.state().ui.status_error, "the refusal is shown as an error");
}

#[path = "tests/tiff_tests.rs"]
mod tiff_tests;

#[path = "tests/tiff_options_tests.rs"]
mod tiff_options_tests;

#[path = "tests/targa_tests.rs"]
mod targa_tests;

#[path = "tests/dds_tests.rs"]
mod dds_tests;

#[path = "tests/jpeg_tests.rs"]
mod jpeg_tests;

/// One frame with the window's close button pressed; the commands the close path sent.
fn window_close_commands(app: &mut PhotocraftApp) -> Vec<egui::ViewportCommand> {
    let mut info = egui::ViewportInfo::default();
    info.events.push(egui::ViewportEvent::Close);
    let mut input = egui::RawInput::default();
    input.viewports.insert(egui::ViewportId::ROOT, info);
    let mut out = crate::app::framework::close_path_frame(app, input);
    out.textures_delta.clear();
    out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default()
}

/// One frame with the window's close button pressed; whether the guard cancelled the close.
fn press_window_close(app: &mut PhotocraftApp) -> bool {
    let commands = &window_close_commands(app);
    let cancelled = commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose));
    assert_eq!(commands.iter().any(|c| matches!(c, egui::ViewportCommand::Focus)), cancelled);
    cancelled
}

#[test]
fn window_close_is_cancelled_while_work_is_unsaved() {
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    assert!(press_window_close(&mut app));
    assert!(app.discard.is_some());
}

/// After the window's GPU device is lost nothing can be drawn, so a prompt could never be seen
/// or answered. While unsaved work can't be written for recovery (the save keeps retrying), the
/// close button must still quit, instead of opening a prompt that never shows.
#[test]
fn window_close_goes_through_once_the_window_device_is_lost() {
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.window_lost = true;
    assert!(!press_window_close(&mut app), "the close isn't cancelled");
    assert!(app.discard.is_none(), "no prompt opened");
}

/// The title bar's close button and File › Exit run `file.exit` through `menus::invoke`, which
/// parks it behind the prompt before any close request is made: once the window's device is
/// lost they must close instead.
#[test]
fn file_exit_quits_without_a_prompt_once_the_window_device_is_lost() {
    let exit = |app: &mut PhotocraftApp| {
        let ctx = app.ctx().clone();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = crate::menus::invoke(app, ui.ctx(), EXIT, json!({}));
        });
        out.textures_delta.clear();
        let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
        commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close))
    };
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    assert!(!exit(&mut app), "a window that can draw asks first");
    assert!(app.discard.is_some());
    app.discard = None;
    app.window_lost = true;
    assert!(exit(&mut app), "the window is asked to close");
    assert!(app.discard.is_none(), "no prompt opened");
}

#[test]
fn unsaved_work_is_any_dirty_document_until_closing_is_confirmed() {
    let mut app = app_with_docs(2);
    assert!(!app.has_unsaved_work());
    make_dirty(&mut app, 1);
    assert!(app.has_unsaved_work(), "a dirty document that isn't the active one counts");
    app.allow_close = true;
    assert!(!app.has_unsaved_work());
}

#[test]
fn window_close_goes_through_when_nothing_is_unsaved_or_it_was_confirmed() {
    let mut app = app_with_docs(1);
    assert!(!press_window_close(&mut app));
    make_dirty(&mut app, 0);
    app.allow_close = true;
    assert!(!press_window_close(&mut app));
    assert!(app.discard.is_none());
}
