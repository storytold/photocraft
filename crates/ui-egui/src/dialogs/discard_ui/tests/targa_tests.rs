//! Targa Options on Save As and Save a Copy: the bits per pixel it asks for, the scripted
//! `tgaBits` that skips it, and recording and playback.
use super::jpeg_tests::{fields, parked, set_field, steps};
use super::*;
use craft::automation::actions::ActionStep;
use craft::host::{Picked, ScriptedFiles};
use egui_kittest::kittest::Queryable;
use photocraft_io::TgaBits;
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Each write: the path, and the Targa bits the export was asked for (as JSON).
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The JSON a write records for `bits`.
fn asked(bits: Option<u8>) -> Vec<u8> {
    serde_json::to_vec(&bits).unwrap()
}

/// Records the Targa options on each write without encoding or touching the filesystem.
fn targa_writes(app: &mut PhotocraftApp) -> Writes {
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.export = Some(Box::new(|_, _, settings| Ok((asked(settings.tga_bits.map(TgaBits::bits)), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    writes
}

/// One 8×8 RGB document (with an "Alpha 1" channel when `alpha_channel`) whose exports record
/// their Targa bits, in a harness running the dialogs and answering a save dialog with `to`.
fn targa_app(alpha_channel: bool, to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs_over(1, Some(Arc::new(ScriptedFiles::answering([Picked::Paths(vec![to.into()])]))));
    if alpha_channel {
        app.run("select.all", json!({})).unwrap();
        app.run("select.saveSelection", json!({})).unwrap();
    }
    let writes = targa_writes(&mut app);
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app| {
            crate::dialogs::run(app, ui);
            app.poll_file_dialog();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    craft::ui::testing::windows::set_os(&h.ctx, craft::host::target::Os::Windows);
    (h, writes)
}

/// Save As to a Targa asks first: 32 bits per pixel when the document has an alpha channel,
/// else 24 (as Photoshop's Targa Options starts), and writes only once answered.
#[test]
fn save_as_to_a_targa_asks_for_its_bits_per_pixel() {
    for (alpha_channel, start) in [(false, TgaBits::Bits24), (true, TgaBits::Bits32)] {
        let (mut h, writes) = targa_app(alpha_channel, "unused.tga");
        h.state_mut().save_as(Some("t.tga".into())).unwrap();
        h.run_steps(2);
        assert_eq!(fields(&h)["bits"].as_str(), Some(start.bits().to_string().as_str()));
        assert_eq!(parked(&h), Some(("t.tga".to_string(), false)));
        assert!(writes.borrow().is_empty(), "nothing is written before the answer");
        assert!(h.state_mut().save_as(Some("other.tga".into())).is_err(), "one save at a time");
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert_eq!(writes.borrow().as_slice(), &[("t.tga".to_string(), asked(Some(start.bits())))]);
        assert!(h.state().parked_save.is_none());
        assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.tga"));
    }
}

/// The choice made in the dialog is the one written.
#[test]
fn targa_options_writes_the_choice_or_cancels() {
    let (mut h, writes) = targa_app(true, "unused.tga");
    h.state_mut().save_as(Some("t.tga".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("24 bits/pixel").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("t.tga".to_string(), asked(Some(24)))]);
}

/// With the real window key handling running, Targa Options owns the first Escape.
#[test]
fn escape_cancels_targa_options_before_deselecting() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app
    });
    let writes = targa_writes(h.state_mut());
    h.run_steps(4);
    h.state_mut().run("select.all", json!({})).unwrap();
    let st = h.state().session.active().unwrap();
    let (selection, history) = (st.doc.selection.clone().expect("Select All created a selection"), st.history.past_len());

    h.state_mut().save_as(Some("t.tga".into())).unwrap();
    h.run_steps(2);
    assert_eq!(parked(&h), Some(("t.tga".to_string(), false)));
    let dialog = h.state().framework.dialogs.open_dialogs().last().expect("Targa Options is open").id;

    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(h.state().framework.dialogs.get_dyn(dialog).is_none(), "Escape closes Targa Options");
    assert!(parked(&h).is_none(), "Escape cancels the parked save");
    assert!(writes.borrow().is_empty(), "the cancelled save writes nothing");
    let st = h.state().session.active().unwrap();
    assert_eq!(st.path, None, "the cancelled save leaves the document unsaved");
    assert_eq!(st.doc.selection.as_ref(), Some(&selection), "the dialog's Escape leaves the selection intact");
    assert_eq!(st.history.past_len(), history, "the dialog's Escape leaves document history intact");

    h.key_press(Key::Escape);
    h.run_steps(4);
    let st = h.state().session.active().unwrap();
    assert!(st.doc.selection.is_none(), "once the dialog is gone, Escape reaches the window and deselects");
    assert_eq!(st.history.past_len(), history + 1, "only the second Escape edits the document");
}

/// Scripts choose with `tgaBits` and are never asked; a grayscale Targa (8-bit, nothing to choose)
/// and other formats aren't asked either.
#[test]
fn scripted_bits_grayscale_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = targa_app(true, "unused.tga");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.tga", "tgaBits": 24})).unwrap();
    assert!(h.state().parked_save.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.tga".to_string(), asked(Some(24)))]);
    for bad in [json!(16), json!("32")] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.tga", "tgaBits": bad})).is_err(), "{bad}");
    }
    assert_eq!(writes.borrow().len(), 1, "a bad tgaBits writes nothing");
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    h.state_mut().run("image.mode.grayscale", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "g.tga"})).unwrap();
    assert!(h.state().parked_save.is_none());
    assert_eq!(writes.borrow()[1..], [("p.png".to_string(), asked(None)), ("g.tga".to_string(), asked(None))]);
}

