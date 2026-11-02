//! Compact threshold-ingredient editor for Palette Map.
//!
//! The list is the composition: each ingredient keeps its own enabled flag and the selected
//! ingredient is the only one whose editor is expanded.  Preview pixels are rendered by the
//! palette-map kernel, so diagnostics cannot drift from the command's actual behaviour.

use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use egui::{Color32, CornerRadius, Sense, TextureHandle, Ui, pos2, vec2};
use photocraft_algo::palette_map::{Method, Palette, Pattern, PatternKind, Settings, render_pixels};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;

const MAX_PATTERNS: usize = 16;
const PREVIEW_SLOTS: usize = MAX_PATTERNS * 2;
const SELECTION_FIELD: &str = "__palettePatternSelection";
const IMPORT_FIELD: &str = "__paletteMatrixImport";
const THUMB_SIDE: usize = 32;
const RAMP_SIZE: [usize; 2] = [128, 48];

const KINDS: [(&str, &str); 5] =
    [("bayer", "Bayer"), ("clusteredDot", "Clustered Dot"), ("blueNoise", "Blue Noise"), ("whiteNoise", "White Noise"), ("custom", "Custom")];

#[derive(Clone, Default)]
struct PreviewCache {
    slots: HashMap<egui::Id, (u64, Result<TextureHandle, String>)>,
}

impl PreviewCache {
    fn get(&mut self, slot: egui::Id, signature: u64, render: impl FnOnce() -> Result<TextureHandle, String>) -> Result<TextureHandle, String> {
        if let Some((cached_signature, value)) = self.slots.get(&slot)
            && *cached_signature == signature
        {
            return value.clone();
        }
        if self.slots.len() >= PREVIEW_SLOTS
            && !self.slots.contains_key(&slot)
            && let Some(evicted) = self.slots.keys().next().copied()
        {
            self.slots.remove(&evicted);
        }
        let value = render();
        self.slots.insert(slot, (signature, value.clone()));
        value
    }
}

fn with_preview_cache<R>(ctx: &egui::Context, f: impl FnOnce(&mut PreviewCache) -> R) -> R {
    let cache_id = egui::Id::unique("palette-pattern-preview-cache");
    let mut cache = ctx.data_mut(|data| std::mem::take(data.get_temp_mut_or_default::<PreviewCache>(cache_id)));
    let result = f(&mut cache);
    ctx.data_mut(|data| data.insert_temp(cache_id, cache));
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn pattern_kind_id(kind: PatternKind) -> &'static str {
    match kind {
        PatternKind::Bayer => "bayer",
        PatternKind::ClusteredDot => "clusteredDot",
        PatternKind::BlueNoise => "blueNoise",
        PatternKind::WhiteNoise => "whiteNoise",
        PatternKind::Custom => "custom",
    }
}

fn pattern_kind_from_id(id: &str) -> PatternKind {
    match id {
        "clusteredDot" => PatternKind::ClusteredDot,
        "blueNoise" => PatternKind::BlueNoise,
        "whiteNoise" => PatternKind::WhiteNoise,
        "custom" => PatternKind::Custom,
        _ => PatternKind::Bayer,
    }
}

fn pattern_kind_label(kind: PatternKind) -> &'static str {
    match kind {
        PatternKind::Bayer => "Bayer",
        PatternKind::ClusteredDot => "Clustered Dot",
        PatternKind::BlueNoise => "Blue Noise",
        PatternKind::WhiteNoise => "White Noise",
        PatternKind::Custom => "Custom",
    }
}

fn matrix_side(size: u32) -> Option<usize> {
    matches!(size, 2 | 4 | 8 | 16).then_some(size as usize)
}

fn matrix_cell_count(size: u32) -> Option<usize> {
    matrix_side(size).and_then(|side| side.checked_mul(side))
}

fn initialize_custom_matrix(pattern: &mut Pattern) {
    let Some(count) = matrix_cell_count(pattern.matrix_size) else { return };
    pattern.matrix.resize(count, 0.5);
}

fn reset_custom_matrix(pattern: &mut Pattern) {
    let Some(count) = matrix_cell_count(pattern.matrix_size) else { return };
    pattern.matrix = vec![0.5; count];
}

fn selected_index(fields: &Map<String, Value>, len: usize) -> Option<usize> {
    fields.get(SELECTION_FIELD).and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()).filter(|index| *index < len)
}

