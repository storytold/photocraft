//! PDF Options on Save As and Save a Copy: a plain PDF or PDF/X-1a with its output condition and
//! bleed, the ICC v4 condition PDF/X-1a can't take, and scripted saves that never ask.
use super::*;
use crate::{FileDialogAnswer, pdf_options_ui};
use egui_kittest::kittest::Queryable;
use std::{cell::RefCell, rc::Rc};

/// What one export was asked for: `None` for a plain PDF, else the PDF/X-1a bleed (pt) and
/// whether the colour is converted.
type Asked = Option<(f64, bool)>;
/// Each write: the path and what its export was asked for.
type Writes = Rc<RefCell<Vec<(String, Asked)>>>;

/// One 8×8 RGB document whose exports record their PDF options, in a harness showing PDF Options
/// and answering a save dialog with `to`.
fn pdf_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| {
        let asked: Asked = settings.pdf.x1a.as_ref().map(|x| (x.bleed_pt, x.source.is_some()));
        Ok((serde_json::to_vec(&asked).unwrap(), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), serde_json::from_slice(bytes).unwrap()));
        Ok(())
    }));
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0));
    let builder = if std::env::var_os("PHOTOCRAFT_PDF_SNAPSHOTS").is_some() { builder.wgpu() } else { builder };
    let h = builder.build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            pdf_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Saves what the harness shows when `PHOTOCRAFT_PDF_SNAPSHOTS` names a folder (for reviews).
fn capture(h: &mut Prompted, name: &str) {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_PDF_SNAPSHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// An ICC v2 CMYK profile file (the built-in profile relabelled v2.1; no v2 profile ships), as
/// an installed printer profile would be, and its path.
fn v2_profile_file() -> String {
    let mut b = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes().to_vec();
    b[8] = 2;
    b[9] = 0x10;
    let path = std::env::temp_dir().join(format!("photocraft-pdf-tests-v2-{}.icc", std::process::id()));
    std::fs::write(&path, b).unwrap();
    path.to_string_lossy().into_owned()
}

/// That profile as an output condition in the menu.
fn v2_condition() -> pdf_options_ui::Candidate {
    let spec = v2_profile_file();
    let profile = photocraft_engine::color_cmds::resolve_profile(&spec, None, None).unwrap();
    pdf_options_ui::Candidate { label: "Test v2".into(), spec, profile }
}

/// Save As to a PDF picked in the dialog asks first, and writes a plain PDF when the standard
/// stays None.
#[test]
fn save_as_to_a_pdf_asks_and_writes_plain_by_default() {
    let (mut h, writes) = pdf_app("t.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    assert!(h.state().pdf_options.as_ref().is_some_and(|p| !p.x1a && !p.copy));
    capture(&mut h, "01-pdf-options");
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("t.pdf".to_string(), None)]);
    assert!(h.state().pdf_options.is_none());
}

/// PDF/X-1a writes the chosen condition and bleed, converting an RGB document to it.
#[test]
fn pdf_x1a_writes_the_chosen_condition_and_bleed() {
    let (mut h, writes) = pdf_app("x.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.candidates.push(v2_condition());
        p.profile = p.candidates.len() - 1;
        p.x1a = true;
        p.bleed_mm = 3.0;
    }
    h.run_steps(2);
    capture(&mut h, "02-pdf-x1a");
    h.get_by_label("Save PDF").click();
    h.run_steps(3);
    let w = writes.borrow();
    assert_eq!(w.len(), 1);
    let (bleed, converted) = w[0].1.expect("PDF/X-1a");
    assert_eq!(w[0].0, "x.pdf");
    assert!((bleed - 3.0 * 72.0 / 25.4).abs() < 1e-9, "3 mm in points: {bleed}");
    assert!(converted, "an RGB document is converted to the output condition");
}

/// An ICC v4 condition (the built-in CMYK) can't be saved as PDF/X-1a:2001: the prompt stays
/// until another condition or the standard None is chosen, or it is cancelled.
#[test]
fn a_version_4_condition_cant_be_saved_as_x1a() {
    let (mut h, writes) = pdf_app("v4.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.profile = p.candidates.iter().position(|c| !c.fits_x1a()).expect("the built-in working CMYK is v4");
        p.x1a = true;
    }
    h.run_steps(2);
    capture(&mut h, "03-pdf-x1a-v4-refused");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(writes.borrow().is_empty(), "nothing written");
    assert!(h.state().pdf_options.is_some(), "still asking");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().pdf_options.is_none());
    assert_eq!(h.state().ui.status, "Save cancelled");
}

/// A script naming a PDF path writes a plain PDF and is never asked.
#[test]
fn scripted_pdf_saves_never_ask() {
    let (mut h, writes) = pdf_app("unused.pdf");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.pdf"})).unwrap();
    assert!(h.state().pdf_options.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.pdf".to_string(), None)]);
}

/// Save a Copy to a PDF asks too, and the copy leaves the document's path alone.
#[test]
fn save_a_copy_to_a_pdf_asks_and_keeps_the_document() {
    let (mut h, writes) = pdf_app("copy.pdf");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().pdf_options.as_ref().map(|p| p.copy), Some(true));
    assert!(pdf_options_ui::park(h.state_mut(), "other.pdf".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.pdf".to_string(), None)]);
    assert_ne!(h.state().session.active().unwrap().path.as_deref(), Some("copy.pdf"));
}

/// The answer is recorded, so an action plays the save back as PDF/X-1a with the same condition
/// and bleed; a script passes the same params and is never asked.
#[test]
fn pdf_x1a_is_recorded_and_scripted_with_the_same_params() {
    let (mut h, writes) = pdf_app("r.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    let spec = {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.candidates.push(v2_condition());
        p.profile = p.candidates.len() - 1;
        p.x1a = true;
        p.bleed_mm = 5.0;
        p.candidates[p.profile].spec.clone()
    };
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    let (id, step) = h.state().session.journal.last().cloned().expect("a recorded step");
    assert_eq!(id, "file.saveAs");
    assert_eq!(step, json!({"path": "r.pdf", "pdfStandard": "x1a2001", "pdfOutputProfile": spec, "pdfBleedMm": 5.0}));

    let ctx = h.ctx.clone();
    let mut scripted = step.clone();
    scripted["path"] = "s.pdf".into();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", scripted).unwrap();
    assert!(h.state().pdf_options.is_none(), "never asked");
    let w = writes.borrow();
    assert_eq!(w.len(), 2);
    assert_eq!(w[1].0, "s.pdf");
    assert!(w[1].1.is_some_and(|(bleed, converted)| (bleed - 5.0 * 72.0 / 25.4).abs() < 1e-9 && converted), "{:?}", w[1]);
    for bad in [
        json!({"path": "b.pdf", "pdfStandard": "x4"}),
        json!({"path": "b.pdf", "pdfBleedMm": 3}),
        json!({"path": "b.pdf", "pdfStandard": "x1a2001", "pdfBleedMm": -1}),
    ] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(w.len(), 2, "bad params write nothing");
}
