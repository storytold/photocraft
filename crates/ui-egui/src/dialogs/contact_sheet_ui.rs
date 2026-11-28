//! File › Automate › Contact Sheet II…, laid out like the reference application's dialog (#13683):
//! the sections Source Images, Document, Thumbnails and Use Filename as Caption, top to bottom.
//!
//! The rows are the command's params, so OK, `ui.dialog.set` and `ui.dialog.confirm` work as for every form. Three
//! of them are kept in the shape the dialog shows and turned into the command's at OK ([`command_params`]): the
//! resolution's unit (the command takes pixels/inch), Bit Depth (`"8"` or `"16"`) and Place (`"across"` or `"down"`).

use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, NumberUnit, Param, ParamKind};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::requests::PathKind;
use photocraft_engine::prefs::Unit;
use serde_json::{Map, Value, json};

pub(crate) const COMMAND: &str = "file.automate.contactSheetII";

const UNITS: [(&str, &str); 3] = [("inches", "inches"), ("cm", "cm"), ("pixels", "pixels")];
const MODES: [(&str, &str); 4] = [("gray", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
const DEPTHS: [(&str, &str); 2] = [("8", "8-bit"), ("16", "16-bit")];
const PLACES: [(&str, &str); 2] = [("across", "across first"), ("down", "down first")];

/// Choice options whose labels are `tl!` literals here.
fn opts(list: &[(&'static str, &'static str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| ((*id).to_owned(), Label::from(tl!(label)))).collect()
}

/// The rows of the dialog, opening on the folder `dir`.
pub(crate) fn rows(dir: String) -> Vec<Param> {
    let resolution = vec![
        NumberUnit::new("in", UnitKind::PixelsPerInch, tl!("pixels/inch"), 1.0),
        NumberUnit::new("cm", UnitKind::PixelsPerCentimetre, tl!("pixels/cm"), 2.54),
    ];
    vec![
        Param::section(tl!("Source Images")),
        Param::new("input", tl!("Input"), ParamKind::text(dir)).browse(PathKind::Folder),
        // The folder, then Include Subfolders (on) and, right below it, Group Images by Folder (off, greyed while
        // Include Subfolders is off).
        Param::new("includeSubfolders", tl!("Include Subfolders"), ParamKind::bool(true)),
        Param::new("groupByFolder", tl!("Group Images by Folder"), ParamKind::bool(false))
            .enabled_when("includeSubfolders", true)
            .tooltip(tl!("Images in subfolders will start on a new Contact Sheet")),
        Param::section(tl!("Document")),
        Param::new("units", tl!("Units"), ParamKind::choice(opts(&UNITS), Some("inches".into()))),
        Param::custom(UNITS_SHOWN, Label::Empty, units_row).full_width(),
        Param::new("width", tl!("Width"), ParamKind::number(Some(0.0), Some(300_000.0), 8.0, 0.5, 2)),
        Param::new("height", tl!("Height"), ParamKind::number(Some(0.0), Some(300_000.0), 10.0, 0.5, 2)),
        Param::number_with_units("resolution", tl!("Resolution"), 300.0, 1.0, 2, resolution).bounds(Some(1.0), Some(10_000.0)),
        Param::new("flatten", tl!("Flatten All Layers"), ParamKind::bool(false)),
        Param::new("mode", tl!("Mode"), ParamKind::choice(opts(&MODES), Some("rgb".into()))),
        Param::new("depth", tl!("Bit Depth"), ParamKind::choice(opts(&DEPTHS), Some("8".into()))),
        Param::section(tl!("Thumbnails")),
        Param::new("place", tl!("Place"), ParamKind::choice(opts(&PLACES), Some("across".into()))),
        Param::new("columns", tl!("Columns"), ParamKind::int(Some(1), Some(100), 5)),
        Param::new("rows", tl!("Rows"), ParamKind::int(Some(1), Some(100), 6)),
        Param::new("rotateForBestFit", tl!("Rotate For Best Fit"), ParamKind::bool(false)),
        Param::new("autoSpacing", tl!("Use Auto-Spacing"), ParamKind::bool(true)),
        Param::section(tl!("Use Filename as Caption")),
        Param::new("caption", tl!("Use Filename as Caption"), ParamKind::bool(true)),
        Param::new("font", tl!("Font"), ParamKind::text(photocraft_text::fonts::DEFAULT_FAMILY)).enabled_when("caption", true),
        Param::new("fontSize", tl!("Font size"), ParamKind::number(Some(1.0), None, 12.0, 1.0, 1)).unit(UnitKind::Points).enabled_when("caption", true),
    ]
}

/// The units Width and Height were last shown in, kept by [`follow_units`].
const UNITS_SHOWN: &str = "unitsShown";

/// The row of [`follow_units`]: it shows nothing, so Units stays a plain list and the dialog remembers it with the
/// size. Following here converts in the pass that picked the unit.
fn units_row(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    follow_units(fields);
    ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())
}

/// Follows the Units list: switching it converts Width and Height at the resolution, as the command reads them, so
/// the sheet keeps its size: 8 x 10 in is 20.32 x 25.4 cm, and 2400 x 3000 px at 300 ppi. The form's builder runs it
/// too, before each pass, OK and automation write, so an OK right after `ui.dialog.set units` sends the converted size.
pub(crate) fn follow_units(fields: &mut DialogFields) {
    // The first build runs before the defaults are in: nothing to follow yet.
    let Some(units) = fields.text("units").map(str::to_owned) else { return };
    if let Some(shown) = fields.text(UNITS_SHOWN).filter(|shown| *shown != units).map(str::to_owned) {
        let ppi = fields.number("resolution").unwrap_or(300.0);
        let (from, to) = (unit_of(&shown), unit_of(&units));
        for (key, default) in [("width", 8.0), ("height", 10.0)] {
            // The command reads no size or 0 as 8 x 10 in the units it is given, so that is the size to convert.
            let shown = fields.number(key).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(default);
            let v = to.from_px(from.to_px(shown, ppi, 0.0, 72.0), ppi, 0.0, 72.0);
            // Kept unrounded (the field shows two decimals): a small pixel size at a high resolution would round to
            // 0 in, which the command reads as no size. Pixels are whole.
            fields.set(key, if to == Unit::Pixels { v.round() } else { v });
        }
    }
    if fields.text(UNITS_SHOWN) != Some(units.as_str()) {
        fields.set(UNITS_SHOWN, units);
    }
}

/// The unit a `units` value stands for, read as the command reads it (inches when unknown).
fn unit_of(units: &str) -> Unit {
    match units {
        "cm" => Unit::Centimeters,
        "mm" => Unit::Millimeters,
        "px" | "pixels" => Unit::Pixels,
        _ => Unit::Inches,
    }
}

/// The dialog's values as the command takes them: the resolution in pixels/inch without its unit, `depth` a number
/// and Place as `placeAcrossFirst`.
pub(crate) fn command_params(map: &mut Map<String, Value>) {
    map.remove("resolution.unit");
    map.remove(UNITS_SHOWN);
    if let Some(depth) = map.get("depth").and_then(Value::as_str).and_then(|d| d.parse::<u64>().ok()) {
        map.insert("depth".into(), json!(depth));
    }
    if let Some(place) = map.remove("place") {
        map.insert("placeAcrossFirst".into(), json!(place.as_str() != Some("down")));
    }
}

#[cfg(test)]
#[path = "contact_sheet_layout_tests.rs"]
mod contact_sheet_layout_tests;
