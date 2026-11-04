//! PDF Options: the prompt File › Save As and Save a Copy show before writing a PDF, as
//! Photoshop's Save Adobe PDF dialog does with its Standard menu: a plain PDF, or PDF/X-1a:2001,
//! PDF/X-3:2002 or PDF/X-4:2008 for a printer, with the output condition (a CMYK profile: ICC v2
//! for X-1a and X-3, v2 or v4 for X-4), the colour conversion and the bleed already inside the
//! document. PDF/X-1a converts RGB, or CMYK in another condition, to the output profile on the way
//! out (Photoshop's Convert to Destination); PDF/X-3 and PDF/X-4 can instead keep the colour in
//! the document's profile (No Conversion) for the printer to convert. The document itself is
//! untouched.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. The last answer is remembered between sessions.

use std::sync::Arc;

use photocraft_cms::{ColorSpace, Profile, ProfileClass};
use photocraft_color::ColorMode;
use photocraft_io::PdfXStandard;

use crate::PhotocraftApp;

/// A CMYK profile the output condition can be: its label in the menu, how a save names it
/// (`"pdfOutputProfile"`: `document`, `working-cmyk` or the file's path) and the profile.
#[derive(Clone)]
pub struct Candidate {
    pub label: String,
    pub spec: String,
    pub profile: Arc<Profile>,
}

impl Candidate {
    /// Whether `standard` can embed this profile: PDF/X-1a and PDF/X-3 are PDF 1.3, which knows
    /// ICC version 2 only; PDF/X-4 takes version 4 too.
    pub fn fits(&self, standard: PdfXStandard) -> bool {
        standard.takes_icc_version(self.profile.version.0)
    }
}

/// A save waiting on the PDF Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// Save a Copy, or a flat PDF of a layered document: it keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as (see `targa_options_ui::Prompt::record`).
    pub record: &'static str,
    /// Standard: a PDF/X standard, or `None` for a plain PDF.
    pub standard: Option<PdfXStandard>,
    /// PDF/X-3 and PDF/X-4: convert the colour to the output profile rather than keep it in the
    /// document's profile.
    pub convert: bool,
    /// The chosen output condition, an index into `candidates`.
    pub profile: usize,
    /// The bleed inside the document, in millimetres.
    pub bleed_mm: f32,
    pub candidates: Vec<Candidate>,
    /// The document's own profile, as its colour is read, and its mode.
    source: Arc<Profile>,
    mode: ColorMode,
    /// The document carries a profile of its own (rather than reading as the working space).
    embedded: bool,
    /// The remembered answer has been applied (it needs the egui context, so on the first frame).
    restored: bool,
}

impl Prompt {
    /// The document's numbers go into the file unchanged: CMYK already in the chosen output
    /// condition (the same profile data, not just the same name), or gray as the black ink unless
    /// No Conversion keeps a gray profile the document carries.
    fn keeps_numbers(&self) -> bool {
        let convert = self.convert || self.standard.is_none_or(|s| !s.keeps_icc_colour());
        match self.mode {
            ColorMode::Grayscale => convert || !self.embedded,
            ColorMode::Cmyk => self.candidates.get(self.profile).is_some_and(|c| photocraft_engine::file_cmds::same_profile(&c.profile, &self.source)),
            _ => false,
        }
    }

    /// What stops the save as chosen, if anything.
    fn problem(&self) -> Option<String> {
        let standard = self.standard?;
        let Some(c) = self.candidates.get(self.profile) else {
            return Some(
                tl!("No CMYK profile is installed. Install the printer's profile (for example ISO Coated v2 or Coated FOGRA39) in Windows and save again.")
                    .into(),
            );
        };
        if !c.fits(standard) {
            return Some(if standard == PdfXStandard::X4 {
                tl!("PDF/X-4 needs an ICC version 2 or 4 profile. Choose the printer's profile (for example PSO Coated v3 or Coated FOGRA39).").into()
            } else {
                crate::i18n::fmt(
                    tl!(
                        "{standard} needs an ICC version 2 profile. Choose the printer's profile (for example ISO Coated v2 or Coated FOGRA39), or save PDF/X-4."
                    ),
                    &[("standard", standard.name())],
                )
            });
        }
        if standard.keeps_icc_colour() && !self.convert && !self.keeps_numbers() && !standard.takes_icc_version(self.source.version.0) {
            return Some(crate::i18n::fmt(
                tl!(
                    "The document's profile ({profile}) is ICC version {version}, which {standard} can't embed. Convert to the output profile, or save PDF/X-4."
                ),
                &[("profile", &self.source.description), ("version", &self.source.version.0.to_string()), ("standard", standard.name())],
            ));
        }
        None
    }
}

