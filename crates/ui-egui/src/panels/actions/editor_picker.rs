//! Stateless Actions editor thumbnail picker. The caller owns metadata and stages explicit choices.
use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Color32, Id, Stroke, Vec2};
use photocraft_engine::actions_cmds::BUTTON_COLORS;

use super::editor_glyphs::{GLYPH_COLUMNS, GLYPHS, display_glyph};
use crate::{icons, theme::Tokens};

/// A deliberate click, not a normalized copy of the current metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Empty selects the default glyph; this is distinct from an explicit ClickAction choice.
    Icon(&'static str),
    /// A measured colour name (empty for none), independent of the current icon.
    Color(&'static str),
}

const CELL: f32 = 32.0;
const GLYPH_LABELS: [&str; 30] = [
    "Click action",
    "Crop",
    "Merge layers",
    "Adjustments",
    "Hue and saturation",
    "Exposure",
    "Colour lookup tables",
    "Black and white",
    "Smart removal",
    "Dodge",
    "Burn",
    "Erase",
    "Layer effects",
    "Generated layer effects",
    "Sharpen",
    "Object selection",
    "Magic wand",
    "Generate",
    "Text colour",
    "Text",
    "Download",
    "Layers",
    "Brush",
    "Select subject",
    "Remove background",
    "Blur",
    "Harmonize layers",
    "Clarity",
    "Generated styles",
    "Show grid",
];

// Picker order differs from ATN index order, especially the two blues and the new teal.
const COLOR_GRID: [(&str, &str); 12] = [
    ("", "None"),
    ("red", "Red"),
    ("orange", "Orange"),
    ("yellow", "Yellow"),
    ("green", "Green"),
    ("teal", "Teal"),
    ("blue", "Light blue"),
    ("blue2", "Blue"),
    ("magenta", "Magenta"),
    ("purple", "Purple"),
    ("violet", "Lavender"),
    ("gray", "Grey"),
];

fn thumbnail_style(color: &str, swatches: &[Color32; 12], tokens: &Tokens) -> (Color32, Color32) {
    let index = BUTTON_COLORS.iter().position(|name| *name == color).filter(|index| *index > 0);
    match index.and_then(|index| swatches.get(index)).copied() {
        Some(fill) => (fill, Tokens::ACTION_THUMBNAIL_GLYPH),
        None => (tokens.card, tokens.text),
    }
}

/// Selection marks remain legible independently of the fixed action-glyph ink.
fn swatch_style(color: &str, swatches: &[Color32; 12], tokens: &Tokens) -> (Color32, Color32) {
    let (fill, glyph) = thumbnail_style(color, swatches, tokens);
    let coloured = BUTTON_COLORS.iter().skip(1).any(|name| *name == color);
    (fill, if coloured { Tokens::action_thumbnail_ink(fill) } else { glyph })
}

fn color_button(ui: &mut egui::Ui, color: &str, label: &str, selected: bool, swatches: &[Color32; 12], tokens: &Tokens) -> egui::Response {
    let (fill, ink) = swatch_style(color, swatches, tokens);
    let style = IconButtonStyle {
        normal: ButtonVisuals { fill, stroke: Stroke::new(1.0, tokens.field_border), icon: ink },
        hovered: ButtonVisuals { fill, stroke: Stroke::new(1.0, tokens.text), icon: ink },
        pressed: None,
        selected: ButtonVisuals { fill, stroke: Stroke::new(2.0, tokens.accent_border), icon: ink },
        corner_radius: tokens.radius_sm.into(),
        focus_stroke: Stroke::new(1.0, tokens.accent),
    };
    IconButton::new(tl!(label), Vec2::splat(CELL), &style)
        .selected(selected)
        .show(ui, |ui, rect, tint| {
            if color.is_empty() {
                let slash = [rect.left_bottom() + egui::vec2(5.0, -5.0), rect.right_top() + egui::vec2(-5.0, 5.0)];
                ui.painter().line_segment(slash, Stroke::new(1.5, tint));
            } else if selected {
                icons::paint(ui, rect, "check", 16.0, tint);
            }
        })
        .on_hover_text(tl!(label))
}

fn contents(ui: &mut egui::Ui, icon: &str, color: &str, swatches: &[Color32; 12], tokens: &Tokens) -> Option<Choice> {
    let mut choice = None;
    ui.label(tl!("Choose icon"));
    if icons::button(ui, "square", CELL, icon.is_empty(), tl!("Default icon")).clicked() {
        choice = Some(Choice::Icon(""));
    }
    egui::Grid::new("action-editor-glyph-grid").num_columns(GLYPH_COLUMNS).spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
        for (index, ((identifier, asset), label)) in GLYPHS.into_iter().zip(GLYPH_LABELS).enumerate() {
            if icons::button(ui, asset, CELL, icon == identifier, label).clicked() {
                choice = Some(Choice::Icon(identifier));
            }
            if index % GLYPH_COLUMNS == GLYPH_COLUMNS - 1 {
                ui.end_row();
            }
        }
    });
    ui.separator();
    ui.label(tl!("Choose color"));
    egui::Grid::new("action-editor-color-grid").num_columns(6).spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
        for (index, (name, label)) in COLOR_GRID.into_iter().enumerate() {
            if color_button(ui, name, label, color == name, swatches, tokens).clicked() {
                choice = Some(Choice::Color(name));
            }
            if index % 6 == 5 {
                ui.end_row();
            }
        }
    });
    if choice.is_some() {
        ui.close();
    }
    choice
}

/// Draw the thumbnail and its picker, returning only an explicit selection.
///
/// `id` scopes popup/widget state for this editor. `icon`/`color` are authoritative raw values;
/// unknowns display neutrally and are never returned as replacements without a click. `swatches`
/// are caller-selected PhotoCraft colours in ATN index order, not measured reference RGB samples.
/// Slot zero is ignored in favour of the neutral theme surface. Register the thirty glyph assets
/// through the existing icon table when connecting this leaf to the actual editor consumer.
pub fn show(ui: &mut egui::Ui, id: Id, icon: &str, color: &str, swatches: &[Color32; 12]) -> Option<Choice> {
    let tokens = Tokens::get(ui.ctx());
    ui.push_id(id, |ui| {
        let mut choice = None;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::hover());
            let (fill, ink) = thumbnail_style(color, swatches, &tokens);
            ui.painter().rect_filled(rect, tokens.radius_sm, fill);
            icons::paint(ui, rect, display_glyph(icon), 28.0, ink);
            ui.menu_button(tl!("Choose icon and color"), |ui| choice = contents(ui, icon, color, swatches, &tokens));
        });
        choice
    })
    .inner
}

#[cfg(test)]
#[path = "editor_picker_tests.rs"]
mod tests;
