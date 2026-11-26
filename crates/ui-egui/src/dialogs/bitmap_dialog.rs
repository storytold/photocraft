//! Image › Mode › Bitmap… (#5559), laid out like Photoshop's dialog: Resolution (Input, read-only,
//! and Output, in pixels/inch or pixels/centimeter), Method (Use: 50% Threshold, Pattern Dither,
//! Diffusion Dither, Halftone Screen, Custom Pattern) with the Custom Pattern picker, shown only
//! for Custom Pattern. The Halftone Screen settings (Frequency in lines/inch or lines/centimeter,
//! Angle, Shape), which Photoshop asks for in a second dialog after OK, show only while
//! Halftone Screen is the method.
//!
//! It is the filter dialog of `image.mode.bitmap` with its own rows ([`params`]): the fields are
//! the command's params (`resolution` in pixels/inch, `frequency` in lines/inch), so OK, the live
//! preview, `ui.dialog.set` and `ui.dialog.confirm` work as for every filter dialog.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, NumberUnit, Param, ParamKind};
use craft::ui::runtime::env::UnitKind;

use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "image.mode.bitmap";

/// Centimetres per inch.
const CM: f64 = 2.54;
/// The command's Output resolution range (pixels/inch).
const RESOLUTION: (f64, f64) = (1.0, 30_000.0);
/// The command's Halftone Screen frequency range (lines/inch).
const FREQUENCY: (f64, f64) = (1.0, 999.0);

const METHODS: [(&str, &str); 5] = [
    ("threshold", "50% Threshold"),
    ("pattern", "Pattern Dither"),
    ("diffusion", "Diffusion Dither"),
    ("halftone", "Halftone Screen"),
    ("customPattern", "Custom Pattern"),
];

/// Photoshop's order of the Halftone Screen shapes.
const SHAPES: [(&str, &str); 6] =
    [("round", "Round"), ("diamond", "Diamond"), ("ellipse", "Ellipse"), ("line", "Line"), ("square", "Square"), ("cross", "Cross")];

fn input_resolution(app: &PhotocraftApp) -> f64 {
    app.session.active().map_or(72.0, |d| f64::from(d.doc.resolution_dpi))
}

/// Open defaults: Output starts at the document's resolution (Input, as in Photoshop), Custom
/// Pattern at the Patterns panel's pattern.
pub(crate) fn defaults(app: &PhotocraftApp, f: &mut DialogFields) {
    f.set("resolution", FieldValue::Number(input_resolution(app)));
    match photocraft_engine::pattern_cmds::resolve(&app.session, "") {
        Some(p) => f.set("pattern", FieldValue::Text(p.id.to_string())),
        None => {
            f.remove("pattern");
        }
    };
}

/// The dialog's rows. The filter dialog's opener gives the dialog the Custom Pattern rows.
pub(crate) fn params(app: &PhotocraftApp) -> Vec<Param> {
    let input = input_resolution(app);
    let choice = |list: &[(&'static str, &'static str)]| -> Vec<(String, craft::ui::runtime::env::Label)> {
        list.iter().map(|(id, l)| (id.to_string(), tl!(l).into())).collect()
    };

    vec![
        Param::section(tl!("Resolution")),
        Param::read_only_text("inputResolution", tl!("Input:"), format!("{input} {}", tl!("Pixels/Inch"))),
        Param::number_with_units(
            "resolution",
            tl!("Output:"),
            input,
            1.0,
            3,
            vec![
                NumberUnit::new("ppi", UnitKind::PixelsPerInch, tl!("Pixels/Inch"), 1.0),
                NumberUnit::new("ppcm", UnitKind::PixelsPerCentimetre, tl!("Pixels/Centimeter"), CM),
            ],
        )
        .bounds(Some(RESOLUTION.0), Some(RESOLUTION.1)),
        Param::section(tl!("Method")),
        Param::new("method", tl!("Use:"), ParamKind::choice(choice(&METHODS), Some("diffusion".to_string()))),
        Param::new("pattern", tl!("Custom Pattern:"), ParamKind::pattern(None)).visible_when("method", "customPattern"),
        Param::number_with_units(
            "frequency",
            tl!("Frequency:"),
            53.0,
            1.0,
            3,
            vec![
                NumberUnit::new("lpi", UnitKind::LinesPerInch, tl!("Lines/Inch"), 1.0),
                NumberUnit::new("lpcm", UnitKind::LinesPerCentimetre, tl!("Lines/Centimeter"), CM),
            ],
        )
        .bounds(Some(FREQUENCY.0), Some(FREQUENCY.1))
        .visible_when("method", "halftone"),
        Param::new("angle", tl!("Angle:"), ParamKind::angle(45.0)).visible_when("method", "halftone"),
        Param::new("shape", tl!("Shape:"), ParamKind::choice(choice(&SHAPES), Some("round".to_string()))).visible_when("method", "halftone"),
    ]
}

#[cfg(test)]
mod tests;
