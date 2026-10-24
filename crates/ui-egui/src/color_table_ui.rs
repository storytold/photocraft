//! Image › Mode › Color Table: the entry grid of an Indexed Color document. Click a swatch to
//! pick its colour (PhotoCraft's Color Picker), right-click it to make it the transparent entry;
//! Load…/Save… exchange Adobe `.act` tables and palette PNGs. The table lives in the dialog's
//! `colors` field (a JSON array of `"#rrggbb"`), seeded from the document when the dialog opens
//! (`filter_dialog::apply_command_defaults`); a preset choice is a starting point — editing any
//! swatch makes the table Custom, as in Photoshop.

use serde_json::{Map, Value, json};

/// Whether `fields` belong to the Color Table dialog.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some("image.mode.colorTable")
}

/// The armed "Set Transparent" pick and the canvas "Color Picker" eyedropper.
pub const TRANSPARENT_PICK: &str = "__transparentPick";
pub const PALETTE_EYEDROPPER: &str = "__paletteEyedropper";
pub const PALETTE_HIGHLIGHT: &str = "__paletteHighlight";
pub const PALETTE_RANGE: &str = "__paletteRange";

/// The table shown in the grid: for Color Table, `colors` when Custom else the chosen preset;
/// for Indexed Color (Custom…), `paletteColors`.
pub fn table_colors(f: &Map<String, Value>) -> Vec<[u8; 3]> {
    if owns(f) {
        match f.get("table").and_then(Value::as_str).unwrap_or("custom") {
            "custom" => f.get(crate::palette_file_ui::COLORS_FIELD).and_then(crate::palette_file_ui::hex_array_to_rgb).unwrap_or_default(),
            preset => photocraft_engine::mode_cmds::preset_table(preset).unwrap_or_default(),
        }
    } else {
        f.get(crate::palette_file_ui::CUSTOM_FIELD).and_then(crate::palette_file_ui::hex_array_to_rgb).unwrap_or_default()
    }
}

/// The shared 256-cell palette grid, Load…/Save…, the armed "Set Transparent" toggle button, and
/// the "Color Picker" canvas eyedropper — used by both Indexed Color (Custom…) and Color Table so
/// the two dialogs always stay in sync.
pub fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let is_color_table = owns(f);
    let colors = table_colors(f);
    if is_color_table && colors.is_empty() {
        return; // No document table (the command is disabled then); never panic.
    }
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    let field = crate::palette_file_ui::colors_field(&cmd);
    let highlight = f.get(PALETTE_HIGHLIGHT).and_then(Value::as_u64).map(|i| i as usize);
    ui.add_space(6.0);
    let transparent_armed = f.get(TRANSPARENT_PICK).and_then(Value::as_bool) == Some(true);
    // The shared 256-cell grid (dashed empties, the transparent marker, Set/Clear on right-click,
    // click for one colour or drag across cells for a first→last ramp).
    if let Some((lo, hi)) = crate::palette_file_ui::palette_grid(ui, f, &colors, highlight) {
        select_cells(f, &colors, lo, hi);
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Load…"), 72.0).clicked() {
            crate::palette_file_ui::request_load(f);
        }
        ui.add_enabled_ui(!colors.is_empty(), |ui| {
            if crate::widgets::secondary_button(ui, tl!("Save…"), 72.0).clicked() {
                // Save what the grid shows, preset or custom.
                f.insert(field.into(), json!(crate::palette_file_ui::hex_array(&colors)));
                crate::palette_file_ui::request_save(f);
            }
        });
        if crate::widgets::toggle_button(ui, tl!("Set Transparent"), 110.0, transparent_armed).clicked() {
            f.insert(TRANSPARENT_PICK.into(), json!(!transparent_armed));
        }
        let palette_eyedropper = f.get(PALETTE_EYEDROPPER).and_then(Value::as_bool) == Some(true);
        ui.add_enabled_ui(!colors.is_empty(), |ui| {
            if crate::icons::button(ui, "pipette", 24.0, palette_eyedropper, tl!("Pick a Palette Colour From the Image")).clicked() {
                f.insert(PALETTE_EYEDROPPER.into(), json!(!palette_eyedropper));
            }
        });
        ui.label(egui::RichText::new(tl!("Color Picker")).color(if palette_eyedropper { t.accent_text } else { t.text_dim }));
    });
}

