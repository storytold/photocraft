//! Save for Web on the framework's `HostedSaveForWeb`: the dialog keeps the settings and asks for
//! pictures; this file encodes them (`web_cmds`), answers the colour and path requests, and runs
//! the engine's `file.export.saveForWebLegacy` at OK.

use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::ui::dialog::{DialogId, DialogResult, StackDialog};
use craft::ui::patterns::form_dialogs::{
    Animation, ColorTable, ColorTableEdit, Dither, HostedSaveForWeb, Resample, SaveForWebOptions, SaveForWebResult, TableMarks, WantedVariant, WebFormat,
    WebMetadata, WebPreset, WebSlice, WebSlices, WebVariant,
};
use craft::ui::runtime::env::Label;
use egui::load::SizedTexture;
use photocraft_engine::web_cmds::{self, WebSettings};
use serde_json::{Value, json};

use super::{PhotocraftApp, web_preview};

/// The engine's name of each format; the one table `options_of` and `params_of` share.
const WEB_FORMATS: [(WebFormat, &str); 6] = [
    (WebFormat::Gif, "gif"),
    (WebFormat::Png8, "png8"),
    (WebFormat::Png24, "png24"),
    (WebFormat::Jpeg, "jpeg"),
    (WebFormat::WebP, "webp"),
    (WebFormat::Wbmp, "wbmp"),
];
const PALETTES: [(ColorTable, &str); 5] = [
    (ColorTable::Perceptual, "perceptual"),
    (ColorTable::Selective, "selective"),
    (ColorTable::Adaptive, "adaptive"),
    (ColorTable::Restrictive, "restrictive"),
    (ColorTable::Exact, "exact"),
];
const DITHERS: [(Dither, &str); 4] = [(Dither::None, "none"), (Dither::Diffusion, "diffusion"), (Dither::Pattern, "pattern"), (Dither::Noise, "noise")];
const METADATA: [(WebMetadata, &str); 4] =
    [(WebMetadata::None, "none"), (WebMetadata::Copyright, "copyright"), (WebMetadata::CopyrightAndContact, "copyrightAndContact"), (WebMetadata::All, "all")];
/// The five methods Photoshop's Save for Web lists; the dialog offers no others.
const RESAMPLES: [(Resample, &str); 5] = [
    (Resample::NearestNeighbor, "nearest"),
    (Resample::Bilinear, "bilinear"),
    (Resample::Bicubic, "bicubic"),
    (Resample::BicubicSmoother, "bicubicSmoother"),
    (Resample::BicubicSharper, "bicubicSharper"),
];

pub(super) const WEB_COMMAND: &str = "file.export.saveForWebLegacy";

fn name_of<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(v, _)| *v == value).map_or(table[0].1, |(_, n)| n)
}

fn by_name<T: Copy>(table: &[(T, &str)], name: &str) -> Option<T> {
    table.iter().find(|(_, n)| *n == name).map(|(v, _)| *v)
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

pub(super) fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The colour table of the open Save for Web dialog: each shifted colour with the colour it becomes. The dialog names
/// colours by value; the engine edits palette entries by index, so the shifts become `colorTableEdits` over the palette
/// each encode indexes with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebTable {
    /// The dialog the shifts belong to.
    dialog: Option<DialogId>,
    /// Each shifted colour and the colour it becomes, in the order they were shifted.
    shifted: Vec<([u8; 3], [u8; 3])>,
}

impl WebTable {
    /// Applies the edits of dialog `id`, each with the colours it applies to; a table of another dialog starts empty.
    /// Returns whether the shifts changed. Lock, Make Transparent, Delete and Sort have no engine counterpart and
    /// change nothing.
    pub(super) fn apply(&mut self, id: DialogId, edits: impl IntoIterator<Item = (ColorTableEdit, Vec<[u8; 3]>)>) -> bool {
        if self.dialog != Some(id) {
            *self = WebTable { dialog: Some(id), shifted: Vec::new() };
        }
        let before = self.shifted.clone();
        for (edit, colors) in edits {
            match edit {
                ColorTableEdit::Shift(to) => colors.iter().for_each(|from| self.shift(*from, to)),
                ColorTableEdit::WebShift => colors.iter().for_each(|from| self.shift(*from, web_safe(*from))),
                ColorTableEdit::Unshift => self.shifted.retain(|(from, _)| !colors.contains(from)),
                ColorTableEdit::UnshiftAll | ColorTableEdit::Restore => self.shifted.clear(),
                _ => {}
            }
        }
        self.shifted != before
    }

