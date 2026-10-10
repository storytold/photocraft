//! File › New: Photoshop's New Document dialog. Category tabs with blank-document presets on the
//! left, Preset Details on the right. Values live in the dialog fields (`width`/`height` in pixels,
//! `resolution` in ppi, `mode`, `depth`, `background`, `name`), so `ui.dialog.set` drives it and
//! Create runs `file.new` with them.

use egui::{Align2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{icons, widgets};

/// A blank-document preset: (name, width px, height px, ppi).
pub type Preset = (&'static str, u32, u32, f32);

/// Photoshop's New Document categories and their blank-document presets.
pub const CATEGORIES: &[(&str, &[Preset])] = &[
    ("Recent", &[("Default PhotoCraft Size", 2100, 1500, 300.0), ("HDTV 1080p", 1920, 1080, 72.0)]),
    (
        "Photo",
        &[
            ("Landscape, 6 x 4", 1800, 1200, 300.0),
            ("Landscape, 7 x 5", 2100, 1500, 300.0),
            ("Landscape, 10 x 8", 3000, 2400, 300.0),
            ("Portrait, 4 x 6", 1200, 1800, 300.0),
            ("Portrait, 5 x 7", 1500, 2100, 300.0),
            ("Square, 5 x 5", 1500, 1500, 300.0),
        ],
    ),
    (
        "Print",
        &[
            ("Letter", 2550, 3300, 300.0),
            ("Legal", 2550, 4200, 300.0),
            ("Tabloid", 3300, 5100, 300.0),
            ("A4", 2480, 3508, 300.0),
            ("A3", 3508, 4961, 300.0),
            ("A5", 1748, 2480, 300.0),
        ],
    ),
    (
        "Art & Illustration",
        &[("Poster", 5400, 7200, 300.0), ("Postcard", 1800, 1200, 300.0), ("Comic Book", 1988, 3075, 300.0), ("Square, 12 x 12", 3600, 3600, 300.0)],
    ),
    (
        "Web",
        &[
            ("Web Most Common", 1366, 768, 72.0),
            ("Web Minimum", 1024, 768, 72.0),
            ("Web Large", 1920, 1080, 72.0),
            ("MacBook Pro 16\"", 3456, 2234, 72.0),
            ("iMac 24\"", 4480, 2520, 72.0),
        ],
    ),
    (
        "Mobile",
        &[
            ("iPhone 16", 1179, 2556, 72.0),
            ("iPhone 16 Pro Max", 1320, 2868, 72.0),
            ("iPad Pro 13\"", 2064, 2752, 72.0),
            ("Android 1080p", 1080, 1920, 72.0),
            ("Apple Watch 45mm", 396, 484, 72.0),
        ],
    ),
    (
        "Film & Video",
        &[
            ("HDTV 1080p", 1920, 1080, 72.0),
            ("HDTV 720p", 1280, 720, 72.0),
            ("UHD 4K", 3840, 2160, 72.0),
            ("DCI 4K", 4096, 2160, 72.0),
            ("UHD 8K", 7680, 4320, 72.0),
        ],
    ),
];

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
    let s = 30.0 / (w.max(h) as f32);
    let page = Rect::from_center_size(r.center(), vec2(w as f32 * s, h as f32 * s));
    ui.painter().rect_stroke(page, 1.0, Stroke::new(1.2, t.text_dim), StrokeKind::Inside);
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

fn saved_presets(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let presets = app.session.presets.documents.clone();
    if presets.is_empty() {
        ui.label(tl!("No saved presets yet."));
        return false;
    }
    let mut create = false;
    egui::ScrollArea::vertical().id_salt("saved-document-presets").max_height(320.0).show(ui, |ui| {
        for preset in &presets {
            ui.push_id(&preset.name, |ui| {
                ui.horizontal(|ui| {
                    let selected = get_s(f, "__savedPreset", "") == preset.name;
                    let label = egui::Button::new(&preset.name).selected(selected).truncate();
                    let response = ui.add_sized(vec2(422.0, 32.0), label).on_hover_text(&preset.name);
                    if response.clicked() {
                        apply_saved_preset(f, preset);
                        create |= response.double_clicked() || response.triple_clicked();
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
    create
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

/// Returns whether a preset double-click requested creation, after applying its settings.
/// A recent category click can make egui count the preset double-click as a triple-click.
pub fn body(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let t = Tokens::get(ui.ctx());
    let cat = get_s(f, "__category", "Recent");
    let mut create = false;
    // Category tabs.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for name in CATEGORIES.iter().map(|c| c.0).chain(["Saved"]) {
            let on = cat == name;
            let label = if name == "Saved" { tl!("Saved") } else { tl!(name) };
            let r = ui.add(egui::Label::new(RichText::new(label).size(13.0).color(if on { t.text } else { t.text_dim })).sense(Sense::click()));
            if on {
                ui.painter().line_segment([r.rect.left_bottom() + vec2(0.0, 3.0), r.rect.right_bottom() + vec2(0.0, 3.0)], Stroke::new(2.0, t.text));
            }
            if r.clicked() {
                f.insert("__category".into(), json!(name));
            }
        }
    });
    ui.add_space(6.0);
    widgets::hairline(ui);
    ui.add_space(8.0);
    let presets = CATEGORIES.iter().find(|c| c.0 == cat).map_or(CATEGORIES[0].1, |c| c.1);
    // The clipboard image's size comes first among the Recent presets.
    let presets: Vec<Preset> = clipboard_preset(f).filter(|_| cat == CATEGORIES[0].0).into_iter().chain(presets.iter().copied()).collect();
    let chosen = get_s(f, "__preset", "");
    ui.horizontal_top(|ui| {
        // Left: preset grid.
        ui.vertical(|ui| {
            ui.set_width(520.0);
            if cat == "Saved" {
                create |= saved_presets(app, ui, f);
                return;
            }
            ui.label(RichText::new(crate::i18n::fmt(tl!("BLANK DOCUMENT PRESETS ({n})"), &[("n", &presets.len().to_string())])).size(11.0).color(t.text_faint));
            ui.add_space(6.0);
            let card = vec2(164.0, 112.0);
            for row in presets.chunks(3) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for p in row {
                        let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                        let on = chosen == p.0;
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
                        if on {
                            ui.painter().rect_stroke(r, t.radius, Stroke::new(1.5, t.accent), StrokeKind::Inside);
                        }
                        page_icon(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 34.0), vec2(40.0, 40.0)), p.1, p.2, &t);
                        let title = card_title(ui.painter(), tl!(p.0), card.x - 12.0, t.text);
                        let elided = title.elided;
                        ui.painter().galley(pos2(r.center().x, r.top() + 70.0 - title.size().y / 2.0), title, t.text);
                        let unit = if p.3 >= 300.0 { "in" } else { "px" };
                        let size = if unit == "in" {
                            format!(
                                "{} x {} in @ {} ppi",
                                widgets::fmt_num(to_unit(p.1 as f32, "in", p.3) as f64),
                                widgets::fmt_num(to_unit(p.2 as f32, "in", p.3) as f64),
                                p.3
                            )
                        } else {
                            format!("{} x {} px @ {} ppi", p.1, p.2, p.3)
                        };
                        ui.painter().text(pos2(r.center().x, r.top() + 97.0), Align2::CENTER_CENTER, size, egui::FontId::proportional(10.5), t.text_faint);
                        let resp = if elided { resp.on_hover_text(tl!(p.0)) } else { resp };
                        if resp.clicked() {
                            apply_preset(f, p);
                            create |= resp.double_clicked() || resp.triple_clicked();
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });
        ui.add_space(10.0);
        // Right: Preset Details.
        ui.vertical(|ui| {
            ui.set_width(260.0);
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
            small_label(ui, tl!("Resolution"));
            ui.horizontal(|ui| {
                let per_cm = get_s(f, "__resUnit", "in") == "cm";
                let mut r = if per_cm { ppi / 2.54 } else { ppi };
                if widgets::value_field(ui, &mut r, 1.0..=30_000.0, "", 110.0).changed() {
                    set_resolution(f, if per_cm { r * 2.54 } else { r });
                }
                let mut ru = get_s(f, "__resUnit", "in");
                if widgets::dropdown(ui, "nd-resunit", &mut ru, &[("in".to_string(), tl!("Pixels/Inch")), ("cm".to_string(), tl!("Pixels/Centimeter"))], 120.0)
                {
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
            save_preset(app, ui, f);
        });
    });
    create
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
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
                |ui, app| {
                    crate::prefs_ui::tick(app, ui.ctx());
                    crate::dialogs::show(app, ui.ctx());
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(3);
            h
        }

        fn name_preset(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
            h.get_by_label("Save Preset…").click();
            h.run_steps(2);
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
            h.run_steps(3);
            assert!(h.state().session.documents().is_empty(), "saving does not create a document");
            assert_eq!(h.state().session.presets.documents[0].name, "Product square");
            type_into(&mut h, 0, "32");
            assert_eq!(h.state().session.presets.documents[0].settings.width, 1600);
            drop(h);

            let mut h = harness_with_store(&store);
            h.get_by_label("Saved").click();
            h.run_steps(2);
            h.get_by_label("Product square").click();
            h.run_steps(2);
            assert!(h.state().session.documents().is_empty(), "selection only populates the form");
            assert_eq!(fields(&h)["name"], "Untitled-1");
            assert_eq!(fields(&h)["background"], "transparent");
            h.get_by_label("Create").click();
            h.run_steps(3);
            assert_eq!(created(&h), (1600, 1600, 72.0));
            let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
            assert_eq!(pixel[3], 0.0);

            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(2);
            h.get_by_label("Saved").click();
            h.run_steps(2);
            h.get_by_label("Delete").click();
            h.run_steps(3);
            drop(h);
            let mut h = harness_with_store(&store);
            h.get_by_label("Saved").click();
            h.run_steps(2);
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
            h.run_steps(2);
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
            assert!(h.state().session.documents().is_empty());
            h.get_by_label("Cancel").click();
            h.run_steps(2);
            name_preset(&mut h, "");
            // Cancel keeps the entered name; clear it as an automation edit, then use the button.
            h.state_mut().ui.dialogs[0].fields.insert("__presetName".into(), serde_json::json!("   "));
            h.run_steps(2);
            h.get_by_label("Save").click();
            h.run_steps(2);
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
        }

        fn harness() -> Harness<'static, PhotocraftApp> {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder()
                .with_size(egui::vec2(1400.0, 900.0))
                .with_step_dt(1.0 / 60.0)
                .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(3);
            h
        }

        fn click_at(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
            h.hover_at(at);
            h.run_steps(1);
            h.drag_at(at);
            h.run_steps(1);
            h.drop_at(at);
            h.run_steps(2);
        }

        /// The Width (0), Height (1) and Resolution (2) fields.
        fn field(h: &Harness<'static, PhotocraftApp>, i: usize) -> egui::Rect {
            h.query_all_by_role(Role::SpinButton).nth(i).map(|n| n.rect()).expect("a size field")
        }

        /// Click into field `i` (which selects its text) and type `text`, as a user does.
        fn type_into(h: &mut Harness<'static, PhotocraftApp>, i: usize, text: &str) {
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
            h.run_steps(2);
        }

        fn enter(h: &mut Harness<'static, PhotocraftApp>) {
            h.key_press(egui::Key::Enter);
            h.run_steps(3);
        }

        fn created(h: &Harness<'static, PhotocraftApp>) -> (u32, u32, f32) {
            assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
            let d = &h.state().session.active().expect("a new document").doc;
            (d.size.width, d.size.height, d.resolution_dpi)
        }

        /// Preset cards paint their titles directly, without an accessibility label.
        fn preset_title_at(h: &Harness<'static, PhotocraftApp>, title: &str) -> egui::Pos2 {
            fn find(shape: &egui::Shape, title: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(text) if text.galley.job.text == title => Some(text.visual_bounding_rect().center()),
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, title)),
                    _ => None,
                }
            }
            h.output().shapes.iter().find_map(|shape| find(&shape.shape, title)).expect("visible preset title")
        }

        fn second_click_creates_once(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
            assert!(h.state().session.documents().is_empty(), "the first click only selects the preset");
            assert_eq!(h.state().ui.dialogs.len(), 1);
            // Two real pointer press/release pairs within egui's double-click interval.
            click_at(h, at);
            h.run_steps(3);
            assert_eq!(h.state().session.documents().len(), 1, "the second click creates exactly one document");
            assert!(h.state().ui.dialogs.is_empty(), "double-click closes the New Document dialog");
            assert!(h.state().ui.views[0].fit_pending, "the normal Create path fits the new document");
        }

        fn assert_pixel(h: &mut Harness<'static, PhotocraftApp>, expected: [f64; 4]) {
            let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
            for (actual, expected) in pixel.as_array().unwrap().iter().zip(expected) {
                assert!((actual.as_f64().unwrap() - expected).abs() < 0.001, "background pixel: {pixel}");
            }
        }

        #[test]
        fn double_click_builtin_preset_creates_its_size_and_current_details() {
            let mut h = harness();
            let mut f = fields(&h);
            f.extend(serde_json::json!({"name":"Portrait proof","mode":"cmyk","depth":16,"background":"transparent"}).as_object().unwrap().clone());
            set_fields(&mut h, f);
            h.get_by_label("Photo").click();
            h.run_steps(2);
            let at = preset_title_at(&h, "Portrait, 4 x 6");
            click_at(&mut h, at);
            assert_eq!(fields(&h)["__preset"], "Portrait, 4 x 6");
            assert_eq!(
                super::super::command_params(&fields(&h)),
                serde_json::json!({"name":"Portrait proof","width":1200,"height":1800,"resolution":300.0,
                    "mode":"cmyk","depth":16,"background":"transparent"})
            );
            second_click_creates_once(&mut h, at);
            assert_eq!(created(&h), (1200, 1800, 300.0));
            let doc = &h.state().session.active().unwrap().doc;
            assert_eq!(doc.name, "Portrait proof");
            assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Cmyk, photocraft_doc::SampleType::U16));
            assert_pixel(&mut h, [0.0; 4]);
        }

        #[test]
        fn double_click_clipboard_preset_creates_its_size_and_current_details() {
            let mut h = harness();
            let mut f = fields(&h);
            super::super::set_clipboard(&mut f, 37, 23);
            apply_preset(&mut f, &("Other size", 100, 200, 300.0));
            f.extend(serde_json::json!({"mode":"gray","depth":8,"background":"white"}).as_object().unwrap().clone());
            set_fields(&mut h, f);
            let at = preset_title_at(&h, super::super::CLIPBOARD);
            click_at(&mut h, at);
            assert_eq!(fields(&h)["__preset"], super::super::CLIPBOARD);
            assert_eq!(
                super::super::command_params(&fields(&h)),
                serde_json::json!({"name":"Untitled-1","width":37,"height":23,"resolution":72.0,
                    "mode":"gray","depth":8,"background":"white"})
            );
            second_click_creates_once(&mut h, at);
            assert_eq!(created(&h), (37, 23, 72.0));
            let doc = &h.state().session.active().unwrap().doc;
            assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::U8));
            assert_pixel(&mut h, [1.0; 4]);
        }

        #[test]
        fn double_click_saved_preset_restores_all_settings_and_captured_background() {
            let mut h = harness();
            h.state_mut()
                .run(
                    "document.presets.save",
                    serde_json::json!({"name":"Saved proof","settings":{
                        "width":48,"height":32,"resolution":254.0,"mode":"rgb","depth":32,
                        "background":"backgroundColor","backgroundColor":[0.125,0.375,0.625],"unit":"mm","resolutionUnit":"cm"
                    }}),
                )
                .unwrap();
            h.state_mut().session.tools.background = [1.0, 0.0, 1.0, 1.0];
            let mut f = fields(&h);
            f.insert("name".into(), serde_json::json!("Current document name"));
            set_fields(&mut h, f);
            h.get_by_label("Saved").click();
            h.run_steps(2);
            let at = h.get_by_label("Saved proof").rect().center();
            click_at(&mut h, at);
            assert_eq!(fields(&h)["__savedPreset"], "Saved proof");
            assert_eq!(fields(&h)["__unit"], "mm");
            assert_eq!(fields(&h)["__resUnit"], "cm");
            second_click_creates_once(&mut h, at);
            assert_eq!(created(&h), (48, 32, 254.0));
            let doc = &h.state().session.active().unwrap().doc;
            assert_eq!(doc.name, "Current document name");
            assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32));
            assert_pixel(&mut h, [0.125, 0.375, 0.625, 1.0]);
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
            // Recent lists the Clipboard card first: three presets instead of two.
            assert!(h.query_by_label_contains("BLANK DOCUMENT PRESETS (3)").is_some());
            let heading = h.get_by_label_contains("BLANK DOCUMENT PRESETS").rect();
            // Pick the second card, then the first (Clipboard) again.
            click_at(&mut h, heading.left_bottom() + egui::vec2(80.0 + 172.0, 60.0));
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Default PhotoCraft Size"));
            click_at(&mut h, heading.left_bottom() + egui::vec2(80.0, 60.0));
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some(super::super::CLIPBOARD));
            enter(&mut h);
            assert_eq!(created(&h), (640, 360, 72.0));
        }

        #[test]
        fn clicking_a_preset_card_after_typing_sets_its_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            h.get_by_label("Photo").click();
            h.run_steps(2);
            // The first card ("Landscape, 6 x 4") sits under the presets heading.
            let heading = h.get_by_label_contains("BLANK DOCUMENT PRESETS").rect();
            click_at(&mut h, heading.left_bottom() + egui::vec2(80.0, 60.0));
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
