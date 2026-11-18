//! Palette files and the editable palette grid shared by Image › Mode › Indexed Color (Custom…)
//! and Image › Mode › Color Table.
//!
//! The grid is the dialog's table field ([`colors_field`]: `colors` for Color Table,
//! `paletteColors` for Indexed Color's Custom palette) drawn as the 16×16 slots of a colour table,
//! so an entry's position is its index whatever the table's length: entries the table does not have
//! yet are empty (dashed) slots. A click opens the host's Color Picker on that entry's colour
//! ([`click_entry`]; an empty slot grows the table with opaque black first, as a new entry does);
//! the answer comes back under a private key and is folded in by [`apply_picks`]. With the
//! "Set Transparent" switch on ([`armed_row`]) a click marks that entry as the table's transparent
//! one instead, or clears the mark of the entry that has it; the marked entry is drawn as a
//! checkerboard with an X. The "Color Picker" pipette ([`eyedropper_row`]) arms the canvas instead:
//! a press on the image highlights the entry that pixel uses ([`sample`], [`match_entry`]).
//!
//! Load… and Save… exchange `.act` colour tables and palette PNGs; parsing and encoding live in
//! [`parse_file`] and [`encode_file`], and the footer buttons are answered by
//! [`crate::color_table_ui::footer_action`].

use craft::behavior::IconId;
use craft::theme::Srgba;
use craft::ui::buttons::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamKind};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::EyedropperRequest;
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::PhotocraftApp;

/// The Color Table dialog's table field: a list of `"#rrggbb"`.
pub const COLORS_FIELD: &str = "colors";
/// The Indexed Color dialog's Custom palette field (its `colors` param is the entry count).
pub const CUSTOM_FIELD: &str = "paletteColors";
/// The table's transparent entry, as an index.
pub const TRANSPARENT: &str = "transparent";
/// The "Set Transparent" switch: the next grid click marks an entry instead of editing it. Private,
/// so it is dropped from the command's parameters.
pub const TRANSPARENT_ARMED: &str = "__paletteTransparentArmed";
/// The palette pipette ("Color Picker") switch: while it is on, a press on the image highlights the
/// entry the pixel uses. Private, like the other two.
pub const EYEDROPPER: &str = "__paletteEyedropper";
/// The key the canvas eyedropper answers the colour under the press with.
pub const SAMPLE_KEY: &str = "__paletteSampled";
/// The entry the pipette last highlighted, drawn with an accent ring. Private as the other two, so
/// the command never sees it.
pub const HIGHLIGHT: &str = "__paletteHighlight";

/// The cells of the grid: a colour table's 16×16 slots, drawn whether or not the table fills them.
const COLS: usize = 16;
const CELLS: usize = COLS * COLS;
/// A cell's side in logical pixels, about a colour table's slot on screen.
const CELL: f32 = 22.0;

/// The field holding the table for `command`'s dialog.
pub fn colors_field(command: &str) -> &'static str {
    if command == crate::color_table_ui::COMMAND { COLORS_FIELD } else { CUSTOM_FIELD }
}

/// The private field a picked colour for entry `i` comes back in.
fn pick_key(i: usize) -> String {
    format!("__paletteColor{i}")
}

/// Entries as `"#rrggbb"` strings (the table field's items).
pub fn hex_array(colors: &[[u8; 3]]) -> Vec<String> {
    colors.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect()
}

