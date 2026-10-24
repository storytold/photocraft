//! DPX Options on Save As and Save a Copy: the bits per sample it asks for, the scripted
//! `dpxBits` that skips it, and action recording and playback (#1049).
use super::*;
use crate::{FileDialogAnswer, dpx_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_io::DpxBits;
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the DPX bits the export was asked for (`None`: the default).
type Writes = Rc<RefCell<Vec<(String, Option<DpxBits>)>>>;

/// One 8×8 RGB document (16-bit when `deep`) whose exports record their DPX bits, in a harness
/// showing DPX Options and answering a save dialog with `to`.
fn dpx_app(deep: bool, to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    if deep {
        app.run("image.mode.bits16", json!({})).unwrap();
    }
    let writes: Writes = Rc::default();
    let (asked, written) = (Rc::new(RefCell::new(None)), writes.clone());
    let seen = asked.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(move |_, _, settings| {
        *seen.borrow_mut() = settings.dpx_bits;
        Ok((Vec::new(), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, _| {
        written.borrow_mut().push((path.to_string(), *asked.borrow()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            dpx_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Save As to a DPX frame asks first, starting at the document's own depth (8 bits for an 8-bit
/// document, else 16), and writes only once answered.
#[test]
fn save_as_to_a_dpx_frame_asks_for_its_bits_per_sample() {
    for (deep, start) in [(false, DpxBits::Bits8), (true, DpxBits::Bits16)] {
        let (mut h, writes) = dpx_app(deep, "unused.dpx");
        h.state_mut().save_as(Some("t.dpx".into())).unwrap();
        h.run_steps(2);
        assert_eq!(h.state().dpx_options.as_ref().map(|p| (p.bits, p.copy)), Some((start, false)));
        assert!(writes.borrow().is_empty(), "nothing is written before the answer");
        assert!(h.state_mut().save_as(Some("other.dpx".into())).is_err(), "one save at a time");
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert_eq!(writes.borrow().as_slice(), &[("t.dpx".to_string(), Some(start))]);
        assert!(h.state().dpx_options.is_none());
        assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.dpx"));
    }
}

/// The choice made in the prompt is the one written; Cancel writes nothing.
#[test]
fn dpx_options_writes_the_choice_or_cancels() {
    let (mut h, writes) = dpx_app(true, "unused.dpx");
    h.state_mut().save_as(Some("t.dpx".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("10 Bits/Channel").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("t.dpx".to_string(), Some(DpxBits::Bits10))]);

    h.state_mut().save_as(Some("c.dpx".into())).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().dpx_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}

/// Scripts choose with `dpxBits` and are never asked; a bad value writes nothing, and other
/// formats (Cineon too, always 10-bit) aren't asked.
#[test]
fn scripted_dpx_bits_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = dpx_app(true, "unused.dpx");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dpx", "dpxBits": 12})).unwrap();
    assert!(h.state().dpx_options.is_none());
    for bad in [json!(24), json!("10")] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "bad.dpx", "dpxBits": bad})).is_err(), "{bad}");
    }
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "f.cin"})).unwrap();
    assert!(h.state().dpx_options.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.dpx".to_string(), Some(DpxBits::Bits12)), ("p.png".to_string(), None), ("f.cin".to_string(), None)]);
}

/// Save a Copy to a DPX frame asks too, and the copy leaves the document's path and unsaved state.
#[test]
fn save_a_copy_to_a_dpx_frame_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = dpx_app(false, "copy.dpx");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dpx_options.as_ref().map(|p| (p.bits, p.copy)), Some((DpxBits::Bits8, true)));
    assert!(dpx_options_ui::park(h.state_mut(), "other.dpx".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.dpx".to_string(), Some(DpxBits::Bits8))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a DPX Save As or Save a Copy with its bits, answered in DPX
/// Options or scripted, and plays them back without asking.
#[test]
fn recorded_dpx_saves_keep_their_bits_and_replay_without_asking() {
    let (mut h, writes) = dpx_app(true, "copy.dpx");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("t.dpx".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("12 Bits/Channel").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("8 Bits/Channel").click();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dpx", "dpxBits": 10})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let want = [("t.dpx".to_string(), Some(DpxBits::Bits12)), ("copy.dpx".to_string(), Some(DpxBits::Bits8)), ("s.dpx".to_string(), Some(DpxBits::Bits10))];
    assert_eq!(writes.borrow().as_slice(), &want);
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(
        steps,
        &[
            ("file.saveAs".into(), json!({"path": "t.dpx", "dpxBits": 12})),
            ("file.saveACopy".into(), json!({"path": "copy.dpx", "dpxBits": 8})),
            ("file.saveAs".into(), json!({"path": "s.dpx", "dpxBits": 10}))
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().dpx_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &want);
}

/// File › Save confirming the document's own frame asks DPX Options, and an action being recorded
/// keeps it as a plain Save with the bits chosen: played back, it writes the frame the action is
/// played on with them, without asking (#3531).
#[test]
fn recorded_plain_save_of_a_dpx_frame_keeps_its_bits() {
    let (mut h, writes) = dpx_app(true, "own.dpx");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "own.dpx", "dpxBits": 16})).unwrap();
    h.state_mut().run("actions.record", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().dpx_options.is_some(), "confirming its own frame asks DPX Options");
    h.get_by_label("10 Bits/Channel").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(steps, &[("file.save".into(), json!({"dpxBits": 10}))]);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "other.dpx", "dpxBits": 16})).unwrap();
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().dpx_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &[("other.dpx".to_string(), Some(DpxBits::Bits10))]);
}

/// Enter on a focused Cancel cancels: egui clicks the focused button on Enter without consuming
/// the key, so the dialog's Enter-for-OK must not save as well.
#[test]
fn enter_on_a_focused_cancel_writes_nothing() {
    let (mut h, writes) = dpx_app(false, "unused.dpx");
    h.state_mut().save_as(Some("t.dpx".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").focus();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().dpx_options.is_none());
    assert!(writes.borrow().is_empty(), "nothing written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}
