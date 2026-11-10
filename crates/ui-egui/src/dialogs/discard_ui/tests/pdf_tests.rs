//! PDF Options on Save As and Save a Copy: a plain PDF or PDF/X-1a with its output condition and
//! bleed, the ICC v4 condition PDF/X-1a can't take, and scripted saves that never ask.
use super::*;
use crate::{FileDialogAnswer, pdf_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_io::PdfXStandard;
use std::{cell::RefCell, rc::Rc};

/// What one export was asked for: `None` for a plain PDF, else the PDF/X standard id, the bleed
/// (pt), whether the document's colour is read through its own profile (converted or embedded)
/// and whether it is converted.
type Asked = Option<(String, f64, bool, bool)>;
/// Each write: the path and what its export was asked for.
type Writes = Rc<RefCell<Vec<(String, Asked)>>>;

/// One 8×8 RGB document at 1 ppi (an 8 × 8 inch page, room for any bleed) whose exports record
/// their PDF options, in a harness showing PDF Options and answering a save dialog with `to`.
fn pdf_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    app.session
        .edit("resolution", |doc, _| {
            doc.resolution_dpi = 1.0;
            Ok(())
        })
        .unwrap();
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| {
        let asked: Asked = settings.pdf.x.as_ref().map(|x| (x.standard.id().to_string(), x.bleed_pt, x.source.is_some(), x.convert));
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
    // One file per call: tests run in parallel, and one rewriting a shared file while another
    // reads it hands the reader a truncated profile.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("photocraft-pdf-tests-v2-{}-{n}.icc", std::process::id()));
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
    assert!(h.state().pdf_options.as_ref().is_some_and(|p| p.standard.is_none() && !p.copy));
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
        p.standard = Some(PdfXStandard::X1a2001);
        p.bleed_mm = 3.0;
    }
    h.run_steps(2);
    capture(&mut h, "02-pdf-x1a");
    h.get_by_label("Save PDF").click();
    h.run_steps(3);
    let w = writes.borrow();
    assert_eq!(w.len(), 1);
    let (standard, bleed, converted, _) = w[0].1.clone().expect("PDF/X-1a");
    assert_eq!(standard, "x1a2001");
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
        p.profile = p.candidates.iter().position(|c| !c.fits(PdfXStandard::X1a2001)).expect("the built-in working CMYK is v4");
        p.standard = Some(PdfXStandard::X1a2001);
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
        p.standard = Some(PdfXStandard::X1a2001);
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
    assert!(w[1].1.as_ref().is_some_and(|(_, bleed, converted, _)| (bleed - 5.0 * 72.0 / 25.4).abs() < 1e-9 && *converted), "{:?}", w[1]);
    for bad in [
        json!({"path": "b.pdf", "pdfStandard": "x5"}),
        json!({"path": "b.pdf", "pdfBleedMm": 3}),
        json!({"path": "b.pdf", "pdfStandard": "x1a2001", "pdfBleedMm": -1}),
    ] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(w.len(), 2, "bad params write nothing");
}

/// PDF/X-4 takes the built-in ICC v4 CMYK that PDF/X-1a and PDF/X-3 refuse, and with No
/// Conversion keeps the document's colour; the answer is recorded with `pdfConvert: false`.
#[test]
fn pdf_x4_takes_a_version_4_condition_and_can_keep_the_colour() {
    let (mut h, writes) = pdf_app("x4.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.profile = p.candidates.iter().position(|c| !c.fits(PdfXStandard::X3_2002)).expect("the built-in working CMYK is v4");
        assert!(p.candidates[p.profile].fits(PdfXStandard::X4));
        p.standard = Some(PdfXStandard::X3_2002);
    }
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(writes.borrow().is_empty(), "PDF/X-3 can't take a v4 condition");
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.standard = Some(PdfXStandard::X4);
        p.convert = false;
    }
    h.run_steps(2);
    capture(&mut h, "04-pdf-x4-no-conversion");
    h.get_by_label("Save PDF").click();
    h.run_steps(3);
    let w = writes.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].1, Some(("x4-2008".to_string(), 0.0, true, false)), "tagged with the document's profile, not converted");
    let (_, step) = h.state().session.journal.last().cloned().expect("a recorded step");
    assert_eq!(step["pdfStandard"], "x4-2008");
    assert_eq!(step["pdfConvert"], false);
}