    fn shift(&mut self, from: [u8; 3], to: [u8; 3]) {
        self.shifted.retain(|(shifted, _)| *shifted != from);
        if from != to {
            self.shifted.push((from, to));
        }
    }

    /// The marks the dialog's table shows.
    fn marks(&self) -> TableMarks {
        let mut marks = TableMarks::default();
        marks.shifted = self.shifted.clone();
        marks
    }

    /// The shifts of dialog `id`; none for another dialog.
    pub(super) fn of(&self, id: DialogId) -> &[([u8; 3], [u8; 3])] {
        if self.dialog == Some(id) { &self.shifted } else { &[] }
    }
}

/// The nearest web-safe colour: each channel on the nearest multiple of 51.
fn web_safe(rgb: [u8; 3]) -> [u8; 3] {
    rgb.map(|v| ((f32::from(v) / 51.0).round() * 51.0) as u8)
}

/// The engine's `colorTableEdits` for `shifted` over `palette`: every entry holding a shifted colour, by index.
pub(super) fn shift_edits(palette: &[[u8; 3]], shifted: &[([u8; 3], [u8; 3])]) -> serde_json::Map<String, Value> {
    palette
        .iter()
        .enumerate()
        .filter_map(|(at, color)| shifted.iter().find(|(from, _)| from == color).map(|(_, to)| (at.to_string(), json!(hex(*to)))))
        .collect()
}

/// The dialog's options for engine parameters: remembered ones, a preset's, or the last export's. Keys the
/// parameters lack keep the dialog's defaults.
pub(super) fn options_of(p: &Value) -> SaveForWebOptions {
    let mut o = SaveForWebOptions::default();
    let text = |k: &str| p.get(k).and_then(Value::as_str);
    let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    if let Some(v) = text("format").and_then(|n| by_name(&WEB_FORMATS, n)) {
        o.format = v;
    }
    if let Some(v) = num("quality") {
        o.quality = v.round().clamp(0.0, 100.0) as u8;
    }
    if let Some(v) = text("palette").and_then(|n| by_name(&PALETTES, n)) {
        o.color_table = v;
    }
    if let Some(v) = num("colors") {
        o.colors = v.round().clamp(2.0, 256.0) as u16;
    }
    if let Some(v) = text("dither").and_then(|n| by_name(&DITHERS, n)) {
        o.dither = v;
    }
    if let Some(v) = num("ditherAmount") {
        o.dither_amount = v.round().clamp(0.0, 100.0) as u8;
    }
    if let Some(v) = num("webSnap") {
        o.web_snap = v.round().clamp(0.0, 100.0) as u8;
    }
    o.transparency = flag("transparency").unwrap_or(o.transparency);
    o.interlaced = flag("interlaced").unwrap_or(o.interlaced);
    o.progressive = flag("progressive").unwrap_or(o.progressive);
    o.optimized = flag("optimized").unwrap_or(o.optimized);
    o.lossless = flag("lossless").unwrap_or(o.lossless);
    o.embed_profile = flag("embedIcc").unwrap_or(o.embed_profile);
    o.convert_srgb = flag("convertToSrgb").unwrap_or(o.convert_srgb);
    o.html = flag("html").unwrap_or(o.html);
    if let Some(m) = text("matte").and_then(parse_hex) {
        o.matte = m;
    }
    if let Some(v) = text("metadata").and_then(|n| by_name(&METADATA, n)) {
        o.metadata = v;
    }
    if let Some(v) = num("percent").filter(|v| *v > 0.0) {
        o.percent = v;
    }
    if let Some(v) = text("resample").and_then(|n| by_name(&RESAMPLES, n)) {
        o.resample = v;
    }
    o.slices = match text("slices") {
        Some("user") => WebSlices::User,
        Some("selected") => {
            let numbers =
                p.get("numbers").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).filter_map(|n| u32::try_from(n).ok()).collect());
            WebSlices::Selected(numbers.unwrap_or_default())
        }
        _ => WebSlices::All,
    };
    o
}

