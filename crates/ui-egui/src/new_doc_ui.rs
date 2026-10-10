//! File › New: searchable formats and an editable canvas preview. Values live in the dialog fields (`width`/`height` in pixels,
//! `resolution` in ppi, `mode`, `depth`, `background`, `name`), so `ui.dialog.set` drives it and
//! Create runs `file.new` with them.

use egui::{Align2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{icons, widgets};

pub use crate::new_doc_catalog::{CATEGORIES, Preset};

const DEPTH_OPTIONS: &[(u64, &str, &str)] = &[(8, "8 bit", "Integer"), (16, "16 bit", "Integer"), (32, "32 bit (float)", "Floating point")];

/// Width/Height units: (key, label, units per inch; 0 = pixels).
pub const UNITS: &[(&str, &str, f32)] =
    &[("px", "Pixels", 0.0), ("in", "Inches", 1.0), ("cm", "Centimeters", 2.54), ("mm", "Millimeters", 25.4), ("pt", "Points", 72.0), ("pica", "Picas", 6.0)];

/// Pixels to the display unit at `ppi`.
pub fn to_unit(px: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => px / ppi.max(1.0) * per_in,
        _ => px,
    }
}

/// Display unit back to pixels at `ppi`.
pub fn from_unit(v: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => (v / per_in * ppi.max(1.0)).round(),
        _ => v.round(),
    }
}

/// A typed size as the whole pixel count `file.new` takes (#254: a float like `512.0` isn't one, so
/// the command fell back to its 1920 x 1080 default). Clamped to the command's 1–300000 range.
pub fn px_value(px: f32) -> Value {
    json!(if px.is_finite() { px.round().clamp(1.0, 300_000.0) as u32 } else { 1 })
}

/// Name of the preset that takes the clipboard image's size.
pub const CLIPBOARD: &str = "Clipboard";

/// Offer the Clipboard preset (`w` × `h` px, at 72 ppi) first under the Recent presets, and
/// select it.
pub fn set_clipboard(f: &mut Map<String, Value>, w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    f.insert("__clipboard".into(), json!([w, h]));
    apply_preset(f, &(CLIPBOARD, w, h, 72.0));
}

/// The clipboard image's size, when the dialog offers the Clipboard preset.
fn clipboard_preset(f: &Map<String, Value>) -> Option<Preset> {
    let size = f.get("__clipboard")?.as_array()?;
    let dim = |i: usize| size.get(i)?.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    Some((CLIPBOARD, dim(0)?, dim(1)?, 72.0))
}

/// Set Width or Height (`key`) from a value typed in `unit`: whole pixels for `file.new`, plus the
/// typed value so the field keeps showing it (see [`shown_size`]).
pub fn set_size(f: &mut Map<String, Value>, key: &str, v: f32, unit: &str, ppi: f32) {
    f.insert(key.into(), px_value(from_unit(v, unit, ppi)));
    f.insert(typed_key(key), json!({"value": v, "unit": unit}));
    f.remove("__preset");
    f.remove("__savedPreset");
}

/// What Width or Height (`key`, `d` pixels when unset) shows in `unit`: the value the user typed
/// while it still gives the field's pixel count, else the pixels converted. Converting the rounded
/// pixels back on every keystroke turned a typed `5` mm into 14 px and the text into `4.9` (#1147).
/// A size over the 300000 px limit shows the limit, not the typed value the document won't have.
pub fn shown_size(f: &Map<String, Value>, key: &str, d: f32, unit: &str, ppi: f32) -> f32 {
    let px = get_f(f, key, d);
    let typed = f.get(&typed_key(key)).and_then(Value::as_object).filter(|t| unit != "px" && t.get("unit").and_then(Value::as_str) == Some(unit));
    match typed.and_then(|t| t.get("value")).and_then(Value::as_f64) {
        Some(v) if from_unit(v as f32, unit, ppi).max(1.0) == px => v as f32,
        _ => to_unit(px, unit, ppi),
    }
}

fn typed_key(key: &str) -> String {
    format!("__{key}Typed")
}

/// Swap Width and Height (the Orientation buttons), with the values typed for them, so a typed
/// 841 x 1189 mm turns into 1189 x 841 mm, not 1188.9 x 841.
pub fn swap_size(f: &mut Map<String, Value>) {
    f.remove("__savedPreset");
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(h));
    f.insert("height".into(), px_value(w));
    let (typed_w, typed_h) = (f.remove(&typed_key("width")), f.remove(&typed_key("height")));
    if let Some(t) = typed_h {
        f.insert(typed_key("width"), t);
    }
    if let Some(t) = typed_w {
        f.insert(typed_key("height"), t);
    }
}

/// Apply a preset to the dialog fields.
pub fn apply_preset(f: &mut Map<String, Value>, p: &Preset) {
    f.remove("__savedPreset");
    f.insert("width".into(), json!(p.1));
    f.insert("height".into(), json!(p.2));
    f.insert("resolution".into(), json!(p.3));
    f.insert("__preset".into(), json!(p.0));
    // Print and photo presets are specified in inches, screen presets in pixels.
    f.insert("__unit".into(), json!(if p.3 >= 300.0 { "in" } else { "px" }));
}