/// A `"#rrggbb"` string as bytes.
fn unhex(s: &str) -> Option<[u8; 3]> {
    let f = crate::color_picker_ui::parse_hex(s)?;
    Some(f.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

/// A table field (a list of `"#rrggbb"`) as entries; `None` when the field is absent or holds
/// anything else.
pub fn table_rgb(fields: &DialogFields, key: &str) -> Option<Vec<[u8; 3]>> {
    let list = fields.list(key)?;
    list.iter().map(|v| if let FieldValue::Text(s) = v { unhex(s) } else { None }).collect()
}

/// Store `colors` in the table field `key`.
pub fn set_table(fields: &mut DialogFields, key: &str, colors: &[[u8; 3]]) {
    fields.set(key, FieldValue::List(hex_array(colors).into_iter().map(FieldValue::Text).collect()));
}

/// Fold the colours the host's Color Picker answered (`__paletteColor<i>`) into the table, and
/// drop the private fields: the row owns them, the command never sees them.
fn apply_picks(fields: &mut DialogFields, key: &str) {
    let mut picked: Vec<(usize, [u8; 3])> = Vec::new();
    for (name, value) in fields.iter() {
        let Some(i) = name.strip_prefix("__paletteColor").and_then(|i| i.parse::<usize>().ok()) else { continue };
        if let FieldValue::Color(rgb) = value {
            picked.push((i, rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)));
        }
    }
    if picked.is_empty() {
        return;
    }
    let mut colors = table_rgb(fields, key).unwrap_or_default();
    for (i, rgb) in picked {
        if let Some(slot) = colors.get_mut(i) {
            *slot = rgb;
        }
        fields.remove(&pick_key(i));
    }
    set_table(fields, key, &colors);
}

/// The table the grid shows and the transparent entry it marks: the dialog's own field (seeded from
/// the document, or loaded, or edited), or for Color Table a chosen preset (which has no
/// transparent entry of its own).
fn shown(fields: &DialogFields, command: &str, key: &str) -> (Vec<[u8; 3]>, Option<usize>) {
    let transparent = fields.int(TRANSPARENT).and_then(|t| usize::try_from(t).ok());
    if command == crate::color_table_ui::COMMAND {
        match fields.text("table").unwrap_or("custom") {
            preset @ ("blackBody" | "grayscale" | "spectrum" | "systemMac" | "systemWindows" | "web") => {
                (photocraft_engine::mode_cmds::preset_table(preset).unwrap_or_default(), None)
            }
            _ => (table_rgb(fields, key).unwrap_or_default(), transparent),
        }
    } else {
        (table_rgb(fields, key).unwrap_or_default(), transparent)
    }
}

/// Make the shown table the dialog's own, so an edit applies to what the grid displays (a preset
/// becomes Custom, as in Photoshop).
fn materialize(fields: &mut DialogFields, command: &str, key: &str, colors: &[[u8; 3]]) {
    set_table(fields, key, colors);
    if command == crate::color_table_ui::COMMAND {
        fields.set("table", FieldValue::Text("custom".into()));
    }
}

/// The "Set Transparent" switch: while it is on, a click on the grid marks that entry as the
/// table's transparent one instead of opening its colour. A row of its own, so its label reads
/// like the dialog's other rows and its state is visible.
pub fn armed_row() -> Param {
    Param::new(TRANSPARENT_ARMED, Label::from(tl!("Set Transparent")), ParamKind::bool(false)).not_remembered()
}

/// The palette pipette ("Color Picker"): while it is on, a press on the image highlights the entry
/// the pixel under it uses ([`sample`]). A row of its own, as the switch above is.
pub fn eyedropper_row() -> Param {
    Param::custom(EYEDROPPER, Label::from(tl!("Color Picker")), |ui, fields, cx| {
        let mut on = fields.bool(EYEDROPPER).unwrap_or(false);
        let was = on;
        let shown = ToggleButton::new(tl!("Pick a Palette Colour From the Image"), Bind::bound(&mut on))
            .icon(IconSpec::svg(IconId::new("pipette")))
            .icon_only()
            .escape_disarms()
            .tooltip(tl!("Pick a Palette Colour From the Image"))
            .show(ui);
        if on != was {
            fields.set(EYEDROPPER, on);
        }
        if on {
            // The next press on the image answers `SAMPLE_KEY` with the colour under it, which the
            // host hands to `sample` (the dialog's own pipette, not a row's colour).
            cx.arm_eyedropper(EyedropperRequest::new(SAMPLE_KEY));
            cx.sampling(true);
        }
        shown.response
    })
}

/// A press at `at` on the image with the palette pipette armed: highlight the entry the sampled
/// pixel uses — the one matching its colour (an exact match after rounding, else the nearest by
/// squared distance, for a dithered edge), or, over a clear pixel, the table's transparent entry (a
/// clear pixel is that entry's, not a colour's). Returns whether the pipette took the press: `false`
/// once its switch is off, dropping the request it left behind so the press goes to the tool.
pub fn sample(app: &mut PhotocraftApp, id: DialogId, at: [f64; 2]) -> bool {
    let Some((command, on)) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|dialog| {
        let command = dialog.key()?;
        owns(command).then(|| (command.to_owned(), dialog.fields().bool(EYEDROPPER).unwrap_or(false)))
    }) else {
        return false;
    };
    if !on {
        crate::dialogs::canvas_input::disarm_dropper(app);
        return false;
    }
    let (colors, transparent) = app
        .framework
        .dialogs
        .get_by_id::<HostedParamDialog>(id)
        .map(|dialog| (visible_table(dialog.fields(), &command), transparent_of(dialog.fields())))
        .unwrap_or_default();
    let Some([r, g, b, a]) = crate::canvas::composite_color_alpha(app, at[0], at[1]) else {
        return true; // Off the image: nothing to highlight, the pipette stays armed.
    };
    let entry = if a > 0.0 { match_entry(&colors, [r, g, b]) } else { transparent };
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id)
        && let Some(entry) = entry
    {
        dialog.set_field(HIGHLIGHT, FieldValue::Int(entry as i64));
    }
    true
}

