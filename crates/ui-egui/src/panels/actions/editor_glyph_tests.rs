//! Source contract tests; no rasterization or app launch.

#[path = "editor_glyphs.rs"]
mod glyphs;

use craft_ui::icons::{SvgIconSet, SvgStyle};

use glyphs::{GLYPH_COLUMNS, GLYPHS, display_glyph};

// Test-only embed table. Production registration belongs with the real editor consumer.
static ASSETS: &[(&str, &[u8])] = &[
    ("action-editor-click", include_bytes!("../../../../../assets/icons/action-editor/click.svg")),
    ("action-editor-crop", include_bytes!("../../../../../assets/icons/action-editor/crop.svg")),
    ("action-editor-merge", include_bytes!("../../../../../assets/icons/action-editor/merge.svg")),
    ("action-editor-adjust", include_bytes!("../../../../../assets/icons/action-editor/adjust.svg")),
    ("action-editor-hue", include_bytes!("../../../../../assets/icons/action-editor/hue.svg")),
    ("action-editor-exposure", include_bytes!("../../../../../assets/icons/action-editor/exposure.svg")),
    ("action-editor-lut", include_bytes!("../../../../../assets/icons/action-editor/lut.svg")),
    ("action-editor-black-white", include_bytes!("../../../../../assets/icons/action-editor/black-white.svg")),
    ("action-editor-smart-remove", include_bytes!("../../../../../assets/icons/action-editor/smart-remove.svg")),
    ("action-editor-dodge", include_bytes!("../../../../../assets/icons/action-editor/dodge.svg")),
    ("action-editor-burn", include_bytes!("../../../../../assets/icons/action-editor/burn.svg")),
    ("action-editor-erase", include_bytes!("../../../../../assets/icons/action-editor/erase.svg")),
    ("action-editor-layer-effects", include_bytes!("../../../../../assets/icons/action-editor/layer-effects.svg")),
    ("action-editor-generative-effects", include_bytes!("../../../../../assets/icons/action-editor/generative-effects.svg")),
    ("action-editor-sharpen", include_bytes!("../../../../../assets/icons/action-editor/sharpen.svg")),
    ("action-editor-object-select", include_bytes!("../../../../../assets/icons/action-editor/object-select.svg")),
    ("action-editor-wand", include_bytes!("../../../../../assets/icons/action-editor/wand.svg")),
    ("action-editor-generate", include_bytes!("../../../../../assets/icons/action-editor/generate.svg")),
    ("action-editor-text-color", include_bytes!("../../../../../assets/icons/action-editor/text-color.svg")),
    ("action-editor-text", include_bytes!("../../../../../assets/icons/action-editor/text.svg")),
    ("action-editor-download", include_bytes!("../../../../../assets/icons/action-editor/download.svg")),
    ("action-editor-layers", include_bytes!("../../../../../assets/icons/action-editor/layers.svg")),
    ("action-editor-brush", include_bytes!("../../../../../assets/icons/action-editor/brush.svg")),
    ("action-editor-subject", include_bytes!("../../../../../assets/icons/action-editor/subject.svg")),
    ("action-editor-remove-background", include_bytes!("../../../../../assets/icons/action-editor/remove-background.svg")),
    ("action-editor-blur", include_bytes!("../../../../../assets/icons/action-editor/blur.svg")),
    ("action-editor-harmonize", include_bytes!("../../../../../assets/icons/action-editor/harmonize.svg")),
    ("action-editor-clarity", include_bytes!("../../../../../assets/icons/action-editor/clarity.svg")),
    ("action-editor-styles", include_bytes!("../../../../../assets/icons/action-editor/styles.svg")),
    ("action-editor-grid", include_bytes!("../../../../../assets/icons/action-editor/grid.svg")),
    ("square", include_bytes!("../../../../../assets/icons/square.svg")),
];

#[test]
fn measured_picker_order_is_complete_and_unique() {
    // Transcribed from the issue's five measured rows, independently of the asset catalog.
    let expected = [
        "ClickAction",
        "ActionsCrop",
        "LayerMerge",
        "AdjustmentGeneral",
        "HueAndSat",
        "ActionsExposure",
        "ColorLookUpTables",
        "BlackAndWhite",
        "SmartRemove",
        "ActionsDodge",
        "ActionsBurn",
        "ActionsErase",
        "LayerEffects",
        "AIGenEffects",
        "Sharpen",
        "ObjectSelectionLasso",
        "MagicWand",
        "AIGenerate",
        "TextColor",
        "ActionsText",
        "ActionsDownload",
        "ActionsLayers",
        "Brush",
        "SelectSubject",
        "RemoveBackground",
        "GaussianBlur",
        "LayerHarmonize",
        "P_AdjustClarity_32x32",
        "AIStyles",
        "ShowGrid",
    ];
    assert_eq!(GLYPH_COLUMNS, 6);
    assert_eq!(GLYPHS.map(|(identifier, _)| identifier), expected);
    assert_eq!(GLYPHS.len() / GLYPH_COLUMNS, 5);
    assert_eq!(ASSETS.len(), GLYPHS.len() + 1);
    let mut identifiers = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for ((identifier, name), (embedded_name, bytes)) in GLYPHS.iter().zip(ASSETS) {
        assert!(identifiers.insert(*identifier));
        assert!(names.insert(*name));
        assert_eq!(name, embedded_name);
        assert_eq!(display_glyph(identifier), *name);
        assert!(!bytes.is_empty());
    }
}

#[test]
fn unknown_identifiers_use_the_neutral_display_fallback() {
    for identifier in ["", "clickaction", "ClickAction ", "future-action-icon", "图标"] {
        assert_eq!(display_glyph(identifier), "square");
    }
    // This API only borrows metadata; the caller keeps the original value for persistence.
    let stored = String::from("future-action-icon");
    assert_eq!(display_glyph(&stored), "square");
    assert_eq!(stored, "future-action-icon");
}

#[test]
fn original_assets_follow_the_existing_svg_source_pipeline() {
    let icons = SvgIconSet::new(ASSETS, SvgStyle { current_color: "#ffffff", stroke_width: Some("1.75") }, "bytes://icons/", "square");
    for (_, name) in GLYPHS {
        assert!(icons.contains(name));
        let original = ASSETS.iter().find(|(key, _)| *key == name).unwrap().1;
        let source = std::str::from_utf8(original).unwrap();
        assert!(source.contains("viewBox=\"0 0 24 24\""));
        assert!(source.contains("stroke=\"currentColor\""));
        assert!(source.contains("stroke-width=\"2\""));
        for forbidden in ["href=", "<image", "<script", "<text", "<filter", "<style"] {
            assert!(!source.contains(forbidden), "{name}: {forbidden}");
        }
        let egui::ImageSource::Bytes { uri, bytes } = icons.source(name) else {
            panic!("icon source must be embedded bytes");
        };
        let expected = source.replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
        assert_eq!(bytes.as_ref(), expected.as_bytes());
        assert_eq!(uri.as_ref(), format!("bytes://icons/{name}.svg"));
    }
    let egui::ImageSource::Bytes { bytes, .. } = icons.source(display_glyph("future-action-icon")) else {
        panic!("fallback must be embedded bytes");
    };
    let square = std::str::from_utf8(include_bytes!("../../../../../assets/icons/square.svg")).unwrap();
    let expected = square.replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
    assert_eq!(bytes.as_ref(), expected.as_bytes());
}
