use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn menus_dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.menus", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn set_field(h: &mut Harness<'_, PhotocraftApp>, id: u64, name: &str, v: Value) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(name.into(), v);
    h.run_steps(2);
}

fn has_label(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

#[test]
fn menus_search_matches_words_in_any_order_over_full_path() {
    let (mut h, id) = menus_dialog();
    assert!(h.query_all_by_role(egui::accesskit::Role::TextInput).any(|n| n.accesskit_node().placeholder() == Some("Search in Application Menus")));

    set_field(&mut h, id, "filter", json!("layer via"));
    assert!(has_label(&h, "New › Layer Via Copy"));
    assert!(has_label(&h, "New › Layer Via Cut"));
    assert!(has_label(&h, "Smart Objects › New Smart Object via Copy"));

    set_field(&mut h, id, "filter", json!("via layer"));
    assert!(has_label(&h, "New › Layer Via Copy"));
    assert!(has_label(&h, "New › Layer Via Cut"));
    assert!(has_label(&h, "Smart Objects › New Smart Object via Copy"));

    set_field(&mut h, id, "filter", json!("edit fade"));
    assert!(has_label(&h, "Fade…"));
}

#[test]
fn menus_search_match_case_filters_exact_case_only() {
    let (mut h, id) = menus_dialog();
    set_field(&mut h, id, "filter", json!("layer via"));
    assert!(has_label(&h, "New › Layer Via Copy"));

    h.get_by_label("Match Case").click();
    h.run_steps(2);
    assert!(!has_label(&h, "New › Layer Via Copy"));

    set_field(&mut h, id, "filter", json!("Layer Via"));
    assert!(has_label(&h, "New › Layer Via Copy"));
    assert!(has_label(&h, "New › Layer Via Cut"));
    assert!(!has_label(&h, "Smart Objects › New Smart Object via Copy"));
}

#[test]
fn keyboard_shortcuts_tab_keeps_original_placeholder_and_has_no_match_case() {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap();
    h.run_steps(4);

    assert!(h.query_all_by_role(egui::accesskit::Role::TextInput).any(|n| n.accesskit_node().placeholder() == Some("command or shortcut")));
    assert!(h.query_by_label("Match Case").is_none());
}
