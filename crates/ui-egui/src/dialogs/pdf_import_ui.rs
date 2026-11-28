//! File › Open on a PDF: Photoshop's Import PDF dialog, as it lays it out. Select Pages or Images
//! with thumbnails (Thumbnail Size Small, Large or Fit Page; click, ⇧-click a range, ⌘/Ctrl-click to
//! toggle), Page Options (Name, Crop To, Anti-aliased), Image Size (Width and Height in a unit,
//! Constrain Proportions, Resolution, Mode, Bit Depth), Suppress Warnings and Open In Reverse
//! Order. OK opens every selected page (or image) as its own document through the ordinary open
//! path, so the tabs, notices and Open Recent behave as for any file. An encrypted file asks for
//! its password first, in a dialog of its own.
//!
//! Both dialogs are hosted parameter dialogs (`dialogs::params`). Import PDF is one custom row
//! that draws Photoshop's two columns; the answers live in the dialog's values (`ui.dialog.set`
//! drives them) and the file's bytes and thumbnail textures in [`Held`], which the row shares.
//! Each opened PDF gets dialog keys of its own, so several PDFs opened together (Open, a drop, the
//! command line) each keep their own file. The import itself is `photocraft_io::pdf_import`, which
//! also holds what was measured on Photoshop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamResult};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::{NumberField, TextField};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_io::pdf_import::{self, CropTo, PdfImage, PdfInfo, PdfOpenOptions};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::dialogs::params;
use crate::theme::Tokens;