/// Fields `file.new` takes (drops the dialog's `__` UI keys).
pub fn command_params(f: &Map<String, Value>) -> Value {
    Value::Object(f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Set the resolution (pixels/inch) the way Photoshop's New Document does (#758): with Width/Height
/// in a physical unit the physical size is kept and the pixel count changes; in pixels the pixels
/// are kept.
pub fn set_resolution(f: &mut Map<String, Value>, new_ppi: f32) {
    let old_ppi = get_f(f, "resolution", 72.0);
    f.insert("resolution".into(), json!(new_ppi));
    if old_ppi != new_ppi {
        f.remove("__savedPreset");
    }
    if get_s(f, "__unit", "px") == "px" || !(old_ppi > 0.0 && new_ppi > 0.0) || old_ppi == new_ppi {
        return;
    }
    let scale = new_ppi / old_ppi;
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(w * scale));
    f.insert("height".into(), px_value(h * scale));
    f.remove("__preset");
    f.remove("__savedPreset");
}

fn get_f(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn get_s(f: &Map<String, Value>, k: &str, d: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
}

fn small_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).size(11.5).color(t.text_dim));
}

/// Lay out a preset card's title centred in `width`: wrapped onto at most two lines, the rest
/// elided, so long translations stay inside the card.
fn card_title(painter: &egui::Painter, title: &str, width: f32, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(title.to_owned(), egui::FontId::proportional(12.0), color, width);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// Paint a page thumbnail with the preset's aspect ratio.
fn page_icon(ui: &egui::Ui, r: Rect, w: u32, h: u32, t: &Tokens) {
    let s = ((r.width() - 8.0) / w.max(1) as f32).min((r.height() - 8.0) / h.max(1) as f32);
    let page = Rect::from_center_size(r.center(), vec2(w as f32 * s, h as f32 * s));
    ui.painter().rect_stroke(page, t.radius_sm, Stroke::new(1.2, t.text_dim), StrokeKind::Inside);
}

/// Restore a snapshot without replacing the new document's name or its clipboard offer.
pub fn apply_saved_preset(f: &mut Map<String, Value>, preset: &photocraft_engine::document_preset_cmds::DocumentPreset) {
    f.remove("backgroundColor");
    if let Some(params) = preset.settings.command_params().as_object() {
        f.extend(params.clone());
    }
    f.insert("__unit".into(), json!(preset.settings.unit));
    f.insert("__resUnit".into(), json!(preset.settings.resolution_unit));
    f.insert("__savedPreset".into(), json!(preset.name));
    f.remove("__preset");
    // Typed Width/Height belong to the previous values (`shown_size`).
    f.remove(&typed_key("width"));
    f.remove(&typed_key("height"));
}

fn saved_presets(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let query = get_s(f, "__search", "").to_lowercase();
    let presets: Vec<_> = app.session.presets.documents.iter().filter(|p| p.name.to_lowercase().contains(query.trim())).cloned().collect();
    if presets.is_empty() {
        ui.label(if app.session.presets.documents.is_empty() { tl!("No saved presets yet.") } else { tl!("No matching formats.") });
        return;
    }
    egui::ScrollArea::vertical().id_salt("saved-document-presets").max_height(320.0).show(ui, |ui| {
        for preset in &presets {
            ui.push_id(&preset.name, |ui| {
                ui.horizontal(|ui| {
                    let selected = get_s(f, "__savedPreset", "") == preset.name;
                    let label = egui::Button::new(&preset.name).selected(selected).truncate();
                    if ui.add_sized(vec2((ui.available_width() - 84.0).max(80.0), 32.0), label).on_hover_text(&preset.name).clicked() {
                        apply_saved_preset(f, preset);
                    }
                    if widgets::secondary_button(ui, tl!("Delete"), 76.0).clicked() {
                        match app.run("document.presets.delete", json!({"name":preset.name})) {
                            Ok(_) => {
                                f.remove("__presetError");
                                if selected {
                                    f.remove("__savedPreset");
                                }
                            }
                            Err(e) => {
                                f.insert("__presetError".into(), json!(e.to_string()));
                            }
                        }
                    }
                });
                let s = &preset.settings;
                small_label(ui, &format!("{} × {} px @ {} ppi · {}/{}", s.width, s.height, s.resolution, s.mode.to_uppercase(), s.depth));
                ui.add_space(8.0);
            });
        }
    });
}

fn save_preset(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(12.0);
    if f.get("__savingPreset").and_then(Value::as_bool) != Some(true) {
        if widgets::secondary_button(ui, tl!("Save Preset…"), 240.0).clicked() {
            f.insert("__savingPreset".into(), json!(true));
            f.remove("__presetError");
        }
    } else {
        // Consume before TextEdit sees Enter, so naming can never confirm the outer dialog.
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let label = ui.label(tl!("Preset Name"));
        let mut name = get_s(f, "__presetName", "");
        ui.add(egui::TextEdit::singleline(&mut name).id_salt("document-preset-name").desired_width(240.0).char_limit(255)).labelled_by(label.id);
        f.insert("__presetName".into(), json!(name));
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Save"), 110.0).clicked() || enter {
                let mut settings = command_params(f);
                settings["unit"] = json!(get_s(f, "__unit", "px"));
                settings["resolutionUnit"] = json!(get_s(f, "__resUnit", "in"));
                match app.run("document.presets.save", json!({"name":name,"settings":settings})) {
                    Ok(_) => {
                        if let Some(preset) = app.session.presets.documents.last() {
                            apply_saved_preset(f, preset);
                        }
                        f.insert("__category".into(), json!("Saved"));
                        f.insert("__search".into(), json!(""));
                        f.remove("__savingPreset");
                        f.remove("__presetName");
                        f.remove("__presetError");
                        ui.ctx().request_repaint();
                    }
                    Err(e) => {
                        f.insert("__presetError".into(), json!(e.to_string()));
                    }
                }
            }
            if widgets::secondary_button(ui, tl!("Cancel"), 110.0).clicked() {
                f.remove("__savingPreset");
                f.remove("__presetError");
            }
        });
    }
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        ui.add(egui::Label::new(error).wrap());
    }
}

