//! BMP Options on Save As and Save a Copy (#8374): the depth it asks for, starting at 24 Bit as
//! Photoshop's does, and the scripted `bmpBits` that skips it.
use super::*;
use crate::{FileDialogAnswer, bmp_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_io::BmpBits;
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the BMP bits the export was asked for (as JSON).
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The JSON a write records for `bits`.
fn asked(bits: Option<u8>) -> Vec<u8> {
    serde_json::to_vec(&bits).unwrap()
}

/// One 8×8 RGB document whose exports record their BMP bits, in a harness showing BMP Options
/// and answering a save dialog with `to`.
fn bmp_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| Ok((asked(settings.bmp_bits.map(BmpBits::bits)), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            bmp_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Save As to a BMP asks first, starting at 24 Bit, writes only once answered, and writes the
/// depth chosen.
#[test]
fn save_as_to_a_bmp_asks_for_its_depth() {
    let (mut h, writes) = bmp_app("unused.bmp");
    h.state_mut().save_as(Some("b.bmp".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().bmp_options.as_ref().map(|p| (p.bits, p.copy)), Some((BmpBits::Bits24, false)));
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    let busy = h.state_mut().save_as(Some("other.bmp".into())).expect_err("one save at a time");
    assert!(busy.contains("BMP"), "the message names the open dialog: {busy}");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("b.bmp".to_string(), asked(Some(24)))]);
    assert!(h.state().bmp_options.is_none());

    h.state_mut().save_as(Some("c.bmp".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("32 Bit").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow()[1], ("c.bmp".to_string(), asked(Some(32))));
}

/// FredApps on #9175: the choice says what each depth does to transparency (24 Bit composites it
/// over white, 32 Bit keeps it as alpha), and offers no depth it can't write.
#[test]
fn bmp_options_say_what_each_depth_does_to_transparency() {
    let (mut h, _writes) = bmp_app("unused.bmp");
    h.state_mut().save_as(Some("b.bmp".into())).unwrap();
    h.run_steps(2);
    let (white, alpha) = ("Transparent areas are composited over white.", "Transparency is kept as the alpha channel.");
    assert!(h.query_by_label(white).is_some() && h.query_by_label(alpha).is_none(), "24 Bit");
    h.get_by_label("32 Bit").click();
    h.run_steps(2);
    assert!(h.query_by_label(alpha).is_some() && h.query_by_label(white).is_none(), "32 Bit");
    for other in ["16 Bit", "Advanced Modes"] {
        assert!(h.query_by_label(other).is_none(), "no {other}");
    }
}

/// Scripts choose with `bmpBits` and are never asked; a grayscale BMP (8-bit, nothing to choose)
/// isn't asked either.
#[test]
fn scripted_bits_and_grayscale_skip_the_bmp_prompt() {
    let (mut h, writes) = bmp_app("unused.bmp");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.bmp", "bmpBits": 32})).unwrap();
    assert!(h.state().bmp_options.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.bmp".to_string(), asked(Some(32)))]);
    assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.bmp", "bmpBits": 16})).is_err());
    assert_eq!(writes.borrow().len(), 1, "a bad bmpBits writes nothing");
    h.state_mut().run("image.mode.grayscale", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "g.bmp"})).unwrap();
    assert!(h.state().bmp_options.is_none());
    assert_eq!(writes.borrow()[1], ("g.bmp".to_string(), asked(None)));
}

/// Save a Copy to a BMP asks too, and keeps the document's path and unsaved changes.
#[test]
fn save_a_copy_to_a_bmp_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = bmp_app("copy.bmp");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().bmp_options.as_ref().map(|p| (p.bits, p.copy)), Some((BmpBits::Bits24, true)));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.bmp".to_string(), asked(Some(24)))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// #9175 review: Save a Copy writes the document as it was when asked, even if it changes while
/// BMP Options is open (as the direct Save a Copy path does).
#[test]
fn save_a_copy_to_a_bmp_writes_the_document_as_it_was_asked() {
    let (mut h, writes) = bmp_app("copy.bmp");
    // Each export records the layer count of the document it was given.
    h.state_mut().services.export = Some(Box::new(|doc, _, _| Ok((serde_json::to_vec(&doc.layers.len()).unwrap(), Vec::new()))));
    let before = h.state().session.active().unwrap().doc.layers.len();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().bmp_options.is_some(), "BMP Options is open");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.bmp".to_string(), serde_json::to_vec(&before).unwrap())]);
}

/// #9175 review: Save As decides whether the BMP is a copy when BMP Options is answered, so a
/// layer added while the prompt is open keeps the document unsaved rather than marking a flat BMP
/// as its saved state.
#[test]
fn save_as_to_a_bmp_rechecks_the_copy_when_answered() {
    let (mut h, writes) = bmp_app("unused.bmp");
    h.state_mut().save_as(Some("flat.bmp".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().bmp_options.as_ref().map(|p| p.copy), Some(false), "one raster layer: a real save");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().len(), 1, "the BMP is still written");
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty(), "the new layer isn't in the BMP, so the document stays unsaved");
    assert_eq!(st.path, None, "and keeps no BMP as its file");
}