/// The engine parameters of `o`. An exact size typed in the dialog (`o.size`, with Constrain Proportions on or off) is
/// sent as `width` and `height`, which replace `percent`. A document with slices adds the slice choice, its numbers and
/// `html`.
pub(super) fn params_of(o: &SaveForWebOptions, has_slices: bool) -> Value {
    let mut p = json!({
        "format": name_of(&WEB_FORMATS, o.format),
        "quality": o.quality,
        "lossless": o.lossless,
        "palette": name_of(&PALETTES, o.color_table),
        "colors": o.colors,
        "dither": name_of(&DITHERS, o.dither),
        "ditherAmount": o.dither_amount,
        "transparency": o.transparency,
        "matte": hex(o.matte),
        "interlaced": o.interlaced,
        "progressive": o.progressive,
        "optimized": o.optimized,
        "embedIcc": o.embed_profile,
        "convertToSrgb": o.convert_srgb,
        "metadata": name_of(&METADATA, o.metadata),
        "percent": o.percent,
        "resample": name_of(&RESAMPLES, o.resample),
        "webSnap": o.web_snap,
    });
    if let Some(size) = o.size {
        if let Some(map) = p.as_object_mut() {
            map.remove("percent");
        }
        p["width"] = json!(size.width);
        p["height"] = json!(size.height);
    }
    if has_slices {
        p["html"] = json!(o.html);
        match &o.slices {
            WebSlices::User => p["slices"] = json!("user"),
            WebSlices::Selected(numbers) => {
                let mut numbers = numbers.clone();
                numbers.sort_unstable();
                numbers.dedup();
                p["slices"] = json!("selected");
                p["numbers"] = json!(numbers);
            }
            _ => p["slices"] = json!("all"),
        }
    }
    p
}

/// The export parameters: `params_of`, refusing Selected Slices with nothing selected.
pub(super) fn web_params(o: &SaveForWebOptions, has_slices: bool) -> Result<Value, String> {
    if has_slices && matches!(&o.slices, WebSlices::Selected(numbers) if numbers.is_empty()) {
        return Err(tl!("No slices are selected. Click a slice in the preview to select it.").to_string());
    }
    Ok(params_of(o, has_slices))
}

/// The key of pane `index`'s indexed-encode cache and textures for document `doc_id`.
pub(super) fn pane_key(doc_id: u64, index: usize) -> egui::Id {
    egui::Id::unique(("web-pane", doc_id, index))
}

/// Keeps the picture alive for the context (a texture handle frees it on drop) and names it for the dialog.
fn texture(ctx: &egui::Context, key: egui::Id, image: egui::ColorImage, shown: egui::Vec2) -> SizedTexture {
    let handle = Arc::new(ctx.load_texture("web-pane", image, egui::TextureOptions::LINEAR));
    let sized = SizedTexture::new(handle.id(), shown);
    ctx.data_mut(|d| d.insert_temp(key, handle));
    sized
}

/// The Original pane's picture: the flattened document, reduced to at most 2048 px on its long side.
fn original_texture(ctx: &egui::Context, doc: &photocraft_doc::Document) -> SizedTexture {
    let (w, h) = (f64::from(doc.size.width).max(1.0), f64::from(doc.size.height).max(1.0));
    let k = (2048.0 / w.max(h)).min(1.0);
    let params = json!({});
    let resample = web_preview::proxy_resample(&params, k);
    let size = json!({"width": (w * k).round().max(1.0), "height": (h * k).round().max(1.0), "resample": resample});
    let settings = WebSettings::default();
    let image = web_cmds::web_document(doc, &size, &settings)
        .map(|(proxy, _, _)| {
            let (pw, ph) = (proxy.size.width as usize, proxy.size.height as usize);
            let rgba: Vec<u8> =
                photocraft_compose::flatten(&proxy).into_px().iter().flat_map(|q| q.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
            egui::ColorImage::from_rgba_unmultiplied([pw, ph], &rgba)
        })
        .unwrap_or_else(|_| egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0; 4]));
    texture(ctx, egui::Id::unique(("web-original", doc.id.0)), image, egui::vec2(w as f32, h as f32))
}