/// Act on a single-cell click (`lo == hi`) or a dragged cell range (`lo < hi`) in the palette grid.
pub fn select_cells(f: &mut Map<String, Value>, colors: &[[u8; 3]], lo: usize, hi: usize) {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let is_color_table = owns(f);
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    let field = crate::palette_file_ui::colors_field(&cmd);
    let transparent_armed = f.get(TRANSPARENT_PICK).and_then(Value::as_bool) == Some(true);
    if lo == hi {
        f.remove(PALETTE_RANGE);
        if transparent_armed {
            // Armed "Set Transparent": a grid click sets the entry; re-clicking it (or the
            // button) clears. A dashed cell grows the table first so the index is valid.
            if lo >= colors.len() {
                materialize(f, colors, lo);
            }
            if f.get("transparent").and_then(Value::as_u64) == Some(lo as u64) {
                f.insert("transparent".into(), Value::Null);
            } else {
                f.insert("transparent".into(), json!(lo));
            }
        } else {
            // A swatch click picks its colour; a dashed cell grows the table (black fill)
            // first, as in Photoshop. Editing a preset's swatch starts from it (the Table
            // becomes Custom).
            if lo >= colors.len() {
                materialize(f, colors, lo);
            } else if is_color_table && f.get("table").and_then(Value::as_str) != Some("custom") {
                f.insert(field.into(), json!(crate::palette_file_ui::hex_array(colors)));
                f.insert("table".into(), json!("custom"));
            }
            let title = if is_color_table { tl!("Table Color") } else { tl!("Palette Color") };
            crate::color_picker_ui::request_field(f, &format!("/{field}/{lo}"), title);
        }
    } else {
        // Dragged a range `lo..=hi`: materialize up to `hi`, highlight the range, and open the
        // two-step Color Picker (First Color at `lo`, then Last Color at `hi`, then linear ramp).
        if hi >= colors.len() {
            materialize(f, colors, hi);
        } else if is_color_table && f.get("table").and_then(Value::as_str) != Some("custom") {
            f.insert(field.into(), json!(crate::palette_file_ui::hex_array(colors)));
            f.insert("table".into(), json!("custom"));
        }
        f.remove(PALETTE_HIGHLIGHT);
        f.insert(PALETTE_RANGE.into(), json!([lo, hi]));
        crate::color_picker_ui::request_field_range(f, field, lo, hi);
    }
}

/// Materialize the grid's table into its field (preset → Custom for Color Table) and pad it with
/// black to index `i`, so a new entry at `i` (and any skipped slots) exist.
fn materialize(f: &mut Map<String, Value>, colors: &[[u8; 3]], i: usize) {
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default();
    let field = crate::palette_file_ui::colors_field(cmd);
    if owns(f) && f.get("table").and_then(Value::as_str) != Some("custom") {
        f.insert("table".into(), json!("custom"));
    }
    let padded = crate::palette_file_ui::pad_to(colors, i);
    f.insert(field.into(), json!(crate::palette_file_ui::hex_array(&padded)));
}
#[cfg(test)]
mod tests {
    use super::*;

    fn fields(table: &str, colors: Option<Vec<&str>>) -> Map<String, Value> {
        let mut f = Map::new();
        f.insert("__command".into(), json!("image.mode.colorTable"));
        f.insert("table".into(), json!(table));
        if let Some(c) = colors {
            f.insert(crate::palette_file_ui::COLORS_FIELD.into(), json!(c));
        }
        f
    }

    #[test]
    fn owns_only_the_color_table_dialog() {
        assert!(owns(&fields("custom", None)));
        let mut other = fields("custom", None);
        other.insert("__command".into(), json!("image.mode.indexedColor"));
        assert!(!owns(&other));
    }

    #[test]
    fn grid_shows_the_document_table_when_custom() {
        let f = fields("custom", Some(vec!["#ff0000", "#00ff00"]));
        assert_eq!(table_colors(&f), vec![[255, 0, 0], [0, 255, 0]]);
    }

    #[test]
    fn grid_shows_the_preset_until_edited() {
        let f = fields("grayscale", Some(vec!["#ff0000"]));
        let colors = table_colors(&f);
        assert_eq!(colors.len(), 256);
        assert_eq!(colors[0], [0, 0, 0]);
        assert_eq!(colors[255], [255, 255, 255]);
    }

    #[test]
    fn editing_a_preset_swatch_materializes_custom() {
        let mut f = fields("grayscale", None);
        // A swatch click (what controls() routes to select_cells), before asking the picker.
        let colors = table_colors(&f);
        select_cells(&mut f, &colors, 5, 5);
        assert_eq!(f.get("table").and_then(Value::as_str), Some("custom"));
        assert_eq!(f.get(crate::palette_file_ui::COLORS_FIELD).and_then(Value::as_array).unwrap().len(), 256);
        // The picker request points at the materialized entry.
        let req = f.get(crate::color_picker_ui::REQUEST).cloned().unwrap();
        assert_eq!(req["field"].as_str(), Some("/colors/5"));
    }

    #[test]
    fn transparent_entry_roundtrips_through_fields() {
        let mut f = fields("custom", Some(vec!["#000000", "#ffffff"]));
        f.insert("transparent".into(), json!(1));
        assert_eq!(f.get("transparent").and_then(Value::as_u64), Some(1));
        f.insert("transparent".into(), Value::Null); // Clear Transparency.
        assert_eq!(f.get("transparent"), Some(&Value::Null));
    }

