//! Save PDF: the prompt File › Save As and Save a Copy show before writing a PDF, laid out as
//! Photoshop's Save Adobe PDF dialog (preset, standard and compatibility above; General,
//! Compression, Output and Summary sections beside): a plain PDF, or PDF/X-1a:2001,
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
    /// The page in points (pixels ÷ resolution × 72), which the bleed has to leave a trim in.
    page_pt: (f64, f64),
    /// The section showing.
    pub section: Section,
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
        let (w, h) = self.page_pt;
        if 2.0 * f64::from(self.bleed_mm) * 72.0 / 25.4 >= w.min(h) {
            let mm = |pt: f64| format!("{:.1}", pt * 25.4 / 72.0);
            return Some(crate::i18n::fmt(
                tl!("A {bleed} mm bleed leaves no trim on this {width} × {height} mm page."),
                &[("bleed", &format!("{:.1}", self.bleed_mm)), ("width", &mm(w)), ("height", &mm(h))],
            ));
        }
        let Some(c) = self.candidates.get(self.profile) else {
            return Some(
                tl!(
                    "No CMYK profile is installed. Install the printer's profile (for example ISO Coated v2 or Coated FOGRA39) on this computer and save again."
                )
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
    let dpi = f64::from(st.doc.resolution_dpi.max(1.0));
    let page_pt = (f64::from(st.doc.size.width) * 72.0 / dpi, f64::from(st.doc.size.height) * 72.0 / dpi);
    let candidates = candidates(app, &st.doc);
    // Start on the document's own condition when it can be one, else the first that fits.
    let profile = candidates.iter().position(|c| c.fits(PdfXStandard::X1a2001)).unwrap_or(0);
    app.pdf_options = Some(Prompt {
        doc,
        path,
        copy,
        record,
        standard: None,
        convert: true,
        profile,
        bleed_mm: 0.0,
        candidates,
        source,
        mode,
        embedded,
        page_pt,
        section: Section::default(),
        restored: false,
    });
    Ok(())
}

/// The dialog's width, its section list's and the height every section's pane keeps, so the
/// window doesn't jump between sections.
const DIALOG_W: f32 = 700.0;
const SECTION_W: f32 = 130.0;
const PANE_H: f32 = 330.0;
/// The label column inside a pane's groups.
const LABEL_W: f32 = 190.0;

/// The sections of the dialog, in Photoshop's order (it also has Security, which PDF/X forbids).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Compression,
    Output,
    Summary,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::General, Section::Compression, Section::Output, Section::Summary];

    fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Compression => "Compression",
            Section::Output => "Output",
            Section::Summary => "Summary",
        }
    }
}

/// The preset that sets `standard`, as Photoshop names its PDF presets.
fn preset_name(standard: PdfXStandard) -> &'static str {
    match standard {
        PdfXStandard::X1a2001 => "[PDF/X-1a:2001]",
        PdfXStandard::X3_2002 => "[PDF/X-3:2002]",
        PdfXStandard::X4 => "[PDF/X-4:2008]",
    }
}

/// The PDF version a file of `standard` is.
fn compatibility(standard: Option<PdfXStandard>) -> &'static str {
    match standard {
        None => "PDF 1.4",
        Some(PdfXStandard::X1a2001 | PdfXStandard::X3_2002) => "PDF 1.3",
        Some(PdfXStandard::X4) => "PDF 1.6",
    }
}

fn description(standard: Option<PdfXStandard>) -> &'static str {
    match standard {
        None => tl!("A plain PDF for viewing and for printers that don't ask for PDF/X. The image keeps its colour and its embedded profile."),
        Some(PdfXStandard::X1a2001) => tl!(
            "PDF/X-1a:2001 (ISO 15930-1) for print: all colour is CMYK or gray, converted to the printer's output condition. Most print shops accept it. PDF 1.3, an ICC version 2 output profile."
        ),
        Some(PdfXStandard::X3_2002) => tl!(
            "PDF/X-3:2002 (ISO 15930-3) for print: like PDF/X-1a, but colour may also keep its own ICC profile for the printer to convert. PDF 1.3, ICC version 2 profiles."
        ),
        Some(PdfXStandard::X4) => tl!(
            "PDF/X-4:2008 (ISO 15930-7), the current print standard: colour converted to the output condition or kept in its own profile, ICC version 2 or 4 profiles, XMP metadata. PDF 1.6."
        ),
    }
}

