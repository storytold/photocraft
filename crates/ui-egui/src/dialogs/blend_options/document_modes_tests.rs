//! #14647: the Mode row that Edit › Fill and Edit › Stroke share greys the modes the active document's colour mode and
//! depth can't use, and a remembered mode that is greyed there opens as Normal.

use craft::ui::patterns::form_dialogs::{DialogFields, ParamDialog, ParamKind};
use photocraft_color::BlendMode;
use serde_json::json;

use super::blending_rows;
use crate::PhotocraftApp;

fn app(mode: &str, depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12, "mode": mode, "depth": depth})).unwrap();
    app
}

/// The labels of the Mode row's disabled entries in the Fill and Stroke rows of `app`.
fn greyed(app: &PhotocraftApp) -> Vec<&'static str> {
    let rows = blending_rows(app);
    let row = rows.iter().find(|p| p.key == "mode").unwrap();
    let ParamKind::Blend { modes, .. } = &row.kind else { panic!("Mode is a blend row") };
    assert_eq!(modes.len(), BlendMode::LAYER_MODES.len(), "every mode stays listed");
    BlendMode::LAYER_MODES.iter().filter(|m| !row.option_enabled(m.key())).map(|m| m.label()).collect()
}

#[test]
fn the_mode_row_greys_what_the_document_cannot_use() {
    let cases: [(&str, u32, &[&str]); 6] = [
        ("grayscale", 8, &["Darker Color", "Lighter Color", "Hue", "Saturation", "Color", "Luminosity"]),
        ("lab", 8, &["Darken", "Color Burn", "Lighten", "Color Dodge", "Difference", "Exclusion", "Subtract", "Divide"]),
        (
            "rgb",
            32,
            &[
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
                "Exclusion",
            ],
        ),
        ("rgb", 16, &[]),
        ("cmyk", 8, &[]),
        ("rgb", 8, &[]),
    ];
    for (mode, depth, want) in cases {
        assert_eq!(greyed(&app(mode, depth)), want, "{mode} {depth}");
    }
}

#[test]
fn without_a_document_nothing_is_greyed() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(greyed(&app).is_empty());
}

/// The framework restores only the remembered values a row accepts, so a greyed mode is dropped and the row opens on
/// its default, Normal; a mode the document can use is kept.
#[test]
fn a_remembered_greyed_mode_opens_as_normal() {
    let mut saved = DialogFields::new();
    saved.set("mode", BlendMode::Darken.key().to_string());
    let lab = ParamDialog::remembered(&blending_rows(&app("lab", 8)), &saved);
    assert_eq!(lab.text("mode"), None, "Darken is greyed in Lab 8-bit");
    let rgb = ParamDialog::remembered(&blending_rows(&app("rgb", 16)), &saved);
    assert_eq!(rgb.text("mode"), Some(BlendMode::Darken.key()), "Darken stays in RGB 16-bit");
}
