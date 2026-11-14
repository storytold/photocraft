use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run();
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

fn set_field(h: &mut Harness<'_, PhotocraftApp>, id: u64, name: &str, v: Value) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(name.into(), v);
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

#[test]
fn save_is_enabled_for_modified_defaults_and_prompts_with_copy_name() {
    let (mut h, id) = dialog();
    assert!(h.get_by_label("Save").accesskit_node().is_disabled());

    set_field(&mut h, id, "overrides", json!({"edit.copy": "F9"}));
    assert!(!h.get_by_label("Save").accesskit_node().is_disabled());

    click(&mut h, "Save");
    assert_eq!(field(&h, id, "newSetName"), json!("PhotoCraft Defaults (modified) Copy"));

    click(&mut h, "Create");
    assert_eq!(field(&h, id, "activeSet"), json!("PhotoCraft Defaults (modified) Copy"));
    assert_eq!(field(&h, id, "newSetName"), Value::Null);
}

#[test]
fn new_set_prefills_copy_name_and_save_overwrites_saved_set() {
    let (mut h, id) = dialog();
    click(&mut h, "New Set…");
    assert_eq!(field(&h, id, "newSetName"), json!("PhotoCraft Defaults Copy"));

    set_field(&mut h, id, "newSetName", json!("my-set"));
    click(&mut h, "Create");
    assert_eq!(field(&h, id, "activeSet"), json!("my-set"));

    click(&mut h, "New Set…");
    assert_eq!(field(&h, id, "newSetName"), json!("my-set Copy"));
    click(&mut h, "Cancel");

    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8"}));
    click(&mut h, "Save");
    assert_eq!(field(&h, id, "newSetName"), Value::Null);
    assert_eq!(field(&h, id, "sets")["my-set"]["shortcuts"]["edit.copy"], json!("F8"));
}