/// The PDF answer in command `cmd`'s params (`None`: none given, so Save As asks).
pub fn params_param(params: &serde_json::Value, cmd: &str) -> Result<Option<photocraft_engine::file_cmds::PdfParams>, String> {
    photocraft_engine::file_cmds::pdf_params(params, cmd).map_err(|e| e.to_string())
}

/// `true` when `path` names a PDF.
pub fn is_pdf_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).as_deref() == Some("pdf")
}

/// The output conditions on offer: the document's own CMYK profile, the working CMYK space, then
/// every installed CMYK output profile, each once (by its data: two files can share a name and
/// differ in colour, and then both are listed, told apart by their file names).
fn candidates(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = Vec::new();
    let mut push = |label: String, spec: String, profile: Arc<Profile>| {
        if profile.color_space != ColorSpace::Cmyk || list.iter().any(|c| photocraft_engine::file_cmds::same_profile(&c.profile, &profile)) {
            return;
        }
        let label = if list.iter().any(|c| c.label == label) {
            let file = std::path::Path::new(&spec).file_name().map_or(spec.clone(), |f| f.to_string_lossy().into_owned());
            format!("{label} ({file})")
        } else {
            label
        };
        list.push(Candidate { label, spec, profile });
    };
    if doc.mode == ColorMode::Cmyk {
        let p = photocraft_engine::color_cmds::document_profile(doc);
        push(crate::i18n::fmt(tl!("Document: {profile}"), &[("profile", &p.description)]), "document".into(), p);
    }
    let working = app.session.color.working(ColorMode::Cmyk);
    push(crate::i18n::fmt(tl!("Working CMYK: {profile}"), &[("profile", &working.description)]), "working-cmyk".into(), working);
    for p in photocraft_engine::installed_profiles::installed() {
        if p.color_space != ColorSpace::Cmyk || p.class != ProfileClass::Output {
            continue;
        }
        if let Ok(profile) = photocraft_engine::color_cmds::resolve_profile(&p.path, None, Some(ColorMode::Cmyk)) {
            push(p.description.clone(), p.path.clone(), profile);
        }
    }
    list
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err("Answer the TIFF, Targa, DDS, JPEG or PDF Options before starting another save".into());
    }
    let doc = app.active_doc_id()?;
    let st = app.session.active().ok_or("no document")?;
    let (source, mode, embedded) = (photocraft_engine::color_cmds::document_profile(&st.doc), st.doc.mode, st.doc.icc_profile.is_some());
    let candidates = candidates(app, &st.doc);
    // Start on the document's own condition when it can be one, else the first that fits.
    let profile = candidates.iter().position(|c| c.fits(PdfXStandard::X1a2001)).unwrap_or(0);
    app.pdf_options =
        Some(Prompt { doc, path, copy, record, standard: None, convert: true, profile, bleed_mm: 0.0, candidates, source, mode, embedded, restored: false });
    Ok(())
}

/// The remembered answer: (standard id or "none", output profile as a save names it, bleed in
/// mm, convert).
type Remembered = (String, String, f32, bool);

fn remembered_id() -> egui::Id {
    egui::Id::unique("pdf-options-last")
}