/// Opens Save for Web for the active document, seeded from the engine's last export (the one copy of what is
/// remembered: it writes `Preferences.dialogs` on every run, scripted ones included). Each document starts at its
/// own full size.
///
/// `None` without a document.
pub fn open_web(app: &mut PhotocraftApp) -> Option<DialogId> {
    let ctx = app.ctx().clone();
    let doc = app.session.active()?.doc.clone();
    let mut seed = app.session.prefs().dialogs.get(WEB_COMMAND).cloned().unwrap_or_else(|| json!({}));
    if let Some(m) = seed.as_object_mut() {
        for key in ["path", "dir", "folder", "width", "height", "percent", "colorTable", "colorTableEdits", "numbers"] {
            m.remove(key);
        }
        if m.get("slices").and_then(Value::as_str) == Some("selected") {
            m.remove("slices");
        }
    }
    let presets = web_cmds::PRESETS
        .iter()
        .filter_map(|name| web_cmds::preset_settings(name).map(|st| WebPreset::new(name.to_string(), options_of(&st.to_params()))))
        .collect();
    let slices = if doc.slices.is_empty() {
        Vec::new()
    } else {
        photocraft_doc::slices::resolve(&doc)
            .iter()
            .map(|r| {
                WebSlice::new(
                    r.number as u32,
                    egui::Rect::from_min_max(egui::pos2(r.rect.x0 as f32, r.rect.y0 as f32), egui::pos2(r.rect.x1 as f32, r.rect.y1 as f32)),
                )
            })
            .collect()
    };
    let mut dialog = HostedSaveForWeb::new(doc.name.clone(), PixelSize::new(doc.size.width, doc.size.height), original_texture(&ctx, &doc))
        .options(options_of(&seed))
        .presets(presets)
        .slices(slices)
        .slice_warning(slice_warning(&doc));
    if let Some(t) = doc.timeline.as_ref().filter(|t| t.duration >= 2) {
        dialog = dialog.animation(Animation::new(t.duration as u32, t.fps));
    }
    let id = app.framework.dialogs.open(dialog);
    app.ui.web_table = WebTable { dialog: Some(id), shifted: Vec::new() };
    Some(id)
}

/// The notice under the preview captions when the document has more slices than are resolved (`overflow_warning`):
/// how many were ignored past the first [`photocraft_doc::slices::MAX_RESOLVED_SLICES`]. `None` otherwise.
pub(super) fn slice_warning(doc: &photocraft_doc::Document) -> Option<Label> {
    photocraft_doc::slices::overflow_warning(doc)?;
    let max = photocraft_doc::slices::MAX_RESOLVED_SLICES;
    let ignored = doc.slices.list.len().saturating_sub(max).to_string();
    let text =
        crate::i18n::fmt(tl!("{count} stored slices past the first {max} were ignored"), &[("count", ignored.as_str()), ("max", max.to_string().as_str())]);
    Some(Label::from(text))
}

/// Seek a preview copy, preserving any edits to the source's currently displayed frame.
pub(super) fn web_frame_document(doc: &photocraft_doc::Document, frame: Option<usize>) -> Result<photocraft_doc::Document, photocraft_engine::EngineError> {
    let Some(frame) = frame else { return Ok(doc.clone()) };
    let mut preview = photocraft_engine::Session::new();
    preview.add_document(doc.clone(), None);
    preview.execute("timeline.setFrame", json!({"frame": frame}))?;
    Ok((*preview.active().ok_or(photocraft_engine::EngineError::NoDocument)?.doc).clone())
}

