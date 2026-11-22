//! Palette Map palette browser and mapping controls.
//!
//! The controls deliberately edit the dialog's JSON fields directly.  This keeps palette choice,
//! imported snapshots, and mapping settings visible to the control channel and lets Cancel discard
//! the whole editing session with the dialog as usual.

use egui::{Color32, CornerRadius, Sense, Ui, vec2};
use photocraft_algo::palette_map::{AlphaMode, Distance, Method, Palette, Settings};
use serde_json::{Map, Value, json};

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const FILTER_COMMAND: &str = "filter.paletteMap";
const IMPORT_REQUEST: &str = "__paletteImport";
const SEARCH_FIELD: &str = "__paletteSearch";
pub(crate) const CUSTOM_ID: &str = "custom";

/// The values the palette controls own: palette choice, embedded snapshot and mapping settings.
const OWNED_KEYS: [&str; 3] = ["paletteId", "paletteSnapshot", "mapping"];

/// Wheel gestures a slider row may take; the palette rows take none (the wheel scrolls their pane).
#[derive(Clone, Copy, Default)]
pub(crate) struct RowGestures;

/// Draw a menu of `options` (`id`, label) at `width`, keeping the id in `value`. The label is drawn
/// by the caller; the response reports a selection as changed.
pub(crate) fn dropdown<L: AsRef<str>>(ui: &mut Ui, id_salt: &str, _label: &str, value: &mut String, options: &[(String, L)], width: f32) -> egui::Response {
    let selected = options.iter().find(|(id, _)| id == value).map(|(_, label)| label.as_ref().to_owned()).unwrap_or_else(|| value.clone());
    let before = value.clone();
    let mut response = egui::ComboBox::from_id_salt(id_salt).width(width).selected_text(selected).show_ui(ui, |ui| {
        for (id, label) in options {
            ui.selectable_value(value, id.clone(), label.as_ref());
        }
    });
    if *value != before {
        response.response.mark_changed();
    }
    response.response
}

/// A label, a slider and its number field on one row. `extra` is kept for the schema rows' trailing
/// control; the palette rows pass `None`.
pub(crate) fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, extra: Option<f32>) -> egui::Response {
    slider_row_with(ui, label, value, range, unit, extra, RowGestures)
}

/// [`slider_row`] with the row's gestures (the palette rows take none).
pub(crate) fn slider_row_with(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    unit: &str,
    _extra: Option<f32>,
    _gestures: RowGestures,
) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut response = ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::t(label)).color(t.text_dim));
        changed |= ui.add(egui::Slider::new(value, range.clone()).show_value(false)).changed();
        let mut shown = *value;
        let drag = egui::DragValue::new(&mut shown).range(range).speed(0.01).suffix(unit);
        if ui.add_sized(vec2(58.0, ui.spacing().interact_size.y), drag).changed() {
            *value = shown;
            changed = true;
        }
    });
    if changed {
        response.response.mark_changed();
    }
    response.response
}

/// A fixed-width number field, as the threshold matrix cells are.
pub(crate) fn value_field(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, width: f32) -> egui::Response {
    let drag = egui::DragValue::new(value).range(range).speed(0.01).suffix(unit);
    ui.add_sized(vec2(width, ui.spacing().interact_size.y), drag)
}

