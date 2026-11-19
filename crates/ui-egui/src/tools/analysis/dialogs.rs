//! The Set Measurement Scale, Select Data Points and Place Scale Marker dialogs: parameter forms hosted on the dialog
//! host, answered through the `ParamFamily` row of `dialogs.rs`.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use photocraft_doc::Document;
use photocraft_engine::analysis_cmds;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The three analysis dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Image › Analysis › Set Measurement Scale.
    Scale,
    /// Image › Analysis › Select Data Points.
    DataPoints,
    /// Image › Analysis › Place Scale Marker.
    ScaleMarker,
}

impl Kind {
    /// The engine command the dialog answers for; it is also the dialog's key.
    fn command(self) -> &'static str {
        match self {
            Kind::Scale => "image.analysis.setMeasurementScale",
            Kind::DataPoints => "image.analysis.selectDataPoints",
            Kind::ScaleMarker => "image.analysis.placeScaleMarker",
        }
    }

    fn of(key: &str) -> Option<Kind> {
        [Kind::Scale, Kind::DataPoints, Kind::ScaleMarker].into_iter().find(|k| k.command() == key)
    }
}

/// The data-point sources of the Select Data Points dialog, in column order.
fn sources() -> [(&'static str, &'static str); 3] {
    [("selection", tl!("Selections")), ("ruler", tl!("Ruler Tool")), ("count", tl!("Count Tool"))]
}

/// The field key of the check box of column `column` in `source`.
fn point_key(source: &str, column: &str) -> String {
    format!("{source}:{column}")
}

/// A "nice" bar length near a fifth of the image width (same rule as the engine default).
fn suggested_length(d: &Document) -> f64 {
    let target = f64::from(d.size.width) / 5.0 * d.measurement.scale.factor();
    let e = 10f64.powf(target.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|m| m * e).find(|v| *v <= target).unwrap_or(e)
}

/// Opens the dialog of `kind` with the values the document and the session hold now. `Err` when the dialog needs a
/// document and none is open.
pub(crate) fn open(app: &mut PhotocraftApp, kind: Kind) -> Result<DialogId, String> {
    let no_document = || tl!("No document is open.").to_owned();
    let mut fields = DialogFields::new();
    match kind {
        Kind::Scale => {
            let doc = app.session.active().ok_or_else(no_document)?.doc.clone();
            let sc = &doc.measurement.scale;
            fields.set("preset", if sc.is_default() { "default" } else { "custom" });
            fields.set("pixelLength", sc.pixel_length);
            fields.set("logicalLength", sc.logical_length);
            fields.set("units", sc.units.clone());
        }
        Kind::DataPoints => {
            let points = serde_json::to_value(&app.session.analysis.data_points).unwrap_or_default();
            for (source, _) in sources() {
                let chosen: Vec<&str> = points.get(source).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                for column in analysis_cmds::available(source) {
                    fields.set(point_key(source, column), chosen.contains(&column));
                }
            }
        }
        Kind::ScaleMarker => {
            let doc = app.session.active().ok_or_else(no_document)?.doc.clone();
            fields.set("length", suggested_length(&doc));
            fields.set("fontSize", 12.0);
            fields.set("displayText", true);
            fields.set("textPosition", "bottom");
            fields.set("color", "black");
        }
    }
    Ok(params::open_param(app, kind.command(), fields, Value::Null, move |f: &mut DialogFields| form(kind, f)))
}

/// Whether `key` is the key of one of these dialogs.
pub(crate) fn owns(key: &str) -> bool {
    Kind::of(key).is_some()
}

/// The engine command and params OK runs with for `fields`.
pub(crate) fn command(kind: Kind, f: &DialogFields) -> Option<(&'static str, Value)> {
    let id = kind.command();
    Some(match kind {
        Kind::Scale => {
            if f.text("preset") == Some("default") {
                (id, json!({"preset": "default"}))
            } else {
                (id, json!({"preset": "custom", "pixelLength": f.number("pixelLength"), "logicalLength": f.number("logicalLength"), "units": f.text("units")}))
            }
        }
        Kind::DataPoints => {
            let chosen = |source: &str| -> Vec<&'static str> {
                analysis_cmds::available(source).into_iter().filter(|c| f.bool(&point_key(source, c)) == Some(true)).collect()
            };
            (id, json!({"selection": chosen("selection"), "ruler": chosen("ruler"), "count": chosen("count")}))
        }
        Kind::ScaleMarker => (
            id,
            json!({"length": f.number("length"), "fontSize": f.number("fontSize"), "displayText": f.bool("displayText"), "textPosition": f.text("textPosition"), "color": f.text("color")}),
        ),
    })
}