/// PDF/X-3 with No Conversion can't embed the document's ICC v4 profile (the built-in sRGB): the
/// prompt says so and stays until the colour is converted.
#[test]
fn pdf_x3_keeping_a_version_4_document_profile_is_refused() {
    let (mut h, writes) = pdf_app("x3.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.candidates.push(v2_condition());
        p.profile = p.candidates.len() - 1;
        p.standard = Some(PdfXStandard::X3_2002);
        p.convert = false;
    }
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(writes.borrow().is_empty(), "nothing written");
    h.state_mut().pdf_options.as_mut().unwrap().convert = true;
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow()[0].1, Some(("x3-2002".to_string(), 0.0, true, true)));
}

/// #5305: a PDF Save As answered with the document's own name records only the folder and
/// extension, so playing the action on another document saves that one under its own name.
#[test]
fn a_pdf_save_as_with_the_documents_name_records_only_the_folder() {
    let (mut h, _writes) = pdf_app("unused.pdf");
    let name = h.state().session.active().unwrap().doc.name.clone();
    h.state_mut().services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(format!("out/{name}.pdf")))]).0);
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    h.state_mut().pdf_options.as_mut().unwrap().standard = Some(PdfXStandard::X4);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    let (id, step) = h.state().session.journal.last().cloned().expect("a recorded step");
    assert_eq!(id, "file.saveAs");
    assert_eq!(step.get("path"), None, "{step}");
    assert_eq!((step["folder"].as_str(), step["extension"].as_str(), step["pdfStandard"].as_str()), (Some("out"), Some("pdf"), Some("x4-2008")));
}

/// A bleed that leaves no trim on the page is refused in the prompt, which stays; a write that
/// fails leaves the prompt open with its destination too.
#[test]
fn the_prompt_stays_on_a_bleed_too_wide_or_a_failed_write() {
    let (mut h, writes) = pdf_app("b.pdf");
    h.state_mut()
        .session
        .edit("resolution", |doc, _| {
            doc.resolution_dpi = 72.0;
            Ok(())
        })
        .unwrap();
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    {
        let p = h.state_mut().pdf_options.as_mut().unwrap();
        p.standard = Some(PdfXStandard::X4);
        p.bleed_mm = 3.0; // the 8 px page at 72 ppi is 2.8 mm wide
    }
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(writes.borrow().is_empty() && h.state().pdf_options.is_some(), "refused, still asking");

    h.state_mut().pdf_options.as_mut().unwrap().bleed_mm = 0.0;
    h.state_mut().services.write = Some(Box::new(|_, _| Err("disk full".into())));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().pdf_options.as_ref().map(|p| p.path.as_str()), Some("b.pdf"), "the prompt and its destination stay");
    assert!(h.state().ui.status_error);
}

/// A replayed PDF/X Save a Copy step and an automation save with PDF/X params write PDF/X, not a
/// plain PDF.
#[test]
fn replayed_copies_and_automation_saves_write_pdf_x() {
    let (mut h, writes) = pdf_app("unused.pdf");
    let x4 = json!({"pdfStandard": "x4-2008", "pdfOutputProfile": "working-cmyk", "pdfConvert": false});
    let mut copy = x4.clone();
    copy["path"] = "copy.pdf".into();
    h.state_mut().replay_save("file.saveACopy", &copy).unwrap();
    // Automation writes through its own granted writer; it lands in the same record.
    let automated = writes.clone();
    h.state_mut().services.automation_write = Some(Box::new(move |path: &str, bytes: &[u8]| {
        automated.borrow_mut().push((path.to_string(), serde_json::from_slice(bytes).unwrap()));
        Ok(())
    }));
    let choices = crate::FormatChoices::from_params(&x4, "app.save").unwrap();
    h.state_mut().save_automation_with(Some("auto.pdf".into()), choices).unwrap();
    let w = writes.borrow();
    assert_eq!(w.len(), 2, "{w:?}");
    for (path, asked) in w.iter() {
        assert_eq!(asked.as_ref().map(|a| (a.0.as_str(), a.3)), Some(("x4-2008", false)), "{path}");
    }
}