/// The dialog's values as the plain JSON map the palette controls edit.
fn dialog_values(fields: &DialogFields) -> Map<String, Value> {
    match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Writes the values the palette controls own back into `fields`, keeping the others untouched.
fn write_owned(fields: &mut DialogFields, values: &Map<String, Value>) {
    for key in OWNED_KEYS {
        match values.get(key) {
            Some(Value::String(text)) => fields.set(key, FieldValue::Text(text.clone())),
            Some(value) => fields.set(key, FieldValue::Json(value.clone())),
            None => {
                fields.remove(key);
            }
        }
    }
}

/// Writes the controls' transient keys (search text, an import request, its error or name) back.
fn write_internal(fields: &mut DialogFields, values: &Map<String, Value>) {
    for (key, value) in values.iter().filter(|(key, _)| key.starts_with("__")) {
        fields.set(key.clone(), FieldValue::Json(value.clone()));
    }
}

/// Draws the palette browser into a custom dialog row, keeping its values in `fields`.
pub(crate) fn palette_row(ui: &mut Ui, fields: &mut DialogFields) -> egui::Response {
    let mut values = dialog_values(fields);
    palette_controls(ui, &mut values);
    write_owned(fields, &values);
    write_internal(fields, &values);
    ui.response()
}

/// Draws the mapping settings into a custom dialog row, keeping its values in `fields`.
pub(crate) fn mapping_row(ui: &mut Ui, fields: &mut DialogFields) -> egui::Response {
    let mut values = dialog_values(fields);
    mapping_controls(ui, &mut values);
    write_owned(fields, &values);
    write_internal(fields, &values);
    ui.response()
}

/// The palette fields a Palette Map dialog opens with: the last used, remembered choices when they
/// still validate, else the bundled default palette.
pub(crate) fn opening_fields(app: &PhotocraftApp, command: &str) -> Map<String, Value> {
    let mut values = Map::new();
    if let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(command) {
        for key in OWNED_KEYS {
            if let Some(value) = saved.get(key).filter(|value| valid_palette_widget_value(key, value)) {
                values.insert(key.into(), value.clone());
            }
        }
    }
    if !fixed_palette_selected(&values) {
        values.remove("paletteId");
        values.remove("paletteSnapshot");
    }
    // A custom selection without its snapshot is no palette: fall back to the bundled default.
    if values.get("paletteId").and_then(Value::as_str) == Some(CUSTOM_ID) && values.get("paletteSnapshot").is_none() {
        values.remove("paletteId");
    }
    ensure_filter_default(&mut values);
    values
}

/// Keeps the palette choices with the dialog's last-used values after a successful run.
pub(crate) fn remember(app: &mut PhotocraftApp, command: &str, fields: &DialogFields) {
    let values = dialog_values(fields);
    let saved: Vec<(String, Value)> =
        OWNED_KEYS.iter().filter_map(|key| values.get(*key).filter(|value| valid_palette_widget_value(key, value)).map(|value| ((*key).to_owned(), value.clone()))).collect();
    app.session.edit_prefs(|prefs| {
        let entry = prefs.dialogs.entry(command.to_owned()).or_insert_with(|| json!({}));
        if let Some(object) = entry.as_object_mut() {
            for (key, value) in saved {
                object.insert(key, value);
            }
        }
    });
}

/// Whether `command` uses the Palette Map controls.
pub fn owns_command(command: &str) -> bool {
    command == FILTER_COMMAND
}

/// Whether a remembered value still fits its key: a non-empty palette id, a valid embedded palette,
/// or mapping settings the engine accepts.
fn valid_palette_widget_value(key: &str, value: &Value) -> bool {
    match key {
        "paletteId" => value.as_str().is_some_and(|id| !id.is_empty()),
        "paletteSnapshot" => serde_json::from_value::<Palette>(value.clone()).is_ok_and(|palette| palette.validate().is_ok()),
        "mapping" => parse_mapping(value).is_ok(),
        _ => false,
    }
}

fn snapshot(fields: &Map<String, Value>) -> Option<Palette> {
    if let Some(value) = fields.get("paletteSnapshot")
        && !value.is_null()
    {
        // An embedded snapshot is authoritative.  Do not fall back to a stale ID when the
        // user-provided snapshot is malformed: the error must remain visible to the dialog.
        return serde_json::from_value(value.clone()).ok();
    }
    selected_id(fields).and_then(photocraft_algo::palettes::builtin).map(|builtin| builtin.palette.clone())
}

fn snapshot_value(palette: &Palette) -> Option<Value> {
    serde_json::to_value(palette).ok()
}

fn set_snapshot(fields: &mut Map<String, Value>, palette: &Palette, id: &str) {
    if let Some(value) = snapshot_value(palette) {
        fields.insert("paletteSnapshot".into(), value);
        fields.insert("paletteId".into(), Value::String(id.into()));
    }
}

pub(crate) fn fixed_palette_selected(fields: &Map<String, Value>) -> bool {
    fields.get("paletteSnapshot").is_some_and(|value| !value.is_null()) || fields.get("paletteId").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
}

fn color32(rgb: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

fn palette_matches(b: &photocraft_algo::palettes::BuiltinPalette, search: &str) -> bool {
    let q = search.trim().to_ascii_lowercase();
    q.is_empty() || b.id.to_ascii_lowercase().contains(&q) || b.palette.name.to_ascii_lowercase().contains(&q) || b.source.to_ascii_lowercase().contains(&q)
}

fn selected_id(fields: &Map<String, Value>) -> Option<&str> {
    fields.get("paletteId").and_then(Value::as_str)
}

fn selected_builtin(fields: &Map<String, Value>, b: &photocraft_algo::palettes::BuiltinPalette) -> bool {
    if selected_id(fields) != Some(b.id) {
        return false;
    }
    fields
        .get("paletteSnapshot")
        .filter(|value| !value.is_null())
        .is_none_or(|value| serde_json::from_value::<Palette>(value.clone()).is_ok_and(|palette| palette == b.palette))
}

/// Filter Palette Map has no generated-palette controls, so give a newly opened dialog an explicit
/// bundled authority rather than sending an empty palette selection to the engine.
pub fn ensure_filter_default(fields: &mut Map<String, Value>) {
    if fixed_palette_selected(fields) {
        return;
    }
    if let Some(builtin) = photocraft_algo::palettes::builtin("photocraft-rgb-cube") {
        set_snapshot(fields, &builtin.palette, builtin.id);
    }
}

/// `text` on one line, cut with an ellipsis past `width`, laid out at `font` in `color`.
pub(crate) fn elided_galley(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let format = egui::TextFormat { font_id: font, color, ..Default::default() };
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), format);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

fn palette_card(ui: &mut Ui, fields: &mut Map<String, Value>, b: &photocraft_algo::palettes::BuiltinPalette, width: f32) {
    let t = Tokens::get(ui.ctx());
    let selected = selected_builtin(fields, b);
    let height = (32.0 + ui.text_style_height(&egui::TextStyle::Body)).ceil();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let count = b.palette.colors.len().to_string();
    let description = crate::i18n::fmt(tl!("{name}  ·  {n} entries"), &[("name", &b.palette.name), ("n", &count)]);
    let response = response.on_hover_text(description);
    let fill = if selected {
        t.accent_soft
    } else if response.hovered() {
        t.hover
    } else {
        t.field
    };
    ui.painter().rect_filled(rect, CornerRadius::same(t.radius_sm as u8), fill);
    if response.has_focus() {
        // Keep keyboard focus visible without turning every palette into a bordered card.
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, t.accent);
    }
    let count = b.palette.colors.len();
    if count > 0 && ui.is_rect_visible(rect) {
        // Fixed-stride sampling aliases repeating tables into grayscale.
        // Show every entry in the palette's own column layout instead.
        let columns = if b.columns == 0 { 16 } else { usize::from(b.columns) }.min(count);
        let cell = vec2((rect.width() - 12.0) / columns as f32, 14.0 / count.div_ceil(columns) as f32);
        // A single mesh avoids anti-aliased seams between subpixel thumbnail cells.
        let mut mesh = egui::Mesh::default();
        mesh.vertices.reserve(count.saturating_mul(4));
        mesh.indices.reserve(count.saturating_mul(6));
        for (index, rgb) in b.palette.colors.iter().enumerate() {
            let offset = vec2((index % columns) as f32 * cell.x, (index / columns) as f32 * cell.y);
            let r = egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0) + offset, cell);
            mesh.add_colored_rect(r, color32(*rgb));
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    }
    let name_font = egui::TextStyle::Body.resolve(ui.style());
    let name_galley = elided_galley(ui, &b.palette.name, name_font, t.text, rect.width() - 14.0);
    ui.painter().galley(rect.left_top() + vec2(6.0, 27.0), name_galley, t.text);
    if selected {
        ui.painter().text(rect.right_top() - vec2(7.0, -5.0), egui::Align2::RIGHT_TOP, "✓", egui::FontId::proportional(13.0), t.accent);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, &b.palette.name));
    let clicked = response.clicked();
    if clicked {
        response.request_focus();
    }
    let key_activate = if response.has_focus() {
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
    } else {
        false
    };
    if (clicked || key_activate) && !selected {
        set_snapshot(fields, &b.palette, b.id);
    }
}