    #[test]
    fn dashed_click_grows_the_table_and_requests_picker() {
        let mut f = fields("custom", Some(vec!["#ff0000", "#00ff00"]));
        // A dashed-cell click (i = 5, table has 2 entries), what controls() routes to select_cells.
        let colors = table_colors(&f);
        select_cells(&mut f, &colors, 5, 5);
        let grown = f.get(crate::palette_file_ui::COLORS_FIELD).and_then(crate::palette_file_ui::hex_array_to_rgb).unwrap();
        assert_eq!(grown.len(), 6);
        assert_eq!((grown[0], grown[1]), ([255, 0, 0], [0, 255, 0]));
        assert_eq!(&grown[2..6], &[[0, 0, 0]; 4], "skipped slots are black");
        let req = f.get(crate::color_picker_ui::REQUEST).cloned().unwrap();
        assert_eq!(req["field"].as_str(), Some("/colors/5"));
    }

    #[test]
    fn armed_transparent_pick_sets_and_clears() {
        let mut f = fields("custom", Some(vec!["#000000", "#ffffff"]));
        f.insert(TRANSPARENT_PICK.into(), json!(true)); // Arm the pipette.
        let colors = table_colors(&f);
        for _ in 0..2 {
            // Click entry 1 twice: set, then clear.
            select_cells(&mut f, &colors, 1, 1);
        }
        assert_eq!(f.get("transparent"), Some(&Value::Null), "re-clicking the same cell clears");
        f.insert(TRANSPARENT_PICK.into(), json!(false)); // Disarm again.
    }

    #[test]
    fn armed_transparent_pick_on_dashed_cell_grows_the_table() {
        let mut f = fields("custom", Some(vec!["#000000"]));
        f.insert(TRANSPARENT_PICK.into(), json!(true));
        let colors = table_colors(&f);
        select_cells(&mut f, &colors, 3, 3); // Dashed cell.
        let grown = f.get(crate::palette_file_ui::COLORS_FIELD).and_then(crate::palette_file_ui::hex_array_to_rgb).unwrap();
        assert_eq!(grown.len(), 4, "the table grew so the transparent index is valid");
        assert_eq!(f.get("transparent").and_then(Value::as_u64), Some(3));
    }

    #[test]
    fn drag_range_10_to_50_runs_two_step_picker_and_interpolates_colors() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut doc = photocraft_doc::Document::with_background(
            "idx",
            photocraft_doc::Size::new(4, 4),
            photocraft_color::ColorMode::Indexed,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::BLACK,
        );
        doc.color_table = Some(photocraft_doc::ColorTable { colors: vec![[0, 0, 0]; 64], transparent: None });
        app.session.add_document(doc, None);
        let dlg_id = crate::filter_dialog::open(&mut app, "image.mode.colorTable").unwrap();
        {
            let d = app.ui.dialog_mut(dlg_id).unwrap();
            let colors = table_colors(&d.fields);
            // User drags from index 10 to index 50 and releases the mouse.
            select_cells(&mut d.fields, &colors, 10, 50);
            assert_eq!(d.fields.get(PALETTE_RANGE), Some(&json!([10, 50])));
        }
        // Step 1: First Color picker opens for index 10.
        crate::color_picker_ui::open_requested(&mut app, dlg_id);
        let first_picker_id = crate::color_picker_ui::top(&app).expect("First Color picker opened");
        assert_eq!(app.ui.dialog_mut(first_picker_id).unwrap().fields.get("__label").and_then(Value::as_str), Some("Color Picker (First Color)"));
        // Set 1st color to #00ff00 and confirm.
        app.ui.dialog_mut(first_picker_id).unwrap().fields.insert("color".into(), json!("#00ff00"));
        crate::dialogs::confirm(&mut app, first_picker_id).unwrap();

        // Step 2: Last Color picker automatically opens for index 50.
        let last_picker_id = crate::color_picker_ui::top(&app).expect("Last Color picker opened");
        assert_eq!(app.ui.dialog_mut(last_picker_id).unwrap().fields.get("__label").and_then(Value::as_str), Some("Color Picker (Last Color)"));
        // Set last color to #ff00ff and confirm.
        app.ui.dialog_mut(last_picker_id).unwrap().fields.insert("color".into(), json!("#ff00ff"));
        crate::dialogs::confirm(&mut app, last_picker_id).unwrap();

        // Verify the table entries 10..=50 smoothly ramp from #00ff00 to #ff00ff.
        let colors = table_colors(&app.ui.dialog_mut(dlg_id).unwrap().fields);
        assert_eq!(colors[9], [0, 0, 0]);
        assert_eq!(colors[10], [0, 255, 0]);
        assert_eq!(colors[30], [128, 128, 128]);
        assert_eq!(colors[50], [255, 0, 255]);
        assert_eq!(colors[51], [0, 0, 0]);
    }
}