/// Keep the modal within the viewport, including its frame and fixed footer.
pub fn dialog_width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() - 48.0).clamp(280.0, 960.0)
}

pub fn body(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let label = ui.label(RichText::new(tl!("Search formats")).color(t.text_dim));
    let search_id = ui.make_persistent_id("new-document-search");
    // Enter in search selects a result. It must never also confirm the outer modal.
    let enter = ui.memory(|m| m.has_focus(search_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let mut query = get_s(f, "__search", "");
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 80.0).max(100.0);
        ui.add(egui::TextEdit::singleline(&mut query).id(search_id).desired_width(width).char_limit(128).hint_text(tl!("Name, size or aspect ratio")))
            .labelled_by(label.id);
        if widgets::secondary_button(ui, tl!("Clear"), 64.0).clicked() {
            query.clear();
        }
    });
    f.insert("__search".into(), json!(query));
    ui.add_space(8.0);
    let mut category = get_s(f, "__category", "Popular");
    ui.horizontal_wrapped(|ui| {
        for name in ["Popular", "Social Media", "All", "Photo", "Print", "Art & Illustration", "Web", "Mobile", "Film & Video", "Saved"] {
            if ui.selectable_label(category == name, RichText::new(tl!(name)).color(if category == name { t.accent_text } else { t.text })).clicked() {
                category = name.to_owned();
                f.insert("__category".into(), json!(name));
                f.insert("__search".into(), json!(""));
                query.clear();
            }
        }
    });
    ui.add_space(10.0);
    let mut presets = crate::new_doc_catalog::formats(&category, &query, crate::i18n::current());
    if let Some(p) = clipboard_preset(f).filter(|p| {
        (category == "Popular" || category == "Recent" || category == "All" || !query.trim().is_empty())
            && crate::new_doc_catalog::matches(p, &query, crate::i18n::current())
    }) {
        presets.insert(0, p);
    }
    if enter {
        if category == "Saved" {
            if let Some(p) = app.session.presets.documents.iter().find(|p| p.name.to_lowercase().contains(query.to_lowercase().trim())).cloned() {
                apply_saved_preset(f, &p);
            }
        } else if let Some(p) = presets.first() {
            apply_preset(f, p);
        }
        ui.memory_mut(|m| m.surrender_focus(search_id));
    }
    let total = ui.available_width();
    if total >= 760.0 {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.vertical(|ui| {
                ui.set_width(total - 308.0);
                catalog(app, ui, f, &category, &presets);
            });
            ui.vertical(|ui| {
                ui.set_width(288.0);
                details(app, ui, f);
            });
        });
    } else {
        catalog(app, ui, f, &category, &presets);
        ui.add_space(12.0);
        details(app, ui, f);
    }
}

fn catalog(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, category: &str, presets: &[Preset]) {
    if category == "Saved" {
        saved_presets(app, ui, f);
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(crate::i18n::fmt(tl!("BLANK DOCUMENT PRESETS ({n})"), &[("n", &presets.len().to_string())])).size(11.0).color(t.text_faint));
    ui.add_space(8.0);
    if presets.is_empty() {
        ui.label(tl!("No matching formats."));
    }
    let columns = ((ui.available_width() + 10.0) / 174.0).floor().clamp(1.0, 3.0) as usize;
    let card = vec2((ui.available_width() - 10.0 * (columns.saturating_sub(1)) as f32) / columns as f32, 146.0);
    let chosen = get_s(f, "__preset", "");
    egui::ScrollArea::vertical().id_salt("new-document-formats").max_height(330.0).auto_shrink([false, true]).show(ui, |ui| {
        for row in presets.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for p in row {
                    let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                    let on = chosen == p.0;
                    let t = if on && t.bevel { Tokens { text: t.accent_text, text_dim: t.accent_text, text_faint: t.accent_text, ..t } } else { t };
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), on, tl!(p.0)));
                    ui.painter().rect_filled(
                        r,
                        t.radius,
                        if on {
                            t.row_selected
                        } else if resp.hovered() {
                            t.hover
                        } else {
                            t.field
                        },
                    );
                    ui.painter().rect_stroke(
                        r,
                        t.radius,
                        Stroke::new(if on || resp.has_focus() { 1.5 } else { 1.0 }, if on || resp.has_focus() { t.accent } else { t.field_border }),
                        StrokeKind::Inside,
                    );
                    page_icon(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 34.0), vec2(72.0, 52.0)), p.1, p.2, &t);
                    let title = card_title(ui.painter(), tl!(p.0), card.x - 16.0, t.text);
                    ui.painter().galley(pos2(r.center().x, r.top() + 82.0 - title.size().y / 2.0), title, t.text);
                    ui.painter().text(
                        pos2(r.center().x, r.top() + 112.0),
                        Align2::CENTER_CENTER,
                        format!("{} × {} px", p.1, p.2),
                        crate::theme::mono(11.0),
                        t.text_dim,
                    );
                    let info = if p.3 >= 300.0 {
                        format!("{} · {} ppi", crate::new_doc_catalog::aspect_ratio(p.1, p.2), p.3)
                    } else {
                        crate::new_doc_catalog::aspect_ratio(p.1, p.2)
                    };
                    ui.painter().text(pos2(r.center().x, r.top() + 132.0), Align2::CENTER_CENTER, info, egui::FontId::proportional(11.0), t.text_faint);
                    if resp.on_hover_text(tl!(p.0)).clicked() {
                        apply_preset(f, p);
                    }
                }
            });
            ui.add_space(10.0);
        }
    });
}