fn browser(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mut search = fields.get(SEARCH_FIELD).and_then(Value::as_str).unwrap_or_default().to_owned();
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(tl!("Palette library")).color(t.text));
        let search_width = (ui.available_width() - 100.0).clamp(110.0, 220.0);
        if ui.add(egui::TextEdit::singleline(&mut search).hint_text(tl!("Search palettes…")).desired_width(search_width)).changed() {
            fields.insert(SEARCH_FIELD.into(), Value::String(search.clone()));
        }
        if ui.small_button(tl!("Import GPL…")).clicked() {
            fields.insert(IMPORT_REQUEST.into(), json!("gpl"));
        }
        if ui.small_button(tl!("Import ACT…")).clicked() {
            fields.insert(IMPORT_REQUEST.into(), json!("act"));
        }
    });
    if let Some(error) = fields.get("__paletteImportError").and_then(Value::as_str) {
        ui.colored_label(t.danger, error);
    }

    let matching: Vec<&photocraft_algo::palettes::BuiltinPalette> =
        photocraft_algo::palettes::builtins().iter().filter(|b| palette_matches(b, &search)).collect();
    let no_match = matching.is_empty();
    let card_width = if ui.available_width() >= 620.0 {
        150.0
    } else if ui.available_width() >= 380.0 {
        140.0
    } else {
        130.0
    };
    // Budget the collection against the viewport, not its unbounded scrolling content. Reserve
    // the chosen table and controls so a library of many presets cannot swallow the whole pane.
    let count = fields.get("paletteSnapshot").and_then(|p| p.get("colors")).and_then(Value::as_array).map_or(256, Vec::len).min(4096);
    let columns = ((ui.available_width() / 19.0).floor() as usize).max(1);
    let palette_height = count.div_ceil(columns) as f32 * 19.0;
    let mapping_reserve = if fields.get("__preview").and_then(Value::as_bool) == Some(false) { 200.0 } else { 0.0 };
    let preceding_controls = ui.cursor().top() - ui.max_rect().top();
    let trailing_controls = ui.spacing().interact_size.y * 2.0 + ui.spacing().item_spacing.y * 4.0 + 4.0;
    let browser_height = (ui.clip_rect().height() - preceding_controls - palette_height - trailing_controls - mapping_reserve).clamp(96.0, 280.0);
    ui.scope(|ui| {
        ui.style_mut().spacing.scroll.floating = false;
        ui.style_mut().spacing.scroll.foreground_color = true;
        egui::ScrollArea::vertical()
            .id_salt("palette-browser-grid")
            .max_height(browser_height)
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for b in &matching {
                        palette_card(ui, fields, b, card_width);
                    }
                    if no_match {
                        ui.label(egui::RichText::new(tl!("No bundled palette matches this search.")).color(t.text_dim));
                    }
                });
            });
    });

    if let Some(palette) = snapshot(fields) {
        let entries = palette.colors.len().to_string();
        let summary = crate::i18n::fmt(tl!("{name}  ·  {n} entries"), &[("name", &palette.name), ("n", &entries)]);
        ui.label(egui::RichText::new(summary).color(t.text));
        selected_grid(ui, &palette);
    } else if fixed_palette_selected(fields) {
        if let Some(value) = fields.get("paletteSnapshot") {
            match serde_json::from_value::<Palette>(value.clone()) {
                Ok(palette) => {
                    if let Err(error) = palette.validate() {
                        let message = crate::i18n::fmt(tl!("Palette invalid: {error}"), &[("error", &error)]);
                        ui.colored_label(t.danger, message);
                    }
                }
                Err(error) => {
                    let message = crate::i18n::fmt(tl!("Palette invalid: {error}"), &[("error", &error.to_string())]);
                    ui.colored_label(t.danger, message);
                }
            }
        } else {
            let error = "no bundled palette matches this selection".to_owned();
            let message = crate::i18n::fmt(tl!("Palette invalid: {error}"), &[("error", &error)]);
            ui.colored_label(t.danger, message);
        }
    }
}