/// A right-aligned field label in a column `w` wide, as Photoshop lines up its dialog labels; a
/// longer translation is elided, with the full label as its tooltip.
fn label_cell(ui: &mut egui::Ui, text: &str, w: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::hover());
    let color = if ui.is_enabled() { t.text } else { t.text_faint };
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::TextStyle::Body.resolve(ui.style()), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;
    let pos = rect.right_center() - egui::vec2(galley.size().x, galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    if elided {
        resp.on_hover_text(text);
    }
}

/// A value shown in a field's frame that can't be edited here (it follows from other choices).
fn readonly_field(ui: &mut egui::Ui, t: &crate::theme::Tokens, text: &str, w: f32) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.set_width(w - 12.0);
        ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim)).truncate());
    });
}

/// A titled group of settings, as Photoshop frames them.
fn group(ui: &mut egui::Ui, t: &crate::theme::Tokens, title: &str, enabled: bool, add: impl FnOnce(&mut egui::Ui)) {
    crate::widgets::section_label(ui, title);
    ui.add_enabled_ui(enabled, |ui| {
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.separator)).corner_radius(t.radius_sm).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
    ui.add_space(8.0);
}

fn general_pane(ui: &mut egui::Ui, t: &crate::theme::Tokens, standard: Option<PdfXStandard>) {
    group(ui, t, tl!("Description"), true, |ui| {
        ui.set_min_height(84.0);
        ui.add(egui::Label::new(egui::RichText::new(description(standard)).color(t.text_dim)).wrap());
    });
    group(ui, t, tl!("Options"), true, |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl!(
                    "The image is saved flattened: layers, type and transparency are not kept as such. Keep the document as .psd or .pcraft to edit it later."
                ))
                .color(t.text_dim),
            )
            .wrap(),
        );
    });
}

fn compression_pane(ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    group(ui, t, tl!("Images"), true, |ui| {
        egui::Grid::new("pdf-compression").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            for (label, value) in [
                (tl!("Downsampling:"), tl!("Do Not Downsample")),
                (tl!("Compression:"), tl!("ZIP (lossless)")),
                (tl!("Image Quality:"), tl!("8 bits per channel")),
            ] {
                label_cell(ui, label, LABEL_W);
                readonly_field(ui, t, value, 240.0);
                ui.end_row();
            }
        });
    });
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl!("Print PDFs are written at full resolution with lossless compression, so nothing is lost on the way to the press."))
                .color(t.text_faint),
        )
        .wrap(),
    );
}