/// The key prefix of an Import PDF dialog; the PDF's number follows.
const IMPORT: &str = "pdf.import.";
/// The key prefix of a PDF's password dialog; the PDF's number follows.
const PASSWORD: &str = "pdf.password.";
/// The dialog's width in points (Photoshop's is about 690 px).
pub const WIDTH: f32 = 700.0;
/// Thumbnail Size choices (field value, label).
pub const THUMB_SIZES: &[(&str, &str)] = &[("small", "Small"), ("large", "Large"), ("fitPage", "Fit Page")];
/// Mode choices (field value, label), in Photoshop's order.
pub const MODES: &[(&str, &str)] = &[("gray", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
/// Bit Depth choices.
pub const DEPTHS: &[(i64, &str)] = &[(8, "8 bit"), (16, "16 bit")];
/// Resolution unit choices (field value, label).
pub const RES_UNITS: &[(&str, &str)] = &[("in", "Pixels/Inch"), ("cm", "Pixels/Centimeter")];
/// Width and Height units: (field value, label, units per inch; 0 for pixels).
pub const UNITS: &[(&str, &str, f32)] =
    &[("px", "Pixels", 0.0), ("in", "Inches", 1.0), ("cm", "Centimeters", 2.54), ("mm", "Millimeters", 25.4), ("pt", "Points", 72.0), ("pica", "Picas", 6.0)];
/// The values the row keeps besides its own `pages`, so automation sets them as it sets `pages`.
const COMPANIONS: &[&str] = &[
    "select",
    "images",
    "thumbnailSize",
    "name",
    "crop",
    "antiAlias",
    "unit",
    "pixelSize",
    "constrain",
    "resolution",
    "resolutionUnit",
    "mode",
    "depth",
    "suppressWarnings",
    "reverseOrder",
];
/// Longest side of a thumbnail, per Thumbnail Size (Fit Page follows the panel).
const SMALL: u32 = 64;
const LARGE: u32 = 128;
/// The thumbnail panel's size in points.
const PANEL: egui::Vec2 = vec2(296.0, 290.0);
/// The height of a labelled row of fields in points.
const ROW_H: f32 = 24.0;
/// The largest side the size fields take, in pixels.
const MAX_SIDE: f64 = 300_000.0;

/// The number of the next PDF opened: each gets dialog keys of its own.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// A rendered thumbnail on the GPU.
struct Thumb {
    tex: egui::TextureHandle,
    size: (u32, u32),
}

/// A page's or image's thumbnail: not tried yet, rendered, or refused. A refused one is not
/// tried again until the Thumbnail Size changes, so a page that fails (a damaged stream, an
/// image over the decode budget) neither re-parses the file every frame nor holds back the
/// thumbnails after it.
enum Slot {
    Pending,
    Ready(Thumb),
    Failed,
}

impl Slot {
    fn ready(&self) -> Option<&Thumb> {
        match self {
            Slot::Ready(t) => Some(t),
            Slot::Pending | Slot::Failed => None,
        }
    }

    fn is_pending(&self) -> bool {
        matches!(self, Slot::Pending)
    }
}

/// The file behind a PDF's dialogs: what the values can't hold.
pub struct Held {
    /// The file name (the Name field starts as it).
    pub file_name: String,
    /// Where it was read from (Open Recent), when known.
    pub path: Option<String>,
    pub bytes: Arc<Vec<u8>>,
    /// The pages, once the file could be read (a password may come first).
    pub info: Option<PdfInfo>,
    /// The raster images, listed when Images is first chosen.
    images: Option<Vec<PdfImage>>,
    /// Why the images couldn't be listed, shown in the panel instead of an empty list.
    images_error: Option<String>,
    /// The accepted user password of an encrypted file.
    pub password: Option<String>,
    thumbs: Vec<Slot>,
    image_thumbs: Vec<Slot>,
    /// The Thumbnail Size the textures were rendered for.
    thumb_key: String,
    /// Bounding boxes per page, computed when a page's size is first needed.
    bounds: Vec<Option<[f32; 4]>>,
    /// The item a ⇧-click range starts from, and its list (`true`: Images).
    anchor: Option<(bool, usize)>,
    /// The tab position the PDF was dropped at, where its pages go; `None`: the end.
    slot: Option<usize>,
}

impl Held {
    /// Takes the pages of `info`, read with `password`.
    fn read(&mut self, info: PdfInfo, password: Option<String>) {
        let n = info.pages.len();
        self.info = Some(info);
        self.password = password;
        self.thumbs = (0..n).map(|_| Slot::Pending).collect();
        self.bounds = (0..n).map(|_| None).collect();
    }
}

/// A PDF whose Import PDF (or password) dialog is up.
pub struct PdfImport {
    /// The PDF's number: its dialogs' keys end with it.
    pub number: u64,
    /// The file, shared with the dialog's row.
    pub held: Arc<Mutex<Held>>,
}

fn lock(held: &Mutex<Held>) -> MutexGuard<'_, Held> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `bytes` should open through this dialog: a PDF, unless it is the one-image kind
/// Photoshop opens like a picture without asking.
pub fn wants_dialog(name: &str, bytes: &[u8]) -> bool {
    photocraft_io::is_pdf_input(name, bytes) && (pdf_import::needs_password(bytes) || !pdf_import::opens_as_image(bytes, None))
}

/// Whether the hosted dialog `key` is an Import PDF dialog.
pub(crate) fn owns_import(key: &str) -> bool {
    key.starts_with(IMPORT)
}

/// Whether the hosted dialog `key` is a PDF's password dialog.
pub(crate) fn owns_password(key: &str) -> bool {
    key.starts_with(PASSWORD)
}

/// The PDF number a dialog key ends with.
fn number_of(key: &str) -> Option<u64> {
    key.strip_prefix(IMPORT).or_else(|| key.strip_prefix(PASSWORD))?.parse().ok()
}

/// The held file of the dialog `key`.
fn held_of(app: &PhotocraftApp, key: &str) -> Option<Arc<Mutex<Held>>> {
    let n = number_of(key)?;
    app.pdf_import.iter().find(|p| p.number == n).map(|p| Arc::clone(&p.held))
}

/// Opens the dialog for the PDF `bytes` read from `path` (when known) as `name`. An encrypted
/// file asks for its password first. Returns the dialog's id.
pub fn open(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: &[u8]) -> Result<DialogId, String> {
    open_placed(app, name, path, bytes, None)
}

/// [`open`] for a PDF dropped on a tab slot: the pages it opens start there.
pub fn open_placed(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: &[u8], slot: Option<usize>) -> Result<DialogId, String> {
    let bytes = Arc::new(bytes.to_vec());
    let info = match pdf_import::info(&bytes, None) {
        Ok(i) => Some(i),
        Err(e) if e.to_string().contains(pdf_import::PASSWORD_REQUIRED) => None,
        Err(e) => return Err(e.to_string()),
    };
    let mut held = Held {
        file_name: name.to_string(),
        path: path.map(str::to_string),
        bytes,
        info: None,
        images: None,
        images_error: None,
        password: None,
        thumbs: Vec::new(),
        image_thumbs: Vec::new(),
        thumb_key: String::new(),
        bounds: Vec::new(),
        anchor: None,
        slot,
    };
    let locked = info.is_none();
    if let Some(info) = info {
        held.read(info, None);
    }
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let held = Arc::new(Mutex::new(held));
    app.pdf_import.push(PdfImport { number, held: Arc::clone(&held) });
    Ok(if locked { open_password(app, number) } else { open_import(app, number, held) })
}

/// The password dialog of PDF `number`.
fn open_password(app: &mut PhotocraftApp, number: u64) -> DialogId {
    let key = format!("{PASSWORD}{number}");
    let fields = DialogFields::from_pairs([("password", FieldValue::Text(String::new()))]);
    let dialog_key = key.clone();
    params::open_param_unsaved(app, &key, fields, Value::Null, move |fields| {
        let rows = vec![
            Param::read_only_text("message", Label::Empty, Label::from(tl!("The PDF is protected by a password."))).full_width(),
            Param::custom("password", tl!("Password"), |ui, fields, _| {
                let mut pw = fields.text("password").unwrap_or_default().to_string();
                let before = pw.clone();
                let response = TextField::new(tl!("Password"), Bind::bound(&mut pw)).masked().show(ui).response;
                if pw != before {
                    fields.set("password", FieldValue::Text(pw));
                }
                response
            })
            .opens_with(FieldValue::Text(String::new())),
        ];
        ParamDialog::command(dialog_key.clone(), tl!("Import PDF"), rows).values(Bind::bound(fields))
    })
}

/// Judges the password dialog's OK: the typed password is tried on the file; a wrong one keeps
/// the dialog up with the reason under the field.
pub(crate) fn check_password(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let Some(held) = held_of(app, &result.key) else { return Err(Label::from(tl!("The PDF is no longer available."))) };
    let pw = result.fields.text("password").unwrap_or_default().to_string();
    let mut held = lock(&held);
    match pdf_import::info(&held.bytes, Some(&pw)) {
        Ok(info) => {
            held.read(info, Some(pw));
            Ok(())
        }
        Err(e) if e.to_string().contains(pdf_import::WRONG_PASSWORD) => Err(Label::from(tl!("That isn't the PDF's password."))),
        Err(_) => Err(Label::from(tl!("The PDF couldn't be read."))),
    }
}

/// The password was accepted: Import PDF opens on the pages.
pub(crate) fn run_password(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let (Some(number), Some(held)) = (number_of(&result.key), held_of(app, &result.key)) else { return };
    open_import(app, number, held);
}

/// A cancelled password or Import PDF dialog: its file goes.
pub(crate) fn cancel(app: &mut PhotocraftApp, _ctx: &egui::Context, id: DialogId) {
    // The dialog's id is its key's: find the PDF whose key it is.
    let ours = |n: u64| dialog_id(format!("{IMPORT}{n}").as_str()) == id || dialog_id(format!("{PASSWORD}{n}").as_str()) == id;
    app.pdf_import.retain(|p| !ours(p.number));
}

/// The Import PDF dialog of PDF `number`, over the file `held`.
fn open_import(app: &mut PhotocraftApp, number: u64, held: Arc<Mutex<Held>>) -> DialogId {
    let key = format!("{IMPORT}{number}");
    let name = lock(&held).file_name.clone();
    let fields = defaults(app, &name);
    let dialog_key = key.clone();
    params::open_param_unsaved(app, &key, fields, Value::Null, move |fields| {
        let held = Arc::clone(&held);
        let row = Param::custom("pages", Label::Empty, move |ui, fields, _| body(ui, fields, &held)).full_width().companions(COMPANIONS.iter().copied());
        ParamDialog::command(dialog_key.clone(), tl!("Import PDF"), vec![row]).values(Bind::bound(fields))
    })
}

/// Photoshop's defaults: the first page, Bounding Box, anti-aliased, 300 ppi, RGB, 8 bits, the
/// ruler unit for the size, proportions free, warnings shown, pages opened in reverse order.
fn defaults(app: &PhotocraftApp, name: &str) -> DialogFields {
    let mut f = DialogFields::new();
    f.set("select", FieldValue::Text("pages".into()));
    f.set("pages", FieldValue::List(vec![FieldValue::Int(1)]));
    f.set("images", FieldValue::List(Vec::new()));
    f.set("thumbnailSize", FieldValue::Text("small".into()));
    f.set("name", FieldValue::Text(name.into()));
    f.set("crop", FieldValue::Text(CropTo::BoundingBox.id().into()));
    f.set("antiAlias", FieldValue::Bool(true));
    // The ruler unit, when the size fields know it (percent falls back to pixels).
    let suffix = app.session.prefs().units_and_rulers.rulers.suffix().trim();
    let unit = UNITS.iter().map(|u| u.0).find(|u| *u == suffix).unwrap_or("px");
    f.set("unit", FieldValue::Text(unit.into()));
    f.set("constrain", FieldValue::Bool(false));
    f.set("resolution", FieldValue::Number(f64::from(pdf_import::DEFAULT_RESOLUTION)));
    f.set("resolutionUnit", FieldValue::Text("in".into()));
    f.set("mode", FieldValue::Text("rgb".into()));
    f.set("depth", FieldValue::Int(8));
    f.set("suppressWarnings", FieldValue::Bool(false));
    f.set("reverseOrder", FieldValue::Bool(true));
    f
}

fn s<'a>(f: &'a DialogFields, k: &str, d: &'a str) -> &'a str {
    f.text(k).unwrap_or(d)
}
fn num(f: &DialogFields, k: &str, d: f32) -> f32 {
    f.number(k).map_or(d, |v| v as f32)
}
fn flag(f: &DialogFields, k: &str, d: bool) -> bool {
    f.bool(k).unwrap_or(d)
}
/// A list item as a whole number, in any form the values keep it (automation sets JSON).
fn whole(v: &FieldValue) -> Option<i64> {
    match v {
        FieldValue::Int(i) => Some(*i),
        FieldValue::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
        FieldValue::Json(j) => j.as_i64(),
        _ => None,
    }
}
/// The selected 1-based numbers under `k`, sorted and unique.
fn picks(f: &DialogFields, k: &str) -> Vec<usize> {
    let mut v: Vec<usize> =
        f.list(k).map(|a| a.iter().filter_map(whole).filter(|n| *n >= 1).filter_map(|n| usize::try_from(n).ok()).collect()).unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}
fn set_picks(f: &mut DialogFields, k: &str, picked: &[usize]) {
    f.set(k, FieldValue::List(picked.iter().map(|n| FieldValue::Int(i64::try_from(*n).unwrap_or(i64::MAX))).collect()));
}
/// The Resolution value in pixels per inch, within the importer's range.
fn resolution(f: &DialogFields) -> f32 {
    num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION)
}

/// The options the values describe for page or image `n`.
pub fn options(f: &DialogFields, n: usize, images: bool, password: Option<&str>) -> PdfOpenOptions {
    let mode = match s(f, "mode", "rgb") {
        "gray" => photocraft_color::ColorMode::Grayscale,
        "cmyk" => photocraft_color::ColorMode::Cmyk,
        "lab" => photocraft_color::ColorMode::Lab,
        _ => photocraft_color::ColorMode::Rgb,
    };
    let depth = if f.int("depth") == Some(16) { photocraft_color::SampleType::U16 } else { photocraft_color::SampleType::U8 };
    let sized = pixel_size_field(f);
    PdfOpenOptions {
        page: if images { 1 } else { n },
        image: images.then_some(n),
        crop: CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default(),
        resolution: resolution(f),
        mode,
        depth,
        anti_alias: flag(f, "antiAlias", true),
        name: Some(s(f, "name", "").to_string()).filter(|n| !n.trim().is_empty()),
        pixel_size: sized.filter(|_| !images),
        password: password.map(str::to_string),
        suppress_warnings: flag(f, "suppressWarnings", false),
    }
}

/// The `pixelSize` value as `[width, height]`. A side past `u32` keeps `u32::MAX` rather than
/// wrapping to a small number, so the importer's page cap scales it down with its warning.
fn pixel_size_field(f: &DialogFields) -> Option<(u32, u32)> {
    let side = |v: &FieldValue| {
        let n = match v {
            FieldValue::Json(j) => j.as_u64(),
            other => whole(other).and_then(|n| u64::try_from(n).ok()),
        }?;
        Some(u32::try_from(n).unwrap_or(u32::MAX))
    };
    match f.list("pixelSize")?.as_ref() {
        [w, h] => Some((side(w)?, side(h)?)),
        _ => None,
    }
}

fn set_pixel_size(f: &mut DialogFields, w: u32, h: u32) {
    f.set("pixelSize", FieldValue::List(vec![FieldValue::Int(i64::from(w)), FieldValue::Int(i64::from(h))]));
}

/// The page box the values crop to for page `index` (0-based), in points as displayed.
fn page_size_pt(st: &mut Held, f: &DialogFields, index: usize) -> Option<(f32, f32)> {
    let info = st.info.as_ref()?;
    let page = info.pages.get(index)?;
    let crop = CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default();
    let b = if crop == CropTo::BoundingBox {
        match st.bounds.get(index).copied().flatten() {
            Some(b) => b,
            None => {
                let b = pdf_import::bounding_box(&st.bytes, index + 1, st.password.as_deref()).unwrap_or(page.crop);
                if let Some(slot) = st.bounds.get_mut(index) {
                    *slot = Some(b);
                }
                b
            }
        }
    } else {
        page.box_for(crop)
    };
    Some(page.displayed_size(b))
}

/// The pixel size the dialog shows: the typed one, else the first selected page's box at the
/// resolution.
fn pixel_size(st: &mut Held, f: &DialogFields) -> Option<(u32, u32)> {
    if let Some((w, h)) = pixel_size_field(f) {
        return Some((w.max(1), h.max(1)));
    }
    let first = picks(f, "pages").first().copied()?;
    let pt = page_size_pt(st, f, first.checked_sub(1)?)?;
    Some(pdf_import::pixel_size(pt, resolution(f)))
}

/// "20.8M" for a size in bytes, as Photoshop's Image Size line (1 M = 1024 × 1024).
pub fn size_label(w: u32, h: u32, mode: &str, depth: i64) -> String {
    let channels = match mode {
        "gray" => 1.0,
        "cmyk" => 4.0,
        _ => 3.0,
    };
    let bytes = f64::from(w) * f64::from(h) * channels * if depth == 16 { 2.0 } else { 1.0 };
    // Three significant digits, as Photoshop prints it (20.8M, 3.20M, 512K).
    let three = |v: f64| {
        if v >= 100.0 {
            format!("{v:.0}")
        } else if v >= 10.0 {
            format!("{v:.1}")
        } else {
            format!("{v:.2}")
        }
    };
    if bytes >= 1024.0 * 1024.0 { format!("{}M", three(bytes / (1024.0 * 1024.0))) } else { format!("{}K", three(bytes / 1024.0)) }
}

/// Pixels to the display unit at `ppi`.
fn to_unit(px: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => px / ppi.max(1.0) * per_in,
        _ => px,
    }
}