/// Draw a compact, gapless palette grid with RGB/index tooltips.
///
/// Rows are virtualised so imported snapshots at the 4096-entry limit do not allocate or paint
/// off-screen cells.  A grid grows to its natural height when it fits, and otherwise uses a
/// visible-when-needed vertical scrollbar.
pub(crate) fn color_grid(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    count: usize,
    color: impl Fn(usize) -> Option<Color32>,
    transparent: Option<usize>,
    max_height: f32,
) {
    const CELL_SIZE: f32 = 19.0;
    let count = count.min(4096);
    if count == 0 {
        return;
    }
    let max_height = if max_height.is_finite() { max_height.max(CELL_SIZE) } else { 240.0 };
    let t = Tokens::get(ui.ctx());
    ui.scope(|ui| {
        ui.style_mut().spacing.scroll.floating = false;
        ui.style_mut().spacing.scroll.foreground_color = true;
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.spacing_mut().interact_size.y = CELL_SIZE;
        let width = ui.available_width().max(CELL_SIZE);
        let mut columns = ((width / CELL_SIZE).floor() as usize).max(1);
        if count.div_ceil(columns) as f32 * CELL_SIZE > max_height {
            let width = (width - ui.spacing().scroll.allocated_width()).max(CELL_SIZE);
            columns = ((width / CELL_SIZE).floor() as usize).max(1);
        }
        let rows = count.div_ceil(columns);
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(max_height)
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show_rows(ui, CELL_SIZE, rows, |ui, row_range| {
                for row in row_range {
                    ui.horizontal(|ui| {
                        for col in 0..columns {
                            let Some(index) = row.checked_mul(columns).and_then(|n| n.checked_add(col)) else { continue };
                            if index >= count {
                                break;
                            }
                            let (rect, response) = ui.allocate_exact_size(vec2(CELL_SIZE, CELL_SIZE), Sense::hover());
                            let is_transparent = transparent == Some(index);
                            let colour = color(index);
                            if is_transparent {
                                let checker = (CELL_SIZE / 3.0).max(2.0);
                                for y in 0..3 {
                                    for x in 0..3 {
                                        let cell = egui::Rect::from_min_size(rect.min + vec2(x as f32 * checker, y as f32 * checker), vec2(checker, checker))
                                            .intersect(rect);
                                        let fill = if (x + y) % 2 == 0 { t.field } else { t.field_border };
                                        ui.painter().rect_filled(cell, 0.0, fill);
                                    }
                                }
                            } else if let Some(colour) = colour {
                                ui.painter().rect_filled(rect, 0.0, colour);
                            }
                            let tip = colour.map_or_else(
                                || is_transparent.then(|| format!("#{index}")),
                                |colour| {
                                    let index_text = index.to_string();
                                    let red = colour.r().to_string();
                                    let green = colour.g().to_string();
                                    let blue = colour.b().to_string();
                                    Some(crate::i18n::fmt(tl!("#{index}: {r}, {g}, {b}"), &[("index", &index_text), ("r", &red), ("g", &green), ("b", &blue)]))
                                },
                            );
                            if let Some(tip) = tip {
                                response.on_hover_text(tip);
                            }
                        }
                    });
                }
            });
    });
}

fn selected_grid(ui: &mut Ui, palette: &Palette) {
    let max_height = ui.available_height().max(19.0);
    color_grid(ui, "palette-selected-grid", palette.colors.len(), |index| palette.colors.get(index).copied().map(color32), None, max_height);
}