/// The picture of one wanted pane: the options' encode of the document's `frame`, reduced to a proxy of at most
/// 2048 px, with the size and palette the dialog shows and the table's `shifted` colours.
fn encode(ctx: &egui::Context, app: &PhotocraftApp, wanted: &WantedVariant, frame: Option<usize>, shifted: &[([u8; 3], [u8; 3])]) -> WebVariant {
    let (index, revision) = (wanted.index, wanted.request.revision);
    let fail = |message: String| WebVariant::failed(index, revision, message);
    let Some(st) = app.session.active() else { return fail("no document".into()) };
    let (doc, rev) = (st.doc.clone(), st.revision);
    let p = params_of(&wanted.request.options, false);
    let (fw, fh) = match web_preview::dimensions(&doc, &p) {
        Ok(size) => size,
        Err(error) => return fail(error),
    };
    let k = (2048.0 / fw.max(fh)).min(1.0);
    let signature = format!("{}|{rev}|{frame:?}", doc.id.0);
    let source = |settings: &WebSettings| {
        let doc = web_frame_document(&doc, frame).map_err(|e| e.to_string())?;
        let size = json!({"width": (fw * k).round().max(1.0), "height": (fh * k).round().max(1.0), "resample": web_preview::proxy_resample(&p, k)});
        let (proxy, _, _) = web_cmds::web_document(&doc, &size, settings).map_err(|e| e.to_string())?;
        let (w, h) = (proxy.size.width, proxy.size.height);
        Ok((photocraft_compose::flatten(&proxy).into_px(), w, h))
    };
    let encoded = match web_preview::optimized(ctx, pane_key(doc.id.0, index), &signature, &p, shifted, source) {
        Ok(encoded) => encoded,
        Err(message) => return fail(message),
    };
    let (w, h) = (encoded.width.max(1), encoded.height.max(1));
    let ratio = (fw * fh) / (f64::from(w) * f64::from(h)).max(1.0);
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &encoded.rgba);
    let picture = texture(ctx, egui::Id::unique(("web-texture", doc.id.0, index)), image, egui::vec2(fw as f32, fh as f32));
    let mut variant = WebVariant::new(index, revision, picture, (encoded.bytes as f64 * ratio) as u64).proxy(ratio > 1.001);
    if let Some(colors) = encoded.colors {
        variant = variant.colors(colors as u16);
    }
    if wanted.request.options.format.indexed() {
        variant = variant.palette(encoded.palette);
    }
    variant
}

fn has_slices(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.doc.slices.is_empty())
}

fn report(app: &mut PhotocraftApp, error: String) {
    app.ui.status = error;
    app.ui.status_error = true;
}

/// The extension the engine writes `options`' format with.
fn engine_ext(options: &SaveForWebOptions) -> String {
    WebSettings::from_params(&params_of(options, false), WEB_COMMAND).map_or_else(|_| options.format.extension().to_string(), |st| st.format.ext().to_string())
}

/// Answers what the open Save for Web dialog asked for since the last call: the panes to encode, the colour
/// pick (the matte and the colour-table cells), the save path, and OK. Each request is taken once, so a second
/// pass in the same frame finds nothing left.
pub(super) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(result) = app.framework.dialogs.take::<HostedSaveForWeb>() {
        if let DialogResult::Ok(result) = result {
            finish(app, ctx, result);
        }
        return;
    }
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedSaveForWeb>() else { return };
    let id = dialog.id();
    let events = dialog.take_table_events();
    if app.ui.web_table.apply(id, events.into_iter().map(|event| (event.edit, event.colors))) {
        let marks = app.ui.web_table.marks();
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id) {
            dialog.set_marks(marks);
        }
    }
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id) else { return };
    let wanted = dialog.wanted_variants();
    let (color, path, frame) = (dialog.take_color_request(), dialog.pick_path(), dialog.frame());
    let current = dialog.current().clone();
    if let Some(request) = color {
        crate::color_picker_ui::open_for_request(app, id, request);
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id)
    {
        dialog.answer_color(&key, color);
    }
    if let Some(request) = path {
        pick_save(app, id, request.key, &current);
    }
    if wanted.is_empty() {
        return;
    }
    let animated = app.session.active().is_some_and(|st| st.doc.timeline.as_ref().is_some_and(|t| t.duration >= 2));
    let shifted = app.ui.web_table.of(id);
    let variants: Vec<WebVariant> = wanted.iter().map(|w| encode(ctx, app, w, animated.then_some(frame as usize), shifted)).collect();
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id) {
        for variant in variants {
            dialog.set_variant(variant);
        }
    }
}