fn set_selected_index(fields: &mut Map<String, Value>, index: Option<usize>) {
    if let Some(index) = index {
        fields.insert(SELECTION_FIELD.into(), json!(index));
    } else {
        fields.remove(SELECTION_FIELD);
    }
}

fn add_pattern(fields: &mut Map<String, Value>, patterns: &mut Vec<Pattern>, kind: PatternKind) -> Option<usize> {
    if patterns.len() >= MAX_PATTERNS {
        return None;
    }
    let mut pattern = Pattern { kind, ..Pattern::default() };
    if kind == PatternKind::Custom {
        initialize_custom_matrix(&mut pattern);
    }
    patterns.push(pattern);
    let index = patterns.len().saturating_sub(1);
    set_selected_index(fields, Some(index));
    Some(index)
}

fn move_selected(patterns: &mut [Pattern], selected: usize, direction: i32) -> Option<usize> {
    let target = match direction {
        -1 if selected > 0 => selected - 1,
        1 if selected + 1 < patterns.len() => selected + 1,
        _ => return None,
    };
    patterns.swap(selected, target);
    Some(target)
}

fn remove_selected(patterns: &mut Vec<Pattern>, selected: usize) -> Option<usize> {
    if selected >= patterns.len() {
        return None;
    }
    patterns.remove(selected);
    if patterns.is_empty() { None } else { Some(selected.saturating_sub(1).min(patterns.len().saturating_sub(1))) }
}

fn remap_import_after_move(fields: &mut Map<String, Value>, from: usize, to: usize) {
    let Some(index) = fields.get(IMPORT_FIELD).and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()) else {
        return;
    };
    let mapped = if index == from {
        to
    } else if index == to {
        from
    } else {
        index
    };
    fields.insert(IMPORT_FIELD.into(), json!(mapped));
}

fn remap_import_after_remove(fields: &mut Map<String, Value>, removed: usize) {
    let Some(index) = fields.get(IMPORT_FIELD).and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()) else {
        return;
    };
    if index == removed {
        fields.remove(IMPORT_FIELD);
    } else if index > removed {
        fields.insert(IMPORT_FIELD.into(), json!(index - 1));
    }
}

fn pattern_signature(pattern: &Pattern) -> u64 {
    let mut hasher = DefaultHasher::new();
    pattern_kind_id(pattern.kind).hash(&mut hasher);
    pattern.amount.to_bits().hash(&mut hasher);
    pattern.scale.hash(&mut hasher);
    pattern.matrix_size.hash(&mut hasher);
    pattern.phase.hash(&mut hasher);
    pattern.seed.hash(&mut hasher);
    pattern.matrix.len().hash(&mut hasher);
    for value in &pattern.matrix {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn diagnostic_settings(pattern: &Pattern) -> Settings {
    let mut diagnostic = pattern.clone();
    // Validate disabled and zero-strength custom ingredients as previews too.  This is only a
    // diagnostic copy; the stored recipe is never normalized while it is being displayed.
    diagnostic.enabled = true;
    if diagnostic.kind == PatternKind::Custom && diagnostic.amount == 0.0 {
        diagnostic.amount = 1.0;
    }
    Settings { method: Method::None, amount: 0.0, patterns: vec![diagnostic], ..Settings::default() }
}

fn preview_image(pattern: &Pattern, size: [usize; 2], ramp: bool) -> Result<egui::ColorImage, String> {
    let [width, height] = size;
    let mut settings = diagnostic_settings(pattern);
    settings.validate()?;
    if let Some(preview_pattern) = settings.patterns.first_mut() {
        preview_pattern.amount = pattern.amount;
    }
    if width == 0 || height == 0 {
        return Err("Preview dimensions must be non-zero".to_owned());
    }
    let count = width.checked_mul(height).ok_or_else(|| "Preview dimensions are too large".to_owned())?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|_| "Could not allocate pattern preview".to_owned())?;
    for _ in 0..height {
        for x in 0..width {
            let value = if ramp { if width == 1 { 0.5 } else { x as f32 / (width - 1) as f32 } } else { 0.5 };
            pixels.push([value, value, value, 1.0]);
        }
    }

    let palette = Palette { name: "Pattern preview".into(), colors: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] };
    render_pixels(&mut pixels, width, &palette, &settings, [0, 0], None)?;
    let rgba = pixels
        .into_iter()
        .map(|pixel| {
            let q = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Color32::from_rgba_unmultiplied(q(pixel[0]), q(pixel[1]), q(pixel[2]), q(pixel[3]))
        })
        .collect();
    Ok(egui::ColorImage::new(size, rgba))
}

