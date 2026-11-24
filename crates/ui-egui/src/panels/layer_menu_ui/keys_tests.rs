//! The Layers panel's right-click menu on the keyboard (#15227, #15219): ↑ on its first enabled entry and ↓ on its last
//! stay there instead of continuing at the other end, and Home and End go to those entries. The menu is craft-libs'
//! `Menu::context`, which draws and drives it; this checks the menu PhotoCraft opens behaves so.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app with a document of two layers and the Layers panel shown, the right-click menu of "Upper" open.
fn menu_open() -> H {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Upper"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h.get_by_role_and_label(Role::TreeItem, "Upper").click_secondary();
    h.run_steps(8);
    assert!(!entries(&h).is_empty(), "the row's menu is open");
    h
}

/// Whether `node` is an entry of a popup menu: a menu row under a menu, not a title of the menu bar.
fn in_menu(node: &egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    if !matches!(node.role(), Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio) {
        return false;
    }
    let mut up = node.parent();
    while let Some(parent) = up {
        match parent.role() {
            Role::Menu => return true,
            Role::MenuBar => return false,
            _ => up = parent.parent(),
        }
    }
    false
}

/// The labels of the menu's enabled entries, top to bottom.
fn entries(h: &H) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = h
        .query_all(egui_kittest::kittest::by().predicate(|node| in_menu(node) && !node.is_disabled()))
        .map(|node| (node.rect().top(), node.accesskit_node().label().unwrap_or_default().to_string()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, label)| label).collect()
}

/// The label of the menu entry that holds the focus after `key`.
fn press(h: &mut H, key: egui::Key) -> Option<String> {
    h.key_press(key);
    h.run_steps(4);
    h.query_all(egui_kittest::kittest::by().predicate(in_menu))
        .find(|node| node.is_focused())
        .map(|node| node.accesskit_node().label().unwrap_or_default().to_string())
}

#[test]
fn the_layer_menu_keys_stop_at_its_ends_and_home_and_end_reach_them() {
    let mut h = menu_open();
    let rows = entries(&h);
    let (first, second, last) = (rows[0].clone(), rows[1].clone(), rows[rows.len() - 1].clone());
    assert_eq!(press(&mut h, egui::Key::End), Some(last.clone()), "End goes to the last entry");
    assert_eq!(press(&mut h, egui::Key::ArrowDown), Some(last.clone()), "↓ on the last entry stays there");
    assert_eq!(press(&mut h, egui::Key::Home), Some(first.clone()), "Home goes to the first entry");
    assert_eq!(press(&mut h, egui::Key::ArrowUp), Some(first), "↑ on the first entry stays there");
    assert_eq!(press(&mut h, egui::Key::ArrowDown), Some(second), "↓ moves on from the first entry");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}
