//! Image › Mode › Color Table…: Load… and Save… for `.act` color tables, as in Photoshop.
//!
//! The dialog itself is the filter one (`filter_dialog`); this module gives it the colour table
//! row ([`rows`]), the OK params rule ([`command_params`]) and the footer buttons' actions
//! ([`footer_action`]). Load fills the pending table from a picked `.act` (OK then applies it);
//! Save writes the pending table — the loaded colors while the Table menu is on Custom, else its
//! preset, else the document's table — to a picked `.act`. Picking a preset after Load… drops the
//! loaded colors at OK. Parsing lives in the engine (`swatch_cmds`).

use craft::theme::Srgba;
use craft::ui::choice::SwatchTable;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The pending table: colours and the transparent entry.
type Table = (Vec<[u8; 3]>, Option<u16>);

fn unhex(s: &str) -> Option<[u8; 3]> {
    let f = crate::color_picker_ui::parse_hex(s)?;
    Some(f.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

/// Whether the Table menu is on Custom (or unset), the only state where loaded colors apply.
fn custom(fields: &DialogFields) -> bool {
    fields.text("table").is_none_or(|t| t == "custom")
}

/// The loaded colours of the dialog's `colors` value, as bytes; `None` when it has none.
fn loaded(fields: &DialogFields) -> Option<Vec<Option<[u8; 3]>>> {
    let list = fields.list("colors")?;
    Some(list.iter().map(|v| if let FieldValue::Text(s) = v { unhex(s) } else { None }).collect())
}

fn doc_table(app: &PhotocraftApp) -> Option<Table> {
    let t = app.session.active()?.doc.color_table.clone()?;
    Some((t.colors, t.transparent.map(u16::from)))
}

/// The pending table: the dialog's loaded colors (while the Table menu is on Custom), else its
/// preset, else `document`'s table.
fn pending_over(fields: &DialogFields, document: Option<Table>) -> Option<Table> {
    if custom(fields)
        && let Some(colors) = loaded(fields)
    {
        let colors: Option<Vec<[u8; 3]>> = colors.into_iter().collect();
        let colors = colors.filter(|c| !c.is_empty() && c.len() <= photocraft_engine::swatch_cmds::ACT_COLORS)?;
        let transparent = fields.int("transparent").and_then(|i| u16::try_from(i).ok());
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

/// The Color Table row of the filter dialog: the pending table as touching swatches. The document's
/// table is snapshotted here, as the row cannot read the app.
pub fn rows(app: &PhotocraftApp) -> Vec<Param> {
    let document = doc_table(app);
    vec![Param::custom("colors", tl!("Colors"), move |ui, fields, _cx| {
        let shown = pending_over(fields, document.clone()).map(|(colors, _)| colors).unwrap_or_default();
        let entries: Vec<(Label, Srgba)> = shown
            .iter()
            .enumerate()
            .map(|(i, [r, g, b])| {
                (Label::from(format!("{i} #{r:02x}{g:02x}{b:02x}")), Srgba::rgb(f32::from(*r) / 255.0, f32::from(*g) / 255.0, f32::from(*b) / 255.0))
            })
            .collect();
        SwatchTable::new(tl!("Colors"), &entries).per_row(16).tips(true).show_all(ui).response
    })]
}

/// The `image.mode.colorTable` params at OK: a preset picked after Load… replaces the loaded
/// table, so its colors and transparent entry are dropped unless the Table menu is on Custom.
pub fn command_params(fields: &DialogFields, mut params: Value) -> Value {
    if !custom(fields)
        && let Some(o) = params.as_object_mut()
    {
        o.remove("colors");
        o.remove("transparent");
    }
    params
}

use crate::dialogs::alerts::report_error;

/// A Load… or Save… footer button of the dialog `id` was pressed (`action` is `"load"` or `"save"`).
pub fn footer_action(app: &mut PhotocraftApp, _ctx: &egui::Context, id: DialogId, action: &str) {
    match action {
        "load" => {
            if let Err(e) = app.pick_file_bytes_filtered(&["act"], move |app, name, bytes| {
                let (colors, transparent) =
                    photocraft_engine::swatch_cmds::act_table(&bytes).map_err(|e| format!("{}: {e}", crate::file_open::display_name(&name)))?;
                // The dialog may close while the picker is open; then there is nothing to fill.
                let Some(d) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return Ok(Value::Null) };
                d.set_field("table", FieldValue::Text("custom".into()));
                d.set_field("colors", FieldValue::List(colors.into_iter().map(FieldValue::Text).collect()));
                match transparent.and_then(|i| i64::try_from(i).ok()) {
                    Some(i) => d.set_field("transparent", FieldValue::Int(i)),
                    None => d.clear_field("transparent"),
                }
                Ok(Value::Null)
            }) && e != crate::file_dialog::CANCELLED
            {
                report_error(app, e);
            }
        }
        "save" => {
            let Some(fields) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone()) else { return };
            let Some((colors, transparent)) = pending(app, &fields) else {
                return report_error(app, tl!("There is no color table to save."));
            };
            let bytes = match photocraft_engine::swatch_cmds::act_bytes(&colors, transparent) {
                Ok(bytes) => bytes,
                Err(e) => return report_error(app, e),
            };
            let n = colors.len() as u64;
            if let Err(e) = app.pick_save("Table.act", move |app, path| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    }

    fn loaded_fields(table: &str) -> DialogFields {
        let mut f = DialogFields::new();
        f.set("table", FieldValue::Text(table.into()));
        f.set("colors", FieldValue::List(vec![FieldValue::Text("#ff0000".into()), FieldValue::Text("#00ff00".into())]));
        f.set("transparent", FieldValue::Int(1));
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
}
