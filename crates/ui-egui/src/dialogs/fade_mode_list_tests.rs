//! #13806: Edit › Fade's Mode list holds the 27 layer blend modes in the reference order and greys the modes a
//! Grayscale 8-bit, Lab 8-bit or RGB 32-bit document can't fade with.

use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use craft::ui::runtime::env::Label;
use photocraft_color::{BlendMode, ColorMode, SampleType};
use serde_json::json;

use super::{COMMAND, greyed, param};
use crate::PhotocraftApp;

/// The reference application's Fade Mode list, a group per row.
const GROUPS: [&[&str]; 6] = [
    &["Normal", "Dissolve"],
    &["Darken", "Multiply", "Color Burn", "Linear Burn", "Darker Color"],
    &["Lighten", "Screen", "Color Dodge", "Linear Dodge (Add)", "Lighter Color"],
    &["Overlay", "Soft Light", "Hard Light", "Vivid Light", "Linear Light", "Pin Light", "Hard Mix"],
    &["Difference", "Exclusion", "Subtract", "Divide"],
    &["Hue", "Saturation", "Color", "Luminosity"],
];

fn app(mode: &str, depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12, "mode": mode, "depth": depth})).unwrap();
    app
}

fn labels(modes: &[BlendMode]) -> Vec<&'static str> {
    modes.iter().map(|m| m.label()).collect()
}

/// The `mode` keys of `edit.fade`'s parameter notation.
fn spec_modes() -> Vec<String> {
    let spec = photocraft_engine::commands::find(COMMAND).unwrap();
    let p = Param::parse_list(spec.params).into_iter().find(|p| p.key == "mode").unwrap();
    let ParamKind::Choice { options, .. } = p.kind else { panic!("mode is not a choice") };
    options.into_iter().map(|(id, _)| id).collect()
}

#[test]
fn the_list_is_the_27_modes_in_the_reference_order() {
    assert_eq!(labels(&BlendMode::LAYER_MODES), GROUPS.concat());
}

#[test]
fn the_params_doc_lists_all_27_modes_and_the_old_names_still_work() {
    let options = spec_modes();
    let modes: Vec<Option<BlendMode>> = options.iter().map(|o| photocraft_engine::commands::parse_blend(o)).collect();
    assert_eq!(modes, BlendMode::LAYER_MODES.map(Some).to_vec());
    for old in ["normal", "multiply", "screen", "overlay", "softLight", "hardLight", "darken", "lighten", "difference", "color", "luminosity"] {
        assert!(options.iter().any(|o| o == old), "{old} is still listed");
    }
}

/// The dialog's row writes the Layers panel's keys; each one fades.
#[test]
fn every_listed_mode_fades() {
    for key in BlendMode::LAYER_MODES.map(|mode| mode.key().to_string()) {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        let out = s.execute(COMMAND, json!({"opacity": 50, "mode": key})).unwrap();
        assert_eq!(out["mode"], json!(photocraft_engine::commands::parse_blend(&key).unwrap().label()), "{key}");
    }
}

#[test]
fn greyed_modes_follow_the_measured_colour_modes_and_depths() {
    use SampleType::*;
    assert_eq!(labels(greyed(ColorMode::Grayscale, U8)), ["Darker Color", "Lighter Color", "Hue", "Saturation", "Color", "Luminosity"]);
    assert_eq!(labels(greyed(ColorMode::Lab, U8)), ["Darken", "Color Burn", "Lighten", "Color Dodge", "Difference", "Exclusion", "Subtract", "Divide"]);
    assert_eq!(
        labels(greyed(ColorMode::Rgb, F32)),
        [
            "Color Burn",
            "Linear Burn",
            "Screen",
            "Color Dodge",
            "Overlay",
            "Soft Light",
            "Hard Light",
            "Vivid Light",
            "Linear Light",
            "Pin Light",
            "Hard Mix",
            "Exclusion"
        ]
    );
    // Measured with none greyed, and the cases not measured grey none.
    for (mode, depth) in
        [(ColorMode::Rgb, U16), (ColorMode::Cmyk, U8), (ColorMode::Rgb, U8), (ColorMode::Grayscale, U16), (ColorMode::Lab, U16), (ColorMode::Cmyk, U16)]
    {
        assert!(greyed(mode, depth).is_empty(), "{mode:?} {depth:?}");
    }
}

/// The row lists every mode and disables the ones the active document greys.
#[test]
fn the_row_disables_the_documents_greyed_modes() {
    for (mode, depth, want) in [("lab", 8, greyed(ColorMode::Lab, SampleType::U8)), ("rgb", 32, greyed(ColorMode::Rgb, SampleType::F32)), ("rgb", 16, &[][..])]
    {
        let row = param(&app(mode, depth), Label::from("Mode"));
        let ParamKind::Blend { modes, .. } = &row.kind else { panic!("Mode is a blend row") };
        assert_eq!(modes.len(), BlendMode::LAYER_MODES.len(), "{mode} {depth}");
        for m in BlendMode::LAYER_MODES {
            assert_eq!(row.option_enabled(m.key()), !want.contains(&m), "{mode} {depth}: {}", m.label());
        }
    }
}
