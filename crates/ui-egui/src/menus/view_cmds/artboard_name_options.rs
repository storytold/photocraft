//! Artboards to Files' Include Artboard Name (#13637): a checkbox, unticked, kept per file type,
//! and while it is ticked the Name Options section (Font, Style, Size, Canvas Extension, Text).
//! The Canvas Extension Color swatch shows only with Other... chosen.

use super::{Param, ParamKind, Value, choice, json, named, opts};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use serde_json::Map;

pub(super) const COMMAND: &str = "file.export.artboardsToFiles";
/// The checkbox, kept per file type.
const INCLUDE: &str = "includeName";
/// The tick of each file type the dialog has shown, `{format: bool}`: shell state, never a command parameter.
const BY_FORMAT: &str = "__includeNameByFormat";
/// The file type the checkbox's value belongs to now.
const FORMAT_SEEN: &str = "__includeNameFormat";
/// Whether the Canvas Extension Color row shows (the checkbox ticked and Other... chosen): shell state the row is
/// shown by, as a row takes one `visible_when` condition.
const OTHER_SHOWN: &str = "__extensionColorShown";

/// Before each pass, OK and automation write: a File Type changed since the last one puts the checkbox's value under
/// the type it was set for and brings back the new type's (unticked for a type not shown yet), so an OK right after a
/// control client picked another type sends that type's own tick. Then sets the colour row's shell key.
pub(super) fn derive(fields: &mut DialogFields) {
    // The first build runs before the defaults are in: nothing to keep yet.
    let Some(format) = fields.text("format").map(str::to_owned) else { return };
    let seen = fields.text(FORMAT_SEEN).map(str::to_owned);
    if seen.as_deref() != Some(format.as_str()) {
        let mut kept = match fields.get(BY_FORMAT) {
            Some(FieldValue::Json(Value::Object(kept))) => kept.clone(),
            _ => Map::new(),
        };
        if let Some(seen) = seen {
            kept.insert(seen, json!(fields.bool(INCLUDE).unwrap_or(false)));
        }
        let on = kept.get(&format).and_then(Value::as_bool).unwrap_or(false);
        fields.set(INCLUDE, on);
        fields.set(BY_FORMAT, FieldValue::Json(Value::Object(kept)));
        fields.set(FORMAT_SEEN, format);
    }
    let other = fields.bool(INCLUDE) == Some(true) && fields.text("extension") == Some("other");
    fields.set(OTHER_SHOWN, other);
}

/// The checkbox and the Name Options rows, which show only while it is ticked. `fonts` are the installed families.
pub(super) fn rows(fonts: &[String]) -> Vec<Param> {
    let mut fonts = fonts.to_vec();
    if !fonts.iter().any(|f| f == "Arial") {
        fonts.insert(0, "Arial".into());
    }
    let shown = |row: Param| row.visible_when(INCLUDE, true);
    let styles =
        opts(&[("Regular", tl!("Regular")), ("Italic", tl!("Italic")), ("Bold", tl!("Bold")), ("Bold Italic", tl!("Bold Italic")), ("Black", tl!("Black"))]);
    let extensions = opts(&[("white", tl!("White")), ("black", tl!("Black")), ("other", tl!("Other..."))]);
    vec![
        // Opens unticked, as the per-type ticks start over at every open.
        Param::new(INCLUDE, tl!("Include Artboard Name"), ParamKind::bool(false)).not_remembered(),
        shown(Param::section(tl!("Name Options"))),
        // Font names are the fonts' own, never catalogue entries.
        shown(choice("font", tl!("Font"), named(&fonts), "Arial").searchable()),
        shown(choice("fontStyle", tl!("Style"), styles, "Regular").tooltip(tl!("Select the font style"))),
        shown(Param::new("fontSize", tl!("Size (pt)"), ParamKind::text("12"))),
        shown(choice("extension", tl!("Canvas Extension"), extensions, "white")),
        Param::new("extensionColor", tl!("Canvas Extension Color"), ParamKind::color(Some([0.5, 0.5, 0.5]))).visible_when(OTHER_SHOWN, true),
        shown(Param::new("textColor", tl!("Text"), ParamKind::color(Some([0.0, 0.0, 0.0])))),
    ]
}

/// The command params of the form's values `map`: the Name Options only while the checkbox is ticked (hidden rows keep
/// their values), the extension colour only with Other... chosen.
pub(crate) fn strip(map: &mut Map<String, Value>) {
    if map.get(INCLUDE).and_then(Value::as_bool) != Some(true) {
        for key in ["font", "fontStyle", "fontSize", "extension", "extensionColor", "textColor"] {
            map.remove(key);
        }
    } else if map.get("extension").and_then(Value::as_str) != Some("other") {
        map.remove("extensionColor");
    }
}
