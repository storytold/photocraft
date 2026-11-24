use craft::behavior::EditEvent;
use craft::ui::dialog::{DialogId, DialogResult, StackDialog as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, NumberUnit, Param, ParamKind};
use craft::ui::runtime::env::UnitKind;
use craft::ui::runtime::requests::PathKind;
use craft_print::ui::{Duplex, HostedPrint, PrintAnswer, PrintFinish, PrintMarks, PrintSection, PrintSheet, Printer, Settings, State};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// The command whose layout the preview and the drag use.
const COMMAND: &str = "file.print";

/// Keys of `file.print` that the print panel's `Settings` carries instead of a section row.
const SETTINGS_KEYS: [&str; 4] = ["printer", "copies", "duplex", "grayscale"];

/// Number fields: a remembered whole number (`100`) is still a number field's value.
const NUMERIC_KEYS: [&str; 4] = ["top", "left", "scale", "bleed"];

/// The printers as the print panel lists them: the default printer first, CUPS queue names (which
/// stand in for spaces with `_`) shown with spaces. Which printer is chosen is `State::poll`'s rule.
pub(super) fn printer_list(list: Option<&Value>) -> Vec<Printer> {
    let named: Vec<(String, bool)> = list
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("default").and_then(Value::as_bool).unwrap_or(false)))).collect())
        .unwrap_or_default();
    default_first(named)
}

/// `(name, is default)` pairs as panel printers, the default ones first and the rest in order.
fn default_first(mut named: Vec<(String, bool)>) -> Vec<Printer> {
    named.sort_by_key(|(_, default)| !*default);
    named.into_iter().map(|(name, default)| Printer::new(name.clone()).label(name.replace('_', " ")).is_default(default)).collect()
}

/// The canonical `PAPERS` name a remembered paper name stands for (`file.print` remembers the name
/// as typed, aliases included); an unknown name stays as it is.
fn paper_key(name: &str) -> String {
    match photocraft_engine::print_cmds::paper_size(name) {
        Some(size) => photocraft_engine::print_cmds::PAPERS.iter().find(|p| (p.1, p.2) == size).map_or_else(|| name.to_string(), |p| p.0.to_string()),
        None => name.to_string(),
    }
}

/// The dialog value of a remembered `file.print` parameter, when it has one.
fn field_value(key: &str, v: &Value) -> Option<FieldValue> {
    match v {
        Value::Null => None,
        Value::Number(n) if NUMERIC_KEYS.contains(&key) => n.as_f64().map(FieldValue::Number),
        _ => serde_json::from_value(v.clone()).ok(),
    }
}

/// The panel settings of the last print's parameters.
fn last_settings(m: &Map<String, Value>) -> Settings {
    let mut settings = Settings::default();
    if let Some(printer) = m.get("printer").and_then(Value::as_str) {
        settings.printer = printer.to_string();
    }
    if let Some(copies) = m.get("copies").and_then(Value::as_u64) {
        settings.copies = copies.clamp(1, 999) as u32;
    }
    settings.duplex = match m.get("duplex").and_then(Value::as_str) {
        Some("off") => Some(Duplex::Off),
        Some("longEdge") => Some(Duplex::LongEdge),
        Some("shortEdge") => Some(Duplex::ShortEdge),
        _ => None,
    };
    settings.grayscale = m.get("grayscale").and_then(Value::as_bool).unwrap_or(false);
    settings
}

/// Writes the panel's settings into the `file.print` fields that carry them.
fn write_settings(fields: &mut DialogFields, settings: &Settings) {
    fields.set("printer", FieldValue::Text(settings.printer.clone()));
    fields.set("copies", FieldValue::Int(i64::from(settings.copies)));
    match settings.duplex {
        Some(Duplex::Off) => fields.set("duplex", FieldValue::Text("off".into())),
        Some(Duplex::LongEdge) => fields.set("duplex", FieldValue::Text("longEdge".into())),
        Some(Duplex::ShortEdge) => fields.set("duplex", FieldValue::Text("shortEdge".into())),
        _ => {
            fields.remove("duplex");
        }
    }
    fields.set("grayscale", FieldValue::Bool(settings.grayscale));
}