fn parse_mapping(value: &Value) -> Result<Settings, String> {
    let settings: Settings = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    settings.validate().map_err(|error| error.to_string())?;
    Ok(settings)
}

fn mapping(fields: &Map<String, Value>) -> Settings {
    if let Some(v) = fields.get("mapping")
        && let Ok(settings) = parse_mapping(v)
    {
        return settings;
    }
    let mut settings = Settings::default();
    let amount = fields.get("ditherAmount").or_else(|| fields.get("amount")).and_then(Value::as_f64).unwrap_or(75.0);
    settings.amount = (amount as f32 / 100.0).clamp(0.0, 1.0);
    let legacy = fields.get("dither").and_then(Value::as_str).unwrap_or("diffusion");
    settings.method = Method::from_id(legacy)
        .or_else(|| match legacy {
            "diffusion" => Method::from_id("floydSteinberg"),
            "pattern" => Method::from_id("bayer"),
            "noise" => Method::from_id("whiteNoise"),
            _ => None,
        })
        .unwrap_or(Method::FloydSteinberg);
    settings
}

fn write_mapping(fields: &mut Map<String, Value>, settings: &Settings) {
    if let Ok(value) = serde_json::to_value(settings) {
        fields.insert("mapping".into(), value);
    }
}

fn method_options() -> [(&'static str, &'static str); 13] {
    [
        ("none", "None"),
        ("bayer", "Bayer"),
        ("clusteredDot", "Clustered Dot"),
        ("floydSteinberg", "Floyd–Steinberg"),
        ("sierraLite", "Sierra Lite"),
        ("atkinson", "Atkinson"),
        ("jarvis", "Jarvis"),
        ("stucki", "Stucki"),
        ("sierra", "Sierra"),
        ("burkes", "Burkes"),
        ("blueNoise", "Blue Noise"),
        ("whiteNoise", "White Noise"),
        ("riemersma", "Riemersma"),
    ]
}

fn draw_method_controls(ui: &mut Ui, settings: &mut Settings) {
    let t = Tokens::get(ui.ctx());
    let mut method = settings.method.id().to_owned();
    let opts = method_options().map(|(id, label)| (id.to_owned(), label));
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Algorithm")).color(t.text_dim));
        let _ = dropdown(ui, "palette-map-method", tl!("Algorithm"), &mut method, &opts, 180.0);
    });
    if let Some(next) = Method::from_id(&method) {
        settings.method = next;
    }
    let mut amount = settings.amount * 100.0;
    if slider_row(ui, "Strength", &mut amount, 0.0..=100.0, "%", None).changed() {
        settings.amount = (amount / 100.0).clamp(0.0, 1.0);
    }
}

fn draw_method_advanced_controls(ui: &mut Ui, settings: &mut Settings) {
    let t = Tokens::get(ui.ctx());
    match settings.method {
        Method::Bayer | Method::ClusteredDot => {
            let mut n = settings.matrix_size.clamp(2, 16) as f32;
            if slider_row_with(ui, "Matrix", &mut n, 2.0..=16.0, "", None, RowGestures::default()).changed() {
                settings.matrix_size =
                    [2, 4, 8, 16].into_iter().min_by_key(|candidate| (i32::from(*candidate as i16) - i32::from(n.round() as i16)).unsigned_abs()).unwrap_or(4);
            }
            let mut scale = settings.scale.clamp(1, 64) as f32;
            slider_row_with(ui, "Scale", &mut scale, 1.0..=64.0, "px", None, RowGestures::default());
            settings.scale = scale.round().clamp(1.0, 64.0) as u32;
        }
        Method::BlueNoise | Method::WhiteNoise => {
            let mut scale = settings.scale.clamp(1, 64) as f32;
            slider_row_with(ui, "Scale", &mut scale, 1.0..=64.0, "px", None, RowGestures::default());
            settings.scale = scale.round().clamp(1.0, 64.0) as u32;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Seed")).color(t.text_dim));
                ui.add(egui::DragValue::new(&mut settings.seed).range(0..=u32::MAX));
                if ui.button(tl!("Random")).clicked() {
                    settings.seed = ui.input(|i| i.time).to_bits() as u32;
                }
            });
        }
        Method::FloydSteinberg | Method::SierraLite | Method::Atkinson | Method::Jarvis | Method::Stucki | Method::Sierra | Method::Burkes => {
            ui.checkbox(&mut settings.serpentine, tl!("Serpentine traversal"));
        }
        Method::Riemersma => {
            let mut history = settings.history as f32;
            if slider_row_with(ui, "History", &mut history, 1.0..=64.0, "px", None, RowGestures::default()).changed() {
                settings.history = history.round().clamp(1.0, 64.0) as u32;
            }
            let mut decay = settings.decay * 100.0;
            if slider_row_with(ui, "Decay", &mut decay, 0.0..=100.0, "%", None, RowGestures::default()).changed() {
                settings.decay = (decay / 100.0).clamp(0.0, 1.0);
            }
        }
        Method::None => {}
    }

    let mut distance = match settings.distance {
        Distance::Rgb => "rgb",
        Distance::Lab => "lab",
    }
    .to_owned();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Distance")).color(t.text_dim));
        dropdown(ui, "palette-map-distance", tl!("Distance"), &mut distance, &[("rgb".into(), "RGB"), ("lab".into(), "Lab")], 120.0);
    });
    settings.distance = if distance == "lab" { Distance::Lab } else { Distance::Rgb };
}