fn preview_texture(ui: &Ui, pattern: &Pattern, ramp: bool, slot: egui::Id) -> Result<TextureHandle, String> {
    let size = if ramp { RAMP_SIZE } else { [THUMB_SIDE, THUMB_SIDE] };
    let mut hasher = DefaultHasher::new();
    (pattern_signature(pattern), ramp, size).hash(&mut hasher);
    let signature = hasher.finish();
    with_preview_cache(ui.ctx(), |cache| {
        cache.get(slot, signature, || {
            let image = preview_image(pattern, size, ramp)?;
            Ok(ui.ctx().load_texture(format!("palette-pattern-{signature}-{ramp}"), image, egui::TextureOptions::NEAREST))
        })
    })
}

fn pattern_summary(pattern: &Pattern, unavailable: bool) -> String {
    let amount = if pattern.amount.is_finite() { format!("{:.0}%", pattern.amount.clamp(0.0, 1.0) * 100.0) } else { "?".to_owned() };
    let mut summary = if matches!(pattern.kind, PatternKind::Bayer | PatternKind::ClusteredDot | PatternKind::Custom) {
        format!("{}×{}  ·  {} px  ·  {amount}", pattern.matrix_size, pattern.matrix_size, pattern.scale)
    } else {
        format!("{} px  ·  {amount}", pattern.scale)
    };
    if unavailable {
        summary.push_str("  ·  !");
    }
    summary
}

fn pattern_row_height(ui: &Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Body) + ui.text_style_height(&egui::TextStyle::Small) + 13.0).max(42.0)
}

fn draw_pattern_row(ui: &mut Ui, fields: &mut Map<String, Value>, patterns: &mut [Pattern], index: usize, selected: &mut usize) {
    let Some(pattern) = patterns.get_mut(index) else { return };
    let t = Tokens::get(ui.ctx());
    let title_font = egui::TextStyle::Body.resolve(ui.style());
    let detail_font = egui::TextStyle::Small.resolve(ui.style());
    let title_height = ui.text_style_height(&egui::TextStyle::Body);
    let text_height = title_height + ui.text_style_height(&egui::TextStyle::Small) + 3.0;
    let row_height = pattern_row_height(ui);
    let available_width = ui.available_width();
    let row_width = if available_width.is_finite() { available_width.max(120.0) } else { 300.0 };
    let (_, rect) = ui.allocate_space(vec2(row_width, row_height));
    let response = ui.interact(rect, ui.scope_id().with(("palette-pattern-row", index)), Sense::click());
    let thumbnail = preview_texture(ui, pattern, false, egui::Id::unique(("palette-pattern-thumbnail", index)));
    let summary = pattern_summary(pattern, thumbnail.is_err());
    let number = (index + 1).to_string();
    let title = format!("{} {number}", crate::i18n::t(pattern_kind_label(pattern.kind)));
    let accessible_title = crate::i18n::fmt(tl!("Ingredient {n}"), &[("n", &number)]);
    let response = response.on_hover_text(format!("{accessible_title} · {summary}"));
    let fill = if *selected == index {
        t.accent_soft
    } else if response.hovered() {
        t.hover
    } else {
        t.field
    };
    ui.painter().rect_filled(rect, CornerRadius::same(t.radius_sm as u8), fill);
    if *selected == index {
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, t.accent);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(t.radius_sm as u8), egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }

    let thumb_y = rect.top() + (rect.height() - 32.0) * 0.5;
    let thumb_rect = egui::Rect::from_min_size(pos2(rect.left() + 30.0, thumb_y), vec2(32.0, 32.0));
    match thumbnail {
        Ok(texture) => {
            let tint = if pattern.enabled { Color32::WHITE } else { Color32::from_white_alpha(105) };
            ui.painter().image(texture.id(), thumb_rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        Err(_) => {
            ui.painter().rect_filled(thumb_rect, CornerRadius::same(t.radius_sm as u8), t.field);
            ui.painter().text(thumb_rect.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(15.0), t.danger);
        }
    }

    let mut enabled = pattern.enabled;
    let checkbox_y = rect.top() + (rect.height() - 20.0) * 0.5;
    let checkbox_rect = egui::Rect::from_min_size(pos2(rect.left() + 5.0, checkbox_y), vec2(20.0, 20.0));
    let checkbox = ui.put(checkbox_rect, egui::Checkbox::new(&mut enabled, ""));
    checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), enabled, tl!("Enabled")));
    if checkbox.changed() {
        if enabled && pattern.kind == PatternKind::Custom {
            initialize_custom_matrix(pattern);
        }
        pattern.enabled = enabled;
    }

    let text_left = thumb_rect.right() + 8.0;
    let title_galley = crate::tab_strip::elided(ui, &title, title_font, t.text, (rect.right() - text_left - 6.0).max(20.0));
    let detail_galley = crate::tab_strip::elided(ui, &summary, detail_font, t.text_dim, (rect.right() - text_left - 6.0).max(20.0));
    let text_top = rect.top() + (rect.height() - text_height) * 0.5;
    ui.painter().galley(pos2(text_left, text_top), title_galley, t.text);
    ui.painter().galley(pos2(text_left, text_top + title_height + 3.0), detail_galley, t.text_dim);

    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), *selected == index, title.clone()));
    if response.has_focus() {
        // The list owns vertical navigation; egui must not also focus a row's toggle.
        ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    let key_select = if response.has_focus() {
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
    } else {
        false
    };
    let previous = if response.has_focus() { ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) } else { false };
    let next = if response.has_focus() { ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) } else { false };
    if response.clicked() || key_select {
        response.request_focus();
        *selected = index;
        set_selected_index(fields, Some(index));
    } else if previous && index > 0 {
        let target = index - 1;
        *selected = target;
        set_selected_index(fields, Some(target));
        ui.memory_mut(|memory| memory.request_focus(ui.scope_id().with(("palette-pattern-row", target))));
    } else if next && index + 1 < patterns.len() {
        let target = index + 1;
        *selected = target;
        set_selected_index(fields, Some(target));
        ui.memory_mut(|memory| memory.request_focus(ui.scope_id().with(("palette-pattern-row", target))));
    }
}