/// The colour conversion, destination and PDF/X fields, and the bleed.
#[allow(clippy::too_many_arguments)]
fn output_pane(
    ui: &mut egui::Ui,
    t: &crate::theme::Tokens,
    p: &Prompt,
    standard: Option<PdfXStandard>,
    profile: &mut usize,
    convert: &mut bool,
    bleed: &mut f32,
    problem: Option<&str>,
) {
    let destination = standard.and(p.candidates.get(*profile)).map(|c| c.profile.description.clone());
    group(ui, t, tl!("Color"), true, |ui| {
        egui::Grid::new("pdf-color").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            label_cell(ui, tl!("Color Conversion:"), LABEL_W);
            match standard {
                Some(s) if s.keeps_icc_colour() => {
                    let options = [(true, tl!("Convert to Destination")), (false, tl!("No Conversion"))];
                    crate::widgets::dropdown_literal(ui, "pdf-convert", tl!("Color Conversion:"), convert, &options, 260.0);
                }
                Some(_) => readonly_field(ui, t, tl!("Convert to Destination"), 260.0),
                None => readonly_field(ui, t, tl!("No Conversion"), 260.0),
            }
            ui.end_row();
            label_cell(ui, tl!("Destination:"), LABEL_W);
            match standard {
                Some(s) if !p.candidates.is_empty() => {
                    let labels: Vec<(usize, String)> = p
                        .candidates
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (i, if c.fits(s) { c.label.clone() } else { format!("{} (ICC v{})", c.label, c.profile.version.0) }))
                        .collect();
                    let options: Vec<(usize, &str)> = labels.iter().map(|(i, l)| (*i, l.as_str())).collect();
                    crate::widgets::dropdown_literal(ui, "pdf-output-profile", tl!("Destination:"), profile, &options, 260.0);
                }
                _ => readonly_field(ui, t, tl!("N/A"), 260.0),
            }
            ui.end_row();
            label_cell(ui, tl!("Profile Inclusion Policy:"), LABEL_W);
            let policy = match standard {
                None if p.embedded => tl!("Include Tagged Source Profiles"),
                None => tl!("Don't Include Profiles"),
                Some(s) if s.keeps_icc_colour() && !*convert && !p.keeps_numbers() => tl!("Include Tagged Source Profiles"),
                Some(_) => tl!("Include Destination Profile"),
            };
            readonly_field(ui, t, policy, 260.0);
            ui.end_row();
        });
        if standard.is_some() && problem.is_none() {
            ui.add_space(4.0);
            let hint = if p.keeps_numbers() {
                tl!("The CMYK numbers are kept.")
            } else if standard.is_some_and(PdfXStandard::keeps_icc_colour) && !*convert {
                tl!("The colours keep the document's profile, embedded in the PDF; the printer converts them.")
            } else {
                tl!("The colours are converted to this profile in the PDF; the document is not changed.")
            };
            ui.add(egui::Label::new(egui::RichText::new(hint).color(t.text_faint)).wrap());
        }
    });
    group(ui, t, "PDF/X", standard.is_some(), |ui| {
        egui::Grid::new("pdf-x").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            let na = tl!("N/A");
            label_cell(ui, tl!("Output Intent Profile Name:"), LABEL_W);
            readonly_field(ui, t, destination.as_deref().unwrap_or(na), 260.0);
            ui.end_row();
            label_cell(ui, tl!("Output Condition Identifier:"), LABEL_W);
            readonly_field(ui, t, if standard.is_some() { "Custom" } else { na }, 260.0);
            ui.end_row();
            label_cell(ui, tl!("Bleed:"), LABEL_W);
            ui.horizontal(|ui| {
                crate::widgets::value_field(ui, bleed, 0.0..=50.0, "mm", 70.0);
                ui.add(egui::Label::new(egui::RichText::new(tl!("already in the document")).color(t.text_faint)).truncate());
            });
            ui.end_row();
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn summary_pane(
    ui: &mut egui::Ui,
    t: &crate::theme::Tokens,
    p: &Prompt,
    standard: Option<PdfXStandard>,
    profile: usize,
    convert: bool,
    bleed: f32,
    problem: Option<&str>,
) {
    let mm = |pt: f64| pt * 25.4 / 72.0;
    let (w, h) = (mm(p.page_pt.0), mm(p.page_pt.1));
    let b = f64::from(bleed);
    let na = tl!("N/A").to_string();
    let rows: Vec<(&str, String)> = vec![
        (tl!("Standard:"), standard.map_or_else(|| tl!("None").to_string(), |s| s.name().to_string())),
        (tl!("Compatibility:"), compatibility(standard).to_string()),
        (tl!("Destination:"), standard.and(p.candidates.get(profile)).map_or(na.clone(), |c| c.profile.description.clone())),
        (
            tl!("Color Conversion:"),
            match standard {
                Some(s) if s.keeps_icc_colour() && !convert => tl!("No Conversion").to_string(),
                Some(_) => tl!("Convert to Destination").to_string(),
                None => tl!("No Conversion").to_string(),
            },
        ),
        (tl!("Page:"), format!("{w:.1} × {h:.1} mm")),
        (tl!("Trim:"), if standard.is_some() { format!("{:.1} × {:.1} mm", w - 2.0 * b, h - 2.0 * b) } else { na }),
    ];
    group(ui, t, tl!("Settings"), true, |ui| {
        egui::Grid::new("pdf-summary").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (label, value) in &rows {
                label_cell(ui, label, 140.0);
                ui.add(egui::Label::new(egui::RichText::new(value).color(t.text)).truncate());
                ui.end_row();
            }
        });
    });
    group(ui, t, tl!("Warnings"), true, |ui| match problem {
        Some(problem) => {
            ui.add(egui::Label::new(egui::RichText::new(problem).color(t.warning)).wrap());
        }
        None => {
            ui.label(egui::RichText::new(tl!("None")).color(t.text_dim));
        }
    });
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
    let (mut standard, mut profile, mut bleed, mut convert, mut section) = (p.standard, p.profile, p.bleed_mm, p.convert, p.section);
    let problem = p.problem();
    let can_save = problem.is_none();
    let modal = egui::Modal::new(egui::Id::unique("pdf-options")).show(ctx, |ui| {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.set_width(DIALOG_W);
        ui.label(egui::RichText::new(tl!("Save as PDF")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        // The preset, the standard it sets and the PDF version that follows, as Photoshop heads its
        // Save Adobe PDF dialog.
        egui::Grid::new("pdf-head").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
            label_cell(ui, tl!("PDF Preset:"), 96.0);
            let presets: Vec<(Option<PdfXStandard>, &str)> =
                std::iter::once((None, tl!("[High Quality Print]"))).chain(PdfXStandard::ALL.iter().map(|s| (Some(*s), preset_name(*s)))).collect();
            crate::widgets::dropdown_literal(ui, "pdf-preset", tl!("PDF Preset:"), &mut standard, &presets, 300.0);
            ui.label("");
            ui.label("");
            ui.end_row();
            label_cell(ui, tl!("Standard:"), 96.0);
            let standards: Vec<(Option<PdfXStandard>, &str)> =
                std::iter::once((None, tl!("None"))).chain(PdfXStandard::ALL.iter().map(|s| (Some(*s), s.name()))).collect();
            crate::widgets::dropdown_literal(ui, "pdf-standard", tl!("Standard:"), &mut standard, &standards, 300.0);
            label_cell(ui, tl!("Compatibility:"), 92.0);
            readonly_field(ui, &t, compatibility(standard), 110.0);
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            // The section list, as Preferences draws it.
            ui.vertical(|ui| {
                ui.set_width(SECTION_W);
                for s in Section::ALL {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(SECTION_W, 24.0), egui::Sense::click());
                    let sel = section == s;
                    if sel {
                        ui.painter().rect_filled(rect, t.radius_sm, t.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                    }
                    let color = if sel { t.text } else { t.text_dim };
                    let mut job = egui::text::LayoutJob::simple_singleline(tl!(s.title()).to_string(), crate::theme::medium(12.5), color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 12.0);
                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(rect.left_center() + egui::vec2(8.0, -galley.size().y / 2.0), galley, color);
                    if resp.clicked() {
                        section = s;
                    }
                }
            });
            crate::widgets::vline(ui, PANE_H);
            ui.vertical(|ui| {
                ui.set_width(DIALOG_W - SECTION_W - 24.0);
                ui.set_min_height(PANE_H);
                ui.label(egui::RichText::new(tl!(section.title())).font(crate::theme::semibold(14.0)).color(t.text));
                ui.add_space(6.0);
                match section {
                    Section::General => general_pane(ui, &t, standard),
                    Section::Compression => compression_pane(ui, &t),
                    Section::Output => output_pane(ui, &t, p, standard, &mut profile, &mut convert, &mut bleed, problem.as_deref()),
                    Section::Summary => summary_pane(ui, &t, p, standard, profile, convert, bleed, problem.as_deref()),
                }
            });
        });
        ui.add_space(6.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        // What stops the save, on its own line where it can't be missed whatever section shows.
        if let Some(problem) = &problem {
            ui.add(egui::Label::new(egui::RichText::new(problem.as_str()).color(t.warning)).wrap());
            ui.add_space(6.0);
        }
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
    p.section = section;
    // Checked again with this frame's edits: a standard picked in the same frame as Save PDF may
    // not fit the chosen profile.
    let can_save = p.problem().is_none();
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    let enter = can_save && !cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if (ok && can_save) || enter {
        let Some(p) = app.pdf_options.take() else { return };
        // Nothing written: the prompt stays, with its destination, for another answer.
        let fail = |app: &mut PhotocraftApp, p: Prompt, e: String| {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
            app.pdf_options = Some(p);
        };
        let chosen = match params(&p) {
            Ok(chosen) => chosen,
            Err(e) => return fail(app, p, e),
        };
        let mut settings = app.save_settings();
        let doc = app.session.documents().iter().find(|d| d.doc.id == p.doc).map(|d| d.doc.clone());
        match (photocraft_engine::file_cmds::PdfRequest::resolve(&app.session.color, &chosen), doc) {
            (Ok(Some(request)), Some(doc)) => settings.pdf = request.options_for(&doc),
            (Ok(_), Some(_)) => {}
            (Err(e), _) => return fail(app, p, e.to_string()),
            // The document is gone: nothing to ask about any more.
            (_, None) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", tl!("The document is closed"))]);
                app.ui.status_error = true;
                return;
            }
        }
        let spec = p.candidates.get(p.profile).map(|c| c.spec.clone()).unwrap_or_default();
        let id = p.standard.map_or("none", PdfXStandard::id).to_string();
        ctx.data_mut(|d| d.insert_persisted::<Remembered>(remembered_id(), (id, spec, p.bleed_mm, p.convert)));
        let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        // An action being recorded plays the save back with the same standard and condition.
        chosen.record(&mut step);
        let path = p.path.clone();
        let written = app.with_document(p.doc, |app| {
            // A Save As that keeps the document's own name records only its folder (#5305).
            app.record_answered_save_as(p.record, &mut step);
            app.write_document(path, &settings, p.copy)
        });
        if written.is_ok() {
            // An action being recorded keeps the Save As or Save a Copy.
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
                // The prompt stays, with its destination, for another standard or condition.
                app.pdf_options = Some(p);
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
