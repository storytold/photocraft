//! #13783: Preferences › Export in titled groups (Quick Export Format, Location, Metadata, Color
//! Space), the reference labels (PNG, JPG, GIF, "Convert to sRGB"), the location as two radio
//! buttons and no editable last-folder field.

use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
        app
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.export.exportPreferences", json!({})).unwrap();
    h.run_steps(8);
    h
}

fn y(h: &Harness<'_, PhotocraftApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().min.y
}

fn toggled(h: &Harness<'_, PhotocraftApp>, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn the_page_has_the_four_quick_export_groups_in_order() {
    let h = harness();
    let order = ["Quick Export Format", "Quick Export Location", "Quick Export Metadata", "Quick Export Color Space"].map(|g| y(&h, g));
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    let values: Vec<String> = h.query_all_by_role(Role::ComboBox).filter_map(|n| n.accesskit_node().value().map(|v| v.to_string())).collect();
    assert!(values.iter().any(|v| v == "PNG"), "the format reads PNG: {values:?}");
    assert!(!values.iter().any(|v| v == "Png"));
    assert!(h.query_by_role_and_label(Role::CheckBox, "Convert to sRGB").is_some());
}

#[test]
fn the_location_is_two_radio_buttons_and_the_last_folder_is_not_a_field() {
    let mut h = harness();
    let ask = "Ask where to export each time";
    let assets = "Export files to an assets folder next to the current document";
    assert!(toggled(&h, Role::RadioButton, ask) && !toggled(&h, Role::RadioButton, assets));
    assert!(h.query_by_label("Last folder").is_none() && h.query_by_label("Last Folder").is_none());
    h.get_by_role_and_label(Role::RadioButton, assets).click();
    h.run_steps(4);
    assert!(toggled(&h, Role::RadioButton, assets) && !toggled(&h, Role::RadioButton, ask), "clicking the label selects it");
    h.get_by_role_and_label(Role::CheckBox, "Convert to sRGB").click();
    h.run_steps(4);
    assert!(!toggled(&h, Role::CheckBox, "Convert to sRGB"), "clicking the label toggles it");
}

#[test]
fn labels_and_search_terms() {
    assert_eq!(["png", "jpg", "gif", "webp", "exr"].map(choice_label), ["PNG", "JPG", "GIF", "WebP", "EXR"]);
    assert_eq!(humanize("convertToSrgb"), "Convert to sRGB");
    for (key, query) in [("convertToSrgb", "srgb"), ("quickExportLocation", "location"), ("quickExportFormat", "format"), ("metadata", "metadata")] {
        assert!(preference_matches("export", key, query), "{key}");
    }
    let mut obj = Map::new();
    obj.insert("lastFolder".into(), json!("/somewhere"));
    assert!(!export_field_visible(&obj, "lastFolder"), "remembered, not shown");
}
