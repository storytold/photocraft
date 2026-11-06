//! File › Open on a PDF: Photoshop's Import PDF dialog, as it lays it out. Select Pages or Images
//! with thumbnails (Thumbnail Size Small, Large or Fit Page; click, ⇧-click a range, ⌘/Ctrl-click to
//! toggle), Page Options (Name, Crop To, Anti-aliased), Image Size (Width and Height in a unit,
//! Constrain Proportions, Resolution, Mode, Bit Depth), Suppress Warnings and Open In Reverse
//! Order. OK opens every selected page (or image) as its own document through the ordinary open
//! path, so the tabs, notices and Open Recent behave as for any file.
//!
//! The answers live in the dialog fields (`ui.dialog.set` drives them); the file's bytes and the
//! thumbnail textures live in [`PdfImport`] on the app. The import itself is
//! `photocraft_io::pdf_import`, which also holds what was measured on Photoshop.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::PdfImportMode;
use photocraft_io::pdf_import::{self, CropTo, PdfImage, PdfInfo, PdfOpenOptions};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::widgets;

/// The field that marks a dialog as this module's.
const FIELD: &str = "__pdfImport";
/// The dialog's id in its fields: the held file is looked up by it, so several PDFs opened
/// together (Open, a drop, the command line) each keep their own.
const DIALOG: &str = "__pdfDialog";
/// The dialog's width in points (Photoshop's is about 690 px).
pub const WIDTH: f32 = 700.0;
/// Thumbnail Size choices (field value, label).
pub const THUMB_SIZES: &[(&str, &str)] = &[("small", "Small"), ("large", "Large"), ("fitPage", "Fit Page")];
/// Mode choices (field value, label), in Photoshop's order.
pub const MODES: &[(&str, &str)] = &[("gray", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
/// Bit Depth choices.
pub const DEPTHS: &[(u64, &str)] = &[(8, "8 bit"), (16, "16 bit")];
/// PDF import choices (field value, label). Auto preserves authored layers when possible.
pub const LAYER_MODES: &[(&str, &str)] = &[("auto", "Auto"), ("layers", "Layers"), ("render", "Render")];
/// Resolution unit choices (field value, label).
pub const RES_UNITS: &[(&str, &str)] = &[("in", "Pixels/Inch"), ("cm", "Pixels/Centimeter")];
/// Longest side of a thumbnail, per Thumbnail Size (Fit Page follows the panel).
const SMALL: u32 = 64;
const LARGE: u32 = 128;
/// The thumbnail panel's size in points.
const PANEL: egui::Vec2 = vec2(296.0, 290.0);

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

/// The file behind the dialog: what the fields can't hold.
pub struct PdfImport {
    /// The dialog's id.
    pub dialog: u64,
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
    /// The item a ⇧-click range starts from.
    anchor: Option<usize>,
    /// The tab position the PDF was dropped at, where its pages go; `None`: the end.
    slot: Option<usize>,
}

/// Whether `bytes` should open through this dialog: a PDF, unless it is the one-image kind
/// Photoshop opens like a picture without asking.
pub fn wants_dialog(name: &str, bytes: &[u8]) -> bool {
    photocraft_io::is_pdf_input(name, bytes) && (pdf_import::needs_password(bytes) || !pdf_import::opens_as_image(bytes, None))
}

/// Whether `fields` are this dialog's.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(FIELD)
}

/// The id of the dialog `fields` belong to.
fn dialog_of(f: &Map<String, Value>) -> Option<u64> {
    f.get(DIALOG).and_then(Value::as_u64)
}

/// The held file of the dialog `fields` belong to, taken out of the app (put it back when the
/// dialog stays up).
fn take(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Option<PdfImport> {
    let id = dialog_of(f)?;
    let i = app.pdf_import.iter().position(|st| st.dialog == id)?;
    Some(app.pdf_import.remove(i))
}

/// Opens `fields` as a new dialog and ties `st` to it: the held file follows the dialog.
fn show(app: &mut PhotocraftApp, mut st: PdfImport, mut fields: Map<String, Value>) -> u64 {
    fields.insert(DIALOG.into(), json!(0));
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert(DIALOG.into(), json!(id));
    }
    st.dialog = id;
    app.pdf_import.push(st);
    id
}

/// Lets the file held for dialog `id` go (the dialog was cancelled or closed).
pub fn forget(app: &mut PhotocraftApp, id: u64) {
    app.pdf_import.retain(|st| st.dialog != id);
}

/// Opens the dialog for the PDF `bytes` read from `path` (when known) as `name`. An encrypted
/// file asks for its password first. Returns the dialog id.
pub fn open(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: &[u8]) -> Result<u64, String> {
    open_placed(app, name, path, bytes, None)
}

/// [`open`] for a PDF dropped on a tab slot: the pages it opens start there.
pub fn open_placed(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: &[u8], slot: Option<usize>) -> Result<u64, String> {
    let bytes = Arc::new(bytes.to_vec());
    let mut f = Map::new();
    f.insert(FIELD.into(), json!(true));
    f.insert("__label".into(), json!(tl!("Import PDF")));
    f.insert("name".into(), json!(name));
    let info = match pdf_import::info(&bytes, None) {
        Ok(i) => Some(i),
        Err(e) if e.to_string().contains(pdf_import::PASSWORD_REQUIRED) => None,
        Err(e) => return Err(e.to_string()),
    };
    if info.is_none() {
        f.insert("__needsPassword".into(), json!(true));
        f.insert("password".into(), json!(""));
    }
    defaults(&mut f, app);
    let pages = info.as_ref().map_or(0, |i| i.pages.len());
    let st = PdfImport {
        dialog: 0,
        file_name: name.to_string(),
        path: path.map(str::to_string),
        bytes,
        info,
        images: None,
        images_error: None,
        password: None,
        thumbs: (0..pages).map(|_| Slot::Pending).collect(),
        image_thumbs: Vec::new(),
        thumb_key: String::new(),
        bounds: (0..pages).map(|_| None).collect(),
        anchor: None,
        slot,
    };
    Ok(show(app, st, f))
}

/// Photoshop's defaults: the first page, Bounding Box, anti-aliased, 300 ppi, RGB, 8 bits, the
/// ruler unit for the size, proportions free, warnings shown, pages opened in reverse order. Auto
/// keeps authored PDF layers when the importer can preserve them.
fn defaults(f: &mut Map<String, Value>, app: &PhotocraftApp) {
    f.insert("select".into(), json!("pages"));
    f.insert("pages".into(), json!([1]));
    f.insert("images".into(), json!([]));
    f.insert("thumbnailSize".into(), json!("small"));
    f.insert("layers".into(), json!("auto"));
    f.insert("crop".into(), json!(CropTo::BoundingBox.id()));
    f.insert("antiAlias".into(), json!(true));
    // The ruler unit, when the size fields know it (percent falls back to pixels).
    let suffix = app.session.prefs().units_and_rulers.rulers.suffix().trim();
    let unit = crate::new_doc_ui::UNITS.iter().map(|u| u.0).find(|u| *u == suffix).unwrap_or("px");
    f.insert("unit".into(), json!(unit));
    f.insert("constrain".into(), json!(false));
    f.insert("resolution".into(), json!(pdf_import::DEFAULT_RESOLUTION));
    f.insert("resolutionUnit".into(), json!("in"));
    f.insert("mode".into(), json!("rgb"));
    f.insert("depth".into(), json!(8));
    f.insert("suppressWarnings".into(), json!(false));
    f.insert("reverseOrder".into(), json!(true));
}

fn s<'a>(f: &'a Map<String, Value>, k: &str, d: &'a str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn num(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn flag(f: &Map<String, Value>, k: &str, d: bool) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(d)
}
/// The selected 1-based numbers under `k`, sorted and unique.
fn picks(f: &Map<String, Value>, k: &str) -> Vec<usize> {
    let mut v: Vec<usize> =
        f.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).filter(|n| *n >= 1).map(|n| n as usize).collect()).unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}

