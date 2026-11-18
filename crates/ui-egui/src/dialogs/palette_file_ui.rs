//! Palette files and the editable palette grid shared by Image › Mode › Indexed Color (Custom…)
//! and Image › Mode › Color Table.
//!
//! The grid is the dialog's table field ([`colors_field`]: `colors` for Color Table,
//! `paletteColors` for Indexed Color's Custom palette) as 16 cells to a row. A click asks the
//! host's Color Picker for that entry ([`pick_key`]); the answer is folded back into the table on
//! the next pass. With the "Set Transparent" switch on, a click marks that entry as the table's
//! transparent one instead (or clears the mark on the entry that has it).
//!
//! Load… and Save… exchange Adobe `.act` colour tables and palette PNGs; parsing and encoding live
//! in [`parse_file`] and [`encode_file`], and the footer buttons are answered by
//! [`crate::color_table_ui::footer_action`].

use craft::theme::Srgba;
use craft::ui::choice::{Checkbox, SwatchTable};
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param};
use craft::ui::runtime::env::Label;

use crate::PhotocraftApp;

/// The Color Table dialog's table field: a list of `"#rrggbb"`.
pub const COLORS_FIELD: &str = "colors";
/// The Indexed Color dialog's Custom palette field (its `colors` param is the entry count).
pub const CUSTOM_FIELD: &str = "paletteColors";
/// The table's transparent entry, as an index.
pub const TRANSPARENT: &str = "transparent";
/// The "Set Transparent" switch: the next grid click marks an entry instead of editing it. Private,
/// so it is dropped from the command's parameters.
pub const TRANSPARENT_ARMED: &str = "transparentArmed";

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

/// The table the grid shows: the dialog's own field, or for Color Table a chosen preset.
fn shown(fields: &DialogFields, command: &str, key: &str, document: Option<&[[u8; 3]]>) -> Vec<[u8; 3]> {
    if command == crate::color_table_ui::COMMAND {
        match fields.text("table").unwrap_or("custom") {
            "custom" => table_rgb(fields, key).or_else(|| document.map(<[_]>::to_vec)).unwrap_or_default(),
            preset => photocraft_engine::mode_cmds::preset_table(preset).unwrap_or_default(),
        }
    } else {
        table_rgb(fields, key).unwrap_or_default()
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

/// The editable palette grid of `command`'s dialog. `document` is the open document's table, which
/// the row cannot read itself; Color Table shows it while its table is Custom with nothing loaded.
pub fn grid_row(command: &'static str, label: Label, document: Option<Vec<[u8; 3]>>) -> Param {
    let key = colors_field(command);
    Param::custom(key, label.clone(), move |ui, fields, cx| {
        apply_picks(fields, key);
        let colors = shown(fields, command, key, document.as_deref());
        let transparent = fields.int(TRANSPARENT).and_then(|t| usize::try_from(t).ok());
        let armed = fields.bool(TRANSPARENT_ARMED).unwrap_or(false);
        let mut switch = armed;
        Checkbox::new(Bind::bound(&mut switch)).label(tl!("Set Transparent").to_string()).show(ui);
        if switch != armed {
            fields.set(TRANSPARENT_ARMED, switch);
        }
        let entries: Vec<(Label, Srgba)> = colors
            .iter()
            .enumerate()
            .map(|(i, [r, g, b])| {
                (Label::from(format!("{i} #{r:02x}{g:02x}{b:02x}")), Srgba::rgb(f32::from(*r) / 255.0, f32::from(*g) / 255.0, f32::from(*b) / 255.0))
            })
            .collect();
        let shown_table = SwatchTable::new(label.clone(), &entries).per_row(16).tips(true).marked(move |i: &usize| Some(*i) == transparent).show_all(ui);
        if let Some(i) = shown_table.selected {
            if armed {
                // Mark the entry the click landed on; clicking the marked entry clears it.
                if transparent == Some(i) {
                    fields.remove(TRANSPARENT);
                } else {
                    materialize(fields, command, key, &colors);
                    fields.set(TRANSPARENT, FieldValue::Int(i as i64));
                }
            } else if let Some((_, color)) = entries.get(i) {
                materialize(fields, command, key, &colors);
                cx.pick_color(&pick_key(i), *color);
            }
        }
        shown_table.response
    })
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

/// The table of the open document, when it is an Indexed one.
pub fn document_table(app: &PhotocraftApp) -> Option<Vec<[u8; 3]>> {
    Some(app.session.active()?.doc.color_table.as_ref()?.colors.clone())
}

/// The table Palette: Custom… opens on: the document's own table when it is Indexed, else the
/// Color Table dialog's default ramp (Black Body), so the grid always has entries to edit.
pub fn opening_table(app: &PhotocraftApp) -> Vec<[u8; 3]> {
    document_table(app).or_else(|| photocraft_engine::mode_cmds::preset_table("blackBody")).unwrap_or_default()
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
}