fn draw_pattern_list(ui: &mut Ui, fields: &mut Map<String, Value>, patterns: &mut [Pattern], selected: &mut usize) {
    let height = (patterns.len() as f32 * (pattern_row_height(ui) + ui.spacing().item_spacing.y + 3.0)).min(280.0);
    egui::ScrollArea::vertical()
        .id_salt("palette-pattern-list")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            for index in 0..patterns.len() {
                draw_pattern_row(ui, fields, patterns, index, selected);
                ui.add_space(3.0);
            }
        });
}

fn draw_pattern_actions(ui: &mut Ui, selected: usize, patterns_len: usize) -> Option<ListAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        let up = ui.add_enabled_ui(selected > 0, |ui| crate::icons::button(ui, "chevron-up", 24.0, false, tl!("Move up"))).inner;
        if up.clicked() {
            action = Some(ListAction::MoveUp);
        }
        let down = ui.add_enabled_ui(selected + 1 < patterns_len, |ui| crate::icons::button(ui, "chevron-down", 24.0, false, tl!("Move down"))).inner;
        if down.clicked() {
            action = Some(ListAction::MoveDown);
        }
        let remove = ui.button(tl!("Remove")).on_hover_text(tl!("Remove"));
        remove.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Remove")));
        if remove.clicked() {
            action = Some(ListAction::Remove);
        }
    });
    action
}

fn apply_list_action(fields: &mut Map<String, Value>, patterns: &mut Vec<Pattern>, selected: &mut usize, action: ListAction) {
    match action {
        ListAction::MoveUp => {
            if let Some(index) = move_selected(patterns, *selected, -1) {
                remap_import_after_move(fields, *selected, index);
                *selected = index;
                set_selected_index(fields, Some(index));
            }
        }
        ListAction::MoveDown => {
            if let Some(index) = move_selected(patterns, *selected, 1) {
                remap_import_after_move(fields, *selected, index);
                *selected = index;
                set_selected_index(fields, Some(index));
            }
        }
        ListAction::Remove => {
            let removed = *selected;
            let next = remove_selected(patterns, removed);
            remap_import_after_remove(fields, removed);
            *selected = next.unwrap_or(0);
            set_selected_index(fields, next);
        }
    }
}

