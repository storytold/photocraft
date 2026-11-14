//! #14317: Edit › Keyboard Shortcuts has a Match Case checkbox beside the search field, off by
//! default. Off, the search ignores case as before; on, a command name must contain the text with
//! the exact case ("copy" finds nothing, "Copy" finds Copy, Copy Merged and Layer Via Copy).

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::shortcuts::search_hit;
use crate::PhotocraftApp;

/// The Keyboard Shortcuts dialog on `tab`, searching for `filter`.
fn dialog(tab: u64, filter: &str, match_case: Option<bool>) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("tab".into(), json!(tab));
    f.insert("filter".into(), json!(filter));
    if let Some(on) = match_case {
        f.insert("matchCase".into(), json!(on));
    }
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

/// The grid rows for the copy commands (names as the grid shows them).
const COPY_ROWS: [&str; 3] = ["   Copy", "   Copy Merged", "   New › Layer Via Copy"];

#[test]
fn match_case_is_off_by_default_and_the_search_ignores_case() {
    let (h, id) = dialog(0, "copy", None);
    let check = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Match Case");
    assert_eq!(check.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert_eq!(field(&h, id, "matchCase"), json!(false));
    for row in COPY_ROWS {
        h.get_by_label(row);
    }
}

#[test]
fn ticking_match_case_needs_the_exact_case() {
    let (mut h, id) = dialog(0, "copy", None);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Match Case").click();
    h.run_steps(3);
    assert_eq!(field(&h, id, "matchCase"), json!(true));
    for row in COPY_ROWS {
        assert!(h.query_by_label(row).is_none(), "{row:?} is written with a capital C, so \"copy\" must not find it");
    }

    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("filter".into(), json!("Copy"));
    h.run_steps(3);
    for row in COPY_ROWS {
        h.get_by_label(row);
    }
}

#[test]
fn match_case_matches_command_names_not_ids_or_keys() {
    // Off: the id and the key still find a command, as before.
    let (h, _) = dialog(0, "layerViaCopy", None);
    h.get_by_label("   New › Layer Via Copy");
    // On: only the name counts, so neither the id nor the shortcut text matches.
    for needle in ["layerViaCopy", "Cmd+J"] {
        let (h, _) = dialog(0, needle, Some(true));
        assert!(h.query_by_label("   New › Layer Via Copy").is_none(), "{needle:?} is not in the command's name");
    }
}

#[test]
fn the_menus_tab_has_no_match_case_and_keeps_a_case_blind_search() {
    let (h, _) = dialog(1, "copy", Some(true));
    assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Match Case").is_none());
    h.get_by_label("   Copy");
}

#[test]
fn search_hit_compares_names_with_the_exact_case_only_when_asked() {
    let local = "Edit Copy Merged";
    let english = "Edit Copy Merged Cmd+Shift+C edit.copyMerged";
    let names = ["Copy Merged", "Copy Merged"];
    assert!(search_hit(local, english, names, "copy", false));
    assert!(search_hit(local, english, names, "shift+c", false));
    assert!(!search_hit(local, english, names, "copy", true));
    assert!(!search_hit(local, english, names, "Edit", true), "the menu path is not the command's name");
    assert!(search_hit(local, english, names, "Copy M", true));
}