/// Save… was pressed: the export policy judges the parameters (with the generated palette) before any picker
/// opens, then the save picker answers the dialog's path request `key`.
fn pick_save(app: &mut PhotocraftApp, id: DialogId, key: String, current: &SaveForWebOptions) {
    let slices = has_slices(app);
    let shifted = app.ui.web_table.of(id).to_vec();
    let checked = web_params(current, slices).and_then(|p| checked_params(app, &p, &shifted));
    let refuse = |app: &mut PhotocraftApp, error: String| {
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id) {
            dialog.cancel_requests();
        }
        report(app, error);
    };
    if let Err(error) = checked {
        return refuse(app, error);
    }
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return refuse(app, "no document".into()) };
    let base = photocraft_doc::slices::base_name(&doc);
    let suggested = if slices && current.html { format!("{base}.html") } else { format!("{base}.{}", engine_ext(current)) };
    let doc_id = doc.id;
    // PLAN: `pick_export` has no cancel hook, so a cancelled picker leaves Save… waiting until `cancel_requests`; the owner of file_dialog.rs adds the hook.
    let asked = app.pick_export(&suggested, move |app, path| {
        app.refocus(doc_id)?;
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id) {
            dialog.answer_path(key, std::path::PathBuf::from(path));
        }
        Ok(Value::Null)
    });
    if let Err(error) = asked {
        refuse(app, error);
    }
}

/// OK: the export runs with the confirmed options and the picked path; the settings are remembered once it succeeded.
/// Done writes nothing and remembers the settings.
fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, result: SaveForWebResult) {
    let shifted = std::mem::take(&mut app.ui.web_table).shifted;
    let Some(picked) = result.path.as_deref() else { return result.remember(ctx) };
    let slices = has_slices(app);
    let outcome = web_params(&result.options, slices).and_then(|mut p| {
        let path = picked.to_string_lossy().into_owned();
        if slices {
            p["dir"] = json!(picked.parent().map(|d| d.to_string_lossy().into_owned()).filter(|d| !d.is_empty()).unwrap_or_else(|| ".".into()));
        } else {
            // The suggested name kept: only the folder, so a recorded action saves each document under its own
            // name, as in Photoshop (#5305). A document renamed since the picker opened no longer matches the
            // picked name, so the whole path is used.
            let base = app.session.active().map(|st| photocraft_doc::slices::base_name(&st.doc)).ok_or("no document")?;
            match kept_name_folder(&path, &base, &engine_ext(&result.options)) {
                Some(folder) => p["folder"] = json!(folder),
                None => p["path"] = json!(path),
            }
        }
        web_confirm(app, &p, &shifted)
    });
    match outcome {
        Ok(_) => result.remember(ctx),
        Err(error) => report(app, error),
    }
}

/// Whether the export policy lets `p` through. Checked while the control/input policy is still installed,
/// before an asynchronous picker can outlive the request.
fn authorize_web(app: &PhotocraftApp, p: &Value) -> Result<(), String> {
    if let Some(authorize) = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None }) {
        authorize(WEB_COMMAND, p).map_err(|e| e.to_string())?;
    }
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(WEB_COMMAND, p)?;
    }
    Ok(())
}

/// `p` with its generated palette, after the policy approved both the request and the palette-bearing parameters.
/// The table's `shifted` colours become `colorTableEdits` over that palette.
fn checked_params(app: &PhotocraftApp, p: &Value, shifted: &[([u8; 3], [u8; 3])]) -> Result<Value, String> {
    authorize_web(app, p)?;
    let mut prepared = p.clone();
    web_preview::prepare_table(app, &mut prepared)?;
    let palette: Option<Vec<[u8; 3]>> =
        prepared.get("colorTable").and_then(Value::as_array).map(|table| table.iter().filter_map(|c| c.as_str().and_then(parse_hex)).collect());
    if let Some(palette) = palette.filter(|_| prepared.get("colorTableEdits").is_none()) {
        let edits = shift_edits(&palette, shifted);
        if !edits.is_empty() {
            prepared["colorTableEdits"] = Value::Object(edits);
        }
    }
    if prepared != *p {
        authorize_web(app, &prepared)?;
    }
    Ok(prepared)
}