/// The options the fields describe for page or image `n`.
pub fn options(f: &Map<String, Value>, n: usize, images: bool, password: Option<&str>) -> PdfOpenOptions {
    let mode = match s(f, "mode", "rgb") {
        "gray" => photocraft_color::ColorMode::Grayscale,
        "cmyk" => photocraft_color::ColorMode::Cmyk,
        "lab" => photocraft_color::ColorMode::Lab,
        _ => photocraft_color::ColorMode::Rgb,
    };
    let depth = if f.get("depth").and_then(Value::as_u64) == Some(16) { photocraft_color::SampleType::U16 } else { photocraft_color::SampleType::U8 };
    let sized = pixel_size_field(f);
    PdfOpenOptions {
        layer_mode: match s(f, "layers", "auto") {
            "layers" => PdfImportMode::Layers,
            "render" => PdfImportMode::Render,
            _ => PdfImportMode::Auto,
        },
        page: if images { 1 } else { n },
        image: images.then_some(n),
        crop: CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default(),
        resolution: num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION),
        mode,
        depth,
        anti_alias: flag(f, "antiAlias", true),
        name: Some(s(f, "name", "").to_string()).filter(|n| !n.trim().is_empty()),
        pixel_size: sized.filter(|_| !images),
        password: password.map(str::to_string),
        suppress_warnings: flag(f, "suppressWarnings", false),
    }
}