/// Opens the Print dialog seeded from the last print (also from an earlier session), filling
/// missing fields with defaults. The printer list is asked for by the dialog's first pass
/// (`take`). Returns the dialog's id without submitting a print job.
pub fn open_print(app: &mut PhotocraftApp) -> DialogId {
    let last = photocraft_engine::print_cmds::last_print(&app.session);
    let mut fields = DialogFields::new();
    let mut state = State::default();
    let mut paper = Value::String("letter".into());
    if let Some(Value::Object(m)) = &last {
        state.settings = last_settings(m);
        if let Some(p) = m.get("paper") {
            paper = p.clone();
        }
        for (k, v) in m {
            if k == "paper" || SETTINGS_KEYS.contains(&k.as_str()) {
                continue;
            }
            if let Some(value) = field_value(k, v) {
                fields.set(k.clone(), value);
            }
        }
    }
    // A custom paper (an array in points) is the `custom` choice with its two sizes; a named paper
    // also seeds the sizes, so choosing Custom starts from the paper that was shown.
    let (name, (w, h)) = match &paper {
        Value::Array(_) => ("custom".to_string(), print_paper_size(Some(&paper))),
        Value::String(n) => (paper_key(n), print_paper_size(Some(&paper))),
        _ => ("letter".to_string(), print_paper_size(None)),
    };
    fields.set("paper", FieldValue::Text(name));
    fields.set("paperWidth", FieldValue::Number(w));
    fields.set("paperHeight", FieldValue::Number(h));
    for (k, v) in [
        ("orientation", FieldValue::Text("portrait".into())),
        ("colorHandling", FieldValue::Text("printerManages".into())),
        ("printerProfile", FieldValue::Text("coated-cmyk".into())),
        ("intent", FieldValue::Text("relative".into())),
        ("bpc", FieldValue::Bool(true)),
        ("center", FieldValue::Bool(true)),
        ("top", FieldValue::Number(0.0)),
        ("left", FieldValue::Number(0.0)),
        ("scale", FieldValue::Number(100.0)),
        ("scaleToFit", FieldValue::Bool(false)),
        ("cornerCropMarks", FieldValue::Bool(false)),
        ("centerCropMarks", FieldValue::Bool(false)),
        ("registrationMarks", FieldValue::Bool(false)),
        ("description", FieldValue::Bool(false)),
        ("labels", FieldValue::Bool(false)),
        ("emulsionDown", FieldValue::Bool(false)),
        ("output", FieldValue::Text(String::new())),
    ] {
        if !fields.contains(k) {
            fields.set(k, v);
        }
    }
    let sections = sections(&fields);
    app.framework.dialogs.open(HostedPrint::new(state, fields, sections).json_form(|fields, listing| {
        if fields.text("paper") == Some("custom") {
            listing.insert("paper".into(), print_params(fields)["paper"].clone());
        }
    }))
}