/// The display unit back to pixels at `ppi`.
fn from_unit(v: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => (v / per_in * ppi.max(1.0)).round(),
        _ => v.round(),
    }
}

/// The thumbnail's longest side in pixels for a Thumbnail Size.
fn thumb_side(size: &str) -> u32 {
    match size {
        "large" => LARGE,
        "fitPage" => (PANEL.x - 40.0) as u32,
        _ => SMALL,
    }
}

/// A thumbnail's texture, or `Failed` when the importer refused it.
fn texture(ctx: &egui::Context, name: String, thumb: Result<pdf_import::Thumbnail, impl std::fmt::Display>) -> Slot {
    match thumb {
        Ok(t) => {
            let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
            Slot::Ready(Thumb { tex: ctx.load_texture(name, img, egui::TextureOptions::LINEAR), size: (t.width, t.height) })
        }
        Err(_) => Slot::Failed,
    }
}

/// Renders the missing thumbnails of the list on show, a few per frame.
fn render_thumbs(st: &mut Held, ctx: &egui::Context, images: bool, size: &str) {
    if st.thumb_key != size {
        st.thumbs.iter_mut().for_each(|t| *t = Slot::Pending);
        st.image_thumbs.iter_mut().for_each(|t| *t = Slot::Pending);
        st.thumb_key = size.to_string();
    }
    if images && st.images.is_none() {
        st.images = Some(match pdf_import::images(&st.bytes, st.password.as_deref()) {
            Ok(v) => v,
            Err(e) => {
                st.images_error = Some(e.to_string());
                Vec::new()
            }
        });
        st.image_thumbs = st.images.as_ref().map_or(Vec::new(), |v| v.iter().map(|_| Slot::Pending).collect());
    }
    let side = thumb_side(size);
    let Held { bytes, password, thumbs, image_thumbs, .. } = st;
    let list = if images { image_thumbs } else { thumbs };
    let mut budget = 3;
    for (i, slot) in list.iter_mut().enumerate() {
        if !slot.is_pending() || budget == 0 {
            continue;
        }
        budget -= 1;
        *slot = if images {
            texture(ctx, format!("pdf-image-thumb-{i}"), pdf_import::image_thumbnail(bytes, i + 1, side, password.as_deref()))
        } else {
            texture(ctx, format!("pdf-page-thumb-{i}"), pdf_import::thumbnail(bytes, i + 1, side, password.as_deref()))
        };
    }
    if list.iter().any(Slot::is_pending) {
        ctx.request_repaint();
    }
}