/// The `pixelSize` field as `[width, height]`. A side past `u32` keeps `u32::MAX` rather than
/// wrapping to a small number, so the importer's page cap scales it down with its warning.
fn pixel_size_field(f: &Map<String, Value>) -> Option<(u32, u32)> {
    let side = |v: &Value| v.as_u64().map(|n| u32::try_from(n).unwrap_or(u32::MAX));
    match f.get("pixelSize").and_then(Value::as_array)?.as_slice() {
        [w, h] => Some((side(w)?, side(h)?)),
        _ => None,
    }
}

/// The page box the fields crop to for page `index` (0-based), in points as displayed.
fn page_size_pt(st: &mut PdfImport, f: &Map<String, Value>, index: usize) -> Option<(f32, f32)> {
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
fn pixel_size(st: &mut PdfImport, f: &Map<String, Value>) -> Option<(u32, u32)> {
    if let Some((w, h)) = pixel_size_field(f) {
        return Some((w.max(1), h.max(1)));
    }
    let first = picks(f, "pages").first().copied()?;
    let pt = page_size_pt(st, f, first.checked_sub(1)?)?;
    Some(pdf_import::pixel_size(pt, num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION)))
}

/// "20.8M" for a size in bytes, as Photoshop's Image Size line (1 M = 1024 × 1024).
pub fn size_label(w: u32, h: u32, mode: &str, depth: u64) -> String {
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

/// The thumbnail's longest side in pixels for a Thumbnail Size.
fn thumb_side(size: &str) -> u32 {
    match size {
        "large" => LARGE,
        "fitPage" => (PANEL.x - 40.0) as u32,
        _ => SMALL,
    }
}

/// Renders the missing thumbnails of the list on show, a few per frame.
fn render_thumbs(st: &mut PdfImport, ctx: &egui::Context, images: bool, size: &str) {
    if st.thumb_key != size {
        st.thumbs.iter_mut().for_each(|t| *t = Slot::Pending);
        st.image_thumbs.iter_mut().for_each(|t| *t = Slot::Pending);
        st.thumb_key = size.to_string();
    }
    let side = thumb_side(size);
    let mut budget = 3;
    if images {
        if st.images.is_none() {
            st.images = Some(match pdf_import::images(&st.bytes, st.password.as_deref()) {
                Ok(v) => v,
                Err(e) => {
                    st.images_error = Some(e.to_string());
                    Vec::new()
                }
            });
            st.image_thumbs = st.images.as_ref().map_or(Vec::new(), |v| v.iter().map(|_| Slot::Pending).collect());
        }
        for (i, slot) in st.image_thumbs.iter_mut().enumerate() {
            if !slot.is_pending() || budget == 0 {
                continue;
            }
            budget -= 1;
            *slot = match pdf_import::image_thumbnail(&st.bytes, i + 1, side, st.password.as_deref()) {
                Ok(t) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
                    Slot::Ready(Thumb { tex: ctx.load_texture(format!("pdf-image-thumb-{i}"), img, egui::TextureOptions::LINEAR), size: (t.width, t.height) })
                }
                Err(_) => Slot::Failed,
            };
        }
        if st.image_thumbs.iter().any(Slot::is_pending) {
            ctx.request_repaint();
        }
        return;
    }
    for (i, slot) in st.thumbs.iter_mut().enumerate() {
        if !slot.is_pending() || budget == 0 {
            continue;
        }
        budget -= 1;
        *slot = match pdf_import::thumbnail(&st.bytes, i + 1, side, st.password.as_deref()) {
            Ok(t) => {
                let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
                Slot::Ready(Thumb { tex: ctx.load_texture(format!("pdf-page-thumb-{i}"), img, egui::TextureOptions::LINEAR), size: (t.width, t.height) })
            }
            Err(_) => Slot::Failed,
        };
    }
    if st.thumbs.iter().any(Slot::is_pending) {
        ctx.request_repaint();
    }
}