/// Whether `command`'s dialog carries the palette table (the two the grid belongs to).
fn owns(command: &str) -> bool {
    command == crate::color_table_ui::COMMAND || command == crate::color_table_ui::INDEXED_COLOR
}

/// The table the dialog `fields` shows: the same rule as [`shown`], from the fields alone.
fn visible_table(fields: &DialogFields, command: &str) -> Vec<[u8; 3]> {
    let (colors, _) = shown(fields, command, colors_field(command));
    colors
}

fn transparent_of(fields: &DialogFields) -> Option<usize> {
    fields.int(TRANSPARENT).and_then(|t| usize::try_from(t).ok())
}

/// The palette entry matching `target` (0..=1 channels): an exact match after rounding, else the
/// nearest by squared 8-bit distance (a dithered edge's colour). `None` for an empty table.
pub fn match_entry(colors: &[[u8; 3]], target: [f32; 3]) -> Option<usize> {
    if colors.is_empty() {
        return None;
    }
    let t = target.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    colors.iter().position(|c| *c == t).or_else(|| {
        colors
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let d = (0..3).map(|k| (i32::from(c[k]) - i32::from(t[k])).pow(2) as u32).sum::<u32>();
                (d, i)
            })
            .min_by_key(|(d, _)| *d)
            .map(|(_, i)| i)
    })
}

/// The palette grid of `command`'s dialog: 16×16 slots, the table's entries painted and the rest
/// dashed. The table is the dialog's own field (seeded from the document when the dialog opens).
pub fn grid_row(command: &'static str, label: Label) -> Param {
    let key = colors_field(command);
    Param::custom(key, label, move |ui, fields, cx| {
        apply_picks(fields, key);
        let (colors, transparent) = shown(fields, command, key);
        let armed = fields.bool(TRANSPARENT_ARMED).unwrap_or(false);
        let highlight = fields.int(HIGHLIGHT).and_then(|i| usize::try_from(i).ok());
        let (response, hit) = grid(ui, &colors, transparent, highlight);
        if let Some(i) = hit {
            if armed {
                // Mark the entry the click landed on; clicking the marked entry clears it, as the
                // switch does.
                if transparent == Some(i) {
                    fields.remove(TRANSPARENT);
                } else {
                    materialize(fields, command, key, &pad_to(&colors, i));
                    fields.set(TRANSPARENT, FieldValue::Int(i as i64));
                }
            } else if let Some(color) = click_entry(command, fields, &colors, i) {
                // The Color Picker opens on the entry's colour and files its answer under the
                // private key `apply_picks` folds back in.
                cx.pick_color(&pick_key(i), srgba(color));
            }
        }
        response
    })
}