/// OK: runs the dialog's command; an engine refusal goes to the status line.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let Some(kind) = Kind::of(&result.key) else { return };
    let Some((id, params)) = command(kind, &result.fields) else { return };
    if let Err(error) = app.run(id, params)
        && error != "cancelled"
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, Label)> {
    pairs.iter().map(|(key, name)| ((*key).to_owned(), Label::from((*name).to_owned()))).collect()
}

/// The dialog of `kind` over `fields`.
fn form(kind: Kind, fields: &mut DialogFields) -> ParamDialog<'_> {
    let key = kind.command();
    let dialog = match kind {
        Kind::Scale => {
            let custom = |p: Param| p.not_remembered().enabled_when("preset", "custom");
            let rows = vec![
                Param::new(
                    "preset",
                    tl!("Presets:"),
                    ParamKind::choice(options(&[("custom", tl!("Custom")), ("default", tl!("Default"))]), Some("custom".into())),
                )
                .not_remembered(),
                custom(Param::new("pixelLength", tl!("Pixel Length:"), ParamKind::number(Some(0.001), Some(1e7), 1.0, 1.0, 4))),
                custom(Param::new("logicalLength", tl!("Logical Length:"), ParamKind::number(Some(0.0001), Some(1e7), 1.0, 1.0, 4))),
                custom(Param::new("units", tl!("Logical Units:"), ParamKind::text("pixels"))),
            ];
            ParamDialog::command(key, tl!("Measurement Scale"), rows).below(|ui, f| {
                let text = if f.text("preset") == Some("default") {
                    "1 pixels = 1.0000 pixels".to_owned()
                } else {
                    format!(
                        "{} pixels = {:.4} {}",
                        f.number("pixelLength").unwrap_or(1.0),
                        f.number("logicalLength").unwrap_or(1.0),
                        f.text("units").unwrap_or("pixels")
                    )
                };
                Text::new(Label::from(text)).show(ui);
            })
        }
        Kind::DataPoints => {
            let row = Param::custom("columns", tl!("Data Points"), |ui, fields, _| {
                Row::new()
                    .show(ui, |ui, _| {
                        for (source, title) in sources() {
                            Stack::new().show(ui, |ui, _| {
                                Text::new(Label::from(title)).show(ui);
                                for column in analysis_cmds::available(source) {
                                    let name = analysis_cmds::COLUMNS.iter().find(|c| c.0 == column).map_or(column, |c| c.1);
                                    let field = point_key(source, column);
                                    let mut on = fields.bool(&field).unwrap_or(false);
                                    Checkbox::new(Bind::bound(&mut on)).label(name.to_owned()).show(ui);
                                    if fields.bool(&field) != Some(on) {
                                        fields.set(field, on);
                                    }
                                }
                            });
                        }
                    })
                    .output
            });
            ParamDialog::command(key, tl!("Select Data Points"), vec![row])
        }
        Kind::ScaleMarker => {
            let rows = vec![
                Param::new("length", tl!("Length:"), ParamKind::number(Some(0.0001), Some(1e7), 1.0, 1.0, 4)).not_remembered(),
                Param::new("fontSize", tl!("Font Size:"), ParamKind::number(Some(1.0), Some(1000.0), 12.0, 1.0, 1)).unit(UnitKind::Points).not_remembered(),
                Param::new("displayText", tl!("Display Text"), ParamKind::bool(true)).not_remembered(),
                Param::new(
                    "textPosition",
                    tl!("Text Position:"),
                    ParamKind::choice(options(&[("bottom", tl!("Bottom")), ("top", tl!("Top"))]), Some("bottom".into())),
                )
                .not_remembered(),
                Param::new("color", tl!("Color:"), ParamKind::choice(options(&[("black", tl!("Black")), ("white", tl!("White"))]), Some("black".into())))
                    .not_remembered(),
            ];
            ParamDialog::command(key, tl!("Measurement Scale Marker"), rows)
        }
    };
    dialog.values(Bind::bound(fields))
}