/// The dialog body. Returns nothing: OK and Cancel are the frame's.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(mut st) = take(app, f) else {
        ui.label(egui::RichText::new(tl!("The PDF is no longer available.")).color(t.danger));
        return;
    };
    if flag(f, "__needsPassword", false) {
        password_body(ui, f, &t);
        app.pdf_import.push(st);
        return;
    }
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
    });
    app.pdf_import.push(st);
}

fn password_body(ui: &mut egui::Ui, f: &mut Map<String, Value>, t: &Tokens) {
    ui.label(egui::RichText::new(tl!("The PDF is protected by a password.")).color(t.text));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Password:")).color(t.text_dim));
        let mut pw = s(f, "password", "").to_string();
        let id = ui.scope_id().with("pdf-password");
        let r = widgets::password_field(ui, id, &mut pw, 220.0).accessible_name(tl!("Password:"));
        if r.changed() {
            f.insert("password".into(), json!(pw));
        }
    });
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(e)).color(t.danger));
    }
}

fn left_column(ui: &mut egui::Ui, f: &mut Map<String, Value>, st: &mut PdfImport, t: &Tokens, images: bool, size: &str) {
    ui.label(egui::RichText::new(tl!("Select:")).color(t.text_dim));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        // A ⇧-click range must not start from an index in the other list.
        if radio(ui, !images, "Pages").clicked() {
            if images {
                st.anchor = None;
            }
            f.insert("select".into(), json!("pages"));
        }
        if radio(ui, images, "Images").clicked() {
            if !images {
                st.anchor = None;
            }
            f.insert("select".into(), json!("images"));
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
            if mods.shift
                && let Some(a) = st.anchor
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
                st.anchor = Some(i);
            } else {
                selected = vec![n];
                st.anchor = Some(i);
            }
            selected.sort_unstable();
            f.insert(key.into(), json!(selected));
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
        let mut cur = size.to_string();
        if widgets::dropdown(ui, "pdf-thumb-size", "Thumbnail Size", &mut cur, &THUMB_SIZES.iter().map(|(v, l)| (v.to_string(), *l)).collect::<Vec<_>>(), 110.0)
        {
            f.insert("thumbnailSize".into(), json!(cur));
        }
    });
}