fn draw_alpha_controls(ui: &mut Ui, settings: &mut Settings) {
    let t = Tokens::get(ui.ctx());
    let mut alpha = match settings.alpha {
        AlphaMode::Preserve => "preserve",
        AlphaMode::Binary => "binary",
        AlphaMode::Dither => "dither",
    }
    .to_owned();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Transparency")).color(t.text_dim));
        dropdown(
            ui,
            "palette-map-alpha",
            tl!("Transparency"),
            &mut alpha,
            &[("preserve".into(), "Preserve"), ("binary".into(), "Binary"), ("dither".into(), "Dither")],
            120.0,
        );
    });
    settings.alpha = match alpha.as_str() {
        "binary" => AlphaMode::Binary,
        "dither" => AlphaMode::Dither,
        _ => AlphaMode::Preserve,
    };
}

pub(crate) fn mapping_controls(ui: &mut Ui, fields: &mut Map<String, Value>) {
    ui.scope(|ui| {
        let t = Tokens::get(ui.ctx());
        if let Some(value) = fields.get("mapping")
            && let Err(error) = parse_mapping(value)
        {
            let message = crate::i18n::fmt(tl!("Mapping settings are invalid: {error}"), &[("error", &error)]);
            ui.colored_label(t.danger, message);
            return;
        }
        let mut settings = mapping(fields);
        let before = settings.clone();
        ui.label(egui::RichText::new(tl!("Dithering")).strong().color(t.text));
        draw_method_controls(ui, &mut settings);
        ui.collapsing(tl!("Advanced"), |ui| {
            draw_method_advanced_controls(ui, &mut settings);
            draw_alpha_controls(ui, &mut settings);
            ui.add_space(3.0);
            crate::dialogs::palette_pattern_ui::show(ui, fields, &mut settings.patterns);
        });
        if settings != before {
            write_mapping(fields, &settings);
        }
    });
}

pub(crate) fn palette_controls(ui: &mut Ui, fields: &mut Map<String, Value>) {
    ui.scope(|ui| {
        browser(ui, fields);
        validation(ui, fields);
    });
}

fn validation(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(palette) = snapshot(fields) else { return };
    if let Err(error) = palette.validate() {
        let message = crate::i18n::fmt(tl!("Palette invalid: {error}"), &[("error", &error)]);
        ui.colored_label(t.danger, message);
    }
}

/// Main Palette Map controls.
pub fn body(ui: &mut Ui, fields: &mut Map<String, Value>) {
    palette_controls(ui, fields);
    mapping_controls(ui, fields);
}

/// Whether the Palette Map fields are valid before they reach the engine.
pub fn can_confirm(fields: &Map<String, Value>) -> bool {
    if fields.get("mapping").is_some_and(|value| parse_mapping(value).is_err()) {
        return false;
    }
    if let Some(value) = fields.get("paletteSnapshot")
        && !value.is_null()
    {
        let Ok(palette) = serde_json::from_value::<Palette>(value.clone()) else {
            return false;
        };
        if palette.validate().is_err() {
            return false;
        }
    }
    if fixed_palette_selected(fields) && snapshot(fields).is_none() {
        return false;
    }
    true
}

fn parse_matrix(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| tl!("threshold matrix must be UTF-8 text").to_string())?;
    let mut values = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';')) {
        if token.is_empty() {
            continue;
        }
        let value = token.parse::<f32>().map_err(|_| crate::i18n::fmt(tl!("invalid threshold value `{token}`"), &[("token", token)]))?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(tl!("threshold values must be finite and between 0 and 1").into());
        }
        if values.len() >= 256 {
            return Err(tl!("threshold matrices are limited to 256 entries").into());
        }
        values.push(value);
    }
    let side = (values.len() as f64).sqrt() as usize;
    if side.saturating_mul(side) != values.len() || !matches!(side, 2 | 4 | 8 | 16) {
        return Err(tl!("threshold matrix must be a square of 2, 4, 8, or 16 entries per side").into());
    }
    Ok((values, side as u32))
}

/// One Palette Map dialog's values as the JSON map its controls edit.
fn open_dialog_values(app: &PhotocraftApp, id: DialogId) -> Option<Map<String, Value>> {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|dialog| dialog_values(dialog.fields()))
}

/// Writes the palette values back into the open dialog `id`; a dialog closed under the picker is left alone.
fn write_dialog_values(app: &mut PhotocraftApp, id: DialogId, values: &Map<String, Value>) {
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return };
    for key in OWNED_KEYS {
        match values.get(key) {
            Some(Value::String(text)) => dialog.set_field(key, FieldValue::Text(text.clone())),
            Some(value) => dialog.set_field(key, FieldValue::Json(value.clone())),
            None => dialog.clear_field(key),
        }
    }
}