fn preview(ui: &mut egui::Ui, f: &Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (w, h) = (get_f(f, "width", 1920.0).clamp(1.0, 300_000.0) as u32, get_f(f, "height", 1080.0).clamp(1.0, 300_000.0) as u32);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 118.0), Sense::hover());
    ui.painter().rect_filled(r, t.radius, t.field);
    page_icon(ui, Rect::from_center_size(r.center() - vec2(0.0, 9.0), vec2(132.0, 78.0)), w, h, &t);
    ui.painter().text(
        r.center_bottom() - vec2(0.0, 14.0),
        Align2::CENTER_CENTER,
        format!("{} × {} px · {}", w, h, crate::new_doc_catalog::aspect_ratio(w, h)),
        crate::theme::mono(11.0),
        t.text_dim,
    );
    ui.add_space(12.0);
}

fn details(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.card).stroke(Stroke::new(1.0, t.card_border)).corner_radius(t.radius).inner_margin(14.0).show(ui, |ui| {
        ui.set_min_width(260.0);
        preview(ui, f);
        ui.label(RichText::new(tl!("PRESET DETAILS")).size(11.0).color(t.text_faint));
        ui.add_space(4.0);
        let mut name = get_s(f, "name", tl!("Untitled-1"));
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(250.0).font(egui::FontId::proportional(15.0))).changed() {
            f.insert("name".into(), json!(name));
        }
        ui.add_space(8.0);
        let ppi = get_f(f, "resolution", 72.0);
        let mut unit = get_s(f, "__unit", "px");
        small_label(ui, tl!("Width"));
        ui.horizontal(|ui| {
            let mut w = shown_size(f, "width", 1920.0, &unit, ppi);
            if widgets::value_field(ui, &mut w, 0.01..=300_000.0, "", 110.0).changed() {
                set_size(f, "width", w, &unit, ppi);
            }
            let opts: Vec<(String, &str)> = UNITS.iter().map(|u| (u.0.to_string(), u.1)).collect();
            if widgets::dropdown(ui, "nd-unit", &mut unit, &opts, 120.0) {
                f.insert("__unit".into(), json!(unit));
                f.remove("__savedPreset");
            }
        });
        small_label(ui, tl!("Height"));
        ui.horizontal(|ui| {
            let mut h = shown_size(f, "height", 1080.0, &unit, ppi);
            if widgets::value_field(ui, &mut h, 0.01..=300_000.0, "", 110.0).changed() {
                set_size(f, "height", h, &unit, ppi);
            }
            ui.add_space(6.0);
            small_label(ui, tl!("Orientation"));
            let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
            for (icon, portrait) in [("rectangle-vertical", true), ("rectangle-horizontal", false)] {
                if icons::button(ui, icon, 24.0, (h > w) == portrait, if portrait { "Portrait" } else { "Landscape" }).clicked() && (h > w) != portrait {
                    swap_size(f);
                }
            }
        });
        ui.add_space(4.0);
        small_label(ui, tl!("Background Contents"));
        let mut bg = get_s(f, "background", "white");
        let opts = [
            ("white".to_string(), tl!("White")),
            ("black".to_string(), tl!("Black")),
            ("backgroundColor".to_string(), tl!("Background Color")),
            ("transparent".to_string(), tl!("Transparent")),
        ];
        if widgets::dropdown(ui, "nd-bg", &mut bg, &opts, 240.0) {
            f.insert("background".into(), json!(bg));
            f.remove("__savedPreset");
            // A fresh choice of Background Color uses today's toolbox colour. Selecting a
            // saved preset instead restores the colour captured when it was saved.
            f.remove("backgroundColor");
        }

        ui.add_space(8.0);
        egui::CollapsingHeader::new(tl!("Advanced"))
            .id_salt("new-document-advanced")
            .default_open(f.get("__advanced").and_then(Value::as_bool).unwrap_or(false))
            .show(ui, |ui| {
                ui.add_space(4.0);
                small_label(ui, tl!("Resolution"));
                ui.horizontal(|ui| {
                    let per_cm = get_s(f, "__resUnit", "in") == "cm";
                    let mut r = if per_cm { ppi / 2.54 } else { ppi };
                    if widgets::value_field(ui, &mut r, 1.0..=30_000.0, "", 110.0).changed() {
                        set_resolution(f, if per_cm { r * 2.54 } else { r });
                    }
                    let mut ru = get_s(f, "__resUnit", "in");
                    if widgets::dropdown(
                        ui,
                        "nd-resunit",
                        &mut ru,
                        &[("in".to_string(), tl!("Pixels/Inch")), ("cm".to_string(), tl!("Pixels/Centimeter"))],
                        120.0,
                    ) {
                        f.insert("__resUnit".into(), json!(ru));
                        f.remove("__savedPreset");
                    }
                });
                ui.add_space(4.0);
                small_label(ui, tl!("Color Mode"));
                ui.horizontal(|ui| {
                    let mut mode = get_s(f, "mode", "rgb");
                    if widgets::dropdown(
                        ui,
                        "nd-mode",
                        &mut mode,
                        &[
                            ("gray".to_string(), tl!("Grayscale")),
                            ("rgb".to_string(), tl!("RGB Color")),
                            ("cmyk".to_string(), tl!("CMYK Color")),
                            ("lab".to_string(), tl!("Lab Color")),
                        ],
                        110.0,
                    ) {
                        f.insert("mode".into(), json!(mode));
                        f.remove("__savedPreset");
                    }
                    let mut depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
                    let depth_options: Vec<(u64, &str, &str)> = DEPTH_OPTIONS.iter().map(|(bits, label, tooltip)| (*bits, *label, *tooltip)).collect();
                    if widgets::dropdown_with_tooltips(ui, "nd-depth", &mut depth, &depth_options, 120.0) {
                        f.insert("depth".into(), json!(depth));
                        f.remove("__savedPreset");
                    }
                });
            });
        save_preset(app, ui, f);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_round_trip_through_pixels() {
        assert_eq!(to_unit(2100.0, "in", 300.0), 7.0);
        assert_eq!(from_unit(7.0, "in", 300.0), 2100.0);
        assert_eq!(from_unit(2.54, "cm", 300.0), 300.0);
        assert_eq!(to_unit(640.0, "px", 72.0), 640.0);
    }

    /// #1147: 5 mm at 72 ppi is 14 px, which is 4.94 mm. The field must keep showing the typed 5.
    #[test]
    fn typed_size_in_a_physical_unit_shows_as_typed() {
        let mut f = crate::state::UiState::new_document_fields();
        set_size(&mut f, "height", 5.0, "mm", 72.0);
        assert_eq!(command_params(&f)["height"], json!(14));
        assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), 5.0);
        // Another unit, or pixels changed elsewhere (a preset, the orientation swap): converted.
        assert_eq!(shown_size(&f, "height", 1080.0, "cm", 72.0), to_unit(14.0, "cm", 72.0));
        f.insert("height".into(), json!(1080));
        assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), to_unit(1080.0, "mm", 72.0));
        // Pixels show whole pixels.
        set_size(&mut f, "width", 512.4, "px", 72.0);
        assert_eq!(shown_size(&f, "width", 1920.0, "px", 72.0), 512.0);
        // Over the 300000 px limit the field shows the limit.
        set_size(&mut f, "width", 300_000.0, "mm", 72.0);
        assert_eq!(command_params(&f)["width"], json!(300_000));
        assert_eq!(shown_size(&f, "width", 1920.0, "mm", 72.0), to_unit(300_000.0, "mm", 72.0));
    }

    /// #1147: the Orientation buttons swap the typed values too: 841 x 1189 mm becomes 1189 x 841.
    #[test]
    fn orientation_swap_keeps_the_typed_values() {
        let mut f = crate::state::UiState::new_document_fields();
        set_size(&mut f, "width", 841.0, "mm", 72.0);
        set_size(&mut f, "height", 1189.0, "mm", 72.0);
        swap_size(&mut f);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone()), (json!(3370), json!(2384)));
        assert_eq!((shown_size(&f, "width", 1920.0, "mm", 72.0), shown_size(&f, "height", 1080.0, "mm", 72.0)), (1189.0, 841.0));
    }

    #[test]
    fn resolution_keeps_physical_size_in_physical_units_and_pixels_in_px() {
        let a4 = CATEGORIES.iter().find(|c| c.0 == "Print").unwrap().1.iter().find(|p| p.0 == "A4").unwrap();
        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, a4);
        f.insert("__unit".into(), json!("in"));
        set_resolution(&mut f, 150.0);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(1240), json!(1754), json!(150.0)));
        assert!(!f.contains_key("__preset"));

        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, a4);
        f.insert("__unit".into(), json!("px"));
        set_resolution(&mut f, 150.0);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(2480), json!(3508), json!(150.0)));
    }

    #[test]
    fn preset_card_titles_fit_the_card_in_every_language() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for lang in crate::i18n::Lang::all() {
                for p in CATEGORIES.iter().flat_map(|c| c.1.iter()) {
                    let title = crate::i18n::tr(lang, p.0);
                    let g = card_title(ui.painter(), title, 152.0, egui::Color32::WHITE);
                    assert!(g.size().x <= 152.0 && g.rows.len() <= 2 && !g.elided, "{}: {title}", lang.code());
                }
            }
        });
        out.textures_delta.clear();
    }

    #[test]
    fn new_document_depth_labels_and_tooltips_are_translated() {
        for lang in crate::i18n::Lang::all().filter(|lang| lang.code() != "en") {
            for (_, label, tooltip) in DEPTH_OPTIONS {
                assert_ne!(crate::i18n::tr(lang, label), *label, "{}: {label}", lang.code());
                assert_ne!(crate::i18n::tr(lang, tooltip), *tooltip, "{}: {tooltip}", lang.code());
            }
        }
    }

    #[test]
    fn clipboard_preset_comes_first_and_is_selected() {
        // Nothing on the clipboard: the dialog opens as before.
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let f = app.new_document_fields();
        assert_eq!(f, crate::state::UiState::new_document_fields());
        assert!(clipboard_preset(&f).is_none());
        // Pixels copied in the app: the Clipboard preset takes their size, at 72 ppi, selected.
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 20, "width": 123, "height": 45})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let f = app.new_document_fields();
        assert_eq!(clipboard_preset(&f), Some((CLIPBOARD, 123, 45, 72.0)));
        assert_eq!((f["width"].as_u64(), f["height"].as_u64(), f["__preset"].as_str()), (Some(123), Some(45), Some(CLIPBOARD)));
        let p = command_params(&f);
        assert!(p.get("__clipboard").is_none(), "file.new never sees the dialog's keys");
        assert_eq!((p["width"].as_u64(), p["height"].as_u64(), p["resolution"].as_f64()), (Some(123), Some(45), Some(72.0)));
    }

    #[test]
    fn preset_sets_size_resolution_and_create_params_drop_ui_keys() {
        let mut f = crate::state::UiState::new_document_fields();
        let a4 = CATEGORIES.iter().find(|c| c.0 == "Print").unwrap().1.iter().find(|p| p.0 == "A4").unwrap();
        apply_preset(&mut f, a4);
        let p = command_params(&f);
        assert_eq!(p["width"], 2480);
        assert_eq!(p["height"], 3508);
        assert_eq!(p["resolution"], 300.0);
        assert!(p.get("__preset").is_none() && p.get("__unit").is_none());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", p).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (2480, 3508, 300.0));
    }

    #[test]
    fn saved_selection_restores_all_settings_but_keeps_the_document_name() {
        let mut session = photocraft_engine::Session::new();
        session
            .execute(
                "document.presets.save",
                json!({"name":"Print proof","settings":{
                    "width":1600,"height":800,"resolution":254.0,"mode":"cmyk","depth":16,
                    "background":"backgroundColor","backgroundColor":[0.2,0.3,0.4],"unit":"mm","resolutionUnit":"cm"
                }}),
            )
            .unwrap();
        let preset = session.presets.documents[0].clone();
        let mut f = crate::state::UiState::new_document_fields();
        set_clipboard(&mut f, 20, 30);
        f.insert("name".into(), json!("Catalog cover"));
        apply_saved_preset(&mut f, &preset);
        assert_eq!(f["name"], "Catalog cover");
        assert_eq!(f["__unit"], "mm");
        assert_eq!(f["__resUnit"], "cm");
        assert_eq!(f["__savedPreset"], "Print proof");
        assert!(f.get("__preset").is_none());
        assert_eq!(clipboard_preset(&f), Some((CLIPBOARD, 20, 30, 72.0)));
        let mut expected = preset.settings.command_params();
        expected["name"] = json!("Catalog cover");
        assert_eq!(command_params(&f), expected);
        f.insert("width".into(), json!(42));
        assert_eq!(session.presets.documents[0], preset, "editing the form does not edit the snapshot");
        apply_preset(&mut f, &(CLIPBOARD, 20, 30, 72.0));
        assert!(f.get("__savedPreset").is_none());
        assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(20), Some(30)));
    }
    /// The real dialog (#254): a typed size must reach `file.new`, however it is confirmed.
    mod dialog {
        use super::super::{CATEGORIES, apply_preset};
        use crate::PhotocraftApp;
        use crate::state::{DialogKind, UiState};
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};
        use std::sync::{Arc, Mutex};

        /// The real preference services, backed by an in-memory store shared across app restarts.
        fn harness_with_store(store: &Arc<Mutex<Option<String>>>) -> Harness<'static, PhotocraftApp> {
            let (read, write) = (store.clone(), store.clone());
            let services = crate::Services {
                load_prefs: Some(Box::new(move || read.lock().unwrap().clone())),
                save_prefs: Some(Box::new(move |text| {
                    *write.lock().unwrap() = Some(text.to_string());
                    Ok(())
                })),
                ..Default::default()
            };
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
                |ui, app| {
                    crate::prefs_ui::tick(app, ui.ctx());
                    crate::dialogs::show(app, ui.ctx());
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run();
            h
        }

        fn name_preset(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
            h.get_by_label("Save Preset…").click();
            h.run();
            h.get_by_role_and_label(Role::TextInput, "Preset Name").click();
            h.run_steps(1);
            h.event(egui::Event::Text(name.into()));
            h.run_steps(1);
        }

        #[test]
        fn save_restart_select_create_and_delete_through_the_dialog() {
            let store = Arc::new(Mutex::new(None));
            let mut h = harness_with_store(&store);
            type_into(&mut h, 0, "1600");
            type_into(&mut h, 1, "1600");
            let mut f = fields(&h);
            f.insert("background".into(), serde_json::json!("transparent"));
            set_fields(&mut h, f);
            name_preset(&mut h, " Product square ");
            h.get_by_label("Save").click();
            h.run();
            assert!(h.state().session.documents().is_empty(), "saving does not create a document");
            assert_eq!(h.state().session.presets.documents[0].name, "Product square");
            type_into(&mut h, 0, "32");
            assert_eq!(h.state().session.presets.documents[0].settings.width, 1600);
            drop(h);

            let mut h = harness_with_store(&store);
            h.get_by_label("Saved").click();
            h.run();
            h.get_by_label("Product square").click();
            h.run();
            assert!(h.state().session.documents().is_empty(), "selection only populates the form");
            assert_eq!(fields(&h)["name"], "Untitled-1");
            assert_eq!(fields(&h)["background"], "transparent");
            h.get_by_label("Create").click();
            h.run();
            assert_eq!(created(&h), (1600, 1600, 72.0));
            let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
            assert_eq!(pixel[3], 0.0);

            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run();
            h.get_by_label("Saved").click();
            h.run();
            h.get_by_label("Delete").click();
            h.run();
            drop(h);
            let mut h = harness_with_store(&store);
            h.get_by_label("Saved").click();
            h.run();
            assert!(h.state().session.presets.documents.is_empty());
            assert!(h.query_by_label("No saved presets yet.").is_some());
        }

        #[test]
        fn enter_saves_the_preset_and_bad_names_leave_the_form_open() {
            let mut h = harness();
            name_preset(&mut h, "Enter test");
            enter(&mut h);
            assert_eq!(h.state().session.presets.documents.len(), 1);
            assert!(h.state().session.documents().is_empty(), "Enter must not also press Create");
            assert_eq!(h.state().ui.dialogs.len(), 1);
            name_preset(&mut h, " ENTER TEST ");
            h.get_by_label("Save").click();
            h.run();
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
            assert!(h.state().session.documents().is_empty());
            h.get_by_label("Cancel").click();
            h.run();
            name_preset(&mut h, "");
            // Cancel keeps the entered name; clear it as an automation edit, then use the button.
            h.state_mut().ui.dialogs[0].fields.insert("__presetName".into(), serde_json::json!("   "));
            h.run();
            h.get_by_label("Save").click();
            h.run();
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
        }

        fn harness() -> Harness<'static, PhotocraftApp> {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h =
                Harness::builder().with_max_steps(64).with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run();
            h
        }

        fn click_at(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
            h.hover_at(at);
            h.run_steps(1);
            h.drag_at(at);
            h.run_steps(1);
            h.drop_at(at);
            h.run();
        }

        /// The Width (0), Height (1) and Resolution (2) fields.
        fn field(h: &Harness<'static, PhotocraftApp>, i: usize) -> egui::Rect {
            h.query_all_by_role(Role::SpinButton).nth(i).map(|n| n.rect()).expect("a size field")
        }

        /// Click into field `i` (which selects its text) and type `text`, as a user does.
        fn type_into(h: &mut Harness<'static, PhotocraftApp>, i: usize, text: &str) {
            if i == 2 && h.query_all_by_role(Role::SpinButton).count() == 2 {
                h.get_by_label("Advanced").click();
                h.run();
            }
            let r = field(h, i);
            click_at(h, r.center());
            for c in text.chars() {
                h.event(egui::Event::Text(c.to_string()));
                h.run_steps(1);
            }
        }

        fn fields(h: &Harness<'static, PhotocraftApp>) -> serde_json::Map<String, serde_json::Value> {
            h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
        }

        fn set_fields(h: &mut Harness<'static, PhotocraftApp>, f: serde_json::Map<String, serde_json::Value>) {
            h.state_mut().ui.dialogs[0].fields = f;
            h.run();
        }

        fn enter(h: &mut Harness<'static, PhotocraftApp>) {
            h.key_press(egui::Key::Enter);
            h.run();
        }

        fn created(h: &Harness<'static, PhotocraftApp>) -> (u32, u32, f32) {
            assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
            let d = &h.state().session.active().expect("a new document").doc;
            (d.size.width, d.size.height, d.resolution_dpi)
        }

        fn search(h: &mut Harness<'static, PhotocraftApp>, query: &str) {
            h.get_by_role_and_label(Role::TextInput, "Search formats").click();
            h.run_steps(1);
            h.event(egui::Event::Text(query.into()));
            h.run();
        }

        #[test]
        fn search_enter_selects_social_size_without_creating_until_create() {
            let mut h = harness();
            let mut f = fields(&h);
            f.insert("name".into(), serde_json::json!("Campaign"));
            f.insert("depth".into(), serde_json::json!(16));
            f.insert("background".into(), serde_json::json!("transparent"));
            set_fields(&mut h, f);
            h.get_by_label("Print").click();
            h.run();
            search(&mut h, "tiktok");
            assert!(h.query_by_role_and_label(Role::Button, "Stories / Reels / Shorts").is_some());
            enter(&mut h);
            assert!(h.state().session.documents().is_empty());
            assert_eq!(fields(&h)["__preset"], "Stories / Reels / Shorts");
            assert_eq!(fields(&h)["name"], "Campaign");
            assert_eq!(fields(&h)["depth"], 16);
            h.get_by_label("Create").click();
            h.run();
            assert_eq!(created(&h), (1080, 1920, 72.0));
            let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
            assert_eq!(pixel[3], 0.0);
        }

        #[test]
        fn empty_search_enter_keeps_the_form_and_clear_restores_the_category() {
            let mut h = harness();
            search(&mut h, "no-such-format");
            assert!(h.query_by_label("No matching formats.").is_some());
            enter(&mut h);
            assert!(h.state().session.documents().is_empty());
            assert_eq!(fields(&h)["width"], 1920);
            h.get_by_label("Clear").click();
            h.run();
            assert!(h.query_by_role_and_label(Role::Button, "Stories / Reels / Shorts").is_some());
        }

        fn assert_centered(h: &Harness<'static, PhotocraftApp>) {
            let id = h.state().ui.dialogs[0].id;
            let rect = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", id)))).expect("modal rect");
            assert!((rect.center() - h.ctx.content_rect().center()).length() <= 2.0, "modal {rect:?}, viewport {:?}", h.ctx.content_rect());
        }

        #[test]
        fn opens_and_reopens_centered_in_every_theme_even_with_a_remembered_position() {
            for theme in crate::theme::ThemeKind::ALL {
                let mut h = harness();
                PhotocraftApp::setup_context(&h.ctx, theme);
                h.ctx.data_mut(|m| m.insert_persisted(egui::Id::new(("dialog-place", "NewDocument")), egui::vec2(5.0, 7.0)));
                h.run();
                assert_centered(&h);
                h.get_by_label("Advanced").click();
                h.run();
                assert_centered(&h);
                assert!(h.ctx.content_rect().contains_rect(h.get_by_label("Create").rect()));
                h.get_by_label("Close").click();
                h.run();
                h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
                h.run();
                assert_centered(&h);
                h.set_size(egui::vec2(640.0, 740.0));
                h.run();
                assert_centered(&h);
                assert!(h.ctx.content_rect().contains_rect(h.get_by_label("Create").rect()));
            }
        }

        #[test]
        fn saving_from_a_search_clears_the_filter_and_saved_search_selects_by_name() {
            let mut h = harness();
            search(&mut h, "4:5");
            enter(&mut h);
            name_preset(&mut h, "Campaign vertical");
            h.get_by_label("Save").click();
            h.run();
            assert_eq!(fields(&h)["__search"], "");
            assert!(h.query_by_label("Campaign vertical").is_some());
            search(&mut h, "campaign");
            enter(&mut h);
            assert_eq!(fields(&h)["__savedPreset"], "Campaign vertical");
            assert!(h.state().session.documents().is_empty());
            h.get_by_label("Create").click();
            h.run();
            assert_eq!(created(&h), (1080, 1350, 72.0));
        }

        #[test]
        fn narrow_viewport_keeps_search_and_create_inside_the_window() {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h =
                Harness::builder().with_max_steps(64).with_size(egui::vec2(640.0, 740.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run();
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 740.0));
            assert!(viewport.contains_rect(h.get_by_label("Create").rect()), "create {:?}, viewport {:?}", h.get_by_label("Create").rect(), viewport);
            assert!(viewport.contains_rect(h.get_by_role_and_label(Role::TextInput, "Search formats").rect()));
            search(&mut h, "1080x1350");
            h.get_by_role_and_label(Role::Button, "Instagram post, 4:5").click();
            h.run();
            h.get_by_label("Create").click();
            h.run();
            assert_eq!(created(&h), (1080, 1350, 72.0));
        }

        #[test]
        fn arithmetic_dimensions_create_the_evaluated_size() {
            let mut h = harness();
            type_into(&mut h, 0, "1920/2");
            type_into(&mut h, 1, "(100+50)*2");
            enter(&mut h);
            assert_eq!(created(&h), (960, 300, 72.0));
        }

        #[test]
        fn typed_size_then_enter_creates_that_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            let f = fields(&h);
            assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(512), Some(512)), "whole pixels: {f:?}");
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
        }

        #[test]
        fn typed_size_then_create_without_leaving_the_field_creates_that_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "300");
            // Still editing Height: click Create straight away.
            let create = h.get_by_label("Create").rect();
            click_at(&mut h, create.center());
            assert_eq!(created(&h), (512, 300, 72.0));
        }

        #[test]
        fn typing_over_a_preset_wins() {
            let mut h = harness();
            let mut f = fields(&h);
            let web = CATEGORIES.iter().find(|c| c.0 == "Web").unwrap().1.iter().find(|p| p.0 == "Web Minimum").unwrap();
            apply_preset(&mut f, web);
            set_fields(&mut h, f);
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            assert!(fields(&h).get("__preset").is_none(), "typing deselects the preset");
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
        }

        #[test]
        fn the_clipboard_card_is_first_and_creates_the_clipboard_size() {
            let mut h = harness();
            let mut f = fields(&h);
            super::super::set_clipboard(&mut f, 640, 360);
            set_fields(&mut h, f);
            assert!(h.query_by_label_contains("BLANK DOCUMENT PRESETS (7)").is_some());
            let clipboard = h.get_by_role_and_label(Role::Button, super::super::CLIPBOARD).rect();
            let social = h.get_by_role_and_label(Role::Button, "Stories / Reels / Shorts").rect();
            assert!(clipboard.left() < social.left(), "clipboard is the first card");
            click_at(&mut h, social.center());
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Stories / Reels / Shorts"));
            click_at(&mut h, clipboard.center());
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some(super::super::CLIPBOARD));
            enter(&mut h);
            assert_eq!(created(&h), (640, 360, 72.0));
        }

        #[test]
        fn clicking_a_preset_card_after_typing_sets_its_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            h.get_by_label("Photo").click();
            h.run();
            h.get_by_role_and_label(Role::Button, "Landscape, 6 x 4").click();
            h.run();
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Landscape, 6 x 4"));
            enter(&mut h);
            assert_eq!(created(&h), (1800, 1200, 300.0));
        }

        #[test]
        fn typed_size_in_inches_converts_at_the_resolution() {
            let mut h = harness();
            type_into(&mut h, 2, "300");
            let mut f = fields(&h);
            f.insert("__unit".into(), serde_json::json!("in"));
            set_fields(&mut h, f);
            type_into(&mut h, 0, "2");
            type_into(&mut h, 1, "1.5");
            enter(&mut h);
            assert_eq!(created(&h), (600, 450, 300.0));
        }

        /// #1147: typed digit by digit, millimetres used to be rewritten after each keystroke
        /// (`8` became `8.1`), so 841 never arrived.
        #[test]
        fn typed_size_in_millimetres_keeps_every_digit() {
            let mut h = harness();
            let mut f = fields(&h);
            f.insert("__unit".into(), serde_json::json!("mm"));
            set_fields(&mut h, f);
            type_into(&mut h, 0, "841");
            type_into(&mut h, 1, "1189");
            enter(&mut h);
            assert_eq!(created(&h), (2384, 3370, 72.0));
        }

        #[test]
        fn changing_units_keeps_the_typed_pixel_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            for unit in ["in", "cm", "mm", "pt", "pica", "px"] {
                let mut f = fields(&h);
                f.insert("__unit".into(), serde_json::json!(unit));
                set_fields(&mut h, f);
            }
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
        }

        #[test]
        fn orientation_swap_keeps_whole_pixels() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "256");
            // The Portrait icon button follows the "Orientation" label (icons have tooltips only).
            let label = h.get_by_label("Orientation").rect();
            let gap = h.ctx.global_style().spacing.item_spacing.x;
            click_at(&mut h, egui::pos2(label.right() + gap + 12.0, label.center().y));
            enter(&mut h);
            assert_eq!(created(&h), (256, 512, 72.0));
        }
    }
}