fn right_column(ui: &mut egui::Ui, f: &mut Map<String, Value>, st: &mut PdfImport, t: &Tokens, images: bool) {
    let label_w = 76.0;
    let field = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
            ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, tl!(label), egui::FontId::proportional(12.0), t.text_dim);
            add(ui);
        });
    };
    let preserve_layers = s(f, "layers", "auto") == "layers";
    ui.add_enabled_ui(!images, |ui| {
        group(ui, t, tl!("Page Options"), |ui| {
            field(ui, "Name:", &mut |ui| {
                let mut name = s(f, "name", "").to_string();
                let r = widgets::text_field(ui, ui.scope_id().with("pdf-name"), &mut name, "", 190.0, false).accessible_name(tl!("Name:"));
                if r.changed() {
                    f.insert("name".into(), json!(name));
                }
            });
            field(ui, "Layers", &mut |ui| {
                let mut import_as = s(f, "layers", "auto").to_string();
                let opts: Vec<(String, &str)> = LAYER_MODES.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                if widgets::dropdown(ui, "pdf-layers", "Layers", &mut import_as, &opts, 120.0) {
                    f.insert("layers".into(), json!(import_as));
                }
            });
            ui.add_enabled_ui(!preserve_layers, |ui| {
                field(ui, "Crop To:", &mut |ui| {
                    let mut cur = s(f, "crop", "boundingBox").to_string();
                    let opts: Vec<(String, &str)> = CropTo::ALL.iter().map(|c| (c.id().to_string(), c.label())).collect();
                    if widgets::dropdown(ui, "pdf-crop", "Crop To", &mut cur, &opts, 120.0) {
                        f.insert("crop".into(), json!(cur));
                        f.remove("pixelSize");
                    }
                });
                let mut aa = flag(f, "antiAlias", true);
                if widgets::checkbox(ui, &mut aa, tl!("Anti-aliased")).changed() {
                    f.insert("antiAlias".into(), json!(aa));
                }
            });
        });
        ui.add_space(6.0);
        ui.add_enabled_ui(!preserve_layers, |ui| {
            let (w, h) = pixel_size(st, f).unwrap_or((0, 0));
            let mode = s(f, "mode", "rgb").to_string();
            let depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
            let title = if w == 0 {
                tl!("Image Size: --").to_string()
            } else {
                crate::i18n::fmt(tl!("Image Size: {size}"), &[("size", &size_label(w, h, &mode, depth))])
            };
            group(ui, t, &title, |ui| {
                let unit = s(f, "unit", "px").to_string();
                let ppi = num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
                let constrain = flag(f, "constrain", false);
                let units: Vec<(String, &str)> = crate::new_doc_ui::UNITS.iter().map(|(v, l, _)| (v.to_string(), *l)).collect();
                let decimals = if unit == "px" { 0 } else { 2 };
                for (key, label, cur, other) in [("width", "Width:", w, h), ("height", "Height:", h, w)] {
                    field(ui, label, &mut |ui| {
                        let mut v = crate::new_doc_ui::to_unit(cur as f32, &unit, ppi);
                        let r = widgets::value_field_prec(ui, &mut v, 0.0..=300_000.0, "", 70.0, decimals).accessible_name(tl!(label));
                        if r.changed() && !r.dragged() {
                            let px = crate::new_doc_ui::from_unit(v, &unit, ppi).round().clamp(1.0, 300_000.0) as u32;
                            let (nw, nh) = if constrain && cur > 0 {
                                let o = (f64::from(px) * f64::from(other) / f64::from(cur)).round().clamp(1.0, 300_000.0) as u32;
                                if key == "width" { (px, o) } else { (o, px) }
                            } else if key == "width" {
                                (px, other.max(1))
                            } else {
                                (other.max(1), px)
                            };
                            f.insert("pixelSize".into(), json!([nw, nh]));
                        }
                        ui.add_space(6.0);
                        let mut u = unit.clone();
                        if widgets::dropdown(ui, &format!("pdf-unit-{key}"), "Units", &mut u, &units, 110.0) {
                            f.insert("unit".into(), json!(u));
                        }
                    });
                }
                let mut c = constrain;
                if widgets::checkbox(ui, &mut c, tl!("Constrain Proportions")).changed() {
                    f.insert("constrain".into(), json!(c));
                }
                field(ui, "Resolution:", &mut |ui| {
                    let res_unit = s(f, "resolutionUnit", "in").to_string();
                    let mut v = if res_unit == "cm" { ppi / 2.54 } else { ppi };
                    let r = widgets::value_field_prec(ui, &mut v, 0.1..=pdf_import::MAX_RESOLUTION, "", 70.0, if res_unit == "cm" { 2 } else { 0 })
                        .accessible_name(tl!("Resolution:"));
                    if r.changed() && !r.dragged() {
                        let new = (if res_unit == "cm" { v * 2.54 } else { v }).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
                        // A typed size is a physical size: it keeps its inches, so its pixels follow.
                        if let Some([pw, ph]) = f
                            .get("pixelSize")
                            .and_then(Value::as_array)
                            .map(|a| [a.first().and_then(Value::as_u64).unwrap_or(1), a.get(1).and_then(Value::as_u64).unwrap_or(1)])
                        {
                            let k = f64::from(new) / f64::from(ppi.max(0.001));
                            f.insert(
                                "pixelSize".into(),
                                json!([(pw as f64 * k).round().clamp(1.0, 300_000.0) as u64, (ph as f64 * k).round().clamp(1.0, 300_000.0) as u64]),
                            );
                        }
                        f.insert("resolution".into(), json!(new));
                    }
                    ui.add_space(6.0);
                    let mut ru = res_unit.clone();
                    let opts: Vec<(String, &str)> = RES_UNITS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                    if widgets::dropdown(ui, "pdf-res-unit", "Resolution Units", &mut ru, &opts, 130.0) {
                        f.insert("resolutionUnit".into(), json!(ru));
                    }
                });
                field(ui, "Mode:", &mut |ui| {
                    let mut m = mode.clone();
                    let opts: Vec<(String, &str)> = MODES.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                    if widgets::dropdown(ui, "pdf-mode", "Mode", &mut m, &opts, 130.0) {
                        f.insert("mode".into(), json!(m));
                    }
                });
                field(ui, "Bit Depth:", &mut |ui| {
                    let mut d = depth;
                    if widgets::dropdown(ui, "pdf-depth", "Bit Depth", &mut d, DEPTHS, 70.0) {
                        f.insert("depth".into(), json!(d));
                    }
                });
            });
        });
    });
    ui.add_space(8.0);
    let mut sw = flag(f, "suppressWarnings", false);
    if widgets::checkbox(ui, &mut sw, tl!("Suppress Warnings")).changed() {
        f.insert("suppressWarnings".into(), json!(sw));
    }
    ui.add_enabled_ui(!images, |ui| {
        let mut rev = flag(f, "reverseOrder", true);
        if widgets::checkbox(ui, &mut rev, tl!("Open In Reverse Order")).changed() {
            f.insert("reverseOrder".into(), json!(rev));
        }
    });
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(e)).color(t.danger));
    }
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