/// The dialog's one row: Photoshop's two columns.
fn body(ui: &mut egui::Ui, f: &mut DialogFields, held: &Mutex<Held>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut st = lock(held);
    let images = s(f, "select", "pages") == "images";
    let size = s(f, "thumbnailSize", "small").to_string();
    render_thumbs(&mut st, ui.ctx(), images, &size);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(PANEL.x);
            left_column(ui, f, &mut st, &t, images, &size);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(WIDTH - PANEL.x - 24.0);
            right_column(ui, f, &mut st, &t, images);
        });
    })
    .response
}

/// A select over `(id, label)` options; the id picked this pass.
fn choose<T: Clone + PartialEq + Send + Sync + std::fmt::Debug + 'static>(
    ui: &mut egui::Ui,
    label: &str,
    salt: &'static str,
    current: T,
    options: &[(T, &str)],
) -> Option<T> {
    let entries: Vec<Entry<T>> = options.iter().map(|(id, name)| Entry::new(id.clone(), tl!(*name).to_string())).collect();
    Select::new(tl!(label).to_string(), Bind::given(current), &entries).id_salt(salt).show(ui).value().cloned()
}

/// A check box; its new state when it was toggled this pass.
fn check(ui: &mut egui::Ui, label: &str, on: bool) -> Option<bool> {
    let mut v = on;
    Checkbox::new(Bind::bound(&mut v)).label(tl!(label).to_string()).show(ui);
    (v != on).then_some(v)
}

