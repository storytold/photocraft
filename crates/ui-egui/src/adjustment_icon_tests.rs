//! #9038: every adjustment kind has its own glyph, the same in the Layers panel thumbnail and on
//! the Adjustments panel button.

use std::collections::BTreeMap;

use photocraft_doc::{Adjustment, ColorMode};
use photocraft_engine::adjust_params::default_for;

use crate::icons;
use crate::panels::{adjustment_icon, adjustment_kind_icon};

/// The adjustment kinds behind `layer.newAdjustmentLayer.<kind>`.
fn kinds() -> Vec<&'static str> {
    let kinds: Vec<_> = photocraft_engine::commands::command_specs().iter().filter_map(|c| c.id.strip_prefix("layer.newAdjustmentLayer.")).collect();
    assert!(kinds.len() >= 16, "{kinds:?}");
    kinds
}

#[test]
fn every_adjustment_kind_has_its_own_registered_icon() {
    let mut seen = BTreeMap::new();
    for kind in kinds() {
        let icon = adjustment_kind_icon(kind);
        assert!(icons::exists(icon), "{kind}: icon {icon:?} is not bundled");
        assert_ne!(icon, "adjustment-layer", "{kind} falls back to the generic glyph");
        if let Some(other) = seen.insert(icon, kind) {
            panic!("{kind} and {other} share the icon {icon:?}");
        }
    }
}

#[test]
fn layer_thumbnail_and_panel_button_show_the_same_icon() {
    for kind in kinds() {
        let adj = default_for(kind, ColorMode::Rgb).unwrap();
        assert_eq!(adjustment_icon(&adj), adjustment_kind_icon(kind), "{kind}");
    }
}

#[test]
fn unsupported_adjustments_keep_the_generic_icon() {
    let adj = Adjustment::Unsupported { psd_key: "test".into(), raw: std::sync::Arc::new([]) };
    assert_eq!(adjustment_icon(&adj), "adjustment-layer");
    assert!(icons::exists("adjustment-layer"));
    assert_eq!(adjustment_kind_icon("noSuchKind"), "adjustment-layer");
}

#[test]
fn adjustment_icons_draw_distinct_glyphs() {
    // A malformed SVG would rasterize to nothing; two identical files would read as one kind.
    let mut seen: BTreeMap<Vec<bool>, &str> = BTreeMap::new();
    for kind in kinds() {
        let icon = adjustment_kind_icon(kind);
        let image = icons::rasterize_svg(icon, 24, 24).unwrap_or_else(|| panic!("{icon} does not rasterize"));
        let ink: Vec<bool> = image.pixels.iter().map(|p| p.a() > 16).collect();
        assert!(ink.iter().filter(|&&on| on).count() > 20, "{icon} draws almost nothing");
        if let Some(other) = seen.insert(ink, icon) {
            panic!("{icon} draws the same pixels as {other}");
        }
    }
}