/// The save choices for `p`'s answer, as a script or a recorded action would pass them.
pub fn params(p: &Prompt) -> Result<photocraft_engine::file_cmds::PdfParams, String> {
    let Some(standard) = p.standard else { return Ok(photocraft_engine::file_cmds::PdfParams::PLAIN) };
    if let Some(problem) = p.problem() {
        return Err(problem);
    }
    let c = p.candidates.get(p.profile).ok_or("PDF/X needs a CMYK output profile; none is installed")?;
    Ok(photocraft_engine::file_cmds::PdfParams {
        standard: Some(standard),
        profile: Some(c.spec.clone()),
        bleed_mm: f64::from(p.bleed_mm.clamp(0.0, 50.0)),
        convert: p.convert || !standard.keeps_icc_colour(),
    })
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.pdf_options else { return };
    if !p.restored {
        p.restored = true;
        if let Some((standard, spec, bleed, convert)) = ctx.data_mut(|d| d.get_persisted::<Remembered>(remembered_id())) {
            p.standard = PdfXStandard::from_id(&standard);
            p.bleed_mm = bleed.clamp(0.0, 50.0);
            p.convert = convert;
            if let Some(i) = p.candidates.iter().position(|c| c.spec == spec) {
                p.profile = i;
            }
        }
    }
    let (mut ok, mut cancel) = (false, false);
    let (mut standard, mut profile, mut bleed, mut convert) = (p.standard, p.profile, p.bleed_mm, p.convert);
    let problem = p.problem();
    let can_save = problem.is_none();
    let modal = egui::Modal::new(egui::Id::unique("pdf-options")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.label(egui::RichText::new(tl!("PDF Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Standard"));
        ui.radio_value(&mut standard, None, tl!("None"));
        for s in PdfXStandard::ALL {
            ui.radio_value(&mut standard, Some(s), s.name());
        }
        if let Some(s) = standard {
            ui.add_space(8.0);
            crate::widgets::section_label(ui, tl!("Output Condition"));
            if !p.candidates.is_empty() {
                let labels: Vec<(usize, String)> = p
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, if c.fits(s) { c.label.clone() } else { format!("{} (ICC v{})", c.label, c.profile.version.0) }))
                    .collect();
                let options: Vec<(usize, &str)> = labels.iter().map(|(i, l)| (*i, l.as_str())).collect();
                crate::widgets::dropdown_literal(ui, "pdf-output-profile", tl!("Output Condition"), &mut profile, &options, 440.0);
            }
            if s.keeps_icc_colour() {
                ui.add_space(8.0);
                crate::widgets::section_label(ui, tl!("Color Conversion"));
                ui.radio_value(&mut convert, true, tl!("Convert to Output Profile"));
                ui.radio_value(&mut convert, false, tl!("No Conversion (keep the document's profile)"));
            }
            ui.add_space(4.0);
            match &problem {
                Some(problem) => {
                    ui.label(problem.as_str());
                }
                None if p.keeps_numbers() => {
                    ui.weak(tl!("The CMYK numbers are kept."));
                }
                None if s.keeps_icc_colour() && !convert => {
                    ui.weak(tl!("The colours keep the document's profile, embedded in the PDF; the printer converts them."));
                }
                None => {
                    ui.weak(tl!("The colours are converted to this profile in the PDF; the document is not changed."));
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Bleed:"));
                crate::widgets::value_field(ui, &mut bleed, 0.0..=50.0, "mm", 70.0);
            });
            ui.weak(tl!("The bleed already in the document: the trim box sits this far inside the page."));
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = ui.add_enabled_ui(can_save, |ui| crate::widgets::primary_button(ui, tl!("Save PDF"), 84.0)).inner.clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.standard = standard;
    p.profile = profile;
    p.bleed_mm = bleed;
    p.convert = convert;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    let enter = can_save && !cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if ok || enter {
        let Some(p) = app.pdf_options.take() else { return };
        let fail = |app: &mut PhotocraftApp, e: String| {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        };
        let chosen = match params(&p) {
            Ok(chosen) => chosen,
            Err(e) => return fail(app, e),
        };
        let mut settings = app.save_settings();
        let doc = app.session.documents().iter().find(|d| d.doc.id == p.doc).map(|d| d.doc.clone());
        match (photocraft_engine::file_cmds::PdfRequest::resolve(&app.session.color, &chosen), doc) {
            (Ok(Some(request)), Some(doc)) => settings.pdf = request.options_for(&doc),
            (Ok(_), Some(_)) => {}
            (Err(e), _) => return fail(app, e.to_string()),
            (_, None) => return fail(app, tl!("The document is closed").into()),
        }
        let spec = p.candidates.get(p.profile).map(|c| c.spec.clone()).unwrap_or_default();
        let id = p.standard.map_or("none", PdfXStandard::id).to_string();
        ctx.data_mut(|d| d.insert_persisted::<Remembered>(remembered_id(), (id, spec, p.bleed_mm, p.convert)));
        let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        // An action being recorded plays the save back with the same standard and condition.
        chosen.record(&mut step);
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.copy));
        if written.is_ok() {
            // An action being recorded keeps the Save As or Save a Copy.
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.copy => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    } else if cancel {
        app.pdf_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_names() {
        assert!(is_pdf_path("a.pdf"));
        assert!(is_pdf_path("C:\\x\\y.PDF"));
        assert!(!is_pdf_path("a.pdf.psd"));
        assert!(!is_pdf_path("a.png"));
    }
}