/// Save As of a document a flat Targa can't hold (here, two layers) is a copy through Targa
/// Options too, as without it: the document keeps its path and its unsaved changes (#2550).
#[test]
fn targa_options_keeps_a_layered_save_as_a_copy() {
    let (mut h, writes) = targa_app(false, "unused.tga");
    make_dirty(h.state_mut(), 0);
    h.state_mut().save_as(Some("t.tga".into())).unwrap();
    h.run_steps(2);
    assert_eq!(parked(&h).map(|p| p.1), Some(true));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("t.tga".to_string(), asked(Some(24)))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a Targa Save As or Save a Copy with its bits, answered in Targa
/// Options or scripted (#2032, #4373), and plays them back without asking.
#[test]
fn recorded_targa_save_as_keeps_its_bits_and_replays_without_asking() {
    let (mut h, writes) = targa_app(true, "copy.tga");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("t.tga".into())).unwrap();
    h.run_steps(2);
    set_field(&mut h, "bits", json!("24"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.tga", "tgaBits": 32})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(
        writes.borrow().as_slice(),
        &[("t.tga".to_string(), asked(Some(24))), ("copy.tga".to_string(), asked(Some(32))), ("s.tga".to_string(), asked(Some(32)))]
    );
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"path": "t.tga", "tgaBits": 24})),
            ActionStep::new("file.saveACopy", json!({"path": "copy.tga", "tgaBits": 32})),
            ActionStep::new("file.saveAs", json!({"path": "s.tga", "tgaBits": 32})),
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().parked_save.is_none(), "playback doesn't ask");
    assert_eq!(
        writes.borrow().as_slice(),
        &[("t.tga".to_string(), asked(Some(24))), ("copy.tga".to_string(), asked(Some(32))), ("s.tga".to_string(), asked(Some(32)))]
    );
}

/// #5305: a Targa Save As answered in Targa Options with the document's own name records only the
/// folder and extension (with the bits), so playback saves each document under its own name; a
/// Save a Copy keeps its path.
#[test]
fn recorded_targa_save_as_with_the_documents_name_keeps_only_the_folder() {
    let (mut h, writes) = targa_app(false, "copy.tga");
    let ctx = h.ctx.clone();
    let name = h.state().session.active().unwrap().doc.name.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some(format!("out/{name}.tga"))).unwrap();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"folder": "out", "extension": "tga", "tgaBits": 24})),
            ActionStep::new("file.saveACopy", json!({"path": "copy.tga", "tgaBits": 24})),
        ]
    );
    writes.borrow_mut().clear();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8, "name": "second"})).unwrap();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let second = std::path::Path::new("out").join("second.tga").to_string_lossy().into_owned();
    assert_eq!(writes.borrow().as_slice(), &[(second, asked(Some(24))), ("copy.tga".to_string(), asked(Some(24)))]);
}