fn left_column(ui: &mut egui::Ui, f: &mut DialogFields, st: &mut Held, t: &Tokens, images: bool, size: &str) {
    ui.label(egui::RichText::new(tl!("Select:")).color(t.text_dim));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        if radio(ui, !images, "Pages").clicked() {
            f.set("select", FieldValue::Text("pages".into()));
        }
        if radio(ui, images, "Images").clicked() {
            f.set("select", FieldValue::Text("images".into()));
        }
    });
    ui.add_space(4.0);
    // The thumbnail panel.
    let (panel, _) = ui.allocate_exact_size(PANEL, Sense::hover());
    ui.painter().rect_filled(panel, t.radius_sm, t.field);
    ui.painter().rect_stroke(panel, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    if let Some(e) = st.images_error.as_deref().filter(|_| images) {
        let c = panel.center();
        ui.painter().text(c - vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, tl!("The PDF couldn't be read."), egui::FontId::proportional(12.0), t.danger);
        ui.painter().text(c + vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(11.0), t.text_dim);
    }
    let key = if images { "images" } else { "pages" };
    let count = if images { st.images.as_ref().map_or(0, Vec::len) } else { st.info.as_ref().map_or(0, |i| i.pages.len()) };
    let mut selected = picks(f, key);
    let side = thumb_side(size) as f32;
    let cell = vec2(side + 16.0, side + 30.0);
    let per_row = ((panel.width() - 8.0) / cell.x).floor().max(1.0) as usize;
    let rows = count.div_ceil(per_row.max(1));
    let inner = panel.shrink(4.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(inner);
    egui::ScrollArea::vertical().id_salt(("pdf-thumbs", key)).max_height(inner.height()).auto_shrink([false, false]).show(&mut child, |ui| {
        let mods = ui.input(|i| i.modifiers);
        let mut clicked = None;
        let (grid, _) = ui.allocate_exact_size(vec2(inner.width(), (rows as f32 * cell.y).max(1.0)), Sense::hover());
        for i in 0..count {
            let (row, col) = (i / per_row.max(1), i % per_row.max(1));
            let r = Rect::from_min_size(grid.min + vec2(col as f32 * cell.x, row as f32 * cell.y), cell);
            if !ui.is_rect_visible(r) {
                continue;
            }
            let thumb = if images { st.image_thumbs.get(i) } else { st.thumbs.get(i) }.and_then(Slot::ready);
            let on = selected.contains(&(i + 1));
            let paper = Rect::from_center_size(pos2(r.center().x, r.top() + 8.0 + side / 2.0), vec2(side, side));
            let (pw, ph) = thumb.map_or((side, side), |th| {
                let k = (side / th.size.0.max(1) as f32).min(side / th.size.1.max(1) as f32);
                (th.size.0 as f32 * k, th.size.1 as f32 * k)
            });
            let img = Rect::from_center_size(paper.center(), vec2(pw, ph));
            ui.painter().rect_filled(img, 0.0, Color32::WHITE);
            if let Some(th) = thumb {
                ui.painter().image(th.tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if on {
                ui.painter().rect_stroke(img.expand(2.0), 0.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
            }
            let label = (i + 1).to_string();
            ui.painter().text(pos2(r.center().x, r.bottom() - 11.0), egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(11.0), t.text);
            if on {
                let y = r.bottom() - 3.0;
                ui.painter().line_segment([pos2(img.left(), y), pos2(img.right(), y)], Stroke::new(2.0, t.accent));
            }
            let resp = ui.interact(r, ui.scope_id().with(("pdf-thumb", key, i)), Sense::click());
            if resp.clicked() {
                clicked = Some(i);
            }
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, format!("{} {label}", if images { "Image" } else { "Page" }))
            });
        }
        if let Some(i) = clicked {
            let n = i + 1;
            // A ⇧-click range must not start from an index in the other list.
            let anchor = st.anchor.filter(|(list, _)| *list == images).map(|(_, a)| a);
            if mods.shift
                && let Some(a) = anchor
            {
                let (lo, hi) = (a.min(i), a.max(i));
                for k in lo..=hi {
                    if !selected.contains(&(k + 1)) {
                        selected.push(k + 1);
                    }
                }
            } else if mods.command {
                if let Some(p) = selected.iter().position(|v| *v == n) {
                    selected.remove(p);
                } else {
                    selected.push(n);
                }
                st.anchor = Some((images, i));
            } else {
                selected = vec![n];
                st.anchor = Some((images, i));
            }
            selected.sort_unstable();
            set_picks(f, key, &selected);
            f.remove("pixelSize");
        }
    });
    ui.add_space(6.0);
    let status = if images && count == 0 {
        tl!("Select one or more images").to_string()
    } else if images {
        crate::i18n::fmt(tl!("{n} of {total} image(s) selected"), &[("n", &selected.len().to_string()), ("total", &count.to_string())])
    } else {
        crate::i18n::fmt(tl!("{n} of {total} page(s) selected"), &[("n", &selected.len().to_string()), ("total", &count.to_string())])
    };
    ui.label(egui::RichText::new(status).color(t.text_dim));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Thumbnail Size:")).color(t.text_dim));
        let sizes: Vec<(String, &str)> = THUMB_SIZES.iter().map(|(v, l)| (v.to_string(), *l)).collect();
        if let Some(v) = choose(ui, "Thumbnail Size", "pdf-thumb-size", size.to_string(), &sizes) {
            f.set("thumbnailSize", FieldValue::Text(v));
        }
    });
}

