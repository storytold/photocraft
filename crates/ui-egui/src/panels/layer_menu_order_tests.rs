//! The layer row menu groups Merge Layers / Merge Visible / Flatten Image right after the export block,
//! and Rename Layer… with the link items, as in the editors people already know.

use craft::layers::ui::panel::MenuTarget;
use craft::layers::ui::tree::LayerRowKey;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::layers_tree::{PhotoLayers, RENAME, row_menu};

/// The menu's sections as value lists, split at separators, for a menu on the active layer.
fn sections(app: &PhotocraftApp) -> Vec<Vec<String>> {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let tree = PhotoLayers::new(app, st, &Default::default(), false, None);
    let mut out = vec![Vec::new()];
    for e in row_menu(app, &tree, &MenuTarget::new(Some(LayerRowKey::Layer(id)))) {
        match e.value() {
            Some(v) => out.last_mut().unwrap().push(v.clone()),
            None => out.push(Vec::new()),
        }
    }
    out
}

fn section_with<'a>(sections: &'a [Vec<String>], id: &str) -> &'a [String] {
    sections.iter().find(|s| s.iter().any(|v| v == id)).unwrap_or_else(|| panic!("{id} is in the menu"))
}

#[test]
fn merge_block_follows_export_and_rename_joins_the_link_items() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    let s = sections(&app);
    let export = s.iter().position(|x| x.iter().any(|v| v == "layer.exportAs")).unwrap();
    let merge = s.iter().position(|x| x.iter().any(|v| v == "layer.mergeVisible")).unwrap();
    assert_eq!(merge, export + 1, "the merge block is the section below Quick Export: {s:?}");
    assert_eq!(section_with(&s, "layer.mergeVisible"), ["layer.mergeDown", "layer.mergeVisible", "layer.flattenImage"]);
    assert_eq!(section_with(&s, RENAME), ["layer.linkLayers", "layer.selectLinkedLayers", RENAME]);
    let last = s.last().unwrap();
    assert!(last.iter().all(|v| v.starts_with("layer.layerStyle.")), "the style block ends the menu: {last:?}");
}