/// While PDF Options waits, menu and control commands that would change the document are held
/// back, as in any modal dialog; navigation still works.
#[test]
fn commands_wait_while_pdf_options_is_open() {
    let (mut h, _writes) = pdf_app("m.pdf");
    assert!(crate::menus::modal_allows(h.state(), "image.mode.cmyk"));
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    assert!(h.state().pdf_options.is_some());
    assert!(!crate::menus::modal_allows(h.state(), "image.mode.cmyk"), "no mode change under the prompt");
    assert!(crate::menus::modal_allows(h.state(), "view.zoomIn"), "navigation still works");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(crate::menus::modal_allows(h.state(), "image.mode.cmyk"), "free again after Cancel");
}

/// An automation save can't name its output profile by file (that would read outside the granted
/// paths); PDF/X answers on a save to another format are ignored rather than resolved.
#[test]
fn automation_saves_take_no_profile_files_and_other_formats_ignore_pdf_answers() {
    let (mut h, writes) = pdf_app("unused.pdf");
    let sink = writes.clone();
    h.state_mut().services.automation_write = Some(Box::new(move |path: &str, _: &[u8]| {
        sink.borrow_mut().push((path.to_string(), None));
        Ok(())
    }));
    let file = json!({"pdfStandard": "x4-2008", "pdfOutputProfile": "C:/secret/printer.icc"});
    let choices = crate::FormatChoices::from_params(&file, "app.save").unwrap();
    let e = h.state_mut().save_automation_with(Some("a.pdf".into()), choices).unwrap_err();
    assert!(e.contains("can't read a profile file"), "{e}");
    assert!(writes.borrow().is_empty(), "nothing written");

    let unresolvable = json!({"pdfStandard": "x4-2008", "pdfOutputProfile": "no-such-profile"});
    let choices = crate::FormatChoices::from_params(&unresolvable, "app.save").unwrap();
    h.state_mut().save_automation_with(Some("a.png".into()), choices).unwrap();
    assert_eq!(writes.borrow().len(), 1, "the PNG is written; its PDF answers are never resolved");
    let choices = crate::FormatChoices::from_params(&file, "app.save").unwrap();
    h.state_mut().save_automation_with(Some("b.png".into()), choices).unwrap();
    assert_eq!(writes.borrow().len(), 2, "a profile file only matters to a PDF");
}

/// Every section of Save PDF draws, for each standard, without changing the answer.
#[test]
fn every_section_draws_for_every_standard() {
    let (mut h, writes) = pdf_app("s.pdf");
    h.state_mut().save_as(None).unwrap();
    h.run_steps(3);
    h.state_mut().pdf_options.as_mut().unwrap().candidates.push(v2_condition());
    for standard in [None, Some(PdfXStandard::X1a2001), Some(PdfXStandard::X3_2002), Some(PdfXStandard::X4)] {
        for section in pdf_options_ui::Section::ALL {
            {
                let p = h.state_mut().pdf_options.as_mut().unwrap();
                p.standard = standard;
                p.section = section;
                p.profile = p.candidates.len() - 1;
                p.bleed_mm = 3.0;
            }
            h.run_steps(2);
            let p = h.state().pdf_options.as_ref().expect("still asking");
            assert_eq!((p.standard, p.section), (standard, section));
            if standard == Some(PdfXStandard::X4) {
                capture(&mut h, &format!("05-x4-{section:?}"));
            }
        }
    }
    // The light theme draws the same dialog from its own tokens.
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, crate::theme::ThemeKind::ProLight);
    h.state_mut().pdf_options.as_mut().unwrap().section = pdf_options_ui::Section::Output;
    h.run_steps(3);
    capture(&mut h, "06-x4-output-light");
    assert!(writes.borrow().is_empty(), "drawing writes nothing");
}
