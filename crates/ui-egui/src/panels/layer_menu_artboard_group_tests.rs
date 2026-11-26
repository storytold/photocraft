//! A group row's context menu leads its artboard block with New Artboard and Artboard from Group, as
//! in the reference application (#14566); other rows keep the block they had.

use craft::layers::ui::panel::{LayersEvent, MenuTarget};
use craft::layers::ui::tree::LayerRowKey;
use photocraft_doc::{Layer, LayerId};
use photocraft_engine::Session;
use serde_json::json;

use super::entries;
use crate::PhotocraftApp;
use crate::panels::layers_tree::{PhotoLayers, row_menu};

const NEW: &str = "layer.new.artboard";
const FROM_GROUP: &str = "layer.new.artboardFromGroup";
const FROM_LAYERS: &str = "layer.new.artboardFromLayers";

/// A document with a pixel layer "Top" inside a top-level group "G", the group active.
fn setup() -> (Box<PhotocraftApp>, LayerId) {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 10, "width": 60, "height": 40})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.groupLayers", json!({"name": "G"})).unwrap();
    let group = active(&app).id;
    (app, group)
}

fn active(app: &PhotocraftApp) -> Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

fn ids(l: &Layer, multi: bool) -> Vec<&'static str> {
    entries(l, multi, false).into_iter().flatten().map(|e| e.1).collect()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

#[test]
fn a_group_row_offers_the_three_artboard_items_in_one_block() {
    let (app, _) = setup();
    let e = entries(&active(&app), false, false);
    let at = e.iter().position(|x| x.is_some_and(|x| x.1 == NEW)).expect("New Artboard is offered");
    let block: Vec<_> = e[at..at + 3].iter().map(|x| x.expect("no separator inside the block")).collect();
    assert_eq!(block, [("New Artboard", NEW), ("Artboard from Group", FROM_GROUP), ("Artboard from Layers…", FROM_LAYERS)]);
    assert!(e[at - 1].is_none(), "the block starts after a separator");
}

#[test]
fn other_rows_keep_the_block_they_had() {
    let (mut app, group) = setup();
    let mut rows = Vec::new();
    // The group in a multi-selection: not measured, so it keeps today's entries.
    rows.push(("a multi-selection", active(&app), true));
    // A pixel layer.
    let top = active(&app).children().unwrap()[0].id;
    app.run("layer.select", json!({"layer": top.0, "mode": "replace"})).unwrap();
    rows.push(("a pixel layer", active(&app), false));
    // An artboard, already made from the group.
    app.run(FROM_GROUP, json!({"layer": group.0})).unwrap();
    app.run("layer.select", json!({"layer": group.0, "mode": "replace"})).unwrap();
    rows.push(("an artboard", active(&app), false));
    for (case, l, multi) in rows {
        let ids = ids(&l, multi);
        assert!(!ids.contains(&NEW) && !ids.contains(&FROM_GROUP), "{case}: {ids:?}");
        assert!(ids.contains(&FROM_LAYERS), "{case}: Artboard from Layers… stays");
    }
}

/// The panel shows the entry with the Layer > New item's enabled rule: a nested group cannot become an
/// artboard, so its row shows the entry greyed.
#[test]
fn the_row_menu_greys_artboard_from_group_like_the_layer_menu() {
    let (mut app, group) = setup();
    let enabled = |app: &PhotocraftApp, id: LayerId| {
        let st = app.session.active().unwrap();
        let tree = PhotoLayers::new(app, st, &Default::default(), false, None);
        let menu = row_menu(app, &tree, &MenuTarget::new(Some(LayerRowKey::Layer(id))));
        menu.iter().find(|e| e.value() == Some(&FROM_GROUP.to_string())).map(|e| e.is_enabled()).expect("the entry is shown")
    };
    assert!(enabled(&app, group), "a top-level group can become an artboard");
    let top = active(&app).children().unwrap()[0].id;
    app.run("layer.select", json!({"layer": top.0, "mode": "replace"})).unwrap();
    app.run("layer.groupLayers", json!({"name": "Inner"})).unwrap();
    let inner = active(&app).id;
    assert!(!enabled(&app, inner), "a nested group cannot");
    assert_eq!(enabled(&app, inner), crate::menus::is_enabled(&app, FROM_GROUP));
}

/// Picking Artboard from Group in the row menu turns the clicked group into an artboard in the same
/// history step as Layer > New > Artboard from Group.
#[test]
fn picking_artboard_from_group_converts_the_clicked_group() {
    let (mut app, group) = setup();
    let ctx = app.ctx().clone();
    let pick = LayersEvent::menu(MenuTarget::new(Some(LayerRowKey::Layer(group))), FROM_GROUP);
    crate::panels::layers_events::apply_layers_events(&mut app, &ctx, vec![pick]);
    let l = app.session.active().unwrap().doc.layer(group).unwrap().clone();
    assert!(l.artboard().is_some(), "the clicked group is an artboard");

    let (mut menu_app, menu_group) = setup();
    crate::menus::invoke(&mut menu_app, &ctx, FROM_GROUP, json!({})).unwrap();
    assert!(menu_app.session.active().unwrap().doc.layer(menu_group).unwrap().artboard().is_some());
    assert_eq!(last_step(&app), last_step(&menu_app), "the same history step as the Layer menu");
}