/// The dialog's sections under the printer panel, as rows over today's `file.print` keys.
/// Lengths are shown in the unit `file.print` keeps them in: positions in inches, the custom paper
/// in points.
pub(super) fn sections(fields: &DialogFields) -> Vec<PrintSection> {
    let choice = |options: &[(&str, &'static str)], default: &str| {
        ParamKind::choice(options.iter().map(|(k, l)| ((*k).to_string(), (*l).into())).collect(), Some(default.to_string()))
    };
    let limit = photocraft_engine::print_cmds::MAX_POSITION_INCHES;

    // Photoshop's Printer Profile menu: the built-ins, then every installed printer (output class)
    // profile by its description.
    let mut profiles: Vec<(String, craft::ui::runtime::env::Label)> = vec![
        ("coated-cmyk".into(), tl!("Coated CMYK").into()),
        ("srgb".into(), "sRGB IEC61966-2.1".into()),
        ("adobe-rgb-compat".into(), tl!("Adobe RGB (1998) compatible").into()),
        ("display-p3".into(), tl!("Display P3").into()),
        ("gray-gamma-2.2".into(), tl!("Gray Gamma 2.2").into()),
    ];
    profiles.extend(
        photocraft_engine::installed_profiles::installed()
            .into_iter()
            .filter(|p| p.is_destination() && p.class == craft::cms::ProfileClass::Output)
            .map(|p| (p.path.clone(), p.description.clone().into())),
    );
    // A remembered profile that is no longer installed still shows, by its file name.
    if let Some(current) = fields.text("printerProfile")
        && !profiles.iter().any(|(k, _)| k == current)
    {
        let stem = std::path::Path::new(current).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        profiles.push((current.to_string(), stem.into()));
    }

    let mut papers: Vec<(String, craft::ui::runtime::env::Label)> =
        photocraft_engine::print_cmds::PAPERS.iter().map(|p| (p.0.to_string(), print_paper_label(p.0).into())).collect();
    papers.push(("custom".into(), tl!("Custom").into()));

    // The rows are shown in the section's Units menu; stored values keep their units (paper in points, position in
    // inches). A typed zero reaches the value, and the engine refuses a paper that small.
    let lengths = |per_unit: f64| {
        vec![
            NumberUnit::new("cm", UnitKind::Centimetres, tl!("Centimeters"), per_unit / 2.54),
            NumberUnit::new("mm", UnitKind::Millimetres, tl!("Millimeters"), per_unit / 25.4),
            NumberUnit::new("in", UnitKind::Inches, tl!("Inches"), per_unit),
        ]
    };
    let pt = |key: &str, label: &'static str| {
        Param::number_in_units(key, label, 612.0, 1.0, 2, "units", lengths(72.0))
            .bounds(Some(0.0), Some(14400.0))
            .visible_when("paper", FieldValue::Text("custom".into()))
    };
    let position = |key: &str, label: &'static str| {
        Param::number_in_units(key, label, 0.0, 0.1, 2, "units", lengths(1.0)).bounds(Some(-limit), Some(limit)).enabled_when("center", FieldValue::Bool(false))
    };
    let on = |key: &str| (key.to_string(), FieldValue::Text("photocraftManages".into()));
    let (on_key, on_value) = on("colorHandling");
    let mark = |key: &str, label: &'static str| Param::new(key, label, ParamKind::bool(false));

    vec![
        PrintSection::new(
            tl!("Color Management"),
            vec![
                Param::new(
                    "colorHandling",
                    tl!("Color Management"),
                    choice(
                        &[
                            ("printerManages", tl!("Printer Manages Colors")),
                            ("photocraftManages", tl!("PhotoCraft Manages Colors")),
                            ("noColorManagement", tl!("No Color Management")),
                        ],
                        "printerManages",
                    ),
                ),
                Param::new("printerProfile", tl!("Profile"), ParamKind::choice(profiles, Some("coated-cmyk".into())))
                    .visible_when(on_key.clone(), on_value.clone()),
                Param::new(
                    "intent",
                    tl!("Mode"),
                    choice(
                        &[
                            ("perceptual", tl!("Perceptual")),
                            ("relative", tl!("Relative Colorimetric")),
                            ("saturation", tl!("Saturation")),
                            ("absolute", tl!("Absolute Colorimetric")),
                        ],
                        "relative",
                    ),
                )
                .visible_when(on_key.clone(), on_value.clone()),
                Param::new("bpc", tl!("Black Point Compensation"), ParamKind::bool(true)).visible_when(on_key, on_value),
            ],
        ),
        PrintSection::new(
            tl!("Position and Size"),
            vec![
                Param::new("units", tl!("Units"), choice(&[("cm", tl!("Centimeters")), ("mm", tl!("Millimeters")), ("in", tl!("Inches"))], "cm")),
                Param::new("paper", tl!("Paper"), ParamKind::choice(papers, Some("letter".into()))),
                pt("paperWidth", tl!("Paper Width")),
                pt("paperHeight", tl!("Paper Height")),
                Param::new("orientation", tl!("Orientation"), choice(&[("portrait", tl!("Portrait")), ("landscape", tl!("Landscape"))], "portrait")),
                Param::new("center", tl!("Center"), ParamKind::bool(true)),
                // Shown while Center owns them, so choosing manual placement cannot resize the dialog.
                position("top", tl!("Top")),
                position("left", tl!("Left")),
                Param::new("scaleToFit", tl!("Scale to Fit Media"), ParamKind::bool(false)),
                Param::new("scale", tl!("Scale (%)"), ParamKind::number(Some(1.0), Some(1000.0), 100.0, 1.0, 2))
                    .enabled_when("scaleToFit", FieldValue::Bool(false)),
            ],
        ),
        PrintSection::new(
            tl!("Printing Marks"),
            vec![
                mark("cornerCropMarks", tl!("Corner Crop Marks")),
                mark("centerCropMarks", tl!("Center Crop Marks")),
                mark("registrationMarks", tl!("Registration Marks")),
                mark("description", tl!("Description")),
                mark("labels", tl!("Labels")),
            ],
        ),
        PrintSection::new(tl!("Functions"), vec![mark("emulsionDown", tl!("Emulsion Down"))]),
        PrintSection::new(
            tl!("Save as PDF"),
            vec![Param::new("output", tl!("Save as PDF"), ParamKind::text("")).hint(tl!("(print to the printer)")).browse(PathKind::SaveFile)],
        ),
    ]
}