/// Reports a failed import under the palette browser and in the status bar.
fn import_error(app: &mut PhotocraftApp, id: DialogId, error: String) {
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
        dialog.set_field("__paletteImportError", FieldValue::Text(error.clone()));
        dialog.clear_field(IMPORT_REQUEST);
        dialog.clear_field("__paletteMatrixImport");
    }
    app.ui.status = error;
    app.ui.status_error = true;
}

/// Answers the palette browser's Import buttons and threshold-matrix Import requests: a per-frame pass
/// over the open Palette Map dialogs, as the rows themselves cannot open a file picker.
pub(crate) fn poll(app: &mut PhotocraftApp) {
    let ids: Vec<DialogId> = app
        .framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|open| (open.key.as_deref() == Some(FILTER_COMMAND)).then_some(open.id))
        .collect();
    for id in ids {
        let Some(values) = open_dialog_values(app, id) else { continue };
        if let Some(index) = values.get("__paletteMatrixImport").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) {
            start_matrix_import(app, id, index);
        } else if let Some(kind) = values.get(IMPORT_REQUEST).and_then(Value::as_str) {
            start_palette_import(app, id, kind.to_owned());
        }
    }
}

/// A palette import (`.gpl` or `.act`) replaces the dialog's embedded snapshot.
fn start_palette_import(app: &mut PhotocraftApp, id: DialogId, kind: String) {
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
        dialog.clear_field(IMPORT_REQUEST);
    }
    let extension = if kind == "act" { "act" } else { "gpl" };
    let result = app.pick_file_bytes_filtered(&[extension], move |app, name, bytes| {
        let palette = if kind == "act" { photocraft_algo::palettes::parse_act(&bytes) } else { photocraft_algo::palettes::parse_gpl(&bytes) }
            .map_err(|error| format!("{}: {error}", crate::file_open::display_name(&name)))?;
        palette.validate().map_err(|error| format!("{}: {error}", crate::file_open::display_name(&name)))?;
        let mut values = open_dialog_values(app, id).ok_or_else(|| tl!("palette dialog was closed").to_string())?;
        set_snapshot(&mut values, &palette, CUSTOM_ID);
        values.insert(SEARCH_FIELD.into(), Value::String(String::new()));
        write_dialog_values(app, id, &values);
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
            dialog.clear_field("__paletteImportError");
            dialog.set_field("__paletteImportName", FieldValue::Text(name));
        }
        Ok(json!({"palette": palette.name, "entries": palette.colors.len()}))
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        import_error(app, id, error);
    }
}

