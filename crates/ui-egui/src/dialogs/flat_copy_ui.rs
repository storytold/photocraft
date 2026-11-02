//! The question File › Save As asks before writing a file that can't keep the document: its layers,
//! alpha channels, saved paths, layer comps or Quick Mask. Such a save is a copy, and the document stays
//! unsaved, which a notice in the corner was too easy to miss. Like TIFF Options: the save is
//! parked in [`Prompt`] while the modal is up, "Don't show again" clears the preference, and Cancel
//! drops the save. Only the save dialog asks: a command with a path never waits on a person.

use crate::{FormatChoices, PhotocraftApp};

/// A save waiting on the answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the copy goes.
    pub path: String,
    choices: FormatChoices,
    /// File › Save back to the document's own file, recorded as one so a replay doesn't write
    /// this path.
    plain_save: bool,
    /// Clears `ask_before_saving_flat_copy` when the save goes ahead.
    pub dont_ask_again: bool,
}

/// Whether saving the active document to `path` should go through the prompt.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    app.session.prefs().file_handling.ask_before_saving_flat_copy && app.save_loses_content(path)
}

/// Parks the save behind the prompt.
pub fn park(app: &mut PhotocraftApp, path: String, choices: FormatChoices, plain_save: bool) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    app.flat_copy = Some(Prompt { doc, path, choices, plain_save, dont_ask_again: false });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Answered and written, or the file's own options were cancelled.
    if !app.save_options_open() {
        app.flat_copy_confirmed = None;
    }
    let Some(p) = &mut app.flat_copy else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut dont_ask = p.dont_ask_again;
    let name = crate::file_open::display_name(&p.path);
    let message = crate::i18n::fmt(
        tl!(
            "“{name}” can't keep this document's layers, alpha channels, saved paths, layer comps or Quick Mask. The document stays open, with its changes unsaved."
        ),
        &[("name", name.as_str())],
    );
    let modal = egui::Modal::new(egui::Id::unique("flat-copy")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("Save a flat copy?")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(message);
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, &mut dont_ask, tl!("Don't show again"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.dont_ask_again = dont_ask;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        let Some(p) = app.flat_copy.take() else { return };
        if p.dont_ask_again {
            app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_flat_copy = false);
        }
        // Answered: the file's own options come next, as if the dialog had just closed.
        app.flat_copy_confirmed = Some(p.path.clone());
        let path = p.path.clone();
        let from = app.session.journal.len();
        let written = app.with_document(p.doc, |app| app.save_to(p.path, p.choices));
        if written.is_ok() && p.plain_save {
            app.journal_as_plain_save(&path, from);
        }
        if let Err(e) = written {
            app.flat_copy_confirmed = None;
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
    } else if cancel {
        app.flat_copy = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileDialogAnswer, Services};
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use std::{cell::RefCell, rc::Rc};

    type Writes = Rc<RefCell<Vec<String>>>;

    /// A layered document; the save dialog answers `to`.
    fn harness(to: &str) -> (egui_kittest::Harness<'static, PhotocraftApp>, Writes) {
        let writes: Writes = Rc::default();
        let written = writes.clone();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-1"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
        app.services.export = Some(Box::new(|_, _, _| Ok((vec![1], Vec::new()))));
        app.services.write = Some(Box::new(move |path, _| {
            written.borrow_mut().push(path.to_string());
            Ok(())
        }));
        let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
            |ui, app| {
                show(app, ui.ctx());
                crate::jpeg_options_ui::show(app, ui.ctx());
                app.poll_file_dialog(ui.ctx(), None);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        (h, writes)
    }

    fn unsaved(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> bool {
        let st = h.state().session.active().unwrap();
        st.revision != st.saved_revision && st.path.is_none()
    }

    #[test]
    fn a_flat_save_asks_first_and_writes_a_copy() {
        let (mut h, writes) = harness("flat.png");
        h.state_mut().run("channel.new", json!({"name": "Alpha 1"})).unwrap();
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        assert!(h.state().flat_copy.is_some(), "it asks");
        assert!(writes.borrow().is_empty(), "nothing is written before the answer");
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().flat_copy.is_none());
        assert_eq!(*writes.borrow(), ["flat.png"]);
        assert!(unsaved(&h), "the document stays unsaved");
        let doc = h.state().active_doc_id().unwrap();
        assert!(h.state().only_saved_flat(doc), "and the close prompt can say so");
        assert!(h.state().ui.notices.iter().all(|n| n.title != "Saving a copy"), "the question said it already");
    }

    #[test]
    fn cancel_writes_nothing() {
        let (mut h, writes) = harness("flat.png");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().flat_copy.is_none() && writes.borrow().is_empty());
        assert_eq!(h.state().ui.status, "Save cancelled");
    }

    #[test]
    fn dont_show_again_clears_the_preference() {
        let (mut h, writes) = harness("flat.png");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.get_by_label("Don't show again").click();
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(!h.state().session.prefs().file_handling.ask_before_saving_flat_copy);
        assert_eq!(*writes.borrow(), ["flat.png"]);
    }

    #[test]
    fn a_save_that_keeps_everything_does_not_ask() {
        let (mut h, writes) = harness("art.psd");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        assert!(h.state().flat_copy.is_none());
        assert_eq!(*writes.borrow(), ["art.psd"]);
        assert!(!h.state().only_saved_flat(h.state().active_doc_id().unwrap()));
    }

    /// A real save ends what the close prompt says about the copy before it.
    #[test]
    fn a_real_save_clears_the_flat_copy_note() {
        let (mut h, _) = harness("flat.png");
        h.state_mut().session.edit_prefs(|p| p.file_handling.ask_before_saving_flat_copy = false);
        h.state_mut().save_as(Some("flat.png".into())).unwrap();
        let doc = h.state().active_doc_id().unwrap();
        assert!(h.state().only_saved_flat(doc));
        h.state_mut().save_as(Some("art.psd".into())).unwrap();
        assert!(!h.state().only_saved_flat(doc));
    }

    /// A JPEG finishes through its own options, after the question.
    #[test]
    fn a_flat_jpeg_is_remembered_once_its_options_are_answered() {
        let (mut h, writes) = harness("flat.jpg");
        h.state_mut().run("channel.new", json!({"name": "Alpha 1"})).unwrap();
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().jpeg_options.is_some() && writes.borrow().is_empty());
        let doc = h.state().active_doc_id().unwrap();
        assert!(!h.state().only_saved_flat(doc), "nothing is written yet");
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(*writes.borrow(), ["flat.jpg"]);
        assert!(h.state().only_saved_flat(doc), "the close prompt can say so");
        assert!(h.state().ui.notices.iter().all(|n| n.title != "Saving a copy"), "the question said it already");
        assert_eq!(h.state().flat_copy_confirmed, None);
    }

    #[test]
    fn cancelling_the_jpeg_options_leaves_no_trace_of_the_save() {
        let (mut h, writes) = harness("flat.jpg");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        let doc = h.state().active_doc_id().unwrap();
        assert!(writes.borrow().is_empty() && !h.state().only_saved_flat(doc));
        assert_eq!(h.state().flat_copy_confirmed, None, "a later save still gets its notice");
    }

    /// With the question turned off, the file's own options still end in the notice.
    #[test]
    fn a_flat_jpeg_says_so_when_the_question_is_off() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().session.edit_prefs(|p| p.file_handling.ask_before_saving_flat_copy = false);
        h.state_mut().run("channel.new", json!({"name": "Alpha 1"})).unwrap();
        h.state_mut().save_as(Some("flat.jpg".into())).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().ui.notices.iter().any(|n| n.title == "Saving a copy"));
        assert!(h.state().only_saved_flat(h.state().active_doc_id().unwrap()));
    }

    #[test]
    fn the_question_names_the_quick_mask() {
        let (mut h, _) = harness("flat.png");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.get_by_label_contains("Quick Mask");
    }

    /// File › Save onto the document's own flat file stays a plain Save in a recording, so a replay
    /// on another document doesn't write this one's path.
    #[test]
    fn saving_over_its_own_flat_file_records_a_plain_save() {
        let (mut h, writes) = harness("own.png");
        h.state_mut().session.active_mut().unwrap().path = Some("own.png".into());
        h.state_mut().save_dialog_flow(None, FormatChoices::default(), true).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(*writes.borrow(), ["own.png"]);
        assert_eq!(h.state().session.journal.last().map(|s| s.0.as_str()), Some("file.save"));
    }

    /// An automation save to a flat file leaves the document without a complete file.
    #[test]
    fn an_automation_save_only_clears_the_note_when_the_file_holds_the_document() {
        let (mut h, _) = harness("unused.png");
        let doc = h.state().active_doc_id().unwrap();
        let revision = h.state().session.active().unwrap().revision;
        h.state_mut().saved_automation(doc, revision, "flat.png", &[], false);
        assert!(h.state().flat_copy_docs.contains(&doc), "a first flat save counts");
        h.state_mut().saved_automation(doc, revision, "flat.png", &[], false);
        assert!(h.state().flat_copy_docs.contains(&doc), "and a later one keeps it");
        h.state_mut().saved_automation(doc, revision, "art.psd", &[], false);
        assert!(!h.state().flat_copy_docs.contains(&doc));
    }

    /// A plain document loses nothing to a flat file, so there's nothing for the close prompt to say.
    #[test]
    fn an_automation_flat_save_of_a_plain_document_is_not_a_flat_copy() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().run("file.new", json!({"width": 4, "height": 4, "name": "Plain", "background": "white"})).unwrap();
        let doc = h.state().active_doc_id().unwrap();
        let revision = h.state().session.active().unwrap().revision;
        h.state_mut().saved_automation(doc, revision, "flat.png", &[], false);
        assert!(!h.state().flat_copy_docs.contains(&doc));
    }

    /// A script's flat save goes through the same record, by the route it really takes.
    #[test]
    fn a_scripted_flat_save_counts_as_a_flat_copy() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().services.automation_write = Some(Box::new(|_, _| Ok(())));
        let doc = h.state().active_doc_id().unwrap();
        h.state_mut().save_automation_with(Some("flat.png".into()), FormatChoices::default()).unwrap();
        assert!(h.state().only_saved_flat(doc));
    }

    /// A TIFF keeps layers and alpha channels, so a flat copy beside one loses nothing.
    #[test]
    fn a_flat_copy_of_a_document_in_a_tiff_is_not_the_only_copy() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().run("channel.new", json!({"name": "Alpha 1"})).unwrap();
        let doc = h.state().active_doc_id().unwrap();
        h.state_mut().flat_copy_docs.insert(doc);
        assert!(h.state().only_saved_flat(doc), "no file of its own yet");
        h.state_mut().session.active_mut().unwrap().path = Some("art.tif".into());
        assert!(!h.state().only_saved_flat(doc));
        h.state_mut().run("path.set", json!({"name": "Path 1", "path": {"subpaths": []}})).unwrap();
        assert!(h.state().only_saved_flat(doc), "a saved path isn't in the TIFF");
    }

    /// A confirmation doesn't outlive a save that never ran.
    #[test]
    fn a_failed_confirmed_save_drops_its_confirmation() {
        let (mut h, _) = harness("flat.png");
        h.state_mut().save_as(None).unwrap();
        h.run_steps(3);
        h.state_mut().flat_copy.as_mut().unwrap().doc = photocraft_doc::DocId(u64::MAX);
        h.get_by_label("OK").click();
        // The frame the answer lands in: the next one would clear it anyway.
        while h.state().flat_copy.is_some() {
            h.step();
        }
        assert_eq!(h.state().flat_copy_confirmed, None);
        assert!(h.state().ui.status_error);
    }

    /// The document's own OpenRaster file counts only if it holds the document.
    #[test]
    fn an_openraster_file_that_cant_hold_the_document_is_no_full_save() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        let doc = h.state().active_doc_id().unwrap();
        h.state_mut().session.active_mut().unwrap().path = Some("art.ora".into());
        assert!(!photocraft_engine::file_cmds::holds_at(&h.state().session.active().unwrap().doc, "art.ora"));
        h.state_mut().flat_copy_docs.insert(doc);
        assert!(h.state().only_saved_flat(doc));
        h.state_mut().session.active_mut().unwrap().path = Some("art.psd".into());
        assert!(!h.state().only_saved_flat(doc));
    }

    /// The question covers one save: the next to the same path gets its notice.
    #[test]
    fn a_confirmed_save_uses_up_its_confirmation() {
        let (mut h, _) = harness("unused.png");
        h.state_mut().run("channel.new", json!({"name": "Alpha 1"})).unwrap();
        h.state_mut().flat_copy_confirmed = Some("flat.png".into());
        let settings = crate::ExportSettings::default();
        h.state_mut().write_flat_aware("flat.png".into(), &settings, true).unwrap();
        assert!(h.state().ui.notices.iter().all(|n| n.title != "Saving a copy"));
        h.state_mut().write_flat_aware("flat.png".into(), &settings, true).unwrap();
        assert!(h.state().ui.notices.iter().any(|n| n.title == "Saving a copy"));
    }

    /// A command with a path never waits on a person.
    #[test]
    fn a_save_with_a_path_does_not_ask() {
        let (mut h, writes) = harness("unused.png");
        h.state_mut().save_as(Some("flat.png".into())).unwrap();
        assert!(h.state().flat_copy.is_none());
        assert_eq!(*writes.borrow(), ["flat.png"]);
    }
}