fn right_column(ui: &mut egui::Ui, f: &mut DialogFields, st: &mut Held, t: &Tokens, images: bool) {
    let label_w = 76.0;
    let field = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        // Exactly one line high: inside this custom row the framework's fields claim the height
        // left in the dialog, so a plain horizontal row grew to it and pushed every row after it
        // out of sight. The row is drawn in a child over a fixed rectangle instead.
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
        ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, tl!(label), egui::FontId::proportional(12.0), t.text_dim);
        add(&mut ui);
    };
    ui.add_enabled_ui(!images, |ui| {
        group(ui, t, tl!("Page Options"), |ui| {
            field(ui, "Name:", &mut |ui| {
                let mut name = s(f, "name", "").to_string();
                let before = name.clone();
                TextField::new(tl!("Name:"), Bind::bound(&mut name)).id_salt("pdf-name").show(ui);
                if name != before {
                    f.set("name", FieldValue::Text(name));
                }
            });
            field(ui, "Crop To:", &mut |ui| {
                let cur = s(f, "crop", "boundingBox").to_string();
                let opts: Vec<(String, &str)> = CropTo::ALL.iter().map(|c| (c.id().to_string(), c.label())).collect();
                if let Some(v) = choose(ui, "Crop To", "pdf-crop", cur, &opts) {
                    f.set("crop", FieldValue::Text(v));
                    f.remove("pixelSize");
                }
            });
            // What the choice keeps, so it can be picked without trying each one (#11414).
            let crop = CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default();
            ui.horizontal(|ui| {
                // Allocated like `field`'s label, so the spacing after it lines the hint up with the dropdown.
                ui.allocate_exact_size(vec2(label_w, 0.0), Sense::hover());
                ui.add(egui::Label::new(egui::RichText::new(tl!(crop.description())).size(11.0).color(t.text_dim)).wrap());
            });
            ui.horizontal(|ui| {
                ui.allocate_exact_size(vec2(label_w, 0.0), Sense::hover());
                if let Some(v) = check(ui, "Anti-aliased", flag(f, "antiAlias", true)) {
                    f.set("antiAlias", FieldValue::Bool(v));
                }
            });
        });
        ui.add_space(6.0);
        let (w, h) = pixel_size(st, f).unwrap_or((0, 0));
        let mode = s(f, "mode", "rgb").to_string();
        let depth = f.int("depth").unwrap_or(8);
        let title =
            if w == 0 { tl!("Image Size: --").to_string() } else { crate::i18n::fmt(tl!("Image Size: {size}"), &[("size", &size_label(w, h, &mode, depth))]) };
        group(ui, t, &title, |ui| {
            let unit = s(f, "unit", "px").to_string();
            let ppi = resolution(f);
            let constrain = flag(f, "constrain", false);
            let units: Vec<(String, &str)> = UNITS.iter().map(|(v, l, _)| (v.to_string(), *l)).collect();
            let decimals = if unit == "px" { 0 } else { 2 };
            for (key, label, salt, cur, other) in [("width", "Width:", "pdf-unit-width", w, h), ("height", "Height:", "pdf-unit-height", h, w)] {
                field(ui, label, &mut |ui| {
                    let shown = Some(f64::from(to_unit(cur as f32, &unit, ppi)));
                    let mut v = shown;
                    NumberField::new(tl!(label), Bind::bound(&mut v)).range(0.0..=MAX_SIDE).commit_decimals(decimals).id_salt(("pdf-size", key)).show(ui);
                    if v != shown
                        && let Some(v) = v
                    {
                        let px = f64::from(from_unit(v as f32, &unit, ppi)).round().clamp(1.0, MAX_SIDE) as u32;
                        let (nw, nh) = if constrain && cur > 0 {
                            let o = (f64::from(px) * f64::from(other) / f64::from(cur)).round().clamp(1.0, MAX_SIDE) as u32;
                            if key == "width" { (px, o) } else { (o, px) }
                        } else if key == "width" {
                            (px, other.max(1))
                        } else {
                            (other.max(1), px)
                        };
                        set_pixel_size(f, nw, nh);
                    }
                    ui.add_space(6.0);
                    if let Some(u) = choose(ui, "Units", salt, unit.clone(), &units) {
                        f.set("unit", FieldValue::Text(u));
                    }
                });
            }
            if let Some(c) = check(ui, "Constrain Proportions", constrain) {
                f.set("constrain", FieldValue::Bool(c));
            }
            field(ui, "Resolution:", &mut |ui| {
                let res_unit = s(f, "resolutionUnit", "in").to_string();
                let shown = Some(f64::from(if res_unit == "cm" { ppi / 2.54 } else { ppi }));
                let mut v = shown;
                NumberField::new(tl!("Resolution:"), Bind::bound(&mut v))
                    .range(0.1..=f64::from(pdf_import::MAX_RESOLUTION))
                    .commit_decimals(if res_unit == "cm" { 2 } else { 0 })
                    .id_salt("pdf-resolution")
                    .show(ui);
                if v != shown
                    && let Some(v) = v
                {
                    let v = v as f32;
                    let new = (if res_unit == "cm" { v * 2.54 } else { v }).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
                    // A typed size is a physical size: it keeps its inches, so its pixels follow.
                    if let Some((pw, ph)) = pixel_size_field(f) {
                        let k = f64::from(new) / f64::from(ppi.max(0.001));
                        let scale = |p: u32| (f64::from(p) * k).round().clamp(1.0, MAX_SIDE) as u32;
                        set_pixel_size(f, scale(pw), scale(ph));
                    }
                    f.set("resolution", FieldValue::Number(f64::from(new)));
                }
                ui.add_space(6.0);
                let opts: Vec<(String, &str)> = RES_UNITS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                if let Some(ru) = choose(ui, "Resolution Units", "pdf-res-unit", res_unit.clone(), &opts) {
                    f.set("resolutionUnit", FieldValue::Text(ru));
                }
            });
            field(ui, "Mode:", &mut |ui| {
                let opts: Vec<(String, &str)> = MODES.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                if let Some(m) = choose(ui, "Mode", "pdf-mode", mode.clone(), &opts) {
                    f.set("mode", FieldValue::Text(m));
                }
            });
            field(ui, "Bit Depth:", &mut |ui| {
                if let Some(d) = choose(ui, "Bit Depth", "pdf-depth", depth, DEPTHS) {
                    f.set("depth", FieldValue::Int(d));
                }
            });
        });
    });
    ui.add_space(8.0);
    if let Some(v) = check(ui, "Suppress Warnings", flag(f, "suppressWarnings", false)) {
        f.set("suppressWarnings", FieldValue::Bool(v));
    }
    ui.add_enabled_ui(!images, |ui| {
        if let Some(v) = check(ui, "Open In Reverse Order", flag(f, "reverseOrder", true)) {
            f.set("reverseOrder", FieldValue::Bool(v));
        }
    });
}

