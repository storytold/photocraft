use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::shortcut_set_tests::{click, dialog, field, set_field};

#[test]
fn shortcut_set_delete_requires_confirmation() {
    let (mut h, id, _) = dialog(vec![]);
    set_field(&mut h, id, "sets", json!({"Retouch": {"shortcuts": {"edit.copy": "F9"}}, "Paint": {}}));
    set_field(&mut h, id, "activeSet", json!("Retouch"));
    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8", "actions.play:Sepia": "F6"}));
    set_field(&mut h, id, "alternates", json!({"edit.copy": ["F7"]}));
    let sets = field(&h, id, "sets");

    click(&mut h, "Delete");
    assert_eq!(field(&h, id, "sets"), sets);
    assert!(h.query_all(egui_kittest::kittest::by().label("Are you sure you want to delete Retouch?").include_labels()).next().is_some());
    click(&mut h, "No");
    assert_eq!(field(&h, id, "activeSet"), "Retouch");
    assert_eq!(field(&h, id, "sets"), sets);
    assert_eq!(field(&h, id, "overrides")["edit.copy"], "F8");

    for key in [egui::Key::Escape, egui::Key::Enter] {
        click(&mut h, "Delete");
        h.key_press(key);
        h.run_steps(3);
        assert!(field(&h, id, "__deleteShortcutSet").is_null());
        assert_eq!(field(&h, id, "activeSet"), "Retouch");
        assert_eq!(field(&h, id, "sets"), sets);
    }

    click(&mut h, "Delete");
    click(&mut h, "Yes");
    assert!(field(&h, id, "sets")["Retouch"].is_null());
    assert!(field(&h, id, "sets")["Paint"].is_object());
    assert_eq!(field(&h, id, "activeSet"), "");
    assert_eq!(field(&h, id, "overrides"), json!({"actions.play:Sepia": "F6"}));
    assert_eq!(field(&h, id, "alternates"), json!({}));
    let delete = h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete");
    assert!(delete.accesskit_node().is_disabled());
}
