//! Layer > Layer Style keeps Blending Options in its own group (#14863).

use crate::menu_catalog::CATALOG;

/// The submenu's rows split into groups by its separators, ids without their prefix.
fn groups() -> Vec<Vec<&'static str>> {
    let mut groups: Vec<Vec<&'static str>> = vec![Vec::new()];
    for row in CATALOG.iter().filter(|r| r.0 == ["Layer", "Layer Style"]) {
        if row.3 == "---" {
            groups.push(Vec::new());
        } else if let Some(last) = groups.last_mut() {
            last.push(row.3.trim_start_matches("layer.layerStyle."));
        }
    }
    groups
}

#[test]
fn blending_options_sits_alone_above_the_effect_entries() {
    let groups = groups();
    let expected: Vec<Vec<&str>> = vec![
        vec!["blendingOptions"],
        vec!["bevelEmboss", "stroke", "innerShadow", "innerGlow", "satin", "colorOverlay", "gradientOverlay", "patternOverlay", "outerGlow", "dropShadow"],
        vec!["copyLayerStyle", "pasteLayerStyle", "clear"],
        vec!["globalLight", "createLayer", "hideAllEffects", "scaleEffects"],
    ];
    assert_eq!(groups, expected);
}

#[test]
fn the_separator_adds_no_command() {
    let separators = CATALOG.iter().filter(|r| r.0 == ["Layer", "Layer Style"] && r.3 == "---").count();
    assert_eq!(separators, 3);
}