/// OK: with a password pending, try it (the dialog stays up with the pages on success, with the
/// error otherwise); else open every selected page or image, each through the ordinary open path.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let Some(mut st) = take(app, f) else { return Err("no PDF is waiting to open".into()) };
    if flag(f, "__needsPassword", false) {
        let pw = s(f, "password", "").to_string();
        let mut fields = f.clone();
        match pdf_import::info(&st.bytes, Some(&pw)) {
            Ok(info) => {
                let n = info.pages.len();
                st.info = Some(info);
                st.password = Some(pw);
                st.thumbs = (0..n).map(|_| Slot::Pending).collect();
                st.bounds = (0..n).map(|_| None).collect();
                fields.remove("__needsPassword");
                fields.remove("password");
                fields.remove("__error");
            }
            Err(e) => {
                fields.insert(
                    "__error".into(),
                    json!(if e.to_string().contains(pdf_import::WRONG_PASSWORD) { "That isn't the PDF's password." } else { "The PDF couldn't be read." }),
                );
            }
        }
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    let images = s(f, "select", "pages") == "images";
    let key = if images { "images" } else { "pages" };
    let mut selected = picks(f, key);
    if selected.is_empty() {
        let mut fields = f.clone();
        fields.insert("__error".into(), json!(if images { "Select one or more images." } else { "Select one or more pages." }));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    if !images && flag(f, "reverseOrder", true) {
        selected.reverse();
    }
    let name = Some(s(f, "name", "").trim()).filter(|n| !n.is_empty()).map_or(st.file_name.clone(), str::to_string);
    let mut opened = 0;
    let mut failed = Vec::new();
    // One set: the tabs keep the chosen order (Open In Reverse Order included) however the
    // background opens finish, starting where the PDF was dropped. Each job retains the same
    // source allocation, including while waiting for memory approval.
    let batch = crate::jobs_ui::begin_batch_at(app, st.slot);
    for (seq, n) in selected.into_iter().enumerate() {
        let order = Some(crate::jobs_ui::OpenOrder { batch, seq });
        match app.open_pdf_choice(&name, &st.bytes, st.path.as_deref(), options(f, n, images, st.password.as_deref()), order) {
            Ok(_) => opened += 1,
            Err(e) => failed.push(e),
        }
    }
    crate::jobs_ui::end_batch(app);
    if let Some(e) = failed.first() {
        app.open_failed(&st.file_name, e);
    }
    Ok(json!({"opened": opened, "failed": failed.len()}))
}

#[cfg(test)]
mod tests;