/// A click on entry `i` with the "Set Transparent" switch off: the shown table becomes the
/// dialog's own, an empty slot grows it with opaque black entries up to `i` (as a new slot does),
/// and the entry's colour — what the Color Picker opens on — is returned. `None` past the table's
/// 256 slots, which the grid never reports.
pub fn click_entry(command: &str, fields: &mut DialogFields, colors: &[[u8; 3]], i: usize) -> Option<[u8; 3]> {
    let grown = pad_to(colors, i);
    let color = *grown.get(i)?;
    materialize(fields, command, colors_field(command), &grown);
    Some(color)
}

/// `colors` grown so index `i` exists, the added entries opaque black (a new entry's colour, never
/// a transparent one) and the table never past its 256 slots. A table that already has the index
/// (or more entries) is left as it is, never shrunk.
fn pad_to(colors: &[[u8; 3]], i: usize) -> Vec<[u8; 3]> {
    let mut v = colors.to_vec();
    let want = i.saturating_add(1).min(CELLS);
    if want > v.len() {
        v.resize(want, [0, 0, 0]);
    }
    v
}

fn srgba([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// The 16×16 table drawn as touching cells: each entry's colour, a dashed slot for one the table
/// does not have yet, a checkerboard and an X over the transparent entry, and an accent ring around
/// the entry the palette pipette last highlighted. Returns the cell a click landed on.
fn grid(ui: &mut egui::Ui, colors: &[[u8; 3]], transparent: Option<usize>, highlight: Option<usize>) -> (egui::Response, Option<usize>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let side = CELL * COLS as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    if response.hovered()
        && let Some(i) = response.hover_pos().and_then(|p| cell_at(rect, p))
    {
        let name = colors.get(i).map_or_else(|| format!("{i}"), |[r, g, b]| format!("{i} #{r:02x}{g:02x}{b:02x}"));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        response.clone().on_hover_text(name);
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        for i in 0..CELLS {
            let cell = cell_rect(rect, i).shrink(0.5);
            match colors.get(i) {
                Some([r, g, b]) => {
                    painter.rect_filled(cell, 1.0, Color32::from_rgb(*r, *g, *b));
                    painter.rect_stroke(cell, 1.0, Stroke::new(0.5, t.field_border), StrokeKind::Inside);
                    if transparent == Some(i) {
                        // The entry that carries transparency reads as no colour at all.
                        checker(painter, cell);
                        let m = vec2(cell.width() * 0.3, cell.height() * 0.3);
                        let stroke = Stroke::new(1.5, t.text);
                        painter.line_segment([cell.center() - m, cell.center() + m], stroke);
                        painter.line_segment([cell.center() + vec2(m.y, -m.x), cell.center() - vec2(m.y, -m.x)], stroke);
                    }
                }
                None => dashed(painter, cell, t.field_border),
            }
            if highlight == Some(i) {
                // The entry the palette pipette last matched, ringed (over a colour or a checker).
                painter.rect_stroke(cell.shrink(0.5), 1.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
            }
        }
    }
    let hit = response.clicked().then(|| response.interact_pointer_pos().and_then(|p| cell_at(rect, p))).flatten();
    (response, hit)
}

/// The rectangle of cell `i` in a table starting at `rect`.
fn cell_rect(rect: Rect, i: usize) -> Rect {
    let (col, row) = (i % COLS, i / COLS);
    Rect::from_min_size(pos2(rect.left() + col as f32 * CELL, rect.top() + row as f32 * CELL), vec2(CELL, CELL))
}

/// The cell `p` falls in, if any.
fn cell_at(rect: Rect, p: egui::Pos2) -> Option<usize> {
    if !rect.contains(p) {
        return None;
    }
    let col = ((p.x - rect.left()) / CELL).floor().max(0.0) as usize;
    let row = ((p.y - rect.top()) / CELL).floor().max(0.0) as usize;
    let i = row * COLS + col;
    (i < CELLS).then_some(i)
}

/// A two-tone checkerboard filling `rect`, drawn as quarter-cell squares.
fn checker(painter: &egui::Painter, rect: Rect) {
    let step = rect.width() / 2.0;
    let light = Color32::from_gray(200);
    let dark = Color32::from_gray(140);
    for row in 0..2 {
        for col in 0..2 {
            let color = if (row + col) % 2 == 0 { light } else { dark };
            let r = Rect::from_min_size(pos2(rect.left() + col as f32 * step, rect.top() + row as f32 * step), vec2(step, step));
            painter.rect_filled(r.intersect(rect), 0.0, color);
        }
    }
}

/// A dashed border: an empty slot of the table (egui has no dash stroke).
fn dashed(painter: &egui::Painter, rect: Rect, color: Color32) {
    let (dash, gap) = (3.0, 2.0);
    for (y, x0, x1) in [(rect.top(), rect.left(), rect.right()), (rect.bottom(), rect.left(), rect.right())] {
        let mut x = x0;
        while x < x1 {
            painter.line_segment([pos2(x, y), pos2((x + dash).min(x1), y)], Stroke::new(1.0, color));
            x += dash + gap;
        }
    }
    for (x, y0, y1) in [(rect.left(), rect.top(), rect.bottom()), (rect.right(), rect.top(), rect.bottom())] {
        let mut y = y0;
        while y < y1 {
            painter.line_segment([pos2(x, y), pos2(x, (y + dash).min(y1))], Stroke::new(1.0, color));
            y += dash + gap;
        }
    }
}

/// Store `colors`' transparent entry index in the field `key`'s dialog (`None` clears it).
pub fn set_transparent(fields: &mut DialogFields, transparent: Option<u64>) {
    match transparent {
        Some(t) => fields.set(TRANSPARENT, FieldValue::Int(t as i64)),
        None => {
            fields.remove(TRANSPARENT);
        }
    }
}

/// The open document's colour table: its entries and its transparent entry index.
pub fn document_table(app: &PhotocraftApp) -> Option<(Vec<[u8; 3]>, Option<u64>)> {
    let t = app.session.active()?.doc.color_table.clone()?;
    Some((t.colors, t.transparent.map(u64::from)))
}

/// The table Palette: Custom… opens on: the document's own table when it is Indexed, else the
/// Color Table dialog's default ramp (Black Body), so the grid always has entries to edit.
pub fn opening_table(app: &PhotocraftApp) -> Vec<[u8; 3]> {
    document_table(app).map(|(colors, _)| colors).or_else(|| photocraft_engine::mode_cmds::preset_table("blackBody")).unwrap_or_default()
}

/// An `.act` table or a palette PNG's entries, by the file's extension.
pub fn parse_file(name: &str, bytes: &[u8]) -> Result<(Vec<[u8; 3]>, Option<u64>), String> {
    match std::path::Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("act") => {
            let (hex, transparent) = photocraft_engine::swatch_cmds::act_table(bytes).map_err(|e| format!("{name}: {e}"))?;
            let colors = hex.iter().filter_map(|c| unhex(c)).collect();
            Ok((colors, transparent))
        }
        Some("png") => match photocraft_codecs::png_palette(bytes).map_err(|e| format!("{name}: {e}"))? {
            Some(p) => Ok((p.colors, p.transparent.map(u64::from))),
            None => Err(format!("{name}: not a palette PNG (colour type 3)")),
        },
        _ => Err(format!("{name}: expected a .act or palette .png file")),
    }
}

/// The file's bytes for the table `colors` with its transparent entry: a palette PNG when the path
/// ends in `.png` (a 1-pixel-tall indexed image carrying the table and its tRNS), else an `.act`.
pub fn encode_file(path: &str, colors: &[[u8; 3]], transparent: Option<u64>) -> Result<Vec<u8>, String> {
    match std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => {
            let indices: Vec<u8> = (0..colors.len()).map(u8::try_from).collect::<Result<_, _>>().map_err(|_| "a palette has at most 256 entries")?;
            let t = transparent.and_then(|t| u8::try_from(t).ok());
            photocraft_codecs::encode_png_indexed(colors.len() as u32, 1, &indices, colors, t, None, None).map_err(|e| e.to_string())
        }
        _ => photocraft_engine::swatch_cmds::act_bytes(colors, transparent.and_then(|t| u16::try_from(t).ok())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_through_the_field() {
        let colors = vec![[255, 0, 0], [0, 128, 255]];
        let mut f = DialogFields::new();
        set_table(&mut f, CUSTOM_FIELD, &colors);
        assert_eq!(table_rgb(&f, CUSTOM_FIELD), Some(colors));
        // Anything that is not hex is no table at all, so the row never panics on it.
        let mut bad = DialogFields::new();
        bad.set(CUSTOM_FIELD, FieldValue::List(vec![FieldValue::Text("nope".into())]));
        assert_eq!(table_rgb(&bad, CUSTOM_FIELD), None);
    }

    #[test]
    fn a_picked_colour_replaces_its_entry_and_drops_the_private_field() {
        let mut f = DialogFields::new();
        set_table(&mut f, CUSTOM_FIELD, &[[1, 2, 3], [4, 5, 6]]);
        f.set(pick_key(1), FieldValue::Color([1.0, 0.0, 0.0]));
        apply_picks(&mut f, CUSTOM_FIELD);
        assert_eq!(table_rgb(&f, CUSTOM_FIELD), Some(vec![[1, 2, 3], [255, 0, 0]]));
        assert!(!f.contains(&pick_key(1)), "the answer is consumed");
        // An index past the table is dropped, never grown.
        f.set(pick_key(9), FieldValue::Color([0.0, 1.0, 0.0]));
        apply_picks(&mut f, CUSTOM_FIELD);
        assert_eq!(table_rgb(&f, CUSTOM_FIELD), Some(vec![[1, 2, 3], [255, 0, 0]]));
    }

    /// A click edits the entry's colour — on an empty slot after growing the table with opaque
    /// black, never a transparent colour — and the Color Picker opens on that colour.
    #[test]
    fn a_click_edits_a_colour_and_an_empty_slot_grows_with_black() {
        let colors = vec![[1, 2, 3], [4, 5, 6], [7, 8, 9]];
        let mut f = DialogFields::new();
        set_table(&mut f, CUSTOM_FIELD, &colors);
        // An entry's own colour is what the picker opens on, and the table is unchanged.
        assert_eq!(click_entry(crate::color_table_ui::INDEXED_COLOR, &mut f, &colors, 1), Some([4, 5, 6]));
        assert_eq!(table_rgb(&f, CUSTOM_FIELD), Some(colors.clone()));
        // An empty slot grows the table with opaque black up to it, and the picker opens on that
        // black — not on a transparent colour.
        assert_eq!(click_entry(crate::color_table_ui::INDEXED_COLOR, &mut f, &colors, 5), Some([0, 0, 0]));
        assert_eq!(table_rgb(&f, CUSTOM_FIELD), Some(vec![[1, 2, 3], [4, 5, 6], [7, 8, 9], [0, 0, 0], [0, 0, 0], [0, 0, 0]]));
        // Editing Color Table's grid makes a preset table Custom; Indexed Color has no preset.
        let mut table = DialogFields::new();
        table.set("table", FieldValue::Text("grayscale".into()));
        click_entry(crate::color_table_ui::COMMAND, &mut table, &colors, 0);
        assert_eq!(table.text("table"), Some("custom"));
        assert_eq!(table_rgb(&table, COLORS_FIELD), Some(colors));
    }

    /// The grid marks the dialog's own table and its transparent entry, and a preset's table
    /// instead once one is chosen.
    #[test]
    fn the_grid_shows_the_tables_own_transparent_entry() {
        let mut f = DialogFields::new();
        set_table(&mut f, COLORS_FIELD, &[[1, 2, 3]]);
        set_transparent(&mut f, Some(0));
        assert_eq!(shown(&f, crate::color_table_ui::COMMAND, COLORS_FIELD), (vec![[1, 2, 3]], Some(0)));
        // A preset has its own 256 entries and no transparent one.
        f.set("table", FieldValue::Text("grayscale".into()));
        assert_eq!(shown(&f, crate::color_table_ui::COMMAND, COLORS_FIELD).1, None);
        assert_eq!(shown(&f, crate::color_table_ui::COMMAND, COLORS_FIELD).0.len(), 256);
        // Indexed Color's Custom palette is the `paletteColors` field.
        let mut idx = DialogFields::new();
        set_table(&mut idx, CUSTOM_FIELD, &[[10, 20, 30], [40, 50, 60]]);
        assert_eq!(shown(&idx, crate::color_table_ui::INDEXED_COLOR, CUSTOM_FIELD), (vec![[10, 20, 30], [40, 50, 60]], None));
    }

    #[test]
    fn files_round_trip_both_formats() {
        let colors = vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]];
        let act = encode_file("Table.act", &colors, Some(1)).unwrap();
        assert_eq!(parse_file("Table.ACT", &act).unwrap(), (colors.clone(), Some(1)));
        let png = encode_file("Table.png", &colors, Some(2)).unwrap();
        assert_eq!(parse_file("Table.png", &png).unwrap(), (colors, Some(2)));
        assert!(parse_file("Table.aco", &act).is_err(), "only .act and palette .png files");
        assert!(parse_file("Table.png", &act).is_err(), "an .act is not a palette PNG");
    }

    /// The grid is a colour table's 16×16 slots whatever the table's length, so an entry's position
    /// is its index: cell 0 is the first and cell 255 the last, and a click outside is none.
    #[test]
    fn the_grid_maps_every_cell_of_the_16_by_16_table() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(CELL * COLS as f32, CELL * COLS as f32));
        assert_eq!(cell_at(rect, pos2(1.0, 1.0)), Some(0));
        assert_eq!(cell_at(rect, pos2(CELL * 15.0 + 1.0, 1.0)), Some(15));
        assert_eq!(cell_at(rect, pos2(1.0, CELL * 15.0 + 1.0)), Some(240));
        assert_eq!(cell_at(rect, pos2(CELL * 15.0 + 1.0, CELL * 15.0 + 1.0)), Some(255));
        assert_eq!(cell_at(rect, pos2(-1.0, 1.0)), None, "left of the table");
        assert_eq!(cell_at(rect, pos2(1.0, rect.bottom() + 1.0)), None, "below the table");
        // The last slot is the 256th entry, never a 257th.
        assert_eq!(CELLS, 256);
    }

    /// The pipette's match: an exact entry after rounding, else the nearest by squared distance (a
    /// dithered edge's colour), and nothing for an empty table.
    #[test]
    fn the_pipette_matches_an_entry_exactly_then_by_distance() {
        let colors = [[255, 0, 0], [0, 128, 0], [10, 20, 30]];
        assert_eq!(match_entry(&colors, [1.0, 0.0, 0.0]), Some(0));
        assert_eq!(match_entry(&colors, [0.0, 128.0 / 255.0, 0.0]), Some(1));
        assert_eq!(match_entry(&colors, [0.05, 0.1, 0.12]), Some(2), "closest to the dark entry");
        assert_eq!(match_entry(&colors, [0.9, 0.1, 0.1]), Some(0), "closer to red than to the dark entry");
        assert_eq!(match_entry(&colors, [5.0, -3.0, 0.0]), Some(0), "out-of-range channels clamp");
        assert_eq!(match_entry(&[], [0.5, 0.5, 0.5]), None, "an empty table matches nothing");
    }
}