/// A titled box, as Photoshop groups the page options and the image size.
fn group(ui: &mut egui::Ui, t: &Tokens, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).color(t.text_dim).size(12.0));
    egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        content(ui);
    });
}

fn radio(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        ui.painter().circle_stroke(r.center(), 5.5, Stroke::new(1.2, if on { t.accent } else { t.text_faint }));
        if on {
            ui.painter().circle_filled(r.center(), 3.0, t.accent);
        }
        let l = ui
            .add(egui::Label::new(egui::RichText::new(tl!(label)).color(t.text_dim)).selectable(false).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.union(l)
    })
    .inner
}

/// Judges Import PDF's OK: it opens something only with a page (or image) selected.
pub(crate) fn check_import(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let images = s(&result.fields, "select", "pages") == "images";
    if picks(&result.fields, if images { "images" } else { "pages" }).is_empty() {
        return Err(Label::from(if images { tl!("Select one or more images.") } else { tl!("Select one or more pages.") }));
    }
    Ok(())
}

/// OK: opens every selected page or image, each through the ordinary open path.
pub(crate) fn run_import(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let Some(held) = held_of(app, &result.key) else { return };
    if let Some(n) = number_of(&result.key) {
        app.pdf_import.retain(|p| p.number != n);
    }
    let f = &result.fields;
    let (file_name, bytes, path, password, slot) = {
        let st = lock(&held);
        (st.file_name.clone(), Arc::clone(&st.bytes), st.path.clone(), st.password.clone(), st.slot)
    };
    let images = s(f, "select", "pages") == "images";
    let mut selected = picks(f, if images { "images" } else { "pages" });
    if !images && flag(f, "reverseOrder", true) {
        selected.reverse();
    }
    let name = Some(s(f, "name", "").trim()).filter(|n| !n.is_empty()).map_or(file_name.clone(), str::to_string);
    let mut failed = Vec::new();
    // One set: the tabs keep the chosen order (Open In Reverse Order included) however the
    // background opens finish, starting where the PDF was dropped.
    let batch = crate::jobs_ui::begin_batch_at(app, slot);
    for (seq, n) in selected.into_iter().enumerate() {
        let order = Some(crate::jobs_ui::OpenOrder { batch, seq });
        if let Err(e) = app.open_pdf_choice(&name, &bytes, path.as_deref(), options(f, n, images, password.as_deref()), order) {
            failed.push(e);
        }
    }
    crate::jobs_ui::end_batch(app);
    if let Some(e) = failed.first() {
        app.open_failed(&file_name, e);
    }
}

#[cfg(test)]
mod tests;