/// A threshold matrix import fills the selected threshold ingredient's matrix.
fn start_matrix_import(app: &mut PhotocraftApp, id: DialogId, index: usize) {
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
        dialog.clear_field("__paletteMatrixImport");
    }
    let result = app.pick_file_bytes_filtered(&["txt", "csv"], move |app, name, bytes| {
        let (matrix, side) = parse_matrix(&bytes).map_err(|error| format!("{}: {error}", crate::file_open::display_name(&name)))?;
        let mut values = open_dialog_values(app, id).ok_or_else(|| tl!("palette dialog was closed").to_string())?;
        let mut settings = values.get("mapping").map(parse_mapping).transpose()?.unwrap_or_default();
        let Some(pattern) = settings.patterns.get_mut(index) else { return Err(tl!("threshold ingredient no longer exists").into()) };
        pattern.matrix_size = side;
        pattern.matrix = matrix;
        values.insert("mapping".into(), serde_json::to_value(settings).map_err(|error| error.to_string())?);
        write_dialog_values(app, id, &values);
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
            dialog.clear_field("__paletteImportError");
            dialog.set_field("__paletteMatrixImportName", FieldValue::Text(name));
        }
        Ok(Value::Null)
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        import_error(app, id, error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_threshold_matrix_obeys_custom_pattern_bounds() {
        use photocraft_algo::palette_map::{Pattern, PatternKind};
        let (values, side) = parse_matrix(b"0, 0.25; 0.75, 1").unwrap();
        assert_eq!((values.clone(), side), (vec![0.0, 0.25, 0.75, 1.0], 2));
        let settings =
            Settings { patterns: vec![Pattern { kind: PatternKind::Custom, matrix: values, matrix_size: side, ..Pattern::default() }], ..Settings::default() };
        settings.validate().unwrap();
        for invalid in ["-0.01, 0; 0.5, 1", "1.01, 0; 0.5, 1", "NaN, 0; 0.5, 1", "inf, 0; 0.5, 1"] {
            assert!(parse_matrix(invalid.as_bytes()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn invalid_matrix_import_preserves_a_confirmable_recipe() {
        use photocraft_algo::palette_map::{Pattern, PatternKind};
        let files = std::sync::Arc::new(craft::host::ScriptedFiles::answering([craft::host::Picked::Contents(vec![(
            "invalid.csv".into(),
            b"-0.01, 0; 0.5, 1".to_vec(),
        )])]));
        let services = crate::Services { files: Some(files), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let revision = app.session.active().unwrap().revision;
        let settings = Settings {
            patterns: vec![Pattern { kind: PatternKind::Custom, matrix: vec![0.0, 0.25, 0.75, 1.0], matrix_size: 2, ..Pattern::default() }],
            ..Settings::default()
        };
        let recipe = serde_json::to_value(settings).unwrap();
        let id = crate::filter_dialog::open(&mut app, FILTER_COMMAND).expect("Palette Map dialog");
        let dialog = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id).expect("the palette dialog");
        dialog.set_field("mapping", FieldValue::Json(recipe.clone()));
        dialog.set_field("__paletteMatrixImport", FieldValue::Json(json!(0)));
        poll(&mut app);
        app.poll_file_dialog();
        let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|dialog| dialog_values(dialog.fields())).unwrap_or_default();
        assert_eq!(fields.get("mapping"), Some(&recipe));
        assert!(can_confirm(&fields));
        assert!(app.ui.status_error);
        assert_eq!(app.session.active().unwrap().revision, revision);
    }

    #[test]
    fn rgb_cube_library_thumbnail_shows_chromatic_colors() {
        let cube = photocraft_algo::palettes::builtin("photocraft-rgb-cube").expect("bundled PhotoCraft RGB cube palette");
        let ctx = egui::Context::default();
        let mut fields = Map::new();
        let mut strip = egui::Rect::NOTHING;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            strip = egui::Rect::from_min_size(ui.cursor().min + vec2(6.0, 6.0), vec2(128.0, 14.0));
            palette_card(ui, &mut fields, cube, 140.0);
        });
        frame.textures_delta.clear();
        let primitives = ctx.tessellate(frame.shapes, frame.pixels_per_point);
        assert!(
            primitives.iter().any(|clipped| {
                let egui::epaint::Primitive::Mesh(mesh) = &clipped.primitive else { return false };
                mesh.vertices.iter().any(|vertex| {
                    let rgb = vertex.color;
                    strip.contains(vertex.pos) && vertex.uv == egui::epaint::WHITE_UV && (rgb.r() != rgb.g() || rgb.g() != rgb.b())
                })
            }),
            "PhotoCraft RGB cube's library thumbnail must not alias its colorful table into a grayscale strip"
        );
    }

    #[test]
    fn legacy_mapping_fields_are_migrated() {
        let mut fields = Map::new();
        fields.insert("dither".into(), json!("pattern"));
        fields.insert("amount".into(), json!(50));

        let s = mapping(&fields);
        assert_eq!(s.method, Method::Bayer);
        assert!((s.amount - 0.5).abs() < 1e-6);
    }

    #[test]
    fn remembered_palette_values_must_still_validate() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // A stale snapshot, a malformed mapping and a custom id without a snapshot are all dropped.
        app.session.edit_prefs(|prefs| {
            prefs.dialogs.insert(
                FILTER_COMMAND.into(),
                json!({
                    "paletteId": "custom",
                    "paletteSnapshot": {"name": "broken", "colors": []},
                    "mapping": {"method": "bogus"},
                }),
            );
        });
        let opening = opening_fields(&app, FILTER_COMMAND);
        let bundled = photocraft_algo::palettes::builtin("photocraft-rgb-cube").expect("bundled default");
        assert_eq!(opening.get("paletteId").and_then(Value::as_str), Some(bundled.id), "an invalid snapshot falls back to the default");
        assert!(opening.get("mapping").is_none(), "a malformed mapping is not restored");
        let expected = serde_json::to_value(&bundled.palette).expect("the default palette serializes");
        assert_eq!(opening.get("paletteSnapshot"), Some(&expected));

        // A valid remembered choice is kept.
        app.session.edit_prefs(|prefs| {
            prefs.dialogs.insert(
                FILTER_COMMAND.into(),
                json!({
                    "paletteId": "custom",
                    "paletteSnapshot": {"name": "two", "colors": [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]},
                    "mapping": {"method": "bayer", "matrixSize": 4, "amount": 0.5},
                }),
            );
        });
        let opening = opening_fields(&app, FILTER_COMMAND);
        assert_eq!(opening.get("paletteId").and_then(Value::as_str), Some(CUSTOM_ID));
        assert_eq!(opening.get("mapping").and_then(|m| m.get("method")).and_then(Value::as_str), Some("bayer"));
    }

    #[test]
    fn cancel_discards_palette_draft_without_document_mutation() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "background": "#223344"})).unwrap();
        let revision = app.session.active().map(|state| state.revision);
        let id = crate::filter_dialog::open(&mut app, FILTER_COMMAND).expect("Palette Map dialog");
        let palette = Palette { name: "draft".into(), colors: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] };
        let mut values = open_dialog_values(&app, id).expect("palette dialog");
        set_snapshot(&mut values, &palette, CUSTOM_ID);
        write_dialog_values(&mut app, id, &values);
        crate::dialogs::cancel(&mut app, &egui::Context::default(), id).unwrap();
        assert_eq!(app.session.active().map(|state| state.revision), revision);
        assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none());
    }
}
