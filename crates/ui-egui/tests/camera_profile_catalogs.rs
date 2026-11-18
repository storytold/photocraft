//! Profile labels follow the catalogs embedded by the shared translation build input.
use photocraft_ui_egui::i18n::{CRAFT_I18N, Lang, tr};

#[test]
fn camera_profile_label_uses_the_shared_catalog_or_english_fallback() {
    assert_eq!(tr(Lang::EN, "Profile"), "Profile");
    for (code, common, app) in CRAFT_I18N {
        let rows: Vec<_> = app.lines().filter_map(|line| line.strip_prefix("\tProfile\t")).collect();
        let shared: Vec<_> = common.lines().filter_map(|line| line.strip_prefix("\tProfile\t")).collect();
        assert!(rows.len() <= 1 && shared.len() <= 1, "{code}: duplicate Profile translation");
        // Missing rows use the documented English fallback. Catalog ownership is craft-i18n.
        if let Some(value) = rows.first().or_else(|| shared.first()) {
            assert!(!value.trim().is_empty(), "{code}: empty Profile translation");
        }
    }
}