fn draw_pattern_editor(ui: &mut Ui, fields: &mut Map<String, Value>, patterns: &mut [Pattern], selected: &mut usize) {
    let Some(pattern) = patterns.get_mut(*selected) else { return };
    let t = Tokens::get(ui.ctx());
    let selected_index = *selected;
    let number = (selected_index + 1).to_string();
    let title = format!("{} {number}", crate::i18n::t(pattern_kind_label(pattern.kind)));
    ui.label(egui::RichText::new(title).strong().color(t.text));

    let preview = preview_texture(ui, pattern, true, egui::Id::unique(("palette-pattern-ramp", selected_index)));
    match &preview {
        Ok(texture) => {
            let width = ui.available_width().clamp(120.0, RAMP_SIZE[0] as f32);
            let height = width * RAMP_SIZE[1] as f32 / RAMP_SIZE[0] as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        Err(error) => {
            ui.colored_label(t.danger, format!("{}: {error}", tl!("Preview")));
        }
    };
    let preview_error = preview.as_ref().err().cloned();

    let old_kind = pattern.kind;
    let mut kind = pattern_kind_id(pattern.kind).to_owned();
    let options = KINDS.iter().map(|(id, label)| ((*id).to_owned(), *label)).collect::<Vec<_>>();
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(tl!("Pattern")).color(t.text_dim));
        let _ = crate::widgets::dropdown(ui, "palette-pattern-kind", tl!("Pattern"), &mut kind, &options, 160.0);
    });
    let kind_changed = pattern_kind_from_id(&kind) != old_kind;
    if kind_changed {
        pattern.kind = pattern_kind_from_id(&kind);
    }

    let amount_before = pattern.amount;
    let mut amount = if amount_before.is_finite() { amount_before * 100.0 } else { 0.0 };
    if crate::widgets::slider_row(ui, "Strength", &mut amount, 0.0..=100.0, "%", None).changed() {
        pattern.amount = (amount / 100.0).clamp(0.0, 1.0);
    }

    let scale_before = pattern.scale;
    let mut scale = scale_before.clamp(1, 64) as f32;
    if crate::widgets::slider_row(ui, "Scale", &mut scale, 1.0..=64.0, "px", None).changed() {
        pattern.scale = scale.round().clamp(1.0, 64.0) as u32;
    }

    let mut matrix_size_changed = false;
    if matches!(pattern.kind, PatternKind::Bayer | PatternKind::ClusteredDot | PatternKind::Custom) {
        let old_matrix_size = pattern.matrix_size;
        let mut matrix_size = old_matrix_size.to_string();
        let matrix_options = [2_u32, 4, 8, 16].map(|size| {
            let label = match size {
                2 => "2 × 2",
                4 => "4 × 4",
                8 => "8 × 8",
                _ => "16 × 16",
            };
            (size.to_string(), label)
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tl!("Matrix")).color(t.text_dim));
            if crate::widgets::dropdown(ui, "palette-pattern-matrix-size", tl!("Matrix"), &mut matrix_size, &matrix_options, 110.0)
                && let Ok(next) = matrix_size.parse::<u32>()
            {
                matrix_size_changed = next != old_matrix_size;
                pattern.matrix_size = next;
            }
        });
    }

    let phase_before = pattern.phase;
    let mut phase = phase_before;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Phase")).color(t.text_dim));
        ui.add(egui::DragValue::new(&mut phase[0]).prefix("x "));
        ui.add(egui::DragValue::new(&mut phase[1]).prefix("y "));
    });
    if phase != phase_before {
        pattern.phase = phase;
    }
    if matches!(pattern.kind, PatternKind::BlueNoise | PatternKind::WhiteNoise) {
        let seed_before = pattern.seed;
        let mut seed = seed_before;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Seed")).color(t.text_dim));
            ui.add(egui::DragValue::new(&mut seed).range(0..=u32::MAX));
        });
        if seed != seed_before {
            pattern.seed = seed;
        }
    }

    if pattern.kind == PatternKind::Custom {
        // Custom matrices are initialized only by an explicit kind/size edit or the Reset
        // control below; merely opening a disabled legacy ingredient never mutates the recipe.
        if kind_changed {
            reset_custom_matrix(pattern);
        } else if matrix_size_changed {
            initialize_custom_matrix(pattern);
        }
        ui.label(egui::RichText::new(tl!("Custom threshold matrix (0–1)")).color(t.text_dim));
        let valid_size = matrix_side(pattern.matrix_size);
        let expected = matrix_cell_count(pattern.matrix_size);
        if preview_error.is_none()
            && let (Some(side), Some(expected)) = (valid_size, expected)
            && pattern.matrix.len() == expected
        {
            egui::ScrollArea::horizontal().id_salt("palette-pattern-matrix-scroll").auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("palette-pattern-matrix").spacing([3.0, 3.0]).show(ui, |ui| {
                    for cell in 0..expected {
                        let Some(value) = pattern.matrix.get_mut(cell) else { continue };
                        let mut next = *value;
                        if crate::widgets::value_field(ui, &mut next, 0.0..=1.0, "", 42.0).changed() {
                            *value = next;
                        }
                        if (cell + 1) % side == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
        } else {
            let error = preview_error.unwrap_or_else(|| format!("{}: invalid matrix", tl!("Custom threshold matrix (0–1)")));
            ui.colored_label(t.danger, error);
            if ui.button(tl!("Reset")).clicked() {
                reset_custom_matrix(pattern);
            }
        }
        if ui.button(tl!("Import matrix…")).clicked() {
            fields.insert(IMPORT_FIELD.into(), json!(*selected));
        }
        if let Some(error) = fields.get("__paletteImportError").and_then(Value::as_str) {
            ui.colored_label(t.danger, error);
        }
    }
}

fn draw_add_menu(ui: &mut Ui, patterns_len: usize) -> Option<PatternKind> {
    if patterns_len >= MAX_PATTERNS {
        let _ = ui.add_enabled(false, egui::Button::new(tl!("Add ingredient")));
        return None;
    }
    let mut selected = None;
    ui.menu_button(tl!("Add ingredient"), |ui| {
        for (id, label) in KINDS {
            if ui.button(crate::i18n::t(label)).clicked() {
                selected = Some(pattern_kind_from_id(id));
                ui.close();
            }
        }
    });
    selected
}

/// Draw the compact ingredient list and the one selected ingredient editor.
///
/// `fields` carries only private view state (`__palettePatternSelection`) and import requests;
/// `patterns` is the recipe-owned vector whose semantic edits are serialized by the caller.
pub(crate) fn show(ui: &mut Ui, fields: &mut Map<String, Value>, patterns: &mut Vec<Pattern>) {
    let t = Tokens::get(ui.ctx());
    let mut selected = selected_index(fields, patterns.len()).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(tl!("Threshold ingredients")).strong().color(t.text));
        if let Some(kind) = draw_add_menu(ui, patterns.len())
            && let Some(index) = add_pattern(fields, patterns, kind)
        {
            selected = index;
        }
    });

    if patterns.is_empty() {
        ui.label(egui::RichText::new(tl!("No patterns")).color(t.text_dim));
        return;
    }
    selected = selected.min(patterns.len().saturating_sub(1));
    let available_width = ui.available_width();
    let layout_width = if available_width.is_finite() { available_width } else { 480.0 };
    let wide = layout_width >= 480.0;
    if wide {
        let list_width = (layout_width * 0.38).clamp(190.0, 300.0);
        let editor_width = (layout_width - list_width - 12.0).max(120.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.vertical(|ui| {
                ui.set_width(list_width);
                draw_pattern_list(ui, fields, patterns, &mut selected);
            });
            ui.vertical(|ui| {
                ui.set_width(editor_width);
                if let Some(action) = draw_pattern_actions(ui, selected, patterns.len()) {
                    apply_list_action(fields, patterns, &mut selected, action);
                }
                if !patterns.is_empty() {
                    draw_pattern_editor(ui, fields, patterns, &mut selected);
                }
            });
        });
    } else {
        draw_pattern_list(ui, fields, patterns, &mut selected);
        ui.add_space(8.0);
        if let Some(action) = draw_pattern_actions(ui, selected, patterns.len()) {
            apply_list_action(fields, patterns, &mut selected, action);
        }
        if !patterns.is_empty() {
            draw_pattern_editor(ui, fields, patterns, &mut selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(kind: PatternKind, seed: u32) -> Pattern {
        Pattern { kind, seed, ..Pattern::default() }
    }

    #[test]
    fn adding_supported_kind_selects_new_identity() {
        let mut fields = Map::new();
        let mut patterns = vec![pattern(PatternKind::Bayer, 1)];
        let index = add_pattern(&mut fields, &mut patterns, PatternKind::Custom);
        assert_eq!(index, Some(1));
        assert_eq!(selected_index(&fields, patterns.len()), Some(1));
        assert_eq!(patterns[1].kind, PatternKind::Custom);
    }

    #[test]
    fn moving_selected_keeps_the_same_pattern_selected() {
        let mut fields = Map::new();
        let mut patterns = vec![pattern(PatternKind::Bayer, 1), pattern(PatternKind::WhiteNoise, 2)];
        set_selected_index(&mut fields, Some(1));
        let old_seed = patterns[1].seed;
        let next = move_selected(&mut patterns, 1, -1);
        assert_eq!(next, Some(0));
        assert_eq!(patterns[0].seed, old_seed);
        set_selected_index(&mut fields, next);
        assert_eq!(selected_index(&fields, patterns.len()), Some(0));
    }

    #[test]
    fn removing_selected_uses_previous_identity_and_empty_clears_selection() {
        let mut fields = Map::new();
        let mut patterns = vec![pattern(PatternKind::Bayer, 1), pattern(PatternKind::WhiteNoise, 2), pattern(PatternKind::BlueNoise, 3)];
        set_selected_index(&mut fields, Some(1));
        let next = remove_selected(&mut patterns, 1);
        set_selected_index(&mut fields, next);
        assert_eq!(next, Some(0));
        assert_eq!(patterns[0].seed, 1);
        assert_eq!(selected_index(&fields, patterns.len()), Some(0));
        let next = remove_selected(&mut patterns, 0);
        let next = remove_selected(&mut patterns, next.unwrap_or(0));
        set_selected_index(&mut fields, next);
        assert!(patterns.is_empty());
        assert_eq!(selected_index(&fields, patterns.len()), None);
    }

    #[test]
    fn opening_selected_disabled_custom_with_empty_matrix_keeps_recipe_unchanged() {
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, Default::default());
        let mut fields = Map::new();
        fields.insert(SELECTION_FIELD.into(), json!(1));
        let mut patterns = vec![
            pattern(PatternKind::Bayer, 1),
            Pattern { kind: PatternKind::Custom, enabled: false, matrix_size: 4, matrix: Vec::new(), ..Pattern::default() },
        ];
        let before = patterns.clone();
        let mut output = ctx
            .run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 720.0))), ..Default::default() }, |ui| {
                show(ui, &mut fields, &mut patterns)
            });
        output.textures_delta.clear();
        assert_eq!(selected_index(&fields, patterns.len()), Some(1));
        assert_eq!(patterns, before, "opening a disabled custom ingredient must not initialize its matrix");
    }

    #[test]
    fn native_row_selection_and_enable_edit_only_the_target_ingredient() {
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};

        let patterns = vec![
            pattern(PatternKind::Bayer, 1),
            Pattern { kind: PatternKind::Custom, enabled: false, matrix_size: 4, matrix: Vec::new(), ..Pattern::default() },
        ];
        let before = patterns.clone();
        let mut h = Harness::builder()
            .with_size(vec2(420.0, 720.0))
            .build_ui_state(|ui, (fields, patterns): &mut (Map<String, Value>, Vec<Pattern>)| show(ui, fields, patterns), (Map::new(), patterns));
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, "Bayer 1").click();
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(3);
        assert_eq!(selected_index(&h.state().0, h.state().1.len()), Some(1));
        assert!(h.get_by_role_and_label(Role::Button, "Custom 2").is_focused(), "arrow navigation moves focus with selection");
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(3);
        assert_eq!(selected_index(&h.state().0, h.state().1.len()), Some(0));
        assert!(h.get_by_role_and_label(Role::Button, "Bayer 1").is_focused());
        h.get_by_role_and_label(Role::Button, "Custom 2").click();
        h.run_steps(3);
        assert_eq!(selected_index(&h.state().0, h.state().1.len()), Some(1));
        assert_eq!(h.state().1, before, "selection must not initialize a disabled custom matrix");

        h.get_all_by_role_and_label(Role::CheckBox, "Enabled").nth(1).expect("second ingredient toggle").click();
        h.run_steps(3);
        assert!(h.state().1[1].enabled);
        assert_eq!(h.state().1[1].matrix, vec![0.5; 16], "explicit enabling creates a usable custom matrix");
        assert_eq!(h.state().1[0], before[0], "editing the selected ingredient leaves the other unchanged");
    }

    #[test]
    fn preview_strength_phase_and_scale_follow_kernel_visual_invariants() {
        let base = Pattern { kind: PatternKind::Bayer, matrix_size: 4, scale: 1, amount: 1.0, ..Pattern::default() };
        let full = preview_image(&base, [THUMB_SIDE, THUMB_SIDE], false).expect("valid preview");
        assert!(full.pixels.contains(&Color32::BLACK));
        assert!(full.pixels.contains(&Color32::WHITE));

        let no_strength = Pattern { amount: 0.0, ..base.clone() };
        let flat = preview_image(&no_strength, [THUMB_SIDE, THUMB_SIDE], false).expect("zero strength is valid");
        assert!(flat.pixels.windows(2).all(|pixels| pixels[0] == pixels[1]), "zero strength has no threshold variation");

        let scaled = Pattern { scale: 2, ..base.clone() };
        let scaled_image = preview_image(&scaled, [THUMB_SIDE, THUMB_SIDE], false).expect("scale is valid");
        for y in (0..THUMB_SIDE - 1).step_by(2) {
            for x in (0..THUMB_SIDE - 1).step_by(2) {
                let at = |x: usize, y: usize| scaled_image.pixels[y * THUMB_SIDE + x];
                assert_eq!(at(x, y), at(x + 1, y), "scale=2 repeats horizontally in each source block");
                assert_eq!(at(x, y), at(x, y + 1), "scale=2 repeats vertically in each source block");
            }
        }

        let shifted = Pattern { phase: [1, 0], ..base };
        assert_ne!(full.pixels, preview_image(&shifted, [THUMB_SIDE, THUMB_SIDE], false).expect("phase is valid").pixels);

        let ramp = preview_image(&shifted, RAMP_SIZE, true).expect("tone-ramp preview is valid");
        assert!((0..RAMP_SIZE[1]).all(|y| ramp.pixels[y * RAMP_SIZE[0]] == Color32::BLACK), "the ramp's dark edge maps to black");
        assert!((0..RAMP_SIZE[1]).all(|y| ramp.pixels[y * RAMP_SIZE[0] + RAMP_SIZE[0] - 1] == Color32::WHITE), "the ramp's bright edge maps to white");
    }

    #[test]
    fn pending_matrix_import_tracks_reorder_and_is_removed_with_target() {
        let mut fields = Map::new();
        let mut patterns = vec![pattern(PatternKind::Bayer, 1), pattern(PatternKind::Custom, 2)];
        fields.insert(IMPORT_FIELD.into(), json!(1));
        let mut selected = 1;
        apply_list_action(&mut fields, &mut patterns, &mut selected, ListAction::MoveUp);
        assert_eq!(selected, 0);
        assert_eq!(fields.get(IMPORT_FIELD).and_then(Value::as_u64), Some(0));
        apply_list_action(&mut fields, &mut patterns, &mut selected, ListAction::Remove);
        assert!(fields.get(IMPORT_FIELD).is_none());
    }

    #[test]
    fn pattern_stack_never_expands_its_host_column() {
        use egui_kittest::Harness;
        type LayoutState = (Map<String, Value>, Vec<Pattern>, Option<(egui::Rect, egui::Rect)>);

        for width in [250.0, 420.0, 544.0, 780.0] {
            let patterns =
                vec![pattern(PatternKind::Bayer, 1), pattern(PatternKind::WhiteNoise, 2), pattern(PatternKind::BlueNoise, 3), pattern(PatternKind::Bayer, 4)];
            let mut h = Harness::builder().with_size(vec2(width, 720.0)).build_ui_state(
                |ui, (fields, patterns, bounds): &mut LayoutState| {
                    let available = ui.available_rect_before_wrap();
                    let content = ui.scope(|ui| show(ui, fields, patterns)).response.rect;
                    *bounds = Some((available, content));
                },
                (Map::new(), patterns, None),
            );
            crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(5);
            let (available, content) = h.state().2.expect("native ingredient layout");
            assert!(
                content.left() >= available.left() - 0.5 && content.right() <= available.right() + 0.5,
                "ingredient content {content:?} must stay inside host {available:?} at width {width}"
            );
        }
    }
}