/// The system's printers, as the engine lists them without waiting: the driver list where the
/// driver API answers (Windows), else the spooler's.
fn take_printer_lists(app: &mut PhotocraftApp) -> Vec<Printer> {
    let system = photocraft_engine::print_cmds::printers_without_waiting(&mut app.session);
    match photocraft_engine::print_cmds::rgb_printers_without_waiting(&app.session) {
        Ok(list) => default_first(list.iter().map(|p| (p.name.clone(), p.default)).collect()),
        Err(_) => printer_list(system.get("printers")),
    }
}

/// Answers the dialog's request for a printer list. The list goes through a channel to
/// `HostedPrint::listen`, so `State::poll`'s choice rule picks the printer. While another print holds the spooler the
/// list is the cached one (or empty), and the dialog asks again on its next pass, after a short wait, so the real
/// list arrives once the spooler is free. A cached list without the chosen printer is not sent, so the chosen
/// printer and its driver settings stay, and Print stays unavailable, until a fresh list arrives. A fresh list
/// without the chosen printer replaces it (an empty list leaves no printer chosen), and the driver settings of the
/// replaced printer are dropped. With no printer chosen (the system's default), the driver settings stay.
fn refresh_printers(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (list, listed) = match photocraft_engine::print_cmds::try_with_spooler(|| take_printer_lists(app)) {
        Some(list) => (list, true),
        None => (take_printer_lists(app), false),
    };
    if let Some(dialog) = app.framework.dialogs.get_mut::<HostedPrint>() {
        let chosen = dialog.json_fields().and_then(|f| f.get("printer").and_then(Value::as_str).map(str::to_owned)).unwrap_or_default();
        let missing = !chosen.is_empty() && !list.iter().any(|p| p.name == chosen);
        let replaces = listed && missing;
        if listed || !missing {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(list);
            if dialog.listen(rx) && replaces {
                dialog.fields_mut().remove("printerSettings");
            }
        }
        if !listed {
            dialog.request_refresh();
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

/// The print dialog's label for a paper: the familiar name and, for the Japanese papers, the
/// customary name with its metric size.
fn print_paper_label(name: &'static str) -> &'static str {
    match name {
        "letter" => "Letter",
        "legal" => "Legal",
        "tabloid" => "Tabloid",
        "a3" => "A3",
        "a4" => "A4",
        "a5" => "A5",
        "a6" => "A6",
        "4x6" => "4 × 6 in",
        "5x7" => "5 × 7 in",
        "postcard" => "はがき (100 × 148 mm)",
        "double-postcard" => "往復はがき (148 × 200 mm)",
        "b5" => "JIS B5 (182 × 257 mm)",
        "b4" => "JIS B4 (257 × 364 mm)",
        "l" => "L判 (89 × 127 mm)",
        "2l" => "2L判 (127 × 178 mm)",
        "kg" => "KG判 (102 × 152 mm)",
        "6cut" => "六切 (203 × 254 mm)",
        "4cut" => "四切 (254 × 305 mm)",
        other => other,
    }
}

/// Unrotated paper dimensions in points, for either the current preset or a custom array.
fn print_paper_size(value: Option<&Value>) -> (f64, f64) {
    let default = (612.0, 792.0);
    match value {
        Some(Value::Array(a)) => (a.first().and_then(Value::as_f64).unwrap_or(default.0), a.get(1).and_then(Value::as_f64).unwrap_or(default.1)),
        Some(Value::String(name)) => photocraft_engine::print_cmds::paper_size(name).unwrap_or(default),
        _ => default,
    }
}

/// Save as PDF › Browse…: where to write the PDF. The answer goes to the open print dialog's
/// `output` row when the file dialog replies. Without it the only way to save a PDF was to type a
/// full path (#5328).
fn browse_pdf(app: &mut PhotocraftApp, suggested: &str) {
    let r = app.pick_save(suggested, move |app, path| {
        let dialog = app.framework.dialogs.get_mut::<HostedPrint>().ok_or("the dialog was closed")?;
        dialog.requests_mut().answer_path("output", std::path::PathBuf::from(path));
        Ok(Value::Null)
    });
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
    }
}

/// Moves the image box to `position`, its top-left corner on the shown paper in paper points,
/// by turning it into the manual Left and Top (inches). Emulsion Down mirrors the shown x; `layout`
/// is the layout the box was drawn from. Center is turned off and the offsets clamp to the
/// engine's safety bound. A position equal to where the box already is, or a nonfinite one,
/// changes nothing and returns false: a click without a drag leaves Center on.
pub(super) fn drag_print_position(f: &mut DialogFields, position: egui::Pos2, layout: &photocraft_engine::print_cmds::PrintLayout) -> bool {
    let (px, py) = (f64::from(position.x), f64::from(position.y));
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    let (paper_w, paper_h) = layout.paper;
    let (x, y, w, h) = layout.rect;
    let shown_x = if layout.emulsion_down { paper_w - x - w } else { x };
    let shown_y = paper_h - y - h;
    if (px - shown_x).abs() < 0.01 && (py - shown_y).abs() < 0.01 {
        return false;
    }
    let left = (if layout.emulsion_down { paper_w - px - w } else { px }) / 72.0;
    let top = py / 72.0;
    if !left.is_finite() || !top.is_finite() {
        return false;
    }
    let limit = photocraft_engine::print_cmds::MAX_POSITION_INCHES;
    fields_set_position(f, left.clamp(-limit, limit), top.clamp(-limit, limit));
    true
}

fn fields_set_position(f: &mut DialogFields, left: f64, top: f64) {
    f.set("center", FieldValue::Bool(false));
    f.set("left", FieldValue::Number(left));
    f.set("top", FieldValue::Number(top));
}

/// The document's flattened picture at the preview size, mirrored for Emulsion Down (the sheet
/// draws the picture unmirrored in the box). The texture lives in the context's memory under the
/// document's id and revision, and the scratch read-error generation (`crate::scratch_reads`).
fn picture(ctx: &egui::Context, doc: &photocraft_doc::Document, revision: u64, mirror: bool) -> egui::load::SizedTexture {
    let key = egui::Id::unique(("print-thumb", doc.id.0, revision, mirror, crate::scratch_reads::generation()));
    let tex: Option<std::sync::Arc<egui::TextureHandle>> = ctx.data(|d| d.get_temp(key));
    let tex = tex.unwrap_or_else(|| {
        let th = photocraft_compose::thumbnail(doc, 400);
        let (w, h) = (th.width as usize, th.height as usize);
        let mut pixels = th.pixels.clone();
        if mirror {
            for row in pixels.chunks_exact_mut(w * 4) {
                let mut mirrored = Vec::with_capacity(row.len());
                for px in row.as_chunks::<4>().0.iter().rev() {
                    mirrored.extend_from_slice(px);
                }
                row.copy_from_slice(&mirrored);
            }
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
        let handle = std::sync::Arc::new(ctx.load_texture("print-thumb", image, egui::TextureOptions::LINEAR));
        ctx.data_mut(|d| d.insert_temp(key, handle.clone()));
        handle
    });
    egui::load::SizedTexture::new(tex.id(), egui::vec2(tex.size()[0] as f32, tex.size()[1] as f32))
}

/// The sheet the preview draws from the engine's shared layout, in paper points with the top-left
/// origin; `None` when the fields describe no valid layout.
fn build_sheet(ctx: &egui::Context, doc: &photocraft_doc::Document, revision: u64, fields: &DialogFields) -> Option<PrintSheet> {
    let l = photocraft_engine::print_cmds::layout(doc, &print_params(fields), COMMAND).ok()?;
    let (pw, ph) = l.paper;
    let (x, y, w, h) = l.rect;
    let shown_x = if l.emulsion_down { pw - x - w } else { x };
    let image = egui::Rect::from_min_size(egui::pos2(shown_x as f32, (ph - y - h) as f32), egui::vec2(w as f32, h as f32));
    let fits = x >= 0.0 && y >= 0.0 && x + w <= pw && y + h <= ph;
    let note = if fits { "" } else { "  ·  larger than the paper" };
    let readout = format!("Scale {:.1}%  ·  {:.2} × {:.2} in{note}", l.scale * 100.0, w / 72.0, h / 72.0);
    let mut marks = PrintMarks::default();
    marks.corner_crop = l.marks.corner_crop;
    marks.center_crop = l.marks.center_crop;
    marks.registration = l.marks.registration;
    marks.description = fields.bool("description").unwrap_or(false);
    marks.labels = fields.bool("labels").unwrap_or(false);
    Some(PrintSheet::new(egui::vec2(pw as f32, ph as f32), image, picture(ctx, doc, revision, l.emulsion_down), readout.into()).marks(marks))
}

/// Serves the open Print dialog and files its answer; called once per frame by `file_ui::take`.
///
/// It drains the dialog's request queues (`take_refresh`, `take_moves`, the Browse request), so a
/// second pass in the same frame finds them empty and applies only what its own input raised.
/// The sheet is rebuilt only when the fields' parameters or the document's revision differ from
/// the last sheet's; the open dialog gets it every pass.
pub(super) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    match app.framework.dialogs.take::<HostedPrint>() {
        Some(DialogResult::Ok(answer)) => return finish(app, answer),
        Some(DialogResult::Cancel) => return,
        // `DialogResult` is non-exhaustive; a result this code does not know files nothing.
        Some(_) => return,
        None => {}
    }
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedPrint>() else { return };
    let refresh = dialog.take_refresh();
    let moves = dialog.take_moves();
    let browse = dialog.requests_mut().take_path();
    let mut fields = dialog.fields_mut().clone();
    if refresh {
        refresh_printers(app, ctx);
    }
    let Some((doc, revision)) = app.session.active().map(|d| (d.doc.clone(), d.revision)) else { return };
    let mut moved = false;
    for event in moves.iter() {
        let position = match event {
            EditEvent::Update { value, .. } => *value,
            EditEvent::Commit { after, .. } => *after,
            _ => continue,
        };
        if let Ok(l) = photocraft_engine::print_cmds::layout(&doc, &print_params(&fields), COMMAND) {
            moved |= drag_print_position(&mut fields, position, &l);
        }
    }
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedPrint>() else { return };
    if moved {
        for key in ["center", "left", "top"] {
            if let Some(v) = fields.get(key) {
                dialog.fields_mut().set(key, v.clone());
            }
        }
    }
    // The last sheet built, with the parameters and revision it was built from. Every pass hands the open dialog the
    // sheet: a dialog opened again over the same settings starts without one and shows nothing until it has it.
    let key = format!("{}|{}|{revision}", print_params(&fields), doc.id.0);
    let slot = egui::Id::unique("print-sheet");
    let cached = ctx.data(|d| d.get_temp::<(String, PrintSheet)>(slot)).filter(|(built, _)| *built == key).map(|(_, sheet)| sheet);
    let sheet = cached.or_else(|| {
        let sheet = build_sheet(ctx, &doc, revision, &fields)?;
        ctx.data_mut(|d| d.insert_temp(slot, (key, sheet.clone())));
        Some(sheet)
    });
    if let Some(sheet) = sheet {
        dialog.set_sheet(sheet);
    }
    if browse.is_some() {
        let current = fields.text("output").unwrap_or_default().to_string();
        let suggested = if current.is_empty() { std::path::Path::new(&doc.name).with_extension("pdf").to_string_lossy().into_owned() } else { current };
        browse_pdf(app, &suggested);
    }
}

/// Files the dialog's answer: Print runs `file.print`, Done keeps the settings without printing.
/// The panel's printer, copies, two-sided mode and grayscale go into the parameters.
fn finish(app: &mut PhotocraftApp, answer: PrintAnswer) {
    let mut fields = answer.fields;
    write_settings(&mut fields, &answer.settings);
    match answer.finish {
        PrintFinish::Print => {
            if let Err(e) = print_confirm(app, &fields) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        PrintFinish::Done | _ => photocraft_engine::print_cmds::remember_print(&mut app.session, &print_params(&fields)),
    }
}

/// `file.print`'s params from the dialog's fields: a custom paper goes back to `[w, h]` in points,
/// a named paper stays a name; the custom size rows themselves are not parameters.
pub(super) fn print_params(f: &DialogFields) -> Value {
    let mut p = Map::new();
    for (k, v) in f.iter() {
        if matches!(k, "paperWidth" | "paperHeight") {
            continue;
        }
        if let Ok(v) = serde_json::to_value(v) {
            p.insert(k.to_string(), v);
        }
    }
    // The ruler unit is the dialog's own display setting; `file.print` takes no `units`.
    p.remove("units");
    // The driver's printable area belongs to a print to the printer, not to a file or a dry run.
    if f.text("output").is_some_and(|output| !output.is_empty()) || f.bool("dryRun") == Some(true) {
        p.remove("printable");
    }
    if f.text("paper") == Some("custom") {
        let (default_w, default_h) = print_paper_size(None);
        p.insert("paper".into(), json!([f.number("paperWidth").unwrap_or(default_w), f.number("paperHeight").unwrap_or(default_h)]));
    }
    Value::Object(p)
}

/// Submits the dialog parameters to `file.print`, propagating command errors unchanged.
/// Updates the status for completed results only; pending jobs report completion later.
pub(super) fn print_confirm(app: &mut PhotocraftApp, f: &DialogFields) -> Result<Value, String> {
    let r = app.run("file.print", print_params(f))?;
    if r["pending"] != json!(true) {
        print_status(app, &r);
    }
    Ok(r)
}

pub(crate) fn print_status(app: &mut PhotocraftApp, r: &Value) {
    app.ui.status = if let Some(printer) = r["spooler"]["printer"].as_str() {
        crate::i18n::fmt(tl!("Sent to {printer}"), &[("printer", printer)])
    } else if r["sent"] == json!(true) {
        let template = "Sent to the printer ({spooler})";
        crate::i18n::fmt(tl!(template), &[("spooler", r["spooler"].as_str().unwrap_or(""))])
    } else {
        let template = "Printed to {path}";
        crate::i18n::fmt(tl!(template), &[("path", r["pdf"].as_str().unwrap_or(""))])
    };
    app.ui.status_error = false;
}

#[cfg(test)]
mod scratch_tests {
    /// A tile that fails to read and recovers keeps the document's revision; the preview picture still draws again,
    /// so it does not keep the zeros it read during the error.
    #[test]
    fn the_preview_picture_draws_again_after_a_scratch_read_error_transition() {
        use photocraft_doc::{ColorMode, Document, SampleType};
        let ctx = egui::Context::default();
        let doc = Document::new("Print", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let first = super::picture(&ctx, &doc, 3, false).id;
        assert_eq!(super::picture(&ctx, &doc, 3, false).id, first, "the same revision keeps its picture");
        crate::scratch_reads::tests::simulate_read_error_transition();
        assert_ne!(super::picture(&ctx, &doc, 3, false).id, first, "the picture draws again after the transition");
    }
}
