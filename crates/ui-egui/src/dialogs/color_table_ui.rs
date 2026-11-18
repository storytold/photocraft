//! Image › Mode › Color Table… and Image › Mode › Indexed Color… (Palette: Custom…): the editable
//! palette grid, Load… and Save… for `.act` color tables and palette PNGs.
//!
//! The dialog itself is the filter one (`filter_dialog`); this module gives it the table row
//! ([`rows`]), the OK params rule ([`command_params`]) and the footer buttons' actions
//! ([`footer_action`]). Load fills the pending table from a picked `.act` or palette PNG (OK then
//! applies it); Save writes the pending table — the loaded colours while the Table menu is on
//! Custom, else its preset, else the document's table — to a picked `.act` or `.png`. Picking a
//! preset after Load… drops the loaded colours at OK. The grid, the file formats and the picker
//! plumbing live in [`crate::palette_file_ui`]; parsing lives in the engine (`swatch_cmds`) and in
//! `photocraft_codecs` for PNG.

use craft::ui::dialog::DialogId;
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::palette_file_ui;

/// The Color Table command whose dialog this module completes.
pub const COMMAND: &str = "image.mode.colorTable";
/// The Indexed Color command whose Custom palette this module completes.
pub const INDEXED_COLOR: &str = "image.mode.indexedColor";

/// The pending table: colours and the transparent entry.
type Table = (Vec<[u8; 3]>, Option<u64>);

/// Whether the Table menu is on Custom (or unset), the only state where loaded colors apply.
fn custom(fields: &DialogFields) -> bool {
    fields.text("table").is_none_or(|t| t == "custom")
}

fn doc_table(app: &PhotocraftApp) -> Option<Table> {
    let t = app.session.active()?.doc.color_table.clone()?;
    Some((t.colors, t.transparent.map(u64::from)))
}

/// The pending table: the dialog's loaded colours (while the Table menu is on Custom), else its
/// preset, else `document`'s table.
fn pending_over(fields: &DialogFields, document: Option<Table>) -> Option<Table> {
    if custom(fields)
        && let Some(colors) = palette_file_ui::table_rgb(fields, palette_file_ui::COLORS_FIELD)
    {
        let colors = (!colors.is_empty() && colors.len() <= photocraft_engine::swatch_cmds::ACT_COLORS).then_some(colors)?;
        let transparent = fields.int(palette_file_ui::TRANSPARENT).and_then(|i| u64::try_from(i).ok());
        return Some((colors, transparent));
    }
    if let Some(name) = fields.text("table")
        && let Some(colors) = photocraft_engine::mode_cmds::preset_table(name)
    {
        return Some((colors, None));
    }
    document
}

fn pending(app: &PhotocraftApp, fields: &DialogFields) -> Option<Table> {
    pending_over(fields, doc_table(app))
}

/// The table rows of the filter dialog: the "Set Transparent" switch and the table's 16×16 grid
/// (seeded from the document by `filter_dialog::fields` when the dialog opens).
pub fn rows(_app: &PhotocraftApp) -> Vec<Param> {
    vec![palette_file_ui::armed_row(), palette_file_ui::grid_row(COMMAND, Label::from(tl!("Colors")))]
}

/// The `image.mode.colorTable` params at OK: a preset picked after Load… replaces the loaded
/// table, so its colours and transparent entry are dropped unless the Table menu is on Custom.
pub fn command_params(fields: &DialogFields, mut params: Value) -> Value {
    if !custom(fields)
        && let Some(o) = params.as_object_mut()
    {
        o.remove(palette_file_ui::COLORS_FIELD);
        o.remove(palette_file_ui::TRANSPARENT);
    }
    params
}

/// The `image.mode.indexedColor` params at OK: `paletteColors` only belongs to the Custom palette.
/// (The grid's private keys, `__…`, never reach `command_params` at all.)
pub fn indexed_color_params(_fields: &DialogFields, mut params: Value) -> Value {
    if let Some(o) = params.as_object_mut()
        && o.get("palette").and_then(Value::as_str) != Some("custom")
    {
        o.remove(palette_file_ui::CUSTOM_FIELD);
        o.remove(palette_file_ui::TRANSPARENT);
    }
    params
}

use crate::dialogs::alerts::report_error;

/// The command a dialog id belongs to (`image.mode.colorTable`, `image.mode.indexedColor`, …).
fn command_of(app: &PhotocraftApp, id: DialogId) -> String {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|d| d.key().map(str::to_owned)).unwrap_or_default()
}

/// The table the dialog `id` would save: the loaded colours while Custom, else the preset, else the
/// document's table (Color Table), or the Custom palette field (Indexed Color).
fn save_table(app: &PhotocraftApp, command: &str, fields: &DialogFields) -> Option<Table> {
    if command == COMMAND {
        return pending(app, fields);
    }
    let colors = palette_file_ui::table_rgb(fields, palette_file_ui::CUSTOM_FIELD)?;
    let transparent = fields.int(palette_file_ui::TRANSPARENT).and_then(|i| u64::try_from(i).ok());
    Some((colors, transparent))
}