/// Runs Save for Web with the final parameters `p`, which say where to write.
pub(super) fn web_confirm(app: &mut PhotocraftApp, p: &Value, shifted: &[([u8; 3], [u8; 3])]) -> Result<Value, String> {
    let p = checked_params(app, p, shifted)?;
    save_for_web(app, p)
}

/// The folder of `path` when its file is the one Save for Web suggests and writes from a folder
/// alone: `base` (a name usable on Windows as it is) with the format's own extension `ext`.
fn kept_name_folder(path: &str, base: &str, ext: &str) -> Option<String> {
    let file = std::path::Path::new(path);
    let portable = photocraft_engine::file_cmds::portable_device_name(base.to_string()) == base;
    let kept = portable && file.file_stem().is_some_and(|s| s.to_string_lossy() == base) && file.extension().is_some_and(|e| e.to_string_lossy() == ext);
    kept.then(|| file.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Runs Save for Web with `p`, which says where to write.
fn save_for_web(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    #[cfg(target_arch = "wasm32")]
    let result = download_for_web(app, p);
    #[cfg(not(target_arch = "wasm32"))]
    let result = app.run(WEB_COMMAND, p);
    let r = result.inspect_err(|error| {
        app.ui.status = error.clone();
        app.ui.status_error = true;
    })?;
    app.sync_views();
    app.ui.status =
        crate::i18n::trn(crate::i18n::current(), r["files"].as_array().map_or(0, Vec::len) as u64, "Saved for Web: {n} file", "Saved for Web: {n} files");
    // Warnings (such as ignored slices) go in the status line: the dialog closes on success.
    if let Some(w) = r["warnings"].as_array().filter(|w| !w.is_empty()) {
        app.ui.status = format!("{} ({})", app.ui.status, w.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
    }
    app.ui.status_error = false;
    Ok(r)
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) fn download_for_web(app: &mut PhotocraftApp, mut p: Value) -> Result<Value, String> {
    // Browser downloads have no directory hierarchy. Keep the HTML's image and spacer URLs
    // beside the downloaded page, matching the names handed to the platform service.
    p["imagesFolder"] = json!("");
    let write = app.services.write.as_mut().ok_or("no download service")?;
    web_cmds::save_for_web_with_writer(&mut app.session, &p, &mut |path, bytes| write(path, bytes).map_err(photocraft_engine::EngineError::Other))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn event(edit: ColorTableEdit, colors: &[[u8; 3]]) -> (ColorTableEdit, Vec<[u8; 3]>) {
        (edit, colors.to_vec())
    }

    /// The table's shifts follow the dialog's edits, and an export edits every palette entry of a shifted colour.
    #[test]
    fn table_shifts_become_palette_edits() {
        let id = egui::Id::unique("save-for-web");
        let (red, blue, off) = ([255, 0, 0], [0, 0, 255], [10, 100, 160]);
        let mut table = WebTable::default();
        assert!(table.apply(id, vec![event(ColorTableEdit::Shift([0, 255, 0]), &[red]), event(ColorTableEdit::WebShift, &[off])]));
        assert_eq!(table.of(id), &[(red, [0, 255, 0]), (off, [0, 102, 153])]);
        assert!(!table.apply(id, vec![event(ColorTableEdit::Lock, &[blue])]), "Lock has no engine counterpart");
        assert!(table.of(egui::Id::unique("another")).is_empty(), "shifts belong to their dialog");

        let edits = shift_edits(&[red, blue, red], table.of(id));
        assert_eq!(Value::Object(edits), json!({"0": "#00ff00", "2": "#00ff00"}));

        assert!(table.apply(id, vec![event(ColorTableEdit::Unshift, &[red])]));
        assert_eq!(table.of(id), &[(off, [0, 102, 153])]);
        assert!(table.apply(id, vec![event(ColorTableEdit::Restore, &[])]));
        assert!(table.of(id).is_empty());
    }
}