/// A Load… or Save… footer button of the dialog `id` was pressed (`action` is `"load"` or `"save"`).
pub fn footer_action(app: &mut PhotocraftApp, _ctx: &egui::Context, id: DialogId, action: &str) {
    let command = command_of(app, id);
    match action {
        "load" => {
            if let Err(e) = app.pick_file_bytes_filtered(&["act", "png"], move |app, name, bytes| {
                let (colors, transparent) = palette_file_ui::parse_file(&name, &bytes)?;
                // The dialog may close while the picker is open; then there is nothing to fill.
                let Some(d) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return Ok(Value::Null) };
                if command == COMMAND {
                    d.set_field("table", FieldValue::Text("custom".into()));
                }
                d.set_field(
                    palette_file_ui::colors_field(&command),
                    FieldValue::List(palette_file_ui::hex_array(&colors).into_iter().map(FieldValue::Text).collect()),
                );
                match transparent.and_then(|i| i64::try_from(i).ok()) {
                    Some(i) => d.set_field(palette_file_ui::TRANSPARENT, FieldValue::Int(i)),
                    None => {
                        d.clear_field(palette_file_ui::TRANSPARENT);
                    }
                }
                Ok(Value::Null)
            }) && e != crate::file_dialog::CANCELLED
            {
                report_error(app, e);
            }
        }
        "save" => {
            let Some(fields) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone()) else { return };
            let Some((colors, transparent)) = save_table(app, &command, &fields) else {
                return report_error(app, tl!("There is no color table to save."));
            };
            let n = colors.len() as u64;
            let default_name = if command == COMMAND { "Table.act" } else { "Palette.act" };
            if let Err(e) = app.pick_save(default_name, move |app, path| {
                let bytes = palette_file_ui::encode_file(&path, &colors, transparent)?;
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, &bytes)?;
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(
                    &crate::i18n::trn(crate::i18n::current(), n, "Saved {n} color to {file}", "Saved {n} colors to {file}"),
                    &[("file", &crate::file_open::display_name(&path))],
                );
                Ok(json!({"path": path}))
            }) {
                report_error(app, e);
            }
        }
        _ => {}
    }
}

/// The notice an Indexed Color or Color Table run posts when it flattened clear pixels.
pub fn warn_flattened(app: &mut PhotocraftApp, command: &str, result: &Value) {
    let Some(n) = result.get("flattened").and_then(Value::as_u64).filter(|n| *n > 0) else { return };
    let what = if command == COMMAND { "Color Table" } else { "Indexed Color" };
    crate::notices::post(
        app,
        tl!("Transparency flattened").to_string(),
        vec![format!("{what}: {n} transparent pixels became opaque.")],
        craft::theme::variant::Tone::Warn,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    }

    fn loaded_fields(table: &str) -> DialogFields {
        let mut f = DialogFields::new();
        f.set("table", FieldValue::Text(table.into()));
        palette_file_ui::set_table(&mut f, palette_file_ui::COLORS_FIELD, &[[255, 0, 0], [0, 255, 0]]);
        f.set(palette_file_ui::TRANSPARENT, FieldValue::Int(1));
        f
    }

    #[test]
    fn pending_prefers_dialog_colors_then_preset() {
        let app = app();
        // Loaded colors win while the Table menu is on Custom, with their transparent entry.
        assert_eq!(pending(&app, &loaded_fields("custom")), Some((vec![[255, 0, 0], [0, 255, 0]], Some(1))));
        // A preset picked after Load… is what Save… writes, not the loaded colors.
        let (colors, transparent) = pending(&app, &loaded_fields("blackBody")).unwrap();
        assert_eq!((colors.len(), transparent), (256, None));
        // Else the preset (256 entries, no transparency).
        let mut fields = DialogFields::new();
        fields.set("table", FieldValue::Text("blackBody".into()));
        let (colors, transparent) = pending(&app, &fields).unwrap();
        assert_eq!((colors.len(), transparent), (256, None));
        assert_eq!(colors[0], [0, 0, 0]);
        // Nothing pending without colors, preset or document table.
        let mut custom = DialogFields::new();
        custom.set("table", FieldValue::Text("custom".into()));
        assert_eq!(pending(&app, &custom), None);
    }

    /// What OK sends after a preset is picked has no loaded colors (which `image.mode.colorTable`
    /// would apply over the preset); Custom keeps them.
    #[test]
    fn a_preset_picked_after_load_drops_the_loaded_colors() {
        let sent = json!({"table": "grayscale", "colors": ["#ff0000"], "transparent": 0});
        let dropped = command_params(&loaded_fields("grayscale"), sent.clone());
        assert_eq!(dropped, json!({"table": "grayscale"}));
        let kept = json!({"table": "custom", "colors": ["#ff0000"], "transparent": 0});
        assert_eq!(command_params(&loaded_fields("custom"), kept.clone()), kept);
    }

    /// Indexed Color's OK keeps the Custom table only while Palette is Custom.
    #[test]
    fn indexed_color_params_keep_the_table_only_for_custom() {
        let sent = json!({"palette": "selective", "paletteColors": ["#ff0000"], "transparent": 0});
        assert_eq!(indexed_color_params(&DialogFields::new(), sent), json!({"palette": "selective"}));
        let custom = json!({"palette": "custom", "paletteColors": ["#ff0000"], "transparent": 0});
        assert_eq!(indexed_color_params(&DialogFields::new(), custom), json!({"palette": "custom", "paletteColors": ["#ff0000"], "transparent": 0}));
    }
}
